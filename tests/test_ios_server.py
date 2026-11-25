"""Server pieces the native iPhone app relies on (2026-10-12): addresses a phone may be given, relaunch in the app's
form, attachments as JSON, the end of the terminal for waiting chats, Bonjour, and Live Activity pushes."""
import base64, json, os, sys, tempfile, threading, time, unittest, urllib.error, urllib.request
from pathlib import Path
from tempfile import mkdtemp
from unittest import mock
from tests.helpers import fresh_sb, user, assistant, entry, write_transcript

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'server'))
import apns, bonjour, fleet, server  # noqa: E402

SID = 'aaaaaaaa-1111-2222-3333-444444444444'
NEW = 'bbbbbbbb-1111-2222-3333-444444444444'
NOW = 1_800_000_000
TOK = 'ab' * 40


def write(p, d):
    p.parent.mkdir(parents=True, exist_ok=True)
    p.write_text(json.dumps(d))


class HostsTest(unittest.TestCase):
    def cfg(self, allow='unset'):
        d = Path(mkdtemp())
        conf = {'bind': ['0.0.0.0'], 'hosts': ['studio'], 'origins': ['http://studio:8130']}
        if allow != 'unset':
            conf['allow'] = allow
        write(d / 'cfg' / 'server.json', conf)
        return server.ServerConfig(d / 'cfg', d / 'state')

    def test_lan_bonjour_and_tailscale_addresses_are_let_in(self):
        c = self.cfg()
        for ok in ('192.168.1.20', '10.4.5.6', '172.20.1.1', '100.64.0.11', 'fe80::1', 'fd12:3456::1',  # leakscan: allow (one of each range)
                   'studio.local', 'my-mac.local', 'studio.tail1234.ts.net', 'box.example.ts.net', 'studio', 'localhost'):
            self.assertTrue(c.host_ok(ok), ok)

    def test_the_world_stays_out(self):
        c = self.cfg()
        for bad in ('8.8.8.8', '1.1.1.1', '2606:4700::1111', 'evil.com', 'studio.local.evil.com', 'ts.net.evil.com',
                    'local', '.local', 'ts.net', 'a..local', ''):
            self.assertFalse(c.host_ok(bad), bad)

    def test_each_kind_can_be_turned_off(self):
        c = self.cfg([])
        for bad in ('10.0.0.10', 'studio.local', 'box.example.ts.net', '100.64.0.11'):
            self.assertFalse(c.host_ok(bad), bad)
        self.assertTrue(c.host_ok('studio'))
        c = self.cfg(['tailscale'])
        self.assertTrue(c.host_ok('100.64.0.11') and c.host_ok('box.example.ts.net'))
        self.assertFalse(c.host_ok('10.0.0.10') or c.host_ok('studio.local'))

    def test_the_port_and_brackets_come_off_first(self):
        app = server.App(fleet.Fleet([], lambda *a: {}), state_dir=Path(mkdtemp()), web_dir=Path(mkdtemp()),
                         server_cfg=self.cfg())
        self.assertTrue(app.host_ok('10.0.0.10:8130'))
        self.assertTrue(app.host_ok('[fe80::1]:8130'))
        self.assertTrue(app.host_ok('studio.local:8130'))
        self.assertFalse(app.host_ok('evil.com:8130'))


