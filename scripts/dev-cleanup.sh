#!/usr/bin/env bash
# Coral developer disk-hygiene umbrella.
#
# Wraps the four maintenance commands documented in
# `docs/DEVELOPMENT.md` (`cargo sweep --time 7`, `cargo sweep
# --installed`, `cargo clean`, `cargo cache --autoclean`) into one
# command with a self-selecting mode based on current `target/` size.
#
# The strategy this enforces is already documented; the script just
# removes the friction of remembering four tools and three thresholds.
#
# Re-run cost: ~1-2s for `--check`; the mutation modes run their
# underlying tool plus a du round-trip.
#
# Usage:
#   ./scripts/dev-cleanup.sh                # default: --check (no mutations)
#   ./scripts/dev-cleanup.sh --soft         # cargo sweep --time 7
#   ./scripts/dev-cleanup.sh --medium       # cargo sweep --installed
#   ./scripts/dev-cleanup.sh --hard         # cargo clean
#   ./scripts/dev-cleanup.sh --auto         # pick mode by current size
#   ./scripts/dev-cleanup.sh --budget 15    # exit 1 if target > 15 GiB
#   ./scripts/dev-cleanup.sh --no-registry  # skip cargo cache --autoclean
#
# Auto thresholds (ratchet-locked to DEVELOPMENT.md):
#   target/ <  5 GiB  -> --check  (no action)
#   target/  5-15 GiB -> --soft   (cargo sweep --time 7)
#   target/ 15-30 GiB -> --medium (cargo sweep --installed)
#   target/ > 30 GiB  -> --hard   (cargo clean)
#
# See: docs/DEVELOPMENT.md "Disk budget" and "Maintenance commands".

set -euo pipefail

# ---------- helpers ----------

if [[ -t 1 ]]; then
  BLUE='\033[0;34m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'; RED='\033[0;31m'; NC='\033[0m'
else
  BLUE=''; GREEN=''; YELLOW=''; RED=''; NC=''
fi
step() { printf "${BLUE}==>${NC} %s\n" "$*"; }
ok()   { printf "${GREEN}[ok]${NC} %s\n" "$*"; }
warn() { printf "${YELLOW}[!]${NC} %s\n" "$*" >&2; }
fail() { printf "${RED}[FAIL]${NC} %s\n" "$*" >&2; }

usage() {
  sed -n '3,32p' "$0" | sed 's/^# \{0,1\}//'
  exit "${1:-0}"
}

# ---------- parse args ----------

MODE=""
BUDGET=""
NO_REGISTRY=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    --check)       MODE="check";  shift ;;
    --soft)        MODE="soft";   shift ;;
    --medium)      MODE="medium"; shift ;;
    --hard)        MODE="hard";   shift ;;
    --auto)        MODE="auto";   shift ;;
    --budget)      BUDGET="$2";   shift 2 ;;
    --no-registry) NO_REGISTRY=1; shift ;;
    -h|--help)     usage 0 ;;
    *) fail "unknown arg: $1"; usage 1 ;;
  esac
done
MODE="${MODE:-check}"

# ---------- repo-root guard ----------

if [[ ! -f Cargo.toml ]] || ! grep -q '^\[workspace\]' Cargo.toml 2>/dev/null; then
  fail "run from the Coral repo root (Cargo.toml with [workspace] not found)"
  exit 2
fi

# ---------- size probe ----------

# du -s -B1 portable: BSD du (macOS) doesn't accept -B; fall back to
# -k and convert. Linux GNU du supports both.
target_bytes() {
  if [[ ! -d target ]]; then echo 0; return; fi
  if out=$(du -s -B1 target 2>/dev/null); then
    echo "$out" | awk '{print $1}'
  else
    # macOS BSD du fallback
    du -sk target 2>/dev/null | awk '{print $1 * 1024}'
  fi
}

