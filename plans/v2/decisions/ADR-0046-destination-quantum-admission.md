# ADR-0046: Destination-Quantum Admission Without Renderer Deferral

| Field | Value |
|---|---|
| ID | ADR-0046 |
| Status | Accepted |
| Phase | 3 |
| Created | 2026-08-24 |
| Last reviewed | 2026-09-01 |
| Related | [ADR-0043](ADR-0043-event-deferral-and-late-clamp.md); [ADR-0044](ADR-0044-deferral-causal-order.md); [ADR-0045](ADR-0045-cross-control-causal-order.md); [ADR-0054](ADR-0054-staged-producer-capacity-calibration.md); [ADR-0055](ADR-0055-refuse-unimplemented-loop-playback.md); ADR-0021 parts 1 and 2; ADR-0032 clauses 17, 21, 23 and 27; [EVD-0015](../evidence/phase-03/EVD-0015-quantum-occupancy.md); `HOST-INV-010`; `HOST-INV-011`; `HOST-INV-018`; `HOST-INV-019`; `HOST-INV-021`; `SOUND-INV-016` |
| Supersedes | **[ADR-0043](ADR-0043-event-deferral-and-late-clamp.md)'s capacity-deferral rule; [ADR-0044](ADR-0044-deferral-causal-order.md) and [ADR-0045](ADR-0045-cross-control-causal-order.md) in full, dissolved rather than answered.** ADR-0043's preserving late clamp, immutable stamp, control-response rule and prohibition on applying an event to produced samples remain in force |
| Superseded by | [ADR-0054](ADR-0054-staged-producer-capacity-calibration.md) supersedes only the Phase 3 numeric-selection deadline; [ADR-0055](ADR-0055-refuse-unimplemented-loop-playback.md) supersedes clause 4's runtime-adoption sentence while loop playback is unsupported; every safety relation and ownership rule here remains accepted |

This record meets `PROCESS.md`'s durable-decision test because it moves a real-time ownership boundary, binds several
later phases, and reverses delivered timing behaviour if changed. It fixes relations, not numeric values.

> **Accepted on 2026-08-25: upstream admission, one publication arbiter, and no movement for capacity.** This
> dissolves ADR-0044 and ADR-0045: both hazards require selective `+Q` movement, and that mechanism no longer exists.

The surviving late clamp does not recreate those hazards. Within one publication pass its boundary `B` is fixed and
the mapping `render_position = max(stamp, B)` is monotone: it may collapse stamp order into a same-position tie, whose
ordering belongs to ADR-0023, but cannot reverse it. Across passes a later boundary means an event actually arrived
late; ADR-0043 and ADR-0022 own that case. ADR-0044 and ADR-0045 instead exist because capacity deferral could move one
already accepted, on-time event by `+Q` while leaving its later dependent event in place.

## Durable boundary

ADR-0043 made an over-full renderer quantum choose which event moves. ADR-0044 then established that no surveyed
selective movement preserves every causal relation, and ADR-0045 established that the same failure crosses controls.
The choice is therefore not an ordering algorithm. It is where overload is owned:

- keep it in the renderer and preserve a total order by moving a larger stream; or
- admit work before rendering, so a renderer quantum is never asked to choose.

This record chooses the second. Reversing it later would change rendered timing and introduce a new hot-path store, so
the boundary belongs in an ADR rather than in a Phase 3 implementation detail.

## Decision boundary

### What this record decides

1. A single real-time publication arbiter is the only normal path that constructs renderer input.
2. Every renderer event is charged to exactly one prepared producer-class share in its destination quantum.
3. Compiled and authored-runtime work are admitted before playback against bounds that make runtime refusal
   unnecessary.
4. A note-on that creates a future release obligation acquires the corresponding hold atomically.
5. The renderer never delays, defers, reorders, trims or drops an event for capacity.
6. An over-full renderer quantum is an invariant violation, with a terminal stream response.

### What this record does not decide

- The numeric event cap, share values, ingress capacities or release-hold capacity. Phase 3 measures useful defaults;
  the relations below make every constructed profile safe at any values that satisfy them.
