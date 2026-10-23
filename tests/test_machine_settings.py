"""Settings, Machines: the Claude folder checked for each system (fleet.check_root), each machine's versions, what it
can do with the user's own switches and lines (caps.json, read by Auto), and the live views of its folders and MCP
servers (server/server.py machine_extra)."""
import json, sys, unittest
from pathlib import Path
from tempfile import mkdtemp
sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'server'))
import features, fleet, server  # noqa: E402
from tests.test_auto import caps, DESK, PC  # noqa: E402

NOW = 1_780_000_000.0
LAB = dict(caps('linux', docker='29', node='22'), ram_free_gb=8.0)


class CheckRootTest(unittest.TestCase):
    def test_home_relative_on_every_system(self):
        for win in (False, True):
            self.assertEqual(fleet.check_root('~', windows=win), '~')
            self.assertEqual(fleet.check_root(' ~/Desktop/Claude/ ', windows=win), '~/Desktop/Claude')
            self.assertEqual(fleet.check_root('~\\Projects\\My Work', windows=win), '~/Projects/My Work')

    def test_mac_and_linux_take_slash_paths(self):
        self.assertEqual(fleet.check_root('/Users/you/Claude'), '/Users/you/Claude')
        self.assertEqual(fleet.check_root('/home/you//code/./x'), '/home/you/code/x')
        for bad in ('C:\\Users\\you', 'code', 'Desktop/Claude', '~me/x'):
            with self.assertRaises(fleet.FleetError, msg=bad):
                fleet.check_root(bad)

    def test_windows_takes_drive_letters_shares_and_spaces(self):
        self.assertEqual(fleet.check_root('C:\\Users\\you\\My Projects', windows=True), 'C:\\Users\\you\\My Projects')
        self.assertEqual(fleet.check_root('d:/Work/Claude', windows=True), 'd:\\Work\\Claude')
        self.assertEqual(fleet.check_root('\\\\nas\\share\\claude', windows=True), '\\\\nas\\share\\claude')
        for bad in ('/Users/you', 'C:relative', 'Claude', 'C:'):
            with self.assertRaises(fleet.FleetError, msg=bad):
                fleet.check_root(bad, windows=True)

    def test_never_dot_dot_or_nul(self):
        for win in (False, True):
            for bad in ('~/../etc', '/Users/you/../../etc', 'C:\\Users\\..\\Windows', '~/a/..', '', '   ', 'x\x00', None, 7, '/' + 'a' * 401):  # leakscan: allow (a path that climbs out)
                with self.assertRaises(fleet.FleetError, msg=repr(bad)):
                    fleet.check_root(bad, windows=win)


class Agent:
    """Two machines with the newest agent, one (lab) with an old one."""
    def __init__(self):
        self.calls = []

    def __call__(self, target, req, timeout):
        self.calls.append((target, req))
        v = req['verb']
        old = target == 'me@z'
        if v == 'list':
            feats = ['flags'] if old else ['flags', 'mcp', 'root_dirs']
            return {'ok': True, 'sessions': [], 'replaced': [], 'features': feats, 'root': '/home/you', 'root_set': False if old else True,
                    'root_suggest': '~' if old else None, 'versions': None if old else {'agent': 'abc1234', 'claude': '2.1.296'},
                    'device': {'os': {'me@x': 'mac', 'me@y': 'windows', 'me@z': 'linux'}[target], 'form': 'desktop'}}
        if v == 'caps':
            return {'ok': True, 'caps': {'me@x': DESK, 'me@y': PC, 'me@z': LAB}[target]}
        if v == 'root_dirs':
            return {'ok': True, 'path': req['path'], 'folders': [{'name': 'Desktop', 'path': 'Desktop', 'kids': True}]}
        if v == 'mcp':
            return {'ok': True, 'servers': [{'name': 'github', 'scope': 'user', 'env': ['GITHUB_TOKEN']}]}
        return {'ok': True}


