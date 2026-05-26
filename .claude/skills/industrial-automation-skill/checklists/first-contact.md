# First contact checklist

Run this the first time you touch IA2 in a session, before any real work. Three unknowns to resolve: where's the `cs` binary, where's the server, and what's open.

## 1. Locate the `cs` binary

```bash
command -v cs && cs --version     # the installer puts cs in ~/.local/bin (on PATH)
ls ./target/release/cs            # ...or it's the build output, if you're in a checkout
```
- On `$PATH` → use `cs`.
- In a checkout but no binary → `cargo build -p ia2-cli --release`, then `./target/release/cs`.
- Skill installed standalone (via `npx skills`), no checkout → clone + build the binaries once:
  `git clone --recursive https://github.com/supcon-international/ia2 && cd ia2 && ./scripts/install-skill.sh` (puts `cs` + `ia2-server` in `~/.local/bin`).
- Set `CS=` to whichever you found so every later command is unambiguous.

## 2. Discover the server URL

`cs` defaults to `http://127.0.0.1:3001`. That's right for a manually-started dev server (`cargo run -p server`) but **wrong for `IA2.app`**, which binds an ephemeral port. Resolve it:

```bash
# 1. Plain dev server on the default port?
curl -sf -m 1 http://127.0.0.1:3001/api/health >/dev/null && SRV=http://127.0.0.1:3001

# 2. IA2.app binds an OS-assigned ephemeral port (macOS: 49152-65535).
#    Find it with lsof (`+c0` so the command name isn't truncated to
#    "ia2-serve"), then confirm /api/health — that also skips the
#    demo-modbus listener the same process may hold:
if [ -z "$SRV" ]; then
  for p in $(lsof -nP -iTCP -sTCP:LISTEN +c0 2>/dev/null | grep ia2-server | grep -oE '127\.0\.0\.1:[0-9]+' | cut -d: -f2); do
    curl -sf -m 1 "http://127.0.0.1:$p/api/health" 2>/dev/null | grep -q '"status":"ok"' && { SRV="http://127.0.0.1:$p"; break; }
  done
fi

# 3. Fallback: scan the ephemeral range (slow; macOS starts at 49152, not 50000).
if [ -z "$SRV" ]; then
  for p in $(seq 49152 65535); do
    if curl -sf -m 0.1 "http://127.0.0.1:$p/api/health" 2>/dev/null | grep -q '"status":"ok"'; then
      SRV="http://127.0.0.1:$p"; break
    fi
  done
fi
echo "SRV=$SRV"
```

If the scan finds nothing, no server is running. Start one — headless: `ia2-server --bind 127.0.0.1:3001 &` then `SRV=http://127.0.0.1:3001`; or from a checkout `cargo run -p server`; or have the user launch `IA2.app` (`open /Applications/IA2.app`). Don't proceed without a reachable `/api/health`.

> Tip: some sessions persist the URL in `/tmp/ia2_srv`. Check there first: `SRV=$(cat /tmp/ia2_srv 2>/dev/null)` then validate it with a health probe before trusting it.

## 3. See what's open

```bash
cs project list --server "$SRV"
```
- **Zero projects** → you'll `cs project create` or `cs project open` as the first real step.
- **One project** → you can omit `--project` on later commands (the active fallback is correct).
- **Two or more** → you **must** pass `--project NAME` on every command. Note which one the user's IDE window is showing (its URL `?project=`), and target that one, or confirm with the user.

## 4. If you're about to do multi-step work

Stop and set up a session wrapper (`03-agent-sessions.md`). Don't fire commands one at a time — the overlay strobes. Draft the whole sequence, then:

```bash
cs agent run --label "<what you're about to do>" --server "$SRV" -- bash -c '... whole workflow ...'
```

## Ready check

You're ready to work when all of these are true:
- [ ] `CS` points at a real binary
- [ ] `SRV` answers `/api/health` with `"status":"ok"`
- [ ] You know how many projects are open and which to target
- [ ] Multi-step work is wrapped in `cs agent run`
