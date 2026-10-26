#!/usr/bin/env bash
# build-release.sh - builds `moon` for the machine it runs on and packages it as that target's
# release archive. scripts/bin-release.sh runs it on the publishing machine, and
# .github/workflows/release.yml on a runner for each other target.

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT_DIR"

if (($# != 1)); then
    echo "usage: scripts/_internal/build-release.sh <target triple of this machine>" >&2
    exit 1
fi
TARGET_TRIPLE="$1"

TAG="v$(bash "$ROOT_DIR/scripts/_internal/version.sh")"
OUTPUT_DIR="$ROOT_DIR/target/release-artifacts/$TAG"
# The executable Cargo builds. install.sh expects it in the archive.
PROGRAM="moon"
ZIG_MAJOR_MINOR="0.15"

if [ "$TARGET_TRIPLE" != "$(bash "$ROOT_DIR/scripts/_internal/host-target.sh")" ]; then
    echo "$TARGET_TRIPLE is built on its own machine, and this one is $(bash "$ROOT_DIR/scripts/_internal/host-target.sh")" >&2
    exit 1
fi

# The native window's terminal comes from Ghostty's Zig source, so a matching Zig has to be
# on PATH before anything is built. Homebrew keeps 0.15 keg-only, so look there too.
require_zig() {
    if command -v brew >/dev/null 2>&1; then
        zig_prefix="$(brew --prefix "zig@$ZIG_MAJOR_MINOR" 2>/dev/null || true)"
        if [ -n "$zig_prefix" ] && [ -x "$zig_prefix/bin/zig" ]; then
            PATH="$zig_prefix/bin:$PATH"
            export PATH
        fi
    fi

    if ! command -v zig >/dev/null 2>&1; then
        cat >&2 <<EOF
zig $ZIG_MAJOR_MINOR.x is required to build the native window's terminal.

  brew install zig@$ZIG_MAJOR_MINOR
EOF
        exit 1
    fi

    zig_version="$(zig version)"
    case "$zig_version" in
        "$ZIG_MAJOR_MINOR".*) ;;
        *)
            echo "zig $zig_version found, but Ghostty needs $ZIG_MAJOR_MINOR.x" >&2
            exit 1
            ;;
    esac
    echo "Using zig $zig_version"
}

require_zig

echo "Building moon dev tools $TAG for $TARGET_TRIPLE..."
cargo build --release --locked

# Only this release's archives are kept: the earlier releases have theirs on GitHub.
find "$ROOT_DIR/target/release-artifacts" -mindepth 1 -maxdepth 1 ! -name "$TAG" -exec rm -rf {} + 2>/dev/null || true
mkdir -p "$OUTPUT_DIR"

archive_name="moonreview-${TARGET_TRIPLE}.tar.gz"
stage_dir="$OUTPUT_DIR/stage/${TARGET_TRIPLE}"
rm -rf "$stage_dir"
mkdir -p "$stage_dir"
cp "$ROOT_DIR/target/release/$PROGRAM" "$stage_dir/$PROGRAM"
chmod 0755 "$stage_dir/$PROGRAM"
tar -C "$stage_dir" -czf "$OUTPUT_DIR/$archive_name" "$PROGRAM"
rm -rf "$OUTPUT_DIR/stage"

(
    cd "$OUTPUT_DIR"
    if command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$archive_name" >"$archive_name.sha256"
    else
        sha256sum "$archive_name" >"$archive_name.sha256"
    fi
)

echo "  $OUTPUT_DIR/$archive_name"
echo "  $OUTPUT_DIR/$archive_name.sha256"
