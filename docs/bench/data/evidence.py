"""CSV capture contracts, independent of IA2. See ../data-validation.md.

These validate the retained capture, not the hardware or its timing guarantees.
All values stay strings so recomputation keeps the historical arithmetic.
"""

import csv
import math
import re
from dataclasses import dataclass, field
from pathlib import Path


class EvidenceError(ValueError):
    """A named artifact cannot support recomputation."""


@dataclass(frozen=True)
class Capture:
    columns: str
    count: int
    integers: str = ""
    enums: dict = field(default_factory=dict)
    first_blank: str = ""
    optional: str = ""
    increasing: str = ""
    counters: str = ""
    # Upper capture-gap bounds, NOT real-time deadline limits.
    max_gaps: dict = field(default_factory=dict)


LEGACY_COLUMNS = (
    "t ts_us scan_rate rpm_master rpm_follower rpm_output pos0 pos1 rev1 "
    "ferr1_cnt ferr1_deg engaged ratio sync_err_cnt sync_err_deg demo_auto "
    "demo_speed jog1 trip1 g_trip_fb fault0 fault1 state0 state1 lockout "
    "run_ok ao1 ai1 valve_cmd_v valve_fb_v coupler_ok servo_ok"
)
BOOL01 = {"0", "1"}
BOOLST = {"TRUE", "FALSE"}
PHASES = {
    "preflight", "enable", "engage-still", "gear-1to1-fwd", "gear-1to1-rev",
    "gear-2to1", "gear-2to1-fwd", "gear-2to1-rev", "teardown",
}
# Existing capture defects, CSV physical lines (header = 1). Do not deduplicate
# them and silently change the published calculation's weighting.
GEAR_DUPLICATES = {31: "5.360", 275: "53.624", 357: "69.915",
                   438: "86.002", 444: "87.010"}


def legacy(count):
    return Capture(
        LEGACY_COLUMNS, count,
        integers="ts_us pos0 pos1 ferr1_cnt state0 state1 ao1 ai1",
        enums={k: BOOL01 for k in (
            "engaged demo_auto trip1 g_trip_fb fault0 fault1 lockout "
            "run_ok coupler_ok servo_ok"
        ).split()},
        first_blank="scan_rate rpm_master rpm_follower rpm_output",
        optional="ratio sync_err_cnt sync_err_deg",
        increasing="t ts_us",
    )


CAPTURES = {
    "scan-cadence-ethercat-only-20260728.csv": legacy(448),
    "scan-cadence-mixed-bus-20260709.csv": legacy(373),
    "scan-cadence-mixed-bus-postfix-20260908.csv": Capture(
        "ts_us scan_count devices_healthy", 600, integers="ts_us scan_count",
        enums={"devices_healthy": BOOL01}, increasing="ts_us", counters="scan_count",
        max_gaps={"ts_us": 400_000},
    ),
    "soak-4h-20260908.csv": Capture(
        "t_unix ts_us scan_count watchdog_tripped devices_healthy", 14400,
        integers="t_unix ts_us scan_count",
        enums={"watchdog_tripped": BOOL01, "devices_healthy": BOOL01},
        increasing="t_unix ts_us", counters="scan_count",
        max_gaps={"ts_us": 2_000_000},
    ),
    "long-uptime-counters-20260908.csv": Capture(
        "t_unix uptime_secs scan_count", 101, integers="t_unix uptime_secs scan_count",
        increasing="t_unix uptime_secs", counters="scan_count",
    ),
    "dual-gear-20260708.csv": Capture(
        "t phase act1 act0 cmd1 cmd0 ferr1 ferr0 engaged trip0 trip1 run_ok state1 state0",
        445, integers="act1 act0 cmd1 cmd0 ferr1 ferr0 state1 state0",
        enums={"phase": PHASES, **{k: BOOLST for k in "engaged trip0 trip1 run_ok".split()}},
    ),
    "valve-calibration-20260701.csv": Capture(
        "dir cmd_V fb_raw fb_V", 42, integers="fb_raw", enums={"dir": {"up", "dn"}},
    ),
    "valve-endpoint-high-20260701.csv": Capture(
        "dir cmd_V fb_raw fb_V", 13, integers="fb_raw", enums={"dir": {"up", "dn"}},
    ),
    "valve-endpoint-low-20260701.csv": Capture(
        "zone cmd_V fb_raw fb_V", 22, integers="fb_raw", enums={"zone": {"low", "high"}},
    ),
    "valve-deadband-20260701.csv": Capture(
        "idx cmd_V fb_raw fb_V", 19, integers="idx fb_raw", increasing="idx",
    ),
    "valve-step-20260701.csv": Capture(
        "t_s cmd_V fb_raw fb_V", 1411, integers="fb_raw", increasing="t_s",
    ),
}


