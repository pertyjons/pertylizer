# V2 Archive Policy and Index

The archive stores non-authoritative historical material whose durable outcome has already been captured in an ADR,
specification, inventory, evidence record, or exit review. It is not a general dumping ground.

## What never moves here merely because it is old

- current control-plane files (`README.md`, `PROCESS.md`, `ROADMAP.md`,
  `NOW.md`) or the glossary;
- the decision register or accepted/superseded ADRs;
- current specifications and inventories;
- accepted exit reviews;
- evidence referenced by an accepted ADR or review.

Stable records remain at stable paths so links from code, commits, tests, and other documents do not decay. Age alone is
not an archive criterion.

## What may be archived

- working notes after their conclusions are captured;
- abandoned proposals that never became ADRs;
- frozen inventory snapshots retained for a review;
- preliminary or obsolete reports with no durable incoming reference;
- completed temporary coordination/handoff documents and frozen trackers from
  the former workflow;
- exploratory material that is useful historically but no longer authoritative.

Pure junk should be deleted; Git already preserves repository history.

## Organization

Archive by migration phase rather than calendar year:

```text
archive/
├── README.md
├── phase-00a/
│   ├── INDEX.md
│   ├── working-notes/
│   ├── rejected-proposals/
│   ├── inventory-snapshots/
│   └── obsolete-reports/
├── phase-00b/
│   └── INDEX.md
├── phase-01/
│   └── INDEX.md
├── phase-03/
│   ├── INDEX.md
│   └── process-history.md
├── phase-05/
│   ├── INDEX.md
│   └── slices.md
└── phase-06/
    ├── INDEX.md
    └── slices.md
```

Create a phase directory only when something is archived.

## Archive procedure

1. Confirm that the material is not an authority or permanent dependency.
2. Capture durable conclusions in the correct active document.
3. Mark the source lifecycle `Archived` where its format supports metadata.
4. Move it with Git so history remains traceable.
5. Repair all Markdown links and search the repository for the old path.
6. Add an entry to that phase's `INDEX.md`.

Each phase archive index uses:

| Original ID/path | Archived path | Reason | Durable replacement | Archived date |
|------------------|---------------|--------|---------------------|---------------|

## Superseded is not archived

An accepted ADR or permanent evidence record that has been replaced is marked
`Superseded` and stays at its original path with a two-way link to its replacement. This preserves the reasoning chain
and prevents broken references.

## Large artifacts

Do not archive large audio, profiler dumps, traces, or build outputs here. Keep a compact evidence record with source
revision, checksum, result, storage location where applicable, and reproduction commands. Automated regression fixtures
belong with the tests that consume them.
