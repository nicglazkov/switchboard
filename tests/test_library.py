"""The tool library: bundles (and attacks on them), packing skills and MCP servers, the push to a machine, chats started
with --plugin-dir, secrets through tool-run, and the server's store, publish, connect, rollback, defaults, delete,
attach and starts, against a fleet whose machines run the real agent code."""
import base64, hashlib, io, json, os, sys, tarfile, threading, time, unittest
from pathlib import Path
from tempfile import mkdtemp
from unittest import mock

from tests.helpers import fresh_sb
from tests.test_jobs import JobBase
from tests.test_rpc import NOW, SID_A, SID_B

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'server'))
import fleet, server, library  # noqa: E402
from tests.test_server import sess  # noqa: E402

SID_C = 'cccccccc-1111-2222-3333-444444444444'


class FakeKeys:
    kind, warning = 'fake', None

    def __init__(self, d=None):
        self.d = dict(d or {})

    def get(self, tool, name):
        return self.d.get(f'{tool}/{name}')

    def has(self, tool, name):
        return f'{tool}/{name}' in self.d

    def set(self, tool, name, value):
        self.d[f'{tool}/{name}'] = value

    def rm(self, tool, name):
        return self.d.pop(f'{tool}/{name}', None) is not None


def tgz(members):
    """A tar.gz from [(TarInfo-ish dict)]: name, data or type/linkname."""
    buf = io.BytesIO()
    with tarfile.open(fileobj=buf, mode='w:gz') as tf:
        for m in members:
            ti = tarfile.TarInfo(m['name'])
            data = m.get('data', b'')
            if 'type' in m:
                ti.type = m['type']
                ti.linkname = m.get('linkname', '')
                tf.addfile(ti)
            else:
                ti.size = len(data)
                tf.addfile(ti, io.BytesIO(data))
    return buf.getvalue()


def plugin_json(name='demo'):
    return {'name': '.claude-plugin/plugin.json', 'data': json.dumps({'name': name, 'description': 'A demo'}).encode()}


SKILL = {'name': 'skills/hello/SKILL.md', 'data': b'---\nname: hello\ndescription: Says hello\n---\nSay hello.\n'}


def make_plugin(root, name='demo', secrets=None, mcp=True, skill=True):
    root = Path(root)
    (root / '.claude-plugin').mkdir(parents=True)
    (root / '.claude-plugin' / 'plugin.json').write_text(json.dumps({'name': name, 'description': 'Demo tool'}))
    if secrets is not None:
        (root / '.claude-plugin' / 'switchboard.json').write_text(json.dumps({'secrets': [{'name': s, 'about': 'k'} for s in secrets]}))
    if skill:
        (root / 'skills' / 'hello').mkdir(parents=True)
        (root / 'skills' / 'hello' / 'SKILL.md').write_text('---\nname: hello\ndescription: Says hello\n---\nhi\n')
    if mcp:
        (root / 'files').mkdir()
        (root / 'files' / 'server.py').write_text('print(1)\n')
        (root / '.mcp.json').write_text(json.dumps({'mcpServers': {'demo': {
            'command': 'python3', 'args': ['${CLAUDE_PLUGIN_ROOT}/files/server.py'], 'env': {'MODE': 'x', **{s: '' for s in secrets or []}}}}}))
    return root


