#!/usr/bin/env bash
# Type-check the CLI for macOS on a Linux host: `cargo check` for
# aarch64-apple-darwin. Nothing is linked or run, so it finds compile errors
# in macOS-only code and nothing else.
#
# Needs: `rustup target add aarch64-apple-darwin`, plus clang and llvm-ar on
# PATH. ring's build script compiles C for the target and wants Apple SDK
# headers; stub-include/ holds four stand-ins that are enough to produce
# objects cargo check never links. They are not an SDK: do not build with them.
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo="$(cd "$here/../.." && pwd)"
tmp="/dev/shm/apas-macos-check-${USER:-user}"
mkdir -p "$tmp"
trap 'rm -rf "$tmp"' EXIT
cd "$repo"
CFLAGS_aarch64_apple_darwin="-ffreestanding -nostdlibinc -I$here/stub-include" \
CC_aarch64_apple_darwin="$(command -v clang)" \
AR_aarch64_apple_darwin="$(command -v llvm-ar)" \
TMPDIR="$tmp" \
CARGO_TARGET_DIR="$repo/target/mac-check" \
cargo check -p apas --all-targets --target aarch64-apple-darwin "$@"
