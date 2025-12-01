# Version History

## [0.32.10] - 2024

### Fixed - Critical Audio Routing Bugs

This release fixes two critical bugs that caused no audio output and "ghost sounds" when switching patches.

#### Bug 1: StereoOutput Not Producing Audio

**Problem:** StereoOutput was classified as a global module, but patches expected it in the voice signal chain. Additionally, StereoOutput had no output ports for Voice to read from, and partial input connections (only `in_l` connected) resulted in silence.

**Solution:**
- `StereoOutput` and `Mixer` are now classified as voice modules in `is_voice_module()`
- Added output ports `"left"`, `"right"`, `"out"` to StereoOutput for Voice to read
- Fixed input handling to support partial connections:
  ```rust
  match (left_in, right_in, mono_in) {
      (Some(l), Some(r), _) => (l[i], r[i]),      // Full stereo
      (Some(l), None, _) => (l[i], l[i]),          // Only left - duplicate
      (None, Some(r), _) => (r[i], r[i]),          // Only right - duplicate
      (None, None, Some(m)) => (m[i], m[i]),       // Mono - duplicate
      (None, None, None) => (0.0, 0.0),            // Silence
  }
  ```

#### Bug 2: Ghost Sounds When Switching Patches

**Problem:** `ClearAllModules` didn't clear `voice_template`, so modules from previous patches remained and played in parallel.

**Solution:** Added `voice_template.clear()` and `rebuild_all_voices()` to `ClearAllModules` handler.

### Changed - Voice Output Detection

- Renamed `Voice::amp_module_id` → `output_module_id`
- Output module detection priority: `StereoOutput` > `Amplifier` > `Mixer`
- Updated `process_audio()` to try port names: `"left"`/`"right"` → `"out_l"`/`"out_r"` → `"out"`

### Fixed - All 20 Patches Updated

All patches were incorrectly routing through effects (Reverb, Delay, Distortion) and oscilloscope in the signal path. Since effects are global modules, these cross-graph connections failed silently.

**Changes to all patches:**
- Removed effects from voice signal chain (effects should use effect chain)
- Connected `amp-1:left/right` directly to `out-1:in_l/in_r`
- Oscilloscope now taps from amplifier for visualization only

### Technical Summary

| File | Changes |
|------|---------|
| `synth_engine.rs` | `is_voice_module()` includes StereoOutput/Mixer, `ClearAllModules` clears template |
| `voice.rs` | `output_module_id` with priority detection, flexible port reading |
| `output.rs` | Added output ports, fixed partial input handling |
| `patches/*.rs` | All 20 patches use direct amp→output stereo routing |

---

## [0.32.9] - 2024

### Added - Dynamic Module Routing

Modules are now automatically routed to the correct graph based on their type:
- **Voice modules** (Oscillator, Filter, Envelope, LFO, Amplifier, etc.) → `voice_template`
- **Global modules** (effects, visualizers, output) → `module_graph`

This ensures voice modules are properly duplicated per voice while effects remain global.

### Changed - Core Architecture

- **`SynthEngine`** (`src/engine/synth_engine.rs`):
  - New `is_voice_module(ModuleType) -> bool` classifies module types
  - New `rebuild_all_voices()` propagates template changes to all voices
  - `AddModuleInstance` routes to `voice_template` or `module_graph` based on type
  - `RemoveModule` checks both graphs, removes from correct one
  - `Connect`/`Disconnect` work correctly across voice and global modules
  - `DisconnectAll` handles both voice template and global graph

### Voice Module Types

The following modules are classified as voice modules (polyphonic):
- `Oscillator`, `MathOscillator`, `SubOscillator`, `Noise`
- `Filter`, `Envelope`, `Lfo`, `Amplifier`

All other module types (effects, visualizers, output) are global.

### Technical Details

- Voice template changes automatically propagate to all active voices
- Comprehensive regression tests for dynamic routing added
- All unit tests passing

---

## [0.32.8] - 2024

