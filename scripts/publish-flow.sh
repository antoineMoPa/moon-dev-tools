#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

if (($# > 0)); then
    echo "usage: scripts/publish-flow.sh" >&2
    exit 1
fi

if [ -n "$(git status --porcelain)" ]; then
    echo "the worktree must be clean before publishing" >&2
    exit 1
fi

if ! git rev-parse --abbrev-ref --symbolic-full-name '@{upstream}' >/dev/null 2>&1; then
    echo "the current branch must have an upstream before publishing" >&2
    exit 1
fi

git push --dry-run --follow-tags

# The release version is the crate's, so the bump is a commit on Cargo.toml and its lockfile
# entry, tagged the way the release assets are named.
VERSION="$(bash scripts/_internal/version.sh)"
NEXT_VERSION="$(awk -F. '{ printf "%s.%s.0", $1, $2 + 1 }' <<<"$VERSION")"
echo "Bumping $VERSION to $NEXT_VERSION..."
sed -i '' "s/^version = \"$VERSION\"$/version = \"$NEXT_VERSION\"/" Cargo.toml
cargo update --workspace --offline
git commit -am "v$NEXT_VERSION"
# Annotated, since `git push --follow-tags` below skips lightweight tags.
git tag -a "v$NEXT_VERSION" -m "v$NEXT_VERSION"

git push --follow-tags
bash scripts/bin-release.sh
