"""Keys on the server side (server/keys.py): server.json's "keys", who still connects by address, the report that
says whether keys only is safe yet, and the downloads port that is not served with keys only."""
import contextlib, io, json, sys, threading, unittest, urllib.error, urllib.request
from pathlib import Path
from tempfile import mkdtemp

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'server'))
import downloads, fleet, keys, nodes, server  # noqa: E402

DAY = 86400
NOW = 1_800_000_000.0
MACH = [{'name': 'studio', 'label': 'Studio', 'target': 'me@x', 'os': 'mac'},
        {'name': 'tower', 'label': 'Tower', 'target': 'me@y', 'os': 'windows'}]
ZEROS = b'\x00' * 64


def write(p, d):
    p.parent.mkdir(parents=True, exist_ok=True)
    p.write_text(json.dumps(d))


def quiet(fn, *a, **kw):
    out = io.StringIO()
    with contextlib.redirect_stdout(out):
        return fn(*a, **kw), out.getvalue()


def config(**kw):
    d = Path(mkdtemp())
    write(d / 'cfg' / 'server.json', dict({'bind': ['0.0.0.0'], 'hosts': ['studio'],
                                           'origins': ['http://10.0.0.5:8130', 'http://studio:8130']}, **kw))
    return quiet(server.ServerConfig, d / 'cfg', d / 'state')


class Quiet:
    def __call__(self, target, req, timeout):
        return {'ok': True, 'sessions': [], 'replaced': []}


def app_for(cfg, clock=None):
    kw = {'clock': clock} if clock else {}
    return server.App(fleet.Fleet([dict(m) for m in MACH], Quiet()), state_dir=Path(mkdtemp()),
                      web_dir=Path(server.__file__).resolve().parent / 'web', server_cfg=cfg, **kw)