### Refactored - Unified Voice/Graph Architecture

Major architectural change: Each `Voice` now owns a `ModuleGraph` instead of a hardcoded list of modules. This eliminates the "architectural schizophrenia" where voices had a hardcoded DSP chain that ignored the dynamic `ModuleGraph` structure.

**Before:**
```rust
// Voice had hardcoded modules and routing
struct Voice {
    modules: Vec<Box<dyn VoiceModule>>,
    module_names: Vec<String>,
    processing_buffers: VoiceProcessingBuffers,  // Hardcoded buffer names
}

// process_audio() had explicit LFO→Osc→Filter→Amp routing
voice.process_audio(left, right, context);
```

**After:**
```rust
// Voice owns a ModuleGraph - fully dynamic routing
struct Voice {
    pub graph: ModuleGraph,  // User-configurable signal chain
    amp_module_id: Option<ModuleId>,  // Cached for stereo output
}

// Voice delegates to graph for processing
voice.process_audio(left, right, context);  // Internally uses graph.process()
```

### Changed - Core Architecture

- **`Voice`** (`src/engine/voice.rs`):
  - Replaced `modules`, `module_names`, `buffers`, `module_outputs`, `processing_buffers` with `graph: ModuleGraph`
  - New `from_graph(id, graph)` constructor
  - `set_param(module_id, param)` delegates to graph
  - `note_on()`, `note_off()`, `reset()` delegate to graph
  - `process_audio()` processes graph then extracts stereo from Amplifier

- **`ModuleGraph`** (`src/engine/graph.rs`):
  - New `get_module_output(module_id, port_name)` for stereo extraction
  - New `find_module_by_type(ModuleType)` for locating Amplifier
  - New `set_oscillator_frequency(Hertz)` for voice pitch injection

- **`VoiceAllocator`** (`src/engine/voice_allocator.rs`):
  - New `with_graph_template(config, &ModuleGraph)` constructor
  - New `rebuild_from_graph(&ModuleGraph)` for hot-reloading template

- **`SynthEngine`** (`src/engine/synth_engine.rs`):
  - Removed `create_template_voice()` function (no more hardcoded Voice)
  - `voice_template: ModuleGraph` now used directly
  - `SetVoiceParameter` updates both template and all active voices

### Benefits

- **True modularity**: Users can now dynamically configure voice signal routing
- **Single source of truth**: Template graph defines both structure and parameters
- **Real-time updates**: Parameter changes apply to template and all voices
- **Cleaner code**: ~400 lines of hardcoded routing removed

### Technical Details

- All 239 unit tests passing
- Stereo output extracted from Amplifier's "left"/"right" ports
- Velocity scaling applied after graph processing
- Pitch bend applied before graph processing via `set_oscillator_frequency()`

---

## [0.32.7] - 2024

### Refactored - Unified Param Architecture

Major refactoring of the parameter system. Replaced the dual-type system (`TypedParam` for ID + `TypedValue` for value) with a single unified `Param` type where each variant carries its own typed value.

**Before:**
```rust
// Two separate types - risk of mismatch
set_param(TypedParam::Oscillator(OscillatorParam::Frequency), TypedValue::Float(440.0))
```

**After:**
```rust
// Single type carries both identity and value
set_param(Param::Oscillator(OscillatorParam::Frequency(Hertz::new(440.0))))
```

### Changed - Core Architecture

- **`Param` enum** - Now data-carrying with typed values:
  - `OscillatorParam::Frequency(Hertz)` instead of just `Frequency`
  - `FilterParam::Cutoff(Hertz)`, `Resonance(NormalizedValue)`, `Drive(Gain)`
  - `EnvelopeParam::Attack(Seconds)`, `Sustain(NormalizedValue)`
  - All effect params similarly carry their domain types

- **`VoiceModule` trait** (`src/modules/core.rs`):
  - `set_param(Param)` - single argument, value embedded
  - `get_param(&Param) -> Option<f32>` - query by param kind

