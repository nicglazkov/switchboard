import json, os, plistlib, tempfile, time, unittest
from pathlib import Path
from unittest import mock
from tests.test_rpc import RpcBase, NOW, SID_A
from tests.helpers import write_transcript, user, assistant


class JobBase(RpcBase):
    def setUp(self):
        super().setUp()
        self.triggered = []
        self.sb.trigger_launcher = lambda: self.triggered.append(1)
        self.launched = []

        def no_claude(*a, **kw):  # titles fall back to the message's first words; never a real claude -p here
            raise RuntimeError('no claude in tests')
        self.sb.run_claude_opus = no_claude

    def runner(self, rc=0, out='  claude stop 3022c163    stop this session\n'):
        def run(argv, cwd, env):
            self.launched.append({'argv': argv, 'cwd': cwd, 'env': env})
            return rc, out
        return run

    def job(self, req):
        r = self.sb.rpc(req, NOW, agents=[])
        self.assertTrue(r['ok'], r)
        return r['job']

    def drain(self, **kw):
        self.sb.launch_queue(runner=kw.get('runner') or self.runner(), find_url=lambda bg: 'https://claude.ai/code/session_X',
                             make_handoff=kw.get('make_handoff') or (lambda tp, sid: self.sb.handoff_path(sid).write_text('HANDOFF ' + sid)),
                             now=NOW + 5, live=kw.get('live') or (lambda bg: 'ready'))


class LaunchArgvTest(JobBase):
    def test_start_argv_is_a_list_with_marker_prompt(self):
        argv = self.sb.launch_argv({'name': 'My task', 'model': 'sonnet', 'prompt': '-rf "quoted"; rm x', 'id': 'j1'}, exe='claude')
        self.assertEqual(argv[:4], ['claude', '--bg', '--remote-control', '--name'])
        self.assertIn('My task', argv)
        self.assertEqual(argv[argv.index('--model') + 1], 'sonnet')
        self.assertEqual(argv[-1], ' -rf "quoted"; rm x')   # leading dash cannot become a flag

    def test_resume_argv(self):
        argv = self.sb.launch_argv({'name': 'x', 'resume': SID_A, 'id': 'j1'}, exe='claude')
        self.assertEqual(argv[argv.index('--resume') + 1], SID_A)

    def test_parse_bg_id(self):
        self.assertEqual(self.sb.parse_bg_id('\x1b[2m  claude logs 03bf7f41  show\n  claude stop 03bf7f41'), '03bf7f41')
        self.assertIsNone(self.sb.parse_bg_id('Workspace not trusted.'))
        # resuming a chat that runs already: Claude names the original first, then the copy it started
        note = ('note: session 76b2f223 is already running in the background, so this started a copy as 68cd24c4. '
                '`claude attach 76b2f223` opens the original.\n  claude attach 68cd24c4  open it\n')
        self.assertEqual(self.sb.parse_bg_id(note), '68cd24c4')


