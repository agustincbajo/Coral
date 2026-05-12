# Coral developer environment bootstrap (Windows / PowerShell).
#
# Idempotent. Run once per checkout to install the disk-management
# tooling described in docs/DEVELOPMENT.md and wire sccache into your
# global cargo config.
#
# Re-run cost: ~5 seconds when everything is installed and wired.

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

function Step($msg) { Write-Host "==> $msg" -ForegroundColor Blue }
function Ok($msg)   { Write-Host "[OK] $msg" -ForegroundColor Green }
function Warn($msg) { Write-Host "[!]  $msg" -ForegroundColor Yellow }

Step "Coral developer bootstrap"

# ---------- 1. tooling ----------
#
# Get-Command guard before install: `cargo install` would replace the
# existing binary even when versions match, which fails on Windows
# whenever sccache.exe is acting as the live rustc-wrapper (file lock
# denies the rename). Skip when present; force an update with
# `cargo install --force <tool>`.

function Install-IfMissing([string]$Tool) {
  if (Get-Command $Tool -ErrorAction SilentlyContinue) {
    Ok "$Tool already installed ($((Get-Command $Tool).Source))"
  } else {
    Step "installing $Tool"
    cargo install --locked $Tool
    if ($LASTEXITCODE -ne 0) { throw "cargo install $Tool failed" }
    Ok "$Tool installed"
  }
}

Install-IfMissing "cargo-sweep"
Install-IfMissing "sccache"
Install-IfMissing "cargo-nextest"

# ---------- 2. global cargo config ----------

$CargoHome = if ($env:CARGO_HOME) { $env:CARGO_HOME } else { Join-Path $env:USERPROFILE ".cargo" }
$GlobalConfig = Join-Path $CargoHome "config.toml"
New-Item -ItemType Directory -Path $CargoHome -Force | Out-Null

$existing = if (Test-Path $GlobalConfig) { Get-Content $GlobalConfig -Raw } else { "" }
$wrapperMatch = [regex]::Match($existing, '(?m)^rustc-wrapper\s*=\s*"([^"]+)"')

if ($wrapperMatch.Success -and $wrapperMatch.Groups[1].Value -eq "sccache") {
  Ok "sccache already wired into $GlobalConfig"
} elseif ($wrapperMatch.Success) {
  Warn "leaving existing rustc-wrapper=`"$($wrapperMatch.Groups[1].Value)`" in $GlobalConfig (replace manually to switch to sccache)"
} else {
  Step "wiring sccache into $GlobalConfig"
  if ($existing -match '(?m)^\[build\]') {
    $patched = $existing -replace '(?m)^\[build\]\r?\n', "[build]`r`nrustc-wrapper = `"sccache`"`r`n"
    Set-Content -Path $GlobalConfig -Value $patched -NoNewline
  } else {
    Add-Content -Path $GlobalConfig -Value "`r`n[build]`r`nrustc-wrapper = `"sccache`""
  }
  Ok "sccache wired into $GlobalConfig"
}

# ---------- 3. sccache cache cap ----------

if (-not $env:SCCACHE_CACHE_SIZE) {
  Warn "SCCACHE_CACHE_SIZE not set in your environment — defaults to 10G. To cap at 5G:"
  Warn "  [Environment]::SetEnvironmentVariable('SCCACHE_CACHE_SIZE','5G','User')"
}

# ---------- 4. summary ----------

@"

────────────────────────────────────────────────────────────────────
Setup complete. Maintenance commands you'll use:

  cargo sweep --time 7      # weekly cleanup; 0.5–3 GB
  cargo sweep --installed   # aggressive; 5–15 GB, slower next build
  cargo clean               # nuclear; ~3 min next build
  sccache --show-stats      # cache hit rate (healthy >= 60%)

Disk budget targets:
  target/                                         < 5 GB healthy, > 15 GB → sweep
  %USERPROFILE%\.cargo\registry\                  < 1 GB healthy
  %LOCALAPPDATA%\Mozilla\sccache\                 10 GB cap (configurable via SCCACHE_CACHE_SIZE)
  crates\coral-ui\assets\src\node_modules\        ~150 MB healthy

Full reference:  docs\DEVELOPMENT.md
────────────────────────────────────────────────────────────────────
"@ | Write-Host
