"""A file that cannot be read is not an empty one (server/durable.py load).

Read as empty, a damaged settings file had its defaults written back over it: opt-in settings turned on, a
restricted server opened up, kept chats stopped. Now the damaged file is put aside, the last good copy is used when
there is one, and a store with no good copy is not written over until the user saves it again."""
import json, os, sys, tempfile, unittest
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'server'))
import durable  # noqa: E402

DAMAGED = (b'', b'\x00' * 439, b'{"a": 1', b'[1, 2]', b'"text"', b'\xff\xfe not text', b'  \n')


def names(d):
    return sorted(os.listdir(d))


def told(p):
    """What the apps are told about this store (durable.status), or None."""
    m = durable._marks.get(str(p))
    return m and [s for s in durable.status() if s['file'] == Path(p).name and s['at'] == m['at']][0]


class LoadTest(unittest.TestCase):
    def setUp(self):
        self.d = Path(tempfile.mkdtemp())
        self.p = self.d / 'a.json'

    def test_a_file_that_is_not_there_is_absent(self):
        self.assertEqual(durable.load(self.p, {'x': 1}), ({'x': 1}, 'absent'))
        self.assertEqual(names(self.d), [])
        durable.put(self.p, '{"k": 2}')  # nothing stands in the way of the first write
        self.assertEqual(durable.load(self.p, {}), ({'k': 2}, 'ok'))
        self.assertIsNone(told(self.p), 'nothing to tell the apps')

    def test_a_damaged_file_with_no_copy_is_broken_and_kept_aside(self):
        for i, raw in enumerate(DAMAGED):
            p = self.d / f's{i}.json'
            p.write_bytes(raw)
            self.assertEqual(durable.load(p, {}), ({}, 'broken'), raw)
            kept = [n for n in names(self.d) if n.startswith(p.name + '.bad-')]
            self.assertEqual(len(kept), 1, raw)
            self.assertEqual((self.d / kept[0]).read_bytes(), raw, 'the damaged file is kept as it was')
            self.assertFalse(p.exists())
            self.assertEqual(durable.load(p, {}), ({}, 'broken'), 'and it stays broken, not absent')
            self.assertTrue(durable.broken(p))

    def test_the_shape_is_part_of_being_readable(self):
        self.p.write_text('{"a": 1}')
        self.assertEqual(durable.load(self.p, [], list)[1], 'broken')
        q = self.d / 'b.json'
        q.write_text('{"a": 1}')
        self.assertEqual(durable.load(q, None, (dict, list), check=lambda d: 'machines' in d)[1], 'broken')
        q.write_text('{"machines": []}')
        self.assertEqual(durable.load(q, None, (dict, list), check=lambda d: 'machines' in d), ({'machines': []}, 'ok'))

    def test_put_keeps_the_file_from_before_and_load_goes_back_to_it(self):
        durable.put(self.p, '{"v": 1}', keep_bak=True)
        self.assertEqual(names(self.d), ['a.json'], 'nothing to keep on the first write')
        durable.put(self.p, '{"v": 2}', keep_bak=True)
        self.assertEqual(json.loads((self.d / 'a.json.bak').read_text()), {'v': 1})
        self.p.write_bytes(b'\x00' * 8)
        self.assertEqual(durable.load(self.p, {}), ({'v': 1}, 'recovered'))
        self.assertEqual(durable.load(self.p, {}), ({'v': 1}, 'ok'), 'the good copy is the file again')
        self.assertFalse(durable.broken(self.p))
        row = told(self.p)
        self.assertEqual(row['status'], 'recovered')
        self.assertTrue(row['kept'].startswith('a.json.bad-'))
        durable.put(self.p, '{"v": 3}', keep_bak=True)  # a recovered store is written as usual
        self.assertEqual(durable.load(self.p, {}), ({'v': 3}, 'ok'))

    def test_a_damaged_file_is_never_kept_as_the_good_copy(self):
        durable.put(self.p, '{"v": 1}', keep_bak=True)
        durable.put(self.p, '{"v": 2}', keep_bak=True)
        self.p.write_bytes(b'')  # a power cut, and then a writer that never read it
        durable.put(self.p, '{"v": 3}', keep_bak=True)
        self.assertEqual(json.loads((self.d / 'a.json.bak').read_text()), {'v': 1})

    def test_the_copy_is_made_where_hard_links_are_not(self):
        durable.put(self.p, '{"v": 1}', keep_bak=True, mode=0o600)
        with mock.patch.object(os, 'link', side_effect=OSError('no links here')):
            durable.put(self.p, '{"v": 2}', keep_bak=True, mode=0o600)
        bak = self.d / 'a.json.bak'
        self.assertEqual(json.loads(bak.read_text()), {'v': 1})
        if os.name != 'nt':
            self.assertEqual(os.stat(bak).st_mode & 0o777, 0o600)

    def test_a_broken_store_is_not_written_over_until_someone_means_it(self):
        self.p.write_bytes(b'{"cut sho')
        self.assertEqual(durable.load(self.p, {})[1], 'broken')
        with self.assertRaises(durable.Broken):
            durable.put(self.p, '{"defaults": true}', keep_bak=True)
        self.assertFalse(self.p.exists())
        self.assertEqual([n for n in names(self.d) if n.endswith('.tmp')], [])
        self.assertTrue(issubclass(durable.Broken, OSError), 'a writer that says "could not save" says it for this too')
        durable.put(self.p, '{"chosen": true}', keep_bak=True, force=True)
        self.assertEqual(durable.load(self.p, {}), ({'chosen': True}, 'ok'))
        self.assertFalse(durable.broken(self.p))
        row = told(self.p)
        self.assertEqual((row['status'], row['replaced']), ('broken', True), 'still told to the apps until a restart')
        durable.put(self.p, '{"chosen": false}', keep_bak=True)  # and from then on it is a store like any other

    def test_a_readable_file_put_back_by_hand_ends_it(self):
        self.p.write_bytes(b'')
        self.assertEqual(durable.load(self.p, {})[1], 'broken')
        self.p.write_text('{"restored": 1}')
        self.assertEqual(durable.load(self.p, {}), ({'restored': 1}, 'ok'))
        self.assertFalse(durable.broken(self.p))
        durable.put(self.p, '{"restored": 2}')

    def test_a_restart_does_not_forget_a_broken_store(self):
        self.p.write_bytes(b'')
        durable.load(self.p, {})
        with mock.patch.dict(durable._marks, clear=True), mock.patch.object(durable, '_looked', set()):  # a new process
            self.assertEqual(durable.load(self.p, {}), ({}, 'broken'))
            with self.assertRaises(durable.Broken):
                durable.put(self.p, '{}')

    def test_only_the_newest_three_damaged_copies_are_kept(self):
        stamps = iter(f'2026112{i}-120000' for i in range(1, 7))
        with mock.patch.object(durable.time, 'strftime', lambda fmt: next(stamps)):
            for _ in range(5):
                self.p.write_bytes(b'')
                durable.load(self.p, {})
        self.assertEqual(names(self.d), ['a.json.bad-20261123-120000', 'a.json.bad-20261124-120000', 'a.json.bad-20261125-120000'])

    def test_a_file_that_cannot_be_opened_is_broken_and_left_where_it_is(self):
        self.p.write_text('{"v": 1}')
        with mock.patch.object(Path, 'read_bytes', side_effect=PermissionError(13, 'no')):
            self.assertEqual(durable.load(self.p, {}), ({}, 'broken'))
        self.assertEqual(names(self.d), ['a.json'])
        with self.assertRaises(durable.Broken):
            durable.put(self.p, '{}')
        self.assertEqual(durable.load(self.p, {}), ({'v': 1}, 'ok'), 'readable again: the mark goes')

    def test_put_aside_also_for_files_that_are_not_json(self):
        k = self.d / 'term.key'
        k.write_bytes(b'short')
        kept = durable.aside(k)
        self.assertTrue(kept.startswith('term.key.bad-'))
        self.assertEqual((self.d / kept).read_bytes(), b'short')
        self.assertIsNone(durable.aside(k), 'nothing there to move')


