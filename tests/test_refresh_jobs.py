"""Three small things at the server's edges: many Refresh taps share a poll, the jobs kept over a restart are put
right before anything watches them, and a POST that does not say how long it is gets a plain answer."""
import json, socket, sys, threading, time, unittest
from pathlib import Path
from tempfile import mkdtemp
sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'server'))
import fleet, server  # noqa: E402
from tests import test_server_jobs  # noqa: E402
from tests.test_server import MACH  # noqa: E402

SID = test_server_jobs.SID


class SlowFleet(fleet.Fleet):
    polls, gate = 0, None

    def poll(self, timeout=15):
        self.polls += 1
        if self.gate:
            self.gate.wait(5)
        return super().poll(timeout)


class RefreshTest(unittest.TestCase):
    def make(self):
        f = SlowFleet(MACH, test_server_jobs.Machine())
        self.state = Path(mkdtemp())
        app = server.App(f, state_dir=self.state, web_dir=self.state)
        app._refresh_soon = lambda *a, **k: None
        return app

    def test_one_refresh_is_one_poll(self):
        app = self.make()
        self.assertIn('machines', app.post('/api/refresh', {}))
        self.assertIn('machines', app.post('/api/refresh', {}))
        self.assertEqual(app.fleet.polls, 2, 'one after the other: each is asked anew')

    def test_many_at_once_share_a_poll(self):
        app = self.make()
        app.fleet.gate = threading.Event()
        out = []
        ts = [threading.Thread(target=lambda: out.append(app.post('/api/refresh', {}))) for _ in range(8)]
        ts[0].start()
        for _ in range(500):
            if app.fleet.polls:
                break
            time.sleep(0.01)
        for t in ts[1:]:
            t.start()
        time.sleep(0.2)  # the seven are waiting on the first one's poll
        app.fleet.gate.set()
        for t in ts:
            t.join(10)
        self.assertEqual((len(out), all('machines' in r for r in out)), (8, True))
        self.assertEqual(app.fleet.polls, 2, 'the first, and one more that began after the others asked')

    def test_a_poll_that_fails_does_not_leave_the_next_one_waiting(self):
        app = self.make()
        boom = [True]
        real = app.fleet.poll

        def poll(timeout=15):
            if boom.pop() if boom else False:
                raise OSError('no')
            return real(timeout)
        app.fleet.poll = poll
        with self.assertRaises(OSError):
            app.post('/api/refresh', {})
        self.assertIn('machines', app.post('/api/refresh', {}))


class RecoverOrderTest(unittest.TestCase):
    def test_the_jobs_are_put_right_and_saved_before_anything_watches_them(self):
        st, now, seen = Path(mkdtemp()), time.time(), []
        job = lambda **kw: dict({'kind': 'fresh', 'machine': 'desk', 'plan': {'req': {'verb': 'fresh', 'sid': SID}}, 'title': 'Lights',
                                 'state': 'running', 'step': 'running', 'created': now - 30, 'watched': 0}, **kw)
        (st / 'jobs.json').write_text(json.dumps({'s-aaaaaaaaaaaa': job(inner='0123456789ab'), 's-bbbbbbbbbbbb': job(),
                                                  's-cccccccccccc': job(inner='0123456789ac')}))

        class App(server.App):
            def _rewatch(self, jid):  # as its thread would find things the moment it starts
                seen.append((jid, self.jobs_lock.acquire(False), json.loads((st / 'jobs.json').read_text())['s-bbbbbbbbbbbb']['state']))
                self.jobs_lock.release()

        class Now(threading.Thread):
            def start(self):
                self._target(*self._args) if self._target.__name__ == '_rewatch' else super().start()
        real, server.threading.Thread = server.threading.Thread, Now
        try:
            App(fleet.Fleet(MACH, test_server_jobs.Machine()), state_dir=st, web_dir=st)
        finally:
            server.threading.Thread = real
        self.assertEqual(seen, [('s-aaaaaaaaaaaa', True, 'error'), ('s-cccccccccccc', True, 'error')])


class NoLengthTest(unittest.TestCase):
    def test_a_post_sent_in_chunks_is_refused_and_the_connection_ends(self):
        st = Path(mkdtemp())
        app = server.App(fleet.Fleet(MACH, test_server_jobs.Machine()), state_dir=st, web_dir=st)
        httpd = server.Server(('127.0.0.1', 0), server.make_handler(app))
        threading.Thread(target=httpd.serve_forever, daemon=True).start()
        self.addCleanup(httpd.shutdown)
        self.addCleanup(httpd.server_close)
        s = socket.create_connection(('127.0.0.1', httpd.server_address[1]), timeout=5)
        self.addCleanup(s.close)
        body = b'{"summary_model": "sonnet"}'
        s.sendall(b'POST /api/prefs HTTP/1.1\r\nHost: localhost\r\nX-Switchboard: 1\r\nContent-Type: application/json\r\n'
                  b'Transfer-Encoding: chunked\r\n\r\n' + b'%x\r\n%s\r\n0\r\n\r\n' % (len(body), body)
                  + b'GET /api/prefs HTTP/1.1\r\nHost: localhost\r\n\r\n')
        got = b''
        while True:
            try:
                more = s.recv(65536)
            except OSError:
                break
            if not more:
                break
            got += more
        head = got.split(b'\r\n\r\n')[0].decode()
        self.assertTrue(head.startswith('HTTP/1.1 411') or head.startswith('HTTP/1.0 411'), head)
        self.assertIn('Connection: close', head)
        self.assertEqual(got.count(b'HTTP/1.'), 1, 'what it sent is not read as a second request')
        self.assertNotEqual(app.prefs()['summary_model'], 'sonnet')


if __name__ == '__main__':
    unittest.main()
