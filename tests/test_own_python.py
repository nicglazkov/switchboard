"""Switchboard's own Python: the pinned download (fetch-python.sh), the kit that carries it (pack-server-kit.sh),
and the installers that unpack it and name it in what they write (install-server.sh, install-agent.sh, the
`switchboard` command). Run for real in a throwaway home folder, like tests/test_install_server.py. The "Python" in
the archives is a short script that runs the Python these tests run with and leaves a line in a file, so a test
can tell which one ran; nothing is downloaded from the internet."""
import gzip, hashlib, io, os, shutil, subprocess, sys, tarfile, unittest
from pathlib import Path

from tests import test_install_server as scripts

INSTALL = scripts.INSTALL
KINDS = ('mac-arm64', 'mac-x86_64', 'linux-x86_64', 'linux-arm64')
OWN = '''#!/bin/bash
# %s
echo "$*" >> "$FAKE_LOG.own"
case " $* " in *" -m http.server "*|*"/server/downloads.py "*) while :; do sleep 1; done ;; esac
exec "%s" "$@"
'''
BROKEN = '#!/bin/sh\nexit 1\n'
# curl as install-agent.sh calls it (curl -fsSL URL -o FILE), where the server's downloads are a folder
CURL = '''#!/bin/sh
p=${2#http://*:8131/}
cp "$FAKE_DL/$p" "$4" 2>/dev/null
'''


def archive(text):
    """A Python archive as python-build-standalone lays one out (python/bin/python3), with `text` as the program."""
    buf = io.BytesIO()
    with gzip.GzipFile(fileobj=buf, mode='wb', mtime=0) as gz, tarfile.open(fileobj=gz, mode='w') as tar:  # the same bytes each time
        for name, data, mode in (('python/bin/python3', text.encode(), 0o755), ('python/lib/marker', b'lib\n', 0o644)):
            ti = tarfile.TarInfo(name)
            ti.size, ti.mode = len(data), mode
            tar.addfile(ti, io.BytesIO(data))
    return buf.getvalue()


def sha(data):
    return hashlib.sha256(data).hexdigest()


