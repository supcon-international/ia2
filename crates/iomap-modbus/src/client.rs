use std::collections::HashMap;
use std::net::SocketAddr;
use std::str::FromStr;
use std::time::Duration;

use async_trait::async_trait;
use iocore::{ChannelValue, IoDevice, IoError};
use project::{
    ModbusChannel, ModbusChannelKind, ModbusConfig, ModbusDataBits, ModbusParity, ModbusRtuParams,
    ModbusStopBits, ModbusTcpParams, ModbusTransport,
};
use tokio_modbus::client::{rtu, tcp, Context, Reader, Writer};
use tokio_modbus::Slave;
use tokio_serial::{
    DataBits as SerialDataBits, Parity as SerialParity, SerialStream, StopBits as SerialStopBits,
};

pub struct ModbusDevice {
    name: String,
    client: Context,
    channels: HashMap<String, ModbusChannel>,
}

impl ModbusDevice {
    pub async fn connect(name: String, config: &ModbusConfig) -> Result<Self, IoError> {
        // Branch on transport: TCP opens a socket, RTU opens a
        // serial port. After this point both yield a
        // `tokio_modbus::client::Context` and the read/write paths
        // below are identical — Modbus PDUs are byte-for-byte the
        // same across both transports.
        let client = match &config.transport {
            ModbusTransport::Tcp(p) => Self::connect_tcp(p, config.slave_id).await?,
            ModbusTransport::Rtu(p) => Self::connect_rtu(p, config.slave_id).await?,
        };
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

    async fn connect_tcp(p: &ModbusTcpParams, slave_id: u8) -> Result<Context, IoError> {
        let addr_str = format!("{}:{}", p.host, p.port);
        let socket = SocketAddr::from_str(&addr_str)
            .map_err(|e| IoError::Connect(format!("invalid address {addr_str}: {e}")))?;
        tcp::connect_slave(socket, Slave(slave_id))
            .await
            .map_err(|e| IoError::Connect(e.to_string()))
    }

    async fn connect_rtu(p: &ModbusRtuParams, slave_id: u8) -> Result<Context, IoError> {
        // Build the serial port spec from our enum mirrors. Each
        // value maps 1:1 to a tokio_serial variant — we keep our
        // own enum so the wire/JSON shape doesn't depend on a
        // third-party crate's enum naming.
        //
        // 500 ms read timeout is generous for most slaves but short
        // enough that a missing slave doesn't wedge the scan loop;
        // tokio-modbus will surface it as a transport error which
        // the bridge logs and continues past.
        let builder = tokio_serial::new(&p.serial_device, p.baud_rate)
            .data_bits(match p.data_bits {
                ModbusDataBits::Five => SerialDataBits::Five,
                ModbusDataBits::Six => SerialDataBits::Six,
                ModbusDataBits::Seven => SerialDataBits::Seven,
                ModbusDataBits::Eight => SerialDataBits::Eight,
            })
            .parity(match p.parity {
                ModbusParity::None => SerialParity::None,
                ModbusParity::Even => SerialParity::Even,
                ModbusParity::Odd => SerialParity::Odd,
            })
            .stop_bits(match p.stop_bits {
                ModbusStopBits::One => SerialStopBits::One,
                ModbusStopBits::Two => SerialStopBits::Two,
            })
            .timeout(Duration::from_millis(500));
        let stream = SerialStream::open(&builder).map_err(|e| {
            IoError::Connect(format!(
                "opening serial port {device}: {e}",
                device = p.serial_device
            ))
        })?;
        Ok(rtu::attach_slave(stream, Slave(slave_id)))
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

    /// Zero every coil and holding register we know about. Discrete /
    /// input registers are read-only on the wire and silently skipped.
    /// We continue on per-channel errors and surface only the first —
    /// the loop's whole job is to drive as many outputs safe as it can,
    /// even if one slave is sick.
    async fn enter_failsafe(&mut self) -> Result<(), IoError> {
        // Snapshot the channel list out of the map so we don't hold a
        // borrow across the await points below.
        let writable: Vec<(String, ModbusChannelKind, u16)> = self
            .channels
            .values()
            .filter(|c| {
                matches!(
                    c.kind,
                    ModbusChannelKind::Coil | ModbusChannelKind::HoldingRegister
                )
            })
            .map(|c| (c.name.clone(), c.kind, c.address))
            .collect();
        let mut first_err: Option<IoError> = None;
        for (name, kind, address) in writable {
            let result = match kind {
                ModbusChannelKind::Coil => {
                    match self.client.write_single_coil(address, false).await {
                        Ok(Ok(())) => Ok(()),
                        Ok(Err(e)) => Err(IoError::Transport(format!("modbus exception: {e}"))),
                        Err(e) => Err(IoError::Transport(e.to_string())),
                    }
                }
                ModbusChannelKind::HoldingRegister => {
                    match self.client.write_single_register(address, 0u16).await {
                        Ok(Ok(())) => Ok(()),
                        Ok(Err(e)) => Err(IoError::Transport(format!("modbus exception: {e}"))),
                        Err(e) => Err(IoError::Transport(e.to_string())),
                    }
                }
                _ => Ok(()),
            };
            if let Err(e) = result {
                tracing::warn!(device = %self.name, channel = %name, %e, "failsafe write failed");
                if first_err.is_none() {
                    first_err = Some(e);
                }
            }
        }
        if let Some(e) = first_err {
            Err(e)
        } else {
            tracing::info!(device = %self.name, "modbus failsafe applied (outputs zeroed)");
            Ok(())
        }
    }
}
