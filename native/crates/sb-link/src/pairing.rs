//! The pairing exchange: a device proves it holds the code or the link's secret, home proves the same back, and
//! they tell each other who they are.
//!
//! It runs over one stream of any kind (`join` on the device, `host` on home). Each side is given both ends' ids
//! and a value tied to the channel under the stream (the TLS exporter, once the stream is a QUIC one), and all
//! three go into the keys. So a middleman who relays the frames between two channels of his own ends up with
//! keys that do not match, exactly as if he had guessed the code wrong.
//!
//! ```text
//!   joiner                                          host
//!   HELLO      version, hint, key-exchange message  ->
//!                                                   <-  REPLY     key-exchange message
//!   CONFIRM_J  proof of the key                     ->
//!                                                   <-  CONFIRM_H proof of the key
//!   ABOUT      sealed {name, kind, os}              ->
//!                                                   <-  WELCOME   sealed {home_id, home_name, relay,
//!                                                                         machine, token}
//! ```
//!
//! Every frame is a 2-byte length, a type byte and a body, and none may be longer than `FRAME_MAX`. The host can
//! answer any frame with ABORT and a reason.
//!
//! The joiner proves itself first, so home never hands a proof to someone who has not shown the code. A wrong
//! proof is one failed guess: the key exchange (pake.rs) gives a guesser nothing to test other codes against.
//! Codes and links last ten minutes, work once, and are dead after five wrong proofs (`Offers`).
//!
//! What this crate does not know: the hub. Home's side takes a function that makes the machine name and token
//! for the device that just proved itself.

mod pake;

use crate::code::{PairLink, SECRET_LEN, ShortCode, valid_relay};
use crate::identity::DeviceId;
use crate::paired::{Kind, NAME_MAX, Via};
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use pake::{Pake, Role, Spake};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::future::Future;
use std::io;
use std::sync::Mutex;
use std::time::Duration;
use subtle::ConstantTimeEq;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub const VERSION: u8 = 1;
/// How long a code or link works, in seconds.
pub const OFFER_TTL: u64 = 600;
/// Wrong proofs before a code or link is dead.
pub const MAX_FAILS: u32 = 5;
/// Codes and links alive at once; making another drops the oldest.
pub const MAX_OFFERS: usize = 8;
/// The longest frame body either side reads.
pub const FRAME_MAX: usize = 2048;
/// How long the frames of one exchange may take in all, not counting the host's `mint`.
pub const EXCHANGE_TIMEOUT: Duration = Duration::from_secs(30);
pub const OS_MAX: usize = 16;
pub const TOKEN_MAX: usize = 256;
const HINT_LEN: usize = 8;

const HELLO: u8 = 1;
const REPLY: u8 = 2;
const CONFIRM_J: u8 = 3;
const CONFIRM_H: u8 = 4;
const ABOUT: u8 = 5;
const WELCOME: u8 = 6;
const ABORT: u8 = 0x7f;

/// Who is at each end of the stream and what it runs over. Both sides must fill it in from what they see
/// themselves, never from what the other side says.
#[derive(Debug, Clone, Copy)]
pub struct Binding<'a> {
    /// The id the joiner dialed and the host answers as: home's id for a link, the code's rendezvous id for a
    /// code.
    pub host: [u8; 32],
    /// The joiner's id.
    pub joiner: [u8; 32],
    /// A value both ends of the channel under the stream share and nobody else can make, such as the TLS
    /// exporter. Empty only where the channel needs no binding (tests).
    pub channel: &'a [u8],
}

/// What the joiner holds: a code someone typed, or the secret from a link.
#[derive(Clone, PartialEq, Eq)]
pub enum Secret {
    Code(ShortCode),
    Link([u8; SECRET_LEN]),
}

impl Secret {
    fn password(&self) -> Vec<u8> {
        match self {
            Secret::Code(c) => [b"code:", c.as_str().as_bytes()].concat(),
            Secret::Link(s) => [b"link:".as_slice(), s].concat(),
        }
    }

