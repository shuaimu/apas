#!/bin/bash
set -e

# APAS Installer - Build from source
# Usage: curl -sSL https://raw.githubusercontent.com/shuaimu/apas/master/install.sh | bash

REPO_URL="https://github.com/shuaimu/apas.git"
INSTALL_DIR="${APAS_INSTALL_DIR:-$HOME/.local/bin}"
BUILD_DIR="${TMPDIR:-/tmp}/apas-build-$$"

# Prefer a static musl build on Linux so the installed CLI doesn't depend on
# the system glibc version (self-updates rebuild the same way). Prints the
# musl target triple on stdout when the toolchain is ready, otherwise nothing
# (caller then builds against the system libc).
setup_musl_target() {
    [ "$(uname -s)" = "Linux" ] || return 1
    local arch target
    arch="$(uname -m)"
    case "$arch" in
        x86_64|aarch64) target="${arch}-unknown-linux-musl" ;;
        *) return 1 ;;
    esac
    command -v rustup >/dev/null 2>&1 || return 1
    rustup target add "$target" >/dev/null 2>&1 || return 1
    # ring (rustls' crypto backend) compiles some C for the musl target and
    # needs a musl C compiler: musl-gcc, from the `musl-tools` package.
    if ! command -v musl-gcc >/dev/null 2>&1; then
        if command -v apt-get >/dev/null 2>&1; then
            if [ "$(id -u)" = 0 ]; then
                apt-get update -qq && apt-get install -y musl-tools >/dev/null 2>&1 || true
            elif command -v sudo >/dev/null 2>&1; then
                sudo apt-get update -qq && sudo apt-get install -y musl-tools >/dev/null 2>&1 || true
            fi
        fi
    fi
    command -v musl-gcc >/dev/null 2>&1 || return 1
    printf '%s' "$target"
}

echo "APAS Installer"
echo "=============="
echo ""

# Check for Rust
if ! command -v cargo &> /dev/null; then
    echo "Rust is not installed. Installing via rustup..."
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
    source "$HOME/.cargo/env"
fi

echo "Cloning repository..."
git clone --depth 1 "$REPO_URL" "$BUILD_DIR"
cd "$BUILD_DIR"

echo "Building apas..."
TARGET="$(setup_musl_target || true)"
if [ -n "$TARGET" ]; then
    echo "  static build target: $TARGET"
    cargo build --release --target "$TARGET" -p apas
    BIN="target/$TARGET/release/apas"
else
    echo "  musl toolchain unavailable — building against the system libc"
    cargo build --release -p apas
    BIN="target/release/apas"
fi

echo "Installing to $INSTALL_DIR..."
mkdir -p "$INSTALL_DIR"
cp "$BIN" "$INSTALL_DIR/"

echo "Cleaning up..."
cd /
rm -rf "$BUILD_DIR"

# Check if install dir is in PATH
if [[ ":$PATH:" != *":${INSTALL_DIR}:"* ]]; then
    echo ""
    echo "Add $INSTALL_DIR to your PATH:"
    echo "  export PATH=\"\$PATH:$INSTALL_DIR\""
fi

echo ""
echo "Installation complete! Run 'apas --help' to get started."