class BundleTest(unittest.TestCase):
    def setUp(self):
        self.sb = fresh_sb(mkdtemp())

    def both(self, data, tool=None):
        """The agent and the server agree on every bundle: the same answer, or both refuse."""
        a = s = None
        try:
            a = self.sb.bundle_info(data, tool)
        except ValueError as e:
            a = ('refused', str(e))
        try:
            s = library.check_bundle(data, tool)
        except ValueError as e:
            s = ('refused', str(e))
        self.assertEqual(a, s)
        return a

    def test_a_plain_bundle_is_read(self):
        info = self.both(tgz([plugin_json(), SKILL, {'name': '.mcp.json', 'data': b'{"mcpServers": {"x": {"command": "y"}}}'}]))
        self.assertEqual((info['name'], info['kinds'], info['skills'], info['servers']), ('demo', ['skill', 'mcp'], ['hello'], ['x']))
        self.assertFalse(info['skill_secrets'])
        sbj = {'name': '.claude-plugin/switchboard.json', 'data': json.dumps({'secrets': [{'name': 'API_KEY', 'about': 'a'}]}).encode()}
        info = self.both(tgz([plugin_json(), SKILL, sbj]))
        self.assertEqual(info['secrets'], [{'name': 'API_KEY', 'about': 'a'}])
        self.assertTrue(info['skill_secrets'])  # a skill script could read it through Bash: flagged

    def test_attacks_are_refused(self):
        base = [plugin_json(), SKILL]
        attacks = {
            'absolute': [{'name': '/etc/evil', 'data': b'x'}],
            'dotdot': [{'name': 'skills/../../evil', 'data': b'x'}],
            'dotdot-only': [{'name': '..', 'type': tarfile.DIRTYPE}],
            'backslash': [{'name': 'skills\\..\\evil', 'data': b'x'}],
            'drive': [{'name': 'C:/evil', 'data': b'x'}],
            'symlink': [{'name': 'skills/hello/link', 'type': tarfile.SYMTYPE, 'linkname': '/etc/passwd'}],
            'hardlink': [{'name': 'skills/hello/hard', 'type': tarfile.LNKTYPE, 'linkname': '.claude-plugin/plugin.json'}],
            'chardev': [{'name': 'dev', 'type': tarfile.CHRTYPE}],
            'fifo': [{'name': 'pipe', 'type': tarfile.FIFOTYPE}],
            'twice': [{'name': 'SKILLS/hello/skill.md', 'data': b'x'}],
        }
        for why, extra in attacks.items():
            with self.subTest(why):
                r = self.both(tgz(base + extra))
                self.assertEqual(r[0], 'refused', why)
        for why, data in {'no plugin.json': tgz([SKILL]), 'nothing to use': tgz([plugin_json()]),
                          'not gzip': b'hello', 'empty': b'', 'bad name': tgz([plugin_json('Bad Name'), SKILL]),
                          'bad json': tgz([{'name': '.claude-plugin/plugin.json', 'data': b'{'}, SKILL])}.items():
            with self.subTest(why):
                self.assertEqual(self.both(data)[0], 'refused', why)
        self.assertIn('not', self.both(tgz([plugin_json(), SKILL]), 'other')[1])  # plugin.json names another tool

    def test_caps(self):
        big = tgz([plugin_json(), SKILL, {'name': 'files/blob', 'data': os.urandom(4096)}])
        for mod in (self.sb, library):
            with mock.patch.object(mod, 'BUNDLE_MAX', 1000):
                self.assertRaisesRegex(ValueError, 'over', (mod.bundle_info if mod is self.sb else mod.check_bundle), big)
            with mock.patch.object(mod, 'BUNDLE_UNPACKED_MAX', 2000):
                self.assertRaisesRegex(ValueError, 'unpacks', (mod.bundle_info if mod is self.sb else mod.check_bundle), big)
            with mock.patch.object(mod, 'BUNDLE_FILES_MAX', 2):
                self.assertRaisesRegex(ValueError, 'more than', (mod.bundle_info if mod is self.sb else mod.check_bundle), big)

    def test_extract_writes_plain_files_and_no_odd_modes(self):
        buf = io.BytesIO()
        with tarfile.open(fileobj=buf, mode='w:gz') as tf:
            for name, data, mode in (('.claude-plugin/plugin.json', json.dumps({'name': 'demo'}).encode(), 0o4777),
                                     ('skills/hello/SKILL.md', b'hi', 0o644), ('skills/hello/run.sh', b'#!/bin/sh\n', 0o755)):
                ti = tarfile.TarInfo(name)
                ti.size, ti.mode = len(data), mode
                tf.addfile(ti, io.BytesIO(data))
        dest = Path(mkdtemp()) / 'out'
        self.sb.extract_bundle(buf.getvalue(), dest)
        self.assertEqual((dest / 'skills/hello/SKILL.md').read_bytes(), b'hi')
        if os.name != 'nt':
            self.assertEqual((dest / '.claude-plugin/plugin.json').stat().st_mode & 0o7777, 0o755)  # no setuid
            self.assertEqual((dest / 'skills/hello/SKILL.md').stat().st_mode & 0o7777, 0o644)

    def test_tool_names(self):
        for ok in ('a', 'web-fetch', 'my-tool-2', 'x' * 40):
            self.assertTrue(self.sb.valid_tool(ok) and library.valid_tool(ok), ok)
        for bad in ('', '-a', 'A', 'a_b', 'a b', '../x', 'x' * 41, None, 3, 'a/b', 'a.b'):
            self.assertFalse(self.sb.valid_tool(bad) or library.valid_tool(bad), bad)
        for s in ('My Skill!', 'web-fetch', '  --x--  ', '', 'ÜBER tool'):
            self.assertEqual(self.sb.tool_slug(s), library.tool_slug(s))
        self.assertEqual(self.sb.tool_slug('My Skill!'), 'my-skill')


