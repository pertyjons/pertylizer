# EVD-0002: Render quantum cost proxy — V1 cost versus block size

| Field         | Value                              |
|---------------|------------------------------------|
| ID            | EVD-0002                           |
| Status        | Complete                           |
| Phase         | 00A                                |
| Created       | 2026-08-12                         |
| Last reviewed | 2026-08-12                         |
| Retention     | Permanent                          |
| Related       | ADR-0037, ADR-0001, P00A-T003, P00A-T006, EVD-0001 |
| Superseded by | —                                  |

Permanent because [ADR-0037](../../decisions/ADR-0037-render-quantum-value.md)
is accepted on the strength of it.

## Question or hypothesis

ADR-0037 must choose a frame count for the internal render quantum `Q`. The
quantity that separates the candidates is per-quantum overhead in the V2
compiled node model, and no V2 renderer exists, so the record fixed a V1 proxy
instead: **how does V1's offline render cost vary with the block size it
processes in?**

The proxy answers a narrower question than the decision asks. It measures the
*shape* of the cost-versus-block-size curve for the per-block work both engines
share. It cannot produce a figure transferable to V2 in absolute terms, and
ADR-0037 withdrew the earlier claim that it errs in a safe direction: V1 pays
per-block work V2 compiles away, and V2 pays carry copies and scheduler work V1
has never paid.

## Acceptance criteria

Fixed by ADR-0037 **before** any data was collected, as an ordered, exhaustive
rule table over `c(Q)`, the cost per rendered second at block size `Q`, and
`r(a, b) = c(a)/c(b) - 1`:

| # | Condition                                                    | Outcome           |
|---|--------------------------------------------------------------|-------------------|
| 1 | any comparison used below falls within 5 pp of its threshold  | Provisional 64    |
| 2 | `r(64,256) > 15%` and `r(128,256) <= 15%`                     | Select 128        |
| 3 | `r(64,256) > 15%` and `r(128,256) > 15%`                      | Escalate          |
| 4 | `r(32,64) <= 2%`                                              | Select 32         |
| 5 | otherwise                                                     | Confirm 64        |

Evaluated in order, stopping at the first that applies.

Two definitions the rule table leaves to the measurement, both fixed here before
the data was read:

- **`c(Q)` is pooled across the corpus, duration-weighted.** Each case's cost per
  rendered second is weighted by the length of audio it renders, so a long case
  does not count twice for being long and a short one is not averaged away.
- **The primary estimator is the minimum** over repeated renders, with the median
  and the mean reported as robustness checks. A render's true cost is contaminated
  additively by preemption and frequency scaling, never subtractively, so the
  minimum is the least contaminated estimate available. Any conclusion that
  depends on which estimator is chosen is by construction a rule 1 result.

## Source and environment

- Source revision: `d4b5001c` on `main`, plus the `render_cost` harness, which
  landed with this record as `c6fb861c`, plus one edited line per build (see
  *Method*). No other source difference exists between the four builds. The
  harness commit is named after the fact for the reason
  [EVD-0001](EVD-0001-corpus-determinism-baseline.md) records: a squash-merge
  replaces the hash any pre-merge revision of this file could have cited.
- Platform and architecture: Linux 7.1.7-200.fc44.x86_64, x86_64,
  13th Gen Intel Core i7-13700H (6 P-cores, 8 E-cores), `performance` governor
- Rust/tool versions: rustc 1.97.1 (8bab26f4f), cargo 1.97.1 (c980f4866)
- Build profile: `--release`
- Audio configuration: 44 100 Hz, stereo, per each case's `render` block; master
  and return effect stages reconstructed, matching `pertylizer render`
- Feature flags: workspace defaults (`gui-egui`, `mcp`, `osc`)
- Host or device simulation: none. No audio device and no window is opened.
- **The machine was not quiesced.** Two unrelated interactive processes held
  roughly half a core throughout. See *Limitations*.

## Inputs

The four cases of the reference corpus, at the digests
[EVD-0001](EVD-0001-corpus-determinism-baseline.md) records. Nothing in the
corpus changed between the two records.

