# Voice Synth — Physically-Inspired Singing/Choir Module Plan

Status: **proposed** (not started). One phase per session, in order.

## Goal

Add a new voice-generating module, `ModuleType::VoiceSynth`, that synthesizes a
natural **singing voice** from a physically-inspired **source–filter** model:

```
lung pressure ──▶ glottal source (LF pulse) ──▶ formant resonators (vowel) ──▶ out
                          ▲                              ▲
                    breath noise + vibrato         vowel / brightness
```

and that can decorrelate itself into a **choir** (built-in unison of N voices
with per-voice flutter/wow, formant jitter and stereo spread).

### Why source–filter / FOF (not Kelly–Lochbaum)

For **sung, sustained vowels** the formant source–filter approach (the IRCAM
CHANT / FOF lineage) is the proven, gold-standard technique and is cheap enough
to stack into an ensemble. Kelly–Lochbaum waveguides (Pink Trombone / Voc) sound
more natural for *speech* but are harder to keep in tune and to multiply into a
choir. We therefore build the source–filter model first; a waveguide variant is
an optional later stretch (Phase 6).

References: CHANT/FOF (Rodet, IRCAM); LF glottal model (Liljencrants–Fant 1985);
choir "chorus effect" from per-voice flutter/wow decorrelation (Ternström &
Sundberg). See `docs/references.md` if we add citations there.

## What already exists (reuse, don't reinvent)

| Need | Existing in repo |
|------|------------------|
| Vowel formant tables (A/E/I/O/U: freq, bandwidth, gain) | `crates/synth_modules/src/formant_filter.rs` (`FORMANT_FREQ/BW/GAIN`) — copy as the resonator starting point |
| 2nd-order bandpass (`BandpassCoeffs`/`BandpassState`) | `formant_filter.rs` lines 50–99 — reuse pattern for parallel formant resonators |
| Generator voice skeleton (note_on→freq, phase accumulators, silence-when-no-note) | `crates/synth_modules/src/am_formant.rs` — closest structural template |
| `fast_sin_turns`, `tube_saturate`, `variable_clip`, `soft_clip` | `crates/synth_modules/src/math.rs` |
| Noise generation | `crates/synth_modules/src/noise.rs` |
| Newtypes (`Hertz`, `Phase`, `NormalizedValue`, `MidiNote`, `Velocity`, `SampleRate`, `Decibels`, `Seconds`) | `synth_core` |

The module is a **generator** → `ModuleCategory::Oscillator`, one instance per
polyphony voice, exactly like `AmFormant`.

---

## DSP design

### 1. Glottal source — LF (Liljencrants–Fant) pulse
Per-period waveform driven by F0. Parameters (all per-period, recomputed on F0
change only):
- **Open quotient (Oq)** — fraction of period the glottis is open (breathiness/tension).
- **Spectral tilt / return phase (Ra)** — high-frequency rolloff (soft vs pressed).
- **Asymmetry (αm)** — pulse skew.