class OwnPython(scripts.Installs):
    def python_in(self, folder, text, sums=None):
        """The kit's python/ folder: the same archive for every kind of computer, and its SHA256SUMS."""
        data = archive(text)
        folder.mkdir(parents=True, exist_ok=True)
        for k in KINDS:
            (folder / f'python-{k}.tar.gz').write_bytes(data)
        (folder / 'SHA256SUMS').write_text(''.join(f'{sums or sha(data)}  python-{k}.tar.gz\n' for k in KINDS))
        return sha(data)

    def kit(self, version, server=scripts.GOOD_SERVER, python=None, sums=None):
        shutil.rmtree(self.tmp / f'kit-{version}', ignore_errors=True)
        script = super().kit(version, server)
        if python:
            self.pin = self.python_in(script.parents[2] / 'python', python, sums)
        return script

    def go(self, version, python=None, sums=None, server=scripts.GOOD_SERVER, wait=20):
        return self.run_script(self.kit(version, server, python, sums), '--yes', '--no-setup', '--plain', '--no-service',
                               '--no-link', SWITCHBOARD_INSTALL_WAIT=str(wait))

    def ran_with_own(self):
        f = Path(self.env['FAKE_LOG'] + '.own')
        return f.read_text().splitlines() if f.exists() else []

    def test_the_kits_python_runs_the_server_and_an_update_with_the_same_one_keeps_it(self):
        own = self.share / 'python'
        code, out = self.go('1.0.0', OWN % ('one', sys.executable))
        self.assertEqual(code, 0, out)
        self.assertIn("Switchboard's own", out)
        self.assertEqual(self.answer(), '1.0.0')
        self.assertEqual((own / '.pin').read_text().strip(), self.pin)
        self.assertTrue(any('server.py' in l for l in self.ran_with_own()), 'the server runs with it')
        self.assertTrue(any('server/downloads.py --port 8131' in l for l in self.ran_with_own()), 'and so do the downloads')
        # the computers added to this server download it from there, with a list to check it against
        dl = self.share / 'downloads' / 'python'
        self.assertEqual(sorted(p.name for p in dl.iterdir()), ['SHA256SUMS'] + sorted(f'python-{k}.tar.gz' for k in KINDS))
        self.assertIn(f'{self.pin}  python-linux-x86_64.tar.gz', (dl / 'SHA256SUMS').read_text())
        # the same build again: it is not unpacked a second time
        (own / 'mine').write_text('still here')
        code, out = self.go('1.0.1', OWN % ('one', sys.executable))
        self.assertEqual((code, self.answer()), (0, '1.0.1'), out)
        self.assertTrue((own / 'mine').exists())
        # another build: it takes the old one's place, and nothing is left beside it
        code, out = self.go('1.0.2', OWN % ('two', sys.executable))
        self.assertEqual((code, self.answer()), (0, '1.0.2'), out)
        self.assertFalse((own / 'mine').exists())
        self.assertIn('# two', (own / 'bin' / 'python3').read_text())
        self.assertEqual((own / '.pin').read_text().strip(), self.pin)
        self.assertEqual(self.leftovers(), [])
        # a kit with none (a copy of the repository): the one that is here goes on being used
        code, out = self.go('1.0.3')
        self.assertEqual((code, self.answer()), (0, '1.0.3'), out)
        self.assertIn("Switchboard's own", out)
        self.assertTrue((own / 'bin' / 'python3').exists())
        self.assertTrue((dl / 'python-linux-x86_64.tar.gz').exists(), 'and the other computers can still download it')
        # the switchboard command runs with it too, and uninstall takes it away
        code, out = self.run_script(self.bin / 'switchboard', 'uninstall', '--yes')
        self.assertEqual(code, 0, out)
        self.assertIn("Switchboard's own Python", out)
        self.assertFalse(own.exists())

    def test_a_python_that_does_not_match_its_checksum_stops_the_install(self):
        code, out = self.go('1.0.0', OWN % ('one', sys.executable), sums='0' * 64)
        self.assertEqual(code, 1, out)
        self.assertIn("the kit's Python", out)
        self.assertIn('does not match its SHA-256. Nothing was installed.', out)
        self.assertFalse((self.share / 'app').exists())
        self.assertFalse((self.share / 'python').exists())
        self.assertFalse((self.share / 'python.new').exists())

    def test_a_python_that_does_not_run_here_is_said_and_this_computers_own_is_used(self):
        code, out = self.go('1.0.0', BROKEN)
        self.assertEqual((code, self.answer()), (0, '1.0.0'), out)
        self.assertIn("(Switchboard's own Python does not run on this computer, so this computer's own is used.)", out)
        self.assertFalse((self.share / 'python').exists())
        self.assertEqual(self.leftovers(), [])

    def test_an_update_that_does_not_start_puts_the_old_python_back(self):
        code, out = self.go('1.0.0', OWN % ('one', sys.executable))
        self.assertEqual((code, self.answer()), (0, '1.0.0'), out)
        code, out = self.go('1.0.1', OWN % ('two', sys.executable), server=scripts.BAD_SERVER, wait=3)
        self.assertNotEqual(code, 0, out)
        self.assertEqual(self.answer(), '1.0.0', out)
        self.assertIn('# one', (self.share / 'python' / 'bin' / 'python3').read_text())
        self.assertEqual(self.leftovers(), [])

    def test_a_python_laid_out_as_a_folder_is_used_the_same_way(self):
        # what the Mac app's kit carries, so that each program in it can be signed
        script = self.kit('1.0.0')
        kind = subprocess.run(['bash', '-c', 'case "$(uname -s)" in Darwin) o=mac ;; *) o=linux ;; esac; '
                               'case "$(uname -m)" in arm64|aarch64) a=arm64 ;; *) a=x86_64 ;; esac; '
                               '[ $o = mac ] && [ "$(sysctl -n hw.optional.arm64 2>/dev/null)" = 1 ] && a=arm64; echo $o-$a'],
                              capture_output=True, text=True).stdout.strip()
        tree = script.parents[2] / 'python' / kind
        (tree / 'bin').mkdir(parents=True)
        (tree / 'bin' / 'python3').write_text(OWN % ('tree', sys.executable))
        (tree / 'bin' / 'python3').chmod(0o755)
        (tree / '.pin').write_text('a' * 64 + '\n')
        code, out = self.run_script(script, '--yes', '--no-setup', '--plain', '--no-service', '--no-link')
        self.assertEqual((code, self.answer()), (0, '1.0.0'), out)
        self.assertIn('# tree', (self.share / 'python' / 'bin' / 'python3').read_text())
        self.assertEqual((self.share / 'python' / '.pin').read_text().strip(), 'a' * 64)
        # and the computers added to this server still get an archive of it
        dl = self.share / 'downloads' / 'python'
        with tarfile.open(dl / f'python-{kind}.tar.gz') as tar:
            self.assertIn('python/bin/python3', tar.getnames())
        got = sha((dl / f'python-{kind}.tar.gz').read_bytes())
        self.assertIn(f'{got}  python-{kind}.tar.gz', (dl / 'SHA256SUMS').read_text())


