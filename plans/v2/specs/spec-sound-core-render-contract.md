# SPEC: Sound Core Render Contract

| Field | Value |
|---|---|
| Status | Current |
| Phase | 1–2 |
| Created | 2026-08-19 |
| Last reviewed | 2026-09-04, ADR-0006 and ADR-0007 accepted |
| Based on | ADR-0001, ADR-0004, ADR-0005, ADR-0021, ADR-0032, ADR-0037, ADR-0040, ADR-0041, ADR-0043, ADR-0046, ADR-0047, ADR-0049, ADR-0050, ADR-0055, ADR-0027, ADR-0006, ADR-0007 |
| Invariant prefix | `SOUND` |
| Supersedes | — |
| Superseded by | — |

Allowed status values are defined in [README.md](README.md). Only a `Current`
specification constrains implementation.

## Scope

This specification is the current executable contract for the experimental
Sound Core renderer, compiler, plan, runtime state, and internal arena through
Phase 2.

## Non-goals

It does not define project lowering, the live host, polyphony, general event
scheduling, modulation, or the final node catalog. Those belong to later phases
in [`ROADMAP.md`](../ROADMAP.md).

It does not define which layouts exist beyond `Mono` and `Stereo`, the summing
law for a stereo-to-mono down-mix, or sample-rate conversion and oversampling
islands. ADR-0041 leaves all three to later phases.

## Terminology

The [shared glossary](../glossary.md) defines *render plan*, *render quantum*,
*host profile*, *sample time*, and *stream epoch*. Here, `Q` is the current
render quantum in frames and the *arena* is the renderer-owned preallocated
storage assigned to compiled signal lifetimes.

## Accepted decisions

| ADR | Current rule represented here |
|---|---|
| [ADR-0001](../decisions/ADR-0001-internal-render-quantum.md) | Host blocks are rendered through fixed internal quanta and carry semantics |
| [ADR-0004](../decisions/ADR-0004-native-node-representation.md) | Admission prepares a node dispatch entry; the hot loop is independent of the catalog |
| [ADR-0005](../decisions/ADR-0005-buffer-liveness-strategy.md) | Conservative deterministic liveness reuse with declared safe in-place processing, over physical regions per ADR-0041 |
| [ADR-0006](../decisions/ADR-0006-parameter-ramp-representation.md) | A parameter slot's runtime value is a linear segment between two resolved values; a retarget starts from the current value; a sample-positioned control is never smoothed |
| [ADR-0007](../decisions/ADR-0007-parameter-modulation-laws.md) | A declaration names one modulation law from a closed set and the parameter slot composes the layers in one order; a kernel reads a resolved value and composes nothing |
| [ADR-0021](../decisions/ADR-0021-host-profile-and-admission-policy.md) | Resource limits are preparation inputs and excess plans are refused before render |
| [ADR-0026](../decisions/ADR-0026-minimum-sample-map-and-zone-model.md) | A sample zone selects by key and velocity and carries root, fine tuning, region, loop, gain and a prepared sample held once per plan; a sampler starts on a declared trigger destination rather than as a second playable node; V1's playback law, forward modes only |
| [ADR-0027](../decisions/ADR-0027-observation-and-analyzer-ownership.md) | An observation tap is a compiler artifact a node kind declares, present in the plan whether or not it is read, and passive toward every downstream signal |
| [ADR-0032](../decisions/ADR-0032-sample-time-and-event-timestamps.md) | Engine time, plan position, epoch, and quantum-local offsets remain distinct |
| [ADR-0037](../decisions/ADR-0037-render-quantum-value.md) | `Q` is 64 frames, and ADR-0037 fixes it finally rather than provisionally |
| [ADR-0040](../decisions/ADR-0040-v2-owns-its-dsp.md) | V2 owns the DSP it renders; no kernel is shared with V1 and no kernel carries a two-engine policy |
| [ADR-0041](../decisions/ADR-0041-interleaved-internal-channel-layout.md) | One signal is one interleaved arena region of `Q` frames of `c` channels, at a recorded offset and length |
| [ADR-0043](../decisions/ADR-0043-event-deferral-and-late-clamp.md) | A late event is clamped forward without rewriting its stamp; its sample and control effects follow the clamped render position |
| [ADR-0046](../decisions/ADR-0046-destination-quantum-admission.md) | Capacity is admitted before rendering; the renderer never moves an event to make a quantum fit |
| [ADR-0047](../decisions/ADR-0047-note-identity-in-the-event-contract.md) | A note-on names an occurrence as well as its node; a release names the occurrence alone |
| [ADR-0049](../decisions/ADR-0049-tempo-ramp-law.md) | A tempo ramp interpolates the beat's length, not the tempo number, so the conversion stays inside the four operations |
| [ADR-0050](../decisions/ADR-0050-transport-activation.md) | A rendering stream moves to a new mapping through one activation value adopted whole at a quantum boundary |
| [ADR-0055](../decisions/ADR-0055-refuse-unimplemented-loop-playback.md) | A loop-bearing activation is refused at the runtime offer until sample-exact wrapping exists; the interval cannot become active-but-unenforced state |

## Invariants

1. **SOUND-INV-001 — Preparation before rendering.** Graph validation,
   scheduling, node admission, resource accounting, buffer assignment, and
   prepared data construction happen off the render path.
2. **SOUND-INV-002 — Bounded hot path.** Rendering allocates no heap memory,
   acquires no blocking lock, performs no I/O or logging, and makes no graph,
   name, or topology decision.
3. **SOUND-INV-003 — Immutable plan.** A compiled plan is immutable during
   rendering. Each renderer instance owns separate mutable node and stream
   state.
4. **SOUND-INV-004 — Host-profile admission.** A plan exceeding its
   `HostProfile` or `RenderLimits` is refused before rendering with an
   attributable resource report. Rendering never truncates the admitted plan to
   fit.
5. **SOUND-INV-005 — Quantum execution.** Caller blocks up to the admitted
   maximum are split into consecutive internal quanta of the current `Q`; carry
   behavior preserves output across arbitrary caller block partitioning.
6. **SOUND-INV-006 — Typed event time.** An event crossing into the renderer
   retains its `StreamEpoch` and absolute `SampleTime`. Its quantum index and
   quantum-local offset are derived only after epoch validation, and they are
   derived from the event's **render position** rather than from its stamp;
   neither is stored in the event envelope. The two coincide unless
   SOUND-INV-016's late clamp moved the event. When an event takes effect is
   decided by what it moves rather than by which payload carried it, per
   SOUND-INV-016.
7. **SOUND-INV-007 — Graph validation.** Compilation refuses unknown nodes or
   ports, wrong directions/domains, illegal fan-in, cycles, missing required
   output structure, and any channel-layout mismatch no permitted conversion
   resolves, with path-local structured diagnostics. Layout refusal happens
   before admission reports resources, so the renderer never receives a plan
   with an unresolved layout.
8. **SOUND-INV-008 — Deterministic lowering.** Stable semantic identities lower
   to compact runtime slots and a deterministic schedule independent of source
   declaration order.
9. **SOUND-INV-009 — Channel storage.** One signal occupies one arena region of
   `c * Q` samples, frame-major: sample `(f, ch)` is at `f * c + ch`. A region
   is an offset and a length the plan records, not an index times the quantum,
   and a mono signal is `Q` contiguous samples. A kernel is told its channel
   count and must be correct for every count its own ports admit. Channel
   layout is a property of a port and its edge and fixes the region's width; a
   layout the build cannot render is not constructible. A layout is an **ordered**
   sequence of channels — channel 0 of a stereo signal is the left channel — and
   no conversion, copy, or boundary operation may permute it silently. This phase
   admits `Mono` and `Stereo` and refuses every other layout rather than
   inventing one.
10. **SOUND-INV-010 — Safe reuse.** Two overlapping live values never share
    storage: their physical sample ranges do not intersect at all, partial
    overlap included. Liveness is per signal rather than per channel, and an
    observation tap is a reader that extends the range it observes. Assignment
    is a pure function of the compiled plan: chains are requested in one total
    order — ascending schedule index of the operation that writes each chain's
    first value — and served first fit by ascending offset from a free list of
    physical regions that coalesces on release, splitting any hole larger than
    the request and appending a region exactly its own length at the arena's
    **exclusive-end** extent when nothing fits. In-place processing occurs only
    where the node declares it safe, no later read needs the input, and the two
    layouts are identical. The strategy is verified by two checks, neither
    optional: a structural check that no two overlapping live ranges were given
    intersecting ranges, over every compiled plan in the suite, and a behavioral
    check that compiling with reuse disabled renders bit-identical audio. That
    disabled mode exists for the check alone and no host profile reaches it.
11. **SOUND-INV-011 — Explicit silence.** An unpatched input reads defined
    silence rather than data left by a previous arena tenant.
12. **SOUND-INV-012 — Closed render entry.** Admission resolves the node's
    prepared data, mutable state shape, ports, controls, kernel entry, and
    in-place eligibility. Adding a node does not add renderer control flow.
13. **SOUND-INV-013 — V2 owns its rendered DSP.** Every kernel reachable from
    the render loop lives in this crate. Audio is not routed through a function
    whose behavior V1's corpus digests pin, and no kernel takes a parameter that
    selects between a V1 law and a V2 law. A dependency may still supply a value,
    a table, or a mathematical primitive that is not a kernel. Each node kind
    justifies itself by executable checks of its own rather than by likeness to
    V1.
14. **SOUND-INV-014 — Explicit conversion.** The only implicit conversion this
    phase inserts is mono to stereo, duplicating each sample into both channels
    of one wider region. A stereo-to-mono edge is a compile error naming the
    edge, both endpoints, and both layouts; no down-mix is summed by default.
    Every conversion is a scheduled operation with an identity, visible in the
    schedule, in the plan's buffer count, in the arena extent the resource
    report states as scratch bytes, and in diagnostics. An output whose layout
    matches the stream is one contiguous copy; where it does not, the resolving
    conversion is an ordinary scheduled operation. External boundaries convert
    at the edge and never reinterpret an arena region.
15. **SOUND-INV-015 — Channel coverage.** A node kind may not be added without
    tests at every channel count its own ports admit. A kernel whose ports admit
    only one channel is tested at one, with a test asserting that its port table
    admits only one, so the exemption is checked rather than assumed.
16. **SOUND-INV-016 — Sample-positioned effects.** ADR-0001 splits when an event
    takes effect, and the split is a property of the **effect**: a note-on,
    note-off, gate or retrigger occurs at the sample its **render position**
    names, while a control-rate response begins at the first quantum boundary at
    or after that position, under clause 13's causality rule. An event's render
    position is its declared sample unless
    [ADR-0043](../decisions/ADR-0043-event-deferral-and-late-clamp.md)'s preserving
    late clamp moved it to the first not-yet-rendered boundary; ADR-0046 removes
    capacity as a second reason to move an event.
    **This is stated over the render position rather than over the declared sample**
    because the clamp alone separates the two: a late event does not take effect at
    its declared sample, and an invariant written over the declared sample is false
    for it — as this one was before ADR-0043.
    The envelope's `time` is never rewritten, so the declared sample remains
    readable and the displacement remains the difference between the two.
    A node kind declares which of the two each of
    its controls is, admission compiles that declaration into the control's target,
    and the renderer reads it there. A caller therefore cannot obtain the other
    timing by choosing another payload: addressing a gate as a parameter and playing
    its node as a note reach one control under one law, and the two render
    bit-identically. A note **on** names a **node**, not one of its controls — which
    control being played moves belongs to the node kind, and a node that declares
    none cannot be addressed by a note at all. A release names no node: SOUND-INV-017
    gives it an occurrence instead, and an earlier revision of this clause said
    "a note event" where it meant the on edge, which made the two invariants
    contradict each other. The schedule is still walked exactly
    once per quantum, so this is neither a second control evaluation (clause 4) nor
    the event-boundary quantum split clause 15 reserves for a later phase: the edges
    due in a quantum are resolved before it renders and handed to the node that owns
    them.
17. **SOUND-INV-017 — Note identity.** A note **on** names an **occurrence** as well
    as the node it plays; a release and a per-note expression event name the
    occurrence **alone**. The occurrence is the sole authority for which note an event
    resolves to.
    [ADR-0047](../decisions/ADR-0047-note-identity-in-the-event-contract.md) supplies
    it. A note-on carries both: the compiled node, which is *what is played*, and an
    identity minted from the producer's disjoint range, which is *which occurrence*.
    A release and a per-note expression event carry the identity **alone**.

    **The node address is absent from a release rather than carried and required to
    agree**, and that is a design choice with a reason. Carrying both admits an event
    whose identity names one occurrence while its node names another, and no reading
    of that event is safe: honouring the node releases the wrong note, and honouring
    the identity silently redefines what the node field means. Removing the field
    removes the case instead of adjudicating it. SOUND-INV-016's rule that a note
    event names a node rather than one of its controls is preserved where it does
    work — the on edge, where the node is what is played.

    **An identity that names no live note is an orphan.** Three cases reach that: a
    **free** index, a **superseded** generation at a live index, and an index that has
    been **retired** under the rule below. Such an event is refused and counted, never
    resolved to another note. One orphan is expected rather than a defect: the release of
    a note a steal ended (ADR-0058, SOUND-INV-025), which preparation drops before it can
    reach the loop and counts under its own name, `released_after_steal`. That is what makes
    [ADR-0046](../decisions/ADR-0046-destination-quantum-admission.md) clause 3's
    promise executable — "a later edge for it is an orphan and is counted rather than
    allowed to release another note" — which the `{ node, edge }` vocabulary before
    this record could not express at all.

    **A generation value is never reused.** The counter at an index is monotone and is
    never wound back, so no stale identity can match the generation live at its index,
    whatever retained it and for however long. When an index exhausts its generation
    space that index is **retired** — withdrawn from its producer's range and never
    minted again — rather than restarted. **Each retirement is counted**, and when a
    producer's remaining range falls below its admitted simultaneous demand that is a
    **named exhaustion condition**, reported rather than absorbed: it is not a producer
    defect, since the producer declared correctly and did not over-emit, so attributing
    it as one would be false. Recovery is building a new table, which restores the full
    index space.

    No finite identity is unconditionally alias-free, and this specification does not
    claim otherwise. What the rules above buy is the direction of failure: a width
    chosen too small costs a reported exhaustion and never a wrongly released note.

    **The index space is at least `max_held_notes`**, which the host-profile
    specification states as a construction relation.

    An identity is valid only for the identity table that minted it, is never
    persisted and never crosses a wire contract. Neither the plan identity nor the
    stream epoch scopes it alone: a re-admission changes the plan without changing the
    epoch, and a re-preparation changes the epoch without changing the plan. **The
    table's own identity is issued strictly increasing and refuses permanently on
    exhaustion**; a saturating issuer would reissue after its ceiling and make two
    tables indistinguishable, which is the one thing this scoping exists to prevent.

    An identity from any other table is **rejected and counted**, never resolved.
    ADR-0032 clause 20's epoch rejection stays a **separate and earlier** filter: an
    event from a dead stream is discarded before its identity is examined at all, so
    the two counters answer different questions and neither substitutes for the other.

    A rebuild while an obligation from the outgoing table is outstanding is **refused**.
    Rejecting the eventual release would contradict ADR-0046 clause 3's guarantee that
    an accepted obligation is never refused later, and stranding it would leave a note
    nothing can release. Lifting that limit is ADR-0048's, for Phase 9.

    **A transport activation advances the allocator's generations earlier than an
    applied mass release does, and that is an amendment rather than an exception.**
    [ADR-0050](../decisions/ADR-0050-transport-activation.md) clause 5 makes it: for a
    schedule replaced at an activation boundary, the index generations move at stamping
    or at candidate build, and the authoritative table takes them when the retired value
    is collected — none of which is "at application". What is preserved is what the mass
    release is *for*: a release arriving after one resolves as an **orphan**, which the
    live-note registry delivers because it is cleared at the boundary. The protection
    against a stale identity matching a newly started note is this invariant's own
    never-reused generation, not the timing of any one release. The window in which the
    allocator lags is unobserved only while the control is the sole minter, which is
    ADR-0050 clause 8's second obligation.

    **Pitch and velocity are fixed by SOUND-INV-021**, which
    [ADR-0025](../decisions/ADR-0025-tuning-representation-and-ownership.md) created on
    acceptance. Its **key and velocity** clauses are what gate a saved note's render, and they
    are implemented: a note-on carries both as validated magnitudes and a saved note renders.
    Its bend clause is built by `P06-S003` and its composition clause by `P06-S004`; neither
    gates this. ADR-0047 adds identity and discharges neither magnitude.

