import json, sys, threading, time, unittest, urllib.request, urllib.error
from pathlib import Path
from tempfile import mkdtemp
sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'server'))
import fleet, guard, apns, server  # noqa: E402

MACH = [{'name': 'desk', 'label': 'Desk', 'target': 'me@x', 'os': 'mac'},
        {'name': 'tower', 'label': 'Tower', 'target': 'me@y', 'os': 'windows'}]


def sess(sid, ctx=200_000, cold=True, mtime=100.0, **kw):
    d = {'sid': sid, 'title': 't' + sid[:2], 'ctx': ctx, 'cold': cold, 'mtime': mtime, 'kind': None, 'url': None,
         'folder': 'p', 'cwd': '/p', 'handoff': False, 'model': 'claude-opus-5-5'}
    d.update(kw)
    return d


class FakeTransport:
    def __init__(self):
        self.down, self.calls = set(), []
        self.replies = {'desk': {'ok': True, 'machine': 'desk', 'sessions': [sess('aa' * 4 + '-1111-2222-3333-444444444444', mtime=200)],
                                 'limits': {'five_hour': 10, 'seven_day': 40, 'updated': 50}},
                        'tower': {'ok': True, 'machine': 'tower', 'sessions': [sess('bb' * 4 + '-1111-2222-3333-444444444444', ctx=20_000, cold=False, mtime=300)],
                                    'limits': {'five_hour': 12, 'seven_day': 41, 'updated': 90}}}

    def __call__(self, target, req, timeout):
        name = 'desk' if target == 'me@x' else 'tower'
        self.calls.append((name, req))
        if name in self.down:
            raise OSError('ssh: connect timed out')
        if req['verb'] == 'list':
            return self.replies[name]
        return {'ok': True, 'echo': req}


class FleetTest(unittest.TestCase):
    def setUp(self):
        self.t = FakeTransport()
        self.f = fleet.Fleet(MACH, self.t, clock=lambda: 1000.0)

    def test_old_and_stuck_sessions_are_marked(self):
        now, day = 10 * 86400.0, 86400
        cases = [  # (session, stale, stalled)
            (sess('a', mtime=now - 3 * day), True, False),                                         # closed, 3 days
            (sess('b', mtime=now - day), False, False),                                            # closed, yesterday
            (sess('c', mtime=now - 3 * day, kind='interactive', status='idle'), False, False),     # still open
            (sess('d', mtime=now - 8 * day, kind='interactive', status='idle'), True, False),      # open, a week
            (sess('e', mtime=now - 12 * day, kind='background', status='busy'), True, True),       # "working" for 12 days
            (sess('f', mtime=now - 7 * 3600, kind='background', status='busy'), False, True),      # stuck, but recent
            (sess('g', mtime=now - 60, kind='background', status='busy'), False, False),           # really working
            (sess('h', mtime=now - 3 * day, kind='background', status='stopped'), True, False),    # stopped counts as closed
        ]
        for s, stale, stalled in cases:
            m = fleet.mark_age(s, now)
            self.assertEqual((m['stale'], m['stalled']), (stale, stalled), s['sid'])
        self.t.replies['desk']['sessions'] = [c[0] for c in cases]
        f = fleet.Fleet(MACH, self.t, clock=lambda: now)
        f.poll()
        self.assertEqual(f.summary()['sessions'], 4)   # b, c, f, g (tower's lone session is long gone)
        self.assertNotIn('stale', self.t.replies['desk']['sessions'][0])   # the stored rows stay as the machine sent them

    def test_snapshot_and_summary(self):
        self.f.poll()
        snap = self.f.snapshot()
        self.assertEqual([m['name'] for m in snap['machines']], ['desk', 'tower'])
        self.assertTrue(all(m['online'] for m in snap['machines']))
        s = self.f.summary()
        self.assertEqual(s['seven_day'], 41)          # newest limits win
        self.assertEqual(s['limits_updated'], 90)     # and say when a session saw them
        self.assertEqual(s['cold'], 1)               # only big + cold counts
        self.assertEqual(s['sessions'], 2)

    def test_week_pace_projects_use_at_reset(self):
        now = 1_000_000.0
        self.t.replies['tower']['limits'].update(seven_day=40, seven_day_reset=now + 3.5 * 86400)
        f = fleet.Fleet(MACH, self.t, clock=lambda: now)
        f.poll()
        self.assertEqual(f.summary()['week_projected'], 80)   # 40% used halfway through the week

    def test_snapshot_carries_launcher_state(self):
        self.t.replies['desk']['launcher'] = {'alive': False, 'beat': None}
        self.f.poll()
        self.assertEqual(self.f.snapshot()['machines'][0]['launcher'], {'alive': False, 'beat': None})

    def test_offline_keeps_last_data(self):
        self.f.poll()
        self.t.down.add('desk')
        self.f.poll()
        m = self.f.snapshot()['machines'][0]
        self.assertFalse(m['online'])
        self.assertIn('timed out', m['error'])
        self.assertEqual(len(m['sessions']), 1)

    def test_a_stopped_chat_shows_stopped_before_the_machine_agrees(self):
        now = [1000.0]
        sid = 'cc' * 4 + '-1111-2222-3333-444444444444'
        self.t.replies['desk']['sessions'] = [sess(sid, kind='background', status='busy', mtime=990)]
        f = fleet.Fleet(MACH, self.t, clock=lambda: now[0])
        f.poll()
        row = lambda: f.snapshot()['machines'][0]['sessions'][0]
        f.mark_stopped('desk', sid)
        self.assertEqual(row()['status'], 'stopped')
        f.poll()  # claude stop lagged: the machine still says busy
        self.assertEqual(row()['status'], 'stopped')
        now[0] += f.STOP_HOLD + 1  # it never agreed: what it says wins again
        self.assertEqual(row()['status'], 'busy')
        f.mark_stopped('desk', sid)
        f.mark_stopped('desk', sid, on=False)  # resumed
        self.assertEqual(row()['status'], 'busy')

    def test_call_validates_machine_and_reports_offline(self):
        with self.assertRaises(fleet.FleetError):
            self.f.call('nope', {'verb': 'list'})
        self.t.down.add('desk')
        with self.assertRaises(fleet.FleetError) as e:
            self.f.call('desk', {'verb': 'stop'})
        self.assertIn('Desk', str(e.exception))


