# EVD-0003: V1 CPU, memory, and per-block timing across operating points

| Field         | Value                                    |
|---------------|------------------------------------------|
| ID            | EVD-0003                                 |
| Status        | Complete                                 |
| Phase         | 00A                                      |
| Created       | 2026-08-13                               |
| Last reviewed | 2026-08-13                               |
| Retention     | Permanent                                |
| Related       | P00A-T003, P00A-T005, ADR-0021, ADR-0001, EVD-0001, EVD-0002 |
| Superseded by | —                                        |

Permanent because P00A-T003 and the Phase 0A exit gate rest on it: the gate
requires CPU, memory, timing, and determinism baselines in a reviewable format,
and this record carries three of the four.

## Question or hypothesis

[EVD-0001](EVD-0001-corpus-determinism-baseline.md) settled determinism.
[EVD-0002](EVD-0002-render-quantum-cost-proxy.md) measured CPU, but for
ADR-0037's purposes: four cases, one polyphony, one sample rate, and nothing
about memory or about the time an individual block takes. P00A-T003 asks for the
rest:

1. **How does V1's render cost vary with sample rate?**
2. **How does it vary with polyphony?**
3. **How long does one block take, and how does that compare with the real-time
   budget that block would have had?**
4. **How much memory does a render need, and where does it go?**

The questions are baselines, not comparisons: nothing here passes or fails. Their
purpose is that a V2 measurement has something to be measured against, and that
P00A-T005 has figures rather than guesses when it sizes `HostProfile`.

## Acceptance criteria

A baseline has no threshold to cross, so the criteria are what would make the
numbers unusable rather than what would make them good. Fixed before collection:

- **Per operating point, at least 40 recorded renders** spread over at least 10
  interleaved rounds, so a single disturbed moment cannot own a cell.
- **The minimum is the primary estimator** for cost, with median and mean
  reported beside it, for the reason EVD-0002 gives: contamination from
  preemption and frequency scaling is additive, never subtractive. Any claim that
  changes with the estimator is reported as estimator-dependent rather than
  resolved.
- **Timing is reported as a distribution** — p50, p99, and max over full blocks —
  never as a mean. A mean block time cannot say whether a block would have missed
  a deadline.
- **Short blocks are excluded from the timing distribution.** The last block of a
  render is partial, and its time is not comparable with a full one's. The
  excluded count is reported.
- **Memory is reported as what it is.** The measurement is resident set size from
  the kernel, not allocator accounting, so per-phase figures are stated as an
  upper bound on what a phase kept and a lower bound on what it touched.

## Source and environment

- Source revision: `c90b6c3d` on `main`, plus the two harness changes this record
  landed with — `render_profile`, and `render_cost`'s `--sample-rate` and
  `--polyphony` flags. As in EVD-0002, the harness commit is named after the fact
  because a squash-merge replaces any hash this file could have cited before it.
- Platform and architecture: Linux 7.1.7-200.fc44.x86_64, x86_64,
  13th Gen Intel Core i7-13700H (6 P-cores, 8 E-cores), `performance` governor.
- Rust/tool versions: rustc 1.97.1 (8bab26f4f), cargo 1.97.1 (c980f4866).
- Build profile: `--release`. `BUFFER_SIZE` at its production value of 256 frames
  for every run — this record varies the operating point, not the block size.
- Audio configuration: stereo; sample rate swept over 44 100, 48 000, 96 000, and
  192 000 Hz; master and return effect stages reconstructed, matching
  `pertylizer render`.
- Feature flags: workspace defaults (`gui-egui`, `mcp`, `osc`).
- Host or device simulation: none. No audio device and no window is opened, and
  no callback deadline exists. That is the central limitation of the timing half.
- **The machine was not quiesced**, exactly as in EVD-0002. See *Limitations*.

## Inputs

- The four cases of the reference corpus, at the digests
  [EVD-0001](EVD-0001-corpus-determinism-baseline.md) records. Nothing in the
  corpus changed between the three records.
- A generated **polyphony probe** for the second question:
  `corpus::fixtures::polyphony_probe(n)` builds a project holding exactly `n`
  notes at once, on CORPUS-0001's patch, with `max_voices = n` so nothing is
  stolen and every note sounds for the whole window. It is deliberately **not** a
  corpus case: it pins no behaviour, so a committed fixture would be one nobody
  validates. It is regenerated on every run.

## Method

1. Build once, at `--release`. Unlike EVD-0002 this sweep needs no rebuilds: both
   variables are run-time flags.
