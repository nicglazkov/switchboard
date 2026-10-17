"""The usage budget (spec docs/specs/2026-10-13-usage-budget.md): a daily share of the weekly plan limit, an optional
cap on the 5-hour window, and an optional hard limit that blocks new prompts and stops busy background chats.

Settings live in <config>/budget.json; what happened (the hard limit's crossings, a lift) in <state>/budget_state.json
and <state>/budget_events.json. The status is worked out here from what the server already keeps: the newest plan
limits a machine reported and the week's samples (fleet.WeekHistory). Every poll carries it to the agents, which keep
it for their hooks (agent/sb.py budget_status)."""
import json, math, os, threading, time, traceback
from concurrent.futures import ThreadPoolExecutor
from datetime import datetime
from pathlib import Path

import fleet as fleet_mod

DEFAULTS = {'daily': 14.3, 'five_hour': None, 'hard': {'on': False, 'daily': 20.0, 'five_hour': None}, 'lifted_until': None}
EVENTS_KEEP = 50
STOP_TIMEOUT = 60
EVENTS_URL = '/?usage=1'


def log(msg):
    print(time.strftime('%Y-%m-%d %H:%M:%S'), msg, flush=True)


def _jload(p, default):
    try:
        d = json.loads(Path(p).read_text())
        return d if isinstance(d, type(default)) else default
    except (OSError, ValueError):
        return default


def _jsave(p, obj):
    p = Path(p)
    p.parent.mkdir(parents=True, exist_ok=True)
    tmp = p.with_name(f'{p.name}.{os.getpid()}.{threading.get_ident()}.tmp')
    tmp.write_text(json.dumps(obj, indent=1))
    os.replace(tmp, p)


class BudgetError(ValueError):
    pass


def _pct(v, what, lo=0.0):
    """A percent setting: null (off) or a number above `lo` and at most 100, kept to one decimal."""
    if v is None:
        return None
    if isinstance(v, bool) or not isinstance(v, (int, float)) or not math.isfinite(v):
        raise BudgetError(f'{what} is a percent from 0.1 to 100, or null for off.')
    v = round(float(v), 1)
    if not lo < v <= 100:
        raise BudgetError(f'{what} is a percent from 0.1 to 100, or null for off.')
    return v


def clean_settings(d):
    """budget.json as read from disk: anything broken falls back to its default, field by field."""
    d = d if isinstance(d, dict) else {}
    out = json.loads(json.dumps(DEFAULTS))

    def keep(src, dst, key, check):
        if key in src:
            try:
                dst[key] = check(src[key])
            except BudgetError:
                pass
    keep(d, out, 'daily', lambda v: _pct(v, 'daily'))
    keep(d, out, 'five_hour', lambda v: _pct(v, 'five_hour'))
    h = d.get('hard') if isinstance(d.get('hard'), dict) else {}
    if isinstance(h.get('on'), bool):
        out['hard']['on'] = h['on']
    keep(h, out['hard'], 'daily', lambda v: _pct(v, 'hard.daily'))
    keep(h, out['hard'], 'five_hour', lambda v: _pct(v, 'hard.five_hour'))
    lu = d.get('lifted_until')
    if isinstance(lu, (int, float)) and not isinstance(lu, bool) and math.isfinite(lu):
        out['lifted_until'] = float(lu)
    return out


def apply_set(cur, body):
    """POST /api/budget {op: 'set', ...}: only the fields given change. Strict: an unknown field or a bad value is
    refused whole, and nothing is saved."""
    extra = set(body) - {'op', 'daily', 'five_hour', 'hard'}
    if extra:
        raise BudgetError(f'Unknown budget field: {sorted(extra)[0]}.')
    if len(body) == 1:
        raise BudgetError('Nothing to set.')
    new = json.loads(json.dumps(cur))
    if 'daily' in body:
        new['daily'] = _pct(body['daily'], 'daily')
    if 'five_hour' in body:
        new['five_hour'] = _pct(body['five_hour'], 'five_hour')
    if 'hard' in body:
        h = body['hard']
        if not isinstance(h, dict) or not h:
            raise BudgetError('hard is an object: {on, daily, five_hour}.')
        extra = set(h) - {'on', 'daily', 'five_hour'}
        if extra:
            raise BudgetError(f'Unknown hard limit field: {sorted(extra)[0]}.')
        if 'on' in h:
            if not isinstance(h['on'], bool):
                raise BudgetError('hard.on is true or false.')
            new['hard']['on'] = h['on']
        if 'daily' in h:
            new['hard']['daily'] = _pct(h['daily'], 'hard.daily')
        if 'five_hour' in h:
            new['hard']['five_hour'] = _pct(h['five_hour'], 'hard.five_hour')
    return new


