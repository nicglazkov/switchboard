"""scripts/install, run for real in a throwaway home folder: install-server.sh over a server that is already there
(an update that does not start puts the old one back), the `switchboard` command's pid files and LaunchAgent
loading, the service files it writes for odd folder names, and the checksum checks of install-agent.sh and
get-server.sh. Nothing outside the temporary folder is touched: launchctl, systemctl, uname and curl are stand-ins
first on PATH, and the stand-in server listens on a free port of its own."""
import hashlib, os, plistlib, shutil, signal, socket, subprocess, sys, tempfile, time, unittest, urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
INSTALL = ROOT / 'scripts' / 'install'

GOOD_SERVER = '''import http.server, os
class H(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        body = VERSION.encode()
        self.send_response(200)
        self.send_header('Content-Length', str(len(body)))
        self.end_headers()
        self.wfile.write(body)
    def log_message(self, *a):
        pass
http.server.HTTPServer(('127.0.0.1', int(os.environ['SB_TEST_PORT'])), H).serve_forever()
'''
BAD_SERVER = 'import sys\nsys.exit(3)\n'

# python3 as the scripts find it. The downloads part (python3 -m http.server 8131) only waits here, so no test opens
# that port.
PYTHON = '''#!/bin/bash
case " $* " in *" -m http.server "*) while :; do sleep 1; done ;; esac
exec "%s" "$@"
'''
HUB = '''#!/bin/sh
[ "$1" = --version ] && { echo "switchboard-hub %s"; exit 0; }
while :; do sleep 1; done
'''
UNAME = '''#!/bin/sh
case "$1" in -s|"") echo %s ;; *) exec /usr/bin/uname "$@" ;; esac
'''
LAUNCHCTL = '''#!/bin/sh
echo "$*" >> "$FAKE_LOG"
case "$1" in
  bootstrap)
    n=$(cat "$FAKE_LOG.n" 2>/dev/null || echo 0); n=$((n + 1)); echo "$n" > "$FAKE_LOG.n"
    [ "$n" -gt "${FAKE_REFUSE:-0}" ] && exit 0
    echo "Bootstrap failed: 5: Input/output error" >&2; exit 5 ;;
  kickstart) exit "${FAKE_KICK:-0}" ;;
esac
exit 0
'''
LOGGED = '''#!/bin/sh
echo "$*" >> "$FAKE_LOG"
exit 0
'''
# curl as install-agent.sh calls it (curl -fsSL URL -o FILE): the "server" is a folder.
CURL = '''#!/bin/sh
cp "$FAKE_DL/$(basename "$2")" "$4" 2>/dev/null
'''


def free_port():
    with socket.socket() as s:
        s.bind(('127.0.0.1', 0))
        return s.getsockname()[1]


