# CLI reference (`cs`)

The binary is `cs`. In a dev checkout it's `target/release/cs` (build with `cargo build -p ia2-cli --release`); installed, it's on `$PATH`. Set a variable once per session:

```bash
CS="${CS:-cs}"   # on PATH after install; in a checkout: ./target/release/cs
SRV="http://127.0.0.1:60349"   # discover the real port — see checklists/first-contact.md
```

## Global flags

| Flag | Meaning |
|---|---|
| `--project NAME` | Target a specific open project. **Global** — works on every subcommand. Omit when only one project is open. |
| `--server URL` | Server base URL. Default `http://127.0.0.1:3001`. **Override for IA2.app** (ephemeral port). |
| `--json` | Machine output on stdout (most read subcommands). |

Exit codes: `0` success · `1` clean run but source has errors · `2` usage error · `≥3` infrastructure failure.

## Static analysis (no server needed)

```bash
cs check pous/main.st                  # validate one POU; --explain for RST docs; --json
cs check a.st b.ld.json                # multiple files, aggregated
cs transpile pous/conveyor.ld.json     # ST that a graphical POU (.ld / .fbd / .sfc.json) lowers to; --with-map (no-op on ST)
cs symbols pous/main.st                # declared symbols (name/type/direction); also reads .ld / .fbd / .sfc.json; --json
cs explain P0073                        # full RST doc for an ironplc problem code
cs project check ~/Documents/IA2/foo   # compile the WHOLE project (every POU + synth CONFIG)
cs project info  ~/Documents/IA2/foo   # list POUs/devices/edges; --json
```

`cs check` / `cs project check` are cheap (compile only, no run). **Run them before every `cs run`.**

## Project lifecycle (server)

```bash
cs project create NAME                 # new project under ~/Documents/IA2/NAME, becomes active
cs project open  /abs/path/to/project  # add to open set, becomes active (doesn't close others)
cs project close                       # close the --project (or active); stops its program
cs project list                        # open projects, active marked with *; --json
```

## POUs

```bash
# create seeds a minimal compileable skeleton for the language
cs pou create main          --type program        --language st
cs pou create safety        --type function_block --language st
cs pou create conveyor      --type program        --language ld
cs pou create batch_state   --type program        --language sfc
cs pou create flow_calc     --type function_block --language fbd

# overwrite source — three input modes
cs pou save main --from main.st          # from a file
cs pou save main --stdin <<'ST' ... ST   # from a heredoc (most common for agents)
echo "$SRC" | cs pou save main           # piped stdin (auto-detected when not a TTY)

cs pou delete main
```

`--type`: `program` | `function_block` | `function`. `--language`: `st` | `ld` | `fbd` | `sfc`.
LD/FBD/SFC sources are **JSON**, not text — `cs pou save` for those expects the graphical-program JSON shape. For hand-authoring, ST is almost always what you want.

## Devices (Modbus / EtherCAT)

```bash
cs device create hmi_plc   --protocol modbus      # TCP defaults (127.0.0.1:502)
cs device create servo_bus --protocol ethercat    # sim NIC "_sim" by default
cs device list                                    # name + protocol; --json
cs device get  hmi_plc                             # full config as JSON (round-trip source)
cs device set  hmi_plc --from cfg.json             # replace config; --from - reads stdin
cs device delete hmi_plc
```

To configure channels / switch a Modbus device to RTU / set EtherCAT PDOs, use
`cs device get NAME` → edit the JSON → `cs device set NAME --from -`. Shapes are in `06-devices-iomap-tasks.md`.

## Edges (deploy targets)

```bash
cs edge create field_pi --host pi@bottle-line.local
cs edge list                                       # name + host; --json
cs edge get  field_pi                              # full config (ssh_port, install_dir, runtime_port…)
cs edge set  field_pi --from edge.json             # replace; --from - for stdin
cs edge delete field_pi
cs deploy field_pi          # tar project → ssh → versioned dir + symlink swap + systemd restart
cs probe  field_pi          # quick ssh+curl reachability (scan count, uptime, version)

# Introspect a live edge (all over ssh+curl — see transport note):
cs edge logs   field_pi [--tail N]  # tail the runtime log (EtherCAT discovery, bus health, connect errors); N default 200, cap 2000
cs edge scan   field_pi             # per-device connect status + discovered EtherCAT topology (index/name/vendor/product/PDI sizes)
cs edge system field_pi             # the edge's NICs / serial ports / arch — pick a nic or tty for a device
```

