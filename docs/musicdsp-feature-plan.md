# MusicDSP Feature Plan — Complete

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

## Status: 26/26 fully exposed

All 26 algorithms are implemented and exposed in the UI/MCP.

---

## 1. MinBLEP Oscillator Anti-Aliasing — DONE

**Code:** `synth_dsp::MinBlepTable` in `crates/synth_dsp/src/oscillators.rs`

**Source:** [Waveform Generator Using MinBLEPs](https://github.com/bdejong/musicdsp/blob/master/source/Synthesis/112-waveform-generator-using-minbleps.rst)

**Completed:** Added `AntiAliasMode` enum (`PolyBlep`/`MinBlep`) and
`OscillatorParam::AntiAlias` parameter. A global `LazyLock<MinBlepTable>` provides
RT-safe lookups. `generate_single_sample()` branches on mode for Sawtooth, Square,
and Pulse waveforms. Triangle keeps PolyBLAMP in both modes.

---

## 2. Linkwitz-Riley Crossover Module — DONE

**Code:** `synth_dsp::LinkwitzRiley` in `crates/synth_dsp/src/filters.rs`

**Source:** [4th-Order Linkwitz-Riley Filters](https://github.com/bdejong/musicdsp/blob/master/source/Filters/266-4th-order-linkwitz-riley-filters.rst)

**Completed:** New `CrossoverSplitter` effect module with `CrossoverParam`
(Frequency, LowGain, HighGain, Mix). Uses stereo `LinkwitzRiley` pairs for
phase-coherent LP/HP band splitting with independent gain control.

---

## 3. LPC Analysis (Vocoder) — DONE

**Code:** `math::autocorrelation()`, `math::levinson_durbin()`, `math::lpc_analysis()`
in `crates/synth_modules/src/math.rs`

**Source:** [LPC Analysis](https://github.com/bdejong/musicdsp/blob/master/source/Analysis/137-lpc-analysis-autocorrelation-levinson-durbin-recursion.rst)

**Completed:** Added RT-safe variants (`autocorrelation_fixed`, `levinson_durbin_fixed`,
`lpc_analysis_fixed`) using `[f32; MAX_LPC_ORDER]` fixed-size arrays — no heap
allocations. New `Vocoder` effect module with `VocoderParam` (Order, WindowSize, Mix)
that analyzes spectral envelope via LPC and applies it as an all-pole filter.

---

## 4. Dynamic Convolution — DONE

**Code:** `math::dynamic_convolution_weights()`, `math::buffer_rms()` in
`crates/synth_modules/src/math.rs`

**Source:** [Dynamic Convolution](https://github.com/bdejong/musicdsp/blob/master/source/Effects/207-dynamic-convolution.rst)

**Completed:** Extended `Convolver` with `ConvolverParam::DynamicMode(NormalizedValue)`.
Three IR levels (soft/medium/loud) with independent `PartitionedConvolver` pairs.
When dynamic mode > 0, all three convolvers run and outputs crossfade based on
`buffer_rms()` and `dynamic_convolution_weights()`. Dynamic amount blends between
static and amplitude-dependent convolution.

---

## 5. Envelope Curve Shapes — DONE

**Code:** `math::cubic_envelope()`, `math::parabolic_envelope()` in
`crates/synth_modules/src/math.rs`

**Source:** [Cubic Polynomial Envelopes](https://github.com/bdejong/musicdsp/blob/master/source/Synthesis/15-cubic-polynomial-envelopes.rst),
[Inverted Parabolic Envelope](https://github.com/bdejong/musicdsp/blob/master/source/Synthesis/14-inverted-parabolic-envelope.rst)

**Completed:** `EnvelopeParam::AttackCurve(BipolarValue)`, `DecayCurve(BipolarValue)`,
and `ReleaseCurve(BipolarValue)` are implemented and exposed. Curve shaping is applied
in the Envelope's `process_sample()` via `apply_curve_shaping()` (power-function approach)
rather than `cubic_envelope()`. The `cubic_envelope()` and `parabolic_envelope()` helper
functions remain available as unused utilities.
