#!/usr/bin/env bash
# Animate a recorded MarketEvent file in the terminal.
#
# Frame k is the matching-engine adapter's offline `--replay` of the recording's
# first k heartbeats, so every frame is the exact, deterministic chart state at
# that moment of the session — nothing is interpolated. Wall-clock time only
# paces the playback.
#
#   scripts/play-recording.sh <recording> [options] [-- <adapter replay flags>]
#
#   --from N      first heartbeat to show (default 1)
#   --to M        last heartbeat to show (default: the last one in the file)
#   --step K      heartbeats advanced per frame (default 1)
#   --delay S     seconds between frames (default 0.08)
#
# Everything after `--` goes to `examples-matching-engine --replay`, e.g.
#
#   scripts/play-recording.sh recordings/large_buy_sweep.txt --from 100 --to 200 \
#       -- --interval-secs 1 --tick-size 0.01 --window 48 --heatmap
#
# Needs a terminal of at least 120x31 (the replay frame is a fixed 120x30).
set -euo pipefail

usage() { sed -n '2,21p' "$0" | sed 's/^# \{0,1\}//'; exit "${1:-0}"; }

[[ $# -ge 1 ]] || usage 1
[[ $1 == -h || $1 == --help ]] && usage 0
recording=$1
shift
[[ -f $recording ]] || { echo "no such recording: $recording" >&2; exit 1; }

from=1 to=0 step=1 delay=0.08
while [[ $# -gt 0 ]]; do
  case $1 in
    --from) from=$2; shift 2 ;;
    --to) to=$2; shift 2 ;;
    --step) step=$2; shift 2 ;;
    --delay) delay=$2; shift 2 ;;
    --) shift; break ;;
    -h|--help) usage 0 ;;
    *) echo "unknown option: $1" >&2; usage 1 ;;
  esac
done

root="$(cd "$(dirname "$0")/.." && pwd)"
cargo build --quiet --release -p examples-matching-engine --manifest-path "$root/Cargo.toml"
bin="${CARGO_TARGET_DIR:-$root/target}/release/examples-matching-engine"

# Line number of every heartbeat: the frame boundaries.
heartbeats=()
while IFS= read -r n; do heartbeats+=("$n"); done < <(awk '/^H /{print NR}' "$recording")
total=${#heartbeats[@]}
(( total > 0 )) || { echo "no heartbeats in $recording" >&2; exit 1; }
(( to == 0 || to > total )) && to=$total

# Frames replay a prefix copy under the recording's own name, so the chart's
# title shows that name rather than a temp path.
work="$(mktemp -d)"
name="$(basename "$recording")"
trap 'rm -rf "$work"; tput cnorm 2>/dev/null || true' EXIT
tput civis 2>/dev/null || true
printf '\033[2J'

for (( k = from; k <= to; k += step )); do
  head -n "${heartbeats[k - 1]}" "$recording" > "$work/$name"
  # Repaint in place. Writing straight to the terminal (not via a pipe) keeps the
  # replay's ANSI color on.
  printf '\033[H'
  (cd "$work" && "$bin" --replay "$name" "$@")
  sleep "$delay"
done
