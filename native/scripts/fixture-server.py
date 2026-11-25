#!/usr/bin/env python3
"""A stand-in Switchboard server for screenshots and driving the desktop app with no fleet behind it.

Serves native/scripts/fixtures/fixture.json as /api/sessions (times shifted to now, so warm chats stay warm), the status
note from server/notes.py, and small canned replies for the other GETs. POSTs are accepted and remembered
(prefs and machines change in memory); nothing reaches a real machine.

    python3 native/scripts/fixture-server.py [port]          default port 8139
    SWITCHBOARD_URL=http://127.0.0.1:8139 target/shots/Switchboard
"""
import copy, json, sys, time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import parse_qs, urlsplit

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'server'))
import notes  # noqa: E402
import os  # noqa: E402
sys.path.insert(0, str(ROOT / 'agent'))
os.environ.setdefault('SB_HOME', str(Path(os.environ.get('TMPDIR', '/tmp')) / 'sb-fixture-home'))
import sb  # noqa: E402  (the agent's cost model, so every chat carries the cost a real agent sends)
# How old the plan numbers are, in seconds (SB_FIXTURE_LIMITS_AGE), to see the "as of" states.
LIMITS_AGE = float(os.environ.get('SB_FIXTURE_LIMITS_AGE') or 60)
import features  # noqa: E402  (tab groups: the server's own store and rules, kept in a scratch file)
import options as options_mod  # noqa: E402  (the opt-in switches, as the server words them)
import models as models_mod  # noqa: E402  (the model menu, as the server builds it)

# The model catalog an agent reports (agent model_catalog), for GET /api/models: a new Fable the chats have not run on.
CATALOG = {'fetched': 1, 'targets': {'fable': 'claude-fable-5-2', 'opus': 'claude-opus-5-5', 'sonnet': 'claude-sonnet-5-5',
                                     'haiku': 'claude-haiku-5-5'},
           'models': [{'id': 'claude-opus-5-5', 'name': 'Opus 5.5', 'desc': 'For complex work and everyday tasks', 'main': True},
                      {'id': 'claude-fable-5-2', 'name': 'Fable 5.2', 'desc': 'For your toughest challenges', 'main': True},
                      {'id': 'claude-sonnet-5-5', 'name': 'Sonnet 5.5', 'desc': 'Most efficient for simpler tasks', 'main': True},
                      {'id': 'claude-haiku-5-5', 'name': 'Haiku 5.5', 'desc': 'Fastest for quick answers', 'main': True}]
           + [{'id': f'claude-{m}', 'name': n, 'main': False} for m, n in
              [('fable-5-1', 'Fable 5.1'), ('fable-5', 'Fable 5'), ('opus-5', 'Opus 5'), ('opus-4-8', 'Opus 4.8'),
               ('sonnet-5', 'Sonnet 5'), ('sonnet-4-6', 'Sonnet 4.6'), ('haiku-4-5-20251001', 'Haiku 4.5')]]}


# SB_FIXTURE_SERVER=ok|old|updating|failed|stopped: Settings > Server. "old" is a server from before 0.8.11 (no
# /api/server), "updating" is part way through a download, "stopped" has the hub down.
SERVER_MODE = os.environ.get('SB_FIXTURE_SERVER') or 'ok'


def server_info():
    now = time.time()
    up = None
    if SERVER_MODE == 'updating':
        up = {'step': 'downloading', 'version': '0.8.12', 'from': '0.8.11', 'started': now - 8, 'at': now, 'ok': None,
              'got': 3_100_000, 'total': 7_400_000}
    elif SERVER_MODE == 'failed':
        up = {'step': 'downloading', 'version': '0.8.12', 'from': '0.8.11', 'started': now - 300, 'at': now - 240,
              'ok': False, 'finished': now - 240, 'error': "the download doesn't match the release's signed list, so it was not installed"}
    return {'ok': True, 'version': '0.8.11', 'commit': 'abc1234', 'mode': 'systemd', 'started': now - 3 * 86400 - 5000,
            'host': 'homeserver', 'os': 'Ubuntu 24.04.1 LTS', 'python': '3.12.3',
            'hub': {'installed': True, 'running': SERVER_MODE != 'stopped', 'version': '0.8.11', 'open': 3, 'linked': 2},
            'downloads': {'running': True}, 'disk': {'free': 182_000_000_000, 'total': 250_000_000_000},
            'can_manage': True, 'why_not': None, 'update': up,
            'install_line': 'curl -fsSL https://github.com/nicglazkov/switchboard/releases/latest/download/get-server.sh | bash'}


SERVER_LOG = '\n'.join(f'2026-10-23T09:{m:02d}:00-0700 homeserver python3[812]: GET /api/sessions 200 ({m % 7 + 2} ms)'
                        for m in range(0, 40))


class HttpError(Exception):
    def __init__(self, code, msg):
        super().__init__(msg)
        self.code, self.msg = code, msg


GROUPS = features.Groups(Path(os.environ['SB_HOME']) / 'fixture-groups.json', HttpError)
# SB_FIXTURE_GROUPS=1: three tab groups to start with (a long name, a short one, one with no name), 8 of the chats.
SEED_GROUPS = [
    {'id': 'a1b2c3d4', 'name': 'Switchboard Mac and iPhone polish pass', 'color': 'blue',
     'members': ['c03ef0d9-fb19-40d7-ba39-eca0294f5d87', '0cc479d1-37ab-40d6-872d-23f341ecf872', 'f81f1ac9-8f5d-4eb3-a97f-d1067b4497b7']},
    {'id': 'b2c3d4e5', 'name': 'Kitchen', 'color': 'red',
     'members': ['5c283686-ec27-4c97-b3d1-5495d5faa56b', 'eb945c42-96ca-48a5-8b10-94cd51d1bf18', 'a0ef6336-80ee-4b71-9202-e0da39469e21']},
    {'id': 'c3d4e5f6', 'name': '', 'color': 'green',
     'members': ['df98b800-4ebc-42da-ac85-2792f47625d7', '784a23de-69e0-4424-995c-b30a4edb42bb']},
]
GROUPS.path.parent.mkdir(parents=True, exist_ok=True)
GROUPS.path.write_text(json.dumps(SEED_GROUPS if os.environ.get('SB_FIXTURE_GROUPS') else []))

FIX = json.loads((ROOT / 'native/scripts/fixtures/fixture.json').read_text())
START = time.time()
PREFS = {'chrome_machine': None, 'last': None, 'presets': [], 'summary_model': 'opus', 'auto_stop_h': 6,
         'appearance': {'theme': 'system', 'term': {'scheme': 'converge-dark', 'size': None, 'size_phone': None,
                                                    'cursor': 'block', 'blink': True}}}
VERSION = {'version': '0.1.0', 'build': 'fixture'}
# The opt-in switches, worded by the server's own options.py. SB_FIXTURE_OPTIONS=on starts them all on (an install
# that had them before); SB_FIXTURE_PUSH=1 has iPhone notifications set up.
OPTIONS = {k: os.environ.get('SB_FIXTURE_OPTIONS') == 'on' for k in options_mod.KEYS}
PUSH_READY = bool(os.environ.get('SB_FIXTURE_PUSH'))

# SB_FIXTURE_COMPACT: Settings > Context window. Words separated by commas:
#   a number (500000, 250000)  the value the server starts with, switched on
#   on                         switched on at 300000
#   mixed   Laptop has a value its user set (500000), Tower's helper is older (knows 300k only, reports nothing)
#   models  Laptop has a window per model saved in Claude Code (/autocompact) instead
#   slow    computers follow a change a few seconds late, to see "Changing to ..."
#   old     a server from before the value existed: the switch only, and nothing reported per computer
COMPACT_WORDS = [w for w in (os.environ.get('SB_FIXTURE_COMPACT') or '').split(',') if w]
COMPACT_OLD = 'old' in COMPACT_WORDS
COMPACT = {'at': next((int(w) for w in COMPACT_WORDS if w.isdigit()), options_mod.COMPACT_DEFAULT), 'changed': 0.0}
if any(w.isdigit() or w == 'on' for w in COMPACT_WORDS):
    OPTIONS['setup_compact_window'] = True
COMPACT['was'] = (OPTIONS['setup_compact_window'], COMPACT['at'])  # what the computers have until they follow


def compact_seen(name):
    """What one computer's helper reports about its own compact window (agent/sb.py compact_state)."""
    if COMPACT_OLD or ('mixed' in COMPACT_WORDS and name == 'tower'):
        return None
    if 'mixed' in COMPACT_WORDS and name == 'laptop':
        return {'set': True, 'raw': '500000', 'value': 500_000, 'own': False, 'other': None}
    if 'models' in COMPACT_WORDS and name == 'laptop':
        return {'set': False, 'raw': None, 'value': None, 'own': False, 'other': 'models'}
    late = 'slow' in COMPACT_WORDS and time.time() - COMPACT['changed'] < 6
    on, at = COMPACT['was'] if late else (OPTIONS['setup_compact_window'], COMPACT['at'])
    if not late:
        COMPACT['was'] = (on, at)
    return {'set': on, 'raw': str(at) if on else None, 'value': at if on else None, 'own': on, 'other': None}


def prefs_view():
    view = options_mod.view(OPTIONS, PUSH_READY, PREFS.get('notify'), COMPACT['at'])
    if COMPACT_OLD:
        view.pop('compact')
    return dict(PREFS, ok=True, **view)


