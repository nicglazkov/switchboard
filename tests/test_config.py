"""Nothing about a machine is built in: the machine list (fleet.json), the server's own addresses (server.json), the
identity the server tells each agent, the appearance settings, the status line under the gauges (notes.py),
relaunching a chat with other flags, and the terminal color schemes every client shares."""
import json, os, sys, tempfile, unittest
from pathlib import Path
from tempfile import mkdtemp
from unittest import mock
ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'server'))
import fleet, notes, server  # noqa: E402

NOW = 1_791_000_000.0


def write(p, d):
    p.parent.mkdir(parents=True, exist_ok=True)
    p.write_text(json.dumps(d))
    return p


class FleetConfigTest(unittest.TestCase):
    def setUp(self):
        self.dir = Path(mkdtemp())

    def test_first_start_seeds_from_the_hub_file_and_keeps_it(self):
        seed = write(self.dir / 'state' / 'machines.json', {'machines': [
            {'name': 'studio', 'label': 'Studio', 'target': 'me@10.0.0.6', 'os': 'mac'},
            {'name': 'lab', 'label': 'Lab', 'target': 'node:lab', 'os': 'linux'},   # linked: comes back from the hub
            {'name': 'Bad!', 'label': 'x', 'target': 'me@x'},                       # never usable
            {'name': 'tower', 'label': 'Tower', 'target': 'me@10.0.0.8', 'os': 'windows'}],
            'origins': ['http://10.0.0.5:8130']})
        cfg = fleet.FleetConfig(self.dir / 'cfg' / 'fleet.json', seed=seed)
        got = cfg.load()
        self.assertEqual([m['name'] for m in got], ['studio', 'tower'])
        self.assertTrue((self.dir / 'cfg' / 'fleet.json').exists())
        seed.write_text('{}')  # later starts read fleet.json only
        self.assertEqual([m['name'] for m in cfg.load()], ['studio', 'tower'])

    def test_nothing_to_seed_from_starts_empty(self):
        self.assertEqual(fleet.FleetConfig(self.dir / 'fleet.json', seed=self.dir / 'none.json').load(), [])
        self.assertEqual(fleet.FleetConfig(self.dir / 'f2.json').load(), [])

    def test_icons_come_from_what_a_machine_is(self):
        self.assertEqual(fleet.icon_of({'os': 'mac', 'form': 'laptop'}), 'laptop')
        self.assertEqual(fleet.icon_of({'os': 'mac', 'form': 'desktop'}), 'desktop')
        self.assertEqual(fleet.icon_of({'os': 'windows'}), 'tower')
        self.assertEqual(fleet.icon_of({'os': 'linux', 'form': 'laptop'}), 'laptop')
        self.assertEqual(fleet.icon_of({'name': 'laptop'}), 'desktop', 'never from the name')

    def test_the_poll_tells_the_agent_who_it_is(self):
        f = fleet.Fleet([{'name': 'studio', 'label': 'Studio', 'target': 'me@x', 'os': 'mac', 'root': '~/code'}])
        f.servers = ['http://10.0.0.5:8130']
        self.assertEqual(f._list_req(f.machines['studio']),
                         {'verb': 'list', 'as': 'studio', 'root': '~/code', 'servers': ['http://10.0.0.5:8130']})

    def test_what_a_machine_says_about_itself_is_kept(self):
        saved = []
        f = fleet.Fleet([{'name': 'studio', 'label': 'Studio', 'target': 'me@x'}])
        f.on_save = saved.append
        self.assertTrue(f.learn('studio', {'os': 'mac', 'device': {'os': 'mac', 'form': 'laptop'}, 'root': '/Users/you', 'root_set': False}))
        self.assertEqual(f.machines['studio'].get('form'), 'laptop')
        self.assertNotIn('root', f.machines['studio'], 'a folder the agent fell back to is not a choice')
        self.assertTrue(f.learn('studio', {'root': '/Users/you/Desktop/Claude'}), 'an older agent: the folder it uses is it')
        self.assertEqual(f.machines['studio']['root'], '/Users/you/Desktop/Claude')
        self.assertFalse(f.learn('studio', {'root': '/elsewhere', 'root_set': True}), 'a folder set in Settings stays')
        self.assertEqual(len(saved), 2)
        self.assertFalse(f.learn('ghost', {'os': 'mac'}))


