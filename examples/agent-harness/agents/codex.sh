#!/bin/bash
# Adapter: OpenAI Codex CLI (`codex`), non-interactive.
#
# Contract with run.sh: stdout line 1 is `HARNESS_TOOL_VERSION: ...`;
# runs in the task workdir with HARNESS_PROMPT / HARNESS_SERVER_URL /
# HARNESS_TIMEOUT_SECS set; combined output becomes the transcript.
# Exit 3 means "blocked" (tool missing), not a task failure.
#
# Invocation verified against codex-cli 0.153.4 (2026-09-08): `codex
# exec` takes the prompt positionally, `--skip-git-repo-check` is
# required because the workdir is a bare mktemp dir, and the run is
# confined by Codex's OWN sandbox rather than bypassing it.
#
# WHY `-s workspace-write` and not the sandbox bypass: Codex's
# workspace-write policy already grants exactly what a task needs —
# writes under the workdir plus /tmp and $TMPDIR, which is where run.sh
# puts the rundir — so the bypass buys nothing and gives up the
# containment. Network access is off by default under that policy and
# must be re-enabled, or the PATH-shimmed `cs` cannot reach the
# harness's loopback server and every task fails for the wrong reason.
#
# stdin is closed: with a positional prompt, `codex exec` still appends
# piped stdin as a `<stdin>` block, so an inherited pipe would silently
# corrupt the task prompt.

set -u

if ! command -v codex >/dev/null 2>&1; then
  echo "codex CLI not found on PATH — install it first" >&2
  echo "  (npm install -g @openai/codex) — blocked, not a task failure" >&2
  exit 3
fi

echo "HARNESS_TOOL_VERSION: $(codex --version 2>/dev/null | head -n 1)"

TIMEOUT_SECS="${HARNESS_TIMEOUT_SECS:-1200}"

# Time budget with whole-tree teardown: the tool runs as the leader of
# its own process group; on timeout the GROUP gets TERM, a short grace,
# then KILL, so children the tool spawned cannot outlive the budget or
# keep the workdir open. One perl implementation everywhere (macOS
# ships no GNU `timeout`) so the behavior is deterministic across
# machines. Exit: child status propagated; signal deaths map to 128+N
# (timeout => 143).
#
# The output is teed to a scratch copy so the exit status can be
# classified afterwards (see the blocked check below); run.sh still sees
# the identical stream on stdout and records it as the transcript.
SCRATCH=$(mktemp "${TMPDIR:-/tmp}/codex-adapter.XXXXXX")
trap 'rm -f "$SCRATCH"' EXIT

perl -e '
  my $secs = shift @ARGV;
  my $pid  = fork;
  die "fork failed: $!\n" unless defined $pid;
  if ($pid == 0) {
    setpgrp(0, 0);                 # own group => timeout kills the tree
    exec @ARGV or die "exec failed: $!\n";
  }
  $SIG{ALRM} = sub {
    kill "TERM", -$pid;            # polite stop for the whole group
    sleep 5;                       # grace for orderly child shutdown
    kill "KILL", -$pid;            # hard stop for anything left
  };
  alarm $secs;
  my $r;
  for (;;) {
    $r = waitpid($pid, 0);
    last if $r == $pid or ($r == -1 and not $!{EINTR});
  }
  my $st = $?;
  exit(128 + ($st & 127)) if $st & 127;
  exit($st >> 8);
' "$TIMEOUT_SECS" \
  codex exec --json --skip-git-repo-check \
  -s workspace-write -c sandbox_workspace_write.network_access=true \
  "$(cat "$HARNESS_PROMPT")" </dev/null 2>&1 | tee "$SCRATCH"
STATUS=${PIPESTATUS[0]}

# A CLI that refused to run is BLOCKED, not a failed task. Codex exits 1
# with a usage/rate-limit banner and an empty session when the account
# has no credits left; without this check the grader sees a workdir with
# no RESULT.md and records "the model failed the task", which is a lie
# about a run that never happened. Keyed on a non-zero exit AND the
# structured error event, never a keyword in model text or command output.
if [ "$STATUS" -ne 0 ] \
   && jq -Rse '
     [split("\n")[] | fromjson?
      | select(.type == "error" or .type == "turn.failed")
      | (.message // .error.message // "")
      | select(type == "string")
      | test("hit your usage limit|usage limit reached|rate limit|quota exceeded|429 too many requests"; "i")]
     | any
   ' "$SCRATCH" >/dev/null 2>&1; then
  echo "codex refused to run: account usage/rate limit reached — blocked, not a task failure" >&2
  exit 3
fi

exit "$STATUS"
