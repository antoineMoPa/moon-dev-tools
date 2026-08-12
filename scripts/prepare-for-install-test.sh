#!/usr/bin/env bash

set -euo pipefail

PROGRAMS=(moonreview moontasks moonshell)
INSTALL_DIR="${MOONREVIEW_INSTALL_DIR:-$HOME/.local/bin}"
CARGO_BIN_DIR="${CARGO_INSTALL_ROOT:-$HOME/.cargo}/bin"

for program in "${PROGRAMS[@]}"; do
    rm -f "$INSTALL_DIR/$program" "$CARGO_BIN_DIR/$program"
done

case "$(uname -s)" in
    Darwin)
        for application in Moonreview Moontasks Moonshell; do
            rm -rf "/Applications/$application.app" "$HOME/Applications/$application.app"
        done
        ;;
    Linux)
        for program in "${PROGRAMS[@]}"; do
            rm -f \
                "$HOME/.local/share/applications/$program.desktop" \
                "$HOME/.local/share/icons/hicolor/256x256/apps/$program.png"
        done
        if command -v update-desktop-database >/dev/null 2>&1; then
            update-desktop-database "$HOME/.local/share/applications"
        fi
        ;;
esac

printf 'Removed Moon tool binaries and desktop launchers. Build and Cargo caches are intact.\n'
