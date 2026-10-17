"""Machine pairing and Add a machine, end to end.

PairCodes and AddOverSsh run against the lab's own pairing stub (a fixed code). LinkedMachine pairs for real: the
real hub (switchboard-hub) mints the code, the real switchboard-node trades it for a token and keeps the link up, and
the real agent (agent/sb.py) answers through it, next to the three fake machines. It needs the two binaries
(scripts/lab/e2e.sh builds them; SB_HUB_BIN and SB_NODE_BIN point at others).
"""
import json, os, re, subprocess, time, unittest
from pathlib import Path

from tests.e2e.lab import LIVE_MAX, ROOT, Lab, LabCase, Stream, free_port, lab_test


def binary(env, name):
    if os.environ.get(env):
        return Path(os.environ[env])
    for d in ('debug', 'release'):
        p = ROOT / 'native' / 'target' / d / name
        if p.exists():
            return p
    return None


HUB, NODE = binary('SB_HUB_BIN', 'switchboard-hub'), binary('SB_NODE_BIN', 'switchboard-node')


@lab_test
class PairCodes(LabCase):
    def test_add_a_machine_gives_a_code_and_a_line_per_os(self):
        code, r = self.lab.post('/api/pair', {})
        self.assertEqual(code, 200, r)
        self.assertRegex(r['code'], r'^[A-Z0-9]{6}$')
        self.assertAlmostEqual(r['expires'] - time.time(), 600, delta=30)
        self.assertEqual(r['pair'], f'switchboard-node pair {r["hub"]} {r["code"]}')
        self.assertEqual(set(r['install']), {'mac', 'linux', 'windows'})
        for os_, line in r['install'].items():
            self.assertIn(f'pair {r["hub"]} {r["code"]}', line, os_)
            self.assertIn('sb.py', line, os_)
        self.assertIn('switchboard-node-mac', r['install']['mac'])
        self.assertIn('switchboard-node-linux', r['install']['linux'])
        self.assertIn('switchboard-node.exe', r['install']['windows'])

    def test_the_line_names_the_address_the_app_used(self):
        """The new machine dials the hub at the address the phone or desktop reached the server on."""
        for host in ('studio.local', '10.0.0.5', 'studio.tail1234.ts.net'):
            code, r = self.lab.post('/api/pair', {}, headers={'Host': f'{host}:{self.lab.port}'})
            self.assertEqual(code, 200, (host, r))
            self.assertTrue(r['hub'].startswith(f'{host}:'), (host, r['hub']))
            self.assertIn(f'http://{host}:', r['install']['linux'])

    @unittest.expectedFailure  # report bug 7: from the server itself the line points the new machine at 127.0.0.1
    def test_the_line_never_points_another_machine_at_localhost(self):
        code, r = self.lab.post('/api/pair', {})  # the Settings page opened on the server's own machine
        self.assertEqual(code, 200, r)
        self.assertNotRegex(r['pair'], r'\b(127\.0\.0\.1|localhost)\b')

    def test_pair_needs_the_app_header(self):
        code, r = self.lab.request('POST', '/api/pair', b'{}', {'Content-Type': 'application/json'})
        self.assertEqual(code, 403, r)


