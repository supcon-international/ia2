use serde::{Deserialize, Serialize};
use ts_rs::TS;

// ---------------- Manifest (project.toml) ----------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectManifest {
    pub name: String,
    #[serde(default = "default_version")]
    pub version: String,
}

fn default_version() -> String {
    "0.1".into()
}

// ---------------- POUs (Program Organization Units) ----------------
//
// A POU is one IEC declaration (a single `PROGRAM`, `FUNCTION_BLOCK`,
// or `FUNCTION` block). The file is just the storage medium — one
// `.st` file may declare multiple POUs side by side.
//
// The tree shows POU declarations; the editor reads/writes by file
// path. `PouFile` carries both: the on-disk identifier + the parser-
// derived declaration list inside it.

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum PouType {
    Program,
    FunctionBlock,
    Function,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum PouLanguage {
    /// Structured Text. Only one supported today; the others reserve
    /// the schema slot so adding LD / FBD / IL / SFC later is non-
    /// breaking for stored projects.
    St,
    Ld,
    Fbd,
    Il,
    Sfc,
}

/// One IEC POU declaration parsed out of a `.st` file. Surfaced in the
/// tree as a separate node — a multi-POU file (e.g. a FB next to a
/// PROGRAM) renders as multiple sibling nodes sharing a file path.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PouDecl {
    /// IEC identifier — what `PROGRAM <inst> WITH <task> : <name>`
    /// references in tasks.toml.
    pub name: String,
    #[serde(rename = "type")]
    pub type_: PouType,
    pub language: PouLanguage,
}

/// A `.st` file on disk and the POU declarations parsed from it.
/// The tree section's children are `PouFile`s; the editor opens by
/// `path`.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct PouFile {
    /// Project-relative slash-path under `pous/`, without `.st`
    /// extension (same identifier system the old `Application.name`
    /// used — keeps the URL pattern `/api/pous/{path}` predictable).
    pub path: String,
    /// Declarations in source order. Empty when the file fails to
    /// parse — the tree still shows the file so the user can fix it.
    pub declarations: Vec<PouDecl>,
}

/// Full read response: same as `PouFile` plus the raw source.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct Pou {
    pub path: String,
    pub source: String,
    pub declarations: Vec<PouDecl>,
}

/// Intermediate type used by `ProjectStore::tree_skeleton` — carries
/// each `.st` file's raw source. The server's `/api/project` handler
/// pairs this with `ironplc_bridge::extract_pou_declarations` to build
/// the final `ProjectTree` with parsed POU declarations. Not exported
/// to TS because the frontend never sees this shape directly.
#[derive(Debug, Clone, Serialize)]
pub struct PouFileSource {
    pub path: String,
    pub source: String,
    /// Detected from the file extension — `.st` → `St`, `.ld.json` →
    /// `Ld`. Lets the bridge route parsing/transpilation per file
    /// without re-probing the filesystem.
    pub language: PouLanguage,
}

/// As `ProjectTree`, but with each POU file represented as raw source
/// rather than parsed declarations. The server fills in declarations
/// at the API layer.
#[derive(Debug, Clone, Serialize)]
pub struct ProjectTreeSkeleton {
    pub name: String,
    pub path: String,
    pub pous: Vec<PouFileSource>,
    pub pou_folders: Vec<String>,
    pub devices: Vec<Device>,
    pub device_folders: Vec<String>,
    pub edges: Vec<Edge>,
    pub edge_folders: Vec<String>,
    pub iomap: IoMap,
    pub tasks: Tasks,
}

// ---------------- Devices ----------------

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum Protocol {
    Modbus,
    Ethercat,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(tag = "protocol", rename_all = "snake_case")]
pub enum ProtocolConfig {
    Modbus(ModbusConfig),
    Ethercat(EthercatConfig),
}

impl ProtocolConfig {
    pub fn protocol(&self) -> Protocol {
        match self {
            ProtocolConfig::Modbus(_) => Protocol::Modbus,
            ProtocolConfig::Ethercat(_) => Protocol::Ethercat,
        }
    }

