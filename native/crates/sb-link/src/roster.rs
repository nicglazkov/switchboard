//! The roster: the home computer's list of paired devices, as a statement only the home computer can make.
//!
//! Home signs the list with its own key, and every paired device keeps the newest one it has seen. A computer
//! that is not home lets a device reach its own chats when that device is on the roster it holds, so the devices
//! can still find each other while the home computer is away.
//!
//! ```text
//! envelope: {"v":1,"body":"<base64url, no padding>","sig":"<base64url, 64 bytes>"}
//! sig      = Ed25519(home key, "switchboard roster 1\n" || body bytes)
//! body     = {"home","home_name","epoch","seq","at","relay","devices":[{"id","name","kind","machine"?,"ver"?,"own"?,"addrs"}]}
//! ```
//!
//! The rules, each with a test below:
//!
//! - **Only home signs.** A roster is taken when its signature verifies under the home id this device pinned
//!   when it paired, and its body names that same home. The bytes are checked as they were sent.
//! - **Newer wins, for good.** Higher `(epoch, seq)` wins, then higher `at`. A device keeps only the best it
//!   has seen (`RosterCache`), in a store whose copy always matches the last save, so falling back to the copy
//!   never brings an older roster (and with it a removed device) back.
//! - **Not from the future.** An `at` more than ten minutes ahead of this device's clock is refused.
//! - **Not for ever.** A roster is honoured for seven days after home signed it. After that it lets nobody in.
//! - **A local block** refuses one device on this computer only, until home changes the list (`Blocks`).
//!
//! Nothing here names the network library, and nothing assumes a computer: a phone keeps the same cache.

use crate::identity::{DeviceId, DeviceKey};
use crate::paired::Kind;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use ed25519_dalek::{Signer, Verifier};
use sb_core::durable::{Bak, Status, Store};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

/// The envelope format this version reads and writes.
pub const VERSION: u32 = 1;
/// The most a body may be, in bytes.
pub const MAX_BYTES: usize = 64 * 1024;
pub const MAX_DEVICES: usize = 64;
pub const MAX_ADDRS: usize = 4;
/// How long after home signed it a roster is honoured, in seconds: seven days.
pub const HONOURED: u64 = 7 * 24 * 3600;
/// How far ahead of this device's clock `at` may be, in seconds.
pub const AHEAD_MAX: u64 = 600;
/// Home signs again after this long with nothing changed, so a roster in use never gets near `HONOURED`.
pub const RESIGN_EVERY: u64 = 6 * 3600;

const SIGNED_AS: &[u8] = b"switchboard roster 1\n";
const NAME_MAX: usize = 64;

fn unix_now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Which roster this is. Ordered as the rule says: epoch, then seq, then at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Stamp {
    pub epoch: u64,
    pub seq: u64,
    pub at: u64,
}

/// One paired device, as home lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RosterDevice {
    pub id: DeviceId,
    #[serde(default)]
    pub name: String,
    pub kind: Kind,
    /// The hub's name for the computer, when it is one that runs chats.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub machine: Option<String>,
    /// The version of its helper, when home knows it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ver: Option<String>,
    /// Whether its helper answers for its own chats while home is away, as home found out when it sent the
    /// list there: true when the helper took it, false when the helper did not know what a list is (it needs
    /// an update). None: home has not been able to ask yet, or this is not a computer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub own: Option<bool>,
    /// Where it was last reached directly.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub addrs: Vec<SocketAddr>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Body {
    pub home: DeviceId,
    #[serde(default)]
    pub home_name: String,
    pub epoch: u64,
    pub seq: u64,
    /// When home signed it, in seconds since 1970.
    pub at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relay: Option<String>,
    #[serde(default)]
    pub devices: Vec<RosterDevice>,
}

/// A roster as it travels and is stored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Envelope {
    pub v: u32,
    pub body: String,
    pub sig: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RosterError {
    /// A format this version does not read.
    Version(u32),
    /// Not base64, not JSON, or not the fields a roster has.
    Encoding,
    TooLarge,
    TooManyDevices,
    /// The signature is not the pinned home's over these bytes.
    Signature,
    /// Signed, but the body names another home.
    WrongHome,
    /// `at` is too far ahead of this device's clock.
    Ahead,
}

impl RosterError {
    /// One word for a program.
    pub fn kind(&self) -> &'static str {
        match self {
            RosterError::Version(_) => "version",
            RosterError::Encoding | RosterError::TooLarge | RosterError::TooManyDevices => "bad_request",
            RosterError::Signature | RosterError::WrongHome => "not_from_home",
            RosterError::Ahead => "ahead",
        }
    }
}

impl fmt::Display for RosterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RosterError::Version(v) => write!(f, "The list of devices is in a newer format ({v}). Update this device."),
            RosterError::Encoding => f.write_str("The list of devices could not be read."),
            RosterError::TooLarge => f.write_str("The list of devices is too large."),
            RosterError::TooManyDevices => f.write_str("The list of devices names too many devices."),
            RosterError::Signature | RosterError::WrongHome => {
                f.write_str("The list of devices was not signed by this device's home computer.")
            }
            RosterError::Ahead => f.write_str("The list of devices is dated in the future. Check this device's clock."),
        }
    }
}

