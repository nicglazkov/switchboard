#!/usr/bin/env python3
"""First-run setup for a Switchboard server: the address your devices use, your machines, and each machine's
Claude folder. Run it on the server machine after install-server (`switchboard setup`); run it again any time.

  setup.py                        ask, step by step
  setup.py --address 10.0.0.5     use this address (skips the first question)
  setup.py --code                 print a code and the lines that link a machine, then stop
  setup.py --ssh studio=me@host   add a machine reached over ssh
  setup.py --root studio=~/Code   set a machine's Claude folder
  setup.py --ssh-key              print the key a machine added over ssh lets in
  setup.py --yes                  take the suggested answer everywhere and ask nothing

It talks only to the server on this machine (http://127.0.0.1:8130) and writes ~/.config/switchboard/server.json.
Python 3.9 or newer, nothing to install.
"""
import argparse, json, os, re, shutil, socket, subprocess, sys, time, urllib.error, urllib.request
from pathlib import Path

PORT, HUB_PORT, DOWNLOADS_PORT = 8130, 8132, 8131
LOCAL = f'http://127.0.0.1:{PORT}'
CFG = Path.home() / '.config' / 'switchboard'
KEY = Path.home() / '.ssh' / 'switchboard_ed25519'
NAME_RE = re.compile(r'[a-z0-9]{1,32}')
IS_WIN = os.name == 'nt'


# ---------------------------------------------------------------- addresses
def _is_ip(s):
    try:
        socket.inet_pton(socket.AF_INET6 if ':' in s else socket.AF_INET, s)
        return True
    except (OSError, ValueError):
        return False


def _run(argv, timeout=4):
    try:
        return subprocess.run(argv, capture_output=True, text=True, timeout=timeout).stdout
    except (OSError, subprocess.SubprocessError):
        return ''


def lan_ip():
    """The address this machine uses on its network (no packet is sent: connect only picks a route)."""
    try:
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as u:
            u.connect(('10.255.255.255', 1))  # leakscan: allow
            ip = u.getsockname()[0]
            return None if ip.startswith('127.') else ip
    except OSError:
        return None


def tailscale():
    """(ips, MagicDNS name) when Tailscale is running here."""
    try:
        d = json.loads(_run(['tailscale', 'status', '--json']) or '{}')
    except ValueError:
        return [], None
    me = d.get('Self') or {}
    ips = [x for x in me.get('TailscaleIPs') or [] if isinstance(x, str) and _is_ip(x) and '.' in x]
    dns = (me.get('DNSName') or '').rstrip('.') or None
    return ips, dns


def candidates(lan=lan_ip, ts=tailscale, hostname=socket.gethostname, mac=sys.platform == 'darwin'):
    """Addresses other devices could use for this server, best first, each with why: [(address, note)]."""
    out = []
    add = lambda a, note: a and a not in [x for x, _ in out] and out.append((a, note))
    ip = lan()
    add(ip, 'this machine on your network')
    ips, dns = ts()
    add(dns, 'Tailscale name (works away from home)')
    for x in ips:
        add(x, 'Tailscale address')
    host = (hostname() or '').split('.')[0].lower()
    if host and host != 'localhost':
        add(f'{host}.local' if mac else host, 'this machine\'s name')
    return out


def server_json(address, others, old=None, port=PORT):
    """server.json for the chosen address: it comes first (what the apps and the machines are told), the others are
    accepted too. Settings the person wrote themselves ("allow", extra hosts) are kept."""
    old = old if isinstance(old, dict) else {}
    hosts = [address] + [h for h in others if h != address]
    hosts += [h for h in old.get('hosts') or [] if isinstance(h, str) and h not in hosts]
    url = lambda h: f'http://[{h}]:{port}' if ':' in h else f'http://{h}:{port}'
    origins = [url(h) for h in hosts]
    origins += [o for o in old.get('origins') or [] if isinstance(o, str) and o not in origins]
    d = {'bind': old.get('bind') or ['0.0.0.0'], 'hosts': hosts, 'origins': origins}
    if isinstance(old.get('allow'), list):
        d['allow'] = old['allow']
    return d


def valid_address(a):
    a = (a or '').strip()
    return bool(a) and len(a) <= 253 and (_is_ip(a) or re.fullmatch(r'[A-Za-z0-9]([A-Za-z0-9.-]*[A-Za-z0-9])?', a) is not None)


