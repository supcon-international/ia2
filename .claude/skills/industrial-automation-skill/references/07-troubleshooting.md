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

### "tasks.toml schedules N PROGRAMs but the project declares VAR_GLOBAL …"
**Cause:** multi-PROGRAM projects run one container per instance, so a `VAR_GLOBAL` can't be shared across them (each instance would get a private, diverging copy). This is the *only* multi-PROGRAM restriction — scheduling several PROGRAMs is otherwise supported, and they run round-robin.
**Fix:** move the shared state behind an I/O mapping or FUNCTION_BLOCK parameter, or reduce `tasks.programs` to a single PROGRAM. Both `cs run` and `cs project check` name the offending globals. See `01-mental-model.md` fact 2.

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

### The run stopped by itself — snapshots frozen, nothing obvious in the way
**Cause:** the program faulted. A VM trap (divide by zero, bad array index, …) in any scheduled instance stops the whole plant and zeroes outputs (failsafe), by design.
**Fix:** read the reason instead of re-running blind: `/api/runtime/status` reports `running: false` with `last_error` carrying the trap message (e.g. `VM trap in main_inst: DivideByZero`), and the SSE stream emitted `error` then `stopped` at the moment it died. On an edge, the runtime's own `/status` carries the same message in `fault`. Fix the arithmetic (guard divisors, clamp indices) and run again — the next `cs run` clears `last_error`.

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

- **Multi-PROGRAM runs are supported** — one container per scheduled instance, round-robin on one scan thread. The sole restriction is no `VAR_GLOBAL` shared across instances (rejected with a clear error). See `01-mental-model.md` fact 2.
- **One running program per server.** Hardware (Modbus/EtherCAT bus) can have one master. Starting a program stops the previous, across all projects.
- **No `AT %IX0.0` located variables.** Bind via `iomap.toml`, not IEC direct addressing.
- **Real EtherCAT is Linux-only** (`CAP_NET_RAW`). On macOS use `nic: "_sim"`.
- **RETAIN restores as i32** — wide types truncate.
- **No per-entry iomap/tasks/device-channel edits.** Whole-document get → edit → set.
- **WSTRING** is upstream-WIP; don't author WSTRING programs expecting them to run.
- **Server port for IA2.app is ephemeral.** Always discover; never hard-code `:3001` when the desktop app is the server.
