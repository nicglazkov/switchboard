//! Terminal tickets. The Python server mints them (`POST /api/term`), the hub checks them once.
//!
//! ticket = base64url(json {m, b, x, n}) + "." + base64url(HMAC-SHA256(key, first part))
//! m machine, b background id, x expiry (unix seconds), n a 16-byte nonce in hex.
//! The key is 32 random bytes in `~/.config/switchboard/term.key` (mode 600), made by whichever of the
//! server or the hub starts first.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::collections::HashMap;
use std::io::Write;
use std::path::Path;
use std::sync::Mutex;

type HmacSha256 = Hmac<Sha256>;

pub const KEY_LEN: usize = 32;
/// A ticket lives this long. The server sets it; the hub refuses any ticket that claims to live longer.
pub const TTL: u64 = 60;
const MAX_LEN: usize = 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Claims {
    pub m: String,
    pub b: String,
    pub x: u64,
    pub n: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TicketError {
    Malformed,
    BadSignature,
    Expired,
    Used,
}

impl TicketError {
    /// What the person sees. Every one of these means: open the terminal again for a new ticket.
    pub fn message(self) -> &'static str {
        match self {
            TicketError::Malformed | TicketError::BadSignature => "This terminal link is not valid. Open the terminal again.",
            TicketError::Expired => "This terminal link expired. Open the terminal again.",
            TicketError::Used => "This terminal link was already used. Open the terminal again.",
        }
    }
}

fn b64(data: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(data)
}

/// base64url with or without `=` padding (Python's urlsafe_b64encode pads).
fn unb64(s: &str) -> Option<Vec<u8>> {
    URL_SAFE_NO_PAD.decode(s.trim_end_matches('=')).ok()
}

fn mac(key: &[u8]) -> HmacSha256 {
    HmacSha256::new_from_slice(key).expect("HMAC takes a key of any length")
}

pub fn valid_machine(m: &str) -> bool {
    !m.is_empty() && m.len() <= 32 && m.bytes().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
}

/// A background session id as `claude agents` prints it: 8 or more lowercase hex characters.
pub fn valid_bg(b: &str) -> bool {
    (8..=64).contains(&b.len()) && b.bytes().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}

fn valid_nonce(n: &str) -> bool {
    n.len() == 32 && n.bytes().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}

/// Mint a ticket (the server does this in Python; the hub uses it in tests and for its own links).
pub fn mint(key: &[u8], machine: &str, bg: &str, now: u64, ttl: u64) -> String {
    let mut n = [0u8; 16];
    getrandom::fill(&mut n).expect("system randomness");
    let claims = Claims { m: machine.into(), b: bg.into(), x: now + ttl, n: n.iter().map(|b| format!("{b:02x}")).collect() };
    sign(key, &claims)
}

pub fn sign(key: &[u8], claims: &Claims) -> String {
    let body = b64(&serde_json::to_vec(claims).expect("claims serialize"));
    let mut m = mac(key);
    m.update(body.as_bytes());
    format!("{body}.{}", b64(&m.finalize().into_bytes()))
}

/// Check signature, shape and expiry. Does not spend the nonce: see [`Nonces::spend`].
pub fn verify(key: &[u8], ticket: &str, now: u64) -> Result<Claims, TicketError> {
    if ticket.len() > MAX_LEN {
        return Err(TicketError::Malformed);
    }
    let (body, sig) = ticket.split_once('.').ok_or(TicketError::Malformed)?;
    let sig = unb64(sig).ok_or(TicketError::Malformed)?;
    let mut m = mac(key);
    m.update(body.as_bytes());
    m.verify_slice(&sig).map_err(|_| TicketError::BadSignature)?; // constant time
    let raw = unb64(body).ok_or(TicketError::Malformed)?;
    let c: Claims = serde_json::from_slice(&raw).map_err(|_| TicketError::Malformed)?;
    if !valid_machine(&c.m) || !valid_bg(&c.b) || !valid_nonce(&c.n) {
        return Err(TicketError::Malformed);
    }
    if c.x <= now {
        return Err(TicketError::Expired);
    }
    if c.x > now + 5 * TTL {
        // signed, but claims a life no server hands out
        return Err(TicketError::Malformed);
    }
    Ok(c)
}

