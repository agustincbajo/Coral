#!/usr/bin/env bash
# Coral one-line installer for Linux + macOS.
#
# Downloads the latest release tarball matching this host's platform/arch,
# verifies the SHA-256, places `coral` on PATH, removes the macOS
# quarantine xattr, and (optionally) registers the Coral marketplace in
# Claude Code's settings. Prints either the 3 paste lines or a single
# "ready" message depending on `--with-claude-config`.
#
# Idempotent: running twice over the same release is a no-op.
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/agustincbajo/Coral/main/scripts/install.sh | bash
#   curl -fsSL https://raw.githubusercontent.com/agustincbajo/Coral/main/scripts/install.sh | bash -s -- --version v0.34.0
#   curl -fsSL https://raw.githubusercontent.com/agustincbajo/Coral/main/scripts/install.sh | bash -s -- --with-claude-config
#   curl -fsSL https://raw.githubusercontent.com/agustincbajo/Coral/main/scripts/install.sh | bash -s -- --skip-plugin-instructions
#
# Flags:
#   --version vX.Y.Z          Pin a specific release tag (skips the
#                             GitHub API "latest" lookup).
#   --install-dir DIR         Override the install target. Default:
#                             `/usr/local/bin` if writable, else
#                             `~/.local/bin` (mkdir -p).
#   --with-claude-config      After install, patch `.claude/settings.json`
#                             (project scope) with the Coral marketplace
#                             via `coral self-register-marketplace`. Opt-in
#                             per FR-ONB-26 — we never touch user config
#                             without explicit consent.
#   --skip-plugin-instructions
#                             Silent for CI (FR-ONB-2). Skips the final
#                             paste-3-lines / ready message and the
#                             `.coral/claude-paste.txt` write.
#
set -euo pipefail

REPO="agustincbajo/Coral"
RELEASES_API="https://api.github.com/repos/${REPO}/releases/latest"
RELEASES_DL="https://github.com/${REPO}/releases/download"

# ----- parse args -----------------------------------------------------------

VERSION=""
INSTALL_DIR=""
WITH_CLAUDE_CONFIG="0"
SKIP_PLUGIN_INSTRUCTIONS="0"
while [ $# -gt 0 ]; do
  case "$1" in
    --version)
      VERSION="${2:-}"
      shift 2
      ;;
    --install-dir)
      INSTALL_DIR="${2:-}"
      shift 2
      ;;
    --with-claude-config)
      WITH_CLAUDE_CONFIG="1"
      shift
      ;;
    --skip-plugin-instructions)
      # FR-ONB-2: silent mode for CI runs. Also suppresses the
      # claude-paste.txt write so we leave no artifacts behind.
      SKIP_PLUGIN_INSTRUCTIONS="1"
      shift
      ;;
    -h|--help)
      sed -n '2,/^set -e/p' "$0" | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    *)
      echo "error: unknown argument: $1" >&2
      exit 2
      ;;
  esac
done

# ----- detect platform ------------------------------------------------------

uname_s=$(uname -s)
uname_m=$(uname -m)

# ----- BACKLOG #12 L4 (v0.40.1): refuse install from inside Claude Code -----
#
# macOS Sequoia stamps every file Claude Code subprocesses write with
# `com.apple.provenance`. The same xattr blocks read access from non-
# tracked processes (a regular Terminal) with EPERM, and the gate
# refuses xattr removal by the tracked app itself (anti-laundering) —
# even `sudo`/authtrampoline fails for paths inside `~/Documents/`.
# Net effect: installing from a Claude Code shell leaves the binary
# and `.coral/config.toml` accessible only to Claude Code, with no
# in-place repair short of a fresh checkout.
#
# Detection: `CLAUDECODE=1` is exported by Claude Code into every
# subprocess (terminal-in-editor, MCP host, hook spawn, etc.). On
# Linux there is no provenance xattr, so the gate doesn't bite —
# only block on Darwin. The escape hatch
# `CORAL_INSTALL_ALLOW_TRACKED_PROCESS=1` proceeds anyway for the
# rare CI scenario where the install artifact is consumed only by
# the same tracked process.
if [ "${uname_s}" = "Darwin" ] \
   && [ "${CLAUDECODE:-}" = "1" ] \
   && [ "${CORAL_INSTALL_ALLOW_TRACKED_PROCESS:-}" != "1" ]; then
  cat >&2 <<'CLAUDECODE_BLOCK'
error: refusing to install from a Claude Code shell on macOS.

  Why: macOS Sequoia stamps every file Claude Code subprocesses write
  with `com.apple.provenance`. That xattr then blocks your regular
  Terminal from reading those files (EPERM on open/stat), and the
  same anti-laundering gate prevents the tracked process from
  stripping the xattr — even `sudo`/authtrampoline fails for paths
  inside `~/Documents/`. The install would end up usable only from
  inside Claude Code, breaking `coral` invocations from your regular
  shell and any tooling outside Claude Code's process tree.

  Fix: re-run this installer from a regular Terminal (Terminal.app,
  iTerm2, kitty — anything NOT spawned by Claude Code). Once the
  binary is on PATH, you can use `coral` from anywhere, including
  inside Claude Code.

  Override (advanced — accepts EPERM in your regular shell):
    CORAL_INSTALL_ALLOW_TRACKED_PROCESS=1 bash install.sh ...

