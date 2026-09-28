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
order alone cannot pair its releases. The concrete source producer now rejects
that regression before queue custody. Another consumer must establish the
same pre-receipt order law or prove a shared execution order across boundaries.

The non-shipping concrete bridge's `PreparedAttempt` passes the same clocks to
the source producer, raw input and audition. The producer checks mapped time,
decreasing ticks, arrival and frontier order before pushing into its ring; it
records the original and reason in a one-shot source failure cell, requests
host halt and returns `Invalid(original, reason)` on failure. The source inbox
passes that failure to the raw owner before servicing its next queued prefix.
The managed two-source service records failures already published at the start
of each call before it drains either queue. The Linux driver now services both
queues while producer workers run so a full-ring retry can make progress, then
joins the workers before final service. If a failure arrives during service
after the prepass, its primary reason may be `PeerInterrupted`; the separate
pre-ring cell still retains its actual cause and quality evidence. For
`SourceQueueFull` and source identity exhaustion, the raw owner requires a
mappable observation and rejects a message arrival before nominal time.
Joined attribution retains an exact late-message stamp when the recorder's
source frontier has already advanced beyond that nominal time. The pre-ring
slot distinguishes source serial exhaustion from raw input-ID exhaustion;
its `IdentityExhausted` reason does not distinguish the source attempt serial
from the source queue serial. Raw input-ID exhaustion keeps its existing
quality policy.
The raw owner checks clock and arrival claims; source order remains a producer
attestation because a queued prefix may not have reached raw admission. It
retains the original in a separate one-shot pre-ring cell. Joined reunion
attributes its exact-late or uncertain quality even when another source's
halt has already marked this source `PeerInterrupted`. That primary
discontinuity remains first-wins. If raw failure recording rejects the claim,
the inbox emits one refusal callback with that recording error, while the
producer's `Invalid` result retains its original reason. It still drains the
queued prefix. The test-only value path still rejects a different observation
before its pending full-ring retry; that comparison cannot distinguish two
equal-valued occurrences. The Linux driver now uses an owned attempt path.
Each valid first submission receives a typed generation and checked attempt
serial. Success returns that attempt ID with a distinct source queue ID. On a full
ring, the producer retains the original and attempt ID while returning a
non-cloneable retry token with the same values. A matching retry keeps the
attempt ID, and only its successful push spends the next queue ID. An
otherwise valid fresh submission while either retry path is pending is a
terminal `Order` refusal, including an equal-valued submission. A foreign or stale
token is returned without changing either source; token matching precedes the
halt check. If halt is already requested, a matching token remains pending
and is returned as `Halted`. Joined retirement reports the producer-owned
original even if the token was dropped during unwinding. Closing the source refuses while that
attempt remains pending. Retirement records `SourceQueueFull` as that source's
pre-ring fault when its one-shot failure cell is still empty, including after a
peer halt; an earlier source fault and the primary discontinuity remain
first-wins. The driver services both source rings while the workers retry
until a bounded deadline, then terminally retires and reports the original
and unexamined suffix if the ring is still full. It also retires and reports
any pending attempt after its workers join, before asking for a device-loss
halt. These reports are the disposition for an original that never obtained
a queue ID. Attempt-serial exhaustion is checked
before queue-serial exhaustion and is terminal before ring custody. The one
producer-owned pending attempt is charged in producer storage; it is outside
the audition quota and supplies no protected release credit.

The concrete falsifiers require distinct attempt and queue IDs for two
equal-valued onsets, a stable token through repeated full retries, concurrent
merger service that lets a blocked retry enter the ring, no queue ID before
push, no exchange with a fresh equal-valued or foreign attempt, joined recovery
after token loss, and exact source attribution for terminal
full-ring retirement. The inbox reports the queue ID with every popped
result, including raw refusals. This proves source-ring custody and retry
identity, not a same-key FIFO ledger or a protected release path.
Raw input repeats its own admission checks. The bridge commits an audition
packet only after raw admission. A pre-ring regression or future-arrival
refusal leaves raw and audition admission, serials and credit unchanged for
that refused observation.
It requests terminal halt: an earlier queued prefix may already have reached
raw admission or may return as identified source refusals when the merger next
services the ring. The driver reports the rejected original and unexamined
suffix, services queued prefixes while workers run and once more after join;
the raw owner retains the rejected observation's quality diagnostic. No
nonterminal rollback is claimed.

Within this bridge, accepted messages from one source have nondecreasing
mapped times, and equal times retain serial order in the live renderer. This
check now precedes source-ring custody. It establishes the order prerequisite
for the producer's own accepted prefix, with the terminal halt disposition
above. It does not establish capacity, same-key FIFO tombstones or a protected
release lane.
The bridge distinguishes a refused original returned without a raw ID from an
audition fault carrying an already accepted raw input ID. A regression or
future-arrival refusal and an accepted-ID audition fault request a host halt.
Renderer-held capacity and the release-credit equation remain open.
The concrete bridge now also reports audition-packet custody separately from
that raw result. `NotQueued` covers pre-ring and pre-raw refusals, frontiers,
an absent audition owner and a failed audition commit after raw acceptance.
`Queued(id)` begins only after the audition queue push succeeds and survives
a later raw-trace attachment fault. Source-inbox service carries that stage
beside its source queue ID. The Linux driver reports queued audition IDs.
Neither stage says that the renderer executed the packet or that a protected
release has credit.
The terminal host policy remains; the source/channel/key FIFO ledger is unbuilt.
The source producer validates clock mapping, arrival, and local order before
publishing to its ring. The inbox forwards that ring in FIFO order with the
same prepared clock as raw input. When a source-queued message is refused by
audition preflight for `Full` or `IdentityExhausted`, the host records a
distinct post-source-ring, pre-raw terminal fault with the original observation.
The raw owner validates the mapped time and arrival and retains this fault
separately from a producer's pre-source-ring fault and its first-wins primary
discontinuity. It attributes exact late quality at reunion. Recording the
post-ring fault first synchronizes any requested halt, so an input already
interrupted by that halt keeps `PeerInterrupted` as primary. A second post-ring
report refuses even when its observation has equal bytes. Equal values at
different custody stages can belong to different source attempts; each is
evaluated for refused-input quality.
Subsequent packets observed after halt retain their own source queue IDs and
return `State` without claiming another fault.
The source report and Linux driver keep the audition preflight reason and any
fault-recording error in separate fields. If registration of a producer's
pre-source-ring fault fails, the report likewise retains the producer's
original and reason with the raw-owner error separately and no source queue ID.
A stale audition source binding is checked before audition capacity. It is a
host configuration refusal and does not claim this source-fault cell. A producer
publishes its pre-source-ring failure before requesting a halt. The pre-ring
recorder retains the original without synchronizing the halt. The first
recorded discontinuity remains
primary: it may be the producer's reason, a prior input fault, or
`PeerInterrupted` if the halt was already synchronized. Reunion evaluates the
pre-ring fault for quality separately when it is not primary. Raw admission
`Full` and `IdentityExhausted` keep their existing quality policy. The new
exact late attribution applies to audition preflight refusals. This retains
terminal attribution without granting shared admission credit or a nonterminal
refused-onset tombstone.