class SettingsTest(unittest.TestCase):
    def setUp(self):
        self.state, self.cfg = Path(mkdtemp()), Path(mkdtemp())
        self.agent = Agent()
        self.fleet = fleet.Fleet([{'name': 'studio', 'label': 'Studio', 'target': 'me@x', 'os': 'mac'},
                                  {'name': 'tower', 'label': 'Tower', 'target': 'me@y', 'os': 'windows'},
                                  {'name': 'laptop', 'label': 'Laptop', 'target': 'me@z', 'os': 'linux'}], self.agent, clock=lambda: NOW)
        self.app = server.App(self.fleet, state_dir=self.state, web_dir=self.state, cfg_dir=self.cfg, clock=lambda: NOW)
        self.app._poll_soon = lambda *a: None
        self.app._refresh_soon = lambda *a, **k: None
        self.fleet.poll()

    def machines(self):
        return {m['name']: m for m in self.app.get('/api/machines')['machines']}

    def test_the_folder_is_checked_for_the_machines_own_system(self):
        self.app.post('/api/machines', {'op': 'root', 'machine': 'tower', 'root': 'D:\\Claude Work'})
        self.app.post('/api/machines', {'op': 'root', 'machine': 'studio', 'root': '~/Desktop/Claude/'})
        m = self.machines()
        self.assertEqual((m['tower']['root'], m['studio']['root']), ('D:\\Claude Work', '~/Desktop/Claude'))
        for body in ({'machine': 'studio', 'root': 'D:\\Claude'}, {'machine': 'tower', 'root': '/Users/you'},
                     {'machine': 'studio', 'root': '~/../../etc'}):
            with self.assertRaises(server.HttpError) as e:
                self.app.post('/api/machines', dict(body, op='root'))
            self.assertEqual(e.exception.code, 400)
        self.assertEqual(self.machines()['studio']['root'], '~/Desktop/Claude')

    def test_versions_and_whether_a_folder_still_needs_picking(self):
        m = self.machines()
        self.assertEqual(m['studio']['versions'], {'agent': 'abc1234', 'claude': '2.1.296'})
        self.assertIsNone(m['laptop']['versions'])
        self.assertTrue(m['laptop']['needs_root'])
        self.assertEqual(m['laptop']['root_suggest'], '~')
        self.assertFalse(m['studio']['needs_root'])
        self.assertTrue(m['studio']['online'])

    def test_folders_and_mcp_come_live_from_the_machine(self):
        r = self.app.get('/api/machines/studio/folders', {'path': ['Desktop']})
        self.assertEqual(r['folders'][0]['name'], 'Desktop')
        self.assertEqual(self.agent.calls[-1][1], {'verb': 'root_dirs', 'path': 'Desktop'})
        self.assertEqual(self.app.get('/api/machines/studio/mcp')['servers'][0]['env'], ['GITHUB_TOKEN'])
        for what in ('folders', 'mcp'):
            with self.assertRaises(server.HttpError) as e:
                self.app.get(f'/api/machines/laptop/{what}')
            self.assertEqual(e.exception.code, 409)
            self.assertIn('Update Switchboard on Laptop', str(e.exception))
        with self.assertRaises(server.HttpError):
            self.app.get('/api/machines/ghost/mcp')

    def test_capabilities_detected_with_switches_and_lines_of_its_own(self):
        v = self.app.get('/api/machines/tower/caps')
        self.assertFalse(v['on'])
        names = {t['key']: t for t in v['tools']}
        self.assertEqual(names['cuda']['detail'], '13.2 on the RTX 5070')
        self.assertTrue(all(t['on'] for t in v['tools']))
        self.app.post('/api/machines', {'op': 'caps', 'machine': 'tower', 'on': True, 'off': ['cuda', 'bogus'],
                                        'extra': ['Can print labels on the Zebra printer', '  ', 'x' * 500]})
        saved = json.loads((self.cfg / 'caps.json').read_text())['tower']
        self.assertEqual(saved['off'], ['cuda'])
        self.assertEqual(saved['extra'], ['Can print labels on the Zebra printer', 'x' * 120])
        v = self.app.get('/api/machines/tower/caps')
        self.assertTrue(v['on'])
        self.assertFalse({t['key']: t for t in v['tools']}['cuda']['on'])
        seen = self.app.feat.auto.caps('tower')
        self.assertIsNone(seen['tools']['cuda'])
        self.assertIsNone(seen['gpu'])
        self.assertEqual(seen['also'][0], 'Can print labels on the Zebra printer')
        self.assertIn('Can print labels', self.machines()['tower']['can'])
        self.assertTrue(self.machines()['tower']['auto'])
        self.app.post('/api/machines', {'op': 'caps', 'machine': 'studio', 'on': True})
        self.assertIs(json.loads((self.cfg / 'caps.json').read_text())['studio'], True, 'plain on stays the old format')
        self.app.post('/api/machines', {'op': 'caps', 'machine': 'tower', 'on': False})
        self.assertIsNone(self.app.feat.auto.caps('tower'))
        self.assertEqual(self.app.get('/api/machines/tower/caps')['extra'][0], 'Can print labels on the Zebra printer', 'kept while off')
        with self.assertRaises(server.HttpError):
            self.app.post('/api/machines', {'op': 'caps', 'machine': 'tower', 'off': 'cuda'})