    pub fn channel_names(&self) -> Vec<String> {
        match self {
            ProtocolConfig::Modbus(c) => c.channels.iter().map(|c| c.name.clone()).collect(),
            ProtocolConfig::Ethercat(c) => c.channels.iter().map(|c| c.name.clone()).collect(),
        }
    }
}

/// Modbus device config. The transport is either TCP (host + port) or
/// RTU (a serial port + line settings).
///
/// **Back-compat note** — projects saved before RTU support landed
/// had `host` + `port` as top-level fields rather than wrapped in a
/// `transport` variant. The custom `Deserialize` below accepts both
/// shapes: a config with `transport` deserialises directly, and one
/// with just `host` + `port` is upgraded in-place to a Tcp variant.
/// On the next write the new shape is persisted, so old projects
/// migrate silently on first save.
#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct ModbusConfig {
    pub transport: ModbusTransport,
    pub slave_id: u8,
    /// Polling interval in milliseconds (u32 so it round-trips through JSON
    /// as a number rather than a TS bigint).
    pub poll_interval_ms: u32,
    #[serde(default)]
    pub channels: Vec<ModbusChannel>,
}

// Custom deserialize so projects authored against the old flat
// `{host, port, …}` shape keep loading. `untagged` tries variants in
// order — the new shape wins because it has a `transport` field;
// the old shape matches when `transport` is missing.
impl<'de> Deserialize<'de> for ModbusConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Compat {
            New {
                transport: ModbusTransport,
                slave_id: u8,
                #[serde(default = "default_poll_interval_ms")]
                poll_interval_ms: u32,
                #[serde(default)]
                channels: Vec<ModbusChannel>,
            },
            // Legacy: TCP fields at the top level, no `transport`.
            // Auto-upgrades to ModbusTransport::Tcp on read.
            Old {
                host: String,
                port: u16,
                slave_id: u8,
                #[serde(default = "default_poll_interval_ms")]
                poll_interval_ms: u32,
                #[serde(default)]
                channels: Vec<ModbusChannel>,
            },
        }
        let v = Compat::deserialize(deserializer)?;
        Ok(match v {
            Compat::New {
                transport,
                slave_id,
                poll_interval_ms,
                channels,
            } => Self {
                transport,
                slave_id,
                poll_interval_ms,
                channels,
            },
            Compat::Old {
                host,
                port,
                slave_id,
                poll_interval_ms,
                channels,
            } => Self {
                transport: ModbusTransport::Tcp(ModbusTcpParams { host, port }),
                slave_id,
                poll_interval_ms,
                channels,
            },
        })
    }
}

fn default_poll_interval_ms() -> u32 {
    100
}

/// Either Modbus TCP (open a socket) or Modbus RTU (open a serial
/// port). The wire payloads are byte-identical; only the transport
/// differs. JSON tag is `"kind"`.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ModbusTransport {
    Tcp(ModbusTcpParams),
    Rtu(ModbusRtuParams),
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ModbusTcpParams {
    pub host: String,
    pub port: u16,
}

