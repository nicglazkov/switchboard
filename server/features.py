"""Switchboard v2 on the server: cleanup (chats nobody uses, offered for a one-click stop), threads (one chat per topic),
the auto machine (Claude picks where a task runs) and dispatch (a session asks another machine for work).

Kept apart from server.py so the parallel builds merge cleanly. server.App owns one Features and calls:
after_poll() from the poller, decorate(snap) from sessions(), get()/post()/action() from its routes, and
job_done() when a server job finishes."""
import json, os, re, threading, time, traceback, urllib.request
from pathlib import Path

import budget as budget_mod
import library as library_mod

SID_RE = re.compile(r'[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}')
NAME_RE = re.compile(r"[A-Za-z0-9 ._,:'+#()-]{1,64}")
HUB_OPEN = 'http://127.0.0.1:8132/internal/open'
LIVE = ('idle', 'busy', 'waiting')  # a background session that is running (stopped ones report 'stopped' or nothing)


def log(msg):
    print(time.strftime('%Y-%m-%d %H:%M:%S'), msg, flush=True)


def jload(p, default):
    try:
        d = json.loads(Path(p).read_text())
        return d if isinstance(d, type(default)) else default
    except (OSError, ValueError):
        return default


def jsave(p, obj):
    p = Path(p)
    tmp = p.with_name(f'{p.name}.{os.getpid()}.{threading.get_ident()}.tmp')
    tmp.write_text(json.dumps(obj, indent=1))
    os.replace(tmp, p)


def hub_open(url=HUB_OPEN, timeout=1.5):
    """{(machine, bg)} with a terminal open on the hub right now. Empty when the hub is down or says nothing."""
    try:
        with urllib.request.urlopen(url, timeout=timeout) as r:
            d = json.loads(r.read() or b'[]')
    except Exception:
        return set()
    rows = d.get('open') if isinstance(d, dict) else d
    return {(x.get('machine'), x.get('bg')) for x in rows or [] if isinstance(x, dict)}


def quiet_s(s, now):
    """Seconds since anything was written in the chat."""
    return now - max(s.get('mtime') or 0, s.get('active') or 0)


# ---------------------------------------------------------------- Cleanup (spec section 2; was the reaper)
# Nothing here stops anything by itself. It finds background chats nobody is using (archived in the Claude app, idle
# for days, stuck, replaced by a newer chat) and leftover claude processes, says why and how much memory they hold,
# and the apps show them in a card: Stop all, Stop, Keep. Only a person's click stops one.
REAPER_DEFAULTS = {'enabled': True, 'idle_h': 72, 'stalled_h': 12, 'replaced_h': 1, 'archived_h': 1}
IDLE_MIN_H = 72          # an idle background chat younger than three days is never suggested
ORPHAN_H = 6
TABS_KEEP = 30 * 86400   # an app that has not sent its tabs for this long is gone; its tabs stop counting
REAPED_KEEP = 200
STOPPED_SHOW = 86400     # a chat stopped from the card stays there for a day, with Resume
PUSH_EVERY = 86400       # the "not in use" notification comes at most once a day
CLEANUP_URL = '/?cleanup=1'


def _zone():
    try:
        from zoneinfo import ZoneInfo
        from fleet import server_tz
        return ZoneInfo(server_tz())
    except Exception:
        return None


def day_label(ts):
    """'Sep 29' in the user's time zone (the server runs in UTC)."""
    import datetime as _dt
    d = _dt.datetime.fromtimestamp(ts, _zone())
    return f'{d.strftime("%b")} {d.day}'


def span(sec):
    """'19 days', '7 hours', '40 minutes'."""
    sec = max(0, int(sec))
    for unit, n in (('day', 86400), ('hour', 3600), ('minute', 60)):
        if sec >= n:
            k = sec // n
            return f'{k} {unit}{"s" if k != 1 else ""}'
    return 'a moment'


def last_use(s):
    """When the chat was last used: its own last message (or, from an older agent, the transcript's write time and
    last reply), or the newest write by its subagents and background commands, whichever is later."""
    own = s.get('last_ts') if s.get('last_ts') is not None else max(s.get('mtime') or 0, s.get('active') or 0)
    return max(own or 0, s.get('child_ts') or 0)


def why_text(reason, s, now):
    if reason == 'archived':
        return f'Archived in the Claude app {day_label(s["archived"])}'
    q = now - last_use(s)
    if reason == 'idle':
        return f'Idle {span(q)}'
    if reason == 'stalled':
        return f'Says it is working, but nothing happened for {span(q)}'
    return f'Replaced by a newer chat {span(q)} ago'