class ModeTest(unittest.TestCase):
    def test_an_install_with_no_keys_entry_is_exactly_as_it_was(self):
        c, said = config()
        self.assertEqual(c.keys, 'off')
        self.assertEqual((c.bind, c.origins), (['0.0.0.0'], ['http://10.0.0.5:8130', 'http://studio:8130']))
        self.assertTrue(c.announce)
        self.assertTrue(c.peer_ok('10.0.0.7') and c.peer_ok('100.64.0.5') and c.host_ok('studio'))
        self.assertNotIn('keys', said)
        app = app_for(c)
        self.assertIsNotNone(app.addresses, 'the apps are still told the addresses')
        self.assertEqual(app.fleet.servers, ['http://10.0.0.5:8130', 'http://studio:8130'])
        req = app.fleet._list_req(app.fleet.machines['studio'])
        self.assertEqual(req['servers'], ['http://10.0.0.5:8130', 'http://studio:8130'], 'helpers report where they did')
        self.assertTrue(downloads.serves(c.path.parent), 'and the downloads are served')
        self.assertNotIn('keys', json.loads(c.path.read_text()), 'nothing is written into the file')

    def test_a_first_start_writes_no_keys_entry(self):
        d = Path(mkdtemp())
        c = server.ServerConfig(d / 'cfg', d / 'state', local=lambda: ['10.0.0.9'])
        self.assertEqual((c.keys, c.bind), ('off', ['0.0.0.0']))
        self.assertNotIn('keys', json.loads((d / 'cfg' / 'server.json').read_text()))

    def test_off_by_name_is_the_same_as_no_entry(self):
        c, _ = config(keys='off')
        self.assertEqual((c.keys, c.bind, c.announce), ('off', ['0.0.0.0'], True))

    def test_both_keeps_the_addresses_and_tells_helpers_this_computer_first(self):
        c, _ = config(keys='both')
        self.assertEqual((c.keys, c.bind, c.announce), ('both', ['0.0.0.0'], True))
        self.assertTrue(c.peer_ok('10.0.0.7'))
        app = app_for(c)
        self.assertIsNotNone(app.addresses)
        self.assertEqual(app.fleet._list_req(app.fleet.machines['studio'])['servers'],
                         ['http://127.0.0.1:8130', 'http://10.0.0.5:8130', 'http://studio:8130'])
        self.assertTrue(downloads.serves(c.path.parent))

    def test_only_listens_to_this_computer_whatever_else_the_file_says(self):
        c, _ = config(keys='only', bind=['0.0.0.0', '10.0.0.5'], peers=['any'])
        self.assertEqual((c.keys, c.bind, c.announce), ('only', ['127.0.0.1'], False))
        for a in ('10.0.0.7', '100.64.0.5', '203.0.113.9', '::ffff:10.0.0.7'):
            self.assertFalse(c.peer_ok(a), a)
        self.assertTrue(c.peer_ok('127.0.0.1') and c.peer_ok('::1'))
        app = app_for(c)
        self.assertIsNone(app.addresses)
        self.assertNotIn('addresses', app.summary())
        self.assertEqual(app.fleet._list_req(app.fleet.machines['studio'])['servers'], ['http://127.0.0.1:8130'])
        self.assertFalse(downloads.serves(c.path.parent))

    def test_a_setting_this_server_does_not_know_is_only_and_the_log_says_so(self):
        for bad in ('onyl', 'Only', '', 5, True, None, ['only'], {'mode': 'both'}):
            c, said = config(keys=bad, peers=['any'])
            self.assertEqual((c.keys, c.bind, c.announce), ('only', ['127.0.0.1'], False), bad)
            self.assertFalse(c.peer_ok('10.0.0.7'), bad)
            self.assertIn('answers this computer only', said)
            self.assertFalse(downloads.serves(c.path.parent), bad)

    def test_a_damaged_server_json_is_still_this_computer_only(self):
        for mode in (None, 'both'):
            d = Path(mkdtemp())
            (d / 'cfg').mkdir()
            (d / 'cfg' / 'server.json').write_bytes(ZEROS)
            c, said = quiet(server.ServerConfig, d / 'cfg', d / 'state', local=lambda: ['10.0.0.9'])
            self.assertEqual((c.bind, c.origins, c.announce), (['127.0.0.1'], [], False))
            self.assertFalse(c.peer_ok('10.0.0.7'))
            self.assertIn('this computer only', said)
            self.assertFalse(downloads.serves(d / 'cfg'), 'and the downloads are not served')

    def test_a_damaged_file_goes_back_to_the_mode_in_its_last_good_copy(self):
        d = Path(mkdtemp())
        mine = {'bind': ['0.0.0.0'], 'hosts': [], 'origins': ['http://10.0.0.5:8130'], 'keys': 'only'}
        write(d / 'cfg' / 'server.json', mine)
        quiet(server.ServerConfig, d / 'cfg', d / 'state')  # a good start keeps a copy
        (d / 'cfg' / 'server.json').write_bytes(ZEROS)
        self.assertEqual(keys.read_mode(d / 'cfg' / 'server.json'), 'only', 'read by the downloads before the server mends it')
        c, _ = quiet(server.ServerConfig, d / 'cfg', d / 'state')
        self.assertEqual((c.keys, c.bind), ('only', ['127.0.0.1']))
        self.assertEqual(json.loads((d / 'cfg' / 'server.json').read_text())['keys'], 'only', 'the entry survives the mend')

    def test_reading_the_mode_beside_the_server_changes_nothing_on_disk(self):
        d = Path(mkdtemp())
        p = d / 'server.json'
        self.assertEqual(keys.read_mode(p), 'off', 'no file: an install that never chose')
        p.write_bytes(ZEROS)
        self.assertEqual(keys.read_mode(p), 'only')
        self.assertEqual(sorted(x.name for x in d.iterdir()), ['server.json'])
        self.assertEqual(p.read_bytes(), ZEROS)
        p.write_text('[1, 2]')
        self.assertEqual(keys.read_mode(p), 'only', 'not an object')
        p.write_text('{"keys": "both"}')
        self.assertEqual(keys.read_mode(p), 'both')

    def test_the_server_is_announced_only_when_an_address_can_reach_it(self):
        told = []
        how = lambda port, cfg, log: told.append(port) or 'dns-sd'
        c, _ = config()
        self.assertEqual(server.announce(c, how), 'dns-sd')
        c, _ = config(keys='both')
        self.assertEqual(server.announce(c, how), 'dns-sd')
        c, _ = config(keys='only')
        r, said = quiet(server.announce, c, how)
        self.assertIsNone(r)
        self.assertEqual(len(told), 2, 'keys only: nothing is announced')
        self.assertIn('not announced', said)


