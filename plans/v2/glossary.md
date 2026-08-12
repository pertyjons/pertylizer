# Pertylizer Core V2 Glossary

This glossary gives humans and AI agents one vocabulary for the V2 effort. It describes intended meanings, not a
substitute for accepted ADRs or normative specifications.

## Architecture

**Pertylizer Core V2**

The combined Project Core V2, Application Core V2, and Sound Core V2 architecture.

**Project Core V2**

The canonical, versioned project document, domain identities, authored graphs, arrangement, assets, and intentionally
persisted editor metadata.

**Application Core V2**

The mutation and coordination boundary shared by GUI, MCP, CLI, importers, undo, save, recovery, and jobs. It owns
operations, validation, transactions, revisions, history, and concurrency rules.

**Sound Core V2**

The graph compiler and real-time/offline renderer. During coexistence its crate is expected to be named
`synth_engine_v2`.

**V1**

The current production architecture and engine. V1 remains the production path until V2 passes the plan's cutover gates.

## Graph and rendering model

**Authoring graph**

The editable, identity-rich graph stored by Project Core V2. It favors useful domain semantics and diagnostics rather
than hot-path execution.

**Compiler IR**

An intermediate representation used to validate, lower, schedule, and prepare an authoring graph before real-time
execution.

**Render plan**

A complete immutable execution plan prepared off the audio thread. The audio thread executes it using compact indices
and bounded resources.

**Render quantum**

The engine's internal maximum processing unit. It is distinct from the host audio callback size and remains an open
decision until accepted in an ADR.

**Host callback**

A buffer request made by an audio host or device. V2 output must not change when the same timeline is partitioned into
different callback sizes.

**Host profile**

Declared platform capabilities and resource budgets used to admit or reject a prepared plan before real-time execution.

**Resource report**

The compiler's account of the nodes, buffers, voices, events, taps, script work, and other bounded resources required by
a plan.

## Project and application model

**Project document**

The sole authority for saved musical and intentional editor state. It must be serializable without reconstructing state
from the GUI or audio engine.

**Application operation**

A typed request to inspect or mutate the project through Application Core V2. Every frontend uses the same operation
semantics.

**Project revision**

A stable version of the canonical document used by history, save, compilation, optimistic concurrency, and
revision-pinned jobs.

**Runtime session**

Non-persisted active state such as transport position, record arm, monitoring, preview behavior, and active loop
playback.

**Performance event**

A timestamped live or sequenced event such as note, controller, expression, or panic. It is not a project mutation.

**Runtime telemetry**

Lossy observations such as meters, scopes, CPU reports, and analyzer output. It must not become persisted project truth.

**Runtime job**

A revision-pinned, cancellable operation such as render, export, or analysis, with progress, diagnostics, and a result
receipt.

**Semantic project digest**

A digest of the canonical document's meaning, independent of JSON field order and formatting. It is the equality
criterion for round-trip, conversion, and frontend conformance tests, and is unrelated to an audio digest.

## Project model nouns

These five are distinct on purpose. Most V1-to-V2 confusion comes from collapsing them.

**PatchDefinition**

The reusable sound-design asset: authoring node graph, defaults, descriptions, interface, and canonical asset
references. Several tracks may reference one definition.

**InstrumentInstance**

A project instance referencing a `PatchDefinition`. It owns performance configuration — polyphony, allocation mode,
tuning, glide, controller mapping, and instance overrides — and its own runtime voices.

**InstrumentTrack**

A timeline owner of placements and events. The product-level default is one track to one instance to one channel
strip.

**ChannelStrip**

The mixer channel of an audio-producing track or output: inserts, fader, pan, mute, solo, sends, and meter identity.
Addressed by `ChannelId`, never by `InstrumentId`.

**AutomationTrack**

A track kind with no instrument and no audio channel. It produces parameter and control events, and exposes
enable/bypass rather than audio mute/solo.

**SampleAsset / SampleMap / SampleZone**

`SampleAsset` is source audio identity and provenance. `SampleZone` maps key/velocity range, root/tuning, playback
region, loop, and gain. `SampleMap` groups zones for a sampler source. A one-sample sampler is one zone, not a
separate model.

**TuningDefinition / PreparedTuning**

`TuningDefinition` is canonical authored tuning data with stable identity. `PreparedTuning` is the immutable
renderer-rate lookup derived from it, and is never the persisted representation.

**EditorMetadata**

Presentation intent deliberately saved with the document — layout, groups, colors — keyed by stable project
identities. Selection, scroll, hover, and drag state are not editor metadata.

## Modulation and scripting

**Mod Matrix**

The per-module modulation slot matrix on a voice module: a fixed set of source-to-parameter slots owned by that
module.

**Mod Grid**

The subsystem of pooled control-rate modulator graphs. An individual graph in that pool is a **Mod Graph**, and is a
project asset with its own identity.

**Note Grid**

The corresponding pool of note-processing graphs. A Mod Graph processes control signals; a Note Graph processes note
events.

**YAMS Note / Control / Audio**

The three YAMS execution domains. They share a language and compiler but have distinct interfaces, state scopes, and
cost budgets: event transformation, one bounded evaluation per control segment, and per-sample processing.

## Migration

**LegacyProjectLowerer**

The temporary adapter that converts the current `ProjectFile`/`Song` model into the V2 compiler IR. It holds all
compatibility knowledge; the render plan holds none. It is expected to disappear after Phase 10D.

**Capability ledger**

The capability and reachability inventory. Every shipped or externally consumed capability carries one disposition:
`Migrate`, `Replace`, `Remove`, `Defer`, or `Compatibility adapter`.

**Plan swap**

Publication of a new complete render plan to the audio thread, activated at an internal quantum boundary. Latest
complete revision wins; retired plans are reclaimed off-thread.

**Tap and subscription**

A tap is a compiler-declared stable observation point with a declared data rate and cost. A subscription is the
host-owned bounded consumer of a tap. Neither may change rendered audio.

## Documentation

**ADR (Architecture Decision Record)**

A durable record of a considered decision, its context, evidence, alternatives, outcome, and consequences.

**Evidence record**

A reproducible benchmark, experiment, code audit, workflow analysis, or test report used to support or challenge a
decision or gate.

**Inventory**

An exhaustive ledger used to prevent V1 capabilities, state, identities, or limits from being silently lost or
accidentally preserved.

**Phase tracker**

The operational record for one migration phase. It tracks tasks, deliverables, verification, and deviations without
redefining the master plan.

**Specification**

The current normative technical contract that implementation and tests must follow. ADRs explain why; specifications
explain what applies now.

**Exit review**

A formal evidence-backed evaluation of every gate required to complete a phase.

**Archive**

Indexed, non-authoritative historical material whose durable conclusions have already been captured in active records.
