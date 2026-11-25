"""What the server does when one of its own files cannot be read (durable.load, applied to every store).

Each test damages a file the way a power cut does (zeros, or cut short) and checks that the safe choice is made:
nothing the user turned off comes back on, nothing the user restricted opens up, and the file is not written over."""
import contextlib, io, json, os, sys, time, unittest
from pathlib import Path
from tempfile import mkdtemp
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'server'))
import budget, durable, features, fleet, library, options, selfupdate, server, serverctl, term  # noqa: E402
from tests import test_library, test_reaper, test_safe_load, test_server_jobs  # noqa: E402
from tests.test_reaper import NOW, cold, sid  # noqa: E402

ZEROS = b'\x00' * 439
MACH = [{'name': 'studio', 'label': 'Studio', 'target': 'me@x', 'os': 'mac'}]


def write(p, d):
    p.parent.mkdir(parents=True, exist_ok=True)
    p.write_text(d if isinstance(d, str) else json.dumps(d))


def kept(p):
    """The copies of p that were put aside."""
    return sorted(n for n in os.listdir(p.parent) if n.startswith(p.name + '.bad-'))


def told(rows, p):
    """What the apps are told about the store at p, from a list like durable.status()."""
    m = durable._marks.get(str(p)) or {}
    return [(r['file'], r['status'], r['replaced']) for r in rows if r['file'] == p.name and r['at'] == m.get('at')]


class Quiet:
    def __call__(self, target, req, timeout):
        return {'ok': True, 'sessions': [], 'replaced': []}


def quiet(fn, *a, **kw):
    out = io.StringIO()
    with contextlib.redirect_stdout(out):
        return fn(*a, **kw), out.getvalue()


class PrefsTest(unittest.TestCase):
    def setUp(self):
        self.state, self.cfg = Path(mkdtemp()), Path(mkdtemp())
        self.prefs = self.state / 'prefs.json'

    def app(self):
        return quiet(server.App, fleet.Fleet(MACH, Quiet()), state_dir=self.state, web_dir=self.state, cfg_dir=self.cfg)

    def test_a_damaged_prefs_file_turns_no_opt_in_on(self):
        write(self.cfg / 'fleet.json', {'machines': MACH})  # a server that ran before
        self.prefs.write_bytes(ZEROS)
        app, said = self.app()
        self.assertEqual(app.options(), {k: False for k in options.KEYS})
        self.assertEqual(app.fleet.options, {k: False for k in options.KEYS}, 'and no machine is told otherwise')
        self.assertFalse(self.prefs.exists(), 'nothing was written in its place')
        self.assertEqual((self.state / kept(self.prefs)[0]).read_bytes(), ZEROS)
        self.assertIn('prefs.json could not be read', said)
        self.assertNotIn('every opt-in setting stays on', said)

    def test_it_stays_that_way_over_a_restart_and_over_what_the_server_saves_by_itself(self):
        write(self.cfg / 'fleet.json', {'machines': MACH})
        self.prefs.write_bytes(ZEROS)
        app, _ = self.app()
        _, said = quiet(app._remember, {'model': None, 'effort': None, 'mode': 'normal', 'flags': [], 'other': ''})
        self.assertFalse(self.prefs.exists(), 'the last setup used is not worth writing over it')
        self.assertNotIn('Traceback', said)
        with mock.patch.dict(durable._marks, clear=True), mock.patch.object(durable, '_looked', set()):
            app, _ = self.app()
            self.assertEqual(app.options(), {k: False for k in options.KEYS})
            self.assertFalse(self.prefs.exists())

    def test_saving_settings_ends_it_and_the_opt_ins_stay_as_saved(self):
        write(self.cfg / 'fleet.json', {'machines': MACH})
        self.prefs.write_bytes(ZEROS)
        app, _ = self.app()
        got = app.set_prefs({'summary_model': 'sonnet', 'ai_ask_cards': True})
        self.assertTrue(got['ok'])
        saved = json.loads(self.prefs.read_text())
        self.assertEqual((saved['summary_model'], saved['ai_ask_cards']), ('sonnet', True))
        self.assertIn(options.MIGRATION, saved['migrations'], 'so the next start does not take it for an old install')
        app, said = self.app()
        self.assertEqual(app.options(), dict({k: False for k in options.KEYS}, ai_ask_cards=True))
        self.assertNotIn('options:', said)

    def test_the_last_good_copy_is_used_when_there_is_one(self):
        app, _ = self.app()  # a new server: the opt-ins start off, and that is written down
        app.set_prefs({'ai_chat_names': True})
        app.set_prefs({'summary_model': 'sonnet'})
        self.prefs.write_bytes(ZEROS)
        app, said = self.app()
        self.assertEqual(app.options()['ai_chat_names'], True)
        self.assertEqual(app.prefs()['summary_model'], 'opus', 'as it was before its last write')
        self.assertIn('prefs.json could not be read', said)
        self.assertEqual(told(app.stores(), self.prefs), [('prefs.json', 'recovered', False)])

    def test_a_save_keeps_what_it_does_not_know(self):
        write(self.prefs, {'migrations': [options.MIGRATION], 'from_a_newer_server': {'a': 1}})
        app, _ = self.app()
        app.set_prefs({'summary_model': 'sonnet'})
        app._remember({'model': None, 'effort': None, 'mode': 'normal', 'flags': [], 'other': ''})
        saved = json.loads(self.prefs.read_text())
        self.assertEqual((saved['from_a_newer_server'], saved['summary_model']), ({'a': 1}, 'sonnet'))

    def test_a_first_start_still_migrates_as_before(self):
        app, said = self.app()
        self.assertIn('a new server', said)
        write(self.cfg / 'fleet.json', {'machines': MACH})
        self.prefs.unlink()
        for bak in self.state.glob('prefs.json.bak'):
            bak.unlink()
        app, said = self.app()
        self.assertIn('every opt-in setting stays on', said)
        self.assertEqual(app.options(), {k: True for k in options.KEYS})