class StartJobTest(JobBase):
    def test_start_queues_triggers_and_launches(self):
        jid = self.job({'verb': 'start', 'folder': 'proj', 'prompt': 'hello', 'model': 'haiku'})
        self.assertEqual(self.triggered, [1])
        self.assertEqual(self.sb.rpc({'verb': 'job', 'id': jid}, NOW, agents=[])['state'], 'queued')
        self.drain()
        r = self.sb.rpc({'verb': 'job', 'id': jid}, NOW + 6, agents=[])
        self.assertEqual(r['state'], 'done')
        self.assertEqual(r['bg_id'], '3022c163')
        self.assertEqual(r['url'], 'https://claude.ai/code/session_X')
        l = self.launched[0]
        self.assertEqual(Path(l['cwd']), (self.root / 'proj').resolve())
        self.assertNotIn('SB_INTERNAL', l['env'])
        self.assertNotIn('ANTHROPIC_API_KEY', l['env'])
        self.assertEqual(l['argv'][-1], 'hello')

    def test_start_rejects_bad_input(self):
        for req in [{'verb': 'start', 'folder': '../x'}, {'verb': 'start', 'folder': 'nope'},
                    {'verb': 'start', 'folder': 'proj', 'model': 'gpt'}, {'verb': 'start', 'folder': 'proj', 'name': 'a;b'}]:
            self.assertFalse(self.sb.rpc(req, NOW, agents=[])['ok'], req)
        self.assertEqual(self.triggered, [])

    def test_start_at_root_allowed(self):
        self.assertTrue(self.sb.rpc({'verb': 'start', 'folder': ''}, NOW, agents=[])['ok'])
        with mock.patch.object(self.sb, 'machine_name', lambda: 'laptop'):
            self.assertTrue(self.sb.rpc({'verb': 'start', 'folder': ''}, NOW, agents=[])['ok'])

    def test_launch_failure_is_readable(self):
        jid = self.job({'verb': 'start', 'folder': 'proj'})
        self.drain(runner=self.runner(rc=1, out='\x1b[31mWorkspace not trusted.\x1b[0m The home directory...'))
        r = self.sb.rpc({'verb': 'job', 'id': jid}, NOW + 6, agents=[])
        self.assertEqual(r['state'], 'error')
        self.assertIn('Workspace not trusted', r['error'])
        self.assertNotIn('\x1b', r['error'])

    def test_unpicked_job_times_out_readably(self):
        with mock.patch.object(self.sb, 'IS_WIN', True):
            jid = self.job({'verb': 'start', 'folder': 'proj'})
            r = self.sb.rpc({'verb': 'job', 'id': jid}, NOW + 700, agents=[])
        self.assertEqual(r['state'], 'error')
        self.assertIn('Nothing picked this up', r['error'])

    def test_continue_with_handoff_text_seeds_the_new_session(self):
        jid = self.job({'verb': 'start', 'folder': 'proj', 'handoff_text': '# Handoff\nFROM OTHER MACHINE', 'name': 'Moved'})
        self.drain()
        prompt = self.launched[0]['argv'][-1]
        self.assertRegex(prompt, r'^\[switchboard:[0-9a-f]+\]')
        out = self.sb.handle_hook({'hook_event_name': 'UserPromptSubmit', 'session_id': 'S9', 'transcript_path': '/none',
                                   'prompt': prompt}, NOW + 10, None)
        self.assertIn('FROM OTHER MACHINE', out['hookSpecificOutput']['additionalContext'])
        again = self.sb.handle_hook({'hook_event_name': 'UserPromptSubmit', 'session_id': 'S9', 'transcript_path': '/none',
                                     'prompt': prompt}, NOW + 11, None)
        self.assertIsNone(again)   # seed is single-use


class HomeFolderTest(JobBase):
    """Claude Code will not run a background session in the home folder; the Laptop's root is home."""
    def setUp(self):
        super().setUp()
        self.home = self.tmp / 'home'
        self.home.mkdir()
        os.environ.update(SB_USER_HOME=str(self.home), SB_ROOT=str(self.home))
        self.mem = self.cdir / 'projects' / self.sb.re.sub(r'[^A-Za-z0-9]', '-', str(self.home.resolve())) / 'memory'
        self.mem.mkdir(parents=True)
        (self.mem / 'MEMORY.md').write_text('- [Canvas](canvas.md)')
        self.transcript(SID_A, str(self.home), NOW - 60, title='Kitchen / remodel')
        self.sb.rpc({'verb': 'list'}, NOW, agents=[])

    def tearDown(self):
        os.environ.pop('SB_USER_HOME', None)
        super().tearDown()

    def test_fresh_from_home_continues_in_a_trusted_folder_with_the_same_memory(self):
        (self.home / 'Desktop' / 'Claude').mkdir(parents=True)  # a Claude folder on the Desktop is used when there is one
        jid = self.job({'verb': 'fresh', 'sid': SID_A})
        self.drain()
        r = self.sb.rpc({'verb': 'job', 'id': jid}, NOW + 6, agents=[])
        stand_in = (self.home / 'Desktop' / 'Claude').resolve()
        self.assertEqual((r['state'], Path(self.launched[0]['cwd'])), ('done', stand_in))
        self.assertTrue(json.loads(self.cjson.read_text())['projects'][self.sb.trust_key(stand_in)]['hasTrustDialogAccepted'])
        link = self.cdir / 'projects' / self.sb.re.sub(r'[^A-Za-z0-9]', '-', str(stand_in)) / 'memory'
        self.assertEqual((link / 'MEMORY.md').read_text(), '- [Canvas](canvas.md)')
        self.assertFalse(json.loads(self.cjson.read_text())['projects'].get(self.sb.trust_key(self.home.resolve())))
        self.sb.launch_dir(str(self.home))  # a second time leaves the link alone
        self.assertTrue(link.is_symlink())

    def test_new_session_in_the_home_folder_does_too(self):
        with mock.patch.object(self.sb, 'machine_name', lambda: 'laptop'):
            self.job({'verb': 'start', 'folder': '', 'prompt': 'plan my week'})
        self.drain()
        self.assertEqual(Path(self.launched[0]['cwd']), (self.home / 'Claude').resolve())

    def test_other_folders_are_untouched(self):
        (self.home / 'proj').mkdir()
        self.assertEqual(self.sb.launch_dir(str(self.home / 'proj')), str(self.home / 'proj'))
        self.assertFalse((self.home / 'Desktop' / 'Claude').exists())

    def test_resume_of_a_home_chat_explains_what_to_do(self):
        jid = self.job({'verb': 'resume', 'sid': SID_A})
        self.drain(runner=self.runner(rc=1, out='Workspace not trusted. The home directory is trusted one session at a time'))
        self.assertEqual(Path(self.launched[0]['cwd']), self.home.resolve())
        r = self.sb.rpc({'verb': 'job', 'id': jid}, NOW + 6, agents=[])
        self.assertIn('Start fresh', r['error'])

    def test_a_failed_start_fresh_keeps_the_held_message(self):
        self.sb.held_set(SID_A, 'send the essay', NOW)
        self.job({'verb': 'fresh', 'sid': SID_A})
        self.drain(runner=self.runner(rc=1, out='claude exited badly'))
        self.assertEqual(self.sb.held_get(SID_A)['prompt'], 'send the essay')


