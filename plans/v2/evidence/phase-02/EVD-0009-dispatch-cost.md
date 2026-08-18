# EVD-0009: Node dispatch — what a prepared function table costs, and what the cost is made of

| Field         | Value                                    |
|---------------|------------------------------------------|
| ID            | EVD-0009                                 |
| Status        | Complete                                 |
| Phase         | 02                                       |
| Created       | 2026-08-18                               |
| Last reviewed | 2026-08-18                               |
| Retention     | Permanent                                |
| Related       | ADR-0004, ADR-0005, P02-T005, P02-T009   |
| Superseded by | —                                        |

Permanent because [ADR-0004](../../decisions/ADR-0004-native-node-representation.md) is decided on it.

## Question or hypothesis

[ADR-0004](../../decisions/ADR-0004-native-node-representation.md) proposes **Option C**: a closed kernel registry
dispatched through a prepared function table. Its acceptance rule B, fixed before any code existed, is:

> The dispatch overhead is measured on the minimal voice path at `Q` = 64, against a hand-written direct-call variant
> of the same plan, using the estimator, draw count, build profile, and binary discipline the evidence rules require.
> **Option C is accepted if the overhead is below 3% of the plan's per-quantum cost.**

with a falsifier stated alongside it: *if the direct-call variant is not faster than the table by a measurable margin,
the harness is dominated by something else and the number is an artifact until that is explained.*

The question is stated so that either answer is publishable. It was run as written, and it **failed** — which is the
outcome this record spends most of its length on, because what the failure is made of turns out to decide more than the
figure does.

## Acceptance criteria

- **A — rule B as written.** `table` against `direct`, below 3%.
- **B — the falsifier.** `direct` must be measurably faster than `table`. If it is not, nothing below is interpreted
  until that is explained.
- **C — the aggregation rule.** Each arm's *time* is the **median across nine program runs** of that run's **minimum
  across twenty-five rounds**, with minimum and maximum reported alongside. Each *ratio* between two arms is **paired**:
  the ratio is taken within each round, the median over a run's rounds is that run's figure, and the median over nine
  runs is the reported figure. No run is excluded for any reason.

  The pairing is not a refinement. A ratio of two independently selected minima can take each arm's best round out of a
  *different* round, and this machine is uneven enough for that to matter: over the same data the unpaired figure for
  rule B's comparison ranges from +4.5% to +13.3% in one set and from −2.8% to +12.5% in the other, where the paired one
  holds to about a point and a half and its medians agree across the two sets to two tenths. Both are retained; the paired one is what the
  conclusion uses.
- **D — controls, compared paired.** Each timed shape is qualified by a null control with its own instruction mix,
  adjacent to the arm it bounds — **including the candidate**, so rule B's comparison has a noise floor in the table's
  own mix and not only in the baseline's. Four are needed rather than one, because four instruction mixes cannot share
  a noise floor. The spread is computed **within each round**, between the control and
  its arm as measured in that round, and reported as the median of those per-round spreads. Dividing two independently
  aggregated medians instead — which an earlier revision did — cancels drift rather than measuring it, and reported a
  noise floor an order of magnitude below the real one.
- **F — rotation, in groups.** The order advances by one **group** each round, where a control and the arm it bounds
  are one group, so no arm keeps a position and no rotation can put an arm before its own control. An earlier revision
  claimed rotation in its prose and ran a fixed order; the one after it rotated flat, which put an arm first on five
  rounds in eight.
- **E — every arm must compute the same signal.** Before any timing, each arm is run past the envelope's attack and
  decay from its own state, and **the carries they produce** are compared sample for sample against the hand-written
  arm's. A mismatch, or any silent carry, aborts the run and prints no timings. Each dispatch arm holds its own arena,
  its own node states and its own carry, so no arm's quanta advance another's phase.

### Corrections retained

Criteria C through F are not formalities. **Nine** things this harness got wrong before it got them right, each kept
because the error is more instructive than the final figure:

1. **Prepared records taken by position.** The first version handed three of four kernels a record of the wrong kind.
   A kernel handed the wrong record returns without writing — the right thing on the audio thread — so the direct arm
   became a measurement of four early returns and came out a hundred times faster than the fused one. Records are now
   found by variant.
2. **The plan's output operation left out of every arm.** Rule B's threshold is a share of *the plan's* per-quantum
   cost, and an arm that stops at the last node divides by a denominator the plan does not have. Every arm now performs
   the output write.