| Case        | Category                  | Rendered seconds |
|-------------|---------------------------|------------------|
| CORPUS-0001 | subtractive-voice         | 3 (2 + 1 tail)   |
| CORPUS-0002 | polyphonic-voice-stealing | 4 (2 + 2 tail)   |
| CORPUS-0003 | mod-matrix-patch          | 3 (2 + 1 tail)   |
| CORPUS-0004 | sends-returns-master      | 4 (2 + 2 tail)   |

## Method

`BUFFER_SIZE` in `crates/pertylizer/src/audio/arrangement_render.rs` is a
compile-time constant, so a block-size sweep is four builds rather than four
runs. All four were built first and kept side by side; only then was anything
measured.

1. For each `Q` in 32, 64, 128, 256: set the constant, build `--release`, and
   copy the binary aside. The constant was restored to its production value of
   256 afterwards — **this measurement changes no V1 behaviour.**
2. Each binary prints the constant it was *built* with on every record, so a
   mislabelled binary cannot silently become a datapoint. This was checked before
   the sweep: all four self-identified correctly.
3. Run 36 rounds. Each round runs all four binaries back to back, each rendering
   every case 2 discarded warm-up times and 15 recorded times. **The builds are
   interleaved within a round rather than measured one build at a time**, so
   machine drift lands on all four block sizes alike instead of being read as a
   block-size effect.
4. Every run is pinned to one physical P-core (`taskset -c 10,11`) so the hybrid
   CPU cannot migrate a render onto an E-core and halve its apparent speed.
5. The timed region is exactly one `render_range_with_tail` call. Project
   loading, engine construction, instrument reconstruction, WAV encoding, and
   digesting are all outside it — they cost the same at every block size, so
   including them would add a constant to all four and compress the ratios the
   rule table reads.
6. A fresh `OfflineEngineSession` per render. A reused session skips its warm-up
   block and clears the previous render's voices first, so its second render is
   not the same work as its first.

That is 36 × 4 × 4 × 17 = 9 792 renders, of which 8 640 are recorded (540 per
case per block size).

Two controls fell out of the data rather than being asserted:

- **The renders are deterministic within a build.** All 540 renders of a given
  (case, block size) reported the same peak and the same RMS to every digit.
- **The renders differ across builds.** Peak and RMS move with block size, which
  is the expected consequence of V1 dispatching sequencer events on block
  boundaries, and confirms the four builds really did render different audio
  rather than four copies of one binary.

## Commands

```text
# one build per block size, the constant edited between them
cargo build --release -p pertylizer --bin render_cost

# 36 interleaved rounds
for round in $(seq 1 36); do
  for q in 32 64 128 256; do
    taskset -c 10,11 ./render_cost-q$q --warmup 2 --reps 15 > round-$round-q$q.jsonl
  done
done
```

`render_cost` emits one JSON object per render: block size, case, repetition,
whether it was a warm-up, elapsed nanoseconds, rendered seconds, cost per
rendered second, frames, peak, and RMS.

## Results

### Cost per rendered second, per case

Milliseconds of CPU per second of rendered audio. 540 renders per cell. The full
table, including the mean and the dispersion, is in
[`EVD-0002-render-cost.csv`](EVD-0002-render-cost.csv).

| Case        | Q=32   | Q=64   | Q=128  | Q=256  | r(64,256) |
|-------------|--------|--------|--------|--------|-----------|
| CORPUS-0001 | 3.481  | 3.072  | 3.015  | 2.719  | +12.98%   |
| CORPUS-0002 | 10.473 | 9.386  | 8.737  | 7.998  | +17.35%   |
| CORPUS-0003 | 3.826  | 3.379  | 3.266  | 3.123  | +8.22%    |
| CORPUS-0004 | 8.345  | 8.414  | 8.288  | 8.215  | +2.42%    |

Minimum estimator. Every case renders far faster than real time; the slowest is
about 120× faster.

### Pooled cost and the rule table's ratios

| Estimator | Q=32  | Q=64  | Q=128 | Q=256 | r(64,256) | r(128,256) | r(32,64) |
|-----------|-------|-------|-------|-------|-----------|------------|----------|
| minimum   | 6.942 | 6.468 | 6.210 | 5.884 | +9.93%    | +5.54%     | +7.34%   |
| median    | 7.486 | 6.803 | 6.428 | 6.198 | +9.78%    | +3.72%     | +10.04%  |
| mean      | 7.666 | 6.978 | 6.618 | 6.322 | +10.38%   | +4.69%     | +9.86%   |

