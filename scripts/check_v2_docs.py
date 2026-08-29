#!/usr/bin/env python3
"""Check mechanical invariants of the Core V2 documentation."""

from __future__ import annotations

import hashlib
import re
import subprocess
import sys
from pathlib import Path
from urllib.parse import unquote


REPO_ROOT = Path(__file__).resolve().parents[1]
V2_ROOT = REPO_ROOT / "plans" / "v2"
ACTIVE_ROOT_DOCUMENTS = (
    "README.md",
    "PROCESS.md",
    "ROADMAP.md",
    "NOW.md",
    "ADR.md",
    "STATUS.md",
    "WORKING-AGREEMENT.md",
    "glossary.md",
)
ACTIVE_DOCUMENT_DIRECTORIES = ("architecture", "specs", "templates")
MARKDOWN_LINK_START = re.compile(r"(?<!!)\[[^\]\n]*\]\(")
MARKDOWN_REFERENCE_LINK = re.compile(r"(?<!!)\[([^\]]+)\]\[([^\]]*)\]")
MARKDOWN_REFERENCE_DEFINITION = re.compile(
    r"^ {0,3}\[([^\]]+)\]:[ \t]*(<[^>\n]*>|\S+)(?:[ \t]+.*)?$", re.MULTILINE
)
MARKDOWN_HEADING = re.compile(r"^ {0,3}#{1,6}\s+(.+?)\s*#*\s*$", re.MULTILINE)
FENCE_START = re.compile(r"^ {0,3}(`{3,}|~{3,})")
BLOCKQUOTE_PREFIX = re.compile(r"^ {0,3}>[ \t]?")
LIST_PREFIX = re.compile(r"^ {0,3}(?:[-+*]|\d{1,9}[.)])[ \t]+")
LITERAL_LINK_LINE = re.compile(
    r"^(?:https?://\S+|mailto:\S+|<https?://[^>]+>|\[[^]]+]\([^)]*\)|\[[^]]+]:\s+\S+(?:\s+.+)?)$"
)
ADR_FILE = re.compile(r"ADR-(\d{4})-[^/]+\.md$")
EVD_FILE = re.compile(r"EVD-(\d{4})-[^/]+\.md$")
REGISTER_ROW = re.compile(r"^\| (ADR-\d{4}) \|", re.MULTILINE)
STATUS_ROW = re.compile(
    r"^\| ADR-\d{4} \|.*?\| (Accepted|Deferred|Proposed|Rejected|Superseded) \|",
    re.MULTILINE,
)
ADR_STATUS_FIELD = re.compile(
    r"^\|\s*Status\s*\|\s*(Accepted|Deferred|Proposed|Rejected|Superseded)\b",
    re.MULTILINE,
)
MAX_PROSE_WIDTH = 120
EVIDENCE_SELF_TESTS = (
    Path("plans/v2/evidence/phase-03/evd_0016_analyse.py"),
)
EVD_0016_ANALYZER_SUCCESS = re.compile(
    r"^EVD-0016 analyzer controls passed \(valid single-direction \+ duplex "
    r"positive \+ 19 classified mutations \+ "
    r"2 F4 negative outcomes \+ RealtimeDenied warning \+ release coverage \+ "
    r"15 endpoint cases\)\.$",
    re.MULTILINE,
)
EVD_0016_SIMULATOR = Path(
    "crates/synth_engine_v2/examples/evd_0016_host_time.rs"
)
EVD_0016_RECORD = Path("plans/v2/evidence/phase-03/EVD-0016-host-time-mapping.md")
EVD_0016_SIMULATOR_DIGEST = re.compile(
    r"provisional simulator CSV.*?SHA-256\s+`([0-9a-f]{64})`", re.DOTALL
)
CPAL_DEPENDENCY_REQUIREMENT = re.compile(
    r'^cpal\s*=\s*\{[^\n]*version\s*=\s*"([^"]+)"', re.MULTILINE
)
CPAL_LOCK_VERSION = re.compile(
    r'^\[\[package\]\]\nname = "cpal"\nversion = "([^"]+)"', re.MULTILINE
)
CPAL_PROBE_VERSION = re.compile(
    r'^const CPAL_VERSION: &str = "([^"]+)";$', re.MULTILINE
)
CPAL_ANALYZER_VERSION = re.compile(r'^CPAL_VERSION = "([^"]+)"$', re.MULTILINE)
CPAL_MEMBER_WORKSPACE_DEPENDENCY = re.compile(
    r'^cpal\s*=\s*\{\s*workspace\s*=\s*true\s*\}\s*$', re.MULTILINE
)


