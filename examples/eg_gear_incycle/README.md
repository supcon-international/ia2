# eg_gear_incycle — in-cycle electronic gear (B-tier motion)

A follower servo whose `target_position` is generated **inside the EtherCAT
cyclic loop**, strictly SYNC0-aligned, by the runtime's gear engine — not in
the PLC scan plane. Runs under `nic = "_sim"` with **no hardware**.

## Why in-cycle

The stock ("A-tier") way to do an electronic gear is to compute the follower
target in the PLC program each scan (~500 Hz) and let the cyclic loop copy it
to the wire. The two planes are asynchronous, so up to one scan of phase
jitter sits between "target computed" and "frame sent". That jitter is
invisible on a slow single axis but becomes the dominant term in inter-axis
sync error at higher speed / for multi-axis gearing / for electronic CAM.

This example uses the `[[gear]]` device feature: the loop computes the target
every bus cycle from a master source, cycle-aligned. The PLC only feeds slow
parameters (ratio / engage / …) through named channels that the device routes
into a lock-free struct instead of PDI bytes, and reads engaged/trip back.
Equivalent to the TwinCAT NC / SoftMotion split.

Field measurement on a dual-SV660N bench: at 13 rpm, 2:1, the in-cycle path
cut mean inter-axis sync error from 1.02° to 0.73° (−29%), matching the
predicted scan-plane jitter term; at low speed the two are equal (error is
dominated by the drives' own following lag).

## What this project shows

- A `[[gear]]` block (`devices/servo.toml`) with a **virtual master** (a
  software accumulator) driving one follower — so it runs with no second axis.
- The PLC program (`pous/main.st`) runs only the CiA402 enable chain and
  forwards gear parameters; it **never writes `target_position`** (the loop
  owns those bytes).
- The engine's fast-plane safety interlocks (all enforced in-loop, so no
  slow-plane mistake can bypass them): shadow-actual until Operation Enabled
  (zero-jump at enable), engage refused unless `max_travel > 0`, ratio/phase
  latched at the engage edge, overtravel hard-clamped to `±max_travel` then
  tripped, re-arm required after an enable loss, non-finite params rejected,
  and (for an axis master) the ±2^31 encoder wrap handled as a wrapping delta.

## Run it (sim, no hardware)

The sim models an *ideal* follower (always Operation Enabled, actual == last
commanded target), so virtual-master gearing is fully exercisable:

```bash
cs project open examples/eg_gear_incycle
cs --project eg_gear_incycle run

cs --project eg_gear_incycle runtime force max_travel 8388608   # 1-rev limit (counts)
cs --project eg_gear_incycle runtime force master_vel  200      # counts / 2 ms cycle
cs --project eg_gear_incycle runtime force ratio_num   2
cs --project eg_gear_incycle runtime force ratio_den   1
cs --project eg_gear_incycle runtime force ratio_step  0.02     # soft-engage ramp
cs --project eg_gear_incycle runtime force engage      1
```

`gear_engaged` goes true and the follower's `target_position` climbs at
~2× `master_vel`; after ~1 rev of travel `gear_trip` latches and it holds.
Force `engage 0` to clear the trip. Force `max_travel 0` and the engine
refuses to engage (locked-by-default). Try `ratio_step -0.1` or a NaN ratio
and nothing runs away — the engine sanitizes both.

## To a real two-axis bench

Set `nic` to your EtherCAT NIC, add the master SV660N as `slave 1`, and change
the gear master to read its feedback:

```toml
master = { kind = "axis", slave_index = 1, actual_pos_offset = 4 }
```

Now the follower gears off the master's real `0x6064` each cycle. Everything
else (enable chain, parameters, interlocks) is identical.

## Files

`devices/servo.toml` (follower PDOs + `[[gear]]`) · `iomap.toml` (enable-chain
+ gear-parameter channels; `target_position` deliberately unmapped) ·
`pous/main.st` (CiA402 + parameter passthrough) · `tasks.toml` (2 ms task).
