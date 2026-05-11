# controlsoftware

Web IDE + runtime for IEC 61131-3 (ST/FB) PLC programming.

- Compile ST → bytecode via vendored [ironplc](https://github.com/ironplc/ironplc), execute on its VM.
- I/O via EtherCAT ([ethercrab](https://crates.io/crates/ethercrab)) and Modbus ([tokio-modbus](https://crates.io/crates/tokio-modbus)).
- AI agents integrate through MCP (Claude Code / Codex / Cursor as clients).

## Layout

```
apps/web            TanStack Start IDE (React + shadcn + Tailwind)
crates/server       axum backend: project mgmt, compile orchestration, runtime control
crates/runtime      PLC scan loop, I/O image, task scheduler
crates/iomap-modbus tokio-modbus adapter
crates/iomap-ethercat ethercrab adapter
crates/ironplc-bridge wraps vendored ironplc parser+codegen+vm
packages/ui         shadcn primitives (per-app config still in apps/web)
packages/api-types  TS types generated from Rust via ts-rs
vendor/ironplc      git submodule (later)
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