def local_link_target(source: Path, raw_target: str) -> tuple[Path, str] | None:
    target = raw_target.strip()
    if target.startswith("<") and target.endswith(">"):
        target = target[1:-1]
    elif target:
        target = target.split(maxsplit=1)[0]
    if target.startswith(("http://", "https://", "mailto:")):
        return None
    path_text, _, fragment = target.partition("#")
    path_text = unquote(path_text)
    if not path_text:
        return source.resolve(), unquote(fragment).lower()
    return (source.parent / path_text).resolve(), unquote(fragment).lower()


def heading_slug(heading: str) -> str:
    without_markup = re.sub(r"<[^>]+>|[`*_~]", "", heading)
    without_links = re.sub(r"\[([^]]+)]\([^)]+\)", r"\1", without_markup)
    without_reference_links = re.sub(r"\[([^]]+)]\[[^]]*]", r"\1", without_links)
    words_and_hyphens = re.sub(r"[^\w\s-]", "", without_reference_links.lower())
    return re.sub(r"\s", "-", words_and_hyphens.strip())


def markdown_without_fenced_code(text: str) -> str:
    visible: list[str] = []
    fence_character = ""
    fence_length = 0
    active_list_indent = 0
    fence_list_indent = 0
    for line in text.splitlines(keepends=True):
        content = line
        while blockquote := BLOCKQUOTE_PREFIX.match(content):
            content = content[blockquote.end() :]

        if fence_character:
            if fence_list_indent:
                leading_spaces = len(content) - len(content.lstrip(" "))
                content = content[min(leading_spaces, fence_list_indent) :]
            closing = re.match(
                rf"^ {{0,3}}{re.escape(fence_character)}{{{fence_length},}}[ \t]*(?:\r?\n)?$",
                content,
            )
            if closing:
                fence_character = ""
                fence_length = 0
                fence_list_indent = 0
            visible.append("\n" if line.endswith("\n") else "")
            continue

        container_indent = active_list_indent
        leading_spaces = len(content) - len(content.lstrip(" "))
        if active_list_indent and leading_spaces >= active_list_indent:
            content = content[active_list_indent:]
        elif content.strip():
            active_list_indent = 0
            container_indent = 0

        while list_item := LIST_PREFIX.match(content):
            container_indent += list_item.end()
            active_list_indent = container_indent
            content = content[list_item.end() :]
            while blockquote := BLOCKQUOTE_PREFIX.match(content):
                content = content[blockquote.end() :]

        opening = FENCE_START.match(content)
        if opening:
            fence = opening.group(1)
            fence_character = fence[0]
            fence_length = len(fence)
            fence_list_indent = container_indent
            visible.append("\n" if line.endswith("\n") else "")
            continue

        if content.startswith(("    ", "\t")):
            visible.append("\n" if line.endswith("\n") else "")
            continue
        visible.append(line)
    return "".join(visible)


def markdown_without_inline_code(text: str) -> str:
    masked = list(text)
    index = 0
    while index < len(text):
        if text[index] != "`":
            index += 1
            continue

        preceding_backslashes = 0
        cursor = index - 1
        while cursor >= 0 and text[cursor] == "\\":
            preceding_backslashes += 1
            cursor -= 1
        if preceding_backslashes % 2 == 1:
            index += 1
            continue

        opening_end = index
        while opening_end < len(text) and text[opening_end] == "`":
            opening_end += 1
        delimiter_length = opening_end - index
        search = opening_end
        closing_end = 0
        while search < len(text):
            closing_start = text.find("`", search)
            if closing_start < 0:
                break
            closing_end = closing_start
            while closing_end < len(text) and text[closing_end] == "`":
                closing_end += 1
            if closing_end - closing_start == delimiter_length:
                for position in range(index, closing_end):
                    if masked[position] not in "\r\n":
                        masked[position] = " "
                index = closing_end
                break
            search = closing_end
        else:
            closing_end = 0

        if closing_end == 0:
            index = opening_end
    return "".join(masked)


