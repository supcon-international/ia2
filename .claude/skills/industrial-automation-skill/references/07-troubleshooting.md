# Troubleshooting + known limits

Each entry: the symptom you'll see, the cause, the fix.

## HTTP / CLI errors

### `409` on `cs device create NAME` (or edge create)
**Cause:** that device/edge already exists in the project.
**Fix:** `cs device list` to confirm; use `cs device set NAME --from -` to reconfigure instead of create.

### `422` on `cs iomap set` / `cs device set` / `cs tasks set`
**Cause:** wrong JSON shape. For iomap, almost always a missing `application` field. For device, a transport union missing its `kind`.
**Fix:** `cs iomap get` (or `device get`) to see the exact shape the server emits, edit *that*, set it back. Field names are snake_case.

### `404` on `cs pou save` / `cs device get` / any project-scoped call
**Cause:** the project isn't open on the server, or `--project NAME` names a project that isn't in the open set.
**Fix:** `cs project list`. If absent, `cs project open /abs/path`. If multiple are open and you omitted `--project`, the server used its LRU active fallback — name it explicitly.

### `400` "project 'X' is not open on this server"
**Cause:** you passed `--project X` but X isn't open.
**Fix:** `cs project open` it first, or pick a name from `cs project list`.

### CLI can't reach the server at all (`connection refused`)
**Cause:** wrong port. `cs` defaults to `:3001`; IA2.app binds an **ephemeral** port.
**Fix:** discover the real port (see `checklists/first-contact.md`). Pass it as `--server`.

### `cs` command works but the IDE shows nothing / a different project
**Cause:** the IDE window is scoped to a different `?project=` than the one your command hit.
**Fix:** the human's window shows project A; your `cs` hit project B (active fallback). Pass `--project A` to match what they're looking at, or have them switch the window's project picker.

## Run / compile errors

### "tasks.toml schedules N PROGRAMs … the runtime can only execute one PROGRAM per run"
**Cause:** the hard ironplc limit — codegen emits one PROGRAM per compilation. `cs run` (whole schedule) refuses 2+.
**Fix:** either reduce `tasks.programs` to one entry, or run a specific one ad-hoc: `cs run --program NAME`. There is no workaround that runs two PROGRAMs simultaneously today.

### `cs project check` fails with `P####`
**Cause:** an IEC source error.
**Fix:** `cs check --explain pous/FILE.st` (or `cs explain P####`) prints the full ironplc problem doc. Common ones: missing `;`, C-style `&&` instead of `AND`, bare number where a `T#…ms` time literal is expected, a second PROGRAM in the file. See `05-iec-61131.md` § quirks.

### "modbus connect failed" in the run log, but the run still starts
**Cause (TCP):** nothing listening at host:port. **(RTU):** the serial device doesn't exist / is busy / wrong permissions.
**Fix:** this is **non-fatal by design** — the device is skipped, the scan loop runs with that device's channels reading zero. For RTU on macOS, confirm the path with `ls /dev/cu.*`; the device often appears as `/dev/cu.usbserial-XXXX`. Permissions: the user (not root) usually owns USB-serial adapters on macOS; on Linux add the user to `dialout`.

### A variable in the Monitor pane won't change no matter what
**Cause:** it's force-pinned and someone forgot to `unforce`.
**Fix:** `cs runtime status --json` lists active forces. `cs runtime unforce NAME`. (This is why workflow recipe D always unforces at the end.)

### RETAIN value didn't persist across restart
**Cause:** either the variable isn't in a `VAR RETAIN` block, or the program was killed hard (not a clean stop) between the 5 s flush windows.
**Fix:** confirm the `VAR RETAIN` declaration. Note the flush cadence is 5 s + on clean stop — up to 5 s of change can be lost on an unclean kill. Also: values restore as i32, so LREAL/LINT/LWORD truncate (use DINT-class for retained counters).

## Overlay / session

### The takeover banner strobes (label changes every couple seconds)
**Cause:** you're running commands without a session — each is a transient heartbeat.
**Fix:** wrap the workflow in `cs agent run --label "…" -- bash -c '…'`. See `03-agent-sessions.md`.

### The banner is stuck on after your work finished
**Cause:** you used `cs agent enter` but never `cs agent leave` (or a script errored before leave).
**Fix:** `cs agent leave` (reads `IA2_AGENT_SESSION`), or the server's 30 s watchdog ends it, or the human clicks "End session". **Prefer `cs agent run`** — it always cleans up, even on Ctrl-C.

### The human clicked "End session" mid-workflow
**Cause:** they want control back.
**Fix:** stop. Don't reopen a session and keep going. Ask what they want.

## EtherCAT (real mode)

### `init_single_group: Timeout(Pdu)` and/or `failed to decode raw PDU data`
The NIC isn't dedicated to EtherCAT. On NetworkManager hosts the usual cause is NM still managing the port — its periodic DHCP/activation corrupts raw L2 frames and flaps the link. Set the interface `unmanaged` and confirm hardware offloads are off (see `docs/edge-deploy.md` → *Dedicate the NIC to EtherCAT*). Use a separate NIC from your SSH/management link.

### Bus discovers the SubDevice but never reaches OP; drive shows a comm fault (e.g. `EE`-class)
A servo can wedge in a half-configured state after an init that aborted mid-configuration. **Power-cycle the drive**, let its panel reach idle, then start the runtime once cleanly. Also confirm the drive has `dc_sync = "sync0"` (servo drives need DC SYNC0 to reach OP) — set it per-device, or per-SubDevice on a mixed servo + IO bus.

### `init sdo write … : object does not exist in the object directory`
An `init_sdo` entry targets a CoE object the drive doesn't have. A failed startup SDO aborts init by design (better than silently running a mis-configured drive) — drop or fix that entry. Example: the Inovance SV660N has no `0x6080` (max motor speed); cap torque via `0x6072` and limit speed in your program instead.

## Known limits (not bugs — design constraints today)

- **One PROGRAM per run.** ironplc codegen limitation. Multi-PROGRAM scheduling is rejected with a clear error.
- **One running program per server.** Hardware (Modbus/EtherCAT bus) can have one master. Starting a program stops the previous, across all projects.
- **No `AT %IX0.0` located variables.** Bind via `iomap.toml`, not IEC direct addressing.
- **Real EtherCAT is Linux-only** (`CAP_NET_RAW`). On macOS use `nic: "_sim"`.
- **RETAIN restores as i32** — wide types truncate.
- **No per-entry iomap/tasks/device-channel edits.** Whole-document get → edit → set.
- **WSTRING** is upstream-WIP; don't author WSTRING programs expecting them to run.
- **Server port for IA2.app is ephemeral.** Always discover; never hard-code `:3001` when the desktop app is the server.
