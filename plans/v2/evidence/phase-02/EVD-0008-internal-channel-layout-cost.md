# EVD-0008: Internal channel layout — strided access, boundary cost, and end-to-end totals

| Field         | Value                         |
|---------------|-------------------------------|
| ID            | EVD-0008                      |
| Status        | Complete                      |
| Phase         | 02                            |
| Created       | 2026-08-17                    |
| Last reviewed | 2026-08-17                    |
| Retention     | Permanent                     |
| Related       | ADR-0002, ADR-0004, ADR-0005, P02-T001, P02-T002 |
| Superseded by | —                             |

Permanent because [ADR-0002](../../decisions/ADR-0002-internal-channel-layout.md) is decided on it.

## Question or hypothesis

[ADR-0002](../../decisions/ADR-0002-internal-channel-layout.md) must choose between a **planar** internal arena — a
buffer is one channel — and an **interleaved** one, where a buffer holds `n` channels and a kernel strides through it.
The register's required basis for that record is "design review and measurement", and the master plan asks for the
choice to be verified "against module and conversion cost in Phase 2".

Three measurable quantities separate the options:

1. **Does strided per-channel access cost more than contiguous access** over one render quantum?
2. **What does the planar arena's boundary transpose cost**, as a share of a minimal voice chain's per-quantum work?
3. **Which arrangement costs less end to end** — each chain plus the boundary operation *that layout* pays?

The question is stated so that either answer is publishable: if strided access were faster and the planar boundary
expensive, the record would have been redrafted toward the interleaved option.

## Acceptance criteria

- **A.** Strided per-channel access is not *faster* than contiguous access by more than the null control's spread.
- **B.** The planar boundary transpose costs **less than 5%** of a minimal five-node chain's per-quantum work at
  `Q` = 64.
- **C.** Every figure in a comparison comes from one run, and each comparison is qualified by a control **with its own
  instruction mix**. A control that fails is a statement about the harness, and nothing downstream of it is
  interpreted until that is explained.
- **D.** The layout whose end-to-end per-quantum cost is lower — chain plus its own boundary operation — is selected,
  provided the difference exceeds that comparison's control spread. Within the spread, the result is inconclusive and
  the record says so.
- **E — the aggregation rule.** Each criterion is evaluated on the **median across nine runs**, with the minimum and
  maximum reported alongside. **No run is excluded for any reason**, and no per-run counting rule is applied.

A and B were fixed in ADR-0002 before any data was collected. C, D and E were added later, each **before** the data it
governs was collected, and each because a review pass established that the previous rule could not decide the
question. The sequence is recorded in full below rather than presented as a clean first pass — including the data sets
they replaced.

**The null controls** are `planar_ctl`, `gain_planar_ctl`, and `stereo_planar_ctl`: each repeats its pair's work bit
for bit as a separately timed arm. Three are needed rather than one because three comparisons with different
instruction mixes cannot share a noise floor. **Each control runs before the arm it bounds**, inside a group that keeps
them adjacent, because the evidence rules require the control to run first and a flat rotation would sometimes time a
measurement before its own control. Tightening that also tightened the controls themselves, from medians of 6.6% and
12.2% to 1.5% and 4.8%.

### Corrections retained

Five things this record got wrong before it got them right, each caught by independent review, each kept because the
failure mode is easy to repeat. The last one reversed the conclusion.

