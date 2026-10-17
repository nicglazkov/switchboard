#!/usr/bin/env bash
# Screenshots in a VM with no fleet: builds the `shots` variant, serves the fixture (scripts/fixture-server.py) and runs
# the app on a private Xvfb display with lavapipe.
#   native/scripts/vm-shot.sh <out-dir> <name>:<SB_SHOT state>:<theme>[:<w>x<h>] ...
# SB_DRIVE_FILE=script.txt drives the app instead of one frame ($O in it is the frames dir). SB_NO_BUILD=1 skips the build.
set -uo pipefail
cd "$(dirname "$0")/.."
OUT=$(realpath -m "${1:?out dir}"); shift
mkdir -p "$OUT"
if [ -z "${SB_NO_BUILD:-}" ]; then
  cargo build --locked -q --profile shots -p sb-desktop --features shots 2>&1 | grep -E '^error' -A8 | head -60
fi
test -x target/shots/Switchboard || { echo "no build"; exit 1; }
PORT=${SB_FIXTURE_PORT:-8139}
python3 scripts/fixture-server.py "$PORT" & FS=$!
trap 'kill $FS 2>/dev/null' EXIT
sleep 0.5
for spec in "$@"; do
  IFS=: read -r name state theme size <<<"$spec"
  size=${size:-1440x900}
  O="$OUT/.run-$name"; rm -rf "$O"; mkdir -p "$O"
  DRIVE=""
  if [ -n "${SB_DRIVE_FILE:-}" ]; then DRIVE="$(sed "s#\\\$O#$OUT#g" "$SB_DRIVE_FILE")"; fi
  VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json SWITCHBOARD_URL="http://127.0.0.1:$PORT" SWITCHBOARD_CACHE="$O/cache" \
    SB_SETTINGS="$O/settings.json" SB_SHOT="$state" SB_THEME="${theme:-light}" SB_SIZE="$size" SB_SHOT_OUT="$OUT/$name.png" \
    SB_DRIVE="$DRIVE" timeout 120 xvfb-run -a -s "-screen 0 ${size}x24" target/shots/Switchboard >"$O/log" 2>&1
  grep -E 'sb-shot|sb-drive unknown|bad key|panic|ERROR' "$O/log" | head -8
done