# SB_FIXTURE_LIBRARY=auto: a tool library from a server that knows Auto (tools say `auto` and `when`, chats say
# `tools_mode`); SB_FIXTURE_LIBRARY=old: the same library from an older server, with no Auto.
LIB_MODE = os.environ.get('SB_FIXTURE_LIBRARY') or ''
LIB = {'tools': [
    {'name': 'web-fetch', 'description': 'Fetch pages that block plain requests', 'kinds': ['mcp'], 'mode': 'local', 'latest': 3,
     'auto': True, 'when': 'A page answers with a Cloudflare or Akamai block, after a normal fetch failed'},
    {'name': 'video-notes', 'description': 'Search YouTube and read transcripts, comments and frames', 'kinds': ['skill'], 'mode': 'local', 'latest': 2,
     'auto': True, 'when': ''},
    {'name': 'price-check', 'description': 'Prices, sold listings and seller details from online marketplaces', 'kinds': ['skill'], 'mode': 'local', 'latest': 1,
     'auto': True, 'when': ''},
    {'name': 'home-assistant', 'description': 'Read and switch lights, blinds and sensors at home', 'kinds': ['mcp'], 'mode': 'local', 'latest': 4,
     'auto': True, 'when': 'Anything about lights, blinds, curtains or sensors at home'},
    {'name': 'personal-gdocs', 'description': 'Personal Sheets and Docs', 'kinds': ['mcp'], 'mode': 'local', 'latest': 1,
     'auto': True, 'when': 'Reading or writing a Sheet or Doc the user shared with it'},
    {'name': 'image-edit', 'description': 'Image editing through a local editor', 'kinds': ['mcp'], 'mode': 'local', 'latest': 1,
     'auto': True, 'when': ''},
    {'name': 'api-hub', 'description': 'Paid scrapers and data APIs (costs money per call)', 'kinds': ['mcp'], 'mode': 'local', 'latest': 1,
     'auto': False, 'when': 'Only when nothing local can get the page, and say the cost'},
], 'defaults': ['video-notes'], 'machines': {}}


# SB_FIXTURE_BUDGET=ok|over|hard|lifted: the usage budget (docs/specs/2026-10-13-usage-budget.md) with today at 7%,
# 18% (over the 14.3% budget), 22% (past a 20% hard limit) or 22% with the hard limit lifted.
BUDGET_MODE = os.environ.get('SB_FIXTURE_BUDGET') or ''
BUDGET = {'daily': 14.3, 'five_hour': None, 'hard': {'on': BUDGET_MODE in ('hard', 'lifted'), 'daily': 20.0, 'five_hour': None},
          'lifted_until': time.time() + 3 * 3600 if BUDGET_MODE == 'lifted' else None}


def budget():
    """Settings and a status worked out the way server/budget.py does, from a fixed today."""
    s = copy.deepcopy(BUDGET)
    today = {'ok': 7.0, 'over': 18.0}.get(BUDGET_MODE, 22.0)
    h = s['hard']
    lifted = bool(s['lifted_until'] and s['lifted_until'] > time.time())
    over_daily = s['daily'] is not None and today >= s['daily']
    over_hard = h['on'] and h['daily'] is not None and today >= h['daily']
    fmt = lambda x: f'{x:g}'
    summary = f'Today: {fmt(today)}% of the week' + (f' (budget {fmt(s["daily"])}%, {"over" if over_daily else "under"}).' if s['daily'] is not None else '.')
    summary += ' Week: 51%, on pace for 112%. 5 hours: 4%. Hard limit: ' + (f'{fmt(h["daily"])}% a day' if h['on'] else 'off') + '.'
    st = {'known': True, 'today': today, 'week': 51.0, 'five_hour': 4.0, 'over_daily': over_daily, 'over_five_hour': False,
          'over_budget': over_daily, 'over_hard': over_hard, 'hard_reason': 'daily' if over_hard else None, 'lifted': lifted,
          'blocking': over_hard and not lifted, 'summary': summary}
    return {'settings': s, 'status': st}


def budget_post(body):
    op = body.get('op')
    if op == 'lift':
        BUDGET['lifted_until'] = time.time() + 3 * 3600
    elif op == 'unlift':
        BUDGET['lifted_until'] = None
    elif op == 'set':
        for k in ('daily', 'five_hour'):
            if k in body:
                BUDGET[k] = body[k]
        for k, v in (body.get('hard') or {}).items():
            BUDGET['hard'][k] = v
    else:
        raise HttpError(400, 'Unknown op.')
    return dict(budget(), ok=True)


def library():
    d = copy.deepcopy(LIB)
    if LIB_MODE != 'auto':
        for t in d['tools']:
            t.pop('auto', None)
            t.pop('when', None)
    return d


# SB_FIXTURE_SENT=1: tasks that chats sent with run_on, in every state: working, stopped at a prompt, finished with
# the report read or not, and stopped without one. They carry `sent` as the server's Dispatch.mark writes it.
SENT = os.environ.get('SB_FIXTURE_SENT')


def sent_tasks(d):
    now = d['now']
    boss = next(s for s in d['machines'][0]['sessions'] if s.get('kind') == 'interactive')
    who = {'machine': d['machines'][0]['name'], 'label': d['machines'][0].get('label') or d['machines'][0]['name'],
           'sid': boss['sid'], 'title': boss['title']}
    n = [0]

    def task(title, status, state, ago, ctx=42000, **facts):
        n[0] += 1
        facts = {k: now - v if k.endswith('_at') else v for k, v in facts.items()}
        return {'sid': f'5e170000-0000-4000-8000-{n[0]:012d}', 'title': f'{title} (for {who["label"]})',
                'cwd': '/Users/you/Claude', 'folder': '', 'mtime': now - ago, 'active': now - ago, 'ctx': ctx,
                'model': 'claude-opus-5-5', 'cold': ago > 3600, 'cache_left': max(0, 3600 - ago), 'kind': 'background',
                'status': status, 'state': state, 'bg_id': f'5e17{n[0]:04d}', 'url': None, 'handoff': False,
                'held': False, 'sent': dict(who, id=f'd-{n[0]:012x}', **facts)}

    by = {m['name']: m for m in d['machines']}
    if 'tower' in by:
        by['tower']['sessions'][:0] = [
            task('Run the maps API test suite', 'busy', 'working', 40),
            task('Check the Windows build of the tile cache', 'stopped', 'done', 380, done_at=380, read_at=240, stopped_at=260),
            task('Profile the tile cache under load', 'stopped', 'done', 7400, ctx=88000, done_at=7400, stopped_at=7280),
            task('List the fonts the installer ships', 'stopped', None, 3000, ctx=0, failed=True, done_at=3000)]
    if 'laptop' in by:
        ask = task('Install the test tools for the parser', 'waiting', 'blocked', 540)
        ask['needs'] = 'Allow it to run npm install in parser/tools?'
        by['laptop']['sessions'][:0] = [
            ask,
            task('Review the parser diff for missed cases', 'busy', 'working', 15, ctx=61000),
            task('Summarise the course notes on recursion', 'idle', 'done', 90, done_at=80)]
    d['origins'] = True


def shifted():
    """The fixture as if it were fetched now: every timestamp moves by the time since the fixture was taken."""
    d = copy.deepcopy(FIX)
    dt = time.time() - d['now']
    d['now'] += dt
    for m in d['machines']:
        if m.get('updated'):
            m['updated'] += dt
        for s in m['sessions']:
            for k in ('mtime', 'active'):
                if s.get(k):
                    s[k] += dt
    sm = d['summary']
    for k in ('five_hour_reset', 'seven_day_reset'):
        if sm.get(k):
            sm[k] += dt
    sm['limits_updated'] = d['now'] - LIMITS_AGE
    if SENT:
        sent_tasks(d)
    for m in d['machines']:
        seen = compact_seen(m['name'])
        for s in m['sessions']:
            if seen is not None:  # where the chat compacts: the computer's window, or the model's own with none set
                s['limit'] = seen['value'] or 1_000_000
            if s.get('ctx'):
                ready = bool(s.get('handoff'))
                s.setdefault('handoff_fresh', ready)
                s['cost'] = sb.cost_model(s['ctx'], min(round(s['ctx'] * 0.38), 55_000), ready, 5_500 if ready else None)
    sm['note'] = notes.pick(d['machines'], sm, d['now'])
    # Memory per machine, and the cleanup card: open chats nobody used for an hour, and one stopped today.
    items, stopped = [], []
    for m in d['machines']:
        m['features'] = ['files', 'upload', 'flags', 'summary', 'rename', 'browse'] + (['places'] if PLACES and m['name'] in ROAM else [])
        if PLACES and m['name'] == 'studio' and len(m['sessions']) > 2:
            m['sessions'][1]['folder'], m['sessions'][2]['folder'] = '~/Documents/site', ICLOUD + '/Notes'
        alive = [s for s in m['sessions'] if s.get('status') == 'idle']
        m.setdefault('mem', {'claude_mb': 380.0 * max(1, len(alive)) + 410, 'procs': len(alive) + 1, 'orphans': []})
        for s in alive:
            if d['now'] - (s.get('active') or d['now']) > 3600 and len(items) < 6:
                items.append({'machine': m['name'], 'label': m.get('label'), 'sid': s['sid'], 'title': s.get('title'),
                              'reason': 'idle', 'why': 'not used for over an hour', 'mb': 380.0})
    if d['machines'] and d['machines'][0]['sessions']:
        s0 = d['machines'][0]['sessions'][-1]
        stopped.append({'machine': d['machines'][0]['name'], 'label': d['machines'][0].get('label'), 'sid': s0['sid'],
                        'title': s0.get('title'), 'at': d['now'] - 5400, 'mb': 412.0})
    d['cleanup'] = {'items': items, 'stopped': stopped}
    if ASKS:
        asks(d)
    if BUDGET_MODE:
        d['budget'] = budget()
    if LIB_MODE == 'auto':
        # One chat started with Auto, one with tools picked by hand.
        ss = [s for m in d['machines'] for s in m['sessions'] if s['sid'][:8] in ('eb945c42', 'f81f1ac9')]
        for s, mode, tools in zip(ss, ('auto', 'pick'), (['video-notes', 'price-check', 'web-fetch', 'home-assistant', 'personal-gdocs'], ['video-notes', 'web-fetch', 'api-hub'])):
            s['tools_mode'], s['tools'] = mode, tools
    d['v'] = 1
    return d


