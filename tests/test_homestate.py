"""The home computer's state as one stream (server/homestate.py): what is packed and what stays, and that an
archive which is cut, changed or from a newer version changes nothing on the computer it is unpacked on."""
import io, json, os, subprocess, sys, tarfile, tempfile, unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'server'))
import homestate  # noqa: E402


def tree(root):
    """{relative name: bytes} of every file under root."""
    root = Path(root)
    return {p.relative_to(root).as_posix(): p.read_bytes() for p in sorted(root.rglob('*')) if p.is_file()}


class Base(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name).resolve()
        self.addCleanup(self.tmp.cleanup)

    def folders(self, name):
        c, s = self.root / name / 'config', self.root / name / 'state'
        c.mkdir(parents=True)
        s.mkdir(parents=True)
        return c, s

    def home(self):
        """A home computer's two folders, with something of every kind."""
        c, s = self.folders('old')
        put = lambda p, text: (p.parent.mkdir(parents=True, exist_ok=True), p.write_text(text, encoding='utf-8'))
        put(c / 'server.json', json.dumps({'bind': ['10.0.0.5'], 'hosts': ['10.0.0.5', 'old'], 'keys': 'both',
                                           'origins': ['http://10.0.0.5:8130'], 'peers': ['203.0.113.0/24'],
                                           'allow': ['private']}))
        put(c / 'fleet.json', '[{"name": "laptop"}]')
        put(c / 'term.key', 'term-key')
        put(c / 'stars.json', '{"a": 1}')
        put(c / 'backup.key', '{"v": 1}')
        put(c / 'roster.json', '{"v": 1}')
        # this computer's own: never packed
        put(c / 'node.json', '{"machine": "old"}')
        put(c / 'node.json.bak', '{"machine": "old"}')
        put(c / 'node.lock', '')
        put(c / 'keys.lock', '')
        put(c / 'device.key', 'K' * 32)
        put(c / 'device.key.bak', 'K' * 32)
        put(c / 'device.key.next', 'N' * 32)
        put(c / 'member.key.old', 'M' * 32)
        put(c / 'blocked.json', '{}')
        put(c / 'home-move.json', '{}')
        put(c / 'home-stepdown.json', '{}')
        put(c / 'fleet.json.bad-20261122-101010', 'zeros')
        put(s / 'paired.json', '{"v": 1, "devices": []}')
        put(s / 'paired.json.bak', '{"v": 1, "devices": []}')
        put(s / 'roster-state.json', '{"v": 1, "epoch": 2, "seq": 9}')
        put(s / 'nodes.json', '{"laptop": {"hash": "x"}}')
        put(s / 'threads.json', '{"t": []}')
        put(s / 'library' / 'tool' / 'run.sh', 'echo hi')
        put(s / 'library.json', '{}')
        # rebuilt or this computer's own: never packed
        put(s / 'browse_index.json', '{}')
        put(s / 'machines.json', '{"origins": []}')
        put(s / 'machines.json.bak', '{"origins": []}')
        put(s / 'server.lock', '')
        put(s / 'logs' / 'server.log', 'a line')
        put(s / 'app' / 'server' / 'server.py', 'print()')
        put(s / 'python' / 'bin' / 'python3', 'elf')
        put(s / 'downloads' / 'node' / 'SHA256SUMS', 'sums')
        put(s / 'run' / 'server.pid', '1')
        put(s / 'update' / 'state.json', '{}')
        put(s / 'home-backup' / 'x.sbk', 'sealed')
        put(s / 'home-incoming' / 'archive.tar', 'tar')
        put(s / 'threads.json.123.tmp', 'half')
        return c, s

    def pack(self, c, s):
        out = self.root / 'state.tar'
        r = homestate.pack(c, s, out)
        return out, r


