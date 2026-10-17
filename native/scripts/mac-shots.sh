#!/usr/bin/env bash
# Builds the `shots` variant for an Intel Mac (this one) on the Mac build host (SB_MAC_BUILD_HOST, see hosts.sh) and
# brings back a runnable bundle.
#   native/scripts/mac-shots.sh [out-dir]   (default /tmp/sb-shots)
# Then:  native/scripts/mac-shot.sh <out.png> [SB_SHOT state] [dark|light] [WxH]
set -euo pipefail
cd "$(dirname "$0")/.."
. scripts/hosts.sh
sb_hosts SB_MAC_BUILD_HOST
H=$SB_MAC_BUILD_HOST
REMOTE=${SB_REMOTE:-src/switchboard-native}  # SB_REMOTE: a separate build dir per worktree
OUT=${1:-/tmp/sb-shots}
ssh -q $H "mkdir -p $REMOTE"
rsync -a --delete --exclude target --exclude dist ./ ../branding.toml $H:$REMOTE/
ssh -q $H "export PATH=\$HOME/.cargo/bin:\$PATH; cd $REMOTE && MACOSX_DEPLOYMENT_TARGET=11.0 cargo build --locked -q --profile shots -p sb-desktop --features shots --target x86_64-apple-darwin 2>&1 | grep -E '^(error|warning: unused)' -A6 | head -60; test -x target/x86_64-apple-darwin/shots/Switchboard"
APP="$OUT/SwitchboardShots.app"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp packaging/macos/Info.plist "$APP/Contents/Info.plist"
cp packaging/macos/AppIcon.icns "$APP/Contents/Resources/AppIcon.icns"
ID=$(sed -n 's/^id = "\([^"]*\)".*/\1/p' ../branding.toml | head -n 1)  # the bundle id prefix, from branding.toml
plutil -replace CFBundleIdentifier -string "$ID.shots" "$APP/Contents/Info.plist"
plutil -replace CFBundleName -string "Switchboard Shots" "$APP/Contents/Info.plist"
scp -q $H:$REMOTE/target/x86_64-apple-darwin/shots/Switchboard "$APP/Contents/MacOS/Switchboard"
echo "built $APP"