# SB_FIXTURE_ASKS=1: waiting chats with one `ask` of each kind (docs/specs/2026-10-13-needs-shaped-replies.md), with
# the long strings the no-collision rule is checked against: a 300-character need, a 120-character chat name, and
# 5 questions with 5 long options each. SB_FIXTURE_ASKS=long puts the long ones first.
ASKS = os.environ.get('SB_FIXTURE_ASKS') or ''
LONG_NEED = ('Answer five questions about the garden plan before I order: whether you already have raised beds, which tools '
             'are in the shed, whether seedlings bought the same week are fine to plant out, how many hours of sun the back '
             'fence gets and whether any of it is shaded, and when you want the soil delivered so it all arrives before Saturday.')[:300]
LONG_NAME = ('Garden plan for the spring: bed layout, seed order, watering schedule, the compost bins and what can be planted '
             'the week before')[:120]
LONG_OPT = ['Yes, every single one of them, and some extra', 'No, none at all, please plan around that',
            'Only some, I will say which in the box below', 'Not sure yet, ask me again on Thursday morning',
            'Something else entirely that I will type out']
FIXTURE_ASKS = [
    {'kind': 'choose', 'summary': 'Pick option 1 (a new Builder window, which Claude recommends) or option 2, and check four assumptions.',
     'options': [{'label': 'Option 1: new Builder window', 'reply': 'Option 1.', 'recommended': True},
                 {'label': 'Option 2: grow the Vibes editor', 'reply': 'Option 2.'}],
     'assumptions': ['OR within a field, AND across fields', 'Songs added by search have no tags', 'Saving again only adds songs',
                     'Show a song count, not total time'], 'source': 'haiku'},
    {'kind': 'questions', 'summary': LONG_NEED, 'source': 'question_tool',
     'questions': [{'q': 'Do you already have raised beds, and if so how many and how big are they?', 'options': LONG_OPT, 'about_you': True},
                   {'q': 'What tools do you have in the shed at the house right now?', 'options': LONG_OPT, 'multi': True, 'about_you': True},
                   {'q': 'Are seedlings bought the same week fine to plant out that weekend?', 'options': LONG_OPT, 'recommended': 0},
                   {'q': 'How many hours of sun does the back fence get?', 'options': LONG_OPT},
                   {'q': 'When should the soil be delivered so everything arrives in time?', 'options': LONG_OPT, 'multi': True}]},
    {'kind': 'task', 'summary': 'Load root.pem onto the door sensor over USB, then confirm.', 'done_reply': 'root.pem is on the door sensor.',
     'steps': ['Plug the sensor in over micro-USB', 'Open the vendor configurator', 'Security > Certificates > Upload',
               'Pick ~/Desktop/Claude/door-sensor/tls/root.pem (keep the name)', 'Change nothing else, then unplug']},
    {'kind': 'fix', 'summary': 'Claude in Chrome never connected, so it cannot drive the browser.',
     'fix': {'action': 'relaunch_chrome', 'label': 'Restart this chat with Chrome', 'alt_reply': 'Chrome is open and signed in now. Try again.'}},
    {'kind': 'open', 'summary': LONG_NEED},
]


def asks(d):
    """Makes the first chats on the board wait, one per ask kind; the second one has the long name."""
    order = FIXTURE_ASKS if ASKS != 'long' else [FIXTURE_ASKS[1], FIXTURE_ASKS[4]] + [FIXTURE_ASKS[i] for i in (0, 2, 3)]
    rows = [s for m in d['machines'] for s in m['sessions'] if s.get('kind') == 'background' or s.get('status') in ('waiting', 'idle', 'busy')]
    for i, (s, a) in enumerate(zip(rows, order)):
        s['status'], s['state'] = 'waiting', 'blocked'
        s['stale'] = False
        s['held'] = False
        s['active'] = s['mtime'] = d['now'] - (2 * 86400 + 600 if i == 3 else 600 + 300 * i)
        s['needs'] = a['summary']
        s['ask'] = dict(a, **{'for': s['active']})
        if i in (0, 3):
            s['star'] = True  # starred: a star on the card and the tab, the tab at the left
        if a['summary'] == LONG_NEED:
            s['title'] = LONG_NAME


def usage():
    """This week by day (local days since the reset, made up but adding up to the week's percent) and 4 past weeks."""
    import datetime
    sm = shifted()['summary']
    now, reset, pct = time.time(), sm.get('seven_day_reset') or time.time() + 3 * 86400, sm.get('seven_day') or 0
    while reset < now:
        reset += 7 * 86400
    start = datetime.date.fromtimestamp(reset - 7 * 86400)
    today = datetime.date.fromtimestamp(now)
    days = [start + datetime.timedelta(n) for n in range((today - start).days + 1)]
    shape = [6, 9, 3, 12, 8, 11, 4][:len(days)]
    used = [round(pct * x / sum(shape), 1) for x in shape]
    out = {'ok': True, 'now': now, 'week': {'pct': pct, 'reset': reset, 'pace': min(100, pct * 1.3)},
           'five': {'pct': sm.get('five_hour'), 'reset': sm.get('five_hour_reset')},
           'days': [{'day': d.isoformat(), 'used': u} for d, u in zip(days, used)],
           'weeks': [{'reset': reset - 7 * 86400 * k, 'pct': x} for k, x in ((4, 62), (3, 88), (2, 71), (1, 97))]}
    if USAGE_MODE != 'old':
        out.update(account_usage(now, reset, pct, sm))
        out['top'] = top_chats(reset - 7 * 86400)
    return out


# SB_FIXTURE_TOP: "Most used this week" on the usage panel. Unset: the fixture's own chats, biggest first (one a
# thread of three chats, one sent by Claude when SB_FIXTURE_SENT is set, one that no machine lists any more) and
# more chats than are shown. `long`: the first row has a very long name and a long machine name. `none`: nobody
# has used anything yet this week. `old`: no machine's helper can count. `reading`: the helpers are still reading
# their transcripts. An old server (SB_FIXTURE_USAGE=old) has no `top` at all.
TOP_MODE = os.environ.get('SB_FIXTURE_TOP') or ''


def top_chats(since):
    """What server/fleet.py top_chats answers under GET /api/usage `top`."""
    d = shifted()
    n = len(d['machines'])
    out = {'since': since, 'total': 0, 'chats': 0, 'rows': [], 'able': n, 'machines': n, 'pending': 0}
    if TOP_MODE in ('none', 'old', 'reading'):
        return dict(out, able=0 if TOP_MODE == 'old' else n, pending=n if TOP_MODE == 'reading' else 0)
    listed = [(m, x) for m in d['machines'] for x in m['sessions'] if not x.get('stale')]
    sent = [mx for mx in listed if mx[1].get('sent')][:1]
    seen, plain = set(), []
    for mx in listed:
        if not mx[1].get('sent') and mx[1].get('title') not in seen:
            seen.add(mx[1].get('title'))
            plain.append(mx)
    picks = plain[:5] + sent
    weights = [61_400_000, 34_900_000, 21_300_000, 9_800_000, 2_400_000, 14_600_000]
    rows = []
    for i, ((m, x), w) in enumerate(zip(picks, weights)):
        rows.append({'machine': m['name'], 'sid': x['sid'], 'name': x.get('title') or '', 'label': m.get('label') or m['name'],
                     'weighted': w, 'alive': True, 'sent': bool(x.get('sent')), 'parts': 3 if i == 1 else 1})
    if d['machines']:
        m = d['machines'][-1]  # a chat that was stopped and cleared mid-week: named, with nothing to open
        rows.append({'machine': m['name'], 'sid': '90ee0000-0000-4000-8000-000000000001', 'name': 'Sort the photo library by year',
                     'label': m.get('label') or m['name'], 'weighted': 17_200_000, 'alive': False, 'sent': False, 'parts': 1})
    if TOP_MODE == 'long' and rows:
        rows[0].update(name=LONG_NAME, label='Workshop computer in the back room', parts=4)
    rows.sort(key=lambda r: -r['weighted'])
    total = sum(r['weighted'] for r in rows) + 11_900_000  # 23 small chats that are not listed
    for r in rows:
        r['share'] = round(100.0 * r['weighted'] / total, 1)
    return dict(out, total=total, chats=len(rows) + 23, rows=rows[:10])


# SB_FIXTURE_USAGE: the account's own usage report on the usage panel. Unset: a full one (every limit, extra usage
# on, the week by product). `reached`: extra usage at its monthly limit. `failed`: the last refresh failed and the
# report is hours old. `none`: no machine's helper can report one. `old`: a server from before the report.
USAGE_MODE = os.environ.get('SB_FIXTURE_USAGE') or ''