CLAUDECODE_BLOCK
  exit 1
fi

case "${uname_s}" in
  Linux)
    case "${uname_m}" in
      x86_64|amd64) target="x86_64-unknown-linux-gnu" ;;
      *)
        echo "error: unsupported Linux arch: ${uname_m}" >&2
        echo "       Coral currently ships x86_64-unknown-linux-gnu only." >&2
        echo "       Use 'cargo install --locked --git https://github.com/${REPO} coral-cli' instead." >&2
        exit 1
        ;;
    esac
    ;;
  Darwin)
    case "${uname_m}" in
      arm64|aarch64) target="aarch64-apple-darwin" ;;
      x86_64) target="x86_64-apple-darwin" ;;
      *)
        echo "error: unsupported macOS arch: ${uname_m}" >&2
        exit 1
        ;;
    esac
    ;;
  *)
    echo "error: unsupported OS: ${uname_s}" >&2
    echo "       Coral installer supports Linux and macOS." >&2
    echo "       For Windows, use scripts/install.ps1." >&2
    exit 1
    ;;
esac

# ----- resolve version ------------------------------------------------------

if [ -z "${VERSION}" ]; then
  if command -v curl >/dev/null 2>&1; then
    VERSION=$(curl -fsSL "${RELEASES_API}" | grep -m1 '"tag_name"' | sed -E 's/.*"tag_name": *"([^"]+)".*/\1/')
  elif command -v wget >/dev/null 2>&1; then
    VERSION=$(wget -qO- "${RELEASES_API}" | grep -m1 '"tag_name"' | sed -E 's/.*"tag_name": *"([^"]+)".*/\1/')
  else
    echo "error: need curl or wget to resolve the latest release tag." >&2
    exit 1
  fi
fi

if [ -z "${VERSION}" ]; then
  echo "error: could not resolve a release tag from GitHub." >&2
  echo "       Pass --version vX.Y.Z to skip the API lookup." >&2
  exit 1
fi

# ----- pick install dir -----------------------------------------------------

if [ -z "${INSTALL_DIR}" ]; then
  if [ -w /usr/local/bin ] 2>/dev/null; then
    INSTALL_DIR="/usr/local/bin"
  else
    INSTALL_DIR="${HOME}/.local/bin"
    mkdir -p "${INSTALL_DIR}"
  fi
fi

# ----- post-install plumbing (forward decls) --------------------------------
#
# `print_post_install_message` runs after the binary is on PATH AND the
# optional marketplace registration. Three modes — gated on SKIP, the
# `--with-claude-config` flag, and whether Claude Code is present on
# this host (FR-ONB-1).

claude_cli_present() {
  # FR-ONB-1: branch the next-steps message based on whether the user
  # already has Claude Code installed. `command -v` is portable; we
  # avoid spawning `claude --version` because that's slow and not
  # required (presence is enough).
  command -v claude >/dev/null 2>&1
}

write_claude_paste_file() {
  # `.coral/claude-paste.txt` makes the 3 lines copy-pasteable from a
  # text editor when the user's terminal isn't on-screen. Skipped under
  # `--skip-plugin-instructions` so CI doesn't leave stray files.
  local install_root
  install_root="$(pwd)/.coral"
  mkdir -p "${install_root}"
  cat > "${install_root}/claude-paste.txt" <<'PASTE'
/plugin marketplace add agustincbajo/Coral
/plugin install coral@coral
/reload-plugins
PASTE
}

