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

| ADR      | Required at Phase 0A exit                             | Status      | Later acceptance gate |
|----------|-------------------------------------------------------|-------------|-----------------------|
| ADR-0001 | `Accepted`                                            | `Proposed`  | —                     |
| ADR-0021 | `Accepted`                                            | `Proposed`  | —                     |
| ADR-0037 | `Accepted`                                            | `Proposed`  | —                     |
| ADR-0032 | `Accepted`                                            | No record   | —                     |
| ADR-0022 | `Accepted`, or `Deferred` with owner and evidence gap | No record   | Before Phase 3        |
| ADR-0028 | `Accepted`, or `Deferred` with owner and evidence gap | No record   | Before Phase 4        |

ADR-0037 is not in the master plan's decision list. It carries the frame count split out of ADR-0001 and is required
`Accepted` here so that the split cannot be used to pass the gate with the quantum's value still open. See *Deviations*.

## Tasks

| ID        | Deliverable                                                 | Status      | Dependencies         | Primary record                                          |
|-----------|-------------------------------------------------------------|-------------|----------------------|---------------------------------------------------------|
| P00A-T001 | Define the reference V1 corpus and preserve/change manifest | Not started | None                 | Future `EVD` record                                     |
| P00A-T002 | Define the comparison result model and headless command     | Not started | P00A-T001            | Future specification/evidence                           |
| P00A-T003 | Capture V1 CPU, memory, timing, and determinism baselines   | Not started | P00A-T001            | Future `EVD` records                                    |
| P00A-T004 | Complete the fixed-limit and overflow audit                 | Active      | None                 | [Resource inventory](../inventories/resource-limits.md) |
| P00A-T005 | Define the initial HostProfile and RenderLimits contract    | Not started | P00A-T004            | Future specification/ADRs                               |
| P00A-T006 | Satisfy every entry in the required-decisions table         | Active      | P00A-T003/P00A-T004  | [Decision register](../ADR.md)                          |
| P00A-T007 | Prepare the formal Phase 0A exit review                     | Not started | All applicable tasks | Future `REV-P00A`                                       |

Phase 0B runs in parallel and has its own tracker. Do not move its inventory work into this phase to make the gate look
complete.

P00A-T006 must accept ADR-0001, ADR-0037, ADR-0021, and ADR-0032. ADR-0022 may be deferred only to the Phase 3 entry
gate and ADR-0028 only to the Phase 4 entry gate; either deferral records an owner and the missing evidence. No other
deferral satisfies the Phase 0A exit gate.

## Active tasks

**P00A-T006 — Satisfy every entry in the required-decisions table.**

- **Scope.** One accepted record under [decisions/](../decisions/README.md) for ADR-0001, ADR-0037, ADR-0021, and
  ADR-0032, plus an accepted-or-deferred ADR-0022 and ADR-0028.
- **State.** Three of six records exist, all `Proposed`:
  [ADR-0001](../decisions/ADR-0001-internal-render-quantum.md) (quantum semantics),
  [ADR-0021](../decisions/ADR-0021-host-profile-and-admission-policy.md) (admission policy), and
  [ADR-0037](../decisions/ADR-0037-render-quantum-value.md) (quantum frame count). ADR-0032, ADR-0022, and ADR-0028
  have no record yet. **Nothing in this phase is accepted.**
- **A review withdrew the acceptance of ADR-0001 and ADR-0021**, which had been marked `Accepted` in the same session
  they were drafted. Four defects made that premature, and each is fixed in the record that carried it:
    - ADR-0001's splitting contract covered only the output side. A callback shorter than `Q` has neither the audio
      input nor the live events to render a full quantum, so the contract was unimplementable for any plan with live
      input. The decision now defines the input carry, the initial fill, the event horizon, and the `SampleTime` epoch,
      and the declared latency changed from `Q - 1` to a constant `Q`.
    - ADR-0021's class model put every configurable budget in `HostProfile`, but the ledger holds budgets that are not
      render-preparation inputs at all (`LIMIT-0063`, `LIMIT-0064`, `LIMIT-0066`, `LIMIT-0068`..`LIMIT-0071`). Failure
      behavior and configuration owner are now separate axes.
    - ADR-0021 hardcoded `64` while ADR-0037 was `Proposed`, and listed a render quantum among its `HostProfile` fields
      that ADR-0001 forbids. Both corrected.
    - ADR-0037's outcome rules overlapped — a measurement could satisfy both "select 32" and "select 128" — and never
      used its own 128-frame datapoint. Replaced by an ordered, exhaustive rule table with an explicit inconclusive
      case.
  The lesson recorded for the remaining records: drafting and accepting in one pass produced four defects that a
  separate review pass caught immediately.
