#!/usr/bin/env python3
"""Fake fleet for the iPhone review. Runs the REAL server.App and web files from a repo copy, but every machine is a
fake agent answered here, so actions never reach a real Claude session.

usage: fakefleet.py <repo-copy> <live.json> <port> <host,host> <ctrl-dir>

Control files in <ctrl-dir> (touch to turn on, rm to turn off):
  off-<machine>     that machine does not answer (ssh timeout)
  fail-<verb>       jobs of that verb end in an error (start, fresh, resume, handoff, continue)
  failnow-<verb>    the machine refuses that verb at once (400)
  slow              jobs take 40 s instead of 8
  empty-<machine>   that machine lists no sessions
  folders-err       folder listing fails
  open.json         [{"machine","bg"}] terminals open on the hub (the reaper leaves those alone)
  config/           the server's config dir: reaper.json, caps.json (no reaper.json: dry run; caps.json is
                    written with all three machines on unless it exists)
  ask-fail          the auto pick's Claude call fails (the rules pick instead); ask-slow makes it take 45 s
  iphone            the laptop has an iPhone connected (caps)
  old-agent         the machines answer like an agent from before v2 (no features: no browse, no flags)
  fast              jobs take 3 s instead of 8 (the end-to-end tests use it)
  work-<sid>        that chat has a subagent running, so a relaunch, stop or fresh start has to wait (busy verb)
Pushes are written to <ctrl-dir>/pushes.log; the reaper's log is <ctrl-dir>/state/reaped.json. Every request a fake
machine gets is written to <ctrl-dir>/rpc.log (one JSON line each, upload data left out). Uploads go through the real
agent's code (sb.rpc_upload) into <ctrl-dir>/home/<machine>/uploads.

Environment:
  LAB_POLL=<s>      seconds between polls (default 5; the real server polls every 20)
  LAB_HUB=1         Add a machine asks a real hub (SWITCHBOARD_HUB_URL) for its code, and machines paired with
                    switchboard-node join the fleet through it, next to the fake ones (see tests/e2e)
"""
import json, os, re, sys, threading, time, uuid
from pathlib import Path

ROOT, LIVE, PORT, HOSTS, CTRL = Path(sys.argv[1]), Path(sys.argv[2]), int(sys.argv[3]), sys.argv[4].split(','), Path(sys.argv[5])
CTRL.mkdir(parents=True, exist_ok=True)
sys.path.insert(0, str(ROOT / 'server'))
import fleet as fleet_mod  # noqa: E402
import server as srv  # noqa: E402

D = json.loads(LIVE.read_text())
SHIFT = time.time() - D['now']
TK = {'active', 'mtime', 'updated', 'five_hour_reset', 'seven_day_reset', 'recent'}


def shift(v, key=None):
    if isinstance(v, dict):
        return {k: shift(x, k) for k, x in v.items()}
    if isinstance(v, list):
        return [shift(x) for x in v]
    if key in TK and isinstance(v, (int, float)) and v > 1e9:
        return v + SHIFT
    return v


D = shift(D)
NOW0 = time.time()


def started(s):
    """live.json from an older agent has no start times: a chat starts a while before it was last used."""
    t = s.get('active') or s.get('mtime') or NOW0
    return s.get('started') or t - 3600 * (1 + (s.get('ctx') or 0) / 40000)


for m in D['machines']:
    for s in m['sessions'] + (m.get('replaced') or []):
        s['started'] = started(s)
LOCK = threading.Lock()
STATE = {}
for m in D['machines']:
    sessions = []
    for s in m['sessions']:
        s = {k: v for k, v in s.items() if k not in ('stale', 'stalled', 'held')}
        s['_warm_until'] = 0
        sessions.append(s)
    STATE[m['name']] = {'label': m['label'], 'root': m['root'], 'limits': m.get('limits'), 'sessions': sessions,
                        'replaced': list(m.get('replaced') or []), 'launcher': m.get('launcher')}

HELD = set()
# v2: Claude's memory per machine, with one leftover process on the studio
MEM = {'studio': {'claude_mb': 3120, 'procs': 7, 'orphans': [{'pid': 48121, 'mb': 410, 'age_h': 30.2}]},
       'laptop': {'claude_mb': 2809, 'procs': 12, 'orphans': []}, 'tower': {'claude_mb': 893, 'procs': 2, 'orphans': []}}


