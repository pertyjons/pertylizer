# SPEC: Host Profile and Render Limits

| Field            | Value                                  |
|------------------|----------------------------------------|
| Status           | Current                                |
| Phase            | 00A                                    |
| Created          | 2026-08-13                             |
| Last reviewed    | 2026-08-25                             |
| Based on         | ADR-0021, ADR-0001, ADR-0032, ADR-0037, ADR-0038, ADR-0043, ADR-0046 |
| Invariant prefix | HOST                                   |
| Supersedes       | —                                      |
| Superseded by    | —                                      |

Allowed status values are defined in [README.md](README.md).
Only a `Current` specification constrains implementation.

This specification is `Current`. `event_egress_capacity` depends on accepted
[ADR-0038](../decisions/ADR-0038-engine-egress-queue-classification.md), and all of its inventory antecedents are
`Classified`.

`HOST-INV-021` is the destination-admission contract selected by
[ADR-0046](../decisions/ADR-0046-destination-quantum-admission.md). A single publication arbiter builds bounded
quantum batches from disjoint prepared producer shares; the renderer never moves an event for capacity. The concrete
renderer-ingress streams and numeric share values remain Phase 3 work, but the checked sum, producer bounds, release
holds and refusal rules are normative. Phase 1 and Phase 2 keep their pre-mutation caller-contract rejection until the
Phase 3 stream boundary supplies sealed batches.