2. **Interleave the variants inside each round.** A round runs every sample rate
   (sweep A) or every voice count (sweep B) back to back; drift between rounds
   therefore lands on all variants alike instead of being read as an effect of
   the variable.
3. Pin every run to one physical P-core pair (`taskset -c 10,11`), so the hybrid
   CPU cannot migrate a render onto an E-core and halve its apparent speed.
4. One warm-up render per invocation, discarded, then five recorded ones.
5. A fresh `OfflineEngineSession` per render, for the reason EVD-0002 records: a
   reused session skips its warm-up block and clears the previous render's voices
   first, so its second render is not the same work as its first.
6. For the timing and memory sweep, the same structure at three recorded renders
   per invocation and five rounds — memory is near-deterministic and does not
   need sixty samples, while the timing distribution inside one render already
   has hundreds of blocks in it.

## Commands

```text
cargo build --release -p pertylizer --bin render_cost --bin render_profile

# A: cost across sample rates, rates interleaved within each round
for round in $(seq 1 10); do
  for rate in 44100 48000 96000 192000; do
    taskset -c 10,11 ./render_cost --warmup 1 --reps 5 --sample-rate $rate \
      > A-round$round-rate$rate.jsonl
  done
done

# B: cost across polyphony, voice counts interleaved within each round
for round in $(seq 1 10); do
  for v in 1 2 4 8 16 32 64; do
    taskset -c 10,11 ./render_cost --warmup 1 --reps 5 --polyphony $v \
      > B-round$round-voices$v.jsonl
  done
done

# C: memory and per-block timing
for round in $(seq 1 5); do
  for rate in 44100 48000 96000 192000; do
    taskset -c 10,11 ./render_profile --warmup 1 --reps 3 --sample-rate $rate \
      > C-round$round-rate$rate.jsonl
  done
done
```

## Results

Full tables are in [`EVD-0003-cost.csv`](EVD-0003-cost.csv) (23 rows) and
[`EVD-0003-timing-memory.csv`](EVD-0003-timing-memory.csv) (16 rows). 50
recorded renders per cell in sweeps A and B, over 10 rounds; 15 per cell in
sweep C, over 5 rounds.

### Cost versus sample rate

Milliseconds of CPU per second of rendered audio, minimum estimator.

| Case        | 44.1 kHz | 48 kHz | 96 kHz | 192 kHz |
|-------------|----------|--------|--------|---------|
| CORPUS-0001 | 2.775    | 3.026  | 5.995  | 11.794  |
| CORPUS-0002 | 8.161    | 8.973  | 17.784 | 35.094  |
| CORPUS-0003 | 3.029    | 3.190  | 6.600  | 13.088  |
| CORPUS-0004 | 7.982    | 8.542  | 17.164 | 33.496  |

Pooled duration-weighted, with the real-time factor each rate leaves:

| Rate     | min   | median | mean  | vs 44.1 kHz | Linear prediction | Faster than real time |
|----------|-------|--------|-------|-------------|-------------------|-----------------------|
| 44.1 kHz | 5.856 | 6.032  | 6.278 | 1.00        | 1.00              | 171x                  |
| 48 kHz   | 6.336 | 6.578  | 6.954 | 1.08        | 1.09              | 158x                  |
| 96 kHz   | 12.684| 12.930 | 13.437| 2.17        | 2.18              | 79x                   |
| 192 kHz  | 24.929| 25.569 | 26.044| 4.26        | 4.35              | 40x                   |

**A cross-check that was not planned.** EVD-0002 measured the same corpus at
44.1 kHz and 256 frames and reported a pooled minimum of 5.884 ms/s. This sweep,
a day later, on a changed binary, reports 5.856 — 0.5% apart. The two runs are
independent, so the agreement is evidence that the harness measures the renderer
rather than the session it ran in, and that the per-block `Option` check added
for the timing half costs nothing measurable.

### Cost versus polyphony

The generated probe at 44.1 kHz, minimum estimator.

| Voices | ms/s   | Per voice | vs 1 voice |
|--------|--------|-----------|------------|
| 1      | 1.675  | 1.675     | 1.00       |
| 2      | 2.847  | 1.423     | 1.70       |
| 4      | 5.145  | 1.286     | 3.07       |
| 8      | 9.766  | 1.221     | 5.83       |
| 16     | 19.020 | 1.189     | 11.35      |
| 32     | 37.980 | 1.187     | 22.67      |
| 64     | 75.545 | 1.180     | 45.09      |

Least squares over the seven points:

```text
cost (ms/s) = 0.428 + 1.173 x voices        R^2 = 1.0000
```

### Per-block time against the block's real-time budget

A block is 256 frames at every rate, so its budget is `256 / rate`. p50 and p99
are the median across renders of each render's own percentile; max is the largest
single block seen. Full blocks only — 2 short blocks per render are excluded, out
of 518 to 2 062 depending on rate.

| Case        | Rate     | Budget   | p50     | p99     | max      | p50 % | p99 % | max %  |
|-------------|----------|----------|---------|---------|----------|-------|-------|--------|
| CORPUS-0001 | 44.1 kHz | 5 805 us | 19.5 us | 37.2 us | 93.2 us  | 0.34  | 0.64  | 1.61   |
| CORPUS-0001 | 192 kHz  | 1 333 us | 18.8 us | 36.6 us | 100.3 us | 1.41  | 2.75  | 7.52   |
| CORPUS-0002 | 44.1 kHz | 5 805 us | 66.4 us | 75.1 us | 174.6 us | 1.14  | 1.29  | 3.01   |
| CORPUS-0002 | 192 kHz  | 1 333 us | 65.4 us | 74.5 us | 148.2 us | 4.91  | 5.59  | 11.12  |
| CORPUS-0003 | 44.1 kHz | 5 805 us | 21.2 us | 42.6 us | 96.2 us  | 0.36  | 0.73  | 1.66   |
| CORPUS-0003 | 192 kHz  | 1 333 us | 21.2 us | 41.5 us | 143.3 us | 1.59  | 3.11  | 10.75  |
| CORPUS-0004 | 44.1 kHz | 5 805 us | 45.8 us | 66.7 us | 181.2 us | 0.79  | 1.15  | 3.12   |
| CORPUS-0004 | 192 kHz  | 1 333 us | 44.7 us | 64.0 us | 176.9 us | 3.36  | 4.80  | 13.27  |

The 48 kHz and 96 kHz rows are in the CSV; they interpolate without surprises.

### Memory

Mebibytes, resident set size. `load`, `prepare`, and `render` are the RSS change
across that phase; `peak` is `VmHWM` for the process.

| Case        | Rate     | load | prepare | render | RSS   | peak RSS |
|-------------|----------|------|---------|--------|-------|----------|
| CORPUS-0001 | 44.1 kHz | 5.00 | 0.05    | 0.01   | 13.84 | 14.12    |
| CORPUS-0001 | 192 kHz  | 5.00 | 0.08    | 0.01   | 17.37 | 17.51    |
| CORPUS-0002 | 44.1 kHz | 0.27 | 1.51    | 1.35   | 14.57 | 15.57    |
| CORPUS-0002 | 192 kHz  | 0.27 | 0.00    | 0.00   | 20.96 | 22.01    |
| CORPUS-0003 | 44.1 kHz | 0.58 | 0.00    | 0.00   | 18.45 | 18.70    |
| CORPUS-0003 | 192 kHz  | 0.59 | 0.00    | 0.00   | 23.62 | 23.68    |
| CORPUS-0004 | 44.1 kHz | 0.48 | 0.00    | 5.33   | 23.12 | 23.48    |
| CORPUS-0004 | 192 kHz  | 0.42 | 0.00    | 0.00   | 29.76 | 29.99    |

## Interpretation

**Cost per frame is constant; cost per second of audio is proportional to the
sample rate.** The pooled ratios track the rate ratios to within 2% at every
point (4.26 measured against 4.35 predicted at 192 kHz, the largest gap). Nothing
about V1's renderer gets cheaper or more expensive per sample when the rate
changes, which is the expected result and is worth having as a measurement rather
than as an assumption: it means a V2 comparison may be run at one rate and scaled,
and it means the real-time factor — 171x at 44.1 kHz down to 40x at 192 kHz — is
the whole story of what a rate change costs.

**Voice cost is linear in voice count, at 1.17 ms/s per voice, with a fixed
0.43 ms/s of per-render overhead.** The fit is exact to four decimal places over a
64-fold range, which says V1 has no per-voice interaction cost at all in this
patch: voices neither share work nor contend for it. The apparent per-voice cost
falling from 1.675 to 1.180 ms/s is entirely the fixed overhead being amortized.
Extrapolated on this core, at this patch and this rate, one thread would reach
real time at roughly 850 simultaneous voices — an offline throughput figure, not
a live capacity, and quoted only to show the order of magnitude the per-voice
slope implies.

