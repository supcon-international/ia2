use std::collections::HashMap;
use std::net::SocketAddr;
use std::str::FromStr;

use async_trait::async_trait;
use iocore::{ChannelValue, IoDevice, IoError};
use project::{ModbusChannel, ModbusChannelKind, ModbusConfig};
use tokio_modbus::client::{tcp, Context, Reader, Writer};
use tokio_modbus::Slave;

pub struct ModbusDevice {
    name: String,
    client: Context,
    channels: HashMap<String, ModbusChannel>,
}

impl ModbusDevice {
    pub async fn connect(name: String, config: &ModbusConfig) -> Result<Self, IoError> {
        let addr_str = format!("{}:{}", config.host, config.port);
        let socket = SocketAddr::from_str(&addr_str)
            .map_err(|e| IoError::Connect(format!("invalid address {addr_str}: {e}")))?;
        let client = tcp::connect_slave(socket, Slave(config.slave_id))
            .await
            .map_err(|e| IoError::Connect(e.to_string()))?;
        let channels = config
            .channels
            .iter()
            .map(|c| (c.name.clone(), c.clone()))
            .collect();
        Ok(Self {
            name,
            client,
            channels,
        })
    }

    fn channel(&self, name: &str) -> Result<ModbusChannel, IoError> {
        self.channels
            .get(name)
            .cloned()
            .ok_or_else(|| IoError::UnknownChannel(name.into()))
    }
}

fn transport<T>(e: impl std::fmt::Display) -> Result<T, IoError> {
    Err(IoError::Transport(e.to_string()))
}

#[async_trait]
impl IoDevice for ModbusDevice {
    fn name(&self) -> &str {
        &self.name
    }

    async fn read_channel(&mut self, channel: &str) -> Result<ChannelValue, IoError> {
        let ch = self.channel(channel)?;
        match ch.kind {
            ModbusChannelKind::Coil => {
                let res = self.client.read_coils(ch.address, 1).await;
                let bits = match res {
                    Ok(Ok(v)) => v,
                    Ok(Err(e)) => return transport(format!("modbus exception: {e}")),
                    Err(e) => return transport(e),
                };
                Ok(ChannelValue::Bool(bits.first().copied().unwrap_or(false)))
            }
            ModbusChannelKind::DiscreteInput => {
                let res = self.client.read_discrete_inputs(ch.address, 1).await;
                let bits = match res {
                    Ok(Ok(v)) => v,
                    Ok(Err(e)) => return transport(format!("modbus exception: {e}")),
                    Err(e) => return transport(e),
                };
                Ok(ChannelValue::Bool(bits.first().copied().unwrap_or(false)))
            }
            ModbusChannelKind::HoldingRegister => {
                let res = self.client.read_holding_registers(ch.address, 1).await;
                let words = match res {
                    Ok(Ok(v)) => v,
                    Ok(Err(e)) => return transport(format!("modbus exception: {e}")),
                    Err(e) => return transport(e),
                };
                Ok(ChannelValue::U16(words.first().copied().unwrap_or(0)))
            }
            ModbusChannelKind::InputRegister => {
                let res = self.client.read_input_registers(ch.address, 1).await;
                let words = match res {
                    Ok(Ok(v)) => v,
                    Ok(Err(e)) => return transport(format!("modbus exception: {e}")),
                    Err(e) => return transport(e),
                };
                Ok(ChannelValue::U16(words.first().copied().unwrap_or(0)))
            }
        }
    }

    async fn write_channel(&mut self, channel: &str, value: ChannelValue) -> Result<(), IoError> {
        let ch = self.channel(channel)?;
        match ch.kind {
            ModbusChannelKind::Coil => {
                let b = value.to_i32() != 0;
                match self.client.write_single_coil(ch.address, b).await {
                    Ok(Ok(())) => Ok(()),
                    Ok(Err(e)) => Err(IoError::Transport(format!("modbus exception: {e}"))),
                    Err(e) => Err(IoError::Transport(e.to_string())),
                }
            }
            ModbusChannelKind::HoldingRegister => {
                let word = value.to_i32() as u16;
                match self.client.write_single_register(ch.address, word).await {
                    Ok(Ok(())) => Ok(()),
                    Ok(Err(e)) => Err(IoError::Transport(format!("modbus exception: {e}"))),
                    Err(e) => Err(IoError::Transport(e.to_string())),
                }
            }
            ModbusChannelKind::DiscreteInput | ModbusChannelKind::InputRegister => {
                Err(IoError::TypeMismatch {
                    channel: channel.into(),
                    value,
                })
            }
        }
    }
}
