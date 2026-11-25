"""Turning keys on and off (server/keys.py's own command, POST /api/keys/mode), a code that is only a link when the
home has no relay, what Settings says about the downloads with keys only, and the real server started with a port
and folders of its own (SWITCHBOARD_PORT, SWITCHBOARD_CONFIG_DIR, SWITCHBOARD_STATE_DIR)."""
import contextlib, io, json, os, re, signal, socket, subprocess, sys, time, unittest, urllib.error, urllib.request
from pathlib import Path
from tempfile import mkdtemp
from unittest import mock

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'server'))
import keys, nodes, server, serverctl  # noqa: E402
from tests.test_keys import DAY, NOW, FakeControl, app_for, quiet, write  # noqa: E402

SERVER_JSON = {'bind': ['0.0.0.0'], 'hosts': ['studio'], 'origins': ['http://studio:8130'], 'mine': 'kept as it is'}


def free_port():
    with socket.socket() as s:
        s.bind(('127.0.0.1', 0))
        return s.getsockname()[1]


def cfg_dir(**kw):
    d = Path(mkdtemp()) / 'cfg'
    write(d / 'server.json', dict(SERVER_JSON, **kw))
    return d


class SetModeTest(unittest.TestCase):
    def test_the_mode_is_written_and_everything_else_in_the_file_stays(self):
        d = cfg_dir()
        self.assertEqual(keys.set_mode(d / 'server.json', 'both'), 'off')
        self.assertEqual(json.loads((d / 'server.json').read_text()), dict(SERVER_JSON, keys='both'))
        self.assertEqual(keys.set_mode(d / 'server.json', 'only'), 'both')
        self.assertEqual(keys.read_mode(d / 'server.json'), 'only')
        # the copy beside it is of the new file: a damaged server.json never goes back to a mode that was left
        self.assertEqual(json.loads((d / 'server.json.bak').read_text())['keys'], 'only')
        self.assertEqual(sorted(p.name for p in d.iterdir()), ['server.json', 'server.json.bak'])

    def test_a_mode_nobody_knows_is_not_written(self):
        d = cfg_dir()
        for bad in ('onyl', '', None, 5, 'ON'):
            with self.assertRaises(ValueError):
                keys.set_mode(d / 'server.json', bad)
        self.assertEqual(json.loads((d / 'server.json').read_text()), SERVER_JSON)

    def test_no_server_json_or_a_damaged_one_is_left_alone(self):
        d = Path(mkdtemp())
        with self.assertRaises(RuntimeError) as e:
            keys.set_mode(d / 'server.json', 'both')
        self.assertIn('Start Switchboard once first', str(e.exception))
        self.assertEqual(list(d.iterdir()), [])
        (d / 'server.json').write_bytes(b'\x00' * 40)
        with self.assertRaises(RuntimeError) as e:
            quiet(keys.set_mode, d / 'server.json', 'both')
        self.assertIn('could not be read', str(e.exception))


class Readiness:
    """Stands in for the server's GET /api/keys/readiness."""

    def __init__(self, report):
        from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
        self.report, me = report, self

        class H(BaseHTTPRequestHandler):
            def log_message(self, *a):
                pass

            def do_GET(self):
                b = json.dumps(me.report).encode()
                self.send_response(200 if self.path.startswith('/api/keys/readiness') or self.path == '/status' else 404)
                self.send_header('Content-Length', str(len(b)))
                self.end_headers()
                self.wfile.write(b)
        self.httpd = ThreadingHTTPServer(('127.0.0.1', 0), H)
        self.url = f'http://127.0.0.1:{self.httpd.server_address[1]}'
        import threading
        threading.Thread(target=self.httpd.serve_forever, daemon=True).start()

    def close(self):
        self.httpd.shutdown()
        self.httpd.server_close()


BLOCKED = {'ok': True, 'keys': 'both', 'ready': False, 'days': 7, 'now': NOW, 'blockers': [
    {'kind': 'machine', 'text': 'Tower is not paired with this home computer. Pair it with a code first, or it loses its connection.'},
    {'kind': 'watching', 'text': 'Connections by address have been watched for 2 of 7 days. Until then nobody can say that none is still in use.'}],
    'machines': [{'name': 'studio', 'label': 'Studio', 'home': True, 'paired': False, 'seen': None, 'path': None},
                 {'name': 'lab', 'label': 'Lab', 'home': False, 'paired': True, 'seen': NOW - 60, 'path': 'relay'},
                 {'name': 'tower', 'label': 'Tower', 'home': False, 'paired': False, 'seen': None, 'path': None}],
    'direct': []}
