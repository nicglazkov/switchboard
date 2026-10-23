#!/usr/bin/env bash
# Install the Switchboard server with one line, on a Mac or Linux machine that has no copy of the repository
# (a headless box at home, say). No sudo.
#
#   curl -fsSL <where the kit is published>/get-server.sh | bash
#   curl -fsSL <where the kit is published>/get-server.sh | bash -s -- --no-hub     # with installer options
#   bash get-server.sh --kit ./switchboard-server-kit.tar.gz                       # a kit you copied over
#
# It downloads the server kit (server/, agent/sb.py and scripts/install/, made by pack-server-kit.sh), checks its
# SHA-256, unpacks it into a temporary folder and runs scripts/install/install-server.sh from there. That copies
# what it needs into ~/.local/share/switchboard, so the temporary folder is removed afterwards.
#
# Options:
#   --kit URL|FILE    the kit (default: SWITCHBOARD_KIT_URL, else the address this copy was packed with)
#   --sha256 HEX      the SHA-256 the kit must have (default: read from <kit>.sha256 next to it, when there is one)
# Everything else goes to install-server.sh (see its --help: --no-hub, --no-service, --yes, --plain, ...).
set -euo pipefail

# pack-server-kit.sh --base URL fills this in. Empty in the repository: there is no public download yet.
KIT_DEFAULT=""

main() {
  local kit="${SWITCHBOARD_KIT_URL:-$KIT_DEFAULT}" sum="" pass=()
  while [ $# -gt 0 ]; do
    case "$1" in
      --kit) kit="${2:?--kit needs a URL or a file}"; shift ;;
      --sha256) sum="${2:?--sha256 needs a hex digest}"; shift ;;
      -h|--help) sed -n '2,/^set -e/p' "$0" 2>/dev/null | sed '$d; s/^# \{0,1\}//'; exit 0 ;;
      *) pass+=("$1") ;;
    esac
    shift
  done

  local fancy=0 R="" G="" X="" ok="ok" bad="x"
  if [ -t 1 ] && [ -z "${NO_COLOR:-}" ] && [ "${TERM:-dumb}" != dumb ]; then
    for a in ${pass[@]+"${pass[@]}"}; do [ "$a" = --plain ] && fancy=-1; done
    [ "$fancy" = 0 ] && fancy=1
  fi
  [ "$fancy" = 1 ] || fancy=0
  if [ "$fancy" = 1 ]; then
    R=$'\033[0m' G=$'\033[38;5;78m' X=$'\033[38;5;203m'
    case "${LC_ALL:-${LC_CTYPE:-${LANG:-}}}" in *[Uu][Tt][Ff]-8*|*[Uu][Tt][Ff]8*) ok="✓" bad="✗" ;; *) ok="+" ;; esac
  fi
  say() { if [ "$fancy" = 1 ]; then printf '  %s%s%s  %s\n' "$G" "$ok" "$R" "$*"; else printf '%s\n' "$*"; fi; }
  die() {
    if [ "$fancy" = 1 ]; then printf '\n  %s%s%s  %s\n\n' "$X" "$bad" "$R" "$*" >&2; else printf 'get-server.sh: %s\n' "$*" >&2; fi
    exit 1
  }

  if [ -z "$kit" ]; then
    die "no server kit to install from. Pass --kit URL (or a file), or set SWITCHBOARD_KIT_URL. Make a kit with scripts/install/pack-server-kit.sh in a copy of the repository."
  fi
  case "$(uname -s)" in
    Linux|Darwin) ;;
    *) die "this is for macOS and Linux. On Windows, run scripts\\install\\install-server.ps1 from the kit." ;;
  esac
  command -v tar >/dev/null || die "tar is not installed; it is needed to unpack the kit."

  fetch() {  # $1 = URL, $2 = file
    if command -v curl >/dev/null; then curl -fsSL --retry 2 -o "$2" "$1"
    elif command -v wget >/dev/null; then wget -q -O "$2" "$1"
    elif command -v python3 >/dev/null; then python3 -c 'import sys, urllib.request; urllib.request.urlretrieve(sys.argv[1], sys.argv[2])' "$1" "$2"
    else return 127; fi
  }
  sha() { if command -v sha256sum >/dev/null; then sha256sum "$1" | cut -d ' ' -f 1; else shasum -a 256 "$1" | cut -d ' ' -f 1; fi; }

  TMP_KIT=$(mktemp -d "${TMPDIR:-/tmp}/switchboard-kit.XXXXXX")
  local tmp=$TMP_KIT
  trap 'rm -rf "$TMP_KIT"' EXIT
  [ "$fancy" = 1 ] && printf '\n'

  local file="$tmp/kit.tar.gz"
  if [ -f "$kit" ]; then
    cp "$kit" "$file"
    [ -n "$sum" ] || { [ -f "$kit.sha256" ] && sum=$(cut -d ' ' -f 1 <"$kit.sha256"); } || true
    say "Server kit from $kit"
  else
    case "$kit" in http://*|https://*) ;; *) die "--kit is neither a file here nor an http(s) address: $kit" ;; esac
    fetch "$kit" "$file" || die "could not download the server kit from $kit"
    [ -n "$sum" ] || { fetch "$kit.sha256" "$tmp/kit.sha256" 2>/dev/null && sum=$(cut -d ' ' -f 1 <"$tmp/kit.sha256"); } || true
    say "Downloaded the server kit ($(($(wc -c <"$file") / 1024)) KB) from ${kit%/*}"
  fi
  if [ -n "$sum" ]; then
    [ "$(sha "$file")" = "$sum" ] || die "the kit's SHA-256 does not match ($sum expected). Nothing was installed."
    say "Its SHA-256 matches"
  else
    say "No SHA-256 to check it against (no --sha256 and no .sha256 next to it)"
  fi
  mkdir "$tmp/kit"
  tar -xzf "$file" -C "$tmp/kit" || die "the kit did not unpack (is it a server kit?)"
  local top="" d
  for d in "$tmp/kit"/*/; do [ -f "${d}scripts/install/install-server.sh" ] && top=${d%/}; done
  [ -n "$top" ] || die "the kit has no scripts/install/install-server.sh."
  say "Unpacked$( [ -f "$top/VERSION" ] && printf ' version %s' "$(head -n 1 "$top/VERSION")" )"

  # Piped into bash, this script's stdin is the download, so first-run setup reads from the terminal instead.
  local rc=0
  if [ ! -t 0 ] && { : </dev/tty; } 2>/dev/null; then
    bash "$top/scripts/install/install-server.sh" ${pass[@]+"${pass[@]}"} </dev/tty || rc=$?
  else
    bash "$top/scripts/install/install-server.sh" ${pass[@]+"${pass[@]}"} || rc=$?
  fi
  return "$rc"
}

main "$@"
