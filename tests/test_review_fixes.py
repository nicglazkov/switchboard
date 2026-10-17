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


if __name__ == '__main__':
    unittest.main()
