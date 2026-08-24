# Core V2: Current Work

Last updated: 2026-08-24

This is the only authority for active Core V2 task state, blockers, and next actions. Durable reasoning and measurements
live in the linked ADRs, specs, and EVDs rather than being repeated here.

## Phase 2 is closed

**[`REV-P02`](reviews/phase-02-exit-review.md) is `Accepted`**, so Phase 2's state in
[`ROADMAP.md`](ROADMAP.md#phase-order) is `Complete` and every P02 task is closed. All six master-plan gate bullets
close. The review carries the gate table and the figures; only the consequences are stated here.

- `Q` = 64 is **confirmed** ([EVD-0012](evidence/phase-02/EVD-0012-render-quantum-real-path.md)), so the restriction on
  hand-unrolled kernels, `Q`-specific buffer layouts and tests asserting a control rate in hertz **is discharged**.
- Equivalence takes the gate's **second branch** — a documented intentional difference. **No `CORPUS-0001` preserve
  claim is broken** ([EVD-0013](evidence/phase-02/EVD-0013-minimal-patch-equivalence.md)); the envelope shape is
  `CORPUS-0001-C2` under [ADR-0042](decisions/ADR-0042-envelope-segment-shape.md).
- CPU closes with **no adapter margin claimed**
  ([EVD-0014](evidence/phase-02/EVD-0014-minimal-patch-cpu.md)).
- P02-T006's dropped `synth_dsp` extraction is **registered** in the review's deviation table, with ADR-0040 clause 5 as
  its acceptance basis. That debt is paid.

The review's *Deviations and residual risks* table is the register for everything Phase 2 leaves behind; it is not
repeated here. Two items are owed to later work rather than to nobody, and both are named there: **a note's pitch and
velocity** (Phase 3's ingress is the first consumer — `NoteEdge` carries neither today), and **`SOUND-INV-012`'s second
sentence**, the one invariant with no executable check.

## Next stream: Phase 3 is not yet activatable

Phase 3's state stays `Not started`. It is **not** blocked on Phase 2, which is closed; it is blocked on decisions that
must be `Accepted` **before implementation begins**. Of the original three, one is closed and two are not; ADR-0044's
survey opened a fourth on 2026-08-24, so **three remain open**.

| Prerequisite | Current status | What it settles, or must settle |
|---|---|---|
| An **ADR-0001 clarification or successor** | **Closed.** [ADR-0043](decisions/ADR-0043-event-deferral-and-late-clamp.md) is `Accepted` (2026-08-20) | Option D with a preserving clamp: an event is assigned to the quantum containing its **render position**, deferral advances that position by `+Q` with the offset preserved and the stamp untouched, clause 16's late condition is asked **once** when an event first becomes due, and a control-rate response begins at the first quantum boundary at or after the render position. Supersedes ADR-0001 clauses 12, 14 and 16 and ADR-0032 clause 16; clause 12's second sentence stands |
| [ADR-0044](decisions/ADR-0044-deferral-causal-order.md) — deferral-induced causal order | **`Deferred`** to the Phase 3 entry gate. **Surveyed** 2026-08-24 over two independent reads; it narrows to same-control order and selects **no candidate** | ADR-0043's named correctness hole. `+Q` moves an event and not the events depending on it, so at `Q` = 64 a note-on stamped at 63 defers to 127 while its note-off at 65 renders first and strands the voice. `ADR-0023` cannot repair it and neither can ordering admission by stamp. **Until it is `Accepted`, no phase may implement deferral at all** |
| [ADR-0045](decisions/ADR-0045-cross-control-causal-order.md) — cross-control causal order | **`Deferred`** to the Phase 3 entry gate. **Opened 2026-08-24** by ADR-0044's survey | The half of the causal-order hazard no same-control mechanism reaches. A control-rate automation at 63 takes effect at boundary 64, before a sample-rate note at 65; deferred to 127 it takes effect at 128, after it. It gates the phase because `ROADMAP.md`'s Phase 3 *Outcome* names automation ordering and because the symptom **persists** — the sine kernel's phase accumulator carries the error after the controls converge. Its candidate space is empty: ADR-0044's F2 frame refuses an audio-thread dependency-graph walk |
| [ADR-0022](decisions/ADR-0022-hardware-time-mapping.md) — hardware time mapping and latency ownership | **`Deferred`** to the Phase 3 entry gate | Unchanged and untouched by ADR-0043. It needs a simulated-host harness with controllable timestamps, drift, block sizes and disconnects, plus per-callback measurements on the three release platforms — and that harness is Phase 3's own work item, so it sits on the critical path rather than beside it |

**What ADR-0043's acceptance changed outside the decision index.** `HOST-INV-021` is **split**: its timing rule is now
normative in the [host-profile specification](specs/spec-host-profile-and-render-limits.md), while the ingress
capacities, the deferred store's bound and exhaustion policy, the admission order and the starvation it permits stay
`Deferred to Phase 3`. In the [Sound Core render contract](specs/spec-sound-core-render-contract.md), `SOUND-INV-016`
is **restated** over the render position — which also repairs a defect the selection exposed, since a late but
never-deferred event does not take effect at its declared sample and the invariant as written was false for it — while
`SOUND-INV-006` changes only in naming the render position as the input its derivation was always meant to take; its
promise that an event retains its `StreamEpoch` and absolute `SampleTime` is what the preserving clamp makes true. Two
unresolved questions are answered and struck. The clamp half of the selection is what the renderer already does, so
**no code changed**.

**Activating Phase 3 is the user's call, and the survey made the gate wider rather than narrower.** ADR-0044's option
survey ran on 2026-08-24 with two independent reads. **It selects no candidate.** Frame F1 asks a repair to hold for
every pair whose meaning depends on order; each candidate is scoped to a proper subset, so none passed, and the
mechanism the narrowing left standing — a bounded same-control **run** translation — failed the second read on four
counts: its scan is bounded only by the ingress and deferred-store capacities Phase 3 has not chosen, a re-deferred
tail event stretches the interval it was chosen to preserve, the run is **partition-dependent** across callbacks, and
its key misses control-rate parameters because `timed_target` filters `SetParameter` to `ControlRate::Sample`.

What the survey did settle is durable: 1a, 1c, 2b, the conditional counted fault and the voice-allocator latch are
eliminated on their merits, and the two claims this stream had been carrying — that ADR-0044 did not depend on the
deferred store's shape, and that a latch also covered ADR-0021's queue drop — are both **false**. The next step is not
a selection. It is the deferred store's capacities decided first so the run form can be costed, or a mechanism nobody
has proposed, **or a successor to ADR-0043 reopening its Option A** — no deferral, capacity overrun is a fault. That
last one dissolves ADR-0044 and ADR-0045 together, because both hazards are deferral-induced, and it is the path the
survey's result makes more attractive than it was on 2026-08-20: D was selected when the causal-order cost was named
but not measured, and this survey is that measurement.

**The scope split stands; the "no gate" half of it did not.** ADR-0044 narrows to same-control order and
[ADR-0045](decisions/ADR-0045-cross-control-causal-order.md) takes cross-control order — but ADR-0045 **is** a Phase 3
entry prerequisite, because `ROADMAP.md`'s Phase 3 *Outcome* names automation ordering and the symptom persists: the
sine kernel's phase accumulator carries a delayed frequency automation's error permanently. **Phase 3's entry gate now
has three conjuncts** — ADR-0044, ADR-0045 and ADR-0022 — where it had two. ADR-0022 is still the one needing a
simulated-host harness built and measured before it can be written at all. Phase 0B's paused stream below is the third
candidate slice; it gates Phase 10 and nothing in Phase 3 depends on it.

## Paused parallel stream: Phase 0B

Outcome: complete the V1 migration inventories and the durable Project and Application Core contracts required before
Phase 10.

This phase remains active in the roadmap, and its execution stream is paused.
It was paused for the Phase 2 slice, which has now closed, so **nothing is
holding it any longer** — resuming it is a choice rather than a wait. On
resumption, select exactly one task, copy its observable completion check here,
and only then mark it `Active`.

| Task           | State       | Resume boundary                                                            |
|----------------|-------------|----------------------------------------------------------------------------|
| P00B-T001      | Paused      | Assign evidenced dispositions in the state-ownership inventory             |
| P00B-T002      | Paused      | Assign reachability and migration dispositions in the capability inventory |
| P00B-T003      | Paused      | Resolve the two format questions that block ADR-0014 review                |
| P00B-T004–T007, P00B-T009 | Not started | Follow the decomposition in the frozen execution record       |
| P00B-T008      | Not started | Re-scope the frozen all-ADR task under `PROCESS.md`'s decision-timing rule  |

This stream does not block Phase 2. Its detailed audit chronology remains in
the [historical Phase 0B execution record](phases/phase-00b-inventories-and-project-contracts.md); new operational state
is recorded only here.

Phase lifecycle and completed gates are recorded once in
[`ROADMAP.md`](ROADMAP.md#phase-order).

## Later owned work

- Phase 3 owns renderer ingress, deferred-event storage, event scheduling, the causal-order policies ADR-0044 and
  ADR-0045 defer, and ADR-0022's simulated-host harness.
- Phase 4 owns current-project lowering and the long-running job contract.
- Phase 5 owns the `LegacyPolyModuleAdapter`'s conversion cost — the largest quantity ADR-0041 moves and the only one
  nobody has measured — and the declarative node API that `SOUND-INV-012`'s uncovered second sentence belongs to.
- Phase 0B gates Phase 10.
- ADR-0039 and `LIMIT-0017` remain Phase 10E work.
