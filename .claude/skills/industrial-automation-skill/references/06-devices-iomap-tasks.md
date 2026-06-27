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
- `rs485` (optional, default off): **Linux half-duplex direction control** (`TIOCSRS485`). Add it when an RTU port opens fine but every request times out with baud/parity/slave/wiring all correct — a **RTS-gated** USB-485 adapter never drives the bus in plain serial mode (the connect error now says so explicitly). Shape: `{ "rts_on_send": true, "rx_during_tx": false, "delay_rts_before_send_ms": 0, "delay_rts_after_send_ms": 0 }`. `rts_on_send` false = drive on RTS *low*; `rx_during_tx` true tolerates 2-wire echo; the delays are turnaround settle times. Linux-only (no-op + warning elsewhere); omit entirely for auto-direction adapters. If it's still silent after enabling, the adapter/coupler itself is suspect — prove the bus with a known-good auto-direction dongle or a separate Modbus master.

### Channel `kind` semantics
| kind | Modbus function | read | write |
|---|---|---|---|
| `coil` | 01/05 | ✓ | ✓ |
| `discrete_input` | 02 | ✓ | ✗ (read-only on the wire) |
| `holding_register` | 03/06 | ✓ | ✓ |
| `input_register` | 04 | ✓ | ✗ |

`address` is the 0-based register/coil address. An iomap `direction: output` against a read-only channel (`discrete_input`/`input_register`) is a type error.

### Register data types (direct-to-instrument)

Register channels take two optional fields (default `"u16"` / `"hi_lo"` — old projects load unchanged):

```json
{ "name": "flow",  "kind": "input_register", "address": 2,  "data_type": "f32", "word_order": "hi_lo" },
{ "name": "temp",  "kind": "input_register", "address": 20, "data_type": "i16" },
{ "name": "total", "kind": "input_register", "address": 30, "data_type": "u32", "word_order": "lo_hi" }
```

- `data_type`: `u16` (default) | `i16` (signed, negatives survive) | `u32` | `i32` | `f32`. 32-bit types span **two consecutive registers** starting at `address` — the norm for instrument floats and totalizers.
- `word_order` (32-bit only): `hi_lo` = ABCD (Modbus-spec default) | `lo_hi` = CDAB (common on Chinese instruments). If a float reads as garbage (e.g. 1.18e-38), flip this first.
- Coils/discretes ignore both fields.

### Polling model (scales to hundreds of channels)

The adapter does NOT issue one request per channel. At connect it merges channels into contiguous **read spans** per function code (bridging gaps ≤8 units), and a background task refreshes the whole mirror every `poll_interval_ms` with a handful of bulk reads. `read_channel` serves the mirror; writes go through a command queue so the single connection (mandatory for RTU serial) is never used concurrently. A failed poll keeps last-known values and retries next tick.

---

## EtherCAT device

```json
{
  "name": "servo_bus",
  "protocol": "ethercat",
  "nic": "_sim",
  "cycle_us": 1000,
  "dc_sync": "off",
  "dc_static_sync_iterations": 0,
  "slaves": [
    { "index": 0, "name": "EK1100", "vendor_id": 2, "product_id": 72100946 },
    {
      "index": 1, "name": "SV660N", "vendor_id": 1048576, "product_id": 786701,
      "dc_sync": "sync0",
      "init_sdo": [
        { "index": 24672, "sub_index": 0, "value": 8, "bits": 8 }
      ]
    }
  ],
  "channels": [
    {
      "name": "do_0", "slave_index": 0, "direction": "rx_pdo",
      "pdo_index": 28672, "sub_index": 1, "bit_length": 1,
      "data_type": "bool", "pdi_byte_offset": 0, "pdi_bit_offset": 0
    }
  ]
}
```

