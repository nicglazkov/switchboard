"""The home computer's state as one file, to move the home computer to another computer or to keep a copy of it.

What is home state: everything in the settings folder and the state folder (server.py `folders`), except what is
this computer's own (its link and key: node.json, device.key, the locks, its logs, the installed programs) and
what is built again when needed (browse_index.json, machines.json). server.json travels without `bind`, `hosts`
and `origins`: those are addresses of the computer it was written on, and the computer it is unpacked on keeps its
own, or has the server work them out at its next start.

The home computer's key is never in this file. Whoever moves or restores the state carries the key by itself
(switchboard-node, home.rs).

The file is a tar. Its first member, `homestate.json`, says which format it is; its last, `end.json`, lists every
file with its size and SHA-256. Unpacking has two steps, and nothing a program reads changes in the first:

  stage    read the whole file into `.home-staging` inside the two folders, and check it against the hash the sender
           named and against `end.json`. An archive that is cut, changed, from a newer version, or that names a
           file outside the two folders or one of this computer's own, is refused and the staging folder removed.
  swap     move what is staged into place, and what was there into `.home-before`. A swap that stopped half way
           is finished by the next one.

Both are refused while a server holds the state folder (server.py takes `server.lock`). The hub takes no lock
of its own: whoever stops the server to move it stops the hub with it (`switchboard stop`).

  python3 homestate.py pack    --config DIR --state DIR --out FILE
  python3 homestate.py stage   --config DIR --state DIR --in FILE [--sha256 HEX]
  python3 homestate.py swap    --config DIR --state DIR
  python3 homestate.py discard --config DIR --state DIR
  python3 homestate.py status  --config DIR --state DIR

Each prints one line of JSON. The two folders are always named: nothing here falls back to the real ones.
"""
import argparse, hashlib, json, os, shutil, sys, tarfile, time
from pathlib import Path

FORMAT = 1
FIRST, LAST = 'homestate.json', 'end.json'
STAGING, BEFORE = '.home-staging', '.home-before'
SCRATCH = (STAGING, BEFORE)
MARKER = 'home-stepdown.json'
LOCKS = ('server.lock',)
STEPPED_DOWN = 'This computer was the home computer until it moved. It is not answering as home.'
# This computer's own, in either folder: never packed, and never taken from an archive.
OWN_NAMES = {
    'node.json', 'blocked.json', 'home-move.json', 'home-arrive.json', 'home-witness.json', MARKER,
    # built again when needed
    'browse_index.json', 'machines.json',
    # the installed programs, their logs and downloads, and where a move or a backup keeps its own files
    'app', 'app.old', 'app.new', 'python', 'python.old', 'downloads', 'logs', 'run', 'update', 'env',
    'home-incoming', 'home-outgoing', 'home-backup', 'own-terminals',
}
OWN_STARTS = ('device.key', 'member.key')
OWN_ENDS = ('.lock', '.log', '.tmp')
SMALL = 8 * 1024 * 1024  # a file up to this size is read whole before it is packed, so a write beside it cannot cut it
ADDRESSES = ('bind', 'hosts', 'origins')


class Refused(Exception):
    """Why nothing was done. `kind` is one word for the program that asked."""

    def __init__(self, kind, msg):
        super().__init__(msg)
        self.kind = kind


def own(parts):
    """Whether a path inside one of the two folders (as its parts) is this computer's own, so it is not home state."""
    top = parts[0]
    base = top[:-4] if top.endswith('.bak') else top
    if base in OWN_NAMES or base.startswith(OWN_STARTS) or base.endswith(OWN_ENDS):
        return True
    return any(p.startswith('.') or '.bad-' in p or p.endswith('.tmp') for p in parts)


def sha256_of(path):
    h = hashlib.sha256()
    with open(path, 'rb') as f:
        for block in iter(lambda: f.read(1 << 20), b''):
            h.update(block)
    return h.hexdigest()


# ---------------------------------------------------------------- who holds the folder

def _try_lock(f):
    try:
        if os.name == 'nt':
            import msvcrt
            f.seek(0)
            msvcrt.locking(f.fileno(), msvcrt.LK_NBLCK, 1)
        else:
            import fcntl
            fcntl.flock(f, fcntl.LOCK_EX | fcntl.LOCK_NB)
        return True
    except OSError:
        return False


def hold(state, name='server.lock'):
    """Takes the lock a running server holds on its state folder. The open file (keep it for as long as the server
    runs), or None when another program has it or it cannot be made."""
    try:
        Path(state).mkdir(parents=True, exist_ok=True)
        f = open(Path(state) / name, 'a+b')
    except OSError:
        return None
    if _try_lock(f):
        return f
    f.close()
    return None


