//! Pure bit-packing helpers for PDI buffers.
//!
//! EtherCAT PDOs pack values at byte + bit offsets within a SubDevice's
//! input or output PDI buffer. For byte-aligned 8/16/32 bit values this
//! is a trivial slice; for digital I/O (1 bit, often 8 channels per byte)
//! we mask + shift. Centralised here so the cyclic-exchange code and the
//! `read_channel` / `write_channel` paths share one bit layout — and so
//! we can unit-test the layout without spinning up ethercrab at all.
//!
//! Endianness: EtherCAT is little-endian. All multi-byte reads/writes
//! use LE byte order. The byte order is not configurable.

use iocore::{ChannelValue, IoError};
use project::EthercatDataType;

/// Read `bit_length` bits starting at `(byte_offset, bit_offset)` from
/// `pdi` and decode according to `data_type`. Returns an `IoError` if
/// the range falls outside the PDI buffer.
pub fn read_value(
    pdi: &[u8],
    byte_offset: usize,
    bit_offset: u8,
    bit_length: u8,
    data_type: EthercatDataType,
) -> Result<ChannelValue, IoError> {
    if bit_length == 0 {
        return Err(IoError::Transport("bit_length must be > 0".into()));
    }
    let total_bits = bit_length as usize;
    let start_bit = (byte_offset * 8) + bit_offset as usize;
    let end_bit = start_bit + total_bits;
    if end_bit > pdi.len() * 8 {
        return Err(IoError::Transport(format!(
            "PDI read out of bounds: need bits {start_bit}..{end_bit}, have {} bits",
            pdi.len() * 8
        )));
    }

    // Bool fast-path: 1 bit, masked out of the byte.
    if matches!(data_type, EthercatDataType::Bool) || bit_length == 1 {
        let byte = pdi[byte_offset];
        let bit = (byte >> bit_offset) & 1;
        return Ok(ChannelValue::Bool(bit != 0));
    }

    // Byte-aligned fast paths for 8 / 16 / 32 bit values. EtherCAT
    // permits sub-byte alignment but our config UI doesn't surface it;
    // bit_offset != 0 with bit_length > 1 is an unsupported config that
    // we surface as an error rather than silently mis-pack.
    if bit_offset != 0 {
        return Err(IoError::Transport(format!(
            "non-byte-aligned multi-bit reads are not supported (bit_length={bit_length}, bit_offset={bit_offset})",
        )));
    }

    let bytes_needed = bit_length.div_ceil(8) as usize;
    let slice = &pdi[byte_offset..byte_offset + bytes_needed];

    Ok(match data_type {
        EthercatDataType::Bool => unreachable!("handled above"),
        EthercatDataType::U8 => ChannelValue::U16(slice[0] as u16),
        EthercatDataType::I8 => ChannelValue::U16(slice[0] as i8 as i16 as u16),
        EthercatDataType::U16 => ChannelValue::U16(u16::from_le_bytes([slice[0], slice[1]])),
        EthercatDataType::I16 => ChannelValue::U16(i16::from_le_bytes([slice[0], slice[1]]) as u16),
        EthercatDataType::U32 => {
            ChannelValue::I32(u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]) as i32)
        }
        EthercatDataType::I32 => {
            ChannelValue::I32(i32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
        }
        EthercatDataType::Real => {
            // REAL is IEEE-754 f32; quantise to i32 so it fits the same
            // ChannelValue lane as integer 32-bit. Loses fractional info
            // but matches how ironplc's VM sees integer-typed PLC vars.
            let f = f32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]);
            ChannelValue::I32(f as i32)
        }
    })
}

