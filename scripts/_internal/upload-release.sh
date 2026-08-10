#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT_DIR"

PACKAGE_VERSION="$(node -p "require('./package.json').version")"
TAG="v$PACKAGE_VERSION"
OUTPUT_DIR="$ROOT_DIR/target/release-artifacts/$TAG"
TARGET_TRIPLES=(
    "aarch64-apple-darwin"
    "x86_64-unknown-linux-gnu"
    "aarch64-unknown-linux-gnu"
)
ASSET_PATHS=()

if ! command -v gh >/dev/null 2>&1; then
    echo "GitHub CLI (gh) is required to upload release assets." >&2
    exit 1
fi

for target_triple in "${TARGET_TRIPLES[@]}"; do
    archive_path="$OUTPUT_DIR/moonreview-${target_triple}.tar.gz"
    checksum_path="${archive_path}.sha256"

    if [ ! -f "$archive_path" ] || [ ! -f "$checksum_path" ]; then
        echo "missing release assets for $TAG and $target_triple; run scripts/_internal/build-release.sh first" >&2
        exit 1
    fi

    ASSET_PATHS+=("$archive_path" "$checksum_path")
done

if gh release view "$TAG" >/dev/null 2>&1; then
    echo "release $TAG already exists; run 'npm version minor' before uploading release assets" >&2
    exit 1
else
    echo "Creating release $TAG and uploading assets..."
    gh release create "$TAG" "${ASSET_PATHS[@]}" \
        --title "$TAG" \
        --notes "moonreview ${TAG#v}"
fi

echo "Release ready: $TAG"