- `nic`: `"_sim"` (or `""`) → in-memory simulator, runs anywhere (macOS dev, CI). Any real interface name (`"eth0"`, `"en7"`) → real `ethercrab` master. **Real mode is Linux + `CAP_NET_RAW` only.**
- `dc_sync` (default `"off"`): bus-level Distributed-clock mode. `"off"` = free-run (right for IO couplers / simple slaves). `"sync0"` = enable the SYNC0 pulse (period = `cycle_us`) — **servo drives like the Inovance SV660N need this to reach OP**; without it the SAFE-OP→OP transition times out.
- `slaves[].dc_sync` (optional, default = inherit the bus-level `dc_sync`): per-SubDevice override for **mixed buses**. A coupler+IO **and** a servo on one ring: leave the bus `"off"` and set just the servo's slave to `"sync0"` (or vice-versa). The bus takes the DC path whenever *any* slave ends up `"sync0"`; slaves left `"off"` free-run inside that DC bus, so a coupler that can't do DC won't block OP.
- `dc_static_sync_iterations` (default `0`): iterations of ethercrab's init-time static-drift compensation (a burst of FRMW frames). `0` skips it — short buses come up fine, and on a non-RT host one lost frame mid-burst aborts init with `Timeout(Pdu)`. Raise to `1000`–`10000` on longer DC buses where clock convergence at OP-entry matters.
- `slaves[].init_sdo` (optional): CoE SDO writes applied in **PRE-OP on every connect**, in order, before PDO mapping is read and before SAFE-OP. This is how drives whose setup doesn't persist in EEPROM get configured each power-up — e.g. the SV660N needs `0x6060 = 8` (CSP mode) every boot, and PDO remapping (`0x1C12`/`0x1C13` → `0x16xx`/`0x1Axx`) goes here too. Each entry: `index` (CoE object), `sub_index`, `value` (signed/unsigned, fits `bits`), `bits` (`8` | `16` | `32`). A failed write aborts init — silently running a drive in the wrong mode is worse than not starting. *(Note the JSON uses decimal: `24672` = `0x6060`.)*
- `direction`: `tx_pdo` (slave→master, i.e. an **input** to your program) | `rx_pdo` (master→slave, an **output**).
- `data_type`: `bool` `u8` `i8` `u16` `i16` `u32` `i32` `real`.
- `pdi_byte_offset` / `pdi_bit_offset`: where this entry sits in the slave's process-data image. **Required for real hardware**; you read these off the slave's ESI/datasheet. Sim mode ignores them. `bit_length < 8` channels (digital I/O packed into a byte) use the bit offset.
- `pdo_index` / `sub_index`: CoE object dictionary coordinates — informational/documentation in this version; the cyclic exchange uses the byte/bit offsets.
- **Capacity**: the master is sized for plant-scale buses — up to **128 subdevices / 4 KiB process image** per device (a 1000-point AI/AO/DI/DO project uses ~660 B). One device = one NIC = one bus; use multiple devices for multiple NICs.

### Bring-up mode (`bringup`)

Default is `{ "mode": "auto" }` — discover process data from the device's runtime CoE PDO-assignment objects (`0x1C12`/`0x1C13`). That works for fixed-PDO servos and slices.

**ESI-driven modular couplers** (whose assembled module PDOs never appear over runtime CoE — `0x1C12` read-only, `0xF030` absent) need `{ "mode": "esi_modular", "esi_path": "esi/coupler.xml" }`. The process image is built from the device's ESI (.xml) file + the modules it reports (object `0xF050`), not hand-entered. Workflow:

```bash
# 1. drop the vendor ESI into the project: <project>/esi/coupler.xml
# 2. create the device with bringup = esi_modular (esi_path is project-relative)
# 3. assemble channels from the ESI + the modules present, in slot order:
cs device esi-assemble coupler --idents 0x10,0x20,0x30   # hex or decimal
```

`esi-assemble` parses the ESI, looks each detected ident up in the module table, concatenates the modules' PDO entries into the input/output images (tracking byte/bit offsets), and **replaces** the device's `channels` with the result — so the UI shows them and iomap can bind them. The idents come from the coupler's `0xF050` scan or the modules you physically installed. Channel names are `m<slot>_<entry>` (slot-namespaced so duplicate module types stay unique).

