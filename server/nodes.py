"""Linked machines (spec section 6): machines whose switchboard-node keeps a link open to the hub, so the server
reaches them through the hub instead of ssh.

The hub owns pairing and the links (native/crates/sb-hub/src/nodeapi.rs). The server:
- adds paired machines to the fleet before each poll (sync), with the target "node:<name>", and drops unpaired ones;
- sends their RPCs through the hub (transport: POST http://127.0.0.1:8132/internal/rpc/<name>), ssh for the rest;
- answers POST /api/pair (Add a machine) with a code from the hub and the command to run on the new machine.
"""
import ipaddress, json, os, re, urllib.error, urllib.request
from pathlib import Path
from urllib.parse import urlsplit

import fleet as fleet_mod
from term import HUB_PORT, ws_host

HUB = (os.environ.get('SWITCHBOARD_HUB_URL') or f'http://127.0.0.1:{HUB_PORT}').rstrip('/')
PREFIX = 'node:'


def hub_call(method, path, body=None, timeout=10, hub=None):
    """JSON from the hub's /internal routes; OSError with the hub's own words when it says no."""
    data = None if body is None else json.dumps(body).encode()
    req = urllib.request.Request((hub or HUB) + path, data=data, method=method, headers={'Content-Type': 'application/json'})
    try:
        with urllib.request.urlopen(req, timeout=timeout) as r:
            return json.loads(r.read() or b'{}')
    except urllib.error.HTTPError as e:
        try:
            v = json.loads(e.read() or b'{}')
        except ValueError:
            v = {}
        raise OSError(v.get('error') or f'the hub answered {e.code}')
    except (urllib.error.URLError, TimeoutError, ConnectionError) as e:
        raise OSError(f'the hub is not reachable ({getattr(e, "reason", e)})')


def transport(target, req, timeout, hub=None):
    """fleet.ssh_transport, except "node:<name>" targets, which go over that machine's link through the hub."""
    if target.startswith(PREFIX):
        return hub_call('POST', f'/internal/rpc/{target[len(PREFIX):]}?timeout={max(1, int(timeout))}', req,
                        timeout=timeout + 5, hub=hub)
    return fleet_mod.ssh_transport(target, req, timeout)


def sync(fleet, hub=None):
    """Bring the fleet in line with the hub's paired machines: add new ones, follow label changes, drop unpaired
    ones. An ssh machine always keeps its name (the hub refuses to pair it). True when something changed; None when
    the hub could not be asked, and then nothing changes."""
    try:
        nodes = hub_call('GET', '/internal/nodes', timeout=3, hub=hub).get('nodes') or []
    except OSError:
        return None
    want = {n['name']: n for n in nodes if isinstance(n, dict) and isinstance(n.get('name'), str)}
    changed = False
    seen = fleet.__dict__.setdefault('node_links', {})  # name -> when the hub says its current link came up
    for name, n in want.items():
        m = {'name': name, 'label': n.get('label') or name, 'target': PREFIX + name, 'os': n.get('os') or ''}
        changed |= fleet.add_machine(m)
        if n.get('online') and seen.get(name) != n.get('since') and fleet.machines.get(name, {}).get('target') == m['target']:
            seen[name] = n.get('since')
            fleet.poll_one(name)  # just linked (or linked again): it shows now, not at the next poll
        elif n.get('online') is False and (fleet.state.get(name) or {}).get('online') and name in seen:
            seen.pop(name)
            fleet.store(name, {'online': False, 'error': 'its link to the hub dropped'})  # offline now, not at the next poll
    for name in list(fleet.order):
        if fleet.machines[name].get('target', '').startswith(PREFIX) and name not in want:
            changed |= fleet.drop_machine(name)
    return changed


def downloads_base(cfg_dir):
    """Where the node and the agent download from: SWITCHBOARD_DOWNLOADS, else <config>/downloads.json's "node", else
    the "node" folder next to its "url" (publish.sh writes the app page there; publish-node.sh the node beside it).
    None when nothing says: no server but the owner's lab serves a fixed port."""
    if os.environ.get('SWITCHBOARD_DOWNLOADS'):
        return os.environ['SWITCHBOARD_DOWNLOADS'].rstrip('/')
    try:
        d = json.loads((Path(cfg_dir) / 'downloads.json').read_text()) if cfg_dir else {}
    except (OSError, ValueError):
        d = {}
    d = d if isinstance(d, dict) else {}
    node, url = d.get('node'), d.get('url')
    if not isinstance(node, str) and isinstance(url, str) and url.rstrip('/').endswith('/app'):
        node = url.rstrip('/')[:-len('/app')] + '/node'
    return node.rstrip('/') if isinstance(node, str) and re.fullmatch(r'https?://[A-Za-z0-9.:\[\]-]+(?:/[A-Za-z0-9._/-]*)?', node) else None


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


def _loopback(h):
    try:
        return h.strip('[]') == 'localhost' or ipaddress.ip_address(h.strip('[]')).is_loopback
    except ValueError:
        return False


def pair(host, hub=None, cfg_dir=None, origins=()):
    """POST /api/pair (Add a machine): a code good for 10 minutes and one machine, and the line that links it."""
    r = hub_call('POST', '/internal/pair', {}, hub=hub)
    h = ws_host(host)
    if _loopback(h):  # asked on the server's own machine: the new machine must not dial itself, so use an address of ours
        own = [urlsplit(o).hostname for o in origins if urlsplit(o).hostname and not _loopback(urlsplit(o).hostname)]
        h = (f'[{own[0]}]' if ':' in own[0] else own[0]) if own else h
    addr = f'{h}:{HUB_PORT}'
    base, line = downloads_base(cfg_dir), f'switchboard-node pair {addr} {r["code"]}'
    out = {'ok': True, 'code': r['code'], 'expires': r['expires'], 'hub': addr, 'pair': line}
    if base:
        return dict(out, install=install_lines(addr, r['code'], base))
    return dict(out, install={'mac': line, 'linux': line, 'windows': line},  # nowhere to download from: the pair line alone
                note='Install switchboard-node on the new machine first, then run this line there.')