class CarryOverTest(JobBase):
    """A chat that continues an old one can do what the old one did."""
    def old_chat(self, extra=(), model='claude-opus-5-5'):
        rows = [dict(user('2026-10-01T00:00:01Z', 'look at canvas'), cwd=str(self.root / 'proj')),
                {'type': 'assistant', 'timestamp': '2026-10-01T00:00:02Z', 'effort': 'medium',
                 'message': {'model': model, 'content': [{'type': 'tool_use', 'name': 'Read', 'input': {}}],
                             'usage': {'input_tokens': 9}}},
                *extra,
                {'type': 'ai-title', 'aiTitle': 'Canvas', 'sessionId': SID_A}]
        p = write_transcript(self.cdir / 'projects' / 'p1', rows, name=f'{SID_A}.jsonl')
        os.utime(p, (NOW - 60, NOW - 60))
        self.sb.rpc({'verb': 'list'}, NOW, agents=[])
        return p

    def chrome_use(self):
        return {'type': 'assistant', 'timestamp': '2026-10-01T00:00:03Z', 'effort': 'xhigh',
                'message': {'model': 'claude-opus-5-5', 'content': [{'type': 'tool_use', 'name': 'mcp__claude-in-chrome__navigate',
                                                                    'input': {'url': 'https://camino.instructure.com'}}]}}

    def argv(self):
        return self.launched[0]['argv']

    def test_fresh_keeps_chrome_and_effort(self):
        self.old_chat([self.chrome_use()])
        self.job({'verb': 'fresh', 'sid': SID_A})
        self.drain()
        a = self.argv()
        self.assertIn('--chrome', a)
        self.assertEqual(a[a.index('--effort') + 1], 'xhigh')
        self.assertNotIn('--model', a)  # Opus is the default; passing it would drop the 1M context setting

    def test_a_chat_that_only_mentions_chrome_does_not_get_it(self):
        self.old_chat([user('2026-10-01T00:00:04Z', 'what does mcp__claude-in-chrome__navigate do?')])
        self.job({'verb': 'fresh', 'sid': SID_A})
        self.drain()
        self.assertNotIn('--chrome', self.argv())

    def test_ultracode_and_sonnet_carry_over(self):
        self.old_chat([{'type': 'attachment', 'attachment': {'type': 'ultra_effort_enter'}}])
        self.assertEqual(self.sb.carry_over(self.cdir / 'projects' / 'p1' / f'{SID_A}.jsonl')['effort'], 'ultracode')
        self.job({'verb': 'resume', 'sid': SID_A})
        self.drain()
        a = self.argv()
        self.assertEqual((a[a.index('--effort') + 1], a[a.index('--model') + 1]), ('ultracode', 'opus'))
        p = self.old_chat([{'type': 'attachment', 'attachment': {'type': 'ultra_effort_enter'}},
                           {'type': 'attachment', 'attachment': {'type': 'ultra_effort_exit'}}], model='claude-sonnet-5-5')
        self.assertEqual(self.sb.carry_over(p, 'claude-sonnet-5-5'), {'effort': 'medium', 'model': 'sonnet'})

    def test_a_stopped_background_chat_resumes_with_its_own_options(self):
        self.old_chat([self.chrome_use()])
        agents = [{'sessionId': SID_A, 'id': 'aaaaaaaa', 'kind': 'background', 'status': 'stopped'}]
        self.assertTrue(self.sb.rpc({'verb': 'resume', 'sid': SID_A}, NOW, agents=agents)['ok'])
        self.drain()
        self.assertNotIn('--chrome', self.argv())

    def test_continue_hands_over_what_the_chat_could_do(self):
        self.old_chat([self.chrome_use()])
        self.sb.handoff_path(SID_A).write_text('# Handoff')
        r = self.sb.rpc({'verb': 'handoff_text', 'sid': SID_A}, NOW, agents=[])
        self.assertEqual(r['carry'], {'chrome': True, 'effort': 'xhigh'})
        self.job({'verb': 'start', 'folder': 'proj', 'handoff_text': '# Handoff', 'from': 'Laptop', 'chrome': True, 'effort': 'xhigh'})
        self.drain()
        self.assertIn('--chrome', self.argv())
        self.assertFalse(self.sb.rpc({'verb': 'start', 'folder': 'proj', 'effort': 'turbo'}, NOW, agents=[])['ok'])

    def test_start_fresh_stops_an_idle_background_original(self):
        self.old_chat()
        agents = [{'sessionId': SID_A, 'id': 'aaaaaaaa', 'kind': 'background', 'status': 'idle'}]
        self.sb.claude_agents = lambda: agents
        self.job({'verb': 'fresh', 'sid': SID_A})
        self.drain()
        self.assertIn(['stop', 'aaaaaaaa'], self.calls)

    def test_start_fresh_leaves_a_busy_or_terminal_original_alone(self):
        self.old_chat()
        for a in ({'sessionId': SID_A, 'id': 'aaaaaaaa', 'kind': 'background', 'status': 'busy'},
                  {'sessionId': SID_A, 'kind': 'interactive', 'status': 'idle', 'pid': 7}):
            self.sb.claude_agents = lambda a=a: [a]
            self.job({'verb': 'fresh', 'sid': SID_A})
            self.drain()
        self.assertEqual([c for c in self.calls if c[0] == 'stop'], [])


