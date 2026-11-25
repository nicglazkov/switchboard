"""State that survives a power cut.

Every state file is written whole to a temp file beside it and then renamed over the old one, so a reader never
sees half a file. That alone is not enough when the power goes: the rename can reach the disk before the new
file's contents do, and the file comes back as zeros (seen on a Windows machine, November 2026). So the contents
are forced to disk first. Then the old file is whole until the new one is.

Temp files carry the writer's pid, so one left by a process that died is never reused: sweep() removes those.

A file that cannot be read is not an empty one. load() tells the two apart. A file that is not there is `absent`,
and the caller's default is right. One that is empty, cut short, not JSON or of the wrong shape is put aside as
<name>.bad-<time> (the newest three are kept), and the copy put() left beside it (<name>.bak, the file as it was
before its last write) is used instead: `recovered`. With no good copy the store is `broken`: the caller gets its
default, and put() refuses to write over the store until someone who means it saves it again (force=True), so
defaults are never written down as if the user had chosen them.

A file that is there and will not open (no permission, too many open files, another program holding it) is not
damaged. It is read again after a moment, and then the copy is used with the file left exactly as it is.
"""
import json
import os
import threading
import time
from pathlib import Path

ORPHAN_AGE = 3600  # a write takes milliseconds: an hour-old temp file has no writer
BAD_KEEP = 3       # copies of a file that could not be read: the newest are kept
READ_RETRY = (0.05, 0.2)  # waits before reading again a file that is there and would not open
ABSENT, OK, RECOVERED, BROKEN = 'absent', 'ok', 'recovered', 'broken'

_lock = threading.RLock()
_marks = {}      # path -> {'status', 'at', 'kept', 'replaced'}: stores found damaged since this start
_looked = set()  # paths whose folder was searched once for a copy an earlier run put aside


class Broken(OSError):
    """A write to a store that could not be read and has no good copy. An OSError, so a writer that already says
    "could not save" says it for this too."""


def _parse(raw, shape, check):
    """The JSON in `raw`. ValueError when there is none, or it is not what the store holds."""
    if not raw.strip(b'\x00 \t\r\n'):
        raise ValueError('empty')
    d = json.loads(raw.decode('utf-8'))
    if shape is not None and not isinstance(d, shape):
        raise ValueError('wrong type')
    if check is not None and not check(d):
        raise ValueError('not what this file holds')
    return d


def _bak(path):
    return path.with_name(path.name + '.bak')


def _bads(path):
    try:
        return sorted(n for n in os.listdir(path.parent) if n.startswith(path.name + '.bad-'))
    except OSError:
        return []


def aside(path):
    """Move a file that cannot be used to <name>.bad-<time>, keeping the newest BAD_KEEP. Its new name, or None
    when there was nothing to move."""
    path = Path(path)
    bad = path.with_name(f'{path.name}.bad-{time.strftime("%Y%m%d-%H%M%S")}')
    try:
        os.replace(path, bad)
    except OSError:
        return None
    for old in _bads(path)[:-BAD_KEEP]:
        try:
            os.unlink(path.with_name(old))
        except OSError:
            pass
    return bad.name


def _mark(path, status, kept):
    _marks[str(path)] = {'status': status, 'at': time.time(), 'kept': kept, 'replaced': False}


def _read(path, waits):
    """The file's bytes. None: it is not there. False: it is there and could not be opened, also after `waits`."""
    for wait in tuple(waits) + (None,):
        try:
            return path.read_bytes()
        except FileNotFoundError:
            return None
        except OSError:
            if wait is None:
                return False
            time.sleep(wait)


def unopened(path):
    """Whether the store is answered from its copy because the file itself would not open."""
    m = _marks.get(str(path))
    return bool(m and m['status'] == RECOVERED and not m['kept'])


def load(path, default, shape=dict, check=None, keep=False):
    """(data, status) for the JSON store at `path`: see the top of this file. shape: the type (or types) its JSON
    is. check(d): whether what was parsed is what the store holds. keep: the .bak is made to match a good file
    on every load, for stores people edit by hand and the server seldom writes."""
    path = Path(path)
    key = str(path)
    raw = _read(path, () if key in _marks else READ_RETRY)
    if raw is False:
        # There, and it will not open. That says nothing about what is in it, so it is not put aside and the copy
        # is not written over it: the copy is only read.
        with _lock:
            try:
                d = _parse(_bak(path).read_bytes(), shape, check)
            except (OSError, ValueError):
                _mark(path, BROKEN, None)
                return default, BROKEN
            _mark(path, RECOVERED, None)
            return d, RECOVERED
    if raw is not None:
        try:
            d = _parse(raw, shape, check)
        except ValueError:
            pass
        else:
            if key in _marks:
                with _lock:
                    m = _marks.get(key)
                    if m and (m['status'] == BROKEN and not m['replaced'] or m['status'] == RECOVERED and not m['kept']):
                        del _marks[key]  # someone put a readable file back, or it opens again and was never damaged
            if keep:
                # After every good load, not only the first: someone who tightened the file by hand and later
                # made a typo gets the tightened file back, not the one from before.
                try:
                    same = _bak(path).read_bytes() == raw
                except OSError:
                    same = False
                if not same:
                    backup(path)
            return d, OK
    with _lock:
        if raw is None:
            if broken(path):
                return default, BROKEN
            if key in _looked:
                return default, ABSENT
            _looked.add(key)
            bads = _bads(path)
            if not bads:
                return default, ABSENT
            kept = bads[-1]  # put aside by an earlier run, and nobody has dealt with it yet
        else:
            _looked.add(key)
            kept = aside(path)
        try:
            raw = _bak(path).read_bytes()
            d = _parse(raw, shape, check)
        except (OSError, ValueError):
            _mark(path, BROKEN, kept)
            return default, BROKEN
        _mark(path, RECOVERED, kept)
        try:
            put(path, raw)
        except OSError:
            pass  # still answered from the copy, and the next load tries again
        return d, RECOVERED


