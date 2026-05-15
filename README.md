# controlsoftware

Web IDE + runtime for IEC 61131-3 (ST/FB) PLC programming.

- Compile ST → bytecode via vendored [ironplc](https://github.com/ironplc/ironplc), execute on its VM.
- I/O via EtherCAT ([ethercrab](https://crates.io/crates/ethercrab)) and Modbus ([tokio-modbus](https://crates.io/crates/tokio-modbus)).
- AI agents integrate through MCP (Claude Code / Codex / Cursor as clients).

## Layout

```
apps/web              TanStack Start IDE (React + shadcn + Tailwind)
crates/cli            `cs` CLI — agent-first static analysis & project tools
crates/server         axum backend: project mgmt, compile orchestration, runtime control
crates/runtime        PLC scan loop, I/O image, task scheduler
crates/iomap-modbus   tokio-modbus adapter
crates/iomap-ethercat ethercrab adapter
crates/ironplc-bridge wraps vendored ironplc parser+codegen+vm
packages/ui           shadcn primitives (per-app config still in apps/web)
packages/api-types    TS types generated from Rust via ts-rs
vendor/ironplc        git submodule (later)
```

## Dev

```bash
# one-time
. "$HOME/.cargo/env"
pnpm install
cargo test -p server   # populates apps/web/src/types/generated/ via ts-rs

# two terminals (orchestration via moon coming later)
pnpm --filter @cs/web dev    # http://localhost:3000
cargo run -p server          # http://localhost:3001
```

## `cs` CLI

Agent-first command-line interface. Static analysis, transpile, and
project inspection — everything you'd want to do **before** the
runtime starts. Online operations (live values, Run/Stop, attach) stay
on the HTTP API.

```bash
cargo build -p controlsoftware-cli              # builds target/debug/cs
alias cs=./target/debug/cs                       # or copy to ~/bin

# validate a POU (auto-detects .st vs .ld.json)
cs check pous/safe_start.ld.json                # human output on stderr
cs check pous/safe_start.ld.json --json         # machine output on stdout

# show the ST that an LD POU compiles to (useful for debugging)
cs transpile pous/safe_start.ld.json            # ST text
cs transpile pous/safe_start.ld.json --with-map # JSON { st, source_map }

# project-level
cs project info  /path/to/project               # POUs / devices / edges
cs project check /path/to/project               # full compile check
```

Exit codes: `0` clean / `1` source has errors / `2` usage / `≥3`
infrastructure. Every subcommand's `--help` describes when to use it
and what to call next — `cs --help`, `cs check --help`, etc.

See `MEMORY/principles.md` § "CLI is the headline agent interface"
for the rationale.
