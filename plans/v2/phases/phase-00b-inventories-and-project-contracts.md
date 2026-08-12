# Phase 0B: Migration Inventories and Project Contracts

| Field         | Value                                                                              |
|---------------|------------------------------------------------------------------------------------|
| Status        | Active                                                                             |
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

Every entry must be `Accepted` before the Phase 0B exit review. Earlier deadlines are additional entry gates on
dependent phases.

| ADR      | Topic                                       | Earlier deadline, if any |
|----------|---------------------------------------------|--------------------------|
| ADR-0013 | Project/session/settings boundary cases     | —                        |
| ADR-0014 | Persistent ID generation and encoding       | —                        |
| ADR-0016 | Known-version unknown-field policy          | —                        |
| ADR-0017 | Asset identity and external references      | —                        |
| ADR-0018 | Editor metadata persistence scope           | —                        |
| ADR-0024 | Recording take and commit semantics         | Before Phase 9           |
| ADR-0025 | Tuning representation and ownership         | Before Phase 6           |
| ADR-0027 | Observation and analyzer ownership          | Before Phase 5           |
| ADR-0029 | Host configuration and remote authorization | —                        |
| ADR-0031 | Supported build and release matrix          | —                        |
| ADR-0034 | Track, source, and channel ownership        | —                        |
| ADR-0035 | Transaction and concurrency semantics       | —                        |
| ADR-0036 | Audio device and input lifecycle            | Before Phase 9           |

## Tasks

| ID        | Deliverable                                                     | Status      | Dependencies         | Primary record                                         |
|-----------|-----------------------------------------------------------------|-------------|----------------------|--------------------------------------------------------|
| P00B-T001 | Complete the persisted-state ownership audit                    | Active      | None                 | [State inventory](../inventories/state-ownership.md)   |
| P00B-T002 | Complete the capability and reachability audit                  | Active      | None                 | [Capability inventory](../inventories/capabilities.md) |
| P00B-T003 | Complete the identity and reference audit                       | Active      | None                 | [Identity inventory](../inventories/identities.md)     |
| P00B-T004 | Trace representative mutation, save, recovery, and render paths | Not started | P00B-T001/P00B-T002  | Future `EVD` records                                   |
| P00B-T005 | Add omission-prone project round-trip fixtures                  | Not started | P00B-T001            | Test evidence                                          |
| P00B-T006 | Define the application-operation result contract                | Not started | P00B-T001/P00B-T003  | Future specification/ADR                               |
| P00B-T007 | Define the Project Format V2 envelope and conversion policy     | Not started | P00B-T001/P00B-T003  | Future specification/ADRs                              |
| P00B-T008 | Accept every ADR in the required-decisions table                | Not started | Relevant inventories | [Decision register](../ADR.md)                         |
| P00B-T009 | Prepare the formal Phase 0B exit review                         | Not started | All applicable tasks | Future `REV-P00B`                                      |

Four decisions in P00B-T008 gate an earlier phase and must be prioritized accordingly: ADR-0027 before Phase 5, ADR-0025
before Phase 6, and ADR-0024 and ADR-0036 before Phase 9.

Deferred decisions may remain visible while Phase 0B is active, but all decisions in P00B-T008 must be `Accepted`
before the Phase 0B exit review can pass.

## Active tasks

Three tasks are active concurrently. This is the coordination the
[phase-tracker guide](README.md) requires a tracker to record: P00B-T001, P00B-T002, and P00B-T003 have no dependency on
each other, own three separate ledgers and three separate identifier series, and were run as one audit pass over one
source revision so their coverage statements are comparable.

**P00B-T001 — Persisted-state ownership audit.**

- **Scope.** One [state-ownership](../inventories/state-ownership.md) entry per persisted structure, plus every save
  source and mirror that supplies it.
- **Non-goals.** Choosing the V2 owner for a contested field — that is ADR-0013 and ADR-0018.
- **Related identifiers.** `STATE-0001`..`STATE-0060`; ADR-0013, ADR-0016, ADR-0017, ADR-0018, ADR-0024, ADR-0034.
- **State after pass 3 (`dd69b657`).** 60 entries; the `Dirty/undo behavior` column is filled for every one, from the
  seven revision terms in `dirty.rs` and the 60 `UndoAction` variants. Three workflows traced (autosave, patch save,
  bundle save/load). Three results are worth the phase's attention:
    - **`active_instrument_id` is watched by no dirty term.** The three instrument-switch sites set it without calling
      `mark_dirty()`, while every save writes it — so changing the focused instrument changes the file that would be
      saved with no `*`, no autosave snapshot, and no close prompt. This is the same class of defect the `layout`,
      `global`, and `effect_order` fingerprints were each added to fix, and it is the fourth instance.
    - **Review follow-up: the author mapping was wrong in both earlier passes.** `ProjectFile.author` does not come
      from `AppSettings.author`. The save path reads a separate per-project field, `current_project_author`; settings
      is only the seed for a new project; and MCP holds a third copy (`STATE-0060`). The mistake was reading the
      declaration (`ProjectBuildOptions.author`) instead of following who fills it, so every remaining
      `Mirrors/save sources` cell derived that way is now treated as unconfirmed.
    - **Pass 1's "three values persisted twice" reading was also wrong, and is corrected in place.** The per-patch
      copies are written as constants from `PatchSettings::default()` on the project save path, so they are dead weight
      rather than a live duplicate — but `session.rs:1372` still applies the patch copy of `octave_offset` on load, so
      the two paths can fight.
