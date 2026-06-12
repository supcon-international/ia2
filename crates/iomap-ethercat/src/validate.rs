//! Connect-time validation of the configured topology against the
//! discovered bus.
//!
//! Pure functions over config + [`SlaveDiscovery`] lists so they are
//! unit-testable without hardware. Used by the real driver right after
//! the bus walk (identity + PDI ranges + channel shapes) and by the sim
//! driver against its derived extents (PDI ranges) — the goal is to fail
//! the *connect* with a precise message instead of dribbling per-cycle
//! `Transport` errors out of the PDI accessors later.

use project::{EthercatChannel, EthercatPdoDirection, EthercatSlave};

use crate::SlaveDiscovery;

/// Compare each configured slave's identity against what the bus walk
/// found at that index. A `vendor_id`/`product_id` of 0 means "not
/// authored" and skips that comparison (back-compat with configs written
/// before identity support); a slave with both at 0 is skipped entirely.
/// Returns every mismatch joined into one message — wrong module *and*
/// wrong position usually show up together, and seeing all of it beats
/// fixing one line per restart.
pub(crate) fn validate_identities(
    configured: &[EthercatSlave],
    discovered: &[SlaveDiscovery],
) -> Result<(), String> {
    let mut problems: Vec<String> = Vec::new();
    for slave in configured {
        if slave.vendor_id == 0 && slave.product_id == 0 {
            continue;
        }
        let Some(found) = discovered.iter().find(|d| d.index == slave.index) else {
            problems.push(format!(
                "slave {idx} ('{name}'): not found on the bus ({n} subdevice(s) discovered)",
                idx = slave.index,
                name = slave.name,
                n = discovered.len()
            ));
            continue;
        };
        if slave.vendor_id != 0 && slave.vendor_id != found.vendor_id {
            problems.push(format!(
                "slave {idx} ('{name}'): vendor_id mismatch: expected {exp:#010x}, found \
                 {got:#010x} ('{got_name}')",
                idx = slave.index,
                name = slave.name,
                exp = slave.vendor_id,
                got = found.vendor_id,
                got_name = found.name
            ));
        }
        if slave.product_id != 0 && slave.product_id != found.product_id {
            problems.push(format!(
                "slave {idx} ('{name}'): product_id mismatch: expected {exp:#010x}, found \
                 {got:#010x} ('{got_name}')",
                idx = slave.index,
                name = slave.name,
                exp = slave.product_id,
                got = found.product_id,
                got_name = found.name
            ));
        }
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(problems.join("; "))
    }
}

/// Verify every channel's PDI window fits inside the discovered (or, in
/// sim mode, derived) extents of its slave: a TxPDO channel must fit the
/// slave's input bytes, an RxPDO channel its output bytes. Counted in
/// bits so bit-packed digital I/O at the last byte's high bits still
/// passes while one bit past the end fails.
pub(crate) fn validate_pdi_ranges(
    channels: &[EthercatChannel],
    discovered: &[SlaveDiscovery],
) -> Result<(), String> {
    let mut problems: Vec<String> = Vec::new();
    for ch in channels {
        let Some(slave) = discovered.iter().find(|d| d.index == ch.slave_index) else {
            problems.push(format!(
                "channel '{name}': slave_index={idx} not present on the bus",
                name = ch.name,
                idx = ch.slave_index
            ));
            continue;
        };
        let (extent_bytes, region) = match ch.direction {
            EthercatPdoDirection::TxPdo => (slave.input_bytes, "input"),
            EthercatPdoDirection::RxPdo => (slave.output_bytes, "output"),
        };
        let start_bit = ch.pdi_byte_offset as u32 * 8 + ch.pdi_bit_offset as u32;
        let end_bit = start_bit + ch.bit_length as u32;
        if end_bit > extent_bytes as u32 * 8 {
            problems.push(format!(
                "channel '{name}': needs bits {start_bit}..{end_bit} of slave {idx} \
                 ('{slave_name}') {region} PDI, but it has only {extent_bytes} byte(s)",
                name = ch.name,
                idx = ch.slave_index,
                slave_name = slave.name,
            ));
        }
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(problems.join("; "))
    }
}

