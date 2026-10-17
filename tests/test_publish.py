"""The signed update list (native/scripts/publish_render.py, update-key.sh): latest.json is made and signed where the
key is, and the publish host keeps it byte for byte and refuses one that does not match the files. Everything runs
in throwaway folders with a throwaway key."""
import base64, json, os, shutil, subprocess, sys, unittest
from pathlib import Path
from tempfile import mkdtemp

ROOT = Path(__file__).resolve().parents[1]
SCRIPTS = ROOT / 'native' / 'scripts'
sys.path.insert(0, str(SCRIPTS))
import publish_render  # noqa: E402

OPENSSL = os.environ.get('SB_OPENSSL') or shutil.which('openssl') or 'openssl'


def has_ed25519():
    d = mkdtemp()
    try:
        r = subprocess.run([OPENSSL, 'genpkey', '-algorithm', 'ed25519', '-out', os.path.join(d, 'k')],
                           capture_output=True, check=False)
        return r.returncode == 0
    except OSError:
        return False
    finally:
        shutil.rmtree(d)


def dist_with_builds(d):
    for name, body, build in (('Switchboard-mac.zip', b'mac app', '20261013120000'),
                              ('Switchboard-linux-x86_64', b'linux app!', '20261013120001')):
        (d / name).write_bytes(body)
        (d / (name + '.build')).write_text(build + '\n')


class ManifestTest(unittest.TestCase):
    def test_the_list_has_each_builds_size_hash_and_id(self):
        d = Path(mkdtemp())
        self.addCleanup(shutil.rmtree, d)
        dist_with_builds(d)
        m = json.loads(publish_render.make_manifest(str(d), '0.9.0'))
        self.assertEqual(m['version'], '0.9.0')
        self.assertEqual(sorted(m['files']), ['linux', 'mac'])
        self.assertEqual(m['files']['mac'], {'file': 'Switchboard-mac.zip', 'size': 7, 'build': '20261013120000',
                                             'sha256': publish_render.sha256(str(d / 'Switchboard-mac.zip'))})


@unittest.skipUnless(has_ed25519(), 'needs OpenSSL 3 with Ed25519 (set SB_OPENSSL)')
class SignTest(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(mkdtemp())
        self.addCleanup(shutil.rmtree, self.tmp)
        self.env = dict(os.environ, SB_OPENSSL=OPENSSL)
        self.key = self.tmp / 'keys' / 'update-key.pem'
        out = subprocess.run(['bash', str(SCRIPTS / 'update-key.sh'), str(self.key)], capture_output=True,
                             text=True, env=self.env, check=True).stdout
        self.pub = out.strip().splitlines()[-1].split('=', 1)[1]
        os.environ['SB_OPENSSL'] = OPENSSL
        self.addCleanup(os.environ.pop, 'SB_OPENSSL', None)

    def test_update_key_makes_a_pair_and_prints_the_public_half(self):
        self.assertEqual(len(base64.b64decode(self.pub)), 32)
        self.assertEqual(oct(self.key.stat().st_mode & 0o777), '0o600')
        self.assertEqual(publish_render.public_key(str(self.key)), self.pub)
        again = subprocess.run(['bash', str(SCRIPTS / 'update-key.sh'), str(self.key)], capture_output=True,
                               text=True, env=self.env)
        self.assertNotEqual(again.returncode, 0, 'it never replaces a key')
        inside = subprocess.run(['bash', str(SCRIPTS / 'update-key.sh'), str(ROOT / 'native' / 'k.pem')],
                                capture_output=True, text=True, env=self.env)
        self.assertNotEqual(inside.returncode, 0, 'and never writes one into the repo')
        self.assertFalse((ROOT / 'native' / 'k.pem').exists())

    def test_a_signature_checks_out_and_a_changed_list_does_not(self):
        data = b'{"version": "0.9.0"}\n'
        sig = base64.b64decode(publish_render.sign(str(self.key), data))
        self.assertEqual(len(sig), 64)
        try:
            from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PublicKey
            from cryptography.exceptions import InvalidSignature
        except ImportError:
            self.skipTest('cryptography is not installed')
        k = Ed25519PublicKey.from_public_bytes(base64.b64decode(self.pub))
        k.verify(sig, data)
        with self.assertRaises(InvalidSignature):
            k.verify(sig, data.replace(b'0.9.0', b'9.9.9'))

    def test_sign_refuses_a_key_the_apps_were_not_built_with(self):
        src = self.tmp / 'latest.json'
        src.write_bytes(b'{}')
        with self.assertRaises(SystemExit):
            publish_render.sign_main(['--key', str(self.key), '--in', str(src), '--out', str(self.tmp / 's'),
                                      '--expect', base64.b64encode(b'\0' * 32).decode()])
        publish_render.sign_main(['--key', str(self.key), '--in', str(src), '--out', str(self.tmp / 's'),
                                  '--expect', self.pub])
        self.assertTrue((self.tmp / 's').read_text().strip())


class RenderTest(unittest.TestCase):
    """The publish host's half, with HOME pointed at a throwaway folder."""

    def setUp(self):
        self.home = Path(mkdtemp())
        self.addCleanup(shutil.rmtree, self.home)
        self.stage = self.home / 'app.incoming'
        shutil.copytree(SCRIPTS / 'download', self.stage)
        dist_with_builds(self.stage)
        self.latest = publish_render.make_manifest(str(self.stage), '0.9.0')
        for b in self.stage.glob('*.build'):  # publish.sh uploads only the list, not these
            b.unlink()
        (self.stage / 'latest.json').write_bytes(self.latest)
        (self.stage / 'latest.json.sig').write_text('c2ln\n')

    def render(self):
        return subprocess.run([sys.executable, str(SCRIPTS / 'publish_render.py'), '--dest', 'app', '--base',
                               'http://192.0.2.10:8131/app', '--version', '0.9.0', '--allow-missing'],
                              capture_output=True, text=True, env=dict(os.environ, HOME=str(self.home)))

    def test_the_signed_list_is_published_byte_for_byte(self):
        r = self.render()
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual((self.home / 'app' / 'latest.json').read_bytes(), self.latest)
        self.assertEqual((self.home / 'app' / 'latest.json.sig').read_text(), 'c2ln\n')

    def test_a_list_that_does_not_match_the_files_is_refused(self):
        (self.stage / 'Switchboard-mac.zip').write_bytes(b'mac app, changed')
        r = self.render()
        self.assertNotEqual(r.returncode, 0)
        self.assertIn('does not match', r.stderr)
        self.assertFalse((self.home / 'app').exists(), 'nothing was swapped in')


if __name__ == '__main__':
    unittest.main()
