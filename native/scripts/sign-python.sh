#!/usr/bin/env bash
# Signs every program and library of the Python a server kit carries (server-kit.sh lays it out as python/<kind>/
# on a Mac), each on its own. Apple's notary refuses an app with any program in it that is not signed with the
# Developer ID, a secure timestamp and the hardened runtime, wherever in the app it is; and codesign --deep does
# not sign programs under Resources.
#   sign-python.sh <the kit's python folder> <identity, or - for ad hoc> [more codesign options]
# Does nothing when the folder is not there. Python needs no entitlements for what Switchboard runs with it (the
# standard library only): its libraries carry the same team's signature, so library validation passes.
set -euo pipefail
DIR=${1:?the python folder}; ID=${2:?identity}; shift 2
[ -d "$DIR" ] || exit 0
n=0
while IFS= read -r -d '' f; do
  case "$(file -b "$f")" in
    Mach-O*) codesign --force "$@" -s "$ID" "$f"; n=$((n + 1)) ;;
  esac
done < <(find "$DIR" -type f \( -perm -u+x -o -name '*.dylib' -o -name '*.so' -o -name '*.a' \) -print0)
echo "signed $n programs and libraries of the kit's Python"
