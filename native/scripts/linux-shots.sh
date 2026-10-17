#!/usr/bin/env bash
# Linux verification frames: builds the `shots` variant on the Linux host (SB_LINUX_HOST) and runs it on a private Xvfb display with
# lavapipe (software Vulkan), so nothing shows on any screen. Frames come back to <out-dir> on this machine.
#   native/scripts/linux-shots.sh <out-dir> <name>:<SB_SHOT state>:<theme> ...      one frame per argument
#   SB_DRIVE_FILE=script.txt native/scripts/linux-shots.sh <out-dir> <name>::<theme>  drives the app ($O = frames dir)
# SB_AS_WINDOWS=1 draws the Windows header; SB_SHOT_SCROLL=<px> scrolls a Settings page down. The app reads SWITCHBOARD_URL, else SB_SHOTS_URL from hosts.sh; point it at a
# fake server before driving any action, only GETs may reach the live one.
set -euo pipefail
cd "$(dirname "$0")/.."
. scripts/hosts.sh
sb_hosts SB_LINUX_HOST
H=$SB_LINUX_HOST
REMOTE=${SB_REMOTE:-switchboard-native-shots}
OUT=${1:?out dir}; shift
URL=${SWITCHBOARD_URL:-}
[ -n "$URL" ] || { sb_hosts SB_SHOTS_URL; URL=$SB_SHOTS_URL; }
mkdir -p "$OUT"
ssh -q $H "mkdir -p $REMOTE"
rsync -a --delete --exclude target --exclude dist --exclude frames ./ ../branding.toml $H:$REMOTE/
DRIVE=""
if [ -n "${SB_DRIVE_FILE:-}" ]; then
  scp -q "$SB_DRIVE_FILE" $H:$REMOTE/drive.txt
  DRIVE=drive.txt
fi
ssh -q $H "export PATH=\$HOME/.cargo/bin:\$PATH; cd $REMOTE && cargo build --locked -q --profile shots -p sb-desktop --features shots 2>&1 | grep -E '^error' -A6 | head -40; test -x target/shots/Switchboard"
for spec in "$@"; do
  IFS=: read -r name state theme <<<"$spec"
  ssh -q $H "cd $REMOTE && ${SB_AS_WINDOWS:+SB_AS_WINDOWS=1} ${SB_PERF:+SB_PERF=1} ${SB_SHOT_SCROLL:+SB_SHOT_SCROLL=$SB_SHOT_SCROLL} bash scripts/remote/linux-shot.sh '$name' '$state' '${theme:-light}' '$URL' '$DRIVE'" || true
  rsync -a --include='*.png' --include='*.log' --exclude='*' "$H:$REMOTE/frames/$name/" "$OUT/"
done
