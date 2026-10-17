import io, json, os, sys, tempfile, unittest
from unittest import mock
from tests.helpers import fresh_sb, user, assistant, write_transcript

BIG = {'input_tokens': 1, 'cache_creation_input_tokens': 0, 'cache_read_input_tokens': 640_000, 'output_tokens': 1}

class HookTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(); self.sb = fresh_sb(self.tmp)
        self.t = write_transcript(self.tmp, [user('2026-10-10T01:00:00Z', 'Remember X.'), assistant('2026-10-10T01:00:05Z', 'ok', usage=BIG)])
        import datetime as dt
        self.now = dt.datetime(2026, 10, 10, 10, tzinfo=dt.timezone.utc).timestamp()  # 9h later
        self.spawned, self.notified = [], []

    def run_hook(self, inp, pid=777, now=None):
        return self.sb.handle_hook(inp, now or self.now, pid, spawn=lambda *a: self.spawned.append(a), notify=self.notified.append)

    def ups(self, prompt, sid='S1'):
        return {'hook_event_name': 'UserPromptSubmit', 'session_id': sid, 'transcript_path': self.t, 'prompt': prompt, 'cwd': '/w'}

    def test_cold_big_prompt_blocks_and_notifies_without_writing_a_summary(self):
        out = self.run_hook(self.ups('fix the lights'))
        self.assertEqual(out['decision'], 'block')
        self.assertIn('640k tokens', out['reason'])
        self.assertIn('Start fresh: about', out['reason'])  # the whole cost, explained on the card
        self.assertIn('Resume: about 1.3M', out['reason'])
        self.assertIn('API price', out['reason'])
        self.assertEqual(self.spawned, [])  # the user may say "go": no summary until Start fresh is picked
        self.assertEqual(self.notified[0]['event'], 'guard')
        self.assertEqual(self.notified[0]['session_id'], 'S1')
        self.assertEqual(self.sb.held_get('S1')['prompt'], 'fix the lights')

    def test_resend_passes_and_clears_held(self):
        self.run_hook(self.ups('fix the lights'))
        self.assertIsNone(self.run_hook(self.ups('fix the lights'), now=self.now + 30))
        self.assertIsNone(self.sb.held_get('S1'))

    def test_go_injects_held_prompt(self):
        self.run_hook(self.ups('fix the lights'))
        out = self.run_hook(self.ups('go'), now=self.now + 30)
        self.assertIn('fix the lights', out['hookSpecificOutput']['additionalContext'])

    def test_clear_after_block_writes_the_summary_then(self):
        self.run_hook(self.ups('fix the lights'))
        with mock.patch.object(self.sb, 'HANDOFF_WAIT', 0):
            self.run_hook({'hook_event_name': 'SessionStart', 'source': 'clear', 'session_id': 'S2', 'transcript_path': '/n'}, now=self.now + 60)
        self.assertEqual(self.spawned, [(self.t, 'S1')])

    def test_clear_after_block_injects_handoff_and_held(self):
        self.run_hook(self.ups('fix the lights'))
        self.sb.handoff_path('S1').write_text('# Handoff: test\nPURPLE')
        out = self.run_hook({'hook_event_name': 'SessionStart', 'source': 'clear', 'session_id': 'S2', 'transcript_path': '/n'}, now=self.now + 60)
        ctx = out['hookSpecificOutput']['additionalContext']
        self.assertIn('PURPLE', ctx); self.assertIn('fix the lights', ctx)
        self.assertIsNone(self.run_hook({'hook_event_name': 'SessionStart', 'source': 'clear', 'session_id': 'S3', 'transcript_path': '/n'}, now=self.now + 90))

    def test_clear_without_pending_injects_nothing(self):
        self.assertIsNone(self.run_hook({'hook_event_name': 'SessionStart', 'source': 'clear', 'session_id': 'S2', 'transcript_path': '/n'}))

    def test_clear_from_other_claude_process_injects_nothing(self):
        self.run_hook(self.ups('fix the lights'), pid=777)
        self.sb.handoff_path('S1').write_text('x')
        self.assertIsNone(self.run_hook({'hook_event_name': 'SessionStart', 'source': 'clear', 'session_id': 'S2', 'transcript_path': '/n'}, pid=888))

    def test_startup_with_sb_handoff_env_injects(self):
        p = os.path.join(self.tmp, 'h.md'); open(p, 'w').write('HANDOFF BODY')
        with mock.patch.dict(os.environ, {'SB_HANDOFF': p}):
            out = self.run_hook({'hook_event_name': 'SessionStart', 'source': 'startup', 'session_id': 'S5', 'transcript_path': '/n'})
        self.assertIn('HANDOFF BODY', out['hookSpecificOutput']['additionalContext'])

    def test_handoff_missing_after_wait_uses_fallback_note(self):
        self.run_hook(self.ups('fix the lights'))
        with mock.patch.object(self.sb, 'HANDOFF_WAIT', 0):
            out = self.run_hook({'hook_event_name': 'SessionStart', 'source': 'clear', 'session_id': 'S2', 'transcript_path': '/n'}, now=self.now + 60)
        ctx = out['hookSpecificOutput']['additionalContext']
        self.assertIn('handoff was not ready', ctx); self.assertIn(self.t, ctx); self.assertIn('fix the lights', ctx)

    def test_session_end_writes_no_summary(self):
        self.run_hook({'hook_event_name': 'SessionEnd', 'session_id': 'S1', 'transcript_path': self.t})
        self.assertEqual(self.spawned, [])

    def test_internal_runs_are_ignored(self):
        with mock.patch.dict(os.environ, {'SB_INTERNAL': '1'}):
            self.assertIsNone(self.run_hook(self.ups('fix the lights')))

    def test_hook_garbage_input_fails_open(self):
        for raw in ('', 'not json', '{}', '{"hook_event_name": "UserPromptSubmit"}'):
            with mock.patch.object(sys, 'stdin', io.StringIO(raw)), mock.patch.object(sys, 'stdout', io.StringIO()) as out:
                rc = self.sb.main(['sb.py', 'hook'])
            self.assertEqual(rc, 0); self.assertEqual(out.getvalue(), '')

    def test_statusline_cli_prints_and_writes_state(self):
        from pathlib import Path
        data = (Path(__file__).parent / 'fixtures' / 'statusline_sample.json').read_text()
        with mock.patch.object(sys, 'stdin', io.StringIO(data)), mock.patch.object(sys, 'stdout', io.StringIO()) as out:
            self.assertEqual(self.sb.main(['sb.py', 'statusline']), 0)
        self.assertIn('Haiku', out.getvalue())
        self.assertIn('ctx_tokens', json.dumps(self.sb.read_state(json.loads(data)['session_id'])))


