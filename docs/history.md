# Version History

## [0.28.0] - 2024

### Added - Performance Panel GUI

- **New Performance Panel** (`src/gui/performance_panel.rs`)
  - Toggleable side panel for real-time performance controls
  - Styled with `module_frame` for consistent synth look
  - `PerformanceState` struct holds panel state

- **Macro Controllers**
  - **Pitch Bend** - Vertical spring-loaded slider (-1 to +1)
    - Returns to center on release (`drag_stopped()`)
    - Sends `EngineCommand::PitchBend` with `BipolarValue`
  - **Mod Wheel** - Vertical slider (0 to 1)
    - Stays where released (no spring-back)
    - Sends `EngineCommand::ModWheel` with `NormalizedValue`

- **Velocity Mapping Knobs**
  - Amp Sensitivity knob (0-100%)
  - Filter Sensitivity knob (0-100%)
  - Uses `Knob` widget for consistent UI

- **GUI Integration** (`src/gui/egui_backend.rs`)
  - Added `show_performance_panel: bool` state toggle
  - Added "🎹 Perf" button in toolbar
  - `SidePanel::left("performance_panel")` with 140px width

### Technical Details
- Custom `vertical_slider()` function with styled thumb and track
- Uses `StrokeKind::Outside` for egui 0.33 compatibility
- Commands sent to `MidiChannel::OMNI` for all parts
- All 229 unit tests passing

---

## [0.27.0] - 2024

### Added - Configurable Expression Settings with Strong Types

- **ExpressionSettings Struct** (`src/engine/voice.rs`)
  - New `ExpressionSettings` type for configurable expressiveness
  - `pitch_bend_range: Semitones` - configurable range (default ±2 semitones)
  - `vibrato_depth: NormalizedValue` - max mod wheel vibrato (default 2.5%)
  - `velocity_to_amp: NormalizedValue` - amplitude sensitivity (default 100%)
  - `velocity_to_filter: NormalizedValue` - filter cutoff sensitivity (default 50%)
  - Added to `Voice` struct as `expression: ExpressionSettings`

- **Type-Safe Pitch Bend DSP**
  - Uses `Semitones::apply(Hertz) -> Hertz` for frequency calculation
  - Eliminates manual `2^(semitones/12)` calculation
  - Pitch bend range now configurable per-voice

- **Configurable Velocity Sensitivity**
  - Formula: `scale = (1 - sensitivity) + sensitivity * velocity`
  - At sensitivity=0: constant output (no velocity effect)
  - At sensitivity=1: full dynamic range
  - Applied to both amplitude and filter cutoff independently

### Technical Details
- `Semitones * f32 -> Semitones` multiplication used for pitch bend scaling
- `NormalizedValue * NormalizedValue -> NormalizedValue` for vibrato depth
- Expression settings copied in `Voice::clone_structure()`
- All 229 unit tests passing

---

## [0.26.0] - 2024

### Added - Complete Expressiveness DSP & Fastrand

- **DSP Implementation** (`src/engine/voice.rs`)
  - Pitch bend: exponential frequency calculation `2^(semitones/12)`
  - Mod wheel: scales vibrato depth `lfo * mod_wheel * 0.025`
  - Velocity → Filter: harder hits open filter more `0.5 + 0.5 * velocity`
  - Velocity → Amp: direct amplitude scaling

- **Replaced NoiseState with fastrand**
  - `oscillator.rs`: White/pink noise now uses `fastrand::f32()`
  - `lfo.rs`: Sample & Hold random uses `fastrand::f32()`
  - `math_oscillator.rs`: Chaos/noise algorithms use `fastrand::f32()`
  - Removed `noise_state: NoiseState` field from all oscillator structs
  - Thread-local storage (TLS) - lock-free and audio-safe

### Technical Details
- `fastrand` is thread-local and lock-free - safe for audio thread
- Pitch bend calculation done once per block (outside sample loop)
- All 229 unit tests passing

---

## [0.25.0] - 2024

### Added - Expressiveness & Total Type Safety

