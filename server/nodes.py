"""Linked machines (spec section 6): machines whose switchboard-node keeps a link open to the hub, so the server
reaches them through the hub instead of ssh.

The hub owns pairing and the links (native/crates/sb-hub/src/nodeapi.rs). The server:
- adds paired machines to the fleet before each poll (sync), with the target "node:<name>"; one the hub stops
  listing stays, offline, until it is removed in Settings;
- sends their RPCs through the hub (transport: POST http://127.0.0.1:8132/internal/rpc/<name>), ssh for the rest;
- answers POST /api/pair (Add a machine) with a code from the hub and the command to run on the new machine.
"""
import ipaddress, json, os, re, urllib.error, urllib.request
from pathlib import Path
from urllib.parse import urlsplit

import fleet as fleet_mod
import releases as releases_mod
from term import HUB_PORT, ws_host

HUB = (os.environ.get('SWITCHBOARD_HUB_URL') or f'http://127.0.0.1:{HUB_PORT}').rstrip('/')
PREFIX = 'node:'


_direct = None


def direct_open(req, timeout=None):
    """urlopen for the hub on this computer: straight there, never through a proxy. urllib takes http_proxy from
    the environment, and with no no_proxy beside it a call to 127.0.0.1 went to the proxy, which cannot reach it."""
    global _direct
    if _direct is None:
        _direct = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    return _direct.open(req, timeout=timeout)


def hub_call(method, path, body=None, timeout=10, hub=None):
    """JSON from the hub's /internal routes; OSError with the hub's own words when it says no.

    fleet.MaybeDelivered (an OSError) when the request went out and no answer came back: the hub's 504 (the machine
    did not answer in time), its 503 for a link that dropped mid-call, or the hub itself going quiet. The machine may
    have acted on those. A machine that is simply not linked (the other 503) never got anything."""
    data = None if body is None else json.dumps(body).encode()
    req = urllib.request.Request((hub or HUB) + path, data=data, method=method, headers={'Content-Type': 'application/json'})
    try:
        with direct_open(req, timeout=timeout) as r:
            return json.loads(r.read() or b'{}')
    except urllib.error.HTTPError as e:
        try:
            v = json.loads(e.read() or b'{}')
        except (ValueError, OSError):
            v = {}
        e.close()
        msg = (v.get('error') if isinstance(v, dict) else None) or f'the hub answered {e.code}'
        midway = e.code == 504 or (e.code == 503 and 'not linked' not in msg)
        raise (fleet_mod.MaybeDelivered if midway else OSError)(msg)
    except urllib.error.URLError as e:  # the request never left: nothing listens there, or it could not be sent
        raise OSError(f'the hub is not reachable ({getattr(e, "reason", e)})')
    except (TimeoutError, ConnectionError) as e:  # sent, and then the hub went quiet or away
        raise fleet_mod.MaybeDelivered(f'the hub did not answer ({e or type(e).__name__})')
    except ValueError:
        raise fleet_mod.MaybeDelivered("the hub's answer could not be read")


def transport(target, req, timeout, hub=None):
    """fleet.ssh_transport, except "node:<name>" targets, which go over that machine's link through the hub."""
    if target.startswith(PREFIX):
        return hub_call('POST', f'/internal/rpc/{target[len(PREFIX):]}?timeout={max(1, int(timeout))}', req,
                        timeout=timeout + 5, hub=hub)
    return fleet_mod.ssh_transport(target, req, timeout)


NOT_LINKED = 'not linked to the hub any more. Link it again from Add a machine, or remove it in Settings.'


