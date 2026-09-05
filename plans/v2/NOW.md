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
| P05-S001 — one declaration for one kind | **Merged** to `main` 2026-09-04 as `51d3cf45` | `NodeDeclaration` exists and `Saw` is declared through it; `SOUND-INV-012` has its conformance row in the [Sound Core contract](specs/spec-sound-core-render-contract.md#conformance-tests), which is the durable record of what the slice holds |
| P05-S002 — the first playable declared kind | **Merged** to `main` 2026-09-04 as `120a8e98` | `Envelope` is declared through `NodeDeclaration` with its note control and velocity destination; the form scan takes a stated list of declared kinds |
| P05-S003 — the remaining sources | **Merged** to `main` 2026-09-04 as `b7200890` | `Sine`, `Silence`, `Constant` and `Impulse` declared; the scan exempts `prepare`'s arms by position |
| P05-S004 — the kinds with inputs | **Merged** to `main` 2026-09-04 as `36c132fd` | `Amplifier`, `Gain` and `Filter` declared, so every kind but the output node is declared through `NodeDeclaration` |
| P05-S005 — the declaration owns preparation | **Merged** to `main` 2026-09-04 as `b58602a5` | Each declaration names its `prepare`; a declaration handed another kind's IR is refused as `CompileError::DeclaredForAnotherKind` |
| P05-S006 — the declaration describes its parameters, and discovery derives from it | **Merged** to `main` 2026-09-04 as `d7d52375` | Typed defaults, `NodeKindId`, and `node::catalog()` derived from the declarations — the exit gate's second bullet in V2's form |
| P05-S007a — the parameter slot: laws and layers | **Merged** 2026-09-04 (`8a7e81d6`) | `SOUND-INV-023` built: `ModulationLaw` per `ControlSpec`, `render::slot` composing base, override and modulation under the law and the type's clamp, every write path in the loop — quantum-rate apply, sample-positioned parameter, note gate and magnitudes, adoption gate-downs, catch-up — reaching a kernel through the slot, the catalog presenting the law, a not-modulatable control compiling to no slot. Twelve mutations caught; EVD-0013 and the three `quantum_cost` digests bit-identical. The modulation sum's producer is Phase 7's; a test-only seam writes it until then |
| P05-S007b — the parameter slot: the ramp segment | **Merged** 2026-09-04 (`f55f9422`) | `SOUND-INV-024` built: `Smoothing` per `ControlSpec` (every declared policy `None`, renders bit-identical), the segment in `SlotState` advanced once per quantum into a per-slot control buffer before the schedule walk, the sine and sawtooth reading their amplitude per frame from it, node state without a quantum-rate control, adoption seeding every slot so a catch-up never ramps, the buffers charged to `mutable_state_bytes`. Open: whether the amplitude de-zippers over a quantum as V1's level does — a delivered-behaviour decision for the user. Slice 8 — latency, tail, reset, cost and tap capability in the declaration — follows |
| P05-S008 — the declared tap: observation as a compiler artifact | **Merged** 2026-09-05 (`ff5d47c6`) | Build `SOUND-INV-022`: a node kind's declaration is the only source of tap capability — a `Monitor` kind that passes its signal through unchanged and declares one tap on its output, with the tap's data type, rate and cost — and the compiled plan carries a tap table derived from its nodes' declarations, addressed stably by node and declared tap, admitted at compilation against `max_observation_taps` in place of the authored `PlanDeclarations::taps` list, present whether or not anything subscribes. **Completion check:** the invariant's conformance row filled — a tap exists only through a declaration, a plan's tap table and its admitted count derive from the nodes, a plan with a monitor renders the same samples as the same plan without it (passivity), and the taps exist with no subscriber — mutation-verified, renders bit-identical. Owed onward, not built here: the host subscription over the tap (`HOST-INV-023`, its first consumer is Phase 9's live host or Phase 10E's facade) and the declaration's latency, tail, reset and scope fields, which no consumer in this phase reads. The phase exit review follows |
| P05-S009 — the host subscription over a declared tap | **Built** 2026-09-05 on `feat/v2-phase5-s009`, under review | Build `HOST-INV-023`'s reachable half, so the exit gate's fifth bullet can be tested rather than argued: a host-owned `ObservationSubscriptions` store, prepared from the profile, that admits a subscription to one declared `TapSlot` of one compiled plan — refusing a slot from another plan, an index the plan has no tap for, a second subscription on one tap, and more subscriptions than the plan has taps — and owns for each a preallocated ring of `telemetry_ring_frames` × the port's channels, written by the renderer once per quantum from the tapped region after the schedule walk, evicting the oldest frames when the reader is behind and counting what was evicted; a read returns the frames, the frames dropped since the last read and how far behind the newest quantum the reader stands. The store is handed to the render call the way the ingress store is, so it can neither fail nor change a compilation or a plan. **Completion check:** `HOST-INV-023`'s conformance row filled and the exit gate's fifth bullet evidenced — the monitored voice rendered with no subscriber, one subscriber and a saturated subscriber is bit-identical in every case; the reader that keeps up reads exactly the frames the output carried; the saturated reader's drop count is exactly what it did not read; the refusals name their reason; the push is in the purity region and allocates nothing — mutation-verified, renders bit-identical. Owed onward: dynamic subscribe and unsubscribe while rendering across a thread boundary, decimation and the versioned telemetry facade (Phase 9 and Phase 10E). Real-time boundary: the core Rust gate and one independent review apply |
| Executable guard for `SOUND-INV-012` | **Built** in P05-S001 | The invariant has its conformance row now. Kind-blindness of the region was already held by `the_render_loop_makes_no_topology_or_naming_decision`; what was missing and is added is `the_render_loop_dispatches_every_node_through_one_site` — exactly one `Kernel::run` site in the hot path and no kernel called by name — mutation-verified by a second dispatch line and by a direct kernel call |
| `LegacyPolyModuleAdapter` conversion-cost measurement | **Withdrawn** 2026-09-05 by the user | No adapter is built: a V1 module the lowerer cannot map to a native kind is refused under `LOWER` rather than adapted, so an adapter would have no consumer and its measurement would measure nothing a phase reads. Recorded as a correction of the phase plan — [Gate correction](master-plan.md#gate-correction-2026-09-05) — not as a residual; the exit bullet is rewritten to say no adapter exists |
| P05-R001 — the smoothing policy of a lowered level | **Residual**, owned by the first lowering that maps V1's *amplifier* level onto a V2 parameter, or first writes a V2 amplitude dynamically | Every declared `Smoothing` is `None`, so a write is a step. That is V1 parity for the one quantum-rate control V2 has: the lowerer maps V1's *oscillator* level onto the V2 amplitude as a static base (`lowering/graph.rs`), and V1's oscillator applies that level unsmoothed (`synth_modules/src/oscillator.rs`, `effective_level`). The control V1 does de-zipper — a linear ramp per block landing exactly on the target — is its *amplifier* level (`synth_modules/src/amplifier.rs`), which the lowerer refuses unless unity because V2's amplifier has no level of its own. The decision the user took on 2026-09-05 is therefore: no declared policy changes now; the parameter that first receives V1's amplifier level, or the first dynamic write to a V2 amplitude, decides its `Smoothing` against V1's per-block ramp with an A/B to measure. Fails closed: nothing is silently ramped. The mechanism is built and mutation-verified (`P05-S007b`), so the decision is a one-line declaration change. An independent read corrected this row's first form, which named the oscillator's own lowering as the trigger — a point already passed |

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

Two streams are active: Phase 5, with `P05-S009` selected, and
Phase 0B, with `P00B-T003` as its selected slice.

Next action: **build `P05-S009`** on a branch off `main`. The exit review's first draft
was blocked by an independent read: the gate's fifth bullet cannot pass without a
subscription surface, and `HOST-INV-023` assigns building that surface to this phase. The
user chose to build it over amending the gate. `REV-P05` follows the slice.
