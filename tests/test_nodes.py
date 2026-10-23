"""Linked machines on the server side (nodes.py): RPCs through the hub, the fleet following the hub's paired list,
and POST /api/pair. A small fake hub stands in for sb-hub's /internal routes."""
import json, sys, threading, unittest
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from tempfile import mkdtemp
sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'server'))
import fleet, nodes, server  # noqa: E402

MACH = [{'name': 'desk', 'label': 'Desk', 'target': 'me@x', 'os': 'mac'}]


class FakeHub:
    def __init__(self):
        self.nodes = [{'name': 'lab', 'label': 'Lab box', 'os': 'linux', 'online': True}]
        self.down, self.seen = set(), []
        hub = self

        class H(BaseHTTPRequestHandler):
            def log_message(self, *a):
                pass

            def _send(self, code, body):
                b = json.dumps(body).encode()
                self.send_response(code)
                self.send_header('Content-Type', 'application/json')
                self.send_header('Content-Length', str(len(b)))
                self.end_headers()
                self.wfile.write(b)

            def do_GET(self):
                hub.seen.append(('GET', self.path, None))
                if self.path == '/internal/nodes':
                    return self._send(200, {'ok': True, 'nodes': hub.nodes})
                self._send(404, {'ok': False, 'error': 'not found'})

            def do_POST(self):
                body = json.loads(self.rfile.read(int(self.headers.get('Content-Length') or 0)) or b'{}')
                hub.seen.append(('POST', self.path, body))
                if self.path == '/internal/pair':
                    return self._send(200, {'ok': True, 'code': 'K7M2QX', 'expires': 1234})
                if self.path.startswith('/internal/rpc/'):
                    name = self.path.split('/')[3].split('?')[0]
                    if name in hub.down:
                        return self._send(503, {'ok': False, 'error': 'Lab box is not linked to the hub right now.'})
                    if body.get('verb') == 'list':
                        return self._send(200, {'ok': True, 'sessions': [{'sid': 's1', 'title': 'On the lab', 'mtime': 1.0}], 'replaced': []})
                    return self._send(200, {'ok': False, 'error': 'nope'})
                self._send(404, {'ok': False, 'error': 'not found'})

        self.httpd = ThreadingHTTPServer(('127.0.0.1', 0), H)
        self.url = f'http://127.0.0.1:{self.httpd.server_address[1]}'
        threading.Thread(target=self.httpd.serve_forever, daemon=True).start()

    def close(self):
        self.httpd.shutdown()
        self.httpd.server_close()