- Same-sample ordering (ADR-0023) or hardware-to-engine time mapping (ADR-0022).
- Voice allocation. This record requires a bounded mass-release event; Phase 6 decides how the allocator applies it.
- Persisted or wire representation. `HostProfile` remains absent from project documents under `HOST-INV-010`.

## Evidence

- At `ee574fe3`, `render/hot.rs:53-68` rejects an event span whose total work exceeds prepared scratch,
  `render/hot.rs:145-160` rejects a quantum whose tally exceeds `max_events_per_quantum`, and
  `render/hot.rs:163-172` has a second refusal at the scratch write. The renderer already has a bounded refusal path;
  it has no deferred store.
- The same source filters stale epochs, foreign plan slots and out-of-horizon ingress before the per-quantum tally
  (`render/hot.rs:85-117`) and preserves the stamp while late-clamping the render position (`render/hot.rs:123-132`).
  Validity filtering, lateness and capacity are three different decisions and remain in that order.
- [EVD-0015](../evidence/phase-03/EVD-0015-quantum-occupancy.md) finds 36 events in the busiest measured quantum, but
  disproves unreachability by number: at 8 kHz and 200 BPM one permitted active placement can produce 3,328 onset
  edges from tick instants inside one quantum. That is a current-quantum onset contribution, not a total-occupancy
  upper bound: simultaneous placements and releases produced earlier can add to it. Occupancy is a sum; independently
  bounding one producer does not bound it.
- The graph-sizing premise has a constructive answer in the current source.
  `crates/synth_sequencer/src/note_graph.rs:1494-1536` evaluates one tick into caller-owned buffers without
  allocation, walking the spine in topological order. Source-context nodes are evaluated on that same path rather than
  passed through: `note_graph.rs:1678-1701` builds their stack-only upstream view and `note_processor.rs:1239-1244`
  dispatches the arpeggiator and the strummed chord from it. The `expand_at_tick` doc comment at
  `note_graph.rs:1504-1506` still describes them as pass-through and is stale; the wiring it defers to a later phase
  exists. `crates/synth_sequencer/src/note_processor.rs:1477-1559`
  materializes that output in a fixed-capacity `ExpansionBuffer`, exposes the complete slice and records overflow.
  `crates/synth_engine/src/sequencer_engine.rs:861-883` already evaluates first and consumes the materialized slice
  second. V2 must turn the old overflow drop into plan refusal or a producer-contract fault, but it needs neither a
  second graph evaluation nor pessimistic publication of 128 events. Publication cost remains Phase 3 measurement,
  not a feasibility premise.

**What could falsify the selection.** A producer class with neither a finite admission bound nor an allowed refusal
before authored playback would make the partition incomplete. A required producer that cannot route through the
single arbiter or declare a finite internal-emission bound would do the same. Either finding reopens this record.

## Options

### Status quo: selective `+Q` deferral

Rejected. ADR-0044's survey selected no repair, and ADR-0045 shows that the inversion is not confined to note pairs.
Preserving the complete total order would instead move an ever-larger suffix and turn overload into timeline slippage.

### Selected: bounded admission before rendering

Every source either owns a destination entitlement, fits a prepared runtime envelope, waits in bounded source storage
until its destination enters the publication horizon, or is refused at a boundary where refusal is allowed. An
accepted due event never waits for destination capacity. A renderer receives only sealed, bounded quantum batches.

This adds no deferred store and makes producer defects attributable. Its cost is conservative admission and a larger
event capacity than the unevidenced current value of 256 may permit.

### Lossy renderer fallback

Rejected. ADR-0021 permits counted loss for live bounded input before renderer admission, but forbids trimming authored
note expansion. A renderer cannot infer which automation point, note edge or state transition is semantically safe to
remove.

## Decision

**Select bounded destination admission and remove capacity deferral.** The following clauses are one contract.

### 1. Fixed profile shares make the sum safe

`max_events_per_quantum` contains six disjoint shares:

1. compiled timeline and automation;
2. authored runtime expansion;
3. live ingress;
4. session and transport;
5. renderer-internal production; and
6. guaranteed releases.

The profile fields are `compiled_event_share`, `authored_runtime_event_share`, `live_event_share`,
`session_event_share`, `internal_event_share` and `release_event_share`, all `EventCount`s.

