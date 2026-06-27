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
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use async_trait::async_trait;
use iocore::{ChannelValue, HealthTracker, HealthTransition, IoDevice, IoError};
use project::{
    ModbusChannel, ModbusChannelKind, ModbusConfig, ModbusDataBits, ModbusDataType, ModbusParity,
    ModbusStopBits, ModbusTransport, ModbusWordOrder,
};
#[cfg(target_os = "linux")]
use project::ModbusRs485;
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

/// Consecutive failed mirror refreshes before the device is flagged
/// unhealthy (one ERROR log per outage, not one per poll).
const UNHEALTHY_AFTER_FAILURES: u32 = 3;
/// Default per-request timeout (TCP connect + every Modbus request) when
/// `ModbusConfig.timeout_ms` is unset.
const DEFAULT_TIMEOUT_MS: u32 = 1_000;
/// Default initial reconnect backoff when `reconnect_backoff_ms` is unset.
const DEFAULT_RECONNECT_BACKOFF_MS: u32 = 1_000;
/// Reconnect backoff doubles per failed attempt up to this cap.
const RECONNECT_BACKOFF_CAP: Duration = Duration::from_secs(10);

/// Per-request timeout from config (adapter default when unset).
fn request_timeout(config: &ModbusConfig) -> Duration {
    Duration::from_millis(config.timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS).max(1) as u64)
}

/// Initial reconnect backoff from config (adapter default when unset).
fn initial_backoff(config: &ModbusConfig) -> Duration {
    let ms = config
        .reconnect_backoff_ms
        .unwrap_or(DEFAULT_RECONNECT_BACKOFF_MS)
        .max(10);
    Duration::from_millis(ms as u64)
}

/// Exponential reconnect backoff: starts at `initial`, doubles per
/// failed attempt, saturates at `cap`, resets on a healthy poll.
#[derive(Debug)]
struct Backoff {
    initial: Duration,
    cap: Duration,
    next: Duration,
}

impl Backoff {
    fn new(initial: Duration, cap: Duration) -> Self {
        let initial = initial.min(cap);
        Self {
            initial,
            cap,
            next: initial,
        }
    }

    /// Delay to wait before the next attempt (doubles for the one after).
    fn next_delay(&mut self) -> Duration {
        let delay = self.next;
        self.next = (self.next * 2).min(self.cap);
        delay
    }

    fn reset(&mut self) {
        self.next = self.initial;
    }
}

/// Internal transfer-failure classification. The distinction drives the
/// reconnect logic:
/// - `Protocol` — the slave answered with a Modbus exception code. The
///   link works; reconnecting would not help.
/// - `Transport` — socket/serial error or request timeout. The
///   connection is dead (or desynced) and must be re-established.
#[derive(Debug)]
enum XferError {
    Protocol(String),
    Transport(String),
}

impl XferError {
    fn message(&self) -> &str {
        match self {
            XferError::Protocol(m) | XferError::Transport(m) => m,
        }
    }

    /// Boundary mapping — both flavors surface to callers as
    /// `IoError::Transport`, preserving the adapter's existing error
    /// surface (`modbus exception: …` messages included).
    fn into_io(self) -> IoError {
        match self {
            XferError::Protocol(m) | XferError::Transport(m) => IoError::Transport(m),
        }
    }
}

/// Run one Modbus request under the per-request timeout and flatten
/// tokio-modbus's nested `Result<Result<T, Exception>, Error>` into our
/// transport/protocol classification. A timeout counts as transport: a
/// late response would desync the transaction stream (fatal on RTU,
/// risky on TCP), so the connection is rebuilt.
async fn request<T, P, E>(
    timeout: Duration,
    fut: impl std::future::Future<Output = Result<Result<T, P>, E>>,
) -> Result<T, XferError>
where
    P: std::fmt::Display,
    E: std::fmt::Display,
{
    match tokio::time::timeout(timeout, fut).await {
        Err(_) => Err(XferError::Transport(format!(
            "request timed out after {}ms",
            timeout.as_millis()
        ))),
        Ok(Err(e)) => Err(XferError::Transport(e.to_string())),
        Ok(Ok(Err(e))) => Err(XferError::Protocol(format!("modbus exception: {e}"))),
        Ok(Ok(Ok(v))) => Ok(v),
    }
}

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
    /// Mirrored from the poll task's `HealthTracker`: `false` after
    /// `UNHEALTHY_AFTER_FAILURES` consecutive failed refreshes, `true`
    /// again on the first successful poll.
    healthy: Arc<AtomicBool>,
    cmd_tx: mpsc::Sender<Cmd>,
    task: Option<tokio::task::JoinHandle<()>>,
}

