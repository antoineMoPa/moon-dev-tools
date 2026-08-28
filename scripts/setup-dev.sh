#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ZIG_MAJOR_MINOR="0.15"
RUST_TOOLCHAIN="${MOONREVIEW_RUST_TOOLCHAIN:-stable}"

require_command() {
    command_name="$1"
    install_hint="$2"

    if ! command -v "$command_name" >/dev/null 2>&1; then
        echo "missing $command_name" >&2
        echo "$install_hint" >&2
        exit 1
    fi
}

install_zig() {
    case "$(uname -s)" in
        Darwin)
            require_command brew "Install Homebrew from https://brew.sh/"
            brew install "zig@$ZIG_MAJOR_MINOR"
            zig_prefix="$(brew --prefix "zig@$ZIG_MAJOR_MINOR")"
            ;;
        *)
            cat >&2 <<EOF
Install zig $ZIG_MAJOR_MINOR.x and put it first on PATH.

Downloads: https://ziglang.org/download/
EOF
            exit 1
            ;;
    esac

    zig_bin="$zig_prefix/bin/zig"
    if [ ! -x "$zig_bin" ]; then
        echo "zig $ZIG_MAJOR_MINOR.x was not found at $zig_bin" >&2
        exit 1
    fi

    zig_version="$($zig_bin version)"
    case "$zig_version" in
        "$ZIG_MAJOR_MINOR".*) ;;
        *)
            echo "zig $zig_version found at $zig_bin, but Ghostty needs $ZIG_MAJOR_MINOR.x" >&2
            exit 1
            ;;
    esac
}

require_command git "Install git from https://git-scm.com/downloads"
require_command rustup "Install rustup from https://rustup.rs/"

echo "Updating Rust toolchain ($RUST_TOOLCHAIN)..."
rustup update "$RUST_TOOLCHAIN"

echo "Initializing git submodules..."
git -C "$ROOT_DIR" submodule update --init --recursive

echo "Installing Zig $ZIG_MAJOR_MINOR.x..."
install_zig

cat <<EOF

Development dependencies are ready.

For cargo builds in this shell, run:
  export PATH="$zig_prefix/bin:\$PATH"
EOF
