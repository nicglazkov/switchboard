//! What a person carries from home to the device that joins: a short code or a link.
//!
//! - **Short code.** 8 letters from the 31 the hub's pairing codes use (no 0, O, 1, I or L), shown `ABCD-EFGH`:
//!   about 40 bits. Typed input is forgiving about case, spaces and dashes and strict about everything else.
//!   Home listens for the joiner at an id made from the code (`rendezvous_key`), so the code alone finds it.
//! - **Link or QR.** `switchboard://pair/1?h=<home id>&r=<relay url>&s=<128-bit secret>&n=<name>`, with the id
//!   and the secret in hex. The joiner dials the home id directly. `r` and `n` may be left out.
//!
//! Both are secrets until used: neither is printed by `Debug`.

use crate::identity::{DeviceId, KEY_LEN, public_of};
use std::fmt;

/// The hub's pairing alphabet (sb-hub nodes.rs).
pub const ALPHABET: &[u8; 31] = b"ABCDEFGHJKMNPQRSTUVWXYZ23456789";
pub const CODE_LEN: usize = 8;
/// The fixed salt of the slow hash (Argon2id) that turns a code into the rendezvous key. Changing it, or the
/// costs below, changes the wire.
pub const RENDEZVOUS_SALT: &str = "switchboard pair rendezvous 2";
/// Argon2id memory, in KiB: 64 MiB.
pub const RENDEZVOUS_MEM_KIB: u32 = 64 * 1024;
/// Argon2id passes.
pub const RENDEZVOUS_PASSES: u32 = 3;
pub const SECRET_LEN: usize = 16;
pub const LINK_PREFIX: &str = "switchboard://pair/1?";
/// Longest link read. A link with the longest name and relay allowed is shorter.
pub const LINK_MAX: usize = 1024;
pub const NAME_MAX: usize = 64;
pub const RELAY_MAX: usize = 200;
/// Longest typed text looked at as a code.
const TYPED_MAX: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodeError {
    /// How many letters and digits were typed, when it is not 8.
    Length(usize),
    /// A character no code has (0, O, 1, I, L, or anything that is not a letter or digit).
    Letter(char),
}

impl fmt::Display for CodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CodeError::Length(n) => write!(f, "A code is {CODE_LEN} letters and digits, and that was {n}."),
            CodeError::Letter(c) => {
                write!(f, "A code never has {c:?} in it. Codes leave out 0, O, 1, I and L.")
            }
        }
    }
}

impl std::error::Error for CodeError {}

/// A short pairing code, as its 8 letters with no dash.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct ShortCode(String);

impl ShortCode {
    /// A new code from the system's random source, every letter equally likely.
    pub fn generate() -> ShortCode {
        let mut out = String::with_capacity(CODE_LEN);
        while out.len() < CODE_LEN {
            for b in crate::random::<16>() {
                // 248 = 31 * 8: below it every letter is equally likely
                if b < 248 && out.len() < CODE_LEN {
                    out.push(char::from(ALPHABET[usize::from(b % 31)]));
                }
            }
        }
        ShortCode(out)
    }

    /// A code as someone typed it: case does not matter, and spaces and dashes of any kind are skipped.
    pub fn parse(typed: &str) -> Result<ShortCode, CodeError> {
        let mut out = String::with_capacity(CODE_LEN);
        let mut count = 0usize;
        for c in typed.chars().take(TYPED_MAX) {
            if c.is_whitespace() || matches!(c, '-' | '\u{2010}'..='\u{2015}' | '\u{2212}' | '_' | '.') {
                continue;
            }
            let up = c.to_ascii_uppercase();
            if !up.is_ascii() || !ALPHABET.contains(&(up as u8)) {
                return Err(CodeError::Letter(c));
            }
            count += 1;
            if out.len() < CODE_LEN {
                out.push(up);
            }
        }
        if count != CODE_LEN || typed.chars().nth(TYPED_MAX).is_some() {
            return Err(CodeError::Length(count));
        }
        Ok(ShortCode(out))
    }

