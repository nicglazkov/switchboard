#!/usr/bin/env bash
# Install the Switchboard server on this machine (macOS or Linux), for the person who runs it. No sudo.
#
#   scripts/install/install-server.sh            # from a copy of the repository or of the server kit
#
# What it puts in place:
#   ~/.local/share/switchboard/app/        the server (server/, agent/sb.py, install/setup.py)
#   ~/.local/bin/switchboard-hub           the hub: terminals and linked machines (built here, or --hub-binary)
#   ~/.local/share/switchboard/downloads/  what Add a machine hands out (switchboard-node for each OS, sb.py)
#   ~/.local/bin/switchboard               start, stop, status, logs, setup
# and runs three user services: the server (:8130), the hub (:8132) and the downloads (:8131). They are systemd
# user services on Linux and LaunchAgents on macOS; with neither, plain background processes.
#
# Options:
#   --hub-binary PATH   use this switchboard-hub instead of building one
#   --node-dir DIR      a folder with switchboard-node-mac, switchboard-node-linux and switchboard-node.exe
#   --no-hub            skip the hub (no terminals, no linked machines; machines are reached over ssh)
#   --no-service        don't register services; start plain background processes
#   --no-start          install only
#   --no-setup          don't run first-run setup at the end
#   --yes               don't ask anything (setup is skipped unless it was given its own answers)
#   --uninstall         stop and remove everything this put in place (settings and history are kept)
#   --plain             plain output, the same as when it goes to a file or another program
#
# On a terminal it checks the machine first, shows each step with a spinner and its time, keeps command output in
# ~/.local/share/switchboard/logs/install.log, and ends with the addresses to use. Piped, with --plain, with
# NO_COLOR set or on a dumb terminal, it prints the plain "== <step>" lines (the Mac app reads those).
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "$HERE/../.." && pwd)"
SHARE="$HOME/.local/share/switchboard"
APP="$SHARE/app"
BIN="$HOME/.local/bin"
CFG="$HOME/.config/switchboard"
DL="$SHARE/downloads/node"
LOGF="$SHARE/logs/install.log"

HUB_BIN="" NODE_DIR="" NO_HUB=0 NO_SERVICE=0 NO_START=0 NO_SETUP=0 YES=0 UNINSTALL=0 PLAIN=0
while [ $# -gt 0 ]; do
  case "$1" in
    --hub-binary) HUB_BIN="$2"; shift ;;
    --node-dir) NODE_DIR="$2"; shift ;;
    --no-hub) NO_HUB=1 ;;
    --no-service) NO_SERVICE=1 ;;
    --no-start) NO_START=1 ;;
    --no-setup) NO_SETUP=1 ;;
    --yes|-y) YES=1 ;;
    --uninstall) UNINSTALL=1 ;;
    --plain) PLAIN=1 ;;
    -h|--help) sed -n '2,/^set -e/p' "$0" | sed '$d; s/^# \{0,1\}//'; exit 0 ;;
    *) echo "install-server.sh: unknown option $1 (see --help)" >&2; exit 2 ;;
  esac
  shift
done

# ---------------------------------------------------------------- how it looks
# FANCY: a terminal that can take colors. Everything else gets exactly the plain lines below.
FANCY=0
if [ "$PLAIN" = 0 ] && [ -t 1 ] && [ -z "${NO_COLOR:-}" ]; then
  case "${TERM:-dumb}" in dumb|unknown|"") ;; *) FANCY=1 ;; esac
fi
[ "$PLAIN" = 1 ] && export SWITCHBOARD_PLAIN=1  # setup.py stays plain too

R="" B="" D="" ACC="" GRN="" YEL="" RED=""
UTF=0
case "${LC_ALL:-${LC_CTYPE:-${LANG:-}}}" in *[Uu][Tt][Ff]-8*|*[Uu][Tt][Ff]8*) UTF=1 ;; esac
if [ "$UTF" = 1 ]; then
  G_OK="✓" G_BAD="✗" G_WARN="!" G_INFO="·" G_MARK="◆"
  FRAMES=("⠋" "⠙" "⠹" "⠸" "⠼" "⠴" "⠦" "⠧" "⠇" "⠏")
  BX_TL="╭" BX_TR="╮" BX_BL="╰" BX_BR="╯" BX_H="─" BX_V="│"
else
  G_OK="+" G_BAD="x" G_WARN="!" G_INFO="-" G_MARK="*"
  FRAMES=("|" "/" "-" "\\")
  BX_TL="+" BX_TR="+" BX_BL="+" BX_BR="+" BX_H="-" BX_V="|"
fi

if [ "$FANCY" = 1 ]; then
  exec 3>&1  # the terminal, while a step's own output goes to the log
  NCOL=$(tput colors 2>/dev/null || true)
  case "$NCOL" in ''|*[!0-9]*) case "$TERM" in *256*|xterm*|screen*|tmux*|rxvt*|linux|vt220|ansi) NCOL=8 ;; *) NCOL=0 ;; esac ;; esac
  if [ "$NCOL" -ge 8 ]; then
    R=$'\033[0m' B=$'\033[1m' D=$'\033[2m'
    case "${COLORTERM:-}" in
      truecolor|24bit) ACC=$'\033[38;2;60;199;194m' GRN=$'\033[38;2;94;201;122m' YEL=$'\033[38;2;232;179;76m' RED=$'\033[38;2;240;98;98m' ;;
      *) if [ "$NCOL" -ge 256 ]; then ACC=$'\033[38;5;80m' GRN=$'\033[38;5;78m' YEL=$'\033[38;5;179m' RED=$'\033[38;5;203m'
         else ACC=$'\033[36m' GRN=$'\033[32m' YEL=$'\033[33m' RED=$'\033[31m'; fi ;;
    esac
  fi
