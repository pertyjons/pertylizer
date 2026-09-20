# SPEC: Recording Takes and Commit

| Field | Value |
|---|---|
| Status | Current |
| Phase | 9/10B |
| Created | 2026-09-11 |
| Last reviewed | 2026-09-12 |
| Based on | ADR-0024, ADR-0036, ADR-0023, ADR-0032, ADR-0038, ADR-0049, ADR-0054, ADR-0055, ADR-0066 |
| Invariant prefix | TAKE |
| Supersedes | — |
| Superseded by | — |

Only a `Current` specification constrains implementation; see [README.md](README.md).
This contract is accepted for the consuming implementations. P09-S002–S005's bounded
storage, exact-input, projection and host-interruption checks and remaining consumer obligations are separated below.

## Scope

This specification defines runtime custody of performed input, finite capture
windows, projection into musical time and the requirements on a later atomic
Application Core commit. Capture and project commit are separate success states.

## Non-goals

No project format, durable recovery encoding, canonical revision representation
or general transaction service is introduced. Those remain Phase 10A/10B/10D.
Physical clock mapping remains ADR-0022. ADR-0065 supplies the exclusive compiled
loop owner, and ADR-0066 supplies its serial recording-boundary consumer. Concurrent
input transfer and loop session controls remain open. ADR-0055 continues to refuse
loop-bearing ordinary activations.
The recorder may be tested against synthetic input and boundary logs first.

## Terminology

A take is owned performed data with capture context and a final outcome, not a
GUI preview or live-note telemetry. A pass is a half-open segment of a recording
session. A performed occurrence identifies an input onset independently of key,
collection index and renderer voice. A watermark is a fenced capture-admission
frontier; the take's seal watermark is the minimum across its sources.

## Accepted decisions

| ADR | Decision it fixes here |
|---|---|
| [ADR-0024](../decisions/ADR-0024-recording-take-and-commit-semantics.md) | Retained takes, pairing, projection, pass storage, bounded finalization and atomic commit requirements |
| [ADR-0036](../decisions/ADR-0036-audio-device-and-input-lifecycle.md) | Host ownership and device-interruption handoff |
| [ADR-0023](../decisions/ADR-0023-same-sample-event-ordering.md) | Declared same-sample producer order |
| [ADR-0032](../decisions/ADR-0032-sample-time-and-event-timestamps.md) | Original epoch-scoped timing and prohibition on persisted runtime time |
| [ADR-0038](../decisions/ADR-0038-engine-egress-queue-classification.md) | Non-dropping custody versus observational egress |
| [ADR-0049](../decisions/ADR-0049-tempo-ramp-law.md) | Forward tempo law; no assumed global floating-point monotonicity |
| [ADR-0054](../decisions/ADR-0054-staged-producer-capacity-calibration.md) | Separate production renderer-share calibration |
| [ADR-0055](../decisions/ADR-0055-refuse-unimplemented-loop-playback.md) | Loop-bearing ordinary activation remains refused; ADR-0065 supplies a separate exclusive owner |
| [ADR-0066](../decisions/ADR-0066-serial-loop-capture-segmentation.md) | Serial actual-journal capture, explicit loop mapping and bounded nominal carry |

## Invariants

The following requirements are normative.

### TAKE-INV-001 — Arming and capture ownership

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
Once attributed, a fault stays recorded even if a later ordered stop narrows the
selected interval. Refusals arriving after sealing use that final interval.

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

### TAKE-INV-002 — Note lifetime, count-in and transport changes

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

### TAKE-INV-003 — Loop recording

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

ADR-0065 supplies sample-exact compiled wraps within an exclusive owner. Its finite
journal retains successful whole-render boundaries and a terminal endpoint.
[ADR-0066](../decisions/ADR-0066-serial-loop-capture-segmentation.md) binds that actual
journal and the exact serial recorder in one `LoopCaptureSession`. It stores one
continuous raw FIFO log with separate identified half-open pass descriptors. Nominal
T routes to the new pass without reordering physical occurrence pairing. The caller
still supplies serial publication order; this is not a concurrent source merge.

