#!/usr/bin/env bash
# Build IA2.app — assembles the Swift shell + the Rust server binary
# + the built React `dist` into a Mac `.app` bundle.
#
# Usage:
#   apps/mac/build.sh            # debug builds, output → apps/mac/build/
#   apps/mac/build.sh release    # release builds (lto, strip, etc.)
#
# We do this by hand instead of going through Xcode because:
#   1. SwiftPM + plain `swift build` is enough — no Xcode-specific
#      project file to maintain.
#   2. The shell is tiny (~5 source files); a shell script is more
#      readable than a synthesized .xcodeproj.
#   3. CI doesn't need Xcode installed, only the command-line tools.
#
# What this does NOT do:
#   - Code-signing / notarization. Handled separately in the release
#     workflow (Sparkle update channel comes with that).
#   - Auto-update plumbing.
#   - DMG creation.

set -euo pipefail

MODE="${1:-debug}"
case "$MODE" in
  debug|release) ;;
  *) echo "Usage: $0 [debug|release]"; exit 2 ;;
esac

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
MAC_DIR="$REPO_ROOT/apps/mac"
WEB_DIR="$REPO_ROOT/apps/web"
BUILD_DIR="$MAC_DIR/build/$MODE"
APP_BUNDLE="$BUILD_DIR/IA2.app"
APP_CONTENTS="$APP_BUNDLE/Contents"

echo "==> Cleaning $BUILD_DIR"
rm -rf "$BUILD_DIR"
mkdir -p "$APP_CONTENTS/MacOS" "$APP_CONTENTS/Resources/web"

# --- 1. Rust server -------------------------------------------------

echo "==> Building Rust server ($MODE)"
if [[ "$MODE" == "release" ]]; then
    (cd "$REPO_ROOT" && cargo build -p server --release)
    SERVER_BIN="$REPO_ROOT/target/release/server"
else
    (cd "$REPO_ROOT" && cargo build -p server)
    SERVER_BIN="$REPO_ROOT/target/debug/server"
fi
# The bundled binary lives at Contents/MacOS/ia2-server — the Swift
# shell's BackendSupervisor.locateServerBinary() looks for this name.
cp "$SERVER_BIN" "$APP_CONTENTS/MacOS/ia2-server"

# --- 1b. ts-rs bindings ---------------------------------------------
# ts-rs exports TS types only as a side effect of `cargo test`, not
# `cargo build`. The React build below imports them by path, so on a
# fresh clone the web build fails with TS2307 module-not-found errors.
# Run the targeted export tests (fast — sub-second when builds are
# cached). `--tests` skips doctests; `export_bindings` is the prefix
# ts-rs generates for every `#[derive(TS)]`-exported type.
echo "==> Generating ts-rs bindings"
(cd "$REPO_ROOT" && cargo test --workspace --tests --quiet export_bindings)

# --- 2. React app ---------------------------------------------------

echo "==> Building React app"
(cd "$WEB_DIR" && pnpm build)
cp -R "$WEB_DIR/dist/." "$APP_CONTENTS/Resources/web/"

# --- 3. Swift shell -------------------------------------------------

echo "==> Building Swift shell ($MODE)"
SWIFT_FLAGS=()
if [[ "$MODE" == "release" ]]; then
    SWIFT_FLAGS+=(-c release)
fi
(cd "$MAC_DIR" && swift build "${SWIFT_FLAGS[@]}")

SWIFT_BIN="$MAC_DIR/.build/$( [[ "$MODE" == "release" ]] && echo release || echo debug )/IA2"
cp "$SWIFT_BIN" "$APP_CONTENTS/MacOS/IA2"

# --- 4. App icon ---------------------------------------------------
#
# We render the icon from code so there's nothing binary in git
# (make-icon.swift produces the iconset; iconutil compiles to .icns).
# Cache the .icns when the source script hasn't changed — full
# regeneration is ~1 s but it's wasted on most builds.

ICON_SRC="$MAC_DIR/Resources/make-icon.swift"
ICON_ICNS="$MAC_DIR/build/AppIcon.icns"
if [[ ! -f "$ICON_ICNS" || "$ICON_SRC" -nt "$ICON_ICNS" ]]; then
    echo "==> Rendering app icon"
    ICONSET="$MAC_DIR/build/AppIcon.iconset"
    rm -rf "$ICONSET"
    swift "$ICON_SRC" "$ICONSET" > /dev/null
    iconutil -c icns "$ICONSET" -o "$ICON_ICNS"
fi
cp "$ICON_ICNS" "$APP_CONTENTS/Resources/AppIcon.icns"

# --- 5. Info.plist -------------------------------------------------

cp "$MAC_DIR/Resources/Info.plist" "$APP_CONTENTS/Info.plist"

# --- 6. ad-hoc signature ------------------------------------------
#
# Without this Gatekeeper refuses to launch on macOS 13+ even when
# running locally. `-` is the ad-hoc identity, suitable for development
# only. Release signing is a separate flow with a Developer ID cert.
echo "==> Ad-hoc signing"
codesign --force --sign - --deep "$APP_BUNDLE"

# --- 7. Done -------------------------------------------------------

echo ""
echo "Built $APP_BUNDLE"
echo ""
echo "Run with:"
echo "  open $APP_BUNDLE"
echo "or directly:"
echo "  $APP_CONTENTS/MacOS/IA2"
