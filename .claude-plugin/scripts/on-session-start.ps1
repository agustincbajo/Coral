# Coral SessionStart hook — Windows PowerShell (FR-ONB-9, PRD v1.4 §6.3).
#
# Invoked by Claude Code on Windows when a session opens in a repo where
# the Coral plugin is enabled. Sibling of `on-session-start.sh` (bash);
# the wrapper script `on-session-start` (no extension) picks the right
# one per OS via a `case` on `$OS`/`$OSTYPE`/`uname -s`.
#
# Stdout is captured by Claude Code and injected into the model's
# context — so the CLAUDE.md routing instructions can branch on
# `coral_status`, `wiki_present`, `warnings`, etc.
#
# Budget per PRD §6.3: <600ms p95 on Windows. Hard cap 5s via
# `Process.WaitForExit($ms)`.
#
# **v0.34.x rewrite**: the previous implementation used
# `Start-Job { ... } | Wait-Job -Timeout 5 | Receive-Job`. PowerShell
# jobs spawn a secondary `powershell.exe` host to host the scriptblock,
# which costs ~500ms of cold-start on Windows runners — the v0.34.0
# Hook-budget CI measured mean=651ms ± 84ms, max=941ms, forcing us to
# raise the threshold to 1200ms. This rewrite drops to a single
# `Diagnostics.Process` invocation with stdout/stderr redirected to
# strings; no secondary PS host means we eliminate the bulk of that
# floor and can put the budget back at 600ms.
#
# Compatibility notes:
#   * Runs under Windows PowerShell 5.1 (`powershell.exe -NoProfile
#     -ExecutionPolicy Bypass -File ...`) — no PowerShell 7-only
#     features (no ternary, no `??`, no `Start-ThreadJob`).
#   * `System.Diagnostics.Process` + `WaitForExit(int)` is the canonical
#     timeout-with-kill primitive in .NET Framework 4.x (the runtime
#     PS 5.1 binds to) and is documented stable across all current
#     Windows versions.
#
# Output contract:
#   * Always exit 0 (degrade gracefully on any error).
#   * Output hard-capped at 8000 chars (Claude Code injects up to 10k of
#     hook stdout into context; exceeding silently truncates with a
#     preview, which would break the CLAUDE.md routing).

$ErrorActionPreference = 'SilentlyContinue'

# Resolve `coral` on PATH. Get-Command is faster than spawning a probe
# process for the missing-binary branch.
$coralCmd = Get-Command coral -ErrorAction SilentlyContinue
if ($null -eq $coralCmd) {
  Write-Output '{"coral_status":"binary_missing","suggestion":"run scripts/install.ps1"}'
  exit 0
}

# Switch into the Claude-provided repo root when present. The hook can
# also be invoked manually for testing; in that case `Get-Location`
# wins.
$cwd = if ($env:CLAUDE_PROJECT_DIR) { $env:CLAUDE_PROJECT_DIR } else { (Get-Location).Path }
Set-Location -Path $cwd -ErrorAction SilentlyContinue

# Build a Process directly. Key choices:
#   * UseShellExecute=false: required to redirect stdout/stderr.
#   * RedirectStandardOutput/Error=true: we collect output synchronously
#     after WaitForExit so the 8000-char cap is measured against the
#     full envelope.
#   * CreateNoWindow=true: prevents a transient console flash on Windows
#     terminals that don't already host the parent process.
#   * FileName=$coralCmd.Source: resolve to the absolute path once so
#     we don't pay the PATH walk twice.
$psi = New-Object System.Diagnostics.ProcessStartInfo
$psi.FileName = $coralCmd.Source
$psi.Arguments = 'self-check --format=json --quick'
$psi.UseShellExecute = $false
$psi.RedirectStandardOutput = $true
$psi.RedirectStandardError = $true
$psi.CreateNoWindow = $true
$psi.WorkingDirectory = $cwd

$proc = New-Object System.Diagnostics.Process
$proc.StartInfo = $psi

