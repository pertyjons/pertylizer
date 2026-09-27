# ADR-0075: Mixed producer activation ownership

| Field | Value |
|---|---|
| ID | ADR-0075 |
| Status | Proposed |
| Phase | 9 |
| Created | 2026-09-26 |
| Last reviewed | 2026-09-27 |
| Related | ADR-0009, ADR-0022, ADR-0023, ADR-0046, ADR-0047, ADR-0048, ADR-0050, ADR-0051, ADR-0055, ADR-0058, ADR-0065, ADR-0072, ADR-0073, ADR-0074, EVD-0024 |
| Amends | None while proposed. Acceptance requires explicit amendments to affected ownership, release and host-profile contracts. |
| Supersedes | — |
| Superseded by | — |

## Problem and current boundary

Phase 9 needs one admitted stream that can render compiled playback and live
notes through transport activation and loop wrap. ADR-0050 clause 8 leaves
non-compiled hold redemption at activation unresolved and relies on control
being the only minter while a candidate is outstanding. Its clause 3
currently promotes the whole copied identity table. ADR-0051 clause 6's
catch-up may write the live voice's rows. The current
`LiveInputStream` owns control, ingress and renderer as one mutable value; its
offer receipt can still resolve to `AuditionOutcome::Refused` at staging.
EVD-0024 measures components, not this combined host.

This record is a design frame, not permission to lift those refusals. No
persisted, wire, shipping or production-facing contract changes here.

## Candidate ownership rule

Test a single immutable table identity with disjoint compiled and live
producer ranges. The audio callback would be the sole writer of the live
range's identity, hold and sounding state. A source thread would submit
bounded operations, not mint identities. In transport mode, control would
stamp candidates against a copy of only the compiled range and promote that
range when it collects the audio-adopted retirement. In loop mode, audio
would own the compiled range under a custody token. A mode transfer must
revoke the old writer before the new one mints. The live range must not be
overwritten by a compiled candidate or rebuilt at a loop boundary.

Plan admission would bind each note producer to its playable target node
instances and control rows. A one-instance node is shared even when producers
write different controls on it; that overlap refuses admission until a
shared-node law exists. A multi-instance node belongs to the producer whose
identity range contains that instance. The binding must be enforced when a
compiled stream is admitted and when live operations enter ingress, including
note edges, expression, bend and producer-scoped release groups. The renderer's
table identity check alone does not enforce a producer's range or playable target.
Compiled mass release, catch-up, slot seeding and loop restoration would be
scoped to compiled-owned per-note rows. Any per-note row reached by both
producers would refuse plan admission until a shared-row law exists. A
non-note target written by the live parameter lane also needs an explicit
catch-up ownership or ordering law; a compiled catch-up must not silently
overwrite its latest live value. The Session admission sum must cover the
catch-up batch, live parameter lane, Stop group and any reset or other
same-quantum Session contributor. One event per admitted address bounds only
the scoped batch's event count. Payload bytes, timed-control fanout,
scoped-restoration scratch, seed storage and the combined share charge still
need measurement; the old address count does not qualify those resources.

An initial target-binding prerequisite may validate exactly one compiled and
one live note producer in one immutable plan, one bound live note slot and
`StealingPolicy::None`. Its off-thread artifact must own the validated plan and
entire admitted compiled stream rather than accept a replaceable caller list.
It must enumerate every gate and magnitude destination against each producer's
identity range. Foreign or unrepresentable targets refuse. This artifact
returns its plan, stream and live slot on refusal so a corrected binding can
retry without recompilation. It accepts only note-target writers: compiled
`SetParameter` and `Controller` writes refuse, and it provides no live
parameter lane. `Fade`, `Reset` and `RestoreController` require explicit
accounting before a mixed host can emit
them; disabling stealing prevents the first two today but is not a proof for
a future emitter. The artifact does not prepare mixed ingress or authorize
rendering. Current mixed-plan, activation and loop refusals remain in force.
The later combined host must admit same-quantum capacity. A compiled-only
activation must neither write nor reseed live-owned instance-local control
slots, their current ramps, next live writes' ramp behavior or local
modulation histories. A shared upstream source or non-note target whose
change can affect a live instance requires its own declared ownership or
ordering law, or admission must refuse that target.

### Scoped restoration rehearsal

The next restoration slice tests one parameter-group event carrying a checked
contiguous instance span. The renderer must resolve that span to exactly the
compiled producer's rows. A group whose rows belong to both producers cannot
use the existing whole-group `SetParameter` or `RestoreController` payload for
compiled catch-up. One event per row is also invalid: the current Session
charge is one event per address plus the boundary release, so expanding a
group into events would exceed an admitted share without a new charge.

The span must be validated off-thread against both the group's instance count
and the bound plan's compiled producer partition. The mixed renderer must
recheck its plan identity and that exact partition membership before applying
any scoped event; an ordinary exclusive renderer must refuse that event. A
forged or stale span must touch neither a live nor a neighbouring parameter
row. For sample-positioned controls, both passes of timed-control collection
must use the same resolved span. Quantum-rate controls must write only the
span's rows' override and, for a controller source, their controller layer.
Ordinary exclusive-stream payloads keep their whole-group behavior. The mixed
renderer's immutable seed scope must be derived from the admitted instance
partition, not from a candidate's events. An adoption may mark only compiled
rows whose boundary restoration retargets `SlotState`;
sample-positioned writes also retarget it through the timed-control path and
consume the seed flag. Otherwise a skipped live row retains `seed_next` and
its next write or modulation takes an unintended step. Shared-sum and global
rows are omitted from both restoration and seeding. During this rehearsal,
compiled `SetParameter` and `Controller` writers still refuse and there is no
live parameter lane, so no admitted source writes those omitted rows through
the parameter path. Their state persists across the seek. Any influence on
live instances needs the separate ordering or admission law above.

