# SPEC: Subsystem Contract Title

| Field            | Value              |
|------------------|--------------------|
| Status           | Draft              |
| Phase            | NN                 |
| Created          | YYYY-MM-DD         |
| Last reviewed    | YYYY-MM-DD         |
| Based on         | ADR-NNNN, ADR-NNNN |
| Invariant prefix | EXAMPLE            |
| Supersedes       | —                  |
| Superseded by    | —                  |

Allowed status values are defined in [../specs/README.md](../specs/README.md).
Only a `Current` specification constrains implementation.

## Scope

State the subsystem this contract governs.

## Non-goals

State what this contract deliberately does not decide, and name the document
that does where one exists.

## Terminology

Define terms not already in [../glossary.md](../glossary.md), or link the
glossary entry.

## Accepted decisions

| ADR | Decision it fixes here |
|-----|------------------------|

A specification records only what follows from accepted decisions. An open
decision appears as an unresolved question below, never as an invented rule.

## Invariants

Use normative language. Each invariant is testable and numbered so tests,
diagnostics, and reviews can cite it.

1. **EXAMPLE-INV-001** — The system must …

Replace `EXAMPLE` with the globally unique prefix registered in
[../specs/README.md](../specs/README.md). Never renumber or reuse an invariant
identifier after it has been cited.

## Types and ownership

Public and domain types, who owns each value, and which layer may mutate it.
Use domain newtypes; raw primitives appear only in arithmetic, serialization
internals, or external wire boundaries.

```rust,ignore
// Proposed interface
```

## Lifecycle and timing

Construction, preparation, activation, retirement, and the thread or phase each
step belongs to.

## Failure and diagnostics

Error cases, their structured codes, what remains valid after a failure, and
what the user sees.

## Real-time and resource constraints

Applicable allocation, locking, bounded-work, and admission rules, plus the
`HostProfile` fields involved. Use `N/A` with a reason when the subsystem never
touches the audio thread.

## Conformance tests

| Invariant | Named test or evidence |
|-----------|------------------------|

## Unresolved questions

| Question | Blocking? | ADR or task |
|----------|-----------|-------------|
