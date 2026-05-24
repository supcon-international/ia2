# First contact checklist

Run this the first time you touch IA2 in a session, before any real work. Three unknowns to resolve: where's the `cs` binary, where's the server, and what's open.

## 1. Locate the `cs` binary

```bash
command -v cs || ls /Users/mercy/codebase/controller/target/release/cs
```
- On `$PATH` → use `cs`.
- Only in the checkout → use the full path, or `cargo build -p ia2-cli --release` first if it's missing/stale.
- Set `CS=` to whichever you found so every later command is unambiguous.

## 2. Discover the server URL

`cs` defaults to `http://127.0.0.1:3001`. That's right for a manually-started dev server (`cargo run -p server`) but **wrong for `IA2.app`**, which binds an ephemeral port. Resolve it:

```bash
# Is a plain dev server on the default port?
curl -sf -m 1 http://127.0.0.1:3001/api/health >/dev/null && SRV=http://127.0.0.1:3001

# Otherwise scan for the IA2.app server (ephemeral, high port). This is
# slow but reliable; stop at the first /api/health that says ok.
if [ -z "$SRV" ]; then
  for p in $(seq 50000 65535); do
    if curl -sf -m 0.1 "http://127.0.0.1:$p/api/health" 2>/dev/null | grep -q '"status":"ok"'; then
      SRV="http://127.0.0.1:$p"; break
    fi
  done
fi
echo "SRV=$SRV"
```

If the scan finds nothing, the app/server isn't running. Either the user needs to launch `IA2.app` (`open /Applications/IA2.app`) or you start a dev server. Don't proceed without a reachable `/api/health`.

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
