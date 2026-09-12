# Write governance on the live edge — real-hardware evidence

Evidence grades (RAW / TRANSCRIPT / PROSE / CONFIG) are defined in
[`README.md`](README.md). The battery's verbatim HTTP transcript is
committed as `data/governance-battery-20260908.txt`.

## Bench

2026-09-08, the EtherCAT bench from
[`ethercat-2ms-dc-sync.md`](ethercat-2ms-dc-sync.md): edge runtime
built from main `0700e40` (sha `78d89dda…`), 2 ms task, three-slave
chain, drives **never enabled** — the battery is zero-motion by
design. `project.toml` carries:

```toml
[governance]
write_mode = "allowlist"

[[governance.rules]]
variable = "max_rev"
min = 0.0
max = 2.0
```

`max_rev` is a pure PLC variable (the motion-latch limit) bound to no
device channel, so clamped writes have no physical effect while the
drive is disabled — and with an allowlist this small, every
motion-adjacent variable is deny-by-default.

## Battery results — RAW (verbatim transcript committed)

| # | Request | Result |
|---|---|---|
| T1 | `/write jog` (unlisted) | **HTTP 403**, body names the reason: `write_mode = allowlist, no rule allows it` |
| T2 | `/write enable = 1` (unlisted) | **HTTP 403** — the enable path itself is governed |
| T3 | `/write max_rev = 5.0` | **HTTP 200, clamped**: echo returns the applied value 2.0 (bit-packed REAL), not the requested 5.0 |
| T4 | `/write max_rev = 1.5` (in range) | HTTP 200, echoed unchanged |
| T5 | `/write max_rev = NaN` | **HTTP 403** — `value NaN cannot be checked against the rule's min/max`; denied, never "clamped" |
| T6 | `/write max_rev = 0.0` (restore) | HTTP 200 |

## Audit ring — RAW

`GET /audit` after the battery (same transcript): the clamped write is
recorded as `{origin: "bench-verify", requested: 1084227584, applied:
1073741824, result: "clamped"}` — requested and applied both kept, the
self-declared `X-IA2-Origin` recorded verbatim; the NaN and unlisted
denials are recorded with their full reason strings and no `applied`;
the CL3C fault-reset `force`/`unforce` pulses that bracketed the
session appear too (force bypasses governance by ADR-0002 decision,
and the audit shows exactly that happening).

## Describe against the real machine — TRANSCRIPT

`cs get devices/<name>/describe` was generated from the live bench
project and checked against the physical chain: all three slaves with
correct identities and order (2 × Inovance SV660N witnesses, Leadshine
CL3C-EC507 at index 2), `dc_sync = "sync0"` per slave, 15 channels
including the read-only coupled-witness bindings, `write_mode:
allowlist` reported, and `write_rules: []` — correct, because the one
rule names a variable bound to no device channel and describe filters
rules to the device's bound variables. The full describe JSON is
retained in the private bench archive (it reproduces a private
project's config, so it is not committed here).

## Limits

- MQTT northbound write rejection is **unverified** on hardware (no
  broker configured on this bench).
- The IDE takeover-overlay path was verified the same day on a local
  isolated IDE instance (sim project, human observer watching the
  overlay, four cases fired from outside the GUI): an origin-less
  write flashed `write <var> (unattributed)`; a self-declared `mqtt`
  write flashed `write <var> — mqtt (self-declared)`; a `gui`-labelled
  write flashed nothing; and a force/unforce pair flashed both calls
  with their self-declared label — every mutating call surfaced,
  including the release. Human-witnessed (TRANSCRIPT); the overlay
  labels a self-declared origin honestly rather than trusting it.
- Load-time validation of a broken `[governance]` table (unknown keys,
  `min > max`) is covered by unit tests, not re-proven on the bench.
