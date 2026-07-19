# References

## DSP & Audio Programming

- [Awesome Audio DSP](https://github.com/BillyDM/awesome-audio-dsp) — Curated list of audio DSP and plugin development
  resources, including tutorials, libraries, and tools for learning and building audio software.
- [Music-DSP Source Code Archive](https://www.musicdsp.org/en/latest/index.html) — Community-driven collection of DSP
  algorithms and code snippets for music applications. Covers filters, effects, synthesis, and analysis. Many individual
  modules are implemented directly from specific archive entries; the in-code links point at the
  [`bdejong/musicdsp`](https://github.com/bdejong/musicdsp) GitHub mirror. See the per-module map below.

## SID (MOS 6581/8580) emulation

References used for the `sid` oscillator module. Fidelity was measured against **reSID**; the combined-waveform and
DAC-nonlinearity models follow **reSIDfp**'s parametric resistor-network approach — but the coefficients are
independently derived from circuit analysis, with **no emulator tables copied** (license isolation).

- [reSID](https://github.com/daglem/reSID) — Dag Lem's cycle-accurate MOS 6581/8580 emulator. The fidelity reference the
  module was A/B-measured against (24-bit accumulator, LFSR noise, combined-waveform bus behaviour, jitter-free
  clock-domain conversion).
- [reSIDfp / libsidplayfp](https://github.com/libsidplayfp/libsidplayfp) — Floating-point reSID successor. Source of the
  parametric R-2R DAC-nonlinearity and combined-waveform ("option C") modelling approach reproduced here from first
  principles.
- [VICE — the Versatile Commodore Emulator](https://vice-emu.sourceforge.io/) — Reference C64 emulator suite that
  bundles
  reSID; the baseline for chip behaviour and the golden A/B measurement path.

## Filters & EQ

- [Audio EQ Cookbook (RBJ)](https://www.w3.org/TR/audio-eq-cookbook/) — Robert Bristow-Johnson's biquad coefficient
  formulas. Source of the peaking / low-shelf / high-shelf EQ coefficients in `synth_dsp::filters` (and the gain
  conversion helper in `synth_modules::math`).

## Voice & formant synthesis

- **FOF / CHANT — Xavier Rodet, IRCAM (1984).** *Fonction d'Onde Formantique* (formant-wave-function) granular
  synthesis:
  each formant is a short enveloped grain re-triggered once per F0. The model behind the `Fof` module (`fof.rs`).
- **Kelly–Lochbaum vocal-tract waveguide — Kelly & Lochbaum (1962).** 1-D scattering-junction waveguide where reflection
  coefficients follow the cross-sectional area profile. Backs the `VocalTract` module (`synth_dsp::waveguide`).
  Reference
  lineage also draws on [Pink Trombone (Neil Thapen)](https://dood.al/pinktrombone/) and Voc (Paul Batchelor).
- [PADsynth — Paul Nasca (ZynAddSubFX)](https://zynaddsubfx.sourceforge.io/doc/PADsynth/PADsynth.htm) — Wavetable
  synthesis by inverse-FFT of a per-harmonic bandwidth-spread spectrum. Behind the `PadSynth` module (`padsynth.rs`;
  also
  mirrored as Music-DSP entry `Synthesis/213`).

## Tuning & microtonality

- [Scala scale file format (Huygens-Fokker / Manuel Op de Coul)](https://www.huygens-fokker.org/scala/scl_format.html) —
  The `.scl` scale and `.kbm` keyboard-mapping formats parsed by `synth_core::tuning` for arbitrary microtonal imports
  (alongside built-in equal-temperament, just-intonation, and Pythagorean tunings).

## Classic named techniques (cited by name)
 
Well-known methods reproduced from their standard descriptions, referenced by name in the code comments:

- **Karplus–Strong** plucked-string physical modelling — `math_oscillator.rs`.
- **Moog** transistor-ladder filter (4-pole, 24 dB/oct) — `filter.rs`.
- **Voss–McCartney** pink-noise generator — `noise.rs`.
- **Bode frequency shifter** — Hilbert-transform all-pass pair + single-sideband modulation — `frequency_shifter.rs`.
- **Feedback Delay Network (FDN)** reverb with a Hadamard mixing matrix (successor to the classic Schroeder
  comb/all-pass
  reverb) — `synth_dsp::fdn`, `effects/reverb.rs`.
- **Weierstrass function** fractal additive synthesis — `fractal_osc.rs`.

## Algorithm implementations from the Music-DSP archive

Each of these modules is implemented from a specific
[Music-DSP archive](https://github.com/bdejong/musicdsp/blob/master/source/) entry (path relative to `…/source/`; each
file carries the exact link):

| Module                               | Technique                               | Archive entry                                                         |
|--------------------------------------|-----------------------------------------|-----------------------------------------------------------------------|
| `oscillator.rs`, `oscillators.rs`    | MinBLEP band-limited step               | `Synthesis/112-waveform-generator-using-minbleps`                     |
| `oscillators.rs`, `sub_osc.rs`       | DSF (super-set of BLIT)                 | `Synthesis/140-dsf-super-set-of-blit`                                 |
| `lfo.rs`                             | Smooth random LFO                       | `Synthesis/269-smooth-random-lfo-generator`                           |
| `drift_generator.rs`                 | Drift generator                         | `Synthesis/183-drift-generator`                                       |
| `chaotic_osc.rs`                     | Rössler & Lorenz oscillators            | `Synthesis/184-rossler-and-lorenz-oscillators`                        |
| `am_formant.rs`                      | AM formantic synthesis                  | `Synthesis/224-am-formantic-synthesis`                                |
| `padsynth.rs`                        | PADsynth method                         | `Synthesis/213-padsynth-synthesys-method`                             |
| `fooglers.rs`                        | "Weird synthesis"                       | `Synthesis/11-weird-synthesis`                                        |
| `math.rs`                            | Cubic / inverted-parabolic envelopes    | `Synthesis/15-…`, `Synthesis/14-inverted-parabolic-envelope`          |
| `formant_filter.rs`                  | Formant filter                          | `Filters/110-formant-filter`                                          |
| `filter.rs`, `synth_dsp::filters`    | Karlsen fast ladder                     | `Filters/240-karlsen-fast-ladder`                                     |
| `crossover.rs`, `synth_dsp::filters` | 4th-order Linkwitz–Riley                | `Filters/266-4th-order-linkwitz-riley-filters`                        |
| `tilt_eq.rs`                         | Simple tilt equalizer                   | `Filters/267-simple-tilt-equalizer`                                   |
| `distortion.rs`, `math.rs`           | Variable-hardness clipping              | `Effects/104-variable-hardness-clipping-function`                     |
| `mid_side.rs`                        | Stereo-field rotation matrix            | `Effects/255-stereo-field-rotation-via-transformation-matrix`         |
| `univibe.rs`                         | Univibe emulator                        | `Effects/277-univox-univibe-emulator`                                 |
| `math.rs`                            | Early-echo image-mirror technique       | `Effects/74-early-echo-s-with-image-mirror-technique`                 |
| `math.rs`                            | Dynamic convolution                     | `Effects/207-dynamic-convolution`                                     |
| `beat_detector.rs`                   | Beat-detector class                     | `Analysis/200-beat-detector-class`                                    |
| `vocoder.rs`, `math.rs`              | LPC (autocorrelation / Levinson–Durbin) | `Analysis/137-lpc-analysis-autocorrelation-levinson-durbin-recursion` |
| `math.rs`                            | Goertzel tone detection                 | `Analysis/107-tone-detection-with-goertzel`                           |
| `math.rs`                            | Noise-shaping / dither                  | `Other/99-noise-shaping-class`                                        |
| `math.rs`                            | Rational tanh approximation             | `Other/238-rational-tanh-approximation`                               |
