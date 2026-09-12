//! Minimal explicitly loaded Npcap ABI. No wpcap/Packet import library.
//! Signatures/layouts follow the Npcap SDK's pcap.h (Windows C long is i32).

use super::{npcap_name, PacketIo};
use libloading::os::windows::Library;
use std::collections::HashSet;
use std::ffi::{c_char, c_int, c_void, CStr, CString, OsString};
use std::os::windows::ffi::OsStringExt;
use std::path::PathBuf;
use std::ptr::{self, NonNull};
use std::sync::{Mutex, OnceLock};
use windows_sys::Win32::NetworkManagement::IpHelper::{
    ConvertInterfaceAliasToLuid, ConvertInterfaceLuidToGuid,
};
use windows_sys::Win32::NetworkManagement::Ndis::NET_LUID_LH;
use windows_sys::Win32::System::LibraryLoader::{
    LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR, LOAD_LIBRARY_SEARCH_SYSTEM32,
};
use windows_sys::Win32::System::SystemInformation::GetSystemDirectoryW;

type Handle = *mut c_void;
#[repr(C)]
struct PacketHeader {
    seconds: i32,
    microseconds: i32,
    captured_len: u32,
    original_len: u32,
}
#[repr(C)]
struct BpfProgram {
    length: u32,
    instructions: *mut c_void,
}

struct Api {
    create: unsafe extern "C" fn(*const c_char, *mut c_char) -> Handle,
    set_snaplen: unsafe extern "C" fn(Handle, c_int) -> c_int,
    set_promisc: unsafe extern "C" fn(Handle, c_int) -> c_int,
    set_timeout: unsafe extern "C" fn(Handle, c_int) -> c_int,
    set_immediate_mode: unsafe extern "C" fn(Handle, c_int) -> c_int,
    activate: unsafe extern "C" fn(Handle) -> c_int,
    setnonblock: unsafe extern "C" fn(Handle, c_int, *mut c_char) -> c_int,
    datalink: unsafe extern "C" fn(Handle) -> c_int,
    compile: unsafe extern "C" fn(Handle, *mut BpfProgram, *const c_char, c_int, u32) -> c_int,
    setfilter: unsafe extern "C" fn(Handle, *mut BpfProgram) -> c_int,
    freecode: unsafe extern "C" fn(*mut BpfProgram),
    sendpacket: unsafe extern "C" fn(Handle, *const u8, c_int) -> c_int,
    next_ex: unsafe extern "C" fn(Handle, *mut *const PacketHeader, *mut *const u8) -> c_int,
    geterr: unsafe extern "C" fn(Handle) -> *const c_char,
    close: unsafe extern "C" fn(Handle),
    // Keep all function pointers valid through capture close.
    _library: Library,
}

impl Api {
    fn load() -> Result<Self, String> {
        let mut system_directory = [0u16; 32768];
        // SAFETY: writable UTF-16 buffer with its exact length.
        let len = unsafe {
            GetSystemDirectoryW(system_directory.as_mut_ptr(), system_directory.len() as u32)
        } as usize;
        if len == 0 || len >= system_directory.len() {
            return Err(format!(
                "locate Windows System32 for Npcap: {}",
                std::io::Error::last_os_error()
            ));
        }
        let path = PathBuf::from(OsString::from_wide(&system_directory[..len]))
            .join("Npcap")
            .join("wpcap.dll");
        // SAFETY: the absolute path comes from the OS, not the project or
        // current directory. Dependencies are restricted to that DLL's
        // directory and System32. No global DLL search path is changed.
        let library = unsafe {
            Library::load_with_flags(&path, LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_SYSTEM32)
        }.map_err(|e| format!(
            "Npcap is unavailable at {}: {e}; install the official Npcap Windows driver matching this process architecture. Simulation (nic=\"_sim\") does not require Npcap",
            path.display()
        ))?;
        // SAFETY: each signature below is the documented Npcap C ABI.
        // The owned library outlives every copied function pointer.
        unsafe {
            macro_rules! symbol {
                ($name:literal) => {
                    *library.get(concat!($name, "\0").as_bytes()).map_err(|e| {
                        format!(
                            "Npcap is missing {}: {e}; update the official Npcap installation",
                            $name
                        )
                    })?
                };
            }
            Ok(Self {
                create: symbol!("pcap_create"),
                set_snaplen: symbol!("pcap_set_snaplen"),
                set_promisc: symbol!("pcap_set_promisc"),
                set_timeout: symbol!("pcap_set_timeout"),
                set_immediate_mode: symbol!("pcap_set_immediate_mode"),
                activate: symbol!("pcap_activate"),
                setnonblock: symbol!("pcap_setnonblock"),
                datalink: symbol!("pcap_datalink"),
                compile: symbol!("pcap_compile"),
                setfilter: symbol!("pcap_setfilter"),
                freecode: symbol!("pcap_freecode"),
                sendpacket: symbol!("pcap_sendpacket"),
                next_ex: symbol!("pcap_next_ex"),
                geterr: symbol!("pcap_geterr"),
                close: symbol!("pcap_close"),
                _library: library,
            })
        }
    }
}

