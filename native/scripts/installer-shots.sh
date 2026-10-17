#!/usr/bin/env bash
# Screenshots of the installer's screens: builds sb-install with the `shots` feature on the Linux host (SB_LINUX_HOST) and
# runs each screen on a private Xvfb display the size of the window, with lavapipe (software Vulkan), so nothing shows on
# any screen and nothing is installed. The sample platform answers like a Mac and changes nothing.
#   native/scripts/installer-shots.sh <out-dir> [screen ...]      default: every screen, light and dark
# Screens: welcome, welcome-update, welcome-update-running, welcome-downgrade, welcome-home, tour-1..4, options,
# options-home, progress, progress-stuck, done, done-update, done-notes, uninstall, uninstall-data, uninstall-done, error.
# SB_REMOTE names the folder on the host (default switchboard-installer-shots); SB_TARGET sets its CARGO_TARGET_DIR.
set -euo pipefail
cd "$(dirname "$0")/.."
. scripts/hosts.sh
sb_hosts SB_LINUX_HOST
H=$SB_LINUX_HOST
REMOTE=${SB_REMOTE:-switchboard-installer-shots}
OUT=${1:?out dir}; shift
SCREENS=("$@")
[ ${#SCREENS[@]} -gt 0 ] || SCREENS=(welcome welcome-update welcome-update-running welcome-downgrade welcome-home
  tour-1 tour-2 tour-3 tour-4 options options-home progress progress-stuck done done-update done-notes
  uninstall uninstall-data uninstall-done error)
mkdir -p "$OUT"
# The repo root's branding.toml sits next to native/ there too, where build.rs looks for it.
ssh -q $H "mkdir -p $REMOTE/native"
rsync -a --delete --exclude target --exclude dist --exclude frames ./ $H:$REMOTE/native/
rsync -a ../branding.toml $H:$REMOTE/
TGT=${SB_TARGET:-\$PWD/target}
ssh -q $H "export PATH=\$HOME/.cargo/bin:\$PATH CARGO_TARGET_DIR=$TGT; cd $REMOTE/native && cargo build --locked -q -p sb-install --features shots 2>&1 | grep -E '^error' -A6 | head -40; test -x $TGT/debug/switchboard-install"
for screen in "${SCREENS[@]}"; do
  for theme in light dark; do
    name="$screen-$theme"
    ssh -q $H "cd $REMOTE/native && mkdir -p frames && rm -f frames/$name.png &&
      VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json SB_SHOT=$screen SB_THEME=$theme SB_SHOT_OUT=\$PWD/frames/$name.png \
      timeout 120 xvfb-run -a -s '-screen 0 800x600x24' $TGT/debug/switchboard-install 2>&1 | grep -E 'sb-shot|panic|ERROR' | head -4" || true
  done
done
rsync -a --include='*.png' --exclude='*' "$H:$REMOTE/native/frames/" "$OUT/"
ls "$OUT" | wc -l
