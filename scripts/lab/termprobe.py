#!/usr/bin/env python3
"""Open a terminal through the server and hub without a browser (stdlib only), for end to end checks.

usage: termprobe.py <server> <machine> (--sid SID | --bg BG) [--seconds 8] [--cols 120 --rows 36]
                    [--send TEXT]... [--resize COLSxROWS] [--origin URL] [--out FILE]

Gets a ticket from POST <server>/api/term, connects to the hub, prints every control frame, sends each --send
(after a pause; \\r, \\x1b and \\x03 escapes work, "@paste:TEXT" sends it as a bracketed paste), and writes the raw
output to --out. Prints the output with the ANSI codes taken out, last lines only. Never types unless told to.
"""
import argparse, base64, json, os, re, socket, struct, sys, time, urllib.request
from urllib.parse import urlsplit


def ticket(server, machine, sid=None, bg=None):
    body = {'machine': machine, **({'bg': bg} if bg else {'sid': sid})}
    r = urllib.request.Request(server.rstrip('/') + '/api/term', data=json.dumps(body).encode(), method='POST',
                               headers={'Content-Type': 'application/json', 'X-Switchboard': '1'})
    with urllib.request.urlopen(r, timeout=10) as resp:
        return json.loads(resp.read())


class Ws:
    def __init__(self, url, origin=None):
        u = urlsplit(url)
        self.s = socket.create_connection((u.hostname, u.port or 80), timeout=15)
        key = base64.b64encode(os.urandom(16)).decode()
        hdr = [f'GET {u.path}?{u.query} HTTP/1.1', f'Host: {u.netloc}', 'Upgrade: websocket', 'Connection: Upgrade',
               f'Sec-WebSocket-Key: {key}', 'Sec-WebSocket-Version: 13']
        if origin:
            hdr.append(f'Origin: {origin}')
        self.s.sendall(('\r\n'.join(hdr) + '\r\n\r\n').encode())
        buf = b''
        while b'\r\n\r\n' not in buf:
            buf += self.s.recv(4096)
        head, self.buf = buf.split(b'\r\n\r\n', 1)
        if b' 101 ' not in head.split(b'\r\n')[0]:
            raise OSError(head.decode(errors='replace'))

    def send(self, data, text=False):
        op = 1 if text else 2
        data = data.encode() if isinstance(data, str) else data
        n = len(data)
        hdr = bytes([0x80 | op]) + (bytes([0x80 | n]) if n < 126 else bytes([0x80 | 126]) + struct.pack('>H', n) if n < 65536
                                     else bytes([0x80 | 127]) + struct.pack('>Q', n))
        mask = os.urandom(4)
        self.s.sendall(hdr + mask + bytes(b ^ mask[i % 4] for i, b in enumerate(data)))

    def _need(self, n):
        while len(self.buf) < n:
            chunk = self.s.recv(65536)
            if not chunk:
                raise EOFError
            self.buf += chunk
        out, self.buf = self.buf[:n], self.buf[n:]
        return out

    def recv(self):
        b0, b1 = self._need(2)
        n = b1 & 0x7f
        if n == 126:
            n = struct.unpack('>H', self._need(2))[0]
        elif n == 127:
            n = struct.unpack('>Q', self._need(8))[0]
        return b0 & 0x0f, self._need(n)


ANSI = re.compile(rb'\x1b\[[0-9;?<>=]*[ -/]*[@-~]|\x1b\][^\x07\x1b]*(\x07|\x1b\\)|\x1b[()][0-9A-Za-z]|\x1b[=>78DEHMNOZc]')


def unescape(s):
    return s.encode().decode('unicode_escape').encode('latin-1').decode('utf-8', 'replace')


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('server'), ap.add_argument('machine')
    ap.add_argument('--sid'), ap.add_argument('--bg')
    ap.add_argument('--seconds', type=float, default=8)
    ap.add_argument('--cols', type=int, default=120), ap.add_argument('--rows', type=int, default=36)
    ap.add_argument('--send', action='append', default=[])
    ap.add_argument('--resize')
    ap.add_argument('--origin')
    ap.add_argument('--out')
    a = ap.parse_args()
    t = ticket(a.server, a.machine, a.sid, a.bg)
    print('ticket:', json.dumps({k: v for k, v in t.items() if k != 'ws'}))
    if not t.get('ok'):
        return 1
    ws = Ws(f'{t["ws"]}&cols={a.cols}&rows={a.rows}', a.origin)
    ws.s.settimeout(0.25)
    out, start, sends = bytearray(), time.time(), list(a.send)
    nxt = start + 4
    while time.time() - start < a.seconds:
        if a.resize and time.time() - start > 2.5:
            c, r = a.resize.split('x')
            ws.send(json.dumps({'type': 'resize', 'cols': int(c), 'rows': int(r)}), text=True)
            print('sent resize', a.resize)
            a.resize = None
        if sends and time.time() > nxt:
            s = sends.pop(0)
            data = ('\x1b[200~' + unescape(s[7:]) + '\x1b[201~') if s.startswith('@paste:') else unescape(s)
            ws.send(data.encode())
            print('sent', repr(data))
            nxt = time.time() + 3
        try:
            op, data = ws.recv()
        except socket.timeout:
            continue
        except EOFError:
            print('socket closed')
            break
        if op == 2:
            out += data
        elif op == 1:
            print('text:', data.decode())
        elif op == 8:
            code = struct.unpack('>H', data[:2])[0] if len(data) >= 2 else None
            print('close:', code, data[2:].decode(errors='replace'))
            break
    print(f'{len(out)} bytes of output')
    if a.out:
        open(a.out, 'wb').write(out)
    plain = ANSI.sub(b' ', bytes(out)).decode('utf-8', 'replace')
    lines = [re.sub(r'\s+', ' ', ln).strip() for ln in re.split(r'[\r\n]+', plain)]
    print('\n'.join([ln for ln in lines if ln][-25:]))
    return 0


if __name__ == '__main__':
    sys.exit(main())
