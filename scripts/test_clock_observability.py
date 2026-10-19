"""Synthetic observability controls; these do not qualify a physical clock."""

from __future__ import annotations

import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

from scripts.clock_observability import Verdict, assess, load_artifact


def fixture() -> dict:
    reports = []
    for sequence in range(3):
        for direction in ("input", "output"):
            stamp = 1_000_000_000 + sequence * 10_000_000
            reports.append({
                "arm": "synthetic-only", "direction": direction, "sequence": sequence,
                "sample_rate_hz": 48_000, "requested_clock": "Monotonic",
                "actual_clock": "Monotonic", "requested_type": "LinkSynchronized",
                "actual_type": "LinkSynchronized", "frame_derivation": "status-hardware-position",
                "status_ok": True, "running": True, "report_valid": True,
                "accuracy_valid": True, "trigger_origin_valid": True,
                "fallback": None, "clamped_negative": False, "timestamp_ns": stamp,
                "system_timestamp_ns": stamp + 150, "driver_timestamp_ns": stamp + 100, "trigger_timestamp_ns": 900_000_000,
                "accuracy_ns": 1_000, "clock_age_bound_ns": 10_000,
                "frame_alignment_bound_ns": 10_000, "observer_before_ns": stamp + 200,
                "observer_after_ns": stamp + 300, "frame_position": sequence * 480,
                "loss_count": 0, "error_count": 0,
            })
    return {"version": 1, "reports": reports}


