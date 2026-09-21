# ADR-0072: Finite live audition and count-in

| Field | Value |
|---|---|
| ID | ADR-0072 |
| Status | Accepted |
| Phase | 9 |
| Created | 2026-09-21 |
| Last reviewed | 2026-09-21 |
| Related | ADR-0071, ADR-0024, ADR-0054, IO-INV-004, TAKE-INV-002 |
| Supersedes | ADR-0050's off-thread-only minter rule and ADR-0046 clause 6, ADR-0047 clause 7 and ADR-0050 clause 8's release timing, solely for the exclusive immutable live owner below |
| Superseded by | ADR-0073 for reusable cells, source quota and bend; ADR-0009 for the separate reset consumer |

The finite quota and unsupported-bend sections below record the original decision.
[ADR-0073](ADR-0073-continuous-simulated-live-input.md) replaces those clauses and
[ADR-0009](ADR-0009-plan-swap-crossfade-and-latency.md) adds a separate reset owner.

## Boundary and falsifiers

The user selected audible simulated input through Core V2's real ingress and
voices. The finite host must preserve capture independently of audition, end held
notes and sustain on ordered Stop/panic, and sound a count-in without recording
invented onsets. A stuck voice, partition-dependent timely output, changed raw
selection when audition is enabled, unresolved sealed audition trace, callback
allocation/destruction or uncharged live storage falsifies this design.

False claims, contradictions, unfillable contracts and safety defects block
acceptance. Optional implementation detail does not. Physical input, production
budgets, arbitrary punch-in and activation with held live notes remain gated.

## Exclusive live owner

`LiveInputStream` exclusively owns one immutable plan, its `StreamControl` and
identity minter, renderer, empty compiled scheduler, ingress and publication
arbiter. Its only note producer has one to eight voices, no stealing, no authored
source and no activation. Callback-local minting and bounded offers are permitted
only inside this owner. Their implementations join the real-time source scan.
This amends ADR-0050's control-thread ownership rule for this consumer; it grants
no authority to move the production minter onto an arbitrary callback.

The compiled loop and live plan have separate gates and renderers. Both start at
engine zero, use identical profile geometry and retain the existing Q-frame carry.
Audition adds no latency beyond that common carry. An input retains its nominal
capture stamp. A late audition uses the first unrendered boundary and records its
actual execution time in its own renderer epoch. Partition equivalence requires
identical delivery before the destination quantum; late delivery instead promises
truthful actual-time outcomes, never retroactive modification of produced audio.

Per-source and per-channel FIFO occurrence trackers include refused-on tombstones.
A release consumes its matching occurrence, not another sounding key with the same
pitch. Sustain retains key-up voices until that source/channel's pedal lifts.
Pitch bend is explicitly unsupported for this finite audition owner; raw capture
retains it normally. Capture admission and ordinary ingress refusal are independent.

A sealed `ReleaseGroup` carries at most eight distinct identities from the latched
producer. Preflight checks every identity before mutation. One source operation
is charged to Live for pedal lift/panic or Session for Stop; its bounded gate and
trigger fanout is included in renderer scratch admission. Empty groups do nothing.
The ingress consumes reserved release holds and frees the minter entries at offer;
the renderer releases its separate registry in event order, including same-quantum
index reuse. This explicit timing exception amends ADR-0046 clause 6, ADR-0047
clause 7 and ADR-0050 clause 8 for the exclusive, non-activating owner. It cannot
redeem holds across a plan swap. Invalid or repeated members mutate nothing.

## Finite source and result custody

The concrete simulated producer accepts at most 64 observations per source for
one attempt, including frontiers. Collection does not replenish the quota, and
it is identical with audition enabled or disabled. Ring saturation returns the
original observation for ordered retry. Total quota exhaustion instead requests
SourceInvalid and returns the unaccepted original; joined interrupted recovery
retains accepted observations without waiting for a later key-off.

Audition packet cells, retained outcome cells and FIFO trackers each cover the
sum of both complete source quotas. No cell is reused before joined retirement.
Thus accepted input never waits for audition collection or a later release in
its own FIFO. Ordered transport end has separate protected custody. The core
live-ingress admission may refuse a sounding onset, independently of capture.

Raw accepted input may carry `Pending(AuditionId)` while mutable. After callback
join every audition operation is resolved as executed with actual epoch/time,
ingress-refused, unsupported, unmatched release, not-sounded or cancelled. The
host reconciles retained raw annotations before sealing and also reports outcomes
for auditioned input outside raw selection. Pending annotations block sealing;
sealed annotations cannot be rewritten. This is not a persisted or wire format.

Applied Stop/panic at B ends all held live notes and closes audition. It wins over
input at B. Every input delivered to the audition owner after closure is cancelled,
even if its nominal time precedes B. Queued later operations are cancelled too.
The finite trackers are retired together rather than reused after closure. Panic
is an ordered complete capture end with Panic provenance, not DeviceLost and not
an Interrupted substitute. Terminal faults instead silence the whole host and
use joined cancellation/recovery without requiring another callback.

## Count-in and metronome

A third exclusive Core V2 renderer contains a global oscillator, envelope and
amplifier. It has no note producer, identity obligations or shared gates. Its
immutable pulse schedule publishes sample-positioned gate writes through the
Session share, with two events per click and one reserved end event in each
quantum. Preparation checks profile geometry, finite storage, positive width,
strictly separated pulses and destination density. Events wait in owned prepared
storage until their destination quantum. Stop/panic lowers the gate at B and
cancels the remaining schedule, including an already sounding click.

The concrete counted recipe fixes 120 BPM, one to sixteen whole quarter notes of
count-in and a four-quarter loop, with the existing finite 32-pass limit. Its
immutable tempo map determines every pulse and reserved recording start. An exact
start which is not quantum aligned is refused rather than moved. Device time
advances during count-in while song position remains stopped. Recording begins
only at its prepared Play boundary; held pre-roll keys acquire no invented onset.
All three renderer resources, pulse storage, scratch, queues and retained results
are charged before activation. Whole-callback failure silences the mixed output.

## Linux harness and verification

The non-shipping `v2_cpal_output capture <ALSA-id> <callback-count>` mode connects
this host to the existing joined ALSA custody model. Two explicit fresh attempts
retain their results; the first uses Stop, the second panic. Each attempt publishes
Play and end only after callbacks start. Two simulated source workers return their
unique producer endpoints before source acknowledgement. Backend failure retains
an interrupted result and reports its receipts even without a final callback.
The unpaced ALSA null device can overtake control publication; a late refusal is
reported rather than treated as a successful physical timing qualification.

Core group/live tests cover sample-exact release, duplicate refusal, index reuse,
per-source sustain and callback partitions. Host tests additionally cover audible
raw reconciliation, count-in, cutting a sounding click, zero callback allocations,
measured admitted heap, equal source quotas with audition on/off, the final allowed
release while results remain uncollected, and joined recovery without a callback.
Scheduled capture tests refuse unresolved audition sealing and post-seal changes.
Independent design consultation found and repaired release custody, publication
lateness, quota deadlock, capture coupling and ambiguous post-Stop outcomes. Independent
uncommitted review and focused repair review passed, with the reader's final narrow
custody follow-up self-audited. The complete repository gate and concrete host tests
passed. The ALSA null smoke run completed both attempts, retained Complete takes
and projected their notes; it establishes no physical timing or MIDI qualification.
