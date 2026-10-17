"""Switchboard v2, second round: flags for a new session (and the Chrome machine), live updates (pings, the stream,
widget pushes), and the session browser (index, search, summary, rename)."""
import http.client, json, os, sys, threading, time, unittest
from pathlib import Path
from tempfile import mkdtemp
from tests.helpers import user, assistant, entry, write_transcript
from tests.test_rpc import RpcBase, NOW, SID_A, SID_B, _iso

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'server'))
import fleet, server, flags as flags_mod  # noqa: E402
from tests.test_server import MACH  # noqa: E402

INJECTIONS = ['--append-system-prompt "x\n--dangerously-skip-permissions"', '--add-dir ../../etc', '--add-dir /etc',
              '--add-dir ~/.ssh', '--add-dir C:\\Windows', '--dangerously-skip-permissions', '--settings evil.json',
              '-p', '--print', '--mcp-config x.json', '--model opus;rm', '--model "opus --chrome"', '--effort max|whoami',
              '--agent ../../x', '--worktree "a b"', '--permission-mode yolo', "--allowed-tools 'Bash(rm -rf ~)'",
              '--append-system-prompt "a & calc"', '--append-system-prompt "%PATH%"', '--append-system-prompt "a > b"',
              'rm -rf /', '--chrome=yes', '--add-dir', '--chrome --no-chrome', '`id`', '--resume abc', '--bg']


class FlagsServerTest(unittest.TestCase):
    def test_chips_and_other_flags(self):
        r = flags_mod.parse([['--chrome'], ['--permission-mode', 'plan'], ['--add-dir', 'proj/sub']],
                            "--model sonnet --effort=high --append-system-prompt 'Be brief.' -w feat-1 --allowedTools 'Bash(git *)'")
        self.assertEqual(r, {'flags': [['--permission-mode', 'plan'], ['--add-dir', 'proj/sub'], ['--append-system-prompt', 'Be brief.'],
                                       ['--worktree', 'feat-1'], ['--allowed-tools', 'Bash(git *)']],
                             'chrome': True, 'model': 'sonnet', 'effort': 'high'})
        self.assertEqual(flags_mod.command_line(r['flags'][:3]), "--permission-mode plan --add-dir proj/sub --append-system-prompt 'Be brief.'")
        self.assertEqual(flags_mod.parse(None, ''), {'flags': [], 'chrome': False, 'model': None, 'effort': None})

    def test_injection_attempts_are_refused(self):
        for bad in INJECTIONS:
            with self.subTest(bad=bad), self.assertRaises(flags_mod.FlagError):
                flags_mod.parse([], bad)
        for bad in [[['--add-dir', '../x']], [['--evil']], [['--chrome', 'x']], [['--agent']], 'x', [['--add-dir', 'a'] * 3],
                    [['--add-dir', f'd{i}'] for i in range(9)], [['--brief']] * 2, [[1]], [['--model', 'gpt']]]:
            with self.subTest(bad=bad), self.assertRaises(flags_mod.FlagError):
                flags_mod.parse(bad)
        with self.assertRaises(flags_mod.FlagError):
            flags_mod.parse([], 'x' * 5000)


