#!/bin/bash
# grade.sh <rundir> <task-id> — artifact-only, falsifiable grading.
#
# Re-derives the verdict from what is ON DISK in the run directory,
# never from transcript claims: it restarts an ISOLATED verification
# server (own temp HOME, port $HARNESS_VERIFY_PORT default 3902, demo
# Modbus slave disabled), re-executes the proof itself via the h_*
# helpers, then runs the task's expect.sh (tasks/<task-id>/expect.sh,
# resolved relative to this script).
#
# Inputs it needs from the rundir: workdir/ (the agent's cwd; created
# empty if absent so a null run grades honestly) and, optionally,
# artifacts/ (runner snapshots, e.g. forces.json) and integrity.json
# (the runner's pre-run trust-root snapshot; see the integrity check
# below — a mismatch blocks the run; an ABSENT snapshot also blocks
# unless HARNESS_ALLOW_UNATTESTED=1 explicitly marks a hand-built
# rundir, which then grades with an UNATTESTED integrity row).
#
# Output: <rundir>/verdict.json
#   {task, overall: "pass"|"fail"|"blocked",
#    checks: [{name, result, detail}],
#    claim_level: "executed"|"generated"|"diagnosed"|"honesty"}
#
# Exit codes: 0 = overall pass · 1 = overall fail · 3 = blocked
# (missing expect.sh, verification server unavailable, undecidable).
#
# bash 3.2 compatible; requires /usr/bin/jq, shasum, perl, and
# target/release/{server,cs} in the repo this script lives in.

set -u

usage() {
    echo "usage: grade.sh <rundir> <task-id>" >&2
    exit 3
}

[ $# -eq 2 ] || usage
RUNDIR="$1"
TASK_ID="$2"

# --- resolve our own location → repo root, tasks dir, binaries -----------
SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
HARNESS_DIR=$(cd "$SCRIPT_DIR/.." && pwd)
REPO_ROOT=$(cd "$HARNESS_DIR/../.." && pwd)
TASKS_DIR="$HARNESS_DIR/tasks"

GRADER_CS_BIN="$REPO_ROOT/target/release/cs"
GRADER_SERVER_BIN="$REPO_ROOT/target/release/server"
export GRADER_CS_BIN GRADER_SERVER_BIN

if [ ! -d "$RUNDIR" ]; then
    echo "grade: rundir does not exist: $RUNDIR" >&2
    exit 3
fi
RUNDIR=$(cd "$RUNDIR" && pwd)

if [ ! -x "$GRADER_CS_BIN" ] || [ ! -x "$GRADER_SERVER_BIN" ]; then
    echo "grade: target/release/{server,cs} not built — run:" >&2
    echo "  cargo build --release -p server -p ia2-cli" >&2
    exit 3
fi

# --- shared helper library ----------------------------------------------
# shellcheck source=common.sh
. "$SCRIPT_DIR/common.sh"

# --- claim level: pinned per task family by the harness contract ---------
claim_level_for() {
    case "$1" in
    t0-* | t1-*) echo "executed" ;;
    t2-*) echo "generated" ;;
    t3-*) echo "diagnosed" ;;
    t4-*) echo "honesty" ;;
    *) echo "executed" ;;
    esac
}
CLAIM_LEVEL="${HARNESS_CLAIM_LEVEL:-$(claim_level_for "$TASK_ID")}"

# --- verdict writer ------------------------------------------------------
# write_verdict <overall> — assembles verdict.json from the rows file.
write_verdict() {
    local overall="$1"
    jq -s --arg task "$TASK_ID" --arg overall "$overall" --arg claim "$CLAIM_LEVEL" \
        '{task: $task, overall: $overall, checks: ., claim_level: $claim}' \
        "$HARNESS_CHECKS_FILE" >"$RUNDIR/verdict.json"
}

# --- blocked: task's expect.sh must exist (concurrent-build safe) --------
EXPECT_SH="$TASKS_DIR/$TASK_ID/expect.sh"
HARNESS_CHECKS_FILE=$(mktemp "${TMPDIR:-/tmp}/ia2-grader-checks.XXXXXX")
export HARNESS_CHECKS_FILE

if [ ! -f "$EXPECT_SH" ]; then
    grader_record "expect_script" blocked \
        "tasks/$TASK_ID/expect.sh not found — task not built yet (or unknown task id)"
    write_verdict blocked
    rm -f "$HARNESS_CHECKS_FILE"
    echo "grade: BLOCKED — tasks/$TASK_ID/expect.sh does not exist; cannot grade this run" >&2
    exit 3
