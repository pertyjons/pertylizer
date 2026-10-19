#!/usr/bin/env python3
"""Owned synthetic affine-clock investigation. No report loader or live consumer.

Coverage is conditional on hard bounds and an affine clock inside the declared
slope prior. A feasible model does not validate either assumption. This program
cannot produce physical timing evidence or an F4 qualification.
"""
from __future__ import annotations

from dataclasses import dataclass
from fractions import Fraction as F
import json
import argparse
import math
from enum import Enum

if __package__ in (None, ""):
    from clock_observability import Nanoseconds, FramePosition
else:
    from .clock_observability import Nanoseconds, FramePosition


class ScenarioArm(Enum):
    PRIMARY = "owned-arm"
    RESTARTED = "owned-restart"


class ScenarioSource(Enum):
    PRIMARY = "owned-source"
    OTHER = "owned-other-source"


class ScenarioClock(Enum):
    OBSERVER = "owned-observer-clock"
    OTHER = "owned-other-clock"


@dataclass(frozen=True)
class RateHertz:
    value: int
    def __post_init__(self) -> None:
        if type(self.value) is not int or self.value <= 0:
            raise ValueError("rate must be a positive integer")


@dataclass(frozen=True)
class DriftPpm:
    value: int
    def __post_init__(self) -> None:
        if type(self.value) is not int or not 0 <= self.value < 1_000_000:
            raise ValueError("drift must be an integer in [0, 1000000)")


@dataclass(frozen=True)
class FrameAlignment:
    value: int
    def __post_init__(self) -> None:
        if type(self.value) is not int or self.value < 0:
            raise ValueError("alignment must be a nonnegative frame count")


@dataclass(frozen=True)
class SyntheticReport:
    """Observer-axis interval, explicitly excluding any report-clock timestamp.

    At some observer time in [before-age, after], the actual frame position is
    in [position-alignment, position+1+alignment]. Position uses floor; the
    closed upper endpoint conservatively includes the excluded integer edge.
    All bounds are hard guarantees of the owned generator, never quantiles.
    """
    arm: ScenarioArm
    source: ScenarioSource
    clock: ScenarioClock
    sequence: int
    before: Nanoseconds
    after: Nanoseconds
    age: Nanoseconds | None
    position: FramePosition
    alignment: FrameAlignment | None


class Refused(ValueError):
    """No interval may be used for this prediction."""


@dataclass(frozen=True)
class FrameInterval:
    lower: FramePosition
    upper: FramePosition
    def __post_init__(self) -> None:
        if not isinstance(self.lower, FramePosition) or not isinstance(self.upper, FramePosition):
            raise ValueError("frame interval requires frame positions")
        if self.lower > self.upper:
            raise ValueError("frame interval is reversed")
    def contains(self, truth: F) -> bool:
        return self.lower.value <= truth <= self.upper.value
    @property
    def width(self) -> int:
        return self.upper.value - self.lower.value


@dataclass(frozen=True)
class Line:
    """Original half-plane coefficients: a*slope + b*intercept <= limit."""
    a: F
    b: F
    limit: F
    def residual(self, point: tuple[F, F]) -> F:
        return self.a * point[0] + self.b * point[1] - self.limit
    def cross(self, other: Line) -> tuple[F, F]:
        determinant = self.a * other.b - other.a * self.b
        if determinant == 0:
            raise Refused("parallel boundaries cannot define a corner")
        return ((self.limit * other.b - other.limit * self.b) / determinant,
                (self.a * other.limit - other.a * self.limit) / determinant)


@dataclass(frozen=True)
class Corner:
    point: tuple[F, F]
    incoming: Line


def clip(polygon: list[Corner], boundary: Line) -> list[Corner]:
    """Intersect using original line pairs, never approximate clipped vertices."""
    result: list[Corner] = []
    if not polygon:
        return result
    start = polygon[-1]
    for end in polygon:
        start_inside = boundary.residual(start.point) <= 0
        end_inside = boundary.residual(end.point) <= 0
        if end_inside:
            if not start_inside:
                result.append(Corner(boundary.cross(end.incoming), boundary))
            result.append(end)
        elif start_inside:
            result.append(Corner(boundary.cross(end.incoming), end.incoming))
        start = end
    return result


