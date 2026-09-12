#!/bin/bash
# Deterministic adapter tests: no model call, account, or network required.
set -eu
HERE=$(cd "$(dirname "$0")" && pwd)
WORK=$(mktemp -d "${TMPDIR:-/tmp}/ia2-adapter-tests.XXXXXX")
trap 'rm -rf "$WORK"' EXIT
cp "$HERE/test-fixtures/fake-cli.sh" "$WORK/codex"
cp "$HERE/test-fixtures/fake-cli.sh" "$WORK/claude"
chmod +x "$WORK/codex" "$WORK/claude"
export PATH="$WORK:$PATH" HARNESS_PROMPT="$HERE/selftest.sh" HARNESS_TIMEOUT_SECS=10
export FAKE_EVENTS FAKE_STATUS
check() {
  local adapter=$1 expected=$2 marker=$3 status=0 output
  output=$(printf 'inherited stdin\n' | bash "$HERE/$adapter.sh" 2>&1) || status=$?
  if [ "$status" -ne "$expected" ]; then
    printf 'FAIL %s: expected exit %s, got %s\n%s\n' "$adapter" "$expected" "$status" "$output"
    exit 1
  fi
  if [ -n "$marker" ]; then
    printf '%s\n' "$output" | grep -Fx "$marker" >/dev/null
  elif printf '%s\n' "$output" | grep -q '^HARNESS_RESOLVED_MODEL:'; then
    echo 'FAIL: model reported without assistant event'; exit 1
  fi
}
FAKE_STATUS=1
FAKE_EVENTS='{"type":"error","message":"You have hit your usage limit"}'
check codex 3 ''
FAKE_EVENTS='{"type":"turn.failed","error":{"message":"429 Too Many Requests"}}'
check codex 3 ''
FAKE_EVENTS='{"type":"item.completed","item":{"type":"agent_message","text":"rate limit test failed"}}'
check codex 1 ''
FAKE_EVENTS='{"type":"item.completed","item":{"type":"command_execution","aggregated_output":"quota exceeded"}}'
check codex 1 ''
FAKE_EVENTS='{"type":"error","message":"compilation failed"}'
check codex 1 ''
FAKE_STATUS=0
FAKE_EVENTS='{"type":"error","message":"rate limit; retrying"}'
check codex 0 ''
FAKE_EVENTS='{"type":"system","model":"configured-not-resolved"}
{"type":"assistant", "message": {"model": "actual-model", "content": []}}'
check claude-code 0 'HARNESS_RESOLVED_MODEL: actual-model'
FAKE_EVENTS='{"type":"user","message":{"model":"untrusted-model"}}'
check claude-code 0 ''
echo '8 adapter regression tests passed'