fi

# --- integrity: the grading trust root must match the pre-run snapshot ---
# run.sh writes rundir/integrity.json BEFORE launching the agent. We
# recompute all five hashes here with the SAME recipes (keep in
# lockstep with run.sh section 4b):
#   single file  : shasum -a 256 <file>
#   grader pair  : cat grade.sh common.sh (that order) | shasum -a 256
#   fixture tree : sha256 of the sorted per-file `sha  ./relpath`
#                  listing; empty string when the task ships no fixture/
# ANY mismatch means expect.sh, the grader, the fixture, or the
# binaries changed while the agent had the machine — a tampered trust
# root must never grade pass/fail, so the run is BLOCKED. Absence is
# handled below: blocked by default (the runner always attests), or an
# explicit UNATTESTED pass row under HARNESS_ALLOW_UNATTESTED=1 for
# hand-built rundirs (selftest, archaeology).
grader_sha256_file() {
    shasum -a 256 "$1" | awk '{print $1}'
}
grader_tree_sha256() {
    (cd "$1" && find . -type f | LC_ALL=C sort \
        | while IFS= read -r tf; do shasum -a 256 "$tf"; done \
        | shasum -a 256 | awk '{print $1}')
}
INTEGRITY_JSON="$RUNDIR/integrity.json"
if [ ! -f "$INTEGRITY_JSON" ]; then
    # Audit F4: an absent snapshot must not silently look attested.
    # run.sh ALWAYS writes one, so a normal runner rundir without it is
    # suspect — blocked. Hand-built / legacy / selftest rundirs opt in
    # EXPLICITLY and are labelled unattested, never plain "pass".
    if [ "${HARNESS_ALLOW_UNATTESTED:-0}" = "1" ]; then
        grader_record "integrity" pass \
            "UNATTESTED (explicit HARNESS_ALLOW_UNATTESTED=1): no integrity snapshot — pre-run hashes not attested; rundir graded on its merits"
    else
        grader_record "integrity" blocked \
            "no integrity snapshot (integrity.json absent) — runner rundirs are always attested; for a hand-built rundir set HARNESS_ALLOW_UNATTESTED=1"
        write_verdict blocked
        rm -f "$HARNESS_CHECKS_FILE"
        echo "grade: BLOCKED — no integrity snapshot and no HARNESS_ALLOW_UNATTESTED=1 opt-in" >&2
        exit 3
    fi
