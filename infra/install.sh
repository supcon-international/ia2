#!/usr/bin/env bash
#
# One-time edge bootstrap. Run this on a fresh edge Linux box as root (or
# with sudo). After it succeeds, the IDE's Deploy button takes over.
#
# What it does:
#   1. Creates /opt/controlsoftware/{versions,bin} owned by root.
#   2. Drops the systemd unit shipped alongside this script.
#   3. Sets a "stub" current/ symlink to versions/_initial/ with the
#      runtime binary the user supplied via $RUNTIME_BIN.
#   4. Enables (but does NOT start) the systemd unit. First start
#      happens when the IDE pushes a project.
#
# Usage:
#   curl -fsSL https://.../install.sh | sudo INSTALL_DIR=/opt/controlsoftware \
#       RUNTIME_BIN=/path/to/controlsoftware-runtime bash
# or:
#   sudo INSTALL_DIR=/opt/controlsoftware RUNTIME_BIN=./controlsoftware-runtime \
#       ./install.sh

set -euo pipefail

INSTALL_DIR="${INSTALL_DIR:-/opt/controlsoftware}"
RUNTIME_BIN="${RUNTIME_BIN:-}"
UNIT_FILE="${UNIT_FILE:-$(dirname "$0")/controlsoftware.service}"

if [ -z "$RUNTIME_BIN" ] || [ ! -f "$RUNTIME_BIN" ]; then
  echo "ERROR: set RUNTIME_BIN to the path of the controlsoftware-runtime binary." >&2
  echo "       Build it on a Linux box matching the edge's arch:" >&2
  echo "         cargo build --release -p controlsoftware-runtime" >&2
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

CONFIGURATION config
    RESOURCE plc_res ON PLC
        TASK plc_task(INTERVAL := T#100ms, PRIORITY := 1);
        PROGRAM plc_task_instance WITH plc_task : main;
    END_RESOURCE
END_CONFIGURATION
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
# The stub main.st here doesn't have an inline CONFIGURATION — tasks.toml
# above is the project-level scheduling source of truth.
sed -i.bak '/^CONFIGURATION/,/^END_CONFIGURATION/d' \
  "$INSTALL_DIR/versions/_initial/project/pous/main.st" 2>/dev/null \
  && rm -f "$INSTALL_DIR/versions/_initial/project/pous/main.st.bak" || true

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
install -m 0644 "$UNIT_FILE" /etc/systemd/system/controlsoftware.service
systemctl daemon-reload
systemctl enable controlsoftware.service

echo ""
echo "==> done."
echo "    Layout: $INSTALL_DIR"
echo "    Unit:   /etc/systemd/system/controlsoftware.service (enabled)"
echo ""
echo "    Start it now with the stub project to verify the binary runs:"
echo "      systemctl start controlsoftware && journalctl -u controlsoftware -f"
echo ""
echo "    Then push a real project from the IDE via Deploy."