@unittest.skipIf(os.name == 'nt' or not shutil.which('bash'), 'needs bash')
class Scripts(unittest.TestCase):
    """A home folder, a PATH of stand-ins, and the way to run a script in them."""
    FOLDER = 'box'

    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp(prefix='sb-install-')).resolve() / self.FOLDER
        self.home = self.tmp / 'home'
        self.fake = self.tmp / 'bin'
        self.home.mkdir(parents=True)
        self.fake.mkdir()
        self.shim('python3', PYTHON % sys.executable)
        self.port = free_port()
        self.url = f'http://127.0.0.1:{self.port}/'
        self.share = self.home / '.local' / 'share' / 'switchboard'
        self.bin = self.home / '.local' / 'bin'
        self.env = dict(HOME=str(self.home), PATH=f'{self.fake}:/usr/bin:/bin:/usr/sbin:/sbin', TZ='UTC', NO_COLOR='1',
                        LANG='C', SB_TEST_PORT=str(self.port), SWITCHBOARD_INSTALL_CHECK_URL=self.url,
                        SWITCHBOARD_RETRY_SLEEP='0.01', FAKE_LOG=str(self.tmp / 'calls.log'))
        self.addCleanup(self.cleanup)

    def cleanup(self):
        for f in (self.share / 'run').glob('*.pid') if (self.share / 'run').is_dir() else []:
            try:
                os.kill(int(f.read_text().strip()), signal.SIGKILL)
            except (OSError, ValueError):
                pass
        shutil.rmtree(self.tmp.parent, ignore_errors=True)

    def shim(self, name, text):
        p = self.fake / name
        p.write_text(text)
        p.chmod(0o755)
        return p

    def run_script(self, script, *args, **env):
        """(exit code, what it printed). Output goes through a file: a part left running must not hold a pipe open."""
        out = self.tmp / 'out.txt'
        with open(out, 'w') as f:
            p = subprocess.run(['bash', str(script), *args], stdin=subprocess.DEVNULL, stdout=f,
                               stderr=subprocess.STDOUT, env=dict(self.env, **env), cwd=str(self.tmp), timeout=180)
        return p.returncode, out.read_text(errors='replace')

    def answer(self, within=10.0):
        """What the stand-in server says (its version), or None when nothing answers in time."""
        end = time.time() + within
        while True:
            try:
                with urllib.request.urlopen(self.url, timeout=2) as r:
                    return r.read().decode()
            except OSError:
                if time.time() > end:
                    return None
                time.sleep(0.2)

    def kit(self, version, server=GOOD_SERVER):
        d = self.tmp / f'kit-{version}'
        for sub in ('server', 'agent', 'scripts/install', 'native/crates/sb-core/src/term'):
            (d / sub).mkdir(parents=True)
        (d / 'server' / 'server.py').write_text(f'VERSION = {version!r}\n' + server)
        (d / 'agent' / 'sb.py').write_text('# the agent\n')
        (d / 'native/crates/sb-core/src/term/schemes.json').write_text('{}')
        (d / 'VERSION').write_text(f'{version} (abc1234)\n')
        for f in ('install-server.sh', 'switchboard', 'setup.py', 'install-agent.sh', 'install-agent.ps1'):
            shutil.copy(INSTALL / f, d / 'scripts' / 'install' / f)
        return d / 'scripts' / 'install' / 'install-server.sh'

    def installed_version(self):
        return (self.share / 'app' / 'VERSION').read_text().split()[0]


class ServerUpdate(Scripts):
    def install(self, version, *args, server=GOOD_SERVER, wait=20):
        return self.run_script(self.kit(version, server), '--yes', '--no-setup', '--plain', '--no-service', *args,
                               SWITCHBOARD_INSTALL_WAIT=str(wait))

    def hub(self, name):
        return str(self.shim(f'hub-{name}', HUB % name))

    def hub_here(self):
        return subprocess.run([str(self.bin / 'switchboard-hub'), '--version'], capture_output=True, text=True).stdout.strip()

    def leftovers(self):
        return sorted(p.name for p in list(self.share.iterdir()) + list(self.bin.iterdir())
                      if p.name.endswith(('.old', '.new', '.bad')))

    def test_an_update_that_does_not_start_puts_the_old_server_back(self):
        code, out = self.install('1.0.0', '--hub-binary', self.hub('A'))
        self.assertEqual(code, 0, out)
        self.assertEqual(self.answer(), '1.0.0')
        self.assertEqual(self.leftovers(), [])

        # the new server exits as soon as it starts
        code, out = self.install('2.0.0', '--hub-binary', self.hub('B'), server=BAD_SERVER, wait=3)
        self.assertEqual(code, 1, out)
        last = [l for l in out.splitlines() if l.strip()][-1]
        self.assertEqual(last, 'install-server.sh: the new server did not start, so the one that was here before '
                               'was put back and started again. See: switchboard logs')
        self.assertEqual(self.installed_version(), '1.0.0')
        self.assertEqual(self.hub_here(), 'switchboard-hub A')
        self.assertEqual(self.answer(), '1.0.0', 'the old server is running again')
        self.assertEqual(self.leftovers(), [])

        # a hub that cannot be used stops the update after the server's files were swapped: they go back too, and
        # the server that was running is never stopped
        pid = (self.share / 'run' / 'server.pid').read_text()
        code, out = self.install('3.0.0', '--hub-binary', str(self.tmp / 'no-such-hub'))
        self.assertEqual(code, 1, out)
        self.assertIn('there is no hub at', out)
        self.assertIn('The server that was here before was put back.', out)
        self.assertEqual(self.installed_version(), '1.0.0')
        self.assertEqual((self.share / 'run' / 'server.pid').read_text(), pid)
        self.assertEqual(self.answer(), '1.0.0')
        self.assertEqual(self.leftovers(), [])

        # and an update that starts replaces both, keeping nothing of the old one
        code, out = self.install('4.0.0', '--hub-binary', self.hub('B'))
        self.assertEqual(code, 0, out)
        self.assertEqual(self.answer(), '4.0.0')
        self.assertEqual(self.installed_version(), '4.0.0')
        self.assertEqual(self.hub_here(), 'switchboard-hub B')
        self.assertEqual(self.leftovers(), [])

        code, out = self.run_script(self.bin / 'switchboard', 'stop')
        self.assertEqual(code, 0, out)
        self.assertIsNone(self.answer(within=0.5))

    def test_a_first_install_that_does_not_start_says_so_and_has_nothing_to_put_back(self):
        code, out = self.install('1.0.0', '--no-hub', server=BAD_SERVER, wait=2)
        self.assertEqual(code, 1, out)
        self.assertEqual([l for l in out.splitlines() if l.strip()][-1],
                         'install-server.sh: the server did not start. See: switchboard logs')
        self.assertEqual(self.installed_version(), '1.0.0')

    def test_a_server_left_moved_aside_by_a_killed_run_is_put_back_first(self):
        old = self.share / 'app.old'
        (old / 'server').mkdir(parents=True)
        (old / 'VERSION').write_text('0.9.0\n')
        # this update's hub cannot be used, so it stops after the swap: what goes back is the server found aside
        code, out = self.install('1.0.0', '--no-start', '--hub-binary', str(self.tmp / 'no-such-hub'))
        self.assertEqual(code, 1, out)
        self.assertEqual(self.installed_version(), '0.9.0')
        self.assertFalse(old.exists())


