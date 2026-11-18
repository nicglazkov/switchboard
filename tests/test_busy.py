"""Relaunch, stop, move and start fresh never cut off a chat's own work without asking: subagents, background
commands, a reply mid-way, results that have not reached the chat yet (2026-10-12)."""
import json, os, sys, tempfile, threading, time, unittest
from pathlib import Path
from tempfile import mkdtemp
from tests.helpers import fresh_sb, user, assistant, entry, write_transcript

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'server'))
import fleet, server  # noqa: E402

SID = 'aaaaaaaa-1111-2222-3333-444444444444'
NOW = 1_800_000_000


class BusyReportTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp()
        self.sb = fresh_sb(self.tmp)
        os.environ['SB_CLAUDE_DIR'] = str(Path(self.tmp) / 'claude')
        self.now = time.time()
        self.t = self.chat([user('2026-10-10T01:00:00Z', 'Run the agents.'),
                            dict(assistant('2026-10-10T01:00:05Z', 'Done.'), message={'role': 'assistant', 'content': [], 'stop_reason': 'end_turn'})])
        self.age(self.t, 3600)

    def tearDown(self):
        os.environ.pop('SB_CLAUDE_DIR', None)

    def chat(self, rows):
        return write_transcript(self.tmp, rows)

    def age(self, f, s):
        os.utime(f, (self.now - s, self.now - s))

    def sub(self, ident, age, desc=None):
        d = Path(self.tmp) / 'claude' / 'projects' / '-w' / SID / 'subagents'
        d.mkdir(parents=True, exist_ok=True)
        f = d / f'agent-{ident}.jsonl'
        f.write_text(json.dumps(user('2026-10-10T01:00:01Z', 'Look through the logs for errors')) + '\n')
        if desc:
            (d / f'agent-{ident}.meta.json').write_text(json.dumps({'description': desc}))
        self.age(f, age)
        return f

    def task(self, ident, age):
        d = Path(tempfile.gettempdir()) / 'claude-test-busy' / '-w' / SID / 'tasks'
        d.mkdir(parents=True, exist_ok=True)
        self.addCleanup(lambda: [p.unlink() for p in d.iterdir()])
        f = d / f'{ident}.output'
        f.write_text('building\n')
        self.age(f, age)
        return f

    def report(self, agents=(), procs=None):
        return self.sb.busy_report(SID, self.now, list(agents), procs, transcript=self.t)

    def test_a_background_jobs_commands_count_as_its_work(self):
        # Claude Code keeps a background session's command output under its job id (<bg_id>-...), not the session id
        d = Path(tempfile.gettempdir()) / 'claude-test-busy' / '-w' / 'c0ffee00-1111-4111-8111-111111111111' / 'tasks'
        d.mkdir(parents=True, exist_ok=True)
        self.addCleanup(lambda: [p.unlink() for p in d.iterdir()])
        f = d / 'b1.output'
        f.write_text('serving\n')
        self.age(f, 30)
        self.assertEqual(self.sb.children_active_ts(SID), 0)
        self.assertAlmostEqual(self.sb.children_active_ts(SID, 'c0ffee00'), self.now - 30, delta=2)
        self.assertEqual(self.sb.children_active_ts(SID, '../x'), 0)

    def test_a_quiet_chat_is_not_busy(self):
        self.sub('old1', 7200)
        r = self.report()
        self.assertFalse(r['live'])
        self.assertEqual(r['summary'], '')

    def test_running_subagents_and_commands_are_counted_and_named(self):
        self.sub('a1', 10, desc='Research PPF shops')
        self.sub('a2', 30)
        self.task('b7', 5)
        r = self.report()
        self.assertTrue(r['live'])
        self.assertEqual(sorted(r['subagents']), ['Look through the logs for errors', 'Research PPF shops'])
        self.assertEqual(r['commands'], ['background command'])
        self.assertEqual(r['summary'], '2 subagents and 1 background command are still running.')

    def test_a_subagents_task_output_is_not_counted_twice(self):
        self.sub('a1', 10)
        self.task('a1', 10)
        r = self.report()
        self.assertEqual((len(r['subagents']), r['commands']), (1, []))
        self.assertEqual(r['summary'], '1 subagent is still running.')

    def test_shells_under_the_chats_process_are_background_commands(self):
        agents = [{'sessionId': SID, 'kind': 'background', 'status': 'idle', 'pid': 500}]
        procs = {501: {'ppid': 500, 'name': '/bin/zsh'}, 502: {'ppid': 500, 'name': 'node'},  # node: an MCP server
                 503: {'ppid': 1, 'name': 'bash'}}
        r = self.report(agents, procs)
        self.assertEqual(r['commands'], ['background command'])

    def test_a_reply_in_progress(self):
        self.t = self.chat([user('2026-10-10T01:00:00Z', 'Fix it'),
                            dict(assistant('2026-10-10T01:00:05Z'), message={'role': 'assistant', 'content': [], 'stop_reason': 'tool_use'})])
        self.assertTrue(self.report()['replying'])
        self.assertEqual(self.report()['summary'], 'Claude is still replying.')
        self.age(self.t, 3600)  # an unfinished turn nobody has touched for an hour is a dead one
        self.assertFalse(self.report()['live'])
        busy = [{'sessionId': SID, 'kind': 'background', 'status': 'busy'}]
        self.assertTrue(self.report(busy)['replying'])

    def test_an_interrupted_turn_or_slash_command_is_not_a_reply(self):
        for text in ('[Request interrupted by user]', '<command-name>/model</command-name>'):
            self.t = self.chat([user('2026-10-10T01:00:00Z', text)])
            self.assertFalse(self.report()['replying'], text)
        self.t = self.chat([user('2026-10-10T01:00:00Z', 'Fix it')])
        self.assertTrue(self.report()['replying'])

    def test_a_finished_result_that_never_reached_the_chat_is_pending(self):
        f = self.sub('a9', 600, desc='Audit the hub')  # finished 10 min ago; the chat last wrote an hour ago
        r = self.report()
        self.assertEqual(r['pending'], ['Audit the hub'])
        self.assertEqual(r['summary'], '1 finished result has not reached the chat yet.')
        rows = [user('2026-10-10T01:00:00Z', 'go'),
                entry('user', '2026-10-10T02:00:00Z', message={'role': 'user', 'content': '<task-notification>\n<task-id>a9</task-id>\n<status>completed</status>'}),
                dict(assistant('2026-10-10T02:00:05Z', 'Read it.'), message={'role': 'assistant', 'content': [], 'stop_reason': 'end_turn'})]
        self.t = self.chat(rows)
        self.age(self.t, 3600)
        self.assertEqual(self.report()['pending'], [])  # delivered
        self.age(f, 7200)
        self.assertEqual(self.report()['pending'], [])  # finished before the chat's last write: it got the result

    def test_output_of_a_command_that_was_never_in_the_background_is_not_pending(self):
        # 2026-11-09, on a Windows PC: a foreground PowerShell call opened Chrome, Chrome kept writing USB errors to the call's
        # output file, and a relaunch waited on a "finished result" that no notification would ever bring
        self.task('b9cvebiye', 600)
        self.assertEqual(self.report()['pending'], [])
        self.t = self.chat([user('2026-10-10T01:00:00Z', 'go'),
                            entry('user', '2026-10-10T01:00:02Z', message={'role': 'user', 'content': [
                                {'type': 'tool_result', 'tool_use_id': 't1', 'content': 'Command running in background with ID: b9cvebiye.'}]}),
                            dict(assistant('2026-10-10T01:00:05Z', 'Started.'), message={'role': 'assistant', 'content': [], 'stop_reason': 'end_turn'})])
        self.age(self.t, 3600)
        self.assertEqual(self.report()['pending'], ['background command'], 'a real background command still counts')

    def test_summary_wording(self):
        s = self.sb.busy_summary
        self.assertEqual(s(['x'], [], True, []), '1 subagent is still running, and Claude is still replying.')
        self.assertEqual(s([], ['c', 'c'], False, ['p', 'q']),
                         '2 background commands are still running, and 2 finished results have not reached the chat yet.')
        self.assertEqual(s(['x'], ['c'], True, ['p']), '1 subagent and 1 background command are still running, Claude is '
                         'still replying, and 1 finished result has not reached the chat yet.')
        for t in (s(['x'], ['c'], True, ['p']),):
            self.assertNotIn('\u2014', t)

    def test_the_busy_verb(self):
        self.sub('a1', 10)
        r = self.sb.rpc({'verb': 'busy', 'sid': SID}, self.now, agents=[])
        self.assertTrue(r['ok'])
        self.assertEqual(r['subagents'], ['Look through the logs for errors'])