The concrete bridge's joined finish, on either recovery or ordered Stop,
interrupts the live renderer before it transfers a packet previously refused
by renderer admission and the queued suffix into that closed renderer. Those
packets receive identified `Cancelled` outcomes, and reunion resolves retained
raw capture annotations without another callback. If the closed renderer still
rejects a packet, finish retains it and reunion refuses. Joined recovery still
cancels host-held commands from the oldest refused audio packet through the
ring to the newest pending packet. When core halt succeeds and all host-held
packets cancel, it then collects outcomes for commands already admitted to the
core. A failed core halt or command cancellation leaves those core completions
for a retry. It reports audition, halt and the first command fault together
when they occur in one attempt.
A failed command cancellation retains that packet and its ordered suffix.
This path does not invent an outcome for a broken identity or capacity
invariant or reserve a release lane.

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

A V2-local source-ring model now tests a narrower candidate release reserve.
Let `S` be one source ring's cell count, `q` its occupied cells and `R_s` its
ring-custodied onsets whose matching releases have not entered that ring.
Preserve `q + R_s <= S`. An onset acquires its ring cell and future release
cell together: it requires `q + R_s <= S - 2` and changes `(q, R_s)` to
`(q + 1, R_s + 1)`.
An ordinary packet requires `q + R_s <= S - 1`, no pending source onset or
release attempt, and no terminal halt. Once all earlier source attempts have
entered the ring or retired, a release for the oldest unreleased
source/channel/key occurrence that entered the ring changes `(q, R_s)` to
`(q + 1, R_s - 1)` and preserves the sum. Since
`R_s > 0` implies `q < S`, that release can enter the ring after ordinary
packets fill every unreserved cell. A release arriving behind a pending onset
retry stays in one source-local pending cell and returns `Blocked(token)`.
The original input remains in that cell. A new onset offered while either
pending cell owns source order gets terminal `Order(original)`. A new release
gets `Order(original)` when the release cell is occupied; when only the onset
retry is pending, it enters that cell and gets `Blocked(token)`. A terminal
fault requests model host halt. Later onset or release offers from either
source get `Halted(original)`. Matching retries after the halt receive
`Halted(token)`; stale or foreign tokens receive `Stale(token)`. The pending
onset cannot retire after the halt. Both pending originals remain held for
joined teardown, which this model does not implement.
Only the exact release token may retry after the earlier onset enters or
retires. A stray pending release then gets a final `Unmatched(original)` and
clears the pending cell. An immediately offered stray release instead returns
`Unmatched(original)` as an offer error. If the blocked release matches an
earlier ring-custodied onset, retiring the pending onset lets that release
enter. Retirement keeps the pending attempt's
source/channel/key tombstone and ledger cell. It returns the tracker, ingress
and result credits. The matching release consumes that tombstone as
`Retired(id)` without entering the ring or touching a later
same-key onset. The model can continue after a retry retires; the concrete
driver instead halts on retirement. A tombstone without a release remains held
for joined teardown; the model does not yet implement that teardown or a
host-visible final result. Ring service decreases `q`; it does not redeem
`R_s`. A queued release redeems only its own reservation, and service cannot
skip earlier packets.
The model checks one and two held onsets, same-key FIFO, ordinary saturation,
blocked release ownership, wrong-token rejection, a final stray release and
retry whose missing ring headroom can be restored by draining `q`.
If `R_s + 2 > S`, draining this ring cannot admit another onset, so this shortage
cannot be an ordered retry. The model returns `NoCredit(original, reason)`
without taking shared credit or ring custody, and requests host halt. One
host-wide fault cell retains the source, original and exact reason for later
attribution. It is the authoritative recovery copy; the returned `Copy` value
is diagnostic and must not be replayed or recorded independently. A future
owner must enforce that custody rule. The cell is not an accepted occurrence
or a replacement for joined teardown. It preserves the first fault while
later offers return `Halted`. The fault cell does not retain their originals:
the caller still owns each `Halted(original)` and a combined host must classify
that unexamined suffix at teardown. A source-reserve shortage and source-order
`Order` have source-local causes. Tracker, ingress, result or ledger shortage
and either model-wide identity counter have shared-state causes. Cause origin
does not change the terminal scope. ADR-0073 establishes a host-terminal policy
for its audition operation credit and tracker exhaustion. Extending that scope
to this model's other shares, source reserve, identity and `Order` is a
conservative rehearsal choice, not an accepted production decision. The model
does not silence real output. Later releases cannot enter either source ring.
The onset preflight is read-only so settlement checks can probe shared credit
without issuing a terminally refused packet. For valid onsets, its check order
is host halt, pending source order, nondrainable source reserve, tracker,
ingress, result, ledger, occurrence identity, then drainable ring headroom.
Tests preserve the cross-source credit-reuse falsifier through that preflight.
Actual terminal examples cover source reserve, tracker, both identity counters
and `Order`; preflight also covers ingress, result and ledger shortage.
Release-attempt identity exhaustion behind an onset retry also retains its
original and reason in the terminal cell without a shared charge or release
pending cell. The earlier charged retry remains held.
The ordinary-packet fixture checks capacity without issuing a refused source
attempt; it has no ordinary-packet retry or terminal-result contract. The
model's host halt has no joined teardown for the pending onset, pending release,
earlier charged attempts, held releases or queued prefixes. It does not silence
real output, settle raw attribution or supply a combined host outcome. Those
source and teardown laws remain open for the combined host.
Neither the local reservation nor the retirement tombstone grants a protected
release claim for the host. The model also lacks raw and audition consumers,
frontiers, cross-thread handoff and a combined outcome. The source-ring
equation is a local prerequisite, not an accepted end-to-end host law.

