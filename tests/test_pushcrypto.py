"""server/pushcrypto.py against the RFCs' own test vectors, against the `cryptography` package where it is
installed, and the sealed notification format end to end."""
import hashlib, json, os, sys, unittest
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'server'))
import pushcrypto as pc  # noqa: E402

H = bytes.fromhex

# RFC 8032, section 7.1: (secret key, public key, message, signature)
ED25519 = [
    ('9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60',
     'd75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a', '',
     'e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e06522490155'
     '5fb8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b'),
    ('4ccd089b28ff96da9db6c346ec114e0f5b8a319f35aba624da8cf6ed4fb8a6fb',
     '3d4017c3e843895a92b70aa74d1b7ebc9c982ccf2ec4968cc0cd55f12af4660c', '72',
     '92a009a9f0d4cab8720e820b5f642540a2b27b5416503f8fb3762223ebdb69da'
     '085ac1e43e15996e458f3613d0f11d8c387b2eaeb4302aeeb00d291612bb0c00'),
    ('c5aa8df43f9f837bedb7442f31dcb7b166d38535076f094b85ce3a2e0b4458f7',
     'fc51cd8e6218a1a38da47ed00230f0580816ed13ba3303ac5deb911548908025', 'af82',
     '6291d657deec24024827e69c3abe01a30ce548a284743a445e3680d7db5ac3ac'
     '18ff9b538d16f290ae67f760984dc6594a7c15e9716ed28dc027beceea1ec40a'),
]


class Ed25519(unittest.TestCase):
    def test_rfc_8032_vectors(self):
        for sk, pk, msg, sig in ED25519:
            self.assertEqual(pc.ed25519_public(H(sk)).hex(), pk)
            self.assertEqual(pc.ed25519_sign(H(sk), H(msg)).hex(), sig)
            self.assertEqual(pc.ed25519_sign(H(sk), H(msg), public=H(pk)).hex(), sig)
            self.assertTrue(pc.ed25519_verify(H(pk), H(msg), H(sig)))

    def test_a_changed_message_key_or_signature_does_not_verify(self):
        sk, pk, msg, sig = (H(x) for x in ED25519[2])
        self.assertFalse(pc.ed25519_verify(pk, msg + b'x', sig))
        self.assertFalse(pc.ed25519_verify(H(ED25519[1][1]), msg, sig))
        for i in (0, 31, 32, 63):
            bad = bytearray(sig)
            bad[i] ^= 1
            self.assertFalse(pc.ed25519_verify(pk, msg, bytes(bad)), i)
        self.assertFalse(pc.ed25519_verify(pk, msg, sig[:63]))
        self.assertFalse(pc.ed25519_verify(pk[:31], msg, sig))
        # S not reduced (S + L): the same point, refused all the same (RFC 8032, 5.1.7)
        s = int.from_bytes(sig[32:], 'little') + pc.L
        self.assertFalse(pc.ed25519_verify(pk, msg, sig[:32] + s.to_bytes(32, 'little')))
        self.assertFalse(pc.ed25519_verify(b'\xff' * 32, msg, sig))  # not a point

    def test_a_seed_is_32_bytes(self):
        with self.assertRaises(ValueError):
            pc.ed25519_sign(b'short', b'm')

    def test_agrees_with_the_cryptography_package(self):
        try:
            from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
            from cryptography.hazmat.primitives import serialization as s
        except ImportError:
            self.skipTest('cryptography is not installed')
        for n in range(12):
            seed, msg = hashlib.sha256(b'seed%d' % n).digest(), hashlib.sha512(b'msg%d' % n).digest() * n
            k = Ed25519PrivateKey.from_private_bytes(seed)
            pub = k.public_key().public_bytes(s.Encoding.Raw, s.PublicFormat.Raw)
            self.assertEqual(pc.ed25519_public(seed), pub)
            self.assertEqual(pc.ed25519_sign(seed, msg), k.sign(msg))
            k.public_key().verify(pc.ed25519_sign(seed, msg), msg)


