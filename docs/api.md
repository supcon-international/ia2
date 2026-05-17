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

## Devices

| Method | Path | Purpose |
|---|---|---|
| `POST` | `/api/devices` | Create a device. Body: `CreateDeviceRequest { name, protocol }`. |
| `POST` | `/api/devices/folders` | Create a folder under `devices/`. |
| `DELETE` | `/api/devices/folders/{path}` | Delete an empty folder. |
| `GET` | `/api/devices/{name}` | Read a device. Returns `Device`. |
| `PUT` | `/api/devices/{name}` | Update full device config. Body: `Device`. |
| `DELETE` | `/api/devices/{name}` | Delete. |

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
| `POST` | `/api/edges/{name}/deploy` | Tar project + runtime binary, scp to edge, atomic symlink swap, restart unit. Returns `DeployReport`. |
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

## Compile, run, observe

| Method | Path | Purpose |
|---|---|---|
| `POST` | `/api/check` | Compile-check ONE source string (no project required). Body: `text/plain`. Returns `CheckDiagnostic[]`. | Fast feedback for editor squiggles
| `POST` | `/api/run` | Compile the whole project + spawn the bridge. Body: `{}` or `RunRequest`. | Reads `tasks.toml` to decide what runs
| `POST` | `/api/stop` | Stop the running program (cooperative; scan loop drains). |
| `GET` | `/api/runtime/status` | Synchronous overview of the runtime. Returns `RuntimeStatus { running, project, scan_count, last_snapshot_us, last_error, devices_connected, programs_active }`. | One-shot, agent-friendly
| `GET` | `/api/runtime/snapshot` | Latest `VarSnapshot` or `null`. | No SSE needed for one-off queries
| `POST` | `/api/runtime/variables/{name}` | Write a variable while running. Body: `WriteVariableRequest { value: <i32-coerceable> }`. Returns the new value. | Critical for debugging closed loops
| `GET` | `/api/events` | SSE stream of `AppEvent` (`snapshot` / `started` / `stopped` / `error`). | For long-running IDE clients
| `GET` | `/api/project/variables` | Flat list of every variable across every POU in the project. Returns `ProjectVariables { variables: [...] }`. | Cross-POU index for agents
| `GET` | `/api/project/pous` | Every IEC POU declared anywhere in the project (parser-driven). Returns `ProjectPous { pous: [{ application, name, kind }] }` — `kind` ∈ `program` / `function_block` / `function`. | Source of truth for "what's schedulable" — multi-POU files (one .st declaring PROGRAM + FB + FUNCTION) are correctly enumerated, unlike `application.kind` which is a heuristic |

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

# Gap audit (May 2026)

Compared against the agent-use-case checklist (see
`memory/principle_api_first.md`), the following were missing **before this
revision**. The bold ones are added in this revision; the rest were already
present.

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

The headless `ia2-runtime` binary (running on the edge) exposes
a small subset of the same surface, bound to `127.0.0.1` only:

| Method | Path | Purpose |
|---|---|---|
| `GET` | `/health` | Liveness. |
| `GET` | `/status` | Project + program instances + scan count + last snapshot. |
| `GET` | `/events` | SSE stream of `VarSnapshot` (bare — no `AppEvent` wrapper). |
| `POST` | `/stop` | Request graceful shutdown. |

Access from the dev machine: open an `ssh -N -L <local>:127.0.0.1:<runtime_port> <edge>`
tunnel (see `/api/edges/{name}/attach`) and hit `http://127.0.0.1:<local>/...`.