class StopIfIdleTest(unittest.TestCase):
    """The stop nobody asked for ("Stop idle chats"): the chat's own machine goes through with it only when nothing
    at all would be cut off at that moment."""

    chat, age, sub, tearDown = BusyReportTest.chat, BusyReportTest.age, BusyReportTest.sub, BusyReportTest.tearDown

    def setUp(self):
        BusyReportTest.setUp(self)
        d = Path(self.tmp) / 'claude' / 'projects' / '-w'
        d.mkdir(parents=True, exist_ok=True)
        self.t = str(d / f'{SID}.jsonl')
        self.rows = [user('2026-10-10T01:00:00Z', 'Run the agents.'),
                     dict(assistant('2026-10-10T01:00:05Z', 'Done.'), message={'role': 'assistant', 'content': [], 'stop_reason': 'end_turn'})]
        self.write(self.rows)
        self.stopped, self.procs = [], {500: {'ppid': 1, 'name': 'claude', 'age': 9 * 3600}}
        self.args = {500: ['claude', '--bg-worker']}  # what each process was started as
        self.sb.stop_bg = lambda bg: self.stopped.append(bg)
        self.sb.claude_procs = lambda: self.procs
        self.sb._cmdlines = lambda pids, loose=None: {p: a for p, a in self.args.items() if p in set(pids)}

    def write(self, rows):
        with open(self.t, 'w') as f:
            for r in rows:
                f.write(json.dumps(r, separators=(',', ':')) + '\n')
        self.age(self.t, 3600)

    def stop(self, status='idle', idle_s=1800, **kw):
        agents = [dict({'sessionId': SID, 'kind': 'background', 'status': status, 'pid': 500, 'id': 'aaaaaaaa'}, **kw)]
        req = {'verb': 'stop', 'sid': SID, 'if_idle': True}
        return self.sb.rpc(dict(req, idle_s=idle_s) if idle_s is not None else req, self.now, agents=agents)

    def tool(self, name, ts='2026-10-10T01:00:03Z'):
        return dict(assistant(ts), message={'role': 'assistant', 'stop_reason': 'tool_use',
                                            'content': [{'type': 'tool_use', 'id': 'tu1', 'name': name, 'input': {'cron': '0 9 * * *'}}]})

    def test_a_quiet_chat_stops(self):
        self.assertEqual(self.stop(), {'ok': True})
        self.assertEqual(self.stopped, ['aaaaaaaa'])
        self.assertIn('stop_if_idle', self.sb.AGENT_FEATURES)

    def test_anything_running_keeps_it(self):
        cases = [('busy', lambda: None, 'not idle'), ('waiting', lambda: None, 'not idle'),
                 ('idle', lambda: self.sub('a1', 10), 'subagent'),
                 ('idle', lambda: self.procs.update({501: {'ppid': 500, 'name': '/bin/zsh'}}), 'background command')]
        self.assertTrue(self.stop()['ok'])
        self.stopped.clear()
        for status, setup, word in cases:
            setup()
            r = self.stop(status)
            self.assertFalse(r['ok'], word)
            self.assertIn(word, r['error'])
        self.assertEqual(self.stopped, [])

    def test_a_scheduled_task_keeps_it_until_it_is_taken_back(self):
        self.write([self.rows[0], self.tool('CronCreate'), self.rows[1]])
        r = self.stop()
        self.assertEqual((r['ok'], r['error']), (False, 'It has a scheduled task.'))
        self.write([self.rows[0], self.tool('CronCreate'), self.tool('CronDelete'), self.rows[1]])
        self.assertTrue(self.stop()['ok'])
        # the words in a message are not a call
        self.write([user('2026-10-10T01:00:00Z', 'What does {"name":"CronCreate"} do?'), self.rows[1]])
        self.assertTrue(self.stop()['ok'])
        self.assertTrue(self.sb.has_schedule(self.t + '.gone'), 'a transcript that cannot be read is no proof')

    def test_what_it_cannot_see_keeps_it(self):
        def boom():
            raise OSError('ps failed')
        self.sb.claude_procs = boom
        self.assertFalse(self.stop()['ok'])
        self.sb.claude_procs = lambda: {}
        r = self.sb.rpc({'verb': 'stop', 'sid': SID, 'if_idle': True}, self.now, agents=[])
        self.assertEqual((r['ok'], r['error']), (False, 'It is not a running background chat.'))
        r = self.stop(kind='interactive')
        self.assertFalse(r['ok'])
        self.assertEqual(self.stopped, [])

    def test_a_chat_just_brought_up_is_left(self):
        # resuming a chat writes nothing to its transcript: only its process says it was just used
        self.procs[500]['age'] = 120
        r = self.stop()
        self.assertEqual((r['ok'], r['error']), (False, 'It was started or resumed a short while ago.'))
        self.procs[500]['age'] = 1801
        self.assertTrue(self.stop()['ok'])
        self.stopped.clear()
        for odd in (None, 'x', True):  # an age nobody could read is no proof
            self.procs[500]['age'] = odd
            self.assertFalse(self.stop()['ok'], odd)
        self.assertEqual(self.stopped, [])

    def test_the_wait_is_checked_on_this_machines_own_clock(self):
        # the server's clock (or this one) may be off: what counts is how long ago the transcript was written here
        self.assertTrue(self.stop(idle_s=3500)['ok'])
        self.stopped.clear()
        r = self.stop(idle_s=3700)
        self.assertEqual((r['ok'], r['error']), (False, 'It was used a short while ago.'))
        for odd in (None, 'soon', True, 0, -5):  # a wait that was not sent, or is no wait: two hours, the shortest there is
            self.assertFalse(self.stop(idle_s=odd)['ok'], odd)
        self.assertEqual(self.stopped, [])
        self.age(self.t, 3 * 3600)
        self.assertTrue(self.stop(idle_s=None)['ok'])

    def test_a_process_it_cannot_find_keeps_it(self):
        for procs in ({}, {7: {'ppid': 1, 'name': 'launchd', 'age': 99999}}):
            self.procs = procs
            r = self.stop()
            self.assertEqual((r['ok'], r['error']), (False, 'This machine could not see what is running in it.'), procs)
        self.procs = {500: {'ppid': 1, 'name': 'claude', 'age': 9 * 3600}}
        self.assertFalse(self.stop(pid=None)['ok'])
        self.assertEqual(self.stopped, [])

    def test_a_windows_shell_under_it_is_a_background_command(self):
        for name in ('bash.exe', 'C:\\Windows\\System32\\cmd.exe', 'PowerShell.exe', 'pwsh.exe', 'pwsh', '-zsh'):
            self.procs[501] = {'ppid': 500, 'name': name}
            r = self.stop()
            self.assertFalse(r['ok'], name)
            self.assertIn('background command', r['error'], name)
        self.procs[501] = {'ppid': 500, 'name': 'conhost.exe'}
        self.assertTrue(self.stop()['ok'])

    def test_someone_attached_to_it_keeps_it(self):
        self.procs[600] = {'ppid': 1, 'name': 'claude', 'age': 60}
        for args in (['/usr/local/bin/claude', 'attach', 'aaaaaaaa'], ['claude.exe', 'attach', 'aaaaaaaa'],
                     ['node', '/opt/claude-code/cli.js', 'attach', 'aaaaaaaa'], ['claude', 'attach', SID],
                     ['claude', '--debug', 'attach', 'aaaaaaaa']):
            self.args[600] = args
            r = self.stop()
            self.assertEqual((r['ok'], r['error']), (False, 'It is open in a terminal.'), args)
        r = self.stop(id='aaaaaaaabbbbcccc')  # a longer id, attached to by its short form
        self.assertEqual((r['ok'], r.get('error')), (False, 'It is open in a terminal.'))
        self.assertEqual(self.stopped, [])
        for args in (['claude', 'attach', 'bbbbbbbb'], ['claude', 'attach'], ['claude', 'logs', 'aaaaaaaa'],
                     ['claude', '-p', 'attach aaaaaaaa'], ['vim', 'attach', 'aaaaaaaa'], ['claude', 'attach', 'aaaaaaa']):
            self.args[600] = args
            self.assertTrue(self.stop()['ok'], args)

    def test_command_lines_it_cannot_read_keep_it(self):
        def boom(pids, loose=None):
            raise OSError('ps failed')
        for how in (boom, lambda pids, loose=None: {}):
            self.sb._cmdlines = how
            r = self.stop()
            self.assertEqual((r['ok'], r['error']), (False, 'This machine could not see what is running in it.'))
        self.procs[600] = {'ppid': 1, 'name': 'claude', 'age': 60}  # a claude process that will not say what it is
        self.sb._cmdlines = lambda pids, loose=None: {500: ['claude', '--bg-worker']}
        self.assertFalse(self.stop()['ok'])
        self.assertEqual(self.stopped, [])

    def test_a_chat_that_started_working_during_the_checks_is_left(self):
        asked = []

        def agents(raw):  # the list is read again just before the stop, and by then the chat has a message
            asked.append(1)
            return [dict(a, status='idle' if len(asked) == 1 else 'busy') for a in raw]
        self.sb.norm_agents = agents
        r = self.stop()
        self.assertEqual((r['ok'], r['error'], self.stopped), (False, 'It is not idle.', []))
        self.assertEqual(len(asked), 2)
        asked.clear()
        self.sb.norm_agents = lambda raw: (asked.append(1), [] if len(asked) > 1 else raw)[1]
        self.assertFalse(self.stop()['ok'], 'gone from the list')
        self.assertEqual(self.stopped, [])

    def test_a_plain_stop_is_as_before(self):
        self.sub('a1', 10)
        agents = [{'sessionId': SID, 'kind': 'background', 'status': 'busy', 'pid': 500, 'id': 'aaaaaaaa'}]
        self.assertEqual(self.sb.rpc({'verb': 'stop', 'sid': SID}, self.now, agents=agents), {'ok': True})
        self.assertEqual(self.stopped, ['aaaaaaaa'])


