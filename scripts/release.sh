#!/usr/bin/env bash
# scripts/release.sh — maintainer-facing entry point for cargo-release.
#
# Three subcommands:
#
#   release.sh preflight
#     Invoked AS the cargo-release pre-release-hook. Reads $NEW_VERSION
#     (cargo-release exports it). Asserts a `## [<v>] - <today>` heading
#     exists in CHANGELOG.md, then runs scripts/ci-locally.sh.
#     Exit 0 = green; exit >0 = abort the release.
#
#   release.sh bump <X.Y.Z>
#     Wraps `cargo release X.Y.Z --no-tag --no-push --execute`. Bumps every
#     workspace-member version + writes the release commit. Tag/push deferred.
#     Maintainer hands off to a tester; on sign-off, they run `release.sh tag`.
#
#   release.sh tag <X.Y.Z>
#     Validates HEAD's commit subject starts with `release(vX.Y.Z):`. Then
#     `cargo release tag X.Y.Z --execute && cargo release push --execute`.
#     The tag-push triggers .github/workflows/release.yml which builds binaries.
#     The maintainer then runs scripts/release-gh.sh AFTER the binary build.
#
#   release.sh --help    prints usage.
#   anything else        usage + exit 2.
#
# Working agreements pinned here:
#   - No Co-Authored-By trailer (release.toml controls the commit message).
#   - All commits authored by Agustin Bajo only (git's user.name).
#   - Local-only by default; tag/push require an explicit second invocation.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

# Colors — silent when stdout isn't a TTY.
if [[ -t 1 ]]; then
    GREEN='\033[0;32m'
    RED='\033[0;31m'
    YELLOW='\033[0;33m'
    BOLD='\033[1m'
    RESET='\033[0m'
else
    GREEN='' RED='' YELLOW='' BOLD='' RESET=''
fi

usage() {
    cat <<'EOF'
Usage: scripts/release.sh <subcommand> [args]

Subcommands:
  preflight              Pre-release hook. Reads $NEW_VERSION. Asserts
                         CHANGELOG section + runs ci-locally.sh.
  bump <X.Y.Z>           Bump versions + create local release commit
                         (no tag, no push).
  tag <X.Y.Z>            After tester sign-off, tag HEAD as vX.Y.Z and
                         push branch + tag.
  --help, -h             This message.

Typical flow for a release:
  1. Hand-write the `## [X.Y.Z] - YYYY-MM-DD` section under
     `## [Unreleased]` in CHANGELOG.md.
  2. scripts/release.sh bump X.Y.Z
  3. (tester validates the bump commit)
  4. scripts/release.sh tag X.Y.Z
  5. (wait for .github/workflows/release.yml to build binaries)
  6. scripts/release-gh.sh vX.Y.Z   # creates the GH release with notes
EOF
}

err() {
    printf "${RED}error:${RESET} %s\n" "$1" >&2
}

ok() {
    printf "${GREEN}ok:${RESET} %s\n" "$1"
}

note() {
    printf "${YELLOW}note:${RESET} %s\n" "$1"
}

# ---- preflight ---------------------------------------------------------------

cmd_preflight() {
    cd "$REPO_ROOT"

    # cargo-release exports NEW_VERSION (since 0.24) for hook subprocesses.
    # Older guides say RELEASE_VERSION; we accept either for forward-compat.
    local version="${NEW_VERSION:-${RELEASE_VERSION:-}}"
    if [[ -z "$version" ]]; then
        err "preflight: \$NEW_VERSION is not set; this hook must be invoked by cargo-release."
        return 2
    fi

    local today
    today="$(date +%Y-%m-%d)"
    local heading="## [$version] - $today"

    # Anchor at line start so we don't accept stray text inside other sections.
    if ! grep -qE "^## \[$(printf '%s' "$version" | sed 's/\./\\./g')\] - $today\$" CHANGELOG.md; then
        err "preflight: CHANGELOG.md is missing the heading: $heading"
        err "  hand-write the section under '## [Unreleased]' before running 'release.sh bump'."
        return 3
    fi
    ok "CHANGELOG section '$heading' present"

    # Idempotent CHANGELOG link-footer rewrite.
    # We do this in the hook (rather than `pre-release-replacements`) because
    # cargo-release runs replacements once per package; with 9 workspace
    # members the regex would re-match 9 times and duplicate lines. The hook
    # also runs once per package, but a bash-level `grep -q` makes it safe.
    rewrite_changelog_footer "$version"

    # Run the local CI gate. Honor a $CI_LOCALLY override so tests can stub it.
    local ci_script="${CI_LOCALLY:-$REPO_ROOT/scripts/ci-locally.sh}"
    if [[ ! -x "$ci_script" ]]; then
        err "preflight: ci-locally script not executable: $ci_script"
        return 4
    fi

    # Cache ci-locally results across cargo-release's per-package hook
    # iterations. cargo-release fires this hook ONCE PER WORKSPACE PACKAGE,
    # but we only need to run the test suite once — the workspace test run
    # covers every crate. Key the marker on HEAD's sha so the cache is
    # automatically invalidated if HEAD moves between bump attempts.
    #
    # v0.22.0.1: pre-fix, the bump on Coral's 9-crate workspace fired the
    # hook 9 times = 9 × ~50s ci-locally = ~7-9 min wall-time. The dogfood
    # of `release.sh bump 0.22.0` exposed this — the bug was real, the
    # fix lets the bump complete in ~50s + N × ~0.1s short-circuits.
    # Honor $CORAL_PREFLIGHT_FORCE=1 to bypass the cache (debugging only).
    local head_sha
    head_sha="$(git -C "$REPO_ROOT" rev-parse HEAD 2>/dev/null || printf 'unknown')"
    local marker="${TMPDIR:-/tmp}/coral-preflight-${version}-${head_sha}.pass"
    if [[ -z "${CORAL_PREFLIGHT_FORCE:-}" && -f "$marker" ]]; then
        ok "ci-locally.sh already passed in this cargo-release run (HEAD=${head_sha:0:8}); skipping"
        return 0
    fi
    note "running $ci_script"
    "$ci_script"
    : > "$marker"
    ok "ci-locally.sh passed (cache marker: $marker)"
}

