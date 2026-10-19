#!/usr/bin/env python3
"""Observation-only readiness check for ADR-0022's replacement clock candidate.

This consumes owned reports extracted from one backend's existing status queries.
It never opens a stream, estimates latency, or accepts a hardware time mapping.
The old EVD-0016 collector and analyzer are deliberately independent.
"""

from __future__ import annotations

import argparse
import json
from dataclasses import dataclass, fields
from enum import Enum
from pathlib import Path


@dataclass(frozen=True, order=True)
class Nanoseconds:
    value: int

    def __post_init__(self) -> None:
        if type(self.value) is not int or self.value < 0:
            raise ValueError("nanoseconds must be a nonnegative integer")


@dataclass(frozen=True, order=True)
class FramePosition:
    value: int

    def __post_init__(self) -> None:
        if type(self.value) is not int or self.value < 0:
            raise ValueError("frame position must be a nonnegative integer")


class Verdict(str, Enum):
    OBSERVABLE = "Observable"
    INCONCLUSIVE = "Inconclusive"
    STOP = "Stop"


class ClockDomain(str, Enum):
    GETTIMEOFDAY = "Gettimeofday"
    MONOTONIC = "Monotonic"
    MONOTONIC_RAW = "MonotonicRaw"


class TimestampType(str, Enum):
    COMPAT = "Compat"
    DEFAULT = "Default"
    LINK = "Link"
    LINK_ABSOLUTE = "LinkAbsolute"
    LINK_ESTIMATED = "LinkEstimated"
    LINK_SYNCHRONIZED = "LinkSynchronized"


class FrameDerivation(str, Enum):
    STATUS_HARDWARE_POSITION = "status-hardware-position"
    SYNTHETIC = "synthetic-frame-counter"
    CALLBACK_COUNT = "callback-count"


@dataclass(frozen=True)
class ClockReport:
    # All keys are required. Null explicitly records unavailable metadata.
    arm: str
    direction: str
    sequence: int
    sample_rate_hz: int
    requested_clock: ClockDomain
    actual_clock: ClockDomain | None
    requested_type: TimestampType
    actual_type: TimestampType | None
    frame_derivation: FrameDerivation | None
    status_ok: bool
    running: bool
    report_valid: bool
    accuracy_valid: bool
    trigger_origin_valid: bool
    fallback: str | None
    clamped_negative: bool
    timestamp_ns: Nanoseconds | None
    system_timestamp_ns: Nanoseconds | None
    driver_timestamp_ns: Nanoseconds | None
    trigger_timestamp_ns: Nanoseconds | None
    accuracy_ns: Nanoseconds | None
    clock_age_bound_ns: Nanoseconds | None
    frame_alignment_bound_ns: Nanoseconds | None
    observer_before_ns: Nanoseconds
    observer_after_ns: Nanoseconds
    frame_position: FramePosition | None
    loss_count: int
    error_count: int

    @classmethod
    def from_dict(cls, value: object) -> ClockReport:
        if not isinstance(value, dict) or set(value) != {f.name for f in fields(cls)}:
            raise ValueError("clock report has unknown or missing fields")
        data = dict(value)
        for key in ("arm", "direction", "requested_clock", "requested_type"):
            if not isinstance(data[key], str) or not data[key]:
                raise ValueError(f"{key} must be a nonempty string")
        if data["direction"] not in ("input", "output"):
            raise ValueError("direction must be input or output")
        for key in ("actual_clock", "actual_type", "frame_derivation", "fallback"):
            if data[key] is not None and (not isinstance(data[key], str) or not data[key]):
                raise ValueError(f"{key} must be null or a nonempty string")
        for key, enum in (("requested_clock", ClockDomain), ("actual_clock", ClockDomain),
                          ("requested_type", TimestampType), ("actual_type", TimestampType),
                          ("frame_derivation", FrameDerivation)):
            if data[key] is not None:
                data[key] = enum(data[key])
        for key in ("sequence", "sample_rate_hz", "loss_count", "error_count"):
            if type(data[key]) is not int or data[key] < 0:
                raise ValueError(f"{key} must be a nonnegative integer")
        if data["sample_rate_hz"] == 0:
            raise ValueError("sample_rate_hz must be positive")
        for key in ("status_ok", "running", "report_valid", "accuracy_valid",
                    "trigger_origin_valid", "clamped_negative"):
            if type(data[key]) is not bool:
                raise ValueError(f"{key} must be boolean")
        for key in data:
            if key.endswith("_ns"):
                if data[key] is None and key in ("observer_before_ns", "observer_after_ns"):
                    raise ValueError(f"{key} must be present")
                if data[key] is not None:
                    data[key] = Nanoseconds(data[key])
        if data["frame_position"] is not None:
            data["frame_position"] = FramePosition(data["frame_position"])
        return cls(**data)


@dataclass(frozen=True)
class Readiness:
    verdict: Verdict
    reasons: tuple[str, ...]
    reports: int


