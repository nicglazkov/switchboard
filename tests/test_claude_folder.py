"""The Claude folder per machine (agent/sb.py): sessions stay inside it on macOS, Linux and Windows, Settings picks it
from the folders under home, and the MCP servers Claude Code has configured are listed with nothing secret in them.
Windows paths are checked with PureWindowsPath, so these run on any machine."""
import json, os, unittest
from pathlib import Path, PurePosixPath, PureWindowsPath
from unittest import mock
from tests.test_rpc import RpcBase, NOW


class InsideRootTest(RpcBase):
    def test_posix(self):
        f = PurePosixPath
        inside = self.sb.inside_root
        self.assertTrue(inside('/Users/you/Claude', '/Users/you/Claude', f))
        self.assertTrue(inside('/Users/you/Claude', '/Users/you/Claude/site/app', f))
        self.assertFalse(inside('/Users/you/Claude', '/Users/you/Claudette', f))
        self.assertFalse(inside('/Users/you/Claude', '/Users/you', f))
        self.assertFalse(inside('/Users/you/Claude', '/etc', f))
        self.assertFalse(inside('/Users/you/Claude', '/users/you/claude/x', f))   # case counts on Linux

    def test_windows_drive_letters_case_and_spaces(self):
        w = PureWindowsPath
        inside = self.sb.inside_root
        self.assertTrue(inside('C:\\Users\\You\\Claude', 'c:\\users\\you\\claude\\My Project', w))
        self.assertTrue(inside('C:\\Users\\You\\Claude', 'C:/Users/You/Claude/site', w))
        self.assertFalse(inside('C:\\Users\\You\\Claude', 'D:\\Users\\You\\Claude\\site', w))
        self.assertFalse(inside('C:\\Users\\You\\Claude', 'C:\\Users\\You', w))
        self.assertTrue(inside('D:\\', 'D:\\Projects\\x', w))
        self.assertFalse(inside('\\\\server\\share\\Claude', 'C:\\Claude', w))


class CheckRootTest(RpcBase):
    def test_home_relative_on_every_system(self):
        for f in (PurePosixPath, PureWindowsPath):
            self.assertEqual(self.sb.check_root('~', f), '~')
            self.assertEqual(self.sb.check_root(' ~/Desktop/Claude/ ', f), '~/Desktop/Claude')
            self.assertEqual(self.sb.check_root('~\\Desktop\\Claude', f), '~/Desktop/Claude')

    def test_absolute_paths(self):
        self.assertEqual(self.sb.check_root('/Users/you/Claude', PurePosixPath), '/Users/you/Claude')
        self.assertEqual(self.sb.check_root('C:\\Users\\You\\My Claude', PureWindowsPath), 'C:\\Users\\You\\My Claude')
        self.assertEqual(self.sb.check_root('d:/Projects', PureWindowsPath), 'd:\\Projects')

    def test_the_poll_only_sets_a_full_path(self):
        self.sb.remember_identity({'as': 'studio', 'root': '~/Desktop/Claude/'})
        self.assertEqual(self.sb.agent_conf()['root'], '~/Desktop/Claude')
        self.sb.remember_identity({'as': 'studio', 'root': '~/../../etc'})
        self.assertEqual(self.sb.agent_conf()['root'], '~/Desktop/Claude')

    def test_refuses(self):
        for bad, f in [('', PurePosixPath), ('Claude', PurePosixPath), ('/Users/you/../x', PurePosixPath), ('~/../etc', PurePosixPath),
                       ('C:Claude', PureWindowsPath), ('Claude', PureWindowsPath), ('C:\\x\\..\\y', PureWindowsPath),
                       ('/x\x00y', PurePosixPath), ('C:\\Users\\You', PurePosixPath), ('x' * 401, PurePosixPath)]:
            with self.subTest(bad=bad, f=f.__name__):
                with self.assertRaises(ValueError):
                    self.sb.check_root(bad, f)


class ClientFoldersTest(RpcBase):
    """What a client may name: a folder under the Claude folder, written either way round."""
    def test_windows_spellings_are_refused_or_mapped(self):
        for bad in ['C:\\Windows', 'c:/x', 'D:relative', '\\\\server\\share', '\\x', '..\\x', 'proj\\..\\..\\x', '~\\x']:
            with self.subTest(bad=bad):
                with self.assertRaises(ValueError):
                    self.sb.safe_path(self.root, bad)
        (self.root / 'My Project' / 'site').mkdir(parents=True)
        self.assertEqual(self.sb.safe_path(self.root, 'My Project\\site'), (self.root / 'My Project' / 'site').resolve())

    def test_a_symlink_out_is_refused(self):
        os.symlink('/tmp', self.root / 'out')
        with self.assertRaises(ValueError):
            self.sb.safe_path(self.root, 'out')