pub(super) struct Capture {
    handle: NonNull<c_void>,
    api: Api,
    nic: String,
    _lease: InterfaceLease,
}

// Only one master in this process may own a NIC. In particular, a driver
// call that outlives a shutdown deadline retains its lease: reconnecting
// cannot create a second sender while the old packet thread still exists.
struct InterfaceLease(String);
static OPEN_INTERFACES: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();

impl InterfaceLease {
    fn acquire(name: &str) -> Result<Self, String> {
        let mut interfaces = OPEN_INTERFACES
            .get_or_init(Mutex::default)
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        if !interfaces.insert(name.to_owned()) {
            return Err(format!(
                "EtherCAT NIC {name} is already owned by an active or not-yet-stopped packet transport"
            ));
        }
        Ok(Self(name.to_owned()))
    }
}

impl Drop for InterfaceLease {
    fn drop(&mut self) {
        OPEN_INTERFACES
            .get_or_init(Mutex::default)
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&self.0);
    }
}

// SAFETY: a capture is moved to its sole packet thread before first use.
// libpcap permits separate handles on separate threads; this type is not
// Sync and no capture call or borrowed packet crosses threads concurrently.
unsafe impl Send for Capture {}

impl Capture {
    pub(super) fn open(nic: &str) -> Result<Self, String> {
        let api = Api::load()?;
        let name = resolve_interface(nic)?;
        let lease = InterfaceLease::acquire(&name)?;
        let device =
            CString::new(name.as_str()).map_err(|_| "Npcap NIC contains NUL".to_string())?;
        let mut error = [0 as c_char; 256];
        // SAFETY: NUL-terminated device and PCAP_ERRBUF_SIZE output buffer.
        let raw = unsafe { (api.create)(device.as_ptr(), error.as_mut_ptr()) };
        let handle = NonNull::new(raw)
            .ok_or_else(|| format!("Npcap create {name}: {}", error_buffer(&error)))?;
        let capture = Self {
            handle,
            api,
            nic: name,
            _lease: lease,
        };
        // SAFETY: unique live pcap handle; setters precede activation.
        unsafe {
            for (setting, result) in [
                ("snaplen", (capture.api.set_snaplen)(raw, 65536)),
                ("promiscuous capture", (capture.api.set_promisc)(raw, 1)),
                ("read timeout", (capture.api.set_timeout)(raw, 1)),
                ("immediate mode", (capture.api.set_immediate_mode)(raw, 1)),
            ] {
                if result != 0 {
                    return Err(capture.error(setting, result));
                }
            }
            let result = (capture.api.activate)(raw);
            if result < 0 {
                return Err(format!(
                    "{}; verify this NIC exists, the Npcap service is running, and capture permissions allow this account (admin-only Npcap requires an elevated runtime)",
                    capture.error("activate", result)
                ));
            }
            if result > 0 {
                // A degraded capture mode cannot be assumed safe for PDOs.
                return Err(
                    capture.error("activation warning (capture requirements not met)", result)
                );
            }
            if (capture.api.datalink)(raw) != 1 {
                // DLT_EN10MB
                return Err(format!(
                    "Npcap NIC {} is not Ethernet; loopback and wireless capture modes cannot carry this EtherCAT transport",
                    capture.nic
                ));
            }
            let result = (capture.api.setnonblock)(raw, 1, error.as_mut_ptr());
            if result != 0 {
                return Err(format!(
                    "Npcap set nonblocking on {}: {}",
                    capture.nic,
                    error_buffer(&error)
                ));
            }
            let mut filter = BpfProgram {
                length: 0,
                instructions: ptr::null_mut(),
            };
            let result = (capture.api.compile)(
                raw,
                &mut filter,
                c"ether proto 0x88a4".as_ptr(),
                1,
                u32::MAX,
            );
            if result != 0 {
                return Err(capture.error("compile EtherCAT packet filter", result));
            }
            let result = (capture.api.setfilter)(raw, &mut filter);
            (capture.api.freecode)(&mut filter);
            if result != 0 {
                return Err(capture.error("install EtherCAT packet filter", result));
            }
        }
        tracing::info!(nic = %capture.nic, "Npcap Ethernet packet transport opened");
        Ok(capture)
    }

    fn error(&self, operation: &str, status: c_int) -> String {
        // SAFETY: live handle; pcap owns this NUL-terminated error string.
        let error = unsafe { (self.api.geterr)(self.handle.as_ptr()) };
        let message = if error.is_null() {
            "no driver details".into()
        } else {
            unsafe { CStr::from_ptr(error) }.to_string_lossy()
        };
        format!(
            "Npcap {operation} on {} (status {status}): {message}",
            self.nic
        )
    }
}

