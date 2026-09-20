# ADR-0069: Simulated input clock and capture

| Field | Value |
|---|---|
| ID | ADR-0069 |
| Status | Accepted |
| Phase | 9 |
| Created | 2026-09-20 |
| Last reviewed | 2026-09-20 |
| Related | ADR-0022, ADR-0036, ADR-0067, ADR-0068, IO-INV-004, IO-INV-005, TAKE-INV-003 |
| Supersedes | — |
| Superseded by | — |

## Boundary and decision

`host::input` supplies an exclusively borrowed, finite simulated note-input owner
and `InputCaptureSession`, a serial composition with ADR-0067's loop recorder.
Each selected input retains its endpoint, independently issued connection generation,
immutable synthetic clock, prepared observation cells and first discontinuity.
The common checked issuer supplies distinct values for input connections and recorder
source bindings. Neither identity is an array index. The composition admits exactly
the armed source set and fresh empty session lanes. No mutable inner owner escapes.

Preparation and explicit start are separate. Loss closes admission and requests
quiescence. Retirement requires quiescence and collection of every input outcome;
an explicit attempt at the same endpoint obtains another generation and successful
preparation returns to Ready. Stale readiness, messages and loss cannot mutate that
replacement. There is no automatic retry, fallback, replay or transport restart.

This consumer adds simulated independent-clock merging to ADR-0067. ADR-0068 remains
the separate concrete thread-transfer proof; these serial input owners are not a
concurrent input queue or a physical backend fence. Audio input, monitoring, live
audition, restart and production timing retain their first-consumer gates.

## Synthetic clock and explicit prefix contract

The clock is a configured oracle over synthetic ticks, not an estimate from hardware.
It declares an engine epoch and frame origin, a source tick origin, a positive rational
number of engine frames per tick, and symmetric tick uncertainty. This finite consumer
requires engine origin zero; source origins and rational rates may differ. Checked
integer arithmetic floors into half-open frame buckets. Only an inclusive uncertainty
interval wholly inside one bucket yields an exact simulated nominal frame. Underflow,
overflow and ambiguous intervals refuse admission and retain the offending observation.
No accepted event is moved by a later correction. ADR-0022 remains Deferred, with its
physical evidence method and qualification unchanged.

A message retains its source tick and exact simulated engine arrival. The oracle gives
its nominal frame; arrival must be at or after nominal. Source ticks and arrivals may
not regress. A frontier F is a mapped engine-frame value with **two explicit promises**:
all earlier messages from that source have arrival strictly below F, and every future
message has nominal frame at or above F. Admission verifies the first promise against
accepted arrivals and refuses any later violation of the second. Advancing F must be
strict. The initial exact fence at zero is a declared synchronization prefix at the
oracle anchor, separate from uncertain message observations.

The merger orders by **arrival**, not nominal time. It uses the minimum admitted F
across every source, emits messages with arrival strictly below that minimum, and
emits fences at or below it. Equal-time fences precede messages; prepared port order
and input occurrence serial settle remaining ties. A source can offer a message
exactly at its frontier, but it stays queued until every source advances beyond it.
Empty queues and worker progress never manufacture a frontier. This withholding is
an ordering guarantee, not a delivery deadline guarantee: a silent peer may keep a
valid message queued until acknowledged audio passes its arrival. Later delivery then
retains a refusal and interrupts, even if that producer honored every frontier promise.

For example, independent messages `(nominal 10, arrival 40)` and `(nominal 20,
arrival 30)` merge in arrival order. A later fence at 100 is valid after those
arrivals; another message with nominal 120 and arrival 140 remains valid. A fence
at 20 after arrival 40, or a subsequent nominal 90 after fence 100, is refused.
Thus delayed input is supported within explicit prefixes, not under arbitrary
latency or violated completeness promises. Recorder lateness remains zero.

## Storage, delivery and loss

Every accepted observation has a checked generation-scoped occurrence ID and a fixed
cell retained until explicit receipt collection. Message admission reserves one cell
for a frontier. Completed unread outcomes still occupy cells. Exhaustion retains the
first refused observation separately, cancels unsent accepted cells with their original
observations and IDs, and interrupts the take without evicting its raw records.

The off-callback merger maps each forwarded cell to its actual serial source receipt.
It locates that cell before consuming the receipt. A full serial source lane is
backpressure: the original cell remains queued for a later worker turn. Other admission
refusals retain the exact error and observation. Consumed refusals, Late and capacity
stops retain the actual recorder outcome and close input admission. No refusal is
silently retimestamped or retried as another successful occurrence. An exact mapped
refusal behind an actually consumed source fence also updates the recorder's existing
late-quality attribution, including a normally sealed take before source closure.
A refused uncertain message whose possible frame interval overlaps a retained selected
interval records `first_uncertain_source` in its bounded quality slot. This makes an
otherwise Complete result effectively Partial without inventing an exact late timestamp.
An interval wholly outside the selection does not mark it. For a ClockRange refusal,
quality attribution intersects the possible tick interval with the oracle's valid tick
and engine-frame domains. A surviving overlap marks the same conservative quality
slot, including uncertainty extending before the clock origin. This diagnostic
intersection never admits clipped input or constructs a timestamp; admission still
refuses the original observation.

An already admitted Stop remains independent of input progress. After Stop, delayed
fences can complete a take without another callback. The existing recorder also
advances its publication floor to Stop: an older queued arrival delivered afterward
is explicitly refused, retained in the input receipt, and interrupts this composition.
The same applies to source admission behind acknowledged audio before Stop. This
consumer does not promise complete takes under unbounded worker stalls.

Device loss interrupts with DeviceLost; timing, capacity or delivery discontinuities
interrupt with SourceInvalid. The selection freezes at existing acknowledged audio
and actually consumed source frontiers, never notification time. Accepted raw outside
a shortened selection remains in the recorder; unsent/refused observations remain
in input receipts. Each input must explicitly acknowledge simulated quiescence before
interrupted finalization. No final audio callback is required. A callback attempted
after closure produces silence and cannot advance the acknowledged snapshot. A
previously successful whole callback is never retrospectively silenced.

Normal sealing precedes `close_completed` and independent input quiescence. Release
requires every input outcome and core receipt to be collected, and returns both the
intact recording session and the input owners. Reconnection cannot erase the old take.
Failed composition or release returns all owning values. Each input byte ceiling
charges its inline owner and fixed cells; endpoint strings are off-thread settings.
The composition charges its additional inline metadata separately from existing
recording, journal and session budgets. Callback paths allocate and deallocate nothing.

## Falsifiers and acceptance

The tests in `recording::notes::loop_capture::tests::ordered::input` compare actual
audio, raw records and normalized input receipts under whole, 64-frame, 256-frame and
irregular callbacks and opposite inter-source submission orders. Hand-authored rational
clock cases, shifted source origins, arrival-inverted nominals and valid delayed input
between interior fences check the time-domain distinction. Equal-frontier withholding,
invalid frontiers, uncertain input, exhaustion, queue pressure, stale callbacks,
late delivery, no-final-callback loss and retained-owner recovery check refusal paths.
Input unit tests check identity exhaustion, preparation failure and clock regression.
Allocator/deallocator guards and the static hot-region check cover callback operations.
Exact byte ceilings admit and one-byte-short ceilings refuse without losing owners.

A false timing claim, hidden eviction, missing outcome, stale-generation mutation,
callback allocation or unfillable lifecycle contract blocks acceptance. Optional
implementation detail does not. The independent review covers this final contract
and code, including the design consultation's nominal/arrival frontier distinction.