class FlagsAgentTest(RpcBase):
    def test_same_list_as_the_server(self):
        self.assertEqual(self.sb.START_FLAGS, flags_mod.START_FLAGS)
        self.assertEqual((self.sb.FLAG_REPEAT, self.sb.FLAGS_MAX), (flags_mod.FLAG_REPEAT, flags_mod.FLAGS_MAX))

    def test_check_flags(self):
        got = self.sb.check_flags([['--permission-mode', 'plan'], ['--add-dir', 'proj'], ['--brief'], ['--allowed-tools', 'Bash(git *) Read']],
                                  self.root, known={'--permission-mode', '--add-dir', '--brief', '--allowed-tools'})
        self.assertEqual(got, ['--permission-mode=plan', f'--add-dir={(self.root / "proj").resolve()}', '--brief',
                               '--allowed-tools=Bash(git *) Read'])
        for bad in [[['--add-dir', '../x']], [['--add-dir', 'nope']], [['--dangerously-skip-permissions']], [['--model', 'opus']],
                    [['--chrome'], ['--no-chrome']], [['--brief', 'x']], [['--agent', 'a;b']], 'x', [['--brief']] * 2]:
            with self.subTest(bad=bad), self.assertRaises(ValueError):
                self.sb.check_flags(bad, self.root, known=None)
        with self.assertRaises(ValueError):  # this machine's Claude Code does not have it
            self.sb.check_flags([['--brief']], self.root, known={'--chrome'})

    def test_launch_puts_flags_before_the_message(self):
        argv = self.sb.launch_argv({'chrome': True, 'flags': ['--permission-mode=plan', '--add-dir=/r/p', '--evil'],
                                    'prompt': 'do it'}, exe='claude')
        self.assertEqual(argv, ['claude', '--bg', '--remote-control', '--chrome', '--permission-mode=plan', '--add-dir=/r/p', 'do it'])

    def test_start_checks_and_queues_flags(self):
        self.sb.claude_flags = lambda refresh=False, runner=None: {'--chrome', '--permission-mode', '--add-dir'}
        r = self.sb.rpc({'verb': 'start', 'folder': 'proj', 'flags': [['--evil']]}, NOW, agents=[])
        self.assertFalse(r['ok'])
        r = self.sb.rpc({'verb': 'start', 'folder': 'proj', 'flags': [['--chrome'], ['--permission-mode', 'plan']]}, NOW, agents=[])
        self.assertTrue(r['ok'], r)
        job = json.loads(next(self.sb.paths_q()['queue'].glob('*.json')).read_text())
        self.assertEqual((job['chrome'], job['flags']), (True, ['--permission-mode=plan']))

    def test_help_flags_are_read_and_cached(self):
        exe = self.tmp / 'claude'
        exe.write_text('#!/bin/sh\n')
        self.sb._claude_exe = lambda: str(exe)
        text = 'Options:\n  --chrome  Enable\n  --model <m>\n  --add-dir <dirs...>\n  see also foo--bar'
        self.assertEqual(self.sb.claude_flags(runner=lambda: text), {'--chrome', '--model', '--add-dir'})
        self.assertEqual(self.sb.claude_flags(runner=lambda: 1 / 0), {'--chrome', '--model', '--add-dir'}, 'cached')
        self.assertIsNone(self.sb.claude_flags(refresh=True, runner=lambda: 'not a help page'))

    def test_list_says_what_this_agent_can_do(self):
        r = self.sb.rpc({'verb': 'list'}, NOW, agents=[])
        self.assertTrue({'flags', 'ping', 'browse', 'rename', 'summary'} <= set(r['features']))


class PingAgentTest(RpcBase):
    def test_hooks_ping_the_server(self):
        sent = []
        self.assertTrue(self.sb.hook_ping({'hook_event_name': 'UserPromptSubmit', 'session_id': SID_A}, {}, NOW, ping=sent.append))
        self.assertFalse(self.sb.hook_ping({'hook_event_name': 'UserPromptSubmit', 'session_id': SID_A}, {'decision': 'block'}, NOW,
                                           ping=sent.append), 'the guard held it: it says so itself')
        self.sb.hook_ping({'hook_event_name': 'Notification', 'session_id': SID_A, 'notification_type': 'permission_prompt'}, {}, NOW,
                          ping=sent.append)
        self.assertFalse(self.sb.hook_ping({'hook_event_name': 'PreToolUse', 'session_id': SID_A}, {}, NOW, ping=sent.append))
        self.assertEqual([(e['why'], e.get('kind')) for e in sent], [('prompt', None), ('wait', 'permission_prompt')])
        os.environ['SB_INTERNAL'] = '1'
        try:
            self.assertFalse(self.sb.hook_ping({'hook_event_name': 'Stop', 'session_id': SID_A}, {}, NOW, ping=sent.append))
        finally:
            os.environ.pop('SB_INTERNAL')

    def test_status_line_pings_on_real_changes_once_a_second(self):
        sent = []
        data = lambda ctx, model='Opus': {'session_id': SID_B, 'model': {'display_name': model},
                                          'context_window': {'current_usage': {'input_tokens': ctx}}}
        self.sb.ctx_from_status = lambda d: d['context_window']['current_usage']['input_tokens']
        self.assertTrue(self.sb.status_ping(data(50_000), None, NOW, ping=sent.append))
        self.assertFalse(self.sb.status_ping(data(50_500), {'ctx_tokens': 50_000, 'model': 'Opus'}, NOW + 2, ping=sent.append))
        self.assertFalse(self.sb.status_ping(data(60_000), {'ctx_tokens': 50_000, 'model': 'Opus'}, NOW + 0.5, ping=sent.append))
        self.assertTrue(self.sb.status_ping(data(60_000), {'ctx_tokens': 50_000, 'model': 'Opus'}, NOW + 1.5, ping=sent.append))
        self.assertEqual([e['ctx'] for e in sent], [50_000, 60_000])

    def test_ping_server_returns_on_the_first_answer(self):
        seen = []
        t0 = time.time()
        ok = self.sb.ping_server({'event': 'ping'}, wait=2, servers=['a', 'b'],
                                 send=lambda base: (time.sleep(5) if base == 'b' else seen.append(base)))
        self.assertTrue(ok)
        self.assertLess(time.time() - t0, 1)
        self.assertFalse(self.sb.ping_server({'event': 'ping'}, wait=0.2, servers=['x'], send=lambda b: 1 / 0))