class FreshTest(unittest.TestCase):
    """put(fresh=True): the copy is of what was just written, so it never holds what was taken out."""

    def setUp(self):
        self.d = Path(tempfile.mkdtemp())
        self.p = self.d / 'a.json'
        self.bak = self.d / 'a.json.bak'

    def test_the_copy_is_the_new_file_and_a_file_of_its_own(self):
        durable.put(self.p, '{"a": 1, "gone": 2}', fresh=True)
        durable.put(self.p, '{"a": 1}', fresh=True)
        self.assertEqual(self.bak.read_text(encoding='utf-8'), '{"a": 1}')
        self.assertFalse(os.path.samefile(self.p, self.bak))
        self.p.write_bytes(b'')
        self.assertEqual(durable.load(self.p, {}), ({'a': 1}, 'recovered'))

    def test_when_the_copy_cannot_be_made_the_old_one_does_not_stay(self):
        durable.put(self.p, '{"a": 1, "gone": 2}', fresh=True)
        with mock.patch.object(durable, 'backup', return_value=False):
            durable.put(self.p, '{"a": 1}', fresh=True)
        self.assertFalse(self.bak.exists())


def unreadable(p, times=None):
    """Path.read_bytes that fails for `p` (the first `times` reads, or always) and works for everything else."""
    real, left = Path.read_bytes, [times]

    def read(self):
        if str(self) == str(p) and (left[0] is None or left[0] > 0):
            left[0] = None if left[0] is None else left[0] - 1
            raise PermissionError(13, 'in use')
        return real(self)
    return mock.patch.object(Path, 'read_bytes', read)


