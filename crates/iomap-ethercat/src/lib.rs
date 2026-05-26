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

/// One subdevice as seen on the bus (real mode, walked at connect) or as
/// configured (sim mode). Plain data — the bridge maps this into its own
/// serializable shape for the `/discover` wire format, so this crate
/// stays free of serde.
#[derive(Debug, Clone)]
pub struct SlaveDiscovery {
    /// Auto-increment bus position assigned by the master.
    pub index: u16,
    /// Subdevice product name (from its EEPROM / ESI identity).
    pub name: String,
    /// Bytes of input (TxPDO, slave→master) process data.
    pub input_bytes: u16,
    /// Bytes of output (RxPDO, master→slave) process data.
    pub output_bytes: u16,
    pub vendor_id: u32,
    pub product_id: u32,
}

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

    /// Subdevices discovered on the bus (real mode) or configured (sim
    /// mode). Used by the runtime's `/discover` endpoint so the IDE can
    /// author PDO maps against the real topology.
    pub fn discovered(&self) -> Vec<SlaveDiscovery> {
        match &self.0 {
            Inner::Sim(s) => s.discovered(),
            Inner::Real(r) => r.discovered(),
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

    async fn enter_failsafe(&mut self) -> Result<(), IoError> {
        match &mut self.0 {
            Inner::Sim(s) => s.enter_failsafe().await,
            Inner::Real(r) => r.enter_failsafe().await,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use project::{EthercatChannel, EthercatDataType, EthercatPdoDirection, EthercatSlave};

    #[test]
    fn is_sim_nic_matches_sentinel_and_empty() {
        assert!(is_sim_nic("_sim"));
        assert!(is_sim_nic(""));
        assert!(!is_sim_nic("eth0"));
        assert!(!is_sim_nic("en0"));
        assert!(!is_sim_nic("_sim_extra"));
    }

    fn sim_config_with_two_outputs_and_one_input() -> EthercatConfig {
        EthercatConfig {
            nic: SIM_NIC.into(),
            cycle_us: 1_000,
            dc_sync: project::EthercatDcSync::Off,
            slaves: vec![EthercatSlave {
                index: 0,
                name: "EL2008".into(),
                vendor_id: 0,
                product_id: 0,
            }],
            channels: vec![
                EthercatChannel {
                    name: "out_motor".into(),
                    slave_index: 0,
                    direction: EthercatPdoDirection::RxPdo,
                    pdo_index: 0x7000,
                    sub_index: 1,
                    bit_length: 1,
                    data_type: EthercatDataType::Bool,
                    pdi_byte_offset: 0,
                    pdi_bit_offset: 0,
                },
                EthercatChannel {
                    name: "out_speed".into(),
                    slave_index: 0,
                    direction: EthercatPdoDirection::RxPdo,
                    pdo_index: 0x7000,
                    sub_index: 2,
                    bit_length: 16,
                    data_type: EthercatDataType::U16,
                    pdi_byte_offset: 2,
                    pdi_bit_offset: 0,
                },
                EthercatChannel {
                    name: "in_estop".into(),
                    slave_index: 0,
                    direction: EthercatPdoDirection::TxPdo,
                    pdo_index: 0x6000,
                    sub_index: 1,
                    bit_length: 1,
                    data_type: EthercatDataType::Bool,
                    pdi_byte_offset: 0,
                    pdi_bit_offset: 0,
                },
            ],
        }
    }

    #[tokio::test]
    async fn enter_failsafe_zeroes_rxpdo_outputs_in_sim_mode() {
        let cfg = sim_config_with_two_outputs_and_one_input();
        let mut dev = EthercatDevice::connect("test".into(), &cfg).await.unwrap();

        // Drive outputs to non-zero values.
        dev.write_channel("out_motor", ChannelValue::Bool(true))
            .await
            .unwrap();
        dev.write_channel("out_speed", ChannelValue::U16(42))
            .await
            .unwrap();

        assert_eq!(
            dev.read_channel("out_motor").await.unwrap().to_i32(),
            1,
            "precondition: motor write took effect"
        );
        assert_eq!(
            dev.read_channel("out_speed").await.unwrap().to_i32(),
            42,
            "precondition: speed write took effect"
        );

        // Trip failsafe.
        dev.enter_failsafe().await.unwrap();

        assert_eq!(
            dev.read_channel("out_motor").await.unwrap().to_i32(),
            0,
            "motor must be zeroed after failsafe"
        );
        assert_eq!(
            dev.read_channel("out_speed").await.unwrap().to_i32(),
            0,
            "speed must be zeroed after failsafe"
        );

        // TxPDO (input) channels remain — failsafe only touches outputs.
        // The default sim value for the input is also 0, so we just
        // confirm the read still succeeds (i.e. the channel still exists
        // and wasn't accidentally deleted).
        let _ = dev.read_channel("in_estop").await.unwrap();
    }
}