# v2: what each machine can do, as the real probes measured it on 2026-10-12 (the studio's from its spec values)
CAPS = {
    'studio': {'os': 'mac', 'arch': 'x86_64', 'cpu': 'Intel(R) Xeon(R) W-2150B CPU @ 3.00GHz', 'cores': 20, 'ram_gb': 64, 'ram_free_gb': 21.5,
             'disk_free_gb': 412, 'gpu': 'Radeon Pro Vega 64', 'tools': {
                 'xcode': '26.3', 'ios_sim': True, 'iphone': 'paired', 'android_sdk': None, 'adb_devices': None, 'emulator': False,
                 'java': None, 'rust': '1.97.1', 'node': '22', 'python': '3.12', 'go': None, 'docker': None, 'dotnet': None,
                 'msvc': None, 'cuda': None, 'tailscale': True},
             'repos': ['switchboard', 'garden-planner', 'video-notes', 'price-check', 'parking-map', 'cloud-status']},
    'laptop': {'os': 'mac', 'arch': 'arm64', 'cpu': 'Apple M1 Max', 'cores': 10, 'ram_gb': 64, 'ram_free_gb': 38.2, 'disk_free_gb': 783,
                'gpu': 'Apple M1 Max', 'tools': {
                    'xcode': '26.3', 'ios_sim': True, 'iphone': 'paired', 'android_sdk': '37', 'adb_devices': 0, 'emulator': True,
                    'java': '21', 'rust': '1.97.1', 'node': '26', 'python': '3.9', 'go': '1.26', 'docker': '29', 'dotnet': None,
                    'msvc': None, 'cuda': None, 'tailscale': True},
                'repos': ['music-player', 'Relay', 'notebook-cli', 'route-planner', 'scratch', 'audio-bridge', 'Overhang']},
    'tower': {'os': 'windows', 'arch': 'AMD64', 'cpu': '13th Gen Intel(R) Core(TM) i9-13900KS', 'cores': 32, 'ram_gb': 64,
                'ram_free_gb': 44.0, 'disk_free_gb': 163, 'gpu': 'NVIDIA GeForce RTX GPU', 'tools': {
                    'xcode': None, 'ios_sim': False, 'iphone': 'none', 'android_sdk': '37', 'adb_devices': 1, 'emulator': True,
                    'java': '21', 'rust': '1.97.1', 'node': '20', 'python': '3.12', 'go': None, 'docker': '29', 'dotnet': None,
                    'msvc': '2022', 'cuda': '13.2', 'tailscale': True},
                'repos': ['transit-app', 'maps-api', 'net-monitor', 'outage-map', 'robot-workspace', 'radar-viewer']},
}
ROOTS = {'studio': 'Desktop/Claude', 'laptop': 'Desktop/Claude', 'tower': 'Documents/code'}
# A live.json with other machine names borrows the made-up machines' answers, in order.
for _name, _spare in zip([n for n in STATE if n not in CAPS], [n for n in CAPS if n not in STATE]):
    CAPS[_name], ROOTS[_name] = CAPS[_spare], ROOTS[_spare]
    if _spare in MEM:
        MEM[_name] = MEM[_spare]
ASKS = []  # (machine, seconds) of each fake Claude pick, for the tests


def fake_ask(machine, req):
    """Stands in for claude -p --model haiku: picks with the server's own rules, a little slower."""
    import features
    if flag('ask-fail'):
        time.sleep(0.5)
        return {'ok': False, 'error': 'Claude did not answer: claude -p failed rc=1 (fake)'}
    t0 = time.time()
    time.sleep(45 if flag('ask-slow') else 2)
    m = re.search(r'Machines:\n(.*?)\n\nFolder the user chose: (.*?)\n\nTask:\n(.*)', req.get('prompt') or '', re.S)
    rows = json.loads(m.group(1))
    cands = [(r['name'], r['label'], dict(r, tools=dict(CAPS[r['name']]['tools'], **r['tools']))) for r in rows]
    folder = '' if m.group(2) == 'none' else m.group(2)
    name, f, why = features.rules_pick(cands, m.group(3), folder)
    ASKS.append((machine, round(time.time() - t0, 2)))
    return {'ok': True, 'text': '```json\n' + json.dumps({'machine': name, 'folder': f, 'why': why}) + '\n```', 'ms': 2000}


