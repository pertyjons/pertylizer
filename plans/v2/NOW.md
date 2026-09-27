# Core V2: Current Work

Last updated: 2026-09-27

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
ADR-0069 adds the bounded simulated input lifecycle and independent-clock merger below.
Physical adapters, qualified concurrent worker bounds, and monitoring
retain their IO-INV-004/005 gates.
Concurrent backend fences, physical held-note swaps, production hardware timing and project
transactions retain their named first-consumer gates.

### Selected work — concurrent live host and duplex capture

The five selected steps extend the simulated sessions with concurrent plan publication,
note recording across resets, independent PCM owners, a Linux duplex candidate and timing/load probes.
[ADR-0074](decisions/ADR-0074-concurrent-live-host-and-duplex-capture.md) defines their ownership boundary.
Steps 1–4 are implemented in the experimental host and passed the complete repository gate and independent review.
Step 5 adds backend latency probes, software loopback and identity/concurrency endurance tests;
physical round-trip calibration and the complete production producer partition remain qualification gates.
[EVD-0024](evidence/phase-09/EVD-0024-live-capacity-qualification.md) adds a controlled
real-producer capacity matrix. Its component probes pass, but the coverage audit does not qualify
production capacity: common host admission, same-stream live transport activation and a
representative workload remain missing. All six shares, release holds and ingress depths retain
their provisional status. No physical loopback is connected.

### Open design work — mixed producer activation

