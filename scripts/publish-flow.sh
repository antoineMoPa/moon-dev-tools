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

echo "Checking npmjs.com authentication..."
if ! NPM_USER="$(npm whoami --registry=https://registry.npmjs.org/)"; then
    echo "sign in with 'npm login --registry=https://registry.npmjs.org/' and try again" >&2
    exit 1
fi
echo "Signed in to npmjs.com as $NPM_USER."

git push --dry-run --follow-tags
npm version minor
git push --follow-tags
npm run bin-release
