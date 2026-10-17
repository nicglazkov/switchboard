"""Auto machine: what each machine can do (caps), the rules, Claude's pick through the ask verb, and New session on Auto."""
import json, socket, sys, tempfile, threading, time, unittest
from pathlib import Path
from tempfile import mkdtemp
from tests.helpers import fresh_sb
sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'server'))
import features, fleet, server  # noqa: E402
from tests.test_server import MACH  # noqa: E402

NEW = 'cc' * 4 + '-1111-2222-3333-444444444444'


def caps(os_='mac', **tools):
    t = {'xcode': None, 'ios_sim': False, 'iphone': 'none', 'android_sdk': None, 'adb_devices': None, 'emulator': False, 'java': None,
         'rust': None, 'node': None, 'python': '3.12', 'go': None, 'docker': None, 'dotnet': None, 'msvc': None, 'cuda': None, 'tailscale': True}
    t.update(tools)
    return {'os': os_, 'arch': 'arm64', 'cpu': 'x', 'cores': 8, 'ram_gb': 64, 'ram_free_gb': 20.0, 'disk_free_gb': 100, 'gpu': None,
            'tools': t, 'repos': []}


DESK = dict(caps('mac', xcode='26.3', iphone='paired'), repos=['switchboard', 'tools/video-notes'], ram_free_gb=12.5)
PC = dict(caps('windows', android_sdk='37', adb_devices=1, cuda='13.2', msvc='2022', docker='29'), gpu='NVIDIA GeForce RTX 5070',
          repos=['garden-app'], ram_free_gb=44.0)


class Rules(unittest.TestCase):
    C = [('desk', 'Desk', DESK), ('tower', 'Tower', PC)]

    def pick(self, prompt, folder='', cands=None):
        return features.rules_pick(cands or self.C, prompt, folder)

    def test_keywords_send_work_where_the_tools_are(self):
        self.assertEqual(self.pick('Fix the SwiftUI layout in the iOS app')[0], 'desk')
        self.assertEqual(self.pick('Build the APK with gradle')[0], 'tower')
        self.assertEqual(self.pick('train a small model on the GPU')[0], 'tower')
        self.assertEqual(self.pick('Fix the Windows installer (MSVC)')[0], 'tower')

    def test_the_why_names_what_decided_it(self):
        self.assertEqual(self.pick('Fix the iOS app')[2], 'Has Xcode 26.')
        self.assertEqual(self.pick('Build the android app')[2], 'Has the Android SDK and a phone connected.')
        self.assertEqual(self.pick('pytorch training run')[2], 'Has CUDA 13.2 on the RTX 5070.')
        on = dict(DESK, tools=dict(DESK['tools'], iphone='connected'))
        self.assertEqual(self.pick('Run it on my iPhone', cands=[('desk', 'Desk', on), ('tower', 'Tower', PC)])[2],
                         'Has Xcode 26 and your iPhone connected.')

    def test_no_keyword_means_most_free_memory(self):
        name, folder, why = self.pick('Summarize the meeting notes')
        self.assertEqual((name, folder), ('tower', ''))
        self.assertEqual(why, 'Most free memory right now (44 GB).')

    def test_a_named_repo_wins_over_memory(self):
        name, folder, why = self.pick('Look at the switchboard server logs')
        self.assertEqual((name, folder, why), ('desk', 'switchboard', 'Has switchboard checked out.'))
        self.assertEqual(self.pick('fix video-notes search')[:2], ('desk', 'tools/video-notes'))
        self.assertEqual(self.pick('anything', folder='garden-app')[:2], ('tower', 'garden-app'))

    def test_a_need_nobody_has_says_so(self):
        name, _, why = self.pick('Fix the iOS app', cands=[('tower', 'Tower', PC)])
        self.assertEqual(name, 'tower')
        self.assertEqual(why, 'No online machine has Xcode; this one has the most free memory.')

    def test_words_inside_other_words_do_not_count(self):
        self.assertEqual(self.pick('Restrain the logging, it is too verbose')[0], 'tower')  # "train" inside "Restrain"
        self.assertEqual(self.pick('Fix the bios settings page')[0], 'tower')  # "ios" inside "bios"
        self.assertEqual(self.pick('Fix the bios settings page')[2], 'Most free memory right now (44 GB).')

    def test_can_line(self):
        self.assertEqual(features.can_line(DESK), 'iOS')
        self.assertEqual(features.can_line(PC), 'Android SDK + phone, CUDA (RTX 5070), Docker, Visual Studio')
        self.assertEqual(features.can_line(caps('linux')), 'General work')
        self.assertIsNone(features.can_line(None))

    def test_claude_answers_are_parsed_and_cleaned(self):
        d = features.parse_pick('```json\n{"machine": "desk", "folder": "", "why": "has Xcode \u2014 and the repo"}\n```')
        self.assertEqual(d['machine'], 'desk')
        self.assertEqual(features.clean_why(d['why']), 'Has Xcode, and the repo.')
        with self.assertRaises(ValueError):
            features.parse_pick('I would pick the Desk.')


