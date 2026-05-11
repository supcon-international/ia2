//! EtherCAT MainDevice IoDevice adapter (scaffolding only).
//!
//! Real EtherCAT mastery via `ethercrab` requires a raw socket on a NIC
//! (CAP_NET_RAW / root on Linux), distributed-clock synchronisation, and
//! PDO mapping per connected slave. None of that works portably on
//! macOS where this is being developed, so for now `connect` returns a
//! polite error and the bridge logs a clear skip. The plumbing — the
//! IoDevice impl, dependency wiring, scan-loop integration — is in
//! place so a real implementation can be dropped in without touching
//! ironplc-bridge.

use async_trait::async_trait;
use iocore::{ChannelValue, IoDevice, IoError};
use project::EthercatConfig;

pub struct EthercatDevice {
    name: String,
}

impl EthercatDevice {
    pub async fn connect(name: String, config: &EthercatConfig) -> Result<Self, IoError> {
        // TODO: open a raw socket on config.nic, scan slaves, configure PDOs,
        // and walk PreOp → SafeOp → Op states. ethercrab's MainDevice handles
        // most of the protocol detail; this stub keeps the rest of the
        // pipeline wired so the runtime can compose Modbus + (future)
        // EtherCAT devices uniformly.
        let _ = name;
        Err(IoError::Connect(format!(
            "EtherCAT adapter not implemented yet (nic={}). Requires raw \
             socket privileges + connected EtherCAT slaves.",
            config.nic
        )))
    }
}

#[async_trait]
impl IoDevice for EthercatDevice {
    fn name(&self) -> &str {
        &self.name
    }

    async fn read_channel(&mut self, channel: &str) -> Result<ChannelValue, IoError> {
        Err(IoError::UnknownChannel(channel.into()))
    }

    async fn write_channel(
        &mut self,
        channel: &str,
        _value: ChannelValue,
    ) -> Result<(), IoError> {
        Err(IoError::UnknownChannel(channel.into()))
    }
}
