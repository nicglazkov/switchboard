"""Dispatch: a chat on one machine sends work to another and gets its report; the MCP server Claude uses for it."""
import io, json, os, subprocess, sys, tempfile, threading, time, unittest
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from tempfile import mkdtemp
from tests.helpers import ROOT, assistant, fresh_sb, user, write_transcript
sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'server'))
import fleet, server  # noqa: E402
from tests.test_auto import DESK, PC, Fake  # noqa: E402
from tests.test_server import MACH  # noqa: E402

FROM = 'aa' * 4 + '-1111-2222-3333-444444444444'
CHILD = 'cc' * 4 + '-1111-2222-3333-444444444444'


class Box(Fake):
    """Fake machines where the started chat shows up in the Desk's list and can be moved along by the test."""
    def __init__(self):
        super().__init__()
        self.row = None
        self.report = 'Did it.\n\nChanged: app/Widget.swift\n\nLeft: nothing.'

    def __call__(self, target, req, timeout):
        name = 'desk' if target == 'me@x' else 'tower'
        if req['verb'] == 'list':
            with self.lock:
                self.calls.append((name, req))
            rows = [dict(self.row)] if self.row and name == self.row['_m'] else []
            return {'ok': True, 'sessions': [{k: v for k, v in r.items() if k != '_m'} for r in rows], 'limits': None,
                    'launcher': {'alive': True} if name == 'desk' else {'alive': None}}
        if req['verb'] == 'report':
            with self.lock:
                self.calls.append((name, req))
            return {'ok': True, 'text': self.report} if req['sid'] == CHILD else {'ok': False, 'error': 'session not found on this machine'}
        if req['verb'] == 'job':
            with self.lock:
                self.calls.append((name, req))
            self.row = {'_m': name, 'sid': CHILD, 'title': 'x', 'status': 'busy', 'ctx': 0, 'cold': False, 'mtime': 1.0, 'kind': 'background'}
            return {'ok': True, 'state': 'done', 'sid': CHILD, 'url': 'https://claude.ai/code/session_CHILD', 'name': 'x'}
        return super().__call__(target, req, timeout)


