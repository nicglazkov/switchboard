"""Start fresh and handoffs, end to end: Start fresh (POST /api/action verb fresh: a handoff, then a new chat seeded
with it, and the old one under Replaced), saving a handoff, and Continue on another machine (POST /api/continue: the
handoff is written on one machine and the new chat starts on another). Checks the jobs, what each machine was asked,
the lists, the thread that ties the chats together, and the pushes when nobody is watching."""
import time, unittest

from tests.e2e.lab import LabCase, lab_test


@lab_test
class StartFresh(LabCase):
    def closed_chat(self, machine='laptop', skip=()):
        """A chat that is not running (nothing to cut off), and that no other test used."""
        return self.lab.pick(machine, lambda s: s.get('kind') is None and s['sid'] not in skip and s['sid'] not in self.used)

    @classmethod
    def setUpClass(cls):
        super().setUpClass()
        cls.used = set()

    def act(self, verb, machine, sid, **kw):
        self.used.add(sid)
        code, r = self.lab.post('/api/action', dict({'machine': machine, 'verb': verb, 'sid': sid}, **kw))
        self.assertEqual(code, 200, r)
        return r

    def test_start_fresh_replaces_the_chat(self):
        old = self.closed_chat()
        j = self.lab.job(self.act('fresh', 'laptop', old['sid'])['job'])
        self.assertEqual((j['state'], j['kind']), ('done', 'fresh'), j)
        self.assertEqual(j['name'], f'{old["title"].split(" (")[0]} (fresh)')
        self.assertEqual([x['req']['sid'] for x in self.lab.rpcs('fresh', 'laptop')][-1], old['sid'])
        new = self.lab.wait(lambda: self.lab.session('laptop', j['sid']), 5, what='the fresh chat')
        self.assertEqual(new['title'], j['name'])
        gone = self.lab.session('laptop', old['sid'], replaced=True)
        self.assertEqual((gone['how'], gone['by']), ('fresh', {'machine': 'laptop', 'sid': j['sid']}))
        self.assertIsNone(self.lab.session('laptop', old['sid']))
        t = self.lab.get(f'/api/thread/laptop/{j["sid"]}')[1]
        self.assertEqual([(c['sid'], c['how']) for c in t['chats']][-2:], [(old['sid'], 'fresh'), (j['sid'], 'current')])

    def test_a_busy_chat_asks_first(self):
        s = self.lab.pick('studio', lambda s: s.get('kind') == 'background' and s.get('status') == 'busy')
        r = self.act('fresh', 'studio', s['sid'])
        self.assertEqual((r['ok'], r['need']), (False, 'busy'), r)
        self.assertIn('Start fresh when', r['error'])
        self.assertEqual([x for x in self.lab.rpcs('fresh', 'studio') if x['req']['sid'] == s['sid']], [])

    @unittest.expectedFailure  # report bug 8: "when they finish" when only Claude's reply is running
    def test_busy_copy_reads_right_for_a_reply(self):
        s = self.lab.pick('studio', lambda s: s.get('kind') == 'background' and s.get('status') == 'busy')
        r = self.act('fresh', 'studio', s['sid'])
        self.assertEqual(r['busy']['subagents'] + r['busy']['commands'], [])
        self.assertNotIn('when they finish', r['error'])

    def test_a_failed_fresh_start_keeps_the_old_chat_and_pushes(self):
        self.lab.flag('fast', False)  # a launch that ends within 6 s of the tap counts as watched: no push
        self.addCleanup(self.lab.flag, 'fast')
        old = self.closed_chat()
        self.lab.flag('fail-fresh')
        try:
            j = self.lab.job(self.act('fresh', 'laptop', old['sid'])['job'], watch=False)  # the app was closed
        finally:
            self.lab.flag('fail-fresh', False)
        self.assertEqual(j['state'], 'error', j)
        self.assertIsNotNone(self.lab.session('laptop', old['sid']))
        p = self.lab.wait(lambda: [p for p in self.lab.pushes() if p['title'] == 'Start fresh did not finish'], 5, what='the push')[-1]
        self.assertEqual(p['url'], f'/s/laptop/{old["sid"]}')
        self.assertIn('The old chat is untouched', p['body'])

    def test_a_fresh_start_nobody_watched_pushes_where_to_go(self):
        self.lab.flag('fast', False)  # a launch that ends within 6 s of the tap counts as watched: no push
        self.addCleanup(self.lab.flag, 'fast')
        old = self.closed_chat()
        j = self.lab.job(self.act('fresh', 'laptop', old['sid'])['job'], watch=False)
        self.assertEqual(j['state'], 'done', j)
        p = self.lab.wait(lambda: [p for p in self.lab.pushes() if p['title'] == 'Fresh session ready'], 5, what='the push')[-1]
        self.assertEqual(p['url'], f'/s/laptop/{j["sid"]}')
        self.assertTrue(p['body'].endswith('on Laptop. Tap to open it.'), p)

    def test_save_a_handoff(self):
        old = self.closed_chat('tower')
        j = self.lab.job(self.act('handoff', 'tower', old['sid'])['job'])
        self.assertEqual(j['state'], 'done', j)
        self.lab.wait(lambda: (self.lab.session('tower', old['sid']) or {}).get('handoff'), 5, what='the handoff to show')
        self.assertEqual([x['req']['close'] for x in self.lab.rpcs('handoff', 'tower') if x['req']['sid'] == old['sid']], [False])


