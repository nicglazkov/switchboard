"""Settings > Server (server/serverctl.py, selfupdate.py): the server's version and parts, its logs, a restart, and an
update that installs a release's kit only when it matches the SHA-256 the app read from the signed list. And the
`switchboard` command's uninstall, run for real in a throwaway home folder with stand-ins for everything it calls."""
import hashlib, io, json, os, shutil, sys, tarfile, time, unittest
from pathlib import Path
from tempfile import TemporaryDirectory
from unittest import mock

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'server'))
sys.path.insert(0, str(ROOT / 'native' / 'scripts'))
import serverctl  # noqa: E402
import selfupdate  # noqa: E402
import publish_render  # noqa: E402
from tests import test_install_server as scripts  # noqa: E402  (the home folder and the PATH of stand-ins)

SHA = 'a' * 64


def kit_bytes(version='0.8.11', extra=None):
    buf = io.BytesIO()
    with tarfile.open(fileobj=buf, mode='w:gz') as t:
        for name, body in [('switchboard-server-kit/VERSION', f'{version} (abc1234)\n'.encode()),
                           ('switchboard-server-kit/scripts/install/install-server.sh', b'echo "== Starting"\n')] + (extra or []):
            info = tarfile.TarInfo(name)
            info.size = len(body)
            t.addfile(info, io.BytesIO(body))
    return buf.getvalue()


class VersionTest(unittest.TestCase):
    def test_the_installed_version_file_is_read(self):
        with TemporaryDirectory() as d:
            (Path(d) / 'server').mkdir()
            (Path(d) / 'VERSION').write_text('0.8.11 (abc1234)\n')
            with mock.patch.object(serverctl, 'HERE', Path(d) / 'server'):
                self.assertEqual(serverctl.version(), ('0.8.11', 'abc1234'))

    def test_a_checkout_reads_cargo_toml(self):
        with TemporaryDirectory() as d:
            (Path(d) / 'server').mkdir()
            (Path(d) / 'native').mkdir()
            (Path(d) / 'native' / 'Cargo.toml').write_text('[workspace.package]\nversion = "0.9.1"\n')
            with mock.patch.object(serverctl, 'HERE', Path(d) / 'server'):
                self.assertEqual(serverctl.version(), ('0.9.1', ''))

    def test_versions_compare_as_numbers(self):
        self.assertLess(serverctl.vkey('0.8.9'), serverctl.vkey('0.8.10'))
        self.assertEqual(serverctl.vkey('junk'), ())


