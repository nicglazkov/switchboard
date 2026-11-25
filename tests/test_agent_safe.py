"""The helper's files that are read, changed and written back: one that cannot be read is not taken for an empty one.

A power cut left a state file full of zeros. Read as "nothing", the next rename wrote one name over the whole list.
The damaged file is now kept as <name>.bad-<time>, and the copy from before its last write (<name>.bak) is used."""
import contextlib, json, os, stat, threading, unittest
from pathlib import Path
from unittest import mock
from tests.test_rpc import RpcBase, SID_A, SID_B

ZEROS = b'\x00' * 439
SID_C = 'cccccccc-1111-2222-3333-444444444444'


def kept(p):
    return sorted(n for n in os.listdir(p.parent) if n.startswith(p.name + '.bad-'))


def read(p):
    return json.loads(p.read_text(encoding='utf-8'))


@contextlib.contextmanager
def shut(*ps):
    """These files are there and will not open (no permission, held by another program). Everything else reads."""
    ps, rb, rt = [str(p) for p in ps], Path.read_bytes, Path.read_text

    def read_bytes(self):
        if str(self) in ps:
            raise PermissionError(13, 'in use')
        return rb(self)

    def read_text(self, *a, **kw):
        if str(self) in ps:
            raise PermissionError(13, 'in use')
        return rt(self, *a, **kw)
    with mock.patch.object(Path, 'read_bytes', read_bytes), mock.patch.object(Path, 'read_text', read_text):
        yield


class SafeReadTest(RpcBase):
    def setUp(self):
        super().setUp()
        self.p = self.sb.paths()['state'] / 'x.json'

    def test_a_file_that_is_not_there_is_empty_and_leaves_nothing_behind(self):
        self.assertEqual(self.sb._jsafe(self.p), {})
        self.assertEqual(os.listdir(self.p.parent), [])

    def test_a_good_file_is_read_as_it_is(self):
        self.sb._jput(self.p, {'a': 1})
        self.assertEqual(self.sb._jsafe(self.p), {'a': 1})
        self.assertEqual(kept(self.p), [])

    def test_a_damaged_file_goes_back_to_the_copy_from_before_its_last_write(self):
        self.sb._jput(self.p, {'a': 1}, keep=True)
        self.sb._jput(self.p, {'a': 1, 'b': 2}, keep=True)
        self.assertEqual(read(self.p.with_name('x.json.bak')), {'a': 1})
        for bad in (ZEROS, b'', b'{"a": 1, "b"', b'[1, 2]', b'"text"', b'\xff\xfe'):
            self.p.write_bytes(bad)
            self.assertEqual(self.sb._jsafe(self.p), {'a': 1}, bad)
        self.assertLessEqual(len(kept(self.p)), self.sb.BAD_KEEP)
        self.assertIn('x.json could not be read', self.sb.paths()['log'].read_text(encoding='utf-8'))

    def test_with_no_copy_it_is_empty_and_the_damaged_file_is_kept(self):
        self.p.write_bytes(ZEROS)
        self.assertEqual(self.sb._jsafe(self.p), {})
        self.assertEqual(len(kept(self.p)), 1)
        self.assertEqual((self.p.parent / kept(self.p)[0]).read_bytes(), ZEROS)
        self.assertFalse(self.p.exists())

    def test_only_the_newest_damaged_copies_are_kept(self):
        for k in range(6):
            (self.p.parent / f'x.json.bad-20261101-00000{k}').write_bytes(b'old')
        self.p.write_bytes(ZEROS)
        self.sb._jsafe(self.p)
        self.assertEqual(len(kept(self.p)), self.sb.BAD_KEEP)
        self.assertNotIn('x.json.bad-20261101-000000', kept(self.p))

    def test_a_damaged_file_is_never_what_the_copy_is_made_from(self):
        self.sb._jput(self.p, {'a': 1}, keep=True)
        self.sb._jput(self.p, {'a': 2}, keep=True)
        self.p.write_bytes(ZEROS)
        self.sb._jput(self.p, {'a': 3}, keep=True)
        self.assertEqual(read(self.p.with_name('x.json.bak')), {'a': 1})

    def test_a_check_says_what_the_file_must_hold(self):
        self.sb._jput(self.p, {'tools': {'t': 1}}, keep=True)
        self.sb._jput(self.p, {'tools': {'t': 1, 'u': 2}}, keep=True)
        self.p.write_text('{"tools": 5}', encoding='utf-8')
        self.assertEqual(self.sb._jsafe(self.p, lambda d: isinstance(d.get('tools'), dict)), {'tools': {'t': 1}})

    def test_a_file_that_will_not_open_is_not_damage(self):
        self.sb._jput(self.p, {'a': 1}, keep=True)
        self.sb._jput(self.p, {'a': 1, 'b': 2}, keep=True)
        before = sorted(os.listdir(self.p.parent))
        with shut(self.p):
            self.assertEqual(self.sb._jsafe(self.p), {})
            with self.assertRaises(OSError):
                self.sb._jput(self.p, {'c': 3}, keep=True)  # what a caller made from that {} is not written over it
        with self.assertRaises(OSError):
            self.sb._jput(self.p, {'c': 3}, keep=True)  # nor a moment later, when it would open again
        self.assertEqual(sorted(os.listdir(self.p.parent)), before, 'nothing put aside, nothing left behind')
        self.assertEqual(read(self.p), {'a': 1, 'b': 2})
        self.assertEqual(self.sb._jsafe(self.p), {'a': 1, 'b': 2})
        self.sb._jput(self.p, {'a': 1, 'b': 2, 'c': 3}, keep=True)  # read again, so it may be written again
        self.assertEqual(read(self.p), {'a': 1, 'b': 2, 'c': 3})

    def test_a_dict_is_what_the_readers_get(self):
        for bad in (b'[1]', b'"x"', b'7', ZEROS):
            self.p.write_bytes(bad)
            self.assertEqual(self.sb._jdict(self.p), {}, bad)
        self.assertEqual(kept(self.p), [], 'a reader moves nothing')