The concrete source ring is also the only path into raw capture. An audition
packet refused before that ring cannot still be captured through the current
path. This bridge configures 32 raw cells per source. The raw owner reserves
one cell for frontiers and invalidates the host when its ordinary cells fill.
Its receipt collector is separate from audition result collection. A release
reserved only in audition can therefore fail before reaching audio under
raw-capture starvation; a new law must include capture credit or state the
resulting terminal disposition.
It must distinguish an audition source receipt from raw capture's
`InputReceipt` and identify where each is settled.

The raw owner now enforces a local protected-capacity gate, requiring `N >= 5`
to accept an onset while its initial frontier remains uncollected. Let
`N` be the source's configured raw-cell count (`N = 32` in the concrete
example host), `h` its occupied cells including the initial frontier
and uncollected receipts, and `R` its outstanding release reservations. It
counts `h` from the actual fixed slots and tracks `R` in a preallocated
same-source, same-channel, same-key ledger. A release redeems the oldest
matching accepted onset; a release without a match is ordinary input.

While `R > 0`, the raw owner preserves `h + R <= N - 1`. An onset's raw cell
and its future release reservation must be preflighted as one charge before
raw admission: `(h, R)` becomes `(h + 1, R + 1)`, requiring
`h + R <= N - 4` beforehand and `h + R <= N - 2` afterward. No raw-accepted
onset may be left without that reservation. With `R > 0`, an ordinary message
must leave `h + R <= N - 2`, while a frontier may leave `h + R <= N - 1`.
Redeeming one reservation changes `(h, R)` to `(h + 1, R - 1)` and preserves
the sum, so it passes the raw capacity check. With `R = 0`, an ordinary
non-onset message still follows the raw owner's `h <= N - 2` capacity check.
Clock, ordering, identity and state checks can still refuse an observation
before or after that capacity check. Every raw offer refusal remains terminal
and retains its first discontinuity. With no held release, ordinary storage
exhaustion remains `Full`; a reservation shortage is `ProtectedCapacity`.
Closing admission after either a complete or interrupted take clears the
local release claims while the take retains its accepted prefix. This local
owner does not supply the host's combined refusal disposition.
The raw owner now also exposes a read-only `preflight_observation` that runs
the same clock, order, capacity and identity plan as immediate admission. It
does not quiesce the source on refusal or reserve credit. A collection may
free a cell between the check and offer; another offer, frontier or lifecycle
change can invalidate a ready result. The source-ring charge must still cover
in-flight packets before this local check can support a host-wide claim.

These local raw safety gates are not an accepted end-to-end release law:
source-ring admission, refused-onset FIFO, audition credit, renderer-held cells
and bounded service remain open. Actual-owner tests cover the five- and
six-cell boundaries, repeated-key FIFO, and terminal refusal. A protected
host must falsify its shared credit against actual raw receipts and ensure an
ordinary capacity refusal stops before raw admission, so it does not quiesce
the source.

An end-to-end candidate that preclaims only source-ring, raw and audition
credit is falsified by the recorder's shared tracker. The concrete bridge
prepares eight tracked-input-note cells for both sources. A ninth held onset
can pass those three checks, reach raw admission and then fail pairing with
`TrackerFull`; a non-capture-eligible onset can fail earlier with
`TrackerReserved` while a capture reserve remains. Both faults invalidate the
recorder source. Protected onset admission must account for the tracker and
its unspent capture reserve before the source producer reports acceptance,
or define a terminal disposition that identifies the already accepted raw
and renderer obligations. Both sources share this tracker, so separate
per-source shadows cannot reserve its cells. The producer cannot inspect
capture eligibility or the unspent reserve at send time; a future host must
use one linearized shared charge with a proved worst-case reserve bound, or
move the admission decision to an owner that can make the same guarantee.

The audition pool is also shared by both sources. A future protected host
using separate per-source checks against its 128-packet ceiling could
simultaneously admit more than 128.
Any shared charge must linearize the combined onset and release reservation,
and redemption must not expose a temporarily lower total to another producer.

Linearizing admission at the merger does not itself justify reclaiming ingress
hold or tracker credit when that merger forwards a release. Those consumers
apply events in mapped-time order across sources, which can differ from
source-ring service order. In an ingress-only fixture using the example live
graph, four source-A onsets can occupy its four ingress holds at time 10.
After rendering those onsets, let `C` be the next quantum's clock. Source A
queues one release at `C + 20`, then source B queues an onset at `C + 10`. If
the merger frees a shadow ingress hold when it forwards A's release, it can
give B a positive model onset receipt, although the renderer still holds all
four note identities at `C + 10` and refuses B's onset. The ingress-only
fixture proves this consumer ordering and refusal; it has no merger credit
ledger or positive model receipt. A combined-host rehearsal must test those.
The renderer's 128 held-occurrence cells are a separate resource; this example
saturates the plan's four ingress holds, not that table. A credit can become
reusable only after its particular consumer has processed the release, or
after a separately proved time-order fence makes earlier reuse safe. Neither
source-ring removal nor raw admission proves that. The cross-source progress
premise for recycling through settlement is stated below.