class SessionJobTest(JobBase):
    def setUp(self):
        super().setUp()
        self.t = self.transcript(SID_A, str(self.root / 'proj'), NOW - 60)
        self.sb.rpc({'verb': 'list'}, NOW, agents=[])

    def test_resume_uses_session_cwd(self):
        self.job({'verb': 'resume', 'sid': SID_A})
        self.drain()
        l = self.launched[0]
        self.assertEqual(Path(l['cwd']), (self.root / 'proj').resolve())
        self.assertIn('--resume', l['argv'])

    def test_fresh_makes_handoff_then_seeds_with_held_message(self):
        self.sb.held_set(SID_A, 'fix the lights', NOW)
        made = []
        self.job({'verb': 'fresh', 'sid': SID_A})
        self.drain(make_handoff=lambda tp, sid: made.append(sid) or self.sb.handoff_path(sid).write_text('HANDOFF BODY'))
        self.assertEqual(made, [SID_A])
        prompt = self.launched[0]['argv'][-1]
        self.assertIn('fix the lights', prompt)
        self.assertIsNone(self.sb.held_get(SID_A))
        out = self.sb.handle_hook({'hook_event_name': 'UserPromptSubmit', 'session_id': 'S9', 'transcript_path': '/none',
                                   'prompt': prompt}, NOW + 10, None)
        self.assertIn('HANDOFF BODY', out['hookSpecificOutput']['additionalContext'])
        # the context names the old chat's handoff file, so the new chat links back to it even with a short handoff
        self.assertEqual(self.sb._handoff_ref([out['hookSpecificOutput']['additionalContext']], 'S9'), SID_A)

    def test_handoff_close_stops_background_session(self):
        agents = [{'sessionId': SID_A, 'id': 'aaaaaaaa', 'kind': 'background', 'status': 'idle'}]
        r = self.sb.rpc({'verb': 'handoff', 'sid': SID_A, 'close': True}, NOW, agents=agents)
        self.sb.claude_agents = lambda: agents
        self.drain()
        self.assertTrue(self.sb.handoff_path(SID_A).exists())
        self.assertIn(['stop', 'aaaaaaaa'], self.calls)
        self.assertEqual(self.sb.rpc({'verb': 'job', 'id': r['job']}, NOW + 6, agents=[])['state'], 'done')

    def test_handoff_text(self):
        self.sb.handoff_path(SID_A).write_text('TEXT')
        self.assertEqual(self.sb.rpc({'verb': 'handoff_text', 'sid': SID_A}, NOW, agents=[])['text'], 'TEXT')


class QueueTest(JobBase):
    def test_drain_is_single_flight(self):
        self.job({'verb': 'start', 'folder': 'proj'})
        lock = self.sb.paths_q()['queue'] / '.lock'
        lock.write_text(str(NOW))
        self.drain()
        self.assertEqual(self.launched, [])
        lock.write_text(str(NOW - 2000))   # stale lock
        self.drain()
        self.assertEqual(len(self.launched), 1)


