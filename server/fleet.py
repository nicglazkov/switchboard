"""Talks to the machines: one forced-command SSH call per request, JSON in and out."""
import json, os, re, subprocess, threading, time
from concurrent.futures import ThreadPoolExecutor
from datetime import datetime, time as dtime, timedelta
from pathlib import Path, PurePosixPath, PureWindowsPath

GUARD_CTX = 150_000
STALE_CLOSED = 2 * 86400  # a chat nothing has open, untouched for 2 days, moves to Older
STALE_OPEN = 7 * 86400    # one still open in a terminal or the background, after a week
STALLED = 6 * 3600        # "busy" with no transcript writes for 6 hours is a stuck status, not work
KEY = '~/.ssh/switchboard_ed25519'
WEEK = 7 * 86400
SAMPLE_EVERY = 15 * 60    # the weekly percent is written down at most this often
RESET_SLACK = 3600        # a weekly reset that moves more than this is a new week
GAP = 3 * 3600            # samples further apart than this, with usage growing between them, leave a day unknown
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
    pass


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
    return out


def check_root(text, windows=False):
    """A Claude folder as Settings sends it: absolute or home-relative ('~', '~/Claude'), with no '..' and no NUL.
    Returns it tidied, or raises FleetError. The agent checks it again the same way (agent/sb.py check_root)."""
    if not isinstance(text, str) or not 0 < len(text.strip()) <= 400 or '\x00' in text:
        raise FleetError('Pick a Claude folder, like ~/Claude.')
    t = text.strip()
    parts = re.split(r'[\\/]+', t)
    if '..' in parts:
        raise FleetError('The Claude folder cannot use "..".')
    if t == '~' or t.startswith(('~/', '~\\')):
        rest = [p for p in parts[1:] if p not in ('', '.')]
        return '~' + ('/' + '/'.join(rest) if rest else '')
    if windows:
        if not re.match(r'^[A-Za-z]:[\\/]', t) and not t.startswith('\\\\'):
            raise FleetError('The Claude folder is a full path, like C:\\Users\\you\\Claude or ~/Claude.')
        return str(PureWindowsPath(t))
    if not t.startswith('/'):
        raise FleetError('The Claude folder is a full path, like ~/Claude or /Users/you/Claude.')
    return str(PurePosixPath(t))


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
            d = json.loads(Path(path).read_text())
        except (OSError, ValueError):
            return None
        rows = d if isinstance(d, list) else d.get('machines') if isinstance(d, dict) else None
        if not isinstance(rows, list):
            return None
        out, seen = [], set()
        for r in rows:
            m = clean_machine(r)
            if m and m['name'] not in seen:
                seen.add(m['name'])
                out.append(m)
        return out

    def load(self):
        """The machines. Seeds the file the first time (ssh machines only: linked ones come back from the hub)."""
        got = self._read(self.path)
        if got is not None:
            return got
        seeded = [m for m in (self._read(self.seed) or []) if not m['target'].startswith('node:')] if self.seed else []
        try:
            self.save(seeded)
        except OSError:
            pass
        return seeded

    def save(self, machines):
        with self.lock:
            self.path.parent.mkdir(parents=True, exist_ok=True)
            tmp = self.path.with_name(f'.{self.path.name}.{os.getpid()}.tmp')
            tmp.write_text(json.dumps({'machines': [clean_machine(m) for m in machines if clean_machine(m)]}, indent=1))
            os.replace(tmp, self.path)


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
    r = subprocess.run(['ssh', '-T', '-i', os.path.expanduser(KEY), '-o', 'BatchMode=yes', '-o', 'IdentitiesOnly=yes',
                        '-o', 'ConnectTimeout=5', '-o', 'ServerAliveInterval=5', target],
                       input=json.dumps(req), capture_output=True, text=True, timeout=timeout)
    out = r.stdout.strip()
    if not out:
        err = (r.stderr or '').strip().splitlines()
        raise OSError(err[-1] if err else f'ssh exited {r.returncode}')
    return json.loads(out.splitlines()[-1])


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
        self.on_change = None  # called with a machine's name when what it reports changed (the server's live updates)
        self.on_save = None  # called with the machine list after it changed (the server writes fleet.json and the hub's file)
        self.servers = []  # the server's own addresses, told to each machine's agent (where its hooks report)
        self.budget_view = None  # set by the server: the usage budget's status, sent with every poll (budget.py)
        self._digest = {}
        self._stopped = {}  # (machine, sid): when Stop succeeded; shown stopped at once, not at the next poll

    STOP_HOLD = 20  # seconds a stop shows before the machine's own list agrees (claude stop can lag a poll)

    def mark_stopped(self, name, sid, on=True):
        """A chat was just stopped (or, on=False, started again): the board shows it at once."""
        with self.lock:
            if on:
                self._stopped[(name, sid)] = self.clock()
            else:
                self._stopped.pop((name, sid), None)

    def _overlay(self, n, s, now):
        t = self._stopped.get((n, s.get('sid')))
        if t is None:
            return s
        if now - t > self.STOP_HOLD or s.get('status') in ('stopped', 'done') or not s.get('kind'):
            self._stopped.pop((n, s.get('sid')), None)
            return s
        return dict(s, status='stopped')

    @staticmethod
    def _blank():
        return {'online': None, 'error': None, 'sessions': [], 'replaced': [], 'limits': None, 'updated': None, 'as_of': None, 'skew': 0.0, 'root': None,
                'launcher': None, 'mem': None, 'features': [], 'versions': None, 'setup': [], 'root_set': None, 'root_suggest': None}

    def config(self):
        """The machine list as saved: in order, without what the machines report."""
        with self.lock:
            return [dict(self.machines[n]) for n in self.order]

    def _saved(self):
        if self.on_save:
            try:
                self.on_save(self.config())
            except Exception:
                pass

    def add_machine(self, m, save=True):
        """A machine added in Settings, or linked through the hub (nodes.py): polled from the next round on. One that is
        already on the list keeps its name for it (the user may have renamed it). True when something changed."""
        m = clean_machine(m)
        if m is None:
            raise FleetError('That machine needs a short id (a-z, 0-9) and an address.')
        with self.lock:
            if m['name'] in self.machines:
                return False
            if len(self.order) >= MACHINES_MAX:
                raise FleetError(f'Switchboard keeps up to {MACHINES_MAX} machines.')
            self.state[m['name']] = self._blank()
            self.machines = dict(self.machines, **{m['name']: m})
            self.order = self.order + [m['name']]
        if save:
            self._saved()
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
        if save:
            self._saved()
        return True

    def rename(self, name, label):
        label = label.strip() if isinstance(label, str) else ''
        if not LABEL_RE.fullmatch(label or '\x00'):
            raise FleetError('A name is 1 to 40 characters.')
        return self._edit(name, label=label)

    def set_root(self, name, root):
        """The machine's Claude folder, checked for its own system: Windows takes C:\\... and \\\\server\\share,
        the others take /..., and every system takes ~ and ~/...."""
        with self.lock:
            os_ = (self.machines.get(name) or {}).get('os')
        return self._edit(name, root=check_root(root, windows=os_ == 'windows'))

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
            if not isinstance(names, list) or sorted(names) != sorted(self.order):
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
        self._saved()
        return True

    def _list_req(self, m):
        """The poll. It tells the agent the name the server knows it by, its folder and where to report, so nothing
        about a machine has to be set up on the machine itself."""
        req = {'verb': 'list', 'as': m['name']}
        if m.get('root'):
            req['root'] = m['root']
        if self.servers:
            req['servers'] = list(self.servers)
        if getattr(self, 'summary_model', None):
            req['summary_model'] = self.summary_model
        view = getattr(self, 'budget_view', None)  # the usage budget's status, cached by the agent for its hooks
        if view:
            try:
                b = view()
            except Exception:
                b = None
            if b:
                req['budget'] = b
        return req

    def _poll_one(self, name, timeout):
        m = self.machines.get(name)
        if m is None:  # unpaired while the round ran
            return
        asked = self.clock()  # live guesses older than this are settled by the answer (server.App overlay)
        r = None
        try:
            r = self.transport(m['target'], self._list_req(m), timeout)
            if not r.get('ok'):
                raise OSError(r.get('error') or 'list failed')
            got = self.clock()
            sent = _num(r.get('clock')) or _num(r.get('now'))  # the machine's own clock as it answered
            upd = {'online': True, 'error': None, 'sessions': r.get('sessions', []), 'replaced': r.get('replaced') or [], 'limits': r.get('limits'),
                   'updated': time.time(), 'as_of': got, 'skew': round(got - sent, 3) if sent else 0.0, 'root': r.get('root'), 'launcher': r.get('launcher'), 'mem': r.get('mem'),
                   'features': r.get('features') if isinstance(r.get('features'), list) else [], 'asked': asked,
                   'versions': r.get('versions') if isinstance(r.get('versions'), dict) else None,
                   'setup': [i for i in r.get('setup') or [] if isinstance(i, dict) and i.get('id')][:12],
                   'root_set': r.get('root_set') if isinstance(r.get('root_set'), bool) else None,
                   'root_suggest': r.get('root_suggest') if isinstance(r.get('root_suggest'), str) else None}
        except Exception as e:
            upd, r = {'online': False, 'error': _short(e), 'asked': asked}, None
        if r is not None:
            self.learn(name, r)
        self.store(name, upd)

    def store(self, name, upd):
        """Keep what a machine said; tell the server when it differs from last time (not just its timestamps)."""
        digest = json.dumps({k: v for k, v in upd.items() if k not in ('updated', 'asked', 'as_of', 'skew')}, sort_keys=True, default=str)
        with self.lock:
            if name not in self.state:
                return
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
        if self.history is not None:
            lim = self._newest_limits(self.snapshot()['machines'])
            self.history.record(lim.get('seven_day'), lim.get('seven_day_reset'), self.clock())

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
    def _newest_limits(snap):
        lims = [m['limits'] for m in snap if m.get('limits')]
        return max(lims, key=lambda l: l.get('updated') or 0) if lims else {}

    def summary(self):
        snap = self.snapshot()['machines']
        newest, now = self._newest_limits(snap), self.clock()
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
        newest, now = self._newest_limits(self.snapshot()['machines']), self.clock()
        week, reset = newest.get('seven_day'), newest.get('seven_day_reset')
        days, weeks = self.history.usage(week, reset, now) if self.history is not None else ([], [])
        return {'ok': True, 'now': now, 'tz': DAY_TZ,
                'five': {'pct': newest.get('five_hour'), 'reset': newest.get('five_hour_reset')},
                'week': {'pct': week, 'reset': reset, 'pace': _projected(week, reset, now)},
                'days': days, 'weeks': weeks}

    def call(self, name, req, timeout=40):
        m = self.machines.get(name)
        if not m:
            raise FleetError('unknown machine')
        try:
            return self.transport(m['target'], req, timeout)
        except Exception as e:
            self.store(name, {'online': False, 'error': _short(e)})  # announced now, not at the next poll
            raise FleetError(f'{m["label"]} is not reachable right now ({_short(e)})')