def markdown_anchors(text: str) -> set[str]:
    text = markdown_without_fenced_code(text)
    anchors: set[str] = set()
    generated: set[str] = set()
    for match in MARKDOWN_HEADING.finditer(text):
        base = heading_slug(match.group(1))
        candidate = base
        suffix = 0
        while candidate in generated:
            suffix += 1
            candidate = f"{base}-{suffix}"
        generated.add(candidate)
    anchors.update(generated)
    anchors.update(re.findall(r"<(?:a|span)\s+(?:id|name)=[\"']([^\"']+)[\"']", text, re.IGNORECASE))
    return {anchor.lower() for anchor in anchors}


def reference_label(label: str) -> str:
    return " ".join(label.split()).casefold()


def inline_markdown_link_targets(text: str) -> list[str]:
    """Return inline-link destinations, including balanced parentheses."""
    targets: list[str] = []
    search_from = 0
    while match := MARKDOWN_LINK_START.search(text, search_from):
        destination_start = match.end()
        depth = 1
        index = destination_start
        while index < len(text):
            character = text[index]
            if character == "\\":
                index += 2
                continue
            if character == "(":
                depth += 1
            elif character == ")":
                depth -= 1
                if depth == 0:
                    targets.append(text[destination_start:index])
                    index += 1
                    break
            index += 1
        search_from = max(index, match.end())
    return targets


def check_link_target(source: Path, raw_target: str, errors: list[str]) -> None:
    target_parts = local_link_target(source, raw_target)
    if target_parts is None:
        return
    target, fragment = target_parts
    try:
        target.relative_to(REPO_ROOT)
    except ValueError:
        errors.append(f"{source.relative_to(REPO_ROOT)}: link escapes repository: {raw_target}")
        return
    if not target.exists():
        errors.append(f"{source.relative_to(REPO_ROOT)}: missing link target: {raw_target}")
        return
    if fragment and target.is_file() and target.suffix == ".md":
        try:
            target_text = target.read_text(encoding="utf-8")
        except OSError as error:
            errors.append(
                f"{source.relative_to(REPO_ROOT)}: cannot read link target {raw_target}: {error}"
            )
            return
        if fragment not in markdown_anchors(target_text):
            errors.append(f"{source.relative_to(REPO_ROOT)}: missing link fragment: {raw_target}")


def check_links(errors: list[str]) -> None:
    for source in sorted(V2_ROOT.rglob("*.md")):
        try:
            text = source.read_text(encoding="utf-8")
        except OSError as error:
            errors.append(f"{source.relative_to(REPO_ROOT)}: cannot read document: {error}")
            continue
        text = markdown_without_fenced_code(text)
        link_text = markdown_without_inline_code(text)
        for target in inline_markdown_link_targets(link_text):
            check_link_target(source, target, errors)

        definitions: dict[str, str] = {}
        for match in MARKDOWN_REFERENCE_DEFINITION.finditer(link_text):
            label = reference_label(match.group(1))
            if label in definitions:
                errors.append(
                    f"{source.relative_to(REPO_ROOT)}: duplicate reference-link definition: "
                    f"{match.group(1)}"
                )
                continue
            definitions[label] = match.group(2)
            check_link_target(source, match.group(2), errors)

        for match in MARKDOWN_REFERENCE_LINK.finditer(link_text):
            label = reference_label(match.group(2) or match.group(1))
            if label not in definitions:
                errors.append(
                    f"{source.relative_to(REPO_ROOT)}: undefined reference link: {match.group(0)}"
                )


