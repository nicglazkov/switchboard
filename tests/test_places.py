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



def case_blind(d):
    """This folder's file system takes Library and library for one name (macOS and Windows as they come)."""
    (d / 'CaseProbe').mkdir(exist_ok=True)
    return (d / 'caseprobe').is_dir()


class PrivateByAnyNameTest(PlacesBase):
    """What is private stays private however it is asked for: another letter case, a symlink on the way, the Claude
    folder being the home folder. The check runs on the real path."""

    def setUp(self):
        super().setUp()
        (self.home / 'Library' / 'Keychains' / 'login.txt').write_text('private')
        (self.home / 'AppData' / 'Roaming' / 'app.txt').write_text('private')
        self.blind = case_blind(self.tmp)

    def refused(self, **req):
        r = self.rpc(**req)
        self.assertFalse(r['ok'], req)
        self.assertNotIn('private"', json.dumps(r.get('file') or {}))
        return r

    def test_another_letter_case(self):
        if not self.blind:
            self.skipTest('this file system tells Library from library')
        for bad in ('~/library', '~/LIBRARY/Keychains', '~/library/keychains', '~/appdata/Roaming', '~/APPDATA',
                    '~/Library/mobile documents', '~/library/Mobile Documents'):
            with self.subTest(bad=bad):
                with self.assertRaises(ValueError):
                    self.sb.resolve_folder(bad)
                self.refused(verb='files', folder=bad)
                self.refused(verb='folder_check', folder=bad)
        self.refused(verb='files', folder='~/library/Keychains', name='login.txt')
        self.refused(verb='start', folder='~/library/Keychains', prompt='read it', trust=True)
        self.refused(verb='mkdir', folder='~/library/New')
        self.assertFalse((self.home / 'Library' / 'New').exists())
        self.assertIsNone(self.sb.folder_of(str(self.home / 'library' / 'Keychains')))
        # iCloud Drive is the one folder in Library that is shown, by any spelling of it
        self.assertTrue(self.rpc(verb='files', folder='~/library/mobile documents/com~apple~clouddocs')['ok'])

    def test_the_names_are_compared_as_the_file_system_does(self):
        with mock.patch.object(self.sb, 'IS_WIN', True):
            self.assertTrue(self.sb._private(('appdata', 'Roaming')))
            self.assertTrue(self.sb._private(('LOCAL SETTINGS',)))
            self.assertFalse(self.sb._private(('library', 'mobile documents', 'COM~APPLE~CLOUDDOCS', 'Notes')))
            self.assertEqual(self.sb._below('/Users/You', '/users/you/AppData'), ('AppData',))
        with mock.patch.object(self.sb, 'IS_WIN', False), mock.patch.object(self.sb.sys, 'platform', 'linux'):
            self.assertFalse(self.sb._private(('library',)), 'on Linux that is a folder of its own')
            self.assertTrue(self.sb._private(('Library',)))
            self.assertIsNone(self.sb._below('/home/you', '/home/You/x'))

    def test_a_symlink_into_a_private_folder(self):
        os.symlink(self.home / '.ssh', self.home / 'Documents' / 'keys')
        os.symlink(self.home / 'Library' / 'Keychains', self.home / 'Documents' / 'chains')
        os.symlink(self.home / 'Library' / 'Mobile Documents' / 'com~apple~CloudDocs', self.home / 'Documents' / 'cloud')
        for bad in ('~/Documents/keys', '~/Documents/chains'):
            with self.subTest(bad=bad):
                with self.assertRaises(ValueError):
                    self.sb.resolve_folder(bad)
                self.refused(verb='files', folder=bad)
        self.refused(verb='files', folder='~/Documents/keys', name='id_ed25519')
        self.refused(verb='files', folder='~/Documents/chains', name='login.txt')
        self.refused(verb='start', folder='~/Documents/keys', prompt='read it', trust=True)
        self.assertEqual(self.sb.resolve_folder('~/Documents/cloud'), self.home.joinpath(*self.sb.ICLOUD_REL))
        self.assertNotIn('keys', [d['name'] for d in self.rpc(verb='files', folder='~/Documents')['dirs']])

    def test_a_place_the_owner_made_inside_a_private_folder_is_shown(self):
        inside = self.home / 'Library' / 'CloudStorage' / 'Drive'
        (inside / 'work' / '.git').mkdir(parents=True)
        (inside / 'work' / 'notes.txt').write_text('hello')
        with self.conf(places=[{'name': 'Drive', 'path': str(inside)}, {'name': 'Home', 'path': str(self.home)}]):
            self.assertFalse(self.sb._files_hidden(inside / 'work'))
            self.assertTrue(self.sb._files_hidden(inside / 'work' / '.git'))
            self.assertEqual(self.rpc(verb='files', folder='@Drive/work', name='notes.txt')['file']['text'], 'hello')
            # home named as a place opens nothing home keeps private
            for p in ('Library', 'Library/Keychains', '.ssh', 'AppData/Roaming'):
                self.assertTrue(self.sb._files_hidden(self.home / p), p)
            self.assertFalse(self.sb._files_hidden(self.home / 'Documents' / 'site'))
        self.assertTrue(self.sb._files_hidden(inside / 'work'), 'not a place: private as before')

    def test_a_symlink_inside_a_place_into_its_dot_folder(self):
        drive = (self.tmp / 'drive').resolve()
        (drive / '.secrets').mkdir(parents=True)
        (drive / '.secrets' / 'k.txt').write_text('private')
        os.symlink(drive / '.secrets', drive / 'plain')
        with self.conf(places=[{'name': 'Data', 'path': str(drive)}]):
            with self.assertRaises(ValueError):
                self.sb.resolve_folder('@Data/plain')
            self.refused(verb='files', folder='@Data/plain', name='k.txt')
            self.assertTrue(self.rpc(verb='files', folder='@Data')['ok'])

    def test_a_file_reached_by_a_name_that_is_not_its_own(self):
        d = self.home / 'Documents' / 'site'
        (d / 'CREDEN~1.JSO').write_text('private')
        real = Path.resolve

        def resolve(p, *a, **kw):  # as Windows does for a short name: CREDEN~1.JSO is credentials.json
            r = real(p, *a, **kw)
            return r.with_name('credentials.json') if r.name == 'CREDEN~1.JSO' else r
        with mock.patch.object(Path, 'resolve', resolve):
            f = self.rpc(verb='files', folder='~/Documents/site', name='CREDEN~1.JSO')['file']
        self.assertTrue(f['secret'])
        self.assertNotIn('text', f)