class LaunchConfinedTest(RpcBase):
    def test_a_chat_from_outside_carries_on_inside(self):
        outside = self.tmp / 'elsewhere'
        outside.mkdir()
        self.assertEqual(self.sb.launch_dir(str(outside)), str(self.root.resolve()))
        self.assertEqual(self.sb.launch_dir(str(self.root / 'proj')), str(self.root / 'proj'))

    def test_a_chat_from_outside_is_not_resumed_there(self):
        outside = self.tmp / 'elsewhere'
        outside.mkdir()
        sid = '44444444-4444-4444-8444-444444444444'
        self.transcript(sid, str(outside), NOW - 100)
        r = self.sb.rpc({'verb': 'resume', 'sid': sid}, NOW, agents=[])
        self.assertFalse(r['ok'])
        self.assertIn('Start fresh carries it on inside', r['error'])
        os.symlink(outside, self.root / 'link-out')  # a symlink in the Claude folder that points out is outside too
        self.assertTrue(self.sb.outside_root(str(self.root / 'link-out')))
        self.assertFalse(self.sb.outside_root(str(self.root / 'proj')))

    def test_without_a_folder_set_nothing_moves(self):
        os.environ.pop('SB_ROOT')
        try:
            other = self.tmp / 'proj2'
            other.mkdir()
            with mock.patch.object(self.sb, 'agent_conf', lambda: {}):
                self.assertEqual(self.sb.launch_dir(str(other)), str(other))
        finally:
            os.environ['SB_ROOT'] = str(self.root)


class RootDirsTest(RpcBase):
    def setUp(self):
        super().setUp()
        self.home = self.tmp / 'home'
        for d in ('Desktop/Claude/site', 'Documents', '.secret', 'Library'):
            (self.home / d).mkdir(parents=True)
        os.symlink('/etc', self.home / 'etc-link')
        os.environ['SB_USER_HOME'] = str(self.home)

    def tearDown(self):
        os.environ.pop('SB_USER_HOME', None)
        super().tearDown()

    def test_one_level_under_home_without_hidden_system_or_linked_folders(self):
        r = self.sb.rpc({'verb': 'root_dirs', 'path': ''}, NOW, agents=[])
        self.assertTrue(r['ok'], r)
        self.assertEqual([f['name'] for f in r['folders']], ['Desktop', 'Documents'])
        self.assertTrue(r['folders'][0]['kids'])
        r = self.sb.rpc({'verb': 'root_dirs', 'path': 'Desktop'}, NOW, agents=[])
        self.assertEqual(r['folders'], [{'name': 'Claude', 'path': 'Desktop/Claude', 'kids': True}])
        self.assertEqual(r['shown'], '~/Desktop')

    def test_never_outside_home(self):
        for bad in ('..', '/etc', 'etc-link', 'C:\\Windows', '~/x'):
            with self.subTest(bad=bad):
                self.assertFalse(self.sb.rpc({'verb': 'root_dirs', 'path': bad}, NOW, agents=[])['ok'])

    def test_suggestion(self):
        with mock.patch.object(self.sb.sys, 'platform', 'darwin'):
            self.assertEqual(self.sb.root_suggest(), '~/Desktop/Claude')
        with mock.patch.object(self.sb.sys, 'platform', 'linux'):
            self.assertEqual(self.sb.root_suggest(), '~')
            (self.home / 'Claude').mkdir()
            self.assertEqual(self.sb.root_suggest(), '~/Claude')


class VersionsTest(RpcBase):
    def test_list_reports_both(self):
        with mock.patch.object(self.sb, 'claude_version', lambda: '2.1.296'):
            r = self.sb.rpc({'verb': 'list'}, NOW, agents=[])
        self.assertEqual(r['versions']['claude'], '2.1.296')
        self.assertRegex(r['versions']['agent'], r'^[0-9a-f]{7}$')

    def test_claude_version_is_asked_once_per_build(self):
        exe = self.tmp / 'claude'
        exe.write_text('#!/bin/sh\n')
        calls = []
        with mock.patch.object(self.sb, '_claude_exe', lambda: str(exe)):
            run = lambda: calls.append(1) or '2.1.296 (Claude Code)\n'
            self.assertEqual(self.sb.claude_version(run), '2.1.296')
            self.assertEqual(self.sb.claude_version(run), '2.1.296')
            self.assertEqual(len(calls), 1)


