#!/usr/bin/env bash
# Screenshot the live UI with headless Chrome on the server host: phone + desktop, light + dark, plus any extra paths.
# Hosts come from build-hosts.env (SB_SERVER_HOST, SB_SERVER_URL; see native/scripts/hosts.sh).
set -euo pipefail
. "$(dirname "$0")/../native/scripts/hosts.sh"
sb_hosts SB_SERVER_HOST SB_SERVER_URL
H=$SB_SERVER_HOST
URL=${SB_SERVER_URL%/}
OUT=${OUT:-/tmp/sb-shots}
rsync -a --delete --exclude .git "$(dirname "$0")/../" $H:~/switchboard-dev/
ssh $H "rm -rf /tmp/shots; mkdir -p /tmp/shots; cd /tmp/shots
shot() { google-chrome --headless=new --no-sandbox --hide-scrollbars --force-device-scale-factor=\$4 --window-size=\$2 --virtual-time-budget=\${5:-7000} --screenshot=/tmp/shots/\$1.png \"\$3\" 2>/dev/null; }
for t in light dark; do
  shot phone-\$t 390,844 '$URL/?theme='\$t 2
  shot desk-\$t 1280,800 '$URL/?theme='\$t 1
done
i=0; for p in $*; do i=\$((i+1)); shot extra\$i-dark 390,844 \"$URL\$p?theme=dark\" 2 12000; done"
mkdir -p "$OUT" && rm -f "$OUT"/*.png && scp -q "$H:/tmp/shots/*.png" "$OUT/"
ls "$OUT"
