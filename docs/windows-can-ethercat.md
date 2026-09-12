# Windows CANopen and EtherCAT

The `codex/windows` branch adds native transports for real EtherCAT and
classic CANopen. Native software checks passed on 2026-09-12. No physical CAN or
EtherCAT device has been connected for this work: successful compilation,
simulation or device enumeration is not evidence of bus communication.
Ordinary Windows scheduling and packet/USB drivers do not make IA2 a
hard-real-time controller.

## EtherCAT dependencies

The protocol stack is EtherCrab 0.7.1, licensed under MIT or Apache-2.0.
Windows raw Ethernet exchange requires a separately installed Npcap
runtime. These are different components: EtherCrab is open source;
[Npcap's license](https://github.com/nmap/npcap/blob/master/LICENSE)
explicitly says Npcap is not open source. The free Npcap edition generally
allows five installations and forbids redistribution. Its unlimited-use
exceptions for particular applications do not cover IA2. Organizations
needing more installations or redistribution rights must use the
appropriate [Npcap OEM license](https://npcap.com/oem/).

Download the signed runtime installer from the
[official Npcap download page](https://npcap.com/#download), review its
license, and install it separately when preparing an EtherCAT bench.
The IA2 ZIP does not include or install Npcap, its DLLs or its kernel
driver. IA2's per-user installation does not grant permission to install
or reconfigure system drivers.

The current upstream downloads are Npcap 1.88 and SDK 1.16, checked on
2026-09-12. The SDK is a build-time archive containing import libraries;
it is not a driver installer. IA2 uses EtherCrab's protocol core with its
own explicitly loaded Windows packet transport, so the normal build does
not require the Npcap SDK. A separate native compilation probe verified
this dependency choice; the complete IA2 native build also passed without
the SDK. Do not copy SDK files into the release ZIP. See the
[Npcap developer guide](https://npcap.com/guide/npcap-devguide.html).

Npcap DLLs normally reside below `%SystemRoot%\System32\Npcap`.
The optional WinPcap-compatible installation also places DLLs in the
system directory and replaces an existing WinPcap installation. IA2
must not require that compatibility option or a global PATH change.
If Npcap was installed with administrator-only access, opening a capture
handle may require elevation. Missing DLLs, a stopped/unavailable driver,
denied access and an unknown interface must produce a connection error;
they must not prevent the IDE or simulation from starting or silently
select simulation. The acceptance table separates verified missing-driver
behavior from cases still requiring an installed driver. See the
[Npcap installation options](https://npcap.com/guide/npcap-users-guide.html).

Use a dedicated wired Ethernet interface for a physical EtherCAT bench.
The NIC selector accepts a local Windows interface alias (including
Unicode names), `{GUID}`, or `\Device\NPF_{GUID}`. Npcap loopback and
remote-capture selectors are rejected.
Do not substitute a Wi-Fi, VPN or loopback capture interface, or use the
interface carrying the only remote-management connection. A Windows
network alias or connected-media flag alone does not prove an EtherCAT
device is present. Driver loading and interface selection must succeed
before discovery; a positive discovery still does not prove PDO mapping,
watchdog behavior or timing under load.

## CANopen through gs_usb

The Windows USB backend uses `nusb` 0.2.7 (MIT or Apache-2.0) and the
Windows WinUSB driver. It does not need a libusb runtime DLL or a vendor
CAN SDK. The adapter must run compatible gs_usb firmware, such as
[candleLight](https://github.com/candle-usb/candleLight_fw); a USB-CAN
product name alone is insufficient, and SLCAN serial firmware is a
different protocol.

The CANopen device configuration selects the adapter using this template:

```toml
interface = "gs_usb:<VIDhex>:<PIDhex>:<serial>:<channel>"
# bitrate is required and must match the physical CAN network.
```

Replace every placeholder with the attached adapter's identity and
channel. VID and PID are hexadecimal. The serial field may be empty
only when matching that VID/PID identifies exactly one attached device;
multiple matches must fail rather than select an arbitrary adapter.
The backend applies the configured bitrate. This integration uses
classic CAN with 11-bit CANopen identifiers; CAN FD and SLCAN are outside
its scope. Extended and remote-request frames must not be interpreted
as ordinary CANopen data.

This version owns the entire USB adapter exclusively: configure one
CANopen node per adapter, including adapters exposing multiple channels.
Shared-bus receive fanout is not implemented. The existing CANopen layer
supports NMT start, heartbeat consumption, expedited SDO values up to four
bytes, and the predefined TPDO/RPDO connection set with an already
configured PDO mapping. Segmented/block SDO, automatic PDO remapping,
CANopen FD and a general-purpose raw-CAN API are not added by this change.

Opening USB or receiving a transmit echo does not mark the node healthy.
IA2 first requires a valid heartbeat, a decodable configured TPDO or a
matching SDO reply from the selected node. Enable the node's heartbeat
producer and a matching IA2 heartbeat timeout for ongoing control health.
A USB send completion confirms delivery to USB, not a CAN ACK or an
actuator movement. USB faults, receive overflow and CAN error frames
close this transport and mark it unhealthy; fix the cause and restart
the runtime to reopen it. Automatic recovery after these faults is not
implemented for gs_usb. Configured failsafe writes remain attempts that
can fail when the physical connection is unavailable.

The USB device/interface must be bound to WinUSB. Firmware with the
appropriate Microsoft OS descriptors can request automatic binding;
otherwise driver binding is a separate, explicit device-setup action.
Do not replace drivers for unrelated USB interfaces. Device-not-found,
ambiguous selection, unsupported firmware/channel, claim failure and
USB disconnection must be reported as failures. The existing Linux
SocketCAN transport remains a separate option. See
[nusb Windows support](https://docs.rs/nusb/0.2.7/nusb/#windows) and
[Microsoft's WinUSB installation mechanism](https://learn.microsoft.com/en-us/windows-hardware/drivers/usbcon/automatic-installation-of-winusb).

## Acceptance record

The earlier [Windows engineering-station validation](windows-validation.md)
predates these transports. Keep its results separate from this work.

| Check | Current result | Evidence needed |
| --- | --- | --- |
| Native Windows build, Clippy, workspace tests and simulation | PASS (2026-09-12) | `scripts/check-windows.ps1`: 3,598 Rust tests passed, 0 failed, 51 ignored; 172 web tests passed; four static-CRT release binaries built; runtime and fieldbus negative tests passed |
| Extracted package without Npcap | PASS (2026-09-12) | PE checks reject startup imports of wpcap/Packet and dynamic CRT DLLs; with system-only PATH, CLI/version and server/help start, runtime simulation passes, and real EtherCAT configuration returns an actionable dependency error |
| Npcap loading and interface selection | NOT RUN | Actual driver version, selected interface and successful loading; no physical discovery required for this check |
| Missing gs_usb adapter and malformed selector | PASS, no hardware attached | Native runtime reports both failures, disconnected discovery and unhealthy devices; no simulation fallback |
| Ambiguous gs_usb selection and invalid bitrate | PASS, unit tests only | Uniqueness and bitrate/firmware-limit tests pass; no multi-adapter or physical bitrate measurement |
| Physical EtherCAT discovery and cyclic PDO exchange | BENCH PENDING | Identity readback, correct mapping, observed input/output exchange and working counters |
| Physical CANopen exchange | BENCH PENDING | Adapter/firmware identity, applied bitrate, node identity, SDO/PDO exchange and heartbeat behavior |
| Disconnect, reconnect, timeout and stop | BENCH PENDING | Confirmed output behavior and released bus/USB handles after failure and shutdown |
| Sustained timing | BENCH PENDING | Measured scan/cycle latency and missed deadlines under representative load; no hard-real-time claim |

Run offline tests and simulations first. Physical output tests require a
separate bench plan with identified devices, a safe output state and an
abort condition. Record the exact commit, package hash, driver/firmware
versions and observed results; preserve **NOT RUN** and **BENCH PENDING**
when hardware is absent.

The standalone `scripts/test-windows-fieldbus.ps1` exercises only malformed
selectors, a reserved USB VID/PID with a nonexistent serial, and the zero
NIC GUID in an isolated copy of `sim_smoke`. Pass `-RuntimePath` to select
the exact executable and `-ArtifactsDirectory` for an empty evidence
directory. `-RequireNpcapAbsent` checks that the runtime DLLs are absent
and that EtherCAT returns the dependency error while HTTP and scanning
continue. It verifies disconnected discovery/health and a bounded clean
stop; it does not test real packet exchange or install drivers.

The native gate now runs this fieldbus test against the release executable.
It exposed and fixed a Windows startup stack overflow before the missing
Npcap error could be reported. Large EtherCrab initialization/state-change
futures and groups are boxed, with a 16 MiB Windows worker stack reservation
for upstream by-value temporaries. This adds no allocation to the cyclic
exchange itself. Regressions also exercise actual EtherCrab initialization
and state changes at the configured 128-subdevice/4096-byte capacity through
an empty fake bus, plus cancellation after the first real PDU is sent to
the fake transport and confirmation that the pump and storage are released.
These are software tests, not 128 physical subdevices or hardware timing
measurements. The macOS full quality gate and final EtherCAT crate's 103
tests also passed; the Windows crate includes three additional native tests.

Package acceptance ran on Windows 11 Home x64 build 26200, PowerShell 5.1,
with Rust 1.95.0 used for the native build. No Npcap SDK/runtime or physical
CAN/EtherCAT device was present. The extracted runtime's SHA256 was
`3719c0b6b444bdf43f7e65e9d81d69db2c6ba0bf82fd712fd61aa983d83f82a7`.
Its four negative fixtures all reported `connected=false`; `/health`
reported `fieldbus_healthy=false`, scans advanced from 3 to 14 during the
check, and `/stop` exited with code 0 without forced cleanup. The separate
simulation smoke also passed pause, step, resume and clean shutdown.
The package manifest records its source revision; the adjacent ZIP hash
identifies the archive. Npcap load success, physical discovery, actual
CAN ACK/SDO/PDO exchange and output behavior remain untested.