def unique_record_ids(pattern: re.Pattern[str], root: Path, label: str, errors: list[str]) -> set[str]:
    found: dict[str, Path] = {}
    for path in sorted(root.rglob("*.md")):
        match = pattern.search(path.name)
        if match is None:
            continue
        record_id = f"{label}-{match.group(1)}"
        previous = found.get(record_id)
        if previous is not None:
            errors.append(
                f"duplicate {record_id}: {previous.relative_to(REPO_ROOT)} and {path.relative_to(REPO_ROOT)}"
            )
        found[record_id] = path
    return set(found)


def check_decision_index(errors: list[str]) -> None:
    decision_ids = unique_record_ids(ADR_FILE, V2_ROOT / "decisions", "ADR", errors)
    index_path = V2_ROOT / "ADR.md"
    if not index_path.is_file():
        return
    try:
        index_text = index_path.read_text(encoding="utf-8")
    except OSError as error:
        errors.append(f"plans/v2/ADR.md: cannot read decision index: {error}")
        return
    rows = REGISTER_ROW.findall(index_text)
    if len(rows) != len(set(rows)):
        errors.append("plans/v2/ADR.md: duplicate decision-index row")
    row_ids = set(rows)
    missing = sorted(decision_ids - row_ids)
    if missing:
        errors.append(f"plans/v2/ADR.md: records missing from index: {', '.join(missing)}")
    if len(STATUS_ROW.findall(index_text)) != len(rows):
        errors.append("plans/v2/ADR.md: an index row has an unrecognized status")

    register_statuses: dict[str, str] = {}
    for line in index_text.splitlines():
        cells = [cell.strip() for cell in line.strip().strip("|").split("|")]
        if len(cells) >= 3 and re.fullmatch(r"ADR-\d{4}", cells[0]):
            register_statuses[cells[0]] = cells[2]

    for path in sorted((V2_ROOT / "decisions").glob("ADR-*.md")):
        match = ADR_FILE.fullmatch(path.name)
        if match is None:
            continue
        record_id = f"ADR-{match.group(1)}"
        try:
            record_text = path.read_text(encoding="utf-8")
        except OSError as error:
            errors.append(f"{path.relative_to(REPO_ROOT)}: cannot read decision record: {error}")
            continue
        status_match = ADR_STATUS_FIELD.search(record_text)
        if status_match is None:
            errors.append(f"{path.relative_to(REPO_ROOT)}: missing or unrecognized Status metadata")
            continue
        register_status = register_statuses.get(record_id)
        if register_status is not None and register_status != status_match.group(1):
            errors.append(
                f"plans/v2/ADR.md: {record_id} status {register_status} does not match "
                f"record status {status_match.group(1)}"
            )


def check_evidence_ids(errors: list[str]) -> None:
    unique_record_ids(EVD_FILE, V2_ROOT / "evidence", "EVD", errors)


def check_evidence_harnesses(errors: list[str]) -> None:
    for relative_path in EVIDENCE_SELF_TESTS:
        analyzer = REPO_ROOT / relative_path
        if not analyzer.is_file():
            errors.append(f"{relative_path}: required evidence self-test is missing")
            continue
        completed = subprocess.run(
            [sys.executable, "-B", str(analyzer), "--self-test"],
            cwd=REPO_ROOT,
            check=False,
            capture_output=True,
            text=True,
        )
        if completed.returncode != 0:
            detail = completed.stderr.strip() or completed.stdout.strip() or "no diagnostic"
            errors.append(f"{relative_path}: self-test failed: {detail}")
        elif EVD_0016_ANALYZER_SUCCESS.search(completed.stdout) is None:
            errors.append(f"{relative_path}: self-test omitted its control summary")


