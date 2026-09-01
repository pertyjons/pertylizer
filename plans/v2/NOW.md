# Core V2: Current Work

Last updated: 2026-09-01

This file contains only active Core V2 state, blockers and next actions. Durable
contracts live in ADRs and specifications; completed Phase 3 coordination
history is indexed in
[`archive/phase-03/process-history.md`](archive/phase-03/process-history.md).

## Active streams

### Phase 4 — bounded lowering slice

[REV-P03](reviews/phase-03-exit-review.md) accepted the Phase 3 exit. The first
Phase 4 slice is deliberately narrower than a full A/B service:

- implement a pure `LegacyProjectLowerer` for one bounded current-project
  subset with deterministic diagnostics;
- permit one bounded in-process smoke render after the lowerer can represent
  that subset;
- keep V1 as the default and add no frontend or shared job surface.

Before that smoke render includes a saved pitched note, close P03-R003 with the
minimum typed pitch and velocity payload. Pure lowering work that does not
render such a note may proceed independently.

[ADR-0028](decisions/ADR-0028-long-running-job-contract.md) may remain
`Deferred` only for this closed scope. Accept it before shared
`RenderRequest`/`RenderResult`, multi-project A/B, streaming, progress,
cancellation, or any GUI, CLI or MCP render surface.

Next action: inventory the smallest current-project fixture that exercises one
track, one supported instrument graph and deterministic lowering diagnostics,
then write the lowerer's typed input/output boundary.

### Phase 0B — active in parallel

Phase 0B remains `Active, parallel`; Phase 10 still waits for its exit.

| Task | State | Current boundary |
|---|---|---|
| P00B-T001 | Complete | Closed 2026-08-29; 64 state entries are `Classified` and coverage-gated |
| P00B-T002 | Paused | Resume by assigning reachability and migration dispositions in the capability inventory |
| P00B-T003 | Active | Fill `Proposed V2 newtype/rule` for all 31 identity entries; this is the selected Phase 0B slice |
| P00B-T004–T007, P00B-T009 | Not started | Follow the frozen Phase 0B decomposition |
| P00B-T008 | Not started | Re-scope the former all-ADR task under `PROCESS.md` decision timing |

## Phase 3 residual obligations

Phase 3 is complete. Its exit review accepted these bounded residuals:

| ID | Residual | Pull-forward rule |
|---|---|---|
| P03-R001 | Sample-exact runtime loop wrap and per-pass note identity remain undecided in [ADR-0052](decisions/ADR-0052-loop-wrap-note-identity.md) | [ADR-0055](decisions/ADR-0055-refuse-unimplemented-loop-playback.md) refuses loop playback meanwhile. Resolve before any V2 loop consumer; Phase 9 cannot exit without it |
| P03-R002 | Current producer shares, event cap, release holds and live-ingress depth remain provisional | [ADR-0054](decisions/ADR-0054-staged-producer-capacity-calibration.md) measures each first real authored/internal producer and requires complete reselection before production live ingress |
| P03-R003 | Note events carry identity but not typed pitch and velocity | Add the minimum typed payload before Phase 4 renders its first saved pitched note; Phase 6 still owns full tuning and expression composition |
| P03-R004 | Numeric note-index and generation widths are safe by checked bounds and fail-closed exhaustion, but not endurance-qualified against a real live workload | Validate the widths before a production live adapter; generation exhaustion retires and reports instead of aliasing |

## Later-owned work

- Phase 5 owns the `LegacyPolyModuleAdapter` conversion-cost measurement and an
  executable guard for `SOUND-INV-012`'s closed renderer-control-flow claim.
- Phase 9 owns ADR-0022 acceptance against retained platform/adapter evidence,
  P03-R001 before loop playback or phase exit, P03-R004 before production live
  ingress, and ADR-0050 clause 8's release-hold redemption and activation-time
  minter ownership before activation can coexist with live ingress.
- ADR-0051's shared-gate ownership law is required before two producers can
  drive one scalar gate through activation/catch-up behavior.
- Phase 10E owns ADR-0039 and `LIMIT-0017`.
- Phase 0B still gates Phase 10.

## Current blockers

No blanket blocker remains on the bounded Phase 4 lowering slice. P03-R003
blocks its first saved pitched-note render, and ADR-0028 blocks expansion into a
shared or frontend-facing render job contract. The other residuals block only
their named consumers.
