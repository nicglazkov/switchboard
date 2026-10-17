"""Auto tools (tool library spec, "Auto mode"): library tools picked at launch, the catalog a chat gets at its start, the
library MCP tool's view of this chat, and the server's auto flag, when hint, update route and session rows."""
import json, os, sys, time, unittest
from pathlib import Path
from unittest import mock

from tests import test_library as tl  # its test classes are not imported by name, so they run once, in their own file
from tests.test_library import make_plugin
from tests.test_rpc import NOW, SID_A, SID_B

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'server'))
import server  # noqa: E402

SID_C = 'cccccccc-1111-2222-3333-444444444444'
SID_D = 'dddddddd-1111-2222-3333-444444444444'
META = {'alpha': {'when': 'Use it to say hello to people.', 'kinds': ['skill', 'mcp'], 'latest': 1},
        'keyed': {'when': 'Fetch blocked pages.', 'kinds': ['mcp'], 'latest': 1}}


class AgentAutoTest(tl.InstallTest):
    def setUp(self):
        super().setUp()
        self.put(make_plugin(self.tmp / 'pa', 'alpha'), 'alpha', 1)
        self.put(make_plugin(self.tmp / 'pk', 'keyed', secrets=['API_KEY'], skill=False), 'keyed', 1)
        self.hooked = []

    def starting_runner(self, sid):
        """A launch during which the new chat's SessionStart hook runs, as Claude Code would."""
        def run(argv, cwd, env):
            self.launched.append({'argv': argv, 'cwd': cwd, 'env': env})
            self.hooked.append(self.sb.handle_hook({'hook_event_name': 'SessionStart', 'source': 'startup', 'session_id': sid,
                                                    'cwd': str(cwd)}, NOW + 6, None))
            return 0, '  claude stop 3022c163    stop this session\n'
        return run

    def start(self, **req):
        r = self.sb.rpc(dict({'verb': 'start', 'folder': 'proj', 'prompt': 'go'}, **req), NOW, agents=[])
        self.assertTrue(r['ok'], r)
        return r['job']

    def result(self, jid):
        return self.sb._jget(self.sb.paths_q()['queue'] / f'{jid}.result.json')

    def test_auto_attaches_only_tools_whose_keys_are_here_and_says_so(self):
        jid = self.start(tools=['alpha', 'keyed'], tools_mode='auto', tools_meta=META)
        with mock.patch.object(self.sb, 'bg_session_id', lambda bg: SID_B):
            self.drain(runner=self.starting_runner(SID_B))
        argv = self.launched[0]['argv']
        self.assertEqual([argv[i + 1] for i, a in enumerate(argv) if a == '--plugin-dir'], [str(self.sb.lib_root() / 'alpha' / '1')])
        res = self.result(jid)
        self.assertEqual(res['tools'], ['alpha'])
        self.assertEqual(res['skipped_tools'], [{'tool': 'keyed', 'missing': ['API_KEY']}])
        rec = self.sb.chat_tools_rec(SID_B)
        self.assertEqual((rec['mode'], rec['tools'], rec['candidates']), ('auto', ['alpha'], ['alpha', 'keyed']))
        ctx = self.hooked[0]['hookSpecificOutput']['additionalContext']
        self.assertEqual(ctx.splitlines(), [
            'Switchboard tools in this chat:',
            '- alpha: Use it to say hello to people. (skill + MCP)',
            'Tools that need a key on this machine: keyed (API_KEY).',
            'Call the switchboard `library` tool to see new or updated tools; a new one needs a relaunch to load '
            '(ask the user, the conversation carries over).'])
        self.assertEqual(list((self.sb.paths()['state'] / 'tools_pending').glob('*.json')), [], 'taken by the hook')
        # the key is set now: the agent's own resume picks again from the same candidates
        self.keys.set('keyed', 'API_KEY', 'v')
        self.transcript(SID_B, str(self.root / 'proj'), NOW - 100)
        self.job({'verb': 'resume', 'sid': SID_B})
        with mock.patch.object(self.sb, 'bg_session_id', lambda bg: SID_C):
            self.drain()
        self.assertEqual(self.launched[-1]['argv'].count('--plugin-dir'), 2)
        self.assertEqual(self.sb.chat_tools_rec(SID_C)['tools'], ['alpha', 'keyed'])
        self.assertEqual(list((self.sb.paths()['state'] / 'tools_pending').glob('*.json')), [], 'no hook ran: cleaned up')

    def test_pick_mode_is_unchanged_and_gets_a_catalog_from_the_installed_copy(self):
        jid = self.start(tools=['alpha', 'keyed'])
        with mock.patch.object(self.sb, 'bg_session_id', lambda bg: SID_B):
            self.drain(runner=self.starting_runner(SID_B))
        self.assertEqual(self.launched[0]['argv'].count('--plugin-dir'), 2)
        self.assertNotIn('skipped_tools', self.result(jid))
        self.assertEqual(self.sb.chat_tools_rec(SID_B)['mode'], 'pick')
        ctx = self.hooked[0]['hookSpecificOutput']['additionalContext']
        self.assertIn('- alpha: Demo tool (skill + MCP)', ctx)
        self.assertIn('- keyed: Demo tool (MCP)', ctx)

    def test_a_chat_without_library_tools_gets_no_catalog(self):
        self.assertIsNone(self.sb.handle_hook({'hook_event_name': 'SessionStart', 'source': 'startup', 'session_id': SID_D,
                                               'cwd': str(self.root / 'proj')}, NOW, None))
        self.start()
        with mock.patch.object(self.sb, 'bg_session_id', lambda bg: SID_B):
            self.drain(runner=self.starting_runner(SID_B))
        self.assertIsNone(self.hooked[0])

    def test_a_record_for_another_folder_or_too_old_is_not_taken(self):
        rec = self.sb.tools_rec(['alpha'], {'alpha': 1}, 'auto', [], META)
        self.sb.tools_pending_put(str(self.root / 'proj'), rec, NOW - 400)
        self.sb.tools_pending_put(str(self.root / 'other'), rec, NOW)
        self.assertIsNone(self.sb.tools_pending_take(str(self.root / 'proj'), NOW))
        self.assertIsNotNone(self.sb.tools_pending_take(str(self.root / 'other'), NOW))
        self.assertIsNone(self.sb.tools_pending_take(str(self.root / 'other'), NOW), 'one chat gets each')

    def test_clear_keeps_the_catalog_of_the_same_process(self):
        self.sb._jput(self.sb.paths()['state'] / f'{SID_A}.tools', self.sb.tools_rec(['alpha'], {'alpha': 1}, 'pick', [], META))
        self.sb.seen_put(4242, SID_A, '/t', NOW)
        out = self.sb.handle_hook({'hook_event_name': 'SessionStart', 'source': 'clear', 'session_id': SID_B, 'cwd': '/x'}, NOW, 4242)
        self.assertIn('- alpha:', out['hookSpecificOutput']['additionalContext'])
        self.assertEqual(self.sb.chat_tools_rec(SID_B)['tools'], ['alpha'])

    def test_the_catalog_stays_short(self):
        tools = [f'tool-{i:02d}' for i in range(40)]
        meta = {t: {'when': 'x' * 400, 'kinds': ['skill']} for t in tools}
        text = self.sb.tools_catalog(self.sb.tools_rec(tools, {}, 'auto', [{'tool': 'k', 'missing': ['A_KEY']}], meta))
        self.assertLessEqual(len(text), self.sb.CATALOG_MAX)
        self.assertIn('more (see library)', text)
        self.assertIn('Tools that need a key on this machine: k (A_KEY).', text)
        self.assertTrue(text.endswith('the conversation carries over).'))
        self.assertTrue(all(len(l) < 200 for l in text.splitlines()))
        self.assertIsNone(self.sb.tools_catalog(self.sb.tools_rec([], {}, 'auto', [], {})))

    def test_the_library_tool_shows_this_chat_and_newer_versions(self):
        self.sb._jput(self.sb.paths()['state'] / f'{SID_A}.tools',
                      self.sb.tools_rec(['alpha'], {'alpha': 1}, 'auto', [{'tool': 'keyed', 'missing': ['API_KEY']}], META, ['alpha', 'keyed']))
        lib = {'tools': [{'name': 'alpha', 'kinds': ['skill'], 'latest': 2, 'when': 'Say hello.', 'description': 'Demo', 'auto': True},
                         {'name': 'keyed', 'kinds': ['mcp'], 'latest': 1, 'when': '', 'description': 'Fetch', 'auto': False}],
               'defaults': [], 'machines': {'desk': {'alpha': {'connected': True}}}}
        out, bad = self.sb.mcp_tool('library', {}, call=lambda m, p, b=None: lib, me='desk', sid=SID_A)
        self.assertFalse(bad)
        a, k = out['tools']
        self.assertEqual((a['name'], a['version'], a['when'], a['in_this_chat'], a['on_this_machine']), ('alpha', 2, 'Say hello.', True, True))
        self.assertIn('v2 is out; this chat loaded v1', a['newer'])
        self.assertEqual((k['in_this_chat'], k['when'], k['auto']), (False, 'Fetch', False))
        self.assertIn('relaunch', k['note'])
        self.assertEqual(out['this_chat']['mode'], 'auto')
        self.assertEqual(out['this_chat']['need_keys'], [{'tool': 'keyed', 'missing': ['API_KEY']}])
        self.assertIn('tools_auto', self.sb.AGENT_FEATURES)

    def test_publish_tool_sends_when(self):
        sent = []

        def call(method, path, body=None):
            sent.append(body)
            return {'ok': True, 'tool': 'demo', 'version': 1} if body.get('done') else {'ok': True}
        out, bad = self.sb.mcp_tool('publish_tool', {'path': str(self.tmp / 'pa'), 'when': ' Use it\nfor hellos. '}, call=call, me='desk')
        self.assertFalse(bad, out)
        self.assertEqual(sent[-1]['done']['when'], 'Use it for hellos.')
        out, bad = self.sb.mcp_tool('publish_tool', {'path': str(self.tmp / 'pa'), 'when': 3}, call=call, me='desk')
        self.assertTrue(bad)


