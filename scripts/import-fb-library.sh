#!/usr/bin/env bash
#
# import-fb-library.sh — import process-control FB blocks into a project
# via the server's library API (the same path the IDE's import dialog
# uses). Blocks land in `pous/lib/process-control/` (read-only via the
# generic POU routes) and the import is recorded in project.toml's
# `[libraries]` table.
#
# Usage:
#   scripts/import-fb-library.sh <project-name-or-dir> [fb_name ...]
#
#   # whole library, project already open in the server:
#   scripts/import-fb-library.sh my_line
#   # open-by-path, then import just a few blocks:
#   scripts/import-fb-library.sh ~/Documents/IA2/my_line fb_pid fb_alarm_hl
#
# Server URL defaults to http://127.0.0.1:3001 (IA2_SERVER_URL overrides).
# Equivalent raw API, for agents:
#   GET    /api/library                  — registry + imported state
#   POST   /api/library/import           — {"library":"…","blocks":["fb_pid.st",…]}
#   DELETE /api/library/<name>           — remove from project

set -euo pipefail

SERVER="${IA2_SERVER_URL:-http://127.0.0.1:3001}"
LIBRARY="process-control"
PROJ="${1:-}"

[ -n "$PROJ" ] || { echo "usage: $0 <project-name-or-dir> [fb_name ...]" >&2; exit 2; }
shift || true

curl -sf "$SERVER/api/health" >/dev/null || {
  echo "ERROR: no IA2 server at $SERVER — start it (or set IA2_SERVER_URL)." >&2
  exit 2
}

# A directory argument means "open that project first" (idempotent),
# then address it by name like any other client.
if [ -d "$PROJ" ]; then
  curl -sf -X POST "$SERVER/api/projects/open" \
    -H "Content-Type: application/json" \
    -d "{\"path\":\"$PROJ\"}" >/dev/null || {
      echo "ERROR: could not open project at $PROJ" >&2; exit 2; }
  PROJ="$(basename "$PROJ")"
fi

blocks_json="[]"
if [ "$#" -gt 0 ]; then
  blocks_json="$(printf '%s\n' "$@" | sed -E 's/(\.st)?$/.st/' \
    | python3 -c 'import json,sys; print(json.dumps([l.strip() for l in sys.stdin if l.strip()]))')"
fi

resp="$(curl -sf -X POST "$SERVER/api/library/import" \
  -H "Content-Type: application/json" \
  -H "X-IA2-Project: $PROJ" \
  -d "{\"library\":\"$LIBRARY\",\"blocks\":$blocks_json}")" || {
    echo "ERROR: import failed — is project '$PROJ' open in the server?" >&2
    echo "       check: curl -s $SERVER/api/library -H 'X-IA2-Project: $PROJ'" >&2
    exit 2
  }

echo "$resp" | python3 -c '
import json, sys
r = json.load(sys.stdin)
print(f"imported {len(r[\"imported\"])} block(s) from {r[\"library\"]}@{r[\"version\"]}:")
for f in r["imported"]:
    print(f"  + pous/lib/{r[\"library\"]}/{f}")
'
echo "Blocks now show under the Libraries section and the FBD/LD '+ Block' palette."
