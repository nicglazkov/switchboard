#!/usr/bin/env bash
# Publish switchboard-node and the agent for Add a machine: the pasted line downloads them from SB_NODE_BASE
# (see hosts.sh), where the lab server on the publish host (SB_PUBLISH_HOST) serves ~/switchboard-lab/node.
#
#   native/scripts/publish-node.sh      # after build-mac.sh, build-linux.sh and build-windows.sh
#
# Reads native/dist/switchboard-node-mac, switchboard-node-linux,
# switchboard-node.exe, and agent/sb.py. The folder is swapped into place in one step.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
NATIVE="$(cd "$HERE/.." && pwd)"
REPO="$(cd "$NATIVE/.." && pwd)"
. "$HERE/hosts.sh"
if [ -n "${SB_HOST:-}" ]; then H=$SB_HOST; else sb_hosts SB_PUBLISH_HOST; H=$SB_PUBLISH_HOST; fi
sb_host_opt SB_NODE_BASE || { sb_hosts SB_PUBLISH_BASE; SB_NODE_BASE=${SB_PUBLISH_BASE%/*}/node; }
DIST="$NATIVE/dist"
for f in switchboard-node-mac switchboard-node-linux switchboard-node.exe; do
  [ -s "$DIST/$f" ] || { echo "missing $DIST/$f (run the build scripts first)" >&2; exit 1; }
done
ssh -q "$H" 'rm -rf ~/switchboard-lab/node.new && mkdir -p ~/switchboard-lab/node.new'
scp -q "$DIST/switchboard-node-mac" "$DIST/switchboard-node-linux" "$DIST/switchboard-node.exe" "$REPO/agent/sb.py" "$H:switchboard-lab/node.new/"
ssh -q "$H" 'cd ~/switchboard-lab && chmod 644 node.new/* && (cd node.new && sha256sum * > SHA256SUMS) &&
  rm -rf node.old && { [ ! -d node ] || mv node node.old; } && mv node.new node && rm -rf node.old && ls -l node'
for f in switchboard-node-mac switchboard-node-linux switchboard-node.exe sb.py; do
  curl -s -o /dev/null -w "$f %{http_code} %{size_download}\n" "${SB_NODE_BASE%/}/$f"
done
