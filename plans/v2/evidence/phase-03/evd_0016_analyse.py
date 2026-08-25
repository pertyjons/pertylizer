#!/usr/bin/env python3
"""Validate and summarize EVD-0016 CPAL callback artifacts."""

from __future__ import annotations

import argparse
import csv
import io
import math
import pathlib
import sys
from dataclasses import dataclass, replace


HEADER = [
    "record_type",
    "direction",
    "sequence",
    "sample_count",
    "channels",
    "sample_rate_hz",
    "callback_ns",
    "endpoint_ns",
    "start_frame",
    "observer_before_ns",
    "stream_now_ns",
    "observer_after_ns",
    "error_count",
    "loss_count",
    "xrun_count",
    "device_unavailable_count",
    "stream_invalidated_count",
    "route_changed_count",
]

RELEASE_PLATFORMS = {"linux", "macos", "windows"}
CALLBACK_PRIORITY_CONTRACT = {
    "linux": (
        "cpal-alsa-helper-noop-without-realtime-dbus",
        "no-cpal-promotion-by-build-contract",
    ),
    "macos": ("coreaudio-backend-managed", "unobserved"),
    "windows": ("cpal-wasapi-promotion-attempt", "unobserved"),
}
CPAL_VERSION = "0.18.2"
CPAL_DEVICE_TYPES = {
    "Speaker",
    "Microphone",
    "Headphones",
    "Headset",
    "Earpiece",
    "Handset",
    "HearingAid",
    "Dock",
    "Tuner",
    "Virtual",
    "Unknown",
}
FIT_WARMUP_RECORDS = 10
MINIMUM_CALLBACKS = 10_000
MINIMUM_FIT_OBSERVATIONS = 1_000
MINIMUM_BRIDGES = FIT_WARMUP_RECORDS + MINIMUM_FIT_OBSERVATIONS
REPO_ROOT = pathlib.Path(__file__).resolve().parents[4]
ENDPOINT_POLICY_CASES = (
    REPO_ROOT
    / "crates"
    / "pertylizer"
    / "examples"
    / "evd_0016_endpoint_policy.tsv"
)


class InvalidArtifact(ValueError):
    """A retained callback artifact violated its declared schema or control."""


class FalsifierTriggered(ValueError):
    """A structurally valid observation fired a declared falsifier."""


@dataclass(frozen=True)
class Callback:
    sequence: int
    sample_count: int
    channels: int
    sample_rate_hz: int
    callback_ns: int
    endpoint_ns: int
    start_frame: int

    @property
    def frames(self) -> int:
        return self.sample_count // self.channels


@dataclass(frozen=True)
class Bridge:
    sequence: int
    observer_before_ns: int
    stream_now_ns: int
    observer_after_ns: int


@dataclass(frozen=True)
class Summary:
    errors: int
    losses: int
    xruns: int
    device_unavailable: int
    stream_invalidated: int
    route_changed: int


@dataclass(frozen=True)
class DirectionResult:
    artifact: str
    platform: str
    direction: str
    callbacks: int
    sample_rate_hz: int
    quantum_frames: int
    minimum_frames: int
    maximum_frames: int
    effective_rate_hz: float
    drift_ppm: float
    fit_residual_p999_frames: float
    period_residual_p999_frames: float
    bridge_half_width_max_frames: float
    bridge_fit_residual_p999_frames: float
    stream_now_freshness_bound_frames: int
    latency_p50_frames: float
    latency_p999_frames: float
    bridged_uncertainty_frames: int
    duplex_uncertainty_frames: int | None
    realtime_denied_count: int
    f4_outcome: str
    falsifiers: str


def parse_nonnegative(row: dict[str, str], field: str) -> int:
    raw = row.get(field, "")
    if raw == "":
        raise InvalidArtifact(f"missing {field}")
    try:
        value = int(raw)
    except ValueError as error:
        raise InvalidArtifact(f"invalid integer {field}={raw!r}") from error
    if value < 0:
        raise InvalidArtifact(f"negative {field}={value}")
    return value


def parse_text(text: str) -> tuple[dict[str, list[str]], list[dict[str, str]]]:
    metadata: dict[str, list[str]] = {}
    csv_lines: list[str] = []
    for line in text.splitlines():
        if line.startswith("# "):
            key_value = line[2:].split("=", 1)
            if len(key_value) == 2:
                metadata.setdefault(key_value[0], []).append(key_value[1])
        elif line.strip():
            csv_lines.append(line)
    if not csv_lines:
        raise InvalidArtifact("artifact has no CSV records")
    reader = csv.DictReader(io.StringIO("\n".join(csv_lines)))
    if reader.fieldnames != HEADER:
        raise InvalidArtifact(
            f"wrong CSV header: expected {HEADER!r}, observed {reader.fieldnames!r}"
        )
    rows = list(reader)
    if not rows:
        raise InvalidArtifact("artifact has a header but no records")
    if any(None in row for row in rows):
        raise InvalidArtifact("a row has more fields than the declared header")
    required_by_kind = {
        "callback": {
            "record_type",
            "direction",
            "sequence",
            "sample_count",
            "channels",
            "sample_rate_hz",
            "callback_ns",
            "endpoint_ns",
            "start_frame",
        },
        "bridge": {
            "record_type",
            "direction",
            "sequence",
            "observer_before_ns",
            "stream_now_ns",
            "observer_after_ns",
        },
        "summary": {
            "record_type",
            "direction",
            "sequence",
            "error_count",
            "loss_count",
            "xrun_count",
            "device_unavailable_count",
            "stream_invalidated_count",
            "route_changed_count",
        },
    }
    for row in rows:
        if any(value is None for value in row.values()):
            raise InvalidArtifact("row has fewer fields than the declared header")
        kind = row["record_type"]
        required = required_by_kind.get(kind)
        if required is None:
            raise InvalidArtifact(f"unknown record_type={kind!r}")
        if row["direction"] not in {"input", "output"}:
            raise InvalidArtifact(f"unknown direction={row['direction']!r}")
        missing = sorted(field for field in required if row[field] == "")
        if missing:
            raise InvalidArtifact(f"{kind} row is missing {', '.join(missing)}")
        unexpected = sorted(
            field for field in HEADER if field not in required and row[field] != ""
        )
        if unexpected:
            raise InvalidArtifact(f"{kind} row populates {', '.join(unexpected)}")
    return metadata, rows