[ADR-0075](decisions/ADR-0075-mixed-producer-activation-ownership.md) frames the
possible mixed-producer activation boundary. It is `Proposed`: target ownership,
scoped catch-up, split identity custody, release-hold redemption, command
order, owner lifetime and capacity all need the ADR's acceptance evidence,
a tested combined host and explicit contract amendments before the current
refusals can be lifted. A candidate first slice is an
isolated audio-owned live ingress with source/result/held-cell credits and
protected releases.
The existing exclusive `LiveInputStream` now preflights held-cell credit against
pending key releases, sustain and Stop ordering. Source queue and result credit
still follow ADR-0073's terminal overload policy; the proposed protected-release
contract and combined host remain unbuilt.
The concrete simulated source now validates mapped time and source order before
queue custody. A terminal refusal retains its original and reason across the
producer/inbox handoff; the raw owner keeps a separate quality fault when an
earlier discontinuity already owns the primary reason. This establishes the
producer order prerequisite but grants no capacity or protected-release credit.
The concrete driver now uses producer-owned attempt IDs and non-cloneable retry
tokens. A full-ring retry cannot be exchanged for a fresh equal-valued attempt;
the producer retains the original if the token is lost, and terminal retirement
reports it while preserving any earlier source fault. The main thread services
source rings while producer workers retry to a bounded deadline. Successful
queue pushes receive distinct IDs even for equal-valued observations. This
establishes the source custody prerequisite, while the combined credit and same-key FIFO
ledger remain open.
The concrete bridge now reports whether raw admission also queued an audition
packet. A failed audition commit has no packet despite its accepted raw ID;
a later raw-trace attachment fault retains the queued packet. Source service
and the Linux driver carry this distinction beside the source queue ID. It
does not grant a protected release or a model receipt.
Audition preflight `Full` and `IdentityExhausted` after source queue custody
now retain a separate pre-raw terminal fault with the original in Core V2.
An earlier producer fault and a prior peer interruption keep their own
records, and the host reports any attribution error beside the original
preflight reason. Later queued packets after the halt return identified
`State` refusals. The ledger and combined credit remain open.
Raw input admission `Full` and `IdentityExhausted` keep their prior quality
policy; this addition covers audition preflight refusals.
Failed registration of a producer's pre-source-ring fault now reports the
producer's original reason and the Core V2 attribution error separately.
An independent design read found that merger forwarding cannot recycle ingress
hold or tracker credit: another source's earlier mapped-time onset can execute
before that release. The two-source ingress regression now runs in the workspace
gate and proves the consumer ordering and hold refusal. A combined admission
ledger, worst-case tracker-reserve bound and protected release service remain
open. Separate recorder-only fixtures prove pointwise tracker-full and
capture-reserve refusals when a second source's earlier onset precedes the
first source's later release; exact reverse publication refuses `PastBoundary`.
The combined host still needs one shared charge and a positive-receipt proof
across both consumers.
The off-thread target binding owns its validated plan, one admitted compiled
stream and one live note slot. It accepts disjoint voice-instance targets and
refuses shared nodes or compiled writers outside note targets. A constructor
now consumes that binding and prepares separate compiled and live identity
range owners with one table identity and stream epoch. Tests cover both
producer orders and cross-range release refusal. These owners expose no
production mixed ingress, rendering or activation; the existing refusals remain
in force. The private owner now has test-only mixed note offers.
The next source-bridge rehearsal must be V2-local: the concrete Linux-example
source rings cannot call those crate-private test-only offers. ADR-0075 now
requires a modeled two-source ring, one shared charge and a same-key FIFO with
separate raw-recorder and ingress dispositions before a combined receipt can
be claimed. The first V2-local test model now exercises bounded source
rings, one shared tracker/ingress/result/ledger charge, exact retry retirement,
same-key accepted/refused FIFO, release reservations through the source FIFO,
and dual-consumer settlement before credit reuse. A release behind an earlier
onset retry retains its original input in a source-local cell; while that cell
is occupied, a new onset or release offer causes a terminal `Order` result
instead of passing it. One host-wide fault cell retains the source, original
and exact first reason for terminal `Order`, uncharged `NoCredit` or release
identity exhaustion. The proposed recovery rule uses the stored copy; this
test model cannot prevent reuse of the identical original returned with the
first error. Later `Halted(original)` values remain caller-owned for joined
recovery. The model then halts onset and release offers from both sources.
A read-only onset preflight preserves the credit-settlement checks
without issuing a refused packet.
Ordinary packets have only a preflight; their refusal law remains open.
Retiring that retry retains a bounded same-key tombstone, so its later release
cannot end a newer occurrence. Its consumers are still fake. Joined teardown
after a terminal fault remains open. Raw occupancy, frontiers, capture reserve,
the real recorder and the mixed renderer remain to be connected. It does not yet
produce a combined consumer outcome or qualify protected-release service.
The actual simulated raw-input owner now preallocates a held-key ledger and
reserves raw cells for matched releases and frontiers. Its terminal refusal
still quiesces that source. The modeled source ring has no shared charge with
these real raw cells, the recorder or mixed ingress, so this is a local safety
gate rather than Phase 9 host acceptance.
The raw owner has a read-only admission preflight using the same plan as its
offer; it grants no reservation across a source-ring handoff.
A V2-local bridge probe now drains modeled onset and release packets from two
sources and offers their retained payloads to actual raw-input owners and the
note recorder. It checks source and payload custody, repeated-key recorder
pairing, and retained raw input IDs. The raw owner and recorder are parallel
test sinks. The probe has no real source-ring handoff, raw `InputReceipt`
settlement, combined result or mixed ingress. Model raw, result and ledger
credits stay held.
The actual serial `InputCaptureSession` now retains each raw release's
`matched_onset` in its `InputReceipt`. A two-source test compares that typed
raw link with the recorder occurrence carried by the same delivered receipt,
including repeated keys. Separate standalone and serial-session device-loss
tests show that cancelled raw receipts retain the admission-time link. These
tests join identities for delivered pairs in the serial fixture but do not
charge the modeled source ring or mixed ingress.
The concrete host connection remains open.
The two-source model now drains three queued onsets and their same-key
releases into an actual serial input/capture owner. The fixture invokes the
model's receipt-checked tracker settlement for each release. It binds raw
onset IDs and requires both delivered receipts to link the release to its
onset and one recorder occurrence before returning credit. Swapped or
cancelled release receipts are refused without returning tracker
credit. A fixed-generation, per-source serial watermark refuses replay of a
raw onset ID even after its model entry is reaped; reconnect is outside this
fixture. The separate fake-consumer settlement refuses entries bound to a raw
onset ID.
The model already returned ingress credit for its modeled refusal at source
service. Result and ledger credits stay charged; this serial raw/recorder
probe has no actual mixed-ingress offer or concrete source-ring handoff.
It has no concrete shared raw-capacity charge.
The modeled queue now also offers its three onsets to the private test-only
mixed owner beside the serial raw/recorder owner. It forwards each queued
key and velocity and retains the source-to-identity association in its ledger;
the private owner has no source field. Two onsets get live identities. The
third receives `Dropped(Hold)` after raw acceptance, which the model records,
and its release still reaches the recorder. A fault in the next audio quantum
classifies exact onset/release edges and redemption flags for the two accepted
identities at teardown. All three raw/recorder pairs still deliver and capture
completes. The model retains accepted ingress credits, result cells and ledger
entries because the private owner gives no per-occurrence outcome after the
fault and the fixture has no joined redemption rule. It uses direct offers
from modeled queues and has no concrete shared raw-capacity charge or production
mixed offer.
The raw owner now exposes typed read-only occupancy, matched-release
reservations and configured capacity. The two-source model charges a pending
raw onset and its future release before queue custody. Ordinary packets also
claim pending raw cells. Actual offers convert pending charges to occupied cells;
the model compares its pressure with the raw owner's snapshot after batches of
onsets, releases, frontiers and receipts. At eight cells, both preflights refuse
an onset when retained onsets or ordinary packets exhaust protected capacity;
retiring a pending ring retry returns its modeled raw charge. This is a model
of shared capacity for the tested running owners, not an atomic
charge held by the concrete source ring or raw owner. An atomic capacity
handoff and joined host result remain open.
The concrete example source inboxes now have a two-source receipt-link test.
It retains each accepted `SourceQueueId` and the raw `InputEventId` returned
when that packet leaves the ring, then matches the latter to delivered raw
receipts and recorder publications. Same-key releases on one source redeem its
onsets in FIFO order; the other source's same key stays separate. The fixture
uses serial producers and has no atomic shared capacity charge or mixed-ingress
offer, so the combined host acceptance gate remains open.
The concrete source packet now retains the producer-validated nominal engine
time through queue custody and retry. Source and managed-host service expose
the paired `SourceQueueStamp` beside each raw offer result; it contains the
`SourceQueueId` and mapped `SampleTime`. A pre-ring failure has no stamp. The
Linux driver reports the mapped time, and the two-source raw-receipt fixture
checks both source clocks. Raw capture still drains each source prefix
independently.
The service callback now returns `SourceHandoff`: the checked queue stamp and
the exact queued `InputObservation` beside its raw offer result. The two-source
fixture joins that original to the delivered raw receipt; an owned full-ring
retry and raw refusals preserve it. Pre-ring faults have no queued handoff.
This supplies source payload custody for a future mixed lane but adds no mixed
staging, shared charge or combined outcome.

