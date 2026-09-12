//! Classic CAN over the gs_usb protocol used by candleLight firmware.
//!
//! Protocol reference: https://github.com/candle-usb/candleLight_fw
//! nusb uses the Windows system WinUSB driver; no proprietary CAN SDK or
//! separately distributed libusb DLL is required. One IA2 CANopen node owns
//! an entire USB adapter. Shared channels/nodes require a future RX fanout.
//!
//! A successful send confirms USB delivery only, never a CAN bus ACK.
//! Some candleLight firmware echoes when the frame enters the controller,
//! before transmission succeeds. CANopen SDO replies/heartbeats remain the
//! evidence that the remote node answered. CAN FD is deliberately disabled.

use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use async_trait::async_trait;
use iocore::IoError;
use nusb::descriptors::TransferType;
use nusb::transfer::{Buffer, Bulk, ControlIn, ControlOut, ControlType, In, Out, Recipient};
use nusb::{Interface, MaybeFuture};

use crate::bus::CanBus;
use crate::frame::CanFrame;

const TIMEOUT: Duration = Duration::from_millis(250);
const HOST_FORMAT: u8 = 0;
const BITTIMING: u8 = 1;
const MODE: u8 = 2;
const BT_CONST: u8 = 4;
const DEVICE_CONFIG: u8 = 5;
const BERR_REPORTING: u32 = 1 << 12;
const USB_QUIRK: u32 = 1 << 9;
const CAN_EFF: u32 = 1 << 31;
const CAN_RTR: u32 = 1 << 30;
const CAN_ERR: u32 = 1 << 29;
const CAN_BUS_OFF: u32 = 1 << 6;
const RX_ECHO: u32 = u32::MAX;
const FRAME_LEN: usize = 20;

/// Stable device selection; an empty serial is allowed only when exactly one
/// connected adapter has the requested VID/PID. Channel is decimal, IDs hex.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selection {
    pub vid: u16,
    pub pid: u16,
    pub serial: String,
    pub channel: u8,
    pub bitrate: u32,
}

impl Selection {
    pub fn parse(interface: &str, bitrate: Option<u32>) -> Result<Self, IoError> {
        let parts: Vec<_> = interface.split(':').collect();
        if parts.len() != 5 || parts[0] != "gs_usb" {
            return Err(connect("expected gs_usb:<vid>:<pid>:<serial>:<channel>"));
        }
        let hex = |s: &str| {
            if s.len() != 4 || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err(connect(
                    "USB VID and PID must each be four hexadecimal digits",
                ));
            }
            u16::from_str_radix(s, 16).map_err(|_| connect("invalid USB VID/PID"))
        };
        if parts[3].chars().any(char::is_control) {
            return Err(connect("USB serial must not contain control characters"));
        }
        if parts[4].is_empty() || !parts[4].bytes().all(|b| b.is_ascii_digit()) {
            return Err(connect("USB CAN channel must be decimal 0-255"));
        }
        let channel = parts[4]
            .parse()
            .map_err(|_| connect("USB CAN channel must be 0-255"))?;
        let bitrate = bitrate
            .filter(|b| (1..=1_000_000).contains(b))
            .ok_or_else(|| connect("gs_usb requires bitrate in 1..=1000000 bit/s (classic CAN)"))?;
        Ok(Self {
            vid: hex(parts[1])?,
            pid: hex(parts[2])?,
            serial: parts[3].into(),
            channel,
            bitrate,
        })
    }

    fn matches(&self, vid: u16, pid: u16, serial: Option<&str>) -> bool {
        self.vid == vid
            && self.pid == pid
            && (self.serial.is_empty() || serial == Some(self.serial.as_str()))
    }
}

fn connect(message: impl std::fmt::Display) -> IoError {
    IoError::Connect(format!("gs_usb: {message}"))
}
fn transport(message: impl std::fmt::Display) -> IoError {
    IoError::Transport(format!("gs_usb: {message}"))
}

