"""Places (agent/sb.py): chats and the file browser reach the whole home folder and the drives added on the machine,
never dot folders, ~/Library or AppData, and Claude goes into a folder outside the Claude folder only after the user
said yes in an app."""
import json, os, unittest
from pathlib import Path
from unittest import mock
from tests.test_rpc import RpcBase, NOW


class PlacesBase(RpcBase):
    def setUp(self):
        super().setUp()
        self.home = (self.tmp / 'home').resolve()
        self.root = self.home / 'Desktop' / 'Claude'
        for d in ('Desktop/Claude/proj', 'Documents/site/src', 'Downloads', '.ssh', 'Library/Keychains',
                  'Library/Mobile Documents/com~apple~CloudDocs/Notes', 'AppData/Roaming'):
            (self.home / d).mkdir(parents=True)
        (self.home / '.ssh' / 'id_ed25519').write_text('key')
        (self.home / 'Documents' / 'site' / 'README.md').write_text('hi')
        os.environ.update(SB_USER_HOME=str(self.home), SB_ROOT=str(self.root))

    def tearDown(self):
        os.environ.pop('SB_USER_HOME', None)
        super().tearDown()

    def rpc(self, **req):
        return self.sb.rpc(req, NOW, agents=[])

    def conf(self, **kw):
        return mock.patch.object(self.sb, 'agent_conf', lambda: kw)


class ResolveTest(PlacesBase):
    def test_three_ways_to_name_a_folder(self):
        r = self.sb.resolve_folder
        self.assertEqual(r(''), self.root.resolve())
        self.assertEqual(r('proj'), (self.root / 'proj').resolve())
        self.assertEqual(r('~/Documents/site'), self.home / 'Documents' / 'site')
        self.assertEqual(r('~\\Documents\\site'), self.home / 'Documents' / 'site')
        self.assertEqual(r('~'), self.home)
        self.assertEqual(r('~/Library/Mobile Documents/com~apple~CloudDocs/Notes'),
                         self.home / 'Library' / 'Mobile Documents' / 'com~apple~CloudDocs' / 'Notes')

    def test_never_private_folders_absolute_paths_or_escapes(self):
        os.symlink('/etc', self.home / 'Documents' / 'etc-link')
        for bad in ('~/.ssh', '~/Documents/../.ssh', '~/Library', '~/Library/Keychains', '~/AppData/Roaming', '/etc',
                    'C:\\Windows', '~/Documents/etc-link', '~/../x', '~/nope', '@Nope', '~/a\x00b', 7):
            with self.subTest(bad=bad):
                with self.assertRaises(ValueError):
                    self.sb.resolve_folder(bad)

    def test_roaming_turned_off_keeps_chats_in_the_claude_folder(self):
        with self.conf(roam=False):
            with self.assertRaises(ValueError):
                self.sb.resolve_folder('~/Documents/site')
            self.assertEqual(self.sb.resolve_folder('proj'), (self.root / 'proj').resolve())
            self.assertIsNone(self.sb.folder_of(str(self.home / 'Documents' / 'site')))

    def test_a_place_added_on_the_machine(self):
        drive = (self.tmp / 'drive').resolve()
        (drive / 'work' / '.git').mkdir(parents=True)
        with self.conf(places=[{'name': 'Data', 'path': str(drive)}]):
            self.assertEqual(self.sb.resolve_folder('@Data/work'), drive / 'work')
            self.assertEqual(self.sb.folder_of(str(drive / 'work')), '@Data/work')
            for bad in ('@Data/../x', '@Data/work/.git', '@Other/work'):
                with self.assertRaises(ValueError):
                    self.sb.resolve_folder(bad)
        self.assertIsNone(self.sb.folder_of(str(drive / 'work')))  # without the place, a client cannot name it

    def test_how_a_chat_folder_is_named(self):
        f = self.sb.folder_of
        self.assertEqual(f(str(self.root / 'proj')), 'proj')
        self.assertEqual(f(str(self.home / 'Documents' / 'site')), '~/Documents/site')
        self.assertIsNone(f(str(self.home / '.ssh')))
        self.assertIsNone(f('/etc'))

    def test_new_folders_still_take_plain_names(self):
        self.assertEqual(self.sb.resolve_folder('~/Documents/new one', must_exist=False, strict_last=True),
                         self.home / 'Documents' / 'new one')
        for bad in ('~/Documents/a;b', 'proj/$(x)', '~/.hidden'):
            with self.assertRaises(ValueError):
                self.sb.resolve_folder(bad, must_exist=False, strict_last=True)
        r = self.rpc(verb='mkdir', folder='~/Documents/made')
        self.assertEqual((r['ok'], r['folder']), (True, '~/Documents/made'))
        self.assertTrue((self.home / 'Documents' / 'made').is_dir())
        self.assertFalse(self.cjson.exists())  # outside the Claude folder nothing is trusted until the chat asks


