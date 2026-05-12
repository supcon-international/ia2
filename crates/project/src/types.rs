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

// ---------------- Applications (POUs) ----------------

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum ApplicationKind {
    Program,
    FunctionBlock,
}

#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct Application {
    pub name: String,
    pub kind: ApplicationKind,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct ApplicationSummary {
    pub name: String,
    pub kind: ApplicationKind,
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

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ModbusConfig {
    pub host: String,
    pub port: u16,
    pub slave_id: u8,
    /// Polling interval in milliseconds (u32 so it round-trips through JSON
    /// as a number rather than a TS bigint).
    pub poll_interval_ms: u32,
    #[serde(default)]
    pub channels: Vec<ModbusChannel>,
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
    /// renders it as hex.
    pub pdo_index: u16,
    /// Sub-index inside the PDO object.
    pub sub_index: u8,
    /// Bit length of the PDO entry. Usually 1, 8, 16, or 32.
    pub bit_length: u8,
    pub data_type: EthercatDataType,
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
    /// live. Convention: `/opt/controlsoftware`. The deploy script lays out
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
    "/opt/controlsoftware".to_string()
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
    pub applications: Vec<ApplicationSummary>,
    /// All folder paths under `applications/` (relative, with `/` separator).
    /// Includes empty folders — the UI needs them to render the tree before
    /// any POUs are placed inside. Folders that contain POUs are listed too.
    pub app_folders: Vec<String>,
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