def find(machine, pred):
    for s in STATE[machine]['sessions']:
        if pred(s):
            return s
    return None


def demo():
    """Every state the page can show, on sessions that exist."""
    im = STATE['studio']['sessions']
    t = time.time()
    # a warm chat with 38 minutes of cache left, and a small warm one
    s = find('studio', lambda s: s['title'].startswith('Team meeting'))
    if s:
        s.update(active=t - 22 * 60, mtime=t - 22 * 60, _warm_until=t + 38 * 60, cold=False)
    s = find('studio', lambda s: s['title'].startswith('Quiz 2'))
    if s:
        s.update(active=t - 4 * 60, mtime=t - 4 * 60, _warm_until=t + 56 * 60, cold=False, ctx=42_000)
    # a held message on a big cold chat
    s = find('studio', lambda s: s['title'].startswith('Kitchen remodel'))
    if s:
        HELD.add(('studio', s['sid']))
        s.update(active=t - 3 * 3600, mtime=t - 3 * 3600)
    # Claude asked something in a background chat with no Remote Control link (copy claude attach)
    s = find('studio', lambda s: s['title'].startswith('Music player'))
    if s:
        s.update(status=None, state='blocked', kind='background', url=None, active=t - 40 * 60, mtime=t - 40 * 60)
    # a stalled one
    s = find('laptop', lambda s: s['title'].startswith('Course notes (fresh)'))
    if s:
        s.update(kind='background', status='busy', state='working', active=t - 7 * 3600, mtime=t - 7 * 3600)
    # one with a handoff ready and a few repeats
    s = find('studio', lambda s: s['title'].startswith('Fix login bug'))
    if s:
        s.update(handoff=True, active=t - 5 * 3600, mtime=t - 5 * 3600)
    s = find('studio', lambda s: s['title'].startswith('New garden bed'))
    if s:
        s.update(repeats=4)
    # the tower has a session working in a terminal
    s = find('tower', lambda s: s['title'].startswith('RAM usage'))
    if s:
        s.update(status='busy', active=t - 30, mtime=t - 30)


demo()
JOBS = {}


CHROME = {'studio': {'installed': True, 'connector': True, 'connected': True},
          'laptop': {'installed': True, 'connector': True, 'connected': False},
          'tower': {'installed': True, 'connector': False, 'connected': False}}
NAMES = {}
OLD = {}
OLD_TOPICS = ['Porch string lights', 'Bike brake pads', 'Grocery budget math', 'Ski lesson', 'DNS update audit', 'Rug pad sizing',
              'Doorbell wiring', 'Radio antenna', 'Narration voices', 'Parking map', 'Music playlists', 'Closet shelves',
              'Printer crash', 'Guest VM network', 'Shower sensor', 'Lamp color match', 'Shelf mount', 'Laptop mount',
              'Deck coating', 'Release notes']


def browse_rows(machine):
    """Every chat here: the live ones, the replaced ones, and older made-up ones (same ids every call)."""
    st, t = STATE[machine], time.time()
    if machine not in OLD:
        import random
        rnd = random.Random(machine)
        OLD[machine] = [{'sid': str(uuid.UUID(int=rnd.getrandbits(128))), 'title': f'{topic} ({kind})', 'age': rnd.uniform(2, 400) * 86400,
                         'folder': rnd.choice(['', 'switchboard', 'garden-planner', 'notes', 'course-notes']), 'ctx': rnd.randint(8, 400) * 1000,
                         'size': rnd.randint(40, 9000) * 1024, 'archived': rnd.random() < 0.2}
                        for topic in OLD_TOPICS for kind in ['fresh']]
    rows = []
    for s in st['sessions'] + st['replaced']:
        last = s.get('active') or s.get('mtime') or t
        rows.append({'sid': s['sid'], 'title': NAMES.get((machine, s['sid'])) or s.get('title') or 'Untitled',
                     'first_prompt': f'Start on {(s.get("title") or "this").split(" (")[0]}.', 'last_prompt': 'Keep going and tell me when it is done.',
                     'cwd': None, 'folder': s.get('folder'), 'model': s.get('model') or 'claude-opus-5-5', 'ctx': s.get('ctx') or 0,
                     'size': 200_000 + (s.get('ctx') or 0) * 6, 'mtime': last, 'started': s.get('started') or last - 3600, 'last': last,
                     'archived': None, 'url': s.get('url'), 'replaced': s.get('how'), 'by': s.get('by')})
    for o in OLD[machine]:
        last = t - o['age']
        rows.append({'sid': o['sid'], 'title': NAMES.get((machine, o['sid'])) or o['title'], 'first_prompt': f'Help me with {o["title"].split(" (")[0].lower()}.',
                     'last_prompt': 'Thanks, that works.', 'cwd': None, 'folder': o['folder'], 'model': 'claude-opus-5-5', 'ctx': o['ctx'],
                     'size': o['size'], 'mtime': last, 'started': last - 7200, 'last': last, 'archived': last if o['archived'] else None,
                     'url': None, 'replaced': None, 'by': None})
    return rows


