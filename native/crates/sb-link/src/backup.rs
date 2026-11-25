//! A copy of the home computer that is useless without its recovery code.
//!
//! The file holds the home computer's state (one stream, made by `server/homestate.py`) and then its key, sealed
//! under a key worked out from the recovery code. Whoever has the file and the code is the home computer;
//! whoever has the file alone has nothing.
//!
//! ```text
//! "SBHOME1\n"
//! {"v":1,"home":"<hex id>","epoch":N,"made":<unix s>,"version":"<x.y.z>",
//!  "kdf":"argon2id","m":65536,"t":3,"p":1,"salt":"<b64url 16>","nonce":"<b64url 19>"}\n      (one line)
//! chunk 0, chunk 1, ... , last chunk
//! ```
//!
//! What is sealed is the state stream, then the 11 bytes `device.key\n`, then the 32 bytes of the key. It is cut
//! into pieces of 1 MiB, each sealed with XChaCha20-Poly1305 (so each is 16 bytes longer in the file). The nonce
//! of a piece is the 19 bytes from the header, then the piece's number as 4 bytes big endian, then one byte
//! that is 1 for the last piece and 0 for the others. The header line, without its line end, is authenticated
//! with every piece. So a piece that is changed, moved, left out or repeated fails, a file that is cut short
//! fails (its last piece is not marked as the last), and so does a header that was edited.
//!
//! The key that seals is Argon2id of the recovery code (its 26 characters, upper case, no dashes) with the salt
//! from the header: 64 MiB, 3 passes, one lane, 32 bytes out.
//!
//! `open` writes the state stream out as it checks each piece, so what it wrote is to be thrown away unless it
//! returns Ok: write to a scratch file, and use it only then.

use std::fmt;
use std::io::{self, Read, Write};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, Zeroizing};

use crate::code::ALPHABET;
use crate::identity::KEY_LEN;

pub const MAGIC: &[u8; 8] = b"SBHOME1\n";
pub const FORMAT: u32 = 1;
/// Bytes sealed in one piece.
pub const CHUNK: usize = 1024 * 1024;
const TAG: usize = 16;
pub const SALT_LEN: usize = 16;
pub const NONCE_LEN: usize = 19;
pub const KDF_MEM_KIB: u32 = 64 * 1024;
pub const KDF_PASSES: u32 = 3;
/// Characters in a recovery code: 26 of 31, a little over 128 bits.
pub const CODE_LEN: usize = 26;
/// What stands between the state and the key.
const KEY_MARK: &[u8; 11] = b"device.key\n";
const TRAILER: usize = KEY_MARK.len() + KEY_LEN;
/// Longest header line read.
const HEADER_MAX: usize = 4096;
/// The most memory a file may ask the slow hash for: 1 GiB. A file cannot make this computer use more.
const KDF_MEM_MAX: u32 = 1024 * 1024;
const KDF_PASSES_MAX: u32 = 16;

#[derive(Debug)]
pub enum BackupError {
    /// Not a Switchboard backup at all.
    NotBackup,
    /// Made by a newer version.
    Newer,
    /// The first piece did not open: the code is wrong, or the file was changed.
    WrongCode,
    /// A later piece did not open, or the file ends early.
    Damaged,
    Io(io::Error),
}

impl fmt::Display for BackupError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BackupError::NotBackup => f.write_str("This file is not a Switchboard backup."),
            BackupError::Newer => f.write_str(
                "This backup was made by a newer version of Switchboard. Update Switchboard on this computer, then try again.",
            ),
            BackupError::WrongCode => {
                f.write_str("The recovery code does not open this backup. Check the code, and that the file is whole.")
            }
            BackupError::Damaged => f.write_str("The backup is cut short or was changed. Nothing was restored."),
            BackupError::Io(e) => write!(f, "The backup could not be read or written ({e})."),
        }
    }
}

impl std::error::Error for BackupError {}

impl From<io::Error> for BackupError {
    fn from(e: io::Error) -> Self {
        BackupError::Io(e)
    }
}