impl std::error::Error for RosterError {}

/// A roster that has been checked: signed by the home it names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Roster {
    pub body: Body,
    pub envelope: Envelope,
}

impl Roster {
    pub fn stamp(&self) -> Stamp {
        Stamp { epoch: self.body.epoch, seq: self.body.seq, at: self.body.at }
    }

    /// When it stops being honoured.
    pub fn expires(&self) -> u64 {
        self.body.at.saturating_add(HONOURED)
    }

    /// Whether it is too old to let anyone in. The clock used is the later of `now` and the roster's own `at`,
    /// so a clock that was set back cannot make an old roster young.
    pub fn stale(&self, now: u64) -> bool {
        now.max(self.body.at) > self.expires()
    }

    pub fn device(&self, id: &DeviceId) -> Option<&RosterDevice> {
        self.body.devices.iter().find(|d| d.id == *id)
    }

    pub fn lists(&self, id: &DeviceId) -> bool {
        self.device(id).is_some()
    }
}

fn limits(body: &Body, bytes: usize) -> Result<(), RosterError> {
    if bytes > MAX_BYTES {
        return Err(RosterError::TooLarge);
    }
    if body.devices.len() > MAX_DEVICES {
        return Err(RosterError::TooManyDevices);
    }
    Ok(())
}

/// What home writes into a body: names cleaned, machine names the hub would take, at most four addresses each.
fn tidy(mut body: Body) -> Body {
    body.home_name = crate::clean(&body.home_name, NAME_MAX);
    for d in &mut body.devices {
        d.name = crate::clean(&d.name, NAME_MAX);
        d.machine = d.machine.take().filter(|m| crate::valid_machine(m));
        d.ver = d.ver.take().map(|v| crate::clean(&v, 32)).filter(|v| !v.is_empty());
        d.addrs.truncate(MAX_ADDRS);
    }
    body
}

/// Signs `body` with home's key. Only the home computer calls this; `body.home` is set to the key's id whatever
/// it said.
pub fn sign(key: &DeviceKey, body: Body) -> Result<Roster, RosterError> {
    let body = tidy(Body { home: key.id(), ..body });
    let bytes = serde_json::to_vec(&body).map_err(|_| RosterError::Encoding)?;
    limits(&body, bytes.len())?;
    let signing = ed25519_dalek::SigningKey::from_bytes(key.secret_bytes());
    let sig = signing.sign(&[SIGNED_AS, &bytes].concat());
    let envelope = Envelope { v: VERSION, body: B64.encode(&bytes), sig: B64.encode(sig.to_bytes()) };
    Ok(Roster { body, envelope })
}

/// Checks an envelope against the home this device pinned. `now` is this device's clock; None skips the check
/// that `at` is not in the future (for a roster read back from this device's own store).
pub fn verify(envelope: &Envelope, home: &DeviceId, now: Option<u64>) -> Result<Roster, RosterError> {
    if envelope.v != VERSION {
        return Err(RosterError::Version(envelope.v));
    }
    if envelope.body.len() > MAX_BYTES * 4 / 3 + 4 {
        return Err(RosterError::TooLarge);
    }
    let bytes = B64.decode(&envelope.body).map_err(|_| RosterError::Encoding)?;
    let sig: [u8; 64] = B64.decode(&envelope.sig).ok().and_then(|s| s.try_into().ok()).ok_or(RosterError::Encoding)?;
    let key = ed25519_dalek::VerifyingKey::from_bytes(&home.0).map_err(|_| RosterError::Signature)?;
    key.verify(&[SIGNED_AS, &bytes].concat(), &ed25519_dalek::Signature::from_bytes(&sig))
        .map_err(|_| RosterError::Signature)?;
    // (only now is the body looked at: nothing unsigned is parsed)
    let body: Body = serde_json::from_slice(&bytes).map_err(|_| RosterError::Encoding)?;
    if body.home != *home {
        return Err(RosterError::WrongHome);
    }
    limits(&body, bytes.len())?;
    if now.is_some_and(|now| body.at > now.saturating_add(AHEAD_MAX)) {
        return Err(RosterError::Ahead);
    }
    Ok(Roster { body: tidy(body), envelope: envelope.clone() })
}

// ---- the cache -------------------------------------------------------------------------------------------------

#[derive(Serialize, Deserialize)]
struct CacheFile {
    v: u32,
    roster: Envelope,
}

#[derive(Default)]
struct Held {
    roster: Option<Arc<Roster>>,
    note: Option<String>,
}

/// The best roster this device has seen, kept on disk. One file, `roster.json` (mode 600), with a copy that
/// always matches the last save.
pub struct RosterCache {
    store: Store,
    home: DeviceId,
    held: Mutex<Held>,
}