/// Nonces seen, kept until their ticket would have expired anyway.
#[derive(Default)]
pub struct Nonces {
    seen: Mutex<HashMap<String, u64>>,
}

impl Nonces {
    /// True the first time a nonce is spent; false for a ticket already used.
    pub fn spend(&self, c: &Claims, now: u64) -> bool {
        let mut seen = self.seen.lock().unwrap_or_else(|e| e.into_inner());
        seen.retain(|_, x| *x > now);
        if seen.contains_key(&c.n) {
            return false;
        }
        seen.insert(c.n.clone(), c.x);
        true
    }

    pub fn len(&self) -> usize {
        self.seen.lock().map(|s| s.len()).unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Verify and spend in one step: what the hub does with every connection.
pub fn redeem(key: &[u8], nonces: &Nonces, ticket: &str, now: u64) -> Result<Claims, TicketError> {
    let c = verify(key, ticket, now)?;
    if !nonces.spend(&c, now) {
        return Err(TicketError::Used);
    }
    Ok(c)
}

/// Read the shared key, or make it: 32 random bytes, mode 600, in a mode 700 folder. Made with a hard link from a
/// finished temp file, so a server and hub starting at the same moment both end up with the same, whole key.
pub fn load_or_create_key(path: &Path) -> std::io::Result<Vec<u8>> {
    match std::fs::read(path) {
        Ok(k) if k.len() == KEY_LEN => {
            #[cfg(unix)]
            {
                // someone loosened it: anyone who reads it can mint terminal tickets
                use std::os::unix::fs::PermissionsExt;
                if std::fs::metadata(path).is_ok_and(|m| m.permissions().mode() & 0o077 != 0) {
                    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
                }
            }
            return Ok(k);
        }
        Ok(k) => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("{} holds {} bytes, not {KEY_LEN}", path.display(), k.len()),
            ));
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    let dir = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
    }
    let mut key = vec![0u8; KEY_LEN];
    getrandom::fill(&mut key).map_err(|e| std::io::Error::other(e.to_string()))?;
    let tmp = dir.join(format!(".term.key.{}.tmp", std::process::id()));
    {
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut f = opts.open(&tmp)?;
        f.write_all(&key)?;
        f.sync_all()?;
    }
    let linked = std::fs::hard_link(&tmp, path);
    let _ = std::fs::remove_file(&tmp);
    match linked {
        Ok(()) => Ok(key),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => load_or_create_key(path),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: [u8; 32] = [7u8; 32];
    const NOW: u64 = 1_791_000_000;

    #[test]
    fn mint_then_verify() {
        let t = mint(&KEY, "desk", "c03ef0d9", NOW, TTL);
        let c = verify(&KEY, &t, NOW).unwrap();
        assert_eq!((c.m.as_str(), c.b.as_str(), c.x), ("desk", "c03ef0d9", NOW + TTL));
        assert_eq!(c.n.len(), 32);
    }

    #[test]
    fn a_ticket_works_once() {
        let n = Nonces::default();
        let t = mint(&KEY, "desk", "c03ef0d9", NOW, TTL);
        assert!(redeem(&KEY, &n, &t, NOW).is_ok());
        assert_eq!(redeem(&KEY, &n, &t, NOW + 1), Err(TicketError::Used));
    }

    #[test]
    fn spent_nonces_are_forgotten_after_expiry() {
        let n = Nonces::default();
        let t = mint(&KEY, "desk", "c03ef0d9", NOW, TTL);
        redeem(&KEY, &n, &t, NOW).unwrap();
        let t2 = mint(&KEY, "desk", "c03ef0d9", NOW + 100, TTL);
        redeem(&KEY, &n, &t2, NOW + 100).unwrap();
        assert_eq!(n.len(), 1);
    }

    #[test]
    fn expired_at_ttl() {
        let t = mint(&KEY, "desk", "c03ef0d9", NOW, TTL);
        assert!(verify(&KEY, &t, NOW + TTL - 1).is_ok());
        assert_eq!(verify(&KEY, &t, NOW + TTL), Err(TicketError::Expired));
    }

    #[test]
    fn other_key_or_edited_body_fails() {
        let t = mint(&KEY, "desk", "c03ef0d9", NOW, TTL);
        assert_eq!(verify(&[8u8; 32], &t, NOW), Err(TicketError::BadSignature));
        let (body, sig) = t.split_once('.').unwrap();
        let mut c: Claims = serde_json::from_slice(&unb64(body).unwrap()).unwrap();
        c.m = "laptop".into();
        let forged = format!("{}.{sig}", b64(&serde_json::to_vec(&c).unwrap()));
        assert_eq!(verify(&KEY, &forged, NOW), Err(TicketError::BadSignature));
    }

    #[test]
    fn malformed() {
        for t in ["", "abc", ".", "a.b", "!!.!!", &"a".repeat(2000)] {
            assert!(verify(&KEY, t, NOW).is_err(), "{t}");
        }
    }

    #[test]
    fn signed_but_bad_fields_are_refused() {
        let ok = Claims { m: "desk".into(), b: "c03ef0d9".into(), x: NOW + 30, n: "0".repeat(32) };
        assert!(verify(&KEY, &sign(&KEY, &ok), NOW).is_ok());
        for bad in [
            Claims { m: "../etc".into(), ..ok.clone() },
            Claims { m: "".into(), ..ok.clone() },
            Claims { b: "c03ef0d".into(), ..ok.clone() },
            Claims { b: "C03EF0D9".into(), ..ok.clone() },
            Claims { b: "c03ef0d9;rm".into(), ..ok.clone() },
            Claims { n: "xyz".into(), ..ok.clone() },
            Claims { x: NOW + 3600, ..ok.clone() },
        ] {
            assert_eq!(verify(&KEY, &sign(&KEY, &bad), NOW), Err(TicketError::Malformed), "{bad:?}");
        }
    }

    /// The same vector is checked by tests/test_term.py, so the Python minting and this check agree byte for byte.
    #[test]
    fn python_vector() {
        let key: Vec<u8> = (0u8..32).collect();
        let t = "eyJtIjogImRlc2siLCAiYiI6ICJjMDNlZjBkOSIsICJ4IjogMTc5MTAwMDA2MCwgIm4iOiAiMDAxMTIyMzM0NDU1NjY3Nzg4OTlhYWJiY2NkZGVlZmYifQ.5ttdVCRzjRL4eEeV7w6PyJnxf2sfnQwToqAVR-rojtU";
        let c = verify(&key, t, NOW).unwrap();
        assert_eq!(c, Claims { m: "desk".into(), b: "c03ef0d9".into(), x: NOW + 60, n: "00112233445566778899aabbccddeeff".into() });
        // padded base64 (Python's urlsafe_b64encode) is accepted too
        let (body, sig) = t.split_once('.').unwrap();
        assert!(verify(&key, &format!("{body}.{sig}="), NOW).is_ok());
    }

    #[test]
    fn key_file_is_made_once_with_mode_600() {
        let dir = std::env::temp_dir().join(format!("sbhub-key-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let p = dir.join("cfg").join("term.key");
        let a = load_or_create_key(&p).unwrap();
        let b = load_or_create_key(&p).unwrap();
        assert_eq!(a, b);
        assert_eq!(a.len(), 32);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&p).unwrap().permissions().mode() & 0o777, 0o600);
            // security I3: a key someone loosened is made private again when it is read
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o644)).unwrap();
            assert_eq!(load_or_create_key(&p).unwrap(), a);
            assert_eq!(std::fs::metadata(&p).unwrap().permissions().mode() & 0o777, 0o600);
        }
        std::fs::write(&p, b"short").unwrap();
        assert!(load_or_create_key(&p).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
