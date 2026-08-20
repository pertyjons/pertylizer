# Core V2: Current Work

Last updated: 2026-08-20

This is the only authority for active Core V2 task state, blockers, and next actions. Durable reasoning and measurements
live in the linked ADRs, specs, and EVDs rather than being repeated here.

## Primary stream: Phase 2

Outcome: render one complete compiled monophonic voice through validated, scheduled V2 nodes and close Phase 2's
equivalence, CPU, and quantum gates.

### Active slice

**Execute P02-T008 and P02-T009: the phase's two evidence records.**

**Both methods are written, reviewed and `Draft`; collection is what remains.** `PROCESS.md` requires the falsifier
and acceptance rule before collection and the method reviewed before any data exists, and that is the part now done:

- **P02-T008** — [EVD-0013](evidence/phase-02/EVD-0013-minimal-patch-equivalence.md), musical equivalence to V1 for
  the equivalent minimal patch, or a documented intentional difference.
- **P02-T009** — [EVD-0014](evidence/phase-02/EVD-0014-minimal-patch-cpu.md), CPU against V1 for that same patch.

They are one slice because they share a fixture, and the review was not a formality: it returned **nine blocking
findings before any data existed**, and three more passes on the repairs. Both records' *History* sections carry them.

Blockers: none. What collection now needs, in order:

1. The V1 fixture builder, outside `FIXTURES` for the reason `corpus::fixtures::polyphony_probe` is.
2. EVD-0013's controls C1, C2 and C3, then its renders and comparisons.
3. `synth_engine_v2` as a **dev-dependency** of `pertylizer`, so EVD-0014's two arms share one binary.
4. EVD-0014's C3, then its null pass, then its sweeps — in that order, and its rule 0 can stop it at the null pass.

Three things they inherit, stated so they are not rediscovered:

- **The instrument, its traps and this machine's behaviour are EVD-0012's**, which reuses EVD-0010's estimator and
  records what went wrong with it. Read that record's *Method*, *History* and *Limitations* before building a harness;
  do not re-derive them here.
- **"The equivalent minimal patch" is now defined**, in EVD-0013, along with the five asymmetries closed in the
  fixture and the two differences that are the subject. It is a counterpart fixture inheriting CORPUS-0001's claim
  classes, not a manifest case.
- **The user decided the three open questions on 2026-08-20**: the counterpart fixture over a new manifest case, the
  dev-dependency over a separate crate or two binaries, and both CPU pairs rather than one.

References:

- [EVD-0012: what the render quantum costs on the real V2 path](evidence/phase-02/EVD-0012-render-quantum-real-path.md),
  whose method and harness these two reuse
- [ADR-0001: render quantum semantics](decisions/ADR-0001-internal-render-quantum.md), clause 17 for why these run
  after P02-T010 rather than beside it
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
| P02-T010          | Complete                         | EVD-0012              |
| P02-T008/P02-T009 | **Active** — methods reviewed    | Collection            |
| P02-T011          | Waiting                          | All Phase 2 outcomes  |

### Next actions

1. Complete P02-T008 and P02-T009, the phase's two evidence records.
2. Close P02-T006 as not-happening and record the dropped extraction as a deviation, per ADR-0040 clause 5.
3. Close `render_loop_purity`'s provenance gap for SOUND-INV-013: it proves every *registered* kernel is
   defined in the checked region, but not that a descriptor's function pointer resolves inside it.
4. Audit the specification's pre-existing conformance rows, which predate this phase: an independent read
   found four that name a check not carrying the invariant — SOUND-INV-001/004, 003/012, 007 and 008. Not a
   defect in the code, and recorded in the specification's unresolved questions.
5. Decide where a note's **pitch and velocity** live. P02-T007 deliberately gave `NoteEdge` neither, because
   nothing in Phase 2 reads either; Phase 3's ingress is the first task that has to.
6. Complete the Phase 2 exit review.

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
