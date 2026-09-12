//! Read-only host inventory. Windows uses native APIs, without shelling
//! out or parsing localized command output; Unix keeps the existing paths.

use serde::Serialize;

#[derive(Serialize)]
struct Nic {
    name: String,
    mac: String,
    /// Kernel operational state (`up`, `down`, `unknown`, etc.).
    operstate: String,
    /// True only when the OS reports connected media.
    carrier: bool,
}

#[derive(Serialize)]
pub(super) struct SystemInfo {
    arch: String,
    os: String,
    nics: Vec<Nic>,
    serial_ports: Vec<String>,
}

pub(super) fn collect_system_info() -> SystemInfo {
    let mut nics = collect_nics();
    nics.sort_by(|a, b| a.name.cmp(&b.name));
    let mut serial_ports = collect_serial_ports();
    serial_ports.sort();
    serial_ports.dedup();
    SystemInfo {
        arch: std::env::consts::ARCH.to_string(),
        os: std::env::consts::OS.to_string(),
        nics,
        serial_ports,
    }
}

#[cfg(not(windows))]
fn collect_nics() -> Vec<Nic> {
    let mut nics = Vec::new();
    if let Ok(entries) = std::fs::read_dir("/sys/class/net") {
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            let p = e.path();
            let read = |f: &str| std::fs::read_to_string(p.join(f)).unwrap_or_default();
            nics.push(Nic {
                mac: read("address").trim().to_string(),
                operstate: read("operstate").trim().to_string(),
                carrier: read("carrier").trim() == "1",
                name,
            });
        }
    }
    nics
}

#[cfg(not(windows))]
fn collect_serial_ports() -> Vec<String> {
    let mut serial_ports = Vec::new();
    if let Ok(entries) = std::fs::read_dir("/dev") {
        for e in entries.flatten() {
            let n = e.file_name().to_string_lossy().to_string();
            // Keep the existing USB-serial/macOS callout inventory;
            // ttyS* nodes may be present without attached hardware.
            if n.starts_with("ttyUSB") || n.starts_with("ttyACM") || n.starts_with("cu.") {
                serial_ports.push(format!("/dev/{n}"));
            }
        }
    }
    serial_ports
}

#[cfg(windows)]
fn collect_nics() -> Vec<Nic> {
    windows::network_interfaces().unwrap_or_else(|error| {
        tracing::warn!(%error, "could not enumerate Windows network interfaces");
        Vec::new()
    })
}

#[cfg(windows)]
fn collect_serial_ports() -> Vec<String> {
    match tokio_serial::available_ports() {
        Ok(ports) => ports.into_iter().map(|port| port.port_name).collect(),
        Err(error) => {
            tracing::warn!(%error, "could not enumerate Windows serial ports");
            Vec::new()
        }
    }
}

#[cfg(windows)]
mod windows {
    use super::Nic;
    use windows_sys::Win32::NetworkManagement::{
        IpHelper::{FreeMibTable, GetIfTable2, MIB_IF_ROW2, MIB_IF_TABLE2},
        Ndis,
    };

    struct InterfaceTable(*mut MIB_IF_TABLE2);

    impl Drop for InterfaceTable {
        fn drop(&mut self) {
            // SAFETY: this owns the allocation returned by GetIfTable2.
            unsafe { FreeMibTable(self.0.cast()) };
        }
    }

    pub(super) fn network_interfaces() -> std::io::Result<Vec<Nic>> {
        let mut ptr = std::ptr::null_mut();
        // SAFETY: the output pointer is valid; the API allocates its table.
        let status = unsafe { GetIfTable2(&mut ptr) };
        if status != 0 {
            return Err(std::io::Error::from_raw_os_error(status as i32));
        }
        if ptr.is_null() {
            return Err(std::io::Error::other("GetIfTable2 returned a null table"));
        }
        let table = InterfaceTable(ptr);
        // SAFETY: successful GetIfTable2 provides NumEntries contiguous
        // rows. Addressing Table through its C struct preserves padding.
        // All strings/data are copied before the allocation is released.
        let rows = unsafe {
            std::slice::from_raw_parts(
                std::ptr::addr_of!((*table.0).Table).cast::<MIB_IF_ROW2>(),
                (*table.0).NumEntries as usize,
            )
        };
        Ok(rows.iter().map(nic_from_row).collect())
    }

    fn nic_from_row(row: &MIB_IF_ROW2) -> Nic {
        let end = row
            .Alias
            .iter()
            .position(|c| *c == 0)
            .unwrap_or(row.Alias.len());
        let name = String::from_utf16_lossy(&row.Alias[..end]);
        let mac_len = (row.PhysicalAddressLength as usize).min(row.PhysicalAddress.len());
        Nic {
            name,
            mac: row.PhysicalAddress[..mac_len]
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<Vec<_>>()
                .join(":"),
            operstate: match row.OperStatus {
                Ndis::IfOperStatusUp => "up",
                Ndis::IfOperStatusDown => "down",
                Ndis::IfOperStatusTesting => "testing",
                Ndis::IfOperStatusDormant => "dormant",
                Ndis::IfOperStatusNotPresent => "notpresent",
                Ndis::IfOperStatusLowerLayerDown => "lowerlayerdown",
                _ => "unknown",
            }
            .to_string(),
            carrier: row.MediaConnectState == Ndis::MediaConnectStateConnected,
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn interface_row_preserves_unicode_and_separates_link_from_operational_state() {
            let mut row = MIB_IF_ROW2 {
                OperStatus: Ndis::IfOperStatusDown,
                MediaConnectState: Ndis::MediaConnectStateConnected,
                PhysicalAddressLength: 6,
                ..Default::default()
            };
            let alias: Vec<u16> = "以太网 2".encode_utf16().collect();
            row.Alias[..alias.len()].copy_from_slice(&alias);
            row.PhysicalAddress[..6].copy_from_slice(&[0x00, 0x01, 0x02, 0xab, 0xcd, 0xef]);
            let nic = nic_from_row(&row);
            assert_eq!(nic.name, "以太网 2");
            assert_eq!(nic.mac, "00:01:02:ab:cd:ef");
            assert_eq!(nic.operstate, "down");
            assert!(nic.carrier);
            row.MediaConnectState = Ndis::MediaConnectStateUnknown;
            assert!(!nic_from_row(&row).carrier);
        }

        #[test]
        fn native_inventory_reads_interfaces_and_serial_ports_without_opening_them() {
            let nics = network_interfaces().expect("GetIfTable2 must succeed on Windows");
            assert!(!nics.is_empty(), "the host must expose at least loopback");
            assert!(nics.iter().all(|nic| !nic.name.is_empty()));
            tokio_serial::available_ports()
                .expect("serial enumeration must succeed without hardware");
        }
    }
}