class ServerConfigTest(unittest.TestCase):
    def test_seeded_from_the_old_origins(self):
        d = Path(mkdtemp())
        write(d / 'state' / 'machines.json', {'origins': ['http://10.0.0.5:8130', 'http://100.64.0.5:8130', 'http://studio:8130']})
        with mock.patch.object(server.socket, 'gethostname', lambda: 'studio.local'):
            c = server.ServerConfig(d / 'cfg', d / 'state')
        self.assertEqual(c.bind, ['10.0.0.5', '100.64.0.5'])
        self.assertEqual(c.origins, ['http://10.0.0.5:8130', 'http://100.64.0.5:8130', 'http://studio:8130'])
        for ok in ('10.0.0.5', '100.64.0.5', 'studio', 'studio.tail1234.ts.net', 'localhost', '127.0.0.1'):
            self.assertTrue(c.host_ok(ok), ok)
        for bad in ('evil.com', 'studio.evil.com', '8.8.8.8', 'local', 'ts.net', ''):
            self.assertFalse(c.host_ok(bad), bad)
        self.assertTrue((d / 'cfg' / 'server.json').exists())

    def test_a_first_start_with_nothing_uses_its_own_addresses(self):
        d = Path(mkdtemp())
        c = server.ServerConfig(d / 'cfg', d / 'state', port=8130, local=lambda: ['10.0.0.9', 'box'])
        self.assertEqual((c.bind, c.origins), (['0.0.0.0'], ['http://10.0.0.9:8130', 'http://box:8130']))
        self.assertTrue(c.host_ok('box') and c.host_ok('10.0.0.9'))


class Quiet:
    def __call__(self, target, req, timeout):
        return {'ok': True, 'sessions': [], 'replaced': [], 'os': 'mac', 'device': {'os': 'mac', 'form': 'desktop'}}


class SettingsApiTest(unittest.TestCase):
    def setUp(self):
        self.state, self.cfg = Path(mkdtemp()), Path(mkdtemp())
        self.fleet = fleet.Fleet([{'name': 'studio', 'label': 'Studio', 'target': 'me@x', 'os': 'mac'},
                                  {'name': 'tower', 'label': 'Tower', 'target': 'me@y', 'os': 'windows'}], Quiet(), clock=lambda: NOW)
        self.app = server.App(self.fleet, state_dir=self.state, web_dir=self.state, cfg_dir=self.cfg, clock=lambda: NOW)
        self.app._poll_soon = lambda *a: None

    def saved(self):
        return json.loads((self.cfg / 'fleet.json').read_text())['machines']

    def test_the_newest_build_comes_from_the_download_page(self):
        self.assertEqual(self.app.get('/api/app'), {'ok': True, 'latest': None, 'set_up': False})  # no downloads.json: nothing offered
        write(self.cfg / 'downloads.json', {'url': 'http://10.0.0.5:8131/app/'})
        self.app._latest = None
        page = {'version': '0.1.0', 'files': {'mac': {'file': 'Switchboard-mac.zip', 'build': '20261013120000', 'sha256': 'ab' * 32}}}

        class R:
            def __init__(self, b): self.b = b
            def read(self, n=-1): return self.b
            def __enter__(self): return self
            def __exit__(self, *a): return False
        with mock.patch.object(server, 'urlopen', return_value=R(json.dumps(page).encode())) as u:
            got = self.app.get('/api/app')
            self.app.get('/api/app')  # kept for a while: one look at the page
        self.assertEqual(u.call_count, 1)
        self.assertEqual(u.call_args[0][0], 'http://10.0.0.5:8131/app/latest.json')
        self.assertEqual(got, {'ok': True, 'latest': page, 'base': 'http://10.0.0.5:8131/app', 'set_up': True})

    def test_machines_can_be_renamed_reordered_added_and_removed(self):
        v = self.app.get('/api/machines')
        self.assertEqual([(m['name'], m['label'], m['icon'], m['via']) for m in v['machines']],
                         [('studio', 'Studio', 'desktop', 'ssh'), ('tower', 'Tower', 'tower', 'ssh')])
        self.app.post('/api/machines', {'op': 'rename', 'machine': 'studio', 'label': 'Office Mac'})
        self.app.post('/api/machines', {'op': 'order', 'names': ['tower', 'studio']})
        v = self.app.post('/api/machines', {'op': 'add', 'name': 'laptop', 'label': 'Laptop', 'target': 'me@10.0.0.7', 'os': 'mac'})
        self.assertEqual([m['name'] for m in v['machines']], ['tower', 'studio', 'laptop'])
        self.assertEqual([(m['name'], m['label']) for m in self.saved()], [('tower', 'Tower'), ('studio', 'Office Mac'), ('laptop', 'Laptop')])
        hub = json.loads((self.state / 'machines.json').read_text())
        self.assertEqual([m['name'] for m in hub['machines']], ['tower', 'studio', 'laptop'], 'the hub reads the same list')
        self.app.post('/api/machines', {'op': 'root', 'machine': 'laptop', 'root': '~/code'})
        self.assertEqual(self.saved()[2]['root'], '~/code')
        self.app.post('/api/machines', {'op': 'remove', 'machine': 'tower'})
        self.assertEqual([m['name'] for m in self.saved()], ['studio', 'laptop'])
        self.assertNotIn('tower', self.fleet.machines)

    def test_bad_changes_are_refused(self):
        for body, code in [({'op': 'add', 'name': 'Bad Name', 'target': 'me@x'}, 400), ({'op': 'add', 'name': 'studio', 'target': 'me@z'}, 409),
                           ({'op': 'add', 'name': 'new', 'target': '-oProxyCommand=x'}, 400), ({'op': 'add', 'name': 'new', 'target': 'node:new'}, 400),
                           ({'op': 'rename', 'machine': 'studio', 'label': ''}, 400), ({'op': 'order', 'names': ['studio']}, 400),
                           ({'op': 'nope'}, 400), ({'op': 'rename', 'machine': 'ghost', 'label': 'x'}, 400)]:
            with self.assertRaises(server.HttpError, msg=body) as e:
                self.app.post('/api/machines', body)
            self.assertEqual(e.exception.code, code, body)

    def test_appearance_syncs_through_the_server(self):
        p = self.app.get('/api/prefs')
        self.assertEqual(p['appearance'], {'theme': 'system', 'term': {'scheme': 'converge-dark', 'size': None, 'size_phone': None,
                                                                         'cursor': 'block', 'blink': True}})
        self.app.post('/api/prefs', {'appearance': {'theme': 'dark', 'term': {'scheme': 'nord', 'size': 15, 'cursor': 'bar'}}})
        self.app.post('/api/prefs', {'appearance': {'term': {'blink': False, 'scheme': 'not-a-scheme', 'size': 400}}})
        a = self.app.get('/api/prefs')['appearance']
        self.assertEqual(a, {'theme': 'dark', 'term': {'scheme': 'nord', 'size': 15, 'size_phone': None, 'cursor': 'bar', 'blink': False}})
        self.assertEqual(self.app.sessions()['prefs']['appearance'], a, 'every client gets it with the board')

    def test_the_summary_carries_the_status_line(self):
        self.fleet.poll()
        self.assertIn('note', self.app.get('/api/summary'))


