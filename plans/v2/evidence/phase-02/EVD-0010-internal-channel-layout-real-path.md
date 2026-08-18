# EVD-0010: Internal channel layout, re-measured against the real voice path

| Field         | Value                                                        |
|---------------|--------------------------------------------------------------|
| ID            | EVD-0010                                                     |
| Status        | Complete                                                     |
| Phase         | 02                                                           |
| Created       | 2026-08-18                                                   |
| Last reviewed | 2026-08-18                                                   |
| Retention     | Permanent                                                    |
| Related       | ADR-0002, ADR-0040, ADR-0041, P02-T012, EVD-0008, EVD-0009    |
| Superseded by | —                                                            |

Permanent because the companion layout record reserved as ADR-0041 is decided on it.

## Question or hypothesis

[EVD-0008](EVD-0008-internal-channel-layout-cost.md) chose between a **planar** arena — a buffer is one channel — and
an **interleaved** one by modelling both layouts under invented arithmetic, because no V2 voice path existed. It
selected interleaved by a median 9.3% end to end, and said of itself that its chains are *"stand-ins, not the phase's
voice path"*, with the re-measurement owed once a real path existed.

[ADR-0002](../../decisions/ADR-0002-internal-channel-layout.md) was then accepted **against** that measurement, on the
argument that a shared kernel written over `&mut [f32]` serves a planar V1 unchanged.
[ADR-0040](../../decisions/ADR-0040-v2-owns-its-dsp.md) proposes removing exactly that premise, which triggers
ADR-0002's second revisit condition verbatim. P02-T005 built the real voice path, so the owed re-measurement can now
run against **the crate's own kernels over the crate's own arena** instead of against a model of two memory layouts.

The question: **on the real V2 voice path, what does the planar arena cost against an interleaved one, end to end?**

"The real voice path" has three honest readings, and they answer different questions, so all three are measured:

- **Shape A — the path as the crate compiles it today.** A stereo profile compiles the minimal voice patch to a *mono*
  chain — sine, filter, envelope, amplifier — plus one widening copy and two strided output writes. This is what the
  phase renders now.
- **Shape B — a stereo signal chain.** The widening moved upstream of the filter and the amplifier, which is what a
  path carrying a stereo signal looks like once a node produces one. This is EVD-0008's criterion-D shape rebuilt with
  V2's real kernels.
- **Shape C — independent per-channel control.** Shape B with two envelopes and two filter settings. EVD-0008 named
  this as the case it had not measured and expected to *narrow* the interleaved margin, because its interleaved
  amplifier reads one control value per frame where this one reads two.

The question is stated so that either answer is publishable. If planar is cheaper on the real path, ADR-0041 retains
it on merit rather than on a withdrawn premise; if interleaved is cheaper by materially more than EVD-0008's 9.3%,
ADR-0002's own revisit condition says that is a reason to reopen it.

## Acceptance criteria

Fixed **before any data was collected**, and, where they are the same rules, taken verbatim from
[EVD-0009](EVD-0009-dispatch-cost.md) — which arrived at them through nine retained corrections.

- **A — the falsifier, stated first.** Each layout arm is paired with a **null control that is that arm's own code**,
  so the spread between them is this comparison's noise floor in this comparison's instruction mix. If a shape's
  margin does not exceed that shape's own control spread, the shape is **inconclusive** and nothing is concluded from
  it. A control that itself misbehaves is a statement about the harness, and nothing downstream of it is interpreted
  until that is explained.
- **B — agreement before timing.** Within a shape, the planar and interleaved arms are run past the envelope's attack
  and decay from their own states, and **the carries they produce are compared sample for sample**. They must be
  **bit-identical**, and neither may be silent, or the run aborts and prints no timings. This is criterion B's whole
  weight: the interleaved kernels are hand-written counterparts of the crate's, and bit-identity is what separates a
  counterpart from a model. EVD-0008's conclusion was reversed once by an interleaved arm that did arithmetic no
  layout requires.
- **C — the aggregation rule.** Each arm's *time* is the **median across nine program runs** of that run's **minimum
  across rounds**. Each *ratio* is **paired within a round**: taken inside each round, the median over a run's rounds
  is that run's figure, and the median over nine runs is the reported figure. **No run is excluded for any reason.**
