#!/usr/bin/env python3
"""Validate retained captures and recompute the numeric claims encoded here.
Stdlib only — no dependency on IA2 code. Contracts and limits are documented
in ../data-validation.md; Markdown prose is not parsed by this script.

Run:  python3 docs/bench/data/recompute.py
Exit: 0 = valid captures and all encoded numeric claims reproduce;
      1 = invalid/missing evidence or a numeric mismatch (printed).
"""

import math
import re
import statistics
import sys
from datetime import datetime
from decimal import Decimal
from pathlib import Path

from evidence import CAPTURES, EvidenceError, read_csv, validate

HERE = Path(__file__).parent
FAILURES: list[str] = []

ENCODER_CNT_PER_REV = 8_388_608  # SV660N / MS1H4 23-bit absolute encoder


def check(label: str, got: float, want) -> None:
    # Counts are exact. Decimal targets allow only half a unit of their last
    # reported place (+ floating-point arithmetic epsilon), NOT a bench
    # acceptance threshold or a claimed measurement uncertainty.
    exact = isinstance(want, int)
    target = want if exact else float(want)
    tol = 0 if exact else float(Decimal(10) ** Decimal(str(want)).as_tuple().exponent / 2)
    ok = math.isfinite(got) and (got == target if exact else abs(got - target) <= tol + 1e-12)
    print(f"  {'ok  ' if ok else 'FAIL'} {label}: got {got:.8f}, target {want} (rounding {tol})")
    if not ok:
        FAILURES.append(label)


def rows(name: str) -> list[dict]:
    return read_csv(HERE / name)


def rates_from_counters(name: str, data: list[dict]) -> list[float]:
    # Revalidate here too: callers supplying in-memory rows cannot skip the
    # timestamp/counter checks. Every adjacent pair contributes; no dt filter.
    validate(name, data)
    return [
        (int(b["scan_count"]) - int(a["scan_count"]))
        / ((int(b["ts_us"]) - int(a["ts_us"])) / 1e6)
        for a, b in zip(data, data[1:])
    ]


def event_time(line: str, name: str) -> str:
    match = re.search(r"(\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d+)Z", line)
    if match is None:
        raise EvidenceError(f"{name}: event lacks an ISO UTC timestamp")
    return match.group(1)


def elapsed(start, end, name):
    if start is None or end is None:
        raise EvidenceError(f"{name}: missing start/recovery event")
    seconds = (datetime.fromisoformat(end) - datetime.fromisoformat(start)).total_seconds()
    if seconds <= 0:
        raise EvidenceError(f"{name}: recovery does not follow start")
    return seconds


# ---------------------------------------------------------------- scan cadence
def scan_cadence() -> None:
    print("scan-cadence-ethercat-only-20260728.csv (2 ms task, EtherCAT only)")
    rates = [float(r["scan_rate"]) for r in rows("scan-cadence-ethercat-only-20260728.csv") if r["scan_rate"]]
    check("mean scan rate [/s]", statistics.mean(rates), 500.0)
    check("min scan rate [/s]", min(rates), 495.3)
    dur = len(rates)  # one sample every ~0.2 s → duration sanity only
    print(f"       samples: {dur} (≈{dur * 0.2:.0f} s of logging)")

    print("scan-cadence-mixed-bus-20260709.csv (2 ms task, EtherCAT + Modbus RTU coupler, pre-fix)")
    mixed = rows("scan-cadence-mixed-bus-20260709.csv")
    rates = [float(r["scan_rate"]) for r in mixed if r["scan_rate"]]
    check("mean scan rate [/s]", statistics.mean(rates), 397.1)
    check("min scan rate [/s]", min(rates), 357.9)
    bad = sum(1 for r in mixed if r["coupler_ok"] != "1")
    check("coupler_ok != 1 samples", bad, 0)
    print(f"       coupler_ok = 1 on {len(mixed) - bad}/{len(mixed)} samples")