class UnreadableTest(unittest.TestCase):
    """The file is there and cannot be opened (no permission, too many open files, held by another program)."""

    def setUp(self):
        self.d = Path(tempfile.mkdtemp())
        self.p = self.d / 'a.json'
        self.p.write_text('{"v": 2}', encoding='utf-8')
        waits = mock.patch.object(durable, 'READ_RETRY', (0, 0))
        waits.start()
        self.addCleanup(waits.stop)

    def test_a_read_that_fails_once_is_tried_again(self):
        with unreadable(self.p, times=2):
            self.assertEqual(durable.load(self.p, {}), ({'v': 2}, 'ok'))
        self.assertIsNone(told(self.p))
        self.assertEqual(names(self.d), ['a.json'])

    def test_the_last_good_copy_is_used_and_nothing_is_moved_or_written(self):
        (self.d / 'a.json.bak').write_text('{"v": 1}', encoding='utf-8')
        with unreadable(self.p):
            self.assertEqual(durable.load(self.p, {}), ({'v': 1}, 'recovered'))
        self.assertEqual(names(self.d), ['a.json', 'a.json.bak'])
        self.assertEqual(self.p.read_text(encoding='utf-8'), '{"v": 2}', 'the file itself is left alone')
        self.assertEqual((told(self.p)['status'], told(self.p)['kept']), ('recovered', None))
        self.assertEqual(durable.load(self.p, {}), ({'v': 2}, 'ok'))
        self.assertIsNone(told(self.p), 'readable again, and nothing was damaged: nothing to tell')

    def test_with_no_good_copy_it_is_broken(self):
        for bak in (None, b'{"v": '):
            if bak is not None:
                (self.d / 'a.json.bak').write_bytes(bak)
            with unreadable(self.p):
                self.assertEqual(durable.load(self.p, {}), ({}, 'broken'))
            self.assertEqual(self.p.read_text(encoding='utf-8'), '{"v": 2}')
            self.assertEqual([n for n in names(self.d) if '.bad-' in n], [])

    def test_it_waits_only_the_first_time(self):
        with unreadable(self.p), mock.patch.object(durable.time, 'sleep') as sleep:
            durable.load(self.p, {})
            self.assertEqual(sleep.call_count, 2)
            durable.load(self.p, {})
            self.assertEqual(sleep.call_count, 2, 'a store that stays unreadable does not slow every read')


class KeepTest(unittest.TestCase):
    """keep=True: stores people edit by hand and the server seldom writes."""

    def setUp(self):
        self.d = Path(tempfile.mkdtemp())
        self.p = self.d / 'a.json'
        self.bak = self.d / 'a.json.bak'

    def test_the_copy_follows_every_good_load(self):
        self.p.write_text('{"v": 1}', encoding='utf-8')
        durable.load(self.p, {}, keep=True)
        self.assertEqual(json.loads(self.bak.read_bytes()), {'v': 1})
        self.p.write_text('{"v": 2}', encoding='utf-8')  # edited by hand
        durable.load(self.p, {}, keep=True)
        self.assertEqual(json.loads(self.bak.read_bytes()), {'v': 2})
        self.p.write_text('{"v": 2', encoding='utf-8')
        self.assertEqual(durable.load(self.p, {}, keep=True), ({'v': 2}, 'recovered'))

    def test_a_copy_that_is_already_the_same_is_not_written_again(self):
        self.p.write_text('{"v": 1}', encoding='utf-8')
        durable.load(self.p, {}, keep=True)
        with mock.patch.object(durable, 'backup', side_effect=AssertionError('written again')):
            durable.load(self.p, {}, keep=True)

    def test_without_keep_a_load_leaves_the_copy_alone(self):
        self.p.write_text('{"v": 1}', encoding='utf-8')
        durable.load(self.p, {})
        self.assertFalse(self.bak.exists())


if __name__ == '__main__':
    unittest.main()