The recorder has the same time-order hazard when its tracker is the limiting
resource. The concrete host's four ingress holds and four live note identities
stop it from safely admitting eight simultaneous onsets into its eight-cell
tracker. A recorder-only fixture with eight tracker cells and no active capture
now isolates the tracker: eight tracked source-A onsets leave source B's
earlier-time onset at `TrackerFull` before A's later-time release frees a cell.
With capture armed, a non-capture-eligible onset can instead hit
`TrackerReserved` before the tracker fills. A separate recorder-only fixture
holds six unselected A onsets against an eight-cell tracker with two cells
reserved for the armed take. Unselected B's earlier-time onset receives
`TrackerReserved` before A's later-time release; two selected onsets still
record afterward. With exact publication stamps, the reverse call order
rejects B's earlier-time onset as `PastBoundary` after A's later-time release.
That check uses published time; a host that delivers an earlier nominal event
with a later published time still needs ordered credit accounting. None of
these fixtures supplies a model receipt or a host ledger.
Before a positive model onset receipt, the tracker charge must cover the
recorder's unspent capture reserve in the worst case: arm or stop boundaries
may change capture eligibility before the onset reaches pairing. Separate
ingress-only and recorder-only rehearsals now establish the two consumer
refusals at their respective limits. The ingress fixture exposes a source-ring
service order opposite to mapped-time execution; the tracker-full and reserve
fixtures test the earlier onset before the later release in consumer order.
The combined host must prove the same admission rule across both consumers.
Any positive model receipt that later becomes `IngressRefused`, `TrackerFull`,
`TrackerReserved` or `PastBoundary` falsifies that rule.

`Retry` cannot stand for every credit shortage. With the initial raw frontier
still uncollected, fourteen onset/release pairs without a later frontier
occupy 29 of 32 raw cells. Retrying the next onset until raw receipts free
cells blocks the frontier behind that onset in the same source FIFO; those
receipts need that frontier. Similarly, if one source holds all eight tracker
cells, retrying its ninth onset for tracker credit blocks its releases behind
it in the same source FIFO. Those releases would free the tracker cells.
Only a shortage resolved by draining the source ring may use ordered `Retry`.
A pre-receipt onset refusal needs a final observable outcome and same-key
FIFO tombstone, so its release cannot end an accepted onset. A protected
release redeems only its own matched ledger reservation. A stray release
cannot spend another key's credit. Ledger exhaustion must fail closed.
Velocity-zero note-on must follow the same key-release classification as the
renderer and recorder.

Frontiers have a distinct liveness problem. At `h + R = N - 2`, one frontier
may consume the remaining raw frontier cell; retrying a second frontier ahead
of a protected release can then hold that release behind it. Coalescing is
possible only while preserving every intervening message's order and the
raw owner's strictly increasing frontier rule; no such mechanism exists yet.
Another source withholding its merger frontier does not block an already
reserved release from reaching audition. It does block raw receipts and
audition settlement, so credits tied to those outcomes cannot be recycled.
A credit-recycling liveness claim must state the other source's progress
premise or a terminal timeout and test the cross-source stall explicitly.
The raw-occupancy shadow also starts with one occupied cell: the initial
frontier bypasses the source ring but remains in the raw owner until its
receipt is collected.

A source-side key ledger needs one FIFO for both receipted and refused
onsets. If the first same-key onset is receipted, the second refused and two
releases follow, a refused-only tombstone queue consumes the first release
and forwards the second as if it belonged to the first onset. The release is
delayed and misattributed; without the second release the note remains held.
The source producer now checks nominal mapping before queue custody. A failure
after that queue receipt still needs explicit custody redemption. A release
staged before a later callback failure may already have spent its ingress
hold and freed its held cell even though its outcome becomes `Cancelled`.
Joined teardown must classify the actual note and release state, not infer it
from that outcome alone. Add these counterexamples to any protected-release
rehearsal before claiming it passes.

A source ledger cannot require the host-reported, settled audition outcome
before forwarding a release. The onset's raw receipt needs a merge frontier:
the minimum frontier over all sources must pass its time. That receipt settles
the audition ID before the host reports its outcome. If the needed frontier
follows the held release in the same source FIFO, neither can progress. A
frontier on another source can create the same cycle; both forms must falsify
any proposed wait rule. A direct renderer outcome has different custody and
does not discharge this host-reported-outcome falsifier.

A successful source-ring push grants queue custody, not a model receipt. The
merger may still return `InputOfferError::Refused` without an `InputEventId`;
audition preflight refusals, including a stale message, and raw admission
failures request host halt. Other pre-admission `Stale` or `State` refusals
can return without a new halt. Their source and occurrence disposition remains
to be specified. `SourceProducer::send` returns `Retry(original)` for a full
ring, `Halted(original)` when its halt check observes shutdown, and
`Invalid(original, reason)` for a pre-ring time refusal that requests halt.
The Linux driver stops its worker at any error and reports the unsent suffix in
source order. A halt that races after the check may leave a queued observation
or a `Retry` result; retry must check halt again. A future nonterminal capacity
refusal with a FIFO tombstone would be a new contract and must distinguish
`Retry` from `Halted`. An onset that reaches the renderer but receives
`Refused(IngressRefused)` still owns a renderer tombstone. Its release must
reach the renderer to consume it. The
note recorder's tracker `TrackerReserved` and `TrackerFull` refusals invalidate
the recorder source.
The raw owner then receives a refused source completion, fails with `Delivery`,
and requests host halt. Its audition packet may already have reached the
renderer, so joined teardown must classify that voice or tombstone as well
as the recorder refusal.

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

### First mixed-ingress rehearsal boundary

The next internal rehearsal may enqueue a live onset only through an exclusive
`&mut MixedOneShotAudio` operation. It has no source-ring, raw-input, recorder,
result-channel or key-matching bridge. Returning a `NoteIdentity` proves only
private ingress acceptance, not execution or a source, model or cross-thread
receipt. The caller must present that exact identity for release; a refused
onset returns no identity. A foreign or already released identity gets
`OrphanRelease`, not a drop. Bend, sustain, transport, capture, mass release and
stealing remain refused. Audio-side minting is test-only rehearsal work, not an
amendment to ADR-0050's off-thread minter rule or ADR-0072's exclusive-owner
exception. Its note offers require
`#[cfg(all(test, feature = "simulated-ingress"))]`; no production mixed offer
is exposed by this step.

