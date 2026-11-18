"""Compact long chats at: one value, kept on the server (server/options.py compact_at), sent with every poll and
written by each helper as CLAUDE_CODE_AUTO_COMPACT_WINDOW. A value the user set themselves is never touched, and an
older helper or an older server means 300k, as before the value existed."""
import json, os, sys, tempfile, threading, unittest
from pathlib import Path
from tempfile import mkdtemp
from tests.helpers import fresh_sb, sb_options
from tests.test_week_history import MACH

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'server'))
import fleet, options, server  # noqa: E402

KEY = 'CLAUDE_CODE_AUTO_COMPACT_WINDOW'
ON = {'setup_compact_window': True}


def app_at(state, reply=None):
    return server.App(fleet.Fleet(MACH, lambda *a, **k: dict({'ok': True, 'sessions': []}, **(reply or {}))), state_dir=state,
                      web_dir=state, sleep=lambda s: None, poll_every=0, cfg_dir=Path(mkdtemp()))


class ServerValueTest(unittest.TestCase):
    def test_a_new_server_has_300k_and_the_switch_off(self):
        app = app_at(Path(mkdtemp()))
        c = app.get('/api/prefs')['compact']
        self.assertEqual(c, {'at': 300_000, 'on': False, 'min': 100_000, 'max': 1_000_000, 'recommended': 300_000,
                             'stops': [150_000, 200_000, 300_000, 500_000]})
        self.assertEqual(app.prefs()['compact_at'], 300_000)

    def test_a_server_with_the_old_switch_on_stays_at_300k(self):
        state = Path(mkdtemp())
        (state / 'prefs.json').write_text(json.dumps({'setup_compact_window': True, 'migrations': [options.MIGRATION]}))
        app = app_at(state)  # prefs.json from before the value existed
        self.assertEqual(app.get('/api/prefs')['compact'], dict(app.get('/api/prefs')['compact'], at=300_000, on=True))
        req = app.fleet._list_req(app.fleet.machines['desk'])
        self.assertEqual((req['options']['setup_compact_window'], req['compact_at']), (True, 300_000))
        row = next(o for o in app.get('/api/prefs')['options'] if o['key'] == 'setup_compact_window')
        self.assertEqual(row['label'], 'Compact long chats at 300k tokens')
        self.assertIn('CLAUDE_CODE_AUTO_COMPACT_WINDOW=300000', row['explain'])

    def test_a_value_is_saved_sent_with_every_poll_and_survives_a_restart(self):
        state = Path(mkdtemp())
        app = app_at(state)
        r = app.post('/api/prefs', {'setup_compact_window': True, 'compact_at': 500_000})
        self.assertEqual((r['compact_at'], r['setup_compact_window']), (500_000, True))
        self.assertEqual(app.fleet._list_req(app.fleet.machines['desk'])['compact_at'], 500_000)
        self.assertEqual(app.sessions()['prefs']['compact_at'], 500_000)
        again = app_at(state)
        self.assertEqual(again.get('/api/prefs')['compact']['at'], 500_000)
        self.assertEqual(again.fleet._list_req(again.fleet.machines['desk'])['compact_at'], 500_000)
        row = next(o for o in again.get('/api/prefs')['options'] if o['key'] == 'setup_compact_window')
        self.assertEqual(row['label'], 'Compact long chats at 500k tokens')  # an older app shows the switch, worded right
        self.assertIn('=500000', row['explain'])

    def test_other_settings_keep_the_value_and_the_switch_keeps_it_too(self):
        app = app_at(Path(mkdtemp()))
        app.post('/api/prefs', {'compact_at': 200_000})
        app.post('/api/prefs', {'summary_model': 'sonnet'})
        app.post('/api/prefs', {'setup_compact_window': True})
        app.post('/api/prefs', {'setup_compact_window': False})  # the default stop: the value is remembered
        self.assertEqual(app.prefs()['compact_at'], 200_000)

    def test_nonsense_is_refused_and_nothing_changes(self):
        app = app_at(Path(mkdtemp()))
        app.post('/api/prefs', {'compact_at': 150_000})
        for bad in (99_999, 1_000_001, 0, -300_000, 300_000.0, 3e5, '300000', '300k', True, None, [300_000], {'at': 1}, float('nan')):
            with self.assertRaises(server.HttpError, msg=repr(bad)) as e:
                app.post('/api/prefs', {'compact_at': bad, 'summary_model': 'sonnet'})
            self.assertEqual(e.exception.code, 400)
        self.assertEqual(app.prefs()['compact_at'], 150_000)
        self.assertEqual(app.prefs()['summary_model'], 'opus', 'a refused save changes nothing else either')
        for edge in (100_000, 1_000_000, 123_456):
            self.assertEqual(app.post('/api/prefs', {'compact_at': edge})['compact_at'], edge)

    def test_a_damaged_stored_value_reads_as_300k(self):
        for stored in ('500000', 5, 2_000_000, None, True, 1.5e5, [], {}):
            state = Path(mkdtemp())
            (state / 'prefs.json').write_text(json.dumps({'compact_at': stored, 'setup_compact_window': True,
                                                          'migrations': [options.MIGRATION]}))
            app = app_at(state)
            self.assertEqual(app.prefs()['compact_at'], 300_000, repr(stored))
            self.assertEqual(app.fleet._list_req(app.fleet.machines['desk'])['compact_at'], 300_000)

    def test_what_a_machine_says_about_its_own_window_reaches_settings(self):
        said = {'set': True, 'raw': '500000', 'value': 500_000, 'own': False, 'other': None}
        app = app_at(Path(mkdtemp()), {'compact': said, 'features': ['compact_at']})
        app.fleet.poll_one('desk')
        m = next(m for m in app.get('/api/machines')['machines'] if m['name'] == 'desk')
        self.assertEqual((m['compact'], 'compact_at' in m['features']), (said, True))

    def test_an_older_helper_says_nothing_and_an_odd_answer_is_dropped(self):
        for said in (None, 'yes', 3, {}, {'set': 'yes'}, {'value': 500_000}):
            self.assertIsNone(options.compact_seen(said), repr(said))
        seen = options.compact_seen({'set': True, 'raw': 'x' * 99, 'value': 5, 'own': 'yes', 'other': 'rm -rf'})
        self.assertEqual(seen, {'set': True, 'own': False, 'value': None, 'raw': 'x' * 24, 'other': None})
        self.assertFalse(options.compact_seen({'set': False, 'own': True})['own'], 'nothing set is nobody\'s')
        app = app_at(Path(mkdtemp()))
        app.fleet.poll_one('desk')
        self.assertIsNone(app.get('/api/machines')['machines'][0]['compact'])


