#!/usr/bin/env bash
# Run the unit tests on the test host (SB_TEST_HOST in build-hosts.env, see native/scripts/hosts.sh).
set -euo pipefail
cd "$(dirname "$0")/.."
. native/scripts/hosts.sh
sb_hosts SB_TEST_HOST
H=$SB_TEST_HOST
D=${SB_TEST_DIR:-switchboard-dev}  # SB_TEST_DIR: a separate copy per worktree, so parallel runs don't clobber
rsync -a --delete --exclude .git --exclude target --exclude dist ./ $H:~/$D/
ssh $H "cd ~/$D && python3 -m unittest discover -s tests -t . ${*:+-k $*} -v"
