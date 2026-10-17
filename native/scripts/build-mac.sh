#!/usr/bin/env bash
# Build Switchboard.app (universal: Apple silicon + Intel) on the Mac build host (SB_MAC_BUILD_HOST, see hosts.sh)
# and bring the zip back.
#   native/scripts/build-mac.sh      (SHADERS=runtime|precompiled to force a mode)
# Output: native/dist/Switchboard-mac.zip  (ad hoc signed .app inside)
#         native/dist/symbols/<version>-<build>/  (.dSYM files for crash backtraces, not shipped)
set -euo pipefail
cd "$(dirname "$0")/.."
. scripts/hosts.sh
sb_hosts SB_MAC_BUILD_HOST
H=$SB_MAC_BUILD_HOST
sb_update_env
REMOTE=${SB_REMOTE:-src/switchboard-native}  # SB_REMOTE: a separate build dir per worktree

ssh -q $H "mkdir -p $REMOTE"
rsync -a --delete --exclude target --exclude dist ./ ../branding.toml $H:$REMOTE/
ssh -q $H "SHADERS=${SHADERS:-auto} SB_UPDATE_PUBKEY='$SB_UPDATE_PUBKEY' SB_UPDATE_FEED='$SB_UPDATE_FEED' bash $REMOTE/scripts/remote/mac-build.sh"
mkdir -p dist
scp -q $H:$REMOTE/dist/Switchboard-mac.zip $H:$REMOTE/dist/Switchboard-mac.zip.build dist/
scp -q $H:$REMOTE/dist/switchboard-node-mac dist/switchboard-node-mac
rsync -a $H:$REMOTE/dist/symbols/ dist/symbols/
ls -l dist/Switchboard-mac.zip