The rehearsal uses the existing registered performance-event ingress queue,
not a second live source store or another drop licence. The test-only mixed
constructor binds that store to the plan, epoch, table, live producer and span;
its note offers use only the audio owner's `LiveRangeMinter`. The public
`PerformanceIngress::prepare` keeps its mixed-plan refusal. An onset checks
the registered ingress slot and release hold before minting its range-scoped
identity last, so their acquisition is atomic. Exhausting slot, hold or
identity uses that queue's existing counted `Dropped` outcome. An onset stamped
before the last accepted release refuses as `NonMonotoneStamp` before taking a
slot, hold or identity. That monotone-stamp check is the private store's
time-order fence, separate from possible index retirement at the generation
ceiling. In this private single-producer store, the exact release converts its
hold into a queue entry and frees the minter index when offered, as the
existing ingress does. A second release of that identity is therefore
`OrphanRelease`. Its later onsets must follow the
release's stamp and FIFO order, including across adoption. This release-at-offer
timing is a test-only rehearsal, not authority to extend ADR-0072's exception
to a production mixed owner; that needs explicit amendments to ADR-0046,
ADR-0047, ADR-0050 and ADR-0072. A later source bridge cannot recycle
cross-source tracker or hold credit merely because it forwarded a release.
Source/channel/key FIFO tombstones and outcome cells remain bridge work.

Before the private owner can arm with this store, it must derive checked bounds
from the bound plan and profile for worst-case Live and Release publication,
Session boundary release and scoped restoration, Compiled suffix, total event
cap, payload size, timed-control fanout and scratch, and seed storage. Missing
or excessive bounds refuse arm without consuming either owner. The open bounds
in NOW.md are work to close, not admission already granted here. Teardown
without another callback must classify queued ingress entries, minted live
identities, final ingress counters and accepted pending releases, including a
release whose hold was spent before a callback fault.

The rehearsal fails if a release of an accepted identity is lost to queue
pressure, a same-time release and reused-index onset execute in reverse FIFO
order, a retired index is counted as free for a later onset, or late
backlog exceeds `live_event_share`, `release_event_share` or the combined
quantum cap at adoption. It must refuse compiled-range identities and match
live audio and rows to a live-only reference with onset and release on both
sides of adoption. It must classify pending, faulted and resumed owners on
teardown. Callback-partition
equality is asserted only for compared callbacks that all succeed; fault
occurrence and rollback still need a separate rule.

One candidate order is to build this isolated rehearsal, then integrate the
source-ring and recorder with one shared charge and a same-key FIFO ledger,
then test the full command and loop matrix above. Existing target binding and
scoped restoration are prerequisites already present in the private owner. The
target-binding prerequisite leaves every mixed-plan, adopted-store, activation
and loop-owner live-ingress refusal in force. A later implementation may lift
only a refusal whose replacement contract and falsifiers it has proved and
explicitly amended. ADR-0072's exclusive owner remains unchanged until that
boundary.

The immediate source-bridge rehearsal must remain inside `synth_engine_v2`.
The concrete two-source rings and audition host live in the `pertylizer`
example, while the mixed owner is crate-private and its note offers require
`#[cfg(all(test, feature = "simulated-ingress"))]`. Compiling V2 as that
example's dependency does not enable V2's test-only operations. A V2-local
fixture may model the rings at their concrete capacity, but passing it proves
only the modeled admission and ordering law. It does not qualify the example's
concurrent producer, ring handoff or `AuditionId` result path. Connecting those
concrete owners requires a separate, reviewable API and custody contract
before any mixed offer is exposed outside the private rehearsal.

The modeled ledger must keep one source/channel/key FIFO containing both
accepted and refused onset occurrences. Each entry needs separate raw-recorder
and mixed-ingress dispositions: a raw-accepted, ingress-refused onset still
needs its matching release delivered to the recorder, while an ingress-accepted,
raw-refused onset still owns an ingress identity or tombstone. A release may
consume only the oldest entry for its own key, including when it is encoded as
a velocity-zero note-on. Source-ring acceptance is queue custody, not a
positive model receipt. The fixture must distinguish that queue result, each
consumer's admission, and the eventual combined outcome. A positive combined
outcome cannot precede either consumer's settlement; a release cannot wait for
that outcome because the required frontier may follow the release in the same
source queue. A partial consumer admission therefore stays explicitly owned
until its release or joined teardown, even when no positive combined outcome
can be reported.

Before this modeled bridge claims protected release, its shared charge must
cover both sources' tracker cells and capture reserve, raw occupancy and
release reservation per source, ingress hold and identity, and retained
outcomes. Charge acquisition and redemption must have one linearization point
in the fixture. A ring-full retry retains its original charge and source
attempt; terminal retirement reports both. Neither source-ring removal nor
forwarding a release returns consumer credit. Reuse requires that consumer's
settled release or a proved time-order fence. Cross-source earlier mapped-time
onsets following an already forwarded later release, repeated frontiers at
`h + R = N - 2`, and another source withholding its frontier remain explicit
falsifiers; the fixture must give each a source-order-preserving disposition
before claiming bounded release service. The current concrete raw owner has
no frontier coalescing, and the private mixed result ring is neither source
associated nor connected to a shared ledger, so no such claim follows from
the isolated ingress rehearsal.