class PhoneServerTest(unittest.TestCase):
    def setUp(self):
        self.calls, self.lock, self.busy = [], threading.Lock(), {'live': False}
        self.feats = ['flags', 'upload', 'busy', 'relaunch']
        self.sess = {'sid': SID, 'title': 'Chat', 'mtime': NOW, 'active': NOW, 'kind': 'background', 'status': 'idle',
                     'bg_id': 'aaaaaaaa', 'now': {'model': 'opus', 'effort': 'high', 'perm': 'acceptEdits', 'chrome': False},
                     'launch': {'flags': [['--remote-control'], ['--permission-mode', 'acceptEdits'], ['--add-dir', 'docs']],
                                'other': [], 'from': 'roster'}}
        me = self

        def transport(target, req, timeout):
            with me.lock:
                me.calls.append(req)
            v = req['verb']
            if v == 'list':
                return {'ok': True, 'replaced': [], 'sessions': [me.sess], 'features': me.feats}
            if v == 'busy':
                return dict(me.busy, ok=True)
            if v == 'resume':
                return {'ok': True, 'job': '0123456789ab'}
            if v == 'job':
                time.sleep(0.05)
                return {'ok': True, 'state': 'done', 'sid': NEW, 'bg_id': 'bbbbbbbb', 'name': 'Chat'}
            if v == 'upload':
                return {'ok': True, 'path': '/Users/you/.switchboard/uploads/' + req['name'], 'name': req['name']}
            return {'ok': True}
        machines = [{'name': 'studio', 'label': 'Studio', 'target': 'me@x', 'os': 'mac'}]
        self.app = server.App(fleet.Fleet(machines, transport, clock=lambda: NOW), state_dir=Path(mkdtemp()), web_dir=Path(mkdtemp()),
                              cfg_dir=Path(mkdtemp()), clock=lambda: NOW, sleep=lambda s: time.sleep(0.005), poll_every=0)
        self.app._refresh_soon = lambda *a, **k: None
        self.app.fleet.poll()

    def verbs(self, v):
        with self.lock:
            return [c for c in self.calls if c['verb'] == v]

    def wait(self, jid):
        for _ in range(400):
            if self.app.jobs[jid]['state'] in ('done', 'error'):
                return self.app.jobs[jid]
            time.sleep(0.01)
        self.fail(f'job stuck: {self.app.jobs[jid]}')

    def relaunch(self, flags, **kw):
        return self.app.post('/api/action', dict({'machine': 'studio', 'verb': 'relaunch', 'sid': SID, 'flags': flags}, **kw))

    # ---- relaunch with the app's change list
    def test_a_change_applies_to_what_it_was_launched_with(self):
        r = self.relaunch({'perm': ['plan'], 'effort': ['max'], 'remove': ['dirs']})
        self.assertEqual(self.wait(r['job'])['state'], 'done')
        req = self.verbs('resume')[0]
        self.assertTrue(req['relaunch'])
        self.assertEqual(req['flags'], [['--permission-mode', 'plan']])
        self.assertEqual((req['model'], req['effort']), ('opus', 'max'))

    def test_adding_folders_and_chrome(self):
        r = self.relaunch({'dirs': ['notes', 'site'], 'chrome': ['on'], 'model': ['sonnet'], 'remove': []})
        self.wait(r['job'])
        req = self.verbs('resume')[0]
        # the phone sends only the folders it adds: the one the chat was launched with stays
        self.assertEqual(req['flags'], [['--permission-mode', 'acceptEdits'], ['--add-dir', 'docs'], ['--add-dir', 'notes'],
                                        ['--add-dir', 'site'], ['--chrome']])
        self.assertEqual(req['model'], 'sonnet')

    def test_a_folder_sent_twice_is_added_once(self):
        r = self.relaunch({'dirs': ['docs', 'notes'], 'remove': []})
        self.wait(r['job'])
        self.assertEqual(self.verbs('resume')[0]['flags'], [['--permission-mode', 'acceptEdits'], ['--add-dir', 'docs'],
                                                            ['--add-dir', 'notes']])

    def test_a_chat_in_the_machines_own_default_is_not_told_to_ask_first(self):
        # Nothing in its launch said a mode, and the agent fills `now` with the machine's default: an explicit
        # "Asks first" is never added for it (an older Switchboard there refuses the name, and it is the default anyway).
        self.sess['launch']['flags'] = [['--remote-control'], ['--add-dir', 'docs']]
        for perm, want in (('default', []), ('manual', []), ('plan', [['--permission-mode', 'plan']])):
            self.calls.clear()
            self.sess['now'] = dict(self.sess['now'], perm=perm)
            self.app.fleet.poll()
            r = self.relaunch({'effort': ['max'], 'remove': []})
            self.wait(r['job'])
            self.assertEqual(self.verbs('resume')[0].get('flags', []), [['--add-dir', 'docs']] + want, perm)

    def test_bypass_comes_along_only_when_it_was_launched_so(self):
        self.sess['now'] = dict(self.sess['now'], perm='bypassPermissions')
        for launch, want in (([], []), ([['--permission-mode', 'acceptEdits']], []),
                             ([['--permission-mode', 'bypassPermissions']], [['--permission-mode', 'bypassPermissions']]),
                             ([['--dangerously-skip-permissions']], [['--permission-mode', 'bypassPermissions']])):
            self.calls.clear()
            self.sess['launch']['flags'] = launch
            self.app.fleet.poll()
            r = self.relaunch({'effort': ['max'], 'remove': []})
            self.wait(r['job'])
            got = [f for f in self.verbs('resume')[0].get('flags', []) if f[0] == '--permission-mode']
            self.assertEqual(got, want, launch)

    def test_asks_first_that_cannot_be_sent_is_said(self):
        r = self.relaunch({'perm': ['default'], 'remove': []})  # this machine's Switchboard does not say its modes
        self.assertIn("Studio can't be told to ask first until Switchboard is updated there", r['note'])
        self.wait(r['job'])
        self.assertEqual(self.verbs('resume')[0]['flags'], [['--add-dir', 'docs']])
        self.app.fleet.state['studio']['models'] = {'perms': ['acceptEdits', 'auto', 'manual', 'plan']}
        self.assertNotIn('note', self.relaunch({'perm': ['default'], 'remove': []}))

    def test_a_pinned_model_and_the_mode_it_runs_in_now_are_kept(self):
        self.sess['now'] = dict(self.sess['now'], perm='plan')  # Shift+Tab since the launch
        self.sess['launch']['flags'] += [['--model', 'claude-opus-4-8'], ['--worktree', 'fix'], ['--no-chrome']]
        self.app.fleet.poll()
        r = self.relaunch({'effort': ['max'], 'remove': []})
        self.wait(r['job'])
        req = self.verbs('resume')[0]
        self.assertEqual(req['model'], 'claude-opus-4-8')
        self.assertEqual(req['flags'], [['--permission-mode', 'plan'], ['--add-dir', 'docs'], ['--no-chrome']])  # no worktree

    def test_asks_first_goes_by_name_only_where_the_machine_lists_it(self):
        r = self.relaunch({'remove': ['perm']})
        self.wait(r['job'])
        self.assertEqual(self.verbs('resume')[0]['flags'], [['--add-dir', 'docs']])  # an agent that names no modes
        self.calls.clear()
        self.app.fleet.state['studio']['models'] = {'perms': ['acceptEdits', 'auto', 'manual', 'plan']}
        r = self.relaunch({'perm': ['default'], 'remove': []})
        self.wait(r['job'])
        self.assertEqual(self.verbs('resume')[0]['flags'], [['--add-dir', 'docs'], ['--permission-mode', 'manual']])

    def test_bad_changes_are_refused(self):
        for flags in ({'dangerously': ['x'], 'remove': []}, {'perm': 'plan'}, {'perm': ['a', 'b']}, {'remove': 'perm'}, ['perm']):
            with self.assertRaises(server.HttpError, msg=flags):
                self.relaunch(flags)
        with self.assertRaises(server.HttpError):
            self.relaunch({'perm': ['yolo'], 'remove': []})  # flags.parse checks the value
        self.assertEqual(self.verbs('resume'), [])

    def test_with_work_running_it_waits_unless_told_now(self):
        self.busy = {'live': True, 'subagents': ['Audit'], 'commands': [], 'replying': False, 'pending': [],
                     'summary': '1 subagent is still running.'}
        r = self.relaunch({'effort': ['low'], 'remove': []})
        time.sleep(0.1)
        self.assertEqual(self.app.jobs[r['job']]['step'], 'waiting')
        self.assertEqual(self.verbs('resume'), [])
        self.busy = {'live': False}
        self.assertEqual(self.wait(r['job'])['state'], 'done')
        r = self.relaunch({'effort': ['low'], 'remove': []}, when='now')
        self.wait(r['job'])

    # ---- Live Activity
    def test_a_live_activity_follows_the_job_and_ends_with_it(self):
        sent = []
        self.app.live_pusher = lambda tok, state, end, after: sent.append((tok, dict(state), end)) or False
        self.busy = {'live': True, 'subagents': ['Audit'], 'commands': [], 'replying': False, 'pending': [],
                     'summary': '1 subagent is still running.'}
        r = self.relaunch({'effort': ['low'], 'remove': []})
        time.sleep(0.05)
        self.assertEqual(self.app.post('/api/live', {'job': r['job'], 'token': TOK}), {'ok': True, 'done': False})
        self.busy = {'live': False}
        self.wait(r['job'])
        for _ in range(100):
            if sent and sent[-1][2]:
                break
            time.sleep(0.01)
        self.assertTrue(all(t == TOK for t, _, _ in sent))
        last = sent[-1]
        self.assertTrue(last[2])
        self.assertEqual(last[1], {'step': 'Chat', 'progress': 1.0, 'done': True, 'failed': False,
                                   'link': f'switchboard://session/studio/{NEW}'})
        steps = [s['step'] for _, s, end in sent if not end]
        self.assertIn('1 subagent is still running.', steps)
        self.assertNotIn('live', self.app.jobs[r['job']])  # nothing more goes to an ended activity

    def test_the_push_that_ends_an_activity_is_tried_again_when_apple_is_not_reached(self):
        # Audit, iOS 15: one failed push, and the Lock Screen said "Starting..." until the system gave up on it.
        sent = []
        answers = ['retry', 'retry', False]
        self.app.live_pusher = lambda tok, state, end, after: sent.append(end) or (answers.pop(0) if end else False)
        r = self.relaunch({'effort': ['low'], 'remove': []})
        self.app.post('/api/live', {'job': r['job'], 'token': TOK})
        self.wait(r['job'])
        for _ in range(200):
            if 'live' not in self.app.jobs[r['job']]:
                break
            time.sleep(0.01)
        self.assertEqual([e for e in sent if e], [True, True, True], 'twice not reached, then through')
        self.assertNotIn('live', self.app.jobs[r['job']])

    def test_an_ending_push_that_never_gets_through_stops_after_a_few_tries(self):
        sent = []
        self.app.live_pusher = lambda tok, state, end, after: sent.append(end) or 'retry'
        r = self.relaunch({'effort': ['low'], 'remove': []})
        self.app.post('/api/live', {'job': r['job'], 'token': TOK})
        self.wait(r['job'])
        for _ in range(200):
            if 'live' not in self.app.jobs[r['job']]:
                break
            time.sleep(0.01)
        self.assertEqual(len([e for e in sent if e]), 1 + len(server.LIVE_END_RETRY))
        self.assertNotIn('live', self.app.jobs[r['job']])

    def test_live_tokens_are_checked(self):
        with self.assertRaises(server.HttpError):
            self.app.post('/api/live', {'job': 's-000000000000', 'token': 'nope'})
        with self.assertRaises(server.HttpError):
            self.app.post('/api/live', {'job': 's-000000000000', 'token': TOK})

    def test_the_activity_push_is_well_formed(self):
        runs = []

        class R:
            stdout, stderr = '\n200', ''
        a = apns.Apns.__new__(apns.Apns)
        a.topic, a.sandbox, a.clock = 'com.example.sb', True, lambda: NOW
        a.jwt = lambda: 'JWT'
        a.runner = lambda args, **k: runs.append(args) or R()
        self.assertEqual(a.push_activity(TOK, {'step': 'x', 'progress': 0.5, 'done': False, 'failed': False, 'link': None})[0], 200)
        args = runs[0]
        self.assertIn('apns-topic: com.example.sb.push-type.liveactivity', args)
        self.assertIn('apns-push-type: liveactivity', args)
        body = json.loads(args[args.index('-d') + 1])['aps']
        self.assertEqual((body['event'], body['timestamp'], body['content-state']['progress']), ('update', NOW, 0.5))
        a.push_activity(TOK, {}, end=True, dismiss_after=60)
        body = json.loads(runs[1][runs[1].index('-d') + 1])['aps']
        self.assertEqual((body['event'], body['dismissal-date']), ('end', NOW + 60))
        with self.assertRaises(ValueError):
            a.push_activity('zz', {})

    # ---- attachments as JSON
    def test_an_attachment_as_json(self):
        httpd = server.make_server('127.0.0.1', 0, self.app)
        port = httpd.server_address[1]
        threading.Thread(target=httpd.serve_forever, daemon=True).start()
        try:
            def post(body, ctype='application/json'):
                req = urllib.request.Request(f'http://127.0.0.1:{port}/api/upload', data=json.dumps(body).encode(), method='POST',
                                             headers={'Host': f'127.0.0.1:{port}', 'Content-Type': ctype, 'X-Switchboard': '1'})
                try:
                    with urllib.request.urlopen(req, timeout=10) as resp:
                        return resp.status, json.loads(resp.read())
                except urllib.error.HTTPError as e:
                    return e.code, json.loads(e.read())
            code, r = post({'machine': 'studio', 'sid': SID, 'name': 'shot.png', 'data': base64.b64encode(b'PNGDATA').decode()})
            self.assertEqual(code, 200, r)
            self.assertEqual(r['path'], '/Users/you/.switchboard/uploads/shot.png')
            self.assertEqual(base64.b64decode(self.verbs('upload')[0]['data']), b'PNGDATA')
            self.assertEqual(post({'machine': 'studio', 'name': 'x', 'data': '%%%'})[0], 400)
            self.assertEqual(post({'machine': 'studio', 'name': 'x', 'data': ''})[0], 400)  # empty
        finally:
            httpd.shutdown()


