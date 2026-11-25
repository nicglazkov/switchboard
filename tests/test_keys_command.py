"""The `switchboard` command and the installers, for keys: `switchboard keys on|only|off|status`, the words passed on
to switchboard-node (pair, code, devices, forget), the downloads started as server/downloads.py (and an older one,
started as `python -m http.server 8131`, still found and stopped), `install-server.sh --keys`, and
`install-agent.sh --pair`. Run for real in a throwaway home folder, like tests/test_install_server.py: the server is
a stand-in that answers the readiness question from a file, switchboard-node is a script that writes down what it
was asked, and the node's control API is a few lines of HTTP on a free port."""
import http.server, json, os, shutil, subprocess, sys, tempfile, threading, time, unittest
from pathlib import Path

from tests import test_install_server as scripts

ROOT, INSTALL = scripts.ROOT, scripts.INSTALL
CTL = INSTALL / 'switchboard'

# The server: every start leaves a line with the mode it found, and GET /api/keys/readiness is a file's contents.
SERVER = '''import http.server, json, os
from pathlib import Path
home = Path(os.environ['HOME'])
conf = home / '.config/switchboard/server.json'
if not conf.exists():  # (the real server writes its settings the first time it starts)
    conf.parent.mkdir(parents=True, exist_ok=True)
    conf.write_text('{}')
try:
    mode = json.loads(conf.read_text()).get('keys', 'off')
except Exception as e:
    mode = 'unreadable'
with open(home / 'starts.log', 'a') as f:
    f.write(mode + '\\n')
class H(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        body = VERSION.encode()
        if self.path == '/api/keys/readiness':
            body = (home / 'readiness.json').read_bytes()
        self.send_response(200)
        self.send_header('Content-Length', str(len(body)))
        self.end_headers()
        self.wfile.write(body)
    def log_message(self, *a):
        pass
http.server.HTTPServer(('127.0.0.1', int(os.environ['SB_TEST_PORT'])), H).serve_forever()
'''
NODE = '''#!/bin/sh
echo "node $*" >> "$FAKE_LOG"
case "$1" in
  code) echo "Code: ABCD-EFGH"; echo "Link: switchboard://pair/1?h=x" ;;
  devices) echo "Laptop  computer  paired today" ;;
  relay) echo "The relay is now $2." ;;
  pair) [ -n "$FAKE_PAIR_FAILS" ] && { echo "switchboard-node: That code is not known." >&2; exit 1; }
        mkdir -p "$HOME/.config/switchboard"
        echo '{"hub": "", "machine": "laptop", "token": "t"}' > "$HOME/.config/switchboard/node.json"
        echo "Paired with Home. This computer is laptop there." ;;
  install) mkdir -p "$HOME/.local/bin"; cp "$0" "$HOME/.local/bin/switchboard-node"; chmod 755 "$HOME/.local/bin/switchboard-node" ;;
  run) exit 0 ;;
  uninstall) rm -f "$HOME/.config/switchboard/node.json" "$0"
             case " $* " in *" --purge "*) rm -f "$HOME/.config/switchboard/device.key"; echo "Removed this computer's key: x" ;; esac ;;
esac
exit 0
'''
NOW = 1_800_000_000
BLOCKED = {'ok': True, 'mode': 'both', 'ready': False, 'now': NOW,
           'machines': [{'name': 'lab', 'label': 'Lab', 'home': True, 'paired': True},
                        {'name': 'laptop', 'label': 'Laptop', 'home': False, 'paired': False}],
           'blockers': [{'kind': 'machine', 'name': 'laptop', 'text': 'Laptop is not paired yet.'}]}
READY = dict(BLOCKED, ready=True, blockers=[],
             machines=[BLOCKED['machines'][0],
                       {'name': 'laptop', 'label': 'Laptop', 'home': False, 'paired': True, 'seen': NOW - 120, 'path': 'direct'}])
ON = ('Keys are on. This home computer still answers by address as well.\n'
      'Next: run "switchboard code" here, and on each other computer run the line it prints.\n'
      '"switchboard keys status" says when it is safe to close the ports with "switchboard keys only".\n')


class Control:
    """The node's control API on this computer: GET /status, once `up` is set."""

    def __init__(self):
        me = self
        self.up, self.asked = True, 0

        class H(http.server.BaseHTTPRequestHandler):
            def do_GET(self):
                me.asked += 1
                data = json.dumps({'ok': True, 'role': 'home'}).encode() if me.up else b'{"ok": false, "error": "keys are off"}'
                self.send_response(200 if me.up else 503)
                self.send_header('Content-Length', str(len(data)))
                self.end_headers()
                self.wfile.write(data)

            def log_message(self, *a):
                pass
        self.httpd = http.server.ThreadingHTTPServer(('127.0.0.1', 0), H)
        self.url = f'http://127.0.0.1:{self.httpd.server_address[1]}'
        threading.Thread(target=self.httpd.serve_forever, daemon=True).start()

    def close(self):
        self.httpd.shutdown()
        self.httpd.server_close()