class HelperBase(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(); self.sb = fresh_sb(self.tmp)
        self.cdir = Path(self.tmp) / 'home' / '.claude'; (self.cdir / 'switchboard').mkdir(parents=True)  # where sb.py lives
        self.settings = self.cdir / 'settings.json'  # a scratch copy: never a real ~/.claude/settings.json

    def put(self, d):
        self.settings.write_text(d if isinstance(d, str) else json.dumps(d))

    def env(self):
        return json.loads(self.settings.read_text()).get('env') or {}

    def setup_json(self):
        return json.loads((self.cdir / 'switchboard' / 'setup.json').read_text())

    def apply(self, on=True, at=None):
        return self.sb.apply_setup(self.cdir, ON if on else {}, compact=str(at) if at else None)


class HelperWriteTest(HelperBase):
    def test_no_settings_file_and_no_env_get_the_value(self):
        self.assertEqual(self.apply(at=500_000), {'setup_compact_window': 'added'})  # no settings.json at all
        self.assertEqual(self.env(), {KEY: '500000'})
        self.assertEqual(self.setup_json(), {'added': {'setup_subagent_rule': False, 'setup_explore_agent': False,
                                                       'setup_compact_window': True}, 'compact_value': '500000'})
        self.put({'model': 'opus', 'env': {'KEEP': '1'}})
        (self.cdir / 'switchboard' / 'setup.json').write_text(json.dumps({'added': {}}))
        self.assertEqual(self.apply(at=150_000), {'setup_compact_window': 'added'})
        self.assertEqual(json.loads(self.settings.read_text()), {'model': 'opus', 'env': {'KEEP': '1', KEY: '150000'}})

    def test_with_no_value_from_the_server_it_is_300000_as_before(self):
        self.put({})
        self.apply()
        self.assertEqual(self.env()[KEY], '300000')
        self.assertEqual(self.sb.compact_want(), '300000')

    def test_a_value_the_user_set_is_left_alone_whatever_the_slider_says(self):
        for theirs in ('500000', '300000', '200k', 'garbage', ''):
            self.put({'env': {KEY: theirs}})
            (self.cdir / 'switchboard' / 'setup.json').write_text(json.dumps({'added': {}}))  # an install that added nothing
            for at in (150_000, 300_000, 500_000):
                self.assertEqual(self.apply(at=at), {}, theirs)
            self.assertEqual(self.apply(on=False), {}, theirs)
            self.assertEqual(self.env()[KEY], theirs)
            self.assertFalse(self.setup_json()['added']['setup_compact_window'])
            st = self.sb.compact_state(self.cdir)
            self.assertEqual((st['set'], st['own'], st['raw']), (True, False, theirs))

    def test_the_old_300000_moves_to_a_new_value_and_back(self):
        self.put({'env': {KEY: '300000', 'KEEP': '1'}})  # written by a helper from before the value existed
        (self.cdir / 'switchboard' / 'setup.json').write_text(json.dumps({'added': {'setup_compact_window': True}}))
        self.assertEqual(self.apply(at=300_000), {}, 'the update itself changes nothing')
        self.assertEqual(self.env()[KEY], '300000')
        self.assertEqual(self.apply(at=500_000), {'setup_compact_window': 'changed'})
        self.assertEqual(self.env(), {KEY: '500000', 'KEEP': '1'})
        self.assertEqual(self.setup_json()['compact_value'], '500000')
        self.assertEqual(self.apply(at=500_000), {}, 'the same again: no write')
        self.assertEqual(self.apply(at=150_000), {'setup_compact_window': 'changed'})
        self.assertEqual(self.sb.compact_state(self.cdir), {'set': True, 'raw': '150000', 'value': 150_000, 'own': True, 'other': None})
        self.assertEqual(self.apply(on=False), {'setup_compact_window': 'removed'})
        self.assertEqual(self.env(), {'KEEP': '1'})
        self.assertNotIn('compact_value', self.setup_json())

    def test_before_setup_json_a_300000_counts_as_switchboards_and_other_values_do_not(self):
        hook = {'type': 'command', 'command': 'python3 /h/.claude/switchboard/sb.py hook'}
        self.put({'env': {KEY: '300000'}, 'hooks': {'Stop': [{'hooks': [hook]}]}})
        self.assertEqual(self.apply(at=200_000), {'setup_compact_window': 'changed'})
        self.assertEqual(self.env()[KEY], '200000')

    def test_a_value_the_user_changed_after_switchboard_set_it_becomes_theirs(self):
        self.put({})
        self.apply(at=300_000)
        self.put({'env': {KEY: '400000'}})  # edited by hand
        self.assertEqual(self.apply(at=500_000), {})
        self.assertEqual(self.apply(on=False), {})
        self.assertEqual(self.env()[KEY], '400000')
        self.apply(at=500_000)
        self.put({'env': {KEY: '500000'}})  # ... and later set to the very value the slider has: still theirs
        self.assertEqual(self.apply(on=False), {})
        self.assertEqual(self.env()[KEY], '500000')

    def test_turning_it_off_removes_only_the_entry(self):
        self.put({'model': 'opus', 'env': {'KEEP': '1'}, 'hooks': {}})
        self.apply(at=500_000)
        self.assertEqual(self.apply(on=False), {'setup_compact_window': 'removed'})
        self.assertEqual(json.loads(self.settings.read_text()), {'model': 'opus', 'env': {'KEEP': '1'}, 'hooks': {}})
        self.put({})
        self.apply(at=500_000)
        self.apply(on=False)
        self.assertEqual(json.loads(self.settings.read_text()), {}, 'an env block Switchboard made goes with its last entry')

    def test_a_window_chosen_in_claude_codes_own_settings_is_not_overridden(self):
        for theirs, word in (({'autoCompactWindow': 200_000}, 'setting'), ({'autoCompactWindow': '500k'}, 'setting'),
                             ({'modelSettings': {'claude-opus-5-5': {'autoCompactWindow': 500_000}}}, 'models')):
            self.put(theirs)
            self.assertEqual(self.apply(at=300_000), {}, theirs)
            self.assertEqual(json.loads(self.settings.read_text()), theirs)
            st = self.sb.compact_state(self.cdir)
            self.assertEqual((st['set'], st['own'], st['other']), (False, False, word))
        for not_a_window in ({'autoCompactWindow': None}, {'autoCompactWindow': 'auto'}, {'autoCompactWindow': True},
                             {'modelSettings': {'claude-opus-5-5': {'effortLevel': 'high', 'autoCompactWindow': 'auto'}}},
                             {'modelSettings': 'x'}, {'modelSettings': {'m': 3}}):
            self.put(not_a_window)
            self.assertEqual(self.apply(at=300_000), {'setup_compact_window': 'added'}, not_a_window)
            self.assertEqual(self.apply(on=False), {'setup_compact_window': 'removed'})

    def test_an_entry_switchboard_already_owns_keeps_following_the_slider_beside_such_a_setting(self):
        self.put({})
        self.apply(at=300_000)
        s = json.loads(self.settings.read_text()); s['autoCompactWindow'] = 200_000
        self.put(s)  # as today: the env entry was there first and stays Switchboard's
        self.assertEqual(self.apply(at=500_000), {'setup_compact_window': 'changed'})
        self.assertEqual(self.sb.compact_state(self.cdir)['other'], 'setting')
        self.assertEqual(self.apply(on=False), {'setup_compact_window': 'removed'})
        self.assertEqual(json.loads(self.settings.read_text()), {'autoCompactWindow': 200_000})

    def test_a_malformed_settings_file_is_never_written(self):
        for text in ('{"env": {', '', 'null', '[1, 2]', '"text"'):
            self.put(text)
            with self.assertRaises(Exception):
                self.apply(at=500_000)
            self.assertEqual(self.settings.read_text(), text)
            self.assertFalse(list(self.cdir.glob('settings.json.*.tmp')))
        self.put('{"env": {')
        self.assertIsNone(self.sb.compact_state(self.cdir), 'nothing is claimed about a file that cannot be read')
        self.put('[1]')
        self.assertEqual(self.sb.compact_state(self.cdir)['set'], False)

    def test_an_env_block_that_is_not_an_object_is_not_touched_when_nothing_is_to_change(self):
        self.put({'env': 'odd'})
        self.assertEqual(self.apply(on=False), {})
        self.assertEqual(json.loads(self.settings.read_text()), {'env': 'odd'})

    def test_a_dry_run_writes_nothing(self):
        self.put({'env': {'KEEP': '1'}})
        self.assertEqual(self.sb.apply_setup(self.cdir, ON, dry_run=True, compact='500000'), {'setup_compact_window': 'added'})
        self.assertEqual(self.env(), {'KEEP': '1'})
        self.assertFalse((self.cdir / 'switchboard' / 'setup.json').exists())

    def test_uninstall_takes_back_the_value_switchboard_wrote_and_only_that(self):
        home = self.cdir.parent
        self.put({'env': {'KEEP': '1'}})
        self.sb.install(home=home)
        self.apply(at=500_000)
        self.sb.uninstall(home=home)
        self.assertEqual(self.env(), {'KEEP': '1'})
        self.put({'env': {KEY: '500000'}})  # the user's own
        self.sb.install(home=home)
        self.apply(at=500_000)
        self.sb.uninstall(home=home)
        self.assertEqual(self.env(), {KEY: '500000'})

    def test_claude_code_writing_the_file_in_between_is_never_undone(self):
        self.put({'env': {'KEEP': '1'}})
        real_write, n = self.sb.cfg_write, []

        def racing(p, data):  # Claude Code saves its own change between Switchboard's read and its write, once
            if not n:
                n.append(1)
                s = json.loads(self.settings.read_text()); s['model'] = 'sonnet'
                self.settings.write_text(json.dumps(s))
                os.utime(self.settings, ns=(1, 1))  # a different file by its stamp, whatever the clock's grain
            return real_write(p, data)
        self.sb.cfg_write = racing
        self.assertEqual(self.apply(at=500_000), {'setup_compact_window': 'added'})
        self.assertEqual(json.loads(self.settings.read_text()), {'env': {'KEEP': '1', KEY: '500000'}, 'model': 'sonnet'})

    def test_many_writers_at_once_leave_one_whole_file(self):
        self.put({'env': {'KEEP': '1'}, 'model': 'opus'})
        errors = []

        def run(at):
            try:
                for _ in range(15):
                    self.sb.apply_setup(self.cdir, ON, compact=str(at))
            except Exception as e:  # CfgChanged after its retries is a refusal, not damage
                if not isinstance(e, self.sb.CfgChanged):
                    errors.append(repr(e))
        ts = [threading.Thread(target=run, args=(at,)) for at in (150_000, 200_000, 300_000, 500_000)]
        [t.start() for t in ts]; [t.join() for t in ts]
        self.assertEqual(errors, [])
        s = json.loads(self.settings.read_text())  # whole, and nothing else lost
        self.assertEqual((s['model'], s['env']['KEEP']), ('opus', '1'))
        self.assertIn(s['env'].get(KEY), ('150000', '200000', '300000', '500000'))
        self.assertFalse(list(self.cdir.glob('settings.json.*.tmp')))


class HelperPollTest(HelperBase):
    """The poll: the value arrives beside the options and is applied to the installed Claude setup only."""
    def setUp(self):
        super().setUp()
        self.old = {k: os.environ.get(k) for k in ('SB_HOME', 'SB_CLAUDE_DIR')}
        os.environ['SB_HOME'] = str(self.cdir / 'switchboard'); os.environ['SB_CLAUDE_DIR'] = str(self.cdir)
        self.sb = fresh_sb.__globals__['importlib'].reload(self.sb)
        self.put({'env': {'KEEP': '1'}})

    def tearDown(self):
        for k, v in self.old.items():
            os.environ.pop(k, None) if v is None else os.environ.__setitem__(k, v)

    def poll(self, on=True, **more):
        self.sb.remember_identity(dict({'options': dict({k: False for k in self.sb.OPTION_KEYS}, setup_compact_window=on)}, **more))

    def test_a_new_value_is_written_and_a_changed_one_follows(self):
        self.poll(compact_at=500_000)
        self.assertEqual(self.env()[KEY], '500000')
        self.poll(compact_at=150_000)  # the switch did not change, the value did
        self.assertEqual(self.env()[KEY], '150000')
        self.poll(on=False, compact_at=150_000)
        self.assertEqual(self.env(), {'KEEP': '1'})

    def test_an_older_server_names_no_value_and_gets_300000(self):
        self.poll()
        self.assertEqual(self.env()[KEY], '300000')
        self.poll(compact_at=500_000)
        self.assertEqual(self.env()[KEY], '500000')
        self.poll()  # the server went back to an older build: back to what the switch always meant
        self.assertEqual(self.env()[KEY], '300000')

    def test_a_value_claude_code_would_not_take_is_never_written(self):
        for bad in (99_999, 2_000_000, '500000', 5e5, True, None, -1, [500_000]):
            self.poll(on=False)
            self.poll(compact_at=bad)
            self.assertEqual(self.env()[KEY], '300000', repr(bad))

    def test_a_poll_with_no_options_changes_nothing(self):
        self.poll(compact_at=500_000)
        self.sb.remember_identity({'compact_at': 150_000})  # not how a server speaks: the value comes with the options
        self.assertEqual(self.env()[KEY], '500000')

    def test_the_helper_says_it_knows_values_and_what_this_computer_has(self):
        self.assertIn('compact_at', self.sb.AGENT_FEATURES)
        self.assertEqual(self.sb.compact_or_none(), {'set': False, 'raw': None, 'value': None, 'own': False, 'other': None})
        self.poll(compact_at=500_000)
        self.assertEqual(self.sb.compact_or_none(), {'set': True, 'raw': '500000', 'value': 500_000, 'own': True, 'other': None})


class WindowTest(unittest.TestCase):
    def setUp(self):
        self.sb = fresh_sb(tempfile.mkdtemp())

    def test_the_env_entry_reads_as_claude_code_reads_it(self):
        w = self.sb.window_tokens
        self.assertEqual([w(x) for x in ('300000', '200k', '1M', '1m', ' 500K ')], [300_000, 200_000, 1_000_000, 1_000_000, 500_000])
        self.assertEqual([w(x) for x in ('200', '5000000', '0')], [100_000, 1_000_000, 100_000], 'held to 100k..1M')
        self.assertEqual([w(x) for x in ('', 'auto', '-5', '3e5', '1.5M', None, 300_000, '9' * 30)], [None] * 8)

    def test_a_chat_compacts_at_the_window_or_the_models_own_whichever_is_smaller(self):
        lim = self.sb.ctx_limit
        self.assertEqual(lim(300_000, 1_000_000), 300_000)
        self.assertEqual(lim(500_000, 200_000), 200_000, 'Claude Code caps the window at the model\'s')
        self.assertEqual(lim(None, 1_000_000), 1_000_000)
        self.assertEqual(lim(300_000, None), 300_000)
        self.assertIsNone(lim(None, None))
        self.assertIsNone(lim('x', 0))

    def test_the_status_line_writes_the_chats_limit_down(self):
        sid = '11111111-2222-3333-4444-555555555555'
        data = {'session_id': sid, 'context_window': {'context_window_size': 200_000, 'current_usage': {'input_tokens': 5}}}
        old = os.environ.get(KEY)
        try:
            os.environ[KEY] = '500000'
            self.sb.write_state(data, 1_000_000)
            self.assertEqual(self.sb.read_state(sid)['ctx_limit'], 200_000)
            data['context_window']['context_window_size'] = 1_000_000
            self.sb.write_state(data, 1_000_001)
            self.assertEqual(self.sb.read_state(sid)['ctx_limit'], 500_000)
            os.environ.pop(KEY)
            self.sb.write_state(data, 1_000_002)
            self.assertEqual(self.sb.read_state(sid)['ctx_limit'], 1_000_000)
        finally:
            os.environ.pop(KEY, None) if old is None else os.environ.__setitem__(KEY, old)


if __name__ == '__main__':
    unittest.main()