class FakeApple:
    """Apple's two push services. `good`: token -> the service it belongs to (True: development builds)."""
    def __init__(self, good, sandbox=True):
        self.good, self.sandbox, self.sent, self.down = good, sandbox, [], set()

    def _answer(self, tok, sandbox):
        sb = self.sandbox if sandbox is None else sandbox
        self.sent.append((tok[:2], sb))
        if sb in self.down:
            return 0, 'curl: (7) Failed to connect'
        if self.good.get(tok) == 'old':
            return 410, '{"reason":"Unregistered"}'
        if self.good.get(tok) == 'busy':
            return 429, '{"reason":"TooManyRequests"}'
        return (200, '') if self.good.get(tok) is sb else (400, '{"reason":"BadDeviceToken"}')

    def push(self, tok, title, body, url, thread=None, collapse=None, sandbox=None):
        return self._answer(tok, sandbox)

    def push_widgets(self, tok, sandbox=None):
        return self._answer(tok, sandbox)

    def push_activity(self, tok, state, end=False, dismiss_after=240, sandbox=None):
        return self._answer(tok, sandbox)


class PushTokensTest(unittest.TestCase):
    """Audit, server 6: when a phone's token is forgotten, and that two phones registering at once both stay."""
    DEV, STORE, DEAD, OLD, BUSY = 'aa' * 32, 'bb' * 32, 'cc' * 32, 'dd' * 32, 'ee' * 32

    def make(self, sandbox=True):
        self.apple = FakeApple({self.DEV: True, self.STORE: False, self.OLD: 'old', self.BUSY: 'busy'}, sandbox)
        self.app = server.App(fleet.Fleet([], lambda t, r, to: {'ok': True}), state_dir=Path(mkdtemp()), web_dir=Path(mkdtemp()),
                              cfg_dir=Path(mkdtemp()), poll_every=0)
        self.push = server.Pushers(self.apple, sleep=lambda s: None)
        self.push.attach(self.app)
        for t in (self.DEV, self.STORE, self.DEAD, self.OLD, self.BUSY):
            self.app.post('/api/devices', {'token': t, 'name': 'iPhone'})
            self.app.post('/api/widget-token', {'token': t})

    def test_a_token_is_tried_on_the_other_service_before_it_is_forgotten(self):
        for sandbox in (True, False):
            self.make(sandbox)
            self.push.alert('t', 'b', '/')
            self.assertEqual(set(self.app.devices()), {self.DEV, self.STORE, self.BUSY}, 'only the token both refuse, and the one Apple unregistered, go')
            self.assertIn((self.STORE[:2], False), self.apple.sent)
            self.assertIn((self.DEV[:2], True), self.apple.sent)
            self.apple.sent.clear()
            self.push.alert('t', 'b', '/')  # where each one worked is remembered: one try each from now on
            self.assertEqual(sorted(self.apple.sent), [(self.DEV[:2], True), (self.STORE[:2], False), (self.BUSY[:2], sandbox)])

    def test_a_service_that_cannot_be_reached_forgets_nobody(self):
        self.make()
        self.apple.down = {False}  # the other service: the token could be good there, nobody can tell
        self.push.alert('t', 'b', '/')
        self.assertIn(self.STORE, self.app.devices())
        self.apple.down = {True, False}
        self.push.alert('t', 'b', '/')
        self.push.widgets()
        self.assertEqual(set(self.app.devices()), {self.DEV, self.STORE, self.DEAD, self.BUSY}, 'only the one Apple unregistered went')
        self.assertEqual(len(self.app.widget_tokens()), 5)

    def test_the_apps_are_told_how_notifications_are_doing(self):
        self.make()
        st = self.app.get('/api/prefs')['push']
        self.assertEqual((st['ready'], st['state'], st['devices'], st['last'], st['test']), (True, 'ready', 5, None, True))
        self.push.alert('Done', 'b', '/')
        last = self.app.get('/api/prefs')['push']['last']
        self.assertEqual((last['ok'], last['title'], last['sent'], last['of']), (False, 'Done', 2, 5))
        self.assertIn('phone', last['why'], 'the first phone that did not get it says why, in words')

    def test_send_a_test_says_how_each_phone_went(self):
        self.make()
        r = self.app.post('/api/push/test', {})
        self.assertFalse(r['ok'])
        self.assertEqual([d['ok'] for d in r['devices']], [True, True, False, False, False])
        self.assertTrue(all(d['why'] for d in r['devices'] if not d['ok']))
        r = self.app.post('/api/push/test', {})  # the two Apple no longer knows are forgotten; the busy one stays
        self.assertEqual([d['ok'] for d in r['devices']], [True, True, False])
        self.assertIn('slow down', r['why'])
        for t in (self.BUSY,):
            self.app.token_gone('devices.json', t)
        r = self.app.post('/api/push/test', {})
        self.assertEqual((r['ok'], r['why'], len(r['devices'])), (True, None, 2))
        self.assertTrue(self.app.get('/api/prefs')['push']['last']['ok'])

    def test_a_test_with_no_phone_or_no_setup_says_what_is_missing(self):
        app = server.App(fleet.Fleet([], lambda t, r, to: {'ok': True}), state_dir=Path(mkdtemp()), web_dir=Path(mkdtemp()),
                         cfg_dir=Path(mkdtemp()), poll_every=0)
        r = app.post('/api/push/test', {})
        self.assertEqual((r['ok'], r['devices']), (False, []))
        self.assertIn('apns.json', r['why'])
        push = server.Pushers(FakeApple({}), sleep=lambda s: None)
        push.attach(app)
        r = app.post('/api/push/test', {})
        self.assertFalse(r['ok'])
        self.assertIn('no phone has registered', r['why'])
        self.assertIn('no phone has registered', app.get('/api/prefs')['push']['explain'])

    def test_a_broken_setup_is_named_and_picked_up_once_fixed(self):
        cfg = Path(mkdtemp())
        self.assertEqual(server.load_push(cfg), (None, None), 'no apns.json: never set up')
        (cfg / 'apns.json').write_text('{"key_id": "K", "team": "T", "topic": "x.y"')
        self.assertEqual(server.load_push(cfg)[1], 'apns.json is not valid JSON.')
        (cfg / 'apns.json').write_text('{"key_id": "K", "topic": "x.y"}')
        self.assertEqual(server.load_push(cfg)[1], 'apns.json has no "team".')
        (cfg / 'apns.json').write_text('{"key_id": "K", "team": "T", "topic": "x.y"}')
        why = server.load_push(cfg)[1]
        self.assertRegex(why, r'^The key file AuthKey_K\.p8 is missing\.$')
        now = [1000.0]
        loads = [(None, why), (FakeApple({}), None)]
        app = server.App(fleet.Fleet([], lambda t, r, to: {'ok': True}), state_dir=Path(mkdtemp()), web_dir=Path(mkdtemp()),
                         cfg_dir=Path(mkdtemp()), poll_every=0)
        push = server.Pushers(None, sleep=lambda s: None, why=why, load=lambda: loads.pop(0), clock=lambda: now[0])
        push.attach(app)
        st = app.get('/api/prefs')['push']
        self.assertEqual((st['ready'], st['state'], st['test']), (False, 'broken', False))
        self.assertIn('AuthKey_K.p8 is missing', st['explain'])
        self.assertNotIn('Notifications are off', st['explain'])
        self.assertEqual(len(loads), 2, 'not read again within the minute')
        now[0] += 61
        self.assertEqual(app.get('/api/prefs')['push']['state'], 'broken')
        now[0] += 61
        st = app.get('/api/prefs')['push']
        self.assertEqual((st['ready'], st['state']), (True, 'ready'))
        self.assertTrue(app.push_ready and app.widget_pusher)

    def test_apples_answers_come_in_words(self):
        import apns
        self.assertIsNone(apns.why(200, ''))
        self.assertIn('could not be reached', apns.why(0, 'curl: (7)'))
        self.assertIn('"key_id" and "team"', apns.why(403, '{"reason":"InvalidProviderToken"}'))
        self.assertIn('"topic"', apns.why(400, '{"reason":"DeviceTokenNotForTopic"}'))
        self.assertEqual(apns.why(400, '{"reason":"PayloadTooLarge"}'), 'Apple refused the notification: PayloadTooLarge.')
        self.assertIn('problem right now', apns.why(503, ''))

    def test_widget_and_activity_tokens_follow_the_same_rule(self):
        self.make()
        self.push.widgets()
        self.assertEqual(set(self.app.widget_tokens()), {self.DEV, self.STORE, self.BUSY})
        self.assertEqual(self.push.live(self.STORE, {}, True, 60), False)
        self.assertEqual(self.push.live(self.DEAD, {}, True, 60), True)
        self.assertEqual(self.push.live(self.OLD, {}, True, 60), True)
        self.assertEqual(self.push.live(self.BUSY, {}, True, 60), 'retry')
        self.apple.down = {True, False}
        self.assertEqual(self.push.live(self.DEV, {}, True, 60), 'retry')

    def test_a_phone_that_registers_while_a_push_is_out_is_kept(self):
        self.make()
        new, real = 'ff' * 32, self.apple.push

        def slow(tok, *a, **k):
            if tok == self.OLD:  # between reading the phones and forgetting this one, another registers
                self.app.post('/api/devices', {'token': new, 'name': 'New phone'})
            return real(tok, *a, **k)
        self.apple.push = slow
        self.push.alert('t', 'b', '/')
        self.assertIn(new, self.app.devices())
        self.assertNotIn(self.OLD, self.app.devices())

    def test_phones_registering_at_the_same_moment_all_stay(self):
        self.make()
        toks = [f'{i:064x}' for i in range(1, 9)]
        start = threading.Event()

        def reg(t):
            start.wait()
            self.app.post('/api/devices', {'token': t, 'name': 'iPhone'})
            self.app.post('/api/widget-token', {'token': t})
        ts = [threading.Thread(target=reg, args=(t,)) for t in toks]
        for t in ts:
            t.start()
        start.set()
        for t in ts:
            t.join()
        self.assertTrue(set(toks) <= set(self.app.devices()))
        self.assertEqual(len(self.app.widget_tokens()), 8, 'the newest eight')
        self.assertEqual(list(self.app.state_dir.glob('*.tmp')), [])

    def test_settings_saved_at_the_same_moment_all_land(self):
        self.make()
        self.app.fleet.add_machine({'name': 'studio', 'label': 'Studio', 'target': 'me@x', 'os': 'mac'})
        start = threading.Event()
        changes = [{'chrome_machine': 'studio'}, {'notify': {'held': False}}, {'ai_auto_machine': True}] * 4

        def save(body):
            start.wait()
            self.app.post('/api/prefs', body)
        ts = [threading.Thread(target=save, args=(b,)) for b in changes]
        for t in ts:
            t.start()
        start.set()
        for t in ts:
            t.join()
        d = json.loads((self.app.state_dir / 'prefs.json').read_text())
        self.assertEqual((d['chrome_machine'], d['notify']['held'], d['ai_auto_machine']), ('studio', False, True))


class TailTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp()
        self.sb = fresh_sb(self.tmp)

    def test_what_a_waiting_chat_shows(self):
        rows = [user('2026-10-10T01:00:00Z', 'Clean the build.'),
                assistant('2026-10-10T01:00:05Z', 'I will check first.', tools=[('Bash', {'command': 'ls build'})]),
                entry('user', '2026-10-10T01:00:06Z', message={'role': 'user', 'content': [{'type': 'tool_result', 'content': 'a b'}]}),
                assistant('2026-10-10T01:00:07Z', 'Two folders.\n\nNow removing them:', tools=[('Bash', {'command': 'rm -rf build/a\n  build/b'})]),
                assistant('2026-10-10T01:00:08Z', 'side work', side=True)]
        p = write_transcript(self.tmp, rows)
        self.assertEqual(self.sb.waiting_tail(p), ['Two folders.', 'Now removing them:', '⏺ Bash(rm -rf build/a build/b)'])

    def test_it_is_capped(self):
        text = '\n'.join(f'line {i} ' + 'x' * 300 for i in range(20))
        p = write_transcript(self.tmp, [user('2026-10-10T01:00:00Z', 'go'), assistant('2026-10-10T01:00:05Z', text)])
        out = self.sb.waiting_tail(p, 123.0)
        self.assertEqual(len(out), self.sb.TAIL_LINES)
        self.assertTrue(out[-1].startswith('line 19 ') and len(out[-1]) == self.sb.TAIL_WIDTH and out[-1].endswith('…'))
        Path(p).write_text('')  # remembered until the transcript changes
        self.assertEqual(self.sb.waiting_tail(p, 123.0), out)
        self.assertEqual(self.sb.waiting_tail(p, 124.0), [])


