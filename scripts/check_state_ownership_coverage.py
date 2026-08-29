#!/usr/bin/env python3
"""Check that the state-ownership ledger covers the persisted project schema.

The ledger claims that every currently persisted field appears exactly once. This
script is what makes that claim falsifiable: it enumerates every leaf-valued path
in ``schemas/project.schema.json`` and requires each one to be claimed by exactly
one ledger entry, using the coverage map the ledger itself carries.

Four failures are the point of the check:

* a persisted field no ledger entry claims — the omission the ledger exists to
  prevent;
* a map rule that claims nothing — a ledger entry describing a field the format
  no longer has, or a path that was renamed;
* two rules claiming one path prefix, or a rule naming an entry the ledger does
  not define, or a ledger entry that is neither mapped nor declared as living
  outside the project schema;
* an entry marked `Classified` while a required cell is blank.

Run from the repository root:

    python3 -B scripts/check_state_ownership_coverage.py
"""

from __future__ import annotations

import json
import re
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
SCHEMA_PATH = REPO_ROOT / "schemas" / "project.schema.json"
LEDGER_PATH = REPO_ROOT / "plans" / "v2" / "inventories" / "state-ownership.md"

MAP_HEADING = "### Schema coverage map"
OUTSIDE_HEADING = "### Entries outside the project schema"
ENTRY_RE = re.compile(r"STATE-\d{4}")


def leaf_paths(schema: dict) -> set[str]:
    """Every leaf-valued path in the schema, with ``[]`` for an array element.

    A container contributes no path of its own: it is covered when its leaves
    are. Recursion through ``$ref`` is cut by remembering the resolved node on
    the current branch, so a self-referential definition yields the path that
    re-enters it and stops there.
    """
    defs = schema.get("$defs", {})

    def resolve(node):
        while isinstance(node, dict) and "$ref" in node:
            ref = node["$ref"]
            if not ref.startswith("#/$defs/"):
                raise ValueError(f"unsupported $ref: {ref}")
            node = defs[ref[len("#/$defs/") :]]
        return node

    leaves: set[str] = set()
    containers: set[str] = set()

    def walk(node, path: str, seen: frozenset) -> None:
        node = resolve(node)
        if not isinstance(node, dict):
            return
        for combinator in ("oneOf", "anyOf", "allOf"):
            for sub in node.get(combinator, []):
                walk(sub, path, seen)
        if "items" in node:
            containers.add(path)
            walk(node["items"], path + "[]", seen)
            return
        properties = node.get("properties")
        if properties:
            containers.add(path)
            for name, sub in properties.items():
                child = path + "." + name
                key = id(resolve(sub))
                if key in seen:
                    leaves.add(child)
                    continue
                walk(sub, child, seen | {key})
            return
        if path:
            leaves.add(path)

    walk(schema, "", frozenset())
    return leaves - containers


def parse_table(text: str, heading: str) -> list[tuple[str, str]]:
    """Rows of the two-column Markdown table under ``heading``."""
    start = text.index(heading)
    rows: list[tuple[str, str]] = []
    for line in text[start:].split("\n")[1:]:
        if line.startswith("#"):
            break
        if not line.startswith("|"):
            continue
        cells = [c.strip() for c in line.strip().strip("|").split("|")]
        if len(cells) != 2 or set(cells[0]) <= set("-: "):
            continue
        if cells[0] in ("Schema path prefix", "Entry"):
            continue
        rows.append((cells[0].strip("`"), cells[1]))
    return rows


LEDGER_ROW_CELLS = 12  # a ten-column row splits into twelve, counting both edges
LEDGER_HEADER = "| ID | Field/state |"
MIGRATION_CELL = 8
EVIDENCE_CELL = 9
STATUS_CELL = 10


