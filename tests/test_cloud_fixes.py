"""cloud/fixes: one test per finding fixed from docs/cloud/reports/cloud-e2e-tests.md and cloud-security-review.md
(see docs/cloud/reports/cloud-fixes.md)."""
import http.client, json, os, socket, sys, threading, time, unittest
from pathlib import Path
from tempfile import mkdtemp
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'server'))
import tests.test_auto as ta  # noqa: E402
import tests.test_busy as tb  # noqa: E402
import tests.test_rpc as tr  # noqa: E402
import tests.test_nodes as tn  # noqa: E402
import tests.test_server_jobs as sj  # noqa: E402
import tests.test_upload as tu  # noqa: E402
from tests.test_server_jobs import SID  # noqa: E402


class JobBase(unittest.TestCase):
    make, wait = sj.ServerJobTest.make, sj.ServerJobTest.wait  # the helpers only, not its tests


class FinishedJobStepTest(JobBase):
    """e2e bug 9: a finished job still said step "queued"."""
    def test_a_done_job_says_done_not_queued(self):
        app = self.make()
        r = app.post('/api/action', {'machine': 'desk', 'verb': 'fresh', 'sid': SID})
        self.wait(app, r['job'])
        j = app.job('server', r['job'])
        self.assertEqual((j['state'], j['step']), ('done', 'done'))



class ReplyOnlyBusyCopyTest(unittest.TestCase):
    """e2e bug 8: "when they finish" when only Claude's reply is running."""
    setUp = tb.BusyServerTest.setUp

    def test_reply_only_reads_claude_finishes(self):
        self.busy = {'live': True, 'subagents': [], 'commands': [], 'replying': True, 'pending': [],
                     'summary': 'Claude is still replying.'}
        for path, body, want in (('/api/action', {'verb': 'fresh'}, 'Start fresh when Claude finishes, or now, which stops the reply.'),
                                 ('/api/relaunch', {'flags': [['--chrome']]}, 'Relaunch when Claude finishes'),
                                 ('/api/continue', {'to': 'desk'}, 'Move it when Claude finishes')):
            r = self.app.post(path, dict(body, machine='studio', sid=tb.SID))
            self.assertEqual(r['need'], 'busy')
            self.assertIn(want, r['error'])
            self.assertNotIn('they', r['error'])

    def test_subagents_still_read_they(self):
        self.busy = {'live': True, 'subagents': ['Audit'], 'commands': [], 'replying': False, 'pending': [],
                     'summary': '1 subagent is still running.'}
        r = self.app.post('/api/action', {'machine': 'studio', 'sid': tb.SID, 'verb': 'fresh'})
        self.assertIn('Start fresh when they finish', r['error'])



class RelaunchOnOldAgentTest(unittest.TestCase):
    """e2e bug 5: relaunch never checked the machine's agent can relaunch; an old one did a plain resume."""
    setUp = tb.BusyServerTest.setUp

    def test_an_agent_without_relaunch_is_told_to_update(self):
        self.fleet.state['studio']['features'] = ['flags', 'busy']
        with self.assertRaises(tb.server.HttpError) as e:
            self.app.post('/api/relaunch', {'machine': 'studio', 'sid': tb.SID, 'flags': [['--chrome']], 'when': 'now'})
        self.assertEqual(e.exception.code, 409)
        self.assertIn('Update Switchboard on Studio', str(e.exception))
        self.assertEqual([c for c in self.calls if c['verb'] == 'resume'], [])



class SlowClaudeFake(ta.Fake):
    """Claude never answers: each ask takes as long as it is allowed to."""
    def __call__(self, target, req, timeout):
        if req['verb'] == 'ask':
            with self.lock:
                self.calls.append(('tower' if target != 'me@x' else 'desk', dict(req, timeout=timeout)))
            time.sleep(timeout)
            raise TimeoutError('ssh timed out')
        return super().__call__(target, req, timeout)


