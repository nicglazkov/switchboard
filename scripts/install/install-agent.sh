#!/usr/bin/env bash
# Install the Switchboard agent on a machine that runs Claude Code (macOS or Linux), and link it to your server.
# No sudo. Usually pasted from the server's setup, like this:
#
#   curl -fsSL http://<server>:8131/install-agent.sh | bash -s -- --server <server> --code <code>
#
# Options:
#   --server ADDR     the Switchboard server (an address or name; its downloads are at http://ADDR:8131)
#   --code CODE       link with this code from Add a machine (needs the hub; no ssh into this machine)
#   --ssh-key "KEY"   let the server in over ssh with this public key, limited to the agent (no shell)
#   --name ID         this machine's id (lowercase letters and digits; default: from its name)
#   --label NAME      this machine's name in Switchboard (default: its own name)
#   --sb PATH         use this sb.py instead of downloading one
#   --node PATH       use this switchboard-node instead of downloading one
#   --update          replace an agent that is already here (by default it is kept)
set -euo pipefail

SERVER="" CODE="" SSHKEY="" NAME="" LABEL="" SB_SRC="" NODE_SRC="" UPDATE=0
while [ $# -gt 0 ]; do
  case "$1" in
    --server) SERVER="$2"; shift ;;
    --code) CODE="$2"; shift ;;
    --ssh-key) SSHKEY="$2"; shift ;;
    --name) NAME="$2"; shift ;;
    --label) LABEL="$2"; shift ;;
    --sb) SB_SRC="$2"; shift ;;
    --node) NODE_SRC="$2"; shift ;;
    --update) UPDATE=1 ;;
    -h|--help) sed -n '2,/^set -e/p' "$0" | sed '$d; s/^# \{0,1\}//'; exit 0 ;;
    *) echo "install-agent.sh: unknown option $1 (see --help)" >&2; exit 2 ;;
  esac
  shift
done

say() { printf '%s\n' "$*"; }
die() { printf 'install-agent.sh: %s\n' "$*" >&2; exit 1; }

case "$(uname -s)" in
  Linux) OSN=linux ;;
  Darwin) OSN=mac ;;
  *) die "this script is for macOS and Linux. On Windows, use install-agent.ps1 in PowerShell." ;;
esac
[ -z "$SERVER" ] || printf '%s' "$SERVER" | grep -Eq '^[A-Za-z0-9][A-Za-z0-9.:_-]*$|^\[[0-9A-Fa-f:.]+\]$' || die "--server takes an address or a name, like 10.0.0.5 or studio.local"
[ -z "$CODE" ] || printf '%s' "$CODE" | grep -Eq '^[A-Za-z0-9-]{4,20}$' || die "--code takes the code shown in Add a machine"
[ -z "$CODE$SSHKEY" ] || [ -n "$SERVER" ] || [ -n "$NODE_SRC" ] || die "--code needs --server (where to link)"

PY=$(command -v python3 || true)
if [ -z "$PY" ]; then
  say "Python 3 is not installed. The agent is a Python program (3.9 or newer)."
  case "$OSN" in
    mac) say "Install it with:  xcode-select --install" ;;
    linux) if command -v apt-get >/dev/null; then say "Install it with:  sudo apt-get install -y python3"
           elif command -v dnf >/dev/null; then say "Install it with:  sudo dnf install -y python3"
           else say "Install python3 with your package manager."; fi ;;
  esac
  exit 1
fi
"$PY" -c 'import sys; sys.exit(0 if sys.version_info >= (3, 9) else 1)' || die "Python 3.9 or newer is needed."

fetch() {  # $1 = url, $2 = file; curl, wget, or Python when neither is here
  if command -v curl >/dev/null; then curl -fsSL "$1" -o "$2"
  elif command -v wget >/dev/null; then wget -q "$1" -O "$2"
  else "$PY" -c 'import sys, urllib.request; open(sys.argv[2], "wb").write(urllib.request.urlopen(sys.argv[1], timeout=60).read())' "$1" "$2"
  fi
}
DL=""
[ -n "$SERVER" ] && DL="http://$SERVER:8131/node"
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
sums_ok() {  # $1 = file, $2 = its name in SHA256SUMS
  [ -n "$DL" ] || return 0
  fetch "$DL/SHA256SUMS" "$TMP/SHA256SUMS" 2>/dev/null || { say "(no SHA256SUMS on the server; not checked)"; return 0; }
  local want got
  want=$(awk -v n="$2" '$2 == n || $2 == "*"n {print $1}' "$TMP/SHA256SUMS")
  got=$("$PY" -c 'import hashlib, sys; print(hashlib.sha256(open(sys.argv[1], "rb").read()).hexdigest())' "$1")
  [ -z "$want" ] || [ "$want" = "$got" ]
}

if ! command -v claude >/dev/null && [ ! -x "$HOME/.local/bin/claude" ] && [ ! -x "$HOME/.claude/local/claude" ]; then
  say "Claude Code is not installed here yet. Switchboard runs it, so install it too:"
  say "  curl -fsSL https://claude.ai/install.sh | bash"
  say "The agent goes in now and starts working once Claude Code is here."
fi

