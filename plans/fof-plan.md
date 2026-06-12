# FOF Generator — CHANT-style Formant-Wave-Function Voice Module Plan

Status: **Not started.** Optional third voice engine alongside the shipped
`VoiceSynth` (source–filter) and `VocalTract` (Kelly–Lochbaum). See
`plans/voice-synth-plan.md` (Phase 6 design note: *"A genuine-FOF generator
could later become a third module if wanted"*). One phase per session.

## Goal

Add `ModuleType::Fof`: a singing-voice generator that builds formants directly
in the **time domain** from overlapping granular **formant wave functions**
(FOF / *Fonction d'Onde Formantique*, IRCAM CHANT, Rodet 1984), rather than by
filtering a source like `VoiceSynth` does.

```
F0 trigger ──▶ per period, fire one FOF grain per formant ──▶ sum overlapping grains ──▶ out
                         each grain = sine(formant Fc) × excitation envelope
```

The three engines stay **separate** — user picks per instrument, none tries to
do everything:

| Module | Engine | Domain |
|--------|--------|--------|
| `VoiceSynth` | source–filter (glottal pulse → bandpass bank) | frequency / filter |
| `VocalTract` | Kelly–Lochbaum waveguide | physical / waveguide |
| **`Fof`** (new) | FOF grains (CHANT) | time / granular |

### Why FOF at all (vs the proven source–filter we already ship)

Perceptually a parallel bandpass bank ≈ FOF for sustained vowels, which is why
source–filter was built first. FOF adds: independent per-formant control of
attack/skirt/bandwidth, the characteristic CHANT "shimmer," cleaner pitch/vowel
decoupling, and natural per-grain stochastics. It is a **refinement, not a
gap** — hence optional.

## The DSP, precisely

A FOF grain is one formant's impulse response, retriggered once per F0 period:

```
fof(t) = A · env(t) · sin(2π·Fc·t + φ)

env(t) = 0                                  for t < 0
       = ½(1 − cos(π·t/tex)) · e^(−β·t)     for 0 ≤ t < tex     (raised-cosine attack into…)
       = e^(−β·t)                           for t ≥ tex          (…exponential decay)
```

- **Fc** — formant center frequency (Hz). Sets the formant.
- **β = π·BW** — exponential decay rate; controls formant **bandwidth** (fast
  decay = wide formant).
- **tex** — excitation/attack time; shapes the formant **skirt** (steepness of
  the high-frequency rolloff). Classic CHANT exposes this as a knob.
- **A, φ** — grain gain and phase per formant (from the vowel table).

Per F0 period we trigger a fresh grain **per formant** (4 bands, reusing
`formant_tables`). Grains from successive periods overlap and sum. Because a
grain can live many periods (low F0 / long decay), several grains per formant
must coexist → a **fixed ring of active grains per formant**, allocated in
`new()`, never grown in `process()`.

Pitch = grain **trigger rate** (F0); vowel = grain **inner shape** (Fc/BW/skirt).
Fully decoupled — the whole point.

## What to reuse (don't reinvent)

| Need | Existing |
|------|----------|
| Vowel tables A/E/I/O/U × 4 bands (freq, BW, gain) | `synth_modules::formant_tables` (`FORMANT_FREQ/BW/GAIN`, `NUM_VOWELS=5`, `NUM_BANDS=4`) — same source the other two engines use |
| Vibrato, `pitch_cv`/`vowel_cv`/`breath_cv` ports, choir/unison decorrelation, stereo `out_l`/`out_r`, `MAX_UNISON=16` sub-voice array | `crates/synth_modules/src/voice_synth.rs` — lift the structure wholesale |
| `fast_sin_turns`, clip/saturate | `crates/synth_modules/src/math.rs` |
| Noise (breath/aspiration) | `crates/synth_modules/src/noise.rs` |
| Newtypes (`Hertz`, `Phase`, `Cents`, `NormalizedValue`, `Seconds`, …) | `synth_core` |

Generator → `ModuleCategory::Oscillator`, one instance per polyphony voice —
exactly like `VoiceSynth`/`AmFormant`.

## Ports & parameters

Ports mirror `VoiceSynth`: `out` (mono sum), `out_l`/`out_r` (stereo spread),
`pitch_cv`, `vowel_cv`, `breath_cv`.

Parameters (`FofParam`, all newtyped — mirror `params/voice_synth.rs`):

| Param | Type | Range | Notes |
|-------|------|-------|-------|
| `Vowel` | `NormalizedValue` | 0..1 | A→E→I→O→U morph |
| `FormantShift` | `NormalizedValue`/`Semitones` | −12..+12 st | tract length / voice type |
| `Skirt` | `NormalizedValue` | 0..1 | grain `tex` — FOF's signature knob (no equivalent in the filter engine) |
| `Bandwidth` | `NormalizedValue` | 0..1 | global BW scale on top of table |
| `Breathiness` | `NormalizedValue` | 0..1 | aspiration noise mix |
| `VibratoRate` | `Hertz` | 0..12 | |
| `VibratoDepth` | `Cents` | 0..100 | |
| `UnisonVoices` | `NormalizedValue`→int | 1..16 | choir size |
| `UnisonDetune` | `Cents` | 0..50 | spread |
| `UnisonSpread` | `NormalizedValue` | 0..1 | stereo width |
| `Level` | `NormalizedValue` | 0..1 | output level |

## Registration checklist (same touch points as VoiceSynth)

New files:
1. `crates/synth_core/src/params/fof.rs` — `FofParam` enum (`same_kind`, `name`,
   `as_f32`, `with_f32`, `Default`; mirror `params/voice_synth.rs`).
2. `crates/synth_modules/src/fof.rs` — the `Fof` module (mirror `voice_synth.rs`).

Edits in `crates/synth_core/src/params/mod.rs` — add an arm in **each** (follow
the existing `VoiceSynth` lines, grep `VoiceSynth` to find them):
`pub mod fof;` + `pub use`, `ModuleType::Fof` variant, `is_voice_module()` arm,
display name `"FOF"`, short key (e.g. `"fof"`), `from` key str, `Param::Fof(...)`
variant, `same_kind`, `module_type()`, `name()`, `as_f32()`, `with_f32()`.

Elsewhere:
- `crates/synth_modules/src/lib.rs` — `pub mod fof;` + re-export `Fof`.
- `crates/pertylizer/src/module_factory.rs` — `ModuleType::Fof => { ... }` in
  `create_voice_module()` **and** add to `ALL_MODULE_TYPES`.
- MCP whitelist: confirm `Fof` is included wherever VoiceSynth/VocalTract are
  listed (the same whitelist that once omitted them — see `docs/history.md`).

## Phased implementation (one per session)

**Phase 1 — Skeleton + single-grain FOF + formant bank (mono).** New param enum;
register everywhere; per-formant FOF grain generator (raised-cosine attack →
exp decay) triggered at F0, summed over a fixed active-grain ring; copied
A/E/I/O/U tables; `Level`, `Vowel`, `FormantShift`, `Skirt`. Silence when no
note. `cargo build`/`clippy`/`test` green; `produces_sound` + `silent_without_note`
tests like `voice_synth`'s.

**Phase 2 — Expressivity.** `Bandwidth`, breathiness (noise), vibrato
(rate/depth), `pitch_cv`/`vowel_cv`/`breath_cv`. Verify vowel morph audibly
differs and `Skirt`/`Bandwidth` change formant shape distinctly from VoiceSynth.

**Phase 3 — Choir / unison.** Lift the `MAX_UNISON` sub-voice array, per-voice
detune/vibrato/onset/pan, stereo `out_l`/`out_r`. RT-safety review (no heap in
`process()`; fixed grain rings × sub-voices). Test: N>1 wider than N=1.

**Phase 4 — Voice types & presets + patch.** SATB via `FormantShift` + F0 range.
Add `patches/fof_choir.rs` (or fold into existing choir patch) and wire into the
example list. A/B it against the VoiceSynth choir patch.

**Phase 5 — Polish.** MCP descriptions, tags (`vocal`, `voice`, `choir`, `fof`,
`chant`), `docs/history.md` entry, `new version` bump, mark this plan + TODO.

## Real-time safety notes
- Active grains held in **fixed-size rings** per formant per sub-voice
  (`[[Grain; MAX_GRAINS]; NUM_BANDS]`), sized in `new()` from the lowest
  supported F0 × longest decay. `process()` only advances/retires grains.
- Recompute grain coefficients **only on F0/param change**, not per sample.
- No `unwrap`/`expect`; `unwrap_or(0.0)` for port reads; for-loops in the hot path.

## Open questions
- **Grain ring size**: worst case = (longest grain lifetime in samples) ÷
  (shortest F0 period). Bound MAX_GRAINS from the lowest musical F0 (~50 Hz bass)
  and cap decay; profile in Phase 1.
- **Analytic grain eval vs precomputed grain wavetable** rebuilt only on
  Oq/skirt change — decide in Phase 1 by profiling (same call the VoiceSynth plan
  left open for its LF source).
- **Share a `voice_common` module?** VoiceSynth and FOF will duplicate vibrato +
  unison/choir + port handling. Consider extracting after Phase 3 once the
  duplication is concrete (don't pre-abstract).
- **Skirt ⇄ Bandwidth interaction**: both touch the spectral envelope; pick
  ranges so they feel independent, not redundant.
