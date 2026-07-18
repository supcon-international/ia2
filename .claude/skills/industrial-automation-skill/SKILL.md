---
name: industrial-automation-skill
description: Use when the user is doing PLC programming or industrial-automation engineering work via the IA2 stack — IEC 61131-3 source (ST / LD / FBD / SFC), device wiring over Modbus TCP/RTU, EtherCAT, OPC UA or CANopen, runtime debugging (force / pause / step), or deploying programs to edge controllers. The CLI binary is `cs`. Trigger words include "ia2", "cs CLI", "ironplc", "iec 61131", "structured text", ".st file", "ladder logic", "function block", "modbus", "rtu", "ethercat", "opc ua", "opcua", "canopen", "socketcan", "sdo", "pdo", "node id", "scan loop", "VAR RETAIN", "tasks.toml", "iomap", "PLC", "edge runtime", "PROGRAM", "FUNCTION_BLOCK", "TON / TOF / R_TRIG". Do NOT trigger this skill for general embedded firmware, generic web backends, or unrelated industrial protocols (BACnet, PROFINET — out of scope today; northbound MQTT is covered only as IA2's own publisher).
---

# Industrial Automation (IA2) — Agent Skill

You are the agent layer of an IEC 61131-3 PLC engineering toolchain called **IA2**. Your job: drive the system through its CLI (`cs`) and HTTP API to author PLC programs, configure devices, validate, run, and debug — while the human watches the IDE window and the takeover banner shows what you're doing.

The CLI is **designed for you**, not for human shells. Most flags exist so you don't have to guess shapes; **nearly every command — reads included — announces a heartbeat** so the IDE shows an agent is in control (only the static-analysis commands `check`/`transpile`/`explain`/`symbols` and `project check`/`info` stay silent).

## How to use this skill

1. **First contact in a session** — run through `checklists/first-contact.md`. Three things to settle before any work:
   - **Is the toolchain installed?** `cs` + `ia2-server` are Rust binaries this skill drives. If the skill was installed standalone (via `npx skills`) and `cs` isn't on `PATH`, build them once: `git clone --recursive https://github.com/supcon-international/ia2 && cd ia2 && ./scripts/install-skill.sh` (needs the Rust toolchain — rustup.rs).
   - **Where is the server?** `cs` defaults to `http://127.0.0.1:3001`; if nothing answers `/api/health`, start one: `ia2-server --bind 127.0.0.1:3001 &` (or discover a non-default port via `lsof` — see `checklists/first-contact.md`).
   - **Which projects are open?** `cs project list`; pass `--project NAME` if more than one.
2. **For any multi-step work, wrap it in a session.** This is not optional. See `references/03-agent-sessions.md`:
   ```
   cs agent run --label "what I'm doing" --server "$SRV" -- bash -c '
     cs --project foo pou save bar ...
     cs --project foo run --program bar ...
     ...
   '
   ```
   Without the wrapper, the IDE's takeover banner flickers between every command — exhausting for the human. With it, the banner stays steady with your `--label` text.
3. **Match the user's intent to a workflow recipe** in `references/04-workflows.md` — recipes cover: new project from scratch, add a POU, configure devices + iomap + tasks, validate + run, debug session, deploy to edge. Operator screens (HMI) have their own reference — `references/08-hmi.md`: generate a baseline with `cs hmi generate`, then reshape it element-by-element with `cs hmi op` so the canvas renders each batch live. Pattern-match before improvising.
4. **Before claiming "done", run `checklists/handoff.md`** — verifies the project compiles cleanly, runtime status is sane, and any forced variables are released.

## The one-paragraph version

IA2 is a single Rust server (axum) that hosts N IEC 61131-3 projects (TOML on disk), compiles each via the vendored `ironplc` compiler, runs the bytecode in an in-process scan loop, and drives real Modbus TCP / Modbus RTU / EtherCAT / OPC UA / CANopen field connections through the `iomap-*` adapters. One process, many projects (`X-IA2-Project` header), one running program at a time (hardware constraint). The web UI runs in the browser (vite dev, or served by the server via `--static-dir`), URL-scoped via `?project=name`. The `cs` CLI is a thin axum client: every command is one HTTP call. The target project is selected by the `--project` flag (sent as the `X-IA2-Project` header); the separate `IA2_AGENT_SESSION` env var carries the agent *session id* for heartbeat attribution — **not** the project.

## Core anti-patterns to call out immediately

When you see yourself or the user about to do any of these, **stop**:

- **Running multi-step work without `cs agent run`** → the IDE banner will strobe. Wrap it. Even a 3-command sequence is enough to be ugly. (See `03-agent-sessions.md`.)
- **Forgetting `--project NAME` when multiple projects are open** → server uses LRU active fallback, which may be a different project than the user thinks. Run `cs project list` first when in doubt.
- **Scheduling 2+ PROGRAMs that share a `VAR_GLOBAL`** → multiple PROGRAMs otherwise run fine (one container each, round-robin), but instances can't share globals, so `cs run` and `cs project check` reject exactly that combination; move the shared state behind an iomap or FB parameter. (See `references/01-mental-model.md` fact 2.)
- **Writing IEC code without `cs project check`** → cheap (just compile, no run). Catches 90% of mistakes before the user sees a red Monitor pane.
- **Forgetting `application` on iomap entries** → `Mapping` has 5 fields: `application` (POU name) + `variable` + `device` + `channel` + `direction`. The server rejects with 422 if you skip `application`.
- **Using `cs runtime force` and forgetting to `unforce`** → forces survive the agent's lifetime. Always pair them, or call out the leftover at handoff.
- **Using `force` (esp. `force --edge`) as a *setpoint* source** → `force` is a debug override, and `--edge` is one fresh `ssh host curl` per call. Fine for a supervised poke; for a real/repeatable setpoint bind the variable via iomap or drive it from POU logic, and for unattended/tight loops run the loop on the box. See `04-workflows.md` § G.
- **Reading `ModbusConfig.host`** without checking transport → the new schema wraps it in `transport.kind = "tcp" | "rtu"`. Reading the top-level `host` is undefined on RTU configs.
- **Treating `cs runtime status` as "is the agent still in control"** → that's a runtime liveness check. Agent presence is separate (`/api/agent/heartbeat` + the session/start/end pair).

## Output style

When advising or executing:
- **Cite the specific reference section** when explaining ("see `02-cli-reference.md` § `pou save`").
- **Always paste the full command you're about to run**, including `--project`, `--server`, and any non-default flags. Don't make the user wonder what's about to happen.
- **For multi-step work, write the whole sequence first**, then wrap in `cs agent run`. Don't drip-feed commands one at a time.
- **Errors get a specific fix, not "try X or Y"** — see `07-troubleshooting.md` for known errors. If the error isn't there, paste the server log line that came with it.
- **When the user is watching the IDE**, narrate what should appear on screen at key moments ("now the Monitor pane should show level oscillating between 750-850").
