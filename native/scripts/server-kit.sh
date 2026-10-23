#!/usr/bin/env bash
# Lays out the server kit the desktop app carries, for "Run the server on this computer" (sb-core/src/hostsetup.rs):
# the server, the agent and the installer, in the repository's own layout, so install-server.sh finds them two
# folders up from itself, plus the hub and the node binary for Add a machine when the build made them.
#   server-kit.sh <repo root> <kit dir> [hub binary] [node binary]
# On a Mac the kit goes in Switchboard.app/Contents/Resources/server-kit (scripts/remote/mac-build.sh).
# The hub and node go in <kit>/native/dist/ as switchboard-hub-<os> and switchboard-node-<os>, where install-server.sh
# looks for them. Without a hub the app runs the installer with --no-hub and says what that leaves out.
set -euo pipefail
SRC=${1:?repo root}; KIT=${2:?kit dir}; HUB=${3:-}; NODE=${4:-}
for f in server/server.py agent/sb.py native/crates/sb-core/src/term/schemes.json scripts/install/install-server.sh \
  scripts/install/setup.py scripts/install/switchboard; do
  [ -f "$SRC/$f" ] || { echo "server-kit: no $SRC/$f" >&2; exit 1; }
done
case "$(uname -s)" in Darwin) OSN=mac ;; *) OSN=linux ;; esac
rm -rf "$KIT"
mkdir -p "$KIT/agent" "$KIT/scripts/install" "$KIT/native/dist"
# The server without caches, everything else as it is in the repository.
( cd "$SRC" && tar -cf - --exclude '__pycache__' --exclude '*.pyc' server ) | ( cd "$KIT" && tar -xf - )
cp "$SRC/agent/sb.py" "$KIT/agent/sb.py"
mkdir -p "$KIT/native/crates/sb-core/src/term"
cp "$SRC/native/crates/sb-core/src/term/schemes.json" "$KIT/native/crates/sb-core/src/term/schemes.json"
for f in install-server.sh setup.py switchboard install-agent.sh install-agent.ps1; do
  if [ -f "$SRC/scripts/install/$f" ]; then cp "$SRC/scripts/install/$f" "$KIT/scripts/install/$f"; fi
done
chmod 755 "$KIT/scripts/install/install-server.sh" "$KIT/scripts/install/switchboard"
if [ -n "$HUB" ] && [ -s "$HUB" ]; then install -m 755 "$HUB" "$KIT/native/dist/switchboard-hub-$OSN"; fi
if [ -n "$NODE" ] && [ -s "$NODE" ]; then install -m 755 "$NODE" "$KIT/native/dist/switchboard-node-$OSN"; fi
echo "server kit: $KIT ($(du -sk "$KIT" | cut -f1) KB, hub: $([ -s "$KIT/native/dist/switchboard-hub-$OSN" ] && echo yes || echo no))"
