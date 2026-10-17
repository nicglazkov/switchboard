#!/usr/bin/env bash
# Build the Mac disk image on the Mac build host (SB_MAC_BUILD_HOST, see hosts.sh) and bring it back:
# the universal app, the universal "Install Switchboard" app, an Applications shortcut, the background from
# native/packaging/installer (make-installer-art.sh) and the window layout from branding.toml [dmg].
#   native/scripts/build-dmg.sh
# Output: native/dist/Switchboard-<version>.dmg
#
# Signing: ad hoc unless SB_SIGN_IDENTITY is set. With it, both apps and the image are signed with that Developer ID,
# and with SB_NOTARY_PROFILE (a notarytool profile saved on the build host) or SB_NOTARY_KEY_ID, SB_NOTARY_ISSUER and
# SB_NOTARY_KEY_PATH (an App Store Connect API key there) the image is also notarized and stapled. See scripts/remote/mac-dmg.sh, "Signing and notarizing".
set -euo pipefail
cd "$(dirname "$0")/.."
. scripts/hosts.sh
sb_hosts SB_MAC_BUILD_HOST
sb_update_env
# Signing settings may also live in the hosts file; the environment still wins.
for v in SB_SIGN_IDENTITY SB_NOTARY_PROFILE; do sb_host_opt $v || true; done
H=$SB_MAC_BUILD_HOST
REMOTE=${SB_REMOTE:-src/switchboard-dmg}  # SB_REMOTE: a separate build dir per worktree

# The repo's layout, so build.rs finds branding.toml one level above native/.
ssh -q $H "mkdir -p $REMOTE/native"
rsync -a --delete --exclude target --exclude dist ./ $H:$REMOTE/native/
rsync -a ../branding.toml $H:$REMOTE/
pass=()
for v in SB_SIGN_IDENTITY SB_NOTARY_PROFILE SB_NOTARY_KEY_ID SB_NOTARY_ISSUER SB_NOTARY_KEY_PATH SHADERS SB_BUILD \
  SB_UPDATE_PUBKEY SB_UPDATE_FEED; do
  [ -n "${!v:-}" ] && pass+=("$v=$(printf '%q' "${!v}")")
done
if [ -z "${SB_SIGN_IDENTITY:-}" ]; then
  ssh -q $H "${pass[*]:-} bash $REMOTE/native/scripts/remote/mac-dmg.sh"
else
  # An ssh session can't use the login keychain, so the signed build runs in the session of whoever is logged in on
  # the build Mac, started with `open`. The first signing there may show a dialog asking to let codesign use the
  # key; "Always Allow" answers it for later builds.
  LOG=$REMOTE/sign.log
  { printf '#!/bin/bash\ncd ~\n'
    for kv in "${pass[@]}"; do printf 'export %s\n' "$kv"; done
    printf '{ bash %s/native/scripts/remote/mac-dmg.sh; echo "SB_EXIT=$?"; } > %s 2>&1\n' "$REMOTE" "$LOG"
  } | ssh -q $H "umask 077; rm -f $LOG; cat > $REMOTE/sign.command; chmod +x $REMOTE/sign.command; open -g $REMOTE/sign.command"
  echo "signing: running in the logged-in session on $H (answer any keychain dialog there)"
  until ssh -q $H "grep -q '^SB_EXIT=' $LOG 2>/dev/null"; do sleep 15; done
  ssh -q $H "cat $LOG; rm -f $REMOTE/sign.command"
  ssh -q $H "grep -q '^SB_EXIT=0' $LOG" || { echo "build-dmg: the signed build failed (log above)" >&2; exit 1; }
fi
VER=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
mkdir -p dist
scp -q "$H:$REMOTE/native/dist/Switchboard-$VER.dmg" dist/
ls -l "dist/Switchboard-$VER.dmg"
