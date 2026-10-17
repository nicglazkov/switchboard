import json, os, tempfile, time, unittest
from pathlib import Path
from tests.helpers import fresh_sb, user, assistant, entry, write_transcript

NOW = 1_800_000_000.0
SID_A = 'aaaaaaaa-1111-2222-3333-444444444444'
SID_B = 'bbbbbbbb-1111-2222-3333-444444444444'


def _iso(t):
    import datetime
    return datetime.datetime.fromtimestamp(t, datetime.timezone.utc).strftime('%Y-%m-%dT%H:%M:%S.%fZ')


class RpcBase(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp())
        self.root = self.tmp / 'root'
        (self.root / 'proj').mkdir(parents=True)
        self.cdir = self.tmp / 'dotclaude'
        (self.cdir / 'projects' / 'p1').mkdir(parents=True)
        self.cjson = self.tmp / 'claude.json'
        os.environ.update(SB_ROOT=str(self.root), SB_CLAUDE_DIR=str(self.cdir), SB_CLAUDE_JSON=str(self.cjson))
        self.sb = fresh_sb(self.tmp)
        self.calls = []
        self.sb.run_claude_cmd = lambda args, **kw: self.calls.append(args) or (f'stopped {args[1]}\n' if args[0] == 'stop' else '')

    def tearDown(self):
        for k in ('SB_ROOT', 'SB_CLAUDE_DIR', 'SB_CLAUDE_JSON'):
            os.environ.pop(k, None)

    def transcript(self, sid, cwd, mtime, title='A title', ctx=200_000, url_id='cse_XYZ123'):
        rows = [entry('bridge-session', '2026-10-01T00:00:00Z', bridgeSessionId=url_id, sessionId=sid),
                dict(user('2026-10-01T00:00:01Z', 'first thing I asked'), cwd=cwd),
                dict(assistant('2026-10-01T00:00:05Z', 'ok', usage={'input_tokens': ctx}), cwd=cwd),
                {'type': 'ai-title', 'aiTitle': title, 'sessionId': sid}]
        p = write_transcript(self.cdir / 'projects' / 'p1', rows, name=f'{sid}.jsonl')
        os.utime(p, (mtime, mtime))
        return p


class SafePathTest(RpcBase):
    def test_refuses_escapes(self):
        os.symlink('/etc', self.root / 'link')
        for bad in ['..', '../x', '/etc', 'C:\\Windows', 'C:/Windows', '~/x', 'proj/../../x', 'link', 'link/passwd', 'a\x00b', '', 'x' * 65]:
            with self.subTest(bad=bad):
                with self.assertRaises(ValueError):
                    self.sb.safe_path(self.root, bad)

    def test_accepts_nested_and_backslashes(self):
        self.assertEqual(self.sb.safe_path(self.root, 'proj/new one'), (self.root / 'proj' / 'new one').resolve())
        self.assertEqual(self.sb.safe_path(self.root, 'proj\\sub'), (self.root / 'proj' / 'sub').resolve())

    def test_valid_name(self):
        self.assertTrue(self.sb.valid_name('My Project-2.0_x'))
        for bad in ['', ' lead', '..', 'a/b', 'a;b', '$(x)', 'x' * 65, None]:
            self.assertFalse(self.sb.valid_name(bad), bad)