Any difference between their checked sum and `max_events_per_quantum` is unusable safety slack. It is not an unnamed
seventh share and cannot be borrowed on the audio thread.

Every event is charged to exactly one share. The shares are profile inputs. `HostProfile` construction validates, with
checked `EventCount` arithmetic, the sum and the plan-independent relations below. Plan admission validates the
compiled plan, runtime-source declarations and prepared session expansions against those fixed shares; preparation
allocates the admitted extents but derives or mutates no share. Shares are fixed for the stream epoch and never
borrowed or renegotiated on the audio thread. Every share is a positive `EventCount` capacity, including in a profile
that disables one of the optional producer classes. Such a class leaves its entitlement unused; another producer may
not reclaim it while the stream runs.

The following lower bounds and storage relations are mandatory:

- the live share covers the sum of the renderer-ingress queue snapshots eligible for one publication pass;
- `max_scheduled_events_in_flight` covers `compiled_event_share` multiplied by
  `max_quanta_per_callback = ceil(maximum_block_size / Q)`;
- `release_event_share >= release_hold_capacity`, so every outstanding non-compiled hold can redeem an individual
  release into one destination quantum; and
- the preallocated sealed-batch store covers `max_events_per_quantum * max_quanta_per_callback`.

Every multiplication and conversion is checked during profile construction and preparation. `max_quanta_per_callback`
is a derived `QuantumCount`, not a raw count smuggled through the API. Carry can reduce the number of quanta a
particular callback renders but cannot increase that maximum.

Plan admission then rejects unless:

- the compiled destination-occupancy envelope and the plan-wide aggregate authored destination envelope fit their
  respective shares;
- the compiled callback window fits the derived compiled floor `compiled_event_share * max_quanta_per_callback`,
  and the plan-wide aggregate maximum of simultaneously retained authored future events fits the headroom above that
  floor. The store is partitioned exactly as the shares are: a sparse compiled plan leaves its floor unused rather
  than lending it to authored retention;
- the internal share covers the sum of every admitted internal producer's declared per-quantum maximum, which is a
  complete bound only because clause 2 confines an internal emission to the quantum that generates it;
- the session share covers the maximum destination contribution of one complete eligible session/transport snapshot,
  including the largest catch-up batch over every legal locate position in that plan; and
- disjoint hold entitlements for every admitted non-compiled note-on producer sum to at most
  `release_hold_capacity`.

`release_hold_capacity` is a positive `EventCount` capacity, not a conversion from `HeldNoteCount` and not the
zero-valued measurement `EventCount::NONE`. Compiled held notes already own plan release entitlements, while
non-compiled held notes consume the separately admitted hold resource. Consequently `max_held_notes` does not promise
that every producer class can fill the whole state capacity independently of event budgets. Diagnostics name the
resource that bound a plan. This keeps `HOST-INV-018`'s domain separation intact while still giving non-compiled
obligations a throughput guarantee.

The numeric values are Phase 3 measurements. They are not coupled policy. All six shares are positive because
`EventCount` capacities reject zero; `release_hold_capacity` is positive for the same reason. An optional producer
that is disabled leaves its fixed share unused. Profile construction rejects a configuration that violates a
plan-independent relation; plan admission rejects work that violates a plan-dependent one. The present default of 256
is not presumed viable and must be reselected before the Phase 3 contract is enabled.

The engine default for `max_scheduled_events_in_flight` retains 4,096 events as authored-future headroom **above**
`compiled_event_share * max_quanta_per_callback`, using checked addition. The floor below that headroom is reserved
for the compiled class under the same no-borrowing rule as the shares: authored retention is charged to the headroom
alone, so a sparse compiled plan does not enlarge what authored retention may hold. The headroom is a provisional
memory choice, not evidence that the value is useful; Phase 3 measures and may reselect it.

### 2. One arbiter publishes only the imminent render span

Producers do not mutate destination buckets concurrently. They write to their bounded source storage; the single
publication arbiter snapshots eligible queue entries, evaluates scheduler work, and fills preallocated **external**
quantum batches for exactly the quanta the imminent `Renderer::render` call can render. It seals those external
batches before the first quantum begins. Renderer-internal emissions use a separate preallocated arena and ledger
inside their reserved share; they never reopen or mutate the external batch.

