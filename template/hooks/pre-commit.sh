#!/usr/bin/env bash
#
# Coral pre-commit hook — refuses commits that include a wiki page
# whose frontmatter has `reviewed: false`. This is the trust-by-curation
# gate for `coral session distill --apply` (and any other LLM-generated
# wiki content): the LLM proposes; humans flip `reviewed: true` after a
# read-through; only then does the page enter the canonical wiki.
#
# Install in your project:
#   ln -s ../../template/hooks/pre-commit.sh .git/hooks/pre-commit
# or copy:
#   cp template/hooks/pre-commit.sh .git/hooks/pre-commit
#   chmod +x .git/hooks/pre-commit
#
# Bypass (please don't): `git commit --no-verify`. CI will still catch
# it via `coral lint --severity critical` on the same lint rule.
#
# v0.20.0 — see CHANGELOG and docs/SESSIONS.md for the full posture.

set -euo pipefail

# Find the coral binary on PATH; fall back to a `cargo run` from the
# repo root so hook works in dev workspaces that haven't installed
# coral globally yet.
if command -v coral >/dev/null 2>&1; then
    CORAL=(coral)
elif [[ -x ./target/release/coral ]]; then
    CORAL=(./target/release/coral)
elif [[ -x ./target/debug/coral ]]; then
    CORAL=(./target/debug/coral)
else
    echo "coral pre-commit: \`coral\` binary not found on PATH and no" >&2
    echo "  target/{release,debug}/coral exists. Skipping the trust-gate" >&2
    echo "  check. Install coral or set up the binary before relying on" >&2
    echo "  this hook." >&2
    exit 0
fi

# Run only the `unreviewed-distilled` rule against the current wiki.
# `--rule` filtering keeps this fast even on large wikis (skips other
# 9 structural rules + the LLM rule + injection scan).
#
# `--severity critical` means: exit non-zero only when an
# `unreviewed-distilled` finding is present (that rule produces
# Critical issues). Other lint warnings/info don't block the commit.
if ! "${CORAL[@]}" lint --rule unreviewed-distilled --severity critical \
        --format markdown 2>/dev/null; then
    cat >&2 <<'EOF'

  ────────────────────────────────────────────────────────────────────
  ✘ Coral pre-commit blocked the commit.

  One or more wiki pages have `reviewed: false` in their frontmatter.
  These pages were proposed by `coral session distill --apply` (or
  another LLM-generated path) and must be human-reviewed before they
  enter the canonical wiki.

  Fix:
    1. Open each `reviewed: false` page and review the body + sources.
    2. Flip `reviewed: false` → `reviewed: true` in the frontmatter.
    3. `git add` the page and re-run `git commit`.

  Bypass (NOT recommended):
    git commit --no-verify

  CI gate:
    coral lint --rule unreviewed-distilled --severity critical
  ────────────────────────────────────────────────────────────────────

EOF
    exit 1
fi

exit 0
