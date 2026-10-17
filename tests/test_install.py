import json, re, tempfile, unittest
from pathlib import Path
from tests.helpers import fresh_sb, sb_options

SETUP_ON = dict(setup_subagent_rule=True, setup_explore_agent=True, setup_compact_window=True)

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
        self.assertNotIn('CLAUDE_CODE_AUTO_COMPACT_WINDOW', s['env'])  # opt-in: apply_setup adds it, never the merge
        self.assertEqual(s['env']['KEEP'], '1')
        self.assertEqual(s['statusLine']['command'], 'python3 /h/.claude/switchboard/sb.py statusline')
        for ev in ('UserPromptSubmit', 'SessionStart', 'SessionEnd'):
            cmds = [h['command'] for g in s['hooks'][ev] for h in g['hooks']]
            self.assertIn('python3 /h/.claude/switchboard/sb.py hook', cmds)
        self.assertIn('other-tool start', [h['command'] for g in s['hooks']['SessionStart'] for h in g['hooks']])
        ss = [h for g in s['hooks']['SessionStart'] for h in g['hooks'] if 'sb.py' in h['command']][0]
        self.assertGreaterEqual(ss['timeout'], 120)

    def test_install_idempotent_and_preserves_other_hooks(self):
        sb_options(self.sb, **SETUP_ON)
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
        sb_options(self.sb, **SETUP_ON)
        self.sb.install(home=self.home); self.sb.uninstall(home=self.home)
        s = json.loads(self.settings.read_text())
        self.assertEqual(s['statusLine']['command'], 'printf old')
        self.assertNotIn('CLAUDE_CODE_AUTO_COMPACT_WINDOW', s['env'])
        self.assertFalse(any('sb.py' in h['command'] for g in s['hooks'].get('SessionStart', []) for h in g['hooks']))
        self.assertEqual((self.home / '.claude' / 'CLAUDE.md').read_text(), '# Global\n')
        self.assertFalse((self.home / '.claude' / 'agents' / 'Explore.md').exists())


