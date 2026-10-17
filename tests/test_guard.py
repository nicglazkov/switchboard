import tempfile, unittest
from tests.helpers import fresh_sb

class GuardTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(); self.sb = fresh_sb(self.tmp)

    def d(self, prompt='fix it', ctx=640_000, cold=True, held=None, has_pass=False, now=10_000):
        return self.sb.guard_decision(prompt, ctx, cold, held, has_pass, now)['action']

    def test_blocks_big_cold(self):
        self.assertEqual(self.d(), 'block')

    def test_allows_small_or_warm(self):
        self.assertEqual(self.d(ctx=149_999), 'allow')
        self.assertEqual(self.d(cold=False), 'allow')

    def test_slash_commands_always_allowed(self):
        self.assertEqual(self.d(prompt='/clear'), 'allow')
        self.assertEqual(self.d(prompt='  /compact focus'), 'allow')

    def test_resend_same_prompt_within_window_allows(self):
        self.assertEqual(self.d(held={'prompt': 'fix it', 'ts': 10_000 - 899}), 'allow')
        self.assertEqual(self.d(held={'prompt': 'fix it', 'ts': 10_000 - 901}), 'block')
        self.assertEqual(self.d(prompt='other', held={'prompt': 'fix it', 'ts': 9_990}), 'block')

    def test_go_releases_held_prompt(self):
        self.assertEqual(self.d(prompt='Go', held={'prompt': 'fix it', 'ts': 9_990}), 'allow_go')
        self.assertEqual(self.d(prompt='go', held=None), 'block')  # nothing held: 'go' is just a prompt

    def test_one_time_pass(self):
        self.assertEqual(self.d(has_pass=True), 'allow')

    def test_is_cold_prefers_statusline_expiry(self):
        self.assertTrue(self.sb.is_cold({'cache_expires_at': 9_000}, {'last_assistant_ts': 9_999}, 10_000))
        self.assertFalse(self.sb.is_cold({'cache_expires_at': 11_000}, {'last_assistant_ts': 1}, 10_000))
        self.assertTrue(self.sb.is_cold({}, {'last_assistant_ts': 10_000 - 3600}, 10_000))
        self.assertFalse(self.sb.is_cold({}, {'last_assistant_ts': 10_000 - 3599}, 10_000))
        self.assertFalse(self.sb.is_cold({}, {'last_assistant_ts': None}, 10_000))

    def test_held_pass_pending_storage(self):
        sb = self.sb
        sb.held_set('S1', 'fix it', 100); self.assertEqual(sb.held_get('S1')['prompt'], 'fix it')
        sb.held_clear('S1'); self.assertIsNone(sb.held_get('S1'))
        (sb.paths()['state'] / 'S1.pass').write_text('100')
        self.assertTrue(sb.pass_take('S1', 110)); self.assertFalse(sb.pass_take('S1', 110))
        sb.pending_put(4242, {'session_id': 'S1', 'ts': 100})
        self.assertIsNone(sb.pending_take(4242, now=100 + 1801))
        sb.pending_put(4242, {'session_id': 'S1', 'ts': 100})
        self.assertEqual(sb.pending_take(4242, now=200)['session_id'], 'S1')
        self.assertIsNone(sb.pending_take(4242, now=200))

    def test_find_claude_pid_posix_name(self):
        table = {10: (9, 'python3'), 9: (8, '/bin/sh'), 8: (1, 'claude'), 1: (0, 'launchd')}
        self.assertEqual(self.sb.find_claude_pid(10, table), 8)

    def test_find_claude_pid_windows_name(self):
        table = {10: (9, 'python.exe'), 9: (8, 'cmd.exe'), 8: (2, 'claude.exe'), 2: (0, 'explorer.exe')}
        self.assertEqual(self.sb.find_claude_pid(10, table), 8)

    def test_find_claude_pid_none_and_cycles(self):
        self.assertIsNone(self.sb.find_claude_pid(10, {10: (9, 'python3'), 9: (10, 'sh')}))

    def test_card_text(self):
        card = self.sb.guard_card(640_000, 9 * 3600, 'http://x/s/desk/S1')
        self.assertIn('640k tokens', card); self.assertIn('9h', card)
        self.assertIn('/clear', card); self.assertIn('http://x/s/desk/S1', card)
        self.assertNotIn('\u2014', card)  # no em dash