class LauncherInstallTest(unittest.TestCase):
    def test_windows_task_xml(self):
        from tests.helpers import fresh_sb
        sb = fresh_sb(tempfile.mkdtemp())
        x = sb.launcher_task_xml('C:\\Python\\pythonw.exe', 'C:/Users/you/.claude/switchboard/sb.py', 'DESKTOP\\me')
        self.assertIn('<LogonType>InteractiveToken</LogonType>', x)
        self.assertIn('pythonw.exe', x)
        self.assertIn('launch-queue', x)
        self.assertNotIn('<TimeTrigger>', x)


if __name__ == '__main__':
    unittest.main()


class CliTest(JobBase):
    def test_rpc_cli_reads_stdin_and_prints_json(self):
        import io, sys
        with mock.patch.object(sys, 'stdin', io.StringIO('{"verb": "folders"}')), mock.patch.object(sys, 'stdout', io.StringIO()) as out:
            self.assertEqual(self.sb.main(['sb.py', 'rpc']), 0)
        self.assertTrue(json.loads(out.getvalue())['ok'])
        with mock.patch.object(sys, 'stdin', io.StringIO('not json')), mock.patch.object(sys, 'stdout', io.StringIO()) as out:
            self.sb.main(['sb.py', 'rpc'])
        self.assertFalse(json.loads(out.getvalue())['ok'])

    def test_install_retires_the_launch_agent_and_spawns_nothing_over_ssh(self):
        home = self.tmp / 'home'; (home / '.claude').mkdir(parents=True)
        la = home / 'Library' / 'LaunchAgents'; la.mkdir(parents=True)
        (la / f'{self.sb.LAUNCHER_LABEL}.plist').write_text('old')
        spawned = []
        with mock.patch.object(self.sb, 'IS_WIN', False), mock.patch.object(self.sb, '_spawn_launcher', lambda: spawned.append(1)), \
             mock.patch.object(self.sb.subprocess, 'run', lambda *a, **k: None):
            self.sb.install(home=home)
        self.assertFalse((la / f'{self.sb.LAUNCHER_LABEL}.plist').exists())
        self.assertEqual(spawned, [])

    def test_session_start_heals_the_launcher(self):
        healed = []
        self.sb.ensure_launcher = lambda: healed.append(1)
        self.sb.handle_hook({'hook_event_name': 'SessionStart', 'source': 'startup', 'session_id': 'S1', 'transcript_path': '/n'}, NOW, None)
        self.assertEqual(healed, [1])


class WinTaskUserTest(JobBase):
    def test_task_user_comes_from_whoami_not_userdomain(self):
        # Over SSH USERDOMAIN is WORKGROUP, which schtasks cannot map to an account.
        seen = {}
        class R:
            def __init__(self, out=''): self.returncode, self.stdout, self.stderr = 0, out, ''
        def fake_run(args, **kw):
            if args[0] == 'whoami':
                return R('desktop-01gmhu4\\live\r\n')
            seen['xml'] = Path(args[args.index('/xml') + 1]).read_text(encoding='utf-16')
            return R()
        with mock.patch.object(self.sb, 'IS_WIN', True), mock.patch.object(self.sb.subprocess, 'run', fake_run), \
             mock.patch.dict(os.environ, {'USERDOMAIN': 'WORKGROUP', 'USERNAME': 'live'}):
            self.sb.install_launcher(self.tmp, str(self.tmp / 'sb.py'))
        self.assertIn('<UserId>desktop-01gmhu4\\live</UserId>', seen['xml'])


class LauncherHealTest(JobBase):
    def test_heal_runs_at_most_once_a_minute(self):
        calls = []
        self.sb.ensure_launcher = lambda: calls.append(1)
        self.sb.heal_launcher(NOW)
        self.sb.heal_launcher(NOW + 30)
        self.sb.heal_launcher(NOW + 61)
        self.assertEqual(len(calls), 2)

    def test_statusline_heals(self):
        import io, sys
        seen = []
        self.sb.heal_launcher = lambda now: seen.append(now)
        with mock.patch.object(sys, 'stdin', io.StringIO('{"session_id": "S1"}')), mock.patch.object(sys, 'stdout', io.StringIO()):
            self.sb.main(['sb.py', 'statusline'])
        self.assertEqual(len(seen), 1)


