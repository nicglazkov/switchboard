"""State that survives a power cut: contents reach the disk before the rename, and no temp file is left for good.

A file on a Windows machine came back from a power cut as 439 zero bytes: the rename had landed, its contents had
not."""
import json, os, sys, tempfile, time, unittest
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'server'))
import durable, features, budget, library  # noqa: E402
from tests.test_rpc import RpcBase, NOW  # noqa: E402


class Order:
    """Records fsync and replace as they happen."""

    def __init__(self):
        self.seen = []
        self.fsync, self.replace = os.fsync, os.replace

    def __enter__(self):
        def fsync(fd):
            self.seen.append('fsync')
            return self.fsync(fd)

        def replace(a, b):
            self.seen.append('replace')
            return self.replace(a, b)
        self.p = [mock.patch.object(os, 'fsync', fsync), mock.patch.object(os, 'replace', replace)]
        for p in self.p:
            p.start()
        return self.seen

    def __exit__(self, *a):
        for p in self.p:
            p.stop()


class ServerDurableTest(unittest.TestCase):
    def setUp(self):
        self.d = Path(tempfile.mkdtemp())

    def test_contents_reach_the_disk_before_the_rename(self):
        with Order() as seen:
            durable.put(self.d / 'a.json', '{"a": 1}')
        self.assertEqual(seen, ['fsync', 'replace'])
        self.assertEqual(json.loads((self.d / 'a.json').read_text()), {'a': 1})
        self.assertEqual(os.listdir(self.d), ['a.json'])

    def test_every_store_saves_that_way(self):
        for save in (features.jsave, library.jsave, budget._jsave):
            with Order() as seen:
                save(self.d / 'x.json', {'k': [1, 2]})
            self.assertEqual(seen, ['fsync', 'replace'], save)
            self.assertEqual(json.loads((self.d / 'x.json').read_text()), {'k': [1, 2]})

    def test_bytes_and_permissions(self):
        durable.put(self.d / 'b.bin', b'\x00\x01', mode=0o600)
        self.assertEqual((self.d / 'b.bin').read_bytes(), b'\x00\x01')
        if os.name != 'nt':
            self.assertEqual(os.stat(self.d / 'b.bin').st_mode & 0o777, 0o600)

    def test_a_failed_write_keeps_the_old_file_and_leaves_no_temp_file(self):
        (self.d / 'a.json').write_text('old')

        def no(a, b):
            raise OSError('disk says no')
        with mock.patch.object(os, 'replace', no), self.assertRaises(OSError):
            durable.put(self.d / 'a.json', 'new')
        self.assertEqual((self.d / 'a.json').read_text(), 'old')
        self.assertEqual(os.listdir(self.d), ['a.json'])

    def test_sweep_takes_only_old_temp_files(self):
        old, new, real = self.d / 'jobs.123.tmp', self.d / 'jobs.456.tmp', self.d / 'jobs.json'
        other = self.d / 'settings.json.tmp-99'
        for f in (old, new, real, other):
            f.write_text('x')
        for f in (old, other, real):
            os.utime(f, (time.time() - 7200,) * 2)
        self.assertEqual(durable.sweep(self.d), 2)
        self.assertEqual(sorted(os.listdir(self.d)), ['jobs.456.tmp', 'jobs.json'])
        self.assertEqual(durable.sweep(self.d / 'not-there'), 0)


class QuietDropTest(unittest.TestCase):
    def test_a_client_that_went_away_prints_no_traceback(self):
        import contextlib, io, server
        s = server.Server.__new__(server.Server)
        for exc, loud in ((ConnectionResetError(54, 'reset'), False), (BrokenPipeError(), False), (KeyError('real'), True)):
            err = io.StringIO()
            with contextlib.redirect_stderr(err):
                try:
                    raise exc
                except Exception:
                    s.handle_error(None, ('127.0.0.1', 1))
            self.assertEqual('Traceback' in err.getvalue(), loud, exc)


class AgentDurableTest(RpcBase):
    def test_contents_reach_the_disk_before_the_rename(self):
        p = self.sb.paths()['state'] / 'x.json'
        with Order() as seen:
            self.sb._jput(p, {'a': 1})
            self.sb._jput_text(p.with_name('y.md'), 'héllo')
        self.assertEqual(seen, ['fsync', 'replace'] * 2)
        self.assertEqual(self.sb._jget(p), {'a': 1})
        self.assertEqual(p.with_name('y.md').read_text(encoding='utf-8'), 'héllo')
        self.assertEqual([n for n in os.listdir(p.parent) if n.endswith('.tmp')], [])

    def test_a_file_of_zeros_reads_as_nothing(self):
        p = self.sb.paths()['state'] / 'z.json'
        p.write_bytes(b'\x00' * 439)
        self.assertIsNone(self.sb._jget(p))

    def test_sweep_takes_only_old_temp_files(self):
        st = self.sb.paths()['state']
        old, new, real = st / 'a.json.1.2.tmp', st / 'b.json.3.4.tmp', st / 'a.json'
        for f in (old, new, real):
            f.write_text('x')
        for f in (old, real):
            os.utime(f, (NOW - 7200,) * 2)
        os.utime(new, (NOW - 60,) * 2)
        self.assertEqual(self.sb.sweep_tmp(NOW), 1)
        self.assertEqual((old.exists(), new.exists(), real.exists()), (False, True, True))


if __name__ == '__main__':
    unittest.main()
