# SPEC: Sound Core Render Contract

| Field | Value |
|---|---|
| Status | Current |
| Phase | 1–2 |
| Created | 2026-08-19 |
| Last reviewed | 2026-08-25 |
| Based on | ADR-0001, ADR-0004, ADR-0005, ADR-0021, ADR-0032, ADR-0037, ADR-0040, ADR-0041, ADR-0043, ADR-0046 |
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
| [ADR-0021](../decisions/ADR-0021-host-profile-and-admission-policy.md) | Resource limits are preparation inputs and excess plans are refused before render |
| [ADR-0032](../decisions/ADR-0032-sample-time-and-event-timestamps.md) | Engine time, plan position, epoch, and quantum-local offsets remain distinct |
| [ADR-0037](../decisions/ADR-0037-render-quantum-value.md) | `Q` is 64 frames, and ADR-0037 fixes it finally rather than provisionally |
| [ADR-0040](../decisions/ADR-0040-v2-owns-its-dsp.md) | V2 owns the DSP it renders; no kernel is shared with V1 and no kernel carries a two-engine policy |
| [ADR-0041](../decisions/ADR-0041-interleaved-internal-channel-layout.md) | One signal is one interleaved arena region of `Q` frames of `c` channels, at a recorded offset and length |
| [ADR-0043](../decisions/ADR-0043-event-deferral-and-late-clamp.md) | A late event is clamped forward without rewriting its stamp; its sample and control effects follow the clamped render position |
| [ADR-0046](../decisions/ADR-0046-destination-quantum-admission.md) | Capacity is admitted before rendering; the renderer never moves an event to make a quantum fit |

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
    bit-identically. A note event names a **node**, not one of its controls — which
    control being played moves belongs to the node kind, and a node that declares
    none cannot be addressed by a note at all. The schedule is still walked exactly
    once per quantum, so this is neither a second control evaluation (clause 4) nor
    the event-boundary quantum split clause 15 reserves for a later phase: the edges
    due in a quantum are resolved before it renders and handed to the node that owns
    them.

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
| SOUND-INV-003, 012 | `node_representation` |
| SOUND-INV-005, 006 | `render_contract` |
| SOUND-INV-007 | `graph_validation` |
| SOUND-INV-008 | `lowering` |
| SOUND-INV-011 | `arena_reuse` |
| SOUND-INV-016 | `note_events`, over the compiled path, and the envelope kernel's own edge tests in `kernels`. **The clamped branch is covered separately**, because `note_events` runs offline where a late event cannot be presented: `render_contract`'s `a_late_note_edge_takes_effect_at_its_clamped_render_position` drives a late note-on and an on-time note-off through one live render and asserts an exact value per frame for both, so an edge dropped, held to a boundary, or applied at the head of the call fails it — a one-frame error is mutation-verified to fail. The count-only `a_late_event_is_clamped_forward_and_counted` cannot see any of that. `note_events` places each edge at an offset that is **not** a multiple of `Q` and asserts an exact value per frame, so an edge quantized to a boundary fails it; it renders the same edge through three host-block partitions and through both payloads and requires all of them bit-identical; and `an_edge_mid_ramp_starts_from_the_level_that_frame_would_have_had` covers the case the instantaneous-segment fixtures cannot see, where the level a frame starts from and the sample before it differ by one step. `layout_baseline` cannot cover any of this — every edge in its fixtures is on a boundary — so it is a regression control here rather than a placement check |
| SOUND-INV-013 | Two mechanisms, and what each one carries is stated rather than blurred. **The type system** enforces exactly one thing: a `Kernel` cannot be *constructed outside* `node::kernels`, its field being private — so a descriptor elsewhere naming any function does not compile (`E0423`, mutation-verified). Every descriptor lives in `node.rs`, so every registered pointer is necessarily one of that module's constants; what those constants wrap is not settled by privacy, and an in-module `Kernel(foreign)` is well-typed. **A bounded source scan** carries the rest — `render_loop_purity`'s `the_kernel_registry_is_closed_and_no_scanned_form_forges_a_kernel` requires every construction site it recognises in that one file to be a declared constant, and checks the entries and constants it can parse agree in both directions. Nine forging routes are mutation-checked. What the scan is not is under *Unresolved questions* |
| SOUND-INV-009, 010, 014 | `layout_baseline` for ADR-0041 clause 16's per-quantum digest comparison over its five fixtures, `arena_reuse` for the structural check over physical sample ranges and for `reuse_renders_bit_identically_to_no_reuse`, and `lowering`'s `a_mono_source_into_a_stereo_stream_widens_into_one_wider_region`, `a_mono_stream_compiles_exactly_one_output_operation` and `an_inserted_conversion_is_reported_and_not_only_scheduled` |
| SOUND-INV-015 | `graph_validation`'s `every_kernel_admits_exactly_one_channel_on_every_port` over the macro-generated catalog, and `kernels`' `the_widening_writes_every_channel_of_every_frame` for the one kernel with two admitted counts |
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
