# ADR-0079: Raw capture interruption in a mixed host

| Field | Value |
|---|---|
| ID | ADR-0079 |
| Status | Accepted |
| Phase | 9 |
| Created | 2026-10-14 |
| Last reviewed | 2026-10-17 |
| Related | ADR-0073, ADR-0075, ADR-0078, EVD-0024 |
| Amends | — |
| Supersedes | — |
| Superseded by | — |

## Decision boundary

A source ring feeds two independent consumers in the experimental combined
host: raw capture and the mixed sound bridge. ADR-0073 makes credit and tracker
exhaustion terminal in its exclusive capture host and keeps raw capture
independent of an ordinary ingress-onset refusal. It does not decide what a
mixed host does when its recorder refuses a late raw packet. Coupling that
refusal to sound would let a silent or delayed source end otherwise valid sound
from the other source. This is an ownership and fault-scope choice that later
host consumers must share.

The maintainer selected raw-only interruption on 2026-10-14. The change is
confined to the experimental mixed host rehearsal and its repository-local
consumers. The maintainer requested the common software boundary on 2026-10-17.
`host::mixed_input` now supplies the library raw-first route and monotone
classification; `mixed_source_bridge` uses them for the concrete two-source
host. [ADR-0081](ADR-0081-combined-host-software-boundary.md) owns that boundary.
No persisted format, wire contract, shipping route or exclusive ADR-0073 host
changes.

## Decision

1. A classified late raw outcome interrupts **raw capture for both sources**.
   The rehearsal recognizes the raw merger's `SessionError::SourceOrder` after
   the source rings have delivered their ordered prefixes, the recorder's
   `NoteCaptureError::PastBoundary`, and `CaptureDisposition::Late`. It retains
   the exact raw event ID, observation and outcome; it does not reduce the
   refusal reason to a boolean. Raw control stops offering either source to
   that capture session.
   Both source producers and mixed sound continue. The host retains every
   source-ring packet's identity whether or not raw capture accepts it.
2. Sound pairing uses the source ring's per-connection, gap-free packet ID and
   its own per-source/channel/key FIFO. It never depends on a raw capture ID or
   the recorder's `matched_onset`. A raw-admitted key-up and the corresponding
   sound key-up agree while both consumers receive the same source prefix;
   after raw interruption the sound FIFO continues alone.
3. A fault in raw **admission while capture is still running** is a terminal
   host fault: the packet cannot be handed onward under the running host's
   raw-first contract. A raw `State` refusal racing with a **classified late**
   interruption is part of that interruption. An unclassified raw self-halt,
   including tracker exhaustion, is terminal rather than raw-only. The bridge's
   terminal causes are `ForeignSource`, `SourceSerial`, `SourceOrder`,
   `SourceHolds`, `ClockExhausted`, `Ledger`, `Source`, `Transfer` and
   `RawSelfHalt`; source
   retry expiry, renderer failure and overrun also stop the complete host.
   A terminal fault silences its entire observing callback and later callbacks;
   joined teardown classifies every source, raw and sound obligation without
   another callback.
   A raw render error other than the expected post-interruption `Closed`, or a
   raw packet-preparation error, is terminal even if a classified late receipt
   was collected earlier. The callback retains the exact render error for
   joined teardown, publishes the host halt and silences its whole output.
   Control retains the exact preparation error and publishes the same
   source-neutral halt before another callback.
   A terminal raw outcome dominates a classified late outcome in either
   completion order. Once a terminal outcome is collected, the host halts and
   cannot be revived by a later late receipt. If late is collected first, raw
   capture alone stops until a terminal outcome appears; that terminal outcome
   then halts the host without rolling back sound already delivered. A raw
   halt with no collected decisive outcome fails closed as `RawSelfHalt`.
4. Exhaustion of ordinary sound-stage cells, command places or ingress credit
   is a named refusal of the affected occurrence and cannot stop raw capture or
   the complete host. Exceeding the source's **declared held-onset bound** is
   `SourceHolds`, a terminal source-contract violation under clause 3. Neither
   a raw-only interruption nor a sound refusal silently changes the other's
   recorded outcome. The raw owner must reserve at least the sum of the two
   sources' declared simultaneous note holds; the current rehearsal declares
   64 per source and prepares 128 raw held and tracked note places, 136 input
   cells per source, and a two-megabyte capture budget. The test fills all 128
   holds concurrently. Otherwise legal simultaneous source holds could cause
   a terminal raw capacity stop.

## Falsifier and evidence

This decision is false if a first decisive late raw outcome silences sound, halts either
source producer, changes a same-key sound pairing, hides the refused raw
packet, or leaves any previously admitted raw packet without exactly one
receipt. It is also false if a true terminal fault resumes output or loses an
outstanding source or command ID.

The concrete two-source rehearsal's
`a_late_raw_refusal_stops_capture_for_both_sources_while_sound_matches_ample_raw`
compares sound samples and outcomes bit-for-bit with an ample-raw control while
the raw merger reports `SessionError::SourceOrder` for the delayed raw packet.
The test retains and checks that exact reason, raw event ID and original
observation. Its joined teardown checks each source ID and raw receipt.
`a_raw_self_halt_without_a_collected_late_reason_stops_the_whole_host` checks
the fail-closed default, and
`terminal_raw_outcome_dominates_late_in_either_completion_order` checks
the pure terminal-priority rule and selected outcome classes; it does not inject
reversed real completions. `a_raw_self_halt_with_a_waiting_peer_packet_keeps_source_custody`
checks that an unclassified raw halt cannot blame or consume the peer's packet.
`a_late_classification_cannot_hide_direct_raw_errors` injects exact render and
preparation errors into the host's common terminal latch after a late decision;
it does not force the recorder to produce a real render fault.
The terminal-cause matrix covers `SourceOrder`,
`Source`, `SourceHolds`, `ForeignSource` and `Transfer` over four callback
partitions. `SourceSerial`, `ClockExhausted`, `Ledger`, `RawSelfHalt` and
renderer-fault injection and reversed real raw completions remain outside that
matrix; the raw self-halt has its separate test. These are
experimental-host proofs, not production or physical-input qualification.

## Consequences and limits

The mixed host needs two halt signals: one for the sources and whole host, and
one for raw capture. Only a self-interruption backed by a classified late
outcome requests the latter alone; an unknown or other cause requests both.
The control loop must continue draining source rings and staging sound after
raw capture stops. The raw recorder may retain a shorter take than sound's
performance; its receipt reports that fact explicitly.

The default-off mixed rehearsal remains outside shipping dependency edges.
ADR-0081 accepts the bounded common software route and stopped-owner reset.
ADR-0075's full combined-host qualification, capacity reselection and physical
timing remain open. No production mixed refusal is lifted here.
