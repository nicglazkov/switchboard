"""Live updates, end to end: every app keeps GET /api/stream open, and on each "change" loads /api/sessions again.
These measure, as such a client sees it, the time from something happening to the new state being on its screen,
and require it to be under 3 seconds. The lab polls its machines every 20 s here, like the real server, so a timed
poll cannot hide a change that was never announced; LiveWithoutPolls turns the timed poll off altogether."""
import sys, time, unittest

from tests.e2e.lab import LIVE_MAX, LabCase, Stream, lab_test


TIMES = []


def tearDownModule():
    if TIMES:
        print('\nlive update times (s): ' + ', '.join(f'{w} {t:.2f}' for w, t in TIMES), file=sys.stderr)


class Follower:
    """A client: hears changes on /api/stream and loads /api/sessions after each one."""

    def __init__(self, lab):
        self.lab, self.stream = lab, Stream(lab)

    def until(self, pred, t0, timeout=LIVE_MAX + 7):
        """Seconds from t0 until a load after a change event shows pred true. Fails if it never does."""
        with self.stream.cv:  # only what was announced after t0 counts
            seen = next((i for i, e in enumerate(self.stream.events) if e[0] >= t0), len(self.stream.events))
        end = time.time() + timeout
        while time.time() < end:
            with self.stream.cv:
                self.stream.cv.wait_for(lambda: len(self.stream.events) > seen, max(0, end - time.time()))
                new = self.stream.events[seen:]
                seen = len(self.stream.events)
            if not any(ev == 'change' for _, ev, _ in new):
                continue
            snap = self.lab.sessions()
            if pred(snap):
                return time.time() - t0
        raise AssertionError(f'the change never reached the client in {timeout} s')

    def close(self):
        self.stream.close()


def row(snap, machine, sid, replaced=False):
    m = next(m for m in snap['machines'] if m['name'] == machine)
    return next((s for s in (m.get('replaced') or [] if replaced else m['sessions']) if s.get('sid') == sid), None)


class LiveCase(LabCase):
    POLL = 20

    def setUp(self):
        self.f = Follower(self.lab)
        self.addCleanup(self.f.close)

    def assertFast(self, took, what):
        TIMES.append((what, took))
        self.assertLess(took, LIVE_MAX, f'{what} took {took:.2f} s to reach the client')