class HelperServersTest(unittest.TestCase):
    def test_the_list_for_each_mode(self):
        o = ['http://10.0.0.5:8130', 'http://127.0.0.1:8130', 'http://studio:8130']
        self.assertEqual(fleet.helper_servers(o), o)
        self.assertEqual(fleet.helper_servers(o, 'off'), o)
        self.assertEqual(fleet.helper_servers(o, 'both'), ['http://127.0.0.1:8130', 'http://10.0.0.5:8130', 'http://studio:8130'])
        self.assertEqual(fleet.helper_servers(o, 'only'), ['http://127.0.0.1:8130'])
        self.assertEqual(fleet.helper_servers([], 'both', port=9000), ['http://127.0.0.1:9000'])
        self.assertEqual(fleet.helper_servers([], 'off'), [])

    def test_a_fleet_nobody_told_about_keys_sends_what_it_always_sent(self):
        f = fleet.Fleet([dict(m) for m in MACH], Quiet())
        self.assertNotIn('servers', f._list_req(f.machines['studio']))
        f.servers = ['http://10.0.0.5:8130']
        self.assertEqual(f._list_req(f.machines['studio'])['servers'], ['http://10.0.0.5:8130'])

    def test_the_helper_takes_the_address_it_is_given(self):
        from tests.helpers import fresh_sb
        sb = fresh_sb(Path(mkdtemp()))
        self.assertTrue(sb.SERVER_URL_RE.fullmatch('http://127.0.0.1:8130'))


class DirectTest(unittest.TestCase):
    def setUp(self):
        self.now = [NOW]
        self.p = Path(mkdtemp()) / 'state' / 'direct_seen.json'

    def direct(self):
        return keys.Direct(self.p, clock=lambda: self.now[0])

    def test_requests_from_this_computer_are_not_noted(self):
        d = self.direct()
        for a in ('127.0.0.1', '127.8.9.1', '::1', '::ffff:127.0.0.1'):
            d.note(a, 'SwitchboardApp/1')
        self.assertEqual(d.view(), {'since': NOW, 'seen': {}})

    def test_each_kind_of_client_is_told_apart(self):
        for ua, kind in (('SwitchboardApp/0.8 iOS', 'iPhone'), ('SwitchboardAndroid/0.8', 'Android'),
                         ('Mozilla/5.0 SwitchboardDesktop/0.8', 'desktop'), ('Python-urllib/3.9', 'helper'),
                         ('Mozilla/5.0 (Macintosh) Safari', 'web'), ('curl/8', 'other'), ('', 'other'), (None, 'other')):
            self.assertEqual(keys.client_kind(ua), kind, ua)

    def test_what_was_seen_survives_a_restart_and_is_not_written_on_every_request(self):
        d = self.direct()
        self.assertEqual(json.loads(self.p.read_text()), {'since': NOW, 'seen': {}}, 'when the watching began is kept')
        d.note('10.0.0.7', 'SwitchboardApp/1')
        first = self.p.read_bytes()
        self.assertEqual(json.loads(first)['seen'], {'iPhone': NOW}, 'a first sight is written at once')
        self.now[0] += 30
        d.note('10.0.0.7', 'SwitchboardApp/1')
        self.assertEqual(self.p.read_bytes(), first, 'not again within ten minutes')
        self.assertEqual(d.view()['seen'], {'iPhone': NOW + 30})
        d.note('::ffff:10.0.0.9', 'Python-urllib/3.9')
        self.assertEqual(json.loads(self.p.read_text())['seen'], {'iPhone': NOW + 30, 'helper': NOW + 30})
        self.now[0] += keys.SAVE_EVERY
        d.note('10.0.0.7', 'SwitchboardApp/1')
        self.now[0] += DAY
        again = self.direct()
        self.assertEqual(again.view(), {'since': NOW, 'seen': {'iPhone': NOW + 30 + keys.SAVE_EVERY, 'helper': NOW + 30}})

    def test_a_lost_file_starts_the_watch_again(self):
        self.direct().note('10.0.0.7', 'SwitchboardApp/1')
        self.p.write_bytes(ZEROS)
        self.p.with_name(self.p.name + '.bak').unlink(missing_ok=True)
        self.now[0] += 30 * DAY
        d, _ = quiet(self.direct)
        self.assertEqual(d.view(), {'since': NOW + 30 * DAY, 'seen': {}}, 'nobody can say what happened meanwhile')

    def test_odd_contents_are_not_trusted(self):
        write(self.p, {'since': 'yesterday', 'seen': {'iPhone': 'x', 'web': -1, 'helper': True, 'nope': 5, 'desktop': NOW - 5}})
        self.assertEqual(self.direct().view(), {'since': NOW, 'seen': {'desktop': NOW - 5}})