class ManageTest(unittest.TestCase):
    def setUp(self):
        self.tmp = TemporaryDirectory()
        self.d = Path(self.tmp.name)
        self.addCleanup(self.tmp.cleanup)

    def installed(self):
        """As install-server.sh leaves it: the switchboard command and a hub in ~/.local/bin."""
        bin_ = self.d / 'bin'
        bin_.mkdir()
        (bin_ / 'switchboard').write_text('#!/bin/sh\n')
        (bin_ / 'switchboard-hub').write_text('')
        return mock.patch.multiple(serverctl, BIN=bin_, mode=mock.Mock(return_value='systemd'),
                                   version=mock.Mock(return_value=('0.8.10', 'abc')))

    def test_a_checkout_or_windows_says_why_it_cannot(self):
        self.assertIn('copy of the code', serverctl.why_not('checkout'))
        self.assertIn('Windows', serverctl.why_not('windows'))
        with mock.patch.object(serverctl, 'BIN', self.d):
            self.assertIn('install line', serverctl.why_not('systemd'))

    def test_an_update_starts_the_updater_apart_from_the_server(self):
        with self.installed(), mock.patch.object(serverctl, 'detach') as det, \
                mock.patch.object(serverctl.releases_mod, 'repo', return_value='acme/board'):
            r = serverctl.start_update(self.d, '0.8.11', SHA)
        self.assertEqual(r['update']['step'], 'starting')
        argv, name = det.call_args[0]
        self.assertEqual(name, 'switchboard-update')
        self.assertIn('https://github.com/acme/board/releases/download/v0.8.11/switchboard-server-kit.tar.gz', argv)
        self.assertIn(SHA, argv)
        self.assertNotIn('--no-hub', argv)
        self.assertTrue((self.d / 'update' / 'selfupdate.py').is_file(), 'it runs from a copy')
        self.assertEqual(serverctl.update_state(self.d)['version'], '0.8.11')

    def test_bad_requests_are_refused(self):
        with self.installed(), mock.patch.object(serverctl, 'detach') as det, \
                mock.patch.object(serverctl.releases_mod, 'repo', return_value='acme/board'):
            for want, sha in (('0.8.11', 'nope'), ('../x', SHA), ('0.8.9', SHA)):
                with self.assertRaises(ValueError):
                    serverctl.start_update(self.d, want, sha)
            serverctl.start_update(self.d, '0.8.11', SHA)
            with self.assertRaises(RuntimeError, msg='one at a time'):
                serverctl.start_update(self.d, '0.8.11', SHA)
            self.assertEqual(det.call_count, 1)

    def test_two_taps_at_the_same_moment_start_one_updater(self):
        # Audit, server 12: the "already running" check and the start were two steps, so two requests could both pass.
        import threading
        started, go, out = [], threading.Event(), []

        def slow(argv, name, log_to=None):
            started.append(argv)
            time.sleep(0.05)

        def tap():
            go.wait()
            try:
                serverctl.start_update(self.d, '0.8.11', SHA)
                out.append('started')
            except RuntimeError as e:
                out.append(str(e))
        with self.installed(), mock.patch.object(serverctl, 'detach', side_effect=slow), \
                mock.patch.object(serverctl.releases_mod, 'repo', return_value='acme/board'):
            ts = [threading.Thread(target=tap) for _ in range(6)]
            for t in ts:
                t.start()
            go.set()
            for t in ts:
                t.join()
        self.assertEqual(len(started), 1)
        self.assertEqual(sorted(out), ['An update is already running.'] * 5 + ['started'])
        self.assertTrue((self.d / 'update' / 'lock').is_file())

    def test_a_lock_left_by_an_updater_that_died_does_not_block_the_next(self):
        with self.installed(), mock.patch.object(serverctl, 'detach') as det, \
                mock.patch.object(serverctl.releases_mod, 'repo', return_value='acme/board'):
            serverctl.start_update(self.d, '0.8.11', SHA)
            u = self.d / 'update'
            # the state file says it failed, the lock is still there, and it is not brand new
            (u / 'state.json').write_text(json.dumps({'step': 'downloading', 'ok': False, 'at': time.time()}))
            with self.assertRaises(RuntimeError, msg='a lock made a moment ago is left alone'):
                serverctl.start_update(self.d, '0.8.11', SHA)
            old = time.time() - serverctl.LOCK_FRESH - 5
            os.utime(u / 'lock', (old, old))
            serverctl.start_update(self.d, '0.8.11', SHA)
            self.assertEqual(det.call_count, 2)
            # one that went quiet for 15 minutes counts as failed, and its lock goes the same way
            (u / 'state.json').write_text(json.dumps({'step': 'downloading', 'ok': None, 'at': time.time() - serverctl.STALE_AFTER - 5}))
            os.utime(u / 'lock', (old, old))
            serverctl.start_update(self.d, '0.8.11', SHA)
            self.assertEqual(det.call_count, 3)

    def test_an_updater_that_could_not_start_leaves_nothing_running(self):
        with self.installed(), mock.patch.object(serverctl, 'detach', side_effect=OSError('no room')), \
                mock.patch.object(serverctl.releases_mod, 'repo', return_value='acme/board'):
            with self.assertRaises(OSError):
                serverctl.start_update(self.d, '0.8.11', SHA)
        self.assertFalse((self.d / 'update' / 'lock').exists())
        s = serverctl.update_state(self.d)
        self.assertEqual((s['ok'], s['error']), (False, "The update couldn't start (no room)."))

    def test_github_turned_off_means_no_updates(self):
        with self.installed(), mock.patch.object(serverctl.releases_mod, 'repo', return_value=None):
            with self.assertRaises(PermissionError):
                serverctl.start_update(self.d, '0.8.11', SHA)

    def test_an_update_that_went_quiet_counts_as_failed(self):
        u = self.d / 'update'
        u.mkdir()
        (u / 'state.json').write_text(json.dumps({'step': 'downloading', 'ok': None, 'at': 1000}))
        self.assertIsNone(serverctl.update_state(self.d, now=1000 + 60)['ok'])
        s = serverctl.update_state(self.d, now=1000 + serverctl.STALE_AFTER + 1)
        self.assertFalse(s['ok'])
        self.assertIn('Try again', s['error'])

    def test_logs_come_from_one_part(self):
        with self.assertRaises(ValueError):
            serverctl.logs('../../etc/passwd')
        logs = self.d / 'logs'
        logs.mkdir()
        (logs / 'hub.log').write_text(''.join(f'line {i}\n' for i in range(50)))
        with mock.patch.multiple(serverctl, SHARE=self.d, mode=mock.Mock(return_value='plain')):
            text = serverctl.logs('hub', 10)
        self.assertEqual(text.splitlines(), [f'line {i}' for i in range(40, 50)])

    def test_info_has_what_settings_shows(self):
        with self.installed(), mock.patch.object(serverctl, '_probe', return_value=None):
            i = serverctl.info(self.d)
        self.assertEqual((i['version'], i['mode'], i['can_manage']), ('0.8.10', 'systemd', True))
        self.assertFalse(i['hub']['running'])
        self.assertIn('get-server.sh | bash', i['install_line'])


