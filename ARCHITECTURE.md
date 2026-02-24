# ARCHITECTURE.md

## Projektöversikt

ModularSynth är en modulär ljudsyntes skriven i Rust.
Stack: Rust 1.93 (Edition 2024), egui (GUI), cpal (audio I/O), ringbuf (lock-free kommunikation), rmcp (MCP-server).

---

## Arkitekturöversikt

```
┌──────────────────────────────────────────────────────────────────────────┐
│                              modular_synth                               │
│                     (Huvudapplikation, GUI, I/O)                         │
└─────────────────────────────────┬────────────────────────────────────────┘
                                  │
          ┌───────────────────┬───┼───────────────┬───────────────┐
          ▼                   ▼   ▼               ▼               ▼
┌──────────────────┐ ┌────────────────┐ ┌──────────────────┐ ┌──────────┐
│  synth_engine    │ │ synth_sequencer│ │  synth_modules   │ │ synth_mcp│
│ (Voice alloc,    │ │ (Pattern, Song,│ │ (Oscillator,     │ │(MCP-server│
│  Graph, Commands)│ │  Events)       │ │  Filter, etc.)   │ │ AI-agent) │
└────────┬─────────┘ └───────┬───────┘ └────────┬─────────┘ └──────────┘
         │                   │                   │
         │            ┌──────┼───────────────────┘
         │            │      │
         ▼            ▼      ▼
┌──────────────────┐ ┌────────────────┐
│    synth_dsp     │ │   synth_awe    │
│ (PolyBLEP,       │ │ (Spatial audio,│
│  Filters, Delay) │ │  Room sim)     │
└────────┬─────────┘ └───────┬───────┘
         │                   │
         └───────┬───────────┘
                 ▼
    ┌────────────────────────┐
    │       synth_core       │
    │ (Types, Traits,        │
    │  Audio abstractions)   │
    └────────────────────────┘
```

**Beroendekedja (bottom-up):**
1. `synth_core` - Inga interna beroenden (endast serde, thiserror)
2. `synth_dsp` - Beror på `synth_core`
3. `synth_awe` - Beror på `synth_core`, `synth_dsp`
4. `synth_sequencer` - Beror på `synth_core`
5. `synth_modules` - Beror på `synth_core`, `synth_dsp`
6. `synth_engine` - Beror på alla ovan
7. `synth_mcp` - Beror på `synth_core` (trait + typer, ej engine)
8. `modular_synth` - Beror på allt, lägger till cpal, egui, midir

---

## Crate-struktur

### synth_core
**Ansvar:** Kärntyper, traits och audio-abstraktioner.
**Nyckeltyper:** `Hertz`, `Gain`, `NormalizedValue`, `MidiNote`, `SampleRate`, `AudioBuffer`
**Nyckeltraits:** `PolyModule`, `AudioProcessor`, `Describable`
**Typ-moduler:** `types/amplitude.rs`, `types/frequency.rs`, `types/pitch.rs`, `types/time.rs`, `types/samples.rs`, `types/normalized.rs`, `types/state.rs`, `types/range.rs`
**Beroenden:** serde, thiserror

### synth_dsp
**Ansvar:** Lågnivå DSP-primitiver som återanvänds av moduler.
**Nyckeltyper:** `DelayLine`, `SvfCoeffs`, `BiquadCoeffs`
**Funktioner:** `poly_blep()` (band-limited waveform generation)
**Beroenden:** synth_core, realfft

### synth_awe
**Ansvar:** Spatial audio och rumssimulering.
**Nyckeltyper:** `AweEngine`, `Room`, `Spatializer`, `SpatialVoice`
**Komponenter:** `room_modes.rs`, `early_reflections.rs`, `wall_absorption.rs`
**Beroenden:** synth_core, synth_dsp

### synth_sequencer
**Ansvar:** Sekvensering med pattern och song.
**Nyckeltyper:** `Pattern`, `Song`, `Note`, `SequencerEvent`
**Beroenden:** synth_core

### synth_modules
**Ansvar:** Färdiga ljudmoduler och effekter.
**Moduler:** `Oscillator`, `Filter`, `Envelope`, `Lfo`, `Amplifier`, `SubOscillator`, `MathOscillator`, `Noise`
**Effekter:** `Delay`, `Reverb`, `Chorus`, `Distortion`, `Compressor`, `Eq`, `Phaser`, `Flanger`
**Fysisk modellering:** `BodyResonance`, `MechanicalNoise`, `KeyboardPanner`
**Beroenden:** synth_core, synth_dsp, fastrand