def account_usage(now, reset, pct, sm):
    """What server/fleet.py adds to /api/usage: when the numbers were seen, the report, and who could give one."""
    if USAGE_MODE == 'none':
        return {'updated': now - 240, 'account': None, 'account_support': {'able': 0, 'online': 0, 'machines': 3}}
    usd = lambda minor: {'minor': minor, 'currency': 'USD', 'exp': 2}
    reached, failed = USAGE_MODE == 'reached', USAGE_MODE == 'failed'
    at = now - (5 * 3600 if failed else 40)
    limits = []
    if sm.get('five_hour') is not None:
        limits.append({'kind': 'session', 'pct': sm['five_hour'], 'reset': sm.get('five_hour_reset'), 'label': None,
                       'severity': 'normal', 'active': False})
    limits += [{'kind': 'weekly_all', 'pct': pct, 'reset': reset, 'label': None, 'severity': 'normal', 'active': True},
               {'kind': 'weekly_scoped', 'pct': 18, 'reset': reset, 'label': 'Fable', 'severity': 'normal', 'active': False},
               {'kind': 'weekly_scoped', 'pct': 77, 'reset': reset, 'label': 'Sonnet', 'severity': 'warning', 'active': False}]
    extra = {'enabled': not reached, 'used': usd(7725 if reached else 1234), 'limit': usd(7500), 'balance': None if reached else usd(2500),
             'pct': 100 if reached else 16, 'reason': 'org_level_disabled_until' if reached else None, 'limit_reached': reached,
             'auto_reload': None}
    products = {'as_of': at - 60, 'start': reset - 7 * 86400,
                'rows': [{'key': 'claude_code', 'name': 'Claude Code', 'pct': 81, 'of_limit': round(pct * 0.81)},
                         {'key': 'chat', 'name': 'Chats', 'pct': 12, 'of_limit': round(pct * 0.12)},
                         {'key': 'cowork', 'name': 'Cowork', 'pct': 4, 'of_limit': round(pct * 0.04)},
                         {'key': 'other', 'name': 'Other', 'pct': 3, 'of_limit': round(pct * 0.03)}]}
    account = {'at': at, 'tried': now - 300 if failed else at, 'error': 'Claude Code took too long to answer' if failed else None,
               'machine': 'studio', 'plan': 'max', 'available': True, 'limits': limits, 'extra': extra, 'products': products}
    return {'updated': at, 'account': account, 'account_support': {'able': 2, 'online': 2, 'machines': 3}}


def session_of(machine, sid):
    for m in FIX['machines']:
        if m['name'] == machine:
            for x in m['sessions']:
                if x['sid'] == sid:
                    return m, x
    return None, None


def flags_view(machine, sid):
    m, x = session_of(machine, sid)
    active = [{'name': '--model', 'value': 'opus', 'label': 'Model', 'text': 'Opus', 'how': 'live'},
              {'name': '--effort', 'value': 'high', 'label': 'Effort', 'text': 'High', 'how': 'live'},
              {'name': '--permission-mode', 'value': 'default', 'label': 'Permissions', 'text': 'Asks first', 'how': 'relaunch'},
              {'name': '--add-dir', 'value': 'notes', 'label': 'Also open', 'text': 'notes', 'how': 'live'},
              {'name': '--remote-control', 'value': None, 'label': 'Remote Control', 'text': 'On', 'how': 'locked'}]
    return {'ok': True, 'active': active, 'other': [], 'summary': 'Opus · High · Asks first · 1 more', 'short': 'Opus · High',
            'complete': True, 'kind': (x or {}).get('kind'), 'status': (x or {}).get('status'),
            'running': (x or {}).get('kind') == 'background'}


WORD = {'opus': 'Opus', 'sonnet': 'Sonnet', 'haiku': 'Haiku', 'fable': 'Fable', 'low': 'Low', 'medium': 'Medium', 'high': 'High',
        'xhigh': 'Extra high', 'max': 'Max', 'ultracode': 'UltraCode', 'default': 'Asks first', 'acceptEdits': 'Accept edits',
        'plan': 'Plan', 'auto': 'Auto', 'bypassPermissions': 'Bypass', 'dontAsk': 'Never asks'}


def flags_plan(body):
    w = body.get('want') or {}
    ch = []
    for k, cur, name, label in (('model', 'opus', '--model', 'Model'), ('effort', 'high', '--effort', 'Effort'), ('perm', 'default', '--permission-mode', 'Permissions')):
        if w.get(k) and w[k] != cur:
            live = k != 'perm' or w[k] == 'plan'
            ch.append({'name': name, 'label': label, 'from': WORD[cur], 'to': WORD.get(w[k], w[k]), 'how': 'live' if live else 'relaunch',
                       'cmd': f'/{"model" if k == "model" else "effort" if k == "effort" else "plan"} {w[k]}' if live else None})
    if w.get('chrome'):
        ch.append({'name': '--chrome', 'label': 'Claude in Chrome', 'from': 'Off', 'to': 'On', 'how': 'relaunch'})
    relaunch = any(c['how'] == 'relaunch' for c in ch)
    return {'ok': True, 'changes': ch, 'cmds': [] if relaunch else [c['cmd'] for c in ch if c.get('cmd')], 'relaunch': relaunch,
            'running': True, 'busy': False, 'dropped': [], 'body': {'model': w.get('model'), 'effort': w.get('effort'), 'flags': []}}


def browse(q):
    d = shifted()
    rows = []
    for m in d['machines']:
        for x in m['sessions']:
            state = 'live' if x.get('status') in ('busy', 'waiting') else 'quiet' if x.get('status') == 'idle' else 'stopped'
            rows.append({'machine': m['name'], 'label': m.get('label'), 'sid': x['sid'], 'title': x.get('title'), 'folder': x.get('folder') or '',
                         'model': x.get('model'), 'ctx': x.get('ctx'), 'size': 180000, 'last': x.get('active') or x.get('mtime'),
                         'started': (x.get('active') or 0) - 7200, 'state': state, 'url': x.get('url'),
                         'first_prompt': 'Look at how this works and tell me what to change first.', 'history': [], 'history_count': 0,
                         **({'sent': x['sent']} if x.get('sent') else {})})
    one = lambda k: (q.get(k) or [''])[0]
    if one('machine'):
        rows = [r for r in rows if r['machine'] == one('machine')]
    if one('state'):
        rows = [r for r in rows if r['state'] == one('state')]
    if one('by'):
        rows = [r for r in rows if (one('by') == 'claude') == bool(r.get('sent'))]
    if one('folder'):
        rows = [r for r in rows if (r['folder'] or '').lower().startswith(one('folder').lower())]
    if one('q'):
        rows = [r for r in rows if one('q').lower() in (r['title'] or '').lower()]
    rows.sort(key=lambda r: -(r['last'] or 0))
    for i, r in enumerate(rows):  # a summary is written for every third chat
        r['handoff'] = i % 3 == 1
        r['last_prompt'] = 'Make the header calmer, and line it up with the footer.'
    if one('thread') == '1':  # one row per thread: chats with the same topic, the newest first
        groups, order = {}, []
        for r in rows:
            g = (r['title'] or '').split(' (')[0][:22].lower()
            if g not in groups:
                groups[g] = []
                order.append(g)
            groups[g].append(r)
        rows = [dict(groups[g][0], history=groups[g][1:], history_count=len(groups[g]) - 1) for g in order]
    folders = sorted({r['folder'] for r in rows if r['folder']})
    return {'ok': True, 'total': len(rows), 'rows': rows, 'facets': {'machines': [{'name': m['name'], 'label': m.get('label')} for m in d['machines']],
                                                                     'folders': folders, 'states': {}, **({'sent': sum(1 for r in rows if r.get('sent'))} if SENT else {})}}


# Settings: each machine's Claude folder, versions, capabilities and MCP servers, changed in memory.
# SB_FIXTURE_NEEDS_ROOT=<name> leaves one machine without a picked folder, to see Settings ask for it.
NEEDS_ROOT = os.environ.get('SB_FIXTURE_NEEDS_ROOT') or ''
EDITS = {'order': None, 'labels': {}, 'roots': {}, 'caps': {}}
TOOLS = {'mac': [('xcode', 'Xcode', '26.3'), ('ios_sim', 'iOS Simulator', None), ('docker', 'Docker', '29'), ('node', 'Node', '22'),
                 ('python', 'Python', '3.13'), ('rust', 'Rust', '1.90'), ('tailscale', 'Tailscale', None)],
         'windows': [('android_sdk', 'Android SDK', '37'), ('cuda', 'CUDA', '13.2 on the RTX GPU'), ('docker', 'Docker', '29'),
                     ('msvc', 'Visual Studio', '2022'), ('dotnet', '.NET', '9.0'), ('node', 'Node', '22'), ('python', 'Python', '3.12')]}
MCP = [{'name': 'github', 'scope': 'user', 'type': 'stdio', 'command': 'npx', 'args': ['-y', '@modelcontextprotocol/server-github'],
        'env': ['GITHUB_PERSONAL_ACCESS_TOKEN'], 'headers': [], 'status': None},
       {'name': 'linear', 'scope': 'user', 'type': 'http', 'url': 'https://mcp.linear.app/sse', 'args': [], 'env': [],
        'headers': ['Authorization'], 'status': None},
       {'name': 'postgres', 'scope': 'project', 'type': 'stdio', 'command': 'uvx', 'args': ['mcp-server-postgres', '--password=•••', 'postgres://db.local:5432/app'],
        'env': [], 'headers': [], 'status': None, 'folder': ''},
       {'name': 'playwright', 'scope': 'plugin', 'plugin': 'browser-tools', 'type': 'stdio', 'command': 'npx',
        'args': ['@playwright/mcp@latest'], 'env': [], 'headers': [], 'status': None}]
