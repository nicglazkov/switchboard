"""Apple push over HTTP/2 via curl, with an ES256 provider token."""
import base64, json, re, subprocess, threading, time

HOSTS = {True: 'https://api.sandbox.push.apple.com', False: 'https://api.push.apple.com'}
TOKEN_RE = re.compile(r'[0-9a-f]{64,200}')


def _b64(b):
    return base64.urlsafe_b64encode(b).rstrip(b'=').decode()


class Apns:
    def __init__(self, key_path, key_id, team, topic, sandbox=True, runner=subprocess.run, clock=time.time):
        from cryptography.hazmat.primitives import serialization
        self.key = serialization.load_pem_private_key(open(key_path, 'rb').read(), None)
        self.key_id, self.team, self.topic, self.sandbox = key_id, team, topic, sandbox
        self.runner, self.clock = runner, clock
        self._tok, self._iat = None, 0
        self.lock = threading.Lock()

    def jwt(self):
        from cryptography.hazmat.primitives import hashes
        from cryptography.hazmat.primitives.asymmetric import ec
        from cryptography.hazmat.primitives.asymmetric.utils import decode_dss_signature
        with self.lock:
            now = int(self.clock())
            if self._tok and now - self._iat < 3000:  # Apple wants a refresh between 20 and 60 min
                return self._tok
            h = _b64(json.dumps({'alg': 'ES256', 'kid': self.key_id}, separators=(',', ':')).encode())
            c = _b64(json.dumps({'iss': self.team, 'iat': now}, separators=(',', ':')).encode())
            r, s = decode_dss_signature(self.key.sign(f'{h}.{c}'.encode(), ec.ECDSA(hashes.SHA256())))
            self._tok, self._iat = f'{h}.{c}.{_b64(r.to_bytes(32, "big") + s.to_bytes(32, "big"))}', now
            return self._tok

    def _auth(self):
        """The authorization header for curl's `-H @-`: on stdin, never in argv, which `ps` shows every local user."""
        return f'authorization: bearer {self.jwt()}\n'

    def _host(self, sandbox=None):
        """Apple's service for development builds or the one for released builds: the one in apns.json, unless
        the caller names the other (Router)."""
        return HOSTS[bool(self.sandbox if sandbox is None else sandbox)]

    def _run(self, args):
        """(HTTP status, Apple's answer). Status 0 when curl itself failed: Apple was not reached."""
        try:
            r = self.runner(args, input=self._auth(), capture_output=True, text=True, timeout=20)
        except (OSError, subprocess.SubprocessError) as e:
            return 0, str(e)[:200]
        lines = (r.stdout or '').rstrip('\n').rsplit('\n', 1)
        try:
            return int(lines[-1]), (lines[0] if len(lines) > 1 else '')
        except ValueError:
            return 0, (r.stderr or r.stdout or '').strip()

    def push_widgets(self, token, sandbox=None):
        """WidgetKit push (iOS 26): the widgets reload their timelines now. No alert; the system budgets these."""
        if not isinstance(token, str) or not TOKEN_RE.fullmatch(token):
            raise ValueError('bad widget push token')
        args = ['curl', '-sS', '--http2', '--max-time', '15', '-w', '\n%{http_code}',
                '-H', '@-', '-H', f'apns-topic: {self.topic}.push-type.widgets',
                '-H', 'apns-push-type: widgets', '-H', 'apns-priority: 10', '-d', json.dumps({'aps': {'content-changed': True}}),
                f'{self._host(sandbox)}/3/device/{token}']
        return self._run(args)

    def push_activity(self, token, state, end=False, dismiss_after=240, sandbox=None):
        """ActivityKit push: a Live Activity moves along (or ends) while the app is not running. `state` is the
        activity's ContentState as JSON."""
        if not isinstance(token, str) or not TOKEN_RE.fullmatch(token):
            raise ValueError('bad activity push token')
        now = int(self.clock())
        aps = {'timestamp': now, 'event': 'end' if end else 'update', 'content-state': state, 'stale-date': now + 15 * 60}
        if end:
            aps['dismissal-date'] = now + int(dismiss_after)
        args = ['curl', '-sS', '--http2', '--max-time', '15', '-w', '\n%{http_code}',
                '-H', '@-', '-H', f'apns-topic: {self.topic}.push-type.liveactivity',
                '-H', 'apns-push-type: liveactivity', '-H', 'apns-priority: 10', '-d', json.dumps({'aps': aps}),
                f'{self._host(sandbox)}/3/device/{token}']
        return self._run(args)

    def push(self, token, title, body, url, thread='switchboard', collapse=None, sandbox=None):
        """An alert. `thread` groups it on the phone with its kind; pushes with the same `collapse` replace each
        other; one that cannot be delivered within a day is dropped, not shown late."""
        if not isinstance(token, str) or not TOKEN_RE.fullmatch(token):
            raise ValueError('bad device token')
        payload = json.dumps({'aps': {'alert': {'title': title, 'body': body}, 'sound': 'default',
                                      'thread-id': thread}, 'url': url})
        extra = ['-H', f'apns-collapse-id: {collapse[:64]}'] if collapse else []
        args = ['curl', '-sS', '--http2', '--max-time', '15', '-w', '\n%{http_code}',
                '-H', '@-', '-H', f'apns-topic: {self.topic}',
                '-H', 'apns-push-type: alert', '-H', 'apns-priority: 10',
                '-H', f'apns-expiration: {int(time.time()) + 86400}', *extra, '-d', payload,
                f'{self._host(sandbox)}/3/device/{token}']
        return self._run(args)


SENT, GONE, RETRY, FAILED = 'sent', 'gone', 'retry', 'failed'


class Router:
    """Sends a push to the service its token belongs to, and says what became of it.

    Apple runs two services: one for builds installed from Xcode (sandbox), one for TestFlight and the App Store.
    A token is good on one and "BadDeviceToken" on the other, and a phone does not say which it is when it registers.
    So a token Apple calls bad is tried on the other service before anyone forgets it: only a token both refuse,
    or one Apple says is no longer registered, is gone. Where a token worked is remembered while the server runs."""

    def __init__(self, apns, sleep=time.sleep):
        self.apns, self.sleep = apns, sleep
        self.env, self.lock = {}, threading.Lock()  # token -> sandbox (True/False), once a push got through

    def _once(self, call, sandbox):
        status, resp = call(sandbox)
        if status == 0:  # Apple was not reached (a dropped connection, a slow network): once more, a moment later
            self.sleep(1)
            status, resp = call(sandbox)
        return status, resp or ''

    def send(self, token, call, log=lambda line: None):
        """call(sandbox) -> (status, answer). SENT; GONE (forget the token); RETRY (Apple was not reached or said to
        come back later: the token stays); FAILED (Apple refused this push for another reason: the token stays)."""
        with self.lock:
            first = self.env.get(token, bool(self.apns.sandbox))
        status, resp = self._once(call, first)
        log(f'{token[:8]} -> {status} {resp[:120]}')
        if status != 200 and 'BadDeviceToken' in resp:
            other = not first
            status, resp = self._once(call, other)
            log(f'{token[:8]} -> {status} {resp[:120]} ({"development" if other else "released"} builds)')
            if 'BadDeviceToken' in resp:
                return GONE
            first = other
        if status == 200:
            with self.lock:
                if len(self.env) > 500:
                    self.env.clear()
                self.env[token] = first
            return SENT
        if status == 410 or 'Unregistered' in resp:
            return GONE
        return RETRY if status in (0, 429) or status >= 500 else FAILED