### synth_engine
**Ansvar:** Ljudmotor med voice allocation, graf och sequencer-körning.
**Nyckeltyper:** `SynthEngine`, `Voice`, `VoiceAllocator`, `ModuleGraph`, `Instrument`
**Kommunikation:** `EngineCommand`, `EngineEvent`, `CommandSender`, `EngineHandle`
**Delad state:** `SharedGraphState`, `ModuleStateSnapshot`, `ConnectionSnapshot` — trådsäker snapshot av grafen för GUI och MCP
**Beroenden:** synth_core, synth_dsp, synth_modules, synth_sequencer, ringbuf, parking_lot

### synth_mcp
**Ansvar:** MCP-server (Model Context Protocol) för AI-agent-integration.
**Nyckeltyper:** `SynthBridge` (trait), `SynthMcpServer` (rmcp handler), `McpBridgeError`
**Verktyg:** 11 MCP tools — list/get instruments, modules, connections, parameters, engine status, diagnostics, set parameter, note on/off
**Transport:** TCP :9850 (GUI-läge) eller stdio (headless)
**Beroenden:** synth_core, rmcp, tokio, schemars, serde
**Dokumentation:** Se `docs/MCP.md` för fullständig beskrivning

### modular_synth
**Ansvar:** Huvudapplikation med GUI, audio I/O och MCP-bridge.
**GUI:** egui-baserat med widgets (knob, meter, port, scope, spectrum, cable, envelope, waveform)
**Vyer:** `patch_editor.rs` (modulär graf), `instrument_rack.rs`, `master_effects.rs`, `awe_view.rs`
**Audio:** cpal-backend
**MCP:** `mcp_bridge.rs` (AppSynthBridge impl), `bin/synth-mcp-bridge.rs` (stdio↔TCP proxy)
**Debug:** `graph_debugger.rs`, `sequencer_debugger.rs`, `voice_debugger.rs`, `signal_probe.rs`
**Beroenden:** Alla crates + cpal, egui, midir

---

## Dataflöde

### UI / MCP → Audio (Commands)

```
┌──────────┐     ┌────────────────┐     ┌──────────────┐
│   GUI    │────▶│  CommandSender │────▶│ SynthEngine  │
│ (egui)   │     │  (ring buffer) │     │(audio thread)│
└──────────┘     └────────────────┘     └──────────────┘
     │                  ▲                       │
     │                  │                       │
     │           ┌──────┘                       │
     │           │                              │
┌──────────┐     │   ┌────────────────┐         │
│   MCP    │─────┘   │  EngineEvent   │◀────────┘
│ (AI agent│         │  (ring buffer) │
│  TCP/stdio)        └────────────────┘
└──────────┘
```

1. GUI eller MCP-agent skickar `EngineCommand` via `CommandSender` (lock-free ring buffer)
2. Audio-tråden processar kommandon i `SynthEngine::process()`
3. Engine skickar tillbaka `EngineEvent` till GUI (meter updates, etc.)
4. Engine uppdaterar `SharedGraphState` vid topologiändringar (läsbart av MCP och GUI)

### Audio Processing

```
MIDI/Sequencer Events
        │
        ▼
┌──────────────────┐
│   VoiceAllocator │ ─── Tilldelar noter till voices
└────────┬─────────┘
         │
         ▼
┌──────────────────┐
│      Voice       │ ─── Innehåller ModuleGraph
│   ┌──────────┐   │
│   │ OSC→FLT→ │   │
│   │ ENV→AMP  │   │
│   └──────────┘   │
└────────┬─────────┘
         │
         ▼
┌──────────────────┐
│   EffectChain    │ ─── Delay, Reverb, etc.
└────────┬─────────┘
         │
         ▼
┌──────────────────┐
│    Instrument    │ ─── Mixar voices, soft clipper
│   (per channel)  │
└────────┬─────────┘
         │
         ▼
┌──────────────────┐
│   Master Output  │ ─── Final stereo out
└──────────────────┘
```

### MCP-kommunikation

```
Claude Code ←stdio→ synth-mcp-bridge ←TCP:9850→ modular-synth
                                                      │
                                    AppSynthBridge läser EngineState
                                    (SharedGraphState, meters, transport)
                                    och skickar EngineCommand
```

Se `docs/MCP.md` för fullständig MCP-dokumentation.

---

## Nyckelkoncept