class PackTest(Base):
    def test_a_round_trip_between_two_computers_keeps_what_is_the_home_and_leaves_what_is_the_computers(self):
        c, s = self.home()
        out, r = self.pack(c, s)
        self.assertEqual(r['sha256'], homestate.sha256_of(out))
        self.assertEqual(r['bytes'], out.stat().st_size)
        with tarfile.open(out) as t:
            names = t.getnames()
        self.assertEqual(names[0], 'homestate.json')
        self.assertEqual(names[-1], 'end.json')
        packed = set(n for n in names[1:-1] if not n.endswith('/'))
        self.assertEqual(packed, {
            'config/server.json', 'config/fleet.json', 'config/term.key', 'config/stars.json', 'config/backup.key',
            'config/roster.json', 'state/paired.json', 'state/paired.json.bak', 'state/roster-state.json',
            'state/nodes.json', 'state/threads.json', 'state/library/tool/run.sh', 'state/library.json'})
        for never in ('node.json', 'device.key', 'keys.lock', 'server.log', 'browse_index', 'machines.json',
                      '.bad-', '.tmp', 'home-move', 'blocked', 'member.key', 'app/', 'python/', 'downloads/'):
            self.assertFalse([n for n in names if never in n], never)

        # the other computer has a server.json and a key of its own, and an old thread list
        c2, s2 = self.folders('new')
        (c2 / 'server.json').write_text(json.dumps({'bind': ['10.0.0.9'], 'hosts': ['10.0.0.9', 'new'],
                                                    'origins': ['http://10.0.0.9:8130'], 'peers': []}))
        (c2 / 'node.json').write_text('{"machine": "new"}')
        (c2 / 'device.key').write_bytes(b'T' * 32)
        (c2 / 'blocked.json').write_text('{"v": 1}')
        (s2 / 'threads.json').write_text('{"old": true}')
        (s2 / 'chats.json').write_text('{"only": "here"}')
        (s2 / 'logs').mkdir()
        (s2 / 'logs' / 'node.log').write_text('kept')
        before = tree(self.root / 'new')

        st = homestate.stage(c2, s2, out, expect=r['sha256'])
        self.assertEqual(st['sha256'], r['sha256'])
        self.assertEqual(homestate.status(c2, s2)['staged'], r['sha256'])
        # staging alone changes nothing a program reads
        now = {k: v for k, v in tree(self.root / 'new').items() if '.home-staging' not in k}
        self.assertEqual(now, before)

        homestate.swap(c2, s2)
        self.assertIsNone(homestate.status(c2, s2)['staged'])
        self.assertFalse((c2 / '.home-staging').exists() or (s2 / '.home-staging').exists())
        got = json.loads((c2 / 'server.json').read_text())
        self.assertEqual((got['bind'], got['hosts'], got['origins']),
                         (['10.0.0.9'], ['10.0.0.9', 'new'], ['http://10.0.0.9:8130']), 'its own addresses stay')
        self.assertEqual((got['keys'], got['peers'], got['allow']), ('both', ['203.0.113.0/24'], ['private']))
        self.assertEqual((c2 / 'node.json').read_text(), '{"machine": "new"}')
        self.assertEqual((c2 / 'device.key').read_bytes(), b'T' * 32)
        self.assertEqual((c2 / 'blocked.json').read_text(), '{"v": 1}')
        self.assertEqual((s2 / 'logs' / 'node.log').read_text(), 'kept')
        self.assertEqual((s2 / 'threads.json').read_text(), '{"t": []}')
        self.assertEqual((s2 / 'library' / 'tool' / 'run.sh').read_text(), 'echo hi')
        self.assertEqual((s2 / 'paired.json').read_text(), '{"v": 1, "devices": []}')
        # what was the other computer's own home state is put aside whole, not mixed in
        self.assertFalse((s2 / 'chats.json').exists())
        self.assertEqual((s2 / '.home-before' / 'chats.json').read_text(), '{"only": "here"}')
        self.assertEqual((s2 / '.home-before' / 'threads.json').read_text(), '{"old": true}')

    def test_a_computer_with_no_server_settings_gets_none_of_the_old_addresses(self):
        c, s = self.home()
        out, r = self.pack(c, s)
        c2, s2 = self.folders('new')
        homestate.stage(c2, s2, out)
        homestate.swap(c2, s2)
        got = json.loads((c2 / 'server.json').read_text())
        self.assertEqual(got, {'keys': 'both', 'peers': ['203.0.113.0/24'], 'allow': ['private']})

    def test_one_folder_for_both(self):
        d = self.root / 'one'
        d.mkdir()
        (d / 'server.json').write_text('{"keys": "only"}')
        (d / 'paired.json').write_text('{"v": 1}')
        (d / 'node.json').write_text('{}')
        out = self.root / 'one.tar'
        r = homestate.pack(d, d, out)
        self.assertEqual(r['files'], 2)
        e = self.root / 'two'
        e.mkdir()
        homestate.stage(e, e, out)
        homestate.swap(e, e)
        self.assertEqual(sorted(p.name for p in e.iterdir()), ['paired.json', 'server.json'])

    def test_links_are_not_followed(self):
        c, s = self.home()
        secret = self.root / 'outside.txt'
        secret.write_text('not home state')
        try:
            os.symlink(secret, s / 'notes.json')
            os.symlink(self.root, s / 'everything')
        except (OSError, NotImplementedError):
            self.skipTest('no links here')
        out, _ = self.pack(c, s)
        with tarfile.open(out) as t:
            names = t.getnames()
        self.assertFalse([n for n in names if 'notes.json' in n or 'everything' in n])


