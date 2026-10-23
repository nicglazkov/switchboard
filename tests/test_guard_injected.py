import os, tempfile, time, unittest
from pathlib import Path
from tests.helpers import fresh_sb, user, assistant, entry, write_transcript

BIG = {'input_tokens': 1, 'cache_creation_input_tokens': 0, 'cache_read_input_tokens': 640_000, 'output_tokens': 1}


class GuardInjectedTest(unittest.TestCase):
    """2026-10-12: the guard held task notifications and agent hand-backs in big idle sessions, so overnight work was
    never picked up; and it kept a session's pre-compaction size after Claude Code compacted it while idle."""

    def setUp(self):
        self.tmp = tempfile.mkdtemp(); self.sb = fresh_sb(self.tmp)
        os.environ['SB_CLAUDE_DIR'] = str(Path(self.tmp) / 'claude')
        self.t = write_transcript(self.tmp, [user('2026-10-10T01:00:00Z', 'Run the agents.'),
                                             assistant('2026-10-10T01:00:05Z', 'ok', usage=BIG)])
        import datetime as dt
        self.now = dt.datetime(2026, 10, 10, 10, tzinfo=dt.timezone.utc).timestamp()  # 9h later: big and cold
        self.spawned, self.notified = [], []

    def tearDown(self):
        os.environ.pop('SB_CLAUDE_DIR', None)

    def run_hook(self, prompt, t=None, now=None):
        inp = {'hook_event_name': 'UserPromptSubmit', 'session_id': 'S1', 'transcript_path': t or self.t,
               'prompt': prompt, 'cwd': '/w'}
        return self.sb.handle_hook(inp, now or self.now, 777, spawn=lambda *a: self.spawned.append(a),
                                   notify=self.notified.append)

    def assert_untouched(self):
        self.assertEqual((self.spawned, self.notified), ([], []))
        self.assertIsNone(self.sb.held_get('S1'))
        self.assertFalse((self.sb.paths()['pending'] / '777.json').exists())

    def test_injected_prompts_always_pass_without_side_effects(self):
        for p in ('<task-notification>\n<task-id>b1</task-id>\n<status>completed</status>',
                  '<agent-message from="a74"> [Subagent hand-back] The text below is the final report',
                  '<cross-session-message from="worker">check this</cross-session-message>',
                  '<teammate-message teammate_id="x">done</teammate-message>',
                  '<<autonomous-loop-dynamic>>', '[SYSTEM NOTIFICATION - NOT USER INPUT] wake',
                  '\n  <task-notification>',
                  'Another Claude session sent a message:\n<agent-message from="a5c7">[Subagent hand-back] done'):
            self.assertIsNone(self.run_hook(p), p)
        self.assert_untouched()

    def test_a_subagents_prompt_passes(self):
        inp = {'hook_event_name': 'UserPromptSubmit', 'session_id': 'S1', 'transcript_path': self.t,
               'prompt': 'look at the logs', 'cwd': '/w', 'agent_id': 'a12'}
        self.assertIsNone(self.sb.handle_hook(inp, self.now, 777, spawn=lambda *a: None, notify=self.notified.append))
        self.assert_untouched()

    def test_typed_prompts_still_guarded(self):
        for p in ('fix the lights', 'a < b please', '< not a tag', '<3 thanks', 'Here is the error:\n<html>'):
            self.assertFalse(self.sb.is_injected(p), p)
        self.assertEqual(self.run_hook('fix the lights')['decision'], 'block')

    def test_running_subagent_keeps_session_warm(self):
        sub = Path(self.tmp) / 'claude' / 'projects' / '-other-cwd' / 'S1' / 'subagents'
        sub.mkdir(parents=True)
        f = sub / 'agent-a1.jsonl'
        f.write_text('{}\n')
        os.utime(f, (self.now - 600, self.now - 600))  # an hour-long subagent, last wrote 10 min ago
        self.assertIsNone(self.run_hook('how is it going?'))
        self.assert_untouched()
        os.utime(f, (self.now - 3700, self.now - 3700))  # gone quiet for over an hour: back to normal
        self.assertEqual(self.run_hook('how is it going?')['decision'], 'block')

    def test_idle_compaction_uses_the_new_size(self):
        rows = [user('2026-10-10T01:00:00Z', 'Run the agents.'), assistant('2026-10-10T01:00:05Z', 'ok', usage=BIG),
                entry('system', '2026-10-10T01:50:00Z', subtype='compact_boundary', content='Conversation compacted',
                      compactMetadata={'trigger': 'auto', 'preTokens': 640_001, 'postTokens': 23_542}),
                user('2026-10-10T01:50:00Z', 'This session is being continued from a previous conversation.')]
        t = write_transcript(self.tmp, rows, name='compacted.jsonl')
        st = self.sb.transcript_stats(t)
        self.assertEqual(st['ctx_tokens'], 23_542)
        import datetime as dt, json
        between = dt.datetime(2026, 10, 10, 1, 30, tzinfo=dt.timezone.utc).timestamp()  # status line from before
        (self.sb.paths()['state'] / 'S1.json').write_text(json.dumps(
            {'ctx_tokens': 640_001, 'cache_expires_at': between + 3600, 'updated': between}))
        self.assertIsNone(self.run_hook('typed after the compaction', t=t))
        self.assert_untouched()


if __name__ == '__main__':
    unittest.main()
