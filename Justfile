# IA2 local task runner. Install: `brew install just` (mac) /
# `cargo install just`. Each recipe mirrors a step that runs in CI;
# `just ci` runs the whole pipeline locally so you can validate
# before push.

# Default: print the menu of recipes.
default:
    @just --list

# Full CI pipeline — same checks GitHub Actions runs on every push.
# Takes ~2 min cold, ~30 s warm.
ci: fmt clippy test web

# rustfmt — fail if any source needs reformatting.
fmt:
    cargo fmt --all -- --check

# clippy with -D warnings — fail on any lint.
clippy:
    cargo clippy --workspace --all-targets -- -D warnings

# All Rust tests across the workspace.
test:
    cargo test --workspace --quiet

# Web build + type-check.
web:
    cd apps/web && pnpm install --frozen-lockfile
    cd apps/web && pnpm tsc --noEmit
    cd apps/web && pnpm build

# Mac shell bundle (debug, then release). Local-only — CI runs
# this on tag pushes via .github/workflows/mac.yml.
mac:
    ./apps/mac/build.sh debug

mac-release:
    ./apps/mac/build.sh release

# Apply rustfmt + clippy auto-fixes. Use before commit when you've
# touched a lot of code; CI just checks, doesn't fix.
fix:
    cargo fmt --all
    cargo clippy --workspace --all-targets --fix --allow-dirty --allow-staged
