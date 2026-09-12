# Windows validation — 2026-09-12

Native Windows engineering-station work on `codex/windows`, based on
`474364d`. The pinned `vendor/ironplc` submodule is unchanged at
`72d6ac4272958bc61215c8fe06cb8ac00af3d416`.

This record predates the native gs_usb and Npcap transport integration.
Its passes do not validate those transports. See the separate
[Windows CANopen/EtherCAT acceptance record](windows-can-ethercat.md)
for their current status and hardware tests still pending.

## Environment

- Windows 11 Home, x64, build 10.0.26200; Intel Core i9-13980HX, 64 GB RAM.
- Rust/Cargo/Clippy 1.95.0; MSVC 14.29.30133 and Windows SDK 10.0.19041.0.
- Node 24.21.0, pnpm 11.0.9; Edge 152 for real browser acceptance.
- Native MSVC executables, without WSL. All PLC exercises use simulated
  adapters or loopback services; no physical device was controlled.

## Results

| Check | Result |
| --- | --- |
| Native Windows adaptation, formatting and workspace Clippy | Pass, zero Clippy warnings |
| Native Windows workspace Rust tests | 3,563 passed, 0 failed, 51 existing upstream ignores |
| macOS adaptation, formatting and workspace Clippy | Pass, zero Clippy warnings |
| macOS workspace Rust tests | 3,566 passed, 0 failed, 51 existing upstream ignores |
| macOS release build, all four binaries | Pass |
| Web tests on Windows and macOS | 172 passed across 15 files on each host |
| Web production build on Windows and macOS | Pass |
| Windows Edge source-build acceptance | 10 checks passed; zero HTTP, console or page errors |
| Windows static-CRT release, all four binaries | Pass |
| Windows release runtime acceptance | Pass: pause at scan 14, four steps to 18, resume to 27, level 2.0, stop exit 0 |
| Release console Ctrl+C | Runtime exits 0 after shutdown; server exits with Windows' default `0xC000013A`; both ports close |
| Release ZIP installation and upgrade | Pass: 138 installed files unchanged across two installs in Chinese paths with spaces; user data and PATH preserved |
| Default per-user installation and Start menu shortcuts | Pass |
| Installed release browser acceptance | 12 checks passed, including library import/datasheet and live HMI values |
| Installed release restart acceptance | 4 checks passed: project, ST, imported library and HMI hashes unchanged; GUI reopens correctly |

Windows excludes the two Unix Bash deployment-execution tests and the
Unix EtherCAT implementation tests, while adding native path tests.
The Unix deployment-execution tests passed on macOS. Windows exercised
its actual `tar.exe` with Unicode paths and multiple input directories.

Browser acceptance covers Chinese project names and paths containing
spaces, create/open, ST editing and saving, bad diagnostics and repair,
LSP initialization, compilation/run, changing Monitor values, stop,
project selection after creation and isolation when another window's
project closes. It found and drove fixes for HTTP project-name encoding,
empty diagnostic arrays and stale window selection.

The installed-release flow observed Monitor values changing from 17 to
25 and HMI values from 36 to 46. After stopping the test runtime and
restarting the installed launcher, the project reopened with identical
saved files and working editor, library datasheet and HMI views. These
two completed browser runs had no unexpected HTTP, console or page errors.
On the earlier first launch with no open project, the existing bootstrap
probe returned one expected `GET /api/project` 409, which the client
handles as an empty state; the browser also logged that failed resource.
There was no page exception. This existing empty-state behavior was not
changed solely to remove a console entry.

The Windows bridge regression suite passed all 125 tests. The original
5 ms watchdog negative controls failed before the Windows timer fix and
passed afterward. Actual 12 ms scan-stall injection still latches outputs
off. No watchdog thresholds, test periods or deadline accounting were
relaxed. This is scheduling correctness evidence, not a hard-real-time
guarantee.

## Evidence and reproduction

Run the commands in [Windows setup](windows.md). The native gate is
`scripts/check-windows.ps1`; installed-package acceptance is
`scripts/test-windows-install.ps1`. Runtime and installation scripts save
JSON results and logs under `target/windows-runtime-smoke/` and
`target/windows-install-smoke/`. Browser results, screenshots and console
exit evidence are retained with the development-machine validation logs.
The release ZIP manifest records its source commit and dirty status;
the adjacent SHA256 file verifies the delivered bytes.

Installed-package acceptance ran with only Windows system directories on
PATH. All four binaries executed; the server started outside the source
tree, served IDE/HMI assets, discovered 44 library blocks without an
explicit library path, and completed a real WebSocket LSP initialization.
The test server and language-server sidecar were stopped afterward.
PowerShell 5.1 log capture also verified that native stderr with exit 0
remains successful and exit 7 remains a failure; this caught and fixed
the installer's initial misclassification of runtime `--help` output.

## Not established by this validation

Physical Modbus RTU/TCP, OPC UA and MQTT device integration, deployment
from Windows to a real Linux edge, sustained timing under production
load, installer signing and Windows service lifecycle remain separate
work. Real EtherCAT and CANopen were unsupported by the Windows build
tested in this record. The subsequent native transport work has its own
acceptance record; it does not retroactively establish hardware results.
COM ports and network-interface records are native OS inventory, not
proof of device communication.
