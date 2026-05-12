# Coral SessionStart hook — Windows PowerShell (FR-ONB-9, PRD v1.4 §6.3).
#
# Invoked by Claude Code on Windows when a session opens in a repo where
# the Coral plugin is enabled. Sibling of `on-session-start.sh` (bash);
# the wrapper script `on-session-start.cmd` (or platform-aware entry in
# plugin.json) picks the right one per OS.
#
# Stdout is captured by Claude Code and injected into the model's
# context — so the CLAUDE.md routing instructions can branch on
# `coral_status`, `wiki_present`, `warnings`, etc.
#
# Budget per PRD §6.3: <400ms p95 on Windows (process spawn is ~50-200ms
# baseline). Hard cap 5s via Wait-Job.
#
# Output contract:
#   * Always exit 0 (degrade gracefully on any error).
#   * Output hard-capped at 8000 chars (Claude Code injects up to 10k of
#     hook stdout into context; exceeding silently truncates with a
#     preview, which would break the CLAUDE.md routing).

$ErrorActionPreference = 'SilentlyContinue'

if (-not (Get-Command coral -ErrorAction SilentlyContinue)) {
  Write-Output '{"coral_status":"binary_missing","suggestion":"run scripts/install.ps1"}'
  exit 0
}

$cwd = if ($env:CLAUDE_PROJECT_DIR) { $env:CLAUDE_PROJECT_DIR } else { Get-Location }
Set-Location -Path $cwd -ErrorAction SilentlyContinue

$job = Start-Job { coral self-check --format=json --quick }
$done = Wait-Job $job -Timeout 5
if (-not $done) {
  Stop-Job $job
  Remove-Job $job -Force
  Write-Output '{"coral_status":"check_failed"}'
  exit 0
}

$output = Receive-Job $job
Remove-Job $job

# Receive-Job returns each emitted line as a separate string; join so
# the 8000-char cap is measured against the full JSON envelope, not the
# longest single line.
if ($output -is [array]) {
  $joined = ($output -join '')
} else {
  $joined = [string]$output
}

if ($joined.Length -gt 8000) {
  Write-Output '{"coral_status":"ok","note":"full output truncated; run /coral:coral-doctor"}'
} else {
  Write-Output $joined
}
exit 0
