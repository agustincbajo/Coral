# Coral developer disk-hygiene umbrella (Windows sibling of dev-cleanup.sh).
#
# Wraps the four maintenance commands documented in
# docs/DEVELOPMENT.md (cargo sweep --time 7, cargo sweep --installed,
# cargo clean, cargo cache --autoclean) into one command with a
# self-selecting mode based on current target/ size.
#
# Empirical anchor: a 2026-05-13 autonomous session let target/ inflate
# to 45.3 GiB before this script existed. With the auto-mode below, a
# comparable session triggers --hard well before that.
#
# Usage:
#   .\scripts\dev-cleanup.ps1                # default: -Mode check
#   .\scripts\dev-cleanup.ps1 -Mode soft     # cargo sweep --time 7
#   .\scripts\dev-cleanup.ps1 -Mode medium   # cargo sweep --installed
#   .\scripts\dev-cleanup.ps1 -Mode hard     # cargo clean
#   .\scripts\dev-cleanup.ps1 -Mode auto     # pick by current size
#   .\scripts\dev-cleanup.ps1 -Budget 15     # exit 1 if target gt 15 GiB
#
# Auto thresholds (mirrors dev-cleanup.sh):
#   target/ lt  5 GiB  -> check  (no action)
#   target/  5-15 GiB  -> soft   (cargo sweep --time 7)
#   target/ 15-30 GiB  -> medium (cargo sweep --installed)
#   target/ gt 30 GiB  -> hard   (cargo clean)
#
# See: docs/DEVELOPMENT.md "Disk budget" and "Maintenance commands".

[CmdletBinding()]
param(
    [ValidateSet('check','soft','medium','hard','auto')]
    [string]$Mode = 'check',
    [int]$Budget = 0,
    [switch]$NoRegistry
)

$ErrorActionPreference = 'Stop'

# ---------- helpers (ASCII-only strings; PS 5.1 reads UTF-8 sans-BOM as cp1252) ----------

function Step([string]$msg) { Write-Host "==> $msg" -ForegroundColor Blue }
function Ok([string]$msg)   { Write-Host "[ok] $msg" -ForegroundColor Green }
function Warn([string]$msg) { Write-Host "[!] $msg" -ForegroundColor Yellow }
function Fail([string]$msg) { Write-Host "[FAIL] $msg" -ForegroundColor Red }

function Format-Bytes([int64]$b) {
    if      ($b -gt 1GB) { '{0:N2} GiB' -f ($b / 1GB) }
    elseif  ($b -gt 1MB) { '{0:N1} MiB' -f ($b / 1MB) }
    elseif  ($b -gt 1KB) { '{0:N1} KiB' -f ($b / 1KB) }
    else                 { "$b B" }
}

function Get-TargetBytes {
    if (-not (Test-Path 'target')) { return [int64]0 }
    $sum = (Get-ChildItem -Path 'target' -Recurse -Force -ErrorAction SilentlyContinue |
            Measure-Object -Property Length -Sum).Sum
    if ($null -eq $sum) { return [int64]0 } else { return [int64]$sum }
}

function Resolve-AutoMode([int64]$bytes) {
    $gib = $bytes / 1GB
    if      ($gib -lt  5) { 'check' }
    elseif  ($gib -lt 15) { 'soft' }
    elseif  ($gib -lt 30) { 'medium' }
    else                  { 'hard' }
}

# ---------- repo-root guard ----------

if (-not (Test-Path 'Cargo.toml') -or
    -not (Get-Content 'Cargo.toml' -ErrorAction SilentlyContinue |
          Select-String -Pattern '^\[workspace\]' -Quiet)) {
    Fail "run from the Coral repo root (Cargo.toml with [workspace] not found)"
    exit 2
}

# ---------- main ----------

$before = Get-TargetBytes
Step ("current target/ size: " + (Format-Bytes $before))

if ($Mode -eq 'auto') {
    $Mode = Resolve-AutoMode $before
    Step "auto-mode resolved to: -Mode $Mode"
}

switch ($Mode) {
    'check' {
        # No mutations. Report breakdown of top sub-dirs.
        # Skip cargo registry du here: it scans tens of thousands of
        # small files (~60s on Windows even when the dir is 1.5 GiB);
        # the registry is cargo-cache's job, run during mutation modes.
        if ((Test-Path 'target') -and $before -gt 0) {
            Step "top sub-dirs (size descending, top 8):"
            Get-ChildItem -Path 'target' -Force | Where-Object PSIsContainer | ForEach-Object {
                $n = $_.Name
                $s = (Get-ChildItem -Path $_.FullName -Recurse -Force -ErrorAction SilentlyContinue |
                      Measure-Object -Property Length -Sum).Sum
                if ($null -eq $s) { $s = 0 }
                [PSCustomObject]@{ Dir = "target/$n"; Size = Format-Bytes $s; Bytes = [int64]$s }
            } | Sort-Object Bytes -Descending | Select-Object Dir, Size -First 8 | Format-Table -AutoSize
        }
    }

    'soft' {
        if (-not (Get-Command cargo-sweep -ErrorAction SilentlyContinue)) {
            Warn "cargo-sweep not installed - run .\scripts\dev-setup.ps1"
            exit 3
        }
        Step "cargo sweep --time 7 (drops artifacts older than 7 days)"
        & cargo sweep --time 7
    }

    'medium' {
        if (-not (Get-Command cargo-sweep -ErrorAction SilentlyContinue)) {
            Warn "cargo-sweep not installed - run .\scripts\dev-setup.ps1"
            exit 3
        }
        Step "cargo sweep --installed (keeps installed-toolchain artifacts only)"
        & cargo sweep --installed
    }

    'hard' {
        Step "cargo clean (everything; next build approx 3 min)"
        & cargo clean
    }
}

# ---------- registry sweep (unless -NoRegistry) ----------

if (-not $NoRegistry -and $Mode -ne 'check') {
    if (Get-Command cargo-cache -ErrorAction SilentlyContinue) {
        Step "cargo cache --autoclean (registry pruning)"
        & cargo cache --autoclean
    } else {
        Warn "cargo-cache not installed - skipping registry prune"
        Warn "  install with: cargo install --locked cargo-cache"
    }
}

# ---------- after-report ----------

$after = Get-TargetBytes
if ($Mode -ne 'check') {
    $freed = $before - $after
    Ok ("target/ size after: " + (Format-Bytes $after) + " (freed " + (Format-Bytes $freed) + ")")
}

# ---------- budget gate ----------

if ($Budget -gt 0) {
    $afterGib = [math]::Round($after / 1GB, 2)
    if ($afterGib -gt $Budget) {
        Fail ("target/ is " + $afterGib + " GiB exceeds budget " + $Budget + " GiB")
        exit 1
    }
    Ok ("target/ " + $afterGib + " GiB within budget " + $Budget + " GiB")
}

exit 0
