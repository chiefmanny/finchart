#!/usr/bin/env bash
# Re-record the README demos from their VHS tapes, then optimize them for the web.
#
#   docs/media/render.sh            # all demos
#   docs/media/render.sh sweep      # one demo: live | tui | sweep | crisis | crosshair
#
# Needs vhs (https://github.com/charmbracelet/vhs) and ffmpeg. Each tape writes a
# raw capture (<name>.raw.gif / .raw.png); this script re-encodes it to a 10 fps,
# 64-color GIF and an 8-bit PNG, then deletes the raw files.
set -euo pipefail

root="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$root"

# Build first so no tape waits on a compile.
cargo build --quiet --release -p finchart

for name in "${@:-live tui sweep crisis crosshair}"; do
  for demo in $name; do
    tape="docs/media/$demo.tape"
    [[ -f $tape ]] || { echo "no such tape: $tape" >&2; exit 1; }
    echo "recording $demo"
    vhs "$tape" >/dev/null
    ffmpeg -loglevel error -y -i "docs/media/$demo.raw.gif" -vf \
      "fps=10,split[a][b];[a]palettegen=max_colors=64:stats_mode=diff[p];[b][p]paletteuse=dither=none:diff_mode=rectangle" \
      "docs/media/$demo.gif"
    ffmpeg -loglevel error -y -i "docs/media/$demo.raw.png" -vf \
      "split[a][b];[a]palettegen=max_colors=64[p];[b][p]paletteuse=dither=none" \
      "docs/media/$demo.png"
    rm -f "docs/media/$demo.raw.gif" "docs/media/$demo.raw.png"
    ls -lh "docs/media/$demo.gif" "docs/media/$demo.png" | awk '{print "  " $5 "  " $NF}'
  done
done
