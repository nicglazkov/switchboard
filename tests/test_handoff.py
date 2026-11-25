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
        with mock.patch.object(sb, 'run_claude_opus', lambda text, prompt, cwd=None, model='opus', meter=None: seen.append(model) or '## Goal\nx'):
            sb.generate_handoff(t, 'S1')
            sb.remember_identity({'summary_model': 'sonnet'})
            sb.generate_handoff(t, 'S1')
            sb.remember_identity({'summary_model': 'gpt'})  # anything else is ignored
            sb.generate_handoff(t, 'S1')
        self.assertEqual(seen, ['opus', 'sonnet', 'sonnet'])


class InlineNoteTest(unittest.TestCase):
    """Half of the real handoffs (96 of 190) were over what a hook may add to a chat, so the new chat was told to
    Read the file first: a second call, about 30k more on the first turn. The summary is asked to be shorter, and a
    note that is still too long gives up whole sections, never the user's instructions."""

    PATH = '/tmp/sbhome/handoffs/S1.md'

    def setUp(self):
        self.sb = fresh_sb(tempfile.mkdtemp())

    def note(self, each=1200, rules=20, verbatim=3, sections=None):
        body = lambda name: f'## {name}\n' + (f'{name} detail. ' * (each // (len(name) + 9))).strip() + '\n'
        names = sections or ['Goal', 'Standing instructions from the user', 'Decisions made', 'Current state',
                             'Open threads and next steps', 'Gotchas', 'Key files and places']
        parts = []
        for n in names:
            if n.startswith('Standing'):
                parts.append(f'## {n}\n' + ''.join(f'- Standing rule {i}: never skip the tests.\n' for i in range(rules)))
            elif n == 'Goal':
                parts.append('## Goal\nShip the lights fix.\n')
            else:
                parts.append(body(n))
        return (f'(Handoff file: {self.PATH})\n\n# Handoff: lights (2026-10-10 10:00)\n\n' + ''.join(parts) +
                '\n\n## Your instructions, verbatim\n' + ''.join(f'- Verbatim rule {i}: always run the tests.\n' for i in range(verbatim)) +
                '\n---\nFull transcript of the old session: `/tmp/t.jsonl`. Do not read it whole; grep it for specific details if this note is missing something.\n')

    def test_the_summary_is_asked_to_fit(self):
        p = self.sb.HANDOFF_PROMPT
        self.assertNotIn('600-1500', p)
        self.assertIn('never more than 1,000 words', p)
        self.assertIn('Never shorten or drop a standing instruction to fit', p)
        self.assertIn('never more than 1,000 words', self.sb.COMPACT_PROMPT)
        # what is asked for fits beside the usual verbatim list, at the 6.5 characters a word real notes run to
        self.assertLess(1000 * 6.5 + 1400 + 400, self.sb.INJECT_MAX)

    def test_a_note_that_fits_is_delivered_whole(self):
        note = self.note(each=600)
        ctx = self.sb._fresh_context(note, None, '/tmp/seed.used.md')
        self.assertIn(note, ctx)
        self.assertNotIn('Left out', ctx)

    def test_a_long_note_gives_up_whole_sections_and_no_read_is_forced(self):
        note = self.note(each=2200)
        self.assertGreater(len(note), self.sb.INJECT_MAX)
        ctx = self.sb._fresh_context(note, 'fix the lights', '/tmp/seed.used.md')
        self.assertLessEqual(len(ctx), self.sb.INJECT_MAX)
        self.assertNotIn('Before doing anything else', ctx)
        self.assertNotIn('[...]', ctx)
        for i in range(20):  # every instruction, in the summary's words and verbatim
            self.assertIn(f'- Standing rule {i}: never skip the tests.', ctx)
        for i in range(3):
            self.assertIn(f'- Verbatim rule {i}: always run the tests.', ctx)
        self.assertIn('## Goal\nShip the lights fix.', ctx)
        self.assertIn('Full transcript of the old session', ctx)
        self.assertIn('fix the lights', ctx)
        self.assertNotIn('## Key files and places', ctx)   # the least needed goes first
        self.assertNotIn('## Gotchas', ctx)
        self.assertIn('## Open threads and next steps', ctx)
        self.assertRegex(ctx, r'Left out of this copy to keep it short: .*Gotchas, Key files and places\. '
                              r'The whole note is in /tmp/sbhome/handoffs/S1\.md: Read it when you need those parts\.')

    def test_only_as_many_sections_go_as_have_to(self):
        note = self.note(each=1800)
        ctx = self.sb._fresh_context(note, None, '/tmp/seed.used.md')
        self.assertLessEqual(len(ctx), self.sb.INJECT_MAX)
        self.assertIn('Left out of this copy to keep it short: Key files and places.', ctx)
        self.assertIn('## Gotchas', ctx)

    def test_a_section_with_more_in_its_heading_is_still_known(self):
        names = ['Goal', 'Standing instructions from the user', 'Decisions made', 'Current state (as of 2026-10-10)',
                 'Open threads and next steps', 'Gotchas', 'Key files and places']
        ctx = self.sb._fresh_context(self.note(each=4000, sections=names), None, '/tmp/seed.used.md')
        self.assertLessEqual(len(ctx), self.sb.INJECT_MAX)
        self.assertNotIn('## Current state', ctx)
        self.assertIn('Current state', ctx.split('Left out of this copy')[1])

    def test_instructions_too_long_on_their_own_are_read_from_the_file(self):
        # nothing of the user's is ever cut to make room: the chat reads the whole file instead, as before
        note = self.note(each=300, rules=150, verbatim=140)
        ctx = self.sb._fresh_context(note, None, '/tmp/seed.used.md')
        self.assertLessEqual(len(ctx), self.sb.INJECT_MAX)
        self.assertIn('Before doing anything else, Read the whole file /tmp/seed.used.md', ctx)

    def test_a_fallback_note_is_read_from_the_file(self):
        note = ('# Handoff: lights (2026-10-10 10:00)\n\n' + self.sb.FALLBACK_MARK + '\n\n## Last messages from the user\n' +
                ''.join(f'- message {i} ' + 'word ' * 150 + '\n' for i in range(20)) + '\n\n## Your instructions, verbatim\n- never X\n')
        ctx = self.sb._fresh_context(note, None, '/tmp/h.md')
        self.assertLessEqual(len(ctx), self.sb.INJECT_MAX)
        self.assertIn('Before doing anything else, Read the whole file /tmp/h.md', ctx)

    def test_without_a_handoff_line_the_file_it_came_from_is_named(self):
        note = self.note(each=2200).split('\n\n', 1)[1]  # the terminal's /clear: the note as it is on disk
        ctx = self.sb._fresh_context(note, None, '/tmp/sbhome/handoffs/S9.md')
        self.assertIn('The whole note is in /tmp/sbhome/handoffs/S9.md: Read it', ctx)
