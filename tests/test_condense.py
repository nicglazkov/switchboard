import tempfile, time, unittest
from tests.helpers import fresh_sb, user, assistant, entry, write_transcript

class CondenseTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(); self.sb = fresh_sb(self.tmp)

    def rows(self):
        return [
            entry('ai-title', None, aiTitle='HA lights'),
            user('2026-10-10T10:00:00Z', 'Remember that the bedside lamp shines in my eyes. Fix the blinds.'),
            entry('user', '2026-10-10T10:00:01Z', isMeta=True, message={'role': 'user', 'content': 'Base directory for this skill: /x\nAlways do X'}),
            entry('user', '2026-10-10T10:00:02Z', origin={'kind': 'task-notification'}, message={'role': 'user', 'content': '<task-notification>never mind</task-notification>'}),
            entry('user', '2026-10-10T10:00:03Z', message={'role': 'user', 'content': [{'type': 'tool_result', 'content': 'secret tool output'}]}),
            user('2026-10-10T10:00:04Z', '<command-name>/model</command-name>'),
            user('2026-10-10T10:00:05Z', 'This session is being continued from a previous conversation. Summary: did A.'),
            assistant('2026-10-10T10:00:06Z', 'Done with the blinds.', tools=[('Edit', {'file_path': '/c/automations.yaml'}), ('Bash', {'command': 'ha core check'})]),
            user('2026-10-10T10:00:07Z', "From now on don't use em dashes."),
        ]

    def test_keeps_real_user_messages_and_drops_injected(self):
        c = self.sb.condense(write_transcript(self.tmp, self.rows()))
        self.assertIn('USER: Remember that the bedside lamp', c['body'])
        self.assertNotIn('Base directory for this skill', c['body'])
        self.assertNotIn('task-notification', c['body'])
        self.assertNotIn('secret tool output', c['body'])
        self.assertNotIn('<command-name>', c['body'])
        self.assertIn('PRIOR COMPACTION SUMMARY: This session is being continued', c['body'])
        self.assertIn('CLAUDE: Done with the blinds.', c['body'])
        self.assertIn('[tool Edit: /c/automations.yaml]', c['body'])
        self.assertIn('[tool Bash: ha core check]', c['body'])

    def test_directives_verbatim_from_user_only(self):
        c = self.sb.condense(write_transcript(self.tmp, self.rows()))
        self.assertIn('Remember that the bedside lamp shines in my eyes.', c['directives'])
        self.assertIn("From now on don't use em dashes.", c['directives'])
        self.assertFalse(any('Always do X' in d for d in c['directives']))

    def test_files_and_title(self):
        c = self.sb.condense(write_transcript(self.tmp, self.rows()))
        self.assertEqual(c['files'], ['/c/automations.yaml'])
        self.assertEqual(c['title'], 'HA lights')

    def test_long_body_keeps_start_and_end(self):
        rows = [user('2026-10-10T10:00:00Z', 'GOAL: build the thing')]
        rows += [user('2026-10-10T10:00:01Z', 'filler ' * 200) for _ in range(400)]
        rows.append(user('2026-10-10T10:00:02Z', 'LAST: ship it'))
        c = self.sb.condense(write_transcript(self.tmp, rows), max_chars=20_000)
        self.assertLessEqual(len(c['body']), 20_200)
        self.assertIn('GOAL: build the thing', c['body'])
        self.assertIn('LAST: ship it', c['body'])
        self.assertIn('[... middle of session omitted ...]', c['body'])

    def test_render_has_sections(self):
        out = self.sb.render_condensed(self.sb.condense(write_transcript(self.tmp, self.rows())))
        for h in ('# Session: HA lights', '## Directives (verbatim)', '## Files touched', '## Conversation'):
            self.assertIn(h, out)

    def test_large_transcript_is_fast(self):
        rows = []
        for i in range(3000):
            rows.append(user('2026-10-10T10:00:00Z', f'question {i}'))
            rows.append(assistant('2026-10-10T10:00:01Z', 'answer ' * 50, tools=[('Read', {'file_path': f'/f{i % 50}.py'})]))
            rows.append(entry('user', '2026-10-10T10:00:02Z', message={'role': 'user', 'content': [{'type': 'tool_result', 'content': 'y' * 8000}]}))
        p = write_transcript(self.tmp, rows)
        t = time.time(); self.sb.condense(p)
        self.assertLess(time.time() - t, 3.0)