The raw owner now exposes `matched_onset(id)` while an accepted raw cell is
retained. A concrete source-inbox test pairs queued originals with raw IDs and
reads repeated-key FIFO release links before any raw receipt is available.
An unmatched release returns `None`; an ID whose core raw cell is absent
returns `ReceiptOwner`, and a foreign generation returns `Stale`. The link must
be read at raw admission before that cell is reaped; a host may later retain a
receipt after reaping. The lookup grants no reservation or combined receipt.
On resumption, carry this immediate link with the source handoff into a bounded
mixed stage, retaining exact refusals and checking credit against both consumers
before reporting acceptance.

A separate V2-local bounded stage model now gives each of two sources packet
cells and reserves one future release cell with each admitted onset. An empty
peer can publish a strictly later frontier even when the other source is full;
repeated frontiers behind queued packets coalesce without spending packet
cells. Tests hold a waiting onset at the peer's equal-time frontier, select an
earlier peer onset and release, then select the waiting onset after the peer
advances. The strict empty-peer rule applies even if source rank would put the
waiting packet first in an equal-time tie. A frontier between queued packets
does not close an equal-time packet. This stage-capacity and ordering model has
no concrete source-ring or raw-owner charge, ordinary-packet disposition,
refund authority, joined fault teardown or production mixed offer.

