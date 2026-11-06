import json
import sys, os, unittest
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

    def test_a_relaunched_chat_keeps_its_flags(self):
        # Claude Code keeps a resumed worker's command line in flagArgs, its message after "--" (2.1.319)
        self.write(SID_A, [assistant('2026-10-01T00:00:05Z', 'ok')])
        (self.cdir / 'daemon').mkdir()
        roster = {'workers': {'aaaaaaaa': {'sessionId': SID_A, 'dispatch': {'sessionId': SID_A, 'launch': {
            'mode': 'resume', 'sessionId': SID_A, 'fork': False,
            'flagArgs': ['--remote-control', '--name', 'x', '--effort', 'high', '--chrome', '--', '--model is not a flag here']},
            'respawnFlags': ['--remote-control', '--name', 'x', '--effort', 'high', '--chrome', '--', 'hi']}}}}
        (self.cdir / 'daemon' / 'roster.json').write_text(json.dumps(roster))
        f = self.flags()
        self.assertTrue(f['chrome'])
        self.assertEqual(f['effort'], 'high')
        self.assertNotIn('model', f)
        r = self.sb.rpc({'verb': 'list'}, NOW, agents=[])
        s = next(s for s in r['sessions'] if s['sid'] == SID_A)
        self.assertTrue(s['now']['chrome'])
        self.assertIn(['--chrome'], s['launch']['flags'])                      # what the next relaunch carries over
        self.assertEqual(s['launch']['flags'].count(['--chrome']), 1, 'respawnFlags repeats them: read once')

    def test_chrome_is_unknown_until_a_relaunched_copy_has_had_a_turn(self):
        # the old copy said "no Chrome"; the copy running now was started with --chrome and has not been asked yet
        delta = entry('attachment', '2026-10-01T00:00:03Z',
                      attachment={'type': 'mcp_instructions_delta', 'addedNames': ['filmcraft']})
        self.write(SID_A, [delta, assistant('2026-10-01T00:00:05Z', 'ok')])
        (self.cdir / 'daemon').mkdir()
        roster = {'workers': {'aaaaaaaa': {'sessionId': SID_A, 'dispatch': {'sessionId': SID_A, 'launch': {
            'mode': 'resume', 'flagArgs': ['--remote-control', '--chrome']}}}}}
        (self.cdir / 'daemon' / 'roster.json').write_text(json.dumps(roster))
        import datetime as dt
        later = dt.datetime(2026, 10, 1, 1, tzinfo=dt.timezone.utc).timestamp() * 1000
        running = [{'sessionId': SID_A, 'kind': 'background', 'status': 'idle', 'id': 'aaaaaaaa', 'startedAt': later}]
        r = self.sb.rpc({'verb': 'list'}, NOW, agents=running)
        s = next(s for s in r['sessions'] if s['sid'] == SID_A)
        self.assertTrue(s['flags'].get('chrome'))
        self.assertNotIn('chrome_missing', s)
        # the same chat whose running copy said "no Chrome" itself is still missing it
        early = [dict(running[0], startedAt=later / 1000 - 3600 * 2)]
        r = self.sb.rpc({'verb': 'list'}, NOW, agents=early)
        s = next(s for s in r['sessions'] if s['sid'] == SID_A)
        self.assertTrue(s.get('chrome_missing'))

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


class _Sys:
    """The real sys with a platform of its own: setting sb.sys.platform must not leak into other tests."""
    def __init__(self, real):
        self.__dict__['_real'] = real

    def __getattr__(self, n):
        return getattr(self._real, n)


