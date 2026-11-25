"""Talks to the machines: one forced-command SSH call per request, JSON in and out."""
import json, os, re, subprocess, threading, time
from concurrent.futures import ThreadPoolExecutor
from datetime import datetime, time as dtime, timedelta
from pathlib import Path, PurePosixPath, PureWindowsPath

import durable
import options as options_mod  # what a machine says about its compact window, checked

GUARD_CTX = 150_000
STALE_CLOSED = 2 * 86400  # a chat nothing has open, untouched for 2 days, moves to Older
STALE_OPEN = 7 * 86400    # one still open in a terminal or the background, after a week
STALLED = 6 * 3600        # "busy" with no transcript writes for 6 hours is a stuck status, not work
KEY = '~/.ssh/switchboard_ed25519'
WEEK = 7 * 86400
SAMPLE_EVERY = 15 * 60    # the weekly percent is written down at most this often
RESET_SLACK = 3600        # a weekly reset that moves more than this is a new week
FIVE_HOURS = 5 * 3600
LIMITS_STEP = 300         # how far an old agent's "numbers seen at" moves before the apps are told again
PCT_MOST = 200            # a percent past this is not a reading at all; one a little over 100 reads as 100
ACCOUNT_EVERY = 15 * 60   # Claude Code's own usage report is asked for this often while chats reply,
ACCOUNT_IDLE = 3600       # this often while nothing here moves,
ACCOUNT_FRESH = 60        # and Refresh takes one no older than this
ACCOUNT_FEATURE = 'account_usage'
GAP = 3 * 3600            # samples further apart than this, with usage growing between them, leave a day unknown
SPEND_FEATURE = 'spend'   # an agent that can say what each chat used since the week began (agent/sb.py spend_report)
SPEND_EVERY = 120         # each machine is asked this often; every poll while it is still reading its transcripts
SPEND_TOP = 10            # chats GET /api/usage lists under `top`
SPEND_CHATS = 60          # chats kept of one machine's answer (the agent sends 40)
def server_tz():
    """The time zone the week's days are counted in: SWITCHBOARD_TZ (set it when the server runs in UTC but you do
    not), else the server's own zone (/etc/localtime, or TZ), else UTC."""
    if os.environ.get('SWITCHBOARD_TZ'):
        return os.environ['SWITCHBOARD_TZ']
    try:
        real = os.path.realpath('/etc/localtime')
        if 'zoneinfo/' in real:
            return real.split('zoneinfo/', 1)[1]
    except OSError:
        pass
    tz = os.environ.get('TZ', '').lstrip(':')
    return tz if re.fullmatch(r'[A-Za-z_]+(?:/[A-Za-z0-9_+-]+)*', tz or '') else 'UTC'


DAY_TZ = server_tz()

# The machines come from one config file (FleetConfig): no machine is built in. A machine is {name, label, target,
# os, form, root}: name is its fixed id, label the user's own name for it, target "user@host" (ssh) or "node:<name>"
# (linked through the hub), os and form what the machine itself reports (they pick its icon), root the folder new
# sessions start in.
NAME_RE = re.compile(r'[a-z0-9]{1,32}')
LABEL_RE = re.compile(r"[^\x00-\x1f<>]{1,40}")
OSES = ('mac', 'windows', 'linux')
FORMS = ('laptop', 'desktop')
MACHINES_MAX = 32


class FleetError(Exception):
    maybe_delivered = False  # True when the machine may have got the request although no answer came back


SLOW_POLLS = 3  # polls in a row that reach a machine and get no answer before it is called offline


class MaybeDelivered(OSError):
    """A call that left this server but got no answer: the machine may have acted on it (a hub link that timed out
    or dropped mid-call, an ssh session that hung). Never sent again by itself: a second copy of a launch is a
    second chat."""


# ssh's own words for a session that was up and then went away: the request may already be running there
SSH_MIDWAY = ('closed by remote host', 'Broken pipe', 'not responding', 'Connection reset')
# ...unless it says so about the hello, before anyone logged in: nothing was sent, so trying again is safe
SSH_BEFORE = ('kex_exchange_identification', 'ssh_exchange_identification', 'banner exchange')


def ssh_midway(msg):
    """Did ssh's last words say the session went away after it was up (the request may have been delivered)?"""
    return any(w in msg for w in SSH_MIDWAY) and not any(w in msg for w in SSH_BEFORE)


def valid_target(t):
    """An ssh destination (user@host or host) that ssh cannot read as an option, or a linked machine's node:<name>.
    The same rule as the hub's (native/crates/sb-hub/src/machines.rs)."""
    if not isinstance(t, str) or not t or len(t) > 255 or t.startswith('-'):
        return False
    if t.startswith('node:'):
        return bool(NAME_RE.fullmatch(t[5:]))
    ok = lambda s: bool(s) and not s.startswith('-') and all(c.isascii() and (c.isalnum() or c in '._-:[]%') for c in s)
    u, at, h = t.partition('@')
    return ok(u) and ok(h) if at else ok(t)


def clean_machine(m):
    """One machine from a file or a request, checked; None when it cannot be used."""
    if not isinstance(m, dict) or not isinstance(m.get('name'), str) or not NAME_RE.fullmatch(m['name']):
        return None
    if not valid_target(m.get('target')):
        return None
    label = m.get('label') if isinstance(m.get('label'), str) and LABEL_RE.fullmatch(m['label'].strip() or '\x00') else None
    out = {'name': m['name'], 'label': (label or m['name']).strip(), 'target': m['target'],
           'os': m.get('os') if m.get('os') in OSES else ''}
    if m.get('form') in FORMS:
        out['form'] = m['form']
    if isinstance(m.get('root'), str) and 0 < len(m['root']) <= 400 and '\x00' not in m['root']:
        out['root'] = m['root']
    if m.get('roam') is False:  # chats stay in the Claude folder; otherwise they may start anywhere in home
        out['roam'] = False
    return out


def check_root(text, windows=False):
    """A Claude folder as Settings sends it: absolute or home-relative ('~', '~/Claude'), with no '..' and no NUL.
    Returns it tidied, or raises FleetError. The agent checks it again the same way (agent/sb.py check_root)."""
    if not isinstance(text, str) or not 0 < len(text.strip()) <= 400 or '\x00' in text:
        raise FleetError('Pick a Claude folder, like ~/Claude.')
    t = text.strip()
    parts = re.split(r'[\\/]+', t)
    if '..' in parts:
        raise FleetError('The Claude folder can\'t use "..".')
    if t == '~' or t.startswith(('~/', '~\\')):
        rest = [p for p in parts[1:] if p not in ('', '.')]
        return '~' + ('/' + '/'.join(rest) if rest else '')
    if windows:
        if not re.match(r'^[A-Za-z]:[\\/]', t) and not t.startswith('\\\\'):
            raise FleetError('The Claude folder has to be a full path, like C:\\Users\\you\\Claude or ~/Claude.')
        return str(PureWindowsPath(t))
    if not t.startswith('/'):
        raise FleetError('The Claude folder has to be a full path, like ~/Claude or /Users/you/Claude.')
    return str(PurePosixPath(t))


def log(msg):
    print(time.strftime('%Y-%m-%d %H:%M:%S'), msg, flush=True)


def helper_servers(origins, keys='off', port=8130):
    """Where each computer's helper reports: the poll's "servers" (docs/plans/2026-11-22-keys-phase-1-2.md, 3.8).

    With keys off this is the server's own addresses, as it always was. With keys on ("both" or "only"), this
    computer's own address comes first: on the home computer that is the server itself, and on a paired computer it
    is the node's forwarder into the tunnel. A computer with neither is refused there at once, and its helper goes
    on to the next address. In "only" the addresses on the network are left out: nothing listens on them."""
    origins = [o for o in origins or [] if isinstance(o, str)]
    if keys not in ('both', 'only'):
        return origins
    own = f'http://127.0.0.1:{port}'
    return [own] + ([] if keys == 'only' else [o for o in origins if o.rstrip('/') != own])


def icon_of(m):
    """The machine's icon, from what it is (never from its name): a laptop, a desktop Mac, or a PC tower."""
    if m.get('form') == 'laptop':
        return 'laptop'
    if m.get('os') in ('windows', 'linux'):
        return 'tower'
    return 'desktop'