    /// What tells home which of its live links this is, sent in the clear. A link's secret is 128 random bits,
    /// so a hash of it gives nothing away. A code has no hint at all: it is short enough to be found from any
    /// hash of it, and home knows which code a joiner means by the rendezvous id it dialed.
    fn hint(&self) -> Vec<u8> {
        match self {
            Secret::Code(_) => vec![],
            Secret::Link(s) => blake3::derive_key("switchboard pair link hint 1", s)[..HINT_LEN].to_vec(),
        }
    }

    pub fn via(&self) -> Via {
        match self {
            Secret::Code(_) => Via::Code,
            Secret::Link(_) => Via::Link,
        }
    }
}

impl From<&PairLink> for Secret {
    fn from(l: &PairLink) -> Secret {
        Secret::Link(l.secret)
    }
}

impl From<ShortCode> for Secret {
    fn from(c: ShortCode) -> Secret {
        Secret::Code(c)
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Secret::Code(_) => "Secret::Code(..)",
            Secret::Link(_) => "Secret::Link(..)",
        })
    }
}

/// What the joiner tells home about itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct About {
    pub name: String,
    pub kind: Kind,
    #[serde(default)]
    pub os: String,
    /// The home computer this device is paired with already and means to stay with. A home computer that is
    /// not that one turns the device down before it makes a place for it (`PairError::OtherHome`), and the code
    /// or link stays good, so the person can say to replace the old home and try again. None: the device has
    /// no home, or was told to replace it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stays_with: Option<DeviceId>,
}

impl About {
    fn cleaned(self) -> About {
        About {
            name: crate::clean(&self.name, NAME_MAX),
            kind: self.kind,
            os: crate::clean(&self.os, OS_MAX),
            stays_with: self.stays_with,
        }
    }
}

/// What home is, for the WELCOME message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Home {
    /// Home's own id (not the rendezvous id).
    pub id: DeviceId,
    pub name: String,
    pub relay: Option<String>,
}

/// What `mint` made for the device: its name at the hub and the token for its link. Neither for a phone.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Grant {
    pub machine: Option<String>,
    pub token: Option<String>,
}

impl fmt::Debug for Grant {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Grant(machine {:?}, token {})", self.machine, if self.token.is_some() { ".." } else { "none" })
    }
}

/// What the joiner learns: who home is, and its place at the hub.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Welcome {
    pub home_id: DeviceId,
    #[serde(default)]
    pub home_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relay: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub machine: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
}

impl fmt::Debug for Welcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Welcome(home {} {:?}, relay {:?}, machine {:?}, token {})",
            self.home_id.short(),
            self.home_name,
            self.relay,
            self.machine,
            if self.token.is_some() { ".." } else { "none" }
        )
    }
}

impl Welcome {
    /// A WELCOME is from a home that proved it holds the secret, and is still checked like anything read off a
    /// wire: what it holds goes into files and addresses.
    fn checked(self) -> Result<Welcome, PairError> {
        let ok = self.home_id.is_key()
            && self.relay.as_deref().is_none_or(valid_relay)
            && self.machine.as_deref().is_none_or(crate::valid_machine)
            && self.token.as_deref().is_none_or(valid_token);
        if !ok {
            return Err(PairError::Protocol("home's answer has a field that cannot be right"));
        }
        Ok(Welcome { home_name: crate::clean(&self.home_name, NAME_MAX), ..self })
    }
}

fn valid_token(t: &str) -> bool {
    !t.is_empty() && t.len() <= TOKEN_MAX && t.bytes().all(|b| b.is_ascii_graphic())
}

/// The device the host just paired with: what goes into the paired list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Joined {
    pub id: DeviceId,
    pub about: About,
    pub machine: Option<String>,
    pub via: Via,
}

/// What the host's `mint` is asked: the device proved it holds the secret, and this is what it says it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub id: DeviceId,
    pub about: About,
    pub via: Via,
}