def dev(machine, last_seen=NOW - 60, **kw):
    return dict({'id': f'id-{machine}-0001', 'name': machine, 'kind': 'computer', 'machine': machine,
                 'paired': NOW - 20 * DAY, 'via': 'code', 'last_seen': last_seen, 'path': 'direct'}, **kw)


class ReadinessTest(unittest.TestCase):
    MACHINES = [{'name': 'studio', 'label': 'Studio'}, {'name': 'tower', 'label': 'Tower'}, {'name': 'lab', 'label': 'Lab box'}]

    def ready(self, mode='both', devices=None, seen=None, since=NOW - 30 * DAY, **kw):
        if devices is None:
            devices = [dev('tower'), dev('lab', path='relay')]
        kw.setdefault('home', 'studio')
        return keys.readiness(mode, self.MACHINES, devices, {'since': since, 'seen': seen or {}}, NOW, **kw)

    def kinds(self, r):
        return [b['kind'] for b in r['blockers']]

    def test_ready_when_every_computer_came_through_the_tunnel_and_nothing_asks_by_address(self):
        r = self.ready()
        self.assertEqual((r['ok'], r['ready'], r['blockers'], r['keys'], r['days']), (True, True, [], 'both', 7))
        self.assertEqual([(m['name'], m['home'], m['paired'], m['path']) for m in r['machines']],
                         [('studio', True, False, None), ('tower', False, True, 'direct'), ('lab', False, True, 'relay')])

    def test_a_computer_that_is_not_paired_blocks_it(self):
        r = self.ready(devices=[dev('tower')])
        self.assertFalse(r['ready'])
        self.assertEqual([(b['kind'], b['machine'], b['paired']) for b in r['blockers']], [('machine', 'lab', False)])
        self.assertIn('Lab box is not paired', r['blockers'][0]['text'])

    def test_a_paired_computer_never_seen_through_the_tunnel_blocks_it(self):
        for never in (None, 0, False, 'soon'):
            r = self.ready(devices=[dev('tower'), dev('lab', last_seen=never)])
            self.assertEqual([(b['kind'], b['machine'], b['paired']) for b in r['blockers']], [('machine', 'lab', True)], never)

    def test_a_phone_paired_under_a_computers_name_does_not_count_for_it(self):
        r = self.ready(devices=[dev('tower'), dev('lab', kind='phone')])
        self.assertEqual(self.kinds(r), ['machine'])

    def test_without_knowing_which_computer_is_home_every_one_must_be_paired(self):
        r = self.ready(home=None)
        self.assertEqual([b['machine'] for b in r['blockers']], ['studio'])

    def test_a_recent_request_by_address_blocks_it_and_an_old_one_does_not(self):
        r = self.ready(seen={'iPhone': NOW - 2 * 3600, 'web': NOW - 8 * DAY, 'helper': NOW - 3 * DAY})
        self.assertEqual([(b['kind'], b['client']) for b in r['blockers']], [('direct', 'iPhone'), ('direct', 'helper')])
        self.assertIn('The iPhone app connected by address 2 hours ago', r['blockers'][0]['text'])
        self.assertEqual(r['blockers'][0]['text'], 'The iPhone app connected by address 2 hours ago. It would stop '
                         'working. Pair the phone with this home computer in the app first.')
        self.assertNotIn('paired_since', r['blockers'][0])
        self.assertEqual(r['blockers'][1]['text'], "A computer's helper connected by address 3 days ago. "
                         'It would stop working.')
        self.assertEqual([d['client'] for d in r['direct']], ['iPhone', 'helper', 'web'])
        self.assertTrue(self.ready(seen={'iPhone': NOW - 2 * 3600}, days=1)['blockers'])
        self.assertTrue(self.ready(seen={'web': NOW - 8 * DAY})['ready'])
        self.assertFalse(self.ready(seen={'web': NOW - 8 * DAY}, days=30)['ready'])

    def test_paired_phones_are_listed_and_a_phone_that_came_with_its_key_since_is_said_so(self):
        phone = lambda name, seen: dict(dev(None, last_seen=seen), kind='phone', name=name)
        base = [dev('tower'), dev('lab')]
        r = self.ready(devices=base + [phone('Pocket', NOW - 600), phone('Spare', None)])
        self.assertTrue(r['ready'], 'a paired phone is nothing to wait for')
        self.assertEqual(r['phones'], [{'name': 'Pocket', 'seen': NOW - 600}, {'name': 'Spare', 'seen': None}])
        self.assertEqual(self.ready()['phones'], [])
        # by address two hours ago, with its key ten minutes ago: still a blocker, since nobody can say it is the
        # same phone, and the sentence says what is known
        r = self.ready(devices=base + [phone('Pocket', NOW - 600)], seen={'Android': NOW - 2 * 3600})
        self.assertEqual([(b['kind'], b['client'], b['paired_since']) for b in r['blockers']],
                         [('direct', 'Android', True)])
        self.assertEqual(r['blockers'][0]['text'], 'The Android app connected by address 2 hours ago. A paired phone '
                         'has connected with its key since, so this may be that phone before it was paired. A phone '
                         'that is not paired would stop working.')
        # a paired phone last seen before that, or never: nothing is known to have moved
        for seen in (NOW - 3 * 3600, None):
            r = self.ready(devices=base + [phone('Pocket', seen)], seen={'iPhone': NOW - 2 * 3600})
            self.assertIn('Pair the phone with this home computer in the app first.', r['blockers'][0]['text'])
            self.assertNotIn('paired_since', r['blockers'][0])
        for r in (self.ready(seen={'iPhone': NOW - 60, 'Android': NOW - 60}),
                  self.ready(devices=base + [phone('Pocket', NOW)], seen={'iPhone': NOW - 60})):
            self.assertNotIn('cannot connect with a key', ' '.join(b['text'] for b in r['blockers']))

    def test_too_short_a_watch_blocks_it(self):
        r = self.ready(since=NOW - 2 * DAY)
        self.assertEqual([(b['kind'], b['watched_days']) for b in r['blockers']], [('watching', 2)])
        self.assertIn('2 of 7 days', r['blockers'][0]['text'])

    def test_a_node_that_does_not_answer_blocks_it_without_blaming_each_computer(self):
        r = keys.readiness('both', self.MACHINES, None, {'since': NOW - 30 * DAY, 'seen': {}}, NOW, node_error='the node is not reachable')
        self.assertEqual(self.kinds(r), ['node'])
        self.assertIn('the node is not reachable', r['blockers'][0]['text'])

    def test_keys_off_and_a_lost_server_json_block_it(self):
        self.assertEqual(self.kinds(self.ready(mode='off')), ['mode'])
        self.assertEqual(self.kinds(self.ready(settings_lost=True)), ['settings'])

    def test_a_computer_that_was_removed_is_not_waited_for(self):
        # Lab was removed (its link is gone and it has no paired device); the same computer paired again is "lab2"
        machines = self.MACHINES[:2] + [{'name': 'lab', 'label': 'Lab box', 'gone': True}, {'name': 'lab2', 'label': 'Lab box'}]
        both = {'since': NOW - 30 * DAY, 'seen': {}}
        r = keys.readiness('both', machines, [dev('tower'), dev('lab2')], both, NOW, home='studio')
        self.assertEqual((r['ready'], r['blockers']), (True, []))
        self.assertEqual([(m['name'], m['paired'], m['removed']) for m in r['machines']],
                         [('studio', False, False), ('tower', True, False), ('lab', False, True), ('lab2', True, False)])
        # a link that is gone while its key is still paired is a computer that can still be cut off
        r = keys.readiness('both', machines, [dev('tower'), dev('lab', last_seen=None), dev('lab2')], both, NOW, home='studio')
        self.assertEqual([(b['kind'], b['machine']) for b in r['blockers']], [('machine', 'lab')])
        self.assertFalse(r['machines'][2]['removed'])
        # and with no answer from the node nothing is called removed
        r = keys.readiness('both', machines, None, both, NOW, home='studio')
        self.assertEqual([m['removed'] for m in r['machines']], [False] * 4)

    def test_the_days_can_be_set_in_server_json(self):
        self.assertEqual(keys.days_of({}), 7)
        self.assertEqual(keys.days_of(None), 7)
        self.assertEqual([keys.days_of({'keys_ready_days': v}) for v in (1, 3, 90)], [1, 3, 90])
        self.assertEqual([keys.days_of({'keys_ready_days': v}) for v in (0, -4, 91, 10 ** 9)], [1, 1, 90, 90], 'kept to 1..90')
        for odd in ('3', 3.5, True, None, [3], {'days': 3}):
            self.assertEqual(keys.days_of({'keys_ready_days': odd}), 7, odd)
        d = Path(mkdtemp())
        self.assertEqual(keys.read_days(d / 'server.json'), 7, 'no file')
        write(d / 'server.json', {'keys': 'both', 'keys_ready_days': 2})
        self.assertEqual(keys.read_days(d / 'server.json'), 2)
        (d / 'server.json').write_text('{half')
        self.assertEqual(keys.read_days(d / 'server.json'), 7)
        self.assertEqual(keys.read_mode(d / 'server.json'), 'only', 'the mode of a file that does not read is still "only"')

    def test_the_words_are_plain(self):
        r = keys.readiness('off', self.MACHINES, [dev('lab', last_seen=None)], {'since': NOW, 'seen': {k: NOW - 5 for k in keys.KINDS}},
                           NOW, settings_lost=True)
        for b in r['blockers']:
            self.assertNotIn(chr(0x2014), b['text'])
            self.assertNotIn('!', b['text'])
            self.assertNotIn('machine', b['text'].lower())


