#!/usr/bin/env bash
# One verification frame on this Mac: opens the shots bundle in a state, saves the GPU frame, quits.
#   native/scripts/mac-shot.sh <out.png> [SB_SHOT state or ""] [dark|light] [WxH] [bundle dir]
# Uses its own settings file, so the real app's settings are never touched. Only GETs reach the live server: set
# SWITCHBOARD_URL to a fake server (scratch fakesrv.py) before driving any action. SB_DRIVE (a script, see view.rs
# capture_when_ready) clicks and types through the app and takes `shot` frames on the way.
set -euo pipefail
out=$1; state=${2:-}; theme=${3:-dark}; size=${4:-1440x900}; dir=${5:-/tmp/sb-shots}
log="${out%.png}.log"
rm -f "$out"
open -n -W "$dir/SwitchboardShots.app" --env SB_SHOT_OUT="$out" --env SB_SHOT="$state" --env SB_THEME="$theme" \
  --env SB_SIZE="$size" --env SB_SETTINGS="$dir/settings.json" --env SB_TIMING=1 \
  --env SB_DRIVE="${SB_DRIVE:-}" --env SWITCHBOARD_URL="${SWITCHBOARD_URL:-}" --env SWITCHBOARD_CACHE="${SWITCHBOARD_CACHE:-}" \
  --stderr "$log" --stdout "$log"
grep -E "sb-shot|sb-timing|sb-drive unknown|bad key|panic|rror" "$log" | head -20
