# Core V2: Current Work

Last updated: 2026-09-09

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
in [`archive/phase-06/`](archive/phase-06/INDEX.md).

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
| P04-R001 | V1 applies one saved velocity twice, under two independent sensitivities; V2 applied it once as one scale on the envelope | **Closed** by `P06-S004` under [ADR-0059](decisions/ADR-0059-velocity-composition.md): a voice has two velocity destinations, each with its own sensitivity and V1's formula, the lowerer carries `vel_sens` and `velocity_amp_sensitivity` to them, and a placed note no longer names velocity as unrepresented. `LOWER-INV-003`'s general rule stands: nothing may issue a parity verdict over a **lowered** outcome that is not `Faithful`, and a placed note is still `UnsupportedScope` through Phase 8's marks |
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
| P06-R001 | The exit gate's first bullet claimed determinism for a fixed **project seed**, and no seed exists in V2: nothing consumes randomness and Phase 7's ADR-0008 owns what a seed is. The event-stream half is held by bits on every path; the seed half is carried, not claimed. Fails closed: no node kind accepts a seed, so a render is a function of its event stream alone | Binds the first slice that gives a node a seed — Phase 7's, under ADR-0008 — to hold a render deterministic for a fixed seed as `tests/determinism.rs` holds it for a fixed stream, and to state where the seed enters |
| P06-R002 | The exit gate's seventh bullet claimed that note identity routes expression across a **plan recompilation**; the runtime resolves a stale identity as foreign to its new table (ADR-0047 clause 8, `an_identity_from_another_table_is_not_an_orphan`) and routes nothing, and a live note surviving a recompilation has no consumer before Phase 9's live host | Binds Phase 9, with ADR-0050 clause 8's redemption of a live note across an activation, to route or refuse such a note by a stated rule rather than by the table's refusal alone |

## Active streams

### Phase 7 — active since 2026-09-06

Activated by selection, as the Phase 6 exit said it would be. Its entry prerequisites are met:
Phase 6 is `Complete` under [REV-P06](reviews/phase-06-exit-review.md), and the Phase 3 gate the
master plan names as the one this phase may not begin before passed under
[REV-P03](reviews/phase-03-exit-review.md). No record is drafted at entry. Under `PROCESS.md`'s
decision-timing rule the phase's open decisions bind the slices that need them, not the entry:

- **ADR-0008** accepts [script identity and seeds](decisions/ADR-0008-yams-state-identity-and-seeds.md)
  for `P07-S005`. Live reload remains deferred to Phase 9's plan swap decisions
  (ADR-0009, ADR-0010); accepting script identity does not enable live replacement.
- **ADR-0012**, automation conflict policy, stays `Proposed` for Phase 10. This phase applies the
  master plan's multiple-writer rule in its strict form: two absolute writers on one target at
  one sample are refused at compilation by name. That is a refusal, not a product choice, and
  fails closed.
- **ADR-0054 clause 2** binds the first slice that makes an authored-runtime producer executable
  (`P07-S007`): it measures that class's high-water occupancy and reselects or retains its share
  before a downstream consumer can enable it.
- **ADR-0007's revisit condition** — the first modulator no listed law expresses — reopens that
  record by amendment, with the new law's arithmetic stated.

