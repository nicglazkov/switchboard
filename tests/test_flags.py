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
        self.sb._cmdlines = lambda pids, loose=None: asked.append(sorted(pids)) or {4242: ['claude', '--resume', '--chrome']}
        agents = [{'sessionId': SID_A, 'kind': 'interactive', 'status': 'idle', 'pid': 4242, 'startedAt': 5}]
        self.assertTrue(self.flags(agents)['chrome'])
        self.assertTrue(self.flags(agents)['chrome'])
        # remembered, not asked again (the list also asks about this machine's own stray processes: not this chat)
        self.assertEqual([a for a in asked if 4242 in a], [[4242]])

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


from tests.test_jobs import JobBase  # noqa: E402


class CarriedFlagsTest(JobBase):
    """A chat that is resumed, started fresh, compacted or moved keeps the flags it was launched with."""
    def old_chat(self, args, rows=()):
        rows = [dict(user('2026-10-01T00:00:01Z', 'look at canvas'), cwd=str(self.root / 'proj')),
                {'type': 'assistant', 'timestamp': '2026-10-01T00:00:02Z', 'effort': 'medium',
                 'message': {'model': 'claude-opus-4-8', 'content': [{'type': 'text', 'text': 'ok'}], 'usage': {'input_tokens': 9}}},
                *rows, {'type': 'ai-title', 'aiTitle': 'Canvas', 'sessionId': SID_A}]
        p = write_transcript(self.cdir / 'projects' / 'p1', rows, name=f'{SID_A}.jsonl')
        os.utime(p, (NOW - 60, NOW - 60))
        (self.cdir / 'daemon').mkdir(exist_ok=True)
        roster = {'workers': {'aaaaaaaa': {'sessionId': SID_A, 'dispatch': {'sessionId': SID_A, 'launch': {
            'mode': 'prompt', 'args': ['--session-id', SID_A, '--remote-control'] + args + ['--name', 'x', 'hi']}}}}}
        (self.cdir / 'daemon' / 'roster.json').write_text(json.dumps(roster))
        self.sb.claude_flags = lambda refresh=False, runner=None: set(self.sb.START_FLAGS)
        self.sb.claude_perms = lambda: ['acceptEdits', 'auto', 'bypassPermissions', 'manual', 'dontAsk', 'plan']
        self.sb.rpc({'verb': 'list'}, NOW, agents=[])
        return p

    ARGS = ['--permission-mode', 'acceptEdits', '--add-dir', 'PROJ', '--add-dir', 'GONE', '--agent', 'reviewer',
            '--append-system-prompt', 'Be brief, and kind', '--worktree', 'fix', '--model', 'claude-opus-4-8', '--no-chrome']

    def args(self):
        (self.root / 'gone').mkdir(exist_ok=True)
        return [{'PROJ': str(self.root / 'proj'), 'GONE': str(self.root / 'gone')}.get(a, a) for a in self.ARGS]

    def test_resume_and_fresh_keep_what_the_chat_ran_with(self):
        for verb in ('resume', 'fresh'):
            with self.subTest(verb=verb):
                self.launched.clear()
                self.old_chat(self.args(), [entry('permission-mode', '2026-10-01T00:00:03Z', permissionMode='plan')])
                (self.root / 'gone').rmdir()  # a folder that is no longer there is left out, the rest still goes
                self.job({'verb': verb, 'sid': SID_A})
                self.drain()
                a = self.launched[0]['argv']
                self.assertIn('--permission-mode=plan', a)  # the mode it was last in, not the one it started in
                self.assertIn(f'--add-dir={(self.root / "proj").resolve()}', a)
                self.assertIn('--agent=reviewer', a)
                self.assertIn('--append-system-prompt=Be brief, and kind', a)
                self.assertIn('--no-chrome', a)
                self.assertEqual(a[a.index('--model') + 1], 'claude-opus-4-8')  # the pinned version, not "opus"
                self.assertFalse([x for x in a if x.startswith('--worktree') or 'gone' in x], a)

    def test_the_brief_for_another_machine_carries_them_too(self):
        self.old_chat(self.args())
        self.sb.handoff_path(SID_A).write_text('NOTES')
        c = self.sb.rpc({'verb': 'handoff_text', 'sid': SID_A}, NOW, agents=[])['carry']
        self.assertEqual((c['effort'], c['model']), ('medium', 'claude-opus-4-8'))
        self.assertEqual(c['flags'], [['--add-dir', 'proj'], ['--add-dir', 'gone'], ['--agent', 'reviewer'],
                                      ['--append-system-prompt', 'Be brief, and kind'], ['--no-chrome'],
                                      ['--permission-mode', 'acceptEdits']])

    def test_the_default_model_and_mode_are_not_named(self):
        p = self.old_chat([], [entry('permission-mode', '2026-10-01T00:00:03Z', permissionMode='default')])
        self.assertEqual(self.sb.carry_over(p, 'claude-opus-4-8', SID_A), {'effort': 'medium'})  # --model opus would drop [1m]
        (self.cdir / 'settings.json').write_text(json.dumps({'model': 'sonnet', 'permissions': {'defaultMode': 'auto'}}))
        self.assertEqual(self.sb.carry_over(p, 'claude-opus-4-8', SID_A),
                         {'effort': 'medium', 'model': 'opus', 'flags': [['--permission-mode', 'default']]})

    def test_a_model_switched_since_the_launch_wins(self):
        self.assertEqual(self.sb.model_arg('claude-opus-4-8', 'opus[1m]'), 'opus[1m]')
        self.assertEqual(self.sb.model_arg('claude-sonnet-5-5', 'claude-opus-4-8'), 'sonnet')
        self.assertIsNone(self.sb.model_arg('claude-opus-4-8', 'sonnet'))
        self.assertIsNone(self.sb.model_arg('claude-opus-4-8', '--evil'))

    def test_effort_is_the_chats_own_not_a_word_in_a_tool_call(self):
        tool = {'type': 'assistant', 'timestamp': '2026-10-01T00:00:04Z',
                'message': {'model': 'claude-opus-4-8', 'usage': {'input_tokens': 9},
                            'content': [{'type': 'tool_use', 'name': 'Agent', 'input': {'effort': 'max', 'prompt': 'x'}}]}}
        p = self.old_chat([], [tool])
        self.assertEqual(self.sb.carry_over(p, 'claude-opus-4-8', SID_A)['effort'], 'medium')

    def test_asks_first_is_only_carried_under_a_name_this_claude_takes(self):
        self.sb.claude_flags = lambda refresh=False, runner=None: set(self.sb.START_FLAGS)
        self.sb.claude_perms = lambda: ['acceptEdits', 'manual', 'plan']
        self.assertEqual(self.sb.soft_flags([['--permission-mode', 'default'], ['--evil'], ['--brief']]),
                         ['--permission-mode=manual', '--brief'])
        self.sb.claude_perms = lambda: []
        self.assertEqual(self.sb.soft_flags([['--permission-mode', 'manual']]), [])
        self.assertEqual(self.sb.soft_flags([['--permission-mode', 'plan']]), ['--permission-mode=plan'])

    def test_a_value_an_older_helper_cut_short_is_never_sent(self):
        cut = {'launch': {}, 'all': [['--append-system-prompt', 'x' * 200], ['--agent', 'reviewer']], 'other': ['--mystery']}
        self.assertEqual(self.sb.whole_flags(cut), ([['--agent', 'reviewer']], ['--mystery', '--append-system-prompt']))
        self.assertEqual(self.sb.whole_flags(dict(cut, v=self.sb.FLAG_RECORD))[0], cut['all'])

    def test_a_record_cut_at_its_spaces_sends_back_no_value_that_could_hold_one(self):
        # An older helper, and `ps` today, cut "--append-system-prompt Always answer in French" and
        # "--tools Bash Read" at the spaces: the short first word looks whole and is not.
        fl = [['--append-system-prompt', 'Always'], ['--tools', 'Bash'], ['--allowed-tools', 'Read'], ['--disallowed-tools', 'Write'],
              ['--add-dir', 'my'], ['--permission-mode', 'plan'], ['--agent', 'reviewer'], ['--model', 'claude-opus-4-8'],
              ['--fallback-model', 'sonnet'], ['--autocompact', '200k'], ['--setting-sources', 'user,project'], ['--brief']]
        safe = fl[5:]
        wordy = ['--append-system-prompt', '--tools', '--allowed-tools', '--disallowed-tools', '--add-dir']
        for rec in ({'all': fl}, {'all': fl, 'v': 1}, {'all': fl, 'v': self.sb.FLAG_RECORD, 'split': True, 'from': 'ps'}):
            self.assertEqual(self.sb.whole_flags(dict(rec, other=['--ide'])), (safe, ['--ide'] + wordy), rec.get('v'))
        self.assertEqual(self.sb.whole_flags({'all': fl, 'v': self.sb.FLAG_RECORD, 'from': 'ps'})[0], fl, 'read whole: sent whole')
        # every flag not on the list takes one word by its own pattern, so it cannot have been cut
        for name, spec in self.sb.START_FLAGS.items():
            if name not in self.sb.WORDY_FLAGS and isinstance(spec, str):
                self.assertNotIn(' ', spec, name)
                self.assertNotEqual(spec, 'folder', name)

    def test_a_command_line_read_through_ps_is_marked(self):
        agents = [{'kind': 'interactive', 'sessionId': SID_A, 'pid': 4242, 'startedAt': 5}]

        def ps(pids, loose=None):
            loose.add(4242)
            return {4242: ['claude', '--append-system-prompt', 'Always', 'answer', 'in', 'French', '--agent', 'reviewer']}
        self.sb._cmdlines = ps
        self.sb._roster_args = lambda: {}
        full = {}
        self.sb.launch_flags(agents, NOW, full)
        rec = json.loads((self.sb.paths()['state'] / 'flags.json').read_text())[SID_A]
        self.assertEqual((rec['v'], rec['split'], rec['from']), (self.sb.FLAG_RECORD, True, 'ps'))
        self.assertEqual(full[SID_A]['flags'], [['--agent', 'reviewer']])
        self.assertEqual(full[SID_A]['other'], ['--append-system-prompt'])
        asked = []
        self.sb._cmdlines = lambda pids, loose=None: asked.append(pids) or {}
        self.sb.launch_flags(agents, NOW + 15, {})
        self.assertEqual(asked, [], 'and it is not asked again at every poll')

    def test_an_old_record_carries_no_half_value_into_a_resume(self):
        p = self.old_chat([])
        (self.cdir / 'daemon' / 'roster.json').unlink()
        flags = self.sb.paths()['state'] / 'flags.json'
        flags.write_text(json.dumps({SID_A: {'launch': {}, 'from': 'ps', 'all': [
            ['--append-system-prompt', 'Always'], ['--tools', 'Bash'], ['--agent', 'reviewer']], 'other': []}}))
        self.assertEqual(self.sb.carry_over(p, 'claude-opus-4-8', SID_A)['flags'], [['--agent', 'reviewer']])

    def test_bypass_is_never_carried_from_the_transcript_alone(self):
        last = [entry('permission-mode', '2026-10-01T00:00:03Z', permissionMode='bypassPermissions')]
        p = self.old_chat([], last)
        self.assertNotIn('flags', self.sb.carry_over(p, 'claude-opus-4-8', SID_A))
        p = self.old_chat(['--permission-mode', 'acceptEdits'], last)   # back to what it was launched with
        self.assertEqual(self.sb.carry_over(p, 'claude-opus-4-8', SID_A)['flags'], [['--permission-mode', 'acceptEdits']])
        for args in (['--permission-mode', 'bypassPermissions'], ['--dangerously-skip-permissions'],
                     ['--allow-dangerously-skip-permissions']):
            p = self.old_chat(args, last)                               # launched so: it stays so
            self.assertEqual(self.sb.carry_over(p, 'claude-opus-4-8', SID_A)['flags'], [['--permission-mode', 'bypassPermissions']], args)

    def test_help_that_cannot_be_read_carries_no_flag(self):
        self.sb.claude_flags = lambda refresh=False, runner=None: None
        self.assertEqual(self.sb.soft_flags([['--permission-mode', 'plan'], ['--brief']]), [])

    def test_a_carried_flag_claude_refuses_never_keeps_the_chat_from_coming_back(self):
        for verb in ('resume', 'fresh'):
            with self.subTest(verb=verb):
                self.launched.clear()
                self.old_chat(['--agent', 'reviewer', '--brief'])

                def run(argv, cwd, env):
                    self.launched.append({'argv': argv})
                    if any(a.startswith('--agent') for a in argv):
                        return 1, "error: unknown option '--agent'\n"
                    return 0, '  claude stop 3022c163    stop this session\n'
                jid = self.job({'verb': verb, 'sid': SID_A})
                self.drain(runner=run)
                self.assertEqual(len(self.launched), 2, 'once with them, once without, and never a third')
                self.assertIn('--agent=reviewer', self.launched[0]['argv'])
                again = self.launched[1]['argv']
                self.assertFalse([a for a in again if a.startswith(('--agent', '--brief'))], again)
                self.assertIn('--resume' if verb == 'resume' else '--bg', again)
                r = self.sb.rpc({'verb': 'job', 'id': jid}, NOW + 9, agents=[])
                self.assertEqual((r['state'], r['dropped_flags']), ('done', ['--agent', '--brief']))
                self.assertIn('Started without --agent, --brief', r['note'])
                self.assertIn('trying again without the carried --agent --brief', self.sb.paths()['log'].read_text())

    def test_only_a_clear_refusal_is_tried_again(self):
        for rc, out, why in ((1, 'boom\n  claude stop 3022c163    stop this session\n', 'it named a chat: something started'),
                             (0, 'no id here\n', 'it did not fail')):
            self.launched.clear()
            self.old_chat(['--agent', 'reviewer'])
            jid = self.job({'verb': 'resume', 'sid': SID_A})
            self.drain(runner=self.runner(rc, out))
            self.assertEqual(len(self.launched), 1, why)
            self.assertEqual(self.sb.rpc({'verb': 'job', 'id': jid}, NOW + 9, agents=[])['state'], 'error')
        # slow to fail: it may have started something
        self.launched.clear()
        self.old_chat(['--agent', 'reviewer'])
        self.sb.CARRY_RETRY_S = 0
        self.job({'verb': 'resume', 'sid': SID_A})
        self.drain(runner=self.runner(1, 'boom\n'))
        self.assertEqual(len(self.launched), 1)

    def test_flags_the_user_just_picked_are_not_dropped_on_a_refusal(self):
        self.old_chat([])
        self.sb.rpc({'verb': 'list'}, NOW, agents=[])
        jid = self.job({'verb': 'resume', 'sid': SID_A, 'relaunch': True, 'flags': [['--brief']]})
        self.drain(runner=self.runner(1, "error: unknown option '--brief'\n"))
        self.assertEqual(len(self.launched), 1)
        r = self.sb.rpc({'verb': 'job', 'id': jid}, NOW + 9, agents=[])
        self.assertEqual(r['state'], 'error')
        self.assertIn('--brief', r['error'])

    def test_long_values_are_kept_whole(self):
        text = 'Be brief. ' * 150
        self.assertEqual(self.sb.flag_list(['--append-system-prompt', text])[0], [['--append-system-prompt', text]])
        self.assertEqual(self.sb.flag_list(['--append-system-prompt', 'x' * 2001]), ([], ['--append-system-prompt']))

    def test_the_list_names_the_extra_folders(self):
        self.old_chat(['--add-dir', str(self.root / 'proj')])
        s = self.sb.rpc({'verb': 'list'}, NOW, agents=[])['sessions'][0]
        self.assertEqual(s['flags']['dirs'], ['proj'])


