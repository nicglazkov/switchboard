import io, json, os, sys, tempfile, unittest
from unittest import mock
from tests.helpers import fresh_sb


class TermModeTest(unittest.TestCase):
    """sb.py is the forced command. `term <bg_id>` from the hub opens `claude attach`; everything else is the RPC."""

    def setUp(self):
        self.tmp = tempfile.mkdtemp()
        self.sb = fresh_sb(self.tmp)

    def test_what_counts_as_a_terminal_request(self):
        tr = self.sb.term_request
        for cmd in (None, '', '   ', 'ls', 'terminal', 'rpc', '{"verb": "list"}'):
            self.assertIsNone(tr(cmd), cmd)
        self.assertEqual(tr('term c03ef0d9'), ('attach', 'c03ef0d9'))
        self.assertEqual(tr(' term c03ef0d9 \n'), ('attach', 'c03ef0d9'))
        self.assertEqual(tr('term ' + 'a' * 36), ('attach', 'a' * 36))
        for cmd in ('term', 'term c03ef0d', 'term C03EF0D9', 'term c03ef0d9; rm -rf ~', 'term c03ef0d9 x', 'term ../x',
                    'term c03ef0d9\nterm x', 'term ' + 'a' * 65):
            self.assertEqual(tr(cmd)[0], 'bad', cmd)

    def test_attach_replaces_the_process_with_claude(self):
        seen = {}
        with mock.patch.dict(os.environ, {'ANTHROPIC_API_KEY': 'sk-test', 'SB_INTERNAL': '1', 'LANG': ''}), \
                mock.patch.object(self.sb, 'IS_WIN', False), mock.patch.object(self.sb, '_claude_exe', lambda: '/x/claude'):
            rc = self.sb.run_term('term c03ef0d9', isatty=lambda: True, execve=lambda p, a, e: seen.update(p=p, a=a, e=e))
        self.assertEqual(rc, 0)
        self.assertEqual((seen['p'], seen['a']), ('/x/claude', ['/x/claude', 'attach', 'c03ef0d9']))
        self.assertNotIn('ANTHROPIC_API_KEY', seen['e'])   # bills the plan login, like every session Switchboard starts
        self.assertNotIn('SB_INTERNAL', seen['e'])         # its hooks run: it is a session you drive
        self.assertEqual(seen['e']['COLORTERM'], 'truecolor')
        self.assertEqual(seen['e']['LANG'], 'en_US.UTF-8')

    def test_windows_runs_it_on_the_console_and_returns_its_code(self):
        calls = []

        def run(argv, **kw):
            calls.append((argv, kw))
            return mock.Mock(returncode=3)
        with mock.patch.object(self.sb, 'IS_WIN', True), mock.patch.object(self.sb, '_claude_exe', lambda: 'C:/claude.exe'):
            rc = self.sb.run_term('term c03ef0d9', isatty=lambda: True, run=run)
        self.assertEqual(rc, 3)
        argv, kw = calls[0]
        self.assertEqual(argv, ['C:/claude.exe', 'attach', 'c03ef0d9'])
        self.assertNotIn('stdin', kw)   # inherited console, nothing redirected
        self.assertNotIn('stdout', kw)

    def test_unix_runs_attach_under_the_watch(self):
        seen = {}
        with mock.patch.object(self.sb, 'IS_WIN', False), mock.patch.object(self.sb, '_claude_exe', lambda: '/x/claude'):
            rc = self.sb.run_term('term c03ef0d9', isatty=lambda: True, watch=lambda a, e: seen.update(a=a) or 0)
        self.assertEqual(rc, 0)
        self.assertEqual(seen['a'], ['/x/claude', 'attach', 'c03ef0d9'])

    def test_leaving_the_chat_closes_the_agent_list_and_says_so(self):
        """`exit` in the chat: Claude Code opens `claude agents` under attach. It is closed, and the tab goes."""
        class Proc:
            pid, waits, killed = 4242, 0, False

            def wait(self, timeout=None):
                self.waits += 1
                if self.waits < 3:
                    raise self_sb.subprocess.TimeoutExpired('claude', timeout)
                return -15

            def kill(self):
                self.killed = True
        self_sb = self.sb
        proc, polls, killed = Proc(), [], []
        lists = iter([[], [777]])

        def children(pid):
            polls.append(pid)
            return next(lists)
        with mock.patch.object(self.sb.os, 'kill', lambda pid, sig: killed.append(pid)):
            rc = self.sb.attach_watch(['/x/claude', 'attach', 'c03ef0d9'], {}, popen=lambda a, **kw: proc,
                                      children=children, poll=0)
        self.assertEqual(rc, self.sb.TERM_LEFT)
        self.assertEqual(polls, [4242, 4242])
        self.assertEqual(killed, [777])

    def test_any_other_end_passes_its_code_back(self):
        proc = mock.Mock(pid=1)
        proc.wait.return_value = 0
        rc = self.sb.attach_watch(['/x/claude', 'attach', 'c03ef0d9'], {}, popen=lambda a, **kw: proc,
                                  children=self.fail, poll=0)
        self.assertEqual(rc, 0)
        proc.wait.return_value = -9
        self.assertEqual(self.sb.attach_watch(['c'], {}, popen=lambda a, **kw: proc, children=self.fail, poll=0), 137)

    def test_only_claude_agents_counts_as_the_agent_list(self):
        def run(argv, **kw):
            if argv[0] == 'pgrep':
                return mock.Mock(stdout='11\n12\n13\n14\n')
            return mock.Mock(stdout='11 /Users/you/.local/bin/claude agents --settings x\n12 /bin/zsh -c agents\n'
                                    '13 /usr/bin/python3 x agents\n14 node /usr/lib/node_modules/@anthropic-ai/claude-code/cli.js agents\n')
        with mock.patch.object(self.sb.subprocess, 'run', run):
            self.assertEqual(self.sb.agent_view_children(10), [11, 14])
        with mock.patch.object(self.sb.subprocess, 'run', lambda argv, **kw: mock.Mock(stdout='')):
            self.assertEqual(self.sb.agent_view_children(10), [])

    def test_refusals(self):
        err = io.StringIO()
        with mock.patch.object(sys, 'stderr', err):
            self.assertEqual(self.sb.run_term('term nope', isatty=lambda: True, execve=self.fail), 2)
            self.assertEqual(self.sb.run_term('term c03ef0d9', isatty=lambda: False, execve=self.fail), 2)
        self.assertIn('not a background session id', err.getvalue())
        self.assertIn('needs ssh -t', err.getvalue())

    def test_a_missing_claude_says_so(self):
        def gone(*a):
            raise FileNotFoundError(2, 'No such file or directory')
        err = io.StringIO()
        with mock.patch.object(sys, 'stderr', err), mock.patch.object(self.sb, 'IS_WIN', False):
            self.assertEqual(self.sb.run_term('term c03ef0d9', isatty=lambda: True, execve=gone), 127)
        self.assertIn("couldn't start Claude Code", err.getvalue())

    def test_main_routes_on_ssh_original_command(self):
        with mock.patch.dict(os.environ, {'SSH_ORIGINAL_COMMAND': 'term c03ef0d9'}), \
                mock.patch.object(self.sb, 'run_term', return_value=7) as rt:
            self.assertEqual(self.sb.main(['sb.py', 'rpc']), 7)
        rt.assert_called_once_with('term c03ef0d9')
        # no command, or any other command: the JSON RPC as before
        for env in ({}, {'SSH_ORIGINAL_COMMAND': 'ls -la'}):
            out = io.StringIO()
            with mock.patch.dict(os.environ, env), mock.patch.object(self.sb, 'run_term', side_effect=self.fail), \
                    mock.patch.object(sys, 'stdin', io.StringIO('{"verb": "nope"}')), mock.patch.object(sys, 'stdout', out):
                os.environ.pop('SSH_ORIGINAL_COMMAND', None) if not env else None
                self.assertEqual(self.sb.main(['sb.py', 'rpc']), 0)
            self.assertFalse(json.loads(out.getvalue())['ok'])


if __name__ == '__main__':
    unittest.main()