class AutoSlowClaudeTest(unittest.TestCase):
    """e2e bug 4: a slow Claude held the Auto pick for 80 s (two askers, 40 s each)."""
    make = ta.AutoServer.make

    def test_one_deadline_for_the_whole_ask(self):
        app = self.make(fake=SlowClaudeFake())
        with mock.patch.object(ta.features, 'ASK_TIMEOUT', 0.6):
            t0 = time.time()
            r = app.post('/api/route', {'prompt': 'Fix the iOS widget'})
            took = time.time() - t0
        self.assertEqual(r['by'], 'rules')
        self.assertLess(took, 1.2)
        self.assertLessEqual(sum(q['timeout'] for _, q in self.f.verbs('ask')), 0.6 + 0.05)



class AutoFirstStartTest(unittest.TestCase):
    """e2e bug 6: on a first start (no caps yet) Auto said no machine was online while all of them were."""
    make = ta.AutoServer.make

    def test_a_pick_asks_for_caps_before_giving_up(self):
        app = self.make()
        with app.feat.auto.lock:
            app.feat.auto.data.clear()  # nothing probed yet, as on a new install
        r = app.post('/api/route', {'prompt': 'Fix the iOS widget'})
        self.assertEqual(r['machine'], 'desk')

    def test_the_preview_stays_instant(self):
        app = self.make()
        with app.feat.auto.lock:
            app.feat.auto.data.clear()
        n = len(self.f.verbs('caps'))
        with self.assertRaises(ta.server.HttpError):
            app.post('/api/route', {'prompt': 'Fix the iOS widget', 'rules': True})
        self.assertEqual(len(self.f.verbs('caps')), n)



class LinkedMachineShowsAtOnceTest(unittest.TestCase):
    """e2e bug 2: a newly linked machine showed up only at the next poll, up to 20 s after it linked."""
    setUp = tn.NodesTest.setUp

    def test_a_sync_polls_a_new_link_at_once(self):
        self.hub.nodes[0]['since'] = 100
        nodes = tn.nodes
        nodes.sync(self.fleet, hub=self.hub.url)
        lab = next(m for m in self.fleet.snapshot()['machines'] if m['name'] == 'lab')
        self.assertTrue(lab['online'])
        self.assertEqual(lab['sessions'][0]['title'], 'On the lab')
        rpcs = lambda: [x for x in self.hub.seen if x[0] == 'POST']
        n = len(rpcs())
        nodes.sync(self.fleet, hub=self.hub.url)
        self.assertEqual(len(rpcs()), n, 'the same link is not asked again every sync')
        self.hub.nodes[0]['since'] = 200  # linked again
        nodes.sync(self.fleet, hub=self.hub.url)
        self.assertEqual(len(rpcs()), n + 1)



class OfflineAtOnceTest(unittest.TestCase):
    """e2e bug 3: a machine that dropped off kept showing online until the next poll."""
    setUp = tn.NodesTest.setUp

    def test_a_dropped_link_is_offline_at_the_next_sync(self):
        self.hub.nodes[0]['since'] = 100
        tn.nodes.sync(self.fleet, hub=self.hub.url)
        told = []
        self.fleet.on_change = told.append
        self.hub.nodes[0].update(online=False, since=None)
        tn.nodes.sync(self.fleet, hub=self.hub.url)
        lab = next(m for m in self.fleet.snapshot()['machines'] if m['name'] == 'lab')
        self.assertFalse(lab['online'])
        self.assertEqual(told, ['lab'])
        self.hub.nodes[0].update(online=True, since=300)  # back: polled at once again
        tn.nodes.sync(self.fleet, hub=self.hub.url)
        self.assertTrue(next(m for m in self.fleet.snapshot()['machines'] if m['name'] == 'lab')['online'])

    def test_a_failed_call_announces_offline(self):
        f = tn.fleet.Fleet(tn.MACH, lambda t, r, to: {'ok': True, 'sessions': []})
        f.poll()
        told = []
        f.on_change = told.append
        f.transport = lambda t, r, to: (_ for _ in ()).throw(OSError('ssh: connect to host x port 22: No route to host'))
        with self.assertRaises(tn.fleet.FleetError):
            f.call('desk', {'verb': 'stop'})
        self.assertFalse(f.state['desk']['online'])
        self.assertEqual(told, ['desk'])



