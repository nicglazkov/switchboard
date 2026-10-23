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
PREFS = {'chrome_machine': None, 'last': None, 'presets': [],
         'appearance': {'theme': 'system', 'term': {'scheme': 'converge-dark', 'size': None, 'size_phone': None,
                                                    'cursor': 'block', 'blink': True}}}
VERSION = {'version': '0.1.0', 'build': 'fixture'}
# The opt-in switches, worded by the server's own options.py. SB_FIXTURE_OPTIONS=on starts them all on (an install
# that had them before); SB_FIXTURE_PUSH=1 has iPhone notifications set up.
OPTIONS = {k: os.environ.get('SB_FIXTURE_OPTIONS') == 'on' for k in options_mod.KEYS}
PUSH_READY = bool(os.environ.get('SB_FIXTURE_PUSH'))


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
    for m in d['machines']:
        for s in m['sessions']:
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
    return {'ok': True, 'now': now, 'week': {'pct': pct, 'reset': reset, 'pace': min(100, pct * 1.3)},
            'five': {'pct': sm.get('five_hour'), 'reset': sm.get('five_hour_reset')},
            'days': [{'day': d.isoformat(), 'used': u} for d, u in zip(days, used)],
            'weeks': [{'reset': reset - 7 * 86400 * k, 'pct': x} for k, x in ((4, 62), (3, 88), (2, 71), (1, 97))]}


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
                         'first_prompt': 'Look at how this works and tell me what to change first.', 'history': [], 'history_count': 0})
    one = lambda k: (q.get(k) or [''])[0]
    if one('machine'):
        rows = [r for r in rows if r['machine'] == one('machine')]
    if one('state'):
        rows = [r for r in rows if r['state'] == one('state')]
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
                                                                     'folders': folders, 'states': {}}}


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
                    'features': ['mcp', 'root_dirs', 'files'] + (['places'] if PLACES and name in ROAM else []),
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


class H(BaseHTTPRequestHandler):
    def log_message(self, fmt, *args):
        pass

    def _json(self, data, code=200):
        raw = json.dumps(data).encode()
        self.send_response(code)
        self.send_header('Content-Type', 'application/json')
        self.send_header('Content-Length', str(len(raw)))
        self.end_headers()
        self.wfile.write(raw)

    def do_GET(self):
        u = urlsplit(self.path)
        p, q = u.path, parse_qs(u.query)
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
        if p == '/api/prefs':
            return self._json(dict(PREFS, ok=True, **options_mod.view(OPTIONS, PUSH_READY, PREFS.get('notify'))))
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
            for k, v in body.items():
                if k == 'appearance' and isinstance(v, dict):
                    PREFS['appearance']['theme'] = v.get('theme', PREFS['appearance']['theme'])
                    PREFS['appearance']['term'].update(v.get('term') or {})
                elif k in options_mod.KEYS:
                    OPTIONS[k] = v is True
                elif k == 'notify' and isinstance(v, dict):
                    PREFS['notify'] = dict(PREFS.get('notify') or {}, **{a: b is not False for a, b in v.items()})
                else:
                    PREFS[k] = v
            return self._json(dict(PREFS, ok=True))
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
        if p == '/api/compact':
            if body.get('confirm'):
                return self._json({'ok': True, 'job': 's-0123456789ab', 'machine': 'server'})
            cost = sb.compact_cost(2 * 18_000)
            return self._json({'ok': True, 'chats': 2, 'found': 2, 'notes': 1, 'machine': body.get('machine'), 'label': None,
                               'sid': body.get('sid'), 'cost': cost})
        return self._json({'ok': True, 'fixture': True})


if __name__ == '__main__':
    port = int(sys.argv[1]) if len(sys.argv) > 1 else 8139
    ThreadingHTTPServer(('127.0.0.1', port), H).serve_forever()