READY = dict(BLOCKED, ready=True, blockers=[])


class CommandTest(unittest.TestCase):
    """python keys.py on|only|off|status|wait: what `switchboard keys ...` runs. It writes server.json and says what
    to do next; the restart is the caller's."""

    def setUp(self):
        self.cfg = cfg_dir()
        self.srv = Readiness(BLOCKED)
        self.addCleanup(self.srv.close)

    def run_cmd(self, *args, server=None):
        out = io.StringIO()
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(out):
            code = keys.main([*args, '--config', str(self.cfg), '--server', server or self.srv.url])
        return code, out.getvalue()

    def mode(self):
        return keys.read_mode(self.cfg / 'server.json')

    def test_on_and_off(self):
        self.assertEqual(self.run_cmd('on'), (0, ''))
        self.assertEqual(self.mode(), 'both')
        code, out = self.run_cmd('on')
        self.assertEqual((code, out), (keys.SAME, 'Keys are already on.\n'))
        self.assertEqual(self.run_cmd('off'), (0, ''))
        self.assertEqual(self.mode(), 'off')
        self.assertEqual(self.run_cmd('off'), (keys.SAME, 'Keys are already off.\n'))

    def test_only_is_refused_until_the_report_says_ready(self):
        keys.set_mode(self.cfg / 'server.json', 'both')
        code, out = self.run_cmd('only')
        self.assertEqual(code, 1)
        self.assertEqual(self.mode(), 'both')
        self.assertEqual(out, '''Not yet: switching to keys only now would cut something off.
  Tower is not paired with this home computer. Pair it with a code first, or it loses its connection.
  Connections by address have been watched for 2 of 7 days. Until then nobody can say that none is still in use.
Nothing was changed. To switch anyway: switchboard keys only --force
''')
        self.srv.report = READY
        self.assertEqual(self.run_cmd('only'), (0, ''))
        self.assertEqual(self.mode(), 'only')
        self.assertEqual(self.run_cmd('only'), (keys.SAME, 'This home computer already uses keys only.\n'))

    def test_only_with_force_says_what_it_may_cut_off(self):
        keys.set_mode(self.cfg / 'server.json', 'both')
        code, out = self.run_cmd('only', '--force')
        self.assertEqual(code, 0)
        self.assertEqual(self.mode(), 'only')
        self.assertEqual(out, '''Warning: switching to keys only although something may be cut off:
  Tower is not paired with this home computer. Pair it with a code first, or it loses its connection.
  Connections by address have been watched for 2 of 7 days. Until then nobody can say that none is still in use.
To go back: switchboard keys on
''')

    def test_only_with_no_server_to_ask_is_refused_too(self):
        keys.set_mode(self.cfg / 'server.json', 'both')
        code, out = self.run_cmd('only', server='http://127.0.0.1:9')
        self.assertEqual(code, 1)
        self.assertEqual(self.mode(), 'both')
        self.assertIn('The server on this computer did not answer', out)
        self.assertIn('Nothing was changed.', out)
        code, out = self.run_cmd('only', '--force', server='http://127.0.0.1:9')
        self.assertEqual((code, self.mode()), (0, 'only'))
        self.assertIn('Warning', out)

    def test_status_in_plain_words(self):
        self.assertEqual(self.run_cmd('status'), (0, '''Keys: off. Computers reach this home computer by address.
To turn keys on: switchboard keys on
'''))
        keys.set_mode(self.cfg / 'server.json', 'both')
        code, out = self.run_cmd('status')
        self.assertEqual(code, 0)
        self.assertEqual(out, '''Keys: on. Computers reach this home computer with a key or by address.
  Studio  this home computer
  Lab  paired, last connected with its key 1 minute ago (through the relay)
  Tower  not paired
Not ready for keys only yet:
  Tower is not paired with this home computer. Pair it with a code first, or it loses its connection.
  Connections by address have been watched for 2 of 7 days. Until then nobody can say that none is still in use.
To pair a computer: switchboard code
''')
        self.srv.report = READY
        self.assertIn('Ready for keys only. To close the ports to the network: switchboard keys only\n', self.run_cmd('status')[1])
        keys.set_mode(self.cfg / 'server.json', 'only')
        self.srv.report = dict(READY, keys='only')
        out = self.run_cmd('status')[1]
        self.assertTrue(out.startswith('Keys: only. Computers reach this home computer with a key, and its ports are closed to the network.\n'), out)
        self.assertIn('To open the addresses again: switchboard keys on\n', out)
        # nothing answers: the mode in the file, and that the rest is not known
        code, out = self.run_cmd('status', server='http://127.0.0.1:9')
        self.assertEqual(code, 0)
        self.assertIn('Keys: only.', out)
        self.assertIn('The server on this computer did not answer', out)

    def test_wait_says_the_next_step_once_the_node_answers(self):
        keys.set_mode(self.cfg / 'server.json', 'both')
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            code = keys.main(['wait', '--config', str(self.cfg), '--control', self.srv.url, '--within', '5'])
        self.assertEqual(code, 0)
        self.assertEqual(out.getvalue(), '''Keys are on. This home computer still answers by address as well.
Next: run "switchboard code" here, and on each other computer run the line it prints.
"switchboard keys status" says when it is safe to close the ports with "switchboard keys only".
''')
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            code = keys.main(['wait', '--config', str(self.cfg), '--control', 'http://127.0.0.1:9', '--within', '0.3'])
        self.assertEqual(code, 1)
        self.assertEqual(out.getvalue(), '''Keys are on in server.json, but the node on this home computer did not answer yet.
Check it with: switchboard-node status
''')

    def test_wait_after_off_and_only(self):
        for mode, words in (('off', 'Keys are off. Computers reach this home computer by address, as before.\n'),
                            ('only', 'This home computer now uses keys only: its ports are closed to the network.\n'
                                     'To open the addresses again: switchboard keys on\n')):
            keys.set_mode(self.cfg / 'server.json', mode)
            out = io.StringIO()
            with contextlib.redirect_stdout(out):
                code = keys.main(['wait', '--config', str(self.cfg), '--control', self.srv.url, '--within', '5'])
            self.assertEqual((code, out.getvalue()), (0, words))

    def test_a_word_it_does_not_know(self):
        with self.assertRaises(SystemExit) as e, contextlib.redirect_stderr(io.StringIO()):
            keys.main(['sideways'])
        self.assertEqual(e.exception.code, 2)

    def test_on_windows_every_command_it_names_is_the_one_installed_there(self):
        # A PC has no "switchboard" command: the installer puts switchboard.ps1 there, and its own sentences call
        # it that ("Link this computer first: switchboard.ps1 link").
        said = []
        with mock.patch.object(keys, 'WINDOWS', True):
            said.append(self.run_cmd('status')[1])
            keys.set_mode(self.cfg / 'server.json', 'both')
            said.append(self.run_cmd('status')[1])
            said.append(self.run_cmd('only')[1])
            self.srv.report = READY
            said.append(self.run_cmd('status')[1])
            said.append(self.run_cmd('status', server='http://127.0.0.1:9')[1])
            self.srv.report = BLOCKED
            said.append(self.run_cmd('only', '--force')[1])
            for mode in ('both', 'only'):
                keys.set_mode(self.cfg / 'server.json', mode)
                out = io.StringIO()
                with contextlib.redirect_stdout(out):
                    keys.main(['wait', '--config', str(self.cfg), '--control', self.srv.url, '--within', '5'])
                said.append(out.getvalue())
            report = keys.readiness('off', [], [], {'since': NOW, 'seen': {}}, NOW)
            said.extend(b['text'] for b in report['blockers'] if b['kind'] == 'mode')
            with self.assertRaises(RuntimeError) as e:
                keys.set_mode(self.cfg / 'nowhere' / 'server.json', 'both')
            said.append(str(e.exception))
        said = '\n'.join(said)
        for line in ('To turn keys on: switchboard.ps1 keys on', 'To pair a computer: switchboard.ps1 code',
                     'To switch anyway: switchboard.ps1 keys only --force', 'To go back: switchboard.ps1 keys on',
                     'To close the ports to the network: switchboard.ps1 keys only', 'Start it with: switchboard.ps1 start',
                     'Next: run "switchboard.ps1 code" here', '"switchboard.ps1 keys status" says',
                     'with "switchboard.ps1 keys only".', 'To open the addresses again: switchboard.ps1 keys on',
                     'Run "switchboard.ps1 keys on" on this home computer first.',
                     'Start Switchboard once first: switchboard.ps1 start'):
            self.assertIn(line, said)
        self.assertIsNone(re.search(r'switchboard (keys|code|start|link)\b', said), said)
        # ... and switchboard.ps1 as it is installed answers to each of those words
        ps1 = (ROOT / 'scripts' / 'install' / 'switchboard.ps1').read_text(encoding='utf-8')
        verbs = set(re.findall(r'switchboard\.ps1 (\w+)', said))
        self.assertEqual(verbs, {'keys', 'code', 'start'})
        for verb in verbs:
            self.assertRegex(ps1, rf"(?m)^    (\{{ \$_ -in [^}}]*)?'{verb}'[^\n]*\{{", verb)
        for what in ('on', 'only', 'off', 'status'):
            self.assertIn(f"'{what}'", re.search(r"\$what -notin ([^)]*)\)", ps1).group(1))
        self.assertIn("$more[0] -eq '--force'", ps1)


