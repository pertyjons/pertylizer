# Phase 0B: Migration Inventories and Project Contracts

| Field         | Value                                                                              |
|---------------|------------------------------------------------------------------------------------|
| Status        | Not started                                                                        |
| Phase         | 00B                                                                                |
| Last reviewed | 2026-08-12                                                                         |
| Master plan   | [Phase 0B](../master-plan.md#phase-0b-migration-inventories-and-project-contracts) |
| Exit review   | Not created                                                                        |

## Objective

Produce the exhaustive V1 migration ledgers and the project/application contracts required before Phase 10, without
blocking the render-core work in Phases 1-4.

The master plan defines scope and exit gates. This tracker records execution state only: the tasks below decompose the
Phase 0B `Work` list and add no scope of their own.

## Entry conditions

- The master plan is available and reviewed for current relevance.
- V1 remains the production engine.
- The documentation workflow and identifier rules are understood.
- A task is assigned and marked `Active` before implementation begins.

This phase may run concurrently with Phase 0A and with Phases 1-4. It does not require Phase 0A to be complete.

## Required decisions

Every entry must be `Accepted` before the Phase 0B exit review. Earlier
deadlines are additional entry gates on dependent phases.

| ADR | Topic | Earlier deadline, if any |
| --- | --- | --- |
| ADR-0013 | Project/session/settings boundary cases | — |
| ADR-0014 | Persistent ID generation and encoding | — |
| ADR-0016 | Known-version unknown-field policy | — |
| ADR-0017 | Asset identity and external references | — |
| ADR-0018 | Editor metadata persistence scope | — |
| ADR-0024 | Recording take and commit semantics | Before Phase 9 |
| ADR-0025 | Tuning representation and ownership | Before Phase 6 |
| ADR-0027 | Observation and analyzer ownership | Before Phase 5 |
| ADR-0029 | Host configuration and remote authorization | — |
| ADR-0031 | Supported build and release matrix | — |
| ADR-0034 | Track, source, and channel ownership | — |
| ADR-0035 | Transaction and concurrency semantics | — |
| ADR-0036 | Audio device and input lifecycle | Before Phase 9 |

## Tasks

| ID        | Deliverable                                                     | Status      | Dependencies         | Primary record                                         |
|-----------|-----------------------------------------------------------------|-------------|----------------------|--------------------------------------------------------|
| P00B-T001 | Complete the persisted-state ownership audit                    | Not started | None                 | [State inventory](../inventories/state-ownership.md)   |
| P00B-T002 | Complete the capability and reachability audit                  | Not started | None                 | [Capability inventory](../inventories/capabilities.md) |
| P00B-T003 | Complete the identity and reference audit                       | Not started | None                 | [Identity inventory](../inventories/identities.md)     |
| P00B-T004 | Trace representative mutation, save, recovery, and render paths | Not started | P00B-T001/P00B-T002  | Future `EVD` records                                   |
| P00B-T005 | Add omission-prone project round-trip fixtures                  | Not started | P00B-T001            | Test evidence                                          |
| P00B-T006 | Define the application-operation result contract                | Not started | P00B-T001/P00B-T003  | Future specification/ADR                               |
| P00B-T007 | Define the Project Format V2 envelope and conversion policy     | Not started | P00B-T001/P00B-T003  | Future specification/ADRs                              |
| P00B-T008 | Accept every ADR in the required-decisions table                | Not started | Relevant inventories | [Decision register](../ADR.md)                         |
| P00B-T009 | Prepare the formal Phase 0B exit review                         | Not started | All applicable tasks | Future `REV-P00B`                                      |

Four decisions in P00B-T008 gate an earlier phase and must be prioritized accordingly: ADR-0027 before Phase 5,
ADR-0025 before Phase 6, and ADR-0024 and ADR-0036 before Phase 9.

Deferred decisions may remain visible while Phase 0B is active, but all decisions in P00B-T008 must be `Accepted`
before the Phase 0B exit review can pass.

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

The formal review must evaluate every Phase 0B exit gate in the master plan and link direct evidence. Phase 10 may not
begin until that review is accepted.