# ---------------------------------------------------------------- the server's API
class Api:
    def __init__(self, base=LOCAL, host=None):
        self.base, self.host = base.rstrip('/'), host

    def call(self, method, path, body=None, timeout=15):
        h = {'X-Switchboard': '1', 'Content-Type': 'application/json'}
        if self.host:  # the address devices use: pairing answers with it
            h['Host'] = f'[{self.host}]:{PORT}' if ':' in self.host else f'{self.host}:{PORT}'
        data = None if body is None else json.dumps(body).encode()
        req = urllib.request.Request(self.base + path, data=data, method=method, headers=h)
        try:
            with urllib.request.urlopen(req, timeout=timeout) as r:
                return json.loads(r.read() or b'{}')
        except urllib.error.HTTPError as e:
            try:
                msg = json.loads(e.read() or b'{}').get('error')
            except ValueError:
                msg = None
            raise RuntimeError(msg or f'the server answered {e.code}')
        except (urllib.error.URLError, OSError) as e:
            raise RuntimeError(f'the server on this machine is not answering ({getattr(e, "reason", e)})')

    def machines(self):
        return self.call('GET', '/api/machines').get('machines') or []

    def wait_up(self, seconds=30):
        end = time.time() + seconds
        while True:
            try:
                return self.call('GET', '/api/summary', timeout=3)
            except RuntimeError:
                if time.time() > end:
                    raise
                time.sleep(1)


def restart_server():
    """Restart the server so it reads server.json again."""
    if IS_WIN:
        ctl = Path(os.environ.get('LOCALAPPDATA') or Path.home() / 'AppData' / 'Local') / 'Switchboard' / 'switchboard.ps1'
        _run(['powershell', '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', str(ctl), 'restart'], timeout=60)
        return
    ctl = shutil.which('switchboard') or str(Path.home() / '.local' / 'bin' / 'switchboard')
    _run([ctl, 'restart'], timeout=60)


# ---------------------------------------------------------------- talking to the person
class Ask:
    def __init__(self, yes=False, inp=input, out=print):
        self.yes, self.inp, self.out = yes, inp, out

    def line(self, prompt, default=''):
        if self.yes:
            return default
        try:
            v = self.inp(f'{prompt}' + (f' [{default}]' if default else '') + ': ').strip()
        except EOFError:
            return default
        return v or default

    def choose(self, prompt, options, default=1):
        """options: [label]; returns the index picked (0-based)."""
        for i, o in enumerate(options, 1):
            self.out(f'  {i}. {o}')
        while True:
            v = self.line(prompt, str(default))
            if v.isdigit() and 1 <= int(v) <= len(options):
                return int(v) - 1
            self.out(f'  Type a number from 1 to {len(options)}.')

    def yes_no(self, prompt, default=True):
        v = self.line(prompt + (' (Y/n)' if default else ' (y/N)'), 'y' if default else 'n').lower()
        return v.startswith('y')


# ---------------------------------------------------------------- steps
def step_address(api, ask, given=None):
    ask.out('\nStep 1 of 3: the server\'s address')
    ask.out('Your phone, your browser and your other machines reach Switchboard at this address.')
    found = candidates()
    try:  # the address picked last time comes first
        before = json.loads((CFG / 'setup.json').read_text()).get('address')
    except (OSError, ValueError, AttributeError):
        before = None
    if before:
        found = [(before, 'what you picked before')] + [(a, n) for a, n in found if a != before]
    if given:
        address = given
    elif not found:
        address = ask.line('Type this machine\'s address (an IP or a name)')
    else:
        i = ask.choose('Which one should they use?',
                       [f'{a}  ({note})' for a, note in found] + ['Another address'])
        address = found[i][0] if i < len(found) else ask.line('Address')
    while not valid_address(address):
        if ask.yes:
            raise SystemExit(f'setup: "{address}" is not an address')
        address = ask.line('That is not an address. Type an IP or a name, like 10.0.0.5 or studio.local')
    path = CFG / 'server.json'
    try:
        old = json.loads(path.read_text())
    except (OSError, ValueError):
        old = None
    new = server_json(address, [a for a, _ in found], old)
    if new != old:
        CFG.mkdir(parents=True, exist_ok=True)
        tmp = path.with_suffix('.tmp')
        tmp.write_text(json.dumps(new, indent=1))
        os.replace(tmp, path)
        ask.out('Saved. Restarting the server so it answers there...')
        restart_server()
        api.wait_up(45)
    ask.out(f'Open Switchboard at http://{address}:{PORT} (in the iPhone app: Settings, Server).')
    (CFG / 'setup.json').write_text(json.dumps({'address': address}, indent=1))
    return address