class OtherAppStoresTest(unittest.TestCase):
    def setUp(self):
        self.state = Path(mkdtemp())
        self.app, _ = quiet(server.App, fleet.Fleet(MACH, Quiet()), state_dir=self.state, web_dir=self.state)

    def test_names_come_back_from_the_last_good_copy(self):
        self.app.set_name('studio/a', 'Alpha')
        self.app.set_name('studio/b', 'Beta')
        (self.state / 'names.json').write_bytes(ZEROS)
        self.assertEqual(self.app.names(), {'studio/a': 'Alpha'})
        self.app.set_name('studio/c', 'Gamma')
        self.assertEqual(self.app.names(), {'studio/a': 'Alpha', 'studio/c': 'Gamma'})

    def test_broken_names_are_not_tidied_away_but_a_rename_is_taken(self):
        p = self.state / 'names.json'
        p.write_bytes(ZEROS)
        self.assertEqual(self.app.names(), {})
        self.app.forget_names(['studio/a'])  # the server's own housekeeping
        self.assertFalse(p.exists())
        self.app.set_name('studio/a', 'Alpha')  # the user's
        self.assertEqual(json.loads(p.read_text()), {'studio/a': 'Alpha'})

    def test_phones_and_chrome_machines(self):
        tok = 'ab' * 32
        self.app._token_add('devices.json', tok, 'iPhone', 16)
        self.app._token_add('devices.json', 'cd' * 32, 'iPad', 16)
        (self.state / 'devices.json').write_bytes(b'{"ab')
        self.assertEqual(list(self.app.devices()), [tok, 'cd' * 32])
        (self.state / 'chrome.json').write_bytes(b'[]')
        self.assertEqual(self.app.chrome_machines(), {})
        self.app.token_gone('devices.json', tok)
        self.assertEqual(list(self.app.devices()), ['cd' * 32])

    def test_a_phone_that_was_taken_off_does_not_come_back_from_the_copy(self):
        gone, stays = 'ab' * 32, 'cd' * 32
        for file, load in (('devices.json', self.app.devices), ('widget_tokens.json', self.app.widget_tokens)):
            self.app._token_add(file, gone, 'iPhone', 16)
            self.app._token_add(file, stays, 'iPad', 16)
            self.app.token_gone(file, gone)
            self.assertNotIn(gone, (self.state / (file + '.bak')).read_text(encoding='utf-8'), file)
            (self.state / file).write_bytes(ZEROS)
            self.assertEqual(list(load()), [stays], file)

    def test_a_phone_pushed_off_the_end_of_the_list_does_not_come_back_either(self):
        for i in range(3):
            self.app._token_add('devices.json', f'{i:02x}' * 32, 'iPhone', 2)
        (self.state / 'devices.json').write_bytes(ZEROS)
        self.assertEqual(list(self.app.devices()), ['01' * 32, '02' * 32])

    def test_the_apps_are_told_which_stores_were_damaged(self):
        (self.state / 'names.json').write_bytes(ZEROS)
        self.app.names()
        with mock.patch.object(serverctl, '_probe', lambda *a, **k: None):
            rows = self.app.get('/api/server')['stores']
        self.assertEqual(told(rows, self.state / 'names.json'), [('names.json', 'broken', False)])
        self.assertEqual(set(rows[0]), {'file', 'status', 'at', 'kept', 'replaced'})
        self.app.feat.library.broken = True
        self.assertIn(('library.json', 'broken'), [(r['file'], r['status']) for r in self.app.stores()])