Loop context retains the original musical interval and tempo. Their forward frame
endpoints and rate must match the loop; unsupported mappings refuse. A loop take has
an explicit mapping kind and no single linear anchor. Automatic note projection
refuses it until the pass-aware projection contract above is implemented.

Raw sealing waits behind a barrier until actual audio observation ends and all
participating source fences cover the selected endpoint; interruption additionally
requires quiescence. Off-thread finalization writes pass/carry metadata before sealing.
The final prefix is bounded by both raw and actual render endpoints, never worker
wall time. Accepted raw outside a retrospectively shortened prefix remains retained.

Physical H does not bound nominal carry when accepted timestamps regress. Before
admitting an interior transition, reconstruct key continuation from accepted onsets
strictly before T and accepted paired releases not strictly before T. If its count exceeds H,
finalize Partial at T with `LoopCarryCapacity`; retain its location and all accepted
raw, and admit neither the next pass nor that transition's carry pair. A stronger
existing interruption remains Interrupted. Terminal closures use each retained
onset's admitted field. Sustain remains separate raw state, not extra key-held carry.
Key metadata alone certifies neither expression nor musical projection.

The combined serial owner and its recording memory share the checked recording
ceiling; journal heap and compiled-loop storage keep their separate declared budgets.
P recording identities are reserved at arm, separately from renderer pass identities.
A pending initial wrap may give an empty initial segment; an exclusive final boundary
never admits an empty following pass. Reading cannot renew any entitlement.

