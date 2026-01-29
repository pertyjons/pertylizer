# synth_modules review (module-by-module)

This document reviews the audio-generation and processing modules in `crates/synth_modules/src`, highlights potential bugs, edge cases, and improvement ideas, and references the relevant files.

## Summary of cross-cutting issues
- Several modules track edge-triggered gate/sync inputs with a `prev_*` local inside `process()`, which resets every audio buffer and can retrigger every buffer if a gate stays high.
- Multiple declared parameters or ports are unused in DSP (no audible effect), which can make UI/automation misleading.
- A few modules expose more inputs/parameters than the implementation actually handles.

---

## Sources / Generators

### Oscillator
File: `crates/synth_modules/src/oscillator.rs`

Findings:
- **Phase offset parameter unused**: `phase_offset` is set via `OscillatorParam::Phase` but never applied in `process()` or `generate_sample()`.
- **Hard-sync edge detection resets each buffer**: `prev_sync` is a local variable in `process()`. If the sync signal is held high, the oscillator will retrigger on every buffer boundary.

Suggestions:
- Apply `phase_offset` in `generate_sample()` or before reading `self.phase`.
- Make `prev_sync` a struct field so edge detection persists across buffers.

### SubOscillator
File: `crates/synth_modules/src/sub_osc.rs`

Findings:
- Square and pulse outputs are not band-limited, which can alias at higher frequencies.

Suggestions:
- Optional: add PolyBLEP for square/pulse to reduce aliasing.

### MathOscillator
File: `crates/synth_modules/src/math_oscillator.rs`

Findings:
- Many algorithms can exceed [-1, 1] (tests allow up to +/-2), which may clip later depending on signal chain. This is likely by design but worth noting.

Suggestions:
- Optional: a soft limiter or output gain normalization per algorithm if a more consistent output level is desired.

### NoiseGenerator
File: `crates/synth_modules/src/noise.rs`

Findings:
- `sample_rate` is stored but unused in the current noise implementations.

Suggestions:
- Optional: if you want sample-rate–independent coloration, incorporate `sample_rate` into the filter coefficients for colored noise.

### SamplePlayer
File: `crates/synth_modules/src/sample_player.rs`

Findings:
- **ReleaseMode::PlayToLoop is not implemented**: it currently behaves like `PlayToEnd` (no specific “stop at loop end” behavior).
- **Sample offset from tracker effects is not supported**: there is no input or state to start playback from a per-note offset.

Suggestions:
- Implement `PlayToLoop` as: on note-off, play until loop end then stop.
- Add a per-note offset input/state (if tracker playback requires it).

---

## Modulation / Control

### Lfo
File: `crates/synth_modules/src/lfo.rs`

Findings:
- **RetriggerMode parameter unused**: `retrigger_mode` is set but not used to change behavior.
- **Retrigger edge detection resets each buffer**: `prev_retrigger` is local in `process()`, causing repeated retriggers if gate is held.

Suggestions:
- Apply `retrigger_mode` inside `process()` or `note_on()`.
- Move `prev_retrigger` to struct state to avoid per-buffer retriggering.

### Envelope (ADSR)
File: `crates/synth_modules/src/envelope.rs`

Findings:
- **Gate edge detection resets each buffer**: `prev_gate` is local in `process()`, causing repeated attacks if gate stays high.

Suggestions:
- Store `prev_gate` as a struct field so the edge detection survives buffer boundaries.

### MultiPointEnvelope
File: `crates/synth_modules/src/multi_point_envelope.rs`

Findings:
- **Gate edge detection resets each buffer**: `prev_gate` is local in `process()`.
- **Velocity sensitivity parameter misapplied**: `VelocitySensitivity` maps directly to `self.velocity` (note velocity), not a sensitivity amount.
- **Tick rate likely wrong for tracker playback**: `tick_rate` defaults to 50 Hz, and nothing in this crate updates it from song tempo/speed.

Suggestions:
- Persist `prev_gate` across buffers.
- Add a dedicated `velocity_sensitivity` field, and apply it to `self.velocity` in `trigger()`.
- Provide an external hook to set tick-rate from song tempo/speed (or wire it in from the importer/engine).

---

## Filters / Shaping

### Filter (SVF) + LadderFilter
File: `crates/synth_modules/src/filter.rs`

Findings:
- **Unused parameters**: `env_amount` and `drive` are defined but not used anywhere in processing.
- **No envelope input**: there is no port or usage to apply `env_amount` even if it were implemented.

Suggestions:
- Either implement envelope modulation (add an input port and apply `env_amount`) and drive (pre/post gain), or remove the params to avoid confusing UI.

---

## Amplification / Mixing

### Amplifier
File: `crates/synth_modules/src/amplifier.rs`

Findings:
- CV pan uses `BipolarValue::new()` to clamp, which is fine. No issues found.

Suggestions:
- Optional: if `clip_mode` is meant to be user-selectable, expose it as a parameter.

### Mixer
File: `crates/synth_modules/src/amplifier.rs`

Findings:
- **Only 4 input level parameters exist**, but there are 8 input ports. Inputs 5–8 cannot be level-controlled.
- **LimitMode is never applied** despite having the parameter.

Suggestions:
- Add `MixerParam::Input5..Input8` and use them in `process()`.
- Implement `LimitMode` or remove the parameter if not needed.

---

## Physical Modeling / Spatial

### KeyboardPanner
File: `crates/synth_modules/src/keyboard_panner.rs`

