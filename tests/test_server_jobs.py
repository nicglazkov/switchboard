import json, sys, threading, time, unittest
from pathlib import Path
from tempfile import mkdtemp
sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'server'))
import fleet, server  # noqa: E402
from tests.test_server import MACH, sess  # noqa: E402

SID = 'aa' * 4 + '-1111-2222-3333-444444444444'


class Machine:
    """A scripted machine: queues a job, then reports queued, running, done."""
    def __init__(self):
        self.calls, self.down, self.lock = [], 0, threading.Lock()
        self.progress = ['queued', 'running', 'done']
        self.result = {'url': 'https://claude.ai/code/session_NEW', 'name': 'tA (fresh)', 'sid': 'cc' * 4 + '-1111-2222-3333-444444444444'}

    def __call__(self, target, req, timeout):
        with self.lock:
            self.calls.append(req)
            if self.down > 0:
                self.down -= 1
                raise OSError('ssh: connect to host x port 22: No route to host')
        v = req['verb']
        if v == 'list':
            return {'ok': True, 'sessions': [sess(SID, title='Lights')], 'limits': None}
        if v in ('fresh', 'resume', 'handoff', 'start'):
            return {'ok': True, 'job': '0123456789ab'}
        if v == 'mkdir':
            return {'ok': True, 'folder': req['folder']}
        if v == 'handoff_text':
            return {'ok': True, 'text': 'NOTES'}
        if v == 'job':
            st = self.progress.pop(0) if len(self.progress) > 1 else self.progress[0]
            return dict({'ok': True, 'state': st}, **(self.result if st == 'done' else {'error': 'boom'} if st == 'error' else {}))
        return {'ok': True}