def held(state):
    """Whether a server is running on this state folder now."""
    for name in LOCKS:
        p = Path(state) / name
        if not p.exists():
            continue
        try:
            f = open(p, 'a+b')
        except OSError:
            return True  # there and it will not open: somebody has it
        try:
            if not _try_lock(f):
                return True
        finally:
            f.close()
    return False


def stepped_down(config):
    """The note a computer keeps once it has stopped being the home computer (switchboard-node writes it), or None.
    A note that is there and cannot be read still counts: {'why': 'unreadable'}."""
    p = Path(config) / MARKER
    try:
        raw = p.read_bytes()
    except FileNotFoundError:
        return None
    except OSError:
        return {'why': 'unreadable'}
    try:
        d = json.loads(raw.decode('utf-8'))
        return d if isinstance(d, dict) else {'why': 'unreadable'}
    except ValueError:
        return {'why': 'unreadable'}


# ---------------------------------------------------------------- pack

def _walk(folder):
    """(parts, path) of every file of home state under `folder`, in a fixed order. Links are not followed."""
    out = []

    def go(d, parts):
        try:
            names = sorted(os.listdir(d))
        except OSError:
            return
        for n in names:
            p, here = d / n, parts + (n,)
            if own(here) or p.is_symlink():
                continue
            if p.is_dir():
                go(p, here)
            elif p.is_file():
                out.append((here, p))
    go(Path(folder), ())
    return out


def _version():
    try:
        import serverctl
        return serverctl.version()[0]
    except Exception:
        return ''


