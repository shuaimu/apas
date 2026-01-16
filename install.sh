#!/bin/bash
set -e

# APAS Installer
# Usage: curl -sSL https://raw.githubusercontent.com/shuaimu/apas/master/install.sh | bash

REPO="shuaimu/apas"
BINARY_NAME="apas"
INSTALL_DIR="${APAS_INSTALL_DIR:-$HOME/.local/bin}"

# Detect OS and architecture
detect_platform() {
    local os arch

    os="$(uname -s)"
    arch="$(uname -m)"

    case "$os" in
        Linux)  os="linux" ;;
        Darwin) os="darwin" ;;
        *)      echo "Unsupported OS: $os"; exit 1 ;;
    esac

    case "$arch" in
        x86_64|amd64)  arch="x86_64" ;;
        aarch64|arm64) arch="aarch64" ;;
        *)             echo "Unsupported architecture: $arch"; exit 1 ;;
    esac

    echo "${os}-${arch}"
}

# Get latest release version from GitHub
get_latest_version() {
    curl -sSL "https://api.github.com/repos/${REPO}/releases/latest" | \
        grep '"tag_name":' | \
        sed -E 's/.*"([^"]+)".*/\1/'
}

# Download and install binary
install_binary() {
    local platform="$1"
    local version="$2"
    local download_url="https://github.com/${REPO}/releases/download/${version}/${BINARY_NAME}-${platform}"
    local tmp_file=$(mktemp)

    echo "Downloading ${BINARY_NAME} ${version} for ${platform}..."

    if ! curl -sSL -f -o "$tmp_file" "$download_url"; then
        echo "Error: Failed to download from $download_url"
        echo ""
        echo "Pre-built binaries may not be available yet."
        echo "You can build from source instead:"
        echo "  git clone https://github.com/${REPO}.git"
        echo "  cd apas"
        echo "  cargo build --release -p apas"
        echo "  cp target/release/apas ~/.local/bin/"
        rm -f "$tmp_file"
        exit 1
    fi

    # Create install directory if needed
    mkdir -p "$INSTALL_DIR"

    # Install binary
    chmod +x "$tmp_file"
    mv "$tmp_file" "${INSTALL_DIR}/${BINARY_NAME}"

    echo "Installed ${BINARY_NAME} to ${INSTALL_DIR}/${BINARY_NAME}"
}

# Check if install dir is in PATH
check_path() {
    if [[ ":$PATH:" != *":${INSTALL_DIR}:"* ]]; then
        echo ""
        echo "WARNING: ${INSTALL_DIR} is not in your PATH"
        echo "Add it to your shell profile:"
        echo ""
        echo "  # For bash (~/.bashrc):"
        echo "  export PATH=\"\$PATH:${INSTALL_DIR}\""
        echo ""
        echo "  # For zsh (~/.zshrc):"
        echo "  export PATH=\"\$PATH:${INSTALL_DIR}\""
    fi
}

main() {
    echo "APAS Installer"
    echo "=============="
    echo ""

    local platform=$(detect_platform)
    echo "Detected platform: ${platform}"

    local version=$(get_latest_version)
    if [ -z "$version" ]; then
        echo "Error: Could not determine latest version"
        echo "Installing from source instead..."
        echo ""
        echo "Run these commands:"
        echo "  git clone https://github.com/${REPO}.git"
        echo "  cd apas"
        echo "  cargo build --release -p apas"
        echo "  cp target/release/apas ~/.local/bin/"
        exit 1
    fi

    echo "Latest version: ${version}"
    echo ""

    install_binary "$platform" "$version"
    check_path

    echo ""
    echo "Installation complete!"
    echo "Run 'apas --help' to get started."
}

main "$@"
