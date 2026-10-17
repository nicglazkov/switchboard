import json, os, tempfile, time, unittest
from tests.helpers import fresh_sb, user, assistant, write_transcript

class HandoffTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(); self.sb = fresh_sb(self.tmp)
        self.t = write_transcript(self.tmp, [
            user('2026-10-10T10:00:00Z', 'Remember the Shelly plug powers the camera.'),
            assistant('2026-10-10T10:00:05Z', 'Noted.'),
        ])

    def test_writes_summary_plus_verbatim_directives_and_footer(self):
        seen = {}
        def runner(stdin_text, system_prompt):
            seen['in'], seen['sys'] = stdin_text, system_prompt
            return '## Goal\nKeep lights sane.'
        p = self.sb.generate_handoff(self.t, 'S1', runner=runner)
        text = p.read_text()
        self.assertTrue(text.startswith('# Handoff: '))
        self.assertIn('## Goal\nKeep lights sane.', text)
        self.assertIn('## Your instructions, verbatim', text)
        self.assertIn('- Remember the Shelly plug powers the camera.', text)
        self.assertIn(self.t, text)  # footer points at the original transcript
        self.assertIn('Standing instructions', seen['sys'])
        self.assertIn('USER: Remember the Shelly plug', seen['in'])

    def test_runner_failure_writes_fallback(self):
        def runner(stdin_text, system_prompt):
            raise RuntimeError('boom')
        text = self.sb.generate_handoff(self.t, 'S1', runner=runner).read_text()
        self.assertIn('Automatic summary failed', text)
        self.assertIn('- Remember the Shelly plug powers the camera.', text)
        self.assertIn('Remember the Shelly plug powers the camera.', text.split('## Last messages from the user')[1])

    def test_freshness(self):
        self.assertFalse(self.sb.handoff_is_fresh('S1', self.t))
        self.sb.generate_handoff(self.t, 'S1', runner=lambda a, b: 'x')
        self.assertTrue(self.sb.handoff_is_fresh('S1', self.t))
        os.utime(self.t, None)  # bookkeeping only (claude stop, cost lines): no new work, still fresh
        self.assertTrue(self.sb.handoff_is_fresh('S1', self.t))
        later = time.strftime('%Y-%m-%dT%H:%M:%S.000Z', time.gmtime(time.time() + 5))
        with open(self.t, 'a') as f:  # Claude replied after the handoff
            f.write(json.dumps(assistant(later, 'more work')) + '\n')
        self.assertFalse(self.sb.handoff_is_fresh('S1', self.t))

    def test_atomic_write_no_partial_file(self):
        self.sb.generate_handoff(self.t, 'S1', runner=lambda a, b: 'x')
        names = os.listdir(self.sb.paths()['handoffs'])
        self.assertEqual(sorted(names), ['S1.md'])


class ClaudeEnvTest(unittest.TestCase):
    def test_spawned_claude_uses_the_plan_login_not_an_api_key(self):
        # The Tower has ANTHROPIC_API_KEY as a user env var; `claude -p` would bill the API instead of the plan.
        import os
        from tests.helpers import fresh_sb
        sb = fresh_sb(tempfile.mkdtemp())
        os.environ['ANTHROPIC_API_KEY'] = 'sk-test'
        try:
            env = sb.claude_env(SB_HANDOFF='x')
        finally:
            del os.environ['ANTHROPIC_API_KEY']
        self.assertNotIn('ANTHROPIC_API_KEY', env)
        self.assertEqual(env['SB_INTERNAL'], '1')
        self.assertEqual(env['SB_HANDOFF'], 'x')


class HandoffCwdTest(unittest.TestCase):
    def test_handoff_runs_in_the_session_folder_not_the_launcher_folder(self):
        from unittest import mock
        tmp = tempfile.mkdtemp(); sb = fresh_sb(tmp)
        proj = os.path.join(tmp, 'proj'); os.mkdir(proj)
        t = write_transcript(tmp, [dict(user('2026-10-10T10:00:00Z', 'hi'), cwd=proj), dict(assistant('2026-10-10T10:00:05Z', 'ok'), cwd=proj)])
        seen = []
        with mock.patch.object(sb, 'run_claude_opus', lambda text, prompt, cwd=None, **kw: seen.append(cwd) or '## Goal\nx'):
            sb.generate_handoff(t, 'S1')
            os.rmdir(proj)
            sb.generate_handoff(t, 'S1')
        self.assertEqual(seen, [proj, str(sb.Path.home())])


class SummaryModelTest(unittest.TestCase):
    def test_opus_by_default_and_sonnet_when_settings_say_so(self):
        from unittest import mock
        tmp = tempfile.mkdtemp(); sb = fresh_sb(tmp)
        t = write_transcript(tmp, [user('2026-10-10T10:00:00Z', 'hi'), assistant('2026-10-10T10:00:05Z', 'ok')])
        seen = []
        with mock.patch.object(sb, 'run_claude_opus', lambda text, prompt, cwd=None, model='opus': seen.append(model) or '## Goal\nx'):
            sb.generate_handoff(t, 'S1')
            sb.remember_identity({'summary_model': 'sonnet'})
            sb.generate_handoff(t, 'S1')
            sb.remember_identity({'summary_model': 'gpt'})  # anything else is ignored
            sb.generate_handoff(t, 'S1')
        self.assertEqual(seen, ['opus', 'sonnet', 'sonnet'])