3. **The output write implemented differently per arm.** The first fix gave the hand arm a `zip` copy and the table arm
   a strided indexed loop, so the arms differed in two things rather than one. Both now call one function.
4. **The output write added to two arms out of three.** The table arm — the candidate — was left without it, which
   made it 1.68% *cheaper* than the hand-written variant and reversed the conclusion. The agreement check did not catch
   it because it compared the arena buffer the last node writes rather than the carry the plan produces; it now
   compares the carry, and refuses a table arm whose carry is silent.

5. **The hybrid rule C names was argued from a bound instead of measured.** The first redraft reasoned that a hybrid
   costs a per-node mixture of the two pure shapes and therefore cannot beat the all-enum arm. The replication then put
   the all-enum arm *below* the threshold, which left the hybrid free to land on either side of it — and measuring it
   put it above **both** pure shapes, which the mixture argument does not predict. The arm exists because a reviewer
   refused the bound.

6. **The control spread computed from independently aggregated medians.** Dividing one arm's median by its control's
   cancels drift instead of measuring it, and reported a floor of 0.07% where the paired figure is 0.86%. Controls are
   now compared to their arm inside the round both were measured in.
7. **Rotation claimed and then rotated wrongly.** The first version's prose said the arms rotated and the loop ran a
   fixed order; the fix rotated flat over every arm, which put an arm before its own control on most rounds.
   The rotation is now over groups, and a control and its arm are one group.

The first four made the candidate look better than it is. That is not a coincidence: an arm that does less work is the
easiest kind of mistake to leave in, because nothing about it looks wrong. The fifth made a competitor look worse than
it might have been, which is the same failure wearing the other coat. The last two made the whole instrument look
sharper than it is, which is the most dangerous of the three kinds — a noise floor an order of magnitude too low would
have made a 2% difference between shapes look decisive.

8. **The candidate had no control of its own.** Rule B's comparison had a null for the baseline and none for the
   table, so its noise floor was measured in somebody else's instruction mix. The table now has `table_ctl`.
9. **The ratios were taken from independently selected minima.** Each arm's best round could come from a different
   round, which is how the same comparison produced −1.6% and +10.8% between runs of one set. Ratios are now paired
   within a round.

**A tenth, found later, and it belongs to the `renderer` arm alone.**
[EVD-0010](EVD-0010-internal-channel-layout-real-path.md) discovered that `PreparedRenderer::prepare` primes its carry
with `Q` frames of silence, so a **first `render` call of exactly `Q` frames serves that priming and renders no
quantum** — and an event presented on it falls outside the call's span, which fails the whole call. This harness opens
its renderer's gate on exactly that call and ignores the returned error, so **its `renderer` arm rendered an ungated
voice**: verified by running this committed harness, whose gate call returns `EventOutsideCallSpan` and whose renderer
output is silent while the hand arm's peak is 0.456.

What it does and does not change: the arm's *work* per quantum is the same either way — the envelope writes a constant
whether it is idle or sustaining, the oscillator and the filter run regardless, and the amplifier multiplies either
way — so the 877.7 ns figure stands, and no acceptance arm is affected because none of them goes through the
renderer. What is weakened is the phrase *"a realistic per-quantum cost"*: it is a realistic cost of rendering a
**silent** voice. The figure is retained rather than recollected, and the defect is recorded here.

**Every one of the first nine was found by review rather than by the harness's own checks**, which is worth recording as
plainly as the figures: this record's conclusion is the product of six review passes, and the earlier drafts of it
reached the opposite conclusion twice.

## Source and environment

- Source revision: `cf751ae5` (branch `feat/core-v2-phase-2`), the commit holding the exact harness both committed
  data sets were produced by — twelve arms, four control pairs, independent state per dispatch arm, group-rotated arm
  order, and both controls and ratios paired within a round. Earlier revisions of the harness produce different figures and are not the ones measured here.
- Platform: Linux 7.1.8, 13th Gen Intel Core i7-13700H, 6 P-cores + 8 E-cores. Load average ≈ 1.2 throughout, which is
  this machine's permanent background load rather than an idle baseline.
- Pinned with `taskset -c 10` — **one thread of one P-core**, not the pair EVD-0008 used. Pinning to the pair left the
  control spread at up to 9.6%; pinning to one thread brought it to 0.5–2.1%, which is what makes a 3% threshold
  discussable at all. Individual runs still occasionally spread to 9.6% even pinned, which is why the aggregation rule
  is a median over nine of them rather than a single run.
