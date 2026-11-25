"""Who may talk to the server: the address a request really came from, not the Host it claims.

A client on the open internet can send `Host: localhost` and `X-Switchboard: 1`. Its own address it cannot choose."""
import contextlib, io, json, sys, threading, unittest, urllib.error, urllib.request
from pathlib import Path
from tempfile import mkdtemp

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'server'))
import fleet, server  # noqa: E402

HOME = ('127.0.0.1', '127.8.9.1', '::1', '10.0.0.7', '169.254.3.4', '100.64.0.5', 'fd7a:115c:a1e0::5', 'fc00::1',
        'fe80::1', 'fe80::1%en0', '::ffff:10.0.0.7', '::ffff:100.64.0.5', '::ffff:127.0.0.1',
        '172.16.0.1', '172.31.255.254', '192.168.1.20', '100.127.255.254', '::ffff:192.168.1.20')  # leakscan: allow
AWAY = ('203.0.113.9', '8.8.8.8', '172.32.0.1', '100.128.0.1', '100.63.255.255', '2001:db8::1', '::ffff:203.0.113.9',
        '0.0.0.0', '::', '', 'studio', None)


def write(p, d):
    p.parent.mkdir(parents=True, exist_ok=True)
    p.write_text(json.dumps(d))


def config(**kw):
    d = Path(mkdtemp())
    write(d / 'cfg' / 'server.json', dict({'bind': ['0.0.0.0'], 'hosts': ['studio'], 'origins': []}, **kw))
    return server.ServerConfig(d / 'cfg', d / 'state')


class PeerRuleTest(unittest.TestCase):
    def test_your_own_networks_are_let_in_and_the_rest_is_not(self):
        for a in HOME:
            self.assertTrue(server.peer_ok(a), a)
        for a in AWAY:
            self.assertFalse(server.peer_ok(a), a)

    def test_a_server_json_without_the_key_keeps_the_defaults(self):
        c = config()
        self.assertTrue(all(c.peer_ok(a) for a in HOME))
        self.assertFalse(any(c.peer_ok(a) for a in AWAY))

    def test_more_networks_can_be_named(self):
        c = config(peers=['203.0.113.0/24', '2001:db8::/32', '198.51.100.7', 'not a network', 5])
        for a in ('203.0.113.9', '::ffff:203.0.113.9', '2001:db8::1', '198.51.100.7', '10.0.0.7', '127.0.0.1'):
            self.assertTrue(c.peer_ok(a), a)
        for a in ('8.8.8.8', '198.51.100.8', ''):
            self.assertFalse(c.peer_ok(a), a)

    def test_any_address_only_when_asked_for_by_name(self):
        c = config(peers=['any'])
        for a in ('8.8.8.8', '2001:db8::1', '10.0.0.7'):
            self.assertTrue(c.peer_ok(a), a)
        self.assertFalse(config(peers=[]).peer_ok('8.8.8.8'))
        self.assertFalse(config(peers='any').peer_ok('8.8.8.8'), 'a list, like the other keys')


class Quiet:
    def __call__(self, target, req, timeout):
        return {'ok': True, 'sessions': [], 'replaced': []}


class PeerHttpTest(unittest.TestCase):
    """Every way in, with a request that looks right in all but where it came from."""

    def serve(self, peer, server_cfg=None):
        state = Path(mkdtemp())
        app = server.App(fleet.Fleet([{'name': 'studio', 'label': 'Studio', 'target': 'me@x', 'os': 'mac'}], Quiet()),
                         state_dir=state, web_dir=Path(server.__file__).resolve().parent / 'web')
        app.server_cfg = server_cfg

        class From(server.make_handler(app)):
            def setup(self):
                super().setup()
                self.client_address = (peer, 50000)
        httpd = server.Server(('127.0.0.1', 0), From)
        threading.Thread(target=httpd.serve_forever, daemon=True).start()
        self.addCleanup(httpd.shutdown)
        self.app, self.port = app, httpd.server_address[1]

    def req(self, path, body=None, ctype='application/json'):
        h = {'Host': 'localhost'}
        data = None
        if body is not None:
            h.update({'Content-Type': ctype, 'X-Switchboard': '1'})
            data = body if isinstance(body, bytes) else json.dumps(body).encode()
        r = urllib.request.Request(f'http://127.0.0.1:{self.port}{path}', data=data, headers=h)
        try:
            with urllib.request.urlopen(r, timeout=5) as resp:
                return resp.status, resp.read(200)
        except urllib.error.HTTPError as e:
            return e.code, e.read()

    WAYS = (('/api/sessions', None), ('/', None), ('/icon.svg', None), ('/api/stream', None),
            ('/api/app/files/Switchboard-0.8.22.dmg', None), ('/api/prefs', {'summary_model': 'sonnet'}),
            ('/api/upload?machine=studio&name=a.txt', b'hello'))

    def test_an_address_from_elsewhere_is_refused_on_every_way_in(self):
        self.serve('203.0.113.9')
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            for path, body in self.WAYS:
                st, b = self.req(path, body, 'application/octet-stream' if isinstance(body, bytes) else 'application/json')
                self.assertEqual(st, 403, path)
                self.assertEqual(json.loads(b), {'ok': False, 'error': 'forbidden address'}, path)
        self.assertEqual(self.app.prefs()['summary_model'], 'opus', 'nothing was changed')
        lines = [ln for ln in out.getvalue().splitlines() if 'refused' in ln]
        self.assertEqual(len(lines), 1, 'one line, not one per request')
        self.assertIn('203.0.113.9', lines[0])

    def test_the_log_says_how_many_were_refused_once_a_minute(self):
        self.serve('203.0.113.9')
        now = [1000.0]
        self.app.clock = lambda: now[0]
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            for _ in range(3):
                self.req('/api/sessions')
            now[0] += 61
            self.req('/api/sessions')
        lines = [ln for ln in out.getvalue().splitlines() if 'refused' in ln]
        self.assertEqual(len(lines), 2)
        self.assertIn('2 more', lines[1])

    def test_a_home_address_is_served_as_before(self):
        for peer in ('10.0.0.7', '100.64.0.5', '::ffff:10.0.0.7'):
            self.serve(peer)
            self.assertEqual(self.req('/api/sessions')[0], 200, peer)
            self.assertEqual(self.req('/api/prefs', {'summary_model': 'opus'})[0], 200, peer)

    def test_a_named_network_and_any_are_served(self):
        for peers in (['203.0.113.0/24'], ['any']):
            self.serve('203.0.113.9', config(peers=peers))
            self.assertEqual(self.req('/api/sessions')[0], 200, peers)

    def test_the_host_is_still_checked(self):
        self.serve('10.0.0.7', config())
        r = urllib.request.Request(f'http://127.0.0.1:{self.port}/api/sessions', headers={'Host': 'evil.example'})
        with self.assertRaises(urllib.error.HTTPError) as e:
            urllib.request.urlopen(r, timeout=5)
        self.assertEqual((e.exception.code, json.loads(e.exception.read())['error']), (403, 'forbidden host'))


if __name__ == '__main__':
    unittest.main()
