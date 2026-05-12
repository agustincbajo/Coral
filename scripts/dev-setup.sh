#!/usr/bin/env bash
# Coral developer environment bootstrap.
#
# Idempotent. Run once per checkout (or after a Rust toolchain upgrade)
# to install the disk-management tooling described in
# docs/DEVELOPMENT.md and wire sccache into your global cargo config.
#
# What this script does:
#   1. Installs cargo-sweep, sccache, cargo-nextest if missing.
#   2. Adds `rustc-wrapper = "sccache"` to ~/.cargo/config.toml (only if
#      not already set).
#   3. Prints disk-budget targets and the daily / weekly maintenance
#      commands.
#
# What this script does NOT do:
#   - Modify your shell rc files.
#   - Set CARGO_INCREMENTAL globally (the repo's .cargo/config.toml
#     already does that scoped to this checkout).
#   - Run `cargo build` — fresh setup means your next build will be
#     ~3 minutes; do that on your own time.
#
# Re-run cost: ~5 seconds if everything is already installed.

set -euo pipefail

GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m'

step() { printf "${BLUE}==>${NC} %s\n" "$*"; }
ok()   { printf "${GREEN}✓${NC} %s\n" "$*"; }
warn() { printf "${YELLOW}!${NC} %s\n" "$*"; }

step "Coral developer bootstrap"

# ---------- 1. tooling ----------

install_if_missing() {
  local tool="$1"
  local crate="${2:-$1}"
  if command -v "$tool" >/dev/null 2>&1; then
    ok "$tool already installed ($(command -v "$tool"))"
  else
    step "installing $crate"
    cargo install --locked "$crate"
    ok "$crate installed"
  fi
}

install_if_missing cargo-sweep    cargo-sweep
install_if_missing sccache        sccache
install_if_missing cargo-nextest  cargo-nextest

# ---------- 2. global cargo config ----------

CARGO_HOME="${CARGO_HOME:-$HOME/.cargo}"
GLOBAL_CONFIG="$CARGO_HOME/config.toml"

mkdir -p "$CARGO_HOME"
touch "$GLOBAL_CONFIG"

if grep -q '^rustc-wrapper *= *"sccache"' "$GLOBAL_CONFIG" 2>/dev/null; then
  ok "sccache already wired into $GLOBAL_CONFIG"
else
  step "wiring sccache into $GLOBAL_CONFIG"
  # Append a [build] section if missing, then the wrapper line.
  if grep -q '^\[build\]' "$GLOBAL_CONFIG" 2>/dev/null; then
    # Section exists — insert under it.
    awk '
      /^\[build\]/ { print; print "rustc-wrapper = \"sccache\""; next }
      { print }
    ' "$GLOBAL_CONFIG" > "$GLOBAL_CONFIG.tmp" && mv "$GLOBAL_CONFIG.tmp" "$GLOBAL_CONFIG"
  else
    # No [build] section yet — append the whole stanza.
    cat >> "$GLOBAL_CONFIG" <<'EOF'

[build]
rustc-wrapper = "sccache"
EOF
  fi
  ok "sccache wired into $GLOBAL_CONFIG"
fi

# ---------- 3. summary ----------

cat <<'EOF'

────────────────────────────────────────────────────────────────────
Setup complete. Maintenance commands you'll use:

  cargo sweep --installed   # quick cleanup, recovers 5–15 GB typically
  cargo sweep --time 7      # cron-friendly, recovers 0.5–3 GB weekly
  cargo clean               # nuclear, recovers everything (~3 min next build)
  sccache --show-stats      # check cache hit rate (healthy is ≥ 60%)

Disk budget targets:
  target/                          < 5 GB healthy, > 15 GB → sweep/clean
  ~/.cargo/registry/               < 1 GB healthy
  crates/coral-ui/assets/src/node_modules/  ~150 MB healthy

Full reference:  docs/DEVELOPMENT.md
────────────────────────────────────────────────────────────────────
EOF
