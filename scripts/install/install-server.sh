#!/usr/bin/env bash
# Install the Switchboard server on this machine (macOS or Linux), for the person who runs it. No sudo.
#
#   scripts/install/install-server.sh            # from a copy of the repository
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
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "$HERE/../.." && pwd)"
SHARE="$HOME/.local/share/switchboard"
APP="$SHARE/app"
BIN="$HOME/.local/bin"
CFG="$HOME/.config/switchboard"
DL="$SHARE/downloads/node"

HUB_BIN="" NODE_DIR="" NO_HUB=0 NO_SERVICE=0 NO_START=0 NO_SETUP=0 YES=0 UNINSTALL=0
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
    -h|--help) sed -n '2,/^set -e/p' "$0" | sed '$d; s/^# \{0,1\}//'; exit 0 ;;
    *) echo "install-server.sh: unknown option $1 (see --help)" >&2; exit 2 ;;
  esac
  shift
done

say() { printf '%s\n' "$*"; }
step() { printf '\n== %s\n' "$*"; }
die() { printf 'install-server.sh: %s\n' "$*" >&2; exit 1; }

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

# ---------------------------------------------------------------- uninstall
if [ "$UNINSTALL" = 1 ]; then
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
  say "Switchboard is removed from this machine. Your settings ($CFG) and history ($SHARE) are still there;"
  say "delete those two folders too if you don't want them."
  exit 0
fi

[ -f "$REPO/server/server.py" ] && [ -f "$REPO/agent/sb.py" ] || die "run this from a full copy of the Switchboard repository (no server/ or agent/ next to it)."

# ---------------------------------------------------------------- what this machine has
step "Checking this machine"
PY=$(command -v python3 || true)
if [ -z "$PY" ]; then
  say "Python 3 is not installed. Switchboard's server is a Python program (3.9 or newer, nothing else to add)."
  case "$OSN" in
    mac) say "Install it with:  xcode-select --install   (or from https://www.python.org/downloads/)" ;;
    linux) if command -v apt-get >/dev/null; then say "Install it with:  sudo apt-get install -y python3"
           elif command -v dnf >/dev/null; then say "Install it with:  sudo dnf install -y python3"
           elif command -v pacman >/dev/null; then say "Install it with:  sudo pacman -S python"
           else say "Install python3 with your package manager."; fi ;;
  esac
  say "Then run this script again."
  exit 1
fi
"$PY" -c 'import sys; sys.exit(0 if sys.version_info >= (3, 9) else 1)' || die "Python $("$PY" -c 'import platform; print(platform.python_version())') is too old. Switchboard needs 3.9 or newer."
say "Python $("$PY" -c 'import platform; print(platform.python_version())') at $PY"
command -v ssh >/dev/null && say "ssh is here (for machines added over ssh)" || say "No ssh client: machines can still link with a code, but not be added over ssh."

