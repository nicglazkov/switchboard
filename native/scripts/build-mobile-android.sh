#!/usr/bin/env bash
# Builds the Android app's keys library (crates/sb-mobile) and the Kotlin file that calls it. Run on a computer with
# Rust and the Android NDK, from anywhere in the repository:
#   native/scripts/build-mobile-android.sh
# Output, all git-ignored (android-native/app/build.gradle.kts picks both up):
#   android-native/app/src/main/jniLibs/<abi>/libsb_mobile.so      arm64-v8a (phones) and x86_64 (the emulator)
#   android-native/app/src/generated/kotlin/uniffi/sb_mobile/sb_mobile.kt   the Kotlin side of the API (UniFFI)
#   android-native/app/src/generated/BUILD                          what it was built from, and how big it is
# Settings:
#   SB_ANDROID_ABIS="arm64-v8a x86_64"  which to build; add armeabi-v7a or x86 for 32-bit devices
#   SB_RELAY_DEFAULT=<url>     the relay built into the library, the same setting the node's build takes
#   SB_MOBILE_PREBUILT=<dir>   no Rust here: copy finished jniLibs/ and kotlin/ folders from <dir> instead of building
#   SB_MOBILE_PROFILE=mobile   the cargo profile (Cargo.toml); "dev" builds fast for trying things out
#   ANDROID_NDK_HOME           the NDK to use; otherwise the newest under $ANDROID_HOME/ndk (or the SDK's usual place)
# Needs: cargo install --locked cargo-ndk, rustup target add aarch64-linux-android x86_64-linux-android, and an NDK
# (r28 or newer, so the libraries are laid out for 16 KB pages, which Google Play asks for).
set -euo pipefail
cd "$(dirname "$0")/.."
APP=../android-native/app
JNI=$APP/src/main/jniLibs
GEN=$APP/src/generated
# The app's minSdk (android-native/app/build.gradle.kts).
API=$(sed -n 's/^ *minSdk = \([0-9]*\).*/\1/p' "$APP/build.gradle.kts" | head -1)

if [ -n "${SB_MOBILE_PREBUILT:-}" ]; then
  [ -d "$SB_MOBILE_PREBUILT/jniLibs" ] && [ -f "$SB_MOBILE_PREBUILT/kotlin/uniffi/sb_mobile/sb_mobile.kt" ] || {
    echo "build-mobile-android: $SB_MOBILE_PREBUILT has no jniLibs/ and kotlin/uniffi/sb_mobile/sb_mobile.kt" >&2
    exit 1
  }
  rm -rf "$JNI" "$GEN"
  mkdir -p "$JNI" "$GEN/kotlin"
  cp -R "$SB_MOBILE_PREBUILT/jniLibs"/. "$JNI"/
  cp -R "$SB_MOBILE_PREBUILT/kotlin"/. "$GEN/kotlin"/
  echo "build-mobile-android: copied the prebuilt library from $SB_MOBILE_PREBUILT"
  exit 0
fi

PROFILE=${SB_MOBILE_PROFILE:-mobile}
ABIS=${SB_ANDROID_ABIS:-arm64-v8a x86_64}
command -v cargo-ndk >/dev/null || { echo "build-mobile-android: cargo-ndk is missing (cargo install --locked cargo-ndk)" >&2; exit 1; }
if [ -z "${ANDROID_NDK_HOME:-}" ]; then
  for sdk in "${ANDROID_HOME:-}" "${ANDROID_SDK_ROOT:-}" "$HOME/Library/Android/sdk" "$HOME/Android/Sdk"; do
    [ -n "$sdk" ] && [ -d "$sdk/ndk" ] || continue
    newest=$(ls "$sdk/ndk" | sort -t. -k1,1n -k2,2n -k3,3n | tail -1)
    [ -n "$newest" ] && { export ANDROID_NDK_HOME=$sdk/ndk/$newest; break; }
  done
fi
[ -d "${ANDROID_NDK_HOME:-}" ] || { echo "build-mobile-android: no Android NDK found (set ANDROID_NDK_HOME)" >&2; exit 1; }

start=$(date +%s)
T=()
for abi in $ABIS; do T+=(-t "$abi"); done
rm -rf "$JNI" "$GEN"
mkdir -p "$JNI" "$GEN/kotlin"
cargo ndk "${T[@]}" --platform "$API" -o "$JNI" build --locked --profile "$PROFILE" -p sb-mobile --lib
# (cargo-ndk also copies the shared libraries two of iroh's crates build for themselves; the app loads only ours,
# which has them inside)
find "$JNI" -name '*.so' ! -name libsb_mobile.so -delete

# The Kotlin file comes from the library itself: a build for this computer is read for what it exports.
cargo build --locked -p sb-mobile --features bindgen
HOSTLIB=target/debug/libsb_mobile.so
[ -f "$HOSTLIB" ] || HOSTLIB=target/debug/libsb_mobile.dylib
target/debug/uniffi-bindgen generate "$HOSTLIB" --language kotlin --no-format --out-dir "$GEN/kotlin"

{
  echo "sb-mobile $(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1), profile $PROFILE, $(rustc --version)"
  echo "NDK $(basename "$ANDROID_NDK_HOME"), android-$API, abis: $ABIS"
  echo "relay built in: ${SB_RELAY_DEFAULT:-none}"
  echo "built in $(( $(date +%s) - start )) s"
  for f in "$JNI"/*/libsb_mobile.so; do
    echo "$(( $(wc -c <"$f") / 1024 )) KiB  ${f#"$JNI"/}"
  done
} >"$GEN/BUILD"
cat "$GEN/BUILD"
