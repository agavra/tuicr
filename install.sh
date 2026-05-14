#!/bin/sh
#
# tuicr installer
#
# Usage:
#   curl -fsSL tuicr.dev/install.sh | sh
#
# Environment variables:
#   TUICR_VERSION       Version to install (default: latest)
#   TUICR_INSTALL_DIR   Where to install the binary (default: $HOME/.local/bin)
#   TUICR_INSTALL_YES   Skip the confirmation prompt (default: prompt if a tty is attached)

set -e

REPO="agavra/tuicr"
BIN="tuicr"
INSTALL_DIR="${TUICR_INSTALL_DIR:-$HOME/.local/bin}"

err() {
    printf 'Error: %s\n' "$1" >&2
    exit 1
}

info() {
    printf '%s\n' "$1"
}

fetch() {
    if command -v curl >/dev/null 2>&1; then
        curl -fsSL "$1"
    elif command -v wget >/dev/null 2>&1; then
        wget -qO- "$1"
    else
        err "neither curl nor wget found"
    fi
}

fetch_to() {
    if command -v curl >/dev/null 2>&1; then
        curl -fsSL -o "$2" "$1"
    elif command -v wget >/dev/null 2>&1; then
        wget -qO "$2" "$1"
    else
        err "neither curl nor wget found"
    fi
}

# Detect OS
OS=$(uname -s)
case "$OS" in
    Linux)
        TARGET_OS="unknown-linux-gnu"
        EXT="tar.gz"
        ;;
    Darwin)
        TARGET_OS="apple-darwin"
        EXT="tar.gz"
        ;;
    MINGW*|MSYS*|CYGWIN*)
        TARGET_OS="pc-windows-msvc"
        EXT="zip"
        ;;
    *)
        err "unsupported operating system: $OS"
        ;;
esac

# Detect arch
ARCH=$(uname -m)
case "$ARCH" in
    x86_64|amd64)
        TARGET_ARCH="x86_64"
        ;;
    aarch64|arm64)
        TARGET_ARCH="aarch64"
        ;;
    *)
        err "unsupported architecture: $ARCH"
        ;;
esac

# Windows only ships x86_64
if [ "$TARGET_OS" = "pc-windows-msvc" ] && [ "$TARGET_ARCH" != "x86_64" ]; then
    err "unsupported Windows architecture: $ARCH (only x86_64 is published)"
fi

TARGET="${TARGET_ARCH}-${TARGET_OS}"

# Resolve version
VERSION="${TUICR_VERSION:-}"
if [ -z "$VERSION" ]; then
    info "Resolving latest release..."
    RELEASES_URL="https://api.github.com/repos/${REPO}/releases/latest"
    TAG=$(fetch "$RELEASES_URL" | grep -o '"tag_name": *"[^"]*"' | head -1 | sed 's/.*"\(v[^"]*\)".*/\1/')
    [ -n "$TAG" ] || err "could not resolve latest release from $RELEASES_URL"
    VERSION="${TAG#v}"
fi
# Normalize: strip leading v if the user passed v0.13.0
VERSION="${VERSION#v}"

ARCHIVE="${BIN}-${VERSION}-${TARGET}.${EXT}"
URL="https://github.com/${REPO}/releases/download/v${VERSION}/${ARCHIVE}"

# Show install plan and confirm
info ""
info "About to install:"
info "  Package:  $BIN $VERSION"
info "  Target:   $TARGET"
info "  Source:   $URL"
info "  Dest:     $INSTALL_DIR/$BIN"
info ""

if [ -z "$TUICR_INSTALL_YES" ] && [ -r /dev/tty ]; then
    printf "Continue? [Y/n] "
    read -r answer </dev/tty
    case "$answer" in
        ""|y|Y|yes|YES|Yes) ;;
        *) err "aborted by user" ;;
    esac
fi

info "Downloading ${ARCHIVE}..."

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

fetch_to "$URL" "$TMP/$ARCHIVE"

info "Extracting..."
case "$EXT" in
    tar.gz)
        tar -xzf "$TMP/$ARCHIVE" -C "$TMP"
        ;;
    zip)
        command -v unzip >/dev/null 2>&1 || err "unzip not found"
        unzip -q "$TMP/$ARCHIVE" -d "$TMP"
        ;;
esac

# Find the binary (handle .exe on Windows)
SRC=$(find "$TMP" -type f \( -name "$BIN" -o -name "${BIN}.exe" \) | head -1)
[ -n "$SRC" ] || err "could not find $BIN binary in extracted archive"

mkdir -p "$INSTALL_DIR"
DEST="$INSTALL_DIR/$(basename "$SRC")"
mv "$SRC" "$DEST"
chmod +x "$DEST"

info ""
info "Installed $BIN $VERSION to $DEST"

# Check PATH
case ":$PATH:" in
    *":$INSTALL_DIR:"*) ;;
    *)
        info ""
        info "Note: $INSTALL_DIR is not on your PATH."
        info "Add this to your shell profile (.bashrc, .zshrc, etc):"
        info ""
        info "    export PATH=\"$INSTALL_DIR:\$PATH\""
        ;;
esac

info ""
info "Run \`$BIN\` in any git, jj, or mercurial repo to start a review."
