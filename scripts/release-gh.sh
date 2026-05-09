#!/usr/bin/env bash
# scripts/release-gh.sh — finalize a GitHub release with hand-written notes.
#
# Usage:
#   scripts/release-gh.sh vX.Y.Z
#
# Run this AFTER `release.sh tag X.Y.Z` has pushed the tag AND
# `.github/workflows/release.yml` has finished building binaries (check with
# `gh run list --workflow release.yml`). The workflow auto-creates the release
# with auto-generated notes; this script overwrites those notes with the
# verbatim CHANGELOG section, preserving uploaded binaries.
#
# What it does:
#   1. Verifies `gh` is on PATH and authenticated.
#   2. Verifies `git rev-parse vX.Y.Z` resolves (tag exists locally).
#   3. Extracts the CHANGELOG section into /tmp/coral-release-vX.Y.Z.md.
#   4. Reads the first non-blank line of the section (the
#      `**Feature release: ...**` bold line) for the release title; strips
#      surrounding asterisks.
#   5. If a GH release for the tag already exists (the workflow's auto-create),
#      runs `gh release edit` to replace title + body. Otherwise runs
#      `gh release create` with the extracted notes.
#   6. Echoes the GH release URL.
#
# Set GH_DRY_RUN=1 to print what would be done without invoking gh.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

err() { printf 'error: %s\n' "$1" >&2; }

if [[ $# -ne 1 ]]; then
    err "usage: scripts/release-gh.sh vX.Y.Z"
    exit 2
fi

raw_tag="$1"
# Accept with or without leading 'v', but always emit `vX.Y.Z` to gh.
if [[ "$raw_tag" =~ ^v ]]; then
    tag="$raw_tag"
    version="${raw_tag#v}"
else
    tag="v$raw_tag"
    version="$raw_tag"
fi

if [[ ! "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[A-Za-z0-9.]+)?$ ]]; then
    err "'$raw_tag' is not a valid X.Y.Z (or vX.Y.Z) version"
    exit 2
fi

DRY="${GH_DRY_RUN:-0}"

if [[ "$DRY" != "1" ]]; then
    if ! command -v gh > /dev/null 2>&1; then
        err "gh (GitHub CLI) not on PATH. Install: https://cli.github.com/"
        exit 5
    fi
    if ! gh auth status > /dev/null 2>&1; then
        err "gh is installed but not authenticated. Run: gh auth login"
        exit 5
    fi
fi

# Tag must exist locally (i.e. release.sh tag was run already).
if ! git -C "$REPO_ROOT" rev-parse "$tag" > /dev/null 2>&1; then
    err "git tag '$tag' does not resolve locally. Run 'release.sh tag $version' first."
    exit 6
fi

notes_file="/tmp/coral-release-${tag}.md"
"$SCRIPT_DIR/extract-changelog-section.sh" "$version" > "$notes_file"
if [[ ! -s "$notes_file" ]]; then
    err "extract-changelog-section.sh produced empty output for $version"
    exit 7
fi

# Title extraction:
#   - skip the `## [X.Y.Z] - YYYY-MM-DD` heading and any blank lines after.
#   - the first body line is by convention `**Feature release: <subject>.** <prose...>`.
#     We extract the leading bold span: text from `**` to the FIRST `**` that closes it.
#   - fall back to "Coral vX.Y.Z" if no bold prefix is found.
title=""
first_body_line=""
while IFS= read -r line; do
    if [[ -z "$line" ]]; then continue; fi
    if [[ "$line" =~ ^##\  ]]; then continue; fi
    first_body_line="$line"
    break
done < "$notes_file"

# Match the leading `**...**` span (greedy until the first closing `**`).
if [[ "$first_body_line" =~ ^\*\*([^*]+(\*[^*]+)*)\*\*  ]]; then
    title="${BASH_REMATCH[1]}"
    # Trim trailing period — looks better as a release title.
    title="${title%.}"
else
    title="Coral $tag"
fi

if [[ "$DRY" == "1" ]]; then
    printf 'DRY RUN — would gh release for %s\n' "$tag"
    printf '  title: %s\n' "$title"
    printf '  notes-file: %s\n' "$notes_file"
    printf '  notes-file size: %d bytes\n' "$(wc -c < "$notes_file" | tr -d ' ')"
    exit 0
fi

# Detect whether a GH release already exists for this tag. The release.yml
# workflow auto-creates it with `generate_release_notes: true`; we replace.
if gh release view "$tag" > /dev/null 2>&1; then
    printf 'updating existing GH release for %s\n' "$tag"
    gh release edit "$tag" --title "$title" --notes-file "$notes_file"
else
    printf 'creating new GH release for %s\n' "$tag"
    gh release create "$tag" --title "$title" --notes-file "$notes_file"
fi

# Echo the URL.
url="$(gh release view "$tag" --json url --jq .url)"
printf '\nrelease URL: %s\n' "$url"