class FleetConfigTest(unittest.TestCase):
    def setUp(self):
        self.d = Path(mkdtemp())
        self.p = self.d / 'cfg' / 'fleet.json'
        self.seed = self.d / 'machines.json'
        write(self.seed, {'machines': [{'name': 'studio', 'label': 'Studio', 'target': 'me@x', 'os': 'mac'},
                                       {'name': 'tower', 'label': 'Tower', 'target': 'me@y', 'os': 'windows'},
                                       {'name': 'lab', 'label': 'Lab', 'target': 'node:lab', 'os': 'linux'}]})
        self.full = [{'name': 'studio', 'label': 'Studio', 'target': 'me@x', 'os': 'mac', 'form': 'laptop',
                      'root': '~/Claude', 'roam': False},
                     {'name': 'tower', 'label': 'Tower', 'target': 'me@y', 'os': 'windows'}]
        self.closed = [dict(m, roam=False) for m in self.full]  # as it comes back from a copy

    def cfg(self):
        return fleet.FleetConfig(self.p, seed=self.seed)

    def test_a_damaged_list_comes_back_from_its_last_good_copy(self):
        c = self.cfg()
        c.save(self.full)
        self.p.write_bytes(ZEROS)
        self.assertEqual(quiet(self.cfg().load)[0], self.closed)
        self.assertEqual(json.loads(self.p.read_text())['machines'], self.closed)
        self.assertEqual(len(kept(self.p)), 1)

    def test_a_machine_that_was_removed_does_not_come_back_from_the_copy(self):
        c = self.cfg()
        c.save(self.full)
        c.save(self.full[:1])  # tower is removed in Settings
        self.assertNotIn('tower', self.p.with_name('fleet.json.bak').read_text(encoding='utf-8'))
        self.p.write_bytes(ZEROS)
        self.assertEqual([m['name'] for m in quiet(self.cfg().load)[0]], ['studio'])

    def test_anywhere_in_home_does_not_come_back_from_the_copy(self):
        c = self.cfg()
        c.save(self.full)   # tower may start chats anywhere in home
        write(self.p, {'machines': [dict(m, roam=False) for m in self.full]})  # turned off by hand,
        self.p.write_text(self.p.read_text(encoding='utf-8')[:-2], encoding='utf-8')  # with a typo
        got, said = quiet(self.cfg().load)
        self.assertEqual([m.get('roam') for m in got], [False, False])
        self.assertIn('Anywhere in home', said)
        self.assertEqual([m.get('roam') for m in self.cfg().load()], [False, False], 'and it stays off after a restart')

    def test_a_good_start_keeps_a_copy_even_if_nothing_is_ever_saved_again(self):
        write(self.p, {'machines': self.full})
        self.assertEqual(self.cfg().load(), self.full)
        self.p.write_bytes(b'{"machines": [{"name": "stu')
        self.assertEqual(quiet(self.cfg().load)[0], self.closed)

    def test_a_list_that_will_not_open_is_read_from_its_copy_and_left_alone(self):
        write(self.p, {'machines': self.full})
        self.cfg().load()
        before = self.p.read_bytes()
        with test_safe_load.unreadable(self.p), mock.patch.object(durable, 'READ_RETRY', (0, 0)):
            got, _ = quiet(self.cfg().load)
        self.assertEqual(got, self.closed)
        self.assertEqual(self.p.read_bytes(), before)

    def test_with_no_copy_the_machines_keep_working_but_stay_in_their_claude_folders(self):
        write(self.p, '{"machines": [{"name": "stu')
        c = self.cfg()
        got = c.load()
        self.assertEqual([(m['name'], m.get('roam')) for m in got], [('studio', False), ('tower', False)],
                         'reseeded from the hub file, which never says where chats may go: nowhere new')
        self.assertFalse(self.p.exists(), 'and that guess is not written down as the list')
        self.assertTrue(durable.broken(self.p))
        with self.assertRaises(OSError):
            c.save(got)
        c.save(got, force=True)  # a change made in Settings
        self.assertEqual(self.cfg().load(), got)

    def test_valid_json_that_is_no_machine_list_is_damage_too(self):
        write(self.p, {'machines': 'none'})
        self.assertEqual([m.get('roam') for m in self.cfg().load()], [False, False])
        self.assertEqual(len(kept(self.p)), 1)

    def test_a_first_start_seeds_and_saves_as_before(self):
        got = self.cfg().load()
        self.assertEqual([(m['name'], m.get('roam')) for m in got], [('studio', None), ('tower', None)])
        self.assertTrue(self.p.exists())


