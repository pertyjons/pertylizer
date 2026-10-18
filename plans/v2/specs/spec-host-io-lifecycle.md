# SPEC: Host I/O Lifecycle

| Field | Value |
|---|---|
| Status | Current |
| Phase | 9 |
| Created | 2026-09-11 |
| Last reviewed | 2026-10-17 |
| Based on | ADR-0036, ADR-0024, ADR-0021, ADR-0032, ADR-0038, ADR-0054, ADR-0061, ADR-0062, ADR-0063, ADR-0064 |
| Invariant prefix | IO |
| Supersedes | — |
| Superseded by | — |

Only a `Current` specification constrains implementation; see [README.md](README.md).
This contract is accepted. The conformance section distinguishes the implemented
simulated output and exact-input note-capture subsets from checks still required before later consumers.

## Scope

The host coordinator owns device discovery, configuration, connection generations,
callback lifetime, bounded input buffering and recovery outside Sound Core.

## Non-goals

Hardware clock mapping, compensation and drift laws remain ADR-0022's gated
work. Plan crossfades and live-note migration remain ADR-0009, ADR-0010 and
ADR-0050's live obligations. This contract permits the initial adapter to
reconfigure while transport is stopped; it does not grant those later behaviors.
No persisted format, wire contract or V1 API changes here.

## Terminology

A connection generation identifies one connection attempt, independently of a
renderer stream epoch. A request records endpoint selection (including explicit
system-default selection), desired rate/layout/buffer preference and permitted
fallback choices. Negotiated configuration records the actual choice and any
permitted substitutions. Observations report actual callback shapes, timing
provenance, latency estimates, discontinuities and counters; observations are
not guaranteed capabilities.

## Accepted decisions

| ADR | Decision it fixes here |
|---|---|
| [ADR-0036](../decisions/ADR-0036-audio-device-and-input-lifecycle.md) | Host ownership, generations, preparation, interruption and explicit recovery |
| [ADR-0024](../decisions/ADR-0024-recording-take-and-commit-semantics.md) | Retained capture and finalization at interruption |
| [ADR-0021](../decisions/ADR-0021-host-profile-and-admission-policy.md) | Queried capabilities versus chosen budgets |
| [ADR-0032](../decisions/ADR-0032-sample-time-and-event-timestamps.md) | Epoch configuration and time scope |
| [ADR-0038](../decisions/ADR-0038-engine-egress-queue-classification.md) | Custodial results versus lossy observation |
| [ADR-0054](../decisions/ADR-0054-staged-producer-capacity-calibration.md) | Producer qualification before production live ingress |
| [ADR-0061](../decisions/ADR-0061-ordered-compiled-session-ownership.md) | Ordered compiled transport and retained command ownership |
| [ADR-0062](../decisions/ADR-0062-coupled-transport-and-note-capture.md) | Total source ordering and coupled capture outcomes |
| [ADR-0079](../decisions/ADR-0079-mixed-host-raw-capture-interruption.md) | Raw-only late interruption and terminal priority in the combined host |
| [ADR-0081](../decisions/ADR-0081-combined-host-software-boundary.md) | Default-off common input route, strict publication and stopped reset |

## Invariants

The following requirements are normative.

### IO-INV-001 — Ownership and state transitions

The host coordinator runs outside the audio callback. Each connection attempt
has its own observable state: `Stopped`, `Preparing`, `Ready`, `Running`,
`Quiescing` or `Unavailable`, and a typed, non-reused generation independent of
endpoint display name, enumeration position and the renderer's `StreamEpoch`.
The coordinator separately reports the selected request, active generation and
any candidate generation; a candidate's `Preparing` state does not overwrite
an existing connection's state. At most one candidate per requested endpoint is
prepared at a time. Exhaustion refuses another attempt rather than reusing a
generation.

An open/negotiation failure takes the candidate to `Unavailable`; a device loss
takes an active connection through `Quiescing` to `Unavailable`. A voluntary
shutdown ends in `Stopped`. A retry creates a fresh generation in `Preparing`.
Old generations remain owned until their callbacks and capture fences are
quiescent; `Unavailable` is not permission to free live callback state.

A lost or canceled candidate cannot activate. Before it owns callback resources,
loss ends in `Unavailable` and cancellation in `Stopped`; a late preparation
completion is refused. Once resources exist, either event enters `Quiescing`
and waits for the same backend/capture fences before reclamation. Candidate loss
does not invalidate an otherwise valid active connection. Candidate callbacks
produce silence and cannot enter the active generation's epoch.

The normal transition is `Stopped -> Preparing -> Ready -> Running`. Preparation
negotiates the configuration, obtains the capability inputs required by
`HostProfile`, admits the plan and allocates callback/input/capture resources.
`Ready` means these are owned and usable, not that a compile was merely queued.
If calibration needs running callbacks, the candidate stream supplies silence
and cannot deliver live ingress or monitoring until its mapping is ready.

Activation publishes the prepared state and its generation coherently. A
callback uses that immutable state for its entire invocation. Commands,
callback errors and completion acknowledgements from a retired generation
cannot act on its replacement. Opening an input connection inside an existing
render epoch still needs its own clock bridge and generation check.

### IO-INV-002 — Stop, reconfiguration and reclamation

A user transport stop follows session ordering and may leave the device running
silence; it does not create an epoch. Device shutdown instead enters
`Quiescing`: close new admissions, publish the stop boundary, retire callback
access, then reclaim streams and resources off-thread. Unsupported hardware
pause uses off-thread stream shutdown, not repeated calls inside the callback.
No final drop of a plan, asset, buffer or stream occurs on a real-time thread.

Rate, layout or prepared-capacity changes require a new renderer epoch and new
preparation under ADR-0032. An observed callback-size change within the admitted
maximum uses the existing epoch and partition-independent renderer. An actual
oversized callback takes the existing terminal fault: silence the complete
callback, invalidate both carries, publish `needs_reprepare`, and render no
later quantum in that epoch. Do not split or truncate it as a recovery shortcut.

Preparation failure leaves the project and last valid plan owned. The old
configuration may keep sounding only while its device and epoch remain valid;
retaining a plan is not a promise that a disconnected device can play it.
Restoring an old rate/layout after retiring its stream is another preparation,
not reuse of the old epoch. Initial reconfiguration occurs while transport is
stopped; seamless changes require the separately accepted live-swap contract.

A device-initiated rate, layout or callback-bound change while prepared reports
`DeviceReconfigured`, quiesces like a loss without needing another callback,
publishes `needs_reprepare`, and interrupts active capture as device
re-preparation. The old plan never renders under the new configuration.
Recovery is an explicit preparation against it, which the request's explicit
fallback set must permit; otherwise that candidate fails visibly and the last
valid plan remains owned.

### IO-INV-003 — Unknown capabilities and fallback

A missing buffer bound is reported as unknown. Neither CPAL's buffer estimate
nor the largest callback observed so far becomes a queried maximum. The initial
adapter refuses activation if it cannot establish the capability required by
HOST-INV-003. An adapter-specific way to establish that bound must be justified
against its backend contract before use. Supporting devices without such a
bound requires an explicit successor to the affected host-profile contract;
this contract does not relabel a chosen ceiling as hardware capability.

Rate/layout substitutions must belong to the request's explicit fallback set.
The result reports the actual choice before activation. Ambiguous endpoint
identity refuses selection; equal names are not proof of the same device.

### IO-INV-004 — Device loss and recovery defaults

Unexpected input or output loss stops transport, finalizes any active recording
as interrupted under ADR-0024, and disables monitoring. Keep the selected device
request and expose which connection failed. After detecting loss, callbacks
that still run produce silence; do not replay stale input or advance musical
playback to catch up with elapsed wall time.

The coordinator may retry the same unambiguous endpoint using one in-flight
attempt, bounded retry frequency and cancellation by a newer user request.
Successful reconnection returns to `Ready`: playback, recording and monitoring
do not resume automatically. A different device is used only under an explicit
fallback preference or a new selection. Enumeration and retry work stays
off-thread. This default also applies when only the input device disappears.

Loss may arrive without another callback. Finalization therefore cannot depend
on a future audio invocation: the coordinator waits for callback quiescence,
then hands retained recording storage to its owner. The last valid capture
boundary, discontinuity and interruption reason travel with the result. It must
not timestamp finalization by the much later GUI notification time.

### IO-INV-005 — Input clocks, monitoring and capture

The input adapter owns a bounded preallocated buffer and any prepared sample-rate
conversion/drift correction. Input and output are independent clock domains,
even if they currently report equal rates. ADR-0022 owns mapping, correction
law, uncertainty and input/output/round-trip latency; this decision fixes their
owner, not numeric latency values or a resampling algorithm.

Monitoring and recording have separate loss policies. Monitoring may discard a
documented portion of stale input to restore its admitted latency window; the
affected range and counters are observable. It must not make a hidden backlog
grow. Recording cannot inherit that eviction: once capture accepts data, retain
it on ADR-0024's custodial path. Capture exhaustion or an input discontinuity
ends an interrupted/partial take rather than reporting complete audio with an
unmarked hole. Worker or GUI backpressure never blocks an input callback.

### IO-INV-006 — Diagnostics and qualification

Expose requested and negotiated configuration, connection generation, lifecycle
state, active epoch/plan acknowledgement, failure cause, retry status, actual
callback size, input loss/backlog and timing provenance. Callback-side reporting
uses preallocated state/counters; formatting, logging and subscriber delivery
are off-thread. Lifecycle faults remain observable if telemetry is saturated.

