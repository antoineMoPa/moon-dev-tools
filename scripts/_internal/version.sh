#!/usr/bin/env bash
# The version of the release, which is the crate's own: `version` under [package] in Cargo.toml.

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

awk '
    /^\[/ { in_package = ($0 == "[package]") }
    in_package && /^version = / {
        gsub(/^version = "|"$/, "")
        print
        exit
    }
' "$ROOT_DIR/Cargo.toml"
