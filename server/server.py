"""Switchboard: every Claude session on every machine, in one page. LAN + Tailscale only, no login."""
import base64, ipaddress, json, os, re, socket, subprocess, sys, threading, time, traceback, uuid
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import parse_qs, urlsplit
from urllib.request import urlopen

sys.path.insert(0, str(Path(__file__).resolve().parent))
import fleet as fleet_mod  # noqa: E402
import term as term_mod  # noqa: E402
import nodes as nodes_mod  # noqa: E402
import features as features_mod  # noqa: E402
import flags as flags_mod  # noqa: E402
import notes as notes_mod  # noqa: E402
import options as options_mod  # noqa: E402
from guard import GuardTimer  # noqa: E402

PORT = 8130
LOCAL_HOSTS = ('localhost', '127.0.0.1')
ACTION_VERBS = {'resume', 'fresh', 'handoff', 'stop', 'dismiss', 'pass'}
MODELS = {'opus', 'sonnet', 'haiku', 'fable', None}
MODES = {'normal', 'max', 'ultracode'}
EFFORTS = {'low', 'medium', 'high', 'xhigh', 'max', None}
PING_WHY = {'prompt', 'stop', 'wait', 'start', 'end', 'status'}
OVERLAY_FOR = 45      # a ping's guess at a session (busy, idle, its context) holds until the machine's next answer
STREAM_FOR, STREAM_MAX, STREAM_BEAT = 600, 64, 15  # live pages: reconnect every 10 min, at most 64, a beat every 15 s
WIDGET_EVERY = 60     # iPhone widget pushes: at most one a minute (Apple budgets them), the last change always goes out
PRESETS_MAX = 12
DEVICES_MAX = 16    # push tokens kept (each phone or iPad, and its reinstalls); the oldest go first
NAME_RE = re.compile(r"[A-Za-z0-9 ._,:'+#()-]{1,64}")
JOB_KEEP, WATCH_FOR, WATCH_GRACE = 3600, 1800, 6
COMPACT_MAX = 40  # the newest chats of a thread that compacting reads
DONE_TITLES = {'fresh': 'Fresh session ready', 'start': 'New session ready', 'resume': 'Session resumed',
               'continue': 'Continued on {where}', 'handoff': 'Handoff saved', 'relaunch': 'Relaunched with new flags',
               'stop': 'Stopped once its work finished', 'compact': 'Thread compacted into a fresh chat'}
FAIL_TITLES = {'fresh': 'Start fresh did not finish', 'start': 'New session did not start', 'resume': 'Resume did not finish',
               'continue': 'Continue did not finish', 'handoff': 'Handoff did not finish', 'relaunch': 'Relaunch did not finish',
               'stop': 'Stop did not finish', 'compact': 'Compacting the thread did not finish'}
RELAUNCH_WAIT = 15 * 60  # a relaunch that waits for Claude to finish its reply gives up after this
CHILD_WAIT = 4 * 3600  # ... and one that waits for subagents or background commands, after this
BUSY_THEN = {'fresh': 'Start fresh when they finish, or now, which stops them.',
             'stop': 'Stop it when they finish, or now, which cuts them off.',
             'handoff': 'Close it when they finish, or now, which stops them.',
             'continue': 'Move it when they finish, or now, which leaves their results behind.'}
BUSY_REPLY = {BUSY_THEN['fresh']: 'Start fresh when Claude finishes, or now, which stops the reply.',  # only the reply is live
              BUSY_THEN['stop']: 'Stop it when Claude finishes, or now, which cuts the reply off.',
              BUSY_THEN['handoff']: 'Close it when Claude finishes, or now, which stops the reply.',
              BUSY_THEN['continue']: 'Move it when Claude finishes, or now, which stops the reply.',
              'Relaunch when they finish, or now, which stops them.': 'Relaunch when Claude finishes, or now, which stops the reply.'}
SID_RE = re.compile(r'[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}|[A-Za-z0-9_-]{1,64}')
JOB_RE = re.compile(r'[0-9a-f]{12}|[a-z]-[0-9a-f]{12}')
TOKEN_RE = re.compile(r'[0-9a-f]{64,200}')
CTYPES = {'.html': 'text/html; charset=utf-8', '.css': 'text/css; charset=utf-8', '.js': 'text/javascript; charset=utf-8',
          '.svg': 'image/svg+xml', '.png': 'image/png', '.json': 'application/json', '.webmanifest': 'application/manifest+json',
          '.woff2': 'font/woff2', '.ico': 'image/x-icon'}
# connect-src: terminals are WebSockets to the hub (term.py), on the host the page came from
HOME_PAGE = """<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="color-scheme" content="light dark"><title>Switchboard</title><link rel="icon" href="/icon.svg">
<style>
:root { --bg: #F4F5F5; --ink: #15191C; --dim: #6A7078; --acc: #0C7480; --on: #fff; }
@media (prefers-color-scheme: dark) { :root { --bg: #121416; --ink: #EDEFF1; --dim: #9AA0A7; --acc: #3CC7C2; --on: #072624; } }
body { margin: 0; min-height: 100vh; display: grid; place-items: center; background: var(--bg); color: var(--ink);
  font: 16px/1.5 system-ui, -apple-system, 'Segoe UI', sans-serif; }
main { max-width: 440px; padding: 32px 20px; text-align: center; }
img { width: 56px; height: 56px; }
h1 { font-size: 24px; margin: 16px 0 8px; }
p { margin: 0 0 20px; color: var(--dim); }
.b { display: inline-block; padding: 10px 18px; border-radius: 9px; background: var(--acc); color: var(--on); font-weight: 600; text-decoration: none; }
.s { font-size: 14px; }
</style></head>
<body><main><img src="/icon.svg" alt=""><h1>Switchboard runs in its apps</h1>
<p>Use the Switchboard app on your Mac, Windows or Linux computer, or on your iPhone. This server is what they talk to.</p>
{{LINK}}</main></body></html>
"""

CSP = ("default-src 'self'; style-src 'self' 'unsafe-inline'; font-src 'self'; "
       f"img-src 'self' data:; connect-src 'self' ws://*:{term_mod.HUB_PORT}; frame-ancestors 'none'")


# ---- appearance, kept on the server so every device agrees: the app's light or dark, and how terminals look
SUMMARY_MODELS = ('opus', 'sonnet')
UPLOAD_MAX = 20 * 1024 * 1024  # the agent's cap too (sb.py UPLOAD_MAX)
UPLOAD_CHUNK_NODE = 1024 * 1024  # a hub link takes 2 MB a request, and base64 adds a third
UPLOAD_CHUNK_SSH = 6 * 1024 * 1024


def _mb(n):
    return f'{n / (1024 * 1024):.1f} MB'.replace('.0 MB', ' MB')


THEMES = ('system', 'light', 'dark')
TERM_CURSORS = ('block', 'bar', 'underline')
TERM_SIZES = range(9, 25)
WEB_DIR = Path(__file__).resolve().parent / 'web'
# The terminal schemes every app offers: one file, the one sb-core embeds (the iPhone's copy is generated from it).
SCHEMES_JSON = Path(__file__).resolve().parents[1] / 'native' / 'crates' / 'sb-core' / 'src' / 'term' / 'schemes.json'


def _scheme_ids():
    try:
        return {s['id'] for s in json.loads(SCHEMES_JSON.read_text())['schemes']}
    except (OSError, ValueError, KeyError, TypeError):
        return set()


SCHEME_IDS = _scheme_ids()


def appearance(d, base=None):
    """{"theme": system|light|dark, "term": {"scheme": "match"|<id> (default converge-dark), "size": 9-24|None, "size_phone": ..., "cursor":
    block|bar|underline, "blink": bool}}. `d` may set only some of it; the rest comes from `base` (or the defaults)."""
    d = d if isinstance(d, dict) else {}
    base = base if isinstance(base, dict) else {}
    bt = base.get('term') if isinstance(base.get('term'), dict) else {}
    t = d.get('term') if isinstance(d.get('term'), dict) else {}
    pick = lambda k, ok, dflt: t[k] if k in t and ok(t[k]) else bt[k] if k in bt and ok(bt[k]) else dflt
    size_ok = lambda v: v is None or (isinstance(v, int) and not isinstance(v, bool) and v in TERM_SIZES)
    theme = d.get('theme') if d.get('theme') in THEMES else base.get('theme') if base.get('theme') in THEMES else 'system'
    return {'theme': theme, 'term': {
        # Claude Code paints for a dark terminal, so the default is a dark scheme, not "match the app"
        'scheme': pick('scheme', lambda v: v == 'match' or v in SCHEME_IDS, 'converge-dark'),
        'size': pick('size', size_ok, None), 'size_phone': pick('size_phone', size_ok, None),
        'cursor': pick('cursor', lambda v: v in TERM_CURSORS, 'block'), 'blink': pick('blink', lambda v: isinstance(v, bool), True)}}


def _is_ip(h):
    try:
        ipaddress.ip_address(h)
        return True
    except ValueError:
        return False


CGNAT = ipaddress.ip_network('100.64.0.0/10')  # Tailscale's addresses
ALLOW_DEFAULT = ('private', 'local', 'tailscale')
ALLOW_KINDS = {'private', 'local', 'tailscale'}
LOCAL_NAME_RE = re.compile(r'[a-z0-9]([a-z0-9-]{0,62})(\.[a-z0-9-]{1,63})*\.local')
TS_NAME_RE = re.compile(r'([a-z0-9]([a-z0-9-]{0,62})\.)+ts\.net')


def _private_ip(h):
    try:
        ip = ipaddress.ip_address(h)
    except ValueError:
        return False
    return ip.is_private or ip.is_link_local or (ip.version == 4 and ip in CGNAT)


def _local_addresses():
    """This machine's names and addresses, best effort, for a first start with nothing to go on."""
    names = {socket.gethostname().split('.')[0].lower()}
    try:
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as u:  # no packet is sent: it only picks a route
            u.connect(('10.255.255.255', 1))  # leakscan: allow
            names.add(u.getsockname()[0])
    except OSError:
        pass
    try:
        out = subprocess.run(['tailscale', 'ip', '-4'], capture_output=True, text=True, timeout=3).stdout.split()
        names.update(x for x in out if _is_ip(x))
    except (OSError, subprocess.SubprocessError):
        pass
    return sorted(n for n in names if n and not n.startswith('127.'))