/// Encode `value` into `pdi` at `(byte_offset, bit_offset)` for
/// `bit_length` bits, coercing to `data_type`. Returns `IoError` on
/// out-of-bounds or unsupported alignment.
pub fn write_value(
    pdi: &mut [u8],
    byte_offset: usize,
    bit_offset: u8,
    bit_length: u8,
    data_type: EthercatDataType,
    value: ChannelValue,
) -> Result<(), IoError> {
    if bit_length == 0 {
        return Err(IoError::Transport("bit_length must be > 0".into()));
    }
    let start_bit = (byte_offset * 8) + bit_offset as usize;
    let end_bit = start_bit + bit_length as usize;
    if end_bit > pdi.len() * 8 {
        return Err(IoError::Transport(format!(
            "PDI write out of bounds: need bits {start_bit}..{end_bit}, have {} bits",
            pdi.len() * 8
        )));
    }

    // Bool / single-bit fast path.
    if matches!(data_type, EthercatDataType::Bool) || bit_length == 1 {
        let bit = match value {
            ChannelValue::Bool(b) => b as u8,
            _ => (value.to_i32() != 0) as u8,
        };
        let mask = 1u8 << bit_offset;
        let cell = &mut pdi[byte_offset];
        *cell = (*cell & !mask) | (bit << bit_offset);
        return Ok(());
    }

    if bit_offset != 0 {
        return Err(IoError::Transport(format!(
            "non-byte-aligned multi-bit writes are not supported (bit_length={bit_length}, bit_offset={bit_offset})",
        )));
    }

    let raw = value.to_i32();
    let bytes_needed = bit_length.div_ceil(8) as usize;
    let target = &mut pdi[byte_offset..byte_offset + bytes_needed];

    match data_type {
        EthercatDataType::Bool => unreachable!("handled above"),
        EthercatDataType::U8 | EthercatDataType::I8 => {
            target[0] = (raw & 0xff) as u8;
        }
        EthercatDataType::U16 | EthercatDataType::I16 => {
            let bytes = (raw as i16).to_le_bytes();
            target[..2].copy_from_slice(&bytes);
        }
        EthercatDataType::U32 | EthercatDataType::I32 => {
            let bytes = raw.to_le_bytes();
            target[..4].copy_from_slice(&bytes);
        }
        EthercatDataType::Real => {
            // Round-trip via f32 so the wire format is IEEE-754, even
            // though our ChannelValue lane is i32.
            let bytes = (raw as f32).to_le_bytes();
            target[..4].copy_from_slice(&bytes);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_write_bool_single_byte() {
        let mut pdi = [0u8; 1];
        write_value(
            &mut pdi,
            0,
            0,
            1,
            EthercatDataType::Bool,
            ChannelValue::Bool(true),
        )
        .unwrap();
        assert_eq!(pdi, [0b0000_0001]);
        let v = read_value(&pdi, 0, 0, 1, EthercatDataType::Bool).unwrap();
        assert_eq!(v, ChannelValue::Bool(true));
    }

    #[test]
    fn bool_bits_pack_independently_within_byte() {
        let mut pdi = [0u8; 1];
        // Set bit 0, bit 3, bit 7
        write_value(
            &mut pdi,
            0,
            0,
            1,
            EthercatDataType::Bool,
            ChannelValue::Bool(true),
        )
        .unwrap();
        write_value(
            &mut pdi,
            0,
            3,
            1,
            EthercatDataType::Bool,
            ChannelValue::Bool(true),
        )
        .unwrap();
        write_value(
            &mut pdi,
            0,
            7,
            1,
            EthercatDataType::Bool,
            ChannelValue::Bool(true),
        )
        .unwrap();
        assert_eq!(pdi, [0b1000_1001]);
        // Clear bit 3 — bits 0 and 7 must remain
        write_value(
            &mut pdi,
            0,
            3,
            1,
            EthercatDataType::Bool,
            ChannelValue::Bool(false),
        )
        .unwrap();
        assert_eq!(pdi, [0b1000_0001]);
        // Read individual bits
        assert_eq!(
            read_value(&pdi, 0, 0, 1, EthercatDataType::Bool).unwrap(),
            ChannelValue::Bool(true)
        );
        assert_eq!(
            read_value(&pdi, 0, 3, 1, EthercatDataType::Bool).unwrap(),
            ChannelValue::Bool(false)
        );
        assert_eq!(
            read_value(&pdi, 0, 7, 1, EthercatDataType::Bool).unwrap(),
            ChannelValue::Bool(true)
        );
    }

    #[test]
    fn u8_roundtrip() {
        let mut pdi = [0u8; 4];
        write_value(
            &mut pdi,
            1,
            0,
            8,
            EthercatDataType::U8,
            ChannelValue::U16(0x42),
        )
        .unwrap();
        assert_eq!(pdi, [0, 0x42, 0, 0]);
        let v = read_value(&pdi, 1, 0, 8, EthercatDataType::U8).unwrap();
        assert_eq!(v, ChannelValue::U16(0x42));
    }

    #[test]
    fn u16_little_endian() {
        let mut pdi = [0u8; 4];
        write_value(
            &mut pdi,
            0,
            0,
            16,
            EthercatDataType::U16,
            ChannelValue::U16(0x1234),
        )
        .unwrap();
        assert_eq!(pdi, [0x34, 0x12, 0, 0]);
        let v = read_value(&pdi, 0, 0, 16, EthercatDataType::U16).unwrap();
        assert_eq!(v, ChannelValue::U16(0x1234));
    }

    #[test]
    fn i16_negative_roundtrip() {
        let mut pdi = [0u8; 2];
        write_value(
            &mut pdi,
            0,
            0,
            16,
            EthercatDataType::I16,
            ChannelValue::U16(-100i16 as u16),
        )
        .unwrap();
        // -100 = 0xFF9C in two's-complement i16 (LE: 0x9C, 0xFF)
        assert_eq!(pdi, [0x9C, 0xFF]);
        let v = read_value(&pdi, 0, 0, 16, EthercatDataType::I16).unwrap();
        match v {
            ChannelValue::U16(raw) => assert_eq!(raw as i16, -100),
            _ => panic!("expected U16, got {v:?}"),
        }
    }

    #[test]
    fn i32_little_endian() {
        let mut pdi = [0u8; 6];
        write_value(
            &mut pdi,
            2,
            0,
            32,
            EthercatDataType::I32,
            ChannelValue::I32(-1),
        )
        .unwrap();
        // -1 in 32-bit LE is 0xFF * 4
        assert_eq!(pdi, [0, 0, 0xFF, 0xFF, 0xFF, 0xFF]);
        let v = read_value(&pdi, 2, 0, 32, EthercatDataType::I32).unwrap();
        assert_eq!(v, ChannelValue::I32(-1));
    }

    #[test]
    fn real_quantises_through_i32() {
        let mut pdi = [0u8; 4];
        write_value(
            &mut pdi,
            0,
            0,
            32,
            EthercatDataType::Real,
            ChannelValue::I32(42),
        )
        .unwrap();
        // 42.0_f32 le bytes
        assert_eq!(pdi, 42.0f32.to_le_bytes());
        let v = read_value(&pdi, 0, 0, 32, EthercatDataType::Real).unwrap();
        assert_eq!(v, ChannelValue::I32(42));
    }

    #[test]
    fn out_of_bounds_read_errors() {
        let pdi = [0u8; 2];
        let err = read_value(&pdi, 2, 0, 8, EthercatDataType::U8).unwrap_err();
        assert!(matches!(err, IoError::Transport(_)));
    }

    #[test]
    fn out_of_bounds_write_errors() {
        let mut pdi = [0u8; 2];
        let err = write_value(
            &mut pdi,
            0,
            0,
            32,
            EthercatDataType::U32,
            ChannelValue::I32(0),
        )
        .unwrap_err();
        assert!(matches!(err, IoError::Transport(_)));
    }

    #[test]
    fn non_aligned_multi_bit_is_rejected() {
        let mut pdi = [0u8; 4];
        // 16-bit value at bit_offset=2 is unsupported
        let err = write_value(
            &mut pdi,
            0,
            2,
            16,
            EthercatDataType::U16,
            ChannelValue::U16(0),
        )
        .unwrap_err();
        assert!(matches!(err, IoError::Transport(_)));
        let err = read_value(&pdi, 0, 2, 16, EthercatDataType::U16).unwrap_err();
        assert!(matches!(err, IoError::Transport(_)));
    }

    #[test]
    fn writes_dont_clobber_adjacent_bool_bits() {
        // Simulates a typical EL1008-style digital input byte where 8 channels
        // share one byte. Writing channel 4 must leave channels 0..3, 5..7
        // untouched.
        let mut pdi = [0b1111_1111u8; 1];
        write_value(
            &mut pdi,
            0,
            4,
            1,
            EthercatDataType::Bool,
            ChannelValue::Bool(false),
        )
        .unwrap();
        assert_eq!(pdi, [0b1110_1111]);
    }
}