fi

cols() {
  local c; c=$(tput cols 2>/dev/null || true)
  case "$c" in ''|*[!0-9]*|0) c=$(stty size 2>/dev/null </dev/tty | awk '{print $2}' || true) ;; esac
  case "$c" in ''|*[!0-9]*|0) c=${COLUMNS:-80} ;; esac
  echo "$c"
}
WIDTH=$(cols); [ "$WIDTH" -gt 78 ] && WIDTH=78
[ "$WIDTH" -lt 40 ] && WIDTH=40
rep() { local s="" i=0; while [ $i -lt "$2" ]; do s="$s$1"; i=$((i + 1)); done; printf '%s' "$s"; }  # $1 x $2
tilde() { case "$1" in "$HOME"/*) printf '~%s' "${1#"$HOME"}" ;; *) printf '%s' "$1" ;; esac; }

now_ms() {
  if [ -n "${EPOCHREALTIME:-}" ]; then local t=${EPOCHREALTIME/[.,]/}; echo $((10#$t / 1000)); return; fi
  local n; n=$(date +%s%N 2>/dev/null || true)
  case "$n" in *N|''|*[!0-9]*) ;; *) echo $((n / 1000000)); return ;; esac
  perl -MTime::HiRes=time -e 'printf "%d\n", time * 1000' 2>/dev/null && return
  echo $(($(date +%s) * 1000))
}
fmt_ms() {
  local ms=$1
  if [ "$ms" -lt 1000 ]; then printf '%d ms' "$ms"
  elif [ "$ms" -lt 10000 ]; then printf '%d.%ds' $((ms / 1000)) $(((ms % 1000) / 100))
  elif [ "$ms" -lt 60000 ]; then printf '%ds' $((ms / 1000))
  else printf '%dm %02ds' $((ms / 60000)) $(((ms % 60000) / 1000)); fi
}

cursor() {
  if [ "$1" = off ]; then tput civis 2>/dev/null || printf '\033[?25l'; else tput cnorm 2>/dev/null || printf '\033[?25h'; fi
} >&3

SPIN_PID=""
spin_start() {  # $1 = what the step is doing
  local label=$1 t0=$SECONDS max=$((WIDTH - 18))
  [ "${#label}" -gt "$max" ] && label="${label:0:$((max - 3))}..."
  cursor off
  (
    trap 'exit 0' TERM
    i=0
    while :; do
      el=$((SECONDS - t0)); ts=""; [ "$el" -ge 2 ] && ts="  $D${el}s$R"
      printf '\r  %s%s%s  %s%-5s%s %s%s\033[K' "$ACC" "${FRAMES[$((i % ${#FRAMES[@]}))]}" "$R" "$D" "$STEP_N/$STEP_TOTAL" "$R" "$label" "$ts" >&3
      i=$((i + 1)); sleep 0.08
    done
  ) &
  SPIN_PID=$!
}
spin_stop() {
  [ -n "$SPIN_PID" ] || return 0
  kill "$SPIN_PID" 2>/dev/null || true
  wait "$SPIN_PID" 2>/dev/null || true
  SPIN_PID=""
  printf '\r\033[K' >&3
  cursor on
}

# A step's result line: glyph, n/N, title, and its time against the right edge.
step_line() {  # $1 = color, $2 = glyph, $3 = title, $4 = time
  local left pad
  left="  $2  $(printf '%-5s' "$STEP_N/$STEP_TOTAL") $3"
  pad=$((WIDTH - ${#left} - ${#4}))
  [ "$pad" -lt 2 ] && pad=2
  printf '  %s%s%s  %s%-5s%s %s%s%s%s\n' "$1" "$2" "$R" "$D" "$STEP_N/$STEP_TOTAL" "$R" "$3" "$(rep ' ' "$pad")" "$D$4" "$R" >&3
}

# Lines shown under a step on a terminal (plain output never shows them).
DETAILS="" WARNED=0
fit() { local t="$*" max=$((WIDTH - 11)); [ "${#t}" -gt "$max" ] && t="${t:0:$((max - 3))}..."; printf '%s' "$t"; }
detail() { [ "$FANCY" = 1 ] && DETAILS="$DETAILS${D}$(fit "$*")${R}"$'\n'; return 0; }
warn() { [ "$FANCY" = 1 ] && { DETAILS="$DETAILS${YEL}$(fit "$*")${R}"$'\n'; WARNED=1; }; return 0; }
show_details() {
  [ -n "$DETAILS" ] || return 0
  printf '%s' "$DETAILS" | while IFS= read -r l; do printf '           %s\n' "$l"; done >&3
  DETAILS=""
}

say() { printf '%s\n' "$*"; }
step() { printf '\n== %s\n' "$*"; }
CUR="" DIE_MSG="" STEP_N=0 STEP_TOTAL=0 T0=0 LOG_MARK=0
die() {
  if [ "$FANCY" = 1 ]; then
    DIE_MSG="$*"
    [ -n "$CUR" ] || printf '\n  %s%s%s  %s\n\n' "$RED" "$G_BAD" "$R" "$*" >&2
    exit 1
  fi
  printf 'install-server.sh: %s\n' "$*" >&2; exit 1
}

# Run one step. Plain: the "== title" line and its output. Terminal: a spinner while its output goes to the log,
# then a check mark (or a cross, with the step's last lines and where the log is).
run_step() {  # $1 = plain title, $2 = title on a terminal, $3 = what the spinner says, $4 = the function
  if [ "$FANCY" = 0 ]; then step "$1"; "$4"; return; fi
  STEP_N=$((STEP_N + 1)); CUR="$3"; DETAILS=""; WARNED=0
  printf '\n== %s\n' "$1" >>"$LOGF"
  LOG_MARK=$(wc -l <"$LOGF" | tr -d ' ')
  T0=$(now_ms)
  spin_start "$3"
  "$4" >>"$LOGF" 2>&1
  local el; el=$(fmt_ms $(($(now_ms) - T0)))
  spin_stop
  if [ "$WARNED" = 1 ]; then step_line "$YEL" "$G_WARN" "$2" "$el"; else step_line "$GRN" "$G_OK" "$2" "$el"; fi
  show_details
  CUR=""
}

on_exit() {
  local rc=$?
  spin_stop
  if [ -n "$CUR" ]; then
    local el; el=$(fmt_ms $(($(now_ms) - T0)))
    step_line "$RED" "$G_BAD" "$(fit "$CUR")" "$el"
    [ -n "$DIE_MSG" ] && printf '           %s\n' "$DIE_MSG" >&3
    if [ "$rc" = 130 ]; then
      printf '           Stopped.\n' >&3
    elif [ -s "$LOGF" ] && [ "$(wc -l <"$LOGF" | tr -d ' ')" -gt "$LOG_MARK" ]; then
      printf '\n           %sLast lines of its output:%s\n' "$D" "$R" >&3
      tail -n +"$((LOG_MARK + 1))" "$LOGF" | grep -v '^$' | tail -n 12 | cut -c 1-$((WIDTH - 13)) |
        while IFS= read -r l; do printf '           %s%s %s%s\n' "$D" "$BX_V" "$l" "$R"; done >&3
    fi
    printf '\n           Full log: %s\n\n' "$(tilde "$LOGF")" >&3
  fi
  exit "$rc"
}
if [ "$FANCY" = 1 ]; then
  trap on_exit EXIT
  trap 'exit 130' INT TERM
fi

VERSION=""
if [ -f "$REPO/VERSION" ]; then VERSION=$(head -n 1 "$REPO/VERSION")
elif [ -f "$REPO/native/Cargo.toml" ]; then VERSION=$(sed -n 's/^version = "\([^"]*\)".*/\1/p' "$REPO/native/Cargo.toml" | head -n 1); fi

header() {  # $1 = what this run does
  local right="${VERSION:+$VERSION}" left="Switchboard" pad
  pad=$((WIDTH - 2 - 2 - ${#left} - 2 - ${#1} - ${#right}))
  [ "$pad" -lt 2 ] && pad=2
  printf '\n  %s%s%s %s%s%s  %s%s%s%s\n' "$ACC" "$G_MARK" "$R" "$B" "$left" "$R" "$1" "$(rep ' ' "$pad")" "$D$right" "$R" >&3
  printf '  %s%s%s\n' "$D" "$(rep "$BX_H" $((WIDTH - 2)))" "$R" >&3
}
section() { printf '\n  %s%s%s\n\n' "$B" "$*" "$R" >&3; }

OS=$(uname -s)
case "$OS" in
  Linux) OSN=linux ;;
  Darwin) OSN=mac ;;
  *) die "this script is for macOS and Linux. On Windows, run scripts\\install\\install-server.ps1 in PowerShell." ;;
esac

has_systemd() { [ "$OSN" = linux ] && command -v systemctl >/dev/null && systemctl --user show-environment >/dev/null 2>&1; }
MODE=plain
if [ "$NO_SERVICE" = 0 ]; then
  if [ "$OSN" = mac ]; then MODE=launchd; elif has_systemd; then MODE=systemd; fi
fi

if [ "$FANCY" = 1 ]; then
  mkdir -p "$(dirname "$LOGF")"
  : >"$LOGF"
fi

# ---------------------------------------------------------------- uninstall
uninstall() {
  [ -x "$BIN/switchboard" ] && "$BIN/switchboard" stop >/dev/null 2>&1 || true
  if [ "$OSN" = mac ]; then
    for p in server hub downloads; do
      launchctl bootout "gui/$(id -u)/com.switchboard.$p" 2>/dev/null || true
      rm -f "$HOME/Library/LaunchAgents/com.switchboard.$p.plist"
    done
  elif command -v systemctl >/dev/null; then
    for u in switchboard switchboard-hub switchboard-downloads; do
      systemctl --user disable --now "$u" >/dev/null 2>&1 || true
      rm -f "$HOME/.config/systemd/user/$u.service"
    done
    systemctl --user daemon-reload >/dev/null 2>&1 || true
  fi
  rm -rf "$APP" "$SHARE/downloads" "$SHARE/run" "$BIN/switchboard-hub" "$BIN/switchboard"
}
if [ "$UNINSTALL" = 1 ]; then
  if [ "$FANCY" = 0 ]; then
    uninstall
    say "Switchboard is removed from this machine. Your settings ($CFG) and history ($SHARE) are still there;"
    say "delete those two folders too if you don't want them."
    exit 0
  fi
  header "removing the server"
  STEP_TOTAL=1
  printf '\n' >&3
  run_step "Removing Switchboard" "Removed the server, hub, downloads and services" "Stopping and removing Switchboard" uninstall
  printf '\n  Your settings and history are kept, in case you install it again:\n' >&3
  printf '    %s%-29s%s %ssettings%s\n' "$ACC" "$(tilde "$CFG")" "$R" "$D" "$R" >&3
  printf '    %s%-29s%s %shistory and logs%s\n' "$ACC" "$(tilde "$SHARE")" "$R" "$D" "$R" >&3
  printf '  %sDelete those two folders too if you do not want them.%s\n\n' "$D" "$R" >&3
  exit 0
fi

[ -f "$REPO/server/server.py" ] && [ -f "$REPO/agent/sb.py" ] || die "run this from a full copy of the Switchboard repository (no server/ or agent/ next to it)."

# ---------------------------------------------------------------- what this machine has
PY=$(command -v python3 || true)
py_missing() {
  say "Python 3 is not installed. Switchboard's server is a Python program (3.9 or newer, nothing else to add)."
  case "$OSN" in
    mac) say "Install it with:  xcode-select --install   (or from https://www.python.org/downloads/)" ;;
    linux) if command -v apt-get >/dev/null; then say "Install it with:  sudo apt-get install -y python3"
           elif command -v dnf >/dev/null; then say "Install it with:  sudo dnf install -y python3"
           elif command -v pacman >/dev/null; then say "Install it with:  sudo pacman -S python"
           else say "Install python3 with your package manager."; fi ;;
  esac
  say "Then run this script again."
}
py_ok() { "$PY" -c 'import sys; sys.exit(0 if sys.version_info >= (3, 9) else 1)'; }
py_ver() { "$PY" -c 'import platform; print(platform.python_version())'; }