DIRS = {'': ['Applications', 'Desktop', 'Documents', 'Downloads', 'Library', 'Movies', 'Music', 'Pictures', 'Public'],
        'Desktop': ['Claude', 'Screenshots'], 'Desktop/Claude': ['switchboard', 'course-notes', 'parser'],
        'Documents': ['code', 'Taxes']}


# SB_FIXTURE_PLACES=1: places beyond the Claude folder (docs: Places). Studio roams its home (Desktop closed by
# macOS privacy, a missing added place, a folder said no to), Laptop keeps Anywhere in home off with one added place,
# Tower's agent is too old for places. New and Continue answer 409 need "trust" for a folder Claude was never let into.
PLACES = bool(os.environ.get('SB_FIXTURE_PLACES'))
ROAM = {'studio': True, 'laptop': False}
ICLOUD = '~/Library/Mobile Documents/com~apple~CloudDocs'
TRUST = {'~/Documents/site': 'yes', '@Data/work': 'no', '~/Documents/Papers': 'no'}
H_ = 3600


def _f(name, size, ago, open_=True):
    return {'name': name, 'size': size, 'mtime': time.time() - ago, 'open': open_}


FS = {
    'studio': {
        '~': (['Applications', 'Desktop', 'Developer', 'Documents', 'Downloads', 'Movies', 'Music', 'Pictures'],
              [_f('todo.txt', 812, 2 * H_)]),
        '~/Documents': (['Papers', 'Recipes', 'site', 'Taxes 2026'],
                        [_f('budget.xlsx', 48_200, 30 * H_), _f('notes.md', 3_900, 3 * H_), _f('resume.pdf', 182_000, 400 * H_)]),
        '~/Documents/site': (['public', 'src'], [_f('README.md', 2_400, 1.5 * H_), _f('index.html', 6_100, 20 * H_),
                                                 _f('logo.png', 38_000, 300 * H_), _f('package.json', 1_100, 20 * H_)]),
        '~/Documents/Papers': (['drafts'], [_f('review.md', 9_800, 50 * H_)]),
        '~/Downloads': ([], [_f('build.tar.gz', 48_000_000, 5 * H_), _f('invoice-0921.pdf', 92_000, 70 * H_),
                             _f('photo.heic', 2_800_000, 26 * H_), _f('talk.mov', 1_240_000_000, 300 * H_)]),
        '~/Developer': (['atlas', 'maps-api', 'parser'], []),
        ICLOUD: (['Notes', 'Shared'], [_f('packing list.md', 1_300, 100 * H_)]),
        ICLOUD + '/Notes': ([], [_f('garden.md', 2_200, 9 * H_), _f('ideas.md', 4_400, 50 * H_)]),
        '@Data': (['exports', 'work'], [_f('dump.csv', 12_400_000, 8 * H_)]),
        '@Data/work': ([], [_f('q3.sqlite', 220_000_000, 4 * H_), _f('schema.sql', 6_000, 4 * H_)]),
    },
    'laptop': {
        '@Courses': (['Physics 101', 'Music 210'], [_f('syllabus.pdf', 220_000, 900 * H_)]),
    },
}


def places_view(name):
    if name == 'tower' or name not in ROAM:
        return {'ok': True, 'roam': False, 'old': True, 'places': [{'kind': 'claude', 'name': 'Claude folder', 'folder': '',
                                                                    'shown': '~/Claude', 'state': 'ok'}]}
    pl = lambda kind, n, folder, state='ok': {'kind': kind, 'name': n, 'folder': folder, 'shown': folder, 'state': state}
    out = [pl('claude', 'Claude folder', '')]
    out[0]['shown'] = '~/Desktop/Claude'
    if ROAM[name] and name == 'studio':
        out += [pl('home', 'Home', '~'), pl('desktop', 'Desktop', '~/Desktop', 'privacy'), pl('documents', 'Documents', '~/Documents'),
                pl('downloads', 'Downloads', '~/Downloads'), pl('code', 'Developer', '~/Developer'), pl('icloud', 'iCloud Drive', ICLOUD)]
    elif ROAM[name]:
        out += [pl('home', 'Home', '~'), pl('documents', 'Documents', '~/Documents'), pl('downloads', 'Downloads', '~/Downloads')]
    if name == 'studio':
        out += [pl('place', 'Data', '@Data'), pl('place', 'Archive', '@Archive', 'missing')]
    else:
        out += [pl('place', 'Courses', '@Courses')]
    return {'ok': True, 'roam': ROAM[name], 'places': out, 'os': 'mac'}


def outside(folder):
    return folder == '~' or folder.startswith('~/') or folder.startswith('@')


def trust_of(folder):
    return TRUST.get(folder, 'ask') if outside(folder) else 'yes'


def shown_of(folder):
    return folder or '~/Desktop/Claude'


def files_view(name, folder, file=None):
    """GET /api/files with places: the listing, or (code, error) the way the agent words it."""
    if outside(folder) and folder[0] == '~' and not ROAM.get(name):
        return 400, 'This machine keeps chats in its Claude folder. Settings can let them start anywhere in home.'
    if folder.startswith('@Archive'):
        return 400, 'folder does not exist'
    base = {'ok': True, 'root': '/Users/you/Desktop/Claude', 'folder': folder, 'shown': shown_of(folder), 'trust': trust_of(folder), 'more': False}
    if folder == '~/Desktop':
        return dict(base, dirs=[], files=[], blocked='privacy',
                    hint='macOS permission not granted for ~/Desktop. On this Mac, open System Settings, Privacy & Security, Files and Folders, and turn it on for the app that runs Claude Code.')
    if file:
        return dict(base, dirs=[], files=[], file={'name': file, 'size': 2048, 'mtime': time.time() - 600,
                                                   'text': '# ' + file + '\n\nThe header and the footer share one grid.\n',
                                                   'binary': False, 'secret': False, 'cut': False})
    dirs, files = FS.get(name, {}).get(folder, ([], [_f('README.md', 1_200, 6 * H_)]))
    rel = lambda d: f'{folder}/{d}' if folder else d
    return dict(base, dirs=[{'folder': rel(d), 'name': d} for d in dirs], files=files)


def caps_view(m):
    st = EDITS['caps'].get(m['name']) or {'on': m['name'] != 'laptop', 'off': [], 'extra': ['Can print shipping labels'] if m.get('os') == 'windows' else []}
    tools = [{'key': k, 'name': n, 'detail': d, 'on': k not in st['off']} for k, n, d in TOOLS.get(m.get('os'), TOOLS['mac'])]
    on = [t['name'] for t in tools if t['on'] and t['key'] in ('xcode', 'android_sdk', 'cuda', 'docker', 'msvc')]
    return {'ok': True, 'on': st['on'], 'off': st['off'], 'extra': st['extra'], 'tools': tools, 'measured': time.time() - 600,
            'error': None, 'can': ', '.join(on + st['extra'][:3]) or 'General work'}


def machines():
    out = []
    byname = {m['name']: m for m in FIX['machines']}
    for name in EDITS['order'] or [m['name'] for m in FIX['machines']]:
        m = byname[name]
        root = EDITS['roots'].get(name, None if name == NEEDS_ROOT else m.get('root'))
        win = m.get('os') == 'windows'
        out.append({'name': name, 'label': EDITS['labels'].get(name) or m.get('label') or name, 'os': m.get('os') or '',
                    'form': None, 'icon': None, 'via': 'link', 'target': None, 'root': root, 'online': m.get('online'),
                    'root_shown': root or ('C:\\Users\\you' if win else '/Users/you'), 'needs_root': not root,
                    'root_suggest': '~/Desktop/Claude' if m.get('os') == 'mac' else '~', 'updated': time.time() - 20,
                    'versions': {'agent': '4e73c65'[:7], 'claude': '2.1.296' if name != 'tower' else '2.1.290'},
                    'features': ['mcp', 'root_dirs', 'files'] + (['places'] if PLACES and name in ROAM else [])
                    + (['compact_at'] if compact_seen(name) is not None else []),
                    'compact': compact_seen(name),
                    'auto': caps_view(m)['on'], 'can': caps_view(m)['can']})
        if PLACES and name in ROAM:
            out[-1]['roam'] = ROAM[name]
    return {'ok': True, 'machines': out, 'max': 12}


def machines_post(body):
    op, name = body.get('op'), body.get('machine')
    if op == 'rename':
        EDITS['labels'][name] = body.get('label')
    elif op == 'root':
        EDITS['roots'][name] = body.get('root')
    elif op == 'roam':
        ROAM[name] = bool(body.get('roam'))
    elif op == 'order':
        EDITS['order'] = body.get('names')
    elif op == 'caps':
        m = next(x for x in FIX['machines'] if x['name'] == name)
        cur = caps_view(m)
        EDITS['caps'][name] = {k: body.get(k, cur[k]) for k in ('on', 'off', 'extra')}
    return machines()


def root_dirs(path):
    kids = DIRS.get(path, [])
    rel = lambda n: f'{path}/{n}' if path else n
    return {'ok': True, 'path': path, 'shown': '~/' + path if path else '~',
            'folders': [{'name': n, 'path': rel(n), 'kids': rel(n) in DIRS} for n in kids],
            'current': '~/Desktop/Claude', 'set': True, 'suggest': '~/Desktop/Claude'}