impl RosterCache {
    /// The cache in `path` for a device paired with `home`. Never fails: a file that cannot be read, or that
    /// holds a roster `home` did not sign, is no roster, and `trouble` says why.
    pub fn load(path: &Path, home: DeviceId) -> RosterCache {
        let store = Store::new(path).private().bak(Bak::Fresh);
        let shown = store.path().display().to_string();
        let mut held = Held::default();
        match store.load_json::<CacheFile>() {
            (Some(f), status) => match verify(&f.roster, &home, None) {
                Ok(r) => {
                    held.roster = Some(Arc::new(r));
                    if status == Status::Recovered {
                        held.note = Some(format!("{shown} could not be read: its last good copy is in use."));
                    }
                }
                Err(e) => {
                    held.note = Some(format!("{shown} does not hold a list this device's home computer signed ({e})."))
                }
            },
            (None, Status::Broken) => {
                held.note = Some(format!(
                    "{shown} could not be read and has no good copy. No device is let in until the home computer \
                     sends its list again."
                ))
            }
            (None, _) => {}
        }
        RosterCache { store, home, held: Mutex::new(held) }
    }

    fn held(&self) -> MutexGuard<'_, Held> {
        self.held.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn home(&self) -> DeviceId {
        self.home
    }

    /// Something the user should be told about the file, once.
    pub fn trouble(&self) -> Option<String> {
        self.held().note.take()
    }

    pub fn get(&self) -> Option<Arc<Roster>> {
        self.held().roster.clone()
    }

    pub fn stamp(&self) -> Option<Stamp> {
        self.held().roster.as_ref().map(|r| r.stamp())
    }

    /// Takes `envelope` if it is a roster this device's home signed and it is newer than the one held.
    /// Ok(true): it was taken, and is on disk. Ok(false): the one held is as new or newer, and stays.
    /// Err: it is not a roster from home; the one held stays.
    pub fn offer(&self, envelope: &Envelope, now: u64) -> Result<bool, RosterError> {
        let next = verify(envelope, &self.home, Some(now))?;
        let mut held = self.held();
        if held.roster.as_ref().is_some_and(|have| next.stamp() <= have.stamp()) {
            return Ok(false);
        }
        // Kept in memory even when the disk refuses it: a newer list must not be ignored because a file would
        // not save. `trouble` says so, and the next one taken tries again.
        if let Err(e) = self.store.put_json(&CacheFile { v: VERSION, roster: next.envelope.clone() }, true) {
            held.note = Some(format!(
                "{} could not be saved ({e}). The newer list of devices is in use until this program stops.",
                self.store.path().display()
            ));
        }
        held.roster = Some(Arc::new(next));
        Ok(true)
    }
}

// ---- local blocks ----------------------------------------------------------------------------------------------

/// A device this computer refuses although the roster lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Block {
    pub id: DeviceId,
    #[serde(default)]
    pub name: String,
    /// The roster held when the block was made. The block ends when home changes the list: a higher
    /// `(epoch, seq)`. Home signing the same list again (a new `at`) does not end it.
    pub epoch: u64,
    pub seq: u64,
    /// When, in seconds since 1970.
    #[serde(default)]
    pub at: u64,
}

#[derive(Serialize, Deserialize)]
struct BlockFile {
    v: u32,
    #[serde(default)]
    blocked: Vec<Block>,
}

/// The devices blocked on this computer: `blocked.json` (mode 600). Blocks are local. They are never sent to
/// another device, so one stolen computer cannot lock the others out while the home computer is away.
pub struct Blocks {
    store: Store,
    list: Mutex<Vec<Block>>,
    /// The file was there and could not be read: every roster device is refused until it reads, because one of
    /// them may be blocked.
    unread: bool,
}

impl Blocks {
    pub fn load(path: &Path) -> Blocks {
        let store = Store::new(path).private().bak(Bak::Fresh);
        let (list, unread) = match store.load_json::<BlockFile>() {
            (Some(f), _) if f.v == VERSION => (f.blocked, false),
            (Some(_), _) => (vec![], true),
            (None, Status::Broken) => (vec![], store.kept().is_none()),
            (None, _) => (vec![], false),
        };
        Blocks { store, list: Mutex::new(list), unread }
    }

    fn list_mut(&self) -> MutexGuard<'_, Vec<Block>> {
        self.list.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn unread(&self) -> bool {
        self.unread
    }

    pub fn list(&self) -> Vec<Block> {
        self.list_mut().clone()
    }

    fn save(&self, list: &[Block]) -> std::io::Result<()> {
        self.store.put_json(&BlockFile { v: VERSION, blocked: list.to_vec() }, true)
    }

    /// Blocks `id` from now on. It is on disk when this returns Ok.
    pub fn block(&self, id: DeviceId, name: &str, held: Stamp, now: u64) -> std::io::Result<()> {
        let mut list = self.list_mut();
        let mut next: Vec<Block> = list.iter().filter(|b| b.id != id).cloned().collect();
        next.push(Block { id, name: crate::clean(name, NAME_MAX), epoch: held.epoch, seq: held.seq, at: now });
        self.save(&next)?;
        *list = next;
        Ok(())
    }

    /// Ok(false) when it was not blocked.
    pub fn unblock(&self, id: &DeviceId) -> std::io::Result<bool> {
        let mut list = self.list_mut();
        if !list.iter().any(|b| b.id == *id) {
            return Ok(false);
        }
        let next: Vec<Block> = list.iter().filter(|b| b.id != *id).cloned().collect();
        self.save(&next)?;
        *list = next;
        Ok(true)
    }

