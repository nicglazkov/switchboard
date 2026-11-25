"""Apple push: straight to Apple over HTTP/2 via curl with an ES256 provider token (Apns), or through a push
gateway that holds the key (Gateway). Router works with either."""
import base64, hashlib, hmac, json, os, re, subprocess, threading, time, urllib.error, urllib.request
from pathlib import Path

HOSTS = {True: 'https://api.sandbox.push.apple.com', False: 'https://api.push.apple.com'}
TOKEN_RE = re.compile(r'[0-9a-f]{64,200}')


def _b64(b):
    return base64.urlsafe_b64encode(b).rstrip(b'=').decode()


class Apns:
    def __init__(self, key_path, key_id, team, topic, sandbox=True, runner=subprocess.run, clock=time.time):
        import es256  # plain Python: the server needs nothing installed to sign Apple's token
        self.es256, self.key = es256, es256.load_pem(Path(key_path).read_bytes())
        self.key_id, self.team, self.topic, self.sandbox = key_id, team, topic, sandbox
        self.runner, self.clock = runner, clock
        self._tok, self._iat = None, 0
        self.lock = threading.Lock()

    def jwt(self):
        with self.lock:
            now = int(self.clock())
            if self._tok and now - self._iat < 3000:  # Apple wants a refresh between 20 and 60 min
                return self._tok
            h = _b64(json.dumps({'alg': 'ES256', 'kid': self.key_id}, separators=(',', ':')).encode())
            c = _b64(json.dumps({'iss': self.team, 'iat': now}, separators=(',', ':')).encode())
            sig = self.es256.raw(*self.es256.sign(self.key, f'{h}.{c}'.encode()))
            self._tok, self._iat = f'{h}.{c}.{_b64(sig)}', now
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

# Apple's reason for refusing a push, in words a person can act on. Anything else is shown as Apple gave it.
REASONS = {
    'InvalidProviderToken': 'Apple refused the key. Check "key_id" and "team" in apns.json against the key file.',
    'ExpiredProviderToken': 'Apple says the sign-in token is too old. Check the clock on the home computer.',
    'BadDeviceToken': 'Apple does not know this phone for this app. Open the app on the phone so it registers again.',
    'Unregistered': 'The app was removed from this phone, or notifications were reset. Open the app on the phone.',
    'DeviceTokenNotForTopic': 'The key is for another app than the one on this phone. Check "topic" in apns.json.',
    'TopicDisallowed': 'The key may not send to this app. Check "topic" in apns.json.',
    'BadTopic': 'Apple does not know the app named by "topic" in apns.json.',
    'TooManyRequests': 'Apple asked to slow down for this phone. It will work again shortly.',
    'TooManyProviderTokenUpdates': 'Apple asked to slow down. It will work again shortly.',
}


def why(status, resp):
    """One sentence for a push that was not delivered to Apple: (HTTP status, Apple's answer) -> words."""
    if status == 200:
        return None
    if status == 0:
        return "Apple's notification service could not be reached from the home computer." + (f' ({resp[:80]})' if resp else '')
    reason = None
    try:
        reason = json.loads(resp).get('reason')
    except (ValueError, AttributeError):
        pass
    if reason in REASONS:
        return REASONS[reason]
    if status >= 500:
        return "Apple's notification service has a problem right now. It will be tried again."
    return f'Apple refused the notification: {reason or (resp[:80] if resp else status)}.'


def setup_problem(e):
    """Why apns.json or its key could not be used, in words: the exception from loading them -> one sentence."""
    if isinstance(e, json.JSONDecodeError):
        return 'apns.json is not valid JSON.'
    if isinstance(e, KeyError):
        return f'apns.json has no "{e.args[0]}".'
    if isinstance(e, FileNotFoundError):
        return f'The key file {Path(e.filename).name if e.filename else ""} is missing.'.replace('  ', ' ')
    if isinstance(e, PermissionError):
        return f'The key file {Path(e.filename).name if e.filename else ""} cannot be read by this user.'.replace('  ', ' ')
    text = str(e).strip() or type(e).__name__
    return text[0].upper() + text[1:] + ('' if text.endswith('.') else '.')


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