- **Macro Controllers** (`src/engine/voice.rs`)
  - Added `pitch_bend: BipolarValue` field to Voice (±2 semitones range)
  - Added `mod_wheel: NormalizedValue` field (scales vibrato depth)
  - Added `aftertouch: NormalizedValue` field
  - Velocity changed from `f32` to `NormalizedValue`

- **New `Bpm` Type** (`src/types/time.rs`)
  - Type-safe tempo representation
  - Methods: `beat_duration()`, `samples_per_beat()`, `beats_per_sample()`
  - Constants: `DEFAULT` (120), `MIN` (20), `MAX` (300)

- **Type-Safe EngineCommand** (`src/engine/commands.rs`)
  - `NoteOn { velocity: NormalizedValue }` - type-safe velocity
  - `PitchBend { value: BipolarValue }` - type-safe bipolar value
  - `ModWheel { value: NormalizedValue }` - NEW command for CC1
  - `Aftertouch { value: NormalizedValue }` - type-safe aftertouch
  - `PolyAftertouch { value: NormalizedValue }` - type-safe poly AT
  - `SetTempo(Bpm)` - type-safe tempo
  - `SetMasterVolume(Gain)` - type-safe gain
  - `SetGlideTime(Seconds)` - type-safe duration
  - `PartParam::GlideTime(Seconds)` - type-safe part glide

- **Command Handlers** (`src/engine/synth_engine.rs`)
  - PitchBend handler: applies to all voices on matching channel
  - ModWheel handler: applies to all voices on matching channel
  - Aftertouch handler: applies to all voices on matching channel
  - PolyAftertouch handler: applies to specific note's voice

- **Dependency Added**
  - `fastrand = "2.3"` for fast random number generation

### Technical Details
- Zero raw `f32` in `EngineCommand` public API
- MIDI values converted to domain types at API boundary
- Compiler catches unit mismatches (e.g., frequency vs gain)
- Voice allocator uses `NormalizedValue` internally
- All 229 unit tests passing

---

## [0.24.0] - 2024

### Added - The Big Rewire: Multitimbral SynthPart Processing

- **SynthPart Voice Processing** (`src/engine/part.rs`)
  - Added internal `voice_left` and `voice_right` `AudioBuffer`s to each part
  - New `SynthPart::process()` method - processes all voices and mixes to output
  - Voice processing logic moved from `SynthEngine` to `SynthPart`
  - Per-part volume (`Gain`) and pan (`BipolarValue`) applied during mixing
  - Handles voice stealing fade-out and glide updates

- **SynthEngine Refactoring** (`src/engine/synth_engine.rs`)
  - `process_voices()` now delegates to `SynthPart::process()` for each part
  - Removed redundant `voice_left`/`voice_right` buffers from engine
  - Cleaner separation: Engine orchestrates, Parts process

- **Sequencer-to-Part Integration**
  - Sequencer events now trigger notes on correct parts
  - `InstrumentId` maps to part index (0 = first part, etc.)
  - Fallback to first part if instrument index out of range

- **Real-time Safety for Part Removal**
  - Added `part_return_producer`/`part_return_consumer` ring buffer
  - `RemovePart` command sends parts back to GUI thread for dropping
  - Prevents memory deallocation on audio thread
  - `cleanup_dropped_modules()` now cleans up both modules and parts

### Technical Details
- Voice processing encapsulated in `SynthPart::process()` (~80 lines)
- `SynthEngine::process_voices()` reduced to ~20 lines
- Type-safe throughout: uses `VoiceState`, `Gain`, `BipolarValue`, `ProcessContext`
- All 229 unit tests passing

---

## [0.23.0] - 2024

### Refactored - Strong Types in Effect Modules