    /// Whether `id` is blocked under the roster `held`.
    pub fn blocks(&self, id: &DeviceId, held: Stamp) -> bool {
        self.unread || self.list_mut().iter().any(|b| b.id == *id && (held.epoch, held.seq) <= (b.epoch, b.seq))
    }

    /// Drops the blocks a roster with `held` has ended. Best effort: a block that is not dropped is not in
    /// force either (`blocks` compares).
    pub fn prune(&self, held: Stamp) {
        let mut list = self.list_mut();
        if list.iter().any(|b| (held.epoch, held.seq) > (b.epoch, b.seq)) {
            let next: Vec<Block> =
                list.iter().filter(|b| (held.epoch, held.seq) <= (b.epoch, b.seq)).cloned().collect();
            if self.save(&next).is_ok() {
                *list = next;
            }
        }
    }
}

// ---- the gate --------------------------------------------------------------------------------------------------

/// Why a device is not let in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refused {
    /// No roster is held, or it does not list the device.
    NotOnRoster,
    /// The roster held is more than seven days old.
    Stale,
    Blocked,
}

impl Refused {
    pub fn kind(self) -> &'static str {
        match self {
            Refused::NotOnRoster => "not_on_roster",
            Refused::Stale => "roster_stale",
            Refused::Blocked => "blocked",
        }
    }

    pub fn message(self) -> &'static str {
        match self {
            Refused::NotOnRoster => "This device is not on the home computer's list of devices.",
            Refused::Stale => {
                "The home computer's list of devices is more than 7 days old here, so nobody is let in until the \
                 home computer is back."
            }
            Refused::Blocked => "This device is blocked on this computer.",
        }
    }
}

/// Who may reach this computer's own surface: a device on the roster held, while that roster is young enough,
/// unless it is blocked here. Asked every time, never remembered.
pub struct RosterGate {
    pub cache: Arc<RosterCache>,
    pub blocks: Arc<Blocks>,
    clock: Box<dyn Fn() -> u64 + Send + Sync>,
}

impl RosterGate {
    pub fn new(cache: Arc<RosterCache>, blocks: Arc<Blocks>) -> RosterGate {
        RosterGate { cache, blocks, clock: Box::new(unix_now) }
    }

    /// With a clock of the caller's (tests).
    pub fn with_clock(mut self, clock: impl Fn() -> u64 + Send + Sync + 'static) -> RosterGate {
        self.clock = Box::new(clock);
        self
    }

    pub fn now(&self) -> u64 {
        (self.clock)()
    }

    pub fn check(&self, id: &DeviceId) -> Result<(), Refused> {
        let Some(roster) = self.cache.get() else { return Err(Refused::NotOnRoster) };
        if roster.stale(self.now()) {
            return Err(Refused::Stale);
        }
        if !roster.lists(id) {
            return Err(Refused::NotOnRoster);
        }
        if self.blocks.blocks(id, roster.stamp()) {
            return Err(Refused::Blocked);
        }
        Ok(())
    }
}