def callbacks_for(rows: list[dict[str, str]], direction: str) -> list[Callback]:
    callbacks: list[Callback] = []
    for row in rows:
        if row["record_type"] != "callback" or row["direction"] != direction:
            continue
        callback = Callback(
            sequence=parse_nonnegative(row, "sequence"),
            sample_count=parse_nonnegative(row, "sample_count"),
            channels=parse_nonnegative(row, "channels"),
            sample_rate_hz=parse_nonnegative(row, "sample_rate_hz"),
            callback_ns=parse_nonnegative(row, "callback_ns"),
            endpoint_ns=parse_nonnegative(row, "endpoint_ns"),
            start_frame=parse_nonnegative(row, "start_frame"),
        )
        if callback.sample_count == 0 or callback.channels == 0:
            raise FalsifierTriggered(
                f"F7: {direction} callback has an empty sample shape"
            )
        if callback.sample_count % callback.channels != 0:
            raise FalsifierTriggered(
                f"F7: {direction} sequence {callback.sequence} has "
                f"{callback.sample_count} samples across {callback.channels} channels"
            )
        if callback.sample_rate_hz == 0:
            raise FalsifierTriggered(f"F7: {direction} callback has a zero sample rate")
        callbacks.append(callback)
    return callbacks


def bridges_for(rows: list[dict[str, str]], direction: str) -> list[Bridge]:
    bridges: list[Bridge] = []
    for row in rows:
        if row["record_type"] != "bridge" or row["direction"] != direction:
            continue
        bridges.append(
            Bridge(
                sequence=parse_nonnegative(row, "sequence"),
                observer_before_ns=parse_nonnegative(row, "observer_before_ns"),
                stream_now_ns=parse_nonnegative(row, "stream_now_ns"),
                observer_after_ns=parse_nonnegative(row, "observer_after_ns"),
            )
        )
    return bridges


def summary_for(rows: list[dict[str, str]], direction: str) -> Summary:
    summaries = [
        row
        for row in rows
        if row["record_type"] == "summary" and row["direction"] == direction
    ]
    if len(summaries) != 1:
        raise InvalidArtifact(
            f"{direction} has {len(summaries)} summaries; expected exactly one"
        )
    row = summaries[0]
    if row["sequence"] != "0":
        raise InvalidArtifact(f"{direction} summary sequence must be zero")
    summary = Summary(
        errors=parse_nonnegative(row, "error_count"),
        losses=parse_nonnegative(row, "loss_count"),
        xruns=parse_nonnegative(row, "xrun_count"),
        device_unavailable=parse_nonnegative(row, "device_unavailable_count"),
        stream_invalidated=parse_nonnegative(row, "stream_invalidated_count"),
        route_changed=parse_nonnegative(row, "route_changed_count"),
    )
    classified_errors = (
        summary.xruns
        + summary.device_unavailable
        + summary.stream_invalidated
        + summary.route_changed
    )
    if classified_errors > summary.errors:
        raise InvalidArtifact(
            f"{direction} classified {classified_errors} errors but total is {summary.errors}"
        )
    return summary


def validate_sequences(values: list[int], label: str) -> None:
    if values != list(range(len(values))):
        raise InvalidArtifact(
            f"{label} sequences are missing, duplicated, or reordered"
        )


def percentile(values: list[float], fraction: float) -> float:
    if not values:
        raise InvalidArtifact("cannot estimate a percentile from no observations")
    ordered = sorted(values)
    index = max(0, math.ceil(fraction * len(ordered)) - 1)
    return ordered[index]


def endpoint_is_disallowed(
    platform: str, name: str, device_id: str, device_type: str
) -> bool:
    ascii_lower = str.maketrans(
        "ABCDEFGHIJKLMNOPQRSTUVWXYZ", "abcdefghijklmnopqrstuvwxyz"
    )
    name = name.translate(ascii_lower)
    device_id = device_id.translate(ascii_lower)
    identity = f"{name} {device_id}"
    virtual_markers = (
        "null",
        "pipewire",
        "pulseaudio",
        "virtual",
        "dummy",
        "blackhole",
        "soundflower",
        "loopback",
        "aggregate device",
        "multi-output device",
        "vb-cable",
        "cable input",
    )
    if device_type == "Virtual" or any(marker in identity for marker in virtual_markers):
        return True
    if device_id in {"alsa:default", "alsa:pulse"}:
        return True
    if platform == "linux":
        return not device_id.startswith("alsa:hw:")
    return False


def fit_clock(callbacks: list[Callback]) -> tuple[float, float, list[float]]:
    usable = callbacks[FIT_WARMUP_RECORDS:]
    if len(usable) < 2:
        raise InvalidArtifact("too few callbacks remain after startup warm-up")
    first_frame = usable[0].start_frame
    first_ns = usable[0].callback_ns
    xs = [float(row.start_frame - first_frame) for row in usable]
    ys = [float(row.callback_ns - first_ns) for row in usable]
    mean_x = sum(xs) / len(xs)
    mean_y = sum(ys) / len(ys)
    denominator = sum((x - mean_x) ** 2 for x in xs)
    if denominator == 0.0:
        raise InvalidArtifact("callback frame positions have no span")
    slope_ns_per_frame = sum(
        (x - mean_x) * (y - mean_y) for x, y in zip(xs, ys, strict=True)
    ) / denominator
    if slope_ns_per_frame <= 0.0:
        raise FalsifierTriggered("F7: fitted stream clock does not advance")
    intercept = mean_y - slope_ns_per_frame * mean_x
    residuals_ns = [
        abs(y - (intercept + slope_ns_per_frame * x))
        for x, y in zip(xs, ys, strict=True)
    ]
    effective_rate_hz = 1_000_000_000.0 / slope_ns_per_frame
    nominal_rate_hz = float(usable[0].sample_rate_hz)
    drift_ppm = (effective_rate_hz / nominal_rate_hz - 1.0) * 1_000_000.0
    return effective_rate_hz, drift_ppm, residuals_ns


