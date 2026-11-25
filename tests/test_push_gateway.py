"""Pushes through a gateway (server/apns.py Gateway): what is signed, what is sealed, what a gateway that stands in
here is sent, and what the server makes of its answers. The stand-in checks the signature the way the real one
does (gateway/gateway.go); tests/fixtures/gateway_request_py.json and push_sealed_py.json are the same bytes the
Go tests read, so the two sides cannot drift apart unnoticed."""
import base64, hashlib, json, os, sys, tempfile, threading, unittest
from http.server import BaseHTTPRequestHandler, HTTPServer
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'server'))
import apns, pushcrypto, server  # noqa: E402

FIXTURES = Path(__file__).resolve().parent / 'fixtures'
SEED = hashlib.sha256(b'switchboard test home').digest()
PHONE = hashlib.sha256(b'switchboard test phone').digest()
EPHEMERAL = hashlib.sha256(b'switchboard test ephemeral').digest()
TOKEN = 'ab' * 32
NOW = 1700000000


def unb64(s):
    return base64.urlsafe_b64decode(s + '=' * (-len(s) % 4))


class FakeGateway(BaseHTTPRequestHandler):
    """Checks the signature as the gateway does, keeps what it was sent, and answers what the test set."""
    seen, answer, code = [], {'status': 200, 'reason': ''}, 200

    def do_POST(self):
        cls = type(self)
        raw = self.rfile.read(int(self.headers.get('Content-Length') or 0))
        h = self.headers
        msg = '\n'.join(['switchboard-gateway-v1', 'POST', self.path, h['X-SB-Time'], h['X-SB-Nonce'],
                         hashlib.sha256(raw).hexdigest()]).encode()
        ok = pushcrypto.ed25519_verify(unb64(h['X-SB-Key']), msg, unb64(h['X-SB-Signature']))
        cls.seen.append({'path': self.path, 'body': json.loads(raw), 'raw': raw, 'headers': h, 'signed': ok})
        code, answer = (cls.code, cls.answer) if ok else (401, {'error': 'the signature does not match'})
        if code in (301, 302, 307, 308):
            self.send_response(code)
            self.send_header('Location', answer)
            self.send_header('Content-Length', '0')
            self.end_headers()
            return
        out = json.dumps(answer).encode()
        self.send_response(code)
        self.send_header('Content-Length', str(len(out)))
        self.end_headers()
        self.wfile.write(out)

    def log_message(self, *a):
        pass