    /// The 8 letters, no dash: what goes into the key derivation and the exchange.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The secret key of the temporary endpoint home listens on while this code is live. The joiner derives the
    /// same key, so it knows the endpoint's id (`rendezvous_id`) without being told an address.
    ///
    /// Slow on purpose (Argon2id, 64 MiB, 3 passes, one lane: a fraction of a second, so call it off the async
    /// threads). The id is public, since the relay sees it, and a code has about 40 bits: with a plain hash,
    /// someone who saw the id could try every code before the code's ten minutes are up.
    pub fn rendezvous_key(&self) -> [u8; KEY_LEN] {
        let params = argon2::Params::new(RENDEZVOUS_MEM_KIB, RENDEZVOUS_PASSES, 1, Some(KEY_LEN))
            .expect("fixed Argon2 costs are valid");
        let mut out = [0u8; KEY_LEN];
        argon2::Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, params)
            .hash_password_into(self.0.as_bytes(), RENDEZVOUS_SALT.as_bytes(), &mut out)
            .expect("Argon2 with a 32-byte output and a fixed salt cannot fail");
        out
    }

    /// The id the joiner dials: the public key of `rendezvous_key`.
    pub fn rendezvous_id(&self) -> DeviceId {
        DeviceId(public_of(&self.rendezvous_key()))
    }
}

/// `ABCD-EFGH`, the way a code is shown.
impl fmt::Display for ShortCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}-{}", &self.0[..CODE_LEN / 2], &self.0[CODE_LEN / 2..])
    }
}

impl fmt::Debug for ShortCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ShortCode(..)")
    }
}

/// Whether `url` can be a relay's address: http or https, a host, an optional port and path, at most 200
/// characters, and no user name, query, fragment, space or control character.
pub fn valid_relay(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("https://").or_else(|| url.strip_prefix("http://")) else { return false };
    let host = rest.split('/').next().unwrap_or("");
    url.len() <= RELAY_MAX
        && !host.is_empty()
        && host.bytes().all(|b| b.is_ascii_alphanumeric() || b"-.:[]".contains(&b))
        && rest.bytes().all(|b| b.is_ascii_alphanumeric() || b"-._~:/[]%".contains(&b))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkError {
    TooLong,
    /// Not `switchboard://pair/1?...`. A later version of the link is refused here too.
    NotALink,
    /// A part that is not `key=value`, or a `%` that is not followed by two hex digits or does not make text.
    Malformed,
    /// A key this version does not know.
    Unknown(String),
    Repeated(char),
    Missing(char),
    /// `h` is not 64 hex digits that are a public key.
    BadHome,
    /// `s` is not 32 hex digits.
    BadSecret,
    BadRelay,
    BadName,
}

impl fmt::Display for LinkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LinkError::TooLong => f.write_str("That link is too long to be a pairing link."),
            LinkError::NotALink => f.write_str("That is not a Switchboard pairing link this version can read."),
            LinkError::Malformed => f.write_str("That pairing link is damaged. Copy it again, or scan the code."),
            LinkError::Unknown(k) => write!(f, "That pairing link has a part this version does not know ({k})."),
            LinkError::Repeated(k) => write!(f, "That pairing link gives {k} twice."),
            LinkError::Missing(k) => write!(f, "That pairing link is missing its {k} part. Copy all of it."),
            LinkError::BadHome => f.write_str("That pairing link does not name a computer. Copy it again."),
            LinkError::BadSecret => f.write_str("That pairing link's secret is damaged. Copy it again."),
            LinkError::BadRelay => f.write_str("That pairing link names a relay that is not an http or https address."),
            LinkError::BadName => f.write_str("That pairing link's name is not usable."),
        }
    }
}

impl std::error::Error for LinkError {}