def bridge_fit_residuals(bridges: list[Bridge]) -> list[float]:
    usable = bridges[FIT_WARMUP_RECORDS:]
    if len(usable) < 2:
        raise InvalidArtifact("too few observer bridges remain after startup warm-up")
    first_stream_ns = usable[0].stream_now_ns
    first_observer_ns = (
        usable[0].observer_before_ns + usable[0].observer_after_ns
    ) / 2.0
    xs = [float(row.stream_now_ns - first_stream_ns) for row in usable]
    ys = [
        (row.observer_before_ns + row.observer_after_ns) / 2.0
        - first_observer_ns
        for row in usable
    ]
    mean_x = sum(xs) / len(xs)
    mean_y = sum(ys) / len(ys)
    denominator = sum((x - mean_x) ** 2 for x in xs)
    if denominator == 0.0:
        raise InvalidArtifact("observer bridges have no stream-clock span")
    slope = sum(
        (x - mean_x) * (y - mean_y) for x, y in zip(xs, ys, strict=True)
    ) / denominator
    if slope <= 0.0:
        raise FalsifierTriggered(
            "F6: observer bridge maps to a non-advancing clock"
        )
    intercept = mean_y - slope * mean_x
    return [
        abs(y - (intercept + slope * x))
        for x, y in zip(xs, ys, strict=True)
    ]


def analyze_direction(
    artifact: pathlib.Path,
    platform: str,
    direction: str,
    rows: list[dict[str, str]],
    minimum_callbacks: int,
    quantum_frames: int,
    realtime_denied_count: int,
    stream_now_freshness_bound_frames: int,
) -> DirectionResult:
    callbacks = callbacks_for(rows, direction)
    if len(callbacks) < minimum_callbacks:
        raise InvalidArtifact(
            f"{direction} has {len(callbacks)} callbacks; need {minimum_callbacks}"
        )
    validate_sequences([row.sequence for row in callbacks], f"{direction} callback")
    rates = {row.sample_rate_hz for row in callbacks}
    channels = {row.channels for row in callbacks}
    if len(rates) != 1 or len(channels) != 1:
        raise FalsifierTriggered(
            f"F7: {direction} format changed within one stream epoch"
        )
    for previous, current in zip(callbacks, callbacks[1:], strict=False):
        if current.callback_ns < previous.callback_ns:
            raise FalsifierTriggered(
                f"F7: {direction} callback clock moved backwards"
            )
        if current.endpoint_ns < previous.endpoint_ns:
            raise FalsifierTriggered(
                f"F7: {direction} endpoint clock moved backwards"
            )
        expected_start = previous.start_frame + previous.frames
        if current.start_frame != expected_start:
            raise InvalidArtifact(
                f"{direction} artifact frame position jumped from {expected_start} "
                f"to {current.start_frame}"
            )
    if direction == "output":
        if any(row.endpoint_ns < row.callback_ns for row in callbacks):
            raise FalsifierTriggered("F7: output playback precedes its callback")
        latencies_ns = [float(row.endpoint_ns - row.callback_ns) for row in callbacks]
    else:
        if any(row.endpoint_ns > row.callback_ns for row in callbacks):
            raise FalsifierTriggered("F7: input capture follows its callback")
        latencies_ns = [float(row.callback_ns - row.endpoint_ns) for row in callbacks]

    bridges = bridges_for(rows, direction)
    if len(bridges) < MINIMUM_BRIDGES:
        raise InvalidArtifact(
            f"{direction} has {len(bridges)} observer bridges; "
            f"need {MINIMUM_BRIDGES} to retain {MINIMUM_FIT_OBSERVATIONS} "
            "after warm-up for p99.9"
        )
    validate_sequences([row.sequence for row in bridges], f"{direction} bridge")
    for previous, current in zip(bridges, bridges[1:], strict=False):
        if current.stream_now_ns < previous.stream_now_ns:
            raise FalsifierTriggered(
                f"F6: {direction} Stream::now moved backwards"
            )
        if current.observer_before_ns < previous.observer_before_ns:
            raise FalsifierTriggered(
                f"F6: {direction} observer clock moved backwards"
            )
    if any(row.observer_after_ns < row.observer_before_ns for row in bridges):
        raise FalsifierTriggered(f"F6: {direction} observer bracket is inverted")

    summary = summary_for(rows, direction)
    classified_errors = (
        summary.xruns
        + summary.device_unavailable
        + summary.stream_invalidated
        + summary.route_changed
        + realtime_denied_count
    )
    if classified_errors > summary.errors:
        raise InvalidArtifact(
            f"{direction} classified {classified_errors} errors but total is "
            f"{summary.errors}"
        )
    if summary.losses != 0:
        raise FalsifierTriggered(
            f"F7: {direction} lost {summary.losses} callback records"
        )
    fatal_errors = summary.errors - realtime_denied_count
    if fatal_errors != 0:
        raise FalsifierTriggered(
            f"F7: {direction} received {fatal_errors} fatal stream errors "
            f"(xruns={summary.xruns}, unavailable={summary.device_unavailable}, "
            f"invalidated={summary.stream_invalidated}, route_changed={summary.route_changed})"
        )

    rate = callbacks[0].sample_rate_hz
    effective_rate, drift_ppm, fit_residuals_ns = fit_clock(callbacks)
    fit_p999_ns = percentile(fit_residuals_ns, 0.999)
    period_residuals_ns = []
    for previous, current in zip(callbacks, callbacks[1:], strict=False):
        elapsed = float(current.callback_ns - previous.callback_ns)
        expected = float(previous.frames) * 1_000_000_000.0 / float(rate)
        period_residuals_ns.append(abs(elapsed - expected))
    bridge_half_width_ns = max(
        (row.observer_after_ns - row.observer_before_ns) / 2.0 for row in bridges
    )
    bridge_fit_p999_ns = percentile(bridge_fit_residuals(bridges), 0.999)
    frames_per_ns = float(rate) / 1_000_000_000.0
    fit_p999_frames = fit_p999_ns * frames_per_ns
    bridge_half_width_frames = bridge_half_width_ns * frames_per_ns
    bridge_fit_p999_frames = bridge_fit_p999_ns * frames_per_ns
    uncertainty = math.ceil(
        fit_p999_frames
        + bridge_fit_p999_frames
        + bridge_half_width_frames
        + float(stream_now_freshness_bound_frames)
        + 1.0
    )
    falsifiers = []
    if uncertainty >= quantum_frames:
        falsifiers.append(
            f"F4: bridged p99.9 uncertainty {uncertainty} frames is at least "
            f"Q={quantum_frames}"
        )

    return DirectionResult(
        artifact=artifact.name,
        platform=platform,
        direction=direction,
        callbacks=len(callbacks),
        sample_rate_hz=rate,
        quantum_frames=quantum_frames,
        minimum_frames=min(row.frames for row in callbacks),
        maximum_frames=max(row.frames for row in callbacks),
        effective_rate_hz=effective_rate,
        drift_ppm=drift_ppm,
        fit_residual_p999_frames=fit_p999_frames,
        period_residual_p999_frames=percentile(period_residuals_ns, 0.999)
        * frames_per_ns,
        bridge_half_width_max_frames=bridge_half_width_frames,
        bridge_fit_residual_p999_frames=bridge_fit_p999_frames,
        stream_now_freshness_bound_frames=stream_now_freshness_bound_frames,
        latency_p50_frames=percentile(latencies_ns, 0.5) * frames_per_ns,
        latency_p999_frames=percentile(latencies_ns, 0.999) * frames_per_ns,
        bridged_uncertainty_frames=uncertainty,
        duplex_uncertainty_frames=None,
        realtime_denied_count=realtime_denied_count,
        f4_outcome="Not supported" if falsifiers else "Inconclusive",
        falsifiers="; ".join(falsifiers),
    )


