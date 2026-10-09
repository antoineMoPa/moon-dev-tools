#!/usr/bin/env bash
# Removes every build of this package from target/, which is what a version bump calls for.
# Cargo names a package's artifacts and incremental caches after a hash that its version is
# part of, so the build after a bump starts new ones beside the old, and nothing reads or
# removes the old ones again. The dependencies' builds are not named after this package's
# version, and stay.

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT_DIR"

PACKAGE="moon-review"
WASM_TARGET="wasm32-unknown-unknown"

# Each place the package is built, as the `cargo clean` arguments that name it: the native debug
# and release builds, the browser build build.rs makes for each of them in `web-build`, and the
# Linux release build of build-release.sh, which has a browser build of its own.
BUILDS=(
    ""
    "--release"
    "--target-dir target/web-build --target $WASM_TARGET"
    "--target-dir target/web-build --target $WASM_TARGET --release"
    "--target-dir target/docker-linux --target x86_64-unknown-linux-gnu --target aarch64-unknown-linux-gnu --release"
    "--target-dir target/docker-linux/web-build --target $WASM_TARGET --release"
)

for build in "${BUILDS[@]}"; do
    # Unquoted: each entry is several arguments.
    # shellcheck disable=SC2086
    cargo clean --package "$PACKAGE" $build
done
