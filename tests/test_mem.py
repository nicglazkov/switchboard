"""Claude's memory and orphans on a machine, and timeouts that end the whole process tree."""
import os, shutil, subprocess, sys, tempfile, time, unittest
from pathlib import Path
from tests.helpers import fresh_sb


def P(ppid, name, age_h=10, mb=100, tty='??'):
    return {'ppid': ppid, 'name': name, 'age': age_h * 3600, 'mb': mb, 'tty': tty}


class OrphanRulesTest(unittest.TestCase):
    def setUp(self):
        self.sb = fresh_sb(tempfile.mkdtemp())
        self.sb.IS_WIN = False

    def test_etime(self):
        self.assertEqual(self.sb._etime_s('38:34'), 38 * 60 + 34)
        self.assertEqual(self.sb._etime_s('11:38:03'), 11 * 3600 + 38 * 60 + 3)
        self.assertEqual(self.sb._etime_s('16-13:03:38'), 16 * 86400 + 13 * 3600 + 3 * 60 + 38)

    def test_only_the_cli_counts_as_claude(self):
        yes = ('claude', 'claude bg-pty-host', 'claude bg-spare', '/Users/you/.local/bin/claude')
        no = ('Claude', '/Applications/Claude.app/Contents/MacOS/Claude', 'Claude Helper (Renderer)',
              '/Applications/Claude.app/Contents/Helpers/chrome-native-host', 'claudette', 'zsh')
        for n in yes:
            self.assertTrue(self.sb.is_claude_cli(n), n)
        for n in no:
            self.assertFalse(self.sb.is_claude_cli(n), n)

    def fixture(self):
        """The Laptop as measured on 2026-10-12, plus planted orphans."""
        procs = {
            20350: P(1, 'login', tty='ttys004'), 20719: P(20350, 'claude', tty='ttys004'),      # a terminal session
            22305: P(1, 'claude bg-pty-host'), 22319: P(22305, 'claude bg-spare', tty='ttys003'),  # re-adopted worker
            89891: P(1, '/Users/you/.local/bin/claude'),                                          # the background service
            40167: P(89891, 'claude bg-pty-host'), 40182: P(40167, 'claude bg-spare'),
            40356: P(89891, 'claude bg-pty-host'), 40376: P(40356, 'claude bg-spare'),            # a warm spare
            501: P(1, 'claude', age_h=30, mb=410),                                                # a stray claude -p
            502: P(1, 'claude', age_h=2),                                                         # too young (reap skips)
            503: P(1, 'claude', tty='ttys009'),                                                   # still has a terminal
            504: P(1, 'claude bg-pty-host', age_h=40), 505: P(504, 'claude bg-spare', age_h=40),  # a leaked worker
            506: P(1, 'Claude'),                                                                  # the desktop app
            507: P(1, 'claude', age_h=9),                                                         # a stale daemon
        }
        agents = [{'kind': 'interactive', 'pid': 20719}, {'kind': 'background', 'pid': 22319}, {'kind': 'background', 'pid': 40182}]
        roster = {'supervisorPid': 89891, 'workers': {'e77f5d8c': {'pid': 22305}, 'c9fb3d27': {'pid': 40167}}}
        args = {501: ['claude', '-p', '--model', 'haiku'], 502: ['claude', '-p'], 503: ['claude'],
                504: ['claude', '--bg-pty-host', '--', '--bg-spare'], 505: ['claude', '--bg-spare'], 507: ['claude', 'daemon', 'run']}
        return procs, agents, roster, args

    def test_orphans_are_only_what_nothing_owns(self):
        procs, agents, roster, args = self.fixture()
        found = self.sb.find_orphans(agents, procs, roster, cmdlines=lambda pids: {p: args[p] for p in pids if p in args})
        self.assertEqual(sorted(o['pid'] for o in found), [501, 502, 504, 505])
        o = next(o for o in found if o['pid'] == 501)
        self.assertEqual((o['mb'], o['age_h']), (410, 30.0))

    def test_sessions_and_the_service_are_never_orphans(self):
        procs, agents, roster, args = self.fixture()
        keep = self.sb.protected_pids(agents, procs, roster)
        for pid in (20719, 22305, 22319, 89891, 40167, 40182, 40356, 40376):
            self.assertIn(pid, keep)
        self.assertNotIn(501, keep)

    def test_without_a_roster_background_helpers_are_left_alone(self):
        procs, agents, roster, args = self.fixture()
        found = self.sb.find_orphans(agents, procs, None, cmdlines=lambda pids: {p: args[p] for p in pids if p in args})
        self.assertEqual(sorted(o['pid'] for o in found), [501, 502])

    def test_unknown_command_line_is_left_alone(self):
        procs, agents, roster, _ = self.fixture()
        self.assertEqual(self.sb.find_orphans(agents, procs, roster, cmdlines=lambda pids: {}), [])

    def test_windows_parent_dead_or_reused(self):
        self.sb.IS_WIN = True
        procs = {10: {'ppid': 9, 'name': 'powershell.exe', 'created': 100.0}, 11: {'ppid': 10, 'name': 'claude.exe', 'created': 200.0},
                 12: {'ppid': 77, 'name': 'claude.exe', 'created': 200.0},                       # parent gone
                 13: {'ppid': 14, 'name': 'claude.exe', 'created': 200.0},
                 14: {'ppid': 1, 'name': 'notepad.exe', 'created': 900.0}}                       # pid reused later
        self.assertFalse(self.sb._parent_gone(procs[11], procs))
        self.assertTrue(self.sb._parent_gone(procs[12], procs))
        self.assertTrue(self.sb._parent_gone(procs[13], procs))

    def test_reap_never_goes_under_six_hours(self):
        procs, agents, roster, args = self.fixture()
        self.sb.find_orphans = lambda ags: [{'pid': 501, 'mb': 410, 'age_h': 30.0, 'start': 1.0},
                                            {'pid': 502, 'mb': 100, 'age_h': 2.0, 'start': 1.0}]
        killed = []
        r = self.sb.reap_orphans([], min_age_h=0.5, kill=lambda o: killed.append(o['pid']))
        self.assertEqual(killed, [501])
        self.assertEqual(r['killed'], [{'pid': 501, 'mb': 410, 'age_h': 30.0}])
        r = self.sb.reap_orphans([], dry_run=True, kill=lambda o: self.fail('dry run killed'))
        self.assertEqual([o['pid'] for o in r['killed']], [501])

    def test_claude_memory_is_every_claude_process_summed(self):
        # The Claude Code command line and its background helpers count; the desktop app and anything else don't.
        procs = {
            10: P(1, 'login', mb=50, tty='ttys001'), 11: P(10, 'claude', mb=1200.4, tty='ttys001'),
            12: P(11, 'claude bg-pty-host', mb=900.7, tty='ttys001'), 13: P(12, '/Users/you/.local/bin/claude', mb=1000.2, tty='ttys001'),
            14: P(1, 'Claude', mb=3000), 15: P(11, 'node', mb=500, tty='ttys001'),
        }
        self.sb.find_orphans = lambda agents, procs: []
        m = self.sb.claude_mem([{'pid': 11}], procs)
        self.assertEqual(m, {'claude_mb': 3101, 'procs': 3, 'orphans': []})
        self.assertEqual(self.sb.claude_mem([], {}), {'claude_mb': 0, 'procs': 0, 'orphans': []})

    def test_list_reports_memory(self):
        self.sb.claude_mem = lambda ags, procs=None: {'claude_mb': 3120, 'procs': 7, 'orphans': []}
        self.sb.claude_procs = lambda: []
        self.sb.list_sessions = lambda now, ags, rep=None, procs=None: []
        r = self.sb.rpc({'verb': 'list'}, time.time(), agents=[])
        self.assertEqual(r['mem'], {'claude_mb': 3120, 'procs': 7, 'orphans': []})
        self.sb.claude_mem = lambda ags, procs=None: 1 / 0
        self.assertIsNone(self.sb.rpc({'verb': 'list'}, time.time(), agents=[])['mem'])


