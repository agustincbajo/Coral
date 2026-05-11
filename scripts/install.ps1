# Coral one-line installer for Windows (PowerShell 5.1+).
#
# Downloads the latest x86_64-pc-windows-msvc release zip, verifies the
# SHA-256, extracts coral.exe to $env:LOCALAPPDATA\Coral\bin, prepends
# that dir to the user PATH if missing, and prints the two lines a user
# should paste into Claude Code to install the plugin.
#
# Usage (one-liner):
#   iwr -useb https://raw.githubusercontent.com/agustincbajo/Coral/main/scripts/install.ps1 | iex
#
# Or with an explicit version:
#   $v = "v0.30.0"
#   & ([scriptblock]::Create((iwr -useb https://raw.githubusercontent.com/agustincbajo/Coral/main/scripts/install.ps1).Content)) -Version $v
#
# Idempotent: re-running over the same version is a no-op.

param(
  [string]$Version = "",
  [string]$InstallDir = "$env:LOCALAPPDATA\Coral\bin"
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$Repo       = "agustincbajo/Coral"
$ApiLatest  = "https://api.github.com/repos/$Repo/releases/latest"
$DownloadDl = "https://github.com/$Repo/releases/download"
$Target     = "x86_64-pc-windows-msvc"

function Write-NextSteps {
  Write-Host ""
  Write-Host "Now, inside Claude Code, paste:"
  Write-Host ""
  Write-Host "  /plugin marketplace add agustincbajo/Coral"
  Write-Host "  /plugin install coral@coral"
  Write-Host ""
  Write-Host 'Then ask Claude: "set up Coral for this repo" - the plugin takes over.'
}

# --- arch check ------------------------------------------------------------

# We only ship x86_64 on Windows. ARM64 users should use `cargo install`.
$arch = $env:PROCESSOR_ARCHITECTURE
if ($arch -ne "AMD64" -and $arch -ne "x86_64") {
  Write-Error @"
Unsupported Windows arch: $arch.
Coral ships x86_64-pc-windows-msvc only. Install via cargo instead:
    cargo install --locked --git https://github.com/$Repo coral-cli
"@
}

# --- resolve version -------------------------------------------------------

if (-not $Version) {
  try {
    $rel = Invoke-RestMethod -Uri $ApiLatest -UseBasicParsing
    $Version = $rel.tag_name
  } catch {
    Write-Error "Failed to query $ApiLatest. Pass -Version vX.Y.Z explicitly. Error: $_"
  }
}

if (-not $Version) {
  Write-Error "Could not resolve a release tag from GitHub."
}

# --- skip if already installed --------------------------------------------

$ExePath = Join-Path $InstallDir "coral.exe"
if (Test-Path $ExePath) {
  try {
    $installed = ((& $ExePath --version 2>$null) | Out-String).Trim() -replace '^coral\s+',''
    if ($installed -and (("v$installed" -eq $Version) -or ($installed -eq $Version))) {
      Write-Host "coral $Version already installed at $ExePath - nothing to do."
      Write-NextSteps
      exit 0
    }
  } catch {
    # Fall through and reinstall.
  }
}

# --- download -------------------------------------------------------------

$Base      = "coral-$Version-$Target"
$ZipName   = "$Base.zip"
$ShaName   = "$ZipName.sha256"
$UrlZip    = "$DownloadDl/$Version/$ZipName"
$UrlSha    = "$DownloadDl/$Version/$ShaName"

$TmpDir    = Join-Path ([System.IO.Path]::GetTempPath()) "coral-install-$([System.Guid]::NewGuid().ToString('N'))"
New-Item -ItemType Directory -Path $TmpDir -Force | Out-Null

$ZipPath   = Join-Path $TmpDir $ZipName
$ShaPath   = Join-Path $TmpDir $ShaName

try {
  Write-Host "Downloading $ZipName ..."
  Invoke-WebRequest -Uri $UrlZip -OutFile $ZipPath -UseBasicParsing
  Invoke-WebRequest -Uri $UrlSha -OutFile $ShaPath -UseBasicParsing

  # --- verify SHA-256 ----------------------------------------------------

  Write-Host "Verifying SHA-256 ..."
  $expected = (Get-Content $ShaPath -Raw).Trim().Split()[0].ToLowerInvariant()
  $actual   = (Get-FileHash -Algorithm SHA256 -Path $ZipPath).Hash.ToLowerInvariant()
  if ($expected -ne $actual) {
    throw "SHA-256 mismatch for $ZipName. expected=$expected actual=$actual"
  }

  # --- extract -----------------------------------------------------------

  Write-Host "Extracting ..."
  $ExtractDir = Join-Path $TmpDir "extract"
  New-Item -ItemType Directory -Path $ExtractDir -Force | Out-Null
  Expand-Archive -Path $ZipPath -DestinationPath $ExtractDir -Force

  # Release layout: <base>/coral.exe inside the zip.
  $Src = Join-Path $ExtractDir (Join-Path $Base "coral.exe")
  if (-not (Test-Path $Src)) {
    # Fallback: search anywhere in the extracted tree.
    $candidate = Get-ChildItem -Path $ExtractDir -Recurse -Filter "coral.exe" | Select-Object -First 1
    if (-not $candidate) {
      throw "coral.exe not found in $ZipName."
    }
    $Src = $candidate.FullName
  }

  # --- install ----------------------------------------------------------

  New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
  Copy-Item -Path $Src -Destination $ExePath -Force

} finally {
  Remove-Item -Recurse -Force -Path $TmpDir -ErrorAction SilentlyContinue
}

# --- PATH prepend (user scope) --------------------------------------------

$UserPath = [Environment]::GetEnvironmentVariable("Path", "User")
if (-not $UserPath) { $UserPath = "" }
$PathParts = $UserPath -split ';' | Where-Object { $_ -ne "" }
if (-not ($PathParts -contains $InstallDir)) {
  $NewPath = ($InstallDir + ';' + $UserPath).TrimEnd(';')
  [Environment]::SetEnvironmentVariable("Path", $NewPath, "User")
  # Also update the current session so the post-install `coral --version` works.
  $env:Path = $InstallDir + ';' + $env:Path
  Write-Host "Prepended $InstallDir to user PATH. Open a new terminal for it to take effect in other shells."
}

# --- post-install ---------------------------------------------------------

Write-Host ""
Write-Host "Installed: $ExePath"
try { & $ExePath --version } catch { }

Write-NextSteps