class FakeControl:
    """Stands in for the node's control API (plan section 3.5) until the node has one."""

    def __init__(self):
        from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
        self.devices = [dev('lab'), dev('tower', last_seen=None, secret='never passed on')]
        self.machine, self.seen, self.fail, self.roster = 'studio', [], None, None
        me = self

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

            def _go(self, method):
                n = int(self.headers.get('Content-Length') or 0)
                body = json.loads(self.rfile.read(n) or b'{}') if n else None
                me.seen.append((method, self.path, body, {k.lower(): v for k, v in self.headers.items()}))
                if me.fail:
                    return self._send(*me.fail)
                if (method, self.path) == ('GET', '/status'):
                    return self._send(200, {'ok': True, 'machine': me.machine})
                if (method, self.path) == ('GET', '/devices'):
                    more = {} if me.roster is None else {'roster': me.roster}
                    return self._send(200, dict({'ok': True, 'devices': me.devices}, **more))
                if (method, self.path) == ('POST', '/pair/new'):
                    return self._send(200, {'ok': True, 'code': 'ABCD-EFGH', 'link': 'switchboard://pair/1?h=x&s=y',
                                            'expires': 1234, 'secret': 'never passed on'})
                if (method, self.path) == ('POST', '/devices/remove'):
                    if not any(d['id'] == body.get('id') for d in me.devices):
                        return self._send(404, {'ok': False, 'error': 'No paired device has that id.'})
                    me.devices = [d for d in me.devices if d['id'] != body['id']]
                    return self._send(200, {'ok': True})
                self._send(404, {'ok': False, 'error': 'not found'})

            def do_GET(self):
                self._go('GET')

            def do_POST(self):
                self._go('POST')

        self.httpd = ThreadingHTTPServer(('127.0.0.1', 0), H)
        self.url = f'http://127.0.0.1:{self.httpd.server_address[1]}'
        threading.Thread(target=self.httpd.serve_forever, daemon=True).start()

    def close(self):
        self.httpd.shutdown()
        self.httpd.server_close()