class X25519(unittest.TestCase):
    def test_rfc_7748_vectors(self):
        self.assertEqual(pc.x25519(H('a546e36bf0527c9d3b16154b82465edd62144c0ac1fc5a18506a2244ba449ac4'),
                                   H('e6db6867583030db3594c1a424b15f7c726624ec26b3353b10a903a6d0ab1c4c')).hex(),
                         'c3da55379de9c6908e94ea4df28d084f32eccf03491c71f754b4075577a28552')
        self.assertEqual(pc.x25519(H('4b66e9d4d1b4673c5ad22691957d6af5c11b6421e0ea01d42ca4169e7918ba0d'),
                                   H('e5210f12786811d3f4b7959d0538ae2c31dbe7106fc03c3efc4cd549c715a493')).hex(),
                         '95cbde9476e8907d7aade45cb4b873f88b595a68799fa152e6f8f7647aac7957')

    def test_rfc_7748_iterations(self):
        k = u = pc.X25519_BASE
        for i in range(1000):
            k, u = pc.x25519(k, u), k
            if i == 0:
                self.assertEqual(k.hex(), '422c8e7a6227d7bca1350b3e2bb7279f7897b87bb6854b783c60e80311ae3079')
        self.assertEqual(k.hex(), '684cf59ba83309552800ef566f2f4d3c1c3887c49360e3875f2eb94d99532c51')

    def test_rfc_7748_key_agreement(self):
        a = H('77076d0a7318a57d3c16c17251b26645df4c2f87ebc0992ab177fba51db92c2a')
        b = H('5dab087e624a8a4b79e17f8b83800ee66f3bb1292618b6fd1c2f8b27ff88e0eb')
        self.assertEqual(pc.x25519_public(a).hex(), '8520f0098930a754748b7ddcb43ef75a0dbf3a0d26381af4eba4a98eaa9b4e6a')
        self.assertEqual(pc.x25519_public(b).hex(), 'de9edb7d7b7dc1b4d35b61c2ece435373f8343c85b78674dadfc7e146f882b4f')
        k = '4a5d9d5ba4ce2de1728e3bf480350f25e07e21c947d19e3376f09b3c1e161742'
        self.assertEqual(pc.x25519(a, pc.x25519_public(b)).hex(), k)
        self.assertEqual(pc.x25519(b, pc.x25519_public(a)).hex(), k)


class Aead(unittest.TestCase):
    # RFC 8439, section 2.8.2
    KEY = bytes(range(0x80, 0xa0))
    NONCE = H('070000004041424344454647')
    AAD = H('50515253c0c1c2c3c4c5c6c7')
    PT = b"Ladies and Gentlemen of the class of '99: If I could offer you only one tip for the future, sunscreen would be it."
    CT = H('d31a8d34648e60db7b86afbc53ef7ec2a4aded51296e08fea9e2b5a736ee62d63dbea45e8ca9671282fafb69da92728b1a71de0a9e'
           '060b2905d6a5b67ecd3b3692ddbd7f2d778b8c9803aee328091b58fab324e4fad675945585808b4831d7bc3ff4def08e4b7a9de5'
           '76d26586cec64b6116')
    TAG = H('1ae10b594f09e26a7e902ecbd0600691')

    def test_rfc_8439_vector(self):
        self.assertEqual(pc.aead_seal(self.KEY, self.NONCE, self.AAD, self.PT), self.CT + self.TAG)
        self.assertEqual(pc.aead_open(self.KEY, self.NONCE, self.AAD, self.CT + self.TAG), self.PT)

    def test_anything_changed_is_refused(self):
        sealed = self.CT + self.TAG
        for i in (0, 50, len(sealed) - 1):
            bad = bytearray(sealed)
            bad[i] ^= 0x80
            with self.assertRaises(ValueError):
                pc.aead_open(self.KEY, self.NONCE, self.AAD, bytes(bad))
        with self.assertRaises(ValueError):
            pc.aead_open(self.KEY, self.NONCE, self.AAD + b'x', sealed)
        with self.assertRaises(ValueError):
            pc.aead_open(bytes(32), self.NONCE, self.AAD, sealed)
        with self.assertRaises(ValueError):
            pc.aead_open(self.KEY, self.NONCE, self.AAD, sealed[:10])

    def test_every_length_round_trips(self):
        for n in (0, 1, 15, 16, 17, 63, 64, 65, 300):
            pt = bytes(range(256)) * 2
            pt = pt[:n]
            self.assertEqual(pc.aead_open(self.KEY, self.NONCE, b'', pc.aead_seal(self.KEY, self.NONCE, b'', pt)), pt)


