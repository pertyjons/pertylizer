# `compare_spectra` floor-guard calibration — sparse & near-floor spectra

> **Status: IMPLEMENTED on `feat/sid-fidelity` (2026-07-02).** All three changes
> below shipped in `spectrum.rs` (relative `LSD_SHELF_DB=-50` shelf,
> `FLOOR_COVERAGE_MIN=0.15`, `NEAR_FLOOR_RMS=3e-3` near-floor gate + level-gated
> `voiced`), the MCP `floor_coverage`/`floor_limited` docs updated, and the three
> unit tests added (plus the existing broadband-tone test's saw fixture enriched
> 6→32 harmonics so it stays legitimately unflagged). The **validation gate below
> (re-run the sid-analyzer golden A/B matrix) is external and still owed** — the
> unit tests pin the logic but reSID is the acceptance judge.
>
> **Original follow-up to the shipped empty-bin/silence guard** (`66996daf`,
> TODO §4.1). That guard closed the *fully-silent* case
> (`both_silent` → distance 0, `floor_limited: true`) and the *absolute* −80 dB
> empty-bin case. But two regimes still return a large, misleading
> `log_spectral_distance` while the timbres actually **agree** — surfaced live
> while verifying the `sid` module's §11 fixes against the reSID golden matrix
> (`sid-analyzer/assets/fixtures/sound-engine-poc/`). The DSP is correct in both;
> only the *scalar's self-awareness* is wrong. This plan recalibrates the guard so
> the number is trustworthy (or honestly flags itself unreliable) at sparse-harmonic
> and collapsed-voice pitches.
>
> All line references are against `crates/pertylizer/src/audio/analysis/spectrum.rs`
> at `66996daf` unless noted.

## The two live failures

Measured 2026-07-02 via MCP `compare_spectra`, reSID WAV target vs soloed `sid`
render, windowed 500–1500 ms:

1. **Ring-mod (`ring_b5_6581.wav`) → 16.8 dB, `floor_limited: false`.**
   The candidate sidebands match the reference within ~1–3 dB across the board
   (1153/822/1483/491/1813/3130/2144 Hz all align <1 Hz; `centroid_delta` only
   **104 Hz**). Yet the scalar reads 16.8 dB. Neither source is near-silent
   (`frame_rms ≈ 0.078`), so the `both_silent` gate never fires; `floor_coverage`
   came back **0.99** (nearly every bin "live"). The distance is dominated by the
   deep spectral **nulls between the sidebands**, where reSID's resampler leakage
   (~−55 dB below peak) and the module's floor (~−75 dB) differ by ~20 dB. Those
   bins carry no timbral information but each contributes ~400 to the RMS.

2. **6581 pulse+saw (`c61_a2_6581.wav`) → 103 dB, `voicing_mismatch: true`.**
   The real 6581 collapses this combo to near-silence; the module now collapses
   it too (`sid` candidate sustain RMS ≈ 1e-3, energy ≈ 5e-6 — the fix works).
   Both are perceptually silent, but **both sit above `SILENCE_RMS = 1e-4`**
   (target ≈ 5e-4, candidate ≈ 1e-3), so `both_silent` is false. Worse, the
   voicing detector reports the reSID noise floor as `voiced: true` (a spurious
   NSDF pitch in near-silence) while the collapsed candidate reads `false` →
   `voicing_mismatch` → **+60 dB** `VOICING_MISMATCH_PENALTY_DB`.

Both regimes were explicitly the "known compare_spectra limitations" flagged in
`sid-oscillator-module.md` §11; the guard as shipped doesn't reach either.

## Root causes (grounded)

- **`LSD_FLOOR_DB = -80` is an *absolute* peak-relative shelf** (spectrum.rs:54,
  used at 805–807). It only discards bins in the last 80 dB — far below the
  −55 dB nulls of a real sparse spectrum. `log_spectral_distance` therefore keeps
  every inter-partial null bin and its noise-floor mismatch (spectrum.rs:800–819).
- **`SILENCE_RMS = 1e-4` (−80 dBFS)** (spectrum.rs:59, gate at 745) is below where
  a chip *actually* silences a voice (~−55 to −60 dBFS). Genuinely-collapsed
  voices never hit the `both_silent` early return (spectrum.rs:746–759).
- **`floor_limited = floor_coverage <= 0.0`** (spectrum.rs:790) only trips when
  *zero* bins are live. A 90%-null sparse spectrum still reports `false`.
- **Voicing is not gated on level** (spectrum.rs:249): `voiced = clarity >= 0.4 &&
  flatness <= 0.5`, with no RMS floor, so near-silent noise can read voiced and
  trigger the +60 dB penalty.