class DispatchServer(unittest.TestCase):
    def make(self):
        self.f = Box()
        self.state, cfg = Path(mkdtemp()), Path(mkdtemp())
        (cfg / 'caps.json').write_text(json.dumps({'desk': True, 'tower': True}))
        app = server.App(fleet.Fleet(MACH, self.f), state_dir=self.state, web_dir=self.state, sleep=lambda s: None, poll_every=0,
                         retry_waits=(0, 0), cfg_dir=cfg, pusher=lambda *a: None)
        app._refresh_soon = lambda *a, **k: None
        app.fleet.poll()
        app.feat.auto.refresh(app.fleet.snapshot()['machines'], wait=5)
        app.feat.dispatch._poll_soon = lambda name: app.fleet.poll_one(name)
        self.app = app
        return app

    def send(self, **kw):
        body = {'from': {'machine': 'tower', 'sid': FROM}, 'machine': 'desk', 'task': 'Build the iOS widget and run its tests.',
                'folder': 'switchboard', 'mode': 'normal'}
        body.update(kw)
        return self.app.post('/api/dispatch', body)

    def until(self, did, want):
        for _ in range(300):
            r = self.app.get(f'/api/dispatch/{did}')
            if r['state'] == want:
                return r
            time.sleep(0.01)
        self.fail(f'stuck at {r}')

    def test_the_whole_trip(self):
        app = self.make()
        r = self.send()
        self.assertRegex(r['id'], r'^d-[0-9a-f]{12}$')
        self.assertEqual((r['machine'], r['ok']), ('desk', True))
        self.assertTrue(r['job'].startswith('s-'))
        (_, start), = self.f.verbs('start')
        self.assertEqual(start['name'], 'Build the iOS widget and run its tests (for Tower)')
        self.assertEqual(start['folder'], 'switchboard')
        self.assertTrue(start['prompt'].startswith('Build the iOS widget and run its tests.\n\n'))
        self.assertIn('You were sent this by a Claude Code session on Tower. Do the work here, then end with a short report: '
                      'what you did, what changed (paths), what is left.', start['prompt'])
        w = self.until(r['id'], 'working')
        self.assertEqual((w['sid'], w['url'], w['report']), (CHILD, 'https://claude.ai/code/session_CHILD', None))
        self.f.row.update(status='idle', ctx=0)  # idle before any reply: not done yet
        app.fleet.poll()
        self.assertEqual(app.get(f'/api/dispatch/{r["id"]}')['state'], 'working')
        self.f.row.update(status='waiting', ctx=9000)
        app.fleet.poll()
        self.assertIn('Waiting for an answer', app.get(f'/api/dispatch/{r["id"]}')['note'])
        self.f.row.update(status='idle', ctx=21000)
        app.fleet.poll()
        d = self.until(r['id'], 'done')
        self.assertEqual(d['report'], self.f.report)
        self.assertEqual(d['label'], 'Desk')
        self.assertIsInstance(d['elapsed'], int)
        n = len(self.f.verbs('report'))
        self.assertEqual(app.get(f'/api/dispatch/{r["id"]}')['report'], self.f.report)
        self.assertEqual(len(self.f.verbs('report')), n)  # read once, kept
        saved = json.loads((self.state / 'dispatch.json').read_text())[r['id']]
        self.assertEqual((saved['state'], saved['from']), ('done', {'machine': 'tower', 'sid': FROM}))

    def test_stopped_counts_as_done(self):
        app = self.make()
        r = self.send()
        self.until(r['id'], 'working')
        self.f.row.update(status='stopped', ctx=4000)
        app.fleet.poll()
        self.assertEqual(self.until(r['id'], 'done')['report'], self.f.report)

    def test_auto_dispatch_says_where_and_why(self):
        app = self.make()
        self.f.answer = {'machine': 'desk', 'folder': 'switchboard', 'why': 'Has Xcode 26'}
        r = self.send(machine='auto', folder='')
        self.assertEqual((r['machine'], r['why']), ('desk', 'Has Xcode 26.'))
        (_, start), = self.f.verbs('start')
        self.assertEqual(start['folder'], 'switchboard')
        job = app.job('server', r['job'])
        self.assertEqual(job['picked'], 'Auto picked Desk. Has Xcode 26.')

    def test_a_launch_that_fails_is_failed(self):
        app = self.make()
        real = self.f.__call__

        def broken(target, req, timeout):
            if req['verb'] == 'job':
                return {'ok': True, 'state': 'error', 'error': 'folder does not exist'}
            return real(target, req, timeout)
        app.fleet.transport = broken
        r = self.send()
        f = self.until(r['id'], 'failed')
        self.assertEqual(f['error'], 'folder does not exist')

    def test_bad_requests(self):
        app = self.make()
        for kw, msg in [({'task': ''}, 'The task is empty.'), ({'machine': 'mars'}, 'unknown machine'),
                        ({'from': {'machine': 'x'}}, 'from.machine must be one of the machines'), ({'mode': 'turbo'}, 'unknown mode'),
                        ({'from': {'machine': 'desk', 'sid': 'nope'}}, 'bad from.sid'), ({'task': 'x' * 20000}, 'too long')]:
            with self.assertRaises(server.HttpError) as e:
                self.send(**kw)
            self.assertIn(msg, str(e.exception))
        with self.assertRaises(server.HttpError) as e:
            app.get('/api/dispatch/d-000000000000')
        self.assertEqual(e.exception.code, 404)

    def test_long_tasks_get_a_name_that_fits(self):
        self.make()
        self.send(task='Refactor: the whole (very) long-winded module into smaller parts please, now!\nmore')
        (_, start), = self.f.verbs('start')
        self.assertLessEqual(len(start['name']), 64)
        self.assertEqual(start['name'], 'Refactor: the whole very long-winded module into (for Tower)')


