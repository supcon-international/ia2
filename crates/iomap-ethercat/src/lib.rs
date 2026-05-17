//! EtherCAT MainDevice IoDevice adapter.
//!
//! Two operating modes, picked by `EthercatConfig.nic`:
//!
//! - `"_sim"` — in-memory PDO buffer, no hardware. Used for macOS
//!   development (raw L2 sockets aren't portable), IDE round-trip
//!   verification, and CI tests. Output channels echo what the program
//!   writes; input channels start at zero.
//!
//! - Anything else (e.g. `"eth0"`, `"eth1"`) — real `ethercrab::MainDevice`
//!   on the named NIC. Walks the bus on connect, transitions to OP,
//!   and drives a cyclic PDO exchange on its own thread (ethercrab uses
//!   `async-io`, not tokio). Requires Linux + `CAP_NET_RAW`.
//!
//! Both modes implement the same `IoDevice` trait so the runtime composes
//! them identically with Modbus devices.

mod bits;
mod real;
mod sim;

use async_trait::async_trait;
use iocore::{ChannelValue, IoDevice, IoError};
use project::EthercatConfig;

/// Sentinel NIC name that selects the in-memory sim path. Anything else
/// is treated as a real network interface name.
pub const SIM_NIC: &str = "_sim";

/// Public façade — internally an enum so the bridge / runtime see one
/// type regardless of mode.
pub struct EthercatDevice(Inner);

enum Inner {
    Sim(sim::SimEthercat),
    Real(real::RealEthercat),
}

impl EthercatDevice {
    pub async fn connect(name: String, config: &EthercatConfig) -> Result<Self, IoError> {
        if is_sim_nic(&config.nic) {
            sim::SimEthercat::connect(name, config)
                .await
                .map(Inner::Sim)
                .map(EthercatDevice)
        } else {
            real::RealEthercat::connect(name, config)
                .await
                .map(Inner::Real)
                .map(EthercatDevice)
        }
    }
}

fn is_sim_nic(nic: &str) -> bool {
    nic == SIM_NIC || nic.is_empty()
}

#[async_trait]
impl IoDevice for EthercatDevice {
    fn name(&self) -> &str {
        match &self.0 {
            Inner::Sim(s) => s.name(),
            Inner::Real(r) => r.name(),
        }
    }

    async fn read_channel(&mut self, channel: &str) -> Result<ChannelValue, IoError> {
        match &mut self.0 {
            Inner::Sim(s) => s.read_channel(channel).await,
            Inner::Real(r) => r.read_channel(channel).await,
        }
    }

    async fn write_channel(&mut self, channel: &str, value: ChannelValue) -> Result<(), IoError> {
        match &mut self.0 {
            Inner::Sim(s) => s.write_channel(channel, value).await,
            Inner::Real(r) => r.write_channel(channel, value).await,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_sim_nic_matches_sentinel_and_empty() {
        assert!(is_sim_nic("_sim"));
        assert!(is_sim_nic(""));
        assert!(!is_sim_nic("eth0"));
        assert!(!is_sim_nic("en0"));
        assert!(!is_sim_nic("_sim_extra"));
    }
}
