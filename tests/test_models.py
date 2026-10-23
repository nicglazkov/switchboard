"""The model picker's list (GET /api/models): the aliases the machines' Claude Code takes, with the newest version
the chats ran on, and the checks a start makes on a model it is given."""
import json, sys, unittest
from pathlib import Path
from tests.test_rpc import RpcBase, NOW
sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'server'))
import flags as flags_mod, models as models_mod, server  # noqa: E402
from tests.test_v2_live import ServerBase  # noqa: E402

HELP = """Options:
  --chrome                              Enable Claude in Chrome
  --model <model>                       Model for the current session. Provide
                                        an alias for the latest model (e.g.
                                        'fable', 'opus', or 'sonnet') or a
                                        model's full name.
  -n, --name <name>                     Set a display name ('x' is not a model)
"""


class ModelListTest(unittest.TestCase):
    def test_nothing_reported_is_the_built_in_list(self):
        r = models_mod.view([])
        self.assertEqual(r['source'], 'fallback')
        self.assertEqual([m['id'] for m in r['models']], ['fable', 'opus', 'sonnet', 'haiku'])
        self.assertEqual([m['label'] for m in r['models']], ['Fable', 'Opus', 'Sonnet', 'Haiku'])

    def test_aliases_with_the_newest_version_seen(self):
        machines = [{'models': {'aliases': ['fable', 'opus', 'sonnet'], 'allowed': None},
                     'sessions': [{'model': 'claude-opus-4-7'}, {'model': 'claude-opus-5-5'}, {'model': 'claude-haiku-4-5-20251001'},
                                  {'model': '<synthetic>'}, {'model': None}]},
                    {'models': {'aliases': ['opus', 'mythos'], 'allowed': None},
                     'sessions': [{'model': 'claude-fable-5-1[1m]'}, {'model': 'claude-zephyr-1'}]}]
        r = models_mod.view(machines)
        self.assertEqual(r['source'], 'machines')
        got = [(m['id'], m['label'], m['detail']) for m in r['models']]
        self.assertEqual(got, [('fable', 'Fable', 'Fable 5.1'), ('opus', 'Opus', 'Opus 5.5'), ('sonnet', 'Sonnet', ''),
                               ('haiku', 'Haiku', 'Haiku 4.5'), ('mythos', 'Mythos', ''), ('claude-zephyr-1', 'Zephyr 1', '')])

    def test_available_models_narrow_it(self):
        machines = [{'models': {'aliases': ['opus'], 'allowed': ['opus', 'claude-sonnet-5']}, 'sessions': []}]
        self.assertEqual([m['id'] for m in models_mod.view(machines)['models']], ['opus', 'sonnet'])

    def test_model_words(self):
        for m, w in [('opus', 'Opus'), ('claude-opus-5-5', 'Opus 5.5'), ('claude-sonnet-4-5-20250929', 'Sonnet 4.5'),
                     ('claude-opus-5', 'Opus 5'), ('claude-opus-4-7[1m]', 'Opus 4.7'), (None, None), ('mythos', 'Mythos')]:
            self.assertEqual(flags_mod.model_word(m), w, m)

    def test_model_checks(self):
        for ok in ['opus', 'fable', 'opus[1m]', 'claude-opus-5-5', 'claude-haiku-4-5-20251001']:
            self.assertTrue(flags_mod.model_ok(ok), ok)
        for bad in ['gpt', 'opus;rm', 'opus --chrome', '-p', 'claude-', '', None, 'Opus']:
            self.assertFalse(flags_mod.model_ok(bad), bad)
        self.assertTrue(flags_mod.model_ok('mythos', ['mythos']))


class ModelServerTest(ServerBase):
    def test_endpoint_and_a_full_id_starts(self):
        self.f.sessions['desk'] = [{'sid': 'x', 'model': 'claude-opus-5-5', 'title': 't', 'mtime': NOW, 'active': NOW}]
        self.app.fleet.poll()
        r = self.app.get('/api/models')
        self.assertEqual(r['models'][1], {'id': 'opus', 'label': 'Opus', 'detail': 'Opus 5.5'})
        self.assertTrue(self.app.post('/api/new', {'machine': 'desk', 'folder': 'p', 'model': 'claude-opus-5-5'})['ok'])
        self.assertTrue(self.app.post('/api/new', {'machine': 'desk', 'folder': 'p', 'model': 'fable'})['ok'])
        with self.assertRaises(server.HttpError):
            self.app.post('/api/new', {'machine': 'desk', 'folder': 'p', 'model': 'mythos'})


class ModelAgentTest(RpcBase):
    def test_help_aliases(self):
        self.assertEqual(self.sb.help_model_aliases(HELP), ['fable', 'opus', 'sonnet'])
        self.assertEqual(self.sb.help_model_aliases('--chrome only'), [])

    def test_list_reports_models_and_start_takes_ids(self):
        exe = self.tmp / 'claude'
        exe.write_text('#!/bin/sh\n')
        self.sb._claude_exe = lambda: str(exe)
        self.sb.run_claude_cmd = lambda args, **kw: HELP.replace("'sonnet'", "'sonnet', 'mythos'") if args == ['--help'] else ''
        (self.cdir / 'settings.json').write_text(json.dumps({'model': 'opus', 'availableModels': ['opus', 'fable']}))
        r = self.sb.rpc({'verb': 'list'}, NOW, agents=[])
        self.assertEqual(r['models'], {'aliases': ['fable', 'opus', 'sonnet', 'mythos'], 'allowed': ['opus', 'fable'], 'default': 'opus'})
        for model in ('claude-opus-5-5', 'fable', 'mythos'):
            self.assertTrue(self.sb.rpc({'verb': 'start', 'folder': 'proj', 'model': model}, NOW, agents=[])['ok'], model)
        for model in ('gpt', 'opus --chrome'):
            self.assertFalse(self.sb.rpc({'verb': 'start', 'folder': 'proj', 'model': model}, NOW, agents=[])['ok'], model)


if __name__ == '__main__':
    unittest.main()
