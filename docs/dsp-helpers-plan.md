# DSP Helper Abstractions Plan

Reduce boilerplate and improve maintainability by introducing four native abstractions
inspired by concepts from dasp (Signal trait, Frame) and fundsp (composable primitives, denormal handling).

## Scope

| Feature | Problem | Impact |
|---------|---------|--------|
| InputReader | 49 `inputs.get()` + 20 `.map(\|b\| b[i]).unwrap_or()` across 22 modules | High |
| Stereo Frame Helpers | 176 manual stereo index calculations across 18 effects | High |
| DenormalGuard | 23 manual `flush_denormals()` calls across 5 files | Medium |
| Composable DSP Primitives | Duplicate stereo filter state management, ad-hoc param smoothing | Low |

---

## Feature 1: InputReader

**File:** `crates/synth_core/src/module_traits.rs`

Zero-cost wrapper that replaces the `.map(|b| b[i]).unwrap_or(default)` pattern
with direct indexing.

```rust
#[derive(Clone, Copy)]
pub struct InputReader<'a> {
    buffer: Option<&'a AudioBuffer>,
    default: f32,
}
```

**API:**
- `InputPorts::reader(name: PortName, default: f32) -> InputReader<'a>` — create reader
- `InputReader::is_connected() -> bool` — check if port has input
- `InputReader::as_slice() -> Option<&[f32]>` — bulk access for connected buffer
- Implements `Index<usize>` — returns buffer sample or default value

**Before/After:**
```rust
// BEFORE (repeated per port, per sample):
let fm = inputs.get(PortName::FM).map(|b| b[i]).unwrap_or(0.0);

// AFTER (reader created once before loop):
let fm = inputs.reader(PortName::FM, 0.0);
for i in 0..samples {
    output[i] = fm[i];  // direct indexing
}
```

**Migration targets (6 highest-usage modules):**
`amplifier.rs`, `filter.rs`, `oscillator.rs`, `lfo.rs`, `output.rs`, `envelope.rs`

---

## Feature 2: Stereo Frame Helpers

**File:** `crates/synth_core/src/types/audio.rs`

Static methods on `StereoSample` + an iterator type to eliminate manual
interleaved stereo indexing (`frame * 2`, `frame * 2 + 1`, bounds checks).

**Primary API — static functions (random-access, used by most effects):**
```rust
impl StereoSample {
    pub fn read_frame(data: &[f32], frame: usize) -> StereoSample
    pub fn write_frame(data: &mut [f32], frame: usize, sample: StereoSample)
}
```

**Secondary API — iterator (sequential patterns):**
```rust
pub struct StereoFrameIter<'a> { data: &'a [f32], frame: usize, num_frames: usize }
// Implements Iterator<Item = StereoSample> + ExactSizeIterator

impl StereoSample {
    pub fn iter_frames(data: &[f32], num_frames: usize) -> StereoFrameIter<'_>
}
```

**Before/After:**
```rust
// BEFORE (in every effect, 176 occurrences total):
let idx_l = frame * 2;
let idx_r = frame * 2 + 1;
let dry = if idx_r < input.len() {
    StereoSample::new(input[idx_l], input[idx_r])
} else if idx_l < input.len() {
    StereoSample::from_mono(input[idx_l])
} else { StereoSample::ZERO };
// ...
if idx_l < output.len() { output[idx_l] = wet.left; }
if idx_r < output.len() { output[idx_r] = wet.right; }

// AFTER:
let dry = StereoSample::read_frame(input, frame);
// ...
StereoSample::write_frame(output, frame, wet);
```

**Migration targets (all 18 effects):**
`delay.rs`, `reverb.rs`, `compressor.rs`, `limiter.rs`, `flanger.rs`, `eq.rs`,
`bbd_delay.rs`, `chorus.rs`, `ensemble_chorus.rs`, `phaser.rs`, `mid_side.rs`,
`shimmer_reverb.rs`, `reverse_gate_reverb.rs`, `modal_resonator.rs`, `granular_fx.rs`,
`frequency_shifter.rs`, `distortion.rs`/`waveshaper.rs`, `convolver.rs`

---

## Feature 3: DenormalGuard

**New file:** `crates/synth_core/src/types/denormal.rs`

RAII guard that sets FTZ+DAZ CPU flags on creation and restores on drop.
Eliminates all per-sample denormal flushing at the hardware level.

