#!/usr/bin/env python3
"""Regression tests for the Core V2 documentation checker."""

from __future__ import annotations

import importlib.util
import hashlib
import subprocess
import tempfile
import unittest
from contextlib import ExitStack
from pathlib import Path
from unittest import mock


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

    def write_pertylizer_manifest(self, dependency: str = "cpal = { workspace = true }") -> Path:
        manifest = self.root / "crates/pertylizer/Cargo.toml"
        manifest.parent.mkdir(parents=True, exist_ok=True)
        manifest.write_text(f"{dependency}\n", encoding="utf-8")
        (self.root / "Cargo.lock").write_text(
            '[[package]]\nname = "cpal"\nversion = "0.18.2"\n',
            encoding="utf-8",
        )
        return manifest

    def test_evidence_simulator_is_opt_in(self) -> None:
        self.assertFalse(CHECKER.parse_args([]).evidence)
        self.assertTrue(CHECKER.parse_args(["--evidence"]).evidence)
        ordinary_checks = (
            "check_control_plane",
            "check_links",
            "check_decision_index",
            "check_evidence_ids",
            "check_evidence_harnesses",
            "check_evidence_dependency_pins",
            "check_spec_prefixes",
            "check_state_ownership_coverage",
            "check_active_document_width",
            "check_derived_source_citations",
        )
        with ExitStack() as stack:
            for check in ordinary_checks:
                stack.enter_context(mock.patch.object(CHECKER, check))
            simulator = stack.enter_context(
                mock.patch.object(CHECKER, "check_evidence_simulators")
            )
            stack.enter_context(mock.patch("builtins.print"))
            self.assertEqual(CHECKER.main([]), 0)
            simulator.assert_not_called()
            self.assertEqual(CHECKER.main(["--evidence"]), 0)
            simulator.assert_called_once()

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

    def test_evidence_harness_self_test_failure_is_reported(self) -> None:
        analyzer = self.write(
            "evidence/phase-03/evd_0016_analyse.py",
            "import sys\nprint('negative control failed', file=sys.stderr)\n"
            "raise SystemExit(1)\n",
        )
        errors: list[str] = []
        CHECKER.check_evidence_harnesses(errors)
        self.assertEqual(
            errors,
            [
                f"{analyzer.relative_to(self.root)}: self-test failed: "
                "negative control failed"
            ],
        )

    def test_evidence_harness_self_test_success_is_accepted(self) -> None:
        summary = (
            "EVD-0016 analyzer controls passed (valid single-direction + duplex "
            "positive + 19 classified mutations + "
            "2 F4 negative outcomes + RealtimeDenied warning + release coverage + "
            "15 endpoint cases)."
        )
        self.write(
            "evidence/phase-03/evd_0016_analyse.py",
            f"print({summary!r})\n",
        )
        errors: list[str] = []
        CHECKER.check_evidence_harnesses(errors)
        self.assertEqual(errors, [])

    def test_missing_evidence_harness_does_not_pass_vacuously(self) -> None:
        errors: list[str] = []
        CHECKER.check_evidence_harnesses(errors)
        self.assertEqual(
            errors,
            [
                "plans/v2/evidence/phase-03/evd_0016_analyse.py: "
                "required evidence self-test is missing"
            ],
        )

    def test_zero_exit_without_control_summary_is_rejected(self) -> None:
        self.write(
            "evidence/phase-03/evd_0016_analyse.py",
            "print('not the declared controls')\n",
        )
        errors: list[str] = []
        CHECKER.check_evidence_harnesses(errors)
        self.assertEqual(
            errors,
            [
                "plans/v2/evidence/phase-03/evd_0016_analyse.py: "
                "self-test omitted its control summary"
            ],
        )

    def test_evidence_simulator_control_success_is_accepted(self) -> None:
        simulator = self.root / CHECKER.EVD_0016_SIMULATOR
        simulator.parent.mkdir(parents=True)
        simulator.write_text("fn main() {}\n", encoding="utf-8")
        output = "matrix output\n"
        digest = hashlib.sha256(output.encode()).hexdigest()
        self.write(
            "evidence/phase-03/EVD-0016-host-time-mapping.md",
            f"The provisional simulator CSV has SHA-256\n`{digest}`.\n",
        )
        completed = subprocess.CompletedProcess(
            [], 0, stdout=output.encode(), stderr=b""
        )
        with mock.patch.object(CHECKER.subprocess, "run", return_value=completed):
            errors: list[str] = []
            CHECKER.check_evidence_simulators(errors)
        self.assertEqual(errors, [])

    def test_evidence_simulator_control_failure_is_reported(self) -> None:
        simulator = self.root / CHECKER.EVD_0016_SIMULATOR
        simulator.parent.mkdir(parents=True)
        simulator.write_text("fn main() {}\n", encoding="utf-8")
        completed = subprocess.CompletedProcess(
            [], 1, stdout=b"", stderr=b"F1 failed"
        )
        with mock.patch.object(CHECKER.subprocess, "run", return_value=completed):
            errors: list[str] = []
            CHECKER.check_evidence_simulators(errors)
        self.assertEqual(
            errors,
            [f"{CHECKER.EVD_0016_SIMULATOR}: control run failed: F1 failed"],
        )

    def test_missing_evidence_simulator_does_not_pass_vacuously(self) -> None:
        errors: list[str] = []
        CHECKER.check_evidence_simulators(errors)
        self.assertEqual(
            errors,
            [f"{CHECKER.EVD_0016_SIMULATOR}: required evidence simulator is missing"],
        )

    def test_missing_cargo_is_reported_as_a_simulator_diagnostic(self) -> None:
        simulator = self.root / CHECKER.EVD_0016_SIMULATOR
        simulator.parent.mkdir(parents=True)
        simulator.write_text("fn main() {}\n", encoding="utf-8")
        with mock.patch.object(
            CHECKER.subprocess,
            "run",
            side_effect=FileNotFoundError("cargo not found"),
        ):
            errors: list[str] = []
            CHECKER.check_evidence_simulators(errors)
        self.assertEqual(
            errors,
            [
                f"{CHECKER.EVD_0016_SIMULATOR}: cannot execute control: "
                "cargo not found"
            ],
        )

    def test_stale_evidence_simulator_digest_is_reported(self) -> None:
        simulator = self.root / CHECKER.EVD_0016_SIMULATOR
        simulator.parent.mkdir(parents=True)
        simulator.write_text("fn main() {}\n", encoding="utf-8")
        stale = "0" * 64
        self.write(
            "evidence/phase-03/EVD-0016-host-time-mapping.md",
            f"The provisional simulator CSV has SHA-256\n`{stale}`.\n",
        )
        completed = subprocess.CompletedProcess(
            [], 0, stdout=b"matrix output\n", stderr=b""
        )
        observed = hashlib.sha256(completed.stdout).hexdigest()
        with mock.patch.object(CHECKER.subprocess, "run", return_value=completed):
            errors: list[str] = []
            CHECKER.check_evidence_simulators(errors)
        self.assertEqual(
            errors,
            [
                f"{CHECKER.EVD_0016_RECORD}: simulator SHA-256 is stale "
                f"(recorded {stale}, observed {observed})"
            ],
        )

    def test_missing_simulator_record_is_reported(self) -> None:
        simulator = self.root / CHECKER.EVD_0016_SIMULATOR
        simulator.parent.mkdir(parents=True)
        simulator.write_text("fn main() {}\n", encoding="utf-8")
        completed = subprocess.CompletedProcess([], 0, stdout=b"matrix\n", stderr=b"")
        with mock.patch.object(CHECKER.subprocess, "run", return_value=completed):
            errors: list[str] = []
            CHECKER.check_evidence_simulators(errors)
        self.assertEqual(len(errors), 1)
        self.assertIn("cannot read simulator digest", errors[0])

    def test_missing_simulator_digest_is_reported(self) -> None:
        simulator = self.root / CHECKER.EVD_0016_SIMULATOR
        simulator.parent.mkdir(parents=True)
        simulator.write_text("fn main() {}\n", encoding="utf-8")
        self.write(
            "evidence/phase-03/EVD-0016-host-time-mapping.md",
            "No simulator digest here.\n",
        )
        completed = subprocess.CompletedProcess([], 0, stdout=b"matrix\n", stderr=b"")
        with mock.patch.object(CHECKER.subprocess, "run", return_value=completed):
            errors: list[str] = []
            CHECKER.check_evidence_simulators(errors)
        self.assertEqual(
            errors,
            [f"{CHECKER.EVD_0016_RECORD}: expected one simulator SHA-256"],
        )

    def test_evidence_cpal_version_must_match_requirement_and_lock(self) -> None:
        self.write_pertylizer_manifest()
        (self.root / "Cargo.toml").write_text(
            'cpal = { version = "0.18.3", features = ["realtime"] }\n',
            encoding="utf-8",
        )
        probe = self.root / "crates/pertylizer/examples/evd_0016_cpal_timestamps.rs"
        probe.parent.mkdir(parents=True)
        probe.write_text(
            'const CPAL_VERSION: &str = "0.18.2";\n', encoding="utf-8"
        )
        self.write(
            "evidence/phase-03/evd_0016_analyse.py",
            'CPAL_VERSION = "0.18.2"\n',
        )
        errors: list[str] = []
        CHECKER.check_evidence_dependency_pins(errors)
        self.assertEqual(
            errors,
            [
                "EVD-0016 CPAL version declarations disagree: "
                "workspace CPAL dependency requirement=0.18.3, "
                "resolved CPAL dependency=0.18.2, "
                "EVD-0016 probe CPAL version=0.18.2, "
                "EVD-0016 analyzer CPAL version=0.18.2"
            ],
        )

    def test_matching_evidence_cpal_versions_are_accepted(self) -> None:
        self.write_pertylizer_manifest()
        (self.root / "Cargo.toml").write_text(
            'cpal = { version = "0.18.2", features = ["realtime"] }\n',
            encoding="utf-8",
        )
        probe = self.root / "crates/pertylizer/examples/evd_0016_cpal_timestamps.rs"
        probe.parent.mkdir(parents=True)
        probe.write_text(
            'const CPAL_VERSION: &str = "0.18.2";\n', encoding="utf-8"
        )
        self.write(
            "evidence/phase-03/evd_0016_analyse.py",
            'CPAL_VERSION = "0.18.2"\n',
        )
        errors: list[str] = []
        CHECKER.check_evidence_dependency_pins(errors)
        self.assertEqual(errors, [])

    def test_resolved_cpal_update_requires_new_evidence_versions(self) -> None:
        self.write_pertylizer_manifest()
        (self.root / "Cargo.toml").write_text(
            'cpal = { version = "0.18.2", features = ["realtime"] }\n',
            encoding="utf-8",
        )
        (self.root / "Cargo.lock").write_text(
            '[[package]]\nname = "cpal"\nversion = "0.18.3"\n',
            encoding="utf-8",
        )
        probe = self.root / "crates/pertylizer/examples/evd_0016_cpal_timestamps.rs"
        probe.parent.mkdir(parents=True)
        probe.write_text(
            'const CPAL_VERSION: &str = "0.18.2";\n', encoding="utf-8"
        )
        self.write(
            "evidence/phase-03/evd_0016_analyse.py",
            'CPAL_VERSION = "0.18.2"\n',
        )
        errors: list[str] = []
        CHECKER.check_evidence_dependency_pins(errors)
        self.assertEqual(
            errors,
            [
                "EVD-0016 CPAL version declarations disagree: "
                "workspace CPAL dependency requirement=0.18.2, "
                "resolved CPAL dependency=0.18.3, "
                "EVD-0016 probe CPAL version=0.18.2, "
                "EVD-0016 analyzer CPAL version=0.18.2"
            ],
        )

    def test_broad_evidence_cpal_requirement_is_rejected(self) -> None:
        self.write_pertylizer_manifest()
        (self.root / "Cargo.toml").write_text(
            'cpal = { version = "0.18", features = ["realtime"] }\n',
            encoding="utf-8",
        )
        probe = self.root / "crates/pertylizer/examples/evd_0016_cpal_timestamps.rs"
        probe.parent.mkdir(parents=True)
        probe.write_text(
            'const CPAL_VERSION: &str = "0.18.2";\n', encoding="utf-8"
        )
        self.write(
            "evidence/phase-03/evd_0016_analyse.py",
            'CPAL_VERSION = "0.18.2"\n',
        )
        errors: list[str] = []
        CHECKER.check_evidence_dependency_pins(errors)
        self.assertEqual(
            errors,
            [
                "EVD-0016 CPAL version declarations disagree: "
                "workspace CPAL dependency requirement=0.18, "
                "resolved CPAL dependency=0.18.2, "
                "EVD-0016 probe CPAL version=0.18.2, "
                "EVD-0016 analyzer CPAL version=0.18.2"
            ],
        )

    def test_version_disagreement_is_reported_when_requirement_is_missing(self) -> None:
        self.write_pertylizer_manifest()
        (self.root / "Cargo.toml").write_text(
            'cpal = "0.18.2"\n',
            encoding="utf-8",
        )
        probe = self.root / "crates/pertylizer/examples/evd_0016_cpal_timestamps.rs"
        probe.parent.mkdir(parents=True, exist_ok=True)
        probe.write_text(
            'const CPAL_VERSION: &str = "0.18.2";\n', encoding="utf-8"
        )
        self.write(
            "evidence/phase-03/evd_0016_analyse.py",
            'CPAL_VERSION = "0.18.3"\n',
        )
        errors: list[str] = []
        CHECKER.check_evidence_dependency_pins(errors)
        self.assertEqual(
            errors,
            [
                "Cargo.toml: expected one workspace CPAL dependency requirement declaration",
                "EVD-0016 CPAL version declarations disagree: "
                "resolved CPAL dependency=0.18.2, "
                "EVD-0016 probe CPAL version=0.18.2, "
                "EVD-0016 analyzer CPAL version=0.18.3",
            ],
        )

    def test_missing_probe_version_file_is_reported(self) -> None:
        self.write_pertylizer_manifest()
        (self.root / "Cargo.toml").write_text(
            'cpal = { version = "0.18.2" }\n', encoding="utf-8"
        )
        self.write(
            "evidence/phase-03/evd_0016_analyse.py",
            'CPAL_VERSION = "0.18.2"\n',
        )
        errors: list[str] = []
        CHECKER.check_evidence_dependency_pins(errors)
        self.assertEqual(len(errors), 1)
        self.assertIn(
            "cannot read EVD-0016 probe CPAL version",
            errors[0],
        )

    def test_member_must_inherit_the_workspace_cpal_pin(self) -> None:
        self.write_pertylizer_manifest('cpal = "0.18.2"')
        (self.root / "Cargo.toml").write_text(
            'cpal = { version = "0.18.2" }\n', encoding="utf-8"
        )
        probe = self.root / "crates/pertylizer/examples/evd_0016_cpal_timestamps.rs"
        probe.parent.mkdir(parents=True, exist_ok=True)
        probe.write_text(
            'const CPAL_VERSION: &str = "0.18.2";\n', encoding="utf-8"
        )
        self.write(
            "evidence/phase-03/evd_0016_analyse.py",
            'CPAL_VERSION = "0.18.2"\n',
        )
        errors: list[str] = []
        CHECKER.check_evidence_dependency_pins(errors)
        self.assertEqual(
            errors,
            [
                "crates/pertylizer/Cargo.toml: expected one CPAL workspace dependency"
            ],
        )

    def test_duplicate_probe_and_missing_analyzer_versions_are_reported(self) -> None:
        self.write_pertylizer_manifest()
        (self.root / "Cargo.toml").write_text(
            'cpal = { version = "0.18.2" }\n', encoding="utf-8"
        )
        probe = self.root / "crates/pertylizer/examples/evd_0016_cpal_timestamps.rs"
        probe.parent.mkdir(parents=True, exist_ok=True)
        probe.write_text(
            'const CPAL_VERSION: &str = "0.18.2";\n' * 2,
            encoding="utf-8",
        )
        self.write(
            "evidence/phase-03/evd_0016_analyse.py",
            "print('no version declaration')\n",
        )
        errors: list[str] = []
        CHECKER.check_evidence_dependency_pins(errors)
        self.assertEqual(
            errors,
            [
                "crates/pertylizer/examples/evd_0016_cpal_timestamps.rs: "
                "expected one EVD-0016 probe CPAL version declaration",
                "plans/v2/evidence/phase-03/evd_0016_analyse.py: "
                "expected one EVD-0016 analyzer CPAL version declaration",
            ],
        )


if __name__ == "__main__":
    unittest.main()
