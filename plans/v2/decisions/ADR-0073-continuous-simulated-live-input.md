# ADR-0073: Continuous simulated live input and audio capture

| Field | Value |
|---|---|
| ID | ADR-0073 |
| Status | Accepted |
| Phase | 9 |
| Created | 2026-09-21 |
| Last reviewed | 2026-09-21 |
| Related | ADR-0072, ADR-0009, ADR-0022, ADR-0054, IO-INV-005, TAKE-INV-002 |
| Supersedes | ADR-0072's whole-attempt source quota, non-reusable audition cells and unsupported bend; its capture/count-in contracts otherwise stand |
| Superseded by | ADR-0074 for concurrent transport, capture coupling and concurrent PCM handoff |

## Historical transport scope

[ADR-0074](ADR-0074-concurrent-live-host-and-duplex-capture.md) supersedes this record's
serialized-only transport, pending-MIDI Busy window, exclusion of plan swaps from
note capture, and the deferral of concurrent audio-input handoff. Statements about those restrictions in the historical body below are
withdrawn for the concurrent consumer. Parameter Busy behavior in the serial preparation
API remains; the concurrent host acquires and installs at the same quantum boundary.
Current specifications define the resulting contract. The remaining DSP, capture and
identity rules in this record stand.

## Boundary and falsifiers

The user selected continuing live input, channel bend, independent parameter updates,
running plan reset, simulated audio recording and independently clocked monitoring.
These are experimental simulated consumers. Physical input, platform timing, production
producer budgets, concurrent audio-input handoff and persisted audio assets remain gated.
A reused audition identity, wrong repeated-key release, unresolved sealed annotation, callback
allocation or destruction, lost accepted outcome, parameter flood displacing Stop,
unmarked recorded gap or monitoring loss changing recorded PCM blocks acceptance.
Optional implementation detail does not.

## Reusable live custody

The concrete source producer has a bounded ring, without a total observation quota.
Full rings return the original observation for ordered retry. Terminal halt refuses
new observations. The live renderer reuses a completed observation cell only when
its consumer takes that outcome. Per-source serial high-water marks survive reuse.
Key trackers are independently reusable after key-up and voice release. Refused
onsets retain tombstones; matching release selects the oldest source/channel/key
serial, never the lowest reused array index. Unmatched key-down obligations remain
bounded, even when their outcomes have already been collected.

The concrete audition bridge admits 128 outstanding operations across queued input,
renderer cells and its result ring. Collection returns that credit. Input receipts
carry the original audition trace. Control publishes a settled ID after the final
input receipt, or after capture admission refuses that observation. Only then can
no future raw record introduce the same pending ID. Audio reconciles at most eight
settled, completed outcomes per callback before moving them into the result ring.
The raw scan visits only Ordinary cells: the concrete 64-record preparation bounds
this at 512 cell inspections per callback. This is a work bound, not a measured
platform deadline. A full result ring retains its owning cells.

A collector may stall within these capacities; it cannot stall indefinitely while
admission continues. Credit or tracker exhaustion terminates the host, silences
output and retains joined recovery, including an unaccepted original observation.
Stop/panic needs no observation credit. This terminal overload policy also applies
when publication density exceeds the provisional profile. It does not promise equal
capture survival with audition enabled under arbitrary result-collector starvation.
Normal ingress onset refusal still leaves raw capture independent.

Loop Stop only closes the recording boundary. Sealing is explicit worker finalization,
after callback join, remaining audition cancellation/reconciliation and source fences.
It does not run in the Stop callback. Joined reconciliation is not limited to eight
outcomes and needs no final callback. Pending annotations still refuse sealing.

## Channel bend and parameters

Bend is plus/minus 200 cents per source and MIDI channel. It follows every accepted
identity on that channel, including sustained key-up notes; new notes inherit the
current value. Raw MIDI remains unchanged. Late execution records the actual first
unrendered time. One bend expands to at most eight ordinary ingress writes. Excess
density, or an inherited bend that fails after onset admission, is a terminal whole
callback failure, never a partially successful execution outcome. Bend coalescing
and production wheel-density qualification are not claimed.