class DaemonPipeTest(JobBase):
    def test_launch_returns_while_a_spawned_daemon_keeps_stdout_open(self):
        # `claude --bg` may start the background service, which inherits our stdout; we must not wait for it.
        import sys, time as _t
        child = ('import subprocess, sys; subprocess.Popen([sys.executable, "-c", "import time; time.sleep(20)"]); '
                 'print("backgrounded · 03bf7f41 · x")')
        t0 = _t.time()
        rc, out = self.sb._run_launch([sys.executable, '-c', child], str(self.tmp), dict(os.environ))
        self.assertLess(_t.time() - t0, 10)
        self.assertEqual(rc, 0)
        self.assertEqual(self.sb.parse_bg_id(out + '\n  claude stop 03bf7f41'), '03bf7f41')
        self.assertIn('backgrounded', out)


class CmdOutputTest(JobBase):
    def test_command_output_excludes_stderr(self):
        import sys
        out = self.sb._run_to_file([sys.executable, '-c', 'import sys; print("[]"); print("warn", file=sys.stderr)'], 10, merge=False)[1]
        self.assertEqual(out.strip(), '[]')


class LaunchTimeoutTest(JobBase):
    def test_timeout_reports_what_claude_printed(self):
        import sys
        with self.assertRaises(RuntimeError) as e:
            self.sb._run_to_file([sys.executable, '-c', 'import time; print("Waiting for login", flush=True); time.sleep(10)'], 1)
        self.assertIn('Waiting for login', str(e.exception))


class ResidentLauncherTest(JobBase):
    def test_ensure_spawns_only_when_no_live_launcher(self):
        spawned = []
        with mock.patch.object(self.sb, 'IS_WIN', False):
            self.sb.ensure_launcher(spawn=lambda: spawned.append(1))
            self.assertEqual(spawned, [1])
            self.sb._jput(self.sb.paths_q()['queue'] / '.launcher', {'pid': os.getpid(), 'ts': time.time()})
            self.sb.ensure_launcher(spawn=lambda: spawned.append(1))
            self.assertEqual(spawned, [1])
            self.sb._jput(self.sb.paths_q()['queue'] / '.launcher', {'pid': 999999, 'ts': time.time()})  # dead pid
            self.sb.ensure_launcher(spawn=lambda: spawned.append(1))
            self.assertEqual(spawned, [1, 1])

    def test_daemon_drains_and_exits_when_sb_py_changes(self):
        drained, ticks = [], []
        me = self.tmp / 'sb_copy.py'; me.write_text('x')
        def sleep(_):
            ticks.append(1)
            if len(ticks) == 2:
                os.utime(me, (1, 1))   # a deploy replaced sb.py
        with mock.patch.object(self.sb, 'launch_queue', lambda: drained.append(1)):
            rc = self.sb.launcher_daemon(me=me, sleep=sleep)
        self.assertEqual(rc, 0)
        self.assertEqual(len(drained), 3)
        self.assertFalse((self.sb.paths_q()['queue'] / '.launcher').exists())

    def test_second_daemon_exits_at_once(self):
        self.sb._jput(self.sb.paths_q()['queue'] / '.launcher', {'pid': os.getpid(), 'ts': time.time()})
        with mock.patch.object(self.sb, 'launch_queue', lambda: self.fail('second launcher drained')):
            self.assertEqual(self.sb.launcher_daemon(me=self.tmp / 'nope.py', sleep=lambda _: None), 0)

    def test_launch_queue_command_is_a_no_op_on_mac(self):
        ran = []
        with mock.patch.object(self.sb, 'IS_WIN', False), mock.patch.object(self.sb, 'launch_queue', lambda: ran.append(1) or 0):
            self.assertEqual(self.sb.main(['sb.py', 'launch-queue']), 0)
        self.assertEqual(ran, [])

    def test_list_reports_launcher_and_mac_timeout_says_what_to_do(self):
        with mock.patch.object(self.sb, 'IS_WIN', False):
            r = self.sb.rpc({'verb': 'list'}, NOW, agents=[])
            self.assertEqual(r['launcher']['alive'], False)
            jid = self.job({'verb': 'start', 'folder': 'proj'})
            err = self.sb.rpc({'verb': 'job', 'id': jid}, NOW + 700, agents=[])['error']
        self.assertIn('Open any Claude Code session', err)