def hub_up():
    try:
        urllib.request.urlopen(f'http://127.0.0.1:{HUB_PORT}/healthz', timeout=3).close()
        return True
    except OSError:
        return False


def link_lines(address, code):
    """What to paste on the new machine. The install script comes from the server's downloads, so nothing here
    depends on the internet."""
    dl = f'http://{address}:{DOWNLOADS_PORT}'
    unix = f'curl -fsSL {dl}/install-agent.sh | bash -s -- --server {address} --code {code}'
    unix_nocurl = (f'python3 -c "import urllib.request,sys; sys.stdout.write(urllib.request.urlopen(\'{dl}/install-agent.sh\')'
                   f'.read().decode())" | bash -s -- --server {address} --code {code}')
    win = f'& ([scriptblock]::Create((iwr -UseBasicParsing {dl}/install-agent.ps1).Content)) -Server {address} -Code {code}'
    return {'mac': unix, 'linux': unix, 'linux (no curl)': unix_nocurl, 'windows': win}


def wait_new(api, before, ask, minutes=10):
    """Wait for machines that weren't there before (two linked at once both count); their names."""
    ask.out('Waiting for it to link (this window keeps waiting while you paste the line; Ctrl+C stops)...')
    end = time.time() + minutes * 60
    try:
        while time.time() < end:
            new = [m for m in api.machines() if m['name'] not in before]
            for m in new:
                ask.out(f'Linked: {m["label"]} ({m["os"] or "its system is not known yet"}).')
            if new:
                return [m['name'] for m in new]
            time.sleep(2)
    except KeyboardInterrupt:
        ask.out('')
    ask.out('It hasn\'t linked yet. When it has, it shows up in Settings, Machines.')
    return []


def link_code(api, address, ask):
    """Add a machine with a code (the hub links it; no ssh)."""
    r = api.call('POST', '/api/pair', {})
    lines = link_lines(address, r['code'])
    ask.out(f'\nCode {r["code"]} works once, for 10 minutes. On the new machine, paste the line for its system:')
    for k, v in lines.items():
        ask.out(f'\n  {k}:\n    {v}')
    ask.out('')
    return r['code'], lines


def ssh_key():
    """The server's key for machines reached over ssh (made once)."""
    pub = KEY.with_name(KEY.name + '.pub')
    if not pub.exists():
        KEY.parent.mkdir(mode=0o700, exist_ok=True)
        r = subprocess.run(['ssh-keygen', '-q', '-t', 'ed25519', '-N', '', '-C', f'switchboard@{socket.gethostname()}', '-f', str(KEY)],
                           capture_output=True, text=True)
        if r.returncode != 0:
            raise RuntimeError(f'could not make an ssh key: {r.stderr.strip()}')
    return pub.read_text().strip()


def ssh_lines(address, pub):
    dl = f'http://{address}:{DOWNLOADS_PORT}'
    return {'mac': f'curl -fsSL {dl}/install-agent.sh | bash -s -- --server {address} --ssh-key "{pub}"',
            'linux': f'curl -fsSL {dl}/install-agent.sh | bash -s -- --server {address} --ssh-key "{pub}"',
            'windows': f'& ([scriptblock]::Create((iwr -UseBasicParsing {dl}/install-agent.ps1).Content)) -Server {address} -SshKey "{pub}"'}


def known_host(host):
    """Trust the machine's ssh host key the first time (ssh in the background can't ask)."""
    h = host.split('@')[-1]
    if subprocess.run(['ssh-keygen', '-F', h], capture_output=True).returncode == 0:
        return
    keys = _run(['ssh-keyscan', '-T', '5', h], timeout=15)
    if keys.strip():
        kh = Path.home() / '.ssh' / 'known_hosts'
        with kh.open('a') as f:
            f.write(keys if keys.endswith('\n') else keys + '\n')


