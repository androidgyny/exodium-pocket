#!/usr/bin/env bash
# get-emulators.sh — Download the Win9x emulators (DOSBox-X, 86Box) for the
# current platform into src-tauri/resources/.
#
# eXoWin9x games boot Windows 95/98: DOSBox-X runs the x98-variant games
# (Staging cannot boot Win9x guests), 86Box runs the 86box-variant handful.
# On Windows the x98 games prefer eXo's own DOSBox-X build extracted from the
# torrent's EXTWin9x.zip at runtime - the build downloaded here is the
# fallback. DOSBox-X publishes NO Linux binaries (Flatpak/distro only), so
# Linux resolves it from PATH/Flatpak at runtime and this script skips it.
#
# Usage:
#   pnpm run get-emulators                 # download for current platform
#   pnpm run get-emulators -- --force      # re-download
#   DOSBOX_X_VERSION=2025.02.01 E86BOX_VERSION=6.0 pnpm run get-emulators
set -euo pipefail

FORCE=0
for arg in "$@"; do
  [[ "$arg" == "--force" ]] && FORCE=1
done

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
RES_DIR="$REPO_ROOT/src-tauri/resources"

# Pinned near eXo's own x98 build (exes dated 2025-02-01) - a drifting
# DOSBox-X may change conf-key behavior under the pack's play.confs.
DOSBOX_X_VERSION="${DOSBOX_X_VERSION:-2025.02.01}"
E86BOX_VERSION="${E86BOX_VERSION:-6.0}"

OS="$(uname -s)"
ARCH="$(uname -m)"

# All four resource dirs must exist with at least a placeholder on every
# platform - tauri.conf.json lists them in bundle.resources and the build
# script hard-errors on a missing path.
for d in dosbox-x dosbox-x-bin 86box 86box-bin; do
  if [[ ! -d "$RES_DIR/$d" ]]; then
    mkdir -p "$RES_DIR/$d"
    touch "$RES_DIR/$d/.placeholder"
  fi
done

STAMP="$RES_DIR/.win9x-emulators-version"
WANT_STAMP="dosbox-x=$DOSBOX_X_VERSION 86box=$E86BOX_VERSION os=$OS"
if [[ "$FORCE" -eq 0 && "$(cat "$STAMP" 2>/dev/null || true)" == "$WANT_STAMP" ]]; then
  echo "Win9x emulators already present ($WANT_STAMP), skipping."
  exit 0
fi

TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

fetch() { # fetch <url> <out>
  echo "Downloading $(basename "$2")..."
  curl -fL --progress-bar -o "$2" "$1"
}

# ── DOSBox-X ─────────────────────────────────────────────────────────────────

