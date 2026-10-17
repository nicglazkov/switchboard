#!/usr/bin/env bash
# Regenerates native/packaging/THIRD-PARTY-NOTICES.txt: the license of every Rust crate the apps are built from (all
# platforms), then the bundled fonts. Run it after changing dependencies and check the result in. The build scripts
# copy the file into Switchboard.app/Contents/Resources, next to Switchboard.exe and into the Linux tarball, and the
# app keeps a built-in copy for About > Open source licenses.
#   native/scripts/notices.sh
# Needs cargo-about (cargo install --locked cargo-about --features cli); CARGO_ABOUT=/path/to/cargo-about picks one
# that is not on PATH. A crate under a license that packaging/notices/about.toml does not accept stops the script.
set -euo pipefail
cd "$(dirname "$0")/.."
ABOUT=${CARGO_ABOUT:-cargo-about}
OUT=packaging/THIRD-PARTY-NOTICES.txt
TMP=$(mktemp)
trap 'rm -f "$TMP"' EXIT

NAME=$(sed -n 's/^name = "\([^"]*\)".*/\1/p' ../branding.toml 2>/dev/null | head -1)
NAME=${NAME:-Switchboard}
{
  echo "Third-party notices for $NAME"
  echo
  echo "$NAME is built from the open source software listed below. Their licenses ask that these notices ship"
  echo "with it. $NAME's own code is under the GNU AGPL-3.0."
  echo
} >"$TMP"

# The workspace's own crates (sb-*) are this app, not third-party code.
"$ABOUT" generate --locked --workspace --fail -c packaging/notices/about.toml packaging/notices/about.hbs |
  grep -Ev '^  sb-(core|desktop|hub|node) [0-9]' >>"$TMP"

{
  echo
  echo
  echo "Fonts"
  echo "====="
  for f in crates/sb-desktop/assets/fonts/OFL-*.txt; do
    family=$(basename "$f" .txt)
    family=${family#OFL-}
    echo
    echo "--------------------------------------------------------------------------------"
    echo "$family (SIL Open Font License 1.1), files: $(cd crates/sb-desktop/assets/fonts && ls "$family"-*.ttf | tr '\n' ' ')"
    echo
    tr -d '\r' <"$f"
  done
} >>"$TMP"

# cargo-about writes some characters as HTML entities; this file is plain text.
sed -e 's/&quot;/"/g' -e "s/&#x27;/'/g" -e 's/&lt;/</g' -e 's/&gt;/>/g' -e 's/&#x3D;/=/g' -e 's/&#x60;/`/g' \
  -e 's/&amp;/\&/g' "$TMP" >"$OUT"
echo "notices: $(grep -c '^  [A-Za-z0-9_-]* [0-9]' "$OUT") crate entries, $(wc -c <"$OUT" | tr -d ' ') bytes, $OUT"