def analyze_artifact(artifact: pathlib.Path) -> list[DirectionResult]:
    return analyze_text(
        artifact, artifact.read_text(encoding="utf-8"), MINIMUM_CALLBACKS
    )


def analyze_text(
    artifact: pathlib.Path, text: str, minimum_callbacks: int
) -> list[DirectionResult]:
    metadata, rows = parse_text(text)
    if metadata.get("evd") != ["EVD-0016"]:
        raise InvalidArtifact(f"{artifact}: missing EVD-0016 marker")
    if metadata.get("cpal_version") != [CPAL_VERSION]:
        raise InvalidArtifact(
            f"{artifact}: measurement did not use CPAL {CPAL_VERSION}"
        )
    if metadata.get("build_profile") != ["release"]:
        raise InvalidArtifact(f"{artifact}: measurement was not made by a release build")
    platforms = metadata.get("os", [])
    if len(platforms) != 1 or platforms[0] not in RELEASE_PLATFORMS:
        raise InvalidArtifact(f"{artifact}: unsupported or ambiguous release platform")
    for key in ("host", "arch"):
        values = metadata.get(key, [])
        if len(values) != 1 or values[0] == "":
            raise InvalidArtifact(f"{artifact}: missing or ambiguous {key}")
    control_fixtures = metadata.get("control_fixture", [])
    if len(control_fixtures) != 1 or control_fixtures[0] not in {
        "none",
        "synthetic-continuous-clock",
    }:
        raise InvalidArtifact(f"{artifact}: unknown or ambiguous control fixture")
    synthetic_control = control_fixtures == ["synthetic-continuous-clock"]
    if synthetic_control != (metadata["host"] == ["synthetic"]):
        raise InvalidArtifact(
            f"{artifact}: synthetic host and control-fixture marker disagree"
        )
    if metadata.get("observer_clock") != ["std-instant-process-monotonic"]:
        raise InvalidArtifact(f"{artifact}: unknown or ambiguous observer clock")
    if metadata.get("observer_thread_priority") != ["normal-not-promoted"]:
        raise InvalidArtifact(f"{artifact}: unknown observer-thread priority policy")
    quantum_values = metadata.get("quantum_frames", [])
    if len(quantum_values) != 1:
        raise InvalidArtifact(f"{artifact}: missing or ambiguous quantum_frames")
    try:
        quantum_frames = int(quantum_values[0])
    except ValueError as error:
        raise InvalidArtifact(f"{artifact}: invalid quantum_frames") from error
    if quantum_frames <= 0:
        raise InvalidArtifact(f"{artifact}: quantum_frames must be positive")
    callback_targets = metadata.get("callback_target", [])
    if len(callback_targets) != 1:
        raise InvalidArtifact(f"{artifact}: missing or ambiguous callback target")
    try:
        callback_target = int(callback_targets[0])
    except ValueError as error:
        raise InvalidArtifact(f"{artifact}: invalid callback target") from error
    if callback_target < minimum_callbacks:
        raise InvalidArtifact(
            f"{artifact}: callback target {callback_target} is below "
            f"the required {minimum_callbacks}"
        )
    directions = sorted(
        {
            row["direction"]
            for row in rows
            if row["record_type"] == "callback" and row["direction"] in {"input", "output"}
        }
    )
    if not directions:
        raise InvalidArtifact(f"{artifact}: no callback direction")
    freshness_bounds: dict[str, int] = {}
    for direction in directions:
        for key in (
            f"{direction}_device_name",
            f"{direction}_device_id",
            f"{direction}_device_type",
        ):
            values = metadata.get(key, [])
            if len(values) != 1 or values[0] == "":
                raise InvalidArtifact(f"{artifact}: missing or ambiguous {key}")
        if metadata.get(f"{direction}_physical_device_attested") != ["true"]:
            raise InvalidArtifact(
                f"{artifact}: {direction} physical endpoint was not attested"
            )
        if metadata.get(f"{direction}_endpoint_policy") != [
            "explicit-physical-endpoint"
        ]:
            raise InvalidArtifact(
                f"{artifact}: {direction} did not use the explicit non-virtual policy"
            )
        if metadata.get(f"{direction}_stream_now_source") != [
            "cpal-stream-instant-backend-private"
        ]:
            raise InvalidArtifact(
                f"{artifact}: {direction} has an unknown Stream::now source"
            )
        if metadata.get(f"{direction}_stream_now_backend_mode") != [
            f"unobservable-through-cpal-{CPAL_VERSION}"
        ]:
            raise InvalidArtifact(
                f"{artifact}: {direction} does not disclose the private backend mode"
            )
        expected_priority_path, expected_priority_observation = (
            CALLBACK_PRIORITY_CONTRACT[platforms[0]]
        )
        if metadata.get(f"{direction}_callback_priority_path") != [
            expected_priority_path
        ]:
            raise InvalidArtifact(
                f"{artifact}: {direction} callback priority path does not match "
                f"{platforms[0]}"
            )
        if metadata.get(f"{direction}_callback_priority_observation") != [
            expected_priority_observation
        ]:
            raise InvalidArtifact(
                f"{artifact}: {direction} callback priority observation does not "
                f"match {platforms[0]}"
            )
        if metadata.get(f"{direction}_bridge_sample_burst") != ["1"]:
            raise InvalidArtifact(
                f"{artifact}: {direction} did not use the declared bridge burst"
            )
        callbacks = callbacks_for(rows, direction)
        device_name = metadata[f"{direction}_device_name"][0]
        device_id = metadata[f"{direction}_device_id"][0]
        device_type = metadata[f"{direction}_device_type"][0]
        if device_type not in CPAL_DEVICE_TYPES:
            raise InvalidArtifact(
                f"{artifact}: {direction} has unknown CPAL device type {device_type!r}"
            )
        if endpoint_is_disallowed(platforms[0], device_name, device_id, device_type):
            raise InvalidArtifact(
                f"{artifact}: {direction} endpoint is virtual, server-backed, "
                f"or not a direct ALSA hardware PCM: {device_name} ({device_id})"
            )
        for suffix, observed in (
            ("sample_rate_hz", callbacks[0].sample_rate_hz),
            ("channels", callbacks[0].channels),
        ):
            key = f"{direction}_{suffix}"
            values = metadata.get(key, [])
            if values != [str(observed)]:
                raise InvalidArtifact(
                    f"{artifact}: {key} does not match callback records"
                )
        for suffix in ("sample_format", "supported_buffer_size"):
            key = f"{direction}_{suffix}"
            values = metadata.get(key, [])
            if len(values) != 1 or values[0] == "":
                raise InvalidArtifact(f"{artifact}: missing or ambiguous {key}")
        negotiated_key = f"{direction}_negotiated_buffer_frames"
        negotiated_values = metadata.get(negotiated_key, [])
        negotiated_error_key = f"{direction}_negotiated_buffer_frames_error"
        negotiated_errors = metadata.get(negotiated_error_key, [])
        if (len(negotiated_values), len(negotiated_errors)) not in {(1, 0), (0, 1)}:
            raise InvalidArtifact(
                f"{artifact}: require exactly one of {negotiated_key} or "
                f"{negotiated_error_key}"
            )
        negotiated_frames = None
        if negotiated_values:
            try:
                negotiated_frames = int(negotiated_values[0])
            except ValueError as error:
                raise InvalidArtifact(f"{artifact}: invalid {negotiated_key}") from error
            if negotiated_frames <= 0:
                raise InvalidArtifact(f"{artifact}: {negotiated_key} must be positive")
        elif negotiated_errors[0] == "":
            raise InvalidArtifact(f"{artifact}: empty {negotiated_error_key}")
        freshness_source_key = f"{direction}_stream_now_freshness_bound_source"
        freshness_source = metadata.get(freshness_source_key, [])
        freshness_key = f"{direction}_stream_now_freshness_bound_frames"
        freshness_values = metadata.get(freshness_key, [])
        freshness_error_key = f"{direction}_stream_now_freshness_bound_error"
        freshness_errors = metadata.get(freshness_error_key, [])
        if (len(freshness_values), len(freshness_errors)) not in {(1, 0), (0, 1)}:
            raise InvalidArtifact(
                f"{artifact}: require exactly one of {freshness_key} or "
                f"{freshness_error_key}"
            )
        if freshness_errors:
            raise InvalidArtifact(
                f"{artifact}: {direction} has no reviewed Stream::now freshness "
                f"bound: {freshness_errors[0]}"
            )
        try:
            freshness_bound = int(freshness_values[0])
        except ValueError as error:
            raise InvalidArtifact(f"{artifact}: invalid {freshness_key}") from error
        if freshness_bound < 0:
            raise InvalidArtifact(f"{artifact}: {freshness_key} must be nonnegative")
        if synthetic_control:
            expected_freshness_source = "synthetic-continuous-clock-control"
        elif platforms[0] == "linux":
            expected_freshness_source = (
                f"cpal-{CPAL_VERSION}-alsa-one-negotiated-period-source-audit"
            )
            if negotiated_frames is None or freshness_bound != negotiated_frames:
                raise InvalidArtifact(
                    f"{artifact}: {freshness_key} must equal the negotiated "
                    "ALSA period"
                )
            if any(callback.frames > freshness_bound for callback in callbacks):
                raise InvalidArtifact(
                    f"{artifact}: {direction} callback exceeds the audited ALSA "
                    "freshness period"
                )
        else:
            raise InvalidArtifact(
                f"{artifact}: no reviewed Stream::now freshness method for "
                f"{platforms[0]}"
            )
        if freshness_source != [expected_freshness_source]:
            raise InvalidArtifact(
                f"{artifact}: {freshness_source_key} does not match the "
                "reviewed method"
            )
        freshness_bounds[direction] = freshness_bound
    results = []
    for direction in directions:
        key = f"{direction}_realtime_denied_count"
        values = metadata.get(key, [])
        if len(values) != 1:
            raise InvalidArtifact(f"{artifact}: missing or ambiguous {key}")
        try:
            realtime_denied_count = int(values[0])
        except ValueError as error:
            raise InvalidArtifact(f"{artifact}: invalid {key}") from error
        if realtime_denied_count < 0:
            raise InvalidArtifact(f"{artifact}: negative {key}")
        results.append(
            analyze_direction(
                artifact,
                platforms[0],
                direction,
                rows,
                minimum_callbacks,
                quantum_frames,
                realtime_denied_count,
                freshness_bounds[direction],
            )
        )
    if directions == ["input", "output"]:
        duplex_uncertainty = sum(
            result.bridged_uncertainty_frames for result in results
        )
        duplex_falsifier = (
            f"F4: duplex input-to-output uncertainty {duplex_uncertainty} "
            f"frames is at least Q={quantum_frames}"
            if duplex_uncertainty >= quantum_frames
            else ""
        )
        results = [
            replace(
                result,
                duplex_uncertainty_frames=duplex_uncertainty,
                f4_outcome=(
                    "Not supported" if duplex_falsifier else "Within F4"
                ),
                falsifiers="; ".join(
                    item
                    for item in (result.falsifiers, duplex_falsifier)
                    if item
                ),
            )
            for result in results
        ]
    return results


