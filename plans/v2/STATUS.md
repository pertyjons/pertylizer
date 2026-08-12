# Pertylizer Core V2 Status

| Field                  | Value                                    |
|------------------------|------------------------------------------|
| Last updated           | 2026-08-12                               |
| Documentation stage    | Workflow accepted; inventories at pass 2 |
| Master plan status     | Proposed and architecture-audited        |
| Active migration phase | 0A and 0B, both `Active` in parallel     |
| Decision records       | 3 of 37 drafted, 2 accepted              |

This is a current-state dashboard, not a work log. Replace stale information instead of appending a chronology.
Historical conclusions belong in ADRs, evidence records, phase reviews, and Git history.

## Current objective

Establish a reviewable Phase 0A baseline and the contracts required to begin the experimental Sound Core V2 path without
weakening or silently replacing the V1 production path. Phase 0B's migration inventories run alongside that work; they
gate Phase 10, not Phase 1.

## Current state

- The consolidated [architecture and migration plan](master-plan.md) exists.
- The V2 documentation responsibilities, registers, and templates are defined, reviewed, and **accepted**. All 23
  documents were checked: every internal Markdown link and heading anchor resolves, both phase trackers conform to
  [templates/phase.md](templates/phase.md), and the identifier series are consistent across README, registers, and
  trackers. No authority conflict was found.
- The decision topics are registered in [ADR.md](ADR.md), now 37 after one split. Three have individual records —
  [ADR-0001](decisions/ADR-0001-internal-render-quantum.md) (render quantum semantics),
  [ADR-0021](decisions/ADR-0021-host-profile-and-admission-policy.md) (host profile and admission), and
  [ADR-0037](decisions/ADR-0037-render-quantum-value.md) (quantum frame count). **ADR-0001 and ADR-0021 are `Accepted`**
  after three review passes; ADR-0037 remains `Proposed` pending measurement. ADR-0037 is a split of the master plan's
  topic 1, recorded as a deviation in the Phase 0A tracker; both quantum records are required `Accepted` at the Phase 0A
  exit, so the split does not weaken the gate.