def add_ssh(api, name, target, label=None, os_=''):
    known_host(target)
    api.call('POST', '/api/machines', {'op': 'add', 'name': name, 'label': label or name, 'target': target, 'os': os_})
    return name


def pick_folder(api, name, ask):
    """Browse the machine's folders one level at a time; returns the folder for new sessions (~/... form)."""
    # Folders come relative to the machine's current folder: the one set before, else its home folder.
    root = next((m.get('root') for m in api.machines() if m['name'] == name), None) or '~'
    try:
        api.call('GET', f'/api/folders/{name}?parent=', timeout=30)
    except RuntimeError as e:
        ask.out(f'Could not look at its folders ({e}).')
        return ask.line('Type its Claude folder', root)
    try:
        desk = api.call('GET', f'/api/folders/{name}?parent=Desktop', timeout=30).get('folders') or []
    except RuntimeError:
        desk = []
    default = 'Desktop/Claude' if any(f.get('name') == 'Claude' for f in desk) else ''
    if ask.yes:
        return _join(root, default)
    here = ''
    while True:
        try:
            kids = api.call('GET', f'/api/folders/{name}?parent={urllib.request.quote(here)}', timeout=30).get('folders') or []
        except RuntimeError as e:
            ask.out(f'  {e}')
            kids = []
        ask.out(f'\n  {_join(root, here)}')
        for i, k in enumerate(kids[:40], 1):
            ask.out(f'    {i}. {k["name"]}' + ('/' if k.get('kids') else ''))
        hint = 'Enter picks this folder, a number opens one, .. goes up'
        v = ask.line(f'  {hint}', '' if here or not default else default)
        if v == '':
            return _join(root, here)
        if v == '..':
            here = here.rsplit('/', 1)[0] if '/' in here else ''
        elif v.isdigit() and 1 <= int(v) <= len(kids[:40]):
            here = kids[int(v) - 1]['folder']
        elif v == default:
            return _join(root, default)
        else:
            ask.out('  Type a number, .. or press Enter.')


def _join(root, rel):
    root = root.rstrip('/\\') or '~'
    return f'{root}/{rel}' if rel else root


def full_folder(f):
    """A folder as the machine reads it: ~/..., /..., or C:\\...; anything else is taken to be in the home folder
    (the agent would read it from wherever it happens to run)."""
    f = (f or '').strip()
    home = str(Path.home())
    if f == home or f.startswith(home + '/'):  # an unquoted ~ that this machine's shell already expanded
        f = '~' + f[len(home):]
    if not f or f == '~' or f.startswith(('~/', '~\\', '/', '\\')) or re.match(r'[A-Za-z]:[\\/]', f):
        return f or '~'
    return '~/' + f.lstrip('./')


def step_folder(api, name, ask, given=None):
    label = next((m['label'] for m in api.machines() if m['name'] == name), name)
    ask.out(f'\nThe Claude folder on {label}: where new sessions start and folders are picked from.')
    folder = full_folder(given or pick_folder(api, name, ask))
    api.call('POST', '/api/machines', {'op': 'root', 'machine': name, 'root': folder})
    ask.out(f'{label}: new sessions start in {folder}.')


def step_machines(api, address, ask):
    ask.out('\nStep 2 of 3: your machines')
    ask.out('Each machine that runs Claude Code gets a small agent. A machine can link with a code (nothing to open')
    ask.out('on it) or be reached over ssh.')
    added = []
    while True:
        have = api.machines()
        if have:
            ask.out('Machines now: ' + ', '.join(f'{m["label"]} ({"online" if m.get("online") else "not answering yet"})' for m in have))
        opts = ['Add a machine with a code', 'Add a machine over ssh', 'Done']
        if not hub_up():
            opts[0] = 'Add a machine with a code (needs the hub, which is not running)'
        i = ask.choose('Pick one', opts, default=3 if have else 1)
        if i == 2:
            return added
        before = {m['name'] for m in have}
        try:
            if i == 0:
                link_code(api, address, ask)
                names = wait_new(api, before, ask)
            else:
                pub = ssh_key()
                ask.out('\nOn that machine, paste the line for its system. It installs the agent and lets this server in:')
                for k, v in ssh_lines(address, pub).items():
                    ask.out(f'\n  {k}:\n    {v}')
                target = ask.line('\nThen type how this server reaches it, as after ssh (user@host)')
                default = re.sub(r'[^a-z0-9]', '', target.split('@')[-1].split('.')[0].lower())[:32] or 'machine'
                name = ask.line('A short id for it (lowercase letters and digits)', default)
                label = ask.line('Its name in Switchboard', name.capitalize())
                add_ssh(api, name, target, label)
                ask.out(f'Added {label}.')
                names = [name]
        except RuntimeError as e:
            ask.out(f'That did not work: {e}')
            continue
        added += names