class ReadinessRouteTest(unittest.TestCase):
    """GET /api/keys/readiness over HTTP, with requests that come from this computer and from elsewhere."""

    def setUp(self):
        self.control = FakeControl()
        self.addCleanup(self.control.close)
        old = nodes.CONTROL
        nodes.CONTROL = self.control.url
        self.addCleanup(setattr, nodes, 'CONTROL', old)
        self.now = [NOW]
        self.peer = ['127.0.0.1']
        c, _ = config(keys='both')
        self.app = app_for(c, clock=lambda: self.now[0])
        app, peer = self.app, self.peer

        class From(server.make_handler(app)):
            def setup(self):
                super().setup()
                self.client_address = (peer[0], 50000)
        httpd = server.Server(('127.0.0.1', 0), From)
        threading.Thread(target=httpd.serve_forever, daemon=True).start()
        self.addCleanup(httpd.server_close)
        self.addCleanup(httpd.shutdown)
        self.port = httpd.server_address[1]

    def get(self, path, ua='Mozilla/5.0'):
        r = urllib.request.Request(f'http://127.0.0.1:{self.port}{path}', headers={'Host': 'localhost', 'User-Agent': ua})
        try:
            with urllib.request.urlopen(r, timeout=5) as resp:
                return resp.status, json.loads(resp.read())
        except urllib.error.HTTPError as e:
            return e.code, json.loads(e.read())

    def test_the_report_over_http(self):
        st, r = self.get('/api/keys/readiness')
        self.assertEqual(st, 200)
        self.assertEqual((r['keys'], r['ready'], r['days']), ('both', False, 7))
        self.assertEqual([(b['kind'], b.get('machine')) for b in r['blockers']], [('machine', 'tower'), ('watching', None)])
        self.assertEqual(r['direct'], [], 'asking from this computer is not a request by address')
        # the tower connects, a week passes with nothing asking by address
        self.control.devices[1]['last_seen'] = NOW
        self.now[0] += 8 * DAY
        st, r = self.get('/api/keys/readiness')
        self.assertEqual((r['ready'], r['blockers']), (True, []))

    def test_a_request_by_address_shows_in_the_report(self):
        self.control.devices[1]['last_seen'] = NOW
        self.now[0] += 8 * DAY
        self.peer[0] = '10.0.0.7'
        self.assertEqual(self.get('/api/summary', ua='SwitchboardApp/0.8')[0], 200)
        self.peer[0] = '127.0.0.1'
        st, r = self.get('/api/keys/readiness')
        self.assertFalse(r['ready'])
        self.assertEqual([(b['kind'], b['client']) for b in r['blockers']], [('direct', 'iPhone')])
        self.now[0] += 3 * DAY
        self.assertFalse(self.get('/api/keys/readiness?days=7')[1]['ready'])
        self.assertTrue(self.get('/api/keys/readiness?days=2')[1]['ready'])
        self.assertEqual(self.get('/api/keys/readiness?days=nonsense')[1]['days'], 7)

    def test_a_refused_address_is_not_noted(self):
        self.peer[0] = '203.0.113.9'
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            self.assertEqual(self.get('/api/summary')[0], 403)
        self.assertEqual(self.app.direct.view()['seen'], {})

    def test_the_days_come_from_server_json_without_a_restart(self):
        self.control.devices[1]['last_seen'] = NOW
        self.now[0] += 3 * DAY
        r = self.get('/api/keys/readiness')[1]
        self.assertEqual((r['days'], r['ready'], [b['kind'] for b in r['blockers']]), (7, False, ['watching']))
        d = json.loads(self.app.server_cfg.path.read_text())
        write(self.app.server_cfg.path, dict(d, keys_ready_days=2))
        r = self.get('/api/keys/readiness')[1]
        self.assertEqual((r['days'], r['ready']), (2, True))
        self.assertEqual(self.get('/api/keys/readiness?days=30')[1]['days'], 30, 'a question that names its own days gets them')
        write(self.app.server_cfg.path, dict(d, keys_ready_days=500))
        self.assertEqual(self.get('/api/keys/readiness')[1]['days'], 90)

    def test_the_report_says_what_an_app_may_offer(self):
        r = self.get('/api/keys/readiness')[1]
        self.assertEqual((r['switch'], r['here']), (True, True), 'this server changes the mode, and this computer asked')
        self.peer[0] = '10.0.0.7'
        self.assertEqual(self.get('/api/keys/readiness')[1]['here'], False, 'another computer on the network')
        self.app.server_cfg.hosts.add('10.0.0.5')  # (a first start puts this computer's addresses there)
        self.peer[0] = '10.0.0.5'
        self.assertEqual(self.get('/api/keys/readiness')[1]['here'], True, "the server's own address is this computer too")
        self.peer[0] = '::ffff:127.0.0.1'
        self.assertEqual(self.get('/api/keys/readiness')[1]['here'], True)

    def test_a_computer_the_hub_no_longer_lists_is_not_waited_for(self):
        f = self.app.fleet
        f.add_machine({'name': 'lab', 'label': 'Lab', 'target': 'node:lab', 'os': 'linux'}, by_user=False)
        f.add_machine({'name': 'old', 'label': 'Old laptop', 'target': 'node:old', 'os': 'mac'}, by_user=False)
        self.control.devices = [dev('lab'), dev('tower')]
        self.control.machine = 'studio'
        self.now[0] += 8 * DAY
        r = self.get('/api/keys/readiness')[1]
        self.assertEqual([(b['kind'], b['machine']) for b in r['blockers']], [('machine', 'old')])
        f.unlinked.add('old')  # what nodes.sync does when the hub stops listing it
        r = self.get('/api/keys/readiness')[1]
        self.assertEqual((r['ready'], r['blockers']), (True, []))
        self.assertEqual([(m['name'], m['removed']) for m in r['machines']],
                         [('studio', False), ('tower', False), ('lab', False), ('old', True)])
        f.unlinked.add('tower')  # reached over ssh: never "removed" by a hub that does not list it
        self.control.devices = [dev('lab')]
        r = self.get('/api/keys/readiness')[1]
        self.assertEqual([(b['kind'], b['machine']) for b in r['blockers']], [('machine', 'tower')])

    def test_no_node_yet_is_a_blocker_and_not_an_error(self):
        nodes.CONTROL = 'http://127.0.0.1:9'
        st, r = self.get('/api/keys/readiness')
        self.assertEqual(st, 200)
        self.assertEqual([b['kind'] for b in r['blockers']], ['node', 'watching'])
        self.assertIn('did not answer', r['blockers'][0]['text'])


