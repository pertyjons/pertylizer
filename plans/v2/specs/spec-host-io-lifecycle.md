# SPEC: Host I/O Lifecycle

| Field | Value |
|---|---|
| Status | Current |
| Phase | 9 |
| Created | 2026-09-11 |
| Last reviewed | 2026-09-11 |
| Based on | ADR-0036, ADR-0024, ADR-0021, ADR-0032, ADR-0038, ADR-0054 |
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
  fence directly and has no pause dependency.
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
output's transport latch; ordered audible transport retains its first-consumer gate.
It has no concurrent pending source queue, hardware input, monitoring or audio
capture. IO-INV-005 remains unimplemented. Physical input loss, backend capture
fences, independent clocks, worker backpressure and actual telemetry/retirement
channel saturation still need the full checks below. Physical timing acceptance requires ADR-0022's retained
platform/adapter evidence and ADR-0054's complete producer calibration before a
production live adapter is enabled.

| Invariant | Full check still required beyond P09-S001/S005 |
|---|---|
| IO-INV-001 | Concurrent publication and backend callback fences; independently opened input generations |
| IO-INV-002 | Ordered session stop, real backend shutdown when pause is unsupported, concurrent off-thread retirement |
| IO-INV-003 | Evidence establishing any physical backend callback bound before activation |
| IO-INV-004 | Physical input loss, concurrent retained capture with/without final callback, and bounded automatic retry if implemented |
| IO-INV-005 | Independent clocks, bounded stalled-worker input and distinct monitoring/capture loss |
| IO-INV-006 | Saturated concurrent telemetry preserves faults and acknowledgements; input backlog and timing provenance |

## Unresolved questions

| Question | Blocking? | ADR or task |
|---|---|---|
| Platform clock mapping, drift law and latency evidence | Before production timing consumption and Phase 9 exit; simulated lifecycle work may proceed | ADR-0022 |
| Seamless plan changes, crossfades and live-note migration | Before the first consumer of each live change | ADR-0009, ADR-0010, ADR-0050 |
| Complete producer capacity calibration | Before a production live adapter | ADR-0054 |
| Backend capability and callback-quiescence proof | Before enabling that backend; unknown required bounds refuse | IO-INV-002, IO-INV-003 |
