# Version History

## [0.34.0] - 2024
### Added - Type Hardening: Semantic State Enums
- **types/state.rs** - Ny modul för semantiska tillstånds-enums som eliminerar "Boolean Blindness"
  - `EnableState` - Enabled/Disabled
  - `MuteState` - Unmuted/Muted
  - `SoloState` - Normal/Solo
  - `BypassState` - Active/Bypassed
  - `SyncMode` - Free/TempoSync
  - `RetriggerMode` - Continue/Retrigger
  - `ClipMode` - Off/Soft/Hard
  - `LimitMode` - Enabled/Disabled
- **types/sample.rs** - Flyttade `LoopMode` och `ReleaseMode` från engine/params/modules.rs
  - Tillhör nu types-modulen där sampling-relaterade typer samlas
  - Bakåtkompatibla re-exports i engine/params/modules.rs

### Changed - Module Refactoring with State Enums
- **SharedEngineState** - `ModuleStateSnapshot` använder nu:
  - `bypass_state: BypassState` istället för `bypassed: bool`
  - `mute_state: MuteState` istället för `muted: bool`
  - `solo_state: SoloState` istället för `solo: bool`
- **Amplifier** - `clip_mode: ClipMode` istället för `soft_clip: bool`
  - Stöd för Off, Soft (tanh), och Hard clipping
- **LFO** - Använder nu:
  - `sync_mode: SyncMode` istället för `tempo_sync: bool`
  - `retrigger_mode: RetriggerMode` istället för `retrigger: bool`
- **StereoOutput** - Använder nu:
  - `mute_state: MuteState` istället för `muted: bool`
  - `limit_mode: LimitMode` istället för `limit_enabled: bool`

---