@unittest.skipIf(sys.platform == 'win32', 'process groups')
class RealProcessTest(unittest.TestCase):
    def setUp(self):
        self.sb = fresh_sb(tempfile.mkdtemp())

    def _alive(self, pid):
        try:
            os.kill(pid, 0)
        except OSError:
            return False
        st = Path(f'/proc/{pid}/stat')
        return not (st.exists() and ') Z' in st.read_text())

    def test_timeout_ends_grandchildren(self):
        tmp = Path(tempfile.mkdtemp())
        pidf = tmp / 'grandchild'
        with self.assertRaises(subprocess.TimeoutExpired):
            self.sb.run_tree(['sh', '-c', f'sleep 60 & echo $! > {pidf}; wait'], '', 1)
        gpid = int(pidf.read_text())
        time.sleep(0.2)
        self.assertFalse(self._alive(gpid), 'a claude -p timeout left its children running')

    def test_run_to_file_timeout_ends_grandchildren(self):
        tmp = Path(tempfile.mkdtemp())
        pidf = tmp / 'grandchild'
        with self.assertRaises(RuntimeError):
            self.sb._run_to_file(['sh', '-c', f'sleep 60 & echo $! > {pidf}; wait'], 1)
        time.sleep(0.2)
        self.assertFalse(self._alive(int(pidf.read_text())))

    @unittest.skipUnless(sys.platform.startswith('linux') or sys.platform == 'darwin', 'ps')
    def test_a_real_orphan_is_found_and_killed(self):
        tmp = Path(tempfile.mkdtemp())
        exe = tmp / 'claude'
        os.symlink(shutil.which('sleep'), exe)
        pidf = tmp / 'pid'
        subprocess.run(['sh', '-c', f'( setsid {exe} 300 < /dev/null > /dev/null 2>&1 & echo $! > {pidf} ) &'], check=True)
        for _ in range(50):
            if pidf.exists() and pidf.read_text().strip():
                break
            time.sleep(0.05)
        pid = int(pidf.read_text())
        time.sleep(0.3)
        try:
            procs = self.sb.claude_procs()
            if procs.get(pid, {}).get('ppid') != 1:
                self.skipTest('orphans are re-parented to a subreaper here, not pid 1')
            found = [o for o in self.sb.find_orphans([], procs, {'workers': {}}) if o['pid'] == pid]
            self.assertEqual(len(found), 1, 'the planted orphan was not found')
            self.assertEqual(self.sb.find_orphans([{'pid': pid}], procs, {'workers': {}}), [], 'a session pid counted as orphan')
            self.sb._kill_orphan(found[0], sleep=lambda s: time.sleep(s))
            time.sleep(0.2)
            self.assertFalse(self._alive(pid))
        finally:
            try:
                os.kill(pid, 9)
            except OSError:
                pass


if __name__ == '__main__':
    unittest.main()
