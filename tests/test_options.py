"""Opt-in settings (server/options.py): off for a new server, on once for one that ran before, sent with every poll,
and each feature falls back to what it does without Claude when its setting is off."""
import json, os, sys, tempfile, unittest
from pathlib import Path
from tempfile import mkdtemp
from tests.helpers import fresh_sb, sb_options, user, assistant, write_transcript
import tests.test_auto as ta
from tests.test_rpc import RpcBase, NOW as RPC_NOW, SID_A
from tests.test_week_history import MACH

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'server'))
import fleet, options, server  # noqa: E402

KEYS = ('ai_ask_cards', 'ai_chat_names', 'ai_auto_machine', 'setup_subagent_rule', 'setup_explore_agent', 'setup_compact_window')


def app_at(state, cfg=None):
    return server.App(fleet.Fleet(MACH, lambda *a, **k: {'ok': True, 'sessions': []}), state_dir=state, web_dir=state,
                      sleep=lambda s: None, poll_every=0, cfg_dir=cfg or Path(mkdtemp()))


class ServerOptionsTest(unittest.TestCase):
    def test_the_keys_match_the_agents(self):
        self.assertEqual(options.KEYS, KEYS)
        self.assertEqual(fresh_sb(tempfile.mkdtemp()).OPTION_KEYS, KEYS)

    def test_a_new_server_starts_with_every_option_off(self):
        app = app_at(Path(mkdtemp()))
        self.assertEqual(app.options(), {k: False for k in KEYS})
        self.assertIn(options.MIGRATION, app.prefs()['migrations'])

    def test_a_server_that_ran_before_turns_them_on_once(self):
        state = Path(mkdtemp())
        (state / 'names.json').write_text('{}')  # state from before this change
        app = app_at(state)
        self.assertEqual(app.options(), {k: True for k in KEYS})
        app.post('/api/prefs', {'ai_chat_names': False})
        again = app_at(state)  # a restart: the migration never runs twice
        self.assertFalse(again.options()['ai_chat_names'])
        self.assertTrue(again.options()['ai_ask_cards'])

    def test_a_machine_list_counts_as_running_before(self):
        cfg = Path(mkdtemp())
        (cfg / 'fleet.json').write_text(json.dumps({'machines': []}))
        self.assertTrue(app_at(Path(mkdtemp()), cfg).options()['ai_auto_machine'])

    def test_an_empty_state_folder_is_new(self):
        state = Path(mkdtemp())
        (state / 'config').mkdir()
        self.assertFalse(any(app_at(state).options().values()))

    def test_get_and_post(self):
        app = app_at(Path(mkdtemp()))
        p = app.get('/api/prefs')
        self.assertEqual([o['key'] for o in p['options']], list(KEYS))
        for o in p['options']:
            self.assertFalse(o['on'])
            self.assertIn(o['kind'], ('ai', 'setup'))
            for words in (o['label'], o['explain'], o['off']):
                self.assertTrue(words and '\u2014' not in words, o)
        self.assertIn('goes to Claude', next(o for o in p['options'] if o['key'] == 'ai_ask_cards')['explain'])
        r = app.post('/api/prefs', {'ai_ask_cards': True, 'setup_explore_agent': True})
        self.assertTrue(r['ai_ask_cards'] and r['setup_explore_agent'] and not r['ai_chat_names'])
        p = app.get('/api/prefs')
        self.assertEqual({o['key'] for o in p['options'] if o['on']}, {'ai_ask_cards', 'setup_explore_agent'})
        self.assertTrue(app.sessions()['prefs']['ai_ask_cards'])
        with self.assertRaises(server.HttpError):
            app.post('/api/prefs', {'ai_chat_names': 'yes'})

    def test_other_settings_keep_the_options(self):
        app = app_at(Path(mkdtemp()))
        app.post('/api/prefs', {'ai_auto_machine': True})
        app.post('/api/prefs', {'summary_model': 'sonnet'})
        app._remember({'model': 'opus'})
        self.assertTrue(app.options()['ai_auto_machine'])

    def test_push_says_whether_it_is_set_up(self):
        app = app_at(Path(mkdtemp()))
        p = app.get('/api/prefs')['push']
        self.assertFalse(p['ready'])
        self.assertIn('apns.json', p['explain'])
        self.assertEqual(p['setup'], '~/.config/switchboard/apns.json')
        app.push_ready = True
        p = app.get('/api/prefs')['push']
        self.assertTrue(p['ready'])
        self.assertIn('Apple', p['explain'])
        self.assertIsNone(p['setup'])

    def test_every_poll_carries_them(self):
        app = app_at(Path(mkdtemp()))
        self.assertEqual(app.fleet._list_req(app.fleet.machines['desk'])['options'], {k: False for k in KEYS})
        app.post('/api/prefs', {'ai_chat_names': True})
        self.assertTrue(app.fleet._list_req(app.fleet.machines['desk'])['options']['ai_chat_names'])

    def test_the_csp_names_no_font_service(self):
        self.assertNotIn('googleapis', server.CSP)
        self.assertNotIn('gstatic', server.CSP)