class NamesTest(RpcBase):
    def names(self):
        return self.sb._names_files()[0]

    def name(self, sid, name):
        tp = self.cdir / 'projects' / 'p1' / f'{sid}.jsonl'
        if not tp.exists():
            self.transcript(sid, str(self.root / 'proj'), 1_800_000_000.0)
        return self.sb.set_given_name(sid, name, tp, running=True)

    def test_a_rename_after_the_list_was_damaged_keeps_the_other_names(self):
        self.name(SID_A, 'first')
        self.name(SID_B, 'second')
        self.names().write_bytes(ZEROS)
        self.name(SID_C, 'third')
        got = read(self.names())
        self.assertEqual((got.get(SID_A), got.get(SID_C)), ('first', 'third'))
        self.assertEqual(len(kept(self.names())), 1)

    def test_the_names_waiting_for_their_chat_to_stop_are_kept_too(self):
        pp = self.sb._names_files()[1]
        self.name(SID_A, 'first')
        self.name(SID_B, 'second')
        pp.write_bytes(ZEROS)
        self.sb.carry_name(SID_A, SID_C)
        self.assertEqual(read(pp).get(SID_A), 'first')

    def test_forgetting_one_name_does_not_forget_the_rest(self):
        self.name(SID_A, 'first')
        self.name(SID_B, 'second')
        self.name(SID_C, 'third')
        self.names().write_bytes(ZEROS)
        self.sb.drop_given_name(SID_C)
        self.assertEqual(read(self.names()), {SID_A: 'first', SID_B: 'second'})

    def test_a_rename_while_the_list_will_not_open_fails_and_loses_nothing(self):
        self.name(SID_A, 'first')
        self.name(SID_B, 'second')
        np, pp, wp = self.sb._names_files()
        for one in (np, pp, wp):
            before = {p: p.read_bytes() for p in (np, pp, wp)}
            with shut(one), self.assertRaises(OSError):
                self.name(SID_C, 'third')
            self.assertEqual({p: p.read_bytes() for p in (np, pp, wp)}, before, one.name)
            self.assertEqual(kept(one), [])
        self.assertIs(self.name(SID_C, 'third'), False)
        self.assertEqual(read(np), {SID_A: 'first', SID_B: 'second', SID_C: 'third'})

    def test_housekeeping_while_the_list_will_not_open_changes_nothing(self):
        self.name(SID_A, 'first')
        self.name(SID_B, 'second')
        np, pp, wp = self.sb._names_files()
        before = {p: p.read_bytes() for p in (np, pp, wp)}
        with shut(np):
            self.sb.drop_given_name(SID_B)
        with shut(pp):
            self.sb.carry_name(SID_A, SID_C)
            self.sb.flush_names([])
        self.assertEqual({p: p.read_bytes() for p in (np, pp, wp)}, before)
        self.assertEqual(kept(np) + kept(pp), [])

    def test_a_list_of_the_wrong_kind_does_not_stop_a_rename(self):
        self.names().write_text('[]', encoding='utf-8')
        self.assertIs(self.name(SID_A, 'first'), False)
        self.assertEqual(read(self.names()), {SID_A: 'first'})
        self.assertIsNone(self.sb.given_name(SID_B))