| Disproved premise | Current statement | How it was caught |
|-------------------|-------------------|-------------------|
| Criterion B's denominator could be five biquads (`chain5`) | The denominator is the *cheapest defensible* five-node chain, because a share measured against an expensive denominator understates it. Against `chain5` the transpose read 2.1–2.5%; against the minimal chain it reads ~3.5% | Review pointed out that this record's own limitations section already said the real chain would be cheaper |
| A minimal chain could be one fused per-sample loop | The chain is **one call per node**, because that is what a block-based node graph runs. Fused, the whole chain is serially dependent and neither layout can vectorize, so the comparison came back inconclusive — a property of the fixture, not of the layouts | The fused form put criterion D at −0.3% to −6.4%; the per-node form resolves it |
| Criterion D could compare a *mono* chain contiguous against the same chain strided | An interleaved arena stores a mono signal as `Q` contiguous samples and strides only where a signal has channels, so charging a mono chain a stride-2 penalty measured a straw man. Criterion D compares a **stereo** chain in both layouts, with the interleaved arm written the way a competent implementer would — channel-agnostic passes contiguous over the whole `2Q` buffer, only the per-channel-stateful filter stepping frame by frame | Review. The straw-man form reported +20.6% to +29.7%; the fair form reports a median of +13.1% |
| A run that broke criterion B could be called disturbed and a "four of five" rule applied | **Criterion E**, declared before this data set was collected: median of nine, nothing excluded. The earlier pass declared a 5% threshold, saw one run at 5.16%, and invented an aggregation rule afterwards — which is fitting the rule to the data, whatever the run's boundary arms looked like | Review, which named it as an unplanned rule rather than a measurement |
| The interleaved arm's amplifier could index `env[index / CHANNELS]` per sample | It reads the envelope **once per frame** through `chunks_exact_mut(2)`. The per-sample form put an integer division and a redundant load in the inner loop — a penalty the layout does not require. **This reversed the result**: criterion D went from favouring planar by a median +13.1% to favouring interleaved by a median −9.3% | Review, which reproduced the corrected form locally and reported the reversal before this record was trusted |

## Source and environment

- Source revision: `9d794a22` (branch `feat/core-v2-phase-2`); the harness depends on no workspace code.
- Platform: Linux 7.1.8, 13th Gen Intel Core i7-13700H, 6 P-cores + 8 E-cores. Load average ≈ 1.2 throughout, which is
  this machine's permanent background load rather than an idle baseline.
- Pinned to one P-core's two threads with `taskset -c 10,11`, because the P/E split otherwise decides the result.
- `rustc -O -C target-cpu=native`, no workspace profile, no criterion.
- `Q` = 64 frames, two channels, `f32`.

## Inputs

`evd_0008_layout_cost.rs`, committed next to this record. It is standalone by design: adding a benchmark to
`synth_engine_v2` would have meant adding a dev-dependency to the crate whose dependency surface Phase 1's
`crate_boundary.rs` exists to police.

Thirteen arms, rotated in order once per round so a drift across a round cannot land on the same arm every time.
Per-arm result is the **minimum** over rounds; the minimum is the right estimator here because every disturbance on
this machine adds time.

| Arm | What it times |
|-----|----------------|
| `planar` / `planar_ctl` | Two mono `Q`-frame buffers, one contiguous biquad pass each, and its control |
| `strided` | One interleaved `2Q` buffer, one stride-2 biquad pass per channel |
| `transpose` | The boundary a **planar** arena pays: two planar buffers written into one interleaved device buffer |
| `boundary_memcpy` | The boundary an **interleaved** arena pays instead: a straight copy of the same `2Q` samples |
| `chain5` | Five sequential biquad passes over one mono buffer |
| `gain_planar` / `gain_planar_ctl` / `gain_strided` | A non-recursive gain pass in each layout, with its own control |
| `chain_min` | The minimal five-node **mono** chain, one call per node — criterion B's denominator |
| `stereo_planar` / `stereo_planar_ctl` / `stereo_inter` | Criterion D: the minimal five-node **stereo** chain in each layout, plus that layout's own boundary operation |

Five fixture choices carry the record, and each was made because the previous one measured the wrong thing:

- **The gain pair exists because the biquad alone answers the wrong question.** A direct-form biquad is serially
  dependent, so it is latency-bound and neither layout can vectorize. Half the operations in a voice path — an
  amplifier, an output copy, a mix — are non-recursive, and those are where a layout can matter at all.
- **`chain_min` is deliberately the cheapest defensible chain**: a one-pole envelope, a naive saw with no band
  limiting, a one-pole filter rather than a biquad, a gain, and a copy. Every real node this phase will render is more
  expensive than its counterpart here, so a boundary share measured against it *overstates* the share against the real
  path.
- **Every node is a separate `#[inline(never)]` call**, because a block-based node graph dispatches per node and
  fusing them would let the optimizer erase the cost being measured.
