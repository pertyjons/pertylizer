# V2 Inventories

Inventories are exhaustive migration ledgers. They protect against both silent loss of shipped behavior and accidental
migration of dead or aspirational V1 architecture.

The four initial inventories are:

- [resource-limits.md](resource-limits.md) — real-time caps and V2 admission rules (**Phase 0A**, gates Phase 1);
- [state-ownership.md](state-ownership.md) — persisted and transient state plus intended V2 ownership (Phase 0B);
- [capabilities.md](capabilities.md) — product, protocol, format, module, and external-consumer coverage (Phase 0B);
- [identities.md](identities.md) — domain identities and cross-boundary references (Phase 0B).

The resource-limit audit is separated because `HostProfile` and admission control depend on it before any V2 code
renders. The other three gate Phase 10 and may be completed while Phases 1-4 run.

Each ledger owns one identifier series and records its own next free number:
`CAP` for capabilities, `STATE` for state ownership, `IDN` for identities, and `LIMIT` for resource limits.

## Register status vocabulary

- `Draft` — the register structure exists but its audit has not begun;
- `Active` — an audit is in progress against a named source revision;
- `Current` — the audit is complete against its recorded revision and no known
  repository change has invalidated it;
- `Needs review` — later changes may have invalidated coverage or conclusions.

The current register is never marked `Superseded` or moved to the archive. A
frozen snapshot may be archived for a review while the living register remains
at its stable path.

Each ledger row uses one of these entry statuses:

- `Discovered` — the entry exists but required fields are incomplete;
- `Investigating` — ownership, reachability, behavior, or migration is being
  established;
- `Classified` — required fields and disposition are filled with supporting
  evidence;
- `Verified` — implementation or an explicit removal/defer decision has passed
  its named migration verification;
- `Needs review` — a later change may have invalidated this entry.

Disposition and entry status are independent. For example, a capability may
have disposition `Remove` and status `Verified` after removal and its coverage
test are confirmed.

## Inventory rules

1. Every entry receives a stable identifier before it is referenced elsewhere.
2. Coverage is not complete merely because a known baseline count matches.
3. Record the discovery method and source revision for every audit pass.
4. Never delete an entry to hide removal. Mark its disposition explicitly.
5. Link decisions, tasks, evidence, implementation, and tests by identifier.
6. A blank field means "not yet investigated", never "not applicable".
7. Use `N/A` only with a short explanation.

Inventory files are living registers. A frozen audit snapshot needed for a review may be archived, but the current
register stays at its stable path.
