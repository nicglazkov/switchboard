import json, re, tempfile, unittest
from pathlib import Path
from tests.helpers import fresh_sb

class InstallTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(); self.sb = fresh_sb(self.tmp)
        self.home = Path(self.tmp) / 'home'; (self.home / '.claude').mkdir(parents=True)
        self.settings = self.home / '.claude' / 'settings.json'
        self.settings.write_text(json.dumps({
            'model': 'opus', 'env': {'KEEP': '1'},
            'statusLine': {'type': 'command', 'command': 'printf old'},
            'hooks': {'SessionStart': [{'hooks': [{'type': 'command', 'command': 'other-tool start'}]}]}}))
        (self.home / '.claude' / 'CLAUDE.md').write_text('# Global\n')

    def test_merge_adds_everything(self):
        s = self.sb.merge_settings(json.loads(self.settings.read_text()), '/h/.claude/switchboard/sb.py', 'python3')
        self.assertEqual(s['env']['CLAUDE_CODE_AUTO_COMPACT_WINDOW'], '300000')
        self.assertEqual(s['env']['KEEP'], '1')
        self.assertEqual(s['statusLine']['command'], 'python3 /h/.claude/switchboard/sb.py statusline')
        for ev in ('UserPromptSubmit', 'SessionStart', 'SessionEnd'):
            cmds = [h['command'] for g in s['hooks'][ev] for h in g['hooks']]
            self.assertIn('python3 /h/.claude/switchboard/sb.py hook', cmds)
        self.assertIn('other-tool start', [h['command'] for g in s['hooks']['SessionStart'] for h in g['hooks']])
        ss = [h for g in s['hooks']['SessionStart'] for h in g['hooks'] if 'sb.py' in h['command']][0]
        self.assertGreaterEqual(ss['timeout'], 120)

    def test_install_idempotent_and_preserves_other_hooks(self):
        self.sb.install(home=self.home); self.sb.install(home=self.home)
        s = json.loads(self.settings.read_text())
        for ev in ('UserPromptSubmit', 'SessionStart', 'SessionEnd'):
            n = sum('sb.py hook' in h['command'] for g in s['hooks'][ev] for h in g['hooks'])
            self.assertEqual(n, 1, ev)
        self.assertEqual((self.home / '.claude' / 'CLAUDE.md').read_text().count('- Subagent models:'), 1)
        self.assertTrue((self.home / '.claude' / 'agents' / 'Explore.md').read_text().startswith('---\nname: Explore\n'))
        self.assertTrue(any(p.name.startswith('settings.json.bak-sb-') for p in (self.home / '.claude').iterdir()))
        self.assertTrue((self.home / '.claude' / 'switchboard' / 'statusline.prev.json').exists())

    def test_hook_command_uses_forward_slashes(self):
        s = self.sb.merge_settings({}, 'C:\\Users\\you\\.claude\\switchboard\\sb.py', 'python')
        self.assertEqual(s['statusLine']['command'], 'python C:/Users/you/.claude/switchboard/sb.py statusline')

    def test_dry_run_changes_nothing(self):
        before = self.settings.read_text()
        self.sb.install(dry_run=True, home=self.home)
        self.assertEqual(self.settings.read_text(), before)
        self.assertFalse((self.home / '.claude' / 'agents').exists())

    def test_uninstall_restores_statusline_and_removes_hooks(self):
        self.sb.install(home=self.home); self.sb.uninstall(home=self.home)
        s = json.loads(self.settings.read_text())
        self.assertEqual(s['statusLine']['command'], 'printf old')
        self.assertNotIn('CLAUDE_CODE_AUTO_COMPACT_WINDOW', s['env'])
        self.assertFalse(any('sb.py' in h['command'] for g in s['hooks'].get('SessionStart', []) for h in g['hooks']))
        self.assertNotIn('- Subagent models:', (self.home / '.claude' / 'CLAUDE.md').read_text())
        self.assertFalse((self.home / '.claude' / 'agents' / 'Explore.md').exists())


class BrandingTest(unittest.TestCase):
    def test_the_launcher_label_matches_branding(self):
        # sb.py is copied to each machine on its own, so it cannot read branding.toml: the two must be kept equal.
        text = (Path(__file__).resolve().parents[1] / 'branding.toml').read_text()
        m = re.search(r'^launcher_id\s*=\s*"([^"]+)"', text, re.M)
        self.assertIsNotNone(m, 'branding.toml has no launcher_id')
        sb = fresh_sb(tempfile.mkdtemp())
        self.assertEqual(sb.LAUNCHER_LABEL, m.group(1))
