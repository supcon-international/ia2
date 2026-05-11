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
mod paths;
mod store;
mod types;

pub use errors::StoreError;
pub use paths::{
    default_projects_dir, default_state_path, load_last_opened, save_last_opened,
};
pub use store::ProjectStore;
pub use types::{
    Application, ApplicationKind, ApplicationSummary, Device, DeviceSummary, Direction,
    EthercatConfig, IoMap, Mapping, ModbusChannel, ModbusChannelKind, ModbusConfig,
    ProjectListing, ProjectManifest, ProjectTree, Protocol, ProtocolConfig,
};
