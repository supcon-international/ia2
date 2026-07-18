#!/usr/bin/env bash
#
# install-skill.sh — set up IA2 for a coding agent (Claude Code, Codex, …).
#
# Builds the `cs` CLI + `ia2-server` + its `lsp-launcher` sidecar, installs
# them on your PATH, and drops
# the `industrial-automation-skill` into ~/.claude/skills/ so your agent can
# author / compile / run / debug / deploy IEC 61131-3 PLC programs through IA2.
#
# Just the SKILL (not the binaries)? The recommended route is the
# vercel-labs/skills installer — it copies the skill + its references:
#     npx skills add https://github.com/supcon-international/ia2/tree/main/.claude/skills/industrial-automation-skill
# This script is the one-shot "skill + cs + ia2-server, from a clone" path.
#
# This is the DEV-MACHINE installer. (For provisioning a Linux edge box, see
# infra/install.sh — different thing.)
#
# Usage — run from a clone of the IA2 repo:
#     git clone --recursive https://github.com/supcon-international/ia2
#     cd ia2 && ./scripts/install-skill.sh
#
# Env knobs:
#     IA2_BIN_DIR   where to install cs + ia2-server   (default: ~/.local/bin)
#     CLAUDE_DIR    Claude config root for the skill    (default: ~/.claude)

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

BIN_DIR="${IA2_BIN_DIR:-$HOME/.local/bin}"
CLAUDE_DIR="${CLAUDE_DIR:-$HOME/.claude}"
SKILL_NAME="industrial-automation-skill"
SKILL_SRC="$REPO_ROOT/.claude/skills/$SKILL_NAME"
SKILL_DST="$CLAUDE_DIR/skills/$SKILL_NAME"

say()  { printf '\033[1;36m==>\033[0m %s\n' "$*"; }
ok()   { printf '\033[1;32m ✓\033[0m %s\n' "$*"; }
die()  { printf '\033[1;31mERROR:\033[0m %s\n' "$*" >&2; exit 1; }

# ---- preflight ----------------------------------------------------------
command -v cargo >/dev/null 2>&1 \
  || die "Rust toolchain not found. Install it from https://rustup.rs and re-run."
[ -d "$SKILL_SRC" ] \
  || die "skill source missing at $SKILL_SRC — run this from inside the IA2 repo clone."

# The vendored ironplc compiler is a git submodule; the build needs it.
# An un-checked-out submodule is an empty directory.
if [ -z "$(ls -A "$REPO_ROOT/vendor/ironplc" 2>/dev/null)" ]; then
  say "fetching the vendored ironplc submodule"
  git -C "$REPO_ROOT" submodule update --init --recursive \
    || die "git submodule update failed (needed for vendor/ironplc)."
fi

# ---- build --------------------------------------------------------------
say "building cs + ia2-server + lsp-launcher (release) — the first build can take a few minutes"
cargo build --release -p ia2-cli -p server -p lsp-launcher

# ---- install binaries ---------------------------------------------------
say "installing binaries → $BIN_DIR"
mkdir -p "$BIN_DIR"
install -m 0755 "$REPO_ROOT/target/release/cs"     "$BIN_DIR/cs"
install -m 0755 "$REPO_ROOT/target/release/server" "$BIN_DIR/ia2-server"
# ia2-server spawns this per editor WebSocket to run the Monaco LSP
# bridge; it looks for the binary NEXT TO ITSELF, so it must be
# installed alongside — without it every editor LSP connection dies
# at spawn.
install -m 0755 "$REPO_ROOT/target/release/lsp-launcher" "$BIN_DIR/lsp-launcher"

# ---- install skill ------------------------------------------------------
say "installing skill → $SKILL_DST"
mkdir -p "$CLAUDE_DIR/skills"
rm -rf "$SKILL_DST"
cp -R "$SKILL_SRC" "$SKILL_DST"

# ---- verify -------------------------------------------------------------
"$BIN_DIR/cs" --help >/dev/null 2>&1 || die "cs failed to run after install."
ok "$("$BIN_DIR/cs" --version 2>/dev/null || echo 'cs') installed and runnable"

# ---- next steps ---------------------------------------------------------
on_path=""; case ":${PATH}:" in *":$BIN_DIR:"*) on_path="yes";; esac

cat <<EOF

$(ok "Done.")
  cs           → $BIN_DIR/cs
  ia2-server   → $BIN_DIR/ia2-server
  lsp-launcher → $BIN_DIR/lsp-launcher
  skill        → $SKILL_DST

Next:
EOF
[ -n "$on_path" ] || cat <<EOF
  1. Put the binaries on your PATH (this shell didn't have $BIN_DIR):
       echo 'export PATH="$BIN_DIR:\$PATH"' >> ~/.zshrc   # or ~/.bashrc; then reopen the shell
EOF
cat <<EOF
  $([ -n "$on_path" ] && echo 1 || echo 2). Start the IA2 server (headless API on :3001):
       ia2-server --bind 127.0.0.1:3001 &
  $([ -n "$on_path" ] && echo 2 || echo 3). Restart your coding agent so it discovers the skill, then just ask it
     to build a PLC program — it'll use the industrial-automation-skill + cs.

Optional — visual IDE + agent-takeover overlay (needs Node/pnpm):
    pnpm -C apps/web install && pnpm -C apps/web build
    ia2-server --bind 127.0.0.1:3001 --static-dir apps/web/dist
EOF