# The user's own time zone: the weekly usage days are counted in it.
TZNAME=""
if [ -n "${TZ:-}" ]; then TZNAME="$TZ"
elif [ -L /etc/localtime ]; then TZNAME=$(readlink /etc/localtime | sed 's#.*/zoneinfo/##')
elif [ -f /etc/timezone ]; then TZNAME=$(cat /etc/timezone)
fi
case "$TZNAME" in */*|UTC) ;; *) TZNAME=UTC ;; esac

# Where the hub comes from: given (--hub-binary), dist (a ready-made one next to the repo), build (Rust), none.
CARGO=$(command -v cargo || { [ -x "$HOME/.cargo/bin/cargo" ] && echo "$HOME/.cargo/bin/cargo"; } || true)
[ -f "$REPO/native/Cargo.toml" ] || CARGO=""  # a server kit has no Rust sources to build from
HUB_SRC=none
if [ "$NO_HUB" = 0 ]; then
  if [ -n "$HUB_BIN" ]; then HUB_SRC=given
  elif [ -x "$REPO/native/dist/switchboard-hub-$OSN" ]; then HUB_SRC=dist
  elif [ -n "$CARGO" ]; then HUB_SRC=build; fi
fi

if [ "$FANCY" = 0 ]; then
  step "Checking this machine"
  if [ -z "$PY" ]; then py_missing; exit 1; fi
  py_ok || die "Python $(py_ver) is too old. Switchboard needs 3.9 or newer."
  say "Python $(py_ver) at $PY"
  if command -v ssh >/dev/null; then say "ssh is here (for machines added over ssh)"; else say "No ssh client: machines can still link with a code, but not be added over ssh."; fi
  say "Time zone $TZNAME"
else
  header "server installer"
  section "Checking this machine"
  NOTES=""
  note() {  # a line under the table; a line that starts with a glyph starts a new note
    case "$*" in *"$G_WARN$R  "*|*"$G_INFO$R  "*) [ -n "$NOTES" ] && NOTES="$NOTES"$'\n' ;; esac
    NOTES="$NOTES$*"$'\n'
  }
  row() {  # $1 = ok|warn|bad|info, $2 = label, $3 = what was found
    local c g
    case "$1" in ok) c=$GRN g=$G_OK ;; warn) c=$YEL g=$G_WARN ;; bad) c=$RED g=$G_BAD ;; *) c=$D g=$G_INFO ;; esac
    if [ "$1" = ok ] || [ "$1" = info ]; then
      printf '  %s%s%s  %-14s%s%s%s\n' "$c" "$g" "$R" "$2" "$D" "$3" "$R" >&3
    else
      printf '  %s%s%s  %-14s%s\n' "$c" "$g" "$R" "$2" "$3" >&3
    fi
  }

  # System
  if [ "$OSN" = mac ]; then sysname="macOS $(sw_vers -productVersion 2>/dev/null || true)"
  elif [ -r /etc/os-release ]; then sysname=$(. /etc/os-release && echo "${PRETTY_NAME:-${NAME:-Linux}}")
  else sysname="Linux $(uname -r)"; fi
  row ok "System" "$sysname, $(uname -m)"

  # Python
  if [ -z "$PY" ]; then
    row bad "Python" "not installed (3.9 or newer is needed)"
    printf '\n' >&3
    py_missing | fold -s -w $((WIDTH - 4)) | sed 's/^/  /' >&3
    printf '\n' >&3
    exit 1
  fi
  if ! py_ok; then row bad "Python" "$(py_ver) is too old"; die "Python $(py_ver) is too old. Switchboard needs 3.9 or newer."; fi
  row ok "Python" "$(py_ver)"

  # Ports
  busy=$("$PY" - <<'PY'
import socket
busy = []
for p in (8130, 8131, 8132):
    s = socket.socket()
    # A server stopped a moment ago leaves connections waiting to close; only a listener counts as busy.
    s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    try:
        s.bind(('0.0.0.0', p))
    except OSError:
        busy.append(str(p))
    finally:
        s.close()
print(' '.join(busy))
PY
)
  if [ -z "$busy" ]; then row ok "Ports" "8130, 8131 and 8132 are free"
  elif [ -d "$APP" ]; then row ok "Ports" "${busy// /, } in use by the Switchboard already here; it restarts"
  else
    row warn "Ports" "${busy// /, } in use by another program"
    if [ "$OSN" = mac ]; then how="lsof -nP -iTCP:${busy%% *} -sTCP:LISTEN"; else how="ss -ltnp 'sport = :${busy%% *}'"; fi
    note "${YEL}${G_WARN}${R}  Another program listens on ${busy// /, }, which Switchboard needs."
    note "   See which one with ${ACC}$how${R}"
  fi

  # Services, and staying up after logout
  case "$MODE" in
    systemd) row ok "Services" "systemd user services" ;;
    launchd) row ok "Services" "LaunchAgents, started when you log in" ;;
    *) if [ "$NO_SERVICE" = 1 ]; then row info "Services" "background processes (--no-service)"
       elif [ "$OSN" = linux ] && command -v systemctl >/dev/null; then
         row warn "Services" "no systemd user session; background processes"
         note "${YEL}${G_WARN}${R}  systemd is here, but not for this login (often after su or sudo -u)."
         note "   Background processes stop at reboot. For services, log in as $(id -un)"
         note "   directly (over ssh or at the console) and run this again."
       else row info "Services" "background processes (no systemd here); they stop at reboot"; fi ;;
  esac
  if [ "$MODE" = systemd ]; then
    if loginctl show-user "$(id -un)" -p Linger 2>/dev/null | grep -q yes; then
      row ok "After logout" "keeps running (linger is on)"
    else
      row warn "After logout" "stops: linger is off"
      note "${YEL}${G_WARN}${R}  systemd stops your user services when you log out, and a headless box"
      note "   usually has nobody logged in. To keep Switchboard running, run this"
      note "   once (it asks for your password):"
      note "     ${ACC}sudo loginctl enable-linger $(id -un)${R}"
    fi
  fi

  # Disk
  kb=$(df -Pk "$HOME" 2>/dev/null | awk 'NR == 2 {print $4}' || true)
  case "$kb" in
    ''|*[!0-9]*) row info "Disk" "free space not known" ;;
    *) human=$(awk -v k="$kb" 'BEGIN { if (k >= 1048576) printf "%.1f GB", k / 1048576; else printf "%d MB", k / 1024 }')
       need=204800; [ "$HUB_SRC" = build ] && need=2097152
       if [ "$kb" -lt "$need" ]; then
         row warn "Disk" "$human free"
         note "${YEL}${G_WARN}${R}  Space is low. The server needs little; building the hub needs about 2 GB."
       else row ok "Disk" "$human free in your home folder"; fi ;;
  esac

  # A laptop?
  battery=0
  if [ "$OSN" = mac ]; then pmset -g batt 2>/dev/null | grep -q InternalBattery && battery=1
  else for t in /sys/class/power_supply/*/type; do [ -r "$t" ] && [ "$(cat "$t" 2>/dev/null)" = Battery ] && battery=1; done; fi
  if [ "$battery" = 1 ]; then
    row warn "Power" "runs on a battery: this looks like a laptop"
    note "${D}${G_INFO}${R}  That works. The server is happiest on a computer that stays on, though:"
    note "   while this one sleeps or has its lid closed, your phone and other"
    note "   machines can't reach the board."
  else row ok "Power" "no battery, so it can stay on"; fi

  # The hub
  case "$HUB_SRC" in
    given) row ok "Hub" "the one from --hub-binary" ;;
    dist) row ok "Hub" "a ready-made one" ;;
    build) row ok "Hub" "builds with Rust (a few minutes the first time)" ;;
    *) if [ "$NO_HUB" = 1 ]; then row info "Hub" "skipped (--no-hub)"
       else
         row warn "Hub" "no Rust and no ready-made hub"
         note "${YEL}${G_WARN}${R}  Without the hub there are no terminals, and machines can't link with a"
         note "   code (ssh only). To add it later, install Rust from ${ACC}https://rustup.rs${R}"
         note "   and run this again."
       fi ;;
  esac

  if command -v ssh >/dev/null; then row ok "ssh" "here, for machines added over ssh"
  else row info "ssh" "not here: machines link with a code instead"; fi
  row info "Time zone" "$TZNAME"

  if [ -n "$NOTES" ]; then printf '\n' >&3; printf '%s' "$NOTES" | sed 's/^/  /' >&3; fi