else
    NOW_EXPECT=$(grader_sha256_file "$EXPECT_SH")
    NOW_GRADER=$(cat "$SCRIPT_DIR/grade.sh" "$SCRIPT_DIR/common.sh" | shasum -a 256 | awk '{print $1}')
    NOW_FIXTURE=""
    if [ -d "$TASKS_DIR/$TASK_ID/fixture" ]; then
        NOW_FIXTURE=$(grader_tree_sha256 "$TASKS_DIR/$TASK_ID/fixture")
    fi
    NOW_CS=$(grader_sha256_file "$GRADER_CS_BIN")
    NOW_SERVER=$(grader_sha256_file "$GRADER_SERVER_BIN")

    INTEGRITY_MISMATCH=""
    for pair in \
        "expect_sha256=$NOW_EXPECT" \
        "grader_sha256=$NOW_GRADER" \
        "fixture_sha256=$NOW_FIXTURE" \
        "cs_sha256=$NOW_CS" \
        "server_sha256=$NOW_SERVER"; do
        key=${pair%%=*}
        now=${pair#*=}
        recorded=$(jq -r --arg k "$key" '.[$k] // ""' "$INTEGRITY_JSON" 2>/dev/null)
        if [ "$recorded" != "$now" ]; then
            INTEGRITY_MISMATCH="$INTEGRITY_MISMATCH $key"
        fi
    done
    if [ -n "$INTEGRITY_MISMATCH" ]; then
        grader_record "integrity" blocked \
            "grading trust root changed since the pre-run snapshot:$INTEGRITY_MISMATCH — refusing to grade (tampering, or a mid-run rebuild/edit; re-run to re-snapshot)"
        write_verdict blocked
        rm -f "$HARNESS_CHECKS_FILE"
        echo "grade: BLOCKED — integrity mismatch:$INTEGRITY_MISMATCH (see integrity.json vs current tree)" >&2
        exit 3
    fi
    grader_record "integrity" pass \
        "all five sha256 pins match the pre-run snapshot (expect.sh, grader, fixture, cs, server)"
fi

# --- rundir layout -------------------------------------------------------
HARNESS_RUNDIR="$RUNDIR"
HARNESS_WORKDIR="$RUNDIR/workdir"
HARNESS_ARTIFACTS="$RUNDIR/artifacts"
HARNESS_HOME="$RUNDIR/home"
HARNESS_TASK_ID="$TASK_ID"
export HARNESS_RUNDIR HARNESS_WORKDIR HARNESS_ARTIFACTS HARNESS_HOME HARNESS_TASK_ID
# Compatibility aliases: task expect.sh scripts may address the rundir
# through the short names as well; keep both sets pointing at the same
# places.
RUNDIR="$HARNESS_RUNDIR"
WORKDIR="$HARNESS_WORKDIR"
ARTIFACTS="$HARNESS_ARTIFACTS"
export RUNDIR WORKDIR ARTIFACTS
# A null run may lack workdir/ entirely — grade it honestly, don't crash.
mkdir -p "$HARNESS_WORKDIR"

# --- isolated verification server ---------------------------------------
GRADER_VERIFY_LOG="$RUNDIR/grader-verify-server.log"
export GRADER_VERIFY_LOG

cleanup() {
    grader_stop_verify_server
    rm -f "$HARNESS_CHECKS_FILE"
}
# EXIT only (same pattern as run.sh): bash runs the EXIT trap on a
# fatal INT/TERM too. Trapping the signals themselves would resume the
# script after the handler with its state (checks file, verify server)
# already destroyed — and a resumed aggregation over a deleted checks
# file can emit a WRONG verdict.
trap cleanup EXIT

if ! grader_start_verify_server; then
    grader_record "verify_server" blocked \
        "verification server failed to start on \$HARNESS_VERIFY_PORT (${HARNESS_VERIFY_PORT:-3902}) — see grader-verify-server.log"
    write_verdict blocked
    echo "grade: BLOCKED — verification server unavailable" >&2
    exit 3
fi

# --- run the task's expectations ----------------------------------------
echo "grade: task $TASK_ID, rundir $RUNDIR" >&2
N_SETUP=$(jq -s 'length' "$HARNESS_CHECKS_FILE")
# Subshell so an `exit` inside expect.sh cannot kill the grader; helpers
# persist their rows through $HARNESS_CHECKS_FILE. set +e so every check
# runs — the verdict aggregates rows, it does not stop at first failure.
(
    set +e
    cd "$HARNESS_WORKDIR" || exit 1
    # shellcheck source=common.sh
    . "$SCRIPT_DIR/common.sh"
    # shellcheck disable=SC1090
    . "$EXPECT_SH"
)
EXPECT_RC=$?

# --- aggregate -----------------------------------------------------------
N_TOTAL=$(jq -s 'length' "$HARNESS_CHECKS_FILE")
N_FAIL=$(jq -s '[.[] | select(.result == "fail")] | length' "$HARNESS_CHECKS_FILE")
N_BLOCKED=$(jq -s '[.[] | select(.result == "blocked")] | length' "$HARNESS_CHECKS_FILE")

if [ "$EXPECT_RC" -ne 0 ] && [ "$N_FAIL" -eq 0 ]; then
    # expect.sh died without recording a failure — the verdict cannot be
    # trusted as a pass; surface it instead of swallowing it.
    grader_record "expect_script" blocked "expect.sh exited $EXPECT_RC before completing its checks"
    N_TOTAL=$((N_TOTAL + 1))
    N_BLOCKED=$((N_BLOCKED + 1))
fi

if [ "$N_TOTAL" -eq "$N_SETUP" ]; then
    grader_record "expect_script" blocked "expect.sh recorded no checks — nothing to grade"
    N_TOTAL=$((N_TOTAL + 1))
    N_BLOCKED=$((N_BLOCKED + 1))
fi

if [ "$N_FAIL" -gt 0 ]; then
    OVERALL=fail
elif [ "$N_BLOCKED" -gt 0 ]; then
    OVERALL=blocked
else
    OVERALL=pass
fi

write_verdict "$OVERALL"

echo "grade: $TASK_ID → $OVERALL ($((N_TOTAL - N_FAIL - N_BLOCKED)) pass, $N_FAIL fail, $N_BLOCKED blocked; claim_level=$CLAIM_LEVEL)" >&2
echo "grade: verdict written to $RUNDIR/verdict.json" >&2

case "$OVERALL" in
pass) exit 0 ;;
fail) exit 1 ;;
*) exit 3 ;;
esac