/// The 26 characters that open a backup. Shown once, when the first backup is made; never stored.
#[derive(Clone, PartialEq, Eq)]
pub struct RecoveryCode(String);

impl RecoveryCode {
    pub fn generate() -> RecoveryCode {
        let mut out = String::with_capacity(CODE_LEN);
        while out.len() < CODE_LEN {
            let mut bytes: [u8; 32] = crate::random();
            for b in bytes {
                // 248 is the largest multiple of 31 in a byte: every character is as likely as any other
                if b < 248 && out.len() < CODE_LEN {
                    out.push(char::from(ALPHABET[usize::from(b % 31)]));
                }
            }
            bytes.zeroize();
        }
        RecoveryCode(out)
    }

    /// The code as a person typed it: either case, with or without dashes and spaces. None when it is not 26
    /// characters of the alphabet.
    pub fn parse(typed: &str) -> Option<RecoveryCode> {
        if typed.len() > 4 * CODE_LEN {
            return None;
        }
        let mut out = String::with_capacity(CODE_LEN);
        for c in typed.chars() {
            if c == '-' || c.is_whitespace() {
                continue;
            }
            let up = c.to_ascii_uppercase();
            if !up.is_ascii() || !ALPHABET.contains(&(up as u8)) || out.len() == CODE_LEN {
                out.zeroize();
                return None;
            }
            out.push(up);
        }
        if out.len() != CODE_LEN {
            out.zeroize();
            return None;
        }
        Some(RecoveryCode(out))
    }

    /// The key that seals, for this salt and these costs. Slow on purpose: call it off the async threads.
    fn derive(&self, salt: &[u8; SALT_LEN], mem_kib: u32, passes: u32, lanes: u32) -> Option<Zeroizing<[u8; KEY_LEN]>> {
        let params = argon2::Params::new(mem_kib, passes, lanes, Some(KEY_LEN)).ok()?;
        let mut out = Zeroizing::new([0u8; KEY_LEN]);
        argon2::Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, params)
            .hash_password_into(self.0.as_bytes(), salt, &mut *out)
            .ok()?;
        Some(out)
    }
}

impl Drop for RecoveryCode {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

/// `XXXXX-XXXXX-XXXXX-XXXXX-XXXXX-X`, the way the code is shown to its owner.
impl fmt::Display for RecoveryCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, c) in self.0.chars().enumerate() {
            if i > 0 && i % 5 == 0 {
                f.write_str("-")?;
            }
            write!(f, "{c}")?;
        }
        Ok(())
    }
}

/// Never the code itself: a log line or a panic message that holds one must not show it.
impl fmt::Debug for RecoveryCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RecoveryCode(..)")
    }
}

/// What seals a backup: the salt and the key worked out from the recovery code with it. A home computer keeps
/// this (not the code) so that it can make its next backup without asking. It cannot be turned back into the code,
/// and it opens only backups made with this salt.
#[derive(Clone)]
pub struct Sealer {
    pub salt: [u8; SALT_LEN],
    key: Zeroizing<[u8; KEY_LEN]>,
}

impl Sealer {
    /// A new salt, and the key for it. Slow (see `RecoveryCode::derive`).
    pub fn new(code: &RecoveryCode) -> Sealer {
        Sealer::with_salt(code, crate::random())
    }

    pub fn with_salt(code: &RecoveryCode, salt: [u8; SALT_LEN]) -> Sealer {
        let key = code.derive(&salt, KDF_MEM_KIB, KDF_PASSES, 1).expect("fixed Argon2 costs are valid");
        Sealer { salt, key }
    }

    /// As stored (`to_stored`). None when it is not one.
    pub fn from_stored(salt: &str, key: &str) -> Option<Sealer> {
        let salt: [u8; SALT_LEN] = B64.decode(salt).ok()?.try_into().ok()?;
        let mut raw = B64.decode(key).ok()?;
        let key: Option<[u8; KEY_LEN]> = raw.as_slice().try_into().ok();
        raw.zeroize();
        Some(Sealer { salt, key: Zeroizing::new(key?) })
    }