class ServerConfig:
    """Where the server listens and which addresses are its own, in ~/.config/switchboard/server.json:
    {"bind": ["10.0.0.5", ...], "hosts": ["10.0.0.5", "studio", ...], "origins": ["http://10.0.0.5:8130", ...],
     "allow": ["private", "local", "tailscale"]}.
    Nothing is built in. The first start seeds it from the origins an earlier server listed for the hub (so a
    server that was already set up keeps its addresses), else from this machine's own names and addresses, on all
    interfaces. Requests whose Host is not one of these are refused (DNS rebinding).

    "allow" also takes, by kind, addresses a phone may have been given: "private" any LAN, link-local or Tailscale IP
    typed as a number, "local" any Bonjour name (studio.local), "tailscale" any MagicDNS name (*.ts.net). A rebinding
    page cannot send any of those as its Host (it needs a public name of its own), so they stay closed to the world.
    Left out, all three are on; [] turns them off."""

    def __init__(self, cfg_dir, state_dir, port=PORT, local=_local_addresses):
        self.path = Path(cfg_dir) / 'server.json'
        d = self._read(self.path)
        if d is None:
            d = self._seed(Path(state_dir) / 'machines.json', port, local)
            try:
                self.path.parent.mkdir(parents=True, exist_ok=True)
                self.path.write_text(json.dumps(d, indent=1))
            except OSError:
                pass
        self.bind = d['bind'] or ['0.0.0.0']
        self.origins = d['origins']
        self.hosts = set(h.lower() for h in d['hosts']) | set(LOCAL_HOSTS)
        self.allow = set(ALLOW_DEFAULT if d.get('allow') is None else d['allow']) & ALLOW_KINDS
        # a tailnet name of any of its hosts (studio.tail1234.ts.net) is its own too
        self.ts_names = [h for h in self.hosts if not _is_ip(h) and '.' not in h and h not in LOCAL_HOSTS]

    @staticmethod
    def _read(path):
        try:
            d = json.loads(Path(path).read_text())
        except (OSError, ValueError):
            return None
        if not isinstance(d, dict):
            return None
        lst = lambda k: [x for x in d.get(k) or [] if isinstance(x, str) and x.strip()]
        return {'bind': lst('bind'), 'hosts': lst('hosts'), 'origins': lst('origins'),
                'allow': lst('allow') if isinstance(d.get('allow'), list) else None}

    @staticmethod
    def _seed(old_file, port, local):
        try:
            origins = [o for o in json.loads(old_file.read_text()).get('origins') or [] if isinstance(o, str)]
        except (OSError, ValueError, AttributeError):
            origins = []
        hosts = [urlsplit(o).hostname for o in origins if urlsplit(o).hostname]
        if hosts:  # and this machine's own name, which its tailnet name starts with
            own = socket.gethostname().split('.')[0].lower()
            return {'bind': [h for h in dict.fromkeys(hosts) if _is_ip(h)] or ['0.0.0.0'],
                    'hosts': list(dict.fromkeys(hosts + ([own] if own else []))), 'origins': origins}
        hosts = local()
        return {'bind': ['0.0.0.0'], 'hosts': hosts, 'origins': [f'http://{h}:{port}' for h in hosts]}

    def host_ok(self, host):
        host = (host or '').lower()
        if host in self.hosts or any(re.fullmatch(re.escape(n) + r'(\.[a-z0-9-]+)*\.ts\.net', host) for n in self.ts_names):
            return True
        if _private_ip(host):
            ip = ipaddress.ip_address(host)
            return 'private' in self.allow or ('tailscale' in self.allow and ip.version == 4 and ip in CGNAT)
        return ('local' in self.allow and bool(LOCAL_NAME_RE.fullmatch(host))) or \
            ('tailscale' in self.allow and bool(TS_NAME_RE.fullmatch(host)))


class HttpError(Exception):
    def __init__(self, code, msg):
        super().__init__(msg)
        self.code = code


def log(msg):
    print(time.strftime('%Y-%m-%d %H:%M:%S'), msg, flush=True)


def fmt_idle(sec):
    sec = int(sec or 0)
    if sec >= 86400:
        return f'{sec // 86400}d'
    if sec >= 3600:
        return f'{sec // 3600}h'
    return f'{max(1, sec // 60)}m'