class BrowseAgentTest(RpcBase):
    def chat(self, sid, mtime, title='Old chat', prompts=('first thing I asked', 'the last thing'), entrypoint='cli'):
        rows = [entry('bridge-session', '2026-01-01T00:00:00Z', bridgeSessionId='cse_X1', sessionId=sid)]
        for i, p in enumerate(prompts):
            rows.append(dict(user(_iso(mtime - 100 + i), p), cwd=str(self.root / 'proj'), entrypoint=entrypoint))
            rows.append(dict(assistant(_iso(mtime - 99 + i), f'answer {i}'), cwd=str(self.root / 'proj')))
        rows.append({'type': 'ai-title', 'aiTitle': title, 'sessionId': sid})
        p = write_transcript(self.cdir / 'projects' / 'p1', rows, name=f'{sid}.jsonl')
        os.utime(p, (mtime, mtime))
        return p

    def test_every_chat_ever_and_only_what_changed(self):
        self.chat(SID_A, NOW - 400 * 86400, title='Ancient')
        self.chat(SID_B, NOW - 60, title='Recent')
        script = 'cccccccc-1111-2222-3333-444444444444'
        self.chat(script, NOW - 60, entrypoint='sdk-py')
        r = self.sb.rpc({'verb': 'browse'}, NOW, agents=[])
        self.assertEqual({x['title'] for x in r['rows']}, {'Ancient', 'Recent'}, 'a year-old chat is there; scripts are not')
        a = next(x for x in r['rows'] if x['sid'] == SID_A)
        self.assertEqual((a['folder'], a['first_prompt'], a['last_prompt'], a['url']),
                         ('proj', 'first thing I asked', 'the last thing', 'https://claude.ai/code/session_X1'))
        self.assertEqual(set(r['all']), {SID_A, SID_B})
        r2 = self.sb.rpc({'verb': 'browse', 'since': NOW - 3600}, NOW, agents=[])
        self.assertEqual([x['sid'] for x in r2['rows']], [SID_B])
        self.sb.browse_meta = lambda f: 1 / 0
        r3 = self.sb.rpc({'verb': 'browse'}, NOW, agents=[])
        self.assertEqual(len(r3['rows']), 2, 'unchanged files come from the cache')

    def test_summary_and_rename(self):
        p = self.chat(SID_A, NOW - 86400, title='Topic')
        r = self.sb.rpc({'verb': 'summary', 'sid': SID_A}, NOW, agents=[])
        self.assertEqual((r['first']['text'], [u['text'] for u in r['recent']], r['answer']),
                         ('first thing I asked', ['first thing I asked', 'the last thing'], 'answer 1'))
        r = self.sb.rpc({'verb': 'rename', 'sid': SID_A, 'name': 'Better name'}, NOW, agents=[])
        self.assertEqual((r['ok'], r['in_transcript']), (True, True))
        self.assertIn('"customTitle": "Better name"', Path(p).read_text().splitlines()[-1])
        os.utime(p, (NOW - 60, NOW - 60))
        self.assertEqual(self.sb.rpc({'verb': 'list'}, NOW, agents=[])['sessions'][0]['title'], 'Better name')
        for bad in [' x', 'a/b', 'x' * 65, '$(id)', None]:
            self.assertFalse(self.sb.rpc({'verb': 'rename', 'sid': SID_A, 'name': bad}, NOW, agents=[])['ok'], bad)
        running = [{'sessionId': SID_A, 'id': 'aaaaaaaa', 'kind': 'background', 'status': 'idle', 'pid': 9}]
        before = Path(p).read_text()
        r = self.sb.rpc({'verb': 'rename', 'sid': SID_A, 'name': 'Live one'}, NOW, agents=running)
        self.assertEqual((r['ok'], r['in_transcript']), (True, False))
        self.assertEqual(Path(p).read_text(), before, 'a running chat\'s transcript is Claude Code\'s to write')

    def ls(self, agents=()):  # now: the agent's own writes move a transcript's time to the real now
        return self.sb.rpc({'verb': 'list'}, time.time(), agents=list(agents))['sessions']

    def titles(self, p):
        return [json.loads(x).get('customTitle') for x in Path(p).read_text().splitlines() if '"custom-title"' in x]

    def test_a_running_chat_gets_the_name_once_it_stops(self):
        p = self.chat(SID_A, NOW - 600, title='Auto title')
        with open(p, 'a') as f:
            f.write(json.dumps({'type': 'custom-title', 'customTitle': 'Launch name', 'sessionId': SID_A}) + '\n')
        running = [{'sessionId': SID_A, 'id': 'aaaaaaaa', 'kind': 'interactive', 'status': 'busy', 'pid': 9}]
        self.sb.rpc({'verb': 'rename', 'sid': SID_A, 'name': 'My name'}, NOW, agents=running)
        self.assertEqual(self.ls(running)[0]['title'], 'My name')
        self.assertEqual(self.titles(p), ['Launch name'], 'nothing written while it runs')
        with open(p, 'a') as f:  # Claude Code writes its own title back as the chat ends
            f.write(json.dumps({'type': 'custom-title', 'customTitle': 'Launch name', 'sessionId': SID_A}) + '\n')
        self.assertEqual(self.ls()[0]['title'], 'My name')
        self.assertEqual(self.titles(p)[-1], 'My name', 'written once it stopped, after Claude Code\'s own line')
        self.ls()
        self.assertEqual(self.titles(p).count('My name'), 1, 'and only once')
        r = self.sb.rpc({'verb': 'rename', 'sid': SID_A, 'name': ''}, NOW, agents=[])
        self.assertEqual((r['ok'], r['in_transcript']), (True, True))
        self.assertEqual(self.titles(p)[-1], 'Launch name', 'clearing brings back the title it had')
        self.assertEqual(self.ls()[0]['title'], 'Launch name')

    def test_clearing_a_name_goes_back_to_the_auto_title(self):
        p = self.chat(SID_A, NOW - 600, title='Auto title')
        self.sb.rpc({'verb': 'rename', 'sid': SID_A, 'name': 'Mine'}, NOW, agents=[])
        self.assertEqual(self.sb.rpc({'verb': 'browse'}, NOW, agents=[])['rows'][0]['title'], 'Mine')
        self.sb.rpc({'verb': 'rename', 'sid': SID_A, 'name': ''}, NOW, agents=[])
        self.assertEqual(self.titles(p), ['Mine', ''], 'an empty title: Claude Code shows its own again')
        self.assertEqual(self.ls()[0]['title'], 'Auto title')
        before = Path(p).read_text()
        self.sb.rpc({'verb': 'rename', 'sid': SID_A, 'name': ''}, NOW, agents=[])
        self.assertEqual(Path(p).read_text(), before, 'clearing a chat never named here changes nothing')

    def test_a_rename_in_claude_code_itself_wins_later(self):
        p = self.chat(SID_A, NOW - 600, title='Auto title')
        self.sb.rpc({'verb': 'rename', 'sid': SID_A, 'name': 'Mine'}, NOW, agents=[])
        with open(p, 'a') as f:
            f.write(json.dumps({'type': 'custom-title', 'customTitle': 'From slash rename', 'sessionId': SID_A}) + '\n')
        self.assertEqual(self.ls()[0]['title'], 'From slash rename')
        self.assertIsNone(self.sb.given_name(SID_A))

    def test_clear_carries_the_name_as_the_next_part(self):
        self.chat(SID_A, NOW - 600, title='Auto title')
        self.chat(SID_B, NOW - 60, title='Other auto title')
        self.sb.rpc({'verb': 'rename', 'sid': SID_A, 'name': 'Mine (part 2)'}, NOW, agents=[])
        self.sb.cleared_by(SID_A, SID_B, NOW)
        self.assertEqual(self.sb.given_name(SID_B), 'Mine (part 3)')
        running = [{'sessionId': SID_B, 'id': 'bbbbbbbb', 'kind': 'interactive', 'status': 'idle', 'pid': 9}]
        rows = self.ls(running)
        self.assertEqual([r['title'] for r in rows if r['sid'] == SID_B], ['Mine (part 3)'])