#[derive(Debug)]
pub enum PairError {
    /// The two sides did not hold the same secret, or were not on the same channel.
    WrongCode,
    /// The code or link is unknown, expired, used or dead.
    Gone,
    /// Someone else is pairing with this code right now.
    Busy,
    /// Home could not make a place for the device (`mint` failed). The code still works.
    Refused(String),
    /// A link's home id is not who answered.
    WrongHome,
    /// The device is paired with another home computer and did not say to replace it (`About::stays_with`).
    /// Nothing was changed on either side, and the code or link still works.
    OtherHome,
    /// The other side speaks a version this one does not.
    Version(u8),
    /// A frame that breaks the rules: too long, out of order, not what it claims to be.
    Protocol(&'static str),
    Timeout,
    Io(io::Error),
    /// Host only: the device was given a place (`mint` ran, the code is used up) and the answer could not be
    /// sent. The caller should take that place away again.
    Undelivered {
        machine: Option<String>,
        error: io::Error,
    },
}

impl PairError {
    /// What to tell the person pairing.
    pub fn message(&self) -> String {
        match self {
            PairError::WrongCode => "That code is wrong. Check it and try again.".into(),
            PairError::Gone => "That code has expired or was already used. Make a new one.".into(),
            PairError::Busy => "Another device is pairing with that code right now. Try again in a moment.".into(),
            PairError::Refused(why) if why.is_empty() => "The other computer could not add this device.".into(),
            PairError::Refused(why) => format!("The other computer could not add this device ({why})."),
            PairError::WrongHome => "The computer that answered is not the one the link names.".into(),
            PairError::OtherHome => "This device is paired with another home computer already, so nothing was \
                                     changed. To pair it with this one instead, say to replace the old one."
                .into(),
            PairError::Version(v) => format!(
                "The two computers pair in different ways (this one uses version {VERSION}, the other {v}). Update \
                 Switchboard on both."
            ),
            PairError::Protocol(what) => format!("Pairing stopped: {what}."),
            PairError::Timeout => "Pairing took too long. Try again.".into(),
            PairError::Io(e) => format!("The connection was lost while pairing ({e})."),
            PairError::Undelivered { error, .. } => {
                format!("The device was added, and the answer never reached it ({error}).")
            }
        }
    }

    /// The same, as one word a program can act on (the sentence is for the person and may change).
    pub fn kind(&self) -> &'static str {
        match self {
            PairError::WrongCode => "wrong_code",
            // unknown, run out, used already, or stopped after too many wrong tries: home does not say which
            PairError::Gone => "expired",
            PairError::Busy => "busy",
            PairError::Refused(_) => "refused",
            PairError::WrongHome => "wrong_home",
            PairError::OtherHome => "other_home",
            PairError::Version(_) => "version",
            PairError::Protocol(_) => "protocol",
            PairError::Timeout => "timeout",
            PairError::Io(_) => "connection_lost",
            PairError::Undelivered { .. } => "undelivered",
        }
    }

    fn abort_reason(&self) -> u8 {
        match self {
            PairError::WrongCode => 1,
            PairError::Gone => 2,
            PairError::Busy => 3,
            PairError::Refused(_) => 4,
            PairError::Version(_) => 6,
            PairError::OtherHome => 7,
            _ => 5,
        }
    }

    fn from_abort(body: &[u8]) -> PairError {
        match body.first() {
            Some(1) => PairError::WrongCode,
            Some(2) => PairError::Gone,
            Some(3) => PairError::Busy,
            Some(4) => PairError::Refused(String::new()),
            Some(6) => PairError::Version(body.get(1).copied().unwrap_or(0)),
            Some(7) => PairError::OtherHome,
            _ => PairError::Protocol("the other side stopped the exchange"),
        }
    }
}

impl fmt::Display for PairError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message())
    }
}

impl std::error::Error for PairError {}

impl From<io::Error> for PairError {
    fn from(e: io::Error) -> PairError {
        PairError::Io(e)
    }
}

// ---- the codes and links home has given out -------------------------------------------------------------------

struct Offer {
    serial: u64,
    secret: Secret,
    expires: u64,
    fails: u32,
    busy: bool,
}

#[derive(Default)]
struct OfferList {
    live: Vec<Offer>,
    serial: u64,
}

/// The codes and links home has given out and not yet seen used. Kept in memory only: a restart ends them all.
#[derive(Default)]
pub struct Offers {
    inner: Mutex<OfferList>,
}

/// Which offer the host answers for.
#[derive(Debug, Clone, Copy)]
pub enum Select<'a> {
    /// The joiner dialed this code's rendezvous id.
    Code(&'a ShortCode),
    /// The joiner dialed home's own id: the link is found by the hint in its HELLO.
    Link,
}

