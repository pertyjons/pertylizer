# Core V2: Current Work

Last updated: 2026-09-05

This file contains only active Core V2 state, blockers and next actions. Durable
contracts live in ADRs and specifications; completed Phase 3 coordination
history is indexed in
[`archive/phase-03/process-history.md`](archive/phase-03/process-history.md),
and Phase 4's durable record is [REV-P04](reviews/phase-04-exit-review.md)
together with its section in the [master plan](master-plan.md#phase-4-current-project-lowering-and-offline-ab-path).
Phase 5's durable record is [REV-P05](reviews/phase-05-exit-review.md) with its section in the
[master plan](master-plan.md#phase-5-declarative-node-and-parameter-api); its slice table is
archived in [`archive/phase-05/`](archive/phase-05/INDEX.md).

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

## Phase 5 — closed

[REV-P05](reviews/phase-05-exit-review.md) is **Accepted** on `1b1252f4`: ten node kinds are
declared once, discovery and validation derive from the same declaration, every parameter
composes in one slot under a declared law and ramps as a declared segment, a declared tap is
the plan's only observation point, and the host's bounded lossy subscription over it changes
no sample. Its gate was corrected on 2026-09-05 — the legacy adapter withdrawn, not deferred —
and the phase exits with two residuals, below. Nine slices, each merged on one independent read
at the user's standing decision; the exit review's first draft was rejected by an independent
read for an unevidenced observation bullet, and the ninth slice is what evidenced it.

## Phase 5 residual obligations

| ID | Residual | Pull-forward rule |
|---|---|---|
| P05-R002 | The exit gate's observation bullet claimed that observation changes no semantic project digest, and that digest is defined only in Phase 10D; the clause is carried, not claimed. ADR-0027 clause 2 already keeps every observation field out of the serialized project. | Phase 10D, when it defines the semantic project digest: its digest test holds that opening, closing or saturating an observation changes no digest, before the digest is used for round-trip or migration checks. Fails closed: no digest exists to misreport |
| P05-R001 | The smoothing policy of a lowered level, owned by the first lowering that maps V1's *amplifier* level onto a V2 parameter, or first writes a V2 amplitude dynamically. | Every declared `Smoothing` is `None`, so a write is a step. That is V1 parity for the one quantum-rate control V2 has: the lowerer maps V1's *oscillator* level onto the V2 amplitude as a static base (`lowering/graph.rs`), and V1's oscillator applies that level unsmoothed (`synth_modules/src/oscillator.rs`, `effective_level`). The control V1 does de-zipper — a linear ramp per block landing exactly on the target — is its *amplifier* level (`synth_modules/src/amplifier.rs`), which the lowerer refuses unless unity because V2's amplifier has no level of its own. The decision the user took on 2026-09-05 is therefore: no declared policy changes now; the parameter that first receives V1's amplifier level, or the first dynamic write to a V2 amplitude, decides its `Smoothing` against V1's per-block ramp with an A/B to measure. Fails closed: nothing is silently ramped. The mechanism is built and mutation-verified (`P05-S007b`), so the decision is a one-line declaration change. An independent read corrected this row's first form, which named the oscillator's own lowering as the trigger — a point already passed |

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

**None for Phase 6's entry.** ADR-0025, its prerequisite, is accepted, and Phase 5 is closed.
Residuals bind later work by name rather than blocking Phase 6's entry: `P05-R002` binds Phase
10D's digest; `P04-R001` — V1's
two velocity sensitivities — before it builds a velocity composition; `P05-R001` — a lowered
level's smoothing policy — before a lowering maps V1's amplifier level onto a V2 parameter or
writes a V2 amplitude dynamically; and Phase 3's residuals, which block only their named
consumers. `P04-R004` binds the first shared render surface, which is Phase 10B's.

One stream is active: Phase 0B, with `P00B-T003` as its selected slice. Phase 5 is closed.

Next action: **activate Phase 6** — polyphony and instrument runtime — by selecting its
first slice. Its entry prerequisite is met: ADR-0025 is `Accepted`. It inherits `P04-R001`
(V1's two velocity sensitivities) before it builds a velocity composition, and `P05-R001`
binds the lowering that first maps a V1 amplifier level.
