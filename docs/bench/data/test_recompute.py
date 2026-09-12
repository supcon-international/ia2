"""Offline falsification tests. No services, model calls, or hardware.

Run: python3 -m unittest discover -s docs/bench/data -p 'test_*.py' -v
"""

import contextlib
import copy
import csv
import io
import math
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

import evidence
import recompute


HERE = Path(__file__).parent
SOAK = "soak-4h-20260908.csv"
GEAR = "dual-gear-20260708.csv"
LEGACY = "scan-cadence-ethercat-only-20260728.csv"


def raw(name):
    with (HERE / name).open(newline="") as stream:
        return list(csv.DictReader(stream))


class CaptureTests(unittest.TestCase):
    def test_every_retained_capture(self):
        for name, spec in evidence.CAPTURES.items():
            with self.subTest(name=name):
                self.assertEqual(len(evidence.read_csv(HERE / name)), spec.count)

    def test_missing_and_extra_rows(self):
        rows = raw(SOAK)
        for broken in ([], rows[:-1], rows + [rows[-1]]):
            with self.subTest(count=len(broken)):
                with self.assertRaisesRegex(evidence.EvidenceError, "data rows"):
                    evidence.validate(SOAK, broken)

    def test_schema_and_cell_errors(self):
        mutations = [lambda r: r.pop("devices_healthy"),
                     lambda r: r.update(extra="1"),
                     lambda r: r.update(devices_healthy=None)]
        for mutate in mutations:
            with self.subTest(mutation=mutate):
                rows = raw(SOAK)
                mutate(rows[0])
                with self.assertRaises(evidence.EvidenceError):
                    evidence.validate(SOAK, rows)

    def test_boolean_domain_prevents_cancellation(self):
        for field in ("devices_healthy", "watchdog_tripped"):
            rows = raw(SOAK)
            rows[5][field], rows[6][field] = "-1", "2"
            with self.subTest(field=field):
                with self.assertRaisesRegex(evidence.EvidenceError, "boolean"):
                    evidence.validate(SOAK, rows)

    def test_nonfinite_and_missing_numeric(self):
        for value in ("nan", "inf", "-inf", "1e999", "", "no-number"):
            rows = raw("valve-step-20260701.csv")
            rows[5]["fb_V"] = value
            with self.subTest(value=value):
                with self.assertRaisesRegex(evidence.EvidenceError, "finite number"):
                    evidence.validate("valve-step-20260701.csv", rows)

    def test_integer_domain(self):
        for value in ("1.5", "-1", "nan", "", str(2**64)):
            rows = raw(SOAK)
            rows[1]["scan_count"] = value
            with self.subTest(value=value):
                with self.assertRaises(evidence.EvidenceError):
                    evidence.validate(SOAK, rows)

    def test_original_audit_duplicate_injection(self):
        rows = raw(SOAK)
        rows[1000]["ts_us"] = rows[999]["ts_us"]
        rows[1000]["scan_count"] = rows[999]["scan_count"]
        with self.assertRaisesRegex(evidence.EvidenceError, "duplicate"):
            recompute.rates_from_counters(SOAK, rows)

    def test_reversed_timestamp(self):
        rows = raw(SOAK)
        rows[1000]["ts_us"] = str(int(rows[999]["ts_us"]) - 1)
        with self.assertRaisesRegex(evidence.EvidenceError, "reversed"):
            evidence.validate(SOAK, rows)

    def test_counter_rollback(self):
        rows = raw(SOAK)
        rows[1000]["scan_count"] = str(int(rows[999]["scan_count"]) - 1)
        with self.assertRaisesRegex(evidence.EvidenceError, "counter moved backwards"):
            evidence.validate(SOAK, rows)

    def test_capture_gap(self):
        for name, delta in ((SOAK, 3_000_000),
                            ("scan-cadence-mixed-bus-postfix-20260908.csv", 500_000)):
            rows = raw(name)
            rows[-1]["ts_us"] = str(int(rows[-2]["ts_us"]) + delta)
            with self.subTest(name=name):
                with self.assertRaisesRegex(evidence.EvidenceError, "capture gap"):
                    evidence.validate(name, rows)

    def test_all_adjacent_pairs_used(self):
        self.assertEqual(len(recompute.rates_from_counters(SOAK, raw(SOAK))), 14399)

    def test_short_positive_interval_is_not_silently_dropped(self):
        rows = raw(SOAK)
        rows[1000]["ts_us"] = str(int(rows[999]["ts_us"]) + 100_000)
        rates = recompute.rates_from_counters(SOAK, rows)
        self.assertEqual(len(rates), 14399)
        self.assertGreater(rates[999], 4000)

    def test_first_derivative_blank_is_explicit(self):
        rows = raw(LEGACY)
        self.assertEqual(rows[0]["scan_rate"], "")
        evidence.validate(LEGACY, rows)
        rows[0]["scan_rate"] = "500"
        with self.assertRaisesRegex(evidence.EvidenceError, "first derivative"):
            evidence.validate(LEGACY, rows)

    def test_later_derivative_blank_is_rejected(self):
        rows = raw(LEGACY)
        rows[10]["scan_rate"] = ""
        with self.assertRaisesRegex(evidence.EvidenceError, "finite number"):
            evidence.validate(LEGACY, rows)

    def test_new_gear_duplicate_not_covered_by_legacy_exception(self):
        rows = raw(GEAR)
        rows[100] = copy.deepcopy(rows[99])
        with self.assertRaisesRegex(evidence.EvidenceError, "unexpected duplicate"):
            evidence.validate(GEAR, rows)

    def test_annotated_gear_duplicate_must_be_identical(self):
        rows = raw(GEAR)
        rows[29]["act0"] = str(int(rows[29]["act0"]) + 1)
        with self.assertRaisesRegex(evidence.EvidenceError, "unexpected duplicate"):
            evidence.validate(GEAR, rows)

    def test_gear_duplicate_cannot_move_in_time(self):
        rows = raw(GEAR)
        rows[28]["t"] = rows[29]["t"] = "5.359"
        with self.assertRaisesRegex(evidence.EvidenceError, "unexpected duplicate"):
            evidence.validate(GEAR, rows)

    def test_calibration_pairs_must_be_unique(self):
        name = "valve-calibration-20260701.csv"
        rows = raw(name)
        rows[4] = copy.deepcopy(rows[3])
        with self.assertRaisesRegex(evidence.EvidenceError, "unique up/down"):
            evidence.validate(name, rows)

    def test_deadband_index_gap(self):
        name = "valve-deadband-20260701.csv"
        rows = raw(name)
        rows[-1]["idx"] = "19"
        with self.assertRaisesRegex(evidence.EvidenceError, "non-contiguous"):
            evidence.validate(name, rows)

    def test_malformed_csv_fails_cleanly(self):
        for body in ("", "ts_us,ts_us,devices_healthy\n1,1,1\n",
                     "ts_us,scan_count,devices_healthy\n1,2,1,extra\n",
                     "ts_us,scan_count,devices_healthy\n1,2\n",
                     'ts_us,scan_count,devices_healthy\n1,2,"unfinished\n'):
            with self.subTest(body=body), tempfile.TemporaryDirectory() as tmp:
                path = Path(tmp) / "scan-cadence-mixed-bus-postfix-20260908.csv"
                path.write_text(body)
                with self.assertRaises(evidence.EvidenceError):
                    evidence.read_csv(path)

    def test_missing_file_and_unknown_contract(self):
        with tempfile.TemporaryDirectory() as tmp:
            for name in (SOAK, "unregistered.csv"):
                with self.subTest(name=name), self.assertRaises(evidence.EvidenceError):
                    evidence.read_csv(Path(tmp) / name)