fi

# ---------------------------------------------------------------- the server
s_server() {
  mkdir -p "$SHARE" "$BIN" "$CFG"
  NEW="$SHARE/app.new"
  rm -rf "$NEW"
  mkdir -p "$NEW/agent" "$NEW/install"
  cp -R "$REPO/server" "$NEW/server"
  find "$NEW/server" -name '__pycache__' -prune -exec rm -rf {} +
  cp "$REPO/agent/sb.py" "$NEW/agent/sb.py"
  cp "$HERE/setup.py" "$NEW/install/setup.py"
  # The terminal schemes the server checks a saved scheme against (server.py SCHEMES_JSON): without it every save
  # falls back to the default scheme.
  SCH=native/crates/sb-core/src/term/schemes.json
  mkdir -p "$NEW/$(dirname "$SCH")"
  cp "$REPO/$SCH" "$NEW/$SCH"
  # What Settings > Server shows as this server's version (serverctl.py).
  if [ -f "$REPO/VERSION" ]; then cp "$REPO/VERSION" "$NEW/VERSION"; elif [ -n "$VERSION" ]; then printf '%s\n' "$VERSION" > "$NEW/VERSION"; fi
  rm -rf "$SHARE/app.old"
  [ -d "$APP" ] && mv "$APP" "$SHARE/app.old"
  mv "$NEW" "$APP"
  rm -rf "$SHARE/app.old"
  install -m 755 "$HERE/switchboard" "$BIN/switchboard"
  printf 'export SWITCHBOARD_TZ=%s\n' "$TZNAME" > "$SHARE/env"
  say "Server in $APP"
  detail "$(tilde "$APP")"
}