18. **SOUND-INV-018 — Transport activation.** Moving a rendering stream to a new
    plan mapping — a seek, a loop wrap, a tempo-map replacement — happens through
    **one activation value**, built off the audio thread and adopted whole.
    [ADR-0050](../decisions/ADR-0050-transport-activation.md) supplies it.

    **The effective point is a quantum boundary.** An activation names a requested
    engine time `T`; it takes effect at the first quantum boundary at or after
    `max(T, clock)`, and the new mapping governs the half-open engine-time interval
    from there. `T` is immutable, exactly as SOUND-INV-016 keeps an event's stamp
    immutable, and an activation whose `T` precedes the clock **at the offer** is
    **late**: it activates at the clock and is counted, so the delay is reportable.
    The offer is the moment the question is asked, because it is the moment the rule
    is about — the candidate was finished after the time it named had passed. Asked
    at adoption the answer is worthless: the clock stands on the boundary by then, so
    every request that does not fall on one would answer yes. A late activation is
    **not** the same as one displaced beyond its own snapped boundary, and neither
    implies the other: a request one frame past a boundary offered against a clock
    already at the next one is late and displaced by nothing.

    The **loop interval** in the candidate request is admitted when the candidate is built, on
    two counts that answer to two records. ADR-0046 clause 4 checks the periodic
    extension of `[start, end)` against the compiled share. `SOUND-INV-017`'s producer
    range is checked over the same pass: the notes it holds open at one instant may not
    exceed what the compiled note producer is admitted for. The subject of both is the
    pass a **wrap** replays — anchored at the loop's start, beginning with nothing
    sounding because the boundary mass release ends the previous pass, and counting a
    crossing release as the bare gate-down it is rather than as a note contract it ends.
    Neither bound implies the other.

    Runtime loop playback **fails closed** under ADR-0055. The interval can pass
    these off-thread checks, but offering its activation returns
    `LoopPlaybackUnsupported` with the interval named, increments the refused-
    activation counter and changes neither active schedule nor loop state. The
    first consumer that enables loop playback must replace this guard with the
    sample-exact mechanism coupled to ADR-0052.

    The **catch-up batch** below is part of this invariant and is built.

    **The adopted anchor is `(effective, requested position)`.** The engine-time half
    is snapped; the plan-position half is not, because moving it would seek somewhere
    other than where the caller asked. A candidate is stamped against `T`, so when the
    effective point differs from `T` every placed event is displaced by the same
    `effective - T`, and the stream carries that as **one offset applied at
    publication** rather than as a rewrite of the stamped list — the shift is uniform,
    so no per-event pass runs on the audio thread. This is a placement rather than a
    clamp: an event's engine time is a function of its plan position and the anchor,
    and the anchor moved.

    The effective point is a function of `T`, `Q` and the clock alone, so it does
    not depend on how the host partitions its callbacks. Where the boundary falls
    strictly inside the quanta a host block would render, the stream renders that
    block as two renderer calls and adopts between them, which SOUND-INV-005's
    carry behaviour already makes free.

    **This is not sample-exact seek or loop.** Placement error is up to `Q - 1`
    frames, 1.33 ms at 48 kHz; what a listener hears is that plus the stream's
    existing `Q`-frame output latency, so an on-time request can be heard as much as
    `Q + (Q - 1)` = 127 frames late, about 2.65 ms at 48 kHz. Only the first `Q - 1`
    of those is new. The master plan's sample-exact loop and seek requirement is not
    satisfied by this rule, and no gate may be read as closing it on this rule's
    strength.

    **A future repeating wrap must keep its ideal phase.** ADR-0052 owns the
    executable mechanism and its conformance check. No runtime wrap helper exists
    while ADR-0055's fail-closed guard is in force.

    What quantum granularity buys beyond implementability is admission. Shares are
    charged per destination quantum, so a boundary-aligned activation puts the old
    stream's last events and the new stream's first events in **different quanta**;
    the junction is admissible under the two admissions the streams already passed
    and needs no third rule. A sample-exact activation would need a junction check
    in the shape of ADR-0046 clause 4's periodic loop extension.

    **Neither carry is touched.** The output carry holds audio for engine samples
    strictly before the clock, and the effective point is at or after the clock, so
    that audio was rendered under the mapping in force when it was rendered. That
    holds because adoption happens **between** renderer calls: a call spanning several
    quanta accumulates them in the carry before copying out, so an adoption inside one
    would leave the carry holding audio from both sides of the boundary.

    Between calls the live carry never exceeds `Q` frames — a call renders the fewest
    quanta that cover its request, so the remainder is under one quantum, and the
    stream starts with exactly `Q`. The `maximum_block_size + Q` the buffer is sized
    to is the peak reached **inside** a call, which is another way of seeing why
    adoption belongs between them. Discarding it would therefore deliver a gap of up
    to `Q` frames for no correctness gain. A listener therefore observes an activation `Q` frames
    after its effective engine time, which is the stream's declared added latency
    and nothing more. An operation that must silence already-rendered audio is a
    **fault** under ADR-0021, not an activation.

    **One value carries the whole adoptable state set**: the anchor, the placed and
    stamped compiled schedule with its cursor, the tempo map where one is replaced,
    and the locate catch-up batch. A candidate may also carry a requested loop
    interval, but ADR-0055 refuses it before adoption and active loop state is absent
    from both transport halves. The stream's arbiter is neither carried
    nor swapped — ADR-0046 clause 2 admits exactly one per stream, so an adopted
    schedule inherits the latched identity of the schedule it replaces. The identity
    table is not swapped either: both schedules mint from the one table, which is
    what makes an occurrence resolvable across an activation.

    **Failure leaves the stream exactly as it was, and refusal happens at the offer
    rather than at adoption.** Most checks run while the candidate is built: admission
    of the stream and of the loop, placement against the new anchor, and stamping.
    **Offering** the candidate can refuse for exactly six further reasons — a schedule
    paired with another stream's renderer, a stream that has already faulted, a stale
    epoch, unsupported loop playback, a superseded sequence, and an occupied exchange
    slot — and each leaves the state in force. **Five of the six increment the refusal
    counter; the pairing does not**, and the exception is the reason it is checked first: the counters belong to
    the stream that was offered to, so attributing this refusal to a renderer that is
    not this schedule's half would put a diagnostic on a stream nobody asked anything.
    The occupied slot is backpressure and not a fault, and it is occupied in two
    distinguishable ways: by a candidate not yet adopted, or by a retired value not yet
    collected. A diagnostic reports which. The faulted stream and the pairing were both
    found by the implementation and are listed here rather than left implicit: after a
    terminal fault no later call advances toward a boundary, so a candidate accepted
    then would never be adopted, never be collected, and never be withdrawable.

    **A candidate is stamped against a copy of the minter.** Stamping is not reversible
    by releasing what it minted — a note-on paired inside the list already advanced its
    index's generation and may have retired it — so the control takes a working copy
    for the build and, before stamping, releases the outgoing schedule's outstanding
    occurrences into it — a schedule records that set, the note-ons its own list never
    paired, when it is stamped. The candidate is therefore minted against the table as it
    will be **after** the activation.

    Withdrawal, including of a candidate returned refused at the offer, drops the copy:
    nothing is consumed, neither an index nor a generation. The copy is promoted when
    the **retired value is collected**, not when the offer is accepted — an accepted
    offer makes adoption infallible *if reached*, and the renderer can end the epoch
    first, in which case no later call advances toward the boundary and the activation
    never happens. A faulted epoch needs no reclamation: re-preparation issues new
    tables. **No candidate built while an activation is outstanding can be adopted**,
    which removes the window between acceptance and collection. *Outstanding* means
    accepted at the offer and not yet collected: the freshness rule below deliberately
    allows two candidates to be **built** against one in-force sequence and refuses the
    loser, and it is acceptance rather than building that closes the door. The control
    does not forbid the build, because it cannot see an acceptance — ADR-0050 clause 6's
    amendment replaces the prohibition with what a candidate *is*: the control issues
    what a candidate supersedes from the sequence it last promoted, so one built in that
    window carries a superseded value necessarily and the offer refuses it.

    The set the copy releases is what the outgoing schedule still **reserves in the
    allocator**, and that is not the set the boundary ends. They overlap without
    containing each other: a note-on paired later in the outgoing list is sounding at
    the boundary but freed its index at stamping, while an unpaired note-on beyond the
    boundary is reserved but never sounded. Each half deals with its own set. The two
    schedules also never compete for a producer's range, which a reclaim performed only
    at retirement could not deliver: a producer whose declared polyphony the outgoing
    schedule already uses would otherwise be unable to build any replacement.

    **Adoption itself is an infallible move.** Nothing between an accepted offer and
    the boundary can invalidate it, and the swap has no branch that can fail. A
    refusal discovered at the boundary would have to roll back a partly applied state
    set or fault the stream, and the atomic set exists to avoid the first while the
    second would turn a caller mistake into a terminal fault. A violation detected
    after adoption is ADR-0046 clause 7's terminal response rather than a rollback.
    Adoption **exchanges** rather than replaces: **every** piece the activation
    replaces — anchor, schedule, tempo map and catch-up batch — moves into the
    return slot the off-thread half collects, so nothing is dropped on the audio
    thread. Naming a subset would be worse than naming none, because the omitted
    pieces are the ones that own allocations.

    **A note sounding at the effective point that a replaced producer started is
    ended there**, as ADR-0046 clause 6's bounded mass-release operation: one
    operation scoped to those producers, charged to the session share, never expanded
    into one event per voice. A **compiled** producer has no hold to redeem — clause 6
    gives its releases a plan entitlement instead — so ending a compiled note frees an
    identity and nothing else, and that is the whole of what this rule covers. What the
    operation must do when its scope reaches a non-compiled producer is named below as
    unsupported rather than described loosely. The scope is the retired schedule's producers and not
    everything: a seek moves plan time, it does not lift a performer's finger.

    **The operation has two halves and only one is on the audio thread.** There,
    adoption clears the retired producers' entries from the live-note registry and
    lowers the gates they name; the registry needs that scoped release and the producer
    ranges it names. Scoping the clear to a producer range is safe, and the reason has
    to be stated exactly because a plausible version of it is false: it is **not** that
    an occurrence enters the registry when its event is applied — the renderer admits
    occurrences during resolution, before the call's first quantum renders. It is that
    the candidate's events reach no render call until after adoption, so at adoption
    the registry can hold none of them whatever the order inside a call.

    The allocator half is the copy above, and it ran earlier. That is an **amendment**
    to SOUND-INV-017's mass-release rule and is recorded there: the generations move at
    stamping or at build rather than at application. What the rule is *for* is
    preserved — a later release for one of those notes resolves as an orphan, delivered
    by the registry being cleared at the boundary — and what keeps a stale identity from
    matching a note the incoming schedule started is the never-reused generation.

    **This rule covers a stream whose note producers are compiled.** Three things it
    does not close are named in ADR-0050 clause 8, as ADR-0051 raised it, and belong
    to later slices: a non-compiled producer's release holds have no redemption
    authority at the boundary, so replacing such a producer's schedule is unsupported
    until holds exist; while an activation is outstanding the control must be the
    **only** minter, since a second producer minting into the authoritative table
    would be erased by promotion; and **one scalar gate reached by two producers has
    no ownership law** — a compiled note and a surviving live note can address the
    same `(node, control)`, so ending the compiled one writes `ZERO` to the gate they
    share and cuts the live note with it. The scope predicate alone does not close
    that third one, because a gate carries neither producer attribution nor depth, and
    neither does excluding an out-of-scope contract from the substitution: every
    prepared target still receives a catch-up row, so a gate a live producer holds
    would be moved by the row itself. Before multiple producers may emit onto one
    gate, target sharing across release scopes must be refused at admission, or a
    voice's gate made producer-exclusive, or a depth-and-ownership aggregation law
    designed — and with it what the catch-up row does for a gate the activation does
    not own. All three hold today because **no stream that can activate has a live
    ingress store**: building an activation is refused once a stream has adopted one,
    and store adoption is refused while an activation candidate is outstanding.
    Those two checks cover both orderings rather than relying on absence, which
    is what changed when live ingress
    arrived — a plan may declare a non-compiled producer, and since Phase 3 one can
    also mint and emit, so the earlier reason, that the compiled producer is the only
    minter, no longer holds on its own. The check is on the adopted **store**: a count
    of open live notes returns to zero while both edges are still queued and neither
    has rendered, so it cannot see a note that is about to sound.

    Neither half is *replaced*: both keep the identity they were opened with, so an
    occurrence stays resolvable across an activation and the registry identity the
    renderer's foreign filter compares against does not move under it.

    **A suffix omits a release whose note-on lies before its anchor.** Seek between a
    compiled note's edges and the suffix holds a release with nothing to pair it with,
    which stamping refuses — so without this rule the commonest seek could not produce
    a schedule at all. The omission is correct rather than lenient: the boundary mass
    release ended that note and the new stream never started it. It happens where the
    suffix is built, off the audio thread, and is **counted**, so it is a named
    transformation rather than a renderer-side drop SOUND-INV-016 forbids. Note
    chasing — restarting such a note at the destination — is a different product
    choice and is not made here. The audible consequence is stated rather than hidden:
    a note sustaining across a loop wrap or a seek is cut, and not resumed.

    **What is omitted is the note contract, not the gate write.** The suffix keeps a
    bare `SetParameter` of `ZERO` on the release's physical gate target, at the
    release's own position and carrying no note identity, so the authored gate
    timeline survives the locate intact.
    [ADR-0051](../decisions/ADR-0051-locate-catch-up-gate-exception.md) clause 5 owns
    this. Without it a gate raised by automation at or after the destination has
    nothing left in the stream that can lower it, and the seek leaves a note sounding
    that playing through would have ended. The write takes the release's place one for
    one, so the candidate never carries more events at a position than the admitted
    stream it came from and admission needs no change. A note slot whose gate has no
    prepared parameter row is refused when the activation is built, rather than
    silently losing its gate-down.

    **Freshness is a sequence, not a value comparison.** The off-thread half issues a
    strictly increasing activation sequence; a candidate names the sequence it
    supersedes and is adopted only when that equals the sequence in force. An anchor
    value cannot serve: a tempo map replaced so that it differs only after the current
    tick leaves the anchor bit-identical while every future position moves. The
    candidate also carries the stream epoch and one from another epoch is refused as
    stale, exactly as an event is under SOUND-INV-006.

    The value in force starts at the sequence the stream was opened with and changes
    at **adoption alone**, never at issue. So an abandoned candidate consumes no
    sequence — the identities it reserved are the paragraph above's — and cannot wedge
    the stream, and two candidates built against one in-force value
    are ordered rather than raced: the first adopted moves the value and the other is
    refused. Because building is off-thread work of unbounded duration, a candidate
    can be finished after its requested time has passed; it then activates at the
    clock and the late counter is what makes that attributable.

    **An activation that relocates carries a catch-up batch, and it covers every
    prepared target rather than only the automated ones.** For a target with a value
    established before the new plan position the batch carries the last such value;
    for a target with none it carries the value that target was **prepared** with.
    The second half is not a detail: a control value lives in node state across an
    activation, so seeking back to before a parameter's first automation point would
    otherwise leave the value that automation set — the stale value being the one the
    seek was supposed to leave behind. Covering every target also makes the batch's
    size exactly the plan's prepared-target count, which is the quantity admission
    checks. It is built with the rest of the candidate, charged to the
    session share, and publishes completely.

    **A physical `(node, control)` gate held open by an in-scope note contract
    immediately before the destination carries `ZERO`**, whatever the last write
    before that position was. Note edges write the history like any other write, so
    a note-off before the destination lowers the gate there; this rule decides the
    remaining case, where the contract is still open.
    [ADR-0051](../decisions/ADR-0051-locate-catch-up-gate-exception.md) owns it. The
    reason is that a gate is **edge-triggered** — the kernel treats a re-asserted
    level as no edge — so automation raising a gate a note already holds is inert
    while playing through, and restoring that value after the boundary mass release
    lowered the gate is a **rising edge** that restarts an envelope no note contract
    stands behind. That is the note chasing this invariant's release rule declines.

    **The predicate is the destination-open contract and not the mass release's
    scope**, which are different sets: a forward seek can land inside a note the
    retired stream never sounded, and that gate must still be low. **In scope** means
    the replaced producer's, as the release scope is, so a seek does not lift a
    performer's finger. The substitution aggregates by **physical target**, because
    two prepared parameter slots aliasing one gate would otherwise disagree and
    whichever published last would win. Open-contract depth is tracked separately
    from the gate history, which reproduces the renderer: every resolving note-off
    writes `ZERO` even where another occurrence on that slot is still open. The rule
    decides a row's **value** and never whether the row exists, so the batch's size
    is still the prepared-target count. Without it an admitted suffix is not a
    correct seek: a suffix refuses every position before its anchor, so nothing in
    the new stream carries the automation value that was in force when the user
    seeked past it. **The catch-up applies before the new stream's events at the same
    sample**, which is what a catch-up is rather than a general same-sample policy:
    the batch is the state already in force at the destination and the stream is what
    happens from there, so the other order would let a stale restoration overwrite the
    new timeline's first event.

    **The retired schedule's unpublished events are not dropped events.** A seek or a
    wrap ends the timeline they belong to, so SOUND-INV-016's prohibition on silently
    discarding an event does not reach them and no counter records them.

    **The stream has two owners.** The off-thread half owns the tempo map, the
    current anchor, admitted streams, identity **minter** and the
    building of candidates; the audio-thread half owns the clock, the carries, node
    state, the **live-note registry** and adoption. That split is what lets a
    candidate be built while the stream renders, without a lock the real-time rules
    forbid.
