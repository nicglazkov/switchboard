"""Keys: how the other computers reach this one (docs/plans/2026-11-22-keys-phase-1-2.md, sections 3.4 and 3.8).

server.json's "keys" is one of:
- "off": by address, as before keys. An install whose server.json has no "keys" entry is this, and nothing changes.
- "both": by address, and through the tunnel the node on this computer runs for paired computers.
- "only": through the tunnel only. The server listens on 127.0.0.1 alone, is not announced on the network, lists no
  addresses for the apps to remember, and the downloads on 8131 are not served.

A value this server does not know ("onyl", a number, a mode from a newer version) is not guessed at: it counts as
"only", the choice that opens nothing to the network, and the log says so.

Also here: when each kind of client last asked by address (Direct), and the report that says whether switching to
"only" would cut anything off (readiness). The server and the hub read the mode when they start and the home node
looks again every few seconds, so changing it is a write of server.json (set_mode) and a restart.

Run as a program it is what `switchboard keys on|only|off|status` calls (main, at the end): it writes the mode and says
what happens next. The restart itself is the caller's.
"""
import argparse
import json
import os
import sys
import threading
import time
from pathlib import Path

import durable

OFF, BOTH, ONLY = 'off', 'both', 'only'
MODES = (OFF, BOTH, ONLY)
READY_DAYS = 7      # days with no request by address before "only" is safe (server.json's "keys_ready_days" says another)
DAYS_MIN, DAYS_MAX = 1, 90
SAVE_EVERY = 600    # seconds between writes of a client kind's last time: a crash loses at most this much
_MISSING = object()


def mode_of(d):
    """(mode, known) from server.json's contents. No "keys" entry is "off": an install from before keys."""
    v = d.get('keys', _MISSING) if isinstance(d, dict) else _MISSING
    if v is _MISSING:
        return OFF, True
    if isinstance(v, str) and v in MODES:
        return v, True
    return ONLY, False


def days_of(d):
    """How many days with no request by address make "only" safe: server.json's "keys_ready_days", a whole number
    kept to 1..90, or READY_DAYS when it is left out or is anything else."""
    v = d.get('keys_ready_days') if isinstance(d, dict) else None
    if not isinstance(v, int) or isinstance(v, bool):
        return READY_DAYS
    return min(DAYS_MAX, max(DAYS_MIN, v))


def read_days(path):
    """days_of for the server.json at `path`, read as read_mode reads it. READY_DAYS when nothing there says."""
    d = _read(Path(path))
    return days_of(d if isinstance(d, dict) else None)


def _read(path):
    """The server.json at `path` as read_mode reads it: its contents, None when there is no file and none put
    aside, False when it does not read and has no good copy."""
    def obj(p):
        try:
            raw = p.read_bytes()
        except FileNotFoundError:
            return None
        except OSError:
            return False
        try:
            d = json.loads(raw.decode('utf-8'))
        except ValueError:
            return False
        return d if isinstance(d, dict) else False

    d = obj(path)
    if d is None and not durable._bads(path):
        return None
    if not isinstance(d, dict):
        d = obj(path.with_name(path.name + '.bak'))
    return d if isinstance(d, dict) else False


def read_mode(path):
    """The mode in the server.json at `path`, for a program beside the server (downloads.py). Nothing on disk is
    changed: the server owns that file. Read the way the server and the hub read it (durable.py; sb-hub config.rs
    read_keys): a file that does not read goes back to its .bak, and with no good copy the answer is "only"."""
    d = _read(Path(path))
    if d is None:
        return OFF  # no file, and none put aside: an install that never chose
    return mode_of(d)[0] if isinstance(d, dict) else ONLY


# ---------------------------------------------------------------- who still asks by address

KINDS = ('iPhone', 'Android', 'desktop', 'helper', 'web', 'other')
KIND_WORDS = {'iPhone': 'The iPhone app', 'Android': 'The Android app', 'desktop': 'The desktop app',
              'helper': "A computer's helper", 'web': 'A browser', 'other': 'Another program'}
PHONES = ('iPhone', 'Android')


def client_kind(ua):
    """Which kind of client sent a request, from its User-Agent (the same split the server's routes use)."""
    ua = ua or ''
    if 'SwitchboardApp' in ua:
        return 'iPhone'
    if 'SwitchboardAndroid' in ua:
        return 'Android'
    if 'SwitchboardDesktop' in ua:
        return 'desktop'
    if ua.startswith('Python-urllib'):
        return 'helper'  # sb.py's hooks and its MCP tools
    return 'web' if 'Mozilla' in ua else 'other'