The bounded stage now also drives the private audio-owned mixed command/result
ring in a two-source test. With either compiled/live producer order, it feeds
the earlier onset and its matched release before the waiting onset, then
releases that onset; each actual audio result accepts the staged request and
echoes its occurrence origin. This joins stage ordering to mixed ingress
results, but still uses synthetic stage packets and bypasses raw input,
the concrete source rings, recorder and combined admission.

ADR-0075 records why raw service must not wait for a mixed-time peer frontier:
the recorder needs far-ahead source frontiers to release its own bounded cells.
A separate bounded mixed staging and result path remains open.
An independent design read rejected a merger-only shared counter as sufficient
mixed admission. A V2-local counterexample gives source A an onset at sample 150
and source B one at 140: both raw owners accept, and the modeled shared credits
remain available, but mixed ingress refuses B with `NonMonotoneStamp` after A's
offer. Mixed offers need mapped-time ordering and an audio-side result path before
a combined positive onset outcome can be promised. Holding a popped onset while
waiting for tracker credit can also block its own release and frontier behind
it. Source-local release reservation, audition release credit, terminal refusal
custody and consumer-specific settlement remain required parts of the design.
The private mixed owner now rehearses an audio-side result path behind
`simulated-ingress`. Arm preallocates one command ring and one result ring at
the registered ingress queue depth. Control submits an exact onset or release
request with a command ID; explicit audio service returns its actual identity
or refusal with the original request. A reserved result slot remains charged
until collection, and owner collection refuses while a command or result is
outstanding. Tests cover two onsets and releases across threads without
audio-service allocation, exhausted control-side result reservation, the
out-of-order timestamp refusal and pending teardown. The private request now
carries an opaque source-occurrence correlation ID, but has no concrete source
handoff, mapped-time merger, automatic callback service or combined
raw/recorder receipt. Its test-only ring storage is bounded by
ingress depth but has no production host-profile byte charge; production
admission still needs one.
The two-source model now builds each onset request from the retained source
input, its supplied mapped stamp and its model-issued occurrence ID. One
model operation submits that request to mixed control and stores the returned
command ID beside the occurrence; there is no separate ID claim. Result
application rejects a swapped command, changed request, wrong result kind or
replay without changing a pending entry. The counterexample binds the first
live identity and records the second source's `NonMonotoneStamp` refusal.
Equal-valued onsets on two sources keep distinct origins and results. A source
release that precedes its onset result retains the later accepted identity
long enough for the model to submit a mixed release from its retained source
input and stamp. It keeps release credit until that command's matching
success result. It reaps a late onset refusal after the source release and
other result credit are gone. The older two-source hold/fault fixture now
consumes this same command/result path for its onsets and releases; it still
retains credits after the injected fault because the joined fault-redemption
rule remains open.
A stamped onset refuses an unstamped release before source-ring custody,
including after its ring retry retires to a source tombstone. A release held
in a source-local pending cell behind that retry gets an exact `MissingStamp`
retry result; the pending cell is cleared and the primary terminal fault owns
the original. A refused mixed release records its source original and audio
reason: it becomes the primary terminal model fault if none exists, or stays
on its entry beside the earlier
primary fault. Both source offers halt. A later onset refusal keeps its
ingress credit charged. Ingress, result and ledger credit are not redeemed
after halt without stopped-owner proof; retry and joined teardown remain open.
This fixture still uses modeled source rings and has no joined receipt or
production source-to-audio command contract.
A V2-local merge selector chooses the earliest stamped onset or release at the
two modeled source heads. Only the positive ordering fixtures use its gated
service helper; other model fixtures use direct service to isolate individual
laws or consumer behavior. An empty peer needs a serviced frontier strictly
later than the candidate time. A frontier must follow that source's last
submitted onset or release stamp. A fixture submits onset stamps 150 then 140
and sends them to the private mixed command ring in 140, 150 order; both
results accept. The fixture does not repeat the earlier raw-bound
counterexample. Equal-time heads use source order. A stamped onset or release
before the last submitted source stamp faults even without a frontier. One
before a frontier also faults with its original; an equal-time message remains
legal.
Source-time validation precedes release matching, so stale unmatched and
retired releases fault without consuming a tombstone.
Another fixture queues source A's onset at 140 and release at 145 beside source
B's onset at 150. The selector services both A packets first; the fixture
submits their mixed commands in time order. B remains held at A's serviced
frontiers 149 and 150 and proceeds when A reaches 151. A's release keeps its
accepted mixed identity. A separate selector test holds a later release behind
the peer onset and uses source order for both release/onset tie directions.
Concrete source-ring frontier handoff, ordinary scheduling, bounded wait
policy, callback service and combined host outcome remain open.
The concrete source and raw owner accept a message at the previous frontier's
time. The selector waits at an equal peer frontier and orders an arriving
equal-time head by source. An independent design read also found that one
undivided shared stage credit pool can strand the peer frontier needed for
progress. Mixed staging still needs a guaranteed peer-progress path, a
protected release reservation, bounded frontier progress and explicit terminal
custody before connection.
The mixed release-refusal fixtures deliberately bypass the selector: they send
the second source's 150 before the first source later offers its release at
145. That release is valid within its source, while the merge order is invalid.
The audio-side `NonMonotoneStamp` and original-custody checks remain defense in
depth. The selector would hold the peer onset while the first source lacks a
frontier. The new model fixture schedules A's release before that frontier;
the concrete source-ring and callback path remain open.
An off-thread check now places and stamps the bound stream against
a disposable copy of only the compiled range. It publishes no events or
reservations. Tests cover wrong-producer and wrong-capacity refusals, disjoint
compiled and live indices through copy/commit, and a post-mint refusal.
Runtime schedule output custody remains open.
The bound halves now enter a joined off-thread owner. It can seal the one
initial schedule with private events and a committed compiled-range minter,
or return the unchanged owner on refusal. The sealed value has no render,
offer, split or event-extraction API; cross-thread handoff and retirement are
still unbuilt.
The binding now retains lowering's step classification: local voice steps,
shared voice-sum steps and global steps, including rowless inserted work. It
derives typed parameter rows from the classified nodes, and both joined halves
share the immutable partition. Note gate and magnitude destinations must fall
inside their producer's local partition. Shared sums and global sources still
need influence and ordering laws; the partition provides no row-scoped
catch-up, reseed, activation or render path.
The partition now also enumerates one checked compiled-instance span per
addressable local parameter group and cross-checks that the spans cover exactly
the compiled rows in either producer order. Global parameter rows are excluded.
A scoped restoration event now reaches only an exact span in a renderer bound to
that partition; an ordinary renderer or a span crossing into live rows refuses
it. The mixed renderer seeds only compiled rows, and controller restoration
retargets both layers once. Renderer-level tests cover held-live audio parity,
the next smoothed live write and modulation, sample and quantum controls,
global-row preservation, invalid spans and allocation-free resolution. A
sampler test now checks that a compiled trigger falls without restarting its
playback or changing held live state, in both producer orders. The current
lowering gives shared voice-sum steps no parameter rows, as the test fixture
asserts; a future lowering with such rows still needs a preservation check.
The joined mixed owner now reconstructs a private compiled prefix at a requested
seek destination. Its non-cloneable off-thread candidate retains the pairing
book, destination-open snapshot and one scoped restoration per compiled local
parameter group. Tests cover strict prefix bounds, equal-position order,
repeated keys, last note magnitude, prepared bases, zero gate and trigger,
both producer orders and refusal without owner mutation. Binding also rejects
a synthetic sample-positioned controller target before preparation. It exposes
no event extraction, render, ingress or activation path. A suffix builder now
classifies the bound source indices and counts crossing releases and
expressions without producing the exclusive whole-group gate write. Repeated
keys pair suffix occurrences before prefix occurrences. A private first-activation
candidate now places that selection at the requested anchor, releases old
compiled reservations in a compiled-range copy and stamps the suffix there.
It retains the new outstanding set and initial sequence baseline, with scoped
restoration before a suffix onset at the destination sample. It has no offer
path. A private timing check now validates one possible effective quantum
boundary and one displacement across the complete list, including refusal if
only a later suffix event overflows engine time. Stamping checks the list's time
order off-thread, so effective-boundary preflight checks only its last event.
An audio owner still must
select the boundary, read the shifted list as its schedule, end old compiled
occurrences before rendering it, suppress the old schedule, account for other
same-quantum contributors, and settle shared/global influence, handoff and
retirement.
The boundary-release design read also found that the renderer's final timed-control
scratch reserved one write per ended identity while its pending queue reserved a
gate plus trigger writes. The scratch and its admission charge now reserve the
same full boundary queue beside a full event quantum. That queue uses the plan's
widest gate-plus-magnitudes width, avoiding event-group and parameter-fanout costs
on every held identity; storage invariants check both bounds. The mixed
activation release handoff remains unimplemented.
An internal renderer rehearsal now previews sounding compiled notes before
releasing them, checks producer and compiled-owned gate/trigger rows, and
refuses inadequate storage before changing its registry. After preflight it
seeds only compiled rows so the boundary release does not ramp. Renderer tests
use directly stamped compiled-provenance and simulated live onsets in both
producer orders, including a sampler trigger and synthetic missing rows; the
compiled occurrence ends while live output and rows match a live-only reference.
An additional renderer test sends a complete scoped restoration in the release
quantum in both producer orders; the next smoothed compiled frequency write
ramps, showing that its seed mark was consumed, while live audio remains equal
to the live-only reference.
The helper is not connected to mixed activation. That owner must publish the
complete scoped restoration in the same boundary quantum to consume all
compiled seed marks; the offer refusal remains.
The bound mixed audio half now owns an ended-note buffer sized from its compiled
span. A private boundary operation refuses a clock mismatch before changing
state, then releases the compiled partition and moves the musical anchor
together without allocation. Its result returns the old anchor for the future
schedule owner to retain with the retired compiled list. The delayed-boundary
renderer rehearsal changes the musical mapping and verifies the new anchor
and retained live audio in both producer orders. Its buffer still needs a
combined-host resource charge; the private one-shot callback now calls this
operation at its fixed boundary.
The privately stamped candidate can now shed its non-sendable source history
off-thread into a boxed, sendable audio capsule. A joined prepared owner can arm
one stamped candidate while stopped: it checks plan, epoch, table and initial
sequence, fixes the effective boundary and displacement from the renderer clock,
then splits the retained control from the audio half and old event list. A
refusal returns both inputs unchanged. Arm checks the renderer registry's
compiled span against the bound partition and proves the actual ended-note,
release-queue and timed-control storage. Corrupted storage refuses without
consuming the attempt. Profile-matched admission follows for the closed
candidate before arm
consumes it: actual restoration spends Session, shifted suffix spends Compiled,
and a one-quantum arbiter stays with the audio half. Refusals preserve both
inputs. A private callback now opens one quantum at a time, keeps the fixed
boundary pending through carry-only calls, suppresses old boundary-time events,
and charges release, scoped restoration, shifted suffix and one test-only live
onset through the same arbiter. Partition tests include a held live note in both
producer orders; one sounding compiled note is released at the boundary and
live audio from that quantum onward equals a live-only reference. Faults silence
the complete callback and retain the capsule, with the terminal cause, boundary
release and restoration charges, cumulative suffix charges, and completed
quanta reported.
Stopped private owners now collect off-thread. Collection checks the original
control/audio pair before consuming either half. A healthy adopted pair promotes
the copied compiled minter, outstanding identities, effective anchor and successor
sequence together, then drops the retired event list off-thread; its audio half
can continue rendering and later tear down. Pending, faulted and defensively
refused pairs return no runnable control. Teardown separates still-sounding
compiled and live identities from compiled notes ended at the boundary and from
uncharged or charged test-live reservations that never entered the renderer
registry.
The private preflight and collection checks now distinguish the original
control/audio pair from capsule plan, epoch and table metadata. A capsule
mismatch faults the callback or refuses promotion while still allowing the
correct pair to tear down. Tests cover terminal preflight, a head fault,
already-faulted renderer state, post-adoption promotion refusal and a fault
after resumed rendering. The defensive second-arm seam remains open.
Other same-quantum Session contributors, production mixed ingress with
payloads beyond this note-only rehearsal, and host resource charges remain open.
No production mixed offer or command refusal has been lifted.
The private stamped candidate now has a checked, allocation-free effective-event
view that applies one displacement on each read without rewriting requested-time
stamps. A renderer rehearsal reads that actual candidate after a delayed boundary,
releases the old compiled occurrence, and plays the destination note after scoped
restoration. In both producer orders its audio matches the sum of separate
live and compiled references with the same voice history. A release-only
reference is silent in the boundary and following quanta while the new compiled
note is audible. That earlier live onset is directly stamped in its test; the
new ingress rehearsal below has a separate test-only offer path.
The existing plan gate already charges the full addressable catch-up batch plus
one boundary release to Session. The mixed scoped batch is a subset of those
addresses, and its compiled suffix is a shifted subset of the stream admitted
against the compiled share. A separate scan of the private event counts would
repeat those bounds. The private arm now composes the existing checked Session,
Compiled, total-event, payload, timed-control, scoped fanout and seed-storage
bounds with the plan's declared release holds, the registered queue's Live depth
and a new full-backlog Release share check. A release share below queue depth
refuses arm and returns both owners unchanged. Other same-quantum Session
contributors and host resource charges remain open for a combined host.
The `#[cfg(test)]` ingress rehearsal in ADR-0075 now offers note-on and exact
identity release through the audio-owned live range and the registered
performance-event queue. A slot, hold or identity shortage uses that queue's
counted drop; a foreign or repeated release is an orphan refusal. Tests cover
same-time release before reused-index onset, non-monotone stamps, retired
indices, a full late backlog at adoption, live-only audio and row parity, and
pending, faulted and resumed teardown. A two-quantum terminal test proves the
callback journal retains entries charged before and during the fault, including
a release whose hold was spent at offer. The stopped registry separately reports
what reached sounding state.
This remains an internal rehearsal with no result-channel or source receipt,
FIFO tombstone, source-ring, raw-input or recorder bridge. Those require a
later shared charge and explicit contract amendments. Release-at-offer timing
is private to this rehearsal; a production mixed owner needs the
ADR-0046/0047/0050/0072 amendments first.