- **`EngineCommand`** (`src/engine/commands.rs`):
  - `SetVoiceParameter { target, param: Param }` - no separate `value` field
  - `SetModuleParameter { module_id, param: Param }`
  - `SetEffectParameter { effect_type, param: Param }`

- **GUI** (`src/gui/`):
  - `RackViewResult.param_changes: Vec<(ModuleId, Param)>`
  - `ModulePanelState.param_values: HashMap<String, f32>` (keyed by name)
  - `param.id.with_f32(value)` creates new Param with updated value

### Removed - Dead Code Cleanup

- **Deleted `traits.rs`** (~640 lines) - Unused `ModuleParam` trait and `Query` enums
- **Removed type aliases** - `TypedWaveform`, `TypedLfoWaveform` (never used)
- **Updated comments** - Removed references to old `TypedValue` system

### Technical Details

- Type safety: Impossible to send wrong value type to parameter
- Domain types: `Hertz`, `Gain`, `Seconds`, `NormalizedValue` enforced at compile time
- `same_kind()` method for comparing param types ignoring values
- `as_f32()` / `with_f32()` for GUI slider compatibility
- All 238 unit tests passing

---

## [0.32.6] - 2024

### Fixed - Dropdown Parameter Synchronization

Fixed GUI dropdown parameters not updating the synth engine. The issue was that dropdown handlers sent `TypedValue::Int(index)` but modules expected specific typed variants.

- **rack_view.rs** - Added proper type conversion for all dropdown parameters:
  - `MathOscillatorParam::Algorithm` → `TypedValue::MathAlgo`
  - `FilterParam::Mode` → `TypedValue::FilterMode`
  - `DelayParam::Mode` → `TypedValue::DelayMode`
  - `DistortionParam::Mode` → `TypedValue::DistortionMode`

### Modules Verified

All modules analyzed and verified for correct GUI/Engine synchronization:

| Module | Parameter | TypedValue | Status |
|--------|-----------|------------|--------|
| MathOscillator | Algorithm | `MathAlgo` | Fixed |
| Filter | Mode | `FilterMode` | Fixed |
| Delay | Mode | `DelayMode` | Fixed |
| Distortion | Mode | `DistortionMode` | Fixed |
| Oscillator | Waveform | `Waveform` | OK (WaveformSelector) |
| LFO | Waveform | `LfoWaveform` | OK (WaveformSelector) |
| SubOsc | Waveform/Octave | `Int` | OK (uses as_int()) |
| NoiseGenerator | Type | `Int` | OK (uses as_int()) |
| Oscillator | FmMode | `Int` | OK (uses as_int()) |

### Technical Details

- Added imports: `FilterParam`, `DelayParam`, `DistortionParam`, `MathOscillatorParam`
- Added imports: `FilterMode`, `DelayMode`, `DistortionMode`, `MathAlgo`
- All 240 unit tests passing

---

## [0.32.5] - 2024

### Added - New Domain Types for Effects

Extended newtype pattern coverage with three new audio domain types:

- **`Ratio`** (`src/types/amplitude.rs`)
  - Type-safe compression ratio (1:1 to 20:1)
  - Constants: `UNITY`, `LIGHT`, `MEDIUM`, `HEAVY`, `LIMITING`
  - Methods: `compress(overshoot_db)`, `clamp_typical()`, `to_ratio_string()`
  - Display format: "4.0:1", "∞:1"

- **`BeatDivision`** (`src/types/time.rs`)
  - Type-safe tempo-synced note divisions
  - Constants: `THIRTY_SECOND`, `SIXTEENTH`, `EIGHTH`, `QUARTER`, `HALF`, `WHOLE`
  - Dotted/triplet variants: `DOTTED_QUARTER`, `DOTTED_EIGHTH`, `TRIPLET_QUARTER`
  - Methods: `to_duration(Bpm)`, `to_samples(Bpm, SampleRate)`, `dotted()`, `triplet()`, `name()`
  - Display: "1/4", "1/8.", "1/16", etc.

