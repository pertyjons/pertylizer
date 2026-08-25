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

## Next stream: Phase 3 is not yet activatable

Phase 3 remains `Not started`; Phase 2 is closed. Phase 3's only remaining entry prerequisite is
[ADR-0022](decisions/ADR-0022-hardware-time-mapping.md), which still needs the simulated-host evidence and
release-platform callback measurements named by that record. Phase 3 may not invent timestamp or latency semantics
while building the harness.

That evidence slice is the next pre-entry work. Building the harness and taking measurements does not activate
scheduler implementation; accepting ADR-0022 against those results does. This is why the harness is on the critical
path even though its durable owner is Phase 3.

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
Phase 3 is enabled, even if that partition would fit within 256.

Activating Phase 3 remains the user's call after ADR-0022 is `Accepted`.

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
  event scheduling, capacity measurements and ADR-0022's simulated-host
  harness. Its exit work also owns ADR-0043's named offline late-clamp test:
  prove the stamp-window selector cannot present a late event, or window by
  clamped render position. That test is not an entry prerequisite.
- Phase 4 owns current-project lowering and the long-running job contract.
- Phase 5 owns the `LegacyPolyModuleAdapter`'s conversion cost — the largest quantity ADR-0041 moves and the only one
  nobody has measured — and the declarative node API that `SOUND-INV-012`'s uncovered second sentence belongs to.
- Phase 0B gates Phase 10.
- ADR-0039 and `LIMIT-0017` remain Phase 10E work.
