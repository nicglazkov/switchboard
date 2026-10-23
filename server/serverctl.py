"""The server looking after itself, for the app's Settings > Server: what runs and how well, its logs, a restart, and
an update to a newer release.

An update installs the newest release's server kit the way the install line does (get-server.sh): download it,
check it, unpack it, run its install-server.sh. The installer stops and starts this server, so the work runs in
selfupdate.py, outside it (a systemd transient unit on Linux, its own session elsewhere), and writes how far it got
to <state>/update/state.json, which this server reads back once it is running again.

The kit's SHA-256 comes from the app, which read it from the release's signed latest.json and checked the signature
with the key compiled into it: this server only downloads the file the app named, from the release the app named,
and installs it only if it matches.
"""
import json, os, platform, re, shutil, socket, subprocess, sys, time, urllib.request
from pathlib import Path

import releases as releases_mod

KIT = 'switchboard-server-kit.tar.gz'
VERSION_RE = re.compile(r'\d{1,4}\.\d{1,4}\.\d{1,4}')
SHA_RE = re.compile(r'[0-9a-f]{64}')
PARTS = ('server', 'hub', 'downloads')
LOG_LINES = 400
STARTED = time.time()
# An update that has said nothing for this long has died (the machine slept or restarted mid-way).
STALE_AFTER = 15 * 60

HOME = Path.home()
SHARE = HOME / '.local' / 'share' / 'switchboard'
APP = SHARE / 'app'
BIN = HOME / '.local' / 'bin'
HERE = Path(__file__).resolve().parent


def version():
    """("0.8.11", "abc1234") from the VERSION file the installer leaves next to the server, or the repository's
    Cargo.toml when it runs from a checkout. ("", "") when neither says."""
    for f in (HERE.parent / 'VERSION',):
        try:
            text = f.read_text().strip().splitlines()[0]
        except (OSError, IndexError):
            continue
        m = re.match(r'(\d+\.\d+\.\d+)(?:\s*\(([0-9a-f]{4,40})\))?', text)
        if m:
            return m.group(1), m.group(2) or ''
    try:
        toml = (HERE.parent / 'native' / 'Cargo.toml').read_text()
        m = re.search(r'^version = "(\d+\.\d+\.\d+)"', toml, re.M)
        if m:
            return m.group(1), ''
    except OSError:
        pass
    return '', ''


def vkey(v):
    return tuple(int(x) for x in v.split('.')) if v and VERSION_RE.fullmatch(v) else ()


def mode():
    """How the parts run: "systemd", "launchd", "plain", "windows", or "checkout" (python3 server/server.py by hand
    from a copy of the repository: nothing to restart or update from here)."""
    if sys.platform == 'win32':
        return 'windows'
    if HERE != (APP / 'server').resolve():
        return 'checkout'
    if sys.platform == 'darwin' and (HOME / 'Library' / 'LaunchAgents' / 'com.switchboard.server.plist').is_file():
        return 'launchd'
    if (HOME / '.config' / 'systemd' / 'user' / 'switchboard.service').is_file() and shutil.which('systemctl'):
        return 'systemd'
    return 'plain'


def why_not(m=None):
    """Why this server can't restart or update itself from the app, or None when it can."""
    m = m or mode()
    if m == 'windows':
        return 'On Windows, run the server installer again to update it.'
    if m == 'checkout':
        return 'This server runs from a copy of the code, so update it there (git pull) and start it again.'
    if not (BIN / 'switchboard').is_file():
        return "The switchboard command isn't in ~/.local/bin, so run the install line on the server once."
    return None


def _probe(url, timeout=1.5):
    try:
        with urllib.request.urlopen(url, timeout=timeout) as r:
            return r.read(4000)
    except Exception:
        return None


def hub():
    """{installed, running, version, open, linked} from the hub's /healthz on this machine."""
    installed = (BIN / 'switchboard-hub').is_file() or mode() == 'checkout'
    raw = _probe('http://127.0.0.1:8132/healthz')
    try:
        h = json.loads(raw) if raw else {}
    except ValueError:
        h = {}
    return {'installed': installed, 'running': bool(h.get('ok')), 'version': h.get('version') or '',
            'open': h.get('open') or 0, 'linked': h.get('linked') or 0}


def downloads():
    return {'running': _probe('http://127.0.0.1:8131/node/SHA256SUMS') is not None}


def disk(path):
    try:
        u = shutil.disk_usage(path)
        return {'free': u.free, 'total': u.total}
    except OSError:
        return None


def update_dir(state):
    return Path(state) / 'update'


def update_state(state, now=None):
    """The last update's progress (selfupdate.py): {step, version, from, started, at, ok, error, ...}, or None."""
    try:
        s = json.loads((update_dir(state) / 'state.json').read_text())
    except (OSError, ValueError):
        return None
    if not isinstance(s, dict):
        return None
    now = now or time.time()
    if s.get('ok') is None and now - float(s.get('at') or 0) > STALE_AFTER:
        s = dict(s, ok=False, error="The update stopped part way (the server may have restarted). Try again.")
    return s