class SetupOptionsTest(unittest.TestCase):
    """The three changes to the user's own Claude setup are opt-in, and turning one off takes back only what
    Switchboard added."""
    def setUp(self):
        self.tmp = tempfile.mkdtemp(); self.sb = fresh_sb(self.tmp)
        self.home = Path(self.tmp) / 'home'; self.cdir = self.home / '.claude'; self.cdir.mkdir(parents=True)
        self.settings, self.cm, self.ex = self.cdir / 'settings.json', self.cdir / 'CLAUDE.md', self.cdir / 'agents' / 'Explore.md'
        self.settings.write_text(json.dumps({'env': {'KEEP': '1'}}))
        self.cm.write_text('# Global\n\n- Be brief.\n')

    def env(self):
        return json.loads(self.settings.read_text()).get('env') or {}

    def test_a_new_install_changes_nothing_of_the_users(self):
        self.sb.install(home=self.home)  # before the first poll
        sb_options(self.sb)  # the poll: all off
        self.sb.install(home=self.home)
        self.assertEqual(self.cm.read_text(), '# Global\n\n- Be brief.\n')
        self.assertFalse(self.ex.exists())
        self.assertEqual(self.env(), {'KEEP': '1'})

    def test_on_adds_each_and_off_takes_back_only_those(self):
        sb_options(self.sb, **SETUP_ON)
        self.sb.install(home=self.home)
        self.assertIn(self.sb.CLAUDE_MD_OPEN + '\n' + self.sb.CLAUDE_MD_RULE + '\n' + self.sb.CLAUDE_MD_CLOSE, self.cm.read_text())
        self.assertEqual(self.ex.read_text(), self.sb.EXPLORE_MD)
        self.assertEqual(self.env()['CLAUDE_CODE_AUTO_COMPACT_WINDOW'], '300000')
        self.cm.write_text(self.cm.read_text() + '\n- Added later by the user.\n')
        done = self.sb.apply_setup(self.cdir, {})
        self.assertEqual(set(done.values()), {'removed'})
        self.assertEqual(self.cm.read_text(), '# Global\n\n- Be brief.\n\n- Added later by the user.\n')
        self.assertFalse(self.ex.exists())
        self.assertEqual(self.env(), {'KEEP': '1'})
        self.assertEqual(self.sb.apply_setup(self.cdir, {}), {}, 'nothing left to take back')

    def test_what_the_user_wrote_is_never_replaced_or_removed(self):
        self.ex.parent.mkdir()
        self.ex.write_text('---\nname: Explore\nmodel: haiku\n---\nMine.\n')
        self.cm.write_text('- Subagent models: my own rule.\n')
        self.settings.write_text(json.dumps({'env': {'CLAUDE_CODE_AUTO_COMPACT_WINDOW': '500000'}}))
        self.sb.install(home=self.home)
        on = dict(SETUP_ON)
        self.assertEqual(self.sb.apply_setup(self.cdir, on), {})
        self.assertEqual(self.sb.apply_setup(self.cdir, {}), {})
        self.assertEqual(self.ex.read_text(), '---\nname: Explore\nmodel: haiku\n---\nMine.\n')
        self.assertEqual(self.cm.read_text(), '- Subagent models: my own rule.\n')
        self.assertEqual(self.env()['CLAUDE_CODE_AUTO_COMPACT_WINDOW'], '500000')
        self.sb.uninstall(home=self.home)
        self.assertTrue(self.ex.exists())
        self.assertEqual(self.cm.read_text(), '- Subagent models: my own rule.\n')

    def test_an_older_install_counts_what_it_wrote_as_its_own(self):
        # before setup.json: install added all three unasked, the rule as a bare line
        hook = {'type': 'command', 'command': 'python3 /h/.claude/switchboard/sb.py hook'}
        self.settings.write_text(json.dumps({'env': {'CLAUDE_CODE_AUTO_COMPACT_WINDOW': '300000'},
                                             'hooks': {'Stop': [{'hooks': [hook]}]}}))
        self.cm.write_text('# Global\n\n' + self.sb.CLAUDE_MD_RULE + '\n')
        self.ex.parent.mkdir(); self.ex.write_text(self.sb.EXPLORE_MD)
        self.sb.install(home=self.home)  # the upgrade, before the server has said: everything stays
        self.assertEqual(self.ex.read_text(), self.sb.EXPLORE_MD)
        self.assertIn(self.sb.CLAUDE_MD_RULE, self.cm.read_text())
        self.assertEqual(self.env()['CLAUDE_CODE_AUTO_COMPACT_WINDOW'], '300000')
        self.assertEqual(self.sb.apply_setup(self.cdir, SETUP_ON), {}, 'the migrated server keeps them on: no change')
        self.assertEqual(self.sb.apply_setup(self.cdir, {}), {k: 'removed' for k in SETUP_ON})
        self.assertEqual(self.cm.read_text(), '# Global\n')
        self.assertFalse(self.ex.exists())
        self.assertNotIn('CLAUDE_CODE_AUTO_COMPACT_WINDOW', self.env())

    def test_the_poll_applies_a_change_only_to_the_installed_setup(self):
        sb_options(self.sb, **SETUP_ON)  # SB_HOME is not inside this Claude folder: nothing is touched
        self.assertFalse(self.ex.exists())
        import os
        os.environ['SB_HOME'] = str(self.cdir / 'switchboard'); os.environ['SB_CLAUDE_DIR'] = str(self.cdir)
        try:
            sb = fresh_sb.__globals__['importlib'].reload(self.sb)
            sb_options(sb, setup_explore_agent=True)
            self.assertEqual(self.ex.read_text(), sb.EXPLORE_MD)
            sb_options(sb, setup_explore_agent=True)  # the same again: no work
            sb_options(sb)
            self.assertFalse(self.ex.exists())
        finally:
            os.environ.pop('SB_CLAUDE_DIR', None)


class BrandingTest(unittest.TestCase):
    def test_the_launcher_label_matches_branding(self):
        # sb.py is copied to each machine on its own, so it cannot read branding.toml: the two must be kept equal.
        text = (Path(__file__).resolve().parents[1] / 'branding.toml').read_text()
        m = re.search(r'^launcher_id\s*=\s*"([^"]+)"', text, re.M)
        self.assertIsNotNone(m, 'branding.toml has no launcher_id')
        sb = fresh_sb(tempfile.mkdtemp())
        self.assertEqual(sb.LAUNCHER_LABEL, m.group(1))
