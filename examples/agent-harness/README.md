# Agent harness

Reproducible evidence that **any** coding agent can drive the full IA2
skill workflow offline: orient → author IEC 61131-3 logic → `cs check`
→ `cs project check` → `cs sim run` proof → honest reporting. This is
IA2's own replayable benchmark — it is **not** a comparison against any
other product. Two claim levels are graded and reported separately:
*executed a guided procedure* (t1) versus *independently generated /
diagnosed* (t2/t3). Hardware is out of scope: sim adapters only, and
the grader enforces that.

## Quickstart

Requires Python 3.11+ (`tomllib`) on PATH in addition to bash, jq and
Perl. The grader parses device TOML and refuses non-sim devices before
starting a scenario; a missing parser blocks grading.

```bash
# 1. build the binaries the harness runs against
cargo build --release -p server -p ia2-cli

# 2. run one agent against one task
cd examples/agent-harness
./run.sh claude-code t1-guided
```

`run.sh --help` prints the full usage. Exit code: `0` verdict pass ·
`1` verdict fail · `2` usage error · `3` infrastructure/blocked
(missing binaries, busy port, adapter tool not installed, task or
grader not present yet).

## What one run does

`run.sh <agent> <task-id>` builds a throwaway run directory
(`mktemp -d`) holding `workdir/` (the agent's cwd: task `prompt.md`
with its literal server URL rewritten to this run's real port, a copy
of the repo skill, an `AGENTS.md` pointer, any task fixture as
`project/`), `home/` (the server's isolated `HOME`), `transcript.txt`,
`meta.json`, and `artifacts/`. Before the agent starts it writes
`integrity.json` — sha256 of the task's `expect.sh`, the grader pair
(`grade.sh` + `common.sh`), the fixture tree, and the `cs`/`server`
binaries — the grader's tamper-evidence baseline. It starts an
isolated IA2 server on `127.0.0.1:3901` (`HARNESS_PORT` overrides),
puts a `cs` shim on the agent's PATH that pins `--server` to that
port, runs `agents/<agent>.sh` under a time budget
(`HARNESS_TIMEOUT_SECS`, default 1200 s; on expiry the adapter TERMs
the tool's whole process group, waits a short grace, then KILLs it),
snapshots runtime status and force state, tears the server down,
scrubs the transcript (token-shaped strings and the user's home
path), then hands the run dir to `grader/grade.sh` — whose
verification server defaults to `HARNESS_PORT`+1
(`HARNESS_VERIFY_PORT` overrides) — and maps the resulting
`verdict.json` to its own exit code.

The run dir is kept by default (path printed); `--clean` deletes it,
`--keep` makes the default explicit.

## Tasks and claim levels

| Task | Claim level |
|---|---|
| `t0-discovery` | executed — orient against a live server |
| `t1-guided` | executed — guided project + scenario + full gate |
| `t2-spec` | generated — logic from a natural-language spec |
| `t3-debug` | diagnosed — find and fix one planted bug |
| `t4-honesty` | honesty — report an unsatisfiable scenario truthfully |

Grading is artifact-only and independent: the grader restarts its own
verification server and re-executes the proofs; transcript claims are
never trusted. Before grading anything it recomputes the five
`integrity.json` hashes (expect script, grader pair, fixture tree,
`cs` and `server` binaries) and refuses to grade — `overall` becomes
`blocked` — if any grading input changed during the agent's turn: a
tampered trust root must never grade pass/fail.

## Adapters

`agents/<name>.sh` is the whole interface: run in the task workdir with
`HARNESS_PROMPT`, `HARNESS_SERVER_URL`, `HARNESS_TIMEOUT_SECS` set,
print `HARNESS_TOOL_VERSION: <version>` as stdout line 1, then drive
the tool non-interactively; combined output becomes the transcript.
Exit `3` means *blocked* — the tool was not installed, or was installed
but refused to run at all (an exhausted account quota is the common
case); the runner reports it as infrastructure, not as a task failure.
An adapter that lets that condition through returns an empty workdir
and the grader records "the model failed the task" about a run that
never happened. Shipping adapters:
`claude-code.sh` (Claude Code) and `codex.sh` (Codex CLI, invocation
verified against `codex-cli 0.153.4` on 2026-09-08 — it runs under
Codex's own `workspace-write` sandbox with network access re-enabled,
not the sandbox bypass, because that policy already covers the workdir
and `$TMPDIR` where the rundir lives). To add an agent, copy one of
them.

## Run records and publishing (`runs/`)

Every completed run copies its shareable subset — scrubbed
`transcript.txt`, `meta.json`, `verdict.json`, `artifacts/` (never
`workdir/` or `home/`) — into `runs/<utc-stamp>-<agent>-<task>-<pid>/`.
The whole `runs/` directory is gitignored: records accumulate locally
as your private evidence base, and publishing any transcript anywhere
is a separate, deliberate human decision — review it first even though
the scrubber already masked token-shaped strings and home paths.

## Honest limits

- The isolation is temp-dir mechanics only — the harness cannot see or
  control what is installed on the user's machine.
- Only `codex.sh` classifies a quota-exhausted CLI as *blocked*;
  `claude-code.sh` has the same hole and is unfixed here, because no
  run has yet exercised it and the exact banner Claude Code prints on
  refusal has not been observed — guessing at it would be the kind of
  unverified claim this harness exists to prevent.
- `claude -p` and `codex exec` both inherit the user's account-level
  configuration, including which model answers; the harness records the
  CLI version in `meta.json` but has no model field and does not
  control that config. Two runs of the same adapter on different days
  are not guaranteed to be the same model.
- The PATH `cs` shim is a rail, not a jail — a determined agent could
  still construct its own URLs; the shim only removes the accidental
  route to a real `:3001` server.
- Leftover agent-spawned processes are group-killed on timeout, but a
  cooperative agent that daemonizes into its own session can outlive
  its run — the harness is a benchmark, not a security boundary.
- The agent runs as the local user and can read the repo, so task
  materials avoid spoilers but cannot be secrets; integrity hashing
  makes tampering with the grading inputs visible, not impossible.
- Grading keys on the prescribed project/variable/scenario names, so a
  correct-but-renamed solution fails — by design, for determinism.
- `runs/` is gitignored: capturing a run is automatic, publishing any
  transcript is a separate human decision.
