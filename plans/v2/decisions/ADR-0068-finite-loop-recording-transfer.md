# ADR-0068: Finite loop recording transfer

| Field | Value |
|---|---|
| ID | ADR-0068 |
| Status | Accepted |
| Phase | 9 |
| Created | 2026-09-20 |
| Last reviewed | 2026-09-20 |
| Related | ADR-0064, ADR-0067, TAKE-INV-003, IO-INV-003 |
| Supersedes | — |
| Superseded by | — |

## Boundary and decision

A fresh `LoopRecordingSession` may split into a control owner and an audio owner.
The audio owner retains the complete existing renderer, journal, raw recorder and
serial command/source lanes. No mutable inner renderer or recorder escapes. The
control owns only the fixed transfer-credit ledger and acknowledged snapshot.
After callback access joins, a worker may reunite both owners, drain delayed sources
and finalize the retained take. There is no simultaneous raw reader or worker-side
recorder mutating the audio owner's take.

This is ADR-0067's separately gated concurrent-transfer consumer. It preserves that
consumer's finite initial Play, ordered Stop, actual loop position, minimum capture
endpoint, whole-callback observation authority and independent source quiescence.
It does not replace the standalone serial constructor or enable restart, rearm,
plan swaps, physical source clocks, audio capture or live audition. ADR-0064's
ordinary compiled-session split retains its separate contract.

The core supplies opaque, non-cloneable owning packets and completions. A checked
`LoopTransferId` names each occurrence with its split's generation and a per-occurrence serial.
Every split obtains a fresh generation from the checked common issuer, so splitting a
reunited untouched session cannot reuse historical transfer IDs.
It is a distinct type from the serial lane's `SessionCommandId` and `SessionSourceId`;
numeric serial equality across these three namespaces is never identity equality.
A successful delivery records the mapping to the lane's eventual receipt. Late or
otherwise invalid semantic delivery yields a retained refusal, never a moved boundary.
A wrong origin, non-increasing transfer serial or closed runtime returns the packet.
Unpublished cancellation returns its exact ID; a failed send must retain its packet.
Dropping an owning packet cannot renew its credit.

With C command slots and S source slots, at most C command and S source packets
exist unresolved across control-held packets, ingress queues, audio cells, return
queues and reader-held completions. Play reserves one of C for Stop. Source pressure
cannot spend command credits. Only explicit control collection or cancellation
returns a credit. Completed receipts may be collected out of order; the acknowledged
snapshot never moves to an earlier audio clock. A stalled completion reader cannot
prevent an already admitted Stop from executing.

## Ordered delivery and late sources

One off-thread merger publishes the source FIFO in nondecreasing publication time,
with fences preceding publications at equal times. Independent source producers
must establish their common order before that FIFO; queue observation alone is not
a source fence. This consumer uses exact simulated stamps and explicit source
frontiers. It provides no independent-clock merge or hardware calibration.

The concrete host captures the ingress queue's occupied prefix once before each
callback and admits exactly that prefix before rendering the whole callback. Later
publication waits for the next cut. Audio dispatch remains ADR-0067's serial
algorithm. Identical on-time delivery and callback partitions must produce the
same audio, raw records, pass/carry metadata and normalized outcomes.

Source timing refusals remain explicit. Before Stop executes, a source action
behind acknowledged audio is refused. After Stop executes, a delayed action may
be admitted under the serial recorder's source-order and endpoint rules. Therefore
a fence racing Stop can be refused before Stop and accepted afterward. The caller
must examine the refusal and explicitly re-offer the required frontier after Stop,
or interrupt the take if the source cannot supply it. A new offer has a new transfer
ID. The refused ID retains its refusal; no inferred fence or automatic replay is
allowed. An unfenced result remains unavailable, not silently complete. The tests
must cover this race and retry as well as ordinary on-time equivalence.

## Joined recovery and result custody

Normal Stop and loss have different closure paths. After normal Stop, the controller
may close transfer publication while the joined audio owner still has open serial
source admission. Already queued source packets may be delivered and drained under
exclusive ownership, with each resulting completion collected. The worker can then
reunite the owners and supply delayed explicit source fences through the serial API.
Normal finalization waits for those fences. Only after sealing does `close_completed`
close serial source admission and permit source-quality retirement and discard.
Closing transfer publication never fabricates serial source closure or quiescence.

On device loss, callback access first joins; `device_lost_after_callback_join` then
freezes the acknowledged capture prefix and cancels pending serial work. Queued and
unpublished packets return to control cancellation. All completed outcomes, including
failed return pushes, remain collectible. Independent source producers must actually
quiesce before their acknowledgements are supplied. Interrupted finalization requires
those acknowledgements and does not require another callback. Loss never replays
pending source fences as evidence of consumed input.

Reunion checks matching origins, an empty control ledger and empty audio/serial
receipt cells, returning both intact owners on refusal. This is the custody barrier
before worker finalization or discard: merely draining receipts into a return queue
does not satisfy it. During split ownership there is no public result/discard API.
The host retains both owners and concrete queue backing until callback access joins;
a loose pair of handles alone is not a physical-backend lifetime guarantee.

## Storage and executable checks

An explicit transfer byte ceiling charges both additional inline layouts and fixed
credit/mapping cells. Existing session, recorder, journal and renderer budgets remain
separate. Preparation failure returns the original session. The development harness
separately bounds its concrete queue cells, ring layouts, padded Arc headers and failed
transfer slots. The existing workspace `ringbuf` is added only as a dev-dependency;
the production dependency graph is unchanged. Test/example builds gain a direct
ringbuf edge and remain system-library-free. The normal graph already reaches
ringbuf and crossbeam-utils through synth_core; no new package is needed by this edge.

The harness uses ringbuf 0.5.1 SPSC queues on OS threads, with test rendezvous outside
guarded callback operations. Its callback prefix, enqueue, render and completion
moves are guarded from their first use for allocation and destruction. The crate's
source-level hot-region checks cover the core admission, receipt and transfer methods.
Core runtime cells preserve outcomes when the concrete completion ring is full.
These are functional concurrency checks, not measured scheduling or physical timing.

Reused transfer identity, a lost or duplicated outcome/credit, serial/threaded result
mismatch, Stop delayed by completion pressure, selection advanced by a guessed fence, successful sealing
before required quiescence, or allocation/destruction/locking/I/O on audio falsifies
this boundary. Exact-boundary Stop, stalled source and result workers, post-cut and
late delivery, explicit fence retry, foreign/reordered packets, byte-ceiling refusal,
repeated split of a reunited untouched session, and recovery without a final callback
are required checks.

## Review

The independent Claude design consultation identified missing decision/specification
amendments, unenforced discard custody, ambiguous normal/loss closure and an unstated
late-fence retry obligation. This record states those rules; the reunion gate and
executable cases enforce them. A false claim, contradiction, unfillable contract or
safety/correctness defect blocks acceptance; optional implementation detail does not.

The independent Claude uncommitted review confirmed the four design repairs. An
additional author audit found transfer-ID reuse across repeated splits; fresh
per-split generations and a regression test close it. Focused independent rereads
accepted that identity rule and the receipt-retention repair, including the injected
missing-mapping test, with no remaining blocking defects. The complete repository
gate passed without warnings or errors after the final code repairs.
