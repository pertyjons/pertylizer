# Phase 0A: Baseline, Limits, and Render-Core Contracts

| Field         | Value                                                                            |
|---------------|----------------------------------------------------------------------------------|
| Status        | Active                                                                           |
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

| ADR      | Required at Phase 0A exit                             | Later acceptance gate |
|----------|-------------------------------------------------------|-----------------------|
| ADR-0001 | `Accepted`                                            | —                     |
| ADR-0021 | `Accepted`                                            | —                     |
| ADR-0032 | `Accepted`                                            | —                     |
| ADR-0022 | `Accepted`, or `Deferred` with owner and evidence gap | Before Phase 3        |
| ADR-0028 | `Accepted`, or `Deferred` with owner and evidence gap | Before Phase 4        |

## Tasks

| ID        | Deliverable                                                 | Status      | Dependencies         | Primary record                                          |
|-----------|-------------------------------------------------------------|-------------|----------------------|---------------------------------------------------------|
| P00A-T001 | Define the reference V1 corpus and preserve/change manifest | Not started | None                 | Future `EVD` record                                     |
| P00A-T002 | Define the comparison result model and headless command     | Not started | P00A-T001            | Future specification/evidence                           |
| P00A-T003 | Capture V1 CPU, memory, timing, and determinism baselines   | Not started | P00A-T001            | Future `EVD` records                                    |
| P00A-T004 | Complete the fixed-limit and overflow audit                 | Active      | None                 | [Resource inventory](../inventories/resource-limits.md) |
| P00A-T005 | Define the initial HostProfile and RenderLimits contract    | Not started | P00A-T004            | Future specification/ADRs                               |
| P00A-T006 | Satisfy every entry in the required-decisions table         | Not started | P00A-T003/P00A-T004  | [Decision register](../ADR.md)                          |
| P00A-T007 | Prepare the formal Phase 0A exit review                     | Not started | All applicable tasks | Future `REV-P00A`                                       |

Phase 0B runs in parallel and has its own tracker. Do not move its inventory work into this phase to make the gate look
complete.

P00A-T006 must accept ADR-0001, ADR-0021, and ADR-0032. ADR-0022 may be deferred only to the Phase 3 entry gate and
ADR-0028 only to the Phase 4 entry gate; either deferral records an owner and the missing evidence. No other deferral
satisfies the Phase 0A exit gate.

## Active task

**P00A-T004 — Complete the fixed-limit and overflow audit.**

- **Scope.** Populate the [resource inventory](../inventories/resource-limits.md) with every fixed cap, truncation
  point, bounded queue, buffer capacity, and script budget in the workspace, each with its enforcement site and — where
  the enforcing code was read — its overflow behavior.
- **Non-goals.** Proposing V2 admission rules (that is P00A-T005 and ADR-0021), measuring anything (that is P00A-T003),
  and changing any current limit.
- **Related identifiers.** `LIMIT-0001`..`LIMIT-0074`; ADR-0001, ADR-0021, ADR-0008, ADR-0022.
- **Expected output.** A ledger whose per-entry blanks are honest "not yet investigated" markers, plus a recorded audit
  method a second pass can rerun and diff.
- **Verification.** Re-running the recorded search at a later revision must not surface a constant absent from the
  ledger. Not yet performed.
- **State after pass 2 (`dd69b657`).** 74 entries. All six required areas pass 1 could not reach are answered, both
  unnumbered families have identifiers, and the two entries that gate this phase are resolved:
    - `LIMIT-0001` — a block above `MAX_BLOCK_SIZE` silently drops the audio-input tail *and* reallocates a voice buffer
      on the audio thread. Not reachable through the app's own configuration, because the requested buffer size is
      hardcoded to at most 1024 (`LIMIT-0057`) — but the callback's frame count is recomputed from what the host
      actually delivers, so a host that ignores the request reaches it. The RT-allocation regression test covers up to
      1024 frames only.
    - `LIMIT-0028` — **not a conflict.** The two constants bound two unrelated features (the shared choir sub-voice bank
      and the classic oscillator's unison spread), and both are array lengths, so neither can be exceeded. A
      silent-truncation register is now compiled: five sites, of which only the event-drop counters have a diagnostic.
      That is the specific input this phase's exit gate needs from ADR-0021.
- **Remaining before the gate.** Nothing further from this task by search alone. Confirming that no *undocumented*
  silent truncation exists needs an executable probe (oversized blocks, >128 metered channels, >32 rack stages), not
  another pass of reading — and no value in this ledger has been measured, only read.
- **Status-vocabulary correction (review follow-up).** Entries were marked `Classified` once their value, site, and
  overflow behavior were established. The [register vocabulary](../inventories/README.md) reserves `Classified` for
  required fields *and* disposition filled with supporting evidence, and this ledger's disposition is
  `Proposed V2 rule`, which is blank pending ADR-0021. All such statuses are downgraded to `Investigating`, and the
  ledger now states the rule inline. `LIMIT-0067` also carried two ADR owners; it is now ADR-0021 alone, matching the
  silent-truncation register.
- **Implementation revision.** Documentation only; no code changed.

## Deliverables and verification

| Task      | Output/revision                                                                      | Verification/evidence                                                                                                                                                                                                                             | Result                                     |
|-----------|--------------------------------------------------------------------------------------|---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|--------------------------------------------|
| P00A-T004 | [Resource inventory](../inventories/resource-limits.md) passes 1 and 2 at `dd69b657` | Two independent discovery methods with opposite blind spots: pass 1 matched constant names, pass 2 matched documented truncation behavior. Neither executes anything, so a truncation that is both unnamed and undocumented would still be missed | Partial — source-read only, no measurement |

## Deviations

No deviations from the master plan are recorded.

Any scope, ordering, or contract change must link to an ADR or an explicit master-plan update. Do not bury architecture
changes in this tracker.

## Exit readiness

Status: Not ready

The formal review must evaluate every Phase 0A exit gate in the master plan and link direct evidence. Phase 1 may not
begin until that review is accepted.
