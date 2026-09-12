#!/bin/bash
# Adapter: Claude Code (`claude` CLI), non-interactive.
#
# Contract with run.sh: stdout line 1 is `HARNESS_TOOL_VERSION: ...`;
# runs in the task workdir with HARNESS_PROMPT / HARNESS_SERVER_URL /
# HARNESS_TIMEOUT_SECS set; combined output becomes the transcript.
# Exit 3 means "blocked" (tool missing), not a task failure.
#
# WHY --dangerously-skip-permissions is acceptable HERE and only here:
# the harness runs the agent in an isolated throwaway workdir with a
# benign fixture, and `cs` on PATH is a shim pinned to the harness's
# loopback server — there is nothing of the user's to damage and no
# route to a real :3001 instance. Do not copy this flag elsewhere.

set -u

if ! command -v claude >/dev/null 2>&1; then
  echo "claude CLI not found on PATH — install Claude Code first" >&2
  echo "  (https://docs.claude.com/en/docs/claude-code) — blocked, not a task failure" >&2
  exit 3
fi

echo "HARNESS_TOOL_VERSION: $(claude --version 2>/dev/null | head -n 1)"

TIMEOUT_SECS="${HARNESS_TIMEOUT_SECS:-1200}"

# Time budget with whole-tree teardown: the tool runs as the leader of
# its own process group; on timeout the GROUP gets TERM, a short grace,
# then KILL, so children the tool spawned (bash commands, MCP servers)
# cannot outlive the budget or keep the workdir open. One perl
# implementation everywhere (macOS ships no GNU `timeout`) so the
# behavior is deterministic across machines. Exit: child status
# propagated; signal deaths map to 128+N (timeout => 143).
# stream-json + verbose: events land on the transcript incrementally,
# so a timeout kill still leaves the full trace (plain -p buffers its
# one text block and a killed run would leave an empty transcript).
#
# The stream is teed to a scratch copy so the RESOLVED MODEL can be
# extracted afterwards (audit F7: the CLI version alone does not pin
# which model answered — stream-json carries it on every assistant
# event). run.sh lifts the trailing HARNESS_RESOLVED_MODEL line into
# meta.json.
SCRATCH=$(mktemp "${TMPDIR:-/tmp}/claude-adapter.XXXXXX")
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
  claude -p "$(cat "$HARNESS_PROMPT")" --dangerously-skip-permissions \
  --output-format stream-json --verbose </dev/null 2>&1 | tee "$SCRATCH"
STATUS=${PIPESTATUS[0]}

# First assistant event names the model that actually answered. Extract
# from structured JSON (jq is already a harness prerequisite); absent —
# e.g. the CLI died before its first event — means no line is emitted
# and meta.json honestly records null.
MODEL=$(jq -Rrs '
  [split("\n")[] | fromjson? | select(.type == "assistant")
   | .message.model | select(type == "string" and length > 0)][0] // empty
' "$SCRATCH")
if [ -n "$MODEL" ]; then
  echo "HARNESS_RESOLVED_MODEL: $MODEL"
fi

exit "$STATUS"
