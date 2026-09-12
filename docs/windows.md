# Native Windows engineering station

The target is **Windows 11 x64 (`x86_64-pc-windows-msvc`)**. The server,
CLI, language-server sidecar and runtime build as native `.exe` files;
WSL is not needed. A successful build alone is not acceptance: run the
native gate and installed-artifact workflow before reporting verification.
See the [recorded native validation](windows-validation.md) for the tested
environment, results and remaining hardware boundaries.

| Capability | Windows scope |
| --- | --- |
| Web IDE, ST/LD/FBD/SFC compilation, language server | Native engineering station |
| Simulation, monitor, alarms, history, HMI | Native server/runtime; scenario evidence required |
| Modbus TCP, OPC UA, MQTT | Native implementations; physical integration requires a separate test |
| Modbus RTU | COM ports; omit Linux-only `transport.rs485` direction control and use an automatic-direction adapter |
| EtherCAT | EtherCrab/Npcap native transport; `_sim` remains available; software checks passed, physical bus acceptance pending |
| CANopen | gs_usb/WinUSB native transport; software checks passed, physical bus acceptance pending; Linux SocketCAN is a separate backend |
| Linux edge deploy | Windows OpenSSH client and `tar.exe`; the runtime must match the Linux edge architecture |
| Windows runtime lifecycle | Foreground process; no Windows service installer, hard real-time claim or automatic restart guarantee |

Windows runtime `GET /system` enumerates native network interfaces and
COM ports. Carrier means connected media, not proof that a field device
is reachable. See [Windows CANopen and EtherCAT](windows-can-ethercat.md)
for external drivers, licensing and the separate bus acceptance record.
No physical CAN or EtherCAT device has been validated by this work.

## Install a prebuilt package

Extract `ia2-windows-x64.zip` into a writable directory. Open **Windows
PowerShell** there and run:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\scripts\install-skill.ps1
```

The package includes four binaries, built web assets, the FB library and
complete skill references/checklists. The release build uses a static
Microsoft C runtime, so installation needs no Rust, Node, pnpm or VC++
Redistributable installer. Physical EtherCAT additionally requires the
separately installed Npcap driver; that driver and its installer are not
included in the IA2 package. `windows-package.json` records the source
commit, dirty status, target and packaging time. The adjacent `.sha256`
file checks transfer integrity, not publisher identity; the ZIP is unsigned.

Default per-user layout:

```text
%LOCALAPPDATA%\IA2\
  IA2.ps1
  bin\cs.exe
  bin\ia2-server.exe
  bin\lsp-launcher.exe
  bin\ia2-runtime.exe
  web\
  library\
%USERPROFILE%\.claude\skills\industrial-automation-skill\
%USERPROFILE%\.agents\skills\industrial-automation-skill\
```

The installer creates **IA2 IDE** and **IA2 Terminal** in your Start menu.
It needs no elevation or symlinks, does not change machine/user PATH or
shell profiles, and does not add firewall rules or start a service. IA2
Terminal puts `bin` on PATH only for that terminal and its child processes.
Start a coding agent there if it needs `cs` by name, or configure it to
use the absolute `cs.exe` path. Restart the agent to discover the skills.

Options: `-InstallRoot 'D:\Tools with spaces\IA2'` changes the application
directory; `-ClaudeDir` / `-AgentsDir` change agent roots; `-NoShortcuts`
omits Start menu entries. `-SkillOnly` installs just the skills with no
build or network step.

Updates stage replacements before swapping each owned directory. The
installer refuses unmanaged destinations, junction/symlink paths, and
destinations overlapping sources or each other.
Stop installed IA2 processes before upgrading. Do not store projects in
application or skill directories: these contain replaceable shipped
assets. Normal projects remain in the user's Documents `IA2` directory,
and preferences remain in the OS user configuration directory. Removing
the application, two installed skills and Start menu entries uninstalls
these assets while preserving project data.

## Start and stop

Open **IA2 IDE**, or run:

```powershell
& "$env:LOCALAPPDATA\IA2\IA2.ps1"
# Custom port:
& "$env:LOCALAPPDATA\IA2\IA2.ps1" -Port 3005
```

The launcher keeps the server in a visible console and passes the installed
web/library paths explicitly. After the server prints its listening
address, open `http://127.0.0.1:3001` (or the selected port) in a browser.
A printed URL alone is not proof of startup; errors remain in the console.
A port already in use causes startup to fail. **Ctrl+C in that console**
stops the process. Closing the browser does not stop PLC execution.
This is development process control, not an emergency stop.

In another PowerShell window:

```powershell
$cs = "$env:LOCALAPPDATA\IA2\bin\cs.exe"
& $cs api GET /health
& $cs ls projects
& $cs ls library
```

`LSP_LAUNCHER` overrides the language-server executable; by default the
server finds `lsp-launcher.exe` next to itself. The launcher binds loopback.
For a custom port, pass `--server http://127.0.0.1:3005` to the CLI.
Project open accepts drive paths, UNC paths and `~\Documents\...`; quote
paths containing spaces. File names reject Windows reserved names and
invalid trailing dots/spaces so projects remain portable.

Library resolution order: `--library-dir`, `IA2_LIBRARY_DIR`, `./library`,
the executable's adjacent `../library`, then the legacy
`~/.local/share/ia2/library` location.

To run a Windows runtime development process, call `ia2-runtime.exe --help`
and launch it in a separate visible console with its documented project
arguments. Ctrl+C stops it. Linux service management and hardware timing
evidence do not transfer to this process.

