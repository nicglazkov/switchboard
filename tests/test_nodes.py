"""Linked machines on the server side (nodes.py): RPCs through the hub, the fleet following the hub's paired list,
and POST /api/pair. A small fake hub stands in for sb-hub's /internal routes."""
import json, sys, threading, unittest
from unittest import mock
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

    def test_a_machine_the_hub_stops_listing_keeps_its_settings(self):
        # The hub answers with no machines when it lost its own file. Dropping them here threw away each one's name,
        # Claude folder and place in the list; linked again, they came back as new machines.
        nodes.sync(self.fleet, hub=self.hub.url)
        self.fleet.poll()
        self.fleet.rename('lab', 'The lab')
        self.fleet.set_root('lab', '~/Work')
        self.fleet.set_roam('lab', False)
        self.fleet.reorder(['lab', 'desk'])
        told = []
        self.fleet.on_change = told.append
        self.hub.nodes = []
        self.assertFalse(nodes.sync(self.fleet, hub=self.hub.url))
        self.assertEqual(self.fleet.order, ['lab', 'desk'])
        lab = self.fleet.snapshot()['machines'][0]
        self.assertEqual((lab['name'], lab['label'], lab['online']), ('lab', 'The lab', False))
        self.assertIn('not linked to the hub any more', lab['error'])
        self.assertEqual(told, ['lab'], 'said once')
        nodes.sync(self.fleet, hub=self.hub.url)
        self.assertEqual(told, ['lab'], 'and not again at every poll')
        # the polls that follow do not swap what to do about it for the hub's "No linked machine is named lab."
        asked = len(self.hub.seen)
        self.fleet.poll()
        nodes.sync(self.fleet, hub=self.hub.url)
        self.assertIn('Link it again from Add a machine', self.fleet.snapshot()['machines'][0]['error'])
        self.assertEqual(told, ['lab'])
        self.assertEqual([p for _, p, _ in self.hub.seen[asked:] if 'rpc/lab' in p], [], 'nobody there to ask')

        # linked again: the same machine, as it was set up, and it shows at once
        self.hub.nodes = [{'name': 'lab', 'label': 'Lab box', 'os': 'linux', 'online': True, 'since': 7}]
        nodes.sync(self.fleet, hub=self.hub.url)
        m = self.fleet.machines['lab']
        self.assertEqual((m['label'], m.get('root'), m.get('roam')), ('The lab', '~/Work', False))
        self.assertTrue(self.fleet.snapshot()['machines'][0]['online'])

    def test_one_odd_machine_or_answer_never_stops_the_sync(self):
        self.hub.nodes = [{'name': 'Not A Name!', 'label': 7, 'os': None}, None, {'name': 5}, {'name': 'lab', 'label': ['x'], 'os': 'linux'}]
        self.assertTrue(nodes.sync(self.fleet, hub=self.hub.url))
        self.assertEqual(self.fleet.order, ['desk', 'lab'])
        self.assertEqual(self.fleet.machines['lab']['label'], 'lab')
        for odd in ('x', None, {'a': 1}):
            self.hub.nodes = odd
            self.assertIsNone(nodes.sync(self.fleet, hub=self.hub.url), 'an answer that is no list changes nothing')
        self.assertEqual(self.fleet.order, ['desk', 'lab'])

    def test_a_call_that_may_have_arrived_is_told_apart_from_one_that_never_left(self):
        said = {}
        hub = self.hub

        class Odd(hub.httpd.RequestHandlerClass):
            def do_POST(self):
                self.rfile.read(int(self.headers.get('Content-Length') or 0))
                self._send(*said['now'])
        hub.httpd.RequestHandlerClass = Odd
        ask = lambda: nodes.transport('node:lab', {'verb': 'start'}, 5, hub=hub.url)
        for code, msg, maybe in ((504, 'Lab box did not answer in 40 s.', True),
                                 (503, 'The link to Lab box dropped before it answered.', True),
                                 (503, 'Lab box is not linked to the hub right now.', False),
                                 (404, 'No linked machine is named lab.', False)):
            said['now'] = (code, {'ok': False, 'error': msg})
            with self.assertRaises(OSError) as e:
                ask()
            self.assertEqual(isinstance(e.exception, fleet.MaybeDelivered), maybe, msg)
            self.assertEqual(str(e.exception), msg)
        with self.assertRaises(OSError) as e:
            nodes.transport('node:lab', {'verb': 'start'}, 1, hub='http://127.0.0.1:9')
        self.assertNotIsInstance(e.exception, fleet.MaybeDelivered, 'nothing listens there: it never left')

    def test_an_empty_fleet_polls(self):
        f = fleet.Fleet([], lambda t, r, to: nodes.transport(t, r, to, hub=self.hub.url))
        f.poll()
        nodes.sync(f, hub=self.hub.url)
        f.poll()
        self.assertTrue(f.snapshot()['machines'][0]['online'])

    def test_removing_a_machine_the_hub_forgot(self):
        # Audit, server 5: unlinked at the hub, it stays in the list until it is removed; the hub's "no such machine"
        # must not stand in the way of that.
        nodes.sync(self.fleet, hub=self.hub.url)
        app = server.App(self.fleet, state_dir=Path(mkdtemp()), web_dir=Path(mkdtemp()))
        old = nodes.HUB
        self.addCleanup(setattr, nodes, 'HUB', old)
        nodes.HUB = 'http://127.0.0.1:9'
        with self.assertRaises(server.HttpError) as e:
            app.post('/api/machines', {'op': 'remove', 'machine': 'lab'})
        self.assertEqual(e.exception.code, 502)
        self.assertIn('lab', self.fleet.machines, 'the hub would bring it back: it stays')
        nodes.HUB, said = self.hub.url, []
        real = nodes.hub_call

        def forgot(method, path, body=None, **kw):
            said.append(path)
            raise OSError('No linked machine is named lab.')
        nodes.hub_call = forgot
        self.addCleanup(setattr, nodes, 'hub_call', real)
        self.assertTrue(app.post('/api/machines', {'op': 'remove', 'machine': 'lab'})['ok'])
        self.assertEqual(said, ['/internal/unpair/lab'])
        self.assertNotIn('lab', self.fleet.machines)

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
        # this server's own downloads, with its install script beside them: the line runs that script, which
        # also brings Switchboard's own Python to a computer that has none
        kit = Path(mkdtemp()) / 'downloads'
        (kit / 'node').mkdir(parents=True)
        for f in nodes.KIT_FILES:
            (kit / 'node' / f).write_text('x')
        (app.cfg_dir / 'downloads.json').unlink()
        r = nodes.pair('100.64.0.5:8130', hub=self.hub.url, cfg_dir=app.cfg_dir, kit_node=kit / 'node')
        self.assertIn('n=$(mktemp)', r['install']['mac'], 'no install script there: the node is fetched by itself')
        for f in ('install-agent.sh', 'install-agent.ps1'):
            (kit / f).write_text('x')
        r = nodes.pair('100.64.0.5:8130', hub=self.hub.url, cfg_dir=app.cfg_dir, kit_node=kit / 'node')
        line = f'curl -fsSL http://100.64.0.5:8131/install-agent.sh | bash -s -- --server 100.64.0.5 --code {r["code"]}'
        self.assertEqual((r['install']['mac'], r['install']['linux']), (line, line))
        self.assertEqual(r['install']['windows'], '& ([scriptblock]::Create((iwr -UseBasicParsing http://100.64.0.5:8131/install-agent.ps1)'
                                                  f'.Content)) -Server 100.64.0.5 -Code {r["code"]}')
        self.assertEqual(r['pair'], f'switchboard-node pair 100.64.0.5:8132 {r["code"]}')
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


