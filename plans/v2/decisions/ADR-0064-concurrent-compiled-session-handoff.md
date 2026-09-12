# ADR-0064: Concurrent compiled-session handoff

| Field | Value |
|---|---|
| ID | ADR-0064 |
| Status | Accepted |
| Phase | 9 |
| Created | 2026-09-12 |
| Last reviewed | 2026-09-12 |
| Related | ADR-0061, ADR-0063, ADR-0050, IO-INV-002 |
| Supersedes | — |
| Superseded by | — |

## Boundary and decision

An ordered compiled session may separate its mutable control and audio owners.
`SessionControl` alone owns the minter, admitted compiled list, preparation state
and command-credit ledger. `SessionAudio` alone owns the renderer, live registry,
scheduler, carry, clock and command slots. Their shared `CompiledPlan` is immutable.
An activation's mutable working copy crosses the boundary by owning transfer and
returns as the same retained value. No shared mutable minter or registry is added.

This is a new concurrency boundary. The serial `offer_play` and `collect` combine
operations that cannot run together on audio: activation preparation and retirement
collection remain off-thread, while new scanned enqueue and completion-transfer
methods move the owning command box without allocating or destroying it. The
serial host and the split audio half share the quantum-boundary/render algorithm.
The compiled list moves to the control half; it is not cloned for the split.

Every prepared command consumes one credit until off-thread collection or explicit
cancellation of its unpublished packet. With `C` credits, Play leaves one credit
for Stop, and only one Play may be outstanding. Credits cover unpublished packets,
commands in transit, runtime slots and completions in transit together. The runtime
byte charge includes its pointer slots and all `C` owning command boxes, allocated
off-thread before submission. The controller's fixed metadata ledger has a separate
byte budget. A concrete host additionally charges its queue and retained-failure
storage. A slow completion reader cannot stop effects already admitted to slots.

## Delivery and collection