Build the model in stages: first test the shared ledger and retry/retirement
with bounded fake consumers; then attach the real raw owner and recorder; then
attach the private mixed owner and classify a fault in a later quantum beside
the recorder's independent receipts. Tracker-full and capture-reserve tests
need fixture capacities that can actually make the tracker the limiting
consumer; the existing two-hold mixed plan and eight-cell tracker cannot do
so by themselves. At every stage, a positive combined outcome followed by
`TrackerFull`, `TrackerReserved`, `PastBoundary` or `IngressRefused`, a release
attributed to a different occurrence, or an unclassified partially admitted
note blocks acceptance. This staged fixture does not lift a production
mixed-producer refusal.
The first local raw/recorder probe now drains three modeled onsets across two
sources, including two repeated-key onsets on one source, and their queued
releases. It offers each dequeued payload to a real `SimulatedNoteInput` owner
and independently to `SimulatedNoteRecorder`. It checks model source and
payload custody, raw input ID ownership, and the recorder's repeated-key
occurrence pairing. The model retains raw/tracker, result and ledger credit;
none is returned merely because the release was forwarded. The probe does not
join the raw owner to the recorder, collect raw `InputReceipt`s, exercise the
concrete ring handoff, or test recorder tracker pressure or mixed ingress.
Its observed pairing is a local prerequisite, not an end-to-end identity or
positive combined-outcome claim.
The actual serial input/capture owner now carries `matched_onset` in each raw
release's `InputReceipt`, naming the oldest held raw `InputEventId` redeemed
at admission. A serial two-source test joins this link to the recorder's
`PerformedOccurrenceId` in delivered publication receipts, with two repeated
keys on one source and a same-key note on another. The test stays within its
two-held-note recorder profile and checks complete capture. Separate
standalone and serial-session device-loss fixtures check that cancelled
releases retain the raw link. The delivered pairs in this serial fixture
establish raw/recorder identity correlation; a cancelled release can name a
raw onset without any recorder occurrence. This does not give the
modeled source ring an actual raw receipt, a shared charge or a mixed-ingress
outcome.
The next two-source model probe drains its retained onset and release packets
into an actual serial `InputCaptureSession`. It checks three distinct recorder
occurrences, including repeated keys on one source, against delivered raw
`InputReceipt.matched_onset` links. The model binds each raw onset ID. Its
delivered-settlement operation requires both delivered receipts to link the
release to its onset and one recorder occurrence before returning tracker
credit. A cross-source release swap or a cancelled release is refused with tracker
credit still charged. Fixed-generation per-source serial watermarks refuse
duplicate raw-onset binding even after the original entry is reaped; this
fixture does not model reconnect. Its fake-consumer settlement cannot redeem
an entry bound to a raw ID.
The model already returned ingress credit for its modeled refusal at source
service, while result and ledger credits stay charged. The serial owner still
receives direct offers from the modeled queue rather than a concrete source-ring
handoff; there is no concrete shared raw-capacity charge, actual mixed-ingress result or
combined host outcome.
The next local probe forwards three modeled onset/release occurrences to the
private test-only mixed owner beside an actual serial raw/recorder owner. The
queue's key and velocity reach mixed ingress, while the model retains source
to identity association; the private owner has no source field. Its two-hold
limit accepts two onset identities and refuses the third with `Dropped(Hold)`
after raw admission. The model records that refusal, and the release still
reaches the recorder. A fault injected after ingress charge in the next audio
quantum retains the exact two onset and two release edges with their times,
identities and redemption flags in mixed teardown classification. Independent
serial capture delivers all three raw/recorder pairs and seals complete. Model
ingress credits for the two accepted identities stay charged alongside result
and ledger cells. The private owner has no per-occurrence outcome after the
fault, and the fixture has no concrete shared raw-capacity charge, concrete
source-ring handoff, joined fault outcome or production mixed offer.
The actual raw owner now reports a typed read-only snapshot of occupied cells,
matched-release reservations and configured capacity. The two-source model
charges a pending onset and its future raw release before queue custody. Queued
ordinary packets also claim pending raw cells. Raw offers convert pending
charges to occupied cells; frontiers and collected receipts update the shadow.
Its pressure equals the real raw owner's snapshot after each batch of onset,
release, frontier and receipt handoffs in both serial fixtures, including the
mixed hold refusal and later fault. With eight raw cells, the model and the raw
owner's admission preflight both refuse an onset after retained onsets or
ordinary packets exhaust protected capacity. A retired source-ring retry
returns its pending modeled raw charge. A discrepancy at a comparison or
preflight is a falsifier for this running-owner shadow accounting; raw owner
faults are outside the model. The snapshot grants no future reservation,
so concurrent source offers and the concrete source-ring handoff still need a
shared atomic admission rule before this can qualify a combined host.
The concrete example `SourceInbox` path now carries two serial producers'
`SourceQueueId`s through actual raw admission. A fixture matches each returned
`InputEventId` to its delivered raw receipt and recorder publication, including
repeated same-key onsets and FIFO releases on one source beside the other
source's same key. This is an identity and payload handoff check for the
example's existing ring and raw owner. It has no atomic shared capacity charge,
concurrent producer proof, mixed-ingress offer or joined outcome.
The concrete source packet also retains the nominal engine `SampleTime` checked
before queue custody, including through an owned full-ring retry. Source and
managed-host service now report a paired `SourceQueueStamp` with each actual raw
offer result. It contains the queued ID and checked time; a pre-ring failure
has no stamp. The Linux driver displays the stamp, and the two-source fixture
checks the 1:1 and 2:1 source-clock mappings against retained raw receipts.
The concrete callback now pairs that stamp with the exact queued
`InputObservation` in `SourceHandoff` beside the actual raw result. A two-source
fixture compares the handoff original with each delivered raw receipt;
full-ring owned retry and raw-refusal fixtures retain the same original.
Pre-ring faults have no queued handoff. This is payload custody for a future
mixed lane, not mixed staging, a shared charge or a combined outcome.

The raw owner now offers a read-only admission-time `matched_onset(id)` lookup
for a retained raw cell. It distinguishes an unmatched release from an absent
ID and refuses another generation. The concrete source-inbox fixture reads
the oldest same-key link for two queued releases before any raw receipt is
collected. This removes the need to wait for receipt publication merely to
learn raw pairing. The lookup must happen while the core raw cell remains;
a host may retain a receipt after that cell is reaped. It does not identify a
mixed occurrence, reserve shared capacity or settle either consumer.