## Types and ownership

Use validated domain newtypes for connection generation, endpoint identity,
rate, frame counts and timing quantities; connection state is a closed enum.
The coordinator owns requests, negotiated resources, active and candidate
connections. The session owns the active epoch. Callback code borrows the
published immutable prepared state. A plan owns no device handle or callback.
Requested configuration belongs to host/application settings, not project data.

## Lifecycle and timing

IO-INV-001 through IO-INV-004 define preparation, activation, quiescence and
recovery. Input and output bridges are per connection; equal reported sample
rates do not establish a common clock. Production mapping is gated by ADR-0022.

## Failure and diagnostics

IO-INV-002 through IO-INV-006 define terminal callback faults, unknown-capability
refusal, interruption and persistent diagnostics. Retained capture follows the
[recording contract](spec-recording-takes-and-commit.md), including source fences
and quality state. A worker/GUI notification cannot be the only payload owner.

## Real-time and resource constraints

All discovery, opening, retry, allocation and reclamation run off-thread.
Callbacks must not allocate, block, log or perform final destruction. Input
buffers and report channels are preallocated and admitted before activation.
[HOST-INV-003 and HOST-INV-020](spec-host-profile-and-render-limits.md#invariants)
retain their capability and recording-retention guarantees.

## Conformance tests

The decision is wrong if any transition can activate mismatched coefficients,
accept a stale generation, free callback-owned state, lose accepted capture,
or require blocking/allocation in the callback. Phase 9 must test:

- A simulated backend changes rate/layout, refuses creation, rejects pause,
  disconnects with and without a final callback, and reconnects under the same
  name with a different identity. The project survives and state is explicit.
- Delayed callbacks, errors and readiness acknowledgements from connection A
  arrive after connection B is ready. None mutates B or enters B's epoch.
- Callback partitions vary below the admitted maximum; one exceeds it. The
  former preserve audio, the latter takes the terminal fault in every build mode.
- Unknown capabilities refuse; a measured estimate is never advertised as a
  guaranteed bound. A selected fallback is reported, an unselected one refused.
- Independent clocks drift while workers stall. Monitoring stays within its
  declared window or visibly faults; capture keeps its accepted prefix and
  exposes the first discontinuity. No concealed growth or missing-data success.
- Retirement and completion channels saturate. No final drop occurs in the
  callback; recovery waits off-thread and retained data still has one owner.

The full checks above remain phase obligations. P09-S001 implements a bounded
output subset in `synth_engine_v2::host::SimulatedHost`. It uses the real compiler
and renderer with harness capabilities; it opens no hardware. Exclusive Rust
borrows serialize simulated callbacks, while an explicit backend acknowledgement
models delayed quiescence. This tests the coordinator's response to a fence;
it does not establish any real backend's fence or concurrent publication proof.
The simulator's start/stop latch freezes rendering while silent; it is not the
ordered runtime-session transport lane. Every retry is a new explicit user
request, with no automatic retry or resumption.
In this simulator, fallback lookup advances past missing endpoints only; an
enumerated endpoint's open/configuration failure is surfaced. A newer request
must wait for shutdown and quiescence of a prepared candidate before replacing it.

`crates/synth_engine_v2/tests/host_lifecycle.rs` covers the following subset:

- IO-INV-001: distinct active/candidate state, readiness only after preparation,
  coherent generation/plan/epoch publication, and stale commands, callbacks and completion.
  Candidate loss or cancellation invalidates readiness and waits for owned resources
  to quiesce; candidate callbacks stay silent and cannot mutate the active connection.
- IO-INV-002: stopped-only configuration changes with a fresh epoch, preparation
  failure retaining the active plan, sine-wave partition equality with equal clocks,
  and terminal oversized callbacks while both Ready and Running. Shutdown uses the
  fence directly and has no pause dependency. A device-initiated rate or bound change
  while running quiesces with or without a final callback and recovers only through a
  permitted preparation; an oversized-callback fault recovers through a larger bound.
- IO-INV-003: unknown bounds refuse even with an estimate or buffer preference;
  ambiguous identity refuses, equal names do not match, and explicit substitutions
  are reported before activation.
- IO-INV-004: output loss with and without a final callback, retained last-valid
  plan, silence after loss, and reconnection remaining Ready until explicitly started.
- IO-INV-006: generation-local persistent faults and callback size remain visible
  without a telemetry reader. Stale messages cannot alter replacement diagnostics.

The `host::tests` unit tests exercise generation exhaustion and stalled retirement
with repeated candidate replacement. The existing allocation guard counts zero
allocation or deallocation events across the first render, terminal fault,
quiescing, stale and Ready callbacks. The callback is also in the purity scan.
Only one active and one candidate connection can own callback resources; activation
waits for old quiescence, and reclamation runs in the coordinator.

P09-S001's local validation includes both commands below. The release invocation
is additional to the repository gate; CI does not currently run this V2 suite in
release mode. The prepared identity in a status record is not a concurrent
callback acknowledgement; that consumer retains its later gate.

```bash
cargo test -p synth_engine_v2 --test host_lifecycle
cargo test -p synth_engine_v2 --release --test host_lifecycle
```

P09-S005 attaches one `SimulatedNoteRecorder` to the active Ready output's
generation and epoch through `SimulatedHost::prepare_note_capture`. Its complete
descriptor and arrays use the recording byte admission. A borrowed
`NoteCaptureControl` admits exact-input publication and explicit capture boundaries
only while that output remains Ready or Running; it cannot replace the recorder.
Arm rejects a context from another epoch, sample rate or prepared stream anchor
before consuming a result slot.
The simulator's transport stop refuses an active take until the caller explicitly
finalizes it. This is not the ordered runtime-session transport consumer.

Output loss, voluntary shutdown and simulated note-source loss close that view.
Terminal render faults close it in the callback; the coordinator subsequently
finalizes capture. The selected end freezes at the minimum previously acknowledged
frontier among the take's sources, bounded by its selected end. An unfenced source
establishes no prefix; count-in loss can select an empty interval at epoch origin.
Accepted raw records beyond that end remain readable outside selection. Neither a
later GUI timestamp nor the renderer's quantum-ahead clock supplies the boundary.
One source reaching the requested end requests a stop; it does not seal the take
while another source's publication frontier still lags. Loss in that waiting state
still interrupts at the minimum known frontier. A take already sealed before loss
keeps its sealed outcome and interval. Different publication frontiers consumed
before loss can therefore produce different selected intervals; a shutdown
acknowledgement cannot stand in for the missing publication fence.
Each bound source acknowledges shutdown separately, without advancing its frontier.
Finalization and output-resource retirement wait for their applicable source
fences; output retirement additionally requires an explicit backend acknowledgement.
A backend acknowledgement attempted too early refuses and must be retried after
source quiescence. No acknowledgement is inferred from a silent callback.

Capture results remain owned after output retirement and replacement. A new output
returns Ready without resuming capture. Preparing another recorder refuses until
the owner has acknowledged current result quality, discarded every retained result
and explicitly released the quiescent storage after backend retirement. Loss of an
unselected but bound note source also stops the output and names that source in
persistent diagnostics. Candidate loss does not affect the active recorder.

For fixed publication frontiers consumed before loss, the falsifier is lost accepted
input, a selection changed by source-shutdown acknowledgement order or GUI-notification
delay, admission after loss, retirement before both fences, or recording quota
replaced while results remain unresolved.
`src/host/capture/tests.rs` checks loss with/without a final callback, unequal source
frontiers and both acknowledgement orders, a natural end awaiting a lagging source,
count-in, candidate and stale-generation isolation, source loss, full ordinary storage and stalled result consumption,
terminal callback faults, explicit disposal and reconnection. The allocation guard
observes zero allocation/deallocation during publication, loss, silent callbacks and
source finalization. Renderer and capture storage reclamation remain off-thread.
The release-mode checks also run locally with:

```bash
cargo test -p synth_engine_v2 --release --lib host::capture::tests
```

This remains an exclusive-borrow simulator, with explicit synthetic source fences.
P09-S006 adds the [ordered capture-boundary lane](spec-recording-takes-and-commit.md#conformance-tests).
Host interruption retains each queued command until dispatch returns its
cancellation receipt, including after output retirement or replacement. Pending
receipts block recording-storage release. Capture commands do not operate the
output's transport latch; the [ordered compiled owner](#ordered-compiled-transport)
provides that separate consumer.
It has no concurrent pending source queue, hardware input, monitoring or audio
capture. IO-INV-005 remains unimplemented. Physical input loss, backend capture
fences, independent clocks, worker backpressure and actual telemetry/retirement
channel saturation still need the full checks below. Physical timing acceptance requires ADR-0022's retained
platform/adapter evidence and ADR-0054's complete producer calibration before a
production live adapter is enabled.

| Invariant | Full check still required beyond P09-S001/S005 |
|---|---|
| IO-INV-001 | Concurrent publication and backend callback fences; independently opened input generations |
| IO-INV-002 | Concurrent session stop, real backend shutdown when pause is unsupported, concurrent off-thread retirement |
| IO-INV-003 | Evidence establishing any physical backend callback bound before activation |
| IO-INV-004 | Physical input loss, concurrent retained capture with/without final callback, and bounded automatic retry if implemented |
| IO-INV-005 | Independent clocks, bounded stalled-worker input and distinct monitoring/capture loss |
| IO-INV-006 | Saturated concurrent telemetry preserves faults and acknowledgements; input backlog and timing provenance |

## Ordered compiled transport

[ADR-0061](../decisions/ADR-0061-ordered-compiled-session-ownership.md) defines the
optional ordered transport mode in `SimulatedHost`. Enable it on a fresh Ready
renderer; device `start` begins callbacks, while session Play/Stop independently
control musical playback. The legacy transport stop refuses this mode. The
[ordered capture extension](#ordered-note-capture) attaches note capture through
the same owner; direct capture control remains unavailable in that mode.

A stopped session holds its playhead while silent quanta advance the renderer's
sole engine clock. Play prepares a new activation from that position, cuts crossing
notes and applies normal parameter catch-up. Commands name aligned engine times in
nondecreasing submission order. Whole-callback shape and maximum length validation
precedes every command. Carry delivery applies no commands; each new quantum
handles its boundary before the scheduler is called. Already rendered carry retains
its normal one-quantum latency across Stop.

`SessionLimits` admits the command descriptor array, runtime descriptor and retained
scalar outcomes against an explicit byte budget. Scheduler, arbiter and activation
payloads retain their existing preparation admission. One outstanding Play retains
its activation until off-thread receipt collection, and cannot consume the final
command slot. Already accepted Stop effects do not depend on receipt consumption.
Same-time commands plus Play's complete catch-up cost must fit the session share;
this is conservative even for a subsequently cancelled Play. Command identities
combine the connection generation with a checked serial; refusal consumes neither.

A same-time Stop cancels a Play only while it remains pre-offer. Accepted scheduler
offers are irrevocable. Their retired resources stay in the original command slot
until off-thread promotion or withdrawal. Loss cancels unapplied commands, preserves
applied outcomes and blocks resource retirement until all receipts are collected.
Callback faults silence the complete output and close admission through Quiescing.
If the final playhead cannot be represented, closure reports the fault once and
marks playback Unavailable; retained outcomes remain collectable on the next call.

`tests/host_session.rs` checks audible boundaries, resume, crossing and later notes,
partitions of 512/256/64/37/1 frames, delayed receipts, cancellation, loss without a
callback, invalid boundaries and whole-buffer rejection. `src/host/session/tests.rs`
checks first-use allocation/deallocation, identity exhaustion, producer-share
refusal and byte-budget refusal before ownership is latched. The purity scan covers
the session callback and scheduler offer/retirement transfer. These are serial
checks; concurrent publication and hardware timing remain gated.

## Ordered note capture

[ADR-0062](../decisions/ADR-0062-coupled-transport-and-note-capture.md) extends the
compiled transport owner. `prepare_ordered_capture` attaches the retained recorder
and a fixed source queue while Ready. Its descriptors and action/outcome slots use
`SessionSourceLimits`; recorder arrays and tempo contexts retain their recording
byte admission. Preparation and arm are off-thread. No direct start, stop, publish
or fence method escapes through `SessionCaptureControl`.

A recording Play requires an armed context matching its stopped position, exact
requested start, rate and epoch. Its immutable ticket and each Stop's captured ticket
travel with the command. Rearm waits for every command/source receipt to be collected.
Stop snapshots only the recorder's currently active ticket; a sealed or discarded
take cannot attach itself to later ordinary commands. Already queued Stops keep
their original tickets. Ordinary Play does not start recording; `offer_session_recording_play` explicitly
couples both operations. Stop ends the associated armed or recording take.

Source actions use nondecreasing publication/frontier times. All equal-time fences
must be offered before equal-time publications; a later fence refuses without
spending identity. Dispatch preserves publication order and original capture stamps.
The serial owner supplies each explicit fence's actually consumed source sequence.
No source frontier is synthesized. This is an exclusive-borrow prefix proof, not a
physical backend acknowledgement.

A same-time Play followed by Stop cancels before fences or capture start. Stop runs
before equal-time fences. A surviving recording Play then drains the complete fence
prefix, checks activation admission and starts capture before its infallible offer.
A missing start fence yields a capture-refusal outcome and leaves Play unoffered;
there is no delayed start. The pending take remains owned for an explicit end.
Remaining source actions before the quantum end follow the command group.

Every command carries an optional, separate `SessionCaptureOutcome`. Routine capture
refusals never propagate as output faults. Stop changes audible state even when its
capture result refuses or source fences delay sealing. Only an invariant failure
between successful activation preflight and offer enters the terminal session-fault
channel. Every source action likewise retains Published, Fenced, Refused or Cancelled
until collection. Loss preserves both receipt lanes; output retirement and capture
result disposal cannot bypass still-held outcomes.

`src/host/session/capture/tests.rs` checks callback partitions of 512/256/64/37/1,
identical audio and selected raw notes, allocation/deallocation from first callback,
missing start fences, same-time cancellation, an independently lagging source,
ordinary capture-stop refusal, saturated source/result slots and loss without a
final callback. Publication occurrence counters are compared within each source;
connection generations intentionally differ between independent fixtures.

This consumer uses exact simulated sources with zero lateness and supplied audition
traces. Standalone punch-in/disarm, live audition, audio input, loop passes and
concurrent or physical source timing require their subsequent consumer checks.

## Ordered serial loop recording

[ADR-0067](../decisions/ADR-0067-ordered-serial-loop-recording.md) adds the finite
`LoopRecordingSession` under the session API. It consumes an armed, unstarted, fresh
`LoopCaptureSession` at engine clock zero with intact priming carry, retaining
exclusive ownership of the renderer, journal, recorder
and both receipt lanes. Its generation uses the simulator's checked common issuer.
Standalone loop constructors and the ordinary linear session retain their contracts.
Every preparation refusal returns the original capture owner, including any existing
take; attempting to convert an active or already rendered fixture cannot discard it.

One recording Play is admitted at the immutable reserved capture start. The
initial-time constructor retains its original behavior; the proposed
[reusable host](../decisions/ADR-0071-reusable-simulated-live-host.md) adds explicit
future-start preparation and checks source-clock reachability. Stops are ordered,
quantum-aligned commands using the existing session identities and outcomes. A new Play,
rearm or plan replacement cannot reuse this finite owner's stopped state. Play reserves
one command slot for Stop; collection is required before occupied receipt slots are reused.
Each same-time command group fits the loop's Session share. A successful Play uses the
already admitted initial restore; cancelled/refused Play and Stops are charged as bounded
operations in a stopped quantum without compiled-loop publication.

Stop precedes an equal-time loop wrap and freezes the actual loop-source position.
It selects silence for subsequent quanta without advancing DSP state. Existing output
carry keeps its original delivery order and priming latency. Dormant note identities stay
exclusively owned until off-thread destruction; no resume or plan swap can redeem them.
Equal-time Play followed by Stop cancels Play; Play after an earlier Stop also cancels.
A surviving Play drains the same-time source-fence prefix before starting capture.
A missing start fence yields a retained capture refusal and no delayed playback start.
Stop changes playback even when the source has stalled or capture refuses its request.

Only a successful whole callback commits its journal boundaries and Stop endpoint.
Stop closes observation with `Finished` at the pre-wrap snapshot; an earlier terminal
remains immutable. A terminal callback failure silences the complete output and closes
observation at the previous successful frontier. Applied receipts describe executed
controls, not delivered audio. The take's minimum endpoint and strongest outcome still
bound capture selection, including raw input accepted during a subsequently failed call.

The existing serial source FIFO preserves stamps and identified outcomes. After Stop,
delayed source actions may still be offered in FIFO order; `drain_stopped_sources` consumes
the finite admitted source FIFO, including explicit source frontiers beyond the last
audio clock. This advances neither engine time nor the selected audio endpoint and
manufactures no source frontier. Off-thread finalization waits for
all participating fences and writes pass/carry metadata before sealing.

Device/source loss closes command and source admission, cancels pending actions and
freezes selection at the minimum acknowledged audio/source endpoints. Each source must
acknowledge quiescence before interrupted finalization. Neither retirement acknowledgement
nor finalization needs another audio callback. Results and both receipt lanes remain
readable, and result discard refuses outstanding command or source outcomes.

The command budget charges its fixed slots and additional wrapper inline storage.
Source queue storage, recording storage, journal heap and the loop renderer retain their
own existing byte ceilings. Hot operations allocate and deallocate nothing.

`recording::notes::loop_capture::tests::ordered` checks identical actual audio, raw records,
pass/carry descriptors and normalized receipts under whole, 64-frame, 256-frame and
irregular callbacks. It covers Stop at an exact loop boundary, missing start fences,
same-time cancellation, queue/result pressure, exact storage ceilings, lagging sources
and loss without another callback. Concurrent transfer, physical timing, live audition,
restart, held-note swaps and pass-aware musical projection remain separate consumers.
The finite transfer consumer below adds the bounded thread handoff without changing this serial API.
After normal sealing, `close_completed` closes publication and cancels pending actions;
source quiescence and explicit quality acknowledgement then permit result discard.

## Finite loop recording transfer

[ADR-0068](../decisions/ADR-0068-finite-loop-recording-transfer.md) adds the finite
`loop_transfer` consumer. A fresh empty-lane `LoopRecordingSession` splits into
`LoopRecordingControl` and `LoopRecordingAudio`. Audio exclusively retains the existing
renderer, journal, raw recorder and serial lanes. Control owns a separate fixed credit ledger.
The callback uses the serial whole-callback algorithm and bounded admission/receipt moves.

Each opaque owning packet has a checked `LoopTransferId`, distinct from the eventual serial
command/source identity. Each split issues a fresh transfer generation, including a repeated split
of an untouched reunited session. C command credits and S source credits cover every unresolved location,
including caller-held packets, concrete queues, audio cells and unread completions. Play reserves
one command credit for Stop; source traffic cannot consume it. Cancellation returns an unpublished
packet's ID. Only control cancellation or collection returns a credit. Protocol refusals return
the packet, while semantic delivery refusals retain an identified completion. No late boundary moves.
Acknowledged snapshots cannot rewind when completions are collected out of order.

One off-thread merger publishes ordered source actions. The concrete host captures one ingress FIFO
prefix before each callback and admits only that prefix. A fence arriving behind acknowledged audio
before Stop is refused; after Stop, explicit delayed source progress follows the serial recorder's
rules. The caller must collect that refusal and explicitly re-offer the needed fence after Stop,
or interrupt if the source cannot provide it. An unresolved fence keeps finalization unavailable.
Neither an ingress cut nor joining audio supplies a source acknowledgement.

After normal Stop, callback access joins before worker custody. Closing control publication leaves
serial source admission open for already queued or delayed fences. The joined owner may admit and
drain those packets without rendering, then return their completions. `reunite` requires matching
origins, an empty control credit ledger and empty runtime receipt cells, returning both intact owners
on refusal. The worker then owns the serial session and may finalize after explicit final fences.
Normal sealing precedes serial `close_completed`, source-quality retirement and explicit discard.
No result/discard API is available on split audio; a completion in a return queue still blocks reunion.

Device loss instead closes serial admission after callback join, freezes acknowledged selection and
cancels pending work. Every queued, unpublished, failed-transfer and runtime outcome must be resolved.
Independent source quiescence is still required before interrupted finalization. No final callback
is needed. Concrete queue backing and both owners must survive callback join; the core handles alone
are not a physical-backend lifetime proof.

Transfer metadata has an explicit byte ceiling in addition to existing session storage. Concrete
host queues require separate bounds. The development ringbuf harness checks real OS-thread transfer,
whole/64/256/irregular callback equivalence, exact-boundary Stop, completion pressure, late-fence retry
and recovery after stalls or loss. Repeated split/reunion must never reuse a transfer ID.
Core hot methods join the source-level purity scan; concrete queue
operations are allocation/destruction guarded. This consumer does not qualify physical timing,
independent-clock source merging, live raw observation, restart, audio capture or monitoring.

## Unresolved questions

| Question | Blocking? | ADR or task |
|---|---|---|
| Platform clock mapping, drift law and latency evidence | Before production timing consumption and Phase 9 exit; simulated lifecycle work may proceed | ADR-0022 |
| General compatible state and live-note migration | Before its first consumer; ADR-0083 accepts only exact native program/layout continuation, and ADR-0082 accepts compiled running reset | ADR-0010, ADR-0048; accepted subsets ADR-0009, ADR-0050, ADR-0082, ADR-0083 |
| Complete producer capacity calibration | Before a production live adapter | ADR-0054 |
| Backend capability and callback-quiescence proof | Before enabling that backend; unknown required bounds refuse. The [Linux output harness](#linux-output-custody-harness) proves the bounded ALSA output-custody subset under ADR-0063; input and physical clock qualification remain open; ADR-0064 covers its concurrent compiled-session queues | IO-INV-002, IO-INV-003 |

## Linux output custody harness

[ADR-0063](../decisions/ADR-0063-linux-cpal-callback-custody.md) fixes the first
physical callback ownership mechanism in the non-shipping `v2_cpal_output`
example. It enumerates only ALSA endpoints and opens an exact ID. Run:

```bash
cargo run -p pertylizer --example v2_cpal_output -- list
cargo run -p pertylizer --example v2_cpal_output -- run alsa:hw:CARD=0,DEV=0 100
cargo run -p pertylizer --example v2_cpal_output -- transport alsa:hw:CARD=0,DEV=0 100
cargo run -p pertylizer --example v2_cpal_output -- plans alsa:hw:CARD=0,DEV=0 100
cargo run -p pertylizer --example v2_cpal_output -- loops alsa:hw:CARD=0,DEV=0 100
```

The endpoint above is an example, not a default or an assertion that this card is
present. The command uses the endpoint's default mono/stereo F32 or I32 format;
other formats refuse explicitly. It prepares a silent graph at the selected rate
and actual negotiated period. Its 8192-frame preparation ceiling is a memory
budget, separate from the ALSA period contract. The data callback borrows the
retained state, validates the entire output shape, renders to prepared scratch
and converts samples into the device buffer. Failures leave prefilled silence,
retain scalar diagnostics and cause the command to fail. Counters cannot wrap.

The `transport` mode queues initial Play before Ready, then collects four applied
Play/Stop/Play/Stop receipts. Later commands use a fixed 8192-frame scheduling lead,
which is a harness choice, not a latency guarantee. A late/refused command or a
callback target reached before all four receipts makes the run fail explicitly.

The `loops` mode chooses the exclusive compiled loop owner before Ready, with
interval `[0,513)`, entry zero and a four-pass observation journal. It has no
ordinary transport or plan-control owner. Each whole backend callback is one
journal render call; an early successful quantum cannot conceal a later failure
in that callback. The reported clock is the last whole-call acknowledgement.
Observation exhaustion preserves its first terminal endpoint while playback
continues. Joined close finishes observation without another callback and reports
the initial state, retained boundaries and terminal before propagating a run error.
Backend faults remain separate diagnostics; observation finish is not a complete
recorded take or a hardware-time certificate.

Ready follows preparation and initial publication. Shutdown clears Ready and joins
the ALSA stream
before any renderer, control, plan, buffer or diagnostic backing can be finally
released. The callback may already be in flight when shutdown begins. Both
explicit close and owner destruction enforce the order, including failures
between open and play and loss without a final callback. The owner is private,
neither `Send` nor `Sync`, and exposes no loose pair of lifetime handles.

The command reports negotiated period, delivered frame count, render clock,
callback count, faults and observed CPAL error categories after join. Success is a
bounded output/custody run,
not accepted physical timestamp calibration or Phase 9 exit evidence. Proxy
endpoints can exercise the harness but cannot qualify physical timing. Input,
MIDI, held-note plan swaps and loop session/capture integration remain separate consumers. The `plans`
mode exercises four stopped silent-plan publications, latest-wins delivery and
withdrawal, retaining every identified receipt. The example tests pin CPAL,
ringbuf, triple_buffer and crossbeam-utils behind this lifecycle proof.

## Split compiled session

[ADR-0064](../decisions/ADR-0064-concurrent-compiled-session-handoff.md) adds
`SessionControl` and `SessionAudio` under `host::session::transfer`. Fresh
preparation checks plan/profile geometry and publication bounds and uses the same
runtime builder as the serial host. Control owns the minter and admitted list;
audio owns renderer, registry, scheduler and slots. Only an immutable plan is
shared. The control metadata and command boxes are separately bounded; queue
storage remains a concrete host charge.

Preparing a command reserves one credit through delivery and collection. One
outstanding Play and the Stop reserve apply across all packet locations, including
unpublished and returned packets. An unpublished or rejected-send packet must be
returned through `cancel`; a completed one through `collect`. Collection resolves
the retained activation off-thread before returning credit. Failure returns the
owning packet. Older collected serials cannot overwrite a newer playback snapshot.

Command origins include the prepared table identity. Epoch and plan equality alone
cannot identify a table, including when one compiled plan is cloned and prepared
again. Scheduler and activation collection also preflight table identity.

A host drains a bounded queue prefix at its declared callback ingress cut, then
renders. Publication after the cut waits for a later callback. Commands delivered
before their requested boundaries preserve the serial host's FIFO, same-time
cancellation and sample behavior. Late delivery yields `DeliveryRefused`, including
for Stop; it does not silently apply later. A surviving Play must still match its
prepared stopped position at the actual boundary. These ordinary refusals never
fault the renderer or erase other audio. Protocol/origin/order/full errors return
the packet without insertion, and a host must retain any failed transfer.

`take_completed` moves a command box without freeing it. Backend shutdown closes
admission, fences callbacks and only then calls `close_after_quiescence`; remaining
runtime commands become retained cancellations. The host also recovers queued and
unpublished packets. A stalled completion reader cannot delay an already admitted
Stop. A terminal render fault silences the complete buffer, prevents later rendering
and leaves commands collectable after the fence.

The core tests use two OS threads and allocation-guarded callback operations, with
blocking test rendezvous outside those operations. The Linux example separately
exercises the [queue owner](../decisions/ADR-0064-concurrent-compiled-session-handoff.md):
control and both queue backing allocations survive through ALSA join, while the
occupied callback pool retains audio and failed transfers. Saturation, ingress-cut
ordering and disappearance without a final callback have their own tests. Capture,
live ingress and loops remain unavailable on the split pair until their separate
consumer contracts are built. Stopped plan readmission is specified below.


### Stopped compiled-plan readmission

The split session's stopped-only replacement follows
[ADR-0064](../decisions/ADR-0064-concurrent-compiled-session-handoff.md#stopped-prepared-plan-readmission).
Five credits bound all prepared candidates and their owning retirements, including
unpublished values. Isolated compiled notes may retire with their old owner as
specified below; their count is explicit and their identities never migrate.
Ordinary transport admission remains frozen until every credit
is resolved off-thread. Installation preserves device epoch and actual clock,
requires empty outgoing carry and no external note obligations, and returns an identified
installed/refused outcome with the owning resources. The Linux example supplies
the [concrete mailbox](../decisions/ADR-0064-concurrent-compiled-session-handoff.md#linux-latest-wins-plan-mailbox):
three optional owning cells, a one-slot return queue and retained failure storage.
It compiles before publishing, immediately cancels overwritten candidates off-thread
and defers installation under retirement pressure. Withdrawal needs no extra credit.
Joined cleanup recovers every cell without requiring another callback. Other hosts
must establish their own transport and custody proof.

### Experimental running compiled reset

[ADR-0082](../decisions/ADR-0082-running-compiled-plan-reset.md) extends ADR-0064
and ADR-0009 with a default-off compiled-only running reset through
`PreparedPlanReplacement`. Preparation requires acknowledged Playing, zero ordinary
credits, no other candidate and no external producer declaration. A fresh pair, Q-frame
scratch and admitted timeline exist before a future aligned boundary B is selected.
The preparation event limit bounds cold timeline width, not producer density. The
application uses 16,384 events as a provisional cold limit, with no producer reselection.

The successor is bootstrapped off-thread at B and the old anchor's position P through
its own ordinary Play activation. Adoption and collection finish without rendering a
DSP quantum: clock B, carry zero, empty exchange/credits, and catch-up/release charge
owed for the first actual quantum. New generation, epoch, table and issuer counters stay
intact. The payload is private until installation and its initial debt cannot be used
for a second activation. If the new song has ended at P, the successor is instead fresh
Stopped(P), without debt. Position is never clamped to the edited end.

Audio verifies source origin, command watermark, Playing anchor, exact B and empty
carry, plus the successor certificate. Late or changed-state attempts return Refused
without changing old audio. A successful exchange keeps both owners for exactly Q output
frames with old*(1-t)+new*t, t=(frame+1)/Q. The final frame preserves new sample bits.
Old registry and authoritative minter end together only after the fade, off-thread;
no old identity addresses the fresh table. No live/mixed release law changes.

The owning return appears only after the fade. Collection clears the old credit before
promoting the fresh controller with its own counters and ledger. Return pressure retains
both owners and blocks new transport/edits. Any render failure silences the whole outer
callback, retains both owners, and returns Interrupted for closed-origin teardown. It
never resumes the old plan or acknowledges clean audio. Explicit application Stop and
disable retain whole-wrapper mute/retirement without needing another old callback.

The application permits one attempt per edit. Compile/preparation/late refusal retains
the previous sound and metadata and defers to pause, Stop or a newer edit. A pending Pause
is applied after owning collection unless a later Play cancels that unsent Pause.
A boundary inside the ordinary four-margin end-stop
lookahead defers to that stop. A shortened new end after P uses ordinary end-stop
scheduling after collection; a stalled collector can therefore stop later than that
end, as with ordinary delayed control polling. Success updates metadata only for the matching install
sequence after successful audio; declined publication restores the previous metadata.
Each renderer is separately admitted; aggregate transition CPU, complete transient
cold-memory qualification, live ingress and production enablement remain open gates.

Conformance: `host::session::transfer::replacement::running::tests` covers bootstrap,
crossing-note cuts, fade arithmetic and endpoint under callback partitions, end-shortening,
late refusal, fresh issuer continuation, in-flight cancellation refusal and interrupted
custody. `lowering::app::edit_tests` covers actual saved-project publication, pending Pause,
late single-attempt metadata recovery and whole-wrapper Stop retirement. Allocation
controls cover installation and rendering. A false audio/receipt claim, identity leak,
callback allocation/destruction or partition-dependent endpoint blocks acceptance.

### Experimental native compiled voice continuation

[ADR-0083](../decisions/ADR-0083-native-compiled-voice-continuation.md) adds
`RunningEditMode::ContinueOrReset` to the owning running replacement lane.
It retains the actual renderer and authoritative minter for an identical native
physical layout and complete ordered compiled program. Only authored non-voice
Trim bases may change, with no smoothing, automation or note-magnitude writes to
those rows. All other prepared state/configuration, authored IDs, execution
scopes/tags, slot assignments, voice policies and tuning must match. Plan-local
mixer, parameter, note and passive tap addresses are checked before normalization.
This SessionAudio API has no observer subscription handoff. Script/controller,
feedback, sampler, loop/capture and external-source cases remain outside the proof.
Unsupported programs choose the preceding running reset; binding/resource failures
retain old sound and surface a diagnostic.

Before Play publication, an optional owning witness copies the exact stamped
activation list, bounded by count and actual capacity. Only applied Play collection
makes it active. The proof binds plan, epoch, table, activation sequence and anchor;
stale metadata is retired cold and selects reset. Two cold successor event copies
are relabelled without minting: the renderer's list and the next active witness.
Every occurrence/generation/time/order is retained. The actual minter's live keys
are checked before publication and rebound at collection without resetting issuers,
generations, ledger or free/retired indices. There is no identity-table rebuild.

At exact aligned B with zero carry, unchanged Playing anchor and no ordinary
credits, preflight validates every runtime binding before mutation. A changed
Trim row must be exactly fresh base state or the static base override restored by
ordinary Play, including absence of ramps and dynamic contributions. Installation
sets the complete row to its new base and retains actual voice/DSP state, clock,
anchor and scheduler cursor. Old plan/event/catch-up owners move into the envelope
for off-thread collection. Ordinary render cannot bypass this transition. Installed
requires the first successful nonempty successor render; zero frames remain pending.
Interrupted closes the actual successor binding for teardown without resuming old
audio. Owning return pressure freezes transport and edits until collection.

The application requests this path only with an unchanged authored end. With a
tracked GUI identity source, authoritative graph lineage must hold through the
post-preparation publication check. Lower-level callers without an attached
authority must ensure graph identity continuity themselves. It retains the reset
lane's Pause/Play, Stop/disable, one-attempt and latest-wins rules.
Prepared kind determines the queued-intent notice; rendered installation
determines identity and omissions. Identical whole
note programs deliberately identify the same performance here, without adding a
persisted placement ID. Graph deletion/recreation invalidates lineage separately.
Initial and private-reset witness failures are reported without disabling ordinary
Play/reset. `ContinuationLimits` bounds count, witness capacity and transition
capacity separately. The provisional application limits are 16,384 stamped events,
4 MiB per witness and 4 MiB per transition, without producer reselection. Command
credits bound queued witness owners, plus one active witness; a continuation charges
two new event copies, proof and fixed payload. Retired owners remain held until
collection. Aggregate transient memory/CPU and production qualification stay open.

Conformance: `host::session::transfer::replacement::continuation::tests` compares
continued held voices and stealing/bend state with normal single-plan references
at four rates and callback partitions. Negative controls cover foreign bindings,
changed programs, capacity, stale witnesses, zero-frame custody and interruption.
`lowering::app::edit_tests` checks real saved voice/insert/return projects, repeated
edits, lineage cancellation and reset followed by continuation. Any false identity,
receipt or real-time claim blocks acceptance. Structural sounding-voice relocation,
changed voice programs, mixed/live migration and observer handoff remain gated.

### Experimental compatible global-state readmission

`SessionControl::prepare_compatible_replacement` opts into a bounded internal
experiment on the stopped compiled-session path. The ordinary replacement keeps
its reset behavior. This experiment does not accept ADR-0010, migrate sounding
voices or change the mixed-reset law.

The compiler retains authored non-voice node IDs, exact execution scopes and
runtime state slots; that table's capacity is charged by host table accounting.
Preparation matches node and parameter IDs, requiring equal scope (including
channel/bus tags), native prepared configuration, output layout and complete
parameter metadata. Runtime slots are addresses, never identity. New, changed
or ineligible nodes stay fresh. Sends remain fresh because application send IDs
are positional. Note targets and note-magnitude destinations also stay fresh,
even outside Voice scope, so retirement cannot leave an orphan gate or pitch.
VM/sample table references are not migration signatures.
Modulation, script/controller routing and feedback refuse this path. Stateful
unidentified steps, including latency compensation, refuse too. Inert conversion
copies outside voice sum/instance groups may remain fresh. Voice state stays
fresh: preserving channel or bus histories is not sounding-voice continuity.

The map binds both plan IDs; installation additionally requires the exact
outgoing identity table, including after a same-plan republication. It checks
every source/destination state and history extent, the one-to-one node map and
all parameter spans before mutation. Map arrays retain their fallibly reserved
capacities, use allocation-free unstable sorting and perform no shrink-to-fit
conversion; their reported bytes cover every retained capacity. Installation copies small current state
and parameter records and swaps preallocated per-node history owners. The
outgoing owner receives the incoming fresh history buffers. Work scales with
matched records, not delay length; no history payload is copied, allocated or
freed in the callback. History accounting charges one Box owner per state
record, without a terminator, plus every sample payload. Clocks, carry and note
identities are not copied.

An isolated compiled-only stopped owner may retire its old note registry and
minter with the old pair. The receipt reports `retired_compiled_notes`; the
incoming registry stays fresh. Any declared external producer range, adopted
ingress store, outstanding activation or mixed partition retains refusal. A
chained, uncollected intermediate minter with compiled notes also blocks
readmission. Play uses the existing activation law: crossing notes are cut,
parameter catch-up is applied and later onsets sound. No note is retriggered to
pretend voice continuity. Other stopped/command checks still apply. A migration refusal
retains the owning candidate and fences older publications like other non-origin
refusals. Five credits bound the number of owning maps. Each preparation gives
its map one fifth of that call's remaining control-byte allowance after charging
the incoming controller. `migration_bytes()` reports that map separately from
`control_bytes()`. Different limits across calls do not form one aggregate byte
ceiling; the host budgets all outstanding owners and their transport. Collection
destroys the map and outgoing owner off-thread.

The migration tests compare continued oscillator/filter/delay rendering with a
stopped compatible replacement after runtime slots move, at four sample rates
and 1/64/256/irregular callbacks. Ordinary reset is the audible negative control.
Separate controls cover changed state, unsupported routing/compensation, foreign
plans, same-plan table changes, budget refusal and atomic storage preflight.
These are software checks; production capacity and hardware timing remain gated.

The experimental application now consumes this opt-in path for eligible edits at
an acknowledged positive paused position. One candidate is in flight, with one
forward cell, one return cell and one retained callback cell; its credit remains
held until off-thread collection. Transport is frozen meanwhile. A valid callback
drains old carry to zero before installation, splitting only when the boundary is
inside the block. Original callback geometry is checked before splitting. A
successful callback containing the boundary acknowledges the new installation,
including when its prefix drained old carry. The sequence travels with the owner;
a queued edit never borrows the previous plan's rendered identity.

A second edit waits for collection. Unsupported routing or unidentified state
falls back to an announced fresh restart. Transport and credit pressure wait. Ineligible producer declarations request
a fresh restart rather than retrying permanently. Failures returned during preparation, including storage, allocation or
map-budget failure, silence V2 and request leaving V2 mode through the V1-reset
path; the old project is not resumed. These failures end V2 mode even when the
fresh unsupported-graph fallback
would not need a migration map. A refused installation, including migration
storage failure discovered during preflight, or an uncollectable return requires a
fresh session. A pending explicit Stop retains fresh-start intent through
switch-ring pressure. Stop, fault recovery and position-zero edits retain
fresh-session behavior. If an eligible edit shortens the arrangement below the
retained position, the existing end-stop policy applies
on resume; position is not clamped. Disable/Stop retire the complete old audio
wrapper and both lane endpoints on the control thread, including unattempted
candidates when no final old-session callback occurs. These controls establish
no capacity or endurance selection. The application cold control allowance is
65,536 bytes, including the existing five-credit envelope accounting and the
map's remaining one-fifth allowance; event shares and callback limits are
unchanged.

Production GUI snapshots bracket authoritative EngineState projection with a
monotonic, session-local module/effect lineage counter. Removal, clearing or
replacement of identities invalidates continuity, even when an intermediate
snapshot is never observed. Song-side bus deletion, rollback and undo also
invalidate under the Song write guard, independently of command-ring admission.
Whole-Song project loading and MCP song replacement do the same, including a
load whose commands fail. This fence does not synchronize Song mutations with
all engine-command publications or repair their existing V1 state semantics.
In particular, MCP song replacement can retain the engine's old return-effect
chains; a later reused bus ID can expose that stored chain. The lineage fence
prevents old DSP-state inheritance, not that separate model/projection defect.
Atomic project/mixer transactions remain Phase 10B work. Only admitted plans
reach the compatible guard: V2 refuses duplicate bus IDs and IDs outside its
eight-bit address field.
The lineage counter's saturation permanently disables this path. The
counter is checked again after preparation before publication; a changed
counter cancels the complete unpublished candidate and returns its credit.
This is not a persisted identity or canonical project revision. Lower-level
callers without an attached authority remain responsible for identity continuity.

Controls cover scope/tag changes, moved runtime slots, exact owner addresses,
atomic refusal of aliased maps and damaged storage, compiled note retirement
and refusal for a declared external producer before its first note. Application
controls exercise real saved voice, insert and return-bus projects, preserving
paused position and rendering later onsets audibly with no callback allocation
or final deallocation. Removing an automation lane restores the new base at
resume, and a note-driven non-voice envelope cannot inherit a retired gate.
Coalesced same-ID replacement and mutation inside a
snapshot force fresh pairs. Changed kernel configuration and unsupported graphs
still reset. Running native voice continuation has the narrower exact-program
contract above; general voice migration remains open.

The bound mixed host separately permits a stopped reset to a fresh owner of an
edited plan with the same rate, layout and maximum block size. It retains the
existing crossfade and reset tombstones; it does not copy DSP or voice state.
The two-source host controls compare changed successor audio, unchanged raw
receipts and complete retirement under whole and split callbacks. A rate change
returns the complete successor for off-thread teardown and device re-prepare.

## Simulated input clock and capture

[ADR-0069](../decisions/ADR-0069-simulated-input-clock-and-capture.md) adds a finite
serial `host::input::InputCaptureSession` over the loop recorder. Each input owns an
independent connection generation, selected endpoint, configured synthetic clock and
fixed observation/receipt cells. Preparation returns Ready; start is explicit.
Retirement and a new same-endpoint attempt require resolved observations and input
quiescence. Stale callbacks cannot affect a replacement or restart the old take.

The clock maps synthetic ticks using a declared rational rate and source origin;
only uncertainty wholly inside one engine-frame bucket yields an exact simulated stamp.
Each message retains its original tick and arrival. Frontiers promise both that every
prior arrival is strictly earlier and every future nominal is at or after the frontier.
The merger orders by arrival, fences first at equal time, and withholds equal-frontier
messages until every participating source advances. These promises preserve the existing
zero-lateness recorder contract. They are not hardware calibration or queue observations.

Full input storage retains accepted cells and the first refused observation. A full
serial source lane leaves cells queued; other delivery refusals retain their actual
identified outcomes and interrupt. Stop cannot wait for a stalled source. Delayed
fences may finish after Stop, but queued arrivals behind the recorder's Stop floor
remain explicit refusals. Peer-source withholding itself can cause that lateness even when the other producer
honors its frontier promises. Complete recording under arbitrary worker delay is not promised.

Loss closes admission and freezes acknowledged selection. Each input needs its own
simulated quiescence acknowledgement; finalization needs no later audio callback.
Normal completion also closes sources before retirement. Release returns the intact
take and input owners only after all receipts and quiescence are resolved. Callback
paths allocate and deallocate nothing; all observation storage has explicit byte ceilings.
The ADR names the serial conformance checks. The split consumer below supplies
concurrent simulated input; physical clocks, qualified worker timing, audio capture
and monitoring remain separately gated by IO-INV-004/005 and ADR-0022.

## Threaded simulated input capture

[ADR-0070](../decisions/ADR-0070-threaded-simulated-input-capture.md) joins the input
merger to finite loop transfer. A fresh `InputCaptureSession` splits into
`InputCaptureControl`, `InputCaptureAudio` and an independent terminal halt handle.
The merger owns input cells and core transfer credits; audio owns the complete
renderer, journal and recorder. Prepared budgets cover both halves and shared
signal storage. The host separately charges fixed queues and failed-send cells.

Producer queue acceptance precedes model admission. The host retains every
queued original observation through admission or an explicit returned refusal.
The merger assigns input identities and retains original clocks/observations until
input-receipt collection, including while core packets or completions are in transit.
A returned core completion frees its core credit, not its input cell. Source
cancellation interrupts instead of silently omitting accepted input from a Complete
take. The serial and split consumers share the explicit-prefix arrival-order rule.

A monotone terminal signal works independently of queue space or worker progress.
Audio polls before packet admission and rendering, and publishes closure after an
audio-originated fault. A pending halt refuses new packet custody; rendering or joined recovery then
closes lanes, retains outcomes and freezes
the last whole acknowledged callback. A racing later request is handled at the next
poll or after callback join; successful audio is never retroactively silenced.
Terminal Stop produces Interrupted, without an ordered Stop receipt. Normal queued
Stop retains its timestamped outcome. The first requested terminal cause wins;
retained input diagnostics may describe a later different cause. When the merger
observes a terminal signal, previously undiagnosed inputs receive `PeerInterrupted`.
Here that marker denotes imposed closure, including user Stop and backend halt;
it does not identify a failed peer. Original diagnostics remain intact.

No result, mutable inner owner or source-quiescence operation escapes either half.
After joining callback access, the host resolves all transfers and reunites ownership.
Merger-side late/uncertain diagnostics reach the raw recorder before results become
readable. Uncertainty attribution for an interrupted active take uses its final
frozen selection; a diagnostic outside that interval remains retained without a
quality marker. Finalization is unavailable while split, so even a fault after ordered Stop
interrupts before a Complete result can escape. After normal reunion/finalization,
the serial late/uncertain Partial rule still applies until source retirement.
The host joins each producer and resolves its queued observations
before explicitly acknowledging that source; audio join alone is insufficient.

Failed owning operations preserve their owners, including an opaque serial owner
if quality reconciliation cannot complete. Reconnect still requires retirement and
a fresh generation, returns Ready, and cannot reuse a previous terminal signal.
The ADR names real-thread, pressure, fault, recovery and allocation conformance tests.
This consumer does not qualify complete recording under arbitrary worker delay,
physical clocks, live audition, restart, audio capture, monitoring or Phase 9 exit.

### Continuous simulated delivery experiment

The internal continuous test host exercises ADR-0070 over repeated callback cuts.
Two input producers, one merger and one audio owner remain on separate threads for
one finite take. Only Play, Stop and initial source fences are queued before audio
starts. Eight waves of new observations are then generated after audio captures
its prefix for each service period. Audio proceeds without waiting for those
producers or the merger; packets sent after the cut wait for the next period.
Fixture channels coordinate the logical schedule outside callback work. Actual
wall-clock overlap depends on OS scheduling and is not an acceptance condition.

`InputCaptureControl::packet_input_id` correlates an outstanding source packet
with its retaining input cell. The host reports an explicit producer frontier
only after successfully pushing that packet and every preceding FIFO packet.
Packet preparation, queue occupancy and returned-credit counts cannot substitute
for that proof. Command and foreign packets have no local input correlation.

The tested profile uses 256 device-output frames per logical service period,
512 frames of synthetic lookahead, and the existing 64-frame prepared silence.
A wave generated after period r's cut must be queued before period r+2's cut.
The merger may service it during r or r+1; servicing it during r+2 is too late
for that cut. The host requests ADR-0070's terminal halt before rendering across
a missed cut. Original timestamps are never moved to rescue the take.

| Resource | Admitted test-host ceiling |
|---|---|
| Input cells per source | 16, including retained input receipts |
| Core source credits | 32, including packet and completion transit |
| Command credits | 4, with the existing Stop reservation |
| Producer queue per source | 9 observations; 12 fixed producer retry cells |
| Merger-to-audio queue | 32 packets |
| Audio-to-merger queue | 8 completions |
| Transport backing and association storage | 64 KiB; fixture channels and result logs are separate |

These are selected sufficient capacities for this finite fixture, not minimal or
production-live sizes. The eight waves contain 32 note messages and 16 explicit
frontiers, plus two initial frontier receipts. Stop is at frame 2816; output spans
3072 frames. The same original independent-clock input produces identical PCM,
raw records, pass/carry metadata, 50 input receipts and two command outcomes under
256-frame, 64-frame and irregular callback partitions at either supported merger
service delay. Occupancy observations can vary with thread interleaving; assertions
check the declared ceilings, not an invariant benchmark high-water number.

The tests run late-delivery and undersized-queue controls before the supported
matrix. A missed deadline, stalled source frontier or stalled merger interrupts
without losing an accepted observation or a returned refusal. Recovery joins
producers and audio, resolves queue/retry cells and receipts, and requires each
source's explicit quiescence acknowledgement. A loss case supplies no final
callback and verifies old-take retention and fresh-generation reconnect.

The executable checks are `continuous_delivery_controls_precede_supported_deadline_and_capacity_matrix`,
`continuous_source_and_worker_stalls_halt_at_the_declared_cut`,
`continuous_no_final_callback_loss_retains_old_take_and_reconnects_explicitly`, and
`packet_correlation_identifies_only_its_retaining_input_owner` in
[`threads/continuous.rs`][continuous-input-tests].
Run them with `cargo test -p synth_engine_v2 continuous --lib`.

[continuous-input-tests]: ../../../crates/synth_engine_v2/src/recording/notes/loop_capture/tests/ordered/input/threads/continuous.rs

This is a logical delivery-budget experiment with an enumerated fixture and
controlled service delays. The driver can wait between logical periods, and the
claims exclude OS response-time guarantees, hardware timestamps, production
buffer sizing and arbitrary load. Physical timing, live audition and the complete
producer qualification remain under their existing first-consumer gates.

### Host-owned mixed transfer rehearsal

[ADR-0078](../decisions/ADR-0078-host-owned-mixed-transfer-rehearsal.md) permits a
default-off, non-shipping Rust API for one bound initial plan with one compiled
and one live note producer and one simulated live source. The host supplies one
empty bounded command queue and one empty bounded result queue, each with at
least the admitted ordinary ingress depth `C`. Both queue halves remain with
the host and are lent to control calls and audio callbacks. Note-on, channel
bend and the exact key-up are the only exposed requests. The ordinary
`PerformanceIngress::prepare` mixed-plan refusal remains in force.

An onset receives a handle only after control has reserved its result and
key-up places. A positive sound result comes from audio service, never from
source staging or command push. Control returns the original observation on a
pre-consumption refusal. A terminal fault publishes one halt to both halves;
the observing callback and later callbacks are silent. Joined off-thread
teardown accounts for outstanding command IDs, handles and queue packets. A
missing returned queue endpoint reports incomplete custody explicitly. Its
sounding identity report comes from the stopped renderer registry, so a
future key-up served before publication still names the sounding note; a
snapshot failure is returned explicitly.

The rehearsal does not connect concrete source inboxes, the raw recorder or
production capacity admission. It does not admit pedal, Panic, plan replacement,
loop playback or a second source. The following bridge and ADR-0081 extend this
initial surface; physical timing and Phase 9 exit remain open. The non-test `mixed_transfer_rehearsal` integration
target uses the `mixed-transfer-rehearsal` feature; the same feature's library
tests inject faults and count complete callback allocations.

### Two-source mixed bridge rehearsal

The `mixed-transfer-rehearsal` feature also offers a concrete two-source bridge
over the host-owned transfer. Its two source rings keep independent connection
generations and packet serials. The bridge stages each source's messages and
frontiers through a bounded peer fence, forwards admitted sound commands, and
retains exact source and command custody through joined teardown. The
`mixed_source_bridge` integration target exercises this bridge with a separate
raw recorder owner and host-owned queues. This is an experimental host shape;
[EVD-0026](../evidence/phase-09/EVD-0026-combined-host-capacity-selection.md)
selects capacities for the frozen serial stereo rehearsal;
[EVD-0027](../evidence/phase-09/EVD-0027-mono-mixed-host-capacity-selection.md)
selects its explicit-fold Mono counterpart. Both are bounded to the declared
64-frame serial host. Production admission, application mono-device admission,
other block sizes and continuous/concurrent capacity qualification remain open.
The functional independent-cadence control below is separate from these measurements.

The [persistent saved-project audio-owner control][mixed-thread-owner-control]
keeps the callback on one worker through joined shutdown. Two source workers and
control use their own halves; prepared reset/activation owners and complete
retirements cross bounded SPSC mailboxes. Stereo and explicit-fold Mono output,
raw receipts and normalized sound outcomes must equal the serial oracle at the
four candidate rates. While the full retirement mailbox holds an uncollected
Start, the audio worker must retain a refused stale candidate in its pending slot
and continue rendering, with no callback/handoff heap activity and no extra final
render. Control unwind must join workers before destroying the control halves.
The logical clock grants one callback after control pumps
its publication window. This short cross-thread lockstep control does not qualify
free-running delivery, continuous scheduling, concurrent queue peaks or capacity,
other block sizes or physical timing.

A separate one-place raw-completion regression in the linked test module exercises
`HostCallback::service_between_callbacks` on the default serial fixture. Completed
raw packets must publish after control frees a completion-ring place, without
another render. The older paced source/control/audio thread test calls this helper
while waiting for render credit; its retry and audio loops have bounded deadlines.

The separate [finite-prefix prototype][mixed-prefix-control] prequeues one saved
project and both source prefixes before handing the callback to its worker. It
then renders 1,024 output frames without per-callback permits or control service.
Stereo/Mono audio, raw receipts and normalized sound outcomes match the serial
oracle for 64-frame, irregular and single-1,024-frame partitions at all four
candidate rates. This functional fixture declares a 1,024-frame maximum block;
its sizes are not EVD-0026/27 capacity or timing qualification.

The test-only startup snapshot distinguishes admitted source frontiers from raw
frontiers successfully sent through the callback's packet ring. A full raw
preparation queue can return no packet without an error; neither that result nor
a failed ring push establishes raw delivery. The sound proof uses the actual
publication-window wait. Separate 64/256-frame maximum-block controls expose
unforwarded heads instead of silently widening their window.

Both startup renderers have one quantum of initial output carry. The guard
checks complete opened quanta, including partial-callback rounding, before either
owner renders. Missing source, sound or raw coverage latches a host-owned reason,
silences the complete failing callback and later output, and requests the existing
host/raw halts. This reason is kept separately from the engine bridge's fault;
no engine fault is invented. The prototype's halted control collects without
staging or pumping late refill. Joined teardown classifies every staged or
unserved packet and retains sounded open/sustained handles, without a final
render. Invalid output shape stays retryable; clock exhaustion closes the host.

Those prefix controls qualify startup only. The independent-cadence control below
adds rolling delivery and reset coordination; concurrent capacity and physical
timing remain unqualified. Production mixed refusals stay in force.

[mixed-prefix-control]: ../../../crates/synth_engine_v2/tests/support/autonomous_prefix.rs

[mixed-thread-owner-control]: ../../../crates/synth_engine_v2/tests/support/concurrent_saved_host.rs

### Combined software host and strict publication

[ADR-0081](../decisions/ADR-0081-combined-host-software-boundary.md) accepts the
bound default-off common software host. `MixedBridgeControl::route_raw_first`
performs raw admission before sound staging, keeps the source-ring identity and
returns raw and sound outcomes separately. `MixedRawPolicy` implements
[ADR-0079](../decisions/ADR-0079-mixed-host-raw-capture-interruption.md): only an
actual classified late raw halt stops raw alone; unknown/terminal outcomes and
direct raw errors stop the host. The exact receipts and errors remain retained. Classification is bound to the
raw source generations. An interrupted raw proof exempts raw coverage only after
a classified late halt, retaining its actual last queued frontier; sound and
terminal checks still apply.
The tolerant bridge still fences delayed peers and records late sound separately.

The optional strict mode requires promised source prefixes. `MixedPublication`
is minted from actual stage heads, frontiers and the publication window.
`MixedRawPublisher` produces a source-generation-bound raw proof only after real
FIFO sends succeed. A failed push and packet preparation grant no credit.
A latest-value channel copies complete pairs through atomic fields bracketed by
checked versions. The callback makes one bounded read attempt before draining
commands, checks actual renderer carry/quantum demand, and never combines pieces
of two updates. Missing strict coverage latches the first host reason and
silences the observing callback and later output before raw rendering. Output
shape/profile validation precedes this source preflight. Strict mode is not the
tolerant peer policy and does not reclassify a raw-only interruption.

A sealed transition proves that no source work is due before its end and binds
both arms and the reset boundary. The wrapper rejects an old-arm proof on the
successor or a wrong transition when crossing a pending reset. Source/recording
ownership survives the sound reset. Old-source key-ups keep their reset
classification; new sound never reinterprets them as a newer key's release.
Retirements are offered immediately after the render that produces them. A full
return channel retains the entire owner while callbacks continue. A missed reset
offer returns its fresh owner for abandon and off-thread rearming.

The [continuous saved-project control][mixed-continuous-control] uses one startup
barrier and then an independent software callback clock while two source workers
and control roll forward. At four rates, Stereo/Mono and three partitions, its
16,384 output frames match an unguarded serial audio/raw/sound oracle, with and
without held-note reset, compiled seek and retirement backpressure. Callback
allocation probes cover publication reads, preflight, owner handoffs and both
renderers. Separate controls check control loss, slot refusal, source/head caps,
foreign raw/arm proofs, missed reset rearming and post-render return service.
Mailbox unit tests check unread replacement, in-flight reads, version exhaustion
and concurrent whole-pair consistency.

These are functional software controls with a separately declared 1,024-frame
maximum block. Their software clock is independent of device sample rate; there
is no timing-budget, concurrent-capacity or physical-device acceptance claim.
Production mixed ingress, application mono-device admission, representative
complete reselection and ADR-0022 physical qualification remain gated.

[mixed-continuous-control]: ../../../crates/synth_engine_v2/tests/support/continuous_saved_host.rs

### Finite audible simulated capture

The non-shipping `capture` mode of `v2_cpal_output` connects reusable live hosting,
retained fresh attempts and pass-aware projection to ALSA callback custody. Its
counted recipe reserves recording after a fixed musical count-in. Independent
Core V2 renderers own compiled playback, real live ingress and the metronome;
ordered Stop or panic ends live notes and sounding clicks at the same engine time.
[ADR-0071](../decisions/ADR-0071-reusable-simulated-live-host.md) owns delayed start
and restart custody; [ADR-0072](../decisions/ADR-0072-finite-live-audition-and-count-in.md)
owns the original finite audition and count-in.
[ADR-0073](../decisions/ADR-0073-continuous-simulated-live-input.md) replaces its
whole-attempt quotas with reusable cells and adds bend and ongoing reconciliation.

The concrete example tests compare timely audio under whole, regular and irregular
callbacks and measure zero callback allocations. They compare retained raw input
with audition on/off beyond 64 observations and recover without a final callback.
The Linux mode reports identified refusals when delivery misses its reserved
boundary. The unpaced null backend is a custody/recovery test, not physical timing
evidence. Physical MIDI, platform timing and production calibration stay gated.

### Continuous simulated audition, swaps and audio input

ADR-0073 defines reusable result/held-key custody and a separate coalescing,
versioned parameter lane. Observation credit exhaustion is an explicit terminal
fault; Stop/panic retains separate custody. Completed raw annotations reconcile
before their audition cells recycle. Worker finalization seals after joined
recovery, including when no final callback arrives.

[ADR-0074](../decisions/ADR-0074-concurrent-live-host-and-duplex-capture.md) extends
ADR-0009's reset/crossfade owner with concurrent latest-wins preparation. Audio
acquires only at a quantum boundary, transfers unstaged future observations,
and retains completed old-epoch annotations until raw reconciliation. Rejected
candidates keep the old plan sounding. Stop/closed/faulted owners cannot be revived.
A bounded retirement queue and separately admitted mailbox retain all owners
until off-thread destruction. Whole-callback success precedes outcome acknowledgement.

The `capture` harness combines this transport with real Core V2 voices and
simulated MIDI recording. The `duplex <input-id> <output-id> <seconds>` mode
owns independent physical input/output and recording-worker endpoints. Original
PCM and CPAL timing metadata form an immutable in-memory take after input join;
monitor buffering, resampling and drops cannot alter it. Recording overflow and
worker capacity retain an exact prefix and first missing source frame.

Requested and negotiated buffers are reported separately. Reconfiguration stops,
joins and finalizes before fresh preparation. Earlier takes survive later failure.
Monitoring declares a 4,096-input-frame target and gain 0.1; recording applies no
compensation. Backend input/output latency estimates are separate, cross-stream
origins are not assumed equal, and physical round-trip latency remains unmeasured
until an explicit loopback path is characterized under ADR-0022.

`v2_live_session` retains the deterministic serialized simulator. The concrete
`v2_cpal_output` example tests concurrent custody, original PCM, allocation and
no-final-callback recovery. Physical timing qualification, anti-aliasing quality,
representative identity endurance and full production producer budgets remain open.

## Application song playback

[ADR-0077](../decisions/ADR-0077-experimental-v2-song-playback-in-the-application.md) owns
the application's first V2 playback path. Implementation follows these rules:

- The path lives under `pertylizer`'s `src/lowering/` behind the non-default `v2-lowering`
  feature; the application reaches it only through `crate::lowering`. A default build has no
  V2 code path and no V2 setting.
- V2 mode lowers the whole open project to one plan and plays it through the ordered session
  transport with its own play, pause and stop; the V1 transport does not drive it. Stop
  returns to the song start by preparing a fresh session. V2 offers no seek, loop or pattern
  preview. Entering and leaving V2 mode stops V1 and resets its DSP, so nothing V1 held or
  rendered silently is heard when the engines switch. Play and pause take effect two maximum
  callbacks plus a quantum after they are requested. Stop, a refused paused edit and leaving V2
  mode silence V2 from the next callback; leaving cannot fail for want of ring space. A
  project refused after a paused edit never plays: V2 is silenced and V2 mode is left through the
  path that resets V1 first.
- A project with any `Refused` lowering diagnostic does not play, and the notice names the
  first refusal. `Unrepresented` diagnostics play and remain visible. V2 mode never writes
  project state; loading and saving are unchanged.
- An edit while stopped re-lowers off the audio thread and installs through stopped-only
  replacement. An edit while playing can use the
  [isolated running reset](#experimental-running-compiled-reset), with previous audio
  retained on compile or pre-install refusal.
  At a positive paused position, eligible graphs use the
  [compatible global-state experiment](#experimental-compatible-global-state-readmission)
  to retain position and eligible unchanged authored non-voice records. Unsupported graphs restart
  from the song start with a notice; position-zero edits and explicit Stop also use a
  fresh session. Pending explicit Stop keeps fresh-start intent through switch-ring
  pressure. Transient transport/credit pressure waits; ineligible producer declarations
  request fresh restart. Paused-edit failures returned during preparation,
  including migration-map budget/allocation/storage failure, silence V2 and
  request leaving V2 mode through the V1-reset path. A collected paused installation refusal
  triggers a fresh restart, including migration storage failure found at installation.
  Play reports busy when a stopped replacement is pending. During a running swap,
  Play cancels an unsent Pause; other Pause intent waits until replacement collection.
  The rendered
  plan identity changes only after a successful callback containing the installation
  boundary.
- MIDI input is disconnected before V2 mode is entered, with a visible notice, and
  reconnects only once the audio callback has released V2. No timestamp-capable input
  reaches V2.
- V2 mode refuses to start, visibly, until an accepted evidence record has reselected the
  compiled and session partition that song playback admits under ADR-0054, over every saved
  project that lowers as a whole, and has shown that no other producer class is reached.
- V2 mode consumes no host timestamp or callback latency and claims no qualified timing.

Conformance: `lowering::tests::live` holds live playback equal to the offline render one
quantum after the play boundary, pause and resume at the paused position, the end stop and
the capacity gate; `lowering::live::tests` holds refused, returned and faulted commands; and
`lowering::tests::app` holds the application switch around the V1 processor with no callback
allocation, V2's own play, pause and stop, release only after the callback retires V2,
leaving V2 mode with a full command ring, silence from the next callback after stop or a
refused edit, a deferred edit, an oversized callback taking V2's terminal fault with a
visible re-preparation at the song start, and the refusals of an unlowerable project, a mono
device and an unknown stream. `lowering::live::paused_edit` checks audible compatible
continuation, partial carry, acknowledged pause, transport freeze, refused installation,
the original callback ceiling and retained foreign-command refusal.
`lowering::app::edit_tests` checks paused publication, repeated edits, explicit Stop,
full-ring Stop retry, retirement, positive-position preparation refusal and recovery
from a refused installation.
EVD-0025 qualifies the partition; the edit controls add no capacity selection.
The application toggle beside the MIDI indicator and the status-bar omissions badge are wiring
in `gui/egui_backend/v2_flow.rs`, verified by the feature build rather than a GUI test.