bytes_to_gib() {
  awk -v b="$1" 'BEGIN { printf "%.2f", b / 1024 / 1024 / 1024 }'
}

bytes_to_human() {
  awk -v b="$1" 'BEGIN {
    if      (b > 1024^3) printf "%.2f GiB", b / 1024^3
    else if (b > 1024^2) printf "%.1f MiB", b / 1024^2
    else if (b > 1024)   printf "%.1f KiB", b / 1024
    else                 printf "%d B",    b
  }'
}

# ---------- auto-pick ----------

resolve_auto() {
  local gib; gib=$(bytes_to_gib "$1")
  awk -v g="$gib" 'BEGIN {
    if (g <  5) print "check"
    else if (g < 15) print "soft"
    else if (g < 30) print "medium"
    else             print "hard"
  }'
}

# ---------- main ----------

BEFORE=$(target_bytes)
BEFORE_H=$(bytes_to_human "$BEFORE")
step "current target/ size: $BEFORE_H"

if [[ "$MODE" == "auto" ]]; then
  MODE=$(resolve_auto "$BEFORE")
  step "auto-mode resolved to: --$MODE"
fi

case "$MODE" in
  check)
    # No mutations. Report breakdown of the top sub-dirs so the
    # maintainer can decide whether to escalate.
    # Skip cargo registry du here: MSYS du on Windows is unbearably
    # slow on .cargo/registry (tens of thousands of small files);
    # the registry is cargo-cache's job, run during mutation modes.
    if [[ -d target ]] && [[ "$BEFORE" -gt 0 ]]; then
      step "top sub-dirs (size descending, top 8):"
      du -sh target/* 2>/dev/null | sort -hr | head -8 || true
    fi
    ;;

  soft)
    if ! command -v cargo-sweep >/dev/null 2>&1; then
      warn "cargo-sweep not installed - run ./scripts/dev-setup.sh"
      exit 3
    fi
    step "cargo sweep --time 7 (drops artifacts older than 7 days)"
    cargo sweep --time 7
    ;;

  medium)
    if ! command -v cargo-sweep >/dev/null 2>&1; then
      warn "cargo-sweep not installed - run ./scripts/dev-setup.sh"
      exit 3
    fi
    step "cargo sweep --installed (keeps installed-toolchain artifacts only)"
    cargo sweep --installed
    ;;

  hard)
    step "cargo clean (everything; next build approx 3 min)"
    cargo clean
    ;;

  *)
    fail "unreachable mode: $MODE"
    exit 4
    ;;
esac

# ---------- registry sweep (always-on unless --no-registry) ----------

if [[ "$NO_REGISTRY" -eq 0 ]] && [[ "$MODE" != "check" ]]; then
  if command -v cargo-cache >/dev/null 2>&1; then
    step "cargo cache --autoclean (registry pruning)"
    cargo cache --autoclean
  else
    warn "cargo-cache not installed - skipping registry prune"
    warn "  install with: cargo install --locked cargo-cache"
  fi
fi

# ---------- after-report ----------

AFTER=$(target_bytes)
AFTER_H=$(bytes_to_human "$AFTER")
FREED=$((BEFORE - AFTER))
FREED_H=$(bytes_to_human "$FREED")

if [[ "$MODE" != "check" ]]; then
  ok "target/ size after: $AFTER_H (freed $FREED_H)"
fi

# ---------- budget gate ----------

if [[ -n "$BUDGET" ]]; then
  AFTER_GIB=$(bytes_to_gib "$AFTER")
  OVER=$(awk -v a="$AFTER_GIB" -v b="$BUDGET" 'BEGIN { print (a > b) ? 1 : 0 }')
  if [[ "$OVER" == "1" ]]; then
    fail "target/ is $AFTER_GIB GiB > budget $BUDGET GiB"
    exit 1
  fi
  ok "target/ $AFTER_GIB GiB within budget $BUDGET GiB"
fi

exit 0
