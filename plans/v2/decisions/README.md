# Core V2 Durable Decisions

This directory contains rationale for durable architecture and product choices.
The compact status index and next identifier are in [`../ADR.md`](../ADR.md).
The decision threshold and review rule are in
[`../PROCESS.md`](../PROCESS.md#durable-decision-or-evidence).

Create an ADR only when a choice crosses a persisted, protocol, public,
real-time/ownership, cross-phase, migration, delivered-behavior, or explicit
product boundary. Internal experimental implementation choices normally belong
in code, tests, an EVD, or the active slice in `NOW.md`.

## Lifecycle

1. `Proposed`: factual premises and the choice are open to one independent
   semantic review.
2. `Accepted`: the durable rationale is historical; update the current
   specification that implementation follows.
3. `Deferred`: name the evidence point or gate and the owner.
4. `Superseded`: retain the old record and accept a successor, but present the
   resulting rule coherently in the current specification.

Accepted ADR reasoning is not rewritten to mirror later status. Factual typos,
links, and metadata may be repaired without changing the decision. A semantic
change uses a successor.

Avoid clause-level supersession as the implementation interface. Historical
records may describe which clauses changed, but implementers read one current
specification rather than merging ADR fragments.

## Review

The author states the decision boundary, evidence, falsifier where applicable,
consequences, and unresolved risk. One reader who did not author the material
reviews it against the stopping rule in `PROCESS.md`. Semantic repairs receive a
focused reread; editorial changes do not start another pass.
