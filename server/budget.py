"""The usage budget (spec docs/specs/2026-10-13-usage-budget.md): a daily share of the weekly plan limit, an optional
cap on the 5-hour window, and an optional hard limit that blocks new prompts and stops busy background chats.

Settings live in <config>/budget.json; what happened (the hard limit's crossings, a lift) in <state>/budget_state.json
and <state>/budget_events.json. The status is worked out here from what the server already keeps: the plan limits
the machines reported (fleet.Fleet.limits) and the week's samples (fleet.WeekHistory). Every poll carries it to the agents, which keep
it for their hooks (agent/sb.py budget_status)."""
import json, math, os, threading, time, traceback
from concurrent.futures import ThreadPoolExecutor
from datetime import datetime
from pathlib import Path

import durable
import fleet as fleet_mod
import notes

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
    durable.put(p, json.dumps(obj, indent=1), tmp=p.with_name(f'{p.name}.{os.getpid()}.{threading.get_ident()}.tmp'))


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


ROOM_NEAR = 48 * 3600   # the daily caps give way only this close to the weekly reset
OLD_NUMBERS = 600       # plan numbers older than this say how old they are (the apps' LIMITS_FRESH)


def room_today(week, today, week_reset, now):
    """How much of the week today can use without running out before the reset: what was left when today began,
    shared evenly over the days left (today counts as one, and so does a part of a day). With 45% left and the
    reset less than a day away, that is all 45% plus what today already used."""
    if week is None or today is None or not week_reset or week_reset <= now:
        return None
    left = max(0.0, 100.0 - (week - today))
    days = max(1, math.ceil((week_reset - now) / 86400 - 1e-9))
    return round(left / days, 1)


def _with_room(cap, room):
    """A daily cap never counts as reached while the week has more room than that today."""
    return None if cap is None else max(cap, room) if room is not None else cap


def when(ts, now, tz):
    """A reset time in the server's zone: '4:10pm' today, else 'Mon 9am'."""
    try:
        zone = fleet_mod._zone(tz)
        t, today = datetime.fromtimestamp(ts, zone), datetime.fromtimestamp(now, zone).date()
    except (TypeError, ValueError, OverflowError, OSError):
        return None
    clock = t.strftime('%I:%M%p').lstrip('0').lower().replace(':00', '')
    return clock if t.date() == today else f'{t.strftime("%a")} {clock}'


def compute(settings, week, week_reset, five, five_reset, today, now, tz, updated=None):
    """The budget's status from the plan limits (pure: the tests drive it directly)."""
    s, h = settings, settings['hard']
    known = week is not None and bool(week_reset) and week_reset > now
    week_over = week is not None and bool(week_reset) and week_reset <= now
    if not known:
        week, today = None, None
    # The 5-hour number counts only while its window is open. One from a window that has reset is not true any
    # more, and a blocked prompt brings no reply to replace it, so it must never keep a cap or the hard limit tripped.
    if five is not None and five_reset and five_reset > now:
        five_state = 'open'
    else:
        # 'reset': the last window anybody saw this week is over (fleet.merge_limits keeps it that long).
        # 'none': a reply showed the week and nobody saw a 5-hour window all week, so the plan does not report one
        five_state = 'reset' if five is not None and five_reset else 'none' if five is None and known else 'unknown'
        five, five_reset = None, None
    lifted = bool(s.get('lifted_until')) and now < s['lifted_until']
    pace = fleet_mod._projected(week, week_reset, now) if known else None
    # Near the weekly reset, and only there, a day may use whatever the week still has left.
    room = room_today(week, today, week_reset, now) if known and week_reset - now <= ROOM_NEAR else None
    daily = _with_room(s['daily'], room)
    over_daily = today is not None and daily is not None and today >= daily
    over_five = five is not None and s['five_hour'] is not None and five >= s['five_hour']
    hard_cap = _with_room(h['daily'], room)
    hard_daily = h['on'] and today is not None and hard_cap is not None and today >= hard_cap
    hard_five = h['on'] and five is not None and h['five_hour'] is not None and five >= h['five_hour']
    over_hard = bool(hard_daily or hard_five)
    day = datetime.fromtimestamp(now, fleet_mod._zone(tz)).date().isoformat()
    st = {'known': known, 'day': day, 'tz': tz if isinstance(tz, str) else str(tz), 'now': now,
          'today': today, 'room': room, 'daily': daily, 'hard_daily': hard_cap, 'week': week, 'week_reset': week_reset if known else None, 'projected': pace,
          'week_over': week_over, 'five_hour': five, 'five_hour_reset': five_reset, 'five_hour_state': five_state, 'updated': updated,
          'over_daily': over_daily, 'over_five_hour': over_five, 'over_budget': over_daily or over_five,
          'over_hard': over_hard, 'hard_reason': 'daily' if hard_daily else 'five_hour' if hard_five else None,
          'lifted': lifted, 'blocking': over_hard and not lifted}
    st['summary'] = summary_text(settings, st, tz)
    st['notice'] = notice_text(settings, st) if st['over_budget'] else None
    st['block'] = block_text(settings, st, tz) if st['blocking'] else None
    return st


def _hit(used, cap):
    """', over' or ', reached' after a cap, or nothing while under it."""
    return '' if used is None or cap is None or used < cap else ', reached' if used == cap else ', over'


