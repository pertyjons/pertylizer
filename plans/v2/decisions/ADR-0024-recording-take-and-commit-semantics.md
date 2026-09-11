# ADR-0024: Recording Takes and Atomic Project Commit

| Field | Value |
|---|---|
| ID | ADR-0024 |
| Status | Accepted |
| Phase | 0B/9/10B |
| Created | 2026-09-11 |
| Last reviewed | 2026-09-11 |
| Related | ADR-0022, ADR-0023, ADR-0032, ADR-0035, ADR-0036, ADR-0038, ADR-0049, ADR-0050, ADR-0052, ADR-0054 |
| Supersedes | — |
| Superseded by | — |

## Durable boundary

Recording crosses real-time ownership, performed-data retention and the later
project transaction boundary. Replace/overdub, sustain and interruption are
also product choices. The user selected this proposal after ADR-0036 as
preparation for Phase 9. The
[master plan](../master-plan.md#phase-9-live-integration-and-immutable-plan-swapping)
requires both decisions before implementation and places project commit in
Phase 10B.

This decision specifies a retained take and its transfer to Application Core.
It does not introduce a project format, persistent ID encoding or a generic job
service. ADR-0028's deferred shared-render service is unaffected. The remaining
capture-policy questions were developed at the user's request
on 2026-09-11: bounded inverse preparation, input pairing, sealing, pass storage
and resource units are selected below. Implementation of the existing runtime
loop and physical clock contracts remains later work. The user accepted this
record together with ADR-0036 on 2026-09-11 for Phase 9 entry.

## Decision boundary

A **take** is owned performed data with capture context and a final outcome.
It is not a GUI preview, a live-note telemetry stream or a series of edits to a
pattern. Capture and project commit have distinct success states. A finalized
take may be complete, partial or interrupted; any of those may still be
uncommitted, conflict-blocked or explicitly discarded by its owner.

Use typed identities for a capture session, take, loop pass and performed note
occurrence. A performed occurrence is not an index, pitch, or a renderer voice
identity: voice stealing must not erase what was played. These are accepted
domain concepts, not new persisted encodings. The canonical target identities
and revision representation belong to Phase 10A and ADR-0014/ADR-0035.

## Evidence

Premises inspected at `f34f57f6`:

- [CAP-0020](../inventories/capabilities.md) owns the V1 recording inventory;
  `crates/synth_engine/src/recording.rs` has held-note storage, count-in,
  quantization and replace/overdub target state. Its recorded-note shape is a
  pitch, velocity, pattern start and duration. This proposal does not claim that
  V1 has a retained raw-event take or atomic revision-checked commit.
- [LIMIT-0051](../inventories/resource-limits.md) and HOST-INV-020 require
  stopping at a recording capacity while keeping previously recorded data.
  The current profile carries 32 held notes and 4,096 recorded events from V1;
  those numbers are not qualification of this proposal's raw-event encoding.
- [ADR-0038](ADR-0038-engine-egress-queue-classification.md) forbids putting the
  only copy of performed data on observational egress. Counting a loss cannot
  turn it into an acceptable recording result.
- [ADR-0032](ADR-0032-sample-time-and-event-timestamps.md) clause 7 forbids
  persisting `SampleTime` and `PlanPosition`, including in receipts. Its session
  scope must remain with any runtime timestamp. The initial recommendation to
  keep raw timing is therefore limited to runtime retention, not raw timestamp
  serialization.
- [ADR-0049](ADR-0049-tempo-ramp-law.md) defines the forward tempo map but
  explicitly leaves its inverse undecided. Its clause 6 also rejects a global
  monotonicity guarantee for the accepted floating-point evaluation. A binary
  search over arbitrary ticks would therefore have an unproved premise. The
  revised proposal checks every tick of a bounded projection interval before
  permitting search; it does not change the accepted forward law.
- [ADR-0052](ADR-0052-loop-wrap-note-identity.md) remains Proposed, and ADR-0055
  still refuses runtime loop playback. Recording cannot claim that a
  pass-number field closes the sample-exact loop/identity boundary.

## Options

1. **Write notes directly into the target as callbacks complete them.** Immediate
   visibility, but project locking, partial replace and concurrent target edits
   enter the capture path. This conflicts with the Phase 9 ownership boundary.
2. **Retain a take; derive one atomic project transaction off-thread.** Additional
   bounded storage and explicit conflict handling, but capture survives GUI
   stalls and failed commits. Original performance remains available for a
   different quantization or target. Recommended.
3. **Keep only completed, already quantized notes.** Compact, but destroys the
   distinction between key release and sustain, original timing and quantized
   placement, and cannot explain partial capture as accurately.

## Decision

### Arming and capture ownership

Arming fixes the destination context, intended musical interval, capture input,
overdub/replace mode, quantization options and a retained snapshot of the tempo
and transport mapping. Default to overdub and quantization off. Armed/count-in
states do not mutate the destination. The capture start/end and same-sample
commands follow ADR-0023's declared producer order; this record adds no global
note-off-before-note-on priority.

Admission reserves capture storage, held-occurrence tracking, finalization
metadata and a non-dropping result slot before accepting the arm command. The
runtime session owns the active take. Workers own sealed chunks and derived
representations; the GUI receives a replaceable preview. Its lifetime or drain
rate cannot own the only copy of the recording.

For note input, capture at the validated performance publication boundary,
before voice allocation, tuning, voice stealing or recorded-note quantization.
Capture admission and live playback admission are separately reported: a note
may be retained even when playback refused it, and a stopped/full recorder does
not suppress otherwise valid live playback. No claim of audible/live equality
may hide that distinction. Each captured event retains source order, occurrence
identity and the original mapped timestamp/provenance; where it also enters the
renderer, associate its actual execution time or explicit refusal outcome.
That association is metadata, not another copy of the sole recording payload.

Late-clamping a playback event does not rewrite its recorded source timestamp.
Capture input is an ordered log per source connection: validation assigns a
monotone publication sequence and occurrence association before any temporal
reordering. Clock conversion supplies the nominal engine sample time and its
provenance. A decreasing nominal timestamp is retained as a source timing
anomaly; a derived negative lifetime refuses projection rather than repairing
or silently reordering the physical note pairing.

Arming explicitly supplies a nonnegative `capture_lateness_allowance`, a duration
in the capture epoch's frames. It is a chosen admission window, not a guarantee
that every physical event will arrive within it. For an observed render-clock
frontier `P`, the candidate watermark is `max(0, P - allowance)`. A worker may
advance a source's watermark only after consuming that source's publication
fence and all records through it. Watermarks never retreat. With several
sources, the take's seal watermark is the minimum of their watermarks; one
source cannot seal another source's unconsumed records. A record published
after its fence with a nominal time below that source's advanced watermark is
refused from capture; audition follows its own late policy. There is no
automatic movement into a newer take. The diagnostic always names source/time.

If that time lies in a retained take's selected capture interval, both before
and after sealing, the refusal sets that take's pre-reserved sticky quality
slot to `LateCaptureInput`, retaining the first affected time and a checked
count. Count saturation remains explicit in the slot. Repeated faults coalesce
without another result entitlement. A refusal outside every retained selected
interval remains a source diagnostic and does not fault an unrelated take.
Before sealing, the first such fault stops capture and finalizes `Partial` at
the ordered stop boundary, preserving every accepted record, including records
later than the discovered gap; this is not a claim of a gap-free prefix.
After sealing, raw data and the sealed outcome remain immutable; the quality
slot changes. Every result consumer, including commit and preview, combines
the outcome with this slot: `Complete` plus a fault is treated as incomplete.
An already applied project transaction is not silently changed or undone.

At requested stop `T`, retain admissible events in `[start, T)` until the
take's seal watermark reaches `T`; source fences also order equal-sample commands under
ADR-0023. If already accepted raw input carries a nominal timestamp beyond an
early stop, keep it marked outside the selected interval; it does not become an
in-range note by clamping and is excluded from automatic projection. If the
render clock stops advancing or a device is lost, close source
admission and obtain its quiescence fence off-thread instead. Preserve every
already accepted record, close at the last valid boundary, and label the result
interrupted rather than waiting for a nonexistent future callback. Callback
threads never wait for the worker or a fence consumer.

`Complete` means no known gap in the admitted capture window at sealing; it is
not a promise about arbitrarily delayed physical input. The quality slot and
result entitlement are reserved at arm whether or not a fault occurs. Retain
them until every participating source generation is quiescent and the take's
owner has acknowledged the current quality state. They continue consuming the
result entitlement after project commit; reaching the limit refuses another
arm until the sources are quiesced/rebound and the quality state acknowledged.
This explicit cost bounds late-fault attribution instead of keeping an
unbounded take index. Quality is not droppable meter telemetry. Source
generations are retired before their attribution state can be reclaimed.
Hardware selection of the allowance and its observed late-refusal rate require
ADR-0022's qualification; exact simulated sources can explicitly select zero.

### Note lifetime, count-in and transport changes

Retain note-on, key release, sustain changes and supported expression separately.
Every admitted note-on mints a fresh `PerformedOccurrenceId` within its typed
source connection generation; checked counter exhaustion stops capture without
reusing an identity. It is independent of the renderer's allocator.

For the initial MIDI 1 input, normalize note-on with velocity zero to key release.
Match releases FIFO to the oldest unreleased onset with the same source
connection, MIDI channel and key. That tuple is a lookup key, not occurrence
identity. For `on A, on B, off, off` on one key, the releases close A then B.
Unmatched releases are counted and retained as diagnostics, never guessed onto
another channel or generation. Sustain does not postpone this key pairing.
Sources with a native occurrence token may use a separately declared exact-token
adapter; accepting arbitrary token schemes is not implied by MIDI 1 support.
A renderer steal affects the audition trace, not captured physical key lifetime.

The source tracker includes pre-capture held keys and their FIFO order: those
entries are marked uncaptured, so their releases cannot close a later captured
onset. Initial pedal/controller state and held-key state must be established
before arming. If that state is unknown, arming refuses until the source is
explicitly reset/reconnected and synchronized. Tracker overflow invalidates
pairing and interrupts capture; refusing just one onset and continuing to pair
later releases would be ambiguous. Space for all tracked occurrences, including
uncaptured ones, is admitted separately from the take's held-note capacity.

Count-in establishes the initial pedal/controller state but records no earlier
note-on. The initial policy captures notes newly started within the recording
interval; keys already held when it starts are not silently retriggered or
invented as new notes. Their later releases cannot close another occurrence.

At normal stop, disarm or panic, finalize at the ordered capture boundary.
Record explicit synthetic closure for any open captured occurrences, preserving
whether the key or pedal was still held. Panic retains the take and is not
discard. A note-on exactly at the exclusive end belongs outside that interval;
finalization uses the prior held state. The half-open interval is determined
from capture time, not the GUI's receipt of a command.

For the first note-only projection, note duration follows key release; sustain
is a separate retained controller. A target that cannot express captured
sustain or expression must refuse automatic commit of that material and keep
the take. Do not silently lengthen notes to approximate pedal behavior. A future
explicit conversion may offer that transformation with its limitations visible.

Seek, tempo-map replacement and device re-preparation finalize the current
capture segment before changing its mapping. A new segment requires a new
explicit capture start. Thus one segment never reinterprets earlier events
using a later tempo map. Same-plan loop wrap is the sole intended exception,
with the separate pass contract below. Device loss follows ADR-0036's quiescence
and last-valid-capture boundary; it never depends on another callback arriving.

### Loop recording

Retain each pass as a separate take segment under one recording session, with
an explicit half-open interval. Preserve the original occurrence and raw
key/pedal sequence across the boundary; do not inject a played note-on into
live audio merely to split a storage segment. Carry-in/carry-out annotations
are distinct from performed events. Pass storage and those annotations consume
admitted capacity; loop recording is not an unbounded vector of takes.

Default to retaining passes separately. Combining selected passes is an
explicit overdub transaction, and choosing a replacement pass never deletes
other retained passes. `PassId` is a checked non-reused session counter, not a
renderer generation or the pass's vector index. Exhaustion finalizes the
recording before another pass is admitted.

The recorder consumes an ordered `PassBoundary` carrying the effective engine
time, old/new pass identities, loop interval and the new transport mapping.
At boundary `T`, pass A owns events before `T` and pass B owns events at/after
`T`. Apply the session boundary before other producers at the same sample,
without changing each source's FIFO occurrence association. A release at `T`
therefore belongs to B even though it closes an occurrence begun in A. A
boundary must be sealed against every capture source fence just like stop.
The physical occurrence and pedal state continue; only capture segmentation
changes. Reserve carry-out/carry-in metadata before crossing a boundary.

For a contiguous multi-pass projection, first join segments by performed
occurrence and project its one onset and one key release on an unrolled musical
timeline; do not create an attack per pass. For an isolated pass that contains a
carry-in note, automatic note-only projection refuses: the target cannot
represent an already sounding continuation by inventing a note-on. The retained
pass can instead be selected together with its onset pass. A carry-out note can
be explicitly trimmed to the selected interval, with a synthetic closure and
visible transformation in the commit preview; without that choice projection
refuses. Pedal/controller state at selection start must be representable by the
target or projection refuses under the existing expression rule. Thus separate
pass storage promises neither seamless isolated-pass playback nor silent cuts.

This completes the recording-side storage and projection policy. It does not
supply the runtime `PassBoundary` producer. ADR-0052 still owns sample-exact wrap
execution, compiled per-pass identities, repeated catch-up admission and
precedence with pending activation. Its production consumer must establish the
above boundary stream without changing the captured physical occurrences.
ADR-0055 remains in force until that loop work is completed. Synthetic boundary
logs can test this recorder contract independently; no successful live-loop
activation or Phase 9 exit is claimed by those tests.

### Runtime timing and project projection

Runtime raw timing carries its session, stream epoch and original mapping
context. It cannot survive that context by retaining an unscoped frame integer.
Before any persistent output, derive musical positions and asset-relative
sample offsets and discard engine-epoch positions from the serialized form.
Unquantized means no user grid snapping; it does not promise infinite musical
resolution. A projection must report its native tick-resolution error separately
from optional quantization and latency compensation.

Select an off-thread prepared projection table over an explicit finite closed
musical interval `[a, b]`. Before capture can rely on projection, checked
`b - a + 1` must fit `max_projection_ticks` and the table's byte budget. Evaluate
`TempoMap::position_of` at **every integer tick** in that interval using the
accepted forward law. Any conversion failure or decreasing adjacent frame
refuses preparation with the offending ticks named. Checking only endpoints or
sampling a stride is insufficient under ADR-0049 clause 6. No assumption about
monotonicity elsewhere in the map enters this certificate.

Retain the checked table with the exact tempo-map revision/rate and target
interval. A lower-bound search in its nondecreasing frame column finds the
adjacent candidate frame values for a captured position. Choose the candidate
with smaller absolute frame distance; for equidistant different frames choose
the earlier musical tick. For an equal-frame plateau, a second lower-bound
search selects the earliest tick on that plateau within `[a, b]`. These are two
different tie cases. Return the chosen tick and the signed difference between
its forward frame and the captured frame. Runtime timestamps stay out of the
persisted result.

A position outside the prepared table's frame range refuses projection; it is
not clamped. A changed map, rate or target interval requires a new table. This
accepts the cost of one bounded off-thread enumeration and table storage rather
than changing the forward tempo law or claiming that sparse measurements prove
an inverse. Preparation failure leaves the take/old prepared capture untouched.
Finite capture windows are explicit: reaching the prepared window's end seals
the segment, and extending it requires a newly admitted window. A loop reuses
one certified interval plus its pass mapping, not an unbounded table of passes.
The conformance tests are required before enabling the projection implementation;
this decision does not claim those tests already exist.

Optional quantization snaps note starts to the selected musical grid, choosing
the earlier grid point on a tie and preserving the derived note duration;
velocity and pedal/expression timing are unchanged. Retain the unsnapped take.
A nonpositive projected duration, unrepresentable value, or snapped note outside
the allowed target interval blocks that projection with the affected occurrence
named. Do not silently delete, truncate or move the note back into range.

Input/output latency, the render adapter's Q and any user offset must have one
named compensation owner and a recorded applied correction. Never subtract a
callback estimate twice. ADR-0022 must settle the mapping and compensation used
before a production recorder can claim aligned placement.

### Overdub, replace and concurrent project edits

Overdub inserts the chosen projected events. Replace applies only to the
explicit target, musical interval and selected event classes. For the initial
note replacement policy, membership is by note start in `[start, end)`; a note
starting before the interval remains untouched even if its tail overlaps it.
This rule is visible in the transaction preview. Controller replacement
requires an explicit selected lane and restoration-at-boundary rule before it
can be offered; selecting notes does not implicitly erase controllers.

Capture keeps stable target references and an expected base revision. Commit
validates both inside the same Application Core transaction that applies the
insertion and any replacement deletion. Start with whole-project revision
equality: unrelated edits may cause a conservative conflict. Never guess a new
target by name, position or current GUI selection. A conflict retains the take;
an explicit retry may select a new target/base without recapturing.

The transaction has a stable operation identity: retry after an uncertain
acknowledgement returns the prior result rather than inserting twice. Undo
restores both removed and inserted material as one operation. A partial or
interrupted take is never automatically committed as replace; the user must
explicitly select its retained material and replacement interval. Ordinary
complete takes may commit under the mode already selected when arming.

These are recording-specific requirements on Phase 10B. They do not declare
ADR-0035 accepted or choose its general transaction implementation. A commit
consumer cannot ship until canonical revisions, identities and undo storage
can enforce them together.

### Capacity, finalization and audio assets

Define ordinary capture capacity separately from emergency finalization space.
Reserve at least one closure record for every admitted open occurrence and one
terminal result per take, plus bounded pass-boundary metadata. When ordinary
storage would be exceeded, stop accepting new capture at a named boundary,
finalize using the reserve, and keep every accepted event. A zero-copy handoff
must retain ownership until acknowledgement; a full notification queue cannot
destroy its payload. New arming refuses while no result/storage entitlement is
available. Reclamation occurs off-thread.

Select the following resource units for the recording implementation. They
are accepted additions/clarifications to `RecordingLimits`, specified in the
current host profile but not yet implemented by its Rust representation:

| Budget | Unit and admission rule | Exhaustion |
|---|---|---|
| `max_held_notes_per_take` (`H`) | Captured occurrences whose keys remain down; the existing value 32 stays a provisional policy choice | Stop before accepting onset `H + 1`; use finalization reserve |
| `max_recorded_events_per_take` (`E`) | Ordinary raw captured records: each onset, key release, sustain or supported expression event consumes one; the existing 4,096 becomes explicitly a chosen raw-record budget, not a V1 completed-note equivalence | Stop before accepting record `E + 1`; retain the prefix |
| `max_tracked_input_notes` | All paired key-down source occurrences, captured and uncaptured; must cover `H` and the known pre-capture state | Interrupt capture and invalidate source pairing; require reset before rearm |
| `max_capture_sources` | Source connections in one armed capture | Refuse another source at arm/reconfiguration |
| `max_capture_passes` (`P`) | Retained pass segments in one take session, including empty passes; at least one | Seal before starting pass `P + 1` |
| `max_pending_capture_results` | Active plus sealed, unacknowledged results; reserve one result and one sticky quality slot per armed take | Refuse another arm until an entitlement is released |
| `max_capture_bytes` | Aggregate bytes for raw records, tracking, reordering, projection tables/results, pass/terminal metadata and still-retained takes/chunks; no uncharged secondary vector | Refuse arm or stop at the last fully retained item |
| `max_audio_capture_frames` | Total source frames per admitted audio segment, at its fixed source rate/layout | Seal a capacity-limited segment before the first excess frame |
| `max_projection_ticks` | Forward-map evaluations/table entries in one certified interval | Refuse table preparation before enumeration/allocation |
| `capture_lateness_allowance` | Nonnegative epoch-frame duration; zero is an explicit valid choice, not missing configuration | Apply the watermark admission rule |

The additional budgets have no inferred production defaults: the first Phase 9
capture configuration must supply typed values explicitly, and construction
refuses missing, zero where positivity is required, inconsistent or
unrepresentable values. Their aggregate cost is checked before allocation and
before accepting arm. Values selected for a simulated fixture are labeled as
fixture policy; production values require the first capture consumer's workload
qualification. This specifies the admission policy; its implementation remains unbuilt.
It does not permit an implementer to choose hidden constants.

`E` excludes finalization and pass metadata, which are charged separately in
`max_capture_bytes`. Reserve `H` terminal occurrence records, one initial
controller-state snapshot per source, and at most `2 * H * (P - 1)` carry records
for interior pass boundaries, using checked arithmetic. A captured occurrence
remains one ordinary onset record regardless of passes; synthetic continuation
metadata is never another performed event. Sustained-but-key-released state is
represented by already retained key releases plus the pedal state, not by
unbounded new held-key entries. Reserve terminal pedal state for every source
as well as the take's outcome, first fault location and publication fences.

Audio chunk count/bytes and their descriptors are included in the aggregate
budget; a worker freeing a consumed transfer chunk does not grant permission
to exceed the total admitted audio-frame or retained-asset storage budget.
All custody is retained until commit/discard and final reference retirement.
A capture may stop when a worker stalls arbitrarily long, but it cannot lose
accepted data or borrow a result slot from telemetry. No live renderer event
share is enlarged here: ADR-0054 still qualifies audition/transport producers
separately before production enablement.

Audio capture hands preallocated sample chunks to a worker together with their
source rate, channel layout and timing continuity. The worker creates immutable
asset data. Input loss, chunk exhaustion or storage failure stops capture and
retains its valid prefix with the first missing frame/range or explicit unknown
gap boundary. Silence inserted for monitoring is not genuine recorded input.

Only a completed, validated asset may be referenced by a project transaction.
Asset finalization precedes atomic project insertion; a failed or conflicting
insertion keeps an owned uncommitted asset for retry. Project commit and its
operation identity publish together. Cleanup must not delete an asset still
held by a take, a project or undo history. Persistent asset identity, storage
format and crash recovery are Phase 10A/10D decisions, not implied guarantees
of an in-memory take. No runtime `SampleTime` is saved in a recovery record.

Discard is an explicit owner action after finalization. Process-crash survival
is not claimed until a durable take/recovery format exists; orderly shutdown
must offer retention through a supported format or report that uncommitted
session data still requires a decision, rather than silently dropping it.

## Falsifier and acceptance checks

The decision fails if a legal stall or interruption can lose accepted data,
change its original timing, apply partial replace, duplicate a retry, or require
allocation/blocking on a callback. Required future checks are:

- The same mapped input and session boundaries under whole, 64, 256 and
  irregular callbacks produce identical raw takes and derived events. Vary
  late execution while keeping original stamps fixed; source timing survives.
- Publish the same in-interval below-watermark record before and after sealing.
  Both set the existing quality slot without another entitlement; the first
  finalizes `Partial`, the second degrades effective quality without rewriting
  raw data or an applied transaction. Repeated faults coalesce, and an
  out-of-interval refusal does not fault an unrelated take. Stall one of two
  sources: the minimum seal watermark waits for its fence. Retain attribution
  through commit until all source generations quiesce and quality is acknowledged.
- Count-in, exact start/end, note-off and sustain at one sample follow the
  declared ordering. Same-key occurrences, stolen voices and held keys at arm
  cannot close or create the wrong recorded note.
- Exhaust each ordinary and result capacity with all held slots occupied and
  stall the worker/GUI. Finalization still succeeds, preserves the prefix and
  names the partial outcome without freeing owned memory on a callback.
- Change or delete the target, fail asset finalization and lose the commit
  acknowledgement. The project is unchanged on failure, the take remains owned,
  and a retry has exactly one effect. Undo restores the entire prior target.
- Compare table lookup against exhaustive nearest-tick selection on steps,
  period ramps, boundaries, equal-frame plateaus and distance ties. Inject an
  adjacent inversion that an endpoint/stride check misses and require preparation
  refusal. Check work/byte bounds, stale table context, out-of-range and
  zero-duration refusals. Report each projection's actual signed error.
- Feed synthetic ordered pass boundaries at non-Q-aligned positions and record
  a held/sustained note across them. A release exactly on a boundary closes the
  original occurrence in the next pass. Contiguous projection has one onset;
  isolated carry-in projection refuses, and carry-out trim requires selection.
  Repeat against the real boundary producer when ADR-0052 is implemented.
- Interrupt audio without a final callback; stop at the last valid watermark,
  preserve source format and prefix, and never label a gapped asset complete.

These are obligations, not passed tests. Physical timestamp/latency evidence
belongs to ADR-0022; no new EVD result is asserted.

## Consequences and risks

- Accepted cost: more bounded storage than completed-note capture,
  conservative revision conflicts, and explicit handling of partial takes.
- Safety control: capture storage, finalization reserve and custody are admitted
  before recording, independently of lossy preview and audition outcomes.
- Revisit: a supported target cannot represent the retained expression or a
  measured workload makes the proposed capture budgets unsuitable. Refuse the
  unsupported projection or requalify the budget before enabling it.

## Specification update and acceptance boundary

Accepted on 2026-09-11 at the user's instruction to accept the reviewed policies
and write their current specifications. The current contract is
[spec-recording-takes-and-commit.md](../specs/spec-recording-takes-and-commit.md), registered with prefix `TAKE`.
The [host profile](../specs/spec-host-profile-and-render-limits.md) incorporates
the recording units, reservation rules and lifecycle link in the same change.
Together ADR-0024 and ADR-0036 discharge Phase 9's entry-decision requirement.

ADR-0022 still gates production mapping/latency and the capture allowance;
ADR-0052 still gates real loop-boundary production. Phase 10A/10B/10D still own
canonical targets, atomic transaction machinery, durable assets and recovery.
These are first-consumer gates, not prerequisites for a stopped-only simulated
host and bounded recorder. No hardware result, runtime-loop implementation or
Phase 9 exit is claimed. Live swaps retain ADR-0009, ADR-0010 and ADR-0050's
separate obligations.

## Review

Codex (GPT model family) authored this decision and its current specification.
The fresh Claude Sonnet readers below are from a different model family and
had no authoring context; no same-family review waiver was used.

A fresh Claude Sonnet reader reviewed the coupled design and complete
uncommitted documentation on 2026-09-11, using read-only tools with MCP and
delegation disabled. It found no stopping-rule defects, including in the
distinction between source capture, live audition, runtime timing and persistent
project placement. It noted optional wording clarification between equal-distance
ties and equal-frame tick plateaus; both remain explicit cases for the inverse
projection checks. This review does not accept the proposal or qualify timing.

Stopping rule: a false conclusion-affecting premise, contradiction, unfillable
contract, safety/correctness defect, or unsupported evidence claim blocks
acceptance. Optional implementation detail does not.

### Policy-completion review

The 2026-09-11 follow-up develops the previously named recording-side choices.
A fresh Claude Sonnet reader reviewed the complete coupled revision with
read-only tools, MCP and delegation disabled. It found ambiguity in late-input
quality handling before versus after sealing. A focused reread also required
explicit multi-source watermark aggregation and late-input conformance checks.
The repair specifies the minimum seal watermark, one pre-reserved quality slot,
pre-seal partial finalization, immutable post-seal data and all-source quiescence
before attribution reclamation. The final focused reread found no remaining
defects; its wording note is resolved by naming the take's seal watermark at
stop. Related-record links were completed. Both review commands completed
successfully. These reviews neither accept the proposal nor qualify hardware
or implementation behavior.

### Acceptance review

A fresh Claude Sonnet reader reviewed the complete uncommitted acceptance diff,
including both new current specifications, with read-only tools, MCP and
delegation disabled, on 2026-09-11. It found a missing pre-arm obligation in the
HOST-INV-020 test row and an obsolete entry-preparation sentence in NOW.md.
Both were repaired. Author audit synchronized the profile's field origins,
quantity-type index and historical counts. A focused reread confirmed those
repairs and found only inconsistent reserve terminology in the test row; author
self-audit replaced it with the invariant's term, `finalization reserve`.
Both reader commands exited successfully. The fast documentation gate and all
35 checker tests passed. No remaining stopping-rule defect is recorded.
Historical proposal reviews above retain their original scope; acceptance makes
no new implementation or measurement claim.
