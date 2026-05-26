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

Modbus channels + iomap are JSON; use get/edit/set. The `device set` body is the full `Device` — top-level `name` + `protocol`, matching `cs device get`. (Full shapes: `06-devices-iomap-tasks.md`.)

```bash
cs agent run --label "Wire HMI to tank_ctrl" --server "$SRV" -- bash -c '
set -e
SRV="'"$SRV"'"

# device, then configure its channels via set
cs --project tank_ctrl device create hmi --protocol modbus --server "$SRV"
cs --project tank_ctrl device set hmi --server "$SRV" --from - <<"JSON"
{ "name": "hmi", "protocol": "modbus",
  "transport": { "kind": "tcp", "host": "127.0.0.1", "port": 5502 },
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
{ "name": "hmi", "protocol": "modbus",
  "transport": { "kind": "rtu", "serial_device": "/dev/cu.usbserial-A1B2",
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

Deploy ships the project **and** the `ia2-runtime` binary — but only if a **Linux ELF** for the edge's arch is present in `target/` (the deploy guards against shipping a wrong-arch/host binary, e.g. a macOS build); otherwise it carries forward the runtime already on the box. So cross-compile `ia2-runtime` for the edge's arch yourself before a binary-bearing deploy — there's no CI building artifacts. The edge runs headless; RETAIN state lives in `<install_dir>/state/retain.json` on the box.

---

## G. Drive / debug a deployed edge runtime (`--edge`)

`cs runtime …` and the introspection commands take `--edge <name>` to hit a *deployed* edge instead of the local server — same surface as the web **Edge → Debug** tab, so the pokes render in the IDE's agent-takeover overlay.

```bash
cs --project tank_ctrl edge scan   field_pi --server "$SRV"            # connect status + discovered EtherCAT topology
cs --project tank_ctrl edge logs   field_pi --tail 80 --server "$SRV"  # OP transitions, bus errors
cs --project tank_ctrl runtime status  --edge field_pi --json --server "$SRV"       # live snapshot from the box
cs --project tank_ctrl runtime force   --edge field_pi --server "$SRV" -- speed 500  # NB the `--` (negatives need it: -- speed -500)
cs --project tank_ctrl runtime unforce --edge field_pi speed --server "$SRV"         # release — IMPORTANT
```

**Transport:** each `--edge` call is one-shot `ssh host curl 127.0.0.1:<runtime_port>/<verb>` — a fresh SSH handshake per call (see `02-cli-reference.md`). Fine for occasional pokes; a poor fit for anything resembling a control loop.

**`force --edge` is a debug override, not a setpoint channel — know when *not* to use it:**
- Driving hardware for more than a quick *supervised* poke via `force` is a smell. For a real, repeatable setpoint make the variable an **iomap-bound input** (HMI register / recipe) or compute it in **POU logic** (e.g. a motion profile), and change it through the normal data path. Keep `force` for "pin this for a moment to see what happens."
- For **unattended / throughput / tight-loop** work, run the loop *on the box* (one persistent ssh, local `curl`s) rather than per-call `cs --edge`. The `cs --edge` path earns its hops only when a human is watching the IDE overlay and you want the action on the same audited path the GUI uses.
- It drives **real outputs** on a live bus. Treat a motion variable as you would at the panel, and pair every `force` with `unforce` (`checklists/handoff.md`).

> Worked example — spinning a real Inovance SV660N in CSP: a CiA-402 state machine in the POU did the enable sequence + `target_position := target_position + speed` ramp; the agent only `force --edge`-ed the internal `speed` knob (then `unforce` to stop). The POU did the control; force just injected the setpoint. Fine for a supervised demo — but for a product feature you'd bind `speed` to a real input rather than debug-force it.

---

## H. Multi-project work

When more than one project is open, **every** command needs `--project`. Check first:

```bash
cs project list --server "$SRV"          # see what's open, which is active (*)
cs --project bottling pou save ... 
cs --project mixer    pou save ...        # different window, different project, no cross-talk
```

Only one program runs at a time across the whole server. If `bottling` is running and you `cs --project mixer run`, the bottling program stops. Tell the user before doing that.