- **The interleaved amplifier reads the envelope once per frame**, not once per sample with an integer division to
  find it. The per-sample form is what an inattentive implementation would write, and measuring it reversed the
  record's conclusion — see the corrections table.
- **The two criterion-D arms deliberately do *not* have the same call count.** Planar makes eight node calls for a
  stereo chain — spread, filter and amplifier twice each — where interleaved makes five wider ones. That asymmetry is
  not an artifact to be equalized away: it **is** part of what choosing a layout buys, and the measured margin
  includes it. Equalizing the call counts would have measured a planar arena that nobody would build.

## V1 use-site reads

The other half of this record, and the reason it is `Related` to ADR-0004 as well: the V1 facts both records rest on,
read once here with pinned citations, so that neither decision carries a bare current line citation of its own. All
reads are at `33bf1162`.

| # | Fact | Site | What it establishes |
|---|------|------|---------------------|
| 1 | An `AudioBuffer` is a flat `Vec<f32>` with no channel concept | `crates/synth_core/src/module_traits.rs:113-115` | V1's buffer is already one channel |
| 2 | The output module exposes `in_l`/`in_r` and `out_l`/`out_r` as separate ports | `crates/synth_modules/src/output.rs:148-163` | V1 expresses stereo as two mono ports, not one wide buffer |
| 3 | The cpal output callback receives an interleaved `&mut [f32]` and derives `frames = data.len() / channels` | `crates/pertylizer/src/audio/backends/cpal_backend.rs:312, 321` | Interleaving is a host-boundary property in V1 |
| 4 | A graph node owns a `Box<dyn PolyModule>`, and the render path calls it through that box, once per block per node | `crates/synth_engine/src/graph.rs:65`, called at `:1026` | V1 dispatches dynamically per node on the audio path, not merely in a type declaration |
| 5 | `PolyModule::process` takes `outputs: &mut HashMap<PortName, AudioBuffer>`, and reachable implementations look ports up inside it | `crates/synth_core/src/module_traits.rs:1812-1817`; use sites `crates/synth_modules/src/filter.rs:438`, `crates/synth_modules/src/amplifier.rs:219,222,225` | V1 performs keyed hash lookups per block on the audio thread — the first construct the Phase 2 gate bans. The signature alone would prove only that a map is passed |
| 6 | V2 Phase 1's arena is per-source mono `Q`-frame buffers; interleaving appears only in the output carry | `crates/synth_engine_v2/src/render.rs:284-287`, `crates/synth_engine_v2/src/render/hot.rs:317-331` | V2 is already planar internally |
| 7 | V2 Phase 1 dispatches by matching a closed `PlanOp` enum inside the quantum loop | `crates/synth_engine_v2/src/render/hot.rs:253-333` | The shape ADR-0004 is deciding whether to keep |

Rows 4, 5 and 7 support ADR-0004 rather than ADR-0002 and are recorded here because they were read in the same pass;
ADR-0004's own measurement is a separate record, allocated when P02-T005 runs it.

## Method

Nine independent runs, each 200 rounds of 50 000 iterations per arm, evaluated under criterion E. Within a run every
arm sees the same build, the same binary, and the same pinning, which is what makes the within-run ratios comparable;
absolute times are **not** compared across runs, because the machine's clock behaviour moves between them and the
evidence rules forbid it.

## Commands

```text
cd plans/v2/evidence/phase-02
rustc -O -C target-cpu=native -o /tmp/evd_0008 evd_0008_layout_cost.rs
for i in $(seq 9); do taskset -c 10,11 /tmp/evd_0008 200 50000; done
```

## Results

Per-run ratios, computed within each run; per-arm absolute minima are in
[`EVD-0008-layout-cost.csv`](EVD-0008-layout-cost.csv).

