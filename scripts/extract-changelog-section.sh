#!/usr/bin/env bash
# scripts/extract-changelog-section.sh — print the changelog section for a single version.
#
# Usage:
#   scripts/extract-changelog-section.sh <X.Y.Z> [PATH]
#
# Args:
#   <X.Y.Z>   the version, with or without a leading 'v' (e.g. 0.21.3 or v0.21.3).
#   [PATH]    path to the changelog (default: CHANGELOG.md, relative to repo root).
#
# Output (stdout): the section starting at `## [X.Y.Z]` line, ending BEFORE the
# next `## [` line. The starting heading line itself is included; the next-version
# heading is not.
#
# Output (stderr): empty on success.
#
# Exit:
#   0  section found and printed.
#   1  version not found in changelog.
#   2  bad invocation.

set -euo pipefail

if [[ $# -lt 1 || $# -gt 2 ]]; then
    cat >&2 <<'EOF'
Usage: scripts/extract-changelog-section.sh <X.Y.Z> [PATH]

  X.Y.Z   version (with or without leading 'v')
  PATH    optional CHANGELOG path (default: CHANGELOG.md at repo root)
EOF
    exit 2
fi

raw="$1"
# Strip optional leading 'v'.
version="${raw#v}"

# Validate SemVer-ish.
if [[ ! "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[A-Za-z0-9.]+)?$ ]]; then
    printf 'error: %s is not a valid X.Y.Z version\n' "$raw" >&2
    exit 2
fi

if [[ $# -eq 2 ]]; then
    path="$2"
else
    SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
    REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
    path="$REPO_ROOT/CHANGELOG.md"
fi

if [[ ! -r "$path" ]]; then
    printf 'error: cannot read %s\n' "$path" >&2
    exit 2
fi

# Escape dots in the version so awk's regex doesn't treat them as wildcards.
escaped="${version//./\\.}"

# awk strategy:
#   - When we hit `## [X.Y.Z]`, set flag=1 and print the line.
#   - When we hit any OTHER `## [` heading, set flag=0 (and don't print).
#   - While flag is 1, print every line.
#
# This includes the starting heading and excludes the next-version heading.
# Multiple matches of the same version (shouldn't happen, but be safe) only
# print the first; once flag flips off it doesn't flip back on.
output=$(awk -v ver="$escaped" '
    BEGIN { flag = 0; seen = 0 }
    {
        if (match($0, "^## \\[" ver "\\]")) {
            if (seen) next
            flag = 1
            seen = 1
            print
            next
        }
        if (flag && match($0, "^## \\[")) {
            flag = 0
            next
        }
        if (flag) print
    }
' "$path")

if [[ -z "$output" ]]; then
    exit 1
fi

printf '%s\n' "$output"