class PairDownloadsTest(unittest.TestCase):
    """e2e bug 1: the install lines downloaded from port 8131, which only the owner's lab serves."""
    def setUp(self):
        self.hub = tn.FakeHub()
        self.addCleanup(self.hub.close)
        self.cfg = Path(mkdtemp())
        self.addCleanup(os.environ.pop, 'SWITCHBOARD_DOWNLOADS', None)
        os.environ.pop('SWITCHBOARD_DOWNLOADS', None)

    def pair(self, host='10.0.0.5:8130'):
        return tn.nodes.pair(host, hub=self.hub.url, cfg_dir=self.cfg)

    def test_no_downloads_means_the_pair_line_alone(self):
        r = self.pair()
        self.assertEqual(r['install'], {k: 'switchboard-node pair 10.0.0.5:8132 K7M2QX' for k in ('mac', 'linux', 'windows')})
        self.assertNotIn('8131', str(r))
        self.assertIn('Install switchboard-node', r['note'])

    def test_downloads_json_says_where(self):
        (self.cfg / 'downloads.json').write_text('{"url": "https://dl.example.net/sb/app/"}')
        self.assertIn('curl -fsSL https://dl.example.net/sb/node/switchboard-node-linux', self.pair()['install']['linux'])
        (self.cfg / 'downloads.json').write_text('{"url": "https://dl.example.net/sb/app", "node": "https://files.example.net/n"}')
        self.assertIn('https://files.example.net/n/switchboard-node-mac', self.pair()['install']['mac'])
        os.environ['SWITCHBOARD_DOWNLOADS'] = 'http://10.0.0.9:9000/node/'
        self.assertIn('http://10.0.0.9:9000/node/switchboard-node.exe', self.pair()['install']['windows'])

    def test_a_bad_url_is_not_used(self):
        (self.cfg / 'downloads.json').write_text('{"node": "http://x/a b; rm -rf ~"}')
        self.assertNotIn('rm -rf', str(self.pair()['install']))



class PairFromTheServerItselfTest(unittest.TestCase):
    """e2e bug 7: from the server's own machine, the pair line pointed the new machine at 127.0.0.1."""
    setUp, pair = PairDownloadsTest.setUp, PairDownloadsTest.pair

    def test_a_loopback_host_uses_an_address_of_the_server(self):
        for host in ('127.0.0.1:8130', 'localhost:8130', '[::1]:8130'):
            r = tn.nodes.pair(host, hub=self.hub.url, cfg_dir=self.cfg,
                              origins=['http://127.0.0.1:8130', 'http://studio.tail1234.ts.net:8130', 'http://10.0.0.5:8130'])
            self.assertEqual(r['pair'], 'switchboard-node pair studio.tail1234.ts.net:8132 K7M2QX')
        r = tn.nodes.pair('127.0.0.1:8130', hub=self.hub.url, cfg_dir=self.cfg, origins=['http://[fd7a::5]:8130'])
        self.assertEqual(r['hub'], '[fd7a::5]:8132')
        self.assertEqual(self.pair('10.0.0.7:8130')['hub'], '10.0.0.7:8132')  # the address the app used, as before



class LiveServer:
    """The real HTTP server on a free port, around a two-machine fleet that answers nothing much."""
    def __init__(self, test, **kw):
        app = tn.server.App(tn.fleet.Fleet(tn.MACH, lambda t, r, to: {'ok': True, 'sessions': []}), state_dir=Path(mkdtemp()),
                            web_dir=Path(__file__).resolve().parents[1] / 'server' / 'web', pusher=lambda *a: None, **kw)
        self.app, self.httpd = app, tn.server.make_server('127.0.0.1', 0, app)
        self.port = self.httpd.server_address[1]
        threading.Thread(target=self.httpd.serve_forever, daemon=True).start()
        test.addCleanup(self.httpd.server_close)
        test.addCleanup(self.httpd.shutdown)

    def raw(self, data, timeout=3):
        """Send bytes as they are; the status line that comes back (or '' when the server says nothing)."""
        with socket.create_connection(('127.0.0.1', self.port), timeout=timeout) as c:
            c.sendall(data)
            try:
                return c.recv(200).split(b'\r\n')[0].decode()
            except socket.timeout:
                return ''