> Real-bus cyclic I/O for `esi_modular` (master-programmed SyncManager/FMMU + logical-RW exchange) is validated against the physical coupler separately; the parsing, assembly, and offline channel authoring above are hardware-independent and the recommended way to author + verify the layout first (run it in `nic: "_sim"` to exercise the program against the assembled channels).

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

---

## OPC UA device (southbound to an existing DCS)

When the site already runs a DCS/PLC that owns the physical I/O, IA2 sits **above** it as the supervisory layer: read PV tags, write SP/command tags. The DCS keeps base regulatory control and safety.

```json
{
  "name": "dcs",
  "protocol": "opcua",
  "endpoint_url": "opc.tcp://10.0.0.10:4840",
  "security": "none",
  "auth": { "kind": "anonymous" },
  "poll_interval_ms": 500,
  "channels": [
    { "name": "ft0202_pv",  "node_id": "ns=2;s=FT0202.PV",  "data_type": "f64", "access": "read" },
    { "name": "fv0203_cmd", "node_id": "ns=2;s=FV0203.CMD", "data_type": "f64", "access": "write", "failsafe": 0.0 }
  ]
}
```

- `auth`: `{ "kind": "anonymous" }` or `{ "kind": "user_password", "username": "...", "password": "..." }`. `security` is `"none"` in v1 (typical for trusted control-network segments / DA-gateway hops).
- `node_id`: the full NodeId string exactly as UaExpert shows it — `ns=2;s=Tag.Path` (string) or `ns=3;i=1042` (numeric).
- `data_type`: `bool` `i16` `u16` `i32` `u32` `f32` `f64`. Floats land on REAL PLC vars with fractions intact (f64 is narrowed to f32).
- `access`: `read` tags are polled into a mirror (ONE bulk Read per `poll_interval_ms` for ALL tags — hundreds of tags stay one round-trip); `write` tags get a direct Write service call when the scan loop pushes an output.
- `failsafe`: optional value written on shutdown/trip. **Leave unset by default** — on a supervisory layer the DCS below keeps authority; only set it for tags that are exclusively IA2's (e.g. a supervisory-enable flag).
- **OPC DA** (classic, COM/DCOM, Windows-only): IA2 does not speak DA. Route DA servers through a DA→UA gateway (KEPServerEX, Matrikon UA Proxy) and point IA2 at the gateway's UA endpoint — the standard architecture for Linux edges.

---

## Northbound (northbound.toml — MQTT to supOS / Tier0)

How the **edge runtime** publishes live data up to the plant platform. MQTT only by design. Managed via `cs northbound get/set` (JSON shape below) or by editing `northbound.toml`; the edge runtime applies it at startup (redeploy/restart to change).

```json
{
  "mqtt": {
    "enabled": true,
    "broker_host": "10.0.0.5",
    "broker_port": 1883,
    "client_id": "",
    "username": "",
    "password": "",
    "topic_prefix": "",
    "publish_interval_ms": 1000,
    "qos": 0,
    "allow_write": false
  }
}
```

- `topic_prefix` defaults to `ia2/<project>`; `client_id` to `ia2-<project>`.
- Topics: `<prefix>/status` (retained `online`/`offline` with LWT — the platform sees crashes), `<prefix>/snapshot` (periodic `{"ts_us":…,"scan":…,"values":{"FT0202":12.7,"alarm_h":true}}` — typed JSON values, one JSON-path hop to map into the platform), and `<prefix>/write` (subscribed only when `allow_write`, payload `{"name":"sp_flow","value":12.5}` → one-shot variable write).
- `allow_write` is **off by default** — turning the northbound link into a control path is an explicit decision. Writes are one-shot (program can overwrite next scan); latch setpoints in program logic.
