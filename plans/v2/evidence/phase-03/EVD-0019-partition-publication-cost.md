# EVD-0019: The whole producer partition's publication cost

| Field | Value |
|---|---|
| ID | EVD-0019 |
| Status | `Supported`, for the cost question it asks; it does **not** close Phase 3's exit-gate partition clause |
| Phase | 03 |
| Created | 2026-08-31 |
| Related | ADR-0046, EVD-0017, `SPEC` host profile and render limits |

## 2026-09-01 phase-exit disposition

ADR-0054 and REV-P03 supersede the Phase 3 numeric-selection deadline quoted
below. This record now supports only the statement that publishing a
synthetically full provisional partition fits the measured callback budget. It
does not qualify producer occupancy or final numeric values. The original gate
wording below is retained as the historical question this evidence evaluated.

## Question and falsifier

**The question.** What does one publication pass cost when **every publishable producer class**
is filled to its own share — that is, at the admitted maximum of the whole partition rather
than of one class — and does that cost stay a small fraction of the callback budget?

**What this record does not do, said first because a reader will come here from the exit
gate.** Phase 3's bullet 8 asks that "the measured partition fits the callback budget", and the
master plan requires `max_events_per_quantum` to be reselected "from the measured partition
before enabling the contract, **even if that partition fits within 256**". This record measures
what the partition **costs to publish**, not the producers' occupancy the reselection needs, so
it does not discharge that clause and two attempts to tick the bullet on it were refuted by
independent review. It is recorded because the cost was unmeasured for the whole partition and
is now measured, not because it closes a gate.

**Why EVD-0017 does not answer it.** That record measures the same pass with only
`compiled_event_share` charged, because the compiled producer was the only one that existed,
and it says so itself and calls its figure a floor. Phase 3's exit gate asks whether "the
measured partition fits the callback budget", and one class is not the partition: at the
harness profile the compiled share is a fraction of `max_events_per_quantum`, so the pass
under test was a fraction of the pass the cap admits.

**One correction to that record's context, not its figures.** At its revision `540ed318`,
production code charged `ProducerClass::Compiled` and nothing else, so **five** of the six
classes lacked a producer; EVD-0017's conclusion says four, which was one too many when it
was written. Three have gained one since: session, from transport activation's boundary
release and locate catch-up, and live and guaranteed release, from the ingress slice. The
correction is recorded here rather than by editing that record, whose measurements are
unaffected.

**What makes the measurement possible without the producers.** The `charge` path is
**producer-independent**: it does the same work whichever class it is given — one ledger read,
one share comparison, one store write by index, two high-water comparisons — and takes the
class as a parameter. A harness can therefore charge each class its own share directly and
exercise that path at the partition's admitted maximum.

**It is the charge path, not the whole arbiter interaction**, and an earlier draft overstated
this as "exactly the work a full set of producers would cause". Two real paths differ: a live
or release drain calls `Publication::reaches` before each charge, and the session boundary
release uses `charge_operation`, which deliberately writes no batch entry. So this is a
synthetic charge-path workload at full occupancy — an upper bound on the store-writing part and
a lower bound on the whole, which is stated rather than implied because the figure is used to
argue smallness.

**The falsifier.** The preferred conclusion is that a full-partition pass is a small fraction
of the callback budget. It is wrong if, **on any of the four measured profiles**, the pass
exceeds **10 %** of that callback's real-time budget (`frames / sample_rate` seconds). The
scope is the measured profiles rather than every constructible one, because a profile's shares
and cap are caller-supplied and the space is unbounded; a universal claim would need a bound on
per-event cost rather than four rows, and this record does not have one. An earlier draft wrote
"any profile this project would construct" and then accepted on the measured four, which an
independent review caught as a silent weakening. Ten percent for EVD-0017's reason: ADR-0021
reserves the rest of the budget for rendering, and a publication pass taking more than a tenth
would make the partition's cost a design problem rather than an accounting one.

**Acceptance.**

- `Supported` — the full-partition pass is under 10 % of the callback budget on each of the
  four measured profiles, which is exactly the falsifier's scope.
- `Not supported` — it exceeds that on any of them.
- `Inconclusive` — the machine's noise floor exceeds the difference being claimed.

**What this cannot establish, stated before the figures rather than after.**

- **It measures the arbiter, not the producers.** Draining an ingress queue, materializing an
  authored batch and building a session snapshot are each work this harness does not do —
  for the live, release and session classes because the harness charges past their producers,
  and for authored runtime and renderer-internal because **those two** have no producer at
  all. An earlier draft said four, inherited from EVD-0017, where the figure was already one
  too many; the correction is above. So this is the cost of *publishing* a full partition,
  not of *producing* one, and the exit-gate clause it serves is the one about the partition
  fitting the budget rather than about any producer's throughput.
- **`Internal` is not charged**, and its absence is structural rather than an omission:
  ADR-0046 clause 2 keeps renderer-internal emissions on the far side of the seal in their own
  arena, and `Publication::charge` refuses the class by name. The external batch's admitted
  maximum is therefore the five publishable shares, and the internal share plus the partition's
  unusable slack are capacity this pass cannot spend.
