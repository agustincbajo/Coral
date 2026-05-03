#!/usr/bin/env bash
# Run the same checks GitHub Actions runs, locally.
#
# Why this exists: when GH Actions billing/runners are unavailable,
# `git push` returns no signal on whether the change is safe to ship.
# This script is the offline substitute — it mirrors the jobs declared
# in .github/workflows/ci.yml that CAN run on a dev laptop (the four
# blocking checks for merging a PR).
#
# What it does NOT cover (jobs that need GitHub-side infrastructure):
#   - Cross-platform smoke matrix (ubuntu + macOS via runners)
#   - Security audit (`cargo-audit`) and license check (`cargo-deny`) —
#     those install heavy external crates; opt in via --slow.
#   - Coverage upload (`cargo-llvm-cov` → Codecov)
#
# Exit code: 0 only if every check passed. Any failure causes immediate
# exit (set -e) so the first red is the one you fix.

set -euo pipefail

# Colors — silent when stdout isn't a TTY (so log captures stay clean).
if [[ -t 1 ]]; then
    GREEN='\033[0;32m'
    RED='\033[0;31m'
    YELLOW='\033[0;33m'
    BOLD='\033[1m'
    RESET='\033[0m'
else
    GREEN='' RED='' YELLOW='' BOLD='' RESET=''
fi

# Working dir = repo root, regardless of where the user invoked us from.
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$REPO_ROOT"

SLOW=0
for arg in "$@"; do
    case "$arg" in
        --slow) SLOW=1 ;;
        --help|-h)
            cat <<EOF
Usage: scripts/ci-locally.sh [--slow]

Mirrors .github/workflows/ci.yml. Default runs the four blocking checks:
  1. cargo fmt --check
  2. cargo clippy -D warnings
  3. cargo test --workspace --all-features
  4. cargo test --test bc_regression -p coral-cli

With --slow, also runs (slower, requires installs):
  5. cargo audit --deny warnings
  6. cargo deny check
EOF
            exit 0 ;;
        *)
            echo "unknown arg: $arg (see --help)"
            exit 2 ;;
    esac
done

step() {
    local n="$1" total="$2" name="$3"
    printf "${BOLD}[%d/%d] %s${RESET}\n" "$n" "$total" "$name"
}

ok() {
    printf "  ${GREEN}✔${RESET} %s\n" "$1"
}

fail() {
    printf "  ${RED}✘${RESET} %s\n" "$1"
}

TOTAL=4
[[ $SLOW -eq 1 ]] && TOTAL=6

START=$(date +%s)

# 1 — fmt
step 1 $TOTAL "cargo fmt --check"
if cargo fmt --all -- --check > /tmp/coral-ci-fmt.log 2>&1; then
    ok "formatting clean"
else
    fail "formatting drift; run: cargo fmt --all"
    cat /tmp/coral-ci-fmt.log
    exit 1
fi

# 2 — clippy
step 2 $TOTAL "cargo clippy --workspace --all-targets -- -D warnings"
if cargo clippy --workspace --all-targets -- -D warnings > /tmp/coral-ci-clippy.log 2>&1; then
    ok "no clippy warnings"
else
    fail "clippy reports warnings"
    cat /tmp/coral-ci-clippy.log
    exit 1
fi

# 3 — full test suite
step 3 $TOTAL "cargo test --workspace --all-features"
if cargo test --workspace --all-features > /tmp/coral-ci-test.log 2>&1; then
    PASSED=$(grep -E "^test result: ok\." /tmp/coral-ci-test.log | awk '{sum += $4} END {print sum+0}')
    ok "all tests pass ($PASSED total)"
else
    fail "test failures"
    grep -E "FAILED|test result: FAILED" /tmp/coral-ci-test.log | head -20
    echo "  full log: /tmp/coral-ci-test.log"
    exit 1
fi

# 4 — bc-regression (already in step 3, but pinned as its own gate so
#     a "BC broke" failure reads loud and clear)
step 4 $TOTAL "cargo test --test bc_regression -p coral-cli"
if cargo test --test bc_regression -p coral-cli > /tmp/coral-ci-bc.log 2>&1; then
    BC_PASSED=$(grep -E "^test result: ok\." /tmp/coral-ci-bc.log | awk '{sum += $4} END {print sum+0}')
    ok "v0.15 single-repo BC contract held ($BC_PASSED tests)"
else
    fail "BACKWARD-COMPAT BROKEN — investigate before commit"
    cat /tmp/coral-ci-bc.log
    exit 1
fi

if [[ $SLOW -eq 1 ]]; then
    # 5 — cargo audit
    step 5 $TOTAL "cargo audit --deny warnings"
    if ! command -v cargo-audit > /dev/null 2>&1; then
        printf "  ${YELLOW}⚠${RESET} cargo-audit not installed; run: cargo install --locked cargo-audit\n"
    elif cargo audit --deny warnings > /tmp/coral-ci-audit.log 2>&1; then
        ok "no advisories"
    else
        fail "advisory found"
        cat /tmp/coral-ci-audit.log
        exit 1
    fi

    # 6 — cargo deny
    step 6 $TOTAL "cargo deny --all-features check"
    if ! command -v cargo-deny > /dev/null 2>&1; then
        printf "  ${YELLOW}⚠${RESET} cargo-deny not installed; run: cargo install --locked cargo-deny\n"
    elif cargo deny --all-features check --hide-inclusion-graph > /tmp/coral-ci-deny.log 2>&1; then
        ok "licenses + duplicates clean"
    else
        fail "deny check failed"
        cat /tmp/coral-ci-deny.log
        exit 1
    fi
fi

DURATION=$(( $(date +%s) - START ))
printf "\n${GREEN}${BOLD}all checks passed in %ds${RESET}\n" "$DURATION"