This rehearsal does not authorize an activation. It must first show, with a
held live note, that a compiled-only restoration leaves every live gate,
magnitude, controller, override, modulation layer, ramp and subsequent write
unchanged. A test-only smoothed live row is necessary: all currently declared
smoothing policies are `None`, which would hide a stray `seed_next`. Both
producer orders, sample-positioned and quantum-rate controls, a live edge at
the boundary, an invalid span, and the unchanged state of omitted global and
shared rows must be tested. A span that fits the group but crosses into the
live producer's partition must refuse before any write. The compiled rows
must restore their gates and trigger destinations to zero without a new rising
edge. A test-only smoothed compiled controller row must also take the restored
controller and override together as a step; two successive retargets could
consume the seed on the first and ramp on the second. Payload size, the
combined Session share charge, per-event timed fanout, scoped-restoration
scratch and seed-scope storage must be remeasured before any mixed offer uses
the payload. The compiled history walk, boundary ordering, release custody and
loop restoration remain separate acceptance work.

### Bound compiled history rehearsal

The next off-thread artifact may derive restoration from only the compiled
prefix of the `MixedJoinedPrepared` owner's fixed, admitted stream. It is a
single-use, non-cloneable candidate bound to that owner's plan, epoch and table,
the requested plan position and sample time, and the index where the prefix
ends. It retains the note-pairing book for a later suffix builder and a separate
snapshot of the notes open at the destination; advancing the book through the
suffix must not change which gates the boundary restoration lowers. The request
refuses when no quantum boundary can follow its sample time. The private events
are stamped at the requested time; a later scheduler must shift them by the
same effective-time displacement as the suffix. Preparing this artifact
neither mints into the authoritative range nor exposes an event, render or
activation offer API.

Walk only events **strictly before** the destination in their admitted order;
equal-position events inside that prefix keep their admitted order, and events
at the destination belong to the suffix. Use the plan's no-stealing note
capacity and slot/key pairing rule.
The already sealed initial schedule has checked pairing and capacity for the
whole stream, so a second refusal for those cases is defensive reconstruction,
not an independently reachable public falsifier. Compiled parameter and
controller writers remain excluded by target admission; encountering one here
still refuses explicitly as another unreachable defensive check. Expression
and bend history ends with its occurrence and does not become a parameter
override. For each addressable group, retain the last note-on magnitude
written before the destination, even when that particular note later ended
and another note remains open. This is the existing exclusive last-write law,
not a claim that the value belongs to the still-open note. Note-on and note-off
update gate and trigger history in that same order;
every gate or trigger with an open compiled note at the destination is forced
to zero. A missing prefix write restores the prepared base, and a NoteSource
restores its base because its occurrence has ended.

History is keyed by the parameter group's address, and each emitted event
reads that exact address. An implementation may use the group's first physical
row as scratch, but that row can lie in the **live** partition when the live
producer is first. Its use as a scratch key grants no authority to write it:
the emitted `ScopedRestore` must carry the bound compiled span and the renderer
must recheck it. The group's first `ParameterTarget::controller` flag decides
the payload kind: a true flag uses
`ScopedParameterRestore::controller_for(group, value, None)` so its controller
layer clears with the override in one retarget; a false flag uses
`override_for(group, value)` so sample-positioned gates and triggers take the
timed-control path. The renderer refuses a mismatched kind, so building one
must be a tested failure rather than a silently omitted restoration. The batch
has exactly one event per admitted restoration group and omits shared-sum and
global rows. A future sample-positioned target with `controller: true` must
refuse binding until the renderer has a timed two-layer restore for it.

This candidate is not an activation. Its zero gate and trigger values are
invalid without a producer-scoped boundary release that ends compiled
occurrences before the batch; otherwise a still-sounding compiled occurrence
and the renderer's note registry disagree about what ended. A later release
custody slice must prove the combined render has no new rising edge. Before an
offer, measure the payload and per-event fanout and admit the batch, boundary
release, live ingress, Stop and other same-quantum Session contributors
together. A later suffix builder must distinguish a release paired in its own
suffix from one whose note-on was in the prefix. **Proposed exception to
ADR-0051 clause 5 and SOUND-INV-018 for this bound mixed note-only shape:**
the latter loses both its note contract and its later physical gate event,
but increments the omitted-release count. The producer-scoped boundary
release and scoped zero restore lower the old occurrence's gate and trigger
rows at the boundary. A suffix note-on can raise its own instance row,
including an index recycled after that release. The old crossing release
emits **no parameter write and no trigger off edge** at its old time; even a
one-row write could lower the new occurrence after index reuse. In playthrough,
`note_target` resolves a note-off to its own identity's one row because this
binding refuses one-instance shared destinations. That is the audible
comparison for two simultaneous occurrences on distinct instances, not a
late write this exception retains. The exclusive bare `SetParameter` instead
fans out to the whole group. This proposed mixed exception ends the old note
at the boundary while no independent compiled `SetParameter` or `Controller`
writer is admitted. The builder must never copy the exclusive path's
whole-group `SetParameter` for that crossing release, which would lower live
instances. This exception
requires an explicit amendment of both contracts before a mixed offer uses
it. If compiled parameter writers become admissible, the omission needs a
new scoped gate-down and a revised capacity charge first. A suffix bend or
expression whose note-on was in the prefix is also dropped and counted,
as in the exclusive path; one paired with a suffix note remains in the suffix.

For the private first-activation rehearsal, suffix placement uses one anchor
equating the requested sample time with the destination plan position. A
candidate stamped against a copy of the compiled range retains that copy, its
new outstanding identity set, the anchor and `ActivationSequence::INITIAL` as
the baseline it would supersede. A later offer must compare that baseline with
the audio owner's in-force sequence and refuse a stale candidate; equal private
identity values do not establish freshness. Before stamping, checked release
of the old schedule's outstanding set must leave the copied minter empty;
afterwards its live count must equal the new outstanding set. Without stealing,
each selected source event must yield exactly one stamped event. These checks
are private preparation evidence, not an offer or a combined capacity claim.