- **`VoiceCount`** (`src/types/audio.rs`)
  - Type-safe voice/polyphony count (u8)
  - Constants: `MONO`, `DUAL`, `QUAD`, `OCTO`, `SIXTEEN`
  - Methods: `clamp_chorus()` (1-4), `clamp_polyphony()` (1-16)

### Refactored - Effect Modules with Strong Types

- **Compressor** (`src/effects/compressor.rs`)
  - `ratio: Ratio` (was `f32`)
  - `attack: Milliseconds` (was `f32`)
  - `release: Milliseconds` (was `f32`)
  - Uses `Ratio::compress()` for gain calculation

- **Delay** (`src/effects/delay.rs`)
  - `sync_division: BeatDivision` (was `f32`)
  - Uses `BeatDivision::to_duration(Bpm)` for tempo-synced delay time

- **Chorus** (`src/effects/distortion.rs`)
  - `voices: VoiceCount` (was `u32`)
  - Uses `VoiceCount::clamp_chorus()` for voice count validation

### Technical Details

- All new types implement `Copy`, `Clone`, `Debug`, `PartialEq`, `Display`
- `#[repr(transparent)]` for zero-cost abstraction
- Consistent `as_f32()`, `as_u8()`, `as_usize()` accessor methods
- All 240 unit tests passing

---

## [0.32.4] - 2024

### Fixed - Waveform Selection Bug

- **Waveform changes now update engine** (`src/gui/rack_view.rs`)
  - GUI was sending `TypedValue::Int(index)` but modules expected `TypedValue::Waveform`
  - Added proper conversion from index to typed waveform value
  - `OscillatorParam::Waveform` → `TypedValue::Waveform(Waveform::from_index(...))`
  - `LfoParam::Waveform` → `TypedValue::LfoWaveform(LfoWaveform::from_index(...))`

### Removed - Noise Waveforms from Oscillator

Noise generation has been moved to the dedicated `NoiseGenerator` module.

- **Waveform enum** (`src/engine/params/oscillators.rs`)
  - Removed `Noise` and `PinkNoise` variants
  - `Waveform::ALL` now has 5 elements (was 7)
  - `from_id()` maps legacy `"noise"` and `"pink_noise"` to `Sine` for backward compatibility

- **Oscillator module** (`src/modules/oscillator.rs`)
  - Removed `pink_rows`, `pink_running_sum`, `pink_index` fields
  - Removed `white_noise()` and `pink_noise()` methods
  - Removed `Waveform::Noise` and `Waveform::PinkNoise` match arms
  - Simplified `reset()` method

- **GUI widget** (`src/gui/widgets/waveform.rs`)
  - Removed `Noise` and `PinkNoise` from `WaveformType` enum
  - `WaveformType::all()` now returns 5 waveforms

- **Patch loading** (`src/gui/patch_bridge.rs`)
  - Legacy patches with noise waveforms map to `Sine`

### Technical Details

- All 235 unit tests passing
- Use `NoiseGenerator` module for noise (white, pink, brown noise types)
- No breaking changes for existing patches (graceful fallback)

---

## [0.32.3] - 2024

### Fixed - CV Modulation Parameter Drift Bugs

Critical fixes for parameters that were permanently modified during CV modulation instead of using effective values.

- **LadderFilter cutoff drift** (`src/modules/filter.rs`)
  - `process_sample()` now takes `effective_cutoff: Hertz` parameter
  - CV modulation calculates effective cutoff without modifying `self.cutoff`
  - Prevents filter cutoff from drifting away from user-set value

- **LFO rate drift** (`src/modules/lfo.rs`)
  - `generate_sample()` now takes `effective_rate: Hertz` parameter
  - CV modulation and tempo sync calculated in `process()` without modifying `self.rate`
  - Prevents LFO rate from drifting during modulation

- **Oscillator PWM drift** (`src/modules/oscillator.rs`)
  - `generate_sample()` now takes `effective_pulse_width: NormalizedValue` parameter
  - PWM modulation uses local effective value instead of modifying `self.pulse_width`
  - Prevents pulse width from drifting away from user-set value