## [0.33.27] - 2024
### Improved - GUI Views Module
- **gui/views/** - Ny modul för återanvändbara GUI-komponenter
  - `views/master_effects.rs` - `MasterEffectParams` och `MasterEffectUiState` typer
  - `views/meters.rs` - `draw_meter()` och `draw_meter_horizontal()` funktioner
  - Reducerade egui_backend.rs från 2698 till 2482 rader (~216 rader flyttade)

---

## [0.33.26] - 2024
### Improved - Code Organization Refactoring
- **gui/input.rs** - Ny modul för keyboard input hantering
  - Extraherade `KEY_MAP` konstant och `handle_keyboard_input()` från egui_backend.rs
  - `KeyboardInputState` struct för att samla input state innan mutation
  - Reducerade egui_backend.rs med ~65 rader
- **SynthEngine command handlers** - Refaktorerade 620-raders `handle_command()` match-block
  - Extraherade 30+ handler-metoder grupperade i kategorier:
    - Instrument management (add, remove, set params, channel, enabled, solo)
    - Note control (note on, note off, all notes off)
    - MIDI controllers (pitch bend, mod wheel, aftertouch, poly aftertouch)
    - Global parameters (master volume, glide time)
    - Voice/module parameters (set voice param, set module param)
    - Reset/clear (reset, clear all modules)
    - Effects (bypass, effect param, enabled, visualizers, add/remove effect)
    - Modular routing (add/remove module, connect, disconnect)
  - Tydligare kodseparation och enklare underhåll

---

## [0.33.25] - 2024
### Added - StereoSample Type & DSP Module
- **StereoSample** - Ny typ i `types/audio.rs` för stereo-samples
  - Ersätter `(f32, f32)` och `[f32; 2]` för stereosignaler
  - Metoder: `new()`, `from_mono()`, `apply_gain()`, `apply_pan()`, `to_mono()`, `mix()`, `soft_clip()`, `hard_clip()`, `peak()`
  - Implementerar `Add`, `Sub`, `Mul<f32>`, `From<(f32, f32)>`, `From<[f32; 2]>`
- **src/dsp/** - Ny modul för återanvändbara DSP-primitiver
  - `dsp/oscillators.rs` - `poly_blep()` och `poly_blep_integrated()` för band-limiterade vågformer
  - `dsp/filters.rs` - `SvfCoeffs` och `BiquadCoeffs` för filterberäkningar
  - `dsp/delay.rs` - `DelayLine` och `InterpolatedDelayLine` för delay-effekter

### Improved - Module Refactoring
- **KeyboardPanner** - Använder nu `StereoSample::apply_pan()` internt
- **StereoOutput** - Refaktorerad att använda `StereoSample` för beräkningar
- **Delay effect** - Använder nu `StereoSample` för stereobearbetning
- **Reverb effect** - Använder nu `StereoSample` för stereobearbetning
- **Oscillator** - Använder nu `dsp::oscillators::poly_blep()` istället för lokal metod

---

## [0.33.24] - 2024
### Improved - Type Methods & Sequencer Types
- **MidiNote::transpose** - Flyttade transponeringslogik till typen, returnerar `Option<MidiNote>`
- **Pitch::transpose** - Sequencer-typ uppdaterad till `Semitones`, returnerar `Option<Pitch>`
- **PatternPlacement** - `transpose` nu `Semitones`, `gain` nu `Gain`
- **TempoChange/Song** - `bpm` och `default_tempo` nu `Bpm` istället för `f32`
- **Pattern::generate_events** - Använder nu `Semitones` för transponering
- **Instrument::transpose_note** - Delegerar nu till `MidiNote::transpose`

---

## [0.33.23] - 2024
### Improved - Strict Type Hardening
- **StereoBalance** - Ny typ för stereopanorering med constant-power gains
- **KeyboardPanner** - Använder nu `MidiNote`, `BipolarValue`, `StereoBalance` istället för primitiver
- **BodyResonance** - Filterstate arrays använder nu `FilterState` istället för `f32`
- **Instrument transpose** - Använder nu `Semitones` istället för `i8`
- **InstrumentParam::Transpose** - Uppdaterad till `Semitones`
- **Oscilloscope** - `mix` och `sample_rate` använder nu `NormalizedValue` och `SampleRate`
- **LevelMeter** - `mix` och `sample_rate` använder nu `NormalizedValue` och `SampleRate`
- **GUI InstrumentUiState** - `transpose` använder nu `Semitones`

---

## [0.33.22] - 2024
### Optimized - GUI Rendering with egui Shape Primitives
- **Kablar** - Ersatte manuell Bézier-loop (32 segment × 3 lager) med `CubicBezierShape`
- **Oscilloskop** - Ersatte ~200 `line_segment()` med en `Shape::line()`
- **Waveform-väljare** - Ersatte 31 `line_segment()` per ikon med `Shape::line()`
- **ADSR Envelope** - Ersatte 4 `line_segment()` med `Shape::line()`
- **Draw Call Batching** - GPU kan nu rita alla punkter i en operation
- **Automatisk LOD** - `CubicBezierShape` hanterar detaljnivå automatiskt

---

## [0.33.21] - 2024
### Improved - Enhanced Sample Player
- **PlaybackState** - Ersatte `bool` med typat enum för tydligare state
- **SampleName** - Newtype för sample-namn istället för rå `String`
- **ReleaseMode** - Tre lägen: `Immediate`, `PlayToEnd`, `PlayToLoop` för flexibel note-off hantering
- **Velocity Sensitivity** - Parameter för hur mycket velocity påverkar volym (0-100%)
- **Nya interpolationer** - Hermite, Lagrange, Sinc8, Sinc16 för högkvalitativ uppspelning
- **Loop Crossfade** - 0-50ms crossfade vid loop-punkter för klickfri looping
- **Root Key Detection** - Automatisk detektion av grundton från filnamn (t.ex. "Piano_C3.wav")
- **WaveformOverview** - Pre-beräknad waveform för effektiv visualisering
- **PlaybackPositionBuffer** - Atomic position buffer för lock-free GUI-synk

### Changed
- Konsoliderade `LoopMode` - endast en definition i params/modules.rs
- Tog bort `loop_mode` från `Sample` struct (hör till spelaren, inte samplen)

---

## [0.33.20] - 2024
### Added - Sample Player & Sample Manager
- **SamplePlayer** - Ny modul för uppspelning av WAV-samples
  - Pitch tracking (transponerar automatiskt baserat på spelade noter)
  - Loop modes: Off, Forward, Backward, PingPong
  - Start/End positions för sample trimming
  - Loop Start/End för preciserad loop-region
  - Speed-kontroll (0.1x - 4.0x)
  - Interpolation: Nearest, Linear, Cubic (Catmull-Rom)
  - Stereo och mono-samples stöds
- **SampleManager** - GUI-thread sample loader med caching
  - Laddar WAV-filer (8/16/24/32-bit int, 32-bit float)
  - Cache förhindrar dubbel-laddning av samma fil
  - Thread-safe via `Arc<Sample>`
- **Nya typer** - `SampleValue`, `PlaybackPosition`, `SampleIndex`, `PlaybackSpeed`, `ChannelMode`, `Interpolation`, `PlaybackDirection`
- **hound** - Nytt beroende för WAV-läsning

---

## [0.33.19] - 2024
### Added - Improved Cables & Auto-Layout
- **Kablar med gravitation** - Kablar hänger nedåt med naturlig "sag" (15% av avståndet)
- **Skuggor** - Svart skugga under kablar för djupkänsla
- **Semi-transparens** - Kablar är delvis genomskinliga (alpha 180)
- **Highlight på hover** - Röd glow-effekt från theme().colors.accent_red
- **Dragging-kablar** - Mindre sag (5%) för responsiv känsla
- **Auto-Layout** - Ny knapp "📐 Auto Layout" som organiserar moduler
  - Vänster→höger baserat på signalflöde (BFS från sources)
  - Envelopes/LFOs placeras under huvudsignalvägen
  - Konfigurerbara avstånd via LayoutConfig

---

## [0.33.18] - 2024
### Fixed - Parameter Routing for Arbitrary Modules
- **SetModuleParameter** används nu för alla voice-moduler istället för SetVoiceParameter
- Fixar parameter-routing för moduler utanför PolyModule enum (env-3, amp-2, sub-1, nse-1, kbp-1, etc)
- **Grand Piano patch** fungerar nu korrekt med alla 3 envelopes
- Tog bort redundant get_voice_module_for_param funktion
- Rensade oanvända imports (PolyModule, Param)

---

## [0.33.17] - 2024
### Changed - Physical Modeling Cleanup
- **Removed** StringResonator, ResonatorBank, VelocityMapper (fungerade inte korrekt)
- **KeyboardPanner** - Not-baserad stereopanorering nu registrerad i GUI-menyn
- **BodyResonance** - Resonanskropp-simulering nu registrerad i GUI-menyn
- **MechanicalNoise** - Mekaniska ljud nu registrerad i GUI-menyn
- **Grand Piano patch** - Uppdaterad med KeyboardPanner för stereo-imaging
- **Physical-menyn** i modulpaletten med de 3 fungerade modulerna

---

## [0.33.16] - 2024
### Added - Physical Modeling Modules
- **StringResonator** - Karplus-Strong string synthesis med inharmonicitet och dämpning
- **ResonatorBank** - Sympatisk resonans med 1-12 avstämbara strängar
- **KeyboardPanner** - Not-baserad stereopanorering för piano-liknande stereo
- **BodyResonance** - Resonanskropp-simulering (soundboard)
- **VelocityMapper** - Velocity-kurvor (Linear, Soft, Hard, S-Curve, Fixed)
- **MechanicalNoise** - Mekaniska ljud (tangent ner/upp, pedal, hammare)
- **ModuleCategory::PhysicalModeling** - Ny kategori för modulpalett

---

## [0.33.15] - 2024
### Added - Keyboard Splitting & MIDI Learn
- **KeyRange** - Ny typ för att definiera vilka noter ett instrument svarar på (keyboard splitting)
- **LearnState** - State machine för MIDI learn (Idle, WaitingForLowNote, WaitingForHighNote)
- **Transpose** - Semitone-offset per instrument (-24 till +24)
- **Instrument Rack UI** - Ny rad med Range-visning, Learn-knapp, Full-knapp, Transpose-kontroll
- **KeyRangeLearned event** - Engine skickar event till GUI när range lärs in
- **note_on/note_off** - Kollar key_range och applicerar transpose

---

## [0.33.14] - 2024
### Improved - GUI Styling & Theme Consistency
- **Master FX Sliders** - Bättre synlighet med mörkare bakgrund och tydlig kontrast
- **WidgetStyle** - Nya fält: knob_arc_segments, slider_rail_height, slider_handle_radius
- **knob.rs** - Använder nu theme().style istället för hårdkodade värden
- **meter.rs** - Använder nu theme().style istället för hårdkodade värden

---

## [0.33.13] - 2024
### Added - Attenuverters for CV Inputs
- **Filter CutoffMod** - Ny "CV Amt" parameter (-1.0 till +1.0) för cutoff CV
- **Oscillator FmAmount** - Ny "FM Amt" parameter (-1.0 till +1.0) för FM input
- Tog bort MIDI debug-spam

---

## [0.33.12] - 2024
### Added - Theme System
- 8 färgteman: Dark, Light, Vintage, Neon, Studio, Dracula, Monokai, Solarized Dark
- Tema-väljare i Settings, WidgetStyle för konsistent styling

---

## [0.33.11] - 2024
### Changed - InputPorts Refactor
- `PolyModule::process()` använder `InputPorts` wrapper istället för HashMap
- Eliminerar HashMap-allokering per audio frame

---

## [0.33.10] - 2024
### Fixed - Realtime Audio Allocations
- Eliminerade `AudioBuffer::new()` i audio thread (~187 allok/sek)
- `Connection` använder `PortName` (Copy) istället för String

---

## [0.33.9] - 2024
### Added - Bypass-knappar
- Power-knapp (⏻) i varje moduls header, bypassade moduler dimmas till 40%

---

## [0.33.8] - 2024
### Added - Master FX Parameters
- Resizable sidopanel, fullständiga parameterkontroller för alla 8 effekttyper

---

## [0.33.7] - 2024
### Added - Master FX Sidebar
- Kollapsbar effektlista i sidopanelen med bypass/remove per effekt

---

## [0.33.6] - 2024
### Added - Mixer & Master Bus
- Solo-knapp, global master effects chain, soft clipper per instrument

---

## [0.33.5] - 2024
### Fixed - Eliminated unwrap/expect
- Fixade 112 Clippy-varningar för unwrap/expect i produktionskod

---

## [0.33.4] - 2024
### Maintenance
- Rensade 29 onödiga clippy allows, uppdaterade 15 beroenden

---

## [0.33.3] - 2024
### Refactored - Clippy Pedantic
- Konfigurerade ~70 pedantic/nursery lints för synth-lämpliga undantag

---

## [0.33.2] - 2024
### Refactored - Best Practices
- `#[must_use]` på transformationsmetoder, tog bort global `#![allow(dead_code)]`
- Konverterade error-typer till thiserror, reducerade unsafe från 5 till 1 block

---

## [0.33.1] - 2024
### Refactored - Idiomatic Iterators
- Konverterade for-loopar till iteratorer utanför hot path, behöll for-loopar i DSP

---

## [0.33.0] - 2024
### Added - Type System Extensions
- `PortName` interning för zero-allocation, `FilterState` DSP-metoder
- `Hertz`, `NormalizedValue`, `MidiChannel`, `BeatDivision` extensions

---

## [0.32.25] - 2024
### Fixed - Instrument Channel Isolation
- Standardinstrument använder CH1 istället för OMNI
- Real-time safe sequencer (pre-allokerad event buffer)

---

## [0.32.24] - 2024
### Refactored - DSP Type Hardening
- `FilterState` i Reverb/Phaser, `Hertz::to_tan_coeff()`, `Milliseconds::to_samples()`

---

## [0.32.23] - 2024
### Fixed - PatchEditor GUI ID Collision
- Window ID inkluderar instrument_id för unika GUI-identifierare
- LadderFilter och NoiseGenerator använder `FilterState`

---

## [0.32.22] - 2024
### Refactored - Total Type Hardening
- `MidiNote` för PolyModule/EngineCommand, `SamplePosition`/`SampleCount` i Voice
- `BufferIndex` i Flanger/Chorus, `NormalizedValue` i SequencerTrack

---

## [0.32.21] - 2024
### Refactored - Type Safety and Enums
- `ModuleType::is_voice_module()`, unified `EffectChain` med `ChainSlot`
- Data-bärande `VoiceState` enum (Idle/Active/Releasing/Stealing)

---

## [0.32.20] - 2024
### Refactored - Per-Instrument Effects
- Flyttade effekter från global MasterBus till per-instrument EffectChain

---

## [0.32.19] - 2024
### Refactored - Per-Instrument PatchEditor
- Varje instrument äger sin egen PatchEditor, patch-laddning per instrument

---

## [0.32.18] - 2024
### Refactored - Per-Instrument Voice Architecture
- `voice_graph` ägs av Instrument istället för SynthEngine

---

## [0.32.17] - 2024
### Refactored - Architectural Terminology
- Renamed: RackView→PatchEditor, VoiceModule→PolyModule, EffectModule→AudioEffect
- SynthPart→Instrument, EffectChain→MasterBus

---

## [0.32.16] - 2024
### Added - Part Manager UI
- Multi-instrument support med Part Manager panel, MIDI-kanal per part

---

## [0.32.15] - 2024
### Improved - Knob Widget
- Värde visas i knob-cirkeln, centraliserad formatering i ParameterUnit
- Custom "Share Tech Mono" font

---

## [0.32.14] - 2024
### Added - MIDI Input Support
- Hardware MIDI via midir med GUI port-väljare, velocity visualization
- Type-safe MIDI parsing, pitch bend, mod wheel, aftertouch

---

## [0.32.13] - 2024
### Fixed - Stereo Output Parameters
- ModuleCategory::Output tillagd i parameter change handling

---

## [0.32.12] - 2024
### Removed - Performance Panel
- Tog bort GUI-komponenten, behöll engine-kommandona för framtida MIDI

---

## [0.32.11] - 2024
### Fixed - Real-time Parameter Updates
- `SetModuleParameter` uppdaterar nu voice_template + alla aktiva voices
- Tog bort 29 ogiltiga oscilloskop-kopplingar från patches

---

## [0.32.10] - 2024
### Fixed - Critical Audio Routing
- StereoOutput klassificeras som voice module, partiella inputs fungerar
- `ClearAllModules` rensar voice_template

---

## [0.32.9] - 2024
### Added - Dynamic Module Routing
- Moduler routas automatiskt till rätt graf baserat på typ

---

## [0.32.8] - 2024
### Refactored - Unified Voice/Graph
- Voice äger ModuleGraph istället för hårdkodad modullista (~400 rader borttaget)

---

## [0.32.7] - 2024
### Refactored - Unified Param Architecture
- `Param` enum med inbakade typade värden, tog bort `TypedValue`

---

## [0.32.6] - 2024
### Fixed - Dropdown Parameter Sync
- Dropdown-handlers skickar rätt TypedValue-variant

---

## [0.32.5] - 2024
### Added - Domain Types for Effects
- `Ratio` (kompression), `BeatDivision` (tempo-sync), `VoiceCount`

---

## [0.32.4] - 2024
### Fixed - Waveform Selection
- GUI skickar TypedValue::Waveform, tog bort noise från Oscillator (använd NoiseGenerator)

---

## [0.32.3] - 2024
### Fixed - CV Modulation Drift
- LadderFilter, LFO, Oscillator använder effective values utan att modifiera parametrar

---

## [0.32.2] - 2024
### Fixed - GUI/Engine Sync
- Startup använder patch_bridge::load_patch() för synkronisering

---

## [0.32.1] - 2024
### Fixed - Ghost Sound Bug
- ClearAllModules inaktiverar parts

---

## [0.32.0] - 2024
### Added - Type Safety & GUI
- Type-safe public APIs med Hertz, Cents, Gain, NormalizedValue
- "New Patch" i File-menyn

---

## [0.31.0] - 2024
### Added - GUI Module Support
- SubOscillator och NoiseGenerator i module palette
- 3 nya example patches

---

## [0.30.0] - 2024
### Added - DSP Improvements
- Envelope curves (attack_curve, decay_curve, release_curve)
- SubOscillator modul (sine, square, -1/-2 oktav)
- NoiseGenerator modul (white, pink, brown, blue, violet)

---

## [0.29.0] - 2024
### Refactored - Modular Patch Structure
- 16 patches extraherade till individuella filer i src/patches/

---

## [0.28.1] - 2024
### Added - Performance Fixes
- Velocity mapping till engine, tempo sync för Delay och LFO
- Module connectivity visualization (Connected/Orphaned/Disconnected)

---

## [0.28.0] - 2024
### Added - Performance Panel
- Pitch bend (spring-back), mod wheel, velocity mapping knobs

---

## [0.27.0] - 2024
### Added - Enhanced Oscilloscope & LFO Tempo Sync
- Oscilloskop med waveform history, time division, trigger modes
- LFO tempo sync med beat divisions

---

## [0.26.0] - 2024
### Added - Engine Events & CPU Tracking
- EngineEvent system med prioriterad kanal
- CPU usage tracking per modul

---

## [0.25.0] - 2024
### Added - Effect Bypass & Master Volume
- Effect bypass per slot, master volume control

---

## [0.24.0] - 2024
### Refactored - Hub Architecture
- EventHub för GUI-engine kommunikation, ersatte direkt polling

---

## [0.23.0] - 2024
### Added - Visual States & Animations
- ModuleVisualState för visuell feedback, cable animations

---

## [0.22.0] - 2024
### Added - Sequencer Engine
- SequencerEngine med transport, looping, note events

---

## [0.21.0] - 2024
### Added - Sequencer Data Model
- Song, Pattern, Note, Track, Automation strukturer

---

## [0.20.0] - 2024
### Added - Effect Chain & Visualizers
- MasterBus med insert effects och visualizers

---

## [0.19.0] - 2024
### Added - Oscilloscope Widget
- Real-time waveform display i GUI

---

## [0.18.0] - 2024
### Added - Level Meter Widget
- VU-meter med peak hold och gradient

---

## [0.17.0] - 2024
### Added - Patch Save/Load
- JSON-baserat patch-format med ModuleBuilder API

---

## [0.16.0] - 2024
### Added - Voice Allocator
- Polyfoni med voice stealing, mono/poly modes

---

## [0.13.2] - 2024
### Fixed - Command Queue Overflow
- Ökade COMMAND_BUFFER_SIZE, DroppedModule wrapper

---

## [0.13.1] - 2024
### Added - Pink Noise & Linear FM
- Pink noise (Voss-McCartney), Linear FM mode, velocity sensitivity

---

## [0.13.0] - 2024
### Added - Math Oscillator
- 18 algoritmer: SineFM, TanChaos, SuperSaw, WaveFolder, Lorenz, KarplusStrong, etc.
- 6 nya example patches

---

## [0.12.0] - 2024
### Initial Release
- Moduler: Oscillator, Filter, Envelope, LFO, Amplifier, Mixer
- Effekter: Delay, Reverb, Distortion, Chorus, Phaser, Flanger, Compressor, EQ
- 10 example patches, piano keyboard, visual cable connections