class ResumeForkTest(JobBase):
    def test_resume_hides_the_superseded_session(self):
        self.transcript(SID_A, str(self.root / 'proj'), NOW - 60)
        self.sb.rpc({'verb': 'list'}, NOW, agents=[])
        self.job({'verb': 'resume', 'sid': SID_A})
        self.drain()
        sids = [s['sid'] for s in self.sb.rpc({'verb': 'list'}, NOW + 10, agents=[])['sessions']]
        self.assertNotIn(SID_A, sids)

    def test_untitled_session_resumes_under_its_first_prompt(self):
        self.transcript(SID_A, str(self.root / 'proj'), NOW - 60, title=None)
        self.job({'verb': 'resume', 'sid': SID_A})
        self.drain()
        argv = self.launched[0]['argv']
        self.assertEqual(argv[argv.index('--name') + 1], self.sb.name_stamp(NOW) + ' first thing I asked (copy)')

    def test_new_session_without_a_name_is_named_for_its_folder_when_the_message_says_nothing(self):
        self.job({'verb': 'start', 'folder': 'proj', 'prompt': 'hi'})
        self.drain()
        argv = self.launched[0]['argv']
        self.assertEqual(argv[argv.index('--name') + 1], 'proj')

    def test_custom_title_wins(self):
        p = self.transcript(SID_A, str(self.root), NOW - 60, title='AI title')
        with open(p, 'a') as f:
            f.write(json.dumps({'type': 'custom-title', 'customTitle': 'My name', 'sessionId': SID_A}) + '\n')
        os.utime(p, (NOW - 60, NOW - 60))
        self.assertEqual(self.sb.rpc({'verb': 'list'}, NOW, agents=[])['sessions'][0]['title'], 'My name')


class LaunchHealthTest(JobBase):
    def seq_runner(self, ids):
        it = iter(ids)
        def run(argv, cwd, env):
            self.launched.append({'argv': argv})
            return 0, f'  claude stop {next(it)}    stop this session\n'
        return run

    def result(self, jid):
        return self.sb.rpc({'verb': 'job', 'id': jid}, NOW + 10, agents=[])

    def test_stuck_launch_is_stopped_and_retried_once(self):
        jid = self.job({'verb': 'start', 'folder': 'proj', 'prompt': 'hi'})
        self.drain(runner=self.seq_runner(['11111111', '22222222']), live=lambda bg: 'stuck' if bg == '11111111' else 'ready')
        r = self.result(jid)
        self.assertEqual((r['state'], r['bg_id']), ('done', '22222222'))
        self.assertEqual(len(self.launched), 2)
        self.assertIn(['stop', '11111111'], self.calls)

    def test_launch_that_never_comes_up_is_a_readable_error(self):
        jid = self.job({'verb': 'start', 'folder': 'proj', 'prompt': 'hi'})
        self.drain(runner=self.seq_runner(['11111111', '22222222']), live=lambda bg: 'stuck')
        r = self.result(jid)
        self.assertEqual(r['state'], 'error')
        self.assertIn('never came up', r['error'])
        self.assertIn(['stop', '22222222'], self.calls)

    def test_wait_live_reads_the_agent_pid(self):
        seen = iter([[{'id': 'abcd1234', 'state': 'blocked'}], [{'id': 'abcd1234', 'pid': 9, 'status': 'idle'}]])
        with mock.patch.object(self.sb, 'claude_agents', lambda: next(seen)):
            self.assertEqual(self.sb.wait_live('abcd1234', wait=5, sleep=lambda _: None), 'ready')
        with mock.patch.object(self.sb, 'claude_agents', lambda: [{'id': 'abcd1234', 'state': 'blocked'}]):
            self.assertEqual(self.sb.wait_live('abcd1234', wait=0, sleep=lambda _: None), 'stuck')


class StoppedBackgroundTest(JobBase):
    STOPPED = {'id': 'aaaaaaaa', 'kind': 'background', 'sessionId': SID_A, 'state': 'done'}

    def test_list_marks_a_stopped_background_session(self):
        self.transcript(SID_A, str(self.root / 'proj'), NOW - 60)
        s = self.sb.rpc({'verb': 'list'}, NOW, agents=[self.STOPPED])['sessions'][0]
        self.assertEqual((s['kind'], s['status']), ('background', 'stopped'))

    def test_resume_continues_a_stopped_background_session_in_place(self):
        self.transcript(SID_A, str(self.root / 'proj'), NOW - 60)
        r = self.sb.rpc({'verb': 'resume', 'sid': SID_A}, NOW, agents=[self.STOPPED])
        self.assertTrue(r['ok'], r)
        self.drain()
        self.assertEqual(self.launched[0]['argv'][1:], ['--bg', '--resume', SID_A])
        sids = [x['sid'] for x in self.sb.rpc({'verb': 'list'}, NOW + 10, agents=[])['sessions']]
        self.assertIn(SID_A, sids)

    def test_resume_of_a_running_background_chat_opens_it_without_a_copy(self):
        # 2026-10-13: two clicks on Resume for a running chat started two copies, and each one hid the rest
        self.transcript(SID_A, str(self.root / 'proj'), NOW - 60)
        running = dict(self.STOPPED, state='blocked', status='idle')
        jid = self.job({'verb': 'resume', 'sid': SID_A})
        with mock.patch.object(self.sb, 'claude_agents', lambda: [running]):
            self.drain()
        self.assertEqual(self.launched, [])
        r = self.sb.rpc({'verb': 'job', 'id': jid}, NOW + 10, agents=[running])
        self.assertEqual((r['state'], r['bg_id'], r['sid']), ('done', 'aaaaaaaa', SID_A))
        sids = [x['sid'] for x in self.sb.rpc({'verb': 'list'}, NOW + 10, agents=[running])['sessions']]
        self.assertIn(SID_A, sids)

    def test_a_copy_never_marks_its_source_as_replaced_by_itself(self):
        self.transcript(SID_A, str(self.root / 'proj'), NOW - 60)
        self.job({'verb': 'resume', 'sid': SID_A})
        with mock.patch.object(self.sb, 'bg_session_id', lambda bg: SID_A):
            self.drain()
        marks = self.sb._jget(self.sb.paths()['state'] / 'replaced.json') or {}
        self.assertNotIn(SID_A, marks)

    def test_agents_include_stopped_ones(self):
        self.sb.run_claude_cmd = lambda args, **kw: self.calls.append(args) or '[]'
        self.sb.claude_agents()
        self.assertEqual(self.calls[-1], ['agents', '--json', '--all'])