def sync(fleet, hub=None):
    """Bring the fleet in line with the hub's paired machines: add new ones and mark the ones the hub no longer
    lists as not linked. An ssh machine always keeps its name (the hub refuses to pair it). True when something
    changed; None when the hub could not be asked, and then nothing changes.

    A linked machine the hub does not list is never dropped here: the hub answers with an empty list when it lost
    its own file, and dropping would throw away the machine's name, folder and place in the list. It stays, offline,
    and picks up where it was when it is linked again. Only Settings removes a machine (server.machines_post)."""
    try:
        r = hub_call('GET', '/internal/nodes', timeout=3, hub=hub)
    except OSError:
        return None
    nodes = r.get('nodes') if isinstance(r, dict) else None
    if not isinstance(nodes, list):
        return None
    want = {n['name']: n for n in nodes if isinstance(n, dict) and isinstance(n.get('name'), str)}
    changed = False
    seen = fleet.__dict__.setdefault('node_links', {})  # name -> when the hub says its current link came up
    # Machines the hub does not list are not asked (fleet._poll_one): the hub would only say "No linked machine is
    # named X.", which would replace the line below that says what to do about it.
    gone = fleet.unlinked
    gone.difference_update(want)
    for name, n in want.items():
        try:  # one machine that cannot be taken (an odd name, a full list) never stops the others
            text = lambda k: n[k] if isinstance(n.get(k), str) else ''
            m = {'name': name, 'label': text('label') or name, 'target': PREFIX + name, 'os': text('os')}
            changed |= fleet.add_machine(m, by_user=False)
            if n.get('online') and seen.get(name) != n.get('since') and (fleet.machines.get(name) or {}).get('target') == m['target']:
                seen[name] = n.get('since')
                fleet.poll_one(name)  # just linked (or linked again): it shows now, not at the next poll
            elif n.get('online') is False and (fleet.state.get(name) or {}).get('online') and name in seen:
                seen.pop(name)
                fleet.store(name, {'online': False, 'error': 'its link to the hub dropped'})  # offline now, not at the next poll
        except Exception:
            continue
    for name in list(fleet.order):
        m = fleet.machines.get(name) or {}  # the poller and the linker both sync: it may be gone already
        if str(m.get('target') or '').startswith(PREFIX) and name not in want:
            seen.pop(name, None)  # so it is asked at once when the hub lists it again
            st = fleet.state.get(name) or {}
            if name not in gone or st.get('online') is not False or st.get('error') != NOT_LINKED:
                gone.add(name)
                fleet.store(name, {'online': False, 'error': NOT_LINKED})
    return changed


# The installers put the server in <base>/app/server and what other machines download in <base>/downloads/node, served
# on DOWNLOADS_PORT (install-server.sh). That folder always holds this server's own version of the node and agent.
DOWNLOADS_PORT = 8131
KIT_NODE = Path(__file__).resolve().parents[2] / 'downloads' / 'node'


def downloads_base(cfg_dir, host=None, kit_node=KIT_NODE):
    """Where the node and the agent download from, first that says: SWITCHBOARD_DOWNLOADS; <config>/downloads.json's
    "node"; this server's own downloads on `host`:8131, when the installer set them up; the "node" folder next to
    downloads.json's "url" (publish.sh writes the app page there, publish-node.sh the node beside it); the newest
    GitHub release (releases.py). None when nothing does."""
    if os.environ.get('SWITCHBOARD_DOWNLOADS'):
        return os.environ['SWITCHBOARD_DOWNLOADS'].rstrip('/')
    try:
        d = json.loads((Path(cfg_dir) / 'downloads.json').read_text(encoding='utf-8')) if cfg_dir else {}
    except (OSError, ValueError):
        d = {}
    d = d if isinstance(d, dict) else {}
    node, url = d.get('node'), d.get('url')
    if not isinstance(node, str) and host and all((Path(kit_node) / f).is_file() for f in KIT_FILES):
        node = f'http://{host}:{DOWNLOADS_PORT}/node'
    if not isinstance(node, str) and isinstance(url, str) and url.rstrip('/').endswith('/app'):
        node = url.rstrip('/')[:-len('/app')] + '/node'
    if not isinstance(node, str) and releases_mod.repo():
        node = f'https://github.com/{releases_mod.repo()}/releases/latest/download'
    return node.rstrip('/') if isinstance(node, str) and re.fullmatch(r'https?://[A-Za-z0-9.:\[\]-]+(?:/[A-Za-z0-9._/-]*)?', node) else None


KIT_FILES = ('sb.py', 'switchboard-node-mac', 'switchboard-node-linux', 'switchboard-node.exe')


def install_lines(addr, code, base):
    """One line per OS, pasted on the new machine: download the node, pair it, run it at login, and put the agent in
    place only when the machine has none (an agent already there is never replaced)."""
    # a fresh mktemp file, never a fixed /tmp name another local user could have put there first
    unix = lambda os_: (f'n=$(mktemp) && curl -fsSL {base}/switchboard-node-{os_} -o "$n" && chmod +x "$n"'
                        f' && "$n" pair {addr} {code} && "$n" install'
                        f' && (test -f ~/.claude/switchboard/sb.py || (mkdir -p ~/.claude/switchboard'
                        f' && curl -fsSL {base}/sb.py -o ~/.claude/switchboard/sb.py && python3 ~/.claude/switchboard/sb.py install))')
    win = (f'$n="$env:TEMP\\switchboard-node.exe"; iwr -UseBasicParsing {base}/switchboard-node.exe -OutFile $n; & $n pair {addr} {code};'
           f' if ($?) {{ & $n install; $s="$HOME\\.claude\\switchboard\\sb.py"; if (-not (Test-Path $s)) {{'
           f' New-Item -ItemType Directory -Force (Split-Path $s) | Out-Null; iwr -UseBasicParsing {base}/sb.py -OutFile $s; python $s install }} }}')
    return {'mac': unix('mac'), 'linux': unix('linux'), 'windows': win}


