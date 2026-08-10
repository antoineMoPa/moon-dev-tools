#!/usr/bin/env bash
# Rasterize the logo-*.svg files at the repo root into the PNGs the executables embed —
# see src/native/logos.rs. Run this after changing a logo, and commit the PNGs it writes.
#
# Rendered here rather than at build time because the shell logo sets its text in Futura,
# and only a machine that has the font renders it right. Needs inkscape and imagemagick.
set -euo pipefail
cd "$(dirname "$0")/.."

mkdir -p assets/logos
for pair in "review moonreview" "shell moonshell" "tasks moontasks"; do
  set -- $pair
  for size in 32 64 128 256 512; do
    # Scaled to the width and padded to square: the logos are wider than they are tall.
    inkscape "logo-$1.svg" --export-type=png --export-filename=/tmp/moonreview-logo.png \
      -w "$size" 2>/dev/null
    magick /tmp/moonreview-logo.png -background none -gravity center \
      -extent "${size}x${size}" "assets/logos/$2-$size.png"
  done
done
rm -f /tmp/moonreview-logo.png
