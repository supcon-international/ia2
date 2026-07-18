# IA2 HTTP API

**Audience:** AI agents (Claude Code, Cursor, Codex) and humans curling for diagnosis.

**Address:** `http://127.0.0.1:3001` (the IDE backend). The edge runtime binary
serves a smaller subset on its own port — see `docs/edge-deploy.md`.

**Auth:** none (localhost-only). Remote access via SSH port-forward.

**Conventions:**
- Resources are plural nouns under `/api/<resource>`.
- Names with `/` (folder-nested POUs, devices, edges) are URL-encoded — the
  `%2F`-encoded form decodes back to `/` inside the path param. E.g.,
  `GET /api/pous/pid_loops%2Ftemperature`.
- All bodies are JSON unless noted. POU sources are `text/plain`.
- Errors are HTTP status + a human-readable body. 4xx for client errors,
  5xx for server bugs.
- Generated TypeScript types live under `apps/web/src/types/generated/` and
  are the source of truth for request/response shapes.

---

## Health & lifecycle

| Method | Path | Purpose | Notes |
|---|---|---|---|
| `GET` | `/health` | Liveness. Returns `HealthStatus`. | Convenience root path |
| `GET` | `/api/health` | Same as `/health` under the `/api` namespace | For agents that scope to `/api` |
| `GET` | `/api/projects` | List discoverable projects. Returns `ProjectListing[]`. | |
| `POST` | `/api/projects` | Create a new project. Body: `CreateProjectRequest`. | |
| `POST` | `/api/projects/open` | Open an existing project. Body: `OpenProjectRequest { path }`. | |
| `POST` | `/api/projects/close` | Close the currently-open project. | |
| `GET` | `/api/projects/open-list` | Every project the server currently has open + which is the active fallback. Returns `OpenProjectsList`. | Multi-window IDE picker |
| `GET` | `/api/fs/browse?path=` | List the sub-directories of `path` (default `~/Documents/IA2`) for the Open-project folder picker — directories only, dotfiles hidden, each flagged `is_project` (has a `project.toml`). Returns `FsListing`. | A browser has no native OS folder dialog |
| `GET` | `/api/project` | Full project tree (applications, devices, edges, iomap, tasks, folder lists). Returns `ProjectTree` or `null` when no project is open. | |
| `POST` | `/api/project/migrate-tasks` | One-shot migrate inline-CONFIGURATION blocks in POU files into `tasks.toml`. Idempotent. Returns `MigrationResponse`. | Legacy projects only |
| `POST` | `/api/project/validate` | Run `compile_project` and return diagnostics without spawning. Returns `Vec<CheckDiagnostic>` (empty = ok). | Pre-flight check before Run/Deploy |

## POUs

A POU is one IEC declaration (PROGRAM / FUNCTION_BLOCK / FUNCTION). A
single `.st` file may declare multiple POUs; the file is the unit on
disk, and the tree (in `/api/project`) shows each declaration as its
own node. The URL identifier in these routes is the **file path** —
slash-separated under `pous/`, no `.st` extension.

| Method | Path | Purpose |
|---|---|---|
| `POST` | `/api/pous` | Create a POU file. Body: `CreatePouRequest { path, type, language }`. `type` is `program` / `function_block` / `function`; `language` currently must be `st`. |
| `POST` | `/api/pous/folders` | Create a folder under `pous/`. Body: `CreateFolderRequest { path }`. |
| `DELETE` | `/api/pous/folders/{path}` | Delete an empty folder. |
| `GET` | `/api/pous/{path}` | Read a POU file. Returns `Pou { path, source, declarations: PouDecl[] }`. |
| `PUT` | `/api/pous/{path}` | Write POU source. Body is raw `text/plain`. |
| `DELETE` | `/api/pous/{path}` | Delete a POU file (and every declaration inside it). |
| `GET` | `/api/pous/{path}/variables` | Variables declared in the file. Returns `VariableInfo[]`. |

## Libraries & device catalog

First-class FB libraries (vendored into `pous/lib/<name>/`) and the
read-only device-template catalog used to pre-fill devices from a bus scan.