### Fixed - Startup Sound Issue

- **Re-enable part after patch load** (`src/gui/patch_bridge.rs`)
  - Added `SetPartEnabled { part_id: FIRST, enabled: true }` after loading patch
  - Fixes "no sound on first startup" caused by ClearAllModules disabling parts

### Changed - Spacey Bass Patch

- **Moved to patches module** (`src/patches/spacey_bass.rs`)
  - Created proper patch file following project conventions
  - Removed inline `create_startup_patch()` from `egui_backend.rs`
  - Startup now loads `crate::patches::patch_spacey_bass()`
  - Added to `example_patches()` as first (default) patch

### Technical Details

- All 235 unit tests passing
- Fixed tests for new function signatures in LFO and Oscillator
- No breaking changes to public API

---

## [0.32.2] - 2024

### Fixed - GUI/Engine Synchronization at Startup

- **Startup patch system** (`src/gui/egui_backend.rs`)
  - Added `create_startup_patch()` function that builds the default synth programmatically
  - `SynthApp::new` now uses `patch_bridge::load_patch()` instead of manual GUI initialization
  - Ensures GUI and Engine are 100% synchronized from the first millisecond
  - Prevents "ghost state" where GUI and Engine had different module configurations

- **Removed manual initialization**
  - Removed all `rack_view.add_module()`, `set_parameter()`, and `add_connection()` calls
  - Now uses the same robust patch loading flow as when opening a file

### Technical Details

- All 235 unit tests passing
- No breaking changes to public API

---

## [0.32.1] - 2024

### Fixed - Ghost Sound Bug

- **ClearAllModules now disables parts** (`src/engine/synth_engine.rs`)
  - Parts are now disabled when `ClearAllModules` is sent
  - Prevents "ghost sound" from hardcoded polyphonic voices after clearing rack
  - Clean slate for "New Patch" - no residual audio from previous state

### Technical Details

- All 235 unit tests passing

---

## [0.32.0] - 2024

### Added - Type Safety Refactoring & GUI Improvements

- **Type-Safe Public APIs** (New Type Idiom compliance)
  - `Voice::set_glide_time(Seconds)` - glide time uses `Seconds` type
  - `Voice::set_oscillator_detune(Cents)` - detune uses `Cents` type
  - `Voice::set_oscillator_frequency(Hertz)` - frequency uses `Hertz` type
  - `VoiceAllocator`: `glide_time: Seconds`, `unison_detune: Cents`
  - `SynthEngineHandle::set_master_volume(Gain)` - volume uses `Gain` type
  - `SynthEngineHandle::note_on(u8, NormalizedValue)` - velocity uses `NormalizedValue`
  - `Flanger::delay_base: Seconds` - delay time uses `Seconds` type

- **New Pitch Type Operations** (`src/types/pitch.rs`)
  - Added `impl Div<f32> for Cents` for unison spread calculations

- **GUI: New Patch Feature** (`src/gui/egui_backend.rs`)
  - Added "📄 New Patch" menu item in File menu
  - `reset_to_new_patch()` method clears all modules and resets state
  - Sends `ClearAllModules` command to engine
  - Automatically adds default StereoOutput module

- **UI Type Safety** (`src/ui/mod.rs`)
  - `UiEvent::SetMasterVolume(Gain)` - type-safe volume events
  - `UiEvent::NoteOn { velocity: NormalizedValue }` - type-safe velocity
  - `UiAdapter::set_master_volume(Gain)` - type-safe API

### Technical Details

- All 235 unit tests passing
- No breaking changes to internal audio processing (VoiceModule trait unchanged)
- Parameter descriptors still use f32 for GUI compatibility (conversion at boundaries)

---

## [0.31.0] - 2024

### Added - GUI Module Support and New Patches