class Reaper:
    def __init__(self, app, cfg_path, opened=hub_open):
        self.app, self.cfg_path, self.opened = app, Path(cfg_path), opened
        self.keep_path, self.log_path = app.state_dir / 'keep.json', app.state_dir / 'reaped.json'
        self.seen_path = app.state_dir / 'cleanup_seen.json'
        self.tabs_path = app.state_dir / 'tabs.json'
        self.lock, self.busy = threading.Lock(), threading.Lock()
        self._open = (0, set())

    def open_now(self):
        """Terminals open on the hub, asked at most every 10 s (every session list reads it)."""
        t = time.time()
        if t - self._open[0] > 10:
            self._open = (t, self.opened())
        return self._open[1]

    def config(self):
        """reaper.json, read on every run so a change needs no restart. A broken file turns suggestions off."""
        try:
            d = json.loads(self.cfg_path.read_text())
            if not isinstance(d, dict):
                raise ValueError('not an object')
        except FileNotFoundError:
            return dict(REAPER_DEFAULTS)
        except (OSError, ValueError) as e:
            return dict(REAPER_DEFAULTS, enabled=False, error=f'reaper.json unreadable: {e}'[:160])
        cfg = dict(REAPER_DEFAULTS)
        for k, v in d.items():
            if k == 'enabled' and isinstance(v, bool):
                cfg[k] = v
            elif k in ('idle_h', 'stalled_h', 'replaced_h', 'archived_h') and isinstance(v, (int, float)) and v > 0:
                cfg[k] = v
        cfg['idle_h'] = max(IDLE_MIN_H, cfg['idle_h'])
        return cfg

    # ---- Keep (never suggest it again)
    def keep(self):
        return jload(self.keep_path, {})

    # ---- tabs the apps have open: never suggested, connected or not (the app quit, the Mac asleep)
    def set_tabs(self, device, keys):
        """A desktop app's saved tabs, replacing what it sent before. keys: "machine:sid" or "machine/sid"."""
        if not isinstance(device, str) or not re.fullmatch(r'[a-z0-9][a-z0-9-]{0,39}', device):
            raise ValueError('bad device')
        if not isinstance(keys, list) or len(keys) > 200:
            raise ValueError('tabs is a list of up to 200 chats')
        out = []
        for k in keys:
            m = re.fullmatch(r'([a-z0-9]{1,32})[:/]([0-9a-f-]{36})', k) if isinstance(k, str) else None
            if m:  # anything else (an older key, a tab still starting) is left out, not the whole list
                out.append(f'{m.group(1)}/{m.group(2)}')
        with self.lock:
            d = jload(self.tabs_path, {})
            d[device] = {'at': self.app.clock(), 'keys': sorted(set(out))}
            jsave(self.tabs_path, d)
        return len(out)

    def tab_keys(self, now=None):
        """{"machine/sid"} open in a tab of an app that reported in the last TABS_KEEP days."""
        now = now or self.app.clock()
        out = set()
        for v in jload(self.tabs_path, {}).values():
            if isinstance(v, dict) and now - (v.get('at') or 0) < TABS_KEEP:
                out.update(k for k in v.get('keys') or [] if isinstance(k, str))
        return out

    def set_keep(self, machine, sid, on=None):
        with self.lock:
            d = self.keep()
            key = f'{machine}/{sid}'
            on = (key not in d) if on is None else bool(on)
            if on:
                d[key] = self.app.clock()
            else:
                d.pop(key, None)
            jsave(self.keep_path, d)
        return on

    # ---- the rules
    def decide(self, machines, now, cfg, keep, opened):
        """Every background chat (never a terminal one) and leftover process the rules would suggest stopping now:
        [{machine, label, sid, bg_id, title, reason, why, since, idle_h, mb}] (orphans carry pid instead of sid)."""
        out = []
        for m in machines:
            if not m.get('online'):
                continue
            label = m.get('label') or m['name']
            rows = [(s, False) for s in m.get('sessions') or []] + [(r, True) for r in m.get('replaced') or []]
            seen = set()
            for s, replaced in rows:
                sid = s.get('sid')
                if (s.get('kind') != 'background' or not s.get('bg_id') or s.get('status') not in LIVE or sid in seen
                        or (m['name'], s['bg_id']) in opened or f'{m["name"]}/{sid}' in keep):
                    continue
                seen.add(sid)
                used = last_use(s)
                q, busy = now - used, s.get('status') == 'busy'
                arch = s.get('archived')
                if arch and now - max(arch, s.get('child_ts') or 0) >= cfg['archived_h'] * 3600 and q >= cfg['archived_h'] * 3600:
                    reason, since = 'archived', arch
                elif replaced and not busy and q >= cfg['replaced_h'] * 3600:
                    reason, since = 'replaced', used
                elif busy and q >= cfg['stalled_h'] * 3600:
                    reason, since = 'stalled', used
                elif not busy and q >= cfg['idle_h'] * 3600:
                    reason, since = 'idle', used
                else:
                    continue
                out.append({'machine': m['name'], 'label': label, 'sid': sid, 'bg_id': s['bg_id'], 'title': s.get('title'),
                            'reason': reason, 'why': why_text(reason, s, now), 'since': since, 'idle_h': int(q // 3600),
                            'mb': s.get('mb') if isinstance(s.get('mb'), int) else None})
            for o in (m.get('mem') or {}).get('orphans') or []:
                age = o.get('age_h') or 0
                if age < ORPHAN_H or not isinstance(o.get('pid'), int) or f'{m["name"]}/pid:{o["pid"]}' in keep:
                    continue
                out.append({'machine': m['name'], 'label': label, 'pid': o['pid'], 'title': 'Leftover Claude process',
                            'reason': 'orphan', 'why': f'No chat owns it; started {span(age * 3600)} ago',
                            'since': now - age * 3600, 'idle_h': int(age), 'mb': o.get('mb') if isinstance(o.get('mb'), int) else None})
        out.sort(key=lambda d: -(d['mb'] or 0))
        return out

    def decisions(self, cfg=None):
        cfg = cfg or self.config()
        if not cfg['enabled']:
            return []
        keep = set(self.keep()) | self.tab_keys()  # a chat open in a tab is in use, even while the app is closed
        return self.decide(self.app.fleet.snapshot()['machines'], self.app.clock(), cfg, keep, self.open_now())

    # ---- the log of what a person stopped
    def entries(self):
        return jload(self.log_path, [])

    def _log(self, entry):
        with self.lock:
            rows = [r for r in self.entries() if not r.get('dry_run')]  # suggestions from before are not stops
            rows.append(entry)
            jsave(self.log_path, rows[-REAPED_KEEP:])
        what = 'could not stop' if entry.get('error') else 'stopped'
        log(f'cleanup: {what} {entry["machine"]} {entry.get("sid") or "pid " + str(entry.get("pid"))} ({entry["reason"]})'
            + (f': {entry["error"]}' if entry.get('error') else ''))

    def reaped(self):
        """{(machine, sid): newest stop} for the session JSON."""
        out = {}
        for r in self.entries():
            if not r.get('dry_run') and not r.get('error') and r.get('sid'):
                out[(r['machine'], r['sid'])] = r
        return out

    # ---- after each poll: tell the phone when there is something new to clean up (once a day at most)
    def run(self):
        if not self.busy.acquire(blocking=False):
            return
        try:
            cfg = self.config()
            if not cfg['enabled']:
                return
            now, items = self.app.clock(), self.decisions(cfg)
            seen = jload(self.seen_path, {})
            keys = {self.key(d) for d in items}
            new = [d for d in items if self.key(d) not in seen.get('keys', {})]
            seen['keys'] = {k: seen.get('keys', {}).get(k, now) for k in keys}  # forget what went away
            if new and now - (seen.get('pushed') or 0) >= PUSH_EVERY:
                seen['pushed'] = now
                self.app._push(*self.notice(items))
            jsave(self.seen_path, seen)
        except Exception:
            log('cleanup failed: ' + traceback.format_exc())
        finally:
            self.busy.release()

    @staticmethod
    def key(d):
        return f'{d["machine"]}/{d.get("sid") or "pid:" + str(d.get("pid"))}'

    @staticmethod
    def notice(items):
        n, mb = len(items), sum(d['mb'] or 0 for d in items)
        what = 'session isn\'t' if n == 1 else 'sessions aren\'t'
        title = f'{n} {what} in use'
        body = (f'They hold {mb_text(mb)}. ' if mb and n > 1 else f'It holds {mb_text(mb)}. ' if mb else '') + \
            'Tap to look; nothing stops until you say so.'
        return title, body, CLEANUP_URL

    # ---- what a person asked for
    def stop(self, items):
        """Stop what a person picked in the card, if the rules still say it is not in use (the machines are asked
        again first). {'stopped', 'skipped', 'failed'}."""
        want = []
        for it in items:
            if not isinstance(it, dict) or not isinstance(it.get('machine'), str):
                raise ValueError('bad item')
            if it.get('sid') is not None and not (isinstance(it['sid'], str) and SID_RE.fullmatch(it['sid'])):
                raise ValueError('bad session id')
            if it.get('pid') is not None and not (isinstance(it['pid'], int) and it['pid'] > 1):
                raise ValueError('bad process id')
            if (it.get('sid') is None) == (it.get('pid') is None):
                raise ValueError('an item is a session (sid) or a process (pid)')
            want.append(self.key(it))
        if len(want) > 100:
            raise ValueError('too many items')
        for name in {k.split('/', 1)[0] for k in want}:
            self.app.fleet.poll_one(name)
        self._open = (0, set())  # a terminal opened a moment ago counts
        now = self.app.clock()
        now_items = {self.key(d): d for d in self.decisions()}
        out = {'stopped': [], 'skipped': [], 'failed': []}
        orphans = {}
        for k in want:
            d = now_items.get(k)
            if not d:
                out['skipped'].append(k)  # used again since, kept, already gone, or never on the card
                log(f'cleanup: skipped {k}: not in use by the rules now')
                continue
            if d.get('pid'):
                orphans.setdefault(d['machine'], []).append(d)
                continue
            try:
                self.app._call(d['machine'], {'verb': 'stop', 'sid': d['sid']}, timeout=40)
                self.app.fleet.mark_stopped(d['machine'], d['sid'])
                self._log({'at': now, 'machine': d['machine'], 'sid': d['sid'], 'title': d.get('title'), 'reason': d['reason'],
                           'mb': d['mb'], 'idle_h': d['idle_h'], 'by': 'user'})
                out['stopped'].append(k)
            except Exception as e:
                self._log({'at': now, 'machine': d['machine'], 'sid': d['sid'], 'reason': d['reason'], 'error': str(e)[:160], 'by': 'user'})
                out['failed'].append({'key': k, 'error': str(e)[:160]})
        for name, ds in orphans.items():
            try:
                r = self.app._call(name, {'verb': 'reap_orphans', 'min_age_h': ORPHAN_H, 'pids': [d['pid'] for d in ds]}, timeout=60)
                killed = {o.get('pid') for o in r.get('killed') or []}
            except Exception as e:
                killed, err = set(), str(e)[:160]
            else:
                err = 'not stopped'
            for d in ds:
                if d['pid'] in killed:
                    self._log({'at': now, 'machine': name, 'pid': d['pid'], 'reason': 'orphan', 'mb': d['mb'], 'idle_h': d['idle_h'], 'by': 'user'})
                    out['stopped'].append(self.key(d))
                else:
                    out['failed'].append({'key': self.key(d), 'error': err})
        for name in {k.split('/', 1)[0] for k in want}:
            self.app._refresh_soon(name, 1)
        return out

    # ---- what clients see
    def report(self):
        """The card: what could go, how much memory it holds, and what was stopped from it today (with Resume)."""
        cfg, now = self.config(), self.app.clock()
        items = self.decisions(cfg)
        live = {(m['name'], s.get('sid')): s for m in self.app.fleet.snapshot()['machines']
                for s in (m.get('sessions') or []) + (m.get('replaced') or [])}
        stopped, shown = [], set()
        for r in reversed(self.entries()):
            k = (r.get('machine'), r.get('sid'))
            if r.get('error') or r.get('dry_run') or not r.get('sid') or k in shown or now - (r.get('at') or 0) > STOPPED_SHOW:
                continue
            shown.add(k)
            s = live.get(k) or {}
            if s.get('status') in LIVE:
                continue  # running again
            stopped.append({'machine': r['machine'], 'label': self.app._label(r['machine']), 'sid': r['sid'],
                            'title': s.get('title') or r.get('title'), 'reason': r.get('reason'), 'at': r['at'], 'mb': r.get('mb')})
        return {'ok': True, 'enabled': cfg['enabled'], 'items': items, 'count': len(items),
                'mb': sum(d['mb'] or 0 for d in items), 'stopped': stopped}

    def decorate(self, snap):
        keep, reaped = self.keep(), self.reaped()
        rep = self.report()
        by = {(d['machine'], d.get('sid')): d for d in rep['items'] if d.get('sid')}
        for m in snap['machines']:
            for s in (m.get('sessions') or []) + (m.get('replaced') or []):
                if f'{m["name"]}/{s.get("sid")}' in keep:
                    s['keep'] = True
                d = by.get((m['name'], s.get('sid')))
                if d:
                    s['cleanup'] = {'reason': d['reason'], 'why': d['why']}
                r = reaped.get((m['name'], s.get('sid')))
                if r and s.get('status') in (None, 'stopped') and (s.get('active') or 0) <= r['at'] + 60:
                    s['reaped'] = {'at': r['at'], 'reason': r['reason'], 'idle_h': r.get('idle_h')}
        snap['cleanup'] = {k: rep[k] for k in ('items', 'count', 'mb', 'stopped')}


def mb_text(mb):
    return f'{mb / 1024:.1f} GB' if mb >= 1024 else f'{int(mb)} MB'


# ---------------------------------------------------------------- Threads (spec section 3)
MONTHS = 'Jan|Feb|Mar|Apr|May|Jun|Jul|Aug|Sep|Oct|Nov|Dec'
STAMP_RE = re.compile(r'^(?:' + MONTHS + r') \d{1,2} \d{1,2}:\d{2}[ap]\s+')
TAG_RE = re.compile(r"\s*\((?:fresh(?: \d{1,3})?|part \d{1,3}|resumed|copy|from [A-Za-z0-9 ]{1,20}|for [A-Za-z0-9 ]{1,20}|\d{1,3}|"
                    r"(?:new|UltraCode|Max effort)(?:, [A-Za-z]{1,12})?)\)\s*$")
LEGACY_FRESH_RE = re.compile(r'(?:\s+fresh)+$', re.I)
FRESH_TAG_RE = re.compile(r'(?:\((?:fresh(?: \d{1,3})?|part \d{1,3})\)|\sfresh)$', re.I)  # the tag Start fresh gives the new chat
STOP = {'a', 'an', 'the', 'and', 'or', 'of', 'for', 'to', 'in', 'on', 'with', 'my', 'i', 'is', 'it', 'at', 'by', 'from'}
SAME_TOPIC = 0.6
OVERLAP = 120          # the older chat may have been used up to this long after the newer one started
CHATS_KEEP = 2000      # chats remembered for history (they leave the machines' lists after three weeks)
CHATS_DAYS = 120
LOOK_BACK = 200        # the same-topic rule looks this many older chats back in a folder
HOWS = {'fresh': 'fresh', 'cleared': 'fresh', 'resumed': 'resumed', 'continued': 'continued', 'handoff': 'handoff',
        'same-name': 'same-topic', 'renamed': 'same-topic', 'same-topic': 'same-topic', 'linked': 'linked', 'compacted': 'compacted'}


def topic_of(title):
    """A chat's topic: its name without the start stamp or the trailing tag Switchboard adds."""
    t = ' '.join(str(title or '').split())
    t = STAMP_RE.sub('', t)
    for _ in range(3):
        t2 = LEGACY_FRESH_RE.sub('', TAG_RE.sub('', t)).strip()
        if t2 == t or not t2:
            break
        t = t2
    return t


def topic_words(title):
    return {w for w in re.findall(r'[a-z0-9]+', topic_of(title).lower()) if w not in STOP}


def jaccard(a, b):
    return len(a & b) / len(a | b) if a and b else 0.0


def repo_key(cwd):
    return re.split(r'[\\/]\.claude[\\/]worktrees[\\/]', cwd or '')[0].replace('\\', '/').rstrip('/').casefold()


def ck(machine, sid):
    return f'{machine}/{sid}'


class Stars:
    """Starred chats: {sid: when}, a chat's own sid and its thread's first. Rows get star: true when either matches, so
    a star survives Start fresh, Continue and Resume (the new chat joins the same thread)."""

    def __init__(self, path, threads=None):
        self.path, self.lock, self.threads = Path(path), threading.Lock(), threads

    def all(self):
        return jload(self.path, {})

    def starred_threads(self, d):
        """Thread roots holding any starred chat: a star stays on through /clear, Start fresh, Continue, Resume and
        a later link that gives the thread an older first chat."""
        if not self.threads:
            return set()
        try:
            chats, _, roots, _ = self.threads.graph()
        except Exception:
            return set()
        out = set()
        for k, r in roots.items():
            if (chats.get(k) or {}).get('sid') in d:
                out.add((chats.get(r) or {}).get('sid'))
        return out

    def set(self, sid, thread=None, on=None, others=()):
        """Star or unstar a chat; `others` are the other chats of its thread, so an unstar clears every part."""
        with self.lock:
            d = self.all()
            keys = {sid} | ({thread} if isinstance(thread, str) and SID_RE.fullmatch(thread) else set())
            every = keys | {x for x in others if isinstance(x, str) and SID_RE.fullmatch(x)}
            on = not (every & d.keys()) if on is None else bool(on)
            for k in (keys if on else every):
                if on:
                    d[k] = time.time()
                else:
                    d.pop(k, None)
            jsave(self.path, d)
        return on

    def decorate(self, snap):
        d = self.all()
        if not d:
            return
        roots = self.starred_threads(d)
        for m in snap['machines']:
            for s in (m.get('sessions') or []) + (m.get('replaced') or []):
                if s.get('sid') in d or s.get('thread') in d or s.get('thread') in roots:
                    s['star'] = True


GROUP_COLORS = ('grey', 'blue', 'red', 'yellow', 'green', 'pink', 'purple', 'cyan')
GROUP_NAME_MAX = 40
GROUP_ID_RE = re.compile(r'[0-9a-f]{8}')


class Groups:
    """Tab groups, shared by every Mac: [{id, name, color, members: [thread sid, ...]}], in the order the tab row shows
    them. Members are thread sids, so a group follows Start fresh, Continue and Resume like a star does. A thread is in
    at most one group, and a group left empty is gone. Which tabs are open, and whether a group is collapsed, stays on
    each Mac."""

    def __init__(self, path, http_error):
        self.path, self.lock, self.HttpError = Path(path), threading.Lock(), http_error

    def all(self):
        out = []
        for g in jload(self.path, []):
            if isinstance(g, dict) and isinstance(g.get('id'), str) and isinstance(g.get('members'), list):
                out.append({'id': g['id'], 'name': g.get('name') if isinstance(g.get('name'), str) else '',
                            'color': g.get('color') if g.get('color') in GROUP_COLORS else 'grey',
                            'members': [t for t in g['members'] if isinstance(t, str)]})
        return out

    def _bad(self, msg):
        raise self.HttpError(400, msg)

    def _name(self, v):
        if not isinstance(v, str):
            self._bad('A group name is text.')
        v = ' '.join(v.split())
        if len(v) > GROUP_NAME_MAX:
            self._bad(f'A group name is up to {GROUP_NAME_MAX} characters.')
        if any(ord(c) < 32 for c in v):
            self._bad('A group name cannot hold control characters.')
        return v

    def _color(self, v):
        if v not in GROUP_COLORS:
            self._bad('A group color is one of ' + ', '.join(GROUP_COLORS) + '.')
        return v

    def _thread(self, v):
        if not (isinstance(v, str) and SID_RE.fullmatch(v)):
            self._bad('bad thread id')
        return v

    def _index(self, v, n):
        if v is None:
            return n
        if not isinstance(v, int) or isinstance(v, bool) or v < 0:
            self._bad('index is a whole number from 0')
        return min(v, n)

    def _find(self, gs, gid):
        if not (isinstance(gid, str) and GROUP_ID_RE.fullmatch(gid)):
            self._bad('bad group id')
        for g in gs:
            if g['id'] == gid:
                return g
        raise self.HttpError(404, 'That group is gone.')

    def _new_id(self, gs):
        taken = {g['id'] for g in gs}
        while True:
            gid = os.urandom(4).hex()
            if gid not in taken:
                return gid

    @staticmethod
    def _take(gs, thread):
        for g in gs:
            if thread in g['members']:
                g['members'].remove(thread)

    def op(self, body):
        """POST /api/groups: {op: create|update|delete|add|remove|move, ...}. Answers {ok, groups} (and the group for
        create)."""
        op = body.get('op')
        if op not in ('create', 'update', 'delete', 'add', 'remove', 'move'):
            self._bad('unknown group op')
        with self.lock:
            gs, out = self.all(), {}
            had = {g['id'] for g in gs if g['members']}
            if op == 'create':
                name = self._name(body.get('name', ''))
                color = self._color(body.get('color', 'grey'))
                members = body.get('members', [])
                if not isinstance(members, list) or len(members) > 500:
                    self._bad('members is a list of thread ids')
                members = list(dict.fromkeys(self._thread(t) for t in members))
                for t in members:
                    self._take(gs, t)
                g = {'id': self._new_id(gs), 'name': name, 'color': color, 'members': members}
                gs.append(g)
                out['group'] = g
            elif op == 'update':
                g = self._find(gs, body.get('id'))
                if 'name' in body:
                    g['name'] = self._name(body['name'])
                if 'color' in body:
                    g['color'] = self._color(body['color'])
            elif op == 'delete':  # ungroups; never closes a chat
                g = self._find(gs, body.get('id'))
                gs.remove(g)
            elif op == 'add':
                g = self._find(gs, body.get('id'))
                t = self._thread(body.get('thread'))
                self._take(gs, t)
                g['members'].insert(self._index(body.get('index'), len(g['members'])), t)
            elif op == 'remove':
                self._take(gs, self._thread(body.get('thread')))
            elif op == 'move':
                g = self._find(gs, body.get('id'))
                gs.remove(g)
                gs.insert(self._index(body.get('index'), len(gs)), g)
            gs = [g for g in gs if g['members'] or g['id'] not in had]  # emptied by this change: gone
            jsave(self.path, gs)
        return dict(out, ok=True, groups=gs)

    def decorate(self, snap):
        gs = self.all()
        snap['groups'] = gs
        of = {t: g['id'] for g in gs for t in g['members']}
        if not of:
            return
        for m in snap['machines']:
            for s in (m.get('sessions') or []) + (m.get('replaced') or []):
                gid = of.get(s.get('thread') or s.get('sid'))
                if gid:
                    s['group'] = gid


class Threads:
    """Chats linked into threads, oldest first. Links, strongest first: manual ("This continues..."), how a chat was
    made (Start fresh, Resume, Continue: from the server's jobs and the machines' replaced marks), a chat that read
    another's handoff file, then the same topic in the same folder. "Not the same topic" makes a chat start its own
    thread. Every chat ever seen is remembered (chats.json), so history outlives the machines' three-week lists."""

    def __init__(self, app):
        self.app = app
        self.path, self.chats_path = app.state_dir / 'threads.json', app.state_dir / 'chats.json'
        self.lock = threading.Lock()
        self.chats = jload(self.chats_path, {})
        self._sig, self._graph, self._version, self._dirty = None, None, 0, False

    # ---- stored state
    def store(self):
        d = jload(self.path, {})
        for k in ('links', 'auto', 'unlinked'):
            d.setdefault(k, {})
        return d

    def _save_store(self, d):
        jsave(self.path, d)

    def record(self, new, old, how):
        """A server job made `new` from `old` (keys "machine/sid")."""
        if new == old:
            return
        with self.lock:
            d = self.store()
            d['auto'][new] = {'to': old, 'how': HOWS.get(how, how), 'at': self.app.clock()}
            self._version += 1
            if len(d['auto']) > 3000:
                d['auto'] = dict(sorted(d['auto'].items(), key=lambda kv: -kv[1].get('at', 0))[:2000])
            self._save_store(d)

    def observe(self, machines, save=True):
        """Remember every chat the machines list, with what threads need. Written to disk after a poll."""
        changed = False
        with self.lock:
            for m in machines:
                for s in (m.get('sessions') or []) + (m.get('replaced') or []):
                    sid = s.get('sid')
                    if not isinstance(sid, str):
                        continue
                    k = ck(m['name'], sid)
                    was = self.chats.get(k) or {}
                    row = {'machine': m['name'], 'sid': sid, 'title': s.get('title') or was.get('title'),
                           'cwd': s.get('cwd') or was.get('cwd') or s.get('folder'),
                           'started': s.get('started') or was.get('started'),
                           'ended': s.get('active') or s.get('mtime') or was.get('ended'), 'ctx': s.get('ctx') or was.get('ctx') or 0,
                           'url': s.get('url') or was.get('url'), 'handoff_of': s.get('handoff_of') or was.get('handoff_of'),
                           'kind': s.get('kind'), 'status': s.get('status'), 'warm': warm(s),
                           'by': s.get('by') if isinstance(s.get('by'), dict) else was.get('by'),
                           'rhow': s.get('how') or was.get('rhow')}
                    if not row['started']:
                        row['started'] = min(row['ended'] or 0, s.get('mtime') or row['ended'] or 0) or None
                    if self.chats.get(k) != row:
                        old = self.chats.get(k) or {}
                        self.chats[k] = row
                        changed = True
                        if any(old.get(f) != row.get(f) for f in ('title', 'cwd', 'started', 'handoff_of', 'by', 'rhow', 'warm')) or \
                                (old.get('ended') or 0) // 600 != (row.get('ended') or 0) // 600:
                            self._version += 1  # something a link depends on: work the threads out again
            if changed:
                now = self.app.clock()
                keep = sorted(self.chats.items(), key=lambda kv: -(kv[1].get('ended') or 0))
                self.chats = {k: v for k, v in keep[:CHATS_KEEP] if now - (v.get('ended') or now) < CHATS_DAYS * 86400}
                self._dirty = True
            if save and self._dirty:
                try:
                    jsave(self.chats_path, self.chats)
                    self._dirty = False
                except OSError:
                    log('could not save chats: ' + traceback.format_exc(limit=1))

    # ---- the graph
    def parents(self, chats=None):
        """{child key: (parent key, how)}."""
        chats = chats if chats is not None else self.chats
        d = self.store()
        out = {}
        older = lambda a, b: (chats[a].get('started') or 0) < (chats[b].get('started') or 0)
        # how a chat was made: replaced marks (machine side) and server jobs
        for k, c in chats.items():
            by = c.get('by') or {}
            if by.get('sid'):
                new = ck(by.get('machine') or c['machine'], by['sid'])
                if new in chats and new != k and new not in out:
                    out[new] = (k, HOWS.get(c.get('rhow') or 'fresh', 'same-topic'))
        for new, e in d['auto'].items():
            if new in chats and e.get('to') in chats:
                out[new] = (e['to'], HOWS.get(e.get('how'), 'fresh'))
        # read another chat's handoff file
        for k, c in chats.items():
            h = c.get('handoff_of')
            if h and k not in out:
                src = ck(c['machine'], h)
                if src in chats and src != k:
                    out[k] = (src, 'handoff')
        # same topic, same folder, same machine: the newest older chat that is stopped, cold or reaped, and was not
        # used after this one started (two chats on one topic at once are two chats)
        groups = {}
        for k, c in chats.items():
            groups.setdefault((c['machine'], repo_key(c.get('cwd'))), []).append(k)
        for keys in groups.values():
            keys.sort(key=lambda k: chats[k].get('started') or 0)
            words = {k: topic_words(chats[k].get('title')) for k in keys}
            for i, k in enumerate(keys):
                if k in out:
                    continue
                st = chats[k].get('started') or 0
                for j in range(i - 1, max(-1, i - 1 - LOOK_BACK), -1):
                    o = keys[j]
                    oc = chats[o]
                    if (oc.get('ended') or 0) > st + OVERLAP or oc.get('warm') or not older(o, k):
                        continue
                    if jaccard(words[k], words[o]) >= SAME_TOPIC:
                        out[k] = (o, 'same-topic')
                        break
        # a chat Start fresh made, that nothing else links: a fresh start from a home-folder chat runs in the Claude
        # folder, and its handoff comes in without naming the file, so the rules above can all miss it. It joins the
        # newest older chat on its topic on the same machine, in any folder, that was done before it started.
        by_machine = {}
        for k, c in chats.items():
            by_machine.setdefault(c['machine'], []).append(k)
        for keys in by_machine.values():
            keys.sort(key=lambda k: chats[k].get('started') or 0)
            for i, k in enumerate(keys):
                if k in out or not FRESH_TAG_RE.search(' '.join(str(chats[k].get('title') or '').split())):
                    continue
                st, words = chats[k].get('started') or 0, topic_words(chats[k].get('title'))
                for j in range(i - 1, max(-1, i - 1 - LOOK_BACK), -1):
                    o = keys[j]
                    oc = chats[o]
                    if (oc.get('ended') or 0) > st + OVERLAP or oc.get('warm') or not older(o, k):
                        continue
                    if jaccard(words, topic_words(oc.get('title'))) >= SAME_TOPIC:
                        out[k] = (o, 'fresh')
                        break
        for k in d['unlinked']:
            out.pop(k, None)
        for k, e in d['links'].items():
            if k in chats and e.get('to') in chats:
                out[k] = (e['to'], 'linked')
        for k in list(out):  # a loop (machines that disagree, a manual link against a heuristic one): cut it
            seen, cur = [k], k
            while cur in out:
                cur = out[cur][0]
                if cur in seen:
                    weak = [x for x in seen if out.get(x, ('', 'linked'))[1] != 'linked'] or seen
                    out.pop(weak[-1], None)
                    break
                seen.append(cur)
        return out

    def root_of(self, k, parents):
        seen = {k}
        while k in parents:
            p = parents[k][0]
            if p in seen:  # a manual link closed a loop: stop here
                break
            seen.add(p)
            k = p
        return k

    def thread(self, machine, sid):
        k = ck(machine, sid)
        chats, parents, roots, members = self.graph()
        if k not in chats:
            return None
        root = roots[k]
        keys = sorted(members[root], key=lambda x: (chats[x].get('started') or 0, chats[x].get('ended') or 0))
        follow = {}
        for c in keys:  # how each chat ended: the link its first successor came by
            if c in parents:
                follow.setdefault(parents[c][0], parents[c][1])
        newest = max(keys, key=lambda x: (chats[x].get('ended') or 0, chats[x].get('started') or 0))  # the one used last (two fresh starts from one chat are two branches)
        out = []
        for c in keys:
            r = chats[c]
            out.append({'machine': r['machine'], 'label': self.app._label(r['machine']), 'sid': r['sid'], 'title': r.get('title'),
                        'started': r.get('started'), 'ended': r.get('ended'), 'ctx': r.get('ctx') or 0,
                        'how': 'current' if c == newest else follow.get(c, 'same-topic'), 'url': r.get('url'),
                        'this': c == k})
        return {'ok': True, 'thread': chats[root]['sid'], 'chats': out}

    # ---- the snapshot clients see
    def graph(self):
        """(chats, parents, roots, members), worked out again only when a chat or a link changed."""
        sig = (self._version, os.path.getmtime(self.path) if self.path.exists() else 0)
        if self._sig != sig:
            chats = dict(self.chats)
            parents = self.parents(chats)
            roots, members = {}, {}
            for k in chats:
                roots[k] = r = self.root_of(k, parents)
                members.setdefault(r, []).append(k)
            self._graph, self._sig = (chats, parents, roots, members), sig
        return self._graph

    def decorate(self, snap):
        self.observe(snap['machines'], save=False)
        chats, parents, roots, members = self.graph()
        order = lambda x: (chats[x].get('started') or 0, chats[x].get('ended') or 0)
        last = lambda x: (chats[x].get('ended') or 0, chats[x].get('started') or 0)  # the chat of a thread used last is shown
        newest = {}  # per thread, the newest chat a machine lists now (one that is gone from disk cannot hide the others)
        for m in snap['machines']:
            for s in (m.get('sessions') or []) + (m.get('replaced') or []):
                k = ck(m['name'], s.get('sid'))
                if k in chats:
                    r = roots[k]
                    if r not in newest or last(k) > last(newest[r]):
                        newest[r] = k
        for m in snap['machines']:
            keep, moved = [], []
            for s in m.get('sessions') or []:
                k = ck(m['name'], s.get('sid'))
                r = roots.get(k, k)
                st = order(k)[0] if k in chats else 0
                s['thread'] = chats[r]['sid'] if r in chats else s.get('sid')
                s['history'] = sum(1 for x in members.get(r, ()) if x != k and (chats[x].get('started') or 0) <= st)
                top = newest.get(r, k)
                live = s.get('status') in ('busy', 'waiting') or s.get('held') or s.get('state') == 'running'
                if top != k and not live:  # an older chat of a thread: folded under Replaced, the newest one is shown
                    moved.append(dict(s, how=follow_how(parents, k, chats) or 'same-topic',
                                      by={'machine': chats[top]['machine'], 'sid': chats[top]['sid']}))
                else:
                    keep.append(s)
            m['sessions'] = keep
            if moved:
                m['replaced'] = moved + list(m.get('replaced') or [])
            for s in m.get('replaced') or []:
                k = ck(m['name'], s.get('sid'))
                r = roots.get(k, k)
                s.setdefault('thread', chats[r]['sid'] if r in chats else s.get('sid'))
        return snap

    def search(self, q='', exclude=None, limit=40):
        """Chats for "This continues...": older than `exclude` (the chat being linked), its own folder first."""
        words = [w for w in re.findall(r'[a-z0-9]+', (q or '').lower())]
        _, _, roots, _ = self.graph()
        mine = roots.get(exclude)
        me = self.chats.get(exclude) or {}
        before, home = me.get('started'), (me.get('machine'), repo_key(me.get('cwd')))
        rows = []
        for k, c in self.chats.items():
            if k == exclude or (before and (c.get('started') or 0) >= before):
                continue
            hay = f'{c.get("title") or ""} {c.get("cwd") or ""} {c["machine"]} {self.app._label(c["machine"])}'.lower()
            if all(w in hay for w in words):
                rows.append(c)
        rows.sort(key=lambda c: ((c['machine'], repo_key(c.get('cwd'))) != home, -(c.get('ended') or 0)))
        return [{'machine': c['machine'], 'label': self.app._label(c['machine']), 'sid': c['sid'], 'title': c.get('title'),
                 'started': c.get('started'), 'ended': c.get('ended'), 'ctx': c.get('ctx') or 0, 'folder': folder_name(c.get('cwd')),
                 'in_thread': bool(mine) and roots.get(ck(c['machine'], c['sid'])) == mine} for c in rows[:limit]]

    def link(self, child, to):
        chats = self.chats
        if child not in chats or to not in chats:
            raise ValueError('That chat is not one Switchboard has seen.')
        if child == to:
            raise ValueError('A chat cannot continue itself.')
        parents = self.parents()
        cur = to
        for _ in range(500):  # the older chat must not already come after this one
            if cur == child:
                raise ValueError('That chat already continues this one.')
            if cur not in parents:
                break
            cur = parents[cur][0]
        with self.lock:
            d = self.store()
            d['links'][child] = {'to': to, 'at': self.app.clock()}
            d['unlinked'].pop(child, None)
            self._save_store(d)
            self._version += 1

    def unlink_newer(self, k):
        """The chats that carry on from k go their own way ("Not the same topic" on an older chat)."""
        _, parents, _, _ = self.graph()
        kids = [c for c, (p, _) in parents.items() if p == k]
        for c in kids:
            self.unlink(c)
        return kids

    def unlink(self, child):
        with self.lock:
            d = self.store()
            d['links'].pop(child, None)
            d['unlinked'][child] = self.app.clock()
            self._save_store(d)
            self._version += 1


def warm(s):
    """In use right now: working (not stuck for hours), waiting for an answer, or open with its cache still warm."""
    st = s.get('status')
    return bool((st == 'busy' and not s.get('stalled')) or st == 'waiting' or s.get('held') or
                (st == 'idle' and not s.get('cold') and not s.get('stale')))


def folder_name(cwd):
    """The folder a chat ran in, as a person would say it."""
    p = (cwd or '').replace('\\', '/').rstrip('/')
    if re.fullmatch(r'(?i)(?:[a-z]:)?/(?:users|home)/[^/]+', p):
        return 'home folder'
    return p.split('/')[-1]


def follow_how(parents, k, chats):
    """How the chat after k came from it, for a chat folded under a newer one."""
    for c, (p, how) in sorted(parents.items(), key=lambda kv: chats.get(kv[0], {}).get('started') or 0):
        if p == k:
            return how
    return None


# ---------------------------------------------------------------- Features: what server.App talks to
# ---------------------------------------------------------------- v2: auto machine (Claude picks where a task runs)
# Each machine says what it can do (agent verb caps); which machines share that is the user's choice, in caps.json next to
# the other settings. A pick is Claude's (Haiku, on one machine that is signed in) with the rules below as the
# fallback, and as the instant preview while a message is typed.
CAPS_EVERY, CAPS_RETRY, CAPS_FRESH_S = 15 * 60, 5 * 60, 8
ASK_TIMEOUT = 20  # one deadline for the whole ask, shared by the askers; then the rules pick
NEEDS = (  # (key, words that ask for it, what a machine needs)
    ('ios', r'ios|ipados|xcode|swift|swiftui|iphone|ipad|testflight|watchos|visionos|cocoapods|simulator'),
    ('android', r'android|apk|aab|gradle|kotlin|adb|jetpack|play store'),
    ('cuda', r'cuda|gpus?|train|training|fine-?tun\w*|pytorch|nvidia|tensorrt|vram'),
    ('windows', r'windows|msvc|visual studio|powershell|win32|winui|\.exe|wsl|directx'),
)
NEED_RE = [(k, re.compile(r'(?<![\w.])(?:' + w + r')(?![\w])', re.I)) for k, w in NEEDS]
NEED_NAME = {'ios': 'Xcode', 'android': 'the Android SDK', 'cuda': 'CUDA', 'windows': 'Windows'}
ASK_SYSTEM = ('You pick which of the user\'s computers should run a coding task in Claude Code. Answer with one JSON '
              'object and nothing else: {"machine": "<name>", "folder": "<one of that machine\'s repos, or empty>", '
              '"why": "<one short sentence, at most 12 words, naming the fact that decided it>"}. '
              'Pick a machine that has what the task needs: Xcode for iOS or macOS apps (a connected iPhone helps '
              'for on-device work), the Android SDK for Android (a connected device helps), CUDA for GPU work, '
              'Windows for Windows-only builds. If the task names a repo or a folder, prefer the machine that has '
              'it and give its path as the folder. "also" lists what the user says a machine can do besides; count it like a '
              'tool. If nothing decides it, pick the one with the most free memory. '
              'Use only names from the list. Never use a dash as punctuation in "why".')


def gpu_short(gpu):
    g = re.sub(r'\b(?:NVIDIA|GeForce|Apple|AMD|Radeon\(TM\)|Intel\(R\))\s+', '', gpu or '').strip()
    return g or gpu or ''


def has_need(caps, key):
    t = (caps or {}).get('tools') or {}
    return bool({'ios': t.get('xcode'), 'android': t.get('android_sdk'), 'cuda': t.get('cuda'),
                 'windows': (caps or {}).get('os') == 'windows'}[key])


def need_phrase(caps, key):
    t = caps.get('tools') or {}
    if key == 'ios':
        return f'Xcode {str(t["xcode"]).split(".")[0]}' + (' and your iPhone connected' if t.get('iphone') == 'connected' else '')
    if key == 'android':
        return 'the Android SDK' + (' and a phone connected' if t.get('adb_devices') else '')
    if key == 'cuda':
        g = gpu_short(caps.get('gpu'))
        return f'CUDA {t["cuda"]}' + (f' on the {g}' if g else '')
    return 'Windows' + (f' with Visual Studio {t["msvc"]}' if t.get('msvc') else '')


def can_line(caps):
    """One line a person reads in the machine's header: 'iOS, Android SDK, CUDA (RTX 5070)'."""
    if not isinstance(caps, dict):
        return None
    t, out = caps.get('tools') or {}, []
    if t.get('xcode'):
        out.append('iOS' + (' + iPhone' if t.get('iphone') == 'connected' else ''))
    if t.get('android_sdk'):
        out.append('Android SDK' + (' + phone' if t.get('adb_devices') else ''))
    if t.get('cuda'):
        g = gpu_short(caps.get('gpu'))
        out.append(f'CUDA ({g})' if g else 'CUDA')
    if t.get('docker'):
        out.append('Docker')
    if t.get('msvc'):
        out.append('Visual Studio')
    out += [x for x in caps.get('also') or []][:3]
    return ', '.join(out) or 'General work'


def and_join(xs):
    xs = [x for x in xs if x]
    return xs[0] if len(xs) == 1 else ', '.join(xs[:-1]) + ' and ' + xs[-1] if xs else ''


def repo_match(repos, folder):
    """The repo path a typed folder means on a machine ('switchboard' or 'Claude/switchboard'), or None."""
    f = (folder or '').replace('\\', '/').strip('/').lower()
    if not f:
        return None
    for rel in repos or []:
        if rel.lower() == f:
            return rel
    for rel in repos or []:
        if rel.rsplit('/', 1)[-1].lower() == f.rsplit('/', 1)[-1]:
            return rel
    return None


def repos_named(repos, prompt):
    """Repos the task mentions by name (at least 3 letters, as a word)."""
    low, out = (prompt or '').lower(), []
    for rel in repos or []:
        name = rel.rsplit('/', 1)[-1].lower()
        if len(name) >= 3 and re.search(r'(?<![\w-])' + re.escape(name) + r'(?![\w-])', low):
            out.append(rel)
    return out


def clean_why(s):
    s = ' '.join(str(s or '').split())
    s = re.sub(r'\s*[\u2014\u2013]\s*', ', ', s).strip(' ,')
    if not s:
        return ''
    s = s[:140].rstrip(' ,;:')
    s = s[0].upper() + s[1:]
    return s if s[-1] in '.!?' else s + '.'


def rules_pick(cands, prompt, folder=''):
    """cands: [(name, label, caps)]. (name, folder, why) by keywords, then a named repo, then free memory."""
    needs = [k for k, rx in NEED_RE if rx.search(prompt or '')]
    free = lambda c: (c.get('ram_free_gb') if isinstance(c.get('ram_free_gb'), (int, float)) else c.get('ram_gb') or 0)

    def score(item):
        name, _, caps = item
        met = [k for k in needs if has_need(caps, k)]
        t = caps.get('tools') or {}
        bonus = (('ios' in met and t.get('iphone') == 'connected') or ('android' in met and bool(t.get('adb_devices'))))
        repo = repo_match(caps.get('repos'), folder) if folder else (repos_named(caps.get('repos'), prompt) or [None])[0]
        return (len(met) + len(extras_hit(caps.get('also'), prompt)), bonus, bool(repo), free(caps)), met, repo

    best = max(cands, key=lambda it: score(it)[0])
    (_, _, _, mem), met, repo = score(best)
    name, _, caps = best
    also = extras_hit(caps.get('also'), prompt)
    if also and not met:
        why = clean_why(f'You said it {also[0][0].lower() + also[0][1:]}') if re.match(r'(?i)(can|has|runs|is)\b', also[0]) else clean_why(f'You noted: {also[0]}')
        return name, (repo or folder or ''), why
    if met:
        why = 'Has ' + and_join([need_phrase(caps, k) for k in met] + ([f'{repo.rsplit("/", 1)[-1]} checked out'] if repo else [])) + '.'
    elif repo:
        why = f'Has {repo.rsplit("/", 1)[-1]} checked out.'
    elif needs:
        why = f'No online machine has {and_join([NEED_NAME[k] for k in needs])}; this one has the most free memory.'
    else:
        why = f'Most free memory right now ({mem:g} GB).' if mem else 'The only machine online.' if len(cands) == 1 else 'Most free memory right now.'
    return name, (repo or folder or ''), why


def parse_pick(text):
    m = re.search(r'\{.*\}', text or '', re.S)
    if not m:
        raise ValueError('no JSON in the answer')
    d = json.loads(m.group(0))
    if not isinstance(d, dict) or not isinstance(d.get('machine'), str):
        raise ValueError('the answer has no machine')
    return d


# What Settings shows for a machine and lets the user switch off: (tool key, name). A switched-off tool is left out of
# what Auto sees, and what Auto sees also carries the user's own lines ("this machine can also do...").
CAP_TOOLS = (('xcode', 'Xcode'), ('ios_sim', 'iOS Simulator'), ('android_sdk', 'Android SDK'), ('emulator', 'Android emulator'),
             ('cuda', 'CUDA'), ('docker', 'Docker'), ('msvc', 'Visual Studio'), ('dotnet', '.NET'), ('java', 'Java'),
             ('rust', 'Rust'), ('go', 'Go'), ('node', 'Node'), ('python', 'Python'), ('tailscale', 'Tailscale'))
CAP_KEYS = {k for k, _ in CAP_TOOLS}
CAP_TIED = {'xcode': {'iphone': 'none', 'ios_sim': False}, 'android_sdk': {'adb_devices': None, 'emulator': False}}
EXTRA_MAX, EXTRA_CHARS = 8, 120


def cap_setting(v):
    """One machine's entry in caps.json: true (on, as detected) or {"on", "off": [tool keys], "extra": [lines]}."""
    if v is True:
        return {'on': True, 'off': [], 'extra': []}
    if not isinstance(v, dict):
        return {'on': False, 'off': [], 'extra': []}
    off = [k for k in v.get('off') or [] if k in CAP_KEYS] if isinstance(v.get('off'), list) else []
    extra = [str(x).strip()[:EXTRA_CHARS] for x in v.get('extra') or [] if str(x).strip()] if isinstance(v.get('extra'), list) else []
    return {'on': v.get('on') is True, 'off': sorted(set(off)), 'extra': extra[:EXTRA_MAX]}


def apply_setting(caps, st):
    """What Auto sees of a machine: its detected caps without the tools switched off, plus the user's own lines."""
    if not isinstance(caps, dict):
        return caps
    out = dict(caps, tools=dict(caps.get('tools') or {}))
    for k in st.get('off') or []:
        if k in out['tools']:
            out['tools'][k] = None if not isinstance(out['tools'][k], bool) else False
        out['tools'].update(CAP_TIED.get(k, {}))
    if 'cuda' in (st.get('off') or []):
        out['gpu'] = None
    if st.get('extra'):
        out['also'] = list(st['extra'])
    return out


def cap_rows(caps, st):
    """Settings' list for one machine: every tool it was found to have, with its version, and whether Auto uses it."""
    t = (caps or {}).get('tools') or {}
    rows = []
    for k, name in CAP_TOOLS:
        v = t.get(k)
        if v in (None, False, 'none', 0, ''):
            continue
        detail = None if isinstance(v, bool) else str(v)
        if k == 'cuda' and caps.get('gpu'):
            detail = f'{v} on the {gpu_short(caps["gpu"])}'
        rows.append({'key': k, 'name': name, 'detail': detail, 'on': k not in (st.get('off') or [])})
    return rows


def extras_hit(extras, prompt):
    """The user's lines for a machine that the task matches: a word of 4 letters or more from the line, as a word."""
    low, hits = (prompt or '').lower(), []
    for line in extras or []:
        words = [w for w in re.findall(r'[a-z0-9][a-z0-9+#.-]{3,}', line.lower()) if w not in STOP_WORDS]
        if any(re.search(r'(?<![\w-])' + re.escape(w) + r'(?![\w-])', low) for w in words):
            hits.append(line)
    return hits


STOP_WORDS = {'this', 'that', 'with', 'from', 'have', 'machine', 'computer', 'also', 'can', 'does', 'into', 'runs', 'work'}


class Auto:
    def __init__(self, app, cfg_file, http_error):
        self.app, self.cfg_file, self.HttpError = app, Path(cfg_file), http_error
        self.file = app.state_dir / 'caps.json'
        self.lock = threading.Lock()
        self.data = jload(self.file, {})  # name -> {'at', 'caps', 'error'}
        self.busy, self._cfg = set(), (None, {})

    def settings(self):
        """{name: {'on', 'off', 'extra'}} from caps.json. Read again when the file changes."""
        try:
            mt = self.cfg_file.stat().st_mtime
        except OSError:
            return {}
        if self._cfg[0] != mt:
            d = jload(self.cfg_file, {})
            self._cfg = (mt, {k: cap_setting(v) for k, v in (d.items() if isinstance(d, dict) else ())})
        return self._cfg[1]

    def enabled(self):
        """{name: True} for the machines the user lets share what they can do."""
        return {k: True for k, v in self.settings().items() if v['on']}

    def set_setting(self, name, body):
        """Settings: Auto on or off for a machine, tools it should not count on, and lines of its own."""
        cur = self.settings().get(name) or cap_setting(None)
        st = cap_setting({'on': body.get('on', cur['on']) is True, 'off': body.get('off', cur['off']), 'extra': body.get('extra', cur['extra'])})
        if 'off' in body and not isinstance(body['off'], list) or 'extra' in body and not isinstance(body['extra'], list):
            raise self.HttpError(400, 'off and extra are lists')
        d = jload(self.cfg_file, {})
        d = d if isinstance(d, dict) else {}
        d[name] = True if st['on'] and not st['off'] and not st['extra'] else st
        jsave(self.cfg_file, d)
        self._cfg = (None, {})
        return st

    def detected(self, name):
        with self.lock:
            c = (self.data.get(name) or {}).get('caps')
        return c if isinstance(c, dict) else None

    def caps(self, name):
        st = self.settings().get(name)
        if not st or not st['on']:
            return None
        return apply_setting(self.detected(name), st)

    def view(self, name, fetch=False):
        """Settings' Capabilities for one machine: Auto on or off, what was found (each with a switch), the user's lines."""
        if fetch:
            self.fetch(name, timeout=20)
        st = self.settings().get(name) or cap_setting(None)
        c = self.detected(name)
        with self.lock:
            rec = self.data.get(name) or {}
        return {'ok': True, 'on': st['on'], 'extra': st['extra'], 'off': st['off'], 'tools': cap_rows(c, st) if c else None,
                'measured': rec.get('at'), 'error': rec.get('error') if not c else None, 'can': can_line(apply_setting(c, st)) if c else None}

    def all(self):
        return {n: self.caps(n) for n in self.app.fleet.machines}

    def fetch(self, name, timeout=20):
        try:
            c = self.app._call(name, {'verb': 'caps'}, timeout=timeout)['caps']
            rec = {'at': self.app.clock(), 'caps': c, 'error': None}
        except Exception as e:
            with self.lock:
                rec = dict(self.data.get(name) or {}, at=self.app.clock() - CAPS_EVERY + CAPS_RETRY, error=str(e)[:200])
        with self.lock:
            self.data[name] = rec
            self.busy.discard(name)
            try:
                jsave(self.file, self.data)
            except OSError:
                log('could not save caps')
        return rec.get('caps')

    def refresh(self, machines, force=False, wait=None):
        """Ask the machines whose caps are old (or all of them, force) again; wait up to `wait` seconds for them."""
        on, now, ts = self.enabled(), self.app.clock(), []
        for m in machines:
            n = m['name']
            if not on.get(n) or not m.get('online'):
                continue
            with self.lock:
                age = now - ((self.data.get(n) or {}).get('at') or 0)
                if n in self.busy or (not force and age < CAPS_EVERY):
                    continue
                self.busy.add(n)
            t = threading.Thread(target=self.fetch, args=(n,), daemon=True)
            t.start()
            ts.append(t)
        end = time.time() + (wait or 0)
        for t in ts if wait else ():
            t.join(max(0, end - time.time()))

    def decorate(self, snap):
        for m in snap['machines']:
            c = self.caps(m['name'])
            m['caps'], m['can'] = c, can_line(c)
            m['chrome'] = self.chrome(m['name'])
            m['chrome_pref'] = False  # no machine is everyone's Chrome machine: each chat picks its own (chrome_machine)

    def chrome(self, name):
        """Claude in Chrome on a machine: {'installed', 'connector', 'connected'}, or None before its agent says.
        Only whether Chrome and the extension's connector are there: never its profiles, cookies or logins."""
        with self.lock:
            c = ((self.data.get(name) or {}).get('caps') or {}).get('chrome')
        return {k: bool(c.get(k)) for k in ('installed', 'connector', 'connected')} if isinstance(c, dict) else None

    def chrome_pick(self, names=None, want=None):
        """(machine, why) for a session that uses Chrome: the Chrome machine picked for this chat when it is online,
        else the only online machine with the Chrome connector. None when that does not settle it."""
        snap = self.app.fleet.snapshot()
        online = [m['name'] for m in snap['machines'] if m.get('online') and (names is None or m['name'] in names)]
        if want in online:
            return want, f'{self.app._label(want)} is this chat\'s Chrome machine.'
        have = [n for n in online if (self.chrome(n) or {}).get('connector')]
        if len(have) == 1:
            return have[0], f'{self.app._label(have[0])} is the only machine online with Claude in Chrome.'
        return None

    def candidates(self):
        snap = self.app.fleet.snapshot()
        out = [(m['name'], m.get('label') or m['name'], self.caps(m['name'])) for m in snap['machines'] if m.get('online')]
        return [x for x in out if x[2]], snap

    def none_reason(self):
        return ('Auto is off: no machine shares what it can do yet.' if not self.enabled()
                else 'No machine Auto can use is online right now.')

    def route(self, prompt, folder='', rules_only=False, chrome=False, chrome_machine=None):
        """{'machine', 'label', 'folder', 'create', 'why', 'by'}: Claude's pick, or the rules' when Claude cannot. A
        session with Chrome goes to its own Chrome machine (chrome_machine), or the only one that has it."""
        prompt, folder = str(prompt or '')[:8000], str(folder or '').strip().strip('/\\')
        if chrome:
            got = self.chrome_pick(want=chrome_machine)
            if got:
                name, why = got
                caps = self.caps(name) or {}
                given = folder and repo_match(caps.get('repos'), folder)
                return {'ok': True, 'machine': name, 'label': self.app._label(name), 'why': why, 'by': 'chrome',
                        'folder': given or folder, 'create': bool(folder and caps.get('repos') is not None and not given)}
        cands, snap = self.candidates()
        fresh = False
        if not cands and not rules_only and any(m.get('online') and self.enabled().get(m['name']) for m in snap['machines']):
            # a first start: machines are online but none has said what it can do yet, so ask before saying none can
            self.refresh([m for m in snap['machines'] if self.enabled().get(m['name'])], force=True, wait=CAPS_FRESH_S)
            cands, snap, fresh = *self.candidates(), True
        if chrome:  # several machines with Chrome and the chosen one is away: Auto picks among them
            with_chrome = [x for x in cands if (self.chrome(x[0]) or {}).get('connector')]
            cands = with_chrome or cands
        if not cands:
            raise self.HttpError(400, self.none_reason())
        if not rules_only and not fresh:
            self.refresh([m for m in snap['machines'] if self.enabled().get(m['name'])], force=True, wait=CAPS_FRESH_S)
            keep = {n for n, _, _ in cands}
            cands, snap = self.candidates()
            cands = [x for x in cands if x[0] in keep] or cands
            if not cands:
                raise self.HttpError(400, self.none_reason())
        name, rfolder, why = rules_pick(cands, prompt, folder)
        pick = {'machine': name, 'folder': rfolder, 'why': why, 'by': 'rules'}
        if not rules_only:
            t0 = time.time()
            try:
                pick = self.ask_claude(cands, snap, prompt, folder)
                pick['ms'] = round((time.time() - t0) * 1000)
            except Exception as e:
                log(f'auto: Claude did not pick ({e}); the rules did')
                pick['fallback'] = str(e)[:200]
        caps = dict((n, c) for n, _, c in cands)[pick['machine']]
        given = folder and repo_match(caps.get('repos'), folder)
        if folder:
            pick['folder'], pick['create'] = (given, False) if given else (folder, True)
        else:
            pick['folder'] = repo_match(caps.get('repos'), pick.get('folder')) or ''
            pick['create'] = False
        return dict(pick, ok=True, label=self.app._label(pick['machine']))

    def askers(self, cands, snap):
        """Who can run Claude for the pick: a PC runs it over SSH; a Mac only through its launcher."""
        by = {m['name']: m for m in snap['machines']}
        win = [n for n, _, c in cands if c.get('os') != 'mac']
        mac = [n for n, _, c in cands if c.get('os') == 'mac' and ((by.get(n) or {}).get('launcher') or {}).get('alive')]
        return win + mac

    def ask_claude(self, cands, snap, prompt, folder):
        askers = self.askers(cands, snap)
        if not askers:
            raise RuntimeError('no machine can ask Claude right now')
        by = {m['name']: m for m in snap['machines']}
        rows = []
        for n, label, c in cands:
            t = {k: v for k, v in (c.get('tools') or {}).items() if v not in (None, False, 'none', 0)}
            busy = sum(1 for s in (by.get(n) or {}).get('sessions') or [] if s.get('status') == 'busy' and not s.get('stalled'))
            rows.append({'name': n, 'label': label, 'os': c.get('os'), 'cpu': c.get('cpu'), 'ram_gb': c.get('ram_gb'),
                         'ram_free_gb': c.get('ram_free_gb'), 'gpu': c.get('gpu'), 'tools': t, 'running_chats': busy,
                         'repos': (c.get('repos') or [])[:40], 'also': c.get('also') or []})
        text = (f'Machines:\n{json.dumps(rows, separators=(",", ":"))}\n\n'
                f'Folder the user chose: {folder or "none"}\n\nTask:\n{prompt[:4000] or "(no message yet: an idle session)"}')
        last, end = None, time.time() + ASK_TIMEOUT
        for asker in askers[:2]:
            if end - time.time() < 1:
                last = last or TimeoutError('Claude took too long')
                break
            try:
                r = self.app._call(asker, {'verb': 'ask', 'prompt': text, 'system': ASK_SYSTEM}, timeout=end - time.time())
                d = parse_pick(r.get('text'))
                names = {n for n, _, _ in cands}
                if d['machine'] not in names:  # a label instead of the name, or something made up
                    d['machine'] = next((n for n, l, _ in cands if l.lower() == d['machine'].lower()), None)
                    if not d['machine']:
                        raise ValueError('it picked a machine that is not in the list')
                return {'machine': d['machine'], 'folder': str(d.get('folder') or ''), 'why': clean_why(d.get('why')) or 'Claude picked it.',
                        'by': 'claude', 'asker': asker}
            except Exception as e:
                last = e
        raise RuntimeError(str(last))

    def resolve_job(self, jid, j):
        """A server job sent to "auto": pick the machine, then it runs like any other job."""
        app = self.app
        app._set(jid, step='choosing')
        plan = j['plan']
        req, auto = dict(plan['req']), plan.get('auto') or {}
        pick = self.route(req.get('prompt') or auto.get('task') or '', auto.get('folder') or '', chrome=req.get('chrome') is True,
                          chrome_machine=plan.get('chrome_machine'))
        if req.get('flags') and 'flags' not in (app.fleet.state.get(pick['machine'], {}).get('features') or []):
            raise self.HttpError(400, f'{pick["label"]} needs the Switchboard update before it can take these flags.')
        req['folder'] = pick['folder']
        plan = dict(plan, req=req, mkdir=pick['folder'] if pick['create'] else None)
        picked = f'Auto picked {pick["label"]}. {pick["why"]}'
        app._set(jid, machine=pick['machine'], plan=plan, picked=picked, note=picked, by=pick['by'], step='sent')
        return dict(j, machine=pick['machine'], plan=plan)


# ---------------------------------------------------------------- v2: dispatch (one session asks another machine for work)
DISPATCH_KEEP, DISPATCH_MAX = 7 * 86400, 300
DISPATCH_NOTE = ('You were sent this by a Claude Code session on {a}. Do the work here, then end with a short report: '
                 'what you did, what changed (paths), what is left.')
TASK_MAX = 18_000
SHOW_LIST_S = 120  # a launched chat shows up in its machine's list within a poll or two


def quick_topic(task, room):
    first = next((l for l in str(task or '').splitlines() if l.strip()), '')
    words = re.sub(r"[^A-Za-z0-9 ._,:'+#-]", ' ', first).split()
    out = ''
    for w in words[:8]:
        if len((out + ' ' + w).strip()) > room:
            break
        out = (out + ' ' + w).strip()
    out = out.rstrip(' .,:-') or 'Task'
    return out[0].upper() + out[1:]


class Dispatch:
    def __init__(self, app, auto, http_error):
        self.app, self.auto, self.HttpError = app, auto, http_error
        self.file = app.state_dir / 'dispatch.json'
        self.lock = threading.Lock()
        self.d = jload(self.file, {})
        self.polled = {}

    def _save(self):  # caller holds lock
        now = self.app.clock()
        for k in [k for k, r in self.d.items() if now - r.get('created', 0) > DISPATCH_KEEP]:
            del self.d[k]
        for k in sorted(self.d, key=lambda k: self.d[k].get('created', 0))[:-DISPATCH_MAX]:
            del self.d[k]
        try:
            jsave(self.file, self.d)
        except OSError:
            log('could not save dispatch.json')

    def start(self, body):
        app = self.app
        src = body.get('from') if isinstance(body.get('from'), dict) else {}
        fm, fsid = src.get('machine'), src.get('sid')
        if not isinstance(fm, str) or fm not in app.fleet.machines:
            raise self.HttpError(400, 'from.machine must be one of the machines')
        if fsid is not None and not (isinstance(fsid, str) and SID_RE.fullmatch(fsid)):
            raise self.HttpError(400, 'bad from.sid')
        task, folder, mode = body.get('task'), body.get('folder') or '', body.get('mode') or 'normal'
        if not isinstance(task, str) or not task.strip():
            raise self.HttpError(400, 'The task is empty.')
        if len(task) > TASK_MAX:
            raise self.HttpError(400, f'The task is too long (at most {TASK_MAX} characters).')
        if not isinstance(folder, str) or len(folder) > 300:
            raise self.HttpError(400, 'bad folder')
        if mode not in ('normal', 'max', 'ultracode'):
            raise self.HttpError(400, 'unknown mode')
        target, pick = body.get('machine'), None
        if target == 'auto':
            pick = self.auto.route(task, folder)
            target, folder = pick['machine'], pick['folder']
        elif not isinstance(target, str) or target not in app.fleet.machines:
            raise self.HttpError(400, 'unknown machine')
        a = re.sub(r'[^A-Za-z0-9 ]', '', app._label(fm))[:20].strip() or 'another machine'
        tail = f' (for {a})'
        name = quick_topic(task, 64 - len(tail)) + tail
        req = {'verb': 'start', 'folder': folder, 'prompt': f'{task.strip()}\n\n{DISPATCH_NOTE.format(a=app._label(fm))}',
               'mode': mode, 'name': name}
        if mode == 'ultracode':
            req['model'] = 'opus'
        mkdir = folder if pick and pick.get('create') else None
        r = app._submit('start', target, {'req': req, 'mkdir': mkdir}, name)
        did = 'd-' + os.urandom(6).hex()
        rec = {'job': r['job'], 'from': {'machine': fm, 'sid': fsid}, 'machine': target, 'task': task[:300], 'folder': folder,
               'name': name, 'created': app.clock(), 'state': 'starting'}
        if pick:
            rec.update(why=pick['why'], by=pick['by'])
            picked = f'Auto picked {pick["label"]}. {pick["why"]}'
            app._set(r['job'], picked=picked, note=picked, by=pick['by'])
        with self.lock:
            self.d[did] = rec
            self._save()
        out = {'ok': True, 'id': did, 'machine': target, 'label': app._label(target), 'job': r['job']}
        if pick:
            out['why'] = pick['why']
        return out

    def _poll_soon(self, name):
        now = time.time()
        if now - self.polled.get(name, 0) < 10:
            return
        self.polled[name] = now
        threading.Thread(target=self.app.fleet.poll_one, args=(name,), daemon=True).start()

    def status(self, did):
        app = self.app
        with self.lock:
            rec = dict(self.d.get(did) or {})
        if not rec:
            raise self.HttpError(404, 'No task with that id. Ids are kept for a week.')
        now = app.clock()
        if rec['state'] not in ('done', 'failed'):
            with app.jobs_lock:
                j = app.jobs.get(rec['job'])
                if j:
                    j['watched'] = now  # the session that sent it is following along; no push for the launch
                    j = dict(j)
            if j:
                rec['machine'] = j.get('machine') or rec['machine']
                if j.get('state') == 'error':
                    rec.update(state='failed', error=j.get('error') or 'It did not start.', done_at=now)
                elif j.get('state') == 'done' and j.get('sid'):
                    rec.update(sid=j['sid'], url=j.get('url'), launched=rec.get('launched') or j.get('finished') or now)
            if rec.get('sid') and rec['state'] not in ('done', 'failed'):
                self._follow(rec, now)
            elif not j and rec['state'] == 'starting':
                rec.update(state='failed', error='Switchboard lost track of the launch. Check the list.', done_at=now)
            with self.lock:
                self.d[did] = rec
                self._save()
        label = app._label(rec['machine'])
        out = {'ok': True, 'id': did, 'state': rec['state'], 'machine': rec['machine'], 'label': label, 'sid': rec.get('sid'),
               'url': rec.get('url'), 'report': rec.get('report') if rec['state'] == 'done' else None,
               'elapsed': int((rec.get('done_at') or now) - rec['created']), 'name': rec.get('name')}
        for k in ('error', 'note', 'why'):
            if rec.get(k):
                out[k] = rec[k]
        return out

    def _follow(self, rec, now):
        """A launched chat: working until it is idle after a reply (or stopped); then its last message is the report."""
        row = None
        for m in self.app.fleet.snapshot()['machines']:
            if m['name'] == rec['machine']:
                row = next((s for s in m['sessions'] if s.get('sid') == rec['sid']), None)
        rec.pop('note', None)
        rec['state'] = 'working'
        if row is None:
            if now - rec['launched'] > SHOW_LIST_S:
                rec.update(state='failed', error=f'The chat is not in {self.app._label(rec["machine"])}\'s list any more.', done_at=now)
            else:
                self._poll_soon(rec['machine'])
            return
        status = row.get('status')
        if status == 'waiting':
            rec['note'] = f'Waiting for an answer or a permission on {self.app._label(rec["machine"])}.'
        replied = (row.get('ctx') or 0) > 0
        if (status == 'idle' and replied) or (status not in LIVE and (replied or now - rec['launched'] > 60)):
            rec.update(state='done', done_at=now)
            try:
                r = self.app._call(rec['machine'], {'verb': 'report', 'sid': rec['sid']}, timeout=20)
                rec['report'] = r.get('text') or ''
                if not rec['report']:
                    rec['note'] = 'It stopped without a written report.'
            except Exception as e:
                rec.update(report='', note=f'Could not read its report: {str(e)[:160]}')
            return
        self._poll_soon(rec['machine'])


# ---------------------------------------------------------------- v2: the session browser (every chat, every machine)
BROWSE_EVERY = 300      # a machine's index is asked for what changed at most this often (and when someone browses)
BROWSE_PAGE_MAX = 100
STATES = ('live', 'quiet', 'stopped', 'archived', 'replaced')


def browse_state(row, live):
    s = live or {}
    if s.get('status') in LIVE:
        return 'live'
    if row.get('archived'):
        return 'archived'
    if row.get('replaced') or s.get('by'):
        return 'replaced'
    if s.get('kind') == 'background' or s.get('status') == 'stopped':
        return 'stopped'
    return 'quiet'


# What each earlier chat of a thread carries, for History's Additional history.
OLDER_KEYS = ('machine', 'label', 'sid', 'title', 'last', 'state', 'folder', 'started', 'ctx', 'size', 'model', 'url',
              'first_prompt', 'last_prompt', 'handoff')


class Browse:
    """Every chat on every machine, indexed here from the machines' browse verb (only what changed since the last
    ask comes over), searched and paged for the session browser."""

    def __init__(self, app, threads):
        self.app, self.threads = app, threads
        self.path = app.state_dir / 'browse_index.json'
        self.lock, self.busy = threading.Lock(), set()
        self.idx = jload(self.path, {})  # machine -> {'rows': {sid: row}, 'since', 'at', 'error', 'partial'}

    def refresh(self, name, force=False):
        with self.lock:
            cur = self.idx.get(name) or {}
            if name in self.busy or (not force and self.app.clock() - (cur.get('at') or 0) < BROWSE_EVERY and not cur.get('partial')):
                return False
            self.busy.add(name)
        try:
            r = self.app._call(name, {'verb': 'browse', 'since': cur.get('since') or 0}, timeout=60)
            with self.lock:
                rows = dict((self.idx.get(name) or {}).get('rows') or {})
                alive = r.get('all') or {}
                rows = {k: v for k, v in rows.items() if k in alive}
                for row in r.get('rows') or []:
                    if isinstance(row, dict) and isinstance(row.get('sid'), str):
                        rows[row['sid']] = row
                since = max([v.get('mtime') or 0 for v in rows.values()] + [0]) if not r.get('partial') else cur.get('since') or 0
                self.idx[name] = {'rows': rows, 'since': since, 'at': self.app.clock(), 'error': None, 'partial': bool(r.get('partial'))}
                jsave(self.path, self.idx)
            polled = {s.get('sid') for m in self.app.fleet.snapshot()['machines'] if m['name'] == name
                      for s in (m.get('sessions') or []) + (m.get('replaced') or [])}
            self.threads.observe([{'name': name, 'sessions': [dict(v, active=v.get('last'), how=v.get('replaced'))
                                                              for k, v in rows.items() if k not in polled]}])
            return True
        except Exception as e:
            with self.lock:
                self.idx[name] = dict(cur, at=self.app.clock(), error=str(e)[:200])
            return False
        finally:
            with self.lock:
                self.busy.discard(name)

    def refresh_all(self, force=False, wait=None):
        ts = []
        for m in self.app.fleet.snapshot()['machines']:
            if m.get('online') and 'browse' in (m.get('features') or []):
                t = threading.Thread(target=self.refresh, args=(m['name'], force), daemon=True)
                t.start()
                ts.append(t)
        end = time.time() + (wait or 0)
        for t in ts if wait else ():
            t.join(max(0, end - time.time()))

    def rows(self):
        """Every chat as one row, live state from the last poll laid over the index."""
        snap = self.app.fleet.snapshot()
        live = {}
        for m in snap['machines']:
            for s in m.get('sessions') or []:
                live[(m['name'], s.get('sid'))] = s
            for s in m.get('replaced') or []:
                live.setdefault((m['name'], s.get('sid')), dict(s, by=s.get('by') or {'replaced': True}))
        names = self.app.names()
        out = {}
        with self.lock:
            idx = {n: dict(v.get('rows') or {}) for n, v in self.idx.items()}
        for name, rows in idx.items():
            if name not in self.app.fleet.machines:
                continue
            for sid, r in rows.items():
                out[(name, sid)] = dict(r, machine=name)
        for (name, sid), s in live.items():  # machines without the browse verb yet, and what changed since the index
            r = out.get((name, sid)) or {'sid': sid, 'machine': name}
            r = dict(r, title=s.get('title') or r.get('title'), cwd=s.get('cwd') or r.get('cwd'), folder=s.get('folder') or r.get('folder'),
                     model=s.get('model') or r.get('model'), ctx=s.get('ctx') or r.get('ctx') or 0,
                     last=max(s.get('last_ts') or s.get('active') or 0, r.get('last') or 0) or None,
                     started=r.get('started') or s.get('started'), url=s.get('url') or r.get('url'),
                     archived=s.get('archived') or r.get('archived'))
            out[(name, sid)] = r
        res = []
        for (name, sid), r in out.items():
            s = live.get((name, sid))
            k = ck(name, sid)
            res.append(dict(r, label=self.app._label(name), title=names.get(k) or r.get('title') or 'Untitled',
                            state=browse_state(r, s), status=(s or {}).get('status'), kind=(s or {}).get('kind'),
                            bg_id=(s or {}).get('bg_id'), key=k))
        return res

    def query(self, q):
        """GET /api/browse: q, machine, folder, from, to, state, thread=1 (group), offset, limit."""
        one = lambda k, d='': (q.get(k) or [d])[0]
        rows = self.rows()
        text = [w for w in re.findall(r'[a-z0-9]+', one('q').lower())]
        machine, folder, state = one('machine'), one('folder').strip('/').lower(), one('state')
        try:
            t_from, t_to = float(one('from', '0') or 0), float(one('to', '0') or 0)
            offset, limit = max(0, int(one('offset', '0'))), max(1, min(BROWSE_PAGE_MAX, int(one('limit', '50'))))
        except ValueError:
            raise ValueError('from, to, offset and limit are numbers')
        if state and state not in STATES:
            raise ValueError('unknown state')

        def keep(r):
            if machine and r['machine'] != machine:
                return False
            if folder and not (r.get('folder') or '').lower().startswith(folder):
                return False
            if state and r['state'] != state:
                return False
            last = r.get('last') or r.get('mtime') or 0
            if (t_from and last < t_from) or (t_to and last > t_to):
                return False
            if text:
                hay = ' '.join(str(r.get(f) or '') for f in ('title', 'folder', 'first_prompt', 'last_prompt', 'label', 'model')).lower()
                return all(w in hay for w in text)
            return True
        found = [r for r in rows if keep(r)]
        found.sort(key=lambda r: -(r.get('last') or r.get('mtime') or 0))
        if one('thread') == '1':  # one row per thread: the newest chat, the older ones as its history
            _, parents, roots, _ = self.threads.graph()
            groups, order = {}, []
            for r in sorted(rows, key=lambda r: -(r.get('last') or r.get('mtime') or 0)):
                g = roots.get(r['key'], r['key'])
                groups.setdefault(g, []).append(r)
            seen = set()
            for r in found:
                g = roots.get(r['key'], r['key'])
                if g in seen:
                    continue
                seen.add(g)
                head, *older = groups[g]
                order.append(dict(head, history=[{k: o.get(k) for k in OLDER_KEYS if o.get(k) is not None}
                                                 for o in older[:20]], history_count=len(older)))
            found = order
        facets = {'machines': sorted({(r['machine'], r['label']) for r in rows}),
                  'folders': sorted({r.get('folder') for r in rows if r.get('folder')})[:300],
                  'states': {s: sum(1 for r in rows if r['state'] == s) for s in STATES}}
        with self.lock:
            status = {n: {'at': v.get('at'), 'error': v.get('error'), 'partial': v.get('partial'), 'count': len(v.get('rows') or {})}
                      for n, v in self.idx.items()}
        page = found[offset:offset + limit]
        slim = ('machine', 'label', 'sid', 'title', 'folder', 'cwd', 'model', 'ctx', 'size', 'last', 'started', 'state', 'status',
                'kind', 'bg_id', 'url', 'archived', 'first_prompt', 'last_prompt', 'history', 'history_count', 'replaced', 'handoff')
        return {'ok': True, 'total': len(found), 'offset': offset, 'rows': [{k: r.get(k) for k in slim if k in r} for r in page],
                'facets': {'machines': [{'name': n, 'label': l} for n, l in facets['machines']], 'folders': facets['folders'],
                           'states': facets['states']}, 'index': status}


class Features:
    ACTIONS = ('keep', 'link', 'unlink', 'rename', 'star')

    def __init__(self, app, cfg_dir, http_error):
        self.app, self.cfg_dir, self.HttpError = app, Path(cfg_dir), http_error
        self.reaper = Reaper(app, self.cfg_dir / 'reaper.json')
        self.threads = Threads(app)
        self.auto = Auto(app, self.cfg_dir / 'caps.json', http_error)
        self.dispatch = Dispatch(app, self.auto, http_error)
        self.browse = Browse(app, self.threads)
        self.library = library_mod.Library(app, http_error)
        self.stars = Stars(self.cfg_dir / 'stars.json', self.threads)
        self.groups = Groups(self.cfg_dir / 'groups.json', http_error)
        self.budget = budget_mod.Budget(app, self.cfg_dir, http_error)
        if hasattr(app.fleet, 'budget_view'):
            app.fleet.budget_view = self.budget.agent_view  # every poll carries it to the agents' hooks

    def after_poll(self):
        try:
            self.threads.observe(self.app.fleet.snapshot()['machines'])
        except Exception:
            log('threads failed: ' + traceback.format_exc())
        self.reaper.run()
        try:
            self.auto.refresh(self.app.fleet.snapshot()['machines'])
        except Exception:
            log('caps failed: ' + traceback.format_exc())
        try:
            self.browse.refresh_all()
        except Exception:
            log('browse failed: ' + traceback.format_exc())
        try:
            self.library.after_poll()
        except Exception:
            log('library failed: ' + traceback.format_exc())
        self.budget.after_poll()

    def decorate(self, snap):
        for m in snap['machines']:  # the fleet's stored rows stay as the machines sent them
            m['replaced'] = [dict(r) for r in m.get('replaced') or []]
        for part in (self.threads, self.reaper, self.auto, self.library, self.stars, self.groups, self.budget):  # stars and groups after threads: they follow a thread
            try:
                part.decorate(snap)
            except Exception:
                log('decorate failed: ' + traceback.format_exc())
        return snap

    def get(self, path, query=None):
        query = query or {}
        if path.startswith('/api/library'):
            return self.library.get(path, query)
        if path in ('/api/cleanup', '/api/reaper'):
            return self.reaper.report()
        if path == '/api/groups':
            return {'ok': True, 'groups': self.groups.all()}
        if path == '/api/budget':
            return self.budget.view()
        m = re.fullmatch(r'/api/thread/([a-z0-9]+)/([0-9a-f-]{36})', path)
        if m:
            self.app._machine(m.group(1))
            r = self.threads.thread(m.group(1), m.group(2))
            if not r:
                raise self.HttpError(404, 'That chat is not one Switchboard has seen.')
            return r
        if path == '/api/chats':
            ex = (query.get('exclude') or [''])[0]
            return {'ok': True, 'chats': self.threads.search((query.get('q') or [''])[0][:200], ex or None)}
        if path == '/api/caps':
            return self.auto.all()
        if path == '/api/browse':
            if not any(v.get('rows') for v in self.browse.idx.values()):
                self.browse.refresh_all(wait=8)  # the first look: give the machines a moment
            else:
                self.browse.refresh_all()  # in the background; this answer uses what is indexed
            try:
                return self.browse.query(query)
            except ValueError as e:
                raise self.HttpError(400, str(e))
        m = re.fullmatch(r'/api/summary/([a-z0-9]+)/([0-9a-f-]{36})', path)
        if m:
            return self.app._call(self.app._machine(m.group(1)), {'verb': 'summary', 'sid': m.group(2)}, timeout=30)
        m = re.fullmatch(r'/api/dispatch/(d-[0-9a-f]{12})', path)
        if m:
            return self.dispatch.status(m.group(1))
        return None

    def post(self, path, body, origin='web'):
        if path.startswith('/api/library/'):
            return self.library.post(path, body)
        if path == '/api/route':
            prompt, folder = body.get('prompt') or '', body.get('folder') or ''
            if not isinstance(prompt, str) or not isinstance(folder, str):
                raise self.HttpError(400, 'prompt and folder are text')
            cm = body.get('chrome_machine') if isinstance(body.get('chrome_machine'), str) else None
            return self.auto.route(prompt, folder, rules_only=body.get('rules') is True, chrome=body.get('chrome') is True,
                                   chrome_machine=cm)
        if path == '/api/dispatch':
            return self.dispatch.start(body)
        if path == '/api/cleanup':
            return self.cleanup(body)
        if path == '/api/tabs':
            try:
                return {'ok': True, 'tabs': self.reaper.set_tabs(body.get('device'), body.get('tabs'))}
            except ValueError as e:
                raise self.HttpError(400, str(e))
        if path == '/api/groups':
            return self.groups.op(body)
        if path == '/api/budget':
            return self.budget.op(body)
        return None

    def cleanup(self, body):
        """The cleanup card: {action: 'stop', items: [{machine, sid} | {machine, pid}]} or {action: 'keep', machine,
        sid | pid} (never suggest it again)."""
        act = body.get('action')
        try:
            if act == 'stop':
                items = body.get('items')
                if not isinstance(items, list) or not items:
                    raise ValueError('nothing to stop')
                for it in items:
                    if isinstance(it, dict):
                        self.app._machine(it.get('machine') if isinstance(it.get('machine'), str) else '')
                return self._stop_checked(items, body.get('when'))
            if act == 'keep':
                name = self.app._machine(body.get('machine') if isinstance(body.get('machine'), str) else '')
                sid, pid = body.get('sid'), body.get('pid')
                if isinstance(sid, str) and SID_RE.fullmatch(sid):
                    return {'ok': True, 'keep': self.reaper.set_keep(name, sid, True)}
                if isinstance(pid, int) and pid > 1:
                    return {'ok': True, 'keep': self.reaper.set_keep(name, f'pid:{pid}', True)}
                raise ValueError('keep needs a session id or a process id')
        except ValueError as e:
            raise self.HttpError(400, str(e))
        raise self.HttpError(400, 'unknown cleanup action')

    def _stop_checked(self, items, when):
        """Stop, but never cut off a chat's subagents, background commands or reply without asking: with no `when`,
        busy chats come back as need 'busy'; 'idle' stops the rest now and queues those until their work ends;
        'now' stops them all."""
        busy = []
        if when != 'now':
            for it in items:
                if isinstance(it, dict) and isinstance(it.get('sid'), str) and SID_RE.fullmatch(it['sid']):
                    b = self.app.busy(it['machine'], it['sid'])
                    if b['live']:
                        busy.append((it, b))
        if busy and when != 'idle':
            n = len(busy)
            lead = busy[0][1]['summary'] if n == 1 else f'{n} of these chats still have work running.'
            return {'ok': False, 'need': 'busy', 'error': f'{lead} Stop them when it finishes, or now, which cuts it off.',
                    'busy': [dict(b, machine=it['machine'], sid=it['sid'], title=self.app._title(it['machine'], it['sid']))
                             for it, b in busy]}
        later = {(it['machine'], it['sid']) for it, _ in busy}
        rest = [it for it in items if not (isinstance(it, dict) and (it.get('machine'), it.get('sid')) in later)]
        out = dict(self.reaper.stop(rest), ok=True) if rest else {'ok': True, 'stopped': [], 'skipped': [], 'failed': []}
        out['queued'] = []
        for m, sid in sorted(later):
            r = self.app._submit('stop', m, {'req': {'verb': 'stop', 'sid': sid}, 'wait_idle': True}, self.app._title(m, sid))
            out['queued'].append({'key': ck(m, sid), 'job': r['job']})
        return out

    def resolve_auto(self, jid, job):
        return self.auto.resolve_job(jid, job)

    def _target(self, body, name):
        sid = body.get('sid')
        if not isinstance(sid, str) or not SID_RE.fullmatch(sid):
            raise self.HttpError(400, 'bad session id')
        return ck(name, sid)

    def action(self, name, verb, body):
        k = self._target(body, name)
        sid = k.split('/', 1)[1]
        if verb == 'keep':
            on = body.get('on')
            return {'ok': True, 'keep': self.reaper.set_keep(name, sid, on if isinstance(on, bool) else None)}
        if verb == 'star':  # a favorite: the chat and every later part of its thread
            on = body.get('on')
            t = self.threads.thread(name, sid) or {}
            others = [c.get('sid') for c in t.get('chats') or []]
            return {'ok': True, 'star': self.stars.set(sid, t.get('thread'), on if isinstance(on, bool) else None, others)}
        if verb == 'link':
            to = body.get('to') if isinstance(body.get('to'), dict) else {}
            target = self._target(to, self.app._machine(to.get('machine') if isinstance(to.get('machine'), str) else ''))
            try:
                self.threads.link(k, target)
            except ValueError as e:
                raise self.HttpError(400, str(e))
            return dict(self.threads.thread(name, sid), ok=True)
        if verb == 'rename':
            name_ = body.get('name')
            if not (isinstance(name_, str) and (name_ == '' or (NAME_RE.fullmatch(name_) and name_ == name_.strip()))):
                raise self.HttpError(400, "A name may use letters, digits, spaces and . , : ' + # ( ) - _ (up to 64).")
            self.app.set_name(k, name_)
            try:  # the machine keeps it too (and writes it into a stopped chat's transcript)
                self.app._call(name, {'verb': 'rename', 'sid': sid, 'name': name_}, timeout=20)
            except Exception as e:
                log(f'rename on {name}: {e}')
            self.app._refresh_soon(name, 0.5)
            return {'ok': True, 'name': name_}
        if verb == 'unlink':
            if body.get('older') is True:
                self.threads.unlink_newer(k)
            else:
                self.threads.unlink(k)
            return dict(self.threads.thread(name, sid) or {'thread': sid, 'chats': []}, ok=True)
        raise self.HttpError(400, 'unknown action')

    def job_done(self, job):
        """A server job finished: a fresh, resume or continue links the new chat to the one it came from, and the new
        chat keeps the tools it started with."""
        self.library.job_done(job)
        try:
            kind, plan = job.get('kind'), job.get('plan') or {}
            if job.get('state') != 'done' or not job.get('sid') or kind not in ('fresh', 'resume', 'continue', 'relaunch', 'compact'):
                return
            if kind == 'continue':
                old = ck(plan.get('src'), plan.get('sid'))
            else:
                old = ck(job['machine'], (plan.get('req') or {}).get('sid'))
            new = ck(job['machine'], job['sid'])
            self.threads.record(new, old, {'fresh': 'fresh', 'resume': 'resumed', 'relaunch': 'resumed', 'continue': 'continued',
                                           'compact': 'compacted'}[kind])
        except Exception:
            log('job_done failed: ' + traceback.format_exc())
