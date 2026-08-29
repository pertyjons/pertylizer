#!/usr/bin/env python3
"""Tests for the state-ownership coverage check.

Every failure mode the check claims to catch is exercised against a mutated
input, because a check that has never been observed to fail establishes nothing.
"""

from __future__ import annotations

import io
import json
import sys
import tempfile
import unittest
from contextlib import redirect_stderr, redirect_stdout
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import check_state_ownership_coverage as cov  # noqa: E402


MINIMAL_SCHEMA = {
    "type": "object",
    "properties": {
        "file_type": {"type": "string"},
        "song": {
            "type": "object",
            "properties": {
                "name": {"type": "string"},
                "tracks": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "id": {"type": "integer"},
                            "solo": {"type": "boolean"},
                        },
                    },
                },
            },
        },
    },
}


def ledger(rules, outside, entries, status="Investigating", migration="mig", evidence="EVD-0018"):
    rows = "\n".join(
        f"| {e} | f | t | o | m | d | v | {migration} | {evidence} | {status} |" for e in entries
    )
    rule_rows = "\n".join(f"| `{p}` | {e} |" for p, e in rules)
    outside_rows = "\n".join(f"| {e} | {why} |" for e, why in outside)
    return f"""# Ledger

| ID | Field/state | Domain type | Current owner | Mirrors | Dirty | Intended V2 owner | Migration | Evidence | Status |
|----|----|----|----|----|----|----|----|----|----|
{rows}

### Schema coverage map

| Schema path prefix | Entry |
|---|---|
{rule_rows}

### Entries outside the project schema

| Entry | Why it is not in the project schema |
|---|---|
{outside_rows}

## Next section
"""


GOOD_RULES = [
    (".file_type", "STATE-0001"),
    (".song.name", "STATE-0002"),
    (".song.tracks", "STATE-0003"),
    (".song.tracks[].solo", "STATE-0004"),
]
GOOD_OUTSIDE = [("STATE-0005", "Session state, never persisted")]
GOOD_ENTRIES = ["STATE-0001", "STATE-0002", "STATE-0003", "STATE-0004", "STATE-0005"]