- **Effect Modules Refactored** (`src/effects/`)
  - `reverb.rs`: Uses `NormalizedValue`, `SampleRate`, `Seconds`
  - `distortion.rs` (Distortion): Uses `NormalizedValue`, `SampleRate`
  - `distortion.rs` (Chorus): Uses `Hertz`, `NormalizedValue`, `Phase`, `SampleRate`
  - `phaser.rs`: Uses `Hertz`, `NormalizedValue`, `BipolarValue`, `Phase`, `SampleRate`
  - `flanger.rs`: Uses `Hertz`, `NormalizedValue`, `BipolarValue`, `Phase`, `SampleRate`
  - `compressor.rs`: Uses `Decibels`, `NormalizedValue`, `SampleRate`
  - `eq.rs`: Uses `Hertz`, `Decibels`, `NormalizedValue`, `SampleRate`

### Technical Details
- Eliminated raw `f32` for domain-specific values in all effect modules
- Consistent use of `.as_f32()` accessor pattern
- Type-safe parameter handling with `TypedParam`/`TypedValue`
- All 229 unit tests passing

---

## [0.22.0] - 2024

### Added - Dynamic Multitimbrality

- **Part System** (`src/engine/part.rs`)
  - `PartId(u64)` newtype for unique part identifiers
  - `MidiChannel(u8)` newtype with OMNI/CH1-16/DRUMS constants
  - `SynthPart` struct encapsulating independent voice allocation
  - Per-part volume (`Gain`) and pan (`BipolarValue`)
  - MIDI channel routing with OMNI mode support

- **New Commands** (`src/engine/commands.rs`)
  - `AddPart { part: Box<SynthPart> }` - real-time safe part creation
  - `RemovePart { part_id: PartId }` - remove part by ID
  - `SetPartParameter { part_id, param: PartParam }` - volume/pan/glide/mode
  - `SetPartMidiChannel { part_id, channel }` - MIDI channel assignment
  - `SetPartEnabled { part_id, enabled }` - enable/disable parts
  - `PartParam` enum: Volume, Pan, GlideTime, AllocationMode, StealingStrategy

- **SynthEngine Refactoring**
  - Replaced single `VoiceAllocator` with `Vec<Box<SynthPart>>`
  - Notes routed to parts based on MIDI channel matching
  - Part volume/pan applied during voice mixing
  - Default part uses OMNI mode for backwards compatibility

### Changed
- `NoteOn`/`NoteOff` commands now use `MidiChannel` instead of raw `u8`
- `EngineHandle::note_on_channel()` and `note_off_channel()` for channel-specific notes

### Technical Details
- Type-safe: No raw `u8` or `usize` in public part APIs
- Real-time safe: Parts created in GUI thread, sent via commands
- Unlimited parts: Dynamic `Vec` allows any number of parts
- All 229 unit tests passing

---

## [0.21.0] - 2024

### Added - Type-Safe Sequencer Engine

- **SequencerEngine** (`src/engine/sequencer_engine.rs`)
  - Real-time playback engine using domain-specific newtypes
  - `SampleRate` instead of `f32` for sample rates
  - `SampleCount` instead of `usize` for buffer sizes
  - `Tick` instead of `u64` for song positions
  - Sub-tick precision via `tick_accumulator: f64`

- **Type-Safe API**
  - `process(samples: SampleCount) -> Vec<SequencerEvent>`
  - `set_sample_rate(sr: SampleRate)`
  - `seek(tick: Tick)`
  - `set_loop(start: Tick, end: Tick, enabled: bool)`

- **Playback Features**
  - Play/Pause/Stop state machine (`PlayState` enum)
  - Automatic NoteOff generation for active notes
  - Loop point support with proper note release
  - Tempo-aware tick calculation: `(samples / sample_rate) * (bpm / 60) * TICKS_PER_QUARTER`

- **SynthEngine Integration**
  - Sequencer processed each audio callback
  - Sample rate synchronized on stream start
  - Events converted to voice triggers (framework ready)

### Technical Details
- Follows Rust newtype idiom throughout
- Zero primitive type leakage in public API
- Thread-safe song access via `Arc<RwLock<Song>>`
- All 220 unit tests passing

---

## [0.20.0] - 2024

### Refactored - Modular Code Structure

