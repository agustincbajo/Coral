#!/usr/bin/env bash
# Coral developer environment bootstrap.
#
# Idempotent. Run once per checkout (or after a Rust toolchain upgrade)
# to install the disk-management tooling described in
# docs/DEVELOPMENT.md and wire sccache into your global cargo config.
#
# Re-run cost: ~5 seconds if everything is already installed and
# already wired.

set -euo pipefail

# TTY-aware colors (no ANSI escapes when piped to a log).
if [[ -t 1 ]]; then
  BLUE='\033[0;34m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'; NC='\033[0m'
else
  BLUE=''; GREEN=''; YELLOW=''; NC=''
fi
step() { printf "${BLUE}==>${NC} %s\n" "$*"; }
ok()   { printf "${GREEN}✓${NC} %s\n" "$*"; }
warn() { printf "${YELLOW}!${NC} %s\n" "$*" >&2; }

step "Coral developer bootstrap"

# ---------- 1. tooling ----------
#
# `command -v` guard before each install: `cargo install` does
# replace an existing binary even when versions match, which fails on
# Windows whenever `sccache.exe` is acting as the live `rustc-wrapper`
# (file lock denies the rename). Skipping when present is the safe
# default; explicit `cargo install --force <tool>` updates a tool.

install_if_missing() {
  local tool="$1"
  if command -v "$tool" >/dev/null 2>&1; then
    ok "$tool already installed ($(command -v "$tool"))"
  else
    step "installing $tool"
    cargo install --locked "$tool"
    ok "$tool installed"
  fi
}

install_if_missing cargo-sweep
install_if_missing sccache
install_if_missing cargo-nextest

# ---------- 2. global cargo config ----------

CARGO_HOME="${CARGO_HOME:-$HOME/.cargo}"
GLOBAL_CONFIG="$CARGO_HOME/config.toml"
mkdir -p "$CARGO_HOME"

# Refuse to clobber a different wrapper. A user with `cranelift` or
# similar will know to revert if they want sccache.
EXISTING_WRAPPER="$(grep -oP '(?<=^rustc-wrapper\s=\s")[^"]+' "$GLOBAL_CONFIG" 2>/dev/null || true)"
if [[ "$EXISTING_WRAPPER" == "sccache" ]]; then
  ok "sccache already wired into $GLOBAL_CONFIG"
elif [[ -n "$EXISTING_WRAPPER" ]]; then
  warn "leaving existing rustc-wrapper=\"$EXISTING_WRAPPER\" in $GLOBAL_CONFIG (replace manually to switch to sccache)"
else
  step "wiring sccache into $GLOBAL_CONFIG"
  if [[ -f "$GLOBAL_CONFIG" ]] && grep -q '^\[build\]' "$GLOBAL_CONFIG"; then
    awk '
      /^\[build\]/ && !inserted { print; print "rustc-wrapper = \"sccache\""; inserted=1; next }
      { print }
    ' "$GLOBAL_CONFIG" > "$GLOBAL_CONFIG.tmp" && mv "$GLOBAL_CONFIG.tmp" "$GLOBAL_CONFIG"
  else
    printf '\n[build]\nrustc-wrapper = "sccache"\n' >> "$GLOBAL_CONFIG"
  fi
  ok "sccache wired into $GLOBAL_CONFIG"
fi

# ---------- 3. sccache cache cap ----------

if [[ -z "${SCCACHE_CACHE_SIZE:-}" ]]; then
  warn "SCCACHE_CACHE_SIZE not set in your shell — defaults to 10G. To cap at 5G:"
  warn "  add  export SCCACHE_CACHE_SIZE=5G  to your shell rc"
fi

# ---------- 4. summary ----------

cat <<EOF

────────────────────────────────────────────────────────────────────
Setup complete. Maintenance commands you'll use:

  cargo sweep --time 7      # weekly cleanup (cron-friendly); 0.5–3 GB
  cargo sweep --installed   # aggressive; recovers 5–15 GB, slower next build
  cargo clean               # nuclear; ~3 min next build
  sccache --show-stats      # cache hit rate (healthy ≥ 60%)

Disk budget targets:
  target/                                   < 5 GB healthy, > 15 GB → sweep
  ~/.cargo/registry/                        < 1 GB healthy
  ~/.cache/sccache/                         10 GB cap (configurable via SCCACHE_CACHE_SIZE)
  crates/coral-ui/assets/src/node_modules/  ~150 MB healthy

Full reference:  docs/DEVELOPMENT.md
────────────────────────────────────────────────────────────────────
EOF