def main(argv=None):
    p = argparse.ArgumentParser(description='First-run setup for a Switchboard server.')
    p.add_argument('--address', help='the address devices use for this server')
    p.add_argument('--code', action='store_true', help='print a code and the lines that link a machine, then stop')
    p.add_argument('--ssh', action='append', default=[], metavar='ID=USER@HOST', help='add a machine reached over ssh')
    p.add_argument('--root', action='append', default=[], metavar='ID=FOLDER', help="set a machine's Claude folder")
    p.add_argument('--ssh-key', action='store_true', help="print this server's ssh public key (made the first time)")
    p.add_argument('--yes', '-y', action='store_true', help='take the suggested answer everywhere')
    p.add_argument('--json', action='store_true', help='with --code: print JSON')
    p.add_argument('--server', default=LOCAL, help=argparse.SUPPRESS)
    a = p.parse_args(argv)
    ask = Ask(yes=a.yes or bool(a.ssh or a.root or a.code) or not sys.stdin.isatty())
    api = Api(a.server)
    try:
        api.wait_up(20)
    except RuntimeError as e:
        print(f'setup: {e}. Start it with: switchboard start', file=sys.stderr)
        return 1
    try:
        saved = json.loads((CFG / 'setup.json').read_text()).get('address')
    except (OSError, ValueError, AttributeError):
        saved = None
    if a.ssh_key:
        print(ssh_key())
        return 0
    if a.code:
        address = a.address or saved or (candidates() or [(lan_ip() or '127.0.0.1', '')])[0][0]
        api.host = address
        r = api.call('POST', '/api/pair', {})
        out = {'code': r['code'], 'expires': r['expires'], 'hub': f'{address}:{HUB_PORT}', 'lines': link_lines(address, r['code'])}
        if a.json:
            print(json.dumps(out, indent=1))
        else:
            link_code(api, address, Ask(yes=True))
        return 0
    if a.ssh or a.root:
        for spec in a.ssh:
            name, _, target = spec.partition('=')
            if not NAME_RE.fullmatch(name) or not target:
                print(f'setup: --ssh takes id=user@host, got {spec}', file=sys.stderr)
                return 2
            add_ssh(api, name, target, name.capitalize())
            print(f'Added {name} ({target}).')
        if a.address:
            step_address(api, ask, a.address)
        for spec in a.root:
            name, _, folder = spec.partition('=')
            step_folder(api, name, ask, folder or None)
        return 0
    print('Switchboard setup. Press Enter to take the suggested answer in [brackets].')
    address = step_address(api, ask, a.address)
    api.host = address
    if ask.yes:
        print('\nStep 2 of 3: your machines (skipped; add them with: switchboard setup)')
        added = []
    else:
        added = step_machines(api, address, ask)
    print('\nStep 3 of 3: Claude folders')
    have = api.machines()
    todo = [m for m in have if m['name'] in added or not m.get('root')]
    if not have:
        print('No machines yet. Add them any time with: switchboard setup')
    elif not todo:
        print('Every machine has its Claude folder already.')
    for m in todo:
        if not m.get('online'):
            print(f'{m["label"]} is not answering yet; set its folder later in Settings, Machines.')
            continue
        try:
            step_folder(api, m['name'], ask)
        except RuntimeError as e:
            print(f'{m["label"]}: {e}')
    print(f'\nAll set. Open http://{address}:{PORT} in a browser, or put {address} in the app\'s Settings.')
    return 0


if __name__ == '__main__':
    try:
        sys.exit(main())
    except KeyboardInterrupt:
        print('\nStopped. Run it again any time: switchboard setup')
        sys.exit(130)