def synthetic_artifact() -> str:
    lines = [
        "# evd=EVD-0016",
        f"# cpal_version={CPAL_VERSION}",
        "# build_profile=release",
        "# os=linux",
        "# host=synthetic",
        "# arch=x86_64",
        "# control_fixture=synthetic-continuous-clock",
        "# observer_clock=std-instant-process-monotonic",
        "# observer_thread_priority=normal-not-promoted",
        "# quantum_frames=64",
        "# callback_target=20",
        "# output_device_name=Synthetic output",
        "# output_device_id=alsa:hw:CARD=synthetic,DEV=0",
        "# output_device_type=Speaker",
        "# output_physical_device_attested=true",
        "# output_endpoint_policy=explicit-physical-endpoint",
        "# output_stream_now_source=cpal-stream-instant-backend-private",
        f"# output_stream_now_backend_mode=unobservable-through-cpal-{CPAL_VERSION}",
        "# output_callback_priority_path=cpal-alsa-helper-noop-without-realtime-dbus",
        "# output_callback_priority_observation=no-cpal-promotion-by-build-contract",
        "# output_bridge_sample_burst=1",
        "# output_realtime_denied_count=0",
        "# output_sample_format=F32",
        "# output_sample_rate_hz=48000",
        "# output_channels=2",
        "# output_supported_buffer_size=Range { min: 64, max: 1024 }",
        "# output_negotiated_buffer_frames=64",
        "# output_stream_now_freshness_bound_source=synthetic-continuous-clock-control",
        "# output_stream_now_freshness_bound_frames=0",
        ",".join(HEADER),
    ]
    rate = 48_000
    frames = 64
    for sequence in range(20):
        callback = sequence * frames * 1_000_000_000 // rate
        row = [
            "callback",
            "output",
            str(sequence),
            str(frames * 2),
            "2",
            str(rate),
            str(callback),
            str(callback + 2_000_000),
            str(sequence * frames),
        ] + [""] * 9
        lines.append(",".join(row))
    for sequence in range(MINIMUM_BRIDGES):
        observer = sequence * 10_000_000
        row = ["bridge", "output", str(sequence)] + [""] * 6 + [
            str(observer),
            str(observer + 1_000),
            str(observer + 2_000),
        ] + [""] * 6
        lines.append(",".join(row))
    lines.append(",".join(["summary", "output", "0"] + [""] * 9 + ["0"] * 6))
    return "\n".join(lines) + "\n"