class HomeIsTheClaudeFolderTest(PlacesBase):
    """A machine whose Claude folder was never picked uses the home folder: its private folders are as private by
    their plain names as by their '~/' names."""

    def setUp(self):
        super().setUp()
        os.environ['SB_ROOT'] = str(self.home)
        (self.home / 'Library' / 'Keychains' / 'login.txt').write_text('private')
        (self.home / 'Library' / 'Mobile Documents' / 'com~apple~CloudDocs' / 'Notes' / 'a.txt').write_text('a note')
        (self.home / 'Documents' / 'site' / '.git').mkdir()

    def test_files_never_lists_or_opens_them(self):
        r = self.rpc(verb='files', folder='')
        self.assertTrue(r['ok'], r)
        self.assertEqual([d['name'] for d in r['dirs']], ['Desktop', 'Documents', 'Downloads'])
        for bad in ('Library', 'Library/Keychains', 'AppData', 'AppData/Roaming', '.ssh', 'Library/Mobile Documents'):
            with self.subTest(bad=bad):
                self.assertFalse(self.rpc(verb='files', folder=bad)['ok'])
                self.assertFalse(self.rpc(verb='folder_check', folder=bad)['ok'])
                with self.assertRaises(ValueError):
                    self.sb.resolve_folder(bad)
        r = self.rpc(verb='files', folder='Library/Keychains', name='login.txt')
        self.assertFalse(r['ok'])
        self.assertNotIn('private"', json.dumps(r))
        self.assertFalse(self.rpc(verb='files', folder='', name='.ssh')['ok'])

    def test_no_chat_or_new_folder_goes_there(self):
        for bad in ('Library/Keychains', '.ssh', 'AppData/Roaming'):
            self.assertFalse(self.rpc(verb='start', folder=bad, prompt='read it')['ok'], bad)
        self.assertFalse(self.rpc(verb='mkdir', folder='Library/New')['ok'])
        self.assertFalse((self.home / 'Library' / 'New').exists())
        self.assertEqual(list(self.sb.paths_q()['queue'].glob('*.json')), [])

    def test_the_pickers_leave_them_out(self):
        names = [d['name'] for d in self.rpc(verb='folders', parent='')['folders']]
        self.assertEqual(names, ['Desktop', 'Documents', 'Downloads'])
        self.assertFalse(self.rpc(verb='folders', parent='Library')['ok'])
        self.assertNotIn('AppData', [d['folder'] for d in self.rpc(verb='folders')['folders']])

    def test_a_chat_that_ran_there_has_no_folder_to_name(self):
        f = self.sb.folder_of
        self.assertIsNone(f(str(self.home / 'Library' / 'Keychains')))
        self.assertIsNone(f(str(self.home / '.ssh')))
        self.assertIsNone(f(str(self.home / 'AppData' / 'Roaming')))
        self.assertEqual(f(str(self.home / 'Documents' / 'site')), 'Documents/site')
        self.assertEqual(f(str(self.home)), '')

    def test_what_is_not_private_still_works(self):
        icloud = '~/Library/Mobile Documents/com~apple~CloudDocs/Notes'  # as the places list names iCloud Drive
        self.assertEqual(self.sb.resolve_folder(icloud), self.home.joinpath(*icloud[2:].split('/')))
        self.assertEqual(self.rpc(verb='files', folder=icloud, name='a.txt')['file']['text'], 'a note')
        self.assertEqual(self.rpc(verb='files', folder='Documents/site', name='README.md')['file']['text'], 'hi')
        # a dot folder inside a project is where a chat may run (a worktree), though Files never shows it
        self.assertEqual(self.sb.resolve_folder('Documents/site/.git'), self.home / 'Documents' / 'site' / '.git')
        self.assertFalse(self.rpc(verb='files', folder='Documents/site/.git')['ok'])

    def test_a_symlink_in_home_into_a_private_folder(self):
        os.symlink(self.home / 'Library' / 'Keychains', self.home / 'chains')
        with self.assertRaises(ValueError):
            self.sb.resolve_folder('chains')
        r = self.rpc(verb='files', folder='chains', name='login.txt')
        self.assertFalse(r['ok'])
        self.assertNotIn('private"', json.dumps(r))


class ClaudeFolderInsideAPrivateFolderTest(PlacesBase):
    def test_an_owner_who_put_it_there_still_has_it(self):
        root = self.home / '.work' / 'Claude'
        (root / 'proj').mkdir(parents=True)
        (root / 'proj' / 'a.txt').write_text('mine')
        os.environ['SB_ROOT'] = str(root)
        self.assertEqual(self.sb.resolve_folder('proj'), root / 'proj')
        self.assertEqual(self.sb.folder_of(str(root / 'proj')), 'proj')
        self.assertEqual(self.rpc(verb='files', folder='proj', name='a.txt')['file']['text'], 'mine')


if __name__ == '__main__':
    unittest.main()
