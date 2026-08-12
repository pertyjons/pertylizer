# Identity and Reference Inventory

| Field         | Value                  |
|---------------|------------------------|
| Status        | Draft                  |
| Phase         | 00B                    |
| Last reviewed | 2026-08-12             |

This ledger records identities and references crossing project, GUI, MCP, history, serialization, import, duplication,
and engine boundaries.

## Problems to flag

- raw primitives used as domain identities;
- type- or order-encoded strings;
- positional references;
- IDs reused across domains;
- display names used as identity;
- runtime slots persisted as identity;
- references repaired heuristically at load time;
- duplication/import behavior without explicit remapping.

## Ledger

Entries use `IDN-NNNN` identifiers. Next free identifier: `IDN-0001`.

| ID | Concept/reference | Current type/encoding | Producers | Consumers | Persistence | Known problem | Proposed V2 newtype/rule | Migration | ADR | Status |
|----|-------------------|-----------------------|-----------|-----------|-------------|---------------|--------------------------|-----------|-----|--------|

## Audit passes

| Date | Source revision | Boundaries inspected | Coverage/result | Evidence |
|------|-----------------|----------------------|-----------------|----------|
