"""scripts/install/setup.py: the addresses it offers, the server.json it writes, folders, and the pasted lines."""
import importlib.util, os, sys, unittest
from pathlib import Path
from unittest import mock

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('sb_setup', ROOT / 'scripts' / 'install' / 'setup.py')
setup = importlib.util.module_from_spec(spec)
spec.loader.exec_module(setup)


class Candidates(unittest.TestCase):
    def test_order_and_notes(self):
        got = setup.candidates(lan=lambda: '10.0.0.5', ts=lambda: (['100.64.0.12'], 'studio.tail1234.ts.net'),
                               hostname=lambda: 'Studio.lan', mac=False)
        self.assertEqual([a for a, _ in got], ['10.0.0.5', 'studio.tail1234.ts.net', '100.64.0.12', 'studio'])

    def test_mac_names_are_bonjour_names(self):
        got = setup.candidates(lan=lambda: None, ts=lambda: ([], None), hostname=lambda: 'Laptop', mac=True)
        self.assertEqual(got, [('laptop.local', "this machine's name")])

    def test_nothing_found(self):
        self.assertEqual(setup.candidates(lan=lambda: None, ts=lambda: ([], None), hostname=lambda: 'localhost'), [])


class ServerJson(unittest.TestCase):
    def test_chosen_address_first(self):
        d = setup.server_json('studio.local', ['10.0.0.5', 'studio.local'])
        self.assertEqual(d['hosts'], ['studio.local', '10.0.0.5'])
        self.assertEqual(d['origins'], ['http://studio.local:8130', 'http://10.0.0.5:8130'])
        self.assertEqual(d['bind'], ['0.0.0.0'])
        self.assertNotIn('allow', d)

    def test_keeps_what_the_person_wrote(self):
        old = {'bind': ['10.0.0.5'], 'hosts': ['nas'], 'origins': ['http://nas:9000'], 'allow': ['tailscale']}
        d = setup.server_json('10.0.0.5', [], old)
        self.assertEqual(d['bind'], ['10.0.0.5'])
        self.assertEqual(d['hosts'], ['10.0.0.5', 'nas'])
        self.assertEqual(d['origins'], ['http://10.0.0.5:8130', 'http://nas:8130', 'http://nas:9000'])
        self.assertEqual(d['allow'], ['tailscale'])

    def test_ipv6_in_brackets(self):
        self.assertEqual(setup.server_json('fd7a::1', [])['origins'], ['http://[fd7a::1]:8130'])

    def test_server_reads_what_setup_writes(self):
        sys.path.insert(0, str(ROOT / 'server'))
        import server
        import tempfile, json
        with tempfile.TemporaryDirectory() as tmp:
            Path(tmp, 'server.json').write_text(json.dumps(setup.server_json('studio.local', ['10.0.0.5'])))
            cfg = server.ServerConfig(tmp, tmp)
            self.assertTrue(cfg.host_ok('studio.local'))
            self.assertTrue(cfg.host_ok('10.0.0.5'))
            self.assertFalse(cfg.host_ok('evil.example.com'))
            self.assertEqual(cfg.origins[0], 'http://studio.local:8130')


class Addresses(unittest.TestCase):
    def test_valid(self):
        for a in ('10.0.0.5', 'studio.local', 'studio', 'fd7a::1', 'studio.tail1234.ts.net'):
            self.assertTrue(setup.valid_address(a), a)
        for a in ('', ' ', 'a b', 'http://x', '-oProxy', 'x;rm', 'host:8130'):
            self.assertFalse(setup.valid_address(a), a)


class Folders(unittest.TestCase):
    def test_full_folder(self):
        with mock.patch.object(setup.Path, 'home', return_value=Path('/home/you')):
            cases = {'Code': '~/Code', './Code': '~/Code', '~/Code': '~/Code', '~': '~', '': '~', '/srv/work': '/srv/work',
                     'C:\\Users\\you': 'C:\\Users\\you', 'D:/work': 'D:/work', '/home/you/Code': '~/Code', '/home/you': '~',
                     '/home/yourname/x': '/home/yourname/x'}
            for given, want in cases.items():
                self.assertEqual(setup.full_folder(given), want, given)

    def test_join(self):
        self.assertEqual(setup._join('~', ''), '~')
        self.assertEqual(setup._join('~', 'Desktop/Claude'), '~/Desktop/Claude')
        self.assertEqual(setup._join('~/Code/', 'app'), '~/Code/app')


class Lines(unittest.TestCase):
    def test_code_lines(self):
        lines = setup.link_lines('studio.local', 'ABC234')
        self.assertEqual(lines['linux'], 'curl -fsSL http://studio.local:8131/install-agent.sh | bash -s -- --server studio.local --code ABC234')
        self.assertEqual(lines['mac'], lines['linux'])
        self.assertIn("urlopen('http://studio.local:8131/install-agent.sh')", lines['linux (no curl)'])
        self.assertIn('-Server studio.local -Code ABC234', lines['windows'])
        self.assertIn('http://studio.local:8131/install-agent.ps1', lines['windows'])

    def test_ssh_lines_quote_the_key(self):
        pub = 'ssh-ed25519 AAAAC3Nza switchboard@server'
        for line in setup.ssh_lines('10.0.0.5', pub).values():
            self.assertIn(f'"{pub}"', line)


class Asking(unittest.TestCase):
    def test_choose_takes_default_and_retries(self):
        out = []
        answers = iter(['9', 'x', '2'])
        ask = setup.Ask(inp=lambda p: next(answers), out=out.append)
        self.assertEqual(ask.choose('Pick one', ['a', 'b', 'c']), 1)
        self.assertEqual(out.count('  Type a number from 1 to 3.'), 2)
        self.assertEqual(setup.Ask(inp=lambda p: '', out=out.append).choose('Pick one', ['a', 'b'], default=2), 1)

    def test_yes_never_asks(self):
        ask = setup.Ask(yes=True, inp=lambda p: self.fail('asked'), out=lambda *a: None)
        self.assertEqual(ask.line('Folder', '~'), '~')
        self.assertTrue(ask.yes_no('Go on?'))


if __name__ == '__main__':
    unittest.main()