class ReviewFixTest(unittest.TestCase):
    setUp, run_hook, ups = HookTest.setUp, HookTest.run_hook, HookTest.ups

    def clear(self, pid=777, dt=60):
        return self.run_hook({'hook_event_name': 'SessionStart', 'source': 'clear', 'session_id': 'S2', 'transcript_path': '/n'}, pid=pid, now=self.now + dt)

    def test_resend_then_clear_injects_nothing(self):
        self.run_hook(self.ups('fix the lights'))
        self.run_hook(self.ups('fix the lights'), now=self.now + 30)
        self.sb.handoff_path('S1').write_text('OLD')
        self.assertIsNone(self.clear())

    def test_go_then_clear_injects_nothing(self):
        self.run_hook(self.ups('fix the lights'))
        self.run_hook(self.ups('go'), now=self.now + 30)
        self.sb.handoff_path('S1').write_text('OLD')
        self.assertIsNone(self.clear())

    def test_startup_in_same_process_discards_pending(self):
        self.run_hook(self.ups('fix the lights'))
        self.run_hook({'hook_event_name': 'SessionStart', 'source': 'startup', 'session_id': 'S7', 'transcript_path': '/n'})
        self.sb.handoff_path('S1').write_text('OLD')
        self.assertIsNone(self.clear())

    def test_clear_never_injects_a_stale_handoff(self):
        self.sb.handoff_path('S1').write_text('STALE DAY ONE')
        os.utime(self.sb.handoff_path('S1'), (1, 1))
        self.run_hook(self.ups('fix the lights'))
        with mock.patch.object(self.sb, 'HANDOFF_WAIT', 0):
            ctx = self.clear()['hookSpecificOutput']['additionalContext']
        self.assertNotIn('STALE DAY ONE', ctx)

    def test_fresh_then_clear_spawn_one_summary(self):
        self.run_hook(self.ups('fresh'))
        self.run_hook(self.ups('something else'), now=self.now + 30)
        self.run_hook({'hook_event_name': 'SessionEnd', 'session_id': 'S1', 'transcript_path': self.t}, now=self.now + 40)
        with mock.patch.object(self.sb, 'HANDOFF_WAIT', 0):
            self.clear(dt=50)
        self.assertEqual(len(self.spawned), 1)

    def test_generation_releases_the_lock(self):
        self.assertTrue(self.sb.handoff_lock('S1', self.now))
        self.assertFalse(self.sb.handoff_lock('S1', self.now))
        self.sb.generate_handoff(self.t, 'S1', runner=lambda *a: '## Goal\nx')
        self.assertTrue(self.sb.handoff_lock('S1', self.now))

    def test_stale_lock_is_ignored(self):
        self.assertTrue(self.sb.handoff_lock('S1', self.now))
        self.assertTrue(self.sb.handoff_lock('S1', self.now + 700))

    def test_stale_state_does_not_block_an_active_session(self):
        import datetime as dt
        recent = dt.datetime.fromtimestamp(self.now - 300, dt.timezone.utc).strftime('%Y-%m-%dT%H:%M:%SZ')
        t = write_transcript(self.tmp, [user(recent, 'hi'), assistant(recent, 'ok', usage=BIG)], name='active.jsonl')
        p = self.sb.paths()['state'] / 'S4.json'
        p.write_text(json.dumps({'ctx_tokens': 900_000, 'cache_expires_at': self.now - 86400, 'updated': self.now - 90000}))
        inp = dict(self.ups('hello'), session_id='S4', transcript_path=t)
        self.assertIsNone(self.run_hook(inp))

    def test_claude_env_for_user_sessions_is_not_internal(self):
        env = self.sb.claude_env(internal=False, SB_HANDOFF='h')
        self.assertNotIn('SB_INTERNAL', env)
        self.assertEqual(self.sb.claude_env()['SB_INTERNAL'], '1')

    def test_windows_spawn_breaks_away_from_job_with_fallback(self):
        calls = []
        def fake_popen(args, **kw):
            calls.append(kw.get('creationflags'))
            if len(calls) == 1:
                raise OSError('access denied')
        with mock.patch.object(self.sb, 'IS_WIN', True), mock.patch.object(self.sb.subprocess, 'Popen', fake_popen):
            self.sb.spawn_handoff(self.t, 'S1')
        self.assertTrue(calls[0] & 0x01000000)
        self.assertFalse(calls[1] & 0x01000000)

    def test_install_failure_returns_nonzero(self):
        from pathlib import Path
        home = Path(self.tmp) / 'home'; (home / '.claude').mkdir(parents=True)
        (home / '.claude' / 'settings.json').write_text('{broken')
        with mock.patch.object(self.sb.Path, 'home', return_value=home):
            self.assertNotEqual(self.sb.main(['sb.py', 'install']), 0)
        self.assertEqual((home / '.claude' / 'settings.json').read_text(), '{broken')

    def test_big_handoff_injection_stays_under_cap_and_points_to_file(self):
        self.run_hook(self.ups('fix the lights'))
        self.sb.handoff_path('S1').write_text('# Handoff\n' + ('word ' * 8000) + '\n## Your instructions, verbatim\n- never X')
        ctx = self.clear()['hookSpecificOutput']['additionalContext']
        self.assertLessEqual(len(ctx), self.sb.INJECT_MAX)
        self.assertIn(str(self.sb.handoff_path('S1')), ctx)
        self.assertIn('fix the lights', ctx)