class ServerJobTest(unittest.TestCase):
    def make(self, m=None, state=None):
        self.m = m or Machine()
        self.pushes = []
        self.state = state or Path(mkdtemp())
        app = server.App(fleet.Fleet(MACH, self.m), state_dir=self.state, web_dir=self.state, pusher=lambda *a: self.pushes.append(a[:3]),
                         sleep=lambda s: None, poll_every=0, retry_waits=(0, 0))
        app._refresh_soon = lambda *a, **k: None
        app.fleet.poll()
        return app

    def wait(self, app, jid, state=('done', 'error')):
        for _ in range(200):
            if app.jobs[jid]['state'] in state:
                return app.jobs[jid]
            time.sleep(0.01)
        self.fail(f'job stuck: {app.jobs[jid]}')

    def test_a_second_resume_click_while_the_first_runs_gets_the_same_job(self):
        app = self.make()
        app._drive = lambda jid: None  # both clicks land while the first is still on its way
        a = app.post('/api/action', {'machine': 'desk', 'verb': 'resume', 'sid': SID}, 'SwitchboardDesktop/1')
        b = app.post('/api/action', {'machine': 'desk', 'verb': 'resume', 'sid': SID}, 'SwitchboardDesktop/1')
        self.assertEqual(a['job'], b['job'])
        app.jobs[a['job']]['state'] = 'done'
        c = app.post('/api/action', {'machine': 'desk', 'verb': 'resume', 'sid': SID}, 'SwitchboardDesktop/1')
        self.assertNotEqual(a['job'], c['job'])

    def test_start_fresh_answers_at_once_and_finishes_without_the_app(self):
        app = self.make()
        r = app.post('/api/action', {'machine': 'desk', 'verb': 'fresh', 'sid': SID}, 'Mozilla/5.0 SwitchboardApp/1')
        self.assertTrue(r['job'].startswith('s-'))
        self.assertEqual(r['machine'], 'server')
        app.jobs[r['job']]['watched'] = 0  # the app was closed right after the tap
        j = self.wait(app, r['job'])
        self.assertEqual(j['state'], 'done')
        self.assertEqual(len(self.pushes), 1)
        title, body, url = self.pushes[0]
        self.assertEqual(title, 'Fresh chat ready')
        self.assertIn('tA (fresh) on Desk', body)
        self.assertEqual(url, '/s/desk/' + 'cc' * 4 + '-1111-2222-3333-444444444444')
        self.assertEqual(json.loads((self.state / 'jobs.json').read_text())[r['job']]['state'], 'done')

    def test_no_push_while_the_app_is_watching(self):
        app = self.make()
        r = app.post('/api/action', {'machine': 'desk', 'verb': 'fresh', 'sid': SID})
        while app.jobs[r['job']]['state'] not in ('done', 'error'):
            app.job('server', r['job'])
            time.sleep(0.005)
        self.assertEqual(self.pushes, [])
        self.assertEqual(app.job('server', r['job'])['name'], 'tA (fresh)')

    def test_unreachable_machine_is_retried(self):
        m = Machine()
        m.down = 2
        app = self.make(m)
        m.down = 3  # after the first poll; the first refusal is the "is anything still running" check
        r = app.post('/api/action', {'machine': 'desk', 'verb': 'fresh', 'sid': SID})
        self.assertIn('busy', [c['verb'] for c in m.calls])
        self.assertEqual(self.wait(app, r['job'])['state'], 'done')
        self.assertEqual(sum(1 for c in m.calls if c['verb'] == 'fresh'), 3)  # two refused connections, then queued once

    def test_machine_down_for_good_fails_with_a_push(self):
        app = self.make()
        self.m.down = 99
        r = app.post('/api/action', {'machine': 'desk', 'verb': 'fresh', 'sid': SID})
        app.jobs[r['job']]['watched'] = 0
        j = self.wait(app, r['job'])
        self.assertEqual(j['state'], 'error')
        self.assertIn('not reachable', j['error'])
        deadline = time.time() + 3  # the push goes out right after the job is marked failed
        while not self.pushes and time.time() < deadline:
            time.sleep(0.02)
        self.assertEqual(self.pushes[0][0], "Start fresh didn't finish")
        self.assertEqual(self.pushes[0][2], f'/s/desk/{SID}')

    def test_launch_error_reported(self):
        app = self.make()
        self.m.progress = ['running', 'error']
        r = app.post('/api/action', {'machine': 'desk', 'verb': 'resume', 'sid': SID})
        j = self.wait(app, r['job'])
        self.assertEqual((j['state'], j['error']), ('error', 'boom'))

    def test_new_session_carries_origin_and_mode(self):
        app = self.make()
        r = app.post('/api/new', {'machine': 'desk', 'folder': 'x', 'create': True, 'prompt': 'go', 'mode': 'ultracode'},
                     'Mozilla/5.0 (iPhone) SwitchboardApp/1')
        self.wait(app, r['job'])
        start = [c for c in self.m.calls if c['verb'] == 'start'][0]
        self.assertEqual((start['origin'], start['mode'], start['folder']), ('iPhone', 'ultracode', 'x'))
        self.assertEqual([c['verb'] for c in self.m.calls if c['verb'] in ('mkdir', 'start')], ['mkdir', 'start'])
        with self.assertRaises(server.HttpError):
            app.post('/api/new', {'machine': 'desk', 'folder': 'x', 'mode': 'turbo'})
        r = app.post('/api/new', {'machine': 'desk', 'folder': 'x'}, 'Mozilla/5.0 Safari')
        self.wait(app, r['job'])
        self.assertEqual([c for c in self.m.calls if c['verb'] == 'start'][-1]['origin'], 'web')
        r = app.post('/api/new', {'machine': 'desk', 'folder': 'x'}, 'SwitchboardDesktop/0.1.0')  # the native app
        self.wait(app, r['job'])
        self.assertEqual([c for c in self.m.calls if c['verb'] == 'start'][-1]['origin'], 'desktop')
        r = app.post('/api/new', {'machine': 'desk', 'folder': 'x'}, 'Mozilla/5.0 (Linux; Android 16) SwitchboardAndroid/1')
        self.wait(app, r['job'])
        self.assertEqual([c for c in self.m.calls if c['verb'] == 'start'][-1]['origin'], 'Android')

    def test_continue_names_the_source(self):
        app = self.make()
        r = app.post('/api/continue', {'machine': 'desk', 'sid': SID, 'to': 'tower', 'folder': 'moved'})
        self.wait(app, r['job'])
        start = [c for c in self.m.calls if c['verb'] == 'start'][0]
        self.assertEqual((start['from'], start['topic'], start['handoff_text']), ('Desk', 'Lights', 'NOTES'))
        self.assertEqual(app.jobs[r['job']]['machine'], 'tower')

    def test_restart_keeps_watching_a_handed_off_launch(self):
        st = Path(mkdtemp())
        now = time.time()
        (st / 'jobs.json').write_text(json.dumps({
            's-aaaaaaaaaaaa': {'kind': 'fresh', 'machine': 'desk', 'plan': {'req': {'verb': 'fresh', 'sid': SID}}, 'title': 'Lights',
                               'state': 'running', 'step': 'running', 'created': now - 30, 'watched': 0, 'inner': '0123456789ab'},
            's-bbbbbbbbbbbb': {'kind': 'start', 'machine': 'desk', 'plan': {'req': {'verb': 'start'}}, 'title': 'New session',
                               'state': 'queued', 'step': 'sent', 'created': now - 30, 'watched': 0}}))
        app = self.make(state=st)
        self.assertEqual(self.wait(app, 's-aaaaaaaaaaaa')['state'], 'done')
        self.assertEqual(app.jobs['s-bbbbbbbbbbbb']['state'], 'error')
        self.assertIn('restarted', app.jobs['s-bbbbbbbbbbbb']['error'])

    def test_a_job_is_asked_about_every_second_at_first_then_less_often(self):
        m = Machine()
        m.progress = ['running'] * 30 + ['done']
        now, slept = [1000.0], []

        def sleep(s):
            slept.append(s)
            now[0] += s
        app = server.App(fleet.Fleet(MACH, m), state_dir=Path(mkdtemp()), web_dir=Path(mkdtemp()), clock=lambda: now[0],
                         sleep=sleep, poll_every=3, retry_waits=(0, 0))
        app.jobs['s-0123456789ab'] = {'kind': 'resume', 'machine': 'desk', 'state': 'queued', 'created': now[0]}
        self.assertEqual(app._watch('s-0123456789ab', 'desk', '0123456789ab')['state'], 'done')
        self.assertEqual(slept[:server.QUICK_FOR], [1] * server.QUICK_FOR, 'a resume that is up in a few seconds opens then')
        self.assertTrue(slept[server.QUICK_FOR:] and set(slept[server.QUICK_FOR:]) == {3}, 'a long job is not asked every second')