class Hkdf(unittest.TestCase):
    def test_rfc_5869_case_1(self):
        prk = pc.hkdf_extract(H('000102030405060708090a0b0c'), b'\x0b' * 22)
        self.assertEqual(prk.hex(), '077709362c2e32df0ddc3f0dc47bba6390b6c73bb50f9c3122ec844ad7c2b3e5')
        self.assertEqual(pc.hkdf_expand(prk, H('f0f1f2f3f4f5f6f7f8f9'), 42).hex(),
                         '3cb25f25faacd57a90434f64d0362f2a2d2d0a90cf1a5a4c5db02d56ecc4c5bf34007208d5b887185865')


class Hpke(unittest.TestCase):
    # RFC 9180, appendix A.2.1: DHKEM(X25519, HKDF-SHA256), HKDF-SHA256, ChaCha20Poly1305, base mode
    INFO = H('4f6465206f6e2061204772656369616e2055726e')
    SK_E = H('f4ec9b33b792c372c1d2c2063507b684ef925b8c75a42dbcbf57d63ccd381600')
    ENC = H('1afa08d3dec047a643885163f1180476fa7ddb54c6a8029ea33f95796bf2ac4a')
    SK_R = H('8057991eef8f1f1af18f4a9491d16a1ce333f695d4db8e38da75975c4478e0fb')
    PK_R = H('4310ee97d88cc1f088a5576c77ab0cf5c3ac797f3d95139c6c84b5429c59662a')
    PT = H('4265617574792069732074727574682c20747275746820626561757479')
    AAD = H('436f756e742d30')
    CT = H('1c5250d8034ec2b784ba2cfd69dbdb8af406cfe3ff938e131f0def8c8b60b4db21993c62ce81883d2dd1b51a28')

    def test_rfc_9180_vector(self):
        self.assertEqual(pc.x25519_public(self.SK_R), self.PK_R)
        key, nonce = pc._hpke_key(pc.x25519(self.SK_E, self.PK_R), self.ENC, self.PK_R, self.INFO)
        self.assertEqual(key.hex(), 'ad2744de8e17f4ebba575b3f5f5a8fa1f69c2a07f6e7500bc60ca6e3e3ec1c91')
        self.assertEqual(nonce.hex(), '5c4d98150661b848853b547f')
        sealed = pc.hpke_seal(self.PK_R, self.INFO, self.PT, aad=self.AAD, ephemeral=self.SK_E)
        self.assertEqual(sealed, self.ENC + self.CT)
        self.assertEqual(pc.hpke_open(self.SK_R, self.INFO, sealed, aad=self.AAD), self.PT)

    def test_each_seal_is_different_and_opens(self):
        a, b = pc.hpke_seal(self.PK_R, b'i', b'hello'), pc.hpke_seal(self.PK_R, b'i', b'hello')
        self.assertNotEqual(a, b)
        self.assertEqual(len(a), 32 + 5 + 16)
        self.assertEqual(pc.hpke_open(self.SK_R, b'i', a), b'hello')

    def test_the_wrong_key_info_or_bytes_do_not_open(self):
        sealed = pc.hpke_seal(self.PK_R, b'i', b'hello')
        for args in ((self.SK_E, b'i', sealed), (self.SK_R, b'other', sealed), (self.SK_R, b'i', sealed[:-1] + b'\x00'),
                     (self.SK_R, b'i', sealed[:40])):
            with self.assertRaises(ValueError):
                pc.hpke_open(*args)

    def test_a_key_that_gives_no_secret_is_refused(self):
        with self.assertRaises(ValueError):
            pc.hpke_seal(bytes(32), b'i', b'hello')