def is_loopback(addr):
    """Whether a request's source address is this computer itself. Everything through the tunnel is: the node
    forwards it to 127.0.0.1."""
    a = str(addr or '').split('%')[0].lower()
    if a.startswith('::ffff:'):
        a = a[7:]
    return a.startswith('127.') or a == '::1'


class Direct:
    """When each kind of client last asked from an address that is not this computer: a request that came by
    address, not through the tunnel. Kept in <state>/direct_seen.json as {"since": t, "seen": {kind: t}}, where
    "since" is when the watching began. In memory on every request; written when a kind is first seen and then at
    most every SAVE_EVERY seconds for each kind."""

    def __init__(self, path, clock=time.time):
        self.path, self.clock = Path(path), clock
        self._lock = threading.Lock()
        self._saved = {}  # kind -> the time that is on disk
        d, status = durable.load(self.path, None, dict)
        seen = d.get('seen') if isinstance(d, dict) else None
        since = d.get('since') if isinstance(d, dict) else None
        num = lambda v: isinstance(v, (int, float)) and not isinstance(v, bool) and v > 0
        self.seen = {k: float(v) for k, v in (seen if isinstance(seen, dict) else {}).items() if k in KINDS and num(v)}
        self._saved = dict(self.seen)
        if num(since):
            self.since = float(since)
        else:
            # No record of when the watching began (a first start, or the file was lost): it begins now. A lost
            # file is written over on purpose, since what it held cannot be had back and an empty watch is the
            # careful answer: readiness waits the full days again.
            self.since = float(self.clock())
            self._write(force=status == durable.BROKEN)

    def _write(self, force=False):
        try:
            self.path.parent.mkdir(parents=True, exist_ok=True)
            durable.put(self.path, json.dumps({'since': self.since, 'seen': self.seen}, indent=1), force=force)
            self._saved = dict(self.seen)
        except OSError:
            pass  # still right in memory, and the next change tries again

    def note(self, addr, ua):
        """One answered request. Does nothing for one from this computer."""
        if is_loopback(addr):
            return
        kind, now = client_kind(ua), float(self.clock())
        with self._lock:
            self.seen[kind] = now
            if now - self._saved.get(kind, float('-inf')) >= SAVE_EVERY:
                self._write()

    def flush(self):
        with self._lock:
            if self.seen != self._saved:
                self._write()

    def view(self):
        with self._lock:
            return {'since': self.since, 'seen': dict(self.seen)}


# ---------------------------------------------------------------- is "only" safe yet

def ago(sec):
    sec = max(0, int(sec))
    for size, word in ((86400, 'day'), (3600, 'hour'), (60, 'minute')):
        if sec >= size:
            n = sec // size
            return f'{n} {word}{"" if n == 1 else "s"} ago'
    return 'just now'