# ---- Through a gateway: the Apple key stays with whoever published the app, not on this computer

STATE_KEY_RE = re.compile(r'[A-Za-z][A-Za-z0-9_]{0,31}')
FCM_TOKEN_RE = re.compile(r'[A-Za-z0-9_:-]{32,4096}')


class Gateway:
    """The same three pushes as Apns, sent to a push gateway instead of to Apple (apns.json: "mode": "gateway").

    The gateway holds the app's Apple key; this computer holds none. Each request is signed with this computer's
    device key (Ed25519), so the gateway can tell computers apart and limit each one, and it keeps nothing.
    What it and Apple can read is kept small:
    - an alert's words are sealed for the one phone (pushcrypto.seal_push) when that phone gave its key, and
      left out when it did not: the phone then shows a fixed sentence;
    - a Live Activity's state goes as numbers and state words, with no chat title and no link;
    - the value that lets a later alert replace an earlier one is a keyed hash, not the chat's id.
    gateway/README.md has the wire format.

    `recipient(token)` answers the phone's 32 byte X25519 public key (bytes, or base64url text), or None."""

    def __init__(self, url, seed, sandbox=True, recipient=None, platform='apns', opener=None, clock=time.time, rand=None):
        import pushcrypto
        url = str(url).rstrip('/')
        if not re.fullmatch(r'https://[A-Za-z0-9.-]+(:\d+)?(/[A-Za-z0-9._~/-]*)?', url) \
                and not re.fullmatch(r'http://(127\.0\.0\.1|localhost)(:\d+)?', url):
            raise ValueError('the gateway address must start with https://')
        if not isinstance(seed, bytes) or len(seed) != 32:
            raise ValueError('the device key must be 32 bytes')
        if platform not in ('apns', 'fcm'):
            raise ValueError('platform must be apns or fcm')
        self.url, self.seed, self.sandbox, self.platform = url, seed, sandbox, platform
        self.recipient = recipient or (lambda token: None)
        self.opener, self.clock, self.rand, self.pc = opener, clock, rand or os.urandom, pushcrypto
        self.public = pushcrypto.ed25519_public(seed)
        self._collapse_key = hashlib.sha256(b'switchboard collapse v1\x00' + seed).digest()

    @classmethod
    def from_config(cls, conf, cfg_dir, recipient=None):
        """apns.json: {"mode": "gateway", "gateway": "https://...", "device_key": path (default: device.key in the
        config folder, this computer's 32 byte key), "sandbox": true|false}."""
        path = Path(str(conf.get('device_key') or Path(cfg_dir) / 'device.key')).expanduser()
        if not path.is_absolute():
            path = Path(cfg_dir) / path
        seed = path.read_bytes()
        if len(seed) != 32:
            raise ValueError(f'{path.name} is not a 32 byte device key')
        return cls(conf['gateway'], seed, sandbox=conf.get('sandbox', True), recipient=recipient)

    def _token(self, token, what):
        ok = isinstance(token, str) and (TOKEN_RE if self.platform == 'apns' else FCM_TOKEN_RE).fullmatch(token)
        if not ok:
            raise ValueError(f'bad {what} token')

    def _base(self, kind, token, sandbox):
        b = {'v': 1, 'platform': self.platform, 'kind': kind, 'token': token}
        if self.platform == 'apns':
            b['env'] = 'sandbox' if bool(self.sandbox if sandbox is None else sandbox) else 'production'
        return b

    def signed(self, body):
        """(headers, body bytes) for POST /v1/push. Signed: six lines joined by "\\n", the last one the SHA-256
        of exactly the bytes sent."""
        raw = json.dumps(body, separators=(',', ':')).encode()
        when, nonce = str(int(self.clock())), _b64(self.rand(16))
        msg = '\n'.join(['switchboard-gateway-v1', 'POST', '/v1/push', when, nonce, hashlib.sha256(raw).hexdigest()]).encode()
        return {'Content-Type': 'application/json', 'X-SB-Key': _b64(self.public), 'X-SB-Time': when, 'X-SB-Nonce': nonce,
                'X-SB-Signature': _b64(self.pc.ed25519_sign(self.seed, msg, self.public))}, raw

    def _open(self, req):
        if self.opener:
            return self.opener(req, timeout=20)
        handlers = [_NoRedirect()]
        if self.url.startswith('https://'):
            import releases
            handlers.append(urllib.request.HTTPSHandler(context=releases.context()))
        return urllib.request.build_opener(*handlers).open(req, timeout=20)

    def _run(self, body):
        """(status, answer) as Apns._run gives them: Apple's own status and reason when the gateway reached Apple;
        the gateway's status and words when it turned the request away; 0 when the gateway was not reached."""
        headers, raw = self.signed(body)
        req = urllib.request.Request(self.url + '/v1/push', data=raw, headers=headers, method='POST')
        try:
            with self._open(req) as r:
                got = json.loads(r.read(65536).decode('utf-8', 'replace'))
            return int(got['status']), str(got.get('reason') or '')
        except urllib.error.HTTPError as e:
            try:
                why = str(json.loads(e.read(4096).decode('utf-8', 'replace')).get('error') or '')
            except (ValueError, AttributeError, OSError):
                why = ''
            return e.code, f'gateway: {why or e.reason}'[:200]
        except (OSError, ValueError, KeyError, TypeError) as e:  # URLError and timeouts are OSErrors
            return 0, f'gateway not reached: {e}'[:200]

    def seal(self, token, title, body, url, thread):
        """base64url of the sealed alert for this token's phone, or None when the phone gave no key. The body is
        cut until the whole thing fits in one push."""
        key = self.recipient(token)
        if isinstance(key, str):
            try:
                key = base64.urlsafe_b64decode(key + '=' * (-len(key) % 4))
            except ValueError:
                key = None
        if not isinstance(key, bytes) or len(key) != 32:
            return None
        title, body = str(title)[:200], str(body)
        while True:
            fields = {'v': 1, 't': int(self.clock()), 'title': title, 'body': body, 'url': str(url)[:300], 'thread': str(thread)[:64]}
            sealed = self.pc.seal_push(self.seed, key, fields, self.public)
            if len(sealed) <= self.pc.PUSH_MAX:
                return _b64(sealed)
            body = body[:max(0, len(body) - max(16, (len(sealed) - self.pc.PUSH_MAX) // 4))]

    def push(self, token, title, body, url, thread='switchboard', collapse=None, sandbox=None):
        self._token(token, 'device')
        b = self._base('alert', token, sandbox)
        try:
            sealed = self.seal(token, title, body, url, thread)
        except ValueError:  # a key no secret can be made with: the phone gets the fixed sentence
            sealed = None
        if sealed:
            b['ciphertext'] = sealed
        if collapse:
            b['collapse'] = _b64(hmac.new(self._collapse_key, str(collapse).encode(), hashlib.sha256).digest()[:16])
        return self._run(b)

    def push_widgets(self, token, sandbox=None):
        self._token(token, 'widget push')
        return self._run(self._base('widgets', token, sandbox))

    def push_activity(self, token, state, end=False, dismiss_after=240, sandbox=None):
        self._token(token, 'activity push')
        b = self._base('liveactivity', token, sandbox)
        b['activity'] = {'event': 'end' if end else 'update', 'state': state_words(state)}
        if end:
            b['activity']['dismiss_after'] = max(0, min(int(dismiss_after), 86400))
        return self._run(b)


def state_words(state):
    """A Live Activity's state as it may leave this computer: numbers, true and false, and nothing. Every text
    stays home (the chat's name in `step`, the link with the computer's name), because the gateway and Apple can
    read a Live Activity push and a one-word chat name would look like a state word to both. `step` becomes one
    of three words: failed, done or working."""
    out = {}
    for k, v in state.items():
        if STATE_KEY_RE.fullmatch(str(k)) and (v is None or isinstance(v, (bool, int)) or (isinstance(v, float) and v == v
                                                                                           and abs(v) != float('inf'))):
            out[str(k)] = v
    out['step'] = 'failed' if state.get('failed') else 'done' if state.get('done') else 'working'
    return out


class _NoRedirect(urllib.request.HTTPRedirectHandler):
    """A signed push goes to the gateway named in apns.json and nowhere it points on to."""
    def redirect_request(self, *a, **kw):
        return None