def scan_cadence_postfix() -> None:
    print("scan-cadence-mixed-bus-postfix-20260908.csv (2 ms task, EtherCAT + RTU coupler, POST-fix)")
    data = rows("scan-cadence-mixed-bus-postfix-20260908.csv")
    rates = rates_from_counters("scan-cadence-mixed-bus-postfix-20260908.csv", data)
    check("mean scan rate [/s]", statistics.mean(rates), 500.0)
    check("min scan rate [/s]", min(rates), 493.6)
    unhealthy = sum(1 for r in data if r["devices_healthy"] != "1")
    check("unhealthy samples", unhealthy, 0)

    print("long-uptime-counters-20260908.csv (continuous-operation counter snapshot)")
    lu = rows("long-uptime-counters-20260908.csv")
    last = lu[-1]
    days = int(last["uptime_secs"]) / 86400
    avg = int(last["scan_count"]) / int(last["uptime_secs"])
    check("lifetime mean scan rate [/s]", avg, 499.73)
    check("continuous uptime [days]", days, 7.85)


def soak_4h() -> None:
    print("soak-4h-20260908.csv (4-hour 1 Hz soak, EtherCAT 2 ms, main build)")
    data = rows("soak-4h-20260908.csv")
    check("samples", len(data), 14400)
    check("watchdog trips", sum(int(r["watchdog_tripped"]) for r in data), 0)
    check("unhealthy samples", sum(1 - int(r["devices_healthy"]) for r in data), 0)
    span = (int(data[-1]["ts_us"]) - int(data[0]["ts_us"])) / 1e6
    check("span [h]", span / 3600, 4.05)
    rates = rates_from_counters("soak-4h-20260908.csv", data)
    check("mean scan rate [/s]", statistics.mean(rates), "500.00")
    check("worst 1 s sample [/s]", min(rates), 498.8)


def cable_pull() -> None:
    print("cable-pull-journal-20260908.txt (bus-loss self-heal, journal excerpt)")
    name = "cable-pull-journal-20260908.txt"
    text = (HERE / name).read_text(encoding="utf-8")
    t_changed = t_recovered = None
    for line in text.splitlines():
        if "bus shape CHANGED" in line and t_changed is None:
            t_changed = event_time(line, name)
        if "recovered; cyclic exchange running again" in line:
            if t_recovered is not None:
                raise EvidenceError(f"{name}: ambiguous repeated recovery event")
            t_recovered = event_time(line, name)
    check("bus-change detection to OP [s]", elapsed(t_changed, t_recovered, name), 2.08)
    check("re-walk backoff engaged", int("re-walk backoff attempt=1" in text), 1)
    check("transport rebuild path exercised", int("supervise loop will rebuild the transport" in text), 1)


def power_cycle() -> None:
    print("powercycle-journal-20260908.txt (drive power-cycle self-heal, journal excerpt)")
    name = "powercycle-journal-20260908.txt"
    fails, backoffs = [], []
    t_changed = t_recovered = t_last_backoff = None
    text = (HERE / name).read_text(encoding="utf-8")
    for line in text.splitlines():
        if "shape CHANGED" in line and t_changed is None:
            t_changed = event_time(line, name)
        if "re-walk step failed" in line:
            fails.append(line)
        if "re-walk backoff" in line:
            m = re.search(r"delay_ms=(\d+)", line)
            if m is None:
                raise EvidenceError(f"{name}: backoff lacks delay_ms")
            backoffs.append(int(m.group(1)))
            t_last_backoff = event_time(line, name)
        if "recovered; cyclic exchange running again" in line:
            t_recovered = event_time(line, name)
    check("clean walk failures during outage", len(fails), 11)
    check("backoff events", len(backoffs), 11)
    check("backoff schedule head 1/2/4 s", int(backoffs[:3] == [1000, 2000, 4000]), 1)
    check("backoff schedule capped at 5 s", int(all(b == 5000 for b in backoffs[3:])), 1)
    check("false successes on the dead bus", text.count("recovered; cyclic exchange running again") - 1, 0)
    if not backoffs:
        raise EvidenceError(f"{name}: missing backoff events")
    walk = elapsed(t_last_backoff, t_recovered, name) - backoffs[-1] / 1000
    check("post-backoff recovery interval [s]", walk, 0.96)
    check("total outage detection-to-recovered [s]", elapsed(t_changed, t_recovered, name), 48.3)