class TimeZoneTest(unittest.TestCase):
    def test_the_days_are_counted_in_a_zone_the_server_names(self):
        import os
        from unittest import mock
        with mock.patch.dict(os.environ, {'SWITCHBOARD_TZ': 'Europe/Berlin'}):
            self.assertEqual(fleet.server_tz(), 'Europe/Berlin')
        with mock.patch.dict(os.environ, {'SWITCHBOARD_TZ': '', 'TZ': ''}), mock.patch('os.path.realpath', lambda p: '/usr/share/zoneinfo/Asia/Tokyo'):
            self.assertEqual(fleet.server_tz(), 'Asia/Tokyo')
        with mock.patch.dict(os.environ, {'SWITCHBOARD_TZ': '', 'TZ': ''}), mock.patch('os.path.realpath', lambda p: '/etc/localtime'):
            self.assertEqual(fleet.server_tz(), 'UTC')
        f = fleet.Fleet([{'name': 'studio', 'label': 'Studio', 'target': 'me@x'}], lambda *a: {'ok': False})
        self.assertEqual(f.summary()['tz'], fleet.DAY_TZ)


class CapsRulesTest(unittest.TestCase):
    def test_a_switched_off_tool_does_not_win_the_pick(self):
        st = features.cap_setting({'on': True, 'off': ['xcode']})
        desk = features.apply_setting(DESK, st)
        self.assertEqual(desk['tools']['iphone'], 'none')
        name, _, why = features.rules_pick([('desk', 'Desk', desk), ('tower', 'Tower', PC)], 'Fix the iOS app')
        self.assertEqual(why, 'No online machine has Xcode. This one has the most free memory.')

    def test_the_users_own_line_decides_when_a_task_names_it(self):
        lab = features.apply_setting(LAB, features.cap_setting({'on': True, 'extra': ['Can print labels on the Zebra printer']}))
        name, _, why = features.rules_pick([('laptop', 'Laptop', lab), ('tower', 'Tower', PC)], 'print shipping labels for the orders')
        self.assertEqual((name, why), ('laptop', 'You said it can print labels on the Zebra printer.'))
        self.assertEqual(features.rules_pick([('laptop', 'Laptop', lab), ('tower', 'Tower', PC)], 'summarize the notes')[0], 'tower')
        self.assertIn('Can print labels', features.can_line(lab))

    def test_old_files_still_read(self):
        self.assertEqual(features.cap_setting(True), {'on': True, 'off': [], 'extra': []})
        self.assertEqual(features.cap_setting(False)['on'], False)
        self.assertEqual(features.cap_setting({'on': True, 'off': 'xcode', 'extra': 'x'}), {'on': True, 'off': [], 'extra': []})


if __name__ == '__main__':
    unittest.main()
