import json, os, stat, tempfile, time, unittest
from pathlib import Path
from tests.helpers import fresh_sb


class SetupCheckTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp()
        self.sb = fresh_sb(self.tmp)
        self.cj = Path(self.tmp) / 'claude.json'
        os.environ['SB_CLAUDE_JSON'] = str(self.cj)
        self.bin = Path(self.tmp) / 'nvm' / 'bin'
        self.bin.mkdir(parents=True)
        for name in ('npx', 'node'):
            f = self.bin / name
            f.write_text('#!/bin/sh\n')
            f.chmod(f.stat().st_mode | stat.S_IEXEC)
        self.cj.write_text(json.dumps({'mcpServers': {
            'seq': {'type': 'stdio', 'command': 'npx', 'args': ['x']},
            'py': {'type': 'stdio', 'command': 'sh'},
            'web': {'type': 'http', 'url': 'https://example.com'},
            'gone': {'type': 'stdio', 'command': str(Path(self.tmp) / 'old' / 'npx')},
        }}))
        self.sb._service_path = lambda roster=None: '/usr/bin:/bin'
        self.sb._login_path = lambda now: f'{self.bin}:/usr/bin:/bin'

    def tearDown(self):
        os.environ.pop('SB_CLAUDE_JSON', None)

    def test_names_off_the_service_path_and_missing_full_paths(self):
        self.assertEqual(sorted(self.sb._mcp_unreachable('/usr/bin:/bin')), ['gone', 'seq'])

    def test_issue_offers_the_fix_and_fix_points_at_full_paths(self):
        issues = self.sb.setup_issues([], time.time(), force=True)
        self.assertEqual([i['fix'] for i in issues], ['mcp-path'])
        r = self.sb.setup_fix('mcp-path', time.time())
        self.assertTrue(r['ok'])
        self.assertEqual(sorted(r['fixed']), ['gone', 'seq'])
        servers = json.loads(self.cj.read_text())['mcpServers']
        self.assertEqual(servers['seq']['command'], str(self.bin / 'npx'))
        self.assertTrue(servers['seq']['env']['PATH'].startswith(str(self.bin) + ':'))
        self.assertEqual(servers['py']['command'], 'sh')
        self.assertEqual(self.sb._mcp_unreachable('/usr/bin:/bin'), {})

    def test_chrome_issue_follows_the_newest_chrome_chat(self):
        old = {'kind': 'background', 'chrome_ok': True, 'started': 1, 'title': 'a'}
        new = {'kind': 'background', 'chrome_missing': True, 'started': 2, 'title': 'b'}
        self.assertIsNotNone(self.sb._chrome_bg_issue([old, new]))
        self.assertIsNone(self.sb._chrome_bg_issue([dict(old, started=3), new]))
        self.assertIsNone(self.sb._chrome_bg_issue([]))


if __name__ == '__main__':
    unittest.main()