class Fake:
    """Two machines; the Desk has the new agent (features), the Tower an older one."""

    def __init__(self):
        self.sessions = {'desk': [], 'tower': []}
        self.rows = {'desk': [], 'tower': []}
        self.features = {'desk': ['flags', 'ping', 'browse', 'rename', 'summary'], 'tower': []}
        self.calls = []

    def __call__(self, target, req, timeout):
        name = 'desk' if target == 'me@x' else 'tower'
        self.calls.append((name, req))
        v = req['verb']
        if v == 'list':
            return {'ok': True, 'sessions': [dict(s) for s in self.sessions[name]], 'replaced': [], 'limits': None,
                    'features': self.features[name]}
        if v == 'browse':
            rows = [r for r in self.rows[name] if r['mtime'] > (req.get('since') or 0)]
            return {'ok': True, 'rows': rows, 'all': {r['sid']: r['mtime'] for r in self.rows[name]}, 'partial': False}
        if v == 'summary':
            return {'ok': True, 'sid': req['sid'], 'first': {'text': 'hi'}, 'recent': [], 'answer': 'done'}
        if v == 'start':
            return {'ok': True, 'job': 'abcdefabcdef'}
        if v == 'job':
            return {'ok': True, 'state': 'done', 'sid': sid(99)}
        if v == 'caps':
            return {'ok': True, 'caps': {'os': 'mac', 'chrome': {'installed': True, 'connector': name == 'desk', 'connected': False}}}
        return {'ok': True}