19. **SOUND-INV-019 — Tempo conversion law.** The tempo map converts a musical
    position into a `PlanPosition` and never into an engine time. Its law uses only
    the four IEEE-754 arithmetic operations, comparison, and rounding, per
    SOUND-INV-006's timing model and the record it rests on, and it rounds to a
    frame exactly once, half away from zero, over the sum of the stored segment
    prefix and the offset inside the segment.

    **A step segment lasts its length in beats times `60 / bpm`.** A **ramp**
    segment interpolates the **period** — seconds per beat — linearly between its
    declared endpoints: with `p0 = 60 / b0`, `p1 = 60 / b1`, `B` the segment's
    length in beats and `beta` the beats elapsed from its start,
    `seconds(beta) = beta * 60 / b0 + (p1 - p0) * beta * beta / (2 * B)`. The
    linear term is the step law's own value, shared rather than recomputed, which is
    what makes the equal-endpoint case below exact. A segment's total duration is
    therefore `B * (p0 + p1) / 2`, but the map stores the next segment's prefix by
    evaluating the same expression at the segment's end rather than that one, so a
    boundary cannot disagree with a query inside the segment. A tick at or past a
    ramp's end belongs to the next segment, so `beta` never exceeds `B`.

    A ramp declares its destination as the **next** change's tempo and reaches it
    exactly there; a ramp with no following change is a step at `b0`. A ramp whose
    two endpoints are equal is bit-identical to that step, and the law has **no
    near-flat branch** — the quadratic term vanishes continuously rather than
    cancelling, which is the property that removes the special case rather than
    hiding it.

    **The rounded conversion is non-decreasing over the domain music reaches, and
    that is a checked property rather than a guarantee.** The step law is
    non-decreasing by composition: every operation in it is monotone in its argument,
    so rounding can merge two ticks onto one frame but never reverse them. The
    accepted ramp evaluation subtracts a positive quadratic from a positive linear
    term whenever the tempo rises, and inherits that subtraction; adjacent ticks can
    therefore convert to decreasing positions once the position's own rounding exceeds
    the per-tick increment. **No threshold is stated**: three were, and each was
    refuted by a better search than the one that produced it. What is stable is that
    both a position decades of audio out **and** a tempo ratio in the thousands are
    needed. A form that is monotone by composition exists and is rejected on
    measurement, not overlooked; the decision record carries that trade. The property
    checked is a sampled one — adjacent ticks in windows of steep ramps — and a
    consumer needing a guarantee must obtain a tighter position bound rather than
    assume one.

    The tempo **reported** inside a ramp is `60 / p(beta)`, with `p(beta)` formed as
    the weighted combination `p0 * (1 - beta / B) + p1 * (beta / B)` and then clamped
    to the interval `[min(p0, p1), max(p0, p1)]` its endpoints define — where the
    convex combination mathematically lies, and where rounding alone can take it out
    of. Its reciprocal is then bounded by the two tempi the caller declared. What is
    linear is the beat's length, not the tempo number, so a display drawing a ramp as
    a straight line between two BPM values draws something the engine does not play.

    A tempo whose **period** is not finite — below `60 / f64::MAX` — is refused where
    every tempo is validated, alongside the non-finite and non-positive values. Its
    period would otherwise reach a position and a reported tempo without passing
    through that validation.

20. **SOUND-INV-020 — Same-sample application order.** Two events carrying the
    same render position apply in the publication pass's **declared drain order**,
    and the unit of order is the producer rather than the capacity class an event
    is charged to. The pass drains session and transport, then compiled, then
    authored runtime in plan declaration order, then live ingress in queue order;
    each producer is drained in one contiguous block and in its own emission
    order; where one position holds several producers they are ordered by plan
    declaration order; and a renderer-internal emission applies after every
    external event at that position, because the rendering of that quantum is what
    produced it.

    **Every session and transport kind the master plan names has a position**,
    assigned by the producer that can emit it rather than chosen per kind:
    transport stop, count-in, metronome, preview and recording state are session
    and transport state and take the session block, beside the boundary release
    and the locate catch-up; panic and sustain lift are charged to the live share
    (ADR-0046 clause 6) and take the live block, ordered against other live edges
    by that store's queue. Play, seek, loop wrap and offline range start are
    **activations** rather than events — ADR-0050 clause 1 adopts one between two
    renderer sub-calls, so the old stream's events and the new stream's are never
    presented together — and what they contribute at a tie is the boundary release
    and the catch-up, which take the session block like any other session event.
    A position is not an event kind: what each of these *does* belongs to the slice
    that builds it.

    **Ordering by producer rather than by class is what keeps a producer's own
    sequence intact.** ADR-0046 clause 1 partitions capacity, not causality: a live
    note-on spends the live share while the release that discharges it redeems a
    hold into the guaranteed-release share, so any order derived from the class
    applies that release before its own note-on wherever the two share a render
    position. Draining the producer's queue once carries both edges in the order
    they were offered.

    The render position is the one ADR-0043's preserving late clamp produced, so a
    late event is ordered where it plays rather than where it was stamped. The
    locate catch-up preceding the new stream's events at one sample
    (`SOUND-INV-018`) is an instance of this rule rather than an exception to it.

    **No type refuses a drain written in the wrong order**, and the guarantee is
    correspondingly a tested one: the relative order of two producers, and the
    contiguity of one producer's block, need separate fixtures, because one event
    per producer cannot distinguish `A1, A2, B1` from `A1, B1, A2`. A producer for
    which no case exists is not covered by either.

21. **SOUND-INV-021 — The note payload's magnitudes.** A note **on** carries, beside the
    occurrence SOUND-INV-017 gives it and the node SOUND-INV-016 gives it, exactly two
    magnitudes: a **key identity** and a **velocity**. A release carries neither. Since
    ADR-0026 a kind may also declare a **trigger** destination, which carries no magnitude
    of its own: the expansion writes the note's on edge to it with the note-on and its off
    edge with the release, so a kind that must start and stop on a note without being its
    address — the sampler — receives both edges through the binding below.
    [ADR-0025](../decisions/ADR-0025-tuning-representation-and-ownership.md) selects the shape
    and this states what implementation must do.

    **The key identity is a keyboard position in `0..=127`, validated rather than clamped.**
    `synth_core::MidiNote::new` replaces a value above 127 with 127, which is the silent
    substitution of out-of-domain input that this crate's boundary rule forbids, so the key
    entering a plan is built through a fallible constructor of this crate's own.

    **The key is not a frequency, and no node converts one to the other.** The renderer resolves
    a key through a **prepared tuning**: immutable, derived off the audio thread from the
    authored `TuningDefinition` Phase 10A owns, and **referenced** by each pitch-producing node
    rather than copied into it. One prepared value exists per distinct tuning; every node of one
    execution scope references the same one, so a scope cannot resolve two keys two ways. The
    table's bytes are charged once to the plan's immutable prepared total and the reference is
    charged per node, so the resource report distinguishes a second scale from a second node.
    Derivation is deterministic and carries a digest, so two preparations of one definition are
    the same table and the report can say so.

    **A note's magnitudes reach nodes by execution scope, and that is the binding.** A note
    plays one node — `SOUND-INV-016` is unchanged — but its key and velocity must reach nodes
    that are not the one played: in the smallest real voice the gate is an envelope's and the
    pitch is an oscillator's. Admission resolves this from **declarations rather than from the
    caller**: it collects, from every node kind within the played node's execution scope, the
    controls that kind declares as a pitch or velocity destination, and the note's address
    becomes that set together with the played node's gate. A producer names a node, never a
    destination, so nothing here reverses `SOUND-INV-016`'s ownership.

    **A plan is refused when one execution scope holds two playable nodes.** The motivating
    case is the voice scope: `ExecutionScope::Voice` names a kind and not an instance, so two
    instruments' nodes are indistinguishable to the rule above and a note for one would reach
    both. The check is stated over **every** scope because the reason is — the binding merges
    within a scope, so two playable nodes sharing any one of them would each move the other's
    velocity, and where the scope has an oscillator they would contend for one pitch. Phase 6
    supplies instance identity and generalises the binding; until then the ambiguous plan is
    refused rather than resolved by declaration order. This is the narrowest rule that is
    implementable now and it names what makes it narrow.

    **A scope with a pitch destination states its own tuning, and one with none is refused.**
    No default is substituted at admission: choosing a scale is the authored model's decision
    and Phase 10A's, and a key with nothing to resolve against has no frequency. That is what
    makes "one prepared value per distinct tuning" a property of the plan rather than of a
    fallback — a plan states a tuning per scope, and admission deduplicates by comparing the
    prepared tables, so two scopes naming one scale share one. The comparison is over the
    tables and not over their digests: a digest is a 64-bit hash, and two scales colliding on
    one would make the second scope resolve every key through the first.

    **A prepared tuning is total over the key range**, so a node always has an answer and the
    audio thread never meets a key it cannot resolve. Totality is **structural**: the prepared
    table has an entry for every key in `0..=127`.

    Preparation additionally refuses any entry that is not a **usable** frequency — non-finite,
    zero or negative — because those reach a phase accumulator and are unrecoverable there.
    What preparation **cannot** establish is that every entry was *authored*: an authored
    definition may map only some keys, and the table type this crate consumes carries no record
    of which, extrapolating an entry for the rest. Completing a partial definition therefore
    belongs to the authored model and to Phase 10A, and this invariant does not claim otherwise.
    The gap is asserted by a named test rather than left implicit.

    **The velocity is one validated normalized magnitude on the on edge.** One magnitude, not
    two: V1 consumes a single saved velocity at both its envelope and its voice output, and two
    consumers of one fact do not need two fields. It is not `synth_core::Velocity`, whose
    constructor clamps. How the two consumers each scale it is the composition clause below.

    **It must be audible, and a plan where it is not is refused.** The Phase 4 gate states that
    a fixed-velocity render cannot satisfy it, so a typed velocity that reaches nothing would
    satisfy the letter of this invariant and none of its purpose. The minimum is therefore
    stated here rather than deferred: the played node's scope must declare at least one velocity
    destination that scales the rendered amplitude, and two renders of one plan at different
    velocities must differ in peak amplitude. A plan whose scope declares none is refused at
    admission. **How V1's two sensitivities compose** — the envelope's and the voice output's —
    is Phase 6's law and is not this; what this fixes is that velocity does something.

    **Both are magnitudes beside the gate, so a note-on resolves to more than one control
    write.** That is a change to the relation SOUND-INV-016 states over one control, and it is
    the whole of it: the node kind still declares which controls a note moves and at what rate,
    so timing still belongs to the destination and a caller still cannot obtain another timing
    by choosing another payload. What changes is cardinality. Admission must charge for the
    expansion — `DueEvent`'s resolved targets and the timed-control scratch relation are both
    sized on it — and a plan whose worst-case quantum exceeds its profile is refused as any
    other over-budget plan is.

    **The magnitudes take effect before the gate they arrive with.** A note's key and velocity
    describe the note the gate starts, so a gate raised at the same sample must see them
    already applied. Two edges at one sample resolve under SOUND-INV-020's declared order like
    any others; what this fixes is that the magnitudes of one note-on are not reordered against
    that note-on's own gate.

    **A per-note bend is a continuous offset in cents applied after resolution**, carried by an
    event addressing the occurrence per ADR-0047 clause 9. It is not a scale step: the tuning
    owns which frequency a key is, and a bend owns how far the note has moved from it. **It is
    the occurrence's own layer of its pitch destinations**: a `Cents` — finite, within ten
    octaves — reaches every pitch destination of the note's scope on the occurrence's instance
    rows as SOUND-INV-023's modulation under the semitone law, kept apart from the modulators'
    sum so that a modulator in force on the destination stays in force across a note-on while
    a new occurrence on the voice starts with its own layer at the law's identity. It is
    sample-positioned, as the pitch destination it moves is. On the compiled path a bend
    addresses the newest open note of its key on its node, as a release does, and preparation
    resolves it to the occurrence — displacing it with a note that took a voice, dropping and
    counting one for a note a steal ended, refusing one for no open note by name; a bend of a
    note the anchor's boundary release ended is omitted and counted by an activation. At the
    live boundary a bend names the occurrence the note-on returned, waits with a start a steal
    deferred, and is refused and counted as an orphan expression where the occurrence is not
    the producer's live note. A bend reaching the loop for an identity naming no live note is
    an orphan under SOUND-INV-017.

    **The bend clause is built by `P06-S003`.** `Cents` is the quantity, `CompiledPayload::Bend`
    and `EventPayload::Bend` the events, `SlotState::express` the layer, `ModulationLaw::combine`
    the composition of the two layers, and the live boundary's `offer_bend` the second site.

    **The trigger destination and the off edge (ADR-0026 clause 5).** A release, which
    carries no magnitude, still expands: it writes the off edge to every trigger destination
    of the released note's scope, at the release's own render position and on the note's own
    voice instance, resolved once with the release's target rather than read from the
    registry in the passes. A trigger's on edge is written by the note-on with the
    magnitudes; an activation's boundary release lowers a crossing note's trigger
    destinations beside its gate, and the catch-up restores no trigger, because an edge is
    not a value a seek can land inside. A note whose key or velocity selects no zone of a
    sampler it reaches is written no on edge there and is counted in the report as
    `notes_outside_zone`. The release's writes are within the note's own width — the gate
    and the magnitudes — so the timed-control scratch needs no further charge; the
    adoption's gate-downs are sized and charged at a note's width per identity for the
    same reason.

    **Velocity composition: a voice has two velocity destinations, each with its own
    sensitivity, and the formulas are V1's bit for bit.**
    [ADR-0059](../decisions/ADR-0059-velocity-composition.md) decides it. The one payload
    velocity `v` reaches every velocity destination in the note's scope, as the binding above
    already says; what each destination *does* with it is that kind's declared law, and there
    are two:

    - The **envelope** scales the level it emits by `1 − s × (1 − v)`, where `s` is its own
      authored `velocity_sensitivity`, a quantum-rate control the kernel reads per frame
      beside the velocity it holds. At `s = 1` the scale is `v`; at `s = 0` the velocity is
      ignored. This is V1's `vel_sens`.
    - A **velocity scaler** is a voice-scope kind — audio in, audio out, in place — that
      scales its input by `(1 − s) + s × v` from its own velocity destination and its authored
      `sensitivity`. At `s = 1` the scale is `v`; at `s = 0` it is unity. This is V1's
      instrument-level `velocity_amp_sensitivity`, applied at voice output.

    A plan authored with both renders a note at the **product** of the two factors, so at V1's
    defaults — both sensitivities one — the level is `v²`, which is V1's. A plan authored
    without a scaler has one destination, the envelope's, as before ADR-0059, and its existing
    renders are unchanged: the evidence digests reproduce. Each factor is computed by its own
    kernel from its own sensitivity and nowhere else; a kernel composes nothing across the
    two, and the product is what the signal path produces by passing through both. Neither
    sensitivity is a per-note magnitude: both are authored bases in the IR, and Phase 7 is
    what may modulate them. `velocity_filter_sensitivity` is V1's own dead field, read by no
    V1 DSP, and has no destination here. The lowerer places a scaler between a voice's
    terminating node and the output, carrying the instrument's sensitivity, which is what
    lets `LOWER-INV-003` drop its velocity clause.