/// What a pairing link or QR code holds.
#[derive(Clone, PartialEq, Eq)]
pub struct PairLink {
    /// Home's id: the joiner dials it, and refuses to pair with anything else.
    pub home: DeviceId,
    /// The relay home is reached through, when it has one.
    pub relay: Option<String>,
    /// 128 random bits, good for one pairing.
    pub secret: [u8; SECRET_LEN],
    /// Home's name, to show before pairing. Not trusted for anything.
    pub name: String,
}

impl PairLink {
    /// Reads a link. Strict: the exact prefix, every key at most once, no key this version does not know, the id
    /// a real public key, and nothing over its length.
    pub fn parse(text: &str) -> Result<PairLink, LinkError> {
        let text = text.trim();
        if text.len() > LINK_MAX {
            return Err(LinkError::TooLong);
        }
        // (the scheme and host of a link may arrive in any case; the rest may not)
        let query = match text.get(..LINK_PREFIX.len()) {
            Some(p) if p.eq_ignore_ascii_case(LINK_PREFIX) => &text[LINK_PREFIX.len()..],
            _ => return Err(LinkError::NotALink),
        };
        let (mut home, mut relay, mut secret, mut name) = (None, None, None, None);
        for part in query.split('&') {
            let (key, value) = part.split_once('=').ok_or(LinkError::Malformed)?;
            let value = unescape(value).ok_or(LinkError::Malformed)?;
            let slot = match key {
                "h" => &mut home,
                "r" => &mut relay,
                "s" => &mut secret,
                "n" => &mut name,
                _ => return Err(LinkError::Unknown(crate::clean(key, 16))),
            };
            if slot.replace(value).is_some() {
                return Err(LinkError::Repeated(key.chars().next().unwrap_or('?')));
            }
        }
        let home = home.ok_or(LinkError::Missing('h'))?;
        let secret = secret.ok_or(LinkError::Missing('s'))?;
        let home = home.parse::<DeviceId>().ok().filter(DeviceId::is_key).ok_or(LinkError::BadHome)?;
        let secret = crate::hex::decode::<SECRET_LEN>(&secret).ok_or(LinkError::BadSecret)?;
        let relay = match relay {
            Some(r) if valid_relay(&r) => Some(r),
            Some(_) => return Err(LinkError::BadRelay),
            None => None,
        };
        let name = name.unwrap_or_default();
        if name.chars().count() > NAME_MAX || name.chars().any(char::is_control) || name.trim() != name {
            return Err(LinkError::BadName);
        }
        Ok(PairLink { home, relay, secret, name })
    }

    /// A link for a new pairing with home: a fresh secret.
    pub fn new(home: DeviceId, relay: Option<String>, name: &str) -> PairLink {
        PairLink { home, relay, secret: crate::random(), name: crate::clean(name, NAME_MAX) }
    }
}

/// The link as text, for a QR code or the clipboard. `PairLink::parse` reads it back.
impl fmt::Display for PairLink {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{LINK_PREFIX}h={}", self.home)?;
        if let Some(r) = &self.relay {
            write!(f, "&r={}", escape(r))?;
        }
        write!(f, "&s={}", crate::hex::encode(&self.secret))?;
        if !self.name.is_empty() {
            write!(f, "&n={}", escape(&self.name))?;
        }
        Ok(())
    }
}

impl fmt::Debug for PairLink {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PairLink(home {}, relay {:?}, name {:?}, secret ..)", self.home.short(), self.relay, self.name)
    }
}

/// Percent-encoding for a query value: letters, digits and `-._~` stay.
fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
            out.push(char::from(b));
        } else {
            out.push('%');
            out.push_str(&crate::hex::encode(&[b]).to_ascii_uppercase());
        }
    }
    out
}