def _running(sessions):
    return sum(1 for s in sessions if s.get('activity') == 'working')


def _cold(sessions):
    return sum(1 for s in sessions if s.get('cache') == 'cold' and s.get('heavy') and s.get('activity') == 'quiet')


def _projected(used, reset, now, week=WEEK):
    """Weekly use at reset if the rest of the week goes like the part so far."""
    if used is None or not reset:
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


def day_used(samples, start, end, tz=DAY_TZ):
    """[{day, used}] for each local calendar day from `start` (the week's start) to `end`: the weekly percent at the
    day's last sample minus its first, never negative. The week's first day counts from 0 at the reset. None for a
    day with too few samples to tell."""
    tz = _zone(tz)
    by = {}
    for t, p in sorted(samples):
        if start < t <= end and p is not None:
            by.setdefault(datetime.fromtimestamp(t, tz).date(), []).append(float(p))
    first = datetime.fromtimestamp(start, tz).date()
    by.setdefault(first, []).insert(0, 0.0)
    out, d, last = [], first, datetime.fromtimestamp(max(start, end), tz).date()
    while d <= last:
        v = by.get(d) or []
        out.append({'day': d.isoformat(), 'used': round(max(0.0, v[-1] - v[0]), 1) if len(v) > 1 else
                    (0.0 if d == first else None)})
        d += timedelta(days=1)
    return out


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
        try:
            weeks = json.loads(self.archive.read_text())['weeks']
            self.weeks = [w for w in weeks if isinstance(w, dict) and isinstance(w.get('reset'), (int, float))][:self.WEEKS]
        except (OSError, ValueError, TypeError, KeyError):
            pass
        try:
            d = json.loads(self.path.read_text())
            reset = float(d['reset'])
            samples = sorted([float(t), float(p)] for t, p in d['samples'])
        except (OSError, ValueError, TypeError, KeyError):
            return
        self.reset, self.samples = reset, samples[-self.KEEP:]

    def record(self, pct, reset, now):
        """Keeps the weekly percent now if the last sample is 15 minutes old. True when it was written down."""
        if pct is None or not reset or reset <= now:  # unknown, or numbers from a week that is over
            return False
        with self.lock:
            if self.reset is None or abs(reset - self.reset) > RESET_SLACK:
                self._archive()
                self.samples = []  # a new week (or an early reset): what was kept belongs to the last one
            self.reset = reset
            self.samples = [s for s in self.samples if reset - WEEK <= s[0] <= now]
            if self.samples and now - self.samples[-1][0] < self.every:
                return False
            self.samples = (self.samples + [[float(now), float(pct)]])[-self.KEEP:]
            try:
                tmp = self.path.with_name(f'{self.path.name}.{os.getpid()}.tmp')
                tmp.write_text(json.dumps({'reset': self.reset, 'samples': self.samples}))
                os.replace(tmp, self.path)
            except OSError:
                pass  # kept in memory; the next sample writes the file again
            return True

    def _archive(self):
        """Keep the week that just ended (its last percent and its days) in week_archive.json. Called under the lock."""
        if self.reset is None or not self.samples:
            return
        old = self.reset
        week = {'reset': old, 'pct': round(self.samples[-1][1], 1),
                'days': day_used(self.samples, old - WEEK, min(old, self.samples[-1][0]), self.tz)}
        self.weeks = ([week] + [w for w in self.weeks if abs(w['reset'] - old) > RESET_SLACK])[:self.WEEKS]
        self.weeks.sort(key=lambda w: -w['reset'])
        try:
            tmp = self.archive.with_name(f'{self.archive.name}.{os.getpid()}.tmp')
            tmp.write_text(json.dumps({'weeks': self.weeks}))
            os.replace(tmp, self.archive)
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
        if pct is not None:
            samples.append((now, float(pct)))
        return day_used(samples, reset - WEEK, now, self.tz), weeks

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
