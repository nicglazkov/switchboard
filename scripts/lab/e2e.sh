#!/usr/bin/env bash
# End-to-end tests against the lab fleet (tests/e2e). Builds the hub and the node for the real pairing test, then
# runs every test. Extra arguments go to unittest, for example:
#   scripts/lab/e2e.sh                                   # everything
#   scripts/lab/e2e.sh tests.e2e.test_e2e_live -v        # one file
# SB_E2E_KEEP=1 keeps each lab's folder (logs, rpc.log, pushes.log) and prints where it is.
set -euo pipefail
cd "$(dirname "$0")/../.."
if command -v cargo >/dev/null; then
  (cd native && cargo build -q -p sb-hub -p sb-node)
else
  echo "cargo is not installed: the real pairing test is skipped" >&2
fi
export SB_E2E=1
if [ $# -eq 0 ]; then
  exec python3 -W ignore::ResourceWarning -m unittest discover -s tests/e2e -t . -v
fi
exec python3 -W ignore::ResourceWarning -m unittest "$@"