@lab_test
class AddOverSsh(LabCase):
    def tearDown(self):
        names = [m['name'] for m in self.lab.get('/api/machines')[1]['machines']]
        for n in names:
            if n not in ('studio', 'laptop', 'tower'):
                self.lab.post('/api/machines', {'op': 'remove', 'machine': n})

    def fleet_file(self):
        return json.loads((self.lab.ctrl / 'config' / 'fleet.json').read_text())['machines']

    def test_add_rename_reorder_remove(self):
        s = Stream(self.lab)
        try:
            t0 = time.time()
            code, r = self.lab.post('/api/machines', {'op': 'add', 'name': 'desk', 'label': 'Desk', 'target': 'me@desk.lan', 'os': 'linux'})
            self.assertEqual(code, 200, r)
            t, _ = s.wait_event('change', t0, LIVE_MAX)  # every open app hears about it
            self.assertLess(t - t0, LIVE_MAX)
        finally:
            s.close()
        row = next(m for m in r['machines'] if m['name'] == 'desk')
        self.assertEqual((row['label'], row['via'], row['target'], row['os']), ('Desk', 'ssh', 'me@desk.lan', 'linux'))
        self.assertIn('desk', [m['name'] for m in self.fleet_file()])
        # the server tries it at once: the lab has no such host, so it shows offline with ssh's own words
        m = self.lab.wait(lambda: (lambda m: m if m.get('online') is False else None)(self.lab.machine('desk')), 5,
                          what='the new machine to be tried')
        self.assertIn('Could not resolve hostname', m['error'])
        self.assertTrue(self.lab.rpcs('list', 'desk') or [x for x in self.lab.rpcs('list') if x['target'] == 'me@desk.lan'])

        code, r = self.lab.post('/api/machines', {'op': 'rename', 'machine': 'desk', 'label': 'Big Desk'})
        self.assertEqual(code, 200, r)
        self.assertEqual(self.lab.machine('desk')['label'], 'Big Desk')

        code, r = self.lab.post('/api/machines', {'op': 'order', 'names': ['desk', 'studio', 'laptop', 'tower']})
        self.assertEqual(code, 200, r)
        self.assertEqual([m['name'] for m in r['machines']][0], 'desk')
        self.assertEqual([m['name'] for m in self.lab.sessions()['machines']][0], 'desk')

        code, r = self.lab.post('/api/machines', {'op': 'remove', 'machine': 'desk'})
        self.assertEqual(code, 200, r)
        self.assertNotIn('desk', [m['name'] for m in r['machines']])
        self.assertNotIn('desk', [m['name'] for m in self.lab.sessions()['machines']])
        self.assertNotIn('desk', [m['name'] for m in self.fleet_file()])

    def test_bad_adds_are_refused_with_a_reason(self):
        cases = [({'name': 'studio', 'target': 'me@desk.lan'}, 409, 'already'),
                 ({'name': 'Big Desk', 'target': 'me@desk.lan'}, 400, 'lowercase'),
                 ({'name': 'x' * 33, 'target': 'me@desk.lan'}, 400, 'lowercase'),
                 ({'name': 'desk', 'target': 'node:desk'}, 400, 'user@host'),
                 ({'name': 'desk', 'target': '-oProxyCommand=touch /tmp/pwned'}, 400, 'user@host'),
                 ({'name': 'desk', 'target': 'me@desk.lan; rm -rf ~'}, 400, 'user@host'),
                 ({'name': 'desk'}, 400, 'user@host')]
        for body, want, words in cases:
            code, r = self.lab.post('/api/machines', dict(body, op='add'))
            self.assertEqual(code, want, (body, r))
            self.assertIn(words, r['error'], body)
        self.assertEqual([m['name'] for m in self.fleet_file()], ['studio', 'laptop', 'tower'])

    def test_unknown_machine_and_op(self):
        self.assertEqual(self.lab.post('/api/machines', {'op': 'remove', 'machine': 'nope'})[0], 400)
        self.assertEqual(self.lab.post('/api/machines', {'op': 'explode'})[0], 400)


class HubLab(unittest.TestCase):
    """A fake fleet with a real hub next to it; nodes paired here run the real agent."""
    POLL = None

    @classmethod
    def setUpClass(cls):
        cls.hub_port = free_port()
        cls.lab = Lab(env={'LAB_HUB': '1', 'SWITCHBOARD_HUB_URL': f'http://127.0.0.1:{cls.hub_port}',
                           'SWITCHBOARD_HUB_PORT': str(cls.hub_port)}, poll=cls.POLL)
        state = cls.lab.ctrl / 'state'
        state.mkdir(parents=True)
        henv = dict(os.environ, SWITCHBOARD_HUB_LISTEN=f'127.0.0.1:{cls.hub_port}', SWITCHBOARD_MACHINES=str(state / 'machines.json'),
                    SWITCHBOARD_TERM_KEY=str(cls.lab.tmp / 'term.key'), HOME=str(cls.lab.tmp / 'hubhome'))
        cls.logs = [open(cls.lab.tmp / 'hub.log', 'w')]
        cls.hub = subprocess.Popen([str(HUB)], env=henv, stdout=cls.logs[0], stderr=subprocess.STDOUT)
        cls.nodes = []
        cls.lab.start(caps=cls.POLL is None)

    @classmethod
    def tearDownClass(cls):
        for p in cls.nodes + [cls.hub]:
            p.terminate()
            p.wait(5)
        for f in cls.logs:
            f.close()
        cls.lab.stop()

    def node_env(self, name):
        home = self.lab.tmp / f'node-{name}'
        (home / 'Desktop' / 'Claude').mkdir(parents=True, exist_ok=True)
        env = dict(os.environ, HOME=str(home), SWITCHBOARD_NODE_CONFIG=str(home / 'node.json'), SWITCHBOARD_SB=str(ROOT / 'agent' / 'sb.py'),
                   SB_HOME=str(home / '.claude' / 'switchboard'))
        env.pop('SB_INTERNAL', None)
        return env

    def node(self, env, *args):
        return subprocess.run([str(NODE), *args], env=env, capture_output=True, text=True, timeout=30)

    def run_node(self, env, name):
        self.logs.append(open(self.lab.tmp / f'node-{name}.log', 'w'))
        p = subprocess.Popen([str(NODE), 'run'], env=env, stdout=self.logs[-1], stderr=subprocess.STDOUT)
        self.nodes.append(p)
        return p

    def hub_nodes(self):
        from urllib.request import urlopen
        with urlopen(f'http://127.0.0.1:{self.hub_port}/internal/nodes', timeout=5) as r:
            return json.loads(r.read())['nodes']


