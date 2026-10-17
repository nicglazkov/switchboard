#!/usr/bin/env bash
# Build Switchboard.exe and its installer on the Windows build host (SB_WINDOWS_BUILD_HOST, see hosts.sh; MSVC) and
# bring them back.
#   native/scripts/build-windows.sh
# Output: native/dist/Switchboard-Setup-<version>.exe  (the installer with the app attached: what people download)
#         native/dist/Switchboard-windows.zip  (Switchboard.exe: GUI subsystem, Converge icon; license notices)
#         native/dist/symbols/<version>-<build>/  (.pdb files and their exes, for crash backtraces; not shipped)
# Signing: set SB_WIN_SIGN_CERT (a .pfx path ON the build host) and optionally SB_WIN_SIGN_PASSWORD,
# SB_WIN_SIGN_TIMESTAMP and SB_WIN_SIGNTOOL. Nothing is signed unless SB_WIN_SIGN_CERT is set.
set -euo pipefail
cd "$(dirname "$0")/.."
. scripts/hosts.sh
sb_hosts SB_WINDOWS_BUILD_HOST
H=$SB_WINDOWS_BUILD_HOST
# SB_REMOTE: the build folder under the build account's home, so two working copies don't share one.
REMOTE=${SB_REMOTE:-src/switchboard-native}
sb_update_env
TMP=$(mktemp -d /tmp/sb-win.XXXXXX)
trap 'rm -rf "${TMP:?}"' EXIT

# Plain files only: no AppleDouble or xattrs in the tarball.
COPYFILE_DISABLE=1 tar --no-xattrs --no-mac-metadata --exclude ./target --exclude ./dist -czf "$TMP/src.tar.gz" . -C .. branding.toml
# The build account's home as scp wants it (C:/Users/<name>); its ssh shell is PowerShell.
W=$(ssh -q $H 'Write-Output $env:USERPROFILE' | tr -d '\r' | tr '\\' /)
[ -n "$W" ] || { echo "build-windows: $H did not say its home folder" >&2; exit 1; }
D=$W/$REMOTE
ssh -q $H "New-Item -ItemType Directory -Force -Path '$(dirname "$D")' | Out-Null"
scp -q "$TMP/src.tar.gz" "$H:$D.tar.gz"
scp -q scripts/remote/windows-build.ps1 "$H:$D-build.ps1"
# ---- Signing: passed on only when set (windows-build.ps1 signs only when SB_WIN_SIGN_CERT is set) ----
SIGN=
for v in SB_WIN_SIGN_CERT SB_WIN_SIGN_PASSWORD SB_WIN_SIGN_TIMESTAMP SB_WIN_SIGNTOOL; do
  if [ -n "${!v:-}" ]; then SIGN="$SIGN \$env:$v='${!v}';"; fi
done
# ---- end of signing ----
ssh -q $H "\$env:SB_UPDATE_PUBKEY='$SB_UPDATE_PUBKEY'; \$env:SB_UPDATE_FEED='$SB_UPDATE_FEED';$SIGN powershell -NoProfile -ExecutionPolicy Bypass -File $D-build.ps1 -Root $D"
mkdir -p dist
rm -f dist/Switchboard-Setup-*.exe
scp -q "$H:$D/dist/Switchboard-Setup-*.exe" dist/
scp -q "$H:$D/dist/Switchboard-windows.zip" dist/Switchboard-windows.zip
scp -q "$H:$D/dist/Switchboard-windows.zip.build" dist/Switchboard-windows.zip.build || true
scp -q "$H:$D/dist/switchboard-node.exe" dist/switchboard-node.exe
mkdir -p dist/symbols
scp -rq "$H:$D/dist/symbols/*" dist/symbols/
chmod 644 dist/Switchboard-windows.zip dist/switchboard-node.exe dist/Switchboard-Setup-*.exe
ls -l dist/Switchboard-Setup-*.exe dist/Switchboard-windows.zip
