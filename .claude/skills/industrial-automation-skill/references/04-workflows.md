# Workflow recipes

Pattern-match the user's intent to one of these. Each is a complete, copy-adaptable sequence. All assume `CS` and `SRV` are set (see `checklists/first-contact.md`) and that multi-step work is wrapped per `03-agent-sessions.md`.

---

## A. New project from scratch → running

```bash
cs agent run --label "New project: tank_ctrl" --server "$SRV" -- bash -c '
set -e
SRV="'"$SRV"'"

# 1. create (becomes the active project)
cs project create tank_ctrl --server "$SRV"

# 2. main PROGRAM (ST). VAR RETAIN values survive restart.
cs --project tank_ctrl pou save main --server "$SRV" --stdin <<"ST"
PROGRAM main
  VAR level : INT := 0; setpoint : INT := 800; valve_open, pump_on : BOOL; END_VAR
  VAR RETAIN cycle_count : DINT := 0; END_VAR
  cycle_count := cycle_count + 1;
  IF valve_open AND level < 1000 THEN level := level + 20; END_IF;
  IF pump_on   AND level > 0    THEN level := level - 15; END_IF;
  IF    level >= setpoint + 50 THEN valve_open := FALSE; pump_on := TRUE;
  ELSIF level <= setpoint - 50 THEN valve_open := TRUE;  pump_on := FALSE; END_IF;
END_PROGRAM
ST

# 3. validate BEFORE running
cs project check ~/Documents/IA2/tank_ctrl

# 4. run one PROGRAM ad-hoc (no tasks.toml needed for this)
cs --project tank_ctrl run --program main --server "$SRV"
'
```

Then tell the user: "Monitor pane should now show `level` oscillating around 800, `valve_open`/`pump_on` toggling, `cycle_count` climbing."

---

## B. Add a device + wire it to program variables

Modbus channels + iomap are JSON; use get/edit/set. (Full shapes: `06-devices-iomap-tasks.md`.)

```bash
cs agent run --label "Wire HMI to tank_ctrl" --server "$SRV" -- bash -c '
set -e
SRV="'"$SRV"'"

# device, then configure its channels via set
cs --project tank_ctrl device create hmi --protocol modbus --server "$SRV"
cs --project tank_ctrl device set hmi --server "$SRV" --from - <<"JSON"
{ "transport": { "kind": "tcp", "host": "127.0.0.1", "port": 5502 },
  "slave_id": 1, "poll_interval_ms": 100,
  "channels": [
    { "name": "estop",  "kind": "discrete_input",   "address": 0 },
    { "name": "valve",  "kind": "coil",             "address": 0 },
    { "name": "level",  "kind": "holding_register", "address": 0 } ] }
JSON

# iomap — note the mandatory "application" field (the POU name)
cs --project tank_ctrl iomap set --server "$SRV" --from - <<"JSON"
{ "mappings": [
  { "application": "main", "variable": "valve_open", "device": "hmi", "channel": "valve", "direction": "output" },
  { "application": "main", "variable": "level",      "device": "hmi", "channel": "level", "direction": "output" } ] }
JSON

cs project check ~/Documents/IA2/tank_ctrl
'
```

---

## C. Configure tasks.toml + run the full schedule

`cs run` (no `--program`) runs the whole tasks.toml. **It errors if tasks.toml schedules 2+ PROGRAMs** (ironplc limit). One PROGRAM per schedule.

```bash
cs --project tank_ctrl tasks set --server "$SRV" --from - <<'JSON'
{ "tasks":    [ { "name": "fast", "interval_ms": 50, "priority": 1 } ],
  "programs": [ { "instance": "main_inst", "program": "main", "task": "fast" } ] }
JSON
cs --project tank_ctrl run --server "$SRV"   # whole schedule
```

---

## D. Debug session (force / pause / step)

```bash
cs agent run --label "Debug fill logic" --server "$SRV" -- bash -c '
set +e
SRV="'"$SRV"'"
cs --project tank_ctrl run --program main --server "$SRV"; sleep 3
cs --project tank_ctrl runtime force setpoint 200 --server "$SRV"; sleep 3   # tank drains
cs --project tank_ctrl runtime pause  --server "$SRV"; sleep 1              # freeze
cs --project tank_ctrl runtime step 20 --server "$SRV"; sleep 2            # advance exactly 20
cs --project tank_ctrl runtime resume --server "$SRV"; sleep 2
cs --project tank_ctrl runtime unforce setpoint --server "$SRV"           # release — IMPORTANT
cs --project tank_ctrl runtime status --json --server "$SRV"              # confirm no leftover forces
'
```

Always `unforce` what you `force`. A leftover force is invisible until someone wonders why a value won't change.

---

## E. RTU (real serial hardware)

Switch a Modbus device to RTU by setting its transport. macOS device paths look like `/dev/cu.usbserial-XXXX`; Linux `/dev/ttyUSB0`; Windows `COM3`.

```bash
cs --project tank_ctrl device set hmi --server "$SRV" --from - <<'JSON'
{ "transport": { "kind": "rtu", "serial_device": "/dev/cu.usbserial-A1B2",
                 "baud_rate": 9600, "data_bits": "eight", "stop_bits": "one", "parity": "none" },
  "slave_id": 1, "poll_interval_ms": 200,
  "channels": [ { "name": "valve", "kind": "coil", "address": 0 } ] }
JSON
```

RTU is slow — keep `poll_interval_ms` ≥ 200 at 9600 baud. A missing serial device fails the device connect gracefully (logged warning, scan loop continues with that device skipped); it does NOT crash the run.

---

## F. Deploy to an edge controller

```bash
cs --project tank_ctrl edge create field_pi --host pi@plc.local --server "$SRV"
cs --project tank_ctrl edge get field_pi --server "$SRV"     # check install_dir / runtime_port
cs deploy field_pi --server "$SRV"                            # tar → ssh → versioned swap → restart
cs probe  field_pi --server "$SRV"                            # confirm the edge runtime came up
```

Deploy needs `ia2-runtime` cross-compiled for the edge's arch (release.yml builds linux x86_64 + aarch64 artifacts). The edge runs headless; RETAIN state lives in `<install_dir>/state/retain.json` on the box.

---

## G. Multi-project work

When more than one project is open, **every** command needs `--project`. Check first:

```bash
cs project list --server "$SRV"          # see what's open, which is active (*)
cs --project bottling pou save ... 
cs --project mixer    pou save ...        # different window, different project, no cross-talk
```

Only one program runs at a time across the whole server. If `bottling` is running and you `cs --project mixer run`, the bottling program stops. Tell the user before doing that.