class FleetConfig:
    """The machine list, in the user's order, kept in ~/.config/switchboard/fleet.json. Settings and Add a machine
    both write here. On the first start it is seeded from the machines.json an earlier server wrote for the hub, so
    machines that were already set up keep working; with neither file, the list starts empty (Add a machine)."""

    def __init__(self, path, seed=None):
        self.path, self.seed = Path(path), Path(seed) if seed else None
        self.lock = threading.Lock()

    @staticmethod
    def _read(path):
        try:
            d = json.loads(Path(path).read_text(encoding='utf-8'))
        except (OSError, ValueError):
            return None
        rows = FleetConfig._rows(d)
        return None if rows is None else FleetConfig._clean(rows)

    @staticmethod
    def _clean(rows):
        out, seen = [], set()
        for r in rows:
            m = clean_machine(r)
            if m and m['name'] not in seen:
                seen.add(m['name'])
                out.append(m)
        return out

    @staticmethod
    def _rows(d):
        rows = d if isinstance(d, list) else d.get('machines') if isinstance(d, dict) else None
        return rows if isinstance(rows, list) else None

    def load(self):
        """The machines. Seeds the file the first time (ssh machines only: linked ones come back from the hub).

        A file that is there and cannot be read is not a first time (durable.load). Its last good copy is used, with
        every machine kept to its Claude folder: the copy may be from before "Anywhere in home" was turned off.
        With no copy, the machines still come from the hub's file so they keep working, but that file knows nothing
        of folders or of "Anywhere in home", so they are kept to their Claude folders too, and the list is not
        written down until someone changes it in Settings."""
        # keep: Settings may never be saved again, so a good start is when the copy is made
        d, status = durable.load(self.path, None, (dict, list), check=lambda d: self._rows(d) is not None, keep=True)
        if status == durable.OK:
            return self._clean(self._rows(d))
        if status == durable.RECOVERED:
            rows = [dict(m, roam=False) for m in self._clean(self._rows(d))]
            if durable.unopened(self.path):
                log('fleet.json could not be opened: its last good copy is in use, and the file is left as it is. '
                    'Each machine is kept to its Claude folder ("Anywhere in home" off) until the file opens again.')
                return rows
            log('fleet.json could not be read: its last good copy is in use, and the damaged file is kept beside it. '
                'Each machine is kept to its Claude folder ("Anywhere in home" off) until you turn it on again in Settings.')
            try:
                self.save(rows)  # written down, so a restart does not turn it back on
            except OSError:
                pass
            return rows
        seeded = [m for m in (self._read(self.seed) or []) if not m['target'].startswith('node:')] if self.seed else []
        if status == durable.BROKEN:
            log('fleet.json could not be read and has no good copy. The machines the hub knows are kept, each in its own '
                'Claude folder ("Anywhere in home" off) until you set them again in Settings.')
            return [dict(m, roam=False) for m in seeded]
        try:
            self.save(seeded)
        except OSError:
            pass
        return seeded

    def save(self, machines, force=False):
        """force: a change the user made, which may start a list that was lost over again (durable.put).
        The copy beside it is of this list, not the one before: a removed machine is in neither."""
        with self.lock:
            self.path.parent.mkdir(parents=True, exist_ok=True)
            durable.put(self.path, json.dumps({'machines': [clean_machine(m) for m in machines if clean_machine(m)]}, indent=1),
                        tmp=self.path.with_name(f'.{self.path.name}.{os.getpid()}.tmp'), fresh=True, force=force)


CHILD_WORK = 15 * 60      # a background chat whose turn is over still works while its subagents or commands wrote this recently
PARKED = 24 * 3600        # waiting on you longer than this, a chat is parked: muted, out of the badges
ACTIVITIES = ('held', 'needs', 'parked', 'working', 'stalled', 'quiet')
CACHES = ('warm', 'cold', 'unknown')


def _num(x):
    return float(x) if isinstance(x, (int, float)) and not isinstance(x, bool) and x > 0 else None


def derive(s, now, skew=0.0, held=False, hook=None, updated=None):
    """The one answer every app shows for a chat (docs/specs/2026-10-14-live-state.md): `activity`, `cache`,
    `cold_at` and `heavy`, from the agent's raw signals, the guard's held messages and the latest hook ping.

    s: the session as its machine reported it. skew: seconds to add to the machine's timestamps to get the server's
    clock. hook: a ping newer than the machine's last answer ({'why': 'prompt'|'stop'|'wait', 'at', 'cold_at'}).
    updated: when the machine answered (server clock), for an older agent's cache_left. Returns the four fields."""
    t = lambda x: (_num(x) + skew) if _num(x) else None
    status, state, kind, turn = s.get('status'), s.get('state'), s.get('kind'), s.get('turn_open')
    active = t(s.get('active')) or t(s.get('mtime')) or now
    # Not the transcript's mtime when there is better: an idle Claude Code touches its transcript about hourly with
    # nothing new in it, which kept a chat that did nothing for a day from ever looking stalled.
    said = max(t(s.get('active')) or 0, t(s.get('last_ts')) or 0) or t(s.get('mtime')) or 0
    progress = max(said, t(s.get('child_ts')) or 0)
    gone = status in ('stopped', 'done')
    waiting = not gone and (status == 'waiting' or (state == 'blocked' and status not in (None, 'busy')))
    working = not gone and (status == 'busy' or state == 'running' or turn is True)
    if turn is False and kind == 'interactive':
        working = False  # Claude Code says busy while a terminal chat's background shells run, though its turn is over
    elif turn is False and kind == 'background' and not (t(s.get('child_ts')) and now - t(s.get('child_ts')) < CHILD_WORK):
        # the same for a background chat: a server it started and left running keeps it "busy" for days. Its
        # subagents or commands still writing keep it working.
        working = False
    why = (hook or {}).get('why')
    if why == 'prompt' and not gone:
        waiting, working = False, True
        progress = max(progress, hook.get('at') or 0)
    elif why == 'stop':
        waiting, working = False, False
    elif why == 'wait' and not gone:
        waiting = True
    if held:
        activity = 'held'
    elif waiting:
        activity = 'parked' if now - active > PARKED else 'needs'
    elif working:
        activity = 'stalled' if progress and now - progress > STALLED else 'working'
    else:
        activity = 'quiet'
    if hook and _num(hook.get('cold_at')):
        cold_at = hook['cold_at']
    elif 'cache_expires_at' in s:
        cold_at = t(s.get('cache_expires_at'))
    elif _num(s.get('cache_left')) and _num(updated):  # an older agent: its seconds left when it answered
        cold_at = updated + s['cache_left']
    else:
        cold_at = None
    if cold_at:
        cache = 'warm' if cold_at > now else 'cold'
    elif 'cache_expires_at' not in s and s.get('cold'):
        cache = 'cold'  # an older agent knew it was cold, but not since when
    else:
        cache = 'unknown'
    return {'activity': activity, 'cache': cache, 'cold_at': round(cold_at, 3) if cold_at else None,
            'heavy': (s.get('ctx') or 0) >= GUARD_CTX}


def mark_age(s, now):
    """Older (stale) sessions stay out of the way by default; stalled ones say busy, but nothing has happened for hours."""
    quiet = now - max(s.get('mtime') or 0, s.get('active') or 0)
    stalled = s['activity'] == 'stalled' if 'activity' in s else s.get('status') == 'busy' and quiet > STALLED
    is_open = bool(s.get('kind')) and s.get('status') not in (None, 'stopped') and not stalled
    idle = now - (s.get('active') or s.get('mtime') or now)
    return dict(s, stale=idle > (STALE_OPEN if is_open else STALE_CLOSED), stalled=stalled)


def ssh_transport(target, req, timeout):
    import os
    try:
        r = subprocess.run(['ssh', '-T', '-i', os.path.expanduser(KEY), '-o', 'BatchMode=yes', '-o', 'IdentitiesOnly=yes',
                            '-o', 'ConnectTimeout=5', '-o', 'ServerAliveInterval=5', target],
                           input=json.dumps(req), capture_output=True, timeout=timeout,
                           # not the server's own encoding: under LANG=C one accent in a chat title failed the machine
                           encoding='utf-8', errors='replace')
    except subprocess.TimeoutExpired:
        raise MaybeDelivered('timed out')
    out = r.stdout.strip()
    if not out:
        err = (r.stderr or '').strip().splitlines()
        msg = err[-1] if err else f'ssh exited {r.returncode}'
        raise (MaybeDelivered if ssh_midway(msg) else OSError)(msg)
    try:
        return json.loads(out.splitlines()[-1])
    except ValueError:  # it ran, and said something that is not an answer
        raise MaybeDelivered('its answer could not be read')


