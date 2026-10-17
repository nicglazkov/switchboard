#!/usr/bin/env bash
# Build switchboard-hub on the server host (SB_SERVER_HOST, see native/scripts/hosts.sh) and run it as the systemd
# user service switchboard-hub (:8132).
#   scripts/deploy-hub.sh
# A restart closes open terminals; the sessions keep running (closing a terminal only detaches).
set -euo pipefail
cd "$(dirname "$0")/.."
. native/scripts/hosts.sh
sb_hosts SB_SERVER_HOST
H=$SB_SERVER_HOST
REMOTE=${SB_REMOTE:-switchboard-hub-build}

ssh -q $H "mkdir -p $REMOTE/server"
rsync -a --delete --exclude target --exclude dist --exclude server ./native/ $H:$REMOTE/
rsync -a server/fleet.py server/term.py server/switchboard-hub.service $H:$REMOTE/server/
ssh -q $H "set -e; export PATH=\$HOME/.cargo/bin:\$PATH; cd ~/$REMOTE
cargo build --release --locked -q -p sb-hub
mkdir -p ~/.local/bin ~/.config/systemd/user
install -m 755 target/release/switchboard-hub ~/.local/bin/switchboard-hub.new
mv -f ~/.local/bin/switchboard-hub.new ~/.local/bin/switchboard-hub
cp server/switchboard-hub.service ~/.config/systemd/user/
# The Switchboard server writes the machine list at start; until a server with terminals has run, write it from fleet.py.
if [ ! -f ~/.local/share/switchboard/machines.json ]; then
  python3 -c 'import sys, pathlib; sys.path.insert(0, \"server\"); import fleet, term
term.write_machines(pathlib.Path.home() / \".local/share/switchboard/machines.json\", fleet.MACHINES, list(term.OWN_ORIGINS))'
  echo 'wrote machines.json from fleet.py'
fi
systemctl --user daemon-reload
systemctl --user enable switchboard-hub >/dev/null 2>&1
systemctl --user restart switchboard-hub
sleep 1
systemctl --user is-active switchboard-hub
curl -sf http://127.0.0.1:8132/healthz && echo
echo \"switchboard-hub \$(~/.local/bin/switchboard-hub --version | cut -d' ' -f2) serving on :8132\""
