# Core V2: Current Work

Last updated: 2026-08-25

This is the only authority for active Core V2 task state, blockers, and next actions. Durable reasoning and measurements
live in the linked ADRs, specs, and EVDs rather than being repeated here.

## Phase 2 is closed

**[`REV-P02`](reviews/phase-02-exit-review.md) is `Accepted`**, so Phase 2's state in
[`ROADMAP.md`](ROADMAP.md#phase-order) is `Complete` and every P02 task is closed. All six master-plan gate bullets
close. The review carries the gate table and the figures; only the consequences are stated here.

- `Q` = 64 is **confirmed** ([EVD-0012](evidence/phase-02/EVD-0012-render-quantum-real-path.md)), so the restriction on
  hand-unrolled kernels, `Q`-specific buffer layouts and tests asserting a control rate in hertz **is discharged**.
- Equivalence takes the gate's **second branch** — a documented intentional difference. **No `CORPUS-0001` preserve
  claim is broken** ([EVD-0013](evidence/phase-02/EVD-0013-minimal-patch-equivalence.md)); the envelope shape is
  `CORPUS-0001-C2` under [ADR-0042](decisions/ADR-0042-envelope-segment-shape.md).
- CPU closes with **no adapter margin claimed**
  ([EVD-0014](evidence/phase-02/EVD-0014-minimal-patch-cpu.md)).
- P02-T006's dropped `synth_dsp` extraction is **registered** in the review's deviation table, with ADR-0040 clause 5 as
  its acceptance basis. That debt is paid.

The review's *Deviations and residual risks* table is the register for everything Phase 2 leaves behind; it is not
repeated here. Two items are owed to later work rather than to nobody, and both are named there: **a note's pitch and
velocity** (Phase 3's ingress is the first consumer — `NoteEdge` carries neither today), and **`SOUND-INV-012`'s second
sentence**, the one invariant with no executable check.

## Active stream: Phase 3 sample-accurate scheduling

Phase 3 is `Active`. It was activated on 2026-08-25 after Phase 2 closed and the maintainer corrected the phase
boundary: Phase 3 consumes events that are already expressed as the current epoch's `SampleTime`; it does not need a
physical device-clock mapping to schedule them. [ADR-0022](decisions/ADR-0022-hardware-time-mapping.md) remains
`Deferred`, but its retained release-platform, adapter-clock, arrival-fallback, and replacement-mapping evidence now
gates Phase 9 exit and any qualified live-timing claim rather than Phase 3 entry.

### Completed bounded slice — compiled note across host partitions

The first implementation slice had one observable completion boundary:

- a compiled-event scheduler presents only events belonging to quanta that the next actual renderer call will produce,
  rejects an epoch mismatch or a schedule that would present an event late, and performs no real-time allocation;
- one note with non-quantum-aligned on and off positions is rendered through actual host-call families `1 x 4096`,
  `16 x 256`, `64 x 64`, and a predeclared irregular partition of the same 4,096 frames;
- all four outputs are bit-identical, and both edges occur at the requested `SampleTime` plus the renderer's declared
  constant `Q` live-output carry, with no late-event diagnostic.

This slice does not add live or arrival-time ingress, a CPAL or MIDI mapping, tempo/session ordering, or final producer
share and capacity values. Simulated timestamped ingress equivalence remains Phase 3 exit work: the same `SampleTime`
sequence presented through a deterministic simulated-ingress producer must equal the precompiled sequence. Physical
hardware equivalence is deliberately not claimed by that test.

That boundary is met by the compiled scheduler in
[`schedule.rs`](../../crates/synth_engine_v2/src/schedule.rs) and its actual-callback conformance test in
[`compiled_schedule.rs`](../../crates/synth_engine_v2/tests/compiled_schedule.rs). The next Phase 3 implementation
slice is deliberately not selected by this completion update.

### Completed slice — ADR-0046's producer shares

`beddf91b` adds the seven ground-2 profile fields ADR-0046 clause 1 creates, with the plan-independent relations
profile construction can decide: the share sum against `max_events_per_quantum`, positivity per field,
`release_event_share >= release_hold_capacity`, and `max_scheduled_events_in_flight` against `compiled_event_share`
over a derived `max_quanta_per_callback`. `QuantumCount` exists so that derived value carries its unit.

Two consequences of the partition are load-bearing and were found by existing tests rather than predicted: six
positive shares cannot fit a cap below six, so a `max_events_per_quantum` of 1 or 4 is no longer representable; and
the compiled floor makes a very small `max_scheduled_events_in_flight` unconstructible.

The defaults are provisional. A following slice registered the one live renderer-ingress store and implemented the
live share's ingress lower bound against it, so **two** obligations stay open in the host-profile specification's
deferred list and are named there rather than repeated here: the sealed-batch store's coverage of its extent, which
needs the arbiter's store; and the measurement that must reselect the partition, the cap and the ingress depth before
live ingress.

### Approved: `synth_engine_v2` API breaks during Phase 3

The maintainer approved, on 2026-08-26, the API breaks the producer-share and ingress-store slices make to
`synth_engine_v2::profile`: `EventLimits::new` changed signature twice, and `command_queue_capacity` and
`event_egress_capacity` moved behind `events().queues()`. `AGENTS.md` requires explicit approval for an API break,
and the first of those breaks was committed in `beddf91b` before the approval was sought — recorded here rather than
left implicit.

The approval is bounded by what it was given for: this crate is experimental and is not a dependency of the
workspace's default members, so it has no in-repo consumer outside its own tests. It is not a standing licence for
persisted, manifest, wire or protocol contracts, which `AGENTS.md` treats separately, and it does not reach any other
crate. ADR-0020 settles the final crate boundaries and names.

### Drafted, awaiting its specification transaction — ADR-0047 note identity

[ADR-0047](decisions/ADR-0047-note-identity-in-the-event-contract.md) is `Proposed` and has had its independent
design review: five rounds, twenty-three blocking findings, and a clean confirmation read on the final scope. No code
was written against it.

It exists because ADR-0046 clause 3 already promises that an orphan note edge "is counted rather than allowed to
release another note", and the current `{ slot, edge }` vocabulary cannot distinguish an orphan from a legitimate
release. Identity is therefore a Phase 3 requirement rather than preparation for Phase 6.

Two consequences bind later work rather than this slice:

- **The record was split at the fifth round.** Rebuilding an identity table rejects every identity from the outgoing
  one, but a note from that table may still be sounding, and refusing its release would contradict ADR-0046 clause
  3's guarantee. ADR-0047 clause 8 refuses the *rebuild* while an obligation is outstanding, which is sufficient for
  Phase 3; the transition itself is registered as **ADR-0048** for Phase 9 beside ADR-0009's plan swap.
- **A new non-reissuing issuer is required**, for the identity table. Neither existing value scopes an identity:
  a re-admission changes the plan but not the epoch, and a re-preparation changes the epoch but not the plan.

Acceptance is not taken here, because it owes a specification transaction: the index relation beside ADR-0046's share
relations, `HOST-INV-021`'s hold contract naming the identity, and an **amendment** of `HOST-INV-009`, whose closed
list licenses two live-input drop causes and states there are no others — an exhausted identity range is a third.

**REV-P02's `NoteEdge` deviation row is not discharged by that record and keeps the owner and deadline
[REV-P02](reviews/phase-02-exit-review.md) gave it: Phase 3, owed before ingress.** ADR-0047 adds one obstacle to it
rather than resolving it — the row's pitch limb is coupled to ADR-0025, which is `Proposed` and targets Phase 6, so
Phase 3 must either accept ADR-0025 early or change REV-P02's disposition explicitly. The velocity limb carries no
such coupling. That choice is open and belongs to the maintainer.

[ADR-0046](decisions/ADR-0046-destination-quantum-admission.md) is `Accepted`. It replaces capacity deferral with
pre-render admission:

- one real-time publication arbiter is the sole normal writer of sealed destination-quantum batches;
- compiled, authored-runtime, live, session, internal and guaranteed-release events use disjoint checked shares;
- compiled schedules and authored destination, future-storage and release-hold envelopes are admitted before playback,
  while complete eligible live and session snapshots fit their own shares;
- every non-compiled note-on that creates a later release obligation acquires its source-storage and
  renderer-capacity hold atomically;
- the renderer never moves an event for capacity, and an impossible over-full sealed batch terminates the stream
  instead of producing partial timing.

The predecessor same-control and cross-control questions are `Superseded`, dissolved rather than answered: both
required selective `+Q` capacity movement, which no longer exists. ADR-0043's preserving late clamp remains in force
for genuinely late events. For one publication boundary its `max(stamp, boundary)` mapping is monotone: it can create
a same-position tie, whose ordering is `ADR-0023`'s, but cannot reverse two accepted events.

The numeric fixed-share values, ingress capacities, release-hold capacity and callback cost are Phase 3
implementation evidence, not unresolved policy. Profile construction checks plan-independent relations; plan
admission checks runtime, session, internal and hold declarations without changing those shares. The current
`max_events_per_quantum = 256` has no claim to be useful and must be reselected from the measured partition before
live ingress is enabled, even if that partition would fit within 256.

## Paused parallel stream: Phase 0B

Outcome: complete the V1 migration inventories and the durable Project and Application Core contracts required before
Phase 10.

This phase remains active in the roadmap, and its execution stream is paused.
It was paused for the Phase 2 slice, which has now closed, so **nothing is
holding it any longer** — resuming it is a choice rather than a wait. On
resumption, select exactly one task, copy its observable completion check here,
and only then mark it `Active`.

| Task           | State       | Resume boundary                                                            |
|----------------|-------------|----------------------------------------------------------------------------|
| P00B-T001      | Paused      | Assign evidenced dispositions in the state-ownership inventory             |
| P00B-T002      | Paused      | Assign reachability and migration dispositions in the capability inventory |
| P00B-T003      | Paused      | Resolve the two format questions that block ADR-0014 review                |
| P00B-T004–T007, P00B-T009 | Not started | Follow the decomposition in the frozen execution record       |
| P00B-T008      | Not started | Re-scope the frozen all-ADR task under `PROCESS.md`'s decision-timing rule  |

This stream does not block Phase 2. Its detailed audit chronology remains in
the [historical Phase 0B execution record](phases/phase-00b-inventories-and-project-contracts.md); new operational state
is recorded only here.

Phase lifecycle and completed gates are recorded once in
[`ROADMAP.md`](ROADMAP.md#phase-order).

## Later owned work

- Phase 3 owns renderer ingress, the publication arbiter and producer shares,
  event scheduling, and capacity measurements. Its exit work
  also owns ADR-0043's named offline late-clamp test:
  prove the stamp-window selector cannot present a late event, or window by
  clamped render position. That test is not an entry prerequisite.
- Phase 4 owns current-project lowering and the long-running job contract.
- Phase 5 owns the `LegacyPolyModuleAdapter`'s conversion cost — the largest quantity ADR-0041 moves and the only one
  nobody has measured — and the declarative node API that `SOUND-INV-012`'s uncovered second sentence belongs to.
- Phase 9 owns completion and acceptance of ADR-0022 against retained evidence for every claimed release platform and
  initial adapter. Phase 9 may build candidates while the record is `Deferred`, but cannot exit or qualify live timing.
- Phase 0B gates Phase 10.
- ADR-0039 and `LIMIT-0017` remain Phase 10E work.