class GuardTimerTest(unittest.TestCase):
    def setUp(self):
        self.sent, self.timers = [], []
        class T:
            def __init__(s, delay, fn): s.fn, s.cancelled = fn, False; self.timers.append(s)
            def start(s): pass
            def cancel(s): s.cancelled = True
        self.g = guard.GuardTimer(20, self.sent.append, timer=T, clock=lambda: self.now)
        self.now = 1000.0
        self.evt = {'event': 'guard', 'machine': 'desk', 'session_id': 'S1', 'title': 'x', 'ctx': 640000, 'idle': 9 * 3600}

    def test_fires_once_after_delay(self):
        self.g.block(self.evt)
        self.g.block(dict(self.evt))       # duplicate block while pending
        self.assertEqual(len(self.timers), 1)
        self.timers[0].fn()
        self.assertEqual(len(self.sent), 1)
        self.assertTrue(self.g.held('desk', 'S1'))

    def test_answer_cancels(self):
        self.g.block(self.evt)
        self.g.answer({'machine': 'desk', 'session_id': 'S1'})
        self.assertTrue(self.timers[0].cancelled)
        self.timers[0].fn()                # a racing timer still must not send
        self.assertEqual(self.sent, [])
        self.assertFalse(self.g.held('desk', 'S1'))

    def test_one_push_per_held_message(self):
        self.g.block(self.evt); self.timers[0].fn()
        for _ in range(5):  # more messages typed into the held chat, hours apart: nothing new to say
            self.now += 3600
            self.g.block(self.evt)
        self.assertEqual((len(self.timers), len(self.sent)), (1, 1))
        self.assertTrue(self.g.held('desk', 'S1'))

    def test_after_an_answer_a_new_hold_waits_half_an_hour(self):
        self.g.block(self.evt); self.timers[0].fn()
        self.g.answer({'machine': 'desk', 'session_id': 'S1'})
        self.now += 600
        self.g.block(self.evt)
        self.assertEqual(len(self.timers), 1)
        self.assertTrue(self.g.held('desk', 'S1'))  # held, shown in the apps, but no second push
        self.g.answer({'machine': 'desk', 'session_id': 'S1'})
        self.now += guard.REPEAT_QUIET
        self.g.block(self.evt)
        self.assertEqual(len(self.timers), 2)

    def test_a_chat_that_moved_on_or_sat_for_hours_is_not_held(self):
        self.g.block(self.evt); self.timers[0].fn()
        self.g.moved({'machine': 'desk', 'session_id': 'S1'})
        self.assertFalse(self.g.held('desk', 'S1'))
        self.now += guard.REPEAT_QUIET + 1
        self.g.block(self.evt); self.timers[1].fn()
        self.now += guard.HELD_FOR + 1
        self.assertFalse(self.g.held('desk', 'S1'))


