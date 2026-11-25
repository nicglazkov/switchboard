"""The cryptography for pushes sent through the gateway, in plain Python: Ed25519 signatures (RFC 8032), and
HPKE (RFC 9180) with X25519, HKDF-SHA256 and ChaCha20-Poly1305 (RFC 7748, 5869, 8439).

Why it is written out here: the server runs on the Python a computer already has, with nothing installed, and
Python's own library has none of these. The code follows the RFCs' reference algorithms and is checked against
their test vectors (tests/test_pushcrypto.py), and against Go's crypto/hpke and crypto/ed25519 by the gateway's
tests (gateway/interop_test.go).

It is not constant time. It signs and seals a few short messages a minute on the home computer, for a gateway
and a phone that never see how long that took to the microsecond, so that is accepted here. Do not use it where
an attacker can time many operations on a key.

The sealed notification format ("switchboard push v1") is at the end; gateway/README.md documents it for the
phone side."""
import hashlib, hmac, json, os

P = 2 ** 255 - 19

# ---- Ed25519 (RFC 8032, section 5.1 and the reference code in section 6)

L = 2 ** 252 + 27742317777372353535851937790883648493
D = -121665 * pow(121666, P - 2, P) % P
SQRT_M1 = pow(2, (P - 1) // 4, P)


def _recover_x(y, sign):
    if y >= P:
        return None
    x2 = (y * y - 1) * pow(D * y * y + 1, P - 2, P) % P
    if x2 == 0:
        return None if sign else 0
    x = pow(x2, (P + 3) // 8, P)
    if (x * x - x2) % P != 0:
        x = x * SQRT_M1 % P
    if (x * x - x2) % P != 0:
        return None
    return P - x if (x & 1) != sign else x


_GY = 4 * pow(5, P - 2, P) % P
_GX = _recover_x(_GY, 0)
G = (_GX, _GY, 1, _GX * _GY % P)  # extended coordinates (X, Y, Z, T), x = X/Z, y = Y/Z, x*y = T/Z


def _add(a, b):
    A = (a[1] - a[0]) * (b[1] - b[0]) % P
    B = (a[1] + a[0]) * (b[1] + b[0]) % P
    C = 2 * a[3] * b[3] * D % P
    DD = 2 * a[2] * b[2] % P
    E, F, G_, H = B - A, DD - C, DD + C, B + A
    return (E * F % P, G_ * H % P, F * G_ % P, E * H % P)


def _mul(s, pt):
    q = (0, 1, 1, 0)
    while s > 0:
        if s & 1:
            q = _add(q, pt)
        pt = _add(pt, pt)
        s >>= 1
    return q


def _compress(pt):
    zinv = pow(pt[2], P - 2, P)
    x, y = pt[0] * zinv % P, pt[1] * zinv % P
    return (y | ((x & 1) << 255)).to_bytes(32, 'little')


def _decompress(b):
    if len(b) != 32:
        return None
    y = int.from_bytes(b, 'little')
    sign = y >> 255
    y &= (1 << 255) - 1
    x = _recover_x(y, sign)
    return None if x is None else (x, y, 1, x * y % P)


def _expand(seed):
    if len(seed) != 32:
        raise ValueError('an Ed25519 seed is 32 bytes')
    h = hashlib.sha512(seed).digest()
    a = int.from_bytes(h[:32], 'little')
    a &= (1 << 254) - 8
    a |= 1 << 254
    return a, h[32:]


def ed25519_public(seed):
    """The 32 byte public key of a 32 byte seed (what iroh calls the secret key and the endpoint id)."""
    return _compress(_mul(_expand(seed)[0], G))


def ed25519_sign(seed, msg, public=None):
    """The 64 byte signature. `public` saves working the public key out again."""
    a, prefix = _expand(seed)
    A = public or _compress(_mul(a, G))
    r = int.from_bytes(hashlib.sha512(prefix + msg).digest(), 'little') % L
    R = _compress(_mul(r, G))
    k = int.from_bytes(hashlib.sha512(R + A + msg).digest(), 'little') % L
    return R + ((r + k * a) % L).to_bytes(32, 'little')


def ed25519_verify(public, msg, sig):
    if len(public) != 32 or len(sig) != 64:
        return False
    A, R = _decompress(public), _decompress(sig[:32])
    s = int.from_bytes(sig[32:], 'little')
    if A is None or R is None or s >= L:
        return False
    k = int.from_bytes(hashlib.sha512(sig[:32] + public + msg).digest(), 'little') % L
    left, right = _mul(s, G), _add(R, _mul(k, A))
    return (left[0] * right[2] - right[0] * left[2]) % P == 0 and (left[1] * right[2] - right[1] * left[2]) % P == 0


# ---- X25519 (RFC 7748, section 5)

def x25519(k, u):
    if len(k) != 32 or len(u) != 32:
        raise ValueError('X25519 keys are 32 bytes')
    k = int.from_bytes(k, 'little')
    k &= (1 << 254) - 8
    k |= 1 << 254
    x1 = int.from_bytes(u, 'little') & ((1 << 255) - 1)
    x2, z2, x3, z3, swap = 1, 0, x1, 1, 0
    for t in range(254, -1, -1):
        kt = (k >> t) & 1
        if swap ^ kt:
            x2, x3, z2, z3 = x3, x2, z3, z2
        swap = kt
        A, B = x2 + z2, x2 - z2
        AA, BB = A * A % P, B * B % P
        E = AA - BB
        DA, CB = (x3 - z3) * A % P, (x3 + z3) * B % P
        x3, z3 = (DA + CB) ** 2 % P, x1 * (DA - CB) ** 2 % P
        x2, z2 = AA * BB % P, E * (AA + 121665 * E) % P
    if swap:
        x2, z2 = x3, z3
    return (x2 * pow(z2, P - 2, P) % P).to_bytes(32, 'little')


X25519_BASE = (9).to_bytes(32, 'little')


def x25519_public(secret):
    return x25519(secret, X25519_BASE)


# ---- ChaCha20-Poly1305 (RFC 8439)

_M32 = 0xffffffff


def _chacha_block(key_words, counter, nonce_words):
    s = [0x61707865, 0x3320646e, 0x79622d32, 0x6b206574, *key_words, counter, *nonce_words]
    x = list(s)

    def qr(a, b, c, d):
        x[a] = (x[a] + x[b]) & _M32; x[d] ^= x[a]; x[d] = ((x[d] << 16) | (x[d] >> 16)) & _M32
        x[c] = (x[c] + x[d]) & _M32; x[b] ^= x[c]; x[b] = ((x[b] << 12) | (x[b] >> 20)) & _M32
        x[a] = (x[a] + x[b]) & _M32; x[d] ^= x[a]; x[d] = ((x[d] << 8) | (x[d] >> 24)) & _M32
        x[c] = (x[c] + x[d]) & _M32; x[b] ^= x[c]; x[b] = ((x[b] << 7) | (x[b] >> 25)) & _M32

    for _ in range(10):
        qr(0, 4, 8, 12); qr(1, 5, 9, 13); qr(2, 6, 10, 14); qr(3, 7, 11, 15)
        qr(0, 5, 10, 15); qr(1, 6, 11, 12); qr(2, 7, 8, 13); qr(3, 4, 9, 14)
    return b''.join(((x[i] + s[i]) & _M32).to_bytes(4, 'little') for i in range(16))


def _words(b):
    return [int.from_bytes(b[i:i + 4], 'little') for i in range(0, len(b), 4)]


def _chacha20(key, nonce, counter, data):
    kw, nw = _words(key), _words(nonce)
    out = bytearray()
    for i in range(0, len(data), 64):
        block = _chacha_block(kw, counter + i // 64, nw)
        out += bytes(a ^ b for a, b in zip(data[i:i + 64], block))
    return bytes(out)


def _poly1305(key, msg):
    r = int.from_bytes(key[:16], 'little') & 0x0ffffffc0ffffffc0ffffffc0fffffff
    s = int.from_bytes(key[16:], 'little')
    acc, p = 0, (1 << 130) - 5
    for i in range(0, len(msg), 16):
        acc = (acc + int.from_bytes(msg[i:i + 16] + b'\x01', 'little')) * r % p
    return ((acc + s) & ((1 << 128) - 1)).to_bytes(16, 'little')


def _pad16(b):
    return b'\x00' * (-len(b) % 16)


def _aead_tag(key, nonce, aad, ct):
    otk = _chacha_block(_words(key), 0, _words(nonce))[:32]
    return _poly1305(otk, aad + _pad16(aad) + ct + _pad16(ct) + len(aad).to_bytes(8, 'little') + len(ct).to_bytes(8, 'little'))


def aead_seal(key, nonce, aad, plaintext):
    if len(key) != 32 or len(nonce) != 12:
        raise ValueError('ChaCha20-Poly1305 takes a 32 byte key and a 12 byte nonce')
    ct = _chacha20(key, nonce, 1, plaintext)
    return ct + _aead_tag(key, nonce, aad, ct)


def aead_open(key, nonce, aad, sealed):
    if len(key) != 32 or len(nonce) != 12 or len(sealed) < 16:
        raise ValueError('not a ChaCha20-Poly1305 message')
    ct, tag = sealed[:-16], sealed[-16:]
    if not hmac.compare_digest(tag, _aead_tag(key, nonce, aad, ct)):
        raise ValueError('the message was changed or the key is wrong')
    return _chacha20(key, nonce, 1, ct)


# ---- HKDF-SHA256 (RFC 5869)

def hkdf_extract(salt, ikm):
    return hmac.new(salt or b'\x00' * 32, ikm, hashlib.sha256).digest()


def hkdf_expand(prk, info, length):
    out, t, i = b'', b'', 1
    while len(out) < length:
        t = hmac.new(prk, t + info + bytes([i]), hashlib.sha256).digest()
        out += t
        i += 1
    return out[:length]


# ---- HPKE (RFC 9180): base mode, DHKEM(X25519, HKDF-SHA256), HKDF-SHA256, ChaCha20-Poly1305, one message

_KEM = b'KEM' + (0x0020).to_bytes(2, 'big')
_HPKE = b'HPKE' + (0x0020).to_bytes(2, 'big') + (0x0001).to_bytes(2, 'big') + (0x0003).to_bytes(2, 'big')


def _lextract(suite, salt, label, ikm):
    return hkdf_extract(salt, b'HPKE-v1' + suite + label + ikm)


def _lexpand(suite, prk, label, info, length):
    return hkdf_expand(prk, length.to_bytes(2, 'big') + b'HPKE-v1' + suite + label + info, length)


def _hpke_key(dh, enc, recipient, info):
    if dh == b'\x00' * 32:
        raise ValueError('not a usable X25519 public key')
    shared = _lexpand(_KEM, _lextract(_KEM, b'', b'eae_prk', dh), b'shared_secret', enc + recipient, 32)
    context = b'\x00' + _lextract(_HPKE, b'', b'psk_id_hash', b'') + _lextract(_HPKE, b'', b'info_hash', info)
    secret = _lextract(_HPKE, shared, b'secret', b'')
    return _lexpand(_HPKE, secret, b'key', context, 32), _lexpand(_HPKE, secret, b'base_nonce', context, 12)


def hpke_seal(recipient, info, plaintext, aad=b'', ephemeral=None):
    """enc (32 bytes) followed by the ciphertext and its 16 byte tag: RFC 9180's single-shot Seal in base mode.
    `ephemeral` is the one-time secret key; only tests pass it."""
    sk = ephemeral or os.urandom(32)
    enc = x25519_public(sk)
    key, nonce = _hpke_key(x25519(sk, recipient), enc, recipient, info)
    return enc + aead_seal(key, nonce, aad, plaintext)


def hpke_open(secret, info, sealed, aad=b''):
    if len(sealed) < 48:
        raise ValueError('not a sealed message')
    enc = sealed[:32]
    key, nonce = _hpke_key(x25519(secret, enc), enc, x25519_public(secret), info)
    return aead_open(key, nonce, aad, sealed[32:])


# ---- "switchboard push v1": a notification only one phone can read, and can tell came from its home computer

PUSH_INFO = b'switchboard push v1'
PUSH_MAX = 2304  # sealed bytes; base64url makes 3072 characters, which fits Apple's 4096 byte payload with room


def seal_push(seed, recipient, fields, public=None, ephemeral=None):
    """Sealed bytes for the phone whose X25519 public key is `recipient`.

    plaintext = signature (64 bytes) || message
    message   = UTF-8 JSON object, at least {"v": 1, "t": <unix seconds>} plus what the notification shows
    signature = Ed25519 by the home computer's device key over  PUSH_INFO || 0x00 || recipient || message
    sealed    = HPKE base mode Seal(recipient, info = PUSH_INFO, aad = "", plaintext)   (enc || ciphertext)

    The signature is inside the seal, so nobody but the phone learns which computer sent it."""
    msg = json.dumps(fields, ensure_ascii=False, separators=(',', ':')).encode()
    sig = ed25519_sign(seed, PUSH_INFO + b'\x00' + recipient + msg, public)
    return hpke_seal(recipient, PUSH_INFO, sig + msg, ephemeral=ephemeral)


def open_push(secret, home_public, sealed):
    """What a phone does with it: the fields, or ValueError. Here for the tests and as the reference."""
    plain = hpke_open(secret, PUSH_INFO, sealed)
    sig, msg = plain[:64], plain[64:]
    if not ed25519_verify(home_public, PUSH_INFO + b'\x00' + x25519_public(secret) + msg, sig):
        raise ValueError('not signed by the home computer')
    fields = json.loads(msg.decode())
    if not isinstance(fields, dict) or fields.get('v') != 1:
        raise ValueError('not a version 1 push')
    return fields
