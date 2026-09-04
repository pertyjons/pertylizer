# Core V2: Current Work

Last updated: 2026-09-04

This file contains only active Core V2 state, blockers and next actions. Durable
contracts live in ADRs and specifications; completed Phase 3 coordination
history is indexed in
[`archive/phase-03/process-history.md`](archive/phase-03/process-history.md),
and Phase 4's durable record is [REV-P04](reviews/phase-04-exit-review.md)
together with its section in the [master plan](master-plan.md#phase-4-current-project-lowering-and-offline-ab-path).

## Phase 4 — closed and merged

[REV-P04](reviews/phase-04-exit-review.md) is **Accepted**: saved projects lower and render
through V2 from their own pinned bytes, at their own pitches and velocities. Its gate and the
roadmap outcome were amended on 2026-09-02 under `PROCESS.md`'s phase-exit rule, so the phase
delivers the V2 side of the headless comparison path rather than the join between the two
paths.
[ADR-0057](decisions/ADR-0057-refuse-parity-verdict-over-a-placed-note.md) owns that decision;
the two obligations it carries are active state and stay below. The branch was squash-merged
to `main` on 2026-09-04 after thirteen independent reads, the last two over the whole squash.

## Phase 4 residual obligations

Phase 4 is complete. Its exit review accepted these two residuals; `P04-R002` and `P04-R003` are
discharged rather than carried.

| ID | Residual | Pull-forward rule |
|---|---|---|
| P04-R001 | V1 applies one saved velocity twice, under two independent sensitivities; V2 applies it once as one scale on the envelope | Every lowering that places a note is marked `UnsupportedScope` and the A/B path refuses to compare it. Phase 6 owns the composition law and inherits this before it builds it. Until then nothing may issue a parity verdict over a **lowered** outcome that is not `Faithful` — no offline engine selection over saved projects, no corpus A/B batch. A harness that builds its own fixtures and never lowers, as EVD-0013's does, is outside the rule |
| P04-R004 | [ADR-0028](decisions/ADR-0028-long-running-job-contract.md) is `Deferred`: a *revisioned* job contract needs Phase 10A's canonical revision and Phase 10B's job capture | All three standing constraints hold until acceptance in Phase 10B. Constraint 3 refuses streaming, progress, cancellation, multi-project A/B and a shared render request/result as **task selections**, so that work does not proceed under another name |

## Active streams

### Phase 5 — active since 2026-09-04

Activated by selection, as the Phase 4 exit said it would be. Its entry prerequisite is met:
[ADR-0027](decisions/ADR-0027-observation-and-analyzer-ownership.md), observation and analyzer
ownership, is **Accepted** with the master plan's split ownership — a persisted analyzer node
owns authored intent only, a compiler-declared tap is the only subscribable point, the host owns
bounded lossy subscriptions admitted by the profile, analysis runs on workers, and one versioned
telemetry facade serves GUI, OSC and the visualizer. The declarative node API can therefore
make a declaration the single source for tap capability without deciding ownership by accident.

| Task | State | Current boundary |
|---|---|---|
| P05-S001 — one declaration for one kind | **Selected** 2026-09-04 | Collapse what `synth_engine_v2` says about the `Saw` kind — today four `match kind` arm sets in `node.rs` (`descriptor`, `ports`, `prepared_payload_bytes`, `state_payload_bytes`), the `parameters::SAW_*` ids in `ir.rs`, and the kernel binding — into **one declaration** that those functions derive from. `Saw` because it is Phase 4's own kind and its frequency is a pitch destination, so `SOUND-INV-021`'s magnitudes are exercised. **Completion check:** (1) the `SOUND-INV-012` guard below lands first and gets the invariant its missing conformance row; (2) a test asserts the derivation for `Saw` — changing one field of the declaration changes the descriptor, the port set and both byte counts together, and no `Saw` arm remains in those four functions — mutation-verified in both directions; (3) the sawtooth renders bit-identically: EVD-0012's and EVD-0013's digests reproduce and the workspace tests pass. **Out of scope, by decision:** `ParamSpec` modulation laws, central parameter slots, the legacy adapter, discovery surfaces and a second kind — each is a later slice, and a field no consumer reads is not built here. Real-time boundary: the kernel binding sits in the purity-scanned region, so the core Rust gate and one independent review apply |
| Executable guard for `SOUND-INV-012` | In P05-S001, first | The Sound Core contract's closed renderer-control-flow claim has no conformance row. The loop is already kind-blind — nothing under `render/` or in `node/kernels.rs` names an `IrNodeKind` — so the guard is a source scan beside `render_loop_purity`'s existing ones asserting exactly that, mutation-verified by naming a kind in the hot path. It precedes the declaration change because that change is the first that could reopen the claim |
| `LegacyPolyModuleAdapter` conversion-cost measurement | Not started | Owed by the Phase 5 work list's adapter bullet: the adapter is transitional and measured separately, before the exit gate's "not required by the renderer itself" can be judged. Not before a second native kind exists to adapt against |

Inherited before it builds: nothing from Phase 4's residuals binds Phase 5 — `P04-R001` is
Phase 6's and `P04-R004` is Phase 10A/10B's.

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

**None for Phase 5's entry.** ADR-0027 is accepted and Phase 4 is merged. Phase 4's two
residuals block their own first consumers — a parity verdict over a placed note, and the first
shared render surface — and neither is Phase 5 work. The Phase 3 residuals block only their
named consumers.

Two streams are active: Phase 5, with `P05-S001` selected, and Phase 0B, with `P00B-T003`
as its selected slice.

Next action: **build `P05-S001`**, guard first, on a branch off `main`; its completion check is
in the table above and is what its commit and review are judged against.