fn unique<T>(mut matches: impl Iterator<Item = T>) -> Result<T, IoError> {
    let candidate = matches.next().ok_or_else(|| connect("no matching USB CAN adapter; connect the specified gs_usb device and ensure its interface uses WinUSB"))?;
    if matches.next().is_some() {
        return Err(connect(
            "multiple USB CAN adapters match; specify a unique serial number",
        ));
    }
    Ok(candidate)
}

static OWNERS: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();

struct Lease(String);
impl Lease {
    fn acquire(id: String) -> Result<Self, IoError> {
        let mut owners = OWNERS
            .get_or_init(Default::default)
            .lock()
            .map_err(|_| connect("USB ownership lock poisoned"))?;
        if !owners.insert(id.clone()) {
            return Err(connect("USB adapter already in use by IA2; this backend supports one CANopen node per USB adapter (including all channels)"));
        }
        Ok(Self(id))
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        if let Ok(mut owners) = OWNERS.get_or_init(Default::default).lock() {
            owners.remove(&self.0);
        }
    }
}

/// A queued USB transport. RX transfers outlive individual `recv` futures:
/// the CANopen io loop may cancel a wait with `select!` without losing a frame.
pub struct GsUsbBus {
    input: nusb::Endpoint<Bulk, In>,
    output: nusb::Endpoint<Bulk, Out>,
    reset: ResetGuard,
    _lease: Lease,
    failed: Option<String>,
    echo: u32,
}

/// Also covers partial initialization, cancellation, and a dropped io task.
/// Normal shutdown awaits reset and reports its error; Drop is bounded fallback.
struct ResetGuard {
    interface: Interface,
    channel: u8,
    armed: bool,
}

impl ResetGuard {
    async fn stop(&mut self) -> Result<(), IoError> {
        if !self.armed {
            return Ok(());
        }
        let result = control_out(&self.interface, MODE, self.channel as u16, &[0; 8]).await;
        // Keep the Drop fallback armed while the request is pending: callers
        // may cancel shutdown itself.
        self.armed = false;
        result
    }
}

impl Drop for ResetGuard {
    fn drop(&mut self) {
        if self.armed {
            let request = out_request(MODE, self.channel as u16, &[0; 8]);
            if let Err(e) = self.interface.control_out(request, TIMEOUT).wait() {
                tracing::error!(%e, "gs_usb reset during drop failed; controller stop unconfirmed");
            }
        }
    }
}

