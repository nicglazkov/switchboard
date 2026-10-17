#!/usr/bin/env python3
"""A stand-in for `ssh <machine> term <bg>` that draws like Claude Code, for testing the terminal pipeline end to end
(spec 2026-10-14). Point a test hub at it: SWITCHBOARD_HUB_SSH=native/scripts/term-fixture.py.

It floods colored lines with wide characters, redraws a framed status box at the bottom inside synchronized updates
(as Claude Code does), and redraws the box at the new size on SIGWINCH. Input: "q" ends it (exit 0), "f" floods again.
Settings come from the environment: TERM_FIXTURE_SECONDS (flood length, default 6), TERM_FIXTURE_RATE (lines per
second, default 4000).
"""
import fcntl
import os
import select
import signal
import struct
import sys
import termios
import time

out = sys.stdout.buffer
SYNC_ON, SYNC_OFF = b"\x1b[?2026h", b"\x1b[?2026l"
WIDE = ["你好", "漢字", "\U0001f600", "안녕"]
resized = False


def size():
    try:
        rows, cols, _, _ = struct.unpack("HHHH", fcntl.ioctl(1, termios.TIOCGWINSZ, b"\0" * 8))
        return max(cols, 20), max(rows, 6)
    except OSError:
        return 80, 24


def on_winch(*_):
    global resized
    resized = True


def box(n, note=""):
    """The status box on the last four rows, drawn whole in one synchronized update."""
    cols, rows = size()
    w = cols - 2
    top = rows - 3
    text = f" line {n}  {cols}x{rows}  {note}"[:w]
    b = [SYNC_ON, b"\x1b7"]
    b.append(f"\x1b[{top};1H\x1b[0;38;5;245m╭{'─' * w}╮".encode())
    b.append(f"\x1b[{top + 1};1H│\x1b[1;38;2;215;119;87m{text.ljust(w)}\x1b[0;38;5;245m│".encode())
    b.append(f"\x1b[{top + 2};1H╰{'─' * w}╯\x1b[0m".encode())
    b.append(f"\x1b[{rows};1H\x1b[2K\x1b[3m? for shortcuts\x1b[0m".encode())
    b += [b"\x1b8", SYNC_OFF]
    return b"".join(b)


def line(n):
    cols, _ = size()
    color = 31 + n % 7
    wide = WIDE[n % len(WIDE)]
    body = f"\x1b[{color}m{n:07d}\x1b[0m \x1b[1m{wide}\x1b[0m " + ("\x1b[4mlink\x1b[24m " if n % 11 == 0 else "")
    fill = "abcdefghij" * ((cols // 10) + 2)
    # every 13th line is longer than the screen: a soft wrap
    return (body + fill[: cols - 14 + (cols // 2 if n % 13 == 0 else 0)]).encode()


def main():
    signal.signal(signal.SIGWINCH, on_winch)
    global resized
    seconds = float(os.environ.get("TERM_FIXTURE_SECONDS", "6"))
    rate = float(os.environ.get("TERM_FIXTURE_RATE", "4000"))
    cols, rows = size()
    out.write(f"term fixture: {' '.join(sys.argv[1:])}\r\n".encode())
    # scroll region above the box, as a full-screen program keeps its footer
    out.write(f"\x1b[1;{rows - 4}r\x1b[{rows - 4};1H".encode())
    out.flush()
    n = 0
    flood_until = time.time() + seconds
    while True:
        now = time.time()
        if resized:
            resized = False
            cols, rows = size()
            out.write(SYNC_ON + f"\x1b[r\x1b[2J\x1b[1;{rows - 4}r\x1b[{rows - 4};1H".encode() + SYNC_OFF)
            out.write(box(n, "resized"))
            out.flush()
        if now < flood_until:
            batch = max(1, int(rate / 100))
            chunk = b"".join(line(n + i) + b"\r\n" for i in range(batch))
            n += batch
            out.write(chunk + box(n, "flooding"))
            out.flush()
            wait = 0.01
        else:
            out.write(box(n, "idle"))
            out.flush()
            wait = 0.5
        try:
            r, _, _ = select.select([0], [], [], wait)
        except InterruptedError:
            continue
        if r:
            data = os.read(0, 4096)
            if not data or b"q" in data:
                out.write(b"\x1b[r\r\nbye\r\n")
                out.flush()
                return 0
            if b"f" in data:
                flood_until = time.time() + seconds


if __name__ == "__main__":
    tty = termios.tcgetattr(0) if os.isatty(0) else None
    if tty:
        raw = termios.tcgetattr(0)
        raw[3] &= ~(termios.ICANON | termios.ECHO)
        termios.tcsetattr(0, termios.TCSANOW, raw)
    try:
        sys.exit(main())
    finally:
        if tty:
            termios.tcsetattr(0, termios.TCSANOW, tty)