# Keys (devices that pair with the home computer): SB_FIXTURE_KEYS=both|ready|norelay|only|off answers the server's keys
# routes; unset, they are 404 as on a server from before keys. SB_FIXTURE_PAIR_AFTER=<seconds>: a new device joins
# that long after Add a device. SB_FIXTURE_PAIR_TTL=<seconds>: how long a code lasts (600).
# SB_FIXTURE_NODE=home|alone|member|relay|localnet|refused also answers the node's control API on this port (point the app at
# it with SWITCHBOARD_NODE_CONTROL=http://127.0.0.1:<port>); SB_FIXTURE_JOIN=ok|wrong_code|expired|unreachable|local_network|
# no_relay|link_no_relay is what POST /pair/join says (link_no_relay: until the join carries "addrs").
# SB_FIXTURE_SWITCH=ok|byhand|blocked|none is what POST /api/keys/mode says: it changes the mode ("restarting"), or the
# server cannot restart itself (409 with "command"), or keys only would cut something off (409 with "blockers"); none is
# a server from before the route (404, and no "switch" in the report). With SB_FIXTURE_KEYS=off the report says the app
# is on the home computer ("here"), so Devices shows Turn keys on.
KEYS_MODE = os.environ.get('SB_FIXTURE_KEYS') or ''
SWITCH = os.environ.get('SB_FIXTURE_SWITCH') or 'ok'
NODE_MODE = os.environ.get('SB_FIXTURE_NODE') or ''
JOIN_SAYS = os.environ.get('SB_FIXTURE_JOIN') or 'ok'
PAIR_AFTER = float(os.environ.get('SB_FIXTURE_PAIR_AFTER') or 0)
PAIR_TTL = float(os.environ.get('SB_FIXTURE_PAIR_TTL') or 600)
HOME_ID = '3f9a1c0e5b7d2a4c6e8f0a1b3c5d7e9f1a2b3c4d5e6f708192a3b4c5d6e7f809'
KEYS = {'removed': set(), 'offer_at': None, 'joined': False, 'mode': None, 'relay': None}


def keys_mode():
    """The mode in force: what SB_FIXTURE_KEYS said, until POST /api/keys/mode changed it."""
    return KEYS['mode'] or KEYS_MODE


def keys_devices():
    now = time.time()
    day = 86400
    out = [
        {'id': 'd-laptop', 'name': 'Laptop', 'kind': 'computer', 'machine': 'laptop', 'paired': now - 3 * day,
         'via': 'code', 'last_seen': now - 4, 'path': 'direct'},
        {'id': 'd-tower', 'name': 'Tower', 'kind': 'computer', 'machine': 'tower', 'paired': now - 12 * day,
         'via': 'upgrade', 'last_seen': now - 9, 'path': 'relay'},
        {'id': 'd-cabin', 'name': 'Cabin Mac mini', 'kind': 'computer', 'machine': None, 'paired': now - 40 * day,
         'via': 'link', 'last_seen': now - 2 * 3600, 'path': None},
        {'id': 'd-phone', 'name': 'iPhone', 'kind': 'phone', 'machine': None, 'paired': now - 600, 'via': 'code',
         'last_seen': None, 'path': None},
    ]
    if KEYS['offer_at'] and PAIR_AFTER and now - KEYS['offer_at'] >= PAIR_AFTER:
        out.append({'id': 'd-new', 'name': 'Workshop PC', 'kind': 'computer', 'machine': 'workshop',
                    'paired': KEYS['offer_at'] + PAIR_AFTER, 'via': 'code', 'last_seen': now - 1, 'path': 'direct'})
    return {'ok': True, 'devices': [d for d in out if d['id'] not in KEYS['removed']]}


def keys_readiness():
    now = time.time()
    was = keys_mode()
    mode = 'both' if was in ('ready', 'norelay') else was
    blockers = []
    if mode == 'off':
        blockers = [{'kind': 'mode', 'text': 'Keys are off.'}]
    if was == 'both' and KEYS['mode'] is None:
        blockers = [
            {'kind': 'machine', 'text': 'Tower has not paired yet.', 'machine': 'tower', 'label': 'Tower', 'paired': False},
            {'kind': 'direct', 'text': 'An iPhone connected by address.', 'client': 'iPhone', 'last': now - 1800},
            {'kind': 'direct', 'text': 'A browser connected by address.', 'client': 'web', 'last': now - 2 * 86400},
        ]
    machines = [
        {'name': 'studio', 'label': 'Studio', 'home': True, 'paired': True, 'seen': now - 2, 'path': None},
        {'name': 'laptop', 'label': 'Laptop', 'home': False, 'paired': True, 'seen': now - 4, 'path': 'direct'},
        {'name': 'tower', 'label': 'Tower', 'home': False, 'paired': KEYS_MODE != 'both', 'seen': now - 9,
         'path': None if KEYS_MODE == 'both' else 'relay'},
    ]
    out = {'ok': True, 'keys': mode, 'ready': mode == 'both' and not blockers, 'days': 7, 'blockers': blockers,
           'machines': machines, 'here': True}
    if SWITCH != 'none':
        out['switch'] = True
    return out


def keys_set_mode(body):
    """POST /api/keys/mode (server.py keys_mode): the answer and its status."""
    want = body.get('keys')
    if want not in ('off', 'both', 'only'):
        return {'ok': False, 'error': 'keys must be off, both or only'}, 400
    word = 'on' if want == 'both' else want
    if SWITCH == 'byhand':
        aim = {'both': 'To turn keys on', 'off': 'To turn keys off', 'only': 'To switch to keys only'}[want]
        command = f'python3 /srv/switchboard/server/keys.py {word}'  # server/keys.py by_hand, for a copy of the code
        return {'ok': False, 'command': command,
                'error': 'Nothing was changed. This server was started by hand from a copy of the code, so it cannot '
                         f'restart itself. {aim}, run this on the home computer, then start the server again: {command}'}, 409
    if SWITCH == 'blocked' and want == 'only':
        blockers = [{'kind': 'machine', 'text': 'Tower has not paired yet.', 'machine': 'tower', 'label': 'Tower'},
                    {'kind': 'direct', 'text': 'An iPhone connected by address 30 minutes ago.', 'client': 'iPhone'}]
        return {'ok': False, 'blockers': blockers,
                'error': 'Not yet: switching to keys only now would cut something off. ' + blockers[0]['text']}, 409
    same = keys_readiness()['keys'] == want
    KEYS['mode'] = want
    return {'ok': True, 'keys': want, 'restarting': not same}, 200


def keys_pair():
    KEYS['offer_at'] = time.time()
    if KEYS_MODE == 'norelay':
        return {'ok': True, 'code': None, 'expires': int(KEYS['offer_at'] + PAIR_TTL),
                'link': f'switchboard://pair/1?h={HOME_ID}&s=5d0c1b2a39485766&n=Studio'}
    return {'ok': True, 'code': 'K7PM-2XQ9', 'expires': int(KEYS['offer_at'] + PAIR_TTL),
            'link': f'switchboard://pair/1?h={HOME_ID}&r=https%3A%2F%2Frelay.example&s=5d0c1b2a39485766&n=Studio'}


def node_status():
    """The node's GET /status (sb-link control.rs `Status`). None: keys are not running here (503)."""
    relay = {'url': 'https://relay.example', 'source': 'home', 'connected': True, 'same_id': False}
    base = {'ok': True, 'version': '0.8.23', 'id': 'd-laptop', 'name': 'Laptop', 'machine': 'laptop', 'relay': relay,
            'forwarders': [], 'devices': 0, 'updates': [], 'problems': []}
    if NODE_MODE == 'home':
        if keys_mode() == 'off':
            return None
        mine = {'url': 'https://relay.example', 'source': 'built in', 'connected': True, 'same_id': False}
        if KEYS_MODE == 'norelay':
            mine = {'url': None, 'source': 'none', 'connected': False, 'same_id': False}
        if KEYS['relay'] is not None:
            mine = {'url': KEYS['relay'] or None, 'source': 'settings' if KEYS['relay'] else 'none',
                    'connected': bool(KEYS['relay']), 'same_id': False}
        return dict(base, role='home', id=HOME_ID, name='Studio', machine='studio', devices=4, relay=mine,
                    addrs=['192.0.2.20:41641', '[fe80::1c2d:3e4f]:41641'])
    if not (NODE_MODE in ('member', 'relay', 'localnet', 'refused') or KEYS['joined']):
        return None
    home = {'id': HOME_ID, 'name': 'Studio', 'connected': True, 'path': 'relay' if NODE_MODE == 'relay' else 'direct',
            'rtt_ms': 18, 'refused': False}
    # The port that leads to the home computer's server is this fixture's own, so a join made here ends on this
    # fixture's board and never on whatever answers on 8130.
    out = dict(base, role='member', home=home,
               forwarders=[{'target': 'api', 'addr': f"127.0.0.1:{KEYS.get('port') or 8139}", 'ok': True},
                           {'target': 'hub', 'addr': '127.0.0.1:8132', 'ok': True}])
    if NODE_MODE == 'localnet':
        words = ('This Mac is not letting switchboard-node use the local network. Turn it on in System Settings, '
                 'Privacy & Security, Local Network.')
        home.update(connected=False, path=None, rtt_ms=None, error='No route to host (os error 65)')
        out['relay'] = dict(relay, connected=False, error='No route to host (os error 65)')
        out['problems'] = [words]
    if NODE_MODE == 'refused':
        home.update(connected=False, path=None, rtt_ms=None, refused=True)
        out['problems'] = ['The home computer no longer lists this computer. Pair it again: switchboard-node pair <code or link>']
    return out