class FleetSavesTest(unittest.TestCase):
    """With fleet.json broken: what a machine reports is not written over it, a change in Settings is."""

    def setUp(self):
        self.state, self.cfg = Path(mkdtemp()), Path(mkdtemp())
        self.p = self.cfg / 'fleet.json'
        self.p.write_bytes(ZEROS)
        write(self.state / 'machines.json', {'machines': MACH})
        machines = fleet.FleetConfig(self.p, seed=self.state / 'machines.json').load()
        self.app, _ = quiet(server.App, fleet.Fleet(machines, Quiet()), state_dir=self.state, web_dir=self.state, cfg_dir=self.cfg)

    def test_what_a_machine_says_about_itself_is_not_saved_over_it(self):
        (changed, said) = quiet(self.app.fleet.learn, 'studio', {'os': 'mac', 'device': {'os': 'mac', 'form': 'laptop'}})
        self.assertTrue(changed)
        self.assertFalse(self.p.exists())
        self.assertIn('could not save the machine list', said)

    def test_a_linked_machine_that_comes_back_stays_in_its_claude_folder(self):
        import nodes
        with mock.patch.object(nodes, 'hub_call', lambda *a, **k: {'nodes': [{'name': 'lab', 'label': 'Lab', 'os': 'linux', 'online': False}]}):
            quiet(nodes.sync, self.app.fleet)
        self.assertIs(self.app.fleet.machines['lab'].get('roam'), False)
        self.assertFalse(self.p.exists())

    def test_a_change_in_settings_is_saved_and_ends_it(self):
        self.assertEqual(self.app.machines_view()['machines'][0]['roam'], False)
        self.app.machines_post({'op': 'roam', 'machine': 'studio', 'roam': True})
        self.assertEqual(json.loads(self.p.read_text())['machines'], [dict(MACH[0])])
        self.assertFalse(durable.broken(self.p))
        import nodes
        with mock.patch.object(nodes, 'hub_call', lambda *a, **k: {'nodes': [{'name': 'lab', 'label': 'Lab', 'os': 'linux', 'online': False}]}):
            quiet(nodes.sync, self.app.fleet)
        self.assertNotIn('roam', self.app.fleet.machines['lab'], 'a machine linked from now on is like any other')


class ServerConfigTest(unittest.TestCase):
    def setUp(self):
        self.d = Path(mkdtemp())
        self.p = self.d / 'cfg' / 'server.json'
        self.mine = {'bind': ['100.64.0.5'], 'hosts': ['studio'], 'origins': ['http://100.64.0.5:8130'], 'allow': [], 'peers': []}

    def cfg(self):
        return quiet(server.ServerConfig, self.d / 'cfg', self.d / 'state', local=lambda: ['10.0.0.9', 'box'])

    def test_a_damaged_file_never_opens_a_server_that_was_closed_down(self):
        write(self.p, self.mine)
        self.cfg()  # a good start keeps a copy
        self.p.write_bytes(ZEROS)
        c, said = self.cfg()
        self.assertEqual((c.bind, c.origins, c.allow), (['100.64.0.5'], ['http://100.64.0.5:8130'], set()))
        self.assertFalse(c.host_ok('10.0.0.9'))
        self.assertEqual(json.loads(self.p.read_text()), self.mine)
        self.assertIn('server.json could not be read', said)

    def test_with_no_copy_it_listens_to_this_computer_only_and_says_so(self):
        self.p.parent.mkdir(parents=True)
        self.p.write_bytes(ZEROS)
        c, said = self.cfg()
        self.assertEqual((c.bind, c.origins, c.allow), (['127.0.0.1'], [], set()))
        self.assertTrue(c.host_ok('localhost'))
        for h in ('10.0.0.9', 'box', 'studio.local', 'studio.tail1234.ts.net'):
            self.assertFalse(c.host_ok(h), h)
        self.assertFalse(c.peer_ok('10.0.0.7'))
        self.assertTrue(c.peer_ok('127.0.0.1'))
        self.assertFalse(self.p.exists(), 'open defaults are not written in its place')
        self.assertIn('this computer only', said)
        c, _ = self.cfg()
        self.assertEqual(c.bind, ['127.0.0.1'], 'and a restart changes nothing')

    def test_a_typo_after_tightening_it_by_hand_does_not_bring_the_wider_one_back(self):
        wide = dict(self.mine, bind=['0.0.0.0'], peers=['any'])
        write(self.p, wide)
        self.cfg()                # the copy is made from the wide one
        write(self.p, self.mine)  # tightened by hand
        self.cfg()                # a good start: the copy follows the file
        self.assertEqual(json.loads(self.p.with_name('server.json.bak').read_text(encoding='utf-8')), self.mine)
        self.p.write_text(json.dumps(self.mine)[:-3], encoding='utf-8')  # and later a typo
        c, _ = self.cfg()
        self.assertEqual(c.bind, ['100.64.0.5'])
        self.assertFalse(c.peer_ok('203.0.113.9'))
        self.assertEqual(json.loads(self.p.read_text(encoding='utf-8')), self.mine)

    def test_a_file_that_cannot_be_opened_at_start_uses_its_copy(self):
        write(self.p, self.mine)
        self.cfg()
        with test_safe_load.unreadable(self.p), mock.patch.object(durable, 'READ_RETRY', (0, 0)):
            c, said = self.cfg()
        self.assertEqual((c.bind, c.origins), (['100.64.0.5'], ['http://100.64.0.5:8130']))
        self.assertTrue(c.peer_ok('10.0.0.7'), 'not this computer only: it has a good copy')
        self.assertIn('could not be opened', said)
        self.assertNotIn('no good copy', said)
        self.assertNotIn('damaged', said)
        self.assertEqual(kept(self.p), [], 'nothing is put aside')

    def test_one_that_cannot_be_opened_and_has_no_copy_is_this_computer_only(self):
        write(self.p, self.mine)
        with test_safe_load.unreadable(self.p), mock.patch.object(durable, 'READ_RETRY', (0, 0)):
            c, said = self.cfg()
        self.assertEqual(c.bind, ['127.0.0.1'])
        self.assertIn('no good copy', said)
        self.assertEqual(json.loads(self.p.read_text(encoding='utf-8')), self.mine)

    def test_a_first_start_seeds_as_before(self):
        c, _ = self.cfg()
        self.assertEqual((c.bind, c.origins), (['0.0.0.0'], ['http://10.0.0.9:8130', 'http://box:8130']))
        self.assertTrue(self.p.exists())