The field set below is complete against the master plan's initial Phase 1 list. Durable corrections are summarized in
[Corrections](#corrections); the full review result is [REV-P00A](../reviews/phase-00a-exit-review.md), not duplicated
here.

Fields owned by decisions that are still `Proposed` — ADR-0009, ADR-0024, ADR-0027, and ADR-0034 — are marked
in the field tables and listed under [*Unresolved questions*](#unresolved-questions). None of them blocks Phase 1.

## Scope

This specification defines `HostProfile`: the single immutable preparation input against which a Sound Core V2 render
plan is admitted. It fixes

- the profile's field set, its internal split, and the type of every field;
- the default value of every field, the basis for that value, and where it is revisited;
- who may set each field and who may never raise it;
- what compilation reports, and what happens when a plan does not fit;
- which V1 limits each field replaces, so that the [resource ledger](../inventories/resource-limits.md)'s 28
  `HostProfile`-owned entries each have a named successor.

It governs plan admission and stream preparation for the V2 renderer. It does not govern V1, which keeps its constants
until it retires.

## Non-goals

- **The render quantum.** Its semantics are [ADR-0001](../decisions/ADR-0001-internal-render-quantum.md)'s and its frame
  count is [ADR-0037](../decisions/ADR-0037-render-quantum-value.md)'s. `Q` appears below only as a symbol; nothing here
  is sized by its value. That was ADR-0037's own restriction while the value was provisional, and it is kept now that
  ADR-0037 fixes it finally, because a limit expressed in `Q` survives a later change of `Q` and one expressed in
  frames does not.
- **Class semantics and configuration ownership.**
  [ADR-0021](../decisions/ADR-0021-host-profile-and-admission-policy.md) decides both axes. This specification sets
  numbers for the one owner ADR-0021 assigned to P00A-T005 and touches none of the other six.
- **Limits owned elsewhere.** 48 of the ledger's 76 entries — none undecided, since ADR-0038 split `LIMIT-0014` —
  belong to a node contract, a domain/format contract, job policy, application settings, a protocol contract, or are
  removed. They are not profile fields and do not appear here except where a node declares a capacity *into* admission.
- **Hardware clock calibration, latency compensation, and drift.**
  [ADR-0022](../decisions/ADR-0022-hardware-time-mapping.md), deferred to the Phase 3 entry gate. This specification
  sizes the forward event horizon; it does not decide how a host timestamp maps into the epoch it is measured against.
- **Job bounds.** Render tail, output size, pre-roll, and quality presets belong to
  [ADR-0028](../decisions/ADR-0028-long-running-job-contract.md), deferred to the Phase 4 entry gate.
- **What an observation tap is for** (ADR-0027), **what a send is** (ADR-0034),
  and **what a recording take is** (ADR-0024). This specification carries their capacities, not their meanings.
  The current internal meaning of a channel layout belongs to the
  [Sound Core render contract](spec-sound-core-render-contract.md).

## Terminology

Defined in [../glossary.md](../glossary.md): *host profile*, *resource report*, *render quantum*, *render plan*, *sample
time*, *stream epoch*. Terms this specification adds:

**Host capabilities**

The subset of the profile that describes what the host and device can actually do. Established by querying them, never
by a compiled-in constant. The application does not raise a capability; it discovers one.

**Render limits**

The subset of the profile that describes budgets the operator chooses: how large a plan may be, **or which streams may
be prepared at all**, before the renderer refuses it. A render limit may be raised, at the cost of memory and CPU that
the resource report accounts for — and only within a ceiling the engine owns, where one exists.

The second clause is not padding: `accepted_sample_rates` is a limit that no plan can exceed, because no plan carries a
rate. An earlier revision defined this class over plan size alone, which left that field outside the category its own
placement rests on. What unites the class is that the operator sets it and a refusal follows, not the shape of the thing
refused.

**Admission**

Compiling a plan against one profile and either producing a prepared plan or refusing it with an attributable
diagnostic. Admission happens off the audio thread, once per prepared plan.

**Advisory budget**

A limit that is reported and never enforced. Exceeding it produces a `CompileWarning`, never a `CompileError`. It is the
`Warning threshold` class of ADR-0021 part 1.

## Accepted decisions

| ADR | Decision it fixes here |
|-----|------------------------|
| [ADR-0021](../decisions/ADR-0021-host-profile-and-admission-policy.md) | That `HostProfile` is an immutable preparation input holding render-preparation capacity; that its capability fields come from queried capability; that only profile-owned entries participate in admission; that a node declares its intrinsic capacity into admission without becoming a profile field; that exceeding a limit never rewrites authored data; that runtime *dropping* is reserved for live bounded queues while limits under other boundaries are enforced at their own admission, retention, or presentation point — the sentence HOST-INV-019 and HOST-INV-020 rest on; that the `Lossy retention/presentation budget` class exists and what it may never bound; that compilation returns a `ResourceReport` with requested, available, and dominant contributors; that P00A-T005 owns the numbers |
| [ADR-0001](../decisions/ADR-0001-internal-render-quantum.md) | That the quantum is a compile-time constant and **not** a profile field (clause 1); that both carries are sized `maximum_block_size + Q` and preallocated at preparation (clause 5); that the output carry is primed with `Q` frames of silence so that any `N` can be served, including `N < Q` (clause 6) — which is why HOST-INV-012 has no lower bound on `maximum_block_size`; that added latency is a constant `Q` frames and a named contributor in the latency accounting (clause 7); that a late event is clamped forward and counted (clause 16, as ADR-0043 superseded it) |
| [ADR-0032](../decisions/ADR-0032-sample-time-and-event-timestamps.md) | The time types the profile's frame-denominated fields are expressed in — `FrameCount` for a horizon, a latency contribution, and a quantum (clause 2); that the forward event horizon is a single profile field binding ingress provenance only (clause 21); that the backward direction has no budget; that a profile's sample rate, layout, and capacity are fixed for the life of a stream epoch (clause 12) |
| [ADR-0043](../decisions/ADR-0043-event-deferral-and-late-clamp.md) | The preserving late clamp: a late event moves to the first not-yet-rendered boundary, its envelope `time` is immutable, the late condition is asked once, and its control-rate response follows the clamped render position. ADR-0046 supersedes the record's capacity-deferral half |
| [ADR-0046](../decisions/ADR-0046-destination-quantum-admission.md) | That one publication arbiter constructs renderer input from six checked producer shares; compiled and authored-runtime work is admitted before playback; releases use end-to-end holds; and the renderer never moves an event for capacity |
| [ADR-0037](../decisions/ADR-0037-render-quantum-value.md) | That no field here may be sized by `Q`'s value, which ADR-0037 now fixes finally at 64 |

Several fields carry values owned by decisions that are still `Proposed`. They appear with the owning ADR named in the
field tables and are repeated under [*Unresolved questions*](#unresolved-questions). A value carried from V1 with an
open owner is a starting point recorded honestly, not a rule invented here.

## Invariants

1. **HOST-INV-001** — A `HostProfile` is immutable for the life of one stream epoch. Changing any field requires
   re-preparation, which issues a new `StreamEpoch` under ADR-0032 clause 12. No field may be mutated in place, and no
   field is read from a global.
2. **HOST-INV-002** — Exactly one profile admits a prepared plan, and the renderer reads every capacity from the
   prepared plan rather than from the profile. A capacity that reaches the audio thread without having passed admission
   is a defect, not a fallback.
3. **HOST-INV-003** — **On the device path**, every **queried capability** — the three values `HOST-INV-005` closes
   over, not `source`, which nothing queries — is established from queried host and
   device capability. A hardcoded advertised range is forbidden, including on a branch that successfully queried the
   device — this is the `LIMIT-0057` anti-pattern ADR-0021 part 4 names. The rule is enforced by construction rather
   than by inspection: `HostCapabilities::from_device` takes every capability as an argument and has no default for
   any of them, so a value that was not queried cannot reach it without a caller writing the constant at the call
   site, where it is visible. The device-less paths have **one constructor each** — `::offline` and `::harness` — and
   neither takes a source argument, so `Device` is not among the tags they can produce.

   **What that buys, stated exactly.** No runtime tag can prove that a query happened — this specification already
   records that finding, and it applies to the constructor split too: `::from_device` is public and takes values, so a
   caller determined to mislabel itself can pass constants and obtain `Device`. Three narrower things do hold, and they
   are the guarantee: a capability that was not queried must be **written at the call site**, where review sees a
   literal instead of a defaulted field; there is no `Default` and no `..Default::default()` tail for the shape test to
   permit;
   and a path with no device **need not** mislabel itself, because `::offline` and `::harness` exist and their tags are
   honest. Earlier
   revisions of this paragraph claimed `Device` was unforgeable and, before that, that the tag could not disagree with
   the path — both stronger than a public constructor can enforce.
4. **HOST-INV-004** — The render quantum is not a profile field, is not derived from any profile field, and is not
   configurable. No field below may be defined in terms of `Q`'s numeric value; a field may be defined in terms of the
   symbol `Q`.
5. **HOST-INV-005** — The profile carries exactly the fields listed in [*The field set*](#the-field-set).

   **The two halves are admitted by different rules, and an earlier revision applied one rule to both.** The
   `HostCapabilities` half is a **closed enumerated set of three queried values** — `sample_rate`, `maximum_block_size`,
   and `channel_layout` — fixed by ADR-0021 part 4, which requires a capability to come from queried capability, and by
   ADR-0032 clause 12, which fixes rate, layout, and capacity for the life of an epoch. A capability is admissible
   because the device has it and the renderer must be **prepared against** it; a limit is what admission **refuses on**.
   `LIMIT-0001` and `LIMIT-0059` therefore appear in those rows as **provenance**, on the same footing as
   `max_held_notes`'s `LIMIT-0031`. Adding a queried capability is a change to this specification, reviewable as such.

   **`source` is the fourth struct member and is not one of the three.** It is not queried from anything — ADR-0021 part
   4's rule could not admit it — and it is not a capability the renderer prepares against. It records *which
   constructor* produced the other three, and the rule that admits it is `HOST-INV-003`. There is **one constructor per
   source** and no source argument anywhere, so no call can pass a tag that disagrees with the constructor it called.
   What that does not do is prove a caller picked the right constructor — see `HOST-INV-003` for the exact guarantee.
   Classifying `source` as a queried capability would make `HOST-INV-003` unsatisfiable, because nothing queries it.

   **`LIMIT-0004` is the case that fixed the boundary between the two halves.** Its entry is owned `HostProfile` and its
   rule refuses an out-of-range job *at admission*, which ground 1 reserves to limits, so `accepted_sample_rates` is a
   `RenderLimits` field and not the fifth capability two earlier revisions made it.

   That scoping is a correction rather than a simplification. The three grounds below were stated over every profile
   field while three capability fields — `sample_rate`, `channel_layout`, and `source` — matched none of them, so the
   named conformance test, which "fails on a field in none", could not have passed; and `maximum_block_size` would have
   matched ground 1 through `LIMIT-0001` while also being queried, which breaks *exactly one*. This is a document
   finding, from enumerating the field set against the grounds by hand while preparing that test; the test itself is
   [P01-T002](../phases/phase-01-experimental-sound-core.md)'s deliverable and does not exist yet.

   A **`RenderLimits`** field is admissible on one of three grounds, **applied in this order so that every such field
   takes exactly one**:

   1. **a ledger entry owned `HostProfile`.** ADR-0021 part 1 is explicit that only these participate in admission and
      that `N/A — removed` holds nothing, so an entry with that owner cannot be a ground however its disposition reads.
      An earlier revision widened this ground to "any disposition that creates a profile capacity" in order to catch
      `LIMIT-0031` and `LIMIT-0075`; that contradicted the accepted decision, and those two are ground 3 instead;
   2. **an accepted ADR that creates it** — `forward_event_horizon` is ADR-0032 clause 21's, while ADR-0046 creates
      the six event-share fields and `release_hold_capacity`;
   3. **the residual: an enumerated list this specification creates.** Ground 3 is a *closed set*, not "everything
      else" — that would make the ownership restriction unenforceable, since a protocol- or job-owned capacity would
      pass by default. The list is: the fourteen no-antecedent fields **minus the eight that ground 2 already selects**
      — `forward_event_horizon` and ADR-0046's seven fields — so six, plus `max_held_notes` and
      `max_events_per_quantum`, whose ledger entries (`LIMIT-0031`, `LIMIT-0075`) appear in the `Replaces` column as
      **provenance** — a different
      question from what admits the field. Adding to this list is a change to this specification, reviewable as such.

      **`event_egress_capacity` is not on this list because it does not need to be — it now satisfies ground 1.**
      It had no ground while `LIMIT-0014` carried an undecided owner, one constant sizing a GUI ring and an OSC ring
      where ADR-0021 allows one owner per entry. [ADR-0038](../decisions/ADR-0038-engine-egress-queue-classification.md)
      part 4 splits them: `LIMIT-0014` keeps the engine-to-GUI ring and is owned `HostProfile`, which is ground 1, and
      the OSC ring becomes `LIMIT-0076` owned by the protocol contract that serializes it — **so it is not a profile
      field at all**, and the profile carries one capacity here rather than two.

   Grounds 1 and 2 are disjoint by construction, and 3 excludes both, so each limit field matches exactly one. **"No V1
   antecedent" is a different axis and does not select a ground**: it answers whether V1 had the thing, not what admits
   the field, and
   `forward_event_horizon` is on both lists precisely because those are different questions. An earlier revision
   defined ground 3 *as* the no-antecedent list, which left `forward_event_horizon` matching two grounds and
   `max_held_notes` matching none. A capacity that belongs to another owner may not be smuggled in on any of the three.
6. **HOST-INV-006** — Compilation returns a `ResourceReport` naming, for every field, the requested amount, the
   available amount, and the dominant contributor to the request. The report is produced whether admission succeeded or
   failed; a refusal is a report plus an error, never an error alone.
7. **HOST-INV-007** — A plan exceeding a render limit is refused with a `CompileError` naming the field, the requested
   amount, the available amount, and the authored object responsible. Admission never truncates, clamps, or drops to
   make a plan fit, per ADR-0021 part 2.

   **It binds the limits a plan can exceed, and `accepted_sample_rates` is the one that no plan can.** A plan does not
   request a sample rate: the profile fixes it for the epoch, so the only way the rate and the range can disagree is
   inside one profile, which `HOST-INV-016` refuses at construction before any plan is compiled. Requiring a plan-level
   `CompileError` for this field would require a plan-level rate, which is exactly the duplicate the master plan's
   `RenderConfig` sketch carried and Phase 1 removed.

   **`accepted_sample_rates` is the clearest case, not the only exception.** Phase 1's implementation of this
   invariant found that **fourteen** of its forty-two reported fields do not take a `HOST-INV-007` refusal. ADR-0046
   adds seven Phase 3 fields: the live and release event shares cannot be exceeded by a plan, while the other five can.
   Once that contract is enabled, **sixteen** of forty-nine fields do not take this refusal, in five groups: the three
   queried capabilities, which describe what a plan is *prepared against*; `accepted_sample_rates`; the three sizing
   fields, which bound nothing; the capacities a plan does not request — `forward_event_horizon`, the two queue depths,
   the two per-voice slot counts, the live and release event shares, and `max_concurrent_retiring_voices`, which is
   derived so that it cannot bind; and the advisory `predicted_quantum_cost_ratio`, whose excess produces a warning.
   The other thirty-three each have a refusal case. The two per-voice slot counts move into that set when a phase
   declares per-voice slot *usage*, which is Phase 7's; until then their rows report the profile's own value.

   **That does not discharge `LIMIT-0004`'s disposition, and an earlier revision claimed it did "in substance".** The
   ledger requires a *job* outside the range to be refused with a job admission error naming the requested rate and the
   profile range. A job is not a plan and not a profile: it asks for a rate before either exists, so the check belongs
   to
   the job contract — [ADR-0028](../decisions/ADR-0028-long-running-job-contract.md), `Deferred` to the Phase 4 entry
   gate — and remains **outstanding**, not satisfied by a construction failure one layer down. What construction gives
   is a floor: no out-of-range stream can be prepared even before the job layer exists. Both enforcement points are real
   and neither substitutes for the other.
8. **HOST-INV-008** — A node's intrinsic capacity is declared by the node, reported in the `ResourceReport`, and
   contributes to admission. It is not a profile field, and no operator setting may raise it.
9. **HOST-INV-009** — Within live input, **dropping** at runtime is permitted only for queues explicitly marked *Live
   bounded queue* in either the [Events field table](#events) or the
   [renderer-ingress source-store registry](#renderer-ingress-source-store-registry). Those two tables are the closed
   live-input drop-licence registry; a marker anywhere else has no effect under this invariant. Every registered queue
   counts its drops, and the count reaches the structured diagnostics report. The queue's capacity is not the only
   shortage that can cause one, and the second cause is licensed here rather than left
   to HOST-INV-021 to invent: a live note-on that cannot atomically acquire both its queue slot **and** its producer
   release hold is dropped at that same live boundary, before acceptance, and never later. It is counted against the
   queue it was offered to, with the exhausted resource — slot or hold — named, so the two causes stay distinguishable
   in the report. This remains the one live-input drop behaviour: no other live-input shortage may be discharged as a
   drop. Engine-egress dropping is outside this registry and is separately licensed by ADR-0038 part 1. This invariant
   governs live-input dropping, not every runtime behaviour: a full non-dropping session/transport source store takes
   HOST-INV-021's caller-boundary refusal; explicit eviction is HOST-INV-019's; a session limit that halts an activity
   is HOST-INV-020's; and an over-full external batch or internal arena takes HOST-INV-021's terminal invariant fault.
   Phase 1 and Phase 2 instead reject their open caller span before mutation. Together with HOST-INV-007's admission
   refusal, [*Failure and diagnostics*](#failure-and-diagnostics) assigns every field one behaviour — or records it as
   a **sizing field**, which bounds nothing and therefore cannot be exceeded.
10. **HOST-INV-010** — A `HostProfile` is never persisted in a project document, patch, or bundle. It describes the
    machine and the operator's budgets, not the work; a project that rendered on one profile must load on another.
11. **HOST-INV-011** — A budget whose meaning is a duration is evaluated in seconds at the prepared sample rate, never
    in frames. A block carries the same work at every sample rate while its real-time budget shrinks with the rate
    ([EVD-0003](../evidence/phase-00a/EVD-0003-cpu-memory-timing-baseline.md)), so a policy stated in frames reads a
    192 kHz plan as no more expensive than a 44.1 kHz one when it is more than four times as expensive per second.
    `forward_event_horizon` is the field this governs, and its default takes a second at the prepared rate.

    **It binds budgets, not sizes, and the two duration-valued sizing fields are named exceptions.**
    `retirement_crossfade` (128 frames) and `telemetry_ring_frames` (4 096 frames) also mean durations, and both are
    flat frame counts, so both shrink by 4.4x across the supported rate range — a crossfade of 2.9 ms at 44.1 kHz and
    0.67 ms at 192 kHz, a scope window of 93 ms against 21 ms. Neither bounds a plan, so neither can misjudge one,
    which is why the invariant does not reach them. Both are nevertheless carried over from V1 unexamined, and the
    crossfade's rate-dependence is audible where the ring's is only cosmetic: ADR-0009 and ADR-0027 own whether they
    should be stated in seconds instead, and the question is recorded as unresolved rather than answered here.

    **Destination occupancy is not a duration budget.** `max_events_per_quantum` retains its event unit and one
    profile-selected value within an epoch; it is not rescaled with rate. Phase 3 must still reselect the current
    unevidenced 256 before enabling ingress. A musical plan's requested `EventCount` is recomputed at the prepared rate
    because a 64-frame window spans different musical time there. A lower rate may therefore make the same project
    request more events and fail admission. This case is explicitly outside the invariant's first sentence: it is
    measurement of work against an unchanged count budget, not frame-based reinterpretation of a duration.
12. **HOST-INV-012** — Both carries are sized `maximum_block_size + Q` frames and preallocated at preparation (ADR-0001
    clause 5). **`maximum_block_size` has no lower bound in terms of `Q`.** A host whose largest block is smaller than
    one quantum is supported, and so is any individual callback of `N < Q`: ADR-0001 clause 6 primes the output carry
    with `Q` frames of silence at stream start precisely so that clause 5's loop can serve any `N` without rendering a
    quantum whose input has not arrived. A profile requiring `maximum_block_size >= Q` would refuse a host the render
    model was built for.
13. **HOST-INV-013** — `forward_event_horizon >= maximum_block_size + Q`, and it binds only events whose provenance is
    `Hardware` or `Arrival` (ADR-0032 clause 21). It never measures the scheduler's own releases of compiled events, and
    there is no backward horizon.

    **It is evaluated exactly once, at ingress admission, against the timestamp as stamped.** Nothing that happens to
    an event after it is admitted re-triggers the check. The clause is here because the horizon's penalty is
    *rejection*: an event accepted into bounded source storage may wait for the next publication snapshot and may be
    late by then, but it is not new ingress. Keeping the envelope's `time` immutable makes that distinction testable.
    ADR-0032 clause 21 already scopes the horizon to bounding "what an *external* producer can enqueue" — this says the
    same thing where an implementer would look for it.
14. **HOST-INV-014** — Memory budgets are checked against the compiler's computed aggregate over prepared nodes, not
    against a process-level measurement. V1 computes no such aggregate anywhere (`LIMIT-0073`); producing one is part of
    what admission is.
15. **HOST-INV-015** — An advisory budget never refuses a plan. It emits a `CompileWarning` carrying the predicted and
    permitted values, and compilation continues.
16. **HOST-INV-016** — A profile whose fields are mutually inconsistent fails validation at construction, before any
    plan is compiled, naming the two fields that disagree. Construction is fallible; there is no partially valid
    profile and no clamping constructor.
17. **HOST-INV-017** — A **relation between capacities is declared once**, in the profile's constructor, rather than
    maintained by an assertion in a third crate. `LIMIT-0023` and `LIMIT-0041` are the case: V1 keeps
    `MAX_MOD_MATRIX_SLOTS <= SCRIPT_HOST_SLOTS` with a compile-time assertion in `synth_modules`, and the profile
    carries `mod_matrix_slots_per_voice` and `script_host_slots_per_voice` as **two fields** with that floor validated
    at construction. An earlier revision made them one field, on a ledger claim that V1 coupled them 1:1; the use-site
    audit found the assertion is an inequality and that `ScriptHost` is module-agnostic, so collapsing them would
    refuse one resource or overprovision the other.
18. **HOST-INV-018** — Every profile field that carries a **quantity** is typed by a domain newtype with a private
    field and a fallible constructor. No such field is a bare `usize`, `u32`, or `f32`, and no two fields whose units
    differ share a type. Two fields carry a **kind** rather than a quantity — `channel_layout` and `source` — and are
    closed enums: they have no private field and no constructor to make fallible, because an enum admits no invalid
    value in the first place. The distinction is what makes the invariant testable; an earlier revision stated it over
    every field, which the two enum fields cannot satisfy and which would have failed its own named conformance test.
19. **HOST-INV-019** — A field marked *lossy* bounds non-authoritative retention or presentation data and evicts by
    design rather than failing. It is ADR-0021 part 1's `Lossy retention/presentation budget` class, and the class's
    condition binds here: the owner exposes an evicted or omitted count, a continuation marker, or an equivalent
    user-visible way to tell a complete view from a trimmed one. A lossy field may never bound canonical project data,
    authored topology, render input, automation, routing, sample mapping, or polyphony.
20. **HOST-INV-020** — A field marked *session limit* is enforced while an activity runs rather than at admission,
    because the quantity it bounds is not knowable when the plan is compiled. Reaching it **stops the activity with a
    counted diagnostic and keeps everything already produced**; it never drops, trims, or overwrites authored data. The
    recording capacities are the only session limits in this profile.
21. **HOST-INV-021 — destination admission makes renderer capacity a construction invariant.**
    [ADR-0046](../decisions/ADR-0046-destination-quantum-admission.md) supersedes ADR-0043's capacity-deferral
    rule. An on-time event's render position is its immutable envelope `time`; a late event uses ADR-0043's preserving
    clamp to the first not-yet-rendered boundary. Capacity never changes either position. The renderer does not defer,
    trim, drop or reorder an event to make a quantum fit.

    **Six disjoint profile shares sit within the total.** Every renderer event is charged exactly once, to compiled
    timeline/automation, authored runtime expansion, live ingress, session/transport, renderer-internal production,
    or guaranteed release. The shares are `HostProfile` inputs. Construction uses checked `EventCount` arithmetic,
    rejects a sum above `max_events_per_quantum`, and validates the plan-independent relations below. Plan admission
    checks its declarations against the fixed shares; preparation allocates the admitted extents but derives or
    mutates no share. Shares remain fixed for the stream epoch, and no source borrows another source's unused capacity
    on the audio thread. Each share and `release_hold_capacity` is a positive `EventCount` capacity;
    `EventCount::NONE` is a measurement and may not represent a disabled producer or hold resource in the profile. A
    disabled producer leaves its share unused.

    A checked sum below `max_events_per_quantum` leaves unusable safety slack, not an unnamed seventh share. No
    producer may borrow it.

    Profile construction requires:

    - the live share to cover the sum of every live renderer-ingress queue snapshot eligible for one publication pass;
    - `max_scheduled_events_in_flight` to cover `compiled_event_share` times
      `max_quanta_per_callback = ceil(maximum_block_size / Q)`;
    - `release_event_share >= release_hold_capacity`, so every outstanding non-compiled hold can redeem an individual
      release into one destination quantum; and
    - the sealed-batch store to cover `max_events_per_quantum * max_quanta_per_callback`.

    All products and conversions are checked. `max_quanta_per_callback` is a derived `QuantumCount`; carry may
    reduce one callback's work but cannot increase this maximum.

    Plan admission separately rejects unless:

    - the compiled destination-occupancy envelope and the plan-wide aggregate authored destination envelope fit their
      respective shares;
    - the compiled callback window fits the derived compiled floor, and the plan-wide aggregate maximum of
      simultaneously retained authored future events fits the headroom `max_scheduled_events_in_flight` retains
      above it. The floor is reserved for the compiled class, not lent to authored retention by a sparse plan;
    - the internal share covers the sum of admitted internal producers' declared per-quantum maxima, which bounds
      them completely only because an internal emission targets its generating quantum;
    - the session share covers the maximum destination contribution of one complete eligible session/transport
      snapshot, including the largest catch-up batch over every legal locate position; and
    - disjoint hold entitlements for all admitted non-compiled note-on producers sum to at most
      `release_hold_capacity`.

    `release_hold_capacity` is an `EventCount` independent of `HeldNoteCount`. An admitted note can consume held
    state without creating an individually held release obligation, for example when its compiled release already has
    a destination entitlement. There is no implicit conversion between the two domain quantities.

    **One publication arbiter owns external renderer input.** Producers write bounded source storage. Immediately
    before a render call, the arbiter snapshots eligible queue entries and fills preallocated external batches for
    exactly the quanta that call can render from the current clock, carries and requested host block. It seals every
    external batch before the first affected quantum begins. Entries arriving during the pass belong to the next
    snapshot, so concurrent input cannot enlarge the work already admitted. Renderer-internal emissions use a
    separate preallocated arena and ledger inside the internal share; they never reopen the external batch, and each
    takes effect in the quantum that generates it. Internal production therefore adds nothing to
    `max_scheduled_events_in_flight` and cannot accumulate at a later destination.

    That exact call span is the publication horizon. It is derived state, not another duration budget. Compiled events
    remain in the plan and live events remain in their bounded source queues until their destination enters it. A
    batch whose meaning is indivisible commits all open-window event slots and release holds, or none. There is no
    reserve-then-publish state and therefore no capacity token a retired quantum can strand.

    **Compiled work takes an entitlement at plan admission.** After ADR-0032 clause 15's one rounding step, the
    compiler rejects a plan when any half-open window of `Q` consecutive integer frame positions exceeds the compiled
    share. This is the worst case over all `Q` integer anchor phases, so play and seek cannot create a new compiled
    overrun.

    Establishing or changing a loop separately checks the periodic extension of the half-open interval
    `[loop_start, loop_end)` over every anchor phase. The check includes enough repetitions to cover a `Q`-frame
    window, including multiple wraps when the loop is shorter than `Q`. Failure leaves the previous transport state
    unchanged and reports the interval, phase, requested count and available count. An accepted loop cannot fail for
    compiled capacity at its wrap. **This check runs off the audio thread**, like the tempo-map replacement below:
    only the sliding window is bounded by `Q`, while the work scales with the compiled events inside the loop
    interval, which no profile capacity bounds. The audio thread adopts an already-admitted loop state atomically and
    does no wrap-time admission of its own.

    A tempo-map edit invalidates compiled and authored-runtime entitlements. The replacement map is compiled and
    re-admitted off the audio thread and activates only with its admitted plan; failure leaves the old plan and tempo
    map active. A sample-rate change requires a new profile and epoch. Neither path reuses occupancy calculated for an
    old time mapping.

    **Authored runtime expansion is admitted by three plan-wide envelopes, not by optimistic runtime space.** Every
    source declares finite conservative maxima for destination occupancy per quantum, simultaneously retained future
    events, and simultaneously outstanding release obligations. Each declaration covers simultaneous placements of
    that source, relevant tick instants, reachable data-dependent branches, every legal loop state and every anchor
    phase. A source missing any finite envelope is refused.

    The compiler composes those declarations across every authored source the plan permits to be active
    simultaneously. It sums declarations unless the compiled plan mechanically proves the corresponding source
    states mutually exclusive. Admission checks the aggregate destination envelope against the authored share, the
    aggregate retained-future envelope against the headroom `max_scheduled_events_in_flight` retains above the
    compiled floor, and the aggregate disjoint hold entitlements against `release_hold_capacity`. Adding, replacing
    or reconfiguring a source recompiles and re-admits the entire aggregate before atomic activation. Per-source
    checks alone do not admit a plan.

    Runtime expansion evaluates once into preallocated scratch and publishes the materialized batch atomically. It
    does not run twice and it does not publish its maximum when fewer events exist. The V1 policy of dropping the
    newest expansion is forbidden: envelope overflow refuses the plan, and post-admission overflow is a
    producer-contract defect. A future authored event remains in its admitted scheduled-event store until its
    destination enters the publication horizon; a future note release uses a hold. Runtime scratch is not a
    future-event queue.

    **Live input may lose only a new event at its bounded boundary.** Queue overflow or a note-on that cannot acquire
    its event slot and live-producer hold is dropped before renderer publication, counted and reported under
    HOST-INV-009. An entry already accepted into the eligible snapshot waits only for its destination to enter the
    publication horizon, never for destination capacity. If it is late when published, ADR-0043's preserving clamp
    applies and is counted; the live share covers the complete snapshot even when every accepted event converges on
    that boundary.

    `release_hold_capacity` is partitioned into disjoint producer entitlements. A non-compiled note-on whose complete
    note-on/release pair is not already in one indivisible materialized open-window batch acquires a hold atomically.
    Every live note-on therefore takes one because its future external note-off is not yet knowable. A matching
    individual release redeems one hold into the release share. Panic and sustain lift stay charged to their live
    source share, transport stop stays charged to the session share, and an authored mass release stays charged to the
    authored share. The allocator redeems all affected holds while applying that bounded source event; it emits no
    second release-share event and may not republish one event per voice. The hold guarantees source storage as well
    as renderer capacity. Compiled releases use plan entitlements and need no hold.

    **Every source has one exhaustion rule.** A compiled plan, authored-runtime envelope, tempo-map replacement or
    invalid loop is refused before activation. New live input may be dropped only as above. A new non-critical session
    command may remain at or be refused by its caller boundary before timestamped acceptance; emergency commands have
    reserved source storage. An accepted session snapshot publishes completely. Plan admission has already checked the
    worst locate catch-up batch. Coalescing is permitted only where the session-order contract declares equivalence.
    An internal producer that exceeds its admitted declaration is a defect.

    Stale-epoch, foreign-plan and out-of-horizon events are filtered and counted before the destination ledger. They
    never consume a share and then move to another one.

    **A share overrun, scheduled-store overrun, over-full external batch or over-full internal arena is a terminal
    stream-contract fault.** It proves a checked sum, an admitted declaration or the sole-publication boundary was
    violated; no conforming producer can cause it. An external producer that exceeds its share takes this response
    even when unusable slack keeps the total below `max_events_per_quantum`. The
    renderer writes silence over the complete current callback and every later callback in the epoch, invalidates both
    carries, publishes atomic `needs_reprepare`, allocates nothing, renders no further quantum, and increments
    attributable diagnostics. External batches stay immutable; internal events contribute through their separate
    arena and the same total occupancy ledger.

    Phase 1 and Phase 2 keep their current pre-mutation `RenderError` for a caller-supplied over-full span. Phase 3
    replaces that open caller boundary with sealed external batches and the terminal invariant response above. Every
    stream reports high-water occupancy per quantum and per share, whether or not a fault occurs.

## Deferred to Phase 3

ADR-0046 fixes the safe relations and every exhaustion class. Phase 3 still owns the concrete renderer-ingress streams
and numeric values because V1 has no timestamped renderer-ingress design to carry over and 256 has no measurement
basis. These are values and internal stream shapes, not undecided overload policy.

| Deferred value or shape | Required Phase 3 result | What holds meanwhile |
|---|---|---|
| Live renderer-ingress streams and their capacities | Name each fixed-capacity live source store, add exactly one row for it marked *Live bounded queue* to the [closed renderer-ingress source-store registry](#renderer-ingress-source-store-registry), and include its snapshot capacity in the live-share lower bound | No profile field may be invented at a call site. ADR-0046 creates seven ground-2 fields and **not** these, so each new ingress capacity needs an admitting ground before it becomes a profile field: an accepted ADR that creates it, or an explicit amendment of HOST-INV-005's closed residual list, reviewable as a change to this specification |
| Six producer-share values and `release_hold_capacity` | Measure useful fixed profile inputs, validate every construction relation and checked sum, and reselect `max_events_per_quantum` from those measurements before enabling Phase 3 | The unevidenced value 256 is not retained merely because the measured partition fits. Construction never derives shares from a plan; every share and `release_hold_capacity` is a positive `EventCount`, a disabled optional class leaves its share unused, and `release_event_share >= release_hold_capacity` |
| Session/transport source storage and plan-time declarations | Name the fixed non-dropping session stores; plan admission checks complete snapshot expansion, every locate catch-up position, all three authored-runtime envelopes, internal declarations and hold entitlements | These stores are not *Live bounded queue* entries and are distinct from legacy `command_queue_capacity`. A future design may reuse that physical queue only through an accepted change that replaces its drop classification and preserves refusal before timestamped acceptance. An undeclared producer cannot publish; plan-dependent work is refused at admission rather than changing a share or failing during playback |
| Publication-arbiter cost | Measure the bounded serial pass with representative and admitted-maximum batches | The graph path needs one materialization, not a second evaluation; high-water counters expose conservatism |

There is no deferred-event store, deferred-store exhaustion policy, starvation order or capacity-displacement counter.
ADR-0046 removes the mechanism that required them.

`max_events_per_quantum` remains normative in Phase 1 and Phase 2. Their event input is a prevalidated bounded span:
plan preparation refuses a statically knowable overrun, and `Renderer::render` rejects a caller span before renderer
state or output is mutated if any absolute quantum exceeds the cap. The total span may exceed the cap when it covers
several quanta. Those phases never defer, drop, clip, partially render or allocate to absorb the violation.

The rejection is a **release-active** mechanism, which is why the signature is fallible. A `debug_assert` may
supplement it during development but can never define this behaviour, because it compiles out of the build that runs.
REV-P00A raised that as a P2 finding against an earlier draft, and it binds Phase 3's sealed-batch boundary for the
same reason.

Phase 3 replaces the open caller span with HOST-INV-021's sealed batches. From that point an over-full quantum is not a
recoverable caller error; it is the terminal invariant fault specified there.

## Types and ownership

`HostProfile` splits into two halves that differ in **who may set the value**, which is the distinction ADR-0021 part 4
draws when it requires capability fields to come from queried capability and leaves the budgets to the operator. The
split is why the master plan's two names — `HostProfile` and `RenderLimits` — both survive: they are not two inputs, and
`RenderLimits` is the budget half of the one input.

```rust,ignore
/// The single immutable preparation input. Constructed off the audio thread,
/// validated once, and fixed for the life of one `StreamEpoch`.
#[must_use]
pub struct HostProfile {
    capabilities: HostCapabilities,
    limits: RenderLimits,
}

/// What the host and device can do. Queried, never chosen.
#[must_use]
pub struct HostCapabilities {
    sample_rate: SampleRate,
    maximum_block_size: FrameCount,
    channel_layout: ChannelLayout,
    source: CapabilitySource,
}

/// Where the capability half came from. An offline job has no device, so it
/// declares one rather than pretending to have queried it.
pub enum CapabilitySource {
    /// Queried from a live device through the audio backend.
    Device,
    /// Declared by an offline render or analysis job (ADR-0028).
    Offline,
    /// Declared by a test harness constructing IR directly.
    Harness,
}

// There is no `DeclaredSource` argument type. An earlier revision had
// `::declared(.., source: DeclaredSource)`, which put the tag back in the
// caller's hands: an offline job could pass `Harness` and a harness `Offline`,
// so the tag said only what the caller typed. One constructor per source
// removes the argument instead of qualifying the claim.

impl HostCapabilities {
    /// The device path. Every capability is an argument; there is no default for
    /// any of them, and no `..Default::default()` tail. Sets `Device`.
    pub fn from_device(
        sample_rate: SampleRate,
        maximum_block_size: FrameCount,
        channel_layout: ChannelLayout,
    ) -> Result<Self, ProfileError>;

    /// An offline render or analysis job (ADR-0028), which has no device to
    /// query. Sets `Offline`; there is no argument that could say otherwise.
    pub fn offline(
        sample_rate: SampleRate,
        maximum_block_size: FrameCount,
        channel_layout: ChannelLayout,
    ) -> Result<Self, ProfileError>;

    /// A test harness constructing IR directly. Sets `Harness`.
    pub fn harness(
        sample_rate: SampleRate,
        maximum_block_size: FrameCount,
        channel_layout: ChannelLayout,
    ) -> Result<Self, ProfileError>;
}

/// What the operator budgets. Raisable, at a cost the `ResourceReport` accounts for.
#[must_use]
pub struct RenderLimits {
    /// `LIMIT-0004`'s successor: the range of sample rates a stream may be
    /// prepared at. A limit rather than a capability because its ledger entry is
    /// a configurable budget owned by `HostProfile` whose rule refuses an
    /// out-of-range job *at admission*, which is what only limits do.
    stream: StreamLimits,
    graph: GraphLimits,
    voices: VoiceLimits,
    events: EventLimits,
    observation: ObservationLimits,
    mixing: MixingLimits,
    memory: MemoryLimits,
    script: ScriptLimits,
    recording: RecordingLimits,
    cost: CostBudget,
}
```

Each group is a struct of newtypes. The newtypes, one per unit:

| Newtype | Unit | Notes |
|---------|------|-------|
| `SampleRate` | Hz | **The V2 type; V1's clamping `synth_core::SampleRate` is replaced, not reused** — the same ground ADR-0021 part 3 gives for `VoiceCount`. See below |
| `SampleRateRange` | Hz pair | The `accepted_sample_rates` limit. Two V2 `SampleRate`s, **both endpoints inclusive**, with a fallible constructor that rejects `minimum > maximum` and a `maximum` above `DeviceSampleRate::MAX_SUPPORTED`; each endpoint's own validity is the rate type's. Equal endpoints are legal — a fixed-rate host is one rate wide |
| `FrameCount` | frames | ADR-0032 clause 2; carries the block size, the horizon, and the crossfade |
| `QuantumCount` | quanta | A derived callback or storage extent; distinct from frames and events |
| `NodeCount`, `EdgeCount`, `FanOut` | nodes, edges, edges per port | Graph extents after polyphony expansion |
| `VoiceCount` | voices | The V2 type; V1's clamping `synth_core::VoiceCount` is replaced, not reused (ADR-0021 part 3) |
| `BusCount`, `SendCount`, `MixChannelCount` | buses, sends, mix channels | The mix-channel count is **not** `ChannelCount`: `synth_core::ChannelCount` already exists in this workspace and means a channel *layout* (`Mono`, `Stereo`, `Multi(n)`). Reusing the name for a count of mix channels is the hazard ADR-0032 clause 5 refused for `SampleOffset` — two unrelated meanings one import away |
| `EventCount` | events | Per quantum, per tick, and per queue |
| `HeldNoteCount` | held notes | Distinct from `VoiceCount`: a held note is a source obligation, a voice is a resource, and more notes can be held than sounded |
| `TapCount` | taps | Observation surface |
| `SlotCount` | slots | Modulation, script host, script state, script output |
| `InstructionCount` | instructions | Script work per program. The per-quantum aggregate is a `ResourceReport` quantity, not a profile field |
| `PreparedBytes` | bytes | Three separate fields; the type carries the unit, the field carries the kind |
| `CostRatio` | dimensionless | Predicted quantum cost over the quantum's real-time budget |

**The rate type is V2's own, because V1's clamps.** `synth_core::SampleRate::new` turns `NaN`, zero, and negative into
`1.0`, so a constructor handed one cannot tell invalid input from a genuine 1 Hz endpoint, and `HOST-INV-018` asks every
quantity field for a *fallible* constructor. Two earlier revisions tried to work around this — reusing the existing
type, then taking raw `f32` Hz at every V2 constructor — and the second traded a silent clamp for an untyped public
argument, which the repository's newtype rule forbids outright. The resolution is the one the specification already
applies one row up: ADR-0021 part 3 replaces V1's clamping `VoiceCount` rather than reusing it, and V1's `SampleRate`
clamps for the same reason and gets the same treatment. V2's `SampleRate` has a private field and a fallible constructor
that rejects a non-finite or non-positive rate.

**The conversion is one-way, and the asymmetry is the point.** V2 to `synth_core` is infallible and provided: the value
has already passed V2's validation, and the permitted `synth_dsp` kernels take `synth_core::SampleRate`, so without it
the only way to reach a kernel would be to unwrap to `f32` and rebuild — an untyped hop at exactly the boundary this
rule protects. `synth_core` to V2 does **not** exist. A third revision offered it as fallible and claimed that kept
clamped values out; it cannot. `SampleRate::new(0.0)`, a negative rate, and `NaN` all arrive as `1.0`, which no
conversion can tell from a rate of one hertz, so a fallible signature would advertise a guarantee it does not hold. A
phase that must
admit a rate from a V1 surface validates the raw value where it is still available, and constructs V2's type directly.

**Ownership.** The application constructs the profile off the audio thread and hands it to the compiler. The compiler
reads it and produces a prepared plan plus a `ResourceReport`. The renderer reads the prepared plan and never the
profile. Nothing else holds a reference: a profile is an argument, not a service.

**Offline and test rendering.** An offline render has no device to query, so `CapabilitySource::Offline` records that
the capability half was declared rather than discovered, and a report or receipt that quotes a profile can say which
it was.
HOST-INV-003 is therefore **scoped to the device path** rather than universal: a job that declares its capabilities
through `::offline` or `::harness` is honest, and a device path that fills one in from a constant is the defect. Review
found the earlier unconditional wording unsatisfiable — it forbade the offline path the same model provides, and it
asked for a
conformance test no runtime tag can pass, since a `Device` tag cannot prove that a query happened. One constructor per
source moves what can be guaranteed from a claim about values to a property of the API shape, and HOST-INV-003 states
exactly how far that reaches.

## The field set

Every default below carries a **basis**, which is one of:

- **Queried** — established at preparation from the device; no default exists and none may be compiled in.
- **Derived** — computed from another field or from a measurement by a stated rule, where the rule and not the taste
  picks the number. Reversing the arithmetic reproduces the value.
- **V1 carry-over** — V1's constant, adopted unchanged so that V2 refuses where V1 silently truncated, without also
  changing what fits. The behaviour changes; the number does not.
- **Chosen** — neither computed nor inherited. Where a measurement informed the choice without determining it, the row
  says **chosen, anchored on** that evidence: the anchor says the value was checked against something real, and the
  label says a different reviewer could have picked a different number from the same data.

**No value in this specification is a measured capacity.** EVD-0003 measured *cost*, not capacity, and the ledger's
third pass measured nothing at all. **Exactly one field is derived from measurement** — `predicted_quantum_cost_ratio`,
where the stated target (the worst observed block reaches its deadline) and the measured spread together fix the
number. Two more are chosen and anchored on EVD-0003: `max_active_voices` and `prepared_immutable_bytes`. An earlier
revision labelled all three *derived*, and review was right that neither of the latter two has a rule that produces its
value — 512 and 64 MiB were picked and then checked, which is a different and weaker thing. Everything else is queried,
carried over, or chosen. Nothing may be tuned to a value in these tables — no layout sized by it, no test asserting it
as a constant — before its revisit point.

### Capabilities

**Two closed sets, and the conformance test closes both.** `HostCapabilities` has exactly **four** members. Exactly
**three** of them are queried capabilities — `sample_rate`, `maximum_block_size`, `channel_layout` — which is the set
ADR-0021 part 4 admits and the set `HOST-INV-005` closes. The fourth, `source`, is the provenance tag those three are
stamped with; it is admitted by `HOST-INV-003` instead, because nothing queries it. Stating both numbers is deliberate:
one number alone made three earlier revisions of this section ambiguous about which rule covers `source`.

| Field | Type | Default | Basis | Replaces | Revisit |
|-------|------|---------|-------|----------|---------|
| `sample_rate` | `SampleRate` | Queried | Queried | — | — |
| `maximum_block_size` | `FrameCount` | Queried; no compiled-in ceiling | Queried | `LIMIT-0001`, `LIMIT-0057` | Phase 9 |
| `channel_layout` | `ChannelLayout` | Queried on the device path; supplied by the caller on the declared paths. No compiled-in default | Queried; the [Sound Core render contract](spec-sound-core-render-contract.md) owns the currently admitted internal meaning | `LIMIT-0059` | Phase 2 for internal storage; Phase 9 for live multichannel support |
| `source` | `CapabilitySource` | Set by the constructor path, not passed as a free choice among all three variants | **Derived**, by the stated rule that the constructor sets it — **not queried.** `HOST-INV-003`'s every-capability-is-an-argument rule scopes to the three host values above, and that invariant states exactly what the split buys: an unqueried value must be written at the call site, no `Default` exists, and a device-less path has an honest constructor to use. It is not a forgery-proof tag, and two earlier revisions claimed it was | — | — |

**The accepted rate range is a render limit, and this table used to carry it.** It moved to
[*Stream*](#stream) below, and two failed attempts are worth recording because each broke something different. An
earlier revision left it here, in the half defined as *queried, never chosen*, while `LIMIT-0004` classifies it as a
budget the operator configures. The revision after that demoted it to a constructor constant, which would have changed a
settled classification without a successor decision and taken the range out of the report. What settles it is the ledger
entry's own rule — a job outside the range "is refused **at admission**" — together with `HOST-INV-005` ground 1, which
says only `HostProfile`-owned entries participate in admission. A capability is what the plan is *prepared against*; a
limit is what admission *refuses on*. The range refuses, so it is a limit, and the closed capability set is the four
fields above, which is what the types sketch always had.

**On the block ceiling.** V1 has two numbers: an engine-wide `MAX_BLOCK_SIZE` of 4 096 frames and a hardcoded advertised
request range of 128–1 024. V2 keeps neither as a constant. The queried value sizes the carries (HOST-INV-012), so an
implausibly large device block becomes a prepared-memory question the memory budget answers, rather than a separate cap
that can disagree with the memory it implies. A callback larger than the queried maximum remains ADR-0021 part 3's
terminal stream-contract fault.

**There is no floor either.** A device whose largest block is smaller than one quantum is admitted unchanged; see
HOST-INV-012 for why the render model already handles it and why an earlier `maximum_block_size >= Q` clause was a
defect rather than a safety margin.

### Stream

| Field | Type | Default | Basis | Replaces | Revisit |
|-------|------|---------|-------|----------|---------|
| `accepted_sample_rates` | `SampleRateRange` | 8 000 – 192 000 Hz | **One basis per endpoint.** Maximum: **Derived** from `DeviceSampleRate::MAX_SUPPORTED`, which `MAX_RENDER_SAMPLE_RATE` already derives from after pass 3's fix — reversing the arithmetic reproduces it. Minimum: **Chosen** — below telephone quality nothing this engine ships is usable, and no rule produces 8 000 | `LIMIT-0004` | Phase 9 |

The one limit that does not bound a plan's size. It bounds which streams may be prepared at all, and `HOST-INV-016`
validates `sample_rate` against it at construction, so a profile cannot be built at a rate its own range excludes. That
cross-half relation is exactly what `HOST-INV-017` requires of a relation between capacities: declared once, in the
constructor.

It is also the one limit **no plan can exceed**, because no plan carries a rate — see `HOST-INV-007`'s narrowing. Its
refusal is a construction failure naming both fields, not a `CompileError`, and its conformance cases sit with
`HOST-INV-016`. Both endpoints are inclusive, so a host that supports exactly one rate is expressible.

**`LIMIT-0004`'s job-admission error is not this field's to deliver.** The ledger requires an out-of-range *job* to be
refused with an error naming the requested rate and the profile range. That check runs where a job's requested rate is
read, which is ADR-0028's job contract, `Deferred` to the Phase 4 entry gate. This field is what such a check reads; it
is not the check. The obligation is listed under [*Unresolved questions*](#unresolved-questions) so it is not lost.

**Raisable, but not past the engine ceiling.** This is a render limit, so an operator may widen it — within
`DeviceSampleRate::MAX_SUPPORTED`, which the constructor enforces. The ceiling is not an operator setting for the same
reason `HOST-INV-008` keeps a node's intrinsic capacity out of the profile: real-time look-ahead and scratch buffers are
sized from it, and a stream above it gets less DSP than its parameters advertise with no diagnostic. That is exactly the
defect `LIMIT-0004` records — V1's render command accepted 384 kHz against a 192 kHz engine and silently halved the
limiter's look-ahead — so a field replacing it must not be able to re-open it. Moving the ceiling is an engine change,
not a profile change.

### Graph

| Field | Type | Default | Basis | Replaces | Revisit |
|-------|------|---------|-------|----------|---------|
| `max_nodes` | `NodeCount` | 16 384 | Derived: the largest of the 68 built-in patches declares 16 voice modules, so the 512-voice budget below implies 8 192 voice nodes; doubled for effect chains, buses, and note/mod graphs | `LIMIT-0060` | Phase 2 exit |
| `max_edges` | `EdgeCount` | 65 536 | Derived: `4 x max_nodes`. The densest built-in patch is 16 modules and 18 connections, 1.13 edges per node, so this is roughly 3.5x the densest shipped material | `LIMIT-0060` | Phase 2 exit |
| `max_fan_out_per_port` | `FanOut` | 64 | Chosen. V1 has no bound and no measurement exists | `LIMIT-0060` | Phase 2 exit |
| `max_mod_graph_nodes` | `NodeCount` | 32 | V1 carry-over | `LIMIT-0025` | Phase 7 |
| `max_note_graph_nodes` | `NodeCount` | 32 | V1 carry-over | `LIMIT-0026`, `LIMIT-0067` | Phase 7 |

**`LIMIT-0060` is the entry this group exists for.** V1 holds connections in an uncapped `Vec` with unbounded fan-out,
and pass 2 confirmed the absence rather than assuming it. Making the three graph extents profile fields is what turns
"unbounded because nobody decided" into "bounded, reported, and raisable".

**`LIMIT-0067` changes shape here, not size.** V1 silently dropped note-processor rack stages beyond 32 at *load*. Under
ADR-0021 part 3 the document loads with every stage intact and compilation refuses the excess. Keeping 32 means the same
racks compile as before; what changes is that the excess is now a named refusal, and that raising it is a profile change
rather than a code change.

### Voices

| Field | Type | Default | Basis | Replaces | Revisit |
|-------|------|---------|-------|----------|---------|
| `voices_per_instrument` range | `VoiceCount` pair | 1 – 128 | V1 carry-over; the clamp becomes a refusal | `LIMIT-0056` | Phase 6 |
| `max_active_voices` | `VoiceCount` | 512 | Chosen, anchored on EVD-0003 — see below | — | Phase 6 |
| `max_held_notes` | `HeldNoteCount` | 512 | Chosen: the same number as `max_active_voices` but **not derived from it** — the types do not convert — and **not** the per-instrument ceiling. See below | `LIMIT-0031` | Phase 6 |
| `retirement_crossfade` | `FrameCount` | 128 frames | V1 carry-over; **ADR-0009 owns the value** | `LIMIT-0030` | Phase 9 |
| `max_concurrent_retiring_voices` | `VoiceCount` | `max_active_voices` | Derived, so that it cannot bind — see below. **ADR-0009 owns any lower value, and the behaviour that would come with it** | — | Phase 9 |

**Where 512 comes from, and what it does not mean.** EVD-0003 measured V1 at `1.173 ms/s per voice + 0.428 ms/s` with
R² = 1.0000 over a 64-fold range. At 512 voices that is about 600 ms of CPU per second of rendered audio — roughly 60%
of one core of the measured machine at 44.1 kHz. The same 512 voices at 192 kHz cost about 2.6 s per second of audio,
which is not real time on that core at all.

That is the argument for HOST-INV-011 stated in numbers: **a voice count is not a CPU budget.** The count bounds what
the compiler must prepare; the [cost budget](#cost) is what notices that the same plan is affordable at one rate and not
at another. A profile that carried only the count would admit both cases identically.

The figure is also a property of one patch. EVD-0003's polyphony probe holds module count per voice, instrument count,
and effect depth fixed, so 1.173 ms/s is the cost of *that* voice, not of a voice. And 512 is not produced by any rule:
it was picked and then checked against the slope, which is why its basis is *chosen, anchored* rather than *derived*.
Phase 6 revisits it against a real allocator.

**A held note is not a voice, and the first version of this field assumed it was.** `max_held_notes` was initially set
to 128, equal to the per-instrument voice ceiling, on the reasoning that held-note tracking would then never bind before
polyphony did. That reasoning is wrong: a sustain pedal, a voice-stealing allocator, and an MPE or sequencer source can
all hold more notes than there are voices to sound them — the allocator has to remember a held note precisely so that it
can re-sound when a voice frees. The count is now 512, engine-wide, and carries its own type so the two concepts
cannot be assigned to each other. Whether even that is enough is Phase 6's, with the allocator in front of it.

**The equality with `max_active_voices` is a coincidence of value, not a derivation, and the field table now says so.**
HOST-INV-018 keeps `HeldNoteCount` and `VoiceCount` unconvertible — that is the whole point of the correction above —
so no arithmetic can express "equal to `max_active_voices`", and nothing would fail if Phase 6 raised the voice count
and left this one at 512. That is the opposite arrangement from `max_concurrent_retiring_voices`, which is genuinely
derived and shares its operand's type precisely so the derivation is expressible. A field whose stated coupling cannot
be written in code is a comment, and it drifts; Phase 6 either restates 512 on its own footing or gives the two a
shared rule that survives being typed apart.

Nor does 512 promise that every producer class can fill the held-state pool independently of the event budgets.
Compiled notes carry plan-admitted releases; non-compiled notes need separately typed `EventCount` hold entitlements
under HOST-INV-021. A plan can therefore bind on held state, compiled occupancy, or non-compiled release holds, and the
`ResourceReport` names which one. No conversion between their newtypes is implied.

**The retirement budget cannot be allowed to bind, and the closure pass found that it could.** It was first set to 64,
one eighth of `max_active_voices`, as a chosen number. But how many voices retire at once is a *runtime* quantity — a
plan swap retires whatever is sounding — so with 512 voices active and a swap arriving, more than 64 voices would want
to retire and the specification said nothing about what happens then. That is the same defect the previous pass found
in the recording capacities and in the event scratch: a field enforced at runtime with no defined behaviour on reaching
it. Here there is no acceptable answer to invent, either — a voice cannot be refused retirement, and stopping the
excess without a crossfade would be an audible degradation that HOST-INV-019 forbids for render output.

So the field is derived from `max_active_voices` and cannot bind. What it still does is *account*: the crossfade
buffers it implies are real prepared memory, and they appear in the `ResourceReport` under the memory budget. ADR-0009
may lower it later, but only together with a defined behaviour for reaching it — which is the work this correction
declines to do on ADR-0009's behalf.

**`LIMIT-0031` is not a `HostProfile` row, and still lands here.** Its ledger owner is `N/A — removed`, because V1's
32-element `Vec::with_capacity` is a hint rather than a bound and exceeding it allocates on the audio thread. Its
disposition says the replacement "is admitted like any other profile capacity", which creates a profile field. It is
recorded here so the entry's closure has a visible successor.

### Events

| Field | Type | Default | Basis | Replaces | Revisit |
|-------|------|---------|-------|----------|---------|
| `max_events_per_quantum` | `EventCount` | 256 until Phase 3 reselects it from measured producer shares before enabling ingress | **Chosen and unevidenced**, not carried over: `LIMIT-0014`'s constant is an egress ring size, while V1's real per-block buffer has no cap. Fitting within 256 does not waive reselection; see below | `LIMIT-0075` — the unbounded `Vec` this field replaces | Phase 3, before ingress |
| `compiled_event_share` | `EventCount` | Absent in Phase 1–2; selected before Phase 3 enables ingress | ADR-0046 fixed profile input for compiled timeline and automation | — | Phase 3 |
| `authored_runtime_event_share` | `EventCount` | Absent in Phase 1–2; selected before Phase 3 enables ingress | ADR-0046 fixed profile input for admitted data-dependent expansion | — | Phase 3 |
| `live_event_share` | `EventCount` | Absent in Phase 1–2; selected before Phase 3 enables ingress | ADR-0046 fixed profile input covering complete eligible live snapshots | — | Phase 3 |
| `session_event_share` | `EventCount` | Absent in Phase 1–2; selected before Phase 3 enables ingress | ADR-0046 fixed profile input covering complete session snapshots and locate catch-up | — | Phase 3 |
| `internal_event_share` | `EventCount` | Absent in Phase 1–2; selected before Phase 3 enables ingress | ADR-0046 fixed profile input for admitted renderer-internal producers | — | Phase 3 |
| `release_event_share` | `EventCount` | Absent in Phase 1–2; selected before Phase 3 enables ingress | ADR-0046 fixed profile input for individual redemptions of non-compiled release holds | — | Phase 3 |
| `release_hold_capacity` | `EventCount` | Absent in Phase 1–2; selected before Phase 3 enables ingress | ADR-0046 fixed profile input for disjoint non-compiled producer hold entitlements | — | Phase 3 |
| `max_note_expansion_per_tick` | `EventCount` | 128 | V1 carry-over. Contributes to HOST-INV-021's conservative authored-runtime envelope; it is not a licence to trim expansion | `LIMIT-0043` | Phase 3 |
| `max_scheduled_events_in_flight` | `EventCount` | Phase 1–2: 4 096. Phase 3: `compiled_event_share * max_quanta_per_callback + 4 096` | The existing chosen value becomes authored-future headroom above a derived compiled floor when ADR-0046 is enabled, with checked arithmetic. Plan admission charges simultaneously retained authored future events to that headroom; a full store may not make an admitted event late | — | Phase 3 |
| `forward_event_horizon` | `FrameCount` | `max(one second at the prepared rate, maximum_block_size + Q)` | Chosen, with a derived floor — see below | — | Phase 3 entry (ADR-0022) |
| `command_queue_capacity` | `EventCount` | 16 384 | V1 carry-over. **Live bounded queue** for legacy engine-command ingress under HOST-INV-009; it is not ADR-0046's Phase 3 session/transport store, whose caller-boundary refusal contract is non-dropping. Reusing this physical queue for that store requires an accepted change that removes this classification and preserves refusal before timestamped acceptance | `LIMIT-0012` | Phase 9 |
| ~~`event_queue_capacity`~~ (critical / high / normal / low) | — | **Withdrawn** | **This field is removed from the profile, and the removal is the correction rather than a simplification.** It was a V1 carry-over of `LIMIT-0013`'s four prioritized rings, admitted through HOST-INV-005 ground 1 by that ledger entry's `HostProfile` ownership. [ADR-0038](../decisions/ADR-0038-engine-egress-queue-classification.md) part 3 establishes that the prioritized channel has no workspace production caller. Its public export leaves external use unknown, so removal is an explicit compatibility break rather than an unreachability claim. The entry moves to `N/A — removed` and this field loses its only admissibility ground. The source evidence belongs to the [resource-limit inventory](../inventories/resource-limits.md), not to a copy in this specification. Sizing a V2 capacity from four rings with no observed workspace production use would carry an unvalidated shape forward: nothing has established that four priority tiers, or these four numbers, are right for anything. V2's engine egress is ADR-0038 part 1's rule with a capacity per surviving entry — `event_egress_capacity` for the GUI ring, and the protocol contract's own for `LIMIT-0076`. If V2 wants priority classes on egress it designs them from a requirement | ~~`LIMIT-0013`~~ — none; the entry is `N/A — removed` | Withdrawn |
| `event_egress_capacity` (engine-to-GUI event ring **only**) | `EventCount` | 256 | V1 carry-over of what `LIMIT-0014`'s constant sizes on the GUI side. **The field is one capacity, not two, and that is a change.** Earlier revisions carried `EventCount x 2` because one V1 constant sizes a GUI ring and an OSC note-telemetry ring; [ADR-0038](../decisions/ADR-0038-engine-egress-queue-classification.md) part 4 splits the ledger entry, and the OSC ring becomes `LIMIT-0076` owned by the protocol contract that serializes it — **not a profile field**. The two may take different sizes from that point on, which is the reason to split them. **Loss semantics are ADR-0038's, not HOST-INV-009's**: this is engine *egress*, which does not meet ADR-0021's definition of a live bounded queue, so dropping is licensed by ADR-0038 part 1 and only under its three conditions — observational payload, counted drop, count in the structured diagnostics report. V1 satisfies none of the three on this ring, which is why the field's conformance work is Phase 5's rather than Phase 1's. **`RecordedNotesFlushed` is custodial under ADR-0038 part 2 and may not share this capacity** | `LIMIT-0014` | Phase 5 |

#### Renderer-ingress source-store registry

This table and the Events field table above are HOST-INV-009's complete **live-input** drop-licence registry; ADR-0038's
engine-egress drop licence is separate. Every concrete Phase 3 renderer-ingress store gets exactly one row here, even
when its capacity is also a `HostProfile` field; the capacity column then references that field. Only this registry row
carries the classification for a renderer-ingress store; a corresponding Events field row references it without
repeating the marker. Rows may be added only with the admitting ground required under *Deferred to Phase 3*. A live
**renderer-ingress** source store absent from this table is not permitted to drop. Non-dropping session/transport
stores do not belong here.

| Store | Capacity owner | Classification | Status |
|---|---|---|---|
| *(none in Phase 1–2)* | — | — | Phase 3 replaces this placeholder with one row per concrete renderer-ingress store before enabling ingress |

**The scheduled-event window is a prepared sizing relation, not an overload policy.** Phase 3's publication
arbiter materializes compiled events only for the imminent render call. A profile therefore validates
`max_scheduled_events_in_flight >= compiled_event_share * max_quanta_per_callback` with checked arithmetic, where
`max_quanta_per_callback = ceil(maximum_block_size / Q)`. The current carry can only reduce a particular call's
quantum count. Separate sealed-batch storage covers `max_events_per_quantum * max_quanta_per_callback`. A full window
cannot delay an admitted compiled event into lateness. Plan admission additionally checks the compiled callback window
against that derived floor and the plan-wide aggregate authored-runtime maximum of simultaneously retained future
events against the headroom above it; the floor is reserved for the compiled class rather than lent to a plan whose
compiled window is sparse. Failure to publish an entitlement before its quantum is a producer-contract defect.

The default adds 4,096 events of provisional authored-future headroom above the compiled floor rather than taking the
larger of the two. Taking `max` would leave zero headroom whenever the compiled product reached 4,096. The addition is
checked and Phase 3 must measure its memory cost and usefulness.

The Phase 1 allocator currently adds one slack quantum to its scratch extent although the public
`quanta_needed_for` calculation proves carry cannot increase the exact ceiling above
`ceil(maximum_block_size / Q)`. Phase 3's sealed-batch preparation uses the exact checked relation above and removes or
documents that old slack; the comment beside the Phase 1 allocation is not a second normative formula.

**The event cap has no V1 value to inherit.** The resource inventory establishes that `LIMIT-0014` and `LIMIT-0076`
are egress rings, while `LIMIT-0075` is V1's uncapped per-block sequencer `Vec`. The current value 256 is chosen and
unevidenced. Phase 3 must reselect it from the measured producer-share partition before enabling ingress, even when the
measured partition would fit within 256. [EVD-0015](../evidence/phase-03/EVD-0015-quantum-occupancy.md) measures a peak
of 36 in the 23 projects whose streams can be derived, but also shows why that observation cannot establish a safe cap:
expansion is bounded per tick while renderer occupancy is per quantum, and releases from different production times can
converge.

HOST-INV-021 therefore partitions the cap instead of treating producer limits as additive evidence. Construction
checks plan-independent relations; plan admission checks the session, internal and authored declarations against the
fixed shares. Every share is a positive `EventCount` capacity. A disabled live, authored, session or internal class
leaves its share unused rather than representing a capacity with `EventCount::NONE`. `release_hold_capacity` is also
a positive capacity, and the release share covers at least that many individual redemptions.

**Data-dependent expansion is materialized once, but admitted conservatively.** `max_note_expansion_per_tick`
contributes to a source's destination-occupancy, retained-future and simultaneous-hold declarations together with
simultaneous placements, rate, tempo, legal loops and every anchor phase. The compiler sums those declarations across
all simultaneously legal authored sources unless it mechanically proves mutual exclusion, then refuses a plan whose
aggregate envelopes do not fit their fixed share or stores. Runtime evaluation writes the actual batch into
preallocated scratch and publishes its live prefix atomically. It never reserves 128 merely because the scratch can
hold 128, and it never trims the prefix to fit.

**The publication arbiter is the one capacity boundary.** It filters stale epochs and foreign plan slots before
charging shares, snapshots bounded input, and seals the call's quantum batches. It does not re-evaluate the forward
horizon: HOST-INV-013 places that check exactly once, at ingress admission into bounded source storage, so an entry
that has merely waited for this pass is never rejected for it. New live input that cannot enter its bounded source
storage is dropped and counted under HOST-INV-009. An entry already in the
eligible snapshot is never dropped for renderer capacity; if it has become late, ADR-0043's preserving clamp and late
counter apply.

Future non-compiled releases use `release_hold_capacity` rather than queued future quantum containers. Plan admission
partitions it into disjoint producer entitlements. A live note-on always acquires a hold; another producer may omit one
only when the complete note-on/release pair is already present in one indivisible materialized open-window batch.
An individual release redeems one hold into the release share. A panic or sustain-lift event remains charged to the
live share, a transport-stop event remains charged to the session share, and an authored mass-release event remains
charged to the authored share. The allocator applies the bounded operation inside that event and atomically redeems
all affected holds as a side effect; it emits neither a second release-share event nor one renderer-internal event per
voice. Thus every mass-release cause stays inside its source's admitted complete snapshot or plan-wide envelope, while
the release share remains available for individual redemptions. Compiled releases use their admitted plan
entitlement.

There is no capacity deferral, deferred store, admission tail, starvation policy or capacity-displacement counter.
Raising `max_events_per_quantum` without satisfying the share relations remains invalid, and no render loop may
allocate to absorb an over-full quantum.

### Observation

| Field | Type | Default | Basis | Replaces | Revisit |
|-------|------|---------|-------|----------|---------|
| `max_observation_taps` | `TapCount` | 128 | V1 carry-over; the silent drop becomes a compile error. **ADR-0027 owns what a tap is** | `LIMIT-0020`, `LIMIT-0062` | Phase 5 |
| `telemetry_ring_frames` | `FrameCount` | 4 096 | V1 carry-over. **Lossy** (HOST-INV-019) — but a **behaviour change**, not a carry-over of behaviour: V1 drops the *newest* samples, as recorded by `LIMIT-0021`. **Since `bac88c0c` the loss is no longer silent**: `read_samples_into` returns the omitted count under `#[must_use]`, which satisfies ADR-0038 part 1 condition 3 in its data-paired form. What is still missing is presentation — no GUI surface draws the gap — so HOST-INV-019's *expose the loss* condition is met at the API and not yet at the surface | `LIMIT-0021` | Phase 5 |
| `analyzer_fft_size` | `FrameCount` | 2 048 | V1 carry-over. A resolution budget; the size travels with the payload | `LIMIT-0022` | Phase 5 |

**The class ADR-0021 gave these rings does not fit them, and this specification may not change it.** ADR-0021 part 1
reserves runtime overflow for queues "fed by external, unbounded-in-time input such as MIDI or user gestures". These
are fed by the engine. The *behaviour* the record chose is still defensible — visualization data is droppable,
diagnostics are not, which is why V1 has four priorities — but the class it was chosen under describes a different kind
of queue, and the record made that classification while the direction was misread here as well. Correcting an accepted
decision is not this specification's to do, so the conflict required an accepted successor; ADR-0038 later supplied
it. HOST-INV-009's live-input drop licence therefore does not cover engine state and diagnostic events merely because
of a label.

**ADR-0021's `LIMIT-0013` evidence was false in both halves, and that was a third accepted-decision conflict.** The
record's decision drivers said that `LIMIT-0013`'s per-priority drop counters were published on OSC. The use-site audit
recorded by ADR-0038 found that the counters have no production reader and OSC publishes a different ring's counter.
The conclusion survives and gets stronger — a counter nobody reads is worse than a counter read over OSC — while
ADR-0038 owns the correction to the accepted decision.

**Nor is `Critical` undroppable.** ADR-0038 records that the producer uses the same fallible send shape for every
priority and increments the corresponding drop counter on failure. The code gives critical events a separate ring,
not a delivery guarantee. **No field here may promise non-droppable diagnostics**, and an earlier revision of this
section did.

`LIMIT-0020` is the field's reason for existing: V1 publishes meters through a 128-slot array whose `publish()` is an
`if let Some(slot)`, so a project with more metered channels loses meters with no signal to anyone. Under ADR-0021 part
3 that is a compile error naming the requested and available tap counts.

### Mixing

| Field | Type | Default | Basis | Replaces | Revisit |
|-------|------|---------|-------|----------|---------|
| `max_mix_channels` | `MixChannelCount` | 256 | Chosen | — | Phase 8 |
| `max_buses` | `BusCount` | 64 | Chosen | — | Phase 8 |
| `max_sends_per_channel` | `SendCount` | 16 | V1 carry-over. **ADR-0034 owns what a send is** | `LIMIT-0024` | Phase 8 |

A plan with 200 metered mix channels is admissible by `max_mix_channels` and refused by `max_observation_taps`, with the
tap budget named as the dominant contributor. That is the intended shape: the two budgets are allowed to disagree, and
the report says which one bound the plan. V1's version of the same situation was a meter that stopped appearing.

### Memory

| Field | Type | Default | Basis | Replaces | Revisit |
|-------|------|---------|-------|----------|---------|
| `prepared_immutable_bytes` | `PreparedBytes` | 64 MiB | Chosen, anchored on EVD-0003 — see below | Admits `LIMIT-0073`'s per-node declarations in aggregate | Phase 2 exit |
| `mutable_state_bytes` | `PreparedBytes` | 32 MiB | Chosen. Scales with polyphony, so it is bounded jointly with `max_active_voices` | Admits `LIMIT-0073`'s per-node declarations in aggregate | Phase 2 exit |
| `buffer_scratch_bytes` | `PreparedBytes` | 16 MiB | Chosen. The carries are the one part that computes: two of `maximum_block_size + Q` frames are 32 KiB each at 4 096 frames and stereo `f32`. The remaining budget is per-node scratch, which nothing measures, so the field is a chosen ceiling and not an arithmetic result | `LIMIT-0002`, `LIMIT-0003` | Phase 2 exit |

**Where 64 MiB comes from, and what it is not.** EVD-0003 reports the render phase adding at most 5.33 MiB of resident
set across the four corpus cases, with peak process RSS of 14–30 MiB dominated by process warm-up rather than by the
engine. The largest single prepared allocation the ledger records is a 96 000-sample buffer — a granular source or a
convolver impulse response, about 375 KiB as `f32` (`LIMIT-0073`). 64 MiB is roughly an order of magnitude above the
measured corpus and still catches a plan that instantiates a hundred convolvers.

**RSS is not prepared bytes**, and EVD-0003 says so itself: resident set is the kernel's view, an upper bound on what a
phase kept and a lower bound on what it touched. The number above is anchored on that measurement, not derived from it
— which is why its basis cell says *chosen, anchored*. An earlier revision labelled it *derived* in the table while
this paragraph said the opposite two lines later; review caught the contradiction, and the table now agrees with the
prose.
HOST-INV-014 is what keeps the two apart in the implementation: the budget is checked against the compiler's aggregate
over prepared nodes, which is a quantity V1 never computes anywhere. That absence is `LIMIT-0073`'s finding, and
producing the aggregate is a Phase 2 deliverable rather than a profile setting.

### Scripts

The master plan asks for "YAMS instructions/state/emits multiplied by scope and polyphony". V1 has only the per-program
half, and that half is the whole of this group: **the aggregate is a reported quantity in the `ResourceReport`, not a
profile field.** See below.

| Field | Type | Default | Basis | Replaces | Revisit |
|-------|------|---------|-------|----------|---------|
| `max_instructions_per_program` | `InstructionCount` | 256 | V1 carry-over | `LIMIT-0032` | Phase 7 |
| `max_sources_per_program` | `SlotCount` | 32 | V1 carry-over | `LIMIT-0033` | Phase 7 |
| `max_state_slots_per_program` | `SlotCount` | 16 | V1 carry-over. Prepared memory times polyphony | `LIMIT-0034` | Phase 7 |
| `max_locals_per_program` | `SlotCount` | 16 | V1 carry-over | `LIMIT-0035` | Phase 7 |
| `max_eval_stack_depth` | `SlotCount` | 64 | V1 carry-over | `LIMIT-0036` | Phase 7 |
| `max_arrays_per_program`, `max_array_elements` | `SlotCount` | 16 / 256 | V1 carry-over | `LIMIT-0039` | Phase 7 |
| `max_emits_per_program` | `SlotCount` | 4 | V1 carry-over | `LIMIT-0042` | Phase 7 |
| `mod_matrix_slots_per_voice` | `SlotCount` | 16 | V1 carry-over. **Two fields with a floor relation**, not one — see below | `LIMIT-0023` | Phase 7 |
| `script_host_slots_per_voice` | `SlotCount` | 16 | V1 carry-over. Validated `>= mod_matrix_slots_per_voice` at construction (HOST-INV-016), which is the relation V1's assertion actually states | `LIMIT-0041` | Phase 7 |

**Two capacities with a floor relation, and the reason is weaker than an earlier revision claimed.** `LIMIT-0023`
(Mod Matrix slots per voice) and `LIMIT-0041` (script host slots) were collapsed into a single field on the ledger's
claim that V1 holds them "coupled 1:1 by a compile-time assertion". The use-site audit recorded in the
[resource-limit inventory](../inventories/resource-limits.md) found an inequality, not equality. Lowering the host
slots below the matrix count breaks the build; raising them alone is legal. It also found no second production
consumer. So the split rests on the inequality alone — V1 permits divergence and does not use it — which makes two
fields a **V2 design choice** rather than a reading of V1, and Phase 7 may reasonably collapse them again.

The profile therefore carries two fields and validates the floor at construction, which is where HOST-INV-017 still
applies: the *relation* is declared once, in the constructor, instead of being maintained by a compile-time assertion
in a third crate.

**The aggregate is reported, not budgeted, and that is a structural choice rather than a gap.** Instructions per program
times scopes times voices is the quantity that actually costs CPU, and nothing in V1 or in the evidence measures what an
instruction costs. An earlier revision put a `max_script_instructions_per_quantum` field in `RenderLimits` with the
value *unset*, which review correctly rejected: a limit with no value is not a limit, and it contradicted this
specification's own promise of a value for every field. A quantity the compiler computes and reports belongs in the
`ResourceReport`, which is where it now lives — `script_instructions_per_quantum`, present from Phase 1, with no
threshold attached. It becomes a `RenderLimits` field when Phase 7 measures a per-instruction cost that can justify a
number. The master plan's coverage is satisfied either way: the quantity is accounted for, and the report names it as a
contributor.

### Recording

| Field | Type | Default | Basis | Replaces | Revisit |
|-------|------|---------|-------|----------|---------|
| `max_held_notes_per_take` | `HeldNoteCount` | 32 | V1 carry-over. **Session limit** (HOST-INV-020). **ADR-0024 owns take semantics** | `LIMIT-0051` | Phase 9 |
| `max_recorded_events_per_take` | `EventCount` | 4 096 | V1 carry-over. **Session limit** (HOST-INV-020). **ADR-0024 owns take semantics** | `LIMIT-0051` | Phase 9 |

Capacity is preallocated and restored after flush, as V1 already does, so the audio thread never reallocates. A take
that would exceed the capacity stops with a counted diagnostic rather than dropping notes.

**These two are the profile's only runtime-enforced fields, and the contract needed a class for them.** How long a take
runs is not knowable when the plan is compiled, so HOST-INV-007's compile-time refusal cannot apply and HOST-INV-009's
queue drop must not: the notes are authored data the moment they are played. HOST-INV-020 is the resulting third
behaviour — stop the activity, count it, keep everything already recorded. Review found the earlier draft asserting
compile-time refusal for every render limit while this section described a runtime stop, with nothing reconciling them.

### Cost

| Field | Type | Default | Basis | Replaces | Revisit |
|-------|------|---------|-------|----------|---------|
| `predicted_quantum_cost_ratio` | `CostRatio` | 0.15, **advisory** | Derived from EVD-0003 — see below | — | Phase 3 (simulated host) |

This is the only field in the profile that is not a count, and the only one the evidence genuinely drives.

EVD-0003 measured, per corpus case, the time one block takes against the real-time budget that block would have had. Two
figures matter. The median block is cheap — 0.34% to 4.91% of budget depending on case and rate. The **maximum** block
is 2.7x to 6.8x the median, on a machine that was not quiesced and had no competing audio thread. A policy sized on
median cost would therefore be sized four to seven times too optimistically, which is EVD-0003's own conclusion.

0.15 is the median-cost threshold at which the worst observed spread reaches the deadline: `0.15 x 6.8 ≈ 1.0`. A plan
whose *predicted* median quantum cost exceeds 15% of the quantum's real-time budget is one whose worst block plausibly
misses, so it warns.

Three properties of this field are deliberate:

- **It never refuses** (HOST-INV-015). The cost model is a prediction from EVD-0002's per-block overhead and EVD-0003's
  per-voice slope, both measured on V1 and on one machine. A prediction that weak may warn; it may not decide.
- **It is a ratio of times, not of frames** (HOST-INV-011). The quantum's budget is `Q / sample_rate` seconds, so the
  same plan at 192 kHz is measured against a budget one quarter the size — which is exactly the asymmetry EVD-0003 found
  and warned that a frame-based policy would invert.
- **It has no real-time evidence behind it.** EVD-0003 measured offline throughput with no host, no device, and no
  callback deadline; every figure is a lower bound on the same work live. The simulated host that Phase 3 builds, and
  that ADR-0022 is deferred to, is what turns this from an advisory into something that could refuse.

## Coverage against the master plan and the ledger

**The master plan's list.** Maximum host block, channel layouts, sample-rate range, nodes, voices, channels, buses,
sends, event fan-out, delayed events, parameter/control/event slots, observation taps, prepared immutable bytes, mutable
state bytes, buffer/scratch bytes, crossfade/retirement budget, YAMS instructions/state/emits multiplied by scope and
polyphony, recording-result and real-time communication capacities, and ADR-0032 clause 21's forward event horizon —
each has a field above.

**Two plan terms are answered rather than carried**, and both are accounted for in the `ResourceReport` instead of
being budgeted in the profile.

- **"Parameter and control slots"** are not separate budgets. A parameter slot is prepared memory belonging to its node
  and a control slot is a buffer, so both are admitted through `prepared_immutable_bytes`, `mutable_state_bytes`, and
  `max_nodes` rather than through a count of their own. A separate count would have to be kept in step with the node
  budget by hand, which is the `LIMIT-0023`/`LIMIT-0041` failure mode HOST-INV-017 exists to prevent.
- **The script-work aggregate** — instructions times scope times polyphony — is computed and reported, with no
  threshold, until Phase 7 can justify one. See [*Scripts*](#scripts) for why a field with an unset value was the wrong
  shape for it.

**The ledger's 28 `HostProfile`-owned entries**, each with its successor field. The count has moved twice and both moves
are recorded rather than absorbed: it was 30 before the pass-4 use-site audit found `LIMIT-0015` to be four
deferred-drop channels and moved it to `N/A — removed`, and it reached 28 again when ADR-0038 removed `LIMIT-0013` — a
public channel with no workspace production caller, removed as an explicit compatibility break — and split
`LIMIT-0014`, whose GUI half is `HostProfile`-owned while its OSC half became `LIMIT-0076` under the protocol contract.
**The mapping is now total**; every previous revision of this table carried at least one entry with no settled owner:

| Entry | Field |
|-------|-------|
| `LIMIT-0001` | `maximum_block_size` |
| `LIMIT-0002`, `LIMIT-0003` | `buffer_scratch_bytes`; sized from `maximum_block_size` rather than set independently |
| `LIMIT-0004` | `accepted_sample_rates`, a `RenderLimits` field — see [*Stream*](#stream) |
| `LIMIT-0012` | `command_queue_capacity` |
| `LIMIT-0013` | **None.** The prioritized channel is `N/A — removed` under ADR-0038 part 3: it has no workspace production caller, public external use remains unknown, and initial V2 intentionally breaks that surface. It therefore has no successor field, and `event_queue_capacity` is withdrawn |
| `LIMIT-0014` | `event_egress_capacity` (the engine-to-GUI ring only, after ADR-0038 part 4's split) — **not** `max_events_per_quantum`, whose antecedent is `LIMIT-0075` |
| `LIMIT-0076` | None. The OSC note-telemetry ring split out of `LIMIT-0014` is owned by the protocol contract, so it has no successor field in this profile — listed here so the split is visible from the mapping rather than only from the ledger |
| `LIMIT-0075` | `max_events_per_quantum` — **provenance, not an admission ground**: the ledger owner is `N/A — removed`, so the field is admitted by HOST-INV-005's residual |
| `LIMIT-0020`, `LIMIT-0062` | `max_observation_taps` |
| `LIMIT-0021` | `telemetry_ring_frames` |
| `LIMIT-0022` | `analyzer_fft_size` |
| `LIMIT-0023` | `mod_matrix_slots_per_voice` |
| `LIMIT-0041` | `script_host_slots_per_voice`, floored by the above rather than equal to it |
| `LIMIT-0024` | `max_sends_per_channel` |
| `LIMIT-0025` | `max_mod_graph_nodes` |
| `LIMIT-0026`, `LIMIT-0067` | `max_note_graph_nodes` |
| `LIMIT-0030` | `retirement_crossfade` |
| `LIMIT-0032`..`LIMIT-0036`, `LIMIT-0039`, `LIMIT-0042` | The per-program script fields |
| `LIMIT-0043` | `max_note_expansion_per_tick` |
| `LIMIT-0051` | The recording fields |
| `LIMIT-0056` | `voices_per_instrument` range |
| `LIMIT-0060` | `max_nodes`, `max_edges`, `max_fan_out_per_port` |

Plus `LIMIT-0031`, whose ledger owner is `N/A — removed` but whose disposition creates `max_held_notes`.

**Fourteen fields have no V1 antecedent**, which is where V1 had nothing rather than something wrong:
`max_active_voices`, `max_scheduled_events_in_flight`, `forward_event_horizon`, `max_mix_channels`, `max_buses`,
`max_concurrent_retiring_voices`, `predicted_quantum_cost_ratio`, the six event-share fields, and
`release_hold_capacity`. ADR-0046 creates the last seven on accepted-ADR ground 2; their numeric defaults remain Phase
3 evidence rather than being invented in this specification.

**The historical count went to eight and back to seven, then ADR-0046 took it to fourteen**, which is worth recording.
`max_events_per_quantum` was listed here when the use-site audit showed `LIMIT-0014` is an egress ring; review then
found the real antecedent — `LIMIT-0075`, V1's
uncapped `sequencer_event_buffer`, which nobody had registered. The antecedent existed all along; the ledger pointed at
the wrong constant. Only the *value* 256 is unsupported by V1, which is why the field's basis stays *chosen*.

An earlier revision listed eleven, and review found the count wrong in three separate ways. The three memory aggregates
were listed as having no antecedent while their own rows named `LIMIT-0002`, `LIMIT-0003`, and `LIMIT-0073` — they are
new as an *aggregate*, which is `LIMIT-0073`'s finding, but the resources themselves are V1's.
`max_script_instructions_per_quantum` is no longer a field at all. And eleven items were summarised as ten in the phase
tracker and `STATUS.md`. The count has since moved three times more — to seven after that correction, to **eight** when
the use-site audit showed `LIMIT-0014` is an egress ring, and back to **seven** when review found the real antecedent,
`LIMIT-0075`. ADR-0046 later added seven genuinely new admission fields rather than revealing new V1 antecedents.

## Lifecycle and timing

1. **Construction** — off the audio thread. The capability half is queried from the device (or declared, with
   `CapabilitySource::Offline` or `Harness`); the limit half is supplied by the application. Construction validates
   every field and every cross-field relation, and is fallible (HOST-INV-016, HOST-INV-018).
2. **Admission** — off the audio thread, once per prepared plan. The compiler reads the profile, computes each
   resource's requested amount and its dominant contributor, and either produces a prepared plan or refuses with a
   `CompileError`. Either way it returns a `ResourceReport` (HOST-INV-006).
3. **Preparation** — off the audio thread. The prepared plan allocates its carries at `maximum_block_size + Q` frames,
   its buffers, and its node state. A `StreamEpoch` is issued (ADR-0032 clause 12), and sample rate, layout, and
   capacity are fixed for its life.
4. **Execution** — on the audio thread. The renderer reads the prepared plan. It reads no profile field, allocates
   nothing, and takes no lock.
5. **Retirement** — a plan swap crossfades over `retirement_crossfade` frames with at most
   `max_concurrent_retiring_voices` voices retiring at once, and the retired plan's memory is released off the audio
   thread. ADR-0009 owns the crossfade's semantics.
6. **Re-preparation** — required for any profile change, and by ADR-0021 part 3's terminal oversized-callback fault. A
   new epoch invalidates every timestamp of the old one (ADR-0032 clause 14).

## Failure and diagnostics

| Situation | Result | Where it is visible |
|-----------|--------|---------------------|
| A profile field is out of its own valid range | Profile construction fails, naming the field and the range | Caller error |
| Two profile fields are mutually inconsistent | Profile construction fails, naming both fields | Caller error |
| A plan exceeds a render limit that is checkable at admission | `CompileError` naming the field, requested, available, and the authored object responsible | `ResourceReport` plus the error |
| A plan exceeds a node's declared intrinsic capacity | `CompileError` naming the node and the capacity; not raisable by any profile setting | `ResourceReport` plus the error |
| A plan's predicted cost exceeds the advisory budget | `CompileWarning` naming the predicted and permitted ratio; compilation continues | `ResourceReport` |
| A live bounded queue overflows at runtime, or an offered live note-on cannot acquire its producer release hold | The item is dropped before acceptance and counted against the offered queue; the report distinguishes slot from hold exhaustion (HOST-INV-009) | Structured diagnostics report |
| A new non-critical Phase 3 session/transport command cannot enter its fixed source store | The command remains caller-owned or is explicitly refused before timestamped acceptance; prior session state is unchanged and no drop counter moves | Fallible caller result naming the source store and requested and available capacity |
| More events are due in one quantum than `max_events_per_quantum` admits — **in Phase 1 and Phase 2** | **The call is rejected before renderer state or output is mutated**, with a release-active error naming that quantum and the requested and available counts. This is the *current* behaviour and it is not deferral: the candidate set is a prevalidated bounded span, so an over-full quantum is a caller-contract violation rather than a state the render loop absorbs. It is stated in full under [*Deferred to Phase 3*](#deferred-to-phase-3), where the prevalidated-span rule lives, and it is covered by a named test in the V2 render-contract suite | `RenderError`, release-active |
| A producer exceeds its share or scheduled-store declaration, or one external batch or the external-plus-internal total exceeds `max_events_per_quantum` — **once Phase 3 implements ingress** | HOST-INV-021's terminal invariant fault, even when unusable total slack remains: the complete current and every later callback in the epoch is silence, both carries are invalidated, atomic `needs_reprepare` is published, and no further quantum renders | Structured diagnostics report, attributed by producer share |
| The compiled callback window would exceed the derived compiled floor of `max_scheduled_events_in_flight`, or retained authored future events would exceed the headroom above it | Profile construction checks the compiled floor and plan admission checks the authored addition before playback. Reaching the condition at runtime means a producer broke its declaration and takes HOST-INV-021's terminal fault; an event is never delayed into lateness to recover capacity | `ResourceReport`, or the structured diagnostics report for a defect |
| A lossy field's capacity is reached | The oldest data is evicted by design, and the evicted count or continuation marker is exposed (HOST-INV-019) | The surface presenting that data |
| A session limit is reached | The activity stops with a counted diagnostic; everything already produced is kept, and nothing authored is dropped (HOST-INV-020) | The recording surface, plus the structured diagnostics report |
| An ingress event is beyond `forward_event_horizon` | Rejected and counted | Structured diagnostics report |
| A callback exceeds `maximum_block_size` | ADR-0021 part 3's terminal stream-contract fault: silence, both carries invalidated, `needs_reprepare` published, nothing allocated | Structured diagnostics report |

**Six behaviours, with the event boundary changing by phase.** Admission refuses a plan, a session/transport source
store refuses at its caller boundary, a live queue drops, a lossy budget evicts, a session limit stops, and a
sealed-batch invariant fault ends the stream epoch. Phase 1 and Phase 2 have no sealed producer boundary, so their
over-full caller span is rejected before mutation instead. Phase 3's publication arbiter makes that input a
construction invariant, which is why the same observed renderer overflow becomes a fault there rather than a load
policy. ADR-0038's engine-egress loss uses the drop behaviour under its separate licence.

`max_scheduled_events_in_flight` has no runtime smoothing behaviour. Its checked lower bound makes every admitted
compiled entitlement for one callback plus its admitted authored future store fit. Delaying release would make the
scheduler itself create lateness and would restore a capacity-dependent timing path ADR-0046 removes.

**Sizing fields bound nothing and cannot be exceeded**, so asking which behaviour they take is a category error:
`analyzer_fft_size` sets a resolution, `telemetry_ring_frames` sets a window length (its *eviction* is the lossy
behaviour, not an overflow), `retirement_crossfade` sets a duration, and `channel_layout`
describes what a stream *is* rather than how much of it there may be. They cost prepared memory and therefore appear in
the `ResourceReport`; they have no failure row because they have no failure.

**`accepted_sample_rates` is not one of them**, and an earlier revision of this paragraph listed it here. It bounds no
size, but it does refuse: a rate outside it fails profile construction, which is the first two rows of the table above.
`HOST-INV-007`'s narrowing says why it is not the third, and why `LIMIT-0004`'s job-admission error stays outstanding
with ADR-0028 rather than being discharged here. The closure pass added this paragraph:
HOST-INV-009 had claimed every profile field falls under one of four runtime behaviours, which was false twice over —
it omitted admission refusal, the behaviour most fields actually take, and it had no place for a field that is a size.

Every counter named above reaches the **structured diagnostics report**, which is the report a Phase exit review
inspects. This is the specific control against the failure mode ADR-0021 records twice: `LIMIT-0013`'s drop counters
existed for years and reached **no consumer at all** — `get_dropped_counts` has no caller, and the OSC feed publishes a
different ring's counter. Both the ledger and ADR-0021 recorded them as "OSC-only", which was the more flattering of
the two possibilities.

## Real-time and resource constraints

- The profile is read only off the audio thread (HOST-INV-002). The renderer holds no reference to it.
- Admission and preparation may allocate; both run off the audio thread.
- Every capacity the audio thread relies on is preallocated at preparation, including both carries, the sealed-batch
  store, event scratch, recording buffers, and each node's mutable state.
- Nothing in the render loop consults a limit to decide whether to allocate. A plan that was admitted fits by
  construction; a plan that does not fit was refused.
- The runtime-variable quantities are bounded source-queue occupancy, materialized authored batches, internal
  emissions, and a take's length. None allocates: live overflow drops and counts, a take stops and counts, authored
  batches fit their admitted envelope, and internal emissions fit their declarations.
- Publication allocates nothing and inspects only the snapshotted, declared capacities. It fills and seals
  preallocated external per-quantum storage for the imminent call before rendering begins; internal production uses
  its separate fixed arena.
- Phase 1 and Phase 2 reject an over-full caller span before mutation. Phase 3 treats a share, scheduled-store,
  external-batch or internal-arena overrun as a terminal contract fault. No phase allocates, defers or trims to absorb
  it.

## Conformance tests

No test exists yet: the V2 crate is Phase 1's, and this specification is written before it. Each row names what must
exist, in the phase that builds the thing it tests.

| Invariant | Named test or evidence | Phase |
|-----------|------------------------|-------|
| HOST-INV-001, HOST-INV-002 | A prepared plan renders after its source profile is dropped; the renderer holds no profile reference | 1 |
| HOST-INV-003 | Two tests, because no runtime check can see where a value came from. **Shape:** `HostCapabilities::from_device` has no defaulted parameter and no `Default` impl, so a caller cannot omit a capability. **Behaviour:** the cpal adapter is driven with a device reporting a non-default buffer range and the resulting profile carries that range — the direct regression test for `LIMIT-0057`, which discarded it | 9 |
| HOST-INV-004 | Partly a review check — no automated test can see that a default was *reasoned* from `Q`. The mechanical half is ADR-0032 clause 4's compile-time assertion `Q <= QuantumOffset::MAX`, which fails the build when `Q` changes and something was sized to its old value, plus a test that `HostProfile` exposes no field carrying a quantum | 1 |
| HOST-INV-005 | A test enumerating profile fields against their admitting rule. The capability half is asserted against the invariant's **closed enumerated capability set**, so adding a capability field fails the test rather than passing silently. Each `RenderLimits` field is enumerated against the three grounds: a ledger entry owned `HostProfile`; a field an accepted ADR creates; and the **enumerated residual set** the invariant lists, each of whose members carries a stated basis and revisit point. The test compares against that explicit list — not against "everything else", which would admit a protocol- or job-owned capacity by default. It asserts each field matches exactly one and fails on a field in none; Phase 3 extends that enumeration with ADR-0046's seven ground-2 fields. **It must not enumerate the no-antecedent list**, which is a different axis, and **must not treat a `Replaces` entry as a ground** — `max_held_notes` and `max_events_per_quantum` name `LIMIT-0031` and `LIMIT-0075` as provenance while being admitted by the residual | 1 |
| HOST-INV-006 | Every compile — succeeding and failing — returns a report whose every field has requested, available, and a dominant contributor | 1 |
| HOST-INV-007 | One refusal case per render limit **a plan can exceed with an error** — twenty-eight in Phase 1 and thirty-three once ADR-0046's Phase 3 fields exist — and the test asserts that the cases *are* that set rather than merely covering some of it. Each asserts the error names the field, both amounts, and the authored object, and that the plan is unchanged. The corresponding fourteen and sixteen fields without that refusal are enumerated in the invariant; `accepted_sample_rates` is covered by HOST-INV-016's rows and `predicted_quantum_cost_ratio` by the advisory-warning test | 1 |
| HOST-INV-008 | A node whose declared capacity is exceeded is refused, and raising every profile field does not admit it | 2 |
| HOST-INV-009 | Enumerate the *Live bounded queue* markers in the Events field table and the renderer-ingress source-store registry, assert those two named tables are the complete live-input marker domain and that ADR-0038 engine-egress queues remain outside it, then overrun each registered queue and assert its own drop count in the diagnostics report. For every queue that admits note-ons, separately leave a queue slot free while exhausting its producer hold entitlement, then assert the refused note-on is attributed to that offered queue and names the hold rather than the slot as the exhausted resource. A separate session-store case refuses before timestamped acceptance without incrementing a drop counter | 3 |
| HOST-INV-010 | A project saved under one profile loads and compiles under another; no serialized field names a profile value | 10D |
| HOST-INV-011 | A fixture whose event envelopes fit at both rates is admitted at 44.1 kHz and warned at 192 kHz by the cost budget alone; no count-valued **profile field** is automatically rescaled. A separate low-rate case shows that the same musical plan may request a larger destination-occupancy `EventCount` and be refused without changing any field's unit or meaning | 3 |
| HOST-INV-012 | Callback sizes from 1 frame to `maximum_block_size`, including non-multiples of `Q`, render identically to a single large block — the partition-invariance suite of ADR-0001. Plus one profile whose `maximum_block_size` is **below** `Q`, which must be admitted and must render identically | 3 |
| HOST-INV-013 | An ingress event one frame beyond the horizon is rejected and counted; a compiled event hours ahead remains in the plan and publishes normally; and a live event accepted into a queue snapshot is not checked again after waiting makes it late | 3 |
| HOST-INV-014 | The compiler's aggregate equals the sum of node-declared prepared bytes for a plan built from known nodes | 2 |
| HOST-INV-015 | A plan over the cost budget compiles and warns; no advisory field can produce a `CompileError` | 1 |
| HOST-INV-016 | A profile with `forward_event_horizon < maximum_block_size + Q` fails construction naming both fields — **and the default profile satisfies it at every admissible `maximum_block_size`**, including one above a second's worth of frames, which is the case the flat one-second default failed. Plus the `accepted_sample_rates` relation, in four cases: a `sample_rate` below the range and one above it each fail naming both fields, and each **inclusive endpoint** is accepted, since a fixed-rate host is a range one rate wide | 1 |
| HOST-INV-017 | The profile carries two fields and rejects a construction with `script_host_slots_per_voice < mod_matrix_slots_per_voice`, naming both; raising the host slots alone is accepted, which is what V1's `<=` assertion permits and the single-field model forbade. The assertion in `synth_modules` is gone | 7 |
| HOST-INV-018 | Every **quantity** field's type has a private field and a fallible constructor; no such field is a bare primitive, and `HeldNoteCount` does not convert to or from `VoiceCount`. The two **kind** fields, `channel_layout` and `source`, are asserted to be closed enums instead — the test enumerates both sets, so a new field must be classified rather than silently escaping the check | 1 |
| HOST-INV-019 | The telemetry ring is overrun and the reader can distinguish a complete window from an overwritten one | 5 |
| HOST-INV-020 | A take reaching each recording capacity stops, is counted, and keeps every event recorded before the stop; no note is dropped and no earlier note is overwritten | 9 |
| HOST-INV-021 | Profile construction rejects zero for each share and `release_hold_capacity`, a release share one event below hold capacity, a share sum above the total and each other plan-independent relation separately; release-share equality and larger values are accepted when the total sum fits, and any remaining total slack is asserted unusable, including a disabled producer's positive share. Plan admission rejects each internal, session, destination, retained-future and hold declaration without changing the fixed shares; two sources that fit individually but overflow a plan-wide authored aggregate are rejected unless the compiler proves them mutually exclusive. Compiled admission rejects the exact first over-full half-open `Q`-frame window under every anchor phase; loop activation rejects a tail/head collision and a loop shorter than `Q` whose repeated copies overfill one quantum, leaving prior transport state unchanged. Replacing the tempo map re-admits both compiled and runtime envelopes and leaves the old pair active on failure. Authored runtime expansion covers destination convergence, future retention and simultaneous holds, then materializes once; a mutation above any share or declaration takes the terminal fault rather than consuming slack or dropping a suffix. A full late live snapshot fits and clamps every event, while the next external event beyond source capacity drops and counts. A complete session snapshot and the largest legal locate catch-up publish without delay; the command beyond reserved source storage is refused before timestamped acceptance. Live and authored hold entitlements are isolated; a note-on plus hold publishes atomically and its later release survives a saturated ordinary live queue, while the refused-note-on mirror produces only a counted orphan release. Converging live, session and authored mass-release causes remain in their own admitted shares and redeem affected holds without a second release event. An indivisible multi-event batch publishes all or none. An admitted internal producer reaches its separate arena maximum without mutating sealed external input; the first event above it takes the fault. A forged over-full external batch and an internal over-emit both silence the complete current and every later callback, invalidate both carries, publish `needs_reprepare`, render no later quantum, and attribute the fault. Per-share and total high-water marks are asserted below and at the limit | 3 |
| `max_scheduled_events_in_flight` (HOST-INV-021's sizing relation, no invariant of its own) | The derived default equals `compiled_event_share * max_quanta_per_callback + 4 096` events, checked with compiled products below, equal to and above 4,096 events; overflow fails construction. Plan admission charges the plan-wide aggregate of retained authored future events above the compiled floor. At the exact admitted bound every event publishes on time; a mutation one event above its declaration takes the producer fault rather than delaying an event | 3 |
| HOST-INV-020, and the retirement budget | A plan swap with `max_active_voices` sounding retires every voice with a crossfade and refuses none, so `max_concurrent_retiring_voices` cannot bind at its derived default | 9 |

## Unresolved questions

| Question | Blocking? | ADR or task |
|----------|-----------|-------------|
| Whether a live host supports layouts beyond the Sound Core specification's currently admitted counts, and whether the profile carries a layout set or one layout. The pass-5 audit found that a multichannel device constructs `Multi(n)`, while V1's internal buffers remain mono/stereo and its output adapter now explicitly silences surplus channels | **Yes for Phase 9**, which queries a real device; no for the current offline renderer. Carrying `Multi(n)` does not itself claim support | Sound Core render contract, Phase 9 |
| What an observation tap is and who owns the analyzer surface; the three capacities here may become one registration budget | No — the capacities stand whatever the taps mean | ADR-0027, Phase 5 |
| The retirement crossfade's value, and whether ADR-0009 wants a concurrent-retirement budget below `max_active_voices` — which it may only take together with a defined behaviour for reaching it | No — V1's 128 frames compiles today, and the derived budget cannot bind | ADR-0009, Phase 9 |
| Recording take and commit semantics, which may change what a "recorded event" is | No | ADR-0024, Phase 9 |
| What a send is, which may change whether `max_sends_per_channel` is per channel or per bus | No | ADR-0034, Phase 8 |
| The script-work aggregate's threshold, which needs a measured per-instruction cost before it can become a `RenderLimits` field rather than a reported quantity | No — the `ResourceReport` carries the quantity meanwhile | Phase 7 |
| Whether same-sample ingress order distinguishes `Hardware` from `Arrival`, and how their measured uncertainties participate in that order | No — HOST-INV-021 partitions capacity without assigning semantic precedence | ADR-0022, ADR-0023, Phase 3 |
| ~~**ADR-0021's `LIMIT-0013` evidence.**~~ **Resolved in Phase 0A, not Phase 3.** Its drivers and disposition describe per-priority drop counters "published on OSC"; they are published nowhere, and the OSC counter it names belongs to another ring. [ADR-0038](../decisions/ADR-0038-engine-egress-queue-classification.md) supersedes both the driver and the disposition on that evidence | Resolved by accepted ADR-0038 | ADR-0038 |
| ~~**The class ADR-0021 gave `LIMIT-0013`'s rings.**~~ **Resolved in Phase 0A, not Phase 3.** They are engine egress, not "fed by external, unbounded-in-time input", so `Live bounded queue` never described them. [ADR-0038](../decisions/ADR-0038-engine-egress-queue-classification.md) part 1 supplies the missing rule and part 3 removes the entry as an explicit compatibility break: there is no workspace production caller, while public external use remains unknown | Resolved by accepted ADR-0038 | ADR-0038 |
| **What V2's live renderer-ingress streams are, and what bounds them.** V1 has no timestamped ingress queue to carry over — `LIMIT-0013` is engine egress and `LIMIT-0012` carries commands. Phase 3 must name each live source store and capacity, add it exactly once as *Live bounded queue* to the [closed renderer-ingress source-store registry](#renderer-ingress-source-store-registry), then include the complete eligible snapshot in HOST-INV-021's live-share lower bound; session/transport storage is separately covered by that invariant's session relation | No for Phase 1, which accepts an open caller span. **Yes for Phase 3**, which cannot construct the publication partition without the streams | Phase 3 |
| Where `LIMIT-0004`'s **job-admission error** is delivered. The ledger requires an out-of-range job to be refused with an error naming the requested rate and this field's range. Profile construction refuses an out-of-range *stream*, which is a floor rather than that error: a job asks for a rate before a profile exists | No for Phase 1, which has no job layer. **Yes for Phase 4**, which cannot close with the disposition undelivered | ADR-0028, Phase 4 |
| Whether `max_nodes` should be anchored independently rather than computed from `max_active_voices`, which is itself only measurement-anchored | No | Phase 2 exit |
| Whether `max_mix_channels` and `max_observation_taps` should be coupled so that every mix channel is guaranteed a tap | No — the report names which budget bound the plan | ADR-0027, Phase 8 |
| Where a profile is stored and who may edit it — application settings, host configuration, or neither | No — Phase 1 constructs it in code | ADR-0013, ADR-0029, Phase 10A |
| Whether the forward horizon survives calibration evidence, and whether a mis-calibrated anchor should widen it or fail preparation | No — the current value rejects and counts, which is the diagnostic | ADR-0022, Phase 3 entry |
| Whether `max_fan_out_per_port` should exist at all, or whether the edge budget alone suffices | No | Phase 2 exit |
| Whether `retirement_crossfade` and `telemetry_ring_frames` should be stated in seconds rather than frames. Both mean durations and both are flat frame counts, so both shrink 4.4x from 44.1 to 192 kHz; HOST-INV-011 does not reach them because neither bounds a plan, but the crossfade's shortening is audible where the ring's is cosmetic | No — V1's values are what V1 ships, and neither can misjudge a plan | ADR-0009, ADR-0027, Phases 9 and 5 |

## Corrections

The reviews of the workflow-reset change and their severity tables are retained in
[REV-P00A](../reviews/phase-00a-exit-review.md); the earlier eleven-pass chronology this section replaced lives in this
file's Git history, not in the review. These disproved premises remain here because repeating one would change
the contract:

| Disproved premise | Current contract |
|-------------------|------------------|
| `LIMIT-0013`'s four rings were renderer ingress | They are engine egress with no workspace production caller; public external use remains unknown. V1 has no timestamped renderer-ingress queue |
| `EVENT_BUFFER_SIZE` was a per-quantum event cap | It sizes egress rings; the V1 sequencer buffer is an uncapped `Vec` |
| Ingress capacity bounded deferred backlog | False: deferral freed ingress slots, so ingress capacity did not bound the separate deferred backlog; that would have required its own store and exhaustion rule. ADR-0046 removes deferral, and the complete eligible ingress snapshot instead contributes a hard lower bound to its own disjoint producer share |
| This specification could refine ADR-0001 lateness | False: a `Current` specification may not narrow an accepted decision by gloss; an accepted successor decision is required. ADR-0043 supplied that clarification on 2026-08-20 |
| All egress payloads could share one lossy policy | Custodial payloads require a separate no-loss path under ADR-0038 |
| A per-instrument held-note count was a plan-wide capacity | `max_held_notes` is plan-wide; node-local holds remain node contracts |
| `HOST-INV-005`'s three grounds covered every profile field | They govern the `RenderLimits` half. `HostCapabilities` has four members: **three** queried capabilities admitted by ADR-0021 part 4 and ADR-0032 clause 12, with their ledger entries as provenance, plus `source`, which nothing queries and `HOST-INV-003` admits |
| The accepted rate range was a capability, or could be demoted to a constructor constant | It is a `RenderLimits` field. `LIMIT-0004` is owned `HostProfile` and refused *at admission*, which ground 1 reserves to limits; a capability is what a plan is prepared against, a limit is what admission refuses on |
| Profile construction discharged `LIMIT-0004`'s job-admission error | It does not. A job requests a rate before a profile exists, so the job-admission error is ADR-0028's and stays outstanding; construction is a floor beneath it |
| `HostCapabilities` held four queried capabilities | It has four members and **three** queried capabilities — rate, block size, layout — plus `source`, which the constructor derives and `HOST-INV-003` admits. ADR-0021 part 4 could not admit a field nothing queries |
| V1's `SampleRate` could be reused, or bypassed with raw `f32` Hz | V2 defines its own checked `SampleRate`, on ADR-0021 part 3's `VoiceCount` ground: V1's clamps `NaN`, zero, and negative to `1.0`, and an untyped `f32` argument trades that for a broken newtype rule |
| Every default informed by EVD-0003 was measurement-derived | One value is derived; two are chosen and anchored; the remainder are queried, carried over, or chosen |

The final independent pass required no contract-clause change. Editorial improvements did not keep the specification
open, and ADR-0038 acceptance satisfied its remaining lifecycle condition.
