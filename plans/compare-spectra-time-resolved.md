# Plan: Time-resolved `compare_spectra` (target-energy-masked distance)

Status: IMPLEMENTED (2026-07-06) on branch `feat/compare-spectra-time-resolved`.
Follow-up to the shipped voicing-penalty un-fold (`30ed9caf`) and the floor-guard
calibration pass. This is "wish (b)" from the sid-analyzer feedback log — the last
piece needed for the SID-export A/B loop to run on MCP tools alone.

Landed in three commits: (1) `compare_time_resolved` pure fn, (2) `envelope_align`
+ `active_time_fraction` pure fns, (3) MCP wiring (`time_resolved` param group on
`compare_spectra`, framed path in `compare_spectra_impl`/`fill_time_resolved`, the
§2.3 honesty warning). The exit-gate A/B run on the archived SID WAVs is left as a
post-merge acceptance check (the ranking is covered by the `spectrum.rs` unit
tests). Known residual MCP gap: unrecognised `mask`/`align` strings default to
"on" silently (only "none" turns them off) — a hardening candidate.

---

## 1. Problem — verified live 2026-07-06 (build 0.315.0 @6bc7d24d)

On silence-dominated, time-sparse material (a 3 s staccato-stab window: short
loud hits, mostly silence/decay between them), the aggregate
`log_spectral_distance` carries **zero ranking signal**, and no guard warns
about it.

Reproduction (slice-7 A/B WAVs: reSID reference vs three sid-analyzer exports
that differ only in release-tail handling):

| Comparison (whole 3 s window)     | LSD          | mel_l2 | floor_coverage | floor_limited |
|-----------------------------------|--------------|--------|----------------|---------------|
| reSID vs export A (slice-7 ON)    | **50.319134**| 61.88  | 1.0            | false         |
| reSID vs export B (gate OFF)      | **50.319134**| 61.55  | 1.0            | false         |
| reSID vs export C (old ON)        | **50.319134**| 59.98  | 1.0            | false         |

Identical to six decimals for three materially different candidates — ground
truth (external voiced-frame masked STFT RMS, and the ear) ranks them
A < B < C. A 500 ms window at 1700 ms shows the same signature (53.696575 for
all three, even at different candidate offsets). Cross-checks against *other*
candidates give distinct values (15.5 / 19.1 dB), so the scalar is **not** a
pure target function — the three exports' *aggregate* spectra really are
identical above the shelf.

### Why (code-anchored)

- `mcp_bridge.rs` `analyze_spectrum_source()` (~10859) analyses each source as
  **one** `analyze_spectrum` call over the whole window — a single
  time-averaged spectrum. In a 3 s window the loud stab peaks dominate the
  average; the release tails (the content that actually differs between the
  candidates, and the thing the slice-7 A/B measures) sit 40–60 dB below the
  stab peak in that average.
- `spectrum.rs` `log_spectral_distance()` (~966): bins below
  `LSD_SHELF_DB = −50` (line 59) on **both** sides are excluded; below
  `LSD_FLOOR_DB = −80` (line 52) they clamp. So the tail content is either
  shelf-excluded or clamped — the scalar is structurally blind to exactly the
  quiet-in-time content being compared.
- `floor_coverage` counts a bin live when it is above the shelf on **either**
  side. The reSID reference has a higher relative floor (chip noise), so every
  bin stays live → coverage 1.0 → `floor_limited: false`. The guard measures
  spectral sparsity, not temporal sparsity — this failure mode is invisible
  to it.

The external workaround that produced the trustworthy numbers: per-frame STFT,
keep frames where the **target** has energy, per-frame log-spectral diff, RMS
over kept frames. Per-frame peak normalisation is what makes tail content
visible — each tail frame's own peak is 0 dB in that frame. This plan makes
that measure native.

A second practical failure mode from the same session: short windows on
staccato content are **alignment-sensitive** (500 ms single windows flip
ordering with ±60 ms offset jitter; the reference and renders start 130–190 ms
apart). The external fix was envelope cross-correlation before comparing.
Fold that in too.

---

## 2. Design

### 2.1 New mode on `compare_spectra` (not a new tool)

Same tool, same sources, same result struct extended. A single new optional
param group activates the framed path:

```
time_resolved:   bool  (default false — existing behaviour unchanged)
hop_ms:          f32   (default 20.0; reuse spectrogram_frame_samples() defaults)
frame_len_ms:    f32   (default 40.0)
mask:            "target_energy" | "none"   (default "target_energy")
align:           "envelope" | "none"        (default "envelope")
align_max_ms:    f32   (default 250.0)
```

`time_resolved: true` computes, per source, a framed magnitude spectrogram via
the existing `analyze_spectrogram()` (`spectrum.rs:356` — one FFT workspace,
`MAX_SPECTROGRAM_FRAMES = 4096` cap already handled by
`spectrogram_frames()`, `mcp_bridge.rs:10630`), then:

1. **Align** (when `align: "envelope"`): 10 ms RMS envelopes of both mono
   buffers, mean-removed cross-correlation over ±`align_max_ms`, shift the
   candidate by the argmax lag. Report the chosen `alignment_offset_ms` in the
   result so the caller can sanity-check it. This runs on the raw windowed
   audio *before* framing, so both spectrograms share a frame grid.
