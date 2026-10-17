import tempfile, unittest
from tests.helpers import fresh_sb, user, assistant, entry, write_transcript

class StatsTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(); self.sb = fresh_sb(self.tmp)

    def test_ctx_is_last_main_assistant_usage(self):
        p = write_transcript(self.tmp, [
            user('2026-10-10T10:00:00Z', 'hi'),
            assistant('2026-10-10T10:00:05Z', 'a', usage={'input_tokens': 1, 'cache_creation_input_tokens': 10, 'cache_read_input_tokens': 100, 'output_tokens': 5}),
            assistant('2026-10-10T10:00:09Z', 'b', usage={'input_tokens': 2, 'cache_creation_input_tokens': 2000, 'cache_read_input_tokens': 300000, 'output_tokens': 9}),
            assistant('2026-10-10T10:00:10Z', 'sub', side=True, usage={'input_tokens': 9, 'cache_creation_input_tokens': 9, 'cache_read_input_tokens': 9, 'output_tokens': 9}),
        ])
        s = self.sb.transcript_stats(p)
        self.assertEqual(s['ctx_tokens'], 302002)
        self.assertIsInstance(s['last_assistant_ts'], float)
        self.assertEqual(s['model'], 'claude-opus-5-5')

    def test_last_assistant_ts_ignores_later_user_entry(self):
        p = write_transcript(self.tmp, [
            assistant('2026-10-10T10:00:00Z', 'a'),
            user('2026-10-10T19:00:00Z', 'new prompt'),
        ])
        s = self.sb.transcript_stats(p)
        import datetime as dt
        self.assertEqual(s['last_assistant_ts'], dt.datetime(2026, 10, 10, 10, tzinfo=dt.timezone.utc).timestamp())

    def test_title_from_ai_title(self):
        p = write_transcript(self.tmp, [entry('ai-title', None, aiTitle='Debug GE Home'), assistant('2026-10-10T10:00:00Z', 'a')])
        self.assertEqual(self.sb.transcript_stats(p)['title'], 'Debug GE Home')

    def test_missing_file_returns_zeros(self):
        s = self.sb.transcript_stats('/nonexistent/x.jsonl')
        self.assertEqual(s, {'last_prompt': None, 'ctx_tokens': 0, 'last_assistant_ts': None, 'model': None, 'title': None, 'cwd': None,
                             'bridge': None, 'custom_title': None, 'entrypoint': None, 'effort': None, 'speed': None, 'perm': None,
                             'chrome_used': False, 'chrome_loaded': None, 'ultra': None, 'compacted_ts': None, 'last_ts': None, 'archived_ts': None})

    def test_reads_only_tail_of_huge_file(self):
        rows = [user('2026-10-10T10:00:00Z', 'x' * 5000) for _ in range(6000)]  # ~30 MB
        rows.append(assistant('2026-10-10T11:00:00Z', 'end', usage={'input_tokens': 1, 'cache_creation_input_tokens': 0, 'cache_read_input_tokens': 7, 'output_tokens': 1}))
        p = write_transcript(self.tmp, rows)
        import time; t = time.time()
        self.assertEqual(self.sb.transcript_stats(p)['ctx_tokens'], 8)
        self.assertLess(time.time() - t, 0.5)