class NegativeLengthTest(unittest.TestCase):
    """Security M1: a negative Content-Length made the server read without limit."""
    def test_negative_and_bad_lengths_are_refused(self):
        srv = LiveServer(self)
        for v in ('-1', 'abc', '1e9'):
            for path, ct in (('/api/prefs', 'application/json'), ('/api/upload?machine=desk&name=x', 'application/octet-stream')):
                head = (f'POST {path} HTTP/1.1\r\nHost: 127.0.0.1:{srv.port}\r\nX-Switchboard: 1\r\n'
                        f'Content-Type: {ct}\r\nContent-Length: {v}\r\n\r\n')
                self.assertIn(' 400', srv.raw(head.encode() + b'{}'), (v, path))  # at once, without waiting for the client to close



class AgentCase(unittest.TestCase):
    setUp, tearDown = tr.RpcBase.setUp, tr.RpcBase.tearDown

    def files(self, **req):
        return self.sb.rpc({'verb': 'files', **req}, tr.NOW, agents=[])


class DotFoldersTest(AgentCase):
    """Security H1: the file browser read dot folders, so tokens and keys leaked."""
    def test_dot_folders_are_never_read(self):
        (self.root / '.config' / 'gh').mkdir(parents=True)
        (self.root / '.config' / 'gh' / 'hosts.yml').write_text('oauth_token: gho_x')
        (self.root / 'proj' / '.ssh').mkdir()
        (self.root / 'proj' / '.ssh' / 'config').write_text('Host x')
        for folder in ('.config/gh', '.config', 'proj/.ssh', 'proj\\.ssh', './.config/gh'):
            self.assertFalse(self.files(folder=folder)['ok'], folder)
            self.assertFalse(self.files(folder=folder, name='hosts.yml')['ok'], folder)

    def test_a_key_with_any_name_is_not_previewed(self):
        (self.root / 'proj' / 'work_ed25519').write_text('-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjE\n')  # leakscan: allow (a fake key)
        f = self.files(folder='proj', name='work_ed25519')['file']
        self.assertTrue(f['secret'])
        self.assertNotIn('text', f)
        (self.root / 'proj' / 'notes.md').write_text('plain notes')
        self.assertEqual(self.files(folder='proj', name='notes.md')['file']['text'], 'plain notes')



class TrailingDotTest(AgentCase):
    """Security M3: on Windows a trailing dot slipped past the secret-file filter ("server.pem." opens server.pem)."""
    def test_a_trailing_dot_is_refused_and_listed_closed(self):
        self.assertFalse(self.files(folder='proj', name='server.pem.')['ok'])
        (self.root / 'proj' / 'server.pem.').write_text('x')  # a name Linux allows: listed, never opened
        r = self.files(folder='proj')
        self.assertEqual([f['open'] for f in r['files'] if f['name'] == 'server.pem.'], [False])



class RootInsideHomeTest(AgentCase):
    """Security H2: the Claude folder root could be set to any path, so the folder boundary meant nothing."""
    def setUp(self):
        AgentCase.setUp(self)
        os.environ.pop('SB_ROOT')  # the root comes from the server's poll, as on a real machine
        self.home = self.tmp / 'home'
        (self.home / 'code').mkdir(parents=True)
        os.environ['SB_USER_HOME'] = str(self.home)
        self.addCleanup(os.environ.pop, 'SB_USER_HOME', None)

    def test_only_a_root_inside_home_is_kept(self):
        self.sb.remember_identity({'verb': 'list', 'root': str(self.home / 'code')})
        self.assertEqual(self.sb.machine_root(), (self.home / 'code').resolve())
        for bad in ('/', '/etc', str(self.tmp), str(self.home / '..'), '\\\\evil\\share', '//evil/share', 'C:\\'):
            self.sb.remember_identity({'verb': 'list', 'root': bad})
            self.assertEqual(self.sb.machine_root(), (self.home / 'code').resolve(), bad)

    def test_a_bad_root_saved_earlier_is_not_used(self):
        (self.sb.SB_HOME).mkdir(parents=True, exist_ok=True)
        (self.sb.SB_HOME / 'agent.json').write_text('{"root": "/etc"}')
        self.assertEqual(self.sb.machine_root(), Path('~').expanduser().resolve())  # the home folder, as with no root set
        self.assertFalse(self.files(folder='', name='passwd')['ok'])



