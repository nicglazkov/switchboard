"""The Auto machine pick and dispatch, end to end: /api/route (the New sheet's preview), New with machine "auto", and
one session sending work to another machine (/api/dispatch), against the lab's three fake machines and its fake
Claude pick (scripts/lab/fakefleet.py fake_ask, which picks with the server's own rules, a little slower)."""
import json, time, unittest

from tests.e2e.lab import LabCase, lab_test


def route(lab, prompt, folder='', **kw):
    code, r = lab.post('/api/route', dict({'prompt': prompt, 'folder': folder}, **kw))
    assert code == 200, r
    return r


@lab_test
class AutoPick(LabCase):
    def setUp(self):
        for f in ('ask-fail', 'ask-slow', 'off-studio', 'off-laptop', 'off-tower'):
            self.lab.flag(f, False)
        self.lab.wait(lambda: all(m['online'] for m in self.lab.sessions()['machines']), 15, what='every machine back online')

    def test_claude_picks_the_machine_with_what_the_task_needs(self):
        cases = [('Build the iOS widget and run it on the simulator', {'studio', 'laptop'}, 'Xcode'),
                 ('Train the classifier with CUDA on the GPU', {'tower'}, 'CUDA'),
                 ('Build the Windows installer with MSVC', {'tower'}, 'Visual Studio'),
                 ('Run the Android app on the emulator', {'laptop', 'tower'}, 'Android')]
        for prompt, want, words in cases:
            r = route(self.lab, prompt)
            self.assertIn(r['machine'], want, (prompt, r))
            self.assertEqual(r['by'], 'claude', r)
            self.assertIn(words, r['why'], r)
            self.assertEqual(r['label'], self.lab.machine(r['machine'])['label'])

    def test_a_named_repo_goes_where_it_is_checked_out(self):
        r = route(self.lab, 'Fix the flaky test in net-monitor')
        self.assertEqual((r['machine'], r['folder'], r['create']), ('tower', 'net-monitor', False), r)
        r = route(self.lab, 'Fix the flaky test', 'music-player')
        self.assertEqual((r['machine'], r['folder'], r['create']), ('laptop', 'music-player', False), r)

    def test_a_folder_no_machine_has_is_made_on_the_pick(self):
        r = route(self.lab, 'Start a new notes app', 'brand-new-thing')
        self.assertEqual((r['folder'], r['create']), ('brand-new-thing', True), r)

    def test_when_claude_cannot_pick_the_rules_do(self):
        self.lab.flag('ask-fail')
        r = route(self.lab, 'Train the classifier with CUDA on the GPU')
        self.assertEqual((r['machine'], r['by']), ('tower', 'rules'), r)
        self.assertIn('fake', r['fallback'])
        r = route(self.lab, 'anything', rules=True)  # the sheet's instant preview never waits for Claude
        self.assertEqual(r['by'], 'rules')

    def test_an_offline_machine_is_never_picked(self):
        self.lab.flag('off-tower')
        self.lab.wait(lambda: self.lab.machine('tower')['online'] is False, 15, what='the tower to drop off')
        r = route(self.lab, 'Train the classifier with CUDA on the GPU')
        self.assertNotEqual(r['machine'], 'tower', r)

    def test_nothing_online_says_so(self):
        for m in ('studio', 'laptop', 'tower'):
            self.lab.flag(f'off-{m}')
        self.lab.wait(lambda: not any(m['online'] for m in self.lab.sessions()['machines']), 15, what='every machine offline')
        code, r = self.lab.post('/api/route', {'prompt': 'anything'})
        self.assertEqual(code, 400)
        self.assertEqual(r['error'], 'No machine Auto can use is online right now.')
        code, r = self.lab.post('/api/new', {'machine': 'auto', 'prompt': 'anything'})
        self.assertEqual((code, r['error']), (400, 'No machine Auto can use is online right now.'))

    def test_new_with_auto_starts_on_the_pick(self):
        before = len(self.lab.rpcs('start'))
        code, r = self.lab.post('/api/new', {'machine': 'auto', 'prompt': 'Build the Windows installer with MSVC', 'folder': 'outage-map'},
                                ua='SwitchboardApp/3.0')
        self.assertEqual(code, 200, r)
        self.assertEqual(r['machine'], 'server')
        j = self.lab.job(r['job'])
        self.assertEqual(j['state'], 'done', j)
        self.assertEqual(j['seen'][0], ('queued', 'choosing'))
        self.assertEqual(j['machine'], 'tower')
        self.assertTrue(j['picked'].startswith('Auto picked Tower.'), j)
        starts = self.lab.rpcs('start')[before:]
        self.assertEqual([s['target'] for s in starts], ['fake@tower'])  # only the pick is asked to start it
        self.assertEqual(starts[0]['req']['folder'], 'outage-map')
        self.assertEqual(starts[0]['req']['origin'], 'iPhone')
        self.lab.wait(lambda: self.lab.session('tower', j['sid']), 5, what='the new chat in the tower list')

    def test_new_with_auto_makes_a_missing_folder_on_the_pick(self):
        code, r = self.lab.post('/api/new', {'machine': 'auto', 'prompt': 'Train the classifier with CUDA', 'folder': 'gpu-sandbox'})
        self.assertEqual(code, 200, r)
        j = self.lab.job(r['job'])
        self.assertEqual((j['state'], j['machine']), ('done', 'tower'), j)
        mk = [x for x in self.lab.rpcs('mkdir') if x['req']['folder'] == 'gpu-sandbox']
        self.assertEqual([x['target'] for x in mk], ['fake@tower'])

    def test_auto_is_off_until_a_machine_shares_what_it_can_do(self):
        caps = self.lab.ctrl / 'config' / 'caps.json'
        was = caps.read_text()
        try:
            caps.write_text(json.dumps({'studio': False, 'laptop': False, 'tower': False}))
            code, r = self.lab.post('/api/route', {'prompt': 'anything'})
            self.assertEqual((code, r['error']), (400, 'Auto is off: no machine shares what it can do yet.'))
            caps.write_text(json.dumps({'studio': True, 'laptop': False, 'tower': False}))
            time.sleep(0.05)
            self.assertEqual(route(self.lab, 'Train the classifier with CUDA')['machine'], 'studio')  # the only one it may use
        finally:
            caps.write_text(was)


