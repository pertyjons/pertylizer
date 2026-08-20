# Core V2: Current Work

Last updated: 2026-08-20

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

Phase 3's state stays `Not started`. It is **not** blocked on Phase 2, which is closed; it is blocked on two decisions
that must be `Accepted` **before implementation begins**, and neither is:

| Prerequisite | Current status | What it must settle |
|---|---|---|
| [ADR-0022](decisions/ADR-0022-hardware-time-mapping.md) — hardware time mapping and latency ownership | **`Deferred`** | Phase 3 may refine it through a superseding ADR on simulated-host evidence; it may **not** invent timestamp semantics inside implementation tasks |
| An **ADR-0001 clarification or successor** | **[ADR-0043](decisions/ADR-0043-event-deferral-and-late-clamp.md), `Proposed`** — drafted, not decided | When clause 16's late condition is evaluated, and whether a quantum may defer an event at all under clauses 12 and 14. The two cannot both be implemented as written; the specification states an interim rule and marks it as a narrowing it may not make. ADR-0043 presents four options with a recommendation and selects none — the choice is the maintainer's. **Whether accepting it closes this prerequisite depends on the option chosen:** a deferral option leaves `+Q` able to render a note-on after its own note-off, whose repair is outside ADR-0043's boundary; the no-deferral option closes it only if Phase 3's capacities make its runtime callback fault unreachable |

Both obligations arrive from Phase 0A, which narrowed P00A-T005 rather than blocking on them — Phase 1 has no live
ingress and no host callback, so the capacity a deferral operates against could not be specified from below. See
[`REV-P00A`](reviews/phase-00a-exit-review.md).

**Activating Phase 3 is the user's call.** One of the two records is now drafted; selecting its option and taking it
to `Accepted` is the next slice on that side — and under a deferral option that slice also owes a causal-order policy,
which ADR-0043 names as a blocker rather than solves. ADR-0022 remains untouched. Phase 0B's
paused stream below is the other candidate; it gates Phase 10 and nothing in Phase 3 depends on it.

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
| P00B-T004–T009 | Not started | Follow the decomposition in the frozen execution record                    |

This stream does not block Phase 2. Its detailed audit chronology remains in
the [historical Phase 0B execution record](phases/phase-00b-inventories-and-project-contracts.md); new operational state
is recorded only here.

Phase lifecycle and completed gates are recorded once in
[`ROADMAP.md`](ROADMAP.md#phase-order).

## Later owned work

- Phase 3 owns renderer ingress, deferred-event storage, event scheduling, and the pending ADR-0001 clarification.
- Phase 4 owns current-project lowering and the long-running job contract.
- Phase 5 owns the `LegacyPolyModuleAdapter`'s conversion cost — the largest quantity ADR-0041 moves and the only one
  nobody has measured — and the declarative node API that `SOUND-INV-012`'s uncovered second sentence belongs to.
- Phase 0B gates Phase 10.
- ADR-0039 and `LIMIT-0017` remain Phase 10E work.
