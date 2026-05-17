# IA2

A simple, agent-first IDE + runtime for IEC 61131-3 PLC programming.

> Positioned against Codesys / TwinCAT / Step 7 — same standard,
> 1/50 the complexity. **Agents (Claude Code, Codex, Cursor) are
> first-class users alongside humans.** Every feature reachable via
> GUI is also reachable via the `cs` CLI and the HTTP API.

## What's in the box

| Component | Tech | Purpose |
|---|---|---|
| **`apps/mac/`** | Swift + AppKit + WKWebView | Native macOS shell (`IA2.app`). Hosts the React UI in a system WebView; supervises the Rust backend as a child process. |
| **`apps/web/`** | React 19 + Vite + TanStack Router + Tailwind 4 | The IDE itself. ST / LD / FBD / SFC editors, runtime Monitor, project tree, IO mapping. Single SPA, serves identically in the desktop shell and the dev `vite` browser. |
| **`crates/server/`** | Rust + axum + tower | HTTP backend (port 3001 dev, random in desktop). REST + SSE. Owns the project, dispatches to ironplc-bridge, schedules tasks. |
| **`crates/cli/`** | Rust + clap + ureq | The `cs` binary — agent-first command-line. Static analysis, project CRUD, runtime debug. See `cs --help`. |
| **`crates/ironplc-bridge/`** | Rust | Wraps vendored [ironplc](https://github.com/ironplc/ironplc) compiler + VM. Adds LD / FBD / SFC → ST transpilers + diagnostics enrichment. |
| **`crates/runtime/`** | Rust | Headless edge runtime (`ia2-runtime` binary). Same scan loop as the IDE-side bridge but with no HTTP / LSP / CORS — designed for Linux edge boxes. |
| **`crates/project/`** | Rust | On-disk project schema (POU files, devices, edges, iomap, tasks). |
| **`crates/iomap-modbus/` `iomap-ethercat/`** | Rust | I/O adapters: Modbus TCP (tokio-modbus), EtherCAT (ethercrab). |
| **`vendor/ironplc/`** | git submodule | The compiler + VM. |

## Two interfaces, one source of truth

The HTTP API is the canonical contract. The desktop UI, the CLI,
agents, and (future) MCP all talk to it. Everything is JSON; everything
is curlable.

```
                    HTTP + SSE (port 3001 or random)
                              │
       ┌──────────────────────┼──────────────────────┐
       ▼                      ▼                      ▼
  apps/web (React)      crates/cli (`cs`)       agents (Claude
   in WKWebView                                   Code / Codex /
   or browser           in terminal / CI          MCP wrappers)
```

When an agent runs `cs pou create`, the server emits a `Mutation`
event over SSE; the IDE's project tree updates in real time and the
editor auto-jumps to the new POU. Same in reverse: when a human saves
a POU in the IDE, an agent's `cs runtime status` sees the new symbol
table immediately.

## Agent takeover overlay

When the `cs` CLI is mid-flight, the IDE shows a pulsing green border
plus a top-centre "Agent in control" banner. User input is softly
blocked while takeover is active. Click **Take over** to suppress for
8 seconds and regain pointer control — designed for the case where a
human needs to step in mid-agent-run without race-conditioning on the
same files.

The signal is a 3-second TTL heartbeat: every mutating `cs` subcommand
pings `POST /api/agent/heartbeat` at start, the server holds the
"active" flag for 3 s of silence, then drops it. Read-only commands
(`cs check`, `cs project info`, `cs runtime status`) don't trigger the
overlay — querying state isn't "operating."

## Quickstart

### Run the IDE

```bash
# one-time
. "$HOME/.cargo/env"
pnpm install
cargo test -p server   # populates apps/web/src/types/generated/

# desktop shell (single binary, recommended)
./apps/mac/build.sh
open apps/mac/build/debug/IA2.app

# OR dev mode — two terminals
pnpm --filter @cs/web dev      # → http://localhost:3000
cargo run -p server            # → http://localhost:3001
```

### Drive it from the CLI

```bash
cargo build -p ia2-cli
alias cs=./target/debug/cs

# the everyday agent loop
cs check pous/safe_start.ld.json        # validate any language
cs project info ~/Documents/IA2/demo    # what's in this project?
cs project check ~/Documents/IA2/demo   # full compile

# project CRUD (talks to a running server)
cs project create my_line               # → ~/Documents/IA2/my_line/
cs project open ~/Documents/IA2/my_line
cs pou create motor --language ld
cs pou save motor --from motor.ld.json
cs pou delete motor

# runtime control
cs run                                  # schedule everything in tasks.toml
cs runtime status
cs runtime force pump_pct 50.0          # type-aware: REAL bit-packed, BOOL as 0/1
cs runtime pause / step / resume
cs stop
```

Exit codes follow Unix: `0` clean / `1` source has errors / `2` usage /
`≥3` infrastructure. Every subcommand supports `--json`. Every
subcommand's `--help` explains when to call it and what to call next —
written for agent readers.

## Project on disk

```
~/Documents/IA2/
└── my_project/
    ├── project.toml         metadata + version
    ├── tasks.toml           task → program scheduling
    ├── iomap.toml           variable ↔ device-channel wiring
    ├── pous/
    │   ├── cascade_pid.st         IEC Structured Text
    │   ├── motor_seal.ld.json     Ladder Diagram (JSON authored)
    │   ├── click_counter.fbd.json Function Block Diagram
    │   └── batch_sequence.sfc.json Sequential Function Chart
    ├── devices/             Modbus / EtherCAT devices
    └── edges/               Deploy targets (Linux edge boxes)
```

JSON for graphical languages (not PLCopen XML) so agents and
`git diff` can read it. LD / FBD / SFC are transpiled to ST before
reaching ironplc; the intermediate ST is observable via
`cs transpile foo.ld.json`.

## Edge deployment

`crates/runtime/` builds the `ia2-runtime` binary — headless, no HTTP,
no LSP. Push it to a Linux edge box, install the systemd unit
(`infra/ia2.service`), then `Deploy` from the IDE to atomically swap
projects without restarting the runtime. See `docs/edge-deploy.md`.

## Design principles

Read `MEMORY/principles.md` first if you're contributing. The headline:

1. **Simplicity is the headline feature.** Defaults work without
   configuration. One concept per screen. No "advanced settings."
2. **Agent-friendly is co-equal with human-friendly.** Anything
   the GUI does, the CLI + HTTP API also do.
3. **Text-first storage.** ST as `.st`, graphical languages as JSON.
   `grep`, `git diff`, `cat` all work.

## License

Apache-2.0. Vendored `ironplc/` is also Apache-2.0.
