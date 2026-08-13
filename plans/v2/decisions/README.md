# V2 Architecture Decision Records

This directory contains one durable record per considered V2 decision **of the `Contract` class**. The canonical
status, class, and next identifier live in [../ADR.md](../ADR.md).

A `Reversible` decision — a value whose later change costs a rebuild and nothing else — has no file here. Its row in
the register's [reversible-decisions table](../ADR.md#reversible-decisions) is the record. Read
[the reversibility test](../ADR.md#the-reversibility-test) before concluding that a decision is one; the default class
is `Contract`, and a decision that defines a shape, a contract clause, or anything a saved file carries is never
reversible.

## File naming

Use:

```text
ADR-NNNN-short-kebab-case-title.md
```

The numeric identifier is permanent. A renamed title must not allocate a new identifier.
Copy [../templates/adr.md](../templates/adr.md) when creating a record.

## Decision lifecycle

1. Add or select a `Proposed` entry in the decision register, and decide its class. The rest of this list is the
   `Contract` path; a `Reversible` entry is finished in the register itself.
2. Create the individual ADR before implementation depends on the choice.
3. Link relevant inventory entries, evidence records, prototypes, and plan gates.
4. Record options fairly, including the status quo.
5. Accept, reject, or defer the ADR explicitly.
6. Update the register and affected specifications.
7. If the decision changes later, create a new ADR and mark the old one
   `Superseded`.

Accepted ADRs are immutable except for spelling, formatting, and repaired links. New evidence may be appended in a
clearly dated addendum, but changing the decision requires a superseding ADR.

## Decision quality

An ADR must state:

- the concrete problem and decision boundary;
- constraints and decision drivers;
- realistic alternatives and their tradeoffs;
- evidence used and important uncertainties;
- the chosen outcome in implementable language;
- positive and negative consequences;
- follow-up work and revisit conditions.

Measurements are required when the choice depends on CPU, allocation, latency, audio quality, compile time, capacity, or
host behavior. Product and domain choices still require explicit scenarios and failure semantics.