# ---------------------------------------------------------------- the agent (sb.py)
SBDIR="$HOME/.claude/switchboard"
SB="$SBDIR/sb.py"
mkdir -p "$SBDIR"
if [ -f "$SB" ] && [ "$UPDATE" = 0 ]; then
  say "The agent is here already ($SB); keeping it. Use --update to replace it."
else
  if [ -n "$SB_SRC" ]; then cp "$SB_SRC" "$TMP/sb.py"
  elif [ "$(basename "$0")" = install-agent.sh ] && [ -f "$(dirname "$0")/../../agent/sb.py" ]; then cp "$(dirname "$0")/../../agent/sb.py" "$TMP/sb.py"
  elif [ -n "$DL" ]; then fetch "$DL/sb.py" "$TMP/sb.py" || die "could not download the agent from $DL/sb.py. Is the server running?"
  else die "where is the agent? Pass --server (to download it) or --sb PATH."
  fi
  sums_ok "$TMP/sb.py" sb.py || die "the agent from $DL did not match its checksum; nothing was changed."
  "$PY" -m py_compile "$TMP/sb.py" || die "the agent did not compile with $PY."
  cp "$TMP/sb.py" "$SB.new" && mv -f "$SB.new" "$SB"
  "$PY" "$SB" install || die "the agent's installer failed (see above)."
  say "Agent installed: Claude Code's hooks, status line and the switchboard MCP server are set up in ~/.claude."
fi

# ---------------------------------------------------------------- link with a code
has_systemd() { [ "$OSN" = linux ] && command -v systemctl >/dev/null && systemctl --user show-environment >/dev/null 2>&1; }
if [ -n "$CODE" ]; then
  NODE="$TMP/switchboard-node"
  if [ -n "$NODE_SRC" ]; then cp "$NODE_SRC" "$NODE"
  else
    fetch "$DL/switchboard-node-$OSN" "$NODE" || die "the server has no switchboard-node for $OSN yet. Add this machine over ssh instead (see docs/install.md)."
    sums_ok "$NODE" "switchboard-node-$OSN" || die "switchboard-node from $DL did not match its checksum; nothing was changed."
  fi
  chmod +x "$NODE"
  [ "$OSN" = mac ] && xattr -d com.apple.quarantine "$NODE" 2>/dev/null || true
  args=(pair "$SERVER:8132" "$CODE")
  [ -n "$NAME" ] && args+=(--name "$NAME")
  [ -n "$LABEL" ] && args+=(--label "$LABEL")
  "$NODE" "${args[@]}" || die "linking failed (see above). Codes work once, for 10 minutes: get a new one with 'switchboard setup --code' on the server."
  if [ "$OSN" = mac ] || has_systemd; then
    "$NODE" install || die "the link is saved, but the node could not be set to run at login (see above)."
  else
    mkdir -p "$HOME/.local/bin" "$HOME/.local/share/switchboard"
    install -m 755 "$NODE" "$HOME/.local/bin/switchboard-node"
    pkill -f "$HOME/.local/bin/switchboard-node run" 2>/dev/null || true
    nohup "$HOME/.local/bin/switchboard-node" run >>"$HOME/.local/share/switchboard/node.log" 2>&1 &
    say "This machine has no user service manager, so switchboard-node is running in the background now."
    say "After a restart, start it again with: ~/.local/bin/switchboard-node run &"
  fi
fi

# ---------------------------------------------------------------- let the server in over ssh
if [ -n "$SSHKEY" ]; then
  case "$SSHKEY" in ssh-ed25519\ *|ssh-rsa\ *|ecdsa-sha2-*) ;; *) die "--ssh-key takes a public key (ssh-ed25519 ...)" ;; esac
  case "$SSHKEY" in *\"*|*\'*) die "--ssh-key has quotes in it; paste the key line exactly" ;; esac
  keybody=$(printf '%s' "$SSHKEY" | awk '{print $2}')
  mkdir -p "$HOME/.ssh" && chmod 700 "$HOME/.ssh"
  touch "$HOME/.ssh/authorized_keys" && chmod 600 "$HOME/.ssh/authorized_keys"
  # One line for this key, limited to the agent: no shell, no forwarding. A PTY is allowed (the hub's terminals).
  line="command=\"$PY $SB rpc\",no-port-forwarding,no-agent-forwarding,no-X11-forwarding $SSHKEY"
  { grep -vF "$keybody" "$HOME/.ssh/authorized_keys" || true; printf '%s\n' "$line"; } > "$HOME/.ssh/ak.tmp"
  mv "$HOME/.ssh/ak.tmp" "$HOME/.ssh/authorized_keys" && chmod 600 "$HOME/.ssh/authorized_keys"
  say "The server can now reach the agent here over ssh (and nothing else)."
  if [ "$OSN" = mac ]; then
    say "Make sure Remote Login is on: System Settings, General, Sharing, Remote Login."
  elif ! (command -v ss >/dev/null && ss -ltn 2>/dev/null | grep -q ':22 ') && ! pgrep -x sshd >/dev/null 2>&1; then
    say "No ssh server seems to be running here. Install and start one (for example: sudo apt-get install -y openssh-server)."
  fi
  say "Back on the server, add it as $(id -un)@$(hostname)."
fi

say ""
say "Done. This machine shows up in Switchboard within a minute."