- **Why ADR-0001 was split.** Only the frame count depends on the missing measurement; the splitting semantics follow
  from the partition-invariance requirement and from V1's code as read. Holding the semantics for a benchmark would
  block Phase 1 for no gain, so ADR-0001 states every clause in terms of `Q` and ADR-0037 carries the value. The gate
  is unchanged in strength: both must be `Accepted` at exit, and ADR-0037 is listed in the table above for that reason.
- **ADR-0037's measurement cannot be taken directly** — the quantity is per-quantum overhead in the V2 node model, and
  no V2 renderer exists. The record names a V1 proxy (render the corpus at 32/64/128/256 by varying
  `arrangement_render.rs:51`) and fixes its outcome rules before the data is collected. Review also withdrew the claim
  that the proxy errs in a safe direction: V1's per-block `resize` is not a real cost, because the buffer is
  preallocated at `MAX_BUFFER_SIZE` (`voice.rs:570`), while V2 adds carry copies and scheduler work V1 never paid. The
  proxy now shows curve shape only, with an explicit inconclusive band.
- **ADR-0021 deliberately excludes numbers.** Its register basis named measurements; the record splits the topic so
  that class semantics and failure behavior — determinable from code already read — are decided there, while every
  numeric default moves to P00A-T005, which already depends on P00A-T003. The scope split still needs an explicit
  accept or reject at the exit review; the register basis is provisionally updated to `V1 cap inventory` to match.
- **What the records changed elsewhere.** Drafting ADR-0021 surfaced a contradiction in the resource ledger: the
  preamble claimed ADR-0021 owned every row of the silent-truncation register while `LIMIT-0013` and `LIMIT-0020`
  carried ADR-0027 alone. Both now carry ADR-0021 for the overflow question and ADR-0027 for tap ownership. That fix
  is independent of acceptance and stands. The five entries' `Proposed V2 rule` cells were filled while ADR-0021 was
  marked accepted and were **reverted** when that was withdrawn, per the ledger's own status rule.