class KeepAndTabsTest(unittest.TestCase):
    """Stop idle chats, with the list of kept chats or of open tabs unreadable: nothing stops by itself."""
    make, calls, auto, look = (getattr(test_reaper.AutoStopTest, n) for n in ('make', 'calls', 'auto', 'look'))

    def test_nothing_stops_while_either_cannot_be_read(self):
        for name in ('keep.json', 'tabs.json'):
            app = self.auto()
            self.m.sessions['desk'] = [cold(1, 30), cold(2, 30)]
            (self.state / name).write_bytes(ZEROS)
            for _ in range(3):
                self.assertEqual(self.look(app), [], name)
            self.assertEqual(len(kept(self.state / name)), 1)

    def test_nothing_stops_while_the_setting_itself_is_lost(self):
        app = self.auto({'auto_stop_h': 0})
        self.m.sessions['desk'] = [cold(1, 30)]
        (self.state / 'prefs.json').write_bytes(b'{"auto_stop_h": 0, "migr')  # no copy: this was its second write
        (self.state / 'prefs.json.bak').unlink()
        self.assertEqual(app.prefs()['auto_stop_h'], 6, 'read as the default, which is on')
        self.assertEqual(self.look(app), [], 'but it may have been off')
        app.post('/api/prefs', {'auto_stop_h': 6})
        self.assertEqual(self.look(app), [sid(1)])

    def test_keeping_a_chat_again_or_an_app_sending_its_tabs_ends_it(self):
        app = self.auto()
        self.m.sessions['desk'] = [cold(1, 30), cold(2, 30)]
        (self.state / 'keep.json').write_bytes(ZEROS)
        (self.state / 'tabs.json').write_bytes(ZEROS)
        self.assertEqual(self.look(app), [])
        app.post('/api/action', {'machine': 'desk', 'verb': 'keep', 'sid': sid(1)})
        self.assertEqual(self.look(app), [], 'the tabs are still not known')
        app.post('/api/tabs', {'device': 'studio', 'tabs': []})
        self.assertEqual(self.look(app), [sid(2)])

    def test_the_last_good_copies_are_used(self):
        app = self.auto()
        self.m.sessions['desk'] = [cold(1, 30), cold(2, 30), cold(3, 30)]
        app.fleet.poll()
        app.post('/api/action', {'machine': 'desk', 'verb': 'keep', 'sid': sid(1)})
        app.post('/api/action', {'machine': 'desk', 'verb': 'keep', 'sid': sid(3)})
        app.post('/api/action', {'machine': 'desk', 'verb': 'keep', 'sid': sid(3)})  # and not kept again: the copy still has 1 and 3
        (self.state / 'keep.json').write_bytes(ZEROS)
        self.assertEqual(self.look(app), [sid(2)])


