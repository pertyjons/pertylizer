# SPEC: Sound Core Render Contract

| Field | Value |
|---|---|
| Status | Current |
| Phase | 1–2 |
| Created | 2026-08-19 |
| Last reviewed | 2026-08-19 |
| Based on | ADR-0001, ADR-0002, ADR-0004, ADR-0005, ADR-0021, ADR-0032, ADR-0037 |
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

ADR-0040 and ADR-0041 are decided but still `Proposed`. Their V2-owned DSP and
interleaved arena do not become current until they are independently reviewed
and accepted together.

## Terminology

The [shared glossary](../glossary.md) defines *render plan*, *render quantum*,
*host profile*, *sample time*, and *stream epoch*. Here, `Q` is the current
render quantum in frames and the *arena* is the renderer-owned preallocated
storage assigned to compiled signal lifetimes.

## Accepted decisions

| ADR | Current rule represented here |
|---|---|
| [ADR-0001](../decisions/ADR-0001-internal-render-quantum.md) | Host blocks are rendered through fixed internal quanta and carry semantics |
| [ADR-0002](../decisions/ADR-0002-internal-channel-layout.md) | The current arena is planar until ADR-0041 is accepted |
| [ADR-0004](../decisions/ADR-0004-native-node-representation.md) | Admission prepares a node dispatch entry; the hot loop is independent of the catalog |
| [ADR-0005](../decisions/ADR-0005-buffer-liveness-strategy.md) | Conservative deterministic liveness reuse with declared safe in-place processing |
| [ADR-0021](../decisions/ADR-0021-host-profile-and-admission-policy.md) | Resource limits are preparation inputs and excess plans are refused before render |
| [ADR-0032](../decisions/ADR-0032-sample-time-and-event-timestamps.md) | Engine time, plan position, epoch, and quantum-local offsets remain distinct |
| [ADR-0037](../decisions/ADR-0037-render-quantum-value.md) | `Q` is currently 64 frames and must be re-measured before Phase 2 exits |

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
   ports, wrong directions/domains, illegal fan-in, cycles, and missing required
   output structure with path-local structured diagnostics.
8. **SOUND-INV-008 — Deterministic lowering.** Stable semantic identities lower
   to compact runtime slots and a deterministic schedule independent of source
   declaration order.
9. **SOUND-INV-009 — Current channel storage.** Until ADR-0041 is accepted, one
   arena slot stores one planar channel of `Q` frames. Mono-to-stereo widening
   and output copies are explicit scheduled operations.
10. **SOUND-INV-010 — Safe reuse.** Two overlapping live values never share
    storage. Reuse is deterministic, and in-place processing occurs only where
    the node declares it safe and no later read needs the input.
11. **SOUND-INV-011 — Explicit silence.** An unpatched input reads defined
    silence rather than data left by a previous arena tenant.
12. **SOUND-INV-012 — Closed render entry.** Admission resolves the node's
    prepared data, mutable state shape, ports, controls, kernel entry, and
    in-place eligibility. Adding a node does not add renderer control flow.

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
| SOUND-INV-008, 009 | `lowering` |
| SOUND-INV-010, 011 | `arena_reuse` |
| Node arithmetic and preparation | `voice_nodes`, internal kernel tests |

## Unresolved questions

Acceptance of ADR-0040 and ADR-0041 updates this specification in one change:
V2 owns its rendered DSP, one signal occupies one interleaved variable-width
arena region, and layout-sensitive invariants and conformance checks are
replaced by the successor contract. Historical ADR and phase records remain
unchanged.

P02-T007 must add the note-event payload, sample-offset behavior, and named
conformance check before this specification can claim that a note edge is
preserved at its declared sample across host-block partitions.
