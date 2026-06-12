//! Modbus TCP / RTU IoDevice adapter — mirror + bulk-poll model.
//!
//! The naive shape (one Modbus round-trip per channel per scan) dies at
//! plant scale: hundreds of channels × a scan loop = thousands of
//! requests per second, and RTU at 9600 baud manages ~20 round-trips/s
//! total. So this adapter mirrors the OPC UA one:
//!
//! - At connect, channels are sorted per function-code group and merged
//!   into contiguous read **spans** (small gaps are bridged — reading a
//!   few throwaway registers beats an extra round-trip).
//! - A background poll task owns the connection and refreshes the whole
//!   mirror every `poll_interval_ms` — a handful of bulk reads instead
//!   of N singles. `read_channel` returns the mirrored value.
//! - Writes (and failsafe) are forwarded to the same task over a command
//!   channel, so the single Modbus connection is never used
//!   concurrently — which RTU serial physically cannot tolerate anyway.
//!
//! 32-bit channel types (`u32`/`i32`/`f32`) span two consecutive
//! registers with configurable word order — the norm for instrument
//! floats and totalizers when IA2 talks to field devices directly.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use async_trait::async_trait;
use iocore::{ChannelValue, IoDevice, IoError};
use project::{
    ModbusChannel, ModbusChannelKind, ModbusConfig, ModbusDataBits, ModbusDataType, ModbusParity,
    ModbusRtuParams, ModbusStopBits, ModbusTcpParams, ModbusTransport, ModbusWordOrder,
};
use tokio::sync::{mpsc, oneshot};
use tokio_modbus::client::{rtu, tcp, Context, Reader, Writer};
use tokio_modbus::Slave;
use tokio_serial::{
    DataBits as SerialDataBits, Parity as SerialParity, SerialStream, StopBits as SerialStopBits,
};

/// Bridge a gap of up to this many registers/bits when merging adjacent
/// channels into one read span. Reading a few dead registers costs
/// microseconds; an extra round-trip costs milliseconds.
const MAX_SPAN_GAP: u16 = 8;
/// Protocol limits per read request (with a little headroom under the
/// spec maxima of 125 registers / 2000 bits).
const MAX_REGS_PER_READ: u16 = 120;
const MAX_BITS_PER_READ: u16 = 1968;

/// One contiguous read of `count` units starting at `start` for a
/// function-code group.
#[derive(Debug, Clone)]
struct Span {
    kind: ModbusChannelKind,
    start: u16,
    count: u16,
}

/// Commands the IoDevice façade sends to the connection-owning task.
enum Cmd {
    Write {
        channel: ModbusChannel,
        value: ChannelValue,
        ack: oneshot::Sender<Result<(), IoError>>,
    },
    Failsafe {
        ack: oneshot::Sender<Result<(), IoError>>,
    },
    Stop,
}

pub struct ModbusDevice {
    name: String,
    channels: HashMap<String, ModbusChannel>,
    mirror: Arc<RwLock<HashMap<String, ChannelValue>>>,
    cmd_tx: mpsc::Sender<Cmd>,
    task: Option<tokio::task::JoinHandle<()>>,
}

