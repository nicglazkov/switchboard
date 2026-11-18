"""State that survives a power cut.

Every state file is written whole to a temp file beside it and then renamed over the old one, so a reader never
sees half a file. That alone is not enough when the power goes: the rename can reach the disk before the new
file's contents do, and the file comes back as zeros (seen on a Windows machine, November 2026). So the contents
are forced to disk first. Then the old file is whole until the new one is.

Temp files carry the writer's pid, so one left by a process that died is never reused: sweep() removes those.
"""
import os
import time
from pathlib import Path

ORPHAN_AGE = 3600  # a write takes milliseconds: an hour-old temp file has no writer


def put(path, data, tmp=None, mode=None):
    """Replace `path` with `data` (text or bytes), durably. tmp: the temp file to use (beside path, unique to the
    writer). mode: the new file's permissions, set before anything is written into it. The temp file is removed
    when the write fails, and the error goes to the caller."""
    path = Path(path)
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
        os.replace(tmp, path)
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