class NotesTest(unittest.TestCase):
    def m(self, *sessions, **kw):
        return dict({'name': 'studio', 'label': 'Studio', 'online': True, 'updated': NOW, 'sessions': list(sessions)}, **kw)

    def s(self, **kw):
        return dict({'sid': 'a', 'title': 'Fix the lights', 'status': 'idle', 'ctx': 20_000, 'cache_left': 0, 'stale': False}, **kw)

    def test_something_that_needs_you_comes_first(self):
        n = notes.pick([self.m(self.s(status='waiting'), self.s(sid='b', status='waiting', title='Other'))],
                       {'seven_day': 99, 'week_projected': 140, 'seven_day_reset': NOW + 3600}, NOW)
        self.assertEqual((n['kind'], n['machine'], n['sid']), ('need', 'studio', 'a'))
        self.assertEqual(n['text'], 'Fix the lights is waiting for you on Studio. 1 more needs you too.')
        held = notes.pick([self.m(self.s())], {}, NOW, held=lambda m, sid: sid == 'a')
        self.assertEqual(held['text'], 'A message to Fix the lights is waiting for your OK.')

    def test_a_big_warm_chat_about_to_go_cold(self):
        n = notes.pick([self.m(self.s(ctx=300_000, cache_left=240))], {'seven_day': 10, 'week_projected': 30}, NOW)
        self.assertEqual((n['kind'], n['text']), ('cooling', 'Fix the lights goes cold in 4 min. Reply before then to keep it cheap: '
                                                  'after that, the next reply re-reads all 300k.'))
        small = notes.pick([self.m(self.s(ctx=20_000, cache_left=240))], {'seven_day': 10, 'week_projected': 30}, NOW)
        self.assertEqual(small['kind'], 'pace')

    def test_an_unusual_burn(self):
        reset = NOW + 3 * 86400  # four days into the week
        sm = {'seven_day': 40, 'week_projected': 70, 'seven_day_reset': reset, 'week_trace': [[NOW - 7200, 30], [NOW - 3700, 32]]}
        n = notes.pick([], sm, NOW)
        self.assertEqual(n['kind'], 'burn')
        self.assertTrue(n['text'].startswith('8% of the week went in the last hour'), n['text'])

    def test_plan_pace_and_quiet(self):
        n = notes.pick([], {'seven_day': 70, 'week_projected': 130, 'seven_day_reset': NOW + 2 * 86400}, NOW)
        self.assertEqual((n['kind'], n['tone']), ('week', 'bad'))
        n = notes.pick([], {'seven_day': 20, 'five_hour': 88, 'five_hour_reset': NOW + 5400, 'week_projected': 50}, NOW)
        self.assertEqual(n['text'], 'The 5-hour window is at 88%. It resets in 1.5 h.')
        self.assertEqual(notes.pick([], {'seven_day': 20, 'week_projected': 40}, NOW)['text'],
                         'On pace for 40% of the week by reset. Plenty of room.')
        self.assertIsNone(notes.pick([], {'seven_day': 20}, NOW))
        for text in (notes.pick([], {'seven_day': 70, 'week_projected': 130, 'seven_day_reset': NOW + 2 * 86400}, NOW)['text'],):
            self.assertNotIn('UltraCode', text)
            self.assertNotIn('\u2014', text)