- **The master plan is synchronized with ADR-0001 and ADR-0021.** Part VII topic 1 and the Phase 0A exit gate name both
  quantum records. The quantum is no longer configurable, and `maximum_block_size` is owned only by `HostProfile`
  instead of being duplicated in `RenderConfig`. These changes landed with the ADRs' acceptance, per the same-change
  rule in [README.md](README.md#sources-of-truth).
- **The accepted contracts now unblock execution.** ADR-0001 fixes quantum, carry, end-of-stream, event-horizon, and
  latency semantics in terms of `Q`. ADR-0021 fixes admission behavior, seven configuration owners, an explicit lossy
  retention/presentation class, and a terminal `needs_reprepare` policy for an oversized host callback.
- All four [inventories](inventories/README.md) have completed two audit passes against `dd69b657` and are `Active`:
  74 `LIMIT`, 59 `STATE`, 55 `CAP`, and 31 `IDN` entries. Pass 1 was a census from schemas and constant names; pass 2
  read the enforcing code, which resolved every gate-blocking question and **disproved three pass-1 hypotheses** — those
  are corrected in place rather than appended. Each ledger records both methods and what each is blind to; none is
  `Current`.
- Phase 0A and 0B implementation and evidence are not claimed complete. No code has been written for V2.
- V2 implementation status must be established from repository evidence before this dashboard makes code-level
  completion claims.

## Next actions

The inventories have stopped being search-limited. What each still lacks is either a decision or an executable check, so
the next actions are those, not a pass 3.

1. **Define the reference render corpus** (P00A-T001) and the comparison result format (P00A-T002). Both P00A-T003 and
   ADR-0037's proxy measurement are defined over that corpus, so nothing measurable starts before it exists.
2. **Sweep all 74 resource-inventory entries** under accepted ADR-0021, assigning both axes — one of six failure classes
   and one of seven configuration owners — plus the rule and diagnostic. Every `Unknown`-class entry must reach a
   terminal class as part of it.
3. **Run ADR-0037's V1 proxy measurement** — render the corpus at 32/64/128/256 frames by varying `BUFFER_SIZE` in
   `arrangement_render.rs:51`, record it as an `EVD` record, and apply the record's ordered rule table. An inconclusive
   result is a real possible outcome, not a failure of the measurement.
4. **Open ADR-0032** (`SampleTime` and event timestamps), the remaining record the Phase 0A exit gate requires
   `Accepted`. ADR-0001 now fixes the epoch and the late-event rule, so ADR-0032 refines the representation on top of
   that rather than inventing it.
5. **Open ADR-0014.** The identity ledger's central finding is that the module id encodes its type at *runtime*
   (`IDN-0029`), not merely on disk, and that a module's script PRNG seed is derived from its instance number — so
   renumbering is audible, not just referential.
6. **Write the first round-trip fixture (P00B-T005) for `STATE-0004`** — changing the focused instrument changes the
   saved file while no dirty term observes it. It is the cheapest executable check the ledgers produced.
7. **Record both audit passes as `EVD` records** so the ledgers' claims are reproducible rather than asserted. No value
   in the resource ledger has been measured; all of them are read from source.

## Documentation-workflow review notes

Two conformance details were noted during the acceptance review and left as they are, because both trackers carry the
information the template asks for:

- Neither phase tracker uses the template's exact `Required decisions` column set. Phase 0A uses
  `Required at Phase 0A exit` / `Later acceptance gate`; Phase 0B uses `Topic` / `Earlier deadline, if any` and states
  the required status in prose above the table rather than in a column.
- The trackers' task tables use a `Primary record` column where the template has `ADRs/inventories`.

## Blockers

No documentation blocker is currently recorded. Open decisions become blockers only when a phase task or exit gate
requires them to be accepted.

## Phase overview

| Phase | Name                                     | Status      | Tracker                                                          | Exit review |
|-------|------------------------------------------|-------------|------------------------------------------------------------------|-------------|
| 0A    | Baseline, limits, and render contracts   | Active      | [Tracker](phases/phase-00a-baseline-and-render-contracts.md)     | Not created |
| 0B    | Inventories and project contracts        | Active      | [Tracker](phases/phase-00b-inventories-and-project-contracts.md) | Not created |
| 1     | Experimental Sound Core V2 crate         | Not started | Create when activated                                            | Not created |
| 2     | Minimal compiled voice graph             | Not started | Create when activated                                            | Not created |
| 3     | Sample-accurate scheduler                | Not started | Create when activated                                            | Not created |
| 4     | V1 lowering and offline A/B              | Not started | Create when activated                                            | Not created |
| 5     | Declarative node and parameter API       | Not started | Create when activated                                            | Not created |
| 6     | Polyphony and instrument runtime         | Not started | Create when activated                                            | Not created |
| 7     | YAMS and unified modulation              | Not started | Create when activated                                            | Not created |
| 8     | Mixer, buses, effects, and latency       | Not started | Create when activated                                            | Not created |
| 9     | Live integration and plan swapping       | Not started | Create when activated                                            | Not created |
| 10A   | Canonical project model and identity     | Not started | Create when activated                                            | Not created |
| 10B   | Application operations and transactions  | Not started | Create when activated                                            | Not created |
| 10C   | History, dirty state, save, and recovery | Not started | Create when activated                                            | Not created |
| 10D   | Project Format V2, assets, conversion    | Not started | Create when activated                                            | Not created |
| 10E   | MCP, CLI, import, and service migration  | Not started | Create when activated                                            | Not created |
| 11    | GUI and workflow migration               | Not started | Create when activated                                            | Not created |
| 12    | Default cutover and V1 retirement        | Not started | Create when activated                                            | Not created |

Phase 0A gates Phase 1; Phase 0B gates Phase 10 and runs in parallel with Phases 1-4. Phase 10 has no gate of its own
and is complete when 10A–10E are. Every sub-phase has its own tracker, exit gate, and review.

## Status maintenance

Update this file when the active task, phase, blocker, or next action changes. Do not mark a phase `Complete` until its
formal exit review is accepted. Do not list speculative implementation progress as verified status.