class PlacesListTest(PlacesBase):
    def test_the_claude_folder_home_its_folders_and_icloud(self):
        with mock.patch.object(self.sb.sys, 'platform', 'darwin'):
            r = self.rpc(verb='places')
        names = [(p['kind'], p['folder']) for p in r['places']]
        self.assertEqual(names[0], ('claude', ''))
        self.assertIn(('home', '~'), names)
        self.assertIn(('documents', '~/Documents'), names)
        self.assertIn(('icloud', '~/Library/Mobile Documents/com~apple~CloudDocs'), names)
        self.assertTrue(r['roam'])
        with self.conf(roam=False):
            self.assertEqual([p['kind'] for p in self.rpc(verb='places')['places']], ['claude'])

    def test_a_folder_the_system_keeps_closed_says_so(self):
        real = os.scandir
        def scandir(p):
            if str(p).endswith('Documents'):
                raise PermissionError(1, 'Operation not permitted')
            return real(p)
        with mock.patch.object(self.sb.os, 'scandir', scandir):
            docs = [p for p in self.rpc(verb='places')['places'] if p['kind'] == 'documents'][0]
        self.assertEqual(docs['state'], 'privacy')


class BrowseTest(PlacesBase):
    def test_home_lists_without_private_folders(self):
        r = self.rpc(verb='files', folder='~')
        self.assertTrue(r['ok'], r)
        self.assertEqual([d['name'] for d in r['dirs']], ['Desktop', 'Documents', 'Downloads'])
        self.assertEqual(r['shown'], '~')
        r = self.rpc(verb='files', folder='~/Desktop')
        self.assertEqual(r['dirs'], [{'folder': '', 'name': 'Claude'}])  # the Claude folder is named as itself
        r = self.rpc(verb='files', folder='~/Documents/site', name='README.md')
        self.assertEqual(r['file']['text'], 'hi')
        self.assertEqual(r['trust'], 'ask')

    def test_names_with_any_characters_browse(self):
        odd = self.home / 'Documents' / 'Résumé & notes (2026)'
        odd.mkdir()
        (odd / 'café.txt').write_text('ok')
        r = self.rpc(verb='files', folder='~/Documents')
        self.assertIn('~/Documents/Résumé & notes (2026)', [d['folder'] for d in r['dirs']])
        self.assertEqual(self.rpc(verb='files', folder='~/Documents/Résumé & notes (2026)', name='café.txt')['file']['text'], 'ok')

    def test_a_folder_macos_keeps_closed_is_not_empty_but_blocked(self):
        real = Path.iterdir
        def iterdir(self_):
            if self_.name == 'Downloads':
                raise PermissionError(1, 'Operation not permitted')
            return real(self_)
        with mock.patch.object(Path, 'iterdir', iterdir), mock.patch.object(self.sb.sys, 'platform', 'darwin'):
            r = self.rpc(verb='files', folder='~/Downloads')
        self.assertEqual((r['ok'], r['blocked']), (True, 'privacy'))
        self.assertIn('Files and Folders', r['hint'])


