# Core V2: Current Work

Last updated: 2026-08-19

This is the only authority for active Core V2 task state, blockers, and next actions. Durable reasoning and measurements
live in the linked ADRs, specs, and EVDs rather than being repeated here.

## Primary stream: Phase 2

Outcome: render one complete compiled monophonic voice through validated, scheduled V2 nodes and close Phase 2's
equivalence, CPU, and quantum gates.

### Active slice

**Execute P02-T013: convert the arena, compiler, kernels, and resource report to the interleaved layout.**

ADR-0040 and ADR-0041 were accepted on 2026-08-19, so the interleaved region contract is what the
[current Sound Core specification](specs/spec-sound-core-render-contract.md) states and the crate is the part that has
not caught up.

The task is ordered by its own check. **First**, on the planar build, commit clause 16's five fixtures and one
baseline file per fixture — 256 per-quantum digests each. **Then** convert, in one task, before another node kind is
added.

It is complete when all of these hold. The digest comparison is behavioural and establishes none of the structural
ones, so each is listed rather than assumed:

- every fixture's per-quantum digests match the committed baselines;
- `arena_reuse`'s structural check compares **physical sample ranges** rather than slot identity, and gains an
  observation-tap case;
- `lowering` asserts one wider region and one contiguous output copy in place of today's assertion of one output
  operation per channel;
- every kernel has a test at **every** channel count its own ports admit, and each mono-only kernel asserts its port
  table, over the whole catalog rather than a subset;
- `graph_validation`'s layout premise enumerates every node kind rather than the hand-written subset it lists today;
- the mono path is re-measured against its pre-conversion figure under EVD-0010's discipline;
- no new allocation or blocking operation enters the render path.

Blockers: none.

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
| P02-T006          | Ready to close as not-happening  | ADR-0040 clause 5     |
| P02-T012          | Complete                         | EVD-0010              |
| P02-T013          | **Active**                       | —                     |
| P02-T007          | Waiting                          | P02-T013              |
| P02-T010          | Waiting                          | P02-T007              |
| P02-T008/P02-T009 | Waiting                          | P02-T007 and P02-T010 |
| P02-T011          | Waiting                          | All Phase 2 outcomes  |

### Next actions

1. Commit clause 16's fixtures and their planar baselines, then implement P02-T013.
2. Close P02-T006 as not-happening and record the dropped extraction as a deviation, per ADR-0040 clause 5.
3. Render the complete voice in P02-T007.
4. Re-measure `Q` in P02-T010 before equivalence and CPU evidence.
5. Close `render_loop_purity`'s provenance gap for SOUND-INV-013: it proves every *registered* kernel is
   defined in the checked region, but not that a descriptor's function pointer resolves inside it.
6. Audit the specification's pre-existing conformance rows, which predate this phase: an independent read
   found four that name a check not carrying the invariant — SOUND-INV-001/004, 003/012, 007 and 008. Out of
   scope for the layout acceptance, and not a defect in the code.
7. Complete P02-T008, P02-T009, and the Phase 2 exit review.

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