NODE = '''#!/bin/sh
echo "$*" >> "$FAKE_LOG"
[ "$1" = uninstall ] || exit 2
rm -f "$HOME/.config/switchboard/node.json" "$0"
echo "Removed the program: $0"
'''


class Uninstall(Scripts):
    """--uninstall and a switchboard-node on the same machine: the installer never put it there."""

    def node(self, hub):
        cfg = self.home / '.config' / 'switchboard'
        cfg.mkdir(parents=True, exist_ok=True)
        (cfg / 'node.json').write_text('{\n  "hub": "%s",\n  "machine": "lab",\n  "token": "t"\n}\n' % hub)
        self.bin.mkdir(parents=True, exist_ok=True)
        p = self.bin / 'switchboard-node'
        p.write_text(NODE)
        p.chmod(0o755)
        (self.bin / 'switchboard-hub').write_text('x')
        self.shim('launchctl', LOGGED)
        self.shim('systemctl', LOGGED)
        return p, cfg / 'node.json'

    def uninstall(self):
        code, out = self.run_script(INSTALL / 'install-server.sh', '--uninstall', '--plain')
        self.assertEqual(code, 0, out)
        self.assertFalse((self.bin / 'switchboard-hub').exists(), 'the server itself is removed either way')
        return out

    def test_a_node_linked_to_another_server_is_left_and_said(self):
        for hub in ('192.0.2.10:8132', 'tower.example.com:8132', '[2001:db8::1]:8132', 'http://192.0.2.10:8151/', ''):
            with self.subTest(hub=hub):
                node, cfg = self.node(hub)
                out = self.uninstall()
                self.assertTrue(node.exists() and cfg.exists(), out)
                self.assertIn('still linked to a Switchboard server', out)
                self.assertIn('switchboard-node uninstall', out)
                calls = (self.tmp / 'calls.log').read_text() if (self.tmp / 'calls.log').exists() else ''
                self.assertNotIn('uninstall', calls, 'the node was not even asked')

    def test_a_node_linked_to_this_machines_own_hub_goes_with_it_and_says_what_it_removed(self):
        import socket as s
        for hub in ('127.0.0.1:8132', 'localhost:8132', 'http://LOCALHOST:8151', '[::1]:8132', s.gethostname() + ':8132'):
            with self.subTest(hub=hub):
                node, cfg = self.node(hub)
                out = self.uninstall()
                self.assertFalse(node.exists() or cfg.exists(), out)
                self.assertIn('which was removed too', out)
                self.assertIn(f'Removed the program: {node}', out, "the node's own words are shown")
                self.assertNotIn('still linked', out)

    def test_no_node_no_word_about_one(self):
        self.bin.mkdir(parents=True, exist_ok=True)
        (self.bin / 'switchboard-hub').write_text('x')
        out = self.uninstall()
        self.assertNotIn('switchboard-node', out)
        self.assertIn('Switchboard is removed from this machine', out)

    def test_a_node_too_old_to_remove_itself_is_said_to_be_still_there(self):
        node, cfg = self.node('127.0.0.1:8132')
        node.write_text('#!/bin/sh\necho "usage: switchboard-node pair <hub> <code>" >&2\nexit 2\n')
        out = self.uninstall()
        self.assertTrue(node.exists() and cfg.exists())
        self.assertIn('could not remove itself', out)
        self.assertIn('usage: switchboard-node pair', out)
        self.assertIn('It is still installed', out)


