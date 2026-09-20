# ADR-0067: Ordered serial loop recording

| Field | Value |
|---|---|
| ID | ADR-0067 |
| Status | Accepted |
| Phase | 9 |
| Created | 2026-09-20 |
| Last reviewed | 2026-09-20 |
| Related | ADR-0062, ADR-0065, ADR-0066, TAKE-INV-003, IO-INV-003 |
| Supersedes | — |
| Superseded by | — |

## Durable boundary

The first ordered loop-recording consumer must reconcile session commands, actual
render observations and source fences without creating two mutable transport owners.
This binds the callback ownership and retained-result boundary of subsequent hosts.
The implementation is a serial simulation, not a concurrent transfer proof.

## Decision

One finite `LoopRecordingSession` consumes an armed, untouched `LoopCaptureSession`
at engine clock zero with its complete priming carry.
It owns the compiled loop, journal, recorder, source queue and command receipts.
Its connection generation comes from the simulator's process-wide checked generation
issuer; it cannot alias another output or input fixture's receipts.
No mutable inner owner escapes. It admits one recording Play at the prepared initial
engine clock and ordered quantum-aligned Stops. Restart, rearm and plan replacement
require a later consumer; they cannot reuse this finite take's mapping.
Every failed conversion returns the original capture owner; an invalid conversion of
an active take cannot dispose of it as a side effect of reporting an error.

The existing session command and source-action vocabulary applies. Equal-time Play
followed by Stop cancels Play. A surviving Play consumes its source-fence prefix
before capture starts. Missing start fences refuse Play and capture, with no delayed
start. Stop precedes a pending loop wrap, freezes musical position and selects silent
future quanta; already rendered output carry keeps its original delivery order.
The applied position is the loop source's actual position, including a pending
exclusive-end wrap, never an extrapolation of a linear session anchor.
Stopped loop identities and DSP state remain exclusively owned until off-thread
destruction. Nothing resumes those dormant identities.

Only successful whole callbacks authorize journal boundaries or a Stop endpoint.
The Stop endpoint uses `Finished`; an earlier journal terminal remains immutable.
A terminal failure silences the whole callback and closes observation at the prior
acknowledged frontier. Applied command receipts describe control execution, never
hardware delivery. Recording retains its minimum-endpoint and strongest-outcome laws.

Source progress does not delay audible Stop. Finalization waits for all participating
fences, and interrupted capture additionally waits for source quiescence. A stopped
source drain can finish explicit source actions, even beyond the last audio clock,
without advancing the selected audio endpoint or requiring another callback. Device loss
closes admission, cancels pending actions, freezes acknowledged audio/source selection
and retains commands, raw input and results through source shutdown.

## Alternatives and costs

Ordinary activation-based sessions continue refusing loops: their linear anchor and
split minter ownership cannot silently acquire the exclusive loop's timeline law.
The finite serial owner reuses the existing recorder and journal instead. It requires
separate preparation for another take and does not supply pause/resume, live audition,
pass-aware projection, audio capture, physical clocks or concurrent source transfer.

Command storage, source storage and wrapper inline storage have explicit byte ceilings.
The existing recorder, journal and loop-renderer budgets remain separately charged.
No hot operation allocates, frees resources, blocks or performs I/O.

## Falsifier and review

Partition-dependent successful audio/raw/pass/carry results, an extra pass at an exact
Stop boundary, lost receipts, source progress delaying Stop, a failed callback authorizing
new boundaries, sealing before required quiescence, or hot allocation falsifies this
consumer. Tests must cover whole, 64-frame, 256-frame and irregular callback partitions,
source stalls, same-time cancellation, saturation and loss without another callback.

The current contracts are the host I/O lifecycle's ordered loop recording section,
SOUND-INV-018's exclusive-loop consumer and TAKE-INV-003. SOUND-INV-024's restore and
smoothing rules do not change: stopped quanta advance no DSP state, and this finite
owner cannot resume it. ADR-0065/0066's standalone constructors retain their contracts;
this decision resolves their deferred finite serial control consumer.

A fresh Claude design consultation found the missing decision/specification amendment
to be blocking; this record and the three current specification updates supply it.
It found no blocking defect in the proposed audio/source authority or retention law.
The independent uncommitted Claude review found that refused Play could produce a
pass outside its empty take window. The repair derives the pass origin from the
retained raw window and tests both endpoints. Focused independent Claude rereads
accepted that repair, returned ownership on preparation failure, stopped source
progress and the related lifecycle/admission repairs with no remaining blocking
defects. The complete repository gate passed without warnings or errors.
False claims, contradictions, unfillable contracts and safety/correctness defects
block acceptance; optional implementation detail does not.

## Subsequent finite transfer consumer

[ADR-0068](ADR-0068-finite-loop-recording-transfer.md) adds bounded owning packet
transfer around this intact serial owner. It preserves the finite transport,
whole-callback authority and source-retirement rules while adding a control credit
ledger and joined reunion barrier before worker finalization. This record's serial
constructor and no-mutable-inner-owner rule remain in force.