def flag(name):
    return (CTRL / name).exists()


_SB = {}


def real_upload(machine, req):
    """The real agent's upload code (sb.rpc_upload), with each fake machine's home under <ctrl-dir>/home."""
    if 'sb' not in _SB:
        os.environ['SB_HOME'] = str(CTRL / 'home' / '_')
        sys.path.insert(0, str(ROOT / 'agent'))
        import sb
        _SB['sb'] = sb
    sb = _SB['sb']
    sb.SB_HOME = CTRL / 'home' / machine
    try:
        return sb.rpc_upload(req)
    except ValueError as e:
        return {'ok': False, 'error': str(e)}


def new_sid():
    return str(uuid.uuid4())


def job_secs():
    return 40 if flag('slow') else 3 if flag('fast') else 8


def add_session(machine, title, folder='', kind='background', ctx=0):
    s = {'sid': new_sid(), 'title': title, 'ctx': ctx, 'cold': False, 'mtime': time.time(), 'active': time.time(), 'started': time.time(),
         'kind': kind, 'status': 'busy', 'state': 'working', 'url': 'https://claude.ai/code/fake-' + uuid.uuid4().hex[:8],
         'bg_id': uuid.uuid4().hex[:8], 'folder': folder, 'cwd': '/fake/' + folder, 'handoff': False, 'model': 'claude-opus-5-5',
         'flags': {'remote': True}, '_warm_until': time.time() + 3600}
    STATE[machine]['sessions'].insert(0, s)
    return s