def sid(n):
    return f'{n:08x}-1111-2222-3333-444444444444'


def live(n, status='idle', **kw):
    return dict({'sid': sid(n), 'title': f'chat {n}', 'kind': 'background', 'status': status, 'bg_id': sid(n)[:8],
                 'mtime': NOW - 60, 'active': NOW - 60, 'ctx': 1000, 'folder': 'p', 'cwd': '/p'}, **kw)


class ServerBase(unittest.TestCase):
    def setUp(self):
        self.f = Fake()
        self.state, self.cfg = Path(mkdtemp()), Path(mkdtemp())
        (self.cfg / 'caps.json').write_text(json.dumps({'desk': True, 'tower': True}))
        self.now = [NOW]
        self.app = server.App(fleet.Fleet(MACH, self.f, clock=lambda: self.now[0]), state_dir=self.state, web_dir=self.state,
                              clock=lambda: self.now[0], cfg_dir=self.cfg, retry_waits=(0, 0))
        self.app._refresh_soon = lambda *a, **k: None
        self.app.feat.reaper.opened = lambda: set()


class NewWithFlagsTest(ServerBase):
    def jobs(self):
        return [r for n, r in self.f.calls if r['verb'] == 'start']

    def test_flags_reach_the_machine_and_are_remembered(self):
        self.app.fleet.poll()
        r = self.app.post('/api/new', {'machine': 'desk', 'folder': 'p', 'prompt': 'hi', 'model': 'opus', 'effort': 'high',
                                       'flags': [['--chrome'], ['--permission-mode', 'plan']], 'other': '--add-dir proj/x'})
        self.assertTrue(r['ok'])
        for _ in range(50):
            if self.jobs():
                break
            time.sleep(0.05)
        req = self.jobs()[0]
        self.assertEqual((req['chrome'], req['effort'], req['model'], req['flags']),
                         (True, 'high', 'opus', [['--permission-mode', 'plan'], ['--add-dir', 'proj/x']]))
        last = self.app.get('/api/prefs')['last']
        self.assertEqual((last['effort'], last['other'], last['flags'][0]), ('high', '--add-dir proj/x', ['--chrome']))

    def test_refused_before_anything_starts(self):
        self.app.fleet.poll()
        for body in [{'other': '--dangerously-skip-permissions'}, {'flags': [['--add-dir', '/etc']]}, {'effort': 'turbo'},
                     {'model': 'opus', 'other': '--model sonnet'}]:
            with self.subTest(body=body), self.assertRaises(server.HttpError) as cm:
                self.app.post('/api/new', dict({'machine': 'desk', 'folder': 'p'}, **body))
            self.assertEqual(cm.exception.code, 400)
        with self.assertRaises(server.HttpError) as cm:  # an older agent would drop them without a word
            self.app.post('/api/new', {'machine': 'tower', 'folder': 'p', 'flags': [['--brief']]})
        self.assertIn('needs the Switchboard update', str(cm.exception))
        r = self.app.post('/api/new', {'machine': 'tower', 'folder': 'p', 'flags': [['--chrome']], 'effort': 'max'})
        self.assertTrue(r['ok'], 'Chrome and effort an older agent knows')

    def test_prefs(self):
        p = self.app.get('/api/prefs')
        self.assertEqual((p['chrome_machine'], p['presets']), (None, []), 'no machine is the Chrome machine until one is picked')
        self.assertIn('--add-dir', p['flags'])
        self.assertEqual((p['takes']['--add-dir'], p['takes']['--chrome'], p['aliases']['-w']), (True, False, '--worktree'))
        self.app.post('/api/prefs', {'chrome_machine': 'tower', 'presets': [{'name': 'Review', 'mode': 'max', 'flags': [['--permission-mode', 'plan']]}]})
        p = self.app.get('/api/prefs')
        self.assertEqual((p['chrome_machine'], p['presets'][0]['name'], p['presets'][0]['mode']), ('tower', 'Review', 'max'))
        for bad in [{'chrome_machine': 'nope'}, {'presets': [{'name': 'x', 'other': '--print'}]}, {'presets': [{'flags': []}]},
                    {'presets': [{'name': f'p{i}'} for i in range(13)]}, {'last': {'model': 'gpt'}}]:
            with self.subTest(bad=bad), self.assertRaises(server.HttpError):
                self.app.post('/api/prefs', bad)
        self.assertEqual({k: v for k, v in self.app.sessions()['prefs'].items() if k != 'appearance'}, {'chrome_machine': 'tower', 'summary_model': 'opus'})


