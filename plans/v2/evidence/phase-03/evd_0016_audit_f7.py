#!/usr/bin/env python3
"""Recount a retained EVD-0016 duplex trace after the analyzer stops at F7."""

from __future__ import annotations

import argparse
import csv
import gzip
import io
import pathlib
import sys


HEADER = [
    "record_type", "direction", "sequence", "sample_count", "channels",
    "sample_rate_hz", "callback_ns", "endpoint_ns", "start_frame",
    "observer_before_ns", "stream_now_ns", "observer_after_ns", "error_count",
    "loss_count", "xrun_count", "device_unavailable_count",
    "stream_invalidated_count", "route_changed_count",
]
SUMMARY_FIELDS = (
    "error_count", "loss_count", "xrun_count", "device_unavailable_count",
    "stream_invalidated_count", "route_changed_count",
)


def number(row: dict[str, str], field: str) -> int:
    raw = row[field]
    if not raw.isascii() or not raw.isdecimal():
        raise ValueError(f"invalid {field}: {raw!r}")
    return int(raw)


def one_metadata(metadata: dict[str, list[str]], key: str) -> str:
    values = metadata.get(key, [])
    if len(values) != 1:
        raise ValueError(f"missing or repeated metadata {key}")
    return values[0]


def audit(path: pathlib.Path) -> list[list[str]]:
    if path.suffix == ".gz":
        with gzip.open(path, "rt", encoding="utf-8", newline="") as source:
            lines = source.read().splitlines()
    else:
        lines = path.read_text(encoding="utf-8").splitlines()
    metadata: dict[str, list[str]] = {}
    csv_lines = []
    for line in lines:
        if line.startswith("# "):
            key, separator, value = line[2:].partition("=")
            if not separator:
                raise ValueError("metadata line has no value")
            metadata.setdefault(key, []).append(value)
        elif line:
            csv_lines.append(line)
    if one_metadata(metadata, "probe_method") != "continuous-duplex-v2":
        raise ValueError("wrong probe method")
    if one_metadata(metadata, "probe_mode") != "duplex":
        raise ValueError("not a duplex trace")
    reader = csv.DictReader(io.StringIO("\n".join(csv_lines)))
    if reader.fieldnames != HEADER:
        raise ValueError("wrong CSV header")
    rows = list(reader)
    if any(None in row or None in row.values() for row in rows):
        raise ValueError("malformed CSV row width")
    if {row["direction"] for row in rows} != {"input", "output"}:
        raise ValueError("unexpected recorded directions")
    if any(row["record_type"] not in {"callback", "bridge", "summary"} for row in rows):
        raise ValueError("unexpected record type")

    output = []
    bridge_times: dict[str, list[int]] = {}
    for direction in ("input", "output"):
        callbacks = [
            row for row in rows
            if row["record_type"] == "callback" and row["direction"] == direction
        ]
        bridges = [
            row for row in rows
            if row["record_type"] == "bridge" and row["direction"] == direction
        ]
        summaries = [
            row for row in rows
            if row["record_type"] == "summary" and row["direction"] == direction
        ]
        if len(callbacks) < 10_000 or len(bridges) < 1_010 or len(summaries) != 1:
            raise ValueError(f"{direction}: incomplete callback, bridge or summary population")
        if [number(row, "sequence") for row in callbacks] != list(range(len(callbacks))):
            raise ValueError(f"{direction}: callback sequence is not consecutive")
        if [number(row, "sequence") for row in bridges] != list(range(len(bridges))):
            raise ValueError(f"{direction}: bridge sequence is not consecutive")

        rates = {number(row, "sample_rate_hz") for row in callbacks}
        channels = {number(row, "channels") for row in callbacks}
        if rates != {48_000} or channels != {2}:
            raise ValueError(f"{direction}: unexpected callback format")
        if one_metadata(metadata, f"{direction}_sample_rate_hz") != "48000":
            raise ValueError(f"{direction}: metadata sample rate differs")
        if one_metadata(metadata, f"{direction}_channels") != "2":
            raise ValueError(f"{direction}: metadata channel count differs")
        positions = []
        callback_times = []
        for row in callbacks:
            samples = number(row, "sample_count")
            if samples == 0 or samples % 2:
                raise ValueError(f"{direction}: invalid callback sample shape")
            positions.append((number(row, "start_frame"), samples // 2))
            callback_times.append(number(row, "callback_ns"))
            number(row, "endpoint_ns")
        if any(
            current_start != previous_start + previous_frames
            for (previous_start, previous_frames), (current_start, _) in
            zip(positions, positions[1:])
        ):
            raise ValueError(f"{direction}: derived frame position jumped")
        backward = sum(
            current < previous
            for previous, current in zip(callback_times, callback_times[1:])
        )

        before = [number(row, "observer_before_ns") for row in bridges]
        after = [number(row, "observer_after_ns") for row in bridges]
        for row in bridges:
            number(row, "stream_now_ns")
        if any(end < start for start, end in zip(before, after)):
            raise ValueError(f"{direction}: observer bracket inverted")
        if any(current < previous for previous, current in zip(before, before[1:])):
            raise ValueError(f"{direction}: observer clock moved backwards")
        bridge_times[direction] = before

        summary = summaries[0]
        if summary["sequence"] != "0":
            raise ValueError(f"{direction}: summary sequence is not zero")
        counts = {key: number(summary, key) for key in SUMMARY_FIELDS}
        realtime = one_metadata(metadata, f"{direction}_realtime_denied_count")
        if not realtime.isascii() or not realtime.isdecimal():
            raise ValueError(f"{direction}: invalid realtime_denied_count")
        classified = (
            counts["xrun_count"] + counts["device_unavailable_count"]
            + counts["stream_invalidated_count"] + counts["route_changed_count"]
            + int(realtime)
        )
        if classified > counts["error_count"]:
            raise ValueError(f"{direction}: classified errors exceed total")
        if (
            int(realtime) != 0
            or counts["loss_count"] != 0
            or counts["device_unavailable_count"] != 0
            or counts["stream_invalidated_count"] != 0
            or counts["route_changed_count"] != 0
            or counts["error_count"] != counts["xrun_count"]
        ):
            raise ValueError(f"{direction}: retained F7 counter contract changed")
        output.append([
            direction, str(len(callbacks)), str(len(bridges)), str(backward),
            str(counts["error_count"]), str(counts["xrun_count"]),
            str(counts["loss_count"]), realtime,
        ])
    if len(bridge_times["input"]) != len(bridge_times["output"]):
        raise ValueError("duplex bridge populations differ")
    if any(
        abs(input_time - output_time) >= 20_000_000
        for input_time, output_time in zip(
            bridge_times["input"], bridge_times["output"]
        )
    ):
        raise ValueError("duplex bridge samples are not concurrent")
    return output


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("artifact", type=pathlib.Path)
    arguments = parser.parse_args()
    try:
        results = audit(arguments.artifact)
    except (OSError, UnicodeError, ValueError) as error:
        print(f"EVD-0016 audit invalid: {error}", file=sys.stderr)
        return 1
    writer = csv.writer(sys.stdout, lineterminator="\n")
    writer.writerow((
        "direction", "callbacks", "bridges", "backward_callback_steps",
        "error_count", "xrun_count", "loss_count", "realtime_denied_count",
    ))
    writer.writerows(results)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
