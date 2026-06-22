# Plan: Synth Modules and Audio Effects Critical Review

This plan details a systematic, critical code review of all synthesizer modules and audio effects in the Pertylizer codebase. The goal is to evaluate implementation quality, identify bugs, verify real-time safety, enforce domain-specific newtype patterns, and suggest optimization or architectural improvements.

---

## 1. Review Objectives & Criteria

Each module will be audited against the following criteria:

### A. Real-Time Safety (Audio Thread)
* **No Heap Allocations:** Ensure `process()` loops never call `Vec::push`, `Box::new`, `String::clone`, or do any operations triggering heap allocations.
* **No Blocking Locks:** Ensure no use of `Mutex::lock`, `RwLock::write`, or `parking_lot` blocking primitives. Only lock-free structures, atomics, or `try_read()` are allowed.
* **No Panics:** Check that all array indexing is bounds-checked or safe. Zero occurrences of `.unwrap()` or `.expect()` in the audio processing path (use `unwrap_or`, `?`, or default values instead).
* **No System Calls:** No file I/O, networking, console prints, or standard logging (e.g., `println!`, `info!`, `error!`) in the real-time processing loop.

### B. Type Safety (Newtype Pattern)
* **Domain Values:** Verify that raw primitives (`f32`, `f64`, `usize`, etc.) are wrapped in domain-specific newtypes (e.g., [Hertz](file:///home/per/github/pertylizer/crates/synth_core/src/lib.rs), [Decibels](file:///home/per/github/pertylizer/crates/synth_core/src/lib.rs), [NormalizedValue](file:///home/per/github/pertylizer/crates/synth_core/src/lib.rs), [BipolarValue](file:///home/per/github/pertylizer/crates/synth_core/src/lib.rs), [SampleRate](file:///home/per/github/pertylizer/crates/synth_core/src/lib.rs)) for parameters.
* **No Raw Primitives:** Ensure raw primitives are restricted solely to loop counters, intermediate arithmetic, and serialization boundaries.

### C. DSP Correctness & Quality
* **Anti-Aliasing:** Verify quality of band-limiting (e.g., BLEP/PolyBLEP) in oscillators.
* **Parameter Smoothing:** Ensure parameter changes are interpolated/smoothed to prevent clicks, pops, or digital zipper noise.
* **Sample Rate Independence:** Verify that all timing-related parameters (frequencies, filter cutoffs, envelope times) scale correctly with changes in `SampleRate`.
* **Filter Stability:** Check that biquad and ladder filter coefficients are correctly bounded to avoid feedback loops or output blowups (NaN/Infinity).

### D. Code Quality & Style
* **Self Usage:** Ensure `impl` blocks consistently reference `Self` instead of the explicit struct name.
* **No Unsafe:** Zero `unsafe` blocks without explicit prior discussion and approval.
* **Clippy compliance:** Ensure it builds with zero warnings under `cargo clippy --all-targets`.

---

## 2. Review Progress Dashboard

| Category | Total Modules/Files | Pending Review | Passed | Improvements Identified |
|---|---|---|---|---|
| **Core Voice & Oscillators** | 19 | 0 | 1 | 18 |
| **Modulators & Generators** | 8 | 0 | 4 | 4 |
| **Physical Modeling & Acoustic** | 4 | 0 | 1 | 3 |
| **Routing & Utility** | 8 | 0 | 7 | 1 |
| **Helpers & Core Infrastructure** | 9 | 0 | 4 | 5 |
| **Audio Effects** | 26 | 0 | 16 | 10 |
| **Total** | **74** | **0** | **33** | **41** |

---

## 3. Detailed Review Tables

### 3.1 Core Voice & Oscillators

These modules form the fundamental sound generation and envelope/filtering engine of a voice.

| Module/Struct Name | Source Code File | Review Status | Notes & Identified Issues |
|---|---|---|---|
| [AdditiveOsc](file:///home/per/github/pertylizer/crates/synth_modules/src/additive_osc.rs) | [additive_osc.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/additive_osc.rs) | ⚠️ Attention | Pitch CV tracking restricted to ±1 octave due to `BipolarValue::new` clamping on input buffer. Potential NaN panics in standard library `clamp` if inputs contain NaN. |
| [AmFormant](file:///home/per/github/pertylizer/crates/synth_modules/src/am_formant.rs) | [am_formant.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/am_formant.rs) | ⚠️ Attention | High `carrier_ratio` modulation offset can cause `ratio` to explode to infinity, producing `NaN` phases which propagate downstream and may cause clamps to panic. |
| [ChaoticOsc](file:///home/per/github/pertylizer/crates/synth_modules/src/chaotic_osc.rs) | [chaotic_osc.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/chaotic_osc.rs) | ❌ Needs Fix | **Real-Time Safety Violation:** `PortName::intern("out_y")` called every block in `process()` (acquires standard RwLock::write). Fix by interning once during `new()` and storing it in a struct field. |
| [Envelope](file:///home/per/github/pertylizer/crates/synth_modules/src/envelope.rs) | [envelope.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/envelope.rs) | ⚠️ Attention | If `sustain` is dynamically modulated to a level higher than the current value during `Decay`, it triggers an instant jump to `Sustain` and causes level jumps/clicks. |
| [Filter](file:///home/per/github/pertylizer/crates/synth_modules/src/filter.rs#L349) | [filter.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/filter.rs) | ❌ Needs Fix | **Real-Time Safety Violation:** `PortName::intern()` called inside the `process()` loop for custom CV ports. Fix by caching these in the struct at instantiation, and use predefined constants like `PortName::GATE` or `PortName::PITCH_CV`. |
| [LadderFilter](file:///home/per/github/pertylizer/crates/synth_modules/src/filter.rs#L704) | [filter.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/filter.rs) | ⚠️ Attention | Filter stabilizes only when `drive > 1.0` (it skips soft clipping at lower drive levels), making it prone to feedback overflow/NaN at high resonance. Modulated `cutoff_cv` can propagate NaN to standard clamp and cause panic. |
| [Fof](file:///home/per/github/pertylizer/crates/synth_modules/src/fof.rs) | [fof.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/fof.rs) | ❌ Needs Fix | **Real-Time Safety Violation:** `PortName::intern()` called every block in `process()` for `pitch_cv`, `vowel_cv`, and `breath_cv`. Fix by caching them in struct fields or using pre-defined constants. |
| [FormantFilter](file:///home/per/github/pertylizer/crates/synth_modules/src/formant_filter.rs) | [formant_filter.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/formant_filter.rs) | ❌ Needs Fix | **Real-Time Safety Violation:** `PortName::intern("vowel_cv")` called every block in `process()`. Fix by caching in a struct field. |
| [FractalOscillator](file:///home/per/github/pertylizer/crates/synth_modules/src/fractal_osc.rs) | [fractal_osc.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/fractal_osc.rs) | ⚠️ Attention | Pitch CV tracking restricted to ±1 octave due to `BipolarValue::new` clamping on input buffer. |
| [GranularOsc](file:///home/per/github/pertylizer/crates/synth_modules/src/granular_osc.rs) | [granular_osc.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/granular_osc.rs) | ⚠️ Attention | Heavy `fill_source_buffer()` (loop of 96,000 iterations containing sinusoids) triggered on the audio thread upon sample rate or source waveform change, causing audio glitches. Fix by pre-calculating/caching waves at start and swapping read pointers atomically. |
| [LaSynth](file:///home/per/github/pertylizer/crates/synth_modules/src/la_synth.rs) | [la_synth.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/la_synth.rs) | ⚠️ Attention | Instantaneous frequency sweep phase logic is mathematically incorrect for variable frequency (sweep depth is doubled). Brightness filter coefficient is sample-rate dependent. |
| [MathOscillator](file:///home/per/github/pertylizer/crates/synth_modules/src/math_oscillator.rs) | [math_oscillator.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/math_oscillator.rs) | ⚠️ Attention | FM modulation depth restricted to ±2 octaves due to `BipolarValue::new` clamping on FM input. Otherwise structurally clean and real-time safe. |
| [NoiseGenerator](file:///home/per/github/pertylizer/crates/synth_modules/src/noise.rs) | [noise.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/noise.rs) | ⚠️ Attention | Brown noise leaky integrator has fixed z-pole coefficient at `0.99`, causing the low-frequency roll-off (leak speed) to be sample-rate dependent (thins low-end at 96kHz). |
| [Oscillator](file:///home/per/github/pertylizer/crates/synth_modules/src/oscillator.rs) | [oscillator.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/oscillator.rs) | ⚠️ Attention | FM/PM inputs wrapped in `BipolarValue::new` which limits modulation depth. Otherwise, highly robust with accurate sub-sample phase reset for hard sync. |
| [PadSynth](file:///home/per/github/pertylizer/crates/synth_modules/src/padsynth.rs) | [padsynth.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/padsynth.rs) | ❌ Needs Fix | **Real-Time Safety Violation:** Calls O(N^2) brute-force DFT `build_wavetable` inside `note_on()` which executes on the real-time audio thread, causing catastrophic CPU spikes (~2.1M loop iterations with trig math) and audio dropouts on note triggers. |
| [Sampler](file:///home/per/github/pertylizer/crates/synth_modules/src/sampler.rs) | [sampler.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/sampler.rs) | ⚠️ Attention | Modulation parameters (like `start_offset`) are clamped inside `SamplePlayer` but lack explicit NaN guards before `clamp()`, which could cause panics if dynamic CV inputs produce NaN. |
| [SubOscillator](file:///home/per/github/pertylizer/crates/synth_modules/src/sub_osc.rs) | [sub_osc.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/sub_osc.rs) | Passed | Structurally clean and real-time safe. Naive waveform shapes are used (except `DsfSaw` which is band-limited). Pitch is updated only at `note_on` and lacks continuous modulation/FM CV. |
| [VoiceSynth](file:///home/per/github/pertylizer/crates/synth_modules/src/voice_synth.rs) | [voice_synth.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/voice_synth.rs) | ❌ Needs Fix | **Real-Time Safety & DSP Bugs:** Calls `PortName::intern()` inside `process()` for custom CV ports. Modulating `open_quotient` to negative values causes division-by-zero (`phase / oq`) and NaNs. Triggers hyperbolic/trig coefficient recalculations inside the sample loop for all active unison voices if `vowel_cv` is modulated. |
| [WavetableOsc](file:///home/per/github/pertylizer/crates/synth_modules/src/wavetable_osc.rs) | [wavetable_osc.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/wavetable_osc.rs) | ⚠️ Attention | FM CV input is wrapped in `BipolarValue::new`, restricting pitch modulation depth to exactly ±1 octave, limiting wide vibrato/sweeps. |

---

### 3.2 Modulators & Generators

Control-rate generators, modulators, and sequencing helper engines.

| Module/Struct Name | Source Code File | Review Status | Notes & Identified Issues |
|---|---|---|---|
| [DriftGenerator](file:///home/per/github/pertylizer/crates/synth_modules/src/drift_generator.rs) | [drift_generator.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/drift_generator.rs) | Passed | Structurally clean and real-time safe. Uses `fastrand` for RT-safe pseudo-random walk. |
| [EnvelopeFollower](file:///home/per/github/pertylizer/crates/synth_modules/src/envelope_follower.rs) | [envelope_follower.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/envelope_follower.rs) | ⚠️ Attention | Output is clamped to `min(1.0)` but lacks lower bound clamp to `0.0`, which can result in negative control signals if input sensitivity is modulated negatively. |
| [Euclidean](file:///home/per/github/pertylizer/crates/synth_modules/src/euclidean.rs) | [euclidean.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/euclidean.rs) | ⚠️ Attention | If `swing` is modulated to a negative value such that swing offset exceeds `samples_per_step`, the step threshold can become `<= 0.0`, causing the clock to trigger on every sample and spin step index rapidly. |
| [KineticModulator](file:///home/per/github/pertylizer/crates/synth_modules/src/kinetic_modulator.rs) | [kinetic_modulator.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/kinetic_modulator.rs) | Passed | Stateless and safe physics-simulation modulator. Bounded outputs, no allocations. |
| [Lfo](file:///home/per/github/pertylizer/crates/synth_modules/src/lfo.rs) | [lfo.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/lfo.rs) | ⚠️ Attention | FM input wrapped in `BipolarValue::new`, limiting LFO frequency modulation depth to exactly ±1 octave. |
| [Mseg](file:///home/per/github/pertylizer/crates/synth_modules/src/mseg.rs) | [mseg.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/mseg.rs) | Passed | Multi-stage envelope generator. Indices and binary searches are safely bounded. No allocations. |
| [RandomGates](file:///home/per/github/pertylizer/crates/synth_modules/src/random_gates.rs) | [random_gates.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/random_gates.rs) | Passed | Real-time safe. Simple coin-toss logic utilizing `fastrand` upon clock trigger. |
| [TuringMachine](file:///home/per/github/pertylizer/crates/synth_modules/src/turing_machine.rs) | [turing_machine.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/turing_machine.rs) | ⚠️ Attention | `current_cv` output is typed as a `NormalizedValue`, which clamps the output pitch CV to `[0.0, 1.0]` (exactly 1.0 octave), preventing the intended range of "up to 2 octaves". |

---

### 3.3 Physical Modeling & Acoustic Simulation

Modules simulating acoustic bodies, resonance systems, vocal mechanics, and physical keyboards.

| Module/Struct Name | Source Code File | Review Status | Notes & Identified Issues |
|---|---|---|---|
| [BodyResonance](file:///home/per/github/pertylizer/crates/synth_modules/src/body_resonance.rs) | [body_resonance.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/body_resonance.rs) | ❌ Needs Fix | **DSP Math / Stability Bug:** Calls `math::biquad_omega(freq, sample_rate)` which does not clamp frequency on the lower bound. A negative modulated frequency produces a negative `sin_w0`, which makes `alpha` negative, causing `a0 = 1.0 + alpha` to be zero or negative, resulting in division by zero and filter state explosion (NaNs). |
| [KeyboardPanner](file:///home/per/github/pertylizer/crates/synth_modules/src/keyboard_panner.rs) | [keyboard_panner.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/keyboard_panner.rs) | Passed | Bounded panning, real-time safe, and no allocations in process loop. |
| [MechanicalNoise](file:///home/per/github/pertylizer/crates/synth_modules/src/mechanical_noise.rs) | [mechanical_noise.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/mechanical_noise.rs) | ❌ Needs Fix | **DSP Math / Stability Bug:** The lowpass filter coefficient `alpha = cutoff_norm` is not clamped on the lower bound. A negative modulated `cutoff` causes negative `alpha`, making `1.0 - alpha > 1.0` (unstable filter feedback), causing filter state to explode (NaNs). |
| [VocalTract](file:///home/per/github/pertylizer/crates/synth_modules/src/vocal_tract.rs) | [vocal_tract.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/vocal_tract.rs) | ❌ Needs Fix | **Real-Time Safety & DSP Bugs:** Calls `PortName::intern()` inside `process()` for custom CV ports. Modulated articulator values are not clamped to `[0, 1]` when CV is not connected, potentially causing out-of-range physical modeling behavior. |

---

### 3.4 Routing & Utility

Level adjustment, mixing, signals, matrix mappings, script engines, and terminal input/output interfaces.

| Module/Struct Name | Source Code File | Review Status | Notes & Identified Issues |
|---|---|---|---|
| [Amplifier](file:///home/per/github/pertylizer/crates/synth_modules/src/amplifier.rs#L150) | [amplifier.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/amplifier.rs) | Passed | Structurally clean, real-time safe, uses linear ramp for parameter smoothing. |
| [Mixer](file:///home/per/github/pertylizer/crates/synth_modules/src/amplifier.rs#L423) | [amplifier.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/amplifier.rs) | Passed | Real-time safe, utilizes pre-interned static port name constants to avoid allocations, features clean limiting toggle. |
| [AudioInput](file:///home/per/github/pertylizer/crates/synth_modules/src/audio_input.rs) | [audio_input.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/audio_input.rs) | Passed | Safe from array indexing panics (handles partial block lengths). Real-time safe. |
| [Fooglers](file:///home/per/github/pertylizer/crates/synth_modules/src/fooglers.rs) | [fooglers.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/fooglers.rs) | ❌ Needs Fix | **DSP Stability & Panic Bug:** Unclamped modulated `tap1` and `tap2` can exceed `[0.0, 1.0]`. If `position > 2.0`, it leads to `delay > write_pos + BUFFER_SIZE` which causes an unsigned underflow panic in debug mode. Unclamped modulated `damping` can also be negative, causing unstable feedback loop explosion. |
| [ModMatrix](file:///home/per/github/pertylizer/crates/synth_modules/src/mod_matrix.rs) | [mod_matrix.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/mod_matrix.rs) | Passed | Clean config storage, no-op process block (routing is applied by Voice). |
| [StereoOutput](file:///home/per/github/pertylizer/crates/synth_modules/src/output.rs) | [output.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/output.rs) | Passed | Real-time safe, pre-allocated buffers, handles partial/missing channel routing inputs gracefully. |
| [RingMod](file:///home/per/github/pertylizer/crates/synth_modules/src/ring_mod.rs) | [ring_mod.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/ring_mod.rs) | Passed | Clean multi-waveform carrier generator, LFO phase wraps safely via `rem_euclid`. |
| [VectorMixer](file:///home/per/github/pertylizer/crates/synth_modules/src/vector_mixer.rs) | [vector_mixer.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/vector_mixer.rs) | Passed | Clean XY crossfader. Uses equal-power bilinear crossfade. No allocations. |

---

### 3.5 Analysis, Helpers & Core Infrastructure

Signal analysis, tracking, scripting, mathematical formulas, and internal shared structures.

| Module/Struct Name / File | Source Code File | Review Status | Notes & Identified Issues |
|---|---|---|---|
| [BeatDetector](file:///home/per/github/pertylizer/crates/synth_modules/src/beat_detector.rs) | [beat_detector.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/beat_detector.rs) | ❌ Needs Fix | **Real-Time Safety & DSP Bug:** Calls `PortName::intern("gate")` inside `process()` (real-time safety violation). Modulating `filter_freq` negatively causes `to_exp_coeff()` to return values `> 1.0`, leading to an unstable lowpass filter feedback loop and NaN state explosion. |
| [PitchTracker](file:///home/per/github/pertylizer/crates/synth_modules/src/pitch_tracker.rs) | [pitch_tracker.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/pitch_tracker.rs) | Passed | Auto-correlation based pitch detection with Goertzel frequency verification. Preallocated ring buffer, no heap allocations. Safe from NaN in frequency tracking. |
| [ScriptModule](file:///home/per/github/pertylizer/crates/synth_modules/src/script_module.rs) | [script_module.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/script_module.rs) | Passed | Real-time safe, evaluates/caches YAMS scripts, pre-interns port names at instantiation. |
| [SignalMonitor](file:///home/per/github/pertylizer/crates/synth_modules/src/signal_monitor.rs) | [signal_monitor.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/signal_monitor.rs) | ⚠️ Attention | Standard library `f32::clamp` panic on NaN when clamping `sample` inside `BipolarValue::new(sample)`. Pass-through itself is RT-safe. |
| Formant Tables Data | [formant_tables.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/formant_tables.rs) | ⚠️ Attention | `interpolate_vowel` calls `vowel.clamp(0.0, 1.0)` on the raw float. If `vowel` is NaN (e.g. from an exploded LFO or CV input), it will trigger a standard library panic, crashing the host. |
| DSP Math Helpers | [math.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/math.rs) | ⚠️ Attention | `hard_clip` and `foldback` functions call `.clamp()` on input values. If these inputs contain `NaN`, it will trigger a standard library panic on the audio thread. |
| Voice Common Types | [voice_common.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/voice_common.rs) | Passed | Shared unison spread and stagger helpers, RT-safe, uses deterministic seed. |
| Wavetable Generation Data | [wavetable_data.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/wavetable_data.rs) | ⚠️ Attention | `WavetableBank::sample` calls `position.clamp(0.0, 1.0)`. If `position` is NaN, it panics. |
| Crate Registration / Exports | [lib.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/lib.rs) | Passed | Crate registration is complete and accurate. |

---

### 3.6 Audio Effects

Standard and spectral effect processors that operate on audio buses.

| Module/Struct Name | Source Code File | Review Status | Notes & Identified Issues |
|---|---|---|---|
| [BbdDelay](file:///home/per/github/pertylizer/crates/synth_modules/src/effects/bbd_delay.rs) | [bbd_delay.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/effects/bbd_delay.rs) | Passed | Bucket-brigade delay emulation. Delay line uses linear interpolation and safe integer wrapping. |
| [Chorus](file:///home/per/github/pertylizer/crates/synth_modules/src/effects/chorus.rs) | [chorus.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/effects/chorus.rs) | ⚠️ Attention | **UI/DSP Mismatch:** The parameter descriptor sets range for `voices` to `1.0`..`8.0`, but the DSP clamps to `1`..`4` to match the stack-allocated `lfo_phases` array, leading to silent clamping for settings > 4. |
| [Compressor](file:///home/per/github/pertylizer/crates/synth_modules/src/effects/compressor.rs) | [compressor.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/effects/compressor.rs) | ❌ Needs Fix | **DSP Bug:** `sidechain_buffer` is statically allocated to 4096 floats. If the block size is larger than 2048 stereo frames (4096 samples), `set_sidechain_input` truncates the sidechain buffer and `read_frame` reads zero for the remaining frames, leading to incorrect compressor behavior (no gain reduction for the second half of the block). |
| [Convolver](file:///home/per/github/pertylizer/crates/synth_modules/src/effects/convolver.rs) | [convolver.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/effects/convolver.rs) | ⚠️ Attention | Heap allocations occur inside `rebuild_ir` during `set_param()`, which runs on the real-time audio thread when receiving parameter changes from the engine. |
| [CrossoverSplitter](file:///home/per/github/pertylizer/crates/synth_modules/src/effects/crossover.rs) | [crossover.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/effects/crossover.rs) | Passed | 2-band Linkwitz-Riley crossover splitter. Safe, RT-safe. |
| [Delay](file:///home/per/github/pertylizer/crates/synth_modules/src/effects/delay.rs) | [delay.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/effects/delay.rs) | Passed | Standard stereo delay. Uses safe buffer index wrapping. |
| [Distortion](file:///home/per/github/pertylizer/crates/synth_modules/src/effects/distortion.rs) | [distortion.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/effects/distortion.rs) | ❌ Needs Fix | **DSP Correctness Bug:** Processes interleaved stereo using a single `filter_state` (mono filter), causing left/right signal bleeding. **CPU Performance Overhead:** Recalculates `exp()` filter coefficient (`coef = cutoff.to_exp_coeff(self.sample_rate)`) for *every sample* inside the loop (should be calculated once per block outside the loop). |
| [EnsembleChorus](file:///home/per/github/pertylizer/crates/synth_modules/src/effects/ensemble_chorus.rs) | [ensemble_chorus.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/effects/ensemble_chorus.rs) | Passed | Multi-voice ensemble chorus. Clean and RT-safe. |
| [Eq](file:///home/per/github/pertylizer/crates/synth_modules/src/effects/eq.rs) | [eq.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/effects/eq.rs) | Passed | Parametric EQ using standard biquad filters. Bounded parameters. |
| [Flanger](file:///home/per/github/pertylizer/crates/synth_modules/src/effects/flanger.rs) | [flanger.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/effects/flanger.rs) | Passed | Standard flanger effect. Safe. |
| [FrequencyShifter](file:///home/per/github/pertylizer/crates/synth_modules/src/effects/frequency_shifter.rs) | [frequency_shifter.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/effects/frequency_shifter.rs) | Passed | Frequency shifter using Hilbert transform. Safe, no allocations. |
| [GranularFx](file:///home/per/github/pertylizer/crates/synth_modules/src/effects/granular_fx.rs) | [granular_fx.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/effects/granular_fx.rs) | Passed | Granular delay effect. No heap allocation, safe ring buffer indexing. |
| [Limiter](file:///home/per/github/pertylizer/crates/synth_modules/src/effects/limiter.rs) | [limiter.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/effects/limiter.rs) | ⚠️ Attention | **Design Limitation:** Look-ahead buffer is statically capped at `MAX_LOOKAHEAD_SAMPLES = 240`. At high sample rates (e.g. 96kHz or 192kHz), this caps the maximum look-ahead time to 2.5ms / 1.25ms instead of the requested 5ms maximum. |
| [MidSide](file:///home/per/github/pertylizer/crates/synth_modules/src/effects/mid_side.rs) | [mid_side.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/effects/mid_side.rs) | Passed | Mid-side matrix encoder/decoder. Safe. |
| [ModalResonator](file:///home/per/github/pertylizer/crates/synth_modules/src/effects/modal_resonator.rs) | [modal_resonator.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/effects/modal_resonator.rs) | Passed | Resonator with multiple bandpass filters. Bounded parameters. |
| [PhaseVocoder](file:///home/per/github/pertylizer/crates/synth_modules/src/effects/phase_vocoder.rs) | [phase_vocoder.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/effects/phase_vocoder.rs) | ❌ Needs Fix | **DSP Bug / Broken Feature:** Spectral freeze mode is broken. It attempts to copy `self.frozen_left/right` into the spectrum, but these vectors are never populated with the active spectrum, producing complete silence when freeze is active. Can be resolved by split borrowing fields of `self` (mut stft_left, mut frozen_left, etc.) and passing them to the process closure. |
| [Phaser](file:///home/per/github/pertylizer/crates/synth_modules/src/effects/phaser.rs) | [phaser.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/effects/phaser.rs) | ⚠️ Attention | **CPU Performance Overhead:** Calls `omega.tan()` inside the process loop for every sample (2 times per frame), introducing unnecessary CPU overhead. |
| [Reverb](file:///home/per/github/pertylizer/crates/synth_modules/src/effects/reverb.rs) | [reverb.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/effects/reverb.rs) | Passed | Multi-tap Schroeder/Fdn reverb. Stable delay lines, no allocations. |
| [ReverseGateReverb](file:///home/per/github/pertylizer/crates/synth_modules/src/effects/reverse_gate_reverb.rs) | [reverse_gate_reverb.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/effects/reverse_gate_reverb.rs) | Passed | Gated reverse reverb effect. Safe. |
| [ShimmerReverb](file:///home/per/github/pertylizer/crates/synth_modules/src/effects/shimmer_reverb.rs) | [shimmer_reverb.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/effects/shimmer_reverb.rs) | ❌ Needs Fix | **DSP Math / Stability Issue:** The pitch-shifted feedback signal `self.feedback_acc` is fed back into the FDN input with gain `1.0` (unattenuated by `feedback_gain` or decay parameters), relying solely on `soft_clip` to prevent infinity. This causes the shimmer effect to have infinite sustain/runaway saturation instead of decaying naturally. |
| [SpectralBlur](file:///home/per/github/pertylizer/crates/synth_modules/src/effects/spectral_blur.rs) | [spectral_blur.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/effects/spectral_blur.rs) | Passed | STFT-based spectral smearing. Preallocated FFT buffers. Safe. |
| [TiltEq](file:///home/per/github/pertylizer/crates/synth_modules/src/effects/tilt_eq.rs) | [tilt_eq.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/effects/tilt_eq.rs) | Passed | 1-pole tilt equalization filter. Bounded gains. |
| [Univibe](file:///home/per/github/pertylizer/crates/synth_modules/src/effects/univibe.rs) | [univibe.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/effects/univibe.rs) | ❌ Needs Fix | **DSP Math Bug:** Calculates `omega = TAU * mod_freq / sr` instead of `PI * mod_freq / sr` for all-pass coefficient, effectively doubling the cutoff frequency and warping phase response. **CPU Performance Overhead:** Calls `omega.tan()` inside the process loop for every sample (4 times per frame), introducing significant CPU overhead. |
| [Vocoder](file:///home/per/github/pertylizer/crates/synth_modules/src/effects/vocoder.rs) | [vocoder.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/effects/vocoder.rs) | Passed | Channel vocoder using bandpass filter bank. Bounded. |
| [Waveshaper](file:///home/per/github/pertylizer/crates/synth_modules/src/effects/waveshaper.rs) | [waveshaper.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/effects/waveshaper.rs) | ⚠️ Attention | **NaN Panic Vulnerability:** Chebyshev mode calls `driven.clamp(-1.0, 1.0)` on raw `f32` (panics if input is `NaN`). **CPU Performance Overhead:** Recalculates `self.drive_gain()` inside the process loop for every sample, which can be pre-calculated once per block. |
| Effects Module Registry | [mod.rs](file:///home/per/github/pertylizer/crates/synth_modules/src/effects/mod.rs) | Passed | Registry is clean, correctly routes parameter settings to effects. |

---

## 4. Second-Opinion Verification — Round 1 (P1 DSP bugs)

Independent re-verification of the nine `❌ Needs Fix` **DSP-correctness** claims, read line-by-line
against the current code. **3 of 9 are false positives.** The refutations cluster around two
systematic blind spots in Round 0:

1. **The mod-matrix offset path is already clamped.** Several claims assume "negative modulated
   frequency/cutoff → unstable coefficient → NaN". But mod-matrix offsets flow through
   `ParamModOffsets::effective()` ([module_traits.rs:1108](file:///home/per/github/pertylizer/crates/synth_core/src/module_traits.rs#L1108)),
   which clamps in **normalized** space and then denormalizes into the parameter's declared range —
   so the modulated value can never leave `[min, max]` and never goes negative. This invalidates the
   negative-modulation premise for **BodyResonance** and **MechanicalNoise** (and likely softens
   several `⚠️` "modulated cutoff_cv can propagate NaN" notes — those only hold for *direct audio-rate
   CV-input-buffer* ports, not the mod-matrix path).
2. **Downstream stages apply their own gain.** **ShimmerReverb**'s "unattenuated 1.0 feedback" ignores
   that the injected signal is the FDN's *already-decayed* output.

| Module | Original | Verification | Second Opinion |
|---|---|---|---|
| BodyResonance | ❌ Needs Fix | ❌ **REFUTED** | Coefficient mechanism is sound in isolation (`biquad_omega` has no lower-bound clamp), but unreachable: freq modulation routes through `effective()` → bounded to `[50, 2000]` Hz, never negative; derived `freq1–3` multiply by factors ≥ 0.7. No NaN/division-by-zero path exists. |
| MechanicalNoise | ❌ Needs Fix | ❌ **REFUTED** | Same root error. Cutoff via `effective()` bounded to `[100, 10000]` Hz → `alpha ∈ (0, 0.5]`, `1−alpha ∈ [0.5, 1)`, always stable. Only the direct `set_parameter` entry is unclamped, and it is descriptor-bounded. |
| Univibe | ❌ Needs Fix | ⚠️ **PARTIAL** | TAU-vs-PI is real but it is a **frequency-placement** error (notches land ~an octave high), *not* a stability/correctness bug — coefficients stay in `[-1, 1]`. Perf claim holds but is understated: `tan()` runs **8×/frame**, not 4×. `mod_freq` is LFO-modulated per-sample so it cannot be fully hoisted; the fix is to cache the duplicated `omega.tan()` on the same line. |
| PhaseVocoder | ❌ Needs Fix | ✅ **CONFIRMED** | Accurate. `frozen_l/r` are only ever zero-filled; the "captured on next STFT frame" code (lines 313–317) was never written → freeze yields silence. Real fix is the **missing capture** (copy live spectrum into the frozen buffers), which does require the split-borrow restructure the note mentions. |
| ShimmerReverb | ❌ Needs Fix | ❌ **REFUTED** | Load-bearing claim is wrong. The `+ feedback_acc` injection is gain-1.0, but `feedback_acc` *is* the FDN's already-decayed output (`feedback_gain` 0.3–0.97 applied inside the FDN), so round-trip loop gain < 1 → natural decay. `soft_clip` is a safety net, not the sole bound. No runaway. |
| Compressor | ❌ Needs Fix | ⚠️ **CONFIRMED (low severity)** | Correct as written, but only reachable at block sizes > 2048 frames. Engine cap is 4096 frames, default 256 → never triggers in normal playback, only oversized offline renders/host buffers. Latent bug; fix = size `sidechain_buffer` to `MAX_BUFFER_SIZE*2` (8192). **Downgrade priority.** |
| Distortion | ❌ Needs Fix | ✅ **CONFIRMED (both)** | Both hold. A single `filter_state` is shared across interleaved L/R → genuine cross-channel bleed (not just per-frame). `tone` is a stored param (not per-sample modulated), so the per-sample `to_exp_coeff()`/`exp()` is truly redundant — hoist once per block. **Cleanest, highest-value P1 fix.** |
| PadSynth | ❌ Needs Fix | ⚠️ **CONFIRMED (figure off)** | Core hazard real: `build_wavetable` runs unconditionally on every `note_on`, on the audio thread, O(N²) brute-force IDFT. But magnitude is **~8.4M** cos-evals worst case (TABLE_SIZE = 4096), *not* "2.1M"; the `a < 1e-8` skip prunes empty bins. Fix: rebuild only when dirty + move off the audio thread. |
| Convolver | ⚠️ Attention | ✅ **CONFIRMED (narrow)** | `rebuild_ir` does allocate (FFT plans, Vecs) on the audio thread via `set_param`. But it is guarded: fires only on `Ir`-type change or `DecayTrim` delta > 0.01 — not `Mix`/`PreDelay`/`Brightness`, not steady-state automation. Real glitch risk on those specific edits only. (Listed under `⚠️` in §3.6, not §3 P1, but verified here for completeness.) |

**Round-1 tally:** 2 fully confirmed (PhaseVocoder, Distortion), 3 confirmed-but-narrowed
(Compressor → low-sev, PadSynth → wrong magnitude, Convolver → rare trigger), 1 partial
(Univibe → freq-placement not stability), **3 refuted** (BodyResonance, MechanicalNoise,
ShimmerReverb). Recommended fix order: **Distortion → PhaseVocoder → PadSynth → Convolver →
Compressor**; drop the three refuted from the work list.

---

## 5. Second-Opinion Verification — Round 2 (`⚠️` modulation-hazard rows)

Re-audit of the warning rows whose hazard rests on "a modulated parameter goes out of range /
negative / NaN," now read through the **`effective()` lens** plus the established `f32::clamp`
correction. The decisive question per row: does the dangerous value arrive via the **mod-matrix
offset path** (`mod_offsets.effective(...)`, clamped into the param's declared range) or via a
**direct CV-input buffer** (`inputs.reader(PortName::…)`, genuinely unbounded)?

**Result: 5 of 7 refuted.** The only survivors read direct CV buffers that bypass `effective()` —
and even there the consequence is **silent NaN propagation / overflow, never a panic.**

| Module | Original | Verification | Second Opinion |
|---|---|---|---|
| AmFormant | ⚠️ Attention | ❌ **REFUTED** | `carrier_ratio` modulates only via `effective()`, range `[0,1]` → `ratio = 0.25·16^x ∈ [0.25, 4.0]`, finite and non-negative. Cannot explode to ∞/NaN. NaN-panic consequence also wrong (`clamp` doesn't panic). Both halves unfounded. |
| LadderFilter | ⚠️ Attention | ⚠️ **PARTIAL — real survivor** | `cutoff_cv` (filter.rs:715, `PortName::CUTOFF_CV`) **is a direct unbounded CV buffer**, so out-of-range/NaN *can* reach `exp2()` → coefficient → ladder state. But consequence is a **stuck-NaN/silent voice**, not a panic. Separately the **drive-gating is genuine**: soft-clip/saturation is skipped at `drive ≤ 1.0`, so the 4-stage ladder runs with resonance feedback `k ≈ 3.96` and *no* nonlinearity → can overflow at high resonance. Fix: always soft-clip (or sanitize `cutoff_cv` / clamp before `exp2`). |
| Sampler | ⚠️ Attention | ❌ **REFUTED** | `start_offset` via `effective()` (bounded); `clamp` doesn't panic on NaN; and `NaN as usize` saturates to 0 with a `frame < data.len()` bounds check on read → silence, never OOB/crash. No hazard. |
| EnvelopeFollower | ⚠️ Attention | ❌ **REFUTED** | `sensitivity` range `[0,1]` via `effective()` (no CV port); output `= (env · scale).min(1.0)` where `env` filters `abs(input) ≥ 0`. Both factors ≥ 0 → output can't go negative. The proposed `.max(0.0)` is unnecessary. |
| Euclidean | ⚠️ Attention | ❌ **REFUTED** | `swing` range `[0,1]` via `effective()` → `swing_offset ≥ 0` → threshold `= samples_per_step + swing_offset` is always `> samples_per_step > 0`. The per-sample clock-runaway is unreachable; `effective()`'s min-clamp is the guard. |
| Fooglers | ❌ Needs Fix | ❌ **REFUTED** | (Was `❌`.) `tap1`/`tap2`/`damping` all via `effective()` (no CV ports) → bounded to `[0.01,1]`/`[0,1]`. Index math `(write_pos + 256 − delay) % 256` with `delay ≤ 255` has a deliberate `+BUFFER_SIZE` underflow guard → no usize underflow. Damping further bounded by `soft_clip` + `feedback.min(0.99)`. **Not a bug — 4th refuted `❌`.** |
| AdditiveOsc | ⚠️ Attention | ⚠️ **PARTIAL** | ±1-octave limit **CONFIRMED** (`freq_cv` direct buffer → `apply_cv = 2^cv`, `cv ∈ [-1,1]`). NaN-panic **REFUTED**: `BipolarValue::new`'s `clamp(-1,1)` doesn't panic on NaN — NaN survives the clamp and propagates silently into frequency. Direct-CV path, so unbounded upstream; only the ±1 hard clamp applies. |

**Round-2 tally:** **5 refuted** (AmFormant, Sampler, EnvelopeFollower, Euclidean, Fooglers),
**2 partial** (LadderFilter — genuine drive-gating overflow + CV-NaN propagation; AdditiveOsc —
NaN propagation + the separate ±1-oct design limit).

### Cross-cutting conclusion (both rounds)

The original review applied one mental model uniformly — *"modulated param → leaves range → `clamp`
panics on NaN → host crash."* Two facts break it across the board:

1. **Mod-matrix params are already clamped** by `effective()` (normalized-space clamp → denormalize
   into declared range). Every "negative/exploding modulation" claim against an `effective()`-only
   param is a false positive: BodyResonance, MechanicalNoise, AmFormant, Sampler, EnvelopeFollower,
   Euclidean, Fooglers — **7 modules**.
2. **`f32::clamp` never panics on NaN** (only if `min > max`). Every "clamp panics → crash" claim has
   the wrong consequence; the real risk is **silent NaN propagation** (a stuck/garbage voice until
   note reset), which only matters where unbounded values can actually enter.

**Where a real (reduced-severity) issue survives, it is always a *direct CV-input buffer*** —
`cutoff_cv`, `freq_cv`, `fm`, etc. — that bypasses `effective()`. So if NaN/overflow robustness is
wanted, the high-leverage fix is **one sanitization (finite-or-zero, optional range clamp) at the
CV-buffer read sites**, not per-module clamp-panic guards. The standalone genuine DSP bug uncovered
here is **LadderFilter's `drive ≤ 1.0` saturation gating** (unbounded resonance feedback), which is
independent of modulation entirely.

---

## 6. Resolution — fixes applied

All genuine bugs surfaced by the two verification rounds were fixed in the working tree (one commit,
no version bump). The 8 false positives (BodyResonance, MechanicalNoise, ShimmerReverb, Fooglers,
AmFormant, Sampler, EnvelopeFollower, Euclidean) and the pure design-limitations (mod-depth,
TuringMachine range) were **not** touched. Full gate green: `cargo fmt --check`, `cargo build`,
`cargo clippy --all-targets`, `cargo test` (0 failures).

**RT-safety — `PortName::intern()` removed from `process()`** (cached interned `PortName` in struct
fields / switched to constants): ChaoticOsc, Fof, FormantFilter, VoiceSynth, VocalTract, BeatDetector.
(Filter was already clean.)

**DSP correctness / stability:**
- Distortion — per-channel `[FilterState; 2]` (fixes L/R bleed) + `tone` coefficient hoisted out of
  the per-sample loop.
- PhaseVocoder — spectral freeze now actually captures the live spectrum (was silence); capture holds
  the **first** hop when a block spans multiple hops (code-review fix).
- LadderFilter — soft-clip/saturation now always applied (was gated behind `drive > 1.0`).
- BeatDetector — `filter_freq` clamped before `to_exp_coeff` (no unstable coefficient).
- VoiceSynth — `open_quotient` floored to avoid div-by-zero; vowel interpolation hoisted to O(1)/recompute.
- VocalTract — articulators clamped to `[0, 1]`.
- Univibe — all-pass prewarp uses `PI` (was `TAU`); duplicate `tan()` cached.
- LaSynth — swept-frequency phase de-doubled; brightness filter made sample-rate independent.
- NoiseGenerator — brown-noise leak coefficient derived from a time constant + sample rate.
- Envelope — Decay→Sustain snap replaced by a symmetric convergence test (no click when sustain is
  modulated above the current level).

**RT-safety — heavy work off the hot path:**
- PadSynth — `build_wavetable` now rebuilds only when dirty (was every `note_on`).
- GranularOsc — heavy `fill_source_buffer` deferred + guarded on actual change.
- Convolver — `rebuild_ir` reuses scratch buffers + FFT planners (allocation-free in steady state).

**Sizing / descriptor:**
- Compressor — sidechain buffer sized to the engine's full interleaved-stereo block.
- Limiter — look-ahead buffer sized from 5 ms × max sample rate (honors 5 ms at 96/192 kHz).
- Chorus — `voices` descriptor range corrected to `1..4` to match the DSP (schemas regenerated).

**NaN robustness:** added `crate::math::sanitize_cv` (non-finite → 0.0) and applied it at the direct
CV-input reads in the oscillators/filters (incl. the `pwm`/`sync`/`pm` reads the code-review caught),
plus NaN guards in `hard_clip`/`foldback`, `interpolate_vowel`, `WavetableBank::sample`, and
`SignalMonitor`.

**Deferred (deeper / cross-crate, out of this pass's scope)** — captured in `plans/TODO.md §5`:
sanitize CV at the input-read boundary (§5.1), promote engine block-size / max-sample-rate to shared
`pub const`s (§5.2), build the convolver IR off the audio thread (§5.3), and PadSynth `set_sample_rate`
dirty hook (§5.4, low priority).