PASTE = '\n\n<pasted_content id="797f">\n{}\n</pasted_content id="797f">\n'


class PastedContentTest(unittest.TestCase):
    """Claude Code wraps a long paste in <pasted_content id="...">. Those are the user's own words (often their
    longest messages), and they used to be dropped with the harness records that also start with a tag."""

    def setUp(self):
        self.tmp = tempfile.mkdtemp(); self.sb = fresh_sb(self.tmp)

    def condense(self, *texts, **kw):
        rows = [user(f'2026-10-10T10:00:{i:02d}Z', t) for i, t in enumerate(texts)]
        return self.sb.condense(write_transcript(self.tmp, rows), **kw)

    def test_a_message_that_is_one_paste_is_kept_without_its_tags(self):
        c = self.condense(PASTE.format('From now on never push to main. The plan has three steps.'))
        self.assertIn('USER: From now on never push to main. The plan has three steps.', c['body'])
        self.assertNotIn('pasted_content', c['body'])
        self.assertIn('From now on never push to main.', c['directives'])
        self.assertEqual(c['user_messages'], ['From now on never push to main. The plan has three steps.'])

    def test_typed_words_around_a_paste_are_kept_too(self):
        c = self.condense('Here is the log' + PASTE.format('error: disk full') + '\nAlways check the disk first.')
        self.assertIn('USER: Here is the log\n\nerror: disk full\n\nAlways check the disk first.', c['body'])
        self.assertIn('Always check the disk first.', c['directives'])

    def test_several_pastes_and_one_never_closed(self):
        c = self.condense(PASTE.format('first part') + PASTE.format('second part'), 'see <pasted_content>\nthe tail')
        self.assertIn('USER: first part\n\nsecond part', c['body'])
        self.assertIn('USER: see\n\nthe tail', c['body'])

    def test_a_pasted_page_that_starts_with_a_tag_is_still_the_users(self):
        c = self.condense(PASTE.format('<html><body>Never cache this page.</body></html>'))
        self.assertIn('USER: <html><body>Never cache this page.</body></html>', c['body'])

    def test_a_very_long_paste_keeps_its_start_and_end(self):
        big = 'START of the paste. ' + 'filler ' * 20_000 + 'END of the paste.'
        c = self.condense('Read this:' + PASTE.format(big))
        self.assertIn('START of the paste.', c['body'])
        self.assertIn('END of the paste.', c['body'])
        self.assertIn('[... middle of the pasted text left out ...]', c['body'])
        self.assertLess(len(c['body']), self.sb.PASTE_KEEP + 300)

    def test_harness_records_are_still_dropped(self):
        noise = ['<system-reminder>be careful</system-reminder>', '<task-notification>\n<task-id>b1</task-id>',
                 '<command-name>/model</command-name>', '<local-command-stdout>Set model to opus</local-command-stdout>',
                 '<bash-input>ls</bash-input>', '<bash-stdout>a.txt</bash-stdout>', '<agent-message from="a1">done',
                 '<<autonomous-loop-dynamic>>', '[Request interrupted by user]']
        c = self.condense(*noise, 'the one real message')
        self.assertEqual(c['user_messages'], ['the one real message'])
        self.assertEqual(c['body'], 'USER: the one real message')

    def test_the_fallback_note_carries_a_pasted_message(self):
        t = write_transcript(self.tmp, [user('2026-10-10T10:00:00Z', PASTE.format('Remember the staging key rotates on Monday.')),
                                        assistant('2026-10-10T10:00:05Z', 'Noted.')])
        def runner(text, prompt):
            raise RuntimeError('offline')
        text = self.sb.generate_handoff(t, 'S1', runner=runner).read_text()
        self.assertIn('- Remember the staging key rotates on Monday.', text.split('## Your instructions, verbatim')[1])

    def test_the_list_shows_a_pasted_message_as_the_last_prompt(self):
        t = write_transcript(self.tmp, [user('2026-10-10T10:00:00Z', 'older'), assistant('2026-10-10T10:00:05Z', 'ok'),
                                        user('2026-10-10T10:01:00Z', PASTE.format('the newest words'))])
        self.assertEqual(self.sb.transcript_stats(t)['last_prompt'], 'the newest words')

