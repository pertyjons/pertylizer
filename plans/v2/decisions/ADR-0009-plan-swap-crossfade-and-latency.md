# ADR-0009: Plan-swap crossfade and latency

| Field | Value |
|---|---|
| ID | ADR-0009 |
| Status | Accepted |
| Phase | 9 |
| Created | 2026-09-21 |
| Last reviewed | 2026-09-21 |
| Related | ADR-0050, ADR-0064, ADR-0072, ADR-0073, P06-R002 |
| Supersedes | ADR-0072's no-activation restriction solely for the separately owned reset consumer below; no identity migration or minter snapshot promotion |
| Superseded by | ADR-0074 for concurrent transport and capture coupling |

## Historical transport scope

[ADR-0074](ADR-0074-concurrent-live-host-and-duplex-capture.md) supersedes this record's
serialized-only transport, pending-MIDI Busy window, and exclusion of plan swaps from
note capture. Statements about those restrictions in the historical body below are
withdrawn for the concurrent consumer. Parameter Busy behavior in the serial preparation
API remains; the concurrent host acquires and installs at the same quantum boundary.
Current specifications define the resulting contract. The remaining DSP, capture and
identity rules in this record stand.

## Decision and falsifiers

The user selected reset plus short crossfade while playing. `SwappingLiveStream`
is an exclusively scheduled, non-shipping simulation over real Core V2 live voices.
Its preparation/compilation, publication calls, callbacks and collection are serialized;
compilation runs outside callbacks and advances no simulated device time. This does
not supply a concurrent compiler mailbox for a continuously running physical device.
An old identity controlling a new voice, an unbounded candidate/retirement count,
callback allocation/destruction, a stale parameter slot reaching the new plan,
partition-dependent timely fade, or a normally retired live registry blocks acceptance.
Optional detail does not.

## Two slots and a fresh epoch

The owner has one active renderer and exactly one secondary slot, which is Pending,
Fading or Retired. Preparation refuses before compilation if that slot is occupied.
It also requires every earlier accepted MIDI operation to have an outcome and no
scheduled transport end. Failure leaves the last valid plan and its voices intact.
An over-budget compiled candidate is discarded outside callbacks. There is no public
owning candidate or returned-retirement API: collection borrows then destroys the
retirement before another preparation credit exists. Peak admission covers both
compiled renderer reports, actual retained plan-table capacities, live metadata,
parameter cells, two inline owners and
one Q-frame mix scratch buffer.

While Pending, new MIDI/parameter offers return Busy for retry after installation.
This consumer has no raw note recorder. The existing note-capture host does not use
this Busy window, avoiding an implicit change to its lateness or annotation contract.
No queued onset is cancelled into a continuing stream without a pairing obligation.

At the next empty-carry quantum boundary B, install a fresh renderer epoch at the
current device clock, removing only its initial priming silence. Source serial
high-water marks, pedal and bend survive. Every old key-down tracker, including a
refused onset, becomes a non-sounding tombstone in the new owner. Releases consume
these before newer matching keys. Key-up sustained voices have no remaining physical
key-release obligation. No voice, envelope, oscillator or script state migrates.
Old pending parameter versions become Cancelled; new values start from the new plan.

## Fade, release and Stop

For exactly Q output frames, render both plans and mix old*(1-t)+new*t, with
`t=(frame+1)/Q`. The old plan accepts no new input. At B+Q its protected Stop group
ends every old note. Render one additional old quantum into discarded scratch so
both the ingress holds and renderer identity registry have actually released.
Only then is retirement available. This adds bounded transition work, not extra
output carry or latency. Zero-release voices therefore still provide a real fade.

A Stop/panic offered before installation cancels the candidate and ends the active
owner. During a split fade it targets both owners. No new command can arrive inside
a serialized callback or its discarded release quantum; that quantum already owns
the forced release. Whole-callback failure silences the entire output span and
retains both interrupted owners for recovery and off-callback collection.

Both owners stage audition and parameter outcomes until the entire outer callback
succeeds, including the discarded release quantum. Failure cancels those staged
outcomes without changing prior successful callbacks. `take_outcome` searches both
active and secondary owners during fade and retirement; collection also borrows
any remaining old outcomes before destroying the owner.

The continuing simulated device clock is stable across replacement; renderer epoch
changes are exposed in audition outcomes. This is reset/refusal of old live identity,
not compatible state migration under ADR-0010 and not Phase 9 production acceptance.