def check_evidence_simulators(errors: list[str]) -> None:
    simulator = REPO_ROOT / EVD_0016_SIMULATOR
    if not simulator.is_file():
        errors.append(f"{EVD_0016_SIMULATOR}: required evidence simulator is missing")
        return
    try:
        completed = subprocess.run(
            [
                "cargo",
                "run",
                "--quiet",
                "-p",
                "synth_engine_v2",
                "--example",
                "evd_0016_host_time",
            ],
            cwd=REPO_ROOT,
            check=False,
            capture_output=True,
        )
    except OSError as error:
        errors.append(f"{EVD_0016_SIMULATOR}: cannot execute control: {error}")
        return
    if completed.returncode != 0:
        detail_bytes = completed.stderr.strip() or completed.stdout.strip()
        detail = detail_bytes.decode(errors="replace") if detail_bytes else "no diagnostic"
        errors.append(f"{EVD_0016_SIMULATOR}: control run failed: {detail}")
        return
    record = REPO_ROOT / EVD_0016_RECORD
    try:
        record_text = record.read_text(encoding="utf-8")
    except OSError as error:
        errors.append(f"{EVD_0016_RECORD}: cannot read simulator digest: {error}")
        return
    digests = EVD_0016_SIMULATOR_DIGEST.findall(record_text)
    if len(digests) != 1:
        errors.append(f"{EVD_0016_RECORD}: expected one simulator SHA-256")
        return
    observed = hashlib.sha256(completed.stdout).hexdigest()
    if observed != digests[0]:
        errors.append(
            f"{EVD_0016_RECORD}: simulator SHA-256 is stale "
            f"(recorded {digests[0]}, observed {observed})"
        )


def check_evidence_dependency_pins(errors: list[str]) -> None:
    member_manifest = REPO_ROOT / "crates/pertylizer/Cargo.toml"
    try:
        member_text = member_manifest.read_text(encoding="utf-8")
    except OSError as error:
        errors.append(
            f"{member_manifest.relative_to(REPO_ROOT)}: cannot read CPAL workspace dependency: {error}"
        )
    else:
        matches = CPAL_MEMBER_WORKSPACE_DEPENDENCY.findall(member_text)
        if len(matches) != 1:
            errors.append(
                "crates/pertylizer/Cargo.toml: expected one CPAL workspace dependency"
            )
    declarations = (
        (
            REPO_ROOT / "Cargo.toml",
            CPAL_DEPENDENCY_REQUIREMENT,
            "workspace CPAL dependency requirement",
        ),
        (REPO_ROOT / "Cargo.lock", CPAL_LOCK_VERSION, "resolved CPAL dependency"),
        (
            REPO_ROOT / "crates/pertylizer/examples/evd_0016_cpal_timestamps.rs",
            CPAL_PROBE_VERSION,
            "EVD-0016 probe CPAL version",
        ),
        (
            V2_ROOT / "evidence/phase-03/evd_0016_analyse.py",
            CPAL_ANALYZER_VERSION,
            "EVD-0016 analyzer CPAL version",
        ),
    )
    versions: list[tuple[str, str]] = []
    for path, pattern, label in declarations:
        try:
            text = path.read_text(encoding="utf-8")
        except OSError as error:
            errors.append(f"{path.relative_to(REPO_ROOT)}: cannot read {label}: {error}")
            continue
        matches = pattern.findall(text)
        if len(matches) != 1:
            errors.append(
                f"{path.relative_to(REPO_ROOT)}: expected one {label} declaration"
            )
            continue
        versions.append((label, matches[0]))
    observed = {version for _, version in versions}
    if len(observed) > 1:
        detail = ", ".join(f"{label}={version}" for label, version in versions)
        errors.append(f"EVD-0016 CPAL version declarations disagree: {detail}")


def check_state_ownership_coverage(errors: list[str]) -> None:
    """Run EVD-0018's coverage check as part of the gate.

    The ledger's exactly-once claim is only enforced if something runs it, so the
    checker and its own mutation tests belong here rather than beside the record
    that introduced them.
    """
    for command, label in (
        (["scripts/check_state_ownership_coverage.py"], "state-ownership coverage"),
        (["-m", "unittest", "scripts/test_check_state_ownership_coverage.py"], "its mutation tests"),
    ):
        completed = subprocess.run(
            [sys.executable, "-B", *command],
            cwd=REPO_ROOT,
            check=False,
            capture_output=True,
            text=True,
        )
        if completed.returncode != 0:
            detail = completed.stderr.strip() or completed.stdout.strip() or "no diagnostic"
            errors.append(f"{label} failed: {detail}")