class ModeRouteTest(unittest.TestCase):
    """POST /api/keys/mode {"keys": "off"|"both"|"only", "force": false}: writes server.json and restarts every
    part, the way Settings > Server restarts. "only" is refused until the report says ready, unless forced."""

    def setUp(self):
        self.control = FakeControl()
        self.addCleanup(self.control.close)
        old = nodes.CONTROL
        nodes.CONTROL = self.control.url
        self.addCleanup(setattr, nodes, 'CONTROL', old)
        self.d = Path(mkdtemp())
        write(self.d / 'cfg' / 'server.json', dict(SERVER_JSON, keys='both'))
        self.cfg, _ = quiet(server.ServerConfig, self.d / 'cfg', self.d / 'state')
        self.now = [NOW]
        self.app = app_for(self.cfg, clock=lambda: self.now[0])
        self.app.cfg_dir = self.d / 'cfg'
        self.restarts = []
        for name, value in (('why_not', lambda m=None: None), ('stuck', lambda m=None: None), ('restart', lambda: self.restarts.append(1) or {'ok': True})):
            p = mock.patch.object(serverctl, name, value)
            p.start()
            self.addCleanup(p.stop)

    def mode(self):
        return keys.read_mode(self.d / 'cfg' / 'server.json')

    def test_off_and_back_on(self):
        self.assertEqual(self.app.post('/api/keys/mode', {'keys': 'off'}), {'ok': True, 'keys': 'off', 'restarting': True})
        self.assertEqual((self.mode(), len(self.restarts)), ('off', 1))
        self.assertEqual(self.app.post('/api/keys/mode', {'keys': 'both'}), {'ok': True, 'keys': 'both', 'restarting': True})
        self.assertEqual((self.mode(), len(self.restarts)), ('both', 2))

    def test_the_mode_it_already_has_restarts_nothing(self):
        self.assertEqual(self.app.post('/api/keys/mode', {'keys': 'both'}), {'ok': True, 'keys': 'both', 'restarting': False})
        self.assertEqual(self.restarts, [])

    def test_only_is_refused_until_ready(self):
        with self.assertRaises(server.HttpError) as e:
            self.app.post('/api/keys/mode', {'keys': 'only'})
        self.assertEqual(e.exception.code, 409)
        self.assertEqual(str(e.exception), 'Not yet: switching to keys only now would cut something off. '
                                           'Tower is paired but has not connected with its key yet.')
        self.assertEqual([b['kind'] for b in e.exception.extra['blockers']], ['machine', 'watching'])
        self.assertEqual((self.mode(), self.restarts), ('both', []))
        # the tower connects and a week passes
        self.control.devices[1]['last_seen'] = NOW
        self.now[0] += 8 * DAY
        self.assertEqual(self.app.post('/api/keys/mode', {'keys': 'only'}), {'ok': True, 'keys': 'only', 'restarting': True})
        self.assertEqual((self.mode(), len(self.restarts)), ('only', 1))

    def test_only_when_asked_to_anyway(self):
        for not_force in (None, 0, 'yes', 1):
            with self.assertRaises(server.HttpError):
                self.app.post('/api/keys/mode', {'keys': 'only', 'force': not_force})
        r = self.app.post('/api/keys/mode', {'keys': 'only', 'force': True})
        self.assertEqual(r, {'ok': True, 'keys': 'only', 'restarting': True})
        self.assertEqual(self.mode(), 'only')

    def test_a_mode_nobody_knows(self):
        for bad in (None, '', 'onyl', 5, ['both'], 'ONLY'):
            with self.assertRaises(server.HttpError) as e:
                self.app.post('/api/keys/mode', {'keys': bad})
            self.assertEqual(e.exception.code, 400, bad)
        self.assertEqual((self.mode(), self.restarts), ('both', []))

    def refused(self, stuck, body):
        with mock.patch.object(serverctl, 'stuck', lambda m=None: stuck):
            with self.assertRaises(server.HttpError) as e:
                self.app.post('/api/keys/mode', body)
        self.assertEqual(e.exception.code, 409)
        self.assertEqual((self.mode(), self.restarts), ('both', []))  # nothing written, nothing restarted
        return str(e.exception), e.exception.extra

    def test_a_server_started_by_hand_says_so_once_and_names_what_to_run(self):
        said, extra = self.refused('checkout', {'keys': 'off'})
        script = Path(server.__file__).resolve().parent / 'keys.py'
        command = f"python3 {script} off --config {self.d / 'cfg'}"
        self.assertEqual(extra, {'command': command})
        # one message: what happened, why, and what to do, ending with the command the app shows apart
        self.assertEqual(said, 'Nothing was changed. This server was started by hand from a copy of the code, so it cannot '
                               'restart itself. To turn keys off, run this on the home computer, then start the server '
                               f'again: {command}')
        self.assertNotIn('git pull', said)  # the words about updating are Settings > Server's
        said, extra = self.refused('checkout', {'keys': 'only', 'force': True})
        self.assertIn(' To switch to keys only, run this on the home computer, ', said)
        self.assertTrue(extra['command'].startswith(f'python3 {script} only'))

    def test_on_windows_the_refusal_names_the_powershell_command(self):
        said, extra = self.refused('windows', {'keys': 'off'})
        command = '& "$env:LOCALAPPDATA\\Switchboard\\switchboard.ps1" keys off'
        self.assertEqual(extra, {'command': command})
        self.assertEqual(said, 'Nothing was changed. On Windows the server cannot restart itself from the app. To turn keys '
                               f'off, run this in PowerShell on the home computer: {command}')
        self.assertNotIn('installer', said)

    def test_without_the_switchboard_command_there_is_nothing_to_copy(self):
        said, extra = self.refused('command', {'keys': 'off'})
        self.assertEqual(extra, {})
        self.assertEqual(said, 'Nothing was changed. This server cannot restart itself, because the switchboard command is '
                               'not in ~/.local/bin on the home computer. Run the install line there once, then try again.')

    def test_the_refusal_is_one_message_in_every_case(self):
        for stuck in ('checkout', 'windows', 'command'):
            for want, aim in (('both', 'turn keys on'), ('off', 'turn keys off'), ('only', 'switch to keys only')):
                text, command = keys.by_hand(stuck, want, self.d / 'cfg' / 'server.json', Path('/srv/sb/server'))
                self.assertTrue(text.startswith('Nothing was changed. '), text)
                self.assertEqual(text.count('cannot restart itself'), 1, text)
                self.assertNotIn('  ', text)
                self.assertFalse(any(c in text for c in '\u2014!'), text)
                if command:
                    self.assertIn(f'To {aim}, run this ', text)
                    self.assertTrue(text.endswith(': ' + command), text)
        # the usual settings folder needs no naming; a path with a space is quoted
        text, command = keys.by_hand('checkout', 'both', Path.home() / '.config' / 'switchboard' / 'server.json',
                                     Path('/srv/my sb/server'))
        self.assertEqual(command, "python3 '/srv/my sb/server/keys.py' on")

    def test_turning_keys_on_needs_the_node_on_this_computer(self):
        keys.set_mode(self.d / 'cfg' / 'server.json', 'off')
        self.cfg, _ = quiet(server.ServerConfig, self.d / 'cfg', self.d / 'state')
        self.app.server_cfg = self.cfg
        with mock.patch.object(nodes, 'CONTROL', 'http://127.0.0.1:9'):
            with self.assertRaises(server.HttpError) as e:
                self.app.post('/api/keys/mode', {'keys': 'both'})
        self.assertEqual(e.exception.code, 409)
        self.assertIn('Keys need switchboard-node on this home computer', str(e.exception))
        self.assertEqual(e.exception.extra, {'command': 'switchboard link'})
        self.assertEqual((self.mode(), self.restarts), ('off', []))
        # a node that answers "keys are off" is a node: that is what it says until the mode changes
        self.control.fail = (503, {'ok': False, 'error': 'Keys are off on this computer.', 'kind': 'off'})
        self.assertEqual(self.app.post('/api/keys/mode', {'keys': 'both'}), {'ok': True, 'keys': 'both', 'restarting': True})
        self.assertEqual((self.mode(), len(self.restarts)), ('both', 1))

    def test_going_to_keys_only_stops_the_announcement_before_the_restart(self):
        order = []
        self.control.devices[1]['last_seen'] = NOW
        self.now[0] += 8 * DAY
        with mock.patch.object(server, 'unannounce', lambda: order.append('unannounce')), \
                mock.patch.object(serverctl, 'restart', lambda: order.append('restart')):
            self.app.post('/api/keys/mode', {'keys': 'only'})
            self.assertEqual(order, ['unannounce', 'restart'])
            self.app.server_cfg = quiet(server.ServerConfig, self.d / 'cfg', self.d / 'state')[0]
            self.app.post('/api/keys/mode', {'keys': 'both'})
        self.assertEqual(order, ['unannounce', 'restart', 'restart'], 'only "only" stops it: the others still answer by address')

    def test_over_http_it_is_guarded_like_every_other_write(self):
        import threading
        peer = ['127.0.0.1']

        class From(server.make_handler(self.app)):
            def setup(self):
                super().setup()
                self.client_address = (peer[0], 50000)
        httpd = server.Server(('127.0.0.1', 0), From)
        threading.Thread(target=httpd.serve_forever, daemon=True).start()
        self.addCleanup(httpd.server_close)
        self.addCleanup(httpd.shutdown)

        def post(headers, host='localhost'):
            r = urllib.request.Request(f'http://127.0.0.1:{httpd.server_address[1]}/api/keys/mode', data=b'{"keys": "off"}',
                                       headers=dict({'Host': host}, **headers), method='POST')
            try:
                with urllib.request.urlopen(r, timeout=5) as resp:
                    return resp.status
            except urllib.error.HTTPError as e:
                return e.code
        good = {'Content-Type': 'application/json', 'X-Switchboard': '1'}
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            self.assertIn(post({'Content-Type': 'application/json'}), (400, 403), 'no X-Switchboard header')
            self.assertEqual(post(good, host='evil.example.com'), 403)
            peer[0] = '203.0.113.9'
            self.assertEqual(post(good), 403)
            self.assertEqual((self.mode(), self.restarts), ('both', []))
            peer[0] = '127.0.0.1'
            self.assertEqual(post(good), 200)
        self.assertEqual((self.mode(), len(self.restarts)), ('off', 1))


