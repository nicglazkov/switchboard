//! Sealed notifications ("switchboard push v1"): what the phone does with the `sb` field of a push.
//!
//! A notification's words are sealed on the home computer for one phone (`server/pushcrypto.py`, `seal_push`), so
//! the push gateway, Apple and Google carry bytes they cannot read. The phone has a second key just for this, an
//! X25519 one, whose public half it gives its home computer as `seal` when it registers its push token. Opening
//! is here once, for both apps:
//!
//! ```text
//! sealed    = enc (32 bytes) || ciphertext || tag (16 bytes)
//!             HPKE (RFC 9180) base mode, DHKEM(X25519, HKDF-SHA256), HKDF-SHA256, ChaCha20-Poly1305,
//!             info = "switchboard push v1", aad empty
//! plaintext = signature (64 bytes) || message
//! signature = Ed25519 by the home computer over  "switchboard push v1" || 0x00 || phone public key || message
//! message   = UTF-8 JSON: {"v": 1, "t": <unix seconds>, "title": ..., "body": ..., "url": ..., "thread": ...}
//! ```
//!
//! HPKE's base mode lets anyone who knows the phone's public key seal to it, so the seal alone says nothing
//! about who wrote the words. The signature does: it must be the home computer's, and it names the phone's own
//! key, so a notification written for another phone and sealed again for this one is refused. `gateway/README.md`
//! is the contract; the fixtures in `tests/fixtures` are what the Python and Go sides agree on.
//!
//! Nothing here keeps a key. The secret comes in as 32 bytes from the app's own storage (the iOS Keychain, an
//! Android Keystore-wrapped file) and is wiped from this crate's memory when the call returns.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chacha20poly1305::aead::{Aead, Payload};
use chacha20poly1305::{ChaCha20Poly1305, Key, KeyInit, Nonce};
use curve25519_dalek::montgomery::MontgomeryPoint;
use ed25519_dalek::{Signature, VerifyingKey};
use hkdf::Hkdf;
use sb_link::identity::DeviceId;
use serde::Deserialize;
use sha2::Sha256;
use subtle::ConstantTimeEq;
use zeroize::Zeroizing;

/// HPKE's `info`, and the first bytes of what the home computer signs.
pub const PUSH_INFO: &[u8] = b"switchboard push v1";
/// The most sealed bytes one push carries (3072 characters of base64url).
pub const PUSH_MAX: usize = 2304;
/// A notification older than this says nothing true about a chat any more: two days.
pub const STALE_AFTER: u64 = 2 * 24 * 60 * 60;

const KEY_LEN: usize = 32;
const TAG_LEN: usize = 16;
const SIG_LEN: usize = 64;
/// "KEM" and the id of DHKEM(X25519, HKDF-SHA256), 0x0020.
const KEM_SUITE: &[u8] = b"KEM\x00\x20";
/// "HPKE" and the ids of the KEM (0x0020), HKDF-SHA256 (0x0001) and ChaCha20-Poly1305 (0x0003).
const HPKE_SUITE: &[u8] = b"HPKE\x00\x20\x00\x01\x00\x03";

/// What a notification says, once opened.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct PushText {
    pub title: String,
    pub body: String,
    /// The page a tap opens (`/s/<machine>/<id>`). The app checks it as it checks any link.
    pub url: Option<String>,
    /// What groups it with others of its kind on the phone.
    pub thread: Option<String>,
    /// When the home computer wrote it, in unix seconds.
    pub sent: u64,
}

/// Why a push was not opened. The app shows the fixed sentence in every case; the kind is for its log.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Error)]
pub enum PushError {
    /// Not a sealed notification at all: the wrong length, not base64url, not version 1, or nothing to show.
    Unreadable,
    /// Sealed for another key than this phone's (or changed on the way).
    NotForThisPhone,
    /// It opened, but the home computer this phone is joined to did not write it for this phone.
    NotFromHome,
    /// Written more than two days ago.
    Stale,
}

impl std::fmt::Display for PushError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            PushError::Unreadable => "not a sealed notification",
            PushError::NotForThisPhone => "not sealed for this phone",
            PushError::NotFromHome => "not written by this phone's home computer",
            PushError::Stale => "written more than two days ago",
        })
    }
}

impl std::error::Error for PushError {}