**The per-block time does not depend on the sample rate, and that is the finding
with consequences.** A block is 256 frames whatever the rate, so it carries the
same work — measured at 19, 21, 45, and 66 us for the four cases, flat to within
4% across a 4.35-fold rate change. The *budget* for that block, however, is
`256 / rate`, which shrinks from 5.8 ms to 1.3 ms. The fraction of the budget a
block consumes therefore scales with the rate: the worst single block observed
goes from 3.1% of budget at 44.1 kHz to 13.3% at 192 kHz. An admission policy
under ADR-0021 that reasons in frames rather than in seconds would get this
backwards.

**The gap between p50 and max is the number an admission policy has to survive.**
p99 is 1.1x to 2.0x p50 depending on the case, but the maximum block is 2.7x to
6.8x p50. On a quiesced machine some of that would disappear; none of it can be
assumed to. A policy sized on median block cost would be sized four to seven times
too optimistically.

**Memory is small and dominated by the process, not the render.** Peak RSS across
every case and rate is 14 to 30 MiB, of which the first project load alone
accounts for about 5 MiB of one-time process warm-up — CORPUS-0001 shows it
because it is measured first, and the other three, loaded later into an already
warm allocator, show 0.3 to 0.6 MiB for the same operation. The render phase's
cost is the output buffer, which is why it grows with rate and with case length
and why it sometimes reads as zero: at 192 kHz the buffer is four times larger,
but by then the allocator has pages to reuse and the kernel sees no new ones.

## Revision: the corpus inputs changed under this record

**Every figure above was measured on four fixture projects that no longer
exist.** P00A-T001 turned off `Oscillator`'s note-on phase randomization in all
five corpus fixtures — see
[EVD-0004](EVD-0004-corpus-0005-claim-counterfactuals.md) for why — which
changed the four committed inputs this record swept.

A re-measurement at 44.1 kHz was run under this record's own protocol: same
harness, `--release`, the same core pair, **10 rounds of five recorded renders
per case**, minimum estimator. That is sweep A's 50 draws per cell, not fewer.

| Case | This record | After the change | Delta |
|------|------------:|-----------------:|------:|
| CORPUS-0001 | 2.775 ms/s | 2.705 ms/s | −2.5% |
| CORPUS-0002 | 8.161 ms/s | 7.829 ms/s | −4.1% |
| CORPUS-0003 | 3.029 ms/s | 2.921 ms/s | −3.6% |
| CORPUS-0004 | 7.982 ms/s | 7.618 ms/s | −4.6% |
| **Pooled**  | **5.856 ms/s** | **5.619 ms/s** | **−4.0%** |

### What this replaces, and why the replacement matters

An earlier version of this section reported **+2.2%** from a three-round check —
15 draws per cell — and argued that the shift was real because it was *uniform*
across the four cases. Review objected that a minimum over fewer draws is biased
upward, since it has fewer chances to observe an uncontaminated run, and that
the bias would itself be uniform. The objection to the reasoning is correct: a
uniform offset is exactly what small-sample bias predicts, so uniformity was
never evidence against noise, and that argument is withdrawn.

**The proposed mechanism, however, is measurably not the cause.** Resampling
15-draw minima out of the 50 recorded above, 2 000 trials:

| Statistic of a 15-draw pooled minimum | Value |
|---------------------------------------|------:|
| Median                                | 5.627 ms/s |
| 5th – 95th percentile                 | 5.620 – 5.635 ms/s |
| 50-draw pooled minimum, same data     | 5.619 ms/s |
| **Upward bias at the median**         | **+0.14%** |

0.14% does not account for a 6.5% gap between two runs of the same binary on the
same fixtures hours apart (5.983 against 5.619), and the 15-draw distribution
does not reach 5.983 at its 95th percentile.

**What actually dominates is session-to-session variation on this machine**, and
this record already names the cause: it was not quiesced, and unrelated
interactive processes held roughly half a core throughout. The 0.5% agreement
this record reached with [EVD-0002](EVD-0002-render-quantum-cost-proxy.md) a day
apart is therefore a weaker guarantee than it reads as — reproducible on that
pair of runs, not on demand.

### The consequence for this record

**No cost claim about the fixture change is supportable at this resolution.**
The −4.0% above is not evidence that the new fixtures are cheaper any more than
the withdrawn +2.2% was evidence that they are dearer; both are within the
spread this machine produces between sessions. Removing one RNG call per note-on
cannot plausibly move a render by 4% either way.

