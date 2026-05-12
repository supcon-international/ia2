//! EtherCAT MainDevice IoDevice adapter.
//!
//! Real ethercrab mastery needs a raw L2 socket (CAP_NET_RAW on Linux,
//! pf_packet, or BPF on macOS), DC sync, PreOp → SafeOp → Op walk, and
//! a PDO map derived from SII/ESI per SubDevice. None of that works
//! portably on macOS where this codebase is developed, and there's no
//! hardware to talk to in the demo environment.
//!
//! So this crate provides:
//!  - **Config-driven sim mode** (default): an in-memory PDO buffer
//!    keyed by channel name. Output channels echo what the program
//!    writes; input channels start at zero. The adapter validates that
//!    requested channel reads/writes match the configured PDO direction
//!    and data type, surfacing iomap mistakes early.
//!  - **Real ethercrab plumbing slot**: keep the IoDevice trait wired so
//!    a future implementation can land without changing ironplc-bridge.
//!
//! The runtime composes Modbus + EtherCAT devices uniformly today —
//! both connect via `connect`, both expose `read_channel` / `write_channel`.
//! Sim mode here keeps the IO Mapping pane informative end-to-end.

use std::collections::HashMap;

use async_trait::async_trait;
use iocore::{ChannelValue, IoDevice, IoError};
use project::{
    EthercatChannel, EthercatConfig, EthercatDataType, EthercatPdoDirection,
};

/// In-memory PDO buffer with per-channel metadata. Lives entirely in
/// process; intended for IDE round-trip verification, not real I/O.
pub struct EthercatDevice {
    name: String,
    /// Last known value per channel, indexed by name.
    values: HashMap<String, ChannelValue>,
    /// Configured PDO metadata indexed by channel name. Used to enforce
    /// direction (no writing to TxPDOs) and to coerce write values to
    /// the declared data type.
    channels: HashMap<String, EthercatChannel>,
}

impl EthercatDevice {
    pub async fn connect(name: String, config: &EthercatConfig) -> Result<Self, IoError> {
        // Validation: each channel must point at a known slave index, and
        // no two channels may share a name. Catch this at connect-time so
        // the bridge logs a useful error instead of silently mis-routing.
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

        tracing::info!(
            name = %name,
            nic = %config.nic,
            cycle_us = config.cycle_us,
            slaves = config.slaves.len(),
            channels = config.channels.len(),
            "ethercat device (sim mode) ready"
        );
        Ok(Self {
            name,
            values,
            channels,
        })
    }

    fn channel(&self, name: &str) -> Result<&EthercatChannel, IoError> {
        self.channels
            .get(name)
            .ok_or_else(|| IoError::UnknownChannel(name.into()))
    }
}

#[async_trait]
impl IoDevice for EthercatDevice {
    fn name(&self) -> &str {
        &self.name
    }

    async fn read_channel(&mut self, channel: &str) -> Result<ChannelValue, IoError> {
        let _meta = self.channel(channel)?;
        // Both TxPDO (slave → master) and RxPDO (master → slave) reads are
        // supported in sim mode — the buffer just holds whatever was last
        // written, defaulting to zero. Real hardware would gate this by
        // direction; left permissive here so the IDE can echo back outputs.
        Ok(self
            .values
            .get(channel)
            .copied()
            .unwrap_or(ChannelValue::I32(0)))
    }

    async fn write_channel(
        &mut self,
        channel: &str,
        value: ChannelValue,
    ) -> Result<(), IoError> {
        let meta = self.channel(channel)?.clone();
        if meta.direction == EthercatPdoDirection::TxPdo {
            // Writing to a TxPDO would be a configuration mistake — the
            // slave produces TxPDOs, not the master. Surface it instead
            // of silently dropping the write.
            return Err(IoError::TypeMismatch {
                channel: channel.into(),
                value,
            });
        }
        let coerced = coerce_to_type(value, meta.data_type);
        self.values.insert(channel.into(), coerced);
        Ok(())
    }
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
