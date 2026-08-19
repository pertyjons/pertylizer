# Core V2: Current Work

Last updated: 2026-08-19

This is the only authority for active Core V2 task state, blockers, and next actions. Durable reasoning and measurements
live in the linked ADRs, specs, and EVDs rather than being repeated here.

## Primary stream: Phase 2

Outcome: render one complete compiled monophonic voice through validated, scheduled V2 nodes and close Phase 2's
equivalence, CPU, and quantum gates.

### Active slice

**Close ADR-0040 and ADR-0041, then execute P02-T013.**

The user has selected V2-owned DSP and an interleaved internal arena. The two records remain `Proposed` until one
focused independent review reaches the semantic stopping rule in [`PROCESS.md`](PROCESS.md#review-stopping-rule).

After acceptance, P02-T013 converts the arena, compiler, kernels, and resource report. It is complete when ADR-0041's
named planar baselines compare bit-identically per quantum, every layout-sensitive kernel test passes, and no new
allocation or blocking operation enters the render path.

Blockers:

- ADR-0040 and ADR-0041 are not yet `Accepted`.
- ADR-0002 therefore remains the current layout decision.

References:

- [ADR-0040: V2 owns its DSP](decisions/ADR-0040-v2-owns-its-dsp.md)
- [ADR-0041: interleaved internal channel layout](decisions/ADR-0041-interleaved-internal-channel-layout.md)
- [EVD-0010: real-path layout measurement](evidence/phase-02/EVD-0010-internal-channel-layout-real-path.md)
- [Current Sound Core render contract](specs/spec-sound-core-render-contract.md)
- [Historical Phase 2 execution record](phases/phase-02-minimal-compiled-voice-graph.md)

### Phase 2 task state

| Task              | State                            | Next dependency       |
|-------------------|----------------------------------|-----------------------|
| P02-T001–T005     | Complete                         | —                     |
| P02-T006          | Closes when ADR-0040 is accepted | ADR-0040/0041 review  |
| P02-T012          | Complete                         | EVD-0010              |
| P02-T013          | Next                             | ADR-0041 accepted     |
| P02-T007          | Waiting                          | P02-T013              |
| P02-T010          | Waiting                          | P02-T007              |
| P02-T008/P02-T009 | Waiting                          | P02-T007 and P02-T010 |
| P02-T011          | Waiting                          | All Phase 2 outcomes  |

### Next actions

1. Complete the focused independent review of ADR-0040 and ADR-0041.
2. Accept both records as one decision update and advance this file.
3. Implement P02-T013.
4. Render the complete voice in P02-T007.
5. Re-measure `Q` in P02-T010 before equivalence and CPU evidence.
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