class BonjourTest(unittest.TestCase):
    def test_the_system_tool_announces_when_zeroconf_is_missing(self):
        started, logs = [], []
        cfg = mock.Mock(origins=['http://192.0.2.11:8130'])
        with mock.patch.dict(sys.modules, {'zeroconf': None}):
            how = bonjour.announce(8130, cfg, logs.append, which=lambda t: '/usr/bin/dns-sd' if t == 'dns-sd' else None,
                                   popen=lambda args, **k: started.append(args))
        self.assertEqual(how, 'dns-sd')
        self.assertEqual(started[0][:5], ['/usr/bin/dns-sd', '-R', started[0][2], '_switchboard._tcp', 'local'])
        self.assertIn('url=http://192.0.2.11:8130', started[0])

    def test_nothing_to_announce_with_says_so(self):
        logs = []
        with mock.patch.dict(sys.modules, {'zeroconf': None}):
            self.assertIsNone(bonjour.announce(8130, mock.Mock(origins=[]), logs.append, which=lambda t: None, popen=None))
        self.assertIn('not announced', logs[-1])

    def test_the_announced_address_is_one_on_the_local_network(self):
        cfg = mock.Mock(origins=['http://100.64.0.10:8130', 'http://10.0.0.5:8130/', 'http://studio:8130'])
        self.assertEqual(bonjour._url(8130, cfg), 'http://10.0.0.5:8130')
        cfg = mock.Mock(origins=['http://100.64.0.10:8130', 'http://studio:8130'])
        self.assertEqual(bonjour._url(8130, cfg), 'http://100.64.0.10:8130')

    def test_the_kits_zeroconf_is_used_when_the_machine_has_none(self):
        try:
            import zeroconf  # noqa: F401
            self.skipTest('zeroconf is installed here')
        except ImportError:
            pass
        with tempfile.TemporaryDirectory() as vendor:
            os.makedirs(os.path.join(vendor, 'zeroconf'))
            with open(os.path.join(vendor, 'zeroconf', '__init__.py'), 'w') as f:
                f.write('class ServiceInfo: pass\nclass Zeroconf: pass\nKIT = True\n')
            path = list(sys.path)
            with mock.patch.dict(sys.modules):
                sys.modules.pop('zeroconf', None)
                try:
                    info, zc = bonjour._zeroconf(vendor)
                    self.assertTrue(sys.modules['zeroconf'].KIT)
                    self.assertEqual((info.__name__, zc.__name__), ('ServiceInfo', 'Zeroconf'))
                    self.assertEqual(sys.path[-1], vendor)  # after the machine's own packages
                finally:
                    sys.path[:] = path
        self.assertEqual(bonjour._zeroconf(os.path.join(vendor, 'gone')), (None, None))


if __name__ == '__main__':
    unittest.main()