class NumericTests(unittest.TestCase):
    def setUp(self):
        recompute.FAILURES.clear()

    def check_cases(self, want, cases):
        for got, passes in cases:
            with self.subTest(got=got), contextlib.redirect_stdout(io.StringIO()):
                recompute.FAILURES.clear()
                recompute.check("probe", got, want)
                self.assertEqual(not recompute.FAILURES, passes)

    def test_exact_count_replaces_six_plus_or_minus_three(self):
        self.check_cases(7, [(7, True), (6, False), (8, False)])

    def test_quantization_is_not_forty_percent_tolerance(self):
        self.check_cases(0.192, [(0.192, True), (0.2, False), (0.27, False)])

    def test_trailing_zero_precision_is_preserved(self):
        self.check_cases("2.0000", [(1.9999513, True), (1.9999, False), (2.01, False)])
        self.check_cases("500.00", [(499.99955, True), (500.01, False)])

    def test_float_rounding_boundary(self):
        self.check_cases(1.23, [(1.235, True), (1.2351, False)])

    def test_nonfinite_calculation_fails(self):
        self.check_cases(1, [(math.nan, False), (math.inf, False), (-math.inf, False)])

    def test_main_resets_prior_failures(self):
        recompute.FAILURES.append("old run")
        with contextlib.redirect_stdout(io.StringIO()):
            self.assertEqual(recompute.main(), 0)

    def test_preflight_missing_evidence_returns_failure(self):
        with tempfile.TemporaryDirectory() as tmp, patch.object(recompute, "HERE", Path(tmp)):
            with contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO()):
                self.assertEqual(recompute.main(), 1)

    def test_journal_events_fail_closed(self):
        with self.assertRaisesRegex(evidence.EvidenceError, "ISO UTC"):
            recompute.event_time("bus shape CHANGED", "journal")
        for start, end in ((None, "2026-09-08T00:00:01"),
                           ("2026-09-08T00:00:01", None),
                           ("2026-09-08T00:00:02", "2026-09-08T00:00:01")):
            with self.subTest(start=start, end=end), self.assertRaises(evidence.EvidenceError):
                recompute.elapsed(start, end, "journal")

    def test_missing_and_duplicate_cable_recovery(self):
        name = "cable-pull-journal-20260908.txt"
        original = (HERE / name).read_text()
        recovery = next(line for line in original.splitlines()
                        if "recovered; cyclic exchange running again" in line)
        for broken in (original.replace(recovery, ""), original + "\n" + recovery):
            with self.subTest(duplicate=broken.endswith(recovery)), tempfile.TemporaryDirectory() as tmp:
                (Path(tmp) / name).write_text(broken)
                with patch.object(recompute, "HERE", Path(tmp)), contextlib.redirect_stdout(io.StringIO()):
                    with self.assertRaises(evidence.EvidenceError):
                        recompute.cable_pull()

    def test_cli_success(self):
        result = subprocess.run([sys.executable, "-B", str(HERE / "recompute.py")],
                                capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr + result.stdout)
        self.assertIn("not hardware acceptance", result.stdout)


if __name__ == "__main__":
    unittest.main()