class MarksTest(RpcBase):
    def test_a_changed_file_keeps_what_it_held_before_the_damage(self):
        p = self.sb.paths()['state'] / 'dismissed.json'
        self.sb.locked_update(p, lambda d: dict(d, a=1))
        self.sb.locked_update(p, lambda d: dict(d, b=2))
        p.write_bytes(ZEROS)
        self.assertEqual(self.sb.locked_update(p, lambda d: dict(d, c=3)), {'a': 1, 'c': 3})
        self.assertEqual(len(kept(p)), 1)

    def test_a_file_that_will_not_open_is_left_alone(self):
        p = self.sb.paths()['state'] / 'dismissed.json'
        self.sb.locked_update(p, lambda d: dict(d, a=1))
        asked = []
        with shut(p):
            self.assertIsNone(self.sb.locked_update(p, lambda d: asked.append(d) or dict(d, b=2)))
        self.assertEqual((asked, read(p), kept(p)), ([], {'a': 1}, []))
        self.assertEqual(self.sb.locked_update(p, lambda d: dict(d, c=3)), {'a': 1, 'c': 3})


class LibraryIndexTest(RpcBase):
    def test_a_damaged_index_does_not_forget_the_installed_tools(self):
        p = self.sb.lib_root() / 'index.json'
        self.sb._lib_save({'tools': {'one': {'version': '1'}}})
        self.sb._lib_save({'tools': {'one': {'version': '1'}, 'two': {'version': '1'}}})
        p.write_bytes(ZEROS)
        self.assertEqual(self.sb.lib_index(), {'tools': {'one': {'version': '1'}}})
        self.assertEqual(len(kept(p)), 1)
        p.write_text('{"tools": []}', encoding='utf-8')
        self.assertEqual(self.sb.lib_index(), {'tools': {'one': {'version': '1'}}})

    def test_an_index_that_will_not_open_is_not_written_over(self):
        p = self.sb.lib_root() / 'index.json'
        full = {'tools': {'one': {'version': '1'}, 'two': {'version': '1'}}}
        self.sb._lib_save(full)
        with shut(p):
            idx = self.sb.lib_index()
            self.assertEqual(idx, {'tools': {}})
            idx['tools']['three'] = {'version': '1'}
            with self.assertRaises(OSError):
                self.sb._lib_save(idx)  # an install made from "no tools"
        self.assertEqual((read(p), kept(p)), (full, []))
        idx = self.sb.lib_index()
        self.assertEqual(idx, full)
        idx['tools']['three'] = {'version': '1'}
        self.sb._lib_save(idx)
        self.assertEqual(sorted(read(p)['tools']), ['one', 'three', 'two'])

    def test_no_index_is_no_tools(self):
        self.assertEqual(self.sb.lib_index(), {'tools': {}})
        self.assertFalse((self.sb.lib_root() / 'index.json.bak').exists())