The private requested-time list orders scoped restoration before suffix
events at the destination. At a future effective boundary, the combined order
must be producer-scoped release of the *old renderer's sounding compiled
occurrences*, then that restoration, then those suffix events. Both prepared
parts must receive the same effective-time displacement. The destination-open
snapshot describes the *new timeline* and selects zero gates and triggers for
restoration; it does not name old-renderer occurrences to release. A suffix
note-on exactly at the destination must follow restoration, or the restoration
could lower its gate and replace its magnitude. A private prepared list may
prove the latter two steps' order now; the first step, effective-time shift,
live preservation and same-quantum capacity remain prerequisites to an offer.
A destination-time note-on rendered silent or re-pitched by restoration
falsifies this order.

The private candidate checks the complete list's nondecreasing stamped-time
order off-thread. A proposed effective quantum boundary then needs only the
latest event to prove that every event fits one frame displacement for
restoration and suffix. It refuses a boundary before the request, a non-quantum
boundary, a signed difference outside `FrameDelta` and any displaced event beyond
engine time. Accepting a boundary where restoration fits but a later suffix
edge overflows would falsify this check. The list
keeps its requested-time stamps; the future audio-side offer must select and
check its actual boundary and apply the same displacement at every read. This
private timing check grants no offer or combined capacity admission.

The renderer now has an internal producer-scoped boundary-release rehearsal. It
previews sounding notes in caller-owned storage, checks the bound compiled
producer, each gate and trigger row against the compiled partition, and the
prepared queue capacity before clearing the registry. It then calls the same
`seed_for_adoption` path and gate/trigger enqueue path as ordinary adoption;
the seed is scoped to compiled rows because this renderer is mixed-bound. The
future activation owner must publish complete scoped restoration in that same
boundary quantum so seed marks on rows without a release write do not survive
to a later ordinary write and turn its ramp into a step. A short ended-note
buffer, a live-producer request, an unbound target, insufficient gate storage
or a pending boundary must leave the registry and pending gates unchanged.
Renderer tests use directly stamped compiled-provenance and
simulated live note-ons against an admitted compiled target, in both producer
orders. They check a sampler trigger, audible live preservation, and synthetic
missing gate/trigger partition rows. These falsify a release that ends nothing,
touches live state or fails to apply boundary controls. The helper is not yet
called by a mixed activation owner. A direct renderer test now combines release
with a complete same-quantum scoped restoration in both producer orders: the
next smoothed compiled frequency write ramps while audible live output matches
its live-only reference. Complete seed consumption by an actual mixed owner,
effective-time selection, combined capacity, source custody and offer remain
unproved.

The bound audio half now reserves ended-note storage for its compiled span
off-thread. A private boundary operation requires the renderer clock to equal
the selected effective quantum, then releases the compiled partition and moves
the musical anchor as one step. It returns both the released count and the old
anchor; a refusal leaves the anchor unchanged. The delayed-boundary rehearsal
seeks to a different musical mapping and checks the moved and returned anchors,
zero allocations in the boundary operation and mixed audio against the sum of
live-only and compiled-only references in both producer orders. The future
combined schedule owner must suppress old events at and after the boundary and
retain the returned anchor with the old compiled list until off-thread
reclamation. The combined host must charge this storage and publish the
complete scoped restoration in the adoption quantum; no mixed schedule invokes
the operation.

The private candidate now also exposes a checked effective-event view. It
keeps the requested-time list intact and adds the same validated displacement
when reading each scoped restoration or compiled suffix event. A renderer test
uses this view at a later quantum boundary, releases the old sounding compiled
occurrence before rendering the candidate, and renders restoration before the
destination note-on. In both producer orders, mixed audio matches the sum of
live-only and compiled-only references within 1e-5; the compiled voice has
the same old oscillator history. A release-only reference is silent in the
boundary and following quanta while the destination note remains audible.
The test supplies a directly stamped live onset; it does not establish source
custody, combined capacity or an audio-owned mixed activation offer.

The private candidate's event-count terms follow from existing admission for
this note-only shape. Plan admission refuses when the full addressable catch-up
count plus one boundary mass release exceeds the Session share. The mixed
restoration groups are a subset of those addresses. The suffix selects from one
`AdmittedCompiledStream`, whose sliding-window admission covers every anchor
phase; deleting events and applying one uniform time shift cannot increase its
per-quantum maximum. The profile's six-share relation covers these two event-count
contributions together. This private bound is false if restoration emits more
events than admitted addresses, the shifted suffix exceeds the compiled share
in any quantum, or the boundary release needs more than its one Session charge.
It does not measure payload bytes, timed-control fanout, scoped-restoration
scratch or seed storage. It also does not admit a combined host: an old compiled
schedule publishing at or after the effective boundary would double-spend the
Compiled share, and another same-quantum Session contributor could exceed its share
beside restoration and release. The future owner must suppress the old schedule
at the selected boundary and account for every other contributor before
offering activation. An offer lacking either guarantee fails that later
combined-host admission, not the private event-count bound.

Rehearsal falsifiers are a last note-on magnitude before a seek with compiled
producer second, repeated-key prefix pairing, zero gates and triggers for
destination-open notes, exact group count, immutable owner/minter custody,
and no writes to live, global or shared rows. A group without a prefix write,
or a NoteSource group, restored to anything but its prepared base fails. So
does expression or bend history appearing as an override, or a group built
with a payload kind that disagrees with `ParameterTarget::controller`. Accepting
a request with no following quantum boundary, shifting the batch differently
from the suffix, or changing the saved destination-open set while pairing the
suffix also fails. An event exactly at the destination entering the prefix,
or equal-position prefix events changing their admitted order, fails. So does
a crossing release omitted without a counted outcome, one that emits any
parameter or trigger write at its old time, or a crossing bend or expression
left in the suffix without its prefix note. A prefix note A followed by a
suffix note B on the same slot but another key must leave B sounding through
A's crossing release. That release is counted exactly once as omitted. With
a repeated key, a release or bend paired to the suffix note must remain in
the suffix and must not be counted as omitted. After B reuses A's released
index, A's crossing release
must emit no write to that reused row, including no trigger off edge. A
crossing bend or expression must increment only the omitted-expression count;
a crossing release must increment only the omitted-release count. A value-only
test of the private batch does not discharge the combined release or
audible-edge falsifiers.

