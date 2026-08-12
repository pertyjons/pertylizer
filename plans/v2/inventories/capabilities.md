# Capability and Reachability Inventory

| Field         | Value                  |
|---------------|------------------------|
| Status        | Draft                  |
| Phase         | 00B                    |
| Last reviewed | 2026-08-12             |

This ledger covers every shipped or externally consumed capability and assigns it a deliberate V2 disposition.

## Known baseline seeds

These counts come from the architecture audit recorded in the master plan. They are discovery seeds, not completeness
assertions or frozen product limits.

| Surface                       | Known baseline |
|-------------------------------|---------------:|
| MCP tools                     |            219 |
| Module types                  |             75 |
| Programmatic built-in patches |             68 |
| Group templates               |             12 |

The complete audit must also discover GUI actions, menus, shortcuts, dialogs, background jobs, CLI entry points, public
Rust exports, formats, schemas, examples, OSC, the standalone visualizer, configuration, and tested-only or exported
subsystems.

## Allowed dispositions

- `Migrate`
- `Replace`
- `Remove`
- `Defer`
- `Compatibility adapter`

## Ledger

Entries use `CAP-NNNN` identifiers. Next free identifier: `CAP-0001`.

| ID | Surface | Capability | Reachable from | Disposition | V2 owner/replacement | Evidence | Status |
|----|---------|------------|----------------|-------------|----------------------|----------|--------|

## Audit passes

| Date | Source revision | Discovery method | Coverage/result | Evidence |
|------|-----------------|------------------|-----------------|----------|

Completion requires each discovered entry to have reachability, disposition, V2 ownership, and verification. Matching
the seed counts alone is insufficient.
