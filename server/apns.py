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

    def push_widgets(self, token):
        """WidgetKit push (iOS 26): the widgets reload their timelines now. No alert; the system budgets these."""
        if not isinstance(token, str) or not TOKEN_RE.fullmatch(token):
            raise ValueError('bad widget push token')
        args = ['curl', '-sS', '--http2', '--max-time', '15', '-w', '\n%{http_code}',
                '-H', '@-', '-H', f'apns-topic: {self.topic}.push-type.widgets',
                '-H', 'apns-push-type: widgets', '-H', 'apns-priority: 10', '-d', json.dumps({'aps': {'content-changed': True}}),
                f'{HOSTS[self.sandbox]}/3/device/{token}']
        r = self.runner(args, input=self._auth(), capture_output=True, text=True, timeout=20)
        lines = (r.stdout or '').rstrip('\n').rsplit('\n', 1)
        try:
            return int(lines[-1]), (lines[0] if len(lines) > 1 else '')
        except ValueError:
            return 0, (r.stderr or r.stdout or '').strip()

    def push_activity(self, token, state, end=False, dismiss_after=240):
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
                f'{HOSTS[self.sandbox]}/3/device/{token}']
        r = self.runner(args, input=self._auth(), capture_output=True, text=True, timeout=20)
        lines = (r.stdout or '').rstrip('\n').rsplit('\n', 1)
        try:
            return int(lines[-1]), (lines[0] if len(lines) > 1 else '')
        except ValueError:
            return 0, (r.stderr or r.stdout or '').strip()

    def push(self, token, title, body, url):
        if not isinstance(token, str) or not TOKEN_RE.fullmatch(token):
            raise ValueError('bad device token')
        payload = json.dumps({'aps': {'alert': {'title': title, 'body': body}, 'sound': 'default',
                                      'thread-id': 'switchboard'}, 'url': url})
        args = ['curl', '-sS', '--http2', '--max-time', '15', '-w', '\n%{http_code}',
                '-H', '@-', '-H', f'apns-topic: {self.topic}',
                '-H', 'apns-push-type: alert', '-H', 'apns-priority: 10', '-d', payload,
                f'{HOSTS[self.sandbox]}/3/device/{token}']
        r = self.runner(args, input=self._auth(), capture_output=True, text=True, timeout=20)
        lines = (r.stdout or '').rstrip('\n').rsplit('\n', 1)
        try:
            return int(lines[-1]), (lines[0] if len(lines) > 1 else '')
        except ValueError:
            return 0, (r.stderr or r.stdout or '').strip()
