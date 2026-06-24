//! Parsing for the integer notations the ESI schema mixes freely.
//!
//! ETG.2000 writes integers three ways, sometimes within one file:
//! `#x1A00` (the EtherCAT-canonical hex form), `0x1A00` (C-style hex), or
//! `6656` (plain decimal). Vendors are not consistent, so every numeric
//! attribute/element is deserialized as a string and normalized through
//! [`parse_int`].

/// Parse an ESI integer in `#x..` / `0x..` / decimal form into `u64`.
/// Leading/trailing whitespace is tolerated (vendors pretty-print).
pub(crate) fn parse_int(s: &str) -> Result<u64, String> {
    let t = s.trim();
    let (radix, digits) = if let Some(h) = t.strip_prefix("#x").or_else(|| t.strip_prefix("#X")) {
        (16, h)
    } else if let Some(h) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
        (16, h)
    } else {
        (10, t)
    };
    if digits.is_empty() {
        return Err(format!("empty integer literal: {s:?}"));
    }
    u64::from_str_radix(digits, radix).map_err(|e| format!("bad integer {s:?}: {e}"))
}

/// Same as [`parse_int`] but range-checked into `u16` (PDO / object indices,
/// SM start addresses).
pub(crate) fn parse_u16(s: &str) -> Result<u16, String> {
    let v = parse_int(s)?;
    u16::try_from(v).map_err(|_| format!("value {v} does not fit u16 ({s:?})"))
}

/// Same as [`parse_int`] but range-checked into `u8` (sub-index, bit length,
/// control byte).
pub(crate) fn parse_u8(s: &str) -> Result<u8, String> {
    let v = parse_int(s)?;
    u8::try_from(v).map_err(|_| format!("value {v} does not fit u8 ({s:?})"))
}

/// Same as [`parse_int`] but into `u32` (vendor id, module ident).
pub(crate) fn parse_u32(s: &str) -> Result<u32, String> {
    let v = parse_int(s)?;
    u32::try_from(v).map_err(|_| format!("value {v} does not fit u32 ({s:?})"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_all_three_notations() {
        assert_eq!(parse_int("#x1A00").unwrap(), 0x1A00);
        assert_eq!(parse_int("0x1a00").unwrap(), 0x1A00);
        assert_eq!(parse_int("6656").unwrap(), 6656);
        assert_eq!(parse_int("  #xF050 ").unwrap(), 0xF050);
        assert_eq!(parse_int("#X74").unwrap(), 0x74);
    }

    #[test]
    fn range_checks() {
        assert_eq!(parse_u16("#x1A00").unwrap(), 0x1A00);
        assert!(parse_u16("#x1FFFF").is_err());
        assert_eq!(parse_u8("16").unwrap(), 16);
        assert!(parse_u8("#x100").is_err());
        assert_eq!(parse_u32("#x000C010D").unwrap(), 0x000C010D);
    }

    #[test]
    fn rejects_junk() {
        assert!(parse_int("").is_err());
        assert!(parse_int("#x").is_err());
        assert!(parse_int("zzz").is_err());
    }
}