def pack(config, state, out, now=None):
    """Writes the home state in `config` and `state` to the file `out`. {sha256, bytes, files}."""
    config, state, out = Path(config), Path(state), Path(out)
    same = config.resolve() == state.resolve()
    files, n = [], 0
    out.parent.mkdir(parents=True, exist_ok=True)
    fd = os.open(out, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    with os.fdopen(fd, 'wb') as raw, tarfile.open(fileobj=raw, mode='w', format=tarfile.PAX_FORMAT) as t:
        def add(name, body=None, src=None):
            info = tarfile.TarInfo(name)
            info.mode, info.mtime = 0o600, int(now or time.time())
            if body is not None:
                info.size = len(body)
                import io
                t.addfile(info, io.BytesIO(body))
                return hashlib.sha256(body).hexdigest(), len(body)
            size = os.fstat(src.fileno()).st_size
            info.size = size
            h = hashlib.sha256()

            class Reader:
                def read(self, k=-1):
                    block = src.read(k)
                    h.update(block)
                    return block
            t.addfile(info, Reader())
            return h.hexdigest(), size

        add(FIRST, json.dumps({'v': FORMAT, 'made': int(now or time.time()), 'version': _version()}).encode())
        for kind, folder in (('config', config),) + ((() if same else (('state', state),))):
            for parts, p in _walk(folder):
                name = kind + '/' + '/'.join(parts)
                try:
                    with open(p, 'rb') as src:
                        if os.fstat(src.fileno()).st_size <= SMALL:
                            digest, size = add(name, body=src.read())
                        else:
                            digest, size = add(name, src=src)
                except FileNotFoundError:
                    continue  # gone since it was listed (a server that is running replaced it): not part of this copy
                except OSError as e:
                    raise Refused('unreadable', f'{p} could not be read ({e}).')
                files.append([name, size, digest])
                n += 1
        add(LAST, json.dumps({'files': files}).encode())
        t.close()
        raw.flush()
        os.fsync(raw.fileno())
    return {'ok': True, 'sha256': sha256_of(out), 'bytes': out.stat().st_size, 'files': n}


# ---------------------------------------------------------------- stage and swap

def _remove(path, root):
    """Removes one of this file's own scratch folders, and nothing else: `path` must be `.home-staging` or
    `.home-before` directly inside `root`."""
    path, root = Path(path), Path(root)
    if path.name not in SCRATCH or path.parent.resolve() != root.resolve() or path.is_symlink():
        raise Refused('unsafe', f'{path} is not a folder this command may remove.')
    if path.exists():
        shutil.rmtree(path)


def _staging(config, state):
    """(where the settings are staged, where the state is staged, the note about what is staged)."""
    return Path(config) / STAGING / 'c', Path(state) / STAGING / 's', Path(state) / STAGING / 'staged.json'


def _note(config, state):
    try:
        d = json.loads(_staging(config, state)[2].read_text(encoding='utf-8'))
        return d if isinstance(d, dict) and d.get('sha256') else None
    except (OSError, ValueError):
        return None


def discard(config, state):
    """Removes what is staged. Nothing a program reads is touched."""
    for folder in (config, state):
        _remove(Path(folder) / STAGING, folder)
    return {'ok': True}


def status(config, state):
    n = _note(config, state)
    return {'ok': True, 'staged': n['sha256'] if n else None, 'held': held(state),
            'stepped_down': stepped_down(config) is not None}


def _not_held(state):
    if held(state):
        raise Refused('held', 'A Switchboard server is running on this computer. Stop it first: switchboard stop')


def _member_parts(name):
    """('config' or 'state', parts) for a member's name, or Refused: only plain paths inside the two folders, and
    never one of this computer's own files."""
    parts = name.split('/')
    if (name.startswith('/') or '\\' in name or len(parts) < 2 or parts[0] not in ('config', 'state')
            or any(p in ('', '.', '..') or ':' in p for p in parts) or own(tuple(parts[1:]))):
        raise Refused('damaged', f'The file names something that is not home state ({name[:80]}).')
    return parts[0], parts[1:]


def stage(config, state, archive, expect=None):
    """Reads `archive` into the staging folders and checks it. {sha256, bytes, files}. On any doubt the staging
    folders are removed and Refused is raised: nothing else on this computer has changed."""
    config, state, archive = Path(config), Path(state), Path(archive)
    _not_held(state)
    try:
        size = archive.stat().st_size
        digest = sha256_of(archive)
    except OSError as e:
        raise Refused('damaged', f'The file could not be read ({e}).')
    damaged = lambda why: Refused('damaged', f'The file is cut short or was changed ({why}). Nothing was unpacked.')
    if expect is not None and digest != expect:
        raise damaged('it is not the file that was sent')
    if size == 0 or size % tarfile.RECORDSIZE:
        raise damaged('its end is missing')
    same = config.resolve() == state.resolve()
    c_dir, s_dir, note = _staging(config, state)
    discard(config, state)
    seen, got, tops = [], {}, {'c': set(), 's': set()}
    try:
        for d in (c_dir, s_dir):
            d.mkdir(parents=True, exist_ok=True)
            os.chmod(d.parent, 0o700)
        try:
            with tarfile.open(archive, mode='r:') as t:
                listed = None
                for m in t:
                    if listed is not None:
                        raise damaged('there is more after its end')
                    if not seen:
                        if m.name != FIRST or not m.isfile():
                            raise damaged('it does not start as home state does')
                        head = json.loads(t.extractfile(m).read(65536).decode('utf-8'))
                        v = head.get('v') if isinstance(head, dict) else None
                        if not isinstance(v, int) or v < 1:
                            raise damaged('it does not say which format it is')
                        if v > FORMAT:
                            raise Refused('newer', 'This file was made by a newer version of Switchboard. Update '
                                                   'Switchboard on this computer, then try again.')
                        seen.append(m.name)
                        continue
                    if m.name == LAST and m.isfile():
                        listed = json.loads(t.extractfile(m).read(64 << 20).decode('utf-8'))
                        continue
                    kind, parts = _member_parts(m.name)
                    if m.isdir():
                        continue
                    if not m.isreg():
                        raise damaged('it holds something that is not a file')
                    if m.name in got:
                        raise damaged('it names a file twice')
                    key = 'c' if kind == 'config' or same else 's'
                    dest = (c_dir if kind == 'config' else s_dir).joinpath(*parts)
                    dest.parent.mkdir(parents=True, exist_ok=True)
                    h, n, src = hashlib.sha256(), 0, t.extractfile(m)
                    fd = os.open(dest, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
                    with os.fdopen(fd, 'wb') as f:
                        for block in iter(lambda: src.read(1 << 20), b''):
                            h.update(block)
                            n += len(block)
                            f.write(block)
                        f.flush()
                        os.fsync(f.fileno())
                    got[m.name] = [n, h.hexdigest()]
                    tops['c' if kind == 'config' else 's'].add(parts[0])
                    del key
        except Refused:
            raise
        except (tarfile.TarError, EOFError, OSError, ValueError, UnicodeError, AttributeError) as e:
            raise damaged(type(e).__name__)
        if not seen or listed is None:
            raise damaged('its end is missing')
        try:
            want = {f[0]: [f[1], f[2]] for f in listed['files']}
        except (TypeError, KeyError, IndexError):
            raise damaged('its list of files cannot be read')
        if want != got:
            raise damaged('a file in it is not what its list says')
        _addresses(c_dir / 'server.json', config / 'server.json')
        done = {'sha256': digest, 'bytes': size, 'files': len(got), 'at': int(time.time()),
                'tops': {k: sorted(v) for k, v in tops.items()}}
        note.write_text(json.dumps(done), encoding='utf-8')
    except BaseException:
        try:
            discard(config, state)
        except Exception:
            pass
        raise
    return {'ok': True, 'sha256': digest, 'bytes': size, 'files': len(got)}


def _addresses(staged, here):
    """The staged server.json keeps the settings that are the home's and takes the addresses of this computer:
    the ones its own server.json names, or none (the server works them out at its next start)."""
    try:
        d = json.loads(staged.read_text(encoding='utf-8'))
    except (OSError, ValueError):
        return
    if not isinstance(d, dict):
        return
    for k in ADDRESSES:
        d.pop(k, None)
    try:
        mine = json.loads(here.read_text(encoding='utf-8'))
    except (OSError, ValueError):
        mine = None
    if isinstance(mine, dict):
        d.update({k: mine[k] for k in ADDRESSES if k in mine})
    staged.write_text(json.dumps(d, indent=1), encoding='utf-8')


def _aside(p, before):
    """Moves `p` into the folder `before`, under a name nothing there has."""
    before.mkdir(parents=True, exist_ok=True)
    to, n = before / p.name, 1
    while to.exists() or to.is_symlink():
        n += 1
        to = before / f'{p.name}.{n}'
    os.replace(p, to)


def swap(config, state):
    """Moves what `stage` checked into place. What was home state here before goes to `.home-before` in the same
    folder, whole, so the two are never mixed. Run again after a stop half way, it finishes."""
    config, state = Path(config), Path(state)
    n = _note(config, state)
    if not n:
        raise Refused('nothing', 'Nothing is staged on this computer.')
    _not_held(state)
    c_dir, s_dir, note = _staging(config, state)
    if not n.get('swapping'):
        for folder in (config, state):
            _remove(folder / BEFORE, folder)
        n['swapping'] = True
        note.write_text(json.dumps(n), encoding='utf-8')
    same = config.resolve() == state.resolve()
    tops = n.get('tops') or {}
    arriving = set(tops.get('c') or []) | set(tops.get('s') or []) if same else None
    moved = 0
    for folder, staged, key in ((config, c_dir, 'c'), (state, s_dir, 's')):
        mine = arriving if same else set(tops.get(key) or [])
        # home state that was here and is not in what arrives: put aside, so no old list is read as the new home's
        for name in sorted(os.listdir(folder)):
            if name not in mine and not own((name,)):
                _aside(folder / name, folder / BEFORE)
        for name in sorted(os.listdir(staged)) if staged.is_dir() else []:
            if own((name,)):
                continue  # (stage refuses these; nothing of this computer's own is ever replaced)
            dest = folder / name
            if dest.exists() or dest.is_symlink():
                _aside(dest, folder / BEFORE)
            os.replace(staged / name, dest)
            moved += 1
    discard(config, state)
    return {'ok': True, 'moved': moved, 'sha256': n['sha256']}


# ---------------------------------------------------------------- command line

def main(argv=None):
    ap = argparse.ArgumentParser(prog='homestate.py')
    ap.add_argument('verb', choices=('pack', 'stage', 'swap', 'discard', 'status'))
    ap.add_argument('--config', required=True)
    ap.add_argument('--state', required=True)
    ap.add_argument('--out')
    ap.add_argument('--in', dest='src')
    ap.add_argument('--sha256')
    a = ap.parse_args(argv)
    try:
        if a.verb == 'pack':
            if not a.out:
                ap.error('pack needs --out FILE')
            r = pack(a.config, a.state, a.out)
        elif a.verb == 'stage':
            if not a.src:
                ap.error('stage needs --in FILE')
            r = stage(a.config, a.state, a.src, expect=a.sha256)
        elif a.verb == 'swap':
            r = swap(a.config, a.state)
        elif a.verb == 'discard':
            r = discard(a.config, a.state)
        else:
            r = status(a.config, a.state)
    except Refused as e:
        print(json.dumps({'ok': False, 'error': str(e), 'kind': e.kind}))
        return 1
    except OSError as e:
        print(json.dumps({'ok': False, 'error': f'It could not be done ({e}).', 'kind': 'failed'}))
        return 1
    print(json.dumps(r))
    return 0


if __name__ == '__main__':
    sys.exit(main())
