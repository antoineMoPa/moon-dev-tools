#!/usr/bin/env bash

# The other half of prepare-for-install-test.sh: take the downloaded release back out and put
# the build you develop with in charge again.
#
# install.sh puts its download in ~/.local/bin, which comes before ~/.cargo/bin on the PATH -
# so a `cargo install --path .` afterwards lands in a directory the shell never reaches, and
# `moon` goes on being the release you were testing, launchers and all. This takes the
# download away, builds this checkout, and points the launchers at what it built.

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

PROGRAMS=(moon moonreview moontasks moonshell)
INSTALL_DIR="${MOONREVIEW_INSTALL_DIR:-$HOME/.local/bin}"
CARGO_BIN_DIR="${CARGO_INSTALL_ROOT:-$HOME/.cargo}/bin"

if [ "$INSTALL_DIR" = "$CARGO_BIN_DIR" ]; then
    echo "cleanup-after-install-test: the download and the build share $INSTALL_DIR" >&2
    echo "there is nothing to take out of the way; run the build and stop." >&2
    exit 1
fi

for program in "${PROGRAMS[@]}"; do
    rm -f "$INSTALL_DIR/$program"
done

cargo install --locked --force --path .
"$CARGO_BIN_DIR/moon" install-launchers

printf '\n%s is what `moon` runs again, built from %s.\n' "$CARGO_BIN_DIR/moon" "$ROOT_DIR"
