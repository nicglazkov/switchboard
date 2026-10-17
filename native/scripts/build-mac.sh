#!/usr/bin/env bash
# Build Switchboard.app (universal: Apple silicon + Intel) on the Mac build host (SB_MAC_BUILD_HOST, see hosts.sh)
# and bring the zip back.
#   native/scripts/build-mac.sh      (SHADERS=runtime|precompiled to force a mode)
# Output: native/dist/Switchboard-mac.zip  (ad hoc signed .app inside)
set -euo pipefail
cd "$(dirname "$0")/.."
. scripts/hosts.sh
sb_hosts SB_MAC_BUILD_HOST
H=$SB_MAC_BUILD_HOST
REMOTE=${SB_REMOTE:-src/switchboard-native}  # SB_REMOTE: a separate build dir per worktree

ssh -q $H "mkdir -p $REMOTE"
rsync -a --delete --exclude target --exclude dist ./ ../branding.toml $H:$REMOTE/
ssh -q $H "SHADERS=${SHADERS:-auto} bash $REMOTE/scripts/remote/mac-build.sh"
mkdir -p dist
scp -q $H:$REMOTE/dist/Switchboard-mac.zip $H:$REMOTE/dist/Switchboard-mac.zip.build dist/
scp -q $H:$REMOTE/dist/switchboard-node-mac dist/switchboard-node-mac
ls -l dist/Switchboard-mac.zip