impl PacketIo for Capture {
    fn send(&mut self, bytes: &[u8]) -> Result<usize, String> {
        let len = c_int::try_from(bytes.len())
            .map_err(|_| "Npcap send frame exceeds C int".to_string())?;
        // SAFETY: live unique handle and valid bytes for len bytes.
        let result = unsafe { (self.api.sendpacket)(self.handle.as_ptr(), bytes.as_ptr(), len) };
        if result != 0 {
            Err(self.error("sendpacket", result))
        } else {
            Ok(bytes.len())
        }
    }

    fn receive(&mut self) -> Result<Option<&[u8]>, String> {
        let mut header = ptr::null();
        let mut data = ptr::null();
        // SAFETY: valid handle, output pointers; nonblocking was required
        // at open. Returned data is borrowed only until the next call.
        let result = unsafe { (self.api.next_ex)(self.handle.as_ptr(), &mut header, &mut data) };
        match result {
            0 => Ok(None),
            1 if !header.is_null() && !data.is_null() => {
                let header = unsafe { &*header };
                if header.captured_len != header.original_len || header.captured_len > 65536 {
                    return Err(format!(
                        "Npcap returned a truncated or oversized frame on {}",
                        self.nic
                    ));
                }
                Ok(Some(unsafe {
                    std::slice::from_raw_parts(data, header.captured_len as usize)
                }))
            }
            _ => Err(self.error("receive", result)),
        }
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        // SAFETY: sole owner closes the handle before Api drops its DLL.
        unsafe { (self.api.close)(self.handle.as_ptr()) };
    }
}

fn error_buffer(bytes: &[c_char]) -> String {
    let bytes: Vec<u8> = bytes
        .iter()
        .take_while(|b| **b != 0)
        .map(|b| *b as u8)
        .collect();
    String::from_utf8_lossy(&bytes).into_owned()
}

fn resolve_interface(nic: &str) -> Result<String, String> {
    if let Some(name) = npcap_name(nic) {
        return Ok(name);
    }
    if nic.is_empty() || nic.contains('\0') || nic.starts_with('\\') || nic.contains("://") {
        return Err("Windows EtherCAT NIC must be a local Ethernet alias, {GUID}, or \\Device\\NPF_{GUID}; loopback/remote capture is unsupported".into());
    }
    let alias: Vec<u16> = nic.encode_utf16().chain(Some(0)).collect();
    let mut luid = NET_LUID_LH { Value: 0 };
    let mut guid = windows_sys::core::GUID::default();
    // SAFETY: terminated UTF-16 alias and writable Windows ABI outputs.
    let result = unsafe { ConvertInterfaceAliasToLuid(alias.as_ptr(), &mut luid) };
    if result != 0 {
        return Err(format!(
            "Windows EtherCAT NIC alias {nic:?} was not found (Windows error {result}); use an exact /system NIC name or Npcap \\Device\\NPF_{{GUID}}"
        ));
    }
    let result = unsafe { ConvertInterfaceLuidToGuid(&luid, &mut guid) };
    if result != 0 {
        return Err(format!(
            "resolve Windows NIC {nic:?} GUID: Windows error {result}"
        ));
    }
    Ok(format!(
        r"\Device\NPF_{{{:08X}-{:04X}-{:04X}-{:02X}{:02X}-{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}}}",
        guid.data1,
        guid.data2,
        guid.data3,
        guid.data4[0],
        guid.data4[1],
        guid.data4[2],
        guid.data4[3],
        guid.data4[4],
        guid.data4[5],
        guid.data4[6],
        guid.data4[7]
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pcap_header_matches_windows_sdk_abi() {
        assert_eq!(std::mem::size_of::<PacketHeader>(), 16);
        assert_eq!(std::mem::offset_of!(PacketHeader, captured_len), 8);
        assert_eq!(
            std::mem::size_of::<BpfProgram>(),
            if cfg!(target_pointer_width = "64") {
                16
            } else {
                8
            }
        );
    }

    #[test]
    fn invalid_interface_is_rejected_without_loading_or_opening_npcap() {
        for nic in [
            "",
            r"\Device\NPF_Loopback",
            "rpcap://example/device",
            "bad\0alias",
        ] {
            assert!(resolve_interface(nic).is_err());
        }
    }

    #[test]
    fn interface_lease_prevents_overlap_until_capture_owner_releases_it() {
        let name = "unit-test-interface-lease";
        let first = InterfaceLease::acquire(name).unwrap();
        assert!(InterfaceLease::acquire(name).is_err());
        drop(first);
        assert!(InterfaceLease::acquire(name).is_ok());
    }
}
