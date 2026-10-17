# Build, test and publish hosts for the scripts in native/scripts and scripts/. They are different on every
# setup, so they live outside the repo, in ${SWITCHBOARD_BUILD_HOSTS:-~/.config/switchboard/build-hosts.env}
# (native/scripts/build-hosts.example.env shows every variable). A variable already set in the environment wins.
#   . "$(dirname "$0")/hosts.sh"; sb_hosts SB_LINUX_HOST      # exits with a message when it is not set
#   sb_host_opt SB_NODE_BASE || SB_NODE_BASE=...              # optional: returns 1 when it is not set
SB_HOSTS_FILE=${SWITCHBOARD_BUILD_HOSTS:-$HOME/.config/switchboard/build-hosts.env}

sb_host_opt() {
  local v=$1 val
  [ -n "${!v:-}" ] && return 0
  [ -f "$SB_HOSTS_FILE" ] || return 1
  # A subshell, so the file sets nothing here but the one variable asked for.
  val=$(set +eu; . "$SB_HOSTS_FILE" >/dev/null 2>&1; eval "printf '%s' \"\${$v:-}\"")
  [ -n "$val" ] || return 1
  printf -v "$v" '%s' "$val"
}

# The update key's public half and the optional feed URL, for the build scripts to pass to cargo (see
# docs/building.md). Neither is secret; a build without SB_UPDATE_PUBKEY has updates off, and this says so.
sb_update_env() {
  sb_host_opt SB_UPDATE_PUBKEY || SB_UPDATE_PUBKEY=""
  sb_host_opt SB_UPDATE_FEED || SB_UPDATE_FEED=""
  case "$SB_UPDATE_PUBKEY$SB_UPDATE_FEED" in
    *[!A-Za-z0-9+/=:._/%~-]*) echo "SB_UPDATE_PUBKEY or SB_UPDATE_FEED has characters no key or URL has" >&2; exit 1 ;;
  esac
  [ -n "$SB_UPDATE_PUBKEY" ] || echo "note: SB_UPDATE_PUBKEY is not set, so this build has updates off" >&2
}

sb_hosts() {
  local v
  for v in "$@"; do
    sb_host_opt "$v" || {
      echo "$v is not set: add it to $SB_HOSTS_FILE (native/scripts/build-hosts.example.env shows the format)." >&2
      exit 1
    }
  done
}