    /// (salt, key) as text, for a file only this user can read.
    pub fn to_stored(&self) -> (String, Zeroizing<String>) {
        (B64.encode(self.salt), Zeroizing::new(B64.encode(*self.key)))
    }
}

impl fmt::Debug for Sealer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Sealer(..)")
    }
}

/// The first line of a backup, readable without the code: which home, how new, and how to work out the key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Header {
    pub v: u32,
    pub home: String,
    pub epoch: u64,
    pub made: u64,
    pub version: String,
    pub kdf: String,
    pub m: u32,
    pub t: u32,
    pub p: u32,
    pub salt: String,
    pub nonce: String,
}

/// What the caller knows about the backup it is making.
#[derive(Debug, Clone)]
pub struct About {
    /// The home computer's id, as hex.
    pub home: String,
    pub epoch: u64,
    /// Unix seconds.
    pub made: u64,
    pub version: String,
}

fn nonce(prefix: &[u8; NONCE_LEN], n: u32, last: bool) -> XNonce {
    let mut b = [0u8; 24];
    b[..NONCE_LEN].copy_from_slice(prefix);
    b[NONCE_LEN..NONCE_LEN + 4].copy_from_slice(&n.to_be_bytes());
    b[23] = u8::from(last);
    XNonce::from(b)
}

