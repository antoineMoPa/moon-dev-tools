#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

PACKAGE_VERSION="$(node -p "require('./package.json').version")"
TAG="v$PACKAGE_VERSION"
OUTPUT_DIR="$ROOT_DIR/target/release-artifacts/$TAG"
MACOS_TARGET_TRIPLE="aarch64-apple-darwin"
LINUX_TARGET_TRIPLES=(
    "x86_64-unknown-linux-gnu"
    "aarch64-unknown-linux-gnu"
)
RUST_TOOLCHAIN="${MOONREVIEW_RUST_TOOLCHAIN:-1.95.0}"
LINUX_DOCKER_BASE_IMAGE="${MOONREVIEW_LINUX_DOCKER_BASE_IMAGE:-${MOONREVIEW_LINUX_DOCKER_IMAGE:-debian:bookworm}}"
LINUX_DOCKER_BUILDER_IMAGE_PREFIX="${MOONREVIEW_LINUX_DOCKER_BUILDER_IMAGE_PREFIX:-moonreview-linux-builder}"

default_linux_build_platform() {
    case "$(uname -m)" in
        arm64 | aarch64)
            echo "linux/arm64"
            ;;
        x86_64 | amd64)
            echo "linux/amd64"
            ;;
        *)
            echo ""
            ;;
    esac
}

LINUX_BUILD_PLATFORM="${MOONREVIEW_LINUX_BUILD_PLATFORM:-$(default_linux_build_platform)}"

checksum_file() {
    archive_path="$1"

    (
        cd "$OUTPUT_DIR"
        if command -v shasum >/dev/null 2>&1; then
            shasum -a 256 "$(basename "$archive_path")" >"$(basename "$archive_path").sha256"
        elif command -v sha256sum >/dev/null 2>&1; then
            sha256sum "$(basename "$archive_path")" >"$(basename "$archive_path").sha256"
        else
            echo "missing checksum tool (shasum or sha256sum)" >&2
            exit 1
        fi
    )
}

package_binary() {
    target_triple="$1"
    binary_path="$2"
    asset_basename="moonreview-${target_triple}"
    stage_dir="$OUTPUT_DIR/stage/${target_triple}"
    archive_path="$OUTPUT_DIR/${asset_basename}.tar.gz"

    mkdir -p "$stage_dir"

    cp "$binary_path" "$stage_dir/moonreview"
    chmod 0755 "$stage_dir/moonreview"

    tar -C "$stage_dir" -czf "$archive_path" moonreview
    checksum_file "$archive_path"

    echo "  $archive_path"
    echo "  ${archive_path}.sha256"
}

ZIG_MAJOR_MINOR="0.15"

# The native window's terminal comes from Ghostty's Zig source, so a matching Zig has to be
# on PATH before anything is built. Homebrew keeps 0.15 keg-only, so look there too.
require_zig() {
    if ! command -v zig >/dev/null 2>&1 && command -v brew >/dev/null 2>&1; then
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

Or build the web frontend only, with: cargo build --release --no-default-features
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

build_macos_arm64() {
    echo "Building moonreview $TAG for $MACOS_TARGET_TRIPLE..."
    cargo build --release --locked
    package_binary "$MACOS_TARGET_TRIPLE" "$ROOT_DIR/target/release/moonreview"
}

build_linux() {
    target_triple="$1"
    target_dir="/work/target/docker-linux-${target_triple}"
    builder_image="${MOONREVIEW_LINUX_DOCKER_BUILDER_IMAGE:-$LINUX_DOCKER_BUILDER_IMAGE_PREFIX:$RUST_TOOLCHAIN-$target_triple}"

    if ! command -v docker >/dev/null 2>&1; then
        echo "Docker is required to build $target_triple." >&2
        exit 1
    fi

    echo "Preparing Linux builder image $builder_image..."
    platform_args=()
    if [ -n "$LINUX_BUILD_PLATFORM" ]; then
        platform_args=(--platform "$LINUX_BUILD_PLATFORM")
    fi

    docker build \
        "${platform_args[@]}" \
        --build-arg BASE_IMAGE="$LINUX_DOCKER_BASE_IMAGE" \
        --build-arg LINUX_TARGET_TRIPLE="$target_triple" \
        --build-arg RUST_TOOLCHAIN="$RUST_TOOLCHAIN" \
        -t "$builder_image" \
        -f scripts/linux-build.Dockerfile \
        scripts

    echo "Building moonreview $TAG for $target_triple with Docker..."
    docker run --rm \
        "${platform_args[@]}" \
        -e DEBIAN_FRONTEND=noninteractive \
        -e CARGO_HOME=/work/target/docker-cargo-home \
        -e RUSTUP_HOME=/opt/rust/rustup \
        -e CARGO_TARGET_DIR="$target_dir" \
        -e CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc \
        -e CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER=x86_64-linux-gnu-gcc \
        -e LINUX_TARGET_TRIPLE="$target_triple" \
        -e HOST_UID="$(id -u)" \
        -e HOST_GID="$(id -g)" \
        -v "$ROOT_DIR:/work" \
        -w /work \
        "$builder_image" \
        bash -lc '
            set -euo pipefail
            export PATH="/opt/rust/cargo/bin:$PATH"
            cargo build --release --locked --target "$LINUX_TARGET_TRIPLE"
            chown -R "$HOST_UID:$HOST_GID" "$CARGO_TARGET_DIR" /work/target/docker-cargo-home /work/node_modules /work/web/dist 2>/dev/null || true
        '

    package_binary "$target_triple" "$ROOT_DIR/target/docker-linux-${target_triple}/$target_triple/release/moonreview"
}

require_zig

mkdir -p "$OUTPUT_DIR"

echo "Created release artifacts:"
build_macos_arm64
for target_triple in "${LINUX_TARGET_TRIPLES[@]}"; do
    build_linux "$target_triple"
done

cat <<EOF
Next step:
  scripts/upload-release.sh
EOF