A renderer-internal emission takes effect in the quantum that generates it and may not target a later one. It is
produced on the renderer side of the sealed boundary, where nothing holds an event for a later quantum:
`max_scheduled_events_in_flight` sizes the arbiter's upstream scheduling, which an internal producer never reaches.
A declared per-quantum maximum is therefore a complete bound on such a producer, rather than a rate below which
occupancy could still accumulate at a destination quantum admitted without it. An internal producer that needs a
future target first requires the destination-occupancy and retained-future envelopes clause 5 imposes on authored
sources, which is a change to this record rather than an implementation choice.

This call-local window is the publication horizon. It is derived from the renderer's clock, carries and requested host
block, not stored as another `HostProfile` duration. Compiled events remain in the plan, and live events remain in
their bounded ingress queues, until the call that can consume them. This is ADR-0032 clause 21's rule that scheduler
releases occur as their quanta approach.

The queue snapshot is load-bearing: events arriving while a pass runs belong to the next pass. Therefore even a
continuously writing correct producer cannot make the current pass inspect more entries than the prepared snapshot
capacity.

Publication and destination admission are one operation. There is no named-quantum reservation that can outlive its
destination, so the reserve-then-publish race has no state to strand. A batch whose meaning is indivisible commits all
of its open-window slots and release holds, or none.

### 3. Every producer has a complete exhaustion rule

| Producer | Bound before publication | When it cannot publish |
|---|---|---|
| Compiled timeline and automation | Clause 4's admission entitlement | Plan, tempo-map replacement or loop activation was already refused; a runtime miss is a producer defect |
| Authored runtime expansion | Clause 5's destination, retained-future and hold envelopes | Plan admission was already refused; exceeding a declaration is a producer defect, never trimmed playback |
| Live ingress | Fixed queues, each recorded exactly once as *Live bounded queue* in the host-profile specification's [closed renderer-ingress source-store registry](../specs/spec-host-profile-and-render-limits.md#renderer-ingress-source-store-registry), and the live share's snapshot lower bound | A new external event may be dropped before publication, counted and reported under ADR-0021; an accepted queue entry waits only for its destination to enter the horizon |
| Session and transport | Fixed non-dropping command storage, distinct from legacy `command_queue_capacity`, a complete-snapshot share and plan-admitted catch-up maxima | A new non-critical command may remain at or be refused by its caller boundary before timestamped acceptance. Emergency commands have reserved source storage. An accepted snapshot publishes completely; a loop state change that does not fit is refused before transport state changes. Coalescing is allowed only where the session-order contract declares equivalence. Reusing the legacy physical queue requires an accepted change that removes its *Live bounded queue* classification and preserves this refusal contract |
| Guaranteed release | Disjoint producer hold entitlements acquired with note-ons, or a compiled entitlement | An over-large authored producer was refused with the plan, or a live note-on is refused at its allowed boundary; an accepted obligation is never refused later |
| Renderer-internal | Clause 2's same-quantum rule and the sum of admitted per-quantum declarations | Exceeding a declaration is the clause 7 defect |

A live note-on acquires both its event slot and a release hold atomically. If it cannot, that note-on is the event
dropped at the live boundary; a later edge for it is an orphan and is counted rather than allowed to release another
note. Once the note-on is published, its matching release cannot be dropped by queue pressure.

Stale-epoch, foreign-plan and out-of-horizon events are rejected and counted before they enter any destination share,
as they are before the tally today. They cannot consume a slot and then be retargeted. The forward horizon is
evaluated at ingress admission and only there, under `HOST-INV-013`: an accepted entry that has merely waited for the
next publication pass is never re-checked against it. A stale epoch or foreign plan slot stays checkable at
publication because the stream epoch can advance after admission.

### 4. Compiled admission covers anchors and loops

A compiled stream is admitted against the compiled share at the prepared sample rate. Admission rounds each musical
position once under ADR-0032 clause 15, then rejects the plan if any window of `Q` consecutive integer frame positions
contains more events than the share. That sliding-window test is exactly the worst case over all `Q` integer anchor
phases, so an admitted linear plan can start or seek at any anchor without a capacity failure.

