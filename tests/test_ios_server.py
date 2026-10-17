"""Server pieces the native iPhone app relies on (2026-10-12): addresses a phone may be given, relaunch in the app's
form, attachments as JSON, the end of the terminal for waiting chats, Bonjour, and Live Activity pushes."""
import base64, json, sys, tempfile, threading, time, unittest, urllib.error, urllib.request
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
        self.assertEqual(req['flags'], [['--permission-mode', 'acceptEdits'], ['--add-dir', 'notes'],
                                        ['--add-dir', 'site'], ['--chrome']])
        self.assertEqual(req['model'], 'sonnet')

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


if __name__ == '__main__':
    unittest.main()
