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

    def test_catalog_names_a_new_version_before_any_chat_runs_on_it(self):
        old = {'fetched': 100, 'targets': {'fable': 'claude-fable-5-1'}, 'models': [{'id': 'claude-fable-5-1', 'name': 'Fable 5.1', 'main': True}]}
        new = {'fetched': 200, 'targets': {'fable': 'claude-fable-5-2', 'opus': 'claude-opus-5-5', 'bad': 'rm -rf'},
               'models': [{'id': 'claude-fable-5-2', 'name': 'Fable 5.2', 'main': True},
                          {'id': 'claude-opus-5-5', 'name': 'Opus 5.5', 'main': True},
                          {'id': 'claude-zephyr-2', 'name': 'Zephyr 2', 'main': True},
                          {'id': 'claude-opus-4-7', 'name': 'Opus 4.7', 'main': False}]}
        machines = [{'models': {'aliases': ['fable', 'opus'], 'allowed': None, 'catalog': new},
                     'sessions': [{'model': 'claude-fable-5-1'}, {'model': 'claude-sonnet-5-5'}]},
                    {'models': {'aliases': ['fable'], 'allowed': None, 'catalog': old}, 'sessions': []}]
        got = [(m['id'], m['detail']) for m in models_mod.view(machines)['models']]
        self.assertEqual(got, [('fable', 'Fable 5.2'), ('opus', 'Opus 5.5'), ('sonnet', 'Sonnet 5.5'), ('haiku', ''),
                               ('claude-zephyr-2', '')])
        new['models'] += [{'id': 'claude-opus-4-8', 'name': 'Opus 4.8'}, {'id': 'claude-fable-5', 'name': 'Fable 5'},
                          {'id': 'claude-fable-5-1', 'name': 'Fable 5.1', 'desc': 'Older'}]
        new['models'][0]['desc'] = 'For your toughest challenges'
        r = models_mod.view(machines)
        self.assertEqual(r['models'][0]['description'], 'For your toughest challenges')
        self.assertEqual(r['models'][0]['version'], 'claude-fable-5-2')
        self.assertEqual([(m['id'], m['label']) for m in r['more']],
                         [('claude-fable-5-1', 'Fable 5.1'), ('claude-fable-5', 'Fable 5'), ('claude-opus-4-8', 'Opus 4.8'),
                          ('claude-opus-4-7', 'Opus 4.7')])
        # a chat already on a newer version than a stale catalog keeps the newer one
        machines[0]['sessions'].append({'model': 'claude-fable-5-3'})
        self.assertEqual(models_mod.view(machines)['models'][0]['detail'], 'Fable 5.3')

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
        self.assertEqual(r['models'][1], {'id': 'opus', 'label': 'Opus', 'detail': 'Opus 5.5', 'description': '', 'version': 'claude-opus-5-5'})
        self.assertEqual(r['more'], [])
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
        self.assertEqual(r['models'], {'aliases': ['fable', 'opus', 'sonnet', 'mythos'], 'allowed': ['opus', 'fable'], 'default': 'opus',
                                       'catalog': None})
        for model in ('claude-opus-5-5', 'fable', 'mythos'):
            self.assertTrue(self.sb.rpc({'verb': 'start', 'folder': 'proj', 'model': model}, NOW, agents=[])['ok'], model)
        for model in ('gpt', 'opus --chrome'):
            self.assertFalse(self.sb.rpc({'verb': 'start', 'folder': 'proj', 'model': model}, NOW, agents=[])['ok'], model)

    def test_catalog_reads_the_newest_file(self):
        d = self.cdir / 'cache' / 'model-catalog'
        d.mkdir(parents=True)
        def cat(fetched, fable, extra=()):
            return json.dumps({'fetchedAt': fetched, 'catalog': {'config': {
                'provider_alias_targets': {'fable': {'default': fable, 'per_provider': {'gateway': 'claude-fable-5'}}, 'x y': {'default': 'z'}},
                'models': [{'id': fable, 'name': fable.replace('claude-fable-', 'Fable ').replace('-', '.'), 'section': 'main',
                            'runtime': {'family': 'fable'}}, {'id': 'not a model'}, *extra]}}})
        (d / 'old-cc.json').write_text(cat(100, 'claude-fable-5-1'))
        (d / 'org-cc.json').write_text(cat(200, 'claude-fable-5-2', [{'id': 'claude-fable-5-1', 'name': 'Fable 5.1', 'section': 'overflow'}]))
        (d / 'published-floor.json').write_text('{"version": 1}')
        (d / 'broken.json').write_text('{')
        c = self.sb.model_catalog()
        self.assertEqual(c['fetched'], 200)
        self.assertEqual(c['targets'], {'fable': 'claude-fable-5-2'})
        self.assertEqual(c['models'], [{'id': 'claude-fable-5-2', 'name': 'Fable 5.2', 'desc': '', 'family': 'fable', 'main': True},
                                       {'id': 'claude-fable-5-1', 'name': 'Fable 5.1', 'desc': '', 'family': '', 'main': False}])
        self.assertIsNone(self.sb.model_catalog(self.tmp / 'nowhere'))


if __name__ == '__main__':
    unittest.main()
