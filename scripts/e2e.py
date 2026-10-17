#!/usr/bin/env python3
"""End-to-end against the live server, on one machine: new -> stop -> resume -> fresh -> fresh again with the client
gone at once (the app closed) -> UltraCode new -> stop + dismiss. Checks names and that jobs finish at the server.

    scripts/e2e.py <machine> [folder]
The server is SB_SERVER_URL, from the environment or build-hosts.env (see native/scripts/hosts.sh)."""
import json, os, socket, sys, time, urllib.request
from pathlib import Path


def server_url():
    if os.environ.get('SB_SERVER_URL'):
        return os.environ['SB_SERVER_URL']
    f = Path(os.environ.get('SWITCHBOARD_BUILD_HOSTS') or Path.home() / '.config/switchboard/build-hosts.env')
    for line in f.read_text().splitlines() if f.is_file() else []:
        k, _, v = line.strip().partition('=')
        if k == 'SB_SERVER_URL' and v:
            return v.strip('\'"')
    raise SystemExit(f'SB_SERVER_URL is not set: add it to {f} (native/scripts/build-hosts.example.env shows the format).')


BASE = server_url().rstrip('/')
M = sys.argv[1]
FOLDER = sys.argv[2] if len(sys.argv) > 2 else 'sb-e2e'


def api(path, body=None):
    req = urllib.request.Request(BASE + path, data=None if body is None else json.dumps(body).encode(),
                                 headers={'Content-Type': 'application/json', 'X-Switchboard': '1'}, method='GET' if body is None else 'POST')
    try:
        with urllib.request.urlopen(req, timeout=120) as r:
            return json.loads(r.read())
    except urllib.error.HTTPError as e:
        return json.loads(e.read())


def wait_job(job, machine='server', limit=360):
    t0 = time.time()
    while time.time() - t0 < limit:
        r = api(f'/api/job/{machine}/{job}')
        if r.get('state') in ('done', 'error') or r.get('ok') is False:
            print(f'   job {job}: {r.get("state")} in {time.time() - t0:.0f}s', r.get('error') or '', r.get('name') or '', r.get('url') or '')
            return r
        time.sleep(2)
    raise SystemExit(f'job {job} timed out')


def sessions(pred):
    snap = api('/api/refresh', {})
    m = next(x for x in snap['machines'] if x['name'] == M)
    return [s for s in m['sessions'] if pred(s)]


def wait_session(pred, label, limit=120):
    t0 = time.time()
    while time.time() - t0 < limit:
        ss = sessions(pred)
        if ss:
            return ss[0]
        time.sleep(4)
    raise SystemExit(f'no session: {label}')


def step(name):
    print(f'-- {name}', flush=True)


step(f'new session on {M} in {FOLDER}')
r = api('/api/new', {'machine': M, 'folder': FOLDER, 'create': True, 'model': 'haiku',
                     'prompt': 'Remember the codeword PELICAN-42. Reply with only: ready'})
assert r.get('ok'), r
j = wait_job(r['job'])
assert j['state'] == 'done' and j.get('bg_id'), j
assert (j.get('name') or '').endswith(' (new, web)'), j
first_bg = j['bg_id']
s = wait_session(lambda s: s.get('folder') == FOLDER and s.get('bg_id') == first_bg and s.get('status') == 'idle', 'first reply')
sid = s['sid']
print('   sid', sid, 'ctx', s['ctx'], 'url', s['url'])

step('stop')
print('  ', api('/api/action', {'machine': M, 'verb': 'stop', 'sid': sid}))
time.sleep(4)

step('resume')
r = api('/api/action', {'machine': M, 'verb': 'resume', 'sid': sid})
j = wait_job(r['job'])
assert j['state'] == 'done', j
s = wait_session(lambda s: s.get('bg_id') == j['bg_id'] and s.get('status') in ('idle', 'busy'), 'resumed and live')
assert s['sid'] == sid, f'background session should resume in place, got {s["sid"]}'
assert len(sessions(lambda x: x['sid'] == sid)) == 1
print('   resumed in place as', j['bg_id'], 'title', s['title'], 'url', j.get('url'))
assert s['title'].endswith(' (new, web)'), s['title']  # resumed in place keeps its own name
api('/api/action', {'machine': M, 'verb': 'stop', 'sid': sid})
time.sleep(4)

step('start fresh (Opus handoff, then a seeded session)')
r = api('/api/action', {'machine': M, 'verb': 'fresh', 'sid': sid})
j = wait_job(r['job'], limit=420)
assert j['state'] == 'done', j
fresh = wait_session(lambda s: s.get('folder') == FOLDER and s['sid'] != sid and s.get('bg_id') == j['bg_id'] and s.get('status') == 'idle',
                     'fresh reply', limit=180)
print('   fresh sid', fresh['sid'], 'ctx', fresh['ctx'], 'title', fresh['title'])
assert fresh['title'].endswith(' (fresh)'), fresh['title']
assert not sessions(lambda x: x['sid'] == sid), 'the replaced chat should be hidden'
print('FRESH_SID', fresh['sid'], 'OLD_SID', sid)
api('/api/action', {'machine': M, 'verb': 'stop', 'sid': fresh['sid']})
time.sleep(4)

step('start fresh again, with the client gone the moment the request is sent (the app was closed)')
body = json.dumps({'machine': M, 'verb': 'fresh', 'sid': fresh['sid']}).encode()
host = BASE.split('//')[1]
c = socket.create_connection((host.split(':')[0], int(host.split(':')[1])), timeout=5)
c.sendall(b'POST /api/action HTTP/1.1\r\nHost: ' + host.encode() + b'\r\nContent-Type: application/json\r\nX-Switchboard: 1\r\n'
          b'User-Agent: SwitchboardApp/1 e2e\r\nContent-Length: ' + str(len(body)).encode() + b'\r\n\r\n' + body)
c.close()
again = wait_session(lambda s: s.get('folder') == FOLDER and s['title'].endswith(' (fresh 2)') and s.get('status') == 'idle',
                     'second fresh, nobody watching', limit=420)
print('   second fresh', again['sid'], 'title', again['title'])

step('UltraCode new session (effort ultracode, Opus)')
r = api('/api/new', {'machine': M, 'folder': FOLDER, 'mode': 'ultracode', 'prompt': 'Reply with only the word: ready'})
j = wait_job(r['job'])
assert j['state'] == 'done' and '(UltraCode' in (j.get('name') or ''), j
uc = wait_session(lambda s: s.get('folder') == FOLDER and s.get('bg_id') == j['bg_id'] and s.get('status') == 'idle', 'ultracode reply')
print('   ultracode', uc['sid'], 'title', uc['title'], 'model', uc['model'])

time.sleep(2)
step('stop + dismiss everything in the test folder')
for x in [y['sid'] for y in sessions(lambda y: y.get('folder') == FOLDER)]:
    print('  ', api('/api/action', {'machine': M, 'verb': 'dismiss', 'sid': x}))
left = sessions(lambda s: s.get('folder') == FOLDER)
print('   left in list:', len(left))
print('E2E OK' if not left else 'E2E LEFTOVERS')