def finish(machine, j):
    """What a finished job leaves behind on the machine."""
    verb, req = j['verb'], j['req']
    if verb == 'start':
        if req.get('handoff_text') is not None:
            topic = req.get('topic') or 'Continued'
            name = f"{topic} (from {req.get('from')})"
        else:
            words = (req.get('prompt') or '').split()[:5]
            tag = {'ultracode': 'UltraCode', 'max': 'Max effort'}.get(req.get('mode'), 'new')
            name = f"{' '.join(words).capitalize() or (req.get('folder') or 'New session')} ({tag}, {req.get('origin') or 'web'})"
        if req.get('name'):
            name = req['name']
        s = add_session(machine, name, req.get('folder') or '')
        if ' (for ' in name:  # dispatched: works for a while, then reports
            s['_idle_at'] = time.time() + (60 if flag('slow') else 6 if flag('fast') else 12)
            s['_report'] = ('Built and ran it here.\n\nWhat changed: ' + (req.get('folder') or 'the Claude folder') +
                            '/README.md\n\nLeft: nothing.')
        elif (req.get('prompt') or '').strip():  # Claude answers the first message, then waits for the next
            s['_idle_at'] = time.time() + (60 if flag('slow') else 2 if flag('fast') else 12)
        return {'name': name, 'sid': s['sid'], 'url': s['url'], 'bg_id': s['bg_id']}
    old = find(machine, lambda s: s['sid'] == req.get('sid'))
    topic = (old or {}).get('title', 'Session')
    if verb == 'fresh':
        name = f"{topic.split(' (')[0]} (fresh)"
        s = add_session(machine, name, (old or {}).get('folder') or '')
        if old:
            STATE[machine]['sessions'].remove(old)
            STATE[machine]['replaced'].insert(0, {'sid': old['sid'], 'title': old['title'], 'active': old['active'], 'how': 'fresh',
                                                  'started': old.get('started'), 'ctx': old.get('ctx'),
                                                  'by': {'machine': machine, 'sid': s['sid']}, 'folder': old.get('folder'),
                                                  'kind': old.get('kind'), 'status': old.get('status'), 'url': old.get('url')})
        HELD.discard((machine, req.get('sid')))
        return {'name': name, 'sid': s['sid'], 'url': s['url']}
    if verb == 'resume' and req.get('relaunch'):  # claude --bg --resume <sid> <flags>: a new id, the old one is Replaced
        flags = [list(f) for f in req.get('flags') or []]
        perm = next((f[1] for f in flags if f[0] == '--permission-mode' and len(f) > 1), None)
        s = add_session(machine, topic, (old or {}).get('folder') or '', ctx=(old or {}).get('ctx') or 0)
        s.update(status='idle', state='done', launch={'flags': [['--remote-control']] + flags, 'other': [], 'from': 'roster'},
                 now={'model': req.get('model'), 'effort': req.get('effort'), 'perm': perm or 'default',
                      'chrome': ['--chrome'] in flags})
        if old:
            STATE[machine]['sessions'].remove(old)
            STATE[machine]['replaced'].insert(0, {'sid': old['sid'], 'title': old['title'], 'active': old['active'], 'how': 'resumed',
                                                  'started': old.get('started'), 'ctx': old.get('ctx'),
                                                  'by': {'machine': machine, 'sid': s['sid']}, 'folder': old.get('folder'),
                                                  'kind': old.get('kind'), 'status': 'stopped', 'url': old.get('url')})
        return {'name': topic, 'sid': s['sid'], 'url': s['url'], 'bg_id': s['bg_id']}
    if verb == 'resume':
        if old:
            old.update(kind='background', status='idle', state='done', active=time.time(), mtime=time.time(), _warm_until=time.time() + 3600)
        return {'name': topic, 'sid': req.get('sid'), 'url': (old or {}).get('url')}
    if verb == 'handoff':
        if old:
            old['handoff'] = True
            if req.get('close'):
                old['status'] = 'stopped'
        return {}
    return {}


def settle(s, now):
    if s.get('_idle_at') and now >= s['_idle_at']:  # a new or dispatched chat finishes its reply
        s.update(status='idle', state='done', ctx=s['ctx'] or 18_400, active=now, mtime=now, _idle_at=None)