The [split-session contract](../specs/spec-host-io-lifecycle.md#split-compiled-session)
requires a host-defined bounded ingress cut before callback rendering. Enqueued
commands act only at new quantum boundaries, after existing carry is delivered.
All available same-time commands preserve FIFO order and ADR-0061's cancellation
of Play followed by Stop before offer. A packet arriving after that boundary gets
a retained delivery refusal; even Stop never silently moves to a later time. The
caller must request a new future boundary after a refusal.

An opaque packet carries session, epoch, plan and identity-table origin. Wrong origin, nonmonotonic
identity/time or unavailable storage returns the owning packet without insertion.
A late packet instead enters a slot marked for refusal at the next boundary; it
never becomes a renderer fault. Before a surviving Play is offered, the audio owner
checks its prepared stopped position against actual playback state. An outdated
position or a no-longer-stopped session also yields a retained refusal. Refusals
stay pending until examined at the head, preserving the completed-prefix invariant.

Taking a completion transfers its box and a scalar playback snapshot; it does not
call `StreamControl::adopted` or `withdraw`. Control collection resolves that same
activation before returning the credit. A collection error returns the owning
packet. Command serials order acknowledged snapshots, so collecting an older
completion cannot rewind the control view. Only one outstanding Play protects the
minter's working-copy rule even while preparation, rendering and collection run
on different OS threads. Stop preparation never mutates the minter.

After the host fences callback access, closing the audio half marks unconsumed
commands cancelled and retains their activations. Packets still in queues or
unpublished remain the host's responsibility. No final callback is required.
The core's pair of handles is not a physical-backend custody proof: a concrete
owner must retain both owners and queue backing through its backend's join.

## Falsifier and current evidence

A lost identified outcome, final command/activation destruction on audio, an
accepted Stop waiting for completion consumption, or a trace delivered before its
deadlines differing from the serial host falsifies this contract. Tests in
`src/host/session/transfer/tests.rs` compare note cuts and resume bit for bit across
1, 37, 64, 256 and 512-frame partitions. Scoped OS-thread tests overlap control
preparation/collection with later audio callbacks. The allocation guard arms before
first enqueue, render and completion transfer. Further cases cover stale positions,
late delivery, out-of-order collection, origin refusal, credit pressure, terminal
buffer faults, byte budgets, identity exhaustion and closure without a callback.

The thread tests use blocking rendezvous outside the guarded callback operations;
they establish disjoint core ownership, not a physical SPSC transport implementation.
The concrete Linux consumer below separately exercises its actual queues and owner.
This split provides no capture/live-ingress constructor, hardware timestamp mapping
or loop implementation. Stopped plan readmission is specified below. Those consumers
retain their existing gates. The design consultation required both the explicit
method decomposition and a new concurrency proof; neither follows merely from
ADR-0063's earlier combined-state custody result.

## Concrete Linux queue consumer

The private `LinuxOutputOwner` in `v2_cpal_output` now holds `SessionControl` and
both ringbuf SPSC backing allocations. Its occupied callback-state pool holds
`SessionAudio`, the command consumer, completion producer and optional owning
failed-transfer packets. The CPAL closure still owns only the pool consumer.
Explicit stream-first destruction joins ALSA before either owner or any backing
allocation can be released. This actual owner graph extends ADR-0063's proof;
no public loose handle pair is claimed to impose that order.

The callback first validates the complete buffer and counters, captures the
command ring's occupied prefix, then admits exactly that bounded prefix before
rendering. Later publication waits for another callback. A protocol refusal keeps
its owning packet in callback storage and faults the adapter. Completion pressure
leaves outcomes in the audio owner or its retained return slot; it cannot prevent
a previously admitted Stop from rendering. Failed pushes always return ownership.
The two queues each have four slots, matching the core credit limit. A separate
16 KiB host transfer budget charges owner layouts, ring objects, packet cells, Arc
control blocks with alignment padding and reserved failed-collection cells. A
test measures actual queue allocation to check the conservative charge.
Core command boxes and control
metadata retain their own budgets. Collected scalar receipts form off-thread
harness diagnostic history; they no longer own a credit or activation.

Opening prepares the owner without starting playback. Initial commands can be
published before Ready and `play`. Every start/wait result proceeds through close.
After join, close collects completions, cancels unconsumed runtime commands and
recovers queued or failed-transfer packets. It reports every identified outcome;
a collection error retains the packet until off-thread cleanup and names its ID.
Implicit destruction uses the same collection routine and reports cleanup errors.
An earlier run failure and a close failure remain separately visible.

The example tests exercise real queues on two OS threads, completion saturation,
publication after the ingress cut, failed sends, malformed buffers, protocol
refusal, every retained packet location and closure without a final callback.
The test-only allocation counter guards first callback use and closure destruction;
a regression also proves it detects freeing a previously allocated value. No new
repository `unsafe` implementation or shipping allocator dependency is introduced.
Physical `transport` runs exercise four applied commands on a silent graph. They
establish bounded ALSA output/transport wiring, not audible note, timestamp, input,
MIDI, plan-swap or loop qualification.

The 2026-09-12 local Linux check used `alsa:hw:CARD=0,DEV=0` (I32) and
`alsa:plughw:CARD=0,DEV=0` (F32), both stereo 48 kHz with a negotiated 512-frame
period. I32 and the F32 retry each completed 100 callbacks, delivered 51,200
frames, reached render clock 51,136 and reported four applied commands with joined
shutdown. The first F32 attempt instead reported a backend error after 43
callbacks: its fourth command remained cancelled and recoverable at close. That
attempt did not retain the CPAL error category, so its cause is unknown. The
harness now retains all observed error categories in fixed status bits, reports
them off-thread and has an allocation-guarded category test. The failed attempt
is not erased by the retry; these observations do not establish stable physical
timing or Phase 9 exit.

## Stopped prepared-plan readmission

The split core now accepts an opaque `PreparedPlanReplacement` containing a fresh
control/audio pair. Its device geometry and epoch match the outgoing pair; its
identity table is new even when the same `CompiledPlan` was cloned. Only this
compiled-session owner can reach the private same-epoch constructor. Raw events,
live ingress, capture and authored Note YAMS remain outside this constructor.
Scheduler, activation and ordinary-command table preflight protects the reachable
same-epoch paths. ADR-0047's outstanding-obligation refusal remains in force;
ADR-0048's held-note transition is not enabled by this stopped-only stage.

Preparation requires acknowledged stopped playback, zero ordinary command credits
and no obligations in the authoritative minter. Installation separately inspects
the renderer's live-note registry: stamping a closed compiled list can leave the
minter empty while Stop freezes a sounding note in the renderer. Candidates name
the authoritative table whose minter preparation checked. If an intervening
uncollected readmission changed the current table, installation also
checks that pair's retained preparation-time minter-obligation flag. Command
watermark equality and the frozen credit interval establish that this intervening
minter has not changed. A later candidate cannot skip obligations in an earlier
uncollected pair. The registry query scans at most the admitted identity partition
and changes nothing. A replacement with outgoing obligations receives a retained
refusal, rather than waiting indefinitely while transport admission is frozen. This
gate protects note obligations; it is not a crossfade, tail-preservation or general
DSP-state migration mechanism.

Exactly five replacement credits cover caller-held unpublished candidates,
mailbox cells, in-flight installation and every retained return. The first free
credit is checked before synchronous preparation; no successful prepared candidate
exists outside this ledger. While any credit remains, Play and Stop preparation
refuse. Explicit cancellation or collection returns a credit off-thread; dropping
an unresolved packet cannot reopen admission. Compiler failure happens before
preparation and must leave a host's last valid publication intact. Preparation
failure changes neither the current pair nor any previously prepared candidate.

The controller byte charge now includes its ledger and all five owning replacement
box layouts, in addition to the ordinary-command ledger. Concrete consumers raise
their controller budget to 16 KiB. Each pair retains ordinary plan/profile resource
admission. At most five candidate pairs plus the active pair coexist, including a
single synchronous preparation transient; this bounds prepared-resource
multiplicity, not every byte of compiler IR, caller-held compiled plans, caches or
process heap. Box layout is not treated as deep plan memory. The concrete host
must separately budget its mailbox, return queue and failed-transfer storage.

A checked per-connection `PlanPublicationId` identifies each occurrence independently
of `PlanId`. Candidates bind the last acknowledged ordinary-command watermark and
unchanged stopped position. The audio owner refuses older or repeated publication
IDs after either installation or refusal in its own connection; a foreign packet
cannot advance this fence. Pending ordinary commands, changed watermark/position,
closed/faulted audio, live obligations or nonempty old output carry also refuse.
The host takes a candidate only after reserving an owning return location and must
collect refusals as well as installations.

At an empty-carry boundary, the fresh renderer receives the actual current clock
and stopped position and removes only its own initial silent quantum. No old
rendered sample is discarded and no second priming quantum is inserted. A host can
use `frames_until_plan_boundary` to drain old carry, then render at most one new
quantum per core call while checking its mailbox between quanta. It still takes
only one ordinary-command ingress cut per backend callback. The core introduces no
host closure or queue dependency into the scanned real-time region.

Installation swaps the whole audio owner into the packet and returns that same
box, now containing the outgoing audio resources. The old control stays on its
thread. Collection checks the actual outgoing table, copies the whole installation
snapshot, synchronizes the new control anchor and preserves command/publication
issuers, closure state and unresolved credits before exchanging controls. This
allows multiple increasing candidates prepared during one frozen interval to
install before collection. An out-of-order retirement returns its owning packet
until the earlier outgoing table has been collected. Reclamation occurs off-thread.

A clock or epoch reset, second priming quantum, lost outcome or credit, old-table
routing, or final resource destruction on audio falsifies this readmission contract.
Tests in `transfer/replacement/tests.rs` cover repeated plan clones, stale/foreign
packets, credit pressure, preparation failure, stopped-mid-note refusal and release
recovery, delayed collection, OS-thread handoff, closure without a final callback,
and exact audio/clock equivalence under 1, 37, 64, 256 and 512-frame partitions.
Callback installation and refusal paths run under the allocation/destruction guard.
The new methods join the source-level real-time scan. The concrete mailbox below
extends this proof; physical plan-swap timing and held-note swaps remain separate.

## Linux latest-wins plan mailbox

The Linux example uses `triple_buffer` 9.0.0 for three owning optional packet cells
and a one-slot ringbuf retirement queue. `PlanControl` owns the input endpoint and
retains the retirement backing through ALSA join. `PlanAudio`, including its
output endpoint and failed-return slot, stays in the occupied callback pool. The
dependency is development-only; the shipping dependency graph does not gain it.
The reviewed operation is a bounded atomic index exchange, with no callback retry
loop. Both endpoints retain the shared allocation until off-thread cleanup.

Control compiles and admits a candidate before changing the mailbox. A compiler
failure therefore preserves the last valid publication. After publishing, control
immediately takes and cancels the old back value that became producer-private.
It does not wait for another edit to release that credit. Audio first checks return
capacity, then takes the latest available candidate at an empty-carry boundary.
A full retirement queue defers adoption; an optional failed push retains the
owning box and must flush before another adoption. Refused installations return
through the same owning path. An invalid protocol packet is retained and faults
the harness. The five core credits cover all these locations together.

Withdrawal publishes an empty cell without requesting a sixth credit. It cancels
unread publication and collects available returns; a candidate already taken by
audio remains outstanding until collection. The returned boolean reports whether
all credits have resolved, not a promise that audio has observed the withdrawal.
After join, cleanup visits the retirement queue, failed-return slot and all three
mailbox cells, including the back cell recovered by an empty publication. Every
installation, refusal and cancellation is reported by publication identity on
the control thread. Collection errors retain their packets and report their IDs.

The host's 16 KiB transfer charge includes a conservative 4096-byte reservation
for the mailbox allocation, the one-slot return ring with aligned Arc header and
five failed-collection pointers. The reservation covers the audited padded-cell
layouts of `crossbeam-utils` 0.8.22, whose largest padding is 256 bytes. An allocation
test checks all four actual mailbox/return/failure allocations against the charge.
The dependency-version guard requires requalification when those implementations
change. Scalar receipt history remains off-thread and outside prepared-resource
charges; these figures do not claim to measure deep plan memory.

The callback validates its complete buffer and counters before touching ingress.
It still takes one command prefix per backend callback, but drains old carry and
renders at most one quantum per core call to expose mailbox boundaries. Tests
exercise actual thread exchange, 20 overwrites under retirement pressure, credit
exhaustion, withdrawal, all three cells at close and protocol-error retention.
The actual callback test publishes a valid constant plan followed by a failed
compile, then verifies identical nonzero audio and clock under 1, 37, 64, 256 and
512-frame partitions. First callback use, installation, refusal and closure-end
destruction are allocation/destruction guarded. Zero, one and two callbacks before
closure disappearance verify cancellation or installation after joined recovery.

`plans` mode exercises four identified silent-plan publications, including two
successive publications and withdrawal, while stopped. A refusal, unresolved
credit or missing receipt at the callback target fails the run. This mode measures
bounded wiring and custody; it does not qualify physical timing or sounding-note
transitions.

On 2026-09-12 the local `plans` run on `alsa:hw:CARD=0,DEV=0` completed 100
callbacks at stereo 48 kHz and a 512-frame period, delivered 51,200 frames and
reached clock 51,136. All four publications resolved: the first and third installed,
the second was overwritten and the fourth withdrawn. The first F32 `plughw` run
resolved the same four publications but failed with a recorded `Xrun` after 71
callbacks (36,352 delivered frames, clock 36,288). Both streams joined; no owning
plan credit remained. The full repository gate was running concurrently, so this
is a retained functional observation under load, not controlled timing evidence.
A subsequent F32 retry completed all 100 callbacks with the same four outcomes,
51,200 delivered frames, clock 51,136 and no backend error. It does not erase the
first attempt's `Xrun` or establish its cause.
