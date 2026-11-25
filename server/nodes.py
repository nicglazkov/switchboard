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
    if base:
        return dict(out, install=install_lines(addr, r['code'], base))
    return dict(out, install={'mac': line, 'linux': line, 'windows': line},  # nowhere to download from: the pair line alone
                note='Install switchboard-node on the new machine first, then run this line there.')