- **GUI Support for New Modules** (`src/gui/egui_backend.rs`, `src/gui/rack_view.rs`)
  - Added SubOscillator to module palette menu ("🔈 Sub" in Oscillators)
  - Added NoiseGenerator to module palette menu ("🌫 Noise" in Oscillators)
  - New `PaletteSelection` variants: `SubOscillator`, `Noise`
  - Methods: `add_sub_oscillator_module()`, `add_noise_module()`

- **Patch System Updates** (`src/patch.rs`)
  - Added `SubOscillator` and `Noise` to `ModuleType` enum
  - Module prefixes: `sub` (sub-osc), `nse` (noise)
  - Full serialization support for new modules

- **Patch Bridge** (`src/gui/patch_bridge.rs`)
  - Handlers for loading `SubOscillator` and `Noise` modules from patches

- **Updated Example Patches** (using new DSP features)
  - `drum_kick.rs`: Punchy envelope curves (-0.8 to -1.0)
  - `drum_snare.rs`: NoiseGenerator (white) + punchy curves
  - `drum_hihat.rs`: NoiseGenerator (white) + punchy curves
  - `aggressive_bass.rs`: SubOscillator (sine, -1 oct) + punchy curves
  - `noise_sweep.rs`: NoiseGenerator (pink) for natural sweep

- **New Example Patches** (3 new patches)
  - `sub_bass.rs`: Deep bass showcasing SubOscillator module
  - `brown_drone.rs`: Dark ambient drone using brown noise
  - `punchy_stab.rs`: Aggressive synth stab demonstrating envelope curves

### Technical Details

- Total example patches: 19 (was 16)
- All 235 unit tests passing

---

## [0.30.0] - 2024

### Added - DSP Improvements for Sound Quality

- **Envelope Curves** (`src/modules/envelope.rs`)
  - New per-stage curve parameters: `attack_curve`, `decay_curve`, `release_curve`
  - Type: `BipolarValue` (-1.0 to +1.0)
  - Negative values = logarithmic/punchy (fast attack, snappy drums)
  - Positive values = gradual/slow (natural fades)
  - Zero = standard exponential (backwards compatible)
  - Modifies exponential coefficients for precise control

- **Sub-Oscillator Module** (`src/modules/sub_osc.rs`) - NEW
  - Dedicated bass reinforcement oscillator
  - Waveforms: Sine, Square, Pulse25
  - Octave transposition: -1 or -2
  - Parameters: `SubOscParam::Waveform`, `Octave`, `Level`
  - Newtypes: Uses `Hertz`, `Phase`, `Gain`, `SampleRate`
  - Module prefix: `sub`

- **Noise Generator Module** (`src/modules/noise.rs`) - NEW
  - Spectral colored noise for textures and percussion
  - Noise colors:
    - White: Flat spectrum (crisp hi-hats, snares)
    - Pink: -3dB/octave (natural, cymbals, atmosphere)
    - Brown: -6dB/octave (dark rumble, thunder)
    - Blue: +3dB/octave (bright, hissing)
    - Violet: +6dB/octave (very bright, sharp)
  - Pink noise: Voss-McCartney algorithm
  - Brown noise: Leaky integrator
  - Blue/Violet: Differentiator filters
  - Parameters: `NoiseParam::Type`, `Level`
  - Module prefix: `nse`

### Technical Details

- **Type System Updates** (`src/engine/params/`)
  - New `SubOscParam` enum in `sub_osc.rs`
  - New `NoiseParam` enum in `noise.rs`
  - Extended `ModuleType`: `SubOscillator`, `Noise`
  - Extended `TypedParam`: `SubOsc(SubOscParam)`, `Noise(NoiseParam)`

- **Module Exports** (`src/modules/mod.rs`)
  - Exports: `SubOscillator`, `SubOscWaveform`, `SubOscOctave`
  - Exports: `NoiseGenerator`, `NoiseType`

- All 235 unit tests passing

---

## [0.29.0] - 2024

### Refactored - Modular Patch Structure