def broken(path):
    """Whether the store was found damaged with no good copy, and nobody has saved it again since."""
    m = _marks.get(str(path))
    return bool(m and m['status'] == BROKEN and not m['replaced'])


def status():
    """The stores found damaged since this start, for the apps: [{file, status, at, kept, replaced}]. status is
    `recovered` (read from its last good copy) or `broken` (no good copy: defaults are in use). kept: the name the
    damaged file has now. replaced: a broken store that has been saved again since."""
    with _lock:
        return [dict(m, file=Path(k).name) for k, m in sorted(_marks.items())]


def backup(path, link=False):
    """Keep the file as it is now as <name>.bak, when it holds JSON. False: nothing kept. link: a hard link where
    the disk has them, for put(), which replaces the file right after, so the link keeps the old contents for
    free. Otherwise a copy of its own, which damage to the file itself cannot reach."""
    path = Path(path)
    bak = _bak(path)
    try:
        raw = path.read_bytes()
        _parse(raw, None, None)
    except (OSError, ValueError):
        return False  # nothing good to keep: the copy from before stays
    try:
        try:
            os.unlink(bak)
        except FileNotFoundError:
            pass
        try:
            if not link:
                raise OSError('a copy was asked for')
            os.link(path, bak)
        except OSError:
            fd = os.open(bak, os.O_WRONLY | os.O_CREAT | os.O_TRUNC | getattr(os, 'O_BINARY', 0), os.stat(path).st_mode & 0o777)
            with os.fdopen(fd, 'wb') as f:
                f.write(raw)
                f.flush()
                os.fsync(f.fileno())
    except OSError:
        return False
    return True


def put(path, data, tmp=None, mode=None, keep_bak=False, force=False, fresh=False):
    """Replace `path` with `data` (text or bytes), durably. tmp: the temp file to use (beside path, unique to the
    writer). mode: the new file's permissions, set before anything is written into it. The temp file is removed
    when the write fails, and the error goes to the caller.

    keep_bak: the file as it is now stays beside it as <name>.bak (when it is good), for load() to go back to.
    fresh: the .bak is a copy of the NEW file instead, for a store whose old contents may hold what was just taken
    out of it (a removed machine, a phone's push token). load() must never bring that back.
    force: write even over a store load() found broken. For a save the user asked for, or a store the server
    gathers itself and can gather again. Any other write to a broken store raises Broken."""
    path = Path(path)
    if str(path) in _marks:
        with _lock:
            m = _marks.get(str(path))
            if m and m['status'] == BROKEN and not m['replaced']:
                if not force:
                    raise Broken(f'{path.name} could not be read and has no good copy, so it is not written over')
                m['replaced'] = True
    tmp = Path(tmp) if tmp is not None else path.with_name(f'{path.name}.{os.getpid()}.tmp')
    raw = data.encode('utf-8') if isinstance(data, str) else data
    try:
        fd = os.open(tmp, os.O_WRONLY | os.O_CREAT | os.O_TRUNC | getattr(os, 'O_BINARY', 0), 0o666 if mode is None else mode)
        with os.fdopen(fd, 'wb') as f:
            if mode is not None and os.name != 'nt':
                os.fchmod(f.fileno(), mode)  # also when an older run left the temp file behind
            f.write(raw)
            f.flush()
            os.fsync(f.fileno())
        if keep_bak and not fresh:
            backup(path, link=True)
        os.replace(tmp, path)
        if fresh and not backup(path):
            try:
                os.unlink(_bak(path))  # no copy at all, rather than one from before the change
            except OSError:
                pass
    except BaseException:
        try:
            os.unlink(tmp)
        except OSError:
            pass
        raise


def sweep(folder, now=None, age=ORPHAN_AGE):
    """Remove temp files no writer is coming back for. Returns how many went."""
    now, gone = time.time() if now is None else now, 0
    try:
        names = os.listdir(folder)
    except OSError:
        return 0
    for n in names:
        if not (n.endswith('.tmp') or '.tmp-' in n):
            continue
        p = os.path.join(folder, n)
        try:
            if os.path.isfile(p) and now - os.path.getmtime(p) > age:
                os.unlink(p)
                gone += 1
        except OSError:
            pass
    return gone
