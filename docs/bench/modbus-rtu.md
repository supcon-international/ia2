# Modbus RTU on real hardware — field-verified evidence

Evidence grades (RAW / TRANSCRIPT / PROSE) are defined in
[`README.md`](README.md). RAW numbers are re-derivable by running
`python3 docs/bench/data/recompute.py`.

## Bench

x86_64 edge box; FTDI FT232 USB-RS485 adapter (auto-direction — no
`TIOCSRS485` ioctl needed) on `/dev/ttyUSB0`; ZHICAN NX6-1261-RTU
coupler head, station 1, **9600 baud 8-E-1**, carrying DI/DO/AI/AO
modules (NX6-1231-AD4V-D analog in, NX6-1232-DA4VC-D analog out).
Actuator on the analog loop: a 0–10 V motorized ball valve
(FRY-02TQ911F-16P-DN15).

## 1. Register map: measured, not trusted — CONFIG

The shipped example [`../../examples/nx6_modbus/`](../../examples/nx6_modbus/)
(PR #14, commit `50d1d15`) carries the register map verified
register-by-register with a multimeter on every terminal:

- The DO word lives at **40193 (address 192) — not the manual's
  40033**.
- The DA4VC-D analog outputs sit at 40194–40197 with a **+1
  register-to-terminal shift** (write 193 → terminal V1; the manual's
  arithmetic lands one off).
- Inputs match the manual (DI 40001, AI 40065–68); module-count and
  system-error registers read as documented.
- Scaling: **3200 counts/V** (0–32000 ↔ 0–10 V).
- A photoelectric switch on DI0 delivered clean 0↔1 transitions with
  no debounce artifacts (TRANSCRIPT — counted live, no log retained).

The point is not this one coupler's quirks; it is that IA2's
text-first device config made the discrepancy easy to find, encode,
and ship as a runnable example.

## 2. RTU in the scan loop, concurrently with EtherCAT — RAW

`data/scan-cadence-mixed-bus-20260709.csv` (2026-07-09, 75 s): the same
runtime driving 2 EtherCAT servo axes at a 2 ms task **and** polling
the RTU coupler. `coupler_ok = 1` on **373/373 samples** with live
analog feedback read through the chain.

- Cost, honestly stated: this pre-fix build paid ~20 % scan cadence
  (mean 397 scans/s vs 500 EtherCAT-only) because Modbus writes
  blocked the scan thread. The blocking write was fixed upstream
  (`cd7e49c`, PR #20), and the post-fix mixed-bus measurement was
  retaken on 2026-09-08: mean 500.0 scans/s with the coupler in-loop
  (`data/scan-cadence-mixed-bus-postfix-20260908.csv`; details in the
  EtherCAT document, §1).

## 3. Analog loop calibration through the coupler — RAW

`data/valve-calibration-20260701.csv`, `valve-step-20260701.csv`,
`valve-deadband-20260701.csv` (2026-07-01). Command written to the AO
module, valve position read back through the AI module, both over the
RTU link. Scope, honestly stated: these runs used standalone data
collection scripts on the edge (same wiring, same protocol settings) —
they characterize the coupler + analog chain + valve, not the IA2
runtime's own polling loop (§2 covers that).

| Measurement | Value |
|---|---|
| Linearity (42-point up/down sweep) | `fb = 0.9963·cmd − 0.0134`, **R² = 0.99991**, max residual 65 mV |
| Hysteresis | **29 mV mean / 82 mV max** (worst at 7.5 and 9.0 V) |
| Step response 0→10 V | no overshoot (peak 9.958 V) |
| Feedback quantization | nonzero jump median **0.192 V** (≈ 0.2 V → ~2 % FS, valve-side, not the 12-bit AI); feedback held across **7 of 18** adjacent command transitions |
| Endpoints | 0.045–0.058 V closed, 9.936–9.938 V open |

One AO channel on this unit is dead (write and readback succeed,
terminal stuck at 0 V — recorded in the shipped config) — a reminder
that a happy register readback proves the bus, not the terminal.
Field verification means meters.

## 4. Failsafe behaviour on runtime stop — measured (2026-09-08)

`data/failsafe-stop-journal-20260908.txt` plus a human-witnessed bench
observation, two rounds, consistent:

- With the analog output driven to 5.0 V (valve at mid-travel,
  feedback 4.93 V), stopping the runtime **immediately** started the
  valve closing — the runtime's own failsafe zeroed the real outputs
  before process exit; the coupler's comms watchdog (which would act
  only after a delay) was not the mechanism. End-state feedback after
  restart: 156 counts ≈ 0.049 V, inside the calibrated closed
  endpoint (§3).
- The same failsafe pass erroneously writes zeros to the **read-only
  input registers** too (DI word, AI 1–4), producing
  `Illegal data address` protocol errors — logged, and demonstrably
  non-blocking: the journal shows the flow continuing to
  `failsafe applied devices=2`. Root cause: channel configs carry no
  read/write direction, so the failsafe treats every holding register
  as writable. Tracked as a known defect (direction semantics); the
  measured behaviour above is why it is graded a correctness defect,
  not an open safety hole **on this device** — ordering or error
  handling on other couplers could differ, which the fix must not
  rely on.

## 5. Known limits

- 9600 8-E-1 with a ~50 ms coupler poll is the only RTU configuration
  measured; no throughput or higher-baud characterization exists.
- The EtherCAT head of this coupler family (ESI-modular) is NOT
  supported — bring-up fails at `into_op`; the raw debug log of that
  failure is part of why the RTU head is the recommended path.
- The valve numbers characterize this valve; the transferable results
  are the method (meter-verified register map, up/down sweep with
  hysteresis separation) and the coupler/AO/AI chain behaviour.