@lab_test
@unittest.skipUnless(HUB and NODE, 'needs switchboard-hub and switchboard-node (scripts/lab/e2e.sh builds them)')
class LinkedMachine(HubLab):
    """Add a machine through the real hub: code, pair, link, RPCs over the link, Remove."""

    def test_pair_link_use_and_remove(self):
        code, p = self.lab.post('/api/pair', {})
        self.assertEqual(code, 200, p)
        self.assertEqual(p['hub'], f'127.0.0.1:{self.hub_port}')
        env = self.node_env('labbox')
        r = self.node(env, 'pair', p['hub'], p['code'], '--name', 'labbox', '--label', 'Lab box')
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertIn('Linked as labbox (Lab box)', r.stdout)
        cfg = Path(env['SWITCHBOARD_NODE_CONFIG'])
        self.assertEqual(oct(cfg.stat().st_mode & 0o777), '0o600')
        self.assertNotIn(json.loads(cfg.read_text())['token'], (self.lab.tmp / 'fakefleet.log').read_text())

        # one code, one machine
        r = self.node(dict(env, SWITCHBOARD_NODE_CONFIG=str(cfg) + '.2'), 'pair', p['hub'], p['code'], '--name', 'other')
        self.assertNotEqual(r.returncode, 0)
        self.assertIn('wrong or has expired', r.stderr)

        self.run_node(env, 'labbox')
        m = self.lab.wait(lambda: next((m for m in self.lab.sessions()['machines'] if m['name'] == 'labbox' and m['online']), None),
                          30, 0.2, 'the linked machine to come online')
        self.assertEqual((m['label'], m['os']), ('Lab box', 'linux'))
        self.assertIn('upload', m['features'])  # the real agent answered, through the hub
        view = next(x for x in self.lab.get('/api/machines')[1]['machines'] if x['name'] == 'labbox')
        self.assertEqual((view['via'], view['target'], view['online']), ('link', None, True))
        self.assertTrue(self.lab.rpcs('list', 'labbox'))

        # a real RPC over the link: a file attached to one of its chats lands in the real agent's uploads folder
        code, up = self.lab.upload('labbox', 'notes.txt', b'hello over the link\n')
        self.assertEqual(code, 200, up)
        self.assertTrue(up['path'].startswith(str(Path(env['SB_HOME']) / 'uploads')), up)
        self.assertEqual(Path(up['path']).read_bytes(), b'hello over the link\n')
        big = os.urandom(3 * 1024 * 1024 + 5)  # 1 MB chunks over a link (the hub takes 4 MB a message, base64 adds a third)
        code, up = self.lab.upload('labbox', 'build.zip', big)
        self.assertEqual(code, 200, up)
        self.assertEqual(Path(up['path']).read_bytes(), big)
        self.assertEqual(len([x for x in self.lab.rpcs('upload', 'labbox') if x['req']['name'] == 'build.zip']), 4)

        # Remove unpairs it: the hub forgets it, the node is told, and it leaves every list
        code, r = self.lab.post('/api/machines', {'op': 'remove', 'machine': 'labbox'})
        self.assertEqual(code, 200, r)
        self.assertNotIn('labbox', [x['name'] for x in r['machines']])
        self.assertEqual([n for n in self.hub_nodes() if n['name'] == 'labbox'], [])
        time.sleep(6)  # a poll later it has not come back
        self.assertNotIn('labbox', [x['name'] for x in self.lab.sessions()['machines']])
        self.lab.wait(lambda: 'removed from Switchboard' in (self.lab.tmp / 'node-labbox.log').read_text(), 10,
                      what='the node to hear it was removed')

    def test_a_linked_machine_cannot_take_an_ssh_machines_name(self):
        code, p = self.lab.post('/api/pair', {})
        r = self.node(self.node_env('studio'), 'pair', p['hub'], p['code'], '--name', 'studio')
        self.assertNotEqual(r.returncode, 0, r.stdout)
        self.assertEqual([m['target'] for m in self.lab.get('/api/machines')[1]['machines'] if m['name'] == 'studio'], ['fake@studio'])



@lab_test
@unittest.skipUnless(HUB and NODE, 'needs switchboard-hub and switchboard-node (scripts/lab/e2e.sh builds them)')
class LinkedMachineLatency(HubLab):
    POLL = 3600  # no timed poll during the test: only something the link itself sets off can make it show

    @unittest.expectedFailure  # report bug 2: a newly linked machine shows up only at the next poll (up to 20 s)
    def test_a_linked_machine_shows_up_quickly(self):
        code, p = self.lab.post('/api/pair', {})
        env = self.node_env('quick')
        self.assertEqual(self.node(env, 'pair', p['hub'], p['code'], '--name', 'quick').returncode, 0)
        t0 = time.time()
        self.run_node(env, 'quick')
        self.lab.wait(lambda: any(n['name'] == 'quick' and n['online'] for n in self.hub_nodes()), 10, 0.05, 'the link')
        linked = time.time()
        self.lab.wait(lambda: any(m['name'] == 'quick' and m['online'] for m in self.lab.sessions()['machines']), 10, 0.1,
                      'the machine in the list')
        self.assertLess(time.time() - linked, LIVE_MAX, f'linked after {linked - t0:.1f} s, listed {time.time() - linked:.1f} s later')