class ChromeAutoTest(ServerBase):
    def test_chrome_goes_to_the_chats_own_chrome_machine(self):
        self.app.post('/api/prefs', {'chrome_machine': 'tower'})  # the old one-for-all setting no longer routes
        self.app.fleet.poll()
        self.app.feat.auto.refresh(self.app.fleet.snapshot()['machines'], force=True, wait=5)
        r = self.app.post('/api/route', {'prompt': 'check my site in the browser', 'chrome': True, 'rules': True, 'chrome_machine': 'desk'})
        self.assertEqual((r['machine'], r['by'], r['why']), ('desk', 'chrome', "Desk is this chat's Chrome machine."))
        self.app.fleet.state['desk']['online'] = False
        self.app.fleet.state['tower'].update(online=True)
        self.app.feat.auto.data['tower']['caps']['chrome']['connector'] = True
        r = self.app.post('/api/route', {'prompt': 'x', 'chrome': True, 'rules': True, 'chrome_machine': 'desk'})
        self.assertEqual((r['machine'], r['why']), ('tower', 'Tower is the only machine online with Claude in Chrome.'))
        m = {m['name']: m for m in self.app.sessions()['machines']}
        self.assertEqual((m['desk']['chrome']['connector'], m['desk']['chrome_pref'], m['tower']['chrome_pref']), (True, False, False))

    def test_each_chat_keeps_its_own_chrome_machine(self):
        self.f.sessions['desk'] = [live(1, 'idle'), live(2, 'idle')]
        self.app.fleet.poll()
        self.app.post('/api/chrome', {'machine': 'desk', 'sid': sid(1), 'chrome_machine': 'tower'})
        self.app.post('/api/chrome', {'machine': 'desk', 'sid': sid(2), 'chrome_machine': 'desk'})
        got = {s['sid']: s['chrome_machine'] for s in self.app.sessions()['machines'][0]['sessions']}
        self.assertEqual(got, {sid(1): 'tower', sid(2): 'desk'})
        # Start fresh / Continue: the new chat inherits the old chat's pick
        self.app._chrome_carry({'state': 'done', 'machine': 'desk', 'sid': sid(3), 'plan': {'req': {'sid': sid(1)}}})
        self.assertEqual(self.app.chrome_for('desk', sid(3)), 'tower')
        self.app._chrome_carry({'state': 'done', 'machine': 'tower', 'sid': sid(4), 'plan': {'src': 'desk', 'sid': sid(2)}})
        self.assertEqual(self.app.chrome_for('tower', sid(4)), 'desk')
        # a new chat takes the one picked in the New sheet
        self.app._chrome_carry({'state': 'done', 'machine': 'desk', 'sid': sid(5), 'plan': {'req': {}, 'chrome_machine': 'tower'}})
        self.assertEqual(self.app.chrome_for('desk', sid(5)), 'tower')
        self.app.post('/api/chrome', {'machine': 'desk', 'sid': sid(1), 'chrome_machine': None})
        self.assertIsNone(self.app.chrome_for('desk', sid(1)))
        for bad in [{'chrome_machine': 'nope'}, {'sid': '../x', 'chrome_machine': 'desk'}]:
            with self.assertRaises(server.HttpError):
                self.app.post('/api/chrome', dict({'machine': 'desk', 'sid': sid(1)}, **bad))