class DownloadsTest(unittest.TestCase):
    def home(self, **cfg):
        d = Path(mkdtemp())
        if cfg:
            write(d / 'cfg' / 'server.json', cfg)
        (d / 'files' / 'node').mkdir(parents=True)
        (d / 'files' / 'node' / 'SHA256SUMS').write_text('abc  switchboard-node-mac\n')
        return d

    def test_the_folder_is_served_as_before(self):
        d = self.home(bind=['0.0.0.0'])
        self.assertTrue(downloads.serves(d / 'cfg'))
        httpd = downloads.make_server(d / 'files', 0, '127.0.0.1')
        threading.Thread(target=httpd.serve_forever, daemon=True).start()
        self.addCleanup(httpd.shutdown)
        self.addCleanup(httpd.server_close)
        err = io.StringIO()
        with contextlib.redirect_stderr(err):
            with urllib.request.urlopen(f'http://127.0.0.1:{httpd.server_address[1]}/node/SHA256SUMS', timeout=5) as r:
                self.assertEqual(r.read(), b'abc  switchboard-node-mac\n')

    def test_with_keys_only_it_stays_up_and_listens_on_nothing(self):
        d = self.home(keys='only')
        waited = []
        with mock_make_server() as made:
            r, said = quiet(downloads.main, ['--directory', str(d / 'files'), '--port', '0', '--config', str(d / 'cfg')],
                            wait=lambda: waited.append(1))
        self.assertEqual((r, waited, made), (0, [1], []))
        self.assertIn('not served', said)

    def test_no_server_json_at_all_serves(self):
        self.assertTrue(downloads.serves(self.home() / 'cfg'))


@contextlib.contextmanager
def mock_make_server():
    made, real = [], downloads.make_server
    downloads.make_server = lambda *a, **kw: made.append(a) or (_ for _ in ()).throw(AssertionError('it listened'))
    try:
        yield made
    finally:
        downloads.make_server = real


if __name__ == '__main__':
    unittest.main()
