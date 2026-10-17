#!/usr/bin/env bash
# Runs ON the Mac build host (called by scripts/build-dmg.sh). Builds the app (mac-build.sh) and the installer, both
# universal, and packs them into dist/Switchboard-<version>.dmg. Never launches either app and never uses Finder or
# AppleScript: the window layout is written straight into the image's .DS_Store.
set -euo pipefail
export PATH="$HOME/.cargo/bin:$PATH"
cd "$(dirname "$0")/../.."  # native/
export MACOSX_DEPLOYMENT_TARGET=11.0

BRANDING=../branding.toml
[ -f "$BRANDING" ] || { echo "mac-dmg: no $BRANDING" >&2; exit 1; }
# `key = value` under [section], quotes and comments dropped.
brand() {
  awk -v s="[$1]" -v k="$2" '
    /^\[/ { in_s = ($0 == s); next }
    in_s && $1 == k { sub(/^[^=]*=[ \t]*/, ""); sub(/[ \t]*#.*$/, ""); gsub(/"/, ""); print; exit }
  ' "$BRANDING"
}
NAME=$(brand app name); ID=$(brand app id)
[ -n "$NAME" ] && [ -n "$ID" ] || { echo "mac-dmg: no [app] name/id in $BRANDING" >&2; exit 1; }
VER=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
INSTALLER="Install $NAME"

# 1. The app (dist/Switchboard.app), universal and signed ad hoc.
bash scripts/remote/mac-build.sh

# 2. The installer, universal, as "Install Switchboard.app".
cargo build --release --locked -q -p sb-install --target aarch64-apple-darwin --target x86_64-apple-darwin
I="dist/$INSTALLER.app"
rm -rf "$I"
mkdir -p "$I/Contents/MacOS" "$I/Contents/Resources"
lipo -create -output "$I/Contents/MacOS/switchboard-install" \
  target/aarch64-apple-darwin/release/switchboard-install target/x86_64-apple-darwin/release/switchboard-install
cp packaging/macos/AppIcon.icns "$I/Contents/Resources/AppIcon.icns"
cat >"$I/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleExecutable</key><string>switchboard-install</string>
  <key>CFBundleIdentifier</key><string>$ID.installer</string>
  <key>CFBundleName</key><string>$INSTALLER</string>
  <key>CFBundleDisplayName</key><string>$INSTALLER</string>
  <key>CFBundleIconFile</key><string>AppIcon</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$VER</string>
  <key>CFBundleVersion</key><string>$VER</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSSupportsAutomaticGraphicsSwitching</key><true/>
</dict>
</plist>
EOF
plutil -lint -s "$I/Contents/Info.plist"
printf 'APPL????' >"$I/Contents/PkgInfo"
codesign --force --deep -s - "$I" 2>/dev/null
codesign --verify --deep --strict "$I"
echo "installer mac: v$VER [$(lipo -archs "$I/Contents/MacOS/switchboard-install")] $(stat -f %z "$I/Contents/MacOS/switchboard-install") bytes"

# ---------------------------------------------------------------------------------------------------------------
# Signing and notarizing (apps). Skipped unless SB_SIGN_IDENTITY is set; the ad hoc signatures above stay.
# ---------------------------------------------------------------------------------------------------------------
if [ -n "${SB_SIGN_IDENTITY:-}" ]; then
  echo "signing: apps with $SB_SIGN_IDENTITY"
  for app in dist/Switchboard.app "$I"; do
    codesign --force --deep --options runtime --timestamp -s "$SB_SIGN_IDENTITY" "$app"
    codesign --verify --deep --strict "$app"
  done
else
  echo "signing: skipped (SB_SIGN_IDENTITY is not set), ad hoc only"
fi

# 3. The image's contents.
W=$(mktemp -d "${TMPDIR:-/tmp}/sb-dmg.XXXXXX")
trap 'hdiutil detach -quiet "$W/mnt" 2>/dev/null || true; rm -rf "$W"' EXIT
S="$W/src"
mkdir -p "$S/.background"
ditto dist/Switchboard.app "$S/$NAME.app"  # mac-build.sh names it Switchboard.app
ditto "$I" "$S/$INSTALLER.app"
ln -s /Applications "$S/Applications"
# One TIFF holding both sizes, so the background is sharp on Retina screens.
tiffutil -cathidpicheck packaging/installer/dmg-background.png packaging/installer/dmg-background@2x.png \
  -out "$S/.background/background.tiff" >/dev/null 2>&1
cp packaging/macos/AppIcon.icns "$S/.VolumeIcon.icns"

# 4. A writable image to lay out, then the compressed one.
MB=$(( $(du -sm "$S" | cut -f1) + 20 ))
hdiutil create -quiet -ov -srcfolder "$S" -volname "$NAME" -fs HFS+ -format UDRW -size "${MB}m" "$W/rw.dmg"
mkdir -p "$W/mnt"
hdiutil attach -quiet -nobrowse -noautoopen -noverify -mountpoint "$W/mnt" "$W/rw.dmg"
if command -v SetFile >/dev/null; then
  SetFile -a C "$W/mnt"  # the volume uses .VolumeIcon.icns
else
  echo "mac-dmg: SetFile not found (Xcode command line tools), so the volume keeps the plain disk icon" >&2
fi

# The Finder window: written with ds_store and mac_alias (the libraries dmgbuild uses) in a throwaway venv.
python3 -m venv "$W/venv"
"$W/venv/bin/pip" install -q --disable-pip-version-check ds_store==1.3.1 mac_alias==2.2.2
dmg() { brand dmg "$1"; }
"$W/venv/bin/python" - "$W/mnt" "$NAME" "$INSTALLER" \
  "$(dmg window_width)" "$(dmg window_height)" "$(dmg icon_size)" \
  "$(dmg app_x)" "$(dmg app_y)" "$(dmg applications_x)" "$(dmg applications_y)" \
  "$(dmg installer_x)" "$(dmg installer_y)" <<'PY'
import os, sys
from ds_store import DSStore
from mac_alias import Alias

mnt, name, installer = sys.argv[1:4]
w, h, icon, ax, ay, px_, py_, ix, iy = (int(v) for v in sys.argv[4:13])
bg = Alias.for_file(os.path.join(mnt, ".background", "background.tiff")).to_bytes()
with DSStore.open(os.path.join(mnt, ".DS_Store"), "w+") as d:
    d["."]["vSrn"] = ("long", 1)
    d["."]["bwsp"] = {
        "WindowBounds": "{{200, 120}, {%d, %d}}" % (w, h),
        "ShowStatusBar": False, "ShowToolbar": False, "ShowTabView": False, "ShowPathbar": False,
        "ShowSidebar": False, "ContainerShowSidebar": False, "PreviewPaneVisibility": False, "SidebarWidth": 0,
    }
    d["."]["icvp"] = {
        "viewOptionsVersion": 1, "backgroundType": 2, "backgroundImageAlias": bg,
        "backgroundColorRed": 1.0, "backgroundColorGreen": 1.0, "backgroundColorBlue": 1.0,
        "iconSize": float(icon), "textSize": 13.0, "labelOnBottom": True, "arrangeBy": "none",
        "gridOffsetX": 0.0, "gridOffsetY": 0.0, "gridSpacing": 100.0,
        "showIconPreview": True, "showItemInfo": False, "scrollPositionX": 0.0, "scrollPositionY": 0.0,
    }
    d["."]["icvl"] = ("type", "icnv")
    d[name + ".app"]["Iloc"] = (ax, ay)
    d["Applications"]["Iloc"] = (px_, py_)
    d[installer + ".app"]["Iloc"] = (ix, iy)
print("dmg layout: %dx%d, icons %d" % (w, h, icon))
PY
rm -rf "$W/mnt/.fseventsd"
sync
hdiutil detach -quiet "$W/mnt"
OUT="dist/Switchboard-$VER.dmg"
rm -f "$OUT"
hdiutil convert -quiet "$W/rw.dmg" -format UDZO -imagekey zlib-level=9 -o "$OUT"

# ---------------------------------------------------------------------------------------------------------------
# Signing and notarizing (image). Skipped unless SB_SIGN_IDENTITY is set. Notarizing also needs either
# SB_NOTARY_PROFILE (a profile saved on this Mac with `xcrun notarytool store-credentials`, so no key or id is in any
# file) or SB_NOTARY_KEY_ID, SB_NOTARY_ISSUER and SB_NOTARY_KEY_PATH (an App Store Connect API key file on this Mac).
# ---------------------------------------------------------------------------------------------------------------
if [ -n "${SB_SIGN_IDENTITY:-}" ]; then
  codesign --force --timestamp -s "$SB_SIGN_IDENTITY" "$OUT"
  auth=()
  if [ -n "${SB_NOTARY_PROFILE:-}" ]; then
    auth=(--keychain-profile "$SB_NOTARY_PROFILE")
  elif [ -n "${SB_NOTARY_KEY_ID:-}" ] && [ -n "${SB_NOTARY_ISSUER:-}" ] && [ -n "${SB_NOTARY_KEY_PATH:-}" ]; then
    auth=(--key "$SB_NOTARY_KEY_PATH" --key-id "$SB_NOTARY_KEY_ID" --issuer "$SB_NOTARY_ISSUER")
  fi
  if [ ${#auth[@]} -gt 0 ]; then
    echo "notarizing: $OUT"
    log=$(xcrun notarytool submit "$OUT" "${auth[@]}" --wait 2>&1) || true
    echo "$log"
    # notarytool can exit 0 for a rejected image, so read its answer.
    if ! grep -q "status: Accepted" <<<"$log"; then
      id=$(sed -n 's/^ *id: //p' <<<"$log" | head -1)
      [ -n "$id" ] && xcrun notarytool log "$id" "${auth[@]}" >&2 || true
      echo "notarizing: not accepted" >&2
      exit 1
    fi
    xcrun stapler staple "$OUT"
    xcrun stapler validate "$OUT"
  else
    echo "notarizing: skipped (set SB_NOTARY_PROFILE, or SB_NOTARY_KEY_ID, SB_NOTARY_ISSUER and SB_NOTARY_KEY_PATH)"
  fi
fi
hdiutil verify -quiet "$OUT"
echo "dmg: $OUT $(stat -f %z "$OUT") bytes"
