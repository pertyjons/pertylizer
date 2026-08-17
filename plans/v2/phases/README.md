# V2 Phase Trackers

Phase trackers turn the strategic master plan into executable, reviewable work. They record tasks, outputs, evidence,
implementation revisions, and deviations. They do not redefine architecture or exit gates.

## Status vocabulary

- `Not started`
- `Active`
- `Blocked`
- `Ready for exit review`
- `Complete`

Only one migration phase should normally be `Active`. Bounded preparation for a later phase is allowed when the active
phase explicitly records the dependency.

Phase 0B is the one declared exception: the master plan runs it in parallel with Phases 1-4, so it may be `Active`
alongside another phase.

Task status is one of `Not started`, `Active`, `Blocked`, `Complete`, or
`Deferred`. A deferred task names its owner, destination phase or revisit
condition, and the gate that permits deferral. A phase normally has one active
task; explicitly independent evidence or inventory tasks may run concurrently
when the tracker records that coordination.

## Creating a tracker

Create a tracker from [../templates/phase.md](../templates/phase.md) only when a phase is being prepared or activated.
Use:

```text
phase-NN-short-kebab-case-name.md
phase-NNx-short-kebab-case-name.md   # sub-phase, e.g. phase-10c-history-and-save.md
```

Tasks use stable identifiers such as `P03-T006`, or `P10C-T004` in a sub-phase. A task records its deliverable,
dependencies, related ADRs/inventories, implementation revisions, and verification. Do not use task completion as proof
that an exit gate passed.

Every tracker contains an exact `Required decisions` table. Do not rely only on
a phase-prefix filter: the table pins the ADRs and required statuses reviewed by
that phase. Keep it synchronized with the authoritative decision register and
master-plan gates.

The `Work` list of the phase in the master plan is its normative scope; tracker tasks decompose that list. A task that
adds, drops, or reinterprets scope requires a plan change or an ADR and belongs under `Deviations`.

## Closing a phase

1. Complete or explicitly defer every scoped task.
2. Update inventories and current specifications.
3. Put every ADR in the tracker's required-decisions table into the exact
   status permitted by that gate.
4. Create an exit review from
   [../templates/exit-review.md](../templates/exit-review.md).
5. Link evidence for every gate.
6. Mark the phase `Complete` only after the review outcome is `Accepted`.

## Trackers

- [Phase 0A: Baseline, limits, and render-core contracts](phase-00a-baseline-and-render-contracts.md)
- [Phase 0B: Migration inventories and project contracts](phase-00b-inventories-and-project-contracts.md)
- [Phase 1: Introduce the experimental Sound Core V2 crate](phase-01-experimental-sound-core.md)