class AgentPython(scripts.Scripts):
    def agent(self, text=None, sums=None):
        dl = self.tmp / 'dl'
        shutil.rmtree(dl, ignore_errors=True)
        (dl / 'python').mkdir(parents=True)
        self.shim('curl', CURL)
        if text:
            data = archive(text)
            for k in KINDS:
                (dl / 'python' / f'python-{k}.tar.gz').write_bytes(data)
            (dl / 'python' / 'SHA256SUMS').write_text(''.join(f'{sums or sha(data)}  python-{k}.tar.gz\n' for k in KINDS))
        sb = self.tmp / 'sb.py'
        sb.write_text('import sys\nprint("agent set up with", sys.argv[0])\n')
        return self.run_script(INSTALL / 'install-agent.sh', '--server', '10.0.0.5', '--sb', str(sb), FAKE_DL=str(dl))

    def test_the_servers_python_is_downloaded_checked_and_used(self):
        own = self.home / '.local' / 'share' / 'switchboard' / 'python'
        code, out = self.agent(OWN % ('one', sys.executable))
        self.assertEqual(code, 0, out)
        self.assertIn(f"Python: Switchboard's own, in {own}", out)
        self.assertTrue(Path(self.env['FAKE_LOG'] + '.own').read_text().strip(), 'the agent was set up with it')
        self.assertTrue((own / '.pin').read_text().strip())
        # it is here now: nothing is downloaded the next time
        (own / 'mine').write_text('x')
        code, out = self.agent(OWN % ('two', sys.executable))
        self.assertEqual(code, 0, out)
        self.assertTrue((own / 'mine').exists())

    def test_a_wrong_checksum_stops_and_a_server_with_none_means_this_computers_own(self):
        own = self.home / '.local' / 'share' / 'switchboard' / 'python'
        code, out = self.agent(OWN % ('one', sys.executable), sums='0' * 64)
        self.assertEqual(code, 1, out)
        self.assertIn('did not match its checksum; nothing was changed.', out)
        self.assertFalse(own.exists())
        code, out = self.agent()
        self.assertEqual(code, 0, out)
        self.assertFalse(own.exists())
        code, out = self.agent(BROKEN)
        self.assertEqual(code, 0, out)
        self.assertIn("(Switchboard's own Python does not run on this computer, so this computer's own is used.)", out)
        self.assertFalse(own.exists())
        self.assertFalse(Path(str(own) + '.new').exists())