class Command(Scripts):
    """The `switchboard` command with the server laid out by hand."""

    def lay_out(self):
        (self.share / 'app' / 'server').mkdir(parents=True)
        (self.share / 'app' / 'server' / 'server.py').write_text("VERSION = 'here'\n" + GOOD_SERVER)
        (self.share / 'downloads').mkdir()
        (self.share / 'run').mkdir()

    def test_a_pid_file_that_now_names_another_program_is_not_believed(self):
        self.lay_out()
        other = subprocess.Popen(['sleep', '300'])
        self.addCleanup(other.wait)
        self.addCleanup(other.kill)
        pidfile = self.share / 'run' / 'server.pid'
        pidfile.write_text(f'{other.pid}\n')
        # stop: the other program is not the server, and is left alone
        code, out = self.run_script(INSTALL / 'switchboard', 'stop')
        self.assertEqual(code, 0, out)
        time.sleep(0.3)
        self.assertIsNone(other.poll(), 'stop killed a program that only had the server\'s old pid')
        self.assertFalse(pidfile.exists())
        # start: the server is not running, whatever the file says
        pidfile.write_text(f'{other.pid}\n')
        code, out = self.run_script(INSTALL / 'switchboard', 'start')
        self.assertEqual(code, 0, out)
        self.assertEqual(self.answer(), 'here')
        self.assertNotEqual(pidfile.read_text().strip(), str(other.pid))
        # and its own pid is believed: a second start leaves it be, stop ends it
        pid = pidfile.read_text()
        self.run_script(INSTALL / 'switchboard', 'start')
        self.assertEqual(pidfile.read_text(), pid)
        self.run_script(INSTALL / 'switchboard', 'stop')
        self.assertIsNone(self.answer(within=0.5))
        self.assertIsNone(other.poll())

    def launchd(self):
        self.shim('uname', UNAME % 'Darwin')
        self.shim('launchctl', LAUNCHCTL)
        agents = self.home / 'Library' / 'LaunchAgents'
        agents.mkdir(parents=True)
        (agents / 'com.switchboard.server.plist').write_text('<plist/>')
        return str(agents / 'com.switchboard.server.plist')

    def calls(self):
        return [l.split() for l in Path(self.env['FAKE_LOG']).read_text().splitlines()]

    def test_a_launch_agent_refused_at_first_is_asked_again(self):
        plist = self.launchd()
        code, out = self.run_script(INSTALL / 'switchboard', 'start', FAKE_REFUSE='3')
        self.assertEqual(code, 0, out)
        uid = os.getuid()
        self.assertEqual(self.calls(), [['bootout', f'gui/{uid}/com.switchboard.server']]
                         + [['bootstrap', f'gui/{uid}', plist]] * 4)
        self.assertNotIn('could not start', out)

    def test_a_launch_agent_refused_for_good_is_started_again_as_it_is(self):
        self.launchd()
        code, out = self.run_script(INSTALL / 'switchboard', 'start', FAKE_REFUSE='99')
        self.assertEqual(code, 0, out)
        calls = self.calls()
        self.assertEqual(len([c for c in calls if c[0] == 'bootstrap']), 20)
        self.assertEqual(calls[-1], ['kickstart', '-k', f'gui/{os.getuid()}/com.switchboard.server'])
        self.assertNotIn('could not start', out)
        # nothing left to try: the reason launchd gave is said
        code, out = self.run_script(INSTALL / 'switchboard', 'start', FAKE_REFUSE='999', FAKE_KICK='3')
        self.assertIn('switchboard: could not start com.switchboard.server: Bootstrap failed: 5', out)


