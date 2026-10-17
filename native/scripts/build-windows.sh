#!/usr/bin/env bash
# Build Switchboard.exe on the Windows build host (SB_WINDOWS_BUILD_HOST, see hosts.sh; MSVC) and bring the zip back.
#   native/scripts/build-windows.sh
# Output: native/dist/Switchboard-windows.zip  (Switchboard.exe: GUI subsystem, Converge icon)
set -euo pipefail
cd "$(dirname "$0")/.."
. scripts/hosts.sh
sb_hosts SB_WINDOWS_BUILD_HOST
H=$SB_WINDOWS_BUILD_HOST
TMP=$(mktemp -d /tmp/sb-win.XXXXXX)
trap 'rm -rf "${TMP:?}"' EXIT

# Plain files only: no AppleDouble or xattrs in the tarball.
COPYFILE_DISABLE=1 tar --no-xattrs --no-mac-metadata --exclude ./target --exclude ./dist -czf "$TMP/src.tar.gz" .
# The build account's home as scp wants it (C:/Users/<name>); its ssh shell is PowerShell.
W=$(ssh -q $H 'New-Item -ItemType Directory -Force -Path $env:USERPROFILE\src | Out-Null; Write-Output $env:USERPROFILE' | tr -d '\r' | tr '\\' /)
[ -n "$W" ] || { echo "build-windows: $H did not say its home folder" >&2; exit 1; }
scp -q "$TMP/src.tar.gz" "$H:$W/src/switchboard-native.tar.gz"
scp -q scripts/remote/windows-build.ps1 "$H:$W/src/switchboard-native-build.ps1"
ssh -q $H "powershell -NoProfile -ExecutionPolicy Bypass -File $W/src/switchboard-native-build.ps1"
mkdir -p dist
scp -q "$H:$W/src/switchboard-native/dist/Switchboard-windows.zip" dist/Switchboard-windows.zip
scp -q "$H:$W/src/switchboard-native/dist/Switchboard-windows.zip.build" dist/Switchboard-windows.zip.build || true
scp -q "$H:$W/src/switchboard-native/dist/switchboard-node.exe" dist/switchboard-node.exe
chmod 644 dist/Switchboard-windows.zip dist/switchboard-node.exe
ls -l dist/Switchboard-windows.zip
