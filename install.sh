#!/bin/bash
set -e

# APAS Installer - Build from source
# Usage: curl -sSL https://raw.githubusercontent.com/shuaimu/apas/master/install.sh | bash

REPO_URL="https://github.com/shuaimu/apas.git"
INSTALL_DIR="${APAS_INSTALL_DIR:-$HOME/.local/bin}"
BUILD_DIR="${TMPDIR:-/tmp}/apas-build-$$"

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
cargo build --release -p apas

echo "Installing to $INSTALL_DIR..."
mkdir -p "$INSTALL_DIR"
cp target/release/apas "$INSTALL_DIR/"

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