Implementation: evaluate the LF closed-form (exponentially-growing sinusoid in
the open phase + exponential return) against a normalized phase accumulator
(`Phase`, like `am_formant`'s `modulator_phase`). Keep it allocation-free; all
coefficients in fixed-size fields. A lookup-free analytic eval is fine at audio
rates; if too costly, fall back to a precomputed per-period wavetable rebuilt
only when Oq/tilt change.

### 2. Resonators — parallel formant bank (FOF-equivalent)
Start with the **parallel biquad bandpass bank** (4–5 formants) using the
existing `BandpassCoeffs` math, fed by the glottal pulse. Formant center
freqs/bandwidths/gains interpolated from an expanded vowel table (extend the
5-vowel A/E/I/O/U set; add at least one "oo/ah/ee" sung-vowel set per voice
type). This is perceptually equivalent to FOF for sustained vowels and reuses
proven code. (True FOF grain generators are an optional Phase 6 swap.)

- **Formant shift** parameter scales all centers together → models vocal-tract
  length → the basis for SATB voice types (short tract = soprano, long = bass).

### 3. Breath / aspiration noise
White/pink noise (from `noise.rs` primitives) shaped by the same formant bank
(or a single broadband resonance), mixed in by a **breathiness** amount. Gate
the noise slightly above the glottal pulse for realism.

### 4. Vibrato
Internal LFO on F0: rate ~5–7 Hz, depth in cents. Add small random
rate/depth/phase per voice for the choir (Phase 4). Vibrato is *essential* for
naturalness — a dead-steady pitch reads as synthetic.

### 5. Amplitude
Envelope is handled by the host patch (connect an `Envelope` → amplifier
`cv_gain`, the standard flow) — the module need not own an ADSR, mirroring
`AmFormant`. It only needs `note_on` (set F0, reset phases) and to output
silence when no note is active.

### 6. Choir / unison (Phase 4)
The ensemble effect comes from **decorrelation**, not a chorus effect:
- `unison_voices` (1..=MAX_UNISON, **fixed array, pre-allocated** — no heap in
  `process()`), e.g. MAX_UNISON = 16.
- Per sub-voice, seeded at `note_on`/`reset`: small detune (cents), independent
  vibrato rate/depth/**phase**, small **formant jitter** (±a few %), onset
  offset (ms), and pan position (stereo spread).
- Sum sub-voices → stereo `out_l`/`out_r` (+ mono `out` sum). The pseudo-random
  amplitude beating of overlapping partials is what the ear hears as "a choir".
- Randomness without `Math.random` constraints: derive per-voice offsets
  deterministically from the sub-voice index + MIDI note (golden-ratio / hashed
  seeds), so it's reproducible and RT-safe.

---

## Ports & parameters

### Ports (`descriptor()`)
- `out` (audio out, mono sum) — always.
- `out_l`, `out_r` (audio out) — for choir stereo spread.
- `pitch_cv` (control in) — F0 / pitch bend offset.
- `vowel_cv` (control in) — modulate vowel position (LFO/Envelope), like
  `FormantFilter`'s `vowel_cv`.
- `breath_cv` (control in) — modulate breathiness.
- (optional) `gate` is implicit via `note_on`/`note_off`, no port needed.

### Parameters (`VoiceSynthParam`, all newtyped)
| Param | Type | Range | Notes |
|-------|------|-------|-------|
| `Vowel` | `NormalizedValue` | 0..1 | A→E→I→O→U morph |
| `FormantShift` | `NormalizedValue` (or `Semitones`) | -12..+12 st | tract length / voice type |
| `Breathiness` | `NormalizedValue` | 0..1 | aspiration noise mix |
| `OpenQuotient` | `NormalizedValue` | 0..1 | glottal Oq (soft↔pressed) |
| `Tilt` | `NormalizedValue` | 0..1 | spectral tilt / return phase |
| `VibratoRate` | `Hertz` | 0..12 | |
| `VibratoDepth` | `Cents` | 0..100 | |
| `UnisonVoices` | `NormalizedValue`→int | 1..16 | choir size |
| `UnisonDetune` | `Cents` | 0..50 | spread |
| `UnisonSpread` | `NormalizedValue` | 0..1 | stereo width |
| `Level` | `NormalizedValue` | 0..1 | output level |

Match `am_formant.rs`'s `ParameterDescriptor::float(...).range().default().unit().widget()`
declaration style and `WidgetHint`/`ResponseCurve` choices.

---

## Registration checklist (exact touch points)

New files:
1. `crates/synth_core/src/params/voice_synth.rs` — `VoiceSynthParam` enum with
   `same_kind`, `name`, `as_f32`, `with_f32`, `Default` (mirror
   `params/formant_filter.rs`).
2. `crates/synth_modules/src/voice_synth.rs` — the `VoiceSynth` module
   (mirror `am_formant.rs`).

Edits in `crates/synth_core/src/params/mod.rs` (add an arm in **each** of these,
following the `FormantFilter`/`AmFormant` lines found at the grep'd locations):
- `pub mod voice_synth;` + `pub use voice_synth::VoiceSynthParam;` (~lines 55/71)
- `ModuleType::VoiceSynth` enum variant (~line 211)
- `is_voice_module()` `matches!` arm (~line 282)
- display name `"Voice Synth"` (~line 417)
- short type key, e.g. `"vox"` (~line 495)
- `from` key str `"vox" => Some(Self::VoiceSynth)` (~line 573)
- `Param::VoiceSynth(VoiceSynthParam)` variant (~line 700)
- `same_kind` arm (~line 784)
- `module_type()` → `ModuleType::VoiceSynth` (~line 860)
- `name()` arm (~line 935)
- `as_f32()` arm (~line 1010)
- `with_f32()` arm (~line 1085)

Edits elsewhere:
- `crates/synth_modules/src/lib.rs` — `pub mod voice_synth;` + re-export
  `VoiceSynth` (keep alpha-ish ordering near `am_formant`).
- `crates/pertylizer/src/module_factory.rs` — `ModuleType::VoiceSynth => { ... }`
  arm in `create_voice_module()` (mirror lines 207–211) **and** add
  `ModuleType::VoiceSynth` to the `ALL_MODULE_TYPES` list (~line 422).

Verify discovery: `list_module_types` / `search_modules` pick it up from
`ALL_MODULE_TYPES`; the GUI inspector gets metadata from the descriptor — no
extra GUI wiring needed for a basic module.

---

## Phased implementation (one per session)

**Phase 1 — Skeleton + glottal source + formant bank (mono).**
New param enum + module file; register everywhere above; LF pulse → parallel
biquad formant bank with the copied A/E/I/O/U tables; `Level`, `Vowel`,
`FormantShift`. Silence when no note. `cargo build`/`clippy`/`test` green; a
`produces_sound` + `silent_without_note` test like `am_formant`'s.

**Phase 2 — Expressivity.** Breathiness (noise path), `OpenQuotient`, `Tilt`,
vibrato (rate/depth), `pitch_cv`/`vowel_cv`/`breath_cv` inputs. Verify vowel
morph audibly differs and vibrato modulates F0.

**Phase 3 — Choir / unison.** Fixed-size sub-voice array, per-voice
detune/vibrato/formant-jitter/onset/pan, stereo `out_l`/`out_r`. RT-safety
review (no heap in `process()`). Test: N>1 produces decorrelated stereo output
wider than N=1.

**Phase 4 — Voice types & presets.** SATB presets via `FormantShift` + F0 range
+ vowel-set tweaks. Add example patches `patches/choir.rs` /
`patches/solo_voice.rs` (see existing `patches/vocal_pad.rs`,
`patches/formant_voice.rs`). Wire into `apply_example_patch`/example list.

**Phase 5 — Polish.** MCP descriptions, tags (`vocal`, `voice`, `choir`),
`docs/history.md` entry, mark TODO. Optional `get_module_type_info` hints.

**Phase 6 (stretch, optional) — True FOF and/or Kelly–Lochbaum variant.** Swap
the biquad bank for genuine FOF grain generators, or add a separate
`ModuleType::VocalTract` Kelly–Lochbaum waveguide (LF source + ~44 scattering
junctions, area-controlled) for *speech*-grade realism. Needs new DSP
primitives (bidirectional delay-line scattering) — scope separately.

---

## Real-time safety notes
- All per-sub-voice state in **fixed-size arrays** (`[_; MAX_UNISON]`),
  allocated in `new()`; `process()` only reads/writes, never grows.
- Recompute LF/formant coefficients **only on parameter/F0 change**, like
  `FormantFilter::update_coeffs` gating on `>0.001` deltas — not per sample.
- No `unwrap`/`expect`; `unwrap_or(0.0)` for port reads; for-loops in the hot
  path.

## Open questions
- LF analytic eval vs per-period wavetable — decide in Phase 1 by profiling.
- Formant count: 4 vs 5 (5th formant adds "singer's formant" ~3 kHz brilliance;
  worth it for choir).
- Should `UnisonVoices` reallocate voices in the engine, or stay self-contained
  inside one module? Plan assumes **self-contained** (simpler, tighter ensemble
  control). Revisit if CPU is an issue.
