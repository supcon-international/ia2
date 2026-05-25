//! In-memory PDO buffer ("sim mode"). Selected when `EthercatConfig.nic`
//! equals `"_sim"`. Used for:
//!  - macOS development (raw L2 sockets need root + BPF; not portable)
//!  - Demo / IDE round-trip verification without hardware
//!  - CI tests of the iomap layer
//!
//! Output channels echo what the program writes; input channels start
//! at zero. Direction and data type are still validated so iomap mistakes
//! surface early, just like real mode.

use std::collections::HashMap;

use async_trait::async_trait;
use iocore::{ChannelValue, IoDevice, IoError};
use project::{EthercatChannel, EthercatConfig, EthercatDataType, EthercatPdoDirection};

pub struct SimEthercat {
    name: String,
    values: HashMap<String, ChannelValue>,
    channels: HashMap<String, EthercatChannel>,
    discovered: Vec<crate::SlaveDiscovery>,
}

impl SimEthercat {
    pub async fn connect(name: String, config: &EthercatConfig) -> Result<Self, IoError> {
        // Validation: each channel must point at a known slave index, and
        // no two channels may share a name.
        let known_slaves: std::collections::HashSet<u16> =
            config.slaves.iter().map(|s| s.index).collect();
        let mut seen_names: std::collections::HashSet<&str> = std::collections::HashSet::new();
        for ch in &config.channels {
            if !seen_names.insert(ch.name.as_str()) {
                return Err(IoError::Connect(format!(
                    "duplicate channel name '{}'",
                    ch.name
                )));
            }
            if !known_slaves.is_empty() && !known_slaves.contains(&ch.slave_index) {
                return Err(IoError::Connect(format!(
                    "channel '{}' references unknown slave_index={}",
                    ch.name, ch.slave_index
                )));
            }
        }

        let channels: HashMap<String, EthercatChannel> = config
            .channels
            .iter()
            .map(|c| (c.name.clone(), c.clone()))
            .collect();
        let values: HashMap<String, ChannelValue> = config
            .channels
            .iter()
            .map(|c| (c.name.clone(), zero_for(c.data_type)))
            .collect();

        // Sim "discovery": report the configured slaves with PDI extents
        // derived from their channels, so /discover works without hardware
        // (dev / CI / authoring against a sim bus).
        let discovered: Vec<crate::SlaveDiscovery> = config
            .slaves
            .iter()
            .map(|s| {
                let extent = |dir: EthercatPdoDirection| -> u16 {
                    config
                        .channels
                        .iter()
                        .filter(|c| c.slave_index == s.index && c.direction == dir)
                        .map(|c| c.pdi_byte_offset + byte_size(c.bit_length))
                        .max()
                        .unwrap_or(0)
                };
                crate::SlaveDiscovery {
                    index: s.index,
                    name: s.name.clone(),
                    input_bytes: extent(EthercatPdoDirection::TxPdo),
                    output_bytes: extent(EthercatPdoDirection::RxPdo),
                    vendor_id: s.vendor_id,
                    product_id: s.product_id,
                }
            })
            .collect();

        tracing::info!(
            name = %name,
            slaves = config.slaves.len(),
            channels = config.channels.len(),
            "ethercat device ready (sim mode)"
        );
        Ok(Self {
            name,
            values,
            channels,
            discovered,
        })
    }

    fn channel(&self, name: &str) -> Result<&EthercatChannel, IoError> {
        self.channels
            .get(name)
            .ok_or_else(|| IoError::UnknownChannel(name.into()))
    }

    /// Configured slaves as a discovery report (sim has no real bus).
    pub fn discovered(&self) -> Vec<crate::SlaveDiscovery> {
        self.discovered.clone()
    }
}

#[async_trait]
impl IoDevice for SimEthercat {
    fn name(&self) -> &str {
        &self.name
    }

    async fn read_channel(&mut self, channel: &str) -> Result<ChannelValue, IoError> {
        let _meta = self.channel(channel)?;
        Ok(self
            .values
            .get(channel)
            .copied()
            .unwrap_or(ChannelValue::I32(0)))
    }

    async fn write_channel(&mut self, channel: &str, value: ChannelValue) -> Result<(), IoError> {
        let meta = self.channel(channel)?.clone();
        if meta.direction == EthercatPdoDirection::TxPdo {
            return Err(IoError::TypeMismatch {
                channel: channel.into(),
                value,
            });
        }
        let coerced = coerce_to_type(value, meta.data_type);
        self.values.insert(channel.into(), coerced);
        Ok(())
    }

    /// Zero every RxPDO (output) channel. TxPDO entries are inputs from
    /// the bus and stay untouched. Matches `RealEthercat::enter_failsafe`
    /// semantics so sim and real behave identically from the scan
    /// loop's perspective.
    async fn enter_failsafe(&mut self) -> Result<(), IoError> {
        let to_zero: Vec<(String, EthercatDataType)> = self
            .channels
            .values()
            .filter(|c| c.direction == EthercatPdoDirection::RxPdo)
            .map(|c| (c.name.clone(), c.data_type))
            .collect();
        for (name, ty) in to_zero {
            self.values.insert(name, zero_for(ty));
        }
        tracing::info!(device = %self.name, "ethercat (sim) failsafe applied");
        Ok(())
    }
}

/// Bytes occupied by a PDO entry of `bit_length` bits (rounded up; a
/// sub-byte entry still occupies one byte).
fn byte_size(bit_length: u8) -> u16 {
    (bit_length as u16).div_ceil(8)
}

fn zero_for(ty: EthercatDataType) -> ChannelValue {
    match ty {
        EthercatDataType::Bool => ChannelValue::Bool(false),
        EthercatDataType::U8
        | EthercatDataType::I8
        | EthercatDataType::U16
        | EthercatDataType::I16 => ChannelValue::U16(0),
        EthercatDataType::U32 | EthercatDataType::I32 | EthercatDataType::Real => {
            ChannelValue::I32(0)
        }
    }
}

fn coerce_to_type(value: ChannelValue, ty: EthercatDataType) -> ChannelValue {
    let raw = value.to_i32();
    match ty {
        EthercatDataType::Bool => ChannelValue::Bool(raw != 0),
        EthercatDataType::U8
        | EthercatDataType::I8
        | EthercatDataType::U16
        | EthercatDataType::I16 => ChannelValue::U16(raw as u16),
        EthercatDataType::U32 | EthercatDataType::I32 | EthercatDataType::Real => {
            ChannelValue::I32(raw)
        }
    }
}
