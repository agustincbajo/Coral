# Coral one-line installer for Windows (PowerShell 5.1+).
#
# Downloads the latest x86_64-pc-windows-msvc release zip, verifies the
# SHA-256, extracts coral.exe to $env:LOCALAPPDATA\Coral\bin, prepends
# that dir to the user PATH if missing, and (optionally) registers the
# Coral marketplace in Claude Code's settings. Prints either the 3
# paste lines or a single "ready" message.
#
# Usage (one-liner):
#   iwr -useb https://raw.githubusercontent.com/agustincbajo/Coral/main/scripts/install.ps1 | iex
#
# Or with explicit args:
#   $v = "v0.34.0"
#   & ([scriptblock]::Create((iwr -useb https://raw.githubusercontent.com/agustincbajo/Coral/main/scripts/install.ps1).Content)) -Version $v -WithClaudeConfig
#
# Idempotent: re-running over the same version is a no-op.
#
# Flags:
#   -Version vX.Y.Z              Pin a specific release tag (skips the
#                                GitHub API "latest" lookup).
#   -InstallDir PATH             Override install dir (default
#                                $env:LOCALAPPDATA\Coral\bin).
#   -WithClaudeConfig            After install, patch the project's
#                                .claude\settings.json via `coral
#                                self-register-marketplace`. Opt-in
#                                per FR-ONB-26.
#   -SkipPluginInstructions      Silent for CI (FR-ONB-2). No final
#                                paste-3-lines or claude-paste.txt.

param(
  [string]$Version = "",
  [string]$InstallDir = "$env:LOCALAPPDATA\Coral\bin",
  [switch]$WithClaudeConfig,
  [switch]$SkipPluginInstructions
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$Repo       = "agustincbajo/Coral"
$ApiLatest  = "https://api.github.com/repos/$Repo/releases/latest"
$DownloadDl = "https://github.com/$Repo/releases/download"
$Target     = "x86_64-pc-windows-msvc"

function Test-ClaudeCli {
  # FR-ONB-1: branch the next-steps message based on whether the user
  # already has Claude Code installed. Get-Command returns $null when
  # the binary is not on PATH and we silence the error.
  $null -ne (Get-Command claude -ErrorAction SilentlyContinue)
}

function Write-ClaudePasteFile {
  # `.coral\claude-paste.txt` lives in the cwd's .coral folder so the
  # user can copy-paste the 3 lines from an editor.
  $coralDir = Join-Path (Get-Location).Path ".coral"
  New-Item -ItemType Directory -Path $coralDir -Force | Out-Null
  $paste = @"
/plugin marketplace add agustincbajo/Coral
/plugin install coral@coral
/reload-plugins
"@
  Set-Content -Path (Join-Path $coralDir "claude-paste.txt") -Value $paste -Encoding utf8
}

function Write-PostInstallMessage {
  if ($SkipPluginInstructions) { return }
  if (-not (Test-ClaudeCli)) {
    Write-Host ""
    Write-Host "Claude Code not installed." -ForegroundColor Yellow
    Write-Host "  Install: https://claude.ai/code  -> run this installer again,"
    Write-Host "  OR use 'coral doctor --wizard' to set up a non-Claude-Code provider"
    Write-Host "  (Anthropic API key, Gemini, or local Ollama)."
    Write-Host ""
    return
  }
  if ($WithClaudeConfig) {
    # FR-ONB-4: --with-claude-config success message.
    Write-Host ""
    Write-Host "Coral installed + marketplace registered." -ForegroundColor Green
    Write-Host "Open Claude Code in your repo and type anything to get started."
    return
  }
  # FR-ONB-4 default: 3 paste lines + claude-paste.txt sidecar.
  Write-ClaudePasteFile
  Write-Host ""
  Write-Host "Next: paste these three lines into Claude Code (one at a time):"
  Write-Host ""
  Write-Host "    /plugin marketplace add agustincbajo/Coral"
  Write-Host "    /plugin install coral@coral"
  Write-Host "    /reload-plugins"
  Write-Host ""
  Write-Host "Then type anything in Claude Code - Coral's CLAUDE.md will guide it."
  Write-Host ""
  Write-Host "(Also saved to .coral\claude-paste.txt for copy-paste from your editor.)"
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
      # FR-ONB-26: still wire marketplace on re-runs when the user
      # asked for it. Idempotent — already-registered exits 0 quietly.
      if ($WithClaudeConfig) {
        try {
          & $ExePath self-register-marketplace --scope=project
        } catch {
          Write-Warning "marketplace registration failed; falling back to paste-3-lines flow: $_"
        }
      }
      Write-PostInstallMessage
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

# FR-ONB-31: Defender SmartScreen hint, printed right after the binary
# is placed on disk so users who hit "this app might harm your device"
# on first run know what to do. We do not ship code-signed binaries
# yet (tracked for v0.35).
Write-Host ""
Write-Host "Windows Defender SmartScreen may block coral.exe on first run." -ForegroundColor Yellow
Write-Host "  If so: right-click -> Properties -> check 'Unblock' -> OK."
Write-Host "  We are working on code signing for v0.35."

# --- PATH prepend (user scope) --------------------------------------------

$UserPath = [Environment]::GetEnvironmentVariable("Path", "User")
if (-not $UserPath) { $UserPath = "" }
$PathParts = $UserPath -split ';' | Where-Object { $_ -ne "" }
if (-not ($PathParts -contains $InstallDir)) {
  $NewPath = ($InstallDir + ';' + $UserPath).TrimEnd(';')
  [Environment]::SetEnvironmentVariable("Path", $NewPath, "User")
  # Also update the current session so the post-install `coral --version` works.
  $env:Path = $InstallDir + ';' + $env:Path
  # FR-ONB-31: the User PATH update doesn't propagate to *other*
  # already-open shells. Loud about it so the user knows to open a
  # fresh terminal instead of confusedly typing `coral` into the old
  # one and getting "not recognized".
  Write-Host ""
  Write-Host "PATH updated for new sessions. Open a NEW PowerShell window to use 'coral'" -ForegroundColor Yellow
  Write-Host "  (current shell still has old PATH outside this script)."
}

# --- post-install ---------------------------------------------------------

Write-Host ""
Write-Host "Installed: $ExePath"
try { & $ExePath --version } catch { }

# FR-ONB-26: opt-in marketplace registration. Failure is non-fatal —
# the paste-3-lines flow is the documented fallback.
if ($WithClaudeConfig) {
  try {
    & $ExePath self-register-marketplace --scope=project
  } catch {
    Write-Warning "marketplace registration failed; falling back to paste-3-lines flow: $_"
  }
}

Write-PostInstallMessage