class NotesOldNumbersTest(unittest.TestCase):
    """Pace advice never comes from an old number without saying it is old."""

    def test_fresh_numbers_read_as_now(self):
        n = notes.pick([], {'seven_day': 20, 'week_projected': 40, 'limits_updated': NOW - 600}, NOW)
        self.assertEqual(n['text'], 'On pace for 40% of the week by reset. Plenty of room.')

    def test_numbers_over_an_hour_old_say_so(self):
        n = notes.pick([], {'seven_day': 70, 'week_projected': 130, 'seven_day_reset': NOW + 2 * 86400, 'limits_updated': NOW - 3 * 3600}, NOW)
        self.assertEqual(n['kind'], 'week')
        self.assertIn('(From numbers 3 h old: they update when a session replies.)', n['text'])
        n = notes.pick([], {'seven_day': 20, 'five_hour': 88, 'five_hour_reset': NOW + 5400, 'limits_updated': NOW - 7200}, NOW)
        self.assertIn('2 h old', n['text'])

    def test_a_week_that_reset_since_gives_no_pace(self):
        n = notes.pick([], {'seven_day': 95, 'week_projected': 130, 'seven_day_reset': NOW - 60, 'limits_updated': NOW - 86400}, NOW)
        self.assertEqual(n['kind'], 'pace')
        self.assertIn('reset since the last numbers', n['text'])