- **D — controls compared paired and adjacent.** Six null controls, one per shape and layout, each computed **within
  the round both it and its arm were measured in**, and reported as the median of those per-round spreads. Dividing
  two independently aggregated medians cancels drift rather than measuring it.
- **E — rotation in groups.** The arm order advances by one **group** each round, where a control and the arm it
  bounds are one group, so no arm keeps a position and no rotation can put an arm before its own control.
- **F — symmetry, stated as a rule rather than assumed.** Both layouts are measured **direct-call**, with every node
  call behind an `#[inline(never)]` boundary in both arms, because a block-based node graph dispatches per node and
  fusing the chain would erase the buffer traffic that *is* the subject. Dispatch is therefore not confounded with
  layout — and because planar makes **more** node calls, this understates planar's cost. The understatement is
  bounded rather than waved at: `walk_only` and `bind_only` measure what one step's binding costs over the real
  arena, and the record states planar's extra calls times that figure.
- **G — the classification rule.** Each shape's outcome is read off the paired median and classed as one of:
  *planar cheaper*, *inconclusive* (inside its own control spread), *interleaved cheaper by less than EVD-0008's
  9.3%*, or *interleaved cheaper by materially more than 9.3%* — which is the threshold
  [ADR-0002's first revisit condition](../../decisions/ADR-0002-internal-channel-layout.md#revisit-conditions) names
  as a reason to reopen. **This record does not choose ADR-0041's outcome.** It supplies the figure that record
  weighs against ADR-0002 clause 2's kernel contract, which is an independent merit no measurement can price.

## Source and environment

- Source revision: `6d244e4d` (branch `feat/core-v2-phase-2`), the commit holding the exact harness this data was
  produced by — symmetric call discipline, one kernel contract, each layout's own arena map, and the per-quantum
  bit-identity check. Earlier revisions of the harness produce different figures and are not the ones measured here;
  what they reported and why is in the corrections table.
- Platform: Linux 7.1.8, 13th Gen Intel Core i7-13700H, 6 P-cores + 8 E-cores. Load average ≈ 1.2 throughout, which is
  this machine's permanent background load rather than an idle baseline.
- Pinned with `taskset -c 10` — one thread of one P-core, which is what EVD-0009 established brings the control spread
  to something a small margin can be read against. Here the controls sit at a **median 0.37% to 0.50%** per arm, worst
  0.73%.
- `cargo build --release`, workspace profile (`lto = true`, `codegen-units = 1`, `panic = abort`), no
  `target-cpu=native`, no criterion. `rustc 1.97.1`.
- `Q` = 64 frames, **stereo**, 48 kHz, `f32`.

## Inputs

The **minimal voice path** — an envelope, a sine, a two-pole filter, an amplifier, an output — built through the
crate's own API and compiled by its own admission, at a **stereo** host profile. Every arm runs the crate's kernels
over the crate's prepared records; the interleaved counterparts of the filter, the amplifier and the widening are the
crate's arithmetic in the crate's order, stepping frames instead of samples, and criterion B is what holds them to it.

### What a stereo profile actually compiles today, and why it decides the shape of this record

`compile` produces this, asserted operation by operation at startup: **sine → filter → envelope → amplifier**, all
*mono*, then **one widening copy** into the slot the envelope has finished with, then **two strided output writes**.
Two buffers. The phase's real path is therefore mono until its last two operations, and the layout question barely
touches it — which is a finding rather than a technicality, and it is why this record measures three shapes rather
than one.

| Shape | What it is | Planar | Interleaved |
|-------|------------|--------|-------------|
| **A — `voice`** | The plan above, exactly | 5 node calls, 2 buffers, a widening copy and two strided writes | 5 node calls, 3 `Q` slots, a widening into a `2Q` buffer and one memcpy |
| **B — `bus`** | The widening moved upstream, so the filter and the amplifier run per channel | 7 node calls, 3 buffers | 5 node calls, 3 `Q` slots |
| **C — `split`** | Shape B with a filter and an envelope of its own per channel | 8 node calls, 3 buffers | 6 node calls, 4 `Q` slots |

Shapes B and C are **not constructible through the compiler today** — no node kind produces stereo, so the only
widening the validator inserts is the one at the output. They are what the path becomes when one does, and they are
built from the same kernels and the same records as shape A. That is a limitation, stated again below.

| Arm | What it times |
|-----|----------------|
| `voice_planar_ctl` / `voice_planar` | Shape A, planar. The two arms are the same code; the spread between them is this comparison's noise floor |
| `voice_asis_ctl` / `voice_asis` | Shape A, planar, ending in the output write `render/hot.rs` performs **today** — index arithmetic per sample instead of a frame-strided iterator |
| `voice_inter_ctl` / `voice_inter` | Shape A, interleaved |
| `bus_planar_ctl` / `bus_planar`, `bus_inter_ctl` / `bus_inter` | Shape B, each layout with its own control |
| `split_planar_ctl` / `split_planar`, `split_inter_ctl` / `split_inter` | Shape C, each layout with its own control |
| `walk_only` / `bind_only` | The real stereo plan's schedule walked, and walked *and bound*. Their difference is what one step's binding costs, which is how the record bounds what the direct-call form leaves out |
| `renderer` | One `render` call for one quantum through the real renderer, for scale |

### Corrections retained

Six asymmetries, **every one found by review before any data was collected**, and five of them favouring the
interleaved arm. They are kept because each is the same failure in a different costume: a comparison in which one side
does work the other does not, for a reason that is not the thing being compared.

| Disproved premise | Current statement | How it was caught |
|-------------------|-------------------|-------------------|
| "Both arms are direct-call, so dispatch is out of the comparison" | The planar shim took the kernel as a **function pointer**, which compiles to an indirect call, and the interleaved kernels were called by name. The planar arms have more nodes, so they paid more of it. Every kernel in every arm is now a direct call to an `#[inline(never)]` shim | Independent review of the harness design |
| The interleaved kernels could take raw coefficients and raw slices | They take a prepared record and a state as enums and read their inputs through the crate's own `NodeIo`, destructured with the same return-without-writing prologue. The earlier form compared a generic planar ABI against a bespoke interleaved one | Independent review |
| An interleaved arm needs one slot per signal | It needs its **peak liveness**, like any arena: the mono buffer dies at the widening, so a control takes its slot. The earlier maps gave every interleaved arm one `Q` slot too many, which favoured planar | Independent review |
| The carry form could be selected through a function pointer | Two methods, both direct. The pointer put an indirect call in the planar arms only — the first correction again, one layer down | Independent review of the committed harness |
| The region helper could be an ordinary function | Inlined. It was a call of its own, and the planar arms make more multi-region operations, so it charged them harness plumbing in proportion to the thing being measured | Disassembly, after the review |
| An interleaved kernel could check its own prepared variant | The widening and the amplifier read neither prepared record nor state, because the crate's `copy` and `amplifier` read neither. Only the interleaved arms were paying those branches | Independent review of the committed harness |

A seventh correction belongs to the *renderer* arm rather than to the layouts, and it is recorded here because it is
a trap in the crate rather than in the harness: **`PreparedRenderer::prepare` primes the carry with `Q` frames of
silence**, so the first `render` call of exactly `Q` frames serves that priming and renders **no quantum**. An event
presented on that call is outside its span, which fails the whole call — and a harness that ignores the returned error
goes on to time a voice whose gate was never opened. This one was caught by comparing the renderer's carry against the
hand-written arm's and finding silence. Two consequences: this harness renders one un-evented call first and treats
any rejected call as fatal, and **EVD-0009's renderer arm has the same defect** — verified by running its committed
harness, whose gate call returns `EventOutsideCallSpan` and whose renderer arm therefore rendered an ungated voice.
Its arithmetic per quantum is the same either way, so its figure stands; the note is in that record.

## Method

Nine independent runs of 25 rounds of 50 000 iterations per arm. Groups are rotated once per round; within a group the
control runs first; the minimum over rounds is one run's figure; the median over nine runs is the reported figure.
Each control is compared to its arm **within the round both were measured in**, and every ratio is taken within a
round as well.

Before any timing, all seven arms are run 200 quanta from their own states — through the attack, the decay and into
the sustain — and their carries are compared **at every one of those quanta**, as raw bit patterns. The run aborts and
prints no timings on the first sample that differs.

## Commands

```bash
cargo build --release --example evd_0010_layout_cost -p synth_engine_v2
for i in $(seq 9); do taskset -c 10 target/release/examples/evd_0010_layout_cost 25 50000; done
```

## Results

**Criterion B held in all nine runs**: every arm bit-identical to its counterpart over 200 quanta, quietest peak
0.4559, and shape A's planar arm bit-identical to the real renderer's carry.

Per-arm times, nanoseconds per quantum, median of nine runs' minima:

| Arm | Median | Min | Max |
|-----|-------:|----:|----:|
| `voice_planar` | 719.8 | 712.1 | 748.2 |
| `voice_asis` | 767.8 | 732.4 | 794.9 |
| `voice_inter` | 706.7 | 662.7 | 728.7 |
| `bus_planar` | 945.1 | 875.5 | 982.7 |
| `bus_inter` | 735.3 | 704.2 | 771.0 |
| `split_planar` | 1009.0 | 981.9 | 1046.8 |
| `split_inter` | 882.4 | 845.5 | 936.1 |
| `walk_only` | 1.1 | 1.0 | 1.1 |
| `bind_only` | 33.9 | 30.6 | 37.1 |
| `renderer` | 853.1 | 787.2 | 884.6 |

The decision figures, paired within each round. **Negative means the interleaved arrangement is cheaper**, which is
EVD-0008's sign convention for the same comparison:

| Run | Shape A | its controls | Shape B | its controls | Shape C | its controls |
|-----|--------:|-------------:|--------:|-------------:|--------:|-------------:|
| 1 | **−2.51%** | 0.46 / 0.44 | **−21.70%** | 0.30 / 0.36 | **−10.92%** | 0.43 / 0.34 |
| 2 | **−2.54%** | 0.26 / 0.67 | **−21.46%** | 0.42 / 0.39 | **−10.86%** | 0.30 / 0.35 |
| 3 | **−2.69%** | 0.40 / 0.38 | **−21.56%** | 0.43 / 0.41 | **−11.19%** | 0.35 / 0.37 |
| 4 | **−2.42%** | 0.50 / 0.51 | **−21.41%** | 0.39 / 0.60 | **−11.05%** | 0.46 / 0.62 |
| 5 | **−2.32%** | 0.29 / 0.50 | **−21.88%** | 0.49 / 0.44 | **−11.00%** | 0.39 / 0.36 |
| 6 | **−2.64%** | 0.63 / 0.68 | **−21.43%** | 0.43 / 0.52 | **−11.10%** | 0.28 / 0.49 |
| 7 | **−2.76%** | 0.73 / 0.47 | **−21.43%** | 0.44 / 0.37 | **−11.18%** | 0.37 / 0.49 |
| 8 | **−2.50%** | 0.59 / 0.51 | **−22.21%** | 0.36 / 0.60 | **−10.94%** | 0.44 / 0.49 |
| 9 | **−2.58%** | 0.42 / 0.49 | **−21.71%** | 0.23 / 0.50 | **−11.24%** | 0.36 / 0.28 |

Every run of every shape is negative, and every margin exceeds its own controls — by about five times in shape A and
by fifty in shape B. Aggregated under criterion C:

| Quantity | Median | Min | Max |
|----------|-------:|----:|----:|
| Shape A, interleaved vs planar, paired | **−2.54%** | −2.76% | −2.32% |
| Shape A, the same unpaired | −2.12% | −6.93% | +1.34% |
| Shape A, paired difference | **−19.6 ns** | −21.0 | −17.9 |
| Shape B, interleaved vs planar, paired | **−21.56%** | −22.21% | −21.41% |
| Shape B, paired difference | **−215.2 ns** | −221.5 | −212.5 |
| Shape C, interleaved vs planar, paired | **−11.05%** | −11.24% | −10.86% |
| Shape C, paired difference | **−118.1 ns** | −120.2 | −113.6 |
| Control spread, all seven arms | 0.37–0.50% | 0.23% | 0.73% |
| One step's binding, paired | 34.5 ns for 5 steps | 33.6 | 37.5 |
| `renderer` against `voice_planar`, paired | +16.89% | +16.57% | +17.28% |
| The renderer's **current** carry write against the frame-strided one | **+45.8 ns**, +5.91% | +45.2 | +48.9 |

Shape A's unpaired figure ranges from −6.93% to +1.34% while its paired figure holds within half a point — the same
demonstration EVD-0009's ninth correction records, on a margin small enough that the unpaired form would have changed
the sign.

## Interpretation

**On the path the phase renders today, the layout is worth about 2.5%. On a path whose signal genuinely has channels,
it is worth 11% to 22%.** Both halves of that sentence are the answer, and neither is the whole one.

**Shape A: −2.54%, or 19.6 ns.** The mono chain is *identical* in both arms — the crate's own kernels over one
contiguous buffer, because an interleaved arena stores a mono signal exactly as a planar one does. The entire
difference is the last two operations: planar copies `Q` samples and then writes two strided channels; interleaved
widens into a `2Q` buffer and memcpys it. That is the honest measurement of what ADR-0002 costs the phase **right
now**, and it is a quarter of what EVD-0008 predicted, because EVD-0008's stereo fixture assumed a chain that this
phase does not yet have.

**Shape B: −21.56%, or 215 ns.** Once the widening moves upstream, planar runs the filter and the amplifier twice —
eight buffer traversals against five, seven node calls against five, and the amplifier reading the envelope twice
where interleaved reads it once per frame. This is EVD-0008's criterion-D shape with real kernels, and it is **more
than twice** the margin that record reported.

**Shape C: −11.05%, or 118 ns.** Giving each channel its own envelope and its own filter setting takes back **half**
of shape B's margin, which is the measured version of the limitation EVD-0008 stated and could not size: its
interleaved amplifier read one control value per frame, and a path with independent per-channel control reads two.
Half the advantage was that one read. What remains is the call count and the buffer count.

**What the direct-call form leaves out — an estimate, and labelled as one.** Both layouts are measured with no
dispatch at all, so neither pays for reaching its kernels. The real renderer does: `bind_only` against `walk_only`
puts the binding of **shape A's five-step schedule at 34.5 ns, about 6.9 ns a step**. Shape A's two arms make the same
number of node calls, so this changes nothing there. Shapes B and C give planar **two more node calls** than
interleaved, which at that per-step figure is roughly **14 ns** on each of those margins.

That 14 ns is an **estimate rather than a measurement**, and three things make it so: `bind_only` walks shape A's
schedule over shape A's planar arena, not shape B's or C's and not an interleaved one; binding cost depends on a
step's regions and its input bindings, which differ between those schedules; and the figure covers the arena binding
only, leaving out the prepared and state lookups and the indirect call itself, which EVD-0009 measured together at
+2.12% for the table against a closed enum. Measuring it properly would need schedules the compiler cannot yet
produce. The direction is not in doubt — planar makes more calls, and every call costs something — but the size of
the correction is not established here, and no conclusion below rests on it.

**A finding about the renderer rather than about the layouts.** `render/hot.rs` writes its output carry with index
arithmetic per sample, and the frame-strided form costs **45.8 ns less per quantum**, 5.9% of shape A's cost. That is
**more than twice the entire layout margin on the path the phase actually renders** (19.6 ns). It is an implementation
debt, not a layout cost, and this record keeps it out of the comparison for exactly that reason — but a reader
weighing 19.6 ns should know that 45.8 ns is sitting next to it, available for the price of an iterator.

**A structural observation the timings do not carry.** In every shape, the interleaved arena needs **more memory**,
not less: a mono-to-stereo widening cannot reuse a mono slot, so shape A needs 3 `Q` slots against planar's 2, and
shape C needs 4 against 3. At `Q` = 64 everything is in L1 and it costs nothing measurable. On a large graph it is a
direction, and it points the other way from the timings.

**Classification under criterion G, including where the rule was not sharp enough.** The rule asks whether each shape
is *"materially more"* than EVD-0008's 9.3% — and it never fixed a number for "materially". That is a defect in the
rule, and the right response is to report it rather than to pick a threshold now that the data is visible, which is
the mistake EVD-0008's fourth correction records.

| Shape | Paired median | Class |
|-------|--------------:|-------|
| A — as compiled today | −2.54% | **Interleaved cheaper, by clearly less than 9.3%** — and outside its own control spread, so not inconclusive |
| B — a stereo chain | −21.56% | **Interleaved cheaper by more than twice 9.3%.** Material on any reading — for a shape the compiler cannot yet produce |
| C — independent per-channel control | −11.05% | **Interleaved cheaper, above 9.3% but by less than two points.** Whether that is "material" is the judgement the rule left open, and ADR-0041 makes it |

## Limitations

- **Only shape A exists.** Shapes B and C cannot be compiled today, because no node kind produces a stereo signal and
  the only widening the validator inserts is the one at the output. They are built from the same kernels, the same
  prepared records and the same arena discipline, and they are what the path becomes when a stereo node arrives — but
  they are a projection, and the record does not present them as the plan the crate renders.
- **The interleaved kernels are hand-written counterparts.** Bit-identity over 200 quanta establishes that they
  compute the same signal, not that a real interleaved V2 would be written exactly this way. The one place this
  matters most is shape C: an interleaved node with per-channel settings is modelled as one node holding two
  coefficient sets, which is the *friendly* form. The alternative a real engine might reach for — de-interleaving into
  two mono buffers and back — is not measured and would cost interleaved more.
- **Both layouts leave an optimization on the table, and neither is taken.** Planar's shape A could write channel 1
  from the same buffer and skip the widening copy entirely; interleaved could alias its final stereo buffer with the
  host carry and skip the memcpy. Both are schedule changes rather than layout properties, and ADR-0002 clause 2 is
  what requires the copy on the planar side, so removing it would be a change to the record under review rather than a
  fairer measurement of it.
- **A duplicated mono source in shape B.** As in EVD-0008: one envelope, two identical channels. Shape C is the
  correction for it, and the margin halves.
- **One machine, one microarchitecture, one quantum, one channel count.** No `target-cpu=native` this time, unlike
  EVD-0008; the workspace release profile with fat LTO, which is what the crate ships.
- **Nothing here prices the architectural merit.** ADR-0002 clause 2 — *"a kernel receives one channel and is never
  told how many there are"* — is a property of the kernel contract, and no harness can put a number on it. It is what
  ADR-0041 must weigh these figures against, and this record deliberately does not do that weighing.

## Conclusion

**Supported, and against the record under review.** Measured on the crate's own kernels over the crate's own arena,
with every asymmetry review could find closed first:

- the interleaved arrangement is cheaper in **all nine runs of all three shapes**, and every margin exceeds its own
  control spread;
- on the path the phase **compiles today**, by a median **2.54%** (19.6 ns of 720) — well under EVD-0008's 9.3%,
  because that record's stereo fixture assumed a chain this phase does not yet have;
- on a path whose signal genuinely has channels, by a median **21.56%**, and by **11.05%** when each channel carries
  its own control — the first materially above ADR-0002's stated reopening threshold on any reading, the second above
  it by a margin the rule did not sharpen in advance;
- and the direct-call form understates planar's cost in those two shapes, by an amount this record estimates at about
  14 ns each and explicitly does not claim to have measured.

**What that does and does not settle about ADR-0002's first revisit condition.** That condition asks for a margin
materially larger than 9% *on the real voice path*, and the only shape that is the real voice path — the one the
compiler produces — came back at **2.54%**. **On its own terms the condition is therefore not met**, and this record
does not claim it is. What reopens ADR-0002 is its *second* condition, which ADR-0040 triggers by removing the
shared-kernel premise. The two projected shapes do not decide anything by themselves either; what they establish is
the **shape of the cost curve** — that the margin is small while signals are mono and becomes 11% to 22% the moment
one is not, which is the same direction ADR-0002's own text predicts when it says the cheapest moment to change the
layout is now.

What the record cannot decide, and does not try to, is whether the kernel contract clause 2 states is worth 2.5% today
and 11% to 22% as the catalog grows. That is ADR-0041's, and this is the figure it weighs.

## Artifacts

| Artifact | Location/digest | Retention or reproduction |
|----------|-----------------|---------------------------|
| Harness | [`crates/synth_engine_v2/examples/evd_0010_layout_cost.rs`](../../../../crates/synth_engine_v2/examples/evd_0010_layout_cost.rs) | Committed at `6d244e4d`; rebuild with the command above |
| Raw results | [`EVD-0010-layout-cost.csv`](EVD-0010-layout-cost.csv) | Committed; nine runs, seventeen arms, every ratio and control |