# ---------------------------------------------------------------- dual gear
def phase_ratio(data: list[dict], phase: str) -> float:
    """Steady-state actual/actual ratio, mid-segment (1/4..3/4 of the
    phase) to skip the soft-engagement ramps — the same window the
    original bench script used, so the published digits reproduce."""
    seg = [r for r in data if r["phase"] == phase]
    i, j = len(seg) // 4, len(seg) * 3 // 4
    d0 = float(seg[j]["act0"]) - float(seg[i]["act0"])
    d1 = float(seg[j]["act1"]) - float(seg[i]["act1"])
    return d0 / d1


def dual_gear() -> None:
    print("dual-gear-20260708.csv (two SV660N axes, 2 ms / CSP / SYNC0)")
    data = rows("dual-gear-20260708.csv")
    check("gear ratio 1:1 forward", phase_ratio(data, "gear-1to1-fwd"), 1.0026)
    check("gear ratio 2:1 forward", phase_ratio(data, "gear-2to1-fwd"), 1.9979)
    check("gear ratio 1:1 reverse", phase_ratio(data, "gear-1to1-rev"), 0.9998)
    check("gear ratio 2:1 reverse", phase_ratio(data, "gear-2to1-rev"), "2.0000")

    def ferr_deg(phase: str):
        seg = [abs(float(r["ferr0"])) for r in data if r["phase"] == phase]
        to_deg = 360.0 / ENCODER_CNT_PER_REV
        return statistics.mean(seg) * to_deg, max(seg) * to_deg

    m, p = ferr_deg("gear-1to1-fwd")
    check("ferr 1:1 fwd mean [deg motor]", m, 0.58)
    _, pr = ferr_deg("gear-1to1-rev")
    check("ferr 1:1 peak fwd/rev [deg motor]", max(p, pr), 2.57)
    m2, p2 = ferr_deg("gear-2to1-fwd")
    check("ferr 2:1 fwd mean [deg motor]", m2, 1.07)
    _, p2r = ferr_deg("gear-2to1-rev")
    check("ferr 2:1 peak fwd/rev [deg motor]", max(p2, p2r), 4.31)

    engage = [r for r in data if r["phase"] == "engage-still"]
    d_follow = float(engage[-1]["act0"]) - float(engage[0]["act0"])
    check("standstill engagement displacement [cnt]", abs(d_follow), 16)

    # Round-trip closure, as the source report defines it: the MASTER
    # axis's settled position after the 1:1 out-and-back, relative to
    # its commanded home (0) — read at the first sample of the settled
    # inter-leg segment that follows the 1:1 reverse leg.
    settled = next(r for r in data if r["phase"] == "gear-2to1")
    closure_cnt = float(settled["act1"])
    check("1:1 round-trip closure [cnt master]", closure_cnt, -7708)
    check(
        "1:1 round-trip closure [deg motor]",
        closure_cnt * 360.0 / ENCODER_CNT_PER_REV,
        -0.33,
    )

    trips = [r for r in data if r["trip0"] == "TRUE" or r["trip1"] == "TRUE"]
    not_ok = [r for r in data if r["phase"].startswith("gear-") and r["run_ok"] != "TRUE"]
    check("trip assertions during run", len(trips), 0)
    check("run_ok false samples during motion", len(not_ok), 0)


