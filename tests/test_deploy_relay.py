"""scripts/deploy-relay.sh: what it accepts, the relay config and the systemd unit it would write, and an install
into a throwaway folder with stand-ins for systemctl, curl, uname and the relay itself first on PATH. Nothing is
downloaded and no service is touched."""
import hashlib, io, os, shutil, subprocess, tarfile, tempfile, unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / 'scripts' / 'deploy-relay.sh'

LOGGED = '''#!/bin/sh
echo "$(basename "$0") $*" >> "$FAKE_LOG"
exit 0
'''
UNAME = '''#!/bin/sh
case "$1" in -s|"") echo Linux ;; -m) echo x86_64 ;; *) exec /usr/bin/uname "$@" ;; esac
'''
# curl as the script calls it: a download (-o FILE URL) is a copy of $FAKE_DL, anything else is the health check.
CURL = '''#!/bin/sh
echo "curl $*" >> "$FAKE_LOG"
out=
while [ $# -gt 0 ]; do [ "$1" = -o ] && out=$2; shift; done
[ -n "$out" ] && { cp "$FAKE_DL" "$out"; exit; }
exit 0
'''
RELAY = '#!/bin/sh\n[ "$1" = --version ] && echo "iroh-relay 1.5.0"\nexit 0\n'


def run(*args, env=None):
    return subprocess.run(['bash', str(SCRIPT), *args], capture_output=True, text=True, env=env, timeout=60)


@unittest.skipIf(os.name == 'nt' or not shutil.which('bash'), 'needs bash')
class Arguments(unittest.TestCase):
    def refused(self, *args):
        r = run(*args)
        self.assertEqual(r.returncode, 1, r.stdout + r.stderr)
        self.assertEqual(r.stdout, '')
        self.assertTrue(r.stderr.startswith('deploy-relay: '), r.stderr)
        return r.stderr

    def test_no_arguments_prints_the_usage(self):
        r = run()
        self.assertEqual(r.returncode, 0)
        self.assertIn('deploy-relay.sh [HOST] install --hostname', r.stdout)
        self.assertNotIn('set -euo', r.stdout)

    def test_a_mode_is_required(self):
        self.assertIn('--dev, or --hostname', self.refused('config'))

    def test_production_needs_a_name_and_a_contact(self):
        self.assertIn('--contact', self.refused('config', '--hostname', 'relay.example.com'))
        self.assertIn('full DNS name', self.refused('config', '--hostname', 'relay', '--contact', 'a@example.com'))
        self.assertIn('DNS name', self.refused('config', '--hostname', 'relay.example.com"\nx = 1', '--contact', 'a@example.com'))
        self.assertIn('email', self.refused('config', '--hostname', 'relay.example.com', '--contact', 'me"'))
        self.assertIn('email', self.refused('config', '--hostname', 'relay.example.com', '--contact', 'nobody'))

    def test_dev_and_a_hostname_do_not_mix(self):
        self.assertIn('do not go together', self.refused('config', '--dev', '--hostname', 'r.example.com', '--contact', 'a@example.com'))

    def test_a_user_service_is_dev_only(self):
        self.assertIn('--user only works with --dev',
                      self.refused('unit', '--user', '--hostname', 'r.example.com', '--contact', 'a@example.com'))
        self.assertIn('--prefix only works with --user', self.refused('unit', '--dev', '--prefix', '/tmp/x'))

    def test_bad_values(self):
        self.assertIn('whole number', self.refused('config', '--dev', '--port', 'abc'))
        self.assertIn('1 to 65535', self.refused('config', '--dev', '--port', '70000'))
        self.assertIn('needs a value', self.refused('config', '--dev', '--port'))
        self.assertIn('unknown option', self.refused('config', '--dev', '--frobnicate'))
        self.assertIn('IP address', self.refused('config', '--dev', '--bind', 'example.com'))
        self.assertIn('--name', self.refused('unit', '--dev', '--name', 'My Relay'))
        self.assertIn('1 or more', self.refused('config', '--dev', '--rx-bytes', '0'))
        self.assertIn('--prefix', self.refused('unit', '--dev', '--user', '--prefix', '/tmp/a"b'))

    def test_after_a_host_a_verb_is_required(self):
        self.assertIn('after the host', self.refused('some-host'))
        self.assertIn('after the host', self.refused('some-host', '--dev'))
        self.assertIn('unknown option', self.refused('--dev'))

    def test_printing_never_leaves_this_machine(self):
        env = dict(os.environ, PATH='/usr/bin:/bin')
        with tempfile.TemporaryDirectory() as d:
            (Path(d) / 'ssh').write_text('#!/bin/sh\necho ssh was called >&2\nexit 9\n')
            (Path(d) / 'ssh').chmod(0o755)
            env['PATH'] = d + os.pathsep + env['PATH']
            r = run('some-host', 'config', '--dev', env=env)
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertIn('http_bind_addr', r.stdout)

    def test_a_host_gets_the_script_and_quoted_arguments(self):
        with tempfile.TemporaryDirectory() as d:
            log = Path(d) / 'log'
            (Path(d) / 'ssh').write_text('#!/bin/sh\nprintf "%s\\n" "$@" > "$FAKE_LOG"\nhead -1 >> "$FAKE_LOG"\n')
            (Path(d) / 'ssh').chmod(0o755)
            env = dict(os.environ, PATH=d + os.pathsep + os.environ['PATH'], FAKE_LOG=str(log))
            r = run('me@box', 'install', '--dev', '--prefix', '/tmp/a b; rm -rf x', '--user', env=env)
            self.assertEqual(r.returncode, 0, r.stderr)
            lines = log.read_text().splitlines()
        self.assertEqual(lines[0], 'me@box')
        self.assertTrue(lines[1].startswith('bash -s -- install --dev --prefix '), lines[1])
        self.assertNotIn('a b; rm', lines[1])  # quoted for the far shell, not pasted in
        self.assertIn('/tmp/a\\ b\\;\\ rm\\ -rf\\ x', lines[1])
        self.assertEqual(lines[2], '#!/usr/bin/env bash')


