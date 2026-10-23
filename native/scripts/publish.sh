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
# Here it makes latest.json (each build's size, SHA-256 and build id) and signs it with the update key, so the
# key never leaves this computer. Then it uploads, and the publish host checks latest.json against the files,
# renders the page and swaps the folder into place (scripts/publish_render.py).
#
# The update key: SB_UPDATE_KEY_FILE names the Ed25519 private key file (native/scripts/update-key.sh makes
# one), from the environment or the hosts file. It is never in the repo. When SB_UPDATE_PUBKEY is set too, a key
# that does not match it is refused. OpenSSL 3 signs; SB_OPENSSL names another openssl.
#
# Options:
#   --dest DIR         remote folder, relative to the remote home (default switchboard-lab/app)
#   --base URL         URL the install commands download from (default SB_PUBLISH_BASE)
#   --host USER@HOST   default SB_PUBLISH_HOST
#   --allow-missing    publish whichever builds exist; the page marks the rest as not published
#   --dist DIR         read the builds from DIR instead of native/dist (for testing)
#   --unsigned         publish without signing; no app offers these builds as an update
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
NATIVE="$(cd "$HERE/.." && pwd)"
DIST="$NATIVE/dist"
H=""
DEST=switchboard-lab/app
BASE=""
ALLOW=""
UNSIGNED=""

while [ $# -gt 0 ]; do
  case "$1" in
    --dest) DEST="${2:?--dest needs a folder}"; shift 2 ;;
    --base) BASE="${2:?--base needs a URL}"; shift 2 ;;
    --host) H="${2:?--host needs user@host}"; shift 2 ;;
    --allow-missing) ALLOW=--allow-missing; shift ;;
    --dist) DIST="$(cd "${2:?--dist needs a folder}" && pwd)"; shift 2 ;;
    --unsigned) UNSIGNED=1; shift ;;
    -h|--help) sed -n '2,29p' "$0"; exit 0 ;;
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

# latest.json and its signature, made here. Each build's id (SB_BUILD, written next to it by the build scripts)
# goes into it, and the apps offer only a newer version and build.
KEY=""
if [ -z "$UNSIGNED" ]; then
  sb_host_opt SB_UPDATE_KEY_FILE || {
    echo "publish: SB_UPDATE_KEY_FILE is not set, so the update list can't be signed." >&2
    echo "  make a key with native/scripts/update-key.sh and point SB_UPDATE_KEY_FILE at it," >&2
    echo "  or pass --unsigned to publish without updates (see docs/building.md)." >&2
    exit 1
  }
  KEY=$SB_UPDATE_KEY_FILE
  [ -f "$KEY" ] || { echo "publish: SB_UPDATE_KEY_FILE names $KEY, which is not there" >&2; exit 1; }
fi
SIGNED="$(mktemp -d)"
trap 'rm -rf "${SIGNED:?}"' EXIT
KIT="$NATIVE/../dist/server-kit/switchboard-server-kit.tar.gz"  # scripts/install/pack-server-kit.sh
KIT_ARG=()
[ -f "$KIT" ] && KIT_ARG=(--kit "$KIT")
python3 "$HERE/publish_render.py" manifest --dist "$DIST" --version "$VERSION" --out "$SIGNED/latest.json" \
  ${KIT_ARG[@]+"${KIT_ARG[@]}"}
FILES+=("$SIGNED/latest.json")
if [ -n "$KEY" ]; then
  sb_host_opt SB_UPDATE_PUBKEY || true
  python3 "$HERE/publish_render.py" sign --key "$KEY" --in "$SIGNED/latest.json" --out "$SIGNED/latest.json.sig" \
    ${SB_UPDATE_PUBKEY:+--expect "$SB_UPDATE_PUBKEY"}
  FILES+=("$SIGNED/latest.json.sig")
  echo "publish: latest.json signed"
else
  echo "publish: --unsigned: no app will offer these builds as an update"
fi

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
  # Add a machine downloads the node and the agent from the folder beside the page: the same release, or a
  # machine added later gets whatever was there last (a 0.1.0 node and agent once, which could not link).
  if [ "$DIST" = "$NATIVE/dist" ] && [ -s "$DIST/switchboard-node-mac" ] && [ -s "$DIST/switchboard-node-linux" ] && [ -s "$DIST/switchboard-node.exe" ]; then
    SB_HOST="$H" bash "$HERE/publish-node.sh"
  else
    echo "publish: the node was not published (build it on all three to publish it with the apps)"
  fi
fi
