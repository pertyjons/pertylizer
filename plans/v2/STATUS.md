# Pertylizer Core V2 Status

| Field                  | Value                             |
|------------------------|-----------------------------------|
| Last updated           | 2026-08-12                        |
| Documentation stage    | Initial structure                 |
| Master plan status     | Proposed and architecture-audited |
| Active migration phase | Pre-Phase 0A preparation          |

This is a current-state dashboard, not a work log. Replace stale information instead of appending a chronology.
Historical conclusions belong in ADRs, evidence records, phase reviews, and Git history.

## Current objective

Establish a reviewable Phase 0A baseline and the contracts required to begin the experimental Sound Core V2 path
without weakening or silently replacing the V1 production path. Phase 0B's migration inventories run alongside that
work; they gate Phase 10, not Phase 1.

## Current state

- The consolidated [architecture and migration plan](master-plan.md) exists.
- The V2 documentation responsibilities, registers, and templates are defined.
- The 36 open decision topics are registered in [ADR.md](ADR.md), but none is accepted by this documentation setup.
- Phase 0A and 0B implementation and evidence are not claimed complete.
- V2 implementation status must be established from repository evidence before this dashboard makes code-level
  completion claims.

## Next actions

1. Review and accept the documentation workflow in this directory.
2. Begin the resource-limit inventory (Phase 0A) and the three migration inventories (Phase 0B) under
   [inventories/](inventories/README.md).
3. Define the reference render corpus and comparison result format as evidence records.
4. Start individual ADRs only when their decisions are actively investigated.
5. Select the first bounded task in
   [phase-00a-baseline-and-render-contracts.md](phases/phase-00a-baseline-and-render-contracts.md).

## Blockers

No documentation blocker is currently recorded. Open decisions become blockers only when a phase task or exit gate
requires them to be accepted.

## Phase overview

| Phase | Name                                     | Status      | Tracker                                                          | Exit review |
|-------|------------------------------------------|-------------|------------------------------------------------------------------|-------------|
| 0A    | Baseline, limits, and render contracts   | Not started | [Tracker](phases/phase-00a-baseline-and-render-contracts.md)     | Not created |
| 0B    | Inventories and project contracts        | Not started | [Tracker](phases/phase-00b-inventories-and-project-contracts.md) | Not created |
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

Phase 0A gates Phase 1; Phase 0B gates Phase 10 and runs in parallel with Phases 1-4. Phase 10 has no gate of its
own and is complete when 10A–10E are. Every sub-phase has its own tracker, exit gate, and review.

## Status maintenance

Update this file when the active task, phase, blocker, or next action changes. Do not mark a phase `Complete` until its
formal exit review is accepted. Do not list speculative implementation progress as verified status.
