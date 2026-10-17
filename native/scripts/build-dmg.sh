#!/usr/bin/env bash
# Build the Mac disk image on the Mac build host (SB_MAC_BUILD_HOST, see hosts.sh) and bring it back:
# the universal app, the universal "Install Switchboard" app, an Applications shortcut, the background from
# native/packaging/installer (make-installer-art.sh) and the window layout from branding.toml [dmg].
#   native/scripts/build-dmg.sh
# Output: native/dist/Switchboard-<version>.dmg
#
# Signing: ad hoc unless SB_SIGN_IDENTITY is set. With it, both apps and the image are signed with that Developer ID,
# and with SB_NOTARY_KEY_ID, SB_NOTARY_ISSUER and SB_NOTARY_KEY_PATH (an App Store Connect API key on the build host)
# the image is also notarized and stapled. See scripts/remote/mac-dmg.sh, "Signing and notarizing".
set -euo pipefail
cd "$(dirname "$0")/.."
. scripts/hosts.sh
sb_hosts SB_MAC_BUILD_HOST
H=$SB_MAC_BUILD_HOST
REMOTE=${SB_REMOTE:-src/switchboard-dmg}  # SB_REMOTE: a separate build dir per worktree

# The repo's layout, so build.rs finds branding.toml one level above native/.
ssh -q $H "mkdir -p $REMOTE/native"
rsync -a --delete --exclude target --exclude dist ./ $H:$REMOTE/native/
rsync -a ../branding.toml $H:$REMOTE/
pass=()
for v in SB_SIGN_IDENTITY SB_NOTARY_KEY_ID SB_NOTARY_ISSUER SB_NOTARY_KEY_PATH SHADERS SB_BUILD; do
  [ -n "${!v:-}" ] && pass+=("$v=$(printf '%q' "${!v}")")
done
ssh -q $H "${pass[*]:-} bash $REMOTE/native/scripts/remote/mac-dmg.sh"
VER=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
mkdir -p dist
scp -q "$H:$REMOTE/native/dist/Switchboard-$VER.dmg" dist/
ls -l "dist/Switchboard-$VER.dmg"
