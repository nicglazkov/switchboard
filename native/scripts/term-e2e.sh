#!/bin/bash
# The terminal pipeline end to end (spec 2026-10-14): a TEST hub on a spare port (never 8132) running the fixture
# program instead of ssh, two viewers flooding and resizing through it (term_probe), then every capture replayed with
# term_replay --check. Run it in native/ on a build machine:  scripts/term-e2e.sh [port]
# It starts nothing that lasts: the hub is killed on the way out.
set -euo pipefail
PORT=${1:-18232}
[ "$PORT" = 8132 ] && { echo "not 8132: that is the real hub" >&2; exit 2; }
CARGO=${CARGO:-cargo}
cd "$(dirname "$0")/.."
$CARGO build -q -p sb-hub --bin switchboard-hub --example term_probe
$CARGO build -q -p sb-core --features grid --example term_replay
T=$(mktemp -d /tmp/sb-term-e2e.XXXXXX)
mkdir -p "$T/out"
echo '{"machines":[{"name":"lab","label":"Lab","target":"fixture@lab","os":"linux"}]}' > "$T/machines.json"
SWITCHBOARD_HUB_LISTEN=127.0.0.1:$PORT SWITCHBOARD_TERM_KEY=$T/term.key SWITCHBOARD_MACHINES=$T/machines.json \
  SWITCHBOARD_HUB_SSH=$PWD/scripts/term-fixture.py TERM_FIXTURE_SECONDS=${TERM_FIXTURE_SECONDS:-6} \
  target/debug/switchboard-hub > "$T/hub.log" 2>&1 &
HUB=$!
trap 'kill $HUB 2>/dev/null || true' EXIT
for _ in $(seq 50); do [ -f "$T/term.key" ] && break; sleep 0.1; done
target/debug/examples/term_probe --hub 127.0.0.1:$PORT --key "$T/term.key" --out "$T/out"
ok=0
for f in "$T"/out/*.rec; do
  echo "== $(basename "$f")"
  if ! target/debug/examples/term_replay "$f" --check | tail -n 4; then ok=1; fi
done
echo "== hub log"
cat "$T/hub.log"
echo "captures: $T/out"
exit $ok