/// Reads until `buf` is full or the input ends. How many bytes were read.
fn fill(input: &mut impl Read, buf: &mut [u8]) -> io::Result<usize> {
    let mut n = 0;
    while n < buf.len() {
        match input.read(&mut buf[n..]) {
            Ok(0) => break,
            Ok(k) => n += k,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(n)
}

/// Seals `state` and then `device_key` into `out`. Returns the header it wrote.
pub fn seal(
    sealer: &Sealer,
    about: &About,
    state: &mut impl Read,
    device_key: &[u8; KEY_LEN],
    out: &mut impl Write,
) -> Result<Header, BackupError> {
    seal_with(sealer, about, crate::random(), CHUNK, state, device_key, out)
}

fn seal_with(
    sealer: &Sealer,
    about: &About,
    prefix: [u8; NONCE_LEN],
    chunk: usize,
    state: &mut impl Read,
    device_key: &[u8; KEY_LEN],
    out: &mut impl Write,
) -> Result<Header, BackupError> {
    let header = Header {
        v: FORMAT,
        home: about.home.clone(),
        epoch: about.epoch,
        made: about.made,
        version: about.version.clone(),
        kdf: "argon2id".into(),
        m: KDF_MEM_KIB,
        t: KDF_PASSES,
        p: 1,
        salt: B64.encode(sealer.salt),
        nonce: B64.encode(prefix),
    };
    let line = serde_json::to_string(&header).map_err(|e| BackupError::Io(io::Error::other(e)))?;
    out.write_all(MAGIC)?;
    out.write_all(line.as_bytes())?;
    out.write_all(b"\n")?;
    let cipher = XChaCha20Poly1305::new((&*sealer.key).into());
    let put = |n: u32, last: bool, plain: &[u8], out: &mut dyn Write| -> Result<(), BackupError> {
        let sealed = cipher
            .encrypt(&nonce(&prefix, n, last), Payload { msg: plain, aad: line.as_bytes() })
            .map_err(|_| BackupError::Io(io::Error::other("sealing failed")))?;
        out.write_all(&sealed)?;
        Ok(())
    };
    // One piece is always held back, so the last one written is known to be the last. The key goes in after the
    // state has ended, and is wiped from this buffer when done.
    let mut buf = Zeroizing::new(vec![0u8; chunk + TRAILER]);
    let mut have = 0usize;
    let mut n = 0u32;
    let mut ended = false;
    loop {
        if !ended {
            let got = fill(state, &mut buf[have..chunk])?;
            have += got;
            if have < chunk {
                ended = true;
                buf[have..have + KEY_MARK.len()].copy_from_slice(KEY_MARK);
                buf[have + KEY_MARK.len()..have + TRAILER].copy_from_slice(device_key);
                have += TRAILER;
            }
        }
        if ended && have <= chunk {
            put(n, true, &buf[..have], out)?;
            break;
        }
        put(n, false, &buf[..chunk], out)?;
        n = n.checked_add(1).ok_or_else(|| BackupError::Io(io::Error::other("the state is too large")))?;
        buf.copy_within(chunk..have, 0);
        have -= chunk;
    }
    out.flush()?;
    Ok(header)
}

/// The header of a backup, and the exact bytes of its line (they are authenticated with every piece). Reads no
/// further than the line's end.
pub fn read_header(input: &mut impl Read) -> Result<(Header, Vec<u8>), BackupError> {
    let mut magic = [0u8; 8];
    if fill(input, &mut magic)? != magic.len() || &magic != MAGIC {
        return Err(BackupError::NotBackup);
    }
    let mut line = Vec::new();
    let mut b = [0u8; 1];
    loop {
        if fill(input, &mut b)? == 0 || line.len() > HEADER_MAX {
            return Err(BackupError::Damaged);
        }
        if b[0] == b'\n' {
            break;
        }
        line.push(b[0]);
    }
    // the format number first, alone: a newer header may have fields this version cannot read
    #[derive(Deserialize)]
    struct V {
        v: u32,
    }
    let v: V = serde_json::from_slice(&line).map_err(|_| BackupError::Damaged)?;
    if v.v > FORMAT {
        return Err(BackupError::Newer);
    }
    let h: Header = serde_json::from_slice(&line).map_err(|_| BackupError::Damaged)?;
    Ok((h, line))
}

/// Opens a backup with its recovery code: writes the state stream to `out` and returns the header and the home
/// computer's key. On any error, what was written to `out` so far must be thrown away.
pub fn open(
    code: &RecoveryCode,
    input: &mut impl Read,
    out: &mut impl Write,
) -> Result<(Header, Zeroizing<[u8; KEY_LEN]>), BackupError> {
    open_with(code, CHUNK, input, out)
}

fn open_with(
    code: &RecoveryCode,
    chunk: usize,
    input: &mut impl Read,
    out: &mut impl Write,
) -> Result<(Header, Zeroizing<[u8; KEY_LEN]>), BackupError> {
    open_keyed(|salt, h| code.derive(salt, h.m, h.t, h.p), chunk, input, out)
}

/// `derive` gives the key for a header that has passed its checks (the tests have one ready, since the real one
/// takes a good part of a second).
fn open_keyed(
    derive: impl FnOnce(&[u8; SALT_LEN], &Header) -> Option<Zeroizing<[u8; KEY_LEN]>>,
    chunk: usize,
    input: &mut impl Read,
    out: &mut impl Write,
) -> Result<(Header, Zeroizing<[u8; KEY_LEN]>), BackupError> {
    let (header, line) = read_header(input)?;
    if header.v != FORMAT
        || header.kdf != "argon2id"
        || header.p != 1
        || header.m > KDF_MEM_MAX
        || header.t == 0
        || header.t > KDF_PASSES_MAX
    {
        return Err(BackupError::Damaged);
    }
    let salt: [u8; SALT_LEN] =
        B64.decode(&header.salt).ok().and_then(|s| s.try_into().ok()).ok_or(BackupError::Damaged)?;
    let prefix: [u8; NONCE_LEN] =
        B64.decode(&header.nonce).ok().and_then(|s| s.try_into().ok()).ok_or(BackupError::Damaged)?;
    let key = derive(&salt, &header).ok_or(BackupError::Damaged)?;
    let cipher = XChaCha20Poly1305::new((&*key).into());

    // One sealed piece and one byte more: the byte says whether another piece follows.
    let mut buf = vec![0u8; chunk + TAG + 1];
    let mut have = fill(input, &mut buf)?;
    // The end of what has been opened and not yet written: the last TRAILER bytes are the key, never the state.
    let mut tail = Zeroizing::new(Vec::<u8>::with_capacity(chunk + 2 * TRAILER));
    let mut n = 0u32;
    loop {
        let last = have <= chunk + TAG;
        let sealed = &buf[..have.min(chunk + TAG)];
        let plain = cipher
            .decrypt(&nonce(&prefix, n, last), Payload { msg: sealed, aad: &line })
            .map(Zeroizing::new)
            .map_err(|_| if n == 0 { BackupError::WrongCode } else { BackupError::Damaged })?;
        tail.extend_from_slice(&plain);
        if last {
            break;
        }
        if tail.len() > TRAILER {
            let keep = tail.len() - TRAILER;
            out.write_all(&tail[..keep])?;
            tail.copy_within(keep.., 0);
            tail.truncate(TRAILER);
        }
        n = n.checked_add(1).ok_or(BackupError::Damaged)?;
        buf[0] = buf[chunk + TAG];
        have = 1 + fill(input, &mut buf[1..])?;
    }
    if tail.len() < TRAILER {
        return Err(BackupError::Damaged);
    }
    let at = tail.len() - TRAILER;
    if &tail[at..at + KEY_MARK.len()] != KEY_MARK {
        return Err(BackupError::Damaged);
    }
    out.write_all(&tail[..at])?;
    out.flush()?;
    let mut device_key = Zeroizing::new([0u8; KEY_LEN]);
    device_key.copy_from_slice(&tail[at + KEY_MARK.len()..]);
    Ok((header, device_key))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::Dice;

    const CODE: &str = "ABCDE-FGHJK-MNPQR-STUVW-XYZ23-4";

    fn about() -> About {
        About { home: "ab".repeat(32), epoch: 7, made: 1_795_000_000, version: "0.9.0".into() }
    }

    /// One slow hash for the whole test run: every test that only needs some sealer shares it.
    fn sealer() -> Sealer {
        static S: std::sync::OnceLock<Sealer> = std::sync::OnceLock::new();
        S.get_or_init(|| Sealer::with_salt(&RecoveryCode::parse(CODE).unwrap(), [7u8; SALT_LEN])).clone()
    }

    fn sealed(state: &[u8], chunk: usize) -> Vec<u8> {
        let mut out = Vec::new();
        seal_with(&sealer(), &about(), [9u8; NONCE_LEN], chunk, &mut &state[..], &[0x42; KEY_LEN], &mut out).unwrap();
        out
    }

    fn opened(file: &[u8], chunk: usize) -> Result<(Vec<u8>, [u8; KEY_LEN]), BackupError> {
        let mut out = Vec::new();
        let (_, key) = open_keyed(|_, _| Some(sealer().key), chunk, &mut &file[..], &mut out)?;
        Ok((out, *key))
    }

    #[test]
    fn a_code_is_26_characters_shown_in_fives_and_typed_any_way() {
        let c = RecoveryCode::generate();
        let shown = c.to_string();
        assert_eq!(shown.len(), 31);
        assert_eq!(shown.split('-').map(str::len).collect::<Vec<_>>(), [5, 5, 5, 5, 5, 1]);
        assert!(shown.bytes().all(|b| b == b'-' || ALPHABET.contains(&b)));
        assert!(RecoveryCode::parse(&shown) == Some(c.clone()));
        assert!(RecoveryCode::parse(&shown.to_lowercase().replace('-', " ")) == Some(c.clone()));
        assert!(RecoveryCode::parse(&format!("  {}\n", shown.replace('-', ""))) == Some(c.clone()));
        assert!(RecoveryCode::generate() != c);
        for bad in
            ["", "ABCDE", &shown[..30], &format!("{shown}A"), &shown.replace(&shown[..1], "0"), "é".repeat(26).as_str()]
        {
            assert!(RecoveryCode::parse(bad).is_none(), "{bad}");
        }
        assert_eq!(format!("{c:?}"), "RecoveryCode(..)");
        assert_eq!(format!("{:?}", sealer()), "Sealer(..)");
    }

    #[test]
    fn what_is_sealed_comes_back_at_every_size_around_a_piece() {
        let mut dice = Dice(4);
        let chunk = 64;
        for len in [0, 1, 20, 21, 22, 63, 64, 65, 64 + 21, 128, 128 + 20, 128 + 21, 128 + 22, 1000] {
            let state: Vec<u8> = (0..len).map(|_| dice.next() as u8).collect();
            let file = sealed(&state, chunk);
            let total = len + TRAILER;
            let pieces = total.div_ceil(chunk).max(1);
            let head = file.iter().skip(MAGIC.len()).position(|b| *b == b'\n').unwrap() + MAGIC.len() + 1;
            assert_eq!(file.len(), head + total + pieces * TAG, "{len}");
            let (back, key) = opened(&file, chunk).unwrap();
            assert_eq!(back, state, "{len}");
            assert_eq!(key, [0x42; KEY_LEN]);
        }
    }

    #[test]
    fn the_key_and_the_state_are_not_readable_in_the_file() {
        let state = b"paired.json and everything else".repeat(10);
        let file = sealed(&state, 64);
        assert!(!file.windows(8).any(|w| w == b"paired.j"));
        assert!(!file.windows(8).any(|w| w == [0x42; 8]));
        assert!(!file.windows(10).any(|w| w == b"device.key"));
    }

    #[test]
    fn the_wrong_code_opens_nothing() {
        let file = sealed(b"state", 64);
        let other = RecoveryCode::parse("ABCDE-FGHJK-MNPQR-STUVW-XYZ23-5").unwrap();
        let mut out = Vec::new();
        let e = open_with(&other, 64, &mut &file[..], &mut out).unwrap_err();
        assert!(matches!(e, BackupError::WrongCode), "{e:?}");
        assert!(out.is_empty());
        assert!(e.to_string().contains("recovery code"));
    }

    #[test]
    fn a_cut_file_is_refused_wherever_it_is_cut() {
        let state: Vec<u8> = (0..300u32).map(|i| i as u8).collect();
        let file = sealed(&state, 64);
        for keep in 0..file.len() {
            let r = opened(&file[..keep], 64);
            assert!(r.is_err(), "cut at {keep} of {}", file.len());
        }
        // cut exactly between two pieces: every piece left is whole, and it is still refused
        let head = file.iter().skip(MAGIC.len()).position(|b| *b == b'\n').unwrap() + MAGIC.len() + 1;
        let e = opened(&file[..head + 2 * (64 + TAG)], 64).unwrap_err();
        assert!(matches!(e, BackupError::Damaged), "{e:?}");
        assert!(opened(&file, 64).is_ok());
    }

    #[test]
    fn a_flipped_bit_anywhere_is_refused() {
        let state: Vec<u8> = (0..200u32).map(|i| i as u8).collect();
        let file = sealed(&state, 64);
        for at in 0..file.len() {
            let mut bad = file.clone();
            bad[at] ^= 0x10;
            assert!(opened(&bad, 64).is_err(), "bit flipped at {at}");
        }
    }

    #[test]
    fn pieces_cannot_be_moved_left_out_or_repeated() {
        let state: Vec<u8> = (0..300u32).map(|i| i as u8).collect();
        let file = sealed(&state, 64);
        let head = file.iter().skip(MAGIC.len()).position(|b| *b == b'\n').unwrap() + MAGIC.len() + 1;
        let piece = 64 + TAG;
        let part = |i: usize| &file[head + i * piece..(head + (i + 1) * piece).min(file.len())];
        let pieces = (file.len() - head).div_ceil(piece);
        assert!(pieces >= 5);
        let join = |order: &[usize]| {
            let mut f = file[..head].to_vec();
            for i in order {
                f.extend_from_slice(part(*i));
            }
            f
        };
        let all: Vec<usize> = (0..pieces).collect();
        assert!(opened(&join(&all), 64).is_ok());
        let mut swapped = all.clone();
        swapped.swap(1, 2);
        assert!(matches!(opened(&join(&swapped), 64), Err(BackupError::Damaged)));
        let mut dropped = all.clone();
        dropped.remove(2);
        assert!(matches!(opened(&join(&dropped), 64), Err(BackupError::Damaged)));
        let mut twice = all.clone();
        twice.insert(2, 1);
        assert!(matches!(opened(&join(&twice), 64), Err(BackupError::Damaged)));
        let mut no_first = all.clone();
        no_first.remove(0);
        assert!(opened(&join(&no_first), 64).is_err());
        // the last piece alone, after the header: it is marked last, but it is not piece 0
        assert!(opened(&join(&[pieces - 1]), 64).is_err());
        // more after the last piece
        let mut more = join(&all);
        more.extend_from_slice(part(0));
        assert!(opened(&more, 64).is_err());
    }

    #[test]
    fn a_piece_from_another_backup_of_the_same_home_does_not_fit() {
        let a = sealed(&[1u8; 200], 64);
        let mut out = Vec::new();
        seal_with(&sealer(), &about(), [8u8; NONCE_LEN], 64, &mut &[1u8; 200][..], &[0x42; KEY_LEN], &mut out).unwrap();
        let head = a.iter().skip(MAGIC.len()).position(|b| *b == b'\n').unwrap() + MAGIC.len() + 1;
        assert_eq!(a.len(), out.len());
        let mut mixed = a.clone();
        mixed[head + 80..head + 160].copy_from_slice(&out[head + 80..head + 160]);
        assert!(opened(&mixed, 64).is_err());
    }

    #[test]
    fn the_header_is_read_without_the_code_and_a_newer_one_is_refused() {
        let file = sealed(b"state", 64);
        let (h, line) = read_header(&mut &file[..]).unwrap();
        assert_eq!((h.v, h.epoch, h.made, h.version.as_str()), (1, 7, 1_795_000_000, "0.9.0"));
        assert_eq!(h.home, "ab".repeat(32));
        assert_eq!((h.kdf.as_str(), h.m, h.t, h.p), ("argon2id", 65536, 3, 1));
        assert!(line.starts_with(br#"{"v":1,"home":""#));
        assert!(matches!(read_header(&mut &b"PK\x03\x04 a zip"[..]), Err(BackupError::NotBackup)));
        assert!(matches!(read_header(&mut &b""[..]), Err(BackupError::NotBackup)));
        let newer = [&MAGIC[..], br#"{"v":2,"shape":"unknown"}"#, b"\nrest"].concat();
        let e = read_header(&mut &newer[..]).unwrap_err();
        assert!(matches!(e, BackupError::Newer));
        assert!(e.to_string().contains("newer version"));
        let endless = [&MAGIC[..], &[b'x'; 5000][..]].concat();
        assert!(matches!(read_header(&mut &endless[..]), Err(BackupError::Damaged)));
    }

    #[test]
    fn a_header_cannot_ask_for_more_memory_than_allowed() {
        let file = sealed(b"state", 64);
        let text = String::from_utf8_lossy(&file[..200]).into_owned();
        assert!(text.contains(r#""m":65536"#));
        let at = file.windows(9).position(|w| w == br#""m":65536"#).unwrap();
        let mut big = file[..at].to_vec();
        big.extend_from_slice(br#""m":99999999"#);
        big.extend_from_slice(&file[at + 9..]);
        let started = std::time::Instant::now();
        assert!(matches!(opened(&big, 64), Err(BackupError::Damaged)));
        assert!(started.elapsed() < std::time::Duration::from_secs(2));
    }

    #[test]
    fn a_stored_sealer_makes_the_same_backups() {
        let s = sealer();
        let (salt, key) = s.to_stored();
        let back = Sealer::from_stored(&salt, &key).unwrap();
        let mut a = Vec::new();
        seal_with(&back, &about(), [9u8; NONCE_LEN], 64, &mut &b"state"[..], &[0x42; KEY_LEN], &mut a).unwrap();
        assert_eq!(a, sealed(b"state", 64));
        assert!(Sealer::from_stored(&salt, "AAAA").is_none());
        assert!(Sealer::from_stored("AAAA", &key).is_none());
        assert!(Sealer::from_stored("not base64 !", &key).is_none());
    }

    #[test]
    fn real_sized_pieces_and_a_random_nonce() {
        let state: Vec<u8> = (0..CHUNK + 5).map(|i| (i % 251) as u8).collect();
        let mut a = Vec::new();
        let h = seal(&sealer(), &about(), &mut &state[..], &[0x42; KEY_LEN], &mut a).unwrap();
        let mut b = Vec::new();
        let h2 = seal(&sealer(), &about(), &mut &state[..], &[0x42; KEY_LEN], &mut b).unwrap();
        assert_ne!(h.nonce, h2.nonce);
        assert_ne!(a[a.len() - 64..], b[b.len() - 64..]);
        let mut out = Vec::new();
        let (back, key) = open(&RecoveryCode::parse(CODE).unwrap(), &mut &a[..], &mut out).unwrap();
        assert_eq!(back, h);
        assert_eq!(out, state);
        assert_eq!(*key, [0x42; KEY_LEN]);
    }

    // Worked out apart from this code, with libsodium through Python's PyNaCl: `pwhash.argon2id.kdf(32, code, salt,
    // opslimit=3, memlimit=65536 * 1024)` and `crypto_aead_xchacha20poly1305_ietf_encrypt(piece, header_line,
    // nonce, key)`, for code ABCDEFGHJKMNPQRSTUVWXYZ234, salt 00..0f, nonce start 64..76, key 00..1f.
    const V_LINE: &str = r#"{"v":1,"home":"abababababababababababababababababababababababababababababababab","epoch":7,"made":1795000000,"version":"0.9.0","kdf":"argon2id","m":65536,"t":3,"p":1,"salt":"AAECAwQFBgcICQoLDA0ODw","nonce":"ZGVmZ2hpamtsbW5vcHFyc3R1dg"}"#;
    const V_SMALL: &str = "5144a39c79620721b2bb04a5ba156f24b21c31bfe604443c592a978740410c595bf94bbdaa031fe90865a0a78592ce6e64fcbca2a68e126c8800b316489d30ea44b6a5b0077a";
    const V_BIG_LEN: usize = 1_048_900;
    const V_BIG_FIRST: &str = "e581374b94cb099532383e5c43363def6cf5bfd8bce994baa360f60990a4a8e0";
    const V_BIG_LAST: &str = "45ea4ca4cc1241937e6e012270531992b4a95e2a742664be10866b52af7074fa";

    #[test]
    fn fixed_vectors_from_another_implementation() {
        let code = RecoveryCode::parse("abcde-fghjk-mnpqr-stuvw-xyz23-4").unwrap();
        let salt: [u8; SALT_LEN] = std::array::from_fn(|i| i as u8);
        let prefix: [u8; NONCE_LEN] = std::array::from_fn(|i| 100 + i as u8);
        let key: [u8; KEY_LEN] = std::array::from_fn(|i| i as u8);
        let s = Sealer::with_salt(&code, salt);

        let mut small = Vec::new();
        seal_with(&s, &about(), prefix, CHUNK, &mut &b"home state\n"[..], &key, &mut small).unwrap();
        let head = MAGIC.len() + V_LINE.len() + 1;
        assert_eq!(&small[..MAGIC.len()], MAGIC);
        assert_eq!(std::str::from_utf8(&small[MAGIC.len()..head - 1]).unwrap(), V_LINE);
        assert_eq!(crate::hex::encode(&small[head..]), V_SMALL);

        // and the other way: a file this code did not write
        let mut theirs = [&MAGIC[..], V_LINE.as_bytes(), b"\n"].concat();
        for pair in V_SMALL.as_bytes().chunks(2) {
            theirs.push(u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap());
        }
        let mut out = Vec::new();
        let (h, k) = open(&code, &mut &theirs[..], &mut out).unwrap();
        assert_eq!((out.as_slice(), *k, h.epoch), (&b"home state\n"[..], key, 7));

        // two pieces of the real size
        let state: Vec<u8> = (0..CHUNK + 5).map(|i| (i % 251) as u8).collect();
        let mut big = Vec::new();
        seal_with(&s, &about(), prefix, CHUNK, &mut &state[..], &key, &mut big).unwrap();
        assert_eq!(big.len(), V_BIG_LEN);
        assert_eq!(crate::hex::encode(&big[head..head + 32]), V_BIG_FIRST);
        assert_eq!(crate::hex::encode(&big[big.len() - 32..]), V_BIG_LAST);
    }
}