# ---------------------------------------------------------------- the hub and the nodes
build() {  # $1 = crate; prints the path of the binary it built
  ( cd "$REPO/native" && "$CARGO" build --release --locked -q -p "$1" >&2 ) || return 1
  echo "$REPO/native/target/release/$2"
}
s_hub() {
  if [ "$HUB_SRC" = dist ]; then HUB_BIN="$REPO/native/dist/switchboard-hub-$OSN"; fi
  if [ -z "$HUB_BIN" ] && [ -n "$CARGO" ]; then
    say "Building switchboard-hub with Rust (a few minutes the first time)..."
    HUB_BIN=$(build sb-hub switchboard-hub) || die "the hub did not build. Run again with --no-hub to install without terminals."
  fi
  if [ -n "$HUB_BIN" ]; then
    [ -f "$HUB_BIN" ] || die "there is no hub at $HUB_BIN."
    "$HUB_BIN" --version >/dev/null 2>&1 || die "$HUB_BIN does not run on this machine (is it built for another system?)."
    install -m 755 "$HUB_BIN" "$BIN/switchboard-hub.new"
    mv -f "$BIN/switchboard-hub.new" "$BIN/switchboard-hub"
    say "Hub: $("$BIN/switchboard-hub" --version)"
    detail "$("$BIN/switchboard-hub" --version), terminals and linking with a code"
  else
    NO_HUB=1
    say "Rust is not installed, so the hub can't be built here, and there is no ready-made one."
    say "Switchboard works without it, but with no terminals and no linking machines with a code (ssh only)."
    say "To add it later: install Rust from https://rustup.rs, then run this script again."
    warn "Skipped, so machines are added over ssh"
  fi
}

