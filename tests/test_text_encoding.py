"""Text is read and written as UTF-8, whatever the computer's own encoding is.

On Windows the default is cp1252 and under LANG=C it is ASCII. A CLAUDE.md in UTF-8 then failed to read (after
settings.json had already been changed), and a chat title with an accent in ssh's output failed the whole machine."""
import os, re, stat, subprocess, sys, tempfile, unittest
from pathlib import Path
from unittest import mock
from tests.helpers import fresh_sb

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'server'))
import fleet  # noqa: E402

RULE = {'setup_subagent_rule': True}


def calls(src, name):
    """Each call of .name( in src, with its arguments, parentheses matched."""
    for m in re.finditer(r'\.' + name + r'\(', src):
        k, depth = m.end(), 1
        while depth:
            depth += {'(': 1, ')': -1}.get(src[k], 0)
            k += 1
        yield src[:m.start()].count('\n') + 1, src[m.end():k - 1]


class SourceTest(unittest.TestCase):
    FILES = sorted((ROOT / 'server').glob('*.py')) + [ROOT / 'agent' / 'sb.py']

    def test_no_text_file_is_read_or_written_in_the_computers_own_encoding(self):
        bad = []
        for f in self.FILES:
            src = f.read_text(encoding='utf-8')
            for name in ('read_text', 'write_text'):
                bad += [f'{f.name}:{n} {name}({args[:40]})' for n, args in calls(src, name) if 'encoding=' not in args]
        self.assertEqual(bad, [])


class SshOutputTest(unittest.TestCase):
    def test_what_a_machine_answers_is_read_as_utf8_and_never_fails_on_a_stray_byte(self):
        seen = {}

        def run(argv, **kw):
            seen.update(kw)
            return subprocess.CompletedProcess(argv, 0, '{"ok": true}', '')
        with mock.patch.object(subprocess, 'run', run):
            self.assertEqual(fleet.ssh_transport('me@x', {'verb': 'list'}, 5), {'ok': True})
        self.assertEqual((seen.get('encoding'), seen.get('errors')), ('utf-8', 'replace'))


class ClaudeMdTest(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp()).resolve()
        self.sb = fresh_sb(self.tmp)
        self.cdir = self.tmp / 'dotclaude'
        (self.cdir / 'switchboard').mkdir(parents=True)
        self.cm = self.cdir / 'CLAUDE.md'

    def test_bytes_that_are_not_utf8_come_back_out_as_they_went_in(self):
        mine = '# Mes règles 日本\n'.encode('utf-8') + b'caf\xe9 \x81\x8d\x8f\x90\x9d\n'
        self.cm.write_bytes(mine)
        self.assertEqual(self.sb.apply_setup(self.cdir, RULE), {'setup_subagent_rule': 'added'})
        after = self.cm.read_bytes()
        self.assertTrue(after.startswith(mine.rstrip(b'\n')), after[:80])
        self.assertIn(b'- Subagent models:', after)
        self.assertEqual(self.sb.apply_setup(self.cdir, {}), {'setup_subagent_rule': 'removed'})
        self.assertEqual(self.cm.read_bytes().rstrip(b'\r\n'), mine.rstrip(b'\n'))

    def test_it_is_read_as_utf8_where_the_computers_encoding_is_another(self):
        self.cm.write_bytes('règle “ici”\n'.encode('utf-8'))
        real = Path.read_text

        def as_cp1252(p, encoding=None, errors=None):
            return real(p, encoding=encoding or 'cp1252', errors=errors)  # what Windows does when none is given
        with mock.patch.object(Path, 'read_text', as_cp1252):
            self.sb.apply_setup(self.cdir, RULE)
        self.assertTrue(self.cm.read_bytes().startswith('règle “ici”'.encode('utf-8')))

    def test_it_is_never_half_written(self):
        self.cm.write_bytes(b'mine\n')
        with mock.patch.object(Path, 'write_text', side_effect=AssertionError('written in place')):
            with mock.patch.object(os, 'replace', side_effect=OSError('the disk is full')):
                with self.assertRaises(OSError):
                    self.sb.apply_setup(self.cdir, RULE)
        self.assertEqual(self.cm.read_bytes(), b'mine\n', 'the old file is whole')
        self.assertEqual([n for n in os.listdir(self.cdir) if n.endswith('.tmp')], [])

    @unittest.skipIf(os.name == 'nt', 'symlinks')
    def test_a_claude_md_kept_elsewhere_and_linked_stays_a_link(self):
        real = self.tmp / 'dotfiles' / 'CLAUDE.md'
        real.parent.mkdir()
        real.write_bytes(b'mine\n')
        os.symlink(real, self.cm)
        self.sb.apply_setup(self.cdir, RULE)
        self.assertTrue(self.cm.is_symlink())
        self.assertIn(b'- Subagent models:', real.read_bytes())

    @unittest.skipIf(os.name == 'nt', 'file modes')
    def test_the_file_keeps_who_may_read_it(self):
        for mode in (0o600, 0o640, 0o444):
            self.cm.write_bytes(b'mine\n')
            os.chmod(self.cm, mode)
            self.sb.apply_setup(self.cdir, RULE)
            self.assertIn(b'- Subagent models:', self.cm.read_bytes())
            self.assertEqual(stat.S_IMODE(os.stat(self.cm).st_mode), mode, oct(mode))
            self.sb.apply_setup(self.cdir, {})
            self.assertEqual(stat.S_IMODE(os.stat(self.cm).st_mode), mode, oct(mode))
            os.chmod(self.cm, 0o600)
            self.cm.unlink()

    @unittest.skipIf(os.name == 'nt', 'file modes')
    def test_a_linked_file_keeps_its_own_mode(self):
        real = self.tmp / 'dotfiles' / 'CLAUDE.md'
        real.parent.mkdir()
        real.write_bytes(b'mine\n')
        os.chmod(real, 0o600)
        os.symlink(real, self.cm)
        self.sb.apply_setup(self.cdir, RULE)
        self.assertEqual(stat.S_IMODE(os.stat(real).st_mode), 0o600)

    def test_the_explore_agent_is_written_and_known_again(self):
        self.assertEqual(self.sb.apply_setup(self.cdir, {'setup_explore_agent': True}), {'setup_explore_agent': 'added'})
        ex = self.cdir / 'agents' / 'Explore.md'
        self.assertEqual(ex.read_bytes().decode('utf-8').replace('\r\n', '\n'), self.sb.EXPLORE_MD)
        self.assertEqual(self.sb.apply_setup(self.cdir, {}), {'setup_explore_agent': 'removed'})
        self.assertFalse(ex.exists())


if __name__ == '__main__':
    unittest.main()