A loop is a periodic stream, not a linear window. Establishing or changing a loop validates the periodic extension of
the half-open plan interval `[loop_start, loop_end)` over every anchor phase. The check repeats enough cycles to cover
one `Q`-frame window, so loops shorter than `Q` are not a special hole. If the compiled share would be exceeded, the
state change fails before activation, the prior transport state remains, and the diagnostic names the loop interval,
phase, requested count and available count. Once accepted, a wrap cannot fail for compiled capacity.

This is finite: positions are already integer `PlanPosition`s, the loop length is a positive integer frame distance,
and at most `ceil(Q / loop_length) + 2` copies can intersect the checked window. Refusing an invalid loop is acceptable
because it is an explicit user operation with an unchanged prior state, rather than an audible failure at the wrap.

**Finite is not the same as real-time, so this check runs off the audio thread**, like the tempo-map replacement
below. Its cost scales with the compiled events inside `[loop_start, loop_end)`, which no profile capacity bounds —
only the *window* it slides is bounded by `Q`. Running it inside a callback would put producer-sized work on the audio
thread, which ADR-0021 forbids. A loop change is therefore validated where the plan is, and the validated loop state
activates atomically; the audio thread only ever adopts an already-admitted loop. This is why an accepted wrap cannot
fail for compiled capacity without the arbiter doing any wrap-time work.

A tempo-map edit changes `PlanPosition` density and invalidates both compiled and authored-runtime entitlements. The
replacement tempo map is compiled and re-admitted off the audio thread, then activates only with the admitted plan;
failure leaves the previous plan and tempo map active. A sample-rate change already requires a new `HostProfile` and
stream epoch. Neither change may reuse occupancy calculated for the old mapping.

### 5. Authored runtime expansion is admitted by destination, then materialized once

An authored source that can vary at runtime declares three conservative maxima: the number of its events that can
**target one destination quantum**, the number of future events it can retain simultaneously, and the number of
release obligations it can hold simultaneously. Each declaration covers every active placement of that source,
every relevant tick instant at the prepared rate, every reachable data-dependent branch, every legal loop state and
every anchor phase. The first is a destination-occupancy envelope, not merely a bound on evaluations performed or
events produced while that quantum renders. A source that cannot provide all three finite maxima is not renderable
under this contract.

The compiler then constructs plan-wide destination, retained-future and hold envelopes across **all authored sources
that the plan permits to be active simultaneously**. It sums their declarations by default; it may reduce a sum only
when the compiled plan mechanically proves the corresponding source states mutually exclusive. Admission checks the
aggregate destination envelope against the authored-runtime share, the aggregate retained-future envelope against
the headroom `max_scheduled_events_in_flight` retains above the compiled floor, and the aggregate disjoint hold
entitlements against `release_hold_capacity`. Adding, replacing or reconfiguring a source recompiles and re-admits
the whole aggregate before atomic activation. Checking one source at a time is not admission: two individually
conforming sources must never jointly exceed a plan-wide resource.

At runtime the source evaluates once into preallocated scratch. The arbiter reads the resulting length and publishes
the batch atomically; it does not run the graph twice and it does not reserve the scratch's maximum when the actual
batch is smaller. The admission envelope, not optimistic runtime availability, is why every conforming batch fits.

This deliberately permits a conservative refusal. It does not permit the current V1 `ExpansionBuffer` behaviour of
dropped newest events: an overflow while computing the envelope refuses the plan, and an overflow after admission is
a producer-contract defect.

An authored event targeting beyond the open window remains in the preallocated scheduled-event store charged during
admission until its destination enters the horizon. A future note release uses clause 6's hold instead. Runtime scratch
is not a hidden unbounded future-event queue.

### 6. Future releases use holds, not future quantum containers

`release_hold_capacity` is partitioned into disjoint entitlements for every admitted non-compiled note-on producer;
the sum is checked at plan admission and no producer borrows another's unused holds. Every such note-on whose complete
note-on/release pair is not already present in one indivisible materialized open-window batch atomically acquires one
hold from its producer entitlement. In particular, a live note-on always takes a hold because a future external
note-off is not yet knowable. The hold owns no destination. When an individual release becomes publishable it redeems
one hold into the guaranteed-release share. A compiled release instead uses the destination entitlement established
by clause 4.

