# ADR-0089: Threaded application observers and backend retirement

| Field | Value |
|---|---|
| ID | ADR-0089 |
| Status | Accepted |
| Phase | 9 |
| Created | 2026-10-18 |
| Last reviewed | 2026-10-18 |
| Related | ADR-0022, ADR-0056, ADR-0076, ADR-0084, ADR-0086, ADR-0088 |
| Supersedes | — |
| Superseded by | — |

## Decision

The application owns a bounded SPSC observation lane beside its session lane.
Only control constructs subscriptions, read buffers and handoff mirrors. Audio
owns the actual store. Install, remove and read commands carry their complete
owners and checked per-lane request IDs; refusal retains ownership and does not
advance the issuer. IDs never wrap. Install requires a pristine store for the
currently installed plan and the exact outgoing store identity. Reusing ended
handles is refused. Read requires the issuing store and matching layout.

The lane admits two outstanding commands including completed returns. Application
control retains at most two additional collected replies. Each transported store
or buffer has an actual-capacity allowance of 4 MiB; an active store and prepared
handoff mirrors are additional separately owned resources. This is not a total
application memory claim or a production-calibrated cost. Audio services at most
one command at outer callback entry and copies no more frames than the profile's
maximum callback size. Read results label plan, handoff generation, dropped
frames, lag and ending; they do not invent an absolute hardware timestamp.

Acknowledged store configuration is mirrored cold. Sound edits prepare handoff
against that mirror and attach it to their owning replacement packet. Install
and remove cannot race a pending sound replacement, and sound edits cannot race
an unacknowledged subscription change. A read may occur during a transition.
The actual store is supplied to every observed render and replacement entry.
The owning edit receipt promotes the mirror only after successful handoff;
ended, detached and interrupted outcomes clear it. A render fault ends the store.
Fresh session replacement, explicit Stop and leaving V2 mode end that session's
subscription lineage rather than implicitly extending handles to a fresh owner.

After callback quiescence, joining matching lane halves returns the active ended
store, completed replies and cancelled commands with their original identities.
A foreign join returns both complete halves. Ordinary rendering never destroys
these objects. The backend custody rule below also covers the no-next-callback
case, in which no render-loop return credit can make progress.

## Backend custody

With `v2-lowering` enabled, the CPAL output closure holds a unique processor
guard and a parent lease declared after the native stream field. Default builds
retain their existing processor route. One process-global cold worker admits at
most eight registrations, counting queued registrations and retained owners.
A stalled destructor or delayed native closure consumes its existing reservation;
new registration refuses at the bound instead of starting more workers. Worker
spawn and registration failure surface as stream-creation errors. Spawn failure
is cached for the process lifetime and requires a process restart before retry;
this prevents concurrent attempts from creating multiple retirement workers.

An outer box is allocated cold. Guard destruction exchanges its unique optional
owner into `atomicbox` 0.4.0's `AtomicOptionBox` with constant valid `AcqRel`
ordering. The audited crate has no runtime dependencies and is MIT/Apache-2.0
licensed; its `swap_mut` and `take` exchange owned pointers without allocation,
deallocation or value destruction. No new unsafe implementation is authored here.
The worker must obtain sole mailbox ownership through `Arc::try_unwrap` before
extracting the payload; a reference-count snapshot is insufficient. A diagnostic
fence has separate ownership and cannot keep the mailbox alive.

The cold worker publishes Reclaiming, destroys both box layers, then publishes
Complete. Under unwinding builds, destructor panic publishes Failed and permanently
consumes its reservation; a possibly panicking panic payload is retained under
that same eight-reservation bound. Release uses panic abort and makes no promise
to recover from destructor panic. Unexpected worker unwind closes admission and
retains the bounded outstanding custody rather than letting a delayed callback
become its final owner. The worker adds no `on_stream_stop` calls.

The application control half separately observes destruction of the switch's
fields through a final drop marker. Its `ProcessorRetirement::Failed` means
only that field destruction occurred during an unwind, including an unrelated
unwind; it attributes no cause. This differs from the backend fence's Failed
outcome for a caught payload destructor panic. This marker proves neither enclosing-box
release nor native-device quiescence nor destructor thread identity. CPAL's
CoreAudio path inherits native synchronous stop/uninitialize assumptions; delayed
closure ownership stays retained. Physical platform qualification remains open.
Standalone examples retain their separate owner guards and are outside this
backend implementation.

## Falsifiers and stopping rule

Acceptance fails on callback allocation, deallocation, blocking or I/O in the new
lane/guard; a stale handle rebound to a fresh store; silent lost coverage; a false
completion marker; a lost owner on refusal, fault, disconnect or close; unbounded
read/registration work; or changed rendered samples in the passive observer
controls. Actual second-thread tests cover stalled subscribers, ring overflow,
backpressure, removal/recreation and running plan handoff at the software boundary.
Backend worker controls cover both parent/closure destruction orders, fixed
admission under delayed callbacks and stalled destruction, and an unwinding
destructor. These controls cannot qualify native quiescence or physical timing.
A contract hole blocks acceptance; optional implementation detail does not.