impl GsUsbBus {
    /// Opens only the explicitly selected adapter. This low-level entry point
    /// is cross-platform for testing; IA2 selects it only on Windows.
    pub async fn open(selection: Selection) -> Result<Self, IoError> {
        let infos = nusb::list_devices().await.map_err(connect)?;
        let info = unique(
            infos.filter(|i| selection.matches(i.vendor_id(), i.product_id(), i.serial_number())),
        )?;
        let lease = Lease::acquire(format!("{:?}", info.id()))?;
        let device = info.open().await.map_err(|e| {
            connect(format!(
                "open adapter: {e}; verify WinUSB binding and close other CAN tools"
            ))
        })?;
        // gs_usb uses interface 0. Never detach another platform's kernel driver.
        let interface = device
            .claim_interface(0)
            .await
            .map_err(|e| connect(format!("claim USB interface 0: {e}; verify WinUSB binding")))?;
        control_out(&interface, HOST_FORMAT, 1, &0x0000_beefu32.to_le_bytes())
            .await
            .map_err(connect)?;
        let config = control_in(&interface, DEVICE_CONFIG, 1, 12)
            .await
            .map_err(connect)?;
        if config.len() != 12 || selection.channel as u16 > config[3] as u16 {
            return Err(connect(
                "invalid device configuration or selected CAN channel does not exist",
            ));
        }
        let capabilities = control_in(&interface, BT_CONST, selection.channel as u16, 40)
            .await
            .map_err(connect)?;
        let timing = bit_timing(&capabilities, selection.bitrate)?;
        let features = u32::from_le_bytes(
            capabilities[..4]
                .try_into()
                .expect("validated capabilities"),
        );
        if features & USB_QUIRK != 0 {
            return Err(connect(
                "adapter requires the LPC546XX USB packet quirk, which is not supported",
            ));
        }
        let mut reset = ResetGuard {
            interface,
            channel: selection.channel,
            armed: true,
        };
        control_out(&reset.interface, MODE, selection.channel as u16, &[0; 8])
            .await
            .map_err(connect)?;
        control_out(
            &reset.interface,
            BITTIMING,
            selection.channel as u16,
            &timing,
        )
        .await
        .map_err(connect)?;
        let descriptor = reset
            .interface
            .descriptor()
            .ok_or_else(|| connect("USB interface descriptor missing"))?;
        let endpoints: Vec<_> = descriptor
            .endpoints()
            .filter(|e| e.transfer_type() == TransferType::Bulk)
            .map(|e| e.address())
            .collect();
        let input_addr = unique(endpoints.iter().copied().filter(|a| a & 0x80 != 0))
            .map_err(|_| connect("USB interface 0 must have exactly one bulk IN endpoint"))?;
        let output_addr = unique(endpoints.iter().copied().filter(|a| a & 0x80 == 0))
            .map_err(|_| connect("USB interface 0 must have exactly one bulk OUT endpoint"))?;
        let mut input = reset
            .interface
            .endpoint::<Bulk, In>(input_addr)
            .map_err(connect)?;
        let output = reset
            .interface
            .endpoint::<Bulk, Out>(output_addr)
            .map_err(connect)?;
        if input.max_packet_size() < FRAME_LEN || output.max_packet_size() < FRAME_LEN {
            return Err(connect(
                "USB packet size is too small for a gs_usb classic frame",
            ));
        }
        // One classic frame per USB transfer. Allocate one complete USB packet
        // so unsolicited firmware padding is visible and can be rejected.
        for _ in 0..8 {
            input.submit(Buffer::new(input.max_packet_size()));
        }
        let mut mode = [0; 8];
        mode[..4].copy_from_slice(&1u32.to_le_bytes());
        mode[4..].copy_from_slice(&(features & BERR_REPORTING).to_le_bytes());
        control_out(&reset.interface, MODE, selection.channel as u16, &mode)
            .await
            .map_err(connect)?;
        // Keep reset armed for all paths after MODE START, including cancellation.
        reset.armed = true;
        tracing::info!(vid = selection.vid, pid = selection.pid, serial = %selection.serial, channel = selection.channel, bitrate = selection.bitrate, "gs_usb channel started; CANopen peer response still required");
        Ok(Self {
            input,
            output,
            reset,
            _lease: lease,
            failed: None,
            echo: 0,
        })
    }

    fn check(&self) -> Result<(), IoError> {
        match &self.failed {
            Some(e) => Err(transport(e)),
            None => Ok(()),
        }
    }

    fn fail(&mut self, e: impl std::fmt::Display) -> IoError {
        let message = e.to_string();
        self.failed = Some(message.clone());
        self.input.cancel_all();
        self.output.cancel_all();
        transport(message)
    }
}

#[async_trait]
impl CanBus for GsUsbBus {
    async fn send(&mut self, frame: CanFrame) -> Result<(), IoError> {
        self.check()?;
        let packet = encode(frame, self.reset.channel, self.echo)?;
        self.echo = self.echo.wrapping_add(1) % 10;
        // A previous future may have been cancelled after submission. Never
        // let its completion masquerade as confirmation of this new write.
        if self.output.pending() != 0 {
            return Err(self.fail("previous USB transmit was cancelled; reopen the adapter"));
        }
        self.output.submit(packet.to_vec().into());
        match tokio::time::timeout(TIMEOUT, self.output.next_complete()).await {
            Ok(c) => {
                if let Err(e) = c.status {
                    return Err(self.fail(format!("USB transmit: {e}")));
                }
                if c.actual_len != FRAME_LEN {
                    return Err(self.fail("short USB transmit; delivery unconfirmed"));
                }
                Ok(())
            }
            Err(_) => {
                Err(self
                    .fail("USB transmit timed out; delivery unconfirmed, adapter must be reopened"))
            }
        }
    }