def fmt(v):
    """18.2 -> '18%', 0.4 -> '0.4%'."""
    if v is None:
        return '?'
    r = round(v)
    return f'{r}%' if r >= 1 or v == 0 else f'{v:.1f}%'


def fmt_pair(a, b):
    """Two percents told apart: '14.4%' and '14.3%' when they would both print as 14%."""
    if a is not None and b is not None and round(a) == round(b) and a != b:
        return f'{a:.1f}%', f'{b:.1f}%'
    return fmt(a), fmt(b)


def compute(settings, week, week_reset, five, five_reset, today, now, tz, updated=None):
    """The budget's status from the plan limits (pure: the tests drive it directly)."""
    s, h = settings, settings['hard']
    known = week is not None and bool(week_reset) and week_reset > now
    if not known:
        week, today = None, None
    lifted = bool(s.get('lifted_until')) and now < s['lifted_until']
    pace = fleet_mod._projected(week, week_reset, now) if known else None
    over_daily = today is not None and s['daily'] is not None and today >= s['daily']
    over_five = five is not None and s['five_hour'] is not None and five >= s['five_hour']
    hard_daily = h['on'] and today is not None and h['daily'] is not None and today >= h['daily']
    hard_five = h['on'] and five is not None and h['five_hour'] is not None and five >= h['five_hour']
    over_hard = bool(hard_daily or hard_five)
    day = datetime.fromtimestamp(now, fleet_mod._zone(tz)).date().isoformat()
    st = {'known': known, 'day': day, 'tz': tz if isinstance(tz, str) else str(tz),
          'today': today, 'week': week, 'week_reset': week_reset if known else None, 'projected': pace,
          'five_hour': five, 'five_hour_reset': five_reset if five is not None else None, 'updated': updated,
          'over_daily': over_daily, 'over_five_hour': over_five, 'over_budget': over_daily or over_five,
          'over_hard': over_hard, 'hard_reason': 'daily' if hard_daily else 'five_hour' if hard_five else None,
          'lifted': lifted, 'blocking': over_hard and not lifted}
    st['summary'] = summary_text(settings, st)
    st['notice'] = notice_text(settings, st) if st['over_budget'] else None
    st['block'] = block_text(settings, st) if st['blocking'] else None
    return st


def summary_text(s, st):
    """What the usage MCP tool says: one short paragraph."""
    if not st['known'] and st['five_hour'] is None:
        return 'Switchboard cannot tell the plan usage yet: no Claude session has reported the plan limits recently.'
    parts = []
    if st['today'] is None:
        parts.append('Today: not known yet' + (f' (budget {fmt(s["daily"])}).' if s['daily'] is not None else '.'))
    elif s['daily'] is None:
        parts.append(f'Today: {fmt(st["today"])} of the week (no daily budget).')
    else:
        a, b = fmt_pair(st['today'], s['daily'])
        parts.append(f'Today: {a} of the week (budget {b}' + (', over' if st['over_daily'] else '') + ').')
    if st['week'] is None:
        parts.append('Week: not known.')
    elif st['projected'] is not None:
        parts.append(f'Week: {fmt(st["week"])}, on pace for {fmt(st["projected"])}.')
    else:
        parts.append(f'Week: {fmt(st["week"])}.')
    if st['five_hour'] is None:
        parts.append('5 hours: not known.')
    elif s['five_hour'] is not None:
        a, b = fmt_pair(st['five_hour'], s['five_hour'])
        parts.append(f'5 hours: {a} (cap {b}' + (', over' if st['over_five_hour'] else '') + ').')
    else:
        parts.append(f'5 hours: {fmt(st["five_hour"])}.')
    h = s['hard']
    if not h['on'] or (h['daily'] is None and h['five_hour'] is None):
        parts.append('Hard limit: off.')
    else:
        lim = ', '.join(x for x in ((f'{fmt(h["daily"])} a day' if h['daily'] is not None else ''),
                                    (f'{fmt(h["five_hour"])} of 5 hours' if h['five_hour'] is not None else '')) if x)
        if st['lifted']:
            parts.append(f'Hard limit: {lim}, lifted for today.')
        elif st['over_hard']:
            parts.append(f'Hard limit: {lim}, reached; new prompts are blocked.')
        else:
            parts.append(f'Hard limit: {lim}.')
    return ' '.join(parts)


