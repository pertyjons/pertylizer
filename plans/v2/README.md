# Pertylizer Core V2

This directory coordinates the migration to a canonical Project Core, one
Application Core mutation boundary, and a compiled Sound Core renderer.

## Start here

For ordinary implementation, read only:

1. [`NOW.md`](NOW.md) for the active slice, blockers, and next actions;
2. the current specification linked by that slice;
3. only the ADR or EVD explicitly linked for the question being answered.

Read [`PROCESS.md`](PROCESS.md) when classifying a new decision, designing a
measurement, or closing a phase. Read [`ROADMAP.md`](ROADMAP.md) when changing
phase order, dependencies, outcomes, or exit boundaries.

## Sources of truth

| Question | Authority |
|---|---|
| What is active now? | [`NOW.md`](NOW.md) |
| What are the phase outcomes and order? | [`ROADMAP.md`](ROADMAP.md) |
| How is V2 work reviewed and closed? | [`PROCESS.md`](PROCESS.md) |
| What must implementation do now? | [`specs/`](specs/README.md) |
| Why was a durable decision made? | [`decisions/`](decisions/README.md) |
| What did a measurement establish? | [`evidence/`](evidence/README.md) |
| What exists in V1 and what happens to it? | [`inventories/`](inventories/README.md) |
| Did a phase pass? | [`reviews/`](reviews/README.md) |

The executable V1 comparison corpus lives under
[`corpus/v2-reference/`](../../corpus/v2-reference/README.md), beside the tests
that run it.

If two authorities disagree, follow [`PROCESS.md`](PROCESS.md#authorities). Do
not duplicate the fact in a third document.

## Directory map

```text
plans/v2/
├── README.md          # Navigation and authority map
├── PROCESS.md         # Risk-based workflow and review stopping rule
├── ROADMAP.md         # Phase outcomes, order, dependencies, exit boundaries
├── NOW.md             # The only active task/status authority
├── ADR.md             # Compact durable-decision status index
├── glossary.md        # Shared Core V2 terminology
├── architecture/      # Explanatory target architecture
├── decisions/         # Durable rationale; ADR.md is the compact index
├── evidence/          # Reproducible observations and measurements
├── inventories/       # V1 migration coverage and dispositions
├── specs/             # Current normative implementation contracts
├── reviews/           # Phase and cutover verdicts
├── phases/            # Frozen execution records from the old workflow
├── diagrams/          # Explanatory pictures
├── templates/         # Lean shapes for new durable artifacts
└── archive/           # Superseded non-authoritative material
```

`master-plan.md`, `STATUS.md`, and `WORKING-AGREEMENT.md` remain as stable-path
legacy pointers or historical material. They are not active authorities.

## Stable identifiers

Identifiers do not change when titles or paths change and are never reused.

| Prefix | Meaning | Example |
|---|---|---|
| `ADR` | Durable architecture or product decision | `ADR-0041` |
| `EVD` | Evidence, experiment, or analysis | `EVD-0010` |
| `CAP` | Capability inventory entry | `CAP-0042` |
| `STATE` | State-ownership entry | `STATE-0018` |
| `IDN` | Identity/reference entry | `IDN-0009` |
| `LIMIT` | Resource-limit entry | `LIMIT-0011` |
| `<SPEC>-INV` | Normative specification invariant | `SOUND-INV-001` |
| `Pxx-T` | Migration task | `P02-T013` |
| `REV` | Formal phase or cutover review | `REV-P02` |

Phase codes use their roadmap spelling. Ordinary phases use two digits
(`02`, `11`); split phases retain their letter (`0A`, `0B`, `10A`–`10E`). The
letter is also part of task and review identifiers, for example `P00B-T001`,
`P10C-T004`, `REV-P00B`, and `REV-P10C`.

The relevant register owns the next free identifier. Use the templates for new
durable artifacts, but do not create a document when a test, implementation, or
`NOW.md` completion check is the correct home.

## Document metadata

Durable records use the metadata table in their template. Common fields are
the stable ID where the document type has one, lifecycle status, phase, created
and last-reviewed dates, related identifiers, and both supersession directions.
Type-specific fields such as source revision, retention, invariant prefix, or
reviewed revision remain in that type's template. Use ISO 8601 dates and do not
invent lifecycle terms outside the document type's guide.