# The home computer is away (keys phase 4a): SB_FIXTURE_AWAY=two|one|three|none|asking|update|stale makes this
# fixture answer the way the helper's stand-in does in its place (sb-link standin.rs): /api/sessions and
# /api/summary carry "away", the home computer's row is empty with a reason, and everything that needs the home
# computer is 503 with kind home_away. With SB_FIXTURE_NODE=member the node's GET /roster and POST
# /devices/block and /devices/unblock answer too (SB_FIXTURE_ROSTER=none|stale: no list, or one past its 7 days).
AWAY = os.environ.get('SB_FIXTURE_AWAY') or ''
ROSTER_MODE = os.environ.get('SB_FIXTURE_ROSTER') or ''
HOME_AWAY = 'The home computer is away. This needs the home computer.'
LACKS = ['new_chat', 'move', 'history', 'files', 'usage', 'library', 'settings', 'notifications']
LACKS_LINE = ('Not available while the home computer is away: new chats, moving a chat, history, files, usage, '
              'library, settings, notifications.')
BLOCKED = {'d-cabin'}


def listed(names):
    return names[0] if len(names) == 1 else ', '.join(names[:-1]) + ' and ' + names[-1]


def away_board(d):
    """The board as the stand-in gives it: the computers that answered, the others empty with why."""
    now = d['now']
    mini = {'name': 'mini', 'label': 'Mini', 'os': 'macos', 'online': True, 'sessions': [], 'replaced': []}
    rows = {m['name']: m for m in d['machines']}
    rows['mini'] = mini
    showing = {'two': ['laptop', 'tower'], 'one': ['laptop'], 'three': ['laptop', 'tower', 'mini'],
               'update': ['laptop']}.get(AWAY, [])
    order = ['studio', 'laptop', 'tower'] + (['mini'] if AWAY in ('three', 'update') else [])
    why = {'studio': 'is_home'}
    for name in order[1:]:
        if name not in showing:
            why[name] = 'needs_update' if (AWAY == 'update' and name == 'mini') else (
                'refused' if AWAY == 'stale' else 'unreachable')
    says = {'is_home': 'The home computer is away.', 'unreachable': '{} cannot be reached just now.',
            'needs_update': '{} needs an update before it can answer while the home computer is away.',
            'refused': '{} does not answer this device.'}
    machines, missing = [], []
    for name in order:
        m = rows[name]
        if name in why:
            missing.append({'machine': name, 'label': m['label'], 'why': why[name]})
            machines.append({'name': name, 'label': m['label'], 'os': m.get('os'), 'online': False, 'sessions': [],
                             'replaced': [], 'error': says[why[name]].format(m['label']), 'why': why[name]})
        else:
            machines.append(dict(m, online=True))
    labels = [rows[n]['label'] for n in showing]
    line = 'Home is away. Showing chats from %s.' % listed(labels) if labels else (
        'Home is away. No other computer can be reached.')
    if AWAY == 'asking':
        line = 'Home is away. Asking the other computers for their chats.'
    if AWAY == 'stale':
        line = ('What this device knows about your computers is 7 days old, so they no longer answer it. Start the '
                'home computer, or replace it.')
    at = now - (8 * 86400 if AWAY == 'stale' else 2 * 3600)
    away = {'since': now - 300, 'home': {'id': HOME_ID, 'name': 'Studio', 'last_seen': now - 320},
            'showing': [{'machine': n, 'label': rows[n]['label']} for n in showing], 'missing': missing,
            'roster': {'at': at, 'expires': at + 7 * 86400}, 'line': line,
            'notes': [says['needs_update'].format(m['label']) for m in missing if m['why'] == 'needs_update'],
            'lacks': LACKS, 'lacks_line': LACKS_LINE}
    chats = [s for m in machines for s in m['sessions'] if not s.get('stale')]
    summary = {'five_hour': None, 'seven_day': None, 'five_hour_reset': None, 'seven_day_reset': None,
               'limits_updated': None, 'week_projected': None, 'sessions': len(chats),
               'running': sum(1 for s in chats if s.get('activity') == 'working'), 'cold': 0,
               'online': len(showing), 'machines': len(machines), 'away': away}
    return {'machines': machines, 'now': now, 'summary': summary, 'v': int(now) // 5, 'prefs': {}, 'away': away}


def node_roster():
    """The node's GET /roster on a computer that joined a home."""
    if ROSTER_MODE == 'none':
        return {'ok': True, 'roster': None, 'devices': []}
    now = time.time()
    at = now - (8 * 86400 if ROSTER_MODE == 'stale' else 2 * 3600)
    row = lambda i, name, kind, machine, **k: dict(  # noqa: E731
        {'id': i, 'name': name, 'kind': kind, 'machine': machine, 'ver': '0.8.23', 'home': False, 'this': False,
         'blocked': i in BLOCKED}, **k)
    devices = [row(HOME_ID, 'Studio', 'computer', 'studio', home=True),
               row('d-laptop', 'Laptop', 'computer', 'laptop', this=True),
               row('d-tower', 'Tower', 'computer', 'tower'),
               row('d-cabin', 'Cabin Mac mini', 'computer', None),
               row('d-phone', 'iPhone', 'phone', None, ver=None)]
    return {'ok': True, 'roster': {'epoch': 1, 'seq': 7, 'at': at, 'expires': at + 7 * 86400}, 'devices': devices}


JOIN_ERRORS = {  # the node's own sentences (sb-link pairing.rs, sb-node keys.rs), all 409
    'wrong_code': 'That code is wrong. Check it and try again.',
    'expired': 'That code has expired or was already used. Make a new one.',
    'unreachable': 'Cannot reach the home computer (timed out).',
    'local_network': 'This Mac is not letting switchboard-node use the local network. Turn it on in System Settings, '
                     'Privacy & Security, Local Network.',
    'no_relay': 'A short code finds the home computer through a relay, and none is set here. Use the link instead, or '
                'set the relay first: switchboard-node relay <url>',
    'link_no_relay': "That link names no relay, so this computer needs the home computer's address: add --addr <ip:port> "
                     "(the home computer's status shows it).",
}


class H(BaseHTTPRequestHandler):
    def log_message(self, fmt, *args):
        pass

    def _json(self, data, code=200):
        raw = json.dumps(data).encode()
        self.send_response(code)
        self.send_header('Content-Type', 'application/json')
        self.send_header('Content-Length', str(len(raw)))
        # Each answer ends its connection (HTTP/1.0), and says so: a client that kept it for the next request would
        # lose a POST sent right after a GET.
        self.send_header('Connection', 'close')
        self.end_headers()
        self.wfile.write(raw)

    def do_GET(self):
        u = urlsplit(self.path)
        p, q = u.path, parse_qs(u.query)
        if AWAY and p in ('/api/sessions', '/api/summary'):
            d = away_board(shifted())
            return self._json(d if p == '/api/sessions' else d['summary'])
        if NODE_MODE and p == '/roster':
            return self._json(node_roster())
        if AWAY and p.startswith('/api/'):
            return self._json({'ok': False, 'kind': 'home_away', 'error': HOME_AWAY}, 503)
        if p == '/api/sessions':
            d = shifted()
            GROUPS.decorate(d)
            return self._json(d)
        if p == '/api/groups':
            return self._json({'ok': True, 'groups': GROUPS.all()})
        if p == '/api/usage':
            return self._json(usage())
        if p == '/api/budget' and BUDGET_MODE:
            return self._json(dict(budget(), ok=True, events=[]))
        if p == '/api/library' and LIB_MODE:
            return self._json(library())
        if p == '/api/summary':
            return self._json(shifted()['summary'])
        if p == '/api/models':
            ms = [dict(m, models={'aliases': ['fable', 'opus', 'sonnet'], 'allowed': None, 'catalog': CATALOG})
                  for m in shifted().get('machines') or []]
            return self._json(models_mod.view(ms))
        if p == '/api/prefs':
            return self._json(prefs_view())
        if PLACES and p.startswith('/api/places/'):
            return self._json(places_view(p.split('/')[3]))
        if PLACES and p.startswith('/api/folder_check/'):
            f = (q.get('folder') or [''])[0]
            return self._json({'ok': True, 'folder': f, 'shown': shown_of(f), 'trust': trust_of(f)})
        if PLACES and p.startswith('/api/files/'):
            r = files_view(p.split('/')[3], (q.get('folder') or [''])[0], (q.get('name') or [None])[0])
            if isinstance(r, tuple):
                return self._json({'ok': False, 'error': r[1]}, r[0])
            return self._json(r)
        if p == '/api/machines':
            return self._json(machines())
        if p.startswith('/api/machines/'):
            name, what = p.split('/')[3:5]
            m = next((x for x in FIX['machines'] if x['name'] == name), None)
            if m is None:
                return self._json({'ok': False, 'error': 'unknown machine'}, 400)
            if what == 'caps':
                return self._json(caps_view(m))
            if what == 'mcp':
                return self._json({'ok': True, 'servers': MCP if m.get('os') == 'mac' else MCP[:2], 'root': m.get('root')})
            return self._json(root_dirs((q.get('path') or [''])[0]))
        if p == '/api/flags':
            return self._json(flags_view((q.get('machine') or [''])[0], (q.get('sid') or [''])[0]))
        if p == '/api/browse':
            return self._json(browse(q))
        if p.startswith('/api/handoff/'):
            return self._json({'ok': True, 'text': '# Handoff: Parser\n\n## Goal\nA recursive-descent parser for the course project.\n\n'
                               '## Standing instructions from the user\n- "Keep the grammar in one file."\n- "Never change the tests."'})
        if p.startswith('/api/summary/'):
            return self._json({'ok': True, 'first': {'text': 'Look at how this works and tell me what to change first.', 'ts': time.time() - 9000},
                               'recent': [{'text': 'Make the header calmer, and line it up with the footer.', 'ts': time.time() - 4000}],
                               'answer': 'Done. The header and the footer now share one grid and baseline.'})
        if p.startswith('/api/files/'):
            folder = (q.get('folder') or [''])[0]
            if q.get('name'):
                return self._json({'ok': True, 'root': '/Users/you/Claude', 'folder': folder, 'dirs': [], 'files': [], 'file': {
                    'name': q['name'][0], 'size': 2048, 'mtime': time.time() - 600, 'text': '# Notes\n\nThe header and the footer share one grid.\n',
                    'binary': False, 'secret': False, 'cut': False}})
            dirs = [] if folder else [{'folder': 'switchboard', 'name': 'switchboard'}, {'folder': 'notes', 'name': 'notes'}]
            files = [{'name': 'README.md', 'size': 4096, 'mtime': time.time() - 3600, 'open': True},
                     {'name': 'plan.md', 'size': 2048, 'mtime': time.time() - 600, 'open': True},
                     {'name': '.env', 'size': 120, 'mtime': time.time() - 86400, 'open': False}]
            return self._json({'ok': True, 'root': '/Users/you/Claude', 'folder': folder, 'dirs': dirs, 'files': files, 'more': False})
        if p == '/api/version':
            return self._json(dict(VERSION, ok=True))
        if p == '/api/server' and SERVER_MODE != 'old':
            return self._json(server_info())
        if p == '/api/server/logs' and SERVER_MODE != 'old':
            return self._json({'ok': True, 'text': SERVER_LOG})
        if p.startswith('/api/folders/'):
            return self._json({'ok': True, 'root': '~/Claude', 'folders': [
                {'folder': '', 'name': 'Claude', 'kids': True}, {'folder': 'switchboard', 'name': 'switchboard', 'kids': True},
                {'folder': 'notes', 'name': 'notes', 'kids': False}]})
        if p == '/api/stream':
            self.send_response(404)
            self.end_headers()
            return
        if KEYS_MODE and p == '/api/keys/readiness':
            return self._json(keys_readiness())
        if KEYS_MODE and p == '/api/keys/devices':
            return self._json(keys_devices())
        if NODE_MODE and p == '/status':
            st = node_status()
            if st is None:
                return self._json({'ok': False, 'error': 'Keys are off on this computer.', 'kind': 'off'}, 503)
            return self._json(st)
        if NODE_MODE and p == '/devices':
            return self._json(keys_devices())
        return self._json({'ok': False, 'error': 'not in the fixture'}, 404)

    def do_POST(self):
        n = int(self.headers.get('Content-Length') or 0)
        try:
            body = json.loads(self.rfile.read(n) or b'{}')
        except ValueError:
            body = {}
        p = urlsplit(self.path).path
        if p == '/api/upload':
            # Where the file lands on its machine, in that machine's own path style (a Windows one gets C:\...).
            q = parse_qs(urlsplit(self.path).query)
            machine, name = (q.get('machine') or [''])[0], (q.get('name') or ['file'])[0]
            m = next((m for m in FIX.get('machines', []) if m.get('name') == machine), {})
            path = ('C:\\Users\\you\\Claude\\uploads\\' if m.get('os') == 'windows' else '/Users/you/Claude/uploads/') + name
            return self._json({'ok': True, 'path': path})
        if p == '/api/prefs' and isinstance(body, dict):
            if 'compact_at' in body and not COMPACT_OLD:
                try:
                    options_mod.compact_check(body['compact_at'])
                except ValueError as e:  # as the server: a refused save changes nothing
                    return self._json({'ok': False, 'error': str(e)}, 400)
            for k, v in body.items():
                if k == 'compact_at':
                    if not COMPACT_OLD:
                        COMPACT.update(at=v, changed=time.time())
                elif k == 'setup_compact_window':
                    OPTIONS[k] = v is True
                    COMPACT['changed'] = time.time()
                elif k == 'appearance' and isinstance(v, dict):
                    PREFS['appearance']['theme'] = v.get('theme', PREFS['appearance']['theme'])
                    PREFS['appearance']['term'].update(v.get('term') or {})
                elif k in options_mod.KEYS:
                    OPTIONS[k] = v is True
                elif k == 'notify' and isinstance(v, dict):
                    PREFS['notify'] = dict(PREFS.get('notify') or {}, **{a: b is not False for a, b in v.items()})
                else:
                    PREFS[k] = v
            return self._json(dict(PREFS, ok=True))
        if p == '/api/usage/refresh':  # Refresh on the usage panel: the same report, as if just asked for
            return self._json(dict(usage(), asked=USAGE_MODE not in ('none', 'old')))
        if p == '/api/action' and body.get('verb') == 'star':
            on = body.get('on')
            return self._json({'ok': True, 'star': True if on is None else bool(on)})
        if p == '/api/budget' and BUDGET_MODE:
            try:
                return self._json(budget_post(body))
            except HttpError as e:
                return self._json({'ok': False, 'error': e.msg}, e.code)
        if p == '/api/library/update' and LIB_MODE == 'auto':
            t = next((x for x in LIB['tools'] if x['name'] == body.get('tool')), None)
            if t:
                if 'auto' in body:
                    t['auto'] = bool(body['auto'])
                if 'when' in body:
                    t['when'] = str(body.get('when') or '')[:200]
            return self._json({'ok': bool(t)})
        if p == '/api/groups':
            try:
                return self._json(GROUPS.op(body))
            except HttpError as e:
                return self._json({'ok': False, 'error': e.msg}, e.code)
        if p == '/api/machines':
            return self._json(machines_post(body))
        if PLACES and p == '/api/privacy_settings':
            return self._json({'ok': True, 'opened': True})
        if PLACES and p in ('/api/new', '/api/continue'):
            f = body.get('folder') or ''
            if outside(f) and trust_of(f) != 'yes' and not body.get('trust'):
                return self._json({'ok': False, 'error': f'Claude has not worked in {f} on this machine yet.', 'need': 'trust',
                                   'folder_shown': shown_of(f), 'declined': trust_of(f) == 'no'}, 409)
        if p == '/api/flags':
            return self._json(flags_plan(body))
        if p.startswith('/api/keys/') or p == '/api/pair_device':
            if not KEYS_MODE:
                return self._json({'ok': False, 'error': 'not in the fixture'}, 404)
            if p == '/api/pair_device':
                return self._json(keys_pair())
            if p == '/api/keys/devices' and body.get('remove'):
                time.sleep(0.4)
                KEYS['removed'].add(body['remove'])
                return self._json({'ok': True})
            if p == '/api/keys/mode' and SWITCH != 'none':
                time.sleep(0.4)
                out, code = keys_set_mode(body)
                return self._json(out, code)
            return self._json({'ok': False, 'error': 'not in the fixture'}, 404)
        if NODE_MODE and p in ('/devices/block', '/devices/unblock'):
            (BLOCKED.add if p.endswith('/block') else BLOCKED.discard)(body.get('id'))
            return self._json({'ok': True})
        if AWAY and p.startswith('/api/') and p not in ('/api/refresh', '/api/event'):
            return self._json({'ok': False, 'kind': 'home_away', 'error': HOME_AWAY}, 503)
        if AWAY and p == '/api/refresh':
            return self._json(away_board(shifted()))
        if p in ('/pair/join', '/pair/new', '/devices/remove', '/relay'):
            if not NODE_MODE:
                return self._json({'ok': False, 'error': 'not in the fixture'}, 404)
            if p == '/pair/new':
                return self._json(keys_pair())
            if p == '/devices/remove':
                KEYS['removed'].add(body.get('id'))
                return self._json({'ok': True})
            if p == '/relay':
                KEYS['relay'] = str(body.get('url') or '')
                return self._json({'ok': True})
            time.sleep(1.5)
            if JOIN_SAYS in JOIN_ERRORS and not (JOIN_SAYS == 'link_no_relay' and body.get('addrs')):
                return self._json({'ok': False, 'error': JOIN_ERRORS[JOIN_SAYS], 'kind': JOIN_SAYS}, 409)
            KEYS['joined'] = True
            return self._json({'ok': True, 'home': {'id': HOME_ID, 'name': 'Studio'}, 'machine': 'laptop'})
        if p == '/api/compact':
            if body.get('confirm'):
                return self._json({'ok': True, 'job': 's-0123456789ab', 'machine': 'server'})
            cost = sb.compact_cost(2 * 18_000)
            return self._json({'ok': True, 'chats': 2, 'found': 2, 'notes': 1, 'machine': body.get('machine'), 'label': None,
                               'sid': body.get('sid'), 'cost': cost})
        return self._json({'ok': True, 'fixture': True})


if __name__ == '__main__':
    port = int(sys.argv[1]) if len(sys.argv) > 1 else 8139
    KEYS['port'] = port
    ThreadingHTTPServer(('127.0.0.1', port), H).serve_forever()