class Fake:
    """Two machines that answer caps, ask, start and job."""
    def __init__(self):
        self.calls, self.lock = [], threading.Lock()
        self.caps = {'desk': DESK, 'tower': PC}
        self.answer = {'machine': 'desk', 'folder': 'switchboard', 'why': 'It has Xcode 26 and the switchboard repo'}
        self.ask_ok, self.launcher, self.down = True, True, set()

    def __call__(self, target, req, timeout):
        name = 'desk' if target == 'me@x' else 'tower'
        v = req['verb']
        with self.lock:
            self.calls.append((name, req))
        if name in self.down:
            raise OSError('ssh: connect to host x port 22: No route to host')
        if v == 'list':
            return {'ok': True, 'sessions': [], 'limits': None, 'launcher': {'alive': self.launcher} if name == 'desk' else {'alive': None}}
        if v == 'caps':
            return {'ok': True, 'caps': self.caps[name]} if self.caps.get(name) else {'ok': False, 'error': "unknown verb 'caps'"}
        if v == 'ask':
            if not self.ask_ok:
                return {'ok': False, 'error': 'Claude did not answer: claude -p failed rc=1'}
            return {'ok': True, 'text': 'Here you go:\n' + json.dumps(self.answer), 'ms': 2100}
        if v == 'start':
            return {'ok': True, 'job': '0123456789ab'}
        if v == 'mkdir':
            return {'ok': True, 'folder': req['folder']}
        if v == 'job':
            return {'ok': True, 'state': 'done', 'sid': NEW, 'url': 'https://claude.ai/code/session_NEW', 'name': 'Fix it (new, web)'}
        return {'ok': True}

    def verbs(self, verb):
        return [(n, r) for n, r in self.calls if r['verb'] == verb]


