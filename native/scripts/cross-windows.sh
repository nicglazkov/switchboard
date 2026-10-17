#!/usr/bin/env bash
# Checks the Windows desktop app from Linux (a cloud VM, CI): cross-compiles sb-core and sb-desktop for
# x86_64-pc-windows-msvc with cargo-xwin and runs their unit tests under Wine. It builds a debug exe too.
# This is a check, not the shipped build: a release Switchboard.exe compiles GPUI's shaders with fxc.exe from the
# Windows SDK and embeds the icon with rc.exe, so it still comes from scripts/build-windows.sh on a Windows machine.
#   native/scripts/cross-windows.sh            check, test, debug build
#   SB_NO_TEST=1 native/scripts/cross-windows.sh   skip the tests
# Needs: rustup target x86_64-pc-windows-msvc, cargo-xwin (cargo install --locked cargo-xwin), clang, lld, and wine64
# for the tests. cargo-xwin downloads the MSVC CRT and Windows SDK headers on first use (accepting Microsoft's license).
set -euo pipefail
cd "$(dirname "$0")/.."
T=x86_64-pc-windows-msvc
export XWIN_ACCEPT_LICENSE=1
cargo xwin build --locked --target $T -p sb-desktop
ls -l target/$T/debug/Switchboard.exe
if [ -z "${SB_NO_TEST:-}" ]; then
  # Wine runs each test binary; no display is needed, nothing here opens a window.
  export WINEDEBUG=-all WINEPREFIX="${WINEPREFIX:-$PWD/target/wine}"
  cargo xwin test --locked --target $T -p sb-core -p sb-desktop
fi
