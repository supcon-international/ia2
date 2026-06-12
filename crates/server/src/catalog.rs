//! Device-catalog registry (`<library-dir>/devices/*.toml`).
//!
//! A catalog entry is a field-validated device template: identity keys
//! (EtherCAT vendor/product) plus the channel layout that was proven on
//! real hardware. `GET /api/device-catalog` lists them; the match
//! endpoint turns a bus-discovery result into a pre-filled device, so
//! "hand-type the PDI offsets off the logs" becomes "confirm what the
//! scan recognised". See library/devices/README.md for the format.

use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// One channel template inside a catalog entry. Mirrors the project's
/// `EthercatChannel` minus `slave_index` (assigned when the template is
/// instantiated against a discovered slave position).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct CatalogChannel {
    pub name: String,
    /// `rx_pdo` (output, master→slave) or `tx_pdo` (input).
    pub direction: String,
    pub pdo_index: u16,
    #[serde(default)]
    pub sub_index: u8,
    pub bit_length: u8,
    pub data_type: String,
    pub pdi_byte_offset: u16,
    #[serde(default)]
    pub pdi_bit_offset: u8,
}

/// A device template from the catalog directory.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct CatalogEntry {
    pub name: String,
    pub protocol: String,
    #[serde(default)]
    pub description: String,
    /// EtherCAT identity keys; 0 = not applicable (e.g. Modbus entry).
    #[serde(default)]
    pub vendor_id: u32,
    #[serde(default)]
    pub product_id: u32,
    #[serde(default)]
    pub requires_dc_sync: Option<String>,
    #[serde(default)]
    pub recommended_cycle_us: Option<u32>,
    #[serde(default)]
    pub channels: Vec<CatalogChannel>,
    /// Bare file stem the entry was loaded from (for display).
    #[serde(default)]
    pub id: String,
}

/// Scan `<library_dir>/devices/` for catalog entries. Tolerant: a
/// missing directory or an unparsable file skips that entry — this is
/// a browse surface, not a validator.
pub fn scan(library_dir: Option<&Path>) -> Vec<CatalogEntry> {
    let Some(root) = library_dir else {
        return Vec::new();
    };
    let dir = root.join("devices");
    let Ok(entries) = fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        if path.extension().and_then(|e| e.to_str()) != Some("toml") || stem.starts_with('.') {
            continue;
        }
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        match toml::from_str::<CatalogEntry>(&text) {
            Ok(mut e) => {
                e.id = stem.to_string();
                out.push(e);
            }
            Err(err) => {
                tracing::warn!(file = %path.display(), %err, "catalog entry failed to parse; skipping");
            }
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// Find the catalog entry matching an EtherCAT identity, if any.
pub fn match_identity(
    entries: &[CatalogEntry],
    vendor_id: u32,
    product_id: u32,
) -> Option<&CatalogEntry> {
    entries
        .iter()
        .find(|e| e.vendor_id != 0 && e.vendor_id == vendor_id && e.product_id == product_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_reads_the_seed_catalog() {
        // The repo ships library/devices/inovance-sv660n.toml; scanning
        // the real library dir must parse it.
        let entries = scan(Some(Path::new("../../library")));
        let sv660n = entries
            .iter()
            .find(|e| e.id == "inovance-sv660n")
            .expect("seed entry parses");
        assert_eq!(sv660n.vendor_id, 1048576);
        assert_eq!(sv660n.product_id, 786701);
        assert_eq!(sv660n.channels.len(), 4);
        assert_eq!(sv660n.requires_dc_sync.as_deref(), Some("sync0"));
        assert!(
            match_identity(&entries, 1048576, 786701).is_some(),
            "identity lookup hits the entry"
        );
        assert!(match_identity(&entries, 1, 2).is_none());
    }
}