class AutoFleet(tl.Fleet):
    """The machine attaches what it was sent except tools named in `no_key` (what the agent's auto_resolve does)."""
    no_key = ('keyed',)

    def __call__(self, target, req, timeout):
        r = super().__call__(target, req, timeout)
        if req['verb'] in ('start', 'resume', 'fresh') and req.get('tools') is not None:
            self.last = req
        if req['verb'] == 'job' and getattr(self, 'last', None) is not None:
            r = dict(r, tools=[t for t in self.last['tools'] if not (self.last.get('tools_mode') == 'auto' and t in self.no_key)])
        return r


class ServerAutoTest(tl.ServerLibraryTest):
    def setUp(self):
        super().setUp()
        feats = self.f.feats
        self.f.__class__ = AutoFleet
        self.f.feats = feats

    def rows(self):
        return {s['sid']: s for m in self.app.sessions()['machines'] if m['name'] == 'desk' for s in m['sessions'] + (m.get('replaced') or [])}

    def test_update_auto_and_when(self):
        self.publish(make_plugin(self.tmp / 'a', 'alpha'), 'alpha')
        t = self.app.get('/api/library')['tools'][0]
        self.assertEqual((t['auto'], t['when']), (True, ''))
        r = self.app.post('/api/library/update', {'tool': 'alpha', 'auto': False, 'when': '  Use it   for hellos. '})
        self.assertEqual((r['auto'], r['when']), (False, 'Use it for hellos.'))
        t = self.app.get('/api/library')['tools'][0]
        self.assertEqual((t['auto'], t['when']), (False, 'Use it for hellos.'))
        for body in ({'tool': 'alpha'}, {'tool': 'alpha', 'auto': 'no'}, {'tool': 'alpha', 'when': 'x' * 301},
                     {'tool': 'alpha', 'when': 'a\nb'}, {'tool': 'alpha', 'color': 'red'}, {'tool': 'nope', 'auto': True}):
            with self.assertRaises(server.HttpError, msg=body):
                self.app.post('/api/library/update', body)
        # a new version keeps the settings; a publish can set the hint
        p = make_plugin(self.tmp / 'a2', 'alpha', mcp=False)
        data, _, _ = self.sb.pack_folder(p, 'alpha')
        import base64, hashlib
        self.app.post('/api/library/publish', {'upload': os.urandom(16).hex(), 'offset': 0, 'data': base64.b64encode(data).decode(),
                                               'done': {'tool': 'alpha', 'sha256': hashlib.sha256(data).hexdigest(), 'when': 'Greets.'}})
        t = self.app.get('/api/library')['tools'][0]
        self.assertEqual((t['latest'], t['auto'], t['when']), (2, False, 'Greets.'))

    def test_auto_start_picks_from_the_library_and_resumes_pick_again(self):
        self.publish(make_plugin(self.tmp / 'a', 'alpha'), 'alpha')
        self.publish(make_plugin(self.tmp / 'k', 'keyed', secrets=['API_KEY'], skill=False), 'keyed')
        self.publish(make_plugin(self.tmp / 'o', 'off', mcp=False), 'off')
        self.app.post('/api/library/update', {'tool': 'off', 'auto': False, 'when': 'Rarely.'})
        self.app.post('/api/library/update', {'tool': 'alpha', 'when': 'Say hello.'})
        r = self.app.post('/api/new', {'machine': 'desk', 'folder': 'x', 'prompt': 'go', 'tools': 'auto'}, 'SwitchboardDesktop/1')
        self.assertEqual(self.wait(r['job'])['state'], 'done')
        start = self.f.verbs('desk', 'start')[-1]
        self.assertEqual((start['tools'], start['tools_mode']), (['alpha', 'keyed'], 'auto'))
        self.assertEqual(start['tools_meta']['alpha'], {'when': 'Say hello.', 'kinds': ['skill', 'mcp'], 'latest': 1})
        self.assertEqual(start['tools_meta']['keyed']['when'], 'Demo tool')  # no hint: the description
        self.assertEqual(self.chat(f'desk/{SID_C}'), ['alpha'])  # what the machine attached
        self.assertEqual(self.lib.d['modes'][f'desk/{SID_C}'], 'auto')
        self.app.fleet.state['desk']['sessions'].append({'sid': SID_C, 'title': 't', 'kind': 'background', 'status': 'stopped',
                                                         'mtime': NOW, 'ctx': 1000, 'cold': False})
        self.assertEqual((self.rows()[SID_C]['tools'], self.rows()[SID_C]['tools_mode']), (['alpha'], 'auto'))
        # a tool published since: the next resume of an auto chat gets it
        self.publish(make_plugin(self.tmp / 'g', 'gamma', mcp=False), 'gamma')
        r = self.app.post('/api/action', {'machine': 'desk', 'verb': 'resume', 'sid': SID_C})
        self.wait(r['job'])
        resume = self.f.verbs('desk', 'resume')[-1]
        self.assertEqual((resume['tools'], resume['tools_mode']), (['alpha', 'gamma', 'keyed'], 'auto'))
        # picked by hand: no longer auto
        self.app.post('/api/library/attach', {'machine': 'desk', 'sid': SID_A, 'tools': ['alpha']})
        self.assertEqual(self.rows()[SID_A]['tools_mode'], 'pick')

    def test_old_clients_and_old_machines(self):
        self.publish(make_plugin(self.tmp / 'a', 'alpha'), 'alpha')
        r = self.app.post('/api/new', {'machine': 'desk', 'folder': 'x', 'prompt': 'go'}, 'SwitchboardDesktop/1')
        self.wait(r['job'])
        start = self.f.verbs('desk', 'start')[-1]
        self.assertEqual(start['tools'], [])  # nothing asked, no defaults: as before
        self.assertNotIn('tools_mode', start)
        r = self.app.post('/api/new', {'machine': 'old', 'folder': 'x', 'tools': 'auto'}, 'SwitchboardDesktop/1')
        self.assertEqual(self.wait(r['job'])['state'], 'done')  # an older machine: auto is nothing, not an error
        self.assertNotIn('tools', self.f.verbs('old', 'start')[-1])
        with self.assertRaises(server.HttpError):
            self.app.post('/api/new', {'machine': 'desk', 'folder': 'x', 'tools': 'everything'})