An independent design consultation rejected a uniform scoped payload kind,
a producer-owned interpretation of the first-row history key, unreachable
public refusal tests, an ambiguous "last pitch" rule, and a value-only claim
of no rising edge. The conditional payload, address-keyed scratch, defensive
check labels, precise last-write rule and release prerequisite above resolve
those findings. The independent uncommitted read also required an explicit
controller-flag rule and mismatch falsifier, a strict prefix boundary, fuller
falsifiers, consistent defensive-refusal labels, and treatment of the
crossing release's unscoped exclusive write. The proposed counted exception
above addresses that last hole for this bound shape; its amendment remains
required before an offer.

### Private one-shot audio rehearsal before an offer

An internal, non-shipping rehearsal may move one sealed initial compiled list
and one privately stamped candidate to a split audio owner. This permission
does not lift the mixed offer, ingress, command or loop refusals and does not
satisfy combined-host admission. No public API or production host may call it.
The rehearsal tests one transition only; a successful arm forbids a second arm.
An arm refusal returns the stamped candidate before capsule construction and
leaves the joined owner unarmed and off-thread for a new attempt or off-thread
drop. It does not consume the one transition.

The off-thread half retains the bound control and initial outstanding set. It
consumes or drops the candidate's source history off-thread and moves a boxed,
sendable capsule to the audio half. The capsule carries the plan, epoch, table,
requested anchor, `INITIAL` baseline, successor sequence, omission counts,
restoration prefix count, ordered stamped events, copied compiled minter and
new outstanding set. The audio half receives the entire bound
`MixedStreamAudio`, including its live range, instance partition and
preallocated compiled ended-note buffer, and owns the initial event list. Arm
runs off-thread while the renderer is stopped, with both halves available to
check plan, epoch, table and `INITIAL`. A refusal returns the stamped candidate
on that thread, before a capsule box exists; neither that candidate nor a
successfully built capsule is finally dropped in a callback. The arm
fixes the first complete quantum at or after both the request and the next
unrendered quantum (`PreparedRenderer::clock()`), even when earlier output
remains in carry. It records the checked uniform displacement, effective anchor
`(effective time, destination position)`, and whether the request preceded the
arm clock. A timing or pairing refusal leaves the renderer and control minter
unchanged.
The recorded late-at-arm bit is defensive while a stopped joined owner has no
render path; this private callback does not count a late activation.

The armed audio owner remains pending until its fixed boundary. It owns exactly
one prepared publication arbiter; no callback accepts a caller-supplied second
arbiter, and its storage returns for off-thread destruction with the owner.
The private callback steps at most one new quantum per publication because this
arbiter prepares one quantum. Carry-only steps publish nothing and do not adopt.
Adoption happens only at the start of a step that opens a new quantum at the
fixed boundary; a callback ending exactly at that boundary leaves the owner
pending. A zero-frame call is a no-op. Output shape is checked before rendering;
a mismatch returns untouched output and may be retried.
Each nonempty callback, before or after adoption, preflights the renderer's
plan, epoch, table and fault state before any publication or boundary change. While
pending, an output-shape refusal keeps the fixed boundary. An internal
plan/epoch/table mismatch or already faulted renderer is terminal and silences the
complete callback. A call served entirely from carry does not adopt. In a call
crossing the boundary, the head publishes every old
event strictly before it; the tail starts with adoption. A head fault silences the
whole callback and leaves the candidate unadopted. The boundary step's clock,
partition, producer, pending-boundary, ended-storage, gate-storage and
unbound-target refusals are terminal too, with the candidate unadopted.

Adoption releases only the compiled partition and moves the renderer to the
effective anchor. The capsule receives the old event list and returned **old**
anchor; its requested-time anchor is never promoted when adoption is delayed.
The retired capsule records the old list's unconsumed cursor and returned old
anchor separately from its requested anchor; its event counts continue to
describe the adopted candidate after the list swap.
The new list starts with scoped restoration, followed by the compiled suffix.
The first new quantum charges one release operation and the entire restoration
prefix to Session, then the due suffix to Compiled. Successful render clears
those debts. The adopted audio owner keeps the retired box while it continues
rendering later callbacks; neither the box nor old vector is finally dropped
on audio. No second arm is accepted. The control half cannot mint or prepare
another schedule while the capsule is away.

If any publication or render fails after adoption, including in a later
quantum or callback, the owner is terminally faulted-adopted and silences the
complete failing callback and subsequent output. It retains the retired box;
no release or restoration debt is retried. Its report identifies the effective
and old anchors, successor sequence, release operation, restoration and suffix
charges that reached the arbiter, and completed render quanta. A render failure
may already have changed DSP state. A terminal fault before adoption is
faulted-unadopted and retains the pending box and initial list. At off-thread
teardown, both terminal states classify sounding compiled and directly
injected live notes as ended; they do not use the compiled ended-note buffer
for live identities. An unadopted capsule's privately minted identities were
never published or sounded and are discarded with its minter copy off-thread;
they are not reported as ended renderer notes. Neither terminal state returns a
runnable control or reuses the initial outstanding set.

Only after audio rendering stops may off-thread rejoin inspect the box. Every
rejoin or teardown, including healthy pending teardown, checks the original
control/audio pairing before consuming either half; a crossed pair is returned
unchanged for correct rejoin. A healthy adopted rejoin atomically promotes the
capsule's copied minter, new outstanding set, effective anchor and successor
sequence, reclaims its old list and returned old anchor off-thread, then may
resume the adopted stream. The original pairing check uses the birth-bound
control and audio halves. Capsule plan, epoch and table are checked separately
before promotion; a mismatch terminates the correctly paired owner. A defensive
promotion refusal, unreachable from the checked private path, terminates both halves off-thread,
classifies sounding notes as ended and returns no runnable control. A healthy
pending owner that stops before its boundary is torn down off-thread with its
capsule, initial list, control and initial outstanding set; the capsule's new
outstanding set is discarded as never published. Any sounding compiled or
directly injected live notes are classified as ended there without putting live
identities in compiled ended-note storage. It returns no runnable control and
cannot resume or rearm. Terminal teardown likewise destroys both halves and
both lists off-thread. The accepted candidate cannot be withdrawn while audio
runs.