class Fleet:
    def __init__(self, machines=(), transport=ssh_transport, clock=time.time):
        self.clock = clock
        machines = [m for m in (clean_machine(x) or (x if isinstance(x, dict) and x.get('name') else None) for x in machines) if m]
        self.machines = {m['name']: dict(m) for m in machines}
        self.order = [m['name'] for m in machines]
        self.transport = transport
        self.state = {n: self._blank() for n in self.order}
        self.lock = threading.Lock()
        self.history = None  # a WeekHistory, attached by the server: the week's samples for the iPhone widgets
        self.unlinked = set()  # linked machines the hub no longer lists (nodes.sync): not asked until it does again
        self._account_busy = threading.Lock()  # one Refresh of the account's usage report at a time
        self._held = {}  # machine: the highest weekly number an agent from before merged limits has shown (limits)
        self.on_change = None  # called with a machine's name when what it reports changed (the server's live updates)
        self.on_save = None  # called with the machine list after it changed (the server writes fleet.json and the hub's file)
        self.by_user = True  # whether the change being saved is one the user made (False: a machine reported it)
        self.closed = False  # fleet.json was lost: machines that link from now on are kept to their Claude folders
        self.servers = []  # the server's own addresses, told to each machine's agent (where its hooks report)
        self.keys = 'off'  # server.json's "keys" (keys.py): with keys on, helpers report to 127.0.0.1 first
        self.port = 8130   # the server's own port, for that address
        self.budget_view = None  # set by the server: the usage budget's status, sent with every poll (budget.py)
        self.options = None  # set by the server: the opt-in settings, sent with every poll (options.py)
        self.compact_at = None  # ... and the compact window that goes with setup_compact_window
        self._digest = {}
        self._late = {}  # machine: polls in a row that reached it and got no answer in time (_slow)
        # What each chat used this week ("Most used this week"): the week's start as the last poll knew it, each
        # machine's last answer (kept for the week, so a machine that is offline still counts), and what was asked.
        self._since, self.spend, self._spend_asked, self.spend_file = None, {}, {}, None
        self._spend_saving = threading.Lock()  # one writer of the file at a time, each writing what is true then
        self._save_lock = threading.Lock()  # saves go out one at a time, each with the list as it is then
        self._stopped = {}  # (machine, sid): when Stop succeeded; shown stopped at once, not at the next poll

    STOP_HOLD = 20  # seconds a stop shows before the machine's own list agrees (claude stop can lag a poll)

    def mark_stopped(self, name, sid, on=True):
        """A chat was just stopped (or, on=False, started again): the board shows it at once."""
        with self.lock:
            now = self.clock()
            for k in [k for k, t in self._stopped.items() if now - t > self.STOP_HOLD]:  # chats that never came back
                del self._stopped[k]
            if on:
                self._stopped[(name, sid)] = now
            else:
                self._stopped.pop((name, sid), None)

    def _overlay(self, n, s, now):
        t = self._stopped.get((n, s.get('sid')))
        if t is None:
            return s
        if now - t > self.STOP_HOLD or s.get('status') in ('stopped', 'done') or \
                (not s.get('kind') and self.state[n].get('agents_ok') is not False):
            self._stopped.pop((n, s.get('sid')), None)
            return s
        return dict(s, status='stopped')

    @staticmethod
    def _blank():
        return {'online': None, 'error': None, 'sessions': [], 'replaced': [], 'limits': None, 'account': None, 'updated': None, 'as_of': None, 'skew': 0.0, 'root': None,
                'launcher': None, 'mem': None, 'features': [], 'versions': None, 'models': None, 'setup': [], 'root_set': None, 'root_suggest': None, 'agents_ok': True, 'boot': None}

    def config(self):
        """The machine list as saved: in order, without what the machines report."""
        with self.lock:
            return [dict(self.machines[n]) for n in self.order]

    def _saved(self, by_user=True):
        if self.on_save:
            with self._save_lock:  # two quick edits: the later save always carries both
                self.by_user = by_user
                try:
                    self.on_save(self.config())
                except Exception:
                    pass
                finally:
                    self.by_user = True

    def add_machine(self, m, save=True, by_user=True):
        """A machine added in Settings, or linked through the hub (nodes.py): polled from the next round on. One that is
        already on the list keeps its name for it (the user may have renamed it). True when something changed."""
        m = clean_machine(dict(m, roam=False) if self.closed and not by_user and isinstance(m, dict) else m)
        if m is None:
            raise FleetError('That machine needs a short id (a-z, 0-9) and an address.')
        with self.lock:
            if m['name'] in self.machines:
                return False
            if len(self.order) >= MACHINES_MAX:
                raise FleetError(f'Switchboard can keep up to {MACHINES_MAX} machines.')
            self.state[m['name']] = self._blank()
            self.machines = dict(self.machines, **{m['name']: m})
            self.order = self.order + [m['name']]
        if save:
            self._saved(by_user)
        return True

    def drop_machine(self, name, save=True):
        """A machine removed in Settings, or unpaired from the hub, leaves the list."""
        with self.lock:
            if name not in self.machines:
                return False
            self.order = [n for n in self.order if n != name]
            self.machines = {k: v for k, v in self.machines.items() if k != name}
            self.state.pop(name, None)
            self._digest.pop(name, None)
            self.unlinked.discard(name)
        if save:
            self._saved()
        return True

    def rename(self, name, label):
        label = label.strip() if isinstance(label, str) else ''
        if not LABEL_RE.fullmatch(label or '\x00'):
            raise FleetError('A name has to be 1 to 40 characters.')
        return self._edit(name, label=label)

    def set_root(self, name, root):
        """The machine's Claude folder, checked for its own system: Windows takes C:\\... and \\\\server\\share,
        the others take /..., and every system takes ~ and ~/...."""
        with self.lock:
            os_ = (self.machines.get(name) or {}).get('os')
        return self._edit(name, root=check_root(root, windows=os_ == 'windows'))

    def set_roam(self, name, on):
        """Whether chats on the machine may start anywhere in its home folder, or only in its Claude folder."""
        if not isinstance(on, bool):
            raise FleetError('roam is true or false')
        with self.lock:
            old = self.machines.get(name)
            if old is None:
                return False
            new = {k: v for k, v in old.items() if k != 'roam'}
            if not on:
                new['roam'] = False
            if new == old:
                return False
            self.machines = dict(self.machines, **{name: new})
        self._saved()
        return True

    def _edit(self, name, **kw):
        with self.lock:
            old = self.machines.get(name)
            if old is None:
                raise FleetError('unknown machine')
            if all(old.get(k) == v for k, v in kw.items()):
                return False
            self.machines = dict(self.machines, **{name: dict(old, **kw)})
        self._saved()
        return True

    def reorder(self, names):
        with self.lock:
            if not isinstance(names, list) or not all(isinstance(n, str) for n in names) or sorted(names) != sorted(self.order):
                raise FleetError('The new order has to name every machine once.')
            if names == self.order:
                return False
            self.order = list(names)
        self._saved()
        return True

    def learn(self, name, r):
        """What a machine says about itself (its OS, laptop or desktop, the folder its sessions start in) is kept with
        it, so its icon and folder are right even while it is offline. A folder set in Settings is never replaced."""
        info = {}
        if r.get('os') in OSES:
            info['os'] = r['os']
        dev = r.get('device') if isinstance(r.get('device'), dict) else {}
        if dev.get('os') in OSES:
            info['os'] = dev['os']
        if dev.get('form') in FORMS:
            info['form'] = dev['form']
        with self.lock:
            old = self.machines.get(name)
            if old is None:
                return False
            # an agent from before fleet.json says nothing of where its folder came from: the folder it uses is it
            if not old.get('root') and isinstance(r.get('root'), str) and r['root'] and r.get('root_set') is not False:
                info['root'] = r['root']
            new = dict(old, **info)
            if new == old:
                return False
            self.machines = dict(self.machines, **{name: new})
        self._saved(by_user=False)
        return True

    def _list_req(self, m):
        """The poll. It tells the agent the name the server knows it by, its folder and where to report, so nothing
        about a machine has to be set up on the machine itself."""
        req = {'verb': 'list', 'as': m['name']}
        if m.get('root'):
            req['root'] = m['root']
        req['roam'] = m.get('roam') is not False
        servers = helper_servers(self.servers, getattr(self, 'keys', 'off'), getattr(self, 'port', 8130))
        if servers:
            req['servers'] = servers
        if getattr(self, 'summary_model', None):
            req['summary_model'] = self.summary_model
        if isinstance(getattr(self, 'options', None), dict):  # the opt-in settings (options.py), each true or false
            req['options'] = dict(self.options)
            if getattr(self, 'compact_at', None):  # where long chats compact; an older helper knows only 300k
                req['compact_at'] = self.compact_at
        view = getattr(self, 'budget_view', None)  # the usage budget's status, cached by the agent for its hooks
        if view:
            try:
                b = view()
            except Exception:
                b = None
            if b:
                req['budget'] = b
        age = self._account_ask(m['name'])
        if age:
            req['usage'] = {'max_age': age}  # this machine's turn to ask Claude Code for the account's usage report
        since = self._spend_ask(m['name'])
        if since:
            req['spend'] = {'since': since}  # what each chat used since the week began, on the machine's own clock
        return req

    def keep_spend(self, path):
        """Keep the machines' answers in this file, so a restart does not forget a machine that is offline."""
        self.spend_file = Path(path)
        d, status = durable.load(self.spend_file, None, dict, keep=True)
        if status in (durable.RECOVERED, durable.BROKEN):
            log('spend.json could not be read: ' + ('its last good copy is in use' if d is not None else 'the machines are asked again'))
        if isinstance(d, dict):
            with self.lock:
                for n, r in d.items():
                    r = clean_spend(r)
                    if r and 'total' in r and n in self.machines:
                        self.spend.setdefault(n, r)

    def _spend_ask(self, name):
        """The week's start on this machine's clock when it is its turn to say what its chats used, else None: every
        SPEND_EVERY, at once when the week changed, and at every poll while it is still reading its transcripts."""
        now = self.clock()
        with self.lock:
            st = self.state.get(name) or {}
            week = self._since
            if not week or SPEND_FEATURE not in (st.get('features') or []):
                return None
            last = self._spend_asked.get(name)
            if last and last['week'] == week and not last.get('more') and 0 <= now - last['at'] < SPEND_EVERY:
                return None
            self._spend_asked[name] = {'week': week, 'at': now}
            return round(week - (st.get('skew') or 0.0), 1)

    def _spend_got(self, name, said, now):
        """A machine's answer to the ask above. One that is still reading says only `more`: what it said before stands."""
        r = clean_spend(said)
        with self.lock:
            asked = self._spend_asked.get(name)
            if r is None or not asked or name not in self.machines:
                return
            if 'total' not in r:
                asked['more'] = bool(r.get('more'))
                return
            new = dict(r, since=asked['week'], at=now)  # the week as the server counts it, not the machine's clock
            old = self.spend.get(name)
            self.spend[name] = new
            same = old and {k: v for k, v in old.items() if k != 'at'} == {k: v for k, v in new.items() if k != 'at'}
            keep = not same and self.spend_file is not None
        if keep:
            # Machines answer side by side. Each writer takes its copy only once it is its turn, or an older copy
            # written last would drop the machine that answered in between.
            with self._spend_saving:
                with self.lock:
                    text = json.dumps(self.spend)
                try:
                    durable.put(self.spend_file, text, keep_bak=True, force=True,  # gathered again from the machines
                                tmp=self.spend_file.with_name(f'{self.spend_file.name}.{os.getpid()}.{threading.get_ident()}.tmp'))
                except OSError:
                    pass

    def spend_view(self):
        """What the machines said their chats used this week: {since, reports: {machine: answer}, able, machines,
        pending}. An answer for another week (the reset moved on while its machine was away) is left out. `able`
        counts machines whose agent can answer, `pending` those online that have not finished their first count."""
        with self.lock:
            week = self._since
            for n in [n for n in self.spend if n not in self.machines]:
                del self.spend[n]  # unpaired
            reports = {n: r for n, r in self.spend.items() if week and abs(r['since'] - week) <= RESET_SLACK}
            can = [n for n in self.order if SPEND_FEATURE in (self.state[n].get('features') or [])]
            pending = [n for n in can if n not in reports and self.state[n].get('online')]
            return {'since': week, 'reports': reports, 'able': len(set(can) | set(reports)), 'machines': len(self.order),
                    'pending': len(pending)}

    def _poll_one(self, name, timeout):
        m = self.machines.get(name)
        if m is None:  # unpaired while the round ran
            return
        if name in self.unlinked and m['target'].startswith('node:'):  # the hub does not list it: there is nobody to ask (nodes.sync says so, and why)
            return
        asked = self.clock()  # live guesses older than this are settled by the answer (server.App overlay)
        r = None
        try:
            req = self._list_req(m)
            r = self.transport(m['target'], req, timeout)
            if not isinstance(r, dict):
                raise OSError('its answer could not be read')
            if not r.get('ok'):
                raise OSError(r.get('error') or 'list failed')
            got = self.clock()
            sent = _num(r.get('clock')) or _num(r.get('now'))  # the machine's own clock as it answered
            # only rows and limits of the right shape: one machine's odd answer never breaks the list for the others
            rows = lambda k: [s for s in r[k] if isinstance(s, dict)] if isinstance(r.get(k), list) else []
            upd = {'online': True, 'error': None, 'sessions': rows('sessions'), 'replaced': rows('replaced'),
                   'limits': r.get('limits') if isinstance(r.get('limits'), dict) else None,
                   'account': clean_account(r.get('account')),
                   'updated': time.time(), 'as_of': got, 'skew': round(got - sent, 3) if sent else 0.0, 'root': r.get('root'), 'launcher': r.get('launcher'), 'mem': r.get('mem'),
                   'features': r.get('features') if isinstance(r.get('features'), list) else [], 'asked': asked,
                   'versions': r.get('versions') if isinstance(r.get('versions'), dict) else None,
                   'models': r.get('models') if isinstance(r.get('models'), dict) else None,
                   'setup': [i for i in r.get('setup') or [] if isinstance(i, dict) and i.get('id')][:12],
                   'compact': options_mod.compact_seen(r.get('compact')),
                   'root_set': r.get('root_set') if isinstance(r.get('root_set'), bool) else None,
                   'root_suggest': r.get('root_suggest') if isinstance(r.get('root_suggest'), str) else None,
                   # False: Claude Code there could not be asked what runs, so no row's kind or status says
                   # anything (a chat with neither is not a stopped one). An agent that never says is believed.
                   'agents_ok': r.get('agents_ok') is not False,
                   'boot': self._boot(name, got, r.get('up'))}
        except Exception as e:
            r = None
            if self._slow(name, e):
                return  # it is there and did not answer in time: what it last said stands for now
            upd = {'online': False, 'error': _short(e), 'asked': asked}
        else:
            self._late.pop(name, None)
        if r is not None:
            self.learn(name, r)
            if 'spend' in req:
                self._spend_got(name, r.get('spend'), upd['as_of'])
        self.store(name, upd)

    def _slow(self, name, e):
        """True when a failed poll says the machine is slow, not gone: the request reached it and no answer came
        back in time (MaybeDelivered), it was online at its last answer, and this has not happened SLOW_POLLS times
        in a row. A busy machine can take longer than the wait to list its chats; calling it offline for that
        closed its chats in the apps and opened them again a few seconds later. A machine that is not linked, or
        cannot be reached at all, is offline at once as before."""
        if not isinstance(e, (MaybeDelivered, subprocess.TimeoutExpired)):
            self._late.pop(name, None)
            return False
        with self.lock:
            if not (self.state.get(name) or {}).get('online'):
                return False
            n = self._late[name] = self._late.get(name, 0) + 1
        return n < SLOW_POLLS

    def _boot(self, name, got, up):
        """When the machine last started, by this server's clock, from the uptime it reports (None from an agent
        that does not say). Worked out afresh at every answer it would wander by the answer's travel time, and
        every wander would count as news for the apps: the time already held stands while the new one is within
        two minutes of it."""
        up = _num(up)
        if up is None or up > 20 * 365 * 86400:
            return None
        boot = round(got - up)
        with self.lock:
            had = (self.state.get(name) or {}).get('boot')
        return had if isinstance(had, (int, float)) and abs(had - boot) < 120 else boot

    def store(self, name, upd):
        """Keep what a machine said; tell the server when it differs from last time (not just its timestamps)."""
        said = {k: v for k, v in upd.items() if k not in ('updated', 'asked', 'as_of', 'skew')}
        with self.lock:
            if name not in self.state:
                return
            lim = said.get('limits')
            if isinstance(lim, dict) and not lim.get('merged'):
                # An agent from before merged limits says whichever chat drew its status line last, with the time of
                # the drawing: different at every poll. What counts as a change is the number that stands for it
                # (_hold), and its age only in steps, so the apps' "as of" keeps up without a nudge every poll.
                shown = self._hold(name, lim, self.clock())
                said['limits'] = [shown.get(k) for k in ('seven_day', 'seven_day_reset', 'five_hour', 'five_hour_reset')] \
                    + [(_num(shown.get('updated')) or 0) // LIMITS_STEP]
            digest = json.dumps(said, sort_keys=True, default=str)
            self.state[name].update(upd)
            changed = self._digest.get(name) != digest
            self._digest[name] = digest
        if changed and self.on_change:
            try:
                self.on_change(name)
            except Exception:
                pass

    def poll(self, timeout=15):
        order = self.order
        if order:
            with ThreadPoolExecutor(len(order)) as ex:
                list(ex.map(lambda n: self._poll_one(n, timeout), order))
        self.record_week()

    def record_week(self):
        """Write down the weekly percent (at most every 15 minutes) for the widgets' week strip and trace."""
        if self.history is None and not any(SPEND_FEATURE in (st.get('features') or []) for st in self.state.values()):
            return
        now = self.clock()
        lim = self.limits(now)
        self._since = week_start(lim.get('seven_day_reset'), now)  # the next poll asks the machines from here
        if self.history is not None:
            self.history.record(lim.get('seven_day'), lim.get('seven_day_reset'), now)

    def poll_one(self, name, timeout=15):
        if name in self.machines:
            self._poll_one(name, timeout)

    def _live(self, n, s, now):
        """A session as the apps get it: a stop just sent, its live state (derive) and its age."""
        st = self.state[n]
        s = self._overlay(n, s, now)
        s = dict(s, **derive(s, now, st.get('skew') or 0.0, updated=st.get('updated')))
        return mark_age(s, now)

    def snapshot(self):
        """Every machine and its sessions, each with its live state (derive; the server adds held chats and hook
        pings, server.App._apply_overlay). `now` is the server's clock, the one `as_of` and `cold_at` are on."""
        now = self.clock()
        with self.lock:
            return {'machines': [dict(self.machines[n], **dict(self.state[n], sessions=[self._live(n, s, now) for s in self.state[n]['sessions']],
                                                                 replaced=[self._overlay(n, r, now) for r in self.state[n].get('replaced') or []]),
                                      target=None, icon=icon_of(self.machines[n]))
                                 for n in self.order], 'now': now}

    @staticmethod
    def _newest_limits(snap, now=None):
        """The plan numbers from what every machine last reported, merged (merge_limits). `updated` comes out on the
        server's clock: each machine's own time plus how far its clock is off."""
        rows = []
        for m in snap:
            lim = m.get('limits')
            if isinstance(lim, dict):
                at, skew = _num(lim.get('updated')), m.get('skew')
                rows.append(dict(lim, updated=float(round(at + skew))) if at and isinstance(skew, (int, float)) else lim)
        return merge_limits(rows, time.time() if now is None else now) or {}

    def limits(self, now=None, snap=None):
        """The plan numbers every part of the server shows and budgets by. An agent from before merged limits reports
        whichever chat redrew its status line last, so its numbers jump around under the true ones; inside one window
        a number only grows, so for such a machine the highest it has shown in each window stands (the week, and
        the 5 hours by their own reset times), as of the last time the machine showed any number. After a restart
        the week starts from the last sample written down (WeekHistory), so it cannot dip then either."""
        now = self.clock() if now is None else now
        snap = [dict(m) for m in (self.snapshot()['machines'] if snap is None else snap)]
        with self.lock:
            for m in snap:
                m['limits'] = self._hold(m['name'], m.get('limits'), now)
        return self._newest_limits(snap, now)

    def _hold(self, name, lim, now):
        """One machine's plan numbers as limits() counts them: its own when its agent merges them, else the highest
        it has shown in each window. Called with the lock held."""
        if not isinstance(lim, dict) or lim.get('merged'):
            self._held.pop(name, None)
            return lim
        last = self.history.last() if self.history is not None and hasattr(self.history, 'last') else None
        lim = sane_limits(lim, now)
        held = self._held.setdefault(name, {})
        at = max(_num(lim.get('updated')) or 0, held.get('updated') or 0)
        for pct_k, reset_k, keep in (('seven_day', 'seven_day_reset', 0), ('five_hour', 'five_hour_reset', WEEK)):
            pct, reset = lim.get(pct_k), _num(lim.get(reset_k))
            seen = _isnum(pct) and bool(reset) and reset > now
            h = held.get(pct_k)
            if h and h[1] + keep <= now:  # its window is over (the 5 hours stay a week, to say "has reset")
                h = None
            if h is None and seen and keep == 0 and last and abs(last[0] - reset) <= RESET_SLACK:
                h = (last[1], last[0])  # a server that just started: where the week stood when it stopped
            if seen and (h is None or reset - h[1] > RESET_SLACK or (abs(reset - h[1]) <= RESET_SLACK and pct > h[0])):
                h = (pct, reset)  # a new window, or more of this one used
            if h is None:
                held.pop(pct_k, None)
                continue
            held[pct_k] = h
            if h[1] > now or not seen:
                lim[pct_k], lim[reset_k] = h
        if at:
            held['updated'] = lim['updated'] = at
        return lim

    def _account_asker(self):
        """The one machine that asks Claude Code for the account's usage report: the first online one whose agent can
        (and whose last try worked, when any did), so the fleet makes one small call, not one per machine. Called
        with the lock held. None when no machine can."""
        able = [n for n in self.order if self.state[n].get('online') and ACCOUNT_FEATURE in (self.state[n].get('features') or [])]
        good = [n for n in able if not (self.state[n].get('account') or {}).get('error')]
        return (good or able or [None])[0]

    def _account_ask(self, name):
        """How old a report this machine's next poll may answer with (seconds), or None when it is not the asker:
        a quarter of an hour while chats reply (a status line saw numbers after the last report), an hour otherwise."""
        with self.lock:
            if name != self._account_asker():
                return None
            at = max((_num((self.state[n].get('account') or {}).get('at')) or 0 for n in self.order), default=0)
            seen = max((_num((self.state[n].get('limits') or {}).get('updated')) or 0 for n in self.order), default=0)
        return ACCOUNT_EVERY if seen > at else ACCOUNT_IDLE

    def account(self, snap=None):
        """The account's usage report for the apps: the newest any machine has, with the machine it came from, its
        times on the server's clock, and `support` (how many machines could report one, and how many are online)
        so the apps can say why there is none. One account is assumed, as for the limits."""
        snap = self.snapshot()['machines'] if snap is None else snap
        best, failed = None, None
        for m in snap:
            a = m.get('account')
            if not isinstance(a, dict):
                continue
            skew = m.get('skew') if isinstance(m.get('skew'), (int, float)) else 0.0
            a = dict(a, machine=m.get('label') or m.get('name'),
                     at=round(a['at'] + skew, 1) if _isnum(a.get('at')) else None,
                     tried=round(a['tried'] + skew, 1) if _isnum(a.get('tried')) else None)
            if a['at'] and (best is None or a['at'] > best['at']):
                best = a
            elif not a['at'] and a.get('error') and failed is None:
                failed = a
        able = [m for m in snap if ACCOUNT_FEATURE in (m.get('features') or [])]
        support = {'able': len(able), 'online': sum(1 for m in able if m.get('online')), 'machines': len(snap)}
        return best or failed, support

    def refresh_account(self, max_age=ACCOUNT_FRESH, timeout=30):
        """Refresh in the apps: ask the asker machine for a report now and keep what it says. One at a time; a
        second Refresh while one runs waits for it and takes its answer. False when nobody could be asked."""
        with self._account_busy:
            with self.lock:
                name = self._account_asker()
                m = self.machines.get(name) if name else None
            if not m:
                return False
            try:
                r = self.transport(m['target'], {'verb': 'account_usage', 'max_age': max_age, 'as': name}, timeout)
            except Exception:
                return False
            if not isinstance(r, dict) or not r.get('ok'):
                return False
            upd = {'account': clean_account(r.get('account'))}
            if isinstance(r.get('limits'), dict):
                upd['limits'] = r['limits']
            self.store(name, upd)
            self.record_week()
            return True

    def summary(self):
        snap = self.snapshot()['machines']
        now = self.clock()
        newest = self.limits(now, snap)
        live = {m['name']: [s for s in m['sessions'] if not s['stale']] for m in snap}
        sessions = [s for m in snap for s in live[m['name']]]
        week, reset = newest.get('seven_day'), newest.get('seven_day_reset')
        daily, trace = self.history.view(week, reset, now) if self.history is not None else ([], [])
        return {'five_hour': newest.get('five_hour'), 'seven_day': week,
                'five_hour_reset': newest.get('five_hour_reset'), 'seven_day_reset': reset,
                'limits_updated': newest.get('updated'),  # when a session last saw these numbers (unix seconds)
                'sessions': len(sessions), 'running': _running(sessions), 'cold': _cold(sessions),
                'week_projected': _projected(week, reset, now),
                'online': sum(1 for m in snap if m['online']), 'machines': len(snap),
                'week_daily': daily, 'week_trace': trace, 'tz': DAY_TZ,
                'machine_list': [{'name': m['name'], 'label': m['label'], 'online': bool(m['online']), 'os': m.get('os') or '',
                                  'icon': m['icon'],
                                  'running': _running(live[m['name']]), 'cold': _cold(live[m['name']])} for m in snap]}

    def usage(self):
        """GET /api/usage: the plan limits the Week chip shows, this week by day, and past weeks' final percents."""
        now = self.clock()
        snap = self.snapshot()['machines']
        newest = self.limits(now, snap)
        week, reset = newest.get('seven_day'), newest.get('seven_day_reset')
        days, weeks = self.history.usage(week, reset, now) if self.history is not None else ([], [])
        account, support = self.account(snap)
        return {'ok': True, 'now': now, 'tz': DAY_TZ,
                'five': {'pct': newest.get('five_hour'), 'reset': newest.get('five_hour_reset')},
                'week': {'pct': week, 'reset': reset, 'pace': _projected(week, reset, now)},
                'updated': newest.get('updated'),  # when these two numbers were last seen (server clock)
                'account': account, 'account_support': support,
                'days': days, 'weeks': weeks}

    def call(self, name, req, timeout=40):
        m = self.machines.get(name)
        if not m:
            raise FleetError('unknown machine')
        try:
            return self.transport(m['target'], req, timeout)
        except Exception as e:
            err = FleetError(f'{m["label"]} is not reachable right now ({_short(e)})')
            err.maybe_delivered = isinstance(e, (MaybeDelivered, subprocess.TimeoutExpired))
            if not err.maybe_delivered:  # it could not be reached at all: announced now, not at the next poll
                self.store(name, {'online': False, 'error': _short(e)})
            # A call that went out and got no answer says nothing of the machine: one slow verb is not offline.
            raise err


def week_start(reset, now):
    """When the week the plan is counting began: seven days before its reset; or, when that reset has passed and no
    chat has seen the next one yet, the passed reset itself (the new week began there). None when no reset is known."""
    if not _isnum(reset) or reset <= 0 or not _isnum(now):
        return None
    if reset > now:
        return reset - WEEK if reset - WEEK <= now else None
    return reset + WEEK * int((now - reset) // WEEK)


def clean_spend(r):
    """A machine's answer about what its chats used (agent/sb.py spend_report) with only what has the right shape:
    {total, n, chats: [{sid, w, title?}]}, or {more: True} while it is still reading. None when it is neither."""
    if not isinstance(r, dict):
        return None
    if r.get('more') is True and 'total' not in r:
        return {'more': True}
    if not _isnum(r.get('total')) or not isinstance(r.get('chats'), list):
        return None
    chats, seen = [], set()
    for c in r['chats'][:SPEND_CHATS]:
        if not isinstance(c, dict) or not isinstance(c.get('sid'), str) or not 0 < len(c['sid']) <= 64 or c['sid'] in seen:
            continue
        if not _isnum(c.get('w')) or c['w'] <= 0:
            continue
        seen.add(c['sid'])
        row = {'sid': c['sid'], 'w': int(c['w'])}
        if isinstance(c.get('title'), str) and c['title'].strip():
            row['title'] = ' '.join(c['title'].split())[:120]
        chats.append(row)
    named = sum(c['w'] for c in chats)
    out = {'total': max(int(r['total']), named), 'n': max(int(r['n']) if _isnum(r.get('n')) else 0, len(chats)), 'chats': chats}
    if _isnum(r.get('since')) and _isnum(r.get('at')):  # as kept in the server's own file
        out.update(since=r['since'], at=r['at'])
    return out


def top_chats(view, listed=None, chats=None, roots=None, labels=None, sent=None, limit=SPEND_TOP):
    """GET /api/usage `top`: the chats that used the most since the week began, largest first.

    view: Fleet.spend_view(). listed: {"machine/sid": row} for the chats the apps list now. chats and roots: every
    chat the server remembers and each one's thread ({key: root key}, features.Threads.graph), so a chat that was
    resumed, started fresh or compacted is one row with what came before it. labels: {machine: its name for people}.
    sent(key): was this chat sent by a chat (run_on)?

    Each row: machine and sid of the chat to open (the thread's chat the apps list now, else its newest), name,
    label, weighted (tokens times the cost weights: an estimate, never the plan's own number), share (percent of
    everything Switchboard saw this week, on every machine), alive (the apps list it now), sent, parts (how many
    chats the row adds up)."""
    listed, chats, roots, labels = listed or {}, chats or {}, roots or {}, labels or {}
    out = {'since': view.get('since'), 'total': 0, 'chats': 0, 'rows': [],
           'able': view.get('able') or 0, 'machines': view.get('machines') or 0, 'pending': view.get('pending') or 0}
    groups, total, unnamed = {}, 0, 0
    for name, r in sorted((view.get('reports') or {}).items()):
        total += r['total']
        unnamed += max(0, r['n'] - len(r['chats']))
        for c in r['chats']:
            k = f'{name}/{c["sid"]}'
            g = groups.setdefault(roots.get(k, k), {'w': 0, 'parts': []})
            g['w'] += c['w']
            g['parts'].append((k, name, c))
    if total <= 0:
        return out
    ended = lambda k: (chats.get(k) or {}).get('ended') or 0
    rows = []
    for g in groups.values():
        keys = [p[0] for p in g['parts']]
        # the chat to show for the thread: the one the apps list now (the newest of them), else the newest used
        shown = [k for k in keys if k in listed]
        k = max(shown or keys, key=lambda k: ((listed.get(k) or {}).get('active') or ended(k), k))
        _, name, c = next(p for p in g['parts'] if p[0] == k)
        row = listed.get(k) or {}
        title = row.get('title') or (chats.get(k) or {}).get('title') or c.get('title') or ''
        rows.append({'machine': name, 'sid': c['sid'], 'name': ' '.join(str(title).split())[:120],
                     'label': labels.get(name) or name, 'weighted': g['w'], 'share': round(100.0 * g['w'] / total, 1),
                     'alive': k in listed, 'sent': bool(row.get('sent') or (sent(k) if sent else False)),
                     'parts': len(keys)})
    rows.sort(key=lambda r: (-r['weighted'], r['machine'], r['sid']))
    out.update(total=total, chats=len(rows) + unnamed, rows=rows[:limit])
    return out


def _running(sessions):
    return sum(1 for s in sessions if s.get('activity') == 'working')


def _cold(sessions):
    return sum(1 for s in sessions if s.get('cache') == 'cold' and s.get('heavy') and s.get('activity') == 'quiet')


def _isnum(v):
    return isinstance(v, (int, float)) and not isinstance(v, bool) and v == v and abs(v) != float('inf')


def clean_account(a):
    """A machine's account usage report (agent/sb.py account_report) with only what has the right shape, so one
    machine's odd answer never reaches the apps: percents 0 to 100, short strings, money as {minor, currency, exp}.
    None when it is not a report."""
    if not isinstance(a, dict):
        return None
    text = lambda v, most=64: v.strip()[:most] if isinstance(v, str) and v.strip() else None
    num = lambda v: v if _isnum(v) else None
    pct = lambda v: max(0, min(100, v)) if _isnum(v) else None

    def money(m):
        if not isinstance(m, dict) or not _isnum(m.get('minor')):
            return None
        exp = m.get('exp')
        return {'minor': m['minor'], 'currency': text(m.get('currency'), 8),
                'exp': exp if isinstance(exp, int) and not isinstance(exp, bool) and 0 <= exp <= 4 else 2}
    out = {'at': num(a.get('at')), 'tried': num(a.get('tried')), 'error': text(a.get('error'), 160),
           'plan': text(a.get('plan'), 32), 'available': a.get('available') is not False, 'limits': [], 'extra': None, 'products': None}
    for r in a.get('limits') if isinstance(a.get('limits'), list) else []:
        if isinstance(r, dict) and text(r.get('kind')) and pct(r.get('pct')) is not None:
            out['limits'].append({'kind': text(r['kind']), 'pct': pct(r['pct']), 'reset': num(r.get('reset')), 'label': text(r.get('label')),
                                  'severity': text(r.get('severity'), 16), 'active': r.get('active') is True})
    out['limits'] = out['limits'][:16]
    e = a.get('extra')
    if isinstance(e, dict):
        flag = lambda k: e[k] if isinstance(e.get(k), bool) else None
        out['extra'] = {'enabled': flag('enabled'), 'used': money(e.get('used')), 'limit': money(e.get('limit')),
                        'balance': money(e.get('balance')), 'pct': pct(e.get('pct')), 'reason': text(e.get('reason')),
                        'limit_reached': e.get('limit_reached') is True, 'auto_reload': flag('auto_reload')}
    p = a.get('products')
    if isinstance(p, dict) and isinstance(p.get('rows'), list):
        rows = [{'key': text(r.get('key')) or '', 'name': text(r.get('name')), 'pct': pct(r.get('pct')), 'of_limit': pct(r.get('of_limit'))}
                for r in p['rows'] if isinstance(r, dict)]
        rows = [r for r in rows if r['name'] and r['pct'] is not None][:16]
        if rows:
            out['products'] = {'as_of': num(p.get('as_of')), 'start': num(p.get('start')), 'rows': rows}
    if out['at'] is None and not out['error']:
        return None
    return out


def sane_limits(lim, now):
    """One sighting of the plan numbers with what cannot be true taken out, so that a single garbled reading never
    stands as "the highest this week": a percent that is not 0 to 100 (a little over reads as 100), or a window
    that would end later than one starting now could (5 hours, 7 days)."""
    out = dict(lim)
    for pct, reset, span in (('five_hour', 'five_hour_reset', FIVE_HOURS), ('seven_day', 'seven_day_reset', WEEK)):
        v, r = lim.get(pct), lim.get(reset)
        if v is None:
            continue
        if not _isnum(v) or not 0 <= v <= PCT_MOST or (_isnum(r) and r > now + span + RESET_SLACK):
            out[pct] = out[reset] = None
        elif v > 100:
            out[pct] = 100
    return out


def merge_limits(rows, now):
    """One answer from many sightings of the plan numbers (dicts with five_hour, seven_day, their reset times and
    `updated`, when a reply saw them). Each window is read by itself: of the sightings whose window is still open,
    the one that resets last is the current window, and the highest percent in it is the truth, because use only
    grows inside a window and a chat that has not replied for a while still shows an older, lower number. `updated`
    is when that highest number was last seen. With no open window, the newest sighting stands (its reset time says
    it is over). Readings that cannot be true are left out first (sane_limits). None when nobody saw a number. The
    agent merges its chats the same way (agent/sb.py merge_limits); one account is assumed."""
    num = _isnum  # a percent may be 0
    at = lambda r: r.get('updated') if num(r.get('updated')) else 0
    rows = [sane_limits(r, now) for r in rows if isinstance(r, dict)]
    rows = [r for r in rows if num(r.get('five_hour')) or num(r.get('seven_day'))]
    if not rows:
        return None

    def window(pct, reset):
        live = [r for r in rows if num(r.get(pct)) and num(r.get(reset)) and r[reset] > now]
        if not live:
            return None
        end = max(r[reset] for r in live)
        live = [r for r in live if end - r[reset] <= RESET_SLACK]
        top = max(r[pct] for r in live)
        return max((r for r in live if r[pct] == top), key=at)
    newest = max(rows, key=at)
    week = window('seven_day', 'seven_day_reset') or max((r for r in rows if num(r.get('seven_day'))), key=at, default=None)
    # No open 5-hour window: the last one anybody saw within the week (its reset time says it is over), else what
    # the newest reply saw. Nothing there means the plan does not report one.
    past = [r for r in rows if num(r.get('five_hour')) and num(r.get('five_hour_reset')) and r['five_hour_reset'] > now - WEEK]
    five = window('five_hour', 'five_hour_reset') or max(past, key=lambda r: r['five_hour_reset'], default=None) or newest
    week = week or {}
    return {'five_hour': five.get('five_hour'), 'seven_day': week.get('seven_day'),
            'five_hour_reset': five.get('five_hour_reset') if five.get('five_hour') is not None else None,
            'seven_day_reset': week.get('seven_day_reset'), 'updated': (week or five).get('updated')}


def _projected(used, reset, now, week=WEEK):
    """Weekly use at reset if the rest of the week goes like the part so far. None once the reset has passed: the
    number is last week's."""
    if used is None or not reset or reset <= now:
        return None
    elapsed = (week - (reset - now)) / week
    if elapsed < 0.05:  # too early in the week to say
        return None
    return round(used / min(elapsed, 1.0))


def _short(e):
    s = str(e) or type(e).__name__
    if isinstance(e, subprocess.TimeoutExpired):
        s = 'timed out'
    return s.strip()[:160]


# ---- the week so far, for the iPhone widgets (week strip and trace)

def _zone(tz):
    if not isinstance(tz, str):
        return tz
    try:
        from zoneinfo import ZoneInfo
        return ZoneInfo(tz)
    except Exception:  # no tz database: the server's own time
        return None


def _midnight_after(ts, tz):
    day = datetime.fromtimestamp(ts, tz).date() + timedelta(days=1)
    return datetime.combine(day, dtime(0), tzinfo=tz).timestamp()


def week_days(reset, tz=DAY_TZ):
    """The week that ends at `reset` as seven (start, end) spans on calendar days in `tz`.

    A week that does not start at midnight touches eight calendar days; the shorter end piece joins its neighbour,
    so a Thursday 9 AM reset gives Thu (from 9 AM) through Wed (to Thursday 9 AM). The iPhone draws the same columns.
    """
    tz = _zone(tz)
    start = reset - WEEK
    edges, t = [start], start
    while True:
        t = _midnight_after(t, tz)
        if t >= reset:
            break
        edges.append(t)
    edges.append(reset)
    while len(edges) > 8:
        if edges[1] - edges[0] < edges[-1] - edges[-2]:
            del edges[1]
        else:
            del edges[-2]
    return list(zip(edges, edges[1:]))


def _pct_at(points, t):
    """The weekly percent at `t` from known (ts, pct) points in time order; None when it cannot be told.
    Between two points it is a straight line, unless they are far apart and usage grew in between."""
    before = after = None
    for p in points:
        if p[0] <= t:
            before = p
        if p[0] >= t:
            after = p
            break
    if before is None or after is None:
        return None
    if after[0] == before[0]:
        return after[1]
    if after[1] - before[1] > 1 and after[0] - before[0] > GAP:
        return None
    return before[1] + (after[1] - before[1]) * (t - before[0]) / (after[0] - before[0])


def week_daily(samples, pct, reset, now, tz=DAY_TZ):
    """Percent of the weekly limit used on each day of the week so far, ending with today. None for a day the
    samples cannot account for: the server was not keeping them yet, or was down while usage grew."""
    start = reset - WEEK
    points = [(start, 0.0)] + [(t, p) for t, p in samples if start < t < now] + [(now, float(pct))]
    out = []
    for a, b in week_days(reset, tz):
        if a >= now:
            break
        lo, hi = _pct_at(points, a), _pct_at(points, min(b, now))
        out.append(None if lo is None or hi is None else round(max(0.0, hi - lo), 1))
    return out


def week_trace(samples, reset):
    """The samples thinned to about one an hour (the last one in each hour since the reset), as [ts, pct]."""
    start, hours = reset - WEEK, {}
    for t, p in samples:
        hours[int((t - start) // 3600)] = [int(t), round(p, 1)]
    return [hours[h] for h in sorted(hours)]


def local_midnight(ts, tz=DAY_TZ):
    """The start of the local calendar day `ts` falls on, in `tz`."""
    tz = _zone(tz)
    return datetime.combine(datetime.fromtimestamp(ts, tz).date(), dtime(0), tzinfo=tz).timestamp()


def end_of_day(ts, tz=DAY_TZ):
    """The midnight that ends the local calendar day `ts` falls on, in `tz`."""
    return _midnight_after(ts, _zone(tz))


def today_used(samples, pct, reset, now, tz=DAY_TZ):
    """Percent of the weekly limit used today (local midnight in `tz`, or the weekly reset if that came later, up to
    now): the weekly percent now minus the weekly percent at the day's start, never negative. The start is read off
    the samples like week_daily does; when they cannot tell it (the server was down across midnight while usage grew),
    the day's first sample stands in. None when nothing can be said."""
    if pct is None or not reset or reset <= now:
        return None
    start = reset - WEEK
    lo = max(local_midnight(now, tz), start)
    points = [(start, 0.0)] + sorted((t, p) for t, p in samples if start < t < now and p is not None) + [(now, float(pct))]
    base = _pct_at(points, lo)
    if base is None:
        later = [p for t, p in points if lo <= t < now]
        if not later:
            return None
        base = later[0]
    return round(max(0.0, float(pct) - base), 1)


def days_used(samples, pct, reset, now, tz=DAY_TZ):
    """[{day, used}] for each local calendar day in `tz` of the week that ends at `reset`, from its first day to the
    day `now` falls on: the weekly percent at the day's end (or now) minus the percent at its start, never negative.
    The percent at each midnight is read off the samples like week_daily does, so the days add up to the week so
    far; a day the samples cannot account for (the server was not keeping them yet, or was down while usage grew) is
    None. The last day is today_used, the number the budget counts."""
    if pct is None or not reset:
        return []
    tz = _zone(tz)
    start, end = reset - WEEK, min(now, reset)
    points = [(start, 0.0)] + sorted((t, float(p)) for t, p in samples if start < t < end and p is not None) + [(end, float(pct))]
    out, d, last = [], datetime.fromtimestamp(start, tz).date(), datetime.fromtimestamp(end, tz).date()
    while d <= last:
        a = max(start, datetime.combine(d, dtime(0), tzinfo=tz).timestamp())
        b = min(end, datetime.combine(d + timedelta(days=1), dtime(0), tzinfo=tz).timestamp())
        lo, hi = _pct_at(points, a), _pct_at(points, b)
        used = None if lo is None or hi is None else round(max(0.0, hi - lo), 1)
        if d == last and now < reset:
            used = today_used(samples, pct, reset, now, tz)
        out.append({'day': d.isoformat(), 'used': used})
        d += timedelta(days=1)
    return out


class WeekHistory:
    """The weekly percent, written down at most every 15 minutes, for the current week only.

    Kept in a small JSON file in the server's state dir, so a restart keeps the week. Samples from before the
    current weekly reset are dropped, so the file never holds more than a week (under 700 samples).
    """
    KEEP = 800
    WEEKS = 8  # past weeks kept in week_archive.json, newest first

    def __init__(self, path, every=SAMPLE_EVERY, tz=DAY_TZ, archive=None):
        self.path, self.every, self.tz = Path(path), every, tz
        self.archive = Path(archive) if archive else self.path.with_name('week_archive.json')
        self.lock = threading.Lock()
        self.reset, self.samples, self.weeks = None, [], []
        # A file that cannot be read goes back to its last good copy (durable.load); with none, the history starts
        # over and the damaged file is kept beside it. keep: the archive is written once a week.
        d, was = durable.load(self.archive, None, dict, check=lambda d: isinstance(d.get('weeks'), list), keep=True)
        if d is not None:
            self.weeks = [w for w in d['weeks'] if isinstance(w, dict) and isinstance(w.get('reset'), (int, float))][:self.WEEKS]
        d, now = durable.load(self.path, None, dict, check=self._week, keep=True)
        for name, status in ((self.archive.name, was), (self.path.name, now)):
            if status in (durable.RECOVERED, durable.BROKEN):
                log(f'{name} could not be read: ' + ('its last good copy is in use' if status == durable.RECOVERED else 'it starts over'))
        if d is None:
            return
        reset, samples = self._week(d)
        self.reset, self.samples = reset, samples[-self.KEEP:]

    @staticmethod
    def _week(d):
        """(reset, samples) from the file's JSON, or None when it holds something else."""
        try:
            return float(d['reset']), sorted([float(t), float(p)] for t, p in d['samples'])
        except (ValueError, TypeError, KeyError):
            return None

    def record(self, pct, reset, now):
        """Keeps the weekly percent now if the last sample is 15 minutes old. True when it was written down."""
        if pct is None or not reset or reset <= now:  # unknown, or numbers from a week that is over
            return False
        if not _isnum(pct) or not 0 <= pct <= PCT_MOST or reset > now + WEEK + RESET_SLACK:
            return False  # not a reading: no week ends more than seven days from now
        pct = min(pct, 100)
        with self.lock:
            if self.reset is not None and self.reset > now + WEEK + RESET_SLACK:
                self.reset, self.samples = None, []  # one such reading was kept by an older server: it goes
            if self.reset is not None and self.reset - reset > RESET_SLACK:
                # A week that ends before the one being kept, which is still running: numbers from somewhere else
                # (a second account). One week is kept; this one is not written into it.
                return False
            if self.reset is None or reset - self.reset > RESET_SLACK:
                if self.reset is not None and self.reset <= now:
                    self._archive()  # only a week that ran to its end is a past week; one cut short is not
                self.samples = []  # a new week (or an early reset): what was kept belongs to the last one
            self.reset = reset
            self.samples = [s for s in self.samples if reset - WEEK <= s[0] <= now]
            if self.samples and now - self.samples[-1][0] < self.every:
                return False
            self.samples = (self.samples + [[float(now), float(pct)]])[-self.KEEP:]
            try:
                durable.put(self.path, json.dumps({'reset': self.reset, 'samples': self.samples}), keep_bak=True, force=True)
            except OSError:
                pass  # kept in memory; the next sample writes the file again
            return True

    def last(self):
        """(reset, percent) of the newest sample, or None: where the week stood when it was last written down."""
        with self.lock:
            return (self.reset, self.samples[-1][1]) if self.reset is not None and self.samples else None

    def _archive(self):
        """Keep the week that just ended (its last percent and its days) in week_archive.json. Called under the lock."""
        if self.reset is None or not self.samples:
            return
        old = self.reset
        pct = self.samples[-1][1]  # where the week ended, as far as anyone saw
        week = {'reset': old, 'pct': round(pct, 1), 'days': days_used(self.samples, pct, old, old, self.tz)}
        self.weeks = ([week] + [w for w in self.weeks if abs(w['reset'] - old) > RESET_SLACK])[:self.WEEKS]
        self.weeks.sort(key=lambda w: -w['reset'])
        try:
            durable.put(self.archive, json.dumps({'weeks': self.weeks}), keep_bak=True, force=True)
        except OSError:
            pass  # kept in memory until the next archive

    def usage(self, pct, reset, now):
        """(days of this week so far, past weeks newest first) for GET /api/usage."""
        with self.lock:
            weeks = [dict(w) for w in self.weeks]
            same = self.reset is not None and reset and abs(reset - self.reset) <= RESET_SLACK
            samples = [tuple(s) for s in self.samples if reset - WEEK <= s[0] <= now] if same else []
        if not reset or reset <= now:
            return [], weeks
        return days_used(samples, pct, reset, now, self.tz), weeks

    def today(self, pct, reset, now):
        """Percent of the week used today so far (today_used), for the usage budget."""
        if pct is None or not reset or reset <= now:
            return None
        with self.lock:
            same = self.reset is not None and abs(reset - self.reset) <= RESET_SLACK
            samples = [tuple(s) for s in self.samples if reset - WEEK <= s[0] <= now] if same else []
        return today_used(samples, pct, reset, now, self.tz)

    def view(self, pct, reset, now):
        """(week_daily, week_trace) for the week ending at `reset`; empty lists when there is nothing to say yet."""
        if pct is None or not reset or reset <= now:
            return [], []
        with self.lock:
            same = self.reset is not None and abs(reset - self.reset) <= RESET_SLACK
            samples = [tuple(s) for s in self.samples if reset - WEEK <= s[0] <= now] if same else []
        return week_daily(samples, pct, reset, now, self.tz), week_trace(samples, reset)