[ADR-0067](../decisions/ADR-0067-ordered-serial-loop-recording.md) adds the finite
[ordered serial loop recording consumer](spec-host-io-lifecycle.md#ordered-serial-loop-recording).
Its coupled Play/Stop, retained receipts, whole-callback audio authority and stopped
source drain preserve these pass/carry and sealing rules.
[Finite loop transfer](spec-host-io-lifecycle.md#finite-loop-recording-transfer) under ADR-0068
retains them across bounded thread handoff and joined worker finalization.
[Simulated input](spec-host-io-lifecycle.md#simulated-input-clock-and-capture) under ADR-0069
adds independent synthetic clocks and arrival-ordered merging behind explicit source
prefixes. Input exhaustion and late delivery retain their observations and interrupt
the take; no-final-callback loss still waits for every input quiescence acknowledgement.
Exact refused input behind a consumed fence retains the existing late-quality
attribution after sealing. An uncertain refused message whose possible interval
overlaps the selection retains `first_uncertain_source`; its quality makes a sealed
Complete result effectively Partial without asserting an exact timestamp. The same
quality rule covers the valid-domain portion of a refused mapping interval; diagnostic
intersection never admits clipped input.
Physical clocks, concurrent input and restart still need their first consumers. ADR-0055 continues
to protect ordinary activation-based schedulers. No concurrent live-loop activation or
Phase 9 exit follows from this serial reference consumer.

### TAKE-INV-004 — Runtime timing and project projection

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
the bounded exact-input implementation's checks are recorded below.

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

### TAKE-INV-005 — Overdub, replace and concurrent project edits

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

### TAKE-INV-006 — Capacity, finalization and audio assets

Define ordinary capture capacity separately from emergency finalization space.
Reserve at least one closure record for every admitted open occurrence and one
terminal result per take, plus bounded pass-boundary metadata. When ordinary
storage would be exceeded, stop accepting new capture at a named boundary,
finalize using the reserve, and keep every accepted event. A zero-copy handoff
must retain ownership until acknowledgement; a full notification queue cannot
destroy its payload. New arming refuses while no result/storage entitlement is
available. Reclamation occurs off-thread.

The [host profile recording fields](spec-host-profile-and-render-limits.md#recording)
are the authority for resource units, defaults and exhaustion. All capacities
and aggregate allocation costs must be checked before arm; the implementation
must not infer missing production values from a fixture. `H`, `E` and `P` below
are the admitted held-occurrence, raw-record and pass limits respectively.

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

## Types and ownership

Use private, validated domain newtypes for capture session, take, pass,
performed occurrence, source connection generation and every capacity/unit.
Counters cannot wrap into reused identity. These are domain concepts, not new
serialized encodings. The session owns active capture and its reserved result;
workers own sealed chunks and derived data; the GUI owns a replaceable preview.
A final outcome (`Complete`, `Partial`, `Interrupted`) is distinct from commit
state (uncommitted, conflict-blocked, committed or explicitly discarded) and
from the retained quality slot. Every consumer combines outcome and quality.
Canonical target identity and revision belong to Phase 10A and ADR-0035.

## Lifecycle and timing

TAKE-INV-001 and TAKE-INV-002 define arm, start, stop, source fences and mapping
changes. TAKE-INV-003 defines pass segmentation. TAKE-INV-004 governs off-thread
projection without replacing original capture time by playback execution time.

## Failure and diagnostics

TAKE-INV-001 defines late-input quality, including discovery after commit.
TAKE-INV-005 defines conflicts and retry; TAKE-INV-006 preserves accepted data
on capacity, interruption or worker/storage failure. Missing input is never
reported as complete audio. These outcomes must remain observable independently
of a stalled GUI and saturated lossy telemetry.

## Real-time and resource constraints

The callback must not allocate, block, log or reclaim the final reference to a
result, stream, chunk or asset. Storage, closure metadata, quality and publication
slots are reserved before arm. Projection preparation, transactions, asset
creation and reclamation run off-thread. Use the host profile's recording
budgets and TAKE-INV-006's checked finalization reserve; no secondary uncharged
collection may hold recording data.

## Conformance tests

P09-S002 implements complete typed capture configuration and a serialized
`SimulatedTakeStore<T: Copy>` in `synth_engine_v2/src/recording.rs`. It reserves
ordinary cells, tracked-input cells, `H` terminal occurrence cells, two source
snapshot regions, `P` pass cells and `2 * H * (P - 1)` carry cells for every
retained-result slot. Checked aggregate bytes include the actual typed cells,
source ledgers, slot descriptors and store descriptor before allocation.
Allocator bookkeeping is outside the requested Rust layout. This storage layer has no arm API
or physical publisher; its opaque cells alone do not establish valid notes,
source snapshots, audio assets or projection results.

The slice's falsifier is a reservation that exceeds its checked byte budget, an
ordinary write that consumes finalization storage, a sealed payload changed by
late attribution, a lost result when notification stalls, or entitlement reused
before source quiescence and current quality acknowledgement. Tests in
`tests/capture_reservations.rs` exercise exact byte boundaries, checked overflow,
every reserved region, ordinary/result exhaustion, two source watermarks,
explicit lateness including zero, pre/post-seal sticky quality, empty intervals,
stale identities and interruption without another callback. Unit tests in
`src/recording/tests.rs` check the owned layout, identity exhaustion, explicit
counter saturation and zero allocations/deallocations during hot operations.
`tests/render_loop_purity.rs` includes `src/recording/hot.rs` in its checked region.

Interruption before the requested start seals an empty selected interval at the
last valid boundary; the original requested interval remains separately readable.
All sources must still acknowledge quiescence. A source generation has one epoch,
consumed frontier, watermark and quiescence state throughout the store: a fence updates every
retained take sharing it, and new reservations inherit that state. Admission
cannot make past input available: a new window starting before any known source's
consumed frontier refuses before reserving storage. The frontier is retained
separately from the allowance-adjusted watermark, including when the watermark
saturates at zero. Quiescence
cannot be undone by reserving another take. Once no retained ledger names a
generation, first introduction must be newer than every previously introduced
generation. This conservative monotone admission rule prevents resurrection with
a fixed high-water mark rather than an unbounded retired-source registry.
Even an empty interval at epoch origin requires every source's explicit fence; an initial numeric zero is not an
acknowledgement. These cases must leave a result that can be read and discarded.

The result remains in its pool slot while notification carries only a lossy,
counted hint; enumeration recovers every sealed result. Borrowed reads neither
copy nor retire it. Explicit off-thread discard requires every synthetic source
to quiesce and the owner to acknowledge current sticky quality. The caller must
keep the store alive until its retained results are resolved. This serial model
does not prove concurrent publication, backend fences or whole-session shutdown.
The eventual publisher must attribute refusals to every retained interval and
charge all additional context, reordering, audio and projection allocations.

P09-S003 adds `recording::notes::SimulatedNoteRecorder`, behind `simulated-ingress`,
for a single serialized exact-input publisher with explicit zero lateness. Its
immutable arm context owns the fixture target/revision, interval, overdub/replace
selection, quantization selection, epoch, anchor and tempo map. Those selections
are retained intent; no projection or project mutation is enabled. Endpoint
mapping is not TAKE-INV-004's monotonicity certificate. Preparation charges all
source, tracker and context arrays; arm additionally charges every retained map.
An arm refusal consumes no result slot. Binding with a controller snapshot asserts
known empty physical key state; subsequent pre-arm input establishes held keys.
Unknown or invalid source state refuses arm. The note owner validates its own live
source registry, so bound sources may first participate in any order; the public
opaque store retains its conservative high-water admission. Count-in and
unselected sources cannot consume the outstanding `H` tracker reserve: exhaustion
invalidates the offending source rather than continuing ambiguous pairing.
Count-in tracks uncaptured keys and controls, then explicit start requires every
selected source's fence at that exact boundary. Cancelling count-in seals an empty interval after source fences; it does
not require source retirement. Loss during count-in also selects an empty interval,
with an `Interrupted` outcome and quiescence. For a never-started empty take,
source snapshots remain the arm-time state; they do not describe its empty end
boundary. A finalized session boundary constrains
future segments even when they select a fresh source. Session boundaries precede
source input at the same sample.

MIDI 1 notes, sustain and channel pitch bend are validated; unsupported messages
refuse. Each source generation has a checked sequence and occurrence counter.
Same-key releases pair FIFO independently of sustain, capture admission and the
supplied audition trace. The trace is synthetic evidence, not renderer integration.
Decreasing nominal time is retained and diagnosed without rewriting source order.
Below-fence input faults every retained matching interval before any pairing or
identity-capacity refusal can return. It updates physical pairing when capacity
permits, but is refused from ordinary capture. A known gap stops further ordinary capture.
An early stop waits for source fences and may still accept eligible pending input;
end-exclusive input cannot rewrite that segment's held or controller state.
Every admitted raw cell also reserves paired-release and synthetic-closure metadata.
This covers a shortened interval exposing more historical open notes than `H`'s
reusable live slots. Finalization rebuilds selected controller state and closures
without altering raw timestamps; `selected_records` distinguishes retained input
outside the final interval. Ordinary or held-note exhaustion cannot consume that
metadata. Paired-release observations also reach reserved onset metadata when the
ordinary release is refused; synthetic closures then report the known key state.
Refused in-interval sustain updates a fixed per-source observation summary.
`controls` remains the accepted capture-log state; `observed_pedal` additionally
accounts for refusals. Before or after the summary's time span the held state is
known. A backwards cut through that coalesced span reports `None` for affected
channels, including `SyntheticClosure::pedal_held`, rather than guessing the lost
intermediate state. No refused-event vector is allocated. A partial/interrupted
take remains owned and is not enabled for automatic projection or commit here.
An early stop may narrow behind a consumed source fence while preserving raw
out-of-selection input; arm cannot recreate that past input for a new take.
Work is bounded by prepared `E`, `H`, `N`, `R` and `S`; these serial
scans are not a measured production callback-time qualification.
Tracker or identity exhaustion invalidates the source, interrupts capture and
requires quiescence plus a fresh generation. Rebind transfers the retired source's
diagnostic snapshot to the off-thread caller, including the first late source/time.
Mapping changes require finalization and a new explicit arm/start; immutable
earlier contexts remain readable.

The slice's falsifier is changed raw timing or FIFO lifetime under a different
serial delivery partition or supplied audition result, an unreserved allocation,
a closure derived from post-end input, an arm against unknown source state, or
late quality applied only to the newest retained result. `tests/note_capture.rs`
checks MIDI validation, pre-arm/count-in keys, same-key and cross-source/channel
pairing, pedal and pitch state, timestamp anomalies, start/end/stop boundaries,
capacity interruption, retained quality, result quota, mapping changes and stale
fence refusal. One exact log is delivered with whole, 64, 256 and irregular source
fence intervals under three audition outcomes. An additional 39-frame partition
places an interior fence exactly on the sample-64 event. The test compares
source-local occurrence identity and raw timing; it does not establish physical
callback scheduling.
`src/recording/notes/tests.rs` checks aggregate byte boundaries including maps,
checked identity exhaustion and zero allocations/deallocations during start,
publication, overflow, sealing and late attribution. The purity checker includes
`src/recording/notes/hot.rs`. The workspace gate runs the debug checks. These
additional commands pass locally for the release checks; they are not new CI steps:

```bash
cargo test -p synth_engine_v2 --release --test note_capture
cargo test -p synth_engine_v2 --release --lib recording::notes::tests
```

This bounded slice does not discharge TAKE-INV-001/002 as a whole: concurrent
queues, nonzero-lateness reordering, physical synchronization, renderer audition,
whole-session shutdown, loop passes and project transactions remain
at their named consumer gates. The owner must retain the recorder until results
are resolved; notifications do not transfer its sole payload or free its quota.

P09-S004 adds `SimulatedNoteRecorder::project_notes` in
`src/recording/notes/projection.rs`. This off-thread consumer returns all derived
notes or a named refusal, leaving raw capture and its reservations unchanged.
It certifies every integer tick in the retained arm interval before lookup;
the capture-only arm API makes no promise that later projection will fit or succeed.
Checked admission covers the inclusive tick count, table, derived notes and
projection descriptor together with all recorder-owned storage. Both arrays
check requested and granted capacity against that aggregate budget. The result
borrows the recorder through its destructor, preventing another projection allocation,
mapping replacement, discard or a quality update while that view exists.
There is no substitute-map argument: the exact owned tempo map, rate, interval,
fixture revision, anchor, epoch and session remain attached to the take.
This serial borrow is not a concurrent worker or commit-custody protocol.

The two lower-bound searches implement nearest-frame selection and earliest-tick
plateau selection independently. Each endpoint reports its signed native
tick-resolution error. Optional grid snapping moves only the note start and
preserves the derived duration; unsnapped endpoints and raw input remain readable.
No physical latency compensation is applied. A nonpositive raw or projected
lifetime, invalid mapping or out-of-target snapped note refuses the whole result
with the occurrence identified. Durations follow the accepted FIFO key release
or the explicit synthetic stop closure; closure provenance remains visible.
Only onsets inside the final selected capture interval become notes.
Incomplete effective quality, captured sustain/expression and nonneutral initial
controller state refuse this note-only result. A partial/interrupted take needs
a future explicit recovery selection; no automatic salvage or controller
conversion is implied. The result is not serialized and applies no project edit.

The slice's falsifier is any disagreement with exhaustive nearest-tick selection,
an undetected interior inversion, use of a foreign mapping, a partial successful
result, lost controller material, changed raw timing or an allocation outside the
aggregate budget. `src/recording/notes/projection/tests.rs` compares real step
and period-ramp maps against every frame in each finite test interval, including
tempo boundaries and dense tick plateaus. Tests in `projection/table.rs` inject
an interior inversion and conversion failure and isolate both tie rules.
Other checks cover FIFO same-key duration under delivery partitions and changed
audition times, retained-map replacement, foreign session/epoch, finite range,
checked tick/byte admission, quantization, synthetic closure, invalid lifetime
and controller/quality refusal. An empty cancelled count-in keeps its unused
initial controllers without inventing notes. Compile-fail examples hold the
exclusive projection borrow through last use and actual destruction.
These tests discharge the finite exact-input lookup and
note-only projection subset of TAKE-INV-004; pass-aware loop projection, physical
compensation and canonical project output remain at their named consumer gates.

P09-S005 connects this recorder to one simulated output generation. The
[host I/O conformance record](spec-host-io-lifecycle.md#conformance-tests) owns the
lifecycle checks. Capture admission closes when that output quiesces; the recorder
freezes its selection against the minimum acknowledged source frontier and retains
raw out-of-selection input. Source shutdown acknowledgements cannot move that
boundary. Count-in with no acknowledged frontier seals an empty interrupted take.
The recorder's added host identity and interruption state are part of its charged
descriptor. Source-by-source finalization uses the existing terminal reserves.
Retained results survive output retirement and block replacement recording storage
until explicit quality-checked disposal and release. This serial integration does
not establish concurrent source queues, physical fences or session ordering.

P09-S006 adds an optional ordered capture-boundary lane in
`recording::notes::session`, available through the simulated host's capture
control. Its commands start a reserved take or end it with stop, disarm or panic
as the retained reason. These are capture operations; audible transport, renderer
panic, metronome and callback-driven boundary production remain later consumers.
The fixture driver explicitly dispatches each engine-epoch boundary. Arm is
off-thread preparation, remains outside the queue and reserves the immutable
context and result before a start can be offered.

Admission checks the epoch, active reservation, exact start time and monotone
command times. Equal-time commands retain offer order. Offers at or before
already consumed source input refuse; a fence beyond the requested time also
refuses the offer. This admission floor includes every source bound to the single
serial publication owner, including unselected sources. It is a conservative
fixture-wide ordering rule; only selected sources own the take's start/seal
fences. Source publication cannot overtake the first pending boundary.
A start stays pending until every selected source fences its exact start. An
ending boundary must run before an equal-time fence, so natural-end sealing
cannot erase its requested reason. Applying an end requests finalization;
sealing still waits for all selected sources. Pre-start held keys remain
uncaptured, and an end-exclusive release cannot rewrite the recorded closure.

The queue's explicit capacity includes one slot reserved against start traffic
for an ending command. A full queue refuses without replacing any accepted
command; it does not provide urgent insertion ahead of already offered future
commands. Fixed slots are charged to the existing capture byte ceiling before
allocation. Command identities include the capture session and a checked serial.
Dispatch returns one identified applied, refused or cancelled receipt, or a
waiting receipt that leaves the command pending. A permanently refused command
is consumed explicitly. Host loss cancels pending work without changing its
requested times, and storage release waits for those cancellation receipts to
be drained as well as for retained takes and source quiescence.

Dispatch declares its boundary time reached even when the requested transition
is refused; subsequent publication and arm cannot move behind that time. Refusal
leaves the take unchanged, not the driver's ordering frontier. An end beyond the
take's requested interval is admitted; if natural completion seals first, that
end returns `Refused(NotActive)` without altering the sealed result. An applied
end is still only a finalization request until sealing. S005's interruption rule
continues to replace an unsealed request with `Interrupted` at the minimum
acknowledged source frontier, retaining the accepted raw data and explicit loss
closures. Only an already sealed result retains its window and outcome on loss.

The falsifier is source input overtaking an accepted equal-time boundary, a lost
accepted command, a start consuming finalization's queue slot, a fabricated
source fence, an uncharged allocation, or a stale command affecting a new host
generation. `recording::notes::session::tests` checks ordering, count-in, each end
reason, queue wrap/full behavior, byte admission, command exhaustion, refusals
and zero allocator activity. `host::capture::tests` adds two-source start/end
fences and cancellation after device loss, terminal callback failure, retirement
and reconnection. The hot operations join `render_loop_purity`'s scanned region.
This serial fixture establishes no concurrent delivery or real-time execution
budget for the future runtime-session producer; ADR-0054's renderer shares are
unchanged.

The contract fails if a legal stall or interruption can lose accepted data,
change its original timing, apply partial replace, duplicate a retry, or require
allocation/blocking on a callback. Required checks at the remaining consumers are:

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
- Carry P09-S004's finite projection checks into runtime loop mapping, physical
  compensation and canonical project output without substituting a current
  context or losing the reported native tick error.
- Feed synthetic ordered pass boundaries at non-Q-aligned positions and record
  a held/sustained note across them. A release exactly on a boundary closes the
  original occurrence in the next pass. Contiguous projection has one onset;
  isolated carry-in projection refuses, and carry-out trim requires selection.
  Repeat against the retained boundary handoff when ADR-0065 capture is integrated.
- Interrupt audio without a final callback; stop at the last valid watermark,
  preserve source format and prefix, and never label a gapped asset complete.

The storage, exact-input, projection, host-interruption and ordered-boundary tests
cover parts of TAKE-INV-001/002/004/006;
none of the following invariants is discharged in full. The built arm context,
serial source ordering and FIFO pairing still need their first physical/concurrent
consumers' qualification. Runtime callbacks, loop passes, audio, physical compensation and
project commit remain first-consumer obligations. Physical timestamp/latency
evidence belongs to ADR-0022; no new EVD result is asserted.

| Invariant | Required future check |
|---|---|
| TAKE-INV-001 | Callback partitions, pre/post-seal late input, two-source fences and attribution retained through commit |
| TAKE-INV-002 | Physical/concurrent source synchronization, count-in, FIFO pairing and mapping-change integration |
| TAKE-INV-003 | Synthetic non-Q-aligned pass boundaries, crossing occurrence projection/refusal and pass exhaustion |
| TAKE-INV-004 | Runtime loop mapping, physical compensation and canonical output retaining the certified context and native tick error |
| TAKE-INV-005 | Revision conflict, failed asset, lost acknowledgement, exactly-once retry and complete undo |
| TAKE-INV-006 | Every capacity exhausted under worker/GUI stall; retained ownership and off-thread reclamation |

The [ordered host capture extension](spec-host-io-lifecycle.md#ordered-note-capture)
couples compiled Play/Stop with the same retained recorder under ADR-0062. Its
conformance record owns the source-queue ordering, allocation and audible-stop
checks. The older capture-only lane remains a separate fixture; its direct controls
cannot bypass the coupled owner.

## Unresolved questions

| Question | Blocking? | ADR or task |
|---|---|---|
| Physical mapping, compensation, arrival allowance and workload qualification | Before production recording; exact simulated inputs may proceed | ADR-0022 and Phase 9 capture qualification |
| Retained runtime loop capture-boundary handoff | Before live loop capture and Phase 9 exit; synthetic logs may test this consumer | ADR-0065; ordinary activation retains ADR-0055 |
| Canonical revisions, transaction service and undo storage | Before shipping the project-commit consumer | Phase 10A/10B, ADR-0035 |
| Durable assets and crash recovery | Before claiming persisted take/asset recovery | Phase 10A/10D |
| Additional input token schemes or controller replacement lanes | Before enabling those optional modes; current refusal remains | TAKE-INV-002, TAKE-INV-005 |