Raw service keeps independent fixed source prefixes. A separate design read
found that applying the mixed-time selector to this raw service would hold a
source's far-ahead frontier behind the other source's earlier messages. The
serial capture owner needs those frontiers to publish and recycle its bounded
cells; `source_cells_recycle_beyond_64_with_audition_and_recover_without_a_last_callback`
is the concrete regression gate for that progress. This is an inference from
the raw frontier law and that fixture, not a qualified mixed service path. A
future mixed lane needs its own bounded custody, raw/ingress/result association
and time-order wait policy. It cannot hold raw capture hostage to a silent
peer or charge that peer's silence as the blocked producer's `SourceQueueFull`.
An independent design read rejected the narrower idea that a merger-thread
shared counter alone could linearize combined mixed admission. The V2-local
`merger_credit_alone_cannot_admit_out_of_order_mixed_ingress` test provides a
falsifier: source A's onset at sample 150 and source B's onset at 140 both enter
their raw owners with model credits available, yet direct mixed offers in
source-service order refuse B as `NonMonotoneStamp`. The mixed offer runs on the
audio half, whose monotone stamp and forward-horizon checks are separate from
merger credit. A combined positive onset outcome therefore needs mapped-time
mixed-offer order and an exact audio-side ingress result returned to the source
ledger. The private mixed owner now has a test-only, preallocated command and
result ring across its control/audio split. A command carries its exact onset
or release request and a local command ID; explicit audio service returns the
actual ingress identity or refusal with that request. Control reserves a result
slot until collection and teardown refuses while any command or result is
outstanding. The two-onset/two-release cross-thread test measures no allocation
in audio service; separate tests falsify lost original requests when the
control-side result reservation is exhausted, silent teardown, and acceptance
of the out-of-order timestamp.
The ring depth follows the registered ingress queue, but these private test
rings have no production byte-budget charge. The request's opaque origin ID
is modeled source-occurrence correlation, not a concrete source-ring identity
or custody contract. There is still no mapped-time ordering across source
rings, callback service, combined raw/recorder outcome, or production mixed
ingress entitlement.
The V2-local two-source model builds each onset request from its retained
source input, supplied mapped stamp and model-issued occurrence ID. One model
operation submits that request and stores the returned command ID beside the
occurrence, so an unrelated ID/request pair cannot be claimed later. Result
application rejects a swapped command, changed request, release-shaped
outcome and replay without changing a pending entry. The first result binds
its live identity; the earlier stamped second onset records its actual
`NonMonotoneStamp` refusal. Equal-valued onsets from different sources keep
distinct origins and results. A source release may precede the onset result:
the model retains its release input and mapped stamp, later submits a release
using that accepted identity, and settles credit only after the matching
audio result succeeds. A late onset refusal reaps the released entry after
the other credits are gone. The earlier two-source hold/fault fixture now
uses this command/result path for onsets and releases while retaining its
post-fault credits until a joined redemption rule exists. This proves model
ledger association around private rings. A concrete source-ring handoff and
combined receipt remain open.
A stamped onset refuses an unstamped release before source-ring custody. The
source hold keeps its stamp even after a blocked ring retry retires to a
tombstone. A release waiting in a source-local pending cell receives an exact
`MissingStamp` retry result; that cell is cleared and the primary terminal
fault owns the original. An audio-side release refusal retains the source
original and reason. It owns the primary terminal model fault if first;
otherwise its entry retains the secondary refusal while the
earlier primary fault remains. Both sources halt. A later onset refusal keeps
its ingress credit charged. Ingress, result and ledger credit stay charged
without stopped-owner proof; the model neither retries the refused release
nor claims joined teardown redemption.
A V2-local two-head merge selector refuses to choose a stamped packet while
an empty peer has no serviced frontier strictly beyond the candidate time. Only
the positive ordering fixtures use its gated model service helper; other model
fixtures use direct service to isolate individual laws or consumer behavior.
The selector chooses the least stamped onset or release head and uses source
order for equal stamps. A fixture submits onset stamps 150 then 140 and sends
them to the private mixed command ring in 140-before-150 order; both results
accept.
Another queues source A's onset at 140 and release at 145 beside source B's
onset at 150. A's two packets enter the private mixed command ring first, the
release retains A's accepted identity, and B stays held at A's serviced
frontiers 149 and 150 before proceeding at 151. A separate selector test
holds a later release behind the peer onset and chooses the lower source index
in both release/onset tie directions.
Neither command-order fixture repeats the earlier raw-bound counterexample. A
source frontier must follow its last submitted onset or release stamp. An onset
or release earlier than the source's last submitted stamp faults even without a
frontier; one before a frontier also faults with its original. An equal-time
message remains legal, so that frontier cannot close the candidate time. The
release check precedes same-key matching: stale unmatched or retired releases
fault, and a retired onset tombstone remains. These are laws for stamped
modeled head selection and source ordering. The model does not schedule
ordinary packets, obtain concrete frontiers from the example's source rings,
bound wait time or prove positive combined admission.
The release-refusal fixtures deliberately bypass the selector: they send the
second source's 150 before the first source later offers its release at 145.
The release is valid within that source, but the merge order is invalid. The
audio-side `NonMonotoneStamp` and original-custody checks remain defense in
depth. The selector would hold the peer onset while the first source lacks a
frontier. The new model fixture schedules that release before the frontier;
concrete source-ring and callback service remain open. A release earlier than
any more recent submitted source onset or release stamp is rejected at source
custody, even if it matches an older held onset.
The concrete source producer and raw owner both accept a message at the
previous frontier time. A source-ring fixture submits that equal-time message;
the V2-local selector fixture holds the peer at frontier 150, then chooses the
lower source index when the equal-time head arrives. That source's next
frontier at 151 then releases the waiting peer. The separate 149/150/151
fixture shows that without an equal-time head the frontier itself must pass
the candidate time. This strict frontier gate matches raw publication, which
holds messages at its current frontier.