class FreshKeywordTest(unittest.TestCase):
    setUp, run_hook, ups = HookTest.setUp, HookTest.run_hook, HookTest.ups

    def test_fresh_arms_handoff_and_clear_loads_it_without_a_held_message(self):
        out = self.run_hook(self.ups('Fresh'), now=self.now)
        self.assertEqual(out['decision'], 'block')
        self.assertIn('/clear', out['reason'])
        self.assertEqual(len(self.spawned), 1)
        self.sb.handoff_path('S1').write_text('# Handoff\nKESTREL')
        ctx = self.run_hook({'hook_event_name': 'SessionStart', 'source': 'clear', 'session_id': 'S2', 'transcript_path': '/n'},
                            now=self.now + 60)['hookSpecificOutput']['additionalContext']
        self.assertIn('KESTREL', ctx)
        self.assertNotIn('held', ctx)

    def test_fresh_works_on_a_warm_session_too(self):
        import datetime as dt
        recent = dt.datetime.fromtimestamp(self.now - 60, dt.timezone.utc).strftime('%Y-%m-%dT%H:%M:%SZ')
        t = write_transcript(self.tmp, [user(recent, 'hi'), assistant(recent, 'ok', usage=BIG)], name='warm.jsonl')
        out = self.run_hook(dict(self.ups('fresh'), transcript_path=t))
        self.assertEqual(out['decision'], 'block')


