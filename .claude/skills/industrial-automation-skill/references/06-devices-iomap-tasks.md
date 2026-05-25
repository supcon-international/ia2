# Config shapes: devices, iomap, tasks

These are the exact JSON shapes the `cs device set` / `cs iomap set` / `cs tasks set` commands accept (and that `... get` returns). Get → edit → set. Field names are snake_case; the server validates and 422s on a wrong shape.

> **`cs device set NAME` takes the full `Device` shape** — not just the config below. The body needs a top-level `"name"` (which must equal `NAME`, else the server 400s) **and** a `"protocol"` discriminator (`"modbus"` | `"ethercat"`), then the protocol's own fields. The CLI passes `--from` through verbatim (it does *not* inject `name`/`protocol` from the positional arg). This is exactly what `cs device get NAME` prints, so get → edit → set round-trips. (`cs iomap set` / `cs tasks set` have no such envelope — their bodies start at `mappings` / `tasks` directly.)

---

## Modbus device

The transport is a **tagged union** on `kind`. This is the post-RTU schema; old projects with flat `host`/`port` still load (auto-upgraded to `kind:"tcp"`), but always **write** the new shape.

### TCP
```json
{
  "name": "hmi",
  "protocol": "modbus",
  "transport": { "kind": "tcp", "host": "192.168.1.50", "port": 502 },
  "slave_id": 1,
  "poll_interval_ms": 100,
  "channels": [
    { "name": "estop",   "kind": "discrete_input",   "address": 0 },
    { "name": "start",   "kind": "discrete_input",   "address": 1 },
    { "name": "valve",   "kind": "coil",             "address": 0 },
    { "name": "level",   "kind": "holding_register", "address": 0 },
    { "name": "temp",    "kind": "input_register",   "address": 0 }
  ]
}
```

### RTU (serial)
```json
{
  "name": "flow_meter",
  "protocol": "modbus",
  "transport": {
    "kind": "rtu",
    "serial_device": "/dev/cu.usbserial-A1B2",
    "baud_rate": 9600,
    "data_bits": "eight",
    "stop_bits": "one",
    "parity": "none"
  },
  "slave_id": 1,
  "poll_interval_ms": 200,
  "channels": [ { "name": "valve", "kind": "coil", "address": 0 } ]
}
```

- `serial_device`: macOS `/dev/cu.usbserial-*`, Linux `/dev/ttyUSB0` or `/dev/ttyS0`, Windows `COM3`.
- `data_bits`: `five` | `six` | `seven` | `eight` (default `eight`).
- `stop_bits`: `one` | `two` (default `one`).
- `parity`: `none` | `even` | `odd` (default `none`).
- The RTU defaults are 8-N-1, so a minimal `{ "kind":"rtu", "serial_device":"…", "baud_rate":9600 }` is valid input — the other three fields fill in.

### Channel `kind` semantics
| kind | Modbus function | read | write |
|---|---|---|---|
| `coil` | 01/05 | ✓ | ✓ |
| `discrete_input` | 02 | ✓ | ✗ (read-only on the wire) |
| `holding_register` | 03/06 | ✓ | ✓ |
| `input_register` | 04 | ✓ | ✗ |

`address` is the 0-based register/coil address. An iomap `direction: output` against a read-only channel (`discrete_input`/`input_register`) is a type error.

---

## EtherCAT device

```json
{
  "name": "servo_bus",
  "protocol": "ethercat",
  "nic": "_sim",
  "cycle_us": 1000,
  "slaves": [
    { "index": 0, "name": "EK1100", "vendor_id": 2, "product_id": 72100946 },
    { "index": 1, "name": "EL2008", "vendor_id": 2, "product_id": 131608658 }
  ],
  "channels": [
    {
      "name": "do_0", "slave_index": 1, "direction": "rx_pdo",
      "pdo_index": 28672, "sub_index": 1, "bit_length": 1,
      "data_type": "bool", "pdi_byte_offset": 0, "pdi_bit_offset": 0
    }
  ]
}
```

- `nic`: `"_sim"` (or `""`) → in-memory simulator, runs anywhere (macOS dev, CI). Any real interface name (`"eth0"`, `"en7"`) → real `ethercrab` master. **Real mode is Linux + `CAP_NET_RAW` only.**
- `direction`: `tx_pdo` (slave→master, i.e. an **input** to your program) | `rx_pdo` (master→slave, an **output**).
- `data_type`: `bool` `u8` `i8` `u16` `i16` `u32` `i32` `real`.
- `pdi_byte_offset` / `pdi_bit_offset`: where this entry sits in the slave's process-data image. **Required for real hardware**; you read these off the slave's ESI/datasheet. Sim mode ignores them. `bit_length < 8` channels (digital I/O packed into a byte) use the bit offset.
- `pdo_index` / `sub_index`: CoE object dictionary coordinates — informational/documentation in this version; the cyclic exchange uses the byte/bit offsets.

---

## IoMap

```json
{
  "mappings": [
    { "application": "main", "variable": "estop_in",   "device": "hmi", "channel": "estop", "direction": "input"  },
    { "application": "main", "variable": "valve_open",  "device": "hmi", "channel": "valve", "direction": "output" }
  ]
}
```

**Five fields, all required:**
- `application` — the POU name the variable lives in (e.g. `"main"`). **Skipping this is the #1 422 cause.**
- `variable` — the IEC variable name in that POU.
- `device` — a device name from `cs device list`.
- `channel` — a channel name on that device.
- `direction` — `input` (channel→variable, read before run_round) | `output` (variable→channel, written after run_round).

Bindings that reference an unknown device/variable/channel are skipped at run time with a warning — they don't fail the run. But a wrong *shape* (missing field) 422s the `set`.

---

## Tasks (tasks.toml)

```json
{
  "tasks":    [ { "name": "fast", "interval_ms": 50,  "priority": 1 } ],
  "programs": [ { "instance": "main_inst", "program": "main", "task": "fast" } ]
}
```

- `tasks[].interval_ms` becomes `TASK fast(INTERVAL := T#50ms, PRIORITY := 1)` in the synthesized CONFIGURATION. Periodic only (event tasks not supported yet).
- `programs[].program` must be a **PROGRAM**-kind POU (not FB/FUNCTION).
- `programs[].instance` is the instance name; `task` references a `tasks[].name`.
- **Keep `programs` length 1.** `cs run` (whole-schedule) errors if 2+ PROGRAMs are scheduled — ironplc emits only one PROGRAM per compilation. Multiple tasks are fine; multiple PROGRAM instances are not (yet).

The scan-loop cadence comes from the bound task's `interval_ms` (the bridge throttles there, because the vendored ironplc doesn't populate the VM task table from CONFIGURATION). So `interval_ms` is the real knob for "how fast does my program scan".
