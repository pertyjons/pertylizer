# EVD-0017: Publication Cost of the One Arbiter

| Field | Value |
|---|---|
| ID | EVD-0017 |
| Status | `Supported`, with one acceptance criterion qualified |
| Phase | 3 |
| Created | 2026-08-26 |
| Last reviewed | 2026-08-26 |
| Supersedes | — |
| Superseded by | — |
| Source revision | 540ed318 |
| Retention | Until phase exit |
| Related | [ADR-0046](../../decisions/ADR-0046-destination-quantum-admission.md) clauses 1, 2 and 7; [EVD-0015](EVD-0015-quantum-occupancy.md) |
| Artifacts | `crates/synth_engine_v2/examples/evd_0017_publication_cost.rs` |

**This section was written before any measurement was taken.** ADR-0046 leaves its numeric
values to Phase 3 and names this cost as owed: "The one-arbiter design makes publication
serial work on the audio thread. Phase 3 must measure its bounded cost."

## Question and falsifier

**The question.** What does one publication pass cost, per event and per pass, and does that
cost stay a small fraction of the callback budget at the admitted maximum?

**Why it is not obvious.** Clause 2 makes one arbiter the sole normal writer of renderer
input, which serialises work that a per-producer design would have spread. Every event costs
a bounds-checked ledger read, a share comparison, a store write and two high-water
comparisons. At the admitted maximum — every quantum of a callback filled to
`max_events_per_quantum` — that is `max_events_per_quantum * max_quanta_per_callback`
charges before the first sample is rendered.

**The falsifier.** The preferred conclusion is that publication is a small fraction of the
callback budget. It is wrong if, at the admitted maximum for a profile this project would
construct, the measured pass exceeds **10 %** of the callback's real-time budget
(`frames / sample_rate` seconds). Ten percent rather than a smaller number because ADR-0021
already reserves the rest of the budget for rendering, and a publication pass that took more
than a tenth would make the partition's cost a design problem rather than an accounting one.

**Acceptance.**

- `Supported` — the admitted-maximum pass is under 10 % of the callback budget on every
  measured profile, and the per-event cost is linear in the event count (so the figure
  extrapolates rather than hiding a superlinear term).
- `Not supported` — either bound fails.
- `Inconclusive` — the measurement's own spread makes the 10 % comparison unresolvable, or
  the machine's noise floor exceeds the difference being claimed.

**What this cannot establish.** It measures the arbiter as built, which today serves one
producer. Live ingress, authored runtime expansion and the internal arena do not exist, so
this is a floor on the eventual cost rather than the final figure. It also runs on one
machine with no audio callback deadline, so it is a lower bound on the same work live — the
same limitation [EVD-0003](../phase-00a/EVD-0003-cpu-memory-timing-baseline.md) records.

**It does not reselect `max_events_per_quantum`.** That reselection needs a measured
*partition*, and four of the six producer classes have no implementation to measure. The
specification's deferred-list row stays open.

## Inputs and controls

- **Build profile:** `--release`. A publication pass is bounds-checked indexing and integer
  comparison, which the development profile does not represent.
- **Profiles measured:** the harness profile at 64, 256, 1 024 and 4 096 frames, all at
  48 kHz, mono. Their callback budgets span 1.33 ms to 85.3 ms, so a per-pass fixed cost and
  a per-event cost separate.
- **Arms:** an empty pass (open, seal, no charge) and a full pass (every quantum filled to
  the compiled share). The empty arm is the control: it isolates `open`'s ledger clear,
  which is the part that scales with the *prepared* store rather than with the work.
- **Symmetry.** Both arms run the same `open`/`seal` pair over the same prepared arbiter, so
  the difference is charging and nothing else.

**Machine.** This host has hybrid P/E cores and a permanent background load, so an unpinned
run measures the scheduler. Every arm is pinned to the same two performance cores with
`taskset -c 10,11`, and the arms are interleaved rather than run in blocks, so a thermal or
load drift affects both equally instead of whichever ran second.

**Estimator.** Minimum over rounds, not mean. The quantity is a lower bound on the true cost
and every source of noise on this machine adds time; the mean would report the background
load. Spread is reported as the interquartile range across rounds so a reader can see
whether the minimum is stable.

## Method

1. Prepare one arbiter per profile, outside the timed region.
2. Per round, per profile, per arm: run the pass `N` times and record the total.
3. Interleave arms within a round; interleave profiles across rounds.
4. Report the minimum per-pass time over rounds, and the per-event cost as
   `(full − empty) / events`.
5. Compare the full-pass minimum against `frames / 48 000` seconds.

## Reproduction

```text
cargo build --release --example evd_0017_publication_cost -p synth_engine_v2
taskset -c 10,11 target/release/examples/evd_0017_publication_cost 25 2000
```

## Results

25 rounds of 2 000 iterations, release build, `taskset -c 10,11`, 48 kHz mono.

| Block | Quanta | Events | Empty pass | Full pass | Full IQR | Per event | % of budget |
|---|---|---|---|---|---|---|---|
| 64 | 1 | 96 | 0.003 µs | 0.142 µs | 0.003 µs | 1.44 ns | 0.011 % |
| 256 | 4 | 384 | 0.003 µs | 0.571 µs | 0.008 µs | 1.48 ns | 0.011 % |
| 1 024 | 16 | 1 536 | 0.003 µs | 2.897 µs | 0.005 µs | 1.88 ns | 0.014 % |
| 4 096 | 64 | 6 144 | 0.009 µs | 11.662 µs | 0.008 µs | 1.90 ns | 0.014 % |

The interquartile range is between 2 % and 4 % of the minimum on every row, so the minimum
is a stable estimate rather than one lucky sample.

**The empty arm is small, and that is the measurement rather than an elision.** It clears the
whole prepared ledger — 1.8 kB at the largest profile — and about nine nanoseconds for that
is ordinary L1 store bandwidth. A first reading of this record called the figure "below
memset speed" and suspected the clear had been optimised away; that used DRAM bandwidth for
an L1-resident buffer and was wrong. The harness now black-boxes a ledger row rather than the
batch length, which makes the clear observable regardless, and the figure did not move when
that was added.

## Conclusion

**`Supported` on the falsifier, by three orders of magnitude.** The admitted-maximum pass
takes 0.014 % of the callback budget at its worst, against a 10 % threshold. Publication is
not a design problem at this scale; it is an accounting one.

**The linearity criterion is qualified rather than met cleanly.** Per-event cost is not
constant: it rises from 1.44 ns at 96 events to 1.90 ns at 6 144, a 32 % increase. The
algorithm is a linear pass, so this is the working set growing — 6 144 events is about 245 kB
of sealed batch, past this core's L2 — and not a superlinear term. It is recorded as a
qualification because the acceptance rule asked for linearity and the observation is not
quite that, and because a reader extrapolating from the smallest row would understate the
largest by a third. The 700-fold margin against the threshold means the qualification does
not change the verdict.

**What this does not establish, restated after the fact.** It measures the arbiter serving
one producer. Live ingress, authored runtime expansion and the internal arena do not exist,
so this is a floor. It runs with no audio callback deadline, so it is a lower bound on the
same work live. And it does **not** reselect `max_events_per_quantum`: that needs a measured
partition, four of whose six classes have nothing to measure. The host-profile
specification's deferred-list row stays open.