class App:
    def __init__(self, fleet, state_dir, web_dir, pusher=None, guard_delay=20, clock=time.time, sleep=time.sleep,
                 poll_every=3, retry_waits=(5, 15, 30), cfg_dir=None, server_cfg=None):
        self.fleet, self.state_dir, self.web_dir = fleet, Path(state_dir), Path(web_dir)
        self.cfg_dir = Path(cfg_dir) if cfg_dir else self.state_dir / 'config'
        had_state = self._had_state()  # before anything below writes a file
        self.state_dir.mkdir(parents=True, exist_ok=True)
        self.server_cfg = server_cfg  # None in tests: only localhost is its own
        self.live_pusher = None  # Live Activity pushes (main wires it to APNs)
        self.push_ready = False  # main sets it when <config>/apns.json and its key load
        self.fleet_cfg = fleet_mod.FleetConfig(self.cfg_dir / 'fleet.json')
        if hasattr(fleet, 'on_save'):
            fleet.on_save = self._fleet_saved
            fleet.servers = list(server_cfg.origins) if server_cfg else []
        self._migrate_options(had_state)
        fleet.summary_model = self.prefs()['summary_model']  # the agents hear it on every poll
        fleet.options = self.options()  # ... and the opt-in settings (options.py)
        if getattr(fleet, 'history', False) is None:  # every poll writes down the weekly percent for the widgets
            fleet.history = fleet_mod.WeekHistory(self.state_dir / 'week_history.json')
        self.pusher = pusher or (lambda *a: None)
        self.guard = GuardTimer(guard_delay, self._push_guard)
        self.clock, self.sleep, self.poll_every, self.retry_waits = clock, sleep, poll_every, retry_waits
        self.jobs, self.jobs_lock = {}, threading.Lock()
        self.chrome_lock = threading.Lock()
        self._names_lock, self._name_tries = threading.Lock(), {}  # names given in Switchboard (set_name)
        # live updates: every change bumps the version; open pages hear it over /api/stream and load again
        self.version, self.changed, self.streams = 0, threading.Condition(), 0
        self.overlay, self.overlay_lock, self._polls = {}, threading.Lock(), set()
        self.widget_pusher, self.widget_lock = None, threading.Lock()
        self._widget, self._answered = {'key': None, 'sent': None, 'last': 0, 'timer': None}, set()
        if hasattr(fleet, 'on_change'):
            fleet.on_change = self._machine_changed
        self.feat = features_mod.Features(self, self.cfg_dir, HttpError)  # v2: reaper, threads, auto, dispatch
        self._recover_jobs()
        self._terms_setup(clock)

    # ---- helpers
    def devices(self):
        try:
            return json.loads((self.state_dir / 'devices.json').read_text())
        except (OSError, ValueError):
            return {}

    def _push(self, title, body, url):
        log(f'push {title!r} {url}')
        try:
            self.pusher(title, body, url)
        except Exception:
            log('push failed: ' + traceback.format_exc())

    def _push_guard(self, evt):
        title = (evt.get('title') or 'A session').strip()[:80]
        body = f'{title}: {int(evt.get("ctx") or 0) // 1000}k context, idle {fmt_idle(evt.get("idle"))}. Tap to start it fresh.'
        self._push('Old session held', body, f'/s/{evt.get("machine")}/{evt.get("session_id")}')

    def _machine(self, body_or_name):
        name = body_or_name if isinstance(body_or_name, str) else body_or_name.get('machine')
        if name not in self.fleet.machines:
            raise HttpError(400, 'unknown machine')
        return name

    def _label(self, name):
        return self.fleet.machines.get(name, {}).get('label') or name

    def _title(self, name, sid):
        given = self.names().get(f'{name}/{sid}')
        if given:
            return given
        for m in self.fleet.snapshot()['machines']:
            if m['name'] == name:
                for s in m['sessions'] + (m.get('replaced') or []):
                    if s.get('sid') == sid:
                        return s.get('title')
        return None

    def _call(self, name, req, timeout=40):
        try:
            r = self.fleet.call(name, req, timeout)
        except fleet_mod.FleetError as e:
            raise HttpError(502, str(e))
        if not r.get('ok'):
            raise HttpError(400, r.get('error') or 'request failed')
        return r

    # ---- files dropped or pasted into a terminal: to the session's machine, in chunks small enough for a hub link
    def upload(self, query, data):
        """POST /api/upload?machine=&name= with the file as the body. The agent writes it under its uploads folder
        and answers with the full path, which the terminal types into the chat."""
        q = lambda k: ((query or {}).get(k) or [''])[0]
        name = self._machine(q('machine'))
        label, st = self._label(name), self.fleet.state.get(name) or {}
        if not data:
            raise HttpError(400, 'That file is empty.')
        if len(data) > UPLOAD_MAX:
            raise HttpError(413, f'That file is {_mb(len(data))}. Files up to {_mb(UPLOAD_MAX)} can be attached.')
        if not st.get('online'):
            raise HttpError(409, f"{label} is offline, so the file can't go there. Try again once {label} is back.")
        if 'upload' not in (st.get('features') or []):
            raise HttpError(409, f'Update Switchboard on {label} to attach files to its chats.')
        node = (self.fleet.machines.get(name) or {}).get('target', '').startswith('node:')
        step, uid, r = UPLOAD_CHUNK_NODE if node else UPLOAD_CHUNK_SSH, uuid.uuid4().hex, {}
        for off in range(0, len(data), step):
            part = data[off:off + step]
            r = self._call(name, {'verb': 'upload', 'id': uid, 'name': q('name') or 'file', 'offset': off, 'size': len(data),
                                  'last': off + step >= len(data), 'data': base64.b64encode(part).decode()}, timeout=90)
        if not isinstance(r.get('path'), str):
            raise HttpError(502, f'{label} did not say where it put the file.')
        return {'ok': True, 'machine': name, 'path': r['path'], 'name': r.get('name') or '', 'size': len(data)}

    def _refresh_soon(self, name, delay=1.5):
        threading.Timer(delay, lambda: self.fleet.poll_one(name)).start()

    def sessions(self):
        snap = self.fleet.snapshot()
        names, chrome = self.names(), self.chrome_machines()
        known = set()
        for m in snap['machines']:
            for s in m['sessions']:
                s['held'] = self.guard.held(m['name'], s['sid'])
            for s in m['sessions'] + (m.get('replaced') or []):
                k = f'{m["name"]}/{s.get("sid")}'
                n = names.get(k)
                if n and s.get('title') == n:
                    known.add(k)  # the machine has it: from here its own list says the name
                elif n:
                    s['title'] = n
                    self._deliver_name(m, k, n)
                s['chrome_machine'] = chrome.get(k)
        if known:
            self.forget_names(known)
        self._apply_overlay(snap)
        snap['summary'] = self.fleet.summary()
        snap['summary']['note'] = self.note(snap['machines'], snap['summary'])
        snap['v'] = self.version
        prefs = self.prefs()
        snap['prefs'] = dict({'chrome_machine': prefs.get('chrome_machine'), 'appearance': prefs['appearance'],
                              'summary_model': prefs['summary_model']}, **options_mod.clean(prefs))
        return self.feat.decorate(snap)

    def note(self, machines, summary):
        """The status line under the plan gauges (notes.py)."""
        try:
            return notes_mod.pick(machines, summary, self.clock(), self.guard.held)
        except Exception:
            log('note failed: ' + traceback.format_exc(limit=2))
            return None

    def summary(self):
        snap = self.fleet.snapshot()
        self._apply_overlay(snap)
        sm = self.fleet.summary()
        sm['note'] = self.note(snap['machines'], sm)
        return sm

    # ---- flags of a running session, and relaunching it with others
    def _session(self, name, sid):
        m = next((m for m in self.fleet.snapshot()['machines'] if m['name'] == name), None)
        s = next((s for s in (m or {}).get('sessions') or [] if s.get('sid') == sid), None)
        if s is None:
            raise HttpError(404, 'That chat is not on the list any more.')
        return m, s

    # ---- the desktop app's "Restart to update": the newest build on the download page
    LATEST_FOR = 300  # seconds a look at the download page is kept

    def app_latest(self):
        """GET /api/app: {latest: <the download page's latest.json> | None, base: <its URL>, feed: {manifest, sig}}.
        Where the page is lives in <config>/downloads.json ({"url": "http://host:port/app"}), which publish.sh writes;
        without it, nothing is offered and set_up is false. `feed` is latest.json as text, byte for byte, with
        latest.json.sig (None when the page has none): the app checks that signature itself, so this server is
        only a messenger and can't change what is offered. `latest` is the parsed copy older apps read. The app
        never installs on its own."""
        now = time.time()
        c = getattr(self, '_latest', None)
        if c and now - c[0] < self.LATEST_FOR:
            return c[1]
        try:
            url = json.loads((self.cfg_dir / 'downloads.json').read_text()).get('url')
        except (OSError, ValueError, AttributeError):
            url = None
        out = {'ok': True, 'latest': None, 'set_up': False}
        if isinstance(url, str) and re.fullmatch(r'https?://[A-Za-z0-9.:\-]+(?:/[A-Za-z0-9._\-/]*)?', url):
            base = url.rstrip('/')
            try:
                with urlopen(base + '/latest.json', timeout=5) as r:
                    text = r.read(200_000).decode('utf-8')
                d = json.loads(text)
                try:
                    with urlopen(base + '/latest.json.sig', timeout=5) as r:
                        sig = r.read(1000).decode('ascii').strip() or None
                except Exception:  # an unsigned page: the app shows nothing
                    sig = None
                out = {'ok': True, 'latest': d if isinstance(d, dict) else None, 'base': base, 'set_up': True,
                       'feed': {'manifest': text, 'sig': sig}}
            except Exception as e:  # the page being down never breaks anything else
                out = {'ok': True, 'latest': None, 'base': base, 'set_up': True, 'error': str(e)[:200]}
        self._latest = (now, out)
        return out

    def flags_view(self, name, sid):
        """GET /api/flags?machine=&sid=: the flags drawer's "Active now" and the header's summary."""
        name = self._machine(name)
        if not isinstance(sid, str) or not SID_RE.fullmatch(sid):
            raise HttpError(400, 'bad session id')
        m, s = self._session(name, sid)
        return dict(flags_mod.view(s), ok=True, machine=name, sid=sid, label=m.get('label') or name, title=s.get('title'),
                    kind=s.get('kind'), status=s.get('status'),
                    running=s.get('kind') == 'background' and s.get('status') in ('idle', 'busy', 'waiting'))

    def flags_plan(self, body):
        """POST /api/flags {machine, sid, want}: each change, and whether it applies live or needs a relaunch."""
        name, sid = self._machine(body), body.get('sid')
        if not isinstance(sid, str) or not SID_RE.fullmatch(sid):
            raise HttpError(400, 'bad session id')
        _, s = self._session(name, sid)
        try:
            return dict(flags_mod.plan(s, body.get('want'), (self.fleet.state.get(name) or {}).get('root')), ok=True)
        except flags_mod.FlagError as e:
            raise HttpError(400, str(e))

    def relaunch(self, body):
        """POST /api/relaunch {machine, sid, flags, other, model, effort, when: "idle"|"now"}: the chat restarts with
        exactly these flags and carries on where it left off (claude --bg --resume <sid> <flags>, in the chat's own
        folder; Claude Code gives it a new id). Mid-reply, it waits for Claude to finish ("idle") or goes now ("now",
        only when the user said so); with neither it answers need "busy" so the client can ask."""
        name, sid = self._machine(body), body.get('sid')
        if not isinstance(sid, str) or not SID_RE.fullmatch(sid):
            raise HttpError(400, 'bad session id')
        try:
            f = flags_mod.parse(body.get('flags') or [], body.get('other') or '')
        except flags_mod.FlagError as e:
            raise HttpError(400, str(e))
        model = body.get('model') or f.get('model')
        effort = body.get('effort') or f.get('effort')
        if model not in MODELS or (effort not in EFFORTS and effort != 'ultracode'):
            raise HttpError(400, 'unknown model or effort')
        m, s = self._session(name, sid)
        label = m.get('label') or name
        if 'relaunch' not in ((self.fleet.state.get(name) or {}).get('features') or []):  # an older agent would just resume
            raise HttpError(409, f'Update Switchboard on {label} to relaunch with new flags.')
        if s.get('kind') == 'interactive' and s.get('status') not in (None, 'stopped'):
            # Switchboard does not own that terminal: it says what to run there instead
            words = ['claude', '--resume', sid] + (['--model', model] if model else []) + (['--effort', effort] if effort else [])
            cmd = ' '.join(words + ([flags_mod.command_line(f['flags'] + ([['--chrome']] if f['chrome'] else []))] if f['flags'] or f['chrome'] else []))
            return {'ok': False, 'need': 'terminal', 'command': cmd,
                    'error': f'This chat is open in a terminal on {label}. Exit Claude there (type /exit), then run this in the same folder.'}
        when = body.get('when')
        need = self.busy_need(name, sid, when, 'Relaunch when they finish, or now, which stops them.')
        if need:
            return need
        req = {'verb': 'resume', 'sid': sid, 'relaunch': True, 'flags': f['flags'] + ([['--chrome']] if f['chrome'] else []),
               'model': model, 'effort': effort}
        return self._submit('relaunch', name, {'req': req, 'wait_idle': when == 'idle'}, self._title(name, sid))

    EDIT_FLAGS = {'perm': '--permission-mode', 'agent': '--agent', 'dirs': '--add-dir', 'chrome': '--chrome'}

    def relaunch_edit(self, body):
        """POST /api/action {verb: "relaunch", machine, sid, flags: {perm|agent|model|effort|chrome|dirs: [values],
        remove: [keys]}, when?}: the iPhone app sends what changes, not the whole set. The change is applied to what
        the chat was launched with, then it goes through relaunch like everyone else's. Without "when" it waits for the
        chat's own work to end (subagents, commands, a reply), so an older app never cuts anything off."""
        name, sid = self._machine(body), body.get('sid')
        if not isinstance(sid, str) or not SID_RE.fullmatch(sid):
            raise HttpError(400, 'bad session id')
        edit = body.get('flags')
        if not isinstance(edit, dict):
            raise HttpError(400, 'flags must say what changes')
        remove = edit.get('remove') or []
        if not isinstance(remove, list) or not all(isinstance(k, str) for k in remove):
            raise HttpError(400, 'remove must be a list of names')
        keys = set(remove) | {k for k in edit if k != 'remove'}
        known = set(self.EDIT_FLAGS) | {'model', 'effort'}
        if keys - known:
            raise HttpError(400, f'{sorted(keys - known)[0][:40]!r} is not a flag the app can change')
        vals = {}
        for k in edit:
            if k == 'remove':
                continue
            v = edit[k]
            if not isinstance(v, list) or not all(isinstance(x, str) for x in v) or (k != 'dirs' and len(v) != 1):
                raise HttpError(400, f'{k}: give one value' if k != 'dirs' else 'dirs: give a list of folders')
            vals[k] = v
        _, s = self._session(name, sid)
        launch = [list(f) for f in ((s.get('launch') or {}).get('flags') or []) if isinstance(f, list) and f]
        now = s.get('now') or {}
        touched = {self.EDIT_FLAGS[k] for k in keys if k in self.EDIT_FLAGS}
        # Switchboard's own (--remote-control) and model and effort (sent on their own) are not carried as flags
        flags = [f for f in launch if flags_mod.ALIASES.get(f[0], f[0]) in flags_mod.START_FLAGS
                 and flags_mod.ALIASES.get(f[0], f[0]) not in touched]
        if 'perm' not in keys and not any(flags_mod.ALIASES.get(f[0], f[0]) == '--permission-mode' for f in flags) \
                and now.get('perm') not in (None, 'default'):
            flags.append(['--permission-mode', now['perm']])  # what it runs with now, when the launch did not say
        if 'chrome' not in keys and now.get('chrome') and not any(f[0] == '--chrome' for f in flags):
            flags.append(['--chrome'])
        for k, flag in self.EDIT_FLAGS.items():
            if k not in vals or k in remove:
                continue
            if k == 'chrome':
                if vals[k][0].lower() not in ('', 'off', 'false', '0'):
                    flags.append(['--chrome'])
            elif k == 'dirs':
                flags += [['--add-dir', d] for d in vals[k] if d.strip()]
            elif vals[k][0] and not (k == 'perm' and vals[k][0] == 'default'):
                flags.append([flag, vals[k][0]])
        pick = lambda k: None if k in remove else (vals[k][0] or None) if k in vals else now.get(k)
        return self.relaunch({'machine': name, 'sid': sid, 'flags': flags, 'model': pick('model'), 'effort': pick('effort'),
                              'when': body.get('when') or 'idle'})

    # ---- what a relaunch, stop, move or fresh start would cut off (the agent's busy verb)
    def busy(self, name, sid):
        """{live, subagents, commands, replying, pending, summary} for one chat. A machine without the busy verb
        answers from the list: live means Claude is replying."""
        try:
            r = self._call(name, {'verb': 'busy', 'sid': sid}, timeout=30)
        except HttpError:
            r = None
        if isinstance(r, dict) and isinstance(r.get('live'), bool):
            out = {k: [str(x) for x in r.get(k) or []][:20] for k in ('subagents', 'commands', 'pending')}
            return dict(out, live=r['live'], replying=bool(r.get('replying')), summary=str(r.get('summary') or ''))
        try:
            _, s = self._session(name, sid)
        except HttpError:
            s = {}
        replying = s.get('status') == 'busy'
        return {'live': replying, 'subagents': [], 'commands': [], 'replying': replying, 'pending': [],
                'summary': 'Claude is still replying.' if replying else ''}

    def busy_need(self, name, sid, when, then):
        """None when it can go ahead (nothing running, or the user already picked "when they finish" or "now");
        else the answer that asks: {ok: False, need: 'busy', busy, error}."""
        if when in ('idle', 'now'):
            return None
        b = self.busy(name, sid)
        if not b['live']:
            return None
        if not (b['subagents'] or b['commands'] or b['pending']):
            then = BUSY_REPLY.get(then, then)
        return {'ok': False, 'need': 'busy', 'busy': b, 'error': f"{b['summary']} {then}".strip()}

    def _wait_idle(self, jid, name, sid):
        """Waits until nothing in the chat would be cut off: no reply, subagent, background command or undelivered
        result. A reply alone is waited on for 15 minutes; subagents and commands for 4 hours."""
        start = self.clock()
        while True:
            if self.jobs.get(jid, {}).get('cancel'):  # first: a called-off job never goes ahead
                raise HttpError(409, 'Cancelled. Nothing was changed.')
            try:
                self._session(name, sid)
            except HttpError:
                return
            b = self.busy(name, sid)
            if not b['live']:
                return
            only_reply = not (b['subagents'] or b['commands'] or b['pending'])
            if self.clock() - start > (RELAUNCH_WAIT if only_reply else CHILD_WAIT):
                raise HttpError(409, ('Claude was still replying after 15 minutes' if only_reply else
                                      'Its work was still running after 4 hours') + ', so nothing was changed. Try again when it is done.')
            self._set(jid, state='queued', step='waiting', note=b['summary'])
            self.sleep(self.poll_every)
            self.fleet.poll_one(name)

    # ---- live updates
    def bump(self):
        with self.changed:
            self.version += 1
            self.changed.notify_all()

    def wait_change(self, v, timeout):
        with self.changed:
            self.changed.wait_for(lambda: self.version != v, timeout)
            return self.version

    def _machine_changed(self, name):
        self.bump()
        first = name not in self._answered  # a machine's first answer since the start is not news
        self._answered.add(name)
        self._widget_check(quiet=first)

    def stream_slot(self, on):
        with self.changed:
            if on and self.streams >= STREAM_MAX:
                return False
            self.streams += 1 if on else -1
            return True

    def ping(self, body):
        """A machine's hook or status line: a prompt went in, Claude finished, it asks for permission, the context
        grew. Shown at once (an overlay on the last poll), then the machine is asked to confirm."""
        name, sid, why = body['machine'], body['session_id'], body.get('why')
        if why not in PING_WHY or not SID_RE.fullmatch(sid):
            raise HttpError(400, 'bad ping')
        o = {}
        if why == 'prompt':
            o['status'] = 'busy'
        elif why == 'stop':
            o['status'] = 'idle'
        elif why == 'wait' and body.get('kind') == 'permission_prompt':
            o['status'] = 'waiting'
        if why in ('prompt', 'stop') or o.get('status') == 'waiting':
            o['why'] = why  # derive() decides from the ping itself, not only the status it stands for
        ctx = body.get('ctx')
        if why == 'status' and isinstance(ctx, int) and not isinstance(ctx, bool) and 0 < ctx < 10_000_000:
            o['ctx'] = ctx
        exp, ts = body.get('cache_expires_at'), body.get('ts')
        if why == 'status' and all(isinstance(x, (int, float)) and not isinstance(x, bool) and x > 0 for x in (exp, ts)) \
                and 0 < exp - ts < 2 * 3600:
            o['cold_at'] = round(exp + (self.clock() - ts), 3)  # its machine's clock, as of the ping, on ours
        if o:
            with self.overlay_lock:
                self.overlay[(name, sid)] = dict(self.overlay.get((name, sid)) or {}, **o, at=self.clock())
            self.bump()
        if why != 'status':  # the status line changes all the time; its numbers need no confirming
            self._poll_soon(name, 0.3)
        return {'ok': True}

    def _apply_overlay(self, snap):
        """Hook pings newer than a machine's last answer, and messages the guard holds, go into each session, and
        derive() (fleet.py) decides its live state again: a ping and a poll never disagree."""
        now = self.clock()
        with self.overlay_lock:
            for k in [k for k, o in self.overlay.items() if now - o['at'] > OVERLAY_FOR]:
                del self.overlay[k]
            ov = dict(self.overlay)
        for m in snap['machines']:
            asked = m.get('asked') or 0
            for s in m['sessions']:
                o = ov.get((m['name'], s.get('sid')))
                if o and o['at'] < asked:  # the machine has answered since: it knows best
                    o = None
                if o and 'ctx' in o:
                    s['ctx'] = o['ctx']
                if o and 'status' in o and s.get('status') in ('idle', 'busy', 'waiting'):
                    s['status'] = o['status']
                s['held'] = self.guard.held(m['name'], s.get('sid'))
                hook = {'why': o.get('why'), 'at': o['at'], 'cold_at': o.get('cold_at')} if o else None
                if hook and s.get('status') not in ('idle', 'busy', 'waiting'):
                    hook['why'] = None  # stopped, or closed: a late ping does not bring it back
                s.update(fleet_mod.derive(s, now, m.get('skew') or 0.0, s['held'], hook, m.get('updated')))
                s['stalled'] = s['activity'] == 'stalled'

    def _poll_soon(self, name, delay):
        """Ask a machine for its list once, soon; pings that come together share one ask."""
        with self.overlay_lock:
            if name in self._polls:
                return
            self._polls.add(name)

        def run():
            with self.overlay_lock:
                self._polls.discard(name)
            try:
                self.fleet.poll_one(name)
            except Exception:
                log('poll failed: ' + traceback.format_exc(limit=1))
        t = threading.Timer(delay, run)
        t.daemon = True
        t.start()

    # ---- iPhone widgets: a WidgetKit push when what they show changed
    def widget_tokens(self):
        try:
            d = json.loads((self.state_dir / 'widget_tokens.json').read_text())
            return d if isinstance(d, dict) else {}
        except (OSError, ValueError):
            return {}

    def _save_json(self, name, d):
        tmp = self.state_dir / f'{name}.{os.getpid()}.{threading.get_ident()}.tmp'
        tmp.write_text(json.dumps(d, indent=1))
        os.replace(tmp, self.state_dir / name)

    def _widget_check(self, quiet=False):
        if not self.widget_pusher:
            return
        s = self.fleet.summary()
        pct = lambda v: round(v) if isinstance(v, (int, float)) else None
        key = json.dumps([pct(s.get('five_hour')), pct(s.get('seven_day')), s.get('running'), s.get('cold'), s.get('online'),
                          s.get('sessions'), [(m['name'], m['online'], m['running'], m['cold']) for m in s.get('machine_list') or []]])
        w = self._widget
        with self.widget_lock:
            if w['key'] is None or quiet:  # the first round after a start: the widgets already show this
                w['key'] = w['sent'] = key
                return
            w['key'] = key
            if w['timer'] or w['sent'] == key:  # one on its way sends the newest; or nothing new to send
                return
            wait = max(0.0, w['last'] + WIDGET_EVERY - self.clock())
            w['timer'] = t = threading.Timer(wait, self._widget_send)
        t.daemon = True
        t.start()

    def _widget_send(self):
        w = self._widget
        with self.widget_lock:
            w['timer'] = None
            if w['sent'] == w['key']:  # it changed back while waiting
                return
            w['last'], w['sent'] = self.clock(), w['key']
        try:
            self.widget_pusher()
        except Exception:
            log('widget push failed: ' + traceback.format_exc(limit=1))

    # ---- machines (Settings): one list in fleet.json, the same one Add a machine writes to
    def host_ok(self, host):
        h = (host or '').strip().lower()
        if h.startswith('['):
            h = h[1:h.index(']')] if ']' in h else h[1:]
        elif h.count(':') == 1:
            h = h.split(':')[0]
        return self.server_cfg.host_ok(h) if self.server_cfg else h in LOCAL_HOSTS

    def _fleet_saved(self, machines):
        try:
            self.fleet_cfg.save(machines)
        except OSError as e:
            log(f'could not save the machine list: {e}')
        try:
            self.terms.write_machines()
        except (OSError, AttributeError) as e:
            log(f'could not write machines.json for the hub: {e}')
        self.bump()

    def machines_view(self):
        snap = {m['name']: m for m in self.fleet.snapshot()['machines']}
        out = []
        for m in self.fleet.config():
            s = snap.get(m['name']) or {}
            linked = m['target'].startswith(nodes_mod.PREFIX)
            out.append({'name': m['name'], 'label': m['label'], 'os': m.get('os') or '', 'form': m.get('form'),
                        'icon': fleet_mod.icon_of(m), 'via': 'link' if linked else 'ssh', 'target': None if linked else m['target'],
                        'root': m.get('root'), 'online': s.get('online'), 'root_shown': self._root_shown(m, s),
                        'needs_root': not m.get('root') and s.get('root_set') is False, 'root_suggest': s.get('root_suggest'),
                        'versions': s.get('versions'), 'features': s.get('features') or [], 'updated': s.get('updated'),
                        'auto': self.feat.auto.settings().get(m['name'], {}).get('on', False), 'can': self.feat.auto.view(m['name'])['can']})
        return {'ok': True, 'machines': out, 'max': fleet_mod.MACHINES_MAX}

    @staticmethod
    def _root_shown(m, s):
        """The Claude folder as Settings shows it: the one picked, else the one the machine uses."""
        r = m.get('root') or s.get('root')
        return r if r else None

    def machine_extra(self, name, what, query):
        """GET /api/machines/<name>/<what>: Settings' live views of one machine. folders: the folders under its home
        folder, to pick the Claude folder from (?path=). mcp: its MCP servers, nothing secret. caps: what it can do."""
        name = self._machine(name)
        feats = self.fleet.state.get(name, {}).get('features') or []
        if what == 'caps':
            return self.feat.auto.view(name, fetch=self.fleet.state.get(name, {}).get('online') is True)
        verb = {'folders': 'root_dirs', 'mcp': 'mcp'}[what]
        if verb not in feats:
            raise HttpError(409, f'Update Switchboard on {self._label(name)} to see this here.')
        req = {'verb': verb}
        if what == 'folders':
            req['path'] = ((query or {}).get('path') or [''])[0]
        return self._call(name, req, timeout=20)

    def machines_post(self, body):
        """Settings: rename, reorder, remove, set a machine's folder, or add one reached over ssh."""
        op = body.get('op')
        try:
            if op == 'rename':
                self.fleet.rename(self._machine(body), body.get('label'))
            elif op == 'root':
                self.fleet.set_root(self._machine(body), body.get('root'))
            elif op == 'caps':  # Auto: on or off, tools to leave out, the user's own lines
                name = self._machine(body)
                self.feat.auto.set_setting(name, body)
                self.bump()
            elif op == 'order':
                self.fleet.reorder(body.get('names'))
            elif op == 'remove':
                name = self._machine(body)
                if self.fleet.machines[name]['target'].startswith(nodes_mod.PREFIX):
                    try:  # a linked machine is unpaired too, or the hub would bring it back
                        nodes_mod.hub_call('POST', f'/internal/unpair/{name}', {})
                    except OSError as e:
                        raise HttpError(502, f'The hub could not unlink it ({e}).')
                self.fleet.drop_machine(name)
            elif op == 'add':
                name, target = body.get('name'), body.get('target')
                if not isinstance(name, str) or not fleet_mod.NAME_RE.fullmatch(name or ''):
                    raise HttpError(400, 'An id is 1 to 32 lowercase letters or digits.')
                if name in self.fleet.machines:
                    raise HttpError(409, 'There is a machine with that id already.')
                if not fleet_mod.valid_target(target) or target.startswith(nodes_mod.PREFIX):
                    raise HttpError(400, 'The address is user@host, as you would type it after ssh.')
                self.fleet.add_machine({'name': name, 'label': body.get('label') or name, 'target': target, 'os': body.get('os') or ''})
                self._poll_soon(name, 0.2)
            else:
                raise HttpError(400, 'unknown change')
        except fleet_mod.FleetError as e:
            raise HttpError(400, str(e))
        return self.machines_view()

    # ---- names given in Switchboard, and the New sheet's settings
    def names(self):
        try:
            d = json.loads((self.state_dir / 'names.json').read_text())
            return d if isinstance(d, dict) else {}
        except (OSError, ValueError):
            return {}

    def set_name(self, key, name):
        """A name given in Switchboard, or '' to clear it. Kept here until the machine's own list says it, so every
        app shows it at once even while the machine is slow or offline."""
        with self._names_lock:
            d = self.names()
            if name:
                d[key] = name
            else:
                d.pop(key, None)
            self._save_json('names.json', d)
        self._name_tries[key] = time.time()
        self.bump()

    def forget_names(self, keys):
        with self._names_lock:
            d = self.names()
            if any(k in d for k in keys):
                self._save_json('names.json', {k: v for k, v in d.items() if k not in keys})

    def _deliver_name(self, m, key, name):
        """The machine was offline when the chat was renamed: tell it again, at most once a minute."""
        if not m.get('online') or time.time() - self._name_tries.get(key, 0) < 60:
            return
        self._name_tries[key] = time.time()
        sid = key.split('/', 1)[1]

        def go():
            try:
                self._call(m['name'], {'verb': 'rename', 'sid': sid, 'name': name}, timeout=20)
                self._refresh_soon(m['name'], 0.5)
            except Exception as e:
                log(f'rename on {m["name"]}: {e}')
        threading.Thread(target=go, daemon=True).start()

    # ---- each chat's own Chrome machine: where its Chrome work goes (Auto, Start fresh, Continue), never one for all
    def chrome_machines(self):
        """{"<machine>/<sid>": "<chrome machine>"} for the chats that picked one."""
        try:
            d = json.loads((self.state_dir / 'chrome.json').read_text())
            return d if isinstance(d, dict) else {}
        except (OSError, ValueError):
            return {}

    def chrome_for(self, name, sid):
        return self.chrome_machines().get(f'{name}/{sid}') if name and sid else None

    def set_chrome_for(self, name, sid, chrome):
        """A chat's Chrome machine, or None to clear it. Only known machines; the newest 2000 chats are kept."""
        if chrome not in (None, '') and chrome not in self.fleet.machines:
            raise HttpError(400, 'unknown machine')
        with self.chrome_lock:
            d = self.chrome_machines()
            d.pop(f'{name}/{sid}', None)
            if chrome:
                d[f'{name}/{sid}'] = chrome
            self._save_json('chrome.json', dict(list(d.items())[-2000:]))
        self.bump()

    def _prefs_raw(self):
        try:
            d = json.loads((self.state_dir / 'prefs.json').read_text())
        except (OSError, ValueError):
            d = {}
        return d if isinstance(d, dict) else {}

    def prefs(self):
        d = self._prefs_raw()
        return dict({'chrome_machine': d.get('chrome_machine'), 'last': d.get('last'), 'presets': d.get('presets') or [],
                     'appearance': appearance(d.get('appearance')),
                     'summary_model': d.get('summary_model') if d.get('summary_model') in SUMMARY_MODELS else 'opus',
                     'migrations': [m for m in d.get('migrations') or [] if isinstance(m, str)]}, **options_mod.clean(d))

    def options(self):
        """The opt-in settings (options.py): {key: bool}."""
        return options_mod.clean(self._prefs_raw())

    def _had_state(self):
        """Whether this server ran before: a machine list, or anything in its state folder."""
        try:
            if (self.cfg_dir / 'fleet.json').exists():
                return True
            return any(not p.is_dir() or any(p.iterdir()) for p in self.state_dir.iterdir())
        except OSError:
            return False

    def _migrate_options(self, had_state):
        """Once: the opt-in settings start off on a new server, and on for one that was already running, so an
        existing install keeps working as before. Recorded in prefs.json so it never runs again."""
        d = self.prefs()
        opts, done, ran = options_mod.migrate(d, had_state)
        if not ran:
            return
        d.update(opts, migrations=done)
        self._save_json('prefs.json', d)
        log('options: ' + ('this server ran before this change, so every opt-in setting stays on' if had_state
                           else 'a new server: every opt-in setting starts off'))

    def flag_set(self, d):
        """A New sheet setup (the last one used, or a preset), checked: model, effort, mode, flags, Other flags."""
        if not isinstance(d, dict):
            raise HttpError(400, 'a setup is an object')
        out = {'model': d.get('model') or None, 'effort': d.get('effort') or None, 'mode': d.get('mode') or 'normal',
               'flags': d.get('flags') or [], 'other': d.get('other') or ''}
        if out['model'] not in MODELS or out['effort'] not in EFFORTS or out['mode'] not in MODES:
            raise HttpError(400, 'unknown model, effort or mode')
        try:
            flags_mod.parse(out['flags'], out['other'])
        except flags_mod.FlagError as e:
            raise HttpError(400, str(e))
        return out

    def set_prefs(self, body):
        d = self.prefs()
        if 'chrome_machine' in body:
            v = body['chrome_machine']
            if v not in (None, '') and v not in self.fleet.machines:
                raise HttpError(400, 'unknown machine')
            d['chrome_machine'] = v or None
        if 'presets' in body:
            ps = body['presets']
            if not isinstance(ps, list) or len(ps) > PRESETS_MAX:
                raise HttpError(400, f'at most {PRESETS_MAX} presets')
            out = []
            for p in ps:
                if not isinstance(p, dict) or not isinstance(p.get('name'), str) or not NAME_RE.fullmatch(p['name'].strip() or '-'):
                    raise HttpError(400, 'a preset needs a short name')
                out.append(dict(self.flag_set(p), name=p['name'].strip()))
            d['presets'] = out
        if 'last' in body:
            d['last'] = self.flag_set(body['last']) if body['last'] else None
        if 'appearance' in body:  # Light, Dark or System, and the terminal's look: one setting for every device
            if not isinstance(body['appearance'], dict):
                raise HttpError(400, 'appearance is an object')
            d['appearance'] = appearance(body['appearance'], d.get('appearance'))
        if 'summary_model' in body:  # who writes Start fresh summaries: Opus (the default) or Sonnet
            if body['summary_model'] not in SUMMARY_MODELS:
                raise HttpError(400, 'the summary model is opus or sonnet')
            d['summary_model'] = body['summary_model']
        for k in options_mod.KEYS:  # the opt-in settings: each one true or false
            if k in body:
                if not isinstance(body[k], bool):
                    raise HttpError(400, f'{k} is true or false')
                d[k] = body[k]
        self._save_json('prefs.json', d)
        self.fleet.summary_model = d['summary_model']
        self.fleet.options = options_mod.clean(d)
        self.bump()
        return dict(d, ok=True)

    def _remember(self, body):
        """The New sheet opens with the setup used last."""
        try:
            d = self.prefs()
            d['last'] = self.flag_set({k: body.get(k) for k in ('model', 'effort', 'mode', 'flags', 'other')})
            self._save_json('prefs.json', d)
        except Exception:
            log('could not remember the setup: ' + traceback.format_exc(limit=1))

    # ---- server-side jobs: the request is taken at once and finished here, so the app can be closed right away.
    # The machine is reached with retries, the launch is watched to the end, and a push says how it went.
    def _jobs_save(self):  # caller holds jobs_lock
        now = self.clock()
        for jid in [k for k, j in self.jobs.items() if j.get('finished') and now - j['finished'] > JOB_KEEP]:
            del self.jobs[jid]
        tmp = self.state_dir / f'jobs.{os.getpid()}.tmp'
        try:
            tmp.write_text(json.dumps(self.jobs, indent=1))
            os.replace(tmp, self.state_dir / 'jobs.json')
        except OSError:
            log('could not save jobs: ' + traceback.format_exc())

    def _set(self, jid, **kw):
        with self.jobs_lock:
            j = self.jobs[jid]
            moved = j.get('live') and any(j.get(k) != kw[k] for k in ('step', 'note', 'state') if k in kw)
            j.update(kw)
            self._jobs_save()
        if moved:
            threading.Thread(target=self._live, args=(jid,), daemon=True).start()

    LIVE_STEPS = {'sent': (0.1, 'Sending'), 'retrying': (0.1, 'Trying to reach the machine'), 'waiting': (0.15, None),
                  'choosing': (0.15, 'Picking a machine'), 'queued': (0.3, 'Queued on the machine'),
                  'handoff': (0.4, 'Writing the handoff'), 'gather': (0.2, "Gathering the thread's chats"),
                  'summary': (0.4, 'Writing the summary, about a minute'), 'starting': (0.65, 'Starting the new chat'),
                  'running': (0.7, 'Starting Claude')}

    def live_state(self, j):
        """A job as the iPhone app's Live Activity draws it (JobActivityAttributes.ContentState)."""
        if j.get('state') == 'done':
            sid = j.get('sid')
            link = f'switchboard://session/{j["machine"]}/{sid}' if sid and SID_RE.fullmatch(sid) else None
            return {'step': j.get('name') or DONE_TITLES[j['kind']].format(where=self._label(j['machine'])), 'progress': 1.0,
                    'done': True, 'failed': False, 'link': link}
        if j.get('state') == 'error':
            return {'step': (j.get('error') or 'Something went wrong.')[:120], 'progress': 1.0, 'done': False, 'failed': True, 'link': None}
        frac, word = self.LIVE_STEPS.get(j.get('step'), (0.2, None))
        return {'step': (word or j.get('note') or 'Waiting for its work to finish')[:120], 'progress': frac, 'done': False,
                'failed': False, 'link': None}

    def _live(self, jid):
        if not self.live_pusher:
            return
        with self.jobs_lock:
            j = dict(self.jobs.get(jid) or {})
        tok = j.get('live')
        if not tok:
            return
        end = j.get('state') in ('done', 'error')
        try:
            gone = self.live_pusher(tok, self.live_state(j), end, 4 * 60 if j.get('state') == 'done' else 60)
        except Exception:
            log('live activity push failed: ' + traceback.format_exc(limit=1))
            return
        if gone or end:
            with self.jobs_lock:
                if jid in self.jobs:
                    self.jobs[jid].pop('live', None)
                    self._jobs_save()

    def _recover_jobs(self):
        """After a restart: keep watching launches already handed to a machine; anything earlier cannot be known."""
        try:
            self.jobs = json.loads((self.state_dir / 'jobs.json').read_text())
        except (OSError, ValueError):
            self.jobs = {}
        now = self.clock()
        for jid, j in self.jobs.items():
            if j.get('state') in ('done', 'error'):
                continue
            if j.get('inner') and now - j.get('created', 0) < WATCH_FOR:
                threading.Thread(target=self._rewatch, args=(jid,), daemon=True).start()
            else:
                j.update(state='error', finished=now,
                         error='Switchboard restarted before this reached the machine. Check the list, then try again.')
        with self.jobs_lock:
            self._jobs_save()

    def _submit(self, kind, machine, plan, title):
        jid = 's-' + uuid.uuid4().hex[:12]
        now = self.clock()
        req = (plan or {}).get('req') or {}
        with self.jobs_lock:
            if kind in ('resume', 'fresh') and req.get('sid') and not req.get('relaunch'):
                # A second click (or a second device) while the first is still on its way gets the same job, not a copy
                for other, j in self.jobs.items():
                    r = (j.get('plan') or {}).get('req') or {}
                    if (j.get('kind') == kind and j.get('machine') == machine and r.get('sid') == req['sid']
                            and not r.get('relaunch') and j.get('state') not in ('done', 'error') and not j.get('cancel')):
                        return {'ok': True, 'job': other, 'machine': 'server'}
            self.feat.library.plan_tools(kind, machine, plan)  # the tools the chat starts with
            self.jobs[jid] = {'kind': kind, 'machine': machine, 'plan': plan, 'title': title, 'state': 'queued',
                              'step': 'sent', 'created': now, 'watched': now}
            self._jobs_save()
        threading.Thread(target=self._drive, args=(jid,), daemon=True).start()
        return {'ok': True, 'job': jid, 'machine': 'server'}

    def _reach(self, jid, name, req):
        """One machine call; an unreachable machine gets a few more tries before the job gives up. A call that
        connected but timed out is not retried: it may have queued the launch already."""
        for wait in (*self.retry_waits, None):
            try:
                return self._call(name, req)
            except HttpError as e:
                if e.code != 502 or wait is None or '(timed out)' in str(e):
                    raise
                self._set(jid, step='retrying', note=str(e))
                self.sleep(wait)

    def _watch(self, jid, name, inner, step=None):
        deadline = self.clock() + WATCH_FOR
        while True:
            try:
                r = self._call(name, {'verb': 'job', 'id': inner}, timeout=30)
                if r.get('state') in ('done', 'error'):
                    return r
                self._set(jid, state='running' if r.get('state') == 'running' else 'queued',
                          step=step or r.get('step') or r.get('state') or 'queued',
                          **({'note': r['note']} if isinstance(r.get('note'), str) else {}))
            except HttpError as e:
                if e.code != 502:
                    return {'state': 'error', 'error': str(e)}
                # the machine dropped off for a moment; the launch carries on there, so keep watching
            if self.clock() > deadline:
                return {'state': 'error', 'error': f'This is taking too long. Check {self._label(name)} in the list in a minute.'}
            self.sleep(self.poll_every)

    def _drive(self, jid):
        with self.jobs_lock:
            j = json.loads(json.dumps(self.jobs[jid]))
        try:
            if j['machine'] == 'auto':
                j = self.feat.resolve_auto(jid, j)
            plan, name = j['plan'], j['machine']
            if j['kind'] == 'continue':
                name, inner = self._continue_steps(jid, plan)
            elif j['kind'] == 'compact':
                name, inner = self._compact_steps(jid, plan)
            else:
                req = dict(plan['req'])
                if plan.get('wait_idle'):
                    self._wait_idle(jid, name, req['sid'])
                if plan.get('mkdir') is not None:
                    req['folder'] = self._reach(jid, name, {'verb': 'mkdir', 'folder': plan['mkdir']}).get('folder') or plan['mkdir']
                req = self.feat.library.ready(name, plan, req)  # tools the machine lacks go there first
                inner = self._reach(jid, name, req).get('job')
            if inner is None:  # a verb that is done when it answers (stop)
                res = {'state': 'done'}
            else:
                self._set(jid, inner=inner, machine=name, state='queued', step='queued')
                res = self._watch(jid, name, inner)
            if j['kind'] == 'continue' and res.get('state') == 'done':
                self._superseded(plan, name, res)
        except HttpError as e:
            res = {'state': 'error', 'error': str(e)}
        except Exception as e:
            log('job failed: ' + traceback.format_exc())
            res = {'state': 'error', 'error': f'Switchboard hit an error: {e}'}
        self._finish(jid, res)

    def _rewatch(self, jid):
        with self.jobs_lock:
            j = dict(self.jobs[jid])
        self._finish(jid, self._watch(jid, j['machine'], j['inner']))

    def _continue_steps(self, jid, p):
        src, dst = p['src'], p['dst']
        if p.get('wait_idle'):
            self._wait_idle(jid, src, p['sid'])
        self._set(jid, state='running', step='handoff')
        hj = self._reach(jid, src, {'verb': 'handoff', 'sid': p['sid']})['job']
        r = self._watch(jid, src, hj, step='handoff')
        if r.get('state') == 'error':
            raise HttpError(400, r.get('error') or 'the handoff did not finish')
        ht = self._reach(jid, src, {'verb': 'handoff_text', 'sid': p['sid']})
        text, carry = ht['text'], ht.get('carry') or {}
        self._set(jid, step='starting')
        folder = p['folder']
        if p.get('create'):
            folder = self._reach(jid, dst, {'verb': 'mkdir', 'folder': folder}).get('folder') or folder
        req = {'verb': 'start', 'folder': folder, 'handoff_text': text, 'from': self._label(src)}
        if p.get('topic'):
            req['topic'] = p['topic']
        if carry.get('chrome') is True:  # what the old chat could do comes along: Claude in Chrome, its effort, its model
            req['chrome'] = True
        if isinstance(carry.get('effort'), str):
            req['effort'] = carry['effort']
        if p.get('model') or isinstance(carry.get('model'), str):
            req['model'] = p.get('model') or carry['model']
        req = self.feat.library.ready(dst, p, req)  # the chat's tools come along
        return dst, self._reach(jid, dst, req)['job']

    # ---- Compact a History thread into one fresh chat
    def _thread_chats(self, name, sid):
        """The thread around a chat, oldest first, or an HttpError when there is nothing to compact."""
        t = self.feat.threads.thread(name, sid)
        chats = [c for c in (t or {}).get('chats') or [] if c.get('machine') in self.fleet.machines]
        if len(chats) < 2:
            raise HttpError(400, 'This chat has no earlier chats to compact with.')
        return chats[-COMPACT_MAX:]

    def _pieces(self, chats, measure, jid=None):
        """Each chat's piece from the machine it lives on, oldest first. A machine too old to know the verb is
        named in the error."""
        by = {}
        for c in chats:
            by.setdefault(c['machine'], []).append(c['sid'])
        got = {}
        for m, sids in by.items():
            if 'compact' not in (self.fleet.state.get(m, {}).get('features') or []):
                raise HttpError(400, f'{self._label(m)} needs the Switchboard update before it can compact a thread.')
            req = {'verb': 'thread_pieces', 'sids': sids, 'measure': measure}
            r = self._reach(jid, m, req) if jid else self._call(m, req, timeout=60)
            for p in r.get('pieces') or []:
                got[(m, p['sid'])] = p
        return [got[(c['machine'], c['sid'])] for c in chats if (c['machine'], c['sid']) in got]

    def compact(self, body):
        """POST /api/compact {machine, sid}: what compacting this chat's thread costs (nothing is written), or with
        confirm: true, the job that writes one summary of the whole thread and opens a new chat from it."""
        name, sid = self._machine(body), body.get('sid')
        if not isinstance(sid, str) or not SID_RE.fullmatch(sid):
            raise HttpError(400, 'bad session id')
        chats = self._thread_chats(name, sid)
        newest = chats[-1]
        if body.get('confirm') is not True:
            pieces = self._pieces(chats, True)
            tokens = sum(p.get('tokens') or 0 for p in pieces) + 60 * len(pieces)  # headings and the rules list
            cost = self._call(newest['machine'], {'verb': 'thread_cost', 'sids': [newest['sid']], 'tokens': tokens})
            return {'ok': True, 'chats': len(chats), 'found': len(pieces), 'notes': sum(1 for p in pieces if p.get('kind') == 'handoff'),
                    'machine': newest['machine'], 'label': self._label(newest['machine']), 'sid': newest['sid'],
                    'cost': {k: cost.get(k) for k in ('tokens', 'summary', 'first', 'total', 'each', 'text')}}
        title = self._title(newest['machine'], newest['sid'])
        return self._submit('compact', newest['machine'], {'chats': [{'machine': c['machine'], 'sid': c['sid']} for c in chats],
                                                           'req': {'sid': newest['sid']}}, title)

    def _compact_steps(self, jid, p):
        self._set(jid, state='running', step='gather')
        chats = p['chats']
        pieces = self._pieces(chats, False, jid)
        if len(pieces) < 2:
            raise HttpError(400, "Fewer than two of the thread's chats could be read. Check that their machines are online.")
        newest = chats[-1]
        self._set(jid, step='summary')
        req = self.feat.library.ready(newest['machine'], p, {'verb': 'compact', 'sid': newest['sid'], 'pieces': pieces})
        return newest['machine'], self._reach(jid, newest['machine'], req)['job']

    def handoff_text(self, name, sid):
        """GET /api/handoff/<machine>/<sid>: a chat's written summary, for History's Additional history."""
        try:
            r = self._call(name, {'verb': 'handoff_text', 'sid': sid})
        except HttpError:
            r = {}
        if not r.get('ok'):
            raise HttpError(404, 'That chat has no summary yet.')
        return {'ok': True, 'text': r.get('text') or ''}

    def _superseded(self, plan, dst, res):
        """The session now lives on dst: the source leaves its machine's list (under Replaced)."""
        try:
            self._call(plan['src'], {'verb': 'supersede', 'sid': plan['sid'], 'by': {'machine': dst, 'sid': res.get('sid') or ''}}, timeout=20)
            self._refresh_soon(plan['src'], 0.5)
        except Exception as e:
            log(f'could not mark {plan["sid"]} as continued: {e}')

    def _finish(self, jid, res):
        now = self.clock()
        keep = {k: res.get(k) for k in ('url', 'name', 'sid', 'bg_id', 'error', 'missing_tools', 'skipped_tools') if res.get(k)}
        if isinstance(res.get('tools'), list):  # what the chat was started with (an auto chat: only tools with their keys)
            keep['tools'] = res['tools']
        state = 'done' if res.get('state') == 'done' else 'error'
        if state == 'error' and not keep.get('error'):
            keep['error'] = 'Something went wrong.'
        with self.jobs_lock:
            j = self.jobs[jid]
            j.update(keep, state=state, step=state, finished=now)  # not the last watch's "queued"
            watched = now - j.get('watched', 0) <= WATCH_GRACE
            j = dict(j)
            self._jobs_save()
        if j.get('live'):
            threading.Thread(target=self._live, args=(jid,), daemon=True).start()
        self._chrome_carry(j)
        self.feat.job_done(j)
        stopped = ((j.get('plan') or {}).get('req') or {}).get('sid')
        if j.get('kind') == 'stop' and state == 'done' and isinstance(stopped, str):
            self.fleet.mark_stopped(j['machine'], stopped)  # a queued stop: off the board as soon as it runs
            self.bump()
        elif isinstance(stopped, str):  # resumed or relaunched since: running again
            self.fleet.mark_stopped(j['machine'], stopped, on=False)
        self._refresh_soon(j['machine'], 0.5)
        if watched or j.get('cancel'):  # the app is showing this job and says so itself, or the user called it off
            return
        where, what = self._label(j['machine']), j.get('name') or j.get('title') or 'The session'
        if state == 'done':
            title = DONE_TITLES[j['kind']].format(where=where)
            body = f'{what} on {where}.' + ('' if j['kind'] in ('handoff', 'stop') else ' Tap to open it.')
        else:
            title, body = FAIL_TITLES[j['kind']], f'{j.get("title") or where}: {j["error"]}'
        url = f'/s/{j["machine"]}/{j["sid"]}' if state == 'done' and j.get('sid') and SID_RE.fullmatch(j['sid']) else '/'
        if state == 'error' and j['plan'].get('req', {}).get('sid'):
            url = f'/s/{j["machine"]}/{j["plan"]["req"]["sid"]}'
        self._push(title, body[:220], url)

    def _chrome_carry(self, j):
        """A finished job's new chat gets the Chrome machine picked for it, or the one the chat it came from had."""
        plan, new = j.get('plan') or {}, j.get('sid')
        if j.get('state') != 'done' or not isinstance(new, str) or not SID_RE.fullmatch(new) or j.get('machine') not in self.fleet.machines:
            return
        pick = plan.get('chrome_machine')
        if not pick:
            old = plan.get('sid') or (plan.get('req') or {}).get('sid')
            pick = self.chrome_for(plan.get('src') or j['machine'], old) if old else None
        if pick and pick in self.fleet.machines and pick != self.chrome_for(j['machine'], new):
            try:
                self.set_chrome_for(j['machine'], new, pick)
            except HttpError:
                pass

    def job(self, name, jid):
        if not JOB_RE.fullmatch(jid or ''):
            raise HttpError(400, 'bad job id')
        if name == 'server':
            with self.jobs_lock:
                j = self.jobs.get(jid)
                if j:
                    j['watched'] = self.clock()
                    j = dict(j)
            if not j:
                raise HttpError(404, 'unknown job')
            return {'ok': True, **{k: j.get(k) for k in ('state', 'step', 'note', 'error', 'url', 'name', 'sid', 'bg_id', 'kind', 'picked', 'by',
                                                          'missing_tools')},
                    'machine': j['machine'], 'label': self._label(j['machine'])}
        r = self._call(self._machine(name), {'verb': 'job', 'id': jid})
        if r['state'] != 'queued':
            self._refresh_soon(name)
        return r

    # ---- terminals (term.py): tickets for the hub, and the machine list it reads
    def _terms_setup(self, clock):
        self.terms = term_mod.Terms(self.fleet, self.state_dir, clock=clock, origins=self.server_cfg.origins if self.server_cfg else ())
        try:
            self.terms.write_machines()
        except OSError as e:
            log(f'could not write machines.json for the hub: {e}')

    def term_ticket(self, body, host):
        try:
            return self.terms.ticket(body, host, SID_RE)
        except term_mod.NotFound as e:
            raise HttpError(404, str(e))
        except term_mod.BadRequest as e:
            raise HttpError(400, str(e))
        except OSError as e:
            log(f'terminal key: {e}')
            raise HttpError(500, 'Switchboard could not read its terminal key.')

    # ---- routes
    def get(self, path, query=None):
        r = self.feat.get(path, query)
        if r is not None:
            return r
        if path == '/api/sessions':
            return self.sessions()
        if path == '/api/machines':
            return self.machines_view()
        if path == '/api/prefs':
            return dict(self.prefs(), ok=True, flags=sorted(flags_mod.START_FLAGS), chips=flags_mod.CHIP_FLAGS,
                        takes={k: v is not None for k, v in flags_mod.START_FLAGS.items()}, aliases=flags_mod.ALIASES,
                        live=flags_mod.LIVE, labels=flags_mod.LABELS, **options_mod.view(self.options(), self.push_ready))
        if path == '/api/summary':
            return self.summary()
        if path == '/api/usage':
            return self.fleet.usage()
        if path == '/api/app':
            return self.app_latest()
        if path == '/api/flags':
            q = query or {}
            return self.flags_view((q.get('machine') or [''])[0], (q.get('sid') or [''])[0])
        m = re.fullmatch(r'/api/machines/([a-z0-9]+)/(folders|mcp|caps)', path)
        if m:
            return self.machine_extra(m.group(1), m.group(2), query)
        m = re.fullmatch(r'/api/handoff/([a-z0-9]+)/([0-9a-f-]{36})', path)
        if m:
            return self.handoff_text(self._machine(m.group(1)), m.group(2))
        m = re.fullmatch(r'/api/job/([a-z0-9]+)/([a-z0-9-]+)', path)
        if m:
            return self.job(m.group(1), m.group(2))
        m = re.fullmatch(r'/api/folders/([a-z0-9]+)', path)
        if m:
            req = {'verb': 'folders'}
            if query and 'parent' in query:  # ?parent=<folder> lists one level of the tree ('' = the root)
                req['parent'] = query['parent'][0]
            return self._call(self._machine(m.group(1)), req)
        m = re.fullmatch(r'/api/files/([a-z0-9]+)', path)
        if m:  # the file browser: ?folder=<folder under the Claude folder>[&name=<file>] (agent: list_files)
            name = self._machine(m.group(1))
            if 'files' not in (self.fleet.state.get(name, {}).get('features') or []):
                raise HttpError(409, 'Update Switchboard on this machine to browse its files.')
            req = {'verb': 'files', 'folder': (query or {}).get('folder', [''])[0]}
            if query and 'name' in query:
                req['name'] = query['name'][0]
            return self._call(name, req)
        raise HttpError(404, 'not found')

    def post(self, path, body, ua='', host=''):
        ua = ua or ''
        origin = ('iPhone' if 'SwitchboardApp' in ua else 'Android' if 'SwitchboardAndroid' in ua
                  else 'desktop' if 'SwitchboardDesktop' in ua else 'web')
        if path == '/api/term':
            return self.term_ticket(body, host)
        if path == '/api/pair':  # Add a machine: a code for switchboard-node (nodes.py)
            try:
                return nodes_mod.pair(host, cfg_dir=self.cfg_dir, origins=self.server_cfg.origins if self.server_cfg else ())
            except OSError as e:
                raise HttpError(502, f'The hub could not make a code ({e}).')
        if path == '/api/machines':
            return self.machines_post(body)
        if path == '/api/relaunch':
            return self.relaunch(body)
        if path == '/api/compact':
            return self.compact(body)
        if path == '/api/setup_fix':  # a setup problem the machine's agent can repair (sb.py setup_fix)
            name, fix = body.get('machine'), body.get('fix')
            if not isinstance(name, str) or not isinstance(fix, str):
                return {'ok': False, 'error': 'Say which machine and what to fix.'}
            r = self._call(name, {'verb': 'setup_fix', 'fix': fix}, timeout=60)
            if r.get('ok'):
                threading.Thread(target=self.fleet.poll_one, args=(name,), daemon=True).start()
            return r
        if path == '/api/live':  # a Live Activity for a server job: its updates come by push while the app sleeps
            jid, tok = body.get('job'), body.get('token')
            if not isinstance(tok, str) or not TOKEN_RE.fullmatch(tok):
                raise HttpError(400, 'bad activity push token')
            with self.jobs_lock:
                j = self.jobs.get(jid) if isinstance(jid, str) else None
                if not j:
                    raise HttpError(404, 'unknown job')
                j['live'] = tok
                done = j.get('state') in ('done', 'error')
            threading.Thread(target=self._live, args=(jid,), daemon=True).start()  # catch it up now
            return {'ok': True, 'done': done}
        if path == '/api/job/cancel':  # a relaunch, stop, move or fresh start still waiting for the chat's work to end
            jid = body.get('job')
            with self.jobs_lock:
                j = self.jobs.get(jid) if isinstance(jid, str) else None
                if not j or j.get('step') != 'waiting' or j.get('state') != 'queued':
                    raise HttpError(409, 'That is not waiting any more.')
                j['cancel'] = True
            return {'ok': True}
        if path == '/api/flags':
            return self.flags_plan(body)
        if path == '/api/refresh':
            self.fleet.poll()
            return self.sessions()
        if path == '/api/action':
            name, verb = self._machine(body), body.get('verb')
            if verb in self.feat.ACTIONS:
                return self.feat.action(name, verb, body)
            if verb == 'relaunch':  # the iPhone app's form of POST /api/relaunch
                return self.relaunch_edit(body)
            if verb not in ACTION_VERBS:
                raise HttpError(400, 'unknown action')
            sid = body.get('sid')
            if not isinstance(sid, str) or not SID_RE.fullmatch(sid):
                raise HttpError(400, 'bad session id')
            req = {'verb': verb, 'sid': sid}
            if verb == 'handoff':
                req['close'] = bool(body.get('close'))
            ends = verb in ('fresh', 'stop') or (verb == 'handoff' and req['close'])  # the chat stops or moves on
            when = body.get('when')
            if ends:
                need = self.busy_need(name, sid, when, BUSY_THEN[verb])
                if need:
                    return need
            if verb in ('pass', 'fresh', 'stop', 'dismiss'):
                self.guard.answer({'machine': name, 'session_id': sid})
            if ends and when == 'idle':  # queued: it goes once nothing is running there any more
                return self._submit(verb, name, {'req': req, 'wait_idle': True}, self._title(name, sid))
            if verb in ('resume', 'fresh', 'handoff'):
                return self._submit(verb, name, {'req': req}, self._title(name, sid))
            r = self._call(name, req)
            if verb == 'stop' and isinstance(r, dict) and r.get('ok'):
                self.fleet.mark_stopped(name, sid)  # off the board now, not at the next poll
                self.bump()
            self._refresh_soon(name)
            return r
        if path == '/api/new':
            auto = body.get('machine') == 'auto'  # v2: Claude picks the machine when the job runs (features.Auto)
            name = 'auto' if auto else self._machine(body)
            folder, model, mode = body.get('folder') or '', body.get('model') or None, body.get('mode') or 'normal'
            effort = body.get('effort') or None
            if model not in MODELS:
                raise HttpError(400, 'unknown model')
            if mode not in MODES:
                raise HttpError(400, 'unknown mode')
            if effort not in EFFORTS:
                raise HttpError(400, 'unknown effort')
            try:  # the chips and Other flags (flags.py); the machine checks them again against its own Claude Code
                fl = flags_mod.parse(body.get('flags'), body.get('other'))
            except flags_mod.FlagError as e:
                raise HttpError(400, str(e))
            for k, v in (('model', model), ('effort', effort)):
                if fl[k] and v and fl[k] != v:
                    raise HttpError(400, f'The {k} is set twice ({v} and {fl[k]} in Other flags).')
            model, effort = fl['model'] or model, fl['effort'] or effort
            req = {'verb': 'start', 'folder': folder, 'prompt': body.get('prompt') or '', 'mode': mode, 'origin': origin}
            if model:
                req['model'] = model
            if effort:
                req['effort'] = effort
            if fl['chrome'] or body.get('chrome') is True:
                req['chrome'] = True
            cm = body.get('chrome_machine') or None  # this chat's own Chrome machine (Auto sends it there)
            if cm is not None and cm not in self.fleet.machines:
                raise HttpError(400, 'unknown machine')
            if fl['flags']:
                req['flags'] = fl['flags']
            if body.get('name'):
                req['name'] = body['name']
            if not auto and fl['flags'] and 'flags' not in (self.fleet.state.get(name, {}).get('features') or []):
                raise HttpError(400, f'{self._label(name)} needs the Switchboard update before it can take these flags.')
            tools = self.feat.library.check_tools(body.get('tools'))  # the defaults join them when the job is submitted
            if tools and tools != 'auto' and not auto and not self.feat.library.has_feature(name):
                raise HttpError(409, f'Update Switchboard on {self._label(name)} to use tools from the library.')
            if auto:
                if not self.feat.auto.candidates()[0] and not (req.get('chrome') and self.feat.auto.chrome_pick(want=cm)):
                    raise HttpError(400, self.feat.auto.none_reason())
                r = self._submit('start', 'auto', {'req': req, 'auto': {'folder': folder}, 'chrome_machine': cm, 'tools': tools},
                                 'New session')
            else:
                r = self._submit('start', name, {'req': req, 'mkdir': folder if body.get('create') else None, 'chrome_machine': cm,
                                                 'tools': tools}, 'New session')
            self._remember(body)
            return r
        if path == '/api/continue':
            src, dst = self._machine(body.get('machine')), self._machine(body.get('to'))
            sid = body.get('sid')
            if not isinstance(sid, str) or not SID_RE.fullmatch(sid):
                raise HttpError(400, 'bad session id')
            model = body.get('model') or None
            if model not in MODELS:
                raise HttpError(400, 'unknown model')
            title = self._title(src, sid)
            need = self.busy_need(src, sid, body.get('when'), BUSY_THEN['continue'])
            if need:
                return need
            return self._submit('continue', dst, {'src': src, 'sid': sid, 'dst': dst, 'folder': body.get('folder') or '',
                                                  'create': bool(body.get('create')), 'model': model, 'topic': title,
                                                  'wait_idle': body.get('when') == 'idle'}, title)
        if path == '/api/event':
            ev = body.get('event')
            if body.get('machine') not in self.fleet.machines or not isinstance(body.get('session_id'), str):
                raise HttpError(400, 'bad event')
            if ev == 'ping':
                return self.ping(body)
            if ev == 'guard':
                self.guard.block(body)
            elif ev == 'guard_answer':
                self.guard.answer(body)
            self._poll_soon(body['machine'], 0.3)
            return {'ok': True}
        if path == '/api/prefs':
            return self.set_prefs(body)
        if path == '/api/chrome':  # {"machine", "sid", "chrome_machine": name|null}: one chat's Chrome machine
            name, sid = self._machine(body.get('machine')), body.get('sid')
            if not isinstance(sid, str) or not SID_RE.fullmatch(sid):
                raise HttpError(400, 'bad session id')
            self.set_chrome_for(name, sid, body.get('chrome_machine'))
            return {'ok': True}
        if path == '/api/widget-token':  # iOS 26 WidgetKit push: the widgets reload when the server says so
            tok = body.get('token')
            if not isinstance(tok, str) or not TOKEN_RE.fullmatch(tok):
                raise HttpError(400, 'bad widget push token')
            d = self.widget_tokens()
            d[tok] = {'name': str(body.get('name') or 'iPhone')[:40], 'added': time.time()}
            for old in sorted(d, key=lambda k: d[k].get('added') or 0)[:-8]:  # a reinstall makes a new token
                d.pop(old, None)
            self._save_json('widget_tokens.json', d)
            return {'ok': True}
        if path == '/api/devices':
            tok = body.get('token')
            if not isinstance(tok, str) or not TOKEN_RE.fullmatch(tok):
                raise HttpError(400, 'bad device token')
            d = self.devices()
            d[tok] = {'name': str(body.get('name') or 'iPhone')[:40], 'added': time.time()}
            for old in sorted(d, key=lambda k: d[k].get('added') or 0)[:-DEVICES_MAX]:  # never grows without limit
                d.pop(old, None)
            tmp = self.state_dir / f'devices.{os.getpid()}.tmp'
            tmp.write_text(json.dumps(d, indent=1))
            os.replace(tmp, self.state_dir / 'devices.json')
            if not self.push_ready:  # the phone is kept, so pushes start once the server is set up
                return {'ok': True, 'push': False, 'note': 'Notifications are not set up on this server. '
                        'They need an APNs key and ~/.config/switchboard/apns.json on the server.'}
            return {'ok': True, 'push': True}
        r = self.feat.post(path, body, origin)
        if r is not None:
            return r
        raise HttpError(404, 'not found')

    # ---- the web: Switchboard runs in the native apps. What is left is a page that says so, and the icons.
    def static(self, path):
        if path.startswith('/static/'):
            rel = path[len('/static/'):]
            if rel not in ('icon.svg', 'favicon-16.png', 'favicon-32.png'):
                return None
        elif path in ('/icon.svg', '/favicon.ico'):
            rel = path[1:]
        else:
            return None
        if '..' in rel.split('/') or rel.startswith('/'):
            return None
        p = (self.web_dir / rel).resolve()
        if not str(p).startswith(str(self.web_dir.resolve()) + os.sep) or not p.is_file():
            return None
        return p

    def home_page(self):
        """The page at /: Switchboard is the native apps now, with the downloads (when publish.sh said where they are)."""
        try:
            url = json.loads((self.cfg_dir / 'downloads.json').read_text()).get('url')
        except (OSError, ValueError, AttributeError):
            url = None
        ok = isinstance(url, str) and re.fullmatch(r'https?://[A-Za-z0-9.:\-]+(?:/[A-Za-z0-9._\-/]*)?', url)
        link = (f'<a class="b" href="{url.rstrip("/")}/">Download Switchboard</a>' if ok
                else '<p class="s">Ask whoever runs this server for the download link.</p>')
        return (HOME_PAGE.replace('{{LINK}}', link)).encode()


