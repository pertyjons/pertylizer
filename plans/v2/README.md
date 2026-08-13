# Pertylizer Core V2 Documentation

This directory is the coordination space for the Pertylizer Core V2 migration. It is written for both human maintainers
and AI agents. The documents separate architecture, current status, decisions, evidence, execution, and historical
material so that no single file has to serve several incompatible purposes.

The V2 effort covers three coupled foundations:

- **Project Core V2** — the canonical project document, identities, assets, and file format;
- **Application Core V2** — operations, validation, transactions, revisions, history, and jobs shared by every frontend;
- **Sound Core V2** — graph compilation and real-time/offline audio rendering.

## Start here

Read these documents in this order before working on V2:

1. [STATUS.md](STATUS.md) — what is active now, what is blocked, and what comes next;
2. the active file under [phases/](phases/README.md) — executable work and verification for the current phase;
3. relevant accepted decisions in [ADR.md](ADR.md) and [decisions/](decisions/README.md);
4. relevant current contracts under [specs/](specs/README.md);
5. the affected sections of the master
   [architecture and migration plan](master-plan.md).

Read the entire master plan when changing phase boundaries, foundational architecture, migration order, or the
definition of done. A narrowly scoped implementation task normally needs only the relevant sections after the documents
above have been read.

## Sources of truth

| Question                                          | Authoritative location                |
|---------------------------------------------------|---------------------------------------|
| What are we building and in what order?           | [master-plan.md](master-plan.md)      |
| Where are we now?                                 | [STATUS.md](STATUS.md)                |
| Which decisions are open or settled?              | [ADR.md](ADR.md)                      |
| Why was a decision made?                          | [decisions/](decisions/README.md)     |
| What observations support it?                     | [evidence/](evidence/README.md)       |
| What exists in V1 and what happens to it?         | [inventories/](inventories/README.md) |
| What work is executable in the current phase?     | [phases/](phases/README.md)           |
| What is the current normative technical contract? | [specs/](specs/README.md)             |
| Has a phase actually passed its gates?            | [reviews/](reviews/README.md)         |
| Where is non-authoritative historical material?   | [archive/](archive/README.md)         |
| What does V1 actually render, and what must survive? | [`corpus/v2-reference/`](../../corpus/v2-reference/README.md) |

The reference corpus is the one authority that does **not** live under
`plans/v2/`. It holds project files and is executed by tests, so it belongs with
the repository's fixtures rather than with its planning documents — see the
artifact policy in [evidence/](evidence/README.md). It is listed here because a
reader who starts at this file would otherwise never find it.

If two documents disagree, do not silently select the convenient answer. Use the table above to identify the authority,
report the conflict, and repair or supersede the other document.

The master plan is authoritative for scope, phase order, and exit gates, but not for details it left open. An accepted
ADR and a current specification supersede provisional wording in the plan; when that happens, update the plan in the
same change.

## Directory map

```text
plans/v2/
├── README.md                 # This guide
├── STATUS.md                 # Small current-state dashboard
├── ADR.md                    # Canonical decision register
├── glossary.md               # Shared V2 terminology
├── master-plan.md            # Master architecture and migration plan
├── decisions/                # One durable record per considered decision
├── diagrams/                 # Explanatory architecture pictures (never normative)
├── evidence/                 # Reproducible experiments and analyses
├── inventories/              # Exhaustive V1-to-V2 coverage ledgers
├── phases/                   # Operational phase trackers
├── specs/                    # Current normative contracts
├── reviews/                  # Formal phase and cutover reviews
├── templates/                # Required document shapes
└── archive/                  # Indexed non-authoritative historical material
```

Directories are populated when they become useful. Do not create empty files for every future phase, specification, or
decision.

## Stable identifiers

Use stable identifiers in documents, issues, commits, test names, and review notes. Identifiers never change when titles
or file names change.

| Prefix       | Meaning                             | Example         |
|--------------|-------------------------------------|-----------------|
| `ADR`        | Architecture or product decision    | `ADR-0007`      |
| `EVD`        | Evidence, experiment, or analysis   | `EVD-0012`      |
| `CAP`        | Capability inventory entry          | `CAP-0042`      |
| `STATE`      | State-ownership entry               | `STATE-0018`    |
| `IDN`        | Identity/reference entry            | `IDN-0009`      |
| `LIMIT`      | Resource-limit entry                | `LIMIT-0011`    |
| `<SPEC>-INV` | Normative specification invariant   | `SOUND-INV-001` |
| `Pxx-T`      | Task in migration phase `xx`        | `P03-T006`      |
| `REV`        | Formal review                       | `REV-P03`       |

Phases 0 and 10 are executed as sub-phases — 0A/0B and 10A–10E — each with its own tracker, gate, and review. They use
the sub-phase letter in the same identifiers: task `P10C-T004`, review `REV-P10C`, tracker `phase-10c-<name>.md`.

Allocate the next number from the relevant register, which records the next free identifier. Never reuse identifiers,
including identifiers belonging to rejected or removed entries.

## Document metadata

Durable records begin with concise metadata. Use ISO 8601 dates.

```text
ID: EVD-0012
Status: Active
Phase: 03
Created: 2026-08-12
Last reviewed: 2026-08-12
Retention: Permanent
Related: ADR-0003, P03-T006
Superseded by: —
```

Allowed lifecycle terms are defined by each document type. Do not invent a new term without adding it to that type's
guide.

## Working rules

1. **Update status, not history.** Keep `STATUS.md` short and current. Git and exit reviews carry history.
2. **Record decisions explicitly.** A discussion is not a decision until it is accepted in the decision register. How
   much apparatus that takes depends on the entry's [class](ADR.md#decision-classes): a `Contract` decision needs an
   accepted record under [decisions/](decisions/README.md), while a `Reversible` one — a value whose later change costs
   a rebuild and nothing else — is accepted as a register row. The class is a judgement about reversibility, never
   about how much work the decision deserves.
3. **Link claims to evidence.** Performance, correctness, parity, and real-time claims require a reproducible `EVD`
   record or a named automated test.
4. **Keep the master plan strategic.** Operational task state belongs in a phase tracker; normative details belong in a
   specification.
5. **Do not duplicate authorities.** A phase tracker may link to a plan gate but must not quietly redefine that gate.
6. **Preserve accepted reasoning.** Accepted ADRs remain immutable apart from spelling or link repairs. Replace a
   changed decision with a superseding ADR.
7. **Close phases formally.** A completed task list does not complete a phase. Its exit review must demonstrate every
   applicable gate.
8. **Archive deliberately.** Move only non-authoritative material after its durable conclusions have been captured
   elsewhere.
9. **Keep repository documents reviewable.** Large audio, traces, profiler dumps, and generated artifacts do not belong
   under `plans/v2/`.
10. **Use English.** Project documentation, code, UI strings, and commit messages follow the repository language rule.

## Expected update flow

When beginning work:

1. confirm the current phase and task in `STATUS.md`;
2. read the phase tracker and relevant accepted ADRs/specifications;
3. create a proposed ADR before making an unresolved architectural choice;
4. create an evidence record before running a decision-driving experiment.

When finishing work:

1. record verification and relevant commits in the phase task;
2. update affected inventories and specifications;
3. update ADR/evidence status where appropriate;
4. update `STATUS.md` with the next actionable state;
5. create or update an exit review only when evaluating a phase gate.

Use the files in [templates/](templates/) rather than inventing new document formats.