class ListTest(RpcBase):
    def test_lists_newest_first_with_merged_fields(self):
        self.transcript(SID_A, str(self.root / 'proj'), NOW - 600, title='Older')
        self.transcript(SID_B, str(self.root), NOW - 60, title='Newer', ctx=40_000)
        agents = [{'sessionId': SID_A, 'id': 'aaaaaaaa', 'kind': 'background', 'status': 'idle', 'state': 'done', 'pid': 1}]
        r = self.sb.rpc({'verb': 'list'}, NOW, agents=agents)
        self.assertTrue(r['ok'])
        s = r['sessions']
        self.assertEqual([x['title'] for x in s], ['Newer', 'Older'])
        a = s[1]
        self.assertEqual(a['sid'], SID_A)
        self.assertEqual(a['bg_id'], 'aaaaaaaa')
        self.assertEqual(a['kind'], 'background')
        self.assertEqual(a['ctx'], 200_000)
        self.assertEqual(a['folder'], 'proj')
        self.assertEqual(a['url'], 'https://claude.ai/code/session_XYZ123')
        self.assertFalse(a['handoff'])
        self.assertIsNone(s[0]['kind'])
        self.assertIn('root', r)

    def test_skips_old_and_caps(self):
        self.transcript(SID_A, str(self.root), NOW - 50 * 86400)  # the list reaches back 45 days; the browser, all of it
        r = self.sb.rpc({'verb': 'list'}, NOW, agents=[])
        self.assertEqual(r['sessions'], [])

    def test_index_cache_reused(self):
        self.transcript(SID_A, str(self.root), NOW - 60)
        self.sb.rpc({'verb': 'list'}, NOW, agents=[])
        orig = self.sb.session_meta
        self.sb.session_meta = lambda p: (_ for _ in ()).throw(AssertionError('re-read'))
        try:
            r = self.sb.rpc({'verb': 'list'}, NOW, agents=[])
        finally:
            self.sb.session_meta = orig
        self.assertEqual(len(r['sessions']), 1)

    def test_dismiss_hides_until_new_activity(self):
        p = self.transcript(SID_A, str(self.root), NOW - 60)
        r = self.sb.rpc({'verb': 'dismiss', 'sid': SID_A}, NOW, agents=[])
        self.assertTrue(r['ok'])
        self.assertEqual(self.sb.rpc({'verb': 'list'}, NOW, agents=[])['sessions'], [])
        with open(p, 'a') as f:  # stopping a session appends bookkeeping; that is not use
            f.write(json.dumps({'type': 'cost-state', 'sessionId': SID_A}) + '\n')
        os.utime(p, (NOW + 5, NOW + 5))
        self.assertEqual(self.sb.rpc({'verb': 'list'}, NOW + 6, agents=[])['sessions'], [])
        with open(p, 'a') as f:
            f.write(json.dumps(assistant(_iso(NOW + 10), 'back again', usage={'input_tokens': 5})) + '\n')
        os.utime(p, (NOW + 10, NOW + 10))
        self.assertEqual(len(self.sb.rpc({'verb': 'list'}, NOW + 20, agents=[])['sessions']), 1)

    def test_dismiss_stops_a_background_session(self):
        self.transcript(SID_A, str(self.root), NOW - 60)
        agents = [{'sessionId': SID_A, 'id': 'aaaaaaaa', 'kind': 'background', 'status': 'busy', 'pid': 1}]
        self.sb.rpc({'verb': 'dismiss', 'sid': SID_A}, NOW, agents=agents)
        self.assertIn(['stop', 'aaaaaaaa'], self.calls)


class ActionTest(RpcBase):
    def test_stop_interactive_is_refused_readably(self):
        agents = [{'sessionId': SID_A, 'kind': 'interactive', 'status': 'idle', 'pid': 1}]
        r = self.sb.rpc({'verb': 'stop', 'sid': SID_A}, NOW, agents=agents)
        self.assertFalse(r['ok'])
        self.assertIn('terminal', r['error'])
        self.assertEqual(self.calls, [])

    def test_stop_background(self):
        agents = [{'sessionId': SID_A, 'id': 'aaaaaaaa', 'kind': 'background', 'status': 'busy', 'pid': 1}]
        self.assertTrue(self.sb.rpc({'verb': 'stop', 'sid': SID_A}, NOW, agents=agents)['ok'])
        self.assertEqual(self.calls, [['stop', 'aaaaaaaa']])

    def test_stop_that_claude_does_not_confirm_is_an_error(self):
        """`claude stop` that could not confirm (or printed nothing) must not come back as ok: the chat would keep
        running while the apps say it stopped."""
        agents = [{'sessionId': SID_A, 'id': 'aaaaaaaa', 'kind': 'background', 'status': 'idle', 'pid': 1}]
        for said in ('', "couldn't confirm aaaaaaaa was stopped"):
            self.sb.run_claude_cmd = lambda args, **kw: said
            r = self.sb.rpc({'verb': 'stop', 'sid': SID_A}, NOW, agents=agents)
            self.assertFalse(r['ok'], said)
            self.assertIn('could not confirm', r['error'])

    def test_stop_of_a_stopped_chat_is_ok_without_calling_claude(self):
        agents = [{'sessionId': SID_A, 'id': 'aaaaaaaa', 'kind': 'background', 'state': 'done'}]
        r = self.sb.rpc({'verb': 'stop', 'sid': SID_A}, NOW, agents=agents)
        self.assertEqual((r['ok'], r.get('note')), (True, 'not running'))
        self.assertEqual(self.calls, [])

    def test_pass_sets_one_time_pass(self):
        self.assertTrue(self.sb.rpc({'verb': 'pass', 'sid': SID_A}, NOW, agents=[])['ok'])
        self.assertTrue(self.sb.pass_take(SID_A, NOW))
        self.assertFalse(self.sb.pass_take(SID_A, NOW))

    def test_bad_requests(self):
        for req in [None, [], {}, {'verb': 'nope'}, {'verb': 'stop', 'sid': '../x'}, {'verb': 'mkdir', 'folder': '../x'}]:
            r = self.sb.rpc(req, NOW, agents=[])
            self.assertFalse(r['ok'], req)
            self.assertIsInstance(r['error'], str)