**Transport.** `cs` never reaches the edge runtime directly — the *local* server shells out to your machine's `ssh` (`~/.ssh/config`, keys, agent; `BatchMode=yes`, `StrictHostKeyChecking=accept-new`). Two shapes:
- **one-shot `ssh host curl 127.0.0.1:<runtime_port>/…`** — used by `probe`, `edge logs/scan/system`, and `runtime … --edge` (below). A fresh ssh per call; fine for pokes, *not* for tight loops.
- **persistent `ssh -N -L <localport>:127.0.0.1:<runtime_port>`** — a server-managed "attach" tunnel for live streaming (log stream, the web **Edge → Debug** tab's polling). Torn down on `cs edge delete` / project close. Not exposed as a `cs` subcommand.

## IoMap + Tasks (whole-document get/set)

```bash
cs iomap get                # current bindings as JSON
cs iomap set --from map.json # replace all; --from - reads stdin
cs tasks get                # current tasks.toml as JSON
cs tasks set --from t.json   # replace all; --from - reads stdin
```

Both are **replace-the-whole-document** operations — `get`, edit, `set`. There's no per-entry add/remove. Shapes in `06-devices-iomap-tasks.md`.

## Run + runtime debug

```bash
cs run                              # run the whole tasks.toml schedule
cs run --program main               # ad-hoc single-PROGRAM run (synth schedule, tasks.toml untouched)
cs run --program main --file pous/main.st   # isolated: compile only this file
cs stop                             # stop the running program

cs runtime status                   # mode + forces + which project/program is running; --json
cs runtime pause                    # freeze the plant (no IO, no run_round)
cs runtime resume
cs runtime step 20                  # run N cycles then auto-pause
cs runtime force  setpoint 200      # pin a variable every scan until unforced
cs runtime unforce setpoint
cs runtime write  setpoint 200      # one-shot write (program can overwrite next cycle)
```

Value encoding for `force`/`write`: the CLI fetches the variable's type from the live snapshot and bit-packs. `TRUE`/`FALSE`/`1`/`0` for BOOL, decimal for INT, decimal float for REAL (IEEE-754 bit pattern sent). **Negative / leading-dash values need a `--` separator** — `cs runtime force -- speed -500` — otherwise clap parses `-500` as a flag and the force silently fails (positives work without it).

**Target an edge runtime with `--edge <name>`.** Every `cs runtime …` verb (`status/pause/resume/step/force/unforce/write`) accepts `--edge field_pi` to drive a *deployed* edge instead of the local server (routed as one-shot `ssh host curl …` — see Edges transport). It's the same surface the web **Edge → Debug** tab uses, so the pokes render in the IDE's agent-takeover overlay. But `force --edge` is a *debug override, not a setpoint channel* — see workflow G in `04-workflows.md` for when **not** to reach for it.

## Agent session (the wrapper you almost always want)

```bash
# Wrap a whole workflow — banner stays steady with --label for the duration
cs agent run --label "Building bottling line" -- bash -c '
  cs project create ...
  cs pou save ...
  cs run ...
'

# Manual session for shell scripts (prefer `run` — it cleans up on Ctrl-C)
SESSION=$(cs agent enter --label "Long task")
# ... many cs calls; they auto-attach via IA2_AGENT_SESSION env ...
cs agent leave --id "$SESSION"
```

See `03-agent-sessions.md` — this is the single most important pattern in the skill.

## Reading the live SSE event stream

There's no `cs events` subcommand. Tail the stream with `curl`:

```bash
curl -sN "$SRV/api/events"     # Server-Sent Events: snapshots, mutations, agent activity, started/stopped
```

For one-shot state, prefer `cs runtime status --json` or `curl -s "$SRV/api/runtime/snapshot"`.

## Northbound (MQTT publishing config)

```bash
cs northbound get                  # northbound.toml as JSON
cs northbound set --from nb.json   # replace; --from - for stdin
```

Shape + topic contract in `06-devices-iomap-tasks.md` § Northbound. The *edge runtime* reads this at startup — `cs deploy` + restart to apply on a box.