def agent(machine, req):
    verb = req.get('verb')
    st = STATE[machine]
    with LOCK:
        if verb == 'list':
            if flag(f'empty-{machine}'):
                return {'ok': True, 'machine': machine, 'sessions': [], 'replaced': [], 'limits': st['limits'], 'root': st['root']}
            now = time.time()
            out = []
            for s in st['sessions']:
                settle(s, now)
                d = {k: v for k, v in s.items() if not k.startswith('_')}
                d['cache_left'] = max(0, round(s['_warm_until'] - now)) if s['_warm_until'] else 0
                if d.get('kind') == 'background' and d.get('status') in ('idle', 'busy', 'waiting') and 'mb' not in d:
                    d['mb'] = 180 + int(s['sid'][:4], 16) % 420
                if s['_warm_until']:
                    d['cold'] = d['cache_left'] <= 0
                out.append(d)
            return {'ok': True, 'machine': machine, 'sessions': out, 'replaced': st['replaced'], 'limits': st['limits'],
                    'root': st['root'], 'launcher': st['launcher'], 'mem': MEM.get(machine),
                    'features': [] if flag('old-agent') else ['flags', 'ping', 'cleanup', 'browse', 'rename', 'summary', 'files',
                                                                 'upload', 'busy', 'relaunch']}
        if verb == 'caps':
            c = json.loads(json.dumps(CAPS[machine]))
            if machine == 'laptop' and flag('iphone'):
                c['tools']['iphone'] = 'connected'
            c.setdefault('chrome', CHROME.get(machine))
            return {'ok': True, 'caps': dict(c, measured=time.time() - 1800)}
        if verb == 'report':
            s = find(machine, lambda s: s['sid'] == req.get('sid'))
            if not s:
                return {'ok': False, 'error': 'session not found on this machine'}
            return {'ok': True, 'text': s.get('_report') or 'Did what was asked (fake report).', 'at': time.time()}
        if verb == 'reap_orphans':
            mem = MEM.get(machine) or {'orphans': []}
            due = [o for o in mem['orphans'] if o['age_h'] >= max(6, req.get('min_age_h') or 6)]
            if not req.get('dry_run'):
                mem['orphans'] = [o for o in mem['orphans'] if o not in due]
                mem['claude_mb'] -= sum(o['mb'] for o in due)
            return {'ok': True, 'killed': due, 'failed': [], 'dry_run': bool(req.get('dry_run'))}
        if verb == 'browse' and not flag('old-agent'):
            rows = browse_rows(machine)
            since = req.get('since') or 0
            return {'ok': True, 'rows': [r for r in rows if r['mtime'] > since], 'all': {r['sid']: r['mtime'] for r in rows},
                    'partial': False, 'now': time.time()}
        if verb == 'summary' and not flag('old-agent'):
            r = next((r for r in browse_rows(machine) if r['sid'] == req.get('sid')), None)
            if not r:
                return {'ok': False, 'error': 'session not found on this machine'}
            return {'ok': True, 'sid': r['sid'], 'title': r['title'], 'first': {'ts': r['started'], 'text': r['first_prompt']},
                    'recent': [{'ts': r['last'] - 600, 'text': 'And the light theme too, please.'}, {'ts': r['last'], 'text': r['last_prompt']}],
                    'answer': 'Done. The widget now refreshes on push, and the timer counts down on its own between pushes.',
                    'cwd': r['cwd'], 'model': r['model'], 'ctx': r['ctx'], 'size': r['size'], 'last': r['last']}
        if verb == 'rename' and not flag('old-agent'):
            NAMES[(machine, req.get('sid'))] = req.get('name')
            s = find(machine, lambda s: s['sid'] == req.get('sid'))
            if s:
                s['title'] = req.get('name')
            return {'ok': True, 'name': req.get('name'), 'in_transcript': not s}
        if verb == 'files':
            root = ROOTS[machine]
            folder, now = req.get('folder') or '', time.time()
            if req.get('name') is not None:
                n = req['name']
                text = (f'# {n}\n\nA file in {folder or root} on the fake {machine}.\n\n' +
                        '\n'.join(f'- line {i}: the browser shows the first 64 KB of a file' for i in range(1, 40)))
                return {'ok': True, 'root': root, 'folder': folder,
                        'file': {'name': n, 'size': 2048 + len(n) * 300, 'mtime': now - 5400, 'secret': n == '.env.local',
                                 'binary': n.endswith('.png'), 'cut': False, 'text': None if n.endswith('.png') else text}}
            subs = {'': ['switchboard', 'garden-planner', 'course-notes', 'notes'], 'switchboard': ['server', 'ios', 'native']}.get(folder, [])
            files = {'': ['README.md', 'todo.txt', 'budget.csv'],
                     'switchboard': ['CLAUDE.md', 'README.md', 'shot.png', 'credentials.json']}.get(folder, ['notes.md', 'main.py'])
            return {'ok': True, 'root': root, 'folder': folder, 'more': False,
                    'dirs': [{'folder': f'{folder}/{n}' if folder else n, 'name': n} for n in subs],
                    'files': [{'name': n, 'size': 900 + i * 14000, 'mtime': now - 3600 * (i + 1),
                               'open': 'credential' not in n} for i, n in enumerate(files)]}
        if verb == 'upload' and not flag('old-agent'):
            return real_upload(machine, req)
        if verb == 'busy' and not flag('old-agent'):
            s = find(machine, lambda s: s['sid'] == req.get('sid'))
            if not s:
                return {'ok': False, 'error': 'session not found on this machine'}
            settle(s, time.time())
            subs = ['general-purpose: Run the test suite'] if flag(f'work-{s["sid"]}') else []
            replying = s.get('status') == 'busy'
            live = bool(subs) or replying
            summary = ('A subagent is still running.' if subs else 'Claude is still replying.' if replying else '')
            return {'ok': True, 'live': live, 'subagents': subs, 'commands': [], 'replying': replying, 'pending': [],
                    'summary': summary}
        if verb == 'folders':
            if flag('folders-err'):
                return {'ok': False, 'error': 'Could not read folders (fake)'}
            root = ROOTS[machine]
            tree = {'': ['switchboard', 'garden-planner', 'course-notes', 'video-notes', 'notes', 'parking-map', 'a really long folder name that keeps going and going'],
                    'switchboard': ['server', 'ios', 'native', 'android', 'docs'], 'switchboard/ios': ['App', 'Widget'],
                    'deep': ['a'], 'video-notes': ['cache']}
            if 'parent' in req:
                p = req['parent']
                kids = tree.get(p, ['notes', 'src'] if p.count('/') < 6 else [])
                return {'ok': True, 'root': root, 'parent': p,
                        'folders': [{'folder': f'{p}/{n}' if p else n, 'name': n, 'kids': (f'{p}/{n}' if p else n) in tree or p.count('/') < 5,
                                     'recent': time.time() - 3600 if n == 'switchboard' else None} for n in kids]}
            return {'ok': True, 'root': root, 'folders': [{'folder': '', 'recent': time.time() - 600},
                                                           {'folder': 'switchboard', 'recent': time.time() - 120},
                                                           {'folder': 'garden-planner', 'recent': time.time() - 86400},
                                                           {'folder': 'course-notes', 'recent': time.time() - 3 * 86400},
                                                           {'folder': 'notes'}]}
        if flag(f'failnow-{verb}'):
            return {'ok': False, 'error': f'{st["label"]} refused it (fake error for {verb})'}
        if verb in ('start', 'fresh', 'resume', 'handoff'):
            jid = uuid.uuid4().hex[:12]
            JOBS[jid] = {'verb': verb, 'req': req, 't0': time.time(), 'secs': job_secs(), 'fail': flag(f'fail-{verb}') or
                         (verb == 'start' and req.get('handoff_text') is not None and flag('fail-continue')), 'res': None}
            return {'ok': True, 'job': jid}
        if verb == 'job':
            j = JOBS.get(req.get('id'))
            if not j:
                return {'ok': False, 'error': 'no such job'}
            el = time.time() - j['t0']
            if el < min(2, j['secs'] / 3):
                return {'ok': True, 'state': 'queued'}
            if el < j['secs']:
                return {'ok': True, 'state': 'running'}
            if j['fail']:
                return {'ok': True, 'state': 'error', 'error': 'Claude did not start there (fake error). The old chat is untouched.'}
            if j['res'] is None:
                j['res'] = finish(machine, j)
            return {'ok': True, 'state': 'done', **j['res']}
        if verb == 'handoff_text':
            return {'ok': True, 'text': 'Fake handoff notes.', 'carry': {}}
        if verb == 'mkdir':
            return {'ok': True, 'folder': req.get('folder')}
        if verb == 'supersede':
            s = find(machine, lambda s: s['sid'] == req.get('sid'))
            if s:
                st['sessions'].remove(s)
                st['replaced'].insert(0, {'sid': s['sid'], 'title': s['title'], 'active': s['active'], 'how': 'continued',
                                          'started': s.get('started'), 'ctx': s.get('ctx'),
                                          'by': req.get('by'), 'folder': s.get('folder'), 'kind': s.get('kind'), 'url': s.get('url')})
            return {'ok': True}
        if verb == 'dismiss':
            s = find(machine, lambda s: s['sid'] == req.get('sid'))
            if s:
                st['sessions'].remove(s)
            st['replaced'] = [r for r in st['replaced'] if r['sid'] != req.get('sid')]
            return {'ok': True}
        if verb == 'stop':
            s = find(machine, lambda s: s['sid'] == req.get('sid'))
            if s:
                s.update(status='stopped', state='done')
            return {'ok': True}
        if verb == 'pass':
            HELD.discard((machine, req.get('sid')))
            return {'ok': True}
    return {'ok': False, 'error': f'unknown verb {verb}'}