class ComputerUseTest(RpcBase):
    def setUp(self):
        super().setUp()
        self.sb.sys = _Sys(sys)
        self.sb.sys.platform = 'darwin'
        self.sb._git_top = lambda cwd: None
        self.folder = str(self.root / 'proj')

    def cj(self):
        return json.loads(self.cjson.read_text())

    def test_on_and_off_for_a_folder_like_mcp_does(self):
        self.cjson.write_text(json.dumps({'projects': {self.folder: {'enabledMcpServers': ['other'],
                                                                     'disabledMcpServers': ['computer-use']}}, 'keep': 1}))
        self.assertTrue(self.sb.set_computer_use(self.folder, True))
        e = self.cj()['projects'][self.folder]
        self.assertEqual(e['enabledMcpServers'], ['other', 'computer-use'])
        self.assertEqual(e['disabledMcpServers'], [])
        self.assertEqual(self.cj()['keep'], 1)
        self.assertTrue(self.sb.computer_use_on(self.folder))
        self.assertFalse(self.sb.set_computer_use(self.folder, True), 'already on: nothing written')
        self.assertTrue((self.tmp / 'claude.json.bak-sb-computer-use').exists())
        self.assertTrue(self.sb.set_computer_use(self.folder, False))
        self.assertEqual(self.cj()['projects'][self.folder]['enabledMcpServers'], ['other'])
        self.assertFalse(self.sb.computer_use_on(self.folder))

    def test_linux_has_none(self):
        self.cjson.write_text('{}')
        self.sb.sys.platform = 'linux'
        self.assertFalse(self.sb.set_computer_use(self.folder, True))
        self.assertEqual(self.cj(), {})
        self.assertFalse(self.sb._computer_use()['supported'])

    def test_windows_gets_windows_mcp_in_the_folders_own_servers(self):
        sb = self.sb
        sb.sys.platform, sb.IS_WIN = 'win32', True
        installs = []
        sb._win_cu_install = lambda: installs.append(1) or r'C:\u\.local\bin\windows-mcp.exe'
        sb._win_desktop = lambda: 'active'
        self.cjson.write_text(json.dumps({'projects': {self.folder: {'mcpServers': {'mine': {'command': 'x'}},
                                                                     'disabledMcpServers': ['windows-mcp']}}}))
        self.assertTrue(sb.set_computer_use(self.folder, True))
        e = self.cj()['projects'][self.folder]
        self.assertEqual(set(e['mcpServers']), {'mine', 'windows-mcp'})
        self.assertEqual(e['mcpServers']['windows-mcp']['args'][0], 'serve')
        self.assertIn('PowerShell', e['mcpServers']['windows-mcp']['args'][2], 'Claude Code has its own shell')
        self.assertEqual(e['disabledMcpServers'], [])
        self.assertTrue(sb.computer_use_on(self.folder))
        self.assertFalse(sb.set_computer_use(self.folder, True), 'already on')
        self.assertTrue(sb.set_computer_use(self.folder, False))
        self.assertEqual(self.cj()['projects'][self.folder]['mcpServers'], {'mine': {'command': 'x'}})
        self.assertFalse(sb.computer_use_on(self.folder))
        sb._win_cu_install = lambda: None
        with self.assertRaises(ValueError):
            sb.set_computer_use(self.folder, True)

    def test_windows_says_when_its_desktop_is_disconnected(self):
        sb = self.sb
        sb.sys.platform, sb.IS_WIN = 'win32', True
        sb._win_cu_exe = lambda: 'windows-mcp.exe'
        real = sb._win_desktop
        sb._win_desktop = lambda: 'active'
        self.assertEqual((sb._computer_use()['supported'], sb._computer_use()['note']), (True, None))
        sb._win_desktop = lambda: 'disconnected'
        self.assertIn('disconnected', sb._computer_use()['note'])
        head = ' USERNAME              SESSIONNAME        ID  STATE   IDLE TIME  LOGON TIME\n'
        self.addCleanup(os.environ.pop, 'USERNAME', None) if 'USERNAME' not in os.environ else None
        sb.os.environ['USERNAME'] = 'live'
        sb._out = lambda argv, timeout=0: head + '>live                                      1  Disc        10:25  10/16/2026 2:54 PM\n'
        self.assertEqual(real(), 'disconnected')
        sb._out = lambda argv, timeout=0: head + '>live                  console             1  Active          .  10/16/2026 2:54 PM\n'
        self.assertEqual(real(), 'active')
        sb._out = lambda argv, timeout=0: ''
        self.assertIsNone(real())

    def test_a_half_written_claude_json_is_left_alone(self):
        self.cjson.write_text('{"projects": ')
        self.assertFalse(self.sb.set_computer_use(self.folder, True))
        self.assertEqual(self.cjson.read_text(), '{"projects": ')

    def test_the_list_shows_it_on_the_chat(self):
        rows = [entry('bridge-session', '2026-10-01T00:00:00Z', bridgeSessionId='cse_X', sessionId=SID_A),
                dict(user('2026-10-01T00:00:01Z', 'hi'), cwd=self.folder), assistant('2026-10-01T00:00:05Z', 'ok')]
        p = write_transcript(self.cdir / 'projects' / 'p1', rows, name=f'{SID_A}.jsonl')
        os.utime(p, (NOW - 60, NOW - 60))
        self.cjson.write_text(json.dumps({'projects': {self.folder: {'enabledMcpServers': ['computer-use']}}}))
        s = next(s for s in self.sb.rpc({'verb': 'list'}, NOW, agents=[])['sessions'] if s['sid'] == SID_A)
        self.assertTrue(s['flags']['computer'])
        self.assertTrue(s['now']['computer'])
        self.cjson.write_text('{}')
        s = next(s for s in self.sb.rpc({'verb': 'list'}, NOW, agents=[])['sessions'] if s['sid'] == SID_A)
        self.assertNotIn('computer', s['flags'])

    def test_one_call_turns_it_on_for_a_running_chats_folder(self):
        rows = [dict(user('2026-10-01T00:00:01Z', 'hi'), cwd=self.folder), assistant('2026-10-01T00:00:05Z', 'ok')]
        p = write_transcript(self.cdir / 'projects' / 'p1', rows, name=f'{SID_A}.jsonl')
        os.utime(p, (NOW - 60, NOW - 60))
        self.cjson.write_text('{}')
        r = self.sb.rpc({'verb': 'computer_use', 'sid': SID_A, 'on': True}, NOW, agents=[])
        self.assertEqual((r['ok'], r['on'], r['changed']), (True, True, True))
        self.assertTrue(self.sb.computer_use_on(self.folder))
        r = self.sb.rpc({'verb': 'computer_use', 'sid': SID_A, 'on': False}, NOW, agents=[])
        self.assertFalse(r['on'])
        self.sb.sys.platform = 'win32'
        r = self.sb.rpc({'verb': 'computer_use', 'sid': SID_A, 'on': True}, NOW, agents=[])
        self.assertFalse(r['ok'])

    def test_the_gate_off_says_why(self):
        self.cjson.write_text(json.dumps({'cachedGrowthBookFeatures': {'tengu_malort_pedway': {'enabled': False}}}))
        c = self.sb._computer_use()
        self.assertFalse(c['supported'])
        self.assertIn('Pro, Max or Ultra', c['why'])
        self.assertIn('computer_use', self.sb.AGENT_FEATURES)


class ReplacedLinkTest(RpcBase):
    def test_a_replaced_chat_keeps_its_claude_link(self):
        # There is no command that archives a chat in the Claude apps; the page links to it so it is one tap away.
        self.transcript(SID_A, str(self.root), NOW - 600, title='Old one')
        self.sb.mark_replaced(SID_A, 'fresh', NOW - 300)
        r = self.sb.rpc({'verb': 'list'}, NOW, agents=[])
        self.assertEqual(r['replaced'][0]['url'], 'https://claude.ai/code/session_XYZ123')


if __name__ == '__main__':
    unittest.main()
