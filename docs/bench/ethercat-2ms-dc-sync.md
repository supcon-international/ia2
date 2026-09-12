# EtherCAT at 2 ms with DC SYNC0 — real-hardware evidence

Evidence grades (RAW / TRANSCRIPT / PROSE) are defined in
[`README.md`](README.md). RAW numbers are re-derivable by running
`python3 docs/bench/data/recompute.py`.

## Bench

x86_64 edge box, Ubuntu 24.04, stock `PREEMPT_DYNAMIC` kernel (NOT a
realtime kernel), one NIC dedicated to EtherCAT. Chain, depending on
campaign: 2 × Inovance SV660N servo drives with MS1H4 motors (23-bit
absolute encoders, 8,388,608 cnt/rev), later joined by a Leadshine
CL3C-EC507 closed-loop stepper as a third, mixed-vendor slave. All
projects run `interval_ms = 2` / `cycle_us = 2000` with per-slave
`dc_sync = "sync0"`.

## 1. Scan cadence: 2 ms holds on a non-RT kernel — RAW

`data/scan-cadence-ethercat-only-20260728.csv`, 2026-07-28. The
logger computed each rate from the runtime's own `scan_count` delta
over its `timestamp_us` delta — an in-runtime counter, not a
wall-clock guess; the committed CSV carries the resulting `scan_rate`
column (the raw counter itself was not logged).

- **Mean 500.0 scans/s, worst single sample 495.3 scans/s**, over
  89 s / 447 samples, EtherCAT only (2 servo axes in CSP).

The same logger on 2026-07-09, with a Modbus RTU coupler polled by the
same runtime (`data/scan-cadence-mixed-bus-20260709.csv`):