```rust
#[must_use]
pub struct DenormalGuard {
    #[cfg(target_arch = "x86_64")]
    previous_mxcsr: u32,
    #[cfg(target_arch = "aarch64")]
    previous_fpcr: u64,
}
```

**Platform support:**
- **x86_64** (Linux, macOS Intel, Windows): Sets FTZ+DAZ via MXCSR register (`_mm_getcsr`/`_mm_setcsr`)
- **aarch64** (macOS Apple Silicon, Linux ARM): Sets FZ via FPCR register (`mrs`/`msr fpcr`)
- **Other architectures**: No-op (fallback, denormals handled by existing `flush_denormals()`)
- All register writes are thread-local, RAII guarantees restore — safe and well-understood
- Standard practice in audio frameworks (JUCE, SuperCollider, etc.)

**Integration:** One line in the cpal audio callback:
```rust
// crates/pertylizer/src/audio/backends/cpal_backend.rs, line ~259
move |data: &mut [f32], _output_info: &cpal::OutputCallbackInfo| {
    let _denormal_guard = DenormalGuard::new();
    // ...existing code...
}
```

**Wire up:** Add `mod denormal;` to `crates/synth_core/src/types/mod.rs`, re-export via `pub use denormal::*;`

**After integration:** Remove all 23 `flush_denormals()` calls from modules and DSP code.
Keep the `flush_denormals()` method itself (do NOT deprecate yet) as fallback for
architectures without hardware denormal prevention.

---

## Feature 4: Composable DSP Primitives

Lowest priority. Thin wrappers for common filter patterns.

### 4a. StereoSvf — `crates/synth_dsp/src/filters.rs`
Paired SVF for stereo processing with shared coefficients.
```rust
pub struct StereoSvf {
    left_ic1: FilterState, left_ic2: FilterState,
    right_ic1: FilterState, right_ic2: FilterState,
}
// process(input: StereoSample, coeffs: &SvfCoeffs, filter_type: SvfFilterType) -> StereoSample
// reset()
```

### 4b. StereoBiquad — `crates/synth_dsp/src/filters.rs`
Paired biquad for stereo processing.
```rust
pub struct StereoBiquad {
    left_z1: FilterState, left_z2: FilterState,
    right_z1: FilterState, right_z2: FilterState,
}
// process(input: StereoSample, coeffs: &BiquadCoeffs) -> StereoSample
// reset()
```

### 4c. OnePoleSmooth — `crates/synth_core/src/types/audio.rs`
Parameter smoother using existing `FilterState::one_pole()`.
```rust
pub struct OnePoleSmooth { state: FilterState, coeff: f32 }
// new(time_seconds: f32, sample_rate: SampleRate) -> Self
// process(target: f32) -> f32
// set(value: f32)  — immediate, skip smoothing
// current() -> f32
```

---

## Implementation Order

| Step | What | Files |
|------|------|-------|
| 1 | `InputReader` + `InputPorts::reader()` | `synth_core/src/module_traits.rs` |
| 2 | `StereoFrameIter` + `read_frame`/`write_frame` | `synth_core/src/types/audio.rs` |
| 3 | `DenormalGuard` | New: `synth_core/src/types/denormal.rs`, edit: `types/mod.rs` |
| 4 | `OnePoleSmooth` | `synth_core/src/types/audio.rs` |
| 5 | `StereoSvf` + `StereoBiquad` | `synth_dsp/src/filters.rs`, `synth_dsp/src/lib.rs` |
| 6 | Tests for all new types | Same files (`#[cfg(test)]` modules) |
| 7 | Integrate `DenormalGuard` in cpal callback | `pertylizer/src/audio/backends/cpal_backend.rs` |
| 8 | Remove `flush_denormals()` calls | `synth_dsp/src/filters.rs`, `synth_modules/src/filter.rs`, `synth_modules/src/body_resonance.rs` |
| 9 | Migrate 6 modules to `InputReader` | `amplifier.rs`, `filter.rs`, `oscillator.rs`, `output.rs`, `lfo.rs`, `envelope.rs` |
| 10 | Migrate 18 effects to stereo frame helpers | All effects in `synth_modules/src/effects/` |

## Verification

All must pass with zero warnings/errors:
```bash
cargo build
cargo clippy --all-targets
cargo test
cargo fmt --check
```