class SelfUpdateTest(unittest.TestCase):
    def test_a_kit_unpacks_to_its_one_folder(self):
        with TemporaryDirectory() as d:
            kit = Path(d) / 'kit.tar.gz'
            kit.write_bytes(kit_bytes())
            top = selfupdate.unpack(kit, Path(d) / 'out')
            self.assertEqual((top / 'VERSION').read_text().split()[0], '0.8.11')

    def test_a_kit_reaching_outside_is_refused(self):
        with TemporaryDirectory() as d:
            kit = Path(d) / 'kit.tar.gz'
            kit.write_bytes(kit_bytes(extra=[('../evil', b'x')]))
            with self.assertRaises(ValueError):
                selfupdate.unpack(kit, Path(d) / 'out')

    def test_a_kit_that_does_not_match_is_never_installed(self):
        with TemporaryDirectory() as d:
            d = Path(d)
            body = kit_bytes()
            (d / 'state.json').write_text('{}')

            def fake_download(url, to, st):
                to.write_bytes(body)
                return hashlib.sha256(body).hexdigest()
            argv = ['selfupdate.py', '--version', '0.8.11', '--sha256', SHA, '--repo', 'acme/board', '--dir', str(d),
                    '--url', 'https://example.invalid/kit']
            with mock.patch.object(sys, 'argv', argv), mock.patch.object(selfupdate, 'download', fake_download), \
                    mock.patch.object(selfupdate, 'install') as inst, mock.patch.object(selfupdate, 'BIN', d):
                selfupdate.main()
            inst.assert_not_called()
            s = json.loads((d / 'state.json').read_text())
            self.assertFalse(s['ok'])
            self.assertIn('signed list', s['error'])

    def test_a_matching_kit_is_installed_and_reported_done(self):
        with TemporaryDirectory() as d:
            d = Path(d)
            body = kit_bytes()
            sha = hashlib.sha256(body).hexdigest()
            (d / 'state.json').write_text('{}')

            def fake_download(url, to, st):
                to.write_bytes(body)
                return sha
            argv = ['selfupdate.py', '--version', '0.8.11', '--sha256', sha, '--repo', 'acme/board', '--dir', str(d),
                    '--url', 'https://example.invalid/kit', '--no-hub']
            with mock.patch.object(sys, 'argv', argv), mock.patch.object(selfupdate, 'download', fake_download), \
                    mock.patch.object(selfupdate, 'install', return_value=(0, [])) as inst:
                selfupdate.main()
            self.assertTrue(inst.call_args[0][1], '--no-hub is passed on')
            s = json.loads((d / 'state.json').read_text())
            self.assertEqual((s['step'], s['ok']), ('done', True))
            self.assertFalse((d / 'work').exists())