/// Reject channel shapes the PDI bit accessors cannot serve: zero-length
/// entries and non-byte-aligned multi-bit values (`bits.rs` supports
/// sub-byte offsets only for single-bit channels). Real mode only — sim
/// mode stores values per name and never touches the bit packers, so
/// legacy sim configs with odd shapes keep connecting.
pub(crate) fn validate_channel_shapes(channels: &[EthercatChannel]) -> Result<(), String> {
    let mut problems: Vec<String> = Vec::new();
    for ch in channels {
        if ch.bit_length == 0 {
            problems.push(format!(
                "channel '{name}': bit_length must be > 0",
                name = ch.name
            ));
            continue;
        }
        if ch.bit_length > 1 && ch.pdi_bit_offset != 0 {
            problems.push(format!(
                "channel '{name}': non-byte-aligned multi-bit entries are not supported \
                 (bit_length={len}, pdi_bit_offset={off})",
                name = ch.name,
                len = ch.bit_length,
                off = ch.pdi_bit_offset
            ));
        }
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(problems.join("; "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use project::EthercatDataType;

    fn slave(index: u16, name: &str, vendor_id: u32, product_id: u32) -> EthercatSlave {
        EthercatSlave {
            index,
            name: name.into(),
            vendor_id,
            product_id,
        }
    }

    fn found(
        index: u16,
        name: &str,
        vendor_id: u32,
        product_id: u32,
        input_bytes: u16,
        output_bytes: u16,
    ) -> SlaveDiscovery {
        SlaveDiscovery {
            index,
            name: name.into(),
            input_bytes,
            output_bytes,
            vendor_id,
            product_id,
        }
    }

    fn channel(
        name: &str,
        slave_index: u16,
        direction: EthercatPdoDirection,
        byte_offset: u16,
        bit_offset: u8,
        bit_length: u8,
    ) -> EthercatChannel {
        EthercatChannel {
            name: name.into(),
            slave_index,
            direction,
            pdo_index: 0x6000,
            sub_index: 1,
            bit_length,
            data_type: EthercatDataType::Bool,
            pdi_byte_offset: byte_offset,
            pdi_bit_offset: bit_offset,
        }
    }

    // ---- identity ---------------------------------------------------------

    #[test]
    fn matching_identity_passes() {
        let cfg = vec![slave(0, "SV660N", 0x00100000, 0x000C0108)];
        let bus = vec![found(0, "SV660-Ecat", 0x00100000, 0x000C0108, 32, 32)];
        assert!(validate_identities(&cfg, &bus).is_ok());
    }

    #[test]
    fn zero_ids_skip_all_identity_checks_even_when_absent() {
        // Back-compat: configs authored before identity support carry 0/0
        // and must keep connecting — even if the slave list doesn't line
        // up with the bus (PDI validation handles actual channel use).
        let cfg = vec![slave(7, "unknown", 0, 0)];
        assert!(validate_identities(&cfg, &[]).is_ok());
    }

    #[test]
    fn vendor_mismatch_reports_expected_and_found() {
        let cfg = vec![slave(0, "EL2008", 0x2, 0x07D83052)];
        let bus = vec![found(0, "EL1008", 0x2, 0x03F03052, 1, 0)];
        let err = validate_identities(&cfg, &bus).unwrap_err();
        assert!(err.contains("product_id mismatch"), "{err}");
        assert!(err.contains("expected 0x07d83052"), "{err}");
        assert!(err.contains("found 0x03f03052"), "{err}");
        assert!(err.contains("'EL1008'"), "{err}");
    }

    #[test]
    fn only_nonzero_fields_are_compared() {
        // product authored, vendor left 0 → vendor difference tolerated.
        let cfg = vec![slave(0, "drive", 0, 0xBEEF)];
        let bus = vec![found(0, "drive", 0x1234, 0xBEEF, 8, 8)];
        assert!(validate_identities(&cfg, &bus).is_ok());
        // …and the authored product still catches a mismatch.
        let bus_wrong = vec![found(0, "other", 0x1234, 0xF00D, 8, 8)];
        let err = validate_identities(&cfg, &bus_wrong).unwrap_err();
        assert!(err.contains("product_id mismatch"), "{err}");
        assert!(!err.contains("vendor_id mismatch"), "{err}");
    }

    #[test]
    fn authored_slave_missing_from_bus_is_an_error() {
        let cfg = vec![slave(1, "EL2008", 0x2, 0x07D83052)];
        let bus = vec![found(0, "EK1100", 0x2, 0x044C2C52, 0, 0)];
        let err = validate_identities(&cfg, &bus).unwrap_err();
        assert!(err.contains("slave 1 ('EL2008'): not found"), "{err}");
        assert!(err.contains("1 subdevice(s) discovered"), "{err}");
    }

    #[test]
    fn multiple_identity_problems_are_all_reported() {
        let cfg = vec![slave(0, "a", 0x1, 0x1), slave(1, "b", 0x2, 0x2)];
        let bus = vec![found(0, "x", 0x9, 0x1, 1, 1), found(1, "y", 0x2, 0x9, 1, 1)];
        let err = validate_identities(&cfg, &bus).unwrap_err();
        assert!(err.contains("vendor_id mismatch"), "{err}");
        assert!(err.contains("product_id mismatch"), "{err}");
        assert!(err.contains("; "), "issues joined: {err}");
    }

    // ---- PDI ranges --------------------------------------------------------

    #[test]
    fn in_range_channels_pass_both_directions() {
        let bus = vec![found(0, "mod", 0, 0, 2, 4)];
        let chans = vec![
            channel("in_word", 0, EthercatPdoDirection::TxPdo, 0, 0, 16),
            channel("out_dword", 0, EthercatPdoDirection::RxPdo, 0, 0, 32),
            // last bit of the last input byte — boundary, still fine
            channel("in_flag", 0, EthercatPdoDirection::TxPdo, 1, 7, 1),
        ];
        assert!(validate_pdi_ranges(&chans, &bus).is_ok());
    }

    #[test]
    fn txpdo_overflowing_input_bytes_is_an_error() {
        let bus = vec![found(0, "EL1002", 0, 0, 1, 0)];
        let chans = vec![channel("in_word", 0, EthercatPdoDirection::TxPdo, 0, 0, 16)];
        let err = validate_pdi_ranges(&chans, &bus).unwrap_err();
        assert!(err.contains("'in_word'"), "{err}");
        assert!(err.contains("bits 0..16"), "{err}");
        assert!(err.contains("input PDI"), "{err}");
        assert!(err.contains("only 1 byte(s)"), "{err}");
    }

    #[test]
    fn rxpdo_overflowing_output_bytes_is_an_error() {
        let bus = vec![found(0, "EL2008", 0, 0, 0, 1)];
        // byte_offset 1 starts past the single output byte
        let chans = vec![channel("out9", 0, EthercatPdoDirection::RxPdo, 1, 0, 1)];
        let err = validate_pdi_ranges(&chans, &bus).unwrap_err();
        assert!(err.contains("bits 8..9"), "{err}");
        assert!(err.contains("output PDI"), "{err}");
    }

    #[test]
    fn one_bit_past_the_end_fails_while_last_bit_passes() {
        let bus = vec![found(0, "io", 0, 0, 1, 0)];
        let last = vec![channel("ok", 0, EthercatPdoDirection::TxPdo, 0, 7, 1)];
        assert!(validate_pdi_ranges(&last, &bus).is_ok());
        let past = vec![channel("bad", 0, EthercatPdoDirection::TxPdo, 1, 0, 1)];
        assert!(validate_pdi_ranges(&past, &bus).is_err());
    }

    #[test]
    fn channel_on_undiscovered_slave_is_an_error() {
        let bus = vec![found(0, "only", 0, 0, 4, 4)];
        let chans = vec![channel("ghost", 3, EthercatPdoDirection::TxPdo, 0, 0, 8)];
        let err = validate_pdi_ranges(&chans, &bus).unwrap_err();
        assert!(err.contains("slave_index=3 not present"), "{err}");
    }

    #[test]
    fn all_range_problems_are_reported_together() {
        let bus = vec![found(0, "io", 0, 0, 1, 1)];
        let chans = vec![
            channel("a", 0, EthercatPdoDirection::TxPdo, 2, 0, 8),
            channel("b", 0, EthercatPdoDirection::RxPdo, 2, 0, 8),
            channel("c", 9, EthercatPdoDirection::TxPdo, 0, 0, 1),
        ];
        let err = validate_pdi_ranges(&chans, &bus).unwrap_err();
        assert_eq!(err.matches("channel '").count(), 3, "{err}");
    }

    // ---- channel shapes -----------------------------------------------------

    #[test]
    fn zero_bit_length_is_rejected() {
        let chans = vec![channel("z", 0, EthercatPdoDirection::TxPdo, 0, 0, 0)];
        let err = validate_channel_shapes(&chans).unwrap_err();
        assert!(err.contains("bit_length must be > 0"), "{err}");
    }

    #[test]
    fn misaligned_multibit_is_rejected_and_packed_bools_pass() {
        let bad = vec![channel("w", 0, EthercatPdoDirection::TxPdo, 0, 2, 16)];
        let err = validate_channel_shapes(&bad).unwrap_err();
        assert!(err.contains("non-byte-aligned"), "{err}");

        let good = vec![
            channel("bit5", 0, EthercatPdoDirection::TxPdo, 0, 5, 1),
            channel("word", 0, EthercatPdoDirection::TxPdo, 2, 0, 16),
        ];
        assert!(validate_channel_shapes(&good).is_ok());
    }
}