impl Offers {
    pub fn new() -> Offers {
        Offers::default()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, OfferList> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn add(&self, secret: Secret, now: u64) -> u64 {
        let mut l = self.lock();
        l.live.retain(|o| o.expires > now);
        while l.live.len() >= MAX_OFFERS {
            l.live.remove(0); // the oldest
        }
        l.serial += 1;
        let (serial, expires) = (l.serial, now + OFFER_TTL);
        l.live.push(Offer { serial, secret, expires, fails: 0, busy: false });
        expires
    }

    /// A new short code and when it stops working (seconds since 1970).
    pub fn mint_code(&self, now: u64) -> (ShortCode, u64) {
        let code = loop {
            let c = ShortCode::generate();
            if !self.lock().live.iter().any(|o| o.secret == Secret::Code(c.clone())) {
                break c;
            }
        };
        let expires = self.add(Secret::Code(code.clone()), now);
        (code, expires)
    }

    /// A new secret for a link (`PairLink`) and when it stops working.
    pub fn mint_link(&self, now: u64) -> ([u8; SECRET_LEN], u64) {
        let secret = crate::random::<SECRET_LEN>();
        (secret, self.add(Secret::Link(secret), now))
    }

    /// Ends a code or link before its time. False when it was not live.
    pub fn revoke(&self, secret: &Secret) -> bool {
        let mut l = self.lock();
        let before = l.live.len();
        l.live.retain(|o| o.secret != *secret);
        l.live.len() != before
    }

    /// The codes that still work at `now`: the ones home has to keep listening for.
    pub fn codes(&self, now: u64) -> Vec<ShortCode> {
        let mut l = self.lock();
        l.live.retain(|o| o.expires > now);
        l.live.iter().filter_map(|o| if let Secret::Code(c) = &o.secret { Some(c.clone()) } else { None }).collect()
    }

    /// How many codes and links still work at `now`.
    pub fn live(&self, now: u64) -> usize {
        let mut l = self.lock();
        l.live.retain(|o| o.expires > now);
        l.live.len()
    }

    /// Wrong proofs so far against a live code or link.
    pub fn fails(&self, secret: &Secret) -> Option<u32> {
        self.lock().live.iter().find(|o| o.secret == *secret).map(|o| o.fails)
    }

    /// Takes the offer a joiner is trying, for one exchange at a time.
    fn begin(&self, select: Select<'_>, hint: &[u8], now: u64) -> Result<Lease<'_>, PairError> {
        let mut l = self.lock();
        l.live.retain(|o| o.expires > now);
        let mut found = None;
        for (i, o) in l.live.iter().enumerate() {
            let hit = match (&o.secret, select) {
                (Secret::Code(c), Select::Code(want)) => hint.is_empty() && c == want,
                // every link is compared, in the same time whether or not it is the one
                (s @ Secret::Link(_), Select::Link) => bool::from(s.hint().ct_eq(hint)),
                _ => false,
            };
            if hit {
                found = Some(i);
            }
        }
        let Some(found) = found else { return Err(PairError::Gone) };
        let offer = &mut l.live[found];
        if offer.busy {
            return Err(PairError::Busy);
        }
        offer.busy = true;
        Ok(Lease { offers: self, serial: offer.serial, secret: offer.secret.clone(), settled: false })
    }
}

/// One exchange's hold on an offer. Dropped without a verdict (the stream broke, the exchange timed out), the
/// offer is simply free again: nothing was learned by anyone.
struct Lease<'a> {
    offers: &'a Offers,
    serial: u64,
    secret: Secret,
    settled: bool,
}

impl Lease<'_> {
    /// The joiner's proof was wrong: one guess spent. The fifth ends the offer.
    fn fail(mut self) {
        self.settled = true;
        let mut l = self.offers.lock();
        if let Some(o) = l.live.iter_mut().find(|o| o.serial == self.serial) {
            o.busy = false;
            o.fails += 1;
        }
        l.live.retain(|o| o.fails < MAX_FAILS);
    }

    /// Paired: the offer is used up.
    fn consume(mut self) {
        self.settled = true;
        self.offers.lock().live.retain(|o| o.serial != self.serial);
    }
}