class AutoOffTest(unittest.TestCase):
    make = ta.AutoServer.make  # the helper only, not its tests

    def test_with_claude_picks_off_the_rules_pick_and_nothing_is_sent(self):
        app = self.make()
        app.post('/api/prefs', {'ai_auto_machine': False})
        r = app.post('/api/route', {'prompt': 'Fix the switchboard iOS widget', 'folder': ''})
        self.assertEqual((r['by'], r['ai']), ('rules', False))
        self.assertEqual(r['machine'], 'desk')  # the one with Xcode
        self.assertEqual(self.f.verbs('ask'), [])

    def test_with_claude_picks_on_claude_picks(self):
        app = self.make()
        r = app.post('/api/route', {'prompt': 'Fix the switchboard iOS widget', 'folder': ''})
        self.assertEqual(r['by'], 'claude')
        self.assertNotIn('ai', r)


class AgentOptionsTest(RpcBase):
    def no_claude(self, *a, **kw):
        raise AssertionError('Claude must not be asked while the setting is off')

    def test_off_until_the_server_says(self):
        self.assertFalse(any(self.sb.option(k) for k in KEYS))
        sb_options(self.sb, ai_chat_names=True)
        self.assertTrue(self.sb.option('ai_chat_names'))
        self.sb.remember_identity({'options': {'ai_chat_names': 'yes'}})  # only true counts
        self.assertFalse(self.sb.option('ai_chat_names'))

    def test_names_fall_back_to_the_first_words(self):
        self.sb.run_claude_opus = self.no_claude
        self.assertEqual(self.sb.prompt_title('please fix the login page'), 'Fix the login page')
        sb_options(self.sb, ai_chat_names=True)
        self.sb.run_claude_opus = lambda *a, **kw: 'Login page fix'
        self.assertEqual(self.sb.prompt_title('please fix the login page'), 'Login page fix')

    def test_ask_cards_show_the_needs_text_and_start_no_pass(self):
        rows = [user('2026-10-01T00:00:00Z', 'plan it'), assistant('2026-10-01T00:00:05Z', 'Option 1 or option 2?')]
        path = write_transcript(self.cdir / 'projects' / 'p1', rows, name=f'{SID_A}.jsonl')
        os.utime(path, (RPC_NOW - 60, RPC_NOW - 60))
        spawned = []
        self.sb.spawn_ask = lambda *x: spawned.append(x)
        self.sb.run_claude_opus = self.no_claude
        a = self.sb.session_ask(SID_A, Path(path), RPC_NOW - 60, None, 'Pick a layout', RPC_NOW)
        self.assertEqual((a['kind'], a['summary']), ('open', 'Pick a layout'))
        self.assertEqual(spawned, [])
        a = self.sb.build_ask(path, SID_A, None)  # a detached pass already queued when the setting went off
        self.assertEqual(a['kind'], 'open')
        sb_options(self.sb, ai_ask_cards=True)
        self.sb.session_ask(SID_A, Path(path), RPC_NOW - 30, None, 'Pick a layout', RPC_NOW)
        self.assertEqual(len(spawned), 1)


if __name__ == '__main__':
    unittest.main()