class FeatureStoresTest(unittest.TestCase):
    def setUp(self):
        self.d = Path(mkdtemp())

    def test_every_store_there_goes_back_to_its_last_good_copy(self):
        for default, a, b in (({}, {'k': 1}, {'k': 2}), ([], [1], [1, 2])):
            p = self.d / f'{type(default).__name__}.json'
            features.jsave(p, a)
            features.jsave(p, b)
            p.write_bytes(ZEROS)
            self.assertEqual(features.jload(p, default), a)
            self.assertEqual(features.jload(p, default), a)
            self.assertEqual(len(kept(p)), 1)

    def test_with_no_copy_it_starts_over_and_the_damaged_file_is_kept(self):
        p = self.d / 'threads.json'
        p.write_bytes(b'{"links": {"a": ')
        self.assertEqual(features.jload(p, {}), {})
        features.jsave(p, {'links': {}})  # what the server gathers itself goes on being written
        self.assertEqual(features.jload(p, {}), {'links': {}})
        self.assertEqual((self.d / kept(p)[0]).read_bytes(), b'{"links": {"a": ')

    def test_the_budget(self):
        p = self.d / 'budget.json'
        budget._jsave(p, {'daily': 10.0})
        budget._jsave(p, {'daily': 12.0})
        p.write_bytes(ZEROS)
        self.assertEqual(budget._jload(p, {}), {'daily': 10.0})
        q = self.d / 'budget_events.json'
        q.write_bytes(b'{}')
        self.assertEqual(budget._jload(q, []), [])
        self.assertEqual(len(kept(q)), 1)


class HistoryTest(unittest.TestCase):
    def setUp(self):
        self.d = Path(mkdtemp())

    def test_the_week_so_far_and_past_weeks_come_back(self):
        p = self.d / 'week_history.json'
        reset = NOW + 3 * 86400
        h = fleet.WeekHistory(p, every=60)
        self.assertTrue(h.record(10, reset, NOW))
        self.assertTrue(h.record(11, reset, NOW + 100))
        self.assertTrue(h.record(12, reset, NOW + 200))
        p.write_bytes(ZEROS)
        write(self.d / 'week_archive.json', {'weeks': [{'reset': 1.0, 'pct': 50}]})
        fleet.WeekHistory(p)  # a good start keeps a copy of the archive
        (self.d / 'week_archive.json').write_bytes(ZEROS)
        h = fleet.WeekHistory(p, every=60)
        self.assertEqual([s[1] for s in h.samples], [10.0, 11.0], 'as it was before its last write')
        self.assertEqual(h.weeks, [{'reset': 1.0, 'pct': 50}])
        self.assertTrue(h.record(13, reset, NOW + 300))
        self.assertEqual(len(json.loads(p.read_text())['samples']), 3)

    def test_a_week_file_with_the_wrong_things_in_it_is_damage_too(self):
        p = self.d / 'week_history.json'
        write(p, {'reset': 'soon', 'samples': 5})
        h = fleet.WeekHistory(p)
        self.assertEqual((h.reset, h.samples), (None, []))
        self.assertEqual(len(kept(p)), 1)
        self.assertTrue(h.record(10, NOW + 86400, NOW), 'and the week starts being written down again')

    def test_what_each_machine_used(self):
        f = fleet.Fleet(MACH, Quiet())
        p = self.d / 'spend.json'
        row = {'since': 5.0, 'at': 6.0, 'total': 9, 'n': 0, 'chats': []}
        write(p, {'studio': row})
        f.keep_spend(p)
        had = dict(f.spend)
        self.assertTrue(had, 'a readable file is taken')
        p.write_bytes(ZEROS)
        g = fleet.Fleet(MACH, Quiet())
        g.keep_spend(p)
        self.assertEqual(g.spend, had, 'the copy a good start kept')
        self.assertEqual(len(kept(p)), 1)


class TermKeyTest(unittest.TestCase):
    def test_a_key_of_the_wrong_length_is_put_aside_and_a_new_one_made(self):
        for raw in (ZEROS[:32], b'', b'short', b'x' * 33):
            p = Path(mkdtemp()) / 'term.key'
            p.write_bytes(raw)
            k, said = quiet(term.load_or_create_key, p)
            self.assertEqual(len(k), term.KEY_LEN)
            self.assertNotEqual(k, ZEROS[:32])
            self.assertEqual(p.read_bytes(), k)
            self.assertEqual((p.parent / kept(p)[0]).read_bytes(), raw)
            self.assertIn('term.key', said)
            self.assertEqual(term.load_or_create_key(p), k, 'and that one is kept')
            if os.name != 'nt':
                self.assertEqual(os.stat(p).st_mode & 0o777, 0o600)