class TrustTest(PlacesBase):
    def start(self, folder, **kw):
        return self.rpc(verb='start', folder=folder, prompt='hello', **kw)

    def test_outside_the_claude_folder_the_user_says_yes_first(self):
        r = self.start('~/Documents/site')
        self.assertFalse(r['ok'])
        self.assertEqual((r['need'], r['folder_shown'], r['declined']), ('trust', '~/Documents/site', False))
        self.assertEqual(self.rpc(verb='folder_check', folder='~/Documents/site')['trust'], 'ask')
        self.assertTrue(self.start('~/Documents/site', trust=True)['ok'])
        projects = json.loads(self.cjson.read_text())['projects']
        self.assertTrue(projects[self.sb.trust_key(self.home / 'Documents' / 'site')]['hasTrustDialogAccepted'])
        self.assertTrue(self.start('~/Documents/site/src')['ok'])  # under a folder Claude was let into
        self.assertEqual(self.rpc(verb='folder_check', folder='~/Documents/site/src')['trust'], 'yes')

    def test_a_no_is_never_overridden_without_asking(self):
        self.cjson.write_text(json.dumps({'projects': {self.sb.trust_key(self.home / 'Documents'): {'hasTrustDialogAccepted': False}}}))
        r = self.start('~/Documents/site')
        self.assertEqual((r['ok'], r['declined']), (False, True))
        self.assertEqual(self.rpc(verb='folder_check', folder='~/Documents/site')['trust'], 'no')
        self.assertTrue(self.start('~/Documents/site', trust=True)['ok'])

    def test_inside_the_claude_folder_nothing_is_asked(self):
        self.assertTrue(self.start('proj')['ok'])
        self.assertEqual(self.rpc(verb='folder_check', folder='proj')['trust'], 'yes')

    def test_a_chat_in_home_is_resumed_where_it_ran(self):
        site = self.home / 'Documents' / 'site'
        self.assertEqual(self.sb.launch_dir(str(site)), str(site))
        self.assertTrue(self.sb.in_places(str(site)))
        self.assertFalse(self.sb.in_places('/etc'))
        self.assertEqual(self.sb.launch_dir('/etc'), str(self.root.resolve()))
        sid = '44444444-4444-4444-8444-444444444444'
        self.transcript(sid, str(site), NOW - 100)
        self.assertTrue(self.rpc(verb='resume', sid=sid)['ok'])
        s = [x for x in self.rpc(verb='list')['sessions'] if x['sid'] == sid][0]
        self.assertEqual(s['folder'], '~/Documents/site')


class PlacesCliTest(PlacesBase):
    def test_add_and_remove_a_drive(self):
        drive = (self.tmp / 'drive').resolve()
        drive.mkdir()
        conf = self.sb.SB_HOME / 'agent.json'
        with mock.patch('builtins.print'):
            self.assertEqual(self.sb.places_cli(['add', str(drive), '--name', 'Data']), 0)
            self.assertEqual(self.sb.agent_conf()['places'], [{'name': 'Data', 'path': str(drive)}])
            self.assertEqual(self.sb.places_cli(['roam', 'off']), 0)
            self.assertIs(self.sb.agent_conf()['roam'], False)
            self.assertEqual(self.sb.places_cli(['remove', 'Data']), 0)
            self.assertEqual(self.sb.places_cli(['add', str(self.tmp / 'nope')]), 1)
        self.assertEqual(json.loads(conf.read_text())['places'], [])

    def test_the_server_can_turn_roaming_off_but_never_add_a_place(self):
        self.sb.remember_identity({'as': 'studio', 'roam': False, 'places': [{'name': 'Root', 'path': '/'}]})
        self.assertIs(self.sb.agent_conf()['roam'], False)
        self.assertNotIn('places', self.sb.agent_conf())


if __name__ == '__main__':
    unittest.main()