class Fetch(scripts.Scripts):
    def setUp(self):
        super().setUp()
        self.site = self.tmp / 'site'
        (self.site / '20990101').mkdir(parents=True)
        self.data = {k: archive(f'# {k}\n') for k in ('mac-arm64', 'linux-x86_64')}
        for k, d in self.data.items():
            (self.site / '20990101' / f'cpython-9.9.9+20990101-{k}.tar.gz').write_bytes(d)
        self.pins = self.tmp / 'pins.txt'
        self.write_pins()
        self.env.update(SWITCHBOARD_PYTHON_PINS=str(self.pins), SWITCHBOARD_PYTHON_CACHE=str(self.tmp / 'cache'),
                        PATH='/usr/bin:/bin:/usr/sbin:/sbin')

    def write_pins(self, **wrong):
        self.pins.write_text(f'# test pins\nrelease 20990101\nversion 9.9.9\nbase file://{self.site}\n' + ''.join(
            f'{k} {wrong.get(k.replace("-", "_"), sha(d))} cpython-9.9.9+20990101-{k}.tar.gz\n' for k, d in self.data.items()))

    def test_downloads_are_checked_and_kept_for_the_next_time(self):
        out_dir = self.tmp / 'out'
        code, out = self.run_script(INSTALL / 'fetch-python.sh', str(out_dir))
        self.assertEqual(code, 0, out)
        self.assertEqual(sorted(p.name for p in out_dir.iterdir()),
                         ['SHA256SUMS', 'VERSION', 'python-linux-x86_64.tar.gz', 'python-mac-arm64.tar.gz'])
        self.assertEqual((out_dir / 'python-mac-arm64.tar.gz').read_bytes(), self.data['mac-arm64'])
        self.assertEqual((out_dir / 'SHA256SUMS').read_text(), ''.join(
            f'{sha(self.data[k])}  python-{k}.tar.gz\n' for k in ('mac-arm64', 'linux-x86_64')))
        self.assertEqual((out_dir / 'VERSION').read_text(), '9.9.9 20990101\n')
        # the site is gone: what was kept is used, and only the kind asked for is written
        shutil.rmtree(self.site)
        code, out = self.run_script(INSTALL / 'fetch-python.sh', str(self.tmp / 'again'), 'linux-x86_64')
        self.assertEqual(code, 0, out)
        self.assertEqual(sorted(p.name for p in (self.tmp / 'again').iterdir()), ['SHA256SUMS', 'VERSION', 'python-linux-x86_64.tar.gz'])
        code, out = self.run_script(INSTALL / 'fetch-python.sh', str(self.tmp / 'more'), 'windows-x86_64')
        self.assertEqual(code, 1, out)
        self.assertIn('has no line for windows-x86_64', out)

    def test_a_download_that_does_not_match_its_pin_is_deleted_and_nothing_is_written_for_it(self):
        self.write_pins(mac_arm64='0' * 64)
        out_dir = self.tmp / 'out'
        code, out = self.run_script(INSTALL / 'fetch-python.sh', str(out_dir), 'mac-arm64')
        self.assertEqual(code, 1, out)
        self.assertIn('does not match its pinned SHA-256', out)
        self.assertIn('It was deleted.', out)
        self.assertEqual([p.name for p in (self.tmp / 'cache').iterdir()], [])
        self.assertFalse((out_dir / 'python-mac-arm64.tar.gz').exists())
        self.assertFalse((out_dir / 'SHA256SUMS').exists())
        # a kept file that was changed since is not trusted either: it is downloaded again
        self.write_pins()
        (self.tmp / 'cache' / 'cpython-9.9.9+20990101-mac-arm64.tar.gz').write_bytes(b'changed')
        code, out = self.run_script(INSTALL / 'fetch-python.sh', str(out_dir), 'mac-arm64')
        self.assertEqual(code, 0, out)
        self.assertEqual((out_dir / 'python-mac-arm64.tar.gz').read_bytes(), self.data['mac-arm64'])

    def test_the_pins_in_the_repository_are_whole(self):
        lines = [l.split() for l in (INSTALL / 'python-pins.txt').read_text().splitlines() if l.strip() and not l.startswith('#')]
        words = {l[0]: l[1:] for l in lines}
        self.assertEqual(sorted(k for k in words if k not in ('release', 'version', 'base')),
                         sorted(KINDS + ('windows-x86_64',)))
        for k in KINDS + ('windows-x86_64',):
            pin, asset = words[k]
            self.assertRegex(pin, r'^[0-9a-f]{64}$')
            self.assertIn(f'cpython-{words["version"][0]}+{words["release"][0]}-', asset)
            self.assertTrue(asset.endswith('-install_only_stripped.tar.gz'), asset)
        self.assertTrue(words['base'][0].startswith('https://'))


@unittest.skipIf(os.name == 'nt' or not shutil.which('bash') or not shutil.which('git'), 'needs bash and git')
class Pack(Fetch):
    def pack(self, *args):
        kit = self.tmp / 'packed' / 'switchboard-server-kit.tar.gz'
        code, out = self.run_script(INSTALL / 'pack-server-kit.sh', '--out', str(kit.parent), '--no-binaries', '--no-sources', '--no-vendor', *args,
                                    PATH=f'{self.fake}:/usr/bin:/bin:/usr/sbin:/sbin')
        return code, out, kit

    def test_the_kit_carries_python_with_its_checksums_or_is_not_made(self):
        code, out, kit = self.pack('--python', 'mac-arm64 linux-x86_64')
        self.assertEqual(code, 0, out)
        self.assertIn('Python: linux-x86_64, mac-arm64', out)
        with tarfile.open(kit) as tar:
            names, top = tar.getnames(), 'switchboard-server-kit'
            self.assertEqual(sorted(n[len(top) + 1:] for n in names if n.startswith(f'{top}/python/')),
                             ['python/SHA256SUMS', 'python/VERSION', 'python/python-linux-x86_64.tar.gz', 'python/python-mac-arm64.tar.gz'])
            self.assertEqual(tar.extractfile(f'{top}/python/python-mac-arm64.tar.gz').read(), self.data['mac-arm64'])
            self.assertIn(f'{top}/scripts/install/fetch-python.sh', names)
            self.assertIn(f'{top}/scripts/install/python-pins.txt', names)
        code, out, kit = self.pack('--no-python')
        self.assertEqual(code, 0, out)
        self.assertIn('Python: none', out)
        with tarfile.open(kit) as tar:
            self.assertFalse([n for n in tar.getnames() if '/python/' in n])
        kit.unlink()
        self.write_pins(mac_arm64='0' * 64)
        shutil.rmtree(self.tmp / 'cache')
        code, out, kit = self.pack('--python', 'mac-arm64')
        self.assertNotEqual(code, 0, out)
        self.assertIn('Python was not added, so no kit was made.', out)
        self.assertFalse(kit.exists())


if __name__ == '__main__':
    unittest.main()