@lab_test
class AutoSlowClaude(LabCase):
    FLAGS = ('fast', 'ask-slow')

    @unittest.expectedFailure  # report bug 4: a slow Claude holds the pick for 80 s (40 s per machine that asks)
    def test_a_slow_claude_does_not_hold_up_the_new_sheet(self):
        t0 = time.time()
        r = route(self.lab, 'Train the classifier with CUDA')
        took = time.time() - t0
        self.assertEqual(r['by'], 'rules', r)  # it fell back to the rules, as it should
        self.assertLess(took, 45, f'the pick took {took:.0f} s')


@lab_test
class FirstStart(LabCase):
    @classmethod
    def setUpClass(cls):
        from tests.e2e.lab import Lab
        cls.lab = Lab(poll=3600).start(caps=False)  # a server that has just started for the first time

    @unittest.expectedFailure  # report bug 6: Auto says no machine is online until the first background probe
    def test_auto_works_right_after_the_server_starts(self):
        self.assertTrue(all(m['online'] for m in self.lab.sessions()['machines']))
        code, r = self.lab.post('/api/route', {'prompt': 'Train the classifier with CUDA'})
        self.assertEqual(code, 200, r)


@lab_test
class Dispatch(LabCase):
    def follow(self, did, timeout=40):
        seen, end = [], time.time() + timeout
        while time.time() < end:
            code, s = self.lab.get(f'/api/dispatch/{did}')
            self.assertEqual(code, 200, s)
            if not seen or seen[-1] != s['state']:
                seen.append(s['state'])
            if s['state'] in ('done', 'failed'):
                return dict(s, seen=seen)
            time.sleep(0.5)
        raise AssertionError(f'dispatch {did} did not end: {seen}')

    def sender(self):
        return self.lab.pick('studio', lambda s: s.get('kind') == 'background' and s.get('status') == 'busy')['sid']

    def test_send_work_to_a_named_machine_and_get_the_report(self):
        code, r = self.lab.post('/api/dispatch', {'from': {'machine': 'studio', 'sid': self.sender()}, 'machine': 'laptop',
                                                  'task': 'Run the parser tests\nand fix what fails', 'folder': 'parser'})
        self.assertEqual(code, 200, r)
        self.assertRegex(r['id'], r'^d-[0-9a-f]{12}$')
        self.assertEqual((r['machine'], r['label']), ('laptop', 'Laptop'))
        s = self.follow(r['id'])
        self.assertEqual(s['seen'][-3:], ['starting', 'working', 'done'], s)
        self.assertEqual(s['name'], 'Run the parser tests (for Studio)')
        self.assertIn('Built and ran it here.', s['report'])
        start = [x for x in self.lab.rpcs('start') if x['req'].get('name') == s['name']][0]
        self.assertEqual(start['target'], 'fake@laptop')
        self.assertTrue(start['req']['prompt'].startswith('Run the parser tests\nand fix what fails\n\nYou were sent this by a Claude Code session on Studio.'))
        self.assertEqual(self.lab.session('laptop', s['sid'])['title'], s['name'])
        self.assertEqual([p for p in self.lab.pushes() if s['name'] in p['body']], [])  # it was being followed: no push

    def test_send_work_to_auto(self):
        code, r = self.lab.post('/api/dispatch', {'from': {'machine': 'laptop'}, 'machine': 'auto',
                                                  'task': 'Train the classifier with CUDA and report the accuracy'})
        self.assertEqual(code, 200, r)
        self.assertEqual(r['machine'], 'tower')
        self.assertIn('CUDA', r['why'])
        s = self.follow(r['id'])
        self.assertEqual((s['state'], s['machine']), ('done', 'tower'), s)
        self.assertIn('CUDA', s['why'])

    def test_a_launch_that_fails_says_so(self):
        self.lab.flag('fail-start')
        try:
            code, r = self.lab.post('/api/dispatch', {'from': {'machine': 'studio'}, 'machine': 'tower', 'task': 'Anything'})
            self.assertEqual(code, 200, r)
            s = self.follow(r['id'])
        finally:
            self.lab.flag('fail-start', False)
        self.assertEqual(s['state'], 'failed', s)
        self.assertIn('Claude did not start there', s['error'])

    def test_bad_requests(self):
        ok = {'from': {'machine': 'studio'}, 'machine': 'laptop', 'task': 'x'}
        cases = [(dict(ok, **{'from': {'machine': 'nope'}}), 'from.machine'), (dict(ok, **{'from': {'machine': 'studio', 'sid': '../x'}}), 'from.sid'),
                 (dict(ok, task='   '), 'empty'), (dict(ok, task='x' * 18_001), 'too long'), (dict(ok, mode='turbo'), 'mode'),
                 (dict(ok, machine='nope'), 'unknown machine'), (dict(ok, folder=['a']), 'folder')]
        for body, words in cases:
            code, r = self.lab.post('/api/dispatch', body)
            self.assertEqual(code, 400, (body, r))
            self.assertIn(words, r['error'])
        self.assertEqual(self.lab.get('/api/dispatch/d-000000000000')[0], 404)