The event vocabulary represents panic, transport stop and sustain lift as bounded mass-release operations. The voice
allocator applies the operation to owned voices within the source event and atomically redeems every affected hold; it
may not emit a second release-share event or expand the operation into one renderer-internal event per voice. Panic
and sustain lift remain charged to the live share, transport stop to the session share, and a script-driven mass
release to that source's authored share. Consequently any number of same-quantum mass-release causes remains inside
the complete source snapshots or plan-wide authored envelope already admitted for that quantum, rather than consuming
an unbounded hidden release subshare. A script-driven source declares and consumes its own hold sub-entitlement under
the same rules, so Phase 7 cannot create an unbounded second obligation class. Exceeding an authored source's admitted
simultaneous-hold maximum is a producer defect; it is not a runtime refusal of a conforming note-on.

Holds close the release edge end to end: an accepted live release has guaranteed source storage as well as a renderer
share. Merely reserving renderer scratch while allowing the ingress queue to drop the edge would not satisfy this
clause.

### 7. Renderer overflow is a defect with a terminal response

The renderer does not move an event for capacity. After clauses 1-6, a share overrun, scheduled-store overrun,
over-full external batch or over-full internal arena means the profile was mis-summed, a producer exceeded its admitted
declaration, or a caller bypassed the publication contract. None is a load condition produced by a conforming source.

On that violation the renderer writes silence over the complete current callback and every later callback in the
epoch, invalidates both carries, publishes atomic `needs_reprepare`, allocates nothing, renders no further quantum, and
increments attributable counters for the structured diagnostics report. The same response applies when an internal
producer exceeds its separate reserved arena during a quantum. "Sealed" always describes immutable external input;
the internal arena contributes to the same total ledger without writing that batch. The existing Phase 1/2
pre-mutation `RenderError` remains those phases' caller-contract response until the Phase 3 stream boundary supplies
sealed batches.

The same terminal response applies when an external producer exceeds its fixed share even if unusable slack means the
quantum total remains below `max_events_per_quantum`. Slack is not recovery capacity, and absorbing the over-emit would
silently turn the declared share into a soft limit.

High-water occupancy is recorded per quantum and per producer share on every stream, not only after a fault.

## Producer-by-failure-mode audit

The acceptance invariant is: **a producer that obeyed every rule above cannot reach clause 7.** This table enumerates
the independent ways the claim could be false.

| Failure mode | Producers reached | Why a conforming producer cannot overfill the renderer |
|---|---|---|
| Simultaneous class peaks | All | Checked share sum; no borrowing |
| Destination retires between reservation and publication | All queued producers | There is no earlier reservation; one arbiter admits and publishes into the sealed call-local batch |
| Events arrive while queues are drained | Live and session | Snapshot closes the eligible set; new arrivals wait for the next pass, and both complete snapshots fit their shares |
| Every accepted ingress event is late and converges on one boundary | Live ingress | The live share covers the complete eligible ingress snapshot; lateness changes position, not admission |
| Note-on succeeds but its release later meets a full queue or quantum | Live and authored runtime | Note-on plus end-to-end hold is atomic; release share covers every possible simultaneous individual redemption |
| Several mass-release causes converge with individual releases | Live, session, authored runtime and guaranteed release | Every mass-release operation stays charged once to its source share and redeems affected holds as a side effect; no second release-share or per-voice event is emitted |
| Repeated runtime note-ons accumulate outstanding releases | Authored runtime and scripts | Plan admission reserves each producer's finite simultaneous-hold entitlement; live producers cannot consume it |
| Unknown or data-dependent graph output or future targeting | Authored runtime | Admission composes every simultaneously legal source into finite plan-wide destination, retained-future and simultaneous-hold envelopes; runtime materializes once and cannot consume more |
| Several accepted session commands become due together | Session and transport | The session share covers the complete eligible snapshot, including every indivisible expansion; no accepted command waits for destination capacity |
| A locate restores every prepared target at once | Session and transport | Plan admission checks the largest catch-up batch over every legal locate position before playback |
| Seek or anchor rebuckets a dense compiled cluster | Compiled | Sliding-window admission covers all `Q` phases |
| Loop wrap merges the tail and head, including multiple wraps per quantum | Compiled | Periodic validation runs before the loop state changes |
| Tempo-map replacement changes event density | Compiled and authored runtime | The replacement map and plan are recompiled and re-admitted before atomic activation; the old pair remains active on failure |
| One semantic operation is only partly published | All batched producers | Batch slots and holds commit atomically |
| Stale epoch, foreign slot or forward-horizon rejection changes occupancy | Live and compiled | Validation happens before the destination ledger |
| Internal event is generated after external input was sealed | Renderer-internal | A separate permanently reserved arena is sized from admitted declarations; an internal emission targets only its generating quantum, so nothing accumulates toward a later destination; external input remains immutable |
| Profile arithmetic or count conversion wraps | All | Fallible typed construction and checked `EventCount` sums; no `HeldNoteCount` conversion |