impl ModbusDevice {
    pub async fn connect(name: String, config: &ModbusConfig) -> Result<Self, IoError> {
        // Branch on transport: TCP opens a socket, RTU opens a serial
        // port. Past this point the Modbus PDUs are identical.
        let mut client = match &config.transport {
            ModbusTransport::Tcp(p) => Self::connect_tcp(p, config.slave_id).await?,
            ModbusTransport::Rtu(p) => Self::connect_rtu(p, config.slave_id).await?,
        };

        let channels: HashMap<String, ModbusChannel> = config
            .channels
            .iter()
            .map(|c| (c.name.clone(), c.clone()))
            .collect();
        let spans = plan_spans(&config.channels);
        let mirror = Arc::new(RwLock::new(HashMap::new()));

        // Seed the mirror with one full poll so the first scan round sees
        // real values — and so an unreachable slave fails the connect
        // loudly instead of silently serving zeros.
        poll_once(&mut client, &spans, &config.channels, &mirror).await?;

        let (cmd_tx, cmd_rx) = mpsc::channel(32);
        let task = tokio::spawn(poll_task(
            name.clone(),
            client,
            spans,
            config.channels.clone(),
            mirror.clone(),
            cmd_rx,
            Duration::from_millis(config.poll_interval_ms.max(20) as u64),
        ));

        tracing::info!(
            device = %name,
            channels = channels.len(),
            poll_ms = config.poll_interval_ms,
            "modbus connected; mirror seeded"
        );

        Ok(Self {
            name,
            channels,
            mirror,
            cmd_tx,
            task: Some(task),
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
        // 500 ms read timeout: generous for most slaves, short enough
        // that a missing slave doesn't wedge the poll task.
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

/// How many addressable units (registers or bits) a channel occupies.
fn unit_len(ch: &ModbusChannel) -> u16 {
    match ch.kind {
        ModbusChannelKind::Coil | ModbusChannelKind::DiscreteInput => 1,
        ModbusChannelKind::HoldingRegister | ModbusChannelKind::InputRegister => {
            ch.data_type.register_len()
        }
    }
}

/// Merge the channel list into bulk-read spans per function-code group:
/// sort by address, bridge gaps ≤ MAX_SPAN_GAP, split at protocol size
/// limits.
fn plan_spans(channels: &[ModbusChannel]) -> Vec<Span> {
    let mut spans = Vec::new();
    for kind in [
        ModbusChannelKind::Coil,
        ModbusChannelKind::DiscreteInput,
        ModbusChannelKind::HoldingRegister,
        ModbusChannelKind::InputRegister,
    ] {
        let max = match kind {
            ModbusChannelKind::Coil | ModbusChannelKind::DiscreteInput => MAX_BITS_PER_READ,
            _ => MAX_REGS_PER_READ,
        };
        let mut points: Vec<(u16, u16)> = channels
            .iter()
            .filter(|c| c.kind == kind)
            .map(|c| (c.address, unit_len(c)))
            .collect();
        points.sort_unstable();
        let mut current: Option<Span> = None;
        for (addr, len) in points {
            let end = addr.saturating_add(len);
            match current.as_mut() {
                Some(span)
                    if addr <= span.start + span.count + MAX_SPAN_GAP
                        && end - span.start <= max =>
                {
                    span.count = span.count.max(end - span.start);
                }
                _ => {
                    if let Some(done) = current.take() {
                        spans.push(done);
                    }
                    current = Some(Span {
                        kind,
                        start: addr,
                        count: len,
                    });
                }
            }
        }
        if let Some(done) = current.take() {
            spans.push(done);
        }
    }
    spans
}

/// Decode one channel's value out of a span's register window.
fn decode_regs(ch: &ModbusChannel, regs: &[u16]) -> ChannelValue {
    let hi_lo = |a: u16, b: u16| ((a as u32) << 16) | b as u32;
    match ch.data_type {
        ModbusDataType::U16 => ChannelValue::U16(regs.first().copied().unwrap_or(0)),
        ModbusDataType::I16 => {
            // Carry as numeric i32 so negative instrument values survive
            // (U16 would smuggle 65496 instead of -40).
            ChannelValue::I32(regs.first().copied().unwrap_or(0) as i16 as i32)
        }
        ModbusDataType::U32 | ModbusDataType::I32 | ModbusDataType::F32 => {
            let (a, b) = (
                regs.first().copied().unwrap_or(0),
                regs.get(1).copied().unwrap_or(0),
            );
            let raw = match ch.word_order {
                ModbusWordOrder::HiLo => hi_lo(a, b),
                ModbusWordOrder::LoHi => hi_lo(b, a),
            };
            match ch.data_type {
                ModbusDataType::F32 => ChannelValue::Real(f32::from_bits(raw)),
                _ => ChannelValue::I32(raw as i32),
            }
        }
    }
}

/// Encode a channel value into 1 or 2 registers for writing.
fn encode_regs(ch: &ModbusChannel, value: ChannelValue) -> Vec<u16> {
    match ch.data_type {
        ModbusDataType::U16 | ModbusDataType::I16 => vec![value.to_i32() as u16],
        ModbusDataType::U32 | ModbusDataType::I32 | ModbusDataType::F32 => {
            let raw: u32 = match ch.data_type {
                ModbusDataType::F32 => value.to_f32().to_bits(),
                _ => value.to_i32() as u32,
            };
            let (hi, lo) = ((raw >> 16) as u16, raw as u16);
            match ch.word_order {
                ModbusWordOrder::HiLo => vec![hi, lo],
                ModbusWordOrder::LoHi => vec![lo, hi],
            }
        }
    }
}

fn xerr<T>(e: impl std::fmt::Display) -> Result<T, IoError> {
    Err(IoError::Transport(e.to_string()))
}

/// One full mirror refresh: every span in one bulk read each.
async fn poll_once(
    client: &mut Context,
    spans: &[Span],
    channels: &[ModbusChannel],
    mirror: &Arc<RwLock<HashMap<String, ChannelValue>>>,
) -> Result<(), IoError> {
    for span in spans {
        match span.kind {
            ModbusChannelKind::Coil | ModbusChannelKind::DiscreteInput => {
                let res = match span.kind {
                    ModbusChannelKind::Coil => client.read_coils(span.start, span.count).await,
                    _ => client.read_discrete_inputs(span.start, span.count).await,
                };
                let bits = match res {
                    Ok(Ok(v)) => v,
                    Ok(Err(e)) => return xerr(format!("modbus exception: {e}")),
                    Err(e) => return xerr(e),
                };
                let mut m = mirror.write().expect("mirror poisoned");
                for ch in channels.iter().filter(|c| c.kind == span.kind) {
                    if ch.address >= span.start {
                        let off = (ch.address - span.start) as usize;
                        if let Some(&b) = bits.get(off) {
                            m.insert(ch.name.clone(), ChannelValue::Bool(b));
                        }
                    }
                }
            }
            ModbusChannelKind::HoldingRegister | ModbusChannelKind::InputRegister => {
                let res = match span.kind {
                    ModbusChannelKind::HoldingRegister => {
                        client.read_holding_registers(span.start, span.count).await
                    }
                    _ => client.read_input_registers(span.start, span.count).await,
                };
                let words = match res {
                    Ok(Ok(v)) => v,
                    Ok(Err(e)) => return xerr(format!("modbus exception: {e}")),
                    Err(e) => return xerr(e),
                };
                let mut m = mirror.write().expect("mirror poisoned");
                for ch in channels.iter().filter(|c| c.kind == span.kind) {
                    if ch.address >= span.start {
                        let off = (ch.address - span.start) as usize;
                        let end = off + ch.data_type.register_len() as usize;
                        if end <= words.len() {
                            m.insert(ch.name.clone(), decode_regs(ch, &words[off..end]));
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

/// Execute one write command against the connection.
async fn do_write(
    client: &mut Context,
    ch: &ModbusChannel,
    value: ChannelValue,
) -> Result<(), IoError> {
    match ch.kind {
        ModbusChannelKind::Coil => {
            let b = value.to_i32() != 0;
            match client.write_single_coil(ch.address, b).await {
                Ok(Ok(())) => Ok(()),
                Ok(Err(e)) => xerr(format!("modbus exception: {e}")),
                Err(e) => xerr(e),
            }
        }
        ModbusChannelKind::HoldingRegister => {
            let regs = encode_regs(ch, value);
            let res = if regs.len() == 1 {
                client.write_single_register(ch.address, regs[0]).await
            } else {
                client.write_multiple_registers(ch.address, &regs).await
            };
            match res {
                Ok(Ok(())) => Ok(()),
                Ok(Err(e)) => xerr(format!("modbus exception: {e}")),
                Err(e) => xerr(e),
            }
        }
        ModbusChannelKind::DiscreteInput | ModbusChannelKind::InputRegister => {
            Err(IoError::TypeMismatch {
                channel: ch.name.clone(),
                value,
            })
        }
    }
}

/// Zero every writable channel. Continues past per-channel errors and
/// returns the first — drive as many outputs safe as possible even if
/// one register write fails.
async fn do_failsafe(
    device: &str,
    client: &mut Context,
    channels: &[ModbusChannel],
) -> Result<(), IoError> {
    let mut first_err: Option<IoError> = None;
    for ch in channels.iter().filter(|c| {
        matches!(
            c.kind,
            ModbusChannelKind::Coil | ModbusChannelKind::HoldingRegister
        )
    }) {
        let zero = match ch.data_type {
            ModbusDataType::F32 => ChannelValue::Real(0.0),
            _ => ChannelValue::I32(0),
        };
        if let Err(e) = do_write(client, ch, zero).await {
            tracing::warn!(device = %device, channel = %ch.name, %e, "failsafe write failed");
            first_err.get_or_insert(e);
        }
    }
    match first_err {
        Some(e) => Err(e),
        None => {
            tracing::info!(device = %device, "modbus failsafe applied (outputs zeroed)");
            Ok(())
        }
    }
}

/// The connection-owning task: periodic mirror refresh, interleaved
/// with write/failsafe commands. Single owner = no concurrent use of
/// the transport (mandatory for RTU, polite for TCP slaves).
async fn poll_task(
    device: String,
    mut client: Context,
    spans: Vec<Span>,
    channels: Vec<ModbusChannel>,
    mirror: Arc<RwLock<HashMap<String, ChannelValue>>>,
    mut cmd_rx: mpsc::Receiver<Cmd>,
    interval: Duration,
) {
    let mut tick = tokio::time::interval(interval);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            cmd = cmd_rx.recv() => match cmd {
                Some(Cmd::Write { channel, value, ack }) => {
                    let _ = ack.send(do_write(&mut client, &channel, value).await);
                }
                Some(Cmd::Failsafe { ack }) => {
                    let _ = ack.send(do_failsafe(&device, &mut client, &channels).await);
                }
                Some(Cmd::Stop) | None => break,
            },
            _ = tick.tick() => {
                if let Err(e) = poll_once(&mut client, &spans, &channels, &mirror).await {
                    // Keep serving last-known values; the next tick retries.
                    tracing::warn!(device = %device, %e, "modbus poll failed; serving last-known values");
                }
            }
        }
    }
    tracing::debug!(device = %device, "modbus poll task exited");
}

#[async_trait]
impl IoDevice for ModbusDevice {
    fn name(&self) -> &str {
        &self.name
    }

    async fn read_channel(&mut self, channel: &str) -> Result<ChannelValue, IoError> {
        let ch = self.channel(channel)?;
        let default = match (ch.kind, ch.data_type) {
            (ModbusChannelKind::Coil | ModbusChannelKind::DiscreteInput, _) => {
                ChannelValue::Bool(false)
            }
            (_, ModbusDataType::F32) => ChannelValue::Real(0.0),
            (_, ModbusDataType::U16) => ChannelValue::U16(0),
            _ => ChannelValue::I32(0),
        };
        Ok(self
            .mirror
            .read()
            .expect("mirror poisoned")
            .get(channel)
            .copied()
            .unwrap_or(default))
    }

    async fn write_channel(&mut self, channel: &str, value: ChannelValue) -> Result<(), IoError> {
        let ch = self.channel(channel)?;
        let (ack, rx) = oneshot::channel();
        self.cmd_tx
            .send(Cmd::Write {
                channel: ch,
                value,
                ack,
            })
            .await
            .map_err(|_| IoError::Transport("modbus poll task gone".into()))?;
        rx.await
            .map_err(|_| IoError::Transport("modbus poll task gone".into()))?
    }

    async fn enter_failsafe(&mut self) -> Result<(), IoError> {
        let (ack, rx) = oneshot::channel();
        self.cmd_tx
            .send(Cmd::Failsafe { ack })
            .await
            .map_err(|_| IoError::Transport("modbus poll task gone".into()))?;
        rx.await
            .map_err(|_| IoError::Transport("modbus poll task gone".into()))?
    }

    async fn shutdown(&mut self) -> Result<(), IoError> {
        let _ = self.cmd_tx.send(Cmd::Stop).await;
        if let Some(task) = self.task.take() {
            // Bounded: the task exits on Stop within one poll cycle.
            let _ = tokio::time::timeout(Duration::from_secs(2), task).await;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reg(name: &str, addr: u16, dt: ModbusDataType, wo: ModbusWordOrder) -> ModbusChannel {
        ModbusChannel {
            name: name.into(),
            kind: ModbusChannelKind::HoldingRegister,
            address: addr,
            data_type: dt,
            word_order: wo,
        }
    }

    #[test]
    fn spans_merge_adjacent_and_bridge_small_gaps() {
        let chans = vec![
            reg("a", 0, ModbusDataType::U16, ModbusWordOrder::HiLo),
            reg("b", 1, ModbusDataType::F32, ModbusWordOrder::HiLo), // regs 1-2
            reg("c", 7, ModbusDataType::U16, ModbusWordOrder::HiLo), // gap of 4 ≤ 8 → same span
            reg("far", 1000, ModbusDataType::U16, ModbusWordOrder::HiLo), // new span
        ];
        let spans = plan_spans(&chans);
        assert_eq!(spans.len(), 2, "{spans:?}");
        assert_eq!(spans[0].start, 0);
        assert_eq!(spans[0].count, 8); // 0..=7
        assert_eq!(spans[1].start, 1000);
        assert_eq!(spans[1].count, 1);
    }

    #[test]
    fn f32_decodes_both_word_orders() {
        let bits = 12.7f32.to_bits();
        let (hi, lo) = ((bits >> 16) as u16, bits as u16);
        let abcd = reg("x", 0, ModbusDataType::F32, ModbusWordOrder::HiLo);
        let cdab = reg("x", 0, ModbusDataType::F32, ModbusWordOrder::LoHi);
        assert_eq!(decode_regs(&abcd, &[hi, lo]), ChannelValue::Real(12.7));
        assert_eq!(decode_regs(&cdab, &[lo, hi]), ChannelValue::Real(12.7));
        // encode is the inverse
        assert_eq!(encode_regs(&abcd, ChannelValue::Real(12.7)), vec![hi, lo]);
        assert_eq!(encode_regs(&cdab, ChannelValue::Real(12.7)), vec![lo, hi]);
    }

    #[test]
    fn i16_decodes_negative_values_numerically() {
        let ch = reg("t", 0, ModbusDataType::I16, ModbusWordOrder::HiLo);
        assert_eq!(decode_regs(&ch, &[(-40i16) as u16]), ChannelValue::I32(-40));
    }

    #[test]
    fn u32_decodes_word_orders() {
        let ch = reg("tot", 0, ModbusDataType::U32, ModbusWordOrder::LoHi);
        // value 0x0001_0002 stored lo-hi: [0x0002, 0x0001]
        assert_eq!(
            decode_regs(&ch, &[0x0002, 0x0001]),
            ChannelValue::I32(0x0001_0002)
        );
    }
}
