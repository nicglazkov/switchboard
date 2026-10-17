#!/usr/bin/env bash
# Publish the built apps and the download page to the publish host (SB_PUBLISH_HOST, see hosts.sh).
#
#   native/scripts/publish.sh                     # to ~/switchboard-lab/app, served at SB_PUBLISH_BASE
#   native/scripts/publish.sh --dest switchboard-pagestage   # a staging folder
#
# Reads native/dist/:
#   Switchboard-mac.zip              (from build-mac.sh)
#   Switchboard-windows.zip          (from build-windows.sh)
#   Switchboard-linux-x86_64         (from build-linux.sh)
#   Switchboard-linux-x86_64.tar.gz  (optional)
#   Switchboard-android.apk          (optional; android/dist/, else the copy on the publish
#                                     host in ~/switchboard-android from android/scripts/build.sh)
#
# Runs nothing heavy here: it uploads, and the publish host hashes the files, renders the page and
# swaps the folder into place (scripts/publish_render.py).
#
# Options:
#   --dest DIR         remote folder, relative to the remote home (default switchboard-lab/app)
#   --base URL         URL the install commands download from (default SB_PUBLISH_BASE)
#   --host USER@HOST   default SB_PUBLISH_HOST
#   --allow-missing    publish whichever builds exist; the page marks the rest as not published
#   --dist DIR         read the builds from DIR instead of native/dist (for testing)
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
NATIVE="$(cd "$HERE/.." && pwd)"
DIST="$NATIVE/dist"
H=""
DEST=switchboard-lab/app
BASE=""
ALLOW=""

while [ $# -gt 0 ]; do
  case "$1" in
    --dest) DEST="${2:?--dest needs a folder}"; shift 2 ;;
    --base) BASE="${2:?--base needs a URL}"; shift 2 ;;
    --host) H="${2:?--host needs user@host}"; shift 2 ;;
    --allow-missing) ALLOW=--allow-missing; shift ;;
    --dist) DIST="$(cd "${2:?--dist needs a folder}" && pwd)"; shift 2 ;;
    -h|--help) sed -n '2,23p' "$0"; exit 0 ;;
    *) echo "publish: unknown option $1 (see --help)" >&2; exit 2 ;;
  esac
done

. "$HERE/hosts.sh"
[ -n "$H" ] || { sb_hosts SB_PUBLISH_HOST; H=$SB_PUBLISH_HOST; }
[ -n "$BASE" ] || { sb_hosts SB_PUBLISH_BASE; BASE=$SB_PUBLISH_BASE; }
BASE="${BASE%/}"

DEST="${DEST#\~/}"
DEST="${DEST%/}"
case "$DEST" in
  ""|/*|*..*|*[!A-Za-z0-9._/-]*) echo "publish: --dest must be a plain folder under the remote home, got '$DEST'" >&2; exit 2 ;;
esac
STAGE="$DEST.incoming"

VERSION="$(sed -n 's/^version = "\([^"]*\)".*/\1/p' "$NATIVE/Cargo.toml" | head -n 1)"
[ -n "$VERSION" ] || { echo "publish: no version in $NATIVE/Cargo.toml" >&2; exit 1; }

FILES=()
MISSING=()
for f in Switchboard-mac.zip Switchboard-windows.zip Switchboard-linux-x86_64; do
  if [ -f "$DIST/$f" ]; then FILES+=("$DIST/$f"); else MISSING+=("$f"); fi
done
[ -f "$DIST/Switchboard-linux-x86_64.tar.gz" ] && FILES+=("$DIST/Switchboard-linux-x86_64.tar.gz")
# Each build's id (SB_BUILD, written next to it by the build scripts) goes into latest.json for "Restart to update".
for b in "$DIST"/Switchboard-mac.zip.build "$DIST"/Switchboard-windows.zip.build "$DIST"/Switchboard-linux-x86_64.build; do
  [ -f "$b" ] && FILES+=("$b")
done
APK="$NATIVE/../android/dist/Switchboard-android.apk"
[ -f "$APK" ] && FILES+=("$APK")

if [ ${#MISSING[@]} -gt 0 ]; then
  if [ -z "$ALLOW" ]; then
    echo "publish: not built yet: ${MISSING[*]}" >&2
    echo "  build them with native/scripts/build-mac.sh, build-windows.sh and build-linux.sh," >&2
    echo "  or pass --allow-missing to publish the rest." >&2
    exit 1
  fi
  echo "publish: skipping ${MISSING[*]}"
fi
[ ${#FILES[@]} -gt 0 ] || { echo "publish: nothing in $DIST to publish" >&2; exit 1; }

echo "publish: Switchboard $VERSION to $H:~/$DEST"
ssh "$H" "mkdir -p '$STAGE'"
# Page assets first (with --delete, so the staging folder starts clean), then the apps.
rsync -a --delete --exclude '.DS_Store' "$HERE/download/" "$H:$STAGE/"
rsync -a --progress "${FILES[@]}" "$H:$STAGE/"
[ -f "$APK" ] || ssh "$H" "[ ! -f ~/switchboard-android/Switchboard-android.apk ] || cp ~/switchboard-android/Switchboard-android.apk '$STAGE/'"
ssh "$H" "cd ~ && python3 - --dest '$DEST' --base '$BASE' --version '$VERSION' $ALLOW" < "$HERE/publish_render.py"

# Tell the Switchboard server where the page is, so the desktop apps can offer the newest build.
ssh "$H" "mkdir -p ~/.config/switchboard && printf '{\"url\": \"%s\"}\n' '$BASE' > ~/.config/switchboard/downloads.json"

if [ "$DEST" = switchboard-lab/app ]; then
  code="$(ssh "$H" "curl -s -o /dev/null -w '%{http_code}' '$BASE/'")"
  echo "publish: $BASE/ answers $code"
fi
