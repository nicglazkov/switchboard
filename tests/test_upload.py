"""Files dropped or pasted into a terminal: the agent's uploads folder (names, chunks, the week it keeps them) and the
server's POST /api/upload that sends them there."""
import base64, json, os, sys, threading, time, unittest, urllib.error, urllib.request
from pathlib import Path
from tempfile import mkdtemp

from tests.helpers import fresh_sb

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'server'))
import fleet, server  # noqa: E402

UID = 'ab' * 16


class AgentUploadTest(unittest.TestCase):
    def setUp(self):
        self.sb = fresh_sb(mkdtemp())

    def put(self, data, name='shot.png', step=None, uid=UID):
        step, r = step or len(data), None
        for off in range(0, len(data), step):
            r = self.sb.rpc_upload({'id': uid, 'name': name, 'offset': off, 'size': len(data), 'last': off + step >= len(data),
                                    'data': base64.b64encode(data[off:off + step]).decode()})
        return r

    def test_names_are_plain_and_never_a_path(self):
        n = self.sb.upload_name
        self.assertEqual(n('Screen Shot 2026-10-12 at 9.41.07 AM.png'), 'Screen-Shot-2026-10-12-at-9.41.07-AM.png')
        self.assertEqual(n('../../etc/passwd'), 'passwd')
        self.assertEqual(n('C:\\Users\\you\\..\\evil.PNG'), 'evil.png')
        self.assertEqual(n('...'), 'file')
        self.assertEqual(n(''), 'file')
        self.assertEqual(n(None), 'file')
        self.assertEqual(n('.bashrc'), 'bashrc')
        self.assertEqual(n('CON.txt'), 'CON-file.txt')
        self.assertEqual(n('résumé "final" (2).pdf'), 'resume-final-2.pdf')
        self.assertEqual(n('a' * 300 + '.jpeg'), 'a' * 64 + '.jpeg')

    def test_chunks_come_together_in_the_days_folder(self):
        data = os.urandom(5000)
        r = self.put(data, 'My Photo.HEIC', step=1024)
        p = Path(r['path'])
        self.assertEqual(p.read_bytes(), data)
        self.assertEqual(p.name, 'My-Photo.heic')
        self.assertEqual(p.parent.parent, self.sb.uploads_root())
        self.assertRegex(p.parent.name, r'^\d{4}-\d{2}-\d{2}$')
        self.assertEqual(r['size'], 5000)
        self.assertEqual([x.name for x in self.sb.uploads_root().iterdir() if x.name.startswith('.part-')], [])

    def test_the_same_name_twice_gets_a_number(self):
        a, b = self.put(b'one'), self.put(b'two', uid='cd' * 16)
        self.assertEqual(Path(a['path']).name, 'shot.png')
        self.assertEqual(Path(b['path']).name, 'shot-2.png')

    def test_bad_requests_are_refused(self):
        bad = [
            {'id': '../x', 'offset': 0, 'size': 3, 'data': 'YWJj'},
            {'id': UID, 'offset': 0, 'size': self.sb.UPLOAD_MAX + 1, 'data': 'YWJj'},
            {'id': UID, 'offset': 0, 'size': 0, 'data': ''},
            {'id': UID, 'offset': -1, 'size': 3, 'data': 'YWJj'},
            {'id': UID, 'offset': 0, 'size': 3, 'data': 'not base64!'},
            {'id': UID, 'offset': 0, 'size': 2, 'data': 'YWJj', 'last': True},  # bigger than it said
        ]
        for req in bad:
            with self.assertRaises(ValueError, msg=req):
                self.sb.rpc_upload(dict(req, name='x.png'))

    def test_a_chunk_out_of_place_stops_the_upload(self):
        self.sb.rpc_upload({'id': UID, 'name': 'a.png', 'offset': 0, 'size': 6, 'data': base64.b64encode(b'abc').decode()})
        with self.assertRaises(ValueError):
            self.sb.rpc_upload({'id': UID, 'name': 'a.png', 'offset': 4, 'size': 6, 'data': base64.b64encode(b'def').decode(), 'last': True})

    def test_a_week_later_they_are_gone(self):
        root = self.sb.uploads_root()
        old, new, keep = root / '2026-01-01', root / '2026-10-11', root / 'notes'
        for d in (old, new, keep):
            d.mkdir(parents=True)
            (d / 'f.png').write_bytes(b'x')
        stale = root / f'.part-{UID}'
        stale.write_bytes(b'x')
        t = time.time()
        os.utime(old, (t - 8 * 86400, t - 8 * 86400))
        os.utime(keep, (t - 30 * 86400, t - 30 * 86400))
        os.utime(stale, (t - 2 * 86400, t - 2 * 86400))
        self.assertEqual(self.sb.prune_uploads(t), 2)
        self.assertFalse(old.exists())
        self.assertFalse(stale.exists())
        self.assertTrue(new.exists())
        self.assertTrue(keep.exists())  # not one of its day folders: never touched

    def test_the_agent_says_it_can(self):
        self.assertIn('upload', self.sb.AGENT_FEATURES)


class ServerUploadTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.sb = fresh_sb(mkdtemp())
        cls.calls, cls.off, cls.feats = [], set(), {'one': ['upload'], 'old': [], 'link': ['upload']}

        def transport(target, req, timeout):
            name = target.split('@')[-1].split(':')[-1]
            if name in cls.off:
                raise OSError('ssh: timed out')
            if req['verb'] == 'list':
                return {'ok': True, 'sessions': [], 'features': cls.feats[name]}
            cls.calls.append((name, req['offset'], len(base64.b64decode(req['data']))))
            try:
                return cls.sb.rpc_upload(req)
            except ValueError as e:
                return {'ok': False, 'error': str(e)}
        machines = [{'name': 'one', 'label': 'Studio', 'target': 'me@one'}, {'name': 'old', 'label': 'Old', 'target': 'me@old'},
                    {'name': 'link', 'label': 'Linked', 'target': 'node:link'}]
        cls.app = server.App(fleet.Fleet(machines, transport), state_dir=Path(mkdtemp()),
                             web_dir=Path(__file__).resolve().parents[1] / 'server' / 'web', guard_delay=0.05)
        cls.app.fleet.poll()
        cls.httpd = server.make_server('127.0.0.1', 0, cls.app)
        cls.port = cls.httpd.server_address[1]
        threading.Thread(target=cls.httpd.serve_forever, daemon=True).start()

    @classmethod
    def tearDownClass(cls):
        cls.httpd.shutdown()

    def post(self, query, data, headers=None):
        h = {'Host': f'127.0.0.1:{self.port}', 'Content-Type': 'application/octet-stream', 'X-Switchboard': '1'}
        h.update(headers or {})
        r = urllib.request.Request(f'http://127.0.0.1:{self.port}/api/upload?{query}', data=data, headers=h, method='POST')
        try:
            with urllib.request.urlopen(r, timeout=10) as resp:
                return resp.status, json.loads(resp.read())
        except urllib.error.HTTPError as e:
            return e.code, json.loads(e.read())

    def test_a_file_lands_on_the_machine_and_its_path_comes_back(self):
        data = os.urandom(3000)
        st, j = self.post('machine=one&name=Screen%20Shot.png', data)
        self.assertEqual(st, 200, j)
        self.assertEqual(Path(j['path']).read_bytes(), data)
        self.assertEqual(j['name'], 'Screen-Shot.png')
        self.assertEqual((j['machine'], j['size']), ('one', 3000))

    def test_a_linked_machine_gets_it_in_small_chunks(self):
        self.calls.clear()
        data = os.urandom(server.UPLOAD_CHUNK_NODE * 2 + 10)
        st, j = self.post('machine=link&name=big.bin', data)
        self.assertEqual(st, 200, j)
        self.assertEqual([c[1:] for c in self.calls], [(0, server.UPLOAD_CHUNK_NODE), (server.UPLOAD_CHUNK_NODE, server.UPLOAD_CHUNK_NODE),
                                                       (2 * server.UPLOAD_CHUNK_NODE, 10)])
        self.assertEqual(Path(j['path']).read_bytes(), data)

    def test_what_cannot_go_says_why(self):
        st, j = self.post('machine=old&name=a.png', b'abc')
        self.assertEqual(st, 409)
        self.assertIn('Update Switchboard on Old', j['error'])
        self.off.add('one')
        try:
            self.app.fleet.poll()
            st, j = self.post('machine=one&name=a.png', b'abc')
            self.assertEqual(st, 409)
            self.assertIn('Studio is offline', j['error'])
        finally:
            self.off.discard('one')
            self.app.fleet.poll()
        self.assertEqual(self.post('machine=one&name=a.png', b'')[0], 400)
        self.assertEqual(self.post('machine=nope&name=a.png', b'abc')[0], 400)

    def test_too_large_and_cross_site_are_refused(self):
        big = server.UPLOAD_MAX
        server.UPLOAD_MAX = 10
        try:
            st, j = self.post('machine=one&name=a.png', b'x' * 11)
        finally:
            server.UPLOAD_MAX = big
        self.assertEqual(st, 413)
        self.assertIn('can be attached', j['error'])
        self.assertEqual(self.post('machine=one&name=a.png', b'abc', {'X-Switchboard': ''})[0], 403)
        self.assertEqual(self.post('machine=one&name=a.png', b'abc', {'Content-Type': 'text/plain'})[0], 403)
        self.assertEqual(self.post('machine=one&name=a.png', b'abc', {'Host': 'evil.example:80'})[0], 403)


if __name__ == '__main__':
    unittest.main()