class DevicesCapTest(unittest.TestCase):
    """Security L1: devices.json grew without limit (the push-token part; the rest needs a login, C1)."""
    def test_only_the_newest_tokens_are_kept(self):
        app = LiveServer(self).app
        for i in range(tn.server.DEVICES_MAX + 5):
            app.post('/api/devices', {'token': f'{i:064x}', 'name': f'Phone {i}'})
        d = app.devices()
        self.assertEqual(len(d), tn.server.DEVICES_MAX)
        self.assertIn(f'{tn.server.DEVICES_MAX + 4:064x}', d)
        self.assertNotIn(f'{0:064x}', d)



class SlowClientTest(unittest.TestCase):
    """Security L4: no request timeout, so slow connections held threads for ever (the server part)."""
    def test_a_client_that_stops_sending_is_dropped(self):
        srv = LiveServer(self)
        self.assertEqual(srv.httpd.RequestHandlerClass.timeout, 60)
        srv.httpd.RequestHandlerClass.timeout = 0.3
        with socket.create_connection(('127.0.0.1', srv.port), timeout=5) as c:
            c.sendall(b'POST /api/prefs HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: 1000\r\n')  # and then nothing
            t0 = time.time()
            while c.recv(1000):
                pass
            self.assertLess(time.time() - t0, 3, 'the server closed it')



class ApnsTokenNotInArgvTest(unittest.TestCase):
    """Security L10: the APNs JWT was on curl's command line, readable by any local user through ps."""
    def test_the_token_goes_on_stdin(self):
        import apns
        a = apns.Apns.__new__(apns.Apns)
        a.topic, a.sandbox, a.clock = 'com.example.sb', True, lambda: 1_800_000_000
        a.jwt = lambda: 'SECRET.JWT.SIG'
        runs = []

        class R:
            stdout, stderr = '\n200', ''
        a.runner = lambda args, **k: runs.append((args, k.get('input'))) or R()
        a.push('ab' * 32, 'T', 'B', '/')
        a.push_widgets('ab' * 32)
        a.push_activity('ab' * 32, {})
        for args, stdin in runs:
            self.assertFalse([x for x in args if 'SECRET' in x])
            self.assertIn('@-', args)
            self.assertEqual(stdin, 'authorization: bearer SECRET.JWT.SIG\n')



class McpToolsAskFirstTest(AgentCase):
    """Security M4: run_on and continue_on let one prompt-injected chat start work on every machine without asking."""
    def test_tools_that_start_work_are_marked_destructive(self):
        tools = {t['name']: t.get('annotations') or {} for t in self.sb.MCP_TOOLS}
        for name in ('run_on', 'continue_on'):
            self.assertTrue(tools[name].get('destructiveHint'), name)
            self.assertFalse(tools[name].get('readOnlyHint'), name)
        self.assertTrue(tools['machines']['readOnlyHint'])



class CmdShimTest(AgentCase):
    """Security M2: on Windows a prompt could break out of cmd.exe when claude is npm's .cmd shim."""
    def test_cmd_metacharacters_never_reach_a_cmd_shim(self):
        for exe in ('C:\\npm\\claude.cmd', 'C:\\npm\\CLAUDE.BAT'):
            for prompt in ('x" & calc & "', 'a | b', '100%', 'two\nlines'):
                with self.assertRaises(ValueError, msg=(exe, prompt)):
                    self.sb.launch_argv({'prompt': prompt}, exe=exe)
            self.assertEqual(self.sb.launch_argv({'prompt': 'fix the login bug'}, exe=exe)[-1], 'fix the login bug')
        self.assertEqual(self.sb.launch_argv({'prompt': 'x" & calc & "'}, exe='C:\\bin\\claude.exe')[-1], 'x" & calc & "')

    def test_the_native_exe_wins_on_windows(self):
        with mock.patch.object(self.sb.os, 'name', 'nt'), \
                mock.patch.object(self.sb.shutil, 'which', lambda n: {'claude.exe': 'C:\\bin\\claude.exe', 'claude': 'C:\\npm\\claude.CMD'}.get(n)):
            self.assertEqual(self.sb._claude_exe(), 'C:\\bin\\claude.exe')