class SyntheticAffineFit:
    """One bounded owned scenario; there is intentionally no physical-data reader.

    Repeated positions are permitted mathematically. The independent hardware
    observability preflight still stops on them; this experiment does not relax it.
    """
    MAX_REPORTS = 128

    def __init__(self, rate: RateHertz, drift: DriftPpm, horizon: Nanoseconds):
        if not isinstance(rate, RateHertz) or not isinstance(drift, DriftPpm) or not isinstance(horizon, Nanoseconds):
            raise ValueError("fit requires rate, drift and a nanosecond horizon")
        self.minimum = F(rate.value * (1_000_000 - drift.value), 10**15)
        self.maximum = F(rate.value * (1_000_000 + drift.value), 10**15)
        self.horizon = horizon
        self.reports: list[SyntheticReport] = []
        self.polygon: list[Corner] = []
        self.reference = 0
        self.failed = False

    def add(self, report: SyntheticReport) -> None:
        try:
            self._add(report)
        except Refused:
            self.failed = True
            raise

    def _add(self, report: SyntheticReport) -> None:
        if self.failed:
            raise Refused("scenario already invalidated")
        if not isinstance(report, SyntheticReport):
            raise Refused("expected a synthetic report")
        if (not isinstance(report.before, Nanoseconds) or not isinstance(report.after, Nanoseconds)
                or (report.age is not None and not isinstance(report.age, Nanoseconds))
                or not isinstance(report.position, FramePosition)
                or (report.alignment is not None and not isinstance(report.alignment, FrameAlignment))):
            raise Refused("report quantity has the wrong unit")
        if len(self.reports) >= self.MAX_REPORTS:
            raise Refused("synthetic report limit")
        if report.age is None or report.alignment is None:
            raise Refused("missing hard age or alignment bound")
        if (not isinstance(report.arm, ScenarioArm) or not isinstance(report.source, ScenarioSource)
                or not isinstance(report.clock, ScenarioClock)):
            raise Refused("missing clock/source/arm identity")
        if type(report.sequence) is not int or report.sequence != len(self.reports):
            raise Refused("missing startup, gap or repeated sequence")
        if report.before.value > report.after.value or report.before.value < report.age.value:
            raise Refused("invalid observer bracket or pre-origin report interval")
        if self.reports:
            previous = self.reports[-1]
            if (report.arm, report.source, report.clock) != (previous.arm, previous.source, previous.clock):
                raise Refused("arm/source/clock change")
            if report.before.value < previous.after.value or report.after.value <= previous.after.value:
                raise Refused("non-monotonic raw observer brackets")
            if report.position < previous.position:
                raise Refused("backward raw frame position")
        else:
            self.reference = report.before.value - report.age.value
        lo = report.before.value - report.age.value - self.reference
        hi = report.after.value - self.reference
        frame_lo = report.position.value - report.alignment.value
        frame_hi = report.position.value + 1 + report.alignment.value
        if not self.reports:
            intercept_min = min(F(frame_lo) - slope * hi for slope in (self.minimum, self.maximum))
            intercept_max = max(F(frame_hi) - slope * lo for slope in (self.minimum, self.maximum))
            left, right = Line(-1, 0, -self.minimum), Line(1, 0, self.maximum)
            bottom, top = Line(0, -1, -intercept_min), Line(0, 1, intercept_max)
            self.polygon = [Corner((self.minimum, intercept_min), left),
                            Corner((self.maximum, intercept_min), bottom),
                            Corner((self.maximum, intercept_max), right),
                            Corner((self.minimum, intercept_max), top)]
        for boundary in (Line(F(lo), F(1), F(frame_hi)), Line(F(-hi), F(-1), F(-frame_lo))):
            self.polygon = clip(self.polygon, boundary)
        if not self.polygon:
            raise Refused("inconsistent affine model; all earlier predictions invalidated")
        self.reports.append(report)

    def predict(self, event_lo: Nanoseconds, event_hi: Nanoseconds,
                decision: Nanoseconds) -> FrameInterval:
        if not all(isinstance(value, Nanoseconds) for value in (event_lo, event_hi, decision)):
            raise Refused("prediction requires observer nanoseconds")
        if self.failed or not self.reports or not self.polygon:
            raise Refused("no usable model")
        if decision < self.reports[-1].after:
            raise Refused("prediction consumed a future report")
        if event_lo < decision:
            raise Refused("forward prediction cannot interpolate or precede its decision")
        if event_hi < event_lo or event_lo < self.reports[0].before:
            raise Refused("invalid or pre-start event interval")
        if event_hi.value > self.reports[-1].after.value + self.horizon.value:
            raise Refused("prediction exceeds predeclared horizon")
        low = min(slope * (event_lo.value - self.reference) + intercept for slope, intercept in (v.point for v in self.polygon))
        high = max(slope * (event_hi.value - self.reference) + intercept for slope, intercept in (v.point for v in self.polygon))
        if low < 0:
            raise Refused("possible pre-epoch frame")
        return FrameInterval(FramePosition(math.floor(low)), FramePosition(math.ceil(high)))


