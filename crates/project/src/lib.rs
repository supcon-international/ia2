//! Disk-backed project store.
//!
//! A project lives in a single directory:
//!
//! ```text
//! my_project/
//! ├── project.toml          # manifest
//! ├── applications/         # one .st file per POU
//! │   └── main.st
//! ├── devices/              # one .toml per device (protocol inside)
//! │   └── tank.toml
//! └── iomap.toml             # variable ↔ channel bindings
//! ```

mod errors;
mod fbd;
mod ld;
mod paths;
mod sfc;
mod store;
mod types;

pub use errors::StoreError;
pub use fbd::{
    FbdBlock, FbdInputBinding, FbdInputSource, FbdOutputBinding, FbdPosition, FbdProgram,
};
pub use ld::{
    LdCoil, LdCoilKind, LdComparator, LdFbInput, LdNode, LdOperand, LdPouType, LdProgram, LdRung,
    LdVarSection, LdVariable,
};
pub use paths::{
    default_projects_dir, default_state_path, load_last_opened, migrate_legacy_dirs,
    save_last_opened,
};
pub use sfc::{SfcAction, SfcProgram, SfcQualifier, SfcStep, SfcTransition};
pub use store::{MigrationReport, ProjectStore};
pub use types::{
    Device, Direction, Edge, EthercatChannel, EthercatConfig, EthercatDataType,
    EthercatPdoDirection, EthercatSlave, IoMap, Mapping, ModbusChannel, ModbusChannelKind,
    ModbusConfig, ModbusDataBits, ModbusParity, ModbusRtuParams, ModbusStopBits, ModbusTcpParams,
    ModbusTransport, Pou, PouDecl, PouFile, PouFileSource, PouLanguage, PouType, ProgramInstance,
    ProjectListing, ProjectManifest, ProjectTree, ProjectTreeSkeleton, Protocol, ProtocolConfig,
    Task, Tasks,
};