The parameter lane has prepared plan-specific slots and strictly increasing versions.
A newer pending write returns the superseded version synchronously. Each cell exposes
the latest version as Pending, Applied or Cancelled; the last applied version remains
separately readable. Stop and retirement cancel pending versions; terminal faults cancel pending and
staged versions. Executions from earlier quanta of a successful callback still apply.
External values obey the declared unit domain; quality-factor writes additionally
require positive Resonance values, without changing the core modulation law. Note-owned magnitude/gate
rows and sample-positioned controls are refused at preparation.

Each prepared cell can publish once per quantum through Session, ahead of Live.
There are at most Session-share-minus-one cells. This owner has no other Session
producer except its one Stop group, so parameter traffic cannot consume release
custody or Stop's slot. The immutable live owner now publishes its empty-compiled
plan directly through its single arbiter; it no longer needs an empty scheduler.
The same source scan covers its publication and parameter hot paths.

## Original PCM and monitoring

`SimulatedAudioInput` serializes input, output and worker calls under exclusive
ownership. It models distinct clocks without claiming a concurrent device adapter.
Preparation validates formats, sizes, a configured synthetic clock and a source/output
rate ratio in [1/8, 8]. Fixed chunk cells own copied interleaved original PCM plus
source-frame origin, nominal engine time and length. Input rejects ragged, oversized,
empty or non-finite blocks. A worker copies accepted chunks into separately admitted,
bounded take storage. The immutable in-memory result retains original format, clock
provenance, chunk stamps and the first gap. There is no asset file or project commit.

Pool exhaustion, source discontinuity, invalid input, clock overflow, worker storage
exhaustion or loss freezes the valid prefix and identifies its first missing source
frame and reason. Worker exhaustion refuses the entire next chunk, without inventing
its suffix. Queued earlier chunks remain drainable. Joined finish needs no further
callback. A later-discovered earlier gap replaces a later gap so the retained result
always describes its actual prefix.

Recording receives source samples before monitoring. Monitoring has separate storage
and never supplies samples to recording. Its exclusive owner may discard oldest
frames on overflow, reports the count and resets fractional interpolation. Source
gaps also reset the monitor. Underflow/initial fill emits counted silence. After
initial target fill, linear interpolation uses the configured rate ratio; every 64
output frames a bounded occupancy estimator adjusts that ratio by up to 1000 ppm.
This is synthetic buffer feedback, not hardware timestamp calibration. Occupancy is
bounded by the prepared buffer; drift outside correction authority produces explicit
drops or silence. The maximum buffered latency is that capacity divided by source
rate; no physical end-to-end latency or antialiasing quality is qualified here.

## Verification and remaining boundary

The continuous live tests exercise 10,000 note cycles, uncollected outcomes with
protected Stop, inherited/channel/sustained bend and 10,000 coalesced parameter writes.
Audio-input tests compare original PCM through monitor overflow, exact gap prefixes,
worker stalls, loss without callbacks, rate conversion and a sustained 100 ppm
synthetic drift. The concrete host tests retain raw annotations across source-cell
reuse and joined recovery. `v2_live_session` exercises MIDI, parameter updates,
running plan reset, PCM recording and monitoring together without an audio device.
Its allocation guard includes the fade and discarded release quantum. The complete
repository gate, both concrete example test suites and independent uncommitted
review with a focused repair reread passed. The running-swap failure regression
checks whole-callback cancellation and separately preserves execution before a
normal Stop. The concrete credit-exhaustion test recovers all 128 accepted results
and the original refused release without another callback.

[ADR-0009](ADR-0009-plan-swap-crossfade-and-latency.md) owns the separate simulated
swapping consumer. It does not install plans inside the note-capture host; coupling
its Busy/retry interval to raw capture needs its own admission/late-input policy.
Physical timing, complete producer calibration, state migration and durable asset
storage remain with their existing owners. This change does not exit Phase 9.
