#!/usr/bin/env python3
"""Collect and aggregate the permanent EVD-0007 baseline."""

from __future__ import annotations

import argparse
import csv
import ctypes
import errno
import hashlib
import json
import math
import os
import statistics
import subprocess
import tempfile
from collections import defaultdict
from pathlib import Path


ROOT = Path(__file__).resolve().parents[4]
MANIFEST = ROOT / "corpus/v2-reference/manifest.json"
PERTYLIZER = ROOT / "target/release/pertylizer"
RENDER_COST = ROOT / "target/release/render_cost"
RENDER_PROFILE = ROOT / "target/release/render_profile"
RATES = (44_100, 48_000, 96_000, 192_000)
CSV_ARTIFACTS = (
    "EVD-0007-determinism.csv",
    "EVD-0007-cost.csv",
    "EVD-0007-timing-memory.csv",
)
ARTIFACT_DIRECTORY = "EVD-0007-artifacts"
AT_FDCWD = -100
RENAME_EXCHANGE = 2
# One round is one interleaved pass over every case at every rate; recorded
# repetitions exclude the one warm-up render each invocation adds.
COST_ROUNDS = 10
COST_REPS = 5
PROFILE_ROUNDS = 5
PROFILE_REPS = 3


def run(command: list[str]) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        command,
        cwd=ROOT,
        check=True,
        text=True,
        capture_output=True,
    )


def load_json(path: Path) -> dict:
    with path.open(encoding="utf-8") as source:
        return json.load(source)


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def render(case: dict, suffix: str, work: Path) -> tuple[Path, dict]:
    output = work / f"{case['id']}-{suffix}.wav"
    receipt = work / f"{case['id']}-{suffix}-receipt.json"
    settings = case["render"]
    run(
        [
            str(PERTYLIZER),
            "render",
            "--input",
            str(MANIFEST.parent / case["input"]),
            "--output",
            str(output),
            "--sample-rate",
            str(settings["sample_rate"]),
            "--bit-depth",
            settings["bit_depth"],
            "--seconds",
            str(settings["seconds"]),
            "--tail-seconds",
            str(settings["tail_seconds"]),
            "--result-json",
            str(receipt),
        ]
    )
    return output, load_json(receipt)


def compare(reference: Path, candidate: Path, name: str, work: Path) -> dict:
    report = work / f"{name}-comparison.json"
    run(
        [
            str(PERTYLIZER),
            "compare",
            "--reference",
            str(reference),
            "--candidate",
            str(candidate),
            "--result-json",
            str(report),
        ]
    )
    return load_json(report)


def require_zero_deltas(report: dict) -> None:
    values = [
        report["level"]["peak_delta_db"],
        report["level"]["rms_delta_db"],
        report["samples"]["max_abs_error"],
        report["timing"]["onset_delta_ms"],
        report["timing"]["envelope_lag_ms"],
        report["pitch"]["delta_cents"],
        report["pitch"]["drift"]["mean_abs_cents"],
        report["pitch"]["drift"]["max_abs_cents"],
        *report["envelope"]["delta_ms"].values(),
        report["envelope"]["max_delta_db"],
        report["envelope"]["max_delta_above_floor_db"],
        report["stereo"]["correlation_delta"],
        report["stereo"]["balance_delta_db"],
        report["spectrum"]["max_abs_delta_db"],
        report["spectrum"]["rms_delta_db"],
        report["loudness"]["delta_lu"],
        *(band["delta_db"] for band in report["spectrum"]["bands"]),
    ]
    if any(value != 0 for value in values):
        raise RuntimeError("same-case comparison contains a non-zero delta")


