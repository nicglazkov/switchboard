#!/usr/bin/env bash
# Builds the iPhone app's keys library (crates/sb-mobile) and the Swift file that calls it. Run on a Mac with Xcode
# and Rust, from anywhere in the repository:
#   native/scripts/build-mobile-ios.sh
# Output, all under ios/Rust (git-ignored; ios/project.yml links it into the app):
#   ios/Rust/SwitchboardLink.xcframework   libsb_mobile.a for iPhones and for the simulator, with its C header
#   ios/Rust/Generated/sb_mobile.swift     the Swift side of the API in crates/sb-mobile/src/lib.rs (UniFFI)
#   ios/Rust/BUILD                         what it was built from, and how big it is
# Settings:
#   SB_IOS_X86_SIM=1          also build the simulator slice for Intel Macs (x86_64-apple-ios)
#   SB_RELAY_DEFAULT=<url>    the relay built into the library, the same setting the node's build takes
#   SB_MOBILE_PREBUILT=<dir>  no Rust here: copy a finished ios/Rust folder from <dir> instead of building
#   SB_MOBILE_PROFILE=mobile  the cargo profile (Cargo.toml); "dev" builds fast for trying things out
# Needs: rustup target add aarch64-apple-ios aarch64-apple-ios-sim (and x86_64-apple-ios for SB_IOS_X86_SIM).
# Nothing here signs anything: a static library is signed as part of the app that links it.
set -euo pipefail
cd "$(dirname "$0")/.."
OUT=../ios/Rust
NAME=SwitchboardLink

if [ -n "${SB_MOBILE_PREBUILT:-}" ]; then
  [ -d "$SB_MOBILE_PREBUILT/$NAME.xcframework" ] && [ -f "$SB_MOBILE_PREBUILT/Generated/sb_mobile.swift" ] || {
    echo "build-mobile-ios: $SB_MOBILE_PREBUILT has no $NAME.xcframework and Generated/sb_mobile.swift" >&2
    exit 1
  }
  rm -rf "$OUT"
  mkdir -p "$OUT"
  cp -R "$SB_MOBILE_PREBUILT"/. "$OUT"/
  echo "build-mobile-ios: copied the prebuilt library from $SB_MOBILE_PREBUILT"
  exit 0
fi

PROFILE=${SB_MOBILE_PROFILE:-mobile}
# (cargo's folder for the dev profile is "debug")
PDIR=$PROFILE
[ "$PROFILE" = dev ] && PDIR=debug
# The app's own deployment target (ios/project.yml), so the linker has no older objects to warn about.
export IPHONEOS_DEPLOYMENT_TARGET=${IPHONEOS_DEPLOYMENT_TARGET:-$(sed -n 's/^ *iOS: "\(.*\)"$/\1/p' ../ios/project.yml | head -1)}
TARGETS=(aarch64-apple-ios aarch64-apple-ios-sim)
[ "${SB_IOS_X86_SIM:-0}" = 1 ] && TARGETS+=(x86_64-apple-ios)

have=$(rustup target list --installed 2>/dev/null || true)
for t in "${TARGETS[@]}"; do
  echo "$have" | grep -qx "$t" || { echo "build-mobile-ios: missing the Rust target $t (rustup target add $t)" >&2; exit 1; }
done

start=$(date +%s)
for t in "${TARGETS[@]}"; do
  cargo build --locked --profile "$PROFILE" -p sb-mobile --lib --target "$t"
done

# The Swift file and the C header come from the library itself: a build for this Mac is read for what it exports.
cargo build --locked -p sb-mobile --features bindgen
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
target/debug/uniffi-bindgen generate target/debug/libsb_mobile.dylib --language swift --no-format --out-dir "$TMP/gen"
mkdir -p "$TMP/headers"
cp "$TMP/gen/sb_mobileFFI.h" "$TMP/headers/"
# (Xcode finds a static library's module by this exact file name)
cp "$TMP/gen/sb_mobileFFI.modulemap" "$TMP/headers/module.modulemap"

# One simulator library for both kinds of Mac when both were built.
SIM=target/aarch64-apple-ios-sim/$PDIR/libsb_mobile.a
if [ "${SB_IOS_X86_SIM:-0}" = 1 ]; then
  mkdir -p "$TMP/sim"
  lipo -create "$SIM" "target/x86_64-apple-ios/$PDIR/libsb_mobile.a" -output "$TMP/sim/libsb_mobile.a"
  SIM=$TMP/sim/libsb_mobile.a
fi

rm -rf "$OUT"
mkdir -p "$OUT/Generated"
xcodebuild -create-xcframework \
  -library "target/aarch64-apple-ios/$PDIR/libsb_mobile.a" -headers "$TMP/headers" \
  -library "$SIM" -headers "$TMP/headers" \
  -output "$OUT/$NAME.xcframework" >/dev/null
cp "$TMP/gen/sb_mobile.swift" "$OUT/Generated/"

{
  echo "sb-mobile $(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1), profile $PROFILE, $(rustc --version)"
  echo "targets: ${TARGETS[*]}"
  echo "relay built in: ${SB_RELAY_DEFAULT:-none}"
  echo "built in $(( $(date +%s) - start )) s"
  for f in "$OUT/$NAME.xcframework"/*/libsb_mobile.a; do
    echo "$(( $(stat -f %z "$f") / 1024 )) KiB  ${f#"$OUT"/}"
  done
} >"$OUT/BUILD"
cat "$OUT/BUILD"
