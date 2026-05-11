#!/usr/bin/env bash
# Coral one-line installer for Linux + macOS.
#
# Downloads the latest release tarball matching this host's platform/arch,
# verifies the SHA-256, places `coral` on PATH, removes the macOS
# quarantine xattr, and prints the two lines a user should paste into
# Claude Code to install the plugin.
#
# Idempotent: running twice over the same release is a no-op.
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/agustincbajo/Coral/main/scripts/install.sh | bash
#   curl -fsSL https://raw.githubusercontent.com/agustincbajo/Coral/main/scripts/install.sh | bash -s -- --version v0.30.0
#
set -euo pipefail

REPO="agustincbajo/Coral"
RELEASES_API="https://api.github.com/repos/${REPO}/releases/latest"
RELEASES_DL="https://github.com/${REPO}/releases/download"

# ----- parse args -----------------------------------------------------------

VERSION=""
INSTALL_DIR=""
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

# ----- skip if already installed at this version ----------------------------

if [ -x "${INSTALL_DIR}/coral" ]; then
  installed_version=$("${INSTALL_DIR}/coral" --version 2>/dev/null | awk '{print $2}' || true)
  if [ "v${installed_version}" = "${VERSION}" ] || [ "${installed_version}" = "${VERSION}" ]; then
    echo "coral ${VERSION} already installed at ${INSTALL_DIR}/coral — nothing to do."
    cat <<'NEXT'

Now, inside Claude Code, paste:

  /plugin marketplace add agustincbajo/Coral
  /plugin install coral@coral

Then ask Claude: "set up Coral for this repo".
NEXT
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

cat <<'NEXT'

Now, inside Claude Code, paste:

  /plugin marketplace add agustincbajo/Coral
  /plugin install coral@coral

Then ask Claude: "set up Coral for this repo" — the plugin takes over.
NEXT