### Uncertainty

A bootstrap over the 36 rounds — resampling whole rounds with replacement, 20 000
iterations, seed 20260812. The round is the resampling unit because renders
inside one round share whatever the machine was doing at that moment, so treating
them as independent would understate exactly the uncertainty that matters.

| Estimator | Ratio      | Point   | 95% interval    | Distance to threshold | Draws within 5 pp of it |
|-----------|------------|---------|-----------------|-----------------------|--------------------------|
| minimum   | r(64,256)  | +9.93%  | +7.20 … +11.64% | 5.07 pp below 15%     | **41.6%**                |
| minimum   | r(128,256) | +5.54%  | +2.86 … +6.14%  | 9.46 pp below 15%     | 0.0%                     |
| minimum   | r(32,64)   | +7.34%  | +6.02 … +11.32% | 5.34 pp above 2%      | 15.3%                    |
| median    | r(64,256)  | +9.78%  | +8.83 … +10.68% | 5.22 pp below 15%     | **29.8%**                |
| median    | r(128,256) | +3.72%  | +3.07 … +4.28%  | 11.28 pp below 15%    | 0.0%                     |
| median    | r(32,64)   | +10.04% | +9.35 … +10.60% | 8.04 pp above 2%      | 0.0%                     |

### A cost model, not just four points

The four points are close to linear in blocks per second (`sample_rate / Q`),
which is what a fixed per-block overhead paid on top of per-frame work predicts.
Fitting `cost per second = a + b × blocks per second`:

| Case        | Per-frame `a` (ms/s) | Per-block `b` (µs) | R²    |
|-------------|----------------------|--------------------|-------|
| CORPUS-0001 | 2.705                | 0.567              | 0.930 |
| CORPUS-0002 | 7.900                | 1.933              | 0.965 |
| CORPUS-0003 | 3.033                | 0.566              | 0.987 |
| CORPUS-0004 | 8.253                | 0.096              | 0.365 |
| Pooled      | 5.845                | 0.823              | 0.964 |

## Interpretation

**Rule 1 applies, and the outcome is a provisional `Q` = 64** with the binding
Phase 2 obligation ADR-0037 attaches to it.

The reasoning, and the part of it that is a judgement:

- Rules 2, 3, and 4 do not fire on any estimator. `r(64,256)` is 9.7–10.4%,
  nowhere near 15%, and `r(32,64)` is 7.3–10.0%, nowhere near 2%. Halving the
  block size from 64 to 32 costs real money and buys only control resolution.
- Rule 1 is the one in question, and it is genuinely close. Read literally
  against a single point estimate, the minimum and the median put `r(64,256)`
  5.07 and 5.22 pp from the 15% threshold — just *outside* the 5 pp band — while
  the mean puts it 4.62 pp away, *inside*. Which side of the rule the measurement
  falls on is therefore decided by the choice of estimator, at a margin of a
  tenth of a percentage point, while the bootstrap interval is 2–4 pp wide and
  puts 30–42% of resampled measurements inside the band.
- ADR-0037's own words are that a result within 5 pp of a threshold "is
  inconclusive by definition", and the margin exists because the proxy's
  direction of error is unknown. A measurement whose classification flips with
  the estimator is the case that language describes, so rule 1 fires.

**What follows if a reviewer disagrees.** The alternative reading — take the
minimum as primary, note 5.07 > 5.00, and stop — reaches rule 5, "confirm 64".
Both readings select 64. The disagreement changes only whether the Phase 2
re-measurement is a binding exit-gate item, and nothing in this record argues
for making that obligation weaker.

**The corpus composition matters more than the pooled number admits.** Per case,
`r(64,256)` ranges from +2.42% to +17.35%, and CORPUS-0002 alone is above the 15%
threshold that rule 2 uses. The cost model says why: per-block overhead scales
with how many voices and modules are active, because it is paid per node per
block. CORPUS-0002 runs eight overlapping notes against a four-voice instrument
and pays 1.93 µs per block; CORPUS-0004 is dominated by effect processing that
does not care how the audio is partitioned, pays 0.096 µs, and shows no usable
block-size dependence at all (R² 0.37). A corpus weighted toward voice-heavy
material would push the pooled ratio up, and a corpus weighted toward
effect-heavy material would push it down. With four of eleven categories
covered, the pooled figure is a property of this corpus as much as of the
renderer — which is a second, independent reason not to treat this measurement
as conclusive.