def synthetic_duplex_artifact() -> str:
    output_lines = synthetic_artifact().splitlines()
    header_index = output_lines.index(",".join(HEADER))
    metadata = output_lines[:header_index]
    output_rows = output_lines[header_index + 1 :]
    input_metadata = []
    for line in metadata:
        if line.startswith("# output_"):
            input_line = line.replace("# output_", "# input_", 1)
            input_line = input_line.replace(
                "Synthetic output", "Synthetic input"
            ).replace("device_type=Speaker", "device_type=Microphone")
            input_metadata.append(input_line)
    input_rows = []
    for line in output_rows:
        row = next(csv.reader([line]))
        row[1] = "input"
        if row[0] == "callback":
            callback_ns = int(row[6]) + 9_000_000
            row[6] = str(callback_ns)
            row[7] = str(callback_ns - 1_000_000)
        elif row[0] == "bridge":
            row[9] = str(int(row[9]) + 3_000_000)
            row[10] = str(int(row[10]) + 9_000_000)
            row[11] = str(int(row[11]) + 3_000_000)
        input_rows.append(",".join(row))
    return (
        "\n".join(
            metadata
            + input_metadata
            + [",".join(HEADER)]
            + output_rows
            + input_rows
        )
        + "\n"
    )


def remove_unique_row(text: str, prefix: str) -> str:
    lines = text.splitlines()
    matches = [index for index, line in enumerate(lines) if line.startswith(prefix)]
    if len(matches) != 1:
        raise AssertionError(f"expected one synthetic row starting with {prefix!r}")
    del lines[matches[0]]
    return "\n".join(lines) + "\n"


def require_release_platforms(results: list[DirectionResult]) -> None:
    observed = {(result.platform, result.direction) for result in results}
    required = {
        (platform, direction)
        for platform in RELEASE_PLATFORMS
        for direction in {"input", "output"}
    }
    missing = required - observed
    if missing:
        raise InvalidArtifact(
            "missing release-platform directions: "
            + ", ".join(
                f"{platform}/{direction}"
                for platform, direction in sorted(missing)
            )
        )


