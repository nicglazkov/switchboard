#!/usr/bin/env bash
# Make the Ed25519 key pair the update list is signed with, and print the public key.
#
#   native/scripts/update-key.sh ~/.config/switchboard/update-key.pem
#
# Writes the private key to the file named (it refuses to replace one, or to write inside this repo) and prints
# SB_UPDATE_PUBKEY=<base64>. Builds made with that variable set check the list's signature; publish.sh signs with
# the private key named by SB_UPDATE_KEY_FILE. Keep the private key out of the repo and backed up: a lost key means
# the apps already out there can't take another update until they are installed again by hand.
#
#   native/scripts/update-key.sh --public FILE   prints the public key of an existing private key
#
# Needs OpenSSL 3 (the macOS /usr/bin/openssl is LibreSSL and can't); SB_OPENSSL names another openssl.
set -euo pipefail
OPENSSL=${SB_OPENSSL:-openssl}
REPO="$(cd "$(dirname "$0")/../.." && pwd)"

pub() {
  local key
  # The DER form is a 12-byte header and then the 32-byte key.
  key=$("$OPENSSL" pkey -in "$1" -pubout -outform DER | tail -c 32 | base64 | tr -d '\n') || {
    echo "update-key: could not read $1 (it needs OpenSSL 3; set SB_OPENSSL)" >&2
    exit 1
  }
  printf '%s' "$key"
}

case "${1:-}" in
  ""|-h|--help) sed -n '2,13p' "$0"; exit 0 ;;
  --public) echo "SB_UPDATE_PUBKEY=$(pub "${2:?--public needs the private key file}")"; exit 0 ;;
esac

OUT=$1
[ -e "$OUT" ] && { echo "update-key: $OUT is already there; not replacing it" >&2; exit 1; }
DIR="$(mkdir -p "$(dirname "$OUT")" && cd "$(dirname "$OUT")" && pwd)"
case "$DIR/" in
  "$REPO"/*) echo "update-key: $OUT is inside the repo; put the private key somewhere else" >&2; exit 1 ;;
esac
(umask 077 && "$OPENSSL" genpkey -algorithm ed25519 -out "$OUT") || {
  echo "update-key: could not make an Ed25519 key (it needs OpenSSL 3; set SB_OPENSSL)" >&2
  exit 1
}
echo "update-key: private key in $OUT (keep it out of the repo; publish.sh reads it from SB_UPDATE_KEY_FILE)"
echo "SB_UPDATE_PUBKEY=$(pub "$OUT")"