class PackTest(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(mkdtemp())
        self.home = self.tmp / 'home'
        self.home.mkdir()
        os.environ.update(SB_USER_HOME=str(self.home), SB_CLAUDE_DIR=str(self.home / '.claude'),
                          SB_CLAUDE_JSON=str(self.home / '.claude.json'), SB_ROOT=str(self.home))
        self.sb = fresh_sb(self.tmp)

    def tearDown(self):
        for k in ('SB_USER_HOME', 'SB_CLAUDE_DIR', 'SB_CLAUDE_JSON', 'SB_ROOT'):
            os.environ.pop(k, None)

    def files_of(self, data):
        with tarfile.open(fileobj=io.BytesIO(data), mode='r:gz') as tf:
            return {m.name: tf.extractfile(m).read() for m in tf if m.isfile()}

    def test_a_skill_folder_is_wrapped_into_the_plugin_layout(self):
        d = self.home / '.claude' / 'skills' / 'Web Search'
        (d / 'scripts').mkdir(parents=True)
        (d / 'SKILL.md').write_text('---\nname: web\ndescription: "Search the web"\n---\nUse scripts/s.py\n')
        (d / 'scripts' / 's.py').write_text('print(1)')
        (d / '.git').mkdir()
        (d / '.git' / 'HEAD').write_text('ref')
        if os.name != 'nt':
            os.symlink('/etc/passwd', d / 'scripts' / 'link')
        data, info, skipped = self.sb.pack_folder(d)
        files = self.files_of(data)
        self.assertEqual(info['name'], 'web-search')
        self.assertEqual(info['kinds'], ['skill'])
        self.assertEqual(info['description'], 'Search the web')
        self.assertIn('skills/web-search/SKILL.md', files)
        self.assertIn('skills/web-search/scripts/s.py', files)
        self.assertEqual(json.loads(files['.claude-plugin/plugin.json'])['name'], 'web-search')
        self.assertEqual(json.loads(files['.claude-plugin/switchboard.json'])['mode'], 'local')
        self.assertFalse(any('.git' in f or f.endswith('link') for f in files))
        self.assertIn('.git', skipped)
        self.assertEqual(self.sb.pack_folder(d)[0], data)  # the same files pack to the same bytes
        src = self.sb.library_sources()
        self.assertEqual([s['name'] for s in src['skills']], ['Web Search'])

    def test_a_plugin_folder_keeps_its_layout_and_takes_a_new_name(self):
        p = make_plugin(self.tmp / 'plug', 'demo', secrets=['API_KEY'])
        data, info, _ = self.sb.pack_folder(p, 'renamed', 'Better words')
        files = self.files_of(data)
        self.assertEqual(json.loads(files['.claude-plugin/plugin.json'])['name'], 'renamed')
        self.assertEqual(info['description'], 'Better words')
        self.assertEqual(info['secrets'], [{'name': 'API_KEY', 'about': 'k'}])
        self.assertIn('files/server.py', files)
        self.assertRaisesRegex(ValueError, 'neither', self.sb.pack_folder, self.tmp)
        self.assertRaisesRegex(ValueError, 'lowercase', self.sb.pack_folder, p, 'Bad')

    def test_mcp_import_drops_env_values_and_copies_home_paths(self):
        srv = self.home / 'tools' / 'scrap'
        srv.mkdir(parents=True)
        (srv / 'run.sh').write_text('#!/bin/sh\n')
        (srv / 'server.py').write_text('print(2)')
        (srv / 'lib').mkdir()
        (srv / 'lib' / 'x.py').write_text('x')
        conf = {'mcpServers': {'scrap': {'command': str(srv / 'run.sh'), 'args': ['~/tools/scrap/server.py', '--port', '8080', '/usr/bin/env'],
                                         'env': {'WEB_FETCH_KEY': 'super-secret-value-123', 'HOME_DIR': '/x'}},
                               'leaky': {'command': 'node', 'args': ['--api-key', 'sk-abcdef0123456789abcdef']},
                               'remote': {'type': 'http', 'url': 'https://mcp.example.com/mcp', 'headers': {'Authorization': 'Bearer x'}}}}
        (self.home / '.claude.json').write_text(json.dumps(conf))
        src = {m['name']: m for m in self.sb.library_sources()['mcp']}
        self.assertEqual(src['scrap']['env_names'], ['HOME_DIR', 'WEB_FETCH_KEY'])
        self.assertNotIn('super-secret', json.dumps(src))
        r = self.sb.rpc({'verb': 'library_pack', 'kind': 'mcp', 'source': 'scrap'}, NOW, agents=[])
        self.assertTrue(r['ok'], r)
        self.assertEqual((r['tool'], r['secrets']), ('scrap', ['WEB_FETCH_KEY', 'HOME_DIR']))
        buf = b''
        while True:  # small chunks, the way the server pulls them
            c = self.sb.rpc({'verb': 'library_chunk', 'id': r['id'], 'offset': len(buf), 'size': 100}, NOW, agents=[])
            buf += base64.b64decode(c['data'])
            if c['last']:
                break
        self.assertEqual(hashlib.sha256(buf).hexdigest(), r['sha256'])
        files = self.files_of(buf)
        mcp = json.loads(files['.mcp.json'])['mcpServers']['scrap']
        self.assertEqual(mcp['command'], '${CLAUDE_PLUGIN_ROOT}/files/scrap/run.sh')
        self.assertEqual(mcp['args'], ['${CLAUDE_PLUGIN_ROOT}/files/scrap/server.py', '--port', '8080', '/usr/bin/env'])
        self.assertNotIn('env', mcp)
        self.assertIn('files/scrap/lib/x.py', files)
        self.assertEqual(json.loads(files['.claude-plugin/switchboard.json'])['secrets'][0]['name'], 'WEB_FETCH_KEY')
        self.assertNotIn(b'super-secret', b''.join(files.values()))
        self.assertFalse(self.sb.rpc({'verb': 'library_chunk', 'id': r['id'], 'offset': 0}, NOW, agents=[])['ok'])  # pulled: gone
        bad = self.sb.rpc({'verb': 'library_pack', 'kind': 'mcp', 'source': 'leaky'}, NOW, agents=[])
        self.assertIn('looks like a key', bad['error'])
        self.assertIn('headers', self.sb.rpc({'verb': 'library_pack', 'kind': 'mcp', 'source': 'remote'}, NOW, agents=[])['error'])
        self.assertIn('no MCP server', self.sb.rpc({'verb': 'library_pack', 'kind': 'mcp', 'source': 'nope'}, NOW, agents=[])['error'])
        self.assertIn('lowercase', self.sb.rpc({'verb': 'library_pack', 'kind': 'mcp', 'source': 'scrap', 'tool': '../x'}, NOW, agents=[])['error'])

    def test_a_command_in_the_home_folder_itself_copies_only_that_file(self):
        (self.home / 'srv.py').write_text('print(3)')
        (self.home / 'private.txt').write_text('nope')
        entries, secrets = self.sb.mcp_bundle('solo', {'command': 'python3', 'args': [str(self.home / 'srv.py')]}, 'solo')
        names = [e[0] for e in entries]
        self.assertIn('files/srv/srv.py', names)
        self.assertFalse(any('private' in n for n in names))
        entries, _ = self.sb.mcp_bundle('h', {'command': 'x', 'args': ['~', str(self.home)]}, 'h')  # the home folder is never copied
        self.assertEqual(json.loads(dict(entries)['.mcp.json'])['mcpServers']['h']['args'], ['~', str(self.home)])
        self.assertFalse(any(n.startswith('files/') for n, *_ in entries))


class InstallTest(JobBase):
    """The push to a machine, launch with --plugin-dir, the MCP wrapper, tool-run and the secret CLI."""
    def setUp(self):
        super().setUp()
        self.keys = FakeKeys()
        self.sb.KEYCHAIN = self.keys

    def put(self, folder, tool, version, step=300, agents=None):
        data, _, _ = self.sb.pack_folder(folder, tool)
        sha, r = hashlib.sha256(data).hexdigest(), None
        for off in range(0, len(data), step):
            r = self.sb.rpc({'verb': 'library_put', 'tool': tool, 'version': version, 'sha256': sha, 'offset': off,
                             'data': base64.b64encode(data[off:off + step]).decode(), 'done': off + step >= len(data)}, NOW, agents=agents or [])
            self.assertTrue(r['ok'], r)
            if r.get('skipped'):
                break
        return r, data

    def test_put_round_trip_skip_and_rollback(self):
        p = make_plugin(self.tmp / 'p1', 'demo')
        r, data = self.put(p, 'demo', 1)
        self.assertEqual(r, {'ok': True, 'installed': 1})
        d = self.sb.tool_dir('demo')
        self.assertEqual(d, self.sb.lib_root() / 'demo' / '1')
        self.assertTrue((d / 'skills' / 'hello' / 'SKILL.md').is_file())
        r, _ = self.put(p, 'demo', 1)
        self.assertTrue(r.get('skipped'))
        (p / 'files' / 'server.py').write_text('print(2)\n')
        self.put(p, 'demo', 2)
        self.assertEqual(self.sb.tool_dir('demo').name, '2')
        self.assertFalse((self.sb.lib_root() / 'demo' / '1').exists())  # nobody runs v1: pruned
        sha = hashlib.sha256(data).hexdigest()
        bad = self.sb.rpc({'verb': 'library_put', 'tool': 'demo', 'version': 3, 'sha256': 'f' * 64, 'offset': 0,
                           'data': base64.b64encode(data).decode(), 'done': True}, NOW, agents=[])
        self.assertIn('damaged', bad['error'])
        lost = self.sb.rpc({'verb': 'library_put', 'tool': 'demo', 'version': 3, 'sha256': sha, 'offset': 50,
                            'data': base64.b64encode(data[50:]).decode(), 'done': True}, NOW, agents=[])
        self.assertIn('lost its place', lost['error'])
        for req in ({'tool': '../demo'}, {'tool': 'demo', 'version': 0}, {'tool': 'demo', 'version': 1, 'sha256': 'x'}):
            r = self.sb.rpc(dict({'verb': 'library_put', 'version': 1, 'sha256': sha, 'offset': 0, 'data': ''}, **req), NOW, agents=[])
            self.assertFalse(r['ok'], req)
        self.assertEqual(self.sb.tool_dir('demo').name, '2')

    def test_a_running_chat_keeps_its_version(self):
        p = make_plugin(self.tmp / 'p1', 'demo')
        self.put(p, 'demo', 1)
        self.sb.chat_tools_put(SID_A, ['demo'], {'demo': 1})
        running = [{'sessionId': SID_A, 'status': 'busy', 'kind': 'background'}]
        (p / 'files' / 'server.py').write_text('print(2)\n')
        self.put(p, 'demo', 2, agents=running)
        self.assertTrue((self.sb.lib_root() / 'demo' / '1').is_dir())
        r = self.sb.library_drop({'tool': 'demo'}, lambda: running)
        self.assertEqual(r['kept'], [1])
        self.assertIsNone(self.sb.tool_dir('demo'))
        self.assertFalse((self.sb.lib_root() / 'demo' / '2').exists())
        self.sb.library_drop({'tool': 'demo'}, lambda: [])
        self.assertFalse((self.sb.lib_root() / 'demo').exists())

    def test_mcp_servers_of_a_tool_with_secrets_run_through_tool_run(self):
        self.put(make_plugin(self.tmp / 'p1', 'keyed', secrets=['API_KEY']), 'keyed', 1)
        self.put(make_plugin(self.tmp / 'p2', 'plain'), 'plain', 1)
        conf = json.loads((self.sb.tool_dir('keyed') / '.mcp.json').read_text())['mcpServers']['demo']
        py, sb_path = self.sb.sb_command()
        self.assertEqual(conf['command'], py)
        self.assertEqual(conf['args'], [sb_path, 'tool-run', 'keyed', '--', 'python3', '${CLAUDE_PLUGIN_ROOT}/files/server.py'])
        self.assertEqual(conf['env'], {'MODE': 'x'})  # tool-run sets the secret
        plain = json.loads((self.sb.tool_dir('plain') / '.mcp.json').read_text())['mcpServers']['demo']
        self.assertEqual(plain['command'], 'python3')

    def test_tool_run_puts_secrets_in_the_env_and_execs(self):
        self.put(make_plugin(self.tmp / 'p1', 'keyed', secrets=['API_KEY', 'OTHER']), 'keyed', 1)
        got, err = [], io.StringIO()
        fake = lambda f, argv, env: got.append((f, argv, env))
        rc = self.sb.tool_run(['keyed', '--', 'node', 'srv.js'], env={'PATH': '/bin'}, execvpe=fake, err=err)
        self.assertEqual(rc, 1)
        self.assertIn('needs API_KEY, OTHER', err.getvalue())
        self.assertEqual(got, [])
        self.keys.set('keyed', 'API_KEY', 'sekrit-one')
        self.keys.set('keyed', 'OTHER', 'v2')
        rc = self.sb.tool_run(['keyed', '--', 'node', 'srv.js'], env={'PATH': '/bin', 'API_KEY': 'old'}, execvpe=fake, err=err,
                              run=lambda argv, env: mock.Mock(returncode=0))
        self.assertEqual(rc, 0)
        if os.name != 'nt':
            f, argv, env = got[0]
            self.assertEqual((f, argv), ('node', ['node', 'srv.js']))
            self.assertEqual((env['API_KEY'], env['OTHER'], env['PATH']), ('sekrit-one', 'v2', '/bin'))
        self.assertEqual(self.sb.tool_run(['Bad', '--', 'x'], err=err), 2)
        self.assertEqual(self.sb.tool_run(['keyed', 'x'], err=err), 2)
        log = (self.sb.paths()['log']).read_text() if self.sb.paths()['log'].exists() else ''
        self.assertNotIn('sekrit', log)

    def test_secret_cli_and_state_never_show_values(self):
        self.put(make_plugin(self.tmp / 'p1', 'keyed', secrets=['API_KEY']), 'keyed', 1)
        out = io.StringIO()
        st = self.sb.library_state(self.keys)
        self.assertEqual(st['tools']['keyed']['missing_secrets'], ['API_KEY'])
        self.assertEqual(st['tools']['keyed']['installed'], 1)
        self.assertEqual(self.sb.secret_cli(['set', 'keyed', 'API_KEY'], keys=self.keys, ask=lambda p: 'hunter2-value', out=out), 0)
        self.assertEqual(self.keys.get('keyed', 'API_KEY'), 'hunter2-value')
        self.assertEqual(self.sb.secret_cli(['list'], keys=self.keys, out=out), 0)
        self.assertIn('keyed: API_KEY set', out.getvalue())
        self.assertNotIn('hunter2', out.getvalue())
        st = self.sb.rpc({'verb': 'library_state'}, NOW, agents=[])
        self.assertEqual(st['tools']['keyed']['missing_secrets'], [])
        self.assertNotIn('hunter2', json.dumps(st))
        self.assertEqual(self.sb.secret_cli(['set', '../x', 'API_KEY'], keys=self.keys, ask=lambda p: 'v', out=out), 2)
        self.assertEqual(self.sb.secret_cli(['set', 'keyed', 'BAD-NAME'], keys=self.keys, ask=lambda p: 'v', out=out), 2)
        self.assertEqual(self.sb.secret_cli(['set', 'keyed', 'API_KEY'], keys=self.keys, ask=lambda p: 'two\nlines', out=out), 1)
        self.assertEqual(self.sb.secret_cli(['rm', 'keyed', 'API_KEY'], keys=self.keys, out=out), 0)
        self.assertIsNone(self.keys.get('keyed', 'API_KEY'))
        self.assertEqual(self.sb.secret_cli([], keys=self.keys, out=out), 2)

    def test_file_secrets_are_mode_600_and_the_mac_keychain_never_sees_the_value_in_argv(self):
        fs = self.sb.FileSecrets(self.tmp / 'secrets.json')
        fs.set('t', 'K', 'value-1')
        self.assertEqual(fs.get('t', 'K'), 'value-1')
        if os.name != 'nt':
            self.assertEqual((self.tmp / 'secrets.json').stat().st_mode & 0o777, 0o600)
        self.assertTrue(fs.rm('t', 'K'))
        self.assertFalse(fs.has('t', 'K'))
        calls = []

        def run(argv, input=None, **kw):
            calls.append((argv, input))
            return mock.Mock(returncode=0, stdout='', stderr='')
        with mock.patch.object(self.sb.subprocess, 'run', run):
            self.sb.MacKeychain().set('t', 'K', 'p"a\\ss word')
        argv, stdin = calls[0]
        self.assertEqual(argv, ['security', '-i'])
        self.assertIn('-w "p\\"a\\\\ss word"', stdin)
        self.assertTrue(all('p"a' not in ' '.join(a) for a, _ in calls))

    def test_launch_adds_plugin_dirs_and_skips_a_missing_tool(self):
        self.put(make_plugin(self.tmp / 'p1', 'demo'), 'demo', 1)
        argv = self.sb.launch_argv({'name': 'x', 'tools': ['demo', 'gone'], 'prompt': 'hi'}, exe='claude')
        i = argv.index('--plugin-dir')
        self.assertEqual(argv[i + 1], str(self.sb.lib_root() / 'demo' / '1'))
        self.assertEqual(argv.count('--plugin-dir'), 1)
        self.assertEqual(argv[-1], 'hi')
        self.assertNotIn('--plugin-dir', self.sb.launch_argv({'name': 'x'}, exe='claude'))

    def test_start_records_the_list_and_a_resume_carries_it(self):
        self.put(make_plugin(self.tmp / 'p1', 'demo'), 'demo', 1)
        r = self.sb.rpc({'verb': 'start', 'folder': 'proj', 'prompt': 'go', 'tools': ['demo', 'gone']}, NOW, agents=[])
        self.assertTrue(r['ok'], r)
        with mock.patch.object(self.sb, 'bg_session_id', lambda bg: SID_B):
            self.drain()
        res = self.sb._jget(self.sb.paths_q()['queue'] / f'{r["job"]}.result.json')
        self.assertEqual(res['missing_tools'], ['gone'])
        self.assertIn('--plugin-dir', self.launched[0]['argv'])
        self.assertEqual(self.sb.chat_tools_get(SID_B), ['demo', 'gone'])
        self.assertEqual(self.sb._jget(self.sb.paths()['state'] / f'{SID_B}.tools')['versions'], {'demo': 1})
        self.transcript(SID_B, str(self.root / 'proj'), NOW - 100)
        self.job({'verb': 'resume', 'sid': SID_B})  # no list from the server: the chat's own
        with mock.patch.object(self.sb, 'bg_session_id', lambda bg: SID_C):
            self.drain()
        self.assertIn('--plugin-dir', self.launched[1]['argv'])
        self.assertEqual(self.sb.chat_tools_get(SID_C), ['demo', 'gone'])
        self.job({'verb': 'resume', 'sid': SID_B, 'tools': []})  # the server says none
        self.drain()
        self.assertNotIn('--plugin-dir', self.launched[2]['argv'])
        bad = self.sb.rpc({'verb': 'start', 'folder': 'proj', 'tools': ['../etc']}, NOW, agents=[])
        self.assertFalse(bad['ok'])

    def test_publish_tool_sends_chunks_the_server_can_put_back_together(self):
        p = make_plugin(self.tmp / 'p1', 'demo', secrets=['API_KEY'])
        (p / 'files' / 'big.bin').write_bytes(os.urandom(5000))
        sent = []

        def call(method, path, body=None):
            sent.append((method, path, body))
            return {'ok': True, 'tool': 'demo', 'version': 4} if body.get('done') else {'ok': True}
        with mock.patch.object(self.sb, 'PUBLISH_CHUNK', 1024):
            out, bad = self.sb.mcp_tool('publish_tool', {'path': 'p1', 'note': 'first'}, call=call, me='desk', cwd=str(self.tmp))
        self.assertFalse(bad, out)
        self.assertEqual((out['tool'], out['version'], out['secrets']), ('demo', 4, ['API_KEY']))
        self.assertIn('warning', out)  # skills plus secrets
        self.assertGreater(len(sent), 2)
        blob = b''.join(base64.b64decode(b['data']) for _, _, b in sent)
        done = sent[-1][2]['done']
        self.assertEqual(done['sha256'], hashlib.sha256(blob).hexdigest())
        self.assertEqual((done['tool'], done['note'], done['machine']), ('demo', 'first', 'desk'))
        self.assertEqual(self.sb.bundle_info(blob)['name'], 'demo')
        out, bad = self.sb.mcp_tool('publish_tool', {'path': str(self.tmp)}, call=call, me='desk')
        self.assertTrue(bad)
        out, bad = self.sb.mcp_tool('publish_tool', {'path': 'p1', 'tool': 'Bad'}, call=call, me='desk', cwd=str(self.tmp))
        self.assertTrue(bad)

    def test_the_agent_says_it_can(self):
        self.assertIn('library', self.sb.AGENT_FEATURES)


MACH = [{'name': 'desk', 'label': 'Desk', 'target': 'me@desk'}, {'name': 'lap', 'label': 'Laptop', 'target': 'node:lap'},
        {'name': 'old', 'label': 'Old', 'target': 'me@old'}]


class Fleet:
    """Machines that run the agent's real library verbs, each in its own SB_HOME, and scripted jobs."""
    def __init__(self, sb, tmp):
        self.sb, self.calls, self.off = sb, [], set()
        self.homes = {m['name']: Path(tmp) / f'home-{m["name"]}' for m in MACH}
        self.feats = {'desk': ['library', 'relaunch', 'flags'], 'lap': ['library'], 'old': []}
        self.lock = threading.Lock()

    def __call__(self, target, req, timeout):
        name = target.split('@')[-1].split(':')[-1]
        if name in self.off:
            raise OSError('ssh: timed out')
        with self.lock:
            self.calls.append((name, req))
            v = req['verb']
            if v == 'list':
                return {'ok': True, 'sessions': [sess(SID_A, kind='background', status='idle')], 'features': self.feats[name]}
            if v.startswith('library_'):
                self.sb.SB_HOME = self.homes[name]
                return self.sb.rpc(req, NOW, agents=[])
            if v in ('start', 'resume', 'fresh', 'compact'):
                return {'ok': True, 'job': '0123456789ab'}
            if v == 'mkdir':
                return {'ok': True, 'folder': req['folder']}
            if v == 'handoff':
                return {'ok': True, 'job': '0123456789ac'}
            if v == 'handoff_text':
                return {'ok': True, 'text': 'the brief', 'carry': {}}
            if v == 'job':
                return {'ok': True, 'state': 'done', 'sid': SID_C, 'name': 'x'}
            if v == 'busy':
                return {'ok': True, 'live': False}
            return {'ok': True}

    def verbs(self, name, verb):
        return [r for n, r in self.calls if n == name and r['verb'] == verb]


class ServerLibraryTest(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(mkdtemp())
        self.sb = fresh_sb(self.tmp)
        self.sb.KEYCHAIN = FakeKeys()
        self.f = Fleet(self.sb, self.tmp)
        self.state = self.tmp / 'state'
        self.app = server.App(fleet.Fleet(MACH, self.f), state_dir=self.state, web_dir=self.state, sleep=lambda s: None,
                              poll_every=0, retry_waits=(0, 0))
        self.app._refresh_soon = lambda *a, **k: None
        self.app.fleet.poll()
        self.lib = self.app.feat.library
        self.lib.background = lambda fn: fn()

    def install_of(self, name, tool):
        self.sb.SB_HOME = self.f.homes[name]
        d = self.sb.tool_dir(tool)
        return int(d.name) if d else None

    def publish(self, folder, tool, step=700, note=''):
        data, _, _ = self.sb.pack_folder(folder, tool)
        uid, r = os.urandom(16).hex(), None
        for off in range(0, len(data), step):
            body = {'upload': uid, 'offset': off, 'data': base64.b64encode(data[off:off + step]).decode()}
            if off + step >= len(data):
                body['done'] = {'tool': tool, 'sha256': hashlib.sha256(data).hexdigest(), 'note': note, 'machine': 'desk'}
            r = self.app.post('/api/library/publish', body)
        return r

    def chat(self, key):
        """A chat's recorded list, once the job that started it has been recorded (just after it shows done)."""
        for _ in range(300):
            if key in self.lib.d['chats']:
                return self.lib.d['chats'][key]
            time.sleep(0.01)
        self.fail(f'{key} was never recorded')

    def wait(self, jid):
        for _ in range(300):
            j = self.app.jobs[jid]
            if j['state'] in ('done', 'error'):
                return j
            time.sleep(0.01)
        self.fail(f'job stuck: {self.app.jobs[jid]}')

    def test_publish_store_connect_publish_again_and_rollback(self):
        p = make_plugin(self.tmp / 'p', 'demo', secrets=['API_KEY'])
        self.assertEqual(self.publish(p, 'demo', note='first'), {'ok': True, 'tool': 'demo', 'version': 1})
        self.assertTrue(self.publish(p, 'demo').get('same'))  # the same bytes add nothing
        self.assertTrue((self.state / 'library' / 'demo' / '1.tar.gz').is_file())
        lib = self.app.get('/api/library')
        t = lib['tools'][0]
        self.assertEqual((t['name'], t['latest'], t['kinds'], t['skill_secrets']), ('demo', 1, ['skill', 'mcp'], True))
        self.assertEqual(t['versions'][0]['by'], 'desk')
        self.assertEqual(t['versions'][0]['note'], 'first')
        self.assertEqual(lib['machines']['desk']['demo'], {'connected': False, 'installed': None, 'missing_secrets': [], 'error': None})

        r = self.app.post('/api/library/connect', {'tool': 'demo', 'machine': 'lap', 'on': True})
        self.assertEqual((r['connected'], r['installed']), (True, 1))
        self.assertEqual(self.install_of('lap', 'demo'), 1)
        self.assertGreaterEqual(len(self.f.verbs('lap', 'library_put')), 1)
        lib = self.app.get('/api/library')
        self.assertEqual(lib['machines']['lap']['demo'], {'connected': True, 'installed': 1, 'missing_secrets': ['API_KEY'], 'error': None})

        (p / 'files' / 'server.py').write_text('print(2)\n')
        self.assertEqual(self.publish(p, 'demo')['version'], 2)
        self.assertEqual(self.install_of('lap', 'demo'), 2)  # machines that have it get the new one
        self.assertIsNone(self.install_of('desk', 'demo'))

        r = self.app.post('/api/library/rollback', {'tool': 'demo', 'version': 1})
        self.assertEqual(r['latest'], 1)
        self.assertEqual(self.install_of('lap', 'demo'), 1)
        self.assertEqual(self.app.get('/api/library')['tools'][0]['latest'], 1)
        with self.assertRaises(server.HttpError):
            self.app.post('/api/library/rollback', {'tool': 'demo', 'version': 9})

        r = self.app.post('/api/library/connect', {'tool': 'demo', 'machine': 'lap', 'on': False})
        self.assertFalse(r['connected'])
        self.assertIsNone(self.install_of('lap', 'demo'))

    def test_what_cannot_go_says_why(self):
        p = make_plugin(self.tmp / 'p', 'demo')
        self.publish(p, 'demo')
        with self.assertRaises(server.HttpError) as e:
            self.app.post('/api/library/connect', {'tool': 'demo', 'machine': 'old', 'on': True})
        self.assertEqual(e.exception.code, 409)
        self.assertIn('Update Switchboard on Old', str(e.exception))
        self.f.off.add('lap')
        self.app.fleet.poll()
        with self.assertRaises(server.HttpError) as e:
            self.app.post('/api/library/connect', {'tool': 'demo', 'machine': 'lap', 'on': True})
        self.assertIn('Laptop is offline', str(e.exception))
        for body in ({'tool': 'nope', 'machine': 'desk', 'on': True}, {'tool': '../x', 'machine': 'desk', 'on': True},
                     {'tool': 'demo', 'machine': 'desk', 'on': 'yes'}):
            with self.assertRaises(server.HttpError):
                self.app.post('/api/library/connect', body)
        bad = tgz([plugin_json('demo'), SKILL, {'name': '../evil', 'data': b'x'}])
        with self.assertRaises(server.HttpError) as e:
            self.app.post('/api/library/publish', {'upload': 'a' * 32, 'offset': 0, 'data': base64.b64encode(bad).decode(),
                                                   'done': {'tool': 'demo', 'sha256': hashlib.sha256(bad).hexdigest()}})
        self.assertIn('outside', str(e.exception))
        with self.assertRaises(server.HttpError) as e:
            self.app.post('/api/library/publish', {'upload': 'b' * 32, 'offset': 0, 'data': base64.b64encode(b'xx').decode(),
                                                   'done': {'tool': 'demo', 'sha256': '0' * 64}})
        self.assertIn('damaged', str(e.exception))
        with mock.patch.object(library, 'BUNDLE_MAX', 10):
            with self.assertRaises(server.HttpError) as e:
                self.app.post('/api/library/publish', {'upload': 'c' * 32, 'offset': 0, 'data': base64.b64encode(b'x' * 11).decode()})
            self.assertEqual(e.exception.code, 413)

    def test_import_a_skill_from_a_machine(self):
        cdir = self.tmp / 'deskclaude'
        (cdir / 'skills' / 'web').mkdir(parents=True)
        (cdir / 'skills' / 'web' / 'SKILL.md').write_text('---\ndescription: Search\n---\nx')
        with mock.patch.dict(os.environ, {'SB_CLAUDE_DIR': str(cdir), 'SB_ROOT': str(self.tmp)}):
            src = self.app.get('/api/library/sources', {'machine': ['desk']})
            self.assertEqual([s['name'] for s in src['skills']], ['web'])
            r = self.app.post('/api/library/import', {'machine': 'desk', 'kind': 'skill', 'source': 'web'})
        self.assertEqual((r['tool'], r['version']), ('web', 1))
        t = self.app.get('/api/library')['tools'][0]
        self.assertEqual((t['description'], t['versions'][0]['note']), ('Search', 'Imported from Desk'))
        self.assertGreater(len(self.f.verbs('desk', 'library_chunk')), 0)
        with self.assertRaises(server.HttpError) as e:
            self.app.get('/api/library/sources', {'machine': ['old']})
        self.assertEqual(e.exception.code, 409)

    def test_starts_merge_defaults_push_first_and_record_the_chat(self):
        self.publish(make_plugin(self.tmp / 'a', 'alpha'), 'alpha')
        self.publish(make_plugin(self.tmp / 'b', 'beta', mcp=False), 'beta')
        self.assertEqual(self.app.post('/api/library/defaults', {'tool': 'beta', 'on': True})['defaults'], ['beta'])
        r = self.app.post('/api/new', {'machine': 'desk', 'folder': 'x', 'prompt': 'go', 'tools': ['alpha']}, 'SwitchboardDesktop/1')
        j = self.wait(r['job'])
        self.assertEqual(j['state'], 'done', j)
        start = self.f.verbs('desk', 'start')[-1]
        self.assertEqual(start['tools'], ['alpha', 'beta'])
        self.assertEqual(self.install_of('desk', 'alpha'), 1)  # pushed before the start
        self.assertEqual(self.install_of('desk', 'beta'), 1)
        self.assertEqual(self.chat(f'desk/{SID_C}'), ['alpha', 'beta'])
        self.assertIn('alpha', self.lib.d['machines']['desk'])
        rows = {s['sid']: s for m in self.app.sessions()['machines'] if m['name'] == 'desk' for s in m['sessions']}
        self.assertNotIn('tools', rows[SID_A])
        # an older machine: defaults are left out, a tool asked for by name says what to do
        r = self.app.post('/api/new', {'machine': 'old', 'folder': 'x'}, 'SwitchboardDesktop/1')
        self.assertEqual(self.wait(r['job'])['state'], 'done')
        self.assertNotIn('tools', self.f.verbs('old', 'start')[-1])
        with self.assertRaises(server.HttpError) as e:
            self.app.post('/api/new', {'machine': 'old', 'folder': 'x', 'tools': ['alpha']})
        self.assertIn('Update Switchboard on Old', str(e.exception))
        with self.assertRaises(server.HttpError):
            self.app.post('/api/new', {'machine': 'desk', 'folder': 'x', 'tools': ['nope']})

    def test_attach_then_relaunch_carries_the_list_and_rows_show_it(self):
        self.publish(make_plugin(self.tmp / 'a', 'alpha'), 'alpha')
        r = self.app.post('/api/library/attach', {'machine': 'desk', 'sid': SID_A, 'tools': ['alpha']})
        self.assertEqual(r['tools'], ['alpha'])
        rows = {s['sid']: s for m in self.app.sessions()['machines'] if m['name'] == 'desk' for s in m['sessions']}
        self.assertEqual(rows[SID_A]['tools'], ['alpha'])
        r = self.app.post('/api/relaunch', {'machine': 'desk', 'sid': SID_A, 'flags': [], 'when': 'now'})
        self.assertEqual(self.wait(r['job'])['state'], 'done')
        resume = self.f.verbs('desk', 'resume')[-1]
        self.assertEqual((resume['relaunch'], resume['tools']), (True, ['alpha']))
        self.assertEqual(self.chat(f'desk/{SID_C}'), ['alpha'])
        r = self.app.post('/api/action', {'machine': 'desk', 'verb': 'fresh', 'sid': SID_C, 'when': 'now'})
        self.assertEqual(self.wait(r['job'])['state'], 'done')
        self.assertEqual(self.f.verbs('desk', 'fresh')[-1]['tools'], ['alpha'])
        self.app.post('/api/library/attach', {'machine': 'desk', 'sid': SID_A, 'tools': []})
        r = self.app.post('/api/action', {'machine': 'desk', 'verb': 'resume', 'sid': SID_A})
        self.wait(r['job'])
        self.assertEqual(self.f.verbs('desk', 'resume')[-1]['tools'], [])  # detached: none, said explicitly
        with self.assertRaises(server.HttpError):
            self.app.post('/api/library/attach', {'machine': 'old', 'sid': SID_A, 'tools': ['alpha']})
        with self.assertRaises(server.HttpError):
            self.app.post('/api/library/attach', {'machine': 'desk', 'sid': 'nope', 'tools': ['alpha']})

    def test_continue_carries_the_list_to_the_other_machine(self):
        self.publish(make_plugin(self.tmp / 'a', 'alpha'), 'alpha')
        self.app.post('/api/library/attach', {'machine': 'desk', 'sid': SID_A, 'tools': ['alpha']})
        r = self.app.post('/api/continue', {'machine': 'desk', 'sid': SID_A, 'to': 'lap', 'when': 'now'})
        self.assertEqual(self.wait(r['job'])['state'], 'done')
        self.assertEqual(self.f.verbs('lap', 'start')[-1]['tools'], ['alpha'])
        self.assertEqual(self.install_of('lap', 'alpha'), 1)
        self.assertEqual(self.chat(f'lap/{SID_C}'), ['alpha'])

    def test_new_tool_starts_a_chat_that_publishes(self):
        r = self.app.post('/api/library/new', {'machine': 'desk', 'tool': 'weather', 'brief': 'Gets the weather'})
        self.assertEqual(self.wait(r['job'])['state'], 'done')
        self.assertEqual(self.f.verbs('desk', 'mkdir')[-1]['folder'], 'library-src/weather')
        start = self.f.verbs('desk', 'start')[-1]
        self.assertIn('publish_tool', start['prompt'])
        self.assertIn('Gets the weather', start['prompt'])
        self.assertEqual(start['folder'], 'library-src/weather')
        for body in ({'machine': 'desk', 'tool': 'Bad', 'brief': 'x'}, {'machine': 'desk', 'tool': 'ok', 'brief': ' '},
                     {'machine': 'old', 'tool': 'ok', 'brief': 'x'}):
            with self.assertRaises(server.HttpError):
                self.app.post('/api/library/new', body)

    def test_delete_drops_it_everywhere(self):
        self.publish(make_plugin(self.tmp / 'a', 'alpha'), 'alpha')
        self.app.post('/api/library/connect', {'tool': 'alpha', 'machine': 'desk', 'on': True})
        self.app.post('/api/library/defaults', {'tool': 'alpha', 'on': True})
        self.app.post('/api/library/attach', {'machine': 'desk', 'sid': SID_A, 'tools': ['alpha']})
        self.app.post('/api/library/delete', {'tool': 'alpha'})
        self.assertIsNone(self.install_of('desk', 'alpha'))
        lib = self.app.get('/api/library')
        self.assertEqual((lib['tools'], lib['defaults']), ([], []))
        self.assertEqual(self.lib.d['chats'][f'desk/{SID_A}'], [])
        self.assertFalse((self.state / 'library' / 'alpha').exists())

    def test_a_machine_that_was_away_catches_up(self):
        p = make_plugin(self.tmp / 'a', 'alpha')
        self.publish(p, 'alpha')
        self.app.post('/api/library/connect', {'tool': 'alpha', 'machine': 'lap', 'on': True})
        self.f.off.add('lap')
        self.app.fleet.poll()
        (p / 'files' / 'server.py').write_text('print(9)\n')
        self.publish(p, 'alpha')
        self.f.off.discard('lap')
        self.app.fleet.poll()
        self.assertEqual(self.install_of('lap', 'alpha'), 1)
        self.lib.states['lap']['at'] = 0
        self.lib.after_poll()
        self.assertEqual(self.install_of('lap', 'alpha'), 2)
        self.lib.d['machines']['lap'] = []  # disconnected while away
        self.lib.states['lap']['at'] = 0
        self.lib.after_poll()
        self.assertIsNone(self.install_of('lap', 'alpha'))

    def test_one_catch_up_drops_every_tool_it_should(self):
        for t in ('alpha', 'beta', 'gamma'):
            self.publish(make_plugin(self.tmp / t, t), t)
            self.app.post('/api/library/connect', {'tool': t, 'machine': 'lap', 'on': True})
        self.assertEqual([self.install_of('lap', t) for t in ('alpha', 'beta', 'gamma')], [1, 1, 1])
        self.lib.d['machines']['lap'] = []  # all three disconnected while away
        self.lib.states['lap']['at'] = 0
        self.lib.after_poll()
        self.assertEqual([self.install_of('lap', t) for t in ('alpha', 'beta', 'gamma')], [None, None, None])

    def test_the_store_survives_a_restart(self):
        self.publish(make_plugin(self.tmp / 'a', 'alpha'), 'alpha')
        self.app.post('/api/library/defaults', {'tool': 'alpha', 'on': True})
        again = library.Library(self.app, server.HttpError)
        self.assertEqual(again.d['defaults'], ['alpha'])
        self.assertEqual(again.d['tools']['alpha']['latest'], 1)


if __name__ == '__main__':
    unittest.main()
