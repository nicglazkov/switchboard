"""The server's own update: no step back by accident, and its progress file survives a power cut.

An older version was refused only when the server knew its own. One that could not read its VERSION file took any
version it was sent. The progress file was written without being forced to disk."""
import json, os, sys, unittest
from pathlib import Path
from tempfile import TemporaryDirectory
from unittest import mock
sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'server'))
import selfupdate, serverctl  # noqa: E402
from tests.test_durable import Order  # noqa: E402

SHA = 'a' * 64


class UpdateGuardTest(unittest.TestCase):
    def setUp(self):
        self.tmp = TemporaryDirectory()
        self.d = Path(self.tmp.name)
        self.addCleanup(self.tmp.cleanup)

    def installed(self, version):
        bin_ = self.d / 'bin'
        bin_.mkdir(exist_ok=True)
        (bin_ / 'switchboard').write_text('#!/bin/sh\n')
        (bin_ / 'switchboard-hub').write_text('')
        return mock.patch.multiple(serverctl, BIN=bin_, mode=mock.Mock(return_value='systemd'),
                                   version=mock.Mock(return_value=version), detach=mock.Mock())

    def test_a_server_that_does_not_know_its_version_takes_no_update(self):
        with self.installed(('', '')), mock.patch.object(serverctl.releases_mod, 'repo', return_value='acme/board'):
            with self.assertRaises(ValueError) as e:
                serverctl.start_update(self.d, '0.1.0', SHA)
            self.assertIn('does not know its own version', str(e.exception))
            self.assertEqual(serverctl.detach.call_count, 0)
        self.assertFalse((self.d / 'update').exists(), 'nothing was started or written')

    def test_one_that_knows_it_still_updates_and_still_refuses_a_step_back(self):
        with self.installed(('0.8.10', 'abc')), mock.patch.object(serverctl.releases_mod, 'repo', return_value='acme/board'):
            with self.assertRaises(ValueError):
                serverctl.start_update(self.d, '0.8.9', SHA)
            self.assertEqual(serverctl.start_update(self.d, '0.8.11', SHA)['update']['from'], '0.8.10')

    def test_the_first_progress_is_forced_to_disk_before_it_is_renamed(self):
        with self.installed(('0.8.10', 'abc')), mock.patch.object(serverctl.releases_mod, 'repo', return_value='acme/board'):
            with Order() as seen:
                serverctl.start_update(self.d, '0.8.11', SHA)
        self.assertIn('fsync', seen)
        self.assertLess(seen.index('fsync'), seen.index('replace'))

    def test_the_updater_forces_each_step_to_disk_too(self):
        st = selfupdate.State(self.d)
        with Order() as seen:
            st.put(step='downloading', ok=None)
        self.assertEqual(seen[:2], ['fsync', 'replace'])
        self.assertEqual(json.loads((self.d / 'state.json').read_text(encoding='utf-8'))['step'], 'downloading')
        self.assertEqual([n for n in os.listdir(self.d) if n.endswith('.tmp')], [])

    def test_a_progress_file_of_the_wrong_kind_does_not_stop_the_updater(self):
        (self.d / 'state.json').write_text('[1, 2]', encoding='utf-8')
        st = selfupdate.State(self.d)
        st.put(step='installing')
        self.assertEqual(json.loads((self.d / 'state.json').read_text(encoding='utf-8'))['step'], 'installing')


if __name__ == '__main__':
    unittest.main()