/// A new key for this phone's notifications: 32 bytes from the system's random source. The app stores it where
/// whatever shows its notifications can read it, and gives `push_public_key` of it to the home computer.
#[uniffi::export]
pub fn new_push_key() -> Vec<u8> {
    let mut b = [0u8; KEY_LEN];
    getrandom::fill(&mut b).expect("the system random source");
    b.to_vec()
}

/// The public half of a notifications key, as the home computer takes it in `seal` (43 characters of base64url),
/// or None when the bytes are not a key.
#[uniffi::export]
pub fn push_public_key(secret: Vec<u8>) -> Option<String> {
    let secret: Zeroizing<[u8; KEY_LEN]> = Zeroizing::new(Zeroizing::new(secret).as_slice().try_into().ok()?);
    Some(URL_SAFE_NO_PAD.encode(public_of(&secret)))
}

/// Opens the `sb` field of a push. `secret`: this phone's notifications key. `home_id`: the id of the home
/// computer the phone is joined to (64 hex digits). `sealed`: the field as it arrived, base64url. `now`: the
/// phone's clock in unix seconds.
#[uniffi::export]
pub fn open_push(secret: Vec<u8>, home_id: String, sealed: String, now: u64) -> Result<PushText, PushError> {
    let secret = Zeroizing::new(secret);
    let secret: Zeroizing<[u8; KEY_LEN]> =
        Zeroizing::new(secret.as_slice().try_into().map_err(|_| PushError::NotForThisPhone)?);
    let home: DeviceId = home_id.parse().map_err(|_| PushError::NotFromHome)?;
    // (the longest field the gateway passes on, so nothing longer is even decoded)
    if sealed.len() > PUSH_MAX.div_ceil(3) * 4 {
        return Err(PushError::Unreadable);
    }
    let sealed = URL_SAFE_NO_PAD.decode(sealed.trim_end_matches('=')).map_err(|_| PushError::Unreadable)?;
    open(&secret, home.as_bytes(), &sealed, now)
}

/// The fields of the message this crate reads. Anything else in it is skipped, so the home computer can add to it.
#[derive(Deserialize)]
struct Message {
    v: u64,
    t: u64,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    thread: Option<String>,
}

/// `open_push` on bytes.
pub fn open(secret: &[u8; KEY_LEN], home: &[u8; KEY_LEN], sealed: &[u8], now: u64) -> Result<PushText, PushError> {
    if sealed.len() < KEY_LEN + SIG_LEN + TAG_LEN || sealed.len() > PUSH_MAX {
        return Err(PushError::Unreadable);
    }
    let plain = hpke_open(secret, PUSH_INFO, sealed, b"").ok_or(PushError::NotForThisPhone)?;
    let (sig, msg) = plain.split_at(SIG_LEN);
    // What was signed names this phone's key: words written for another phone do not pass, whoever sealed them.
    let mut signed = Vec::with_capacity(PUSH_INFO.len() + 1 + KEY_LEN + msg.len());
    signed.extend_from_slice(PUSH_INFO);
    signed.push(0);
    signed.extend_from_slice(&public_of(secret));
    signed.extend_from_slice(msg);
    let key = VerifyingKey::from_bytes(home).map_err(|_| PushError::NotFromHome)?;
    let sig = Signature::from_slice(sig).map_err(|_| PushError::NotFromHome)?;
    key.verify_strict(&signed, &sig).map_err(|_| PushError::NotFromHome)?;
    // Only now is anything in the message believed.
    let m: Message = serde_json::from_slice(msg).map_err(|_| PushError::Unreadable)?;
    if m.v != 1 {
        return Err(PushError::Unreadable);
    }
    if now > m.t && now - m.t > STALE_AFTER {
        return Err(PushError::Stale);
    }
    let text = |s: Option<String>| s.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    let (title, body) = (text(m.title), text(m.body));
    if title.is_none() && body.is_none() {
        return Err(PushError::Unreadable);
    }
    Ok(PushText {
        title: title.unwrap_or_default(),
        body: body.unwrap_or_default(),
        url: text(m.url),
        thread: text(m.thread),
        sent: m.t,
    })
}

/// The X25519 public key of a secret key (RFC 7748).
fn public_of(secret: &[u8; KEY_LEN]) -> [u8; KEY_LEN] {
    MontgomeryPoint::mul_base_clamped(*secret).to_bytes()
}

