//! A device's key and its id.
//!
//! The key is 32 random bytes in `~/.config/switchboard/device.key` (mode 600). It is made once, the way the
//! hub's `term.key` is: written to a temp file, forced to disk, then hard-linked into place, so two programs
//! starting at once end up with the same key. One copy, `device.key.bak`, is written beside it when it is made.
//!
//! A key file a power cut emptied or cut short is put aside (`device.key.bad-<time>`, sb-core durable.rs) and the
//! copy is used and written back. With no good copy there is no key, and none is made in its place without being
//! asked: a new key is a new device, which every paired device would have to be told about.
//!
//! The bytes are an Ed25519 secret key, and the device's id is its public key. That is exactly what iroh does
//! with a `SecretKey` (`SecretKey::from_bytes`, and `EndpointId` is the public key), so the id here is the id the
//! endpoint will have.

use sb_core::durable::{Status, Store};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use zeroize::Zeroize;

pub const KEY_LEN: usize = 32;

/// Where the key lives for the user whose home folder is `home`.
pub fn key_path(home: &Path) -> PathBuf {
    home.join(".config/switchboard/device.key")
}

/// The public key of a 32-byte secret key: Ed25519, as iroh derives an `EndpointId`.
pub fn public_of(secret: &[u8; KEY_LEN]) -> [u8; KEY_LEN] {
    ed25519_dalek::SigningKey::from_bytes(secret).verifying_key().to_bytes()
}

/// A device's public key: what the paired list holds and what a peer is known by.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DeviceId(pub [u8; KEY_LEN]);

impl DeviceId {
    pub fn as_bytes(&self) -> &[u8; KEY_LEN] {
        &self.0
    }

    /// Whether the bytes are a point on the curve, which every real public key is.
    pub fn is_key(&self) -> bool {
        ed25519_dalek::VerifyingKey::from_bytes(&self.0).is_ok()
    }

    /// The first 8 hex digits, enough to tell devices apart in a log line.
    pub fn short(&self) -> String {
        crate::hex::encode(&self.0[..4])
    }
}

impl fmt::Display for DeviceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&crate::hex::encode(&self.0))
    }
}

impl fmt::Debug for DeviceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "DeviceId({self})")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BadId;

impl fmt::Display for BadId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a device id is 64 hex digits")
    }
}

impl std::error::Error for BadId {}

impl FromStr for DeviceId {
    type Err = BadId;

    /// 64 hex digits of either case.
    fn from_str(s: &str) -> Result<DeviceId, BadId> {
        crate::hex::decode::<KEY_LEN>(s).map(DeviceId).ok_or(BadId)
    }
}

impl Serialize for DeviceId {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for DeviceId {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<DeviceId, D::Error> {
        let text = <std::borrow::Cow<'de, str>>::deserialize(d)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

/// How `DeviceKey::load` found the key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Found {
    /// Made just now: this device has never been paired with anything.
    Made,
    Loaded,
    /// The file could not be read, and its copy was used and written back. `kept` names the file as it was found.
    Recovered {
        kept: Option<String>,
    },
}

/// This device's secret key. It is never printed, and it is wiped when dropped.
#[derive(Clone)]
pub struct DeviceKey([u8; KEY_LEN]);

impl Drop for DeviceKey {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

impl fmt::Debug for DeviceKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "DeviceKey(id {})", self.id().short())
    }
}

impl DeviceKey {
    /// A new key that is stored nowhere.
    pub fn generate() -> DeviceKey {
        DeviceKey(crate::random())
    }

    pub fn from_bytes(bytes: [u8; KEY_LEN]) -> DeviceKey {
        DeviceKey(bytes)
    }

    /// The secret itself, for the endpoint (`iroh::SecretKey::from_bytes`). Keep it out of logs.
    pub fn secret_bytes(&self) -> &[u8; KEY_LEN] {
        &self.0
    }

    pub fn id(&self) -> DeviceId {
        DeviceId(public_of(&self.0))
    }