class CoverageCheckTest(unittest.TestCase):
    def run_check(self, schema, ledger_text):
        with tempfile.TemporaryDirectory() as tmp:
            schema_path = Path(tmp) / "schema.json"
            ledger_path = Path(tmp) / "ledger.md"
            schema_path.write_text(json.dumps(schema), encoding="utf-8")
            ledger_path.write_text(ledger_text, encoding="utf-8")
            old = (cov.SCHEMA_PATH, cov.LEDGER_PATH)
            cov.SCHEMA_PATH, cov.LEDGER_PATH = schema_path, ledger_path
            try:
                out, err = io.StringIO(), io.StringIO()
                with redirect_stdout(out), redirect_stderr(err):
                    code = cov.main()
                return code, out.getvalue() + err.getvalue()
            finally:
                cov.SCHEMA_PATH, cov.LEDGER_PATH = old

    def test_minimal_input_passes(self):
        code, _ = self.run_check(MINIMAL_SCHEMA, ledger(GOOD_RULES, GOOD_OUTSIDE, GOOD_ENTRIES))
        self.assertEqual(code, 0)

    def test_unclaimed_persisted_field_fails(self):
        schema = json.loads(json.dumps(MINIMAL_SCHEMA))
        schema["properties"]["song"]["properties"]["tempo"] = {"type": "number"}
        code, text = self.run_check(schema, ledger(GOOD_RULES, GOOD_OUTSIDE, GOOD_ENTRIES))
        self.assertEqual(code, 1)
        self.assertIn(".song.tempo", text)

    def test_rule_matching_nothing_fails(self):
        rules = GOOD_RULES + [(".song.gone", "STATE-0002")]
        code, text = self.run_check(MINIMAL_SCHEMA, ledger(rules, GOOD_OUTSIDE, GOOD_ENTRIES))
        self.assertEqual(code, 1)
        self.assertIn(".song.gone", text)

    def test_rule_naming_undefined_entry_fails(self):
        rules = [(p, "STATE-9999" if p == ".file_type" else e) for p, e in GOOD_RULES]
        entries = [e for e in GOOD_ENTRIES if e != "STATE-0001"]
        code, text = self.run_check(MINIMAL_SCHEMA, ledger(rules, GOOD_OUTSIDE, entries))
        self.assertEqual(code, 1)
        self.assertIn("STATE-9999", text)

    def test_entry_neither_mapped_nor_outside_fails(self):
        entries = GOOD_ENTRIES + ["STATE-0006"]
        code, text = self.run_check(MINIMAL_SCHEMA, ledger(GOOD_RULES, GOOD_OUTSIDE, entries))
        self.assertEqual(code, 1)
        self.assertIn("STATE-0006", text)

    def test_duplicate_prefix_fails(self):
        rules = GOOD_RULES + [(".file_type", "STATE-0002")]
        code, text = self.run_check(MINIMAL_SCHEMA, ledger(rules, GOOD_OUTSIDE, GOOD_ENTRIES))
        self.assertEqual(code, 1)
        self.assertIn("claimed twice", text)

    def test_outside_table_naming_undefined_entry_fails(self):
        outside = GOOD_OUTSIDE + [("STATE-9998", "not a ledger row")]
        code, text = self.run_check(MINIMAL_SCHEMA, ledger(GOOD_RULES, outside, GOOD_ENTRIES))
        self.assertEqual(code, 1)
        self.assertIn("STATE-9998", text)

    def test_reference_tables_do_not_define_entries(self):
        text = ledger(GOOD_RULES, GOOD_OUTSIDE, GOOD_ENTRIES)
        self.assertEqual(set(cov.ledger_rows(text)), set(GOOD_ENTRIES))

    def test_a_ten_column_row_outside_the_ledger_table_defines_nothing(self):
        text = ledger(GOOD_RULES, GOOD_OUTSIDE, GOOD_ENTRIES)
        text += "\n## Elsewhere\n\n| STATE-9999 | f | t | o | m | d | v | mig | e | Classified |\n"
        self.assertNotIn("STATE-9999", cov.ledger_rows(text))

    def test_a_later_table_after_a_blank_line_defines_nothing(self):
        text = ledger(GOOD_RULES, GOOD_OUTSIDE, GOOD_ENTRIES)
        text += "\n| STATE-9997 | f | t | o | m | d | v | mig | e | Classified |\n"
        self.assertNotIn("STATE-9997", cov.ledger_rows(text))

    def test_classified_row_with_a_blank_required_cell_fails(self):
        text = ledger(GOOD_RULES, GOOD_OUTSIDE, GOOD_ENTRIES, status="Classified", migration="")
        code, out = self.run_check(MINIMAL_SCHEMA, text)
        self.assertEqual(code, 1)
        self.assertIn("blank required cell", out)

    def test_classified_row_with_every_required_cell_passes(self):
        text = ledger(GOOD_RULES, GOOD_OUTSIDE, GOOD_ENTRIES, status="Classified")
        code, out = self.run_check(MINIMAL_SCHEMA, text)
        self.assertEqual(code, 0, out)

    def test_entry_both_mapped_and_outside_fails(self):
        outside = GOOD_OUTSIDE + [("STATE-0001", "claimed twice")]
        code, text = self.run_check(MINIMAL_SCHEMA, ledger(GOOD_RULES, outside, GOOD_ENTRIES))
        self.assertEqual(code, 1)
        self.assertIn("both mapped and declared", text)

    def test_longest_prefix_wins_over_container(self):
        hit = cov.claim(".song.tracks[].solo", GOOD_RULES)
        self.assertEqual(hit, (".song.tracks[].solo", "STATE-0004"))
        hit = cov.claim(".song.tracks[].id", GOOD_RULES)
        self.assertEqual(hit, (".song.tracks", "STATE-0003"))

    def test_containers_contribute_no_path(self):
        paths = cov.leaf_paths(MINIMAL_SCHEMA)
        self.assertIn(".song.tracks[].id", paths)
        self.assertNotIn(".song", paths)
        self.assertNotIn(".song.tracks", paths)

    def test_recursive_ref_terminates(self):
        schema = {
            "$defs": {
                "Node": {
                    "type": "object",
                    "properties": {"name": {"type": "string"}, "child": {"$ref": "#/$defs/Node"}},
                }
            },
            "type": "object",
            "properties": {"root": {"$ref": "#/$defs/Node"}},
        }
        paths = cov.leaf_paths(schema)
        self.assertIn(".root.name", paths)
        self.assertIn(".root.child", paths)
        self.assertTrue(all(p.count(".child") <= 1 for p in paths))

    def test_repository_ledger_and_schema_agree(self):
        out, err = io.StringIO(), io.StringIO()
        with redirect_stdout(out), redirect_stderr(err):
            code = cov.main()
        self.assertEqual(code, 0, out.getvalue() + err.getvalue())


if __name__ == "__main__":
    unittest.main()
