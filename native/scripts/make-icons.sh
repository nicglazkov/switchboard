#!/usr/bin/env bash
# Regenerate the app icons from the Converge brand kit's masters in native/packaging/brand. Runs nothing heavy here;
# it only ships files.
#   native/packaging/windows/app.ico   16 24 32 48 64 128 256 (16 and 32 are the kit's hand-tuned masters)
#   native/packaging/macos/AppIcon.icns all iconset sizes incl. @2x, from icon-macos.svg
#   native/packaging/linux/switchboard.png 512, from icon-square.svg
# Rendering (rsvg-convert, Pillow) happens on SB_LINUX_HOST; iconutil runs on SB_MAC_BUILD_HOST (see hosts.sh).
set -euo pipefail
cd "$(dirname "$0")/.."
NATIVE=$PWD
KIT=$NATIVE/packaging/brand
. scripts/hosts.sh
sb_hosts SB_LINUX_HOST SB_MAC_BUILD_HOST
LINUX=$SB_LINUX_HOST
MAC=$SB_MAC_BUILD_HOST
RW='switchboard-native-icons'          # fixed remote work dir (relative to remote home)

[ -f "$KIT/icon-macos.svg" ] || { echo "brand kit not found at $KIT" >&2; exit 1; }

ssh -q $LINUX "rm -rf ~/switchboard-native-icons && mkdir -p ~/$RW/in ~/$RW/ico ~/$RW/AppIcon.iconset"
scp -q "$KIT/icon-macos.svg" "$KIT/icon-square.svg" "$KIT/app-16.png" "$KIT/app-32.png" \
  "$NATIVE/scripts/make-ico.py" $LINUX:$RW/in/

ssh -q $LINUX "set -e; cd ~/$RW
command -v rsvg-convert >/dev/null || { echo 'install librsvg2-bin on the Linux host' >&2; exit 1; }
for s in 24 48 64 128 256; do rsvg-convert -w \$s -h \$s in/icon-square.svg -o ico/\$s.png; done
cp in/app-16.png ico/16.png; cp in/app-32.png ico/32.png
rsvg-convert -w 512 -h 512 in/icon-square.svg -o switchboard.png
python3 in/make-ico.py app.ico ico/16.png ico/24.png ico/32.png ico/48.png ico/64.png ico/128.png ico/256.png
r() { rsvg-convert -w \$1 -h \$1 in/icon-macos.svg -o AppIcon.iconset/\$2; }
r 16 icon_16x16.png; r 32 icon_16x16@2x.png; r 32 icon_32x32.png; r 64 icon_32x32@2x.png
r 128 icon_128x128.png; r 256 icon_128x128@2x.png; r 256 icon_256x256.png; r 512 icon_256x256@2x.png
r 512 icon_512x512.png; r 1024 icon_512x512@2x.png"

mkdir -p "$NATIVE/packaging/windows" "$NATIVE/packaging/macos" "$NATIVE/packaging/linux"
scp -q $LINUX:$RW/app.ico "$NATIVE/packaging/windows/app.ico"
scp -q $LINUX:$RW/switchboard.png "$NATIVE/packaging/linux/switchboard.png"

# iconutil (on the Mac) turns the iconset into a proper .icns.
ssh -q $MAC "rm -rf ~/switchboard-native-icons && mkdir -p ~/$RW"
ssh -q $LINUX "cd ~/$RW && tar -cf - AppIcon.iconset" | ssh -q $MAC "cd ~/$RW && tar -xf - && iconutil -c icns AppIcon.iconset -o AppIcon.icns"
scp -q $MAC:$RW/AppIcon.icns "$NATIVE/packaging/macos/AppIcon.icns"
ssh -q $MAC "rm -rf ~/switchboard-native-icons"
ssh -q $LINUX "rm -rf ~/switchboard-native-icons"

ls -l "$NATIVE/packaging/windows/app.ico" "$NATIVE/packaging/macos/AppIcon.icns" "$NATIVE/packaging/linux/switchboard.png"