def collect_determinism(manifest: dict, output_dir: Path, work: Path) -> None:
    cases = {case["id"]: case for case in manifest["cases"]}
    control_a, control_a_receipt = render(cases["CORPUS-0006"], "control", work)
    control_b, control_b_receipt = render(cases["CORPUS-0008"], "control", work)
    control = compare(control_a, control_b, "different-case-control", work)
    if (
        control["files_identical"]
        or control["samples"]["identical"]
        or control["warnings"]
        or control_a_receipt["warnings"]
        or control_b_receipt["warnings"]
    ):
        raise RuntimeError("different-case comparison control did not observe a clean difference")

    rows = []
    for case in manifest["cases"]:
        first, first_receipt = render(case, "a", work)
        second, second_receipt = render(case, "b", work)
        report = compare(first, second, f"{case['id']}-same-case", work)
        if report["warnings"] or first_receipt["warnings"] or second_receipt["warnings"]:
            raise RuntimeError(f"{case['id']} emitted a warning")
        if not report["files_identical"] or not report["samples"]["identical"]:
            raise RuntimeError(f"{case['id']} is not deterministic")
        require_zero_deltas(report)
        # Hashed here rather than read from the render receipt on purpose: this
        # is an independent check of the on-disk file against the manifest, so
        # it cannot inherit a defect in the renderer's own digesting.
        input_path = MANIFEST.parent / case["input"]
        if sha256(input_path) != case["sha256"]:
            raise RuntimeError(f"{case['id']} input digest differs from the manifest")
        if first_receipt["output"]["sha256"] != second_receipt["output"]["sha256"]:
            raise RuntimeError(f"{case['id']} output digests differ")
        rows.append(
            {
                "case": case["id"],
                "input_sha256": case["sha256"],
                "output_sha256": first_receipt["output"]["sha256"],
                "frames": first_receipt["audio"]["frames"],
                "files_identical": "true",
                "samples_identical": "true",
                "warnings": 0,
            }
        )
    write_csv(output_dir / "EVD-0007-determinism.csv", rows)


def collect_jsonl(
    binary: Path,
    case_ids: list[str],
    rounds: int,
    repetitions: int,
    raw_path: Path,
    cpu_list: str | None,
) -> list[dict]:
    rows = []
    with raw_path.open("w", encoding="utf-8") as raw:
        for round_number in range(1, rounds + 1):
            for rate in RATES:
                affinity = ["taskset", "-c", cpu_list] if cpu_list else []
                result = run(
                    affinity
                    + [
                        str(binary),
                        "--warmup",
                        "1",
                        "--reps",
                        str(repetitions),
                        "--sample-rate",
                        str(rate),
                    ]
                )
                if result.stderr.strip():
                    raise RuntimeError(
                        f"{binary.name} emitted diagnostics in round {round_number} "
                        f"at {rate} Hz: {result.stderr.strip()}"
                    )
                for line in result.stdout.splitlines():
                    row = json.loads(line)
                    require_finite_numbers(row, binary.name)
                    row["round"] = round_number
                    row["cpu_list"] = cpu_list or "unbound"
                    raw.write(json.dumps(row, sort_keys=True, separators=(",", ":")) + "\n")
                    rows.append(row)
    require_exact_records(rows, case_ids, rounds, repetitions, binary.name)
    return rows


# The fields each measurement binary must emit as non-null finite numbers,
# keyed by binary name. A constant, not per-call state.
REQUIRED_FIELDS = {
    "render_cost": {
        "block_size",
        "rep",
        "elapsed_ns",
        "rendered_seconds",
        "cost_per_rendered_second",
        "sample_rate",
        "frames",
        "peak",
        "rms",
    },
    "render_profile": {
        "rep",
        "sample_rate",
        "blocks",
        "full_blocks",
        "block_frames",
        "block_budget_ns",
        "block_p50_ns",
        "block_p99_ns",
        "block_max_ns",
        "block_p50_ratio",
        "block_p99_ratio",
        "block_max_ratio",
        "rss_load_delta",
        "rss_prepare_delta",
        "rss_render_delta",
        "rss_bytes",
        "rss_peak_bytes",
    },
}


def require_finite_numbers(row: dict, source: str) -> None:
    for field in REQUIRED_FIELDS[source]:
        value = row.get(field)
        if value is None:
            raise RuntimeError(f"{source} emitted null for {field}")
        if isinstance(value, float) and not math.isfinite(value):
            raise RuntimeError(f"{source} emitted non-finite {field}")


def require_exact_records(
    rows: list[dict],
    case_ids: list[str],
    rounds: int,
    repetitions: int,
    source: str,
) -> None:
    expected = {
        (case, rate, round_number, rep, rep == 0)
        for case in case_ids
        for rate in RATES
        for round_number in range(1, rounds + 1)
        for rep in range(repetitions + 1)
    }
    keys = [
        (
            row["case"],
            row["sample_rate"],
            row["round"],
            row["rep"],
            row["warmup"],
        )
        for row in rows
    ]
    actual = set(keys)
    duplicates = len(keys) - len(actual)
    if duplicates or actual != expected:
        raise RuntimeError(
            f"{source} record set differs: duplicates={duplicates}, "
            f"missing={expected - actual}, extra={actual - expected}"
        )


