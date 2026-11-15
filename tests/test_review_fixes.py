"""Plan 2 final review: one test per Critical/Important finding."""
import datetime as dt, json, os, sys, tempfile, threading, time, unittest
from pathlib import Path
from unittest import mock
from tests.helpers import fresh_sb, write_transcript, user, assistant
from tests.test_rpc import NOW, SID_A
from tests.test_jobs import JobBase

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'server'))
import fleet, server  # noqa: E402
from tests.test_server import FakeTransport, MACH  # noqa: E402

BIG = {'input_tokens': 1, 'cache_creation_input_tokens': 0, 'cache_read_input_tokens': 640_000, 'output_tokens': 1}


class HookBase(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(); self.sb = fresh_sb(self.tmp)
        self.t = write_transcript(self.tmp, [user('2026-10-10T01:00:00Z', 'Remember X.'), assistant('2026-10-10T01:00:05Z', 'ok', usage=BIG)])
        self.now = dt.datetime(2026, 10, 10, 10, tzinfo=dt.timezone.utc).timestamp()
        self.spawned, self.notified = [], []

    def hook(self, inp, pid=777, now=None):
        return self.sb.handle_hook(inp, now or self.now, pid, spawn=lambda *a: self.spawned.append(a), notify=self.notified.append)

    def ups(self, prompt, sid='S1'):
        return {'hook_event_name': 'UserPromptSubmit', 'session_id': sid, 'transcript_path': self.t, 'prompt': prompt, 'cwd': '/w'}


# 1. Guard events from the hooks must get through the server's POST check.
class NotifyReachesServerTest(unittest.TestCase):
    def test_hook_guard_event_is_accepted_by_the_real_server(self):
        sb = fresh_sb(tempfile.mkdtemp())
        app = server.App(fleet.Fleet(MACH, FakeTransport()), state_dir=Path(tempfile.mkdtemp()),
                         web_dir=Path(__file__).resolve().parents[1] / 'server' / 'web', pusher=lambda *a: None, guard_delay=30)
        httpd = server.make_server('127.0.0.1', 0, app)
        threading.Thread(target=httpd.serve_forever, daemon=True).start()
        try:
            with mock.patch.object(sb, 'SERVERS', (f'http://127.0.0.1:{httpd.server_address[1]}',)):
                sb.notify_server({'event': 'guard', 'machine': 'desk', 'session_id': SID_A, 'ctx': 640_000,
                                  'idle': 3600, 'title': 't', 'cwd': '/w', 'ts': NOW})
                self.assertIn(('desk', SID_A), app.guard.pending)       # the push timer is running
                sb.notify_server({'event': 'guard_answer', 'machine': 'desk', 'session_id': SID_A, 'ts': NOW})
                self.assertNotIn(('desk', SID_A), app.guard.pending)    # and the answer cancelled it
        finally:
            httpd.shutdown()


# 2. A job reported as failed must never run later.
class FinalJobStateTest(JobBase):
    def test_mac_job_reported_failed_never_launches(self):
        with mock.patch.object(self.sb, 'IS_WIN', False):
            jid = self.job({'verb': 'start', 'folder': 'proj', 'prompt': 'build it'})
            self.assertEqual(self.sb.rpc({'verb': 'job', 'id': jid}, NOW + 90, agents=[])['state'], 'error')
            self.drain()   # the launcher shows up after the user opened a session, as the error said to
            self.assertEqual(self.launched, [])
            self.assertEqual(self.sb.rpc({'verb': 'job', 'id': jid}, NOW + 95, agents=[])['state'], 'error')

    def test_windows_job_reported_failed_never_launches(self):
        with mock.patch.object(self.sb, 'IS_WIN', True):
            jid = self.job({'verb': 'start', 'folder': 'proj'})
            self.assertEqual(self.sb.rpc({'verb': 'job', 'id': jid}, NOW + 700, agents=[])['state'], 'error')
        self.drain()
        self.assertEqual(self.launched, [])

    def test_launcher_skips_jobs_older_than_the_queue_ttl(self):
        jid = self.job({'verb': 'start', 'folder': 'proj'})
        self.sb.launch_queue(runner=self.runner(), find_url=lambda bg: None, make_handoff=lambda *a: None,
                             now=NOW + self.sb.JOB_QUEUED_TTL + 1, live=lambda bg: 'ready')
        self.assertEqual(self.launched, [])
        self.assertEqual(self.sb.rpc({'verb': 'job', 'id': jid}, NOW + 700, agents=[])['state'], 'error')

    def test_launcher_trigger_failure_leaves_no_job_behind(self):
        def boom():
            raise OSError('schtasks timed out')
        self.sb.trigger_launcher = boom
        r = self.sb.rpc({'verb': 'start', 'folder': 'proj'}, NOW, agents=[])
        self.assertFalse(r['ok'])
        self.drain()
        self.assertEqual(self.launched, [])

    def test_job_claimed_by_the_launcher_is_not_expired(self):
        with mock.patch.object(self.sb, 'IS_WIN', False):
            jid = self.job({'verb': 'start', 'folder': 'proj'})
            seen = []
            def run(argv, cwd, env):
                seen.append(self.sb.rpc({'verb': 'job', 'id': jid}, NOW + 90, agents=[])['state'])
                return 0, '  claude stop 3022c163    stop this session\n'
            self.drain(runner=run)
        self.assertEqual(seen, ['running'])


# 3. A long handoff handed to a fresh session must point at a file that still exists.
class SeedPathTest(HookBase):
    def test_long_seeded_handoff_points_at_a_readable_file(self):
        text = '# Handoff\n' + 'detail line\n' * 2000 + '## Your instructions, verbatim\n- KEEP-THIS-RULE\n'
        seed_id, _ = self.sb._seed(text, None, self.now)
        out = self.hook(self.ups(f'[switchboard:{seed_id}] go on'))
        ctx = out['hookSpecificOutput']['additionalContext']
        path = ctx.split('Read the whole file ')[1].split(' (especially')[0]
        self.assertIn('KEEP-THIS-RULE', Path(path).read_text())
        again = self.hook(self.ups(f'[switchboard:{seed_id}] go on')) or {}
        self.assertNotIn('KEEP-THIS-RULE', json.dumps(again))   # still single use

    def test_used_seeds_are_cleaned_up_after_a_week(self):
        old_id, _ = self.sb._seed('old', None, self.now)
        self.hook(self.ups(f'[switchboard:{old_id}] x'))
        used = list(self.sb.paths_q()['seeds'].glob(f'{old_id}*'))
        self.assertEqual(len(used), 1)
        os.utime(used[0], (time.time() - 8 * 86400,) * 2)
        new_id, _ = self.sb._seed('new', None, self.now)
        self.hook(self.ups(f'[switchboard:{new_id}] x'))
        self.assertFalse(used[0].exists())


# 4. "Send anyway" must not leave a pass that disables the guard later.
class PassTest(HookBase):
    def test_resend_after_send_anyway_uses_up_the_pass(self):
        self.assertEqual(self.hook(self.ups('fix the lights'))['decision'], 'block')
        (self.sb.paths()['state'] / 'S1.pass').write_text(str(self.now + 10))   # rpc pass from the app
        self.assertIsNone(self.hook(self.ups('fix the lights'), now=self.now + 30))
        later = self.hook(self.ups('next big question'), now=self.now + 3 * 86400)
        self.assertEqual(later['decision'], 'block')

    def test_go_after_send_anyway_uses_up_the_pass(self):
        self.hook(self.ups('fix the lights'))
        (self.sb.paths()['state'] / 'S1.pass').write_text(str(self.now + 10))
        self.hook(self.ups('go'), now=self.now + 30)
        self.assertEqual(self.hook(self.ups('another'), now=self.now + 3 * 86400)['decision'], 'block')

    def test_an_old_pass_does_not_wave_a_prompt_through(self):
        (self.sb.paths()['state'] / 'S1.pass').write_text(str(self.now))
        out = self.hook(self.ups('new question'), now=self.now + self.sb.PENDING_TTL + 60)
        self.assertEqual(out['decision'], 'block')

    def test_a_fresh_pass_still_lets_the_next_prompt_through(self):
        (self.sb.paths()['state'] / 'S1.pass').write_text(str(self.now))
        self.assertIsNone(self.hook(self.ups('new question'), now=self.now + 120))


# 5. A held prompt is delivered once.
class HeldOnceTest(HookBase):
    def test_clear_consumes_the_held_prompt(self):
        self.hook(self.ups('fix the lights'))
        self.sb.handoff_path('S1').write_text('HANDOFF')
        self.hook({'hook_event_name': 'SessionStart', 'source': 'clear', 'session_id': 'S2', 'transcript_path': '/n'}, now=self.now + 60)
        self.assertIsNone(self.sb.held_get('S1'))

    def test_fresh_keyword_carries_the_held_prompt_and_cancels_the_push(self):
        self.hook(self.ups('fix the lights'))
        self.hook(self.ups('fresh'), now=self.now + 20)
        self.assertEqual(self.notified[-1]['event'], 'guard_answer')
        self.sb.handoff_path('S1').write_text('HANDOFF')
        out = self.hook({'hook_event_name': 'SessionStart', 'source': 'clear', 'session_id': 'S2', 'transcript_path': '/n'}, now=self.now + 60)
        self.assertIn('fix the lights', out['hookSpecificOutput']['additionalContext'])


class HeldOnceJobTest(JobBase):
    def setUp(self):
        super().setUp()
        self.t = self.transcript(SID_A, str(self.root / 'proj'), NOW - 60)
        self.sb.rpc({'verb': 'list'}, NOW, agents=[])

    def test_fresh_ignores_a_stale_held_prompt(self):
        self.sb.held_set(SID_A, 'already handled long ago', NOW - self.sb.PENDING_TTL - 100)
        self.job({'verb': 'fresh', 'sid': SID_A})
        self.drain()
        self.assertNotIn('already handled', self.launched[0]['argv'][-1])

    def test_app_fresh_cancels_the_terminal_clear_injection(self):
        self.sb.pending_put(777, {'session_id': SID_A, 'transcript_path': self.t, 'held': 'fix the lights', 'ts': NOW})
        self.sb.held_set(SID_A, 'fix the lights', NOW)
        self.job({'verb': 'fresh', 'sid': SID_A})
        self.drain()
        self.assertIn('fix the lights', self.launched[0]['argv'][-1])
        out = self.sb.handle_hook({'hook_event_name': 'SessionStart', 'source': 'clear', 'session_id': 'S2',
                                   'transcript_path': '/n'}, NOW + 60, 777, notify=lambda e: None)
        self.assertIsNone(out)


# 6. The launcher heartbeat stays fresh during a long job, and only one launcher runs.
class LauncherBeatTest(JobBase):
    def test_beat_keeps_going_while_a_job_runs(self):
        me = self.tmp / 'sb_copy.py'; me.write_text('x')
        beat = self.sb.paths_q()['queue'] / '.launcher'
        seen = []
        def long_job():
            seen.append(self.sb._jget(beat)['ts'])
            time.sleep(0.35)
            seen.append(self.sb._jget(beat)['ts'])
        with mock.patch.object(self.sb, 'LAUNCHER_BEAT', 0.05), mock.patch.object(self.sb, 'launch_queue', long_job):
            self.sb.launcher_daemon(me=me, sleep=lambda _: os.utime(me, (1, 1)))
        self.assertGreater(seen[1], seen[0] + 0.15)

    def test_launcher_yields_to_another_live_launcher(self):
        me = self.tmp / 'sb_copy.py'; me.write_text('x')
        beat = self.sb.paths_q()['queue'] / '.launcher'
        drains, ticks = [], []
        def sleep(_):
            self.sb._jput(beat, {'pid': os.getppid(), 'ts': time.time()})   # another launcher won the race
            ticks.append(1)
            if len(ticks) >= 5:
                os.utime(me, (1, 1))   # bound the test: a deploy ends any launcher
        with mock.patch.object(self.sb, 'launch_queue', lambda: drains.append(1)):
            self.sb.launcher_daemon(me=me, sleep=sleep)
        self.assertEqual(len(drains), 1)
        self.assertEqual(self.sb._jget(beat)['pid'], os.getppid())

    def test_empty_queue_lock_from_a_racing_launcher_counts_as_held(self):
        self.job({'verb': 'start', 'folder': 'proj'})
        (self.sb.paths_q()['queue'] / '.lock').write_text('')   # created, timestamp not written yet
        self.drain()
        self.assertEqual(self.launched, [])


# 7. Bookkeeping lines after a handoff must not make it stale.
class HandoffFreshnessTest(JobBase):
    def test_bookkeeping_after_the_handoff_keeps_it_fresh(self):
        t = Path(self.transcript(SID_A, str(self.root / 'proj'), NOW - 60))
        self.sb.handoff_path(SID_A).write_text('HANDOFF')
        old = time.time() - 30
        os.utime(self.sb.handoff_path(SID_A), (old, old))
        with open(t, 'a') as f:
            f.write(json.dumps({'type': 'last-prompt', 'sessionId': SID_A}) + '\n')
        self.assertTrue(self.sb.handoff_is_fresh(SID_A, str(t)))
        later = dt.datetime.fromtimestamp(time.time() + 5, dt.timezone.utc).strftime('%Y-%m-%dT%H:%M:%S.000Z')
        with open(t, 'a') as f:
            f.write(json.dumps(assistant(later, 'new reply')) + '\n')
        self.assertFalse(self.sb.handoff_is_fresh(SID_A, str(t)))


class SmallHelperFixesTest(JobBase):
    """0.8.20 review, the helper's small ones."""
    def test_a_send_nobody_takes_is_over_as_soon_as_all_said_no(self):
        def refused(base):
            raise OSError('connection refused')
        for stagger in (None, 0.05):
            t0 = time.time()
            self.assertFalse(self.sb._post_any({'event': 'x'}, 5, ['http://127.0.0.1:9', 'http://127.0.0.1:10'], send=refused,
                                               stagger=stagger))
            self.assertLess(time.time() - t0, 0.3, stagger)
        t0 = time.time()
        self.assertFalse(self.sb._post_any({'event': 'x'}, 5, []))
        self.assertLess(time.time() - t0, 0.3)
        # one that takes it still wins, however the other answers
        self.assertTrue(self.sb._post_any({'event': 'x'}, 5, ['a', 'b'], send=lambda b: refused(b) if b == 'a' else None))
        self.assertTrue(self.sb._post_any({'event': 'x'}, 5, ['a', 'b'], send=lambda b: refused(b) if b == 'b' else None, stagger=0.05))

    def test_a_claim_file_that_is_not_text_is_stale_not_a_crash(self):
        lock = self.sb.paths()['state'] / 'x.lock'
        lock.write_bytes(b'\xff\xfe\x00garbage\x80')
        os.utime(lock, (NOW - 9999, NOW - 9999))
        self.assertEqual(self.sb._lock_read(lock)['pid'], None)
        self.assertTrue(self.sb.lock_take(lock, NOW, 60))
        self.assertEqual(json.loads(lock.read_text())['pid'], os.getpid())

    def test_claude_agents_that_said_nothing_is_not_nothing_running(self):
        for out, rc in (('', 0), ('[]', 1), ('not json', 0), ('{}', 0)):
            def run(args, timeout=30, rc=None, out=out, code=rc):
                rc.append(code)
                return out
            self.sb.run_claude_cmd = run
            self.assertFalse(self.sb.agents_known(self.sb.claude_agents()), (out, rc))
        self.sb.run_claude_cmd = lambda args, timeout=30, rc=None: rc.append(0) or '[]'
        ags = self.sb.claude_agents()
        self.assertTrue(self.sb.agents_known(ags))
        self.assertEqual(ags, [])

    def test_start_fresh_does_not_wait_for_a_writer_that_left_the_fallback(self):
        # a hook-started run wrote the fallback note and dropped its claim between our look and our try for the claim
        self.transcript(SID_A, str(self.root / 'proj'), NOW - 60)
        self.sb.rpc({'verb': 'list'}, NOW, agents=[])
        self.sb.handoff_path(SID_A).write_text(self.sb.FALLBACK_MARK + '\nraw')
        self.sb.handoff_lock = lambda sid, now: False
        slept = []
        self.job({'verb': 'fresh', 'sid': SID_A})
        with mock.patch.object(self.sb.time, 'sleep', lambda s: slept.append(s)):
            self.drain(make_handoff=lambda tp, sid: self.fail('not ours to write'))
        self.assertEqual([x for x in slept if x == 2], [])
        self.assertEqual(len(self.launched), 1)

    def test_a_write_that_cannot_land_leaves_no_temp_file(self):
        st = self.sb.paths()['state']
        with mock.patch.object(self.sb.os, 'replace', mock.Mock(side_effect=PermissionError('in use'))):
            with self.assertRaises(OSError):
                self.sb._jput(st / 'x.json', {'a': 1})
            with self.assertRaises(OSError):
                self.sb._jput_text(st / 'y.md', 'text')
        self.assertEqual([f.name for f in st.iterdir() if f.name.endswith('.tmp')], [])

    def test_a_chats_tool_list_stays_while_its_transcript_does(self):
        st, old = self.sb.paths()['state'], NOW - 60 * 86400
        proj = self.cdir / 'projects' / 'p'
        proj.mkdir(parents=True, exist_ok=True)
        gone = 'aaaaaaaa-0000-4000-8000-000000000009'
        t = proj / f'{SID_A}.jsonl'
        t.write_text('{}\n')
        for f in (t, ):
            os.utime(f, (old, old))
        for sid in (SID_A, gone):
            for ext in ('.tools', '.ping'):
                (st / f'{sid}{ext}').write_text('{}')
                os.utime(st / f'{sid}{ext}', (old, old))
        self.sb.prune_state(NOW, force=True)
        self.assertEqual(sorted(f.name for f in st.iterdir() if f.name.endswith(('.tools', '.ping'))), [f'{SID_A}.tools'])

    def test_a_process_that_will_not_say_its_command_line_is_not_asked_at_every_poll(self):
        agents = [{'kind': 'interactive', 'sessionId': SID_A, 'pid': 4242, 'startedAt': 5}]
        asked = []
        self.sb._cmdlines = lambda pids, loose=None: asked.append(list(pids)) or {}
        self.sb._roster_args = lambda: {}
        for dt_ in (0, 15, 30, 599):
            self.assertEqual(self.sb.launch_flags(agents, NOW + dt_), {SID_A: {}})
        self.assertEqual(asked, [[4242]])
        self.sb._cmdlines = lambda pids, loose=None: asked.append(list(pids)) or {4242: ['claude', '--chrome']}
        self.assertEqual(self.sb.launch_flags(agents, NOW + 700), {SID_A: {'chrome': True}})  # asked again later: healed
        self.sb.launch_flags(agents, NOW + 715)
        self.assertEqual(len(asked), 2)
        # what an older helper wrote for this very process is kept when the process will not say
        flags = self.sb.paths()['state'] / 'flags.json'
        flags.write_text(json.dumps({SID_A: {'launch': {'chrome': True}, 'all': [['--chrome']], 'pid': 4242, 'started': 5}}))
        self.sb._cmdlines = lambda pids, loose=None: asked.append(list(pids)) or {}
        self.assertEqual(self.sb.launch_flags(agents, NOW + 800), {SID_A: {'chrome': True}})
        self.sb.launch_flags(agents, NOW + 815)
        self.assertEqual(len(asked), 3)

    def test_a_first_message_by_file_survives_a_launch_that_never_came_up(self):
        long = 'x' * 7000
        self.sb._is_shim = lambda exe: True  # an npm install on Windows: a long first message goes by file
        self.job({'verb': 'start', 'folder': 'proj', 'prompt': long, 'handoff_text': 'HANDOFF', 'name': 'Long'})
        seen, health = [], iter(['dead', 'ready'])

        def run(argv, cwd, env):
            self.launched.append({'argv': argv})
            # the chat gets as far as its first hook, which takes the message and the handoff
            seen.append(self.sb.handle_hook({'hook_event_name': 'UserPromptSubmit', 'session_id': 'S9', 'transcript_path': '/none',
                                             'prompt': argv[-1]}, NOW + 6, None))
            return 0, '  claude stop 3022c163    stop this session\n'
        self.sb.run_claude_cmd = lambda args, **kw: ''
        self.drain(runner=run, live=lambda bg: next(health))
        self.assertEqual(len(seen), 2)
        for out in seen:
            ctx = out['hookSpecificOutput']['additionalContext']
            self.assertIn('HANDOFF', ctx)
            self.assertIn('x' * 100, ctx)


if __name__ == '__main__':
    unittest.main()