class LiveTest(ServerBase):
    def test_a_ping_shows_at_once_then_the_machine_settles_it(self):
        self.f.sessions['desk'] = [live(1, 'idle')]
        self.app.fleet.poll()
        polls = []
        self.app._poll_soon = lambda name, delay: polls.append(name)
        v = self.app.version
        self.app.post('/api/event', {'event': 'ping', 'why': 'prompt', 'machine': 'desk', 'session_id': sid(1)})
        self.assertGreater(self.app.version, v)
        self.assertEqual(self.app.sessions()['machines'][0]['sessions'][0]['status'], 'busy')
        self.assertEqual(polls, ['desk'])
        self.app.post('/api/event', {'event': 'ping', 'why': 'status', 'machine': 'desk', 'session_id': sid(1), 'ctx': 88_000})
        self.assertEqual(self.app.sessions()['machines'][0]['sessions'][0]['ctx'], 88_000)
        self.assertEqual(polls, ['desk'], 'status pings need no confirming')
        self.now[0] += 2
        self.f.sessions['desk'] = [live(1, 'idle', ctx=90_000)]
        self.app.fleet.state['desk']['asked'] = self.now[0]
        self.app.fleet.store('desk', dict(self.app.fleet.state['desk'], sessions=self.f.sessions['desk'], asked=self.now[0] + 1))
        s = self.app.sessions()['machines'][0]['sessions'][0]
        self.assertEqual((s['status'], s['ctx']), ('idle', 90_000), 'the machine answered after the ping: it wins')
        for bad in [{'why': 'nope'}, {'why': 'prompt', 'session_id': '../x'}]:
            with self.assertRaises(server.HttpError):
                self.app.post('/api/event', dict({'event': 'ping', 'machine': 'desk', 'session_id': sid(1)}, **bad))

    def test_a_changed_machine_bumps_the_version(self):
        self.app.fleet.poll()
        v = self.app.version
        self.app.fleet.poll()
        self.assertEqual(self.app.version, v, 'nothing changed: no bump')
        self.f.sessions['desk'] = [live(2)]
        self.app.fleet.poll()
        self.assertGreater(self.app.version, v)

    def test_stream(self):
        srv = server.make_server('127.0.0.1', 0, self.app)
        threading.Thread(target=srv.serve_forever, daemon=True).start()
        try:
            c = http.client.HTTPConnection('127.0.0.1', srv.server_address[1], timeout=5)
            c.request('GET', '/api/stream')
            r = c.getresponse()
            self.assertEqual((r.status, r.getheader('Content-Type')), (200, 'text/event-stream'))
            buf = b''
            while b'event: hello' not in buf:
                buf += r.fp.readline()
            t0 = time.time()
            threading.Timer(0.2, self.app.bump).start()
            buf = b''
            while b'event: change' not in buf:
                buf += r.fp.readline()
            self.assertLess(time.time() - t0, 1.5)
            c.close()
            c = http.client.HTTPConnection('127.0.0.1', srv.server_address[1], timeout=5)
            c.request('GET', '/api/stream', headers={'Host': 'evil.example'})
            self.assertEqual(c.getresponse().status, 403)
        finally:
            srv.shutdown()

    def test_widget_push_on_what_widgets_show(self):
        pushed = []
        self.app.widget_pusher = lambda: pushed.append(1)
        self.app.fleet.poll()
        self.assertEqual(pushed, [], 'the first look after a restart sends nothing')
        self.f.sessions['desk'] = [live(3, 'busy')]
        self.app.fleet.poll()
        for _ in range(40):
            if pushed:
                break
            time.sleep(0.05)
        self.assertEqual(pushed, [1])
        self.f.sessions['desk'] = [live(3, 'busy', ctx=5000)]
        self.app.fleet.poll()  # the context grew: the widgets do not show that
        self.assertIsNone(self.app._widget['timer'])
        self.f.sessions['desk'] = []
        self.app.fleet.poll()  # within the minute: waits for it
        self.assertIsNotNone(self.app._widget['timer'])
        self.app._widget['timer'].cancel()
        self.assertEqual(pushed, [1])

    def test_widget_tokens(self):
        tok = 'ab' * 32
        self.assertEqual(self.app.post('/api/widget-token', {'token': tok}), {'ok': True})
        self.assertIn(tok, self.app.widget_tokens())
        with self.assertRaises(server.HttpError):
            self.app.post('/api/widget-token', {'token': 'zz'})