- **The master plan is deliberately not yet updated.** ADR-0001 forbids `quantum` in both `RenderConfig`
  (`master-plan.md:411`) and `HostProfile` (`master-plan.md:1767`), and the
  [authority rule](../README.md#sources-of-truth) requires the plan to change in the same change as the accepted ADR
  that supersedes it. While the record is `Proposed` the plan is correct as it stands; both edits are listed as
  acceptance-gated follow-up in ADR-0001.
- **Remaining in this task.** ADR-0032, ADR-0022, and ADR-0028 have no record; ADR-0001 and ADR-0021 need a second
  review pass after this revision; ADR-0037 needs its measurement.
- **Implementation revision.** Documentation only; no code changed. The records cite source reads at `5cd24de8`, one
  commit later than the inventories' `dd69b657`.

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
- **Remaining before the gate.** Two things, neither of which is more searching. First, the class sweep, which is
  blocked until ADR-0021 is accepted — it is still `Proposed`, so the `Proposed V2 rule` column is blank for all 74
  entries and nothing has been filled. Once accepted, that ADR assigns each entry a failure class *and* one of seven
  configuration owners, and every `Unknown`-class entry must reach a terminal class. Second, confirming that no
  *undocumented* silent truncation exists needs an executable probe (oversized blocks, >128 metered channels, >32 rack
  stages) — and no value in this ledger has been measured, only read.
- **Status-vocabulary correction (review follow-up).** Entries were marked `Classified` once their value, site, and
  overflow behavior were established. The [register vocabulary](../inventories/README.md) reserves `Classified` for
  required fields *and* disposition filled with supporting evidence, and this ledger's disposition is
  `Proposed V2 rule`, which was blank pending ADR-0021. All such statuses are downgraded to `Investigating`, and the
  ledger now states the rule inline. `LIMIT-0067` also carried two ADR owners; it is now ADR-0021 alone, matching the
  silent-truncation register. The five filled entries stay `Investigating` too: they have a rule but no `EVD` record.
- **Implementation revision.** Documentation only; no code changed.

## Deliverables and verification

| Task      | Output/revision                                                                                                                                                        | Verification/evidence                                                                                                                                                                                                                                    | Result                                       |
|-----------|------------------------------------------------------------------------------------------------------------------------------------------------------------------------|----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|----------------------------------------------|
| P00A-T004 | [Resource inventory](../inventories/resource-limits.md) passes 1 and 2 at `dd69b657`                                                                                   | Two independent discovery methods with opposite blind spots: pass 1 matched constant names, pass 2 matched documented truncation behavior. Neither executes anything, so a truncation that is both unnamed and undocumented would still be missed        | Partial — source-read only, no measurement   |
| P00A-T006 | [ADR-0001](../decisions/ADR-0001-internal-render-quantum.md), [ADR-0021](../decisions/ADR-0021-host-profile-and-admission-policy.md), and [ADR-0037](../decisions/ADR-0037-render-quantum-value.md), all `Proposed` at `5cd24de8` | Every option and consequence traces to a cited inventory entry or source site. A review pass over the first revision found four defects — an unimplementable buffering contract, a class model inapplicable to its own ledger, a hardcoded quantum contradicting two other records, and overlapping outcome rules — and withdrew the two acceptances. All four are fixed; none of the three records has been reviewed since | Partial — 3 of 6 records, 0 accepted |

## Deviations

**One registered topic split into two identifiers.** The master plan's Part VII topic 1 names a single internal-quantum
decision, and the Phase 0A exit gate names `ADR-0001 (RenderQuantum)`. That topic is now carried by two records:
ADR-0001 for the splitting semantics and ADR-0037 for the frame count, both currently `Proposed`.

- **Why.** The frame count is the only part depending on a measurement that cannot yet be taken, and Phase 1 needs the
  semantics to begin. One record could not hold two statuses.
- **How the gate is preserved.** ADR-0037 is added to this tracker's required-decisions table as `Accepted`-at-exit, so
  the exit gate still requires the quantum's value to be settled. The split moves where the value is recorded, not
  whether it must be decided.
- **What a reviewer should check.** That no clause of ADR-0001 silently assumes a particular `Q`, and that ADR-0037's
  acceptance criteria were fixed before its measurement was taken.
- **Master-plan sync, step 1 — done.** The plan is authoritative for exit gates, so it must not lag a registered split.
  [Part VII](../master-plan.md#part-vii-open-decisions) topic 1 now names both records and states which fixes what, and
  the [Phase 0A exit gate](../master-plan.md#phase-0a-baseline-limits-and-render-core-contracts) now requires both
  `Accepted`.
- **Master-plan sync, step 2 — acceptance-gated.** Three references still grant the quantum a configuration surface
  that ADR-0001 removes: the Phase 0A `HostProfile` work item (`master-plan.md:264`), `RenderConfig::quantum`
  (`master-plan.md:411`), and the `HostProfile` field list (`master-plan.md:1767`). They are correct as long as
  ADR-0001 is `Proposed` and must all be removed on acceptance; ADR-0001's follow-up table lists all three.

Any scope, ordering, or contract change must link to an ADR or an explicit master-plan update. Do not bury architecture
changes in this tracker.

## Exit readiness

Status: Not ready

The formal review must evaluate every Phase 0A exit gate in the master plan and link direct evidence. Phase 1 may not
begin until that review is accepted.
