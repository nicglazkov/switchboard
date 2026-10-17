#!/usr/bin/env bash
# Regenerate the installer's art from branding.toml (scripts/installer_art.py): the disk image background and the
# installer window's header. Rendering (rsvg-convert, the app's fonts) happens on SB_LINUX_HOST; see hosts.sh.
#   native/scripts/make-installer-art.sh
# Output: native/packaging/installer/{dmg-background.png,dmg-background@2x.png,header-light.png,header-dark.png}
#         native/packaging/windows/setup.ico (the Windows installer's icon, needs Pillow on the Linux host)
set -euo pipefail
cd "$(dirname "$0")/.."
. scripts/hosts.sh
sb_hosts SB_LINUX_HOST
H=$SB_LINUX_HOST
RW=$(ssh -q $H 'mktemp -d /tmp/sb-installer-art.XXXXXX')
trap 'ssh -q $H "rm -rf $RW"' EXIT

ssh -q $H "mkdir -p $RW/native/scripts $RW/native/crates/sb-desktop/assets $RW/native/packaging/brand"
rsync -a scripts/installer_art.py $H:$RW/native/scripts/
rsync -a packaging/brand/icon-square.svg $H:$RW/native/packaging/brand/
rsync -a crates/sb-desktop/assets/fonts $H:$RW/native/crates/sb-desktop/assets/
rsync -a ../branding.toml $H:$RW/
ssh -q $H "command -v rsvg-convert >/dev/null || { echo 'install librsvg2-bin on the Linux host' >&2; exit 1; }
python3 $RW/native/scripts/installer_art.py $RW/out"
mkdir -p packaging/installer packaging/windows
rsync -a --exclude setup.ico "$H:$RW/out/" packaging/installer/
rsync -a "$H:$RW/out/setup.ico" packaging/windows/setup.ico
ls -l packaging/installer packaging/windows/setup.ico