# Release assets embed a build timestamp (dosbox-x-macosx-arm64-20250201150724.zip),
# so resolve them from the release's asset list instead of hardcoding.
DBX_API="https://api.github.com/repos/joncampbell123/dosbox-x/releases/tags/dosbox-x-v${DOSBOX_X_VERSION}"
DBX_URLS="$(curl -fsSL "$DBX_API" | grep -o 'https://[^"]*download/[^"]*' || true)"

dbx_url() { # dbx_url <grep-pattern>
  echo "$DBX_URLS" | grep -E "$1" | head -1
}

case "$OS" in
  Darwin)
    case "$ARCH" in
      arm64)  DBX_URL="$(dbx_url 'macosx-arm64-[^"]*\.zip')" ;;
      x86_64) DBX_URL="$(dbx_url 'macosx-x86_64-[^"]*\.zip')" ;;
      *) echo "Unsupported macOS arch: $ARCH"; exit 1 ;;
    esac
    [[ -z "$DBX_URL" ]] && { echo "ERROR: no DOSBox-X macOS asset for v${DOSBOX_X_VERSION}"; exit 1; }
    fetch "$DBX_URL" "$TMP_DIR/dosbox-x-mac.zip"
    unzip -q "$TMP_DIR/dosbox-x-mac.zip" -d "$TMP_DIR/dbx"
    APP_SRC="$(find "$TMP_DIR/dbx" -type d -name "dosbox-x.app" | head -1)"
    if [[ -z "$APP_SRC" ]]; then
      echo "ERROR: dosbox-x.app not found in $DBX_ARCHIVE"; exit 1
    fi
    rm -rf "$RES_DIR/dosbox-x/dosbox-x.app"
    mkdir -p "$RES_DIR/dosbox-x"
    cp -R "$APP_SRC" "$RES_DIR/dosbox-x/dosbox-x.app"
    rm -f "$RES_DIR/dosbox-x/.placeholder"
    xattr -cr "$RES_DIR/dosbox-x/dosbox-x.app" 2>/dev/null || true
    codesign --force --deep --sign - "$RES_DIR/dosbox-x/dosbox-x.app"
    echo "Installed: $RES_DIR/dosbox-x/dosbox-x.app"
    ;;
  Linux)
    echo "DOSBox-X: no official Linux binaries - resolved from PATH/Flatpak at runtime."
    ;;
  MINGW*|MSYS*|CYGWIN*)
    # Naming drifted across releases: -mingw-win64- (2025) vs -mingw64-…-portable (2026).
    DBX_URL="$(dbx_url 'mingw-win64-[^"]*\.zip|mingw64-[^"]*\.zip')"
    [[ -z "$DBX_URL" ]] && { echo "ERROR: no DOSBox-X Windows asset for v${DOSBOX_X_VERSION}"; exit 1; }
    fetch "$DBX_URL" "$TMP_DIR/dosbox-x-win.zip"
    unzip -q "$TMP_DIR/dosbox-x-win.zip" -d "$TMP_DIR/dbx"
    EXE_SRC="$(find "$TMP_DIR/dbx" -type f -name "dosbox-x.exe" | head -1)"
    if [[ -z "$EXE_SRC" ]]; then
      echo "ERROR: dosbox-x.exe not found in the DOSBox-X Windows archive"; exit 1
    fi
    rm -rf "$RES_DIR/dosbox-x-bin"
    mkdir -p "$RES_DIR/dosbox-x-bin"
    cp -r "$(dirname "$EXE_SRC")"/. "$RES_DIR/dosbox-x-bin/"
    echo "Installed: $RES_DIR/dosbox-x-bin/dosbox-x.exe"
    ;;
esac

# ── 86Box ────────────────────────────────────────────────────────────────────

# Asset names embed a build number (e.g. 86Box-Linux-x86_64-b9001.AppImage),
# so resolve them from the release's asset list instead of hardcoding.
E86_API="https://api.github.com/repos/86Box/86Box/releases/tags/v${E86BOX_VERSION}"
E86_URLS="$(curl -fsSL "$E86_API" | grep -o 'https://[^"]*download/[^"]*' || true)"
if [[ -z "$E86_URLS" ]]; then
  echo "ERROR: could not list 86Box v${E86BOX_VERSION} release assets"; exit 1
fi

pick_url() { # pick_url <grep-pattern>
  echo "$E86_URLS" | grep -E "$1" | head -1
}

case "$OS" in
  Darwin)
    URL="$(pick_url '86Box-macOS-[^"]*\.zip')"
    [[ -z "$URL" ]] && { echo "ERROR: no 86Box macOS asset"; exit 1; }
    fetch "$URL" "$TMP_DIR/86box-mac.zip"
    unzip -q "$TMP_DIR/86box-mac.zip" -d "$TMP_DIR/e86"
    APP_SRC="$(find "$TMP_DIR/e86" -type d -name "86Box.app" | head -1)"
    [[ -z "$APP_SRC" ]] && { echo "ERROR: 86Box.app not found"; exit 1; }
    rm -rf "$RES_DIR/86box/86Box.app"
    mkdir -p "$RES_DIR/86box"
    cp -R "$APP_SRC" "$RES_DIR/86box/86Box.app"
    rm -f "$RES_DIR/86box/.placeholder"
    xattr -cr "$RES_DIR/86box/86Box.app" 2>/dev/null || true
    codesign --force --deep --sign - "$RES_DIR/86box/86Box.app"
    echo "Installed: $RES_DIR/86box/86Box.app"
    ;;
  Linux)
    case "$ARCH" in
      x86_64)  URL="$(pick_url '86Box-Linux-x86_64[^"]*\.AppImage')" ;;
      aarch64) URL="$(pick_url '86Box[^"]*Linux-arm64[^"]*\.AppImage')" ;;
      *) echo "Unsupported Linux arch: $ARCH"; exit 1 ;;
    esac
    [[ -z "$URL" ]] && { echo "ERROR: no 86Box Linux asset"; exit 1; }
    mkdir -p "$RES_DIR/86box"
    fetch "$URL" "$RES_DIR/86box/86Box.AppImage"
    chmod +x "$RES_DIR/86box/86Box.AppImage"
    rm -f "$RES_DIR/86box/.placeholder"
    echo "Installed: $RES_DIR/86box/86Box.AppImage"
    ;;
  MINGW*|MSYS*|CYGWIN*)
    URL="$(pick_url '86Box-Windows-64[^"]*\.zip')"
    [[ -z "$URL" ]] && { echo "ERROR: no 86Box Windows asset"; exit 1; }
    fetch "$URL" "$TMP_DIR/86box-win.zip"
    rm -rf "$RES_DIR/86box-bin"
    mkdir -p "$RES_DIR/86box-bin"
    unzip -q "$TMP_DIR/86box-win.zip" -d "$RES_DIR/86box-bin"
    # Normalize: the exe must sit at 86box-bin/86Box.exe for resolve_86box.
    EXE_SRC="$(find "$RES_DIR/86box-bin" -type f -iname "86Box.exe" | head -1)"
    if [[ -n "$EXE_SRC" && "$EXE_SRC" != "$RES_DIR/86box-bin/86Box.exe" ]]; then
      mv "$EXE_SRC" "$RES_DIR/86box-bin/86Box.exe"
    fi
    echo "Installed: $RES_DIR/86box-bin/86Box.exe"
    ;;
esac

echo "$WANT_STAMP" > "$STAMP"
echo "Win9x emulators ready."