22. **SOUND-INV-022 — An observation tap is a declared, passive compiler artifact.**
    [ADR-0027](../decisions/ADR-0027-observation-and-analyzer-ownership.md) decides ownership
    and this states what the compiler and renderer must do. A tap names a stable signal point
    with a declared data type, rate and resource cost, and a node kind's declaration is its
    only source: a tap not declared there does not exist, and nothing may subscribe to a node's
    internals. **A declared tap exists in the compiled plan whether or not any subscriber
    exists**, so a headless compilation and one with every observation enabled produce the
    same plan and the same semantic digest; what a subscriber's absence may omit is the
    consumer-side capture and analysis, which the host owns under `HOST-INV-023`, never the
    tap or the node's downstream signal. A tap is passive: it changes no audio sample. The
    taps a plan declares are admitted at compilation against `max_observation_taps`, as
    `LIMIT-0020` already states, independently of subscribers. Analysis that would cost
    real-time budget — FFT, feature extraction, history — is not a tap; it exists only as an
    authored node that declares a bounded cost and passes every node's real-time gate.

    **Built by `P05-S008`.** The `Monitor` kind is the one declaration that carries a tap —
    a pass-through with one `TapSpec` on its output — and the compiled plan's tap table is
    derived from its nodes' declarations, addressed stably by node and port, admitted at
    compilation against `max_observation_taps` by the same count, and present whether or not
    anything subscribes; the authored `PlanDeclarations::taps` list is gone. ADR-0005
    clause 6's reader is real: a tapped region is live to the end of the quantum. The
    subscription over the tap is `HOST-INV-023`'s and waits for its consumer.

23. **SOUND-INV-023 — One modulation law per parameter, composed in the slot.**
    [ADR-0007](../decisions/ADR-0007-parameter-modulation-laws.md) decides the set and this
    states what implementation must do. A declaration names exactly one law for each
    addressable parameter, from the closed set the record lists with its arithmetic —
    normalized, bipolar, semitone, decibel and physical additive; multiplicative gain;
    thresholded boolean where explicitly supported; not-modulatable. The parameter slot
    composes the layers in the master plan's order — the stored base; an automation override
    that replaces the base; a controller layer that replaces likewise where declared; the
    modulation sum, each modulator `amount × source` in the law's units; the law's
    arithmetic; the type's clamp; then `SOUND-INV-024`'s segment — and a replacement layer
    replaces the base only, never the modulation. Modulation depth is stated by the edge in
    the law's units, never scaled inside a target. A write from any layer but the base to a
    not-modulatable parameter is refused at admission. `SOUND-INV-018`'s catch-up restores
    the last pre-destination `SetParameter` of every prepared target, and that value is an
    **override-layer** write: the slot takes it as its override and re-derives modulation
    from the modulators, never from the flattened value. **A kernel reads one resolved value and
    composes nothing**: `SOUND-INV-013`'s prohibition on a law-selecting kernel parameter
    extends to the law itself, and two native kinds cannot compose one law differently
    because neither composes.

    **Built by `P05-S007a`** for every layer that has a writer: the base, the override and
    the modulation sum, composed in `render::slot`, with the catalog presenting the law
    beside the unit. The modulation sum's producer is Phase 7's; until then a crate-private,
    test-only seam writes it so the composition is a tested fact. No declared parameter is
    not-modulatable and none declares a controller layer; both branches are stated here and
    the compiler builds the refusal — a not-modulatable control compiles to no slot — but
    neither is exercised by a declaration yet.

24. **SOUND-INV-024 — A parameter ramp is a linear segment in the slot.**
    [ADR-0006](../decisions/ADR-0006-parameter-ramp-representation.md) decides the shape and
    this states what implementation must do. A slot's runtime value is its current value, its
    target and the frames remaining. **The slot advances before the kernel reads**: on frame
    `k` of a segment of `N` frames the kernel reads `current + (target − current) × (k + 1) /
    N`, so the segment's last frame reads exactly the target — V1's own filter convention —
    and every later frame holds it; a segment with no frames remaining is a step read on its
    first frame. Its endpoints
    are `SOUND-INV-023`'s resolved values, and the segment composes nothing. A new write
    retargets from the **current** value — never from the previous target — over the
    duration the declaration's smoothing policy states for that parameter, and that policy is
    `None` for a gate and for every `ControlRate::Sample` destination, whose timing
    `SOUND-INV-016` owns. The representation is one whether automation, modulation or a
    caller retargeted it. A kernel reads one value per sample and never advances a segment;
    advancing is the slot's, in the loop, so the purity scan sees it once. **An activation
    never ramps**: `SOUND-INV-018`'s catch-up seeds the slot with current equal to target and
    no frames remaining, so the first frame the new mapping governs reads the restored value
    in force, as that invariant requires.

    **Built by `P05-S007b`.** The segment lives in `render::slot` beside the layers, is
    advanced once per quantum before the schedule walk into a per-slot control buffer, and
    a kernel reads its quantum-rate control from that buffer per frame — node state no
    longer carries one. Every declared policy is `Smoothing::None`, so every write is still
    a step and renders are unchanged; the segment's facts are tested through a test-only
    policy seam. `None` on the oscillator amplitude is V1 parity — V1 applies the level the
    lowerer maps there unsmoothed; the level V1 de-zippers per block is its amplifier's,
    which the lowerer refuses unless unity — and which parameter first declares
    `Smoothing::Quantum` is `P05-R001` in `NOW.md`, decided by the user on 2026-09-05.

25. **SOUND-INV-025 — A voice scope is one prepared shape and `N` instances of state.**
    The compiler instantiates the plan's voice scope once per **identity index** of the
    producers that play it — the sum of their `simultaneous_notes`, and one where no producer
    is declared — so every index SOUND-INV-017 can mint names a voice instance and a note-on
    that finds no free index is refused and counted there rather than stealing (`P06-S002`
    owns stealing, after its decision). **Prepared data is shared and cloned for no
    instance**: every instance of a voice-scope node is a scheduled step over the **one**
    prepared record, with its own node state, its own row of every writable control and its
    own output buffer. A node outside the scope is scheduled once. **Routing is by the
    identity index**: a note's gate and magnitudes land on the row of the instance its
    identity names, and on the one row of a control whose node has one instance — an
    instrument-scope envelope played beside a voice-scope one shares its row across the
    notes, as it did before. **A parameter write addresses the control, not an instance**: a
    `SetParameter` — a caller's, an automation lane's, a catch-up's — fans out over the
    control's rows, so opening a voice-scope gate by parameter opens every instance and a
    plan's addressable-parameter count is unchanged by `N`. **The voice sum is inserted
    work**: wherever a voice-scope output feeds a node outside the scope, and only there, the
    compiler copies instance 0's buffer into a sum region and accumulates each later instance
    into it in instance order, and the consumer reads the sum; a consumer inside the scope
    reads the same instance's buffer. **The output is scheduled once and is never a
    voice-scope node**: an `Output` declared in the voice scope is refused at validation,
    because lowered there it would read one instance and drop the rest without a word. **A
    release names the newest open note on its node with its key**: `CompiledPayload::NoteOff`
    carries the key so that two notes overlapping on one node resolve their releases to the
    notes that opened them; preparation pairs a release with the most recent unreleased
    note-on of that key on that node, and every walk that decides which side of a boundary a
    release's note-on lies on — an activation's history against its suffix, a loop's outside
    against its inside — keeps its depths by node **and key** under the same rule. **The
    report charges what preparation holds**: node state, parameter slots, ramp buffers, taps
    and the timed-control scratch scale with `N` — the scratch on the wider of a note-on's
    expansion and a write fanned out over the rows of the widest `ControlRate::Sample` group,
    which is `N` where the voice scope declares such a control and one where it does not —
    the arena's preflight bound counts every instance's region, and the prepared row does
    not; `max_active_voices` is admitted against the **derived** `N`, and a plan with no voice
    scope requests none. A tap row names its own instance's step.
    A plan with one identity index compiles to today's schedule exactly, which is what keeps
    every existing render bit-identical.

    **A full producer steals under the plan's declared policy, or refuses**
    ([ADR-0058](../decisions/ADR-0058-voice-allocation-and-stealing.md)). `PlanDeclarations`
    carries one `StealingPolicy` — `None`, the default and the refusal above; `Oldest`; or
    `SameNote` — consulted **only** when every admitted index of the producer is held: a free
    index is taken as SOUND-INV-017 states, and a releasing voice's index is free. Under
    `Oldest` the note whose note-on is earliest is taken; under `SameNote` the newest held note
    on the same node and key is retriggered at the new note's position with no fade and no reset —
    its gate falls and rises before one frame is written, so the envelope re-attacks from the
    level it stood at and no release is rendered — and failing one, the oldest is taken; under
    `Oldest` a taken note that shares the key still fades. **The taken voice
    fades, is reset, and the new note starts when the fade completes**: every voice-sum step of
    the taken instance scales its contribution by a linear ramp from one to zero over the
    declared `fade` frames from the new note's render position, then holds silence; at that
    position plus `fade` exactly, every step of the instance — each voice-scope node's and each
    sum's — is restored to its prepared state, and the new note's gate and magnitudes land as
    SOUND-INV-021 orders them. A stealing plan sums even a single voice, so the fade has a
    step. Two control indices are **reserved to the render loop** for this, a reset and a
    fade-out, which no declaration may use. **The taken note is ended** as an `EndedNote` is —
    its index freed, its generation advanced — and its own later release names a superseded
    generation: preparation drops that release as a named transformation and counts it as
    `released_after_steal`, apart from the orphan count that reports a producer defect. **A
    note that takes a voice by fade-then-start keeps its authored length**: its release is
    displaced by the same `fade` as its start, so a note shorter than the fade is not released
    before it begins. On the compiled path a release names a key, not an occurrence, so where
    the taking note shares the taken note's key — `SameNote`'s retrigger — the taken note's
    later release pairs with the newest open note of that key, the taking one, under this
    invariant's pairing rule; the taking note's own release is then the one dropped. **The
    choice is made off the audio thread**, by the one pairing authority every walk holds —
    stamping's two passes, an activation's history and suffix, a loop's repeating pass — and
    the loop receives the fade, the reset and the delayed note-on as timed events, so its work
    is the expansion it already does plus one multiply per frame on the fading instance. **The
    expansion is charged**: a reset's controls against the timed-control scratch beside a
    note-on's expansion and a write's fan-out, and the fade's and the new note's positions
    against the compiled share at preparation, which refuses a schedule whose steals overrun
    it. A plan whose voice scope declares `None` compiles to the schedule it had, bit for bit.

    **The live boundary steals the same way, and the renderer sees one shape.** A live note-on
    that finds every admitted index held takes a voice under the plan's policy at the offer,
    off the audio thread: the minter carries each live index's node, key and mint order and
    names the victim. The queue drains head-first and requires non-decreasing stamps, so the
    start `fade` frames on cannot be queued ahead of offers that follow: the fade is queued at
    the offer and the start waits outside the queue, one slot per identity index, until the
    drain's window reaches it, when it is published as the same reset and note-on the compiled
    path stamps; a release offered while the start is pending waits with it, displaced by the
    same `fade`. The taken note's own later release is counted at the boundary as
    `released_after_steal`, never queued and never an orphan. **A committed voice is not
    taken**, on either path: a voice whose deferred start has not yet happened, or whose
    displaced release is still to land, is ineligible, and a note-on that finds every voice so
    committed is an over-emission — refused at preparation on the compiled path, dropped with
    the identity named at the boundary — never a second note folded onto one instance.
    **Per-note expression follows the occurrence** (`P06-S003`): a bend is the occurrence's
    own layer on its instance's pitch rows, a new occurrence starts with that layer at identity
    whether it took the voice or found it free, a released note keeps its bend through its
    tail, and expression addressed to a note a steal ended is dropped and counted with that
    note's release. A
    note that took a voice keeps its index in the minter until its displaced release lands,
    not until its release is placed, so nothing mints onto a voice whose deferred start is
    still to come. At the boundary a taken voice becomes takeable once the drain has published
    its start, one drain after the fade; the compiled path knows the same fact from the list.
    **A hold goes with the voice**:
    where every hold is outstanding when the note-on arrives — a producer declares no more
    holds than notes, so a full producer holds every reservation — the taken note's hold
    passes to the note that takes it, because the release it reserved room for will not be
    queued; where a hold is free the new note takes its own and the taken note's is
    discharged when its release arrives. What waits outside the queue is charged against the
    queue's room as the entries it will publish.

    **Built by `P06-S001`, `P06-S002` and `P06-S002b`.** Instances are `Lowering::schedule`
    over one `PreparedSlot`, the voice sum is the `copy` and `accumulate` kernels, rows are
    `ParameterTarget::instances`, routing is `voice_row` in `render/hot.rs`, the catch-up is
    one row per address; stealing is `schedule::OpenNotes` and `stamp_all`, the fade is the
    sum kernels' `Fade`, the reset is each stateful kernel's `RESET` arm, and the instance
    groups are the plan's `instance_groups` and `sum_groups`; at the live boundary it is
    `IdentityTable::victim` and `PerformanceIngress::steal`, with the deferred start in the
    store's `pending` slots and `publish_pending` in the drain. The conformance row lists the
    falsifiers.

