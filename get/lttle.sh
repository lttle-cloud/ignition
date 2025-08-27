#!/usr/bin/env bash
# Installs lttle 0.1.0-rc5.
# Repo: lttle-cloud/ignition
# Assets (plain binaries):
#   - macOS arm64: lttle_darwin_aarch64
#   - Linux x86_64: lttle_linux_x86_64
# Installs as: /usr/local/bin/lttle
set -euo pipefail

VERSION="0.1.0-rc5"
DARWIN_URL="https://github.com/lttle-cloud/ignition/releases/download/v0.1.0-rc5/lttle_darwin_aarch64"
LINUX_URL="https://github.com/lttle-cloud/ignition/releases/download/v0.1.0-rc5/lttle_linux_x86_64"

# RUNTIME OVERRIDES
INSTALL_DIR="${INSTALL_DIR:-/usr/local/bin}"
BINARY_NAME_INSTALL="lttle"
USE_SUDO="${USE_SUDO:-}"

say()  { printf "%b\n" "$*"; }
fail() { say "❌ $*"; exit 1; }
need() { command -v "$1" >/dev/null 2>&1 || fail "Missing dependency: $1"; }

install_file() {
  local src="$1" dst="$INSTALL_DIR/$BINARY_NAME_INSTALL"
  mkdir -p "$INSTALL_DIR"
  if [[ -w "$INSTALL_DIR" && -z "$USE_SUDO" ]]; then
    install -m 0755 "$src" "$dst"
  else
    need sudo
    sudo install -m 0755 "$src" "$dst"
  fi
  say "✅ Installed: $dst"
}

# ── Detect supported platform ────────────────────────────────────────────────
OS="$(uname -s)"
ARCH="$(uname -m)"
case "$OS:$ARCH" in
  Darwin:arm64)             TARGET_URL="$DARWIN_URL";;
  Linux:x86_64|Linux:amd64) TARGET_URL="$LINUX_URL";;
  *) fail "Unsupported platform. Supported: macOS arm64, Linux x86_64.";;
esac

need curl

# ── Download → chmod → install ──────────────────────────────────────────────
WORKDIR="$(mktemp -d)"; trap 'rm -rf "$WORKDIR"' EXIT
FILE="$WORKDIR/lttle.tmp"

say "• Downloading lttle v0.1.0-rc5 from: $TARGET_URL"
curl -fL --retry 3 -o "$FILE" "$TARGET_URL"

chmod +x "$FILE" || true

say "• Installing to $INSTALL_DIR as '$BINARY_NAME_INSTALL'…"
install_file "$FILE"

# Show version if available
if "$INSTALL_DIR/$BINARY_NAME_INSTALL" --version >/dev/null 2>&1; then
  "$INSTALL_DIR/$BINARY_NAME_INSTALL" --version || true
fi

say "🎉 Done."