def midpoint_control(reports: list[SyntheticReport], event: Nanoseconds,
                     rate: RateHertz, free_slope: bool) -> tuple[F, F]:
    """Negative controls: midpoint regression with max residual, same report cut.

    This is deliberately not a guaranteed interval: unknown age biases every
    midpoint in the same direction, so a tiny residual can hide large error.
    """
    times = [F(report.before.value + report.after.value, 2) for report in reports]
    frames = [F(report.position.value) for report in reports]
    mean_t, mean_f = sum(times) / len(times), sum(frames) / len(frames)
    slope = F(rate.value, 10**9)
    if free_slope:
        denominator = sum((t - mean_t)**2 for t in times)
        if not denominator:
            raise Refused("negative control needs distinct report times")
        slope = sum((t - mean_t)*(f - mean_f) for t, f in zip(times, frames)) / denominator
    intercept = mean_f - slope * mean_t
    residual = max(abs(f - (slope*t + intercept)) for t, f in zip(times, frames))
    predicted = slope * event.value + intercept
    return predicted - residual, predicted + residual


def owned_scenario(rate: int = 48_000, ppm: int = 100, age: int = 200_000,
                   alignment: int = 3) -> tuple[SyntheticAffineFit, F, F]:
    """Pessimistic same-sign age/alignment errors; no alternating pinning pattern."""
    nominal = RateHertz(rate)
    fit = SyntheticAffineFit(nominal, DriftPpm(200), Nanoseconds(2_000_000_000))
    slope = F(rate * (1_000_000 + ppm), 10**15)
    intercept = F(1_000_000)
    for sequence in range(12):
        before = 1_000_000_000 + sequence * 250_000_000
        position = math.floor(slope * (before - age) + intercept) - alignment
        fit.add(SyntheticReport(ScenarioArm.PRIMARY, ScenarioSource.PRIMARY, ScenarioClock.OBSERVER, sequence,
                               Nanoseconds(before), Nanoseconds(before + 10_000), Nanoseconds(age),
                               FramePosition(position), FrameAlignment(alignment)))
    return fit, slope, intercept


def main() -> None:
    results = []
    for rate in (44_100, 48_000, 88_200, 96_000):
        fit, slope, intercept = owned_scenario(rate)
        event = Nanoseconds(fit.reports[-1].after.value + 1_000_000_000)
        interval = fit.predict(event, event, event)
        truth = slope * event.value + intercept
        controls = [midpoint_control(fit.reports, event, RateHertz(rate), free) for free in (False, True)]
        results.append({"rate_hz":rate, "interval_width_frames":interval.width,
                        "truth_covered":interval.contains(truth),
                        "midpoint_nominal_excludes_truth":not controls[0][0] <= truth <= controls[0][1],
                        "midpoint_free_excludes_truth":not controls[1][0] <= truth <= controls[1][1]})
    print(json.dumps({"synthetic":True, "physical_qualification":False,
                      "assumptions":"hard observer-axis bounds and constant affine drift inside declared prior",
                      "limitation":"feasibility does not validate assumptions; no physical collector or adapter consumer",
                      "scenarios":results}, indent=2))


if __name__ == "__main__":
    argparse.ArgumentParser(description=__doc__).parse_args()
    main()
