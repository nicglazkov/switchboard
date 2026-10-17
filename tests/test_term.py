import base64, hashlib, hmac, json, os, stat, sys, threading, unittest, urllib.request, urllib.error
from pathlib import Path
from tempfile import mkdtemp
sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'server'))
import fleet, server, term  # noqa: E402

MACH = [{'name': 'desk', 'label': 'Desk', 'target': 'me@x', 'os': 'mac'},
        {'name': 'tower', 'label': 'Tower', 'target': 'me@y', 'os': 'windows'}]
SID = {k: f'{k * 8}-1111-2222-3333-444444444444' for k in 'abcdef'}


def sess(sid, **kw):
    d = {'sid': sid, 'title': 'Chat ' + sid[:2], 'ctx': 20_000, 'cold': False, 'mtime': 100.0, 'kind': None, 'status': None,
         'bg_id': None, 'url': None, 'folder': 'p', 'cwd': '/p', 'handoff': False, 'model': 'claude-opus-5-5'}
    d.update(kw)
    return d


class Transport:
    def __init__(self):
        self.down = set()
        self.sessions = {
            'desk': [sess(SID['a'], kind='background', status='busy', bg_id='aaaaaaaa', title='Tax summary'),
                     sess(SID['b'], kind='interactive', status='idle'),
                     sess(SID['c'], kind='background', status='stopped', bg_id='cccccccc'),
                     sess(SID['d']),
                     sess(SID['e'], kind='background', status=None, state='blocked', bg_id='eeeeeeee')],
            'tower': [sess(SID['f'], kind='background', status='idle', bg_id='ffffffff')]}

    def __call__(self, target, req, timeout):
        name = 'desk' if target == 'me@x' else 'tower'
        if name in self.down:
            raise OSError('ssh: connect timed out')
        return {'ok': True, 'sessions': self.sessions[name], 'replaced': []}


def unpack(ticket):
    body, sig = ticket.split('.')
    return json.loads(base64.urlsafe_b64decode(body + '=' * (-len(body) % 4))), body, sig