- `cargo build --release`, workspace profile, no criterion. **That profile sets `lto = true`**, so the harness links
  the crate with fat link-time optimization and a kernel can be inlined across the boundary exactly as it could inside
  the crate. The comparison therefore does not disadvantage the closed shapes, which an earlier revision of this record
  wrongly claimed it did.
- `Q` = 64 frames, mono, 48 kHz, `f32`.

## Inputs

The **minimal voice path** the master plan's Phase 2 scope names, built through the crate's own public API and compiled
by its own admission: an envelope, a sine oscillator, a two-pole filter, an amplifier, and the output.

Every arm runs the **same kernels over the same prepared records** — the crate's, not a model of them. A hand-written
variant that prepared its own coefficients would be measuring a different filter.

| Arm | What it is |
|-----|------------|
| `direct_ctl` / `direct` | The hand-written variant: each kernel called by name over buffers the caller holds. The two arms are the same code; the spread between them is this comparison's noise floor |
| `enum_ctl` / `enum` | **ADR-0004's Option A**, over the same schedule, the same arena and the same binding, dispatched by a `match` on a closed enum |
| `hybrid_ctl` / `hybrid` | **The hybrid acceptance rule C names**: a closed enum for the two hottest primitives of this path — the oscillator, which evaluates a transcendental per sample, and the filter, which runs a two-integrator recurrence — and the table for the envelope and the amplifier, which are a multiply and an add |
| `table` | **Option C**: the schedule walked, each step's slots bound, and the kernel called through the pointer admission resolved |
| `bind_only` | The same walk and the same binding, calling nothing. What the arena costs, separately from what dispatch costs |
| `fused` | The path's arithmetic in one function, with no node boundary for the optimizer to keep — and with the envelope reduced to the sustain value the timed quanta hold, rather than its kernel and its segment machine. Not an acceptance arm, not in the agreement check, and **not the same computation**: it is a loose upper bound on what removing node boundaries could win, inflated by the control work it also removes |
| `renderer` | One `render` call for exactly one quantum through the real renderer, so the overhead can be stated as a share of a realistic per-quantum cost. **Its voice is ungated** — see the correction below |

## Method

Groups are rotated once per round; within a group the control runs first; the minimum over rounds is one run's figure;
the median over nine runs is the reported figure. Each control is compared to its arm **within the round both were
measured in**, and the median of those per-round spreads is this comparison's noise floor.

## Commands

```bash
cargo build --release --example evd_0009_dispatch_cost -p synth_engine_v2
taskset -c 10 target/release/examples/evd_0009_dispatch_cost 25 50000
```

Nine runs of that command produced [`EVD-0009-dispatch-cost.csv`](EVD-0009-dispatch-cost.csv); nine more, immediately
after and unchanged, produced [`EVD-0009-dispatch-cost-replication.csv`](EVD-0009-dispatch-cost-replication.csv).

## Results

**The canonical set**, and a full **replication** collected immediately after it under identical conditions. Both are
committed; the replication is here because the corrected harness changed the answer once already, and a single set is
no longer something this record is willing to conclude from.

| Arm | Canonical median | Canonical min–max | Replication median | Replication min–max |
|-----|-----------------:|------------------:|-------------------:|--------------------:|
| `direct_ctl` | 800.3 | 784.8–837.2 | 814.9 | 805.1–862.3 |
| `direct` | 807.0 | 755.9–856.3 | 836.7 | 762.4–858.3 |
| `enum_ctl` | 847.0 | 837.1–900.2 | 891.4 | 830.1–907.3 |
| `enum` | 843.0 | 819.1–918.6 | 854.0 | 807.2–905.4 |
| `hybrid_ctl` | 858.9 | 820.4–940.7 | 900.9 | 849.3–926.8 |
| `hybrid` | 867.7 | 816.0–939.6 | 861.9 | 818.8–923.1 |
| `table_ctl` | 856.5 | 845.5–936.4 | 873.0 | 845.9–926.8 |
| `table` | 844.9 | 817.2–925.8 | 885.9 | 858.1–911.2 |
| `walk_only` | 0.8 | 0.7–0.8 | 0.8 | 0.7–0.8 |
| `bind_only` | 29.4 | 28.3–35.1 | 30.4 | 29.0–34.7 |
| `fused` | 538.9 | 531.6–574.7 | 542.2 | 532.3–572.4 |
| `renderer` | 877.7 | 862.8–940.8 | 885.9 | 866.7–933.4 |