class Keys(scripts.Scripts):
    """The command with the server laid out by hand, as a plain background process."""

    def setUp(self):
        super().setUp()
        self.control = Control()
        self.addCleanup(self.control.close)
        self.cfg = self.home / '.config' / 'switchboard'
        self.env.update(SWITCHBOARD_PORT=str(self.port), SWITCHBOARD_NODE_CONTROL=self.control.url,
                        SWITCHBOARD_KEYS_WAIT='3')
        shutil.copytree(ROOT / 'server', self.share / 'app' / 'server', ignore=shutil.ignore_patterns('__pycache__', 'vendor'))
        (self.share / 'app' / 'server' / 'server.py').write_text("VERSION = 'here'\n" + SERVER)
        for d in (self.share / 'downloads', self.share / 'run', self.cfg, self.bin):
            d.mkdir(parents=True, exist_ok=True)
        (self.cfg / 'server.json').write_text('{"bind": "all"}\n')
        self.readiness(BLOCKED)

    def readiness(self, report):
        (self.home / 'readiness.json').write_text(json.dumps(report))

    def node(self):
        p = self.bin / 'switchboard-node'
        p.write_text(NODE)
        p.chmod(0o755)

    def sb(self, *args, **env):
        return self.run_script(CTL, *args, **env)

    def mode(self):
        return json.loads((self.cfg / 'server.json').read_text()).get('keys')

    def starts(self, n=0):
        """The mode each start of the server found, in order. The command starts the server in the background and
        does not wait for it, so a test that expects its `n`-th start waits for that one to be up (its line is
        written, and it answers) before reading the list: then the list is whole however slow the computer is."""
        f = self.home / 'starts.log'
        read = lambda: f.read_text().split() if f.exists() else []
        end = time.time() + 60
        while n and len(read()) < n and time.time() < end:
            time.sleep(0.05)
        if n:
            self.assertEqual(self.answer(within=60), 'here', f'start {n} of the server never answered')
        return read()

    def node_calls(self):
        log = Path(self.env['FAKE_LOG'])
        return [l[5:] for l in log.read_text().splitlines() if l.startswith('node ')] if log.exists() else []

    def test_on_only_and_off_write_the_mode_restart_and_say_what_is_next(self):
        self.node()
        self.sb('start')
        self.assertEqual(self.starts(1), ['off'])

        code, out = self.sb('keys', 'on')
        self.assertEqual((code, out), (0, ON))
        self.assertEqual((self.mode(), self.starts(2)), ('both', ['off', 'both']), 'written first, then one restart')
        self.assertEqual(json.loads((self.cfg / 'server.json').read_text())['bind'], 'all', 'nothing else in the file changes')
        self.assertEqual(self.node_calls(), [], 'the node reads the mode itself: it is not restarted')
        self.assertEqual(self.answer(), 'here')
        # again: said, and nothing is restarted
        code, out = self.sb('keys', 'on')
        self.assertEqual((code, out, len(self.starts())), (0, 'Keys are already on.\n', 2))

        # only: refused while something would be cut off, with what
        code, out = self.sb('keys', 'only')
        self.assertEqual(code, 1)
        self.assertEqual(out, 'Not yet: switching to keys only now would cut something off.\n'
                              '  Laptop is not paired yet.\n'
                              'Nothing was changed. To switch anyway: switchboard keys only --force\n')
        self.assertEqual((self.mode(), len(self.starts())), ('both', 2))
        # ... and allowed once the server says it is ready
        self.readiness(READY)
        code, out = self.sb('keys', 'only')
        self.assertEqual((code, out), (0, 'This home computer now uses keys only: its ports are closed to the network.\n'
                                          'To open the addresses again: switchboard keys on\n'))
        self.assertEqual((self.mode(), self.starts(3)), ('only', ['off', 'both', 'only']))

        code, out = self.sb('keys', 'off')
        self.assertEqual((code, out), (0, 'Keys are off. Computers reach this home computer by address, as before.\n'))
        self.assertEqual((self.mode(), self.starts(4)), ('off', ['off', 'both', 'only', 'off']))
        self.sb('stop')

    def test_only_can_be_forced_and_says_plainly_what_that_may_cost(self):
        self.node()
        (self.cfg / 'server.json').write_text('{"keys": "both"}\n')
        self.sb('start')
        self.assertEqual(self.answer(), 'here')
        code, out = self.sb('keys', 'only', '--force')
        self.assertEqual(code, 0, out)
        self.assertEqual(out, 'Warning: switching to keys only although something may be cut off:\n'
                              '  Laptop is not paired yet.\n'
                              'To go back: switchboard keys on\n'
                              'This home computer now uses keys only: its ports are closed to the network.\n'
                              'To open the addresses again: switchboard keys on\n')
        self.assertEqual((self.mode(), self.starts(2)), ('only', ['both', 'only']))
        self.sb('stop')

    def test_a_node_that_does_not_answer_after_the_restart_is_said(self):
        self.node()
        self.control.up = False
        self.sb('start')
        code, out = self.sb('keys', 'on', SWITCHBOARD_KEYS_WAIT='1')
        self.assertEqual(code, 1)
        self.assertEqual(out, 'Keys are on in server.json, but the node on this home computer did not answer yet.\n'
                              'Check it with: switchboard-node status\n')
        self.assertEqual(self.mode(), 'both')
        self.assertGreater(self.control.asked, 1, 'it was asked more than once before giving up')
        self.sb('stop')

    def test_keys_need_the_node_on_this_computer(self):
        for what in ('on', 'only'):
            code, out = self.sb('keys', what)
            self.assertEqual(code, 1)
            self.assertEqual(out, 'Keys need switchboard-node on this home computer, and it is not installed here.\n'
                                  'Link this computer first: switchboard link\n')
        self.assertEqual((self.mode(), self.starts()), (None, []))
        # off needs nothing, and a wrong word is the usage line
        code, out = self.sb('keys', 'off')
        self.assertEqual((code, out), (0, 'Keys are already off.\n'))
        code, out = self.sb('keys', 'sideways')
        self.assertEqual((code, out), (2, 'usage: switchboard keys on|only [--force]|off|status\n'))

    def test_status_says_the_mode_who_is_paired_and_what_stands_in_the_way(self):
        code, out = self.sb('keys')
        self.assertEqual((code, out), (0, 'Keys: off. Computers reach this home computer by address.\n'
                                          'To turn keys on: switchboard keys on\n'))
        (self.cfg / 'server.json').write_text('{"keys": "both"}\n')
        self.sb('start')
        self.assertEqual(self.answer(), 'here')
        code, out = self.sb('keys', 'status')
        self.assertEqual((code, out), (0, 'Keys: on. Computers reach this home computer with a key or by address.\n'
                                          '  Lab  this home computer\n'
                                          '  Laptop  not paired\n'
                                          'Not ready for keys only yet:\n'
                                          '  Laptop is not paired yet.\n'
                                          'To pair a computer: switchboard code\n'))
        self.readiness(READY)
        code, out = self.sb('keys', 'status')
        self.assertIn('  Laptop  paired, last connected with its key 2 minutes ago (directly)\n'
                      'Ready for keys only. To close the ports to the network: switchboard keys only\n', out)
        # the command's own status has a line for keys once they are on, and says why the downloads are not there
        code, out = self.sb('status')
        self.assertIn('  keys       on: computers connect with a key or by address\n', out)
        self.assertIn('  downloads  http://127.0.0.1:8131', out)
        (self.cfg / 'server.json').write_text('{"keys": "only"}\n')
        code, out = self.sb('status')
        self.assertIn('  keys       only: computers connect with a key, and the ports are closed to the network\n', out)
        self.assertIn('  downloads  not served (keys only)\n', out)
        (self.cfg / 'server.json').write_text('{}\n')
        code, out = self.sb('status')
        self.assertNotIn('keys', out)
        self.sb('stop')

    def test_pair_code_devices_and_forget_are_switchboard_nodes(self):
        for args in (('code',), ('devices',), ('pair', 'ABCD-EFGH'), ('forget', 'Laptop'), ('home', 'status')):
            code, out = self.sb(*args)
            self.assertEqual(code, 1)
            self.assertEqual(out, 'switchboard-node is not installed on this computer. On the home computer, run: '
                                  'switchboard link\n')
        self.node()
        code, out = self.sb('code')
        self.assertEqual((code, out), (0, 'Code: ABCD-EFGH\nLink: switchboard://pair/1?h=x\n'))
        code, out = self.sb('devices')
        self.assertEqual((code, out), (0, 'Laptop  computer  paired today\n'))
        code, out = self.sb('pair', 'switchboard://pair/1?h=x', '--name', 'My Laptop')
        self.assertEqual((code, out), (0, 'Paired with Home. This computer is laptop there.\n'))
        code, out = self.sb('forget', 'My Laptop')
        self.assertEqual(code, 0, out)
        # moving the home computer, a backup of it and bringing it back are switchboard-node's as well
        code, out = self.sb('home', 'move', 'Tower', '--retire')
        self.assertEqual(code, 0, out)
        self.assertEqual(self.node_calls(), ['code', 'devices', 'pair switchboard://pair/1?h=x --name My Laptop',
                                             'forget My Laptop', 'home move Tower --retire'])
        # what the node says when it refuses is what is said, with its exit code
        code, out = self.sb('pair', 'WRONG', FAKE_PAIR_FAILS='1')
        self.assertEqual((code, out), (1, 'switchboard-node: That code is not known.\n'))

    def test_the_usage_line_names_them(self):
        code, out = self.sb('nonsense')
        self.assertEqual(code, 2)
        self.assertEqual(out, 'usage: switchboard start|stop|restart|status|logs [lines]|setup|link|keys on|only|off|status|'
                              'code|pair <code-or-link>|devices|forget <name>|home status|move|backup|restore|'
                              'uninstall [--purge]\n')