TARGETS = {f'fake@{n}': n for n in STATE}


RPC_LOCK = threading.Lock()


def rpc_log(target, req):
    row = {'t': time.time(), 'target': target, 'req': {k: v for k, v in req.items() if k != 'data'}}
    with RPC_LOCK, open(CTRL / 'rpc.log', 'a') as f:
        f.write(json.dumps(row) + '\n')


def transport(target, req, timeout):
    rpc_log(target, req)
    if target.startswith('node:'):  # a machine paired through the real hub (LAB_HUB=1)
        return srv.nodes_mod.transport(target, req, timeout)
    name = TARGETS.get(target)
    if name is None:  # a machine added in Settings over ssh: the lab has no such host
        time.sleep(0.3)
        raise OSError(f'ssh: Could not resolve hostname {target.split("@")[-1]}: Name or service not known')
    if flag(f'off-{name}'):
        time.sleep(0.3)
        raise OSError('ssh: connect to host port 22: Operation timed out')
    time.sleep(0.15)
    if req.get('verb') == 'ask':  # outside the lock: it takes a while, like the real one
        if flag('ask-slow') and timeout < 45:
            time.sleep(timeout)
            raise OSError('ssh: timed out')
        return fake_ask(name, req)
    return agent(name, req)


MACHINES = [{'name': m['name'], 'label': m['label'], 'target': f"fake@{m['name']}", 'os': m.get('os') or 'mac'} for m in D['machines']]
fl = fleet_mod.Fleet(MACHINES, transport)