**The ratios, paired within each round** — the figures the conclusion uses, and the only ones it uses. Each is the
median over nine runs of that run's median per-round ratio, with the run-to-run range beside it:

| Quantity | Canonical | range | Replication | range |
|----------|----------:|------:|------------:|------:|
| **`table` over `direct` — rule B's own comparison** | **+7.31%** | 6.51–7.96 | **+7.47%** | 6.79–7.84 |
| **`hybrid` over `direct` — rule C's shape, measured** | **+7.08%** | 6.49–7.93 | **+7.34%** | 6.93–8.27 |
| `enum` over `direct` — Option A, every node closed | +5.39% | 4.62–5.88 | +5.10% | 4.75–5.70 |
| **`table` over `enum` — the dispatch shape with the arena on both sides** | **+2.12%** | 1.52–2.54 | **+2.09%** | 1.54–2.49 |
| `table` over `hybrid` | −0.18% | −0.64–0.26 | −0.04% | −0.52–0.38 |
| Binding, as `bind_only` minus `walk_only` within each round | 32.18 ns | 31.40–36.30 | 32.43 ns | 31.55–35.31 |

**The controls, paired within each round.** Each figure is the median over nine runs of that run's median per-round
spread between a control and the arm it bounds:

| Control | Canonical | Replication |
|---------|----------:|------------:|
| `direct_ctl` against `direct` | 0.98% | 0.82% |
| `enum_ctl` against `enum` | 0.75% | 0.73% |
| `hybrid_ctl` against `hybrid` | 0.73% | 0.83% |
| `table_ctl` against `table` | 0.86% | 0.90% |

Every run's paired summary is committed as
[`EVD-0009-paired-summaries.csv`](EVD-0009-paired-summaries.csv), because the per-arm CSV keeps one minimum per arm per
run and cannot reproduce a paired figure. That file also carries the **unpaired** ratios the harness still emits, from
the per-arm minima: over these two sets rule B's unpaired comparison ranges from +4.54% to +13.33% and from −2.82% to
+12.49%, where the paired one holds to about a point and a half. A record that reported only those would be reporting a measurement whose sign is not
stable, which is why this one does not use them anywhere.

The per-arm medians above are reported for scale — how long a quantum takes, and how the arms sit relative to each
other. **They are not used to derive any figure**: a difference between two independently aggregated medians is the
estimator this record rejects, so where a difference matters it is measured paired and appears in the table above.

The falsifier does not trip: `direct` is faster than `table` by 7.31% and 7.47%, roughly eight times the paired control
medians, and in the same direction in every set collected across every revision of the harness.

## Interpretation

**Rule B fails as written**, at +7.31% and +7.47% against a 3% threshold, so rule C applies: redraft, and measure the
hybrid it names.

**No dispatch shape measured here clears the threshold.** That comes before any comparison between the shapes: the
table is +7.31% and +7.47%, the hybrid +7.08% and +7.34%, and a closed enum for every node +5.39% and +5.10%. Rule B's
3% is not a bar one shape passes and the others fail; it is a bar none of them reaches, because most of the gap to a
hand-written variant is not the dispatch.

**The hybrid is the same measurement as the table.** They differ by −0.18% and −0.04%, well inside paired control
medians of 0.7–1.0%. Rule C's alternative offers nothing to move to, and that is now a measurement rather than the
bound an earlier revision of this record tried to argue from.

**The dispatch shape's own price is two per cent.** The table against a closed enum for every node — both walking the
same schedule and binding the same arena, differing in dispatch and nothing else — is **+2.12% and +2.09%**, with
run-to-run ranges of 1.52–2.54 and 1.54–2.49. Two sets agree to three hundredths of a point. That is the number the
decision is about: what Option C costs over the shape that closes the node set. It is measured with fat LTO on, so the
closed shape gets the inlining it would have in place; nothing about the harness disadvantages it.

**The rest of the gap is not attributed here.** The arena binding measures 32.18 ns and 32.43 ns per quantum, paired
against its own walk-only control which itself costs 0.8 ns, and it is the largest identified item with an obvious
remedy. What this record does **not** do is express it as a share of the table's excess over the hand-written variant:
the two are measured against different baselines, and a subtraction across them would be arithmetic rather than
evidence.

**Removing node boundaries is worth more than choosing between them, and the figure is doubly loose.** `fused` sits
about a third below `table` — a figure computed from the two arms' medians rather than paired, because the harness
emits no paired ratio for it, and therefore indicative only by this record's own standard. It also *skips* the
envelope's kernel and its segment machine, and is not in the agreement check. It is an **upper bound on an
opportunity**, stated as a direction rather than as a number, and sizing it needs both a paired ratio and a fused arm
that runs the envelope.

