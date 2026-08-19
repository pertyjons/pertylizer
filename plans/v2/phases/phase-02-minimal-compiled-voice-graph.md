# Phase 2: Minimal Compiled Voice Graph

> **Frozen execution record.** Active Phase 2 state moved to
> [`../NOW.md`](../NOW.md) on 2026-08-19. This document preserves the detailed
> execution and review chronology as it stood before the workflow
> simplification; it is not an active status or current-contract authority.

| Field | Value |
|-------|-------|
| Record state | Frozen 2026-08-18; see [`NOW.md`](../NOW.md) |
| Phase | 02 |
| Started | 2026-08-17 |
| Last updated | 2026-08-18 |
| Master plan | [`master-plan.md`](../master-plan.md#phase-2-minimal-compiled-voice-graph) |
| Phase status at freeze | Active |
| Exit review at freeze | Not created |

## Objective

Compile and render exactly one vertical sound path — note events into an envelope, an oscillator, a filter, an
amplifier, and the plan's output — through a graph the compiler validated, ordered, and laid out. Phase 1 built a
renderer that executes a hand-written operation list; Phase 2 is where the operation list stops being hand-written: a
plan becomes compact numeric slots, a topological schedule, an arena whose buffers are reused when their lifetimes do
not overlap, and nodes whose immutable prepared data is separated from their mutable state.

It is also where `Q` stops being provisional. ADR-0037's 64 frames was accepted in Phase 0A against a V1 proxy that
came back inconclusive, and this phase's exit gate makes the re-measurement **binding** — against real V2 nodes, which
is the first time such a thing exists.

This section records the scope and entry assumptions at the migration point.
[`../ROADMAP.md`](../ROADMAP.md) owns the live outcome,
[`../NOW.md`](../NOW.md) owns current task state, and
[`../PROCESS.md`](../PROCESS.md) owns the working method.

## Entry conditions

- Phase 1 closed at [`REV-P01`](../reviews/phase-01-exit-review.md), `Accepted`; `synth_engine_v2` is on `main` at
  `33bf1162` with its five gate bullets each decided by an executable check.
- The reference corpus and both headless commands exist — `pertylizer render` and `pertylizer compare`, P00A-T001 and
  P00A-T002. They were the entry condition for the extraction path, and they remain the instrument for **comparing**
  the two engines whether or not any code is shared. See
  [the execution choice](#recorded-execution-choice-extraction-rather-than-adapters), which is under challenge.
- No Phase 3 mechanism is required. Renderer ingress, the deferred store, and the scheduler that *derives* an event's
  timestamp from a timeline stay Phase 3's; this phase's events remain the prevalidated bounded span the host-profile
  specification defines, presented by the caller.

  **What that does not defer is where a note edge lands.** ADR-0001 clause 14 splits the two deliberately: a
  note-on, note-off, gate, or retrigger occurs **at its declared sample inside the quantum**, while only the
  *control-rate* response to a mid-quantum event begins at the next boundary (clause 13). Phase 2 therefore applies
  gate edges at their sample offset and evaluates control values once per quantum — quantizing the note edge to the
  boundary would be a contract violation, not a deferral, and would be invisible in a fixture whose notes happen to
  start on one.

## Required decisions

Four decisions target this phase. Each is pinned to the task that first depends on it rather than to phase entry,
because three of them are decisions this phase's own prototype is supposed to inform — accepting them before the code
that measures them exists would be the "decide it because a gate needs its status" the working agreement forbids.

| ADR | Required status | Deadline or permitted deferral |
| --- | --- | --- |
| ADR-0002 | `Accepted` | Before P02-T002 lands — that task validates channel-layout compatibility, so it is the first to consume the semantics, not P02-T003 which merely converts between them. Phase 1 deviation 4 deliberately left `ChannelLayout` at `Mono`/`Stereo` so that this record, not the implementation, defines the vocabulary |
| ADR-0004 | `Accepted` | Before P02-T005 lands. Native node representation. **It also decides what the fourth gate bullet means**: "adding a second simple DSP node does not require changing renderer control flow" is undecidable until the record says what counts as control flow and what counts as a table entry |
| ADR-0005 | `Accepted` | Before P02-T004 lands. Buffer liveness. The master plan's own guidance is conservative-first, so the record is expected to be short — but the arena's reuse rule is a correctness contract, not a value, and a wrong one aliases two live signals |
| ADR-0033 | `Proposed` permitted | Phase 3. Phase 2 **refuses every cycle** rather than scheduling one, which needs no feedback rule: the refusal is the status quo, and the delay-boundary rule is what would relax it. Until it is accepted, nothing here may assume a cycle can be broken, and the cycle diagnostic must not promise a rule that does not exist |
| ADR-0037 | `Accepted`, **provisional** | Confirmed or superseded at this phase's exit gate (P02-T010). Until it passes, nothing may be tuned to `Q`: no hand-unrolled kernel, no `Q`-specific buffer layout, no test asserting a control rate in hertz. **Sized by `Q` is not tuned to it** — the renderer's per-source buffers and both ADR-0001 carries are already `Q`-sized and follow the constant when it changes; what the restriction forbids is a layout that only works at one value |
| ADR-0001, ADR-0021, ADR-0032, ADR-0038 | `Accepted` | Carried from Phase 1 unchanged. This phase adds nodes to a renderer that already implements them |

## Recorded execution choice: extraction rather than adapters

> **Under challenge, and not a live instruction.** [ADR-0040](../decisions/ADR-0040-v2-owns-its-dsp.md) proposes that
> V2 owns its DSP and that this extraction does not happen. Until that record and its companion layout decision are
> resolved, everything in this section is history rather than direction: it records what was chosen on 2026-08-17 and
> why, so the challenge can be read against it. Nothing here should be started.

The master-plan work list permits either — "use existing DSP kernels where their state and configuration can be
separated cleanly. Use a narrow temporary adapter where that is cheaper than extraction." **This phase takes the
extraction path**, decided by the user on 2026-08-17, to mix as little V1 into V2 as possible.

The kernel gets **one shared home, and both engines call it**. The home is `synth_dsp`: it already holds both stateless
math and stateful structs, and it is already on `synth_engine_v2`'s four-name dependency allowlist, so nothing about the
crate boundary changes. Putting a shared kernel in `synth_engine_v2` instead is **excluded rather than merely worse** —
V1 would then depend on V2 and break the accepted Phase 1 exit gate that the crate can be deleted without affecting V1.

One cost is accepted openly rather than left to be discovered: `synth_dsp` depends on `synth_core` **and on `realfft`**
(`crates/synth_dsp/Cargo.toml` at `33bf1162`), for `spectral.rs`. V2 inherits that transitively the moment it calls a
shared kernel. It breaks no accepted rule — the allowlist names the crate, not its dependency closure — but the claim
that V2's dependency surface stays at `synth_core` would be false, and if the transitive pull ever matters the fix is a
feature flag on the spectral module, not a new crate.

Three consequences this phase carries because of it:

1. **Extraction changes V1 code**, because it separates an existing kernel's state from its configuration in place
   rather than wrapping it. The acceptance instrument is the ten-case reference corpus: each case's rendered
   `output.sha256` must be **bit-identical across the extraction**. Running it is not optional, and it is what makes
   the path safe rather than merely tidy.

   It is also **not sufficient by itself**, and P02-T006 owns the gap: every one of the ten fixtures has an empty
   `automation` list on every pattern, so no corpus digest exercises the parameter-override path at all. A regression
   in it would leave all ten digests bit-identical. The task therefore adds an eleventh instrument before the
   extraction lands: a project whose automation lane drives an ADSR value, rendered headlessly, with its
   `output.sha256` **pinned before the extraction and asserted after**. Two separate things have to hold, and only
   together do they close the gap:

   - a **control**, run first, proving the fixture exercises the path at all — the same render with the lane removed
     must produce a different digest, or the pinned digest is pinning nothing;
   - the **pinned digest itself**, which is what catches a changed resolved value or changed timing. "Automation still
     changes the render" would pass while the override resolved to a different number, which is precisely the
     regression this exists to catch.
2. **It is also what makes the `Q` re-measurement mean anything.** ADR-0037's re-measurement is binding on the real V2
   node model; measuring through an adapter would have measured the adapter.
3. **The coefficient trap is real and it is the expensive kind.** V1's envelope recomputes its stage times, its
   exponential coefficient, and its curve shaping **per sample** (`synth_modules/src/envelope.rs:257` at `33bf1162`,
   inside `process_sample`). V2 wants that as prepared per-quantum data, which is a genuine speed win and may make V2
   measure *faster* than V1 — but moving it **changes the numbers** whenever a control value moves inside a quantum.
   The shared kernel must therefore be *parameterised* on when the coefficient is computed rather than quietly
   changed. Done wrong it surfaces as a corpus digest failure with no obvious cause.

The survey that preceded this choice concluded two things worth not re-deriving:

- **`synth_dsp::filters` needs no extraction at all.** `SvfCoeffs` and `BiquadCoeffs` are already pure configuration
  with `process(input, &mut state, ..)`, and `StereoSvf` is the mirror image — a state-only struct taking `&SvfCoeffs`.
  The `oscillators` free functions are stateless and `MinBlepTable` is prepared data by nature. **So the vertical path
  starts at the filter**, which can render before the envelope extraction is begun.
- **The work is in `synth_modules`**, where one struct holds configuration, state, prepared data, and V1 plumbing that
  V2 does not reproduce in this phase: `AudioBuffer` outputs (V2 has the arena), an `Arc<..PositionBuffer>` of atomics
  (V1 GUI telemetry; an observation tap in V2), `mod_offsets` (V1's mod matrix; Phase 7 in V2), and `override_*`. The
  shared part of the envelope is the stage machine and the curve shaping, and nothing else.

  **`override_*` is not mod-matrix plumbing**, and treating it as such is how the extraction would silently drop a
  shipped feature. It is the *sequencer automation* path: an `AutomationTarget::{Instrument, Module}` lane reaches
  `Instrument::apply_param_override`, then `VoiceGraph::apply_param_override` (`synth_engine/src/graph.rs:617` at
  `33bf1162`), then the module's `set_param_override`, which is what replaces an effective ADSR value while leaving
  the stored base untouched. V2 owns that as central automation in Phase 5, not Phase 7. The extraction keeps the
  override *composition* on the V1 side of the seam — the shared kernel receives already-resolved values — so V1
  behaviour is unchanged and V2 does not inherit a mechanism it has not decided yet.

## Tasks

| ID | Deliverable | Status | Dependencies | ADRs/specs |
|----|-------------|--------|--------------|------------|
| P02-T001 | The four Phase 2 decisions drafted, reviewed, and put in the status their gate requires | **Complete** — ADR-0002, ADR-0004 and ADR-0005 `Accepted`, the ADR-0033 note recorded. ADR-0004 is accepted **as a redraft under its own rule C**, its rule B having been run as written and failed | None | ADR-0002, ADR-0004, ADR-0005, ADR-0033, EVD-0008, EVD-0009 |
| P02-T002 | Graph validation with path-local diagnostics: existence, direction, domain, layout, fan-in policy, cycles, required output | **Complete** | ADR-0002, ADR-0033 | Master plan Phase 2 work list |
| P02-T003 | Lowering: stable identities to compact numeric slots, a topological schedule, and the implicit fan-in, conversion, and output operations | **Complete** | P02-T002 | ADR-0002 |
| P02-T004 | The preallocated buffer arena, liveness-based reuse, and in-place processing only where a node declares it safe | **Complete** | P02-T003 | ADR-0005 |
| P02-T005 | Prepared/mutable node separation and the node vocabulary the path needs | **Complete** | P02-T003 | ADR-0004, EVD-0009 |
| P02-T006 | ~~Kernel extraction into `synth_dsp`~~ — closed as not-happening, with the dropped extraction recorded as a deviation | **Closes with ADR-0040's acceptance**, whose clause 5 leaves nothing of this task inside the phase. The decision is made; the record is one review away | P02-T005 | ADR-0040 |
| P02-T007 | A monophonic note rendered through the complete path | **Blocked on P02-T013**, not on the decision any more: every node kind added before the arena is converted is a kernel to rewrite during it | P02-T004, P02-T005, **P02-T013** | ADR-0001 clauses 13 and 14 |
| P02-T012 | The internal channel layout re-measured against the **real** voice path, as the input to ADR-0041 | **Complete** — [EVD-0010](../evidence/phase-02/EVD-0010-internal-channel-layout-real-path.md). Interleaved cheaper in all nine runs of all three shapes: **2.54%** on the plan the crate compiles today, **21.56%** on a stereo chain, **11.05%** with independent per-channel control | P02-T005 | ADR-0002 revisit conditions, ADR-0040 clause 7, EVD-0008 |
| P02-T013 | The arena, the compiler, the kernels and the report converted to the **interleaved** layout, verified by a bit-identical render against digests taken from the planar build | **Next**, once ADR-0041 is accepted | P02-T012, ADR-0041 accepted | ADR-0041 clauses 1-17; ADR-0005 clauses 3, 6 and 9 |
| P02-T010 | The binding `Q` re-measurement; ADR-0037 confirmed or superseded | Not started | P02-T007 | ADR-0037, EVD-0002 |
| P02-T008 | Musical equivalence to V1, or a documented intentional difference, as an `EVD` record. ADR-0002's owed re-measurement moved to **P02-T012**, which does not depend on T007 and therefore runs before more nodes make the answer dearer | Not started | P02-T007, **P02-T010** | Corpus manifest claim classes; ADR-0001 clause 17; ADR-0002 revisit |
| P02-T009 | CPU against V1 for the equivalent minimal patch, as an `EVD` record | Not started | P02-T007, **P02-T010** | Master plan Phase 2 exit gate |
| P02-T011 | Formal exit review `REV-P02` | Not started | All applicable tasks | Working agreement, review protocol |

**P02-T013 runs before P02-T007, and P02-T010 before the two evidence tasks.** The first is ADR-0041 clause 17: a
node kind added before the arena is converted is a kernel to rewrite during the conversion.
The second is unchanged, and P02-T009 now carries an extra reason to sit behind the conversion — it compares CPU
against V1 for *whichever arena the engine has*, so running it before P02-T013 would measure the layout being
replaced.

**Why P02-T010 runs before the two evidence tasks, restated.** ADR-0001 clause 17 makes a render digest comparable
only within one quantum value, and both the equivalence and the CPU comparison are measured at whatever `Q` is. A
re-measurement that supersedes 64 therefore invalidates any equivalence or CPU evidence collected before it, and the
phase-gate rule requires evidence for the revision it evaluates. If `Q` changes after either record is written, that
record is rerun rather than reinterpreted.

## Decided, pending one review

**Both decisions are made; both records are one review pass from `Accepted`.** The user decided on 2026-08-18, with
[EVD-0010](../evidence/phase-02/EVD-0010-internal-channel-layout-real-path.md) in front of the choice: **V2 owns its
DSP** ([ADR-0040](../decisions/ADR-0040-v2-owns-its-dsp.md) option B), and **the internal arena becomes interleaved**
([ADR-0041](../decisions/ADR-0041-interleaved-internal-channel-layout.md), superseding ADR-0002 in full and ADR-0005's
clauses 1, 2, 4, 5, 7 and 8).

Neither record is `Accepted` yet, and the reason is the working agreement's own rule: a `Contract`-class ADR is not
accepted in the session that drafts it, and needs a reader who did not author it. ADR-0041's *Status* section lists
the four edits that move together when it is — including ADR-0002 to `Superseded`, its text untouched, and ADR-0040 to
`Accepted`, which its clause 7 has been waiting for.

Until then ADR-0002 remains the standing authority, so **P02-T006 and P02-T007 still do not proceed** — but the reason
has changed. T006 closes the moment ADR-0040 is accepted. T007 now waits on **P02-T013**, the conversion itself, for
the same reason it waited on the decision: every node kind added before the arena changes is a kernel to rewrite
during it.

## Active task

**P02-T013 — the arena converted to the interleaved layout ADR-0041 decides.** It is the phase's next piece of work,
and it starts the moment ADR-0041 is accepted.

What the decision rests on. [EVD-0010](../evidence/phase-02/EVD-0010-internal-channel-layout-real-path.md) measured
the layout against the crate's own kernels over the crate's own arena, in three shapes, with six review-found
asymmetries closed before any data was collected and the arms required to render bit-identical carries over every
quantum of a 200-quantum settle:

- **On the plan a stereo profile compiles today**, interleaved is cheaper by a median **2.54%** — 19.6 ns of 720. The
  chain is mono until its last two operations, so the whole difference is a widening copy plus two strided writes
  against a widening plus a memcpy. That is **a quarter of what EVD-0008 predicted**.
- **On a stereo chain**, by **21.56%**, because planar runs the filter and the amplifier once per channel.
- **With a control signal per channel**, by **11.05%**: half of the stereo-chain margin was the interleaved
  amplifier's single control read per frame, which EVD-0008 named as its own unmeasured caveat.
- **Shapes B and C are projections** — no node kind produces stereo yet, so the compiler cannot build either. What
  they establish is the shape of the cost curve, not a figure the phase pays today. ADR-0002's *first* revisit
  condition, a margin materially larger than 9% on the real path, is therefore **not** met; what reopened the record
  is its second condition, which ADR-0040 triggers.

What the conversion touches, read rather than assumed. Three places encode a **uniform** slot width and all three
change: `bind` resolves a slot as `slot.index() * quantum`
([`node/kernels.rs:403`](../../../crates/synth_engine_v2/src/node/kernels.rs)), the renderer's output operation does
the same ([`render/hot.rs:278`](../../../crates/synth_engine_v2/src/render/hot.rs)), and the arena is allocated as
`buffer_count * quantum` ([`render.rs:352`](../../../crates/synth_engine_v2/src/render.rs)). Beyond those: the
liveness unit becomes one signal rather than one channel, the widening writes one wider buffer instead of a second
one, the per-channel output operations collapse into one copy, every stateful kernel becomes channel-aware, and every
kernel needs a test at every channel count its own ports admit, under ADR-0041 clause 12.

**The acceptance check is a bit-identical render**, ADR-0041 clause 16: the arithmetic is not changing, only where the
samples live, so **five named fixtures** must render the same bytes across the conversion, against per-quantum digests
committed from the planar build in the commit immediately before it. The clause specifies those fixtures as compilable
graphs rather than describing them, because "every plan the suite compiles" is not a check anyone can run. EVD-0010 already demonstrated that both layouts can render one signal bit
for bit, so this is a check the phase knows it can pass rather than a hope.

**Two debts this measurement leaves, both recorded where they belong rather than here.** The renderer's own
output-carry loop costs **45.8 ns per quantum** more than a frame-strided form — more than twice the layout margin on
the path it renders — and the conversion is the natural moment to fix it, since that loop is being rewritten anyway.
And the largest unmeasured quantity this decision moves is **Phase 5's `LegacyPolyModuleAdapter`**, which must convert
at every V1 module port in both directions; ADR-0041's follow-up table carries the measurement and its revisit
conditions carry the consequence.

T002 through T005 are complete, so the
compiler is a pipeline and the renderer executes nodes: validate against a declared port table, lower to compact slots
with the implicit operations scheduled, assign the arena by liveness, and dispatch each node through the function
pointer admission resolved for it.

What P02-T005 left the next task:

- a **node registry** — [`node.rs`](../../../crates/synth_engine_v2/src/node.rs) — holding everything a kind declares:
  its ports, its controls, its prepared data, its kernel, and whether it may run in place. Adding a kind touches it,
  the IR vocabulary, a kernel and a test, and nothing else;
- **kernels** — [`node/kernels.rs`](../../../crates/synth_engine_v2/src/node/kernels.rs) — with one signature carrying
  prepared data, mutable state and slots, never `&self`, and a binding that resolves an input to *three* states rather
  than two: unpatched, in place, and patched;
- a **vocabulary** for the vertical path: an envelope, the existing sine, a state-variable low-pass, and an amplifier
  driven by a control input. Whether any of them is shared with V1 is what ADR-0040 is deciding;
- a **purity region** that now covers the loop *and* the kernels, with the registry checked to be closed — which is
  ADR-0004 clause 4's transitive requirement made executable.

| Decision | State |
|----------|-------|
| [ADR-0002](../decisions/ADR-0002-internal-channel-layout.md) | **`Accepted`, and superseded in full by [ADR-0041](../decisions/ADR-0041-interleaved-internal-channel-layout.md) on that record's acceptance.** Planar arena, accepted on the shared-kernel argument **against** its own measurement — and that argument is what ADR-0040 withdraws. It remains the standing authority until ADR-0041 is accepted; its text is left exactly as accepted |
| [ADR-0041](../decisions/ADR-0041-interleaved-internal-channel-layout.md) | **`Proposed`, decided.** The arena becomes interleaved: one signal is one buffer of `Q` frames of `c` channels, a mono signal is byte-for-byte what it is today, a kernel is told its channel count, and the host boundary becomes a copy. Supersedes ADR-0002 in full and ADR-0005 clauses 1, 2, 4, 5, 7 and 8, and withdraws ADR-0005's optimality argument, which assumed equal-sized slots. User decision, 2026-08-18, on [EVD-0010](../evidence/phase-02/EVD-0010-internal-channel-layout-real-path.md) |
| [ADR-0040](../decisions/ADR-0040-v2-owns-its-dsp.md) | **`Proposed`, decided.** V2 owns the DSP it renders; `synth_dsp` stays V1's. Held at `Proposed` by its own clause 7 until ADR-0041 is accepted |
| [ADR-0005](../decisions/ADR-0005-buffer-liveness-strategy.md) | **`Accepted`**. Conservative linear-scan liveness, in-place as a merged value chain, and two checks that make an aliasing bug fail loudly — including a bit-identical reuse-off comparison |
| [ADR-0004](../decisions/ADR-0004-native-node-representation.md) | **`Accepted`, as a redraft under its own rule C.** Rule B was run as written and failed, at +7.31% and +7.47% in two nine-run sets against a 3% threshold ([EVD-0009](../evidence/phase-02/EVD-0009-dispatch-cost.md)). **No dispatch shape passes it**: the hybrid rule C names was built and measured at +7.08% and +7.34% — the same measurement as the table — and a closed enum for every node at +5.39% and +5.10%. The price of the choice itself is measured directly: **at least +2.12% and +2.09%** for the table against that closed enum over the same arena, agreeing across two sets to three hundredths of a point and stated as a lower bound because the enum arm walks the table's schedule rather than a natively enum one. Option C is kept for what it does not close: the node set at the crate boundary, which Phase 5's declarative API needs open. User decision, 2026-08-18 |
| ADR-0033 | `Proposed`, with the register note that Phase 2 refuses every cycle and may not assume one can be broken |

- **Non-goals.** Any decision a later phase owns: ramp representation (ADR-0006, Phase 3/5), modulation laws
  (ADR-0007, Phase 5), event segmentation (ADR-0003, Phase 3), and the feedback rule itself (ADR-0033, Phase 3).

## Deliverables and verification

| Task | Output/revision | Verification/evidence | Result |
|------|-----------------|-----------------------|--------|
| P02-T004 | `synth_engine_v2::arena` (private), with `compile` restructured so the report states the arena it assigns | `src/tests/arena_reuse.rs` — ADR-0005 clause 8's two checks over every plan shape this phase can build: no two overlapping lives share a slot, and reuse renders **bit-identically** to `NoReuse`. Plus a four-gain chain collapsing to one buffer, in-place declined where the input is read again, and two compilations assigning identically | Complete |
| P02-T003 | `ParameterSlot` and `PlanId` in `plan`, per-channel output and duplication operations, `ChannelIndex` | `tests/lowering.rs` — 8 cases: an address resolving to a slot and an unknown one to `None`, distinct slots per address, a foreign plan's slot refused by identity *and* counted, one output operation per channel with the widening as its own scheduled operation and its own buffer, dense buffer slots, and the scratch budget covering the buffer a stereo plan actually allocates | Complete |
| P02-T002 | `synth_engine_v2::validate`, wired into `compile` between admission and lowering | `tests/graph_validation.rs` — 15 cases: one refusal per rule asserting the fields its class requires, the unreached-output warning, the mono-to-stereo conversion rendered on both channels, a gain rendered through the schedule, identical bytes from two declaration orders, and a 4 096-node chain that a quadratic compiler would take minutes to admit | Complete |
| P02-T005 | `synth_engine_v2::node` and `node::kernels`, with `PlanOp` reduced to a node step and the renderer's own output boundary | `tests/node_representation.rs` — rendering a plan leaves its prepared data unchanged, two streams over one plan keep separate states, a compiled slot moves one node's control and not another's, a merged gain scales rather than silences, and a widening is an ordinary scheduled kernel. `tests/voice_nodes.rs` and `src/tests/kernels.rs` — the filter's unity DC gain, its three preparation refusals, its silence after ringing; the envelope's exact segment durations, its release from any level, its edge-triggered gate and its boundary level; the amplifier's two domains. `tests/render_loop_purity.rs` — the region now covers the kernels, and the registry is checked to be closed | Complete |
| P02-T012 | [EVD-0010](../evidence/phase-02/EVD-0010-internal-channel-layout-real-path.md) and its harness, `examples/evd_0010_layout_cost.rs` | Nine runs of three shapes, each layout arm against a null control in its own instruction mix, groups rotated, every ratio and control spread paired within a round. **Six asymmetries closed before collection**, five of them favouring interleaved — an indirect call per planar node, a bespoke interleaved kernel contract, an extra arena slot per interleaved arm, a function pointer selecting the carry write, a region helper that was a call of its own, and discriminant checks only interleaved paid. Criterion B compares raw bit patterns at **every quantum** of the settle, not one settled block, and shape A's arm is asserted against the compiler's operations and the real renderer's carry | Complete |
| P02-T001 | [ADR-0002](../decisions/ADR-0002-internal-channel-layout.md), [ADR-0004](../decisions/ADR-0004-native-node-representation.md), [ADR-0005](../decisions/ADR-0005-buffer-liveness-strategy.md); an ADR-0033 note in the register | [EVD-0008](../evidence/phase-02/EVD-0008-internal-channel-layout-cost.md) for ADR-0002 — nine runs, rule fixed before the data, five recorded corrections, and a conclusion that went against the preferred option. [EVD-0009](../evidence/phase-02/EVD-0009-dispatch-cost.md) for ADR-0004 — twelve arms including four control pairs rotated in groups, ratios paired within a round, an agreement check over every arm's output, nine retained corrections, and a rule that failed as written. Register agreement; six independent review passes | Complete |

## Exit gate mapping

The master plan's six gate bullets, each against the check that will decide it. A gate with no executable check is not
a gate; this table is filled in as the checks are written, and no bullet may close on prose.

| Gate | Deciding check | State |
|------|----------------|-------|
| The hot path contains no port strings, `HashMap` lookups, graph traversal, topology decisions, or buffer resizing | `tests/render_loop_purity.rs`, now seven checks. The Phase 2 constructs are banned by name; every call in the file must match something defined there or a justified allowlist entry; no lowercase name may be imported, so a free function cannot be brought in to be called; and the parameter-route scan Phase 1 left behind is gone — an event carries a compiled slot. **The bound is stated in the test rather than implied**: a name scan cannot resolve receivers, so an escape would have to be a method named like an accessor on a type the file already holds | **Passes** for the constructs; the receiver-resolution gap is recorded, not closed |
| The graph compiler reports a useful path-local diagnostic for an invalid cable and for a missing output path | One refusal case per validation rule, each asserting the fields **its own failure class** requires. Endpoints belong to cable errors only: a domain, direction, or layout mismatch names the edge, both endpoints, and both mismatched values; a fan-in violation names the destination port and the competing edges; a cycle names the edge that closes it and the node it re-enters; a missing output names how many sources it stranded. `HOST-INV-007`'s admission shape — a field, two amounts, an authored object — is a *resource* refusal and is not this | **Passes**, with one stated gap: the layout rule's *refusing* direction has no constructible case while every node kind in this phase produces mono, and `the_layout_refusal_has_no_constructible_case_in_this_phase` asserts that premise so it fails the day a stereo output arrives |
| The basic voice render is musically equivalent to V1, or the difference is documented and intentional | P02-T008's `EVD` record, over `pertylizer compare` against the corpus case the minimal patch corresponds to | Open |
| Adding a second simple DSP node does not require changing renderer control flow | ADR-0004 clause 1 defines control flow as the code in `render/hot.rs`, and clause 2 makes the bullet a **diff**. The amplifier is that diff, at `f0750c22`: it touches the IR vocabulary, the registry, a kernel and a test, and no line of `render/hot.rs`, the compiler, the validator, the arena or the report. A structural search puts every node-kind reference outside the registry in the IR and in three `IrNodeKind::Output` arms, none of which changes when a kind is added | **Passes**, by a change rather than by an argument |
| CPU use is no worse than V1 for the equivalent minimal patch, allowing a temporary documented margin for adapters | P02-T009's `EVD` record, with the control run first and the estimator, draw count, build profile, and binary matched across the two arms — [the rule EVD-0004 was written to stop repeating](../evidence/README.md#state-the-falsifier-before-measuring-and-run-its-control-first) | Open |
| The render quantum is re-measured against real V2 nodes, and ADR-0037 is confirmed or superseded | P02-T010's `EVD` record, with its falsifier and acceptance rule stated **before** the measurement | Open |

## Deviations

Recorded rather than absorbed. The extraction path itself is a permitted choice within the master plan's work list
rather than a deviation, and it is recorded
[above](#recorded-execution-choice-extraction-rather-than-adapters).

1. **P02-T002 adds one node kind, `Gain`, ahead of the node work in P02-T005.** Without a node that has an *input*
   port, no plan can have a fan-in, a cycle, a chain, or a path through anything, so five of the seven validation
   rules would have had no constructible case and the gate would have closed on prose. It is added inside the existing
   `IrNodeKind` and `PlanOp` enums — the shape Phase 1 already has — so it makes no claim about the node
   representation [ADR-0004](../decisions/ADR-0004-native-node-representation.md) is still deciding.
2. **An output nothing reaches is a `CompileWarning`, not a refusal.** The gate asks for a diagnostic for "a missing
   output path", and the refusal case is the one that matters: sources with no output node at all. An output whose
   input is unpatched renders silence, which is a legitimate intermediate state — and once plans swap live, refusing
   it would mean the engine rejects a plan the moment a cable is unplugged. It warns instead, and the warning says the
   plan renders silence.
3. **P02-T004 reordered admission.** Phase 1 built the resource report first, because a refusal carries it. The arena's
   size is a function of the *assignment*, though, so a report built before lowering can only state an upper bound — and
   an upper bound refuses plans that fit. The order is now: a preflight report over that upper bound; **structural
   validation**, because an invalid cable is the actionable diagnostic and refusing a malformed graph on a limit would
   hide it; refusal on any admission-checked field *before* the arena row, so a graph the profile refuses outright never
   reaches lowering; then lowering, assignment, an exact report, and the remaining fields. Two refusal paths therefore
   return before an arena exists — a structural one and an early-limit one — and both carry a report whose arena row
   `ResourceReport::arena_is_estimated` marks as a bound. One admission fixture changed with the order: the fan-out case
   used an invalid cable to make its two edges, and was refused for the cable rather than the limit.
4. **The report's memory rows are measured from the representation rather than declared per kind.** ADR-0004's registry
   gives every node one prepared record and one state record whatever its kind, so the aggregate is a count times a
   size — which is what a plan actually allocates, and what `HOST-INV-014` asks the report to state. The per-kind
   figures survive as the *attribution*: every node costs the same, so the object responsible is the one whose payload
   is widest. Two consequences are recorded rather than absorbed. The output node is excluded, because it has no kernel
   and lowering builds it no record; and the compiler's own widening operations are counted, because ADR-0002 clause 7
   makes them scheduled operations with prepared data like any other node. Admission passes the exact count once
   lowering has produced it, and an upper bound before that, exactly as it does for the arena.

5. **P02-T005 widened the crate's public surface to the kernel interface.** `NodeIo`, the kernel functions, the control
   indices and `NodeStep`'s constructor are public. The reason is EVD-0009: rule B's comparison is only honest if both
   arms call *the same kernels over the same prepared records*, and a harness outside the crate cannot do that against
   a crate-private interface. A harness that reimplemented the kernels would have been measuring a model of them —
   which is the failure mode this repository has recorded before. The surface is also where Phase 5's declarative node
   API is heading, so it is not surface invented for a benchmark.

6. **`CompileError::UnsupportedOutputPort` is replaced by `UnknownPort` and `PortDirection`.** Phase 1 checked the one
   port its lowering read; Phase 2 checks every edge against the node's declared port table, so the specific variant
   became a special case of a general rule. A port that exists in the *other* direction gets its own variant, because
   "no such port" and "your cable is backwards" send a reader to different places.

## Exit readiness

Status: **Not ready**, and the list has changed shape rather than shortened. Two of the six gate bullets have deciding
checks that pass — the path-local diagnostics, with its one stated gap, and the node-addition bullet, decided by the
amplifier's diff rather than by an argument. **Both of those checks are re-run after P02-T013**: the node-addition
bullet is a diff against `render/hot.rs`, and the conversion rewrites that file's output operation, so the bullet is
re-decided against the converted renderer rather than inherited across it.

The decisions are no longer all in the status their gate requires, and that is the phase's first exit blocker rather
than a formality: **ADR-0041 and ADR-0040 are `Proposed`**, ADR-0002 stands only until ADR-0041 is accepted, ADR-0004
and ADR-0005 are `Accepted` — the latter with two clauses superseded — and ADR-0033 is `Proposed` with Phase 2
refusing every cycle.

The remaining four bullets wait on the conversion, then the rendered note, then the binding `Q` re-measurement, then
the two evidence tasks — in that order, because a render digest is comparable only within one quantum value, and
because P02-T009's CPU comparison against V1 is a comparison of whichever arena the engine has.

**One debt this phase now carries into P02-T009.** EVD-0009 measured the compiled path at about 865 ns per quantum
against 540 ns for one fused function — about a third less, a figure that is neither paired nor over the same
computation, since the fused arm skips the envelope's control work. It is an **upper bound on an opportunity** rather
than a measurement of what node boundaries cost, and this tracker carries it as such: it is the largest bound in the record, and sizing it needs a fused arm that runs the envelope. The arena binding measures 32.18 ns per
quantum, paired against a walk-only control that costs 0.8, and it is the largest identified item with an obvious remedy — it
has already come down from 47 ns by moving the binding decision to admission. The CPU gate is measured against V1
rather than against a fused ideal, so none of this is a gate failure; it is where the room is, and P02-T009 is the task
that will find out whether any of it is needed.

## Next actions

1. **Accept ADR-0041 and ADR-0040**, which is one independent review pass and the four status edits ADR-0041's
   *Status* section lists. Both decisions are made; nothing about them is open.
2. **Then P02-T013**, the conversion, summarised under *Active task* — and it is the moment to pay off the renderer's
   45.8 ns output-carry loop, since that loop is being rewritten anyway.
3. The other side finding from P02-T012 is already recorded where it belongs: `PreparedRenderer::prepare` primes its
   carry with a quantum of silence, so a first `render` call of exactly `Q` frames renders nothing and rejects any
   event presented on it — which had left
   [EVD-0009](../evidence/phase-02/EVD-0009-dispatch-cost.md)'s `renderer` arm rendering an ungated voice, now noted
   in that record.
4. Then T007, then **P02-T010 before P02-T008 and P02-T009** — a render digest is comparable only within one quantum
   value, so a re-measurement that supersedes 64 invalidates any equivalence or CPU record collected before it. T009's
   CPU comparison against V1 is now a comparison of a **different arena** than the one it was scoped against, which
   ADR-0041's follow-up table records.
5. Keep Phase 0B moving independently; it gates Phase 10, not this phase.