The off-thread arm now checks the ended-note span against the renderer's own
compiled registry range, which must equal the bound partition, and checks the
actual gate, trigger and timed-control storage. The timed-control bound includes
a full external event quantum beside the compiled release queue; the queue's
per-note width conservatively includes every magnitude, though release writes
only gate and Trigger rows. These storage inequalities hold by valid renderer
construction; a defensive refusal after corrupted storage returns the stamped
candidate and joined owner unchanged, with the one transition still available
for a corrected attempt. This storage check alone does not prove the combined
producer load, scoped fanout or restoration-plus-suffix-and-live event count.
Arm also compares the supplied profile's sample rate, layout, maximum block,
event cap, Compiled share and forward horizon with the bound plan. The Session
share comes from that profile and is checked against the candidate's actual
closed-schedule charge. Arm checks the actual restoration/suffix payload split
and nondecreasing effective times. It charges restoration plus one release
operation to Session and checks every shifted suffix quantum against the same
Compiled share as the old admitted stream. It prepares the single-quantum
arbiter before consuming the
candidate. A sample-positioned scoped restoration span must fit the plan's
admitted fanout; with the prior storage check, the closed private schedule's
event and control writes fit the prepared buffers. Every admission refusal
returns both inputs unchanged. This closed schedule has no production live
ingress or other Session contributor. A single test-only simulated live onset
may be armed off-thread before rendering, only before the boundary and within
the supplied profile's Live share. The callback publishes it as Live after
Session and Compiled in the same arbiter, without a source receipt or production
ingress. A private callback now renders the admitted closed schedule one quantum
at a time. Its read-only report records adoption and fault state, the effective
and retired anchors, sequence, compiled release count, release and restoration
charges and cumulative suffix charges reached by the arbiter, completed render
quanta, and the first terminal cause. Every private callback failure is terminal
and silences its complete output block, even if a prior step in that block succeeded; output
shape refusal is the retryable exception. Tests compare one-call, 64-frame,
8-frame and irregular partitions with a held live note in both producer orders,
and compare boundary and later held-live audio with a live-only renderer after
asserting that a sounding compiled note contributed audio before the boundary.
The one test event spends at most one Live credit in its own quantum; the
profile's shares sum to no more than the event cap, and the renderer's prepared
scratch covers the plan's widest event beside the compiled release queue.
Before extending this private path to more contributors, off-thread
admission must prove that all contributors in that path fit their shares and the
event limit and that their total control writes, including scoped restoration
and the compiled release, fit the timed-control storage. Otherwise the renderer
can refuse the event span or silently omit a control write when its scratch
fills. A queued prior release remains a defensive arm refusal and a terminal
boundary fault if it somehow appears later. The private fault rules do not
decide ADR-0073's future combined-host terminal scope. Payload size, fanout for
other event classes and producers, scratch and seed charges, and the
crossing-release
amendments to ADR-0051 clause 5 and SOUND-INV-018 remain necessary before a
production offer, alongside the other combined-host acceptance work in this ADR.

The rehearsal's defensive second-arm seam fails if it changes owner state.
Ordinary rehearsal checks fail if carry-only output adopts; old boundary-time
events publish; preboundary events disappear; restoration spends Compiled
credit; audio callbacks allocate, deallocate or finally drop the retired box;
or a held live note's audio, row or ramp changes in either producer order.
With the same admitted source prefix and arm clock,
every compared callback must succeed and one-call, 64-frame, 256-frame and
irregular partitions must render bit-identically with equal counts. Tests must
place an old event immediately before and at the boundary, a destination onset
there, and a bound live-range note published through the same arbiter as `Live`
and held across it. That seam grants no source receipt or production ingress.
Fault tests must cover arm refusal and off-thread drop, wrong-call retry at the
fixed boundary, carry-only output, head and boundary-step faults, failed first
and later new quanta with the charge report, terminal plan/epoch/table and
already-faulted preflight, crossed-pair rejoin, crossed healthy-pending
teardown, and terminal teardown. Collection tests must prove that a delayed
boundary promotes the
effective anchor, copied minter, outstanding set and sequence together,
reclaims the retired list off-thread, and tears down a still-pending owner
without returning runnable control, with sounding compiled and live notes
classified as ended, no live identity in
compiled ended storage, and private unadopted identities discarded. A
test-only promotion refusal must end the correctly paired owner off-thread.
These are rehearsal checks, not the combined-host acceptance matrix.

### Protected note credit rehearsal still needs a law

A note-on/key-release-only rehearsal is a possible next slice, but no credit
equation is accepted yet. An independent design read falsified an attempted
`Q + H <= C` rule: the concrete bridge has one 128-operation outstanding
credit across queued input, renderer cells and retained results, not three
interchangeable credits. Splitting that custody or returning an ordinary
packet after its receipt would change ADR-0073's terminal overload policy and
needs an explicit amendment. A protected source lane also needs an order law:
the current source producer retries a full-ring packet before later packets,
while the live renderer stages by nominal time, source rank and serial. The
renderer admits a later serial with an earlier nominal time, so source arrival
order alone cannot pair its releases. A new consumer must either reject that
regression before receipt or prove a shared execution order across boundaries.

The future proof must carry a potential onset, its source/channel/key FIFO
tombstone, tracker cell, identity, ingress hold, protected release path and
result custody through every handoff. An onset refused after an upstream
receipt cannot simply disappear: with repeated keys, its later release would
otherwise end a newer sounding note. An accepted queued packet that later
refuses needs a result cell or a defined terminal disposition; returning its
original after receipt is not an available outcome in the current bridge.
Ingress minting and tracker writes happen immediately, while
`LiveInputStream` reports their outcomes only after the callback succeeds.
On later failure it closes the owner and marks unresolved entries, including
staged and still-queued ones, `Cancelled`; it does not undo minted identities
or held cells. Joined teardown must classify those remaining obligations.
Any nonterminal callback rollback would be a new contract under item 1's
terminal-fault scope and an explicit ADR-0073 amendment. The eventual
acceptance rule must account for callback-size dependence of committed versus
cancelled outcomes. Tracker occupancy survives result collection, so a result
cell cannot replace a held tracker cell. A repeated-key onset without a
matching release retains an obligation until an explicit termination or
joined cancellation rule resolves it.

