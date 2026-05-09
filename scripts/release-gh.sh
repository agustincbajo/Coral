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
#     We extract the leading bold span: text from the OPENING `**` up to the
#     CLOSING `**` that terminates the title.
#   - fall back to "Coral vX.Y.Z" if no bold prefix is found.
#
# Nesting policy (MEDIUM 3 in the v0.22.0 tester audit): the leading bold
# span MUST NOT contain a nested `**bold**` run. A line like
#   **Feature release: This is **really** important.** body
# is ambiguous — the naïve `[^*]+` greedy match stops at the first inner
# `**`, truncating the title. We DETECT this case (>=4 `**` markers on
# the line) and exit with a clear error so the maintainer can refactor
# the CHANGELOG line. Use a single emphasis style (e.g. backticks for
# code, asterisks ONLY for the outer title bold).
title=""
first_body_line=""
while IFS= read -r line; do
    if [[ -z "$line" ]]; then continue; fi
    if [[ "$line" =~ ^##\  ]]; then continue; fi
    first_body_line="$line"
    break
done < "$notes_file"

if [[ "$first_body_line" =~ ^\*\* ]]; then
    # MEDIUM 3 (v0.22.0 tester audit): the title's leading bold span MUST
    # NOT contain a nested `**…**` run. Markdown's rendered semantics say
    # the FIRST `**` after the opener closes the bold span. A line like
    #   **Feature release: This is **really** important.** rest
    # has 4 `**` markers — markdown renders it as
    #   <b>Feature release: This is </b>really<b> important.</b> rest
    # which is almost certainly not what the maintainer intended (the
    # intended bold ran from start to "important.**"). We DETECT this
    # case via the heuristic "title ends in whitespace" (the truncated
    # title `Feature release: This is ` always ends with a space because
    # the closer-`**` came mid-sentence) and fail loud with remediation
    # hints. Maintainer's options:
    #   1. Drop the inner `**…**` run (use backticks or *italic* instead).
    #   2. Put the title on its own line — `**Title.**\n\nBody prose…`.
    #
    # We compute the title via the markdown-spec FIRST-marker rule:
    # strip the leading `**`, then strip everything from the FIRST `**`
    # onwards.
    rest_after_opener="${first_body_line#\*\*}"
    if [[ "$rest_after_opener" != *"**"* ]]; then
        err "first body line opens with '**' but has no closing '**'; cannot extract title"
        err "  line was: $first_body_line"
        exit 8
    fi
    # `${var%%\*\**}` = strip LONGEST trailing suffix matching `**<anything>`,
    # leaving everything BEFORE the first `**` close.
    leading_span="${rest_after_opener%%\*\**}"
    if [[ -z "$leading_span" ]]; then
        err "first body line opens with '****' (empty bold title); refusing to guess a title"
        err "  line was: $first_body_line"
        exit 8
    fi
    # MEDIUM 3 detection: a leading_span that ends with whitespace is the
    # smoking gun for a nested-`**` truncation — legitimate titles end at
    # a closing `**` placed AFTER terminal punctuation (`.**`, `)**`,
    # `\`**`), never after a space. Trailing newlines are ruled out
    # already (we read line-by-line with IFS=$'\n').
    if [[ "$leading_span" =~ [[:space:]]$ ]]; then
        err "title appears truncated by a nested '**...**' span on the first body line."
        err "  extracted: '${leading_span}'"
        err "  remediation: rewrite the line so the title's bold span is unambiguous —"
        err "    (a) drop the inner '**...**' (use backticks or *italic* instead), or"
        err "    (b) put the title on its own line, with body prose starting on the next line."
        err "  full line: $first_body_line"
        exit 8
    fi
    # Trim trailing period — looks better as a release title.
    title="${leading_span%.}"
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