class NodesTest(unittest.TestCase):
    def setUp(self):
        self.hub = FakeHub()
        self.addCleanup(self.hub.close)
        self.ssh = []
        real_ssh, fleet.ssh_transport = fleet.ssh_transport, lambda t, r, to: self.ssh.append(t) or {'ok': True, 'sessions': []}
        self.addCleanup(setattr, fleet, 'ssh_transport', real_ssh)
        self.fleet = fleet.Fleet(MACH, lambda t, r, to: nodes.transport(t, r, to, hub=self.hub.url))

    def test_rpcs_go_through_the_hub_for_linked_machines_only(self):
        r = nodes.transport('node:lab', {'verb': 'list'}, 15, hub=self.hub.url)
        self.assertEqual(r['sessions'][0]['sid'], 's1')
        self.assertEqual(self.hub.seen[-1], ('POST', '/internal/rpc/lab?timeout=15', {'verb': 'list'}))
        nodes.transport('me@x', {'verb': 'list'}, 15, hub=self.hub.url)
        self.assertEqual(self.ssh, ['me@x'])
        self.hub.down.add('lab')
        with self.assertRaises(OSError) as e:
            nodes.transport('node:lab', {'verb': 'list'}, 15, hub=self.hub.url)
        self.assertEqual(str(e.exception), 'Lab box is not linked to the hub right now.')
        with self.assertRaises(OSError) as e:
            nodes.transport('node:lab', {'verb': 'list'}, 1, hub='http://127.0.0.1:9')
        self.assertIn('the hub is not reachable', str(e.exception))

    def test_the_fleet_follows_the_hub(self):
        self.assertTrue(nodes.sync(self.fleet, hub=self.hub.url))
        self.assertEqual(self.fleet.order, ['desk', 'lab'])
        self.assertEqual(self.fleet.machines['lab'], {'name': 'lab', 'label': 'Lab box', 'target': 'node:lab', 'os': 'linux'})
        self.assertFalse(nodes.sync(self.fleet, hub=self.hub.url), 'nothing new')

        self.fleet.poll()
        snap = {m['name']: m for m in self.fleet.snapshot()['machines']}
        self.assertTrue(snap['lab']['online'])
        self.assertEqual(snap['lab']['sessions'][0]['title'], 'On the lab')
        self.assertIsNone(snap['lab']['target'], 'targets never reach the page')
        self.hub.down.add('lab')
        self.fleet.poll()
        lab = next(m for m in self.fleet.snapshot()['machines'] if m['name'] == 'lab')
        self.assertFalse(lab['online'])
        self.assertIn('not linked', lab['error'])

        # the name the user gave a machine in Settings stays; an ssh machine's name is never taken over
        self.hub.nodes = [{'name': 'lab', 'label': 'Lab', 'os': 'linux'}, {'name': 'desk', 'label': 'Fake', 'os': 'linux'}]
        self.assertFalse(nodes.sync(self.fleet, hub=self.hub.url))
        self.assertEqual(self.fleet.machines['lab']['label'], 'Lab box')
        self.assertEqual(self.fleet.machines['desk']['target'], 'me@x')

        # the hub cannot be asked: nothing changes
        self.assertIsNone(nodes.sync(self.fleet, hub='http://127.0.0.1:9'))
        self.assertIn('lab', self.fleet.machines)

        # unpaired: it leaves the list
        self.hub.nodes = []
        self.assertTrue(nodes.sync(self.fleet, hub=self.hub.url))
        self.assertEqual(self.fleet.order, ['desk'])
        self.assertEqual([m['name'] for m in self.fleet.snapshot()['machines']], ['desk'])
        self.assertEqual(self.fleet.summary()['machines'], 1)

    def test_an_empty_fleet_polls(self):
        f = fleet.Fleet([], lambda t, r, to: nodes.transport(t, r, to, hub=self.hub.url))
        f.poll()
        nodes.sync(f, hub=self.hub.url)
        f.poll()
        self.assertTrue(f.snapshot()['machines'][0]['online'])

    def test_add_a_machine(self):
        state = Path(mkdtemp())
        app = server.App(self.fleet, state_dir=state, web_dir=Path(__file__).resolve().parents[1] / 'server' / 'web', guard_delay=0.05)
        old, nodes.HUB = nodes.HUB, self.hub.url
        self.addCleanup(setattr, nodes, 'HUB', old)
        app.cfg_dir.mkdir(parents=True, exist_ok=True)
        (app.cfg_dir / 'downloads.json').write_text('{"url": "http://100.64.0.5:8131/app"}')  # what publish.sh writes
        r = app.post('/api/pair', {}, host='100.64.0.5:8130')
        self.assertEqual(r['code'], 'K7M2QX')
        self.assertEqual(r['hub'], '100.64.0.5:8132')
        self.assertEqual(r['pair'], 'switchboard-node pair 100.64.0.5:8132 K7M2QX')
        mac, win = r['install']['mac'], r['install']['windows']
        self.assertTrue(mac.startswith('n=$(mktemp) && curl -fsSL http://100.64.0.5:8131/node/switchboard-node-mac -o "$n"'))
        self.assertIn('"$n" pair 100.64.0.5:8132 K7M2QX && "$n" install', mac)
        self.assertIn('test -f ~/.claude/switchboard/sb.py ||', mac)  # an agent already there is never replaced
        self.assertIn('switchboard-node-linux', r['install']['linux'])
        self.assertIn('iwr -UseBasicParsing http://100.64.0.5:8131/node/switchboard-node.exe', win)
        self.assertIn('if (-not (Test-Path $s))', win)
        nodes.HUB = 'http://127.0.0.1:9'
        with self.assertRaises(server.HttpError) as e:
            app.post('/api/pair', {}, host='10.0.0.5:8130')
        self.assertEqual(e.exception.code, 502)
        self.assertIn("The hub couldn't make a code", str(e.exception))


class AgentNameTest(unittest.TestCase):
    """sb.py names its machine the way the node paired it, so hook events reach the right machine."""

    def test_the_paired_name_wins(self):
        import os
        from tests.helpers import fresh_sb
        tmp = Path(mkdtemp())
        old = os.environ.get('HOME')
        os.environ['HOME'] = str(tmp)
        self.addCleanup(lambda: os.environ.__setitem__('HOME', old) if old else os.environ.pop('HOME', None))
        sb = fresh_sb(tmp)
        default = sb.machine_name()
        self.assertRegex(default, r'^[a-z0-9]{1,32}$')  # this computer's own name until the server says otherwise
        sb.remember_identity({'verb': 'list', 'as': 'studio'})
        self.assertEqual(sb.machine_name(), 'studio', 'the name the server polls it as')
        default = 'studio'
        cfg = tmp / '.config' / 'switchboard'
        cfg.mkdir(parents=True)
        (cfg / 'node.json').write_text(json.dumps({'hub': 'h:8132', 'machine': 'lab', 'token': 'x'}))
        self.assertEqual(sb.machine_name(), 'lab')
        (cfg / 'node.json').write_text(json.dumps({'machine': '../Bad'}))
        self.assertEqual(sb.machine_name(), default)
        (cfg / 'node.json').write_text('{nope')
        self.assertEqual(sb.machine_name(), default)


if __name__ == '__main__':
    unittest.main()
