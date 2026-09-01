# Phase 3 Process History

This is a compact index of coordination material removed from `NOW.md` on
2026-09-01. It is historical and non-authoritative. The complete pre-reset text
is recoverable exactly from revision `8d6c9075`, which is an ancestor of `main`,
with:

```bash
git show 8d6c9075:plans/v2/NOW.md
```

Durable outcomes remain at their stable ADR, specification, evidence and review
paths. Current task state remains in [`NOW.md`](../../NOW.md).

## Retired API-approval ledger

The maintainer approved these experimental `synth_engine_v2` Rust API changes
during Phase 3. The exact signatures and variant inventories are in the source
snapshot above.

| Date | Approved scope |
|---|---|
| 2026-08-26 | Producer-share and ingress-store changes in `profile`; note identity and compiled-payload signatures and variants; admitted compiled-stream signatures and refusal variants; the loop-density diagnostic variant |
| 2026-08-27 | Transport-activation signatures and refusal variants |
| 2026-08-28 | Publication occupancy accessors count charges rather than only batch entries |
| 2026-08-31 | `TimeSource::Simulated` with the deterministic simulated-ingress producer |
| 2026-09-01 | Live-ingress activation/store refusals, unrepresentable event amounts, horizon refusal and foreign-slot refusal |

These approvals were bounded to the experimental crate and its repository-local
development/test consumers. They never covered persisted formats, manifests,
wire or protocol contracts, production dependencies, external consumers or
unsafe code.

The 2026-09-01 process reset supersedes the former per-signature approval rule
with the standing experimental-API approval in the repository instructions.
That rule carries the same exclusions and requires affected repository-local
consumers to change in the same commit.

## Parked findings retained by active owners

| Finding from the retired coordination log | Current owner |
|---|---|
| Runtime loop wrapping needs sample-exact placement and a coherent per-pass note identity; quantum-granular designs were refuted | [ADR-0052](../../decisions/ADR-0052-loop-wrap-note-identity.md), residual P03-R001 and [ADR-0055](../../decisions/ADR-0055-refuse-unimplemented-loop-playback.md)'s fail-closed guard |
| The provisional event partition cannot be calibrated against authored-runtime and renderer-internal producers before those producers exist | [ADR-0054](../../decisions/ADR-0054-staged-producer-capacity-calibration.md), residual P03-R002 |
| Note identity landed without pitch and velocity payload semantics | Residual P03-R003 in [`master-plan.md`](../../master-plan.md), required by the first saved pitched-note lowering slice |
| Note index and generation widths had safety relations but no representative live endurance evidence | Residual P03-R004 in [`master-plan.md`](../../master-plan.md), required before production live ingress |
| A real producer/consumer handoff is not modeled by the borrowed simulated-ingress fixture | Phase 9 live integration; ADR-0022 still owns hardware-time mapping and live-timing qualification |
| Tempo-ramp behavior intentionally differs from V1 | [ADR-0049](../../decisions/ADR-0049-tempo-ramp-law.md) and its Phase 4 A/B comparison category |

## Review chronology

The retired log contained detailed iteration-by-iteration review narratives.
Their accepted conclusions live in ADR-0043 through ADR-0053, the Phase 3 EVDs,
and the current sound/host specifications. Failed drafts and reviewer counts are
not active task state; use the immutable source snapshot above when historical
reconstruction is necessary.
