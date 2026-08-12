#!/usr/bin/env bash

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

if (($# > 0)); then
    echo "usage: scripts/bin-release.sh" >&2
    exit 1
fi

sh -n install.sh
bash -n scripts/_internal/build-release.sh
bash -n scripts/_internal/upload-release.sh

bash scripts/_internal/build-release.sh

bash scripts/_internal/upload-release.sh
