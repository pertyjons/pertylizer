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

### Phase 6 — active since 2026-09-05

Activated by selection, as the Phase 5 exit said it would be. Its entry prerequisite is met:
[ADR-0025](decisions/ADR-0025-tuning-representation-and-ownership.md) is **Accepted**, so a
voice's pitch resolves through the prepared tuning the plan already holds rather than a second
model. Two decisions bind slices inside the phase rather than its entry, under `PROCESS.md`'s
decision-timing rule: the voice **allocation and stealing** policy — which voice a note-on
takes when none is free, and how the taken voice ends — is a delivered-behaviour decision no
record holds yet and is required before the slice that steals; and ADR-0026, the sample map
and zone model — a register entry with no record yet — is `Proposed` and required before the
sampler slice. Until the stealing decision, a note-on that finds no free voice is refused and
counted, which is the identity range's existing exhaustion path and fails closed.

What V2 has at the phase's start: one instance of the voice scope per plan, so every note a
producer plays lands on the same envelope and oscillator; note identity per occurrence with a
disjoint index range per producer sized by `simultaneous_notes`, admitted, minted off-thread
and released with an orphan count (`SOUND-INV-017`); the note payload's key and velocity
expanded to every declared destination in the played node's scope, through the prepared
tuning (`SOUND-INV-021`); one parameter slot per addressable parameter (`SOUND-INV-023`,
`SOUND-INV-024`); and a report that admits `active_voices` as a declared count nothing yet
derives. The phase turns the voice scope into a `VoicePlan` — one immutable prepared shape,
`N` instances of mutable state — and routes each occurrence to its own instance.

| Task | State | Current boundary |
|---|---|---|
| P06-S001 — `N` voice instances of one prepared voice plan | **Merged** 2026-09-05 (`7f9c5ed1`); `SOUND-INV-025` built, one independent read, eight defects repaired | The compiler instantiates the voice scope once per identity index of the producer that plays it — `simultaneous_notes` instances — sharing every prepared record and cloning none: `N` node slots per voice-scope node over one prepared record, per-instance state, per-instance parameter slots and buffers for voice-scope parameters (voice-local state, with a `SetParameter` fanning its override write out to every instance and a note's magnitudes landing on its own), and a compiler-inserted **accumulate** into one voice-sum region wherever a voice-scope output feeds a non-voice node. The renderer routes a note's gate and magnitudes to the instance its identity index names, so two overlapping notes sound as two voices. A note-on with no free index is refused and counted, as today. **Completion check:** a plan declaring one simultaneous note renders bit-identically to today (EVD-0013 and the `quantum_cost` digests reproduce); two overlapping notes render as the sum of two single-note renders, sample for sample; a release ends its own voice and no other; the report's mutable and prepared rows scale with `N` for state and slots and not at all for prepared records, and preparation allocates exactly what the rows charge; `max_active_voices` is admitted against the derived instance count rather than a declared one; the purity scan covers the accumulate kernel and the per-voice routing; mutation-verified. Real-time boundary and admission: the core Rust gate and one independent review apply |
| P06-S002 — voice stealing under a decided policy | **Merged** 2026-09-06 (`15adfbd3`); ADR-0058 accepted and reviewed with it, one independent read (agy), five defects repaired | Needs the allocation and stealing ADR first: which voice is taken (V1 offers oldest, quietest, lowest priority, same note, none), how the taken voice ends at a precise sample (a release-then-retrigger over the retirement crossfade, or a cut), and what a stolen voice's expression keeps or clears — one documented rule, tested at sample offsets |
| P06-S002b — stealing at the live boundary | Not started | ADR-0058 clause 6's second site: the live boundary's note-on offer takes a voice under the same policy when the producer is full. The minter must carry each live index's node, key and mint order so the boundary can choose the victim off-thread as stamping does; the fade, reset and delayed start are then the same events the compiled path stamps, and the taken note's later release is counted as `released_after_steal` at the boundary. Split from `P06-S002` because the boundary holds no record of open keys today |
| P06-S003 — per-note expression and the bend clause | Not started | ADR-0047 clause 9's reserved per-note event, `SOUND-INV-021`'s bend clause (a cents offset after resolution), and the rule that allocation, stealing, sustain, retrigger and release preserve or clear expression; note identity must still route expression after stealing and after a plan recompilation |
| P06-S004 — velocity composition | Not started | Inherits `P04-R001` before it builds: V1 applies one saved velocity under two independent sensitivities; V2 states the composition as a modulation under one of `SOUND-INV-023`'s laws on the velocity destination, and the lowering's `UnsupportedScope` mark comes off placed notes when a parity verdict can be offered |
| P06-S005 — the one-zone sampler on the prepared map/zone contract | Not started | Needs ADR-0026 accepted first: an immutable sample map with zones, key/velocity selection, root and tuning, playback region and a prepared sample reference, of which the native sampler selects exactly one zone without a per-note allocation or a single-sample API |
| P06-S006 — one tuning through every path | Not started | Built-in 12-TET and one non-12-TET mapping produce the same pitches through the live, sequenced, offline and analysis-facing paths; the prepared tuning contract already exists and this holds every consumer to it |
| P06-S007 — determinism under pressure | Not started | The exit's evidence: polyphonic output deterministic for a fixed seed and event stream under stealing pressure, and equivalent offline and live instance behaviour |

Inherited before it builds: `P04-R001` (velocity composition, `P06-S004`) and `P05-R001` (a
lowered level's smoothing policy, binding the lowering that first maps V1's amplifier level or
writes a V2 amplitude dynamically). `P05-R002` is Phase 10D's and does not bind this phase.


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

Two streams are active: Phase 6, with `P06-S001` and `P06-S002` merged and `P06-S002b` or
`P06-S003` next, and Phase 0B, with `P00B-T003` as its selected slice.

Next action: **build `P06-S002b`**, the live boundary's steal — the minter must carry each live
index's node, key and mint order so the boundary can choose a victim off-thread as stamping
does — or `P06-S003`, per-note expression and the bend clause; the two are independent.