def check_spec_prefixes(errors: list[str]) -> None:
    prefix_pattern = re.compile(
        r"^\| Invariant prefix\s*\|\s*`?([A-Z][A-Z0-9_]*)`?\s*\|$",
        re.MULTILINE,
    )
    seen: dict[str, Path] = {}
    for path in sorted((V2_ROOT / "specs").glob("spec-*.md")):
        match = prefix_pattern.search(path.read_text(encoding="utf-8"))
        if match is None:
            errors.append(f"{path.relative_to(REPO_ROOT)}: missing invariant prefix metadata")
            continue
        prefix = match.group(1)
        previous = seen.get(prefix)
        if previous is not None:
            errors.append(
                f"duplicate specification prefix {prefix}: "
                f"{previous.relative_to(REPO_ROOT)} and {path.relative_to(REPO_ROOT)}"
            )
        seen[prefix] = path


def check_control_plane(errors: list[str]) -> None:
    required = [
        "README.md",
        "PROCESS.md",
        "ROADMAP.md",
        "NOW.md",
        "ADR.md",
        "STATUS.md",
        "WORKING-AGREEMENT.md",
    ]
    for name in required:
        if not (V2_ROOT / name).is_file():
            errors.append(f"plans/v2/{name}: required control-plane file is missing")

    redirects = {
        "STATUS.md": "NOW.md",
        "WORKING-AGREEMENT.md": "PROCESS.md",
    }
    for source, target in redirects.items():
        path = V2_ROOT / source
        if not path.is_file():
            continue
        try:
            text = path.read_text(encoding="utf-8")
        except OSError as error:
            errors.append(f"plans/v2/{source}: cannot read stable-path pointer: {error}")
            continue
        if target not in text or "superseded" not in text.lower():
            errors.append(f"plans/v2/{source}: must be a superseded pointer to {target}")


def active_documents() -> list[Path]:
    documents = {V2_ROOT / name for name in ACTIVE_ROOT_DOCUMENTS}
    documents.update(V2_ROOT.rglob("README.md"))
    for directory in ACTIVE_DOCUMENT_DIRECTORIES:
        documents.update((V2_ROOT / directory).rglob("*.md"))
    return sorted(documents)


def check_active_document_width(errors: list[str]) -> None:
    style_files = active_documents()

    for path in sorted(style_files):
        if not path.is_file():
            continue
        try:
            text = path.read_text(encoding="utf-8")
        except OSError as error:
            errors.append(f"{path.relative_to(REPO_ROOT)}: cannot read document: {error}")
            continue
        visible_text = markdown_without_fenced_code(text)
        for line_number, line in enumerate(visible_text.splitlines(), start=1):
            stripped = line.lstrip()
            if (
                stripped.startswith("|")
                or LITERAL_LINK_LINE.fullmatch(stripped)
                or len(line) <= MAX_PROSE_WIDTH
            ):
                continue
            errors.append(
                f"{path.relative_to(REPO_ROOT)}:{line_number}: active prose exceeds "
                f"{MAX_PROSE_WIDTH} characters"
            )


def check_derived_source_citations(errors: list[str]) -> None:
    citation = re.compile(r"\.rs(?::|#L)\d+")
    for path in active_documents():
        if not path.is_file():
            continue
        try:
            text = path.read_text(encoding="utf-8")
        except OSError as error:
            errors.append(f"{path.relative_to(REPO_ROOT)}: cannot read document: {error}")
            continue
        for line_number, line in enumerate(text.splitlines(), start=1):
            if citation.search(line):
                errors.append(
                    f"{path.relative_to(REPO_ROOT)}:{line_number}: active derived document "
                    "copies a source-line citation"
                )


def main() -> int:
    errors: list[str] = []
    check_control_plane(errors)
    check_links(errors)
    check_decision_index(errors)
    check_evidence_ids(errors)
    check_evidence_harnesses(errors)
    check_evidence_simulators(errors)
    check_evidence_dependency_pins(errors)
    check_spec_prefixes(errors)
    check_state_ownership_coverage(errors)
    check_active_document_width(errors)
    check_derived_source_citations(errors)
    if errors:
        for error in errors:
            print(error, file=sys.stderr)
        return 1
    print("Core V2 documentation checks passed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