class RelaunchTest(unittest.TestCase):
    SID = 'aaaaaaaa-1111-2222-3333-444444444444'

    def setUp(self):
        self.calls, self.status, self.kind = [], 'idle', 'background'
        app = self

        def transport(target, req, timeout):
            app.calls.append(req)
            if req['verb'] == 'list':
                return {'ok': True, 'replaced': [], 'sessions': [{'sid': app.SID, 'title': 'Chat', 'mtime': NOW, 'active': NOW,
                                                                  'kind': app.kind, 'status': app.status, 'bg_id': 'aaaaaaaa'}], 'features': ['relaunch']}
            if req['verb'] == 'resume':
                return {'ok': True, 'job': '0123456789ab'}
            if req['verb'] == 'job':
                return {'ok': True, 'state': 'done', 'sid': 'bbbbbbbb-1111-2222-3333-444444444444', 'bg_id': 'bbbbbbbb'}
            return {'ok': True}
        self.fleet = fleet.Fleet([{'name': 'studio', 'label': 'Studio', 'target': 'me@x', 'os': 'mac'}], transport, clock=lambda: NOW)
        self.app = server.App(self.fleet, state_dir=Path(mkdtemp()), web_dir=Path(mkdtemp()), cfg_dir=Path(mkdtemp()),
                              clock=lambda: NOW, sleep=lambda s: None, poll_every=0)
        self.app._refresh_soon = lambda *a, **k: None
        self.fleet.poll()

    def body(self, **kw):
        return dict({'machine': 'studio', 'sid': self.SID, 'model': 'sonnet', 'effort': 'high',
                     'flags': [['--chrome'], ['--permission-mode', 'plan']]}, **kw)

    def wait(self, jid):
        for _ in range(200):
            j = self.app.job('server', jid)
            if j['state'] in ('done', 'error'):
                return j
            import time
            time.sleep(0.01)
        self.fail('job never finished')

    def test_relaunch_resumes_with_exactly_these_flags(self):
        r = self.app.post('/api/relaunch', self.body())
        j = self.wait(r['job'])
        self.assertEqual((j['state'], j['kind']), ('done', 'relaunch'))
        req = next(c for c in self.calls if c['verb'] == 'resume')
        self.assertEqual(req, {'verb': 'resume', 'sid': self.SID, 'relaunch': True, 'model': 'sonnet', 'effort': 'high',
                               'flags': [['--permission-mode', 'plan'], ['--chrome']]})

    def test_mid_reply_it_asks_then_waits_or_goes_now(self):
        self.status = 'busy'
        self.fleet.poll()
        r = self.app.post('/api/relaunch', self.body())
        self.assertEqual((r['ok'], r['need']), (False, 'busy'))
        self.assertFalse([c for c in self.calls if c['verb'] == 'resume'], 'never killed without asking')
        r = self.app.post('/api/relaunch', self.body(when='now'))
        self.assertEqual(self.wait(r['job'])['state'], 'done')

    def test_a_chat_open_in_someone_elses_terminal_is_left_alone(self):
        self.kind = 'interactive'
        self.fleet.poll()
        r = self.app.post('/api/relaunch', self.body())
        self.assertEqual((r['ok'], r['need']), (False, 'terminal'))
        self.assertIn('claude --resume ' + self.SID, r['command'])

    def test_bad_flags_are_refused(self):
        with self.assertRaises(server.HttpError):
            self.app.post('/api/relaunch', self.body(flags=[['--print']]))
        with self.assertRaises(server.HttpError):
            self.app.post('/api/relaunch', self.body(model='gpt'))


class FlagsDrawerTest(unittest.TestCase):
    S = {'kind': 'background', 'status': 'idle', 'now': {'model': 'opus', 'effort': 'high', 'perm': 'default', 'chrome': False},
         'launch': {'flags': [['--remote-control'], ['--add-dir', 'proj'], ['--verbose']], 'other': ['--mystery'], 'from': 'roster'}}

    def test_active_now_and_the_header_line(self):
        import flags
        v = flags.view(self.S)
        self.assertEqual([(x['name'], x['text'], x['how']) for x in v['active']], [
            ('--model', 'Opus', 'live'), ('--effort', 'High', 'live'), ('--permission-mode', 'Asks first', 'relaunch'),
            ('--add-dir', 'proj', 'live'), ('--verbose', 'On', 'relaunch'), ('--remote-control', 'On', 'locked')])
        self.assertEqual(v['summary'], 'Opus · High · Asks first · 2 more')
        self.assertEqual(v['other'], ['--mystery'])

    def test_live_changes_are_typed_and_the_rest_relaunch(self):
        import flags
        p = flags.plan(self.S, {'model': 'sonnet', 'perm': 'plan', 'flags': [['--add-dir', 'proj'], ['--add-dir', 'docs'], ['--verbose']]}, '/r')
        self.assertFalse(p['relaunch'])
        self.assertEqual(p['cmds'], ['/model sonnet', '/plan', '/add-dir /r/docs'])
        p = flags.plan(self.S, {'flags': [['--add-dir', 'proj']], 'chrome': True})
        self.assertTrue(p['relaunch'])
        self.assertEqual(p['cmds'], [], 'one relaunch carries every change')
        self.assertEqual([(c['name'], c['how']) for c in p['changes']], [('--chrome', 'relaunch'), ('--verbose', 'relaunch')])
        self.assertEqual(p['body'], {'model': 'opus', 'effort': 'high', 'flags': [['--add-dir', 'proj'], ['--chrome']]})
        self.assertEqual(flags.plan(self.S, {'flags': [['--add-dir', 'proj'], ['--verbose']]})['changes'], [], 'nothing changed')
        stopped = dict(self.S, status='stopped')
        self.assertEqual({c['how'] for c in flags.plan(stopped, {'model': 'sonnet', 'flags': []})['changes']}, {'relaunch'})
        with self.assertRaises(flags.FlagError):
            flags.plan(self.S, {'flags': [['--print']]})
        with self.assertRaises(flags.FlagError):
            flags.plan(self.S, {'other': '--permission-mode plan'})
        p = flags.plan(self.S, {'flags': [['--add-dir', 'proj'], ['--verbose']], 'other': '--brief'})
        self.assertEqual([(c['name'], c['to'], c['how']) for c in p['changes']], [('--brief', 'On', 'relaunch')])
        self.assertEqual(flags.view(self.S)['short'], 'Opus \u00b7 High')