### Selected work — ordered transport through live I/O

The user selected six implementation steps on 2026-09-21, in this order:
reusable simulated live hosting with commands delivered during rendering;
explicit restart with retained earlier takes; audible simulated note input;
pass-aware musical projection; integrated count-in/metronome/arm/panic; and
the complete simulated-input chain in the Linux output harness.
Each step retains its named first-consumer contracts and independent review.
All six are implemented with bounded tests, independent review and the complete
repository gate. The Linux null-backend smoke run completed both retained attempts.
Ownership decisions are
[ADR-0071](decisions/ADR-0071-reusable-simulated-live-host.md) and
[ADR-0072](decisions/ADR-0072-finite-live-audition-and-count-in.md).
Physical MIDI and hardware timing qualification remain open.

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
adds sample-exact compiled playback. Physical timing qualification remains open; concurrent held-note swaps
remain gated.
Its [finite journal](decisions/ADR-0065-exclusive-sample-exact-loop-owner.md#retained-finite-observations)
retains successful render boundaries through worker stalls. The
[serial loop recorder](decisions/ADR-0066-serial-loop-capture-segmentation.md) adds
raw pass segmentation and bounded key carry.
[Ordered serial loop recording](specs/spec-host-io-lifecycle.md#ordered-serial-loop-recording)
adds finite coupled Play/Stop, retained command/source outcomes and finalization after
source stalls or loss without another callback under ADR-0067.
[Finite loop recording transfer](specs/spec-host-io-lifecycle.md#finite-loop-recording-transfer)
adds bounded command/source custody across threads and joined worker finalization under ADR-0068.
[Simulated input clock and capture](specs/spec-host-io-lifecycle.md#simulated-input-clock-and-capture)
adds independently generated inputs, configured synthetic clocks, explicit source prefixes
and retained loss/reconnect outcomes under ADR-0069.
[Threaded simulated input capture](specs/spec-host-io-lifecycle.md#threaded-simulated-input-capture)
connects the merger and audio owners with bounded custody and independent halt under ADR-0070.
The [continuous delivery experiment](specs/spec-host-io-lifecycle.md#continuous-simulated-delivery-experiment)
checks recurring input against explicit logical delivery and queue budgets.
OS timing qualification remains a separate consumer.
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
ADR-0067 integrates finite serial controls and capture within its exclusive ownership contract. The initial
physical target is Linux with CPAL and the existing V1 device-selection behavior,
as selected by the user, using a built-in audio endpoint. The retained Linux
runs used different host/device configurations; the 2026-09-26 local host has
an ALC256 endpoint. The user confirmed that no physical MIDI device was available
for local verification; MIDI checks continue with simulated sources. Physical
MIDI qualification remains open. The [Linux timing evidence](evidence/phase-03/EVD-0016-host-time-mapping.md)
rejects the 2026-09-12 direct run under F4 and the 2026-09-26 30-frame direct
run under F7; ADR-0022 qualification remains open.
Simulator and transport work proceed before physical qualification. Phase 0B and
Phase 10 work remain with their existing owners.

## Active streams

### Phase 0B — active in parallel

Phase 0B remains `Active, parallel`; Phase 10 still waits for its exit.

| Task | State | Current boundary |
|---|---|---|
| P00B-T001 | Complete | Closed 2026-08-29; 64 state entries are `Classified` and coverage-gated |
| P00B-T002 | Paused | Resume by assigning reachability and migration dispositions in the capability inventory |
| P00B-T003 | Complete | Identity/reference audit and proposed conversion dispositions closed in [EVD-0023](evidence/phase-00b/EVD-0023-identity-reference-coverage.md). Implementation and format/API approval remain with the named consumers. |
| P00B-T004–T007, P00B-T009 | Not started | Follow the frozen Phase 0B decomposition |
| P00B-T008 | Not started | Re-scope the former all-ADR task under `PROCESS.md` decision timing |

Next for Phase 0B: resume P00B-T002 with reachability and migration dispositions
in the [capability inventory](inventories/capabilities.md). P00B-T006 and
P00B-T007 still own the operation-result and format contracts; the completed
identity audit supplies their conversion cases and first-consumer obligations.

## Phase 3 residual obligations

Phase 3 is complete. Its exit review accepted these bounded residuals:

| ID | Residual | Pull-forward rule |
|---|---|---|
| P03-R001 | [ADR-0065](decisions/ADR-0065-exclusive-sample-exact-loop-owner.md) supplies the standalone sample-exact loop owner | ADR-0067/0068 add finite serial controls and thread transfer; ADR-0069 adds simulated independent source merging |
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

Phase 8 has no remaining exit blocker. Phase 0B next resumes `P00B-T002`;
Phase 9 has completed P09-S001 through P09-S006.
The accepted residuals above block their named first consumers.
Session share 128 and total cap 360 remain provisional until
Phase 9's complete reselection under ADR-0054.