class GuardEventTest(unittest.TestCase):
    setUp, run_hook, ups = HookTest.setUp, HookTest.run_hook, HookTest.ups

    def answers(self):
        return [e for e in self.notified if e['event'] == 'guard_answer']

    def test_resend_go_and_clear_each_report_an_answer(self):
        self.run_hook(self.ups('fix the lights'))
        self.run_hook(self.ups('fix the lights'), now=self.now + 30)
        self.assertEqual(self.answers()[-1]['session_id'], 'S1')
        self.run_hook(self.ups('again'), now=self.now + 40000)
        self.run_hook(self.ups('go'), now=self.now + 40030)
        self.assertEqual(len(self.answers()), 2)
        self.run_hook(self.ups('third'), now=self.now + 80000)
        self.sb.handoff_path('S1').write_text('x')
        self.run_hook({'hook_event_name': 'SessionStart', 'source': 'clear', 'session_id': 'S2', 'transcript_path': '/n'}, now=self.now + 80030)
        self.assertEqual(len(self.answers()), 3)

    def test_pass_is_not_spent_on_a_slash_command_or_warm_prompt(self):
        (self.sb.paths()['state'] / 'S1.pass').write_text(str(self.now))
        self.assertIsNone(self.run_hook(self.ups('/status')))
        self.assertIsNone(self.run_hook(self.ups('fix the lights')))   # cold + big: pass used here
        self.assertFalse((self.sb.paths()['state'] / 'S1.pass').exists())

    def test_notify_falls_back_to_tailscale(self):
        tried = []
        def fake_open(req, timeout):
            tried.append(req.full_url)
            if '10.0.0.5' in req.full_url:
                raise OSError('no route')
            class R:
                def close(self): pass
            return R()
        with mock.patch.object(self.sb.urllib.request, 'urlopen', fake_open):
            self.sb.notify_server({'event': 'x'})
        self.assertEqual(tried, [], 'no server is built in: nothing is sent until the server has said where it is')
        # the server's poll says where it is; the agent keeps it and tries each address in that order
        self.sb.remember_identity({'verb': 'list', 'as': 'desk', 'servers': ['http://10.0.0.5:8130', 'http://100.64.0.5:8130/']})
        with mock.patch.object(self.sb.urllib.request, 'urlopen', fake_open):
            self.sb.notify_server({'event': 'x'})
        self.assertEqual([u.split('/api')[0] for u in tried], ['http://10.0.0.5:8130', 'http://100.64.0.5:8130'])