impl Drop for Lease<'_> {
    fn drop(&mut self) {
        if !self.settled
            && let Some(o) = self.offers.lock().live.iter_mut().find(|o| o.serial == self.serial)
        {
            o.busy = false;
        }
    }
}

// ---- frames -------------------------------------------------------------------------------------------------

async fn write_frame<W: AsyncWrite + Unpin>(w: &mut W, kind: u8, body: &[u8]) -> io::Result<()> {
    if body.len() > FRAME_MAX {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "a pairing frame over the limit"));
    }
    let mut frame = Vec::with_capacity(3 + body.len());
    frame.extend_from_slice(&(body.len() as u16 + 1).to_be_bytes());
    frame.push(kind);
    frame.extend_from_slice(body);
    w.write_all(&frame).await?;
    w.flush().await
}

/// One frame: its type and body. The length is checked before anything is set aside for the body.
async fn read_frame<R: AsyncRead + Unpin>(r: &mut R) -> Result<(u8, Vec<u8>), PairError> {
    let mut head = [0u8; 3];
    r.read_exact(&mut head).await?;
    let len = usize::from(u16::from_be_bytes([head[0], head[1]]));
    if len == 0 {
        return Err(PairError::Protocol("an empty frame"));
    }
    if len - 1 > FRAME_MAX {
        return Err(PairError::Protocol("a frame over the limit"));
    }
    let mut body = vec![0u8; len - 1];
    r.read_exact(&mut body).await?;
    Ok((head[2], body))
}

/// The next frame, which must be a `want`. An ABORT becomes the error it names.
async fn expect<R: AsyncRead + Unpin>(r: &mut R, want: u8) -> Result<Vec<u8>, PairError> {
    match read_frame(r).await? {
        (kind, body) if kind == want => Ok(body),
        (ABORT, body) => Err(PairError::from_abort(&body)),
        _ => Err(PairError::Protocol("a frame out of order")),
    }
}

/// Tells the other side why the exchange is over. Best effort: it may already be gone.
async fn abort<W: AsyncWrite + Unpin>(w: &mut W, e: &PairError) {
    let mut body = vec![e.abort_reason()];
    if let PairError::Version(_) = e {
        body.push(VERSION);
    }
    let _ = write_frame(w, ABORT, &body).await;
}

// ---- keys ---------------------------------------------------------------------------------------------------

/// Everything both sides saw, each part with its length so no two sets of parts hash alike.
fn transcript(b: &Binding<'_>, hint: &[u8], msg_j: &[u8], msg_h: &[u8]) -> [u8; 32] {
    let mut h = blake3::Hasher::new_derive_key("switchboard pair 1 transcript");
    for part in [&[VERSION][..], &b.host[..], &b.joiner[..], b.channel, hint, msg_j, msg_h] {
        h.update(&(part.len() as u64).to_le_bytes());
        h.update(part);
    }
    *h.finalize().as_bytes()
}

struct Keys {
    confirm_j: [u8; 32],
    confirm_h: [u8; 32],
    seal_j: [u8; 32],
    seal_h: [u8; 32],
}

impl Keys {
    fn derive(shared: &[u8], transcript: &[u8; 32]) -> Keys {
        let mut h = blake3::Hasher::new_derive_key("switchboard pair 1 master");
        h.update(&(shared.len() as u64).to_le_bytes());
        h.update(shared);
        h.update(transcript);
        let master = *h.finalize().as_bytes();
        let sub = |label: &[u8]| *blake3::keyed_hash(&master, label).as_bytes();
        Keys {
            confirm_j: sub(b"confirm joiner"),
            confirm_h: sub(b"confirm host"),
            seal_j: sub(b"seal joiner"),
            seal_h: sub(b"seal host"),
        }
    }
}

/// ChaCha20-Poly1305 under a key used for exactly one message, so the nonce is fixed. The frame type is bound
/// in, so one sealed message cannot stand in for the other.
fn seal(key: &[u8; 32], kind: u8, plain: &[u8]) -> Result<Vec<u8>, PairError> {
    ChaCha20Poly1305::new(Key::from_slice(key))
        .encrypt(Nonce::from_slice(&[0u8; 12]), Payload { msg: plain, aad: &[kind] })
        .map_err(|_| PairError::Protocol("a message could not be sealed"))
}