# ---------------------------------------------------------------- valve / RTU
def valve() -> None:
    print("valve-calibration-20260701.csv (0-10 V valve via NX6 RTU coupler, 9600 8-E-1)")
    data = rows("valve-calibration-20260701.csv")
    xs = [float(r["cmd_V"]) for r in data]
    ys = [float(r["fb_V"]) for r in data]
    n = len(xs)
    mx, my = statistics.mean(xs), statistics.mean(ys)
    sxx = sum((x - mx) ** 2 for x in xs)
    sxy = sum((x - mx) * (y - my) for x, y in zip(xs, ys))
    slope = sxy / sxx
    intercept = my - slope * mx
    ss_res = sum((y - (slope * x + intercept)) ** 2 for x, y in zip(xs, ys))
    ss_tot = sum((y - my) ** 2 for y in ys)
    r2 = 1 - ss_res / ss_tot
    check("linearity slope", slope, 0.9963)
    check("linearity intercept [V]", intercept, -0.0134)
    check("R^2", r2, 0.99991)
    max_resid = max(abs(y - (slope * x + intercept)) for x, y in zip(xs, ys))
    check("max residual [V]", max_resid, 0.065)

    # hysteresis: |up - down| at matching commands
    ups = {r["cmd_V"]: float(r["fb_V"]) for r in data if r["dir"] == "up"}
    downs = {r["cmd_V"]: float(r["fb_V"]) for r in data if r["dir"] == "dn"}
    common = sorted(set(ups) & set(downs), key=float)
    hyst = [abs(ups[c] - downs[c]) for c in common]
    check("hysteresis mean [V]", statistics.mean(hyst), 0.029)
    check("hysteresis max [V]", max(hyst), 0.082)

    print("valve-step-20260701.csv (0→10 V step)")
    step = rows("valve-step-20260701.csv")
    fb = [float(r["fb_V"]) for r in step]
    check("step peak (no overshoot) [V]", max(fb), 9.958)

    print("valve-deadband-20260701.csv (0.1 V micro-steps around 5 V)")
    db = rows("valve-deadband-20260701.csv")
    deltas = [
        abs(float(b["fb_V"]) - float(a["fb_V"]))
        for a, b in zip(db, db[1:])
    ]
    held = sum(1 for d in deltas if d == 0.0)
    jumps = [d for d in deltas if d > 0.0]
    check("deadband: cmd steps fb held through", held, 7)
    check("feedback quantization step, median [V]", statistics.median(jumps), 0.192)

    print("valve-endpoint-{low,high}-20260701.csv (seat/end behaviour)")
    # Endpoint envelope: union of every committed cmd=0 / cmd=10 sample
    # (calibration sweep + dedicated endpoint dwell runs).
    lo = [float(r["fb_V"]) for r in rows("valve-endpoint-low-20260701.csv") if float(r["cmd_V"]) == 0.0]
    lo += [float(r["fb_V"]) for r in rows("valve-calibration-20260701.csv") if float(r["cmd_V"]) == 0.0]
    hi = [float(r["fb_V"]) for r in rows("valve-endpoint-high-20260701.csv") if float(r["cmd_V"]) == 10.0]
    hi += [float(r["fb_V"]) for r in rows("valve-calibration-20260701.csv") if float(r["cmd_V"]) == 10.0]
    check("closed endpoint min [V]", min(lo), 0.045)
    check("closed endpoint max [V]", max(lo), 0.058)
    check("open endpoint min [V]", min(hi), 9.936)
    check("open endpoint max [V]", max(hi), 9.938)


def main() -> int:
    FAILURES.clear()
    try:
        # Fail before reporting any headline if a CSV capture is malformed.
        for name in CAPTURES:
            rows(name)
        print("Capture validation passed (5 documented legacy gear duplicates retained).")
        scan_cadence()
        scan_cadence_postfix()
        soak_4h()
        cable_pull()
        power_cycle()
        dual_gear()
        valve()
    except (EvidenceError, OSError, ValueError, ZeroDivisionError) as exc:
        print(f"FAIL evidence: {exc}", file=sys.stderr)
        return 1
    if FAILURES:
        print(f"\n{len(FAILURES)} number(s) FAILED to reproduce: {FAILURES}")
        return 1
    print("\nAll encoded numeric claims reproduce from validated captures (not hardware acceptance).")
    return 0


if __name__ == "__main__":
    sys.exit(main())
