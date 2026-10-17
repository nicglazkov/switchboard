#!/usr/bin/env bash
# Runs on the Linux host (from scripts/linux-shots.sh): one shots run on a private Xvfb display with lavapipe.
#   linux-shot.sh <name> <SB_SHOT state> <theme> <server url> [drive file]
set -uo pipefail
cd "$(dirname "$0")/../.."
name=$1; state=$2; theme=${3:-light}; url=$4; drive=${5:-}
O="$PWD/frames/$name"
rm -rf "${O:?}"
mkdir -p "$O"
export VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json
export SWITCHBOARD_URL="$url" SWITCHBOARD_CACHE="$O/cache" SB_SETTINGS="$O/settings.json"
export SB_SHOT="$state" SB_THEME="$theme" SB_SIZE=1440x900 SB_SHOT_OUT="$O/$name.png" SB_TIMING=1
if [ -n "$drive" ] && [ -f "$drive" ]; then
  SB_DRIVE="$(sed "s#\\\$O#$O#g" "$drive")"
  export SB_DRIVE
fi
timeout 180 xvfb-run -a -s '-screen 0 1440x900x24' target/shots/Switchboard >"$O/$name.log" 2>&1
grep -E 'sb-shot|sb-drive unknown|bad key|panic|ERROR' "$O/$name.log" | head -12