def self_test() -> None:
    valid = synthetic_artifact()
    metadata, rows = parse_text(valid)
    if metadata.get("evd") != ["EVD-0016"]:
        raise AssertionError("valid control lost its metadata")
    valid_results = analyze_text(pathlib.Path("synthetic.csv"), valid, 20)
    if len(valid_results) != 1 or valid_results[0].f4_outcome != "Inconclusive":
        raise AssertionError(
            "valid single-direction control did not remain F4-inconclusive"
        )
    duplex = synthetic_duplex_artifact()
    _, duplex_rows = parse_text(duplex)
    input_stream_times = {
        row["stream_now_ns"]
        for row in duplex_rows
        if row["record_type"] == "bridge" and row["direction"] == "input"
    }
    output_stream_times = {
        row["stream_now_ns"]
        for row in duplex_rows
        if row["record_type"] == "bridge" and row["direction"] == "output"
    }
    if input_stream_times == output_stream_times:
        raise AssertionError("duplex control reused one raw stream clock")
    duplex_results = analyze_text(pathlib.Path("synthetic-duplex.csv"), duplex, 20)
    if len(duplex_results) != 2 or any(
        result.f4_outcome != "Within F4" for result in duplex_results
    ):
        raise AssertionError("valid duplex control did not pass F4")
    try:
        require_release_platforms(duplex_results)
    except InvalidArtifact:
        pass
    else:
        raise AssertionError("incomplete release-platform coverage was accepted")
    complete_platform_results = [
        replace(result, platform=platform)
        for platform in sorted(RELEASE_PLATFORMS)
        for result in duplex_results
    ]
    require_release_platforms(complete_platform_results)
    with ENDPOINT_POLICY_CASES.open(encoding="utf-8", newline="") as policy_file:
        policy_rows = list(csv.DictReader(policy_file, delimiter="\t"))
    if len(policy_rows) != 15:
        raise AssertionError("shared endpoint policy matrix must have 15 cases")
    for row in policy_rows:
        expected = {"true": True, "false": False}.get(row["disallowed"])
        if expected is None:
            raise AssertionError("endpoint policy row has no boolean outcome")
        device_id = row["device_id"]
        observed = endpoint_is_disallowed(
            row["platform"], row["name"], device_id, row["device_type"]
        )
        if observed != expected:
            raise AssertionError(
                f"wrong endpoint policy for {row['platform']} {row['name']} "
                f"({device_id})"
            )

    realtime_warning = (
        valid.replace("# os=linux", "# os=windows")
        .replace(
            "# output_callback_priority_path="
            "cpal-alsa-helper-noop-without-realtime-dbus",
            "# output_callback_priority_path=cpal-wasapi-promotion-attempt",
        )
        .replace(
            "# output_callback_priority_observation="
            "no-cpal-promotion-by-build-contract",
            "# output_callback_priority_observation=unobserved",
        )
        .replace(
            "# output_realtime_denied_count=0",
            "# output_realtime_denied_count=1",
        )
        .replace(
            "summary,output,0,,,,,,,,,,0,0,0,0,0,0",
            "summary,output,0,,,,,,,,,,1,0,0,0,0,0",
        )
    )
    warning_results = analyze_text(
        pathlib.Path("synthetic-realtime-warning.csv"), realtime_warning, 20
    )
    if warning_results[0].f4_outcome == "Not supported":
        raise AssertionError("RealtimeDenied warning was treated as a fatal stream error")

    mutations = {
        "reversed timestamp": valid.replace(
            "callback,output,10,128,2,48000,13333333",
            "callback,output,10,128,2,48000,100",
        ),
        "endpoint direction": valid.replace(
            "callback,output,0,128,2,48000,0,2000000",
            "callback,output,0,128,2,48000,100,0",
        ),
        "reversed endpoint timestamp": valid.replace(
            "callback,output,10,128,2,48000,13333333,15333333",
            "callback,output,10,128,2,48000,13333333,13500000",
        ),
        "reversed bridge timestamp": valid.replace(
            "bridge,output,1,,,,,,,10000000,10001000,10002000",
            "bridge,output,1,,,,,,,10000000,500,10002000",
        ),
        "missing sequence": valid.replace("callback,output,10,", "callback,output,11,"),
        "duplicate sequence": valid.replace("callback,output,10,", "callback,output,9,"),
        "inconsistent frame position": valid.replace(
            "callback,output,10,128,2,48000,13333333,15333333,640",
            "callback,output,10,128,2,48000,13333333,15333333,641",
        ),
        "malformed frame count": valid.replace(
            "callback,output,0,128,2,", "callback,output,0,127,2,"
        ),
        "inconsistent error summary": valid.replace(
            "summary,output,0,,,,,,,,,,0,0,0,0,0,0",
            "summary,output,0,,,,,,,,,,0,0,1,0,0,0",
        ),
    }
    expected_failures = {
        "reversed timestamp": FalsifierTriggered,
        "endpoint direction": FalsifierTriggered,
        "reversed endpoint timestamp": FalsifierTriggered,
        "reversed bridge timestamp": FalsifierTriggered,
        "missing sequence": InvalidArtifact,
        "duplicate sequence": InvalidArtifact,
        "inconsistent frame position": InvalidArtifact,
        "malformed frame count": FalsifierTriggered,
        "inconsistent error summary": InvalidArtifact,
    }
    for label, mutation in mutations.items():
        try:
            _, mutated_rows = parse_text(mutation)
            analyze_direction(
                pathlib.Path("synthetic.csv"),
                "linux",
                "output",
                mutated_rows,
                20,
                64,
                0,
                0,
            )
        except (InvalidArtifact, FalsifierTriggered) as error:
            if not isinstance(error, expected_failures[label]):
                raise AssertionError(
                    f"analyzer misclassified the {label} mutation as {type(error).__name__}"
                ) from error
            continue
        raise AssertionError(f"analyzer accepted the {label} mutation")

    artifact_mutations = {
        "missing freshness bound": valid.replace(
            "# output_stream_now_freshness_bound_frames=0\n", ""
        ),
        "wrong freshness source": valid.replace(
            "synthetic-continuous-clock-control",
            f"cpal-{CPAL_VERSION}-alsa-one-negotiated-period-source-audit",
        ),
        "unmarked synthetic control": valid.replace(
            "# control_fixture=synthetic-continuous-clock",
            "# control_fixture=none",
        ),
        "unreviewed macOS freshness": (
            valid.replace("# os=linux", "# os=macos")
            .replace("# host=synthetic", "# host=coreaudio")
            .replace(
                "# control_fixture=synthetic-continuous-clock",
                "# control_fixture=none",
            )
            .replace(
                "# output_callback_priority_path="
                "cpal-alsa-helper-noop-without-realtime-dbus",
                "# output_callback_priority_path=coreaudio-backend-managed",
            )
            .replace(
                "# output_callback_priority_observation="
                "no-cpal-promotion-by-build-contract",
                "# output_callback_priority_observation=unobserved",
            )
        ),
        "unreviewed Windows freshness": (
            valid.replace("# os=linux", "# os=windows")
            .replace("# host=synthetic", "# host=wasapi")
            .replace(
                "# control_fixture=synthetic-continuous-clock",
                "# control_fixture=none",
            )
            .replace(
                "# output_callback_priority_path="
                "cpal-alsa-helper-noop-without-realtime-dbus",
                "# output_callback_priority_path=cpal-wasapi-promotion-attempt",
            )
            .replace(
                "# output_callback_priority_observation="
                "no-cpal-promotion-by-build-contract",
                "# output_callback_priority_observation=unobserved",
            )
        ),
        "virtual endpoint": valid.replace(
            "# output_device_type=Speaker", "# output_device_type=Virtual"
        ),
        "callback target below minimum": valid.replace(
            "# callback_target=20", "# callback_target=19"
        ),
        "insufficient callbacks": remove_unique_row(
            valid, "callback,output,19,"
        ),
        "insufficient bridges": remove_unique_row(
            valid, f"bridge,output,{MINIMUM_BRIDGES - 1},"
        ),
        "truncated summary": valid.replace(
            "summary,output,0,,,,,,,,,,0,0,0,0,0,0",
            "summary,output,0",
        ),
    }
    for label, mutation in artifact_mutations.items():
        try:
            analyze_text(pathlib.Path("synthetic.csv"), mutation, 20)
        except InvalidArtifact:
            continue
        raise AssertionError(f"analyzer accepted the {label} mutation")

    wide_bridge = valid.replace(
        "bridge,output,0,,,,,,,0,1000,2000",
        "bridge,output,0,,,,,,,0,1000,3000000",
    )
    _, wide_bridge_rows = parse_text(wide_bridge)
    wide_result = analyze_direction(
        pathlib.Path("synthetic.csv"),
        "linux",
        "output",
        wide_bridge_rows,
        20,
        64,
        0,
        0,
    )
    f4_negative_results = [("wide-bracket", wide_result)]

    wide_fit = valid.replace(
        "bridge,output,500,,,,,,,5000000000,5000001000,5000002000",
        "bridge,output,500,,,,,,,5000000000,5002001000,5000002000",
    ).replace(
        "bridge,output,600,,,,,,,6000000000,6000001000,6000002000",
        "bridge,output,600,,,,,,,6000000000,6002001000,6000002000",
    )
    _, wide_fit_rows = parse_text(wide_fit)
    wide_fit_result = analyze_direction(
        pathlib.Path("synthetic.csv"),
        "linux",
        "output",
        wide_fit_rows,
        20,
        64,
        0,
        0,
    )
    f4_negative_results.append(("wide-fit", wide_fit_result))
    for label, result in f4_negative_results:
        if result.f4_outcome != "Not supported" or "F4:" not in result.falsifiers:
            raise AssertionError(f"analyzer did not report the {label} F4 outcome")
    mutation_count = len(mutations) + len(artifact_mutations)
    print(
        "EVD-0016 analyzer controls passed "
        f"(valid single-direction + duplex positive + {mutation_count} classified "
        f"mutations + {len(f4_negative_results)} F4 negative outcomes + "
        f"RealtimeDenied warning + release coverage + {len(policy_rows)} endpoint cases)."
    )