def pusher(title, body, url):
    with open(CTRL / 'pushes.log', 'a') as f:
        f.write(json.dumps({'t': time.strftime('%H:%M:%S'), 'title': title, 'body': body, 'url': url}) + '\n')


state = CTRL / 'state'
state.mkdir(parents=True, exist_ok=True)
(CTRL / 'config').mkdir(parents=True, exist_ok=True)
if not (CTRL / 'config' / 'caps.json').exists():
    (CTRL / 'config' / 'caps.json').write_text(json.dumps({n: True for n in STATE}))
# a background chat the reaper stopped two hours ago, after 26 h idle (shows the note and the Resume path)
_r = find('studio', lambda s: s.get('kind') == 'background' and s.get('status') == 'stopped')
if _r and not (state / 'reaped.json').exists():
    _r.update(active=time.time() - 28 * 3600, mtime=time.time() - 28 * 3600)
    (state / 'reaped.json').write_text(json.dumps([{'at': time.time() - 2 * 3600, 'machine': 'studio', 'sid': _r['sid'],
                                                    'bg_id': _r.get('bg_id'), 'title': _r['title'], 'reason': 'idle', 'idle_h': 26}]))
app = srv.App(fl, state, ROOT / 'server' / 'web', pusher=pusher, guard_delay=1, cfg_dir=CTRL / 'config',
              server_cfg=srv.ServerConfig(CTRL / 'config', state, port=PORT))  # without it only localhost is let in


def _lab_pair(host, hub=None):  # Add a machine in the lab: a made-up code, so the live hub is never asked for one
    nm, h = srv.nodes_mod, srv.nodes_mod.ws_host(host)
    return {'ok': True, 'code': 'K7M2QX', 'expires': time.time() + 600, 'hub': f'{h}:8132', 'pair': f'switchboard-node pair {h}:8132 K7M2QX',
            'install': nm.install_lines(f'{h}:8132', 'K7M2QX', f'http://{h}:8131/node')}


if not os.environ.get('LAB_HUB'):
    srv.nodes_mod.pair = _lab_pair


def opened():
    try:
        return {(x['machine'], x['bg']) for x in json.loads((CTRL / 'open.json').read_text())}
    except (OSError, ValueError, KeyError, TypeError):
        return set()


app.feat.reaper.opened = opened
real_held = app.guard.held
app.guard.held = lambda m, sid: (m, sid) in HELD or real_held(m, sid)
real_answer = app.guard.answer


def answer(evt):
    HELD.discard((evt.get('machine'), evt.get('session_id')))
    real_answer(evt)


app.guard.answer = answer
POLL_EVERY = float(os.environ.get('LAB_POLL') or 5)


def hub_sync():
    if os.environ.get('LAB_HUB'):
        srv.nodes_mod.sync(fl)


hub_sync()
fl.poll()


def poller():
    while True:
        time.sleep(POLL_EVERY)
        try:
            hub_sync()
            fl.poll()
            app.feat.after_poll()
        except Exception as e:
            print('poll failed', e, flush=True)


threading.Thread(target=poller, daemon=True).start()
for h in HOSTS[1:]:
    threading.Thread(target=lambda h=h: srv.make_server(h, PORT, app).serve_forever(), daemon=True).start()
print(f'fake fleet on {HOSTS}:{PORT}', flush=True)
srv.make_server(HOSTS[0], PORT, app).serve_forever()