class ApnsTest(unittest.TestCase):
    def setUp(self):
        try:
            from cryptography.hazmat.primitives.asymmetric import ec
            from cryptography.hazmat.primitives import serialization
        except ImportError:
            self.skipTest('cryptography is not installed')
        self.key = ec.generate_private_key(ec.SECP256R1())
        d = Path(mkdtemp())
        self.kp = d / 'k.p8'
        self.kp.write_bytes(self.key.private_bytes(serialization.Encoding.PEM, serialization.PrivateFormat.PKCS8, serialization.NoEncryption()))
        self.ran = []

    def runner(self, status='200', body=''):
        def run(args, **kw):
            self.ran.append(args)
            class R: pass
            r = R(); r.returncode = 0; r.stdout = body + '\n' + status; r.stderr = ''
            return r
        return run

    def test_jwt_verifies_and_is_cached(self):
        import base64
        from cryptography.hazmat.primitives.asymmetric import ec
        from cryptography.hazmat.primitives.asymmetric.utils import encode_dss_signature
        from cryptography.hazmat.primitives import hashes
        now = [1000.0]
        a = apns.Apns(self.kp, 'KID123', 'TEAM', 'com.x', runner=self.runner(), clock=lambda: now[0])
        tok = a.jwt()
        h, c, sig = tok.split('.')
        pad = lambda s: s + '=' * (-len(s) % 4)
        self.assertEqual(json.loads(base64.urlsafe_b64decode(pad(h))), {'alg': 'ES256', 'kid': 'KID123'})
        self.assertEqual(json.loads(base64.urlsafe_b64decode(pad(c))), {'iss': 'TEAM', 'iat': 1000})
        raw = base64.urlsafe_b64decode(pad(sig))
        self.assertEqual(len(raw), 64)
        der = encode_dss_signature(int.from_bytes(raw[:32], 'big'), int.from_bytes(raw[32:], 'big'))
        self.key.public_key().verify(der, f'{h}.{c}'.encode(), ec.ECDSA(hashes.SHA256()))
        now[0] += 600
        self.assertEqual(a.jwt(), tok)
        now[0] += 3000
        self.assertNotEqual(a.jwt(), tok)

    def test_push_builds_curl_call(self):
        a = apns.Apns(self.kp, 'KID', 'TEAM', 'com.example.switchboard', runner=self.runner())
        status, _ = a.push('ab' * 32, 'Cold session held', 'body', '/s/desk/S1')
        self.assertEqual(status, 200)
        args = self.ran[0]
        self.assertIn('--http2', args)
        self.assertTrue(args[-1].endswith('/3/device/' + 'ab' * 32))
        payload = json.loads(args[args.index('-d') + 1])
        self.assertEqual(payload['url'], '/s/desk/S1')
        self.assertEqual(payload['aps']['alert']['title'], 'Cold session held')
        self.assertNotIn('category', payload['aps'])   # plain notification, no action buttons

    def test_rejects_bad_token(self):
        a = apns.Apns(self.kp, 'KID', 'TEAM', 'com.x', runner=self.runner())
        with self.assertRaises(ValueError):
            a.push('../../x', 't', 'b', '/')


class HttpTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.t = FakeTransport()
        cls.state = Path(mkdtemp())
        cls.pushes = []
        cls.app = server.App(fleet.Fleet(MACH, cls.t), state_dir=cls.state, web_dir=Path(__file__).resolve().parents[1] / 'server' / 'web',
                             pusher=lambda *a: cls.pushes.append(a), guard_delay=0.05)
        cls.app.fleet.poll()
        cls.httpd = server.make_server('127.0.0.1', 0, cls.app)
        cls.port = cls.httpd.server_address[1]
        threading.Thread(target=cls.httpd.serve_forever, daemon=True).start()

    @classmethod
    def tearDownClass(cls):
        cls.httpd.shutdown()

    def req(self, path, body=None, headers=None, host=None):
        h = {'Host': host or f'127.0.0.1:{self.port}'}
        if body is not None:
            h.update({'Content-Type': 'application/json', 'X-Switchboard': '1'})
        h.update(headers or {})
        r = urllib.request.Request(f'http://127.0.0.1:{self.port}{path}', data=None if body is None else json.dumps(body).encode(),
                                   headers=h, method='POST' if body is not None else 'GET')
        try:
            with urllib.request.urlopen(r, timeout=5) as resp:
                return resp.status, resp.read()
        except urllib.error.HTTPError as e:
            return e.code, e.read()

    def test_sessions(self):
        st, b = self.req('/api/sessions')
        self.assertEqual(st, 200)
        self.assertEqual(len(json.loads(b)['machines']), 2)

    def test_the_web_is_a_page_that_points_to_the_apps(self):
        st, b = self.req('/')
        self.assertEqual(st, 200)
        self.assertIn(b'Switchboard runs in its apps', b)
        self.assertNotIn(b'app.js', b)
        self.assertNotIn('web', json.loads(self.req('/api/sessions')[1]))  # no page build id: no pages to reload

    def test_only_what_the_native_apps_load_is_served(self):
        for path in ('/icon.svg', '/favicon.ico'):
            self.assertEqual(self.req(path)[0], 200, path)
        for path in ('/static/vendor/xterm.js', '/static/term-schemes.json', '/static/app.js', '/new', '/browse', '/t/desk/S1', '/s/desk/S1', '/manifest.webmanifest', '/static/../server.py'):
            self.assertEqual(self.req(path)[0], 404, path)

    def test_folder_tree_passes_the_parent(self):
        echo = lambda q: json.loads(self.req('/api/folders/desk' + q)[1])['echo']
        self.assertEqual(echo('?parent=Code%2Fapps'), {'verb': 'folders', 'parent': 'Code/apps'})
        self.assertEqual(echo('?parent='), {'verb': 'folders', 'parent': ''})
        self.assertEqual(echo(''), {'verb': 'folders'})

    def test_files_needs_an_agent_that_has_them(self):
        st, _ = self.req('/api/files/desk?folder=proj')
        self.assertEqual(st, 409)
        st = self.app.fleet.state['desk']
        had = st.get('features')
        st['features'] = ['files']
        try:
            echo = lambda q: json.loads(self.req('/api/files/desk' + q)[1])['echo']
            self.assertEqual(echo('?folder=proj&name=a.md'), {'verb': 'files', 'folder': 'proj', 'name': 'a.md'})
            self.assertEqual(echo(''), {'verb': 'files', 'folder': ''})
        finally:
            st['features'] = had

    def test_places_and_the_trust_question_before_new(self):
        st = self.app.fleet.state['desk']
        had = st.get('features')
        st['features'] = []
        b = json.loads(self.req('/api/places/desk')[1])
        self.assertTrue(b['old'])
        self.assertEqual([p['kind'] for p in b['places']], ['claude'])  # an older machine has only its Claude folder
        st, body = self.req('/api/new', {'machine': 'desk', 'folder': '~/Documents/site', 'prompt': 'hi'})
        self.assertEqual(st, 409)
        self.app.fleet.state['desk']['features'] = ['places']
        answers = {'trust': 'ask'}
        real = self.t.__call__
        def call(target, req, timeout, real=real):
            if req['verb'] == 'folder_check':
                self.t.calls.append(('desk', req))
                return {'ok': True, 'trust': answers['trust'], 'shown': '~/Documents/site', 'folder': req['folder']}
            return real(target, req, timeout)
        self.app.fleet.transport = call
        try:
            st, body = self.req('/api/new', {'machine': 'desk', 'folder': '~/Documents/site', 'prompt': 'hi'})
            b = json.loads(body)
            self.assertEqual((st, b['need'], b['folder_shown'], b['declined']), (409, 'trust', '~/Documents/site', False))
            answers['trust'] = 'no'
            self.assertTrue(json.loads(self.req('/api/new', {'machine': 'desk', 'folder': '~/Documents/site', 'prompt': 'hi'})[1])['declined'])
            self.t.calls.clear()
            st, body = self.req('/api/new', {'machine': 'desk', 'folder': '~/Documents/site', 'prompt': 'hi', 'trust': True})
            self.assertEqual(st, 200, body)
            for _ in range(100):
                starts = [c[1] for c in self.t.calls if c[1]['verb'] == 'start']
                if starts:
                    break
                time.sleep(0.02)
            self.assertIs(starts[0]['trust'], True)
            self.assertEqual(self.req('/api/new', {'machine': 'desk', 'folder': 'proj', 'prompt': 'hi'})[0], 200)  # the Claude folder: no question
            self.t.calls.clear()
            self.assertEqual(self.req('/api/privacy_settings', {'machine': 'desk'})[0], 200)
            self.assertIn('privacy_settings', [c[1]['verb'] for c in self.t.calls])
        finally:
            self.app.fleet.state['desk']['features'] = had

    def test_roam_setting(self):
        st, _ = self.req('/api/machines', {'op': 'roam', 'machine': 'desk', 'roam': False})
        self.assertEqual(st, 200)
        self.assertIs(self.app.fleet.machines['desk']['roam'], False)
        self.assertIs(self.app.fleet._list_req(self.app.fleet.machines['desk'])['roam'], False)
        self.assertFalse([m for m in json.loads(self.req('/api/machines')[1])['machines'] if m['name'] == 'desk'][0]['roam'])
        self.req('/api/machines', {'op': 'roam', 'machine': 'desk', 'roam': True})
        self.assertNotIn('roam', self.app.fleet.machines['desk'])
        self.assertEqual(self.req('/api/machines', {'op': 'roam', 'machine': 'desk', 'roam': 'yes'})[0], 400)

    def test_server_page_routes(self):
        st, body = self.req('/api/server')
        self.assertEqual(st, 200)
        i = json.loads(body)
        self.assertEqual(i['mode'], 'checkout')        # tests run from the repository
        self.assertFalse(i['can_manage'])
        self.assertIn('git pull', i['why_not'])
        self.assertEqual(self.req('/api/server/logs?part=../x')[0], 400)
        self.assertEqual(self.req('/api/server/update', {'version': '9.9.9', 'sha256': 'a' * 64})[0], 409)
        self.assertEqual(self.req('/api/server/restart', {})[0], 409)
        self.assertEqual(self.req('/api/server/restart', {}, headers={'X-Switchboard': ''})[0], 403)

    def test_post_requires_header(self):
        st, _ = self.req('/api/action', {'machine': 'desk', 'verb': 'stop', 'sid': 'x'}, headers={'X-Switchboard': ''})
        self.assertEqual(st, 403)

    def test_bad_host_refused(self):
        st, _ = self.req('/api/sessions', host='evil.example:8130')
        self.assertEqual(st, 403)

    def test_action_routes_allowed_verbs_only(self):
        st, b = self.req('/api/action', {'machine': 'desk', 'verb': 'stop', 'sid': 'S'})
        self.assertEqual(st, 200)
        self.assertEqual(json.loads(b)['echo']['verb'], 'stop')
        st, _ = self.req('/api/action', {'machine': 'desk', 'verb': 'mkdir', 'folder': 'x'})
        self.assertEqual(st, 400)

    def test_new_session_makes_folder_then_starts(self):
        self.t.calls.clear()
        st, b = self.req('/api/new', {'machine': 'tower', 'folder': 'newproj', 'create': True, 'prompt': 'hi', 'model': 'haiku'})
        self.assertEqual(st, 200, b)
        self.assertEqual(json.loads(b)['machine'], 'server')  # answered at once; the server does the rest
        for _ in range(100):
            verbs = [c[1]['verb'] for c in self.t.calls if c[0] == 'tower' and c[1]['verb'] in ('mkdir', 'start')]
            if len(verbs) == 2:
                break
            time.sleep(0.02)
        self.assertEqual(verbs, ['mkdir', 'start'])

    def test_guard_event_pushes_unless_answered(self):
        self.pushes.clear()
        self.req('/api/event', {'event': 'guard', 'machine': 'desk', 'session_id': 'S1', 'ctx': 640000, 'idle': 3600 * 9, 'title': 'Lights'})
        self.req('/api/event', {'event': 'guard', 'machine': 'desk', 'session_id': 'S2', 'ctx': 640000, 'idle': 3600 * 9, 'title': 'Other'})
        self.req('/api/event', {'event': 'guard_answer', 'machine': 'desk', 'session_id': 'S2'})
        time.sleep(0.3)
        self.assertEqual(len(self.pushes), 1)
        self.assertIn('Lights', self.pushes[0][1])
        self.assertEqual(self.pushes[0][2], '/s/desk/S1')
        self.assertEqual(self.pushes[0][3:], ('held', 'held-S1'))  # replaces an older one for the chat on the phone

    def test_each_kind_of_notification_can_be_turned_off(self):
        st, b = self.req('/api/prefs')
        self.assertEqual([n['key'] for n in json.loads(b)['notify_kinds']], ['held', 'jobs', 'cleanup', 'budget'])
        self.assertTrue(all(n['on'] for n in json.loads(b)['notify_kinds']))
        st, b = self.req('/api/prefs', {'notify': {'held': False}})
        self.assertEqual(st, 200, b)
        self.assertEqual(json.loads(b)['notify'], {'held': False, 'jobs': True, 'cleanup': True, 'budget': True})
        self.assertEqual(self.req('/api/prefs', {'notify': {'held': 'no'}})[0], 400)
        self.assertEqual(self.req('/api/prefs', {'notify': {'spam': True}})[0], 400)
        try:
            self.pushes.clear()
            self.req('/api/event', {'event': 'guard', 'machine': 'desk', 'session_id': 'S9', 'ctx': 640000, 'idle': 3600 * 9, 'title': 'Off'})
            time.sleep(0.3)
            self.assertEqual(self.pushes, [])
        finally:
            self.req('/api/prefs', {'notify': {'held': True}})

    def test_devices_and_summary(self):
        st, b = self.req('/api/devices', {'token': 'cd' * 32, 'name': 'iPhone'})
        self.assertEqual(st, 200)
        self.assertFalse(json.loads(b)['push'], 'no apns.json here: the phone is told notifications are not set up')
        self.assertIn('cd' * 32, (self.state / 'devices.json').read_text())
        st, _ = self.req('/api/devices', {'token': 'nothex'})
        self.assertEqual(st, 400)
        st, b = self.req('/api/summary')
        self.assertEqual(json.loads(b)['seven_day'], 41)


