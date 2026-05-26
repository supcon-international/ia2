# Mental model

Three layers. Memorise them — every other reference assumes this picture.

```
┌──────────────────────────────────────────────────────────────────┐
│  Client(s)                                                       │
│                                                                  │
│   IA2.app (Mac shell, WKWebView)        Browser tab (web SPA)    │
│   ─────────────────────────────────     ────────────────────     │
│   one binary, embeds the server         points at any server     │
│   `Cmd+N` → new window                  URL-scoped per ?project  │
│                                                                  │
│   `cs` CLI (this is what you use)                                │
│   ─────────────────────────────────                              │
│   binary at target/release/cs (or in $PATH)                      │
│   talks HTTP only; defaults to http://127.0.0.1:3001             │
└──────────────────────────────────────────────────────────────────┘
                              │ HTTP + SSE
                              ▼
┌──────────────────────────────────────────────────────────────────┐
│  IA2 server (axum, Rust)                                         │
│                                                                  │
│   ProjectRegistry           Program (singleton)                  │
│   ───────────────────       ─────────────────                    │
│   N open projects           one PROGRAM at a time                │
│   `X-IA2-Project` header    (hardware constraint — Modbus,       │
│   selects which              EtherCAT bus can have only one      │
│                              master per process)                 │
│                                                                  │
│   Agent activity            Event broadcaster                    │
│   ───────────────────       ─────────────────                    │
│   Session OR heartbeat      Mutation events tagged with          │
│   drives takeover overlay   project name; web filters            │
└──────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌──────────────────────────────────────────────────────────────────┐
│  ironplc-bridge (vendored ironplc + scan loop)                   │
│                                                                  │
│   compile_project_full() → (Container, ProgramMetadata)          │
│   spawn_with_options()   → ProgramHandle                         │
│     - scan loop ticks at tasks.toml interval (default 100ms)     │
│     - RETAIN vars persisted to <project>/state/retain.json       │
│     - panic / VM trap / stop → failsafe on all devices           │
└──────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌──────────────────────────────────────────────────────────────────┐
│  IoDevice adapters (iomap-*)                                     │
│                                                                  │
│   Modbus TCP  ── tokio-modbus over TcpStream                     │
│   Modbus RTU  ── tokio-modbus over tokio_serial::SerialStream    │
│   EtherCAT    ── ethercrab on a dedicated OS thread              │
│                                                                  │
│   Trait: read_channel / write_channel / enter_failsafe           │
└──────────────────────────────────────────────────────────────────┘
```

## Five facts you must remember

### 1. One server hosts N projects; a header picks which

Projects live in `~/Documents/IA2/<name>/`. The server holds a `ProjectRegistry` keyed by name. Every CLI request carries `X-IA2-Project: <name>` (set by `--project` flag); missing header → server falls back to its LRU "active" project. **Always pass `--project` if `cs project list` shows more than one.**

### 2. One running PROGRAM at a time

There is exactly one `Program` slot in server state. Starting a new program stops the previous one. The slot remembers which project owns it, so `/api/runtime/status` answers "running: foo/main" or "running: bar/conveyor". Don't try to "run two programs" — there's no `tasks.toml` shape that allows it (ironplc codegen emits only one PROGRAM per compilation).

### 3. The scan loop is real and drives real hardware

`spawn_with_options()` starts a dedicated `std::thread` running a tokio current-thread runtime. Each scan:
- Input phase: every iomap'd `direction: input` reads its channel
- Force phase: pinned values overwrite VM state
- VM `run_round(now_us)` executes the bytecode
- Output phase: every iomap'd `direction: output` writes its channel
- Sleep until `next_scan_at` (cadence from `tasks.toml interval_ms`)

The default interval is 100 ms. Faster than 10 ms taxes the snapshot fan-out. RTU at 9600 baud is slow enough that 200-500 ms cadence is the realistic floor.

### 4. The agent overlay has two modes

- **Session mode** (preferred): `cs agent run --label "X"` opens an explicit session. Banner stays on with `X` as text. Server's watchdog only ends the session after 30 s of no heartbeat (crash recovery).
- **Transient heartbeat mode** (back-compat): a single `cs` command sends one heartbeat; the overlay flashes on with the command name; ages out after 3 s. **Always prefer session mode for multi-step work.**

### 5. Mutations are project-scoped on the wire

Every `MutationEvent` carries a `project: String` field. Web clients filter SSE events to those matching their URL `?project=`. So window A editing project `foo` doesn't make window B (showing project `bar`) re-fetch its tree.

## Where things live on disk

```
~/Documents/IA2/
  bottling_line/                       ← one project
    project.toml                       ← manifest (name, version)
    pous/
      main.st                          ← Structured Text PROGRAM/FB/FUNCTION
      conveyor.ld.json                 ← LD as JSON (graphical)
      reactor.fbd.json                 ← FBD as JSON
      batch_state.sfc.json             ← SFC as JSON
    devices/
      hmi_plc.toml                     ← per-device config (modbus / ethercat)
    edges/
      field_pi.toml                    ← deploy targets (SSH host etc.)
    iomap.toml                         ← variable ↔ device.channel bindings
    tasks.toml                         ← PROGRAM instance ↔ task schedule
  
  .ia2-open-projects.json              ← server's "open this on startup" list
  state/                               ← per-project, sibling of project dir
    bottling_line/
      retain.json                      ← VAR RETAIN persisted values
```

## Where you talk to the server

| Goal | HTTP | CLI equivalent |
|---|---|---|
| Health check | `GET /api/health` | (none — use `cs project list`) |
| List open projects | `GET /api/projects/open-list` | `cs project list` |
| Open a project | `POST /api/projects/open {path}` | `cs project open PATH` |
| Get project tree | `GET /api/project` (with header) | `cs project info PATH` |
| Save POU source | `PUT /api/pous/{path}` | `cs pou save NAME --stdin` |
| Validate full project | `POST /api/project/validate` | `cs project check PATH` |
| Start running | `POST /api/run` | `cs run [--program X]` |
| Pause / step / resume | `POST /api/runtime/{action}` | `cs runtime pause/step/resume` |
| Force a variable | `POST /api/runtime/forces/{name}` | `cs runtime force NAME VALUE` (negatives need `-- NAME -N`) |
| One-shot write / unforce | `POST /api/runtime/variables/{name}` · `DELETE /api/runtime/forces/{name}` | `cs runtime write NAME VALUE` · `cs runtime unforce NAME` |
| IoMap / Tasks docs | `GET·PUT /api/iomap` · `GET·PUT /api/tasks` | `cs iomap get/set` · `cs tasks get/set` |
| Live snapshot / status | `GET /api/runtime/snapshot` | `cs runtime status --json` (mode + forces + vars) |
| Edge introspection | `GET /api/edges/{name}/{logs,discover,system,status}` | `cs edge logs/scan/system` · `cs probe` |
| Drive an edge runtime | `POST /api/edges/{name}/runtime/{op}` | `cs runtime <op> --edge NAME` |
| SSE event stream | `GET /api/events` | (SSE — see `02-cli-reference.md`) |
| Start agent session | `POST /api/agent/session/start` | `cs agent enter` / `cs agent run -- ...` |
| End agent session | `POST /api/agent/session/end` | `cs agent leave` / auto on `cs agent run` exit |