OLD = 'bb' * 4 + '-1111-2222-3333-444444444444'


class CompactMachine(Machine):
    """Two chats of one thread on one machine, with the compact verbs."""
    def __call__(self, target, req, timeout):
        v = req['verb']
        if v == 'list':
            with self.lock:
                self.calls.append(req)
            return {'ok': True, 'features': ['compact'], 'limits': None,
                    'sessions': [sess(OLD, title='Lights', mtime=time.time() - 9000), sess(SID, title='Lights (fresh)', mtime=time.time() - 100)]}
        if v == 'thread_pieces':
            with self.lock:
                self.calls.append(req)
            return {'ok': True, 'pieces': [{'sid': x, 'kind': 'handoff' if x == OLD else 'condensed', 'tokens': 10_000,
                                            **({} if req.get('measure') else {'text': 'T' + x[:2], 'title': 't'})} for x in req['sids']]}
        if v == 'thread_cost':
            with self.lock:
                self.calls.append(req)
            return {'ok': True, 'tokens': req['tokens'], 'summary': 1, 'first': 2, 'total': 3, 'each': 4, 'text': 'About 3 up front.'}
        if v == 'compact':
            with self.lock:
                self.calls.append(req)
            return {'ok': True, 'job': '0123456789ab'}
        return super().__call__(target, req, timeout)


class CompactTest(unittest.TestCase):
    make, wait = ServerJobTest.make, ServerJobTest.wait

    def thread(self):
        app = self.make(CompactMachine())
        app.feat.threads.observe(app.sessions()['machines'])
        app.feat.threads.record(f'desk/{SID}', f'desk/{OLD}', 'fresh')
        return app

    def test_cost_first_and_nothing_written(self):
        app = self.thread()
        r = app.post('/api/compact', {'machine': 'desk', 'sid': OLD})
        self.assertEqual((r['chats'], r['found'], r['notes'], r['sid']), (2, 2, 1, SID))
        self.assertEqual(r['cost']['text'], 'About 3 up front.')
        cost = [c for c in self.m.calls if c['verb'] == 'thread_cost'][0]
        self.assertEqual(cost['tokens'], 20_120)          # both chats plus their headings
        self.assertFalse([c for c in self.m.calls if c['verb'] == 'compact'])

    def test_confirmed_it_sends_every_piece_oldest_first_and_links_the_new_chat(self):
        app = self.thread()
        r = app.post('/api/compact', {'machine': 'desk', 'sid': SID, 'confirm': True})
        j = self.wait(app, r['job'])
        self.assertEqual(j['state'], 'done', j)
        c = [c for c in self.m.calls if c['verb'] == 'compact'][0]
        self.assertEqual((c['sid'], [p['sid'] for p in c['pieces']]), (SID, [OLD, SID]))
        new = self.m.result['sid']
        link = app.feat.threads.store()['auto'][f'desk/{new}']
        self.assertEqual((link['to'], link['how']), (f'desk/{SID}', 'compacted'))

    def test_an_earlier_chats_summary_can_be_read(self):
        app = self.thread()
        self.assertEqual(app.get(f'/api/handoff/desk/{OLD}'), {'ok': True, 'text': 'NOTES'})

    def test_a_lone_chat_has_nothing_to_compact(self):
        app = self.make(CompactMachine())
        with self.assertRaises(server.HttpError):
            app.post('/api/compact', {'machine': 'desk', 'sid': SID})

    def test_an_old_agent_is_named(self):
        m = CompactMachine()
        base = m.__call__
        m2 = lambda t, req, timeout: dict(base(t, req, timeout), features=[]) if req['verb'] == 'list' else base(t, req, timeout)
        app = self.make(m2)
        self.m = m
        app.feat.threads.observe(app.sessions()['machines'])
        app.feat.threads.record(f'desk/{SID}', f'desk/{OLD}', 'fresh')
        with self.assertRaises(server.HttpError) as e:
            app.post('/api/compact', {'machine': 'desk', 'sid': SID})
        self.assertIn('Desk needs the Switchboard update', str(e.exception))


if __name__ == '__main__':
    unittest.main()
