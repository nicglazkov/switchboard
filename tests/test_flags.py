import json, os, unittest
from tests.helpers import user, assistant, entry, write_transcript
from tests.test_rpc import RpcBase, NOW, SID_A, SID_B


class FlagArgsTest(RpcBase):
    def test_reads_the_flags_worth_showing(self):
        f = self.sb.flags_from_args(['claude', '--resume', '--chrome', '--effort', 'max', '--model=sonnet',
                                     '--permission-mode', 'plan', '--remote-control', '--agent', 'reviewer'])
        self.assertEqual(f, {'chrome': True, 'effort': 'max', 'model': 'sonnet', 'perm': 'plan', 'remote': True, 'agent': 'reviewer'})
        self.assertEqual(self.sb.flags_from_args(['--dangerously-skip-permissions']), {'perm': 'bypassPermissions'})
        self.assertEqual(self.sb.flags_from_args(['--chrome', '--no-chrome']), {})

    def test_a_first_message_that_mentions_a_flag_is_not_a_flag(self):
        self.assertEqual(self.sb.flags_from_args(['--name', 'x', 'please use --chrome here']), {})
        self.assertEqual(self.sb.flags_from_args(['--effort']), {})          # no value
        self.assertEqual(self.sb.flags_from_args(['--effort', '--chrome']), {'chrome': True})


class SessionFlagsTest(RpcBase):
    def write(self, sid, rows, mtime=NOW - 60):
        rows = [entry('bridge-session', '2026-10-01T00:00:00Z', bridgeSessionId='cse_X', sessionId=sid),
                dict(user('2026-10-01T00:00:01Z', 'first thing I asked'), cwd=str(self.root))] + rows + [
                {'type': 'ai-title', 'aiTitle': 'T ' + sid[:4], 'sessionId': sid}]
        p = write_transcript(self.cdir / 'projects' / 'p1', rows, name=f'{sid}.jsonl')
        os.utime(p, (mtime, mtime))

    def flags(self, agents=(), sid=SID_A):
        r = self.sb.rpc({'verb': 'list'}, NOW, agents=list(agents))
        return next(s for s in r['sessions'] if s['sid'] == sid)['flags']

    def test_transcript_says_what_is_on_now(self):
        a = assistant('2026-10-01T00:00:05Z', 'ok', tools=[('mcp__claude-in-chrome__navigate', {})], model='claude-sonnet-5-5')
        a.update(effort='max')
        a['message']['usage']['speed'] = 'fast'
        self.write(SID_A, [entry('permission-mode', '2026-10-01T00:00:02Z', permissionMode='plan'), a])
        self.assertEqual(self.flags(), {'chrome': True, 'effort': 'max', 'model': 'sonnet', 'perm': 'plan', 'fast': True, 'remote': True})

    def test_defaults_make_no_chips(self):
        (self.cdir / 'settings.json').write_text(json.dumps({'model': 'opus[1m]', 'effortLevel': 'high',
                                                            'permissions': {'defaultMode': 'auto'}}))
        a = assistant('2026-10-01T00:00:05Z', 'ok', model='claude-opus-4-5')    # an older Opus does follow effortLevel
        a.update(effort='high')
        self.write(SID_A, [entry('permission-mode', '2026-10-01T00:00:02Z', permissionMode='auto'), a])
        self.assertEqual(self.flags(), {'remote': True})

    def test_opus_5_5_runs_at_medium_unless_told(self):
        # A top-level effortLevel in user settings does not reach Opus 5.5, so medium is its normal, and high is a choice.
        (self.cdir / 'settings.json').write_text(json.dumps({'effortLevel': 'high'}))
        a = assistant('2026-10-01T00:00:05Z', 'ok', model='claude-opus-5-5')
        a.update(effort='medium')
        self.write(SID_A, [a])
        self.assertNotIn('effort', self.flags())
        b = assistant('2026-10-01T00:00:05Z', 'ok', model='claude-opus-5-5')
        b.update(effort='high')
        self.write(SID_B, [b], mtime=NOW - 30)
        self.assertEqual(self.flags(sid=SID_B)['effort'], 'high')

    def test_ultracode_from_the_start_of_a_long_chat(self):
        a = assistant('2026-10-01T00:00:05Z', 'ok')
        a.update(effort='xhigh')
        pad = [assistant('2026-10-01T00:00:06Z', 'x' * 400_000) for _ in range(12)]   # pushes the marker out of the tail
        self.write(SID_A, [entry('system', '2026-10-01T00:00:02Z', subtype='x', content={'type': 'ultra_effort_enter'})] + pad + [a])
        self.assertEqual(self.flags()['effort'], 'ultracode')

    def test_background_launch_flags_come_from_the_roster_and_are_remembered(self):
        self.write(SID_A, [assistant('2026-10-01T00:00:05Z', 'ok')])
        (self.cdir / 'daemon').mkdir()
        roster = {'workers': {'aaaaaaaa': {'sessionId': SID_A, 'dispatch': {'sessionId': SID_A, 'launch': {
            'mode': 'prompt', 'args': ['--session-id', SID_A, '--remote-control', '--chrome', '--name', 'x', 'hi']}}}}}
        (self.cdir / 'daemon' / 'roster.json').write_text(json.dumps(roster))
        self.assertTrue(self.flags()['chrome'])
        (self.cdir / 'daemon' / 'roster.json').write_text('{"workers": {}}')   # it finished; the chip stays
        self.assertTrue(self.flags()['chrome'])

    def test_terminal_flags_come_from_the_command_line_once(self):
        self.write(SID_A, [assistant('2026-10-01T00:00:05Z', 'ok')])
        asked = []
        self.sb._cmdlines = lambda pids: asked.append(sorted(pids)) or {4242: ['claude', '--resume', '--chrome']}
        agents = [{'sessionId': SID_A, 'kind': 'interactive', 'status': 'idle', 'pid': 4242, 'startedAt': 5}]
        self.assertTrue(self.flags(agents)['chrome'])
        self.assertTrue(self.flags(agents)['chrome'])
        self.assertEqual(asked, [[4242]])                                     # remembered, not asked again

    def test_a_broken_roster_or_settings_never_breaks_the_list(self):
        self.write(SID_A, [assistant('2026-10-01T00:00:05Z', 'ok')])
        (self.cdir / 'daemon').mkdir()
        (self.cdir / 'daemon' / 'roster.json').write_text('{"workers": [1, 2]}')
        (self.cdir / 'settings.json').write_text('[]')
        self.assertIsInstance(self.flags(), dict)


class ReplacedLinkTest(RpcBase):
    def test_a_replaced_chat_keeps_its_claude_link(self):
        # There is no command that archives a chat in the Claude apps; the page links to it so it is one tap away.
        self.transcript(SID_A, str(self.root), NOW - 600, title='Old one')
        self.sb.mark_replaced(SID_A, 'fresh', NOW - 300)
        r = self.sb.rpc({'verb': 'list'}, NOW, agents=[])
        self.assertEqual(r['replaced'][0]['url'], 'https://claude.ai/code/session_XYZ123')


if __name__ == '__main__':
    unittest.main()