impl crate::endpoint::Gate for RosterGate {
    fn allows(&self, id: &[u8; 32]) -> bool {
        self.check(&DeviceId(*id)).is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::endpoint::Gate;
    use crate::testing::scratch;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    const T0: u64 = 1_795_000_000;

    fn dev(name: &str, kind: Kind) -> (DeviceKey, RosterDevice) {
        let key = DeviceKey::generate();
        let d = RosterDevice {
            id: key.id(),
            name: name.into(),
            kind,
            machine: (kind == Kind::Computer).then(|| name.to_lowercase()),
            ver: Some("0.9.0".into()),
            own: None,
            addrs: vec![],
        };
        (key, d)
    }

    fn body(home: &DeviceKey, epoch: u64, seq: u64, at: u64, devices: &[RosterDevice]) -> Body {
        Body {
            home: home.id(),
            home_name: "Home".into(),
            epoch,
            seq,
            at,
            relay: Some("https://relay.example.com".into()),
            devices: devices.to_vec(),
        }
    }

    fn signed(home: &DeviceKey, epoch: u64, seq: u64, at: u64, devices: &[RosterDevice]) -> Envelope {
        sign(home, body(home, epoch, seq, at, devices)).unwrap().envelope
    }

    #[test]
    fn a_roster_goes_out_in_the_format_the_plan_gives_and_reads_back() {
        let home = DeviceKey::generate();
        let (_, mut laptop) = dev("Laptop", Kind::Computer);
        laptop.addrs = vec!["192.0.2.10:51820".parse().unwrap()];
        let (_, mut phone) = dev("Phone", Kind::Phone);
        phone.ver = None;
        let r = sign(&home, body(&home, 3, 57, T0, &[laptop.clone(), phone.clone()])).unwrap();
        assert_eq!(r.envelope.v, 1);
        assert!(!r.envelope.body.contains('=') && !r.envelope.sig.contains('='));
        assert_eq!(B64.decode(&r.envelope.sig).unwrap().len(), 64);
        let v: serde_json::Value = serde_json::from_slice(&B64.decode(&r.envelope.body).unwrap()).unwrap();
        assert_eq!(
            v,
            serde_json::json!({
            "home": home.id().to_string(), "home_name": "Home", "epoch": 3, "seq": 57, "at": T0,
            "relay": "https://relay.example.com",
            "devices": [
                {"id": laptop.id.to_string(), "name": "Laptop", "kind": "computer", "machine": "laptop",
                 "ver": "0.9.0", "addrs": ["192.0.2.10:51820"]},
                {"id": phone.id.to_string(), "name": "Phone", "kind": "phone"},
            ]})
        );
        let back = verify(&r.envelope, &home.id(), Some(T0)).unwrap();
        assert_eq!(back, r);
        assert_eq!(back.stamp(), Stamp { epoch: 3, seq: 57, at: T0 });
        assert!(back.lists(&phone.id) && !back.lists(&DeviceKey::generate().id()));
        assert_eq!(back.expires(), T0 + 7 * 86_400);
        // the envelope survives a trip through JSON, as it travels
        let text = serde_json::to_string(&r.envelope).unwrap();
        let again: Envelope = serde_json::from_str(&text).unwrap();
        assert_eq!(verify(&again, &home.id(), Some(T0)).unwrap(), r);
    }

    #[test]
    fn what_home_found_out_about_a_helper_travels_with_it_and_is_absent_until_home_knows() {
        let home = DeviceKey::generate();
        let (_, mut old) = dev("Mini", Kind::Computer);
        old.own = Some(false);
        let (_, mut new) = dev("Tower", Kind::Computer);
        new.own = Some(true);
        let (_, unasked) = dev("Laptop", Kind::Computer);
        let r = sign(&home, body(&home, 1, 4, T0, &[old.clone(), new.clone(), unasked.clone()])).unwrap();
        let v: serde_json::Value = serde_json::from_slice(&B64.decode(&r.envelope.body).unwrap()).unwrap();
        let owns: Vec<_> = v["devices"].as_array().unwrap().iter().map(|d| d.get("own").cloned()).collect();
        assert_eq!(owns, vec![Some(false.into()), Some(true.into()), None]);
        let back = verify(&r.envelope, &home.id(), Some(T0)).unwrap();
        assert_eq!(
            [&old, &new, &unasked].map(|d| back.device(&d.id).unwrap().own),
            [Some(false), Some(true), None],
            "and a list signed before the field existed reads as not known"
        );
    }

    #[test]
    fn a_changed_body_or_signature_is_refused() {
        let home = DeviceKey::generate();
        let (_, laptop) = dev("Laptop", Kind::Computer);
        let good = signed(&home, 1, 1, T0, std::slice::from_ref(&laptop));
        assert!(verify(&good, &home.id(), Some(T0)).is_ok());

        // the body with a device added by someone who is not home, under the old signature
        let (_, thief) = dev("Thief", Kind::Computer);
        let forged = serde_json::to_vec(&body(&home, 1, 2, T0, &[laptop.clone(), thief])).unwrap();
        let e = Envelope { body: B64.encode(&forged), ..good.clone() };
        assert_eq!(verify(&e, &home.id(), Some(T0)), Err(RosterError::Signature));

        // one bit of the body, one bit of the signature
        let mut bytes = B64.decode(&good.body).unwrap();
        bytes[10] ^= 1;
        let e = Envelope { body: B64.encode(&bytes), ..good.clone() };
        assert_eq!(verify(&e, &home.id(), Some(T0)), Err(RosterError::Signature));
        let mut sig = B64.decode(&good.sig).unwrap();
        sig[5] ^= 0x40;
        let e = Envelope { sig: B64.encode(&sig), ..good.clone() };
        assert_eq!(verify(&e, &home.id(), Some(T0)), Err(RosterError::Signature));

        // not base64, a short signature, a version nobody knows
        for e in [
            Envelope { body: "***".into(), ..good.clone() },
            Envelope { sig: B64.encode([0u8; 63]), ..good.clone() },
            Envelope { sig: String::new(), ..good.clone() },
        ] {
            assert_eq!(verify(&e, &home.id(), Some(T0)), Err(RosterError::Encoding));
        }
        let e = Envelope { v: 2, ..good.clone() };
        assert_eq!(verify(&e, &home.id(), Some(T0)), Err(RosterError::Version(2)));
    }

    #[test]
    fn only_the_pinned_home_can_sign() {
        let home = DeviceKey::generate();
        let (member_key, member) = dev("Laptop", Kind::Computer);
        // a member signs a roster of its own making, naming the real home
        let mut b = body(&home, 9, 9, T0, std::slice::from_ref(&member));
        let bytes = serde_json::to_vec(&b).unwrap();
        let sig = ed25519_dalek::SigningKey::from_bytes(member_key.secret_bytes()).sign(&[SIGNED_AS, &bytes].concat());
        let e = Envelope { v: 1, body: B64.encode(&bytes), sig: B64.encode(sig.to_bytes()) };
        assert_eq!(verify(&e, &home.id(), Some(T0)), Err(RosterError::Signature));
        // `sign` cannot be made to name a home that is not the key's
        b.home = home.id();
        let own = sign(&member_key, b).unwrap();
        assert_eq!(own.body.home, member_key.id());
        assert_eq!(verify(&own.envelope, &home.id(), Some(T0)), Err(RosterError::Signature));
        // another home's good roster is refused by a device paired with this one
        let other = DeviceKey::generate();
        let theirs = signed(&other, 1, 1, T0, &[member]);
        assert_eq!(verify(&theirs, &home.id(), Some(T0)), Err(RosterError::Signature));
        // a body that names another home under this home's own signature (only home itself could make it)
        let bytes = serde_json::to_vec(&body(&other, 1, 1, T0, &[])).unwrap();
        let sig = ed25519_dalek::SigningKey::from_bytes(home.secret_bytes()).sign(&[SIGNED_AS, &bytes].concat());
        let e = Envelope { v: 1, body: B64.encode(&bytes), sig: B64.encode(sig.to_bytes()) };
        assert_eq!(verify(&e, &home.id(), Some(T0)), Err(RosterError::WrongHome));
        // a signature made for something else by the same key does not pass as a roster
        let sig = ed25519_dalek::SigningKey::from_bytes(home.secret_bytes()).sign(&bytes);
        let e = Envelope { v: 1, body: B64.encode(&bytes), sig: B64.encode(sig.to_bytes()) };
        assert_eq!(verify(&e, &home.id(), Some(T0)), Err(RosterError::Signature));
    }

    #[test]
    fn a_roster_dated_far_ahead_is_refused_and_a_little_ahead_is_not() {
        let home = DeviceKey::generate();
        let e = signed(&home, 1, 1, T0 + AHEAD_MAX, &[]);
        assert!(verify(&e, &home.id(), Some(T0)).is_ok(), "ten minutes of clock drift");
        let e = signed(&home, 1, 1, T0 + AHEAD_MAX + 1, &[]);
        assert_eq!(verify(&e, &home.id(), Some(T0)), Err(RosterError::Ahead));
        let cache = RosterCache::load(&scratch("roster-ahead").join("roster.json"), home.id());
        assert_eq!(cache.offer(&e, T0), Err(RosterError::Ahead));
        assert!(cache.get().is_none());
    }

    #[test]
    fn too_much_is_refused_on_both_sides() {
        let home = DeviceKey::generate();
        let many: Vec<RosterDevice> = (0..MAX_DEVICES + 1).map(|i| dev(&format!("d{i}"), Kind::Phone).1).collect();
        assert_eq!(sign(&home, body(&home, 1, 1, T0, &many)).unwrap_err(), RosterError::TooManyDevices);
        assert!(sign(&home, body(&home, 1, 1, T0, &many[..MAX_DEVICES])).is_ok());
        // home itself signing too many: still refused by whoever reads it
        let bytes = serde_json::to_vec(&body(&home, 1, 1, T0, &many)).unwrap();
        let sig = ed25519_dalek::SigningKey::from_bytes(home.secret_bytes()).sign(&[SIGNED_AS, &bytes].concat());
        let e = Envelope { v: 1, body: B64.encode(&bytes), sig: B64.encode(sig.to_bytes()) };
        assert_eq!(verify(&e, &home.id(), Some(T0)), Err(RosterError::TooManyDevices));
        let e = Envelope { v: 1, body: "A".repeat(MAX_BYTES * 2), sig: e.sig };
        assert_eq!(verify(&e, &home.id(), Some(T0)), Err(RosterError::TooLarge));
        // more than four addresses are cut, names are cleaned, a machine name the hub would refuse is dropped
        let (_, mut d) = dev("  Lap\u{7}top  ", Kind::Computer);
        d.machine = Some("Not A Machine".into());
        d.addrs = (0..9).map(|i| format!("192.0.2.{i}:1").parse().unwrap()).collect();
        let r = sign(&home, body(&home, 1, 1, T0, &[d])).unwrap();
        assert_eq!((r.body.devices[0].name.as_str(), r.body.devices[0].addrs.len()), ("Laptop", MAX_ADDRS));
        assert_eq!(r.body.devices[0].machine, None);
    }

    #[test]
    fn stamps_order_by_epoch_then_seq_then_at() {
        let s = |epoch, seq, at| Stamp { epoch, seq, at };
        assert!(s(2, 1, 1) > s(1, 99, 99));
        assert!(s(1, 3, 1) > s(1, 2, 99));
        assert!(s(1, 2, 5) > s(1, 2, 4));
        assert!(s(1, 2, 4) == s(1, 2, 4));
    }

    #[test]
    fn a_lower_roster_never_replaces_a_higher_one() {
        let home = DeviceKey::generate();
        let (_, a) = dev("A", Kind::Computer);
        let (_, b) = dev("B", Kind::Phone);
        let d = scratch("roster-order");
        let cache = RosterCache::load(&d.join("roster.json"), home.id());
        assert!(cache.get().is_none() && cache.stamp().is_none());
        // the first one: A and B
        assert_eq!(cache.offer(&signed(&home, 1, 5, T0, &[a.clone(), b.clone()]), T0), Ok(true));
        // B is removed
        let newer = signed(&home, 1, 6, T0 + 10, std::slice::from_ref(&a));
        assert_eq!(cache.offer(&newer, T0 + 10), Ok(true));
        assert!(!cache.get().unwrap().lists(&b.id));
        // the old one again (B replaying it), the same one again, an older epoch with a huge seq, an older at
        for old in [
            signed(&home, 1, 5, T0, &[a.clone(), b.clone()]),
            newer.clone(),
            signed(&home, 0, 900, T0 + 99, &[a.clone(), b.clone()]),
            signed(&home, 1, 6, T0 + 9, &[a.clone(), b.clone()]),
            signed(&home, 1, 5, T0 + 500, &[a.clone(), b.clone()]),
        ] {
            assert_eq!(cache.offer(&old, T0 + 600), Ok(false));
            assert_eq!(cache.stamp(), Some(Stamp { epoch: 1, seq: 6, at: T0 + 10 }));
            assert!(!cache.get().unwrap().lists(&b.id), "the removed device is back");
        }
        // signed again with nothing changed: only `at` moves, and it is taken
        assert_eq!(cache.offer(&signed(&home, 1, 6, T0 + 20, std::slice::from_ref(&a)), T0 + 20), Ok(true));
        // a higher epoch wins whatever its seq
        assert_eq!(cache.offer(&signed(&home, 2, 1, T0 + 5, std::slice::from_ref(&a)), T0 + 20), Ok(true));
        assert_eq!(cache.stamp(), Some(Stamp { epoch: 2, seq: 1, at: T0 + 5 }));
        // something that is not from home changes nothing
        let other = DeviceKey::generate();
        assert_eq!(cache.offer(&signed(&other, 9, 9, T0, std::slice::from_ref(&b)), T0), Err(RosterError::Signature));
        assert_eq!(cache.stamp(), Some(Stamp { epoch: 2, seq: 1, at: T0 + 5 }));
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn falling_back_to_the_copy_never_brings_an_older_roster_back() {
        let home = DeviceKey::generate();
        let (_, a) = dev("A", Kind::Computer);
        let (_, b) = dev("B", Kind::Phone);
        let d = scratch("roster-bak");
        let path = d.join("roster.json");
        let old = signed(&home, 1, 5, T0, &[a.clone(), b.clone()]);
        {
            let cache = RosterCache::load(&path, home.id());
            cache.offer(&old, T0).unwrap();
            cache.offer(&signed(&home, 1, 6, T0 + 10, std::slice::from_ref(&a)), T0 + 10).unwrap();
        }
        let bak = fs::read_to_string(d.join("roster.json.bak")).unwrap();
        assert_eq!(bak, fs::read_to_string(&path).unwrap(), "the copy matches the last save");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        }
        // a power cut leaves zeros where the roster was
        fs::write(&path, [0u8; 400]).unwrap();
        let cache = RosterCache::load(&path, home.id());
        assert_eq!(cache.stamp(), Some(Stamp { epoch: 1, seq: 6, at: T0 + 10 }));
        assert!(!cache.get().unwrap().lists(&b.id), "the removed device came back from the copy");
        assert!(cache.trouble().unwrap().contains("last good copy"));
        assert!(cache.trouble().is_none(), "said once");
        // and after the fall back the older one is still refused
        assert_eq!(cache.offer(&old, T0 + 20), Ok(false));
        assert!(!cache.get().unwrap().lists(&b.id));
        // it also holds across a plain restart
        let again = RosterCache::load(&path, home.id());
        assert_eq!(again.offer(&old, T0 + 20), Ok(false));
        assert_eq!(again.stamp(), Some(Stamp { epoch: 1, seq: 6, at: T0 + 10 }));
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn a_stored_roster_that_home_did_not_sign_lets_nobody_in() {
        let home = DeviceKey::generate();
        let (_, a) = dev("A", Kind::Computer);
        let d = scratch("roster-planted");
        let path = d.join("roster.json");
        // someone writes a roster of another key's into the file
        let other = DeviceKey::generate();
        let planted = CacheFile { v: 1, roster: signed(&other, 5, 5, T0, std::slice::from_ref(&a)) };
        fs::write(&path, serde_json::to_vec(&planted).unwrap()).unwrap();
        let cache = Arc::new(RosterCache::load(&path, home.id()));
        assert!(cache.get().is_none());
        assert!(cache.trouble().unwrap().contains("home computer signed"));
        let gate = RosterGate::new(cache, Arc::new(Blocks::load(&d.join("blocked.json")))).with_clock(|| T0);
        assert_eq!(gate.check(&a.id), Err(Refused::NotOnRoster));
        // random bytes never panic and never let anyone in
        let mut dice = crate::testing::Dice(0x5eed_0004);
        for _ in 0..40 {
            let _ = fs::remove_file(d.join("roster.json.bak"));
            fs::write(&path, dice.bytes(300)).unwrap();
            assert!(RosterCache::load(&path, home.id()).get().is_none());
        }
        let _ = fs::remove_dir_all(&d);
    }

    fn gate_at(d: &Path, home: &DeviceKey, clock: Arc<AtomicU64>) -> RosterGate {
        let cache = Arc::new(RosterCache::load(&d.join("roster.json"), home.id()));
        let blocks = Arc::new(Blocks::load(&d.join("blocked.json")));
        RosterGate::new(cache, blocks).with_clock(move || clock.load(Ordering::Relaxed))
    }

    #[test]
    fn a_stale_roster_allows_nobody() {
        let home = DeviceKey::generate();
        let (_, a) = dev("A", Kind::Computer);
        let d = scratch("roster-stale");
        let clock = Arc::new(AtomicU64::new(T0));
        let gate = gate_at(&d, &home, clock.clone());
        assert_eq!(gate.check(&a.id), Err(Refused::NotOnRoster), "no roster at all");
        assert!(!gate.allows(&a.id.0));
        gate.cache.offer(&signed(&home, 1, 1, T0, std::slice::from_ref(&a)), T0).unwrap();
        assert_eq!(gate.check(&a.id), Ok(()));
        assert!(gate.allows(&a.id.0));
        assert_eq!(gate.check(&DeviceKey::generate().id()), Err(Refused::NotOnRoster));
        // to the second: honoured through at + 7 days, then nobody
        clock.store(T0 + HONOURED, Ordering::Relaxed);
        assert_eq!(gate.check(&a.id), Ok(()));
        clock.store(T0 + HONOURED + 1, Ordering::Relaxed);
        assert_eq!(gate.check(&a.id), Err(Refused::Stale));
        assert!(!gate.allows(&a.id.0));
        assert_eq!(gate.check(&home.id()), Err(Refused::Stale), "nobody means nobody");
        // home signing again makes it young
        gate.cache.offer(&signed(&home, 1, 1, T0 + HONOURED, std::slice::from_ref(&a)), T0 + HONOURED + 1).unwrap();
        assert_eq!(gate.check(&a.id), Ok(()));
        // a clock set far back does not make an old roster young: the roster's own time counts too
        let r = gate.cache.get().unwrap();
        assert!(!r.stale(0) && r.stale(r.expires() + 1));
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn a_blocked_device_is_refused_until_home_changes_the_list() {
        let home = DeviceKey::generate();
        let (_, a) = dev("A", Kind::Computer);
        let (_, b) = dev("B", Kind::Phone);
        let d = scratch("roster-block");
        let clock = Arc::new(AtomicU64::new(T0));
        let gate = gate_at(&d, &home, clock.clone());
        gate.cache.offer(&signed(&home, 1, 4, T0, &[a.clone(), b.clone()]), T0).unwrap();
        assert_eq!(gate.check(&b.id), Ok(()));
        gate.blocks.block(b.id, "B", gate.cache.stamp().unwrap(), T0).unwrap();
        assert_eq!(gate.check(&b.id), Err(Refused::Blocked));
        assert_eq!(gate.check(&a.id), Ok(()), "only the blocked one");
        // it survives a restart
        let again = gate_at(&d, &home, clock.clone());
        assert_eq!(again.check(&b.id), Err(Refused::Blocked));
        assert_eq!(again.blocks.list()[0].name, "B");
        // home signing the same list again does not end it
        again.cache.offer(&signed(&home, 1, 4, T0 + 60, &[a.clone(), b.clone()]), T0 + 60).unwrap();
        assert_eq!(again.check(&b.id), Err(Refused::Blocked));
        // home changing the list does: it has decided about its devices since
        again.cache.offer(&signed(&home, 1, 5, T0 + 90, &[a.clone(), b.clone()]), T0 + 90).unwrap();
        assert_eq!(again.check(&b.id), Ok(()));
        again.blocks.prune(again.cache.stamp().unwrap());
        assert!(again.blocks.list().is_empty());
        assert!(gate_at(&d, &home, clock.clone()).blocks.list().is_empty(), "and the file says so");
        // unblock by hand
        again.blocks.block(b.id, "B", again.cache.stamp().unwrap(), T0).unwrap();
        assert_eq!(again.check(&b.id), Err(Refused::Blocked));
        assert!(again.blocks.unblock(&b.id).unwrap());
        assert!(!again.blocks.unblock(&b.id).unwrap());
        assert_eq!(again.check(&b.id), Ok(()));
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn a_block_list_that_cannot_be_read_refuses_everyone() {
        let home = DeviceKey::generate();
        let (_, a) = dev("A", Kind::Computer);
        let d = scratch("roster-block-newer");
        fs::write(d.join("blocked.json"), r#"{"v":2,"blocked":[]}"#).unwrap();
        let gate = gate_at(&d, &home, Arc::new(AtomicU64::new(T0)));
        gate.cache.offer(&signed(&home, 1, 1, T0, std::slice::from_ref(&a)), T0).unwrap();
        assert!(gate.blocks.unread());
        assert_eq!(gate.check(&a.id), Err(Refused::Blocked));
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn refusals_have_the_kinds_the_plan_names() {
        assert_eq!(Refused::NotOnRoster.kind(), "not_on_roster");
        assert_eq!(Refused::Stale.kind(), "roster_stale");
        assert_eq!(Refused::Blocked.kind(), "blocked");
        for r in [Refused::NotOnRoster, Refused::Stale, Refused::Blocked] {
            assert!(!r.message().contains('!') && !r.message().contains('\u{2014}'));
        }
    }
}
