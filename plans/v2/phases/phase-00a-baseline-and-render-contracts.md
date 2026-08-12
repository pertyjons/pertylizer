# Phase 0A: Baseline, Limits, and Render-Core Contracts

| Field         | Value                                                                            |
|---------------|----------------------------------------------------------------------------------|
| Status        | Not started                                                                      |
| Phase         | 00A                                                                              |
| Last reviewed | 2026-08-12                                                                       |
| Master plan   | [Phase 0A](../master-plan.md#phase-0a-baseline-limits-and-render-core-contracts) |
| Exit review   | Not created                                                                      |

## Objective

Establish measurable V1 baselines, a headless comparison harness, the real-time limit audit, and the few contracts and
decisions Sound Core V2 needs before Phase 1 begins.

The master plan defines scope and exit gates. This tracker records execution state only: the tasks below decompose the
Phase 0A `Work` list and add no scope of their own.

## Entry conditions

- The master plan is available and reviewed for current relevance.
- V1 remains the production engine.
- The documentation workflow and identifier rules are understood.
- A task is assigned and marked `Active` before implementation begins.

## Required decisions

| ADR | Required at Phase 0A exit | Later acceptance gate |
| --- | --- | --- |
| ADR-0001 | `Accepted` | — |
| ADR-0021 | `Accepted` | — |
| ADR-0032 | `Accepted` | — |
| ADR-0022 | `Accepted`, or `Deferred` with owner and evidence gap | Before Phase 3 |
| ADR-0028 | `Accepted`, or `Deferred` with owner and evidence gap | Before Phase 4 |

## Tasks

| ID        | Deliverable                                                  | Status      | Dependencies         | Primary record                                          |
|-----------|--------------------------------------------------------------|-------------|----------------------|---------------------------------------------------------|
| P00A-T001 | Define the reference V1 corpus and preserve/change manifest   | Not started | None                 | Future `EVD` record                                     |
| P00A-T002 | Define the comparison result model and headless command      | Not started | P00A-T001            | Future specification/evidence                           |
| P00A-T003 | Capture V1 CPU, memory, timing, and determinism baselines    | Not started | P00A-T001            | Future `EVD` records                                    |
| P00A-T004 | Complete the fixed-limit and overflow audit                  | Not started | None                 | [Resource inventory](../inventories/resource-limits.md) |
| P00A-T005 | Define the initial HostProfile and RenderLimits contract     | Not started | P00A-T004            | Future specification/ADRs                               |
| P00A-T006 | Satisfy every entry in the required-decisions table           | Not started | P00A-T003/P00A-T004  | [Decision register](../ADR.md)                          |
| P00A-T007 | Prepare the formal Phase 0A exit review                      | Not started | All applicable tasks | Future `REV-P00A`                                       |

Phase 0B runs in parallel and has its own tracker. Do not move its inventory work into this phase to make the gate
look complete.

P00A-T006 must accept ADR-0001, ADR-0021, and ADR-0032. ADR-0022 may be deferred only to the Phase 3 entry gate and
ADR-0028 only to the Phase 4 entry gate; either deferral records an owner and the missing evidence. No other deferral
satisfies the Phase 0A exit gate.

## Active task

No task is active.

When a task is activated, record:

- exact scope and non-goals;
- relevant ADRs and inventory identifiers;
- expected output and verification;
- implementation branch or revision when code is involved.

## Deliverables and verification

| Task | Output/revision | Verification/evidence | Result |
|------|-----------------|-----------------------|--------|

## Deviations

No deviations from the master plan are recorded.

Any scope, ordering, or contract change must link to an ADR or an explicit master-plan update. Do not bury architecture
changes in this tracker.

## Exit readiness

Status: Not ready

The formal review must evaluate every Phase 0A exit gate in the master plan and link direct evidence. Phase 1 may not
begin until that review is accepted.