def agent_lines(host, code):
    """The same link through this server's own install script (install-agent.sh and .ps1 in its downloads): it also
    brings Switchboard's own Python when the new computer has none, and checks each download against SHA256SUMS."""
    dl = f'http://{host}:{DOWNLOADS_PORT}'
    unix = f'curl -fsSL {dl}/install-agent.sh | bash -s -- --server {host} --code {code}'
    win = f'& ([scriptblock]::Create((iwr -UseBasicParsing {dl}/install-agent.ps1).Content)) -Server {host} -Code {code}'
    return {'mac': unix, 'linux': unix, 'windows': win}


def _loopback(h):
    try:
        return h.strip('[]') == 'localhost' or ipaddress.ip_address(h.strip('[]')).is_loopback
    except ValueError:
        return False


def pair(host, hub=None, cfg_dir=None, origins=(), kit_node=KIT_NODE):
    """POST /api/pair (Add a machine): a code good for 10 minutes and one machine, and the line that links it."""
    r = hub_call('POST', '/internal/pair', {}, hub=hub)
    h = ws_host(host)
    if _loopback(h):  # asked on the server's own machine: the new machine must not dial itself, so use an address of ours
        own = [urlsplit(o).hostname for o in origins if urlsplit(o).hostname and not _loopback(urlsplit(o).hostname)]
        h = (f'[{own[0]}]' if ':' in own[0] else own[0]) if own else h
    addr = f'{h}:{HUB_PORT}'
    base, line = downloads_base(cfg_dir, h, kit_node), f'switchboard-node pair {addr} {r["code"]}'
    out = {'ok': True, 'code': r['code'], 'expires': r['expires'], 'hub': addr, 'pair': line}
    scripts = Path(kit_node).parent
    if base == f'http://{h}:{DOWNLOADS_PORT}/node' and all((scripts / f).is_file() for f in ('install-agent.sh', 'install-agent.ps1')):
        return dict(out, install=agent_lines(h, r['code']))  # this server's own downloads, with its install script
    if base:
        return dict(out, install=install_lines(addr, r['code'], base))
    return dict(out, install={'mac': line, 'linux': line, 'windows': line},  # nowhere to download from: the pair line alone
                note='Install switchboard-node on the new machine first, then run this line there.')


# ---------------------------------------------------------------- the node's control API (keys)
#
# The node on this computer answers on 127.0.0.1:8137 once keys are on (the control API, section 3.5 of
# docs/plans/2026-11-22-keys-phase-1-2.md; native/crates/sb-node/src/control.rs). What is here is the server's side, a
# thin proxy to it. With keys off, or no node, every call below raises OSError and the routes that use them say the
# node did not answer.
#
# The routes it uses:
#   GET  /status          -> {"ok": true, "machine": "<this computer's name in the list, when it has one>", ...}
#   GET  /devices         -> {"ok": true, "devices": [{"id", "name", "kind": "computer"|"phone", "machine"?,
#                             "paired": <time>, "via": "code"|"link"|"upgrade",
#                             "last_seen": <time or null>, "path": "direct"|"relay"|null,
#                             "ver"?: "<the version its helper runs>", "answers_away"?: <whether it took the
#                             home computer's list of devices, so it can answer while home is away>}],
#                             "roster"?: {"epoch", "seq", "at", "expires"} (the list of devices as last signed)}
#   POST /pair/new    {}  -> {"ok": true, "code": "ABCD-EFGH", "link": "switchboard://pair/1?...", "expires": <time>}
#                            ("code" is null when the home has no relay: the link alone pairs the device)
#   POST /devices/remove {"id": "<device id>"} -> {"ok": true}
# The server's routes for these are GET and POST /api/keys/devices and POST /api/pair_device (the plan says
# /api/devices, but POST /api/devices is already where a phone registers for notifications).
# The control API refuses a request with an Origin, a Host that is not this computer, or a POST that is not JSON
# (as the hub's internal routes do), so nothing is sent on from the client's own headers.