class ServiceFiles(Scripts):
    """A home folder whose name has a space, an ampersand, angle brackets and a percent sign in it."""
    FOLDER = 'R&D 100% <lab>'

    def install(self, system):
        self.shim('uname', UNAME % system)
        self.shim('launchctl', LOGGED)
        self.shim('systemctl', LOGGED)
        return self.run_script(self.kit('1.0.0'), '--yes', '--no-setup', '--plain', '--no-start', '--no-hub')

    def test_launch_agents_read_back_as_written(self):
        code, out = self.install('Darwin')
        self.assertEqual(code, 0, out)
        with open(self.home / 'Library/LaunchAgents/com.switchboard.server.plist', 'rb') as f:
            p = plistlib.load(f)
        app = str(self.share / 'app')
        self.assertEqual(p['ProgramArguments'], [str(self.fake / 'python3'), '-u', f'{app}/server/server.py'])
        self.assertEqual(p['WorkingDirectory'], app)
        self.assertEqual(p['StandardOutPath'], str(self.home / 'Library/Logs/Switchboard/server.log'))
        self.assertTrue(p['EnvironmentVariables']['PATH'].startswith(f'{self.bin}:'))
        with open(self.home / 'Library/LaunchAgents/com.switchboard.downloads.plist', 'rb') as f:
            self.assertEqual(plistlib.load(f)['ProgramArguments'][-1], str(self.share / 'downloads'))

    def test_systemd_units_quote_each_word(self):
        code, out = self.install('Linux')
        self.assertEqual(code, 0, out)
        unit = (self.home / '.config/systemd/user/switchboard.service').read_text()
        app = str(self.share / 'app').replace('%', '%%')
        py = str(self.fake / 'python3').replace('%', '%%')
        self.assertIn(f'\nExecStart="{py}" "-u" "{app}/server/server.py"\n', unit)
        self.assertIn(f'\nWorkingDirectory={app}\n', unit)
        self.assertIn('100%% <lab>', unit)
        self.assertNotIn('100% ', unit)


class Checksums(Scripts):
    def agent(self, sums):
        """install-agent.sh with a local sb.py against a server whose SHA256SUMS is `sums` (None: it has none)."""
        dl = self.tmp / 'dl'
        dl.mkdir(exist_ok=True)
        self.shim('curl', CURL)
        sb = self.tmp / 'sb.py'
        sb.write_text('print("agent set up")\n')
        if sums is not None:
            (dl / 'SHA256SUMS').write_text(sums.replace('SB', hashlib.sha256(sb.read_bytes()).hexdigest()))
        code, out = self.run_script(INSTALL / 'install-agent.sh', '--server', '10.0.0.5', '--sb', str(sb), FAKE_DL=str(dl))
        return code, out, (self.home / '.claude' / 'switchboard' / 'sb.py').exists()

    def test_a_list_without_the_file_does_not_vouch_for_it(self):
        code, out, there = self.agent('0' * 64 + '  switchboard-node-linux\n')
        self.assertEqual(code, 1, out)
        self.assertIn("(the server's SHA256SUMS has no line for sb.py)", out)
        self.assertIn('did not match its checksum; nothing was changed.', out)
        self.assertFalse(there)

    def test_a_wrong_line_fails_and_a_right_one_installs(self):
        code, out, there = self.agent('0' * 64 + '  sb.py\n')
        self.assertEqual((code, there), (1, False), out)
        code, out, there = self.agent('SB  sb.py\n')
        self.assertEqual((code, there), (0, True), out)

    def test_a_server_with_no_list_is_said_and_allowed(self):
        code, out, there = self.agent(None)
        self.assertEqual((code, there), (0, True), out)
        self.assertIn('(no SHA256SUMS on the server; not checked)', out)

    def test_an_empty_sha256_next_to_a_kit_is_a_failed_check(self):
        kit = self.tmp / 'kit.tar.gz'
        kit.write_bytes(b'not a kit')
        Path(f'{kit}.sha256').write_text('')
        code, out = self.run_script(INSTALL / 'get-server.sh', '--kit', str(kit), '--plain')
        self.assertEqual(code, 1, out)
        self.assertIn('get-server.sh: the .sha256 next to the kit has no SHA-256 in it. Nothing was installed.', out)
        # with none next to it, that is said and the kit goes on to be unpacked (and this one is not a kit)
        os.remove(f'{kit}.sha256')
        code, out = self.run_script(INSTALL / 'get-server.sh', '--kit', str(kit), '--plain')
        self.assertIn('No SHA-256 to check it against', out)
        self.assertIn('the kit did not unpack', out)
        self.assertFalse(self.share.exists())


if __name__ == '__main__':
    unittest.main()
