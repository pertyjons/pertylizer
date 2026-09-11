# ADR-0036: Audio Device and Input Lifecycle

| Field | Value |
|---|---|
| ID | ADR-0036 |
| Status | Accepted |
| Phase | 0B/9 |
| Created | 2026-09-11 |
| Last reviewed | 2026-09-11 |
| Related | ADR-0021, ADR-0022, ADR-0024, ADR-0032, ADR-0038, ADR-0054 |
| Supersedes | — |
| Superseded by | — |

## Durable boundary

This choice binds host resource ownership, the real-time boundary, recording
failure semantics and the user's experience of device loss. The user selected
this decision first, followed by ADR-0024, as preparation for Phase 9. Both are
entry decisions in the [master plan](../master-plan.md#phase-9-live-integration-and-immutable-plan-swapping).

The user accepted this lifecycle together with ADR-0024 on 2026-09-11.
Production-adapter enablement still has the gates below.
ADR-0022 still owns clock mapping and latency compensation; ADR-0024 owns the
recording result at an interruption. The coupled interruption boundary is
accepted together. Plan crossfades, live-note migration and general transport
activation remain with ADR-0009, ADR-0010 and ADR-0050's open live obligations.
An initial stopped-only adapter can implement the lifecycle without pretending
to implement those later live-edit stages.

## Decision boundary

Select one host owner for device discovery, negotiation, stream construction,
connection generations, callback lifetime, input buffering and recovery.
Device handles and platform callbacks never enter a compiled render plan.
Requested settings belong to host/application configuration; active negotiated
state belongs to the runtime session. Neither is added to the project format.
This decision changes no existing persisted format, wire contract or V1 API.

Three distinct records are needed:

- **Request:** selected endpoint identity or explicit system-default selection,
  desired rate, layout and buffer preference, and permitted fallback choices.
- **Negotiated configuration:** actual selected device/configuration and which
  requested values were substituted under that policy; preparation uses this.
- **Observation:** actual callback shapes, timing provenance, latency estimates,
  discontinuities and counters. An observed value is not a guaranteed limit.

## Evidence

Premises inspected at `f34f57f6`:

- [LIMIT-0001 and LIMIT-0057](../inventories/resource-limits.md) own the oversized
  callback failure and the hardcoded advertised-range finding. HOST-INV-003
  requires queried device capabilities, and the current host specification
  already defines an oversized callback as a terminal fault.
- [CAP-0008](../inventories/capabilities.md) identifies the existing input-device,
  monitoring and recording surface. `crates/pertylizer/src/audio/input.rs`
  currently owns separate engine and GUI rings and accumulates recording data
  on the GUI side. This proposal's worker ownership is a V2 design choice,
  not a claim that V1 already implements it.
- [ADR-0032](ADR-0032-sample-time-and-event-timestamps.md) fixes rate, layout and
  capacity for an epoch and rejects old-epoch events. A transport stop alone
  does not restart the render clock.
- The repository selects CPAL 0.18.2. Its
  [stream contract](https://docs.rs/cpal/0.18.2/cpal/traits/trait.StreamTrait.html)
  says the buffer-size query is an estimate, not a guaranteed callback bound;
  pause may be unsupported. Its
  [clock contract](https://docs.rs/cpal/0.18.2/cpal/struct.StreamInstant.html)
  does not guarantee a common origin between streams. Read on 2026-09-11.
- [EVD-0016](../evidence/phase-03/EVD-0016-host-time-mapping.md) and
  [ADR-0022](ADR-0022-hardware-time-mapping.md) own timing qualification. No new
  physical-device measurement or supported-platform claim is made here.

## Options

1. **Direct callback-driven recovery.** Reopen devices or resize resources as
   errors arrive. Shorter control code, but device operations and allocation
   cannot occur in the callback; configuration changes can race stale work.
2. **One explicit host lifecycle, prepared before activation.** More state and
   acknowledgement machinery, but every callback has a known configuration,
   owner and retirement condition. Recommended.
3. **Always switch to the system default and resume automatically.** Convenient
   for casual playback, but can change physical routing and restart monitoring
   or recording unexpectedly. Retain as an explicit future preference, not the
   initial default.

## Decision

### Ownership and state transitions

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

### Stop, reconfiguration and reclamation

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

### Unknown capabilities and fallback

A missing buffer bound is reported as unknown. Neither CPAL's buffer estimate
nor the largest callback observed so far becomes a queried maximum. The initial
adapter refuses activation if it cannot establish the capability required by
HOST-INV-003. An adapter-specific way to establish that bound must be justified
against its backend contract before use. Supporting devices without such a
bound requires an explicit successor to the affected host-profile contract;
this decision does not relabel a chosen ceiling as hardware capability.

Rate/layout substitutions must belong to the request's explicit fallback set.
The result reports the actual choice before activation. Ambiguous endpoint
identity refuses selection; equal names are not proof of the same device.

### Device loss and recovery defaults

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

### Input clocks, monitoring and capture

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

### Diagnostics and qualification

Expose requested and negotiated configuration, connection generation, lifecycle
state, active epoch/plan acknowledgement, failure cause, retry status, actual
callback size, input loss/backlog and timing provenance. Callback-side reporting
uses preallocated state/counters; formatting, logging and subscriber delivery
are off-thread. Lifecycle faults remain observable if telemetry is saturated.

## Falsifier and acceptance checks

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

## Consequences and risks

- Accepted cost: reconnecting requires an explicit resume; unknown
  backend bounds may initially exclude devices. Both outcomes are visible.
- Safety control: one owner, generation fencing, admitted resources and an
  off-thread quiescence barrier precede replacement or destruction.
- Revisit: evidence that a backend cannot provide the required capability or
  callback-quiescence guarantee reopens its adapter design before enablement.

## Specification update and acceptance boundary

Accepted on 2026-09-11 at the user's instruction to accept the reviewed policies
and write their current specifications. The current contract is
[spec-host-io-lifecycle.md](../specs/spec-host-io-lifecycle.md), registered with prefix `IO`.
The [host profile](../specs/spec-host-profile-and-render-limits.md) incorporates
the recording units, reservation rules and lifecycle link in the same change.
Together ADR-0024 and ADR-0036 discharge Phase 9's entry-decision requirement.

ADR-0022 still gates production mapping/latency and the capture allowance;
ADR-0052 still gates real loop-boundary production. Phase 10A/10B/10D still own
canonical targets, atomic transaction machinery, durable assets and recovery.
These are first-consumer gates, not prerequisites for a stopped-only simulated
host and bounded recorder. No hardware result, runtime-loop implementation or
Phase 9 exit is claimed. Live swaps retain ADR-0009, ADR-0010 and ADR-0050's
separate obligations.

## Review

Codex (GPT model family) authored this decision and its current specification.
The fresh Claude Sonnet readers below are from a different model family and
had no authoring context; no same-family review waiver was used.

A fresh Claude Sonnet reader reviewed the coupled design and complete
uncommitted documentation on 2026-09-11, using read-only tools with MCP and
delegation disabled. It found no stopping-rule defects and verified the cited
repository premises. It noted an optional clarification of the coordinator
state list versus per-generation state, including the `Unavailable` transition;
that is carried to the concrete lifecycle implementation design. This review
is not hardware evidence and does not accept the proposal.

Stopping rule: a false conclusion-affecting premise, contradiction, unfillable
contract, safety/correctness defect, or unsupported evidence claim blocks
acceptance. Optional implementation detail does not.

### Policy-completion review

The 2026-09-11 follow-up makes the state machine per connection generation and
closes the interrupted-take reference against ADR-0024's fenced sealing policy.
A fresh Claude Sonnet reader reviewed the complete coupled revision with
read-only tools, MCP and delegation disabled, and found no lifecycle defects.
The recording-side findings and their successful focused reread are recorded
in ADR-0024's policy-completion review. This review does not accept either
proposal or qualify a hardware integration.

### Acceptance review

A fresh Claude Sonnet reader reviewed the complete uncommitted acceptance diff,
including both new current specifications, with read-only tools, MCP and
delegation disabled, on 2026-09-11. It found a missing pre-arm obligation in the
HOST-INV-020 test row and an obsolete entry-preparation sentence in NOW.md.
Both were repaired. Author audit synchronized the profile's field origins,
quantity-type index and historical counts. A focused reread confirmed those
repairs and found only inconsistent reserve terminology in the test row; author
self-audit replaced it with the invariant's term, `finalization reserve`.
Both reader commands exited successfully. The fast documentation gate and all
35 checker tests passed. No remaining stopping-rule defect is recorded.
Historical proposal reviews above retain their original scope; acceptance makes
no new implementation or measurement claim.