**Voice:** En ljudkälla som spelar en not. Har `VoiceState` (Idle/Active/Releasing/Stealing).
Innehåller en `ModuleGraph` för flexibel signalkedja.

**Instrument:** Samling av voices för ett timbre. Har egen `VoiceAllocator` för polyfoni,
`EffectChain`, och `MidiChannel`.

**ModuleGraph:** Riktad acyklisk graf (DAG) av ljudmoduler. Hanterar:
- Registrering och uppslag av moduler via `ModuleId`
- Kopplingar mellan portar (`Connection`)
- Topologisk sortering för korrekt processningsordning

**ModuleId:** Typsäker identifierare för modulinstans, format `{prefix}-{instance}` (t.ex. "osc-1", "flt-2").

**SharedGraphState:** Trådsäker snapshot av modulgrafen. Uppdateras av audio-tråden vid
topologi-/parameterändringar. Läses av GUI och MCP-servern via `RwLock`.

**Pattern:** En sekvens av noter organiserad i rader och spår.

**Tick:** Minsta tidsenheten i sequencern. 960 ticks = 1 quarter note (PPQN).

**Newtype Pattern:** Alla domänvärden wrappas i typsäkra typer istället för råa primitiver.
Se `synth_core::audio::types` för `Hertz`, `Gain`, `NormalizedValue`, etc.

**SynthBridge:** Trait som abstraherar synth-engine för MCP-servern. Implementeras av
`AppSynthBridge` i `modular_synth`. Tillåter testning och alternativa backends.

---

## Kritiska Invarianter

**AUDIO THREAD:** Får ALDRIG allokera minne (malloc/new). Orsakar ljudglapp.
Undvik: `Vec::push`, `HashMap::insert`, `String::clone`, `Box::new`.

**NEWTYPE:** Publika API:er använder newtypes, inte primitiver.
Fel: `fn set_freq(hz: f32)` → Rätt: `fn set_freq(freq: Hertz)`.

**VOICE STATE:** Använd enum-varianter med embedded data för att göra ogiltiga tillstånd orepresenterbara.
`VoiceState::Active { note, velocity, start_time }` istället för separata fält.

**LOCK-FREE:** Kommunikation mellan trådar via `ringbuf`, inte `Mutex::lock`.
Undantag: `parking_lot::Mutex` för visualiseringsbuffertar (icke-kritisk path).

**MODULE CLEANUP:** Moduler som tas bort skickas tillbaka till main thread via ring buffer
för att undvika deallokering på audio thread (`DroppedModule`, `DroppedItem`).

---

## Vanliga Operationer

### Lägga till ny modul-typ

1. Definiera modulen i `synth_modules/src/` (implementera `PolyModule` trait)
2. Lägg till `ModuleType`-variant i `synth_core/src/params/mod.rs`
3. Lägg till params-enum i `synth_core/src/params/` (t.ex. `NewModuleParam`)
4. Exponera via `synth_modules/src/lib.rs`
5. Lägg till factory i `synth_engine` för att skapa modulinstanser
6. Lägg till GUI-panel i `modular_synth/src/gui/panels/`

**Berörda filer:** `params/mod.rs`, `params/*.rs`, `synth_modules/src/*.rs`, `graph.rs`

### Lägga till ny effekt

1. Skapa effekt i `synth_modules/src/effects/`
2. Lägg till i `EffectType` enum i `synth_engine/src/commands.rs`
3. Lägg till i `EffectChain::add_effect()` factory
4. Exponera via `synth_modules/src/lib.rs`

**Berörda filer:** `effects/*.rs`, `commands.rs`, `effect_chain.rs`

---

## Filstruktur