An independent design read falsified an undivided shared atomic stage-credit
pool: source A can consume every credit with heads waiting for B while B needs
one credit to publish the frontier or earlier head that unblocks A. Calling B's
refusal `MixedStageFull` does not restore progress. A protected mixed onset also
needs a future release reservation that ordinary traffic cannot spend, and
consecutive stage frontiers must not exhaust credit behind a waiting head.
Before implementation, the stage needs a peer-progress path available despite
the other source's saturation (for example reserved per-source credit or an
uncharged frontier), a joint onset/release reservation, and bounded progress
through arbitrarily many consecutive stage frontiers behind a wait (for example
stage-only coalescing). Refund authority must reject duplicate and stale
claims; every raw refusal and ordinary packet needs an exact disposition, and
joined teardown must conserve all outstanding credits. A terminal shortage is
valid only when its owner and original are identified and peer silence is not
misattributed. These are design falsifiers, not an accepted stage contract.

The V2-local `stage_tests` model now exercises one bounded candidate for this
progress law after hypothetical independent raw service. Each source owns two
packet cells in the saturation fixtures and four in the between-packets
fixture; an onset claims its own cell and one protected release cell.
A frontier attaches to the preceding queued packet or advances an empty lane,
so repeated frontiers behind a full lane coalesce without using packet credit.
The selector uniformly requires an empty peer's applied frontier strictly
beyond the candidate time, even when source rank would put the candidate first
in an equal-time tie. A saturated source waiting at 150 does not consume the
peer's cells: the peer can submit and service an earlier onset and release.
Its frontier 151 releases the wait while frontier 150 does not. A second fixture
keeps one hundred consecutive frontiers behind a full lane, then verifies its
final frontier only after both earlier packets leave. A third fixture places
frontier 150 between two packets and proves an equal-time peer still waits.
These stage-only fixtures do not connect the example's source rings, raw owner
or mixed command ring. They do not assign a host fault or resolve ordinary
packets, refund authority, teardown, or the shared admission charge; the
preceding falsifiers remain open.

`bounded_stage_feeds_actual_mixed_command_results_in_time_order` now feeds the
stage's selected packets to the private mixed control/audio command ring in
both compiled/live producer orders. Source B's onset at 140 and its protected
release at 145 return accepted audio-side results before source A's waiting
onset at 150. B's applied frontier 151 lets the stage select A's onset, and
B's later frontier 161 lets it select A's release at 160. Each command/result
echoes its staged occurrence origin and request, and the release uses the
accepted onset identity.
The separate `mixed_ingress_result_channel_returns_non_monotone_refusal_with_original`
test demonstrates the audio refusal if 150 is offered before 140; the stage
test requires the positive result for 140 before 150 in both producer orders.
This is a direct stage-to-mixed-ring rehearsal with synthetic packets. It does
not include concrete source-ring custody, raw admission, the recorder, a shared
charge, or a positive combined host receipt.

The V2-local `RawStageBridge` replaces those synthetic packets with actual
per-source `SimulatedNoteInput` admissions. Each raw-accepted note reads its
`matched_onset` link immediately and enters the stage with its original and
mapped time; a release finds its staged occurrence only through that raw
same-key FIFO link. `raw_release_links_carry_source_handoffs_into_staged_mixed_results`
repeats key 60 on one source and shows both releases redeem the onsets raw
paired them with, in both producer orders, with every combined result positive
only after raw admission, stage credit and the audio-side result.
`lane_refusals_after_raw_admission_keep_the_raw_handoff_and_its_link` separates
a raw refusal, which never reaches the stage, from lane refusals that retain the
raw handoff: `NoPacketCredit`, a release linked to that lane-refused onset,
an unmatched raw release and an ordinary packet. Raw capacity and stage credit
remain independent charges checked in sequence, not one atomic shared charge.
`mixed_onset_refusal_disposes_its_raw_linked_release_without_an_offer` holds two
live notes against the two release holds, so source A's second key-60 onset
receives `Dropped(Hold)`. Raw FIFO links A's second release to that onset. A lane
selects each onset before its release, so the onset's mixed result is known when
the release is selected; the release leaves the stage without a mixed command
and its combined result carries the onset refusal and raw link. The same-key
release of the accepted onset and the peer's release succeed, and a later onset
reuses a returned hold. This is a model disposition, not a recorder or raw
settlement rule.
`joined_teardown_accounts_every_outstanding_bridge_credit` stops the owner with
two raw-linked releases and one onset still staged, two release reservations,
two accepted onsets and one `Dropped(Hold)` onset outstanding. Collection ends
`Pending` without rendering and `Faulted` after an injected fault that follows
ingress. The bridge drains its stage and ledger: every unoffered packet returns
its raw handoff, every release reservation names its raw onset, and every
accepted onset identity has exactly one onset edge or sounding entry across the
queued ingress edges, the faulted-callback journal and the sounding set, counted
so that a duplicate in one location fails. The bridge is empty afterwards and
returns its raw owners; each still resolves the unoffered packets' raw links.
The bridge also records every raw ID a raw owner issued to it, including
frontiers and lane-refused packets. After teardown, ending both raw connections
yields exactly one `Cancelled` receipt per recorded ID with the same
`matched_onset` link, and each owner's raw occupancy is zero. The only other
receipt per owner is the tick-0 frontier that `prepare` admits before any bridge
traffic; settlement names it separately rather than attributing it to the
bridge. This is cancellation settlement: delivered raw receipts and recorder
publication are not exercised here, and no refund authority or
restart after teardown is defined. The bridge still has no concrete source ring
or atomic shared charge.

The same read found that holding a popped onset until tracker credit returns
can deadlock when its release or required frontier is behind that onset in the
source FIFO. Only a shortage resolved by draining the source ring can justify
ordered retry; other shortages need an explicit terminal or
per-occurrence refusal with custody and a same-key tombstone. The source ring
must carry the onset identity redeemed by each release, including a release
blocked behind an onset retry. A future shared charge must also reserve the
audition pool for the protected release. When the onset was refused before raw
admission, its matching release is ordinary to the raw owner and needs either
ordinary-cell credit or an explicit rule to skip raw delivery. These are
falsifiable prerequisites, not an accepted combined admission contract.
Raw release admission converts its reserved cell to occupancy; the merger can
observe recorder release publication only in a delivered receipt, and audition
credit returns through settlement and collection. The private mixed channel
returns an explicit audio-side offer result but has no source or shared-credit
association. A shared ledger must name each of these distinct events before
any credit can be recycled.
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
