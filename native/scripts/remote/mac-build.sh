#!/usr/bin/env bash
# Runs ON the Mac build host (called by scripts/build-mac.sh). Builds a universal Switchboard.app, signs it
# ad hoc and zips it to dist/Switchboard-mac.zip. Never launches the app. The debug symbols (one .dSYM per arch)
# go to dist/symbols/<version>-<build>/, outside the zip.
#   SHADERS=auto (default) | precompiled | runtime
#     precompiled needs Xcode's Metal Toolchain (`xcrun metal`); runtime compiles the Metal shaders
#     from source at first launch (GPUI's runtime_shaders feature). auto picks precompiled when it can.
set -euo pipefail
export PATH="$HOME/.cargo/bin:$PATH"
cd "$(dirname "$0")/../.."  # the native/ copy build-mac.sh synced (SB_REMOTE picks which)

export MACOSX_DEPLOYMENT_TARGET=11.0
TARGETS=(aarch64-apple-darwin x86_64-apple-darwin)
installed=$(rustup target list --installed)
for t in "${TARGETS[@]}"; do
  grep -qx "$t" <<<"$installed" || rustup target add "$t" >/dev/null
done

SHADERS=${SHADERS:-auto}
if [ "$SHADERS" = auto ]; then
  if xcrun metal --version >/dev/null 2>&1; then SHADERS=precompiled; else SHADERS=runtime; fi
fi
FEATURES=()
[ "$SHADERS" = runtime ] && FEATURES=(--features gpui_platform/runtime_shaders)

# The build id the app compares with the download page's latest.json ("Restart to update").
export SB_BUILD="${SB_BUILD:-$(date -u +%Y%m%d%H%M%S)}"
start=$(date +%s)
cargo build --release --locked -q -p sb-desktop --target aarch64-apple-darwin --target x86_64-apple-darwin ${FEATURES[@]+"${FEATURES[@]}"}  # bash 3.2: an empty array trips set -u
secs=$(( $(date +%s) - start ))

VER=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
mkdir -p dist
rm -rf dist/Switchboard.app dist/Switchboard-mac.zip
mkdir -p dist/Switchboard.app/Contents/MacOS dist/Switchboard.app/Contents/Resources
lipo -create -output dist/Switchboard.app/Contents/MacOS/Switchboard \
  target/aarch64-apple-darwin/release/Switchboard target/x86_64-apple-darwin/release/Switchboard
cp packaging/macos/Info.plist dist/Switchboard.app/Contents/Info.plist
cp packaging/macos/AppIcon.icns dist/Switchboard.app/Contents/Resources/AppIcon.icns
cp packaging/THIRD-PARTY-NOTICES.txt dist/Switchboard.app/Contents/Resources/THIRD-PARTY-NOTICES.txt
# Symbols for reading crash backtraces (docs/building.md). Only this build's are kept here; build-mac.sh copies them.
# cp -L: cargo's release/*.dSYM are symlinks into deps/.
SYM=dist/symbols/$VER-$SB_BUILD
rm -rf dist/symbols
mkdir -p "$SYM"
for t in "${TARGETS[@]}"; do
  cp -RL "target/$t/release/Switchboard.dSYM" "$SYM/Switchboard-$t.dSYM"
done
# Bundle id and name come from branding.toml (build-mac.sh copies it into this folder); the plist only
# holds placeholders.
BRANDING=branding.toml; [ -f "$BRANDING" ] || BRANDING=../branding.toml
brand() { sed -n "s/^$1 = \"\([^\"]*\)\".*/\1/p" "$BRANDING" | head -1; }
APP_NAME=$(brand name); DESKTOP_ID=$(brand desktop_id)
[ -n "$APP_NAME" ] && [ -n "$DESKTOP_ID" ] || { echo "mac-build: no name/desktop_id in $BRANDING" >&2; exit 1; }
plutil -replace CFBundleIdentifier -string "$DESKTOP_ID" dist/Switchboard.app/Contents/Info.plist
plutil -replace CFBundleName -string "$APP_NAME" dist/Switchboard.app/Contents/Info.plist
plutil -replace CFBundleDisplayName -string "$APP_NAME" dist/Switchboard.app/Contents/Info.plist
plutil -replace CFBundleShortVersionString -string "$VER" dist/Switchboard.app/Contents/Info.plist
plutil -replace CFBundleVersion -string "$VER" dist/Switchboard.app/Contents/Info.plist
plutil -lint -s dist/Switchboard.app/Contents/Info.plist
printf 'APPL????' > dist/Switchboard.app/Contents/PkgInfo

codesign --force --deep -s - dist/Switchboard.app 2>/dev/null
codesign --verify --deep --strict dist/Switchboard.app
ditto -c -k --keepParent dist/Switchboard.app dist/Switchboard-mac.zip
echo "$SB_BUILD" > dist/Switchboard-mac.zip.build

archs=$(lipo -archs dist/Switchboard.app/Contents/MacOS/Switchboard)
bin=$(stat -f %z dist/Switchboard.app/Contents/MacOS/Switchboard)
zip=$(stat -f %z dist/Switchboard-mac.zip)
echo "mac: v$VER [$archs] shaders=$SHADERS build=${secs}s binary=$bin bytes zip=$zip bytes"

# switchboard-node for Add a machine: one universal binary, signed ad hoc like the app.
cargo build --release --locked -q -p sb-node --target aarch64-apple-darwin --target x86_64-apple-darwin
lipo -create -output dist/switchboard-node-mac target/aarch64-apple-darwin/release/switchboard-node target/x86_64-apple-darwin/release/switchboard-node
codesign --force -s - dist/switchboard-node-mac
for t in "${TARGETS[@]}"; do
  cp -RL "target/$t/release/switchboard-node.dSYM" "$SYM/switchboard-node-$t.dSYM"
done
echo "symbols: $SYM ($(du -sk "$SYM" | cut -f1) KB)"
echo "node mac: [$(lipo -archs dist/switchboard-node-mac)] $(stat -f %z dist/switchboard-node-mac) bytes"