```
modular-synth/
├── crates/
│   ├── synth_core/        # Kärntyper och traits
│   │   └── src/
│   │       ├── audio/     # AudioProcessor, backends
│   │       ├── types/     # Newtypes: amplitude, frequency, pitch, time, samples, normalized
│   │       ├── params/    # Parameter-enums per modul
│   │       ├── module_traits.rs
│   │       └── lib.rs
│   ├── synth_dsp/         # DSP-primitiver
│   │   └── src/
│   │       ├── oscillators.rs  # PolyBLEP
│   │       ├── filters.rs      # SVF, Biquad
│   │       ├── delay.rs        # Delay lines
│   │       └── spectral/       # FFT, STFT, partitioned convolver
│   ├── synth_awe/         # Spatial audio & rumssimulering
│   │   └── src/
│   │       ├── awe_engine.rs      # Huvudmotor
│   │       ├── room.rs            # Rumsmodell
│   │       ├── spatializer.rs     # 3D-positionering
│   │       ├── spatial_voice.rs   # Per-röst spatial
│   │       ├── room_modes.rs      # Resonansfrekvenser
│   │       └── early_reflections.rs
│   ├── synth_sequencer/   # Sekvensering
│   │   └── src/
│   │       ├── pattern.rs      # Pattern container
│   │       ├── song.rs         # Song arrangement
│   │       └── events.rs       # SequencerEvent
│   ├── synth_modules/     # Ljudmoduler
│   │   └── src/
│   │       ├── oscillator.rs
│   │       ├── filter.rs
│   │       ├── envelope.rs
│   │       └── effects/        # Delay, Reverb, Chorus, etc.
│   ├── synth_engine/      # Ljudmotor
│   │   └── src/
│   │       ├── synth_engine.rs    # Huvudmotor
│   │       ├── voice.rs           # Voice med state
│   │       ├── voice_allocator.rs
│   │       ├── graph.rs           # ModuleGraph (DAG)
│   │       ├── instrument.rs      # Multitimbral
│   │       ├── commands.rs        # EngineCommand/Event, ModuleId
│   │       ├── shared_state.rs    # SharedGraphState, snapshots
│   │       └── state.rs           # EngineState (Arc-delad)
│   ├── synth_mcp/         # MCP-server (AI-agent-integration)
│   │   └── src/
│   │       ├── lib.rs          # serve_stdio(), serve_tcp()
│   │       ├── bridge.rs       # SynthBridge trait
│   │       ├── server.rs       # rmcp ServerHandler, 11 tools
│   │       ├── types.rs        # ModuleInfo, ParameterInfo, etc.
│   │       └── error.rs        # McpBridgeError
│   └── modular_synth/     # Huvudapplikation
│       └── src/
│           ├── main.rs
│           ├── mcp_bridge.rs      # AppSynthBridge (SynthBridge impl)
│           ├── bin/
│           │   └── synth-mcp-bridge.rs  # stdio↔TCP proxy
│           ├── gui/
│           │   ├── app/           # App state, uppdateringsloop
│           │   ├── patch_editor.rs     # Modulär grafredigering
│           │   ├── instrument_rack.rs  # Instrumentpaneler
│           │   ├── master_effects.rs   # Effektkedja
│           │   ├── awe_view.rs         # Spatial audio-vy
│           │   └── widgets/       # knob, meter, port, scope, spectrum, cable, envelope
│           ├── audio/       # cpal-backend
│           ├── io/          # MIDI, fil-I/O
│           └── debug/       # graph_debugger, voice_debugger, signal_probe
├── docs/
│   ├── history.md       # Versionshistorik
│   ├── TODO.md          # Framtida features
│   └── MCP.md           # MCP-dokumentation
└── CLAUDE.md            # AI-instruktioner
```

---

## Kända Begränsningar

- **Ingen Sequencer-vy ännu** — Grundläggande SequencerEngine finns men GUI byggs from scratch
- **Ingen WAV-export** — Endast realtidsuppspelning
- **Ingen Undo/Redo** — Patch-ändringar kan inte ångras
- **Graf använder HashMap** — Planerad optimering till "baked graph" för cache-lokalitet
- **Endast stereo output** — Ingen surround-support
- **MCP Fas 1 (read/write)** — Topologiändring (add/remove module, connect/disconnect) ej implementerat ännu

---

## Externa Beroenden

| Crate | Syfte |
|-------|-------|
| cpal | Plattformsoberoende audio I/O |
| egui/eframe | Immediate-mode GUI (med `egui_extras`, `egui-file-dialog`) |
| ringbuf | Lock-free ring buffers |
| parking_lot | Snabbare mutex för icke-kritiska paths |
| midir | MIDI I/O |
| serde/serde_json | Serialisering för patch-filer |
| thiserror | Ergonomisk error-hantering |
| fastrand | Snabba, deterministiska slumptal |
| arrayvec | Heap-fria fixed-size collections |
| realfft | FFT-beräkningar för spektrala effekter |
| dirs | Hantering av systemkataloger (t.ex. för patchar) |
| rmcp | MCP-server (Model Context Protocol) |
| tokio | Async runtime för MCP TCP-server |
| schemars | JSON Schema-generering för MCP-verktygsparametrar |
