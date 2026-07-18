//! CiA 301 frame encode/decode — the pure-logic core of the adapter.
//!
//! Everything here is data-in/data-out (no I/O, no time), so the whole
//! protocol surface is unit-testable without a bus: COB-ID layout for
//! the predefined connection set, NMT commands, heartbeat parsing,
//! expedited SDO upload/download, and the little-endian value packing
//! for every `CanopenDataType`.

use iocore::ChannelValue;
use project::CanopenDataType;

/// One classic CAN 2.0A frame (11-bit id). CANopen's predefined
/// connection set lives entirely in the base id space.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CanFrame {
    pub id: u16,
    pub data: [u8; 8],
    pub len: u8,
}

impl CanFrame {
    pub fn new(id: u16, bytes: &[u8]) -> Self {
        debug_assert!(bytes.len() <= 8);
        let mut data = [0u8; 8];
        data[..bytes.len()].copy_from_slice(bytes);
        Self {
            id,
            data,
            len: bytes.len() as u8,
        }
    }

    pub fn payload(&self) -> &[u8] {
        &self.data[..self.len as usize]
    }
}

// ---------------- COB-IDs (predefined connection set) ----------------

pub mod cob {
    /// NMT command frame (master → all nodes).
    pub const NMT: u16 = 0x000;

    pub fn sdo_request(node: u8) -> u16 {
        0x600 + node as u16
    }
    pub fn sdo_response(node: u8) -> u16 {
        0x580 + node as u16
    }
    pub fn heartbeat(node: u8) -> u16 {
        0x700 + node as u16
    }
    pub fn emcy(node: u8) -> u16 {
        0x080 + node as u16
    }

    /// TPDO1..4 = 0x180/0x280/0x380/0x480 + node-id (device → master).
    pub fn tpdo(slot: u8, node: u8) -> u16 {
        0x180 + 0x100 * (slot.clamp(1, 4) as u16 - 1) + node as u16
    }
    /// RPDO1..4 = 0x200/0x300/0x400/0x500 + node-id (master → device).
    pub fn rpdo(slot: u8, node: u8) -> u16 {
        0x200 + 0x100 * (slot.clamp(1, 4) as u16 - 1) + node as u16
    }
}

// ---------------- NMT ----------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum NmtCommand {
    Start = 0x01,
    Stop = 0x02,
    EnterPreOperational = 0x80,
    ResetNode = 0x81,
    ResetCommunication = 0x82,
}