class AutoServer(unittest.TestCase):
    def make(self, on=('desk', 'tower'), fake=None):
        self.f = fake or Fake()
        self.state, self.cfg = Path(mkdtemp()), Path(mkdtemp())
        if on is not None:
            (self.cfg / 'caps.json').write_text(json.dumps({n: True for n in on}))
        self.pushes = []
        app = server.App(fleet.Fleet(MACH, self.f), state_dir=self.state, web_dir=self.state, sleep=lambda s: None, poll_every=0,
                         retry_waits=(0, 0), cfg_dir=self.cfg, pusher=lambda *a: self.pushes.append(a))
        app._refresh_soon = lambda *a, **k: None
        app.fleet.poll()
        app.feat.auto.refresh(app.fleet.snapshot()['machines'], wait=5)
        self.app = app
        return app

    def wait(self, jid):
        for _ in range(300):
            if self.app.jobs[jid]['state'] in ('done', 'error'):
                return self.app.jobs[jid]
            time.sleep(0.01)
        self.fail(f'job stuck: {self.app.jobs[jid]}')

    def test_caps_are_opt_in_per_machine(self):
        app = self.make(on=('tower',))
        self.assertEqual(app.get('/api/caps'), {'desk': None, 'tower': PC})
        snap = {m['name']: m for m in app.sessions()['machines']}
        self.assertIsNone(snap['desk']['caps'])
        self.assertIsNone(snap['desk']['can'])
        self.assertEqual(snap['tower']['can'], 'Android SDK + phone, CUDA (RTX 5070), Docker, Visual Studio')
        self.assertEqual([n for n, _ in self.f.verbs('caps')], ['tower'])  # never asked the Desk

    def test_no_file_means_auto_is_off(self):
        app = self.make(on=None)
        self.assertEqual(app.get('/api/caps'), {'desk': None, 'tower': None})
        with self.assertRaises(server.HttpError) as e:
            app.post('/api/route', {'prompt': 'x', 'rules': True})
        self.assertIn('Auto is off', str(e.exception))
        with self.assertRaises(server.HttpError):
            app.post('/api/new', {'machine': 'auto', 'prompt': 'x'})

    def test_turning_one_on_takes_effect_without_a_restart(self):
        app = self.make(on=())
        self.assertEqual(app.get('/api/caps')['desk'], None)
        time.sleep(0.01)
        (self.cfg / 'caps.json').write_text(json.dumps({'desk': True}))
        import os
        os.utime(self.cfg / 'caps.json', (time.time() + 5, time.time() + 5))
        app.feat.auto.refresh(app.fleet.snapshot()['machines'], wait=5)
        self.assertEqual(app.get('/api/caps')['desk'], DESK)

    def test_caps_are_asked_again_only_when_old(self):
        app = self.make()
        n = len(self.f.verbs('caps'))
        app.feat.after_poll()
        time.sleep(0.05)
        self.assertEqual(len(self.f.verbs('caps')), n)

    def test_an_old_agent_without_caps_is_retried_later_not_every_poll(self):
        f = Fake()
        f.caps['desk'] = None
        app = self.make(fake=f)
        self.assertIsNone(app.get('/api/caps')['desk'])
        n = len(f.verbs('caps'))
        app.feat.after_poll()
        time.sleep(0.05)
        self.assertEqual(len(f.verbs('caps')), n)

    def test_preview_is_rules_only(self):
        app = self.make()
        r = app.post('/api/route', {'prompt': 'Fix the iOS widget', 'rules': True})
        self.assertEqual((r['machine'], r['label'], r['why'], r['by']), ('desk', 'Desk', 'Has Xcode 26.', 'rules'))
        self.assertEqual(self.f.verbs('ask'), [])

    def test_claude_picks_on_the_pc_first(self):
        app = self.make()
        r = app.post('/api/route', {'prompt': 'Fix the switchboard iOS widget', 'folder': ''})
        self.assertEqual((r['machine'], r['folder'], r['by']), ('desk', 'switchboard', 'claude'))
        self.assertEqual(r['why'], 'It has Xcode 26 and the switchboard repo.')
        (asker, req), = self.f.verbs('ask')
        self.assertEqual(asker, 'tower')  # a PC runs claude over SSH; a Mac needs its launcher
        self.assertIn('"name":"desk"', req['prompt'])
        self.assertIn('Fix the switchboard iOS widget', req['prompt'])
        self.assertIn('JSON', req['system'])

    def test_a_mac_asks_only_with_its_launcher_up(self):
        app = self.make()
        self.f.down.add('tower')
        app.fleet.poll()
        self.f.launcher = False
        app.fleet.poll()
        r = app.post('/api/route', {'prompt': 'Fix the iOS widget'})
        self.assertEqual(r['by'], 'rules')
        self.assertEqual(self.f.verbs('ask'), [])
        self.f.launcher = True
        app.fleet.poll()
        r = app.post('/api/route', {'prompt': 'Fix the iOS widget'})
        self.assertEqual((r['by'], self.f.verbs('ask')[0][0]), ('claude', 'desk'))

    def test_claude_failing_falls_back_to_the_rules(self):
        app = self.make()
        self.f.ask_ok = False
        r = app.post('/api/route', {'prompt': 'pytorch training'})
        self.assertEqual((r['machine'], r['by']), ('tower', 'rules'))
        self.assertIn('rc=1', r['fallback'])

    def test_a_made_up_machine_falls_back_and_a_label_is_understood(self):
        app = self.make()
        self.f.answer = {'machine': 'MacStudio', 'why': 'x'}
        self.assertEqual(app.post('/api/route', {'prompt': 'notes'})['by'], 'rules')
        self.f.answer = {'machine': 'Tower', 'why': 'most memory'}
        r = app.post('/api/route', {'prompt': 'notes'})
        self.assertEqual((r['machine'], r['by'], r['why']), ('tower', 'claude', 'Most memory.'))

    def test_a_typed_folder_is_kept_and_made_when_new(self):
        app = self.make()
        self.f.answer = {'machine': 'tower', 'folder': 'garden-app', 'why': 'x'}
        r = app.post('/api/route', {'prompt': 'notes', 'folder': 'scratch'})
        self.assertEqual((r['folder'], r['create']), ('scratch', True))
        r = app.post('/api/route', {'prompt': 'notes', 'folder': 'GARDEN-APP'})
        self.assertEqual((r['folder'], r['create']), ('garden-app', False))
        self.f.answer = {'machine': 'tower', 'folder': '../../etc', 'why': 'x'}
        r = app.post('/api/route', {'prompt': 'notes'})
        self.assertEqual((r['folder'], r['create']), ('', False))  # only a repo that machine listed

    def test_offline_machines_are_not_picked(self):
        app = self.make()
        self.f.down.add('tower')
        app.fleet.poll()
        r = app.post('/api/route', {'prompt': 'pytorch training', 'rules': True})
        self.assertEqual(r['machine'], 'desk')
        self.assertEqual(r['why'], 'No online machine has CUDA; this one has the most free memory.')

    def test_new_session_on_auto(self):
        app = self.make()
        r = app.post('/api/new', {'machine': 'auto', 'prompt': 'Fix the switchboard iOS widget', 'folder': ''})
        self.assertEqual(r['machine'], 'server')
        j = self.wait(r['job'])
        self.assertEqual(j['state'], 'done')
        self.assertEqual(j['machine'], 'desk')
        self.assertEqual(j['picked'], 'Auto picked Desk. It has Xcode 26 and the switchboard repo.')
        (where, start), = self.f.verbs('start')
        self.assertEqual((where, start['folder'], start['prompt']), ('desk', 'switchboard', 'Fix the switchboard iOS widget'))
        view = app.job('server', r['job'])
        self.assertEqual((view['machine'], view['label'], view['by']), ('desk', 'Desk', 'claude'))
        self.assertEqual(view['picked'], j['picked'])

    def test_new_session_on_auto_with_a_new_folder(self):
        app = self.make()
        self.f.ask_ok = False
        r = app.post('/api/new', {'machine': 'auto', 'prompt': 'pytorch training', 'folder': 'runs'})
        j = self.wait(r['job'])
        self.assertEqual((j['state'], j['machine'], j['by']), ('done', 'tower', 'rules'))
        self.assertEqual([r['folder'] for _, r in self.f.verbs('mkdir')], ['runs'])

    def test_auto_with_nothing_online_says_so_at_once(self):
        app = self.make()
        self.f.down.update({'desk', 'tower'})
        app.fleet.poll()
        with self.assertRaises(server.HttpError) as e:
            app.post('/api/new', {'machine': 'auto', 'prompt': 'x'})
        self.assertEqual(str(e.exception), 'No machine Auto can use is online right now.')

    def test_an_auto_job_that_cannot_pick_fails_with_the_reason(self):
        app = self.make()
        real, calls = app.feat.auto.candidates, []

        def gone():  # online when the tap is taken, gone a moment later when the job runs
            calls.append(1)
            return real() if len(calls) == 1 else ([], real()[1])
        app.feat.auto.candidates = gone
        r = app.post('/api/new', {'machine': 'auto', 'prompt': 'x'})
        j = self.wait(r['job'])
        self.assertEqual((j['state'], j['machine']), ('error', 'auto'))
        self.assertEqual(j['error'], 'No machine Auto can use is online right now.')
        self.assertEqual(self.f.verbs('start'), [])