class GatewayCase(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.httpd = HTTPServer(('127.0.0.1', 0), FakeGateway)
        cls.url = f'http://127.0.0.1:{cls.httpd.server_address[1]}'
        threading.Thread(target=cls.httpd.serve_forever, daemon=True).start()

    @classmethod
    def tearDownClass(cls):
        cls.httpd.shutdown()
        cls.httpd.server_close()

    def setUp(self):
        FakeGateway.seen, FakeGateway.answer, FakeGateway.code = [], {'status': 200, 'reason': ''}, 200
        self.keys = {}
        self.g = apns.Gateway(self.url, SEED, recipient=self.keys.get, clock=lambda: NOW)

    def sent(self):
        self.assertEqual(len(FakeGateway.seen), 1)
        return FakeGateway.seen[0]


class Signing(GatewayCase):
    def test_a_push_is_signed_with_the_device_key(self):
        self.assertEqual(self.g.push_widgets(TOKEN), (200, ''))
        got = self.sent()
        self.assertTrue(got['signed'])
        self.assertEqual(got['path'], '/v1/push')
        h = got['headers']
        self.assertEqual(unb64(h['X-SB-Key']), pushcrypto.ed25519_public(SEED))
        self.assertEqual(h['X-SB-Time'], str(NOW))
        self.assertEqual(len(unb64(h['X-SB-Nonce'])), 16)
        self.assertEqual(len(unb64(h['X-SB-Signature'])), 64)
        self.assertEqual(got['body'], {'v': 1, 'platform': 'apns', 'kind': 'widgets', 'token': TOKEN, 'env': 'sandbox'})

    def test_every_request_has_its_own_nonce(self):
        self.g.push_widgets(TOKEN)
        self.g.push_widgets(TOKEN)
        a, b = (s['headers']['X-SB-Nonce'] for s in FakeGateway.seen)
        self.assertNotEqual(a, b)

    def test_the_signature_covers_the_exact_bytes(self):
        headers, raw = self.g.signed({'v': 1, 'kind': 'widgets'})
        self.assertEqual(raw, b'{"v":1,"kind":"widgets"}')
        msg = b'switchboard-gateway-v1\nPOST\n/v1/push\n1700000000\n' + headers['X-SB-Nonce'].encode() + b'\n' + \
            hashlib.sha256(raw).hexdigest().encode()
        self.assertTrue(pushcrypto.ed25519_verify(self.g.public, msg, unb64(headers['X-SB-Signature'])))

    def test_the_address_must_be_https(self):
        for bad in ('http://push.example.com', 'ftp://x', 'https://', 'https://push.example.com/?x=1', 'push.example.com'):
            with self.assertRaises(ValueError):
                apns.Gateway(bad, SEED)
        self.assertEqual(apns.Gateway('https://push.example.com/', SEED).url, 'https://push.example.com')

    def test_the_key_must_be_32_bytes(self):
        with self.assertRaises(ValueError):
            apns.Gateway(self.url, b'short')


class Alerts(GatewayCase):
    def test_without_the_phones_key_nothing_readable_is_sent(self):
        self.g.push(TOKEN, 'Held: fix the tests', 'on desk', '/s/desk/S1', thread='held', collapse='desk/S1')
        got = self.sent()
        self.assertEqual(set(got['body']), {'v', 'platform', 'kind', 'token', 'env', 'collapse'})
        for word in (b'Held', b'desk', b'S1', b'held'):
            self.assertNotIn(word, got['raw'])

    def test_with_the_phones_key_the_words_are_sealed_for_it(self):
        self.keys[TOKEN] = apns._b64(pushcrypto.x25519_public(PHONE))
        self.g.push(TOKEN, 'Held: fix the tests', 'on desk', '/s/desk/S1', thread='held')
        got = self.sent()
        # The sealed part is random bytes written as letters, where a short word like "S1" turns up by chance now and
        # then. So: nothing beside it carries a word, and inside it none of what was said can be read.
        text = got['body']['ciphertext']
        self.assertEqual(got['raw'].count(text.encode()), 1)
        beside = got['raw'].replace(text.encode(), b'')
        for word in (b'Held', b'desk', b'S1', b'held'):
            self.assertNotIn(word, beside)
        sealed = unb64(text)
        for said in (b'Held: fix the tests', b'on desk', b'/s/desk/S1', b'"title"', b'"thread"'):
            self.assertNotIn(said, sealed)
            self.assertNotIn(said, text.encode())
        fields = pushcrypto.open_push(PHONE, pushcrypto.ed25519_public(SEED), sealed)
        self.assertEqual(fields, {'v': 1, 't': NOW, 'title': 'Held: fix the tests', 'body': 'on desk', 'url': '/s/desk/S1',
                                  'thread': 'held'})
        # ... and only that phone opens it
        with self.assertRaises(ValueError):
            pushcrypto.open_push(hashlib.sha256(b'another phone').digest(), pushcrypto.ed25519_public(SEED), sealed)

    def test_a_long_body_is_cut_to_fit(self):
        self.keys[TOKEN] = pushcrypto.x25519_public(PHONE)  # bytes work too
        self.g.push(TOKEN, 'T', 'é\U0001f600"' * 3000, '/s/desk/S1')
        sealed = unb64(self.sent()['body']['ciphertext'])
        self.assertLessEqual(len(sealed), pushcrypto.PUSH_MAX)
        self.assertGreater(len(sealed), pushcrypto.PUSH_MAX - 200)
        self.assertLessEqual(len(self.sent()['raw']), 8192)  # the gateway's limit for a request
        fields = pushcrypto.open_push(PHONE, pushcrypto.ed25519_public(SEED), sealed)
        self.assertTrue(fields['body'].startswith('é\U0001f600"'))

    def test_a_key_that_is_not_one_means_the_fixed_sentence(self):
        for bad in ('not base64 !', apns._b64(b'short'), bytes(32), 7):
            FakeGateway.seen = []
            self.keys[TOKEN] = bad
            self.assertEqual(self.g.push(TOKEN, 'T', 'b', '/'), (200, ''))
            self.assertNotIn('ciphertext', self.sent()['body'])

    def test_collapse_is_a_keyed_hash(self):
        self.g.push(TOKEN, 'T', 'b', '/', collapse='desk/S1')
        self.g.push(TOKEN, 'T', 'b', '/', collapse='desk/S1')
        self.g.push(TOKEN, 'T', 'b', '/', collapse='desk/S2')
        a, b, c = (s['body']['collapse'] for s in FakeGateway.seen)
        self.assertEqual(a, b)
        self.assertNotEqual(a, c)
        self.assertRegex(a, r'^[A-Za-z0-9_-]{22}$')
        other = apns.Gateway(self.url, hashlib.sha256(b'another home').digest())
        other.push(TOKEN, 'T', 'b', '/', collapse='desk/S1')
        self.assertNotEqual(FakeGateway.seen[-1]['body']['collapse'], a)  # another computer, another value

    def test_development_or_released_builds(self):
        self.g.push(TOKEN, 'T', 'b', '/')
        self.g.push(TOKEN, 'T', 'b', '/', sandbox=False)
        apns.Gateway(self.url, SEED, sandbox=False).push_widgets(TOKEN)
        self.assertEqual([s['body']['env'] for s in FakeGateway.seen], ['sandbox', 'production', 'production'])

    def test_a_bad_token_is_refused_here(self):
        for call in (lambda: self.g.push('../../x', 't', 'b', '/'), lambda: self.g.push_widgets('zz'),
                     lambda: self.g.push_activity(None, {})):
            with self.assertRaises(ValueError):
                call()
        self.assertEqual(FakeGateway.seen, [])

    def test_android_tokens(self):
        g = apns.Gateway(self.url, SEED, platform='fcm')
        tok = 'fGx-' + 'A1b2_C3:d4' * 14
        self.assertEqual(g.push(tok, 'T', 'b', '/'), (200, ''))
        self.assertEqual(self.sent()['body'], {'v': 1, 'platform': 'fcm', 'kind': 'alert', 'token': tok})


class LiveActivity(GatewayCase):
    def test_only_numbers_and_states_leave(self):
        state = {'step': 'Fix the flaky tests', 'progress': 0.7, 'done': False, 'failed': False,
                 'link': 'switchboard://session/desk/0123', 'bad key!': 1, 'nan': float('nan')}
        self.g.push_activity(TOKEN, state)
        got = self.sent()
        self.assertEqual(got['body']['activity'], {'event': 'update', 'state': {'step': 'working', 'progress': 0.7, 'done': False,
                                                                                'failed': False}})
        for word in (b'Fix', b'flaky', b'desk', b'0123'):
            self.assertNotIn(word, got['raw'])

    def test_a_one_word_name_does_not_pass_as_a_state(self):
        self.assertEqual(apns.state_words({'step': 'refactor', 'done': True, 'failed': False})['step'], 'done')
        self.assertEqual(apns.state_words({'step': 'boom', 'failed': True})['step'], 'failed')
        self.assertEqual(apns.state_words({'step': 'secret', 'link': None}), {'step': 'working', 'link': None})

    def test_the_end(self):
        self.g.push_activity(TOKEN, {'step': 'New chat name', 'progress': 1.0, 'done': True, 'failed': False, 'link': None},
                             end=True, dismiss_after=240)
        self.assertEqual(self.sent()['body']['activity'],
                         {'event': 'end', 'dismiss_after': 240,
                          'state': {'step': 'done', 'progress': 1.0, 'done': True, 'failed': False, 'link': None}})


class Answers(GatewayCase):
    """What the gateway and Apple say, as apns.Router reads it."""

    def router(self):
        return apns.Router(self.g, sleep=lambda s: None)

    def send(self):
        lines = []
        got = self.router().send(TOKEN, lambda sb: self.g.push(TOKEN, 'T', 'b', '/', sandbox=sb), lines.append)
        return got, lines

    def test_sent(self):
        self.assertEqual(self.send()[0], apns.SENT)

    def test_apple_says_the_token_is_gone(self):
        FakeGateway.answer = {'status': 410, 'reason': 'Unregistered'}
        self.assertEqual(self.send()[0], apns.GONE)

    def test_a_token_of_the_other_service_is_tried_there(self):
        FakeGateway.answer = {'status': 400, 'reason': 'BadDeviceToken'}
        self.assertEqual(self.send()[0], apns.GONE)
        self.assertEqual([s['body']['env'] for s in FakeGateway.seen], ['sandbox', 'production'])

    def test_the_gateway_says_slow_down(self):
        FakeGateway.code, FakeGateway.answer = 429, {'error': 'too many pushes, try again in a minute'}
        got, lines = self.send()
        self.assertEqual(got, apns.RETRY)
        self.assertIn('429 gateway: too many pushes', lines[0])

    def test_the_gateway_refuses_the_request(self):
        FakeGateway.code, FakeGateway.answer = 401, {'error': "the request's clock is more than five minutes from ours"}
        got, lines = self.send()
        self.assertEqual(got, apns.FAILED)  # the token stays: nothing is wrong with the phone
        self.assertIn("clock is more than five minutes", lines[0])

    def test_the_gateway_could_not_reach_apple(self):
        FakeGateway.code, FakeGateway.answer = 502, {'error': 'the push service was not reached'}
        self.assertEqual(self.send()[0], apns.RETRY)

    def test_the_gateway_is_not_reached(self):
        self.g = apns.Gateway('http://127.0.0.1:1', SEED)
        status, why = self.g.push_widgets(TOKEN)
        self.assertEqual(status, 0)
        self.assertTrue(why.startswith('gateway not reached'))
        self.assertEqual(self.send()[0], apns.RETRY)

    def test_an_answer_that_is_not_one(self):
        FakeGateway.answer = ['nonsense']
        self.assertEqual(self.g.push_widgets(TOKEN)[0], 0)

    def test_a_redirect_is_not_followed(self):
        FakeGateway.code, FakeGateway.answer = 307, self.url + '/elsewhere'
        status, why = self.g.push_widgets(TOKEN)
        self.assertEqual(status, 307)
        self.assertEqual([s['path'] for s in FakeGateway.seen], ['/v1/push'])

    def test_the_log_line_has_no_whole_token(self):
        got, lines = self.send()
        self.assertNotIn(TOKEN, ''.join(lines))


class Config(unittest.TestCase):
    def setUp(self):
        self.cfg = Path(tempfile.mkdtemp())
        (self.cfg / 'device.key').write_bytes(SEED)

    def test_gateway_mode(self):
        g = server.push_sender(self.cfg, {'mode': 'gateway', 'gateway': 'https://push.example.com', 'sandbox': False})
        self.assertIsInstance(g, apns.Gateway)
        self.assertEqual((g.url, g.sandbox, g.seed), ('https://push.example.com', False, SEED))
        self.assertTrue(all(hasattr(g, m) for m in ('push', 'push_widgets', 'push_activity', 'sandbox')))

    def test_the_key_file_can_be_named(self):
        (self.cfg / 'other.key').write_bytes(PHONE)
        for name in ('other.key', str(self.cfg / 'other.key')):
            g = server.push_sender(self.cfg, {'mode': 'gateway', 'gateway': 'https://push.example.com', 'device_key': name})
            self.assertEqual(g.seed, PHONE)

    def test_what_is_wrong_is_said(self):
        with self.assertRaisesRegex(ValueError, 'not a 32 byte device key'):
            (self.cfg / 'device.key').write_bytes(b'x' * 31)
            server.push_sender(self.cfg, {'mode': 'gateway', 'gateway': 'https://push.example.com'})
        (self.cfg / 'device.key').unlink()
        with self.assertRaises(OSError):
            server.push_sender(self.cfg, {'mode': 'gateway', 'gateway': 'https://push.example.com'})
        with self.assertRaises(KeyError):
            (self.cfg / 'device.key').write_bytes(SEED)
            server.push_sender(self.cfg, {'mode': 'gateway'})
        with self.assertRaisesRegex(ValueError, 'mode is'):
            server.push_sender(self.cfg, {'mode': 'carrier-pigeon'})

    def test_direct_mode_is_the_default_and_unchanged(self):
        try:
            from cryptography.hazmat.primitives.asymmetric import ec
            from cryptography.hazmat.primitives import serialization as s
        except ImportError:
            self.skipTest('cryptography is not installed')
        (self.cfg / 'apns').mkdir()
        (self.cfg / 'apns' / 'AuthKey_KID.p8').write_bytes(ec.generate_private_key(ec.SECP256R1()).private_bytes(
            s.Encoding.PEM, s.PrivateFormat.PKCS8, s.NoEncryption()))
        for conf in ({'key_id': 'KID', 'team': 'TEAM', 'topic': 'com.example.sb'},
                     {'mode': 'direct', 'key_id': 'KID', 'team': 'TEAM', 'topic': 'com.example.sb', 'sandbox': False}):
            a = server.push_sender(self.cfg, conf)
            self.assertIsInstance(a, apns.Apns)
            self.assertEqual((a.key_id, a.team, a.topic, a.sandbox), ('KID', 'TEAM', 'com.example.sb', conf.get('sandbox', True)))

    def test_direct_mode_without_its_fields_is_still_an_error(self):
        with self.assertRaises(KeyError):
            server.push_sender(self.cfg, {})


class PhoneKey(unittest.TestCase):
    """A phone may give its sealing key when it registers; the gateway sender finds it by token."""

    def setUp(self):
        import fleet
        self.app = server.App(fleet.Fleet({}, None), state_dir=Path(tempfile.mkdtemp()),
                              web_dir=Path(__file__).resolve().parents[1] / 'server' / 'web', pusher=lambda *a: None)

    def test_the_key_is_kept_with_the_token(self):
        key = apns._b64(pushcrypto.x25519_public(PHONE))
        self.app.post('/api/devices', {'token': TOKEN, 'name': 'Phone', 'seal': key}, None)
        self.assertEqual(self.app.push_key(TOKEN), key)
        self.assertIsNone(self.app.push_key('cd' * 32))

    def test_a_phone_without_one_and_a_bad_one(self):
        self.app.post('/api/devices', {'token': TOKEN, 'name': 'Phone'}, None)
        self.assertIsNone(self.app.push_key(TOKEN))
        self.assertNotIn('seal', self.app.devices()[TOKEN])
        self.app.post('/api/devices', {'token': TOKEN, 'name': 'Phone', 'seal': '../etc'}, None)
        self.assertIsNone(self.app.push_key(TOKEN))


def fixtures():
    """The two files the Go tests read. Every input is fixed, so they come out the same each time."""
    recipient = pushcrypto.x25519_public(PHONE)
    fields = {'v': 1, 't': NOW, 'title': 'Held: fix the tests', 'body': 'café · desk', 'url': '/s/desk/S1', 'thread': 'held'}
    sealed = pushcrypto.seal_push(SEED, recipient, fields, ephemeral=EPHEMERAL)
    g = apns.Gateway('https://push.example.com', SEED, clock=lambda: NOW, rand=lambda n: bytes(range(n)))
    headers, raw = g.signed({'v': 1, 'platform': 'apns', 'kind': 'alert', 'token': TOKEN, 'env': 'production',
                             'ciphertext': apns._b64(sealed), 'collapse': 'c2Vzc2lvbg'})
    return {
        'push_sealed_py.json': {'home_public': pushcrypto.ed25519_public(SEED).hex(), 'phone_secret': PHONE.hex(),
                                'phone_public': recipient.hex(), 'message': json.dumps(fields, ensure_ascii=False, separators=(',', ':')),
                                'sealed': sealed.hex()},
        'gateway_request_py.json': {'time': NOW, 'headers': {k: v for k, v in headers.items() if k.startswith('X-SB-')},
                                    'body': raw.decode()},
    }


class Fixtures(unittest.TestCase):
    def test_the_files_the_go_tests_read_are_what_this_code_makes(self):
        for name, want in fixtures().items():
            if os.environ.get('SB_WRITE_FIXTURES'):
                (FIXTURES / name).write_text(json.dumps(want, indent=1, ensure_ascii=False) + '\n', encoding='utf-8')
            self.assertEqual(json.loads((FIXTURES / name).read_text(encoding='utf-8')), want, name)


if __name__ == '__main__':
    unittest.main()
