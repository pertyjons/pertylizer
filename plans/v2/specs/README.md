# V2 Normative Specifications

Specifications describe the current contract that implementation and tests must follow. They are created when a
subsystem is sufficiently decided to need one coherent normative description.

Examples expected during the migration include:

- core invariants and dependency boundaries;
- Project Core document and identity model;
- Application Core operation and revision contract;
- Sound Core graph, compiler, and render-plan contract;
- Project Format V2 and asset contract;
- host I/O, clock, latency, and admission contract;
- observation and long-running job contracts.

Do not create speculative placeholder specifications. Until a specification is created and backed by accepted ADRs, the
master plan and accepted individual decisions remain the applicable guidance.

## ADR versus specification

- An ADR records **why** a choice was made and what alternatives were rejected.
- A specification records **what** the system must do now.

When an accepted decision changes behavior, update the current specification in the same change. Preserve the old
reasoning in its ADR rather than retaining several competing versions of a living specification.

## Status vocabulary

- `Draft` — the contract is incomplete or still depends on unresolved ADRs;
- `Current` — the normative contract backed by accepted decisions;
- `Superseded` — replaced in full by another named specification.

Only a `Current` specification constrains implementation. A superseded
specification remains at its stable path and links to its replacement.

## Required specification shape

Copy [../templates/spec.md](../templates/spec.md). Each specification includes:

- status and last-reviewed date;
- scope and explicit non-goals;
- terminology and related accepted ADRs;
- normative invariants using unambiguous language;
- public/domain types and ownership boundaries;
- lifecycle, timing, failure, and diagnostic behavior;
- real-time and resource constraints where applicable;
- conformance tests and unresolved questions.

Use `spec-<short-kebab-case-name>.md`. A specification has no numeric identifier,
but it has a permanent, globally unique uppercase invariant prefix registered
below. Invariants use `<PREFIX>-INV-NNN`, for example `SOUND-INV-001`, so a test,
diagnostic, ADR, or review never cites an ambiguous bare `INV-1`. Neither the
file name nor invariant prefix is reused; prefer preserving the stable file name
when its display title changes.

## Specification register

| Specification | Invariant prefix | Status | Scope |
| --- | --- | --- | --- |

Add a row when creating a specification and check that its prefix is unique
before allocating invariants.

Use domain newtypes in all proposed Rust interfaces. Raw primitives may appear only for arithmetic, serialization
internals, or external wire boundaries.
