#!/usr/bin/env bash
# Lays out the server kit the desktop app carries, for "Run the server on this computer" (sb-core/src/hostsetup.rs):
# the server, the agent and the installer, in the repository's own layout, so install-server.sh finds them two
# folders up from itself, plus the hub and the node binary for Add a machine when the build made them.
#   server-kit.sh <repo root> <kit dir> [hub binary] [node binary]
# On a Mac the kit goes in Switchboard.app/Contents/Resources/server-kit (scripts/remote/mac-build.sh).
# The hub and node go in <kit>/native/dist/ as switchboard-hub-<os> and switchboard-node-<os>, where install-server.sh
# looks for them. Without a hub the app runs the installer with --no-hub and says what that leaves out.
# Switchboard's own Python goes in when <repo root>/python has it (scripts/install/fetch-python.sh put it there; the
# build scripts do that with SB_KIT_PYTHON=1): only the builds for this kind of computer. On a Mac each is laid out
# as python/<kind>/, not left as an archive, so that every program in the app can be signed (sign-python.sh): a
# program inside an archive cannot be, and Apple's notary reads archives. Elsewhere the archive goes in as it is.
# Without it, install-server.sh uses the computer's own Python, or downloads the pinned build when there is none.
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
sha() { if command -v sha256sum >/dev/null; then sha256sum "$1" | cut -d ' ' -f 1; else shasum -a 256 "$1" | cut -d ' ' -f 1; fi; }
PYS=""
if [ -f "$SRC/python/SHA256SUMS" ]; then
  for a in "$SRC"/python/python-"$OSN"-*.tar.gz; do
    [ -f "$a" ] || continue
    n=$(basename "$a"); k=${n#python-}; k=${k%.tar.gz}
    pin=$(awk -v n="$n" '$2 == n { print $1; exit }' "$SRC/python/SHA256SUMS")
    [ -n "$pin" ] && [ "$(sha "$a")" = "$pin" ] || { echo "server-kit: $n does not match its line in python/SHA256SUMS" >&2; exit 1; }
    mkdir -p "$KIT/python"
    if [ "$OSN" = mac ]; then
      mkdir -p "$KIT/python/$k"
      tar -xzf "$a" -C "$KIT/python/$k" --strip-components 1
      printf '%s\n' "$pin" > "$KIT/python/$k/.pin"
    else
      cp "$a" "$KIT/python/$n"
      printf '%s  %s\n' "$pin" "$n" >> "$KIT/python/SHA256SUMS"
    fi
    PYS="$PYS${PYS:+, }$k"
  done
  # (the app shows this version before it installs: sb-core/src/hostsetup.rs)
  if [ -n "$PYS" ] && [ -f "$SRC/python/VERSION" ]; then cp "$SRC/python/VERSION" "$KIT/python/VERSION"; fi
fi
for f in fetch-python.sh python-pins.txt; do
  if [ -f "$SRC/scripts/install/$f" ]; then cp "$SRC/scripts/install/$f" "$KIT/scripts/install/$f"; fi
done
if [ -n "$HUB" ] && [ -s "$HUB" ]; then install -m 755 "$HUB" "$KIT/native/dist/switchboard-hub-$OSN"; fi
if [ -n "$NODE" ] && [ -s "$NODE" ]; then install -m 755 "$NODE" "$KIT/native/dist/switchboard-node-$OSN"; fi
echo "server kit: $KIT ($(du -sk "$KIT" | cut -f1) KB, hub: $([ -s "$KIT/native/dist/switchboard-hub-$OSN" ] && echo yes || echo no), Python: ${PYS:-none})"