## Limitations

- **One path, five nodes, one machine.** A larger catalog changes the branch-prediction picture for every shape, and
  nothing here predicts which way.
- **One hybrid split.** "The few hottest primitives" is two of four nodes here, chosen by arithmetic — a transcendental
  and a recurrence against a multiply and an add. A different split would land somewhere else, and this record measured
  the split that gives the hybrid its best case on this path rather than every split.
- **The fused arm is not the same computation**, and its 36–39% gap is therefore an upper bound on what removing node
  boundaries could win rather than a measurement of it. Sizing that opportunity needs a fused arm that runs the
  envelope, and this record does not have one.
- **The dispatch figures are a couple of per cent against control medians of about one.** They reproduce across two
  sets to a few hundredths, which is what makes them usable; they are not precise to a tenth, and no conclusion here
  needs them to be.
- **The hand-written arm mirrors this plan's assignment**, which is what makes it a variant of the same plan rather
  than of a different program — but it mirrors it by hand, and asserts only the buffer count. A future change to the
  arena that kept two buffers while changing which node aliases which would not trip that assertion.
- **The enum arm is not a faithful Option A schedule, and the direction of that error is against the decision this
  record feeds.** It walks the *table's* schedule zipped with a parallel vector of kinds, so each step still carries
  the function pointer it does not use and the loop reads two structures instead of one. A schedule that was natively
  an enum would be narrower and read once. The measured +2.12% and +2.09% is therefore a **lower bound** on what the
  table costs over a faithful Option A — the error understates the price of the shape this record's consumer keeps,
  which is the direction that deserves to be stated rather than assumed away. Re-running against a native enum
  schedule is recorded as follow-up work on ADR-0004.
- **The 36–39% fused figure is not paired**, unlike every other difference here; the harness emits no paired ratio for
  it. It is reported as a direction and is not used to attribute anything.

## Conclusion

**Not supported**, for the hypothesis this record was written to test.

Rule B **fails**: the prepared function table costs +7.31% and +7.47% more per quantum, in two independent nine-run
sets, than a hand-written direct-call variant of the same plan, against a 3% threshold. The falsifier does not trip.

Rule C's redraft is therefore mandatory, and the hybrid rule C proposes was **measured rather than argued about**: at
+7.08% and +7.34% it fails the same threshold and is the same measurement as the table. Within the alternatives rule C
names, there is nothing to move to.

Nor is there outside them: a closed enum for every node costs +5.39% and +5.10%, also above the threshold. **No
dispatch shape this record measured passes rule B.** What the threshold was aimed at is not where the time is.

What the measurement settles, within a stated limit, is the price of the choice: **+2.12% and +2.09%** for the table
against a closed enum over the same arena, with fat LTO on so the closed shape is not denied its inlining. The limit is
that the enum arm walks the table's schedule zipped with a vector of kinds rather than a schedule that is natively an
enum, so **the figure is a lower bound** — a faithful Option A would be at least this much cheaper and possibly more.
Whether two per cent or somewhat more of a voice quantum is worth a node set that stays open at the crate boundary is a
decision rather than a measurement, and it belongs to
[ADR-0004](../../decisions/ADR-0004-native-node-representation.md), which records it.

## Artifacts

| Artifact | Location/digest | Retention or reproduction |
|----------|-----------------|---------------------------|
| Harness | [`crates/synth_engine_v2/examples/evd_0009_dispatch_cost.rs`](../../../../crates/synth_engine_v2/examples/evd_0009_dispatch_cost.rs) at `cf751ae5` | Committed. It lives in the crate rather than beside this record because it calls the crate's own kernels; a standalone copy would be a model of them |
| Raw results | [`EVD-0009-dispatch-cost.csv`](EVD-0009-dispatch-cost.csv) | Committed; nine runs, twelve arms |
| Replication | [`EVD-0009-dispatch-cost-replication.csv`](EVD-0009-dispatch-cost-replication.csv) | Committed; a second nine-run set of the same twelve arms, collected immediately after the first under identical conditions |
| Paired summaries | [`EVD-0009-paired-summaries.csv`](EVD-0009-paired-summaries.csv) | Committed; every run's paired control spreads and paired ratios, for both sets, plus the unpaired ratios for comparison. The per-arm CSVs keep one minimum per arm per run and cannot reproduce a paired figure |
