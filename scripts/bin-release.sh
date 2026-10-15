#!/usr/bin/env bash
# bin-release.sh - fills the draft release of the checked-out version tag and publishes it. This
# machine builds its own target and uploads it; GitHub Actions (.github/workflows/release.yml)
# builds the others. Nothing is published until every archive is on the draft and checks out.
# Run again from the same tag to retry after a failure.

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

if (($# > 0)); then
    echo "usage: scripts/bin-release.sh" >&2
    exit 1
fi

TAG="v$(bash scripts/_internal/version.sh)"
WORKFLOW="release.yml"
# Every archive the release carries; install.sh downloads one of these.
TARGET_TRIPLES=(
    "aarch64-apple-darwin"
    "x86_64-unknown-linux-gnu"
    "aarch64-unknown-linux-gnu"
)

if [ -n "$(git status --porcelain)" ]; then
    echo "the worktree must be clean before releasing" >&2
    exit 1
fi
if [ "$(git rev-parse "$TAG^{commit}")" != "$(git rev-parse HEAD)" ]; then
    echo "check out $TAG before releasing it" >&2
    exit 1
fi
if [ "$(gh release view "$TAG" --json isDraft --jq .isDraft)" != true ]; then
    echo "$TAG has no draft release to fill" >&2
    exit 1
fi

sh -n install.sh

LOCAL_TARGET="$(bash scripts/_internal/host-target.sh)"
bash scripts/_internal/build-release.sh "$LOCAL_TARGET"
local_archive="target/release-artifacts/$TAG/moonreview-${LOCAL_TARGET}.tar.gz"
gh release upload "$TAG" "$local_archive" "$local_archive.sha256" --clobber

newest_run_id() {
    gh run list --workflow "$WORKFLOW" --branch "$TAG" --event workflow_dispatch \
        --json databaseId --jq '.[0].databaseId // ""' --limit 1
}

# `gh workflow run` answers without the run's id, so the run is the newest once it is not the
# one that was newest before.
previous_run_id="$(newest_run_id)"
gh workflow run "$WORKFLOW" --ref "$TAG" -f local_target="$LOCAL_TARGET"
run_id=""
for _ in $(seq 60); do
    run_id="$(newest_run_id)"
    [ "$run_id" != "$previous_run_id" ] && break
    sleep 2
done
if [ "$run_id" = "$previous_run_id" ]; then
    echo "the $WORKFLOW run did not appear; $TAG stays a draft" >&2
    exit 1
fi
gh run watch "$run_id" --exit-status

# Into an empty folder, so a file left from an earlier attempt cannot stand in for a missing upload.
downloaded_dir="target/release-artifacts/$TAG/downloaded"
rm -rf "$downloaded_dir"
gh release download "$TAG" --dir "$downloaded_dir"
for target_triple in "${TARGET_TRIPLES[@]}"; do
    archive_name="moonreview-${target_triple}.tar.gz"
    (cd "$downloaded_dir" && shasum -a 256 -c "$archive_name.sha256")
    if [ "$(tar -tzf "$downloaded_dir/$archive_name")" != "moon" ]; then
        echo "$archive_name holds something other than just moon" >&2
        exit 1
    fi
done

gh release edit "$TAG" --draft=false --latest
echo "Release ready: $TAG"