What V2 has at the phase's start: one parameter slot per addressable parameter, composing the
base, an override, the modulation sum and a per-note expression layer under one declared law
(`SOUND-INV-023`) and then a linear segment (`SOUND-INV-024`) — and the modulation sum has no
producer; a crate-private test seam writes it. The declared controls are the oscillators'
`frequency` (semitone law, sample-positioned, the pitch destination) and `amplitude` (decibel
law, quantum rate), the envelope's gate, velocity and sensitivity, the velocity scaler's and the
sampler's; the low-pass filter declares **no** control, so its cutoff and resonance are authored
fields nothing can address, and the envelope's times are likewise. Control-domain edges run
between ports (the envelope's per-sample output into the amplifier), but no signal feeds a slot,
no modulator kind exists, and nothing consumes randomness. The compiled timeline stamps
`SetParameter` override writes at plan positions under the compiled share, which is the
mechanism an automation lane lowers onto. The lowerer refuses a placed pattern carrying
automation and a Mod Grid instance V1's own builder returns, and reports a saved YAMS script
as unrepresented, each naming this phase; the note-processor rack and Note Grid graphs are
refused under a Phase 6 label that phase never claimed. The profile carries the phase's
capacities unchanged from V1 — the script limits, `max_mod_graph_nodes` and the two per-voice
slot counts — reported against themselves until a plan declares usage
(`spec-host-profile-and-render-limits`). What V1 has is inventoried: the per-module Mod Matrix
(`CAP-0024`), the pooled Mod Grid and Note Grid graphs (`CAP-0026`), the YAMS runtimes
(`CAP-0030`) and the automation tools (`CAP-0007`).

| Task | State | Current boundary |
|---|---|---|
| P07-S001 — the modulation edge and the first native modulator | **Merged** 2026-09-06 (`aeb41092`); `SOUND-INV-027` built, one independent read (agy), one defect repaired and reread clean; found and fixed a pre-existing per-node ramp-buffer layout defect for a voice-scope node with two quantum-rate controls at two or more voices | A **modulation edge** in the IR — a control-domain output into a declared parameter, with an amount in the target law's units — validated (units against the law, a not-modulatable target, a source that is not control-domain), admitted (edges into a voice scope's parameters counted per voice against `mod_matrix_slots_per_voice`, which moves that row into the refusal set), scheduled by dependency (the source's step before the slot's resolve before the consumer's), and summed into the slot's modulation sum once per quantum, so the layer gains its producer and the test seam is retired. Scope: an instrument-scope source into a voice-scope parameter broadcasts to every instance; a voice-scope source is per instance; a voice-scope source into an outer-scope target is refused by name, and no reduction is built. The first source is an **`Lfo`** kind with a quantum-rate control output — V1's deterministic waveforms (sine, triangle, sawtooth, square), rate in hertz and depth as declared controls so they are themselves targets, phase offset, bipolar or unipolar — with `SampleAndHold` and `SmoothRandom` refused by name until ADR-0008 gives a node a seed. The first target is the oscillator's `frequency`, the one semitone-law control that exists: vibrato. **Completion check:** a sine under an LFO at `±n` semitones renders at each quantum the pitch `base × 2^(n·lfo/12)` against an oracle from the LFO's closed form, sample for sample; sample-identical under four host partitions, which is the exit gate's cadence shape for a native modulator; a plan with no edge renders bit-identically to today (EVD-0013 and the `quantum_cost` digests reproduce); two edges on one slot sum; an override write with an edge in force keeps the modulation; the unit mismatch, the not-modulatable target, the voice-to-instrument edge, the random waveform and a per-voice edge count over the profile are each refused by name; the purity scan covers the LFO kernel and the per-quantum sum; mutation-verified. `P05-R001` does not bind: the target is a pitch, not an amplitude. Real-time path and admission: the core Rust gate and one independent review apply |
| P07-S002a — the filter's and the envelope's controls | **Merged** 2026-09-07 (`eb679358`); one independent read (agy), five wording and fixture points repaired, no behaviour changed | The filter's cutoff and resonance and the envelope's attack, decay, sustain and release are quantum-rate controls under their laws — two units, a quality factor and seconds, joined the declaration vocabulary under the physical-additive law — so V1's instrument automation targets are addressable. A kernel re-derives its coefficients or frames only where the value moves and holds the coefficients where the pair has no usable filter; an unmodulated plan is bit-identical, the digests reproduce. Stated in `SOUND-INV-023` |
| P07-S002b — automation as the override layer | **Merged** 2026-09-07 (`5d453fa4`); one independent read (codex, `gpt-6-astra`), three defects repaired and reread clean | A placed pattern's lane on the filter's cutoff or resonance or the envelope's attack, decay, sustain or release lowers to the values V1's sequencer would emit — V1's own `value_at`, tick, dedup threshold and descriptor, on the lowest module of its type — each as one `SetParameter` override write at its plan position under the compiled share, with the prepared base restored where V1's transport stops. Two writers on one target at one sample are refused by name, over intersecting ticks and over emissions rounding to one frame; the walk is bounded in ticks; the other lane classes stay refused for their owners. No amplitude is addressed, so `P05-R001` is not inherited. Stated in `SOUND-INV-023` and the lowering specification |
| P07-S003 — the Mod Matrix and the Mod Grid as edges | **Merged** 2026-09-08 (`7eda9406`); one independent read (codex, `gpt-6-astra`), five defects repaired, a focused reread found two more, repaired | V1's per-module Mod Matrix slots and the global Mod Grid instances V1's own builder returns lower into modulation edges under `SOUND-INV-027`, each V1 per-target scale becoming the edge's depth (ADR-0007's accepted cost): the LFO is the one source, lowered to V2's `Lfo` kind (voice scope from a patch, global scope from the grid), and the targets whose V1 law is V2's — the filter's cutoff and the oscillator's three pitch keys — are carried; the resonance, an LFO's rate or depth, a level, an envelope time, an envelope or macro source, a scripted slot, a random shape, a tempo-synced LFO, a cable out of an LFO, a track-scoped instance and the grid's cheap sources and track, master and channel targets are refused by name. Each edge is marked unrepresented for its timing (`CORPUS-0003-C1`). What V1 skips before reading lowers to nothing. The corpus's Mod Matrix case lowers, so three saved projects lower where `P04-R002` recorded two. Stated in the lowering specification under `LOWER-INV-004` |
| P07-S004 — controllers and per-note expression as sources | **Merged** 2026-09-09; independently reviewed | Declared controllers and occurrence-scoped source updates use the existing event capacities and parameter pipeline; all six V1 matrix macros lower to these sources. Contract and conformance: [`SOUND-INV-023`](specs/spec-sound-core-render-contract.md#invariants) |
| P07-S005 — YAMS Control as a node kind | **Merged** 2026-09-09 (`852fef1c`); full repository gate and independent review | Accepts ADR-0008's identity and seed half, and inherits `P06-R001`: a render is deterministic for a fixed seed as `tests/determinism.rs` holds it for a fixed stream, and the slice states where the seed enters. Compile source off-thread with the existing `synth_script` compiler into immutable program data with an interface schema, stable local parameter keys, source and destination declarations, a state layout and a cost estimate; a `Script` kind whose sources are bound to slots — runtime code resolves no name — whose local `param` knobs are ordinary declared controls, so a lane automates one through the same pipeline as a native parameter, and whose output is a typed control signal that never writes a stored parameter; one bounded evaluation per quantum charged to the profile's script limits times polyphony; a missing, cyclic or scope-invalid binding refused with a source-level diagnostic; a time-varying program sample-identical under partitions; removing or renaming a script parameter orphans its lane with a diagnostic rather than retargeting it |
| P07-S006 — YAMS Audio | **Built** 2026-09-09; full repository gate and independent review; merge pending | The per-sample domain as a kind: bounded and allocation-free at the maximum configured block size and voice count, with the cost warning published for an expensive program multiplied by maximum polyphony |
| P07-S007 — YAMS Note and the authored-runtime producer | Not started | The event-transformation domain, which is the first authored-runtime producer: measured under ADR-0054 clause 2 and its share reselected or retained before a consumer can enable it. The note-processor rack and the Note Grid graphs the lowerer refuses today are this domain's first consumers; whether they lower in this phase is decided at this slice |
| P07-S008 — one combine order, and determinism for a fixed seed | Not started | The exit's evidence: one target driven by a Mod Matrix edge, a Mod Grid edge and a script through the one slot, held to the documented order by an oracle; and the seed clause `P06-R001` carried from Phase 6, held on every path once `P07-S005` gives a node a seed |

Inherited before it builds: `P06-R001` (a fixed project seed, `P07-S005`) and `P05-R001` (a
lowered level's smoothing policy, binding the first slice that modulates or automates a V2
amplitude or maps V1's amplifier level). `P06-R002` is Phase 9's and `P05-R002` is Phase 10D's;
neither binds this phase.

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

- Phase 6 owned `P04-R001`'s composition law and `SOUND-INV-021`'s **bend** clause; both are
  built (`P06-S003`, `P06-S004`).
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

Nothing blocks the selected slice. `P07-S001`'s rule that a modulation source declares no
input port and no sample-positioned control is what keeps the envelope from being a source;
`P07-S003` refuses V1's envelope matrix source by name for it, and the slice that lowers that
source owns the collection split that lifts the rule. `P07-S003` also left an LFO's depth and
rate uncarried — V1 clamps the depth's offset per contribution and offsets the rate in hertz —
and no lowered target is an LFO parameter, so a modulation cycle cannot close today; the slice
that gives the LFO a lowered target owes the refusal that names the slot. Residuals bind later
work by name: `P06-R001` — a fixed project seed — binds `P07-S005`, the first slice that gives a
node one, and ADR-0008 is drafted before it; `P06-R002` binds Phase 9's live host; `P05-R002`
binds Phase 10D's digest; `P05-R001` — a lowered level's smoothing policy — binds the first slice
that modulates or automates a V2 amplitude or maps V1's amplifier level; and Phase 3's residuals
block only their named consumers. `P04-R004` binds the first shared render surface, which is
Phase 10B's.

Two streams are active: Phase 7, with `P07-S006` selected, and Phase 0B, with `P00B-T003`
as its selected slice.

Next action: **verify and squash-merge `P07-S006`**, then select `P07-S007` (Note YAMS).
Continue through `P07-S008` and the Phase 7 exit review under the user's
2026-09-09 instruction. Reload remains owned by Phase 9.
