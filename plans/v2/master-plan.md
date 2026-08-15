# Pertylizer Core V2: Project, Application, and Compiled Audio Engine

## Status

Proposed and architecture-audited against the current codebase on 2026-08-12.
This is the consolidated architecture and migration plan for three
coupled foundations:

- **Project Core V2** — the canonical, versioned document and asset model;
- **Application Core V2** — operations, transactions, validation, history, and
  revision coordination shared by every frontend;
- **Sound Core V2** — the compiled real-time and offline audio engine.

It combines the decisions reached from reviewing the current real-time engine,
graph execution, sequencing, parameter automation, YAMS, modulation, tracks,
mixer routing, project ownership, save/load paths, undo, dirty state, GUI-owned
metadata, MCP mutations, and offline reconstruction.

This is deliberately a migration plan, not a rewrite mandate. The current
`SynthEngine` remains the production engine until V2 has passed explicit audio,
real-time, determinism, and workflow gates. V2 starts as an offline renderer and
is integrated into live playback only after it has proven the architecture.

The audio subsystem is called **Sound Core V2** in this document. The temporary
crate should be named `synth_engine_v2` while both engines coexist. **Pertylizer
Core V2** refers to the combined project, application, and sound architecture.
Do not rename the existing crates or split the new architecture into many crates
before the dependency boundaries have been demonstrated by working code.

### Authority of this document

This plan is authoritative for scope, phase order, and exit gates. It is not
authoritative for settled details: an accepted ADR in
[the decision register](ADR.md) and a current document under [specs/](specs/README.md)
supersede provisional wording here. When an accepted decision contradicts this
plan, update the affected sections in the same change instead of leaving two
live answers. Operational task state belongs in a
[phase tracker](phases/README.md); the `Work` lists below define the normative
scope those trackers decompose.

## Contents

