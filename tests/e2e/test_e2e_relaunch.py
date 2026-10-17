"""Relaunch with flags, end to end: the flags drawer (GET and POST /api/flags), POST /api/relaunch from the desktop,
the iPhone's form (POST /api/action verb relaunch, which sends only what changes), waiting for a chat's own work to
end first, cancelling that wait, and chats open in a terminal. A relaunch is claude --bg --resume <sid> <flags>: the
chat gets a new id and the old one goes under Replaced (scripts/lab/fakefleet.py does the same)."""
import time, unittest

from tests.e2e.lab import LabCase, lab_test


@lab_test
class Relaunch(LabCase):
    def new_chat(self, machine='laptop', prompt='Write the parser tests'):
        code, r = self.lab.post('/api/new', {'machine': machine, 'prompt': prompt, 'folder': 'parser', 'create': True})
        self.assertEqual(code, 200, r)
        j = self.lab.job(r['job'])
        self.assertEqual(j['state'], 'done', j)
        self.lab.wait(lambda: (self.lab.session(machine, j['sid']) or {}).get('status') == 'idle', 15, what='Claude to reply')
        return j['sid']

    def relaunch(self, body, machine='laptop'):
        code, r = self.lab.post('/api/relaunch', dict({'machine': machine}, **body))
        self.assertEqual(code, 200, r)
        return r

    def test_relaunch_with_new_flags(self):
        sid = self.new_chat()
        code, v = self.lab.get(f'/api/flags?machine=laptop&sid={sid}')
        self.assertEqual(code, 200, v)
        self.assertTrue(v['running'])
        code, plan = self.lab.post('/api/flags', {'machine': 'laptop', 'sid': sid, 'want': {'perm': 'acceptEdits', 'flags': [['--add-dir', 'notes']]}})
        self.assertEqual(code, 200, plan)
        self.assertTrue(plan['relaunch'])
        self.assertEqual({c['name']: c['how'] for c in plan['changes']}, {'--permission-mode': 'relaunch', '--add-dir': 'live'})

        r = self.relaunch(dict(plan['body'], sid=sid, model='sonnet', effort='high'))
        j = self.lab.job(r['job'])
        self.assertEqual((j['state'], j['kind']), ('done', 'relaunch'), j)
        rpc = [x for x in self.lab.rpcs('resume', 'laptop') if x['req']['sid'] == sid][-1]['req']
        self.assertTrue(rpc['relaunch'])
        self.assertEqual(sorted(rpc['flags']), [['--add-dir', 'notes'], ['--permission-mode', 'acceptEdits']])
        self.assertEqual((rpc['model'], rpc['effort']), ('sonnet', 'high'))

        new = j['sid']
        self.assertNotEqual(new, sid)
        self.lab.wait(lambda: self.lab.session('laptop', new), 5, what='the relaunched chat')
        old = self.lab.session('laptop', sid, replaced=True)
        self.assertEqual((old['how'], old['by']), ('resumed', {'machine': 'laptop', 'sid': new}))
        self.assertIsNone(self.lab.session('laptop', sid))
        code, v = self.lab.get(f'/api/flags?machine=laptop&sid={new}')
        self.assertEqual({x['name']: x['value'] for x in v['active']},
                         {'--model': 'sonnet', '--effort': 'high', '--permission-mode': 'acceptEdits', '--add-dir': 'notes', '--remote-control': None})
        code, t = self.lab.get(f'/api/thread/laptop/{new}')
        self.assertEqual([c['sid'] for c in t['chats']], [sid, new])  # one thread: the history follows the chat
        self.assertEqual([p for p in self.lab.pushes() if p['title'].startswith('Relaunch')], [])  # the app was watching

    def test_the_iphone_sends_only_what_changes(self):
        sid = self.new_chat(prompt='Clean up the notes')
        j = self.lab.job(self.relaunch({'sid': sid, 'flags': [['--add-dir', 'notes']], 'model': 'sonnet'})['job'])
        mid = j['sid']
        self.lab.wait(lambda: self.lab.session('laptop', mid), 5, what='the first relaunch')
        code, r = self.lab.post('/api/action', {'machine': 'laptop', 'verb': 'relaunch', 'sid': mid, 'flags': {'perm': ['plan']}},
                                ua='SwitchboardApp/3.0')
        self.assertEqual(code, 200, r)
        j = self.lab.job(r['job'])
        self.assertEqual(j['state'], 'done', j)
        rpc = [x for x in self.lab.rpcs('resume', 'laptop') if x['req']['sid'] == mid][-1]['req']
        self.assertIn(['--add-dir', 'notes'], rpc['flags'])  # kept from before
        self.assertIn(['--permission-mode', 'plan'], rpc['flags'])  # the change
        self.assertEqual(rpc['model'], 'sonnet')  # kept from what it runs with now
        code, r = self.lab.post('/api/action', {'machine': 'laptop', 'verb': 'relaunch', 'sid': j['sid'], 'flags': {'turbo': ['on']}})
        self.assertEqual(code, 400, r)

    def test_a_busy_chat_asks_then_waits_for_its_work(self):
        sid = self.new_chat(prompt='Run the long suite')
        self.lab.flag(f'work-{sid}')
        try:
            code, r = self.lab.post('/api/relaunch', {'machine': 'laptop', 'sid': sid, 'flags': [['--permission-mode', 'plan']]})
            self.assertEqual(code, 200, r)
            self.assertEqual((r['ok'], r['need']), (False, 'busy'), r)
            self.assertEqual(r['busy']['subagents'], ['general-purpose: Run the test suite'])
            self.assertIn('Relaunch when they finish, or now', r['error'])
            jid = self.relaunch({'sid': sid, 'flags': [['--permission-mode', 'plan']], 'when': 'idle'})['job']
            j = self.lab.wait(lambda: (lambda j: j if j.get('step') == 'waiting' else None)(self.lab.get(f'/api/job/server/{jid}')[1]), 10,
                              what='the job to wait')
            self.assertEqual(j['note'], 'A subagent is still running.')
            time.sleep(4)
            self.assertEqual([x for x in self.lab.rpcs('resume', 'laptop') if x['req']['sid'] == sid], [])  # nothing cut off
        finally:
            self.lab.flag(f'work-{sid}', False)
        j = self.lab.job(jid)
        self.assertEqual(j['state'], 'done', j)
        self.assertEqual(len([x for x in self.lab.rpcs('resume', 'laptop') if x['req']['sid'] == sid]), 1)

    def test_a_wait_can_be_called_off(self):
        sid = self.new_chat(prompt='Run another long suite')
        self.lab.flag(f'work-{sid}')
        try:
            jid = self.relaunch({'sid': sid, 'flags': [], 'model': 'haiku', 'when': 'idle'})['job']
            self.lab.wait(lambda: self.lab.get(f'/api/job/server/{jid}')[1].get('step') == 'waiting', 10, what='the job to wait')
            code, r = self.lab.post('/api/job/cancel', {'job': jid})
            self.assertEqual(code, 200, r)
            j = self.lab.job(jid)
        finally:
            self.lab.flag(f'work-{sid}', False)
        self.assertEqual((j['state'], j['error']), ('error', 'Cancelled. Nothing was changed.'))
        self.assertEqual([x for x in self.lab.rpcs('resume', 'laptop') if x['req']['sid'] == sid], [])
        self.assertIsNotNone(self.lab.session('laptop', sid))
        self.assertEqual(self.lab.post('/api/job/cancel', {'job': jid})[0], 409)  # only a waiting job can be called off

    def test_now_cuts_in_at_once(self):
        sid = self.new_chat(prompt='Run the slow build')
        self.lab.flag(f'work-{sid}')
        try:
            j = self.lab.job(self.relaunch({'sid': sid, 'flags': [], 'effort': 'max', 'when': 'now'})['job'])
        finally:
            self.lab.flag(f'work-{sid}', False)
        self.assertEqual(j['state'], 'done', j)
        self.assertNotIn(('queued', 'waiting'), j['seen'])

    def test_a_chat_open_in_a_terminal_gets_the_command_to_run(self):
        s = self.lab.pick('studio', lambda s: s.get('kind') == 'interactive' and s.get('status') == 'idle')
        code, r = self.lab.post('/api/relaunch', {'machine': 'studio', 'sid': s['sid'], 'flags': [['--permission-mode', 'plan']], 'model': 'sonnet'})
        self.assertEqual(code, 200, r)
        self.assertEqual((r['ok'], r['need']), (False, 'terminal'))
        self.assertEqual(r['command'], f'claude --resume {s["sid"]} --model sonnet --permission-mode plan')
        self.assertIn('Exit Claude there (type /exit)', r['error'])

    def test_a_failed_relaunch_leaves_the_chat_alone(self):
        sid = self.new_chat(prompt='Check the logs')
        self.lab.flag('fail-resume')
        try:
            j = self.lab.job(self.relaunch({'sid': sid, 'flags': [], 'model': 'opus'})['job'])
        finally:
            self.lab.flag('fail-resume', False)
        self.assertEqual(j['state'], 'error', j)
        self.assertIn('The old chat is untouched', j['error'])
        self.assertIsNotNone(self.lab.session('laptop', sid))

    def test_bad_requests(self):
        sid = self.lab.pick('laptop', lambda s: True)['sid']
        for body, code in (({'sid': sid, 'other': '--bogus-flag'}, 400), ({'sid': sid, 'model': 'gpt'}, 400), ({'sid': '../../x'}, 400),
                           ({'sid': '00000000-0000-0000-0000-000000000000'}, 404), ({'machine': 'nope', 'sid': sid}, 400)):
            got, r = self.lab.post('/api/relaunch', dict({'machine': 'laptop'}, **body))
            self.assertEqual(got, code, (body, r))


@lab_test
class RelaunchOnAnOldAgent(LabCase):
    FLAGS = ('fast', 'old-agent')

    @unittest.expectedFailure  # report bug 5: the server never checks the agent has the relaunch verb
    def test_an_agent_without_relaunch_is_asked_to_update(self):
        s = self.lab.pick('studio', lambda s: s.get('kind') == 'background' and s.get('status') == 'stopped')
        code, r = self.lab.post('/api/relaunch', {'machine': 'studio', 'sid': s['sid'], 'flags': [['--permission-mode', 'plan']]})
        self.assertIn(code, (400, 409), r)
        self.assertIn('Update Switchboard', r.get('error') or '')