def summary_text(s, st, tz=None):
    """What the usage MCP tool says: one short paragraph. Reset times are in `tz`, the server's zone."""
    now, tz = st['now'], st['tz'] if tz is None else tz
    if not st['known'] and st['five_hour'] is None:
        if st['week_over']:
            return ('The week has reset and no chat has replied since, so Switchboard has no new plan numbers yet. '
                    'They come with the next reply.')
        return 'Switchboard cannot tell the plan usage yet: no Claude session has reported the plan limits recently.'
    parts = []
    raised = ', raised to use what is left before the reset'
    if st['today'] is None:
        parts.append('Today: not known yet' + (f' (daily budget {fmt(s["daily"])}).' if s['daily'] is not None else '.'))
    elif s['daily'] is None:
        parts.append(f'Today used {fmt(st["today"])} of the weekly limit (no daily budget).')
    else:
        a, b = fmt_pair(st['today'], st['daily'])
        parts.append(f'Today used {a} of the weekly limit (daily budget {b}' + (raised if st['daily'] > s['daily'] else '')
                     + _hit(st['today'], st['daily']) + ').')
    if st['week'] is None:
        parts.append('Week: the week has reset; new numbers come with the next reply.' if st['week_over'] else 'Week: not known.')
    else:
        at = when(st['week_reset'], now, tz)
        parts.append(f'Week: {fmt(st["week"])} used' + (f', on pace for {fmt(st["projected"])}' if st['projected'] is not None else '')
                     + (f', resets {at}' if at else '') + '.')
    h = s['hard']
    unchecked = st['five_hour_state'] == 'none'  # a 5-hour cap with no 5-hour number to hold it against
    if st['five_hour_state'] == 'open':
        at = when(st['five_hour_reset'], now, tz)
        a, b = fmt_pair(st['five_hour'], s['five_hour']) if s['five_hour'] is not None else (fmt(st['five_hour']), None)
        parts.append(f'5 hours: {a} used' + (f' (cap {b}{_hit(st["five_hour"], s["five_hour"])})' if b else '')
                     + (f', resets {at}' if at else '') + '.')
    elif st['five_hour_state'] == 'reset':
        parts.append('5 hours: no 5-hour window open; the next reply starts a new one.')
    elif unchecked:
        parts.append('5 hours: not reported on this plan' + (f', so the {fmt(s["five_hour"])} cap cannot be checked'
                                                               if s['five_hour'] is not None else '') + '.')
    else:
        parts.append('5 hours: not known.')
    if not h['on'] or (h['daily'] is None and h['five_hour'] is None):
        parts.append('Hard limit: off.')
    else:
        # The daily limit as it is enforced: near the reset it gives way like the budget does.
        day = st['hard_daily']
        lim = ', '.join(x for x in ((f'{fmt(day)} a day' + (raised if day > h['daily'] else '') if day is not None else ''),
                                    (f'{fmt(h["five_hour"])} of 5 hours' + (' (cannot be checked)' if unchecked else '')
                                     if h['five_hour'] is not None else '')) if x)
        if st['lifted']:
            parts.append(f'Hard limit: {lim}, lifted for today.')
        elif st['over_hard']:
            parts.append(f'Hard limit: {lim}, reached; new prompts are blocked.')
        else:
            parts.append(f'Hard limit: {lim}.')
    age = now - st['updated'] if isinstance(st['updated'], (int, float)) and st['updated'] > 0 else None
    if age is not None and age >= OLD_NUMBERS:
        parts.append(f'These numbers are from {notes._left(age)} ago; they update when a chat replies.')
    return ' '.join(parts)


def notice_text(s, st):
    """The one line a chat gets, once a day, when the user is over budget."""
    pace = f'; week on pace for {fmt(st["projected"])}' if st['projected'] is not None else ''
    if st['over_daily']:
        a, b = fmt_pair(st['today'], st['daily'])
        why = f'over their daily usage budget (today used {a} of the weekly limit, daily budget {b}{pace})'
    else:
        a, b = fmt_pair(st['five_hour'], s['five_hour'])
        why = f'over their 5-hour usage cap (5-hour window at {a}, cap {b}{pace})'
    return f'Switchboard: the user is {why}. Mention this to them once, briefly, then carry on.'


def block_text(s, st, tz=None):
    """Why a prompt was blocked, shown to the user in the chat."""
    h = s['hard']
    if st['hard_reason'] == 'daily':
        a, b = fmt_pair(st['today'], st['hard_daily'])
        return (f"Switchboard hard limit: you've used {a} of your weekly limit today (daily limit {b}). "
                'Lift it for today in Switchboard Settings, or wait until tomorrow.')
    a, b = fmt_pair(st['five_hour'], h['five_hour'])
    at = when(st['five_hour_reset'], st['now'], st['tz'] if tz is None else tz)
    return (f'Switchboard hard limit: your 5-hour window is at {a} (limit {b}). Lift it for today in Switchboard '
            'Settings, or wait for the window to reset' + (f' at {at}.' if at else '.'))


def stop_text(n, st):
    """The notification and event when the hard limit stops chats."""
    chats = 'No background chats were running' if n == 0 else f'Stopped {n} chat' + ('s' if n != 1 else '')
    where = (f"You've used {fmt(st['today'])} of your weekly limit today" if st['hard_reason'] == 'daily'
             else f'Your 5-hour window is at {fmt(st["five_hour"])}')
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
        lim = f.limits(now) if hasattr(f, 'limits') else {}
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
        self.app._push('Hard limit hit', text, EVENTS_URL, 'budget', 'budget')
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