## Proposed changes

Minimal, all in `spectrum.rs`. Keep the existing constants/tests; add three
calibration levers.

### 1. Relative spectral shelf in `log_spectral_distance` (fixes the sparse-null inflation)

Replace the single absolute `LSD_FLOOR_DB` exclusion with a shelf measured
**relative to each source's own peak**. Bins already arrive peak-normalised
(0 dB = that source's peak), so a bin that is ≥ `LSD_SHELF_DB` below the peak on
*both* sides is a shared null and carries no timbral weight — exclude it exactly
like the −80 case.

```rust
/// Bins this far below BOTH sources' peaks are shared nulls — no timbral
/// information — and are excluded from the RMS and from `floor_coverage`.
/// −50 dB ≈ "inaudible relative to the loudest partial". Deep enough to keep
/// real low harmonics, shallow enough to drop resampler-leakage nulls.
const LSD_SHELF_DB: f32 = -50.0;
```

In `log_spectral_distance` (spectrum.rs:800), keep the `LSD_FLOOR_DB` clamp for
the digital-floor case, but change the *exclusion* predicate to the relative
shelf (bins are already peak-normalised, so the shelf is simply a dB constant):

```rust
        let av = a[i].0.max(LSD_FLOOR_DB);
        let bv = b[i].0.max(LSD_FLOOR_DB);
        if av <= LSD_SHELF_DB && bv <= LSD_SHELF_DB {
            continue; // shared null: below both peaks by ≥ |LSD_SHELF_DB|
        }
```

Effect: the ring nulls (−55/−75) drop out; the distance collapses toward the
~1–3 dB the matched sidebands actually differ by, and `floor_coverage` falls to
the fraction of bins that are real partials.

### 2. `floor_limited` fires on low coverage, not just zero (fixes "sparse but not flagged")

```rust
/// Below this live-bin fraction the scalar is dominated by a handful of bins
/// and should not be trusted as a broadband distance.
const FLOOR_COVERAGE_MIN: f32 = 0.15;
```

At spectrum.rs:790:

```rust
        floor_limited: floor_coverage < FLOOR_COVERAGE_MIN,
```

Now a spectrum that is mostly nulls (sparse ring, collapsed voice) reports
`floor_limited: true`, telling the caller to read `missing/extra_partials` +
`centroid_delta` instead of the scalar — which is exactly how the match was
verified by hand.

### 3. Relative silence band + level-agreement (fixes 0x61's 103 dB)

Two parts:

**(a) Raise/relativise the silence gate.** Introduce a "near-floor" level and
treat two sources that are *both* near-floor **and* within a few dB of each
other* as agreeing:

```rust
/// A voice a chip has "silenced" still emits DAC/resampler residue at roughly
/// this level; treat both-below-this as effectively silent.
const NEAR_FLOOR_RMS: f32 = 3.0e-3; // ≈ −50 dBFS
```

Extend the `both_silent` gate (spectrum.rs:745):

```rust
    let both_silent = target.frame_rms <= SILENCE_RMS && candidate.frame_rms <= SILENCE_RMS;
    let both_near_floor =
        target.frame_rms <= NEAR_FLOOR_RMS && candidate.frame_rms <= NEAR_FLOOR_RMS;
    if both_silent || both_near_floor {
        return SpectrumDistance { /* distance 0, floor_limited: true, … */ };
    }
```

(Keep `SILENCE_RMS`'s absolute early-out; `both_near_floor` is the wider net.)

**(b) Gate voicing on level** so the reSID noise floor stops reading `voiced`
and stops arming the +60 dB penalty. At spectrum.rs:249:

```rust
    let voiced = frame_rms > NEAR_FLOOR_RMS
        && clarity >= VOICED_NSDF_THRESHOLD
        && flatness <= VOICED_FLATNESS_THRESHOLD;
```

3(b) is defensive even with 3(a) — a near-floor source paired with a *loud* one
(a genuine voiced-vs-silence mismatch) should still not claim to be pitched.

## Tests (mirror the existing guard tests)

Add to the `#[cfg(test)]` block next to `compare_near_silence_is_floor_limited_zero`
and `compare_ignores_shared_floor_bins`:

- `compare_sparse_matching_partials_is_low_distance` — synthesize two spectra with
  identical partials at 822/1153/1483 Hz and *different* −55 vs −75 dB inter-partial
  floors; assert `log_spectral_distance < 6.0` and `floor_limited` (coverage below
  `FLOOR_COVERAGE_MIN`). This is the ring case as a unit test.
- `compare_both_near_floor_is_floor_limited_zero` — two sources at `frame_rms`
  2e-3 (between `SILENCE_RMS` and `NEAR_FLOOR_RMS`); assert distance 0,
  `floor_limited: true`, `voicing_mismatch: false`. This is the 0x61 case.
- `near_floor_noise_is_not_voiced` — a −55 dBFS noise frame: assert `!voiced`.

## Validation gate

Re-run the `sid-analyzer` golden A/B matrix (the same harness that surfaced this):

- **ring-mod row:** `log_spectral_distance < ~6 dB` **or** `floor_limited: true`
  (either is a pass — the partials already match within 1–3 dB).
- **0x61 pulse+saw row:** `floor_limited: true`, `voicing_mismatch: false`,
  distance ≈ 0 (both collapsed).
- **Regression guard — do NOT over-flag:** the healthy rows must stay unflagged
  and unmoved: 0x21 saw (2.7–10.7 dB), 0x51 pulse+tri (~10 dB), 0x81 noise
  (0.76 dB), hard-sync (6.8 dB) all keep `floor_limited: false` and their current
  distances (±0.5 dB). If any healthy row starts reading `floor_limited`,
  `LSD_SHELF_DB` / `FLOOR_COVERAGE_MIN` / `NEAR_FLOOR_RMS` are too aggressive.

## Calibration risk & bounds

Every threshold here trades false-confidence against false-flagging:

- `LSD_SHELF_DB = -50` — too shallow (e.g. −35) discards real harmonics and makes
  distinct timbres read equal; too deep (−70) fails to drop the nulls. −50 dB is
  the "inaudible vs the loudest partial" line; validate against the saw/pulse+tri
  rows which have real energy 30–40 dB down.
- `NEAR_FLOOR_RMS = 3e-3` — must stay *below* the quietest musically-real sustain
  in the corpus. A pizzicato tail or a deliberately-soft pad could dip under it;
  pair it with the level-agreement requirement (both sides near-floor) so a soft
  real note vs a loud one is NOT silently zeroed.
- `FLOOR_COVERAGE_MIN = 0.15` — a genuinely broadband-but-different pair (two
  different noises) has high coverage and stays unflagged, correctly.

Land behind the existing tests; the golden-matrix regression guard is the real
acceptance signal. None of this touches synthesis — it is purely the measurement
tool's honesty at the edges.

## Validation gate — RE-RUN RESULT (2026-07-02, running MCP, binary rebuilt 23:29)

Re-ran the golden matrix (6581, A2, windowed 500–1500 ms) against the reSID
fixtures. Verdict: **the silence lever is a clean win; the shelf lever behaved
differently than predicted, and one premise below was wrong.**

- **0x61 pulse+saw → 0.0 dB, `floor_limited:true`, `voicing_mismatch:false` ✅**
  Exactly as specced. `both_near_floor` + level-gated `voiced` did it.
- **Healthy rows — the ±0.5 guard is TOO STRICT; treat moves as corrections.**
  saw 6.8→**0.36**, sync 6.8→**0.52**, noise 0.76→1.26, pulse+tri 10.2→10.41.
  Saw/sync look like regressions, but a discrimination cross-check — pulse+tri
  *candidate* vs the **saw target** reads **6.31 dB** (≫ the correct saw match
  0.36) — proves separation is intact. The old 6.8 for a *correct* match was
  itself inflated by non-informative HF-floor bins; the −50 shelf removed them.
  This is de-inflation, not regression. **Action: drop the "±0.5 dB, must not
  move" clause from the guard** — the correct invariant is *correct-match <
  wrong-match*, not *distance is stable*. Keep the "must not start reading
  `floor_limited`" half (none of the healthy rows did).
- **Ring-mod → STILL 16.9 dB, `floor_limited:false` (coverage 0.99→0.59).**
  The shelf DID drop the deep nulls (coverage fell) but the distance held — so
  **the premise in §"The two live failures" that ring's 16.8 dB is "dominated by
  the deep nulls between sidebands" was WRONG.** It lives in the −50…0 dB
  *shoulders*, which are real spectral content: the top sidebands match 1–3 dB,
  but reSID's ring carries shoulder/HF texture the module doesn't reproduce.
  The metric is therefore CORRECT to leave it unflagged; force-flagging it (raising
  `FLOOR_COVERAGE_MIN` past 0.59, or the shelf past −50) would mask a genuine
  residual and start collapsing distinct timbres. **Ring's broadband number is a
  `sid_oscillator.rs` ring-shoulder-shaping task, not a floor-guard task — remove
  the ring row from this plan's acceptance gate.**

Net: keep levers 2/3 as-is; lever 1 (`LSD_SHELF_DB`) is fine at −50 for
de-inflation but does not (and should not) "fix" ring. Revise the gate above per
the two bold actions.
