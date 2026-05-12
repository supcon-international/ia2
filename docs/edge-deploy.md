# Edge deployment

How to ship a controlsoftware project to a Linux edge box and use the IDE
for online debugging against it.

## What this is

The IDE has three top-level concepts:

| | What it represents |
| --- | --- |
| **Applications** | POUs — your ST / FB source files |
| **Devices** | Fieldbus things your program talks to (Modbus, EtherCAT) |
| **Edges** | Linux boxes where the program runs in production |

Edges are deploy targets, not runtime peers. The IDE never opens a network
port to one directly — every interaction goes through SSH. You add an
`~/.ssh/config` entry for the edge, list it in the project, click Deploy,
and the IDE bundles the project + a runtime binary, scp's it over,
atomically swaps the live version, and restarts the systemd unit.

## One-time edge setup

You need a Linux box reachable over SSH with key-based auth (no password
prompts — the IDE runs `ssh -o BatchMode=yes`).

1. **Get a runtime binary for the edge's architecture**. From a Linux dev
   machine of the same arch:
   ```sh
   cargo build --release -p controlsoftware-runtime
   # binary lands at target/release/controlsoftware-runtime
   ```
   For cross-arch (e.g. ARM64 edge from x86_64 dev), use `cross`:
   ```sh
   cargo install cross --git https://github.com/cross-rs/cross
   cross build --release -p controlsoftware-runtime \
     --target aarch64-unknown-linux-gnu
   # binary lands at target/aarch64-unknown-linux-gnu/release/controlsoftware-runtime
   ```
   All deps are pure Rust; cross-compile should succeed without extra
   system libs.

2. **Bootstrap the edge**. From your dev machine:
   ```sh
   scp infra/controlsoftware.service edge:/tmp/
   scp infra/install.sh             edge:/tmp/
   scp target/.../release/controlsoftware-runtime edge:/tmp/
   ssh edge "sudo INSTALL_DIR=/opt/controlsoftware \
                 RUNTIME_BIN=/tmp/controlsoftware-runtime \
                 UNIT_FILE=/tmp/controlsoftware.service \
                 bash /tmp/install.sh"
   ```
   Verify with `ssh edge systemctl status controlsoftware`. It should be
   *enabled, not yet started*.

3. **Optional: smoke-start the stub**. Confirms the binary itself runs:
   ```sh
   ssh edge "sudo systemctl start controlsoftware && \
             curl -s http://127.0.0.1:13001/health"
   # → {"status":"ok","uptime_secs":2,"scan_count":15}
   ```

## In the IDE

1. **Add the edge to your project**. Click `+` next to "Edges" in the
   tree. Name it (free-form, no spaces), and point `Host` at your SSH
   alias. The IDE will run literally `ssh <host>`, so any
   `~/.ssh/config` entry works — including jump hosts, custom keys, etc.

2. **Wait for the probe**. The edge pane auto-probes every 10 s and on
   open. Green badge = the runtime's `/health` came back ok.

3. **Deploy**. Click `Deploy`. The IDE:
   - `tar`s your project directory + (if found) a freshly-built
     `controlsoftware-runtime` from the dev machine
   - Pipes the tar into `ssh edge bash …` which extracts to
     `$INSTALL_DIR/versions/<UTC-timestamp>/`, atomically swaps the
     `current` symlink, and `systemctl restart controlsoftware`s
   - Streams the remote script's output back into the pane

4. **Attach for live debugging**. Click `Attach`. The IDE opens an
   `ssh -N -L 127.0.0.1:<random>:127.0.0.1:<edge_runtime_port>` tunnel
   and switches the MonitorPane / VariablesPanel SSE source over to it.
   The same charts and pills you use locally now reflect the running
   program on the edge. Click `Detach` to go back to local mode.

## Layout on the edge

After deploy, an edge box looks like:

```
/opt/controlsoftware/
├── current → versions/2026-05-12T08-30-00Z/       (atomic symlink)
├── versions/
│   ├── 2026-05-12T08-30-00Z/       latest
│   │   ├── runtime                  binary
│   │   └── project/                 project.toml + applications/ + devices/ + iomap.toml
│   ├── 2026-05-12T07-15-00Z/       previous (kept for rollback)
│   └── _initial/                   install.sh stub
└── (state for retained variables would go here)
```

## Rollback

There's no Rollback button (yet). Manually:
```sh
ssh edge
sudo ls /opt/controlsoftware/versions/   # find the previous timestamp
sudo ln -sfn /opt/controlsoftware/versions/<prev> /opt/controlsoftware/.current.new
sudo mv -Tf /opt/controlsoftware/.current.new /opt/controlsoftware/current
sudo systemctl restart controlsoftware
```

The Deploy code uses the same symlink-swap recipe; doing it by hand for
rollback is just "point `current` at an older version and restart".

## Security notes

- The runtime binds **127.0.0.1** on the edge — only ever reachable via
  the SSH tunnel the IDE sets up. Don't poke a hole in the firewall to
  expose `:13001` directly.
- The systemd unit grants `CAP_NET_RAW` so that EtherCAT (when wired)
  works. If you only use Modbus, you can drop it and run as a dedicated
  user; see the comments in `controlsoftware.service`.
- Credentials are **not stored** in the project. The IDE's only auth
  mechanism is whatever `ssh` resolves via your agent / `~/.ssh/config`.
- Hardening is on (`PrivateTmp`, `ProtectSystem=strict`, `NoNewPrivileges`).
  If you find a legitimate access blocked, loosen carefully — these are
  there to limit what a misbehaving runtime can touch.

## Caveats

- **EtherCAT** is currently in simulation mode — the IDE will let you
  configure PDOs and the runtime will accept the config, but no real
  fieldbus traffic happens until `iomap-ethercat` is wired to the
  `ethercrab` MainDevice. Modbus TCP works end-to-end today.
- **Retained / persistent variables** aren't yet preserved across
  deploys. Each new `current` starts fresh.
- **Hot patch / online change** (Codesys-style in-place code update) is
  not implemented. Deploy is stop → swap → start. Plan downtime.
- **Real-time**: stock Linux gives soft-RT only; scan jitter is in the
  millisecond range. Acceptable for 10–100 ms cycles, not for sub-ms
  hard real-time control.