s_downloads() {
  mkdir -p "$DL.new"
  cp "$REPO/agent/sb.py" "$DL.new/sb.py"
  for f in switchboard-node-mac switchboard-node-linux switchboard-node.exe; do
    for d in "$NODE_DIR" "$REPO/native/dist"; do
      [ -n "$d" ] && [ -s "$d/$f" ] && { cp "$d/$f" "$DL.new/$f"; break; }
    done
  done
  own="switchboard-node-$OSN"
  if [ ! -s "$DL.new/$own" ] && [ "$NO_HUB" = 0 ] && [ -n "$CARGO" ]; then
    say "Building switchboard-node for this machine's kind ($OSN)..."
    b=$(build sb-node switchboard-node) && cp "$b" "$DL.new/$own"
  fi
  chmod 644 "$DL.new"/*
  ( cd "$DL.new" && if command -v sha256sum >/dev/null; then sha256sum -- * > SHA256SUMS; else shasum -a 256 -- * > SHA256SUMS; fi )
  rm -rf "$DL.old"; [ -d "$DL" ] && mv "$DL" "$DL.old"; mv "$DL.new" "$DL"; rm -rf "$DL.old"
  for f in install-agent.sh install-agent.ps1; do [ -f "$HERE/$f" ] && install -m 644 "$HERE/$f" "$SHARE/downloads/$f"; done
  local ready="" missing=""
  for f in switchboard-node-mac switchboard-node-linux switchboard-node.exe; do
    if [ -s "$DL/$f" ]; then say "  ready: $f"; else say "  missing: $f (Add a machine can't link that kind of machine with a code yet; see docs/install.md)"; fi
    case "$f" in *mac) k=Mac ;; *linux) k=Linux ;; *) k=Windows ;; esac
    if [ -s "$DL/$f" ]; then ready="$ready${ready:+, }$k"; else missing="$missing${missing:+, }$k"; fi
  done
  detail "The helper${ready:+, and linking with a code for $ready}"
  [ -z "$missing" ] || warn "No switchboard-node for $missing: add those over ssh"
}

# ---------------------------------------------------------------- services
exec_of() {
  case "$1" in
    server) echo "$PY|-u|$APP/server/server.py" ;;
    hub) echo "$BIN/switchboard-hub" ;;
    downloads) echo "$PY|-m|http.server|8131|--directory|$SHARE/downloads" ;;
  esac
}
desc_of() {
  case "$1" in
    server) echo "Switchboard (Claude Code sessions across your machines)" ;;
    hub) echo "Switchboard hub (terminals and linked machines)" ;;
    downloads) echo "Switchboard downloads for Add a machine" ;;
  esac
}
s_services() {
  "$BIN/switchboard" stop >/dev/null 2>&1 || true
  PARTS="server downloads"; [ "$NO_HUB" = 0 ] && PARTS="server hub downloads"
  if [ "$MODE" = systemd ]; then
    U="$HOME/.config/systemd/user"; mkdir -p "$U"
    for p in server hub downloads; do
      n=switchboard; [ "$p" = server ] || n="switchboard-$p"
      if ! echo "$PARTS" | grep -qw "$p"; then systemctl --user disable --now "$n" >/dev/null 2>&1 || true; rm -f "$U/$n.service"; continue; fi
      {
        printf '[Unit]\nDescription=%s\nAfter=network-online.target\nWants=network-online.target\n\n[Service]\n' "$(desc_of "$p")"
        printf 'WorkingDirectory=%s\nExecStart=%s\nEnvironment=SWITCHBOARD_TZ=%s\n' "$APP" "$(exec_of "$p" | sed 's/|/ /g')" "$TZNAME"
        [ "$p" = hub ] && printf 'Environment=SWITCHBOARD_HUB_LISTEN=0.0.0.0:8132\n'
        printf 'Restart=always\nRestartSec=3\nNoNewPrivileges=yes\n\n[Install]\nWantedBy=default.target\n'
      } > "$U/$n.service"
    done
    systemctl --user daemon-reload
    for p in $PARTS; do n=switchboard; [ "$p" = server ] || n="switchboard-$p"; systemctl --user enable "$n" >/dev/null 2>&1; done
    detail "$(for p in $PARTS; do n=switchboard; [ "$p" = server ] || n="switchboard-$p"; printf '%s ' "$n"; done | sed 's/ $//; s/ /, /g'), in systemctl --user"
  elif [ "$MODE" = launchd ]; then
    LA="$HOME/Library/LaunchAgents"; LOGS="$HOME/Library/Logs/Switchboard"; mkdir -p "$LA" "$LOGS"
    for p in server hub downloads; do
      f="$LA/com.switchboard.$p.plist"
      if ! echo "$PARTS" | grep -qw "$p"; then launchctl bootout "gui/$(id -u)/com.switchboard.$p" 2>/dev/null || true; rm -f "$f"; continue; fi
      args=$(exec_of "$p" | tr '|' '\n' | sed 's#.*#    <string>&</string>#')
      cat > "$f" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>Label</key><string>com.switchboard.$p</string>
  <key>ProgramArguments</key><array>
$args
  </array>
  <key>WorkingDirectory</key><string>$APP</string>
  <key>EnvironmentVariables</key><dict>
    <key>SWITCHBOARD_TZ</key><string>$TZNAME</string>
    <key>PATH</key><string>$BIN:/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin</string>
  </dict>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><true/>
  <key>StandardOutPath</key><string>$LOGS/$p.log</string>
  <key>StandardErrorPath</key><string>$LOGS/$p.log</string>
</dict></plist>
EOF
    done
    detail "com.switchboard.* in ~/Library/LaunchAgents"
  else
    detail "Started and stopped by the switchboard command"
  fi
}

ok=""
s_start() {
  "$BIN/switchboard" start >/dev/null

  # The server answers within a few seconds of starting.
  for _ in $(seq 1 30); do
    if "$PY" -c 'import urllib.request; urllib.request.urlopen("http://127.0.0.1:8130/api/summary", timeout=2).close()' 2>/dev/null; then ok=1; break; fi
    sleep 1
  done
  "$BIN/switchboard" status
  # On a terminal a failure shows the end of this step's output, so put the server's own last lines there.
  [ -n "$ok" ] || [ "$FANCY" = 0 ] || "$BIN/switchboard" logs 8 2>&1 | grep -v '^== ' || true
  [ -n "$ok" ] || die "the server did not start. See: switchboard logs"
  [ "$MODE" = systemd ] && ! loginctl show-user "$(id -un)" -p Linger 2>/dev/null | grep -q yes && \
    say "Tip: to keep Switchboard running while you're logged out, run: sudo loginctl enable-linger $(id -un)"
  case ":$PATH:" in *":$BIN:"*) ;; *) say "Tip: add $BIN to your PATH to use the switchboard command." ;; esac
  detail "$("$BIN/switchboard" status | sed -n 's/^  \([a-z]*\) .* running$/\1/p' | tr '\n' ' ' | sed 's/ $//; s/ /, /g') answering"
}

# ---------------------------------------------------------------- the steps
if [ "$FANCY" = 1 ]; then
  STEP_TOTAL=3; [ "$NO_HUB" = 0 ] && STEP_TOTAL=4; [ "$NO_START" = 0 ] && STEP_TOTAL=$((STEP_TOTAL + 1))
  section "Installing"
  case "$HUB_SRC" in build) hub_says="Building the hub with Rust (a few minutes the first time)" ;; *) hub_says="Putting the hub in place" ;; esac
  case "$MODE" in systemd) svc="systemd user services" ;; launchd) svc="LaunchAgents" ;; *) svc="background processes" ;; esac
fi
run_step "Putting the server in place" "Server in place" "Copying the server" s_server
if [ "$NO_HUB" = 0 ]; then
  run_step "The hub" "The hub" "${hub_says:-}" s_hub
fi
run_step "Downloads for Add a machine" "Downloads for Add a machine" "Getting the downloads ready" s_downloads

if [ "$FANCY" = 0 ]; then
  step "Running it ($MODE)"
  s_services
  if [ "$NO_START" = 1 ]; then
    say "Installed. Start it with: switchboard start"
    exit 0
  fi
  s_start
else
  run_step "Running it ($MODE)" "Set up as $svc" "Setting up $svc" s_services
  [ "$NO_START" = 1 ] || run_step "Starting" "Started" "Starting Switchboard and waiting for it to answer" s_start
fi

# ---------------------------------------------------------------- first-run setup
SETUP_RAN=0
if [ "$NO_START" = 0 ] && [ "$NO_SETUP" = 0 ] && [ "$YES" = 0 ] && [ -t 0 ]; then
  if [ "$FANCY" = 1 ]; then
    section "First-run setup"
    "$PY" "$APP/install/setup.py" || printf '\n  Setup stopped. Run it again any time with: %sswitchboard setup%s\n' "$ACC" "$R" >&3
    SETUP_RAN=1
  else
    step "First-run setup"
    "$PY" "$APP/install/setup.py" || say "Setup stopped. Run it again any time with: switchboard setup"
  fi
elif [ "$FANCY" = 0 ]; then
  say ""
  say "Next: run 'switchboard setup' to pick the server's address and add your machines."
fi
[ "$FANCY" = 1 ] || exit 0

# ---------------------------------------------------------------- the summary (terminal only)
# Addresses: the one picked in setup first, then this machine on the network, its name, and Tailscale.
ADDRS=$("$PY" - "$APP/install" "$CFG/setup.json" "$OSN" <<'PY' 2>/dev/null || true
import json, os, shutil, socket, subprocess, sys
sys.path.insert(0, sys.argv[1])
import setup
out, seen = [], set()
def add(a, note):
    if a and a not in seen:
        seen.add(a); out.append((a, note))
try:
    add(json.load(open(sys.argv[2])).get('address'), 'picked in setup')
except Exception:
    pass
add(setup.lan_ip(), 'this machine on your network')
host = (socket.gethostname() or '').split('.')[0].lower()
avahi = sys.argv[3] == 'mac' or subprocess.run(['sh', '-c', 'pgrep -x avahi-daemon'], capture_output=True).returncode == 0
if host and host != 'localhost':
    add(f'{host}.local' if avahi else host, 'its name on your network' if avahi else 'its name, where your network knows it')
ips, dns = setup.tailscale()
add(dns, 'Tailscale, works away from home')
for x in ips[:1]:
    add(x, 'Tailscale address')
for a, n in out:
    print(f'{a}\t{n}')
PY
)
BONJOUR=1
# The kit carries python-zeroconf in server/vendor; the server falls back to it the same way.
if [ "$OSN" = linux ] && ! command -v avahi-publish >/dev/null \
  && ! "$PY" -c 'import sys; sys.path.append(sys.argv[1]); import zeroconf' "$APP/server/vendor" 2>/dev/null; then BONJOUR=0; fi

# A box with a border. Lines may carry colors; the padding counts only what shows. When a line is wider than the
# terminal, the box is drawn open on the right instead.
vis() { printf '%s' "$1" | sed $'s/\033\\[[0-9;]*m//g'; }
CARD=""
cl() { CARD="$CARD$*"$'\n'; }
card() {
  local inner=$((WIDTH - 6)) l v open=0
  while IFS= read -r l; do v=$(vis "$l"); [ "${#v}" -gt "$inner" ] && inner=${#v}; done <<<"${CARD%$'\n'}"
  [ $((inner + 6)) -gt "$(cols)" ] && open=1
  printf '\n  %s%s%s%s\n' "$D" "$BX_TL" "$(rep "$BX_H" $((inner + 2)))" "$([ "$open" = 0 ] && printf '%s' "$BX_TR")$R" >&3
  while IFS= read -r l; do
    v=$(vis "$l")
    if [ "$open" = 1 ]; then printf '  %s%s%s %s%s\n' "$D" "$BX_V" "$R" "$l" "$R" >&3
    else printf '  %s%s%s %s%s%s %s%s%s\n' "$D" "$BX_V" "$R" "$l" "$R" "$(rep ' ' $((inner - ${#v})))" "$D" "$BX_V" "$R" >&3; fi
  done <<<"${CARD%$'\n'}"
  printf '  %s%s%s%s\n' "$D" "$BX_BL" "$(rep "$BX_H" $((inner + 2)))" "$([ "$open" = 0 ] && printf '%s' "$BX_BR")$R" >&3
}

if [ "$NO_START" = 1 ]; then
  cl "$B$GRN$G_OK$R  ${B}Switchboard is installed$R"
  cl ""
  cl "   Start it with ${ACC}switchboard start$R"
else
  parts="server :8130"; [ "$NO_HUB" = 0 ] && parts="$parts  $G_INFO  hub :8132"; parts="$parts  $G_INFO  downloads :8131"
  cl "$B$GRN$G_OK$R  ${B}Switchboard is running$R"
  cl "   $D$parts$R"
fi
cl ""
if [ -n "$ADDRS" ]; then
  cl "${B}Open it at$R"
  aw=0
  while IFS=$'\t' read -r a _; do u="http://$a:8130"; [ "${#u}" -gt "$aw" ] && aw=${#u}; done <<<"$ADDRS"
  while IFS=$'\t' read -r a n; do
    u="http://$a:8130"
    cl "   $ACC$u$R$(rep ' ' $((aw - ${#u} + 3)))$D$n$R"
  done <<<"$ADDRS"
  cl ""
fi
cl "${B}Phones and the Mac app$R"
if [ "$BONJOUR" = 1 ]; then
  cl "   On this network they find the server by themselves (Bonjour)."
  cl "   Away from home, type the Tailscale name in Settings, Server."
else
  cl "   They find a server by themselves through Bonjour, which needs"
  cl "   ${ACC}avahi-utils$R or ${ACC}python3-zeroconf$R here. Until then, type the"
  cl "   address in Settings, Server."
fi
cl ""
cl "${B}Everyday commands$R"
cl "   ${ACC}switchboard status$R    ${D}is everything running?$R"
cl "   ${ACC}switchboard logs$R      ${D}the last lines of each part's log$R"
cl "   ${ACC}switchboard stop$R      ${D}stop it; switchboard start runs it again$R"
if [ "$SETUP_RAN" = 0 ]; then
  cl "   ${ACC}switchboard setup$R     next: pick the address, add your machines"
else
  cl "   ${ACC}switchboard setup$R     ${D}change the address, machines and folders$R"
fi
case ":$PATH:" in *":$BIN:"*) ;; *)
  cl ""
  cl "$YEL$G_WARN$R  Add $(tilde "$BIN") to your PATH to use the switchboard command."
esac
card

# A QR code of the first address, to open the board on a phone. Only when qrencode is already here.
first=$(printf '%s\n' "$ADDRS" | head -n 1 | cut -f 1)
if [ -n "$first" ] && [ "$NO_START" = 0 ] && [ "$UTF" = 1 ] && command -v qrencode >/dev/null && [ "$(cols)" -ge 40 ]; then
  printf '\n  %sScan to open http://%s:8130 on your phone%s\n' "$D" "$first" "$R" >&3
  qrencode -t ANSIUTF8 -m 2 "http://$first:8130" 2>/dev/null | sed 's/^/  /' >&3 || true
fi
printf '\n  %sThe full install log is in %s%s\n\n' "$D" "$(tilde "$LOGF")" "$R" >&3