Before any note-only rehearsal claims protected release, saturate each
boundary with earlier ordinary packets and uncollected outcomes, then show
that the exact reserved source/channel/key release reaches audio after bounded
service without skipping an accepted prefix. Test a refused first onset
followed by an accepted second onset on the same key, a later serial with an
earlier nominal time, a failure in a later quantum of one callback, and a
collected result whose key still occupies a tracker cell. Joined teardown
must retain or classify staged outcomes, completed but uncollected results,
key tombstones, queued control-to-audio packets, accepted pending releases
and unexamined source packets without requiring another callback. Pedal,
mass release, bend, transport, reset, activation and loop still need their own
credit and redemption laws; this paragraph grants no mixed-render permission.

The concrete source ring is also the only path into raw capture. An audition
packet refused before that ring cannot still be captured through the current
path. Raw capture has 32 cells per source, reserves one for frontiers and
invalidates the host when its ordinary cells fill. Its receipt collector is
separate from audition result collection. A release reserved only in audition
can therefore fail before reaching audio under raw-capture starvation; a new
law must include capture credit or state the resulting terminal disposition.
It must distinguish an audition source receipt from raw capture's
`InputReceipt` and identify where each is settled.

A source-side key ledger needs one FIFO for both receipted and refused
onsets. If the first same-key onset is receipted, the second refused and two
releases follow, a refused-only tombstone queue consumes the first release
and forwards the second as if it belonged to the first onset. The release is
delayed and misattributed; without the second release the note remains held.
Nominal mapping currently happens after the source ring. Checking mapped time
before receipt would require moving the mapping earlier; a failure after
receipt needs explicit custody redemption. A release
staged before a later callback failure may already have spent its ingress
hold and freed its held cell even though its outcome becomes `Cancelled`.
Joined teardown must classify the actual note and release state, not infer it
from that outcome alone. Add these counterexamples to any protected-release
rehearsal before claiming it passes.

### Split identity custody is not a table copy

An independent design consultation rejected a late merge of a compiled
candidate's whole identity-table copy with a concurrently moving live range.
Once audio has adopted an activation, refusing its rightful retirement during
control collection leaves the renderer playing identities the authoritative
minter does not own and leaves `in_force` behind the scheduler. Builder checks
and audio-side offer guards must ensure that a normally adopted retirement
passes rightful collection. The offer must require the candidate's
`supersedes` to equal audio's `in_force`; the single exchange slot must refuse
another offer while pending or retired; and control's `sequence <= in_force`
check must refuse replay after promotion.
`StreamControl::adopted` still returns the box on `ForeignStream`,
`ForeignTable`, `NotAdopted` or `AlreadyPromoted`; the latter two cover a
wrongly submitted unadopted candidate or an invalid replay, not a normally
adopted retirement.
The compiled range needs exclusive writer custody until collection, which
must promote its minter, outstanding set, anchor and activation sequence as
one operation.

Matching compiled slots to a baseline is not a freshness test when two
candidates contain the same slots; activation sequence still decides that.
Unadopted candidates may mint identical index/generation pairs in private
copies. The falsifier is an identity from such a copy reaching rendering, not
the existence of equal private values.

A per-producer minter would need a one-time split, with `TableId`, `ProducerId`
and span provenance checked together. Existing `IdentityTable` methods index
absolute slots and allow whole-table release, copy and rebuild; merely handing
each thread a shorter vector of that type is unsound. A live owner must expose
neither compiled-range minting nor an audio-thread copy or final drop. Its
split must also be coordinated with stream control and scheduler custody.
`StreamControl::plan_activation` retains a whole-table working copy in each
candidate; `stamp_compiled_counted` takes a temporary copy and writes the whole
table back on success. Every mixed control, both initial `open` and a
replacement through `replace_compiled`, must be born split in
`open_in_epoch` or an equivalent constructor before its control value is
exposed to any caller or `SessionRuntime::prepare_detached` stamps it.
`Origin.table` must match the split owners' `TableId`; each owner must also
retain its producer-span provenance.
The constructor alone cannot bind later work: `CompiledEventScheduler::prepare`,
`SessionRuntime::prepare_detached`, `plan_activation` and public
`stamp_compiled` currently accept a caller-supplied stream or event list.
The target-binding artifact owns the plan used for its target checks and
validates its admitted stream and live slot against that plan. The mixed
constructor must consume this owned triple, move it into a non-clonable split
owner and bind it to that owner's epoch and `TableId`; it must take no separate
plan argument.
Plan identity is necessary for that match but does not establish exclusive
custody: callers can admit equal streams again. Every mixed stamping path,
including a seek, must read the `AdmittedCompiledStream` stored
inside that owner; its API must take no later caller-supplied stream or event
list. `CompiledPlan` and `AdmittedCompiledStream` remain clonable, while the
target-binding artifact is non-clonable. The owner consumes the artifact; the
absence of a later stream parameter, not the artifact's `Clone` status alone,
prevents a separately admitted equal list from replacing it. A compile-fail
API test must show that no separate plan or second stream can be supplied to
the first owner's stamping path. Dropping an owner must leave no usable stamp
authority for its table.
A replacement plan needs a fresh target binding against that plan, moved into
a fresh split owner before the replacement control or renderer is exposed in
the existing epoch. The old binding cannot authorize a new plan's events.

Ordinary `StreamControl::open` and `open_authored` still reach
`open_in_epoch` with a whole-table minter, and `replace_compiled` does too.
They may keep their compiled-only uses of a plan with mixed declarations, but
mixed ingress must require the distinct split-born owner. That owner must not
expose ordinary `StreamControl` live offers or `minter_mut`, which would
operate on the control-side minter. The existing mixed-ingress refusal remains
until this separation is implemented and tested.

