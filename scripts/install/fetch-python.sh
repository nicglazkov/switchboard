#!/usr/bin/env bash
# Download Switchboard's own Python for the kits, checked against the SHA-256 pinned in python-pins.txt.
#
#   scripts/install/fetch-python.sh OUT_DIR                      # every kind in python-pins.txt
#   scripts/install/fetch-python.sh OUT_DIR mac-arm64 mac-x86_64 # only these
#
# It writes OUT_DIR/python-<kind>.tar.gz for each kind (mac-arm64, mac-x86_64, linux-x86_64, linux-arm64,
# windows-x86_64), OUT_DIR/SHA256SUMS with one line for each of them, and OUT_DIR/VERSION. Each archive unpacks to
# one folder, python/. pack-server-kit.sh and the build scripts put these in the kits; install-server.sh and
# install-agent.sh unpack the one for the computer they run on.
#
# A download is kept in ~/.cache/switchboard/python (SWITCHBOARD_PYTHON_CACHE), so a kit is packed again without
# the network. A file that does not match its pinned SHA-256 is deleted and nothing is written for it: the script
# stops. Nothing that was downloaded is ever run here.
#
# For tests and mirrors: SWITCHBOARD_PYTHON_PINS (another pins file), SWITCHBOARD_PYTHON_BASE (another address the
# release folders are under).
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
PINS="${SWITCHBOARD_PYTHON_PINS:-$HERE/python-pins.txt}"
CACHE="${SWITCHBOARD_PYTHON_CACHE:-$HOME/.cache/switchboard/python}"
case "${1:-}" in
  ""|-h|--help) sed -n '2,/^set -e/p' "$0" | sed '$d; s/^# \{0,1\}//'; [ -n "${1:-}" ] && exit 0 || exit 2 ;;
esac
OUT=$1; shift
die() { echo "fetch-python.sh: $*" >&2; exit 1; }
[ -f "$PINS" ] || die "no pins file at $PINS"

word() { awk -v k="$1" '$1 == k { print $2; exit }' "$PINS"; }
RELEASE=$(word release); VERSION=$(word version)
BASE="${SWITCHBOARD_PYTHON_BASE:-$(word base)}"
[ -n "$RELEASE" ] && [ -n "$VERSION" ] && [ -n "$BASE" ] || die "$PINS names no release, version or base"
KINDS="$*"
[ -n "$KINDS" ] || KINDS=$(awk 'NF == 3 && $1 !~ /^#/ && length($2) == 64 { print $1 }' "$PINS" | tr '\n' ' ')

sha() { if command -v sha256sum >/dev/null; then sha256sum "$1" | cut -d ' ' -f 1; else shasum -a 256 "$1" | cut -d ' ' -f 1; fi; }
fetch() {  # $1 = URL, $2 = file
  if command -v curl >/dev/null; then curl -fsSL --retry 2 -A switchboard-kit "$1" -o "$2"
  elif command -v wget >/dev/null; then wget -q -U switchboard-kit "$1" -O "$2"
  else die "curl or wget is needed to download Python"; fi
}

mkdir -p "$OUT" "$CACHE"
: >"$OUT/SHA256SUMS.new"
for kind in $KINDS; do
  want=$(awk -v k="$kind" 'NF == 3 && $1 == k { print $2; exit }' "$PINS")
  asset=$(awk -v k="$kind" 'NF == 3 && $1 == k { print $3; exit }' "$PINS")
  [ -n "$want" ] && [ -n "$asset" ] || die "$PINS has no line for $kind"
  case "$want" in *[!0-9a-f]*) die "the SHA-256 pinned for $kind is not a SHA-256" ;; esac
  [ "${#want}" = 64 ] || die "the SHA-256 pinned for $kind is not a SHA-256"
  file="$CACHE/$asset"
  if [ ! -f "$file" ] || [ "$(sha "$file")" != "$want" ]; then
    rm -f "$file" "$file.part"
    # (a + in a GitHub release address is written %2B)
    fetch "$BASE/$RELEASE/$(printf '%s' "$asset" | sed 's/+/%2B/g')" "$file.part" || { rm -f "$file.part"; die "could not download $asset"; }
    got=$(sha "$file.part")
    if [ "$got" != "$want" ]; then
      rm -f "$file.part"
      die "$asset does not match its pinned SHA-256 ($want expected, $got downloaded). It was deleted."
    fi
    mv "$file.part" "$file"
  fi
  cp "$file" "$OUT/python-$kind.tar.gz"
  printf '%s  python-%s.tar.gz\n' "$want" "$kind" >>"$OUT/SHA256SUMS.new"
  echo "python-$kind.tar.gz  $(($(wc -c <"$file") / 1024)) KB  CPython $VERSION"
done
mv "$OUT/SHA256SUMS.new" "$OUT/SHA256SUMS"
printf '%s %s\n' "$VERSION" "$RELEASE" >"$OUT/VERSION"
