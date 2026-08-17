# Pertylizer Core V2 Status

| Field | Value |
|-------|-------|
| Last updated | 2026-08-15 |
| Active phases | 0B; Phase 1 is ready to start |
| Phase 0A review | [`REV-P00A`](reviews/phase-00a-exit-review.md), `Accepted` |
| Sound Core V2 implementation | Not started |

This file is a short index, not a work log or an independent source of task status. The
[Phase 0A tracker](phases/phase-00a-baseline-and-render-contracts.md),
[Phase 0B tracker](phases/phase-00b-inventories-and-project-contracts.md), decision
[register](ADR.md), and [exit review](reviews/phase-00a-exit-review.md) are authoritative for their respective data.
The workflow and authority rules are in [WORKING-AGREEMENT.md](WORKING-AGREEMENT.md).

## Current objective

Start the experimental Sound Core V2 crate against the accepted Phase 0A contracts. Continue Phase 0B independently;
it gates Phase 10 rather than Phase 1.

## Phase 0A summary

| Task | Current state | Authority |
|------|---------------|-----------|
| Reference corpus | Complete for Phase 0A; ten fixtures and one owned reproducibility gap | [P00A-T001](phases/phase-00a-baseline-and-render-contracts.md#p00a-t001--define-the-reference-v1-corpus-and-preservechange-manifest) |
| Comparison command | Complete | [P00A-T002](phases/phase-00a-baseline-and-render-contracts.md#completed-tasks) |
| Baseline measurements | Complete; EVD-0007 covers the current ten-case corpus | [P00A-T003](phases/phase-00a-baseline-and-render-contracts.md#completed-tasks) |
| Resource audit | Complete for Phase 0A; every row has the required proposed rule and diagnostic, and the probe passes | [P00A-T004](phases/phase-00a-baseline-and-render-contracts.md#p00a-t004--complete-the-fixed-limit-and-overflow-audit) |
| Initial host profile | Complete and `Current` for the Phase 1 field set | [P00A-T005](phases/phase-00a-baseline-and-render-contracts.md#p00a-t005--define-the-initial-hostprofile-and-renderlimits-contract) |
| Required Phase 0A decisions | Complete | [P00A-T006](phases/phase-00a-baseline-and-render-contracts.md#completed-tasks) |
| Exit review | Complete at `Accepted` | [REV-P00A](reviews/phase-00a-exit-review.md) |

## Next actions

1. Start the minimal Phase 1 vertical slice.
2. Continue Phase 0B independently.
3. Keep ADR-0039 and `LIMIT-0017` visible as Phase 10E work.

## Later work already owned

- Phase 3 owns renderer-ingress capacity, deferred-event storage, and the ADR-0001 clarification.
- Phase 4 owns the long-running job contract.
- Phase 0B owns project, identity, format, asset, and application-operation contracts before their named implementation
  gates.