Candidate offer, control-side collection through `StreamControl::adopted`, and
withdrawal currently compare `TableId` without producer-span provenance.
Audio-side `PreparedRenderer::adopt` makes no table comparison. Mixed custody
must check the compiled producer and span at offer, collection and withdrawal,
and prove that audio adoption cannot receive a candidate with the wrong scope.
The renderer's live-note registry can admit an event into any index of its
table, so it must also reject a compiled edge naming a live-range index before
it changes a live registry row. The target binding's span arithmetic and the
split minters' spans must come from
one authority or be compared at construction.
There must be no later split of an ordinary control: public `stamp_compiled`
can mint without a scheduler and can leave no outstanding note while still
advancing generations, so a live-count or scheduler check cannot prove that
an ordinary control has never stamped. This constructor boundary excludes
every pre-split candidate and stamp. Today
`IngressPrepareError::MixedProducerPlan` refuses mixed ingress, `latch_store`
refuses outstanding candidates, `plan_activation` refuses an adopted ingress
store, and stamping refuses outstanding candidates. Lifting
those refusals requires new range-scoped stamping and candidate copies, while
the old whole-table paths remain unreachable for that stream. A standalone
range split without this stream-control handoff fails even if its ranges are
sound. Acceptance must falsify two interleavings: a live mint or release after
a compiled candidate snapshot but before its promotion by
`StreamControl::adopted`, and a live mint or release during a compiled stamp's
copy/commit window. A later live mint or resolve must not see a rewound
generation, duplicate identity or lost obligation in either case.

Compiled-only activation also reaches audio-side state: renderer adoption
reseeds every parameter slot, and catch-up currently fans writes across every
voice instance. The split must scope both operations to compiled-owned rows
and preserve live-owned current ramps, the next live write's ramp behavior and
local modulation history. A held live note that survives identity resolution
but changes gate, magnitude, ramp or modulation at adoption still falsifies
the split. Shared upstream sources and other non-note targets need an explicit
ordering law or admission refusal before the mixed host can render.

The live owner's source receipt still needs a stable way to name a later
release, bend or group before an audio-owned minter returns an identity. The
hold entitlement and identity mint must be acquired or refused atomically
under HOST-INV-009; moving only the mint to audio leaves a hold stranded on a
late refusal. The control-side replacement guard currently reads its minter,
candidate count and live-note diagnostic. The session audio guard checks the
renderer registry and a prepared control-minter obligation snapshot when the
table changes; the live swap guard checks renderer obligations and ingress
holds. A split owner must keep all those guards authoritative.
`IdentityTable::rebuild` itself reads only that table's live count and cannot
stand in for a mixed-owner obligation check.

The interval from candidate preparation through control collection has
distinct custodies. Before offer, a candidate may be held by its caller or
by a `PreparedSessionCommand` in transit or a `SessionRuntime` command slot
on the audio side. A refused send returns the prepared packet and its box for
`SessionControl::cancel`. After offer, the candidate is in the scheduler's
`Exchange::Pending`. After adoption, its box owns the new compiled minter and
outstanding set in `Exchange::Retired`. An offer refusal returns the box to
the immediate caller; the session runtime restores it to the command slot.
A refused capture start restores the unoffered candidate to that slot too.
`take_retired` can move a retirement back into its adopting command slot or
to a `CompiledEventScheduler::collect()` caller. On close,
`close_exchange` can instead return either Pending or Retired to a session
command slot. The serial host can resolve a candidate in its command slot
through `SessionRuntime::collect`; a refusal restores the box there. The
transfer host can use `SessionAudio::take_completed` to move a command entry
holding either an adopted retirement or an unadopted candidate into a
`CompletedSessionCommand` in transit to `SessionControl::collect`. Both
collection paths route by the effective-time marker to `adopted` or
`withdraw`. A failed send, `cancel` or `collect` must retain any returned
packet and its box. The audio owner retains the live range throughout.
Joined teardown must inspect all these locations, including a candidate
awaiting withdrawal and a delayed or refused collection. Any live mapping
that depends on the transport anchor must use the audio-adopted anchor in that
interval, not control's old one. Identity continuity alone is not an audible
acceptance test: compiled catch-up currently fans parameter addresses across
voice instances, so it can alter a held live note's gate, magnitude, ramp or
modulation while its identity still resolves. Tests must cover those states
and live mint, release, reuse and retirement after a candidate snapshot, plus
release in the adoption quantum.

## Unresolved acceptance work

The candidate rule is insufficient until a concrete combined host answers
and tests all of the following. None is delegated to an implementer as an
unstated assumption.

Acceptance must explicitly amend ADR-0050 clauses 3, 5, 7, 8 and 9; ADR-0046
clauses 3 and 6 and ADR-0047 clauses 3 and 7 for the split source receipt,
audio acceptance and mixed-host release timing; ADR-0051
clauses 4, 5 and 6; ADR-0072's exclusive-owner mint and release limit;
ADR-0073's Session-only Stop assumption and terminal credit-exhaustion policy
for protected releases and, if changed, its inherited-bend failure and other
credit-exhaustion policies;
ADR-0074's reset custody;
ADR-0055's loop-activation refusal; and ADR-0065's exclusive loop owner.
ADR-0009's separate reset owner must be
preserved or explicitly amended. Host-profile and sound-core render
invariants, including HOST-INV-009, HOST-INV-022, SOUND-INV-017 and
SOUND-INV-018, also need explicit updates. ADR-0055's loop-activation
refusal and ADR-0065's no-live-ingress limit remain in force until this
combined-host loop acceptance gate is met and those limits are explicitly
amended.