## Build from source

Prerequisites: Git with the `vendor/ironplc` submodule, Rust's MSVC x64
toolchain, Visual Studio C++ Build Tools (Desktop development with C++ and
Windows SDK), and Node/pnpm matching root `package.json`. Use an x64 Native
Tools terminal if the linker or SDK cannot be found. GNU/MinGW is outside
this target. The native gate is validated with Rust **1.95.0**, matching
the existing macOS gate; newer Clippy releases can add warnings in the
locked upstream compiler. Select the validated toolchain for this clone:

```powershell
git clone --recursive https://github.com/supcon-international/ia2
Set-Location ia2
rustup toolchain install 1.95.0 --profile minimal --component clippy --component rustfmt
rustup override set 1.95.0
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\scripts\install-skill.ps1
```

The source installer initializes submodules, installs locked web
dependencies, runs server tests to generate TypeScript types, builds all
four release binaries and the web UI. Installation does not replace the
full quality gate. `-SkipBuild` consumes prebuilt
`target\x86_64-pc-windows-msvc\release` and `apps\web\dist` artifacts,
and fails if any required artifact is absent.

`scripts/build-windows.ps1` builds an explicit x64 MSVC release with
`-C target-feature=+crt-static`, preserving caller flags and restoring
them afterward. It leaves global Cargo config and ordinary debug builds
unchanged. Use this helper for distributable releases; a plain host
`cargo build --release` may depend on the VC++ runtime installed by the
development tools.

With Git symlinks disabled, `.agents\skills\industrial-automation-skill`
is a text placeholder, not a discoverable directory. Read canonical
`.claude\skills\industrial-automation-skill\SKILL.md` directly for repo
work, or use `-SkillOnly` to install real user copies. Git symlinks need
Developer Mode or appropriate privileges; the installer needs neither.

## Native acceptance and packaging

```powershell
pnpm install --frozen-lockfile
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\scripts\check-windows.ps1
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\scripts\package-windows.ps1 -SkipBuild
```

The gate checks skill metadata/discovery and installation (upgrades,
spaces in paths and refusal guards), formatting, workspace Clippy with
zero warnings, workspace tests, four release binaries and web build/tests.
It builds `server.exe` before CLI simulation tests: missing executables
are failures, not successfully skipped business tests. `-AdaptationOnly`
runs just the offline installer checks. Linux Bash deployment execution
tests still need Unix coverage; Windows exercises real local archive
operations and script generation. The gate also runs
`scripts/test-windows-runtime.ps1` against the static-CRT release: a
device-free scenario in a Chinese path with spaces, real system inventory,
pause/write/step/resume, and confirmed clean process stop. Its JSON/log
evidence stays under `target/windows-runtime-smoke/`. That script may be
run separately with `-RuntimePath` to select an executable.

Without `-SkipBuild`, `package-windows.ps1` also builds. It verifies all
four binaries have x64 PE headers and uses .NET ZIP creation to include
the hidden `.claude` tree. `-OutputPath` accepts paths containing spaces.
Default output: `dist\ia2-windows-x64.zip` and its SHA256 file.

Verify the extracted release separately from the source gate:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\scripts\test-windows-install.ps1 -PackagePath .\dist\ia2-windows-x64.zip
```

This checks the ZIP hash and hidden skill files, two identical installations
in isolated Chinese paths with spaces, unchanged user data/PATH, installed
CLI/runtime execution, server startup outside the repository, bundled web
assets, automatic library discovery and a real LSP initialize exchange.
It runs with only Windows system directories on PATH, stops its server and
sidecar, and saves results/logs under `target/windows-install-smoke/`.
The test does not install into the default user location or create shortcuts.

Before distribution, extract the ZIP into a different path containing
spaces, install it, and verify a real project create/open, compilation,
language-server connection, scenario, monitor updates, library import,
HMI load and Ctrl+C stop. Check browser errors and data persistence after
restart. Source tests do not replace installed-artifact checks. Report
physical serial/network integration and Linux edge deployment with their
own evidence.

## Deploy from Windows to Linux

Windows is the engineering host; `cs deploy` still provisions Linux using
SSH, Bash and systemd. Install Windows OpenSSH Client if `ssh.exe` is absent,
and ensure `tar.exe` is on the server's PATH. Put edge aliases in
`%USERPROFILE%\.ssh\config` and verify key-based
`ssh -o BatchMode=yes <alias>` first.

Installed `ia2-runtime.exe` is never a Linux payload. Supply a Linux ELF
matching the target architecture through `IA2_RUNTIME_BIN`, or reuse the
runtime already provisioned there. See [edge deployment](edge-deploy.md)
for Linux provisioning and failure reporting. Local simulation alone is
not hardware readiness evidence.

## Implementation references

- [Microsoft: Rust prerequisites on Windows](https://learn.microsoft.com/en-us/windows/dev-environment/rust/setup)
- [Rust: static and dynamic C runtimes](https://doc.rust-lang.org/reference/linkage.html#static-and-dynamic-c-runtimes)
- [Microsoft: Windows OpenSSH setup](https://learn.microsoft.com/en-us/windows-server/administration/openssh/openssh_install_firstuse)
- [Microsoft: Start-Process quoting](https://learn.microsoft.com/en-us/powershell/module/microsoft.powershell.management/start-process?view=powershell-5.1)
- [Microsoft: Compress-Archive omits hidden files](https://learn.microsoft.com/en-us/powershell/module/microsoft.powershell.archive/compress-archive?view=powershell-5.1)
