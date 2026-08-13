# SPEC: Host Profile and Render Limits

| Field            | Value                                  |
|------------------|----------------------------------------|
| Status           | Draft                                  |
| Phase            | 00A                                    |
| Created          | 2026-08-13                             |
| Last reviewed    | 2026-08-13                             |
| Based on         | ADR-0021, ADR-0001, ADR-0032, ADR-0037 |
| Invariant prefix | HOST                                   |
| Supersedes       | —                                      |
| Superseded by    | —                                      |

Allowed status values are defined in [README.md](README.md).
Only a `Current` specification constrains implementation.

**Why this is `Draft` and what makes it `Current`.** Every clause below follows from an accepted decision, and the field
set is complete against the master plan's list. **Three passes have run** — an author pass, an independent pass (five
findings, two High), and a bounded closure pass over those corrections (four findings, one substantive) — and every
finding is corrected here. [*Review status*](#review-status) keeps the record of what each pass found and what
correcting it changed; that record is the argument for the remaining step, not decoration.

**A fourth pass — the confirmation read of the closure pass's own three changes — has now run and found two more**, and
they are corrected too. What remains is one **independent** read. Of the four passes so far only one was independent,
and every pass has found something; the criterion for stopping is therefore not "a pass finds nothing" but "a pass that
changes no contract clause", which this one does not meet. See [*Review status*](#review-status).

Fields owned by decisions that are still `Proposed` — ADR-0002, ADR-0009, ADR-0024, ADR-0027, and ADR-0034 — are marked
in the field tables and listed under [*Unresolved questions*](#unresolved-questions). None of them blocks Phase 1. The
specification becomes `Current` when the closure pass has run and P00A-T005 is closed.

## Scope

This specification defines `HostProfile`: the single immutable preparation input against which a Sound Core V2 render
plan is admitted. It fixes

- the profile's field set, its internal split, and the type of every field;
- the default value of every field, the basis for that value, and where it is revisited;
- who may set each field and who may never raise it;
- what compilation reports, and what happens when a plan does not fit;
- which V1 limits each field replaces, so that the [resource ledger](../inventories/resource-limits.md)'s 30
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
- **Limits owned elsewhere.** 44 of the ledger's 74 entries belong to a node contract, a domain/format contract, job
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
5. **HOST-INV-005** — The profile carries exactly the fields listed in [*The field set*](#the-field-set). Adding a field
   requires either a resource-ledger entry whose configuration owner is `HostProfile` or an accepted ADR that creates
   one. A capacity that belongs to another owner may not be smuggled in as a profile field.
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
   halts an activity is HOST-INV-020's, and a quantum that cannot admit every due event is HOST-INV-021's. Together
   with HOST-INV-007's admission refusal these are the five behaviours, and
   [*Failure and diagnostics*](#failure-and-diagnostics) assigns each field exactly one — or records it as a **sizing
   field**, which bounds nothing and therefore cannot be exceeded.
10. **HOST-INV-010** — A `HostProfile` is never persisted in a project document, patch, or bundle. It describes the
    machine and the operator's budgets, not the work; a project that rendered on one profile must load on another.
11. **HOST-INV-011** — A budget whose meaning is a duration is evaluated in seconds at the prepared sample rate, never
    in frames. A block carries the same work at every sample rate while its real-time budget shrinks with the rate
    ([EVD-0003](../evidence/phase-00a/EVD-0003-cpu-memory-timing-baseline.md)), so a policy stated in frames reads a
    192 kHz plan as no more expensive than a 44.1 kHz one when it is more than four times as expensive per second.
12. **HOST-INV-012** — Both carries are sized `maximum_block_size + Q` frames and preallocated at preparation (ADR-0001
    clause 5). **`maximum_block_size` has no lower bound in terms of `Q`.** A host whose largest block is smaller than
    one quantum is supported, and so is any individual callback of `N < Q`: ADR-0001 clause 6 primes the output carry
    with `Q` frames of silence at stream start precisely so that clause 5's loop can serve any `N` without rendering a
    quantum whose input has not arrived. A profile requiring `maximum_block_size >= Q` would refuse a host the render
    model was built for.
13. **HOST-INV-013** — `forward_event_horizon >= maximum_block_size + Q`, and it binds only events whose provenance is
    `Hardware` or `Arrival` (ADR-0032 clause 21). It never measures the scheduler's own releases of compiled events, and
    there is no backward horizon.
14. **HOST-INV-014** — Memory budgets are checked against the compiler's computed aggregate over prepared nodes, not
    against a process-level measurement. V1 computes no such aggregate anywhere (`LIMIT-0073`); producing one is part of
    what admission is.
15. **HOST-INV-015** — An advisory budget never refuses a plan. It emits a `CompileWarning` carrying the predicted and
    permitted values, and compilation continues.
16. **HOST-INV-016** — A profile whose fields are mutually inconsistent fails validation at construction, before any
    plan is compiled, naming the two fields that disagree. Construction is fallible; there is no partially valid
    profile and no clamping constructor.
17. **HOST-INV-017** — One capacity is declared once. Where V1 held one capacity in two constants kept in step by a
    compile-time assertion (`LIMIT-0023` and `LIMIT-0041`), the profile declares one field and both consumers read it.
18. **HOST-INV-018** — Every profile field is typed by a domain newtype with a private field and a fallible
    constructor. No profile field is a bare `usize`, `u32`, or `f32`, and no two fields whose units differ share a type.
19. **HOST-INV-019** — A field marked *lossy* bounds non-authoritative retention or presentation data and evicts by
    design rather than failing. It is ADR-0021 part 1's `Lossy retention/presentation budget` class, and the class's
    condition binds here: the owner exposes an evicted or omitted count, a continuation marker, or an equivalent
    user-visible way to tell a complete view from a trimmed one. A lossy field may never bound canonical project data,
    authored topology, render input, automation, routing, sample mapping, or polyphony.
20. **HOST-INV-020** — A field marked *session limit* is enforced while an activity runs rather than at admission,
    because the quantity it bounds is not knowable when the plan is compiled. Reaching it **stops the activity with a
    counted diagnostic and keeps everything already produced**; it never drops, trims, or overwrites authored data. The
    recording capacities are the only session limits in this profile.
21. **HOST-INV-021** — Where more events are due in one quantum than `max_events_per_quantum` admits, the excess is
    **deferred, not dropped**. Deferral moves an event to the boundary of the quantum *after* the one that could not
    admit it, where it is re-evaluated and may defer again. It is counted under its own
    **capacity-deferral counter**, and it **must not fire ADR-0001 clause 16's late counter**: nothing about the
    producer was late, and merging the two would report an engine capacity shortfall as an external timing fault. This
    is ADR-0032 clause 22's rule applied to a second case — that record separates the pre-epoch clamp from the late
    counter for exactly this reason, and states that a single test would pass on the wrong policy.

    Clause 16 is not the mechanism here, only the precedent that an event may be moved forward rather than dropped. Its
    own rule — "clamped to the first not-yet-rendered quantum boundary" — is circular for this case, because the
    quantum that cannot admit the event has not been rendered yet either, so applying it literally would place the
    event back where it did not fit.

    Compiled events have precedence over ingress events within a quantum, since the scheduler released them against a
    capacity it knows (ADR-0032 clause 27). Dropping remains possible only at the live bounded queue under
    HOST-INV-009, which is the one place a drop is counted and the one place an external producer can outrun the
    engine.

    **This does not breach ADR-0032 clause 23**, which forbids *ADR-0023* from perturbing a timestamp to encode
    precedence. Deferral moves an event because a bounded resource is full, which ADR-0001 clause 16 already
    establishes as legitimate, and it never reorders two events to express priority. A reader arriving from clause 23
    should find that stated rather than have to derive it.

    **The two counters count causes, not events, and one event may raise both.** An event that arrives late is clamped
    forward under clause 16 and raises the late counter; if the quantum it is clamped into is itself full, it is then
    deferred and raises the deferral counter as well. That is correct — two distinct things happened to it — but it
    means the counters may not be added to obtain a number of affected events, and a diagnostics consumer that sums
    them overcounts. Each counter answers one question: *how often was a producer late*, and *how often was the engine
    full*.

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
| `max_held_notes` | `HeldNoteCount` | 512 | Chosen: equal to `max_active_voices`, **not** to the per-instrument ceiling — see below | `LIMIT-0031` | Phase 6 |
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
can re-sound when a voice frees. The count is now `max_active_voices`, engine-wide, and carries its own type so the two
concepts cannot be assigned to each other. Whether even that is enough is Phase 6's, with the allocator in front of it.

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
| `max_events_per_quantum` | `EventCount` | 256 | V1 carry-over (the engine event scratch) | `LIMIT-0014` | Phase 3 |
| `max_note_expansion_per_tick` | `EventCount` | 128 | V1 carry-over. Admitted against `max_events_per_quantum` where the expansion is statically knowable, and covered by HOST-INV-021 where it is not — see below. Part 2 forbids trimming an expansion to fit | `LIMIT-0043` | Phase 3 |
| `max_scheduled_events_in_flight` | `EventCount` | 4 096 | Chosen. Bounds the scheduler's release window under ADR-0032 clause 27; **self-limiting** — see below. V1 has no antecedent because it has no scheduler | — | Phase 3 |
| `forward_event_horizon` | `FrameCount` | `max(one second at the prepared rate, maximum_block_size + Q)` | Chosen, with a derived floor — see below | — | Phase 3 entry (ADR-0022) |
| `command_queue_capacity` | `EventCount` | 16 384 | V1 carry-over. **Live bounded queue** | `LIMIT-0012` | Phase 9 |
| `event_queue_capacity` (critical / high / normal / low) | `EventCount` x 4 | 256 / 256 / 512 / 2 048 | V1 carry-over. **Live bounded queue** | `LIMIT-0013` | Phase 3 |
| return scratch capacity | `EventCount` | 256 | V1 carry-over; admitted against the mixing budget | `LIMIT-0015` | Phase 8 |

**The release window is self-limiting, and the confirmation read is why that is written down.** Nothing exceeds
`max_scheduled_events_in_flight` from outside: the scheduler owns its own release rate and releases at most that many
compiled events at a time. Reaching it therefore delays a release rather than failing one, and nothing is lost — the
events are still in the plan. What it degrades into, when a passage genuinely has more events due than the window
holds, is lateness: the scheduler falls behind, and the events it releases late are counted by ADR-0001 clause 16 like
any other late event. That is the field's failure behaviour, and the first draft of this section stated none — the
third field in this specification to be enforced at runtime with no defined behaviour on reaching it, after the
recording capacities and the retirement budget. The shape recurs often enough to be worth naming: **a field whose
bound is a runtime quantity needs its behaviour written even when the answer turns out to be "it cannot bind".**

**More events can be due in one quantum than the scratch admits, and that case has a rule now.** The four ingress
queues hold 3 072 events between them while `max_events_per_quantum` is 256, so the queues alone can present more work
for one quantum than it can take. Review found this undefined, and it is not covered by admission: ADR-0021 part 1's
"a prepared plan may not exceed a `HostProfile` limit at runtime" binds the *plan*, and live ingress is by definition
not part of it.

HOST-INV-021 fixes it: **the excess is deferred, not dropped.** An event that does not fit is moved to the boundary of
the quantum *after* the one that could not admit it, re-evaluated there, and counted under its own capacity-deferral
counter — never applied retroactively and never silently discarded. Compiled events take precedence within a quantum
because the scheduler released them against a capacity it knows (ADR-0032 clause 27); ingress events yield. Dropping
stays where ADR-0021 already puts it, at the live bounded queue, which is the one place an external producer can outrun
the engine and the one place a drop is counted.

**The counter is deliberately not ADR-0001 clause 16's**, and the first draft of this correction had it wrong. Clause
16 counts an event whose *producer* was late; a deferred event arrived on time and the *engine* was full. Merging them
would publish a capacity shortfall as an external timing fault, and ADR-0032 clause 22 already establishes the rule for
this — it separates the pre-epoch clamp from the late counter on the same grounds, and warns that one test would pass
on the wrong policy. Clause 16's position rule does not transfer either: "the first not-yet-rendered quantum boundary"
is circular here, because the quantum that could not admit the event is itself unrendered.

**Note expansion needs both answers for the same reason.** `LIMIT-0043`'s ledger rule refuses an over-expanding tick at
preparation, which works for a deterministic processor whose expansion the compiler can compute. A script-driven note
graph's expansion is data-dependent and not knowable then, so admission bounds what it can and HOST-INV-021 carries the
rest. Without that split the field would have had a compile-time rule and a runtime hole, which is the shape of defect
this section exists to close.

This makes `max_events_per_quantum` a smoothing budget rather than a cliff: a burst is spread over the following
quanta at up to `Q` frames of added delay each, which is the same delay ADR-0001 clause 14 already charges a
mid-quantum event's control response. A sustained overrun is then visible as a rising late count rather than as
silence. Phase 3 owns confirming that the deferral cannot starve a low-priority event indefinitely under sustained
load; it is listed as an unresolved question.

**The forward horizon, and why one second.** ADR-0032 clause 21 makes this one profile field, binding ingress
provenance only, because an event held for an unbounded time pins a queue slot. Three quantities bound the choice from
below: a host delivers a block's events stamped within that block (at most `maximum_block_size`), an adapter may stamp
slightly ahead, and HOST-INV-013 requires at least `maximum_block_size + Q`. Nothing bounds it from above except the
cost of a pinned slot, which is `event_queue_capacity` slots times the horizon.

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
| `telemetry_ring_frames` | `FrameCount` | 4 096 | V1 carry-over. **Lossy** (HOST-INV-019): oldest samples are overwritten by design, and the reader can tell a stale window from a fresh one | `LIMIT-0021` | Phase 5 |
| `analyzer_fft_size` | `FrameCount` | 2 048 | V1 carry-over. A resolution budget; the size travels with the payload | `LIMIT-0022` | Phase 5 |

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
| `modulation_slots_per_voice` | `SlotCount` | 16 | V1 carry-over, **declared once** for both the Mod Matrix and the script host | `LIMIT-0023`, `LIMIT-0041` | Phase 7 |

**One capacity, one field.** `LIMIT-0023` (Mod Matrix slots per voice) and `LIMIT-0041` (script host slots) are the same
capacity seen from two sides, held equal in V1 by a compile-time assertion because letting them diverge would silently
drop the high routings' scripts. HOST-INV-017 makes the assertion unnecessary: one field, two readers.

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

**The ledger's 30 `HostProfile`-owned entries**, each with its successor field:

| Entry | Field |
|-------|-------|
| `LIMIT-0001` | `maximum_block_size` |
| `LIMIT-0002`, `LIMIT-0003` | `buffer_scratch_bytes`; sized from `maximum_block_size` rather than set independently |
| `LIMIT-0004` | Accepted rate range |
| `LIMIT-0012` | `command_queue_capacity` |
| `LIMIT-0013` | `event_queue_capacity` (four priorities) |
| `LIMIT-0014` | `max_events_per_quantum` |
| `LIMIT-0015` | Return scratch capacity |
| `LIMIT-0020`, `LIMIT-0062` | `max_observation_taps` |
| `LIMIT-0021` | `telemetry_ring_frames` |
| `LIMIT-0022` | `analyzer_fft_size` |
| `LIMIT-0023`, `LIMIT-0041` | `modulation_slots_per_voice`, one field |
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

An earlier revision listed eleven, and review found the count wrong in three separate ways. The three memory aggregates
were listed as having no antecedent while their own rows named `LIMIT-0002`, `LIMIT-0003`, and `LIMIT-0073` — they are
new as an *aggregate*, which is `LIMIT-0073`'s finding, but the resources themselves are V1's.
`max_script_instructions_per_quantum` is no longer a field at all. And eleven items were summarised as ten in the phase
tracker and `STATUS.md`, both of which now say seven.

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
| More events are due in one quantum than `max_events_per_quantum` admits | The excess is **deferred** to the following quantum's boundary and counted under its own capacity-deferral counter — never under the late counter, which describes a producer that was late; compiled events take precedence (HOST-INV-021) | Structured diagnostics report |
| A lossy field's capacity is reached | The oldest data is evicted by design, and the evicted count or continuation marker is exposed (HOST-INV-019) | The surface presenting that data |
| A session limit is reached | The activity stops with a counted diagnostic; everything already produced is kept, and nothing authored is dropped (HOST-INV-020) | The recording surface, plus the structured diagnostics report |
| An ingress event is beyond `forward_event_horizon` | Rejected and counted | Structured diagnostics report |
| A callback exceeds `maximum_block_size` | ADR-0021 part 3's terminal stream-contract fault: silence, both carries invalidated, `needs_reprepare` published, nothing allocated | Structured diagnostics report |

**Five behaviours, plus one category that has none.** Admission refuses, a live queue drops, a quantum defers, a lossy
budget evicts, and a session limit stops. The first review pass found an earlier draft claiming compile-time refusal for
every render limit while three fields visibly did something else at runtime — the telemetry ring overwrote, a recording
take stopped, and an over-full quantum did nothing defined at all. The taxonomy above is what reconciles them, and
HOST-INV-009's narrowed wording is what keeps "drop" from covering the other three.

**Sizing fields bound nothing and cannot be exceeded**, so asking which behaviour they take is a category error:
`analyzer_fft_size` sets a resolution, `telemetry_ring_frames` sets a window length (its *eviction* is the lossy
behaviour, not an overflow), `retirement_crossfade` sets a duration, and the accepted rate range and `channel_layout`
describe what a stream *is* rather than how much of it there may be. They cost prepared memory and therefore appear in
the `ResourceReport`; they have no failure row because they have no failure. The closure pass added this paragraph:
HOST-INV-009 had claimed every profile field falls under one of four runtime behaviours, which was false twice over —
it omitted admission refusal, the behaviour most fields actually take, and it had no place for a field that is a size.

Every counter named above reaches the **structured diagnostics report**, which is the report a Phase exit review
inspects. This is the specific control against the failure mode ADR-0021 records twice: `LIMIT-0013`'s drop counters
existed for years, published only over OSC, where no user looked.

## Real-time and resource constraints

- The profile is read only off the audio thread (HOST-INV-002). The renderer holds no reference to it.
- Admission and preparation may allocate; both run off the audio thread.
- Every capacity the audio thread relies on is preallocated at preparation, including both carries, the event scratch,
  the return scratch, the recording buffers, and each node's mutable state.
- Nothing in the render loop consults a limit to decide whether to allocate. A plan that was admitted fits by
  construction; a plan that does not fit was refused.
- The runtime-variable quantities are the live bounded queues' occupancy, the number of events due in one quantum, and
  a take's length. None of the three allocates: the queue drops and counts (HOST-INV-009), the quantum defers and counts
  (HOST-INV-021), and the take stops and counts (HOST-INV-020).

## Conformance tests

No test exists yet: the V2 crate is Phase 1's, and this specification is written before it. Each row names what must
exist, in the phase that builds the thing it tests.

| Invariant | Named test or evidence | Phase |
|-----------|------------------------|-------|
| HOST-INV-001, HOST-INV-002 | A prepared plan renders after its source profile is dropped; the renderer holds no profile reference | 1 |
| HOST-INV-003 | Two tests, because no runtime check can see where a value came from. **Shape:** `HostCapabilities::from_device` has no defaulted parameter and no `Default` impl, so a caller cannot omit a capability. **Behaviour:** the cpal adapter is driven with a device reporting a non-default buffer range and the resulting profile carries that range — the direct regression test for `LIMIT-0057`, which discarded it | 9 |
| HOST-INV-004 | Partly a review check — no automated test can see that a default was *reasoned* from `Q`. The mechanical half is ADR-0032 clause 4's compile-time assertion `Q <= QuantumOffset::MAX`, which fails the build when `Q` changes and something was sized to its old value, plus a test that `HostProfile` exposes no field carrying a quantum | 1 |
| HOST-INV-005 | A test enumerating profile fields against the ledger's `HostProfile`-owned entries plus the listed no-antecedent fields | 1 |
| HOST-INV-006 | Every compile — succeeding and failing — returns a report whose every field has requested, available, and a dominant contributor | 1 |
| HOST-INV-007 | One refusal case per render limit, asserting the error names the field, both amounts, and the authored object; and asserting the plan is unchanged | 1 |
| HOST-INV-008 | A node whose declared capacity is exceeded is refused, and raising every profile field does not admit it | 2 |
| HOST-INV-009 | Each live bounded queue is overrun and its drop count is asserted in the diagnostics report | 3 |
| HOST-INV-010 | A project saved under one profile loads and compiles under another; no serialized field names a profile value | 10D |
| HOST-INV-011 | The same plan is admitted at 44.1 kHz and warned at 192 kHz by the cost budget alone, with no field count changed | 3 |
| HOST-INV-012 | Callback sizes from 1 frame to `maximum_block_size`, including non-multiples of `Q`, render identically to a single large block — the partition-invariance suite of ADR-0001. Plus one profile whose `maximum_block_size` is **below** `Q`, which must be admitted and must render identically | 3 |
| HOST-INV-013 | An ingress event one frame beyond the horizon is rejected and counted; a compiled event hours ahead is released normally | 3 |
| HOST-INV-014 | The compiler's aggregate equals the sum of node-declared prepared bytes for a plan built from known nodes | 2 |
| HOST-INV-015 | A plan over the cost budget compiles and warns; no advisory field can produce a `CompileError` | 1 |
| HOST-INV-016 | A profile with `forward_event_horizon < maximum_block_size + Q` fails construction naming both fields — **and the default profile satisfies it at every admissible `maximum_block_size`**, including one above a second's worth of frames, which is the case the flat one-second default failed | 1 |
| HOST-INV-017 | The Mod Matrix and the script host read one field; the V1 compile-time assertion coupling two constants is gone | 7 |
| HOST-INV-018 | Every profile field's type has a private field and a fallible constructor; no field is a bare primitive, and `HeldNoteCount` does not convert to or from `VoiceCount` | 1 |
| HOST-INV-019 | The telemetry ring is overrun and the reader can distinguish a complete window from an overwritten one | 5 |
| HOST-INV-020 | A take reaching each recording capacity stops, is counted, and keeps every event recorded before the stop; no note is dropped and no earlier note is overwritten | 9 |
| HOST-INV-021 | A quantum is presented with more due events than it admits: the excess renders one quantum later, no event is lost, and a compiled event is never displaced by an ingress one. **Two counters, asserted separately** — the capacity-deferral counter rises by exactly the deferred count and the late counter does not move at all; and the mirror case, an event that is genuinely late, moves the late counter and not the deferral counter. ADR-0032 clause 22 is the precedent for why one test would pass on the wrong policy. A third case covers an event that is **both** late and deferred, asserting each counter rises exactly once | 3 |
| HOST-INV-020, and the retirement budget | A plan swap with `max_active_voices` sounding retires every voice with a crossfade and refuses none, so `max_concurrent_retiring_voices` cannot bind at its derived default | 9 |

## Unresolved questions

| Question | Blocking? | ADR or task |
|----------|-----------|-------------|
| What a channel layout is beyond mono/stereo, and whether the profile carries a layout set or one layout | No — stereo is V1's only constructed layout (`LIMIT-0059`) | ADR-0002, Phase 2 |
| What an observation tap is and who owns the analyzer surface; the three capacities here may become one registration budget | No — the capacities stand whatever the taps mean | ADR-0027, Phase 5 |
| The retirement crossfade's value, and whether ADR-0009 wants a concurrent-retirement budget below `max_active_voices` — which it may only take together with a defined behaviour for reaching it | No — V1's 128 frames compiles today, and the derived budget cannot bind | ADR-0009, Phase 9 |
| Recording take and commit semantics, which may change what a "recorded event" is | No | ADR-0024, Phase 9 |
| What a send is, which may change whether `max_sends_per_channel` is per channel or per bus | No | ADR-0034, Phase 8 |
| The script-work aggregate's threshold, which needs a measured per-instruction cost before it can become a `RenderLimits` field rather than a reported quantity | No — the `ResourceReport` carries the quantity meanwhile | Phase 7 |
| Whether HOST-INV-021's deferral can starve a low-priority event under sustained overrun. Compiled events take precedence **unconditionally**, so a plan that saturates `max_events_per_quantum` every quantum defers ingress indefinitely. The confirmation read sharpened this: the fix is probably not a starvation bound but a *reserved ingress allowance* the scheduler leaves free under ADR-0032 clause 27, which turns unbounded starvation into a declared budget — that is new design, and Phase 3 owns it | No — nothing is lost, only delayed, and the deferral counter makes the condition visible | ADR-0003, ADR-0023, Phase 3 |
| Whether `max_nodes` should be anchored independently rather than computed from `max_active_voices`, which is itself only measurement-anchored | No | Phase 2 exit |
| Whether `max_mix_channels` and `max_observation_taps` should be coupled so that every mix channel is guaranteed a tap | No — the report names which budget bound the plan | ADR-0027, Phase 8 |
| Where a profile is stored and who may edit it — application settings, host configuration, or neither | No — Phase 1 constructs it in code | ADR-0013, ADR-0029, Phase 10A |
| Whether the forward horizon survives calibration evidence, and whether a mis-calibrated anchor should widen it or fail preparation | No — the current value rejects and counts, which is the diagnostic | ADR-0022, Phase 3 entry |
| Whether `max_fan_out_per_port` should exist at all, or whether the edge budget alone suffices | No | Phase 2 exit |

## Review status

**One author pass, then one independent review pass, whose findings are corrected above.** The record of what that pass
found is kept here rather than in a commit message, because it is the argument for why the next pass is still required.

The review raised five findings; all five stand, and the fifth contained three separate defects.

| Finding | Severity | Correction |
|---------|----------|------------|
| `maximum_block_size >= Q` refused hosts the render model is built for. ADR-0001 clause 6 primes the output carry precisely so that a callback of `N < Q` can be served, so a device whose largest block is 32 frames was being rejected by a constraint with no purpose | High | HOST-INV-012 rewritten: the carries are still `maximum_block_size + Q`, and there is no lower bound in terms of `Q`. A conformance case with `maximum_block_size < Q` is added |
| The runtime-overflow contract contradicted three field tables. HOST-INV-009 permitted runtime loss only for live bounded queues, while the telemetry ring overwrote by design, a recording take stopped at capacity, and an over-full quantum had no defined behaviour at all — the four ingress queues hold 3 072 events against a 256-event scratch | High | HOST-INV-009 narrowed to *dropping*; HOST-INV-019 (lossy eviction), HOST-INV-020 (session limit) and HOST-INV-021 (per-quantum deferral) added. The failure table now lists five distinct behaviours, and each field carries exactly one |
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

**Status after four passes.** Author, independent (five findings, two High), bounded closure (four, one substantive),
and confirmation (two). Every finding is corrected here.

**On when to stop.** Each pass has found something, and on this evidence the next one would too — so "a pass finds
nothing" is not a criterion that will ever be met, and treating it as one would keep this `Draft` forever. The
criterion used instead: **a pass that changes no contract clause.** The confirmation read does not meet it — defining a
failure behaviour for `max_scheduled_events_in_flight` is a clause — so this specification stays `Draft` for one more
independent read, and it should be an *independent* one rather than a fifth pass by its author. Of the four so far,
only one was.

The standing checks from the previous pass remain, and none was disturbed by these corrections:

- that no default is sized by `Q`'s provisional value, directly or through arithmetic;
- that every one of the ledger's 30 `HostProfile` entries has a successor field, and that no field exists without an
  entry or a stated no-antecedent reason;
- that the one derived and two anchored EVD-0003 figures use the evidence as the evidence states it — cost, not
  capacity, and RSS as an upper bound rather than as prepared bytes;
- that no clause here contradicts ADR-0021 part 1's two axes, ADR-0001 clauses 1, 5, 6 and 16, or ADR-0032 clauses 21,
  22 and 23;
- that the `HostCapabilities`/`RenderLimits` split refines ADR-0021 part 4 rather than replacing it;
- that answering the master plan's "parameter and control slots" and its script-work aggregate through the report,
  rather than as budgets of their own, is right — and if it is, that the plan's own field list is corrected when this
  specification becomes `Current`, per the [documentation authority rule](../README.md#sources-of-truth).
