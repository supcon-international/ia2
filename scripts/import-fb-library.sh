#!/usr/bin/env bash
#
# import-fb-library.sh — copy the process-control FB library into a
# project so its blocks are usable (in ST, and in the FBD/LD palette).
#
# Usage:
#   scripts/import-fb-library.sh <project-dir> [fb_name ...]
#
#   # whole library:
#   scripts/import-fb-library.sh ~/Documents/IA2/my_line
#   # just a few:
#   scripts/import-fb-library.sh ~/Documents/IA2/my_line fb_pid fb_alarm_hl
#
# The graphical editors discover FUNCTION_BLOCKs from the open project,
# so after importing, reopen/refresh the project and the blocks appear
# in the "+ Block" palette. (demo_main.st is skipped — it's a demo
# PROGRAM, not a library block.)

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
LIB="$REPO_ROOT/library/process-control/pous"
PROJ="${1:-}"

[ -n "$PROJ" ] || { echo "usage: $0 <project-dir> [fb_name ...]" >&2; exit 2; }
[ -d "$LIB" ] || { echo "library not found at $LIB" >&2; exit 2; }
[ -f "$PROJ/project.toml" ] || {
  echo "ERROR: $PROJ is not an IA2 project (no project.toml)." >&2
  echo "       Point at a project dir, e.g. ~/Documents/IA2/<name>." >&2
  exit 2
}

mkdir -p "$PROJ/pous"
shift || true

if [ "$#" -gt 0 ]; then
  files=()
  for n in "$@"; do
    f="$LIB/${n%.st}.st"
    [ -f "$f" ] || { echo "no such library block: $n" >&2; exit 2; }
    files+=("$f")
  done
else
  files=("$LIB"/fb_*.st)   # every FB; demo_main.st deliberately excluded
fi

n=0
for f in "${files[@]}"; do
  base="$(basename "$f")"
  cp "$f" "$PROJ/pous/$base"
  echo "  + $base"
  n=$((n + 1))
done
echo "imported $n function block(s) into $PROJ/pous/"
echo "Reopen the project in the IDE (or it'll refresh on next tree load);"
echo "the blocks then show under '+ Block' in the FBD/LD editors."