def make_handler(app):
    class H(BaseHTTPRequestHandler):
        server_version = 'Switchboard'
        protocol_version = 'HTTP/1.1'
        timeout = 60  # a client that sends nothing for this long is dropped (slow or idle sockets never hold a thread)

        def log_message(self, fmt, *args):
            pass

        def _send(self, code, data, ctype='application/json', cache='no-store'):
            if not isinstance(data, bytes):
                data = json.dumps(data).encode()
            self.send_response(code)
            self.send_header('Content-Type', ctype)
            self.send_header('Content-Length', str(len(data)))
            self.send_header('Cache-Control', cache)
            self.send_header('X-Content-Type-Options', 'nosniff')
            self.send_header('Referrer-Policy', 'no-referrer')
            if ctype.startswith('text/html'):
                self.send_header('Content-Security-Policy', CSP)
            self.end_headers()
            self.wfile.write(data)

        def _host_ok(self):
            return app.host_ok(self.headers.get('Host'))

        def _length(self):
            """Content-Length, never negative: read(-1) would read until the client stops sending."""
            v = (self.headers.get('Content-Length') or '0').strip()
            if not v.isdigit():
                raise HttpError(400, 'bad Content-Length')
            return int(v)

        def _guarded(self, fn):
            if not self._host_ok():
                return self._send(403, {'ok': False, 'error': 'forbidden host'})
            try:
                self._send(200, fn())
            except HttpError as e:
                self._send(e.code, {'ok': False, 'error': str(e)})
            except Exception:
                log('error: ' + traceback.format_exc())
                self._send(500, {'ok': False, 'error': 'server error'})

        def _stream(self):
            """Server-sent events: 'change' whenever a session or a machine changed, so pages load again at once."""
            if not self._host_ok():
                return self._send(403, {'ok': False, 'error': 'forbidden host'})
            if not app.stream_slot(True):
                return self._send(503, {'ok': False, 'error': 'too many live pages'})
            try:
                self.send_response(200)
                self.send_header('Content-Type', 'text/event-stream')
                self.send_header('Cache-Control', 'no-store')
                self.send_header('X-Accel-Buffering', 'no')
                self.send_header('Connection', 'close')
                self.end_headers()
                self.close_connection = True
                v = app.version
                self.wfile.write(f'retry: 2000\nevent: hello\ndata: {json.dumps({"v": v})}\n\n'.encode())
                self.wfile.flush()
                end = time.time() + STREAM_FOR
                while time.time() < end:
                    nv = app.wait_change(v, STREAM_BEAT)
                    if nv == v:
                        self.wfile.write(b': beat\n\n')
                    else:
                        time.sleep(0.12)  # a burst of changes goes out as one
                        v = app.version
                        self.wfile.write(f'event: change\ndata: {{"v": {v}}}\n\n'.encode())
                    self.wfile.flush()
            except OSError:
                pass
            finally:
                app.stream_slot(False)

        def do_GET(self):
            u = urlsplit(self.path)
            path = u.path
            if path == '/api/stream':
                return self._stream()
            if path.startswith('/api/'):
                return self._guarded(lambda: app.get(path, parse_qs(u.query, keep_blank_values=True)))
            if not self._host_ok():
                return self._send(403, {'ok': False, 'error': 'forbidden host'})
            if path == '/':
                return self._send(200, app.home_page(), 'text/html; charset=utf-8', 'no-cache')
            p = app.static(path)
            if not p:
                return self._send(404, b'not found', 'text/plain')
            self._send(200, p.read_bytes(), CTYPES.get(p.suffix, 'application/octet-stream'),
                       'no-cache')

        def do_POST(self):
            u = urlsplit(self.path)
            path = u.path

            def upload():
                ctype = self.headers.get('Content-Type') or ''
                if self.headers.get('X-Switchboard') == '1' and 'application/json' in ctype:
                    # the iPhone app: {machine, sid, name, data: base64}, inside the usual 1 MB body
                    n = self._length()
                    if n > 1_000_000:
                        raise HttpError(413, 'That file is too big to send from the phone.')
                    try:
                        b = json.loads(self.rfile.read(n) or b'{}')
                        raw = base64.b64decode(b.get('data') or '', validate=True)
                    except (ValueError, AttributeError, TypeError):
                        raise HttpError(400, 'bad file')
                    if not isinstance(b, dict) or not isinstance(b.get('machine'), str):
                        raise HttpError(400, 'bad file')
                    return app.upload({'machine': [b['machine']], 'name': [str(b.get('name') or 'file')[:120]]}, raw)
                if self.headers.get('X-Switchboard') != '1' or 'application/octet-stream' not in ctype:
                    raise HttpError(403, 'missing X-Switchboard header')
                n = self._length()
                if n > UPLOAD_MAX:
                    raise HttpError(413, f'That file is {_mb(n)}. Files up to {_mb(UPLOAD_MAX)} can be attached.')
                return app.upload(parse_qs(u.query, keep_blank_values=True), self.rfile.read(n) if n else b'')
            if path == '/api/upload':
                return self._guarded(upload)

            def run():
                if self.headers.get('X-Switchboard') != '1' or 'application/json' not in (self.headers.get('Content-Type') or ''):
                    raise HttpError(403, 'missing X-Switchboard header')
                n = self._length()
                if n > 1_000_000:
                    raise HttpError(413, 'too large')
                try:
                    body = json.loads(self.rfile.read(n) or b'{}')
                except ValueError:
                    raise HttpError(400, 'bad json')
                if not isinstance(body, dict):
                    raise HttpError(400, 'bad json')
                return app.post(path, body, self.headers.get('User-Agent') or '', self.headers.get('Host') or '')
            self._guarded(run)
    return H