class TicketTest(unittest.TestCase):
    def setUp(self):
        self.dir = Path(mkdtemp())
        self.t = Transport()
        self.fleet = fleet.Fleet(MACH, self.t)
        self.fleet.poll()
        self.terms = term.Terms(self.fleet, self.dir, key_path=self.dir / 'cfg' / 'term.key', clock=lambda: 1_791_000_000,
                                env={'SWITCHBOARD_ORIGINS': 'http://10.0.0.5:8150, http://localhost:8151/'},
                                origins=('http://10.0.0.5:8130', 'http://100.64.0.5:8130'))

    def ticket(self, body, host='10.0.0.5:8130'):
        return self.terms.ticket(body, host, server.SID_RE)

    def test_a_running_background_chat_gets_a_ticket(self):
        r = self.ticket({'machine': 'desk', 'sid': SID['a']})
        self.assertEqual({k: r[k] for k in ('ok', 'bg', 'title', 'machine', 'label')},
                         {'ok': True, 'bg': 'aaaaaaaa', 'title': 'Tax summary', 'machine': 'desk', 'label': 'Desk'})
        self.assertTrue(r['ws'].startswith('ws://10.0.0.5:8132/term?t='))
        claims, body, sig = unpack(r['ws'].split('t=', 1)[1])
        self.assertEqual((claims['m'], claims['b'], claims['x']), ('desk', 'aaaaaaaa', 1_791_000_060))
        self.assertRegex(claims['n'], r'^[0-9a-f]{32}$')
        key = (self.dir / 'cfg' / 'term.key').read_bytes()
        want = base64.urlsafe_b64encode(hmac.new(key, body.encode(), hashlib.sha256).digest()).rstrip(b'=').decode()
        self.assertEqual(sig, want)

    def test_waiting_and_idle_background_chats_open_too(self):
        self.assertEqual(self.ticket({'machine': 'desk', 'sid': SID['e']})['bg'], 'eeeeeeee')
        self.assertEqual(self.ticket({'machine': 'tower', 'sid': SID['f']})['bg'], 'ffffffff')

    def test_why_it_cannot_open(self):
        r = self.ticket({'machine': 'desk', 'sid': SID['b']})
        self.assertEqual({k: r[k] for k in ('ok', 'need', 'error', 'machine', 'label', 'sid')},
                         {'ok': False, 'need': 'terminal', 'error': "This chat is open in a terminal on Desk, so it can't be opened here.",
                          'machine': 'desk', 'label': 'Desk', 'sid': SID['b']})
        self.assertTrue(r['title'])
        for k in 'cd':
            r = self.ticket({'machine': 'desk', 'sid': SID[k]})
            self.assertEqual((r['ok'], r['need']), (False, 'resume'), k)
            self.assertIn('Resume', r['error'])
        self.t.down.add('tower')
        self.fleet.poll()
        r = self.ticket({'machine': 'tower', 'sid': SID['f']})
        self.assertEqual((r['ok'], r['need']), (False, 'offline'))
        self.assertIn('Tower is offline', r['error'])

    def test_after_resume_the_bg_id_is_enough(self):
        r = self.ticket({'machine': 'desk', 'bg': 'cccccccc'})   # the list still says stopped; the resume job knows better
        self.assertEqual((r['ok'], r['bg'], r['title']), (True, 'cccccccc', 'Chat cc'))
        r = self.ticket({'machine': 'desk', 'bg': '0123abcd'})   # not on the list yet at all
        self.assertEqual((r['ok'], r['bg'], r['title']), (True, '0123abcd', None))

    def test_bad_requests(self):
        for body in ({'machine': 'nope', 'sid': SID['a']}, {'sid': SID['a']}, {'machine': 'desk'}, {'machine': 'desk', 'sid': 'a b'},
                     {'machine': 'desk', 'bg': 'xyz'}, {'machine': 'desk', 'bg': 'AAAAAAAA'}, {'machine': 'desk', 'bg': 5}):
            with self.assertRaises(term.BadRequest, msg=body):
                self.ticket(body)
        with self.assertRaises(term.NotFound):
            self.ticket({'machine': 'desk', 'sid': SID['f']})

    def test_ws_host_is_the_one_the_client_used(self):
        for host, want in (('100.64.0.5:8130', 'ws://100.64.0.5:8132/'), ('server', 'ws://server:8132/'),
                           ('[fd7a::1]:8130', 'ws://[fd7a::1]:8132/'), ('10.0.0.5:8150', 'ws://10.0.0.5:8132/')):
            self.assertTrue(self.ticket({'machine': 'desk', 'sid': SID['a']}, host)['ws'].startswith(want), host)

    def test_tickets_are_all_different(self):
        a = self.ticket({'machine': 'desk', 'sid': SID['a']})['ws']
        b = self.ticket({'machine': 'desk', 'sid': SID['a']})['ws']
        self.assertNotEqual(a, b)

    def test_machine_list_for_the_hub(self):
        self.terms.write_machines()
        d = json.loads((self.dir / 'machines.json').read_text())
        self.assertEqual(d['machines'], MACH)
        self.assertEqual(d['origins'][:2], ['http://10.0.0.5:8130', 'http://100.64.0.5:8130'])
        self.assertIn('http://10.0.0.5:8150', d['origins'])
        self.assertIn('http://localhost:8151', d['origins'])

    def test_key_file(self):
        p = self.dir / 'k' / 'term.key'
        a = term.load_or_create_key(p)
        self.assertEqual(len(a), 32)
        self.assertEqual(term.load_or_create_key(p), a)
        self.assertEqual(stat.S_IMODE(os.stat(p).st_mode), 0o600)
        self.assertEqual(stat.S_IMODE(os.stat(p.parent).st_mode), 0o700)
        self.assertEqual([x.name for x in p.parent.iterdir()], ['term.key'])  # no temp file left behind
        p.write_bytes(b'short')
        with self.assertRaises(OSError):
            term.load_or_create_key(p)

    def test_same_vector_as_the_hub(self):
        """native/crates/sb-hub/src/ticket.rs checks this exact ticket."""
        t = term.mint(bytes(range(32)), 'desk', 'c03ef0d9', 1_791_000_000, nonce='00112233445566778899aabbccddeeff')
        self.assertEqual(t, 'eyJtIjogImRlc2siLCAiYiI6ICJjMDNlZjBkOSIsICJ4IjogMTc5MTAwMDA2MCwgIm4iOiAiMDAxMTIyMzM0NDU1NjY3Nzg4OTlhYWJiY2NkZGVlZmYifQ'
                            '.5ttdVCRzjRL4eEeV7w6PyJnxf2sfnQwToqAVR-rojtU')

    def test_origins_from_the_environment(self):
        self.assertEqual(term.extra_origins({'SWITCHBOARD_ORIGINS': ' http://A:1/ ,http://b:2  http://c:3'}),
                         ['http://a:1', 'http://b:2', 'http://c:3'])
        self.assertEqual(term.extra_origins({}), [])


class TermHttpTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.state = Path(mkdtemp())
        (cls.state / 'cfg').mkdir()
        (cls.state / 'cfg' / 'server.json').write_text(json.dumps({'bind': ['10.0.0.5'], 'hosts': ['10.0.0.5', '100.64.0.5', 'server'],
                                                                   'origins': ['http://10.0.0.5:8130', 'http://100.64.0.5:8130']}))
        cfg = server.ServerConfig(cls.state / 'cfg', cls.state)
        cls.app = server.App(fleet.Fleet(MACH, Transport()), state_dir=cls.state,
                             web_dir=Path(__file__).resolve().parents[1] / 'server' / 'web', guard_delay=0.05,
                             cfg_dir=cls.state / 'cfg', server_cfg=cfg)
        cls.app.terms.key_path = cls.state / 'term.key'
        cls.app.fleet.poll()
        cls.httpd = server.make_server('127.0.0.1', 0, cls.app)
        cls.port = cls.httpd.server_address[1]
        threading.Thread(target=cls.httpd.serve_forever, daemon=True).start()

    @classmethod
    def tearDownClass(cls):
        cls.httpd.shutdown()

    def req(self, path, body=None, headers=None, host='10.0.0.5:8130'):
        h = {'Host': host}
        if body is not None:
            h.update({'Content-Type': 'application/json', 'X-Switchboard': '1'})
        h.update(headers or {})
        r = urllib.request.Request(f'http://127.0.0.1:{self.port}{path}', data=None if body is None else json.dumps(body).encode(),
                                   headers=h, method='POST' if body is not None else 'GET')
        try:
            with urllib.request.urlopen(r, timeout=5) as resp:
                return resp.status, resp.read(), resp.headers
        except urllib.error.HTTPError as e:
            return e.code, e.read(), e.headers

    def test_the_server_writes_the_machine_list_at_start(self):
        d = json.loads((self.state / 'machines.json').read_text())
        self.assertEqual([m['name'] for m in d['machines']], ['desk', 'tower'])

    def test_api_term(self):
        st, b, _ = self.req('/api/term', {'machine': 'desk', 'sid': SID['a']}, host='100.64.0.5:8130')
        r = json.loads(b)
        self.assertEqual(st, 200)
        self.assertTrue(r['ws'].startswith('ws://100.64.0.5:8132/term?t='))
        st, b, _ = self.req('/api/term', {'machine': 'desk', 'sid': SID['b']})
        self.assertEqual((st, json.loads(b)['need']), (200, 'terminal'))
        st, b, _ = self.req('/api/term', {'machine': 'desk', 'sid': 'nope'})
        self.assertEqual(st, 404)
        st, b, _ = self.req('/api/term', {'machine': 'mars', 'sid': SID['a']})
        self.assertEqual(st, 400)

    def test_api_term_needs_the_header_like_every_post(self):
        st, b, _ = self.req('/api/term', {'machine': 'desk', 'sid': SID['a']}, headers={'X-Switchboard': ''})
        self.assertEqual(st, 403)
        st, b, _ = self.req('/api/term', {'machine': 'desk', 'sid': SID['a']}, host='evil.example')
        self.assertEqual(st, 403)

    def test_terminal_pages_and_xterm_are_gone(self):
        self.assertEqual(self.req('/t/desk/' + SID['a'])[0], 404)  # the terminal lives in the native apps now
        self.assertEqual(self.req('/static/vendor/xterm.js')[0], 404)  # the iPhone draws its own (SwiftTerm)
        self.assertEqual(self.req('/t/desk/../x')[0], 404)


if __name__ == '__main__':
    unittest.main()
