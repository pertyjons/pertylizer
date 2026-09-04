# Core V2: Current Work

Last updated: 2026-09-02

This file contains only active Core V2 state, blockers and next actions. Durable
contracts live in ADRs and specifications; completed Phase 3 coordination
history is indexed in
[`archive/phase-03/process-history.md`](archive/phase-03/process-history.md),
and Phase 4's durable record is [REV-P04](reviews/phase-04-exit-review.md)
together with its section in the [master plan](master-plan.md#phase-4-current-project-lowering-and-offline-ab-path).

## Phase 4 — closed

[REV-P04](reviews/phase-04-exit-review.md) is **Accepted**: saved projects lower and render
through V2 from their own pinned bytes, at their own pitches and velocities. Its gate and the
roadmap outcome were amended on 2026-09-02 under `PROCESS.md`'s phase-exit rule, so the phase
delivers the V2 side of the headless comparison path rather than the join between the two
paths.
[ADR-0057](decisions/ADR-0057-refuse-parity-verdict-over-a-placed-note.md) owns that decision;
the two obligations it carries are active state and stay below.

## Phase 4 residual obligations

Phase 4 is complete. Its exit review accepted these two residuals; `P04-R002` and `P04-R003` are
discharged rather than carried.

| ID | Residual | Pull-forward rule |
|---|---|---|
| P04-R001 | V1 applies one saved velocity twice, under two independent sensitivities; V2 applies it once as one scale on the envelope | Every lowering that places a note is marked `UnsupportedScope` and the A/B path refuses to compare it. Phase 6 owns the composition law and inherits this before it builds it. Until then nothing may issue a parity verdict over a **lowered** outcome that is not `Faithful` — no offline engine selection over saved projects, no corpus A/B batch. A harness that builds its own fixtures and never lowers, as EVD-0013's does, is outside the rule |
| P04-R004 | [ADR-0028](decisions/ADR-0028-long-running-job-contract.md) is `Deferred`: a *revisioned* job contract needs Phase 10A's canonical revision and Phase 10B's job capture | All three standing constraints hold until acceptance in Phase 10B. Constraint 3 refuses streaming, progress, cancellation, multi-project A/B and a shared render request/result as **task selections**, so that work does not proceed under another name |

## Active streams

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
| P03-R003 | Note events carry identity but not typed pitch and velocity | **Closed.** A note-on carries a validated key and velocity, resolves the key through the plan's prepared tuning, expands to the control writes its scope declares, and a saved note's own magnitudes reach it. Phase 6 still owns the full composition law, which the work list is explicit this does not decide |
| P03-R004 | Numeric note-index and generation widths are safe by checked bounds and fail-closed exhaustion, but not endurance-qualified against a real live workload | Validate the widths before a production live adapter; generation exhaustion retires and reports instead of aliasing |

## Later-owned work

- Phase 5 owns the `LegacyPolyModuleAdapter` conversion-cost measurement and an
  executable guard for `SOUND-INV-012`'s closed renderer-control-flow claim.
- Phase 6 owns `P04-R001`'s composition law, and `SOUND-INV-021`'s **bend** clause, which is
  not built: a per-note bend is a continuous offset in cents applied after resolution, carried
  by the event ADR-0047 clause 9 reserves, and neither the event nor the offset exists. Nothing
  in Phase 4 reached it, so it waits for its first consumer.
- Phase 9 owns ADR-0022 acceptance against retained platform/adapter evidence,
  P03-R001 before loop playback or phase exit, P03-R004 before production live
  ingress, and ADR-0050 clause 8's release-hold redemption and activation-time
  minter ownership before activation can coexist with live ingress.
- ADR-0051's shared-gate ownership law is required before two producers can
  drive one scalar gate through activation/catch-up behavior.
- Phase 10A owns the canonical project revision `P04-R004` waits for; Phase 10B owns ADR-0028's
  acceptance and the revision-pinned job service.
- Phase 10E owns ADR-0039 and `LIMIT-0017`.
- Phase 0B still gates Phase 10.

## Current blockers

**None for Phase 4, which is closed.** Its two residuals block their own first consumers — a
parity verdict over a placed note, and the first shared render surface — and neither is work in
progress. The Phase 3 residuals block only their named consumers.

Phase 0B remains the one active stream, with `P00B-T003` as its selected slice.

Next action: **select the next stream.** This exit removes Phase 4 as Phase 5's dependency; it
does not make Phase 5 ready. Phase 5 has its own entry prerequisite — ADR-0027, observation and
analyzer ownership, is `Proposed` and the [master plan](master-plan.md#phase-5-declarative-node-and-parameter-api)
requires it `Accepted` before Phase 5 implementation begins, so the declarative node API does
not make GUI buffers or protocol subscriptions part of authored DSP state. Activating Phase 5
therefore starts with that decision. Activation is a selection rather than a consequence of
this exit, so it is recorded here when it is made.
