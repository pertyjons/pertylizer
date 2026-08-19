# Core V2: Current Work

Last updated: 2026-08-19

This is the only authority for active Core V2 task state, blockers, and next actions. Durable reasoning and measurements
live in the linked ADRs, specs, and EVDs rather than being repeated here.

## Primary stream: Phase 2

Outcome: render one complete compiled monophonic voice through validated, scheduled V2 nodes and close Phase 2's
equivalence, CPU, and quantum gates.

### Active slice

**Execute P02-T007: render the complete voice through the compiled path.**

P02-T013 landed the interleaved arena, so the crate now renders through the contract the
[current Sound Core specification](specs/spec-sound-core-render-contract.md) states. What it does not yet do is the
thing the phase exists for: a note event arriving at its declared sample and the whole voice rendering from it.

The task is complete when a note edge takes effect at the sample it names rather than at the quantum boundary that
follows it, the voice renders end to end from that edge, and the specification can drop the unresolved question that
says P02-T007 owes the note-event payload, sample-offset behaviour, and a named conformance check.

Two things it inherits, both stated so they are not rediscovered:

- **The gate is a control today.** ADR-0001 clause 13 governs when a node observes an event, and clause 14 puts a note
  edge at its declared sample; the envelope's gate currently arrives at a quantum boundary like any other control, and
  closing that is this task's subject rather than a detail of it.
- **The first `render` call renders no quantum.** It returns the carry `prepare` primed with `Q` frames of silence and
  refuses any event presented with it. Every fixture and baseline in the crate is written around that, and a note
  scheduled onto that call fails rather than sounding.

Blockers: none.

References:

- [Current Sound Core render contract](specs/spec-sound-core-render-contract.md), whose unresolved questions name what
  this task owes
- [ADR-0001: render quantum semantics](decisions/ADR-0001-internal-render-quantum.md), clauses 13 and 14
- [ADR-0041: interleaved internal channel layout](decisions/ADR-0041-interleaved-internal-channel-layout.md), clause 16
  for the baseline comparison every later change now renders against
- [EVD-0011: the mono path after the conversion](evidence/phase-02/EVD-0011-mono-path-cost.md)
- [Historical Phase 2 execution record](phases/phase-02-minimal-compiled-voice-graph.md)

### Phase 2 task state

| Task              | State                            | Next dependency       |
|-------------------|----------------------------------|-----------------------|
| P02-T001–T005     | Complete                         | —                     |
| P02-T006          | Ready to close as not-happening  | ADR-0040 clause 5     |
| P02-T012          | Complete                         | EVD-0010              |
| P02-T013          | Complete                         | EVD-0011              |
| P02-T007          | **Active**                       | —                     |
| P02-T010          | Waiting                          | P02-T007              |
| P02-T008/P02-T009 | Waiting                          | P02-T007 and P02-T010 |
| P02-T011          | Waiting                          | All Phase 2 outcomes  |

### Next actions

1. Render the complete voice in P02-T007, sample-accurate note edge included.
2. Close P02-T006 as not-happening and record the dropped extraction as a deviation, per ADR-0040 clause 5.
3. Re-measure `Q` in P02-T010 before equivalence and CPU evidence.
4. Close `render_loop_purity`'s provenance gap for SOUND-INV-013: it proves every *registered* kernel is
   defined in the checked region, but not that a descriptor's function pointer resolves inside it.
5. Audit the specification's pre-existing conformance rows, which predate this phase: an independent read
   found four that name a check not carrying the invariant — SOUND-INV-001/004, 003/012, 007 and 008. Out of
   scope for the layout acceptance, and not a defect in the code.
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