- **Standing findings.** One concept split across two file sections (`STATE-0010`/`STATE-0043`), two author fields of
  different shape (`STATE-0036`), transport state in the document (`STATE-0045`), and 14 entries with no undo action.

**P00B-T002 — Capability and reachability audit.**

- **Scope.** One [capability](../inventories/capabilities.md) entry per surface, with the exact member count where it
  was measured.
- **Non-goals.** Assigning dispositions before reachability is established.
- **Related identifiers.** `CAP-0001`..`CAP-0507`; ADR-0029, ADR-0030, ADR-0031.
- **State after pass 3 (`dd69b657`).** 507 identifiers: 55 surface entries plus a 452-row per-item enumeration. The
  seven application-wide GUI actions and the startup recovery offer are enumerated from `AppShortcut::ALL`, a closed
  table that also renders the menu, so that set is complete by construction rather than by search. MCP behavior
  annotations are exactly complete: **71** read-only + **97** `destructive_hint = false` + **51**
  `destructive_hint = true` = 219. (Passes 1-2 reported 98 `false`; a raw `rg` also matches the comment at
  `server/tools/batch.rs:35` explaining why `batch_execute` is *not* non-destructive.) The 51 destructive tools give
  ADR-0029 a ready-made gating set.
- **Per-item enumeration (review follow-up).** The master plan's `Work` list requires every `EngineCommand` and
  `EngineEvent` variant, every MCP tool, and every module type, patch, and template to appear individually. Passes 1
  and 2 recorded family rollups instead and justified the gap by pointing at ADR-0030 and ADR-0031 — which cover the
  public facade and the build matrix, **not** general dispositions. That reasoning was wrong and is withdrawn. Pass 3
  generates the enumeration from source as `CAP-0056`..`CAP-0507`; the 14 family rows stay at their identifiers as
  rollups carrying no disposition.
- **Still open, without an ADR excuse.** No entry has a disposition. That is the remaining substance of this task, not
  a formality deferred elsewhere, and until it is done the Phase 0B exit gate cannot pass. `CAP-0017` (multi-client
  hub) is additionally the one surface whose shipped reachability is unknown.

**P00B-T003 — Identity and reference audit.**

- **Scope.** One [identity](../inventories/identities.md) entry per identity or cross-boundary reference in the project
  document.
- **Non-goals.** Proposing the V2 identity rule — that is ADR-0014.
- **Related identifiers.** `IDN-0001`..`IDN-0031`; ADR-0014, ADR-0015, ADR-0016, ADR-0017, ADR-0030.
- **State after pass 2 (`dd69b657`).** 31 entries. Undo, duplication, engine identity, the bundle format, and the one
  name-as-identity path are now covered; two pass-1 hypotheses were disproved by reading the code and are corrected in
  place. What the pass established:
    - **Undo does not restore a deleted note's identity** (`IDN-0027`). Undoing a delete dispatches `AddNote`, which
      allocates a fresh `NoteId`; the id in the snapshot is carried but unused. Any `NoteId` held across the undo
      dangles, and `next_note_id` climbs once per cycle.
    - **The module id encodes its type at runtime too** (`IDN-0029`) — `ModuleId { module_type, instance }` is the
      engine's own key, so this is not a format-layer problem, and `script_seed_base` derives a PRNG seed from the
      instance number, making renumbering audible.
    - **One load-time heuristic repair exists** (`IDN-0028`), in the legacy Mod Matrix address upgrade — but *not* in
      the two places pass 1 suspected: pattern duplication remaps note ids properly, and the module-instance counter is
      reconciled by `max()`.
- **Deliberately still open.** `TransactionId`/`ClientId` wait on `CAP-0017`'s reachability question; tracker-module
  import is on an unmerged branch and does not exist at this revision.

All three tasks stay `Active`.

**Status-vocabulary correction (review follow-up).** Passes 1-2 marked entries `Classified` on the strength of their
facts being settled. The [register vocabulary](../inventories/README.md) requires required fields *and* disposition
filled *with supporting evidence*, and no ledger here has either a disposition or an `EVD` record yet. Every
`Classified` status has been downgraded and each ledger now states the rule inline so it cannot recur. The statuses
were the misleading part, not the content.

What remains is decision work — dispositions, V2 owners, admission rules — plus the evidence records. That is the
substance of these three tasks, and it is not further searching.

## Deliverables and verification

| Task      | Output/revision                                                                     | Verification/evidence                                                                                                                                                                   | Result  |
|-----------|-------------------------------------------------------------------------------------|-----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|---------|
| P00B-T001 | [State inventory](../inventories/state-ownership.md) passes 1 and 2 at `dd69b657`   | Not yet verified. The round-trip fixtures of P00B-T005 are the intended verification and do not exist yet; `STATE-0004`'s dirty gap is a natural first fixture                          | Partial |
| P00B-T002 | [Capability inventory](../inventories/capabilities.md) passes 1 and 2 at `dd69b657` | Seed counts reproduced by an independent count; GUI actions enumerated from a closed table rather than by search; no reachability verification performed                                | Partial |
| P00B-T003 | [Identity inventory](../inventories/identities.md) passes 1 and 2 at `dd69b657`     | Pass 1 derived from the committed schema artifact; pass 2 read the code paths but did not execute them, so `IDN-0027` is a reading of the dispatch rather than an observed reproduction | Partial |

## Deviations

No deviations from the master plan are recorded.

Any scope, ordering, or contract change must link to an ADR or an explicit master-plan update. Do not bury architecture
changes in this tracker.

## Exit readiness

Status: Not ready

The formal review must evaluate every Phase 0B exit gate in the master plan and link direct evidence. Phase 10 may not
begin until that review is accepted.