Findings:
- `sample_rate` is stored but unused. No functional issues observed.

Suggestions:
- Optional: remove unused `sample_rate` or use it for smoothing transitions.

### MechanicalNoise
File: `crates/synth_modules/src/mechanical_noise.rs`

Findings:
- **Potential divide-by-zero**: `envelope_samples` can be 0 if `duration` is set to 0 (no clamp in `set_param`), leading to division by zero in envelope calculation.
- `envelope_phase` is unused after being set.

Suggestions:
- Clamp duration to a minimum > 0 in `set_param` and/or guard `envelope_samples`.
- Remove or use `envelope_phase` if intended.

### BodyResonance
File: `crates/synth_modules/src/body_resonance.rs`

Findings:
- No obvious logic bugs; processing and parameters appear consistent.

Suggestions:
- Optional: provide note-based modulation (e.g., update `frequency` or `mix` on `note_on`).

---

## Output

### StereoOutput
File: `crates/synth_modules/src/output.rs`

Findings:
- Limit and mute behavior appears correct. No issues found.

Suggestions:
- Optional: expose limiter threshold if you want control beyond on/off.

---

## Effects

### Delay
File: `crates/synth_modules/src/effects/delay.rs`

Findings:
- Looks coherent. Note that `time_right` is ignored in tempo-sync mode.

Suggestions:
- Optional: allow stereo time offsets even in tempo-sync mode (e.g., offset by ratios).

### Chorus
File: `crates/synth_modules/src/effects/chorus.rs`

Findings:
- **Voices parameter not exposed in descriptor**: `ChorusParam::Voices` is handled in `set_param`, but no UI/descriptor parameter exists to control it.

Suggestions:
- Add a parameter in the descriptor for voice count.

### Flanger
File: `crates/synth_modules/src/effects/flanger.rs`

Findings:
- **Rate CV port is unused**: `rate_cv` is declared but not read in `process()`.

Suggestions:
- Read `rate_cv` and modulate `rate` or `phase_inc` accordingly.

### Phaser
File: `crates/synth_modules/src/effects/phaser.rs`

Findings:
- **Rate CV port is unused**: `rate_cv` is declared but not read in `process()`.

Suggestions:
- Read `rate_cv` and modulate LFO rate accordingly.

### Reverb
File: `crates/synth_modules/src/effects/reverb.rs`

Findings:
- No obvious logic bugs found in the core processing.

Suggestions:
- Optional: expose more tuning controls or early reflection handling if needed.

### Distortion
File: `crates/synth_modules/src/effects/distortion.rs`

Findings:
- **Bitcrush depth fixed**: `bit_depth` is fixed at 8.0 and never exposed as a parameter, so `Bitcrush` mode cannot be configured.

Suggestions:
- Add a `BitDepth` parameter to `DistortionParam` and use it to set `bit_depth`.

### Compressor
File: `crates/synth_modules/src/effects/compressor.rs`

Findings:
- **Sidechain port is unused**: `sidechain` input exists but isn’t used in level detection.

Suggestions:
- If sidechain is desired, use it to compute `peak_db` instead of the main input (or add a mix option).

### EQ
File: `crates/synth_modules/src/effects/eq.rs`

Findings:
- No obvious logic bugs found; coefficients are recalculated on param changes and sample-rate changes.

Suggestions:
- Optional: consider smoothing parameter changes to avoid zipper noise.

---

## Suggested next steps
1) Pick a few high-impact fixes (e.g., gate edge persistence, unused params/ports) and I can implement them.
2) If you want, we can add tests that assert gate/sync handling across buffers and that parameters actually influence output.

---

## Newtype-idiom & Rust best practices (från CLAUDE.md)

Denna sektion är en projektövergripande check mot newtype‑idiomet och Rust‑best‑practices. Fokuset är på publika API:er och domänvärden som fortfarande använder råa primitiver.

### Newtype-avvikelser (råa primitives i publika API:er)
- `crates/synth_engine/src/instrument.rs:742` – `note_on(&mut self, note: MidiNote, velocity: f32)` använder `f32` i publikt API. Bör vara `Velocity` eller `NormalizedValue`.
- `crates/synth_engine/src/voice_allocator.rs:213` – `note_on(&mut self, note: MidiNote, velocity: f32)` använder `f32`.
- `crates/synth_engine/src/voice_allocator.rs:329` – `note_on_fixed_voice(..., velocity: f32)` använder `f32`.
- `crates/synth_engine/src/voice.rs:131` – `GlideState` håller `from_freq`, `to_freq`, `current_freq`, `time`, `position` som `f32`. Dessa är domänvärden (Hertz/Seconds/NormalizedValue).

### Rust best practices enligt CLAUDE.md
- Newtypes ska ha `#[must_use]`, men saknas på:
  - `crates/synth_core/src/types/sample.rs:20` (`SampleValue`)
  - `crates/synth_core/src/types/sample.rs:99` (`SampleIndex`)
- Visibility: interna typer bör vara `pub(crate)` där möjligt:
  - `crates/synth_engine/src/voice.rs:131` (`GlideState`) verkar intern men är `pub`.

### Förslag på åtgärder
1) Byt `velocity: f32` till `Velocity`/`NormalizedValue` i publika API:er och följ ändringen genom call‑chain.
2) Konvertera `GlideState` till newtypes (`Hertz`, `Seconds`, `NormalizedValue`) och sänk visibility till `pub(crate)`.
3) Lägg till `#[must_use]` på `SampleValue` och `SampleIndex`.
