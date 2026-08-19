# Core V2: Current Work

Last updated: 2026-08-19

This is the only authority for active Core V2 task state, blockers, and next actions. Durable reasoning and measurements
live in the linked ADRs, specs, and EVDs rather than being repeated here.

## Primary stream: Phase 2

Outcome: render one complete compiled monophonic voice through validated, scheduled V2 nodes and close Phase 2's
equivalence, CPU, and quantum gates.

### Active slice

**Execute P02-T010: re-measure `Q` against the real voice path.**

P02-T007 landed the note edge and the complete voice, so the crate now renders the thing the phase exists for: a note
event at its declared sample driving an oscillator, a filter, an envelope and an amplifier into the output. That is
also the plan every remaining Phase 2 measurement has to be taken over.

ADR-0037 fixed `Q` at 64 from a V1 proxy and requires a re-measurement against the real V2 renderer before Phase 2
exits. It runs now because ADR-0001 clause 17 makes a render digest comparable only within one quantum value: an
equivalence or CPU record collected at 64 is invalidated rather than reinterpreted if the re-measurement supersedes it.

The task is complete when the measurement's falsifier and acceptance rule are written before any data is collected, the
candidate quanta are compared on the voice path the crate now compiles, and ADR-0037 is either confirmed or superseded
by a record that states its method, its limitations and its conclusion.

Two things it inherits, both stated so they are not rediscovered:

- **The instrument has five known traps**, all found by review of EVD-0010 and EVD-0011 rather than by the data:
  per-call timing measures the clock rather than the renderer, the acceptance threshold must be chosen before the
  numbers are seen, a two-build A/B measures a net effect and cannot attribute it, the order of the arms must be
  counterbalanced against their identity, and the estimator (minimum over rounds, median over runs) is part of the
  method rather than a presentation choice.
- **The first `render` call renders no quantum.** It returns the carry `prepare` primed with `Q` frames of silence and
  refuses any event presented with it, so a harness opens its gate on the second call.

Blockers: none.

References:

- [ADR-0037: the render quantum's value](decisions/ADR-0037-render-quantum-value.md), whose Phase 2 re-measurement this
  is
- [ADR-0001: render quantum semantics](decisions/ADR-0001-internal-render-quantum.md), clause 17 for why this runs
  before the two evidence tasks
- [EVD-0011: the mono path after the conversion](evidence/phase-02/EVD-0011-mono-path-cost.md), whose method and
  reproduction recipe this measurement reuses
- [Current Sound Core render contract](specs/spec-sound-core-render-contract.md)
- [Historical Phase 2 execution record](phases/phase-02-minimal-compiled-voice-graph.md)

### Phase 2 task state

| Task              | State                            | Next dependency       |
|-------------------|----------------------------------|-----------------------|
| P02-T001–T005     | Complete                         | —                     |
| P02-T006          | Ready to close as not-happening  | ADR-0040 clause 5     |
| P02-T012          | Complete                         | EVD-0010              |
| P02-T013          | Complete                         | EVD-0011              |
| P02-T007          | Complete                         | `note_events`         |
| P02-T010          | **Active**                       | —                     |
| P02-T008/P02-T009 | Waiting                          | P02-T007 and P02-T010 |
| P02-T011          | Waiting                          | All Phase 2 outcomes  |

### Next actions

1. Re-measure `Q` in P02-T010 before equivalence and CPU evidence.
2. Close P02-T006 as not-happening and record the dropped extraction as a deviation, per ADR-0040 clause 5.
3. Close `render_loop_purity`'s provenance gap for SOUND-INV-013: it proves every *registered* kernel is
   defined in the checked region, but not that a descriptor's function pointer resolves inside it.
4. Audit the specification's pre-existing conformance rows, which predate this phase: an independent read
   found four that name a check not carrying the invariant — SOUND-INV-001/004, 003/012, 007 and 008. Not a
   defect in the code, and recorded in the specification's unresolved questions.
5. Decide where a note's **pitch and velocity** live. P02-T007 deliberately gave `NoteEdge` neither, because
   nothing in Phase 2 reads either; Phase 3's ingress is the first task that has to.
6. Complete P02-T008, P02-T009, and the Phase 2 exit review.

## Paused parallel stream: Phase 0B

Outcome: complete the V1 migration inventories and the durable Project and Application Core contracts required before
Phase 10.

This phase remains active in the roadmap, but its execution stream is paused
while the Phase 2 slice above is active. On resumption, select exactly one task,
copy its observable completion check here, and only then mark it `Active`.

| Task           | State       | Resume boundary                                                            |
|----------------|-------------|----------------------------------------------------------------------------|
| P00B-T001      | Paused      | Assign evidenced dispositions in the state-ownership inventory             |
| P00B-T002      | Paused      | Assign reachability and migration dispositions in the capability inventory |
| P00B-T003      | Paused      | Resolve the two format questions that block ADR-0014 review                |
| P00B-T004–T009 | Not started | Follow the decomposition in the frozen execution record                    |

This stream does not block Phase 2. Its detailed audit chronology remains in
the [historical Phase 0B execution record](phases/phase-00b-inventories-and-project-contracts.md); new operational state
is recorded only here.

Phase lifecycle and completed gates are recorded once in
[`ROADMAP.md`](ROADMAP.md#phase-order).

## Later owned work

- Phase 3 owns renderer ingress, deferred-event storage, event scheduling, and the pending ADR-0001 clarification.
- Phase 4 owns current-project lowering and the long-running job contract.
- Phase 0B gates Phase 10.
- ADR-0039 and `LIMIT-0017` remain Phase 10E work.