class PairDeviceTest(unittest.TestCase):
    def test_a_home_with_no_relay_gives_a_link_and_no_code(self):
        control = FakeControl()
        self.addCleanup(control.close)
        with mock.patch.object(nodes, 'control_call', lambda *a, **kw: {'ok': True, 'code': None, 'expires': 600,
                                                                      'link': 'switchboard://pair/1?h=x&s=y'}):
            self.assertEqual(nodes.pair_device(), {'ok': True, 'code': None, 'link': 'switchboard://pair/1?h=x&s=y', 'expires': 600})
        # still not passed on: an answer with neither, or with something that is no code
        for bad in ({'ok': True}, {'ok': True, 'code': None}, {'ok': True, 'code': 5, 'link': 'switchboard://pair/1?h=x'},
                    {'ok': True, 'code': None, 'link': 7}, {'ok': True, 'code': 'ABCD-EFGH'}):
            with mock.patch.object(nodes, 'control_call', lambda *a, **kw: bad):
                with self.assertRaises(OSError):
                    nodes.pair_device()


class DownloadsReportTest(unittest.TestCase):
    def test_with_keys_only_they_are_not_served_and_no_port_is_asked(self):
        asked = []
        with mock.patch.object(serverctl, '_probe', lambda url, timeout=1.5: asked.append(url) or b'x'):
            self.assertEqual(serverctl.downloads('only'),
                             {'running': False, 'served': False,
                              'note': 'Not served with keys only. A new computer pairs with a code or a link.'})
            self.assertEqual(asked, [])
            self.assertEqual(serverctl.downloads('both'), {'running': True, 'served': True})
            self.assertEqual(serverctl.downloads(), {'running': True, 'served': True})
            self.assertEqual(asked, ['http://127.0.0.1:8131/node/SHA256SUMS'] * 2)
            self.assertEqual(serverctl.info(Path(mkdtemp()), keys='only')['downloads']['served'], False)


