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
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ModbusConfig {
    pub host: String,
    pub port: u16,
    pub slave_id: u8,
    pub poll_interval_ms: u64,
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
    /// Network interface (e.g. "eth0"). Placeholder until we wire ethercrab.
    pub nic: String,
    #[serde(default)]
    pub slaves: Vec<EthercatSlave>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct EthercatSlave {
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Device {
    pub name: String,
    #[serde(flatten)]
    pub config: ProtocolConfig,
}

#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct DeviceSummary {
    pub name: String,
    pub protocol: Protocol,
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

// ---------------- Project tree (frontend response) ----------------

#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct ProjectTree {
    pub name: String,
    pub path: String,
    pub applications: Vec<ApplicationSummary>,
    pub devices: Vec<DeviceSummary>,
    pub iomap: IoMap,
}

// ---------------- Project list entry ----------------

#[derive(Debug, Clone, Serialize, TS)]
#[ts(export)]
pub struct ProjectListing {
    pub name: String,
    pub path: String,
    pub is_last_opened: bool,
}