The only paths left to clause 7 are named contract violations. This is stronger than saying the measured corpus is
small: it remains true at the admitted maximum of every producer simultaneously.

## Consequences and risks

- ADR-0044 and ADR-0045 become `Superseded`, dissolved rather than answered. ADR-0022 remained Phase 3's only entry
  prerequisite at this decision's acceptance; its later boundary correction moved physical evidence to Phase 9 exit.
- No deferred store, starvation order, displacement counter or causal-order repair is implemented.
- The current value 256 may be too small once live snapshots, release holds and internal production receive hard
  guarantees. Phase 3 must measure and select a useful partition before enabling ingress.
- Conservative graph envelopes can refuse a project whose actual run would fit. High-water counters show the gap;
  a later accepted optimisation may tighten an envelope without changing this ownership boundary.
- The one-arbiter design makes publication serial work on the audio thread. Phase 3 must measure its bounded cost, but
  the existing materialize-then-consume graph path establishes that no second graph pass is required.
- Reopen if a required producer has no finite envelope, if the arbiter cannot meet callback cost at a useful profile,
  or if a correct source reaches clause 7.

## Specification update

Acceptance updates the current specifications in the same transaction:

- `HOST-INV-021` becomes the share, arbiter, admission, loop and hold contract above; its numeric values and concrete
  ingress stream capacities remain Phase 3 work.
- `SOUND-INV-016` names only the late clamp as a reason render position differs from the declared sample.
- ADR-0043 remains accepted for that clamp but its capacity-deferral half is superseded.
- ADR-0044 and ADR-0045 become `Superseded`; ADR-0022's later boundary correction moves its physical evidence to the
  Phase 9 exit gate.

## Review

Reviewer: independent Claude Code semantic review with only `Read`, `Grep` and `Glob`, slash commands disabled and MCP
disabled. The first read found nine blocking contract defects. Repairs were self-audited and followed by focused
independent rereads; those reads exposed residual authority metadata, profile enumeration, stale historical language,
source-citation and obligation-tracking gaps, each repaired before the next read. The final targeted reread of the
authoritative `NOW.md` → Phase 3 exit-gate → ADR-0043 tracking chain and supersession history reported no defects. The
author separately ran the documentation gate, evidence artifact and recorded SHA-256 because the reader had no shell.

After Claude Code's subsequent uncommitted review repairs, an independent Codex review found three remaining defects:
authored-runtime declarations were not composed across sources, zero-valued optional shares could not be represented
as `EventCount` capacities, and EVD-0015 labelled an onset-window calculation as a total renderer-occupancy upper
bound. Codex repaired those clauses, reused one parsed evidence population, guarded its empty cases and regenerated
the artifact and digest. Focused independent Claude Code rereads then exposed and bounded follow-on gaps in live-input
drop classification, mass-release share ownership, historical supersession language and failure-taxonomy accounting.
Each repair was self-audited before the next focused read; the final HOST-INV-009 stopping read reported no defects.

Stopping rule: a false conclusion-affecting fact, contradiction, unfillable producer contract, safety/correctness
defect, or evidence incapable of supporting the claim blocks acceptance. Optional implementation detail does not.