class WakeTest(RpcBase):
    def beat(self):
        self.sb._jput(self.sb.paths_q()['queue'] / '.launcher', {'pid': os.getpid(), 'ts': time.time()})

    def test_queueing_on_a_mac_with_no_launcher_wakes_one(self):
        detached = []
        with mock.patch.object(self.sb, 'IS_WIN', False), mock.patch.object(self.sb, '_detach', lambda argv: detached.append(argv)):
            self.sb.trigger_launcher()
            self.assertEqual(detached[0][-1], 'wake')
            self.beat()
            self.sb.trigger_launcher()
        self.assertEqual(len(detached), 1)

    def test_wake_asks_claude_for_a_promptless_session_then_removes_it(self):
        ran = []
        def run(argv, cwd):
            ran.append((argv, cwd))
            return 0, '  claude stop abcd1234    stop this session\n'
        with mock.patch.object(self.sb, '_claude_exe', lambda: 'claude'):
            ok = self.sb.wake_launcher(run=run, sleep=lambda _: self.beat())
        self.assertTrue(ok)
        wd = self.sb.SB_HOME / 'wake'
        self.assertEqual(ran, [(['claude', '--bg', '--name', 'switchboard-wake'], str(wd))])
        trusted = json.loads(self.cjson.read_text())['projects']
        self.assertTrue(trusted[self.sb.trust_key(wd)]['hasTrustDialogAccepted'])
        self.assertEqual(self.calls[-2:], [['stop', 'abcd1234'], ['rm', 'abcd1234']])

    def test_wake_is_single_flight(self):
        self.sb._jput_text(self.sb.paths_q()['queue'] / '.waking', str(time.time()))
        self.assertFalse(self.sb.wake_launcher(run=lambda *a: self.fail('second wake ran'), sleep=lambda _: None))


class MacQueuedJobTest(JobBase):
    def test_mac_job_fails_fast_when_no_launcher_came_up(self):
        with mock.patch.object(self.sb, 'IS_WIN', False):
            jid = self.job({'verb': 'start', 'folder': 'proj'})
            self.assertEqual(self.sb.rpc({'verb': 'job', 'id': jid}, NOW + 20, agents=[])['state'], 'queued')
            r = self.sb.rpc({'verb': 'job', 'id': jid}, NOW + 90, agents=[])
        self.assertEqual(r['state'], 'error')
        self.assertIn('Open any Claude Code session', r['error'])


class WakeRetryTest(WakeTest):
    def test_wake_tries_a_second_session_when_the_first_starts_no_launcher(self):
        ids = iter(['aaaa1111', 'bbbb2222'])
        ran = []
        def run(argv, cwd):
            ran.append(argv)
            return 0, f'  claude stop {next(ids)}    stop this session\n'
        ticks = []
        def sleep(_):
            ticks.append(1)
            if len(ran) == 2:
                self.beat()
        with mock.patch.object(self.sb, '_claude_exe', lambda: 'claude'):
            ok = self.sb.wake_launcher(run=run, sleep=sleep, wait=0.05)
        self.assertTrue(ok)
        self.assertEqual(len(ran), 2)
        self.assertIn(['rm', 'aaaa1111'], self.calls)
        self.assertIn(['rm', 'bbbb2222'], self.calls)
