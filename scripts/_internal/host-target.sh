#!/usr/bin/env bash
# host-target.sh - prints the release target triple this machine builds.

set -euo pipefail

case "$(uname -s) $(uname -m)" in
    "Darwin arm64") echo "aarch64-apple-darwin" ;;
    "Linux x86_64") echo "x86_64-unknown-linux-gnu" ;;
    "Linux aarch64") echo "aarch64-unknown-linux-gnu" ;;
    *)
        echo "no release target is built on $(uname -s) $(uname -m)" >&2
        exit 1
        ;;
esac