- **Mean 397 scans/s, worst 358 scans/s** — a ~20 % cadence loss with
  the coupler in the loop, traced to blocking Modbus writes on the scan
  thread and fixed upstream (`cd7e49c`, PR #20).

The post-fix mixed-bus measurement was retaken on the bench on
2026-09-08 (`data/scan-cadence-mixed-bus-postfix-20260908.csv`, 125 s /
600 samples, real RTU coupler polled by the same runtime, main build
`0700e40`): **mean 500.0 scans/s, worst 493.6 scans/s, both devices
healthy on every sample** — the regression is closed on real hardware,
RAW on both sides of the fix.

Bonus longevity datum (`data/long-uptime-counters-20260908.csv`): a
counter snapshot of the previous runtime instance showed
**499.73 scans/s lifetime average over 7.85 days of continuous
operation** (scan_count / uptime from the runtime's own monotonic
counters). This is a mean only — it proves sustained cadence, not a
jitter distribution.

## 2. Two-axis electronic gear accuracy — RAW

`data/dual-gear-20260708.csv`, 2026-07-08 (445 retained rows at nominal 0.2 s over an
87 s scripted run). Follower axis geared to the master's *actual*
encoder feedback, 2 ms / CSP / SYNC0. Master jogged at ≈4.3 rpm.
Ratios are steady-state actual/actual over the middle half of each
leg (the source script's window, skipping soft-engagement ramps):

The capture contains five exact duplicate rows (440 distinct samples),
explicitly listed in [`data-validation.md`](data-validation.md). The
historical row weighting/window indices are retained for reproducibility;
the duplicates are not presented as independent observations.

| Measurement | Value |
|---|---|
| Gear ratio, 1:1 leg (fwd / rev) | **1.0026 / 0.9998** |
| Gear ratio, 2:1 leg (fwd / rev) | **1.9979 / 2.0000** |
| Follower following error, 1:1 (mean / peak, motor side) | 0.58° / 2.57° |
| Follower following error, 2:1 (mean / peak, motor side) | 1.07° / 4.31° |
| Standstill engagement displacement | **16 cnt** (≈ encoder noise floor) |
| Master 1:1 round-trip closure vs commanded home | **−7708 cnt = −0.33° motor** |
| Trip interlocks asserted during the run | 0 (`trip0`/`trip1` FALSE on every sample, `run_ok` TRUE through all motion) |

The following error is drive-side proportional-loop lag without
feed-forward (it scales with speed), not a bus or master limitation;
through the follower's 10:1 reducer the 1:1 mean is 0.06° at the
output flange.

## 3. Scan watchdog latch, A/B on the same bench — TRANSCRIPT

Fixture and procedure are committed
([`../../examples/watchdog_latch/`](../../examples/watchdog_latch/),
[`watchdog-latch-runbook.md`](watchdog-latch-runbook.md)); sampled
values are quoted verbatim in PR #38's bench comment (2026-08-18).
Conditions: 2 ms task, 2 × SV660N + CL3C stepper, fixture parked at
CiA402 Ready-to-switch-on, zero motion; scan burn forced cadence down
to 112–119 scans/s (~7 ms/scan) as the vacuity control.

- **Pre-fix**: statusword `16#1631` on 10/10 samples over 10 s —
  `0x0006` kept flowing to the wire *after* the runtime had logged its
  failsafe. The watchdog announced safety and then gave it up.
- **Post-fix** (`34995e7`): `16#1650` (Switch-on-disabled) on 10/10
  samples, `watchdog_tripped = true`, while the VM demonstrably kept
  computing (`scan_count` 6362 → 7628) and the wire held zeros.
- Restart clears the latch; with the overrun still present it re-trips
  within ~40 ms; with the overrun removed, cadence returned to ~490/s.

Method deviation, disclosed: the pre-fix side ran the previously
installed runtime (genuinely pre-fix, but not a fresh build of the
parent commit), and the runbook's optional tcpdump one-frame-race check
was not run.

## 4. Bus-loss self-heal (supervised re-walk) — RAW (both flavors)

Why it exists: any bus interruption longer than the SM watchdog used to
wedge the runtime — the frame exchange "recovered" while every slave
sat outside OP, outputs ignored, until a process restart (757 ms)
cured it. A ~10 s cable pull produced **7063 consecutive scans** of
controlword-echo mismatch — the PLC writing, the drive not applying.
(PR #41 bench comment, 2026-08-27; root-caused after a journal
excavation found a 5 m 43 s bus outage hiding inside an "idle" window.)

Acceptance of the fix (PR #42, edge-native build `2874f17`,
sha-verified, 2026-08-31; quoted in the PR's bench comment):

- **Cable pull ~11 s**: demotion detected (`wkc 9→5`, per-slave AL
  codes), first re-walk timed out on the still-settling bus, 1 s
  backoff, second walk replayed the full `init_sdo` set —
  **replug-to-OP ≈ 2 s**. The drive had latched its own E81B fault
  during the outage; one reset pulse cleared it and enable then
  completed normally — bus recovery is hands-off, the drive-side fault
  ack is not.
- **Drive power-cycle, 38 s off**: 9 re-walk attempts during the
  outage, all failing cleanly with `Timeout` on the exact 1/2/4/5/5 s
  backoff schedule — **no false success against a dead bus** — then
  power-on-to-OP **< 1 s** including full reconfiguration of
  factory-fresh slaves (`mode_display = 8`, controlword echo alive,
  `err = 0x0000`).
- **Held-enable semantics**: the axis re-enabled automatically after
  the re-walk with **`actual_pos` Δ = 0 counts** through the event.
- Enable-time regression: 622/622/622 ms against a 602–733 ms baseline.

The cable-pull case was re-run on 2026-09-08 against the current main
build (`0700e40`, edge runtime sha `78d89dda…`) with the journal
RETAINED this time (`data/cable-pull-journal-20260908.txt`, hostname
redacted; `recompute.py` re-derives the timings from it): ~10 s pull →
10 × `Timeout(Pdu)` → unhealthy, inputs frozen; replug detected as
`[Op,Op,Op] → [SafeOp×3]` wkc 9→5 with per-slave AL codes; first
re-walk failed on the settling bus, 1000 ms backoff; the ethercrab
socket pump died on a late frame and **the supervise loop rebuilt the
transport** (the PR #42 hardening path, observed live for the first
time); PRE-OP census re-identified all three slaves; **replug-to-OP
2.08 s**, baseline `[Op,Op,Op] wkc=9` restored. The drive-side E81B
latch again required one reset pulse — bus recovery is hands-off, the
drive fault ack is not. Precision note: the 2.08 s is measured from
the runtime *detecting* the bus-shape change to cyclic exchange
recovered — the physical replug instant is not independently
instrumented — and the drive's own fault display cleared only ~91 s
later via the reset pulse: bus recovery and drive-fault acknowledgement
are two separate events.

The power-cycle flavor was re-run the same day with the journal
retained (`data/powercycle-journal-20260908.txt`; `recompute.py`
re-derives every number): drives powered off ~48 s → demotion detected
as `[Op,Op,Op] → [Op,None,None]` wkc 9→3, the socket pump died and
**the supervise loop rebuilt the transport** (explicitly logged this
time); **11 re-walk attempts during the outage, every one failing
cleanly with `Timeout` on the exact 1/2/4/5…5 s backoff schedule —
zero false successes against a dead bus**; on power return the next
walk re-identified all three factory-fresh slaves and reached OP in
**0.96 s**. Contrast with the cable pull: after a power cycle the
drive's fault memory is wiped, so no E81B latch and no reset pulse
were needed — the two failure flavors differ exactly in the
drive-side aftermath, and both are now journal-backed.

## 5. Mixed-vendor chain and encoder cross-calibration — PROSE

Recorded live on the bench, artifacts not retained; treat as context,
not as specification:

- First non-Inovance CiA402 slave (Leadshine CL3C-EC507) reached OP on
  the same chain as the SV660Ns; CSP confirmed via `mode_display = 8`.
- The stepper's `cnt_per_rev = 10000` was cross-calibrated against a
  coupled, disabled SV660N's 23-bit encoder: 10000.04 (20 rev
  unidirectional), 10000 (fwd/rev × 3), ±0.014 % by a 100-rev
  amplification run; coupling backlash ≈ 0.09°.
- A single-axis validation report from 2026-06-18 (tables in a rendered
  report, no CSV) measured ~507 scans/s at the same 2 ms / SYNC0
  configuration — a secondary corroborator of §1.

## 6. Known limits and unmet gates

- The kernel is not RT: the same 2 ms fixture **self-trips its scan
  watchdog within ~90 s on a macOS development host with no fault
  injected** (PROSE — observed during soak-tool development, no
  retained log). 2 ms is an edge-hardware number, not a laptop number.
- The soak gate is now closed with a retained log
  (`data/soak-4h-20260908.csv`): **4.05 h at 1 Hz sampling on the main
  build — mean 500.00 scans/s, worst one-second sample 498.8 scans/s,
  zero watchdog trips, zero unhealthy samples across all 14,400
  rows**. The 7.85-day counter snapshot in §1 still proves only a
  lifetime mean; sub-second jitter remains uncharacterized beyond
  what the 1 Hz deltas bound.
- §4's held-enable acceptance (re-enable across a re-walk with
  `actual_pos` Δ = 0) remains TRANSCRIPT-grade — the 2026-09-08 re-runs
  were done with the axis deliberately unenabled.
- An earlier README claim that the in-cycle gear path cut inter-axis
  sync error by 29 % did not survive re-analysis of its raw data (the
  analysis method was not committed, and the improvement is smaller
  than the measurement's window sensitivity); the claim has been
  withdrawn in place (`examples/eg_gear_incycle/`).
- ESI-driven modular couplers are out of scope: bring-up of an
  ESI-modular EtherCAT coupler head fails at `into_op` (phantom RxPDO
  assignment; runtime CoE object-dictionary enumeration is not
  implemented). The raw debug log of that failure exists; the RTU head
  of the same coupler family is the supported path (see
  [`modbus-rtu.md`](modbus-rtu.md)).