26. **SOUND-INV-026 — A sampler plays one zone of a prepared map, and the sample is held
    once.** [ADR-0026](../decisions/ADR-0026-minimum-sample-map-and-zone-model.md) decides
    the model and this states what implementation must do.

    **A `SampleZone` is the unit of mapping and a `SampleMap` an ordered list of zones.** A
    zone carries a key range and a velocity range, both inclusive; a root key; a fine tuning
    in cents; a playback region of a start and an exclusive end inside its sample; an
    optional loop inside that region; a gain; and the sample it plays, by reference into the
    plan's table. Every field is a validated type: an empty region, a loop outside its region
    and an inverted range are refused where they are built, and a region past its sample or
    a reference the plan does not hold is refused at IR construction, where the table is.

    **Phase 6 plays the one-zone subset and refuses the rest by name.** A map of two or more
    zones is refused at admission as `MapBeyondOneZone`, never played from its first; a
    direction other than `Forward` is refused as `DirectionNotBuilt`, never played forwards;
    a sample recorded at a rate other than the stream's is refused as `SampleRateMismatch`,
    never played at the stream's rate — V1's speed formula never read a source rate either,
    so the mismatch would play mis-pitched on both engines, and a rate ratio or a resampler
    is a decision with no consumer yet; and `Loop` over a zone declaring no loop is refused
    as `LoopWithoutRegion`, never played as `Sustain`.
    The types admit `N` zones and three directions so that the multi-zone slice extends the
    selection and changes no type. A note whose key or velocity falls outside the one zone's
    ranges plays nothing on the sampler and is counted, not refused.

    **A prepared sample is immutable PCM with its shape and digest, held once per plan.**
    Interleaved `f32` frames, one or two channels, a frame count, the source rate and an
    FNV-1a digest, prepared off the audio thread where an empty buffer, a ragged frame
    count and a non-finite value are refused. The plan's table holds one entry per
    **distinct** sample **a sampler node reaches** through its map, compared by the frames
    rather than by digest — a sample the IR holds and no sampler plays is neither prepared
    into the plan nor charged, which is the same set — and a `SampleSlot`
    references it as a `TuningSlot` references a tuning; the frames sit behind an `Arc`, so
    `N` voice instances and a cloned plan share one allocation. The report charges each
    distinct entry once to `prepared_immutable_bytes` and one slot per sampler node, over the
    same set admission binds, and the charge is exact because the preflight may refuse on
    that row. The persisted asset behind a prepared sample is Phase 10A's and 10D's; the
    prepared key it must meet is source digest plus preparation profile.

    **The sampler is a voice-scope source that starts on a trigger and is never a note's
    address.** It declares three sample-positioned destinations — trigger, pitch and
    velocity — and no `note_control`, so a scope holding a sampler and an envelope still
    holds one playable node and `SOUND-INV-021`'s refusal of two is untouched; a note
    naming the sampler itself resolves to no note slot. Its level and its velocity
    sensitivity are authored quantum-rate controls read per frame. One player state per
    voice instance — a position, a rate, a velocity, a phase, a fade count and the trigger's
    held state — is sized at preparation and reset by the on edge; nothing is allocated on a
    note, and the kernel is in the real-time region the purity scan covers.

    **The rate law is V1's, through the tuning.** The rate is the frequency the scope's
    prepared tuning resolves for the note's key, over the frequency the same tuning resolves
    for the zone's root, times `2^(fine_cents / 1200)`: the pitch destination carries the
    first, admission writes the second into the prepared record where the scope is known,
    and the kernel divides two numbers and converts no key. Under twelve-tone equal
    temperament the table's values are V1's `Hertz::from_midi` bit for bit, so the ratio is
    V1's `played / root` exactly.

    **Playback is V1's.** Two-tap linear interpolation with the second tap wrapped into the
    loop's start at the loop's end and clamped at the region's end; a stereo sample summed to
    mono as `(left + right) × 0.5` after each channel's read; the zone's gain and the level
    per frame; the velocity under `(1 − s) + s × v` with the sampler's own sensitivity
    (ADR-0059's rule, a third destination); a start offset seeking to
    `start + (end − start) × offset` when the offset is above V1's `0.001` threshold.
    `OneShot` plays the region out and ignores the off edge; `Sustain` plays once and fades
    linearly over V1's 512 frames from the off edge; `Loop` repeats the loop while the read
    continues, the fade included, as V1's player keeps looping through its release. A
    reset (ADR-0058) silences the sampler; a slot the plan does not resolve renders silence.

    **Built by `P06-S005`.** The types are `sample.rs`; the kind is `node::SAMPLER` and
    `prepare_sampler`, prepared against a `PrepareContext` that carries the plan's resolved
    sample slots; the table is `CompiledPlan::prepared_samples` and `SampleSlot`; the
    trigger is `NoteMagnitude::Trigger`, written on the on edge with the magnitudes and on
    the off edge through `DueEvent::note_of`; the kernel is `kernels::sampler`. The
    conformance row lists the falsifiers.

## Types and ownership

A compiled plan is immutable. Each renderer owns its mutable node and stream
state, preallocated arena, carries, and parameter values. The experimental
crate does not own GUI, MCP, filesystem, device, project-load, or V1 mutation
behavior. No production crate depends on it while it remains the deletable
migration boundary.

## Lifecycle and timing

The caller supplies immutable host capabilities and render limits. The compiler
returns either a compiled plan plus resource report or a structured diagnostic
with the report accumulated to the refusal. A renderer binds one plan to its
own preallocated arena, carries, parameter values, and node state.

## Failure and diagnostics

Invalid graph or resource input fails at compilation. A caller span that
violates an admitted render precondition returns `RenderError`; a debug-only
assertion is not release behavior. The last valid output/carry state is handled
according to the relevant accepted render-quantum clause rather than silently
fabricating success.

## Real-time and resource constraints

Compilation and preparation may allocate off the audio thread. Rendering uses
only resources admitted by the immutable plan and performs no allocation,
blocking lock, I/O, logging, graph discovery, or capacity growth. Resource
excess is a preparation refusal rather than runtime truncation.

## Conformance tests

| Invariants | Named checks |
|---|---|
| SOUND-INV-001, 004 | `admission`, `crate_boundary` |
| SOUND-INV-002 | `render_allocation`, `render_loop_purity` |
| SOUND-INV-003 | `node_representation` |
| SOUND-INV-005, 006 | `render_contract` |
| SOUND-INV-007 | `graph_validation` |
| SOUND-INV-008 | `lowering` |
| SOUND-INV-011 | `arena_reuse` |
| SOUND-INV-012 | Three source scans in `render_loop_purity`, each mutation-verified: `the_render_loop_makes_no_topology_or_naming_decision` forbids `IrNodeKind` anywhere in the real-time region, so the loop cannot branch on a kind; `the_render_loop_dispatches_every_node_through_one_site` holds the hot path to exactly one `Kernel::run` site and no kernel called by name, so a kind cannot be reached by a second path; and `the_kernel_registry_is_closed_and_no_scanned_form_forges_a_kernel` keeps the callee set enumerable. The admission half — that a kind's facts are resolved once, from one place — is `P05-S001`'s `NodeDeclaration`: `a_declared_kinds_registry_facts_derive_from_its_declaration` reads the sawtooth's descriptor, ports and byte attributions back through the registry and finds the declaration's values, and `a_declared_kind_appears_in_the_registry_only_by_deferring_to_its_declaration` holds every arm of a declared kind to a deferring form. Every kind but the output node is declared — the output node has no kernel and is the renderer's boundary rather than a node's work — and the scan takes a stated list of declared kinds so a kind added to `declaration` behind its back fails. Preparation is the declaration's too: each names the function that builds its prepared record from its IR fields, the registry's `prepare` forwards, and a declaration handed another kind's IR is refused rather than rendered as silence. Discovery derives from the same value: `node::catalog()` presents every declared kind's ports and parameters — name, unit, default, rate, note magnitude — and `discovery_is_derived_from_the_declarations_admission_reads` and `discovery_and_validation_describe_the_same_ports` hold it to the declaration and to `ports()` |
| SOUND-INV-016 | `note_events`, over the compiled path, and the envelope kernel's own edge tests in `kernels`. **The clamped branch is covered separately**, because `note_events` runs offline where a late event cannot be presented: `render_contract`'s `a_late_note_edge_takes_effect_at_its_clamped_render_position` drives a late note-on and an on-time note-off through one live render and asserts an exact value per frame for both, so an edge dropped, held to a boundary, or applied at the head of the call fails it — a one-frame error is mutation-verified to fail. The count-only `a_late_event_is_clamped_forward_and_counted` cannot see any of that. `note_events` places each edge at an offset that is **not** a multiple of `Q` and asserts an exact value per frame, so an edge quantized to a boundary fails it; it renders the same edge through three host-block partitions and through both payloads and requires all of them bit-identical; and `an_edge_mid_ramp_starts_from_the_level_that_frame_would_have_had` covers the case the instantaneous-segment fixtures cannot see, where the level a frame starts from and the sample before it differ by one step. `layout_baseline` cannot cover any of this — every edge in its fixtures is on a boundary — so it is a regression control here rather than a placement check |
| SOUND-INV-013 | Two mechanisms, and what each one carries is stated rather than blurred. **The type system** enforces exactly one thing: a `Kernel` cannot be *constructed outside* `node::kernels`, its field being private — so a descriptor elsewhere naming any function does not compile (`E0423`, mutation-verified). Every descriptor lives in `node.rs`, so every registered pointer is necessarily one of that module's constants; what those constants wrap is not settled by privacy, and an in-module `Kernel(foreign)` is well-typed. **A bounded source scan** carries the rest — `render_loop_purity`'s `the_kernel_registry_is_closed_and_no_scanned_form_forges_a_kernel` requires every construction site it recognises in that one file to be a declared constant, and checks the entries and constants it can parse agree in both directions. Nine forging routes are mutation-checked. What the scan is not is under *Unresolved questions* |
| SOUND-INV-009, 010, 014 | `layout_baseline` for ADR-0041 clause 16's per-quantum digest comparison over its five fixtures, `arena_reuse` for the structural check over physical sample ranges and for `reuse_renders_bit_identically_to_no_reuse`, and `lowering`'s `a_mono_source_into_a_stereo_stream_widens_into_one_wider_region`, `a_mono_stream_compiles_exactly_one_output_operation` and `an_inserted_conversion_is_reported_and_not_only_scheduled` |
| SOUND-INV-015 | `graph_validation`'s `every_kernel_admits_exactly_one_channel_on_every_port` over the macro-generated catalog, and `kernels`' `the_widening_writes_every_channel_of_every_frame` for the one kernel with two admitted counts |
| SOUND-INV-017 | Two files, and the split is the invariant's own. **The minting table** is covered by `identity`: the three orphan branches separately — a free index, a live index at a superseded generation, and a retired one — along with disjoint producer ranges asserted by minting each producer's **whole** range rather than one index, the foreign-table resolution, counted retirement, over-emission distinguished from retirement erosion, a scoped mass release that leaves other producers alone, the refusal to rebuild while an obligation is outstanding, and the index-space relation at its boundary — enforced at `HostProfile` construction, as the invariant requires, not only where a table is built. Three are mutation-verified: resolving without comparing the generation, restarting a generation instead of retiring the index, and overlapping the producer ranges. **The event side** is covered by `note_identity`, over the compiled path. A release carries no node, so the fixture is two gates in series whose releases differ in *shape* — one instantaneous, one a ramp — because two sustain levels would render the same product and hide a release resolved to the wrong note. Sixteen cases: a release resolved through its occurrence; the same list with the other release rendering the other shape; two releases interleaved so the first names the **older** outstanding note; a producer's range bounding its *polyphony* rather than a piece's note count, with a one-note producer playing eight notes in sequence, refusing two at once, and stamping a valid list afterwards — a mint that failed part-way through having left nothing reserved; a reissued index still resolving **both** of the notes that used it, which is why the renderer keeps a registry the events write rather than reading the minter; a refused list leaving the minter as it found it, whose falsifier is the valid retry that follows; a spent release replayed against a node that has since been replayed, moving nothing and counted as an orphan rather than silently skipped, with the report **naming the occurrence** it refused; an occurrence from another plan's table refused and counted with the epoch equal, so the stale-epoch filter cannot be what caught it; a node address from another plan refused at stamping, which the renderer can no longer catch because its foreign filter compares the occurrence's table; the stamp carrying the renderer's own epoch; a compiled release that opens nothing refused at stamping; a plan declaring no producer unable to stamp a note at all; a second compiled producer refused at admission; and the compiled producer looked up rather than assumed to be producer 0, checked by declaring a one-wide runtime producer first so an assumed producer 0 exhausts on its second mint. `admission` adds the identity halves' memory to the scratch budget. Fifteen properties are mutation-verified: pairing by recency, resolving without the generation, dropping the table comparison, dropping the second-compiled-producer refusal, swallowing an unmatched release, assuming producer 0, never freeing a released occurrence, dropping the note-slot provenance check, not freeing an index at the paired release, resolving a release through the minter instead of the registry, minting during validation so a refused list keeps its indices, committing a partial mint instead of discarding it, refusing an orphan without counting it, refusing an orphan without naming it, and leaving the identity halves out of the scratch budget. **The live half is now covered in part**, by `simulated_ingress`: a live producer acquires its queue slot, release hold and identity together, the hold is spent when the release takes the slot it reserved and not a second time at publication, and a note-on is refused when the queue cannot also hold the release it would owe — that reservation arithmetic mutation-verified in both directions. The hold's exhaustion is asserted on its own, with six identities still free; **identity exhaustion is not asserted and cannot be**, because admission refuses a producer owing more releases than it can sound notes, so with `holds <= notes` the hold is always reached first, and the only remaining route is a generation space no test can walk. That conclusion holds only because a store is bound to the plan its renderer renders: a store carrying one plan's eight-hold entitlement while minting into another's two-index range makes the shortage reachable although each plan separately satisfies the relation, and an independent review found the binding missing. And clause 4's orphan attribution is carried by one named identity plus an aggregate count rather than by per-producer counts. **Naming the identity does name the producer**, since the ranges are disjoint by construction, so an index falls in exactly one. **The aggregate count is what cannot be attributed**: the renderer records the most recent orphan's identity and counts the rest, so two producers orphaning in one call are reported as one identity and a total. That is the gap this row records, and it is a property of the report's shape rather than of how many producers emit — two minting paths now exist, `stamp_compiled` and the live boundary's `StreamControl::offer_note_on`. Three earlier revisions justified the gap by the producer count, then by there being a single emitter, then by the disjoint ranges; independent reviews caught all three, the last because ranges identify the named occurrence and not the count beside it. The situation is not reachable here yet, and by a check rather than a convention: a live ingress store refuses a plan that also declares a compiled producer, which is how ADR-0051 clause 6's boundary is enforced while a gate reached by more than one has no ownership law. Plans declaring both exist in this crate's fixtures and are harmless, because none builds a store and without one a non-compiled declaration cannot emit. The generation ceiling remains a construction parameter for the reason this row gave before: walking a `u32` by minting is unreachable, and a rule no test can reach is a rule nobody has checked |
| SOUND-INV-018 | `transport_activation.rs` and `transport.rs`: the effective point invariant to the four host partitions `2048`, `256`, `64` and `37`; a superseded candidate refused and counted; a stale-epoch candidate refused; both exchange occupancies refusing and told apart; an offer paired with another stream's renderer touching no counter; an offer into a faulted stream refused rather than trapping the candidate; a note cut at the boundary, asserted per frame on both sides; a seek between a note's edges buildable with the omission counted; a release with no note-on at all refused; a history note edge needing a producer, and a history holding more notes than that producer admits refused; an activation needing a schedule to replace; the schedule in force releasing its index to the replacement, and a withdrawn candidate's reservations released, both falsified by over-emission at a partition of one; every committed stamping keeping its reservations reclaimable; the minter standing still while a candidate holds a snapshot of it; a stream having one schedule and a second refused; a loop whose periodic extension does not fit refused at the build, and the same pass judged a second time against the compiled producer's admitted simultaneous notes, falsified by a late entry whose history and suffix each hold what the producer admits while the pass they straddle holds one more, by a pass that inherits the depth open where the loop starts, and by a crossing release lowering a count it ends nothing of; the pass that is judged being the one a wrap replays rather than the suffix the candidate carries, falsified by a late entry whose skipped events are the ones that collide at the wrap; and a crossing release counting in that pass, because clause 5 keeps its gate write where the bare omission dropped the event; the release scope being the compiled producer alone, and an empty producer declared before a sounding one not hiding the compaction; an adoption never happening in a call that renders no quantum; the boundary mass release charged as one session operation; a refused call adopting nothing when the boundary is the clock; a block the renderer cannot serve adopting nothing; a fault in **each** half of a split silencing the whole callback; a late activation taking effect at the clock and counted, an off-grid request that snaps forward without being late, and a late one displaced by nothing; a loop-bearing candidate passing off-thread admission but being refused at offer with its interval and counter asserted and active loop state absent from both halves by construction; the retired schedule coming back with the cursor, anchor and displacement it replaced; a retirement from another stream refused rather than promoted; withdrawal refusing a foreign candidate and a retirement and returning both; the catch-up restoring every prepared target with the boundary release charged beside it; a crossing note's gate cut whether or not automation touched it after its note-on, the equality falsifying a last-write restore and the silence falsifying no substitution at all, mutation-verified against both; and an omitted crossing release carrying its gate-down in place of the note contract, counted, with the pair that lies wholly after the destination placed untouched beside it. **Two parts are not covered, and neither is deferred silently**: a live ingress note left sounding across an activation needs a producer that can emit, which ADR-0050 clause 8 puts out of scope; and the ideal-phase law for runtime wraps waits behind ADR-0055's refusal and belongs to ADR-0052's first executable consumer |
| SOUND-INV-020 | `simulated_ingress.rs`, over the one pair of producers that exists, and the coverage is narrower than the rule. **The decisive case is a note and its release at one render position**: applied in the order they were offered the gate rises and falls at one sample and nothing sounds, while any order derived from ADR-0046's capacity classes applies the release first, refuses it as an orphan, and leaves the note sounding to the end. A mutation letting a release jump ahead of its own note-on fails that test and no other. The equivalence render is a weaker check than it looks and is cited as such: its two edges sit at different samples, so position sorting restores their order however they were presented — an independent review refuted an earlier claim that it proved release ordering. What it does establish is placement, falsified by displacing every ingress stamp one frame. Beside them: an entry whose destination is past the window waits and later lands on its own sample; and the ring wraps across three blocks with one alternating gate write per quantum, so the rendered square wave falsifies a **lost** entry while a **duplicated** one — idempotent in the audio, since one write charged twice into its own quantum wins the same way — is caught by the live share's peak instead. Both mutations were run, and an earlier version of that fixture wrote the same value from every slot and could see neither. **The rest of the declared sequence is not covered.** Authored runtime and renderer-internal producers do not exist, transport stop and panic have no producer, and the declaration-order rule needs two producers **emitting** into one position, which no plan can do until ADR-0051 clause 6 supplies a gate-ownership law — a live store refuses a plan that also declares a compiled producer, so the combination is refused rather than merely absent. Contiguity has no fixture for the same reason. Those cases arrive with the producers they need, and no test notices a producer nobody wrote a case for |
| SOUND-INV-021 | **Built end to end for the key, velocity and bend clauses.** The boundary types and the prepared tuning are covered by `tuning_tests` — totality over the key range, refusal of an out-of-range key rather than the clamp `synth_core` performs, determinism and a digest that separates two scales, a non-12-tone scale preparing, and the definedness gap recorded as a measurement. **The declarations and the binding** are covered by `note_magnitudes.rs`: a note's key reaching an oscillator the note did not name, asserted by comparing node slots rather than by counting — an expansion writing both magnitudes to the played node has the same length; the same oscillator moved to another scope contributing no magnitude, which is the falsifier for "collect every pitch destination in the plan"; a pitch destination whose scope states no tuning refused by name; two playable nodes in one scope refused with **both** named, beside the same two in two scopes admitted, which is what makes the shared scope rather than the second note target the thing refused; two scopes naming one scale sharing one prepared table and two scales being two; a second scale costing exactly one table in the report's prepared row; a scope given two tunings refused rather than taking the last; and the destination's rate read back from the compiled parameter target, so a caller cannot obtain another timing by choosing another payload. `node.rs`'s `descriptor_destinations_are_sample_positioned` holds the rate and the declaration together for every kind, and `a_playable_kind_declares_no_magnitude_twice` refuses a kind naming one control twice. **The payload reaching the audio** is covered by the same file, and each check is a ratio rather than an inequality, because velocity reaching *something* is not velocity reaching the amplitude: a quarter velocity renders a quarter of the peak; an octave renders twice the zero crossings; and one key under twelve-tone and nineteen-tone renders two pitches, asserted both in the audio and exactly at the plan, since two similar frequencies could cross zero the same number of times. `a_notes_magnitudes_are_in_force_on_the_sample_its_gate_rises` places the note at an offset that is not a multiple of the quantum and reads the ratio over its **own first quantum**, so a magnitude arriving late is caught where the tail would still be right. `a_locate_restores_the_pitch_the_last_note_before_it_carried` covers ADR-0051 clause 1 over the magnitudes: two seeks past two different notes, with automation opening the gate at the destination because ADR-0050 clause 5 cuts every crossing note's own. `admission`'s `admission_stays_linear_in_the_node_count` guards how the two per-scope report figures are *computed*: the obvious form asks each node whether its scope holds a playable node, which is a scan inside a scan and runs twice per compile — and an oversized plan pays it before the `max_nodes` refusal. Measured at 4 096 nodes in release: 479 ms quadratic against 4.5 ms linear, so the test's 400 ms ceiling is two orders of magnitude of headroom rather than a timing comparison. `render_scratch` charges the expansion, over **both** plan shapes — a silent plan writes one control per event and a real voice writes three — and compares the IR figure admission charges against what the plan expands to, so an under-charged scratch fails there rather than by being overrun. Mutations run and caught: a quantum-rate pitch destination; collecting from every node rather than from the scope; sharing one table for two distinct scales; comparing the node instead of the scope in the ambiguity check; dropping the expansion; displacing the magnitudes by one frame; and neutralising the catch-up's magnitude restoration. **The bend clause was built by `P06-S003`**, held by `voice_tests`: `a_bend_moves_one_occurrence_by_its_cents_under_the_semitone_law` (a +100-cent bend on one voice renders exactly what a sample-positioned write of the frequency the law resolves for one semitone renders — the law's own figure, reached by two paths — and differs from the unbent render), `a_bend_reaches_only_the_occurrence_it_names` (two voices, the bent note at half velocity: the render is the bent note alone plus the other alone, so neither the other voice nor the velocity row moved), `a_new_occurrence_on_a_voice_starts_unbent` (the next note on a bent-then-released voice reads its own key's frequency from the oscillator's state), `a_bend_for_a_note_a_steal_ended_is_dropped_and_counted_with_its_release`, `a_bend_naming_no_open_note_is_refused_at_preparation`, `a_bend_of_a_note_that_took_a_voice_is_displaced_with_its_start`; the modulator-in-force case by `a_sample_positioned_write_and_a_notes_pitch_both_reach_the_kernel_composed`, which a single-layer first draft failed; `simulated_ingress`'s `a_live_bend_moves_the_note_it_names_as_the_compiled_one_does` and `a_live_bend_of_a_note_whose_start_is_deferred_waits_with_it` (live and compiled bends render the same samples on a pitched voice, and differ from the unbent render) and `a_live_bend_naming_a_note_the_producer_does_not_hold_is_refused_as_an_orphan`; and `transport_activation`'s `a_bend_of_a_note_opened_before_the_anchor_is_omitted_and_counted`. Five mutations run and caught: a note-on keeping the previous occupant's bend; a bend landing on every row of the note; a bend written as an override rather than the occurrence's layer; a deferred live bend never published; an omitted bend not counted. **One independent read (agy, `gemini-3.8-flash-high`, over the source and over the tests separately; codex out of quota) found one defect and four lesser points, all repaired**: the loop's passes resolved a bend's note from the live registry at pass time, after the same call's releases had been applied, so a bend followed in one call by its note's release moved nothing — the node is resolved once with the target and carried on the due event; a bend waiting with a deferred start took no room and could fail at publication — it is charged as the entry it publishes and refused for want of room; a waiting bend was published without regard to whether its start had been — it waits for the start; a displaced time the clock cannot hold was refused as an orphan — it is refused as past the horizon; and a taken note's bend was counted with its release — it has its own count. The test read found a tautological assertion in the orphan-bend test and a stale heading on this row, both corrected. **Two properties are not covered, and neither is deferred silently.** The order of the two writes *within* one offset is unfalsifiable here — measured, not assumed: moving the magnitudes after the gate fails no test, because a kernel applies every control due at a frame before it writes that frame and the envelope's gate law does not read the velocity. The order is kept because it is the one that stays correct if a kernel ever does. And a digest collision is not constructible, so the content comparison carries that case structurally. **The velocity refusal is unreachable here and says so**: the one playable kind declares a velocity destination, so a scope with a playable node always has one, and `the_velocity_refusal_has_no_constructible_case_in_this_phase` records that rather than leaving an untested branch unexplained. **The lowerer carries a project's own magnitudes**, covered in `pertylizer`'s lowering tests: two songs differing in one field render an octave apart and half a velocity renders half the peak — ratios rather than inequalities, so a lowerer sending a constant would fail them, which is mutation-verified; a placement's transpose is *applied* rather than reported and renders what authoring the transposed pitch renders, also mutation-verified; a transpose that leaves the keyboard falls back to the **authored** pitch, which is what V1's own `sequencer_engine::make_pending_note` does — an earlier revision refused the whole performance on the belief that V1 dropped such a note, and an independent review read the V1 site and refuted it; a persisted transpose that is not a keyboard offset is refused by name **and by value**, before the arithmetic rather than after, because `Semitones` is a transparent `f32` and an infinity overflows `Pitch::transpose` into a panic — mutation-verified against that panic; a saved velocity that is not a number is refused, while an out-of-range one is **not** caught here and the row says so, because `synth_core::Velocity` clamps at deserialization and the substitution happens in the project format's own type; and both pinned corpus cases now render from their own bytes, which is the work list's precondition — "before rendering the first saved pitched note, close P03-R003 with minimum typed pitch and velocity payload semantics" — discharged. What that does **not** close is a parity claim: V1 applies one saved velocity twice, at the envelope's own sensitivity and again at voice output, and V2 applies it as one scale on the envelope, so every arrangement placing a note is marked `UnsupportedScope` and the A/B path refuses to compare it. The composition is Phase 6's, and the work list is explicit that closing `P03-R003` does not decide it; Phase 4 exits carrying it as the named residual `P04-R001`, so Phase 6 inherits it before it builds the law. Four defects an independent review found are covered by their own regressions: a velocity written through the *parameter* path could invert or over-amplify the envelope, so the state is a typed `NoteVelocity` written through its **saturating** constructor — a second constructor beside the refusing one, so the type owns the policy rather than a comment at the assignment — and `2.0`, `-1.0` and `0.25` are asserted to render `1.0`, `0.0` and `0.25`, with the note payload never reaching it because a payload velocity is built through `NoteVelocity::new`; the report's tuning charge counted scopes no note reaches, which the preflight is allowed to **refuse** on, so the charge is computed over the scopes holding a playable node — one function serving both the preflight and the exact report — and `a_plan_is_not_refused_for_a_tuning_no_note_reaches` reproduces the false refusal under a prepared-byte limit between the exact charge and the former bound; the plan's tables are deduplicated by content rather than by digest; and the magnitude range is a typed pair with private fields, so the start-plus-length arithmetic exists once inside `CompiledPlan::note_magnitudes_of` — which takes a `NoteSlot` rather than a bare target, so a slot from another plan is refused instead of yielding an in-bounds slice of unrelated entries. **The composition clause was built by `P06-S004`**, held by `voice_tests`: `the_envelope_scales_its_level_by_v1s_sensitivity_law` (a voice with the envelope's sensitivity at a half and no scaler renders, at half velocity, exactly `1 − 0.5 × (1 − 0.5)` times the full-velocity render, sample for sample, and at a sensitivity of one exactly half) and `a_velocity_scaler_applies_v1s_output_law_after_the_envelope` (the same voice with a scaler at V1's default renders, at half velocity, exactly a quarter of the full-velocity render — the product — and with the scaler's sensitivity at zero the envelope's factor alone). Both are exact comparisons of two renders rather than tolerances, because the formulas are V1's bit for bit and a rounding would be a defect. `node_representation` holds the scaler as the eleventh declared kind with its two controls, and `descriptor_destinations_are_sample_positioned` holds its velocity destination to the sample rate as it holds the envelope's. Bit-identical: EVD-0013's aligned render and the three `quantum_cost` digests reproduce, because every existing fixture authors the envelope's sensitivity at one, where `1 − 1 × (1 − v)` is `v` exactly. **The lowerer's half** is in `pertylizer`'s lowering tests: `v1s_two_sensitivities_compose_to_the_velocity_squared` renders one saved instrument at V1's defaults at full and half velocity and asserts the peak ratio a quarter, which a single scale would fail at a half; `a_placed_note_names_no_velocity_gap_and_still_refuses_a_parity_verdict` and `a_saved_instrument_and_song_render_through_v2_and_are_audible` assert no diagnostic names the composition, over one note and over four, while the first also asserts the outcome still refuses a parity verdict through Phase 8's marks; `the_envelopes_own_sensitivity_lowers_from_vel_sens` authors the envelope's `vel_sens` at zero, with the fixture's amplitude sensitivity also at zero so the scaler is unity, and asserts half the velocity renders the same peak, which a lowerer sending V1's default fails; and `a_saved_notes_own_velocity_reaches_the_render` keeps its half-velocity-half-peak ratio by authoring the fixture's amplitude sensitivity at zero. Four mutations run and caught: the envelope ignoring its sensitivity; the scaler dropping its unity term; the lowerer placing no scaler; and the lowerer sending V1's default in place of the saved `vel_sens`. The earlier residual — Phase 4's `P04-R001`, that V1 applies one saved velocity twice and V2 once — is closed by this. **One tuning through every path is held by `P06-S006`** (`tests/tuning_paths.rs`, with `common::nineteen_tet`): `the_sequenced_offline_and_live_paths_render_one_key_the_same_under_nineteen_tet` (the compiled stream, the offline render over the same events — one quantum of carry apart, as the offline path trims the priming quantum — and the live boundary offering the same edges render key 72 as the same samples under nineteen-tone; the render differs from twelve-tone's; and two keys under nineteen-tone resolve to two frequencies **at the plan**, through `magnitude_value`, and render two pitches over one span — the control a first draft lacked, whose two renders spanned different lengths and crossed zero the same number of times by coincidence, and the one that catches a resolution ignoring the key, since the two tables differ at every key including the reference); `the_observation_tap_reads_the_pitch_the_output_carries_under_nineteen_tet` (the analysis-facing path: a tap on the voice's output, read block by block with nothing dropped, holds exactly the output's samples one quantum of carry earlier, and reads two pitches under the two tunings); `a_locate_restores_the_pitch_through_the_tuning_the_plan_states` (the activation catch-up under the two tunings restores two pitches, and the nineteen-tone one crosses zero at the straight render's rate); `simulated_ingress`' `a_live_note_under_nineteen_tet_reaches_the_same_samples_as_the_compiled_one`; and `sampler_tests`' `the_rate_is_the_scopes_tunings_ratio_under_a_non_twelve_tone_scale` (the sampler's rate is the nineteen-tone table's ratio for key 72 over the root, not an octave). The lowerer states twelve-tone for every saved project, because that is all V1 plays, and per-project selection is Phase 10A's authored model. Mutations run and caught: the catch-up restoring no pitch; the live boundary dropping the offered key; every key resolving to the reference; and the sampler's root resolving through twelve-tone whatever the scope states. **One independent read (agy, `gemini-3.8-flash-high`; codex out of quota) found one real point and four lesser ones, all repaired**: the seek test's name and comments credited the boundary release with lowering the sampler's trigger while `SOUND-INV-026`'s row credits the catch-up's restore — the test is renamed for the behaviour it measures and its comments say which mechanism holds; a helper's doc comment had been split from its function; an assertion message counted twelve steps as nineteen; a doc comment placed a release after the destination it precedes; and two two-tuning comparisons did not first hold the twelve-tone render audible. **One finding is not a defect**: that the tap and the output have equal lengths and the carry assertion must panic — measured, the tap holds one quantum fewer frames than the output, and the test passes as written |
| SOUND-INV-022 | **Built by `P05-S008`; the subscription is `HOST-INV-023`'s and owed to its consumer.** `NodeDeclaration::taps` is the single source: `only_the_monitor_declares_a_tap_and_a_tap_names_an_output_port` holds every kind to no tap but the monitor, the monitor to exactly one on an output port carrying audio, and the catalog's taps to the declaration's. **The plan derives its table from the declarations**: `a_tap_exists_only_through_a_declaration_and_names_the_node_and_port` compiles the same voice with and without a monitor — one tap against none — resolves it by node and port, refuses a node whose kind declares none and a port the monitor declares none on, and reads the tap's region back as the monitor's own scheduled output; its cost is one quantum of the port's layout. **Admitted by count from the same walk**: `admission`'s refusal case for `max_observation_taps` is two monitors against a profile allowing one, refused naming both counts, and the `MaxObservationTaps` row's requested figure is what the table holds (`a_tapped_signal_stays_live_to_the_end_of_the_quantum`). **Passive**: `a_monitor_is_passive_and_its_tap_reads_the_signal_that_passed_through` renders the monitored voice bit-identical to the unmonitored one, and reads the tapped region after a render as exactly the last quantum the caller received, with the following quantum as the control that this is a signal point and not a period; `the_monitor_kernel_passes_its_input_through_in_every_input_state` holds the kernel's patched, in-place and unpatched branches. **ADR-0005 clause 6 has its case**: the arena pins a tapped virtual slot to the end of the schedule, so no later operation writes it and no in-place merge takes it over; the test's control re-runs the assignment unpinned and shows a later gain *would* have reused the region. **The exit gate's fifth bullet** — the same project compiles headless and with observation enabled to the same plan — holds structurally: compilation takes no observation input at all, a tap's presence depends on a declaration alone, and there is no subscription surface to vary. Mutations run and caught: the arena not pinning; the monitor writing silence; the compiler ignoring declared taps; the admitted count not the declarations'; the tap region not remapped through the arena; the catalog presenting no taps; the monitor declaring none; `resolve_tap` ignoring the port. Bit-identical: EVD-0013's aligned render and the three `quantum_cost` digests reproduce. **One independent read found two defects, both repaired**: the exhaustive kind fixture behind the registry test omitted the monitor, so the new kind's preparation, byte accounting, ports and forwarding were never exercised there — it is in the fixture and the declared count is ten; and the tap's byte rate was a raw `u64` on a public row — it is a `QuantumBytes` newtype, a rate its own type so a subscription's admission cannot take it for a byte total. **The subscription over the tap is built by `P05-S009`** (`HOST-INV-023`'s row): the reachable half — admission against the plan's taps, the bounded lossy ring, the per-quantum push after the schedule walk — with the live contract Phase 9's to verify; `PreparedRenderer::tap_block` remains a test-only read of the region itself. The declaration's latency, tail, reset and execution-scope fields are not built: no consumer in this phase reads one, and each is owed to the first that does |
| SOUND-INV-023 | **Built by `P05-S007a`, with the modulation layer's producer owed to Phase 7.** `node::ModulationLaw` is the closed set, one per `ControlSpec`, and `a_declared_control_pairs_its_law_with_its_unit` holds every declared control to the record's pairing — a frequency semitone-additive, a linear amplitude decibel-additive, a level normalized-additive, a gate thresholded because the envelope explicitly supports it — and every note destination to a law that admits a write; `discovery_is_derived_from_the_declarations_admission_reads` holds the catalog's law to the declaration's. **The arithmetic** is `ModulationLaw::resolve` in `render::slot`, and `each_law_resolves_as_adr_0007_states_it` checks every law against the record's figure with a neighbouring law giving a different answer; `every_law_resolves_to_its_base_at_its_identity` is the property the bit-identical renders rest on — `2^0` and `10^0` are exactly one — with the one bit an additive law loses, a negative zero's sign, recorded rather than claimed away. **The composition** is `SlotState`: base, override and modulation, the law, the type's clamp, then `ParameterValue::saturating` for the exponential laws' overflow; `an_override_replaces_the_base_and_leaves_the_modulation_in_force` is clause 2's last sentence on the slot alone. **Every write path reaches a kernel through the slot**, each with its own falsifier in `parameter_slot_tests`: a quantum-rate `apply` (an amplitude override under −20 dB renders a fifth of the unmodulated peak, not one), a sample-positioned `SetParameter` and a note's key through the magnitude expansion (both land exactly doubled under +12 st, read from the oscillator's state), an adopted activation's gate-downs through the same `push_timed_control`, and **the catch-up as an override-layer write** — `an_activations_catch_up_is_an_override_write_and_keeps_the_modulation_in_force` seeks past a note under +12 st and reads the restored A4 doubled, which a catch-up writing the flattened figure into state would fail. The gate's law is checked at the kernel: a caller's 0.3 releases and 0.6 holds, and what the envelope holds is exactly the boolean. **A kernel composes nothing, by scan**: `a_kernel_composes_nothing_and_the_law_is_applied_in_one_place` refuses the law, the slot, the composition call and both exponentials in `node/kernels.rs`, with the slot file as the control that those names exist, and walks the crate for exactly one definition of the law's arithmetic; the slot module is in the purity scan's region, `exp2`, `powf` and `clamp` are on its allowlist with their reasons, and every `clamp` in the region is held to two literal bounds. **Refusal at admission** is the compiler giving a not-modulatable control no slot and no address, and a note destination on such a control refusing the plan by name (`DestinationWithoutSlot`); no declared control is not-modulatable, so that branch is built but unexercised and the declaration test is what keeps it unreachable. Mutations run and caught: `apply` handing state the caller's value; `push_timed_control` bypassing the slot; an override that does not land; a prepared slot starting at zero rather than its identity; the catch-up falling back to zero rather than the row's base; the compiled base being the declared default rather than the prepared value; the semitone law dividing by six; the gate declared not-modulatable; a kernel calling `exp2`; the catalog presenting a fixed law; a note's gate and a magnitude each resolving to the neighbouring slot. Bit-identical after the rewiring: EVD-0013's aligned render and all three `quantum_cost` digests reproduce, which is the identity property above seen end to end. **One independent read found four defects, all repaired**: the slots the renderer allocates were not in `mutable_state_bytes`, so a profile limit between the reported and the actual figure admitted a plan that allocated past it — `slot_payload_bytes` now charges one `SlotState` per writable control into that row, and `the_slots_the_renderer_holds_are_charged_to_the_mutable_state_row` holds the row to one record per scheduled record plus exactly what preparation holds; the note-destination binder resolved each destination by a search over the target table, quadratic in the node count for a scope of many pitch destinations and run before the exact budget refusal — it now indexes the table once, and a node's own note control is found among its own slots; the catch-up share counted every declared control while a not-modulatable one compiles to no target, overstating the batch — the count now applies the same `admits_writes` filter; and the slot held its base, override and sum as raw floats, erasing the parameter value's finite invariant and the sum's unit — they are a `ParameterValue`, an `Option<ParameterValue>` and a `ModulationSum` newtype now, with the raw arithmetic confined to `resolve`. **Owed**: the modulation sum's producer (Phase 7), a controller layer (no declaration names one), and `SOUND-INV-024`'s segment (`P05-S007b`) |
| SOUND-INV-024 | **Built by `P05-S007b`, with every declared policy `None`.** `Smoothing` is the per-parameter policy on `ControlSpec`, `None` or one quantum; `a_declared_control_pairs_its_law_with_its_unit` holds every gate and every `ControlRate::Sample` destination to `None`, and the catalog presents the policy beside the law. **The segment** is `SlotState`'s current value, target, add-per-frame and remaining count: a resolved value is the target, `retarget` fixes the add from the **current** value, and `advance` writes one quantum of values — one add per frame, with the segment's last frame assigned the target exactly — into the slot's buffer before any kernel runs, in the one loop in `render/hot.rs` that moves a segment. **The kernel reads per frame**: `NodeIo::ramps` is the node's quantum-rate control buffers in declaration order, `ramp_of` names one, and the sine and sawtooth read their amplitude from it on every frame; the `Sine` and `Saw` state variants lost their amplitude field, `NodeState::set_control` is gone, and the stored base a slot starts from is `kernels::authored_value`'s read of the prepared record. `parameter_slot_tests` holds each clause: `a_segment_reads_past_its_start_on_its_first_frame_and_exactly_its_target_on_its_last` (first frame one tenth of the way over ten frames, monotone, last frame `== 1.0` by bits, later frames held, and a `None` policy a step on its first frame); `a_retarget_mid_segment_continues_from_the_current_value_not_the_previous_target` (halfway toward one, a write of zero starts from a half and reaches zero exactly); `the_kernel_reads_the_segment_per_frame_and_a_step_policy_renders_as_before` (through the renderer: under the seam's one-quantum policy every frame of the write's quantum is the stepped render scaled by `(k + 1) / 64`, and from the next quantum on the two renders are identical — the phase untouched by the per-frame read); `a_seeded_slot_takes_its_next_write_as_a_step_whatever_its_policy` and `an_activation_never_ramps_even_under_a_smoothing_policy` (adoption seeds every slot, the catch-up's write lands in force on the first quantum the new mapping governs, and the seed is spent by that one write). A gate and a pitch destination are unsmoothed by declaration and land as timed controls at their render position, which `SOUND-INV-016`'s and `SOUND-INV-021`'s tests already hold. **Charged**: one quantum of `f32` per quantum-rate writable control, in `slot_bytes` beside the slot itself, and `the_slots_the_renderer_holds_are_charged_to_the_mutable_state_row` compares it with what preparation holds. The purity scan covers the advance and the read: `advance`, `retarget` and `seed` are defined in the region, `ramp_of` in the kernel file, and `Option::or` and `slice::last` are on the allowlist with their reason. Mutations run and caught are listed in the slice's commit. Bit-identical: EVD-0013's aligned render and the three `quantum_cost` digests reproduce, which is what a zero-frame segment reproducing every step means. **One independent read found three defects, all repaired**: the two `usize` index tables preparation allocates beside the buffers — a buffer offset per slot and a per-node run table — were not in `mutable_state_bytes`, so a plan admitted at its ceiling allocated past it — they are charged now, the offset with its slot and the run table per scheduled record plus a terminator, and the charge test holds both against what is held; the increment's endpoint subtraction could overflow for two legal finite values of opposite sign, making the first advance saturate to the target instead of ramping — it is computed in `f64` and narrowed once divided; and the EVD-0009/0010 harnesses fed their hand-built sine a unity ramp while the fixture authors `0.8`, so their arm-against-renderer comparisons were of two signals — each harness now seeds its ramp from the prepared record. Running them to confirm that showed both had been panicking at admission since `SOUND-INV-021` — their voice scope named no tuning, and the gate compiles examples without running them — so their fixtures now declare one, and both run to completion with their arm checks passing. **Decided by the user 2026-09-05, and not a defect**: no declared parameter smooths, and for the one quantum-rate control V2 has that is V1 parity rather than a deferral — the lowerer maps V1's *oscillator* level onto the V2 amplitude as a static base, and V1's oscillator applies that level unsmoothed. The level V1 does de-zipper per block is its *amplifier* level, which the lowerer refuses unless unity because V2's amplifier has no level of its own. `P05-R001` names the trigger: the parameter that first receives V1's amplifier level, or the V2 amplitude the first time a lowering writes it dynamically, decides its `Smoothing` against V1's per-block ramp with an A/B to measure; until then every write is a step, as every existing render exhibits. An independent read corrected the first form of this sentence, which had named the oscillator's own lowering as the trigger, a point already passed. **Owed**: nothing of this invariant to a later slice |
| SOUND-INV-025 | **Built by `P06-S001`.** `voice_tests` (in-crate, to read the state table and the prepared records) holds each clause against a four-voice plan: `two_overlapping_notes_render_as_the_sum_of_two_single_note_renders` (sample for sample, and louder than either alone), `a_release_ends_its_own_voice_and_no_other` (after the second note's release the render **is** the first note alone, bit for bit), `a_release_of_a_key_ends_the_newest_open_note_on_that_key` (two notes on one key, the softer struck second; the one release leaves the first, which ending the oldest would not), `each_note_lands_on_its_own_instance_and_the_rest_stay_at_rest` (two keys' frequencies and two held gates on two instances, the prepared frequency and a released gate on the other two, read from the kernels' state), `a_parameter_write_to_a_voice_scope_control_reaches_every_instance` and `a_quantum_rate_write_to_a_voice_scope_control_reaches_every_instance` (a gate write and an amplitude write on four voices render four times the one-voice render of the same writes, on the sample-positioned and the quantum-rate path), `a_quantum_full_of_fanned_out_writes_has_room_in_the_control_scratch` (the compiled share's worth of gate writes in one quantum on sixteen voices: every instance's run fits, where a scratch sized on a note-on's expansion drops the trailing runs without a word), `the_preflight_arena_bound_covers_a_voiced_plans_exact_arena` (a plan refused on its voice count carries the preflight's scratch row, and that bound is not below what lowering the same plan takes), `prepared_data_is_shared_and_state_is_per_instance` (twelve node steps over three prepared records plus the sum's four, sixteen rows and four addresses, the prepared row grown by the sum's records alone, the mutable row equal to what preparation holds), `a_plan_with_one_simultaneous_note_has_one_instance_and_no_voice_sum`, and `the_voice_count_is_admitted_as_derived_from_the_producers` (`LimitExceeded` names `MaxActiveVoices`, the derived four and the profile's two). `note_identity`'s `two_voices` fixture — a voice-scope chain feeding an instrument-scope amplifier, sixteen indices — is the cross-scope case: its releases interleaved across the two nodes each end their own note, which is the consumer outside the scope reading the sum rather than instance 0. `admission`'s `an_output_declared_in_the_voice_scope_is_refused` names the refusal and admits the same plan with a global output; `transport_activation`'s `a_release_across_the_anchor_pairs_by_key_and_not_by_node` (key A before the destination, key B after it on the same node, A's release after B's note-on: the release is the omitted crossing one and B's pair is stamped, where a walk keyed by node alone refused the activation as unmatched) and `a_loops_crossing_release_closes_its_own_key_and_not_a_note_open_inside` (a producer admitted two, a loop whose pass holds three: refused on the loop's own rule, where a walk keyed by node alone let the crossing release close another key's note and admitted a loop whose first wrap over-emits); `scratch_tests`' `the_fan_out_width_is_the_voice_count_only_where_a_voice_control_is_sample_positioned` (one for a voice scope with no sample-positioned control at every polyphony, the voice count for the real voice, and the IR's figure equal to the plan's); and `tap_tests` holds each tap row to its own instance's step and that step's region. The purity scan covers `accumulate`, `voice_row` and the fan-out, with `then_some` on the allowlist with its reason. Bit-identical: EVD-0013's aligned render and the three `quantum_cost` digests reproduce, after the repairs as before them. **Seventeen mutations run and caught**: every note routed to instance 0; a quantum-rate write reaching the first row only; a sample-positioned write likewise; the accumulate adding nothing; the sum skipping instance 0; rows claiming one instance; the instance count not floored at one; state records not per instance; a release ignoring the key; a release ending the oldest of one key; the derived count not admitted; the fanned-out write not charged; a tap row naming instance 0's step; the fan-out width always the voice count; the depth table ignoring the key; the arena bound counting each node once; an output in the voice scope admitted. **One independent read found eight defects, all repaired**: the activation walk's history and suffix depths, and the loop's before and inside depths, were kept per node while the release rule had become per node and key — a release of a key opened before the anchor could be absorbed by a key opened after it and the activation refused as unmatched, and a loop's crossing release could close another key's note and undercount the pass — they are one `NoteDepths` table per `(slot, key)` now; an `Output` in the voice scope was accepted and read one instance, silently dropping the others — refused at validation as `OutputInVoiceScope`; the preflight arena bound counted each authored node's region once where every instance writes one — it counts scheduled records now; the timed-control scratch was charged `N` writes per event for every plan, refusing or over-allocating a plan whose voice scope has no sample-positioned control — the width is the widest `ControlRate::Sample` group's instance count, derived on both sides and held equal; every tap row named instance 0's step; the sum-source membership test was a linear scan per voice node over a sorted list — a binary search now; and the target's instance count was a bare `u32` — it is a `VoiceCount`. **`P06-S002` built the compiled path's stealing**, held by `voice_tests` against sample-exact oracles built from single-note renders: `a_full_producer_takes_the_oldest_voice_fades_it_and_starts_the_new_note_when_the_fade_ends` (two voices held, a third note: the oldest fades over 128 frames by the sum kernel's own gain, the other plays on, and the new note is the note rendered from time zero on a fresh stream shifted to where the fade completes — the reset voice's oscillator restarts at phase zero, which a note rendered alone at that position would not; the taken note's release counted once), `the_taken_voice_is_reset_so_the_new_note_attacks_from_silence` (the same under a 10 ms attack, where an un-reset envelope would hold full level), `a_same_note_policy_retriggers_the_held_key_at_once_and_without_a_fade` (the held key's voice carries the new velocity from the new note's own position), `a_same_note_policy_takes_the_newest_held_note_of_the_key` (two held notes on one key: the newer is taken), `a_one_voice_stealing_plan_sums_its_single_voice_so_the_fade_has_a_step`, `a_plan_declaring_no_stealing_refuses_a_full_producer_as_before` (the minter's over-emission, at the third note), `a_steal_whose_expansion_overruns_the_compiled_share_is_refused_at_preparation` (the share's worth of writes where the reset and the new note land: the source admits, preparation refuses by name), `the_steal_expansion_is_derived_alike_from_the_ir_and_the_plan`; `node`'s `every_declared_control_is_below_the_reserved_floor`; and `transport_activation`'s `a_history_that_steals_builds_and_counts_the_taken_notes_release` (a stealing history is not an over-emission; the taken note's release is counted, the crossing ones omitted). Bit-identical under `None`: EVD-0013's aligned render and the three `quantum_cost` digests reproduce, and the sum kernels take their old loop when no fade is in force. `a_note_shorter_than_the_fade_still_starts_and_ends_on_the_voice_it_took` and `an_oldest_policy_fades_a_taken_voice_even_when_the_keys_match` hold the displaced release and the policy's shape; `transport_activation`'s `a_loops_repeating_pass_charges_a_steals_expansion_where_it_lands` holds the loop's density rule to the expansion. **`P06-S002b` built the live boundary's steal**, held by `simulated_ingress` to the compiled path bit for bit: `a_live_note_on_into_a_full_producer_steals_as_the_compiled_one_does` (the same three notes offered live and stated compiled render the same samples; the taken note's release is counted once at the boundary and no orphan is reported), `a_live_release_offered_while_the_start_is_pending_is_displaced_with_it` (a note shorter than the fade keeps its length on both paths and is silent after its displaced release), `a_live_same_note_retrigger_matches_the_compiled_one`, and `a_live_producer_declaring_no_stealing_still_drops_the_note_on_by_name` (under `None` the hold runs out with the voices and the drop names it, as before). The simulated source is still written at one site, `envelope_for`, which the scan holds. `a_voice_taken_twice_before_its_first_victims_release_counts_every_release_and_leaks_no_hold` (five notes on two voices, every release counted, the next note-on finding a hold), `a_live_note_arriving_while_a_taken_voice_waits_to_start_takes_the_other_voice`, `a_live_note_on_finding_every_voice_waiting_to_start_is_dropped_by_name`; and on the compiled path `voice_tests`' `a_voice_waiting_to_start_is_not_taken_and_a_released_one_stays_committed_to_its_tail` (read from the stamped events: the second fade names the started voice, and no start lands on the committed index) and `a_note_on_finding_every_voice_waiting_to_start_is_an_over_emission`. **Nine mutations run and caught** at the boundary and in the book: the newest voice taken; a pending note's release not displaced; a taken note's release treated as an orphan; the deferred start never published; a waiting or ending voice eligible in the book; a released index freed at its authored position; the boundary freeing a deferred note's index at the offer; the boundary taking a waiting voice; a second victim on one voice overwriting the first. **One independent read (agy, `gemini-3.8-flash-high`; codex out of quota) found three defects, all repaired, and made two observations that were not defects**: a voice whose start was pending was refused as a victim only while the record lived, and the record lived forever once the note had started with no release yet — the record now lives until the release is published and says whether the voice has started; a note released while its start was pending had its index freed at the offer, so a later note could mint onto a voice whose deferred start then clobbered it — the index is freed when the displaced release lands, by a sweep at the next offer, and the compiled path had the same hole in the minter, closed by deferring its releases likewise; a voice taken twice before its first occupant's release lost that occupant's record, so the release became an orphan and its hold leaked — taken notes are recorded per reservation, with an evicting list for those whose hold went with the voice. Not defects: the deferred records are indexed by the identity's index into a table sized by the whole partition, which the disjoint ranges make exact; and the ring's room is charged for what waits outside it by design, now as the two live-class entries a start publishes. **Ten more mutations run and caught**: the steal taking the newest voice; `SameNote` taking the oldest of the key; the fade never decaying; the envelope ignoring the reset; a release after a steal not counted; the expansion not charged to the share; the history refusing instead of stealing; the taken-in note's release left at its authored position; the repeating pass charging the authored position alone; `Oldest` retriggering a shared key. **One independent read (agy, `gemini-3.8-flash-high`, over the source and over the tests and specifications separately; codex was out of quota) found five defects, all repaired**: a note taking a voice by fade-then-start had its release left at its authored position, so a note shorter than the fade was released before it started and never ended — its release is displaced with its start; the loop's repeating pass charged a stealing note-on at its authored position only — it charges the reset and the delayed start where they land, and the displaced release; a fade or reset's instance index was taken from the identity unchecked — it is bounded as `voice_row` bounds a note's; a fade naming no live note was released silently — it is counted as an orphan; and two doc comments had been attached to the wrong functions. Two of its findings were not defects and are recorded as such: the scheduler's per-quantum figure is the plan's compiled share rather than a measure of the input stream, so the expansion cannot outgrow it once the stamped positions pass the share check; and the `SameNote` test's ending — the taken note's release ending the taking note — is the compiled stream's key-pairing rule, now stated in the invariant above, not the identity clause's violation, though the test's own comment had explained it wrongly and is corrected. **Owed**: `P04-R001`'s velocity composition, which lands on the per-instance velocity row this slice made |
| SOUND-INV-026 | **Built by `P06-S005`.** `sampler_tests` (in-crate) holds every playback clause as an **exact** comparison of a render against an oracle written from V1's law in the test: `a_note_at_the_root_plays_the_sample_at_its_recorded_rate_and_fades_on_release` (rate one at the root, then V1's 512-frame linear fade from the off edge, then silence); `a_key_above_the_root_reads_at_v1s_frequency_ratio` (a fifth above, a non-dyadic ratio computed as V1's `f64::from(played_f32) / f64::from(root_f32)`, every fractional position read by V1's two taps); `a_fine_tune_scales_the_rate_by_v1s_factor` (`2^(50 / 1200)` at the root); `a_stereo_sample_is_summed_to_mono_as_v1_sums_it` (per-channel reads halved); `one_shot_ignores_the_off_edge_and_plays_the_region_out`; `loop_mode_repeats_the_loop_while_held_and_fades_on_release_still_looping` (a whole tone above the root so every position is fractional and the second tap's wrap into the loop's start carries weight — at integer positions a clamp there is invisible, which a first draft of the test could not see); `a_start_offset_seeks_into_the_region_as_v1_does` (a quarter in, and `0.0005` under V1's threshold seeking nowhere); `velocity_scales_the_output_under_v1s_law` (three sensitivity–velocity pairs, the envelope's own sensitivity at zero so the sampler's factor is isolated); `the_second_note_on_the_voice_restarts_the_read`; `the_kernel_alone_plays_from_the_on_edge` (the kernel against hand-built controls, apart from the renderer, its whole quantum at half velocity); `a_regions_start_is_where_the_read_and_the_offset_begin` (a region from frame 1000, with and without an offset); `a_sample_at_another_rate_and_loop_mode_without_a_loop_are_refused_by_name`; `a_sample_no_sampler_reaches_is_not_prepared_into_the_plan`. The three tail tests feed the sampler to the output past the envelope-driven amplifier, because a transparent envelope with a zero release silences exactly the tail under test — an envelope shaping the sampler's fade is the ordinary voice and is not what those tests measure. The model and its refusals: `a_note_outside_the_zone_plays_nothing_and_is_counted` (a key above the range and a velocity below it, each silent and each counted once in `notes_outside_zone`, beside a note inside that sounds and counts nothing); `a_map_of_two_zones_and_a_direction_not_built_are_refused_by_name` (both variants, by name); `a_region_past_the_sample_and_a_dangling_reference_are_refused_at_construction` (a region one frame past the sample, a zone naming no sample, a sampler naming no map, a loop leaving its region, an empty region, an inverted key range, a non-finite sample and a ragged stereo buffer); `equal_samples_are_held_once_and_the_charge_is_what_the_plan_holds` (two IR references with equal content are one plan entry, the second reference plays, and `GraphIr::sample_bytes` equals the entry's bytes plus one slot); `a_note_sent_to_the_sampler_itself_is_refused` (no note slot resolves for it while the envelope's does). `tuning_tests`' `the_twelve_tone_table_is_v1s_formula_bit_for_bit` holds every key of the prepared table equal to V1's `Hertz::from_midi` by bits, which is what makes the rate V1's ratio exactly rather than within a rounding. `node.rs`'s registry tests hold the sampler as the twelfth declared kind, its five controls paired with their laws, its three destinations sample-positioned by `descriptor_destinations_are_sample_positioned`, and its record prepared against a one-sample fixture; `node_representation` and `graph_validation` carry it in their catalogs. `render_loop_purity` covers the kernel and the four names the loop now reaches — the plan's sample table, a region's start, a frame's index and the outside-zone counter — each justified on the allowlist. Bit-identical: EVD-0013's aligned render and the three `quantum_cost` digests reproduce; no existing fixture holds a sampler or a trigger, and the adoption queue's wider allocation changes no sample. Sixteen mutations run and caught: the rate dropping the fine-tune factor; the second tap clamping at the loop's end instead of wrapping; a stereo sample summed rather than halved; the off edge starting no fade; the start offset ignoring V1's threshold; a release writing no off edge to the trigger; the trigger ignoring the zone's ranges; the sample table not deduplicated; the sample charge omitting the slots; the root frequency never filled; one-shot honouring the off edge; the loop stopping once the fade starts; a map of several zones playing its first; a direction not built playing forwards; the velocity law dropping its unity term; and an unmatched note not counted — and three after the read: a sample at another rate admitted, `Loop` without a loop played as `Sustain`, and every IR sample prepared whether reached or not. **A sampler under a seek is held by `P06-S006`** (`tuning_paths`' `a_seek_past_a_sounding_sampler_note_silences_it_from_the_boundary`, named for the behaviour it measures rather than for a mechanism: the sampler feeds the output directly, sounds before the seek and still sounds late in a render with no seek, and is silent from the boundary quantum plus the carry plus V1's fade). Four combined mutations establish the mechanism rather than assume it: on the compiled path the trigger of a crossing note is lowered by the **catch-up's restore of every target** — the value table carries no on edge for a trigger, because `write_magnitudes` skips it, so the restore writes the trigger's authored zero — and removing both explicit sites changes nothing; if the catch-up is made to carry the on edge, the `gate_rows` extension is what cuts it (removing that site then fails the test) while the adoption's trigger gate-downs do not fire for a compiled crossing note at all — they serve **live** notes, which ADR-0050 clause 8 keeps out of an activation's scope, so that site is read against the gate's own adoption path and not measured. The outside-zone count is taken where the renderer expands the note-on into its writes — the pass that holds the key, the velocity and the prepared record, and decides through the zone's two ranges without any frequency — rather than in the resolve step, which holds the identity alone; the kernel, which sees only controls, decides nothing about the zone. **One independent read (agy, `gemini-3.8-flash-high`, over the source and over the tests and records separately; codex out of quota) found two defects in the code and two contract holes in the records, all repaired**: every IR sample was prepared into the plan whether a sampler reached it or not, while the charge counted only the reached ones — the table is built over the reached set now, and `a_sample_no_sampler_reaches_is_not_prepared_into_the_plan` holds the two together; the sampler's state record was charged a field the state does not hold — removed; a sample recorded at another rate was carried and never compared — refused by name; and `Loop` over a zone with no loop fell back to `Sustain` — refused by name. The read also asked for the kernel-alone test to decode the velocity control and check its whole quantum, and for a region whose start is not zero, which `a_regions_start_is_where_the_read_and_the_offset_begin` supplies. **Three of its findings are not defects and are recorded as such**: that a release's trigger rows are addressed by the identity's index rather than by a voice index — since `P06-S001` the identity index *is* the voice instance, and every magnitude write in the loop addresses rows the same way; that the widened adoption queue's charge has no test — `render_scratch`'s `check_one` compares `timed_control_scratch_bytes` against `PreparedRenderer::control_scratch_bytes`, which sums the adoption queue with the scratch, and both changed together; and that the outside-zone count would need a frequency converted back to a key — it is decided where the key is, as the sentence before this one now says |
| SOUND-INV-019 | `tempo.rs`: a beat's exact frame at a constant tempo; a step holding the old tempo up to its change; a half-frame position rounding away from zero rather than truncating; a position being the stored prefix plus its own offset, and independent of what was asked before it; a tick past exact integer range refused rather than answered; and the ramp's own nine — an equal-endpoint ramp equal to a step bit for bit, falsified by the ramp recomputing the shared linear term; a ramp lasting its beats times the mean of its two periods, asserted as the corpus fixture's exact 48 000 frames and explicitly not V1's 44 361, falsified by the quadratic term's sign and by treating every change as a ramp; positions non-decreasing across steep ramps in both directions over adjacent ticks in four sampled windows, falsified by that same sign; a tempo whose period overflows refused at construction, and a 6000-to-`1e100` BPM ramp reporting a real tempo one tick before its end, falsified separately by dropping the period check and by either rejected interpolation form; chained ramps each reaching the next declared tempo with a continuous junction, falsified by pointing a ramp at the last one; a trailing ramp behaving as a step, falsified by giving it a degenerate destination; and the reported tempo being the reciprocal of the interpolated period rather than a straight line between two tempo numbers, falsified by reporting the declared tempo. The standing source scan covers the five functions the law reaches **and is closed under calls**: every call those bodies make must be to one of the five or to a named arithmetic or accessor method, and no allowlisted name may itself be a function this module defines — so a transcendental can hide neither in an unfollowed helper nor behind an allowlisted name, both mutation-verified. It strips comments and attributes but not a line holding a quote, and that exemption is checked directly, since the module's own source cannot exercise it |
| Node arithmetic and preparation | `voice_nodes`, internal kernel tests |

## Unresolved questions

**SOUND-INV-013's in-module residual is bounded and named.** The recorded gap
was that a descriptor's function pointer might resolve outside the region. The
compiler now settles where a descriptor may *point*: a `Kernel` cannot be
constructed outside `node::kernels`, every descriptor is in `node.rs`, so every
registered pointer is one of that module's constants. It does **not** settle what
those constants wrap — an in-module `Kernel(foreign)` is well-typed, and what
rejects it is the source scan rather than the type system. So the part of the
original gap that no mechanism closes is a constant declared *inside*
`node::kernels` around a function that module names by path, in a spelling the
scan does not recognise. `render_loop_purity` scans that
file's construction sites, and nine spellings are mutation-checked — a free
function returning `Kernel`, a method returning `Option<Self>`, an associated
constant, functional-record syntax, a type alias, a widened field, an
unregistered kernel, an unused constant, and a descriptor naming a foreign
function — but it is a scan for source forms, and a scan for a grammar cannot be
exhaustive. Closing it would need a resolver-level check of the pointers the
registry actually holds, which pointer identity across codegen units makes
delicate. The residual is a deliberate edit inside the file whose purpose is to
hold the kernels, which is a narrower thing than the gap this replaced.

**Four pre-existing conformance rows name a check that does not carry the
invariant.** An independent read of the rows this phase inherited found
SOUND-INV-001/004, 003/012, 007 and 008 each paired with a check narrower than
the invariant it is listed against. The finding predates this phase's work and is
not a defect in the code; closing it is open work tracked in `NOW.md`.
