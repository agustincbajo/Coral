#!/usr/bin/env bash
# Coral SessionStart hook (FR-ONB-9, PRD v1.4 §6.3).
#
# Invoked by Claude Code when a session opens in a repo where the Coral
# plugin is enabled. Stdout is injected into the model's context (silent
# from the user's perspective) so the CLAUDE.md routing instructions can
# branch on `coral_status`, `wiki_present`, `warnings`, etc.
#
# Budget targets per PRD §6.3:
#   * <150ms p95 Linux/macOS
#   * <400ms p95 Windows (process spawn overhead)
#   * hard cap 5s via `timeout` — if `coral` hangs we degrade gracefully
#
# Output contract:
#   * Always exit 0 (failure modes degrade to a minimal JSON envelope).
#   * Output is hard-capped at 8000 chars to stay under the 10k cap that
#     Claude Code applies to hook stdout. If `coral self-check` exceeds
#     it, we emit a fallback envelope and tell the user to run
#     `/coral:coral-doctor` for the full report.
#
# See `.claude-plugin/scripts/on-session-start.ps1` for the Windows
# sibling that the OS-aware wrapper picks when `$OS == 'Windows_NT'`.

set -u

command -v coral >/dev/null 2>&1 || {
  printf '{"coral_status":"binary_missing","suggestion":"run scripts/install.sh"}'
  exit 0
}

cd "${CLAUDE_PROJECT_DIR:-$PWD}" 2>/dev/null || exit 0

OUTPUT=$(timeout 5 coral self-check --format=json --quick 2>/dev/null) || {
  printf '{"coral_status":"check_failed"}'
  exit 0
}

if [ "${#OUTPUT}" -gt 8000 ]; then
  printf '{"coral_status":"ok","note":"full output truncated; run /coral:coral-doctor"}'
else
  printf '%s' "$OUTPUT"
fi
exit 0