def assess(reports: list[ClockReport]) -> Readiness:
    """Inspect every record, including startup. Stop dominates missing metadata.

    An arm is one duplex generation. Restart/recovery requires a new arm and a
    separate assessment; it cannot hide a discontinuity inside an accepted run.
    Even complete reports establish observability only, never F4 precision.
    """
    incomplete: list[str] = []
    stopped: list[str] = []
    previous: dict[str, ClockReport] = {}
    last_known: dict[tuple[str, str], object] = {}
    arm = reports[0].arm if reports else None
    for report in reports:
        tag = f"{report.direction}/{report.sequence}"
        if report.arm != arm:
            stopped.append(f"{tag}: arm changed; assess recovery separately")
        if not report.status_ok or not report.running:
            stopped.append(f"{tag}: status failed or stream is not running")
        if report.clamped_negative:
            stopped.append(f"{tag}: backend clamped a negative clock difference")
        if report.loss_count or report.error_count:
            stopped.append(f"{tag}: lost reports or backend errors")
        if report.observer_after_ns < report.observer_before_ns:
            stopped.append(f"{tag}: backward observer bracket")
        if not report.report_valid:
            incomplete.append(f"{tag}: timestamp report invalid or absent (including Compat)")
        if not report.accuracy_valid:
            incomplete.append(f"{tag}: accuracy not attested by backend report")
        if not report.trigger_origin_valid:
            stopped.append(f"{tag}: invalid trigger origin")
        if report.actual_type == TimestampType.COMPAT:
            incomplete.append(f"{tag}: timestamp source is not individually attested")
        if report.frame_derivation in (FrameDerivation.CALLBACK_COUNT, FrameDerivation.SYNTHETIC):
            incomplete.append(f"{tag}: {report.frame_derivation.value} is not hardware frame correspondence")
        if report.fallback is not None:
            incomplete.append(f"{tag}: explicit backend fallback: {report.fallback}")
        for key in (
            "actual_clock", "actual_type", "timestamp_ns", "system_timestamp_ns", "driver_timestamp_ns",
            "trigger_timestamp_ns", "accuracy_ns", "clock_age_bound_ns",
            "frame_alignment_bound_ns", "frame_position", "frame_derivation",
        ):
            if getattr(report, key) is None:
                incomplete.append(f"{tag}: missing {key}")
        if report.actual_clock != report.requested_clock:
            incomplete.append(f"{tag}: requested clock was not returned")
        if report.actual_type != report.requested_type:
            incomplete.append(f"{tag}: requested timestamp type was not returned")
        # Audio time can be relative to the trigger while the trigger timestamp
        # is absolute. Their numeric values must never be ordered against each
        # other. The backend's explicit origin-validity flag owns that check.
        old = previous.get(report.direction)
        if old is not None:
            if report.sequence != old.sequence + 1:
                stopped.append(f"{tag}: duplicate, reordered or missing sequence")
            if report.observer_before_ns < old.observer_after_ns:
                stopped.append(f"{tag}: observer clock moved backward or queries overlap")
        elif report.sequence != 0:
            stopped.append(f"{tag}: startup prefix missing")
        for key in ("timestamp_ns", "system_timestamp_ns", "driver_timestamp_ns", "frame_position",
                    "actual_clock", "actual_type", "requested_clock", "requested_type",
                    "sample_rate_hz", "frame_derivation", "trigger_timestamp_ns"):
            value = getattr(report, key)
            if value is None:
                continue
            binding = report.direction, key
            old_value = last_known.get(binding)
            if old_value is not None:
                if key in ("timestamp_ns", "system_timestamp_ns", "driver_timestamp_ns", "frame_position"):
                    if value <= old_value:
                        stopped.append(f"{tag}: stale, repeated or backward {key}")
                elif value != old_value:
                    stopped.append(f"{tag}: clock source, frame derivation, rate changed or trigger origin changed ({key})")
            last_known[binding] = value
        previous[report.direction] = report
    for direction in ("input", "output"):
        count = sum(report.direction == direction for report in reports)
        if count < 2:
            incomplete.append(f"{direction}: two advancing observations required")
    reasons = tuple(stopped + incomplete)
    verdict = Verdict.STOP if stopped else Verdict.INCONCLUSIVE if incomplete else Verdict.OBSERVABLE
    return Readiness(verdict, reasons, len(reports))


def load_artifact(text: str) -> list[ClockReport]:
    def unique_object(pairs: list[tuple[str, object]]) -> dict[str, object]:
        result: dict[str, object] = {}
        for key, value in pairs:
            if key in result:
                raise ValueError(f"duplicate JSON field: {key}")
            result[key] = value
        return result

    def invalid_number(value: str) -> None:
        raise ValueError(f"non-finite JSON number: {value}")

    value = json.loads(text, object_pairs_hook=unique_object, parse_constant=invalid_number)
    if not isinstance(value, dict) or set(value) != {"version", "reports"}:
        raise ValueError("artifact must contain exactly version and reports")
    if type(value["version"]) is not int or value["version"] != 1:
        raise ValueError("unsupported clock-report artifact version")
    if not isinstance(value["reports"], list):
        raise ValueError("reports must be an array")
    return [ClockReport.from_dict(report) for report in value["reports"]]


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("artifact", type=Path)
    args = parser.parse_args()
    try:
        result = assess(load_artifact(args.artifact.read_text(encoding="utf-8")))
    except (OSError, ValueError, RecursionError) as error:
        parser.exit(2, f"Invalid artifact: {error}\n")
    print(json.dumps({"verdict": result.verdict.value, "reports": result.reports,
                      "reasons": result.reasons, "physical_qualification": False}, sort_keys=True))
    return 0 if result.verdict == Verdict.OBSERVABLE else 1


if __name__ == "__main__":
    raise SystemExit(main())