/// NMT frames address one node, or every node with `node = 0`.
pub fn nmt_frame(cmd: NmtCommand, node: u8) -> CanFrame {
    CanFrame::new(cob::NMT, &[cmd as u8, node])
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NmtState {
    BootUp,
    Stopped,
    Operational,
    PreOperational,
    Unknown(u8),
}

/// Heartbeat payload = 1 byte of NMT state (top toggle bit reserved).
pub fn parse_heartbeat(frame: &CanFrame) -> Option<NmtState> {
    if frame.len < 1 {
        return None;
    }
    Some(match frame.data[0] & 0x7F {
        0x00 => NmtState::BootUp,
        0x04 => NmtState::Stopped,
        0x05 => NmtState::Operational,
        0x7F => NmtState::PreOperational,
        other => NmtState::Unknown(other),
    })
}

// ---------------- SDO (expedited) ----------------
//
// Every `CanopenDataType` fits in 4 bytes, so the adapter only speaks
// expedited transfers. A segmented response (a server offering a big
// object) is surfaced as an explicit error rather than half-supported.

/// Client → server: read `index:sub` (initiate upload).
pub fn sdo_upload_request(node: u8, index: u16, sub: u8) -> CanFrame {
    let [lo, hi] = index.to_le_bytes();
    CanFrame::new(cob::sdo_request(node), &[0x40, lo, hi, sub, 0, 0, 0, 0])
}

/// Client → server: write `data` (1/2/4 bytes) to `index:sub`
/// (initiate download, expedited + size-indicated).
pub fn sdo_download_request(node: u8, index: u16, sub: u8, data: &[u8]) -> CanFrame {
    debug_assert!((1..=4).contains(&data.len()));
    let n = 4 - data.len() as u8;
    // ccs=1, e=1, s=1, n = unused bytes.
    let cmd = 0x23 | (n << 2);
    let [lo, hi] = index.to_le_bytes();
    let mut payload = [cmd, lo, hi, sub, 0, 0, 0, 0];
    payload[4..4 + data.len()].copy_from_slice(data);
    CanFrame::new(cob::sdo_request(node), &payload)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SdoResponse {
    /// Expedited upload came back with `len` valid data bytes.
    UploadOk {
        index: u16,
        sub: u8,
        data: [u8; 4],
        len: usize,
    },
    /// Download acknowledged.
    DownloadOk { index: u16, sub: u8 },
    /// Server aborted the transfer.
    Abort { index: u16, sub: u8, code: u32 },
    /// Server initiated a segmented transfer — out of scope.
    Segmented { index: u16, sub: u8 },
}

/// Decode a frame from the server's SDO response COB. Returns `None`
/// for frames that aren't SDO responses (too short / unknown command).
pub fn parse_sdo_response(frame: &CanFrame) -> Option<SdoResponse> {
    if frame.len < 8 {
        return None;
    }
    let cmd = frame.data[0];
    let index = u16::from_le_bytes([frame.data[1], frame.data[2]]);
    let sub = frame.data[3];
    match cmd >> 5 {
        // scs=2: upload response
        2 => {
            let expedited = cmd & 0x02 != 0;
            if !expedited {
                return Some(SdoResponse::Segmented { index, sub });
            }
            let size_indicated = cmd & 0x01 != 0;
            let n = ((cmd >> 2) & 0x03) as usize;
            let len = if size_indicated { 4 - n } else { 4 };
            let mut data = [0u8; 4];
            data.copy_from_slice(&frame.data[4..8]);
            Some(SdoResponse::UploadOk {
                index,
                sub,
                data,
                len,
            })
        }
        // scs=3: download response
        3 => Some(SdoResponse::DownloadOk { index, sub }),
        // cs=4: abort
        4 => {
            let code =
                u32::from_le_bytes([frame.data[4], frame.data[5], frame.data[6], frame.data[7]]);
            Some(SdoResponse::Abort { index, sub, code })
        }
        _ => None,
    }
}

/// Human text for the abort codes a supervisory master actually hits.
pub fn abort_text(code: u32) -> &'static str {
    match code {
        0x0503_0000 => "toggle bit not alternated",
        0x0504_0000 => "SDO protocol timed out",
        0x0504_0001 => "command specifier not valid",
        0x0601_0000 => "unsupported access to object",
        0x0601_0001 => "attempt to read a write-only object",
        0x0601_0002 => "attempt to write a read-only object",
        0x0602_0000 => "object does not exist in the object dictionary",
        0x0604_0041 => "object cannot be mapped to the PDO",
        0x0604_0042 => "mapped objects exceed PDO length",
        0x0606_0000 => "access failed due to a hardware error",
        0x0607_0010 => "data type does not match, length of service parameter does not match",
        0x0607_0012 => "data type does not match, length too high",
        0x0607_0013 => "data type does not match, length too low",
        0x0609_0011 => "sub-index does not exist",
        0x0609_0030 => "value range of parameter exceeded",
        0x0609_0031 => "value of parameter written too high",
        0x0609_0032 => "value of parameter written too low",
        0x0800_0000 => "general error",
        0x0800_0020 => "data cannot be transferred or stored",
        0x0800_0022 => "data cannot be transferred or stored because of the present device state",
        _ => "unknown abort code",
    }
}

// ---------------- Value packing ----------------

/// Byte width of a data type on the wire.
pub fn type_len(ty: CanopenDataType) -> usize {
    match ty {
        CanopenDataType::Bool | CanopenDataType::U8 | CanopenDataType::I8 => 1,
        CanopenDataType::U16 | CanopenDataType::I16 => 2,
        CanopenDataType::U32 | CanopenDataType::I32 | CanopenDataType::F32 => 4,
    }
}

/// Encode a channel value as its little-endian wire bytes.
pub fn value_to_bytes(value: ChannelValue, ty: CanopenDataType) -> ([u8; 4], usize) {
    let mut out = [0u8; 4];
    let len = type_len(ty);
    match ty {
        CanopenDataType::Bool => out[0] = (value.to_i32() != 0) as u8,
        CanopenDataType::U8 => out[0] = value.to_i32() as u8,
        CanopenDataType::I8 => out[0] = value.to_i32() as i8 as u8,
        CanopenDataType::U16 => out[..2].copy_from_slice(&(value.to_i32() as u16).to_le_bytes()),
        CanopenDataType::I16 => out[..2].copy_from_slice(&(value.to_i32() as i16).to_le_bytes()),
        CanopenDataType::U32 => out.copy_from_slice(&(value.to_i32() as u32).to_le_bytes()),
        CanopenDataType::I32 => out.copy_from_slice(&value.to_i32().to_le_bytes()),
        CanopenDataType::F32 => out.copy_from_slice(&value.to_f32().to_le_bytes()),
    }
    (out, len)
}

/// Decode little-endian wire bytes into the channel-lane value for the
/// declared type. `bytes` must hold at least `type_len(ty)` bytes.
pub fn bytes_to_value(bytes: &[u8], ty: CanopenDataType) -> Option<ChannelValue> {
    if bytes.len() < type_len(ty) {
        return None;
    }
    Some(match ty {
        CanopenDataType::Bool => ChannelValue::Bool(bytes[0] != 0),
        CanopenDataType::U8 => ChannelValue::U16(bytes[0] as u16),
        CanopenDataType::I8 => ChannelValue::I32(bytes[0] as i8 as i32),
        CanopenDataType::U16 => ChannelValue::U16(u16::from_le_bytes([bytes[0], bytes[1]])),
        CanopenDataType::I16 => ChannelValue::I32(i16::from_le_bytes([bytes[0], bytes[1]]) as i32),
        CanopenDataType::U32 => {
            ChannelValue::I32(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as i32)
        }
        CanopenDataType::I32 => {
            ChannelValue::I32(i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
        }
        CanopenDataType::F32 => {
            ChannelValue::Real(f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cob_ids_follow_predefined_connection_set() {
        assert_eq!(cob::sdo_request(0x22), 0x622);
        assert_eq!(cob::sdo_response(0x22), 0x5A2);
        assert_eq!(cob::heartbeat(0x05), 0x705);
        assert_eq!(cob::emcy(0x05), 0x085);
        assert_eq!(cob::tpdo(1, 0x10), 0x190);
        assert_eq!(cob::tpdo(4, 0x10), 0x490);
        assert_eq!(cob::rpdo(1, 0x10), 0x210);
        assert_eq!(cob::rpdo(4, 0x10), 0x510);
    }

    #[test]
    fn nmt_frame_addresses_one_node_or_all() {
        let f = nmt_frame(NmtCommand::Start, 5);
        assert_eq!(f.id, 0x000);
        assert_eq!(f.payload(), &[0x01, 5]);
        let all = nmt_frame(NmtCommand::ResetCommunication, 0);
        assert_eq!(all.payload(), &[0x82, 0]);
    }

    #[test]
    fn heartbeat_states_parse() {
        let hb = |b: u8| CanFrame::new(cob::heartbeat(3), &[b]);
        assert_eq!(parse_heartbeat(&hb(0x00)), Some(NmtState::BootUp));
        assert_eq!(parse_heartbeat(&hb(0x04)), Some(NmtState::Stopped));
        assert_eq!(parse_heartbeat(&hb(0x05)), Some(NmtState::Operational));
        assert_eq!(parse_heartbeat(&hb(0x7F)), Some(NmtState::PreOperational));
        // Toggle bit (bit 7) must be masked off.
        assert_eq!(parse_heartbeat(&hb(0x85)), Some(NmtState::Operational));
        assert_eq!(parse_heartbeat(&CanFrame::new(0x703, &[])), None);
    }

    #[test]
    fn sdo_upload_request_encodes_ccs2() {
        let f = sdo_upload_request(0x22, 0x6041, 0x00);
        assert_eq!(f.id, 0x622);
        assert_eq!(f.payload(), &[0x40, 0x41, 0x60, 0x00, 0, 0, 0, 0]);
    }

    #[test]
    fn sdo_download_request_sets_size_bits() {
        // 2 bytes → n=2 → cmd 0x2B
        let f = sdo_download_request(0x22, 0x6040, 0, &0x000Fu16.to_le_bytes());
        assert_eq!(f.data[0], 0x2B);
        assert_eq!(&f.data[1..4], &[0x40, 0x60, 0x00]);
        assert_eq!(&f.data[4..6], &[0x0F, 0x00]);
        // 4 bytes → n=0 → cmd 0x23
        let f4 = sdo_download_request(1, 0x60FF, 0, &500i32.to_le_bytes());
        assert_eq!(f4.data[0], 0x23);
        // 1 byte → n=3 → cmd 0x2F
        let f1 = sdo_download_request(1, 0x6060, 0, &[0x03]);
        assert_eq!(f1.data[0], 0x2F);
    }

    #[test]
    fn sdo_upload_response_roundtrip() {
        // Server answers 0x4B (expedited, size-indicated, 2 bytes) for a u16.
        let f = CanFrame::new(
            cob::sdo_response(0x22),
            &[0x4B, 0x41, 0x60, 0x00, 0x37, 0x02, 0, 0],
        );
        match parse_sdo_response(&f) {
            Some(SdoResponse::UploadOk {
                index,
                sub,
                data,
                len,
            }) => {
                assert_eq!((index, sub, len), (0x6041, 0, 2));
                assert_eq!(u16::from_le_bytes([data[0], data[1]]), 0x0237);
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn sdo_download_ack_and_abort_parse() {
        let ack = CanFrame::new(cob::sdo_response(1), &[0x60, 0x40, 0x60, 0, 0, 0, 0, 0]);
        assert_eq!(
            parse_sdo_response(&ack),
            Some(SdoResponse::DownloadOk {
                index: 0x6040,
                sub: 0
            })
        );
        let abort = CanFrame::new(
            cob::sdo_response(1),
            &[0x80, 0x40, 0x60, 0x00, 0x02, 0x00, 0x01, 0x06],
        );
        match parse_sdo_response(&abort) {
            Some(SdoResponse::Abort { code, .. }) => {
                assert_eq!(code, 0x0601_0002);
                assert_eq!(abort_text(code), "attempt to write a read-only object");
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn segmented_upload_is_surfaced_not_mangled() {
        // scs=2 with e=0 → segmented initiate.
        let f = CanFrame::new(cob::sdo_response(1), &[0x41, 0x08, 0x10, 0, 16, 0, 0, 0]);
        assert_eq!(
            parse_sdo_response(&f),
            Some(SdoResponse::Segmented {
                index: 0x1008,
                sub: 0
            })
        );
    }

    #[test]
    fn value_packing_roundtrips_every_type() {
        let cases: Vec<(ChannelValue, CanopenDataType)> = vec![
            (ChannelValue::Bool(true), CanopenDataType::Bool),
            (ChannelValue::U16(200), CanopenDataType::U8),
            (ChannelValue::I32(-5), CanopenDataType::I8),
            (ChannelValue::U16(0xBEEF), CanopenDataType::U16),
            (ChannelValue::I32(-1234), CanopenDataType::I16),
            (ChannelValue::I32(70000), CanopenDataType::U32),
            (ChannelValue::I32(-70000), CanopenDataType::I32),
            (ChannelValue::Real(12.75), CanopenDataType::F32),
        ];
        for (v, ty) in cases {
            let (bytes, len) = value_to_bytes(v, ty);
            assert_eq!(len, type_len(ty));
            let back = bytes_to_value(&bytes[..len], ty).unwrap();
            // Compare through the numeric lane — variants may differ
            // (U8 decodes as U16) but the value must survive.
            assert_eq!(back.to_f64(), v.to_f64(), "{ty:?}");
        }
    }

    #[test]
    fn bytes_to_value_rejects_short_buffers() {
        assert!(bytes_to_value(&[0x01], CanopenDataType::U16).is_none());
        assert!(bytes_to_value(&[], CanopenDataType::Bool).is_none());
    }
}
