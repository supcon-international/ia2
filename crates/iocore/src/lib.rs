//! The trait an external fieldbus adapter (Modbus, EtherCAT, …) implements
//! so the ironplc VM scan loop can read inputs before `run_round` and write
//! outputs after.
//!
//! Lives in its own crate so concrete adapters (`iomap-modbus`,
//! `iomap-ethercat`) and the ironplc-bridge runtime can all depend on it
//! without forming a cycle.

use async_trait::async_trait;
use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub enum ChannelValue {
    Bool(bool),
    U16(u16),
    I32(i32),
}

impl ChannelValue {
    /// Coerce to the i32 the ironplc VM accepts via `write_variable`.
    pub fn to_i32(self) -> i32 {
        match self {
            Self::Bool(b) => b as i32,
            Self::U16(v) => v as i32,
            Self::I32(v) => v,
        }
    }
}

#[derive(Debug, Error)]
pub enum IoError {
    #[error("unknown channel '{0}'")]
    UnknownChannel(String),
    #[error("type mismatch: channel '{channel}' cannot accept {value:?}")]
    TypeMismatch {
        channel: String,
        value: ChannelValue,
    },
    #[error("connect: {0}")]
    Connect(String),
    #[error("transport: {0}")]
    Transport(String),
}

/// A fieldbus device — read/write a logical channel by name. Implementations
/// own their connection / runtime / cache.
#[async_trait]
pub trait IoDevice: Send {
    fn name(&self) -> &str;
    async fn read_channel(&mut self, channel: &str) -> Result<ChannelValue, IoError>;
    async fn write_channel(
        &mut self,
        channel: &str,
        value: ChannelValue,
    ) -> Result<(), IoError>;
}