class MkdirTest(RpcBase):
    def test_mkdir_creates_and_pretrusts(self):
        self.cjson.write_text(json.dumps({'numStartups': 5, 'projects': {'/other': {'x': 1}}}))
        r = self.sb.rpc({'verb': 'mkdir', 'folder': 'proj/fresh idea'}, NOW, agents=[])
        self.assertTrue(r['ok'], r)
        p = (self.root / 'proj' / 'fresh idea').resolve()
        self.assertTrue(p.is_dir())
        d = json.loads(self.cjson.read_text())
        self.assertEqual(d['numStartups'], 5)
        self.assertEqual(d['projects']['/other'], {'x': 1})
        self.assertTrue(d['projects'][self.sb.trust_key(p)]['hasTrustDialogAccepted'])
        self.assertTrue(list(self.tmp.glob('claude.json.bak-sb-*')))
        self.assertEqual(r['folder'], 'proj/fresh idea')

    def test_mkdir_skips_write_when_ancestor_trusted(self):
        self.cjson.write_text(json.dumps({'projects': {self.sb.trust_key(self.root.resolve()): {'hasTrustDialogAccepted': True}}}))
        before = self.cjson.read_text()
        self.assertTrue(self.sb.rpc({'verb': 'mkdir', 'folder': 'deep/er'}, NOW, agents=[])['ok'])
        self.assertEqual(self.cjson.read_text(), before)

    def test_folders_lists_subdirs_and_recent(self):
        (self.root / '.hidden').mkdir()
        self.transcript(SID_A, str(self.root / 'proj'), NOW - 60)
        r = self.sb.rpc({'verb': 'folders'}, NOW, agents=[])
        names = [f['folder'] for f in r['folders']]
        self.assertEqual(names[0], 'proj')
        self.assertNotIn('.hidden', names)

    def test_folder_tree_one_level_at_a_time(self):
        (self.root / 'proj' / 'src' / 'deep').mkdir(parents=True)
        (self.root / 'proj' / '.git').mkdir()
        for d in ('empty', 'bad;name', 'node_modules', '.hidden'):
            (self.root / d).mkdir()
        self.transcript(SID_A, str(self.root / 'proj' / 'src'), NOW - 60)
        r = self.sb.rpc({'verb': 'folders', 'parent': ''}, NOW, agents=[])
        self.assertTrue(r['ok'] and r['root'])
        self.assertEqual([(f['folder'], f['kids']) for f in r['folders']], [('empty', False), ('proj', True)])
        self.assertEqual([bool(f['recent']) for f in r['folders']], [False, True])  # a session ran inside proj
        r = self.sb.rpc({'verb': 'folders', 'parent': 'proj'}, NOW, agents=[])
        self.assertEqual([(f['folder'], f['name'], f['kids']) for f in r['folders']], [('proj/src', 'src', True)])
        for bad in ('../x', '/etc', 'nope', 7):
            self.assertFalse(self.sb.rpc({'verb': 'folders', 'parent': bad}, NOW, agents=[])['ok'], bad)

    def test_files_browses_inside_the_root_only(self):
        (self.root / 'proj' / 'src').mkdir(parents=True)
        (self.root / 'proj' / '.git').mkdir()
        (self.root / 'proj' / 'README.md').write_text('hello')
        (self.root / 'proj' / 'blob.bin').write_bytes(b'\x00\x01')
        (self.root / 'proj' / '.env').write_text('KEY=1')
        (self.root / 'proj' / 'server.pem').write_text('-----BEGIN')
        (self.root / 'proj' / 'out.txt').symlink_to('/etc/hosts')
        r = self.sb.rpc({'verb': 'files', 'folder': 'proj'}, NOW, agents=[])
        self.assertEqual([d['name'] for d in r['dirs']], ['src'])
        self.assertEqual([(f['name'], f['open']) for f in r['files']], [('blob.bin', True), ('README.md', True), ('server.pem', False)])
        f = self.sb.rpc({'verb': 'files', 'folder': 'proj', 'name': 'README.md'}, NOW, agents=[])['file']
        self.assertEqual((f['text'], f['binary'], f['cut']), ('hello', False, False))
        self.assertTrue(self.sb.rpc({'verb': 'files', 'folder': 'proj', 'name': 'blob.bin'}, NOW, agents=[])['file']['binary'])
        f = self.sb.rpc({'verb': 'files', 'folder': 'proj', 'name': 'server.pem'}, NOW, agents=[])['file']
        self.assertTrue(f['secret'])
        self.assertNotIn('text', f)
        for req in ({'folder': '../x'}, {'folder': '/etc'}, {'folder': 'proj', 'name': '.env'}, {'folder': 'proj', 'name': 'out.txt'},
                    {'folder': 'proj', 'name': '../README.md'}, {'folder': 'nope'}, {'folder': 7}):
            self.assertFalse(self.sb.rpc({'verb': 'files', **req}, NOW, agents=[])['ok'], req)

    def test_a_worktree_session_lists_its_repo(self):
        self.transcript(SID_A, str(self.root / 'proj' / '.claude' / 'worktrees' / 'fix-x'), NOW - 60)
        names = [f['folder'] for f in self.sb.rpc({'verb': 'folders'}, NOW, agents=[])['folders']]
        self.assertEqual(names[0], 'proj')
        self.assertFalse([n for n in names if 'worktrees' in n])


