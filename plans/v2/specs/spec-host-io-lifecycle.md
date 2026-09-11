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
This contract is accepted for the first consuming implementation. Its conformance
checks are required future work, not claims of existing code or measurements.

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

These are required future checks, not results. Physical timing acceptance still
requires ADR-0022's retained platform/adapter evidence and ADR-0054's complete
producer calibration before a production live adapter is enabled.

| Invariant | Required future check |
|---|---|
| IO-INV-001 | Simulated transitions and stale-generation commands/callbacks |
| IO-INV-002 | Rate/layout reprepare, unsupported pause, oversized callback in both build modes, off-thread retirement |
| IO-INV-003 | Unknown capability, ambiguous identity and explicit fallback refusal |
| IO-INV-004 | Loss with/without final callback; reconnect stays Ready and preserves retained capture |
| IO-INV-005 | Independent clocks, bounded stalled-worker input and distinct monitoring/capture loss |
| IO-INV-006 | Saturated telemetry preserves lifecycle fault and active-generation acknowledgement |

## Unresolved questions

| Question | Blocking? | ADR or task |
|---|---|---|
| Platform clock mapping, drift law and latency evidence | Before production timing consumption and Phase 9 exit; simulated lifecycle work may proceed | ADR-0022 |
| Seamless plan changes, crossfades and live-note migration | Before the first consumer of each live change | ADR-0009, ADR-0010, ADR-0050 |
| Complete producer capacity calibration | Before a production live adapter | ADR-0054 |
| Backend capability and callback-quiescence proof | Before enabling that backend; unknown required bounds refuse | IO-INV-002, IO-INV-003 |