class BusyServerTest(unittest.TestCase):
    """The server asks before it cuts anything off, queues "when they finish", and goes at once on "now"."""

    def setUp(self):
        self.calls, self.busy = [], {'live': False}
        self.lock = threading.Lock()
        app = self

        def transport(target, req, timeout):
            with app.lock:
                app.calls.append(req)
            v = req['verb']
            if v == 'list':
                return {'ok': True, 'replaced': [], 'sessions': [{'sid': SID, 'title': 'Chat', 'mtime': NOW, 'active': NOW,
                                                                  'kind': 'background', 'status': 'idle', 'bg_id': 'aaaaaaaa'}], 'features': ['relaunch']}
            if v == 'busy':
                return dict(app.busy, ok=True)
            if v in ('resume', 'fresh', 'handoff', 'start'):
                return {'ok': True, 'job': '0123456789ab'}
            if v == 'handoff_text':
                return {'ok': True, 'text': 'NOTES'}
            if v == 'job':
                return {'ok': True, 'state': 'done', 'sid': 'bbbbbbbb-1111-2222-3333-444444444444', 'bg_id': 'bbbbbbbb'}
            return {'ok': True}
        machines = [{'name': 'studio', 'label': 'Studio', 'target': 'me@x', 'os': 'mac'},
                    {'name': 'desk', 'label': 'Desk', 'target': 'me@y', 'os': 'mac'}]
        self.fleet = fleet.Fleet(machines, transport, clock=lambda: NOW)
        self.app = server.App(self.fleet, state_dir=Path(mkdtemp()), web_dir=Path(mkdtemp()), cfg_dir=Path(mkdtemp()),
                              clock=lambda: NOW, sleep=lambda s: time.sleep(0.005), poll_every=0)
        self.app._refresh_soon = lambda *a, **k: None
        self.fleet.poll()

    def working(self):
        self.busy = {'live': True, 'subagents': ['Audit', 'Logs'], 'commands': ['background command'], 'replying': False,
                     'pending': [], 'summary': '2 subagents and 1 background command are still running.'}

    def verbs(self, v):
        with self.lock:
            return [c for c in self.calls if c['verb'] == v]

    def wait(self, jid, states=('done', 'error')):
        for _ in range(400):
            j = self.app.job('server', jid)
            if j['state'] in states:
                return j
            time.sleep(0.005)
        self.fail(f'job stuck: {j}')

    def relaunch(self, **kw):
        return self.app.post('/api/relaunch', dict({'machine': 'studio', 'sid': SID, 'model': 'sonnet', 'effort': 'high',
                                                    'flags': [['--chrome']]}, **kw))

    def test_relaunch_asks_and_says_what_is_running(self):
        self.working()
        r = self.relaunch()
        self.assertEqual((r['ok'], r['need']), (False, 'busy'))
        self.assertEqual(r['busy']['subagents'], ['Audit', 'Logs'])
        self.assertTrue(r['error'].startswith('2 subagents and 1 background command are still running.'))
        self.assertEqual(self.verbs('resume'), [], 'nothing is stopped without the second step')

    def test_relaunch_when_they_finish_waits_then_goes(self):
        self.working()
        r = self.relaunch(when='idle')
        j = self.wait(r['job'], ('queued',))
        for _ in range(200):
            j = self.app.job('server', r['job'])
            if j.get('step') == 'waiting':
                break
            time.sleep(0.005)
        self.assertEqual((j['step'], j['note']), ('waiting', '2 subagents and 1 background command are still running.'))
        self.assertEqual(self.verbs('resume'), [])
        self.busy = {'live': False}
        self.assertEqual(self.wait(r['job'])['state'], 'done')
        self.assertEqual(len(self.verbs('resume')), 1)

    def test_a_queued_relaunch_can_be_cancelled(self):
        self.working()
        r = self.relaunch(when='idle')
        for _ in range(200):
            if self.app.job('server', r['job']).get('step') == 'waiting':
                break
            time.sleep(0.005)
        self.assertTrue(self.app.post('/api/job/cancel', {'job': r['job']})['ok'])
        j = self.wait(r['job'])
        self.assertEqual(j['state'], 'error')
        self.assertIn('Cancelled', j['error'])
        self.assertEqual(self.verbs('resume'), [])

    def test_a_cancel_after_the_wait_ended_is_too_late(self):
        # The chat went quiet and the job went ahead, but its step still says "waiting": the cancel is refused,
        # so the app is never told "cancelled" for a relaunch that then runs.
        self.working()
        gate, reached = threading.Event(), threading.Event()
        go = self.fleet.transport

        def held(target, req, timeout):
            if req['verb'] == 'resume':
                reached.set()
                gate.wait(5)
            return go(target, req, timeout)
        self.fleet.transport = held
        r = self.relaunch(when='idle')
        for _ in range(200):
            if self.app.job('server', r['job']).get('step') == 'waiting':
                break
            time.sleep(0.005)
        self.busy = {'live': False}
        self.assertTrue(reached.wait(5))
        self.assertEqual(self.app.job('server', r['job']).get('step'), 'waiting')
        with self.assertRaises(server.HttpError) as e:
            self.app.post('/api/job/cancel', {'job': r['job']})
        self.assertEqual(e.exception.code, 409)
        gate.set()
        self.assertEqual(self.wait(r['job'])['state'], 'done')
        self.assertEqual(len(self.verbs('resume')), 1)

    def test_relaunch_now_goes_at_once(self):
        self.working()
        r = self.relaunch(when='now')
        self.assertEqual(self.wait(r['job'])['state'], 'done')
        self.assertEqual(self.verbs('busy'), [], 'the user already chose')

    def test_stop_fresh_and_close_ask_first(self):
        self.working()
        for body in ({'verb': 'stop'}, {'verb': 'fresh'}, {'verb': 'handoff', 'close': True}):
            r = self.app.post('/api/action', dict(body, machine='studio', sid=SID))
            self.assertEqual(r.get('need'), 'busy', body)
        self.assertEqual(self.verbs('stop') + self.verbs('fresh') + self.verbs('handoff'), [])
        r = self.app.post('/api/action', {'machine': 'studio', 'sid': SID, 'verb': 'handoff'})  # no close: harmless
        self.assertTrue(r['ok'])

    def test_stop_when_they_finish_is_queued(self):
        self.working()
        r = self.app.post('/api/action', {'machine': 'studio', 'sid': SID, 'verb': 'stop', 'when': 'idle'})
        self.assertTrue(r['job'].startswith('s-'))
        time.sleep(0.05)
        self.assertEqual(self.verbs('stop'), [])
        self.busy = {'live': False}
        self.assertEqual(self.wait(r['job'])['state'], 'done')
        self.assertEqual(len(self.verbs('stop')), 1)

    def test_a_quiet_chat_stops_without_asking(self):
        r = self.app.post('/api/action', {'machine': 'studio', 'sid': SID, 'verb': 'stop'})
        self.assertTrue(r['ok'])
        self.assertEqual(len(self.verbs('stop')), 1)

    def test_moving_a_busy_chat_asks(self):
        self.working()
        r = self.app.post('/api/continue', {'machine': 'studio', 'sid': SID, 'to': 'desk'})
        self.assertEqual(r['need'], 'busy')
        self.assertIn('Move it when they finish', r['error'])

    def test_cleanup_stop_asks_for_busy_chats_and_queues_them(self):
        self.working()
        r = self.app.post('/api/cleanup', {'action': 'stop', 'items': [{'machine': 'studio', 'sid': SID}]})
        self.assertEqual(r['need'], 'busy')
        self.assertEqual(r['busy'][0]['sid'], SID)
        self.assertEqual(self.verbs('stop'), [])
        r = self.app.post('/api/cleanup', {'action': 'stop', 'when': 'idle', 'items': [{'machine': 'studio', 'sid': SID}]})
        self.assertEqual(len(r['queued']), 1)
        self.busy = {'live': False}
        self.assertEqual(self.wait(r['queued'][0]['job'])['state'], 'done')
        self.assertEqual(len(self.verbs('stop')), 1)

    def test_an_older_machine_without_the_busy_verb_falls_back_to_the_list(self):
        self.busy = {}  # answers {'ok': True} with no report
        r = self.relaunch()
        self.assertTrue(r['ok'])


if __name__ == '__main__':
    unittest.main()
