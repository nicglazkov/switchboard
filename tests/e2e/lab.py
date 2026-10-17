"""End-to-end tests against the lab fleet (scripts/lab/fakefleet.py): the real server.App and its HTTP routes, with
every machine faked, so nothing reaches a real Claude session.

Each test class starts its own fake fleet on a free port, with its own control folder, and talks to it only over HTTP
the way the apps do (X-Switchboard header, JSON bodies, /api/stream for live updates). They are slow (jobs take a
few seconds, like real launches), so they run only with SB_E2E=1:

    scripts/lab/e2e.sh                       # builds the hub and node for the pairing test, then runs them all
    SB_E2E=1 python3 -m unittest discover -s tests/e2e -t .
"""
import http.client, json, os, shutil, signal, socket, subprocess, sys, tempfile, threading, time, unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
FAKEFLEET = ROOT / 'scripts' / 'lab' / 'fakefleet.py'
FIXTURE = ROOT / 'native' / 'scripts' / 'fixtures' / 'fixture.json'
E2E = os.environ.get('SB_E2E') == '1'
MACHINES = ('studio', 'laptop', 'tower')
LIVE_MAX = 3.0  # a change reaches every open client in under this many seconds


def lab_test(cls):
    return unittest.skipUnless(E2E, 'end-to-end lab tests: run scripts/lab/e2e.sh (or set SB_E2E=1)')(cls)


def free_port():
    with socket.socket() as s:
        s.bind(('127.0.0.1', 0))
        return s.getsockname()[1]


class Lab:
    """One fake fleet: start(), then get/post/upload, wait, job, flag and rpcs."""

    def __init__(self, flags=('fast',), env=None, poll=None):
        self.tmp = Path(tempfile.mkdtemp(prefix='sb-e2e-'))
        self.ctrl = self.tmp / 'ctrl'
        self.ctrl.mkdir()
        for f in flags:
            self.flag(f)
        self.port = free_port()
        self.env = dict(os.environ, **(env or {}))
        self.env.pop('SB_HOME', None)
        if poll is not None:
            self.env['LAB_POLL'] = str(poll)
        self.proc = None

    # ---- the process
    def start(self, timeout=30, caps=True):
        """caps: wait for the first probe of what each machine can do, so Auto has something to pick from."""
        self.caps = caps
        self.log = open(self.tmp / 'fakefleet.log', 'w')
        self.proc = subprocess.Popen([sys.executable, str(FAKEFLEET), str(ROOT), str(FIXTURE), str(self.port), '127.0.0.1', str(self.ctrl)],
                                     stdout=self.log, stderr=subprocess.STDOUT, env=self.env, start_new_session=True)
        end = time.time() + timeout
        while time.time() < end:
            if self.proc.poll() is not None:
                raise RuntimeError('the fake fleet exited:\n' + (self.tmp / 'fakefleet.log').read_text())
            try:
                code, d = self.get('/api/sessions')
                if code == 200 and all(m['online'] for m in d['machines'] if m['name'] in MACHINES) and \
                        (not self.caps or all(self.get('/api/caps')[1].get(n) for n in MACHINES)):
                    return self
            except OSError:
                pass
            time.sleep(0.2)
        raise RuntimeError('the fake fleet did not come up:\n' + (self.tmp / 'fakefleet.log').read_text())

    def stop(self):
        if self.proc and self.proc.poll() is None:
            os.killpg(self.proc.pid, signal.SIGTERM)
            try:
                self.proc.wait(5)
            except subprocess.TimeoutExpired:
                os.killpg(self.proc.pid, signal.SIGKILL)
        if getattr(self, 'log', None):
            self.log.close()
        if os.environ.get('SB_E2E_KEEP'):
            print(f'kept {self.tmp}')
        else:
            shutil.rmtree(self.tmp, ignore_errors=True)

    # ---- HTTP, as the apps send it
    def request(self, method, path, body=None, headers=None, timeout=60):
        c = http.client.HTTPConnection('127.0.0.1', self.port, timeout=timeout)
        h = {'Host': f'127.0.0.1:{self.port}'}
        h.update(headers or {})
        try:
            c.request(method, path, body=body, headers=h)
            r = c.getresponse()
            raw = r.read()
        finally:
            c.close()
        try:
            return r.status, json.loads(raw or b'{}')
        except ValueError:
            return r.status, raw

    def get(self, path, **kw):
        return self.request('GET', path, **kw)

    def post(self, path, body, ua='SwitchboardDesktop/1.0', headers=None):
        h = {'X-Switchboard': '1', 'Content-Type': 'application/json', 'User-Agent': ua}
        h.update(headers or {})
        return self.request('POST', path, json.dumps(body).encode(), h)

    def upload(self, machine, name, data):
        from urllib.parse import urlencode
        q = urlencode({'machine': machine, 'name': name})
        return self.request('POST', f'/api/upload?{q}', data, {'X-Switchboard': '1', 'Content-Type': 'application/octet-stream'})

    # ---- what the fleet shows
    def sessions(self):
        code, d = self.get('/api/sessions')
        assert code == 200, d
        return d

    def machine(self, name, d=None):
        return next(m for m in (d or self.sessions())['machines'] if m['name'] == name)

    def session(self, machine, sid, replaced=False):
        m = self.machine(machine)
        rows = m.get('replaced') or [] if replaced else m['sessions']
        return next((s for s in rows if s.get('sid') == sid), None)

    def pick(self, machine, pred):
        """A live chat on that machine that pred likes."""
        return next(s for s in self.machine(machine)['sessions'] if pred(s))

    # ---- control files (scripts/lab/fakefleet.py)
    def flag(self, name, on=True):
        p = self.ctrl / name
        if on:
            p.write_text('')
        else:
            p.unlink(missing_ok=True)

    def rpcs(self, verb=None, machine=None):
        try:
            rows = [json.loads(l) for l in (self.ctrl / 'rpc.log').read_text().splitlines() if l.strip()]
        except OSError:
            return []
        return [r for r in rows if (verb is None or r['req'].get('verb') == verb)
                and (machine is None or r['target'] in (f'fake@{machine}', f'node:{machine}'))]

    def pushes(self):
        try:
            return [json.loads(l) for l in (self.ctrl / 'pushes.log').read_text().splitlines() if l.strip()]
        except OSError:
            return []

    # ---- waiting
    @staticmethod
    def wait(pred, timeout=10, every=0.1, what='the condition'):
        end = time.time() + timeout
        while True:
            v = pred()
            if v:
                return v
            if time.time() > end:
                raise AssertionError(f'timed out after {timeout} s waiting for {what}')
            time.sleep(every)

    def job(self, jid, timeout=60, watch=True):
        """Follows a server job (GET /api/job/server/<id>, as the apps do) to its end; returns the last answer with
        `seen`, the (state, step) pairs on the way. watch=False only reads jobs.json, so the server pushes."""
        seen, end = [], time.time() + timeout
        while time.time() < end:
            if watch:
                code, j = self.get(f'/api/job/server/{jid}')
                assert code == 200, j
            else:
                j = json.loads((self.ctrl / 'state' / 'jobs.json').read_text()).get(jid) or {}
            st = (j.get('state'), j.get('step'))
            if not seen or seen[-1] != st:
                seen.append(st)
            if j.get('state') in ('done', 'error'):
                return dict(j, seen=seen)
            time.sleep(0.25)
        raise AssertionError(f'job {jid} did not finish in {timeout} s: {seen}')


