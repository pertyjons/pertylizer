# Core V2: Current Work

Last updated: 2026-09-12

This file contains only active Core V2 state, blockers and next actions. Durable
contracts live in ADRs and specifications; completed Phase 3 coordination
history is indexed in
[`archive/phase-03/process-history.md`](archive/phase-03/process-history.md),
and Phase 4's durable record is [REV-P04](reviews/phase-04-exit-review.md)
together with its section in the [master plan](master-plan.md#phase-4-current-project-lowering-and-offline-ab-path).
Phase 5's durable record is [REV-P05](reviews/phase-05-exit-review.md) with its section in the
[master plan](master-plan.md#phase-5-declarative-node-and-parameter-api); its slice table is
archived in [`archive/phase-05/`](archive/phase-05/INDEX.md). Phase 6's durable record is
[REV-P06](reviews/phase-06-exit-review.md) with its section in the
[master plan](master-plan.md#phase-6-polyphony-and-instrument-runtime); its slice table is archived
in [`archive/phase-06/`](archive/phase-06/INDEX.md). Phase 7's durable record is
[REV-P07](reviews/phase-07-exit-review.md) with its section in the
[master plan](master-plan.md#phase-7-yams-mod-grid-and-unified-modulation); its slice table is
archived in [`archive/phase-07/`](archive/phase-07/INDEX.md).

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
| P04-R001 | V1 applies one saved velocity twice, under two independent sensitivities; V2 applied it once as one scale on the envelope | **Closed** by `P06-S004` under [ADR-0059](decisions/ADR-0059-velocity-composition.md): a voice has two velocity destinations, each with its own sensitivity and V1's formula, the lowerer carries `vel_sens` and `velocity_amp_sensitivity` to them, and a placed note no longer names velocity as unrepresented. `LOWER-INV-003`'s general rule stands: nothing may issue a parity verdict over a **lowered** outcome that is not `Faithful`, and any remaining timing or lifetime marker still yields `UnsupportedScope`; S007's fully represented single-note control admits comparison |
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
| P05-R001 | The smoothing policy of a lowered level, owned by the first lowering that maps V1's *amplifier* level onto a V2 parameter, or first writes a V2 amplitude dynamically. **Half decided by `P08-S001`**: the mix channel's fader is the first dynamic write to a V2 amplitude and declares `Smoothing::None`, because V1 applies the instrument fader as one gain per block in `mix_channel_busses` and never ramps it (`SOUND-INV-031`). The amplifier-level half still binds the first slice that maps V1's amplifier level. | Every declared `Smoothing` is `None`, so a write is a step. That is V1 parity for the one quantum-rate control V2 has: the lowerer maps V1's *oscillator* level onto the V2 amplitude as a static base (`lowering/graph.rs`), and V1's oscillator applies that level unsmoothed (`synth_modules/src/oscillator.rs`, `effective_level`). The control V1 does de-zipper — a linear ramp per block landing exactly on the target — is its *amplifier* level (`synth_modules/src/amplifier.rs`), which the lowerer refuses unless unity because V2's amplifier has no level of its own. The decision the user took on 2026-09-05 is therefore: no declared policy changes now; the parameter that first receives V1's amplifier level, or the first dynamic write to a V2 amplitude, decides its `Smoothing` against V1's per-block ramp with an A/B to measure. Fails closed: nothing is silently ramped. The mechanism is built and mutation-verified (`P05-S007b`), so the decision is a one-line declaration change. An independent read corrected this row's first form, which named the oscillator's own lowering as the trigger — a point already passed |

## Phase 6 — closed

[REV-P06](reviews/phase-06-exit-review.md) is **Accepted** on `e7e2d0a9`: a voice scope is one
prepared shape and `N` instances of state, stealing under three decided policies ends the taken
voice and starts the new note at a precise displaced sample on the compiled path and at the
live boundary, a bend follows its occurrence through a steal, velocity composes as V1 composes
it, one zone of a prepared sample map plays through a declared trigger, one prepared tuning is
held to every path, and a polyphonic render under pressure is the same bits run to run, under
every host partition, offline and live. Its gate was corrected on 2026-09-06 — the seed clause
and the live-note-across-recompilation clause carried, not claimed — and the phase exits with
two residuals, below. Eight slices, each merged on one independent read at the user's standing
decision. The slice table is archived at `archive/phase-06/`.

## Phase 6 residual obligations

| ID | Residual | Pull-forward rule |
|---|---|---|
| P06-R001 | The exit gate's first bullet claimed determinism for a fixed **project seed**, and no seed existed in V2 at Phase 6's exit; the event-stream half was held by bits on every path and the seed half carried, not claimed | **Closed** by `P07-S005` and `P07-S008` under [ADR-0008](decisions/ADR-0008-yams-state-identity-and-seeds.md): the seed enters as the project seed, the node, the instance and the stable script-state identity, never plan order or revision, and `tests/determinism.rs` holds Phase 6's pressure fixture — seeded per voice in the Control and Audio domains — to the same bits for one seed under fresh compilation, every partition, offline and simulated live, with a changed seed changing the samples (`fixed_project_seed_survives_stealing_recompile_partitions_offline_and_live`) |
| P06-R002 | The exit gate's seventh bullet claimed that note identity routes expression across a **plan recompilation**; the runtime resolves a stale identity as foreign to its new table (ADR-0047 clause 8, `an_identity_from_another_table_is_not_an_orphan`) and routes nothing, and a live note surviving a recompilation has no consumer before Phase 9's live host | Binds Phase 9, with ADR-0050 clause 8's redemption of a live note across an activation, to route or refuse such a note by a stated rule rather than by the table's refusal alone |

## Phase 7 — closed

[REV-P07](reviews/phase-07-exit-review.md) is **Accepted** on the squash that adds it: a
modulation edge feeds any declared parameter's modulation layer from a control-domain output
and the LFO is its first source, the filter's and the envelope's fields are addressable
controls, a placed pattern's instrument automation lowers onto the override layer as V1 emits
it with simultaneous absolute writers refused by name, V1's Mod Matrix and Mod Grid lower onto
that edge at V1's scale and its macros onto declared controllers, three YAMS domains run on one
off-thread compiler — Control as a scheduled node with stable local keys and identity-seeded
state, Audio per sample and allocation-free at the profile's maxima, Note as one finite
exclusively owned source measured (EVD-0020) before it was enabled — and the exit's evidence
holds every contributor to one slot under one law and a render to its event stream and project
seed alone, which closes `P06-R001`. Nine slices, each merged on one independent read; the
squashes of the last four were read whole. The slice table is archived at `archive/phase-07/`.
ADR-0060 owns the bounded Note source and its exclusive stream ownership.

## Phase 7 residual obligations

| ID | Residual | Pull-forward rule |
|---|---|---|
| P07-R001 | Saved Note Grid graphs and note-processor racks remain refused. The finite standalone authored stream's previous-quantum inputs and explicit cuts are not a saved-rack fidelity claim. | Phase 10A's canonical note-processing work item owns the model and lowering. Before any saved Note Grid/rack consumer, define its ordering, timing, identity, source bounds and fidelity dispositions, then test its saved inputs through the lowerer. Phase 10A cannot exit with this refusal unassigned. |

Owed onward, not residuals: ADR-0008's hot-edit clauses to Phase 9's plan-swap decisions
(ADR-0009, ADR-0010); ADR-0054 clause 3's simultaneous six-share reselection to Phase 9; the
saved sources, targets and lane classes `P07-S002b` and `P07-S003` refuse by name, and a saved
YAMS script, to the slice that first lowers each; and ADR-0012's product conflict policy to
Phase 10. `P05-R001` was not triggered: the phase's combined target is the filter's cutoff, its
seeded modulation targets the pitch, and an Audio program multiplying a signal is not a write to
an amplitude parameter.

## Phase 8 — closed and merged

[REV-P08](reviews/phase-08-exit-review.md) is **Accepted**: compiled channels,
returns, sends, inserts, master processing, declared latency/compensation and
explicit shared feedback are built for the Mono/Stereo subset. Channel, return
and master meters use declared bounded taps. [EVD-0022](evidence/phase-08/EVD-0022-routing-qualification.md)
qualifies every pinned corpus input for deterministic V2 rendering or named
refusal. Complete repository checks and independent reviews passed.

S006–S008 and the exit records are squash-merged to `main` on 2026-09-11;
S001–S005 were already on `main`. The phase's coordination is archived in
[phase-08](archive/phase-08/INDEX.md). Phase 9 entry readiness and its next implementation slice are stated below.
Existing ADR-0028/P04-R004 still owns whole-project V1/V2 orchestration in Phase
10B. P05-R001 still blocks nonunity amplifier-level lowering. The first consumer
of an uncarried saved effect, port, script, note-lifetime rule or modulation law
must establish its fidelity disposition before lifting its diagnostic.

## Phase 8 residual obligations

| ID | Residual | Pull-forward rule |
|---|---|---|
| P08-R001 | Oversampling and rate islands remain unbuilt; saved nonunity oversampling is diagnosed and excludes a parity verdict. | The first Sound Core rate-extension slice, before any nonunity-rate node or saved oversampling consumer, must define rate conversion, history and composed latency and qualify them. Phase 9 inherits this before such an expansion. |
| P08-R002 | Saved multiple terminals, module-input fan-in, implicit cyclic feedback, additional distortion/delay modes and tempo-following delay time remain refused by name. | The first lowering slice for each named route must define its law, explicit graph representation and a same-input V1 oracle before lifting that refusal. Native fan-in and explicit shared feedback already exist; no saved feedback boundary is inferred. |

## Phase 9 — active

[ADR-0036](decisions/ADR-0036-audio-device-and-input-lifecycle.md) and
[ADR-0024](decisions/ADR-0024-recording-take-and-commit-semantics.md) are `Accepted`.
Their current contracts are [host I/O lifecycle](specs/spec-host-io-lifecycle.md)
and [recording takes and commit](specs/spec-recording-takes-and-commit.md), with
capture budgets in the [host profile](specs/spec-host-profile-and-render-limits.md#recording).
P09-S001 builds the stopped-only simulated output coordinator; its bounded
conformance and remaining checks are recorded in the
[host I/O specification](specs/spec-host-io-lifecycle.md#conformance-tests).

P09-S002 builds complete typed recording configuration and the serialized
reservation/retained-result fixture under TAKE-INV-001 and TAKE-INV-006. Its
checked byte layout, separate finalization reserves, retained quality and
remaining consumer gates are recorded in the
[recording specification](specs/spec-recording-takes-and-commit.md#conformance-tests).
P09-S003 adds typed exact-input note recording against synthetic ordered
boundaries: immutable arm context, checked source ordering, FIFO note/pedal
pairing, retained timing and bounded finalization under TAKE-INV-001/002. Its
serial publisher uses explicit zero lateness and S002's reservations; supplied
audition traces do not affect capture. The recording specification records the
bounded checks and the remaining consumer gates.

P09-S004 adds finite certified note projection under TAKE-INV-004: exhaustive
tick-table admission, nearest-tick selection, optional start quantization and
explicit whole-result refusals against the retained context. Its bounded
checks and remaining consumer gates are in the
[recording specification](specs/spec-recording-takes-and-commit.md#conformance-tests).
P09-S005 connects the simulated output generation to retained exact-input note
capture: loss closes admission, freezes the acknowledged capture frontier and
waits for source/backend fences without another callback. Reconnection retains
unresolved results and does not resume capture. Its bounded checks and remaining
consumer gates are in the
[host I/O specification](specs/spec-host-io-lifecycle.md#conformance-tests).
P09-S006 adds a bounded serial lane for ordered capture start/end boundaries,
with source-fence waiting and explicit cancellation after host loss. Its checks
and remaining audible-transport scope are recorded in the
[recording specification](specs/spec-recording-takes-and-commit.md#conformance-tests).
Concurrent loop capture and physical adapters remain separately gated.
Input lifecycle, independent clocks and monitoring still require IO-INV-004 and
IO-INV-005 checks before their first consumers. Concurrent backend fences,
concurrent input/capture, held-note swaps, production hardware timing and project
transactions retain their named first-consumer gates.

### Selected work — ordered transport through live I/O

P09-S007 builds ordered compiled Play/Stop, resume and coupled exact-input note
capture. The [session contract](specs/spec-host-io-lifecycle.md#ordered-note-capture)
records its bounded serial checks. The non-shipping
[Linux output harness](specs/spec-host-io-lifecycle.md#linux-output-custody-harness)
adds actual ALSA callback custody under ADR-0063. The
[split compiled session](specs/spec-host-io-lifecycle.md#split-compiled-session)
separates control preparation/collection from audio ownership under ADR-0064, with
concrete command and completion queues in the Linux harness. Stopped plan readmission
preserves device time and refuses outstanding note obligations in the split core.
The Linux harness supplies latest-wins plan publication, off-thread collection and
joined recovery of all mailbox cells. The
[exclusive loop owner](specs/spec-sound-core-render-contract.md#exclusive-compiled-loops)
adds sample-exact compiled playback. Session/capture integration and physical timing
qualification remain open; held-note swaps remain gated.
Its [finite journal](decisions/ADR-0065-exclusive-sample-exact-loop-owner.md#retained-finite-observations)
retains successful render boundaries through worker stalls. The
[serial loop recorder](decisions/ADR-0066-serial-loop-capture-segmentation.md) adds
raw pass segmentation and bounded key carry; concurrent capture still needs source
ordering and transfer.
The Linux harness also runs this owner through its `loops` output mode under
[ADR-0063](decisions/ADR-0063-linux-cpal-callback-custody.md#exclusive-loop-output-consumer).

The user selected these three work items on 2026-09-11, in this order:

1. P09-S007: connect audible transport and recording to ordered session boundaries.
   Identical input and boundaries must produce identical audio and raw takes under
   whole, 64-frame, 256-frame and irregular callbacks. Accepted commands retain
   identified outcomes through source stalls and device loss.
2. Publish prepared plans across threads, retain the last valid plan on compile
   failure and reclaim retired resources off-thread. Resolve live-note ownership
   before enabling plan changes with sounding notes.
3. Implement sample-exact loop playback and pass identity, input lifecycle, audio
   capture and monitoring with independent clocks. Qualify physical adapters only
   against retained platform timing evidence and complete producer calibration.

Each item is implemented in bounded slices with the repository's risk-selected
checks and independent reviews. ADR-0065 resolves the standalone loop boundary;
controls and capture need integration within its exclusive ownership contract. The initial
physical target is Linux with CPAL and the existing V1 device-selection behavior,
as selected by the user, using this computer's built-in audio device. The user
confirmed that no physical MIDI device is available on this computer; local MIDI
verification continues with simulated sources. Physical MIDI qualification remains
open. The
[Linux timing evidence](evidence/phase-03/EVD-0016-host-time-mapping.md)
rejects the current direct candidate under F4; ADR-0022 qualification remains open.
Simulator and transport work proceed before physical qualification. Phase 0B and
Phase 10 work remain with their existing owners.

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
| P03-R001 | [ADR-0065](decisions/ADR-0065-exclusive-sample-exact-loop-owner.md) supplies the standalone sample-exact loop owner | Session controls and concurrent capture remain; ordinary activation retains ADR-0055's refusal |
| P03-R002 | Current producer shares, event cap, release holds and live-ingress depth remain provisional | [ADR-0054](decisions/ADR-0054-staged-producer-capacity-calibration.md) measures each first real authored/internal producer and requires complete reselection before production live ingress |
| P03-R003 | Note events carry identity but not typed pitch and velocity | **Closed.** A note-on carries a validated key and velocity, resolves the key through the plan's prepared tuning, expands to the control writes its scope declares, and a saved note's own magnitudes reach it. Phase 6 still owns the full composition law, which the work list is explicit this does not decide |
| P03-R004 | Numeric note-index and generation widths are safe by checked bounds and fail-closed exhaustion, but not endurance-qualified against a real live workload | Validate the widths before a production live adapter; generation exhaustion retires and reports instead of aliasing |

## Later-owned work

- Phase 6 owned `P04-R001`'s composition law and `SOUND-INV-021`'s **bend** clause; both are
  built (`P06-S003`, `P06-S004`).
- Phase 9 owns ADR-0022 acceptance against retained platform/adapter evidence,
  P03-R001 before integrated loop capture or phase exit, P03-R004 before production live
  ingress, and ADR-0050 clause 8's release-hold redemption and activation-time
  minter ownership before activation can coexist with live ingress.
- ADR-0051's shared-gate ownership law is required before two producers can
  drive one scalar gate through activation/catch-up behavior.
- Phase 10A owns the canonical project revision `P04-R004` waits for and the note-processing
  work item `P07-R001` binds; Phase 10B owns ADR-0028's acceptance and the revision-pinned
  job service.
- Phase 10E owns ADR-0039 and `LIMIT-0017`.
- Phase 0B still gates Phase 10.

## Current blockers

Phase 8 has no remaining exit blocker. Phase 0B continues with `P00B-T003`
selected; Phase 9 has completed P09-S001 through P09-S006.
The accepted residuals above block their named first consumers.
Session share 128 and total cap 360 remain provisional until
Phase 9's complete reselection under ADR-0054.