| Method | Path | Purpose |
|---|---|---|
| `GET` | `/api/hmi` | List the project's HMI screens: `HmiListEntry[] { path, title, level }`. |
| `POST` | `/api/hmi` | Create an empty screen. Body `{ path, title? }`; returns the fresh `HmiDoc`. Emits SSE `hmi` mutation. |
| `GET` | `/api/hmi/{path}` | Read one screen as `HmiDoc` (slug percent-encoded into one segment, like `/api/pous/{path}`). |
| `PUT` | `/api/hmi/{path}` | Replace the whole document (editor saves). Rejects structural errors; returns remaining warnings as `HmiIssue[]`. Emits SSE `hmi` mutation with empty `touched`. |
| `DELETE` | `/api/hmi/{path}` | Delete the screen. Emits SSE `hmi` mutation (`hmi_deleted`). |
| `POST` | `/api/hmi/{path}/ops` | THE incremental authoring surface: body `{ ops: HmiOp[] }` (`add_node` / `update_node` / `remove_node` / `set_meta`), applied atomically. Returns `{ touched, issues }`; the SSE `hmi` mutation carries the same `touched` node ids so every open canvas spawn-animates exactly the elements this batch placed. |
| `GET` | `/api/hmi/{path}/check` | Structural validation plus variable-existence warnings against the project's POUs. Returns `HmiIssue[]`. |
| `POST` | `/api/hmi/{path}/generate` | Deterministic first-pass screen from project truth (alarmbar, per-POU sections, BOOL→indicator, numeric→value, `*_sp`→setpoint input, one trend). Body `{ force?, title? }`; 409 if the screen exists and `force` is absent. Returns the generated `HmiDoc`. |
| `GET` | `/api/hmi-symbols` | The built-in symbol catalog (`HmiSymbolInfo[]`: name, bindable keys, props, default size) — an agent's palette reference. |
| `GET` | `/api/library` | Registry libraries + per-project import state. Returns `LibrarySummary[]` (name, version, `imported_version`, `imported_files`). |
| `POST` | `/api/library/import` | Vendor blocks into the project. Body: `ImportLibraryRequest { library, blocks?[] }` (empty `blocks` = all; re-import overwrites = the update path). Returns `ImportLibraryResponse { library, version, imported[] }`. |
| `DELETE` | `/api/library/{name}` | Drop `pous/lib/<name>/` and the project.toml entry. Idempotent. |
| `GET` | `/api/device-catalog` | Validated device templates from `<library-dir>/devices/`. Returns `CatalogEntry[]`. |
| `GET` | `/api/device-catalog/match?vendor_id=&product_id=` | Resolve a discovered slave's identity to a catalog template (pre-fill a device from an EtherCAT scan instead of hand-typing PDI offsets). Returns `CatalogEntry`; 404 when the identity isn't catalogued. |

## Devices

| Method | Path | Purpose |
|---|---|---|
| `POST` | `/api/devices` | Create a device. Body: `CreateDeviceRequest { name, protocol }`. |
| `POST` | `/api/devices/folders` | Create a folder under `devices/`. |
| `DELETE` | `/api/devices/folders/{path}` | Delete an empty folder. |
| `GET` | `/api/devices/{name}` | Read a device. Returns `Device`. |
| `PUT` | `/api/devices/{name}` | Update full device config. Body: `Device`. |
| `DELETE` | `/api/devices/{name}` | Delete. |
| `POST` | `/api/devices/{name}/esi-assemble` | Assemble a modular EtherCAT coupler's channels from its ESI file + the modules it reports. Body: `EsiAssembleRequest { detected: u32[] }` (module idents in slot order). Requires the device to be EtherCAT with `bringup = esi_modular`; the assembled channels **replace** the device's channel list. Returns the updated `Device`. |

## Edges (deploy targets)