    /// The key in `path`, or None when there has never been one. Err when the file is damaged and has no good
    /// copy, or cannot be read at all: the caller must not carry on as a new device.
    pub fn load(path: &Path) -> io::Result<Option<(DeviceKey, Found)>> {
        let store = store(path);
        let (key, status) = store.load(|raw| <[u8; KEY_LEN]>::try_from(raw).ok());
        match (key, status) {
            (Some(k), Status::Recovered) => {
                tighten(path);
                Ok(Some((DeviceKey(k), Found::Recovered { kept: store.kept() })))
            }
            (Some(k), _) => {
                tighten(path);
                let key = DeviceKey(k);
                // a key from before a crash between the link and the copy still gets its copy
                if !bak_path(path).exists() {
                    let _ = write_bak(path, &key);
                }
                Ok(Some((key, Found::Loaded)))
            }
            (None, Status::Broken) => {
                let kept = store.kept().map(|k| format!(" (kept as {k})")).unwrap_or_default();
                Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "{} could not be read{kept} and has no good copy. This device's key is lost: make a new one \
                         and pair again.",
                        path.display()
                    ),
                ))
            }
            (None, _) => Ok(None),
        }
    }

    /// The key in `path`, made now when there has never been one. A damaged key with no good copy is an error,
    /// not a reason to make another (`replace` does that when the user asks).
    pub fn load_or_create(path: &Path) -> io::Result<(DeviceKey, Found)> {
        if let Some(found) = DeviceKey::load(path)? {
            return Ok(found);
        }
        let dir = path.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."));
        std::fs::create_dir_all(dir)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
        }
        let key = DeviceKey::generate();
        static N: AtomicU64 = AtomicU64::new(0);
        let tmp = dir.join(format!(".device.key.{}.{}.tmp", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
        let written = write_new(&tmp, &key.0);
        // a hard link fails when the name is taken, so of two programs starting at once one key wins
        let linked = written.and_then(|()| std::fs::hard_link(&tmp, path));
        let _ = std::fs::remove_file(&tmp);
        match linked {
            Ok(()) => {
                sync_dir(path);
                write_bak(path, &key)?;
                Ok((key, Found::Made))
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => match DeviceKey::load(path)? {
                Some((key, _)) => Ok((key, Found::Loaded)),
                None => Err(io::Error::other(format!("{} was there and is gone again", path.display()))),
            },
            Err(e) => Err(e),
        }
    }

    /// A new key over whatever is in `path`, with a new copy. The old key is gone, so this device has to be
    /// paired again everywhere: only for a user who asked for exactly that.
    pub fn replace(path: &Path) -> io::Result<DeviceKey> {
        let key = DeviceKey::generate();
        DeviceKey::install(path, &key)?;
        Ok(key)
    }

    /// `key` over whatever is in `path`, with its copy: the key a computer is handed when it becomes the home
    /// computer, or the one it made before it stopped being it.
    pub fn install(path: &Path, key: &DeviceKey) -> io::Result<()> {
        store(path).put(&key.0, true)?;
        write_bak(path, key)
    }
}

fn store(path: &Path) -> Store {
    // (no `.bak` rule here: durable.rs copies only JSON, so the key's one copy is written by `write_bak`)
    Store::new(path).private()
}

fn bak_path(path: &Path) -> PathBuf {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    path.with_file_name(format!("{name}.bak"))
}

/// The key's copy: where durable.rs looks when the key file cannot be read. Written whole and forced to disk.
fn write_bak(path: &Path, key: &DeviceKey) -> io::Result<()> {
    Store::new(bak_path(path)).private().put(&key.0, true)
}

fn write_new(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts.open(path)?;
    f.write_all(bytes)?;
    f.sync_all()
}

/// A key file someone left readable by others is made private again. (Windows: the profile folder's ACL.)
fn tighten(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(m) = std::fs::metadata(path)
            && m.permissions().mode() & 0o077 != 0
        {
            let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
        }
    }
    #[cfg(not(unix))]
    let _ = path;
}

