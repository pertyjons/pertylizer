# ARCHITECTURE.md

## Projektöversikt

ModularSynth är en modulär ljudsyntes skriven i Rust.
Stack: Rust 1.93+, egui (GUI), cpal (audio I/O), ringbuf (lock-free kommunikation).

---

## Arkitekturöversikt

```
┌──────────────────────────────────────────────────────────────────────────┐
│                              modular_synth                               │
│                     (Huvudapplikation, GUI, I/O)                         │
└─────────────────────────────────┬────────────────────────────────────────┘
                                  │
              ┌───────────────────┼───────────────────┐
              ▼                   ▼                   ▼
┌─────────────────────┐ ┌─────────────────┐ ┌─────────────────────────────┐
│    synth_engine     │ │ synth_sequencer │ │       synth_modules         │
│ (Voice allocation,  │ │ (Pattern, Song, │ │ (Oscillator, Filter, etc.)  │
│  Graph, Commands)   │ │  Events)        │ │                             │
└──────────┬──────────┘ └────────┬────────┘ └──────────────┬──────────────┘
           │                     │                         │
           └─────────────────────┼─────────────────────────┘
                                 ▼
                    ┌────────────────────────┐
                    │       synth_dsp        │
                    │ (PolyBLEP, Filters,    │
                    │  Delay lines)          │
                    └────────────┬───────────┘
                                 │
                                 ▼
                    ┌────────────────────────┐
                    │       synth_core       │
                    │ (Types, Traits,        │
                    │  Audio abstractions)   │
                    └────────────────────────┘
```

**Beroendekedja (bottom-up):**
1. `synth_core` - Inga interna beroenden (endast serde, thiserror)
2. `synth_dsp` - Beror på synth_core
3. `synth_sequencer` - Beror på synth_core
4. `synth_modules` - Beror på synth_core, synth_dsp
5. `synth_engine` - Beror på alla ovan
6. `modular_synth` - Beror på allt, lägger till cpal, egui, midir

---

## Crate-struktur

### synth_core
**Ansvar:** Kärntyper, traits och audio-abstraktioner.
**Nyckeltyper:** `Hertz`, `Gain`, `NormalizedValue`, `MidiNote`, `SampleRate`, `AudioBuffer`
**Nyckeltraits:** `PolyModule`, `AudioProcessor`, `Describable`
**Beroenden:** serde, thiserror

### synth_dsp
**Ansvar:** Lågnivå DSP-primitiver som återanvänds av moduler.
**Nyckeltyper:** `DelayLine`, `SvfCoeffs`, `BiquadCoeffs`
**Funktioner:** `poly_blep()` (band-limited waveform generation)
**Beroenden:** synth_core

### synth_sequencer
**Ansvar:** Sekvensering med pattern och song.
**Nyckeltyper:** `Pattern`, `Song`, `Note`, `SequencerEvent`
**Beroenden:** synth_core

### synth_modules
**Ansvar:** Färdiga ljudmoduler och effekter.
**Moduler:** `Oscillator`, `Filter`, `Envelope`, `Lfo`, `Amplifier`
**Effekter:** `Delay`, `Reverb`, `Chorus`, `Distortion`, `Compressor`, `Eq`, `Phaser`, `Flanger`
**Fysisk modellering:** `BodyResonance`, `MechanicalNoise`, `KeyboardPanner`
**Beroenden:** synth_core, synth_dsp

### synth_engine
**Ansvar:** Ljudmotor med voice allocation, graf och sequencer-körning.
**Nyckeltyper:** `SynthEngine`, `Voice`, `VoiceAllocator`, `ModuleGraph`, `Instrument`
**Kommunikation:** `EngineCommand`, `EngineEvent`, `CommandSender`, `EngineHandle`
**Beroenden:** synth_core, synth_dsp, synth_modules, synth_sequencer

### modular_synth
**Ansvar:** Huvudapplikation med GUI och I/O.
**GUI:** egui-baserat (PatchEditor, InstrumentRack)
**Audio:** cpal-backend
**Beroenden:** Alla crates + cpal, egui, midir

---

## Dataflöde

### UI → Audio (Commands)