- **GUI Widgets Split** (`src/gui/widgets/`)
  - Split monolithic `widgets.rs` (1032 lines) into 8 focused modules
  - `knob.rs` - Rotary knob widget with response curves
  - `meter.rs` - Audio level meters (peak, RMS, stereo)
  - `port.rs` - Port widget with direction/type enums
  - `cable.rs` - Bezier cable drawing utilities
  - `scope.rs` - Oscilloscope display widget
  - `envelope.rs` - ADSR visualization and interactive editor
  - `waveform.rs` - Waveform selector with visual preview
  - `frame.rs` - Module frame container

- **Parameter System Split** (`src/engine/params/`)
  - Split `typed_params.rs` (1427 lines) into logical groups
  - `oscillators.rs` - `Waveform`, `MathAlgo`, oscillator params
  - `filters.rs` - `FilterMode`, `FilterParam`
  - `envelopes.rs` - `EnvelopeParam`
  - `lfo.rs` - `LfoWaveform`, `LfoParam`
  - `effects.rs` - All effect modes and parameters
  - `modules.rs` - Amplifier, mixer, sample, visualizer params
  - `mod.rs` - `ModuleType`, `TypedParam`, `TypedValue`, `Port`

- **Engine Subsystems Extracted** (`src/engine/`)
  - `metering.rs` - `MeteringSystem` for peak/RMS tracking
  - `effect_chain.rs` - `EffectChain`, `EffectSlot`, `VisualizerSlot`

- **SynthEngine Cleanup** (`src/engine/synth_engine.rs`)
  - Delegates to `EffectChain` and `MeteringSystem`
  - Reduced complexity through composition

### Technical Details
- Backwards compatibility preserved via `pub use params as typed_params`
- All 213 unit tests passing
- Zero functional changes - pure structural refactoring

---

## [0.19.0] - 2024

### Added
- **Additional Audio Types** (`src/types/audio.rs`) - Extended newtype coverage
  - `Tempo` - BPM values with beat duration methods
  - `BufferIndex` - Index for delay lines and circular buffers with wrap/advance
  - `FrameCount` - Sample count with duration conversion
  - `NoiseState` - Xorshift random state (u32) with `next()` method
  - `FilterState` - IIR filter state with `one_pole()` method
  - `Amplitude` - Peak/RMS measurements with `update_peak()` and `decay()`

- **Decibels Extensions** - New methods in `amplitude.rs`
  - `to_linear()` - Convert dB to linear amplitude
  - `from_linear()` - Create dB from linear value

### Refactored
- **Modules with Extended Types** - Consistent type usage across DSP code
  - `oscillator.rs`: Uses `NoiseState` for white/pink noise generation
  - `lfo.rs`: Uses `NoiseState` for sample-and-hold
  - `filter.rs`: Uses `MidiNote` for key tracking, `BipolarValue` for env amount
  - `amplifier.rs` (Mixer): Uses `[Gain; 8]` for channel levels
  - `output.rs`: Uses `Gain`, `BipolarValue`, `Decibels`, `Amplitude` for metering
  - `math_oscillator.rs`: Full type coverage with `Hertz`, `Phase`, `NormalizedValue`, `SampleRate`, `NoiseState`, `BufferIndex`, `FrameCount`

- **Effects with Typed Values** - Type safety for effect parameters
  - `delay.rs`: Uses `Seconds`, `NormalizedValue`, `Hertz`, `SampleRate`, `BufferIndex`, `FilterState`

