"""In-model coverage plus explicit counterexamples to unwarranted clock claims."""
from dataclasses import replace
from fractions import Fraction as F
import math
import unittest
import subprocess
import sys
import json
from pathlib import Path
from scripts.clock_interval_fit import (
    SyntheticAffineFit, SyntheticReport, RateHertz, DriftPpm, FrameAlignment,
    FramePosition, Nanoseconds, Refused, ScenarioArm, ScenarioSource, ScenarioClock,
    midpoint_control, owned_scenario, Line, Corner, clip,
)


def report(sequence, time, position, alignment=0, age=0):
    return SyntheticReport(ScenarioArm.PRIMARY, ScenarioSource.PRIMARY, ScenarioClock.OBSERVER, sequence, Nanoseconds(time),
                           Nanoseconds(time), Nanoseconds(age), FramePosition(position), FrameAlignment(alignment))


def fresh(ppm=200):
    return SyntheticAffineFit(RateHertz(48_000), DriftPpm(ppm), Nanoseconds(2_000_000_000))


class ClockIntervalFitTests(unittest.TestCase):
    def test_coverage_at_four_rates_both_drift_extremes_and_uncertainty_boundaries(self):
        for rate in (44_100, 48_000, 88_200, 96_000):
            for ppm in (-200, 0, 200):
                for age in (0, 200_000):
                    for alignment in (0, 3):
                        fit, slope, intercept = owned_scenario(rate, ppm, age, alignment)
                        for horizon in (0, 1_000_000, 1_000_000_000, 2_000_000_000):
                            event = Nanoseconds(fit.reports[-1].after.value + horizon)
                            self.assertTrue(fit.predict(event, event, event).contains(slope * event.value + intercept))

    def test_closed_quantization_boundary_and_zero_drift_prior(self):
        fit = fresh(0)
        fit.add(report(0, 1_000_000_000, 48_000))
        event = Nanoseconds(1_001_000_000)
        interval = fit.predict(event, event, event)
        self.assertTrue(interval.contains(F(48_048)))
        self.assertTrue(interval.contains(F(48_049)))  # Conservatively closed floor upper edge.

    def test_negative_midpoint_controls_both_exclude_truth_under_same_sign_age(self):
        for rate in (44_100, 48_000, 88_200, 96_000):
            fit, slope, intercept = owned_scenario(rate)
            event = Nanoseconds(fit.reports[-1].after.value + 1_000_000_000)
            truth = slope * event.value + intercept
            for free_slope in (False, True):
                low, high = midpoint_control(fit.reports, event, RateHertz(rate), free_slope)
                self.assertFalse(low <= truth <= high)
            self.assertTrue(fit.predict(event, event, event).contains(truth))

    def test_nesting_from_additional_reports_and_pessimistic_horizon_growth(self):
        fit = fresh()
        event = Nanoseconds(2_900_000_000)
        previous = None
        for sequence in range(8):
            time = 1_000_000_000 + sequence * 200_000_000
            fit.add(report(sequence, time, 1_000_000 + time * 48_000 // 10**9, 3, 200_000))
            interval = fit.predict(event, event, event)
            if previous is not None:
                self.assertGreaterEqual(interval.lower, previous.lower)
                self.assertLessEqual(interval.upper, previous.upper)
            previous = interval
        widths = []
        for horizon in (0, 100_000_000, 500_000_000, 1_000_000_000):
            at = Nanoseconds(fit.reports[-1].after.value + horizon)
            widths.append(fit.predict(at, at, at).width)
        self.assertEqual(widths, sorted(widths))

    def test_future_reports_pre_start_events_and_excess_horizon_are_refused(self):
        fit, _, _ = owned_scenario()
        latest = fit.reports[-1].after.value
        for lo, hi, decision in [(latest, latest, latest - 1), (0, latest, latest),
                                 (latest, latest - 1, latest),
                                 (latest + 2_000_000_001, latest + 2_000_000_001, latest + 2_000_000_001)]:
            with self.assertRaises(Refused):
                fit.predict(Nanoseconds(lo), Nanoseconds(hi), Nanoseconds(decision))

    def test_all_clock_metadata_and_raw_order_faults_permanently_stop_the_scenario(self):
        first = report(0, 1_000_000_000, 48_000)
        second = report(1, 1_001_000_000, 48_048)
        changes = [dict(age=None), dict(alignment=None), dict(arm=ScenarioArm.RESTARTED), dict(source=ScenarioSource.OTHER),
                   dict(clock=ScenarioClock.OTHER), dict(sequence=3), dict(position=FramePosition(1)),
                   dict(before=Nanoseconds(999_999_999)), dict(after=Nanoseconds(999_999_999))]
        for change in changes:
            fit = fresh()
            fit.add(first)
            with self.assertRaises(Refused): fit.add(replace(second, **change))
            with self.assertRaises(Refused): fit.predict(second.after, second.after, second.after)
            with self.assertRaises(Refused): fit.add(second)
        fit = fresh()
        with self.assertRaises(Refused): fit.add(replace(first, sequence=1))

    def test_repeated_frame_positions_are_mathematically_valid_but_not_hardware_readiness(self):
        fit = fresh()
        fit.add(report(0, 1_000_000_000, 48_000, alignment=64))
        fit.add(report(1, 1_000_100_000, 48_000, alignment=64))
        self.assertTrue(fit.polygon)

    def test_a_single_underdeclared_bound_can_exclude_truth_without_an_empty_model(self):
        fit = fresh(0)
        for sequence in range(3):
            time = (sequence + 1) * 1_000_000_000
            true_frame = 1_000_000 + 48_000 * (sequence + 1)
            fit.add(report(sequence, time, true_frame + (50 if sequence == 1 else 0), 0 if sequence == 1 else 100))
        at = Nanoseconds(3_000_000_000)
        self.assertTrue(fit.polygon)
        self.assertFalse(fit.predict(at, at, at).contains(F(1_144_000)))

    def test_unbridged_clock_offset_is_not_detected_by_feasibility(self):
        fit = fresh(0)
        for sequence in range(3):
            observer = (sequence + 1) * 1_000_000_000
            # Incorrectly substituting a report clock that is 1 ms ahead violates the axis guarantee.
            fit.add(report(sequence, observer + 1_000_000, 1_000_000 + observer * 48_000 // 10**9))
        at = Nanoseconds(3_002_000_000)
        self.assertTrue(fit.polygon)
        self.assertFalse(fit.predict(at, at, at).contains(F(1_000_000) + F(48_000, 10**9) * at.value))

    def test_unseen_curvature_inside_slope_prior_excludes_truth_after_a_long_linear_prefix(self):
        fit = fresh(200)
        for sequence, seconds in enumerate((0, 10, 20)):
            fit.add(report(sequence, 1_000_000_000 + seconds * 10**9, 1_000_000 + 48_000 * seconds))
        at = Nanoseconds(22_000_000_000)  # One second after the last report.
        predicted = fit.predict(at, at, at)
        # f(t) after the prefix = linear + 4*(t-20)^2. Its derivative reaches
        # 48008 frames/s, still inside the declared +/-200 ppm slope prior.
        self.assertLessEqual(F(48_008, 10**9), fit.maximum)
        self.assertGreaterEqual(F(48_000, 10**9), fit.minimum)
        self.assertTrue(fit.polygon)
        self.assertFalse(predicted.contains(F(1_000_000 + 48_000 * 21 + 4)))
        # A separate undeclared forward jump is likewise invisible before a new report.
        self.assertFalse(predicted.contains(F(1_000_000 + 48_000 * 21 + 20)))

    def test_non_affine_reports_with_slopes_inside_prior_refuse_further_predictions(self):
        fit = fresh(200)
        fit.add(report(0, 1_000_000_000, 1_000_000))
        at = Nanoseconds(1_001_000_000)
        self.assertTrue(fit.predict(at, at, at).contains(F(1_000_048)))
        # f(t) = 1000000 + 48000*t + (12/25)*t^2 for elapsed seconds 0..10.
        # Every instantaneous slope stays within nominal..+200 ppm; three
        # non-collinear reports isolate curvature from a slope-prior violation.
        for seconds in (0, 5, 10):
            derivative = F(48_000) + F(24, 25) * seconds
            self.assertTrue(fit.minimum <= derivative / 10**9 <= fit.maximum)
        fit.add(report(1, 6_000_000_000, 1_000_000 + 48_000 * 5 + 12))
        with self.assertRaises(Refused):
            fit.add(report(2, 11_000_000_000, 1_000_000 + 48_000 * 10 + 48))
        with self.assertRaises(Refused):
            fit.predict(Nanoseconds(6_000_000_000), Nanoseconds(6_000_000_000), Nanoseconds(6_000_000_000))

    def test_drift_just_outside_prior_hides_in_short_prefix_but_refuses_over_long_span(self):
        fit = SyntheticAffineFit(RateHertz(48_000), DriftPpm(200), Nanoseconds(200 * 10**9))
        truth_slope = F(48_000 * 1_000_210, 10**15)  # +210 ppm, ten beyond the prior.
        for sequence in range(3):
            elapsed = sequence * 1_000_000
            fit.add(report(sequence, 1_000_000_000 + elapsed, math.floor(1_000_000 + truth_slope * elapsed)))
        at = Nanoseconds(101_000_000_000)
        truth = 1_000_000 + truth_slope * (at.value - 1_000_000_000)
        self.assertFalse(fit.predict(at, at, at).contains(truth))
        with self.assertRaises(Refused): fit.add(report(3, at.value, math.floor(truth)))

    def test_wrong_nominal_rate_is_refused_after_informative_span(self):
        fit = fresh()
        fit.add(report(0, 1_000_000_000, 44_100))
        with self.assertRaises(Refused): fit.add(report(1, 2_000_000_000, 88_200))

    def test_widening_event_uncertainty_never_narrows_the_prediction(self):
        fit, slope, intercept = owned_scenario()
        center = fit.reports[-1].after.value + 1_000_000
        widths = []
        for uncertainty in (0, 1_000, 100_000):
            result = fit.predict(Nanoseconds(center - uncertainty), Nanoseconds(center + uncertainty), Nanoseconds(center - uncertainty))
            self.assertTrue(result.contains(slope * center + intercept))
            widths.append(result.width)
        self.assertEqual(widths, sorted(widths))

    def test_command_has_no_physical_input_mode_or_qualification_output(self):
        script = Path(__file__).with_name("clock_interval_fit.py")
        result = subprocess.run([sys.executable, "-B", str(script)], capture_output=True, text=True, check=True)
        output = json.loads(result.stdout)
        self.assertIs(output["synthetic"], True)
        self.assertIs(output["physical_qualification"], False)
        self.assertEqual({row["rate_hz"]: row["interval_width_frames"] for row in output["scenarios"]},
                         {44_100: 28, 48_000: 29, 88_200: 45, 96_000: 48})
        self.assertTrue(all(row["truth_covered"] and row["midpoint_nominal_excludes_truth"]
                            and row["midpoint_free_excludes_truth"] for row in output["scenarios"]))
        result = subprocess.run([sys.executable, "-B", str(script), "physical.json"], capture_output=True, text=True)
        self.assertEqual(result.returncode, 2)
        self.assertNotIn("physical_qualification", result.stdout)

    def test_closed_clipping_retains_a_feasible_segment_and_a_single_point(self):
        left, right = Line(F(-1), F(0), F(0)), Line(F(1), F(0), F(1))
        bottom, top = Line(F(0), F(-1), F(0)), Line(F(0), F(1), F(1))
        polygon = [Corner((F(0), F(0)), left), Corner((F(1), F(0)), bottom),
                   Corner((F(1), F(1)), right), Corner((F(0), F(1)), top)]
        segment = clip(polygon, Line(F(1), F(0), F(0)))
        self.assertTrue(segment)
        self.assertTrue(all(corner.point[0] == 0 for corner in segment))
        point = clip(segment, Line(F(0), F(1), F(0)))
        self.assertTrue(point)
        self.assertTrue(all(corner.point == (0, 0) for corner in point))

    def test_reversal_repeated_bracket_and_missing_identity_guards_are_independent(self):
        first = report(0, 1_000_000_000, 48_000, alignment=100)
        cases = [replace(report(1, 1_001_000_000, 47_999, alignment=100)),
                 replace(first, sequence=1)]
        for second in cases:
            fit = fresh()
            fit.add(first)
            with self.assertRaises(Refused): fit.add(second)
            self.assertTrue(fit.failed)
        for field in ("arm", "source", "clock"):
            fit = fresh()
            with self.assertRaises(Refused): fit.add(replace(first, **{field: None}))
            with self.assertRaises(Refused): fit.add(first)

    def test_wrong_unit_or_malformed_reports_permanently_invalidate_the_scenario(self):
        valid = report(0, 1_000_000_000, 48_000)
        for field, value in (("before", FramePosition(1)), ("after", 12),
                             ("age", FrameAlignment(0)), ("position", Nanoseconds(48_000)),
                             ("alignment", Nanoseconds(0))):
            fit = fresh()
            with self.assertRaises(Refused): fit.add(replace(valid, **{field: value}))
            with self.assertRaises(Refused): fit.add(valid)
        fit = fresh()
        with self.assertRaises(Refused): fit.add(None)
        with self.assertRaises(Refused): fit.add(valid)

    def test_interpolation_and_events_before_decision_are_refused(self):
        fit, _, _ = owned_scenario()
        latest = fit.reports[-1].after
        for event in (fit.reports[0].after, Nanoseconds(latest.value - 1)):
            with self.assertRaises(Refused): fit.predict(event, event, latest)

    def test_report_bound_and_numeric_invariants(self):
        fit = fresh(0)
        for sequence in range(128):
            t = (sequence + 1) * 1_000_000_000
            fit.add(report(sequence, t, (sequence + 1) * 48_000))
        with self.assertRaises(Refused): fit.add(report(128, 129_000_000_000, 129 * 48_000))
        for cls, values in [(RateHertz, (0, -1, True)), (DriftPpm, (-1, 1_000_000, True)),
                            (FrameAlignment, (-1, True))]:
            for value in values:
                with self.assertRaises(ValueError): cls(value)


if __name__ == "__main__": unittest.main()