@unittest.skipIf(os.name == 'nt' or not shutil.which('bash'), 'needs bash')
class Text(unittest.TestCase):
    def out(self, *args):
        r = run(*args)
        self.assertEqual(r.returncode, 0, r.stderr)
        return r.stdout

    def toml(self, text):
        try:
            import tomllib
        except ImportError:  # Python before 3.11
            self.skipTest('no tomllib')
        return tomllib.loads(text)

    def test_dev_config(self):
        text = self.out('config', '--dev')
        self.assertIn('http_bind_addr = "[::]:3340"\n', text)
        self.assertIn('metrics_bind_addr = "127.0.0.1:9090"\n', text)  # metrics never on a public address
        self.assertIn('access = "everyone"\n', text)
        self.assertNotIn('[tls]', text)
        self.assertNotIn('enable_quic_addr_discovery', text)  # the relay refuses to start with it and no TLS
        c = self.toml(text)
        self.assertEqual(c['limits'], {'accept_conn_limit': 50.0, 'accept_conn_burst': 100,
                                       'client': {'rx': {'bytes_per_second': 4194304, 'max_burst_bytes': 8388608}}})

    def test_dev_options(self):
        text = self.out('config', '--dev', '--port', '3999', '--metrics-port', '9393', '--bind', '203.0.113.7', '--no-limits')
        self.assertIn('http_bind_addr = "203.0.113.7:3999"\n', text)
        self.assertIn('metrics_bind_addr = "127.0.0.1:9393"\n', text)
        self.assertNotIn('limits', text)
        self.assertIn('http_bind_addr = "[fd7a::1]:3340"\n', self.out('config', '--dev', '--bind', 'fd7a::1'))

    def test_production_config(self):
        text = self.out('config', '--hostname', 'relay.example.com', '--contact', 'ops@example.com',
                        '--rx-bytes', '1048576', '--rx-burst', '2097152', '--conn-rate', '10', '--conn-burst', '20')
        c = self.toml(text)
        self.assertEqual(c['http_bind_addr'], '[::]:80')
        self.assertIs(c['enable_quic_addr_discovery'], True)
        self.assertEqual(c['access'], 'everyone')
        self.assertEqual(c['metrics_bind_addr'], '127.0.0.1:9090')
        self.assertEqual(c['tls'], {'https_bind_addr': '[::]:443', 'quic_bind_addr': '[::]:7842', 'hostname': 'relay.example.com',
                                    'cert_mode': 'LetsEncrypt', 'contact': 'ops@example.com',
                                    'cert_dir': '/var/lib/switchboard-relay/certs'})
        self.assertEqual(c['limits'], {'accept_conn_limit': 10.0, 'accept_conn_burst': 20,
                                       'client': {'rx': {'bytes_per_second': 1048576, 'max_burst_bytes': 2097152}}})
        # Every key is one iroh-relay 1.5.0 reads (src/main.rs Config, TlsConfig, Limits).
        self.assertLessEqual(set(c), {'http_bind_addr', 'enable_quic_addr_discovery', 'metrics_bind_addr', 'access', 'tls', 'limits'})

    def test_system_unit(self):
        text = self.out('unit', '--hostname', 'relay.example.com', '--contact', 'ops@example.com')
        self.assertIn('ExecStart="/opt/switchboard-relay/iroh-relay" --config-path "/etc/switchboard-relay/relay.toml"\n', text)
        for line in ('User=sb-relay', 'Group=sb-relay', 'AmbientCapabilities=CAP_NET_BIND_SERVICE', 'NoNewPrivileges=true',
                     'ProtectSystem=strict', 'ReadWritePaths=/var/lib/switchboard-relay', 'KillSignal=SIGINT',
                     'Restart=on-failure', 'WantedBy=multi-user.target'):
            self.assertIn(line + '\n', text)
        self.assertNotIn('--dev', text)

    def test_user_unit(self):
        text = self.out('unit', '--dev', '--user', '--name', 'relay-two', '--prefix', '/srv/my relay')
        self.assertIn('ExecStart="/srv/my relay/iroh-relay" --config-path "/srv/my relay/relay.toml" --dev\n', text)
        self.assertIn('WantedBy=default.target\n', text)
        for word in ('User=', 'AmbientCapabilities', 'ProtectSystem'):
            self.assertNotIn(word, text)