class UpdateStateTest(unittest.TestCase):
    def test_the_updaters_progress_reaches_the_disk_before_it_is_renamed(self):
        from tests.test_durable import Order
        d = Path(mkdtemp())
        st = selfupdate.State(d)
        with Order() as seen:
            st.put(step='download', ok=None)
        self.assertEqual(seen, ['fsync', 'replace'])
        self.assertEqual(json.loads((d / 'state.json').read_text())['step'], 'download')
        self.assertEqual(os.listdir(d), ['state.json'])

    def test_the_first_line_of_an_update_is_written_the_same_way(self):
        from tests.test_durable import Order
        state = Path(mkdtemp())
        with mock.patch.object(serverctl, 'why_not', lambda *a: None), mock.patch.object(serverctl, 'detach', lambda *a, **k: None), \
                mock.patch.object(serverctl, 'version', lambda: ('0.8.1', '')), \
                mock.patch.object(serverctl.releases_mod, 'repo', lambda: 'owner/name'), Order() as seen:
            serverctl.start_update(state, '0.8.2', 'a' * 64)
        self.assertEqual(seen[:2], ['fsync', 'replace'])
        self.assertEqual(json.loads((state / 'update' / 'state.json').read_text())['step'], 'starting')

    def test_a_zeroed_progress_file_reads_as_no_update(self):
        state = Path(mkdtemp())
        write(state / 'update' / 'state.json', '')
        (state / 'update' / 'state.json').write_bytes(ZEROS)
        self.assertIsNone(serverctl.update_state(state))
        self.assertEqual(selfupdate.State(state / 'update').s, {})


class LibraryStoreTest(unittest.TestCase):
    """library.json: the tools, and which machine has which. Read as empty, every machine would drop every tool."""
    setUp, publish, install_of = (test_library.ServerLibraryTest.setUp, test_library.ServerLibraryTest.publish,
                                  test_library.ServerLibraryTest.install_of)

    def tool(self, name, machine=None):
        self.made = getattr(self, 'made', 0) + 1  # a folder of its own each time: one tool may be published twice
        self.publish(test_library.make_plugin(self.tmp / f'{name}-{self.made}', name), name)
        if machine:
            self.app.post('/api/library/connect', {'tool': name, 'machine': machine, 'on': True})

    def start(self):
        """The library as the next start of the server reads it, and what it logged."""
        lib, said = quiet(library.Library, self.app, server.HttpError)
        lib.background = lambda fn: fn()
        return lib, said

    def drops(self, lib, machine):
        self.f.calls.clear()
        quiet(lib.reconcile, machine)
        return self.f.verbs(machine, 'library_drop')

    def test_a_damaged_library_comes_back_from_its_last_good_copy(self):
        self.tool('alpha', 'lap')
        p = self.state / 'library.json'
        p.write_bytes(ZEROS)
        lib, said = self.start()
        self.assertFalse(lib.broken)
        self.assertEqual((list(lib.d['tools']), lib.d['machines']['lap']), (['alpha'], ['alpha']))
        self.assertEqual((self.state / kept(p)[0]).read_bytes(), ZEROS, 'the file as it was found is kept')
        self.assertEqual(json.loads(p.read_text())['machines']['lap'], ['alpha'], 'and the copy is written back')
        self.assertIn('last good copy', said)
        self.assertEqual(self.drops(lib, 'lap'), [])
        self.assertEqual(self.install_of('lap', 'alpha'), 1)

    def test_a_tool_that_was_deleted_does_not_come_back_from_the_copy(self):
        self.tool('alpha', 'lap')
        self.tool('beta', 'lap')
        quiet(self.app.post, '/api/library/delete', {'tool': 'beta'})
        self.app.post('/api/library/connect', {'tool': 'alpha', 'machine': 'lap', 'on': False})
        (self.state / 'library.json').write_text('{"tools": {"alpha": {"latest": 1, "vers')
        lib, _ = self.start()
        self.assertFalse(lib.broken)
        self.assertEqual(list(lib.d['tools']), ['alpha'], 'its files are gone: it must not be offered again')
        self.assertEqual(lib.d['machines'].get('lap') or [], [], 'nor a tool put back on a machine it was taken off')

    def test_with_no_copy_no_machine_is_emptied(self):
        self.tool('alpha', 'lap')
        p = self.state / 'library.json'
        for text in ('{"tools": {"alpha": {"latest": 1, "vers', '[]', ZEROS.decode()):
            p.write_text(text)
            p.with_name('library.json.bak').unlink()
            lib, said = self.start()
            self.assertTrue(lib.broken, text)
            self.assertEqual(lib.d['tools'], {})
            self.assertFalse(p.exists())
            self.assertEqual((self.state / kept(p)[-1]).read_text(), text, 'kept for whoever repairs it')
            self.assertIn("no machine's tools are touched", said)
            self.assertEqual(self.drops(lib, 'lap'), [])
            self.assertEqual(self.install_of('lap', 'alpha'), 1)
            # over a restart too, with the file gone and the one put aside still there
            with mock.patch.dict(durable._marks, clear=True), mock.patch.object(durable, '_looked', set()):
                later, _ = self.start()
                self.assertTrue(later.broken)
                self.assertEqual(self.drops(later, 'lap'), [])
            # a tool published now is saved: the library starts over from here
            self.app.feat.library = lib
            self.tool('alpha')
            self.assertEqual(list(json.loads(p.read_text())['tools']), ['alpha'])
            self.assertTrue(lib.broken, 'and still nothing is dropped until the server starts again')
            self.assertEqual(told(self.app.stores(), p), [('library.json', 'broken', True)], 'said once, not twice')

    def test_a_library_an_earlier_server_put_aside_still_counts(self):
        # before durable.load it was moved to library.json.bad, and the next start found no library.json
        (self.state / 'library.json').unlink(missing_ok=True)
        (self.state / 'library.json.bad').write_text('{"tools": {"alpha"')
        with mock.patch.dict(durable._marks, clear=True), mock.patch.object(durable, '_looked', set()):
            lib, _ = self.start()
        self.assertTrue(lib.broken)
        self.assertEqual((self.state / 'library.json.bad').read_text(), '{"tools": {"alpha"')

    def test_a_first_start_has_an_empty_library_and_that_is_fine(self):
        self.assertFalse(self.lib.broken)
        self.assertNotIn(str(self.state / 'library.json'), durable._marks)