def make_server(host, port, app):
    s = ThreadingHTTPServer((host, port), make_handler(app))
    s.daemon_threads = True
    return s


def apns_key_path(cfg, conf):
    """apns.json's "key" (a file name under <config>/apns), else the name Apple gives it: AuthKey_<key_id>.p8."""
    name = conf.get('key') or f'AuthKey_{conf["key_id"]}.p8'
    return Path(cfg) / 'apns' / Path(str(name)).name


def main():
    from apns import Apns
    home = Path.home()
    state = home / '.local' / 'share' / 'switchboard'
    cfg = home / '.config' / 'switchboard'
    apns = None
    try:
        conf = json.loads((cfg / 'apns.json').read_text())
        apns = Apns(apns_key_path(cfg, conf), conf['key_id'], conf['team'], conf['topic'], sandbox=conf.get('sandbox', True))
    except Exception as e:
        log(f'push disabled: {e}')

    app_ref = {}

    def pusher(title, body, url):
        if not apns:
            return
        app = app_ref['app']
        devs = app.devices()
        for tok in list(devs):
            status, resp = apns.push(tok, title, body, url)
            log(f'apns {tok[:8]} -> {status} {resp[:120]}')
            if status == 410 or 'BadDeviceToken' in resp or 'Unregistered' in resp:
                devs.pop(tok, None)
                (app.state_dir / 'devices.json').write_text(json.dumps(devs, indent=1))

    server_cfg = ServerConfig(cfg, state)
    fleet_cfg = fleet_mod.FleetConfig(cfg / 'fleet.json', seed=state / 'machines.json')
    app = App(fleet_mod.Fleet(fleet_cfg.load(), transport=nodes_mod.transport), state, Path(__file__).resolve().parent / 'web',
              pusher=pusher, cfg_dir=cfg, server_cfg=server_cfg)
    app_ref['app'] = app
    app.push_ready = bool(apns)

    def widget_pusher():
        toks = app.widget_tokens()
        for tok in list(toks):
            status, resp = apns.push_widgets(tok)
            log(f'widget push {tok[:8]} -> {status} {resp[:120]}')
            if status == 410 or 'BadDeviceToken' in resp or 'Unregistered' in resp:
                toks.pop(tok, None)
                app._save_json('widget_tokens.json', toks)
    if apns:
        app.widget_pusher = widget_pusher

        def live_pusher(tok, state, end, dismiss_after):
            status, resp = apns.push_activity(tok, state, end=end, dismiss_after=dismiss_after)
            log(f'live activity push {tok[:8]} -> {status} {resp[:120]}')
            return status == 410 or 'BadDeviceToken' in resp or 'Unregistered' in resp
        app.live_pusher = live_pusher
    import bonjour  # noqa: E402  the iPhone app finds the server on the network
    bonjour.announce(PORT, server_cfg, log)

    def poller():
        while True:
            try:
                nodes_mod.sync(app.fleet)  # machines linked through the hub (switchboard-node)
                app.fleet.poll()
                threading.Thread(target=app.feat.after_poll, daemon=True).start()  # stops and probes never hold up a poll
            except Exception:
                log('poll failed: ' + traceback.format_exc())
            time.sleep(20)
    threading.Thread(target=poller, daemon=True).start()

    def linker():  # a machine that links through the hub shows within 2 s, not at the next poll
        while True:
            time.sleep(2)
            try:
                nodes_mod.sync(app.fleet)
            except Exception:
                log('hub sync failed: ' + traceback.format_exc())
    threading.Thread(target=linker, daemon=True).start()
    def bind(host):  # Tailscale can come up after us: keep trying
        while True:
            try:
                srv = make_server(host, PORT, app)
                log(f'switchboard on {host}:{PORT}')
                srv.serve_forever()
            except OSError as e:
                log(f'cannot bind {host} yet: {e}')
                time.sleep(15)
    for host in server_cfg.bind[1:]:
        threading.Thread(target=bind, args=(host,), daemon=True).start()
    bind(server_cfg.bind[0])


if __name__ == '__main__':
    main()