fn open(key: &[u8; 32], kind: u8, sealed: &[u8]) -> Result<Vec<u8>, PairError> {
    ChaCha20Poly1305::new(Key::from_slice(key))
        .decrypt(Nonce::from_slice(&[0u8; 12]), Payload { msg: sealed, aad: &[kind] })
        .map_err(|_| PairError::Protocol("a message was not sealed with this exchange's key"))
}

fn same(a: &[u8], b: &[u8]) -> bool {
    bool::from(a.ct_eq(b))
}

// ---- the joiner ---------------------------------------------------------------------------------------------

/// The joining device's side. Ok is home's answer: store `home_id`, `relay` and the token, then connect.
///
/// `binding.host` is the id that was dialed. With a link it must be the link's home id, and the answer is
/// refused unless it names the same id.
pub async fn join<S>(
    stream: &mut S,
    secret: &Secret,
    binding: &Binding<'_>,
    about: &About,
) -> Result<Welcome, PairError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    match tokio::time::timeout(EXCHANGE_TIMEOUT, join_with::<S, Spake>(stream, secret, binding, about)).await {
        Ok(r) => r,
        Err(_) => Err(PairError::Timeout),
    }
}

async fn join_with<S, P>(
    stream: &mut S,
    secret: &Secret,
    binding: &Binding<'_>,
    about: &About,
) -> Result<Welcome, PairError>
where
    S: AsyncRead + AsyncWrite + Unpin,
    P: Pake,
{
    let hint = secret.hint();
    let (pake, msg_j) = P::start(Role::Joiner, &secret.password(), &binding.joiner, &binding.host);
    let mut hello = vec![VERSION, hint.len() as u8];
    hello.extend_from_slice(&hint);
    hello.extend_from_slice(&msg_j);
    write_frame(stream, HELLO, &hello).await?;

    let msg_h = expect(stream, REPLY).await?;
    let shared = pake.finish(&msg_h).ok_or(PairError::Protocol("a key-exchange message that is not one"))?;
    let keys = Keys::derive(&shared, &transcript(binding, &hint, &msg_j, &msg_h));
    write_frame(stream, CONFIRM_J, &keys.confirm_j).await?;

    let proof = expect(stream, CONFIRM_H).await?;
    if !same(&proof, &keys.confirm_h) {
        return Err(PairError::WrongCode);
    }
    let body = serde_json::to_vec(&about.clone().cleaned()).map_err(io::Error::other)?;
    write_frame(stream, ABOUT, &seal(&keys.seal_j, ABOUT, &body)?).await?;

    let sealed = expect(stream, WELCOME).await?;
    let plain = open(&keys.seal_h, WELCOME, &sealed)?;
    let welcome = serde_json::from_slice::<Welcome>(&plain)
        .map_err(|_| PairError::Protocol("home's answer could not be read"))?
        .checked()?;
    if matches!(secret, Secret::Link(_)) && welcome.home_id.0 != binding.host {
        return Err(PairError::WrongHome);
    }
    Ok(welcome)
}

// ---- the host -----------------------------------------------------------------------------------------------

/// Home's side, for one joiner on one stream. Ok is the device to add to the paired list.
///
/// - `select` says which offer this stream is for (see `Select`).
/// - `binding` is filled in from the connection: the id this endpoint answered as, and the joiner's id.
/// - `now` is the time in seconds since 1970, for expiry.
/// - `mint` runs once the joiner has proved it holds the secret, and makes its place (at the hub: a machine name
///   and a token). Err from it is sent on as "could not add this device", and the code can be tried again.
///
/// A wrong proof counts against the code. A stream that breaks or goes quiet does not: nothing was shown to it.
pub async fn host<S, F, Fut>(
    stream: &mut S,
    offers: &Offers,
    select: Select<'_>,
    binding: &Binding<'_>,
    home: &Home,
    now: u64,
    mint: F,
) -> Result<Joined, PairError>
where
    S: AsyncRead + AsyncWrite + Unpin,
    F: FnOnce(Request) -> Fut,
    Fut: Future<Output = Result<Grant, String>>,
{
    host_with::<S, Spake, F, Fut>(stream, offers, select, binding, home, now, mint).await
}