class ControlProxyTest(unittest.TestCase):
    """GET and POST /api/keys/devices and POST /api/pair_device: a thin proxy to the node's control API on this computer
    (plan section 3.5). The node has no control API yet, so a stand-in answers with the plan's shapes."""

    def setUp(self):
        from tests.test_keys import FakeControl
        self.control = FakeControl()
        self.addCleanup(self.control.close)
        old = nodes.CONTROL
        nodes.CONTROL = self.control.url
        self.addCleanup(setattr, nodes, 'CONTROL', old)
        self.app = server.App(fleet.Fleet(MACH, lambda *a: {'ok': True, 'sessions': []}), state_dir=Path(mkdtemp()),
                              web_dir=Path(__file__).resolve().parents[1] / 'server' / 'web')

    def test_the_paired_devices_with_the_fields_an_app_needs_and_no_other(self):
        r = self.app.get('/api/keys/devices')
        self.assertTrue(r['ok'])
        self.assertEqual([d['machine'] for d in r['devices']], ['lab', 'tower'])
        self.assertEqual(sorted(r['devices'][1]), ['id', 'kind', 'last_seen', 'machine', 'name', 'paired', 'path', 'via'])
        method, path, body, headers = self.control.seen[-1]
        self.assertEqual((method, path, body), ('GET', '/devices', None))
        self.assertNotIn('origin', headers)
        self.assertTrue(headers['host'].startswith('127.0.0.1:'))

    def test_a_code_and_a_link_for_one_more_device(self):
        r = self.app.post('/api/pair_device', {'anything': 'is ignored'})
        self.assertEqual(r, {'ok': True, 'code': 'ABCD-EFGH', 'link': 'switchboard://pair/1?h=x&s=y', 'expires': 1234})
        method, path, body, headers = self.control.seen[-1]
        self.assertEqual((method, path, body), ('POST', '/pair/new', {}))
        self.assertIn('application/json', headers['content-type'])
        self.assertNotIn('origin', headers)

    def test_remove_a_device(self):
        gone = self.control.devices[1]['id']
        self.assertEqual(self.app.post('/api/keys/devices', {'remove': gone}), {'ok': True})
        self.assertEqual(self.control.seen[-1][:3], ('POST', '/devices/remove', {'id': gone}))
        self.assertEqual(len(self.app.get('/api/keys/devices')['devices']), 1)
        with self.assertRaises(server.HttpError) as e:
            self.app.post('/api/keys/devices', {'remove': gone})
        self.assertEqual(e.exception.code, 502)
        self.assertIn('No paired device has that id.', str(e.exception), "the node's own words")
        asked = len(self.control.seen)
        for bad in (None, '', 5, 'x', '../status', 'a b c d e f g h', ['id-tower-0001'], 'x' * 200):
            with self.assertRaises(server.HttpError) as e:
                self.app.post('/api/keys/devices', {'remove': bad})
            self.assertEqual(e.exception.code, 400, bad)
        self.assertEqual(len(self.control.seen), asked, 'nothing was sent on')

    def test_removing_a_computers_device_removes_the_computer_from_the_list(self):
        f, hub = self.app.fleet, []
        f.add_machine({'name': 'lab', 'label': 'Lab', 'target': 'node:lab', 'os': 'linux'}, by_user=False)
        f.add_machine({'name': 'studio', 'label': 'Studio', 'target': 'node:studio', 'os': 'mac'}, by_user=False)
        f.add_machine({'name': 'tower', 'label': 'Tower', 'target': 'me@y', 'os': 'windows'})
        self.control.devices.append(dict(self.control.devices[0], id='id-studio-0001', machine='studio'))
        self.control.devices.append(dict(self.control.devices[0], id='id-phone-00001', kind='phone', machine='lab'))
        before = [n for n in f.order if n not in ('lab', 'studio')]
        with mock.patch.object(nodes, 'hub_call', lambda method, path, body=None, **kw: hub.append((method, path)) or {'ok': True}):
            # a phone paired under a computer's name is only a phone
            self.assertEqual(self.app.post('/api/keys/devices', {'remove': 'id-phone-00001'}), {'ok': True})
            self.assertIn('lab', f.order)
            # this home computer's own entry stays
            self.assertEqual(self.app.post('/api/keys/devices', {'remove': 'id-studio-0001'}), {'ok': True})
            self.assertIn('studio', f.order)
            # a computer reached over ssh keeps its row
            self.assertEqual(self.app.post('/api/keys/devices', {'remove': 'id-tower-0001'}), {'ok': True})
            self.assertIn('tower', f.order)
            self.assertEqual(hub, [])
            self.assertEqual(self.app.post('/api/keys/devices', {'remove': 'id-lab-0001'}), {'ok': True})
        self.assertEqual(hub, [('POST', '/internal/unpair/lab')])
        self.assertEqual([n for n in f.order if n != 'studio'], before, 'the computer left the list with its device')

    def test_a_hub_that_does_not_answer_does_not_keep_the_row(self):
        f = self.app.fleet
        f.add_machine({'name': 'lab', 'label': 'Lab', 'target': 'node:lab', 'os': 'linux'}, by_user=False)

        def down(*a, **kw):
            raise OSError('the hub is not reachable')
        with mock.patch.object(nodes, 'hub_call', down):
            self.assertEqual(self.app.post('/api/keys/devices', {'remove': 'id-lab-0001'}), {'ok': True})
        self.assertNotIn('lab', f.order)

    def test_a_device_the_node_refuses_to_remove_keeps_its_computer(self):
        f = self.app.fleet
        f.add_machine({'name': 'lab', 'label': 'Lab', 'target': 'node:lab', 'os': 'linux'}, by_user=False)
        real = nodes.control_call

        def refuse(method, path, *a, **kw):
            if path == '/devices/remove':
                raise OSError('That device could not be removed.')
            return real(method, path, *a, **kw)
        with mock.patch.object(nodes, 'control_call', refuse), self.assertRaises(server.HttpError):
            self.app.post('/api/keys/devices', {'remove': 'id-lab-0001'})
        self.assertIn('lab', f.order)

    def test_no_node_there_is_said_plainly(self):
        nodes.CONTROL = 'http://127.0.0.1:9'
        for call in (lambda: self.app.get('/api/keys/devices'), lambda: self.app.post('/api/pair_device', {}),
                     lambda: self.app.post('/api/keys/devices', {'remove': 'id-tower-0001'})):
            with self.assertRaises(server.HttpError) as e:
                call()
            self.assertEqual(e.exception.code, 502)
            self.assertIn('The node on this computer', str(e.exception))

    def test_an_answer_that_is_not_what_was_asked_for_is_not_passed_on(self):
        for fail in ((200, ['devices']), (200, {'ok': True, 'devices': 'many'}), (200, {'ok': True}), (500, {'error': 'x' * 900})):
            self.control.fail = fail
            for call in (lambda: self.app.get('/api/keys/devices'), lambda: self.app.post('/api/pair_device', {})):
                with self.assertRaises(server.HttpError) as e:
                    call()
                self.assertEqual(e.exception.code, 502, fail)
                self.assertLess(len(str(e.exception)), 300)

    def test_who_can_answer_while_this_home_computer_is_away(self):
        """Keys phase 4a: the node says which version each computer runs and whether it took the list of devices,
        and when it last signed that list. Both are passed on, and nothing else is."""
        import keys
        from tests.test_keys import NOW, dev
        self.control.devices = [dev('lab', ver='0.8.23', answers_away=True), dev('tower', ver='0.8.22', answers_away=False),
                                dev('mini'), dev('odd', answers_away='yes', ver=7)]
        self.control.roster = {'epoch': 1, 'seq': 4, 'at': NOW - 3600, 'expires': NOW + 6 * 86400, 'secret': 'x'}
        r = self.app.get('/api/keys/devices')
        self.assertEqual([(d['machine'], d.get('ver'), d.get('answers_away')) for d in r['devices']],
                         [('lab', '0.8.23', True), ('tower', '0.8.22', False), ('mini', None, None), ('odd', None, None)])
        self.assertEqual(r['roster'], {'epoch': 1, 'seq': 4, 'at': NOW - 3600, 'expires': NOW + 6 * 86400})
        machines = [{'name': n, 'label': n.capitalize()} for n in ('studio', 'lab', 'tower', 'mini', 'laptop')]
        rep = keys.readiness('both', machines, r['devices'], {'since': NOW - 30 * 86400, 'seen': {}}, NOW, home='studio')
        self.assertEqual([(m['name'], m['ver'], m['away']) for m in rep['machines']],
                         [('studio', None, None), ('lab', '0.8.23', True), ('tower', '0.8.22', False),
                          ('mini', None, None), ('laptop', None, None)])
        self.assertEqual([keys._row(m, NOW) for m in rep['machines']], [
            '  Studio  this home computer',
            '  Lab  paired, last connected with its key 1 minute ago (directly); can answer for its own chats while '
            'this home computer is away',
            '  Tower  paired, last connected with its key 1 minute ago (directly); needs an update before it can answer '
            'while this home computer is away',
            '  Mini  paired, last connected with its key 1 minute ago (directly)',
            '  Laptop  not paired'])
        # an older node says nothing of the kind, and nothing is made up
        self.control.roster = None
        self.control.devices = [dev('lab')]
        r = self.app.get('/api/keys/devices')
        self.assertEqual((r['roster'], sorted(r['devices'][0])),
                         (None, ['id', 'kind', 'last_seen', 'machine', 'name', 'paired', 'path', 'via']))
        for odd in ('soon', {'at': 'x', 'expires': 1}, {'at': True, 'expires': 2, 'epoch': 1, 'seq': 1}):
            self.control.roster = odd
            self.assertIsNone(self.app.get('/api/keys/devices')['roster'], odd)

    def test_the_control_api_is_only_ever_this_computer(self):
        for elsewhere in ('http://10.0.0.5:8137', 'https://127.0.0.1:8137', 'http://example.com', 'http://localhost.example.com:8137'):
            with self.assertRaises(OSError) as e:
                nodes.control_call('GET', '/devices', control=elsewhere)
            self.assertIn('not on this computer', str(e.exception), elsewhere)


if __name__ == '__main__':
    unittest.main()