def notice_text(s, st):
    """The one line a chat gets, once a day, when the user is over budget."""
    pace = f'; week on pace for {fmt(st["projected"])}' if st['projected'] is not None else ''
    if st['over_daily']:
        a, b = fmt_pair(st['today'], s['daily'])
        why = f'over their daily usage budget (today {a} of the week, budget {b}{pace})'
    else:
        a, b = fmt_pair(st['five_hour'], s['five_hour'])
        why = f'over their 5-hour usage cap (5-hour window at {a}, cap {b}{pace})'
    return f'Switchboard: the user is {why}. Mention this to them once, briefly, then carry on.'


def block_text(s, st):
    """Why a prompt was blocked, shown to the user in the chat."""
    h = s['hard']
    if st['hard_reason'] == 'daily':
        a, b = fmt_pair(st['today'], h['daily'])
        return (f'Switchboard hard limit: today is at {a} of the week (limit {b}). Lift it for today in Switchboard '
                'Settings, or wait until tomorrow.')
    a, b = fmt_pair(st['five_hour'], h['five_hour'])
    return (f'Switchboard hard limit: the 5-hour window is at {a} (limit {b}). Lift it for today in Switchboard '
            'Settings, or wait for the window to reset.')


def stop_text(n, st):
    """The notification and event when the hard limit stops chats."""
    chats = 'No background chats were working' if n == 0 else f'Stopped {n} chat' + ('s' if n != 1 else '')
    where = (f'Today at {fmt(st["today"])} of the week' if st['hard_reason'] == 'daily'
             else f'The 5-hour window is at {fmt(st["five_hour"])}')
    return f'{chats}. {where}.'