class UploadPartsTest(unittest.TestCase):
    """Security L7: unfinished uploads piled up without limit, and two uploads of one name could overwrite."""
    setUp, put = tu.AgentUploadTest.setUp, tu.AgentUploadTest.put

    def test_unfinished_uploads_are_capped(self):
        with mock.patch.object(self.sb, 'UPLOAD_PARTS_MAX', 3000):
            for i in range(5):
                self.sb.rpc_upload({'id': f'{i:032x}', 'name': 'x', 'offset': 0, 'size': 2000, 'last': False,
                                    'data': tu.base64.b64encode(b'a' * 1000).decode()})
                time.sleep(0.01)
            parts = sorted(p.name for p in self.sb.uploads_root().glob('.part-*'))
        self.assertEqual(parts, [f'.part-{3:032x}', f'.part-{4:032x}'], 'the oldest went first')

    def test_a_name_taken_meanwhile_is_never_overwritten(self):
        first = Path(self.put(b'one', 'notes.txt')['path'])
        real_exists = Path.exists
        with mock.patch.object(Path, 'exists', lambda p: False if p.name.startswith('notes') else real_exists(p)):
            second = Path(self.put(b'two', 'notes.txt', uid='cd' * 16)['path'])  # as if the check raced the other upload
        self.assertEqual((first.read_bytes(), second.read_bytes()), (b'one', b'two'))
        self.assertEqual(second.name, 'notes-2.txt')



class StateFolderModeTest(unittest.TestCase):
    """Security L9: the agent's state (handoffs, uploads, the log) was readable by other local users."""
    @unittest.skipIf(os.name == 'nt', 'POSIX modes')
    def test_the_switchboard_folder_is_private(self):
        from tests.helpers import fresh_sb
        tmp = mkdtemp()
        old = os.umask(0o022)
        try:
            sb = fresh_sb(tmp)
            sb.log('hello')
        finally:
            os.umask(old)
        self.assertEqual(sb.SB_HOME.stat().st_mode & 0o777, 0o700)



class LogRotationTest(unittest.TestCase):
    """Security L8: the agent log never rotated (the rotation part)."""
    def test_the_log_rolls_over(self):
        from tests.helpers import fresh_sb
        sb = fresh_sb(mkdtemp())
        with mock.patch.object(sb, 'LOG_MAX', 100):
            for i in range(20):
                sb.log(f'line {i}')
        p = sb.paths()['log']
        self.assertLessEqual(p.stat().st_size, 200)
        self.assertTrue(p.with_name('sb.log.1').exists())
        self.assertIn('line 19', p.read_text())



class HookCommandQuotingTest(AgentCase):
    """Security I8: the hook command was built without quoting, so a home folder with a space broke it."""
    def test_paths_with_spaces_are_quoted_and_found_again(self):
        sb_path = 'C:\\Users\\you two\\.claude\\switchboard\\sb.py'
        s = self.sb.merge_settings({}, sb_path, 'C:/Program Files/Python313/python.exe')
        cmd = s['hooks']['Stop'][0]['hooks'][0]['command']
        self.assertEqual(cmd, '"C:/Program Files/Python313/python.exe" "C:/Users/you two/.claude/switchboard/sb.py" hook')
        again = self.sb.merge_settings(s, sb_path, 'C:/Program Files/Python313/python.exe')
        self.assertEqual(len(again['hooks']['Stop']), 1, 'installing again replaces it')
        self.assertEqual(self.sb.merge_settings({}, '/h/sb.py', 'python3')['statusLine']['command'], 'python3 /h/sb.py statusline')



class HookSessionIdTest(AgentCase):
    """Security I9: hook state paths used the hook's session_id without checking it."""
    def test_a_path_as_session_id_writes_nothing(self):
        for sid in ('../../evil', 'a/b', '..', 'x' * 65, 7):
            self.sb.write_state({'session_id': sid, 'context_window': {'used_percentage': 5}}, tr.NOW)
            self.assertIsNone(self.sb.handle_hook({'hook_event_name': 'SessionStart', 'session_id': sid}, tr.NOW, 1,
                                                  spawn=lambda *a: None, notify=lambda *a: None))
        self.assertFalse(list(self.tmp.parent.glob('evil*')) + list(self.tmp.rglob('evil*')))
        self.assertEqual([p.name for p in self.sb.paths()['state'].glob('*.json') if '..' in p.name or '/' in p.name], [])
        self.sb.write_state({'session_id': tr.SID_A}, tr.NOW)
        self.assertTrue((self.sb.paths()['state'] / f'{tr.SID_A}.json').exists())