### Technical Details
- All new types implement `Copy`, `Clone`, `Debug`, `PartialEq`
- `#[repr(transparent)]` for zero-cost abstraction
- Consistent `as_f32()`, `as_usize()`, `as_u32()` accessor methods
- Compile-time prevention of unit mismatches (e.g., can't mix Seconds with Hertz)

---

## [0.18.0] - 2024

### Added
- **Arithmetic Macros** (`src/types/macros.rs`) - Reduce boilerplate for newtypes
  - `impl_additive!` - Add/Sub traits
  - `impl_scaling!` - Mul<f32>/Div<f32> scaling
  - `impl_ratio!` - T / T -> f32 for ratios
  - `impl_float_conversions!` - From<f32> conversions
  - `impl_newtype_arithmetic!` - Combines all above

- **DSP Methods on Types** - Domain-specific audio processing methods
  - `Phase`: `triangle()`, `sawtooth()`, `pulse(width)`, `difference(other)`
  - `Hertz`: `period_samples(sample_rate)`
  - `Gain`: `from_pan(pan)` -> `(Gain, Gain)` constant power panning
  - `Seconds`: `to_exp_coeff(sample_rate)`, `to_samples(sample_rate)`

### Refactored
- **Modules use type methods** - Cleaner DSP code
  - `oscillator.rs`: Uses `Phase::triangle()`, `Phase::sawtooth()`, `Phase::pulse()`
  - `lfo.rs`: Uses `Phase::sin()`, `Phase::triangle()`, etc.
  - `envelope.rs`: Uses `Seconds::to_exp_coeff()` for exponential curves
  - `amplifier.rs`: Uses `Gain::from_pan()` for stereo panning

### Technical Details
- Macros use `#[inline]` for performance
- `frequency.rs` and `time.rs` cleaned up with macro calls
- Removed duplicate coefficient calculation code from envelope
- Removed duplicate pan calculation code from amplifier

---

## [0.17.0] - 2024

### Refactored
- **Voice Architecture** - Moved DSP logic from SynthEngine to Voice
  - New `Voice::process_audio()` method contains complete signal chain
  - `VoiceProcessingBuffers` moved from SynthEngine to Voice struct
  - Each voice now owns its pre-allocated buffers (avoids heap allocations)
  - `SynthEngine::process_voices()` reduced from ~200 to ~70 lines

### Technical Details
- Signal chain in `Voice::process_audio()`: LFO → Oscillators → Filter → Amplifier
- Exposed `glide`, `steal_fade_samples`, `steal_fade_counter` for engine access
- Better encapsulation: Engine orchestrates, Voice processes
- Easier to test voices in isolation and extend with new architectures

---

## [0.16.0] - 2024

### Changed
- **Newtype Pattern in Audio Modules** - Domain-specific types for type safety
  - `oscillator.rs`: frequency→`Hertz`, pulse_width→`NormalizedValue`, phase→`Phase`, detune→`Cents`
  - `envelope.rs`: attack/decay/release→`Seconds`, sustain→`NormalizedValue`, sample_rate→`SampleRate`
  - `filter.rs`: cutoff→`Hertz`, resonance→`NormalizedValue`, key_tracking→`NormalizedValue`
  - `lfo.rs`: rate→`Hertz`, depth→`NormalizedValue`, phase→`Phase`

### Refactored
- **GUI Architecture** - Extracted patch logic to separate module
  - New `patch_bridge.rs` module (~500 lines) for patch load/save logic
  - `egui_backend.rs` reduced from ~1294 to ~872 lines (33% reduction)
  - Better separation of concerns between GUI and engine communication

### Technical Details
- Types from `crate::types`: `Hertz`, `Cents`, `Phase`, `NormalizedValue`, `Seconds`, `SampleRate`
- Key methods: `.as_f32()`, `.advance()`, `.phase_increment()`, `.clamp_audible()`, `.clamp_detune()`
- Constants: `Hertz::A4`, `Phase::ZERO`, `NormalizedValue::CENTER/MAX/MIN`, `SampleRate::DVD_QUALITY`

---

## [0.13.2] - 2024

### Fixed
- **Command Queue Saturation** - Critical fix for patch loading reliability
  - Increased command buffer from 1024 to 16384 entries
  - Added `send_blocking()` method for reliable patch loading
  - Commands no longer silently dropped when buffer is full
- **Real-time Safety** - Modules now dropped on main thread, not audio thread
  - New return channel sends removed modules back to GUI for cleanup
  - Prevents audio dropouts (glitches) during module removal
  - `cleanup_dropped_modules()` called each frame in GUI

### Technical Details
- `COMMAND_BUFFER_SIZE` increased to 16384
- `EngineHandle::send_blocking()` waits for queue space with timeout protection
- `DroppedModule` wrapper and return channel (`RETURN_BUFFER_SIZE: 256`)
- `ModuleGraph::remove_module_and_return()` for deferred cleanup
- `load_patch_data()` uses blocking sends for ClearAllModules, Connect, and settings

---

## [0.13.1] - 2024

### Added
- **Pink Noise** - New waveform for the Oscillator module using Voss-McCartney algorithm
  - Softer, warmer noise with equal energy per octave (-3dB/octave slope)
  - Great for wind effects, percussion, and pads
- **Linear FM Mode** - New FM mode option for Oscillator
  - Exponential: Classic 1V/octave style (pitch tracking)
  - Linear: Hz-based FM for stable harmonic ratios across all pitches
  - Essential for bell-like and metallic FM tones
- **Velocity Sensitivity** - Exposed in Envelope module
  - Control how much note velocity affects envelope amplitude
  - Range 0 (ignore) to 1 (full response)
- **Filter Envelope Amount** - New parameter for Filter module
  - Scale envelope CV from -1.0 to +1.0
  - Enables inverted envelope response for "rubber band" bass sounds

### Technical Details
- `Waveform::PinkNoise` variant added with 16-row Voss-McCartney generator
- `FmMode` enum (Exponential/Linear) with `OscillatorParam::FmMode`
- `EnvelopeParam::VelocitySensitivity` exposed in descriptor
- `FilterParam::EnvAmount` scales `cutoff_cv` input

---

## [0.13.0] - 2024

### Added
- **Math Oscillator Module** - Advanced oscillator with 18 mathematical synthesis algorithms:
  - **Phase-based (Stateless):**
    - SineFM - Basic FM synthesis with carrier/modulator
    - TanChaos - Tan distortion with noise
    - SuperSaw - Multiple detuned sawtooth waves
    - BitWise - Digital glitch/bytebeat style
    - WaveFolder - West coast style wave folding
    - Formant - Vocal formant simulation
    - PhaseDist - Casio CZ style phase distortion
    - Metallic - Ring modulation for metallic tones
    - Fractal - Weierstrass-like fractal function
    - Chebyshev - Chebyshev polynomial waveshaping
    - Walsh - Walsh function synthesis
    - Pulsar - Pulsar synthesis (windowed sine bursts)
    - Shepard - Infinite rising/falling tone illusion
  - **Iterative/Chaotic (Stateful):**
    - Bytebeat - Classic bytebeat formula synthesis
    - Lorenz - Lorenz strange attractor chaos
    - Logistic - Logistic map chaos
    - FeedbackFM - Self-modulating FM
  - **Buffer-based:**
    - KarplusStrong - Physical modeling plucked string

- **GUI Integration** - Math Oscillator available in module palette under Oscillator submenu
- **6 New Example Patches:**
  - Chaos Drone - Evolving textures using Lorenz attractor
  - Karplus Guitar - Physical modeling plucked string
  - Shepard Riser - Infinite rising tone effect
  - Bytebeat Glitch - Retro digital algorithmic music
  - Wave Folder Bass - West coast synthesis bass
  - Formant Voice - Vocal-like synthesis

### Technical Details
- New `MathAlgo` enum in typed_params.rs with 18 algorithm variants
- New `MathOscillatorParam` enum for module parameters
- Internal state management for chaos attractors and delay lines
- CV modulation inputs for Param A and Param B

---

## [0.12.0] - 2024

### Added
- Initial modular synthesizer implementation
- Basic modules: Oscillator, Filter, Envelope, LFO, Amplifier, Mixer
- Effects: Delay, Reverb, Distortion, Chorus, Phaser, Flanger, Compressor, EQ
- Visualizers: Oscilloscope, Level Meter
- Stereo Output module
- Patch save/load system (JSON format)
- 10 example patches:
  - Deep Space Pad
  - Aggressive Bass
  - Vintage Lead
  - Ambient Keys
  - Kick Drum
  - Snare Drum
  - Hi-Hat
  - Pluck Synth
  - FM Bell
  - Noise Sweep
- Piano keyboard for note input
- Module palette for adding modules
- Visual cable connections
- Typed parameter system with compile-time safety