class ShutdownTest(unittest.TestCase):
    def test_what_was_seen_by_address_is_written_when_the_server_stops(self):
        now = [NOW]
        cfg, _ = quiet(server.ServerConfig, cfg_dir(keys='both'), Path(mkdtemp()))
        app = app_for(cfg, clock=lambda: now[0])
        path = app.state_dir / 'direct_seen.json'
        app.seen_from('10.0.0.7', 'SwitchboardApp/1')
        now[0] += 30
        app.seen_from('10.0.0.7', 'SwitchboardApp/1')
        self.assertEqual(json.loads(path.read_text())['seen'], {'iPhone': NOW}, 'not written on every request')
        app.shutdown()
        self.assertEqual(json.loads(path.read_text())['seen'], {'iPhone': NOW + 30})


class AnnouncementTest(unittest.TestCase):
    """The program that announces the server on the network (dns-sd, avahi-publish-service) is a child of the
    server. Nothing ended it when the server ended, so it went on announcing a server that was gone."""

    def setUp(self):
        import bonjour
        self.bonjour = bonjour
        self.addCleanup(bonjour.stop)
        bonjour.stop()

    def announce(self, popen, tool='avahi-publish-service'):
        cfg = mock.Mock(origins=['http://10.0.0.5:8130'])
        return self.bonjour.announce(8130, cfg, lambda line: None, which=lambda t: f'/usr/bin/{t}' if t == tool else None,
                                     popen=popen, zeroconf=lambda: (None, None))

    def test_the_child_is_ended_when_the_server_stops(self):
        started = []

        def popen(args, **kw):  # a real process that would outlive us, standing in for the tool
            started.append((args, subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(600)'], **kw)))
            return started[-1][1]
        self.assertEqual(self.announce(popen), 'avahi')
        child = started[0][1]
        self.addCleanup(lambda: child.poll() is None and child.kill())
        self.assertEqual(started[0][0][0], '/usr/bin/avahi-publish-service')
        self.assertIsNone(child.poll(), 'announcing for as long as the server runs')
        cfg, _ = quiet(server.ServerConfig, cfg_dir(keys='both'), Path(mkdtemp()))
        said = []
        with mock.patch.object(server, 'log', said.append):
            app_for(cfg).shutdown()
        self.assertIsNotNone(child.poll(), 'the server stopped, and so did the announcement')
        self.assertIn('bonjour: no longer announcing', said)
        self.assertEqual(self.bonjour.stop(), 0, 'and a second stop has nothing to do')

    def test_a_child_that_ignores_being_asked_is_killed(self):
        class Stubborn:
            def __init__(self):
                self.calls, self.code = [], None

            def poll(self):
                return self.code

            def terminate(self):
                self.calls.append('terminate')

            def kill(self):
                self.calls.append('kill')
                self.code = -9

            def wait(self, timeout=None):
                self.calls.append('wait')
                if self.code is None:
                    raise subprocess.TimeoutExpired('avahi-publish-service', timeout)
                return self.code
        it = Stubborn()
        self.assertEqual(self.announce(lambda args, **kw: it, tool='dns-sd'), 'dns-sd')
        self.assertEqual(self.bonjour.stop(wait=0.01), 1)
        self.assertEqual(it.calls, ['terminate', 'wait', 'kill', 'wait'])

    def test_a_child_that_already_ended_and_a_stop_that_fails_never_raise(self):
        gone = mock.Mock()
        gone.poll.return_value = 0
        self.announce(lambda args, **kw: gone)
        self.assertEqual(self.bonjour.stop(), 1)
        gone.terminate.assert_not_called()
        bad = mock.Mock()
        bad.poll.side_effect = OSError('no such process')
        self.announce(lambda args, **kw: bad)
        said = []
        self.assertEqual(self.bonjour.stop(said.append), 0)
        self.assertIn('could not stop announcing', said[0])
        server.unannounce(lambda log: 1 / 0)  # whatever goes wrong, stopping the server goes on

    def test_zeroconf_takes_its_record_back(self):
        made = []

        class Zc:
            def __init__(self):
                self.closed = False
                made.append(self)

            def register_service(self, info):
                pass

            def close(self):
                self.closed = True
        cfg = mock.Mock(origins=['http://10.0.0.5:8130'])
        how = self.bonjour.announce(8130, cfg, lambda line: None, zeroconf=lambda: (lambda *a, **kw: object(), Zc))
        self.assertEqual(how, 'zeroconf')
        self.assertEqual(self.bonjour.stop(), 1)
        self.assertTrue(made[0].closed)


@unittest.skipIf(os.name == 'nt', 'signals')
class RealServerTest(unittest.TestCase):
    """server.py itself, started the way a scratch run starts it: a port and folders of its own."""

    def test_a_port_and_folders_of_its_own_and_a_clean_stop(self):
        d = Path(mkdtemp())
        port = free_port()
        write(d / 'cfg' / 'server.json', {'bind': ['127.0.0.1'], 'hosts': [], 'origins': [], 'keys': 'both'})
        write(d / 'cfg' / 'fleet.json', {'machines': []})
        home = d / 'home'
        home.mkdir()
        # the tools that announce on the network, stood in for: each writes its pid and stays until it is ended
        tools = d / 'tools'
        tools.mkdir()
        for tool in ('dns-sd', 'avahi-publish-service'):
            (tools / tool).write_text(f'#!/bin/sh\necho $$ > "{d}/announcer.pid"\nexec sleep 600\n')
            (tools / tool).chmod(0o755)
        env = dict(os.environ, HOME=str(home), SWITCHBOARD_PORT=str(port), SWITCHBOARD_CONFIG_DIR=str(d / 'cfg'),
                   PATH=f"{tools}{os.pathsep}{os.environ.get('PATH', '')}",
                   SWITCHBOARD_STATE_DIR=str(d / 'state'), SWITCHBOARD_HUB_URL='http://127.0.0.1:9',
                   SWITCHBOARD_NODE_CONTROL='http://127.0.0.1:9', PYTHONUNBUFFERED='1',
                   PYTHONDONTWRITEBYTECODE='1')  # (Apple's Python 3.9 keeps its compiled files under the home folder)
        log = open(d / 'log.txt', 'w')
        p = subprocess.Popen([sys.executable, str(ROOT / 'server' / 'server.py')], env=env, stdout=log, stderr=subprocess.STDOUT,
                             stdin=subprocess.DEVNULL)
        self.addCleanup(log.close)
        self.addCleanup(p.kill)
        got, end = None, time.time() + 20
        while got is None and time.time() < end and p.poll() is None:
            try:
                with urllib.request.urlopen(f'http://127.0.0.1:{port}/api/keys/readiness', timeout=2) as r:
                    got = json.loads(r.read())
            except OSError:
                time.sleep(0.2)
        said = (d / 'log.txt').read_text()
        self.assertIsNotNone(got, said)
        self.assertEqual(got['keys'], 'both')
        self.assertIn(f'switchboard on 127.0.0.1:{port}', said)
        self.assertTrue((d / 'state' / 'direct_seen.json').is_file(), 'the state is in the folder it was given')
        self.assertEqual([p.name for p in home.rglob('*') if p.is_file()], [], 'and nothing in the home folder')
        child = None
        if 'bonjour: announcing' in said and '(zeroconf)' not in said:  # (a machine with python-zeroconf has no child)
            pid_file, end = d / 'announcer.pid', time.time() + 10
            while time.time() < end and not (pid_file.exists() and pid_file.read_text().strip()):
                time.sleep(0.05)
            child = int(pid_file.read_text())
            self.addCleanup(lambda: self.alive(child) and os.kill(child, signal.SIGKILL))
            self.assertTrue(self.alive(child))
        p.send_signal(signal.SIGTERM)
        self.assertEqual(p.wait(timeout=10), 0, 'SIGTERM ends it cleanly')
        if child:
            self.assertFalse(self.alive(child), 'and the program that announced it on the network ended with it')
            self.assertIn('bonjour: no longer announcing', (d / 'log.txt').read_text())

    @staticmethod
    def alive(pid):
        try:
            os.kill(pid, 0)
            return True
        except OSError:
            return False

    def test_the_defaults_are_what_they_were(self):
        env = {k: v for k, v in os.environ.items() if not k.startswith('SWITCHBOARD_')}
        r = subprocess.run([sys.executable, '-c', 'import sys; sys.path.insert(0, sys.argv[1]); import server, serverctl, nodes\n'
                            'print(server.PORT, server.folders(), serverctl.HUB_URL, serverctl.DOWNLOADS_URL)',
                            str(ROOT / 'server')], env=dict(env, HOME='/home/you'), capture_output=True, text=True)
        self.assertEqual(r.stdout.strip(), "8130 (PosixPath('/home/you/.config/switchboard'), PosixPath('/home/you/.local/share/switchboard')) "
                                           "http://127.0.0.1:8132 http://127.0.0.1:8131", r.stderr)


if __name__ == '__main__':
    unittest.main()
