"""ES256 signatures (ECDSA on P-256 with SHA-256) in plain Python, for the token Apple's push service asks for.

Why it is written out here: the server runs on the Python a computer already has, or on the one Switchboard
brings, with nothing installed, and Python's own library cannot sign with an elliptic curve key. The nonce is
the deterministic one of RFC 6979, so no random source is involved, and the code is checked against that RFC's
test vectors (tests/test_es256.py).

It is not constant time. It signs one short token every 50 minutes on the home computer, which nobody else can
time, so that is accepted here. Do not use it where an attacker can time many signatures with a key."""
import base64, hashlib, hmac, re

P = 0xFFFFFFFF00000001000000000000000000000000FFFFFFFFFFFFFFFFFFFFFFFF
N = 0xFFFFFFFF00000000FFFFFFFFFFFFFFFFBCE6FAADA7179E84F3B9CAC2FC632551
B = 0x5AC635D8AA3A93E7B3EBBD55769886BC651D06B0CC53B0F63BCE3C3E27D2604B
G = (0x6B17D1F2E12C4247F8BCE6E563A440F277037D812DEB33A0F4A13945D898C296,
     0x4FE342E2FE1A7F9B8EE7EB4A7C0F9E162BCE33576B315ECECBB6406837BF51F5)
_OID_P256 = bytes.fromhex('2a8648ce3d030107')


def _add(a, b):
    """Two points of the curve added; None is the point at infinity."""
    if a is None:
        return b
    if b is None:
        return a
    (x1, y1), (x2, y2) = a, b
    if x1 == x2:
        if (y1 + y2) % P == 0:
            return None
        m = (3 * x1 * x1 - 3) * pow(2 * y1, -1, P) % P
    else:
        m = (y2 - y1) * pow(x2 - x1, -1, P) % P
    x3 = (m * m - x1 - x2) % P
    return x3, (m * (x1 - x3) - y1) % P


def _mul(k, pt):
    out = None
    while k:
        if k & 1:
            out = _add(out, pt)
        pt = _add(pt, pt)
        k >>= 1
    return out


def on_curve(pt):
    x, y = pt
    return 0 <= x < P and 0 <= y < P and (y * y - (x * x * x - 3 * x + B)) % P == 0


def public(d):
    """The public point of private number d."""
    return _mul(d, G)


def _nonce(d, digest):
    """RFC 6979 section 3.2 with HMAC-SHA256: the same key and message always give the same nonce."""
    x, h1 = d.to_bytes(32, 'big'), (int.from_bytes(digest, 'big') % N).to_bytes(32, 'big')
    v, k = b'\x01' * 32, b'\x00' * 32
    k = hmac.new(k, v + b'\x00' + x + h1, hashlib.sha256).digest()
    v = hmac.new(k, v, hashlib.sha256).digest()
    k = hmac.new(k, v + b'\x01' + x + h1, hashlib.sha256).digest()
    v = hmac.new(k, v, hashlib.sha256).digest()
    while True:
        v = hmac.new(k, v, hashlib.sha256).digest()
        cand = int.from_bytes(v, 'big')
        if 1 <= cand < N:
            return cand
        k = hmac.new(k, v + b'\x00', hashlib.sha256).digest()
        v = hmac.new(k, v, hashlib.sha256).digest()


def sign(d, msg):
    """(r, s) for msg, signed with private number d."""
    if not 1 <= d < N:
        raise ValueError('not a P-256 private key')
    digest = hashlib.sha256(msg).digest()
    k = _nonce(d, digest)
    r = _mul(k, G)[0] % N
    s = pow(k, -1, N) * (int.from_bytes(digest, 'big') + r * d) % N
    if not (r and s):  # one chance in 2^256; the caller's next token has another timestamp
        raise ValueError('this message cannot be signed with this key')
    return r, s


def verify(pub, msg, r, s):
    """True when (r, s) is pub's signature of msg."""
    if not (on_curve(pub) and 1 <= r < N and 1 <= s < N):
        return False
    e = int.from_bytes(hashlib.sha256(msg).digest(), 'big') % N
    w = pow(s, -1, N)
    pt = _add(_mul(e * w % N, G), _mul(r * w % N, pub))
    return pt is not None and pt[0] % N == r


def raw(r, s):
    """The 64 bytes a JWT carries: r then s, 32 bytes each."""
    return r.to_bytes(32, 'big') + s.to_bytes(32, 'big')


# ---- reading the key file Apple gives (AuthKey_XXXX.p8: PEM, PKCS #8) ----

def _der(b, i=0):
    """(tag, value, index after it) of the DER item at b[i]."""
    if i + 2 > len(b):
        raise ValueError('key file ends early')
    tag, n, i = b[i], b[i + 1], i + 2
    if n & 0x80:
        size = n & 0x7F
        if not 1 <= size <= 4 or i + size > len(b):
            raise ValueError('key file has a length this cannot read')
        n, i = int.from_bytes(b[i:i + size], 'big'), i + size
    if i + n > len(b):
        raise ValueError('key file ends early')
    return tag, b[i:i + n], i + n


def _items(b):
    out, i = [], 0
    while i < len(b):
        tag, val, i = _der(b, i)
        out.append((tag, val))
    return out


def _sec1(body):
    """The private number in an ECPrivateKey (RFC 5915): SEQUENCE { 1, OCTET STRING, ... }."""
    items = _items(body)
    if len(items) < 2 or items[0] != (0x02, b'\x01') or items[1][0] != 0x04:
        raise ValueError('not an elliptic curve private key')
    return int.from_bytes(items[1][1], 'big'), items[2:]


def load_pem(text):
    """The private number of a P-256 key in PEM: PKCS #8 ("PRIVATE KEY", what Apple gives) or SEC 1
    ("EC PRIVATE KEY"). An encrypted key, or one on another curve, is refused with a line that says so."""
    if isinstance(text, bytes):
        text = text.decode('ascii', 'replace')
    m = re.search(r'-----BEGIN ((?:EC |ENCRYPTED )?PRIVATE KEY)-----(.*?)-----END \1-----', text, re.S)
    if not m:
        raise ValueError('no private key in the file')
    kind = m.group(1)
    if kind.startswith('ENCRYPTED'):
        raise ValueError('the key file is encrypted with a password; Apple\'s .p8 files are not')
    try:
        der = base64.b64decode(''.join(m.group(2).split()), validate=True)
    except ValueError:
        raise ValueError('the key file is not valid base64')
    tag, body, _ = _der(der)
    if tag != 0x30:
        raise ValueError('not a private key file')
    if kind == 'EC PRIVATE KEY':
        d, rest = _sec1(body)
        if not any(t == 0xA0 and _OID_P256 in v for t, v in rest):
            raise ValueError('the key is not on the P-256 curve')
    else:
        items = _items(body)
        if len(items) < 3 or items[0][0] != 0x02 or items[1][0] != 0x30 or items[2][0] != 0x04:
            raise ValueError('not a PKCS #8 private key')
        if _OID_P256 not in items[1][1]:
            raise ValueError('the key is not on the P-256 curve')
        tag, inner, _ = _der(items[2][1])
        if tag != 0x30:
            raise ValueError('not an elliptic curve private key')
        d, _ = _sec1(inner)
    if not 1 <= d < N:
        raise ValueError('not a P-256 private key')
    return d
