"""Terminals (spec section 1): tickets for the hub (sb-hub, :8132), the machine list it reads, the origins it lets in.

A client asks POST /api/term for a chat; the answer is a WebSocket URL on the hub with a ticket in it. The ticket is
base64url(json {m, b, x, n}) + "." + base64url(HMAC-SHA256(key, first part)), good for 60 s and one connection.
The key is 32 random bytes in ~/.config/switchboard/term.key (mode 600), made by whichever of server or hub starts
first. The hub reads machines (name -> ssh target) and extra web origins from machines.json, written here at start.
"""
import base64, hashlib, hmac, json, os, re, secrets, time
from pathlib import Path

HUB_PORT = int(os.environ.get('SWITCHBOARD_HUB_PORT') or 8132)
TTL = 60
KEY_LEN = 32
BG_RE = re.compile(r'[0-9a-f]{8,64}')
# The server's own pages come from its config (server.py ServerConfig); SWITCHBOARD_ORIGINS adds more.
GONE = ('stopped', 'done')


class BadRequest(ValueError):
    pass


class NotFound(ValueError):
    pass


def default_key_path():
    return Path.home() / '.config' / 'switchboard' / 'term.key'


def load_or_create_key(path):
    """The shared key. Made from a finished temp file with a hard link, so a hub starting at the same moment can never
    read half a key, and whichever starts second reads the first one's."""
    path = Path(path)
    try:
        k = path.read_bytes()
    except FileNotFoundError:
        k = None
    if k is not None:
        if len(k) != KEY_LEN:
            raise OSError(f'{path} holds {len(k)} bytes, not {KEY_LEN}')
        try:
            if os.name != 'nt' and path.stat().st_mode & 0o077:
                os.chmod(path, 0o600)  # someone loosened it: anyone who reads it can mint terminal tickets
        except OSError:
            pass
        return k
    path.parent.mkdir(parents=True, exist_ok=True)
    try:
        os.chmod(path.parent, 0o700)
    except OSError:
        pass
    key = secrets.token_bytes(KEY_LEN)
    tmp = path.with_name(f'.term.key.{os.getpid()}.tmp')
    fd = os.open(tmp, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    try:
        os.write(fd, key)
        os.fsync(fd)
    finally:
        os.close(fd)
    try:
        os.link(tmp, path)
    except FileExistsError:
        return load_or_create_key(path)
    finally:
        tmp.unlink(missing_ok=True)
    return key


def _b64(b):
    return base64.urlsafe_b64encode(b).rstrip(b'=').decode()


def mint(key, machine, bg, now, ttl=TTL, nonce=None):
    claims = {'m': machine, 'b': bg, 'x': int(now) + ttl, 'n': nonce or secrets.token_hex(16)}
    body = _b64(json.dumps(claims).encode())
    return body + '.' + _b64(hmac.new(key, body.encode(), hashlib.sha256).digest())


def extra_origins(env=None):
    raw = (os.environ if env is None else env).get('SWITCHBOARD_ORIGINS') or ''
    return [o.strip().rstrip('/').lower() for o in re.split(r'[\s,]+', raw) if o.strip()]


def write_machines(path, machines, origins):
    """{"machines": [{name, label, target, os}], "origins": [...]}, replaced in one step."""
    path = Path(path)
    data = {'machines': [{k: m.get(k) for k in ('name', 'label', 'target', 'os')} for m in machines],
            'origins': list(dict.fromkeys(origins)), 'written': int(time.time())}
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp = path.with_name(f'{path.name}.{os.getpid()}.tmp')
    with os.fdopen(os.open(tmp, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600), 'w') as f:  # the hub trusts its origins
        f.write(json.dumps(data, indent=1))
    if os.name != 'nt':
        os.chmod(tmp, 0o600)  # also when an older run left the temp file behind
    os.replace(tmp, path)


def ws_host(host_header):
    """The host the client used to reach the server, without the port, ready for a URL."""
    h = (host_header or '').strip()
    if not re.fullmatch(r'[A-Za-z0-9.:\[\]-]{1,255}', h):
        h = ''  # it goes into URLs and pasted command lines: nothing but a host name, an address and a port
    if h.startswith('['):
        return h[:h.index(']') + 1] if ']' in h else h
    if h.count(':') == 1:
        h = h.rsplit(':', 1)[0]
    return h or '127.0.0.1'


class Terms:
    def __init__(self, fleet, state_dir, key_path=None, clock=time.time, hub_port=None, env=None, origins=()):
        self.fleet, self.state_dir, self.clock = fleet, Path(state_dir), clock
        self.key_path = Path(key_path) if key_path else default_key_path()
        self.hub_port = hub_port or HUB_PORT
        self.origins = [o.rstrip('/').lower() for o in origins] + extra_origins(env)
        self._key = None

    def write_machines(self):
        """At start and after every change in Settings: the hub resolves machine names to ssh targets from this file."""
        machines = self.fleet.config()
        write_machines(self.state_dir / 'machines.json', machines, self.origins)

    def key(self):
        if self._key is None:
            self._key = load_or_create_key(self.key_path)
        return self._key

    def ticket(self, body, host, sid_re):
        """POST /api/term: {"machine", "sid"} or {"machine", "bg"} -> a WebSocket URL, or why it can't open."""
        name = body.get('machine')
        if not isinstance(name, str) or name not in self.fleet.machines:
            raise BadRequest('unknown machine')
        snap = next(m for m in self.fleet.snapshot()['machines'] if m['name'] == name)
        label = snap.get('label') or name
        chats = list(snap.get('sessions') or []) + list(snap.get('replaced') or [])
        bg, sid = body.get('bg'), body.get('sid')
        if bg is not None:
            if not isinstance(bg, str) or not BG_RE.fullmatch(bg):
                raise BadRequest('bad background id')
        elif not isinstance(sid, str) or not sid_re.fullmatch(sid):
            raise BadRequest('bad session id')
        s = next((c for c in chats if (c.get('bg_id') == bg if bg is not None else c.get('sid') == sid)), None)
        # A need says which chat it is about too, so a page opened from a link can name it.
        about = {'machine': name, 'label': label, 'title': (s or {}).get('title'), 'sid': (s or {}).get('sid') or sid}
        if snap.get('online') is False:
            return self._need('offline', f"{label} is offline, so its terminal can't open. Try again once {label} is back.", about)
        if s is None and bg is None:
            raise NotFound('That chat is not on the list any more.')
        if bg is None:
            kind, status = s.get('kind'), s.get('status')
            if kind == 'background' and s.get('bg_id') and status not in GONE:
                bg = s['bg_id']
            elif kind == 'interactive' and status not in GONE:
                return self._need('terminal', f"This chat is open in a terminal on {label}, so it can't be opened here.", about)
            else:
                return self._need('resume', 'This chat is not running. Resume it to open its terminal.', about)
        if not BG_RE.fullmatch(bg or ''):
            return self._need('resume', 'This chat is not running. Resume it to open its terminal.', about)
        t = mint(self.key(), name, bg, self.clock())
        return {'ok': True, 'ws': f'ws://{ws_host(host)}:{self.hub_port}/term?t={t}', 'bg': bg,
                'title': (s or {}).get('title'), 'machine': name, 'label': label}

    @staticmethod
    def _need(need, error, about):
        return {'ok': False, 'error': error, 'need': need, **about}