if __name__ == '__main__':
    unittest.main()


class ContinueTest(HttpTest):
    def test_continue_makes_target_folder_and_seeds_handoff(self):
        t = self.t
        orig = t.replies.copy()
        def call(target, req, timeout):
            name = 'desk' if target == 'me@x' else 'tower'
            t.calls.append((name, req))
            v = req['verb']
            if v == 'handoff': return {'ok': True, 'job': 'aaaaaaaaaaaa'}
            if v == 'job': return {'ok': True, 'state': 'done'}
            if v == 'handoff_text': return {'ok': True, 'text': 'NOTES', 'carry': {'chrome': True, 'effort': 'ultracode'}}
            if v == 'mkdir': return {'ok': True, 'folder': req['folder']}
            if v == 'start': return {'ok': True, 'job': 'bbbbbbbbbbbb'}
            return orig[name]
        self.app.fleet.transport = call
        try:
            t.calls.clear()
            st, b = self.req('/api/continue', {'machine': 'desk', 'sid': 'S1', 'to': 'tower', 'folder': 'moved', 'create': True})
            self.assertEqual(st, 200, b)
            for _ in range(50):
                if any(c[1]['verb'] == 'start' for c in t.calls): break
                time.sleep(0.05)
            start = [c for c in t.calls if c[1]['verb'] == 'start'][0]
            self.assertEqual(start[0], 'tower')
            self.assertEqual(start[1]['handoff_text'], 'NOTES')
            self.assertEqual([c[1]['verb'] for c in t.calls if c[0] == 'tower' and c[1]['verb'] != 'job'], ['mkdir', 'start'])
            self.assertEqual(start[1]['from'], 'Desk')
            self.assertEqual((start[1]['chrome'], start[1]['effort']), (True, 'ultracode'))  # what the old chat could do
            self.assertNotIn('topic', start[1])  # S1 is not on the list, so the new session is named from its own folder
            for _ in range(50):
                if any(c[1]['verb'] == 'supersede' for c in t.calls): break
                time.sleep(0.05)
            sup = [c for c in t.calls if c[1]['verb'] == 'supersede']
            self.assertEqual([(c[0], c[1]['sid'], c[1]['by']['machine']) for c in sup], [('desk', 'S1', 'tower')])
        finally:
            self.app.fleet.transport = t
