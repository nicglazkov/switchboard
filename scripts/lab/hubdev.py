#!/usr/bin/env python3
"""A branch's server against the REAL machines, on a test port, to try terminals end to end.

usage: hubdev.py <repo-copy> <port> <host,host> <state-dir>

HUBDEV_NO_SSH=1 leaves the three real machines out, so only machines linked to the hub with switchboard-node show
(SWITCHBOARD_HUB_URL and SWITCHBOARD_HUB_PORT point at a test hub).

Its polling only reads, and /api/term only mints tickets. Actions (new, stop, resume, fresh) DO reach the real
machines: use them only on a test session named sb-test-<what>, and stop it afterwards. No pushes. Its own state
dir (jobs, machines.json), never the live server's. SWITCHBOARD_HUB_PORT points the ws URLs at a test hub.
"""
import sys, threading, time, traceback
from pathlib import Path

ROOT, PORT, HOSTS, STATE = Path(sys.argv[1]), int(sys.argv[2]), sys.argv[3].split(','), Path(sys.argv[4])
sys.path.insert(0, str(ROOT / 'server'))
import fleet as fleet_mod  # noqa: E402
import nodes as nodes_mod  # noqa: E402
import server as srv  # noqa: E402

import os  # noqa: E402
machines = [] if os.environ.get('HUBDEV_NO_SSH') else fleet_mod.MACHINES
app = srv.App(fleet_mod.Fleet(machines, transport=nodes_mod.transport), STATE, ROOT / 'server' / 'web',
              pusher=lambda *a: srv.log(f'(no push) {a[0]}'))


def poller():
    while True:
        try:
            nodes_mod.sync(app.fleet)
            app.fleet.poll()
        except Exception:
            srv.log('poll failed: ' + traceback.format_exc())
        time.sleep(20)


threading.Thread(target=poller, daemon=True).start()
srv.ALLOWED_HOSTS.update(HOSTS)
for h in HOSTS[1:]:
    s = srv.make_server(h, PORT, app)
    threading.Thread(target=s.serve_forever, daemon=True).start()
srv.log(f'hubdev on {",".join(HOSTS)}:{PORT}, state {STATE}')
srv.make_server(HOSTS[0], PORT, app).serve_forever()