@unittest.skipIf(os.name == 'nt' or not shutil.which('bash'), 'needs bash')
class Install(unittest.TestCase):
    """A --user install into a temporary folder. The download is a tarball made here; its checksum is not the
    pinned one, so the first test is that the script refuses it."""

    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, self.tmp, True)
        fake = self.tmp / 'fake'
        fake.mkdir()
        for name, body in (('systemctl', LOGGED), ('journalctl', LOGGED), ('uname', UNAME), ('curl', CURL)):
            (fake / name).write_text(body)
            (fake / name).chmod(0o755)
        self.log = self.tmp / 'log'
        self.log.write_text('')
        self.tgz = self.tmp / 'relay.tgz'
        with tarfile.open(self.tgz, 'w:gz') as t:
            info = tarfile.TarInfo('./iroh-relay')
            info.size, info.mode = len(RELAY), 0o755
            t.addfile(info, io.BytesIO(RELAY.encode()))
        self.prefix = self.tmp / 'relay'
        self.env = dict(os.environ, PATH=str(fake) + os.pathsep + os.environ['PATH'], HOME=str(self.tmp / 'home'),
                        FAKE_LOG=str(self.log), FAKE_DL=str(self.tgz))
        self.env.pop('XDG_CONFIG_HOME', None)
        self.unit = self.tmp / 'home' / '.config' / 'systemd' / 'user' / 'relay-t.service'

    def install(self, *more):
        return run('install', '--dev', '--user', '--name', 'relay-t', '--prefix', str(self.prefix), *more, env=self.env)

    def place_relay(self):
        """The right binary is already there, as after a first install."""
        self.prefix.mkdir()
        (self.prefix / 'iroh-relay').write_text(RELAY)
        (self.prefix / 'iroh-relay').chmod(0o755)

    def calls(self, word):
        return [line for line in self.log.read_text().splitlines() if line.startswith(word)]

    def test_a_download_with_the_wrong_checksum_is_refused(self):
        r = self.install()
        self.assertEqual(r.returncode, 1)
        self.assertIn('does not match the expected checksum (got ' + hashlib.sha256(self.tgz.read_bytes()).hexdigest(), r.stderr)
        self.assertFalse((self.prefix / 'iroh-relay').exists())
        self.assertFalse(self.unit.exists())
        self.assertEqual(self.calls('systemctl'), [])
        url = self.calls('curl')[0].split()[-1]
        self.assertEqual(url, 'https://github.com/n0-computer/iroh/releases/download/v1.5.0/'
                              'iroh-relay-v1.5.0-x86_64-unknown-linux-musl.tar.gz')

    def test_install_writes_config_and_unit_and_starts_it(self):
        self.place_relay()
        r = self.install('--port', '3999')
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertIn('is running as relay-t (updated)', r.stdout)
        self.assertIn(':3999', r.stdout)
        self.assertEqual((self.prefix / 'relay.toml').read_text(), run('config', '--dev', '--port', '3999').stdout)
        self.assertIn(f'ExecStart="{self.prefix}/iroh-relay" --config-path "{self.prefix}/relay.toml" --dev', self.unit.read_text())
        self.assertTrue((self.prefix / 'state').is_dir())
        self.assertEqual(self.calls('systemctl'), ['systemctl --user daemon-reload', 'systemctl --user enable relay-t',
                                                   'systemctl --user restart relay-t'])
        self.assertEqual(len(self.calls('curl')), 1)  # the health check only: the binary was there
        self.assertIn('http://127.0.0.1:3999/healthz', self.calls('curl')[0])

    def test_a_second_install_changes_nothing(self):
        self.place_relay()
        self.assertEqual(self.install().returncode, 0)
        before = {p: p.stat().st_mtime_ns for p in (self.prefix / 'relay.toml', self.unit)}
        self.log.write_text('')
        r = self.install()
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertIn('(no change)', r.stdout)
        self.assertEqual({p: p.stat().st_mtime_ns for p in before}, before)
        self.assertEqual(self.calls('systemctl'), ['systemctl --user enable relay-t', 'systemctl --user start relay-t'])
        self.assertFalse((self.prefix / 'relay.toml.bak').exists())

    def test_other_options_rewrite_the_config_and_restart(self):
        self.place_relay()
        self.assertEqual(self.install().returncode, 0)
        old = (self.prefix / 'relay.toml').read_text()
        self.log.write_text('')
        self.assertEqual(self.install('--rx-bytes', '1048576').returncode, 0)
        self.assertIn('bytes_per_second = 1048576', (self.prefix / 'relay.toml').read_text())
        self.assertEqual((self.prefix / 'relay.toml.bak').read_text(), old)
        self.assertEqual(self.calls('systemctl'), ['systemctl --user enable relay-t', 'systemctl --user restart relay-t'])

    def test_uninstall_removes_only_its_own_files_and_can_run_twice(self):
        self.place_relay()
        self.assertEqual(self.install().returncode, 0)
        other = self.unit.parent / 'something-else.service'
        other.write_text('[Unit]\n')
        self.log.write_text('')
        for _ in range(2):
            r = run('uninstall', '--user', '--name', 'relay-t', '--prefix', str(self.prefix), env=self.env)
            self.assertEqual(r.returncode, 0, r.stderr)
            self.assertIn('relay-t is removed', r.stdout)
        self.assertFalse(self.prefix.exists())
        self.assertFalse(self.unit.exists())
        self.assertTrue(other.exists())
        self.assertEqual(self.calls('systemctl')[:2], ['systemctl --user disable --now relay-t', 'systemctl --user daemon-reload'])

    def test_a_system_install_needs_root(self):
        if os.geteuid() == 0:
            self.skipTest('running as root')
        r = run('install', '--dev', env=self.env)
        self.assertEqual(r.returncode, 1)
        self.assertIn('as root', r.stderr)
        self.assertEqual(self.calls('systemctl'), [])

    def test_status_and_logs(self):
        r = run('status', '--user', '--name', 'relay-t', '--prefix', str(self.prefix), env=self.env)
        self.assertEqual(r.returncode, 1)
        self.assertIn('relay-t is not installed here', r.stderr)
        self.place_relay()
        self.assertEqual(self.install().returncode, 0)
        self.log.write_text('')
        r = run('status', '--user', '--name', 'relay-t', '--prefix', str(self.prefix), env=self.env)
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertIn('systemctl --user --no-pager status relay-t', self.calls('systemctl'))
        self.assertIn('http://127.0.0.1:9090/metrics', self.calls('curl')[0])
        r = run('logs', '--user', '--name', 'relay-t', env=self.env)
        self.assertEqual(self.calls('journalctl'), ['journalctl --user --no-pager -n 200 -u relay-t'])


if __name__ == '__main__':
    unittest.main()