    async fn recv(&mut self) -> Result<CanFrame, IoError> {
        self.check()?;
        let mut ignored = 0;
        loop {
            // nusb documents next_complete as cancel-safe; submitted buffers
            // remain queued when CANopen's select! chooses another branch.
            let c = self.input.next_complete().await;
            if let Err(e) = c.status {
                return Err(self.fail(format!("USB receive/disconnect: {e}")));
            }
            let result = decode(&c.buffer[..c.actual_len], self.reset.channel);
            self.input.submit(Buffer::new(self.input.max_packet_size()));
            match result {
                Ok(Some(frame)) => return Ok(frame),
                Ok(None) => {
                    ignored += 1;
                    if ignored == 16 {
                        ignored = 0;
                        tokio::task::yield_now().await;
                    }
                }
                Err(e) => return Err(self.fail(e)),
            }
        }
    }

    async fn shutdown(&mut self) -> Result<(), IoError> {
        self.failed
            .get_or_insert_with(|| "USB CAN channel closed".into());
        self.input.cancel_all();
        self.output.cancel_all();
        self.reset.stop().await
    }
}

fn out_request(request: u8, value: u16, data: &[u8]) -> ControlOut<'_> {
    ControlOut {
        control_type: ControlType::Vendor,
        recipient: Recipient::Interface,
        request,
        value,
        index: 0,
        data,
    }
}
async fn control_out(
    interface: &Interface,
    request: u8,
    value: u16,
    data: &[u8],
) -> Result<(), IoError> {
    interface
        .control_out(out_request(request, value, data), TIMEOUT)
        .await
        .map_err(transport)
}
async fn control_in(
    interface: &Interface,
    request: u8,
    value: u16,
    length: u16,
) -> Result<Vec<u8>, IoError> {
    interface
        .control_in(
            ControlIn {
                control_type: ControlType::Vendor,
                recipient: Recipient::Interface,
                request,
                value,
                index: 0,
                length,
            },
            TIMEOUT,
        )
        .await
        .map_err(transport)
}

fn encode(frame: CanFrame, channel: u8, echo: u32) -> Result<[u8; FRAME_LEN], IoError> {
    if frame.id > 0x7ff || frame.len > 8 {
        return Err(transport("CANopen requires an 11-bit ID and DLC <= 8"));
    }
    let mut out = [0; FRAME_LEN];
    out[..4].copy_from_slice(&echo.to_le_bytes());
    out[4..8].copy_from_slice(&(frame.id as u32).to_le_bytes());
    out[8] = frame.len;
    out[9] = channel;
    out[12..12 + frame.len as usize].copy_from_slice(frame.payload());
    Ok(out)
}

fn decode(packet: &[u8], channel: u8) -> Result<Option<CanFrame>, IoError> {
    if packet.len() != FRAME_LEN {
        return Err(transport(format!(
            "invalid classic CAN USB frame length {} (expected 20)",
            packet.len()
        )));
    }
    if packet[9] != channel {
        return Ok(None);
    }
    if packet[10] & 1 != 0 {
        return Err(transport("CAN receive overflow; data was lost"));
    }
    if packet[10] & !1 != 0 {
        return Err(transport(
            "unsupported CAN FD or frame flags in classic CAN mode",
        ));
    }
    if packet[8] > 8 {
        return Err(transport("invalid classic CAN DLC > 8"));
    }
    let id = u32::from_le_bytes(packet[4..8].try_into().expect("checked size"));
    if id & CAN_ERR != 0 {
        if id & CAN_BUS_OFF != 0 {
            return Err(transport(
                "CAN bus-off; channel stopped, inspect wiring/bitrate before reopening",
            ));
        }
        return Err(transport(format!(
            "CAN bus error {:#x}, detail {:02x?}; channel stopped",
            id & 0x1fff_ffff,
            &packet[12..20]
        )));
    }
    if id & CAN_EFF == 0 && id & 0x1fff_ffff > 0x7ff {
        return Err(transport("invalid standard CAN ID > 0x7ff"));
    }
    // CANopen predefined connections are standard data frames. Extended IDs
    // must never be truncated to 11 bits, and TX echo is not a peer response.
    if id & (CAN_EFF | CAN_RTR) != 0
        || u32::from_le_bytes(packet[..4].try_into().expect("checked size")) != RX_ECHO
    {
        return Ok(None);
    }
    Ok(Some(CanFrame::new(
        id as u16,
        &packet[12..12 + packet[8] as usize],
    )))
}