print_post_install_message() {
  if [ "${SKIP_PLUGIN_INSTRUCTIONS}" = "1" ]; then
    return 0
  fi
  if ! claude_cli_present; then
    cat <<MISSING_CLAUDE

⚠ Claude Code not installed.
  Install: https://claude.ai/code → run this installer again,
  OR use \`coral doctor --wizard\` to set up a non-Claude-Code provider
  (Anthropic API key, Gemini, or local Ollama).

MISSING_CLAUDE
    return 0
  fi
  if [ "${WITH_CLAUDE_CONFIG}" = "1" ]; then
    # FR-ONB-4: --with-claude-config success message.
    cat <<READY

✅ Coral installed + marketplace registered.
   Open Claude Code in your repo and type anything to get started.
READY
    return 0
  fi
  # FR-ONB-4 default path: 3 paste lines + claude-paste.txt sidecar.
  write_claude_paste_file
  cat <<NEXT

📋 Next: paste these three lines into Claude Code (one at a time):

    /plugin marketplace add agustincbajo/Coral
    /plugin install coral@coral
    /reload-plugins

Then type anything in Claude Code — Coral's CLAUDE.md will guide it.

(Also saved to .coral/claude-paste.txt for copy-paste from your editor.)
NEXT
}

# ----- skip if already installed at this version ----------------------------

if [ -x "${INSTALL_DIR}/coral" ]; then
  installed_version=$("${INSTALL_DIR}/coral" --version 2>/dev/null | awk '{print $2}' || true)
  if [ "v${installed_version}" = "${VERSION}" ] || [ "${installed_version}" = "${VERSION}" ]; then
    echo "coral ${VERSION} already installed at ${INSTALL_DIR}/coral — nothing to do."
    # FR-ONB-26: still wire marketplace on re-runs when the user asked
    # for `--with-claude-config`. The subcommand is idempotent.
    if [ "${WITH_CLAUDE_CONFIG}" = "1" ]; then
      "${INSTALL_DIR}/coral" self-register-marketplace --scope=project \
        || echo "warn: marketplace registration failed; falling back to paste-3-lines flow" >&2
    fi
    print_post_install_message
    exit 0
  fi
fi

# ----- download -------------------------------------------------------------

base="coral-${VERSION}-${target}"
tarball="${base}.tar.gz"
sha_file="${tarball}.sha256"
url_tar="${RELEASES_DL}/${VERSION}/${tarball}"
url_sha="${RELEASES_DL}/${VERSION}/${sha_file}"

tmpdir=$(mktemp -d)
# Use a trap for cleanup so partial downloads don't accumulate.
trap 'rm -rf "${tmpdir}"' EXIT

echo "Downloading ${tarball} ..."
if command -v curl >/dev/null 2>&1; then
  curl -fL --proto '=https' --tlsv1.2 -o "${tmpdir}/${tarball}" "${url_tar}"
  curl -fL --proto '=https' --tlsv1.2 -o "${tmpdir}/${sha_file}" "${url_sha}"
else
  wget -O "${tmpdir}/${tarball}" "${url_tar}"
  wget -O "${tmpdir}/${sha_file}" "${url_sha}"
fi

# ----- verify SHA-256 -------------------------------------------------------

cd "${tmpdir}"
echo "Verifying SHA-256 ..."
if command -v shasum >/dev/null 2>&1; then
  shasum -a 256 -c "${sha_file}"
elif command -v sha256sum >/dev/null 2>&1; then
  sha256sum -c "${sha_file}"
else
  echo "error: neither shasum nor sha256sum found; refusing to install without verification." >&2
  exit 1
fi

# ----- extract + install ----------------------------------------------------

tar -xzf "${tarball}"

# Try to write to the install dir directly; fall back to sudo only if needed.
if [ -w "${INSTALL_DIR}" ]; then
  install -m 0755 "${base}/coral" "${INSTALL_DIR}/coral"
else
  echo "Note: ${INSTALL_DIR} is not writable by $(id -un); using sudo for the install step."
  sudo install -m 0755 "${base}/coral" "${INSTALL_DIR}/coral"
fi

# Remove macOS quarantine xattr so Gatekeeper doesn't block the first run.
if [ "${uname_s}" = "Darwin" ] && command -v xattr >/dev/null 2>&1; then
  xattr -d com.apple.quarantine "${INSTALL_DIR}/coral" 2>/dev/null || true
fi

# Restore the original cwd (we cd'd into ${tmpdir} for the shasum check)
# so subsequent steps that touch the repo's `.coral/` use the right
# directory. The EXIT trap still rm -rf's tmpdir.
cd - >/dev/null

# ----- post-install message ------------------------------------------------

echo
echo "Installed: ${INSTALL_DIR}/coral"
"${INSTALL_DIR}/coral" --version || true

# Warn if INSTALL_DIR is not on PATH (common with ~/.local/bin on fresh systems).
case ":${PATH}:" in
  *":${INSTALL_DIR}:"*) ;;
  *)
    echo
    echo "Warning: ${INSTALL_DIR} is not on your PATH."
    echo "Add this to your shell profile (.bashrc / .zshrc):"
    echo "    export PATH=\"${INSTALL_DIR}:\$PATH\""
    ;;
esac

# FR-ONB-31: WSL2 detection. When running under WSL2 the user is on a
# Windows host but installed the Linux binary — if they invoke Claude
# Code on the Windows side (not in WSL), the Linux `coral` won't be
# reachable. We warn but don't abort because Coral-in-WSL is also a
# legitimate setup.
if [ -r /proc/version ] && grep -qi microsoft /proc/version 2>/dev/null; then
  cat >&2 <<'WSL'

⚠ Detected WSL2. Coral binary installed for Linux.
  If you use Claude Code on Windows host (not in WSL),
  install the Windows binary instead via install.ps1.
WSL
fi

# FR-ONB-26: delegate marketplace registration to the binary (no jq
# required cross-platform). The subcommand is idempotent + atomic; a
# failure here is non-fatal because the paste-3-lines flow is the
# fallback experience the user can still complete manually.
if [ "${WITH_CLAUDE_CONFIG}" = "1" ]; then
  "${INSTALL_DIR}/coral" self-register-marketplace --scope=project \
    || echo "warn: marketplace registration failed; falling back to paste-3-lines flow" >&2
fi

print_post_install_message