1. **Source acceptance and release custody.** Define which receipt is only a
   reservation, where HOST-INV-009 accepts a live note, and how source queue,
   occurrence, identity, hold, result, protected note-off and any inherited-bend
   credits compose.
   Decide and test the exact redemption boundary, source/channel scope and
   late-edge outcome for individual release, key-up under sustain, pedal lift,
   Stop, Panic, reset/crossfade and stealing if admitted. In particular, a
   key-up under sustain cannot free a hold for a still-gated note, and a
   compiled-only activation must leave live holds untouched. State whether a
   crossfade ends obligations at adoption or after its tail; do not infer
   timing from old-owner retirement. A drop before acceptance must create no
   hold, and an accepted note's protected release cannot be silently lost;
   reset, Panic or an admitted steal needs a distinct terminal disposition.
   Decide whether inherited bend is precharged atomically or retains
   ADR-0073's terminal callback policy, and amend that policy if changed.
   If any terminal fault path remains in the combined host, acceptance must
   select whether it ends the whole stream, only the live partition or only
   the callback. It must define callback rollback, fault occurrence and time
   across callback partitions, compiled playback and candidate custody, loop
   custody, raw capture, accepted live holds and releases, later source
   operations, parameter versions and a fading old owner. This may be in this
   ADR or a separately accepted contract; a changed ADR-0073 host-termination
   policy needs an explicit amendment.

   Pedal-up, Stop/Panic and reset/crossfade need bounded, protected
   mass-release work, including a fading old owner. Every shortage and
   cancelled edge needs a named outcome and diagnostic category. Before a
   mixed host runs, joined teardown must also classify a still-held onset,
   an accepted pending release and an unexamined source packet; no one of
   those may disappear with the owner.
2. **Command order and generations.** Define one quantum-boundary order for
   Stop, Panic, Start, reset/crossfade, activation, loop entry/exit/wrap and
   source operations. Specify token publication, stale candidate rejection,
   stopped-state behaviour, already-adopted generation collection and
   source releases after a command. A normal release tail must be allowed
   while a gate and identity are ended. A prior drop or steal must keep its
   original terminal classification.
3. **Owner lifetime.** Prove that a compiled activation and loop handoff keep
   the exact live renderer, rows, holds and ramp. A whole-renderer reset may
   instead end them, but must reconcile receipt and result custody before
   old-owner retirement. Refuse identity-table rebuild while either producer
   range has outstanding obligations, as ADR-0047 clause 8 currently
   requires; resolve ADR-0048 before enabling any mixed-host rebuild path.
   Bound active and fading owners, return slots and
   final destruction off-thread. Define how a reset while stopped affects a
   prepared Start.
4. **Capacity and stealing.** Admit the worst-case same-quantum live and
   compiled release fanout, including a fading owner. Keep ADR-0058's victim
   search off audio or explicitly amend it with a bounded real-time proof.
   ADR-0065's loop-mode `StealingPolicy::None` remains in force until amended.
   Re-run the full producer partition on the common host; component evidence
   alone cannot qualify production values.

## Falsifiers and order of work

For the same admitted source and command prefix, single-callback, 64-frame, 256-frame
and irregular runs must have equal output, outcomes and raw take when every
compared callback succeeds. If any terminal fault path remains in the host,
the eventual fault contract must state whether fault occurrence and time are
invariant across partitions or may differ under a named rule, and must give
the rollback criterion before acceptance. A
compiled activation or loop transition that cuts, retriggers, re-pitches or
jumps the ramp of a held live note falsifies the ownership rule. A stale
candidate that reissues a spent generation, a hold stranded, redeemed twice
or freed at key-up while the note remains sustained, a release silently lost
without a named terminal disposition, an unreported receipt, or a callback allocation, lock or
final drop also blocks acceptance. The eventual redemption law must add
falsifiers for each release cause, source/channel scope, chosen boundary,
late-edge and cancelled-onset/release outcome, crossfade ending point, and
diagnostic category before
acceptance. It must also falsify the selected inherited-bend policy, each
source queue, occurrence, identity, hold, result, protected note-off and any
inherited-bend shortage outcome, and the per-quantum fanout and credit bound
for pedal-up, Stop, Panic and reset/crossfade mass release, including a fading
old owner. If any terminal fault path remains, its contract must add
falsifiers for selected scope, fault occurrence and time across partitions,
callback rollback, compiled output and candidate disposition, loop
generation, raw capture, accepted live holds/releases, fading-owner custody,
post-fault source operations and parameter-version disposition before
acceptance.
A test suite that never combines held live
notes, transport commands, loop transitions and saturated credits cannot
discharge these falsifiers.
A target-binding artifact that accepts two note producers whose playable
targets reach the same per-note row or one-instance node falsifies its first
slice, even if that node's control rows differ; a later shared-node law needs
separate acceptance. Within the first slice's declared shape — exactly one
compiled and one live producer, one bound live slot, `StealingPolicy::None`,
valid slots and note-target writers only — refusing disjoint multi-instance
node rows also fails its supported case. Substituting a plan, compiled stream or
live slot after binding falsifies that artifact. A refused binding that loses
the caller's plan or stream also fails its custody rule. A later mixed ingress
that accepts a cross-producer release group fails its producer binding. No mixed
owner, including the private rehearsal owner after rejoin, may accept Start,
Stop, Panic, activation or loop commands until that command's scope, hold
redemption and outcome law is explicitly amended and tested. The private
one-shot transition above is an internal rehearsal step, not an accepted
activation command or offer path.
Compiled-only catch-up must not write or reseed any live-owned instance-local
control slot, current ramp, next live write's ramp behavior or local
modulation history. A shared upstream source or live parameter-lane target
that can affect that instance follows its declared ordering law or refuses
admission. The combined Session charge and
same-quantum producer total must fit admission.

One candidate order is to build an isolated audio-owned live ingress with
source/result/held-cell credits and protected releases, then add
producer-to-target admission and scoped restoration, then integrate compiled
partition custody and test the full command matrix above. The target-binding
prerequisite leaves every mixed-plan, adopted-store, activation and loop-owner
live-ingress refusal in force. A later implementation may lift only the
refusal whose replacement contract and falsifiers it has proved and explicitly
amended; ADR-0072's exclusive owner remains unchanged until that boundary.
EVD-0024's component matrix remains
insufficient for production capacity; ADR-0022's hardware timing and physical
round-trip gates remain separate.

## Review and stopping rule

An independent design consultation compared a cross-thread live ledger with
an audio-owned live range and identified the latter as the candidate above.
Repeated independent semantic reads of a broader draft found unresolved
Stop/Start/reset, pedal, release and capacity contracts. Those rules were
removed from this proposal and are listed as acceptance work, not decisions.

A false code premise, conflicting ownership, a release without guaranteed
credit, an ambiguous ordering rule, or real-time allocation, lock or final
drop blocks acceptance. Optional implementation detail does not.