if __name__ == '__main__':
    unittest.main()


class GlanceFieldsTest(RpcBase):
    def test_limits_carry_reset_times(self):
        self.sb.write_state({'session_id': SID_A, 'rate_limits': {'five_hour': {'used_percentage': 16, 'resets_at': NOW + 3600},
                                                                  'seven_day': {'used_percentage': 49, 'resets_at': NOW + 86400}}}, NOW)
        lim = self.sb.latest_limits()
        self.assertEqual((lim['five_hour_reset'], lim['seven_day_reset']), (NOW + 3600, NOW + 86400))

    def test_session_reports_cache_time_left(self):
        self.transcript(SID_A, str(self.root), NOW - 60)
        self.sb.write_state({'session_id': SID_A, 'prompt_cache': {'expires_at': NOW + 1800}}, NOW)
        s = self.sb.rpc({'verb': 'list'}, NOW, agents=[])['sessions'][0]
        self.assertEqual(s['cache_left'], 1800)
        self.assertFalse(s['cold'])


class ActivityTest(RpcBase):
    def test_bookkeeping_lines_do_not_count_as_activity(self):
        # A late non-message line (e.g. artifact ledger) bumps mtime; activity and cache come from the last reply.
        import datetime
        last = datetime.datetime(2026, 10, 1, 0, 0, 5, tzinfo=datetime.timezone.utc).timestamp()
        self.transcript(SID_A, str(self.root), NOW - 60)
        self.sb.write_state({'session_id': SID_A, 'prompt_cache': {'expires_at': NOW + 600}}, last + 10)
        s = self.sb.rpc({'verb': 'list'}, NOW, agents=[])['sessions'][0]
        self.assertEqual(s['active'], last)
        self.assertEqual(s['cache_left'], 600)