class HostCharactersTest(unittest.TestCase):
    """Security I6: the request's Host went into install lines and terminal URLs unchecked."""
    def test_only_host_characters_get_through(self):
        import term
        self.assertEqual(term.ws_host('studio.tail1234.ts.net:8130'), 'studio.tail1234.ts.net')
        self.assertEqual(term.ws_host('[fd7a::5]:8130'), '[fd7a::5]')
        for bad in ('x;rm -rf ~', 'a b:8130', 'h$(id):1', "q'uote", 'x"y'):
            self.assertEqual(term.ws_host(bad), '127.0.0.1', bad)



class KeyModeTest(unittest.TestCase):
    """Security I3: term.key was read without checking its mode."""
    @unittest.skipIf(os.name == 'nt', 'POSIX modes')
    def test_a_loosened_key_is_made_private_again(self):
        import term
        p = Path(mkdtemp()) / 'term.key'
        k = term.load_or_create_key(p)
        os.chmod(p, 0o644)
        self.assertEqual(term.load_or_create_key(p), k)
        self.assertEqual(p.stat().st_mode & 0o777, 0o600)



class MachinesFileModeTest(unittest.TestCase):
    """Security I5: machines.json (whose origins the hub trusts) was written with the default umask."""
    @unittest.skipIf(os.name == 'nt', 'POSIX modes')
    def test_machines_json_is_private(self):
        import term
        p = Path(mkdtemp()) / 'machines.json'
        old = os.umask(0o022)
        try:
            term.write_machines(p, [{'name': 'desk', 'label': 'Desk', 'target': 'me@x', 'os': 'mac'}], ['http://10.0.0.5:8130'])
        finally:
            os.umask(old)
        self.assertEqual(p.stat().st_mode & 0o777, 0o600)
        self.assertEqual(json.loads(p.read_text())['origins'], ['http://10.0.0.5:8130'])



class ApnsKeyNameTest(unittest.TestCase):
    """Security I7: one developer's APNs key file name was written into the server."""
    def test_the_key_file_comes_from_apns_json(self):
        f = tn.server.apns_key_path
        self.assertEqual(f('/c', {'key_id': 'ABC123'}), Path('/c/apns/AuthKey_ABC123.p8'))
        self.assertEqual(f('/c', {'key_id': 'ABC123', 'key': 'mine.p8'}), Path('/c/apns/mine.p8'))
        self.assertEqual(f('/c', {'key_id': 'x', 'key': '../../etc/passwd'}), Path('/c/apns/passwd'))
        self.assertNotIn('WX4HQNUZ3N', (Path(tn.server.__file__)).read_text())



class InstallLineTempFileTest(unittest.TestCase):
    """Security H6 (the part fixed here): the install line ran a fixed /tmp/switchboard-node another user could plant."""
    setUp, pair = PairDownloadsTest.setUp, PairDownloadsTest.pair

    def test_the_unix_lines_use_mktemp(self):
        (self.cfg / 'downloads.json').write_text('{"url": "http://10.0.0.5:8131/app"}')
        for os_ in ('mac', 'linux'):
            line = self.pair()['install'][os_]
            self.assertTrue(line.startswith('n=$(mktemp) && '), line)
            self.assertNotIn('/tmp/switchboard-node', line)

    def test_the_line_runs_in_sh(self):
        import subprocess
        (self.cfg / 'downloads.json').write_text('{"url": "http://127.0.0.1:9/app"}')  # nothing listens: curl fails first
        r = subprocess.run(['sh', '-c', self.pair()['install']['linux']], capture_output=True, text=True, timeout=20)
        self.assertNotEqual(r.returncode, 0)
        self.assertIn('curl', r.stderr)


if __name__ == '__main__':
    unittest.main()
