#!/usr/bin/env python3
"""Regression tests for the Core V2 documentation checker."""

from __future__ import annotations

import importlib.util
import tempfile
import unittest
from pathlib import Path


SCRIPT = Path(__file__).with_name("check_v2_docs.py")
SPEC = importlib.util.spec_from_file_location("check_v2_docs", SCRIPT)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError(f"cannot load {SCRIPT}")
CHECKER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(CHECKER)


class DocumentationCheckerTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary_directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary_directory.cleanup)
        self.root = Path(self.temporary_directory.name)
        self.v2_root = self.root / "plans" / "v2"
        self.v2_root.mkdir(parents=True)
        self.old_repo_root = CHECKER.REPO_ROOT
        self.old_v2_root = CHECKER.V2_ROOT
        CHECKER.REPO_ROOT = self.root
        CHECKER.V2_ROOT = self.v2_root
        self.addCleanup(self.restore_roots)

    def restore_roots(self) -> None:
        CHECKER.REPO_ROOT = self.old_repo_root
        CHECKER.V2_ROOT = self.old_v2_root

    def write(self, relative: str, text: str) -> Path:
        path = self.v2_root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text, encoding="utf-8")
        return path

    def test_same_document_fragments_are_checked(self) -> None:
        self.write(
            "README.md",
            "# Real heading\n\n[valid](#real-heading)\n[invalid](#missing-heading)\n",
        )
        errors: list[str] = []
        CHECKER.check_links(errors)
        self.assertEqual(
            errors,
            ["plans/v2/README.md: missing link fragment: #missing-heading"],
        )

    def test_heading_slug_matches_github_whitespace_behavior(self) -> None:
        heading = "Phase 0A — baseline and render contracts"
        self.assertEqual(
            CHECKER.heading_slug(heading),
            "phase-0a--baseline-and-render-contracts",
        )

    def test_generated_heading_slugs_share_one_collision_namespace(self) -> None:
        anchors = CHECKER.markdown_anchors("# Foo\n# Foo-1\n# Foo\n")
        self.assertEqual(anchors, {"foo", "foo-1", "foo-2"})

    def test_fenced_links_and_headings_are_not_scanned(self) -> None:
        source = self.write(
            "README.md",
            "# Repeated\n\n```markdown\n# Repeated\n[example](missing.md)\n```\n\n"
            "[second](#repeated-1)\n",
        )
        errors: list[str] = []
        CHECKER.check_links(errors)
        self.assertEqual(
            errors,
            [f"{source.relative_to(self.root)}: missing link fragment: #repeated-1"],
        )

    def test_fenced_blocks_inside_markdown_containers_are_not_scanned(self) -> None:
        self.write(
            "README.md",
            "> ```markdown\n> [quoted](missing-quoted.md)\n> # Quoted heading\n> ```\n\n"
            "10. ```markdown\n    [listed](missing-listed.md)\n    # Listed heading\n    ```\n",
        )
        errors: list[str] = []
        CHECKER.check_links(errors)
        self.assertEqual(errors, [])

    def test_fence_on_a_list_continuation_line_is_not_scanned(self) -> None:
        self.write(
            "README.md",
            "10. Item\n\n    ```markdown\n    [listed](missing-listed.md)\n"
            "    # Listed heading\n    ```\n",
        )
        errors: list[str] = []
        CHECKER.check_links(errors)
        self.assertEqual(errors, [])

    def test_reference_links_support_spaces_and_empty_self_targets(self) -> None:
        self.write("target with spaces.md", "# Section one\n")
        self.write(
            "README.md",
            "[full][target]\n[collapsed][]\n[self][]\n\n"
            "[target]: <target with spaces.md#section-one>\n"
            "[collapsed]: <target with spaces.md>\n"
            "[self]: <>\n",
        )
        errors: list[str] = []
        CHECKER.check_links(errors)
        self.assertEqual(errors, [])

    def test_inline_code_link_syntax_is_not_scanned(self) -> None:
        self.write(
            "README.md",
            "Use `[example](not-a-file.md)` to demonstrate inline link syntax.\n",
        )
        errors: list[str] = []
        CHECKER.check_links(errors)
        self.assertEqual(errors, [])

    def test_escaped_backticks_do_not_hide_a_real_link(self) -> None:
        self.write(
            "README.md",
            "\\` [real](missing.md) \\`\n",
        )
        errors: list[str] = []
        CHECKER.check_links(errors)
        self.assertEqual(
            errors,
            ["plans/v2/README.md: missing link target: missing.md"],
        )

    def test_inline_link_destination_supports_balanced_parentheses(self) -> None:
        self.write("target_(v2).md", "# Target\n")
        self.write("README.md", "[target](target_(v2).md)\n")
        errors: list[str] = []
        CHECKER.check_links(errors)
        self.assertEqual(errors, [])

    def test_width_check_ignores_fenced_code_inside_a_blockquote(self) -> None:
        self.write(
            "README.md",
            "> ```text\n> " + ("x" * 140) + "\n> ```\n",
        )
        errors: list[str] = []
        CHECKER.check_active_document_width(errors)
        self.assertEqual(errors, [])

    def test_missing_control_plane_files_are_diagnostics_not_exceptions(self) -> None:
        errors: list[str] = []
        CHECKER.check_control_plane(errors)
        self.assertEqual(len(errors), 7)
        self.assertIn("plans/v2/ADR.md: required control-plane file is missing", errors)
        self.assertIn(
            "plans/v2/WORKING-AGREEMENT.md: required control-plane file is missing",
            errors,
        )

        CHECKER.check_decision_index(errors)
        self.assertEqual(len(errors), 7)

    def test_record_and_register_status_must_match(self) -> None:
        self.write(
            "ADR.md",
            "| ID | Topic | Status | Phase | Record | Note |\n"
            "|---|---|---|---|---|---|\n"
            "| ADR-0001 | Example | Proposed | 1 | [ADR](decisions/ADR-0001-example.md) | — |\n",
        )
        self.write(
            "decisions/ADR-0001-example.md",
            "# ADR-0001: Example\n\n| Field | Value |\n|---|---|\n| ID | ADR-0001 |\n"
            "| Status | Accepted |\n",
        )
        errors: list[str] = []
        CHECKER.check_decision_index(errors)
        self.assertIn(
            "plans/v2/ADR.md: ADR-0001 status Proposed does not match record status Accepted",
            errors,
        )

    def test_active_document_set_covers_all_specs_glossary_and_readmes(self) -> None:
        expected = {
            self.write("glossary.md", "# Glossary\n"),
            self.write("specs/spec-first.md", "# First\n"),
            self.write("specs/spec-second.md", "# Second\n"),
            self.write("evidence/README.md", "# Evidence\n"),
            self.write("inventories/README.md", "# Inventories\n"),
            self.write("evidence/nested/README.md", "# Nested evidence\n"),
        }
        self.assertTrue(expected.issubset(set(CHECKER.active_documents())))

    def test_source_line_citations_are_checked_in_the_active_set(self) -> None:
        self.write("NOW.md", "Current behavior is in `state.rs:123`.\n")
        errors: list[str] = []
        CHECKER.check_derived_source_citations(errors)
        self.assertEqual(
            errors,
            ["plans/v2/NOW.md:1: active derived document copies a source-line citation"],
        )


if __name__ == "__main__":
    unittest.main()
