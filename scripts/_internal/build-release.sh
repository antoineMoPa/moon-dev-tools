#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT_DIR"

TAG="v$(bash "$ROOT_DIR/scripts/_internal/version.sh")"
OUTPUT_DIR="$ROOT_DIR/target/release-artifacts/$TAG"
MACOS_TARGET_TRIPLE="aarch64-apple-darwin"
# The executable Cargo builds. install.sh expects it in the archive.
PROGRAMS=(
    "moon"
)
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

package_binaries() {
    target_triple="$1"
    build_dir="$2"
    asset_basename="moonreview-${target_triple}"
    stage_dir="$OUTPUT_DIR/stage/${target_triple}"
    archive_path="$OUTPUT_DIR/${asset_basename}.tar.gz"

    rm -rf "$stage_dir"
    mkdir -p "$stage_dir"

    for program in "${PROGRAMS[@]}"; do
        if [ ! -f "$build_dir/$program" ]; then
            echo "$target_triple build produced no $program in $build_dir" >&2
            exit 1
        fi
        cp "$build_dir/$program" "$stage_dir/$program"
        chmod 0755 "$stage_dir/$program"
    done

    tar -C "$stage_dir" -czf "$archive_path" "${PROGRAMS[@]}"
    checksum_file "$archive_path"

    echo "  $archive_path (${PROGRAMS[*]})"
    echo "  ${archive_path}.sha256"
}

ZIG_MAJOR_MINOR="0.15"

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

build_macos_arm64() {
    echo "Building moon dev tools $TAG for $MACOS_TARGET_TRIPLE..."
    cargo build --release --locked
    package_binaries "$MACOS_TARGET_TRIPLE" "$ROOT_DIR/target/release"
}

LINUX_ZIG_GLOBAL_CACHE_DIR="$ROOT_DIR/target/docker-zig-global-cache"

# Zig's HTTP client drops Ghostty's package downloads inside the containers (ReadFailed,
# HttpConnectionClosing), while the macOS build fetches them fine. Packages sit under their
# content hash in `p/` and are the same on every platform, so the containers start from the
# ones the macOS build fetched and only download what Linux alone needs.
seed_linux_zig_packages() {
    host_zig_global_cache_dir="$(zig env | sed -n 's/^ *\.global_cache_dir = "\(.*\)",$/\1/p')"
    if [ ! -d "$host_zig_global_cache_dir/p" ]; then
        echo "the macOS build left no Zig packages in $host_zig_global_cache_dir/p" >&2
        exit 1
    fi
    mkdir -p "$LINUX_ZIG_GLOBAL_CACHE_DIR/p"
    rsync -a --ignore-existing "$host_zig_global_cache_dir/p/" "$LINUX_ZIG_GLOBAL_CACHE_DIR/p/"
}

# Both triples in one cargo and one target directory: the host half of the build - build scripts,
# proc macros, and the browser build moon's build.rs compiles into `<target dir>/web-build` - is
# the same for both, and is compiled once instead of once per triple.
build_linux() {
    target_dir="/work/target/docker-linux"
    builder_image="${MOONREVIEW_LINUX_DOCKER_BUILDER_IMAGE:-$LINUX_DOCKER_BUILDER_IMAGE_PREFIX:$RUST_TOOLCHAIN}"

    if ! command -v docker >/dev/null 2>&1; then
        echo "Docker is required to build ${LINUX_TARGET_TRIPLES[*]}." >&2
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
        --build-arg LINUX_TARGET_TRIPLES="${LINUX_TARGET_TRIPLES[*]}" \
        --build-arg RUST_TOOLCHAIN="$RUST_TOOLCHAIN" \
        -t "$builder_image" \
        -f scripts/_internal/linux-build.Dockerfile \
        scripts

    echo "Building moon dev tools $TAG for ${LINUX_TARGET_TRIPLES[*]} with Docker..."
    docker run --rm \
        "${platform_args[@]}" \
        -e DEBIAN_FRONTEND=noninteractive \
        -e CARGO_HOME=/work/target/docker-cargo-home \
        -e ZIG_GLOBAL_CACHE_DIR=/work/target/docker-zig-global-cache \
        -e RUSTUP_HOME=/opt/rust/rustup \
        -e CARGO_TARGET_DIR="$target_dir" \
        -e CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc \
        -e CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER=x86_64-linux-gnu-gcc \
        -e LINUX_TARGET_TRIPLES="${LINUX_TARGET_TRIPLES[*]}" \
        -e HOST_UID="$(id -u)" \
        -e HOST_GID="$(id -g)" \
        -v "$ROOT_DIR:/work" \
        -w /work \
        "$builder_image" \
        bash -c '
            set -euo pipefail
            # libghostty-vt-sys clones Ghostty into the bind-mounted target dir, whose owner
            # does not match the container user, so git refuses to touch it without this.
            git config --global --add safe.directory "*"
            target_args=()
            for target_triple in $LINUX_TARGET_TRIPLES; do
                target_args+=(--target "$target_triple")
            done
            cargo build --release --locked "${target_args[@]}"
            chown -R "$HOST_UID:$HOST_GID" "$CARGO_TARGET_DIR" /work/target/docker-cargo-home "$ZIG_GLOBAL_CACHE_DIR" 2>/dev/null || true
        '

    for target_triple in "${LINUX_TARGET_TRIPLES[@]}"; do
        package_binaries "$target_triple" "$ROOT_DIR/target/docker-linux/$target_triple/release"
    done
}

require_zig

# `moon licenses` prints the file compiled into the executable, so it has to name what this
# build links before anything is built.
"$ROOT_DIR/scripts/third-party-licenses.py" --check

mkdir -p "$OUTPUT_DIR"

echo "Created release artifacts:"
build_macos_arm64
seed_linux_zig_packages
build_linux

cat <<EOF
Next step:
  scripts/_internal/upload-release.sh
EOF