class Stream:
    """A client on /api/stream: every event with the time it arrived."""

    def __init__(self, lab):
        self.lab, self.events, self.cv = lab, [], threading.Condition()
        self.conn = http.client.HTTPConnection('127.0.0.1', lab.port, timeout=700)
        self.conn.request('GET', '/api/stream', headers={'Host': f'127.0.0.1:{lab.port}'})
        self.sock = self.conn.sock  # the response takes it over; close() needs it to hang up
        self.resp = self.conn.getresponse()
        assert self.resp.status == 200, self.resp.status
        self.t = threading.Thread(target=self._read, daemon=True)
        self.t.start()
        self.wait_event('hello', 0, 5)

    def _read(self):
        ev = None
        try:
            for raw in self.resp:
                line = raw.decode().rstrip('\n')
                if line.startswith('event: '):
                    ev = line[7:]
                elif line.startswith('data: ') and ev:
                    with self.cv:
                        self.events.append((time.time(), ev, json.loads(line[6:]).get('v')))
                        self.cv.notify_all()
                    ev = None
        except (OSError, ValueError):
            pass

    def wait_event(self, kind, after, timeout):
        """The first `kind` event at or after time `after`: (t, v)."""
        end = time.time() + timeout
        with self.cv:
            while True:
                for t, ev, v in self.events:
                    if ev == kind and t >= after:
                        return t, v
                left = end - time.time()
                if left <= 0:
                    raise AssertionError(f'no {kind} event within {timeout} s')
                self.cv.wait(left)

    def close(self):
        try:
            self.sock.shutdown(socket.SHUT_RDWR)
        except OSError:
            pass
        self.t.join(2)
        self.resp.close()
        self.sock.close()
        self.conn.close()


class LabCase(unittest.TestCase):
    """A test class with one fake fleet for all its tests."""
    FLAGS = ('fast',)
    ENV = None
    POLL = None

    @classmethod
    def setUpClass(cls):
        cls.lab = Lab(cls.FLAGS, cls.ENV, cls.POLL).start()

    @classmethod
    def tearDownClass(cls):
        cls.lab.stop()
