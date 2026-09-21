# ADR-0074: Concurrent live host and duplex capture

| Field | Value |
|---|---|
| ID | ADR-0074 |
| Status | Accepted |
| Phase | 9 |
| Created | 2026-09-21 |
| Last reviewed | 2026-09-21 |
| Related | ADR-0009, ADR-0063, ADR-0073 |
| Supersedes | ADR-0009's serialized-only transport boundary and ADR-0073's deferred capture coupling and concurrent PCM handoff |
| Superseded by | — |

## Durable boundary

The next consumer moves plan preparation and PCM acquisition onto threads concurrent
with audio rendering. Candidate destruction, note-result reconciliation, and PCM
ownership therefore need a concrete real-time boundary before connecting the host.
The shipping application, project persistence and physical MIDI timing are unchanged.

## Decision

The experimental Linux capture host uses a latest-wins triple buffer of optional
prepared live owners. The control thread alone compiles and replaces publisher-owned
values. Audio reads the latest publication at an empty carry/quantum boundary and
installs it immediately. A publication racing after acquisition is a later revision.
Future unstaged input entries move by cell index into the fresh owner's empty cells;
they have not acquired an ingress identity. Staged/completed outcomes remain in the
old owner. Source serial high-water marks, pedal, bend and key-down tombstones copy;
DSP/voice state resets. The original Q-frame fade and rendered old-note release stand.

Acquisition checks candidate freshness, exact profile, ordered source generations and
entry capacities. Retirement clears the private freshness mark, so a returned owner
cannot be re-offered as a prepared candidate.
Entry and held-tracker capacities derive from the same quota. Invalid or over-budget
candidates return to control without changing sounding playback. An ended, closed or
faulted owner never installs another candidate. While audio service continues, later
publications return cancelled. After a host fault bypasses audition service, unread
publications remain in the mailbox until both endpoints are destroyed after join;
no cancellation receipt or counter increment is claimed for that destruction.
The concrete adapter processes an applied Stop/panic before attempting acquisition.
Superseded unread publications have latest-wins semantics, not individual command receipts.
Active plan/installation time and rejection/cancellation counters are published together
through a separate Copy snapshot after a successful whole audio callback.

The host services the mailbox between bounded render segments. It commits audition
outcomes only after every segment of the outer callback succeeds. There is no arbitrary
host callback inside the core render loop. Failure silences the whole outer span.

A one-slot SPSC returns retired owners. An old owner's entry cells must all be empty:
audio first reconciles the raw annotation, pushes its Copy result, and consumes the
entry. The bounded reconciliation scan searches both owners. A stalled result collector
can exhaust the existing 128 observation credits and trigger ADR-0073's explicit terminal
policy; a stalled plan collector only delays subsequent installations. The latter never
blocks rendering or protected Stop/panic.

The concrete transport admits eight fixed payload positions at 8 MB each, plus 64 KiB
transport overhead: active, secondary, three mailbox cells, retirement ring, compiler
local and one failure-retention slot. Mutable publisher ownership allows only one
compilation at a time. Audio also waits for vacancy in the retirement ring before
acquisition. The active/secondary renderer has a 16.1 MB aggregate ceiling. This ceiling
covers the fixture's 8,192-frame maximum callback; its compiled DSP allocation alone
exceeds 6 MB. Temporary compiler allocations are off-thread, outside prepared-payload
admission. These numbers are experimental ceilings, not production resource qualification.
Both transport endpoints and callback pools remain owned until backend join. Remaining
candidates and owners are destroyed off-thread even when no final callback arrives.

## Concurrent PCM and lifecycle

Independent input and output callbacks and an off-thread worker own separate endpoints.
Input copies original finite mono/stereo frames into a recording SPSC and a separate
monitor SPSC. Recording admits an entire input block only when its ring has room;
overflow freezes the exact accepted prefix and first missing input frame. Worker capacity
exhaustion likewise retains the prefix, without waiting for another callback. Input stop
and loss provide an explicit report; worker finalization happens after input join.

The monitor producer drops a full new block with a counter. The consumer may shed old
backlog before passing contiguous source frames to the bounded nominal-clock resampler.
Source-frame discontinuities reset interpolation. Monitor loss and resampling never
change recorded original samples. The existing synthetic monitor's internal test recorder
is not the physical take; physical results retain CPAL input timestamps separately.

The ALSA `duplex` harness requests a 256-frame buffer, validates the negotiated buffer
against its 8,192-frame allocation, and supports F32/I32/I16 mono/stereo input and output
with equal channel layouts. It reports requested/negotiated configuration, backend error
categories, original take extent, gaps, monitor counters and backend latency estimates.
Monitoring uses gain 0.1 and an explicit 4,096-input-frame target. Recording applies no
latency compensation; cross-stream timestamps are never subtracted as though calibrated.
Input and output timestamp differences are backend estimates, not measured ADC/DAC or
round-trip latency. The runner stops, joins and finalizes before reopening at a supported
alternate rate. Earlier takes remain owned through a later preparation or runtime failure.
This is an isolated physical candidate under ADR-0022, not a production timing consumer.

## Evidence and falsifiers

The `v2_cpal_output` example tests fail on callback allocation/destruction, wrong-plan
admission silencing a held voice, future input starving a swap, stale candidates reviving
Stop, lost raw repeated-key annotations, changed original PCM, an unbounded monitor
backlog, or lost takes after worker starvation/reconfiguration/no-final-callback loss.
The example includes real control/audio compiler concurrency and three-thread PCM tests.
The core tests additionally exercise one million real identity generations and a reduced
finite-generation exhaustion control. Neither that count nor backend estimates select a
production producer partition or retire P03-R004/ADR-0022's qualification obligations.

## Alternatives and consequences

Keeping the serial simulators would not exercise ownership races. Moving DSP state would
require ADR-0010 and is independent of the reset/crossfade stage selected here. A mutex
around the serial owners would cross the real-time boundary. Fixed ownership slots cost
reserved capacity and may delay structural edits while collection stalls; this is accepted
in exchange for continued playback and bounded custody. Production adaptation, full mixed
producer timing, physical round-trip calibration and resampling quality remain later gates.

## Specification update

Update the host lifecycle, recording, and render specifications with this concrete
experimental transport while retaining their physical timing and production gates.