class Report(unittest.TestCase):
    def test_the_last_words_not_the_tool_calls(self):
        tmp = tempfile.mkdtemp()
        sb = fresh_sb(tmp)
        tp = write_transcript(tmp, [user('2026-10-12T10:00:00Z', 'go'), assistant('2026-10-12T10:00:05Z', 'Working on it.'),
                                    assistant('2026-10-12T10:01:00Z', None, tools=[('Bash', {'command': 'ls'})]),
                                    assistant('2026-10-12T10:02:00Z', 'Done.\n\nChanged: a.py'),
                                    assistant('2026-10-12T10:02:30Z', 'side', side=True),
                                    assistant('2026-10-12T10:03:00Z', None, tools=[('Read', {'file_path': 'x'})])])
        text, at = sb.last_reply(tp)
        self.assertEqual(text, 'Done.\n\nChanged: a.py')
        self.assertIsNotNone(at)


class Mcp(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp()
        self.sb = fresh_sb(self.tmp)

    def rpc(self, method, params=None, mid=1, tool=None):
        return self.sb.mcp_handle({'jsonrpc': '2.0', 'id': mid, 'method': method, 'params': params or {}}, tool=tool)

    def test_handshake_and_tools(self):
        r = self.rpc('initialize', {'protocolVersion': '2025-06-18', 'capabilities': {}, 'clientInfo': {'name': 't', 'version': '1'}})
        self.assertEqual(r['result']['protocolVersion'], '2025-06-18')
        self.assertEqual(r['result']['serverInfo']['name'], 'switchboard')
        self.assertIn('tools', r['result']['capabilities'])
        self.assertIsNone(self.sb.mcp_handle({'jsonrpc': '2.0', 'method': 'notifications/initialized'}))
        names = [t['name'] for t in self.rpc('tools/list')['result']['tools']]
        self.assertEqual(names, ['machines', 'run_on', 'check', 'continue_on', 'publish_tool', 'library', 'usage'])
        self.assertEqual(self.rpc('ping')['result'], {})
        self.assertEqual(self.rpc('nope')['error']['code'], -32601)
        self.assertEqual(self.rpc('tools/call', {'name': 'rm_rf'})['error']['code'], -32602)
        self.assertEqual(self.sb.mcp_handle({'id': 3})['error']['code'], -32600)

    def test_errors_come_back_as_tool_results(self):
        def tool(name, args):
            raise self.sb.RpcError('Switchboard is not reachable from here (URLError).')
        r = self.rpc('tools/call', {'name': 'machines', 'arguments': {}}, tool=tool)['result']
        self.assertTrue(r['isError'])
        self.assertEqual(r['structuredContent'], {'error': 'Switchboard is not reachable from here (URLError).'})
        self.assertIn('not reachable', r['content'][0]['text'])

    def test_machines(self):
        snap = {'machines': [{'name': 'desk', 'label': 'Desk', 'online': True, 'can': 'iOS',
                              'sessions': [{'status': 'busy', 'stalled': False}, {'status': 'busy', 'stalled': True}, {'status': 'idle'}]},
                             {'name': 'tower', 'label': 'Tower', 'online': False, 'can': None, 'sessions': []}]}
        out, bad = self.sb.mcp_tool('machines', {}, call=lambda m, p, b=None: snap, me='tower')
        self.assertFalse(bad)
        self.assertEqual(out['machines'], [
            {'name': 'desk', 'label': 'Desk', 'online': True, 'can': 'iOS', 'running_chats': 1, 'this_machine': False},
            {'name': 'tower', 'label': 'Tower', 'online': False, 'can': '', 'running_chats': 0, 'this_machine': True}])

    def test_run_on_and_check(self):
        sent, states = [], iter(['starting', 'working', 'working', 'done'])

        def call(method, path, body=None):
            sent.append((method, path, body))
            if path == '/api/dispatch':
                return {'ok': True, 'id': 'd-0123456789ab', 'machine': 'laptop', 'job': 's-1', 'why': 'Has Xcode 26.'}
            st = next(states)
            return {'ok': True, 'state': st, 'machine': 'laptop', 'label': 'Laptop', 'elapsed': 30,
                    'report': 'R' if st == 'done' else None, 'url': 'https://claude.ai/code/x'}
        out, bad = self.sb.mcp_tool('run_on', {'machine': 'auto', 'task': 'Build it', 'folder': 'sb'}, call=call, me='desk', sid=FROM)
        self.assertEqual((out['id'], out['machine'], out['why'], bad), ('d-0123456789ab', 'laptop', 'Has Xcode 26.', False))
        self.assertEqual(sent[0], ('POST', '/api/dispatch', {'from': {'machine': 'desk', 'sid': FROM}, 'machine': 'auto',
                                                             'task': 'Build it', 'folder': 'sb', 'mode': 'normal'}))
        clock, slept = [0.0], []
        out, bad = self.sb.mcp_tool('check', {'id': 'd-0123456789ab', 'wait_s': 999}, call=call,
                                    sleep=lambda s: (slept.append(s), clock.__setitem__(0, clock[0] + s)), clock=lambda: clock[0])
        self.assertEqual((out['state'], out['report'], bad), ('done', 'R', False))
        self.assertEqual(slept, [5, 5, 5])
        out, bad = self.sb.mcp_tool('check', {'id': 'bad'}, call=call)
        self.assertTrue(bad)

    def test_check_gives_up_at_the_wait(self):
        clock = [0.0]

        def call(method, path, body=None):
            return {'ok': True, 'state': 'working', 'machine': 'desk', 'elapsed': 10}
        out, bad = self.sb.mcp_tool('check', {'id': 'd-0123456789ab', 'wait_s': 12}, call=call,
                                    sleep=lambda s: clock.__setitem__(0, clock[0] + s), clock=lambda: clock[0])
        self.assertEqual((out['state'], bad), ('working', False))
        self.assertEqual(clock[0], 12)
        clock[0] = 0
        self.sb.mcp_tool('check', {'id': 'd-0123456789ab', 'wait_s': 10_000}, call=call,
                         sleep=lambda s: clock.__setitem__(0, clock[0] + s), clock=lambda: clock[0])
        self.assertEqual(clock[0], 240)  # never more than 240 s

    def test_continue_on_moves_this_chat_into_the_same_repo(self):
        sent = []

        def call(method, path, body=None):
            sent.append((method, path, body))
            if path == '/api/caps':
                return {'laptop': {'repos': ['work/Switchboard', 'other']}}
            return {'ok': True, 'job': 's-0123456789ab', 'machine': 'server'}
        out, bad = self.sb.mcp_tool('continue_on', {'machine': 'laptop'}, call=call, me='desk', sid=FROM, cwd='/Users/you/x/switchboard')
        self.assertFalse(bad)
        self.assertEqual(sent[-1], ('POST', '/api/continue', {'machine': 'desk', 'sid': FROM, 'to': 'laptop', 'folder': 'work/Switchboard'}))
        self.assertEqual(out['job'], 's-0123456789ab')
        out, bad = self.sb.mcp_tool('continue_on', {'machine': 'desk'}, call=call, me='desk', sid=FROM)
        self.assertTrue(bad)
        self.sb.my_session = lambda: None
        out, bad = self.sb.mcp_tool('continue_on', {'machine': 'laptop'}, call=call, me='desk')
        self.assertIn('Could not tell which chat', out['error'])

    def test_which_chat_this_is(self):
        sb = self.sb
        self.assertEqual(sb.my_session({'CLAUDE_SESSION_ID': FROM}), FROM)
        me = os.getpid()
        table = {me: (4242, 'python3'), 4242: (1, 'claude'), 1: (0, 'launchd')}
        self.assertIsNone(sb.my_session({}, table))
        sb.seen_put(4242, FROM, '/t.jsonl', 100.0)
        self.assertEqual(sb.my_session({}, table), FROM)
        sb.seen_put(4242, CHILD, '/u.jsonl', 101.0)  # /clear or a resume in the same terminal
        self.assertEqual(sb.my_session({}, table), CHILD)

    def test_the_hook_remembers_the_chat(self):
        sb = self.sb
        sb.handle_hook({'hook_event_name': 'SessionStart', 'session_id': FROM, 'transcript_path': '/t.jsonl', 'source': 'resume'},
                       100.0, 777, spawn=lambda *a: None, notify=lambda e: None)
        self.assertEqual(json.loads((sb.paths()['state'] / 'seen' / '777.json').read_text())['sid'], FROM)

    def test_stdio_serves_lines_and_answers_pings_during_a_long_call(self):
        gate = threading.Event()

        def handle(msg):
            if msg.get('method') == 'tools/call':
                gate.wait(5)
            return self.sb.mcp_handle(msg, tool=lambda n, a: ({'ok': 1}, False))

        class Feed(io.StringIO):
            pass
        lines = [json.dumps({'jsonrpc': '2.0', 'id': 1, 'method': 'tools/call', 'params': {'name': 'machines'}}),
                 'not json', json.dumps({'jsonrpc': '2.0', 'id': 2, 'method': 'ping'}), '']
        out = io.StringIO()
        t = threading.Thread(target=self.sb.mcp_serve, args=(iter([l + '\n' for l in lines]), out, handle))
        t.start()
        for _ in range(200):
            if '"id": 2' in out.getvalue():
                break
            time.sleep(0.01)
        got = [json.loads(l) for l in out.getvalue().splitlines()]
        self.assertEqual([g.get('id') for g in got], [None, 2])  # the parse error and the ping, while the call waits
        gate.set()
        t.join(5)
        got = [json.loads(l) for l in out.getvalue().splitlines()]
        self.assertEqual(got[-1]['id'], 1)
        self.assertEqual(got[-1]['result']['structuredContent'], {'ok': 1})

    def test_real_process_over_pipes(self):
        env = dict(os.environ, SB_HOME=str(Path(self.tmp) / 'h'), PYTHONDONTWRITEBYTECODE='1')
        msgs = [{'jsonrpc': '2.0', 'id': 1, 'method': 'initialize', 'params': {'protocolVersion': '2025-06-18', 'capabilities': {},
                                                                                'clientInfo': {'name': 'test', 'version': '0'}}},
                {'jsonrpc': '2.0', 'method': 'notifications/initialized'},
                {'jsonrpc': '2.0', 'id': 2, 'method': 'tools/list'},
                {'jsonrpc': '2.0', 'id': 3, 'method': 'tools/call', 'params': {'name': 'check', 'arguments': {'id': 'nope'}}}]
        p = subprocess.run([sys.executable, str(ROOT / 'agent' / 'sb.py'), 'mcp'], input=''.join(json.dumps(m) + '\n' for m in msgs),
                           capture_output=True, text=True, timeout=20, env=env)
        got = {r['id']: r for r in map(json.loads, p.stdout.splitlines())}
        self.assertEqual(sorted(got), [1, 2, 3])
        self.assertEqual(got[1]['result']['protocolVersion'], '2025-06-18')
        self.assertEqual(len(got[2]['result']['tools']), 7)
        self.assertTrue(got[3]['result']['isError'])
        self.assertEqual(p.stderr, '')


class Hub(unittest.TestCase):
    def test_lan_then_tailscale_and_errors_in_plain_words(self):
        sb = fresh_sb(tempfile.mkdtemp())
        seen = []

        class H(BaseHTTPRequestHandler):
            def log_message(self, *a):
                pass

            def do_GET(self):
                seen.append((self.path, self.headers.get('X-Switchboard')))
                if self.path == '/api/dispatch/d-000000000000':
                    body, code = b'{"ok": false, "error": "No task with that id."}', 404
                else:
                    body, code = b'{"ok": true, "machines": []}', 200
                self.send_response(code)
                self.send_header('Content-Type', 'application/json')
                self.end_headers()
                self.wfile.write(body)
        srv = ThreadingHTTPServer(('127.0.0.1', 0), H)
        threading.Thread(target=srv.serve_forever, daemon=True).start()
        try:
            up = f'http://127.0.0.1:{srv.server_port}'
            self.assertEqual(sb.hub('GET', '/api/sessions', servers=('http://127.0.0.1:1', up)), {'ok': True, 'machines': []})
            self.assertEqual(seen, [('/api/sessions', '1')])
            with self.assertRaises(sb.RpcError) as e:
                sb.hub('GET', '/api/dispatch/d-000000000000', servers=(up,))
            self.assertEqual(str(e.exception), 'No task with that id.')
            with self.assertRaises(sb.RpcError) as e:
                sb.hub('GET', '/x', servers=('http://127.0.0.1:1',), timeout=1)
            self.assertIn('not reachable', str(e.exception))
        finally:
            srv.shutdown()
            srv.server_close()


class InstallMcp(unittest.TestCase):
    def test_merge_is_idempotent_backed_up_and_reversible(self):
        sb = fresh_sb(tempfile.mkdtemp())
        home = Path(tempfile.mkdtemp())
        cj = home / '.claude.json'
        cj.write_text(json.dumps({'numStartups': 9, 'mcpServers': {'other': {'type': 'stdio', 'command': 'x'}}, 'projects': {'/p': {}}}))
        self.assertTrue(sb.install_mcp(home, 'C:\\Users\\you\\.claude\\switchboard\\sb.py', 'python'))
        d = json.loads(cj.read_text())
        self.assertEqual(d['mcpServers']['switchboard'], {'type': 'stdio', 'command': 'python',
                                                          'args': ['C:/Users/you/.claude/switchboard/sb.py', 'mcp'], 'env': {}})
        self.assertEqual((d['numStartups'], d['projects'], d['mcpServers']['other']), (9, {'/p': {}}, {'type': 'stdio', 'command': 'x'}))
        backups = list(home.glob('.claude.json.bak-sb-*'))
        self.assertEqual(len(backups), 1)
        self.assertNotIn('switchboard', json.loads(backups[0].read_text())['mcpServers'])
        self.assertFalse(sb.install_mcp(home, 'C:\\Users\\you\\.claude\\switchboard\\sb.py', 'python'))  # nothing to change
        self.assertEqual(len(list(home.glob('.claude.json.bak-sb-*'))), 1)
        self.assertTrue(sb.uninstall_mcp(home))
        d = json.loads(cj.read_text())
        self.assertEqual(d['mcpServers'], {'other': {'type': 'stdio', 'command': 'x'}})
        self.assertFalse(sb.uninstall_mcp(home))

    def test_no_file_yet(self):
        sb = fresh_sb(tempfile.mkdtemp())
        home = Path(tempfile.mkdtemp())
        self.assertTrue(sb.install_mcp(home, '/Users/you/.claude/switchboard/sb.py', 'python3'))
        self.assertEqual(json.loads((home / '.claude.json').read_text())['mcpServers']['switchboard']['args'],
                         ['/Users/you/.claude/switchboard/sb.py', 'mcp'])
        self.assertFalse(sb.uninstall_mcp(Path(tempfile.mkdtemp())))


if __name__ == '__main__':
    unittest.main()
