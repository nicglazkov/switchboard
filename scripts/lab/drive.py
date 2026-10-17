#!/usr/bin/env python3
"""Headless Chrome driver for the iPhone review (based on /tmp/sblab/cdp.py, no deps).

usage: drive.py <plan.json> <outdir>
A plan is a list of steps, run in order:
  {"size": [w, h], "mobile": true, "scale": 2}     device metrics (mobile defaults to w < 600)
  {"scheme": "dark" | "light"}                       prefers-color-scheme
  {"motion": "reduce"}                               prefers-reduced-motion
  {"ua": "..."}                                      user agent (before nav)
  {"safe": [top, right, bottom, left]}               safe-area insets (iPhone 17 Pro: 62, 0, 34, 0)
  {"zoom": 1.2}                                      CSS zoom on <html> before scripts (like WKWebView pageZoom)
  {"nav": "http://..."}                              navigate and wait for load
  {"wait": 1.5}
  {"js": "expr"}                                     evaluate (awaits promises), prints the value
  {"shot": "name.png"}                               screenshot of the viewport
  {"tap": "css selector" | [x, y]}                   touch tap at the element's center
  {"swipe": [x0, y0, x1, y1, ms]} or {"swipe": "sel", "dx": -200, "dy": 0, "ms": 250}
  {"type": "text"}                                   insert text into the focused field
  {"log": true}                                      print console messages and errors from here on
"""
import base64, json, os, signal, socket, struct, subprocess, sys, time, urllib.request

plan, outdir = json.load(open(sys.argv[1])), sys.argv[2]
os.makedirs(outdir, exist_ok=True)
signal.signal(signal.SIGALRM, lambda *a: sys.exit('drive.py: timed out'))
signal.alarm(int(os.environ.get('DRIVE_TIMEOUT', '240')))
with socket.socket() as _s:
    _s.bind(('127.0.0.1', 0))
    port = _s.getsockname()[1]