CONTROL_PORT = 8137
CONTROL = (os.environ.get('SWITCHBOARD_NODE_CONTROL') or f'http://127.0.0.1:{CONTROL_PORT}').rstrip('/')
DEVICE_ID_RE = re.compile(r'[A-Za-z0-9_-]{8,128}')
DEVICE_KEYS = ('id', 'name', 'kind', 'machine', 'paired', 'via', 'last_seen', 'path')
ROSTER_KEYS = ('epoch', 'seq', 'at', 'expires')
PAIR_KEYS = ('code', 'link', 'expires')


def control_call(method, path, body=None, timeout=10, control=None):
    """JSON from the node's control API on this computer; OSError with the node's own words when it says no, or
    when nothing answers there."""
    base = (control or CONTROL).rstrip('/')
    u = urlsplit(base)
    try:
        local = u.scheme == 'http' and ipaddress.ip_address(u.hostname or '').is_loopback
    except ValueError:
        local = False
    if not local:  # the control API is this computer's own: a setting that points elsewhere is not followed
        raise OSError('the node address is not on this computer')
    data = None if body is None else json.dumps(body).encode()
    req = urllib.request.Request(base + path, data=data, method=method, headers={'Content-Type': 'application/json'})
    try:
        with direct_open(req, timeout=timeout) as r:
            v = json.loads(r.read(1_000_000) or b'{}')
    except urllib.error.HTTPError as e:
        try:
            v = json.loads(e.read(100_000) or b'{}')
        except (ValueError, OSError):
            v = {}
        e.close()
        raise OSError(str((v.get('error') if isinstance(v, dict) else None) or f'the node answered {e.code}')[:200])
    except urllib.error.URLError as e:
        raise OSError(f'the node is not reachable ({getattr(e, "reason", e)})')
    except (TimeoutError, ConnectionError) as e:
        raise OSError(f'the node did not answer ({e or type(e).__name__})')
    except ValueError:
        raise OSError("the node's answer could not be read")
    if not isinstance(v, dict):
        raise OSError("the node's answer could not be read")
    return v


def node_status(control=None):
    """The control API's GET /status, as it is."""
    return control_call('GET', '/status', timeout=5, control=control)


def devices(control=None):
    """GET /api/keys/devices: {"ok": true, "devices": [...], "roster": {...} or null}, the devices paired with this
    home computer, and when it last signed its list of them. Only the fields named above are passed on, so nothing
    else the node keeps beside a device can reach an app. "ver" and "answers_away" are there only when the node
    knows them (keys phase 4a): a text and a true or false."""
    v = control_call('GET', '/devices', timeout=5, control=control)
    rows = v.get('devices')
    if not isinstance(rows, list):
        raise OSError("the node's answer could not be read")
    out = []
    for d in rows:
        if not (isinstance(d, dict) and isinstance(d.get('id'), str)):
            continue
        row = {k: d[k] for k in DEVICE_KEYS if k in d}
        if isinstance(d.get('ver'), str) and d['ver']:
            row['ver'] = d['ver'][:40]
        if isinstance(d.get('answers_away'), bool):
            row['answers_away'] = d['answers_away']
        out.append(row)
    number = lambda x: isinstance(x, (int, float)) and not isinstance(x, bool)
    r = v.get('roster')
    roster = {k: r[k] for k in ROSTER_KEYS} if isinstance(r, dict) and all(number(r.get(k)) for k in ROSTER_KEYS) else None
    return {'ok': True, 'devices': out, 'roster': roster}


def pair_device(control=None):
    """POST /api/pair_device: {"ok": true, "code", "link", "expires"}, a code and a link that pair one device.
    "code" is null when this home computer has no relay: the link still pairs it."""
    v = control_call('POST', '/pair/new', {}, control=control)
    if not isinstance(v.get('link'), str) or not (v.get('code') is None or isinstance(v.get('code'), str)):
        raise OSError("the node's answer could not be read")
    v.setdefault('code', None)
    return dict({'ok': True}, **{k: v[k] for k in PAIR_KEYS if k in v})


def device_remove(dev, control=None):
    """POST /api/keys/devices {"remove": id}: that device's key stops working."""
    control_call('POST', '/devices/remove', {'id': dev}, control=control)
    return {'ok': True}