def recorded_cells(rows: list[dict]) -> dict[tuple[str, int], list[dict]]:
    cells: dict[tuple[str, int], list[dict]] = defaultdict(list)
    for row in rows:
        if not row["warmup"]:
            cells[(row["case"], row["sample_rate"])].append(row)
    return cells


def aggregate_cost(rows: list[dict], case_ids: list[str], output_dir: Path) -> None:
    output = []
    for (case, rate), cell in sorted(recorded_cells(rows).items()):
        require_cell_repetitions(cell, COST_ROUNDS, COST_REPS, case, rate, "CPU")
        values = [row["cost_per_rendered_second"] * 1000 for row in cell]  # s/s -> ms/s
        output.append(
            {
                "case": case,
                "sample_rate": rate,
                "renders": len(cell),
                "rounds": COST_ROUNDS,
                "min_ms_per_s": f"{min(values):.4f}",
                "median_ms_per_s": f"{statistics.median(values):.4f}",
                "mean_ms_per_s": f"{statistics.fmean(values):.4f}",
            }
        )
    require_matrix(output, case_ids)
    write_csv(output_dir / "EVD-0007-cost.csv", output)


def aggregate_profile(rows: list[dict], case_ids: list[str], output_dir: Path) -> None:
    output = []
    mib = 1024 * 1024
    for (case, rate), cell in sorted(recorded_cells(rows).items()):
        require_cell_repetitions(
            cell, PROFILE_ROUNDS, PROFILE_REPS, case, rate, "timing/RSS"
        )
        if len({row["block_frames"] for row in cell}) != 1:
            raise RuntimeError(f"mixed block sizes for {case} at {rate} Hz")
        median = lambda field: statistics.median(row[field] for row in cell)
        mean_mib = lambda field: statistics.fmean(row[field] for row in cell) / mib
        output.append(
            {
                "case": case,
                "sample_rate": rate,
                "renders": len(cell),
                "block_frames": cell[0]["block_frames"],
                "budget_us": f"{median('block_budget_ns') / 1000:.1f}",
                "median_p50_us": f"{median('block_p50_ns') / 1000:.1f}",
                "median_p99_us": f"{median('block_p99_ns') / 1000:.1f}",
                "observed_max_us": f"{max(row['block_max_ns'] for row in cell) / 1000:.1f}",
                "median_p50_pct": f"{median('block_p50_ratio') * 100:.2f}",
                "median_p99_pct": f"{median('block_p99_ratio') * 100:.2f}",
                "observed_max_pct": f"{max(row['block_max_ratio'] for row in cell) * 100:.2f}",
                "mean_rss_load_mib": f"{mean_mib('rss_load_delta'):.2f}",
                "mean_rss_prepare_mib": f"{mean_mib('rss_prepare_delta'):.2f}",
                "mean_rss_render_mib": f"{mean_mib('rss_render_delta'):.2f}",
                "mean_rss_mib": f"{mean_mib('rss_bytes'):.2f}",
                "mean_peak_rss_mib": f"{mean_mib('rss_peak_bytes'):.2f}",
            }
        )
    require_matrix(output, case_ids)
    write_csv(output_dir / "EVD-0007-timing-memory.csv", output)


def require_cell_repetitions(
    cell: list[dict],
    rounds: int,
    repetitions: int,
    case: str,
    rate: int,
    source: str,
) -> None:
    expected = {
        (round_number, rep)
        for round_number in range(1, rounds + 1)
        for rep in range(1, repetitions + 1)
    }
    keys = [(row["round"], row["rep"]) for row in cell]
    actual = set(keys)
    duplicates = len(keys) - len(actual)
    if duplicates or actual != expected:
        raise RuntimeError(
            f"incomplete {source} cell for {case} at {rate} Hz: "
            f"duplicates={duplicates}, missing={expected - actual}, extra={actual - expected}"
        )


