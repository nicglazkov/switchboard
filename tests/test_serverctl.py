"""Settings > Server (server/serverctl.py, selfupdate.py): the server's version and parts, its logs, a restart, and an
update that installs a release's kit only when it matches the SHA-256 the app read from the signed list."""
import hashlib, io, json, sys, tarfile, time, unittest
from pathlib import Path
from tempfile import TemporaryDirectory
from unittest import mock

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'server'))
sys.path.insert(0, str(ROOT / 'native' / 'scripts'))
import serverctl  # noqa: E402
import selfupdate  # noqa: E402
import publish_render  # noqa: E402

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
