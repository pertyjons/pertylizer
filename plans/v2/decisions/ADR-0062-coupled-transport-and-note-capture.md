# ADR-0062: Coupled transport and note capture

| Field | Value |
|---|---|
| ID | ADR-0062 |
| Status | Accepted |
| Phase | 9 |
| Created | 2026-09-12 |
| Last reviewed | 2026-09-12 |
| Related | ADR-0061, ADR-0024, ADR-0036, TAKE-INV-001, IO-INV-002 |
| Supersedes | — |
| Superseded by | — |

## Boundary and decision

This extends ADR-0061's serial owner to capture admission, source dispatch and
retained command outcomes. It binds the host and recorder's real-time custody.
The current contract is [ordered note capture](../specs/spec-host-io-lifecycle.md#ordered-note-capture).
No persisted, wire, physical timestamp or concurrent backend contract changes.

The session may attach one prepared recorder and one bounded source-action queue
before device start. Direct capture control is unavailable in this mode. Arming
fixes a stopped-position anchor, future Play time, rate and epoch. A recording Play
carries its reserved take; every Stop snapshots the recorder's currently active
ticket at offer. A sealed/discarded take is no longer associated with new commands;
an already queued Stop retains its original ticket.
Rearming waits for all command and source outcomes to be collected, so a new take
cannot redirect an accepted command.

Source actions are explicitly ordered by publication/frontier time. At equal times,
all fences precede all publications; an offer violating that order refuses before
consuming an identity or slot. The serial owner supplies each fence's actually
consumed publication sequence. This certifies only this exclusive source queue's
prefix; it is not an inferred acknowledgement from a concurrent physical backend.

Commands remain FIFO at equal times. A Play followed by a same-time Stop cancels
before capture start or fence dispatch. Stop applies before equal-time fences. A
surviving recording Play has no later same-time Stop: it drains the complete
same-time fence prefix, checks scheduler offer admission, starts capture, and only
then offers the activation immediately before rendering. Both checks use the same
exclusive owner; capture start cannot mutate scheduler admission. A missing start
fence refuses the coupled Play at its original time, leaves the activation pre-offer
and retains the armed take. It never retries at a later boundary.

Capture outcomes are separate retained values. Routine start/stop refusals never
enter the renderer-fault channel. An audible Stop applies even if its capture end
refuses or sealing waits for another source. An unexpected activation refusal after
successful preflight and capture start is an explicit terminal invariant fault,
retained and diagnosed; it does not silently continue partial playback.

Each source action reserves its own outcome slot. Source admission and result
storage have explicit fixed capacities and byte budgets, separate from recording
storage. Loss cancels unconsumed source actions and commands and keeps their
receipts. Backend retirement waits for these receipts and existing capture/source
fences. Result disposal cannot bypass still-held session outcomes.

## Falsifier and review boundary

The same ordered input trace, commands and fences must produce the same samples,
selected raw records and outcomes across whole and split callbacks. A lost accepted
outcome, callback allocation/deallocation, late retry of a refused coupled start,
audible Stop delayed by a source, or ordinary capture refusal silencing other audio
falsifies this design. The allocation-guarded and boundary tests in
`src/host/session/capture/tests.rs` exercise those cases, including equal-time
cancellation and loss without a final callback.

The design consultation rejected independent fence-order narratives for Start and
Stop. The total source order and cancellation-before-fences rule above resolve the
collision. It also required explicit capture-result isolation; a separate retained
field and matched errors provide it. Final code review covers these repairs.

This first coupling supports compiled Play and Stop with exact synthetic note
sources and zero lateness. Standalone punch-in/disarm, live audition, loop passes,
physical clocks, audio input and concurrent publication remain subsequent consumers;
none is established by these tests.