@lab_test
class ContinueElsewhere(LabCase):
    def test_continue_on_another_machine(self):
        src = self.lab.pick('studio', lambda s: s.get('kind') is None)
        code, r = self.lab.post('/api/continue', {'machine': 'studio', 'sid': src['sid'], 'to': 'tower', 'folder': 'maps-api'})
        self.assertEqual(code, 200, r)
        j = self.lab.job(r['job'], timeout=90)
        self.assertEqual((j['state'], j['kind'], j['machine']), ('done', 'continue', 'tower'), j)
        self.assertIn(('running', 'handoff'), j['seen'])
        asked = [(x['target'], x['req']['verb']) for x in self.lab.rpcs() if x['req'].get('sid') == src['sid'] or
                 x['req'].get('topic') == src['title']]
        self.assertEqual([a for a in asked if a[1] in ('handoff', 'handoff_text', 'start', 'supersede')],
                         [('fake@studio', 'handoff'), ('fake@studio', 'handoff_text'), ('fake@tower', 'start'), ('fake@studio', 'supersede')])
        start = [x['req'] for x in self.lab.rpcs('start', 'tower') if x['req'].get('handoff_text') is not None][-1]
        self.assertEqual((start['folder'], start['from'], start['topic']), ('maps-api', 'Studio', src['title']))
        new = self.lab.wait(lambda: self.lab.session('tower', j['sid']), 5, what='the chat on the tower')
        self.assertEqual(new['title'], f'{src["title"]} (from Studio)')
        old = self.lab.wait(lambda: self.lab.session('studio', src['sid'], replaced=True), 5, what='the Studio chat under Replaced')
        self.assertEqual((old['how'], old['by']), ('continued', {'machine': 'tower', 'sid': j['sid']}))
        t = self.lab.get(f'/api/thread/tower/{j["sid"]}')[1]
        self.assertEqual([(c['machine'], c['sid']) for c in t['chats']][-2:], [('studio', src['sid']), ('tower', j['sid'])])

    def test_a_failed_continue_leaves_the_source(self):
        src = self.lab.pick('laptop', lambda s: s.get('kind') is None)
        self.lab.flag('fail-continue')
        try:
            code, r = self.lab.post('/api/continue', {'machine': 'laptop', 'sid': src['sid'], 'to': 'studio'})
            j = self.lab.job(r['job'], timeout=90)
        finally:
            self.lab.flag('fail-continue', False)
        self.assertEqual(j['state'], 'error', j)
        time.sleep(1)
        self.assertIsNotNone(self.lab.session('laptop', src['sid']))
        self.assertEqual([x for x in self.lab.rpcs('supersede') if x['req']['sid'] == src['sid']], [])

    def test_bad_continues(self):
        sid = self.lab.pick('studio', lambda s: True)['sid']
        for body in ({'machine': 'studio', 'sid': sid, 'to': 'nope'}, {'machine': 'studio', 'sid': 'x/../y', 'to': 'laptop'},
                     {'machine': 'studio', 'sid': sid, 'to': 'laptop', 'model': 'gpt'}):
            self.assertEqual(self.lab.post('/api/continue', body)[0], 400, body)