/// Modbus RTU serial-line parameters. The defaults match the most
/// common configuration in the wild (9600-8-N-1) so a minimal
/// `{"kind":"rtu","serial_device":"/dev/ttyUSB0","baud_rate":9600}`
/// JSON is valid input.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ModbusRtuParams {
    /// OS device path. macOS: `/dev/cu.usbserial-*` (or
    /// `/dev/tty.usbserial-*` for synchronous open). Linux:
    /// `/dev/ttyUSB0` / `/dev/ttyS0`. Windows: `COM3`.
    pub serial_device: String,
    /// Bits per second. Modbus RTU is typically one of
    /// 1200 / 2400 / 4800 / 9600 / 19200 / 38400 / 57600 / 115200.
    pub baud_rate: u32,
    /// 5 / 6 / 7 / 8 data bits. Modbus RTU is 8 in practice; we
    /// expose the choice so non-standard slaves still work.
    #[serde(default)]
    pub data_bits: ModbusDataBits,
    /// 1 / 2 stop bits. Spec: 2 when parity is None, 1 when parity
    /// is Even/Odd. We let the user set it explicitly — vendor
    /// variations are common.
    #[serde(default)]
    pub stop_bits: ModbusStopBits,
    /// None / Even / Odd. Even is the spec default; None is what
    /// Modicon / many Chinese inverters actually ship.
    #[serde(default)]
    pub parity: ModbusParity,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, TS, PartialEq, Eq, Default)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum ModbusParity {
    #[default]
    None,
    Even,
    Odd,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, TS, PartialEq, Eq, Default)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum ModbusDataBits {
    Five,
    Six,
    Seven,
    #[default]
    Eight,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, TS, PartialEq, Eq, Default)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum ModbusStopBits {
    #[default]
    One,
    Two,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ModbusChannel {
    pub name: String,
    pub kind: ModbusChannelKind,
    pub address: u16,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum ModbusChannelKind {
    Coil,
    DiscreteInput,
    HoldingRegister,
    InputRegister,
}

#[cfg(test)]
mod modbus_config_compat_tests {
    use super::*;

    /// Old-shape JSON without `transport` upgrades to a Tcp variant.
    /// Same shape the IDE / on-disk project files have been writing
    /// since v0.
    #[test]
    fn deserializes_legacy_flat_shape_as_tcp() {
        let json = serde_json::json!({
            "host": "192.168.1.50",
            "port": 502,
            "slave_id": 1,
            "poll_interval_ms": 100,
            "channels": [],
        });
        let cfg: ModbusConfig = serde_json::from_value(json).unwrap();
        match cfg.transport {
            ModbusTransport::Tcp(p) => {
                assert_eq!(p.host, "192.168.1.50");
                assert_eq!(p.port, 502);
            }
            other => panic!("expected Tcp, got {other:?}"),
        }
    }

    /// New-shape JSON with `transport.kind = "tcp"` round-trips.
    #[test]
    fn roundtrips_new_tcp_shape() {
        let json = serde_json::json!({
            "transport": { "kind": "tcp", "host": "127.0.0.1", "port": 5502 },
            "slave_id": 7,
            "poll_interval_ms": 50,
            "channels": [],
        });
        let cfg: ModbusConfig = serde_json::from_value(json.clone()).unwrap();
        let back = serde_json::to_value(&cfg).unwrap();
        assert_eq!(back["transport"]["kind"], "tcp");
        assert_eq!(back["slave_id"], 7);
    }

    /// RTU shape with minimal fields uses sensible defaults
    /// (8-N-1 — the universal Modbus RTU default).
    #[test]
    fn parses_rtu_with_defaults_filled_in() {
        let json = serde_json::json!({
            "transport": {
                "kind": "rtu",
                "serial_device": "/dev/ttyUSB0",
                "baud_rate": 19200,
            },
            "slave_id": 2,
            "poll_interval_ms": 100,
            "channels": [],
        });
        let cfg: ModbusConfig = serde_json::from_value(json).unwrap();
        match cfg.transport {
            ModbusTransport::Rtu(p) => {
                assert_eq!(p.serial_device, "/dev/ttyUSB0");
                assert_eq!(p.baud_rate, 19200);
                assert_eq!(p.data_bits, ModbusDataBits::Eight);
                assert_eq!(p.parity, ModbusParity::None);
                assert_eq!(p.stop_bits, ModbusStopBits::One);
            }
            other => panic!("expected Rtu, got {other:?}"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct EthercatConfig {
    /// Network interface bound to the MainDevice (e.g. "eth0", "en0").
    /// Persisted so the IDE can preserve the user's intent even on hosts
    /// where the NIC isn't currently up.
    pub nic: String,
    /// DC SYNC0 cycle time in microseconds. Defaults to 1 ms (1 kHz).
    #[serde(default = "default_cycle_us")]
    pub cycle_us: u32,
    /// Bus topology — describes the SubDevices the MainDevice expects to
    /// find on the ring. Order matters: the `index` here is the auto-
    /// incremented 0-based position on the bus, matching how ethercrab
    /// numbers slaves after the discovery walk.
    #[serde(default)]
    pub slaves: Vec<EthercatSlave>,
    /// PDO channels exposed to IO mapping. Kept flat (rather than nested
    /// under each slave) so the iomap layer treats them identically to
    /// Modbus channels — referenced by a unique name string, resolved
    /// via this list.
    #[serde(default)]
    pub channels: Vec<EthercatChannel>,
}

fn default_cycle_us() -> u32 {
    1_000
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct EthercatSlave {
    /// 0-based position on the ring. Mirrors the order ethercrab discovers
    /// SubDevices in; written here so configuration is preserved even if
    /// the bus isn't currently up.
    pub index: u16,
    /// Friendly name; surfaced in the device editor and helpful in logs.
    pub name: String,
    /// 32-bit vendor ID from ESI/SII (e.g. 0x00000002 for Beckhoff). 0
    /// when unknown — the runtime treats 0 as "skip identity check".
    #[serde(default)]
    pub vendor_id: u32,
    /// 32-bit product code from ESI/SII. 0 when unknown.
    #[serde(default)]
    pub product_id: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct EthercatChannel {
    /// Unique channel name — what iomap entries reference.
    pub name: String,
    /// Which SubDevice on the bus this PDO entry lives on (matches
    /// `EthercatSlave.index`).
    pub slave_index: u16,
    pub direction: EthercatPdoDirection,
    /// CoE object dictionary index (e.g. 0x6000 for the first TxPDO entry
    /// on a typical digital input slave). Stored as a plain u16 — the UI
    /// renders it as hex. Informational in real mode (the cyclic exchange
    /// uses `pdi_byte_offset` + `pdi_bit_offset` for fast lookup), but
    /// kept on the channel so users can document the source PDO entry.
    pub pdo_index: u16,
    /// Sub-index inside the PDO object.
    pub sub_index: u8,
    /// Bit length of the PDO entry. Usually 1, 8, 16, or 32.
    pub bit_length: u8,
    pub data_type: EthercatDataType,
    /// Byte offset of this PDO entry inside the SubDevice's input (TxPDO)
    /// or output (RxPDO) PDI region. Required in real mode — the cyclic
    /// task pulls bytes from `[pdi_byte_offset .. pdi_byte_offset +
    /// ceil(bit_length / 8)]` of the SubDevice's PDI buffer. Defaults to
    /// 0 for back-compat with existing sim-only configs (those ignore it).
    #[serde(default)]
    pub pdi_byte_offset: u16,
    /// Bit offset *within* the byte at `pdi_byte_offset`. 0 is the LSB.
    /// Only meaningful for `bit_length < 8` channels (e.g. digital I/O
    /// where 8 channels share one byte). Defaults to 0.
    #[serde(default)]
    pub pdi_bit_offset: u8,
}

/// PDO direction from the MainDevice's perspective.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum EthercatPdoDirection {
    /// TxPDO — SubDevice → MainDevice (controller reads this each cycle).
    TxPdo,
    /// RxPDO — MainDevice → SubDevice (controller writes this each cycle).
    RxPdo,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum EthercatDataType {
    Bool,
    U8,
    I8,
    U16,
    I16,
    U32,
    I32,
    Real,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Device {
    pub name: String,
    #[serde(flatten)]
    pub config: ProtocolConfig,
}

// ---------------- IO Mapping ----------------

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    Input,  // bus → variable
    Output, // variable → bus
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Mapping {
    pub application: String,
    pub variable: String,
    pub direction: Direction,
    pub device: String,
    pub channel: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct IoMap {
    #[serde(default)]
    pub mappings: Vec<Mapping>,
}

// ---------------- Edges (deploy targets) ----------------

/// A Linux box where the compiled program is meant to actually run. Sister
/// concept to `Device`: Device = thing the program talks to over a bus,
/// Edge = thing the program runs on.
///
/// On purpose, we do **not** store SSH credentials in this struct. The IDE
/// runs `ssh <host>` and lets the OS resolve the host via `~/.ssh/config`
/// (keys, agent, jump hosts, all of that). The user lists hosts here only
/// so the project records *which* edges this project deploys to.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Edge {
    pub name: String,
    /// SSH hostname or `~/.ssh/config` alias (preferred).
    pub host: String,
    /// SSH port. Defaults to 22; here so the user can override per-edge
    /// without polluting their global SSH config.
    #[serde(default = "default_ssh_port")]
    pub ssh_port: u16,
    /// SSH user. Empty string means "let ssh decide" — typically resolved
    /// via `~/.ssh/config` or the current user name.
    #[serde(default)]
    pub ssh_user: String,
    /// Absolute path on the edge box where the runtime + project versions
    /// live. Convention: `/opt/ia2`. The deploy script lays out
    /// `versions/<timestamp>/` and atomically swaps `current -> …`.
    #[serde(default = "default_install_dir")]
    pub install_dir: String,
    /// Local TCP port the runtime's monitor server binds to on the edge.
    /// Always `127.0.0.1:<port>` — remote access is via SSH port-forward,
    /// never direct exposure.
    #[serde(default = "default_runtime_port")]
    pub runtime_port: u16,
    /// Free-form notes — "production line 1", "test bench", whatever.
    #[serde(default)]
    pub notes: String,
}

fn default_ssh_port() -> u16 {
    22
}

fn default_install_dir() -> String {
    "/opt/ia2".to_string()
}

fn default_runtime_port() -> u16 {
    13001
}

// ---------------- Tasks (project-level scheduling) ----------------

/// One scheduling task — periodic only in V1. Maps directly to IEC's
/// `TASK <name>(INTERVAL := T#<ms>ms, PRIORITY := <priority>);`.
///
/// Event-triggered (`SINGLE :=`) tasks are deferred to a later iteration.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Task {
    pub name: String,
    /// Period in milliseconds; emitted as `T#<ms>ms` in the synthesized
    /// CONFIGURATION block.
    pub interval_ms: u32,
    /// IEC priority — lower numbers are higher priority on most runtimes.
    pub priority: i32,
}

/// One `PROGRAM <instance> WITH <task> : <program_type>;` binding. Only
/// PROGRAM-kind POUs can be instantiated here (FBs and FUNCTIONs are
/// used from inside PROGRAMs); the UI enforces this on the dropdown.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ProgramInstance {
    /// Instance name, unique within the configuration.
    pub instance: String,
    /// PROGRAM-kind POU this instance is of (matches a name in
    /// `applications/`).
    pub program: String,
    /// Task name this instance is scheduled on.
    pub task: String,
}

/// Project-level scheduling — replaces the per-POU inline CONFIGURATION
/// blocks. Lives in `tasks.toml`. The runtime synthesizes the IEC
/// CONFIGURATION block from this at compile time.
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Tasks {
    #[serde(default)]
    pub tasks: Vec<Task>,
    #[serde(default)]
    pub programs: Vec<ProgramInstance>,
}

// ---------------- Project tree (frontend response) ----------------

#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct ProjectTree {
    pub name: String,
    pub path: String,
    /// Every `.st` file under `pous/` with its parsed declarations.
    /// The tree renders each declaration as a node; multi-POU files
    /// (FB + PROGRAM in one .st) show up as sibling nodes that share
    /// a `path`.
    pub pous: Vec<PouFile>,
    /// All folder paths under `pous/` (slash-separated, relative).
    /// Includes empty folders.
    pub pou_folders: Vec<String>,
    /// Full Device records (config inline) so the IO Mapping UI can
    /// resolve channel kind/address without per-device fetches.
    pub devices: Vec<Device>,
    /// Same as `app_folders` but rooted at `devices/`.
    pub device_folders: Vec<String>,
    pub iomap: IoMap,
    /// Deploy targets — Linux boxes where the program is meant to run.
    pub edges: Vec<Edge>,
    /// Folders under `edges/`, including empty ones.
    pub edge_folders: Vec<String>,
    /// Project-level scheduling. May be empty for a fresh project; the
    /// migration step populates it from inline CONFIGURATION blocks the
    /// first time an old project is opened.
    pub tasks: Tasks,
}

// ---------------- Project list entry ----------------

#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct ProjectListing {
    pub name: String,
    pub path: String,
    pub is_last_opened: bool,
}
