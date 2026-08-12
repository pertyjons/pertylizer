# Real-Time Resource and Admission Inventory

| Field         | Value                  |
|---------------|------------------------|
| Status        | Draft                  |
| Phase         | 00A                    |
| Last reviewed | 2026-08-12             |

This ledger records every current fixed limit, truncation point, bounded queue, buffer capacity, and script budget. V2
must preserve, raise, remove, or expose each as an explicit admission rule with a structured diagnostic.

## Limit classes

- `Platform capability`
- `Configurable safety budget`
- `Warning threshold`
- `Implementation artifact to remove`
- `Unknown`

## Ledger

Entries use `LIMIT-NNNN` identifiers. Next free identifier: `LIMIT-0001`.

| ID | Resource/limit | Current value | Enforcement site | Overflow behavior | Limit class | Proposed V2 rule | Diagnostic | Evidence | ADR | Status |
|----|----------------|--------------:|------------------|-------------------|-------------|------------------|------------|----------|-----|--------|

## Required areas

Include render quantum, layouts, voices, nodes, graph edges, events and fan-out, channels, buses, sends, buffers,
telemetry taps, recording buffers, command and event queues, prepared memory, sample zones, and YAMS/script work.

## Audit passes

| Date | Source revision | Search/measurement method | Coverage/result | Evidence |
|------|-----------------|---------------------------|-----------------|----------|