| Run | Kernel control | `strided` vs `planar`, biquad | Gain control | `gain_strided` vs `gain_planar` | `transpose` share of `chain_min` | Criterion-D control | Interleaved vs planar, end to end |
|-----|---------------:|------------------------------:|-------------:|--------------------------------:|---------------------------------:|--------------------:|----------------------------------:|
| 1 | 0.08% | +1.2% | 9.10% | +454.5% | 3.69% | 1.14% | **−4.8%** |
| 2 | 1.45% | +1.9% | 0.20% | +125.6% | 3.62% | 3.71% | **−7.5%** |
| 3 | 3.90% | +4.5% | 0.46% | +131.8% | 3.65% | 2.46% | **−9.3%** |
| 4 | 5.93% | +0.5% | 9.65% | +298.4% | 4.74% | 0.17% | **−12.2%** |
| 5 | 0.56% | +1.8% | 2.41% | +455.0% | 3.42% | 0.97% | **−5.6%** |
| 6 | 0.27% | −6.2% | 4.79% | +128.6% | 3.27% | 2.46% | **−10.5%** |
| 7 | 4.66% | +0.8% | 1.81% | +296.3% | 3.82% | 1.75% | **−14.5%** |
| 8 | 0.21% | +0.9% | 9.91% | +450.4% | 3.26% | 0.36% | **−5.1%** |
| 9 | 6.59% | −5.5% | 11.34% | +455.4% | 3.15% | 6.42% | **−17.3%** |

A **negative** end-to-end figure means the interleaved arrangement is *cheaper*. Every run is negative, and every run's
magnitude exceeds that run's own control spread — nine of nine, including run 9's 17.3% against a 6.42% control. Bold
marks them all.

Aggregated under criterion E — median first, because that is the declared statistic:

| Quantity | Median | Min | Max |
|----------|-------:|----:|----:|
| Kernel control spread | 1.45% | 0.08% | 6.59% |
| `strided` vs `planar`, biquad | +0.92% | −6.18% | +4.51% |
| Gain control spread | 4.79% | 0.20% | 11.34% |
| `gain_strided` vs `gain_planar` | **+298%** | +126% | +455% |
| `transpose` share of `chain_min` | **3.62%** | 3.15% | 4.74% |
| Criterion-D control spread | 1.75% | 0.17% | 6.42% |
| Interleaved vs planar, end to end | **−9.30%** | −17.33% | −4.83% |

The absolute arms behind criterion D, per run, in nanoseconds — planar chain, interleaved chain, transpose, memcpy:

| Run | 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 | 9 |
|-----|--:|--:|--:|--:|--:|--:|--:|--:|--:|
| `stereo_planar` | 345.4 | 370.8 | 357.2 | 352.0 | 350.0 | 379.1 | 372.2 | 351.2 | 384.2 |
| `stereo_inter` | 328.7 | 342.9 | 324.0 | 309.2 | 330.3 | 339.2 | 318.2 | 333.3 | 317.6 |
| `transpose` | 9.7 | 10.3 | 9.6 | 12.5 | 9.7 | 9.1 | 10.0 | 9.1 | 9.1 |
| `boundary_memcpy` | 3.9 | 4.4 | 3.7 | 4.0 | 3.8 | 3.6 | 4.4 | 3.6 | 3.6 |

## Interpretation

**Criterion D selects the interleaved arena, not the planar one.** End to end the interleaved arrangement is cheaper
in all nine runs, median **−9.3%**, range −4.8% to −17.3%, against a criterion-D control whose median spread is 1.75%,
and every run's margin exceeds its own control. This is the reverse of what this record reported one revision
ago, and the reason is recorded in the corrections table rather than smoothed over: the interleaved amplifier was
indexing `env[index / 2]` per sample, and writing it the way anyone would — one envelope read per frame — moved the
result by more than twenty points.

Where the margin comes from is visible in the absolute arms. The interleaved chain makes **five node calls** where the
planar one makes **eight** for the same stereo work, touches one buffer instead of two, and pays a 3.6–4.4 ns memcpy
at the boundary instead of a 9.1–12.5 ns transpose. The per-channel-stateful filter's frame-wise form costs it nothing
measurable at `Q` = 64: two interleaved states in one pass are as cheap as two contiguous passes.