def readiness(mode, machines, devices, direct, now, days=READY_DAYS, home=None, node_error=None, settings_lost=False):
    """Whether switching to "only" would cut anything off, worked out from what was seen and never assumed.

    machines: [{"name", "label", "gone"?}], every computer in the list ("gone": its link was removed, so the hub no
    longer lists it). devices: the paired devices as the home node's
    control API lists them (nodes.devices), or None when it did not answer (node_error says why). direct: a
    Direct.view(). home: the name of this computer's own entry in `machines`, when the node says which it is; it
    needs no tunnel to reach itself.

    The answer's "phones" lists the paired phones ({"name", "seen"}); a "direct" blocker for a phone app carries
    "paired_since": true when a paired phone has come through the tunnel after that request by address.

    Ready means no blockers:
    - every other computer has a paired device of its own that has been seen through the tunnel. A computer whose
      link was removed and that has no paired device is not waited for: it has no connection left to lose, and its
      row says "removed" (the same computer paired again is another row, with a name of its own);
    - nothing has asked by address for `days` days, and the watching has gone on for that long.
    """
    blockers = []
    block = lambda kind, text, **more: blockers.append(dict({'kind': kind, 'text': text}, **more))
    if settings_lost:
        block('settings', 'server.json could not be read, so this home computer answers itself only for now. '
                          'Put a good server.json back before changing how computers connect.')
    if mode == OFF:
        block('mode', f'Keys are off, so no computer connects with a key yet. Run "{cmd("keys on")}" on this '
                      'home computer first.')
    if devices is None:
        block('node', 'The node on this home computer did not answer'
                      + (f' ({node_error})' if node_error else '') + ', so paired computers cannot be checked.')
    by_machine = {}
    for d in devices or []:
        if isinstance(d, dict) and d.get('kind') == 'computer' and isinstance(d.get('machine'), str):
            best = by_machine.get(d['machine'])
            if best is None or (d.get('last_seen') or 0) > (best.get('last_seen') or 0):
                by_machine[d['machine']] = d
    rows = []
    for m in machines:
        name, label = m['name'], m.get('label') or m['name']
        d = by_machine.get(name)
        seen = d.get('last_seen') if d else None
        seen = seen if isinstance(seen, (int, float)) and not isinstance(seen, bool) and seen > 0 else None
        mine = home is not None and name == home
        removed = bool(m.get('gone')) and d is None and not mine and devices is not None
        # ("ver": the version its helper runs; "away": whether it can answer for its own chats while this home
        # computer is away, None when the node has not said)
        rows.append({'name': name, 'label': label, 'home': mine, 'paired': d is not None, 'seen': seen,
                     'path': d.get('path') if d and d.get('path') in ('direct', 'relay') else None, 'removed': removed,
                     'ver': d.get('ver') if d and not mine and isinstance(d.get('ver'), str) else None,
                     'away': d.get('answers_away') if d and not mine and isinstance(d.get('answers_away'), bool) else None})
        if mine or devices is None or removed:
            continue
        if d is None:
            block('machine', f'{label} is not paired with this home computer. Pair it with a code first, or it '
                             'loses its connection.', machine=name, label=label, paired=False)
        elif seen is None:
            block('machine', f'{label} is paired but has not connected with its key yet.',
                  machine=name, label=label, paired=True)
    # Phones pair as computers do, but are nobody's row in the list of computers: each is listed, and none is waited
    # for (a phone that still comes by address shows under "direct" below).
    phones = []
    for d in devices or []:
        if isinstance(d, dict) and d.get('kind') == 'phone':
            seen = d.get('last_seen')
            seen = seen if isinstance(seen, (int, float)) and not isinstance(seen, bool) and seen > 0 else None
            phones.append({'name': d.get('name') if isinstance(d.get('name'), str) and d.get('name') else 'Phone',
                           'seen': seen})
    since = direct.get('since') or now
    watched = max(0.0, now - since)
    if watched < days * 86400:
        n = int(watched // 86400)
        block('watching', f'Connections by address have been watched for {n} of {days} days. '
                          'Until then nobody can say that none is still in use.', watched_days=n)
    recent = []
    for kind in KINDS:
        last = (direct.get('seen') or {}).get(kind)
        if not last:
            continue
        recent.append({'client': kind, 'last': last})
        if now - last < days * 86400:
            text = f'{KIND_WORDS[kind]} connected by address {ago(now - last)}.'
            if kind not in PHONES:
                block('direct', text + ' It would stop working.', client=kind, last=last)
            elif any(p['seen'] is not None and p['seen'] > last for p in phones):
                # (which phone asked by address is not known, so this still counts: only time clears it)
                block('direct', text + ' A paired phone has connected with its key since, so this may be that phone '
                                       'before it was paired. A phone that is not paired would stop working.',
                      client=kind, last=last, paired_since=True)
            else:
                block('direct', text + ' It would stop working. Pair the phone with this home computer in the app '
                                       'first.', client=kind, last=last)
    return {'ok': True, 'keys': mode, 'ready': not blockers, 'days': days, 'watching_since': since, 'now': now,
            'blockers': blockers, 'machines': rows, 'phones': phones, 'direct': recent}


# ---------------------------------------------------------------- changing the mode

def set_mode(path, want):
    """Writes "keys": `want` into the server.json at `path`, durably, and leaves the rest of the file as it is.
    Returns the mode it had. RuntimeError when there is no server.json to change, or it cannot be read: this never
    makes one, because the server fills in the addresses it listens on the first time it starts."""
    if not isinstance(want, str) or want not in MODES:
        raise ValueError('The mode is "off", "both" or "only".')
    path = Path(path)
    d, status = durable.load(path, None, dict)
    if status == durable.BROKEN or (d is None and durable._bads(path)):
        raise RuntimeError('server.json could not be read and has no good copy. Put a good server.json back first.')
    if d is None:
        raise RuntimeError(f'There is no server.json yet. Start Switchboard once first: {cmd("start")}')
    before = mode_of(d)[0]
    # fresh: the copy beside it is of the new file, so a damaged server.json never goes back to a mode that was left
    durable.put(path, json.dumps(dict(d, keys=want), indent=1), fresh=True)
    return before


def by_hand(stuck, want, config, here):
    """(the sentence, the command or None) for a server that was asked to change the mode and cannot restart itself
    (serverctl.stuck says why: "windows", "checkout" or "command"). One message: nothing changed, why, and what to
    run on the home computer instead. The sentence ends with the command, which an app shows apart with Copy.
    `config` is this server's server.json and `here` the folder its code runs from."""
    word = {BOTH: 'on'}.get(want, want)
    aim = {BOTH: 'To turn keys on', OFF: 'To turn keys off', ONLY: 'To switch to keys only'}[want]
    if stuck == 'windows':
        command = f'& "$env:LOCALAPPDATA\\Switchboard\\switchboard.ps1" keys {word}'
        return ('Nothing was changed. On Windows the server cannot restart itself from the app. '
                f'{aim}, run this in PowerShell on the home computer: {command}'), command
    if stuck == 'checkout':
        import shlex
        command = f"python3 {shlex.quote(str(Path(here) / 'keys.py'))} {word}"
        folder = Path(config).parent
        if folder != Path(os.environ.get('SWITCHBOARD_CONFIG_DIR') or Path.home() / '.config' / 'switchboard'):
            command += f' --config {shlex.quote(str(folder))}'
        return ('Nothing was changed. This server was started by hand from a copy of the code, so it cannot restart '
                f'itself. {aim}, run this on the home computer, then start the server again: {command}'), command
    return ('Nothing was changed. This server cannot restart itself, because the switchboard command is not in '
            '~/.local/bin on the home computer. Run the install line there once, then try again.'), None


WINDOWS = os.name == 'nt'


def cmd(rest):
    """The line to type for `switchboard <rest>` on this computer. A PC has no `switchboard` command: the installer
    puts switchboard.ps1 there (scripts/install/switchboard.ps1), which is what its own sentences call it."""
    return ('switchboard.ps1 ' if WINDOWS else 'switchboard ') + rest


SAME = 3  # main's exit code when the mode asked for is the one it has: nothing to restart
SAYS = {OFF: 'Keys: off. Computers reach this home computer by address.',
        BOTH: 'Keys: on. Computers reach this home computer with a key or by address.',
        ONLY: 'Keys: only. Computers reach this home computer with a key, and its ports are closed to the network.'}
ALREADY = {OFF: 'Keys are already off.', BOTH: 'Keys are already on.', ONLY: 'This home computer already uses keys only.'}


def _ask(url, timeout=5):
    """The JSON at `url` on this computer, never through a proxy. OSError when nothing answers."""
    import urllib.request
    try:
        with urllib.request.build_opener(urllib.request.ProxyHandler({})).open(url, timeout=timeout) as r:
            v = json.loads(r.read(2_000_000))
    except ValueError:
        raise OSError('its answer could not be read')
    except Exception as e:  # URLError, a timeout, a reset: all the same to the person reading
        raise OSError(str(getattr(e, 'reason', e)) or type(e).__name__)
    if not isinstance(v, dict):
        raise OSError('its answer could not be read')
    return v


def _report(server):
    """(the readiness report, None) or (None, why the server did not give one)."""
    try:
        return _ask(server.rstrip('/') + '/api/keys/readiness'), None
    except OSError as e:
        return None, f'The server on this computer did not answer ({e})'


def _row(m, now):
    label = str(m.get('label') or m.get('name') or '?')
    if m.get('home'):
        return f'  {label}  this home computer'
    if m.get('removed'):
        return f'  {label}  removed, so it is not waited for'
    if not m.get('paired'):
        return f'  {label}  not paired'
    seen = m.get('seen')
    away = {True: '; can answer for its own chats while this home computer is away',
            False: '; needs an update before it can answer while this home computer is away'}.get(m.get('away'), '')
    if not isinstance(seen, (int, float)) or isinstance(seen, bool):
        return f'  {label}  paired, has not connected with its key yet{away}'
    how = {'relay': ' (through the relay)', 'direct': ' (directly)'}.get(m.get('path'), '')
    return f'  {label}  paired, last connected with its key {ago(now - seen)}{how}{away}'


def _blockers(r):
    return '\n'.join(f"  {b.get('text')}" for b in r.get('blockers') or [] if isinstance(b, dict) and b.get('text'))


def _status(path, server):
    mode = read_mode(path)
    print(SAYS[mode])
    if mode == OFF:
        print(f'To turn keys on: {cmd("keys on")}')
        return 0
    r, why = _report(server)
    if r is None:
        print(f'{why}, so which computers are paired is not known. Start it with: {cmd("start")}')
        return 0
    now = r.get('now') if isinstance(r.get('now'), (int, float)) else time.time()
    for m in r.get('machines') or []:
        if isinstance(m, dict):
            print(_row(m, now))
    if mode == ONLY:
        print(f'To open the addresses again: {cmd("keys on")}')
    elif r.get('ready') is True:
        print(f'Ready for keys only. To close the ports to the network: {cmd("keys only")}')
    else:
        print('Not ready for keys only yet:')
        print(_blockers(r))
        print(f'To pair a computer: {cmd("code")}')
    return 0


def _set(path, want, server, force):
    if read_mode(path) == want:
        print(ALREADY[want])
        return SAME
    if want == ONLY:
        r, why = _report(server)
        ready = r is not None and r.get('ready') is True
        if not ready and not force:
            if r is None:
                print(f'{why}, so nobody can say what keys only would cut off.')
            else:
                print('Not yet: switching to keys only now would cut something off.')
                print(_blockers(r))
            print(f'Nothing was changed. To switch anyway: {cmd("keys only --force")}')
            return 1
        if not ready:
            if r is None:
                print(f'Warning: switching to keys only although nobody can say what it cuts off. {why}.')
            else:
                print('Warning: switching to keys only although something may be cut off:')
                print(_blockers(r))
            print(f'To go back: {cmd("keys on")}')
    try:
        set_mode(path, want)
    except (RuntimeError, OSError) as e:
        print(f'switchboard: {e}', file=sys.stderr)
        return 1
    return 0


def _wait(path, control, within):
    """After the restart: what is true now, and the next step. With keys on, once the node on this computer answers."""
    mode = read_mode(path)
    if mode == OFF:
        print('Keys are off. Computers reach this home computer by address, as before.')
        return 0
    import nodes  # the control API's own caller: this computer only, never through a proxy
    end = time.time() + within
    while True:
        try:
            nodes.control_call('GET', '/status', timeout=3, control=control)
            break
        except OSError:
            if time.time() >= end:
                print(('Keys are on' if mode == BOTH else 'Keys only is set') + ' in server.json, but the node on this home '
                      'computer did not answer yet.\nCheck it with: switchboard-node status')
                return 1
            time.sleep(0.5)
    if mode == BOTH:
        print('Keys are on. This home computer still answers by address as well.\n'
              f'Next: run "{cmd("code")}" here, and on each other computer run the line it prints.\n'
              f'"{cmd("keys status")}" says when it is safe to close the ports with "{cmd("keys only")}".')
    else:
        print('This home computer now uses keys only: its ports are closed to the network.\n'
              f'To open the addresses again: {cmd("keys on")}')
    return 0


def main(argv=None):
    ap = argparse.ArgumentParser(prog='switchboard keys', description='How the other computers reach this home computer.')
    ap.add_argument('what', choices=('on', 'only', 'off', 'status', 'wait', 'mode'))
    ap.add_argument('--force', action='store_true', help='with "only": switch although something may be cut off')
    ap.add_argument('--config', default=os.environ.get('SWITCHBOARD_CONFIG_DIR') or str(Path.home() / '.config' / 'switchboard'))
    ap.add_argument('--server', default=f"http://127.0.0.1:{os.environ.get('SWITCHBOARD_PORT') or 8130}")
    ap.add_argument('--control', default=None)
    ap.add_argument('--within', type=float, default=float(os.environ.get('SWITCHBOARD_KEYS_WAIT') or 30))
    a = ap.parse_args(argv)
    path = Path(a.config) / 'server.json'
    if a.what == 'mode':  # one word, for the switchboard command's status
        print(read_mode(path))
        return 0
    if a.what == 'status':
        return _status(path, a.server)
    if a.what == 'wait':
        return _wait(path, a.control, a.within)
    return _set(path, {'on': BOTH, 'only': ONLY, 'off': OFF}[a.what], a.server, a.force)


if __name__ == '__main__':
    sys.exit(main())