def validate(name, data):
    """Reject malformed/partial captures, including when called on in-memory rows."""
    if name not in CAPTURES:
        raise EvidenceError(f"{name}: no capture contract")
    spec = CAPTURES[name]
    columns = set(spec.columns.split())
    if len(data) != spec.count:
        raise EvidenceError(f"{name}: expected {spec.count} data rows, got {len(data)}")
    integer_fields = set(spec.integers.split())
    optional = set(spec.optional.split())
    first_blank = set(spec.first_blank.split())
    numeric = []
    for line, row in enumerate(data, 2):
        def bad(message):
            raise EvidenceError(f"{name}:{line}: {message}")

        if set(row) != columns:
            bad("missing or unexpected columns")
        values = {}
        for key, value in row.items():
            if not isinstance(value, str):
                bad(f"{key}: missing/extra cell or non-text value")
            if line == 2 and key in first_blank and value != "":
                bad(f"{key}: first derivative sample must be blank")
            if value == "" and (key in optional or (line == 2 and key in first_blank)):
                continue
            if key in spec.enums:
                if value not in spec.enums[key]:
                    bad(f"{key}: invalid enum/boolean {value!r}")
                continue
            if key in integer_fields:
                if not re.fullmatch(r"-?[0-9]+", value):
                    bad(f"{key}: expected an integer")
                number = int(value)
                if abs(number) > 2**64 - 1:
                    bad(f"{key}: integer outside retained 64-bit capture domain")
            else:
                try:
                    number = float(value)
                except ValueError:
                    bad(f"{key}: expected a finite number")
                if not math.isfinite(number):
                    bad(f"{key}: expected a finite number")
            if key in {"ts_us", "scan_count", "uptime_secs", "t_unix", "idx", "fb_raw"} and number < 0:
                bad(f"{key}: negative counter/time/raw input")
            values[key] = number
        if numeric:
            prev = numeric[-1]
            for key in spec.increasing.split():
                delta = values[key] - prev[key]
                if delta <= 0:
                    bad(f"{key}: duplicate or reversed sample time/index")
                if key in spec.max_gaps and delta > spec.max_gaps[key]:
                    bad(f"{key}: capture gap exceeds {spec.max_gaps[key]}")
            for key in spec.counters.split():
                if values[key] < prev[key]:
                    bad(f"{key}: counter moved backwards")
        numeric.append(values)

    if name == "dual-gear-20260708.csv":
        seen = {}
        for line, (a, b) in enumerate(zip(data, data[1:]), 3):
            if float(b["t"]) < float(a["t"]):
                raise EvidenceError(f"{name}:{line}: reversed sample time")
            if b["t"] == a["t"] or float(b["t"]) == float(a["t"]):
                if a != b or GEAR_DUPLICATES.get(line) != b["t"]:
                    raise EvidenceError(f"{name}:{line}: unexpected duplicate timestamp")
                seen[line] = b["t"]
        if seen != GEAR_DUPLICATES:
            raise EvidenceError(f"{name}: historical duplicate annotations no longer match")
        if {r["phase"] for r in data} != PHASES:
            raise EvidenceError(f"{name}: missing phase")
    if name == "valve-calibration-20260701.csv":
        expected = {(direction, step / 2) for direction in ("up", "dn") for step in range(21)}
        if {(r["dir"], float(r["cmd_V"])) for r in data} != expected:
            raise EvidenceError(f"{name}: expected unique up/down points at 0..10 V by 0.5 V")
    if name == "valve-deadband-20260701.csv":
        if [int(r["idx"]) for r in data] != list(range(spec.count)):
            raise EvidenceError(f"{name}: non-contiguous sample indices")
    return data


def read_csv(path: Path):
    if path.name not in CAPTURES:
        raise EvidenceError(f"{path.name}: no capture contract")
    try:
        with path.open(newline="", encoding="utf-8") as stream:
            reader = csv.reader(stream, strict=True)
            header = next(reader, [])
            expected = CAPTURES[path.name].columns.split()
            if len(header) != len(set(header)) or set(header) != set(expected):
                raise EvidenceError(f"{path.name}: missing, duplicate or unexpected header")
            data = []
            for record in reader:
                if len(record) != len(header):
                    raise EvidenceError(f"{path.name}:{reader.line_num}: wrong cell count")
                data.append(dict(zip(header, record)))
    except (OSError, UnicodeError, csv.Error) as exc:
        raise EvidenceError(f"{path.name}: {exc}") from exc
    return validate(path.name, data)