/// Undoes `escape`. None for a `%` without two hex digits, a raw character a query value cannot hold, or bytes
/// that are not UTF-8.
fn unescape(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'%' => {
                let pair = s.get(i + 1..i + 3)?;
                out.push(crate::hex::decode::<1>(pair)?[0]);
                i += 3;
            }
            c if c.is_ascii_graphic() && !b"#&=+\"<>\\^`{|}".contains(&c) => {
                out.push(c);
                i += 1;
            }
            _ => return None,
        }
    }
    String::from_utf8(out).ok()
}

/// What someone pasted or typed on the join screen: a link, or else a code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PairInput {
    Code(ShortCode),
    Link(PairLink),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputError {
    Code(CodeError),
    Link(LinkError),
}

impl fmt::Display for InputError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            InputError::Code(e) => e.fmt(f),
            InputError::Link(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for InputError {}

impl PairInput {
    /// Anything with `:` in it is read as a link, since no code has one.
    pub fn parse(text: &str) -> Result<PairInput, InputError> {
        if text.contains(':') {
            PairLink::parse(text).map(PairInput::Link).map_err(InputError::Link)
        } else {
            ShortCode::parse(text).map(PairInput::Code).map_err(InputError::Code)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::DeviceKey;
    use crate::testing::Dice;

    #[test]
    fn the_alphabet_is_the_hubs() {
        assert_eq!(ALPHABET.len(), 31);
        for c in b"0O1IL" {
            assert!(!ALPHABET.contains(c));
        }
        let mut sorted = ALPHABET.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), 31);
    }

    #[test]
    fn a_new_code_is_eight_letters_shown_in_two_halves() {
        let mut seen = std::collections::HashSet::new();
        for _ in 0..200 {
            let c = ShortCode::generate();
            assert_eq!(c.as_str().len(), CODE_LEN);
            assert!(c.as_str().bytes().all(|b| ALPHABET.contains(&b)), "{}", c.as_str());
            let shown = c.to_string();
            assert_eq!(shown.len(), 9);
            assert_eq!(&shown[4..5], "-");
            assert_eq!(ShortCode::parse(&shown).unwrap(), c);
            seen.insert(c);
        }
        assert_eq!(seen.len(), 200);
        assert_eq!(format!("{:?}", ShortCode::generate()), "ShortCode(..)", "a code is never printed by Debug");
    }

    #[test]
    fn typed_codes_are_normalised() {
        let want = ShortCode::parse("ABCDEFGH").unwrap();
        assert_eq!(want.as_str(), "ABCDEFGH");
        assert_eq!(want.to_string(), "ABCD-EFGH");
        for typed in [
            "abcd-efgh",
            " ABCD EFGH ",
            "a b c d e f g h",
            "AbCd\u{2013}EfGh",
            "abcd\u{2014}efgh\n",
            "ABCD_EFGH",
            "\tabcdefgh",
        ] {
            assert_eq!(ShortCode::parse(typed).as_ref(), Ok(&want), "{typed:?}");
        }
        assert_eq!(ShortCode::parse("xy23-4567").unwrap().as_str(), "XY234567");
    }

    #[test]
    fn typed_codes_that_cannot_be_codes_are_refused() {
        assert_eq!(ShortCode::parse(""), Err(CodeError::Length(0)));
        assert_eq!(ShortCode::parse("ABCD-EFG"), Err(CodeError::Length(7)));
        assert_eq!(ShortCode::parse("ABCD-EFGHJ"), Err(CodeError::Length(9)));
        assert_eq!(ShortCode::parse("ABCD-EFG0"), Err(CodeError::Letter('0')));
        assert_eq!(ShortCode::parse("ABCD-EFGo"), Err(CodeError::Letter('o')));
        assert_eq!(ShortCode::parse("1BCD-EFGH"), Err(CodeError::Letter('1')));
        assert_eq!(ShortCode::parse("ABCD-EFGl"), Err(CodeError::Letter('l')));
        assert_eq!(ShortCode::parse("ABCD-EFG!"), Err(CodeError::Letter('!')));
        assert_eq!(ShortCode::parse("ABCD-EFGÉ"), Err(CodeError::Letter('É')));
        // a letter whose uppercase is not ASCII must not slip through as one that is
        assert_eq!(ShortCode::parse("ABCD-EFGſ"), Err(CodeError::Letter('ſ')));
        assert!(matches!(ShortCode::parse(&"A".repeat(5000)), Err(CodeError::Length(_))));
        assert!(matches!(ShortCode::parse(&format!("ABCDEFGH{}", " ".repeat(100))), Err(CodeError::Length(_))));
        assert!(CodeError::Letter('0').to_string().contains("0, O, 1, I and L"));
    }

    #[test]
    fn the_rendezvous_key_is_fixed_by_the_code() {
        // Fixed vectors: a joiner and a home on different versions must agree on these forever.
        let code = ShortCode::parse("ABCD-EFGH").unwrap();
        assert_eq!(RENDEZVOUS_SALT, "switchboard pair rendezvous 2");
        assert_eq!((RENDEZVOUS_MEM_KIB, RENDEZVOUS_PASSES), (65536, 3));
        assert_eq!(crate::hex::encode(&code.rendezvous_key()), VECTOR_KEY);
        assert_eq!(code.rendezvous_id().to_string(), VECTOR_ID);
        assert_eq!(ShortCode::parse("abcd efgh").unwrap().rendezvous_key(), code.rendezvous_key());
        let other = ShortCode::parse("ABCD-EFGJ").unwrap();
        assert_ne!(other.rendezvous_key(), code.rendezvous_key());
        assert!(code.rendezvous_id().is_key());
    }

    // Worked out apart from this code, with Python's argon2-cffi (hash_secret_raw, Type.ID, version 19,
    // time_cost 3, memory_cost 65536, parallelism 1) and cryptography (Ed25519).
    const VECTOR_KEY: &str = "66d9febf38c70b4da0be0f329e23a74631c9681941ddfaa24edeeeeb7ab938ee";
    const VECTOR_ID: &str = "07ef0adb411452d0243de7f4b9c7c14cfaf89b4538f63aed4420538d1f1ee9f9";

    fn home() -> DeviceId {
        DeviceKey::from_bytes([7u8; 32]).id()
    }

    #[test]
    fn a_link_prints_and_reads_back() {
        let link = PairLink {
            home: home(),
            relay: Some("https://relay.example.com:8443/".into()),
            secret: [0xab; 16],
            name: "Sam's Mac & café".into(),
        };
        let text = link.to_string();
        assert_eq!(
            text,
            format!(
                "switchboard://pair/1?h={}&r=https%3A%2F%2Frelay.example.com%3A8443%2F&s={}&n=Sam%27s%20Mac%20%26%20caf%C3%A9",
                home(),
                "ab".repeat(16)
            )
        );
        assert_eq!(PairLink::parse(&text), Ok(link.clone()));
        assert_eq!(PairLink::parse(&format!("  {text}\n")), Ok(link.clone()));
        assert_eq!(PairLink::parse(&text.replacen("switchboard://pair", "SWITCHBOARD://Pair", 1)), Ok(link.clone()));
        assert!(!format!("{link:?}").contains("abab"), "the secret is never printed by Debug");

        // the relay and the name may be left out, and the order of the parts does not matter
        let bare = PairLink { relay: None, name: String::new(), ..link.clone() };
        assert_eq!(bare.to_string(), format!("switchboard://pair/1?h={}&s={}", home(), "ab".repeat(16)));
        assert_eq!(PairLink::parse(&bare.to_string()), Ok(bare.clone()));
        assert_eq!(PairLink::parse(&format!("switchboard://pair/1?s={}&h={}", "AB".repeat(16), home())), Ok(bare));

        let fresh = PairLink::new(home(), None, "  a\u{7}b  ");
        assert_eq!(fresh.name, "ab");
        assert_ne!(fresh.secret, PairLink::new(home(), None, "").secret);
        assert_eq!(PairLink::parse(&fresh.to_string()), Ok(fresh));
    }

    #[test]
    fn links_are_read_strictly() {
        let h = home().to_string();
        let s = "ab".repeat(16);
        let ok = format!("switchboard://pair/1?h={h}&s={s}");
        assert!(PairLink::parse(&ok).is_ok());
        let cases: Vec<(String, LinkError)> = vec![
            (String::new(), LinkError::NotALink),
            ("switchboard://pair/1".into(), LinkError::NotALink),
            (format!("switchboard://pair/2?h={h}&s={s}"), LinkError::NotALink),
            (format!("https://pair/1?h={h}&s={s}"), LinkError::NotALink),
            (format!("switchboard://pair/1?h={h}"), LinkError::Missing('s')),
            (format!("switchboard://pair/1?s={s}"), LinkError::Missing('h')),
            (format!("{ok}&h={h}"), LinkError::Repeated('h')),
            (format!("{ok}&s={s}"), LinkError::Repeated('s')),
            (format!("{ok}&x=1"), LinkError::Unknown("x".into())),
            (format!("{ok}&H={h}"), LinkError::Unknown("H".into())),
            (format!("{ok}&"), LinkError::Malformed),
            (format!("{ok}&n"), LinkError::Malformed),
            (format!("{ok}&n=%"), LinkError::Malformed),
            (format!("{ok}&n=%4"), LinkError::Malformed),
            (format!("{ok}&n=%zz"), LinkError::Malformed),
            (format!("{ok}&n=%ff"), LinkError::Malformed),
            (format!("{ok}&n=a b"), LinkError::Malformed),
            (format!("{ok}&n=a#b"), LinkError::Malformed),
            (format!("{ok}&n=a=b"), LinkError::Malformed),
            (format!("{ok}&n=é"), LinkError::Malformed),
            (format!("{ok}#frag"), LinkError::Malformed),
            (format!("switchboard://pair/1?h={}&s={s}", &h[1..]), LinkError::BadHome),
            (format!("switchboard://pair/1?h={h}0&s={s}"), LinkError::BadHome),
            (format!("switchboard://pair/1?h={}&s={s}", "zz".repeat(32)), LinkError::BadHome),
            (format!("switchboard://pair/1?h={h}&s={}", &s[2..]), LinkError::BadSecret),
            (format!("switchboard://pair/1?h={h}&s={s}00"), LinkError::BadSecret),
            (format!("switchboard://pair/1?h={h}&s={}", "g".repeat(32)), LinkError::BadSecret),
            (format!("{ok}&r=ftp%3A%2F%2Fx"), LinkError::BadRelay),
            (format!("{ok}&r=https%3A%2F%2F"), LinkError::BadRelay),
            (format!("{ok}&r=https%3A%2F%2Fuser%40host"), LinkError::BadRelay),
            (format!("{ok}&r=https%3A%2F%2Fhost%2F%3Fq"), LinkError::BadRelay),
            (format!("{ok}&r=https%3A%2F%2Fa%20b"), LinkError::BadRelay),
            (format!("{ok}&r="), LinkError::BadRelay),
            (format!("{ok}&r=https%3A%2F%2F{}", "a".repeat(200)), LinkError::BadRelay),
            (format!("{ok}&n=a%0Ab"), LinkError::BadName),
            (format!("{ok}&n=%20a"), LinkError::BadName),
            (format!("{ok}&n={}", "a".repeat(65)), LinkError::BadName),
            (format!("{ok}&n={}", "a".repeat(2000)), LinkError::TooLong),
        ];
        for (text, want) in cases {
            assert_eq!(PairLink::parse(&text), Err(want), "{text}");
        }
        // an id that is 64 hex digits and not a point on the curve is not a computer
        let not_a_key =
            (0u8..=255).map(|b| DeviceId([b; 32])).find(|id| !id.is_key()).expect("some 32 bytes are not a public key");
        assert_eq!(PairLink::parse(&format!("switchboard://pair/1?h={not_a_key}&s={s}")), Err(LinkError::BadHome));
        assert_eq!(PairLink::parse(&format!("{ok}&n={}", "%C3%A9".repeat(64))).unwrap().name.chars().count(), 64);
    }

    #[test]
    fn relay_addresses() {
        for good in [
            "https://relay.example.com",
            "https://relay.example.com/",
            "http://127.0.0.1:3340",
            "https://[fd7a:115c:a1e0::1]:443/relay",
            "http://box.tail1234.ts.net:3340/",
        ] {
            assert!(valid_relay(good), "{good}");
        }
        for bad in [
            "",
            "relay.example.com",
            "https://",
            "https:///path",
            "ws://relay.example.com",
            "HTTPS://relay.example.com",
            "https://user@relay.example.com",
            "https://relay.example.com/?x=1",
            "https://relay.example.com/#x",
            "https://relay example.com",
            "https://relay.example.com\n",
            "https://rélay.example.com",
        ] {
            assert!(!valid_relay(bad), "{bad:?}");
        }
    }

    #[test]
    fn what_was_pasted_is_a_link_or_a_code() {
        let link = PairLink::new(home(), Some("http://127.0.0.1:3340".into()), "Home");
        assert_eq!(PairInput::parse(&link.to_string()), Ok(PairInput::Link(link)));
        assert_eq!(PairInput::parse("abcd-efgh"), Ok(PairInput::Code(ShortCode::parse("ABCDEFGH").unwrap())));
        assert_eq!(PairInput::parse("http://x"), Err(InputError::Link(LinkError::NotALink)));
        assert_eq!(PairInput::parse("abcd"), Err(InputError::Code(CodeError::Length(4))));
    }

    #[test]
    fn random_text_never_panics_the_parsers() {
        let mut dice = Dice(0xc0de_0001);
        let good = PairLink::new(home(), Some("https://relay.example.com/".into()), "Sam's Mac").to_string();
        for round in 0..20_000 {
            let text = match round % 4 {
                // any bytes at all
                0 => String::from_utf8_lossy(&dice.bytes(80)).into_owned(),
                // a real link with a few bytes changed, cut or doubled
                1 => {
                    let mut b = good.clone().into_bytes();
                    for _ in 0..=dice.below(3) {
                        let at = dice.below(b.len());
                        match dice.below(3) {
                            0 => b[at] = dice.next() as u8,
                            1 => b.truncate(at),
                            _ => b.insert(at, b"%&=#?/:+ \0"[dice.below(10)]),
                        }
                        if b.is_empty() {
                            break;
                        }
                    }
                    String::from_utf8_lossy(&b).into_owned()
                }
                // the prefix, then noise from the characters a link is made of
                2 => {
                    let n = dice.below(120);
                    let tail: String =
                        (0..n).map(|_| char::from(b"hrsn=&%0123456789abcdefABCDEF:/.-"[dice.below(33)])).collect();
                    format!("{LINK_PREFIX}{tail}")
                }
                // code-shaped noise
                _ => {
                    let n = dice.below(14);
                    (0..n).map(|_| char::from(b"ABCDEFGHJKMNPQRSTUVWXYZ23456789-- 01ilo"[dice.below(39)])).collect()
                }
            };
            if let Ok(link) = PairLink::parse(&text) {
                // whatever is accepted prints to something that reads back the same
                assert_eq!(PairLink::parse(&link.to_string()), Ok(link), "{text:?}");
            }
            if let Ok(code) = ShortCode::parse(&text) {
                assert_eq!(ShortCode::parse(&code.to_string()), Ok(code), "{text:?}");
            }
            let _ = PairInput::parse(&text);
            let _ = valid_relay(&text);
            let _ = text.parse::<DeviceId>();
        }
    }
}