- [Part I: Migration Plan](#part-i-migration-plan)
  - [Migration principles](#migration-principles)
  - [Coexistence architecture](#coexistence-architecture)
  - [Phase 0A: Baseline, limits, and render-core contracts](#phase-0a-baseline-limits-and-render-core-contracts)
  - [Phase 0B: Migration inventories and project contracts](#phase-0b-migration-inventories-and-project-contracts)
  - [Phase 1: Introduce the experimental Sound Core V2 crate](#phase-1-introduce-the-experimental-sound-core-v2-crate)
  - [Phase 2: Minimal compiled voice graph](#phase-2-minimal-compiled-voice-graph)
  - [Phase 3: Sample-accurate scheduler and block-partition invariance](#phase-3-sample-accurate-scheduler-and-block-partition-invariance)
  - [Phase 4: Current-project lowering and offline A/B path](#phase-4-current-project-lowering-and-offline-ab-path)
  - [Phase 5: Declarative node and parameter API](#phase-5-declarative-node-and-parameter-api)
  - [Phase 6: Polyphony and instrument runtime](#phase-6-polyphony-and-instrument-runtime)
  - [Phase 7: YAMS, Mod Grid, and unified modulation](#phase-7-yams-mod-grid-and-unified-modulation)
  - [Phase 8: Mixer, channels, buses, effects, and latency](#phase-8-mixer-channels-buses-effects-and-latency)
  - [Phase 9: Live integration and immutable plan swapping](#phase-9-live-integration-and-immutable-plan-swapping)
  - [Phase 10: Project Core V2 and Application Core V2 migration](#phase-10-project-core-v2-and-application-core-v2-migration)
    - [Phase 10A: Canonical project model and stable identity](#phase-10a-canonical-project-model-and-stable-identity)
    - [Phase 10B: Application operations and transactions](#phase-10b-application-operations-and-transactions)
    - [Phase 10C: History, dirty state, save, and recovery](#phase-10c-history-dirty-state-save-and-recovery)
    - [Phase 10D: Project Format V2, assets, and conversion](#phase-10d-project-format-v2-assets-and-conversion)
    - [Phase 10E: MCP, CLI, import, and service migration](#phase-10e-mcp-cli-import-and-service-migration)
  - [Phase 11: GUI and workflow migration](#phase-11-gui-and-workflow-migration)
  - [Phase 12: Default cutover and V1 retirement](#phase-12-default-cutover-and-v1-retirement)
- [Part II: Target Architecture](#part-ii-target-architecture)
- [Part III: Verification and Quality Gates](#part-iii-verification-and-quality-gates)
- [Part IV: Risks and Controls](#part-iv-risks-and-controls)
- [Part V: Scope and Non-goals](#part-v-scope-and-non-goals)
- [Part VI: Coordination With Existing Plans](#part-vi-coordination-with-existing-plans)
- [Part VII: Open Decisions](#part-vii-open-decisions)
- [Definition of Done](#definition-of-done)

## Objective

Replace the current collection of project, GUI, session, engine-mirror, and
save-time reconstruction paths with one coherent flow:

```text
Project Core V2 document
        |
        v
Application Core V2 operations and revisions
        |
        v
Sound Core V2 compiler and immutable render plan
```

Project Core V2 must be the sole authority for persisted musical and editor
state. Application Core V2 must be the sole mutation boundary used by GUI, MCP,
CLI, undo, save, recovery, and future hosts. Sound Core V2 must execute a
complete immutable render plan prepared off-thread. Together they must provide:

- sample-accurate event and automation timing;
- behavior independent of host callback partitioning;
- structural real-time safety by construction;
- a compact, index-based hot path without names or hash maps;
- one parameter composition system for UI edits, automation, modulation, MIDI,
  Mod Grid, and YAMS;
- reusable patch definitions with independent runtime instrument instances;
- explicit track, channel, bus, scope, rate, latency, and signal semantics;
- an explicit host-I/O and clock-domain contract for audio devices, audio
  input, MIDI, monitoring, latency, drift, reconnect, and timestamp mapping;
- separate project-operation, runtime-session, live-performance, and recording
  result paths so transport state never masquerades as a persisted edit;
- declared resource profiles and compile-time admission instead of scattered
  runtime caps or silent truncation;
- the same render core for live playback, offline rendering, analysis, CLI use,
  and future runtime-library use;
- a cleaner module-authoring contract where a module supplies DSP and a single
  declarative interface rather than duplicating engine plumbing;
- a versioned project and asset format with explicit conversion and validation;
- one transaction and revision stream for mutations, history, dirty state,
  compilation, save, autosave, and optimistic concurrency;
- stable domain identities that do not encode type, order, or presentation;
- strict separation of project state, runtime session state, user settings,
  editor presentation state, host/service configuration, and lossy runtime
  telemetry;
- one revisioned, cancellable job contract for offline render, analysis,
  export, and other long-running work;
- one observation/telemetry facade for GUI meters, passive signal taps, OSC,
  and external visualizers without making their buffers project state.

The central architectural rule is:

> The project document is authoritative. Frontends request application
> operations. The audio thread executes a prepared plan. No layer reconstructs
> persisted truth from the audio engine.

The audio thread does not own or edit the project model, validate graphs,
resolve names, construct modules, clone voice graphs, allocate buffers, compile
scripts, or destroy retired plans.

---

# Part I: Migration Plan

## Migration principles

1. **V1 remains shippable.** Current development and releases continue while V2
   is experimental. No user project is opened with V2 by default until the
   cutover gate is met.
2. **Start offline.** V2 first renders into caller-owned buffers with no audio
   device, GUI, command ring, or live graph swapping.
3. **Migrate vertical slices.** Complete one audible path end to end before
   expanding horizontally across the module catalog.
4. **Adapt the current project model first.** A `LegacyProjectLowerer` converts
   today's `ProjectFile`, `Song`, instruments, and graphs into the V2 compiler
   IR. The project and GUI models are redesigned only after the render core is
   proven.
5. **Do not duplicate DSP algorithms without need.** Extract or adapt existing
   oscillators, filters, envelopes, effects, samplers, and script programs.
   Replace engine-facing plumbing, not validated DSP merely for uniformity.
6. **Make parity measurable.** V1 and V2 offline renders are produced from the
   same input project and analyzed by deterministic comparison tools.
7. **Permit intentional differences.** V2 is expected to correct block timing,
   control-rate dependence, gain staging, and other V1 semantics. Differences
   must be classified and documented rather than hidden behind loose tolerances.
8. **No big-bang GUI migration.** The existing GUI continues editing the current
   model until application operations and the new canonical project model are
   ready.
9. **Every phase has a stop gate.** A phase that cannot demonstrate its stated
   simplification or invariant is reviewed before more code is ported.
10. **No premature compatibility layer.** Active development permits project
    and API breaks. During migration, preserve examples and test fixtures with a
    one-shot converter or resave path, not permanent dual semantics.
11. **One persisted authority.** A value that changes the saved project must
    have one owner in Project Core V2. Engine snapshots, GUI overlays, command
    mirrors, and fingerprints may be temporary migration inputs, never final
    ownership.
12. **Separate kinds of state.** Project document state, runtime session state,
    user settings, editor-only presentation state, and telemetry must be
    classified explicitly. Convenience is not sufficient reason to persist one
    inside another.
13. **Frontends are adapters.** GUI, MCP, CLI, importers, and future runtime
    hosts use the same application operations and validation. None defines its
    own mutation semantics.
14. **Preserve safety mechanisms, replace their observation source.** Atomic
    save, recovery, undo coalescing, and dirty-state correctness remain active
    during coexistence. They move to the canonical revision stream only after
    equivalent behavior is proven.
15. **Inventory capabilities, not only persisted fields.** Every shipped or
    externally consumed capability receives an explicit migrate, replace,
    remove, defer, or compatibility-adapter decision. Exported but unreachable
    experimental code is not a V2 requirement merely because it exists.
16. **Reject at preparation boundaries.** Capacity, unsupported host features,
    missing assets, invalid mappings, and incompatible protocols fail with
    structured diagnostics before audio execution. Runtime truncation is never
    a substitute for admission control.

## Coexistence architecture

During migration the two engines coexist behind an explicit development-only
selection:

```text
Current ProjectFile / Song
             |
             +--> current apply/load path --> SynthEngine V1
             |
             +--> LegacyProjectLowerer
                        |
                        v
                 ProjectGraphIr
                        |
                        v
                 GraphCompiler V2
                        |
                        v
                  RenderPlan V2
```

The engines should not both drive the audio device. A/B comparison is primarily
offline:

```text
same project + same render request
            |                 |
            v                 v
       V1 audio buffer    V2 audio buffer
            \                 /
             +--> comparison + analysis report
```

Once live integration begins, a development setting selects one renderer at
stream construction. There is one active engine per output stream.

## Phase 0A: Baseline, limits, and render-core contracts

Phase 0A is the entry condition for Phase 1. It establishes what V2 will be
measured against and the few contracts the render core needs before it can
compile or execute anything.

### Work

- Capture representative V1 renders and analysis results for a bounded corpus:
  - basic subtractive voice;
  - stereo oscillator or spatial voice;
  - sampler patch;
  - Mod Matrix patch;
  - YAMS control patch;
  - YAMS AudioScript patch;
  - polyphonic pad with voice stealing;
  - instrument inserts;
  - sends, returns, and master effects;
  - tempo-map arrangement;
  - shared-patch or shared-instrument edge cases.
- Record V1 behavior that V2 is intended to preserve and behavior it is intended
  to change.
- Add a comparison result model that reports at least:
  - peak and RMS error;
  - onset offset;
  - fundamental frequency and pitch drift;
  - envelope landmarks;
  - stereo correlation and channel energy;
  - spectrum-band differences;
  - integrated loudness where relevant;
  - render determinism digest.
- Measure V1 CPU and memory for the reference corpus at common polyphony and
  sample rates.
- Complete the fixed-limit and overflow audit: every current hard cap,
  truncation point, bounded queue, buffer capacity, and script budget, with its
  enforcement site, overflow behavior, and whether V2 preserves, raises,
  removes, or exposes it as configurable.
- Define an initial `HostProfile`/`RenderLimits` contract covering maximum host
  block, layouts, voices, nodes, event fan-out, channels, buses, sends,
  telemetry taps, recording buffers, prepared memory, script work, and the
  forward event-timestamp horizon of ADR-0032 clause 21. There is no backward
  horizon budget: an event earlier than the current quantum is ADR-0001 clause
  16's, and clamping it forward is not a configurable behavior.
  **The word "initial" is load-bearing, and Phase 0A closes this item on the
  field list above rather than on a complete runtime contract.** Renderer-ingress
  capacity and the deferred store are not in that list, are not derivable from
  anything in it, and have moved to [Phase 3](#phase-3-sample-accurate-scheduler-and-block-partition-invariance)
  with the ADR-0001 clarification they depend on. `max_events_per_quantum` stays
  here: it is the successor to V1's unbounded per-block event `Vec`, and only the
  runtime behaviour for a candidate set exceeding it moves. The reasoning is in
  [REV-P00A](reviews/phase-00a-exit-review.md).
- Open an architecture decision record under [decisions/](decisions/README.md)
  for every entry in the [decision register](ADR.md) whose target phase begins
  with `0A`. Accept the render-quantum semantics, the quantum frame count, the
  sample-time/event-timestamp, and the host-profile/admission decisions before
  Phase 0A closes because Phase 1 implements those contracts. Hardware time mapping may remain `Deferred` only
  to the explicit Phase 3 entry gate, and the long-running job contract only to
  the explicit Phase 4 entry gate. A deferred ADR names that target gate, its
  owner, and the evidence still required. The register — not this plan — is
  authoritative for the topic list, identifiers, status, and target phase;
  [Part VII](#part-vii-open-decisions) explains why each decision is open.

### Exit gate

- [ ] The reference corpus and comparison command can run without a GUI or
      physical audio device.
- [ ] Every known intentional V1-to-V2 semantic change has a named comparison
      category rather than being treated as generic error.
- [ ] CPU, memory, timing, and determinism baselines are saved in a reviewable
      format.
- [ ] ADR-0001 (render quantum semantics and splitting), ADR-0037 (render
      quantum frame count), ADR-0021 (host profile/admission), and ADR-0032
      (`SampleTime` and event timestamps) are `Accepted`. ADR-0001 and ADR-0037
      together carry what [Part VII](#part-vii-open-decisions) topic 1 states as
      one decision; both are required here, so the split does not weaken this
      gate.
- [ ] ADR-0022 (hardware time mapping) and ADR-0028 (long-running jobs) are
      either `Accepted` or `Deferred` to their named Phase 3 and Phase 4 entry
      gates with an owner and outstanding evidence recorded.
- [ ] Every current fixed RT/resource cap appears once in the resource
      inventory with a proposed V2 admission rule and a user-visible overflow
      diagnostic; no unexplained silent truncation is accepted as baseline
      behavior.

## Phase 0B: Migration inventories and project contracts

Phase 0B is the entry condition for Phase 10 and runs in parallel with Phases
1-4. It establishes what must not be lost in the migration, and the contracts
the project and application layers need. It is separated from Phase 0A so that
an exhaustive V1 audit cannot block the render-core evidence the rest of the
plan depends on.

Some 0B decisions gate an earlier phase, as recorded in the register's target
phase: ADR-0027 before Phase 5, ADR-0025 before Phase 6, and ADR-0024 and
ADR-0036 before Phase 9. Accept each by its earlier entry gate even if the
remaining Phase 0B work continues in parallel.

### Work

- Produce a persisted-state ownership ledger. For every field currently written
  by project, patch, bundle, or recovery save, record:
  - its domain meaning and type;
  - its current owner and every mirror;
  - which path supplies it during GUI, MCP, CLI, and recovery save;
  - how dirty state observes it;
  - how undo restores it;
  - its intended Project Core V2 owner;
  - whether it is actually project-authored state (including persisted
    `EditorMetadata`), runtime session, user settings, frontend-local transient
    state, host/service configuration, runtime job state, runtime telemetry, or
    intentionally removed.
- Produce an exhaustive capability and reachability ledger, including:
  - every current `EngineCommand` and `EngineEvent` variant;
  - all MCP tools and their read/mutate/render/service behavior;
  - every GUI action, menu/shortcut workflow, dialog-mediated operation, and
    background job;
  - every public Rust re-export and planned runtime-library entry point;
  - all module types, built-in patches, group templates, schemas, file types,
    import/export paths, and bundled examples;
  - OSC, the standalone visualizer, headless tools, configuration files, and
    other external consumers;
  - exported or tested-only subsystems such as multi-client hubs, engine command
    batches, and input multiplexers.
- Seed the generated inventory from the current known baseline: 219 MCP tools,
  75 module types, 68 programmatic built-in patches, and 12 group templates,
  plus discovered GUI/CLI/public/protocol surfaces. These counts are audit
  snapshots, not frozen product limits; newly discovered/added entries must be
  classified automatically rather than hidden by an expected-count assertion.
- Classify every capability as `migrate`, `replace`, `remove`, `defer`, or
  `compatibility adapter`, and record whether it is actually reachable in a
  shipped product path. The ledger must prevent both accidental omission and
  accidental migration of dead/aspirational architecture.
- Inventory all identities and references crossing project, GUI, MCP, undo, and
  engine boundaries. Mark raw primitives, type/order-encoded strings,
  positional references, reused IDs, and references requiring load-time repair.
- Trace the complete current mutation and reconstruction paths for at least:
  - module add/remove/connect and parameter change;
  - instrument and track creation;
  - automation edit;
  - mixer/return/master edit;
  - sample import and destructive sample edit;
  - GUI layout edit;
  - project save through GUI and MCP;
  - project load, recovery, rollback, and offline render.
- Add round-trip fixtures for data previously prone to omission: module layout,
  groups, canvas metadata, visualizers, effect order, master/return effects,
  samples, transport loop, automation, and active selection where it is
  intentionally persisted.
- Define the initial application-operation result contract: project revision,
  effect (`complete`, `partial`, or `none`), diagnostics, compile impact, and
  optional created/affected domain IDs. Effect and error severity are
  orthogonal.
- Define the Project Format V2 envelope and conversion policy at the contract
  level. Implementation waits until Phase 10D.
- Open an architecture decision record for every register entry whose target
  phase begins with `0B`, and resolve each one before Phase 0B closes. A decision
  may be deferred while the phase is active, but a deferred decision cannot pass
  the Phase 0B exit review. These cover
  project/session/settings/telemetry boundaries, persistent identity and
  remapping, transaction and concurrency semantics, the format envelope and
  unknown-field policy, asset identity, editor-metadata scope,
  track/source/channel ownership, tuning ownership, observation ownership,
  recording and take semantics, audio-device and input lifecycle, host/service
  configuration and authorization, and the supported build matrix.

### Exit gate

- [ ] Every currently persisted field appears exactly once in the ownership
      ledger and has a proposed V2 owner or an explicit removal decision.
- [ ] Every capability in the GUI, MCP, CLI, public Rust API, module/preset
      catalog, file/import/export surface, and external protocol appears once in
      the capability ledger with reachability and a migration decision.
- [ ] Every identity and cross-boundary reference appears once in the identity
      ledger with a proposed V2 rule.
- [ ] The current GUI, MCP, CLI, recovery, and rollback save sources are mapped;
      no project field remains supplied by an undocumented overlay or mirror.
- [ ] Round-trip fixtures cover the omission-prone data above and fail when a
      field is dropped.
- [ ] The stable-ID, operation-result, state-classification, and format-envelope
      contracts are recorded before their implementation begins.
- [ ] Recording/session lanes, tuning, observability, external protocol
      ownership, host/service configuration, and the supported build matrix are
      recorded before dependent work begins.
- [ ] Every ADR whose target phase begins with `0B` is `Accepted`; none remains
      `Proposed` or `Deferred` at the Phase 0B exit review.

## Phase 1: Introduce the experimental Sound Core V2 crate

### Work

- Add one experimental workspace crate, initially `synth_engine_v2`.
- Keep its dependency surface small:
  - `synth_core` for existing domain newtypes;
  - selected DSP kernels from `synth_dsp`;
  - selected module implementations or extracted kernels from `synth_modules`;
  - no GUI, MCP, CPAL, filesystem, or project-loading dependency.
- Define the host-driven renderer boundary:

```rust,ignore
pub struct RenderConfig {
    pub sample_rate: SampleRate,
    pub host_profile: HostProfile,
}

pub trait Renderer {
    fn render(&mut self, output: AudioBlockMut<'_>, events: TimedEvents<'_>);
}
```

- Define a minimal compiler IR with stable typed IDs for nodes, ports, buffers,
  parameters, scopes, and signal domains.
- Define `HostProfile` as an immutable preparation input, not global constants
  read by the renderer. Compilation returns a resource report containing
  prepared bytes, scratch bytes, voices, nodes, event capacity, channel/bus
  capacity, observation taps, and the limiting profile field.
- Define `CompileError`, `CompileWarning`, and a structured diagnostics report.
- Add an offline harness that constructs IR directly in tests and renders a
  caller-selected number of frames.
- Do not connect V2 to existing projects yet.

### Exit gate

- [ ] An empty plan and a constant/sine source render deterministically.
- [ ] Rendering accepts varying caller block sizes up to the configured maximum
      and splits every such block into the chosen fixed internal quantum.
- [ ] A plan exceeding its host profile is rejected before rendering with an
      attributable resource diagnostic; the renderer never silently clips the
      graph, event fan-out, voices, sends, or observation set to fit.
- [ ] The render loop takes no locks, performs no heap allocation, performs no
      I/O, and emits no logging.
- [ ] The crate can be deleted without affecting V1 behavior or public APIs.

## Phase 2: Minimal compiled voice graph

### Scope

Implement exactly one useful vertical sound path:

```text
note events
    -> envelope
    -> oscillator
    -> filter
    -> amplifier
    -> output
```

### Work

- Implement graph validation:
  - node and port existence;
  - direction and signal-domain compatibility;
  - channel-layout compatibility;
  - one-source or fan-in policy per input;
  - cycle detection;
  - required output validation.
- Compile stable names and IDs to compact numeric slots.
- Topologically schedule nodes.
- Insert implicit operations for fan-in, mono/stereo conversion, and output
  copies where required.
- Implement a preallocated buffer arena.
- Perform initial liveness analysis so non-overlapping signal lifetimes reuse
  buffer storage.
- Support in-place processing only when declared safe by the node.
- Implement minimal prepared-node and mutable-state separation:

```text
PreparedNode: immutable coefficients/assets/interface
NodeState:    oscillator phase/filter history/envelope stage
```

- Use existing DSP kernels where their state and configuration can be separated
  cleanly. Use a narrow temporary adapter where that is cheaper than extraction.
- Render a monophonic note through the complete path.

### Exit gate

- [ ] The hot path contains no port strings, `HashMap` lookups, graph traversal,
      topology decisions, or buffer resizing.
- [ ] The graph compiler reports a useful path-local diagnostic for an invalid
      cable and for a missing output path.
- [ ] The basic voice render is musically equivalent to V1 or has a documented
      intentional difference.
- [ ] Adding a second simple DSP node does not require changing renderer control
      flow.
- [ ] CPU use is no worse than V1 for the equivalent minimal patch, allowing a
      temporary documented margin for adapters.
- [ ] The render quantum's frame count is re-measured against real V2 nodes, and
      ADR-0037 is either confirmed or superseded. Its `Q` = 64 was accepted in
      Phase 0A on a V1 proxy that came back inconclusive (rule 1, EVD-0002), and
      the record makes this re-measurement binding rather than advisory: Phase 2
      is the last point at which changing the constant is still cheap. Until it
      passes, no hand-unrolled kernel, `Q`-specific buffer layout, or test
      asserting a control rate in Hz may depend on the value.

## Phase 3: Sample-accurate scheduler and block-partition invariance

ADR-0022 (hardware time mapping and latency ownership) must be `Accepted`
before Phase 3 implementation begins. Phase 3 may refine it through a
superseding ADR if simulated-host evidence invalidates an assumption; it may
not invent timestamp semantics inside implementation tasks.

**An ADR-0001 clarification or successor must also be `Accepted` before
implementation begins**, covering two questions the host-profile specification
could not settle from below: when clause 16's late condition is evaluated, and
whether a quantum may defer an event at all under clauses 12 and 14. A deferred
event keeps its timestamp, so once the quantum it could not enter has rendered,
a literal clause 16 counts it late — while the specification's deferral rule
forbids exactly that. The two cannot both be implemented as written. The
specification states an interim rule (the condition is asked once, when an event
first becomes due) and marks it as a narrowing of an accepted decision that it
may not make; Phase 3 is where that narrowing is decided properly or rejected.

These two obligations arrive here from Phase 0A, which narrowed P00A-T005 rather
than blocking on them: Phase 1 has no live ingress and no host callback, so the
capacity a deferral operates against cannot be specified from anything Phase 0A
can observe. See [REV-P00A](reviews/phase-00a-exit-review.md).

### Work

- Introduce the time types fixed by ADR-0032: absolute `SampleTime`,
  `FrameCount`, `FrameDelta`, `PlanPosition`, quantum-local `QuantumOffset`, and
  `StreamEpoch`. The offset type is named `QuantumOffset` because
  `synth_core::SampleOffset` is an unrelated `f32` in V1.
- Anchor `PlanPosition` to `SampleTime` in the session scheduler at play, seek,
  loop wrap, and offline range start. The tempo map produces plan positions; it
  never produces engine times.
- Define distinct fixed-size event families:
  - `PerformanceEvent` for note/controller/expression/panic input;
  - `SessionEvent` for play, stop, seek, loop transition, count-in, metronome,
    preview, and recording state;
  - compiled timeline/automation events generated from the document.
- Give live events an engine-epoch timestamp contract and an ingress mapper from
  hardware MIDI/audio timestamps. Untimestamped adapters must declare and
  measure their arrival-time fallback rather than pretending it is exact.
- **Define what V2's renderer-ingress streams are, and what bounds each.** V1
  has nothing to carry over: `LIMIT-0013`'s prioritized rings are engine egress
  and are never constructed, and `LIMIT-0012`'s ring carries commands, so there
  is no timestamped renderer-ingress queue in V1 at all. Until this is defined,
  the host profile has no field for the capacity an over-full quantum would
  defer against, and any admission rule written over one is written over a
  quantity that does not exist.
- **Bound the deferred store, and define its behaviour on exhaustion.** This
  does **not** follow from the ingress capacities and is not satisfied by
  setting them: deferring an event *frees* its upstream slot, so ingress
  capacity bounds the arrival rate rather than the backlog, and note expansion
  multiplies one released event into many. A finite store can still exhaust, and
  the audio thread may not allocate to grow it — so "no event is lost" needs
  either a proven bound or a defined loss, and currently has neither.
- Reserve sufficient note/voice identity for polyphonic pressure, per-note bend,
  release velocity, and future MPE/MIDI 2.0 mapping without requiring those
  protocols in the first implementation.
- Compile or schedule note, gate, transport, and parameter events with offsets
  inside the render quantum.
- Split processing at event boundaries or pass bounded event spans to nodes.
- Convert musical time through the tempo map into sample time without losing
  ramp semantics.
- Ensure note-on, note-off, retrigger, legato, and parameter discontinuities
  occur at their declared sample.
- Decide and implement interpolation for control-rate values between evaluation
  points.
- Make internal control rate a function of the fixed quantum, never the host
  callback size.
- Add partition-invariance tests:

```text
1 x 4096 frames
16 x 256 frames
64 x 64 frames
irregular host blocks with the same total frames
```

- Require identical output for deterministic nodes where floating-point
  operation order is unchanged. Use a narrowly justified tolerance only where
  a node algorithm makes bit identity impossible.

### Exit gate

- [ ] A note starting inside a host block begins at the exact requested sample.
- [ ] A note ending inside the same host block produces the expected non-empty
      duration rather than being released before the block renders.
- [ ] Tempo steps and ramps map to stable sample positions.
- [ ] Equivalent timestamped hardware/live and precompiled event streams reach
      the same sample offsets after ingress mapping.
- [ ] Play/stop/seek, count-in, metronome, preview, loop transition, and panic
      have declared ordering against note/controller/automation events at the
      same sample.
- [ ] The reference V2 renders are invariant to host block partitioning.
- [ ] Every renderer-ingress stream has a declared capacity in the host profile,
      and the deferred store has a declared bound together with a defined,
      counted behaviour on exhaustion. Neither may be satisfied by the other.
- [ ] The ADR-0001 clarification or successor is `Accepted`, and the host-profile
      specification's interim lateness rule is either ratified by it or replaced.
      The specification's deferral invariant may not be implemented while the two
      still contradict each other.

[Phase 7](#phase-7-yams-mod-grid-and-unified-modulation) may not begin before
this gate passes. Modulation and script timing are meaningless until the event
timing contract is stable.

## Phase 4: Current-project lowering and offline A/B path

ADR-0028 (long-running job contract) must be `Accepted` before Phase 4
implementation begins. The initial contract may deliberately leave frontend
adapters to Phase 10B, but streaming, progress, cancellation, revision pinning,
and result ownership must already have one authoritative meaning.

### Work

- Add `LegacyProjectLowerer` outside the V2 core crate. It may depend on current
  project, sequencer, module, and session types.
- Lower a bounded subset of the current model into `ProjectGraphIr`:
  - one instrument;
  - its voice patch;
  - basic parameter values;
  - a note or a simple arrangement;
  - no mixer or live commands yet.
- Resolve current string and positional identities during lowering. V2 IR must
  only contain stable typed identities after lowering.
- Feed samples and other assets as already prepared immutable data; no file
  loading reaches V2.
- Add a development-only offline engine selection to the existing render and
  analysis harnesses.
- Introduce the shared immutable `RenderRequest`/`RenderResult` contract used by
  A/B, export, analysis, CLI, and later background jobs. The request names the
  project/render revision, sample rate, range, tail/output policy, channel/stem
  selection, and resource profile.
- Make the V2 path render incrementally into a caller-owned sink/callback. A
  full in-memory audio buffer is an optional caller policy, not a renderer
  requirement.
- Define cooperative cancellation and monotonic progress at render-quantum
  boundaries so later GUI and MCP job wrappers do not need a second render
  orchestration model.
- Produce a structured A/B report for current example projects whose modules are
  in the supported subset.
- Keep project save and load unchanged. V2 is a consumer only.

### Exit gate

- [ ] At least three existing saved projects lower and render through V2 without
      hand-rebuilding their patches in tests.
- [ ] Unsupported modules and targets produce structured diagnostics naming the
      project object and reason.
- [ ] V1 remains the default for GUI, MCP, CLI, and release rendering.
- [ ] The lowerer contains compatibility knowledge; the V2 render plan does not.
- [ ] Offline V2 rendering can stream, report progress, and cancel without
      changing its deterministic output when allowed to finish.

## Phase 5: Declarative node and parameter API

ADR-0027 (observation and analyzer ownership) must be `Accepted` before Phase 5
implementation begins, so the declarative node API does not accidentally make
GUI buffers or protocol subscriptions part of authored DSP state.

### Work

- Define the long-term node interface around four concepts:
  - static/declarative interface specification;
  - off-thread preparation;
  - bounded mutable runtime state;
  - domain-specific processing.
- A module declaration must be the single source for:
  - stable module type ID;
  - ports, domains, layouts, and labels;
  - parameters, units, ranges, response curves, and defaults;
  - modulation law and smoothing policy;
  - execution scope support;
  - latency and tail reporting;
  - reset and state-layout requirements;
  - resource/cost contribution and observation/tap capability;
  - UI and discovery metadata.
- Generate or derive repetitive catalog, descriptor, schema, serialization, and
  registry surfaces where practical.
- Replace module-owned parameter composition with central parameter slots.
- Define the parameter layers explicitly:

```text
stored base
    -> absolute automation override
    -> controller layer where applicable
    -> additive/multiplicative modulation layers
    -> parameter-specific mapping and clamp
    -> smoothing or de-zipper ramp
    -> resolved DSP value/ramp
```

- `ParamSpec` must declare a modulation law. Do not pretend every parameter is a
  linear normalized value. Initial laws should cover at least:
  - normalized additive;
  - bipolar additive;
  - semitone additive;
  - decibel additive;
  - physical linear additive;
  - multiplicative gain;
  - thresholded boolean where explicitly supported;
  - non-modulatable choice values.
- Keep domain newtypes at project, API, and module boundaries. Raw numeric slots
  are permitted only in the compiled DSP representation and intermediate
  arithmetic.
- Introduce stable `NodeId` and `ParamKey`; never encode a runtime target as
  `module_type + instance index`.
- Add a temporary `LegacyPolyModuleAdapter` for modules not yet migrated. The
  adapter must be clearly marked as transitional and measured separately.
- Migrate the minimal Phase 2 modules to the native API.
- Define passive observation independently of GUI ownership:
  - a persisted analyzer/monitor node may own authored settings;
  - a compiler-inserted tap names a stable signal point and declared rate/data
    budget;
  - a runtime subscription owns buffers, generations, and consumer lifetime;
  - an unsubscribed tap is absent or reduced according to declared cost policy.
- Keep expensive spectrum/feature analysis off the audio thread unless a native
  node explicitly declares and passes its bounded RT cost. Passive observation
  must not alter the audio signal or require a GUI object during compilation.

### Exit gate

- [ ] A native simple module implements DSP without `set_param`, `get_param`,
      `get_params`, output hash maps, manual generic modulation storage, or
      engine-specific YAMS hooks.
- [ ] The same declaration drives compiler validation and user-facing discovery.
- [ ] Automation and modulation combine identically for every native module.
- [ ] Stable targets survive node reorder and insertion.
- [ ] The same project compiles headless and with GUI/OSC observation enabled;
      observation changes no audible samples or semantic project digest.
- [ ] The legacy adapter is not required by the renderer itself.

## Phase 6: Polyphony and instrument runtime

ADR-0025 (tuning representation and ownership) must be `Accepted` before Phase
6 implementation begins. Voice pitch and per-note expression must not establish
a second tuning model while the project representation remains undecided.

### Work

- Compile one immutable `VoicePlan` per patch topology.
- Separate shared prepared data from per-voice mutable state:

```text
shared across voices:
  operation schedule
  buffer layout
  prepared wavetables, samples, coefficient tables, bytecode

per voice:
  oscillator phase
  envelope/filter/script state
  note/expression/glide state
  voice-local parameter state
```

- Implement voice allocation, note ownership, release, stealing, and retrigger
  with sample-accurate events.
- Port per-note velocity, pressure, glide, legato, vibrato, expression, track
  pitch, and tuning semantics.
- Define a prepared tuning contract consumed by every pitch-producing node.
  It includes the resolved note/frequency mapping, reference pitch, keyboard
  mapping, stable tuning identity, and deterministic digest; nodes must not
  independently hardcode MIDI-to-12-TET conversion.
- Test channel-wide expression separately from note-identity expression. Voice
  allocation, stealing, sustain, retrigger, and release must preserve or clear
  expression according to one documented rule.
- Define the prepared sample-voice boundary around immutable sample maps/zones.
  The initial native sampler may select from exactly one zone, but key/velocity
  selection, root/tuning, playback region, and prepared sample reference use the
  same bounded contract that a later multi-zone implementation extends.
- Ensure voice state identity does not depend on vector order after a plan
  rebuild.
- Define oversampling first as a compiled voice-plan property, then evolve to
  rate islands after the base contract is stable.
- Preallocate the maximum configured voice state and scratch needed by a plan.
- Do not resize a live voice pool on the audio thread.

### Exit gate

- [ ] Polyphonic output is deterministic for a fixed project seed and event
      stream.
- [ ] Voice stealing begins and completes at precise sample offsets.
- [ ] Increasing configured polyphony changes prepared memory, not audio-thread
      allocation behavior.
- [ ] Shared immutable plan data is not cloned once per voice.
- [ ] A native per-voice module has no knowledge of the voice allocator.
- [ ] Built-in 12-TET and at least one non-12-TET/Scala mapping produce the same
      pitches through live, sequenced, offline, and analysis-facing paths.
- [ ] Note identity remains sufficient to route polyphonic pressure/per-note
      expression after voice stealing and plan recompilation.
- [ ] A native one-zone sampler uses the prepared map/zone contract without
      per-note allocation or a special single-sample voice API.

## Phase 7: YAMS, Mod Grid, and unified modulation

### Domain split

Treat YAMS as three execution domains sharing a language/compiler, not one
universal runtime node:

```text
YAMS Note:    timed event transformation
YAMS Control: one bounded evaluation per control segment/quantum
YAMS Audio:   per-sample audio processing
```

### Work

- Compile YAMS source off-thread into immutable program data plus:
  - interface schema;
  - stable local parameter keys;
  - source and destination declarations;
  - state-layout signature;
  - stack and scratch requirements;
  - instruction and cost estimate;
  - structured diagnostics.
- Bind textual YAMS sources to compiler slots. Runtime YAMS code must not resolve
  names or module addresses.
- Make source semantics explicit:
  - stored/base parameter value;
  - automated value;
  - previous resolved/effective value;
  - module signal output;
  - transport/context input;
  - voice/track/instrument/global macro.
- Reject an algebraic self-dependency. Feedback is allowed only through an
  explicit delay or a documented previous-quantum source.
- Integrate script dependencies into the graph schedule rather than evaluating
  them as engine-side special passes.
- Apply the scope rules defined in Part II. A voice script may not write a
  shared instrument, track, bus, or global target without an explicit reduction
  operation.
- Make script output a typed control/audio/event signal. A script never mutates
  a stored module parameter directly.
- Route all script modulation through the central parameter pipeline.
- Make YAMS local parameters ordinary stable parameter slots so they can be
  automated and externally modulated.
- Define hot-edit behavior:
  - same interface and compatible state layout: program-only swap;
  - changed interface: compile a new render plan;
  - compile failure: keep the last valid program active;
  - incompatible state: reset or crossfade according to policy.
- Seed stateful/random operations from stable project seed, node ID, voice ID,
  and stable script-state identity, never from plan order or plan revision.
- Enforce bounded execution and publish cost warnings for expensive audio-rate
  scripts multiplied by maximum polyphony.
- Lower current Mod Matrix and Mod Grid routes into the same modulation target
  representation. Preserve their product-level authoring differences without
  retaining separate target semantics in the renderer.

### Exit gate

- [ ] Control YAMS output is invariant to host callback size.
- [ ] Audio YAMS remains bounded and allocation-free at maximum configured block
      size and voice count.
- [ ] A missing, cyclic, or scope-invalid binding is rejected by the compiler
      with a source-level diagnostic.
- [ ] Automation of a YAMS local parameter uses the same parameter pipeline as a
      native module.
- [ ] Removing or renaming a script parameter cannot silently retarget an
      automation lane.
- [ ] Mod Matrix, Mod Grid, and YAMS contributions have one documented combine
      order.

## Phase 8: Mixer, channels, buses, effects, and latency

### Work

- Introduce the compiled audio hierarchy:

```text
instrument voice sum
    -> patch-level shared processing/output trim
    -> track channel inserts
    -> channel fader/pan
    -> sends
    -> group/return bus graph
    -> master graph
    -> output policy
```

- Use `ChannelId` and `BusId` for audio routing. Sends, meters, sidechains, and
  mixer automation must not be keyed by `InstrumentId`.
- Unify voice modules and effects at the native runtime processing boundary.
  Scope and placement determine lifetime; effects do not require a separate
  engine execution mechanism.
- Support explicit channel layouts and conversions:
  - mono;
  - stereo;
  - extensible multichannel metadata even if initial output remains stereo.
- Use the internal channel layout selected by ADR-0002 in Phase 2. Planar is the
  expected outcome; convert only at host or format boundaries.
- Keep internal floating-point summation linear with headroom. Saturation,
  soft clipping, and limiting are explicit nodes or explicit sink policies, not
  hidden per-channel mixing behavior.
- Preserve values above full scale in float offline output unless the selected
  render/output policy requests limiting or clipping.
- Let nodes declare latency and tail. Compile path latency and insert
  compensation where required.
- Let nodes declare supported rates. Compile oversampling islands and explicit
  up/downsampling operations after the initial whole-plan oversampling path is
  stable.
- Compile acyclic sidechains in current-quantum dependency order. Require an
  explicit delay for cyclic feedback rather than applying a hidden callback
  delay to every sidechain.
- Compile return-to-return routing as the same bus graph used for tracks and
  groups.

### Exit gate

- [ ] Two independent channels using the same patch definition retain
      independent faders, inserts, sends, meters, voices, and tails.
- [ ] Sidechain latency is reported and independent of host callback size.
- [ ] Float offline output preserves documented internal headroom.
- [ ] Node and route latency is visible in diagnostics and compensated according
      to the declared policy.
- [ ] Return and group routing require no renderer-specific topological sort per
      block.
- [ ] Common channel and bus processing remains allocation-free under the RT
      guard.

## Phase 9: Live integration and immutable plan swapping

ADR-0024 (recording take and commit semantics) and ADR-0036 (audio-device and
input lifecycle) must be `Accepted` before Phase 9 implementation begins. Live
integration may not define either policy implicitly through host-adapter or
recording code.

### Work

- Implement `AudioProcessor` for the V2 renderer or add a thin adapter over its
  host-driven buffer API.
- Implement a host-I/O adapter outside Sound Core that owns audio/MIDI device
  enumeration, requested versus negotiated configuration, stream creation,
  start/stop, disconnect detection, hotplug/reconnect, and error recovery.
  Device handles and platform callbacks never enter the render plan.
- Treat input and output as independent clock domains. The adapter owns bounded
  input buffering, resampling/drift correction, backlog policy, input-drop
  counters, and conversion of capture timestamps into the engine sample epoch.
- Publish measured/estimated input, output, and round-trip latency separately.
  Monitoring and recording choose an explicit compensation policy; neither
  relies on an undocumented callback-size approximation.
- Handle negotiated sample-rate/layout changes as an explicit stop, prepare or
  recompile, activate, and resume lifecycle. A live plan is never silently run
  with coefficients or sample assets prepared for another rate.
- Introduce separate communication lanes:
  - immutable full-plan mailbox for structural revisions;
  - timestamped live performance-event queue;
  - ordered runtime-session command queue for transport, preview, count-in,
    metronome, record arm/disarm, loop state, and panic;
  - versioned/coalescable parameter-control updates;
  - lossy telemetry/meter channel;
  - bounded recording-result channel.
- Define overflow priorities per lane. Note-off, panic, transport stop, and
  recording finalization cannot be silently displaced by lower-value continuous
  controller or telemetry updates.
- Implement note-recording state against the engine sample epoch, including
  count-in, loop boundaries, replace/overdub, held/sustained notes, quantization
  policy, and bounded partial-take diagnostics. The audio thread emits a result;
  it never mutates a pattern or project document.
- Implement audio-input monitoring as an explicit graph/host source and audio
  capture as immutable asset data plus timing/provenance metadata handed off
  off-thread. Committing either a MIDI take or recorded sample is a later
  Application Core transaction.
- Implement observation subscriptions over compiler-declared taps and general
  telemetry. Subscription buffer allocation, FFT/feature work, protocol
  encoding, and subscriber cleanup occur off the audio thread.
- Do not reproduce the monolithic V1 `EngineCommand` enum.
- Full-plan publication uses latest-wins semantics because every plan is a
  complete projection of a project revision. No structural edit is represented
  only by an increment that can be dropped.
- Publish `ProjectRevision`/`PlanRevision` and an atomic active-plan
  acknowledgement.
- Swap plans only at an internal quantum boundary.
- Return retired plans through a bounded off-thread reclamation channel. The
  audio thread never performs the final drop of a plan, program, sample asset,
  descriptor, or other potentially allocating object.
- Stage live editing in three steps:
  1. plan swap allowed only while stopped;
  2. swap with reset and short output crossfade;
  3. compatible state migration for unchanged stable nodes and voices.
- State migration matches stable IDs and compatible state-layout signatures.
  It must never infer identity from graph position.
- Add underrun, overflow, rejected-plan, and retired-plan diagnostics without
  logging from the audio callback.

### Exit gate

- [ ] V2 can be selected for live playback without changing project-save
      semantics.
- [ ] A structural project edit is either absent or present as one active plan;
      the audio thread cannot observe a partially edited topology.
- [ ] A failed compile keeps the previous active plan sounding.
- [ ] Plan swap, event delivery, and parameter updates allocate nothing and take
      no locks on the audio thread.
- [ ] Device disconnect/reconnect, negotiated buffer-size changes, and sample-
      rate changes either recover through the declared lifecycle or fail with a
      visible host diagnostic while preserving the project and last valid plan.
- [ ] Independent input/output clocks remain latency-bounded during a sustained
      monitoring test, with drift/drop counters visible and no hidden backlog
      growth.
- [ ] Timestamped MIDI/live events, monitoring, note recording, and audio
      recording have measured latency/error bounds and deterministic simulated-
      host tests requiring no physical device.
- [ ] Count-in, metronome, loop recording, replace/overdub, sustain, stop, and
      panic order correctly at quantum and loop boundaries.
- [ ] Enabling meters, scopes, or OSC/visualizer subscribers cannot alter audio,
      block the renderer, or make a headless plan require GUI-owned state.
- [ ] Retired resources are reclaimed off-thread under saturation tests.
- [ ] The active revision exposed to GUI/MCP matches the audible plan revision.

## Phase 10: Project Core V2 and Application Core V2 migration

Do not begin the broad GUI migration in
[Phase 11](#phase-11-gui-and-workflow-migration) before Sound Core V2 has passed
the offline and live render gates above. Until then, `LegacyProjectLowerer`
remains the boundary and the current project format remains the production
format.

Phase 10 is deliberately split into five gated migrations. It is not one large
model replacement hidden behind a single exit gate. The application must be
able to run a mixed state during this phase: a canonical V2 document and
application operation may still drive V1 through a temporary runtime adapter,
while the same revision compiles to Sound Core V2 for comparison.

### Target state partitions

```text
ProjectDocument                         persisted, canonical
|-- Metadata
|-- Assets
|   |-- Pattern
|   |-- AutomationClip / ControlClip
|   |-- PatchDefinition
|   |-- NoteGraph
|   |-- ModGraph
|   |-- SampleAsset
|   |-- SampleMap / SampleZone definitions
|   |-- TuningDefinition
|   `-- future Wavetable / Impulse / AudioClip assets
|-- InstrumentInstances
|-- Timeline
|   |-- Tracks
|   |   |-- InstrumentTrack
|   |   |-- AutomationTrack
|   |   |-- AudioTrack (later implementation)
|   |   `-- GroupTrack (later implementation)
|   |-- Placements
|   |-- TempoMap
|   |-- TimeSignatures
|   `-- Sections / markers
|-- MixerGraph
|   |-- ChannelStrip
|   |-- GroupBus
|   |-- ReturnBus
|   `-- MasterBus
|-- ControllerMappings
`-- EditorMetadata                       persisted presentation intent only

RuntimeSession                           never reconstructed into the document
|-- transport/playhead and preview state
|-- focused/selected/armed objects
|-- live input, monitoring, recording, and device connection state
`-- compile and active-plan coordination

UserSettings                             persisted outside the project
|-- audio/MIDI device preferences
|-- default author/directories
`-- global UI preferences

HostConfig                               deployment/service-owned
|-- MCP/OSC endpoints and protocol policy
|-- enabled platform/features and authorization policy
`-- default HostProfile / render resource budgets

RuntimeJobs                              revision-pinned, cancellable
|-- render/export/analysis/conversion jobs
|-- progress, cancellation, diagnostics, and receipts
`-- retained project/asset snapshot for job lifetime

RuntimeTelemetry                         lossy observation only
|-- meters/scopes/CPU/voice state
|-- underrun/drop counters
|-- active render revision
`-- observation/OSC subscriber state
```

The Phase 0B ownership ledger decides borderline cases. Keyboard octave, preview
glide, active selection, loop playback state, editor canvas data, render
settings, and recording state must not remain in a project merely because the
current save path can reach them.

### Phase 10A: Canonical project model and stable identity

#### Work

- Introduce `ProjectDocument` as the sole persisted authority. It is plain
  authoring data: no audio-thread objects, command senders, GUI widget state,
  locks, atomics, compiled programs, device handles, or runtime telemetry.
- Give every persistent domain concept an opaque newtype identity. At minimum:
  project asset, patch definition, instrument instance, node, parameter, track,
  placement, clip, automation lane, channel, bus, sample, graph, mapping, and
  editor-layout identities.
- Identity must not encode module type, display order, vector position, scope,
  or user-visible name. A node called `flt-1` may be displayed that way, but its
  correctness must not depend on the prefix.
- Define one central duplication/remapping service for copy, duplicate, import,
  template instantiate, patch fork, and project conversion. References are
  either remapped deliberately or become explicit unresolved references with a
  diagnostic; they never silently point at the object that later occupies a
  reused slot.
- Introduce `PatchDefinition` as the reusable sound-design asset. It owns the
  authoring node graph, defaults, descriptions, interface, canonical asset
  references/preparation intent, and patch-level shared processing.
- Introduce `InstrumentInstance` as a project instance referencing a patch. It
  owns performance configuration such as polyphony, allocation mode, tuning,
  glide, MIDI/controller mapping, and instance parameter overrides.
- Introduce `TuningDefinition` as authored project data with stable identity,
  reference pitch, scale/mapping semantics, and optional Scala/KBM source or
  asset references. An `InstrumentInstance` references it or the declared
  project default; the prepared frequency table is compiled data, not the only
  persisted representation.
- Separate sample audio identity from sampler mapping. `SampleAsset` owns source
  audio/provenance; `SampleZone` owns key/velocity ranges, root/tuning, playback
  region, loop, gain, and selection policy; a reusable `SampleMap` groups zones.
  The initial one-sample sampler is represented as one zone rather than a
  special incompatible model.
- Make the normal product invariant:

```text
one InstrumentTrack
    -> one InstrumentInstance
    -> one ChannelStrip
```

- Permit several tracks to share a `PatchDefinition`, but give them separate
  runtime instances and channels by default.
- Model advanced many-event-track-to-one-instrument routing explicitly later;
  do not obtain it accidentally by sharing an ordinary track instrument ID.
- Split patch-level sound processing from channel inserts. Give patch output
  trim, instance trim, and channel fader distinct identities and target paths.
- Keep patterns instrument-neutral. Placement and track binding supply source,
  channel, and scope context during event compilation.
- Remove persisted per-note track ownership where it is vestigial. Keep editor
  voice lane and event-source identity as distinct concepts.
- Split automation by owner:
  - clip automation: relative and follows a placement;
  - track automation: absolute song timeline owned by a track;
  - project automation: buses, master, and project-scoped targets;
  - tempo map: separate time-domain authority.
- Use stable relative target references in reusable clips:

```text
ThisTrack.Channel.Volume
ThisTrack.Source.OutputTrim
ThisTrack.Source.Node(NodeId).Param(ParamKey)
```

- Use explicit stable targets for cross-track and project automation.
- Make `AutomationTrack` a first-class track kind with no instrument instance or
  audio channel. It produces parameter/control events only.
- Support an optional target binding for an automation track so lanes can use
  `ThisTarget` relative addressing.
- Reserve `AudioTrack` and `GroupTrack` variants only when their semantics are
  ready; do not persist empty placeholders solely for future compatibility.
- Move persistent presentation intent into typed `EditorMetadata` keyed by
  stable project identities: node positions, groups, canvas layout, colors, and
  other information intentionally saved with the document.
- Keep transient selection, hover, open popups, drag state, scroll caches, live
  meters, and compile progress out of `EditorMetadata`.
- Persist analyzer/monitor nodes and their authored display/DSP settings only
  when they are intentional project content. Runtime sample buffers, FFT frames,
  subscriber lists, freeze snapshots, meter peaks, and OSC connection state are
  never `EditorMetadata` or graph serialization fields.
- Give the document one content revision and an optional stable project seed.
  Subsystem generation counters may remain caches but are not competing notions
  of document truth.

#### Exit gate

- [ ] The ownership ledger maps every persisted V1 value to exactly one V2
      document field or an explicit removal decision.
- [ ] No V2 domain reference relies on a raw primitive, type prefix, display
      name, vector position, or current engine slot.
- [ ] An automation-only track requires no dummy instrument, channel strip, or
      meter.
- [ ] Two tracks sharing a patch definition remain independently mixable.
- [ ] A one-sample sampler and a multisample-ready map use the same zone/mapping
      model; adding a second key/velocity zone does not require a voice, project,
      or GUI data-model replacement.
- [ ] Project/default and per-instance tuning references round-trip without
      persisting prepared runtime tables as canonical truth.
- [ ] A reusable pattern or automation clip contains no accidental concrete
      track or instrument dependency.
- [ ] Serializing `ProjectDocument` requires no GUI or engine snapshot.

### Phase 10B: Application operations and transactions

#### Work

- Introduce Application Core V2 as the only mutation boundary for persisted
  project state. GUI, MCP, CLI, importers, tests, and future hosts submit the
  same typed operations.
- Define an operation vocabulary around domain intent rather than engine
  commands: create track, connect nodes, set parameter base value, place clip,
  edit automation, import asset, change routing, fork patch, and similar
  operations.
- Define a common operation result containing:
  - input and resulting project revision;
  - effect: `complete`, `partial`, or `none`;
  - diagnostics with stable codes, severity, and object paths;
  - created and affected domain IDs where applicable;
  - compile impact: none, parameter/control update, timeline rebuild, graph
    compile, asset preparation, or full plan compile.
- Keep effect and error verdict independent. A script source may be stored while
  compilation fails; that is a partial effect with an error, not either total
  success or no effect.
- Make multi-object edits atomic by default. An operation validates against one
  input revision, constructs a candidate document, checks invariants, and then
  publishes one resulting revision.
- Permit explicitly partial import/conversion operations only when every skipped
  object is represented in the result and the caller opted into partial
  behavior.
- Support optimistic concurrency with an expected base revision. A stale
  frontend receives a conflict result; it does not overwrite a newer project or
  infer success from queued engine commands.
- Separate operation validation from Sound Core compilation. A structurally
  valid authoring edit may still fail to compile; the edited document remains
  visible while the previous valid render plan continues sounding.
- Add a compile coordinator that coalesces superseded project revisions without
  hiding which revision is editing, compiling, rejected, or audible.
- During coexistence, provide a narrow V1 runtime adapter that applies a
  committed project operation or full document projection to V1. V1 commands
  are an implementation detail, not the public application API.
- Make read APIs consume immutable project snapshots plus separately identified
  runtime telemetry. A read must say which project and active render revisions
  its result describes where that distinction matters.
- Define `CommitRecordedTake` and `ImportRecordedAudio` as ordinary atomic
  application transactions. They validate the target still exists, map the
  engine-epoch result into musical/project time, retain overflow/latency
  diagnostics, create stable note/asset/placement IDs, and obey replace/overdub
  semantics without letting the audio thread edit canonical state.
- Add a revision-pinned job service for render, export, audio analysis, symbolic
  analysis where asynchronous, and conversion. Starting a job captures one
  immutable project/asset snapshot; progress/cancellation/result state is
  runtime data and job completion does not silently mutate the document.
- Keep pure symbolic analysis and composition transforms below frontend wire
  types. They consume canonical domain snapshots and return domain results or
  proposed application operations; they do not depend on `EngineState`, MCP
  DTOs, or GUI state.

#### Exit gate

- [ ] GUI-independent tests can perform representative edits using only
      application operations and inspect the resulting document revision.
- [ ] The same operation produces the same document and diagnostics through
      direct, MCP, CLI/import, and test adapters.
- [ ] A failed compile leaves the committed authoring revision inspectable and
      the previous valid render revision audible.
- [ ] No operation reports complete effect when a requested object, field, or
      asset was dropped.
- [ ] No public application operation exposes V1 or V2 engine command types.
- [ ] Note/audio recording results commit through the same transaction,
      history, diagnostics, and optimistic-concurrency rules as hand edits.
- [ ] A long-running job remains pinned to its named revision while later edits
      continue, can be cancelled, and never reports output as belonging to a
      newer project revision.
- [ ] Symbolic analysis/composition domain code imports no MCP, GUI, or audio-
      engine state types.

### Phase 10C: History, dirty state, save, and recovery

#### Work

- Derive undo and redo from committed application changes, not from view-owned
  inverse logic. Choose and document one of:
  - invertible typed operations;
  - before/after domain fragments in a `ChangeSet`;
  - persistent document snapshots with structural sharing;
  - a measured hybrid for large immutable assets.
- Preserve gesture coalescing, but move its semantic boundary to application
  transactions. A knob drag, note drag, or automation gesture commits one
  history entry regardless of frontend.
- Represent compound edits as one transaction with one inverse. Create-and-place,
  delete-with-references, patch fork, and converter changes must not leave a
  half-undoable project.
- Derive dirty state from document identity/history position relative to the
  last successful save. Remove subsystem fingerprints and untracked-mutation
  escape hatches only after every persisted mutation is forced through the
  operation boundary.
- Save an immutable `ProjectDocument` snapshot at a named revision. Never rebuild
  the save payload from audio-thread mirrors, GUI overlays, or asynchronously
  published control snapshots.
- Feed autosave and crash recovery the same canonical snapshot and revision as
  manual save.
- Keep large sample/audio edits memory-bounded by sharing immutable asset blobs
  and measuring retained history cost. History eviction must not invalidate the
  current document or its asset references.
- Keep blobs referenced by the current document, retained history, active save,
  recovery snapshot, compile, or runtime job alive. Orphan collection is an
  explicit off-thread operation with a dry-run/reference report; deleting a
  project reference never races an active plan or makes undo unrecoverable.
- Define whether remote operations enter the user's undo history. The default
  should be one coherent project history; a remote transaction may be marked as
  externally authored but must not become an invisible mutation.
- Keep current atomic replacement, destination file-permission preservation,
  and recovery lifecycle behavior. This phase changes the data source, not the
  safety guarantees; remote authorization is a separate Phase 10E host concern.

#### Exit gate

- [ ] Every persisted application operation is undoable or explicitly declared
      non-undoable before execution; no frontend can bypass that declaration.
- [ ] Undoing to the saved history position reports clean without consulting
      engine, GUI, sample-library, or mixer fingerprints.
- [ ] GUI save, MCP save, autosave, recovery, and rollback capture semantically
      equivalent canonical document snapshots for the same revision.
- [ ] Save can proceed while audio renders because it snapshots canonical data,
      not a synchronized reconstruction of runtime state.
- [ ] Undo/redo, dirty state, and recovery remain correct under failed compile,
      stale frontend revision, and large immutable asset tests.

### Phase 10D: Project Format V2, assets, and conversion

#### Work

- Define a typed format envelope with an exact discriminator and format version.
  Parsing must reject unsupported versions before deserializing them as the
  current model.
- Separate four steps with distinct result types:
  1. decode the container/envelope;
  2. migrate or convert its version;
  3. validate references and domain invariants;
  4. open the resulting `ProjectDocument` and compile it.
- Unknown or removed fields must never disappear silently. Either reject them or
  retain/report them under an explicit compatibility policy chosen for that
  format version.
- Keep JSON as the initial inspectable document encoding unless measurement or a
  concrete feature requires another encoding. Format V2 is about ownership,
  validation, and versioning, not gratuitous binary serialization.
- Define a project package manifest for embedded assets. It records document
  entry, asset entries, content type, size, content digest, and stable asset ID.
- Give samples, wavetables, impulse responses, scripts/bytecode sources where
  appropriate, and future audio clips one asset-reference model.
- Distinguish source representation from prepared runtime representation:
  - preserve declared/native sample rate, channel layout, encoding metadata,
    source digest, and authored crop/loop/zone information;
  - derive renderer-rate PCM, mipmaps, resampling kernels, decoded impulse
    responses, and similar caches from source plus preparation settings;
  - key prepared caches by source digest and preparation profile rather than
    treating an eager 48 kHz conversion as canonical project data.
- Define source provenance and privacy. Imported local paths, recording device
  names, timestamps, and external URIs have explicit persist/package/redact
  policy and are not leaked in portable projects by accident.
- Define asset lifecycle and garbage collection across document snapshots,
  history, autosave/recovery, active plans, and jobs. Removing the last visible
  project reference does not destroy a blob still required for undo or render.
- Decide embedded versus external assets explicitly:
  - embedded assets produce a portable package;
  - external assets retain a declared URI/path plus expected digest;
  - missing or digest-mismatched assets produce structured diagnostics;
  - silent replacement by a same-named file is forbidden.
- Use immutable or copy-on-write asset blobs so project snapshots, undo, offline
  render, and compilation can share canonical source data safely. Separately
  keyed prepared caches may be shared where their preparation profile matches.
- Align standalone patch/preset and template formats with the same canonical
  node, parameter, identity, and asset types. A preset may be a smaller envelope,
  but it must not maintain a second incompatible graph schema.
- Make sample maps/zones and tuning definitions use the same asset/reference
  and conversion machinery. Full disk streaming, slicing, and AudioTrack remain
  deferred features, but their source identity and prepared-data boundary must
  not require a new project asset taxonomy.
- Build a one-shot current-format converter. It must:
  - convert every current instrument into a `PatchDefinition` plus at least one
    `InstrumentInstance`;
  - convert each current sequencer track into an `InstrumentTrack` and channel;
  - detect several tracks sharing one current runtime instrument and require an
    explicit independent-instance or shared-runtime preservation choice;
  - convert current pattern host-track automation into relative clip
    automation;
  - convert pinned automation into track/project automation as appropriate;
  - convert module, effect, send, bus, sample, graph, and GUI-layout references
    to stable V2 identities;
  - report every default, repair, ambiguity, omission, and unsupported object.
- Resave bundled examples and regenerate schema fixtures after the format is
  selected. Permanent backward-compatible deserialization is not required.
- Compile and round-trip every programmatic built-in patch and group template,
  not only files under the bundled examples directory.
- Add a semantic project digest independent of JSON field ordering and pretty
  formatting. Use it for round-trip and migration tests, not as an audio digest.

#### Exit gate

- [ ] Every bundled example converts with an empty unexplained-loss set.
- [ ] Load-save-load preserves the semantic project digest, asset digests,
      stable identities, editor metadata, and unresolved-reference diagnostics.
- [ ] A newer, older, malformed, or wrongly discriminated file cannot be
      mistaken for the current format.
- [ ] Missing, corrupt, or mismatched embedded/external assets are named before
      playback or render and cannot produce an apparently clean load.
- [ ] Standalone patches/templates and full projects share one graph and
      parameter serialization model.
- [ ] All built-in patches, group templates, sample maps/zones, and tuning
      definitions validate, round-trip, and compile or produce an intentional
      named exclusion.
- [ ] Source/native asset identity survives preparation at two render sample
      rates; cache differences do not alter the semantic project digest.
- [ ] Asset orphan collection preserves blobs retained by undo, recovery,
      active plans, and jobs, and provenance redaction follows package policy.
- [ ] `LegacyProjectLowerer` is no longer needed for newly saved projects.

### Phase 10E: MCP, CLI, import, and service migration

#### Work

- Move MCP mutating tools onto Application Core operations. MCP input/output
  schemas adapt the domain API but do not duplicate its validation or mutation
  rules.
- Move project reads, discovery, lint, analysis setup, and save onto immutable
  canonical snapshots plus explicitly revisioned runtime telemetry.
- Map every MCP tool from the Phase 0B capability ledger to a domain read,
  application operation, runtime-session command, telemetry subscription,
  revision-pinned job, compatibility adapter, or explicit removal. Tool count
  parity alone is insufficient; effect/revision semantics must be classified.
- Move CLI rendering, project conversion, exporters, importers, and future
  runtime-library setup onto the same Project Core and Application Core entry
  points.
- Generate or derive wire schemas, project schemas, and discovery metadata from
  the same domain and operation declarations where practical. Do not expose a
  universal untyped operation just to avoid writing adapters.
- Make batch operations application transactions or ordered transaction groups
  with explicit effect and rollback semantics. Rollback restores a canonical
  revision, never an engine-reconstructed approximation.
- Give each frontend a conformance suite over a shared operation corpus. The
  corpus asserts resulting semantic project digest, diagnostics, affected IDs,
  and compile impact rather than frontend-specific prose.
- Remove non-GUI save requests for GUI-built projects once editor metadata is in
  the canonical document; every frontend already sees the same revision.
- Keep V1 runtime adaptation behind Application Core until default cutover. MCP
  and CLI must not select correctness rules based on which engine is active.
- Put long-running render/analysis/export calls behind the common job service.
  GUI, CLI, and MCP task/progress/cancellation adapters observe the same job and
  receipt contract instead of each owning a render loop.
- Migrate OSC to the runtime telemetry/observation facade. Version the protocol,
  preserve or explicitly break each address/payload contract, and test the
  standalone visualizer as an external consumer without giving it access to
  `EngineState`.
- Separate `UserSettings` from deployment-owned `HostConfig`. Audio/MIDI user
  preferences, MCP/OSC network configuration, feature/platform capability, and
  resource budgets must each have one owner and an explicit load/default/error
  policy.
- Decide remote authorization at the service boundary. If remote mutation or
  multiple clients remain in scope, authenticate/authorize before Application
  Core operations and session commands; do not port engine-level permission or
  priority code as the domain model. If not supported, declare local-only as a
  tested host policy.
- Define a narrow public runtime/application facade and dependency-direction
  rules. Do not continue re-exporting internal engine/session implementation
  types as the accidental long-term API.

#### Exit gate

- [ ] GUI-independent MCP, CLI, importer, and direct-operation tests produce the
      same canonical document for the shared conformance corpus.
- [ ] MCP save and rollback cannot flatten or omit editor metadata because they
      never reconstruct it from the engine.
- [ ] Batch rollback, stale-revision conflict, partial import, and failed compile
      have one effect/diagnostic vocabulary across frontends.
- [ ] No non-GUI service mutates persisted project state through `SynthSession`,
      `EngineCommand`, `SharedSong::write`, or an equivalent bypass.
- [ ] The GUI is the only remaining frontend awaiting view-by-view migration in
      Phase 11.
- [ ] Every current MCP tool, CLI command, OSC message, public facade entry, and
      external consumer has a tested adapter or an explicit removal/break
      decision recorded in the capability ledger.
- [ ] Render/analysis progress and cancellation are consistent across GUI, CLI,
      and MCP; no frontend blocks a service executor for the duration of DSP.
- [ ] OSC/visualizer consumes revisioned telemetry without importing engine
      state, and protocol-version mismatch is visible rather than silent.
- [ ] Host/service configuration and authorization cannot mutate project truth
      or leak onto the audio thread.

## Phase 11: GUI and workflow migration

### Work

- Migrate one view at a time behind an application-facing presenter/controller
  boundary. Each migrated view reads an immutable project snapshot and emits
  typed intents/operations; it does not hold a writable domain model.
- Keep the existing high-level views where they remain useful: Rack, Note Grid,
  Mod Grid, Pattern, Arrangement, Mixer, Sample, and analysis surfaces.
- Change their data source from engine snapshots and view-owned persistence to
  the canonical application model.
- Split GUI state into:
  - persisted `EditorMetadata` edited through application operations;
  - transient per-view interaction state such as selection, scroll, drag,
    popup, and cached geometry;
  - revisioned runtime telemetry read through a separate facade.
- Remove save-time GUI overlays as soon as each corresponding editor-metadata
  field has a canonical owner. A view must never be queried to complete a save.
- Move domain validation, ID creation/remapping, reference cleanup, and compound
  mutation semantics out of view code. GUI code chooses user intent and
  presentation, not project invariants.
- Introduce a compiler-status surface:
  - editing revision;
  - compiling revision;
  - active/audible revision;
  - compile warnings/errors;
  - plan CPU/memory/latency estimate.
- Add host/session status surfaces without treating them as project edits:
  negotiated audio/MIDI devices, sample rate/buffer/layout, input/output/round-
  trip latency, reconnect state, input drops/drift, event overflows, record arm,
  count-in, metronome, active take, and take-commit diagnostics.
- Drive meters, scopes, analyzers, and OSC/visualizer status through the
  observation/telemetry facade. Views subscribe/unsubscribe; they never inject a
  GUI-owned buffer into a project graph or audio module.
- Present render/export/analysis through the shared job model with revision,
  progress, cancellation, warnings, output/receipt, and stale-result labeling.
- Rack edits mutate `PatchDefinition` or a unique instance override through
  transactions. Offer explicit "make unique" when editing a shared patch should
  fork it.
- Mixer strips represent `ChannelStrip`/`BusId`, not instruments.
- Track headers render controls appropriate to the track kind:
  - instrument track: source, note controls, channel state;
  - automation track: target binding, enable/bypass, lane controls;
  - group track: bus controls;
  - audio track later: clip and input controls.
- Add automation-track creation and editing without showing a dummy instrument,
  VU meter, or audio solo semantics.
- Distinguish automation-track enable/bypass from audio mute/solo. Disabling an
  automation layer must reveal the underlying base/other layers; it must not
  latch the last emitted value.
- Surface conflicting absolute automation writers and scope-invalid modulation
  as compiler diagnostics at the relevant lane/node.
- Preserve text-edit input gating, undo coalescing, atomic save, recovery, and
  other existing project-safety behavior through the application operation
  layer.
- Decompose the current application backend by responsibility only as views
  migrate. Do not perform a separate cosmetic rewrite or one-shot file split;
  remove obsolete coordination fields when their last direct-mutation caller is
  gone.

### Exit gate

- [ ] All structural audio edits compile off-thread and publish a complete plan.
- [ ] No view writes persisted state that the canonical project snapshot cannot
      see.
- [ ] Saving never waits for, requests, or overlays state from a GUI frame.
- [ ] The mixer, arrangement, Rack, automation, YAMS, Note Grid, and Mod Grid
      workflows operate against V2 concepts without V1 engine mirrors.
- [ ] GUI operations pass the same application conformance corpus as MCP, CLI,
      and direct-operation adapters.
- [ ] Compile failures are actionable and do not stop the last valid sound.
- [ ] Recording a note take or audio sample, then committing/undoing it, follows
      the same Application Core history path as an equivalent manual edit.
- [ ] Closing a scope/analyzer view releases its subscription without changing
      the project, audio, or active plan except where an explicitly persisted
      analyzer node itself was edited.

## Phase 12: Default cutover and V1 retirement

### Preconditions

- Every supported module either has a native V2 node or a reviewed intentional
  exclusion.
- The reference project corpus renders successfully through V2.
- Live, offline, analysis, CLI, and future runtime-library paths use the same
  render core.
- Project load/save no longer reconstructs canonical state from V1 engine
  snapshots.
- V2 passes all quality, real-time, determinism, and workflow gates below.
- The capability ledger has no unclassified reachable entry and no migrated
  tested-only subsystem mistaken for a product requirement.
- Host-I/O simulation, recording, job cancellation, OSC/visualizer protocol,
  public facade, feature matrix, MSRV, and supported-platform gates pass.

### Work

- Make V2 the default engine for development builds, then release builds.
- Keep one temporary opt-in V1 fallback for a bounded release window only if it
  materially helps diagnose regressions.
- Stop adding engine-level features to V1 after V2 becomes the development
  default.
- Remove V1-specific command rings, control snapshot mirrors, deferred-drop
  protocols, graph rebuild paths, and duplicated offline orchestration once no
  caller remains.
- Rename `synth_engine_v2` to the final engine crate name only after V1 removal.
- Reassess `synth_core`: move module-catalog-specific parameter types outward so
  the core contains genuinely shared domain and runtime contracts.
- Update architecture documentation, screenshots where UI changed, examples,
  schemas, packaging, MCP documentation, and the game-runtime plan.
- Remove dead exported V1 architecture and narrow public re-exports to the
  reviewed Project/Application/Sound/host facades.

### Exit gate

- [ ] No production path instantiates V1.
- [ ] No feature flag or hidden setting can select a stale engine accidentally.
- [ ] The V2 crate no longer carries migration-only adapters that are not needed
      for project conversion.
- [ ] The full workspace quality gate passes.
- [ ] Supported no-default/default/all-feature combinations, MSRV, Linux,
      macOS, Windows, release packaging, and the separately built visualizer
      consumer pass the declared matrix.
- [ ] All built-in patches/templates and every retained external protocol/public
      facade consumer pass conversion/compile/conformance checks.
- [ ] V1 removal is covered in release history and the new architecture is the
      only documented architecture.

---

# Part II: Target Architecture

## Layer boundaries

```text
GUI / MCP / CLI / importers / tests / runtime facade
                         |
                         v
                  Application Core V2
          operations / transactions / history
             /             |              \
            v              v               v
       diagnostics   canonical revision   compile coordinator
                           |                       |
             +-------------+-------------+         |
             |                           |         |
             v                           v         v
       Project I/O V2              ProjectGraphIr builder
    encode/decode/assets                   |
                                           v
                                    GraphCompiler V2
                                / diagnostics   \ prepared plan
                               v                 v
                         frontend feedback   RenderPlan + RuntimeState
                                                   |
                                                   v
                                          Sound Core renderer
                                                   |
                                                   v
                                  host audio/MIDI I/O + service adapters

RuntimeSession, RuntimeJobs, RuntimeTelemetry, and HostConfig are adjacent to,
not children of, the ProjectDocument. UserSettings and HostConfig are owned by
the host application/deployment outside the project package.
```

Rules:

- Application Core owns validation, transactions, history semantics, and
  project revisions.
- Project Core is optimized for authoring, persistence, and deterministic
  frontend-independent mutation.
- Project I/O owns format envelopes, version conversion, package manifests, and
  asset resolution. It does not compile or apply engine commands.
- The compiler IR contains semantic graph identities but no GUI objects or
  protocol types.
- The prepared render plan contains compact runtime identities and no user-facing
  names in the hot path.
- The renderer cannot query or mutate the project model.
- The audio thread publishes status and telemetry, never canonical project
  state.
- Runtime telemetry cannot become persisted truth merely because a frontend can
  read it.
- Host adapters own devices, external clocks, protocols, files, worker
  scheduling, and authorization. Sound Core owns none of those facilities.
- Dependencies point inward: frontend/protocol/host crates may adapt domain and
  renderer facades; Project/Application/Sound cores never import GUI, MCP, OSC,
  CPAL, standalone-visualizer, or wire-schema types.

## Authoring graph, compiler IR, and render plan

### Authoring graph

Optimized for editing, persistence, diagnostics, and user intent. It contains:

- stable IDs;
- names and descriptions;
- module and parameter declarations;
- cables;
- groups and layout;
- reusable asset references;
- automation and modulation intent;
- temporarily incomplete edits if the transaction model permits them.

### Compiler IR

Optimized for validation and transformation. It contains:

- resolved stable node and parameter IDs;
- explicit signal domain, rate, channel layout, and scope;
- explicit source and destination bindings;
- prepared asset references;
- implicit-conversion requirements;
- event and automation timelines;
- no GUI layout or transport protocol objects.

### Render plan

Optimized for execution. It contains:

- ordered operations;
- numeric input/output/parameter/state slots;
- precomputed fan-in and conversion operations;
- buffer arena layout and lifetimes;
- immutable prepared node data;
- fixed-size mutable state layout;
- event-routing tables;
- latency and tail metadata;
- stable-plan identity and project revision;
- no validation branches, strings, hash maps, filesystem paths, or construction
  logic in the render loop.

## Signal domains

Initial domains:

```text
Audio    sample streams with declared channel layout and rate
Control  constant, ramp, or bounded control-rate stream
Gate     sample-timed transitions with conventional low/high semantics
Event    timed note/controller/transport events
```

Potential future domains such as spectral frames must be added explicitly, not
encoded as arbitrary audio buffers without timing and layout metadata.

Conversions are compiler operations with documented latency and cost:

```text
Control -> Audio: interpolation/expansion
Gate -> Control: low/high value stream
Event -> Gate: event-to-gate generator
Mono -> Stereo: duplication or declared pan law
Stereo -> Mono: declared downmix
Rate N -> Rate M: explicit resampling
```

Invalid implicit conversions are compile errors. Audio-to-control conversion
requires an explicit follower/analyzer node because it is a signal-processing
choice, not a representation cast.

## Execution scopes

Initial scope hierarchy:

```text
Global
  -> Bus
      -> Channel/Track
          -> InstrumentInstance
              -> Voice
```

Outer-to-inner values may broadcast when declared compatible. Inner-to-outer
values require an explicit reduction:

```text
Global -> Voice: allowed broadcast
Track -> its InstrumentInstance/Voice: allowed broadcast
Voice -> Track/Bus/Global: rejected without ReduceSum/Mean/Max/etc.
```

The compiler must reject ambiguous scope crossings. Runtime operation order may
not decide which voice wins.

## Graph scheduling and feedback

- Acyclic dependencies are topologically scheduled once during compilation.
- Multiple sources to an input produce an explicit fan-in operation or are
  rejected according to the port policy.
- An instantaneous dependency cycle is a compile error.
- Feedback is legal only through a node/operation declaring positive latency.
- Strongly connected components may be scheduled when every cycle includes a
  delay boundary.
- Previous-quantum semantics are explicit in source or delay metadata, never an
  accidental result of reading an old buffer.
- The compiler emits the effective feedback latency in diagnostics.

## Buffer arena and operation plan

- Buffers are allocated off-thread for the compile-time quantum and admitted
  channel layout.
- Port names compile to `BufferSlot` or event/control slot IDs.
- Liveness analysis reuses storage after the last consumer.
- In-place processing is used only when the node declares safe aliasing.
- Fan-out uses shared read views unless a consumer requires mutable ownership.
- The compiler accounts for oversampling, latency compensation, and crossfade
  scratch in the plan size.
- Bounds are validated at compile time. Runtime slices are constructed from
  validated ranges without unsafe code unless a separate reviewed decision
  changes the project-wide unsafe policy.

Illustrative operation plan:

```text
Process envelope: event slot 0 -> control buffer 0
Process LFO:                     -> control buffer 1
Resolve cutoff ParamSlot 7 from base/automation/mod buffer 1
Process oscillator:             -> audio buffer 2
Process filter: audio buffer 2  -> audio buffer 3
Resolve gain ParamSlot 12 from envelope buffer 0
Process amplifier in place on audio buffer 3
Route audio buffer 3 to instrument output
```

## Resource profiles and admission

`HostProfile` is an explicit preparation input describing available/reviewed
capacity, not a bag of renderer globals. It covers at least:

- maximum host block;
- channel layouts and sample-rate/rate-factor range;
- nodes, voices, channels, buses, sends, event fan-out, and delayed events;
- parameter/control/event slots and observation taps;
- prepared immutable bytes, mutable state bytes, buffer/scratch bytes, and
  crossfade/retirement budget;
- YAMS instructions/state/emits multiplied by scope and polyphony;
- recording-result and realtime communication capacities.

Compilation produces a `ResourceReport` with requested, available, and dominant
contributors. Exceeding a hard capability is a compile/preparation error;
approaching a configurable performance budget may be a warning or configured
refusal. Runtime overflow is reserved for genuinely live bounded queues and is
counted/reported. It never silently changes authored topology, automation,
sample mapping, note expansion, routing, or polyphony.

## Node contract

A native node should provide:

```text
NodeSpec      declarative, immutable interface and metadata
prepare       off-thread construction of immutable prepared data
StateLayout   bounded mutable-state description
reset         real-time-safe state reset
process       domain-specific bounded processing
```

The generic renderer should not contain language-specific or module-specific
methods. YAMS, sampler hydration, modulation matrices, pitch-aware sources, and
visualizers are represented by node kinds, prepared assets, events, parameter
slots, or taps rather than optional methods on every module trait.

## Parameter and target model

Every runtime parameter has:

- stable owner identity;
- stable `ParamKey`;
- typed/default authoring value;
- accepted unit/range;
- response/mapping curve;
- automation support;
- modulation support and law;
- smoothing policy;
- resolved runtime slot;
- explicit scope.

Target references are stable and semantic:

```text
GlobalTarget
BusTarget(BusId, ParamKey)
ChannelTarget(ChannelId, ParamKey)
TrackTarget(TrackId, ParamKey)
InstanceTarget(InstrumentInstanceId, ParamKey)
NodeTarget(InstrumentInstanceId, NodeId, ParamKey)
RelativeTarget(ThisTrack/ThisTarget/ThisInstance, ...)
```

The compiler resolves them to slots. A missing target remains a structured
orphan/diagnostic; it must never fall through to another slot.

### Multiple writers

- At most one active absolute automation writer may control a target at a sample
  unless an explicit priority/blend policy is declared.
- Multiple additive or multiplicative modulators may compose according to the
  target's modulation law.
- Conflicting absolute writers are compile errors or explicit warnings selected
  by product policy; they are never resolved by iteration order.
- Disabling an automation or modulation layer recomputes the target from the
  remaining layers. The last emitted value does not latch.

## Timing model

- Musical authoring time remains tick/beat based.
- Compiler/scheduler conversion produces absolute sample time for a concrete
  tempo map and sample rate.
- Live events carry an absolute or engine-epoch sample time and are converted to
  quantum-local offsets.
- Nodes receive exact event spans for their processing segment.
- A fixed internal quantum defines control-rate evaluation and plan-swap
  boundaries.
- Host callback size is an adapter concern and does not alter synthesis,
  modulation, scripts, or automation timing.
- Offline and live rendering use the same segmenter and render operations.

## Host I/O and clock domains

- Sound Core exposes a host-driven buffer/event API and has no CPAL, MIDI,
  filesystem, network, device, or system-clock dependency.
- The host adapter owns requested and negotiated device configuration, callback
  partitioning, channel conversion at the boundary, start/stop, disconnect,
  hotplug/reconnect, and platform errors.
- Output sample time is the renderer epoch. Hardware MIDI/input timestamps are
  mapped into that epoch with a declared calibration/fallback and error bound.
- Audio input and output may use independent hardware clocks. Bounded buffering,
  asynchronous resampling/drift correction, and backlog/drop policy live in the
  host adapter and publish telemetry.
- Input, output, and round-trip latency are distinct values. Monitoring,
  preview, note recording, audio capture, and offline rendering each declare
  whether and how they compensate latency.
- A negotiated sample-rate/layout change invalidates prepared data according to
  its declared dependencies and follows an explicit stop/prepare/activate/resume
  lifecycle.
- Simulated hosts with controllable timestamps, drift, block sizes, disconnects,
  and negotiation results are the primary correctness harness; physical-device
  tests are supplementary.

## Performance, session, and recording model

Four concepts remain separate:

```text
ProjectOperation   persisted domain mutation through Application Core
SessionCommand     transport/preview/record-arm/metronome/device intent
PerformanceEvent   timestamped note/controller/expression/panic input
RecordingResult    bounded engine output awaiting an application transaction
```

- Project operations never enter the audio callback as incremental structural
  commands; they compile into plans/control timelines.
- Session commands have explicit ordering at equal sample time and do not enter
  project history unless a later application transaction commits authored data.
- Performance events carry channel/source and sufficient note identity for
  polyphonic expression. MPE/MIDI 2.0 adapters may be deferred, but the event
  identity must not preclude them.
- Note recording is based on renderer sample time, then mapped to musical time
  using the captured tempo/loop context. Count-in, metronome, replace/overdub,
  sustain/held notes, quantization, and overflow are explicit take metadata.
- Audio capture produces immutable source audio plus sample rate, layout,
  timestamp/latency, drop, and provenance metadata. Monitoring is not itself a
  persisted recording.
- `CommitRecordedTake`/`ImportRecordedAudio` validates the target revision and
  creates canonical notes/assets/placements atomically; stale targets produce a
  conflict or explicit retarget flow rather than a silent partial commit.

## Track, source, and channel model

### Product-level default

```text
InstrumentTrack
  -> one InstrumentInstance
  -> one ChannelStrip
```

This is a user-facing invariant, not a limitation in compiler IR. It gives the
common workflow a clear mental model while retaining future routing flexibility.

### Patch definition versus instance

```text
PatchDefinition:
  reusable node graph, sound-design defaults, descriptions,
  canonical asset references and preparation intent

InstrumentInstance:
  patch reference, performance config, instance overrides, voice/runtime state

ChannelStrip:
  channel inserts, volume, pan, mute, solo, sends, meter identity
```

Several instrument tracks may reference one patch definition. They normally
receive separate instances and channels.

If future workflows need several event tracks to drive one instrument instance,
model that explicitly:

```text
EventTrack A --+
               +--> shared InstrumentInstance --> one ChannelStrip
EventTrack B --+
```

Those event tracks are then not independent mixer channels.

### Automation tracks

`AutomationTrack` is a timeline/control owner with no instrument and no audio
channel. It may contain:

- absolute authored automation lanes;
- reusable automation/control clips;
- YAMS control programs;
- Mod Graph assignments;
- MIDI/controller mappings;
- explicit or target-relative parameter routes.

It exposes enable/bypass, not ordinary audio mute/solo. It has no VU meter and
does not appear as an audio channel unless the UI deliberately provides a
control-track section.

### Layering and groups

Prefer explicit child tracks or a layer source over changing an ordinary track
to hold a vector of instruments. Fan-out and fan-in remain visible, separately
mixable, and compiler-validatable.

## Tuning and sample mapping

- `TuningDefinition` is canonical authored data. It records scale, keyboard
  mapping, reference note/frequency, stable identity, and optional Scala/KBM
  source/asset provenance. `PreparedTuning` is the immutable renderer-rate
  lookup representation derived from it.
- Every pitch-producing node, sequencer expansion, preview path, analysis
  expectation, and offline/live renderer uses the same resolved tuning context.
  Direct MIDI-to-12-TET conversion is permitted only when the selected tuning
  explicitly is 12-TET.
- `SampleAsset` identifies source audio; `SampleZone` maps key/velocity ranges,
  root/tuning, playback/crop/loop region, gain, and selection policy;
  `SampleMap` groups zones for a sampler source.
- The first V2 sampler may support exactly one zone, but it must consume the
  zone/map model. Multisampling, round-robin, slicing, streaming, and AudioTrack
  remain optional capabilities rather than reasons to replace asset identity or
  voice plumbing later.
- Prepared sample data is keyed by source digest plus preparation profile. It is
  disposable cache/state, not canonical project audio.

## Mixer and bus graph

- A channel belongs to an audio-producing track/output, not to a patch asset.
- Instrument sound-design effects may live in the patch/instance shared stage.
- Mixing effects live in the channel, group, return, or master graph.
- Sends originate from channel/bus outputs and target `BusId`.
- Sidechains reference an explicit channel/bus tap with pre/post position.
- Meters are keyed by channel/bus/output identity.
- Group, return, and master routing are compiled graph operations.
- Solo/mute semantics are defined on channels and buses. Instrument-instance
  enable is a source/runtime control with a distinct meaning.

## YAMS architecture

### Shared language, distinct runtimes

- Parser, diagnostics, functions, constants, bytecode utilities, and program
  metadata may be shared.
- Note, control, and audio programs expose distinct typed interfaces.
- Runtime state is allocated per declared scope.
- Immutable bytecode is shared across voices/instances where compatible.

### Bindings

- Textual names exist only in source and diagnostics.
- Successful compilation/binding produces typed slots.
- Parameter reads state whether they observe base, automated, or delayed
  effective values.
- Signal reads add dependency edges.
- Scope-invalid reads/writes fail compilation.
- Interface-changing edits recompile the containing plan.

### State and hot reload

- Stateful operations have stable state identities where preservation matters.
- Compatible edits may migrate state by stable identity and layout signature.
- Incompatible edits reset and optionally crossfade.
- Randomness derives from stable IDs and project seed.
- Compile failure keeps the last valid program active.

### Cost and safety

- Programs have bounded stack, state, sources, outputs, generated events, and
  instruction work.
- The compiler estimates worst-case cost multiplied by scope and polyphony.
- Audio-rate scripts may be warned or rejected above a configured safety budget.
- NaN/Inf sanitation occurs at declared boundaries.

## Canonical state partitions

### Project document

- The project document is the only authority for persisted musical, routing,
  asset-reference, and intentionally persisted editor state.
- It contains authoring intent and stable identities, not compiled slots,
  prepared DSP state, engine command payloads, UI widgets, locks, or telemetry.
- A save snapshots one immutable document revision. It does not reconstruct the
  project from an asynchronously mirrored audio engine or query views for
  missing fields.
- Engine state is a compiled, disposable projection of a project revision.

### Runtime session

- Transport position, live preview, current focus, record arm, open device
  connections, pending compile, and active-plan coordination are runtime
  session state unless a specific product rule says otherwise.
- Session state may reference stable project identities but is not folded into
  document equality, undo, dirty state, or semantic project digests.
- Persisted playback constructs such as loop regions or markers must be modeled
  as explicit document objects, not inferred from the current transport.
- Count-in/metronome state, pending note/audio take, monitoring, input routing,
  device negotiation/reconnect, and timestamp calibration are session/host state
  until an explicit application operation commits authored results.

### User settings and editor metadata

- Audio/MIDI device preferences, default directories, author defaults, and
  global appearance preferences live outside the project.
- `EditorMetadata` contains only presentation intent worth sharing with the
  document: graph layout, groups, colors, and comparable authored organization.
- Selection, hover, scroll caches, popup state, temporary text buffers, and
  animation state remain frontend-local and transient.

### Host and service configuration

- Deployment-owned MCP/OSC endpoints, enabled services/features, authorization,
  protocol compatibility policy, and default resource budgets live in
  `HostConfig`, not the project or renderer.
- Per-user audio/MIDI device preference may select a desired host configuration,
  but the negotiated active device/stream remains runtime session state.
- Configuration formats have an explicit owner, validation/default policy, and
  diagnostics. A malformed service config may fall back only when that policy is
  visible; it cannot silently mutate project or sound semantics.

### Runtime jobs

- Render, export, analysis, and conversion jobs capture one immutable project
  and asset snapshot plus its semantic/revision identity.
- Progress, cancellation, worker ownership, temporary files/buffers, result
  receipts, and stale-result labels are runtime state.
- Jobs share the same streaming render/analysis core across GUI, CLI, and MCP.
  Frontend task handles adapt the job; they do not own a second implementation.
- Cancelling or failing a job leaves canonical state and the last good output
  intact. Any result intended to modify the project is applied later through a
  separate optimistic Application Core transaction.

### Runtime telemetry

- Meters, scopes, CPU, active voices, queue saturation, and current render
  revision are lossy observations.
- Telemetry is read separately and never merged back as canonical project
  state.
- Compile acknowledgements state which project revision is currently audible.

## Observation and external telemetry

- A persisted analyzer/monitor node contains authored intent and parameters,
  never a GUI buffer, FFT frame cache, subscriber, socket, or connection status.
- Compiler-declared taps identify stable signal points, data type/rate, and
  resource cost. The host activates only subscriptions admitted by its profile.
- Runtime subscriptions own bounded rings/atomics, generation/revision tags,
  decimation, and consumer lifetime. Slow consumers lose observations and
  receive drop/staleness metadata; they never block rendering.
- Expensive FFT, feature extraction, history accumulation, protocol encoding,
  and visualization occur on non-RT workers unless an explicit bounded node is
  part of the authored DSP graph.
- GUI meters/scopes, OSC, and the standalone visualizer consume one telemetry
  facade. External messages carry protocol and active-plan/project revision as
  appropriate; a version mismatch is diagnosed rather than decoded as a nearby
  shape.
- Adding/removing a passive subscriber cannot change audio samples, canonical
  document digest, or headless compilability.

## Stable identity and references

- Every persistent identity is a domain newtype. Runtime loop indices and
  compiler slots are distinct non-persistent types.
- Identity never derives from module type, display label, list position, graph
  schedule, or current runtime address.
- Names and address strings are authoring syntax. The binder resolves them to
  stable identities and then runtime slots; unresolved references remain named
  diagnostics rather than falling through to another object.
- Duplication and import use one remapping mechanism that covers nested graphs,
  automation targets, sends, sidechains, mappings, editor metadata, and assets.
- Deletion has a declared reference policy per relationship: cascade, reject,
  clear, or retain as unresolved. Container behavior is never an accidental
  consequence of vector removal.

## Application operations, revisions, and history

- GUI, MCP, CLI, importers, tests, and future hosts call the same application
  operations and validation.
- A transaction reads one document revision, validates and applies domain
  intent, and publishes at most one successor revision.
- Operation effect (`complete`, `partial`, `none`) is independent of diagnostic
  severity and transport-level error. Partial work must identify exactly what
  landed and what did not.
- Compound operations are atomic unless the operation explicitly offers a
  diagnosed partial-import mode.
- Optimistic concurrency compares an expected base revision before mutation.
- History records committed application changes, not GUI gestures or engine
  commands. Gesture coalescing determines transaction boundaries but not domain
  semantics.
- Undo, dirty state, save, autosave, recovery, rollback, and compile scheduling
  consume the same revision stream.
- A compile coordinator tracks editing, queued, compiling, rejected, prepared,
  and active/audible revisions. A failed compile never rewrites the document or
  stops the last valid plan.
- During coexistence, V1 and V2 are runtime adapters downstream of the same
  committed operation; their command models do not leak upward.

## Project format and assets

- The format envelope has a typed discriminator and exact version checked
  before model decoding.
- Decode, version conversion, validation, asset resolution, and compilation are
  separate stages with structured diagnostics.
- Unknown, removed, invalid, or unsupported content is rejected or explicitly
  reported; it is never silently ignored by permissive deserialization.
- The initial document encoding remains inspectable JSON unless a concrete need
  justifies another encoding.
- Packages use a manifest and content digests for embedded assets. External
  references carry expected identity/digest and a missing/mismatch policy.
- Samples, wavetables, impulse responses, and future audio clips share the asset
  reference and package mechanism.
- Source/native assets remain distinct from prepared runtime caches. Preparation
  is keyed by source digest and `HostProfile`/render settings and may produce
  rate/layout-specific PCM, mipmaps, decoded tables, or other disposable data.
- Samples use canonical `SampleMap`/`SampleZone` authoring. Tunings use canonical
  `TuningDefinition`; Scala/KBM source or resolved authoring data follows the
  same package/reference policy.
- Provenance and privacy policy covers external paths/URIs, recording device
  metadata, and portable-package redaction.
- Asset collection accounts for document, history, recovery, active-plan,
  compile, and runtime-job references. Collection is explicit and off-thread.
- Standalone patches, presets, and templates reuse canonical graph, parameter,
  identity, and asset types rather than defining parallel schemas.
- A semantic document digest supports round-trip and migration verification
  independent of JSON formatting and field order.

## Real-time communication

### Structural plans

- Full immutable plan publication.
- Latest complete revision wins.
- Block/quantum-boundary activation.
- Explicit active-revision acknowledgement.
- Off-thread retirement and destruction.

### Live events

- Timestamped and bounded.
- Priority policy distinguishes note-off/panic from lower-value controller data.
- Overflow is counted and surfaced; it is never silently treated as success.
- Event payloads are fixed-size or draw from preallocated storage.

### Runtime session control

- Transport, seek, preview, loop, count-in, metronome, record arm/disarm, and
  panic use a distinct bounded ordered lane.
- Same-sample ordering against performance/timeline events is specified and
  tested. Stop/panic/finalization have protected priority.
- Session commands do not dirty the project. Committing resulting authored
  notes/audio uses Application Core.

### Parameter control

- Versioned target slots.
- Coalescing is permitted for untimed UI knob updates.
- Scheduled automation and performance events are not coalesced across their
  required time points.
- Updates targeting a stale plan revision are remapped by stable identity on the
  control side or rejected; the audio thread does not search.

### Telemetry

- Meters/scopes/CPU/voice status are lossy observations.
- Atomics or dedicated bounded rings are appropriate.
- Telemetry saturation cannot block rendering or structural control.

### Recording results

- Note/audio result channels are bounded and revision/timestamp tagged.
- Overflow/drop/latency/quantization/loop/overdub facts accompany a partial take
  and cannot be erased when it is committed or rejected.
- Heap-owning take/audio data is finalized, transferred, and destroyed off the
  audio thread through preallocated or bounded handoff structures.

## Output and numeric policy

- Internal sample format remains `f32` unless measured evidence supports a
  targeted higher-precision stage.
- Internal buses permit headroom above full scale.
- Nonlinear mixing is explicit DSP, not hidden engine protection.
- Hardware output and integer file formats define their clip/limit/dither policy.
- Float files may preserve over-full-scale values.
- Denormal handling remains explicit for every live and offline render entry.
- NaN/Inf handling is defined at module and routing boundaries so one invalid
  source cannot poison persistent downstream state.

## Latency, tails, and rates

Every node/operation declares where applicable:

- processing latency;
- lookahead;
- tail length or unbounded-tail classification;
- supported channel layouts;
- supported rate factors;
- in-place capability;
- reset cost and semantics.

The compiler:

- sums path latency;
- inserts compensation delays according to policy;
- reports total live/output latency;
- derives offline tail requirements or warns when a user cap truncates them;
- groups compatible oversampled operations into rate islands;
- accounts for conversion latency in sidechain and parallel paths.

---

# Part III: Verification and Quality Gates

## Correctness tests

- Graph validation for missing nodes/ports, incompatible domains/layouts,
  duplicate inputs, fan-in, cycles, delayed feedback, and missing output paths.
- Stable target identity across reorder, insertion, deletion, duplication, and
  patch sharing.
- Sample-exact note, gate, automation, tempo, loop, seek, and retrigger events.
- Same-sample ordering for performance events, session commands, compiled
  timeline events, count-in/metronome, loop transition, recording, stop, and
  panic.
- Parameter composition across base, automation, MIDI/controller, Mod Grid,
  Mod Matrix, and YAMS.
- Scope validation and explicit reductions.
- Channel, sidechain, send, return, group, and master routing.
- Latency compensation and tail calculation.
- State reset, plan swap, compatible migration, incompatible reset, and
  crossfade.
- Host negotiation, disconnect/reconnect, sample-rate/layout reprepare, MIDI/
  input timestamp mapping, clock drift, latency compensation, monitoring, and
  bounded input backlog using simulated hosts.
- Note and audio recording for count-in, loop boundary, replace/overdub,
  sustain/held notes, quantization, overflow, stale target, commit, undo, and
  redo.
- Tuning agreement across preview, live, sequenced, offline, and analysis paths
  for 12-TET, a built-in alternate tuning, and Scala/KBM mapping.
- Sample-zone key/velocity/root/loop selection using the same one-zone and
  multi-zone-ready model.
- Passive observation equivalence: enabling/disabling GUI, analyzer, OSC, and
  visualizer subscriptions changes no rendered samples.
- Project conversion for every bundled example.
- Stable domain identity across project round-trip, duplication, import,
  conversion, list reorder, and unrelated object deletion.
- Reference policy for deletion and missing targets: cascade, reject, clear, or
  diagnosed unresolved reference as declared by the relationship.
- State classification tests proving session, settings, transient GUI state,
  and telemetry do not change the semantic project digest.
- Save equivalence across GUI, MCP, CLI, autosave, recovery, and rollback for
  the same project revision.
- Revision-pinned render/analysis jobs remain on their captured document while
  editing continues and label results with the correct project/render revision.

## Project format and asset tests

- Envelope discrimination and exact format-version acceptance/rejection.
- Decode, conversion, validation, asset resolution, and compile diagnostics
  remain attributable to their own stage.
- Unknown/removed-field detection; no schema-valid input content disappears
  without a diagnostic or explicit conversion rule.
- Load-save-load semantic digest equality independent of JSON formatting.
- Standalone patch/preset/template and full-project graph equivalence.
- Embedded asset manifest, size, digest, and stable-ID verification.
- External asset missing, relocated, changed-content, and digest-mismatch paths.
- Immutable asset sharing across document snapshots, undo, offline rendering,
  and live compilation.
- Source/native asset identity and semantic digest stability across preparation
  at multiple sample rates/layouts.
- `SampleMap`/`SampleZone` and `TuningDefinition` package, external-reference,
  conversion, and round-trip behavior.
- Provenance/path redaction for portable packages.
- Orphan collection with references retained by undo/redo, recovery, active
  plan, pending compile, and runtime jobs.
- Corrupt/truncated package and atomic-save failure paths retain the last good
  document and assets.

## Application operation conformance tests

Maintain one shared corpus of representative operations and expected results.
Run it through direct Application Core calls and every migrated frontend.

The corpus covers:

- create, update, reorder, duplicate, import, delete, and compound operations;
- complete, partial, and no-effect results independently combined with warning
  and error diagnostics;
- created/affected IDs and stable remapping;
- expected-base revision success and stale-revision conflict;
- history entry, undo, redo, coalesced gesture, and saved-position dirty state;
- compile-impact classification and failed-compile behavior;
- atomic batch success, rejected batch, explicit partial import, and rollback;
- equivalent semantic project digest across GUI, MCP, CLI/importer, and direct
  adapters.
- note/audio take commit including latency/overflow diagnostics, stale target,
  replace/overdub, undo, and redo;
- asynchronous job start/cancel/result handling without implicit project
  mutation.

## Determinism tests

- Repeated offline render digest equality.
- Live/offline render equality for identical event streams and configuration.
- Host block-partition invariance.
- Stable event placement under equivalent simulated MIDI/audio callback
  partitions, timestamp epochs, and bounded clock drift.
- Stable random behavior across plan recompilation caused by unrelated edits.
- Stable behavior across graph serialization order.
- Stable topo scheduling when independent nodes can be ordered several ways.

## Real-time tests

The allocation guard must cover more than steady-state DSP:

- common note and controller events;
- automation events;
- plan polling with no pending plan;
- plan activation;
- program-only YAMS swap;
- panic/all-notes-off;
- sample and asset reference changes where supported;
- session transport/preview/count-in/metronome/record commands;
- timestamped live note/controller/per-note expression and sustain paths;
- audio-input underflow/backlog/drop/drift-correction paths;
- recording-result success, partial/overflow, finalization, and cancellation;
- retired-plan return;
- observation subscribe/poll/unsubscribe and slow-consumer saturation;
- telemetry saturation;
- queue overflow paths.

Also verify:

- no locks;
- no filesystem or system-clock access;
- no logging;
- no unbounded iteration;
- no final drop of heap-owning control objects;
- no buffer growth at maximum declared configuration;
- bounded failure behavior when return/telemetry/event channels are full.
- no project mutation, asset destruction, device/protocol work, or job cleanup
  on the audio thread.

## Performance tests

- Basic native-node graph versus V1 equivalent.
- Representative voice counts and stealing pressure.
- Native versus legacy-adapter module cost.
- Modulation-heavy patch.
- Control and audio YAMS at maximum supported scopes.
- Mixer with many channels, sends, and returns.
- Oversampling islands.
- Plan compile time and prepared memory size.
- Plan swap/crossfade peak CPU.
- Audio-input resampling/drift correction and monitoring at supported device
  rate combinations.
- Observation disabled, meters only, scopes, FFT/features, and OSC subscriber
  loads measured separately.
- Streaming render/stems versus optional in-memory collection, including cancel
  latency and peak memory.
- Resource admission/report generation at near-limit and rejected profiles.

Before default cutover:

- common native V2 graphs should be no slower than V1 under equivalent
  semantics;
- any remaining regression above an agreed small margin must be explained by a
  deliberate semantic improvement or fixed;
- peak callback cost, not only average CPU, must fit the real-time budget;
- plan memory must scale predictably with node count, channel count, and maximum
  voices.

## Audio comparison policy

Classify comparisons:

1. **Exact parity expected** — extracted DSP with the same timing and inputs.
2. **Feature parity, numerical drift permitted** — changed operation order or
   explicit interpolation with equivalent audible behavior.
3. **Intentional semantic correction** — sample timing, fixed control rate,
   linear headroom, explicit sidechain latency, parameter composition.
4. **Known unsupported migration scope** — reported as such, never rendered
   silently with missing behavior.

Each intentional correction requires a focused test that asserts the new rule.

## Compiler diagnostics quality

Every diagnostic should carry as available:

- project revision;
- graph/track/patch identity;
- node and port/parameter identity;
- source location for YAMS;
- error code and concise explanation;
- relevant scope/rate/layout/latency facts;
- actionable suggestion;
- whether the previous active plan remains in use.

Diagnostics are application data. MCP and GUI format the same structured
problem; neither re-derives compiler rules.

## Capability and migration coverage

The Phase 0B ledger is a maintained executable/checkable artifact, not a one-time
document. Before cutover it must show:

- every current MCP tool, CLI command, GUI workflow, `EngineCommand`/
  `EngineEvent`, public Rust facade, file/schema/import/export path, OSC message,
  and external consumer mapped to its V2 owner or explicit removal;
- every module type, programmatic built-in patch, group template, and bundled
  example converted and compiled, or intentionally excluded with a reason;
- every exported but unreachable/test-only subsystem explicitly removed,
  retained as internal test support, or promoted by a product decision;
- no frontend capability marked migrated while it still bypasses Application
  Core, uses engine snapshots as canonical data, or owns a separate render loop.

The check fails on a newly added unclassified capability so the ledger cannot
silently become stale during the multi-phase migration.

## Build, feature, platform, and external-consumer matrix

Before each live/default cutover milestone, run the repository quality gates and
the declared compatibility matrix:

- workspace format, build, clippy, tests, and rustdoc;
- no-default, default, all-feature, and every separately supported feature
  combination rather than assuming those three cover conditional dead code;
- MSRV and current stable Rust;
- Linux, macOS, and Windows release builds and package assembly;
- standalone visualizer build plus OSC protocol/telemetry conformance;
- headless CLI, MCP service, and public runtime facade smoke/conformance tests;
- generated schemas, bundled examples, programmatic presets/templates, package
  manifests, and release assets.

The final V2 crate dependency graph is checked for forbidden upward dependencies
on GUI, MCP, OSC, CPAL, visualizer, or frontend wire DTOs.

---

# Part IV: Risks and Controls

## Risk: two engines diverge for too long

Controls:

- offline-first bounded phases;
- vertical slices;
- one comparison corpus;
- explicit default-engine milestone;
- stop adding equivalent engine plumbing to both after V2 becomes development
  default;
- remove V1 promptly after the cutover gates pass.

## Risk: the legacy adapter becomes permanent

Controls:

- measure adapter cost separately;
- forbid new engine-specific behavior in the adapter;
- require native nodes for the minimal path, YAMS, core modulation, and critical
  effects before cutover;
- track every adapted module in a migration table.

## Risk: over-generalized node abstraction

Controls:

- keep Note, Control, and Audio execution domains distinct;
- generalize IDs, specs, compilation, diagnostics, and state preparation, not
  every processing loop;
- prove each abstraction with at least two materially different consumers;
- prefer explicit conversion/reduction nodes over hidden universal coercion.

## Risk: compiler complexity merely moves bugs

Controls:

- pure compiler inputs and outputs;
- deterministic IR snapshots;
- property tests for arbitrary DAGs and buffer liveness;
- compiler diagnostics tests;
- no runtime fallback that reinterprets an invalid plan;
- debug plan dumps that map operations back to authoring IDs.

## Risk: live plan state migration dominates the project

Controls:

- stopped-only activation first;
- reset plus short crossfade second;
- migrate only unchanged nodes with matching stable identity and layout;
- allow some module state to declare non-migratable;
- do not make perfect tail/voice preservation a prerequisite for proving the
  compiler and renderer.

## Risk: the combined migration expands without a stable boundary

Project Core, Application Core, and Sound Core must align, but that does not
authorize redesigning every Pertylizer feature at once.

Controls:

- use the Phase 0B ownership ledger to define the boundary;
- implement Project/Application Core only after the offline and live Sound Core
  gates prove the downstream contract;
- migrate one operation family and one frontend/view at a time;
- reuse working DSP, atomic I/O, recovery lifecycle, and UI workflows behind the
  new boundaries;
- require a named invariant or removal of duplicated ownership for every broad
  refactor.

## Risk: the canonical document becomes another runtime mirror

Controls:

- forbid runtime-only types, locks, atomics, command senders, and compiled slots
  in `ProjectDocument`;
- save only the document revision, never a merge with runtime snapshots;
- maintain explicit `RuntimeSession`, `UserSettings`, `EditorMetadata`,
  `HostConfig`, `RuntimeJobs`, and `RuntimeTelemetry` classifications;
- test that telemetry and transient session changes leave the semantic project
  digest unchanged.

## Risk: a permissive new format silently loses content

Controls:

- check discriminator/version before typed decoding;
- detect unknown or removed fields according to the selected version policy;
- separate decode, conversion, validation, asset resolution, and compile
  diagnostics;
- gate conversion on an empty unexplained-loss set;
- compare semantic document and asset digests after round-trip.

## Risk: operation abstraction becomes an untyped universal command

Controls:

- operations express domain intent with domain newtypes;
- frontend wire DTOs adapt typed operations rather than replacing them with
  stringly typed paths and generic JSON values;
- compile impact is metadata on a committed domain change, not the operation's
  semantic identity;
- prove shared semantics through conformance tests, not by exposing one unsafe
  catch-all mutation endpoint.

## Risk: project-model and GUI rewrite begins too early

Controls:

- keep `LegacyProjectLowerer` until V2 live playback is proven;
- defer broad authoring changes to Phase 10;
- build canonical application operations in vertical workflow slices;
- do not keep two writable canonical models.

## Risk: shared-patch semantics surprise users

Controls:

- distinguish patch definition, instrument instance, and channel in naming and
  UI;
- make normal tracks use independent instances;
- offer explicit linked patch editing and "make unique";
- require an explicit converter choice for current multi-track shared runtime
  instruments.

## Risk: automation conflicts become more visible

This is desirable but may reject projects that previously depended on iteration
order or last-writer behavior.

Controls:

- diagnose every conflicting target and source lane;
- offer explicit priority/combine modes only after a concrete use case;
- provide a project conversion report;
- never silently choose a winner.

## Risk: YAMS audio-rate cost explodes with polyphony

Controls:

- static instruction and state bounds;
- plan cost estimation;
- warnings or configured refusal above budget;
- shared immutable bytecode;
- native vector/block operations where the language semantics allow them;
- representative worst-case benchmarks.

## Risk: host I/O and recording are treated as a final adapter detail

The renderer can be correct offline yet fail live through timestamp loss,
independent input/output clocks, device renegotiation, monitoring latency, or a
recording path that edits project state from the wrong thread.

Controls:

- decide host clocks and latency in Phase 0A, and session lanes and take
  semantics in Phase 0B;
- simulate negotiation, drift, hotplug, disconnect, overflow, and timestamp
  mapping before relying on physical-device testing;
- keep device/protocol code outside Sound Core;
- commit note/audio takes through ordinary Application Core transactions;
- require measured latency/drop diagnostics rather than hidden compensation.

## Risk: bounded execution becomes scattered silent caps

Controls:

- one explicit `HostProfile` and compile-time `ResourceReport`;
- inventory every V1 cap and truncation in Phase 0A;
- reject authored topology/routing/event expansion before activation when it
  cannot fit;
- reserve lossy behavior for declared telemetry/live queue classes and always
  publish counters;
- test near-limit and over-limit projects as first-class compiler cases.

## Risk: observation and external protocols contaminate the engine

Controls:

- persist analyzer intent, never runtime buffers or subscribers;
- compiler-declared taps plus host-owned bounded subscriptions;
- non-RT FFT/protocol workers by default;
- audio-equivalence tests with observation disabled/enabled/saturated;
- versioned OSC/visualizer conformance through the telemetry facade, not
  `EngineState` access.

## Risk: offline work fragments into frontend-specific render loops

Controls:

- one streaming renderer and revision-pinned job contract;
- GUI, CLI, MCP, analysis, and export adapt progress/cancellation/receipts;
- peak-memory and cancellation-latency tests;
- no job result mutates the project without a separate optimistic transaction;
- remove duplicate V1 offline orchestration only after result parity.

## Risk: future sampling/tuning forces another voice and format rewrite

Controls:

- canonical source asset versus disposable prepared data;
- one-zone-first `SampleMap`/`SampleZone` model;
- canonical `TuningDefinition` versus `PreparedTuning`;
- shared pitch/tuning context for preview, sequence, live, render, and analysis;
- explicit asset provenance, retention, and garbage-collection policy.

## Risk: the capability ledger becomes stale or migrates dead architecture

Controls:

- generate/check inventories where practical and fail on unclassified additions;
- record reachability separately from public/exported/tested existence;
- require a product decision before promoting engine hubs, transactions, input
  multiplexers, or similar unused abstractions;
- include external protocols, public facades, built-ins, and feature/platform
  combinations in the cutover gate.

---

# Part V: Scope and Non-goals

## In scope

- compiled audio/control/event plans;
- sample-accurate scheduler;
- fixed internal quantum;
- native module and parameter API;
- polyphony;
- YAMS domains;
- Mod Matrix and Mod Grid lowering;
- track/source/channel separation;
- automation tracks;
- mixer/bus graph;
- live immutable plan swapping;
- host audio/MIDI adapter contract, independent clock domains, latency,
  monitoring, reconnect, and simulated-host verification;
- runtime-session commands, timestamped performance events, note/audio
  recording results, and transactional take commit;
- declared host/resource profiles and compile-time admission reports;
- prepared tuning and canonical tuning ownership;
- sample asset/map/zone boundary sufficient for one-zone playback and future
  multisampling without a model replacement;
- passive observation taps, telemetry subscriptions, OSC/visualizer migration,
  and protocol versioning;
- revision-pinned streaming render/analysis/export jobs with progress,
  cancellation, receipts, and stem/channel selection;
- Project Core V2 canonical document and stable identity model;
- Application Core V2 operations, transactions, revisions, history, and
  optimistic concurrency;
- Project Format V2 envelopes, validation, packages, asset references, and
  one-shot conversion;
- separation of project, runtime session, user settings, editor metadata, and
  host/service configuration, runtime jobs, and telemetry;
- shared frontend operation conformance across GUI, MCP, CLI, and importers;
- GUI migration required by those concepts;
- V1 retirement.

## Not initial scope

- third-party binary plugin ABI;
- network-distributed DSP;
- GPU audio processing;
- arbitrary dynamic allocation from scripts;
- unbounded feedback graphs without explicit delay;
- automatic preservation of every DSP tail across every structural edit;
- bit-identical output to V1 where V2 deliberately fixes timing or gain
  semantics;
- backward-compatible persistence of every historical project version;
- simultaneous V1 and V2 live output as a user feature;
- replacing inspectable JSON with a binary document format without a measured
  requirement;
- persisting every transient session or GUI interaction detail merely because
  Project Core can represent metadata;
- rewriting validated DSP, atomic save, recovery, or existing workflows unless
  integration evidence requires a semantic change;
- a third writable project representation maintained for a frontend;
- immediate implementation of AudioTrack, GroupTrack, multichannel hardware
  output, or every future signal domain merely because the model can represent
  them;
- full multisample-zone playback features, round-robin, slicing, timestretch,
  disk streaming, or granular sample sources beyond the canonical boundary
  needed to avoid a later data-model rewrite;
- an MPE/MIDI 2.0 hardware adapter, external MIDI clock/transport sync, or full
  DAW/plugin-host synchronization in the first cut, although event identity and
  timestamping must leave room for them;
- generalized remote multi-user collaboration, priority arbitration, or an
  engine-level permission model; a local-only service policy is acceptable
  unless Phase 0B explicitly promotes remote control to a product requirement.

---

# Part VI: Coordination With Existing Plans

## MCP agent API redesign

The application-operation and canonical-project work in
[`plans/mcp-agent-api-redesign.md`](../mcp-agent-api-redesign.md) aligns with
Sound Core V2. It should not recreate V1 engine commands as the new application
abstraction. Operations
should mutate canonical project state, produce a revision, and let the compiler
publish the corresponding plan.

During coexistence, the application layer may use separate V1 and V2 runtime
adapters behind the same prepared project mutation. V2 must not depend on MCP
types or wire schemas.

## Game runtime library

[`plans/game-runtime-library.md`](../game-runtime-library.md) currently assumes
`SynthEngine` is the sole engine implementation. Preserve its stronger
principle rather than its current
type name: Studio, games, CLI, and offline tools must use the same selected
render core. Do not publish a V1-specific facade contract that makes V2 cutover
needlessly breaking while this work is active.

V2's host-driven renderer, immutable prepared plan, and control/event separation
are intended to simplify that runtime substantially.

## Headless render and analysis

The existing headless renderer and analyzers, planned in
[`plans/headless-render-cli.md`](../headless-render-cli.md), are the first
integration and A/B harness. They should select an engine internally during
development while keeping one user-facing render contract. Their deterministic
receipts and digests are useful V2 evidence.

Replace their differing orchestration with the common revision-pinned streaming
job contract as V2 becomes usable. Pure symbolic analyzers consume canonical
domain snapshots and return domain results; audio analyzers consume the shared
renderer output. Neither analysis core may depend on MCP DTOs or `EngineState`.

## Sampling, tuning, and recording backlog

The open sampling work in [`plans/TODO.md`](../TODO.md) correctly warns that
multi-sample zones should be modeled before voice and GUI assumptions harden.
This plan owns
the architectural prerequisite: `SampleAsset`/`SampleMap`/`SampleZone`, source
versus prepared data, provenance, retention, and transactional recording import.
It does not pull full streaming, slicing, timestretch, granular sample sourcing,
or AudioTrack implementation into the initial cutover.

Alternative tuning follows the same rule: V2 must establish canonical
`TuningDefinition` and one shared prepared pitch path; richer tuning UI and every
possible import/export workflow may land later.

## OSC and standalone visualizer

The standalone visualizer is a shipped external consumer even though it is not
a normal workspace member. V2 replaces its direct dependency on engine-shaped
telemetry with the observation facade and a versioned protocol contract. Build,
package, protocol mismatch, drop/staleness, and audio-noninterference tests are
part of the cutover matrix.

## Project safety

Atomic save, recovery, undo, dirty-state correctness, and input routing remain
non-negotiable. Project Core V2 changes their observation source, not their
guarantees. Keep current counters/fingerprints and GUI undo paths active until
the corresponding operation family is proven to use canonical revisions and
history exclusively; remove each legacy observer only with a conformance and
recovery test demonstrating its replacement. V2 work must not regress current
safeguards while both architectures coexist.

---

# Part VII: Open Decisions

Resolve these at the named phase, using measurements rather than preference.
Every topic has a permanent identifier in the [decision register](ADR.md), which
is authoritative for its status, class, and target phase; the text below states
what is open and why. A topic is settled only when it is accepted in that
register — for most topics that means an accepted record under
[decisions/](decisions/README.md), and for a topic classed `Reversible` it means
an accepted register row with a value and a named revisit point. Neither class
is settled by a preference stated here.

1. **Internal quantum (ADR-0001, ADR-0037)** — likely 32 or 64 frames; decide in
   Phase 0A before Phase 1, then verify the accepted choice while implementing
   Phase 1. This topic is carried by **two** records: ADR-0001 fixes the
   splitting semantics — one compile-time value everywhere, whole quanta only,
   input and output carries, control evaluated once per quantum, and the
   end-of-stream drain — while ADR-0037 fixes the frame count, which is the only
   part that waits on a measurement. Both must be `Accepted` before Phase 1, and
   both now are: `Q` = 64, provisionally, because the V1 proxy measurement
   (EVD-0002) came back inconclusive at the resolution ADR-0037's own rule table
   demands. Confirming or superseding that value against real V2 nodes is a
   [Phase 2 exit-gate item](#phase-2-minimal-compiled-voice-graph).
2. **Internal channel layout (ADR-0002)** — planar is preferred; verify
   against module and conversion cost in Phase 2.
3. **Event segmentation API (ADR-0003)** — renderer-level segment split versus
   event spans consumed by selected nodes; decide in Phase 3.
4. **Native node representation (ADR-0004)** — trait objects, enum dispatch,
   or a hybrid; measure compile time, runtime cost, and module ergonomics in
   Phase 2/5.
5. **Buffer liveness sophistication (ADR-0005)** — begin with correct
   conservative reuse; optimize only after profiling.
6. **Parameter ramp representation (ADR-0006)** — scalar/start-end/piecewise
   segment; decide with sample-accurate automation in Phase 3/5.
7. **Modulation laws (ADR-0007)** — ratify the initial set from real existing
   parameters in Phase 5.
8. **YAMS state identity (ADR-0008)** — named state, semantic compiler
   identity, or reset policy; decide in Phase 7.
9. **Plan swap crossfade length and latency (ADR-0009)** — decide from audible
   tests in Phase 9.
10. **Compatible state migration surface (ADR-0010)** — private engine
    mechanism versus a node-declared migration hook; decide in Phase 9.
11. **Shared current instrument conversion (ADR-0011)** — independent
    instances versus explicit shared event routing; decide per affected
    example in Phase 10.
12. **Automation conflict policy (ADR-0012)** — error versus explicit
    priorities; start strict in Phase 10.
13. **Project/session boundary cases (ADR-0013)** — keyboard octave, preview
    glide, transport loop, active selection, render settings, and record-arm
    state; classify from product semantics in Phase 0B/10A rather than current
    location.
14. **Persistent ID generation and encoding (ADR-0014)** — random, monotonic,
    namespaced, or hybrid; decide in Phase 0B/10A from duplication,
    deterministic fixtures, merge/import, and wire-format needs.
15. **History representation (ADR-0015)** — invertible operations, domain
    `ChangeSet`, structurally shared document snapshots, or a measured hybrid;
    decide in Phase 10C before migrating views.
16. **Unknown-field policy (ADR-0016)** — strict rejection versus retained
    diagnosed extensions for a known format version; decide in Phase 0B/10D.
    Silent ignore is not an option.
17. **Asset identity and external reference policy (ADR-0017)** — content
    digest, stable asset ID, source versus prepared representation,
    provenance/privacy, retention/garbage collection, relocation search, and
    explicit relink behavior; decide in Phase 0B/10D before AudioTrack or
    expanded sampling work.
18. **Editor metadata scope (ADR-0018)** — which layout/organization fields
    are intentional shared project content versus user-local presentation;
    establish the persistence boundary from the ownership ledger in Phase
    0B, then enforce and refine it with the canonical model in Phase 10A.
19. **Remote history semantics (ADR-0019)** — whether MCP/import transactions
    enter the ordinary undo stack, a labeled shared history, or an explicit
    non-undoable path requiring caller consent; decide in Phase 10B/10C.
20. **Final crate boundaries and names (ADR-0020)** — begin with dependency
    rules; decide physical crate splits only after working
    Project/Application/Sound vertical slices demonstrate them and V1
    retirement is within reach.
21. **Host profile and admission policy (ADR-0021)** — which limits are hard
    platform capabilities, configurable safety budgets, or warnings; decide in
    Phase 0A before Phase 1 from the V1 cap inventory and measured resource
    reports.
22. **Hardware time mapping (ADR-0022)** — timestamp epoch, calibration,
    late-event policy, independent audio clock correction, and
    latency-compensation ownership; investigate in Phase 0A and accept before
    Phase 3, then verify and refine through Phase 9 with simulated-host evidence.
23. **Session event ordering (ADR-0023)** — ordering/priority for play, stop,
    seek, loop, count-in, metronome, record, note/controller, automation, and
    panic at the same sample; decide in Phase 3 before recording/live
    integration.
24. **Recording take semantics (ADR-0024)** — replace/overdub, quantization,
    loop flushing, sustain/held-note finalization, partial overflow, stale
    target, and audio capture commit; decide in Phase 0B before Phase 9 and
    enforce the commit side in Phase 10B.
25. **Tuning representation (ADR-0025)** — resolved authored data versus
    retained Scala/KBM source/assets, project default versus instance
    override, reference pitch, and per-note bend composition; decide in Phase
    0B before Phase 6 and enforce the project representation in Phase 10A.
26. **Sample map minimum (ADR-0026)** — exact `SampleZone` fields and one-zone
    playback subset needed at V2 cutover without committing to full
    multisampling; decide in Phase 6/10A/10D.
27. **Observation ownership (ADR-0027)** — which analyzers are persisted graph
    nodes versus passive compiler taps, subscription cost/admission,
    worker-side FFT/features, and OSC payload/version policy; decide in Phase
    0B before Phase 5 and verify the live subscription contract in Phase 9.
28. **Long-running job contract (ADR-0028)** — streaming sink, progress unit,
    cancellation granularity, stem/channel selection, temporary output
    atomicity, retention, and stale-result labeling; investigate in Phase 0A,
    accept before Phase 4, and implement the shared service in Phase 10B.
29. **Host/service configuration and authorization (ADR-0029)** — boundary
    between user preference, deployment config, active runtime state,
    local-only policy, and any promoted remote authorization requirement;
    decide in Phase 0B/10E.
30. **Public facade and compatibility surface (ADR-0030)** — retained
    Project/Application/Sound/host entry points, external consumers, and
    intentionally broken V1 re-exports; decide before Phase 10E and enforce at
    Phase 12.
31. **Supported build matrix (ADR-0031)** — exact feature combinations,
    platforms, MSRV, visualizer/protocol consumer, and packaging gates
    required at each cutover milestone; record in Phase 0B and ratify before
    Phase 12.
32. **Sample-time and event-timestamp representation (ADR-0032)** — width,
    origin, and overflow behavior of absolute `SampleTime`, quantum-local
    offsets, and the timestamp carried by live events; decide in Phase 0A
    together with the quantum and verify its event use in Phase 3.
33. **Graph feedback rule (ADR-0033)** — required delay boundary, whether
    strongly connected components may be scheduled, and how effective feedback
    latency is reported; decide in Phase 2/3.
34. **Track, source, and channel ownership (ADR-0034)** — the enforced
    relationship between `InstrumentTrack`, `InstrumentInstance`,
    `PatchDefinition`, and `ChannelStrip`, and how shared patches and layered
    sources are expressed; decide in Phase 0B/10A.
35. **Transaction and concurrency semantics (ADR-0035)** — atomicity of
    compound operations, permitted partial effects, expected-base-revision
    conflict handling, and gesture coalescing boundaries; decide in Phase
    0B/10B.
36. **Audio device and input lifecycle (ADR-0036)** — requested versus
    negotiated stream configuration, hotplug/reconnect, sample-rate and layout
    changes, and the prepare/activate/resume sequence that owns them; decide in
    Phase 0B before Phase 9.

---

# Definition of Done

This list restates the phase exit gates in Part I and the gates in Part III; it
is a final checklist, not a source of new requirements. An item here without a
corresponding phase gate is a defect in one of the two.

Pertylizer Core V2 is complete when all of the following are true:

- the audio thread only executes prepared bounded operations and consumes
  timestamped events/control updates;
- structural edits compile off-thread into complete immutable revisions;
- the active audible project revision is observable and acknowledged;
- rendering is independent of host callback partitioning;
- live and offline paths use the same renderer and timing semantics;
- host audio/MIDI adapters own device lifecycle and external clocks; negotiated
  rate/layout changes, hotplug/reconnect, timestamp mapping, monitoring, drift,
  and input/output/round-trip latency have simulated-host coverage;
- project operations, session commands, performance events, and recording
  results are distinct typed paths with documented same-sample ordering;
- note/audio recording, count-in, metronome, loop, replace/overdub, sustain,
  overflow, latency compensation, commit, undo, and stale-target behavior are
  explicit and tested;
- every active plan is admitted against an explicit `HostProfile`, exposes a
  `ResourceReport`, and never silently truncates authored topology, routing,
  event expansion, sample mapping, or polyphony to fit runtime constants;
- native modules do not duplicate generic routing, automation, modulation,
  smoothing, descriptor, and registry plumbing;
- YAMS Note, Control, and Audio programs are bound, scoped, bounded, and
  scheduled through the compiler;
- canonical tuning definitions and prepared tuning data are distinct, and all
  pitch-producing/live/offline/analysis paths agree on the selected tuning;
- sampler authoring separates source assets from sample maps/zones and prepared
  runtime data, with a one-zone implementation that does not block future
  multisampling;
- PatchDefinition, InstrumentInstance, InstrumentTrack, ChannelStrip, and Bus
  have distinct ownership and identities;
- automation-only tracks exist without dummy instruments or audio channels;
- patterns and reusable control clips use stable relative targets;
- mixer sends, meters, and sidechains address channels/buses rather than
  instruments;
- internal mixing has explicit headroom and output policy;
- latency, feedback delay, oversampling conversion, and tails are explicit;
- one `ProjectDocument` is the sole persisted authority and can be serialized
  without consulting GUI or engine state;
- project, runtime session, user settings, editor metadata, and telemetry have
  explicit non-overlapping ownership, alongside separate host/service config and
  revision-pinned runtime jobs;
- all persistent identities are domain newtypes independent of type prefix,
  display order, name, and runtime slot;
- GUI, MCP, CLI, importers, save, undo, dirty state, recovery, and rollback use
  the same Application Core operations, validation, and revision stream;
- frontend operation conformance tests produce equivalent semantic project
  digests, diagnostics, affected IDs, and compile impacts;
- undo/redo and dirty state derive from canonical history/revisions without
  subsystem fingerprints or untracked-mutation escape hatches;
- Project Format V2 checks its version before decoding, reports unsupported or
  removed content, and never silently ignores requested project data;
- project packages verify embedded assets and diagnose missing or changed
  external assets before rendering;
- source asset identity, provenance/privacy, rate/layout-specific preparation,
  retention, and off-thread orphan collection are explicit and tested across
  history, recovery, active plans, and jobs;
- standalone patch/preset/template formats reuse the canonical graph,
  parameter, identity, and asset models;
- manual save, MCP save, autosave, recovery, and rollback snapshot the same
  canonical revision and preserve editor metadata;
- render, export, and analysis use one streaming job contract with project/
  render revision, progress, cancellation, bounded memory, optional stems,
  diagnostics, and receipts across GUI, CLI, and MCP;
- persisted analyzers, passive taps, runtime subscriptions, GUI meters/scopes,
  OSC, and the standalone visualizer have explicit separate ownership;
  observation saturation or subscription changes cannot alter audio or project
  digests;
- analysis/composition domain code consumes canonical snapshots and has no
  dependency on engine state, GUI state, or MCP wire DTOs;
- all bundled examples have been converted and verified;
- every module type, built-in patch, group template, MCP tool, GUI/CLI workflow,
  public facade entry, file/import/export surface, OSC message, and external
  consumer is classified and migrated, deliberately removed, or explicitly
  deferred; unreachable experimental architecture is not carried forward by
  accident;
- the full real-time, determinism, audio comparison, performance, and workspace
  quality gates pass;
- the declared feature/MSRV/platform/package/visualizer matrix passes and the
  core dependency graph contains no forbidden upward frontend/protocol/device
  dependencies;
- no production code path depends on the V1 engine.