/// RFC 9180's LabeledExtract: the pseudorandom key, and the same thing ready to expand from.
fn labeled_extract(suite: &[u8], salt: &[u8], label: &[u8], ikm: &[u8]) -> (Zeroizing<[u8; 32]>, Hkdf<Sha256>) {
    let mut m = Zeroizing::new(Vec::with_capacity(7 + suite.len() + label.len() + ikm.len()));
    m.extend_from_slice(b"HPKE-v1");
    m.extend_from_slice(suite);
    m.extend_from_slice(label);
    m.extend_from_slice(ikm);
    let (prk, hk) = Hkdf::<Sha256>::extract(Some(salt), &m);
    (Zeroizing::new(prk.into()), hk)
}

/// RFC 9180's LabeledExpand, for the short outputs used here.
fn labeled_expand<const N: usize>(
    suite: &[u8],
    prk: &Hkdf<Sha256>,
    label: &[u8],
    info: &[&[u8]],
) -> Zeroizing<[u8; N]> {
    let len = (N as u16).to_be_bytes();
    let mut parts: Vec<&[u8]> = vec![&len, b"HPKE-v1", suite, label];
    parts.extend_from_slice(info);
    let mut out = Zeroizing::new([0u8; N]);
    prk.expand_multi_info(&parts, &mut *out).expect("a few bytes, far below what HKDF can give");
    out
}

/// What opens (or seals) the one message of a context.
struct OneMessage {
    key: Zeroizing<[u8; 32]>,
    nonce: Zeroizing<[u8; 12]>,
}

/// The AEAD key and nonce for the one message of a base-mode context (RFC 9180, 4.1 Decap or Encap's secret,
/// then 5.1 KeySchedule with no PSK). None when the shared point is all zero, which no honest key gives.
fn hpke_key(dh: &[u8; KEY_LEN], enc: &[u8; KEY_LEN], recipient: &[u8; KEY_LEN], info: &[u8]) -> Option<OneMessage> {
    if bool::from(dh.ct_eq(&[0u8; KEY_LEN])) {
        return None;
    }
    let (_, eae) = labeled_extract(KEM_SUITE, b"", b"eae_prk", dh);
    let shared: Zeroizing<[u8; 32]> = labeled_expand(KEM_SUITE, &eae, b"shared_secret", &[enc, recipient]);
    let (psk_id_hash, _) = labeled_extract(HPKE_SUITE, b"", b"psk_id_hash", b"");
    let (info_hash, _) = labeled_extract(HPKE_SUITE, b"", b"info_hash", info);
    let (_, secret) = labeled_extract(HPKE_SUITE, &*shared, b"secret", b"");
    // (mode 0: base)
    let context: [&[u8]; 3] = [&[0u8], &*psk_id_hash, &*info_hash];
    Some(OneMessage {
        key: labeled_expand(HPKE_SUITE, &secret, b"key", &context),
        nonce: labeled_expand(HPKE_SUITE, &secret, b"base_nonce", &context),
    })
}