class BrowseServerTest(ServerBase):
    def test_index_search_page_and_rename(self):
        self.f.rows['desk'] = [{'sid': sid(i), 'title': f'Chat {i}', 'folder': 'proj' if i % 2 else 'other', 'cwd': '/r/p',
                                'first_prompt': 'fix the widget' if i == 7 else 'hello', 'last_prompt': 'bye', 'model': 'claude-opus-5-5',
                                'ctx': 100, 'size': 2000, 'mtime': NOW - i * 86400, 'last': NOW - i * 86400, 'started': NOW - i * 86400 - 60,
                                'archived': NOW - 86400 if i == 3 else None, 'url': None, 'replaced': None} for i in range(1, 121)]
        self.f.sessions['desk'] = [live(1, 'busy', folder='proj')]
        self.app.fleet.poll()
        r = self.app.get('/api/browse', {'limit': ['50']})
        self.assertEqual((r['total'], len(r['rows']), r['rows'][0]['sid'], r['rows'][0]['state']), (120, 50, sid(1), 'live'))
        r2 = self.app.get('/api/browse', {'limit': ['50'], 'offset': ['100']})
        self.assertEqual(len(r2['rows']), 20)
        self.assertEqual([x['sid'] for x in self.app.get('/api/browse', {'q': ['widget']})['rows']], [sid(7)])
        self.assertEqual(self.app.get('/api/browse', {'state': ['archived']})['total'], 1)
        self.assertEqual(self.app.get('/api/browse', {'folder': ['proj']})['total'], 60)
        self.assertEqual(self.app.get('/api/browse', {'from': [str(NOW - 10.5 * 86400)]})['total'], 10)
        self.assertEqual(self.app.get('/api/browse', {'machine': ['tower']})['total'], 0)
        with self.assertRaises(server.HttpError):
            self.app.get('/api/browse', {'state': ['bogus']})
        self.assertEqual(self.app.get(f'/api/summary/desk/{sid(7)}')['answer'], 'done')
        self.app.post('/api/action', {'machine': 'desk', 'verb': 'rename', 'sid': sid(7), 'name': 'The widget fix'})
        self.assertEqual(self.app.get('/api/browse', {'q': ['widget fix']})['rows'][0]['title'], 'The widget fix')
        self.assertIn(('desk', {'verb': 'rename', 'sid': sid(7), 'name': 'The widget fix'}), self.f.calls)
        with self.assertRaises(server.HttpError):
            self.app.post('/api/action', {'machine': 'desk', 'verb': 'rename', 'sid': sid(7), 'name': 'a/b'})
        self.f.sessions['desk'] = [live(7)]
        self.app.fleet.poll()
        self.assertEqual(self.app.sessions()['machines'][0]['sessions'][0]['title'], 'The widget fix')

    def test_a_name_shows_at_once_and_steps_aside_once_the_machine_has_it(self):
        self.f.sessions['desk'] = [live(7)]
        self.app.fleet.poll()
        v = self.app.version
        self.app.post('/api/action', {'machine': 'desk', 'verb': 'rename', 'sid': sid(7), 'name': 'Mine'})
        self.assertGreater(self.app.version, v, 'every app hears of it over the stream')
        self.assertEqual(self.app.sessions()['machines'][0]['sessions'][0]['title'], 'Mine')
        self.assertEqual(self.app._title('desk', sid(7)), 'Mine')
        self.f.sessions['desk'] = [live(7, title='Mine')]  # the machine's own list says it now
        self.app.fleet.poll()
        self.app.sessions()
        self.assertEqual(self.app.names(), {}, 'the machine is the one place it lives from here')
        self.app.post('/api/action', {'machine': 'desk', 'verb': 'rename', 'sid': sid(7), 'name': ''})
        self.assertIn(('desk', {'verb': 'rename', 'sid': sid(7), 'name': ''}), self.f.calls)
        self.assertEqual(self.app.names(), {})

    def test_a_name_given_while_the_machine_was_away_reaches_it_later(self):
        self.f.sessions['desk'] = [live(7)]
        self.app.fleet.poll()
        self.app.set_name(f'desk/{sid(7)}', 'Mine')
        self.app._name_tries[f'desk/{sid(7)}'] = 0  # long ago
        self.app.sessions()
        for _ in range(50):
            if ('desk', {'verb': 'rename', 'sid': sid(7), 'name': 'Mine'}) in self.f.calls:
                break
            time.sleep(0.05)
        self.assertIn(('desk', {'verb': 'rename', 'sid': sid(7), 'name': 'Mine'}), self.f.calls)


if __name__ == '__main__':
    unittest.main()