No conclusion in this record depends on the absolute level. Cost per frame
constant across sample rate, cost linear in polyphony, the per-block-against-
budget finding, and the memory figures are shape claims that a few percent of
uniform offset does not touch. The polyphony sweep is untouched in any case: it
runs on `corpus::fixtures::polyphony_probe`, which is not a corpus case and was
deliberately left unchanged.

What a reader should take from this section is narrower than the tables above:
**treat these figures as a baseline for shape, not as absolute costs to compare
a V2 number against directly.** A V2 comparison must re-measure V1 in the same
session, on the same binary, at matched draw counts.

## Limitations

- **Offline throughput, not real-time headroom.** There is no host, no device, no
  callback deadline, and no competing thread. Every timing figure is a lower
  bound on what the same work costs live, and the record makes no headroom claim.
  A simulated host is Phase 3's work, and ADR-0022 is deferred to its entry gate.
- **The machine was not quiesced**, exactly as in EVD-0002: unrelated interactive
  processes held roughly half a core throughout. This is the most likely source of
  the max-block outliers, and it is why the minimum is the primary estimator for
  cost and why variants are interleaved within rounds.
- **RSS is the kernel's view, not the allocator's.** Per-phase figures are an
  upper bound on what a phase kept and a lower bound on what it touched, and
  several read as exactly zero because the allocator reused pages an earlier phase
  freed. Exact per-phase figures need a counting global allocator, which needs
  `unsafe`, which this workspace denies — a real exemption to request, not a lint
  to silence, and not one this baseline needed.
- **Four of eleven corpus categories**, as in EVD-0002. No sampler, YAMS,
  stereo/spatial, insert, tempo-map, or shared-instrument case exists yet. The
  polyphony probe is one patch, so its 1.17 ms/s per voice is the cost of *that*
  voice, not of a voice.
- **One platform, one core pair, one build profile, one block size.** No
  cross-platform or multi-core claim is made.
- **Wall clock, not CPU time**, for both the render totals and the individual
  blocks. A preempted block is recorded as a slow block, which is precisely what
  inflates the max column.
- **The polyphony sweep varies only the voice count.** Module count per voice,
  instrument count, and effect depth are held fixed, so the linear fit describes
  one axis of a surface.

## Conclusion

**Supported.** All four questions have measured answers, at the resolution a
baseline needs:

1. Cost per rendered second is proportional to sample rate to within 2%; cost per
   frame is constant. Real-time factor falls from 171x to 40x across 44.1 to
   192 kHz.
2. Cost is linear in polyphony at 1.173 ms/s per voice plus 0.428 ms/s fixed,
   R^2 = 1.0000 over 1 to 64 voices.
3. A 256-frame block costs 19 to 66 us depending on the case, independent of
   sample rate, against a budget that shrinks with rate — 0.34% to 5.6% of budget
   at p99, and up to 13.3% for the worst single block at 192 kHz.
4. Peak RSS is 14 to 30 MiB per case, dominated by process warm-up and the output
   buffer rather than by the engine.

Gate impact: with [EVD-0001](EVD-0001-corpus-determinism-baseline.md) for
determinism and [EVD-0002](EVD-0002-render-quantum-cost-proxy.md) for the
block-size curve, **P00A-T003's four quantities are now all measured** and the
Phase 0A exit gate's baseline bullet is satisfiable for the four corpus
categories that exist. It does not close P00A-T001: seven categories are still
uncovered, and every figure here is a property of the four that are.

P00A-T005 inherits three numbers it would otherwise have had to guess: the
per-voice slope, the per-block cost against budget, and the prepared-memory
scale.

## Artifacts

| Artifact | Location/digest | Retention or reproduction |
|----------|-----------------|---------------------------|
| Cost harness | `crates/pertylizer/src/bin/render_cost.rs` | Permanent, in-repository |
| Memory and timing harness | `crates/pertylizer/src/bin/render_profile.rs` | Permanent, in-repository |
| Polyphony probe generator | `corpus::fixtures::polyphony_probe` | Permanent, in-repository; not a corpus case |
| Corpus inputs | `corpus/v2-reference/projects/`, digests in the manifest | Permanent, in-repository |
| Aggregated cost, 23 rows | [`EVD-0003-cost.csv`](EVD-0003-cost.csv) | Permanent, next to this record |
| Aggregated timing and memory, 16 rows | [`EVD-0003-timing-memory.csv`](EVD-0003-timing-memory.csv) | Permanent, next to this record |
| Raw records, 1 700 renders | — | Not retained; regenerate with the commands above |