/// RFC 9180's single-shot Open in base mode: `sealed` is enc, then the ciphertext and its tag.
fn hpke_open(secret: &[u8; KEY_LEN], info: &[u8], sealed: &[u8], aad: &[u8]) -> Option<Zeroizing<Vec<u8>>> {
    if sealed.len() < KEY_LEN + TAG_LEN {
        return None;
    }
    let enc: [u8; KEY_LEN] = sealed[..KEY_LEN].try_into().ok()?;
    let dh = Zeroizing::new(MontgomeryPoint(enc).mul_clamped(*secret).to_bytes());
    let OneMessage { key, nonce } = hpke_key(&dh, &enc, &public_of(secret), info)?;
    // (the first message of a context: its nonce is the base nonce itself)
    ChaCha20Poly1305::new(Key::from_slice(&*key))
        .decrypt(Nonce::from_slice(&*nonce), Payload { msg: &sealed[KEY_LEN..], aad })
        .ok()
        .map(Zeroizing::new)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};

    fn h(s: &str) -> Vec<u8> {
        assert!(s.len().is_multiple_of(2));
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
    }

    fn h32(s: &str) -> [u8; 32] {
        h(s).try_into().unwrap()
    }

    /// The other side, for tests: RFC 9180's single-shot Seal with the one-time key given.
    fn hpke_seal(recipient: &[u8; 32], info: &[u8], plain: &[u8], aad: &[u8], ephemeral: &[u8; 32]) -> Vec<u8> {
        let enc = public_of(ephemeral);
        let dh = MontgomeryPoint(*recipient).mul_clamped(*ephemeral).to_bytes();
        let OneMessage { key, nonce } = hpke_key(&dh, &enc, recipient, info).expect("a usable key");
        let ct = ChaCha20Poly1305::new(Key::from_slice(&*key))
            .encrypt(Nonce::from_slice(&*nonce), Payload { msg: plain, aad })
            .unwrap();
        [enc.to_vec(), ct].concat()
    }

    /// What `server/pushcrypto.py`'s `seal_push` does, with `signed_for` as the phone key the signature names.
    fn seal_as(
        home: &SigningKey,
        recipient: &[u8; 32],
        signed_for: &[u8; 32],
        message: &str,
        ephemeral: u8,
    ) -> Vec<u8> {
        let signed = [PUSH_INFO, &[0], signed_for, message.as_bytes()].concat();
        let plain = [home.sign(&signed).to_bytes().to_vec(), message.as_bytes().to_vec()].concat();
        hpke_seal(recipient, PUSH_INFO, &plain, b"", &[ephemeral; 32])
    }

    fn seal(home: &SigningKey, recipient: &[u8; 32], message: &str) -> Vec<u8> {
        seal_as(home, recipient, recipient, message, 7)
    }

    const NOW: u64 = 1_700_000_000;

    fn message(t: u64) -> String {
        format!(
            r#"{{"v":1,"t":{t},"title":"Held: fix the tests","body":"studio","url":"/s/studio/S1","thread":"held"}}"#
        )
    }

    struct Pair {
        home: SigningKey,
        home_id: [u8; 32],
        secret: [u8; 32],
        public: [u8; 32],
    }

    fn pair() -> Pair {
        let home = SigningKey::from_bytes(&[3u8; 32]);
        let secret = [5u8; 32];
        Pair { home_id: home.verifying_key().to_bytes(), home, secret, public: public_of(&secret) }
    }

    fn fixture(name: &str) -> serde_json::Value {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../tests/fixtures").join(name);
        serde_json::from_slice(&std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))).unwrap()
    }

    #[test]
    fn the_rfc_9180_vector_opens_and_seals_the_same() {
        // Appendix A.2.1: DHKEM(X25519, HKDF-SHA256), HKDF-SHA256, ChaCha20Poly1305, base mode, sequence 0.
        let info = h("4f6465206f6e2061204772656369616e2055726e");
        let sk_e = h32("f4ec9b33b792c372c1d2c2063507b684ef925b8c75a42dbcbf57d63ccd381600");
        let enc = h32("1afa08d3dec047a643885163f1180476fa7ddb54c6a8029ea33f95796bf2ac4a");
        let sk_r = h32("8057991eef8f1f1af18f4a9491d16a1ce333f695d4db8e38da75975c4478e0fb");
        let pk_r = h32("4310ee97d88cc1f088a5576c77ab0cf5c3ac797f3d95139c6c84b5429c59662a");
        let pt = h("4265617574792069732074727574682c20747275746820626561757479");
        let aad = h("436f756e742d30");
        let ct = h("1c5250d8034ec2b784ba2cfd69dbdb8af406cfe3ff938e131f0def8c8b60b4db21993c62ce81883d2dd1b51a28");

        assert_eq!(public_of(&sk_r), pk_r);
        assert_eq!(public_of(&sk_e), enc);
        let dh = MontgomeryPoint(pk_r).mul_clamped(sk_e).to_bytes();
        let OneMessage { key, nonce } = hpke_key(&dh, &enc, &pk_r, &info).unwrap();
        assert_eq!(key.to_vec(), h("ad2744de8e17f4ebba575b3f5f5a8fa1f69c2a07f6e7500bc60ca6e3e3ec1c91"));
        assert_eq!(nonce.to_vec(), h("5c4d98150661b848853b547f"));
        let sealed = [enc.to_vec(), ct].concat();
        assert_eq!(hpke_seal(&pk_r, &info, &pt, &aad, &sk_e), sealed);
        assert_eq!(hpke_open(&sk_r, &info, &sealed, &aad).unwrap().to_vec(), pt);
    }

    #[test]
    fn the_wrong_key_info_or_bytes_do_not_open() {
        let p = pair();
        let sealed = hpke_seal(&p.public, b"i", b"hello", b"", &[9; 32]);
        assert_eq!(hpke_open(&p.secret, b"i", &sealed, b"").unwrap().to_vec(), b"hello");
        assert!(hpke_open(&[6; 32], b"i", &sealed, b"").is_none(), "another key");
        assert!(hpke_open(&p.secret, b"other", &sealed, b"").is_none(), "another info");
        assert!(hpke_open(&p.secret, b"i", &sealed, b"aad").is_none(), "another aad");
        let mut changed = sealed.clone();
        *changed.last_mut().unwrap() ^= 1;
        assert!(hpke_open(&p.secret, b"i", &changed, b"").is_none(), "a changed tag");
        assert!(hpke_open(&p.secret, b"i", &sealed[..40], b"").is_none(), "cut short");
        // enc = 0 gives the all-zero shared point, which is no secret at all
        let zero = [vec![0u8; 32], sealed[32..].to_vec()].concat();
        assert!(hpke_open(&p.secret, b"i", &zero, b"").is_none());
    }

    #[test]
    fn what_python_sealed_opens() {
        let f = fixture("push_sealed_py.json");
        let s = |k: &str| f[k].as_str().unwrap().to_string();
        let secret = h32(&s("phone_secret"));
        assert_eq!(public_of(&secret).to_vec(), h(&s("phone_public")));
        let got = open(&secret, &h32(&s("home_public")), &h(&s("sealed")), NOW + 60).unwrap();
        assert_eq!(
            got,
            PushText {
                title: "Held: fix the tests".into(),
                body: "café · desk".into(),
                url: Some("/s/desk/S1".into()),
                thread: Some("held".into()),
                sent: NOW,
            }
        );
        // and through the call the apps make: bytes, hex and base64url as they have them
        let sealed = URL_SAFE_NO_PAD.encode(h(&s("sealed")));
        assert_eq!(open_push(secret.to_vec(), s("home_public"), sealed.clone(), NOW + 60), Ok(got));
        assert_eq!(push_public_key(secret.to_vec()), Some(URL_SAFE_NO_PAD.encode(h(&s("phone_public")))));
        assert_eq!(open_push(secret.to_vec(), s("home_public"), sealed, NOW + STALE_AFTER + 1), Err(PushError::Stale));
    }

    #[test]
    fn what_go_sealed_opens() {
        let f = fixture("push_sealed_go.json");
        let s = |k: &str| f[k].as_str().unwrap().to_string();
        let got = open(&h32(&s("phone_secret")), &h32(&s("home_public")), &h(&s("sealed")), 0).unwrap();
        assert!(!got.title.is_empty() || !got.body.is_empty());
        // another home computer's key: the same bytes are nobody's
        assert_eq!(open(&h32(&s("phone_secret")), &pair().home_id, &h(&s("sealed")), 0), Err(PushError::NotFromHome));
    }

    #[test]
    fn a_push_from_home_opens() {
        let p = pair();
        let sealed = seal(&p.home, &p.public, &message(NOW));
        let got = open(&p.secret, &p.home_id, &sealed, NOW).unwrap();
        assert_eq!((got.title.as_str(), got.body.as_str()), ("Held: fix the tests", "studio"));
        assert_eq!(got.url.as_deref(), Some("/s/studio/S1"));
        assert_eq!(got.sent, NOW);
    }

    #[test]
    fn another_phones_push_is_refused() {
        let p = pair();
        let other = [8u8; 32];
        // sealed for the other phone: this one cannot even open it
        let theirs = seal(&p.home, &public_of(&other), &message(NOW));
        assert_eq!(open(&p.secret, &p.home_id, &theirs, NOW), Err(PushError::NotForThisPhone));
        // written (signed) for the other phone, then sealed again for this one by someone who knows its public
        // key: it opens, and the signature names the wrong phone
        let passed_on = seal_as(&p.home, &p.public, &public_of(&other), &message(NOW), 7);
        assert_eq!(open(&p.secret, &p.home_id, &passed_on, NOW), Err(PushError::NotFromHome));
    }

    #[test]
    fn a_push_nobody_at_home_signed_is_refused() {
        let p = pair();
        let stranger = SigningKey::from_bytes(&[4u8; 32]);
        let sealed = seal(&stranger, &p.public, &message(NOW));
        assert_eq!(open(&p.secret, &p.home_id, &sealed, NOW), Err(PushError::NotFromHome));
        // a signature of the right key over other words
        let signed = [PUSH_INFO, &[0], &p.public[..], message(NOW).as_bytes()].concat();
        let plain = [p.home.sign(&signed).to_bytes().to_vec(), message(NOW + 1).into_bytes()].concat();
        let swapped = hpke_seal(&p.public, PUSH_INFO, &plain, b"", &[7; 32]);
        assert_eq!(open(&p.secret, &p.home_id, &swapped, NOW), Err(PushError::NotFromHome));
        // a home id that is no key at all
        assert_eq!(open(&p.secret, &[0xff; 32], &sealed, NOW), Err(PushError::NotFromHome));
    }

    #[test]
    fn an_old_push_is_stale_and_a_clock_behind_is_not() {
        let p = pair();
        let at = |t: u64, now: u64| open(&p.secret, &p.home_id, &seal(&p.home, &p.public, &message(t)), now);
        assert!(at(NOW, NOW + STALE_AFTER).is_ok(), "two days to the second");
        assert_eq!(at(NOW, NOW + STALE_AFTER + 1), Err(PushError::Stale));
        assert!(at(NOW + 3600, NOW).is_ok(), "the phone's clock is behind the computer's");
    }

    #[test]
    fn only_version_1_with_something_to_show_is_read() {
        let p = pair();
        let with = |m: &str| open(&p.secret, &p.home_id, &seal(&p.home, &p.public, m), NOW);
        assert_eq!(with(r#"{"v":2,"t":1700000000,"title":"a","body":"b"}"#), Err(PushError::Unreadable));
        assert_eq!(with(r#"{"t":1700000000,"title":"a","body":"b"}"#), Err(PushError::Unreadable));
        assert_eq!(with(r#"{"v":1,"title":"a","body":"b"}"#), Err(PushError::Unreadable), "no time");
        assert_eq!(with(r#"{"v":1,"t":1700000000,"title":" ","body":""}"#), Err(PushError::Unreadable));
        assert_eq!(with(r#"["v",1]"#), Err(PushError::Unreadable));
        assert_eq!(with("not json"), Err(PushError::Unreadable));
        // a field this version does not know is skipped, and a body alone is enough
        let got = with(r#"{"v":1,"t":1700000000,"body":"b","later":{"x":1}}"#).unwrap();
        assert_eq!((got.title.as_str(), got.body.as_str(), got.url, got.thread), ("", "b", None, None));
    }

    #[test]
    fn what_cannot_be_a_sealed_push_is_unreadable() {
        let p = pair();
        assert_eq!(open(&p.secret, &p.home_id, &[], NOW), Err(PushError::Unreadable));
        assert_eq!(
            open(&p.secret, &p.home_id, &[1; 111], NOW),
            Err(PushError::Unreadable),
            "too short to hold a signature"
        );
        assert_eq!(open(&p.secret, &p.home_id, &[1; PUSH_MAX + 1], NOW), Err(PushError::Unreadable));
        let id = "07".repeat(32);
        assert_eq!(open_push(p.secret.to_vec(), id.clone(), "not base64 !".into(), NOW), Err(PushError::Unreadable));
        assert_eq!(open_push(p.secret.to_vec(), id.clone(), "A".repeat(4097), NOW), Err(PushError::Unreadable));
        assert_eq!(open_push(vec![1; 31], id, "AAAA".into(), NOW), Err(PushError::NotForThisPhone));
        assert_eq!(open_push(p.secret.to_vec(), "home".into(), "AAAA".into(), NOW), Err(PushError::NotFromHome));
    }

    #[test]
    fn a_push_key_is_32_bytes_new_each_time_with_a_43_character_public_half() {
        let (a, b) = (new_push_key(), new_push_key());
        assert_eq!(a.len(), 32);
        assert_ne!(a, b);
        let public = push_public_key(a.clone()).unwrap();
        assert_eq!(public.len(), 43);
        assert!(public.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-'));
        assert_eq!(push_public_key(a), Some(public));
        assert_eq!(push_public_key(vec![1; 31]), None);
    }

    #[test]
    fn a_new_key_opens_what_is_sealed_to_its_public_half() {
        let p = pair();
        let secret: [u8; 32] = new_push_key().try_into().unwrap();
        let public: [u8; 32] =
            URL_SAFE_NO_PAD.decode(push_public_key(secret.to_vec()).unwrap()).unwrap().try_into().unwrap();
        let sealed = URL_SAFE_NO_PAD.encode(seal(&p.home, &public, &message(NOW)));
        let home_id: String = p.home_id.iter().map(|b| format!("{b:02x}")).collect();
        assert!(open_push(secret.to_vec(), home_id, sealed, NOW).is_ok());
    }
}