class JobsStoreTest(unittest.TestCase):
    """jobs.json: launches the server has taken and is still watching."""
    make, wait = test_server_jobs.ServerJobTest.make, test_server_jobs.ServerJobTest.wait
    SID = test_server_jobs.SID

    def running(self, now):
        return {'kind': 'fresh', 'machine': 'desk', 'plan': {'req': {'verb': 'fresh', 'sid': self.SID}}, 'title': 'Lights',
                'state': 'running', 'step': 'running', 'created': now - 30, 'watched': 0, 'inner': '0123456789ab'}

    def test_a_jobs_file_of_zeros_starts_the_list_over_and_the_next_job_is_saved(self):
        st = Path(mkdtemp())
        app = self.make(state=st)
        r = app.post('/api/action', {'machine': 'desk', 'verb': 'resume', 'sid': self.SID})
        self.wait(app, r['job'])
        # no copy is kept to go back to: one would be a step behind what the machines were really sent
        self.assertEqual([n for n in os.listdir(st) if n.startswith('jobs.')], ['jobs.json'])
        (st / 'jobs.json').write_bytes(ZEROS)
        app, said = quiet(self.make, state=st)
        self.assertEqual(app.jobs, {})
        self.assertEqual((st / kept(st / 'jobs.json')[0]).read_bytes(), ZEROS)
        self.assertIn('jobs.json could not be read: the file is kept beside it', said)
        r = app.post('/api/action', {'machine': 'desk', 'verb': 'resume', 'sid': self.SID})
        self.assertEqual(self.wait(app, r['job'])['state'], 'done')
        self.assertIn(r['job'], json.loads((st / 'jobs.json').read_text()))

    def test_a_file_that_reads_in_part_keeps_what_reads(self):
        st, now = Path(mkdtemp()), time.time()
        text = json.dumps({'s-aaaaaaaaaaaa': self.running(now), 's-bbbbbbbbbbbb': None})
        (st / 'jobs.json').write_text(text)
        app, said = quiet(self.make, state=st)
        self.assertEqual(self.wait(app, 's-aaaaaaaaaaaa')['state'], 'done')
        self.assertNotIn('s-bbbbbbbbbbbb', app.jobs)
        self.assertEqual((st / kept(st / 'jobs.json')[0]).read_text(), text, 'the file as it was is kept')


class LeftAloneTest(unittest.TestCase):
    """Raw reads that are not stores the server reads and then writes, so durable.load is not for them."""

    def test_reaper_json_is_only_read_and_a_damaged_one_turns_suggestions_off(self):
        cfg = Path(mkdtemp()) / 'reaper.json'
        cfg.write_bytes(ZEROS)
        got = features.Reaper(mock.Mock(state_dir=cfg.parent), cfg).config()
        self.assertEqual((got['enabled'], 'unreadable' in got['error']), (False, True))
        self.assertEqual(cfg.read_bytes(), ZEROS, 'it is the user\'s file: left as it is, with nothing put beside it')
        self.assertEqual(os.listdir(cfg.parent), ['reaper.json'])

    def test_the_seed_for_the_machine_list_is_only_read(self):
        d = Path(mkdtemp())
        (d / 'machines.json').write_bytes(ZEROS)
        self.assertIsNone(fleet.FleetConfig._read(d / 'machines.json'))
        self.assertEqual(os.listdir(d), ['machines.json'])


if __name__ == '__main__':
    unittest.main()
