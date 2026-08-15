# SPEC: Host Profile and Render Limits

| Field            | Value                                  |
|------------------|----------------------------------------|
| Status           | Draft                                  |
| Phase            | 00A                                    |
| Created          | 2026-08-13                             |
| Last reviewed    | 2026-08-15                             |
| Based on         | ADR-0021, ADR-0001, ADR-0032, ADR-0037, ADR-0038 |
| Invariant prefix | HOST                                   |
| Supersedes       | —                                      |
| Superseded by    | —                                      |

Allowed status values are defined in [README.md](README.md).
Only a `Current` specification constrains implementation.

**Why this is `Draft`, what makes it `Current`, and what has been narrowed out of it.** Every clause below follows from
an accepted decision **with two exceptions.** The first is `event_egress_capacity`, whose admissibility rests on
[ADR-0038](../decisions/ADR-0038-engine-egress-queue-classification.md) — a record that is `Proposed`, so ADR-0021's
original clauses still bind until it is accepted and this specification cannot become `Current` before then. The
second is **HOST-INV-021, which proposes two exceptions to ADR-0001 rather than applying them**: that
a quantum may defer at all, which departs from clauses 12 and 14; and an interim rule for when clause 16's late
condition is evaluated. The invariant records both as narrowings of an accepted decision that this specification is not
entitled to make.