def print_results(results: list[DirectionResult]) -> None:
    writer = csv.writer(sys.stdout, lineterminator="\n")
    writer.writerow(DirectionResult.__dataclass_fields__.keys())
    for result in results:
        writer.writerow(
            [
                result.artifact,
                result.platform,
                result.direction,
                result.callbacks,
                result.sample_rate_hz,
                result.quantum_frames,
                result.minimum_frames,
                result.maximum_frames,
                f"{result.effective_rate_hz:.6f}",
                f"{result.drift_ppm:.3f}",
                f"{result.fit_residual_p999_frames:.3f}",
                f"{result.period_residual_p999_frames:.3f}",
                f"{result.bridge_half_width_max_frames:.3f}",
                f"{result.bridge_fit_residual_p999_frames:.3f}",
                result.stream_now_freshness_bound_frames,
                f"{result.latency_p50_frames:.3f}",
                f"{result.latency_p999_frames:.3f}",
                result.bridged_uncertainty_frames,
                result.duplex_uncertainty_frames,
                result.realtime_denied_count,
                result.f4_outcome,
                result.falsifiers,
            ]
        )


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("artifacts", nargs="*", type=pathlib.Path)
    parser.add_argument("--require-release-platforms", action="store_true")
    parser.add_argument("--self-test", action="store_true")
    arguments = parser.parse_args()
    if arguments.self_test:
        self_test()
    results: list[DirectionResult] = []
    for artifact in arguments.artifacts:
        results.extend(analyze_artifact(artifact))
    if arguments.require_release_platforms:
        require_release_platforms(results)
    if results:
        print_results(results)
    if not arguments.self_test and not results:
        parser.error("provide --self-test and/or at least one artifact")
    return int(any(result.f4_outcome == "Not supported" for result in results))


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except InvalidArtifact as error:
        print(f"EVD-0016 invalid: {error}", file=sys.stderr)
        raise SystemExit(1) from error
    except FalsifierTriggered as error:
        print(f"EVD-0016 not supported: {error}", file=sys.stderr)
        raise SystemExit(1) from error