fn sync_dir(path: &Path) {
    #[cfg(unix)]
    {
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty())
            && let Ok(d) = std::fs::File::open(dir)
        {
            let _ = d.sync_all();
        }
    }
    #[cfg(not(unix))]
    let _ = path;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::scratch;
    use std::fs;

    fn names(dir: &Path) -> Vec<String> {
        let mut v: Vec<String> =
            fs::read_dir(dir).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        v.sort();
        v
    }

    #[test]
    fn the_id_is_the_ed25519_public_key() {
        // RFC 8032, section 7.1, test 1
        let secret = crate::hex::decode("9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60").unwrap();
        let key = DeviceKey::from_bytes(secret);
        assert_eq!(key.id().to_string(), "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a");
        assert!(key.id().is_key());
        assert_eq!(format!("{key:?}"), "DeviceKey(id d75a9801)", "the secret is never printed");
    }

    #[test]
    fn an_id_reads_and_prints_as_hex() {
        let id = DeviceKey::generate().id();
        let text = id.to_string();
        assert_eq!(text.len(), 64);
        assert_eq!(text.parse::<DeviceId>(), Ok(id));
        assert_eq!(text.to_uppercase().parse::<DeviceId>(), Ok(id));
        assert_eq!(serde_json::to_string(&id).unwrap(), format!("\"{text}\""));
        assert_eq!(serde_json::from_str::<DeviceId>(&format!("\"{text}\"")).unwrap(), id);
        for bad in ["", "abc", &text[1..], &format!("{text}0"), &format!("{}zz", &text[2..])] {
            assert_eq!(bad.parse::<DeviceId>(), Err(BadId), "{bad:?}");
        }
        assert!(serde_json::from_str::<DeviceId>("\"nope\"").is_err());
        assert!(serde_json::from_str::<DeviceId>("7").is_err());
    }

    #[test]
    fn the_key_is_made_once_and_is_private() {
        let d = scratch("key-once");
        let path = d.join("cfg/device.key");
        assert!(DeviceKey::load(&path).unwrap().is_none());
        let (a, found) = DeviceKey::load_or_create(&path).unwrap();
        assert_eq!(found, Found::Made);
        assert_eq!(fs::read(&path).unwrap(), a.secret_bytes());
        assert_eq!(fs::read(d.join("cfg/device.key.bak")).unwrap(), a.secret_bytes(), "one copy at creation");
        assert_eq!(names(&d.join("cfg")), ["device.key", "device.key.bak"], "no temp file is left");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            for f in ["cfg/device.key", "cfg/device.key.bak"] {
                assert_eq!(fs::metadata(d.join(f)).unwrap().permissions().mode() & 0o777, 0o600, "{f}");
            }
            assert_eq!(fs::metadata(d.join("cfg")).unwrap().permissions().mode() & 0o777, 0o700);
        }
        let (b, found) = DeviceKey::load_or_create(&path).unwrap();
        assert_eq!(found, Found::Loaded);
        assert_eq!(a.secret_bytes(), b.secret_bytes());
        assert_eq!(a.id(), b.id());
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn programs_starting_at_once_get_one_key() {
        let d = scratch("key-race");
        let path = d.join("device.key");
        let ids: Vec<DeviceId> = std::thread::scope(|s| {
            let hs: Vec<_> = (0..8).map(|_| s.spawn(|| DeviceKey::load_or_create(&path).unwrap().0.id())).collect();
            hs.into_iter().map(|h| h.join().unwrap()).collect()
        });
        assert!(ids.iter().all(|i| *i == ids[0]), "{ids:?}");
        assert_eq!(fs::read(&path).unwrap(), fs::read(d.join("device.key.bak")).unwrap());
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn a_damaged_key_falls_back_to_its_copy() {
        for (tag, damage) in [("zeros", vec![0u8; 32]), ("cut", vec![7u8; 9]), ("empty", vec![]), ("long", vec![1; 33])]
        {
            let d = scratch(&format!("key-{tag}"));
            let path = d.join("device.key");
            let (made, _) = DeviceKey::load_or_create(&path).unwrap();
            fs::write(&path, &damage).unwrap();
            let (back, found) = DeviceKey::load_or_create(&path).unwrap();
            assert_eq!(back.secret_bytes(), made.secret_bytes(), "{tag}");
            let Found::Recovered { kept: Some(kept) } = found else { panic!("{tag}: {found:?}") };
            assert_eq!(fs::read(d.join(&kept)).unwrap(), damage, "{tag}: the file as it was found is kept");
            assert_eq!(fs::read(&path).unwrap(), made.secret_bytes(), "{tag}: the copy is written back");
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600, "{tag}");
            }
            assert_eq!(DeviceKey::load(&path).unwrap().unwrap().1, Found::Loaded, "{tag}");
            let _ = fs::remove_dir_all(&d);
        }
    }

    #[test]
    fn a_lost_key_is_an_error_and_no_new_one_is_made() {
        let d = scratch("key-lost");
        let path = d.join("device.key");
        let (made, _) = DeviceKey::load_or_create(&path).unwrap();
        fs::write(&path, [0u8; 32]).unwrap();
        fs::write(d.join("device.key.bak"), b"short").unwrap();
        for _ in 0..2 {
            let e = DeviceKey::load_or_create(&path).unwrap_err();
            assert_eq!(e.kind(), io::ErrorKind::InvalidData);
            assert!(e.to_string().contains("pair again"), "{e}");
            assert!(!path.exists(), "nothing was made in its place");
        }
        // the user asks for a new key: a new device
        let fresh = DeviceKey::replace(&path).unwrap();
        assert_ne!(fresh.id(), made.id());
        let (again, found) = DeviceKey::load_or_create(&path).unwrap();
        assert_eq!((again.id(), found), (fresh.id(), Found::Loaded));
        assert_eq!(fs::read(d.join("device.key.bak")).unwrap(), fresh.secret_bytes());
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn a_missing_copy_is_written_again_and_a_removed_key_stays_removed() {
        let d = scratch("key-copy");
        let path = d.join("device.key");
        let (made, _) = DeviceKey::load_or_create(&path).unwrap();
        fs::remove_file(d.join("device.key.bak")).unwrap();
        DeviceKey::load(&path).unwrap().unwrap();
        assert_eq!(fs::read(d.join("device.key.bak")).unwrap(), made.secret_bytes());
        // someone removed the key on purpose: the copy alone does not bring the old device back
        fs::remove_file(&path).unwrap();
        assert!(DeviceKey::load(&path).unwrap().is_none());
        let (next, found) = DeviceKey::load_or_create(&path).unwrap();
        assert_eq!(found, Found::Made);
        assert_ne!(next.id(), made.id());
        assert_eq!(fs::read(d.join("device.key.bak")).unwrap(), next.secret_bytes());
        let _ = fs::remove_dir_all(&d);
    }

    #[cfg(unix)]
    #[test]
    fn a_key_left_open_to_others_is_made_private() {
        use std::os::unix::fs::PermissionsExt;
        let d = scratch("key-mode");
        let path = d.join("device.key");
        DeviceKey::load_or_create(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        DeviceKey::load(&path).unwrap().unwrap();
        assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        let _ = fs::remove_dir_all(&d);
    }
}
