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