class AgentIdentityTest(unittest.TestCase):
    def setUp(self):
        from tests.helpers import fresh_sb
        self.tmp = Path(tempfile.mkdtemp())
        self.sb = fresh_sb(self.tmp)
        for k in ('SB_ROOT', 'SB_SERVER', 'SB_USER_HOME'):
            self.addCleanup(os.environ.__setitem__, k, os.environ[k]) if k in os.environ else self.addCleanup(os.environ.pop, k, None)
            os.environ.pop(k, None)
        os.environ['SB_USER_HOME'] = str(self.tmp)  # a root the server sends must be inside the home folder

    def test_the_server_says_who_and_where(self):
        sb = self.sb
        self.assertFalse(sb.root_is_set())
        self.assertEqual(sb.servers(), ())
        sb.remember_identity({'verb': 'list', 'as': 'studio', 'root': str(self.tmp), 'servers': ['http://10.0.0.5:8130', 'javascript:x']})
        self.assertEqual((sb.machine_name(), sb.machine_root(), sb.servers()), ('studio', self.tmp.resolve(), ('http://10.0.0.5:8130',)))
        self.assertTrue(sb.root_is_set())
        sb.remember_identity({'verb': 'list', 'as': '../bad', 'root': 'x' * 500})
        self.assertEqual((sb.machine_name(), sb.machine_root()), ('studio', self.tmp.resolve()), 'bad values are ignored')
        os.environ['SB_SERVER'] = 'http://127.0.0.1:9/'
        self.assertEqual(sb.servers(), ('http://127.0.0.1:9',))

    def test_flags_off_a_real_command_line(self):
        sb = self.sb
        (self.tmp / 'proj').mkdir()
        os.environ['SB_ROOT'] = str(self.tmp)
        args = ['/usr/local/bin/node', '/opt/claude/cli.js', '--model', 'sonnet', '--permission-mode=plan', '--chrome',
                '--add-dir', str(self.tmp / 'proj'), '--add-dir', '/etc', '--name', 'Chat', '--resume', 'abc', '--verbose',
                '--mystery-flag', 'please use --effort max']
        flags, other = sb.flag_list(args)
        self.assertEqual(flags, [['--model', 'sonnet'], ['--permission-mode', 'plan'], ['--chrome'], ['--add-dir', 'proj'], ['--verbose']])
        self.assertEqual(other, ['--add-dir', '--mystery-flag'], 'a folder outside the root and flags it does not set are named only')
        bg = ['--bg', '--remote-control', '--name', 'x', '--effort', 'xhigh', '--agent', 'reviewer']
        self.assertEqual(sb.flag_list(bg)[0], [['--remote-control'], ['--effort', 'xhigh'], ['--agent', 'reviewer']])

    def test_a_list_reply_says_what_the_machine_is(self):
        r = self.sb.rpc({'verb': 'list', 'as': 'studio'}, NOW, agents=[])
        self.assertEqual(r['machine'], 'studio')
        self.assertIn(r['os'], ('mac', 'linux', 'windows'))
        self.assertIn(r['device']['form'], ('laptop', 'desktop', None))
        self.assertIs(r['root_set'], False)


class SchemesTest(unittest.TestCase):
    def test_one_list_for_every_client(self):
        core = (ROOT / 'native' / 'crates' / 'sb-core' / 'src' / 'term' / 'schemes.json').read_text()
        self.assertFalse((ROOT / 'server' / 'web' / 'term-schemes.json').exists(), 'one copy, in sb-core')
        import server
        d = json.loads(core)
        self.assertEqual(server._scheme_ids(), {s['id'] for s in d['schemes']}, 'the server checks against the same file')
        ids = [s['id'] for s in d['schemes']]
        self.assertGreaterEqual(len(ids), 6)
        self.assertEqual(len(ids), len(set(ids)))
        self.assertTrue(any(s['dark'] for s in d['schemes']) and any(not s['dark'] for s in d['schemes']))
        self.assertIn(d['match']['dark'], ids)
        self.assertIn(d['match']['light'], ids)
        for s in d['schemes']:
            self.assertEqual(len(s['ansi']), 16, s['id'])
        self.assertEqual(set(server.SCHEME_IDS), set(ids), 'the server accepts exactly these')


if __name__ == '__main__':
    unittest.main()
