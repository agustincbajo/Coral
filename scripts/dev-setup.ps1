# Coral developer environment bootstrap (Windows / PowerShell).
#
# Idempotent. Run once per checkout to install the disk-management
# tooling described in docs/DEVELOPMENT.md and wire sccache into your
# global cargo config.
#
# What this script does:
#   1. Installs cargo-sweep, sccache, cargo-nextest if missing.
#   2. Adds `rustc-wrapper = "sccache"` to %USERPROFILE%\.cargo\config.toml
#      (only if not already set).
#   3. Prints disk-budget targets and the maintenance commands.
#
# What this script does NOT do:
#   - Modify your $PROFILE.
#   - Set CARGO_INCREMENTAL globally (the repo's .cargo/config.toml
#     handles that for this checkout).
#   - Run `cargo build`.
#
# Re-run cost: ~5 seconds if everything is installed.

$ErrorActionPreference = "Stop"

function Step($msg) { Write-Host "==> $msg" -ForegroundColor Blue }
function Ok($msg)   { Write-Host "[OK] $msg" -ForegroundColor Green }
function Warn($msg) { Write-Host "[!]  $msg" -ForegroundColor Yellow }

Step "Coral developer bootstrap"

# ---------- 1. tooling ----------

function Install-IfMissing {
  param([string]$Tool, [string]$Crate = $null)
  if (-not $Crate) { $Crate = $Tool }
  if (Get-Command $Tool -ErrorAction SilentlyContinue) {
    Ok "$Tool already installed ($((Get-Command $Tool).Source))"
  } else {
    Step "installing $Crate"
    cargo install --locked $Crate
    if ($LASTEXITCODE -ne 0) { throw "cargo install $Crate failed" }
    Ok "$Crate installed"
  }
}

Install-IfMissing "cargo-sweep"    "cargo-sweep"
Install-IfMissing "sccache"        "sccache"
Install-IfMissing "cargo-nextest"  "cargo-nextest"

# ---------- 2. global cargo config ----------

$CargoHome = if ($env:CARGO_HOME) { $env:CARGO_HOME } else { Join-Path $env:USERPROFILE ".cargo" }
$GlobalConfig = Join-Path $CargoHome "config.toml"

if (-not (Test-Path $CargoHome)) {
  New-Item -ItemType Directory -Path $CargoHome -Force | Out-Null
}
if (-not (Test-Path $GlobalConfig)) {
  New-Item -ItemType File -Path $GlobalConfig | Out-Null
}

$existing = Get-Content $GlobalConfig -Raw -ErrorAction SilentlyContinue
if ($null -eq $existing) { $existing = "" }

if ($existing -match '(?m)^rustc-wrapper\s*=\s*"sccache"') {
  Ok "sccache already wired into $GlobalConfig"
} else {
  Step "wiring sccache into $GlobalConfig"
  if ($existing -match '(?m)^\[build\]') {
    # [build] section exists — insert the rustc-wrapper line just after it.
    $patched = $existing -replace '(?m)^\[build\]\r?\n', "[build]`r`nrustc-wrapper = `"sccache`"`r`n"
    Set-Content -Path $GlobalConfig -Value $patched -NoNewline
  } else {
    # Append a fresh stanza.
    Add-Content -Path $GlobalConfig -Value "`r`n[build]`r`nrustc-wrapper = `"sccache`""
  }
  Ok "sccache wired into $GlobalConfig"
}

# ---------- 3. summary ----------

@"

────────────────────────────────────────────────────────────────────
Setup complete. Maintenance commands you'll use:

  cargo sweep --installed   # quick cleanup, recovers 5–15 GB typically
  cargo sweep --time 7      # weekly, recovers 0.5–3 GB
  cargo clean               # nuclear, recovers everything (~3 min next build)
  sccache --show-stats      # cache hit rate (healthy is >= 60%)

Disk budget targets:
  target/                                         < 5 GB healthy
  %USERPROFILE%\.cargo\registry\                  < 1 GB healthy
  %LOCALAPPDATA%\Mozilla\sccache\                 10 GB cap (configurable)
  crates\coral-ui\assets\src\node_modules\        ~150 MB healthy

Full reference:  docs\DEVELOPMENT.md
────────────────────────────────────────────────────────────────────
"@ | Write-Host
