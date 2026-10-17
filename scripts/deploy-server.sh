#!/usr/bin/env bash
# Install or update the Switchboard server on the server host as a systemd user service.
# Hosts come from build-hosts.env (SB_SERVER_HOST, SB_SERVER_URL; see native/scripts/hosts.sh).
set -euo pipefail
cd "$(dirname "$0")/.."
. native/scripts/hosts.sh
sb_hosts SB_SERVER_HOST SB_SERVER_URL
H=$SB_SERVER_HOST
URL=${SB_SERVER_URL%/}
rsync -a --delete --exclude .git --exclude .superpowers --exclude tests --exclude docs ./ $H:~/switchboard/
ssh $H 'set -e; mkdir -p ~/.config/systemd/user; cp ~/switchboard/server/switchboard.service ~/.config/systemd/user/
systemctl --user stop sb-trial 2>/dev/null || true
systemctl --user daemon-reload; systemctl --user enable --now switchboard >/dev/null 2>&1; systemctl --user restart switchboard
sleep 3; systemctl --user is-active switchboard; curl -sf '"'$URL'"'/api/summary >/dev/null && echo "serving at '"$URL"'"'