2. **Mask** (when `mask: "target_energy"`): a frame is compared only when the
   *target* frame's broadband RMS is voiced-relevant — above
   `max(NEAR_FLOOR_RMS, 0.05 × max_frame_rms)` (the relative term matches the
   external ground-truth measure; the absolute term reuses the existing
   constant, `spectrum.rs:74`). Masked-out frames are counted, not compared.
3. **Per-frame distance**: existing `log_spectral_distance()` on each aligned
   frame pair (per-frame peak-normalised log bins — this is the step that
   restores visibility of quiet-in-time content), then RMS over compared
   frames. Same for a framed `mel_l2`.

### 2.2 Result extensions (`synth_mcp/src/types.rs` `CompareSpectraResult`)

```
time_resolved_lsd:        Option<f32>   // RMS of per-frame LSD over compared frames
time_resolved_mel_l2:     Option<f32>
frames_compared:          Option<u32>
frames_masked:            Option<u32>
alignment_offset_ms:      Option<f32>
worst_frames:             Option<Vec<{ time_seconds, lsd }>>  // top 5 — WHERE it diverges
```

Aggregate fields keep their meaning and are still computed (the framed mode is
additive, not a replacement). `worst_frames` is the actionable payload: it
points the caller at the timestamps to listen to / re-window.

### 2.3 Honesty guard for the aggregate scalar (small, independent)

The aggregate path stays blind to temporal sparsity, so tell the caller when
they're holding the wrong tool: while downmixing in
`analyze_spectrum_source()`, compute the 10 ms-envelope **active-time
fraction** (frames above `NEAR_FLOOR_RMS` / total). If either source's
fraction is `< 0.6` and `time_resolved` is off, append a warning:
`"time-sparse content (target active 31% of window) — aggregate distances
average over silence; use time_resolved: true"`. No behaviour change, one
new warning string.

---

## 3. Implementation steps

All steps below are DONE (checked). `active_time_fraction` was factored into a
pure fn (step 2) rather than inlined in the aggregate path; envelope alignment
returns a lag in 10 ms windows so it handles sources at different sample rates
natively. Step 6 was a no-op — the MCP schema is generated from schemars at
runtime, there is no mirror file to regenerate.

1. [x] `spectrum.rs`: add `pub fn compare_time_resolved(target_frames,
   candidate_frames, mask) -> TimeResolvedDistance` next to `compare()`
   (~880). Pure function over two `Vec<SpectrogramFrame>`; per-frame
   `log_spectral_distance` / `mel_l2_distance` reuse the existing private fns.
2. [x] `spectrum.rs` or a sibling module: `envelope_align(a: &[f32], b: &[f32],
   sr, max_lag_ms) -> i64` (10 ms RMS envelope + argmax cross-correlation).
   Pure, unit-testable. (Signature took both sample rates and returns the lag in
   10 ms windows so differing-rate sources align natively.)
3. [x] `mcp_bridge.rs` `compare_spectra_impl()` (~10919): when `time_resolved`,
   keep the raw mono buffers from both source analyses (today
   `analyze_spectrum_source()` consumes them — split it so the mono + sample
   rate come back alongside the `SpectrumResult`), align, frame via
   `spectrogram_frames()`, call `compare_time_resolved`, fill the new fields.
   Resample to a common rate first when the sources differ (the sample path
   runs at native rate — reuse whatever `compare` already does for the
   aggregate log-bin alignment, which is rate-independent by construction;
   the framed path needs a shared hop in *seconds*, so frame each source at
   its own rate with the same hop_ms/frame_len_ms and pair frames by index).
4. [x] `synth_mcp`: params on the `compare_spectra` tool (server.rs ~4711), result
   fields (types.rs), tool description gains one sentence: aggregate mode
   averages over time — for staccato/time-varying material set
   `time_resolved: true`.
5. [x] Step 2.3 warning in the aggregate path.
6. [x] Regenerate MCP schema mirrors if applicable. (No-op — schemars generates
   the schema at runtime; no mirror file exists.)

---

## 4. Tests

- `envelope_align`: synthetic click train vs itself delayed 150 ms → returns
  +150 ms within one hop; `align_max_ms` clamp respected.
- `compare_time_resolved` masking: target = tone bursts with silent gaps,
  candidate = same bursts + junk in the gaps → masked mode ignores the junk
  (distance ≈ 0); `mask: "none"` sees it.
- **The ranking regression this whole plan exists for**: target = burst train
  where each burst has a quiet decaying tail; candidate A = correct tails,
  candidate B = tails replaced by held tone, candidate C = tails silenced.
  Assert aggregate LSD does NOT separate A/B/C (documents the blind spot) and
  `time_resolved_lsd(A) < time_resolved_lsd(B) < time_resolved_lsd(C)`.
- Active-time-fraction warning fires on the burst train, not on a sustained
  pad.
- Frame-cap: 3 s at hop 20 ms ≈ 150 frames — far under the 4096 cap; a
  degenerate hop still surfaces the existing truncation warning.

---

## 5. Exit gate

The slice-7 A/B runs end-to-end over MCP with **one `compare_spectra` call per
candidate** (`time_resolved: true`, defaults otherwise) and reproduces the
ground-truth ranking (slice-7 ON < gate OFF < old ON) on the archived WAVs —
no external numpy, no manual offset hunting, no per-window sweeps. The 15-call
windowed workaround from 2026-07-06 (5 × 500 ms × 3 candidates, hand-computed
offsets, hand-averaged) is retired.
