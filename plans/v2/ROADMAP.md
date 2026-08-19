# Core V2 Roadmap

This file is authoritative for Core V2 outcomes, phase order, dependencies, and
gate summaries. Active task state belongs only in [`NOW.md`](NOW.md). The phase
links in the table below lead to the complete exit criteria retained in Part I
of [`master-plan.md`](master-plan.md); those criteria remain authoritative and
the summaries here do not replace or weaken them. The rest of the master plan
is historical design material rather than an operational authority.

## Migration strategy

1. Keep V1 shippable until cutover.
2. Prove Sound Core offline before live integration.
3. Build vertical slices before broad catalogs or frontend migration.
4. Keep Project, Application, and Sound Core ownership separate.
5. Require executable evidence at phase exits; do not freeze cheap internal
   implementation choices merely to make a gate look settled.

## Phase order

`State` is the phase-lifecycle authority and changes only at activation or an
accepted exit review. `NOW.md` owns task activity within an active phase.

| Phase | Outcome | State | Depends on |
|---|---|---|---|
| [0A](master-plan.md#phase-0a-baseline-limits-and-render-core-contracts) | V1 baselines, resource limits, and initial render contracts | Complete | — |
| [0B](master-plan.md#phase-0b-migration-inventories-and-project-contracts) | Migration inventories and Project/Application contracts | Active, parallel | — |
| [1](master-plan.md#phase-1-introduce-the-experimental-sound-core-v2-crate) | Deletable experimental Sound Core renderer | Complete | 0A |
| [2](master-plan.md#phase-2-minimal-compiled-voice-graph) | One complete compiled voice graph | Active | 1 |
| [3](master-plan.md#phase-3-sample-accurate-scheduler-and-block-partition-invariance) | Sample-accurate scheduler and host-block invariance | Not started | 2 |
| [4](master-plan.md#phase-4-current-project-lowering-and-offline-ab-path) | Current-project lowering and offline V1/V2 comparison | Not started | 3 |
| [5](master-plan.md#phase-5-declarative-node-and-parameter-api) | Declarative node and parameter API | Not started | 4 |
| [6](master-plan.md#phase-6-polyphony-and-instrument-runtime) | Polyphony and instrument runtime | Not started | 5 |
| [7](master-plan.md#phase-7-yams-mod-grid-and-unified-modulation) | YAMS, Mod Grid, and unified modulation | Not started | 6 |
| [8](master-plan.md#phase-8-mixer-channels-buses-effects-and-latency) | Mixer, channels, buses, effects, and latency | Not started | 7 |
| [9](master-plan.md#phase-9-live-integration-and-immutable-plan-swapping) | Live integration and immutable plan swapping | Not started | 8 |
| [10A](master-plan.md#phase-10a-canonical-project-model-and-stable-identity) | Canonical project model and stable identity | Not started | 0B |
| [10B](master-plan.md#phase-10b-application-operations-and-transactions) | Application operations and transactions | Not started | 10A |
| [10C](master-plan.md#phase-10c-history-dirty-state-save-and-recovery) | History, dirty state, save, and recovery | Not started | 10B |
| [10D](master-plan.md#phase-10d-project-format-v2-assets-and-conversion) | Versioned project format, assets, and conversion | Not started | 10A–10C |
| [10E](master-plan.md#phase-10e-mcp-cli-import-and-service-migration) | MCP, CLI, import, service, and public-facade migration | Not started | 10A–10D |
| [11](master-plan.md#phase-11-gui-and-workflow-migration) | GUI and workflow migration | Not started | 9, 10E |
| [12](master-plan.md#phase-12-default-cutover-and-v1-retirement) | Default cutover and V1 retirement | Not started | 11 |

Phase 0B may run alongside Sound Core Phases 1–4. Phase 10 does not begin until
0B passes. Detailed work is activated in `NOW.md` only when it becomes the next
bounded slice.

## Outcomes and exit boundaries

### Phase 0A — baseline and render contracts

Outcome: a headless reference corpus, comparison command, V1 cost baselines,
bounded resource inventory, and initial host/render contracts.

Exit: [`REV-P00A`](reviews/phase-00a-exit-review.md) is accepted. Later findings
reopen it only if they invalidate relied-on evidence or a Phase 1 safety or
correctness guarantee.

### Phase 0B — migration inventories and project contracts

Outcome: every persisted field, reachable capability, and stable identity has
one evidenced V2 disposition; omission-prone data has round-trip fixtures; the
Project/Application contracts needed by Phase 10 are accepted.

Exit requires:

- complete state, capability, and identity inventories;
- mapped save, recovery, rollback, GUI, MCP, and CLI mutation paths;
- round-trip fixtures for omission-prone state;
- current stable-ID, operation-result, state-boundary, and format-envelope
  specifications;
- no required Phase 0B durable decision left open.

### Phase 1 — experimental Sound Core

Outcome: a host-driven, offline `synth_engine_v2` renderer that is deterministic,
bounded by `HostProfile`, real-time safe in its render loop, and removable
without changing V1.

Exit: [`REV-P01`](reviews/phase-01-exit-review.md) is accepted.

### Phase 2 — minimal compiled voice graph

Outcome: note events render through envelope, oscillator, filter, amplifier, and
output using validated topology, compact slots, a preallocated arena, liveness
reuse, and separated prepared/state data.

Exit requires:

- no names, topology work, allocation, or resizing in the hot path;
- path-local graph diagnostics;
- a second node without renderer-control-flow changes;
- musical equivalence to V1 or an explicit intentional difference;
- CPU evidence against the equivalent V1 patch;
- the binding real-node re-measurement of `Q`, followed by confirmation of
  ADR-0037 or a superseding decision; either result requires the corresponding
  current-spec update.

### Phase 3 — sample-accurate scheduling

Outcome: typed absolute and plan time, bounded event ingress and deferral,
sample-accurate note/transport/automation ordering, and output invariant to host
block partitioning.

Exit requires exact within-block event placement, stable tempo mapping,
declared same-sample ordering, bounded exhaustion behavior, and identical
deterministic output across equivalent host-block partitions.

### Phase 4 — current-project lowering and offline A/B

Outcome: current projects lower to V2 without GUI or live-engine state and can be
rendered through the same headless comparison path as V1.

Exit requires deterministic lowering diagnostics, corpus A/B evidence, and a
revisioned cancellable long-running job contract.

### Phase 5 — declarative nodes and parameters

Outcome: one declaration supplies node ports, parameters, preparation,
diagnostics, DSP entry points, automation/modulation metadata, and discovery
surfaces.

Exit requires representative native and legacy-adapted nodes, one parameter
composition law, generated discovery/schema agreement, and measured adapter
costs inside the phase budget.

### Phase 6 — polyphony and instruments

Outcome: immutable patch definitions create independent runtime instances with
bounded voice allocation, stealing, expression, sampler preparation, and stable
identity.

Exit requires deterministic voice behavior under pressure and equivalent
offline/live instance behavior.

### Phase 7 — YAMS and modulation

Outcome: YAMS, Mod Grid, automation, MIDI, and direct edits compile into one
typed parameter-composition system with bounded execution.

Exit requires deterministic dependency ordering, explicit conflict diagnostics,
and no script compilation or allocation on the audio thread.

### Phase 8 — mixer and latency graph

Outcome: tracks, channels, buses, sends, returns, inserts, master processing,
latency, and compensation are explicit graph concepts rather than frontend or
engine mirrors.

Exit requires routing/effect parity, deterministic latency compensation, and
bounded feedback/refusal behavior.

### Phase 9 — live integration

Outcome: off-thread compilation publishes immutable plans to live playback
without blocking audio; devices, clocks, recording, telemetry, and retirement
have explicit ownership.

Exit requires safe plan swapping, last-valid-plan behavior on compile failure,
simulated-host coverage, bounded retirement, and live/offline render agreement.

### Phase 10A — canonical project model

Outcome: one canonical project document owns persisted musical/editor state and
stable identities, independent of engine and frontend runtime state.

Exit requires complete inventory coverage, deterministic snapshots, and no
position- or presentation-derived persistent identity.

### Phase 10B — application operations

Outcome: every mutation flows through typed, revisioned transactions with one
effect/diagnostic vocabulary and optimistic-concurrency behavior.

Exit requires shared conformance behavior for direct, GUI-independent, and
remote adapters, including partial and failed operations.

### Phase 10C — history, save, and recovery

Outcome: undo/redo, dirty state, save, autosave, and recovery derive from
canonical revisions rather than engine or GUI reconstruction.

Exit requires equivalent snapshots from every save path and correct behavior
under failed compilation, stale revisions, and immutable assets.

### Phase 10D — format and assets

Outcome: a strictly versioned format and package model separates decode,
conversion, validation, opening, canonical source assets, and prepared caches.

Exit requires deterministic semantic round trips, explicit rejection of wrong
versions, asset integrity diagnostics, and a one-shot current-format converter.

### Phase 10E — services and external surfaces

Outcome: MCP, CLI, importers, jobs, OSC, configuration, authorization, and the
public facade adapt the same Project/Application/telemetry contracts.

Exit requires a classified and tested disposition for every reachable external
surface and no service bypass around Application Core.

### Phase 11 — GUI migration

Outcome: views read immutable canonical snapshots, emit typed operations, and
consume runtime telemetry separately from project state.

Exit requires all persisted GUI state to have a canonical owner, no save-time
view overlays, shared operation conformance, and actionable compile/session
status.

### Phase 12 — cutover and V1 retirement

Outcome: V2 is the only production architecture and migration-only engine
plumbing is removed.

Exit requires no V1 production path, complete supported-platform/feature/MSRV
coverage, converted retained content and external consumers, release history,
and documentation that describes only the final architecture.

## Definition of done

Core V2 is complete when Project Core is the sole persisted truth, Application
Core is the sole mutation/history boundary, Sound Core is the common bounded
renderer for every execution mode, all retained capabilities have a tested
disposition, and V1 can be removed without losing supported behavior.