class ClockObservabilityTests(unittest.TestCase):
    def check_case(self, artifact: dict, expected: Verdict, reason: str) -> None:
        result = assess(load_artifact(json.dumps(artifact)))
        self.assertEqual(result.verdict, expected, result.reasons)
        if reason:
            self.assertTrue(any(reason in item for item in result.reasons), result.reasons)

    def test_complete_duplex_is_observable_without_a_precision_verdict(self) -> None:
        self.check_case(fixture(), Verdict.OBSERVABLE, "")
        value = fixture()
        for report in value["reports"]:
            report["accuracy_ns"] = 1_000_000_000
        # Availability is deliberately independent of any F4 threshold.
        self.check_case(value, Verdict.OBSERVABLE, "")

    def test_missing_metadata_is_inconclusive_and_never_zero(self) -> None:
        keys = ("actual_clock", "actual_type", "timestamp_ns", "system_timestamp_ns", "driver_timestamp_ns",
                "trigger_timestamp_ns", "accuracy_ns", "clock_age_bound_ns",
                "frame_alignment_bound_ns", "frame_position", "frame_derivation")
        for key in keys:
            with self.subTest(key=key):
                value = fixture()
                for report in value["reports"]:
                    report[key] = None
                self.check_case(value, Verdict.INCONCLUSIVE, f"missing {key}")

    def test_fallback_and_invalid_reports_are_not_silently_accepted(self) -> None:
        for change, reason in (
            ({"report_valid": False}, "report invalid"),
            ({"accuracy_valid": False}, "accuracy not attested"),
            ({"requested_type": "Compat", "actual_type": "Compat"}, "not individually attested"),
            ({"report_valid": False, "actual_type": "Compat"}, "report invalid"),
            ({"actual_type": "Default"}, "timestamp type was not returned"),
            ({"actual_clock": "Gettimeofday"}, "clock was not returned"),
            ({"fallback": "Instant-after-status-error"}, "explicit backend fallback"),
            ({"fallback": "get_htstamp-before-Running"}, "explicit backend fallback"),
        ):
            with self.subTest(change=change):
                value = fixture()
                for report in value["reports"]:
                    report.update(change)
                self.check_case(value, Verdict.INCONCLUSIVE, reason)

    def test_broken_stream_stops_even_with_missing_metadata(self) -> None:
        for change, reason in (
            ({"status_ok": False}, "status failed"),
            ({"running": False}, "not running"),
            ({"trigger_origin_valid": False}, "invalid trigger origin"),
            ({"loss_count": 1}, "lost reports"),
            ({"error_count": 1}, "backend errors"),
            ({"clamped_negative": True}, "clamped a negative"),
            ({"observer_after_ns": 0}, "backward observer bracket"),
        ):
            with self.subTest(change=change):
                value = fixture()
                value["reports"][0].update(change, accuracy_ns=None)
                self.check_case(value, Verdict.STOP, reason)

    def test_stale_backward_lost_and_changed_sources_stop(self) -> None:
        for change, reason in (
            ({"sequence": 0}, "sequence"),
            ({"sequence": 2}, "sequence"),
            ({"timestamp_ns": 1_000_000_000}, "stale, repeated or backward"),
            ({"timestamp_ns": 999_999_999}, "stale, repeated or backward"),
            ({"driver_timestamp_ns": 1_000_000_100}, "driver_timestamp_ns"),
            ({"frame_position": 0}, "frame_position"),
            ({"actual_clock": "Gettimeofday"}, "clock source"),
            ({"actual_type": "Default"}, "clock source"),
            ({"requested_type": "Default"}, "clock source"),
            ({"frame_derivation": "synthetic-frame-counter"}, "frame derivation"),
            ({"sample_rate_hz": 44_100}, "rate changed"),
            ({"arm": "restarted"}, "arm changed"),
            ({"trigger_timestamp_ns": 900_000_001}, "trigger origin changed"),
            ({"observer_before_ns": 1_000_000_000}, "observer clock"),
        ):
            with self.subTest(change=change):
                value = fixture()
                value["reports"][2].update(change)
                self.check_case(value, Verdict.STOP, reason)

    def test_null_gap_preserves_last_known_clock_and_source_evidence(self) -> None:
        for key in ("timestamp_ns", "system_timestamp_ns", "driver_timestamp_ns", "frame_position"):
            with self.subTest(key=key):
                value = fixture()
                value["reports"][2][key] = None
                value["reports"][4][key] = value["reports"][0][key]
                self.check_case(value, Verdict.STOP, f"backward {key}")
        for key, changed in (("actual_clock", "Gettimeofday"), ("actual_type", "Default"),
                             ("frame_derivation", "synthetic-frame-counter"),
                             ("trigger_timestamp_ns", 900_000_001)):
            with self.subTest(key=key):
                value = fixture()
                value["reports"][2][key] = None
                value["reports"][4][key] = changed
                self.check_case(value, Verdict.STOP, key)
        for key in ("actual_clock", "actual_type", "frame_derivation", "trigger_timestamp_ns"):
            with self.subTest(missing=key):
                value = fixture()
                value["reports"][2][key] = None
                self.check_case(value, Verdict.INCONCLUSIVE, f"missing {key}")

    def test_declared_callback_counter_and_compat_aliases_cannot_pass(self) -> None:
        for derivation in ("callback-count", "synthetic-frame-counter"):
            value = fixture()
            for report in value["reports"]:
                report["frame_derivation"] = derivation
            self.check_case(value, Verdict.INCONCLUSIVE, "not hardware frame correspondence")
        for alias in ("COMPAT", "compat", "SND_PCM_AUDIO_TSTAMP_TYPE_COMPAT"):
            value = fixture()
            for report in value["reports"]:
                report["actual_type"] = report["requested_type"] = alias
            with self.assertRaises(ValueError):
                load_artifact(json.dumps(value))

    def test_startup_is_never_discarded(self) -> None:
        value = fixture()
        value["reports"] = value["reports"][2:]
        self.check_case(value, Verdict.STOP, "startup prefix missing")
        value = fixture()
        value["reports"][0]["clamped_negative"] = True
        self.check_case(value, Verdict.STOP, "clamped")

    def test_relative_audio_and_absolute_trigger_have_distinct_origins(self) -> None:
        value = fixture()
        for report in value["reports"]:
            report["timestamp_ns"] -= 1_000_000_000
        self.check_case(value, Verdict.OBSERVABLE, "")

    def test_missing_direction_and_empty_stream_are_inconclusive(self) -> None:
        self.check_case({"version": 1, "reports": []}, Verdict.INCONCLUSIVE, "two advancing")
        value = fixture()
        value["reports"] = value["reports"][::2]
        self.check_case(value, Verdict.INCONCLUSIVE, "output")

    def test_closed_versioned_schema_rejects_bad_external_values(self) -> None:
        cases = []
        value = fixture()
        value["extra"] = 1
        cases.append(value)
        for version in (0, 2, True, "1"):
            value = fixture()
            value["version"] = version
            cases.append(value)
        for key, bad in (("accuracy_ns", -1), ("sequence", True), ("running", 1),
                         ("timestamp_ns", float("nan")), ("sample_rate_hz", 0),
                         ("frame_position", 0.5), ("observer_before_ns", None),
                         ("direction", "duplex"), ("actual_clock", "")):
            value = fixture()
            value["reports"][0][key] = bad
            cases.append(value)
        for key in fixture()["reports"][0]:
            value = fixture()
            del value["reports"][0][key]
            cases.append(value)
        value = fixture()
        value["reports"][0]["accuracy_typo_ns"] = 0
        cases.append(value)
        for value in cases:
            with self.subTest(value=value):
                with self.assertRaises(ValueError):
                    load_artifact(json.dumps(value))
        with self.assertRaisesRegex(ValueError, "duplicate"):
            load_artifact('{"version":1,"version":1,"reports":[]}')

    def test_key_order_and_cli_keep_physical_qualification_false(self) -> None:
        original = fixture()
        self.assertEqual(load_artifact(json.dumps(original)), load_artifact(json.dumps(original, sort_keys=True)))
        with tempfile.TemporaryDirectory() as directory:
            artifact = Path(directory) / "fixture.json"
            script = Path(__file__).with_name("clock_observability.py")
            for value, code, verdict in ((original, 0, "Observable"),
                                         ({"version": 1, "reports": []}, 1, "Inconclusive")):
                artifact.write_text(json.dumps(value), encoding="utf-8")
                result = subprocess.run([sys.executable, "-B", str(script), str(artifact)],
                                        capture_output=True, text=True, check=False)
                self.assertEqual(result.returncode, code, result.stderr)
                report = json.loads(result.stdout)
                self.assertEqual(report["verdict"], verdict)
                self.assertIs(report["physical_qualification"], False)
            stopped = fixture()
            stopped["reports"][0]["loss_count"] = 1
            artifact.write_text(json.dumps(stopped), encoding="utf-8")
            result = subprocess.run([sys.executable, "-B", str(script), str(artifact)],
                                    capture_output=True, text=True, check=False)
            self.assertEqual(result.returncode, 1)
            self.assertEqual(json.loads(result.stdout)["verdict"], "Stop")
            for malformed in ('{"version":0,"reports":[]}', '[' * 2000):
                artifact.write_text(malformed, encoding="utf-8")
                result = subprocess.run([sys.executable, "-B", str(script), str(artifact)],
                                        capture_output=True, text=True, check=False)
                self.assertEqual(result.returncode, 2)
                self.assertIn("Invalid artifact:", result.stderr)


if __name__ == "__main__":
    unittest.main()
