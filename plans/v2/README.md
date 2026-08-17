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

1. [WORKING-AGREEMENT.md](WORKING-AGREEMENT.md) — how evidence, decisions, gates, and reviews are handled;
2. [STATUS.md](STATUS.md) — a short index of what is active and blocked;
3. the active file under [phases/](phases/README.md) — authoritative task status and next actions;
4. relevant accepted decisions in [ADR.md](ADR.md) and [decisions/](decisions/README.md);
5. relevant current contracts under [specs/](specs/README.md);
6. the affected sections of the master
   [architecture and migration plan](master-plan.md).

Read the entire master plan when changing phase boundaries, foundational architecture, migration order, or the
definition of done. A narrowly scoped implementation task normally needs only the relevant sections after the documents
above have been read.

## Sources of truth

| Question                                          | Authoritative location                |
|---------------------------------------------------|---------------------------------------|
| What are we building and in what order?           | [master-plan.md](master-plan.md)      |
| How do we investigate, decide, and review it?     | [WORKING-AGREEMENT.md](WORKING-AGREEMENT.md) |
| Where are we now?                                 | [phases/](phases/README.md)           |
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
├── WORKING-AGREEMENT.md      # Evidence, decision, gate, and review workflow
├── STATUS.md                 # Small derived current-state index
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

The operational rules live in [WORKING-AGREEMENT.md](WORKING-AGREEMENT.md). Use the files in
[templates/](templates/) rather than inventing new document formats, and keep all project documentation, code, UI
strings, and commit messages in English.