**Criterion A holds, and it bounds less than it appears to.** Naive strided access is a median **+298%** slower than
contiguous for a channel-agnostic gain — but the competent interleaved chain measured under criterion D *does not
stride*: it runs channel-agnostic work over the whole buffer and steps frame-wise only where per-channel state
requires it. Criterion A therefore establishes that a *naive* interleaved implementation is much slower, which is a
statement about implementations rather than about layouts. The biquad arms remain inconclusive (median +0.9%, range
−6.2% to +4.5%, kernel control median 1.45%).

**Criterion B holds.** The planar transpose is 3.15% to 4.74% of the minimal chain, median 3.62%, inside the 5%
threshold in every run. It is also, at ~6 ns more than a memcpy, about a fifth of criterion D's margin — the rest is
the extra node calls and the second buffer.

**Criterion C holds**, with each comparison qualified by a control that repeats its own work and runs before it.

**What the measurement cannot see, and what therefore has to decide the record.** Phase 2's recorded execution choice
is kernel *extraction into a shared home both engines call*. V1 is planar — a mono `AudioBuffer` per port — so a shared
kernel written over `&mut [f32]` serves V1 directly. An interleaved V2 could not call that kernel over its own buffers
without either a second frame-wise variant of every kernel or a de-interleave into scratch. That is an architectural
cost this harness has no way to measure, and it is now the quantity the decision turns on, because the performance
question came back **against** planar rather than for it.

## Limitations

- **`chain_min` and the stereo chain are stand-ins, not the phase's voice path.** They are deliberately the cheapest
  defensible five-node chains. That makes criterion B's boundary share conservative, and it makes criterion D's
  *percentage* an upper bound on the interleaved advantage: a real chain with more layout-insensitive recursive work
  raises both totals and shrinks the margin, though the absolute difference — extra node calls, a second buffer, a
  transpose instead of a memcpy — would hold. When P02-T007 renders the real path, criterion D is recomputed against
  it; nothing here licenses skipping that.
- **The stereo signal in the criterion-D arms is a duplicated mono source with one shared envelope.** That is what a
  monophonic voice into a stereo bus looks like, and it is favourable to the interleaved arm, whose amplifier reads
  one envelope value per frame. A path with genuinely independent per-channel control values would read two, and the
  margin would narrow by an amount this record has not measured.
- One machine, one microarchitecture, `target-cpu=native`. The gain ratio behind criterion A would narrow on a
  platform with narrower vectors; criterion D's margin is call-count and buffer-count driven and would not.
- The harness measures one arithmetic workload at one quantum. It does not model cache pressure from a large graph,
  where many mono buffers could behave differently from one wide one — a direction that would favour interleaved
  further.
- Criterion B's denominator is a **mono** chain while the transpose it is compared against is a stereo boundary
  operation, which is the conservative pairing.
- **Nothing here measures the cost the decision now turns on**: whether one kernel shape can be shared between a
  planar V1 and V2. That is an architectural question, not a timing one.
- Nothing here measures V1 or V2 code. It measures the two representations.

## Conclusion

**Not supported — for the hypothesis this record was written to test.** Under the declared aggregation rule:

- criterion D **selects the interleaved arena**, by a median of 9.3% end to end, negative in all nine runs against a
  control spread of 1.75%;
- criterion B holds (planar transpose a median 3.62% of the minimal chain, under the 5% threshold);
- criterion A holds but bounds only *naive* strided implementations, which the criterion-D interleaved arm is not.

The measurement therefore does not support accepting ADR-0002 on performance grounds. Whatever layout that record
selects, it must now do so on the architectural argument — one kernel shape shared with V1, which the extraction path
depends on — and must state the measured cost of that choice: about 9% of a minimal stereo chain's per-quantum work on
this fixture and this machine.

This is the outcome the question was phrased to allow. The earlier revision of this record reported the opposite and
was wrong for a reason that had nothing to do with layouts.

## Artifacts

| Artifact | Location/digest | Retention or reproduction |
|----------|-----------------|---------------------------|
| Harness | [`evd_0008_layout_cost.rs`](evd_0008_layout_cost.rs) | Committed; rebuild with the command above |
| Raw results | [`EVD-0008-layout-cost.csv`](EVD-0008-layout-cost.csv) | Committed; nine runs, thirteen arms, corrected harness |
