#!/bin/bash
# Install Google's checksum-verified release payload for this platform without replacing an existing CLI.
set -euo pipefail
install_dir="$HOME/.local/bin"
if [[ ${1:-} == --dir ]]; then install_dir=${2:?missing install directory}; shift 2; fi
launch=false
if [[ ${1:-} == --exec ]]; then
    launch=true
    shift
elif (( $# )); then
    echo "Usage: install-antigravity.sh [--dir DIRECTORY]" >&2
    exit 2
fi
target="$install_dir/agy"
if [[ ! -x "$target" || ! -f "$target" ]]; then
    if [[ -e "$target" || -L "$target" ]]; then
        echo "Cannot install Antigravity: $target already exists and is not executable." >&2
        exit 1
    fi
    os=$(uname -s)
    arch=$(uname -m)
    platform="$os-$arch"
    if [[ "$os" == Linux ]] && { [[ -e /etc/alpine-release ]] || ldd /bin/ls 2>&1 | grep -qi musl; }; then
        platform="$platform-musl"
    fi
    # SHA-512 digests from Google's platform manifests for release 1.5.16.
    case "$platform" in
        Linux-x86_64)
            asset=linux-x64/cli_linux_x64.tar.gz
            digest=3dee9f26e298156db673f6ce2a0f5dfed5b30006e04ceb9809c68070e7b717587b33a3da0f8bc2eda45af81a50bc8b14c82b28ae5832bcfe3e7fd49397117b31 ;;
        Linux-x86_64-musl)
            asset=linux-x64-musl/cli_linux_x64_musl.tar.gz
            digest=9f868c13f308557192b8c3fa30f1251283167b7e551e9f0f05ed0fa2640e91e02accaeacd41692a0422e8de90c1cc0b6e327f2b6befbd11080a4881d45bf61b1 ;;
        Linux-aarch64)
            asset=linux-arm/cli_linux_arm64.tar.gz
            digest=23c193151499a4986fd5fa0d8132665cc8ffbad6cc0b3ff4fd53a2cb68abb76b149a992b836d53063443cd89add335f14d8134fb02abdcecbf6e1eaef6c53d33 ;;
        Linux-aarch64-musl)
            asset=linux-arm-musl/cli_linux_arm64_musl.tar.gz
            digest=4bc5c802c97ca19404c666136f1e74dc880a3b0346e7c6950ad4e217056c6cc86ae5afa633aa5989a260600f49cb6e71db438558c4f2d44e5f5bdab8bb38980c ;;
        Darwin-x86_64)
            asset=darwin-x64/cli_mac_x64.tar.gz
            digest=b3eae073ee633032e9d473cdc7725c30bdfe2e09436bc36cfb3422e7e5fa3733de9591a7f78d02362294e7bf87e345d70e5d839689e526129a6f3fc041857a05 ;;
        Darwin-arm64)
            asset=darwin-arm/cli_mac_arm64.tar.gz
            digest=33f5617dc74fea606d47b2483c6d271c850c9f3c9c55b724c664c3c0d4b82100d819fbcf55714f851765f6628d4296883ac8939b4fdeb55d5e6b9ce923135d18 ;;
        *) echo "No Antigravity release for $platform" >&2; exit 1 ;;
    esac
    mkdir -p "$install_dir"
    stage=$(mktemp -d "$install_dir/.apas-agy.XXXXXXXX")
    trap 'rm -rf -- "$stage"' EXIT
    echo "Installing Antigravity CLI 1.5.16 for $platform..."
    curl --fail --silent --show-error --location --proto '=https' --proto-redir '=https' \
        --connect-timeout 15 --max-time 600 --retry 2 \
        "https://storage.googleapis.com/antigravity-public/antigravity-cli/1.5.16-5250855658717184/$asset" -o "$stage/release.tgz"
    if [[ "$os" == Darwin ]]; then
        actual=$(shasum -a 512 "$stage/release.tgz")
    else
        actual=$(sha512sum "$stage/release.tgz")
    fi
    if [[ ${actual%% *} != "$digest" ]]; then echo "Antigravity download failed checksum verification." >&2; exit 1; fi
    tar -xzOf "$stage/release.tgz" antigravity > "$stage/agy"
    chmod 755 "$stage/agy"
    "$stage/agy" --version
    # Publish the complete executable once, including when two tabs install together.
    ln "$stage/agy" "$target" || { [[ -f "$target" && -x "$target" ]] || exit 1; }
    rm -rf -- "$stage"
    trap - EXIT
fi
if "$launch"; then exec "$target" "$@"; fi
echo "Antigravity CLI is installed at $target"
