# Persisted-State Ownership Inventory

| Field         | Value                  |
|---------------|------------------------|
| Status        | Draft                  |
| Phase         | 00B                    |
| Last reviewed | 2026-08-12             |

This ledger records every field currently saved, reconstructed, mirrored, or used as a dirty/undo signal, then assigns
one intended V2 state partition.

## V2 ownership classes

- `Project document` — authored state, including intentionally persisted
  `EditorMetadata`;
- `Runtime session` — transport, focus/selection, preview, recording, active
  device connection, and compile/plan coordination;
- `User settings` — persisted per-user preferences outside the project;
- `Frontend-local transient` — hover, scroll caches, open dialogs, temporary
  input, drag state, and comparable view-local state;
- `Host/service configuration` — deployment, protocol, authorization, feature,
  and default resource policy;
- `Runtime job` — revision-pinned render, export, analysis, or conversion work;
- `Runtime telemetry` — lossy observation and counters;
- `Removed` — a current field or mirror with no V2 equivalent.

These classes are mutually exclusive. `EditorMetadata` is not a peer of the
project document: it is the persisted presentation-intent section inside it.
Frontend-local presentation state is a separate non-persisted class.

## Ledger

Entries use `STATE-NNNN` identifiers. Next free identifier: `STATE-0001`.

| ID | Field/state | Domain type | Current owner | Mirrors/save sources | Dirty/undo behavior | Intended V2 owner | Migration | Evidence | Status |
|----|-------------|-------------|---------------|----------------------|---------------------|-------------------|-----------|----------|--------|

## Required workflow coverage

Audit GUI save, MCP save, autosave, recovery, rollback, patch/preset save, bundle save, import/export, and offline
render reconstruction. A field is not complete until every current mirror and save source is understood.

## Audit passes

| Date | Source revision | Paths inspected | Coverage/result | Evidence |
|------|-----------------|-----------------|-----------------|----------|