class Push(unittest.TestCase):
    SEED = hashlib.sha256(b'home').digest()
    PHONE = hashlib.sha256(b'phone').digest()

    def test_the_phone_reads_what_home_sealed(self):
        fields = {'v': 1, 't': 1700000000, 'title': 'Held: fix the tests', 'body': 'desk · 12 % of the week left', 'url': '/s/desk/S1'}
        sealed = pc.seal_push(self.SEED, pc.x25519_public(self.PHONE), fields)
        self.assertEqual(pc.open_push(self.PHONE, pc.ed25519_public(self.SEED), sealed), fields)
        for word in (b'Held', b'desk', b'/s/'):
            self.assertNotIn(word, sealed)

    def test_layout(self):
        """The bytes are what gateway/README.md says, so a phone written from that page can read them."""
        fields = {'v': 1, 't': 5, 'title': 'T'}
        recipient = pc.x25519_public(self.PHONE)
        sealed = pc.seal_push(self.SEED, recipient, fields, ephemeral=Hpke.SK_E)
        self.assertEqual(sealed[:32], Hpke.ENC)
        plain = pc.hpke_open(self.PHONE, b'switchboard push v1', sealed)
        msg = b'{"v":1,"t":5,"title":"T"}'
        self.assertEqual(plain[64:], msg)
        self.assertTrue(pc.ed25519_verify(pc.ed25519_public(self.SEED), b'switchboard push v1\x00' + recipient + msg, plain[:64]))
        self.assertEqual(len(sealed), 32 + 64 + len(msg) + 16)

    def test_another_phone_cannot_read_it(self):
        sealed = pc.seal_push(self.SEED, pc.x25519_public(self.PHONE), {'v': 1, 't': 1})
        with self.assertRaises(ValueError):
            pc.open_push(hashlib.sha256(b'other phone').digest(), pc.ed25519_public(self.SEED), sealed)

    def test_one_from_another_computer_is_refused(self):
        """Anyone who knows the phone's public key can seal to it. Only the home computer can sign."""
        sealed = pc.seal_push(hashlib.sha256(b'stranger').digest(), pc.x25519_public(self.PHONE), {'v': 1, 't': 1})
        with self.assertRaisesRegex(ValueError, 'not signed'):
            pc.open_push(self.PHONE, pc.ed25519_public(self.SEED), sealed)

    def test_one_sealed_for_another_phone_cannot_be_passed_on(self):
        """The signature names the phone it was for: a second phone that re-seals it to the first is refused."""
        other = hashlib.sha256(b'other phone').digest()
        for_other = pc.seal_push(self.SEED, pc.x25519_public(other), {'v': 1, 't': 1, 'title': 'x'})
        plain = pc.hpke_open(other, pc.PUSH_INFO, for_other)
        resealed = pc.hpke_seal(pc.x25519_public(self.PHONE), pc.PUSH_INFO, plain)
        with self.assertRaisesRegex(ValueError, 'not signed'):
            pc.open_push(self.PHONE, pc.ed25519_public(self.SEED), resealed)

    def test_go_sealed_fixture_opens(self):
        """Sealed by Go's crypto/hpke and signed by crypto/ed25519 (gateway/interop_test.go prints it)."""
        f = json.loads((Path(__file__).resolve().parent / 'fixtures' / 'push_sealed_go.json').read_text())
        fields = pc.open_push(H(f['phone_secret']), H(f['home_public']), H(f['sealed']))
        self.assertEqual(fields, {'v': 1, 't': 1700000000, 'title': 'Sealed by Go', 'body': 'café'})


if __name__ == '__main__':
    unittest.main()