- **It does not reselect `max_events_per_quantum`.** That is a different question — what the
  cap *should* be — and it needs the producers' own occupancy, which authored runtime and
  renderer-internal cannot yet supply at all. The host-profile specification's deferred row
  stays open, and this record does not touch it.
- One machine, no callback deadline: a lower bound on the same work live, as
  [EVD-0003](../phase-00a/EVD-0003-cpu-memory-timing-baseline.md) records for every timing
  figure here.

## Inputs and controls

- **Build profile:** `--release`. A publication pass is bounds-checked indexing and integer
  comparison, which the development profile does not represent.
- **Profiles measured:** the harness profile at 64, 256, 1 024 and 4 096 frames, 48 kHz, mono,
  as EVD-0017 used, so the two records' rows are comparable directly.
- **Arms:** the empty pass and the compiled-only pass EVD-0017 already defines, plus a third
  that charges every publishable class its own share in every quantum of the window. Keeping
  the first two is what makes the third readable: the difference between the second and the
  third is the cost of the classes EVD-0017 left out, and the first still isolates `open`'s
  ledger clear.
- **Symmetry.** All three arms run the same `open`/`seal` pair over the same prepared arbiter.

**Machine.** Hybrid P/E cores with a permanent background load, so every arm is pinned to the
same two performance cores with `taskset -c 10,11` and the arms are interleaved within each
round rather than run in blocks.

**Estimator.** Minimum over rounds, with the interquartile range across rounds reported beside
it. The quantity is a lower bound and every source of noise on this machine adds time, so the
mean would report the background load.

## Method

Fixed before any figure was taken, which is what `PROCESS.md` requires of a decision-driving
measurement. The arm is `partition_pass` in the same harness EVD-0017 uses, so the three arms
share one `open`/`seal` pair, one prepared arbiter and one interleaved round structure.

## Source revision

`5d829e7e` plus the uncommitted harness arm this record adds. **The method was written before
the figures were taken and the record says so, but that ordering is not independently
auditable**: the record and its harness arm reach version control in one commit, so nothing in
the history separates them. An independent review made the point, and it is recorded rather
than argued away — a reader who needs the ordering guaranteed should treat this record's
framing as unverified and read the falsifier on its own merits.

## Reproduction

```
taskset -c 10,11 cargo run --release -p synth_engine_v2 --example evd_0017_publication_cost
```

15 rounds of 2 000 iterations. The `part` columns are this record's; the others are
EVD-0017's and are unchanged.

## Results

| Block | Quanta | Partition events | Pass | IQR | Per event | % of budget |
|---|---|---|---|---|---|---|
| 64 | 1 | 240 | 0.451 us | 0.048 us | 1.86 ns | 0.034 % |
| 256 | 4 | 960 | 2.535 us | 0.149 us | 2.64 ns | 0.048 % |
| 1 024 | 16 | 3 840 | 10.389 us | 0.212 us | 2.70 ns | 0.049 % |
| 4 096 | 64 | 15 360 | 44.596 us | 0.593 us | 2.90 ns | 0.052 % |

The five publishable shares sum to **240** of the profile's 256-event cap; the remaining 16
are the internal share and the partition's unusable slack, neither of which this pass can
spend. The partition therefore charges 2.5x what EVD-0017's compiled-only arm does, which is
the gap between "one class" and "the partition" that this record exists to close.

## Conclusion

**`Supported`, with about 190x of margin — two orders of magnitude, not three.** The worst
full-partition pass is 0.052 % of its callback's budget against a 10 % falsifier. Filling
every publishable share in every quantum a callback can render costs about a twentieth of a
percent of the time available, so the
partition's publication cost is an accounting quantity rather than a design constraint — the
same verdict EVD-0017 reached for one class, now established for the whole of what the cap
admits.

**Per-event cost is 1.86 to 2.90 ns and rises with the batch**, as EVD-0017's does and for the
same reason: 15 360 events is about 610 kB of sealed batch, well past this core's L2, so the
working set grows rather than the algorithm changing. It is recorded here too because a reader
extrapolating the smallest row would understate the largest by half.

**The spread is wider than EVD-0017's** — an interquartile range of 1.3 % to 10.6 % of the
minimum, against 2 % to 4 % there — and the smallest profile is the worst of them. A
0.451 microsecond pass is short enough that the round's own overhead is a visible fraction of
it, which is a property of measuring something this cheap rather than of the work. The margin
against the threshold is about 190x — two orders of magnitude — so the spread does not reach the
verdict.

**It does not reselect `max_events_per_quantum`**, and the host-profile specification's
deferred row stays open. That question asks what the cap *should* be, which needs the
producers' own occupancy; this one asks what the cap as it stands costs to publish. Answering
the second does not answer the first, and an earlier reading of Phase 3's exit gate that
treated them as one clause was refuted by independent review.
