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
8. A successor may instead supersede **named clauses only**. The old record keeps its status, gains
   `Superseded in part` in its metadata naming the successor, and the successor's `Supersedes` field lists **every**
   clause it replaces — no clause is superseded by implication. Anything not named still binds.
9. **Partial supersession takes effect when the successor is accepted, not when it is written.** Only an accepted
   decision constrains implementation, so while the successor is `Proposed` the old clauses remain in force — a
   `Superseded in part` pointer added early is a notice that a replacement exists, never a repeal. Anything that
   depends on the replacement therefore depends on a `Proposed` record and must say so, which is what keeps the gap
   visible instead of leaving two records that each look authoritative.

Accepted ADRs are immutable except for spelling, formatting, and repaired links. New evidence may be appended in a
clearly dated addendum, but changing the decision requires a superseding ADR.

**Why clause-level supersession exists, and when it is the wrong tool.** ADR-0021 is the case that produced it: two of
its clauses rest on facts a later audit disproved, while the rest of the record — two orthogonal axes, seven
configuration owners, six failure classes, four of five site dispositions — is unaffected and is cited throughout the
inventories and the host-profile specification. Superseding the whole record would have required restating all of that
to change two clauses, and a restatement is where reasoning gets lost. **Use whole-record supersession when the
decision's shape changes**, and clause-level only when the surviving clauses are genuinely independent of the replaced
ones. If a reader cannot apply the old record without also reading the successor for most questions, the split is
wrong and the record should be superseded whole.

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

### Every claim about current behaviour carries a `file:line` citation

A record that says what V1 does must name where, at a stated revision. This is not bibliographic decoration: it is the
one rule that separates a behaviour someone read from a behaviour someone assumed, and the two look identical in prose.

Two defects that reached review illustrate the cost of skipping it, and both were in claims written **without** a
citation while the surrounding record cited its other sources:

- ADR-0014's first revision asserted that a module's phase is deterministic, having read `Oscillator::note_on` and seen
  the `Phase::ZERO` branch — without reading what `uni_phase` defaults to (`oscillator.rs:149` at `3555c52c`, `NormalizedValue::MAX`).
  The claim was backwards.
- EVD-0004's first revision asserted that the delay is linear, so that a difference could be attributed to a clipper.
  It soft-clips its feedback write (`effects/delay.rs:300` at `3555c52c`), and the attribution was unfounded.

A claim you cannot cite is a claim to go and check, not a claim to soften with "probably".
