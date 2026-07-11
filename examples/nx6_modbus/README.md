# nx6_modbus — ZHICAN NX6-1261-RTU coupler over Modbus-RTU (field-verified)

Drives a **ZHICAN NX6-1261-RTU coupler + 4 I/O modules** over **Modbus-RTU**:
a photoelectric switch on DI and a 0-10V motorized ball valve (FRY-02T DN15)
with closed-loop position feedback on AO/AI. Pure configuration — IA2's stock
Modbus-RTU support, no runtime changes. (Also a working alternative to the
EtherCAT path for the same module family.)

## Module stack & register map (field-verified 2026-07-01)

> ⚠️ **The output region does not match the vendor manual's fixed-partition
> table** (manual p.9). Outputs are packed at the start of the AO region in
> enumeration order, and the DA register→terminal order is shifted by one.
> The table below is what the hardware actually does — verified by writing
> registers and metering every terminal.

| Module | Function | Holding register | IA2 channel / address | Verified |
|---|---|---|---|---|
| NX6-1221-ID16N | 16 DI | 40001 | `di_word` / 0 | ✓ photoelectric switch |
| NX6-1231-AD4V-D | 4 AI (voltage) | 40065-40068 | `ai1..ai4` / 64-67 | ✓ `ai1` = valve feedback (terminal A) |
| NX6-1222-OD16N | 16 DO | **40193** (not 40033!) | `do_word` / **192** | ✓ write test lights channel LEDs |
| NX6-1232-DA4VC-D | 4 AO (voltage) | **40194-40197** | `ao1..ao4` / **193-196** | ✓ terminals = **V1/V2/V3/V0** (shifted by one) |

- **Scale**: AO and AI are both 0-10V ↔ 0-32000 (**3200 counts/V**, measured).
  The module-config region (40321+) left at factory zeros already selects the
  0-10V voltage range — nothing to write.
- ⚠️ On our test unit the `ao4` DAC output stage (addr 196 → terminal V0/M0)
  is **damaged** — writes and read-backs succeed but the terminal stays at 0V
  (likely a 24V brush during initial wiring). Valve control uses
  `ao1` (addr 193 → **V1/M1**) instead.
- The coupler only supports **FC 3/6/16** (holding registers), so every
  channel is `kind = "holding_register"`. DI/DO are bits inside a word — use
  WORD bit operations in the program.

## Wiring

1. **RS485**: USB-RS485 adapter on the edge host; adapter **A↔coupler A,
   B↔B** (swap if no response). The coupler's 485 terminal block is pluggable.
2. **Two 24VDC feeds**: Us (coupler) and Up (modules/field) — both required;
   E to ground.
3. **Termination**: add 120Ω at both bus ends if the link is flaky.
4. **DIP switches** (manual p.9): SW1-6 = station address in binary (this
   project uses **1** → only SW1=ON); SW7-8 = baud rate, **9600** → both OFF
   (factory default). Serial framing is fixed **8 data / 1 stop / even
   parity**.
5. **Field devices**: photoelectric switch (OMRON E3ZG-R61-S) on **ID16N
   channel 0 = DI0**. Valve FRY-02TQ911F-16P-DN15: grey (control in) →
   **DA4VC-D V1**, white (signal common, shared by control and feedback) →
   **M1**, brown (feedback out) → **AD4V-D A+**, white common also to A−;
   red/blue 24V from the *same* supply as the coupler (single 0V reference).
   ⚠️ Keep 24V well away from signal terminals — that is how V0 died.

## Commissioning checklist — all four items confirmed

1. ✅ **Serial device**: `/dev/ttyUSB0` = FTDI FT232 (`0403:6001`) with
   hardware auto-direction → no `[transport.rs485]` section needed.
2. ✅ **AO/AI range**: factory-default config region = 0-10V voltage mode.
3. ✅ **Addresses from measurement, not the manual**: see table above;
   `devices/coupler.toml` carries the verified map.
4. ✅ **AO calibration**: 0-10V ↔ 0-32000 linear; closed-loop error ≤ ±0.08V
   across the full range.

## Deploy to an edge host

Use the product deploy path (`cs deploy`) — see `docs/edge-deploy.md` for the
full flow, including one-time edge setup:

```bash
cs project open examples/nx6_modbus
cs --project nx6_modbus edge create bench --host edge   # your ssh host/alias
cs deploy bench                                         # tar → ssh → versioned swap → restart
cs probe  bench                                         # confirm the runtime came up
```

`cs deploy` carries forward the runtime already on the box unless a matching
Linux `ia2-runtime` binary is present to ship. The serial port needs root or
`dialout` membership for the runtime user. A manual rsync + symlink-swap deploy
is possible but is not the supported path.

## Verify / poke (edge runtime HTTP API on 127.0.0.1:13001)

```bash
# link up?  no timeout / CRC errors in the log means good
journalctl -u ia2 --since "30 sec ago" | grep -iE "modbus|coupler|connect"
# read the photoelectric switch: watch di_word / sensor0 flip
curl -s 127.0.0.1:13001/status
# drive the valve: 32000 = 10V ≈ full open, 16000 = 5V, 0 = closed
curl -s -XPOST 127.0.0.1:13001/force -H 'Content-Type: application/json' \
     -d '{"name":"ao1_setpoint","value":16000}'
# release when done
curl -s -XPOST 127.0.0.1:13001/unforce -H 'Content-Type: application/json' \
     -d '{"name":"ao1_setpoint"}'
```

## Validation record

**2026-06-30 — link & inputs** (edge host + FTDI adapter)
- Station 1 / 9600 / 8-E-1, FC3 responses clean (all CRC OK); module
  count (40356) = 4, system error (40389) = 0.
- FTDI FT232 auto-direction adapter needs no `TIOCSRS485` — replaces the
  RTS-gated dongle whose silent TX failure was issue #13.
- DI path: 42 clean 0↔1 flips of DI0 (40001 bit0) from the photoelectric
  switch, no bounce, no cross-talk.
- AI region confirmed at addr 64-67.

**2026-07-01 — outputs & valve closed loop**
- Output map measured terminal-by-terminal (see table): DO word at 40193,
  DA4VC-D at 40194-97 with the V1/V2/V3/V0 terminal shift. **Do not trust
  the manual's partition table — measure.**
- `ao4`/V0 DAC found dead on this unit (constant 0V); control moved to V1.
- Valve closed-loop calibration (control V1/M1, feedback HR40065):
  linearity R² = 0.9999 (fb ≈ 0.996·cmd − 0.013), no overshoot, hysteresis
  29mV avg / 82mV max, stroke ~8.5s each way (10s nominal), closed 0.05V /
  open 9.94V, effective resolution ~0.2V (2% FS).

## Files

`devices/coupler.toml` (Modbus-RTU transport + channels) · `iomap.toml`
(variable↔channel bindings) · `pous/main.st` (test logic) · `tasks.toml`
(50ms task).