prof = f'/tmp/sblab/prof-ios-{os.getpid()}'
chrome = subprocess.Popen(['google-chrome', '--headless=new', '--no-sandbox', '--disable-gpu', '--hide-scrollbars',
                           '--force-device-scale-factor=2', f'--remote-debugging-port={port}', f'--user-data-dir={prof}', 'about:blank'],
                          stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
events, logs = [], [False]
try:
    for _ in range(60):
        try:
            tabs = json.loads(urllib.request.urlopen(f'http://127.0.0.1:{port}/json').read())
            ws = next(t['webSocketDebuggerUrl'] for t in tabs if t['type'] == 'page')
            break
        except Exception:
            time.sleep(0.2)
    path = ws.split(str(port), 1)[1]
    s = socket.create_connection(('127.0.0.1', port), timeout=60)
    key = base64.b64encode(os.urandom(16)).decode()
    s.sendall(f'GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n'
              f'Sec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n\r\n'.encode())
    buf = b''
    while b'\r\n\r\n' not in buf:
        buf += s.recv(4096)
    buf = buf.split(b'\r\n\r\n', 1)[1]

    def recv_exact(n):
        global buf
        while len(buf) < n:
            chunk = s.recv(1 << 20)
            if not chunk:
                sys.exit('drive.py: DevTools closed')
            buf += chunk
        d, buf = buf[:n], buf[n:]
        return d

    def recv():
        b1, b2 = recv_exact(2)
        n = b2 & 0x7f
        if n == 126:
            n = struct.unpack('>H', recv_exact(2))[0]
        elif n == 127:
            n = struct.unpack('>Q', recv_exact(8))[0]
        return recv_exact(n)

    mid = [0]

    def handle_event(m):
        if not logs[0]:
            return
        if m.get('method') == 'Runtime.consoleAPICalled':
            print('console:', ' '.join(str(a.get('value', a.get('description', ''))) for a in m['params']['args'])[:400])
        elif m.get('method') == 'Runtime.exceptionThrown':
            print('EXCEPTION:', m['params']['exceptionDetails'].get('exception', {}).get('description', '')[:600])

    def call(method, **params):
        mid[0] += 1
        data = json.dumps({'id': mid[0], 'method': method, 'params': params}).encode()
        mask = os.urandom(4)
        n = len(data)
        hdr = bytes([0x81]) + (bytes([0x80 | n]) if n < 126 else bytes([0x80 | 126]) + struct.pack('>H', n)
                               if n < 65536 else bytes([0x80 | 127]) + struct.pack('>Q', n))
        s.sendall(hdr + mask + bytes(c ^ mask[i % 4] for i, c in enumerate(data)))
        while True:
            m = json.loads(recv())
            if m.get('id') == mid[0]:
                if 'error' in m:
                    raise RuntimeError(f"{method}: {m['error']}")
                return m['result']
            handle_event(m)

    def drain(sec):
        end = time.time() + sec
        s.settimeout(0.05)
        try:
            while time.time() < end:
                try:
                    handle_event(json.loads(recv()))
                except (socket.timeout, TimeoutError):
                    pass
        finally:
            s.settimeout(60)

    def ev(expr):
        r = call('Runtime.evaluate', expression=expr, awaitPromise=True, returnByValue=True)
        if r.get('exceptionDetails'):
            raise RuntimeError(r['exceptionDetails'].get('exception', {}).get('description', '') or str(r['exceptionDetails']))
        return r.get('result', {}).get('value')

    def center(sel):
        if sel.startswith('text:'):  # the last (innermost) button-like element whose text has the words
            words = sel[5:]
            find = (f"[...document.querySelectorAll('button, a, [role=button], .row, .rp, .act, .skey, .f-row .fold-b')]"
                    f".filter(e => e.offsetParent !== null && e.textContent.includes({json.dumps(words)})).pop()")
        else:
            find = f"document.querySelector({json.dumps(sel)})"
        r = ev(f"(() => {{ const e = {find}; if (!e) return null; e.scrollIntoView({{block: 'nearest'}}); "
               f"const b = e.getBoundingClientRect(); return [b.left + b.width / 2, b.top + b.height / 2]; }})()")
        if r is None:
            raise RuntimeError(f'no element {sel}')
        return r

    def touch(kind, x, y):
        pts = [] if kind == 'touchEnd' else [{'x': x, 'y': y, 'id': 1, 'radiusX': 6, 'radiusY': 6, 'force': 1}]
        call('Input.dispatchTouchEvent', type=kind, touchPoints=pts)

    call('Page.enable')
    call('Runtime.enable')
    w, h, mobile = 390, 844, True
    for st in plan:
        try:
            if 'size' in st:
                w, h = st['size']
                mobile = st.get('mobile', w < 600)
                call('Emulation.setDeviceMetricsOverride', width=w, height=h, deviceScaleFactor=st.get('scale', 2), mobile=mobile)
                call('Emulation.setTouchEmulationEnabled', enabled=mobile, maxTouchPoints=5 if mobile else 1)
            elif 'scheme' in st or 'motion' in st:
                feats = []
                if 'scheme' in st:
                    feats.append({'name': 'prefers-color-scheme', 'value': st['scheme']})
                if 'motion' in st:
                    feats.append({'name': 'prefers-reduced-motion', 'value': st['motion']})
                call('Emulation.setEmulatedMedia', features=feats)
            elif 'ua' in st:
                call('Emulation.setUserAgentOverride', userAgent=st['ua'])
            elif 'safe' in st:
                t, r, b, l = st['safe']
                call('Emulation.setSafeAreaInsetsOverride', insets={'top': t, 'right': r, 'bottom': b, 'left': l})
            elif 'zoom' in st:
                call('Page.addScriptToEvaluateOnNewDocument',
                     source=f"document.documentElement && (document.documentElement.style.zoom = '{st['zoom']}');")
            elif 'nav' in st:
                call('Page.navigate', url=st['nav'])
                drain(st.get('settle', 4))
            elif 'wait' in st:
                drain(st['wait'])
            elif 'js' in st:
                v = ev(st['js'])
                if v is not None:
                    print('js:', json.dumps(v)[:2000] if not isinstance(v, str) else v[:4000])
                drain(st.get('after', 0.3))
            elif 'shot' in st:
                drain(st.get('before', 0.2))
                shot = call('Page.captureScreenshot', format='png')
                open(os.path.join(outdir, st['shot']), 'wb').write(base64.b64decode(shot['data']))
                print('shot:', st['shot'])
            elif 'tap' in st:
                x, y = st['tap'] if isinstance(st['tap'], list) else center(st['tap'])
                if mobile:
                    touch('touchStart', x, y)
                    drain(0.06)
                    touch('touchEnd', x, y)
                else:
                    for kind in ('mousePressed', 'mouseReleased'):
                        call('Input.dispatchMouseEvent', type=kind, x=x, y=y, button='left', clickCount=1)
                drain(st.get('after', 0.6))
            elif 'swipe' in st:
                sw = st['swipe']
                if isinstance(sw, list):
                    x0, y0, x1, y1, ms = sw
                else:
                    x0, y0 = center(sw)
                    x1, y1, ms = x0 + st.get('dx', 0), y0 + st.get('dy', 0), st.get('ms', 250)
                n = max(6, int(ms / 16))
                touch('touchStart', x0, y0)
                for i in range(1, n + 1):
                    f = i / n
                    touch('touchMove', x0 + (x1 - x0) * f, y0 + (y1 - y0) * f)
                    time.sleep(ms / 1000 / n)
                if st.get('hold'):
                    drain(st['hold'])
                touch('touchEnd', x1, y1)
                drain(st.get('after', 0.8))
            elif 'type' in st:
                call('Input.insertText', text=st['type'])
                drain(0.2)
            elif 'ctrl' in st:   # turn a fake-fleet control file on (or off with "off": true)
                name = os.path.basename(st['ctrl'])
                fp = os.path.join(os.path.expanduser('~/sb-ios-lab/ctrl'), name)
                if st.get('off'):
                    if os.path.exists(fp):
                        os.remove(fp)
                else:
                    os.makedirs(os.path.dirname(fp), exist_ok=True)
                    open(fp, 'w').close()
                print('ctrl:', name, 'off' if st.get('off') else 'on')
            elif 'log' in st:
                logs[0] = bool(st['log'])
        except RuntimeError as e:
            print('STEP FAILED', json.dumps(st)[:120], '->', str(e)[:400])
finally:
    chrome.kill()
    subprocess.run(['rm', '-rf', prof])