def require_matrix(rows: list[dict], case_ids: list[str]) -> None:
    expected = {(case, rate) for case in case_ids for rate in RATES}
    actual = {(row["case"], int(row["sample_rate"])) for row in rows}
    if actual != expected:
        raise RuntimeError(f"measurement matrix differs: missing={expected - actual}, extra={actual - expected}")


def write_csv(path: Path, rows: list[dict]) -> None:
    with path.open("w", encoding="utf-8", newline="") as destination:
        writer = csv.DictWriter(
            destination,
            fieldnames=list(rows[0]),
            lineterminator="\n",
        )
        writer.writeheader()
        writer.writerows(rows)


def exchange_directories(first: Path, second: Path) -> None:
    libc = ctypes.CDLL(None, use_errno=True)
    renameat2 = libc.renameat2
    renameat2.argtypes = (
        ctypes.c_int,
        ctypes.c_char_p,
        ctypes.c_int,
        ctypes.c_char_p,
        ctypes.c_uint,
    )
    renameat2.restype = ctypes.c_int
    result = renameat2(
        AT_FDCWD,
        os.fsencode(first),
        AT_FDCWD,
        os.fsencode(second),
        RENAME_EXCHANGE,
    )
    if result != 0:
        error = ctypes.get_errno()
        raise OSError(error, os.strerror(error), f"{first} <-> {second}")


def publish_csv_set(staging_dir: Path, output_dir: Path) -> None:
    missing = [name for name in CSV_ARTIFACTS if not (staging_dir / name).is_file()]
    if missing:
        raise RuntimeError(f"refusing to publish an incomplete CSV set: {missing}")
    target = output_dir / ARTIFACT_DIRECTORY
    if target.exists():
        if not target.is_dir() or target.is_symlink():
            raise RuntimeError(f"artifact target is not a real directory: {target}")
        try:
            exchange_directories(staging_dir, target)
        except AttributeError as error:
            raise OSError(
                errno.ENOSYS,
                "atomic directory exchange requires Linux renameat2",
                target,
            ) from error
    else:
        os.replace(staging_dir, target)


def detected_cpu_list() -> str | None:
    if not hasattr(os, "sched_getaffinity"):
        return None
    allowed = sorted(os.sched_getaffinity(0))
    if not allowed:
        raise RuntimeError("the process affinity set is empty")
    return ",".join(str(cpu) for cpu in allowed[:2])


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument(
        "--cpu-list",
        help="taskset CPU list; omitted selects up to two CPUs from this process's allowed affinity set",
    )
    args = parser.parse_args()
    output_dir = args.output_dir.resolve()
    output_dir.mkdir(parents=True, exist_ok=True)
    manifest = load_json(MANIFEST)
    case_ids = [case["id"] for case in manifest["cases"]]
    cpu_list = args.cpu_list or detected_cpu_list()

    run(["cargo", "build", "--release", "-p", "pertylizer", "--bin", "pertylizer", "--bin", "render_cost", "--bin", "render_profile"])
    # --release reuses the artifacts the line above just built instead of
    # paying a second full dev-profile compile plus unoptimized renders; the
    # corpus digests are profile-independent (the dev-profile workspace gate
    # verifies the same digests).
    run(["cargo", "test", "--release", "-p", "pertylizer", "--test", "corpus_manifest"])
    with tempfile.TemporaryDirectory(prefix=".evd-0007-stage-", dir=output_dir) as temp:
        staging_root = Path(temp)
        staging_dir = staging_root / ARTIFACT_DIRECTORY
        staging_dir.mkdir()
        render_work = staging_root / "render-work"
        render_work.mkdir()
        collect_determinism(manifest, staging_dir, render_work)
        cost = collect_jsonl(
            RENDER_COST,
            case_ids,
            COST_ROUNDS,
            COST_REPS,
            staging_root / "raw-cost.jsonl",
            cpu_list,
        )
        aggregate_cost(cost, case_ids, staging_dir)
        profile = collect_jsonl(
            RENDER_PROFILE,
            case_ids,
            PROFILE_ROUNDS,
            PROFILE_REPS,
            staging_root / "raw-timing-memory.jsonl",
            cpu_list,
        )
        aggregate_profile(profile, case_ids, staging_dir)
        publish_csv_set(staging_dir, output_dir)


if __name__ == "__main__":
    main()