# The user's own time zone: the weekly usage days are counted in it.
TZNAME=""
if [ -n "${TZ:-}" ]; then TZNAME="$TZ"
elif [ -L /etc/localtime ]; then TZNAME=$(readlink /etc/localtime | sed 's#.*/zoneinfo/##')
elif [ -f /etc/timezone ]; then TZNAME=$(cat /etc/timezone)
fi
case "$TZNAME" in */*|UTC) ;; *) TZNAME=UTC ;; esac
say "Time zone $TZNAME"

# ---------------------------------------------------------------- the server
step "Putting the server in place"
mkdir -p "$SHARE" "$BIN" "$CFG"
NEW="$SHARE/app.new"
rm -rf "$NEW"
mkdir -p "$NEW/agent" "$NEW/install"
cp -R "$REPO/server" "$NEW/server"
find "$NEW/server" -name '__pycache__' -prune -exec rm -rf {} +
cp "$REPO/agent/sb.py" "$NEW/agent/sb.py"
cp "$HERE/setup.py" "$NEW/install/setup.py"
rm -rf "$SHARE/app.old"
[ -d "$APP" ] && mv "$APP" "$SHARE/app.old"
mv "$NEW" "$APP"
rm -rf "$SHARE/app.old"
install -m 755 "$HERE/switchboard" "$BIN/switchboard"
printf 'export SWITCHBOARD_TZ=%s\n' "$TZNAME" > "$SHARE/env"
say "Server in $APP"

# ---------------------------------------------------------------- the hub and the nodes
CARGO=$(command -v cargo || { [ -x "$HOME/.cargo/bin/cargo" ] && echo "$HOME/.cargo/bin/cargo"; } || true)
BUILT=""
build() {  # $1 = crate; prints the path of the binary it built
  ( cd "$REPO/native" && "$CARGO" build --release --locked -q -p "$1" >&2 ) || return 1
  echo "$REPO/native/target/release/$2"
}
if [ "$NO_HUB" = 0 ]; then
  step "The hub"
  if [ -z "$HUB_BIN" ] && [ -x "$REPO/native/dist/switchboard-hub-$OSN" ]; then HUB_BIN="$REPO/native/dist/switchboard-hub-$OSN"; fi
  if [ -z "$HUB_BIN" ] && [ -n "$CARGO" ]; then
    say "Building switchboard-hub with Rust (a few minutes the first time)..."
    HUB_BIN=$(build sb-hub switchboard-hub) || die "the hub did not build. Run again with --no-hub to install without terminals."
    BUILT=1
  fi
  if [ -n "$HUB_BIN" ]; then
    [ -f "$HUB_BIN" ] || die "there is no hub at $HUB_BIN."
    "$HUB_BIN" --version >/dev/null 2>&1 || die "$HUB_BIN does not run on this machine (is it built for another system?)."
    install -m 755 "$HUB_BIN" "$BIN/switchboard-hub.new"
    mv -f "$BIN/switchboard-hub.new" "$BIN/switchboard-hub"
    say "Hub: $("$BIN/switchboard-hub" --version)"
  else
    NO_HUB=1
    say "Rust is not installed, so the hub can't be built here, and there is no ready-made one."
    say "Switchboard works without it, but with no terminals and no linking machines with a code (ssh only)."
    say "To add it later: install Rust from https://rustup.rs, then run this script again."
  fi
fi

step "Downloads for Add a machine"
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
for f in switchboard-node-mac switchboard-node-linux switchboard-node.exe; do
  [ -s "$DL/$f" ] && say "  ready: $f" || say "  missing: $f (Add a machine can't link that kind of machine with a code yet; see docs/install.md)"
done

# ---------------------------------------------------------------- services
step "Running it ($MODE)"
"$BIN/switchboard" stop >/dev/null 2>&1 || true
PARTS="server downloads"; [ "$NO_HUB" = 0 ] && PARTS="server hub downloads"
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
fi

if [ "$NO_START" = 1 ]; then
  say "Installed. Start it with: switchboard start"
  exit 0
fi
"$BIN/switchboard" start >/dev/null

# The server answers within a few seconds of starting.
ok=""
for _ in $(seq 1 30); do
  if "$PY" -c 'import urllib.request; urllib.request.urlopen("http://127.0.0.1:8130/api/summary", timeout=2).close()' 2>/dev/null; then ok=1; break; fi
  sleep 1
done
"$BIN/switchboard" status
[ -n "$ok" ] || die "the server did not start. See: switchboard logs"
[ "$MODE" = systemd ] && ! loginctl show-user "$(id -un)" -p Linger 2>/dev/null | grep -q yes && \
  say "Tip: to keep Switchboard running while you're logged out, run: sudo loginctl enable-linger $(id -un)"
case ":$PATH:" in *":$BIN:"*) ;; *) say "Tip: add $BIN to your PATH to use the switchboard command." ;; esac

# ---------------------------------------------------------------- first-run setup
if [ "$NO_SETUP" = 0 ] && [ "$YES" = 0 ] && [ -t 0 ]; then
  step "First-run setup"
  "$PY" "$APP/install/setup.py" || say "Setup stopped. Run it again any time with: switchboard setup"
else
  say ""
  say "Next: run 'switchboard setup' to pick the server's address and add your machines."
fi