impl ModbusDevice {
    pub async fn connect(name: String, config: &ModbusConfig) -> Result<Self, IoError> {
        let timeout = request_timeout(config);
        let mut client = establish(&config.transport, config.slave_id, timeout).await?;

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
        poll_once(&mut client, &spans, &config.channels, &mirror, timeout)
            .await
            .map_err(XferError::into_io)?;

        let healthy = Arc::new(AtomicBool::new(true));
        let (cmd_tx, cmd_rx) = mpsc::channel(32);
        let task = tokio::spawn(poll_task(
            name.clone(),
            client,
            config.clone(),
            spans,
            mirror.clone(),
            cmd_rx,
            healthy.clone(),
        ));

        tracing::info!(
            device = %name,
            channels = channels.len(),
            poll_ms = config.poll_interval_ms,
            timeout_ms = timeout.as_millis() as u64,
            "modbus connected; mirror seeded"
        );

        Ok(Self {
            name,
            channels,
            mirror,
            healthy,
            cmd_tx,
            task: Some(task),
        })
    }

    fn channel(&self, name: &str) -> Result<ModbusChannel, IoError> {
        self.channels
            .get(name)
            .cloned()
            .ok_or_else(|| IoError::UnknownChannel(name.into()))
    }
}

/// Open the configured transport: TCP opens a socket (bounded by the
/// per-request timeout — a SYN to a black-holed host can otherwise hang
/// for minutes), RTU opens a serial port. Past this point the Modbus
/// PDUs are identical. Used both at `connect` and by the poll task's
/// reconnect path.
async fn establish(
    transport: &ModbusTransport,
    slave_id: u8,
    timeout: Duration,
) -> Result<Context, IoError> {
    match transport {
        ModbusTransport::Tcp(p) => {
            let addr_str = format!("{}:{}", p.host, p.port);
            let socket = SocketAddr::from_str(&addr_str)
                .map_err(|e| IoError::Connect(format!("invalid address {addr_str}: {e}")))?;
            match tokio::time::timeout(timeout, tcp::connect_slave(socket, Slave(slave_id))).await {
                Err(_) => Err(IoError::Connect(format!(
                    "connect to {addr_str} timed out after {}ms",
                    timeout.as_millis()
                ))),
                Ok(Err(e)) => Err(IoError::Connect(e.to_string())),
                Ok(Ok(ctx)) => Ok(ctx),
            }
        }
        ModbusTransport::Rtu(p) => {
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
                .timeout(timeout);
            let stream = SerialStream::open(&builder).map_err(|e| {
                IoError::Connect(format!(
                    "opening serial port {device}: {e}",
                    device = p.serial_device
                ))
            })?;
            if let Some(rs485) = &p.rs485 {
                #[cfg(target_os = "linux")]
                apply_rs485_linux(&stream, rs485).map_err(|e| {
                    IoError::Connect(format!(
                        "enabling RS485 mode on {device}: {e}",
                        device = p.serial_device
                    ))
                })?;
                #[cfg(not(target_os = "linux"))]
                {
                    let _ = rs485;
                    tracing::warn!(
                        device = %p.serial_device,
                        "rs485 config ignored: TIOCSRS485 is Linux-only"
                    );
                }
            }
            Ok(rtu::attach_slave(stream, Slave(slave_id)))
        }
    }
}