@unittest.skipIf(os.name == 'nt', 'file modes')
class SecretsFileTest(RpcBase):
    def setUp(self):
        super().setUp()
        self.s = self.sb.FileSecrets(self.tmp / 'home' / 'secrets.json')

    def test_a_damaged_file_does_not_cost_the_other_secrets(self):
        self.s.set('tool', 'A_KEY', 'one')
        self.s.set('tool', 'B_KEY', 'two')
        self.s.path.write_bytes(ZEROS)
        self.s.set('tool', 'C_KEY', 'three')
        self.assertEqual((self.s.get('tool', 'A_KEY'), self.s.get('tool', 'C_KEY')), ('one', 'three'))
        for n in os.listdir(self.s.path.parent):
            self.assertEqual(stat.S_IMODE(os.stat(self.s.path.parent / n).st_mode), 0o600, n)

    def test_removing_one_after_damage_keeps_the_rest(self):
        self.s.set('tool', 'A_KEY', 'one')
        self.s.set('tool', 'B_KEY', 'two')
        self.s.set('tool', 'C_KEY', 'three')
        self.s.path.write_bytes(ZEROS)
        self.assertTrue(self.s.rm('tool', 'B_KEY'))
        self.assertEqual((self.s.get('tool', 'A_KEY'), self.s.get('tool', 'B_KEY')), ('one', None))

    def test_a_removed_secret_is_not_left_in_the_copy(self):
        self.s.set('tool', 'A_KEY', 'one')
        self.s.set('tool', 'B_KEY', 'hunter-two')
        self.assertTrue(self.s.rm('tool', 'B_KEY'))
        bak = self.s.path.with_name('secrets.json.bak')
        self.assertNotIn('hunter-two', bak.read_text(encoding='utf-8'))
        self.assertEqual(stat.S_IMODE(os.stat(bak).st_mode), 0o600)
        self.s.path.write_bytes(ZEROS)
        self.s.set('tool', 'C_KEY', 'three')
        self.assertEqual((self.s.get('tool', 'A_KEY'), self.s.get('tool', 'B_KEY')), ('one', None))

    def test_a_replaced_secret_is_not_left_in_the_copy(self):
        self.s.set('tool', 'A_KEY', 'leaked-one')
        self.s.set('tool', 'A_KEY', 'new-one')
        self.assertNotIn('leaked-one', self.s.path.with_name('secrets.json.bak').read_text(encoding='utf-8'))

    def test_the_copy_is_a_file_of_its_own(self):
        self.s.set('tool', 'A_KEY', 'one')
        bak = self.s.path.with_name('secrets.json.bak')
        self.assertFalse(os.path.samefile(bak, self.s.path), 'not a link: damage to one must not reach the other')

    def test_a_file_that_will_not_open_is_not_written_over(self):
        self.s.set('tool', 'A_KEY', 'one')
        before = sorted(os.listdir(self.s.path.parent))
        with shut(self.s.path):
            with self.assertRaises(OSError):
                self.s.set('tool', 'B_KEY', 'two')
            with self.assertRaises(OSError):
                self.s.rm('tool', 'A_KEY')
        self.assertEqual(sorted(os.listdir(self.s.path.parent)), before)
        self.assertEqual(self.s.get('tool', 'A_KEY'), 'one')
        self.s.set('tool', 'B_KEY', 'two')
        self.assertEqual((self.s.get('tool', 'A_KEY'), self.s.get('tool', 'B_KEY')), ('one', 'two'))

    def test_two_saves_at_once_both_land(self):
        go, errs = threading.Barrier(8), []

        def one(k):
            try:
                go.wait()
                self.s.set('tool', f'KEY_{k}', f'v{k}')
            except Exception as e:  # noqa: BLE001
                errs.append(e)
        ts = [threading.Thread(target=one, args=(k,)) for k in range(8)]
        for t in ts:
            t.start()
        for t in ts:
            t.join()
        self.assertEqual(errs, [])
        self.assertEqual([self.s.get('tool', f'KEY_{k}') for k in range(8)], [f'v{k}' for k in range(8)])
        self.assertEqual([n for n in os.listdir(self.s.path.parent) if n.endswith('.tmp')], [])


class WrongKindTest(RpcBase):
    def test_state_files_that_hold_a_list_do_not_break_the_list_of_chats(self):
        st = self.sb.paths()['state']
        for n in ('index.json', 'dismissed.json', 'replaced.json', 'names.json', 'names_pending.json', 'flags.json',
                  'condensed.json', self.sb.TTL_FILE):
            (st / n).write_text('[1, 2]', encoding='utf-8')
        self.assertEqual(self.sb.launch_flags([], 1_800_000_000.0), {})
        self.transcript(SID_A, str(self.root / 'proj'), 1_800_000_000.0)
        r = self.sb.rpc({'verb': 'list'}, 1_800_000_000.0, agents=[])
        self.assertEqual((r['ok'], len(r['sessions'])), (True, 1))


if __name__ == '__main__':
    unittest.main()