@lab_test
class Live(LiveCase):
    def ping(self, machine, sid, why, **kw):
        code, r = self.lab.post('/api/event', dict({'event': 'ping', 'machine': machine, 'session_id': sid, 'why': why}, **kw))
        self.assertEqual(code, 200, r)

    def test_a_prompt_and_a_finished_reply(self):
        s = self.lab.pick('studio', lambda s: s.get('kind') == 'interactive' and s.get('status') == 'idle')
        t0 = time.time()
        self.ping('studio', s['sid'], 'prompt')  # the machine's hook: a prompt went in
        self.assertFast(self.f.until(lambda d: row(d, 'studio', s['sid'])['status'] == 'busy', t0), 'Working')
        t0 = time.time()
        self.ping('studio', s['sid'], 'stop')  # Claude finished
        self.assertFast(self.f.until(lambda d: row(d, 'studio', s['sid'])['status'] == 'idle', t0), 'Idle')

    def test_context_from_the_status_line(self):
        s = self.lab.pick('laptop', lambda s: s.get('kind') == 'interactive' and s.get('status') == 'idle')
        t0 = time.time()
        self.ping('laptop', s['sid'], 'status', ctx=123_456)
        self.assertFast(self.f.until(lambda d: row(d, 'laptop', s['sid'])['ctx'] == 123_456, t0), 'the context size')

    def test_a_rename(self):
        s = self.lab.pick('tower', lambda s: s.get('kind') is None)
        t0 = time.time()
        code, r = self.lab.post('/api/action', {'machine': 'tower', 'verb': 'rename', 'sid': s['sid'], 'name': 'Renamed for the live test'})
        self.assertEqual(code, 200, r)
        self.assertFast(self.f.until(lambda d: row(d, 'tower', s['sid'])['title'] == 'Renamed for the live test', t0), 'A rename')

    def test_a_new_session_shows_once_it_has_started(self):
        code, r = self.lab.post('/api/new', {'machine': 'laptop', 'prompt': 'Live test chat', 'folder': ''})
        j = self.lab.job(r['job'])
        t0 = time.time()
        self.assertFast(self.f.until(lambda d: row(d, 'laptop', j['sid']) is not None, t0), 'A new chat')

    def test_start_fresh_moves_the_old_chat(self):
        s = self.lab.pick('laptop', lambda s: s.get('kind') is None)
        code, r = self.lab.post('/api/action', {'machine': 'laptop', 'verb': 'fresh', 'sid': s['sid']})
        j = self.lab.job(r['job'])
        t0 = time.time()
        took = self.f.until(lambda d: row(d, 'laptop', j['sid']) and row(d, 'laptop', s['sid'], True) and not row(d, 'laptop', s['sid']), t0)
        self.assertFast(took, 'Start fresh')

    def test_continue_updates_both_machines(self):
        s = self.lab.pick('studio', lambda s: s.get('kind') is None)
        code, r = self.lab.post('/api/continue', {'machine': 'studio', 'sid': s['sid'], 'to': 'laptop'})
        j = self.lab.job(r['job'], timeout=90)
        t0 = time.time()
        took = self.f.until(lambda d: row(d, 'laptop', j['sid']) and row(d, 'studio', s['sid'], True), t0)
        self.assertFast(took, 'Continue')

    def test_appearance_reaches_every_device(self):
        t0 = time.time()
        code, r = self.lab.post('/api/prefs', {'appearance': {'theme': 'dark'}})
        self.assertEqual(code, 200, r)
        self.assertFast(self.f.until(lambda d: d['prefs']['appearance']['theme'] == 'dark', t0), 'Dark mode')
        self.lab.post('/api/prefs', {'appearance': {'theme': 'system'}})

    def test_many_clients_hear_one_change(self):
        others = [Stream(self.lab) for _ in range(8)]
        try:
            s = self.lab.pick('studio', lambda s: s.get('kind') == 'interactive' and s.get('status') == 'idle')
            t0 = time.time()
            self.lab.post('/api/event', {'event': 'ping', 'machine': 'studio', 'session_id': s['sid'], 'why': 'prompt'})
            for o in others:
                t, _ = o.wait_event('change', t0, LIVE_MAX)
                self.assertFast(t - t0, 'One of eight clients hearing it')
        finally:
            for o in others:
                o.close()
            self.lab.post('/api/event', {'event': 'ping', 'machine': 'studio', 'session_id': s['sid'], 'why': 'stop'})

    def test_the_stream_version_matches_the_list(self):
        s = Stream(self.lab)
        try:
            t0 = time.time()
            self.lab.post('/api/prefs', {'appearance': {'theme': 'light'}})
            _, v = s.wait_event('change', t0, LIVE_MAX)
            self.assertGreaterEqual(self.lab.sessions()['v'], v)
        finally:
            s.close()
            self.lab.post('/api/prefs', {'appearance': {'theme': 'system'}})


@lab_test
class LiveWithoutPolls(LiveCase):
    """Changes no hook announces: only a timed poll would find them, and here there is none."""
    POLL = 3600

    @classmethod
    def setUpClass(cls):
        from tests.e2e.lab import Lab
        cls.lab = Lab(poll=cls.POLL).start(caps=False)

    def tearDown(self):
        self.lab.flag('off-tower', False)

    @unittest.expectedFailure  # report bug 3: a machine that drops off shows online until the next poll (up to 20 s)
    def test_a_machine_going_offline(self):
        t0 = time.time()
        self.lab.flag('off-tower')
        took = self.f.until(lambda d: next(m for m in d['machines'] if m['name'] == 'tower')['online'] is False, t0, timeout=8)
        self.assertFast(took, 'Offline')

    def test_a_job_on_a_machine_still_refreshes_it(self):
        """A job's own end asks its machine at once, with or without timed polls."""
        code, r = self.lab.post('/api/new', {'machine': 'tower', 'prompt': 'No poll chat', 'folder': ''})
        j = self.lab.job(r['job'])
        t0 = time.time()
        self.assertFast(self.f.until(lambda d: row(d, 'tower', j['sid']) is not None, t0), 'A new chat with no timed poll')


@lab_test
class StreamLimits(LabCase):
    def test_at_most_64_live_pages_and_slots_come_back(self):
        streams = [Stream(self.lab) for _ in range(64)]
        try:
            code, r = self.lab.get('/api/stream', timeout=5)
            self.assertEqual((code, r['error']), (503, 'too many live pages'))
        finally:
            for s in streams:
                s.close()
        # a closed page frees its slot when the server next writes to it (a beat every 15 s at most)
        t0 = time.time()
        self.lab.post('/api/prefs', {'appearance': {'theme': 'system'}})  # a change makes it write now
        s = self.lab.wait(lambda: self._try_stream(), 20, 0.5, 'a free slot')
        s.close()
        self.assertLess(time.time() - t0, 20)

    def _try_stream(self):
        try:
            return Stream(self.lab)
        except AssertionError:
            return None
