"""The one status line every client shows under the plan gauges: the thing most worth knowing right now, from the
user's own sessions and usage. In order: something needs them, a big warm chat is about to go cold, usage is
climbing much faster than usual, the week or the 5-hour window is running out, then plain pace. Short and calm;
None when there is nothing to say.

{"kind": "need"|"cooling"|"burn"|"week"|"five"|"pace", "tone": "need"|"warm"|"warn"|"bad"|"ok", "text": "...",
 "machine": "<name>", "sid": "<session id>"}   (machine and sid only when the line is about one chat)
"""
import time

GUARD_CTX = 150_000      # a chat this big re-reads a lot once its cache is cold
COOLING_SOON = 5 * 60    # say so when a big warm chat has this long left
BURN_MIN = 3.0           # points of the week in the last hour, at least,
BURN_TIMES = 3.0         # and this many times the week's hourly average, before it counts as unusual
WEEK = 7 * 86400
OLD_LIMITS = 3600        # usage numbers older than this are named as old in any advice built on them


def _left(secs):
    secs = max(0, int(secs))
    if secs < 90:
        return '1 min' if secs >= 30 else 'under a minute'
    if secs < 3600:
        return f'{round(secs / 60)} min'
    if secs < 86400:
        h = secs / 3600
        return f'{round(h)} h' if h >= 3 else f'{h:.1f} h'.replace('.0 h', ' h')
    d = round(secs / 86400)
    return '1 day' if d == 1 else f'{d} days'


def _title(s):
    t = ' '.join(str(s.get('title') or 'A chat').split())
    return t if len(t) <= 48 else t[:47].rstrip() + '…'


def _waiting(s):
    if 'activity' in s:  # the server's derive() (fleet.py) already decided
        return s['activity'] == 'needs'
    return not s.get('stalled') and (s.get('status') == 'waiting' or (s.get('state') == 'blocked' and s.get('status') not in (None, 'busy', 'stopped')))


def _week_rate(sm, now):
    """(points in the last hour, the week's average points per hour), or None when the trace cannot tell."""
    trace, week, reset = sm.get('week_trace') or [], sm.get('seven_day'), sm.get('seven_day_reset')
    if week is None or not reset or len(trace) < 2:
        return None
    hours = (now - (reset - WEEK)) / 3600
    if hours < 6:  # too early in the week for an average
        return None
    ago = [p for t, p in trace if t <= now - 3600]
    if not ago:
        return None
    return float(week) - float(ago[-1]), float(week) / hours


def pick(machines, sm, now=None, held=lambda m, sid: False):
    """machines: the snapshot's machines (sessions marked with stale, stalled); sm: the fleet summary."""
    now = time.time() if now is None else now
    live = [(m, s) for m in machines if m.get('online') is not False for s in m.get('sessions') or [] if not s.get('stale')]
    # 1. Something needs you: Claude asked a question, or a message is held at the cold-chat guard.
    need = [(m, s) for m, s in live if _waiting(s) or held(m['name'], s.get('sid'))]
    if need:
        m, s = max(need, key=lambda x: x[1].get('active') or 0)
        more = len(need) - 1
        if held(m['name'], s.get('sid')) and not _waiting(s):
            text = f'A message to {_title(s)} is waiting for your OK.'
        else:
            text = f'{_title(s)} is waiting for you on {m.get("label") or m["name"]}.'
        if more:
            text += f' {more} more {"needs" if more == 1 else "need"} you too.'
        return {'kind': 'need', 'tone': 'need', 'text': text, 'machine': m['name'], 'sid': s.get('sid')}
    # 2. A big chat is warm for a few more minutes: replying now is cheap, later it re-reads everything.
    cooling = []
    for m, s in live:
        if 'activity' in s:
            left = (s['cold_at'] - now) if s.get('cold_at') and s.get('activity') == 'quiet' else 0
        else:
            left = (s.get('cache_left') or 0) - max(0, now - (m.get('updated') or now))
        if 0 < left <= COOLING_SOON and (s.get('ctx') or 0) >= GUARD_CTX and s.get('status') != 'busy':
            cooling.append((left, m, s))
    if cooling:
        left, m, s = min(cooling, key=lambda x: x[0])
        k = f'{round((s.get("ctx") or 0) / 1000)}k'
        return {'kind': 'cooling', 'tone': 'warm', 'text': f'{_title(s)} goes cold in {_left(left)}. Reply soon to keep it cheap. '
                f'After that, the next reply re-reads all {k}.',
                'machine': m['name'], 'sid': s.get('sid')}
    week, five, proj = sm.get('seven_day'), sm.get('five_hour'), sm.get('week_projected')
    reset, five_reset = sm.get('seven_day_reset'), sm.get('five_hour_reset')
    # Switchboard only learns the plan's numbers when a session replies. Advice from an old number says how old.
    seen = sm.get('limits_updated')
    age = now - seen if isinstance(seen, (int, float)) and seen > 0 else None
    old = age is not None and age > OLD_LIMITS
    if week is not None and reset and reset <= now:
        return {'kind': 'pace', 'tone': 'ok', 'text': "Your week has reset. You'll see new numbers once a chat replies."}
    aged = (lambda text: f'{text} (Updated {_left(age)} ago.)') if old else (lambda text: text)
    # 3. Usage is climbing much faster than this week's usual.
    rate = _week_rate(sm, now)
    if rate and rate[0] >= BURN_MIN and rate[0] >= BURN_TIMES * rate[1]:
        return {'kind': 'burn', 'tone': 'warn', 'text': aged(f'You used {round(rate[0])}% of the week in the last hour, '
                f'about {round(rate[0] / max(rate[1], 0.1))} times your usual pace.')}
    # 4. The plan: the 5-hour window nearly full, or the week on pace to run out.
    if five is not None and five >= 80 and five_reset and five_reset > now:
        return {'kind': 'five', 'tone': 'bad' if five >= 95 else 'warn',
                'text': aged(f'Your 5-hour window is at {round(five)}%. It frees up in {_left(five_reset - now)}.')}
    if proj is not None and proj > 100 and reset and reset > now:
        return {'kind': 'week', 'tone': 'bad', 'text': aged(f"At this rate you'll run out {_left(reset - now)} before the week resets. "
                'Lighter chats make it last.')}
    if proj is not None and proj > 85:
        return {'kind': 'week', 'tone': 'warn', 'text': aged(f"At this rate you'll end the week around {proj}%, so there's not much room for long runs.")}
    # 5. Plain pace, or nothing.
    if week is None:
        return {'kind': 'pace', 'tone': 'ok', 'text': 'Usage shows up once a chat replies.'}
    if proj is not None:
        return {'kind': 'pace', 'tone': 'ok', 'text': aged(f"At this rate you'll end the week around {proj}%." + (' Plenty of room.' if proj <= 60 else ''))}
    return None