async fn host_with<S, P, F, Fut>(
    stream: &mut S,
    offers: &Offers,
    select: Select<'_>,
    binding: &Binding<'_>,
    home: &Home,
    now: u64,
    mint: F,
) -> Result<Joined, PairError>
where
    S: AsyncRead + AsyncWrite + Unpin,
    P: Pake,
    F: FnOnce(Request) -> Fut,
    Fut: Future<Output = Result<Grant, String>>,
{
    // Up to the joiner's ABOUT, under one deadline.
    let proved = tokio::time::timeout(EXCHANGE_TIMEOUT, async {
        let hello = expect(stream, HELLO).await?;
        let (hint, msg_j) = match hello.as_slice() {
            [v, ..] if *v != VERSION => return Err(PairError::Version(*v)),
            [_, n, rest @ ..] if matches!(usize::from(*n), 0 | HINT_LEN) && rest.len() > usize::from(*n) => {
                rest.split_at(usize::from(*n))
            }
            _ => return Err(PairError::Protocol("a first frame that cannot be read")),
        };
        let lease = offers.begin(select, hint, now)?;
        let (pake, msg_h) = P::start(Role::Host, &lease.secret.password(), &binding.joiner, &binding.host);
        let shared = pake.finish(msg_j).ok_or(PairError::Protocol("a key-exchange message that is not one"))?;
        let keys = Keys::derive(&shared, &transcript(binding, hint, msg_j, &msg_h));
        write_frame(stream, REPLY, &msg_h).await?;

        let proof = expect(stream, CONFIRM_J).await?;
        if !same(&proof, &keys.confirm_j) {
            lease.fail();
            return Err(PairError::WrongCode);
        }
        write_frame(stream, CONFIRM_H, &keys.confirm_h).await?;

        let sealed = expect(stream, ABOUT).await?;
        let plain = open(&keys.seal_j, ABOUT, &sealed)?;
        let about = serde_json::from_slice::<About>(&plain)
            .map_err(|_| PairError::Protocol("the device's description could not be read"))?
            .cleaned();
        if about.stays_with.is_some_and(|other| other != home.id) {
            // It proved it holds the secret, and belongs to another home computer: it is told so before a
            // place is made for it. Not a wrong try: the lease is let go, and the same code works once the
            // person has said to replace the old home.
            return Err(PairError::OtherHome);
        }
        Ok((lease, keys, about))
    })
    .await
    .unwrap_or(Err(PairError::Timeout));
    let (lease, keys, about) = match proved {
        Ok(p) => p,
        Err(e) => {
            if !matches!(e, PairError::Io(_) | PairError::Timeout) {
                abort(stream, &e).await;
            }
            return Err(e);
        }
    };

    let id = DeviceId(binding.joiner);
    let via = lease.secret.via();
    let grant = match mint(Request { id, about: about.clone(), via }).await {
        Ok(g) => g,
        Err(why) => {
            // not the joiner's doing: the code is free to be tried again
            drop(lease);
            let e = PairError::Refused(crate::clean(&why, 200));
            abort(stream, &e).await;
            return Err(e);
        }
    };
    lease.consume();

    let welcome = Welcome {
        home_id: home.id,
        home_name: crate::clean(&home.name, NAME_MAX),
        relay: home.relay.clone(),
        machine: grant.machine.clone(),
        token: grant.token,
    };
    let sent = async {
        let body = serde_json::to_vec(&welcome).map_err(io::Error::other)?;
        let sealed = seal(&keys.seal_h, WELCOME, &body).map_err(|e| io::Error::other(e.message()))?;
        match tokio::time::timeout(EXCHANGE_TIMEOUT, write_frame(stream, WELCOME, &sealed)).await {
            Ok(r) => r,
            Err(_) => Err(io::Error::new(io::ErrorKind::TimedOut, "the device stopped reading")),
        }
    }
    .await;
    match sent {
        Ok(()) => Ok(Joined { id, about, machine: grant.machine, via }),
        Err(error) => Err(PairError::Undelivered { machine: grant.machine, error }),
    }
}

#[cfg(test)]
mod tests;