def info(state, latest=None, now=None):
    """GET /api/server."""
    v, commit = version()
    m = mode()
    up = update_state(state, now)
    out = {
        'ok': True, 'version': v, 'commit': commit, 'mode': m, 'started': STARTED,
        'host': socket.gethostname(), 'os': _os_name(), 'python': platform.python_version(),
        'hub': hub(), 'downloads': downloads(), 'disk': disk(state),
        'can_manage': why_not(m) is None, 'why_not': why_not(m), 'update': up,
        'install_line': f'curl -fsSL https://github.com/{releases_mod.repo() or releases_mod.REPO}'
                        f'/releases/latest/download/get-server.sh | bash',
    }
    if latest:
        out['latest'] = latest
    return out


def _os_name():
    if sys.platform == 'darwin':
        return 'macOS ' + (platform.mac_ver()[0] or '')
    if sys.platform.startswith('linux'):
        try:
            for line in Path('/etc/os-release').read_text().splitlines():
                if line.startswith('PRETTY_NAME='):
                    return line.split('=', 1)[1].strip().strip('"')
        except OSError:
            pass
        return 'Linux'
    return platform.system()


def logs(part, lines=200):
    """The newest `lines` lines of one part's log, as text."""
    if part not in PARTS:
        raise ValueError('no such part')
    lines = max(10, min(int(lines), LOG_LINES))
    m = mode()
    if m == 'systemd':
        unit = 'switchboard' if part == 'server' else f'switchboard-{part}'
        try:
            r = subprocess.run(['journalctl', '--user', '-u', unit, '-n', str(lines), '--no-pager', '-o', 'short-iso'],
                               capture_output=True, text=True, timeout=10)
            return r.stdout or r.stderr
        except (OSError, subprocess.SubprocessError) as e:
            return f"Couldn't read the log: {e}"
    folder = HOME / 'Library' / 'Logs' / 'Switchboard' if m == 'launchd' else SHARE / 'logs'
    f = folder / f'{part}.log'
    try:
        with open(f, 'rb') as fh:
            fh.seek(0, 2)
            size = fh.tell()
            fh.seek(max(0, size - 400_000))
            text = fh.read().decode('utf-8', 'replace')
    except OSError:
        return f'There is no log at {f} yet.'
    return '\n'.join(text.splitlines()[-lines:])


def detach(argv, name, log_to=None):
    """Runs argv apart from this server, so stopping the server's service does not stop it too."""
    env = dict(os.environ, PATH=f"{BIN}:{os.environ.get('PATH') or '/usr/bin:/bin'}")
    if mode() == 'systemd' and shutil.which('systemd-run'):
        unit = f'{name}-{int(time.time())}'
        r = subprocess.run(['systemd-run', '--user', '--collect', '--quiet', f'--unit={unit}',
                            f'--setenv=PATH={env["PATH"]}', '--', *argv], capture_output=True, text=True, timeout=20)
        if r.returncode == 0:
            return
    out = open(log_to, 'ab') if log_to else subprocess.DEVNULL
    try:
        subprocess.Popen(argv, stdin=subprocess.DEVNULL, stdout=out, stderr=subprocess.STDOUT, env=env,
                         cwd=str(HOME), start_new_session=True, close_fds=True)
    finally:
        if log_to:
            out.close()


def restart():
    """POST /api/server/restart: every part stops and starts again a moment from now."""
    why = why_not()
    if why:
        raise PermissionError(why)
    # A beat first, so this answer reaches the app before the server goes.
    detach(['/bin/sh', '-c', f'sleep 1; exec "{BIN / "switchboard"}" restart'], 'switchboard-restart')
    return {'ok': True}


def start_update(state, want, sha, busy_ok=False, now=None):
    """POST /api/server/update {version, sha256}: installs that release's server kit if its SHA-256 matches."""
    why = why_not()
    if why:
        raise PermissionError(why)
    if not (isinstance(want, str) and VERSION_RE.fullmatch(want)):
        raise ValueError('That is not a version.')
    if not (isinstance(sha, str) and SHA_RE.fullmatch(sha)):
        raise ValueError("That is not the kit's SHA-256.")
    r = releases_mod.repo()
    if not r:
        raise PermissionError('Updates from GitHub are turned off on this server (SWITCHBOARD_RELEASES=off).')
    cur, _ = version()
    if cur and vkey(want) < vkey(cur):
        raise ValueError(f'This server already has {cur}.')
    now = now or time.time()
    s = update_state(state, now)
    if s and s.get('ok') is None and not busy_ok:
        raise RuntimeError('An update is already running.')
    d = update_dir(state)
    d.mkdir(parents=True, exist_ok=True)
    first = {'step': 'starting', 'version': want, 'from': cur, 'started': now, 'at': now, 'ok': None, 'error': None}
    (d / 'state.json').write_text(json.dumps(first))
    # The updater runs from a copy: the installer replaces the folder this one is in.
    tool = d / 'selfupdate.py'
    shutil.copyfile(HERE / 'selfupdate.py', tool)
    argv = [sys.executable or 'python3', '-I', str(tool), '--version', want, '--sha256', sha, '--repo', r,
            '--dir', str(d), '--url', releases_mod.asset(r, want, KIT)]
    if not (BIN / 'switchboard-hub').is_file():
        argv.append('--no-hub')
    detach(argv, 'switchboard-update', log_to=d / 'update.log')
    return {'ok': True, 'update': first}