class Downloads(scripts.Scripts):
    """The downloads are server/downloads.py now, and one started the old way is still this command's to stop."""

    def lay_out(self):
        (self.share / 'app' / 'server').mkdir(parents=True)
        (self.share / 'app' / 'server' / 'server.py').write_text("VERSION = 'here'\n" + scripts.GOOD_SERVER)
        (self.share / 'downloads').mkdir()
        (self.share / 'run').mkdir()

    def command_of(self, pid):
        return subprocess.run(['ps', '-p', str(pid), '-o', 'command='], capture_output=True, text=True).stdout.strip()

    def test_started_as_downloads_py_and_stopped(self):
        self.lay_out()
        code, out = self.run_script(CTL, 'start')
        self.assertEqual(code, 0, out)
        pidfile = self.share / 'run' / 'downloads.pid'
        pid = int(pidfile.read_text())
        want = f"{self.share}/app/server/downloads.py --port 8131 --directory {self.share}/downloads"
        deadline = time.time() + 5
        while want not in self.command_of(pid) and time.time() < deadline:
            time.sleep(0.1)
        self.assertIn(want, self.command_of(pid))
        # a second start leaves it be: the command line is known for this part's
        self.run_script(CTL, 'start')
        self.assertEqual(int(pidfile.read_text()), pid)
        self.run_script(CTL, 'stop')
        self.assertEqual(self.command_of(pid), '')
        self.assertFalse(pidfile.exists())

    def test_one_started_the_old_way_is_still_found_and_stopped(self):
        self.lay_out()
        old = subprocess.Popen([str(self.fake / 'python3'), '-m', 'http.server', '8131', '--directory', str(self.share / 'downloads')],
                               env=self.env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        self.addCleanup(old.wait)
        self.addCleanup(old.kill)
        pidfile = self.share / 'run' / 'downloads.pid'
        pidfile.write_text(f'{old.pid}\n')
        time.sleep(0.3)
        self.assertIn('http.server 8131', self.command_of(old.pid))
        code, out = self.run_script(CTL, 'stop')
        self.assertEqual(code, 0, out)
        self.assertIsNotNone(old.wait(timeout=10), 'the old downloads process was not stopped')
        self.assertFalse(pidfile.exists())

    def test_the_services_the_installer_writes_run_downloads_py(self):
        import plistlib
        for system in ('Darwin', 'Linux'):
            with self.subTest(system=system):
                self.shim('uname', scripts.UNAME % system)
                self.shim('launchctl', scripts.LOGGED)
                self.shim('systemctl', scripts.LOGGED)
                shutil.rmtree(self.tmp / 'kit-1.0.0', ignore_errors=True)
                code, out = self.run_script(self.kit('1.0.0'), '--yes', '--no-setup', '--plain', '--no-start', '--no-hub')
                self.assertEqual(code, 0, out)
                py, app, dl = str(self.fake / 'python3'), str(self.share / 'app'), str(self.share / 'downloads')
                if system == 'Darwin':
                    with open(self.home / 'Library/LaunchAgents/com.switchboard.downloads.plist', 'rb') as f:
                        args = plistlib.load(f)['ProgramArguments']
                    self.assertEqual(args, [py, f'{app}/server/downloads.py', '--port', '8131', '--directory', dl])
                else:
                    unit = (self.home / '.config/systemd/user/switchboard-downloads.service').read_text()
                    self.assertIn(f'\nExecStart="{py}" "{app}/server/downloads.py" "--port" "8131" "--directory" "{dl}"\n', unit)
                    self.assertNotIn('http.server', unit)


class Uninstall(scripts.Scripts):
    """`switchboard uninstall`: this computer's key and its list of paired devices stay unless --purge is given."""

    def lay_out(self):
        cfg = self.home / '.config' / 'switchboard'
        for d in (self.share / 'app' / 'server', self.bin, cfg):
            d.mkdir(parents=True, exist_ok=True)
        (cfg / 'device.key').write_text('k')
        (cfg / 'node.json').write_text('{}')
        (self.share / 'paired.json').write_text('{}')
        p = self.bin / 'switchboard-node'
        p.write_text(NODE)
        p.chmod(0o755)
        self.shim('systemctl', scripts.LOGGED)
        self.shim('launchctl', scripts.LOGGED)
        return cfg

    def calls(self):
        return [l for l in Path(self.env['FAKE_LOG']).read_text().splitlines() if l.startswith('node ')]

    def test_plain_keeps_the_key_and_the_paired_devices(self):
        cfg = self.lay_out()
        code, out = self.run_script(CTL, 'uninstall', '--yes')
        self.assertEqual(code, 0, out)
        self.assertEqual(self.calls(), ['node uninstall'])
        self.assertTrue((cfg / 'device.key').exists() and (self.share / 'paired.json').exists())
        self.assertNotIn('pair again', out)

    def test_purge_removes_them_and_says_what_that_means(self):
        cfg = self.lay_out()
        code, out = self.run_script(CTL, 'uninstall', '--purge', '--yes')
        self.assertEqual(code, 0, out)
        self.assertEqual(self.calls(), ['node uninstall --purge'])
        self.assertFalse(cfg.exists() or self.share.exists(), out)
        self.assertIn("This computer's key and its list of paired devices are removed: every device that was paired with "
                      'this computer will need to pair again.\n', out)
        # a computer that never had keys hears nothing about them
        self.lay_out()
        (cfg / 'device.key').unlink()
        (self.share / 'paired.json').unlink()
        code, out = self.run_script(CTL, 'uninstall', '--purge', '--yes')
        self.assertEqual(code, 0, out)
        self.assertNotIn('pair again', out)


class InstallWithKeys(scripts.Installs):
    """install-server.sh --keys: the home computer links itself, turns keys on and prints a code and a link."""

    def setUp(self):
        super().setUp()
        self.control = Control()
        self.addCleanup(self.control.close)
        self.fakehub = scripts.FakeHub()
        self.addCleanup(self.fakehub.close)
        self.env.update(SWITCHBOARD_PORT=str(self.port), SWITCHBOARD_NODE_CONTROL=self.control.url, SWITCHBOARD_KEYS_WAIT='3',
                        SWITCHBOARD_HUB_URL=f'http://{self.fakehub.addr}')

    def go(self, version, *args):
        code, out = self.install(version, '--hub-binary', self.hub('A'), *args)
        self.assertEqual(code, 0, out)
        self.assertEqual(self.answer(), version, 'the server runs whatever became of the keys')
        return out

    def node_calls(self):
        log = Path(self.env['FAKE_LOG'])
        return [l[5:] for l in log.read_text().splitlines() if l.startswith('node ')] if log.exists() else []

    def kit(self, version, server=None, node=True):
        shutil.rmtree(self.tmp / f'kit-{version}', ignore_errors=True)
        script = super().kit(version, SERVER)  # (the stand-in that writes server.json, whatever was asked for)
        root = script.parents[2]
        for f in (ROOT / 'server').glob('*.py'):
            if f.name != 'server.py':
                shutil.copy(f, root / 'server' / f.name)
        if node:
            (root / 'native' / 'dist').mkdir(parents=True)
            for n in ('switchboard-node-mac', 'switchboard-node-linux'):
                (root / 'native' / 'dist' / n).write_text(NODE)
        return script

    def mode(self):
        return json.loads((self.home / '.config/switchboard/server.json').read_text()).get('keys')

    def test_a_first_install_with_keys_ends_with_a_code_and_a_link_for_the_next_computer(self):
        out = self.go('1.0.0', '--keys')
        self.assertIn('== This computer\nThis computer is linked to its own server, so its chats show on the board.\n', out)
        self.assertIn('\n== Keys\n'
                      'Keys are on: other computers can join this home computer with a code or a link.\n'
                      "For the next computer (this works once, for a few minutes; 'switchboard code' gives another):\n"
                      '  Code: ABCD-EFGH\n'
                      '  Link: switchboard://pair/1?h=x\n'
                      'On that computer, run install-agent.sh (or install-agent.ps1 on Windows) with: '
                      '--pair <the code or the link>\n', out)
        self.assertEqual(self.mode(), 'both')
        self.assertEqual(self.started(self.node_calls()), [f'pair {self.fakehub.addr} CODE1', 'install', 'code'])
        self.assertEqual((self.home / 'starts.log').read_text().split(), ['off', 'both'], 'one restart, after the mode')

    def test_without_the_option_nothing_about_keys_changes(self):
        out = self.go('1.0.0')
        self.assertNotIn('Keys', out)
        self.assertIsNone(self.mode())
        self.assertNotIn('code', self.node_calls())

    def test_keys_that_cannot_be_turned_on_leave_a_working_server_and_the_line_to_run(self):
        # no switchboard-node for this kind of computer in the kit: nothing to hold this computer's key
        script = self.kit('1.0.0', node=False)
        code, out = self.run_script(script, '--yes', '--no-setup', '--plain', '--no-service', '--hub-binary', self.hub('A'), '--keys')
        self.assertEqual(code, 0, out)
        self.assertIn('== Keys\nKeys could not be turned on:\n'
                      '  Keys need switchboard-node on this home computer, and it is not installed here.\n'
                      '  Link this computer first: switchboard link\n'
                      'The server is installed and running. To turn keys on, run: switchboard keys on\n', out)
        self.assertIsNone(self.mode())
        self.assertEqual(self.answer(), '1.0.0')


SB = 'import sys\nprint("agent set up")\n'


class AgentPair(scripts.Scripts):
    """install-agent.sh --pair: a second computer joins with a code or a link, and no address is typed."""

    def setUp(self):
        super().setUp()
        self.sb = self.tmp / 'sb.py'
        self.sb.write_text(SB)
        self.node = self.tmp / 'switchboard-node-x'
        self.node.write_text(NODE)
        self.node.chmod(0o755)
        self.shim('uname', scripts.UNAME % 'Darwin')  # (where the node sets itself up as a service)
        self.shim('xattr', '#!/bin/sh\nexit 0\n')
        self.shim('sysctl', '#!/bin/sh\nexit 1\n')

    def agent(self, *args, **env):
        return self.run_script(INSTALL / 'install-agent.sh', '--sb', str(self.sb), *args, **env)

    def calls(self):
        log = Path(self.env['FAKE_LOG'])
        return [l[5:] for l in log.read_text().splitlines() if l.startswith('node ')] if log.exists() else []

    def test_a_code_or_a_link_pairs_and_sets_the_node_to_run(self):
        link = 'switchboard://pair/1?h=abc&s=def'
        for typed in ('ABCD-EFGH', link):
            with self.subTest(typed=typed):
                Path(self.env['FAKE_LOG']).unlink(missing_ok=True)
                code, out = self.agent('--pair', typed, '--node', str(self.node), '--label', 'My Laptop', '--update')
                self.assertEqual(code, 0, out)
                self.assertEqual(self.calls(), [f'pair {typed} --name My Laptop', 'install'])
                self.assertIn('Paired with Home. This computer is laptop there.\n', out)
                self.assertTrue((self.bin / 'switchboard-node').is_file())
                self.assertTrue(out.endswith('\nDone. This machine shows up in Switchboard within a minute.\n'), out)

    def test_a_relay_is_set_before_pairing(self):
        code, out = self.agent('--pair', 'ABCD-EFGH', '--relay', 'https://relay.example.com', '--node', str(self.node))
        self.assertEqual(code, 0, out)
        self.assertEqual(self.calls(), ['relay https://relay.example.com', 'pair ABCD-EFGH', 'install'])

    def test_a_pairing_that_fails_says_how_to_get_another_code(self):
        code, out = self.agent('--pair', 'ABCD-EFGH', '--node', str(self.node), FAKE_PAIR_FAILS='1')
        self.assertEqual(code, 1)
        self.assertIn('switchboard-node: That code is not known.\n', out)
        self.assertIn("install-agent.sh: pairing failed (see above). A code or a link works once, for a few minutes: get a "
                      "new one with 'switchboard code' on the home computer.\n", out)
        self.assertNotIn('install', self.calls())

    def test_what_cannot_work_is_refused_before_anything_is_changed(self):
        for args, said in (
                (('--pair', 'ABCD-EFGH'), 'install-agent.sh: --pair needs switchboard-node: pass --node PATH, or --server ADDR to '
                                          'download it from a home computer that still answers by address.\n'),
                (('--pair', 'ABCD-EFGH', '--code', 'WXYZ', '--node', str(self.node)),
                 'install-agent.sh: --pair and --code are two ways to join: use one.\n'),
                (('--pair', 'has spaces', '--node', str(self.node)),
                 'install-agent.sh: --pair takes the code or the link that "switchboard code" printed on the home computer\n'),
                (('--relay', 'https://relay.example.com'), 'install-agent.sh: --relay goes with --pair\n'),
                (('--pair', 'ABCD-EFGH', '--relay', 'ftp://x', '--node', str(self.node)),
                 'install-agent.sh: --relay takes a relay\'s address, like https://relay.example.com\n')):
            with self.subTest(args=args):
                code, out = self.agent(*args)
                self.assertEqual((code, out), (1, said))
                self.assertFalse((self.home / '.claude' / 'switchboard' / 'sb.py').exists())
        self.assertEqual(self.calls(), [])


class WindowsScripts(unittest.TestCase):
    """The PowerShell scripts are read, not run (no PowerShell where these tests run): what they start and say."""

    def text(self, name):
        return (INSTALL / name).read_text(encoding='utf-8')

    def test_the_downloads_task_runs_downloads_py_and_an_older_one_is_still_found(self):
        task = [l for l in self.text('install-server.ps1').splitlines() if l.startswith("Task 'SwitchboardDownloads'")]
        self.assertEqual(task, ['Task \'SwitchboardDownloads\' """$pyw"" ""$App\\server\\downloads.py"" --port $DlPort '
                                '--directory ""$Dl""" (Join-Path $Logs \'downloads.log\')'])
        self.assertIn("$DlPort = if ($env:SWITCHBOARD_DOWNLOADS_PORT) { $env:SWITCHBOARD_DOWNLOADS_PORT } else { '8131' }",
                      self.text('install-server.ps1'))
        ctl = self.text('switchboard.ps1')
        self.assertIn('-like "*$App\\server\\downloads.py*"', ctl)
        self.assertIn("-like '*http.server 8131*'", ctl, 'an update still stops the downloads the old install started')
        self.assertNotIn('-m http.server', self.text('install-server.ps1'))

    def test_the_command_has_the_same_words_as_on_a_mac_or_linux(self):
        ctl = self.text('switchboard.ps1')
        sh = (INSTALL / 'switchboard').read_text()
        for said in ('  downloads  not served (keys only)',
                     '  keys       on: computers connect with a key or by address',
                     '  keys       only: computers connect with a key, and the ports are closed to the network',
                     "This computer's key and its list of paired devices are removed: every device that was paired with "
                     'this computer will need to pair again.'):
            self.assertIn(said, ctl)
            self.assertIn(said, sh)
        self.assertIn("'keys' { Keys }", ctl)
        self.assertIn("{ $_ -in 'code', 'pair', 'devices', 'forget', 'home' } { Node-Cmd }", ctl)
        self.assertIn("if ($purge) { $nodeArgs += '--purge' }", ctl)
        self.assertIn("'link' { Link }", ctl)

    def test_keys_are_refused_only_while_the_hub_or_the_node_is_missing(self):
        ctl = self.text('switchboard.ps1')
        keys = ctl[ctl.index('function Keys {'):ctl.index('# ---------------------------------------------------------------- link')]
        self.assertNotIn('not installed on Windows yet', ctl)
        for said in ("Write-Host 'Keys need the hub on this home computer, and it is not installed here.'",
                     "Write-Host 'Keys need switchboard-node on this home computer, and it is not installed here.'",
                     "Write-Host 'Link this computer first: switchboard.ps1 link'"):
            self.assertIn(said, keys)
        # status and off never need either; on and only pass keys.py's answer on (3: nothing changed) and restart
        self.assertLess(keys.index("if ($what -eq 'status')"), keys.index('if (-not (Test-Path $HubExe))'))
        self.assertIn("if ($what -ne 'off') {", keys)
        tail = keys[keys.index('& $Py $KeysPy $what @more'):]
        self.assertEqual([l.strip() for l in tail.splitlines()[:6]],
                         ['& $Py $KeysPy $what @more', 'if ($LASTEXITCODE -eq 3) { exit 0 }',
                          'if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }', 'Stop-Parts; Start-Sleep 1; Start-Parts',
                          '& $Py $KeysPy wait', 'exit $LASTEXITCODE'])

    def test_the_hub_is_installed_started_stopped_and_reported(self):
        inst, ctl = self.text('install-server.ps1'), self.text('switchboard.ps1')
        for text in (inst, ctl):
            self.assertIn("$Tasks = @('SwitchboardServer', 'SwitchboardHub', 'SwitchboardDownloads')", text)
            self.assertIn("$HubExe = Join-Path $Base 'switchboard-hub.exe'", text)
        # where it comes from: -HubBinary, or the ready-made one in the kit; -NoHub leaves it out
        self.assertIn("} elseif (Test-Path (Join-Path $Repo 'native\\dist\\switchboard-hub.exe')) { $HubSrc = ", inst)
        self.assertIn('if ($HubBinary) {', inst)
        self.assertIn('if (-not $NoHub) {', inst)
        # it listens for the computers linked to it, as on a Mac or Linux
        self.assertIn('$listen = if ($env:SWITCHBOARD_HUB_LISTEN) { $env:SWITCHBOARD_HUB_LISTEN } else { "0.0.0.0:$HubPort" }', inst)
        self.assertIn('Task \'SwitchboardHub\' """$HubExe""" (Join-Path $Logs \'hub.log\') "set ""SWITCHBOARD_HUB_LISTEN=$listen"" && "', inst)
        self.assertIn('Environment=SWITCHBOARD_HUB_LISTEN=0.0.0.0:8132', (INSTALL / 'install-server.sh').read_text())
        # stopped by where its program is, so another hub on this PC is left alone; a node it did not start too
        self.assertIn("Where-Object { $_.ExecutablePath -and ($exes -contains $_.ExecutablePath) }", ctl)
        self.assertIn('$exes = @($HubExe); if ($ByHand) { $exes += $NodeExe }', ctl)
        self.assertIn('if (Test-Path $HubExe) { Write-Host "  hub        http://127.0.0.1:$HubPort  $(Up "http://127.0.0.1:$HubPort/healthz")" }', ctl)
        self.assertIn("else { Write-Host '  hub        not installed (terminals and linked machines are off)' }", ctl)
        self.assertIn('echo "  hub        not installed (terminals and linked machines are off)"', (INSTALL / 'switchboard').read_text())
        # the firewall line names the hub's port when there is a hub
        self.assertIn('$fw = if ($HasHub) { "$Port,$DlPort,$HubPort" } else { "$Port,$DlPort" }', inst)

    def test_the_node_builds_go_in_the_downloads_and_this_pc_links_itself(self):
        inst, ctl = self.text('install-server.ps1'), self.text('switchboard.ps1')
        self.assertIn("$NodeNames = @('switchboard-node-mac', 'switchboard-node-linux', 'switchboard-node.exe')", inst)
        self.assertIn("foreach ($d in @($NodeDir, (Join-Path $Repo 'native\\dist'))) {", inst)
        self.assertIn("$said = @(Ctl-Out @('link', '--first'))", inst)
        self.assertIn('if ($HasHub -and -not $NoLink) {', inst)
        sh, unix_ctl = (INSTALL / 'install-server.sh').read_text(), (INSTALL / 'switchboard').read_text()
        for said in ('This computer is linked to its own server, so its chats show on the board.',
                     'This server has other computers already, so this one was not linked. To link it too, run: ',
                     'This computer could not be linked to its own server:',
                     'The server is installed and running. To link this computer, run: ',
                     'Keys could not be turned on:', 'The server is installed and running. To turn keys on, run: ',
                     'Keys are on: other computers can join this home computer with a code or a link.',
                     'A code for the next computer could not be made:',
                     'On that computer, run install-agent.sh (or install-agent.ps1 on Windows) with: --pair <the code or the link>'):
            self.assertIn(said, inst)
            self.assertIn(said, sh)
        for said in ('The hub on this computer did not answer.', 'This server has no switchboard-node for this kind of computer.',
                     'The hub on this computer gave no code.', 'This computer is linked to its own server.',
                     'This server has no hub, so computers are added over ssh. Run: ',
                     'This computer is linked already. To link it again, first run: '):
            self.assertIn(said, ctl)
            self.assertIn(said, unix_ctl)
        link = ctl[ctl.index('function Link {'):ctl.index('# code, pair, devices and forget')]
        # a code only from this PC's own hub, the node from this server's own downloads, and never a second link
        self.assertIn("$node = Join-Path $Dl 'node\\switchboard-node.exe'", link)
        self.assertIn("$out = @(Run-Out $new @('pair', ($HubUrl -replace '^[a-zA-Z]+://', ''), $code))", link)
        self.assertIn('if ((Test-Path $NodeCfg) -and (Test-Path $NodeExe)) {', link)
        self.assertIn("if ($first -and -not ($n -eq '0' -and (Ask 'fleet') -eq '0')) { exit 3 }", link)
        self.assertIn("$out = @(Run-Out $new @('install'))", link)

    def test_without_tasks_nothing_is_registered_and_the_command_runs_the_parts_itself(self):
        inst, ctl = self.text('install-server.ps1'), self.text('switchboard.ps1')
        task = inst[inst.index('function Task('):inst.index("Task 'SwitchboardServer'")]
        self.assertLess(task.index('if ($NoTasks) { return }'), task.index('schtasks /Create'))
        self.assertEqual(inst.count('schtasks /Create'), 1)
        self.assertIn("$Base = if ($env:SWITCHBOARD_BASE) { $env:SWITCHBOARD_BASE } else { Join-Path $env:LOCALAPPDATA 'Switchboard' }", inst)
        self.assertIn("$Base = if ($env:SWITCHBOARD_BASE) { $env:SWITCHBOARD_BASE } else { Join-Path $env:LOCALAPPDATA 'Switchboard' }", ctl)
        self.assertIn("$ByHand = Test-Path (Join-Path $Base 'by-hand.txt')", ctl)
        start = ctl[ctl.index('function Start-Parts {'):ctl.index('function Stop-Parts')]
        self.assertLess(start.index('if (-not $ByHand) {'), start.index('schtasks /Run'))
        self.assertIn("foreach ($t in ($Tasks + 'SwitchboardNode')) {", start)
        # by hand, the node is a background process too: `switchboard-node install` would register a logon task
        link = ctl[ctl.index('function Link {'):ctl.index('# code, pair, devices and forget')]
        self.assertLess(link.index('if ($ByHand) {'), link.index("@('install')"))
        self.assertIn('Runner \'SwitchboardNode\' """$NodeExe"" run" (Join-Path $Logs \'node.log\')', link)
        # and a link some other switchboard-node on this PC keeps is never replaced: it is looked for where the
        # node keeps it (USERPROFILE: a HOME that an ssh session or a test sets moves nothing), and found there it
        # stops the link
        self.assertIn("$Me = if ($env:USERPROFILE) { $env:USERPROFILE } else { $HOME }\n$NodeHome = $Me\n", ctl)
        self.assertNotIn('$env:HOME', ctl)
        self.assertIn("else { Join-Path $NodeHome '.config\\switchboard\\node.json' }", ctl)
        self.assertLess(link.index('if ($ByHand -and (Test-Path $NodeCfg)) {'), link.index("@('pair'"))
        self.assertIn("by a switchboard-node that is not this server's. It is left as it is.", link)
        # the settings file is read only for such a server
        self.assertIn('if ($ByHand -and (Test-Path $EnvFile)) {', ctl)

    def test_the_windows_build_makes_the_hub_and_the_kit_carries_it(self):
        native = INSTALL.parent.parent / 'native' / 'scripts'
        build = (native / 'remote' / 'windows-build.ps1').read_text(encoding='utf-8')
        self.assertIn('cargo build --release --locked -q -p sb-node -p sb-hub', build)
        self.assertIn("foreach ($b in 'switchboard-node', 'switchboard-hub') {", build)
        self.assertIn("foreach ($b in 'Switchboard', 'switchboard-node', 'switchboard-hub', 'switchboard-install') {", build)
        self.assertIn('scp -q "$H:$D/dist/switchboard-hub.exe" dist/switchboard-hub.exe', (native / 'build-windows.sh').read_text())
        self.assertIn("'switchboard-hub.exe'", (INSTALL / 'pack-server-kit.sh').read_text())
        for text in (self.text('install-server.ps1'), self.text('switchboard.ps1'), build):
            for bad in (chr(0x2014), '!\n'):
                self.assertNotIn(bad, text)

    def test_the_agent_installer_pairs_with_a_code_or_a_link(self):
        ps, sh = self.text('install-agent.ps1'), (INSTALL / 'install-agent.sh').read_text()
        self.assertIn("$pairArgs = @('pair', $Pair)", ps)
        self.assertIn('& $exe relay $Relay', ps)
        for flag, said in (('-Pair', "takes the code or the link that \"switchboard code\" printed on the home computer"),
                           ('-Relay', "takes a relay's address, like https://relay.example.com"),
                           ('-Pair', 'needs switchboard-node: pass -Node PATH, or -Server ADDR to download it from a home '
                                     'computer that still answers by address.')):
            self.assertIn(f'{flag} {said}', ps)
            self.assertIn(('-' + flag.lower() + ' ' + said).replace('-Node', '--node').replace('-Server', '--server'), sh.replace('\\"', '"'))
        for text in (ps, sh):
            self.assertIn("pairing failed (see above). A code or a link works once, for a few minutes: get a new one with "
                          "'switchboard code' on the home computer.", text)
            for bad in (chr(0x2014), '!\n'):
                self.assertNotIn(bad, text)


class RelayAtBuildTime(unittest.TestCase):
    """SB_RELAY_DEFAULT: the relay built into switchboard-node. Empty unless whoever builds names one."""

    SCRIPTS = INSTALL.parent.parent / 'native' / 'scripts'

    def env(self, relay):
        with tempfile.TemporaryDirectory() as d:
            r = subprocess.run(['bash', '-c', '. "$1"; sb_update_env; printf "[%s]" "$SB_RELAY_DEFAULT"', '-',
                                str(self.SCRIPTS / 'hosts.sh')], capture_output=True, text=True,
                               env={'PATH': os.environ['PATH'], 'HOME': d, 'SB_UPDATE_PUBKEY': 'x',
                                    **({} if relay is None else {'SB_RELAY_DEFAULT': relay})})
        return r.returncode, r.stdout, r.stderr

    def test_it_is_empty_by_default_and_a_url_when_set(self):
        self.assertEqual(self.env(None), (0, '[]', ''))
        for url in ('https://relay.example.com', 'http://[fd00::1]:3340', 'https://relay.example.com/path'):
            self.assertEqual(self.env(url), (0, f'[{url}]', ''))

    def test_what_is_not_a_url_stops_the_build(self):
        for bad, said in (('relay.example.com', "SB_RELAY_DEFAULT must be a relay's http:// or https:// address, or empty\n"),
                          ("https://x';echo", 'SB_RELAY_DEFAULT has characters no URL has\n'),
                          ('https://a b', 'SB_RELAY_DEFAULT has characters no URL has\n')):
            self.assertEqual(self.env(bad), (1, '', said))

    def test_every_build_passes_it_on_and_the_example_leaves_it_empty(self):
        for name in ('build-linux.sh', 'build-mac.sh', 'build-windows.sh', 'build-dmg.sh'):
            text = (self.SCRIPTS / name).read_text()
            self.assertIn('sb_update_env', text, name)
            self.assertIn('SB_RELAY_DEFAULT', text, name)
        self.assertIn('\nSB_RELAY_DEFAULT=\n', (self.SCRIPTS / 'build-hosts.example.env').read_text())
        keys = (self.SCRIPTS.parent / 'crates' / 'sb-node' / 'src' / 'keys.rs').read_text()
        self.assertIn('option_env!("SB_RELAY_DEFAULT")', keys)


if __name__ == '__main__':
    unittest.main()