/// Enable Linux RS485 mode (`TIOCSRS485`) on the freshly-opened serial fd so
/// the UART driver toggles RTS/DE around each frame. Required for RTS-gated
/// RS485 transceivers (common on cheap USB-485 dongles): without it the
/// transmitter never engages and every Modbus request times out — the byte
/// stream looks correct in software but nothing is driven onto the bus.
#[cfg(target_os = "linux")]
fn apply_rs485_linux(stream: &SerialStream, cfg: &ModbusRs485) -> std::io::Result<()> {
    use std::os::unix::io::AsRawFd;

    // linux/serial.h: `struct serial_rs485` is 8 x u32.
    #[repr(C)]
    struct SerialRs485 {
        flags: u32,
        delay_rts_before_send: u32,
        delay_rts_after_send: u32,
        _padding: [u32; 5],
    }
    const SER_RS485_ENABLED: u32 = 1 << 0;
    const SER_RS485_RTS_ON_SEND: u32 = 1 << 1;
    const SER_RS485_RTS_AFTER_SEND: u32 = 1 << 2;
    const SER_RS485_RX_DURING_TX: u32 = 1 << 4;
    const TIOCSRS485: libc::c_ulong = 0x542F;

    let mut flags = SER_RS485_ENABLED;
    flags |= if cfg.rts_on_send {
        SER_RS485_RTS_ON_SEND
    } else {
        SER_RS485_RTS_AFTER_SEND
    };
    if cfg.rx_during_tx {
        flags |= SER_RS485_RX_DURING_TX;
    }
    let rs = SerialRs485 {
        flags,
        delay_rts_before_send: cfg.delay_rts_before_send_ms,
        delay_rts_after_send: cfg.delay_rts_after_send_ms,
        _padding: [0; 5],
    };
    // SAFETY: the fd is open for the duration of the call and `&rs` is a
    // valid, correctly-sized `struct serial_rs485` argument for TIOCSRS485.
    let rc = unsafe { libc::ioctl(stream.as_raw_fd(), TIOCSRS485, &rs) };
    if rc != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
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
                    if addr
                        <= span
                            .start
                            .saturating_add(span.count)
                            .saturating_add(MAX_SPAN_GAP)
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

/// One full mirror refresh: every span in one bulk read each.
async fn poll_once(
    client: &mut Context,
    spans: &[Span],
    channels: &[ModbusChannel],
    mirror: &Arc<RwLock<HashMap<String, ChannelValue>>>,
    timeout: Duration,
) -> Result<(), XferError> {
    for span in spans {
        match span.kind {
            ModbusChannelKind::Coil | ModbusChannelKind::DiscreteInput => {
                let bits = match span.kind {
                    ModbusChannelKind::Coil => {
                        request(timeout, client.read_coils(span.start, span.count)).await?
                    }
                    _ => {
                        request(timeout, client.read_discrete_inputs(span.start, span.count))
                            .await?
                    }
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
                let words = match span.kind {
                    ModbusChannelKind::HoldingRegister => {
                        request(
                            timeout,
                            client.read_holding_registers(span.start, span.count),
                        )
                        .await?
                    }
                    _ => {
                        request(timeout, client.read_input_registers(span.start, span.count))
                            .await?
                    }
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

/// Execute one write command against the connection. Read-only kinds are
/// rejected up front in `write_channel`, before the command is queued.
async fn do_write(
    client: &mut Context,
    ch: &ModbusChannel,
    value: ChannelValue,
    timeout: Duration,
) -> Result<(), XferError> {
    match ch.kind {
        ModbusChannelKind::Coil => {
            let b = value.to_i32() != 0;
            request(timeout, client.write_single_coil(ch.address, b)).await
        }
        ModbusChannelKind::HoldingRegister => {
            let regs = encode_regs(ch, value);
            if regs.len() == 1 {
                request(timeout, client.write_single_register(ch.address, regs[0])).await
            } else {
                request(timeout, client.write_multiple_registers(ch.address, &regs)).await
            }
        }
        ModbusChannelKind::DiscreteInput | ModbusChannelKind::InputRegister => Err(
            XferError::Protocol(format!("channel '{}' is read-only", ch.name)),
        ),
    }
}

/// Zero every writable channel. Continues past per-channel *protocol*
/// errors and returns the first — drive as many outputs safe as possible
/// even if one register write is rejected. A *transport* error aborts the
/// sweep instead: the link is gone, so the remaining writes can only burn
/// the shutdown budget timing out one by one.
async fn do_failsafe(
    device: &str,
    client: &mut Context,
    channels: &[ModbusChannel],
    timeout: Duration,
) -> Result<(), XferError> {
    let mut first_err: Option<XferError> = None;
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
        match do_write(client, ch, zero, timeout).await {
            Ok(()) => {}
            Err(e @ XferError::Transport(_)) => {
                tracing::warn!(device = %device, channel = %ch.name, error = %e.message(), "failsafe aborted: transport gone");
                return Err(e);
            }
            Err(e) => {
                tracing::warn!(device = %device, channel = %ch.name, error = %e.message(), "failsafe write failed");
                first_err.get_or_insert(e);
            }
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

/// Drop a dead connection and schedule the next reconnect attempt.
fn drop_link(
    device: &str,
    link: &mut Option<Context>,
    retry_at: &mut tokio::time::Instant,
    backoff: &mut Backoff,
) {
    *link = None;
    let delay = backoff.next_delay();
    *retry_at = tokio::time::Instant::now() + delay;
    tracing::warn!(
        device = %device,
        retry_in_ms = delay.as_millis() as u64,
        "modbus transport error; connection dropped, reconnecting with backoff"
    );
}

/// Health bookkeeping for one failed mirror refresh. Exactly one ERROR
/// per outage (when the threshold is crossed); WARN before that, DEBUG
/// for the repeats while already unhealthy.
fn note_poll_failure(device: &str, health: &mut HealthTracker, error: &str) {
    match health.record_failure() {
        HealthTransition::BecameUnhealthy => {
            tracing::error!(
                device = %device,
                consecutive_failures = health.consecutive_failures(),
                error = %error,
                "modbus device unhealthy; serving last-known values until it recovers"
            );
        }
        _ if health.is_healthy() => {
            tracing::warn!(device = %device, error = %error, "modbus poll failed; serving last-known values");
        }
        _ => {
            tracing::debug!(device = %device, error = %error, "modbus poll still failing");
        }
    }
}

/// The connection-owning task: periodic mirror refresh, interleaved
/// with write/failsafe commands. Single owner = no concurrent use of
/// the transport (mandatory for RTU, polite for TCP slaves).
///
/// Resilience model:
/// - every failed refresh bumps the consecutive-failure counter; at
///   `UNHEALTHY_AFTER_FAILURES` the shared `healthy` flag flips (ERROR
///   logged once), and the first successful refresh clears it (INFO);
/// - a *transport* failure additionally drops the connection and
///   re-establishes it with exponential backoff (initial =
///   `reconnect_backoff_ms`, doubling to a 10 s cap) — instead of
///   polling a dead context forever. RTU reopens the serial port the
///   same way;
/// - while disconnected, writes fail fast and reads keep serving the
///   last-known mirror.
async fn poll_task(
    device: String,
    client: Context,
    config: ModbusConfig,
    spans: Vec<Span>,
    mirror: Arc<RwLock<HashMap<String, ChannelValue>>>,
    mut cmd_rx: mpsc::Receiver<Cmd>,
    healthy: Arc<AtomicBool>,
) {
    let interval = Duration::from_millis(config.poll_interval_ms.max(20) as u64);
    let timeout = request_timeout(&config);
    let mut health = HealthTracker::with_flag(UNHEALTHY_AFTER_FAILURES, healthy);
    let mut backoff = Backoff::new(initial_backoff(&config), RECONNECT_BACKOFF_CAP);
    let mut link: Option<Context> = Some(client);
    let mut retry_at = tokio::time::Instant::now();

    let mut tick = tokio::time::interval(interval);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            cmd = cmd_rx.recv() => match cmd {
                Some(Cmd::Write { channel, value, ack }) => {
                    let res = match link.as_mut() {
                        Some(client) => do_write(client, &channel, value, timeout).await,
                        None => Err(XferError::Transport(
                            "modbus connection down (reconnecting)".into(),
                        )),
                    };
                    if matches!(res, Err(XferError::Transport(_))) && link.is_some() {
                        drop_link(&device, &mut link, &mut retry_at, &mut backoff);
                    }
                    let _ = ack.send(res.map_err(XferError::into_io));
                }
                Some(Cmd::Failsafe { ack }) => {
                    let res = match link.as_mut() {
                        Some(client) => {
                            do_failsafe(&device, client, &config.channels, timeout).await
                        }
                        None => Err(XferError::Transport(
                            "modbus connection down (reconnecting)".into(),
                        )),
                    };
                    if matches!(res, Err(XferError::Transport(_))) && link.is_some() {
                        drop_link(&device, &mut link, &mut retry_at, &mut backoff);
                    }
                    let _ = ack.send(res.map_err(XferError::into_io));
                }
                Some(Cmd::Stop) | None => break,
            },
            _ = tick.tick() => {
                // Reconnect first if the link is down and the backoff has
                // elapsed, so the same tick can already refresh the mirror.
                if link.is_none() && tokio::time::Instant::now() >= retry_at {
                    match establish(&config.transport, config.slave_id, timeout).await {
                        Ok(ctx) => {
                            tracing::info!(device = %device, "modbus transport re-established");
                            link = Some(ctx);
                        }
                        Err(e) => {
                            let delay = backoff.next_delay();
                            retry_at = tokio::time::Instant::now() + delay;
                            tracing::debug!(
                                device = %device,
                                %e,
                                retry_in_ms = delay.as_millis() as u64,
                                "modbus reconnect attempt failed"
                            );
                        }
                    }
                }
                match link.as_mut() {
                    Some(client) => {
                        match poll_once(client, &spans, &config.channels, &mirror, timeout).await {
                            Ok(()) => {
                                backoff.reset();
                                if health.record_success() == HealthTransition::Recovered {
                                    tracing::info!(device = %device, "modbus device recovered; mirror refreshing again");
                                }
                            }
                            Err(e) => {
                                let transport_dead = matches!(e, XferError::Transport(_));
                                note_poll_failure(&device, &mut health, e.message());
                                if transport_dead {
                                    drop_link(&device, &mut link, &mut retry_at, &mut backoff);
                                }
                            }
                        }
                    }
                    // No link: the mirror is stale — count it as a failed
                    // refresh so the unhealthy flag flips even if the very
                    // first transport drop happened on a write.
                    None => note_poll_failure(
                        &device,
                        &mut health,
                        "connection down (reconnecting)",
                    ),
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
        // Reject read-only kinds before the round-trip to the poll task —
        // same `TypeMismatch` the write used to produce, just earlier.
        if matches!(
            ch.kind,
            ModbusChannelKind::DiscreteInput | ModbusChannelKind::InputRegister
        ) {
            return Err(IoError::TypeMismatch {
                channel: ch.name.clone(),
                value,
            });
        }
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

    /// `false` once the poll task has seen `UNHEALTHY_AFTER_FAILURES`
    /// consecutive failed refreshes (link down / slave silent); `true`
    /// again after the first successful refresh. Reads keep serving
    /// last-known values while unhealthy — this flag is how callers
    /// tell live data from a stale mirror.
    fn is_healthy(&self) -> bool {
        self.healthy.load(Ordering::Relaxed)
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
    fn spans_plan_without_overflow_at_top_of_address_space() {
        // Registers densely packed near the top of the 16-bit address space
        // (gaps ≤ MAX_SPAN_GAP) used to overflow `span.start + span.count +
        // MAX_SPAN_GAP` (u16): debug builds panicked, release wrapped and
        // mis-merged. With saturating adds the plan is computed cleanly.
        let chans = vec![
            reg("a", 65500, ModbusDataType::U16, ModbusWordOrder::HiLo),
            reg("b", 65507, ModbusDataType::U16, ModbusWordOrder::HiLo),
            reg("c", 65535, ModbusDataType::U16, ModbusWordOrder::HiLo),
        ];
        let spans = plan_spans(&chans); // must not panic
                                        // 65500 and 65507 are within MAX_SPAN_GAP → one span; 65535 is a
                                        // 28-register gap away → its own span.
        assert_eq!(spans.len(), 2, "{spans:?}");
        assert_eq!(spans[0].start, 65500);
        assert_eq!(spans[1].start, 65535);
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

    // ---- reconnect backoff state machine ---------------------------------

    #[test]
    fn backoff_doubles_to_cap_and_resets() {
        let mut b = Backoff::new(Duration::from_secs(1), Duration::from_secs(10));
        assert_eq!(b.next_delay(), Duration::from_secs(1));
        assert_eq!(b.next_delay(), Duration::from_secs(2));
        assert_eq!(b.next_delay(), Duration::from_secs(4));
        assert_eq!(b.next_delay(), Duration::from_secs(8));
        assert_eq!(b.next_delay(), Duration::from_secs(10), "caps at 10s");
        assert_eq!(b.next_delay(), Duration::from_secs(10), "stays capped");
        b.reset();
        assert_eq!(b.next_delay(), Duration::from_secs(1), "reset → initial");
    }

    #[test]
    fn backoff_initial_above_cap_is_clamped() {
        let mut b = Backoff::new(Duration::from_secs(60), Duration::from_secs(10));
        assert_eq!(b.next_delay(), Duration::from_secs(10));
        b.reset();
        assert_eq!(b.next_delay(), Duration::from_secs(10));
    }

    #[test]
    fn config_timing_helpers_apply_defaults_and_overrides() {
        let mut cfg = ModbusConfig {
            transport: ModbusTransport::Tcp(project::ModbusTcpParams {
                host: "127.0.0.1".into(),
                port: 502,
            }),
            slave_id: 1,
            poll_interval_ms: 100,
            timeout_ms: None,
            reconnect_backoff_ms: None,
            channels: vec![],
        };
        assert_eq!(request_timeout(&cfg), Duration::from_millis(1_000));
        assert_eq!(initial_backoff(&cfg), Duration::from_millis(1_000));
        cfg.timeout_ms = Some(250);
        cfg.reconnect_backoff_ms = Some(50);
        assert_eq!(request_timeout(&cfg), Duration::from_millis(250));
        assert_eq!(initial_backoff(&cfg), Duration::from_millis(50));
        // Degenerate values are clamped to something usable.
        cfg.timeout_ms = Some(0);
        cfg.reconnect_backoff_ms = Some(0);
        assert_eq!(request_timeout(&cfg), Duration::from_millis(1));
        assert_eq!(initial_backoff(&cfg), Duration::from_millis(10));
    }
}