**HOST-INV-021 is therefore `Deferred` to Phase 3 rather than normative here**, together with the ingress capacities and
the deferred store it operates over. The identifier is **retained and not reused** — identifiers never change, and a
withdrawn invariant that came back under a new number would lose the four external passes that landed on this one. What
that leaves behind is stated positively in [*Deferred to Phase 3*](#deferred-to-phase-3) rather than by deletion: an
implementation reading this specification at `Current` must find no clause that silently assumes the deferral mechanism.
The scoping decision is [REV-P00A](../reviews/phase-00a-exit-review.md)'s, and its ground is that
Phase 1 has no live ingress and no host callback, so the capacity a deferral operates against cannot be specified from
anything Phase 0A can observe.

The field set is complete against the master plan's list. [*Review status*](#review-status) keeps the record of what
each pass found and what correcting it changed; that record is the argument for the remaining step, not decoration.

**Eleven passes have now run, and the last five are the first conducted by someone who did not author part of this
document.** It found five, three of them P1, and the most consequential had stood since the first independent pass:
the event-queue capacities this specification reasoned from are the engine's **egress** rings, not renderer ingress,
so V1 has no timestamped ingress queue to carry over and HOST-INV-021's deferral has no capacity to operate against
yet. All five are corrected.

The two passes after it found five and six more — including that the immutable stamp and ADR-0001 clause 16 **cannot
both be implemented as written**, that the deferred store has no exhaustion policy, and that `LIMIT-0013`'s rings hold
a class ADR-0021 reserves for something else. Three of those are open questions requiring an accepted-decision
successor rather than anything this specification may settle. The criterion for stopping is not "a pass finds nothing",
which on eleven passes' evidence will never happen, but "a pass that changes no contract clause"; no external pass has
met it. **The sixth pass's recommendation to close P00A-T005 on the whole contract stays withdrawn** —
[*Review status*](#review-status) records why both of its grounds failed.

**What has since changed is the task's scope, not the findings.** The four blockers those passes produced are disposed
in two directions rather than cleared:

- **Two are settled in Phase 0A.** [ADR-0038](../decisions/ADR-0038-engine-egress-queue-classification.md) supplies the
  engine-egress rule ADR-0021 lacked — the class question its rings were given — and splits `LIMIT-0014`, so
  `event_egress_capacity` has a settled ledger antecedent and HOST-INV-005 is satisfiable for it. Both were gate-bound
  through the exit gate's resource-inventory clause and could not have moved.
- **Two are Phase 3's**, recorded in the master plan's Phase 3 work list and entry gate rather than only here: the
  ADR-0001 clarification, and V2's ingress streams plus a separate bound and exhaustion policy for the deferred store.
  They are not one item — deferring an event frees its upstream slot, so an ingress capacity bounds the arrival rate
  and not the backlog.

This specification closes P00A-T005 on the master plan's field list, which is what that item asks for, and carries
HOST-INV-021's mechanism forward as a stated deferral rather than as an unstated gap.

Fields owned by decisions that are still `Proposed` — ADR-0002, ADR-0009, ADR-0024, ADR-0027, and ADR-0034 — are marked
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
  is sized by its provisional value, which ADR-0037's acceptance forbids.
- **Class semantics and configuration ownership.** [ADR-0021](../decisions/ADR-0021-host-profile-and-admission-policy.md)
  decides both axes. This specification sets numbers for the one owner ADR-0021 assigned to P00A-T005 and touches none
  of the other six.
- **Limits owned elsewhere.** 48 of the ledger's 76 entries — none undecided, since ADR-0038 split `LIMIT-0014` — belong to a node contract, a domain/format contract, job
  policy, application settings, a protocol contract, or are removed. They are not profile fields and do not appear here
  except where a node declares a capacity *into* admission.
- **Hardware clock calibration, latency compensation, and drift.**
  [ADR-0022](../decisions/ADR-0022-hardware-time-mapping.md), deferred to the Phase 3 entry gate. This specification
  sizes the forward event horizon; it does not decide how a host timestamp maps into the epoch it is measured against.
- **Job bounds.** Render tail, output size, pre-roll, and quality presets belong to
  [ADR-0028](../decisions/ADR-0028-long-running-job-contract.md), deferred to the Phase 4 entry gate.
- **What an observation tap is for** (ADR-0027), **what a channel layout is** (ADR-0002), **what a send is** (ADR-0034),
  and **what a recording take is** (ADR-0024). This specification carries their capacities, not their meanings.

## Terminology

Defined in [../glossary.md](../glossary.md): *host profile*, *resource report*, *render quantum*, *render plan*, *sample
time*, *stream epoch*. Terms this specification adds:

**Host capabilities**

The subset of the profile that describes what the host and device can actually do. Established by querying them, never
by a compiled-in constant. The application does not raise a capability; it discovers one.

**Render limits**

The subset of the profile that describes budgets the operator chooses: how large a plan may be before the renderer
refuses it. A render limit may be raised, at the cost of memory and CPU that the resource report accounts for.

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
| [ADR-0001](../decisions/ADR-0001-internal-render-quantum.md) | That the quantum is a compile-time constant and **not** a profile field (clause 1); that both carries are sized `maximum_block_size + Q` and preallocated at preparation (clause 5); that the output carry is primed with `Q` frames of silence so that any `N` can be served, including `N < Q` (clause 6) — which is why HOST-INV-012 has no lower bound on `maximum_block_size`; that added latency is a constant `Q` frames and a named contributor in the latency accounting (clause 7); that a late event is clamped forward and counted (clause 16) |
| [ADR-0032](../decisions/ADR-0032-sample-time-and-event-timestamps.md) | The time types the profile's frame-denominated fields are expressed in — `FrameCount` for a horizon, a latency contribution, and a quantum (clause 2); that the forward event horizon is a single profile field binding ingress provenance only (clause 21); that the backward direction has no budget; that a profile's sample rate, layout, and capacity are fixed for the life of a stream epoch (clause 12) |
| [ADR-0037](../decisions/ADR-0037-render-quantum-value.md) | That `Q` is provisional, so no field here may be sized by its value |

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
3. **HOST-INV-003** — **On the device path**, every field of `HostCapabilities` is established from queried host and
   device capability. A hardcoded advertised range is forbidden, including on a branch that successfully queried the
   device — this is the `LIMIT-0057` anti-pattern ADR-0021 part 4 names. The rule is enforced by construction rather
   than by inspection: `HostCapabilities::from_device` takes every capability as an argument and has no default for
   any of them, so a value that was not queried cannot reach it without a caller writing the constant at the call
   site, where it is visible. `::declared` is the separate constructor for the offline and harness paths, and it sets
   `CapabilitySource` itself so the tag cannot disagree with the constructor that produced it.
4. **HOST-INV-004** — The render quantum is not a profile field, is not derived from any profile field, and is not
   configurable. No field below may be defined in terms of `Q`'s numeric value; a field may be defined in terms of the
   symbol `Q`.
5. **HOST-INV-005** — The profile carries exactly the fields listed in [*The field set*](#the-field-set). A field is
   admissible on one of three grounds, **applied in this order so that every field takes exactly one**:

   1. **a ledger entry owned `HostProfile`.** ADR-0021 part 1 is explicit that only these participate in admission and
      that `N/A — removed` holds nothing, so an entry with that owner cannot be a ground however its disposition reads.
      An earlier revision widened this ground to "any disposition that creates a profile capacity" in order to catch
      `LIMIT-0031` and `LIMIT-0075`; that contradicted the accepted decision, and those two are ground 3 instead;
   2. **an accepted ADR that creates it** — `forward_event_horizon` is ADR-0032 clause 21's;
   3. **the residual: an enumerated list this specification creates.** Ground 3 is a *closed set*, not "everything
      else" — that would make the ownership restriction unenforceable, since a protocol- or job-owned capacity would
      pass by default. The list is: the seven no-antecedent fields **minus `forward_event_horizon`, which ground 2 already selects** — so six — plus `max_held_notes` and `max_events_per_quantum`,
      whose ledger entries (`LIMIT-0031`, `LIMIT-0075`) appear in the `Replaces` column as **provenance** — a different
      question from what admits the field. Adding to this list is a change to this specification, reviewable as such.

      **`event_egress_capacity` is not on this list because it does not need to be — it now satisfies ground 1.**
      It had no ground while `LIMIT-0014` carried an undecided owner, one constant sizing a GUI ring and an OSC ring
      where ADR-0021 allows one owner per entry. [ADR-0038](../decisions/ADR-0038-engine-egress-queue-classification.md)
      part 4 splits them: `LIMIT-0014` keeps the engine-to-GUI ring and is owned `HostProfile`, which is ground 1, and
      the OSC ring becomes `LIMIT-0076` owned by the protocol contract that serializes it — **so it is not a profile
      field at all**, and the profile carries one capacity here rather than two.

   Grounds 1 and 2 are disjoint by construction, and 3 excludes both, so each field matches exactly one. **"No V1 antecedent" is a different axis and
   does not select a ground**: it answers whether V1 had the thing, not what admits the field, and
   `forward_event_horizon` is on both lists precisely because those are different questions. An earlier revision
   defined ground 3 *as* the no-antecedent list, which left `forward_event_horizon` matching two grounds and
   `max_held_notes` matching none. A capacity that belongs to another owner may not be smuggled in on any of the three.
6. **HOST-INV-006** — Compilation returns a `ResourceReport` naming, for every field, the requested amount, the
   available amount, and the dominant contributor to the request. The report is produced whether admission succeeded or
   failed; a refusal is a report plus an error, never an error alone.
7. **HOST-INV-007** — A plan exceeding a render limit is refused with a `CompileError` naming the field, the requested
   amount, the available amount, and the authored object responsible. Admission never truncates, clamps, or drops to
   make a plan fit, per ADR-0021 part 2.
8. **HOST-INV-008** — A node's intrinsic capacity is declared by the node, reported in the `ResourceReport`, and
   contributes to admission. It is not a profile field, and no operator setting may raise it.
9. **HOST-INV-009** — **Dropping** at runtime is permitted only for the queues explicitly marked *live bounded queue* in
   the field tables. Every such queue counts its drops, and the count reaches the structured diagnostics report. This
   invariant governs dropping, not every runtime behaviour: explicit eviction is HOST-INV-019's, a session limit that
   halts an activity is HOST-INV-020's, and a quantum that cannot admit every due event is HOST-INV-021's — **which
   is `Deferred to Phase 3` and does not bind here**, so four of the five behaviours are in force and an over-full
   quantum is a caller contract violation rather than a runtime state. Together
   with HOST-INV-007's admission refusal these are the five behaviours, and
   [*Failure and diagnostics*](#failure-and-diagnostics) assigns each field exactly one — or records it as a **sizing
   field**, which bounds nothing and therefore cannot be exceeded.
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
    an event after it is admitted re-triggers the check — deferral above all. The clause is here because the horizon's
    penalty is *rejection*, and re-evaluating a repeatedly deferred event against a horizon it drifts toward would turn
    HOST-INV-021's "deferred, not dropped" into a drop after enough overload, which is precisely the guarantee that
    invariant exists to make. Keeping the envelope's `time` immutable makes the point moot for the stamp itself; stating
    it makes the point moot for an implementation that checks the render position instead. ADR-0032 clause 21 already
    scopes the horizon to bounding "what an *external* producer can enqueue" — this says the same thing where an
    implementer would look for it.
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
21. **HOST-INV-021 — `Deferred to Phase 3`. This invariant is not normative when this specification becomes
    `Current`, and no Phase 1 or Phase 2 implementation may rely on it.** It is the one clause here that proposes
    exceptions to an accepted decision rather than applying one, and the capacity it operates against does not exist in
    V1 to be carried over. The identifier is retained rather than freed: it is what four external review passes are
    indexed to, and reissuing the mechanism under a new number in Phase 3 would strand that record. The text below
    stands unchanged because it is the specification *of the deferred work* — the constraints Phase 3 inherits, and the
    reasons three earlier revisions of it were wrong — not because any part of it binds Phase 1. See
    [*Deferred to Phase 3*](#deferred-to-phase-3).

    Where more events are due in one quantum than `max_events_per_quantum` admits, the excess is
    **deferred, not dropped — conditionally.** The guarantee holds while the deferred store has room, and no bound for
    that store is derivable from the current field set (see below), so **this invariant is not implementable until
    Phase 3 defines the ingress capacities and the store's size or its exhaustion behaviour.** The clause is written in
    full because the rest of it constrains that work; it is not yet a promise the renderer can keep. Deferral advances an event's **render position** by exactly `Q` frames, so it is
    rendered in the following quantum at the same offset, where it is re-evaluated and may defer again. It is counted
    under its own **capacity-deferral counter**, and it **must not fire ADR-0001 clause 16's late counter — because
    clause 16's condition does not hold.** That clause triggers on "an event whose timestamp falls in an
    already-rendered quantum", and the quantum that could not admit this event has not rendered. The argument is about
    the condition, deliberately: an earlier revision argued instead that the *producer* was not at fault, which reads
    a cause into a clause that states none and would have let this specification narrow an accepted decision by
    glossing it. The cause still matters — it is why a separate counter exists — but it is not what excludes clause 16
    here. This is ADR-0032 clause 22's rule applied to a second case: that record separates the pre-epoch clamp from
    the late counter on the same footing, and states that a single test would pass on the wrong policy.

    **The envelope's `time` is immutable, and deferral does not rewrite it.** ADR-0032 clause 17 stamps every event
    with `(epoch, time, provenance)`; deferral changes which quantum renders the event, not what the event says about
    when it was stamped. The distinction is not bookkeeping pedantry — it is observable in three places. **Diagnostics:**
    the counter says how often the engine was full, and only a preserved stamp lets the report also say *how far* an
    event was displaced, which under sustained overrun is the quantity that matters and is otherwise invisible.
    **Recording:** ADR-0032 clause 7 resolves a take's placement to musical time before saving, so a rewritten stamp
    would silently quantize a played performance forward by however many quanta the engine was overloaded — an
    authored-data change that ADR-0021 part 2 forbids outright. **The ingress-equivalence gate:** ADR-0032 clause 23's
    stated harm is that moving an event off its declared sample "would make the sample position a lie", and that harm
    does not care whether the cause was precedence or capacity, even though the prohibition is written about the
    former. So the render position is derived from the **post-clamp** position rather than from the stamp:
    `render_position = clamped_position + Q x deferrals`, where `clamped_position` is `time` for an on-time event and
    ADR-0001 clause 16's first not-yet-rendered boundary for a late one. Deriving it from `time` directly would ignore
    the clamp displacement, so an event that was both late *and* deferred could land back in a quantum that has already
    rendered — the case the both-late-and-deferred conformance test covers. The deferral count travels with the event
    rather than replacing its stamp.

    **This does not settle the clause-12 question; it widens it.** ADR-0001 clause 12 says an event "is assigned to the
    quantum containing its timestamp", and clause 14 requires sample-positioned effects — note-on, note-off, gate,
    retrigger — to occur "at their declared sample within the quantum". Deferral departs from both. Clause 16 is the
    only exception ADR-0001 grants, and it is granted **for an already-rendered quantum**, which this invariant
    explicitly says is not the case here. So the precedent does not cover deferral: an earlier revision reasoned that
    clause 16 shows the first sentence "describes the ordinary case rather than an absolute", and that inference is not
    ADR-0001's to give. What deferral does honour is clause 12's binding second sentence, "no event is ever applied to
    samples already produced".

    **The ADR-0001 blocker is therefore broader than the late-counter question.** It is not only *when* clause 16's
    condition is evaluated; it is whether a quantum may defer at all under clauses 12 and 14, and if so, what the
    exception's shape is. Both belong in one ADR-0001 clarification or successor, and until it lands HOST-INV-021 is
    proposing an exception to an accepted decision rather than applying one. **Whether clause 16's clamp likewise preserves or rewrites the stamp is ADR-0001's question, not this
    specification's**, and it is recorded as unresolved rather than answered here by implication.

    Clause 16 is not the mechanism here, only the precedent that an event may be moved forward rather than dropped. Its
    own rule — "clamped to the first not-yet-rendered quantum boundary" — is circular for this case, because the
    quantum that cannot admit the event has not been rendered yet either, so applying it literally would place the
    event back where it did not fit.

    **`+Q` rather than the boundary, because the boundary loses the sample position.** An earlier revision moved a
    deferred event to the *boundary* of the following quantum. That collapses every deferred event in one quantum onto
    offset 0, which does two things this contract should not do silently: it moves a **sample-positioned** effect —
    note-on, note-off, gate, retrigger — off its declared sample, which ADR-0001 clause 14 preserves and which its
    consequences state as "note and gate timing is unaffected"; and it manufactures same-`SampleTime` collisions that
    become ADR-0023's problem, invented by a capacity shortfall rather than authored. Adding `Q` preserves the offset,
    preserves the spacing between two deferred events, and is exactly the "up to `Q` frames of added delay" this
    specification already claims. It also composes: a second deferral is another `+Q`.

    **What defers is the tail of a defined order, not whatever the implementation reaches last.** Events due in a
    quantum are admitted in this order, and the events that do not fit are the ones deferred:

    1. compiled events before ingress events, on the basis given below;
    2. within each of those two groups, by ascending render position;
    3. ties by ADR-0023's same-sample order once it is accepted, and until then by plan order for compiled events and,
       for ingress ones, by **arrival order into the renderer** — a stream-independent, monotone sequence the envelope
       of ADR-0032 clause 17 can carry. An earlier revision broke the tie by queue priority and then by position within
       a ring; there is no such ring, since the only prioritized rings this document has identified are engine egress,
       and V2's ingress streams are undefined. A tie-break that names structures which do not exist cannot be
       implemented or tested. Priority-based tie-breaking returns as an option when the ingress contract defines
       classes, and is listed with the priority question below.

    Rule 2 is what covers **compiled events overrunning a quantum on their own**, which `max_note_expansion_per_tick`
    makes reachable whenever a script-driven note graph's expansion is not statically knowable. Without it, admitting
    in arrival order would defer an earlier-positioned event past a later-positioned one.

    **Rule 1's basis is provenance exactness, not ADR-0032 clause 23.** An earlier revision justified subordinating
    queue priority by saying that deferring an earlier-stamped event past a later-stamped one to honour priority is
    clause 23's forbidden perturbation. That argument proves too much: **rule 1 does exactly the same thing**, every
    time a compiled event positioned late in a quantum is admitted while an ingress event positioned early in it
    defers, which is the ordinary case rather than a corner. Either the argument kills both cross-position reorderings
    or it kills neither, and it cannot be used for one and against the other.

    The basis that does separate them is what the timestamps are worth. A compiled event's position is exact by
    construction (ADR-0032 clause 18) because it came from the plan and the tempo map; an `Arrival`-stamped ingress
    event carries error the adapter itself declares unmeasured (clause 19). Displacing the exact one desynchronizes
    authored music against itself; displacing the inexact one adds to an error already present and already reported.
    **Queue priority stays subordinate on the same basis**, and now for a reason that survives it: priority is a
    *delivery* class, not a claim about timestamp accuracy — two events stamped by one adapter carry identical
    uncertainty whichever ring they were routed to, so priority says nothing about which position deserves to be kept.

    That basis raises a question this specification does not answer: it would order `Hardware`-stamped ingress ahead of
    `Arrival`-stamped ingress by the same reasoning, and there is no such tier. Whether there should be depends on what
    the two uncertainties actually are, which is ADR-0022's evidence and Phase 3's work.

    **What the order costs to evaluate is a real-time constraint, and this specification states the constraint
    without prescribing the mechanism.** Admission runs on the audio thread, so it must allocate nothing and do work
    bounded by capacities the profile declares, over preallocated storage. That is the whole requirement. It does
    **not** forbid sorting: once Phase 3 fixes the ingress and deferred-store capacities the candidate set is bounded,
    and an in-place sort over preallocated storage satisfies the rule as well as a merge does. A previous revision
    banned comparison sorting outright, which prescribed a mechanism while claiming not to.

    An earlier revision went further and declared the answer — a five-way merge of the four prioritized rings plus the
    compiled stream, guarded by a position-monotone enqueue contract. **That was withdrawn, for two independent
    reasons.** The rings it named are `LIMIT-0013`'s, which carry `EngineEvent` *out* of the engine toward the GUI and
    are not renderer ingress at all; V2's ingress streams are not yet defined or budgeted, so nothing here can count
    them. And monotone enqueue would not have been sufficient even if they were: deferral itself destroys the ordering
    a head-merge needs. If a ring holds `A` at the end of quantum `k` followed by `B` at the start of `k+1`, deferring
    `A` puts its render position after `B`'s while `B` is still behind `A` in the FIFO, and a head-merge cannot reach
    `B`. Deferred events therefore need their own preallocated, bounded stream, merged as a further input — which is a
    capacity this profile does not yet have a field for.

    **A bounded deferred store needs an exhaustion policy, and no bound is derivable from the current field set.** The
    store is preallocated, therefore finite; sustained overrun, or the starvation channel below, keeps events in it. A
    previous revision claimed it could be sized to the sum of the ingress capacity and `max_scheduled_events_in_flight`,
    on the reasoning that every deferred event already occupies a slot upstream. **That is wrong twice.** Moving an
    event into the store *frees* its upstream slot, so the two capacities bound arrivals per unit time rather than
    outstanding backlog. And `max_note_expansion_per_tick` turns one released event into many, so the release window
    does not bound the event count it produces either.

    So there is no arithmetic that makes exhaustion unreachable, and the question is a real one: **either the ingress
    contract bounds outstanding expanded events, or deferral is not unconditional and store exhaustion needs a defined
    behaviour** — which would be a sixth runtime behaviour, or a drop, and a drop is what HOST-INV-021 exists to
    prevent. This is the invariant's weakest point and it is stated rather than papered over: as written, "no event is
    lost" holds only while the store has room, and nothing here establishes that it does.

    **Defining the V2 ingress streams, their capacities, and the deferred store's derived size is therefore Phase 3's**,
    and it is a blocker rather than a refinement: until it lands, HOST-INV-021's "no event is lost" rests on a store
    with no size. **Nothing in this invariant binds meanwhile** — an earlier revision said the admission order did,
    which contradicts the invariant's own deferral header two paragraphs up and is withdrawn. What survives being
    deferred is one negative constraint, stated in [*Deferred to Phase 3*](#deferred-to-phase-3): no phase may
    allocate to absorb an over-full quantum. The admission order is a property of the deferred mechanism and goes
    with it. It is stated here as Phase 3's inheritance, independent of how
    the streams are arranged.

    **Preserving the offset costs a second starvation channel, and the order has no age term.** A deferred event keeps
    its offset, so an event positioned late in its quantum arrives late in the next one too, and under sustained
    overrun it loses rule 2 to natively-due events positioned early in theirs — every round, indefinitely. Waiting
    confers nothing. That is a starvation surface **among ingress events themselves**, independent of the
    compiled-precedence one already recorded, and the two have different fixes: a reserved ingress allowance addresses
    the first, and only an age term in the admission order addresses the second. Both are Phase 3's, and both are
    listed as unresolved. **What holds meanwhile is weaker than "nothing is lost"**, and this paragraph said that
    unconditionally until review caught it: nothing is lost *while the deferred store has room*, and no bound for that
    store exists yet. What is unqualified is the visibility — the capacity-deferral counter makes the condition
    observable, and because the stamp is immutable the report can carry displacement per event, which is the quantity
    that would actually show starvation.

    Dropping remains possible only at the live bounded queue under HOST-INV-009, which is the one place a drop is
    counted and the one place an external producer can outrun the engine.

    **This does not breach ADR-0032 clause 23**, which forbids *ADR-0023* from perturbing a timestamp to encode
    precedence. Deferral moves an event because a bounded resource is full and never reorders two events to express priority.
    **It is not authorised by ADR-0001 clause 16**, as an earlier revision of this paragraph claimed: clause 16's
    exception is for an already-rendered quantum, which is explicitly not the case here, and whether a quantum may
    defer at all is one of the two things the ADR-0001 blocker above asks for. A reader arriving from clause 23
    should find that stated rather than have to derive it. **The scope argument is not what does the work, though.**
    Clause 23's reason — that moving an event off its declared sample "would make the sample position a lie" — applies
    whatever the cause, and a review that leaned on the wording rather than the reason would have blessed a rewritten
    stamp. What actually keeps deferral honest is the immutable envelope above: the declared sample survives, and only
    the render position moves.

    **The two counters answer different questions, and one event may raise both.** Clause 16's counter is a
    **condition** counter — how often an event's timestamp fell in a quantum that had already rendered — and it fires
    whatever the cause, including when the cause is this engine's own release window. The capacity-deferral counter is
    a **cause** counter: how often a profile capacity was the thing that moved an event. An event that arrives late is
    clamped forward and raises the first; if the quantum it lands in is itself full, it is deferred and raises the
    second as well. **A delayed release does not automatically raise both**: ADR-0032 clause 27 releases events as
    their quanta approach, so a full window can delay an event that is still on time, and the late counter rises only
    when the delay pushes a timestamp into a quantum that has rendered. All of that is correct — two distinct facts about one event
    — but it means the counters may not be added to obtain a number of affected events, and a diagnostics consumer
    that sums them overcounts.

## Deferred to Phase 3

One invariant and two capacities are deliberately not normative in this specification. This section states what that
leaves for Phase 1 and Phase 2, because a deferral written only as an absence is indistinguishable from an oversight —
and because a reader who finds HOST-INV-021's text in the invariant list needs to be told, at the point of use, that it
does not bind.

| Deferred | Where it goes | What holds meanwhile |
|----------|---------------|----------------------|
| **HOST-INV-021** — per-quantum deferral of an over-full event set | Phase 3 work list and entry gate | Nothing in Phase 1 or 2 may implement deferral, and nothing may assume an over-full quantum has a defined runtime behaviour. It does not, and this specification no longer claims one |
| **V2's renderer-ingress streams and their capacities** | Phase 3 work list | No profile field describes them, and none may be invented at a call site. V1 has none to carry over |
| **The deferred store's bound and exhaustion policy** | Phase 3 work list | Separate from the above: deferring frees the upstream slot, so an ingress capacity does not bound the backlog |
| **When ADR-0001 clause 16's condition is evaluated, and whether a quantum may defer at all** | ADR-0001 clarification or successor, `Accepted` before Phase 3 implementation | The interim rule stated under HOST-INV-021 is a narrowing this specification is not entitled to make. It is not in force |

**`max_events_per_quantum` does not move with them, and this is the distinction that matters.** The field stays
normative here. It is the successor to `LIMIT-0075`, V1's uncapped per-block `Vec::with_capacity(128)` that grows inside
the audio callback, and it is the only thing in this profile that bounds that growth. What moves to Phase 3 is the
**runtime behaviour when a candidate set exceeds it** — the deferral mechanism — not the capacity itself. Dropping the
field along with the mechanism would leave V2's event path with the same unbounded allocation V1 has, which is the
defect the ledger opened `LIMIT-0075` to record.

**What Phase 1 must therefore say about its own event input.** Phase 1 is not "compiles but does not render": its API
accepts a `TimedEvents` span, its harness renders caller-selected frame counts, and its exit gate requires
deterministic rendering with an allocation-free render loop and no silent clipping of event fan-out. With the runtime
behaviour deferred, that boundary needs a rule or it accepts arbitrary input while defining neither an error nor an
overflow behaviour — the exact shape of defect the first review pass found in the runtime contract.

The rule is: **Phase 1's event input is a prevalidated bounded span, and the bound is checked where a failure can be
returned.** Two halves, because they land at different points in the API:

1. **At plan preparation**, which already returns a `ResourceReport` and a `CompileError`: a plan whose statically
   knowable per-quantum event count exceeds `max_events_per_quantum` is **refused**, naming the requested and
   available counts and the authored object responsible. This is the existing admission behaviour in the failure
   table, not a new one, and it needs no API change.
2. **At `Renderer::render`**, which the master plan defines as returning `()`
   ([Phase 1 work list](../master-plan.md#phase-1-experimental-sound-core-v2-crate)): the span holding at most
   `max_events_per_quantum` events per quantum is a **documented precondition of the caller**, not something the
   renderer can report. A renderer must not defer, drop, clip, or grow to absorb a violation.

**The second half is not enforceable as written, and this specification does not pretend otherwise.** A precondition a
signature cannot express is a comment, and this document's own history says what happens to a rule that only exists in
prose. Phase 1 owes **one** of two things, and choosing between them is Phase 1's, not this specification's:

- a fallible render signature, so the violation is returned rather than assumed away; or
- some other **release-active** failure mechanism the render loop can afford, since every alternative this
  specification already forbids — dropping, clipping, growing, deferring — is off the table.

A `debug_assert` is **not** one of the two, and an earlier revision of this passage offered it as though it were. It
compiles out of the release build, which is the build that runs, so it defines nothing where the definition is needed,
and a conformance test over it would pin only the debug configuration. Add one as a development aid; it closes
nothing.

Until one of them exists, the honest statement is that Phase 1 has an unchecked precondition here, and it is listed
under [*Unresolved questions*](#unresolved-questions) rather than presented as settled. What is not in doubt is the
negative half: **no phase may absorb an over-full quantum by allocating**, which is the constraint the deferral
mechanism was reaching for and the one part of it that survives being deferred.

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

/// The subset `::declared` may set. `Device` is absent by construction, so a
/// declared profile cannot claim to have queried a device.
pub enum DeclaredSource {
    Offline,
    Harness,
}

impl HostCapabilities {
    /// The device path. Every capability is an argument; there is no default for
    /// any of them, and no `..Default::default()` tail. Sets `Device`.
    pub fn from_device(
        sample_rate: SampleRate,
        maximum_block_size: FrameCount,
        channel_layout: ChannelLayout,
    ) -> Result<Self, ProfileError>;

    /// The offline and harness paths, which have no device to query. Sets the
    /// source it is given, so the tag cannot disagree with the constructor.
    pub fn declared(
        sample_rate: SampleRate,
        maximum_block_size: FrameCount,
        channel_layout: ChannelLayout,
        source: DeclaredSource,
    ) -> Result<Self, ProfileError>;
}

/// What the operator budgets. Raisable, at a cost the `ResourceReport` accounts for.
#[must_use]
pub struct RenderLimits {
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
| `SampleRate` | Hz | Existing `synth_core` newtype |
| `FrameCount` | frames | ADR-0032 clause 2; carries the block size, the horizon, and the crossfade |
| `NodeCount`, `EdgeCount`, `FanOut` | nodes, edges, edges per port | Graph extents after polyphony expansion |
| `VoiceCount` | voices | The V2 type; V1's clamping `synth_core::VoiceCount` is replaced, not reused (ADR-0021 part 3) |
| `BusCount`, `SendCount`, `ChannelCount` | buses, sends, mix channels | |
| `EventCount` | events | Per quantum, per tick, and per queue |
| `HeldNoteCount` | held notes | Distinct from `VoiceCount`: a held note is a source obligation, a voice is a resource, and more notes can be held than sounded |
| `TapCount` | taps | Observation surface |
| `SlotCount` | slots | Modulation, script host, script state, script output |
| `InstructionCount` | instructions | Script work per program. The per-quantum aggregate is a `ResourceReport` quantity, not a profile field |
| `PreparedBytes` | bytes | Three separate fields; the type carries the unit, the field carries the kind |
| `CostRatio` | dimensionless | Predicted quantum cost over the quantum's real-time budget |

**Ownership.** The application constructs the profile off the audio thread and hands it to the compiler. The compiler
reads it and produces a prepared plan plus a `ResourceReport`. The renderer reads the prepared plan and never the
profile. Nothing else holds a reference: a profile is an argument, not a service.

**Offline and test rendering.** An offline render has no device to query, so `CapabilitySource::Offline` records that the
capability half was declared rather than discovered, and a report or receipt that quotes a profile can say which it was.
HOST-INV-003 is therefore **scoped to the device path** rather than universal: a job that declares its capabilities
through `::declared` is honest, and a device path that fills one in from a constant is the defect. Review found the
earlier unconditional wording unsatisfiable — it forbade the offline path the same model provides, and it asked for a
conformance test no runtime tag can pass, since a `Device` tag cannot prove that a query happened. The two constructors
move the guarantee from a claim about values to a property of the API shape.

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

| Field | Type | Default | Basis | Replaces | Revisit |
|-------|------|---------|-------|----------|---------|
| `sample_rate` | `SampleRate` | Queried | Queried | — | — |
| accepted rate range | `SampleRate` pair | 8 000 – 192 000 Hz | Derived from `DeviceSampleRate::MAX_SUPPORTED`, which `MAX_RENDER_SAMPLE_RATE` already derives from after pass 3's fix | `LIMIT-0004` | Phase 9 |
| `maximum_block_size` | `FrameCount` | Queried; no compiled-in ceiling | Queried | `LIMIT-0001`, `LIMIT-0057` | Phase 9 |
| `channel_layout` | `ChannelLayout` | Queried on the device path; supplied by the caller on the declared paths. No compiled-in default | Queried; **ADR-0002 owns what a layout may be** | `LIMIT-0059` | Phase 2 |

**On the block ceiling.** V1 has two numbers: an engine-wide `MAX_BLOCK_SIZE` of 4 096 frames and a hardcoded advertised
request range of 128–1 024. V2 keeps neither as a constant. The queried value sizes the carries (HOST-INV-012), so an
implausibly large device block becomes a prepared-memory question the memory budget answers, rather than a separate cap
that can disagree with the memory it implies. A callback larger than the queried maximum remains ADR-0021 part 3's
terminal stream-contract fault.

**There is no floor either.** A device whose largest block is smaller than one quantum is admitted unchanged; see
HOST-INV-012 for why the render model already handles it and why an earlier `maximum_block_size >= Q` clause was a
defect rather than a safety margin.

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
| `max_events_per_quantum` | `EventCount` | 256 | **Chosen**, and the value is not carried over: `LIMIT-0014`'s constant is an egress ring size, and V1's real per-block buffer has no cap at all. See below | `LIMIT-0075` — the unbounded `Vec` this field replaces | Phase 3 |
| `max_note_expansion_per_tick` | `EventCount` | 128 | V1 carry-over. Admitted against `max_events_per_quantum` where the expansion is statically knowable, and covered by HOST-INV-021 where it is not — see below. Part 2 forbids trimming an expansion to fit | `LIMIT-0043` | Phase 3 |
| `max_scheduled_events_in_flight` | `EventCount` | 4 096 | Chosen. Bounds the scheduler's release window under ADR-0032 clause 27; **self-limiting** — see below. V1 has no antecedent because it has no scheduler | — | Phase 3 |
| `forward_event_horizon` | `FrameCount` | `max(one second at the prepared rate, maximum_block_size + Q)` | Chosen, with a derived floor — see below | — | Phase 3 entry (ADR-0022) |
| `command_queue_capacity` | `EventCount` | 16 384 | V1 carry-over. **Live bounded queue** | `LIMIT-0012` | Phase 9 |
| ~~`event_queue_capacity`~~ (critical / high / normal / low) | — | **Withdrawn** | **This field is removed from the profile, and the removal is the correction rather than a simplification.** It was a V1 carry-over of `LIMIT-0013`'s four prioritized rings, admitted through HOST-INV-005 ground 1 by that ledger entry's `HostProfile` ownership. [ADR-0038](../decisions/ADR-0038-engine-egress-queue-classification.md) part 3 establishes that the prioritized channel is **never constructed outside its own tests** — `prioritized_event_channel` (`crates/synth_engine/src/event_priority.rs:99`) has no production caller — so the entry moves to `N/A — removed` and this field loses its only admissibility ground. Sizing a V2 capacity from four rings that V1 never ran would carry an unvalidated shape forward: nothing has established that four priority tiers, or these four numbers, are right for anything. V2's engine egress is ADR-0038 part 1's rule with a capacity per surviving entry — `event_egress_capacity` for the GUI ring, and the protocol contract's own for `LIMIT-0076`. If V2 wants priority classes on egress it designs them from a requirement | ~~`LIMIT-0013`~~ — none; the entry is `N/A — removed` | Withdrawn |
| `event_egress_capacity` (engine-to-GUI event ring **only**) | `EventCount` | 256 | V1 carry-over of what `LIMIT-0014`'s constant sizes on the GUI side. **The field is one capacity, not two, and that is a change.** Earlier revisions carried `EventCount x 2` because one V1 constant sizes a GUI ring and an OSC note-telemetry ring; [ADR-0038](../decisions/ADR-0038-engine-egress-queue-classification.md) part 4 splits the ledger entry, and the OSC ring becomes `LIMIT-0076` owned by the protocol contract that serializes it — **not a profile field**. The two may take different sizes from that point on, which is the reason to split them. **Loss semantics are ADR-0038's, not HOST-INV-009's**: this is engine *egress*, which does not meet ADR-0021's definition of a live bounded queue, so dropping is licensed by ADR-0038 part 1 and only under its three conditions — observational payload, counted drop, count in the structured diagnostics report. V1 satisfies none of the three on this ring, which is why the field's conformance work is Phase 5's rather than Phase 1's. **`RecordedNotesFlushed` is custodial under ADR-0038 part 2 and may not share this capacity** | `LIMIT-0014` | Phase 5 |

**The release window is self-limiting, and the confirmation read is why that is written down.** (**Deferred with
HOST-INV-021** — `max_scheduled_events_in_flight` is the scheduler's field and the scheduler is Phase 3's; the
paragraph states what Phase 3 inherits, not a Phase 1 obligation.) Nothing exceeds
`max_scheduled_events_in_flight` from outside: the scheduler owns its own release rate and releases at most that many
compiled events at a time. Reaching it therefore delays a release rather than failing one, and nothing is lost — the
events are still in the plan. That is the field's failure behaviour, and the first draft of this section stated none —
the third field in this specification to be enforced at runtime with no defined behaviour on reaching it, after the
recording capacities and the retirement budget. The shape recurs often enough to be worth naming: **a field whose
bound is a runtime quantity needs its behaviour written even when the answer turns out to be "it cannot bind".**

**When a delayed release makes an event miss its quantum, both counters rise, and getting that wrong twice is what
clarified what each counter is for.** The confirmation read said such an event is "counted by ADR-0001 clause 16 like
any other late event" and said nothing about the cause. The pass after it swung the other way and *suppressed* the
late counter, on the reasoning that the producer was the engine. **Both were wrong, and in the same place.** ADR-0001
clause 16 is an accepted contract whose trigger is a *condition* — "an event whose timestamp falls in an
already-rendered quantum is late" — not a cause; nothing in it asks who was at fault, and a specification may not
narrow an accepted decision by glossing it. Here the condition genuinely holds, unlike in HOST-INV-021's case where
the quantum has not rendered. So clause 16 applies in full: clamped forward **and counted late**. The
capacity-deferral counter rises alongside it, which is the attribution the confirmation read was missing, added
without removing a count the specification does not own.

**More events can be due in one quantum than the scratch admits, and that case has a rule now.** Two sources produce
the overrun. Live ingress is unbounded in time by definition — a MIDI burst, a gesture stream, an MPE controller — so
nothing about a prepared plan bounds how many events arrive stamped for one quantum. And a script-driven note graph's
expansion is data-dependent, so `max_note_expansion_per_tick` cannot be checked at admission for the case that matters.
Neither is covered by admission: ADR-0021 part 1's "a prepared plan may not exceed a `HostProfile` limit at runtime"
binds the *plan*, and neither of these is part of it.

**An earlier revision argued this from the wrong numbers**, and the argument is withdrawn rather than repaired. It said
"the four ingress queues hold 3 072 events between them while `max_events_per_quantum` is 256". Read at `3555c52c`,
those are `LIMIT-0013`'s prioritized rings, and they are **egress**:

- `crates/synth_engine/src/event_priority.rs:1-5` — the module's own summary is "Priority-based event system for
  GUI-Engine communication";
- `event_priority.rs:44-55` — `TimestampedEvent` wraps an `EngineEvent`, which is the *out* direction under the
  architecture's `EngineCommand` (in) / `EngineEvent` (out) split (`CLAUDE.md`, *Thread Model*);
- `event_priority.rs:75-78` — the 256 / 256 / 512 / 2 048 buffer sizes the 3 072 figure came from;
- `crates/synth_engine/src/lib.rs:58-59` is the **only** occurrence of `PrioritizedEventProducer`,
  `PrioritizedEventConsumer`, or `prioritized_event_channel` outside the defining module — a re-export. Nothing in the
  workspace constructs the channel, so in V1 it is unwired as well as pointed the wrong way.

So they cannot bound what one quantum is presented with. The claim entered at the first independent review pass and
survived four more, which is worth recording: **a number that supports the conclusion you already hold does not get
audited.** It was found by an external reader checking it against the source rather than against the argument.

**Both numbers in that premise were wrong, and the second one reaches the ledger.** `max_events_per_quantum` = 256 was
labelled a V1 carry-over of "the engine event scratch", citing `LIMIT-0014`. At `3555c52c`,
`crates/synth_engine/src/synth_engine.rs:81` defines `EVENT_BUFFER_SIZE = 256`, and its only two uses are
`synth_engine.rs:811` (`HeapRb::<EngineEvent>`, the egress ring again) and `:815` (`HeapRb::<NoteEvent>`, OSC
telemetry). **It is not a renderer scratch.** What V1 actually fills per block is
`sequencer_event_buffer: Vec<SequencerEvent>` — `synth_engine.rs:722`, allocated `Vec::with_capacity(128)` at `:895`,
cleared and refilled at `:4139-4141` — a capacity *hint* with no cap, the same shape as `LIMIT-0031`.

So **V1 has no per-quantum event limit**, 256 is chosen rather than carried over, and
[`LIMIT-0014`](../inventories/resource-limits.md)'s description and citation are wrong in the ledger. That is a
**P00A-T004 finding against a task marked `Complete`**: the entry was recorded from a constant's *name* across two
discovery passes, and pass 3 — which read enforcing code to assign classes — did not reach its use sites either. The
ledger entry still maps to a successor field, so coverage stands; what fails is its basis. (The count itself moved separately: `LIMIT-0015` left the cohort in the same audit, taking it from 30 to **29**, and
`LIMIT-0014` was then undecided pending a GUI/OSC split, which ADR-0038 part 4 has since performed; the count is
**28**, and every one is settled.) The no-antecedent count
rose to eight and returned to seven once review registered `max_events_per_quantum`'s real antecedent, `LIMIT-0075`.

**V1 has no timestamped renderer-ingress queue to carry over either.** `LIMIT-0012`'s command ring
(`crates/synth_engine/src/synth_engine.rs:49`, `CommandCapacity::DEFAULT` = 16 384) is the only in-direction capacity it
has, and it carries `EngineCommand`s rather than positioned events. Defining V2's ingress streams and their capacities
is Phase 3's, and is listed as unresolved.

> **Deferred to Phase 3.** Everything from here to the end of this subsection describes HOST-INV-021's deferral mechanism, which is **not normative** — see [*Deferred to Phase 3*](#deferred-to-phase-3). It is retained as the specification of the deferred work and the record of what three earlier revisions of it got wrong. No Phase 1 or Phase 2 implementation may act on it.

HOST-INV-021 fixes it: **the excess is deferred, not dropped.** An event that does not fit has its *render position*
advanced by exactly `Q` frames — the following quantum, same offset — is re-evaluated there, and is counted under its
own capacity-deferral counter; never applied retroactively and never silently discarded. **The envelope's stamp is not
rewritten**, so the report can say how far an event was displaced and a recorded performance is not quantized forward
by an overload. **Which events are the excess is a rule**, not a consequence of iteration order: compiled events are
admitted before ingress ones because a compiled position is exact by construction while an `Arrival` one carries
declared-unmeasured error, and within each group admission is by ascending render position, so the tail is what defers.
That second rule is what covers a *compiled* overrun, which `max_note_expansion_per_tick` makes reachable on its own.
Dropping stays where ADR-0021 already puts it, at the live bounded queue, which is the one place an external producer
can outrun the engine and the one place a drop is counted.

**The counter is deliberately not ADR-0001 clause 16's, and it took three attempts to say why correctly.** The first
draft charged deferral to clause 16's counter. The second suppressed it on the grounds that the *producer* was not at
fault — which reads a cause into a clause that states none, and is the gloss the external pass rejected. The reason
that holds is about clause 16's **condition**: it fires for "an event whose timestamp falls in an already-rendered
quantum", and the quantum that could not admit this event has not rendered. Its position rule does not transfer either
— "the first not-yet-rendered quantum boundary" is circular here for the same reason. ADR-0032 clause 22 is the
precedent for keeping the two counters apart, and warns that one test would pass on the wrong policy.

**When that condition is evaluated is an open question, and it is ADR-0001's.** Once quantum `k` renders without a
deferred event, that event's preserved timestamp *does* fall in an already-rendered quantum, so a literal reading of
clause 16 would fire the late counter on the next re-evaluation — which this invariant forbids. The two rules cannot
both be implemented as written. The interim rule this specification operates under is that **clause 16's condition is
asked once, when an event first becomes due, and deferral does not re-ask it**: an event admitted on time was never
late, and re-asking answers a question already answered. That is a narrowing of an accepted decision, so this
specification does not get to make it — it is registered as an unresolved question requiring an ADR-0001 clarification
or successor **before Phase 3 implements either rule**.

**Note expansion needs both answers for the same reason.** `LIMIT-0043`'s ledger rule refuses an over-expanding tick at
preparation, which works for a deterministic processor whose expansion the compiler can compute. A script-driven note
graph's expansion is data-dependent and not knowable then, so admission bounds what it can and HOST-INV-021 carries the
rest. Without that split the field would have had a compile-time rule and a runtime hole, which is the shape of defect
this section exists to close.

This makes `max_events_per_quantum` a smoothing budget rather than a cliff: a burst is spread over the following
quanta at `Q` frames of added delay per deferral, with the intra-quantum offset preserved. **That delay is a real
degradation and not a re-charge of one ADR-0001 already takes.** Clause 14 charges up to `Q - 1` frames to the
*control-rate response* of a mid-quantum event and leaves the event's sample-positioned effects where they were
stamped — its consequences say so explicitly, "note and gate timing is unaffected". Deferral moves the whole event,
note and gate included. Preserving the offset is what keeps the degradation to a whole-quantum shift instead of also
destroying the spacing inside the burst, but it is still a shift, and a sustained overrun is visible as a rising
capacity-deferral count and a rising per-event displacement. Phase 3 owns starvation, which has **two** channels rather
than one: compiled precedence starves ingress as a group, and the offset-preserving `+Q` with no age term starves a
late-positioned ingress event against early-positioned ones. Both are listed as unresolved questions.

**The forward horizon, and why one second.** ADR-0032 clause 21 makes this one profile field, binding ingress
provenance only, because an event held for an unbounded time pins a queue slot. Three quantities bound the choice from
below: a host delivers a block's events stamped within that block (at most `maximum_block_size`), an adapter may stamp
slightly ahead, and HOST-INV-013 requires at least `maximum_block_size + Q`. Nothing bounds it from above except the
cost of a pinned slot — **and that cost cannot be quantified yet**. An earlier revision put it at
`event_queue_capacity` slots times the horizon; that field sized the engine's egress rings and is now withdrawn entirely, so it never sized ingress
slots. The quantity is the ingress capacity times the horizon, and the ingress capacity is what Phase 3 must define.

One second is chosen because it is far above every legitimate ingress stamp — a 4 096-frame block is 85 ms at 48 kHz —
and far below the duration at which a stuck slot stops looking like a scheduling decision and starts looking like a
leak. It is deliberately *not* a musical duration: an external producer that wants to schedule a note a bar ahead
belongs in the plan, which the scheduler releases quantum by quantum and which clause 21 explicitly does not measure
against this horizon.

**The default takes the maximum of that second and HOST-INV-013's floor, and the closure pass is why.** A flat one
second was a default that could fail its own validation: `maximum_block_size` is queried and has no compiled-in
ceiling, so a device reporting a block above one second's worth of frames would produce a profile whose horizon is
below `maximum_block_size + Q` — refused at construction, on a device the specification otherwise admits. Such a device
is implausible, which is exactly why it would not have been found by testing. Deriving the floor removes the case
instead of relying on nobody meeting it.

The value is revisited at the Phase 3 entry gate with ADR-0022, because the thing that produces an absurd forward
timestamp is a mis-calibrated epoch anchor, and ADR-0022 owns calibration. Until it is accepted, an out-of-horizon event
is rejected and counted, which is a diagnostic for exactly that fault.

### Observation

| Field | Type | Default | Basis | Replaces | Revisit |
|-------|------|---------|-------|----------|---------|
| `max_observation_taps` | `TapCount` | 128 | V1 carry-over; the silent drop becomes a compile error. **ADR-0027 owns what a tap is** | `LIMIT-0020`, `LIMIT-0062` | Phase 5 |
| `telemetry_ring_frames` | `FrameCount` | 4 096 | V1 carry-over. **Lossy** (HOST-INV-019) — but a **behaviour change**, not a carry-over of behaviour: V1 drops the *newest* samples (`crates/synth_engine/src/visualizers/mod.rs:126-146` at `3555c52c`, the writer `master_scope` actually uses). **Since `bac88c0c` the loss is no longer silent**: `read_samples_into` returns the omitted count under `#[must_use]` (`:280`, taken at `:321`, returned at `:329`), which satisfies ADR-0038 part 1 condition 3 in its data-paired form. What is still missing is presentation — no GUI surface draws the gap — so HOST-INV-019's *expose the loss* condition is met at the API and not yet at the surface | `LIMIT-0021` | Phase 5 |
| `analyzer_fft_size` | `FrameCount` | 2 048 | V1 carry-over. A resolution budget; the size travels with the payload | `LIMIT-0022` | Phase 5 |

**The class ADR-0021 gave these rings does not fit them, and this specification may not change it.** ADR-0021 part 1
reserves runtime overflow for queues "fed by external, unbounded-in-time input such as MIDI or user gestures". These
are fed by the engine. The *behaviour* the record chose is still defensible — visualization data is droppable,
diagnostics are not, which is why V1 has four priorities — but the class it was chosen under describes a different kind
of queue, and the record made that classification while the direction was misread here as well. Correcting an accepted
decision is not this specification's to do, so the conflict is an unresolved question needing an **ADR-0021
clarification or successor**, alongside the clause-16 one. What must not happen meanwhile is HOST-INV-009's dropping
licence being read as covering engine state and diagnostic events because of a label.

**ADR-0021's `LIMIT-0013` evidence is false in both halves, and that is a third accepted-decision conflict.** The
record's decision drivers say "`LIMIT-0013` has per-priority drop counters published on OSC, which is closer to a
telemetry channel than to a user-visible diagnostic", and its disposition promotes them "from an OSC-only publication"
into the structured diagnostics report. At `3555c52c` neither half holds: `get_dropped_counts`
(`event_priority.rs:194`) has no caller anywhere, so the counters are published *nowhere*, and OSC's
`/synth/engine/event_drops` reads `EngineState::event_drops` (`state.rs:585`) — a different counter on a different
ring. The conclusion survives and gets stronger — a counter nobody reads is worse than a counter read over OSC — but
the record cites a state of the world that does not exist, and correcting it is ADR-0021's, not this specification's.

**Nor is `Critical` undroppable.** `PrioritizedEventProducer::send` (`event_priority.rs:141-150`) uses one
`try_push` per priority, `Critical` included, and increments that priority's drop counter on failure. The module's own
doc comment says critical events "are never dropped"; the code gives them a separate ring, not a guarantee. **No field
here may promise non-droppable diagnostics**, and an earlier revision of this section did.

`LIMIT-0020` is the field's reason for existing: V1 publishes meters through a 128-slot array whose `publish()` is an
`if let Some(slot)`, so a project with more metered channels loses meters with no signal to anyone. Under ADR-0021 part
3 that is a compile error naming the requested and available tap counts.

### Mixing

| Field | Type | Default | Basis | Replaces | Revisit |
|-------|------|---------|-------|----------|---------|
| `max_mix_channels` | `ChannelCount` | 256 | Chosen | — | Phase 8 |
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

**Two capacities with a floor relation, and the reason is weaker than an earlier revision claimed.** `LIMIT-0023` (Mod Matrix slots per voice) and `LIMIT-0041` (script host slots) were collapsed into a single
field on the ledger's claim that V1 holds them "coupled 1:1 by a compile-time assertion". The use-site audit read the
assertion: `crates/synth_modules/src/mod_matrix.rs:30` at `3555c52c` is
`assert!(MAX_MOD_MATRIX_SLOTS <= synth_core::script::SCRIPT_HOST_SLOTS)` — an **inequality**. Lowering the host slots
below the matrix count breaks the build; raising them alone is legal. **What that does *not* show is a second
consumer.** An earlier revision argued `ScriptHost` is module-agnostic and therefore serves unrelated modules; at
`3555c52c` the only production `ScriptHost::new()` in the workspace is the Mod Matrix's own
(`crates/synth_modules/src/mod_matrix.rs:55`), the others being tests in `script/host.rs`. The type is generic; V1 does
not exercise that. So the split rests on the `<=` assertion alone — V1 permits divergence and does not use it — which
makes two fields a **V2 design choice** rather than a reading of V1, and Phase 7 may reasonably collapse them again.

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

**The ledger's 28 `HostProfile`-owned entries**, each with its successor field. The count has moved twice and both moves are recorded rather than absorbed: it was 30 before the pass-4 use-site audit found `LIMIT-0015` to be four deferred-drop channels and moved it to `N/A — removed`, and it reached 28 again when ADR-0038 removed `LIMIT-0013` — a channel never constructed outside its own tests — and split `LIMIT-0014`, whose GUI half is `HostProfile`-owned while its OSC half became `LIMIT-0076` under the protocol contract. **The mapping is now total**; every previous revision of this table carried at least one entry with no settled owner:

| Entry | Field |
|-------|-------|
| `LIMIT-0001` | `maximum_block_size` |
| `LIMIT-0002`, `LIMIT-0003` | `buffer_scratch_bytes`; sized from `maximum_block_size` rather than set independently |
| `LIMIT-0004` | Accepted rate range |
| `LIMIT-0012` | `command_queue_capacity` |
| `LIMIT-0013` | **None.** The prioritized channel is `N/A — removed` under ADR-0038 part 3 — it is never constructed outside its own tests — so it has no successor field, and `event_queue_capacity` is withdrawn |
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

**Seven fields have no V1 antecedent**, which is where V1 had nothing rather than something wrong: `max_active_voices`,
`max_scheduled_events_in_flight`, `forward_event_horizon`, `max_mix_channels`, `max_buses`,
`max_concurrent_retiring_voices`, and `predicted_quantum_cost_ratio`.

**The count went to eight and back to seven**, which is worth recording. `max_events_per_quantum` was listed here when
the use-site audit showed `LIMIT-0014` is an egress ring; review then found the real antecedent — `LIMIT-0075`, V1's
uncapped `sequencer_event_buffer`, which nobody had registered. The antecedent existed all along; the ledger pointed at
the wrong constant. Only the *value* 256 is unsupported by V1, which is why the field's basis stays *chosen*.

An earlier revision listed eleven, and review found the count wrong in three separate ways. The three memory aggregates
were listed as having no antecedent while their own rows named `LIMIT-0002`, `LIMIT-0003`, and `LIMIT-0073` — they are
new as an *aggregate*, which is `LIMIT-0073`'s finding, but the resources themselves are V1's.
`max_script_instructions_per_quantum` is no longer a field at all. And eleven items were summarised as ten in the phase
tracker and `STATUS.md`. The count has since moved three times more — to seven after that correction, to **eight** when the use-site audit
showed `LIMIT-0014` is an egress ring, and back to **seven** when review found the real antecedent, `LIMIT-0075`.

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
| A live bounded queue overflows at runtime | The item is dropped and counted per queue and priority (HOST-INV-009) | Structured diagnostics report |
| More events are due in one quantum than `max_events_per_quantum` admits | **Deferred to Phase 3 — this row is not normative.** In Phase 1 and Phase 2 the candidate set is a prevalidated bounded span and an over-full quantum cannot arise; the mechanism below is what Phase 3 inherits. **Conditional even then, until Phase 3 bounds the deferred store** — the guarantee below holds only while that store has room, and its exhaustion behaviour is undefined. The excess is **deferred**: the render position advances by exactly `Q` frames with the offset preserved and the envelope's stamp untouched, counted under the capacity-deferral counter — and not under ADR-0001 clause 16's late counter, whose *condition* does not hold because the quantum has not rendered (see the open question on when that condition is evaluated). What defers is the tail of a defined admission order: compiled before ingress, then ascending render position (HOST-INV-021) | Structured diagnostics report, with the per-event displacement as well as the count |
| The scheduler's release window `max_scheduled_events_in_flight` is full (**deferred with HOST-INV-021**) | The release is delayed, never failed; nothing is lost, since the events are still in the plan. Where the delay makes an event's timestamp fall in an already-rendered quantum, ADR-0001 clause 16 applies in full — clamped forward **and counted late** — and the **capacity-deferral** counter rises as well, attributing the cause to a profile capacity rather than to a producer | Structured diagnostics report |
| A lossy field's capacity is reached | The oldest data is evicted by design, and the evicted count or continuation marker is exposed (HOST-INV-019) | The surface presenting that data |
| A session limit is reached | The activity stops with a counted diagnostic; everything already produced is kept, and nothing authored is dropped (HOST-INV-020) | The recording surface, plus the structured diagnostics report |
| An ingress event is beyond `forward_event_horizon` | Rejected and counted | Structured diagnostics report |
| A callback exceeds `maximum_block_size` | ADR-0021 part 3's terminal stream-contract fault: silence, both carries invalidated, `needs_reprepare` published, nothing allocated | Structured diagnostics report |

**Five behaviours, plus one category that has none — and one of the five is deferred.** Admission refuses, a live
queue drops, a quantum defers (**HOST-INV-021, not normative until Phase 3**; four behaviours are in force meanwhile),
a lossy
budget evicts, and a session limit stops. The first review pass found an earlier draft claiming compile-time refusal for
every render limit while three fields visibly did something else at runtime — the telemetry ring overwrote, a recording
take stopped, and an over-full quantum did nothing defined at all. The taxonomy above is what reconciles them, and
HOST-INV-009's narrowed wording is what keeps "drop" from covering the other three.

**The release window's row is deferral, not a sixth behaviour.** `max_scheduled_events_in_flight` and
`max_events_per_quantum` bind at different points and share one behaviour: the event moves forward and the
capacity-deferral counter records that a profile capacity was the cause. What differs is whether ADR-0001 clause 16
*also* applies, and that turns on its condition rather than on any choice made here. Under HOST-INV-021 the quantum has
not rendered, so the condition fails and only the deferral counter moves; a delayed release misses a quantum that has
rendered, so clause 16 applies in full and both counters move. Two fields, one behaviour, and one accepted decision
that this specification reads rather than reinterprets.

**Sizing fields bound nothing and cannot be exceeded**, so asking which behaviour they take is a category error:
`analyzer_fft_size` sets a resolution, `telemetry_ring_frames` sets a window length (its *eviction* is the lossy
behaviour, not an overflow), `retirement_crossfade` sets a duration, and the accepted rate range and `channel_layout`
describe what a stream *is* rather than how much of it there may be. They cost prepared memory and therefore appear in
the `ResourceReport`; they have no failure row because they have no failure. The closure pass added this paragraph:
HOST-INV-009 had claimed every profile field falls under one of four runtime behaviours, which was false twice over —
it omitted admission refusal, the behaviour most fields actually take, and it had no place for a field that is a size.

Every counter named above reaches the **structured diagnostics report**, which is the report a Phase exit review
inspects. This is the specific control against the failure mode ADR-0021 records twice: `LIMIT-0013`'s drop counters
existed for years and reached **no consumer at all** — `get_dropped_counts` has no caller, and the OSC feed publishes a different ring's counter. Both the ledger and ADR-0021 recorded them as "OSC-only", which was the more flattering of the two possibilities.

## Real-time and resource constraints

- The profile is read only off the audio thread (HOST-INV-002). The renderer holds no reference to it.
- Admission and preparation may allocate; both run off the audio thread.
- Every capacity the audio thread relies on is preallocated at preparation, including both carries, the event scratch,
  the recording buffers, and each node's mutable state.
- Nothing in the render loop consults a limit to decide whether to allocate. A plan that was admitted fits by
  construction; a plan that does not fit was refused.
- The runtime-variable quantities are the live bounded queues' occupancy, the number of events due in one quantum, and
  a take's length. None of the three allocates: the queue drops and counts (HOST-INV-009), and the take stops and
  counts (HOST-INV-020). **The middle one — the number of events due in one quantum — has no runtime behaviour in
  Phase 1 or Phase 2**, because HOST-INV-021 is
  deferred: the candidate set is a prevalidated bounded span, so an over-full quantum is a caller contract violation
  rather than a state the render loop handles. The constraint that survives the deferral is the negative one — nothing
  in the render loop may allocate to absorb an over-full quantum, in any phase.
- **Admitting events into a quantum must allocate nothing and do work bounded by declared capacities**, over
  preallocated storage. The real problem today is that the candidate set has no declared bound at all: the ingress
  streams and the deferred store are undefined, so no ordering strategy can be shown to terminate in bounded time.
  HOST-INV-021 states the constraint and stops there. Two revisions overreached before it did — one prescribed a
  five-way merge over rings that turned out to be engine *egress*, the other banned comparison sorting, which a bounded
  candidate set makes unnecessary. The streams, their capacities, and the bounded deferred store are Phase 3's.

## Conformance tests

No test exists yet: the V2 crate is Phase 1's, and this specification is written before it. Each row names what must
exist, in the phase that builds the thing it tests.

| Invariant | Named test or evidence | Phase |
|-----------|------------------------|-------|
| HOST-INV-001, HOST-INV-002 | A prepared plan renders after its source profile is dropped; the renderer holds no profile reference | 1 |
| HOST-INV-003 | Two tests, because no runtime check can see where a value came from. **Shape:** `HostCapabilities::from_device` has no defaulted parameter and no `Default` impl, so a caller cannot omit a capability. **Behaviour:** the cpal adapter is driven with a device reporting a non-default buffer range and the resulting profile carries that range — the direct regression test for `LIMIT-0057`, which discarded it | 9 |
| HOST-INV-004 | Partly a review check — no automated test can see that a default was *reasoned* from `Q`. The mechanical half is ADR-0032 clause 4's compile-time assertion `Q <= QuantumOffset::MAX`, which fails the build when `Q` changes and something was sized to its old value, plus a test that `HostProfile` exposes no field carrying a quantum | 1 |
| HOST-INV-005 | A test enumerating profile fields against the invariant's three grounds: a ledger entry owned `HostProfile`; a field an accepted ADR creates; and the **enumerated residual set** the invariant lists, each of whose members carries a stated basis and revisit point. The test compares against that explicit list — not against "everything else", which would admit a protocol- or job-owned capacity by default. It asserts each field matches exactly one and fails on a field in none. **It must not enumerate the no-antecedent list**, which is a different axis, and **must not treat a `Replaces` entry as a ground** — `max_held_notes` and `max_events_per_quantum` name `LIMIT-0031` and `LIMIT-0075` as provenance while being admitted by the residual | 1 |
| HOST-INV-006 | Every compile — succeeding and failing — returns a report whose every field has requested, available, and a dominant contributor | 1 |
| HOST-INV-007 | One refusal case per render limit, asserting the error names the field, both amounts, and the authored object; and asserting the plan is unchanged | 1 |
| HOST-INV-008 | A node whose declared capacity is exceeded is refused, and raising every profile field does not admit it | 2 |
| HOST-INV-009 | Each live bounded queue is overrun and its drop count is asserted in the diagnostics report | 3 |
| HOST-INV-010 | A project saved under one profile loads and compiles under another; no serialized field names a profile value | 10D |
| HOST-INV-011 | The same plan is admitted at 44.1 kHz and warned at 192 kHz by the cost budget alone, with no field count changed | 3 |
| HOST-INV-012 | Callback sizes from 1 frame to `maximum_block_size`, including non-multiples of `Q`, render identically to a single large block — the partition-invariance suite of ADR-0001. Plus one profile whose `maximum_block_size` is **below** `Q`, which must be admitted and must render identically | 3 |
| HOST-INV-013 | An ingress event one frame beyond the horizon is rejected and counted; a compiled event hours ahead is released normally; and an admitted event is never re-checked, asserted by the deferral case above | 3 |
| HOST-INV-014 | The compiler's aggregate equals the sum of node-declared prepared bytes for a plan built from known nodes | 2 |
| HOST-INV-015 | A plan over the cost budget compiles and warns; no advisory field can produce a `CompileError` | 1 |
| HOST-INV-016 | A profile with `forward_event_horizon < maximum_block_size + Q` fails construction naming both fields — **and the default profile satisfies it at every admissible `maximum_block_size`**, including one above a second's worth of frames, which is the case the flat one-second default failed | 1 |
| HOST-INV-017 | The profile carries two fields and rejects a construction with `script_host_slots_per_voice < mod_matrix_slots_per_voice`, naming both; raising the host slots alone is accepted, which is what V1's `<=` assertion permits and the single-field model forbade. The assertion in `synth_modules` is gone | 7 |
| HOST-INV-018 | Every **quantity** field's type has a private field and a fallible constructor; no such field is a bare primitive, and `HeldNoteCount` does not convert to or from `VoiceCount`. The two **kind** fields, `channel_layout` and `source`, are asserted to be closed enums instead — the test enumerates both sets, so a new field must be classified rather than silently escaping the check | 1 |
| HOST-INV-019 | The telemetry ring is overrun and the reader can distinguish a complete window from an overwritten one | 5 |
| HOST-INV-020 | A take reaching each recording capacity stops, is counted, and keeps every event recorded before the stop; no note is dropped and no earlier note is overwritten | 9 |
| HOST-INV-021 (**deferred — see [*Deferred to Phase 3*](#deferred-to-phase-3); none of these tests may be written against Phase 1**) | A quantum is presented with more due events than it admits: the excess renders one quantum later **at the same intra-quantum offset**, no event is lost **while the deferred store has room** — the unconditional form cannot be tested until Phase 3 sizes that store, and a separate exhaustion case is owed once it chooses a policy — and a compiled event is never displaced by an ingress one. **Two counters, asserted separately** — the capacity-deferral counter rises by exactly the deferred count and the late counter does not move at all; and the mirror case, an event that is genuinely late, moves the late counter and not the deferral counter. ADR-0032 clause 22 is the precedent for why one test would pass on the wrong policy. A third case covers an event that is **both** late and deferred, asserting each counter rises exactly once. A fourth pins the admission order: a quantum over-full with ingress events alone defers the latest-positioned and not the last-arrived, asserted by presenting them in reverse position order; a fifth over-fills a quantum with **compiled** events alone, through a note expansion the compiler could not predict, and asserts the same tail rule. A sixth asserts the stamp is **immutable**: a deferred event's envelope `time` is unchanged after any number of deferrals, its render position is `clamped_position + Q x deferrals` — the clamped base, so an event that is both late and deferred is not sent back into a rendered quantum — and the diagnostics report carries the displacement — the direct regression test for a rewritten stamp quantizing a recorded performance forward. A seventh drives a repeatedly-deferred ingress event past `forward_event_horizon`'s worth of deferrals and asserts it is still rendered, never rejected | 3 |
| `max_scheduled_events_in_flight` (HOST-INV-021's counter, no invariant of its own; **deferred with it**) | The release window is saturated so that an event's timestamp falls in an already-rendered quantum: it is clamped forward and **both** counters rise — late, because ADR-0001 clause 16's condition holds, and capacity-deferral, because a profile capacity caused it. The specification got this field wrong in both directions before settling here, so the test asserts both counters rather than either | 3 |
| HOST-INV-020, and the retirement budget | A plan swap with `max_active_voices` sounding retires every voice with a crossfade and refuses none, so `max_concurrent_retiring_voices` cannot bind at its derived default | 9 |

## Unresolved questions

| Question | Blocking? | ADR or task |
|----------|-----------|-------------|
| **How Phase 1 enforces the per-quantum event bound at `Renderer::render`.** With HOST-INV-021 deferred, the span is a caller precondition, and the master plan's `render` returns `()` — so a violation cannot be reported. Phase 1 owes a **release-active** mechanism; a fallible signature is the obvious one, and a `debug_assert` is not one, since it compiles out of the build that runs. **Preparation-time refusal is unaffected** and needs no API change; this is only about the per-call span | **Yes for Phase 1**, which otherwise ships an unchecked precondition where the deferral mechanism used to be. No for Phase 2, which inherits whatever Phase 1 chooses | Phase 1 |
| What a channel layout is beyond mono/stereo, and whether the profile carries a layout set or one layout. **The premise that made this safe is false**: the pass-5 audit found `ChannelCount::from` returns `Multi(n)` for `n > 2` and the cpal backend feeds it the device's channel count, so a multichannel device constructs a layout the engine's buffers then ignore | **Yes for Phase 9**, which queries a real device; no for Phase 1. `channel_layout` is queried, so a profile can now carry a value nothing honours | ADR-0002, Phase 2 |
| What an observation tap is and who owns the analyzer surface; the three capacities here may become one registration budget | No — the capacities stand whatever the taps mean | ADR-0027, Phase 5 |
| The retirement crossfade's value, and whether ADR-0009 wants a concurrent-retirement budget below `max_active_voices` — which it may only take together with a defined behaviour for reaching it | No — V1's 128 frames compiles today, and the derived budget cannot bind | ADR-0009, Phase 9 |
| Recording take and commit semantics, which may change what a "recorded event" is | No | ADR-0024, Phase 9 |
| What a send is, which may change whether `max_sends_per_channel` is per channel or per bus | No | ADR-0034, Phase 8 |
| The script-work aggregate's threshold, which needs a measured per-instruction cost before it can become a `RenderLimits` field rather than a reported quantity | No — the `ResourceReport` carries the quantity meanwhile | Phase 7 |
| Whether HOST-INV-021's deferral can starve an event under sustained overrun. **Two channels, not one.** (a) Compiled events take precedence unconditionally, so a plan saturating `max_events_per_quantum` every quantum defers ingress indefinitely; the likely fix is a *reserved ingress allowance* the scheduler leaves free under ADR-0032 clause 27, turning unbounded starvation into a declared budget. (b) `+Q` preserves the offset and the admission order has no age term, so an event positioned late in its quantum loses to natively-due events positioned early in theirs every round — starvation among ingress events themselves, which only an age term addresses. Both are new design, and Phase 3 owns both | **Conditionally blocking for Phase 3.** "Nothing is lost" holds only while the deferred store has room, and no safe bound for that store exists yet — so this cannot be called non-blocking until the ingress and deferred-store contract lands. Displacement per event is at least reportable, because the stamp is immutable | ADR-0003, ADR-0023, Phase 3 |
| Whether ingress should have a `Hardware`-before-`Arrival` tier. Rule 1's basis is provenance exactness, and that basis would order the two ingress provenances as well; there is no such tier, because whether the difference is real depends on what the two uncertainties measure | No — the current order is stated and testable | ADR-0022, Phase 3 |
| **When ADR-0001 clause 16's condition is evaluated.** A deferred event keeps its stamp, so once the quantum it could not enter has rendered, its timestamp does fall in an already-rendered quantum and a literal clause 16 would count it late — which HOST-INV-021 forbids. The interim rule here is that the condition is asked once, when an event first becomes due, and deferral does not re-ask it. **That narrows an accepted decision, which this specification may not do**, so it needs an ADR-0001 clarification or successor | **Yes for Phase 3**, which cannot implement both rules as written. No for Phase 1 | ADR-0001, Phase 3 |
| ~~**ADR-0021's `LIMIT-0013` evidence.**~~ **Resolved in Phase 0A, not Phase 3.** Its drivers and disposition describe per-priority drop counters "published on OSC"; they are published nowhere, and the OSC counter it names belongs to another ring. [ADR-0038](../decisions/ADR-0038-engine-egress-queue-classification.md) supersedes both the driver and the disposition on that evidence | Resolved, subject to ADR-0038's acceptance | ADR-0038 |
| ~~**The class ADR-0021 gave `LIMIT-0013`'s rings.**~~ **Resolved in Phase 0A, not Phase 3.** They are engine egress, not "fed by external, unbounded-in-time input", so `Live bounded queue` never described them. [ADR-0038](../decisions/ADR-0038-engine-egress-queue-classification.md) part 1 supplies the missing rule and part 3 removes the entry outright, since its channel is never constructed outside its own tests | Resolved, subject to ADR-0038's acceptance. Until then HOST-INV-009's dropping licence must still not be read as covering engine state and diagnostics | ADR-0038 |
| **What V2's renderer-ingress streams are, and what bounds them.** V1 has no timestamped ingress queue to carry over — `LIMIT-0013`'s prioritized rings are engine *egress*, and `LIMIT-0012`'s command ring carries commands rather than positioned events — so the profile currently has no field for the capacity HOST-INV-021's deferral operates against. Bound up with it: the **deferred store**, which needs its own preallocated capacity, since deferral de-orders a FIFO and deferred events must be merged as a stream of their own | No for Phase 1, which compiles rather than renders live. **Yes for Phase 3**, which cannot build admission without them | Phase 3 |
| Whether ADR-0001 clause 16's clamp preserves or rewrites the event's stamp. HOST-INV-021 decides it for deferral — the stamp is immutable, the render position is derived — and the same question applies to the clamp, where a rewritten stamp would have the same three consequences. It is ADR-0001's to answer, and this specification deliberately does not answer it by implication | No — the two mechanisms are separately counted and separately testable | ADR-0001, Phase 3 |
| Whether `max_nodes` should be anchored independently rather than computed from `max_active_voices`, which is itself only measurement-anchored | No | Phase 2 exit |
| Whether `max_mix_channels` and `max_observation_taps` should be coupled so that every mix channel is guaranteed a tap | No — the report names which budget bound the plan | ADR-0027, Phase 8 |
| Where a profile is stored and who may edit it — application settings, host configuration, or neither | No — Phase 1 constructs it in code | ADR-0013, ADR-0029, Phase 10A |
| Whether the forward horizon survives calibration evidence, and whether a mis-calibrated anchor should widen it or fail preparation | No — the current value rejects and counts, which is the diagnostic | ADR-0022, Phase 3 entry |
| Whether `max_fan_out_per_port` should exist at all, or whether the edge budget alone suffices | No | Phase 2 exit |
| Whether `retirement_crossfade` and `telemetry_ring_frames` should be stated in seconds rather than frames. Both mean durations and both are flat frame counts, so both shrink 4.4x from 44.1 to 192 kHz; HOST-INV-011 does not reach them because neither bounds a plan, but the crossfade's shortening is audible where the ring's is cosmetic | No — V1's values are what V1 ships, and neither can misjudge a plan | ADR-0009, ADR-0027, Phases 9 and 5 |
| Whether queue priority should be able to reorder deferral against the timestamp. HOST-INV-021 says no, on **provenance exactness**: priority is a delivery class and says nothing about a timestamp's accuracy, so it cannot justify keeping one position over another. The clause-23 argument an earlier revision gave is **withdrawn** — it also invalidated compiled-before-ingress ordering. This is what makes the starvation question above sharp, since a saturated quantum then defers by position with no regard for what the priority classes were for | No — the rule is stated and testable either way | ADR-0023, Phase 3 |

## Review status

**One author pass, then one independent review pass, whose findings are corrected above.** The record of what that pass
found is kept here rather than in a commit message, because it is the argument for why the next pass is still required.

The review raised five findings; all five stand, and the fifth contained three separate defects.

| Finding | Severity | Correction |
|---------|----------|------------|
| `maximum_block_size >= Q` refused hosts the render model is built for. ADR-0001 clause 6 primes the output carry precisely so that a callback of `N < Q` can be served, so a device whose largest block is 32 frames was being rejected by a constraint with no purpose | High | HOST-INV-012 rewritten: the carries are still `maximum_block_size + Q`, and there is no lower bound in terms of `Q`. A conformance case with `maximum_block_size < Q` is added |
| The runtime-overflow contract contradicted three field tables. HOST-INV-009 permitted runtime loss only for live bounded queues, while the telemetry ring overwrote by design, a recording take stopped at capacity, and an over-full quantum had no defined behaviour at all — argued at the time from "the four ingress queues hold 3 072 events against a 256-event scratch", **which the external pass later showed to be false**: those are egress rings. The finding stands; its supporting number did not | High | HOST-INV-009 narrowed to *dropping*; HOST-INV-019 (lossy eviction), HOST-INV-020 (session limit) and HOST-INV-021 (per-quantum deferral) added. The failure table now lists five distinct behaviours, and each field carries exactly one |
| `HostCapabilities` could not satisfy HOST-INV-003. The model permits `Offline` and `Harness` sources that declare values, the layout row carried a hardcoded `Stereo`, and no runtime tag can prove a query happened — so the conformance test as written was impossible | Medium | The invariant is scoped to the device path and enforced by API shape: `::from_device` with no defaulted parameter, `::declared` for the rest, setting the tag itself. The layout default is removed. The test becomes one shape assertion plus a `LIMIT-0057` regression |
| `max_script_instructions_per_quantum` was a `RenderLimits` field with the value *unset*, contradicting this specification's promise of a value for every field | Medium | The aggregate moves to the `ResourceReport` as a reported quantity with no threshold. It becomes a profile field when Phase 7 can justify a number |
| Provenance and counts were reported inconsistently: `prepared_immutable_bytes` was labelled *derived* in its table and *anchored, not derived* two lines below; 512 voices had no rule that produces 512; and eleven no-antecedent fields were summarised as ten, with three of them naming V1 entries in their own `Replaces` column | Medium | The basis vocabulary gains *chosen, anchored on*. Exactly one field is now *derived* from measurement, two are anchored, and the no-antecedent list is seven — corrected here, in the phase tracker, and in `STATUS.md` |

**One further defect was found while correcting these**, which is the pattern this project has recorded three times
already — the pass that reviews a correction finds something in the correction. `max_held_notes` was set to 128 on the
reasoning that a held note cannot outnumber a voice. It can: a sustain pedal, a stealing allocator, and an MPE or
sequencer source all hold notes that are not sounding, and the allocator tracks a held note precisely so that it can
re-sound one. The field is now `max_active_voices`, engine-wide, with its own `HeldNoteCount` type so that the two
concepts cannot be assigned to each other.

**The bounded closure pass over those corrections has now run, and it found four defects — one substantive.** That is
the fourth consecutive time in this phase that a pass over a correction has found something in the correction, which is
now less a warning than a measured property of this work.

| Finding | Correction |
|---------|------------|
| **HOST-INV-021 reused ADR-0001 clause 16's counter and its position rule, and neither fits.** A deferred event is not late — its producer was on time and the *engine* was full — so counting it as late would publish a capacity shortfall as an external timing fault. Clause 16's rule is also circular here: "the first not-yet-rendered quantum boundary" is the quantum that just failed to admit the event | Deferral gets its own capacity-deferral counter and an explicit position rule (the boundary of the quantum *after* the one that could not admit it, re-evaluated there). ADR-0032 clause 22 is cited as the precedent — it separates the pre-epoch clamp from the late counter for the same reason and warns that one test would pass on the wrong policy. The conformance row now asserts both counters in both directions |
| **`forward_event_horizon`'s default could fail its own validation.** `maximum_block_size` is queried with no compiled-in ceiling, so a device reporting a block longer than a second's worth of frames yields a horizon below HOST-INV-013's floor — a profile refused at construction on a device the specification otherwise admits | The default is `max(one second, maximum_block_size + Q)`. The case is removed rather than left to nobody meeting it, which is what made it invisible to testing |
| **`max_concurrent_retiring_voices` = 64 recreated the defect the previous pass had just fixed.** How many voices retire at once is a runtime quantity, so with 512 sounding and a plan swap arriving, more than 64 would want to retire and nothing said what happens. There is also no good answer to invent: a voice cannot be refused retirement, and stopping the excess uncrossfaded is an audible degradation HOST-INV-019 forbids | Derived from `max_active_voices` so it cannot bind, while still accounting its crossfade buffers in the memory budget. ADR-0009 may lower it, but only together with a defined behaviour for reaching it |
| **HOST-INV-009 claimed every profile field falls under one of four runtime behaviours**, which was false twice: it omitted admission refusal, the behaviour most fields take, and it had no place for a field that is a *size* rather than a bound — `analyzer_fft_size`, `telemetry_ring_frames`, `retirement_crossfade`, the rate range, and `channel_layout` cannot be exceeded at all | Five behaviours plus a named sizing category, stated in both places, with the sizing fields enumerated |

Two smaller gaps closed with them: `max_note_expansion_per_tick` had a compile-time rule and a runtime hole, since a
script-driven note graph's expansion is not statically knowable — HOST-INV-021 now carries that case explicitly; and
`::declared`'s `DeclaredSource` was a dangling type name in a normative interface, now declared, with `Device` absent by
construction so a declared profile cannot claim to have queried a device.

**A confirmation read of those three changes has now run, and found two more.** Neither is in the three changes
themselves, which is worth noting — both are things the closure pass's corrections made visible rather than caused.

| Finding | Correction |
|---------|------------|
| **The two counters were implied to partition the cases, and they do not.** An event that arrives late is clamped forward and raises the late counter; if the quantum it lands in is full it is then deferred and raises the deferral counter as well. Both firings are correct, but a diagnostics consumer that adds them to count affected events overcounts | HOST-INV-021 now states that the counters count *causes*, not events, and that one event may raise both. The conformance row gains a third case asserting each counter rises exactly once for an event that is both |
| **`max_scheduled_events_in_flight` had no defined behaviour on reaching it** — the third field in this specification enforced at runtime with none, after the recording capacities and the retirement budget | It is self-limiting: the scheduler owns its release rate, so reaching the window delays a release rather than failing one, and a passage with more events due than the window holds degrades into ordinary lateness under ADR-0001 clause 16. Written down, with the recurring shape named: a field whose bound is a runtime quantity needs its behaviour stated even when the answer is "it cannot bind" |

The read also sharpened, without resolving, the one question the closure pass left open. Compiled events take precedence
**unconditionally**, so a plan that saturates `max_events_per_quantum` every quantum defers ingress forever. The likely
fix is not a starvation bound but a *reserved ingress allowance* the scheduler leaves free under ADR-0032 clause 27 —
which is new design, and Phase 3's.

**The independent read the confirmation pass asked for has now run, and it found seven — two of them High, and the
first is in the confirmation read's own correction.** That is the fifth consecutive pass over a correction to find
something in the correction.

| Finding | Severity | Correction |
|---------|----------|------------|
| **`max_scheduled_events_in_flight`'s new failure behaviour recreated the defect the closure pass had just fixed.** It said an event the scheduler releases late is "counted by ADR-0001 clause 16 like any other late event" — but the producer there is the *scheduler*, and the bound it hit is a profile capacity, so charging it to a counter that answers *how often was a producer late* is precisely the merge HOST-INV-021 forbids two sections earlier. The field also appeared in neither the failure table nor the sizing list, though HOST-INV-009 claims that partition covers every field | High | The delayed release raises the **capacity-deferral** counter. Clause 16's *position* rule does transfer here, unlike in HOST-INV-021's case, because the quantum has genuinely rendered — cause and mechanism are separable, and this field takes the mechanism without the attribution. A failure row is added, restoring HOST-INV-009's partition |
| **HOST-INV-021 said *that* the excess defers but not *which* events are the excess.** No rule among ingress events, though four priority classes exist and the starvation question presupposes an ordering; and no rule at all for compiled events overrunning a quantum alone, which `max_note_expansion_per_tick`'s data-dependent case makes reachable. An implementation deferring by arrival order would reorder two ingress events against their own timestamps | High | An explicit admission order: compiled before ingress, then ascending `SampleTime`, ties to ADR-0023. Priority deliberately does *not* reorder against the timestamp, because deferring an earlier-stamped event past a later one to honour priority is ADR-0032 clause 23's forbidden perturbation. Two conformance cases added, one per direction, and the priority choice is recorded as an unresolved question rather than left implicit |
| **Deferring to the quantum *boundary* moved a sample-positioned event off its declared sample**, and the text excused it as "the same delay ADR-0001 clause 14 already charges". Clause 14 charges the *control-rate* response and leaves note and gate timing untouched — its consequences say so in those words. The boundary rule also collapses every deferred event onto offset 0, manufacturing same-`SampleTime` collisions for ADR-0023 out of a capacity shortfall | Medium | Deferral is `+Q` frames: same offset in the following quantum. Spacing inside a burst survives, no collisions are invented, and the delay is exactly the "up to `Q` frames" the specification already claimed. The remaining shift is now stated as a real degradation rather than as a re-charge of an existing one |
| **HOST-INV-005's admission rule did not admit six of this specification's own fields.** It required a `HostProfile`-owned ledger entry or an accepted ADR; `max_held_notes` has ledger owner `N/A — removed`, and five of the seven no-antecedent fields are created here. Its own conformance row already assumed a third ground | Medium | Three grounds, named, with the third — this specification, with a stated basis and a revisit point — carrying the six. The conformance test fails a field matching none |
| **HOST-INV-018 was unsatisfiable for two fields.** It demanded a private field and a fallible constructor of *every* profile field; `channel_layout` and `source` are enums with neither. This is the HOST-INV-003 shape the independent pass already found once: an invariant whose named test cannot pass | Medium | Scoped to **quantity** fields, with the two **kind** fields called out as closed enums — an enum admits no invalid value, so there is nothing to make fallible. The test enumerates both sets, so a new field must be classified rather than escape the check |
| **HOST-INV-011 was contradicted by two fields in the tables.** It forbids stating a duration-valued budget in frames, while `retirement_crossfade` (128) and `telemetry_ring_frames` (4 096) are flat frame counts meaning durations — both shrinking 4.4x from 44.1 to 192 kHz, the crossfade audibly | Medium | The invariant is scoped to budgets, since neither sizing field can misjudge a plan, and the two are named as exceptions rather than left to contradict it. Whether they *should* be stated in seconds is recorded as an unresolved question for ADR-0009 and ADR-0027 |
| **`max_held_notes` was coupled to `max_active_voices` in prose and nowhere else.** Its basis is *chosen* with the literal 512, and HOST-INV-018 keeps the two types unconvertible, so nothing would fail if Phase 6 raised the voice count and left this one behind. The opposite arrangement from `max_concurrent_retiring_voices`, which is derived and shares its operand's type for exactly that reason | Low | The table says "the same number as `max_active_voices` but not derived from it — the types do not convert", and the drift is handed to Phase 6 as a choice rather than left as a comment |

**The targeted pass over those two rules has run, and found six — three High.** It was scoped to the deferral
admission order and the `+Q` position rule, and every finding fell inside that scope, which is the first time a pass
here has been contained by its own targeting.

| Finding | Severity | Correction |
|---------|----------|------------|
| **`+Q` never said whether it rewrites the event's `SampleTime` or only which quantum renders it**, and the two differ observably. A rewritten stamp makes per-event displacement unreportable, so the one quantity that would show starvation disappears; it makes ADR-0032 clause 7's resolution of a take to musical time **quantize a played performance forward** by however long the engine was overloaded, which is the authored-data change ADR-0021 part 2 forbids outright; and it is the harm ADR-0032 clause 23 names — "would make the sample position a lie" — arriving from capacity instead of precedence, which that prohibition's wording does not cover but its reason does | High | The envelope's `time` is immutable after stamping. Deferral advances a **derived render position**, `time + Q x deferrals`, and the deferral count travels with the event. The diagnostics report carries displacement as well as occurrence. Two conformance cases added |
| **The reason given for subordinating queue priority proves too much.** "Deferring an earlier-stamped event past a later-stamped one to honour priority is clause 23's forbidden perturbation" — but **rule 1 does exactly that**, every time a compiled event positioned late in a quantum is admitted while an ingress event positioned early in it defers, which is the ordinary case rather than a corner. The argument kills both cross-position reorderings or neither | High | Rule 1's basis is restated as **provenance exactness**: a compiled position is exact by construction (ADR-0032 clause 18), an `Arrival` one carries declared-unmeasured error (clause 19). Priority stays subordinate on a reason that survives it — priority is a *delivery* class, and two events from one adapter carry identical uncertainty whichever ring they took. The basis exposes a tier the specification does not have, `Hardware` before `Arrival`, recorded as an open question for ADR-0022 |
| **Preserving the offset creates a second starvation channel that nothing recorded.** The admission order has no age term, so an event positioned late in its quantum keeps that offset through every deferral and loses rule 2 to natively-due events positioned early in theirs — every round, indefinitely. That starves ingress *against other ingress*, independent of the compiled-precedence channel, and the two need different fixes | High | Both channels stated, in the invariant and in the unresolved-questions table, with their different fixes named — a reserved allowance for the first, an age term for the second — and both routed to Phase 3. Nothing is invented here: an age term is itself a cross-position reordering and needs the basis argument the finding above just repaired |
| **A repeatedly deferred ingress event drifting toward `forward_event_horizon` would be *rejected*** under an implementation that re-checks the horizon against the render position, turning "deferred, not dropped" into a drop after enough overload. ADR-0032 clause 21 scopes the horizon to what an external producer may *enqueue*, but HOST-INV-013 never said the check happens once | Medium | HOST-INV-013 states it: evaluated exactly once, at ingress admission, against the timestamp as stamped, and nothing after admission re-triggers it. A conformance case drives an event past a horizon's worth of deferrals and asserts it still renders |
| **The admission order was stated without the precondition that makes it implementable.** As a comparison sort it is `O(n log n)` over a candidate set reaching the four rings' 3 072 events plus the release window's 4 096, inside a callback — a cost the real-time section's list of runtime-variable quantities does not account for | Medium | The rings are **position-monotone by contract**, with a counted ingress fault for an out-of-order enqueue, so admission is a five-way merge of sorted streams: `O(n log 5)`, no allocation, five cursors. Stated in HOST-INV-021, in the failure table, and in the real-time section |
| **Rule 3's interim tie-break left same-position events from one ring undecided** — priority separates rings, not events within a ring. The "iteration order decides" shape at a finer grain than the previous pass fixed | Low | Arrival order within one ring, which the monotonicity contract above already supplies |

**An external review pass then ran — the first not conducted by an author of this document — and found five, three of
them P1.** Three are in material the two passes above wrote, and one had stood since the first independent pass.

| Finding | Severity | Correction |
|---------|----------|------------|
| **The document has been reasoning from the wrong queues since the first independent pass.** "The four ingress queues hold 3 072 events between them" names `LIMIT-0013`'s prioritized rings, which carry `EngineEvent` **out** of the engine toward the GUI — the architecture's `EngineCommand` (in) / `EngineEvent` (out) split — and which V1 does not wire to anything at all. They are not renderer ingress, so they could never bound what one quantum is presented with, and they cannot be inputs to an admission merge. The claim entered as motivation for HOST-INV-021 and survived four passes | P1 | The numeric argument is withdrawn, not repaired. HOST-INV-021 rests on the two sources that actually produce an overrun — live ingress, unbounded in time by definition, and data-dependent note expansion. `event_queue_capacity`'s row now says it is an egress capacity. **V1 has no timestamped renderer-ingress queue to carry over at all**, so defining V2's ingress streams and their capacities is recorded as a Phase 3 blocker |
| **`+Q` deferral destroys the ordering the five-way merge required**, even granting the monotone-enqueue precondition the previous pass added. If a ring holds `A` at the end of quantum `k` followed by `B` at the start of `k+1`, deferring `A` puts its render position after `B`'s while `B` remains behind `A` in the FIFO, and a head-merge cannot reach `B` | P1 | The merge is withdrawn as normative, along with the monotone-enqueue contract and its failure row. What remains is the **constraint** — admission may not allocate and may not comparison-sort a runtime-sized set — stated without prescribing the mechanism. Deferred events need their own preallocated bounded stream, which is a capacity this profile has no field for; that and the ingress streams go to Phase 3 together |
| **Suppressing ADR-0001 clause 16's late counter for a delayed release overrode an accepted decision.** Clause 16 triggers on a *condition* — an event whose timestamp falls in an already-rendered quantum — not on a cause, and here the condition genuinely holds. A specification may not narrow an accepted decision by glossing its counter as "how often was a producer late" | P1 | Both counters rise: late, because clause 16 applies in full, and capacity-deferral, as the attribution the confirmation read was missing. The gloss is corrected everywhere — clause 16's is a **condition** counter, the deferral counter is a **cause** counter. HOST-INV-021's own exclusion of the late counter is re-argued on the condition (that quantum has not rendered), which is stronger than the cause argument it replaced and does not narrow ADR-0001 |
| **HOST-INV-005's three grounds were neither disjoint nor exhaustive.** `forward_event_horizon` matched two — ADR-0032 clause 21 creates it *and* it is on the no-antecedent list, which the invariant had defined ground 3 as — while `max_held_notes` matched none | P2 | The grounds are applied in a stated order so every field takes exactly one, ground 1 is widened to "a ledger entry whose disposition creates a profile capacity" so `LIMIT-0031` carries `max_held_notes`, and **"no V1 antecedent" is named as a different axis** that does not select a ground |
| **A non-monotone enqueue was given a counter but no disposition.** Keeping the event breaks the sorted-stream precondition; rejecting it is a runtime behaviour absent from the five-behaviour taxonomy | P2 | Moot with the merge withdrawn: the precondition and its failure row are gone. Phase 3 states the disposition when it defines the streams |

**A second external pass over those corrections found five more, one P1**, and **a third found six, two P1** — the
seventh and eighth consecutive times a pass over a correction has found something in the correction.

| Finding | Severity | Correction |
|---------|----------|------------|
| **The immutable stamp and ADR-0001 clause 16 cannot both be implemented as written.** Once the quantum a deferred event could not enter has rendered, the event's preserved timestamp *does* fall in an already-rendered quantum — clause 16's condition, which this document now insists fires regardless of cause — while HOST-INV-021 forbids the late counter there. The previous correction fixed the *cause* confusion and created a *timing* one | P1 | Stated as a conflict rather than resolved by fiat. The interim rule is that clause 16's condition is asked **once, when an event first becomes due**, and deferral does not re-ask it. Because that narrows an accepted decision, it is registered as an unresolved question needing an **ADR-0001 clarification or successor before Phase 3**, not settled here |
| **The producer-based gloss survived in two places after the invariant was fixed** — the `max_events_per_quantum` failure row and the Events prose both still said the late counter "describes a producer that was late", which is the reading the external pass had just rejected | P2 | Both restated on clause 16's condition. A correction applied to an invariant and not to the prose that restates it is how this document has repeatedly kept a defect alive in a second location |
| **HOST-INV-005's conformance row tested different grounds than the invariant defines.** It enumerated `HostProfile`-owned entries and the no-antecedent list, so it would have rejected `max_held_notes` and double-matched `forward_event_horizon` — the exact two failures the invariant had just been rewritten to prevent | P2 | The row enumerates the three grounds in order, asserts each field matches exactly one, and says explicitly that the no-antecedent list is not one of them |
| **The withdrawn clause-23 rationale survived in the unresolved-questions table**, where it would have told Phase 3 to revisit priority against an argument this specification had already refuted | P2 | Restated on provenance exactness, with the withdrawal named |
| **The central V1 claim carried no `file:line` citations**, violating [the rule](../decisions/README.md#every-claim-about-current-behaviour-carries-a-fileline-citation) this repository added precisely because an uncited behaviour claim and an assumed one look identical in prose — and this was a claim correcting five passes' worth of an assumed one | P2 | Cited at `3555c52c`: the module summary, `TimestampedEvent`'s `EngineEvent` payload, the four buffer sizes, and the fact that `lib.rs:58-59` is the only reference to the channel outside its own module, which is what supports "unwired" |

The third external pass found two further P1s, both consequences of the queue-direction discovery that the previous
correction had propagated only partway, plus four consistency defects introduced by the corrections themselves.

| Finding | Severity | Correction |
|---------|----------|------------|
| **The bounded deferred store had no exhaustion policy**, so HOST-INV-021's "no event is lost" could not coexist with the audio thread's prohibition on allocating | P1 | Recorded as a blocker. A first attempt to derive a safe size — ingress capacity plus the release window — was itself refuted by the next pass and is documented as wrong: deferring *frees* the upstream slot, so those capacities bound arrival rate rather than backlog, and `max_note_expansion_per_tick` multiplies one released event into many. **No bound is derivable from the current field set**, so as written "no event is lost" holds only while the store has room |
| **`LIMIT-0013`'s rings kept the `Live bounded queue` class after being identified as egress.** ADR-0021 reserves that class for queues "fed by external, unbounded-in-time input"; these are fed by the engine, so the class was assigned under the same misreading this document made. Left as it was, HOST-INV-009's dropping licence reads as covering engine state and diagnostics | P1 | The conflict is recorded, not resolved: correcting an accepted decision is not this specification's to do, so it becomes an **ADR-0021 clarification or successor**, alongside the clause-16 one. The behaviour ADR-0021 chose stays defensible; the class does not describe it |
| **The forward horizon was still sized against an egress capacity** — its pinned-slot cost was `event_queue_capacity` times the horizon | P2 | Withdrawn. The quantity is the ingress capacity times the horizon, and that capacity is Phase 3's to define |
| **`STATUS.md` asserted the falsified 3 072-event premise and its own refutation in the same section** | P2 | The earlier claim is qualified in place, as a current-state dashboard requires |
| **Three documents disagreed on the pass count** after the seventh was recorded — seven, eight, one external, two | P2 | Eight total, six author, two external, stated the same way in all three |
| **The closure checklists omitted the ADR-0001 conflict** that the same documents had just called a blocker | P2 | All three now list the same three blockers |

A fourth external pass found four more, two P1, both in the third's corrections: the deferred-store derivation above
did not bound what it claimed to, and rule 3's interim tie-break named ingress rings that this document had just
established do not exist. Both are corrected — the first by withdrawing the derivation and stating the blocker plainly,
the second by breaking ties on arrival order into the renderer, which is stream-independent. The other two were the
pass count and a closure list that had lost the ADR-0021 item again.

A fifth external pass found the other half of the same premise: **`max_events_per_quantum` = 256 is not a V1
carry-over either.** `EVENT_BUFFER_SIZE` is an egress ring size, and V1's actual per-block sequencer buffer is an
uncapped `Vec::with_capacity(128)`. `LIMIT-0014`'s ledger description is wrong, which makes this a **P00A-T004 finding
against a task marked `Complete`** — the entry was recorded from a constant's name by two discovery passes, and the
class pass did not reach its use sites. The field is now *chosen*, and the no-antecedent count rises to eight.

**Status after eleven passes.** Author, independent (five findings, two High), bounded closure (four, one substantive),
confirmation (two), independent (seven, two High), targeted (six, three High), and five external passes (five with
three P1, five with one P1, six with two P1, four with two P1, and a fifth that found the `LIMIT-0014` misreading). Every finding is corrected here.

**On when to stop.** Each pass has found something, and the criterion used is **a pass that changes no contract
clause**. The external pass does not meet it: it withdrew a normative merge, re-argued a counter exclusion, and
rewrote HOST-INV-005.

**The sixth pass recommended closing P00A-T005. That recommendation is withdrawn.** It rested on two claims, and the
external pass falsified both. It said the sixth pass's findings were *contained by its targets* — but the containment
was an artefact of the targeting, and a reader who was not the author found a factual error sitting outside those
targets that had stood through four passes. And it said the additions were *preconditions for rules that already
existed* rather than new mechanism — but one of those preconditions was insufficient for the mechanism it guarded,
and the mechanism itself was built on queues that run the wrong way.

**The lesson is about who reviews, not how many times.** Six of the eleven passes were conducted by an author of the
document; the five that were not found the two highest-severity errors in the file — both long-standing misreadings of
V1 — by checking numbers against the source instead of against the arguments they supported. The pattern this project has recorded — that a
correction is new material — now has a companion: **an author's pass re-reads the reasoning, and a number that supports
the conclusion you already hold does not get audited.**

**What P00A-T005 needed before it could close** was not another read of the same kind. **Four** items were
substantive rather than editorial: (1) **an ADR-0001 clarification or successor** answering *both* whether a quantum may defer at all under clauses 12
and 14, and when clause 16's condition is evaluated — resolving only the second would leave deferral unauthorised; (2) **an ADR-0021 one** on the class
`LIMIT-0013`'s engine-egress rings were given; (3) **`LIMIT-0014`'s GUI/OSC split**, without which
`event_egress_capacity` leaves HOST-INV-005 unsatisfied; and (4) **V2's renderer-ingress streams, plus a separate bound
or exhaustion policy for the deferred store**, without which HOST-INV-021's "no event is lost" rests on a store with no
capacity.

**[REV-P00A](../reviews/phase-00a-exit-review.md) made the scoping call, and it did not split them
two-and-two the way this section anticipated.** Items 2 and 3 could *not* move to Phase 3: both are bound by the Phase
0A exit gate's resource-inventory clause, which requires every fixed cap to carry a proposed V2 rule and a
user-visible overflow diagnostic, and four ledger rows sat outside `Classified` on exactly those two questions. They
are settled in Phase 0A by [ADR-0038](../decisions/ADR-0038-engine-egress-queue-classification.md), which supplies the
engine-egress rule and performs the split. Items 1 and 4 are Phase 3's, recorded in the master plan's Phase 3 work list
and entry gate — the plan being authoritative for scope and phase order, a note here would not have moved them.

**What made the narrow close defensible is the gate text, not the phrase "Phase 1 compiles rather than renders live"**,
which is imprecise: Phase 1 does render, offline, and its exit gate requires deterministic rendering and an
allocation-free loop. The gate contains no bullet requiring this specification to reach any status; what binds
P00A-T005 is the phase's Work list, which asks for an *initial* contract over a named field list — and renderer-ingress
capacity and the deferred store are not in it. See [*Deferred to Phase 3*](#deferred-to-phase-3) for what that leaves,
including the prevalidated-bounded-span rule Phase 1's event boundary needs in the mechanism's absence.

**The five external passes have settled what kind of review this document responds to.** Six author passes each found
something and each left something. The external ones found a false number that five reads had accepted, then a rule
conflict created by that correction, then a half-propagated fix, then a capacity derivation that did not bound what it
claimed, and finally a second long-standing misreading — `LIMIT-0014` — that no author pass had questioned. **They also
caught two false findings in the audit convened to fix the first one**, both made the same way: an absence asserted
from a name search, and a behaviour asserted from one line without reading what feeds it. None was subtle once looked
for.

**That chain is the real result of this task, and it is not a review-process observation.** Five consecutive external
passes have each landed on the same object: HOST-INV-021. Deferral cannot be specified without knowing what feeds the
renderer and how much of it there can be, and **every V1 number the invariant was built on has now been falsified** —
the 3 072-event ingress premise, the 256-event scratch, and the derived deferred-store bound. Two of the three were
ledger entries recorded from constant names rather than use sites, which puts the last finding inside P00A-T004 rather
than P00A-T005. Every attempt to complete the invariant from the fields that exist has produced a claim the next reader
falsified. **The gap is in the contract, not in the reviewing** — and the ledger needs a use-site audit for the entries
Phase 3 depends on. The
operative lesson is not that more passes are needed but that **an author re-reads the reasoning while an external
reader checks the claims**, and only the second kind has caught this document's factual and cross-record errors.

The standing checks from the previous pass remain, and none was disturbed by these corrections:

- that no default is sized by `Q`'s provisional value, directly or through arithmetic;
- that every one of the ledger's 28 `HostProfile` entries has a successor field, and that no field exists without an
  entry or a stated no-antecedent reason;
- that the one derived and two anchored EVD-0003 figures use the evidence as the evidence states it — cost, not
  capacity, and RSS as an upper bound rather than as prepared bytes;
- that no clause here contradicts ADR-0021 part 1's two axes, ADR-0001 clauses 1, 5, 6 and 16, or ADR-0032 clauses 21,
  22 and 23;
- that the `HostCapabilities`/`RenderLimits` split refines ADR-0021 part 4 rather than replacing it;
- that answering the master plan's "parameter and control slots" and its script-work aggregate through the report,
  rather than as budgets of their own, is right — and if it is, that the plan's own field list is corrected when this
  specification becomes `Current`, per the [documentation authority rule](../README.md#sources-of-truth).
