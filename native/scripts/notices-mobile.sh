#!/usr/bin/env bash
# Regenerates native/packaging/mobile/THIRD-PARTY-NOTICES.txt: the license of every Rust crate inside the phone apps'
# keys library (crates/sb-mobile, for iPhone and Android). Run it after changing that library's dependencies and
# check the result in. Both apps ship the file as it is and show it under Settings, Open source licenses: the
# iPhone app as a bundled resource (ios/project.yml), the Android app as an asset (android-native/app/build.gradle.kts).
#   native/scripts/notices-mobile.sh
# Needs cargo-about, like scripts/notices.sh (CARGO_ABOUT=/path/to/cargo-about picks one that is not on PATH). A
# crate under a license that packaging/notices/about-mobile.toml does not accept stops the script.
set -euo pipefail
cd "$(dirname "$0")/.."
ABOUT=${CARGO_ABOUT:-cargo-about}
OUT=packaging/mobile/THIRD-PARTY-NOTICES.txt
TMP=$(mktemp)
trap 'rm -f "$TMP"' EXIT

NAME=$(sed -n 's/^name = "\([^"]*\)".*/\1/p' ../branding.toml 2>/dev/null | head -1)
NAME=${NAME:-Switchboard}
{
  # (each paragraph is one line: the apps show this on a narrow screen and wrap it themselves)
  echo "The part of $NAME that connects to your home computer is built from the open source software listed below. Their licenses ask that these notices ship with it. $NAME's own code is under the GNU AGPL-3.0."
  echo
} >"$TMP"

# The workspace's own crates (sb-*) are this app, not third-party code.
"$ABOUT" generate --locked --fail -m crates/sb-mobile/Cargo.toml -c packaging/notices/about-mobile.toml packaging/notices/about-mobile.hbs |
  grep -Ev '^  sb-(core|desktop|hub|install|link|mobile|node) [0-9]' >>"$TMP"

mkdir -p "$(dirname "$OUT")"
# cargo-about writes some characters as HTML entities, and a few licenses come with Windows line ends; this file is
# plain text with one kind of line end.
tr -d '\r' <"$TMP" | sed -e 's/&quot;/"/g' -e "s/&#x27;/'/g" -e 's/&lt;/</g' -e 's/&gt;/>/g' -e 's/&#x3D;/=/g' -e 's/&#x60;/`/g' \
  -e 's/&amp;/\&/g' >"$OUT"
echo "notices: $(grep -c '^  [A-Za-z0-9_-]* [0-9]' "$OUT") crate entries, $(wc -c <"$OUT" | tr -d ' ') bytes, $OUT"