```
┌──────────┐     ┌────────────────┐     ┌──────────────┐
│   GUI    │────▶│  CommandSender │────▶│ SynthEngine  │
│ (egui)   │     │  (ring buffer) │     │(audio thread)│
└──────────┘     └────────────────┘     └──────────────┘
     │                                         │
     │           ┌────────────────┐            │
     └───────────│  EngineEvent   │◀───────────┘
                 │  (ring buffer) │
                 └────────────────┘
```

1. GUI skickar `EngineCommand` via `CommandSender` (lock-free ring buffer)
2. Audio-tråden processar kommandon i `SynthEngine::process()`
3. Engine skickar tillbaka `EngineEvent` till GUI (meter updates, etc.)

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

**Pattern:** En sekvens av noter organiserad i rader och spår.

**Tick:** Minsta tidsenheten i sequencern. 960 ticks = 1 quarter note (PPQN).

**Newtype Pattern:** Alla domänvärden wrappas i typsäkra typer istället för råa primitiver.
Se `synth_core::types` för `Hertz`, `Gain`, `NormalizedValue`, etc.

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
2. Lägg till `ModuleType`-variant i `synth_core/src/params/module_type.rs`
3. Lägg till params-enum i `synth_core/src/params/` (t.ex. `NewModuleParam`)
4. Exponera via `synth_modules/src/lib.rs`
5. Lägg till factory i `synth_engine` för att skapa modulinstanser
6. Lägg till GUI-panel i `modular_synth/src/gui/panels/`

**Berörda filer:** module_type.rs, params/*.rs, synth_modules/src/*.rs, graph.rs

### Lägga till ny effekt

1. Skapa effekt i `synth_modules/src/effects/`
2. Lägg till i `EffectType` enum i `synth_engine/src/commands.rs`
3. Lägg till i `EffectChain::add_effect()` factory
4. Exponera via `synth_modules/src/lib.rs`

**Berörda filer:** effects/*.rs, commands.rs, effect_chain.rs

---

## Filstruktur

```
modular-synth/
├── crates/
│   ├── synth_core/        # Kärntyper och traits
│   │   └── src/
│   │       ├── types/     # Newtypes (Hertz, Gain, etc.)
│   │       ├── params/    # Parameter-enums per modul
│   │       ├── audio/     # AudioProcessor, AudioBackend
│   │       └── module_traits.rs
│   ├── synth_dsp/         # DSP-primitiver
│   │   └── src/
│   │       ├── oscillators.rs  # PolyBLEP
│   │       ├── filters.rs      # SVF, Biquad
│   │       └── delay.rs        # Delay lines
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
│   │       └── effects/        # Delay, Reverb, etc.
│   ├── synth_engine/      # Ljudmotor
│   │   └── src/
│   │       ├── synth_engine.rs # Huvudmotor
│   │       ├── voice.rs        # Voice med state
│   │       ├── voice_allocator.rs
│   │       ├── graph.rs        # ModuleGraph
│   │       ├── instrument.rs   # Multitimbral
│   │       └── commands.rs     # EngineCommand/Event
│   └── modular_synth/     # Huvudapplikation
│       └── src/
│           ├── main.rs
│           ├── gui/
│           │   ├── egui_backend.rs  # Huvudfönster
│           │   └── patch_editor.rs  # Modulär redigering
│           ├── audio/       # cpal-backend
│           └── io/          # MIDI, fil-I/O
├── docs/
│   ├── history.md       # Versionshistorik
│   └── TODO.md          # Framtida features
└── CLAUDE.md            # AI-instruktioner
```

---

## Kända Begränsningar

- **Ingen Sequencer-vy ännu** — Grundläggande SequencerEngine finns men GUI byggs from scratch
- **Ingen WAV-export** — Endast realtidsuppspelning
- **Ingen Undo/Redo** — Patch-ändringar kan inte ångras
- **Graf använder HashMap** — Planerad optimering till "baked graph" för cache-lokalitet
- **Endast stereo output** — Ingen surround-support

---

## Externa Beroenden

| Crate | Syfte |
|-------|-------|
| cpal | Plattformsoberoende audio I/O |
| egui/eframe | Immediate-mode GUI |
| ringbuf | Lock-free ring buffers |
| parking_lot | Snabbare mutex för icke-kritiska paths |
| midir | MIDI I/O |
| serde/serde_json | Serialisering för patch-filer |
| thiserror | Ergonomisk error-hantering |