# PRD spec: 5-second hard cap. `WaitForExit($ms)` returns $true if the
# process exited within the window, $false on timeout — in which case
# we Kill() to release the handles and emit the `check_failed` envelope.
$timeoutMs = 5000

# Async stdout/stderr read avoids the classic Win32 deadlock where the
# child fills the redirected stdout pipe (~4KB on Windows) and blocks
# waiting for the parent to drain, while the parent is blocked on
# WaitForExit. Begin*ReadLine wires up an event loop so the OS drains
# the pipes into in-memory buffers as bytes arrive.
$stdoutSb = New-Object System.Text.StringBuilder
$stderrSb = New-Object System.Text.StringBuilder
$stdoutHandler = {
  param($sender, $e)
  if ($null -ne $e.Data) { [void]$Event.MessageData.AppendLine($e.Data) }
}
$stderrHandler = {
  param($sender, $e)
  if ($null -ne $e.Data) { [void]$Event.MessageData.AppendLine($e.Data) }
}
$stdoutEvent = Register-ObjectEvent -InputObject $proc -EventName 'OutputDataReceived' -Action $stdoutHandler -MessageData $stdoutSb
$stderrEvent = Register-ObjectEvent -InputObject $proc -EventName 'ErrorDataReceived'  -Action $stderrHandler -MessageData $stderrSb

$started = $false
try {
  $started = $proc.Start()
  $proc.BeginOutputReadLine()
  $proc.BeginErrorReadLine()
}
catch {
  # Could not spawn (coral.exe was deleted between Get-Command and
  # Process.Start, or AV blocked it). Degrade to the same envelope as
  # a check failure — the binary_missing branch is reserved for the
  # explicit "not on PATH" case detected above.
  Unregister-Event -SourceIdentifier $stdoutEvent.Name -ErrorAction SilentlyContinue
  Unregister-Event -SourceIdentifier $stderrEvent.Name -ErrorAction SilentlyContinue
  Write-Output '{"coral_status":"check_failed"}'
  exit 0
}

if (-not $started) {
  Unregister-Event -SourceIdentifier $stdoutEvent.Name -ErrorAction SilentlyContinue
  Unregister-Event -SourceIdentifier $stderrEvent.Name -ErrorAction SilentlyContinue
  Write-Output '{"coral_status":"check_failed"}'
  exit 0
}

$exitedInTime = $proc.WaitForExit($timeoutMs)
if (-not $exitedInTime) {
  # Timeout: kill the child to release file handles + return the
  # PRD-mandated degraded envelope. Kill() on a non-existent process
  # throws; we swallow that — the desired post-state is "process is
  # gone" and a race with self-exit gets there anyway.
  try { $proc.Kill() } catch { }
  try { $proc.WaitForExit(1000) | Out-Null } catch { }
  Unregister-Event -SourceIdentifier $stdoutEvent.Name -ErrorAction SilentlyContinue
  Unregister-Event -SourceIdentifier $stderrEvent.Name -ErrorAction SilentlyContinue
  Write-Output '{"coral_status":"check_failed"}'
  exit 0
}

# WaitForExit($ms)=true only guarantees the process tree exited; the
# parameterless WaitForExit() additionally flushes the async output
# event handlers. Without it the last few hundred bytes of stdout can
# race the script's read of $stdoutSb. See dotnet/runtime#21996.
$proc.WaitForExit()
Unregister-Event -SourceIdentifier $stdoutEvent.Name -ErrorAction SilentlyContinue
Unregister-Event -SourceIdentifier $stderrEvent.Name -ErrorAction SilentlyContinue

if ($proc.ExitCode -ne 0) {
  Write-Output '{"coral_status":"check_failed"}'
  exit 0
}

# Trim the trailing newline AppendLine adds — the cap is measured
# against the JSON envelope alone, not pretty-printed lines.
$joined = $stdoutSb.ToString().TrimEnd("`r", "`n")

if ($joined.Length -gt 8000) {
  Write-Output '{"coral_status":"ok","note":"full output truncated; run /coral:coral-doctor"}'
} else {
  Write-Output $joined
}
exit 0