class CapsProbe(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp()
        self.sb = fresh_sb(self.tmp)

    def test_shape_and_speed_here(self):
        t0 = time.time()
        c = self.sb.probe_caps(self.tmp)
        self.assertLess(time.time() - t0, 4.0)
        for k in ('os', 'arch', 'cpu', 'cores', 'ram_gb', 'ram_free_gb', 'disk_free_gb', 'gpu', 'tools', 'repos'):
            self.assertIn(k, c)
        self.assertEqual(set(c['tools']), {'xcode', 'ios_sim', 'iphone', 'android_sdk', 'adb_devices', 'emulator', 'java', 'rust', 'node',
                                           'python', 'go', 'docker', 'dotnet', 'msvc', 'cuda', 'tailscale'})
        self.assertIn(c['tools']['iphone'], ('connected', 'paired', 'none'))
        self.assertIsInstance(c['cores'], int)
        json.dumps(c)

    def test_repos_two_levels_down_newest_first(self):
        root = Path(self.tmp) / 'root'
        for rel in ('a', 'grp/b', 'grp/c/d', 'node_modules/x', '.hidden'):
            (root / rel / '.git').mkdir(parents=True)
        self.assertEqual(sorted(self.sb._repos(root)), ['a', 'grp/b'])  # not hidden ones, not node_modules, not three down

    def test_cached_six_hours_with_live_bits_fresh(self):
        sb = self.sb
        calls = []
        sb.probe_caps = lambda root=None: calls.append(1) or {'tools': {'xcode': None, 'android_sdk': None, 'iphone': 'none'}, 'repos': []}
        sb._ram = lambda: (64, 33.3)
        a = sb.machine_caps(1000.0)
        b = sb.machine_caps(1000.0 + 5 * 3600)
        self.assertEqual(len(calls), 1)
        self.assertEqual((a['measured'], b['measured'], b['ram_free_gb']), (1000.0, 1000.0, 33.3))
        sb.machine_caps(1000.0 + 7 * 3600)
        self.assertEqual(len(calls), 2)
        sb.machine_caps(1000.0 + 7 * 3600, force=True)
        self.assertEqual(len(calls), 3)

    def test_adb_devices_are_read_from_its_socket(self):
        srv = socket.socket()
        srv.bind(('127.0.0.1', 0))
        srv.listen(1)
        self.sb.ADB_ADDR = srv.getsockname()
        got = []

        def serve():
            c, _ = srv.accept()
            got.append(c.recv(64))
            body = b'R5CT1234\tdevice\nemulator-5554\toffline\n'
            c.sendall(b'OKAY' + b'%04x' % len(body) + body)
            c.close()

        threading.Thread(target=serve, daemon=True).start()
        self.assertEqual(self.sb._adb_devices(), 1)
        self.assertEqual(got, [b'000chost:devices'])
        srv.close()
        self.sb.ADB_ADDR = ('127.0.0.1', 1)  # nothing there: no server means no devices, and adb is never started
        self.assertEqual(self.sb._adb_devices(), 0)

    def test_caps_verb(self):
        self.sb.machine_caps = lambda now, force=False: {'os': 'linux', 'forced': force}
        r = self.sb.rpc_feat('caps', {'refresh': True}, 1.0, lambda: [])
        self.assertEqual(r, {'ok': True, 'caps': {'os': 'linux', 'forced': True}})


class Ask(unittest.TestCase):
    def setUp(self):
        self.sb = fresh_sb(tempfile.mkdtemp())

    def test_pc_answers_inline(self):
        r = self.sb.rpc_ask({'prompt': 'p', 'system': 's'}, 1.0, ask=lambda p, s: f'{p}|{s}', mac=False)
        self.assertEqual(r['text'], 'p|s')
        with self.assertRaises(self.sb.RpcError):
            self.sb.rpc_ask({'prompt': 'p', 'system': 's'}, 1.0, ask=lambda p, s: 1 / 0, mac=False)
        with self.assertRaises(ValueError):
            self.sb.rpc_ask({'prompt': '', 'system': 's'}, 1.0, mac=False)

    def test_mac_needs_the_launcher(self):
        self.sb.launcher_status = lambda now: {'alive': False}
        with self.assertRaises(self.sb.RpcError) as e:
            self.sb.rpc_ask({'prompt': 'p', 'system': 's'}, time.time(), mac=True)
        self.assertIn('launcher', str(e.exception))

    def test_mac_asks_through_the_launcher(self):
        sb = self.sb
        sb.launcher_status = lambda now: {'alive': True}
        stop = threading.Event()
        threading.Thread(target=sb.ask_worker, args=(stop,), kwargs={'ask': lambda p, s: '{"machine": "desk"}'}, daemon=True).start()
        try:
            r = sb.rpc_ask({'prompt': 'p', 'system': 's'}, time.time(), mac=True)
            self.assertEqual(r['text'], '{"machine": "desk"}')
            self.assertEqual(list(sb._asks_dir().iterdir()), [])  # the asker cleans up after itself
        finally:
            stop.set()

    def test_mac_ask_errors_and_timeouts(self):
        sb = self.sb
        sb.launcher_status = lambda now: {'alive': True}
        stop = threading.Event()
        threading.Thread(target=sb.ask_worker, args=(stop,), kwargs={'ask': lambda p, s: 1 / 0}, daemon=True).start()
        try:
            with self.assertRaises(sb.RpcError) as e:
                sb.rpc_ask({'prompt': 'p', 'system': 's'}, time.time(), mac=True)
            self.assertIn('Claude did not answer', str(e.exception))
        finally:
            stop.set()
        sb.ASK_WAIT = 0.3  # nobody answering
        with self.assertRaises(sb.RpcError) as e:
            sb.rpc_ask({'prompt': 'p', 'system': 's'}, time.time(), mac=True)
        self.assertIn('too long', str(e.exception))


if __name__ == '__main__':
    unittest.main()