class McpTest(RpcBase):
    SECRETS = ('sk-ant-api03-AAAAbbbbCCCCdddd1234', 'ghp_0123456789abcdefABCDEF0123456789', 'hunter2-password-value',  # leakscan: allow (fake keys)
               'xoxb-1234-5678-abcdefghij', 'Bearer eyJhbGciOiJIUzI1NiJ9.e30.abc', 'top-secret-header', 'q=private-query',  # leakscan: allow (fake keys)
               'user:pass@', 'AKIAIOSFODNN7EXAMPLE', 'k9Xz2LmQ8vRt4WpY7sNc3BhJ')  # leakscan: allow (fake keys)

    def setUp(self):
        super().setUp()
        self.cjson.write_text(json.dumps({
            'mcpServers': {
                'github': {'type': 'stdio', 'command': 'npx', 'args': ['-y', '@modelcontextprotocol/server-github', '--token', 'ghp_0123456789abcdefABCDEF0123456789'],  # leakscan: allow (fake keys)
                           'env': {'GITHUB_PERSONAL_ACCESS_TOKEN': 'ghp_0123456789abcdefABCDEF0123456789', 'LOG_LEVEL': 'debug'}},  # leakscan: allow (fake keys)
                'linear': {'type': 'http', 'url': 'https://user:pass@mcp.example.com/sse?q=private-query#frag',
                           'headers': {'Authorization': 'Bearer eyJhbGciOiJIUzI1NiJ9.e30.abc', 'X-Api-Key': 'top-secret-header'}},
                'db': {'command': '/usr/local/bin/db-mcp', 'args': ['--password=hunter2-password-value', 'API_KEY=sk-ant-api03-AAAAbbbbCCCCdddd1234',  # leakscan: allow (fake keys)
                                                                  'k9Xz2LmQ8vRt4WpY7sNc3BhJ', 'postgres://admin:pw@db.local:5432/app?sslmode=require',
                                                                  '--port', '5432', '/Users/you/data']},
                'slack': {'command': 'slack-mcp', 'args': ['xoxb-1234-5678-abcdefghij'], 'env': {'AWS_KEY': 'AKIAIOSFODNN7EXAMPLE'}},  # leakscan: allow (fake keys)
            },
            'projects': {str(self.root / 'proj'): {'mcpServers': {'local-one': {'command': 'node', 'args': ['server.js']}}},
                         '/somewhere/else': {'mcpServers': {'not-mine': {'command': 'x'}}}},
        }))
        (self.root / '.mcp.json').write_text(json.dumps({'mcpServers': {'team': {'type': 'sse', 'url': 'https://mcp.example.com/sse/k9Xz2LmQ8vRt4WpY7sNc3BhJ'}}}))
        plug = self.tmp / 'plugin-a'
        (plug / '.claude-plugin').mkdir(parents=True)
        (plug / '.claude-plugin' / 'plugin.json').write_text(json.dumps({'mcpServers': {'browser': {'command': 'browser-mcp'}}}))
        off = self.tmp / 'plugin-b'
        off.mkdir()
        (off / '.mcp.json').write_text(json.dumps({'mcpServers': {'disabled-one': {'command': 'nope'}}}))
        (self.cdir / 'plugins').mkdir(parents=True)
        (self.cdir / 'plugins' / 'installed_plugins.json').write_text(json.dumps({'version': 2, 'plugins': {
            'tools@market': [{'installPath': str(plug)}], 'off@market': [{'installPath': str(off)}]}}))
        (self.cdir / 'settings.json').write_text(json.dumps({'enabledPlugins': {'tools@market': True, 'off@market': False}}))

    def servers(self):
        r = self.sb.rpc({'verb': 'mcp'}, NOW, agents=[])
        self.assertTrue(r['ok'], r)
        return {s['name']: s for s in r['servers']}, json.dumps(r)

    def test_every_scope(self):
        s, _ = self.servers()
        self.assertEqual({n: x['scope'] for n, x in s.items()},
                         {'github': 'user', 'linear': 'user', 'db': 'user', 'slack': 'user', 'local-one': 'local', 'team': 'project', 'browser': 'plugin'})
        self.assertEqual(s['local-one']['folder'], 'proj')
        self.assertEqual(s['browser']['plugin'], 'tools')
        self.assertEqual((s['db']['type'], s['linear']['type'], s['team']['type']), ('stdio', 'http', 'sse'))

    def test_no_secret_ever_leaves(self):
        s, raw = self.servers()
        for secret in self.SECRETS:
            with self.subTest(secret=secret):
                self.assertNotIn(secret, raw)
        self.assertNotIn('hunter2', raw)
        self.assertNotIn('admin:pw', raw)

    def test_what_is_kept_is_still_useful(self):
        s, _ = self.servers()
        self.assertEqual(s['github']['command'], 'npx')
        self.assertEqual(s['github']['args'], ['-y', '@modelcontextprotocol/server-github', '--token', '•••'])
        self.assertEqual(s['github']['env'], ['GITHUB_PERSONAL_ACCESS_TOKEN', 'LOG_LEVEL'])   # names only
        self.assertEqual(s['linear']['url'], 'https://mcp.example.com/sse')
        self.assertEqual(s['linear']['headers'], ['Authorization', 'X-Api-Key'])
        self.assertEqual(s['db']['args'], ['--password=•••', 'API_KEY=•••', '•••', 'postgres://db.local:5432/app', '--port', '5432', '/Users/you/data'])
        self.assertEqual(s['slack']['args'], ['•••'])
        self.assertEqual(s['team']['url'], 'https://mcp.example.com/sse/•••')
        self.assertIsNone(s['db']['status'])

    def test_nothing_configured(self):
        self.cjson.write_text('{}')
        (self.root / '.mcp.json').unlink()
        (self.cdir / 'plugins' / 'installed_plugins.json').unlink()
        self.assertEqual(self.sb.rpc({'verb': 'mcp'}, NOW, agents=[])['servers'], [])


if __name__ == '__main__':
    unittest.main()
