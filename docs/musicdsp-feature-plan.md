# MusicDSP Feature Plan — Remaining Work

Features from [musicdsp.org](https://www.musicdsp.org/en/latest/index.html)
([GitHub source](https://github.com/bdejong/musicdsp)).

## Attribution Rule

Every source file implementing an algorithm from this plan **must** include a comment linking
to the specific GitHub file and crediting it as the source. Example:

```rust
// Algorithm source: https://github.com/bdejong/musicdsp/blob/master/source/Filters/24-moog-vcf.rst
// From the Music-DSP Source Code Archive (https://www.musicdsp.org/)
```

---

## Status: 21/26 fully exposed, 5 internal-only

All 26 algorithms are implemented in code. The 5 items below have DSP primitives written
but are **not yet exposed in the UI/MCP** — they need to be wired into modules with
parameters and descriptors to become usable.

---

## 1. MinBLEP Oscillator Anti-Aliasing

**Code:** `synth_dsp::MinBlepTable` in `crates/synth_dsp/src/oscillators.rs`

**Source:** [Waveform Generator Using MinBLEPs](https://github.com/bdejong/musicdsp/blob/master/source/Synthesis/112-waveform-generator-using-minbleps.rst)

**What exists:** A `MinBlepTable` struct that generates a pre-computed minimum-phase
band-limited step table (windowed sinc → integrated). Has `new()`, `lookup()`, and
`length_samples()` methods.

**What's needed to expose:**
1. Add an anti-aliasing mode parameter to the Oscillator module (`OscillatorParam::AaMode`)
   with choices: `PolyBLEP` (current default), `MinBLEP`
2. In `oscillator.rs` process(), when MinBLEP mode is selected:
   - Create a static/lazy `MinBlepTable` (one-time allocation at init, not in process)
   - At each discontinuity (sawtooth reset, square edge), insert the MinBLEP correction
     from the table instead of the PolyBLEP polynomial
3. Add the parameter to the Oscillator descriptor

**Effort:** Medium — requires understanding the discontinuity insertion pipeline in the
existing oscillator.

---

## 2. Linkwitz-Riley Crossover Module

**Code:** `synth_dsp::LinkwitzRiley` in `crates/synth_dsp/src/filters.rs`

**Source:** [4th-Order Linkwitz-Riley Filters](https://github.com/bdejong/musicdsp/blob/master/source/Filters/266-4th-order-linkwitz-riley-filters.rst)

**What exists:** A `LinkwitzRiley` struct with `process(input, cutoff, sample_rate) -> (lp, hp)`
that provides phase-coherent LP + HP splitting (sum to unity).

**What's needed to expose:**
1. Create a new voice module `CrossoverSplitter` (or effect module)
   - Parameters: `CrossoverFreq` (Hertz), `Mix` (NormalizedValue)
   - Ports: audio input, LP output, HP output
2. Register `ModuleType::CrossoverSplitter` in params/mod.rs (all methods)
3. Create param enum `CrossoverParam`
4. Add to `module_factory.rs` and `ALL_MODULE_TYPES`
5. In process(), call `LinkwitzRiley::process()` and route LP/HP to respective outputs

**Effort:** Medium — standard new module pattern, straightforward.

---

## 3. LPC Analysis (Vocoder Foundation)

**Code:** `math::autocorrelation()`, `math::levinson_durbin()`, `math::lpc_analysis()`
in `crates/synth_modules/src/math.rs`

**Source:** [LPC Analysis](https://github.com/bdejong/musicdsp/blob/master/source/Analysis/137-lpc-analysis-autocorrelation-levinson-durbin-recursion.rst)

**What exists:** Functions for autocorrelation and Levinson-Durbin recursion that extract
LPC filter coefficients from audio. Documented in PitchTracker as NOT RT-safe (returns `Vec`).

**What's needed to expose:**
1. **Cannot be called in `process()`** — `lpc_analysis()` allocates a `Vec` (heap)
2. Options:
   a. Pre-allocate fixed-size buffers (e.g., `[f32; 32]` for order 32) and rewrite
      `levinson_durbin` to use them → becomes RT-safe
   b. Run LPC in a background thread, send coefficients to audio thread via ring buffer
   c. Use as offline-only analysis (e.g., in a vocoder module that analyzes at block
      boundaries with pre-allocated storage)
3. Create a `Vocoder` or `LpcFilter` module that applies the extracted coefficients as
   an all-pole filter to a carrier signal (classic vocoder architecture)

**Effort:** Large — requires RT-safe rewrite and vocoder architecture design.

---

## 4. Dynamic Convolution

**Code:** `math::dynamic_convolution_weights()`, `math::buffer_rms()` in
`crates/synth_modules/src/math.rs`

**Source:** [Dynamic Convolution](https://github.com/bdejong/musicdsp/blob/master/source/Effects/207-dynamic-convolution.rst)

**What exists:** Helper functions for selecting/crossfading between impulse responses
based on input amplitude. Documented in Convolver as requiring multi-IR support.

**What's needed to expose:**
1. Extend `Convolver` to support **multiple IR buffers** (e.g., 2-4 at different levels)
   - Add `ConvolverParam::DynamicMode(bool)` to enable/disable
   - Store multiple `PartitionedConvolver` instances (one per IR level)
2. In process():
   - Call `buffer_rms()` on the input block
   - Call `dynamic_convolution_weights()` to get crossfade between two adjacent IRs
   - Process input through both convolvers, crossfade outputs
3. Add IR loading for multiple levels (requires UI for selecting multiple files)
4. Add parameters to Convolver descriptor

**Effort:** Large — multi-IR management, increased memory usage, UI for multiple IR loading.

---

## 5. Envelope Curve Shapes

**Code:** `math::cubic_envelope()`, `math::parabolic_envelope()` in
`crates/synth_modules/src/math.rs`

**Source:** [Cubic Polynomial Envelopes](https://github.com/bdejong/musicdsp/blob/master/source/Synthesis/15-cubic-polynomial-envelopes.rst),
[Inverted Parabolic Envelope](https://github.com/bdejong/musicdsp/blob/master/source/Synthesis/14-inverted-parabolic-envelope.rst)

**What exists:** Two functions for shaped envelope curves — `cubic_envelope(t, curve)` for
logarithmic/linear/exponential shapes, and `parabolic_envelope(t)` for bell curves.

**What's needed to expose:**
1. Add `EnvelopeParam::AttackCurve(NormalizedValue)` and `DecayCurve(NormalizedValue)`
   parameters to the Envelope module (or a single `Curve` parameter)
2. In the Envelope's stage progression, apply `cubic_envelope(t, curve)` to shape the
   transition between levels instead of the current linear/exponential ramp
3. Optionally add `parabolic_envelope` as a one-shot envelope shape option
4. Update the Envelope descriptor with the new curve parameters

**Effort:** Small — the Envelope module already has stage progression logic; just wrap
the time position through `cubic_envelope()` before applying it.