- **Patch Directory Structure** (`src/patches/`)
  - Extracted all 16 example patches from `patch.rs` to individual files
  - New modular structure with one patch per file
  - Central `mod.rs` with re-exports and `example_patches()` function

- **Patch Files Created:**
  - `deep_space_pad.rs`, `aggressive_bass.rs`, `vintage_lead.rs`, `ambient_keys.rs`
  - `drum_kick.rs`, `drum_snare.rs`, `drum_hihat.rs`
  - `pluck_synth.rs`, `fm_bell.rs`, `noise_sweep.rs`
  - `chaos_drone.rs`, `karplus_guitar.rs`, `shepard_riser.rs`
  - `bytebeat_glitch.rs`, `wave_folder_bass.rs`, `formant_voice.rs`

- **Patch Routing Fixed**
  - Updated 6 math oscillator patches with missing Oscilloscope visualization
  - All patches now end with: `[Effect] → Oscilloscope → Stereo Output`
  - Consistent signal flow across all example patches

- **Code Cleanup** (`src/patch.rs`)
  - Reduced from ~2163 lines to 367 lines
  - Kept core types: `Patch`, `ModuleState`, `ModuleType`, `ParamValue`, `ConnectionState`, `PatchSettings`, `PatchError`, `ModuleBuilder`
  - Added re-export: `pub use crate::patches::example_patches;`

### Technical Details
- Pattern: Each patch file uses `use crate::patch::{Patch, ModuleBuilder, ModuleType};`
- All patches use fluent `ModuleBuilder` API
- Backwards compatible: `example_patches()` still returns `Vec<Patch>`
- All 229 unit tests passing

---

## [0.28.1] - 2024

### Added - Performance Fixes & Module Visualization

- **Performance Panel Velocity Mapping** (`src/engine/commands.rs`, `src/engine/part.rs`)
  - Added `PartParam::VelocityAmpSensitivity(NormalizedValue)` command
  - Added `PartParam::VelocityFilterSensitivity(NormalizedValue)` command
  - Connected GUI knobs to engine via `SetPartParameter` commands
  - Velocity settings now propagate to all voices in part

- **Tempo Sync for Delay** (`src/effects/delay.rs`)
  - Added `tempo_sync: bool` and `sync_division: f32` fields
  - New `synced_delay_time()` method calculates delay from BPM
  - Formula: `delay_seconds = (60.0 / bpm) * sync_division`
  - Added `DelayParam::SyncDivision` parameter variant

- **Tempo Sync for LFO** (`src/modules/lfo.rs`, `src/engine/params/lfo.rs`)
  - Added `LfoParam::SyncDivision` parameter variant
  - LFO tempo sync logic now configurable via parameters
  - Sync divisions in beats (0.25 = 1/16, 1.0 = 1/4, etc.)

- **Envelope Curves** (`src/modules/envelope.rs`)
  - Added `attack_curve`, `decay_curve`, `release_curve` fields (-1.0 to 1.0)
  - New `apply_curve(x, curve)` function for shaping
  - Formula: `x^(1 + curve*3)` for exponential, `x^(1/(1 - curve*3))` for logarithmic
  - Negative = logarithmic (fast start), Positive = exponential (slow start)

- **Module Connectivity Visualization** (`src/gui/rack_view.rs`)
  - New `ModuleConnectivity` enum: `Connected`, `Orphaned`, `Disconnected`
  - `calculate_connectivity()` uses BFS backwards from output modules
  - Visual dimming based on connectivity status:
    - **Connected**: Full opacity (1.0), green indicator ●
    - **Orphaned**: 60% opacity, yellow indicator ○ (has connections but not routed)
    - **Disconnected**: 40% opacity, gray indicator ○ (no connections)
  - Recalculates on module/connection add/remove

### Technical Details
- Uses type-safe `NormalizedValue` for velocity sensitivity
- BFS traversal builds reverse adjacency map for connectivity analysis
- Frame opacity applied via `gamma_multiply(opacity)` on fill and stroke
- All 229 unit tests passing

---

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