**The transferable finding is the slope, and it is mild.** Per-block overhead is
about 0.8 µs pooled on this machine. At 44.1 kHz that is 8.8% of total render
cost at Q=64 and 16% at Q=32 — a real cost, but nothing like the cliff that would
have forced 128. Nothing here suggests any candidate is unaffordable; the curve
separates them by tens of percent of a cost that is already 100× faster than real
time offline.

**This is not a real-time headroom measurement.** It measures offline throughput
on one core with no callback deadline, no host, and no other instrument
competing. It says nothing about worst-case per-callback latency, which is what
an admission policy under ADR-0021 would need.

## Limitations

- **The proxy measures V1, not V2.** The quantity ADR-0037 actually cares about
  is per-quantum overhead in the compiled node model, which does not exist yet.
  V2 removes per-block work V1 pays and adds carry copies and scheduler work V1
  never pays; the net direction is unknown. This is the reason rule 1 exists and
  the reason the Phase 2 re-measurement is binding.
- **Four of eleven corpus categories.** No sampler, YAMS, stereo/spatial, insert,
  tempo-map, or shared-instrument case exists yet, and the interpretation above
  shows the pooled ratio is sensitive to exactly that mix.
- **The machine was not quiesced.** Two unrelated interactive processes held
  roughly half a core for the whole sweep. This is the main source of the 2–17%
  per-cell dispersion, and it is why the minimum is the primary estimator and why
  the builds are interleaved. It biases the *absolute* costs upward; it should
  not bias the *ratios*, since every block size is measured under the same
  conditions in every round — but "should not" is an argument, not a control, and
  a quiet machine would have produced a narrower interval and possibly a decisive
  answer.
- **Wall clock, not CPU time.** A preempted render is recorded as a slow render.
  The minimum estimator and 540 repetitions per cell mitigate this; they do not
  eliminate it.
- **One platform, one build profile, one sample rate, one core.** No
  cross-platform, cross-rate, or multi-core claim is made. The absolute figures
  are specific to this CPU.
- **Only four block sizes.** The linear cost model is fitted to four points and
  is a description of them, not an independently validated model.

## Conclusion

**Supported** for the question it can answer: V1's render cost grows with block
count in a mild, close-to-linear way, at about 0.8 µs per block on this machine,
and no candidate quantum is disqualified by cost.

**Inconclusive** at the resolution ADR-0037's rule table demands: `r(64,256)`
lands close enough to the 15% threshold that the classification depends on the
estimator, and the per-case spread straddles the threshold outright.

Gate impact: this is the measurement ADR-0037's acceptance required. Applying the
rule table gives rule 1 — **accept ADR-0037 with `Q` = 64 provisional**, and the
Phase 2 re-measurement against real V2 nodes becomes a Phase 2 exit-gate item
rather than a suggestion. Until then, nothing may treat 64 as settled: no
hand-unrolled kernel, no `Q`-specific buffer layout, no test asserting a control
rate in Hz.

It does **not** close P00A-T003. These are CPU-throughput figures for four cases
at one polyphony and one sample rate; that task needs memory and timing too, and
at common polyphony and sample rates.

## Artifacts

| Artifact | Location/digest | Retention or reproduction |
|----------|-----------------|---------------------------|
| Measurement harness | `crates/pertylizer/src/bin/render_cost.rs` | Permanent, in-repository |
| Corpus inputs | `corpus/v2-reference/projects/`, digests in the manifest | Permanent, in-repository |
| Aggregated results, 16 rows | [`EVD-0002-render-cost.csv`](EVD-0002-render-cost.csv) | Permanent, next to this record |
| Raw records, 9 792 renders (~2 MB JSONL) | — | Not retained; regenerate with the commands above. Every figure this record draws on is either in the CSV or derived from it by the arithmetic stated in *Acceptance criteria* |
| Four block-size builds | — | Not retained; one edited constant each, rebuilt in about 90 seconds |
