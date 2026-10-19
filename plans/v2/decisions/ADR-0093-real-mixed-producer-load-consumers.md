# ADR-0093: Real mixed producer load consumers

| Field | Value |
|---|---|
| ID | ADR-0093 |
| Status | Accepted |
| Phase | 9 |
| Created | 2026-10-19 |
| Last reviewed | 2026-10-19 |
| Related | ADR-0023, ADR-0046, ADR-0054, ADR-0075, ADR-0081, ADR-0090, ADR-0092 |
| Supersedes | — |
| Amends | ADR-0075 and ADR-0081 for the two named mixed consumers; ADR-0090 excludes programmed Trim base changes |
| Superseded by | — |

## Decision

The experimental mixed owner admits two executable load consumers: a finite
repeating parameter program for global native Trim levels, and periodic clear
records for global native Delay history. These consumers exercise actual
publication and DSP effects. Synthetic operation charges cannot substitute for
an unavailable producer in a six-class measurement. Saved-project application
lowering does not enable either consumer.

Each parameter program has a stable program identity and stable authored node
and parameter addresses. This first consumer accepts only nonnegative Trim
levels; the general amplitude type also permits signal inversion. Admission
resolves every address in the actual plan.
The compiler sorts programs by stable identity and refuses duplicate identities,
shared targets across programs and duplicate phase/target entries. Periods are
positive `FrameCount` values; phases are `FrameCount` offsets strictly inside
the period, measured from absolute `SampleTime` zero. Compiled Start, Stop,
Seek, loop mapping and note-only Panic do not change that phase.

A fresh owner publishes each target's latest periodic value at its first
rendered sample, through the same AuthoredRuntime arena as subsequent changes.
It skips phase entries at that first sample because initialization already
contains them. The conservative per-quantum envelope is the number of distinct
targets plus entry count times `ceil(Q / period)`. This covers initialization
and every phase alignment. Programs retain zero future events and zero release
holds, and borrow no note identity entitlement. The aggregate authored
admission includes these parameter-only envelopes beside note YAMS envelopes.
Events at one sample follow stable program order; all targets of one program
remain contiguous. At equal samples the mixed owner publishes Session and
compiled/Panic-cut work before these authored programs, then live work; internal
controls follow the sealed external events. Authored parameter stamps use exact engine-authored
provenance. This extends the existing Authored source beyond note output.
Trim remains quantum-rate: sub-quantum writes count individually, while audible
claims follow its existing next-boundary composition law.

An internal declaration names one actual global native Delay and its positive
period and phase. Duplicate node declarations refuse. The compiler derives
`ceil(Q / period)` for each emitter and sums these bounds against Internal;
callers cannot supply an arbitrary quota. The plan retains resolved emitters.
After external composition, the renderer generates current-quantum records in
a separate fixed arena. A clear happens only because a record reaches the
Delay kernel's positioned reset control. The derived per-node control run is
sorted by offset, with external controls before internal controls at equal
offsets and internal declaration order equal to stable node order. No internal
record mutates sealed external input or survives its generating quantum.
An arena or combined-total overrun ends the epoch: silence the complete current
and later callbacks, invalidate both carries and publish `needs_reprepare`.
Internal faults increment their own internal-producer counter. Authored envelope
faults increment the authored-source counter; the publication counter denotes
an actual external-arbiter failure. Occupancy counts accepted records, excluding
a refused attempt, including a partially completed failing quantum.

Both target classes must be strictly downstream of voice sums. The cold proof
refuses modulation, explicit feedback, scripts and controller bindings in these
first-consumer plans, then follows actual preceding audio writers, accounting
for reused buffer regions. Any affected Local or SharedSum step refuses.
A post-sum master gain or a tail containing live sound affects the mixed signal;
it does not write a live voice instance. Existing mixed binding still refuses
compiled parameter/controller writers and note-authored sources. Ordinary
non-mixed preparation and the exclusive note YAMS owner refuse these programs,
so an unsupported owner cannot silently omit them.

Fixed-geometry continuation requires identical programs, resolved geometry and
programmed target bases. It keeps actual initialization state, overrides and
absolute phase. A programmed Trim base change refuses cold as a plan-shape
change; it cannot become a runtime Storage fault or overwrite the current phase
value. Structural producer continuation refuses. An explicit reset uses two
independent owners at the same absolute sample phase and the existing fade.
Immutable templates and resolved arrays are charged by retained extent; the
internal arena and extra timed controls are charged to prepared scratch.

Periodic clears zero both complete native Delay history lines. Their admitted
work bound is the sum of `2 * prepared line length * per-quantum emitter bound`
sample writes. The compiler reports this checked bound and emits an
`InternalClearCostUnqualified` advisory. This work is explicitly excluded from
`predicted_quantum_cost_ratio`: no calibrated coefficient exists, and the
ordinary node estimate is not a CPU bound for these load consumers. The
calibration runner reports the admitted sample-write bound beside actual
callback timings. Event/storage admission cannot establish a callback deadline.
Normal downstream enabling remains forbidden pending ADR-0054's cost and
occupancy evidence. This recorded exclusion is part of the software scope;
finite work does not mean a production deadline is met. During reset fade,
the independent old and new owner bounds add; neither owner borrows credit.

A report represents availability explicitly. An absent or never-observed new
producer remains unavailable; an installed, observed idle producer can report
zero. Per-class peaks are independent. Combined occupancy is the actual sealed
external total, including Session operation charges, plus that quantum's
internal records. A callback serving carry opens no new observation. Neither a
sum of class peaks nor an unopened zero row is a simultaneous measurement.

The deterministic controls and calibration runner establish executable
mechanisms and software bounds, not representative data-dependent occupancy.
ADR-0054 still requires occupancy and retain-or-reselect evidence before
downstream enabling, simultaneous six-share selection, and a representative
production workload. No production capacity, physical timing or native
quiescence is qualified here.

## Falsifiers and stopping rule

Acceptance fails if suppressed authored publication still changes the programmed
gain, or suppressed internal records still clear a warm tail. A real emitter's
first record above its admitted bound must silence the entire callback and
later callbacks. Multi-target equal-phase programs must remain contiguous in
stable declaration order. A fresh phase value, compatible continuation and
reset must follow the same absolute sample origin. Actual mixed controls must
observe all six executable classes, attribute same-quantum totals, preserve
protected releases and measure no callback allocation or destruction.

A contradictory law, missing class reported as zero, lost owner, unbounded work
or real-time allocation blocks acceptance. Optional implementation detail does
not. Physical qualification and production limit selection remain separate.