class RefuseTest(Base):
    def packed(self):
        c, s = self.home()
        out, r = self.pack(c, s)
        c2, s2 = self.folders('new')
        (c2 / 'node.json').write_text('{"machine": "new"}')
        (s2 / 'threads.json').write_text('{"old": true}')
        return out, r, c2, s2

    def untouched(self, c2, s2):
        self.assertEqual(tree(c2), {'node.json': b'{"machine": "new"}'})
        self.assertEqual(tree(s2), {'threads.json': b'{"old": true}'})

    def test_a_cut_archive_changes_nothing(self):
        out, r, c2, s2 = self.packed()
        raw = out.read_bytes()
        for keep in (0, 100, 2000, len(raw) // 2, len(raw) - 1024, len(raw) - 10240):
            cut = self.root / 'cut.tar'
            cut.write_bytes(raw[:keep])
            with self.assertRaises(homestate.Refused, msg=keep):
                homestate.stage(c2, s2, cut)
            self.untouched(c2, s2)
            with self.assertRaises(homestate.Refused):
                homestate.swap(c2, s2)

    def test_a_changed_byte_is_noticed(self):
        out, r, c2, s2 = self.packed()
        raw = bytearray(out.read_bytes())
        at = raw.find(b'{"t": []}')
        self.assertGreater(at, 0)
        raw[at + 2] ^= 1
        bad = self.root / 'bad.tar'
        bad.write_bytes(bytes(raw))
        with self.assertRaises(homestate.Refused) as e:
            homestate.stage(c2, s2, bad)
        self.assertEqual(e.exception.kind, 'damaged')
        self.untouched(c2, s2)
        # and the hash the sender named is checked before anything is unpacked
        with self.assertRaises(homestate.Refused) as e:
            homestate.stage(c2, s2, out, expect='0' * 64)
        self.assertEqual(e.exception.kind, 'damaged')
        self.untouched(c2, s2)

    def test_a_newer_format_is_refused(self):
        out, r, c2, s2 = self.packed()
        newer = self.root / 'newer.tar'
        with tarfile.open(newer, 'w') as t:
            for name, body in (('homestate.json', json.dumps({'v': homestate.FORMAT + 1}).encode()),
                               ('end.json', b'{"files": []}')):
                info = tarfile.TarInfo(name)
                info.size = len(body)
                t.addfile(info, io.BytesIO(body))
        with self.assertRaises(homestate.Refused) as e:
            homestate.stage(c2, s2, newer)
        self.assertEqual(e.exception.kind, 'newer')
        self.assertIn('newer version', str(e.exception))
        self.untouched(c2, s2)

    def make(self, members):
        """An archive with these (name, bytes or None for a folder, kind) members between a good first and last."""
        import hashlib
        p = self.root / 'made.tar'
        files = []
        with tarfile.open(p, 'w') as t:
            def add(name, body, kind=tarfile.REGTYPE, link=''):
                info = tarfile.TarInfo(name)
                info.type, info.linkname = kind, link
                info.size = len(body) if kind == tarfile.REGTYPE else 0
                t.addfile(info, io.BytesIO(body) if kind == tarfile.REGTYPE else None)
            add('homestate.json', json.dumps({'v': 1}).encode())
            for name, body, kind, link in members:
                add(name, body, kind, link)
                if kind == tarfile.REGTYPE:
                    files.append([name, len(body), hashlib.sha256(body).hexdigest()])
            add('end.json', json.dumps({'files': files}).encode())
        return p

    def test_an_archive_cannot_write_outside_the_two_folders_or_over_this_computers_own_files(self):
        out, r, c2, s2 = self.packed()
        R, L, S = tarfile.REGTYPE, tarfile.LNKTYPE, tarfile.SYMTYPE
        for members in (
            [('config/../../evil.txt', b'x', R, '')],
            [('/etc/evil', b'x', R, '')],
            [('elsewhere/a.json', b'x', R, '')],
            [('config/node.json', b'{}', R, '')],
            [('config/device.key', b'K' * 32, R, '')],
            [('config/device.key.bak', b'K' * 32, R, '')],
            [('state/app/server/server.py', b'x', R, '')],
            [('state/.home-staging/x', b'x', R, '')],
            [('state/link', b'', S, '/etc/passwd')],
            [('state/hard', b'', L, 'config/server.json')],
            [('state/a.json', b'1', R, ''), ('state/a.json', b'2', R, '')],
        ):
            with self.assertRaises(homestate.Refused, msg=members[0][0]) as e:
                homestate.stage(c2, s2, self.make(members))
            self.assertEqual(e.exception.kind, 'damaged', members[0][0])
            self.untouched(c2, s2)
        self.assertFalse((self.root / 'evil.txt').exists())
        # and a good one made the same way is taken
        homestate.stage(c2, s2, self.make([('state/a.json', b'1', R, '')]))
        homestate.discard(c2, s2)
        self.untouched(c2, s2)

    def test_unpacking_is_refused_while_a_server_holds_the_folder(self):
        out, r, c2, s2 = self.packed()
        held = homestate.hold(s2)
        self.assertIsNotNone(held)
        try:
            self.assertTrue(homestate.status(c2, s2)['held'])
            with self.assertRaises(homestate.Refused) as e:
                homestate.stage(c2, s2, out)
            self.assertEqual(e.exception.kind, 'held')
        finally:
            held.close()
        (s2 / 'server.lock').unlink()
        self.untouched(c2, s2)
        homestate.stage(c2, s2, out)
        held = homestate.hold(s2)
        try:
            with self.assertRaises(homestate.Refused) as e:
                homestate.swap(c2, s2)
            self.assertEqual(e.exception.kind, 'held')
        finally:
            held.close()
        self.assertFalse(homestate.status(c2, s2)['held'])
        homestate.swap(c2, s2)
        self.assertEqual((s2 / 'threads.json').read_text(), '{"t": []}')

    def test_a_swap_that_stopped_half_way_is_finished_by_the_next_one(self):
        out, r, c2, s2 = self.packed()
        homestate.stage(c2, s2, out)
        real, n = os.replace, [0]

        def dies(a, b):
            n[0] += 1
            if n[0] == 4:
                raise KeyboardInterrupt('power cut')
            return real(a, b)
        os.replace = dies
        try:
            with self.assertRaises(KeyboardInterrupt):
                homestate.swap(c2, s2)
        finally:
            os.replace = real
        self.assertIsNotNone(homestate.status(c2, s2)['staged'], 'still to finish')
        homestate.swap(c2, s2)
        want = {k.split('/', 1)[1]: v for k, v in self.archive_files(out).items() if k.startswith('state/')}
        got = {k: v for k, v in tree(s2).items() if not k.startswith('.home-before')}
        self.assertEqual(got, want)
        self.assertEqual((s2 / '.home-before' / 'threads.json').read_text(), '{"old": true}')

    def archive_files(self, out):
        with tarfile.open(out) as t:
            return {m.name: t.extractfile(m).read() for m in t.getmembers()
                    if m.isfile() and m.name not in ('homestate.json', 'end.json')}

    def test_folders_that_are_not_scratch_are_never_removed(self):
        with self.assertRaises(homestate.Refused):
            homestate._remove(self.root / 'new' / 'state', self.root / 'new' / 'state')
        (self.root / 'x').mkdir()
        with self.assertRaises(homestate.Refused):
            homestate._remove(self.root / 'x', self.root)


class CommandTest(Base):
    def run_cmd(self, *args):
        tool = Path(homestate.__file__)
        r = subprocess.run([sys.executable, '-I', str(tool), *map(str, args)], capture_output=True, text=True)
        return r.returncode, json.loads(r.stdout.strip().splitlines()[-1])

    def test_the_command_line_says_what_it_did_in_one_line_of_json(self):
        c, s = self.home()
        out = self.root / 'cmd.tar'
        code, r = self.run_cmd('pack', '--config', c, '--state', s, '--out', out)
        self.assertEqual((code, r['ok']), (0, True))
        c2, s2 = self.folders('new')
        code, st = self.run_cmd('stage', '--config', c2, '--state', s2, '--in', out, '--sha256', r['sha256'])
        self.assertEqual((code, st['sha256']), (0, r['sha256']))
        code, st = self.run_cmd('status', '--config', c2, '--state', s2)
        self.assertEqual((st['staged'], st['held']), (r['sha256'], False))
        code, st = self.run_cmd('swap', '--config', c2, '--state', s2)
        self.assertEqual(code, 0)
        self.assertTrue((s2 / 'nodes.json').exists())
        code, st = self.run_cmd('swap', '--config', c2, '--state', s2)
        self.assertEqual((code, st['ok'], st['kind']), (1, False, 'nothing'))
        cut = self.root / 'cut.tar'
        cut.write_bytes(out.read_bytes()[:900])
        code, st = self.run_cmd('stage', '--config', c2, '--state', s2, '--in', cut)
        self.assertEqual((code, st['ok'], st['kind']), (1, False, 'damaged'))
        self.assertNotIn('!', st['error'])

    def test_the_two_folders_must_be_named(self):
        r = subprocess.run([sys.executable, '-I', homestate.__file__, 'pack', '--out', str(self.root / 'x.tar')],
                           capture_output=True, text=True)
        self.assertNotEqual(r.returncode, 0)
        self.assertFalse((self.root / 'x.tar').exists())


class ServerSideTest(Base):
    def test_the_server_holds_its_folder_and_stays_down_on_a_computer_that_stepped_down(self):
        c, s = self.folders('srv')
        self.assertIsNone(homestate.stepped_down(c))
        (c / 'home-stepdown.json').write_text(json.dumps({'v': 1, 'why': 'moved', 'at': 5}))
        self.assertEqual(homestate.stepped_down(c)['why'], 'moved')
        (c / 'home-stepdown.json').write_text('')
        self.assertEqual(homestate.stepped_down(c)['why'], 'unreadable', 'a marker that cannot be read still counts')
        self.assertIn('was the home computer', homestate.STEPPED_DOWN)

    def test_settings_that_came_without_addresses_get_this_computers_own(self):
        import server
        c, s = self.folders('srv')
        (c / 'server.json').write_text(json.dumps({'keys': 'both', 'peers': ['203.0.113.0/24'], 'allow': ['private']}))
        cfg = server.ServerConfig(c, s, port=8130, local=lambda: ['10.0.0.9', 'new'])
        self.assertEqual(cfg.bind, ['0.0.0.0'])
        self.assertIn('10.0.0.9', cfg.hosts)
        self.assertEqual(cfg.origins, ['http://10.0.0.9:8130', 'http://new:8130'])
        d = json.loads((c / 'server.json').read_text())
        self.assertEqual((d['keys'], d['peers'], d['allow'], d['hosts']), ('both', ['203.0.113.0/24'], ['private'], ['10.0.0.9', 'new']))
        # a file that names its addresses, even as none, is left as it is
        (c / 'server.json').write_text(json.dumps({'bind': [], 'hosts': [], 'origins': []}))
        cfg = server.ServerConfig(c, s, port=8130, local=lambda: ['10.0.0.9'])
        self.assertEqual(cfg.origins, [])


if __name__ == '__main__':
    unittest.main()