/// Derive exact nominal bit timing from controller capabilities, not an
/// assumed 48/80 MHz clock. Prefer common CAN sample points and more quanta.
fn bit_timing(reply: &[u8], bitrate: u32) -> Result<[u8; 20], IoError> {
    if reply.len() != 40 || bitrate == 0 || bitrate > 1_000_000 {
        return Err(connect("invalid bit timing capabilities/bitrate"));
    }
    let words: Vec<u32> = reply
        .chunks_exact(4)
        .map(|b| u32::from_le_bytes(b.try_into().expect("four-byte word")))
        .collect();
    let [_, clock, t1min, t1max, t2min, t2max, sjwmax, brpmin, brpmax, inc] = words.as_slice()
    else {
        unreachable!()
    };
    if *clock == 0
        || *clock > 1_000_000_000
        || *t1min == 0
        || t1min > t1max
        || *t1max > 1024
        || *t2min == 0
        || t2min > t2max
        || *t2max > 1024
        || *sjwmax == 0
        || *brpmin == 0
        || brpmin > brpmax
        || *brpmax > 65536
        || *inc == 0
    {
        return Err(connect(
            "invalid or unsupported controller bit timing limits",
        ));
    }
    let target = if bitrate > 800_000 {
        750u64
    } else if bitrate > 500_000 {
        800
    } else {
        875
    };
    let mut best: Option<(u64, u64, u32, u32, u32)> = None;
    for brp in (*brpmin..=*brpmax).step_by(*inc as usize) {
        let divisor = brp as u64 * bitrate as u64;
        if !(*clock as u64).is_multiple_of(divisor) {
            continue;
        }
        let tq = *clock as u64 / divisor;
        if tq < 1 + *t1min as u64 + *t2min as u64 || tq > 1 + *t1max as u64 + *t2max as u64 {
            continue;
        }
        let lo = (*t2min as u64).max(tq.saturating_sub(1 + *t1max as u64));
        let hi = (*t2max as u64).min(tq - 1 - *t1min as u64);
        if lo > hi {
            continue;
        }
        let ideal = (tq * (1000 - target) + 500) / 1000;
        let t2 = ideal.clamp(lo, hi) as u32;
        let t1 = (tq - 1 - t2 as u64) as u32;
        let error = ((1 + t1 as u64) * 1000).abs_diff(target * tq);
        if best.is_none_or(|(old_error, old_tq, ..)| {
            error * old_tq < old_error * tq || (error * old_tq == old_error * tq && tq > old_tq)
        }) {
            best = Some((error, tq, brp, t1, t2));
        }
    }
    let (_, _, brp, t1, t2) = best.ok_or_else(|| {
        connect(format!(
            "controller cannot represent bitrate {bitrate} exactly"
        ))
    })?;
    let mut out = [0; 20];
    for (chunk, value) in out
        .chunks_exact_mut(4)
        .zip([t1 / 2, t1 - t1 / 2, t2, 1, brp])
    {
        chunk.copy_from_slice(&value.to_le_bytes());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn received(id: u32, data: &[u8]) -> [u8; FRAME_LEN] {
        let mut packet = encode(CanFrame::new(0, data), 0, RX_ECHO).unwrap();
        packet[4..8].copy_from_slice(&id.to_le_bytes());
        packet
    }

    #[test]
    fn selection_is_explicit_and_unambiguous() {
        let s = Selection::parse("gs_usb:1d50:606f:ABC:0", Some(500_000)).unwrap();
        assert!(s.matches(0x1d50, 0x606f, Some("ABC")));
        assert!(!s.matches(0x1d50, 0x606f, Some("DEF")));
        assert!(Selection::parse("gs_usb:1d50:606f::0", None).is_err());
        for bad in [
            "gs_usb",
            "gs_usb:1d50:606f:0",
            "gs_usb:0x1d50:606f::0",
            "gs_usb:1d50:606f::256",
            "gs_usb:1d50:606f::+1",
        ] {
            assert!(Selection::parse(bad, Some(500_000)).is_err(), "{bad}");
        }
        assert!(unique(std::iter::empty::<u8>())
            .unwrap_err()
            .to_string()
            .contains("no matching"));
        assert!(unique([1, 2].into_iter())
            .unwrap_err()
            .to_string()
            .contains("multiple"));
        assert_eq!(unique([1].into_iter()).unwrap(), 1);
    }

    #[test]
    fn adapter_ownership_is_exclusive_and_released() {
        let owner = Lease::acquire("test-device".into()).unwrap();
        assert!(Lease::acquire("test-device".into()).is_err());
        drop(owner);
        assert!(Lease::acquire("test-device".into()).is_ok());
    }

    #[test]
    fn wire_frames_preserve_boundaries_and_do_not_forge_peer_responses() {
        for id in [0, 0x581, 0x7ff] {
            for len in 0..=8 {
                let data: Vec<_> = (0..len).collect();
                let packet = received(id, &data);
                assert_eq!(
                    decode(&packet, 0).unwrap(),
                    Some(CanFrame::new(id as u16, &data))
                );
            }
        }
        assert!(decode(&received(CAN_EFF | 0x581, &[1]), 0)
            .unwrap()
            .is_none());
        assert!(decode(&received(CAN_RTR | 0x581, &[1]), 0)
            .unwrap()
            .is_none());
        assert!(
            decode(&encode(CanFrame::new(0x581, &[1]), 0, 3).unwrap(), 0)
                .unwrap()
                .is_none()
        );
        assert!(decode(&received(0x581, &[1]), 1).unwrap().is_none());
        assert!(decode(&received(0x801, &[1]), 0).is_err());
    }

    #[test]
    fn malformed_frames_and_bus_faults_are_not_silently_accepted() {
        let p = received(0x581, &[0; 8]);
        for len in 0..FRAME_LEN {
            assert!(decode(&p[..len], 0).is_err());
        }
        let mut p = p;
        p[8] = 9;
        assert!(decode(&p, 0).is_err());
        p[8] = 8;
        for flag in [1, 2, 4, 8] {
            p[10] = flag;
            assert!(decode(&p, 0).is_err());
        }
        assert!(decode(&received(CAN_ERR | CAN_BUS_OFF, &[0; 8]), 0)
            .unwrap_err()
            .to_string()
            .contains("bus-off"));
        assert!(decode(&received(CAN_ERR | 4, &[0; 8]), 0).is_err());
        assert!(encode(
            CanFrame {
                id: 0x800,
                data: [0; 8],
                len: 8
            },
            0,
            0
        )
        .is_err());
        assert!(encode(
            CanFrame {
                id: 1,
                data: [0; 8],
                len: 9
            },
            0,
            0
        )
        .is_err());
    }

    fn capabilities(clock: u32) -> Vec<u8> {
        [BERR_REPORTING, clock, 1, 16, 1, 8, 4, 1, 1024, 1]
            .into_iter()
            .flat_map(u32::to_le_bytes)
            .collect()
    }

    #[test]
    fn bitrates_are_derived_from_the_reported_controller_clock() {
        for clock in [48_000_000, 80_000_000] {
            for bitrate in [
                10_000, 20_000, 50_000, 125_000, 250_000, 500_000, 800_000, 1_000_000,
            ] {
                let bytes = bit_timing(&capabilities(clock), bitrate).unwrap();
                let t: Vec<_> = bytes
                    .chunks_exact(4)
                    .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
                    .collect();
                assert_eq!(clock / (t[4] * (1 + t[0] + t[1] + t[2])), bitrate);
                assert!((1..=16).contains(&(t[0] + t[1])));
                assert!((1..=8).contains(&t[2]));
            }
        }
        assert!(bit_timing(&capabilities(48_000_000), 123_457).is_err());
        assert!(bit_timing(&[0; 40], 500_000).is_err());
        assert!(bit_timing(&[0; 39], 500_000).is_err());
    }
}
