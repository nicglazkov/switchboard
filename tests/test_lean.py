"""The helpers that live as long as a chat or a tab stay small and go when their chat or tab goes: they start over
from cached bytecode, a closed tab takes its `claude attach` with it, and an MCP server ends with its chat."""
import io, json, os, shutil, signal, subprocess, sys, tempfile, threading, time, unittest
from pathlib import Path
from unittest import mock
from tests.helpers import ROOT, fresh_sb


class LeanStartTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp()
        self.sb = fresh_sb(self.tmp)
        self.copy = Path(self.tmp) / 'agent' / 'sb.py'  # a copy: the real folder gets no bytecode from a test
        self.copy.parent.mkdir()
        shutil.copy(ROOT / 'agent' / 'sb.py', self.copy)

    def argv(self, *args, env=None, **kw):
        with mock.patch.object(self.sb, '__file__', str(self.copy)):
            return self.sb.lean_argv(['sb.py', *args], {} if env is None else env, **kw)

    def test_only_the_long_lived_ones_start_over(self):
        term = {'SSH_ORIGINAL_COMMAND': 'term c03ef0d9'}
        for args, env in ((('mcp',), {}), (('launcher',), {}), (('rpc',), term)):
            got = self.argv(*args, env=env, exe='/usr/bin/python3')
            self.assertEqual(got[0], '/usr/bin/python3', args)
            self.assertEqual(got[-len(args) - 3:], ['-c', self.sb.LEAN_BOOT, self.copy.resolve().as_posix(), *args])
        for args, env in ((('hook',), {}), (('statusline',), {}), (('rpc',), {}), (('need-ask', 't', 's'), {}), ((), {}),
                          (('mcp',), {'PYTHONDONTWRITEBYTECODE': '1'})):
            self.assertIsNone(self.argv(*args, env=env, exe='/usr/bin/python3'), args)
        self.assertIsNone(self.argv('mcp', exe=''), 'no interpreter known')
        with mock.patch.object(self.sb, 'IS_WIN', True):
            self.assertIsNone(self.argv('mcp', exe='python'), 'Windows has no exec that keeps the process')

    def test_the_bytecode_is_written_before_the_restart(self):
        self.assertFalse((self.copy.parent / '__pycache__').exists())
        self.assertIsNotNone(self.argv('mcp', exe=sys.executable))
        self.assertEqual(len(list((self.copy.parent / '__pycache__').glob('sb.*.pyc'))), 1)

    def test_a_folder_that_cannot_keep_bytecode_runs_as_before(self):
        with mock.patch('py_compile.compile', side_effect=OSError('read-only')):
            self.assertIsNone(self.argv('mcp', exe=sys.executable))

    @unittest.skipIf(os.name == 'nt', 'no exec on Windows')
    def test_the_real_thing_answers_and_ends_with_its_pipe(self):
        env = dict(os.environ, HOME=self.tmp, SB_HOME=str(Path(self.tmp) / 'sbhome'))
        p = subprocess.Popen([sys.executable, str(self.copy), 'mcp'], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                             stderr=subprocess.PIPE, text=True, env=env, cwd=self.tmp)
        try:
            p.stdin.write(json.dumps({'jsonrpc': '2.0', 'id': 7, 'method': 'ping'}) + '\n')
            p.stdin.flush()
            self.assertEqual(json.loads(p.stdout.readline())['id'], 7)
            cmd = subprocess.run(['ps', '-o', 'command=', '-p', str(p.pid)], capture_output=True, text=True).stdout
            self.assertIn(' -c ', cmd, 'the same process, now running from bytecode')
            self.assertTrue(cmd.strip().endswith('sb.py mcp'), 'and still found by its name')
            p.stdin.close()
            self.assertEqual(p.wait(timeout=20), 0)
        finally:
            if p.poll() is None:
                p.kill()
            p.stdout.close()
            p.stderr.close()


class GoneWithItsOwnerTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp()
        self.sb = fresh_sb(self.tmp)

    @unittest.skipIf(os.name == 'nt', 'POSIX signals')
    def test_a_closed_tab_takes_claude_attach_with_it(self):
        # The hub hangs up on the helper alone. The attach under it used to be left to the kernel and to Claude.
        got = []

        class Proc:
            pid = 4242

            def wait(self, timeout=None):
                if not got:
                    os.kill(os.getpid(), signal.SIGTERM)  # the helper is told to go while it waits
                    time.sleep(0.05)
                return -15

            def send_signal(self, sig):
                got.append(sig)
        before = signal.getsignal(signal.SIGTERM)
        rc = self.sb.attach_watch(['/x/claude', 'attach', 'c03ef0d9'], {}, popen=lambda a, **kw: Proc(),
                                  children=self.fail, poll=0)
        self.assertEqual(got, [signal.SIGTERM])
        self.assertEqual(rc, 143)
        self.assertIs(signal.getsignal(signal.SIGTERM), before, 'and the handler is put back')

    def test_the_tab_helper_looks_for_the_agent_list_once_a_second(self):
        self.assertGreaterEqual(self.sb.AGENT_VIEW_POLL, 1.0)

    def test_an_mcp_server_does_not_outlive_its_chat_by_minutes(self):
        started, release = threading.Event(), threading.Event()

        def handle(msg):
            if msg.get('method') == 'tools/call':
                started.set()
                release.wait(60)  # a check() that waits on another machine
            return {'jsonrpc': '2.0', 'id': msg.get('id'), 'result': {}}
        inp = io.StringIO(json.dumps({'jsonrpc': '2.0', 'id': 1, 'method': 'tools/call'}) + '\n')
        self.sb.MCP_LINGER = 0.2
        t = time.time()
        self.assertEqual(self.sb.mcp_serve(inp, io.StringIO(), handle), 0)
        release.set()
        self.assertTrue(started.is_set())
        self.assertLess(time.time() - t, 5)

    def test_an_answer_that_cannot_be_written_does_not_end_all_answering(self):
        d = self.sb._asks_dir()
        d.mkdir(parents=True, exist_ok=True)
        now, asked = time.time(), []
        for n in range(5):
            (d / f'q{n}.json').write_text(json.dumps({'prompt': 'p', 'system': 's', 'created': now}))
        real, stop = self.sb._jput, threading.Event()

        def jput(p, obj):
            if str(p).endswith('.result.json'):
                raise OSError('disk full')
            return real(p, obj)

        def ask(prompt, system):
            asked.append(prompt)
            if len(asked) >= 5:
                stop.set()
            return 'ok'
        with mock.patch.object(self.sb, '_jput', jput):
            th = threading.Thread(target=self.sb.ask_worker, args=(stop, ask), daemon=True)
            th.start()
            stop.wait(10)
            th.join(5)
        self.assertGreaterEqual(len(asked), 5, 'the fourth and fifth were still answered')


if __name__ == '__main__':
    unittest.main()
