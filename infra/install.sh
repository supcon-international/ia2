#!/usr/bin/env bash
#
# One-time edge bootstrap. Run this on a fresh edge Linux box as root (or
# with sudo). After it succeeds, the IDE's Deploy button takes over.
#
# What it does:
#   1. Creates /opt/ia2/{versions,bin} owned by root.
#   2. Drops the systemd unit shipped alongside this script.
#   3. Sets a "stub" current/ symlink to versions/_initial/ with the
#      runtime binary the user supplied via $RUNTIME_BIN.
#   4. Enables (but does NOT start) the systemd unit. First start
#      happens when the IDE pushes a project.
#
# Usage:
#   curl -fsSL https://.../install.sh | sudo INSTALL_DIR=/opt/ia2 \
#       RUNTIME_BIN=/path/to/ia2-runtime bash
# or:
#   sudo INSTALL_DIR=/opt/ia2 RUNTIME_BIN=./ia2-runtime \
#       ./install.sh

set -euo pipefail

INSTALL_DIR="${INSTALL_DIR:-/opt/ia2}"
RUNTIME_BIN="${RUNTIME_BIN:-}"
UNIT_FILE="${UNIT_FILE:-$(dirname "$0")/ia2.service}"

if [ -z "$RUNTIME_BIN" ] || [ ! -f "$RUNTIME_BIN" ]; then
  echo "ERROR: set RUNTIME_BIN to the path of the ia2-runtime binary." >&2
  echo "       Build it on a Linux box matching the edge's arch:" >&2
  echo "         cargo build --release -p ia2-runtime" >&2
  echo "       or cross-compile (see docs/edge-deploy.md)." >&2
  exit 2
fi

if [ "$(id -u)" -ne 0 ]; then
  echo "ERROR: install.sh must run as root (use sudo)." >&2
  exit 2
fi

echo "==> creating layout at $INSTALL_DIR"
mkdir -p "$INSTALL_DIR/versions/_initial"
mkdir -p "$INSTALL_DIR/versions/_initial/project/pous"
mkdir -p "$INSTALL_DIR/versions/_initial/project/devices"
mkdir -p "$INSTALL_DIR/versions/_initial/project/edges"

# Minimal project.toml + sample POU so the runtime doesn't refuse to
# start before the first deploy. The IDE's first deploy replaces this.
cat > "$INSTALL_DIR/versions/_initial/project/project.toml" <<'EOF'
name = "edge_stub"
version = "0.0"
EOF
cat > "$INSTALL_DIR/versions/_initial/project/pous/main.st" <<'EOF'
PROGRAM main
    VAR
        counter : INT;
    END_VAR
    counter := counter + 1;
END_PROGRAM
EOF
cat > "$INSTALL_DIR/versions/_initial/project/iomap.toml" <<'EOF'
EOF
# Project-level scheduling: one 100 ms task running `main`. Replaced on
# the first deploy from the IDE.
cat > "$INSTALL_DIR/versions/_initial/project/tasks.toml" <<'EOF'
[[tasks]]
name = "plc_task"
interval_ms = 100
priority = 1

[[programs]]
instance = "main_inst"
program = "main"
task = "plc_task"
EOF

echo "==> installing runtime binary"
install -m 0755 "$RUNTIME_BIN" "$INSTALL_DIR/versions/_initial/runtime"

echo "==> swapping current symlink"
TMPLINK="$INSTALL_DIR/.current.new"
ln -sfn "$INSTALL_DIR/versions/_initial" "$TMPLINK"
mv -Tf "$TMPLINK" "$INSTALL_DIR/current"

echo "==> installing systemd unit"
if [ ! -f "$UNIT_FILE" ]; then
  echo "ERROR: unit file not found at $UNIT_FILE" >&2
  exit 2
fi
install -m 0644 "$UNIT_FILE" /etc/systemd/system/ia2.service
systemctl daemon-reload
systemctl enable ia2.service

echo ""
echo "==> done."
echo "    Layout: $INSTALL_DIR"
echo "    Unit:   /etc/systemd/system/ia2.service (enabled)"
echo ""
echo "    Start it now with the stub project to verify the binary runs:"
echo "      systemctl start ia2 && journalctl -u ia2 -f"
echo ""
echo "    Then push a real project from the IDE via Deploy."
