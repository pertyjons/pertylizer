# MOS 6581 pulse+triangle calibration

> **Status:** proposed.
>
> **Planning baseline:** Pertylizer `3cfb9581`, 2026-08-27. The checkout also
> contains unrelated in-progress Core V2 transport changes; this plan does not
> depend on or modify them.

## Objective

Replace the self-referential MOS 6581 pulse+triangle acceptance test with a
measured calibration that preserves the large waveform-shape improvement of
the native SID oscillator while correcting its remaining high-frequency loss.

The target is the `sid_oscillator`'s combined-waveform bus model, not an EQ or
gain workaround in sid-analyzer.

## New evidence

For Yie Ar Kung Fu II, subtune 1, voice 1, a ten-second A/B changed from two
summed oscillators to one native pulse+triangle mask:

| Metric | Old graph | Native combined bus |
|---|---:|---:|
| RMS error | 0.06755 | 0.00573 |
| Pitch error | 5.383 Hz | 0 Hz |
| Log-spectral distance | 10.278 | 2.254 |
| Spectral-centroid error | 698.7 Hz | 689.8 Hz |
| Rolloff error | 2142.6 Hz | 2121.0 Hz |

The amplitude, pitch, and broad spectral shape are substantially better, but
the centroid and rolloff show that the output remains too dark.

`SidOscillator::combine_bus` currently assigns pulse+triangle to
`neighbour_support`. The test `pt_6581_keeps_neighbour_support_model` compares
that function only with an oracle composed from the same implementation; it
guards code stability, not chip fidelity. Its comment that pulse+triangle was
at the A/B floor is falsified by the named render above and must be revised when
the calibration lands.

## Evidence contract

Build a deterministic reference matrix before changing the model:

- MOS 6581 reference only; keep the current MOS 8580 plain-AND behavior as a
  separate control.
- At least three musical frequencies spanning bass, mid, and lead registers.
- Pulse widths on both sides of 50%, including narrow, quarter, half, and wide
  duty cycles.
- One exact accumulator-cycle capture where practical, plus named musical
  windows from at least two SID files that actually hold pulse+triangle.
- Fixed SID model, clock, sample rate, DC-coupling policy, voice isolation,
  source frame/tick range, and renderer revision in every receipt.

Use a legally redistributable measured capture or a reproducible external SID
oracle. Do not copy combined-waveform tables from an emulator with an
incompatible license. Store derived normalized features and digests when raw
audio cannot be checked in.

The matrix records at least RMS, DC before/after the blocker, fundamental and
harmonic magnitudes, spectral centroid, rolloff, and log-spectral distance. A
gain-only match is insufficient: it can improve RMS while leaving the waveform
shape wrong.

## Model work

Evaluate small, reviewable model families against the complete matrix:

1. asymmetric neighbour support above/below each bus bit;
2. support strength dependent on pulse width or accumulator region;
3. a compact per-bit pulldown weight model before the existing 6581 DAC;
4. a generated lookup only if a parametric fit cannot meet the bounds and its
   provenance/licensing are explicit.

Fit parameters outside the audio thread and check in only the minimal
deterministic constants or generated artifact. The processing path must remain
allocation-free and keep the current 4× oversampling/decimation route for
combined waveforms.

Do not change the pure triangle, pure pulse, saw+triangle, pulse+saw,
all-three, noise-corruption, ring-mod, sync, DAC, or DC-blocker behavior to make
one musical window pass. Those are explicit negative controls.

## Verification

Add two layers of tests:

### Digital/model tests

- deterministic bus output for every 12-bit accumulator code at each fitted
  pulse-width fixture;
- fitted feature bounds against the measured matrix, not against
  `neighbour_support` itself;
- MOS 8580 output unchanged;
- all existing combined-waveform and hardware-interaction tests unchanged
  unless new external evidence directly invalidates one.

### Render tests

- repeat the Yie Ar Kung Fu II window with a pinned sid-analyzer exporter and
  Pertylizer renderer receipt;
- add a second named pulse+triangle tune/window to prevent one-song fitting;
- compare a nearby pure-pulse or pure-triangle window as a no-regression
  control;
- report every metric, including those that did not improve.

Set numeric acceptance bounds from the frozen reference matrix before fitting.
At minimum, the candidate must not regress RMS error, pitch error, or
log-spectral distance from the current native graph, and must materially reduce
both centroid and rolloff error on the two named windows. “Materially” receives
an exact threshold in the evidence fixture before implementation begins.

## Implementation phases

1. Freeze receipts, source windows, reference features, and falsifiable metric
   bounds.
2. Add an offline fitting/evaluation harness that cannot run on the audio
   thread and emits deterministic candidate reports.
3. Select the smallest model meeting every bound and document its independent
   derivation.
4. Replace the self-oracle test with measured feature fixtures and retain the
   old implementation as an explicit comparison in the evaluation report only.
5. Run the full Pertylizer feature gate and independent uncommitted review.

## Exit gate

- Two named SID windows and the frequency/pulse-width matrix meet their frozen
  amplitude and spectral bounds.
- Centroid and rolloff improve without regressing RMS, pitch, LSD, pure
  waveforms, other combined masks, or MOS 8580.
- The checked-in evidence is deterministic and its source/licensing are named.
- The audio callback remains allocation-free and no new table is built or
  resized in `process()`.
- The misleading “A/B floor” claim and self-oracle acceptance test are removed
  or rewritten to state exactly what they prove.
