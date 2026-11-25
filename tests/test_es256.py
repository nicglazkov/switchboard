"""server/es256.py against RFC 6979's own test vectors (appendix A.2.5), against the `cryptography` package
where it is installed, and reading the key file Apple gives."""
import base64, sys, unittest
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'server'))
import es256  # noqa: E402

# RFC 6979, A.2.5: the key, its public point, and the signatures of "sample" and "test" with SHA-256.
D = 0xC9AFA9D845BA75166B5C215767B1D6934E50C3DB36E89B127B8A622B120F6721
UX = 0x60FED4BA255A9D31C961EB74C6356D68C049B8923B61FA6CE669622E60F29FB6
UY = 0x7903FE1008B8BC99A41AE9E95628BC64F2F1B20C2D7E9F5177A3C294D4462299
SIGNED = [
    (b'sample', 0xEFD48B2AACB6A8FD1140DD9CD45E81D69D2C877B56AAF991C34D0EA84EAF3716,
     0xF7CB1C942D657C41D436C7A1B6E29F65F3E900DBB9AFF4064DC4AB2F843ACDA8),
    (b'test', 0xF1ABB023518351CD71D881567B1EA663ED3EFCF6C5132B354F28D3B0B7D38367,
     0x019F4113742A2B14BD25926B49C649155F267E60D3814B4C0CC84250E46F0083),
]


def _tlv(tag, body):
    n = len(body)
    size = bytes([n]) if n < 0x80 else bytes([0x80 | len(n.to_bytes((n.bit_length() + 7) // 8, 'big'))]) + n.to_bytes((n.bit_length() + 7) // 8, 'big')
    return bytes([tag]) + size + body


def _sec1(d, curve=True):
    x, y = es256.public(d)
    pub = _tlv(0xA1, _tlv(0x03, b'\x00\x04' + x.to_bytes(32, 'big') + y.to_bytes(32, 'big')))
    params = _tlv(0xA0, _tlv(0x06, bytes.fromhex('2a8648ce3d030107'))) if curve else b''
    return _tlv(0x30, _tlv(0x02, b'\x01') + _tlv(0x04, d.to_bytes(32, 'big')) + params + pub)


def _pem(label, der):
    b = base64.b64encode(der).decode()
    return f'-----BEGIN {label}-----\n' + '\n'.join(b[i:i + 64] for i in range(0, len(b), 64)) + f'\n-----END {label}-----\n'


def pkcs8_pem(d, curve='2a8648ce3d030107'):
    """The file Apple gives for a push key (AuthKey_XXXX.p8) holding private number d."""
    alg = _tlv(0x30, _tlv(0x06, bytes.fromhex('2a8648ce3d0201')) + _tlv(0x06, bytes.fromhex(curve)))
    return _pem('PRIVATE KEY', _tlv(0x30, _tlv(0x02, b'\x00') + alg + _tlv(0x04, _sec1(d, curve=False))))


class Vectors(unittest.TestCase):
    def test_the_public_point(self):
        self.assertEqual(es256.public(D), (UX, UY))
        self.assertTrue(es256.on_curve((UX, UY)))
        self.assertFalse(es256.on_curve((UX, UY + 1)))

    def test_rfc_6979_signatures(self):
        for msg, r, s in SIGNED:
            self.assertEqual(es256.sign(D, msg), (r, s))
            self.assertTrue(es256.verify((UX, UY), msg, r, s))
            self.assertFalse(es256.verify((UX, UY), msg + b'.', r, s))
            self.assertFalse(es256.verify((UX, UY), msg, r, s ^ 1))
            self.assertEqual(len(es256.raw(r, s)), 64)

    def test_a_key_outside_the_curves_range_is_refused(self):
        for d in (0, es256.N, es256.N + 5):
            with self.assertRaises(ValueError):
                es256.sign(d, b'x')
        self.assertFalse(es256.verify((UX, UY), b'x', 0, 1))
        self.assertFalse(es256.verify((UX, UY), b'x', 1, es256.N))


class KeyFile(unittest.TestCase):
    def test_apples_file_is_read(self):
        self.assertEqual(es256.load_pem(pkcs8_pem(D)), D)
        self.assertEqual(es256.load_pem(pkcs8_pem(D).encode()), D)
        self.assertEqual(es256.load_pem('junk before\n' + pkcs8_pem(D).replace('\n', '\r\n')), D)

    def test_the_older_form_is_read(self):
        self.assertEqual(es256.load_pem(_pem('EC PRIVATE KEY', _sec1(D))), D)

    def test_what_it_cannot_use_is_refused_in_words(self):
        for text, word in (('', 'no private key'),
                           (_pem('ENCRYPTED PRIVATE KEY', b'\x30\x00'), 'password'),
                           (pkcs8_pem(D, curve='2b81040022'), 'P-256'),          # P-384's name
                           (_pem('PRIVATE KEY', b'\x30\x03\x02\x01\x00'), 'PKCS'),
                           (_pem('PRIVATE KEY', b'\x30\x82\xff\xff'), 'early'),
                           (_pem('PRIVATE KEY', b'').replace('\n\n', '\n!!!\n'), 'base64')):
            with self.assertRaises(ValueError) as e:
                es256.load_pem(text)
            self.assertIn(word, str(e.exception))


class AgainstCryptography(unittest.TestCase):
    def setUp(self):
        try:
            import cryptography  # noqa: F401
        except ImportError:
            self.skipTest('cryptography is not installed')

    def test_each_reads_the_others_keys_and_signatures(self):
        from cryptography.hazmat.primitives import hashes, serialization
        from cryptography.hazmat.primitives.asymmetric import ec
        from cryptography.hazmat.primitives.asymmetric.utils import decode_dss_signature, encode_dss_signature
        for _ in range(3):
            key = ec.generate_private_key(ec.SECP256R1())
            pem = key.private_bytes(serialization.Encoding.PEM, serialization.PrivateFormat.PKCS8, serialization.NoEncryption())
            d = es256.load_pem(pem)
            self.assertEqual(d, key.private_numbers().private_value)
            nums = key.public_key().public_numbers()
            self.assertEqual(es256.public(d), (nums.x, nums.y))
            r, s = es256.sign(d, b'a token')
            key.public_key().verify(encode_dss_signature(r, s), b'a token', ec.ECDSA(hashes.SHA256()))
            r, s = decode_dss_signature(key.sign(b'another', ec.ECDSA(hashes.SHA256())))
            self.assertTrue(es256.verify((nums.x, nums.y), b'another', r, s))
            self.assertEqual(serialization.load_pem_private_key(pkcs8_pem(d).encode(), None).private_numbers().private_value, d)


if __name__ == '__main__':
    unittest.main()