# Derive `<owner>/<repo>` from `git remote get-url origin`. Strips trailing
# `.git` and either `git@github.com:` or `https://github.com/` prefix. Falls
# back to the historical hardcoded `agustincbajo/Coral` if origin is missing
# or unparseable — that keeps preflight from blowing up on tempdir test
# fixtures that lack a real origin.
#
# MEDIUM 4 (v0.22.0 tester audit): pre-fix this was hardcoded inside
# `rewrite_changelog_footer`, so a fork would silently emit `agustincbajo`
# URLs in its CHANGELOG link-footer.
github_owner_repo() {
    local origin
    origin="$(git -C "$REPO_ROOT" remote get-url origin 2>/dev/null || true)"
    if [[ -z "$origin" ]]; then
        printf 'agustincbajo/Coral'
        return 0
    fi
    # Strip trailing `.git`.
    origin="${origin%.git}"
    # Match either git@github.com:OWNER/REPO or https://github.com/OWNER/REPO.
    # We DON'T validate the host beyond that — a self-hosted GitHub Enterprise
    # would parse the same way, and the comparison/tag URL shape is identical.
    local owner_repo=""
    if [[ "$origin" =~ ^git@[^:]+:(.+)$ ]]; then
        owner_repo="${BASH_REMATCH[1]}"
    elif [[ "$origin" =~ ^https?://[^/]+/(.+)$ ]]; then
        owner_repo="${BASH_REMATCH[1]}"
    fi
    if [[ -z "$owner_repo" || "$owner_repo" != */* ]]; then
        printf 'agustincbajo/Coral'
        return 0
    fi
    printf '%s' "$owner_repo"
}

# Idempotently rewrite the link-footer at the bottom of CHANGELOG.md so:
#   [Unreleased]: …/compare/vNEW...HEAD
#   [NEW]:       …/releases/tag/vNEW
#   [PREV]:      …/releases/tag/vPREV   (preserved from before)
#   …
#
# If the [NEW]: line is already present (idempotency), leave the file alone.
rewrite_changelog_footer() {
    local version="$1"
    if grep -qE "^\[$(printf '%s' "$version" | sed 's/\./\\./g')\]: " CHANGELOG.md; then
        ok "CHANGELOG link-footer already names [$version]; no rewrite needed"
        return 0
    fi

    # Derive owner/repo from git origin (MEDIUM 4 fix). Escape `/` for the
    # awk regex AND for the sed-style URL-rewrite line below.
    local owner_repo owner_repo_re
    owner_repo="$(github_owner_repo)"
    owner_repo_re="$(printf '%s' "$owner_repo" | sed 's|/|\\/|g')"

    # Extract the previous version from the `[Unreleased]: …compare/vX.Y.Z…HEAD` line.
    # If the footer is missing entirely, log and continue — the maintainer can
    # add a footer later. This keeps preflight robust against truncated/test
    # CHANGELOG fixtures.
    local prev
    prev="$(grep -E "^\[Unreleased\]: https://github\.com/${owner_repo}/compare/v[0-9]+\.[0-9]+\.[0-9]+\.\.\.HEAD\$" CHANGELOG.md \
        | sed -E "s|^\[Unreleased\]: https://github\.com/${owner_repo}/compare/v([0-9]+\.[0-9]+\.[0-9]+)\.\.\.HEAD\$|\1|" || true)"
    if [[ -z "$prev" ]]; then
        note "rewrite_changelog_footer: no [Unreleased]: compare/vX.Y.Z…HEAD line found; skipping footer rewrite"
        return 0
    fi

    # In-place edit: replace the [Unreleased]: line with two lines (Unreleased
    # bumped to NEW + new tag link), keeping the previous [PREV]: line below.
    # Use awk to avoid sed-platform-portability issues (BSD vs GNU).
    local tmp
    tmp="$(mktemp)"
    awk -v new="$version" -v prev="$prev" -v owner_repo_re="$owner_repo_re" -v owner_repo="$owner_repo" '
        BEGIN {
            rewritten = 0
            re = "^\\[Unreleased\\]: https://github\\.com/" owner_repo_re "/compare/v[0-9]+\\.[0-9]+\\.[0-9]+\\.\\.\\.HEAD$"
        }
        $0 ~ re {
            if (!rewritten) {
                print "[Unreleased]: https://github.com/" owner_repo "/compare/v" new "...HEAD"
                print "[" new "]: https://github.com/" owner_repo "/releases/tag/v" new
                rewritten = 1
                next
            }
        }
        { print }
    ' CHANGELOG.md > "$tmp" && mv "$tmp" CHANGELOG.md

    if ! grep -qE "^\[$(printf '%s' "$version" | sed 's/\./\\./g')\]: " CHANGELOG.md; then
        err "rewrite_changelog_footer: rewrite did not land; check CHANGELOG.md for shape drift."
        return 1
    fi
    ok "CHANGELOG link-footer rewritten ([Unreleased]: v$prev → v$version, owner=$owner_repo)"
}

# ---- bump --------------------------------------------------------------------

cmd_bump() {
    if [[ $# -ne 1 ]]; then
        err "bump: expected exactly one arg <X.Y.Z>"
        usage >&2
        return 2
    fi
    local version="$1"
    if [[ ! "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[A-Za-z0-9.]+)?$ ]]; then
        err "bump: '$version' is not a SemVer X.Y.Z (or X.Y.Z-prerelease)."
        return 2
    fi

    cd "$REPO_ROOT"

    if ! command -v cargo-release > /dev/null 2>&1; then
        err "cargo-release not installed. Install it with:"
        err "  cargo install --locked cargo-release"
        return 5
    fi

    note "running: cargo release $version --no-tag --no-push --no-confirm --execute"
    cargo release "$version" --no-tag --no-push --no-confirm --execute

    ok "bumped to $version, committed locally (no tag, no push)"
    cat <<EOF

next steps:
  - inspect the bump commit:  git log -1
  - hand off to tester for sign-off
  - on green:                 scripts/release.sh tag $version
EOF
}

# ---- tag ---------------------------------------------------------------------

cmd_tag() {
    if [[ $# -ne 1 ]]; then
        err "tag: expected exactly one arg <X.Y.Z>"
        usage >&2
        return 2
    fi
    local version="$1"
    if [[ ! "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[A-Za-z0-9.]+)?$ ]]; then
        err "tag: '$version' is not a SemVer X.Y.Z."
        return 2
    fi

    cd "$REPO_ROOT"

    # Validate HEAD's subject starts with `release(vX.Y.Z):`. AC #6.
    local subject
    subject="$(git log -1 --pretty=%s)"
    local expected_prefix="release(v$version):"
    if [[ "$subject" != "$expected_prefix"* ]]; then
        err "tag: HEAD subject does not start with '$expected_prefix'"
        err "  got: $subject"
        err "  did you forget to run 'release.sh bump $version' first?"
        return 6
    fi
    ok "HEAD subject prefix '$expected_prefix' matches"

    if ! command -v cargo-release > /dev/null 2>&1; then
        err "cargo-release not installed. Install it with:"
        err "  cargo install --locked cargo-release"
        return 5
    fi

    # `cargo release tag` does NOT take a positional version argument —
    # it derives the tag name from the workspace's `[workspace.package]
    # version` (which the prior `release.sh bump $version` already wrote
    # into Cargo.toml). The earlier shape `cargo release tag $version`
    # crashed with `unexpected argument '$version' found`. v0.22.2
    # dogfood revealed this; pre-fix, the v0.22.0 tester didn't catch
    # it because the test suite stash-validated `cargo release` only
    # against tempdir clones that never reached the tag step.
    note "running: cargo release tag --no-confirm --execute"
    cargo release tag --no-confirm --execute
    note "running: cargo release push --no-confirm --execute"
    cargo release push --no-confirm --execute

    ok "tagged v$version and pushed branch + tag"
    cat <<EOF

next steps:
  - .github/workflows/release.yml is now building binaries for the tag.
  - WAIT for that workflow to finish (check 'gh run list --workflow release.yml').
  - then:                     scripts/release-gh.sh v$version
EOF
}

# ---- main --------------------------------------------------------------------

if [[ $# -lt 1 ]]; then
    usage >&2
    exit 2
fi

sub="$1"
shift

case "$sub" in
    preflight)  cmd_preflight "$@" ;;
    bump)       cmd_bump "$@" ;;
    tag)        cmd_tag "$@" ;;
    --help|-h|help) usage; exit 0 ;;
    *)          err "unknown subcommand: $sub"
                usage >&2
                exit 2 ;;
esac
