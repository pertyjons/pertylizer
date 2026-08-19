# SPEC: Sound Core Render Contract

| Field | Value |
|---|---|
| Status | Current |
| Phase | 1–2 |
| Created | 2026-08-19 |
| Last reviewed | 2026-08-19 |
| Based on | ADR-0001, ADR-0004, ADR-0005, ADR-0021, ADR-0032, ADR-0037, ADR-0040, ADR-0041 |
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
| [ADR-0037](../decisions/ADR-0037-render-quantum-value.md) | `Q` is currently 64 frames and must be re-measured before Phase 2 exits |
| [ADR-0040](../decisions/ADR-0040-v2-owns-its-dsp.md) | V2 owns the DSP it renders; no kernel is shared with V1 and no kernel carries a two-engine policy |
| [ADR-0041](../decisions/ADR-0041-interleaved-internal-channel-layout.md) | One signal is one interleaved arena region of `Q` frames of `c` channels, at a recorded offset and length |

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
   quantum-local offset are derived only after epoch validation; neither is
   stored in the event envelope. The currently admitted control event is
   evaluated at quantum rate as permitted by ADR-0001.
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
| SOUND-INV-007 | `graph_validation`, except its layout half, which the pending row below covers |
| SOUND-INV-008 | `lowering` |
| SOUND-INV-011 | `arena_reuse` |
| SOUND-INV-013 | `render_loop_purity`, partially: the kernel registry is closed and every registered kernel is defined inside the checked region. It cannot see a descriptor that points at a function outside it — see *Unresolved questions* |
| SOUND-INV-009, 010, 014, 015 | **pending P02-T013.** Today's checks verify the planar contract these invariants replace: `lowering` requires one output operation per channel, `arena_reuse` compares slot identity rather than physical ranges and has no observation-tap case, and `graph_validation`'s layout premise enumerates node kinds by hand and omits three that exist. The conversion brings its own checks, including ADR-0041 clause 16's baseline comparison |
| Node arithmetic and preparation | `voice_nodes`, internal kernel tests |

## Unresolved questions

**The crate does not yet meet SOUND-INV-009, SOUND-INV-010's region rules, or
SOUND-INV-014's contiguous output copy.** Its arena is still planar with uniform
slots; P02-T013 is the conversion that brings it to this contract, and its
acceptance check is ADR-0041 clause 16's per-quantum digest comparison against
baselines generated from the planar build in the commit before the conversion.

**SOUND-INV-013's falsifier is partial, and the gap is named rather than
assumed closed.** `render_loop_purity` requires the kernel registry to be closed
and every registered kernel to be defined in the checked region, so an ordinary
call out to a V1 kernel fails it. What it cannot see is a descriptor whose
function pointer resolves outside that region; `crate_boundary` does not close
that either, because its allowlist admits `synth_dsp` and `synth_modules`, which
ADR-0040 clause 1 permits for a non-kernel value or table. Closing or replacing
that check is open work.

**SOUND-INV-015 has no closed check.** `graph_validation` asserts mono output
layouts for a hand-written subset of node kinds, which is a partial form of the
port-table assertion, and the kernel tests exercise the mono kernels that exist.
What is missing is the obligation over the whole catalog at every admitted count;
it arrives with P02-T013, since no node kind produces a stereo signal before it.

P02-T007 must add the note-event payload, sample-offset behavior, and named
conformance check before this specification can claim that a note edge is
preserved at its declared sample across host-block partitions.