class Budget:
    def __init__(self, app, cfg_dir, http_error):
        self.app, self.HttpError = app, http_error
        self.path = Path(cfg_dir) / 'budget.json'
        self.state_path = app.state_dir / 'budget_state.json'
        self.events_path = app.state_dir / 'budget_events.json'
        self.lock, self.busy = threading.Lock(), threading.Lock()

    # ---- settings
    def settings(self):
        return clean_settings(_jload(self.path, {}))

    def _save(self, s):
        with self.lock:
            _jsave(self.path, s)

    def _tz(self):
        h = getattr(self.app.fleet, 'history', None)
        return getattr(h, 'tz', None) or fleet_mod.DAY_TZ

    def op(self, body):
        op = body.get('op')
        now = self.app.clock()
        if op == 'set':
            try:
                self._save(apply_set(self.settings(), body))
            except BudgetError as e:
                raise self.HttpError(400, str(e))
        elif op == 'lift':
            if len(body) != 1:
                raise self.HttpError(400, 'lift takes no fields.')
            until = fleet_mod.end_of_day(now, self._tz())
            self._save(dict(self.settings(), lifted_until=until))
            st = self._state()
            st['lift_day'] = self.status(now)['day']
            _jsave(self.state_path, st)
            self._event({'kind': 'lift', 'until': until, 'text': 'Hard limit lifted for the rest of today.'})
        elif op == 'unlift':
            if len(body) != 1:
                raise self.HttpError(400, 'unlift takes no fields.')
            self._save(dict(self.settings(), lifted_until=None))
        else:
            raise self.HttpError(400, 'unknown budget op (set, lift or unlift)')
        if hasattr(self.app, 'bump'):
            self.app.bump()
        return self.view()

    # ---- status
    def status(self, now=None):
        now = self.app.clock() if now is None else now
        f = self.app.fleet
        lim = f._newest_limits(f.snapshot()['machines']) if hasattr(f, '_newest_limits') else {}
        week, reset = lim.get('seven_day'), lim.get('seven_day_reset')
        h = getattr(f, 'history', None)
        today = h.today(week, reset, now) if h is not None and hasattr(h, 'today') else None
        st = compute(self.settings(), week, reset, lim.get('five_hour'), lim.get('five_hour_reset'), today, now,
                     self._tz(), lim.get('updated'))
        return st

    def agent_view(self):
        """What every poll tells the agents (fleet.Fleet._list_req), for their hooks and the usage MCP tool."""
        st = self.status()
        return {k: st[k] for k in ('known', 'day', 'over_budget', 'blocking', 'summary', 'notice', 'block')}

    def settings_view(self, now=None):
        s = self.settings()
        now = self.app.clock() if now is None else now
        if s['lifted_until'] and s['lifted_until'] <= now:
            s['lifted_until'] = None
        return s

    def view(self):
        now = self.app.clock()
        return {'ok': True, 'settings': self.settings_view(now), 'status': self.status(now),
                'events': list(reversed(_jload(self.events_path, [])))[:10]}

    def decorate(self, snap):
        now = self.app.clock()
        snap['budget'] = {'settings': self.settings_view(now), 'status': self.status(now)}

    # ---- the hard limit: stop busy background chats once, the first time a day it is crossed
    def _state(self):
        return _jload(self.state_path, {})

    def _event(self, e):
        e = dict(e, at=self.app.clock())
        with self.lock:
            rows = _jload(self.events_path, [])
            rows.append(e)
            _jsave(self.events_path, rows[-EVENTS_KEEP:])

    def after_poll(self):
        if not self.busy.acquire(blocking=False):
            return None
        try:
            return self._check()
        except Exception:
            log('budget failed: ' + traceback.format_exc())
            return None
        finally:
            self.busy.release()

    def _check(self):
        now = self.app.clock()
        st = self.status(now)
        if not st['blocking']:
            return None
        state = self._state()
        if state.get('hard_day') == st['day'] or state.get('lift_day') == st['day']:
            return None  # once a day, and never again on a day it was lifted
        state['hard_day'] = st['day']
        _jsave(self.state_path, state)
        stopped, failed = self.stop_all()
        n = len(stopped)
        text = stop_text(n, st)
        log(f'budget: hard limit hit ({st["hard_reason"]}, today {st["today"]}, 5h {st["five_hour"]}): '
            f'stopped {n}' + (f', {len(failed)} machines failed' if failed else ''))
        self._event({'kind': 'hard', 'day': st['day'], 'reason': st['hard_reason'], 'today': st['today'],
                     'five_hour': st['five_hour'], 'stopped': stopped, 'failed': failed, 'text': f'Hard limit hit: {text}'})
        self.app._push('Hard limit hit', text, EVENTS_URL)
        if hasattr(self.app, 'bump'):
            self.app.bump()
        return {'stopped': stopped, 'failed': failed}

    def stop_all(self):
        """Ask every online machine whose agent knows budget_stop to stop its busy background chats (claude stop:
        they can be resumed; nothing is deleted)."""
        f = self.app.fleet
        names = [n for n in f.order if (f.state.get(n) or {}).get('online')
                 and 'budget_stop' in ((f.state.get(n) or {}).get('features') or [])]

        def one(name):
            try:
                r = self.app._call(name, {'verb': 'budget_stop'}, timeout=STOP_TIMEOUT)
                return name, [s for s in r.get('stopped') or [] if isinstance(s, dict)], None
            except Exception as e:
                return name, [], str(e)[:160]
        stopped, failed = [], []
        if names:
            with ThreadPoolExecutor(len(names)) as ex:
                for name, rows, err in ex.map(one, names):
                    for s in rows:
                        sid = s.get('sid') if isinstance(s.get('sid'), str) else None
                        stopped.append({'machine': name, 'sid': sid, 'title': self.app._title(name, sid) if sid else None})
                    if err:
                        failed.append({'machine': name, 'error': err})
                    if rows and hasattr(self.app, '_refresh_soon'):
                        self.app._refresh_soon(name, 1)
        return stopped, failed