class LaunchWordsTest(RpcBase):
    def test_a_bare_worktree_cannot_take_the_message(self):
        argv = self.sb.launch_argv({'flags': ['--worktree', '--permission-mode=plan'], 'prompt': 'do it'}, exe='claude')
        self.assertEqual(argv, ['claude', '--bg', '--worktree', '--remote-control', '--permission-mode=plan', 'do it'])
        self.assertEqual(self.sb.check_flags([['--worktree']]), ['--worktree'])
        self.assertEqual(self.sb.flag_list(['-w', '--chrome'])[0], [['--worktree'], ['--chrome']])

    def test_ultracode_keeps_the_model_it_is_given(self):
        a = self.sb.launch_argv({'effort': 'ultracode', 'model': 'fable', 'prompt': 'x'}, exe='claude')
        self.assertEqual((a[a.index('--model') + 1], a[a.index('--effort') + 1]), ('fable', 'ultracode'))
        a = self.sb.launch_argv({'mode': 'ultracode', 'prompt': 'x'}, exe='claude')  # the New sheet's mode: Opus
        self.assertEqual((a[a.index('--model') + 1], a[a.index('--effort') + 1]), ('opus', 'ultracode'))
        self.assertNotIn('--model', self.sb.launch_argv({'effort': 'ultracode', 'prompt': 'x'}, exe='claude'))

    def test_permission_modes_go_by_the_name_this_claude_lists(self):
        new, old = ['acceptEdits', 'manual', 'plan'], ['acceptEdits', 'default', 'plan']
        self.assertEqual(self.sb.check_flags([['--permission-mode', 'default']], perms=new), ['--permission-mode=manual'])
        self.assertEqual(self.sb.check_flags([['--permission-mode', 'manual']], perms=old), ['--permission-mode=default'])
        self.assertEqual(self.sb.check_flags([['--permission-mode', 'manual']], perms=new), ['--permission-mode=manual'])
        with self.assertRaises(ValueError):
            self.sb.check_flags([['--permission-mode', 'auto']], perms=new)
        self.assertEqual(self.sb.check_flags([['--permission-mode', 'auto']], perms=None), ['--permission-mode=auto'])

    HELP = ('Usage: claude [options]\n\nOptions:\n'
            '  --add-dir <directories...>            Additional directories\n'
            '  --allowedTools, --allowed-tools <tools...>  Tools to allow\n'
            '  --permission-mode <mode>              Permission mode to use for the session (choices: "acceptEdits",\n'
            '                                        "auto", "manual", "plan")\n'
            '  --restricted                          Like --tools "" with no --brief and no --add-dir\n'
            '  -w, --worktree [name]                 Create a new git worktree\n')

    def test_help_only_counts_a_flag_where_it_is_defined(self):
        self.assertEqual(self.sb.help_flags(self.HELP),
                         ['--add-dir', '--allowed-tools', '--allowedTools', '--permission-mode', '--restricted', '--worktree'])
        self.assertEqual(self.sb.help_perm_modes(self.HELP), ['acceptEdits', 'auto', 'manual', 'plan'])
        self.assertEqual(self.sb.help_perm_modes('Options:\n  --chrome  x\n'), [])
        self.assertEqual((self.sb.ask_mode(['manual', 'default']), self.sb.ask_mode(['default']), self.sb.ask_mode([])),
                         ('manual', 'default', None))

    def test_a_macs_argument_block_gives_the_words_as_they_were(self):
        words = [b'claude', b'--append-system-prompt', b'Be brief, and kind', b'--add-dir', b'/Users/you/My Stuff', b'']
        raw = len(words).to_bytes(4, sys.byteorder) + b'/usr/local/bin/claude\0\0\0\0' + b'\0'.join(words) + b'\0HOME=/Users/you\0'
        self.assertEqual(self.sb.parse_procargs2(raw), [w.decode() for w in words])
        self.assertIsNone(self.sb.parse_procargs2(raw[:40]))   # cut short
        self.assertIsNone(self.sb.parse_procargs2(b'\0\0'))

    @unittest.skipUnless(sys.platform == 'darwin', 'kern.procargs2 is a Mac thing')
    def test_a_mac_is_asked_for_exact_words_before_ps(self):
        import subprocess
        # sleep itself will not do: it takes 'a b' for a bad number and is gone before it can be looked at
        words = [sys.executable, '-c', 'import time; print("up", flush=True); time.sleep(30)', 'a b']
        p = subprocess.Popen(words, stdout=subprocess.PIPE)
        try:
            self.assertEqual(p.stdout.readline(), b'up\n')  # it has started, and with these words
            got = self.sb._cmdlines([p.pid])
            self.assertEqual(got[p.pid][1:], words[1:])
            self.assertEqual(os.path.basename(got[p.pid][0]).lower()[:6], 'python')
        finally:
            p.kill()
            p.wait()
            p.stdout.close()