| Method | Path | Purpose |
|---|---|---|
| `POST` | `/api/edges` | Create an edge. Body: `CreateEdgeRequest { name, host }`. |
| `POST` | `/api/edges/folders` | Create a folder under `edges/`. |
| `DELETE` | `/api/edges/folders/{path}` | Delete an empty folder. |
| `GET` | `/api/edges/{name}` | Read an edge. Returns `Edge`. |
| `PUT` | `/api/edges/{name}` | Update an edge. Body: `Edge`. |
| `DELETE` | `/api/edges/{name}` | Delete. Also tears down any open attach tunnel. |
| `GET` | `/api/edges/{name}/probe` | SSH+curl the edge's runtime `/health`. Returns `EdgeProbe`. |
| `GET` | `/api/edges/{name}/logs?tail=N` | Tail the edge runtime's journald logs over ssh (`tail` clamped to 2000, default 200). Returns JSON. |
| `GET` | `/api/edges/{name}/discover` | Per-device connect status + discovered EtherCAT topology from the edge, so PDO maps can be authored against the real bus. Returns JSON. |
| `GET` | `/api/edges/{name}/system` | Edge interfaces / serial ports / arch — for authoring device configs against real edge facts. Returns JSON. |
| `GET` | `/api/edges/{name}/status` | Proxy the edge runtime's `/status` (project + scan count + debug mode/forces + last snapshot). Returns JSON. |
| `POST` | `/api/edges/{name}/runtime/{op}` | Proxy an online-debug op to the *deployed* edge runtime over ssh. `op` ∈ {`pause`,`resume`,`step`,`write`,`force`,`unforce`}; body forwarded as the remote payload (e.g. `{cycles}` for step, `{name,value}` for write/force). |
| `POST` | `/api/edges/{name}/deploy` | Tar project + runtime binary + web assets (when this server has a `--static-dir`; they land at `current/web` for the edge's HMI panel), ssh to edge, atomic symlink swap, restart unit. Returns `DeployReport`. |
| `POST` | `/api/edges/{name}/attach` | Open `ssh -N -L` tunnel to the edge runtime port. Returns `AttachInfo { local_port }`. |
| `POST` | `/api/edges/{name}/detach` | Close the tunnel. |
| `GET` | `/api/edges/{name}/attachment` | Current attachment state. Returns `AttachmentStatus { attached, local_port }`. |

## IO Mapping

| Method | Path | Purpose |
|---|---|---|
| `GET` | `/api/iomap` | Read iomap.toml. Returns `IoMap`. |
| `PUT` | `/api/iomap` | Replace iomap.toml. Body: `IoMap`. |

## Tasks (project-level scheduling)

| Method | Path | Purpose |
|---|---|---|
| `GET` | `/api/tasks` | Read tasks.toml. Returns `Tasks { tasks: [], programs: [] }`. |
| `PUT` | `/api/tasks` | Replace tasks.toml. Body: `Tasks`. |

## Northbound (edge → platform publishing)

| Method | Path | Purpose |
|---|---|---|
| `GET` | `/api/northbound` | Read the edge's northbound (MQTT → supOS/Tier0) publishing config. Returns `NorthboundConfig`. |
| `PUT` | `/api/northbound` | Replace the northbound config. Body: `NorthboundConfig`. |

## Compile, run, observe

| Method | Path | Purpose |
|---|---|---|
| `POST` | `/api/check` | Compile-check ONE source string (no project required). Body: `text/plain`. Returns `CheckDiagnostic[]`. | Fast feedback for editor squiggles
| `POST` | `/api/symbols?language=st\|ld\|fbd\|sfc` | Extract declared variables from one source string (any language; default `st`). Body: `text/plain`. Returns `VariableInfo[]`. | Backs the editor's binding picker
| `POST` | `/api/run` | Compile the whole project + spawn the bridge. Body: `{}` or `RunRequest`. | Reads `tasks.toml` to decide what runs
| `POST` | `/api/stop` | Stop the running program (cooperative; scan loop drains). |
| `GET` | `/api/runtime/status` | Synchronous overview of the runtime. Returns `RuntimeStatus { running, project, scan_count, last_snapshot_us, last_error, devices_connected, programs_active }`; `last_error` carries the VM-trap / panic message when a run dies (also emitted as SSE `error` + `stopped`), `null` after a clean run. | One-shot, agent-friendly
| `GET` | `/api/runtime/snapshot` | Latest `VarSnapshot` or `null`. | No SSE needed for one-off queries
| `POST` | `/api/runtime/variables/{name}` | Write a variable while running. Body: `WriteVariableRequest { value: <i32-coerceable> }`. Returns the new value. | Critical for debugging closed loops
| `GET` | `/api/events` | SSE stream of `AppEvent` (`snapshot` / `started` / `stopped` / `error`). | For long-running IDE clients
| `GET` | `/api/project/variables` | Flat list of every variable across every POU in the project. Returns `ProjectVariables { variables: [...] }`. | Cross-POU index for agents
| `GET` | `/api/project/pous` | Every IEC POU declared anywhere in the project (parser-driven). Returns `ProjectPous { pous: [{ application, name, kind }] }` — `kind` ∈ `program` / `function_block` / `function`. | Source of truth for "what's schedulable" — multi-POU files (one .st declaring PROGRAM + FB + FUNCTION) are correctly enumerated, unlike `application.kind` which is a heuristic |

## Runtime debug control

Online debugging of the locally-running program (the IDE-side bridge). For
the *deployed* edge runtime, proxy the same ops through
`POST /api/edges/{name}/runtime/{op}`. All return `409` when nothing is running.

| Method | Path | Purpose |
|---|---|---|
| `POST` | `/api/runtime/pause` | Freeze the scan loop (last outputs hold). Returns `ModeResponse { mode }`. |
| `POST` | `/api/runtime/resume` | Resume free-running. Returns `ModeResponse`. |
| `POST` | `/api/runtime/step` | Advance N cycles while paused. Body: `StepRequest { cycles }` (default 1). Returns `ModeResponse`. |
| `GET` | `/api/runtime/forces` | List currently-forced variables. Returns `ForceEntry[]` (`[]` when not running). |
| `POST` | `/api/runtime/forces/{name}` | Pin a variable every cycle until released. Body: `ForceRequest { value }`. Returns `ForceResponse { name, value }`; 404 unknown variable, 409 if stopped. |
| `DELETE` | `/api/runtime/forces/{name}` | Release a forced variable. Idempotent (200 even if it wasn't forced). |

## Agent activity (takeover overlay)

Drives the IDE's "an agent is operating" overlay. See
`crates/server/src/events.rs` for the protocol. Read-only `cs` commands
don't call these.

| Method | Path | Purpose |
|---|---|---|
| `POST` | `/api/agent/heartbeat` | Transient one-off ping. Body: `AgentHeartbeatRequest { command, session? }`. Overlay flashes on, then ages out. |
| `POST` | `/api/agent/session/start` | Open an explicit takeover session (overlay stays on with `label`). Body: `{ id, label }`. Returns `AgentSessionResponse`. A fresh start replaces any open session. |
| `POST` | `/api/agent/session/end` | Close a session. Body: `{ id? }` (omit to force-end whatever's open — the IDE's "kick agent" button). Returns `RunResponse { ok }` (`ok=false` if nothing matched). |

## Bridges

| Method | Path | Purpose |
|---|---|---|
| `GET` | `/api/lsp` | WebSocket upgrade. Bridges to a freshly-spawned ironplc LSP process (JSON-RPC). | Frame format = bare JSON-RPC bodies — proxy adds/strips Content-Length headers for stdio |

## Internal / debug aids

These are intentionally prefixed `_` so they're easy to spot. Stable API
contract but only useful when wiring up demos or when the runtime hasn't
been pointed at real hardware yet.

| Method | Path | Purpose |
|---|---|---|
| `GET` | `/api/_demo/slave` | Peek the in-process demo Modbus slave's first 32 entries per address space. Returns `DemoSlaveSnapshot`. |
| `PUT` | `/api/_demo/slave/{kind}/{addr}` | Inject a value into the demo slave (e.g., to simulate a discrete-input edge). `kind` ∈ {`coil`, `discrete_input`, `holding_register`, `input_register`}; body: `{ value: bool | u16 }`. |

---

# Coverage

This doc was reconciled against the router on **2026-07-11** — every
`.route()` in `crates/server/src/main.rs` has a row above, and every route in
`crates/runtime/src/main.rs` has a row in the Edge runtime table below. When
you add a route, add its row here in the same change; the generated TypeScript
types under `apps/web/src/types/generated/` remain the source of truth for
shapes.

Notable capabilities, mapped to the agent-use-case checklist (see
`MEMORY/principles.md`):

- ✅ Whole-project compile-check → **POST /api/project/validate**
- ✅ One-shot latest snapshot (no SSE required) → **GET /api/runtime/snapshot**
- ✅ Runtime overview without curl-ing both `/health` and the SSE stream → **GET /api/runtime/status**
- ✅ Write a variable while running (debug agents) → **POST /api/runtime/variables/{name}**
- ✅ Inject input signals into demo slave → **PUT /api/_demo/slave/{kind}/{addr}**
- ✅ Delete a folder under applications / devices / edges → **DELETE /api/.../folders/{path}**
- ✅ Cross-POU variable index → **GET /api/project/variables**
- ✅ Cross-POU declaration index (real schedulable POU names) → **GET /api/project/pous**
- ✅ Health-under-/api alias → **GET /api/health**

# Redundancies (kept on purpose)

- `/health` + `/api/health` — `/health` is the convenience root for monitoring
  tooling; `/api/health` is the agent-friendly mirror. Trivial cost.
- `/api/check` + `/api/project/validate` — different scopes: `check` is "compile
  this string of source" (used by the editor while typing), `validate` is
  "compile the whole project" (used by agents before Run/Deploy).

# Edge runtime API (separate process)

The headless `ia2-runtime` binary (running on the edge) exposes the runtime
slice of the same surface — liveness/status, log tailing, discovery, and the
full online-debug set — bound to `127.0.0.1` only. The IDE proxies most of
these through `/api/edges/{name}/…` (and `/api/edges/{name}/runtime/{op}` for
the debug ops).

| Method | Path | Purpose |
|---|---|---|
| `GET` | `/health` | Liveness: status, uptime, scan count, plus `fieldbus_healthy` (true only when every configured device's transport is live) and a per-device `devices` health breakdown. |
| `GET` | `/status` | Project + PROGRAM instances + device list + scan count + last snapshot + debug mode + forces, plus `device_health` (per-device transport health; a `false` entry means that device's inputs are frozen at last-known values) and `fault` (why the scan loop died — VM trap / panic message — `null` while running or after a clean stop). |
| `GET` | `/events` | SSE stream of `VarSnapshot` (bare — no `AppEvent` wrapper). |
| `GET` | `/logs?tail=N` | The most recent `tail` (default 200) captured log lines. What `cs edge logs` pulls over the tunnel. |
| `GET` | `/logs/stream` | SSE stream of log lines as they're emitted (no backlog; pair with `/logs` for history). |
| `GET` | `/discover` | Per-device connect reports + discovered EtherCAT topology. Powers `cs edge scan`. |
| `GET` | `/system` | Edge NICs / serial ports / arch, for authoring device configs against real edge facts. Powers `cs edge system`. |
| `POST` | `/pause` | Freeze the scan loop (last outputs hold). Returns `{ mode }`. |
| `POST` | `/resume` | Resume free-running. Returns `{ mode }`. |
| `POST` | `/step` | Advance N cycles while paused. Body: `{ cycles }`. Returns `{ mode }`. |
| `POST` | `/write` | One-shot write of a variable. Body: `{ name, value }`. Returns the applied value. |
| `POST` | `/force` | Pin a variable every cycle until released. Body: `{ name, value }`. |
| `POST` | `/unforce` | Release a forced variable. Body: `{ name }`. |
| `POST` | `/stop` | Request graceful shutdown. |
| `GET` | `/api/hmi` | HMI screens deployed with the project: `[{ path, title, level }]` (same row shape as the IDE route). Read-only — screens are edited in the IDE and arrive via deploy. |
| `GET` | `/api/hmi/{path}` | One screen's full `HmiDoc` JSON. 404 if the deployed project has no such screen. |

With `--static-dir` pointing at the built web assets the runtime also serves the
standalone operator panel: `GET /hmi` lists the deployed screens, `GET /hmi/{rest}`
renders one (`/` redirects to `/hmi`). The panel is a separate vite entry
(`hmi.html`) that talks only to this runtime's own surface — `/events` for live
values, `/write` for confirmed actions, `/status` for the fault strip — so an
operator client needs nothing but a browser and this port.

Access from the dev machine: open an `ssh -N -L <local>:127.0.0.1:<runtime_port> <edge>`
tunnel (see `/api/edges/{name}/attach`) and hit `http://127.0.0.1:<local>/...`.