# The helper as `switchboard uninstall` asks it to take itself out of Claude Code.
HELPER = '''import os, sys
open(os.environ['FAKE_LOG'], 'a').write('helper ' + ' '.join(sys.argv[1:]) + '\\n')
sys.exit(int(os.environ.get('FAKE_HELPER_RC') or 0))
'''
NODE = '''#!/bin/sh
echo "node $*" >> "$FAKE_LOG"
[ -z "${FAKE_NODE_FAILS:-}" ] || { echo "its service would not stop" >&2; exit 1; }
rm -f "$HOME/.config/switchboard/node.json"
echo "Removed the link to the hub."
'''
SERVICES = '''#!/bin/sh
echo "services $*" >> "$FAKE_LOG"
exit 0
'''


class UninstallTest(scripts.Scripts):
    """`switchboard uninstall [--purge]`: one command for the helper, the node, the server and Switchboard's own
    Python, in that order. Settings, history and keys stay unless --purge is given."""

    def setUp(self):
        super().setUp()
        self.shim('uname', scripts.UNAME % 'Darwin')
        self.shim('launchctl', SERVICES)
        self.cfg = self.home / '.config' / 'switchboard'
        self.helper = self.home / '.claude' / 'switchboard'
        self.agents = self.home / 'Library' / 'LaunchAgents'
        self.logs = self.home / 'Library' / 'Logs' / 'Switchboard'
        for d in (self.cfg, self.helper / 'queue', self.agents, self.logs, self.bin, self.share / 'app' / 'server',
                  self.share / 'downloads' / 'node', self.share / 'python' / 'bin', self.share / 'logs', self.share / 'update'):
            d.mkdir(parents=True)
        for f in (self.cfg / 'server.json', self.cfg / 'fleet.json', self.cfg / 'node.json', self.cfg / 'term.key',
                  self.share / 'history.jsonl', self.share / 'env', self.logs / 'server.log',
                  self.agents / 'com.switchboard.server.plist', self.agents / 'com.switchboard.hub.plist',
                  self.agents / 'com.other.thing.plist', self.bin / 'switchboard-hub', self.bin / 'other-program'):
            f.write_text('x')
        (self.helper / 'sb.py').write_text(HELPER)
        (self.home / '.claude' / 'settings.json').write_text('{"model": "opus"}')
        node = self.bin / 'switchboard-node'
        node.write_text(NODE)
        node.chmod(0o755)
        self.ctl = self.bin / 'switchboard'
        shutil.copy(scripts.INSTALL / 'switchboard', self.ctl)
        self.ctl.chmod(0o755)

    def uninstall(self, *args, **env):
        return self.run_script(self.ctl, 'uninstall', *args, **env)

    def calls(self):
        log = Path(self.env['FAKE_LOG'])
        return log.read_text().splitlines() if log.exists() else []

    def gone(self, *paths):
        return [str(p.relative_to(self.home)) for p in paths if p.exists()]

    def program_paths(self):
        return (self.share / 'app', self.share / 'downloads', self.share / 'python', self.share / 'update', self.share / 'env',
                self.bin / 'switchboard', self.bin / 'switchboard-hub', self.bin / 'switchboard-node', self.helper / 'sb.py',
                self.agents / 'com.switchboard.server.plist', self.agents / 'com.switchboard.hub.plist', self.cfg / 'node.json')

    def test_everything_goes_in_order_and_your_data_stays(self):
        code, out = self.uninstall('--yes')
        self.assertEqual(code, 0, out)
        calls = self.calls()
        self.assertEqual(calls[:2], ['helper uninstall', 'node uninstall'], 'the helper first, then the node')
        uid = os.getuid()
        self.assertEqual([c for c in calls[2:] if 'bootout' in c][-3:],
                         [f'services bootout gui/{uid}/com.switchboard.{p}' for p in ('server', 'hub', 'downloads')])
        self.assertEqual(self.gone(*self.program_paths()), [])
        # what is the user's own stays, and so does everything that is not Switchboard's
        for p in (self.cfg / 'server.json', self.cfg / 'fleet.json', self.cfg / 'term.key', self.share / 'history.jsonl',
                  self.logs / 'server.log', self.helper / 'queue', self.agents / 'com.other.thing.plist',
                  self.bin / 'other-program', self.home / '.claude' / 'settings.json'):
            self.assertTrue(p.exists(), p)
        self.assertEqual(out, '''Switchboard is removed from this computer.
Removed:
  the helper: its hooks, status line and tools are out of Claude Code (~/.claude)
  switchboard-node, which linked this computer to a server
    Removed the link to the hub.
  the server, its hub, its downloads and what started them
  Switchboard's own Python
  the switchboard command
Left on this computer:
  ~/.config/switchboard  settings and keys
  ~/.local/share/switchboard  history and logs
  ~/Library/Logs/Switchboard  logs
  ~/.claude/switchboard  the helper's own files
Settings, history and keys are kept in case you install it again. Delete those folders if you do not want them.
Your other computers each have their own copy. Run this on them too.
''')

    def test_purge_takes_settings_history_and_keys_too(self):
        code, out = self.uninstall('--purge', '--yes')
        self.assertEqual(code, 0, out)
        self.assertEqual(self.gone(self.cfg, self.share, self.logs, self.helper, *self.program_paths()), [])
        for p in (self.home / '.claude' / 'settings.json', self.bin / 'other-program', self.agents / 'com.other.thing.plist'):
            self.assertTrue(p.exists(), p)
        for line in ('  ~/.config/switchboard  settings and keys', '  ~/.local/share/switchboard  history and logs',
                     "  ~/.claude/switchboard  the helper's own files", 'Nothing of Switchboard is left on this computer.'):
            self.assertIn(line + '\n', out)
        self.assertNotIn('Left on this computer', out)

    def test_a_part_that_will_not_go_is_said_and_the_rest_still_goes(self):
        code, out = self.uninstall('--purge', '--yes', FAKE_NODE_FAILS='1', FAKE_HELPER_RC='1')
        self.assertEqual(code, 0, out)
        self.assertTrue((self.bin / 'switchboard-node').exists() and (self.helper / 'sb.py').exists())
        self.assertIn('  ~/.claude/switchboard/sb.py  the helper, which could not take itself out of Claude Code\n', out)
        self.assertIn('  ~/.local/bin/switchboard-node  it could not remove itself. Run: switchboard-node uninstall\n'
                      '    its service would not stop\n', out)
        # a helper that is still in Claude Code keeps the Python it runs with, even with --purge
        self.assertTrue((self.share / 'python' / 'bin').is_dir())
        self.assertIn("  ~/.local/share/switchboard/python  Switchboard's own Python, which the helper still uses\n", out)
        self.assertEqual(self.gone(self.share / 'app', self.bin / 'switchboard-hub', self.bin / 'switchboard', self.cfg), [])

    def test_only_a_server_here_and_nothing_else(self):
        for p in (self.helper, self.share / 'python'):
            shutil.rmtree(p)
        (self.bin / 'switchboard-node').unlink()
        code, out = self.uninstall('--yes')
        self.assertEqual(code, 0, out)
        self.assertEqual(self.calls()[0].split()[0], 'services', 'no helper and no node to ask')
        self.assertNotIn('helper', out)
        self.assertNotIn('switchboard-node', out)
        self.assertNotIn('Python', out)

    def test_an_option_it_does_not_know_removes_nothing(self):
        code, out = self.uninstall('--everything')
        self.assertEqual(code, 2, out)
        self.assertIn('usage: switchboard uninstall [--purge] [--yes]', out)
        self.assertEqual(self.calls(), [])
        self.assertTrue(self.ctl.exists() and (self.share / 'app').exists())


class ManifestTest(unittest.TestCase):
    def test_the_list_names_the_kit_of_this_version_only(self):
        with TemporaryDirectory() as d:
            d = Path(d)
            (d / 'Switchboard-mac.zip').write_bytes(b'mac')
            (d / 'Switchboard-mac.zip.build').write_text('20261023120000')
            kit = d / 'switchboard-server-kit.tar.gz'
            kit.write_bytes(kit_bytes('0.8.11'))
            m = json.loads(publish_render.make_manifest(str(d), '0.8.11', str(kit)))
            self.assertEqual(m['files']['server']['sha256'], hashlib.sha256(kit.read_bytes()).hexdigest())
            m = json.loads(publish_render.make_manifest(str(d), '0.8.12', str(kit)))
            self.assertNotIn('server', m['files'])


if __name__ == '__main__':
    unittest.main()