def ledger_rows(text: str) -> dict[str, list[str]]:
    """Entries the ledger *defines*, keyed by id, as their raw cells.

    A row counts only while the most recent table header is the ledger's own. The
    coverage map and the outside-the-schema table also start their rows with an
    entry id, and a shape test alone is too weak: any row elsewhere in the file
    that happened to have ten columns would define an entry, which is what makes
    the "names an entry the ledger does not define" checks unable to fire.
    """
    rows: dict[str, list[str]] = {}
    in_ledger = False
    for line in text.split("\n"):
        if not line.startswith("|"):
            # Any non-row line ends the table, a blank one included. Markdown
            # tables are contiguous, so nothing weaker is safe: leaving the flag
            # set across a blank line lets the next ten-column table define
            # entries, which is the hole this function exists to close.
            in_ledger = False
            continue
        if line.startswith(LEDGER_HEADER):
            in_ledger = True
            continue
        if not in_ledger:
            continue
        cells = line.split("|")
        if len(cells) != LEDGER_ROW_CELLS:
            continue
        name = cells[1].strip()
        if ENTRY_RE.fullmatch(name):
            rows[name] = cells
    return rows


def claim(path: str, rules: list[tuple[str, str]]) -> tuple[str, str] | None:
    """The longest matching rule, so a field-level rule beats its container."""
    best: tuple[str, str] | None = None
    for prefix, entry in rules:
        if path == prefix or path.startswith(prefix + ".") or path.startswith(prefix + "["):
            if best is None or len(prefix) > len(best[0]):
                best = (prefix, entry)
    return best


def main() -> int:
    schema = json.loads(SCHEMA_PATH.read_text(encoding="utf-8"))
    ledger = LEDGER_PATH.read_text(encoding="utf-8")

    paths = leaf_paths(schema)
    rules = parse_table(ledger, MAP_HEADING)
    outside = {entry for entry, _ in parse_table(ledger, OUTSIDE_HEADING)}
    rows = ledger_rows(ledger)
    defined = set(rows)

    failures: list[str] = []

    repeated = sorted({p for p in (prefix for prefix, _ in rules) if [q for q, _ in rules].count(p) > 1})
    if repeated:
        failures.append(
            "coverage rules claiming the same path prefix, so a field is claimed twice:\n  "
            + "\n  ".join(repeated)
        )

    used: dict[str, int] = {prefix: 0 for prefix, _ in rules}
    unclaimed: list[str] = []
    for path in sorted(paths):
        hit = claim(path, rules)
        if hit is None:
            unclaimed.append(path)
        else:
            used[hit[0]] += 1
    if unclaimed:
        failures.append(
            "persisted fields claimed by no ledger entry:\n  "
            + "\n  ".join(unclaimed[:40])
            + (f"\n  ... and {len(unclaimed) - 40} more" if len(unclaimed) > 40 else "")
        )

    dead = sorted(prefix for prefix, count in used.items() if count == 0)
    if dead:
        failures.append("coverage rules matching no persisted field:\n  " + "\n  ".join(dead))

    mapped = {entry for _, entry in rules}
    unknown = sorted(mapped - defined)
    if unknown:
        failures.append("coverage rules naming an undefined entry:\n  " + "\n  ".join(unknown))

    stray = sorted(outside - defined)
    if stray:
        failures.append("outside-the-schema list naming an undefined entry:\n  " + "\n  ".join(stray))

    unaccounted = sorted(defined - mapped - outside)
    if unaccounted:
        failures.append(
            "ledger entries neither mapped nor declared outside the schema:\n  "
            + "\n  ".join(unaccounted)
        )

    both = sorted(mapped & outside)
    if both:
        failures.append("entries both mapped and declared outside the schema:\n  " + "\n  ".join(both))

    # The register vocabulary makes `Classified` a row-level status: required
    # fields and disposition filled with supporting evidence. A row that claims
    # it while a required cell is blank is the defect that downgraded every
    # status here once already.
    premature = sorted(
        name
        for name, cells in rows.items()
        if cells[STATUS_CELL].strip() == "Classified"
        and not (cells[MIGRATION_CELL].strip() and cells[EVIDENCE_CELL].strip())
    )
    if premature:
        failures.append(
            "entries marked `Classified` with a blank required cell:\n  " + "\n  ".join(premature)
        )

    if failures:
        for failure in failures:
            print(f"FAIL: {failure}", file=sys.stderr)
        return 1

    print(
        f"State-ownership coverage passed: {len(paths)} persisted leaf paths, "
        f"{len(rules)} coverage rules, {len(defined)} ledger entries "
        f"({len(outside)} outside the project schema)."
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
