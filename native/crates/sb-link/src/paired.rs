//! The devices home has paired with: the allow list for the tunnel.
//!
//! `~/.local/share/switchboard/paired.json` (mode 600), beside the hub's nodes.json:
//! `{"v":1,"devices":[{"id","name","kind":"computer|phone","machine"?,"paired","via":"code|link|upgrade"}]}`.
//!
//! It is a durable store (sb-core durable.rs) whose copy, `paired.json.bak`, always matches the last save. So a
//! list a power cut emptied comes back from the copy, and a device that was removed is gone from the copy in the
//! same save: falling back can never let a removed device in again. The same rule the hub's nodes.rs keeps.
//!
//! A list that cannot be read allows nobody. One that is there and will not open, or was written by a newer
//! version, is also not written until it reads, so a list this program could not see is never replaced by an
//! empty one.

use crate::identity::DeviceId;
use sb_core::durable::{Bak, Status, Store};
use serde::{Deserialize, Serialize};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

/// The file format this version reads and writes.
pub const VERSION: u32 = 1;
pub const NAME_MAX: usize = 64;

/// Where the list lives for the user whose home folder is `home`.
pub fn paired_path(home: &Path) -> PathBuf {
    home.join(".local/share/switchboard/paired.json")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Computer,
    Phone,
}

/// How the device was paired.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Via {
    Code,
    Link,
    /// A computer that was already linked with a token, paired when home turned keys on.
    Upgrade,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Device {
    pub id: DeviceId,
    #[serde(default)]
    pub name: String,
    pub kind: Kind,
    /// The hub's name for the computer, when it is one that runs chats.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub machine: Option<String>,
    /// When, in seconds since 1970.
    #[serde(default)]
    pub paired: u64,
    pub via: Via,
}

#[derive(Debug, Serialize, Deserialize)]
struct PairedFile {
    v: u32,
    #[serde(default)]
    devices: Vec<Device>,
}

#[derive(Default)]
struct State {
    list: Vec<Device>,
    /// Why the list could not be read, while it still cannot be. Nothing is written until it reads.
    unread: Option<String>,
    /// The file was damaged, has been put aside and had no good copy: the next save is the first of a new list.
    lost: bool,
    /// What `load` has to tell the user, once.
    note: Option<String>,
}

pub struct Paired {
    store: Store,
    state: Mutex<State>,
}

impl Paired {
    /// The list in `path`. Never fails: a list that cannot be read is an empty one that allows nobody, and
    /// `trouble` says why.
    pub fn load(path: &Path) -> Paired {
        let store = Store::new(path).private().bak(Bak::Fresh);
        let state = read(&store);
        Paired { store, state: Mutex::new(state) }
    }

    pub fn path(&self) -> &Path {
        self.store.path()
    }

    fn state(&self) -> MutexGuard<'_, State> {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if s.unread.is_some() {
            // a file that would not open at the start may open by now
            let mut next = read(&self.store);
            if next.unread.is_some() {
                next.note = s.note.take(); // still unread: already said, or still to say
            }
            *s = next;
        }
        s
    }

    /// Something the user should be told about the file, once: it was recovered from its copy, lost, or cannot
    /// be read.
    pub fn trouble(&self) -> Option<String> {
        self.state().note.take()
    }

    /// The list could not be read and still cannot: a device that is refused may be a paired one.
    pub fn unread(&self) -> bool {
        self.state().unread.is_some()
    }

    pub fn list(&self) -> Vec<Device> {
        self.state().list.clone()
    }

    pub fn get(&self, id: &DeviceId) -> Option<Device> {
        self.state().list.iter().find(|d| d.id == *id).cloned()
    }

    /// Whether the peer with this public key may open a tunnel.
    pub fn allows(&self, id: &[u8; 32]) -> bool {
        self.state().list.iter().any(|d| d.id.0 == *id)
    }

    /// Adds a device, in place of any with the same id. It is on disk when this returns Ok.
    pub fn add(&self, device: Device) -> io::Result<()> {
        let device = Device {
            name: crate::clean(&device.name, NAME_MAX),
            machine: device.machine.filter(|m| crate::valid_machine(m)),
            ..device
        };
        let mut s = self.state();
        let mut next: Vec<Device> = s.list.iter().filter(|d| d.id != device.id).cloned().collect();
        next.push(device);
        self.save(&mut s, next)
    }

    /// Forgets a device: it is refused from now on, and it is gone from the copy too. Ok(false) when it was not
    /// in the list.
    pub fn remove(&self, id: &DeviceId) -> io::Result<bool> {
        let mut s = self.state();
        if s.unread.is_none() && !s.list.iter().any(|d| d.id == *id) {
            return Ok(false);
        }
        let next: Vec<Device> = s.list.iter().filter(|d| d.id != *id).cloned().collect();
        self.save(&mut s, next).map(|()| true)
    }

    fn save(&self, s: &mut State, next: Vec<Device>) -> io::Result<()> {
        if let Some(why) = &s.unread {
            return Err(io::Error::other(format!(
                "{} could not be read ({why}), so it is not being changed",
                self.store.path().display()
            )));
        }
        self.store.put_json(&PairedFile { v: VERSION, devices: next.clone() }, s.lost)?;
        s.lost = false;
        s.list = next;
        Ok(())
    }
}

fn read(store: &Store) -> State {
    let path = store.path().display();
    match store.load_json::<PairedFile>() {
        (Some(f), _) if f.v != VERSION => State {
            unread: Some(format!("it is version {}, and this program reads version {VERSION}", f.v)),
            note: Some(format!(
                "{path} was written by a newer version (format {}). No paired device is let in until this program \
                 is updated.",
                f.v
            )),
            ..State::default()
        },
        (Some(f), status) => State {
            note: (status == Status::Recovered).then(|| {
                let kept = store.kept().map(|k| format!(", and the file as it was is kept as {k}")).unwrap_or_default();
                format!(
                    "{path} could not be read: its last good copy is in use{kept}. A device paired or removed \
                     since that copy has to be done again."
                )
            }),
            list: f.devices,
            ..State::default()
        },
        (None, Status::Broken) => match store.kept() {
            // damaged, put aside, no copy: the list is lost, and the next pairing starts a new one
            Some(kept) => State {
                lost: true,
                note: Some(format!(
                    "{path} could not be read (kept as {kept}) and has no good copy. Every device has to be paired \
                     again."
                )),
                ..State::default()
            },
            // there, and it will not open: left exactly as it is
            None => State {
                unread: Some("it would not open".into()),
                note: Some(format!("{path} would not open. No paired device is let in until it can be read.")),
                ..State::default()
            },
        },
        (None, _) => State::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::DeviceKey;
    use crate::testing::scratch;
    use std::fs;

    fn device(name: &str, kind: Kind) -> Device {
        Device {
            id: DeviceKey::generate().id(),
            name: name.into(),
            kind,
            machine: None,
            paired: 1_795_000_000,
            via: Via::Code,
        }
    }

    fn ids(p: &Paired) -> Vec<DeviceId> {
        p.list().into_iter().map(|d| d.id).collect()
    }

    #[test]
    fn devices_are_saved_in_the_format_the_plan_gives() {
        let d = scratch("paired-format");
        let path = d.join("share/paired.json");
        let p = Paired::load(&path);
        assert!(p.list().is_empty() && p.trouble().is_none() && !p.unread());
        let mut mac = device("  Sam's\u{7} Netbook ", Kind::Computer);
        mac.machine = Some("netbook".into());
        mac.via = Via::Upgrade;
        let phone = Device { via: Via::Link, ..device("Phone", Kind::Phone) };
        p.add(mac.clone()).unwrap();
        p.add(phone.clone()).unwrap();

        let v: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(
            v,
            serde_json::json!({"v": 1, "devices": [
                {"id": mac.id.to_string(), "name": "Sam's Netbook", "kind": "computer", "machine": "netbook",
                 "paired": 1_795_000_000u64, "via": "upgrade"},
                {"id": phone.id.to_string(), "name": "Phone", "kind": "phone", "paired": 1_795_000_000u64,
                 "via": "link"},
            ]})
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            for f in ["paired.json", "paired.json.bak"] {
                assert_eq!(fs::metadata(d.join("share").join(f)).unwrap().permissions().mode() & 0o777, 0o600);
            }
        }
        let again = Paired::load(&path);
        assert_eq!(ids(&again), [mac.id, phone.id]);
        assert!(again.allows(&mac.id.0) && again.allows(&phone.id.0));
        assert!(!again.allows(&DeviceKey::generate().id().0));
        assert_eq!(again.get(&mac.id).unwrap().machine.as_deref(), Some("netbook"));
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn pairing_again_replaces_the_device() {
        let d = scratch("paired-again");
        let p = Paired::load(&d.join("paired.json"));
        let a = device("old name", Kind::Computer);
        p.add(a.clone()).unwrap();
        p.add(Device { name: "new name".into(), machine: Some("Not A Machine".into()), ..a.clone() }).unwrap();
        let list = p.list();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "new name");
        assert_eq!(list[0].machine, None, "a machine name the hub would refuse is not stored");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn a_removed_device_never_returns_from_the_copy() {
        let d = scratch("paired-removed");
        let path = d.join("paired.json");
        let p = Paired::load(&path);
        let (a, b) = (device("a", Kind::Computer), device("b", Kind::Phone));
        p.add(a.clone()).unwrap();
        p.add(b.clone()).unwrap();
        assert!(p.remove(&a.id).unwrap());
        assert!(!p.remove(&a.id).unwrap(), "already gone");
        assert!(!p.allows(&a.id.0));
        let bak = fs::read_to_string(d.join("paired.json.bak")).unwrap();
        assert!(!bak.contains(&a.id.to_string()) && bak.contains(&b.id.to_string()), "{bak}");

        // a power cut leaves zeros where the list was
        fs::write(&path, [0u8; 300]).unwrap();
        let back = Paired::load(&path);
        assert_eq!(ids(&back), [b.id]);
        assert!(!back.allows(&a.id.0) && back.allows(&b.id.0));
        assert!(back.trouble().unwrap().contains("last good copy"));
        assert!(back.trouble().is_none(), "said once");
        assert!(!fs::read_to_string(&path).unwrap().contains(&a.id.to_string()));
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn a_cut_list_with_no_copy_lets_nobody_in_and_starts_over() {
        let d = scratch("paired-lost");
        let path = d.join("paired.json");
        let a = device("a", Kind::Computer);
        Paired::load(&path).add(a.clone()).unwrap();
        fs::write(&path, br#"{"v":1,"devices":[{"id":"#).unwrap();
        fs::remove_file(d.join("paired.json.bak")).unwrap();
        let p = Paired::load(&path);
        assert!(p.list().is_empty() && !p.allows(&a.id.0) && !p.unread());
        assert!(p.trouble().unwrap().contains("paired again"));
        assert_eq!(sb_core::durable::bads(&path).len(), 1, "the file as it was found is kept");
        // pairing is someone meaning it: the new list is written
        let b = device("b", Kind::Phone);
        p.add(b.clone()).unwrap();
        assert_eq!(ids(&Paired::load(&path)), [b.id]);
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn a_list_from_a_newer_version_is_neither_used_nor_written() {
        let d = scratch("paired-newer");
        let path = d.join("paired.json");
        let a = device("a", Kind::Computer);
        let text = format!(r#"{{"v":2,"devices":[{{"id":"{}","kind":"computer","via":"code","extra":1}}]}}"#, a.id);
        fs::write(&path, &text).unwrap();
        let p = Paired::load(&path);
        assert!(p.unread() && p.list().is_empty() && !p.allows(&a.id.0));
        assert!(p.trouble().unwrap().contains("newer version"));
        assert!(p.add(device("b", Kind::Phone)).is_err());
        assert!(p.remove(&a.id).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), text, "left exactly as it was");
        // the same file at this version reads, with fields this version does not know ignored
        fs::write(&path, text.replace(r#""v":2"#, r#""v":1"#)).unwrap();
        assert!(!p.unread());
        assert_eq!(ids(&p), [a.id]);
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn a_wrong_shape_or_a_bad_id_is_damage_not_an_empty_list() {
        for (tag, text) in [
            ("array", "[]".to_string()),
            ("no-version", r#"{"devices":[]}"#.to_string()),
            ("bad-id", r#"{"v":1,"devices":[{"id":"zz","kind":"phone","via":"code"}]}"#.to_string()),
            (
                "bad-kind",
                format!(r#"{{"v":1,"devices":[{{"id":"{}","kind":"toaster","via":"code"}}]}}"#, "ab".repeat(32)),
            ),
        ] {
            let d = scratch(&format!("paired-{tag}"));
            let path = d.join("paired.json");
            let a = device("a", Kind::Computer);
            Paired::load(&path).add(a.clone()).unwrap();
            fs::write(&path, &text).unwrap();
            let p = Paired::load(&path);
            assert_eq!(ids(&p), [a.id], "{tag}: the copy is used");
            assert_eq!(sb_core::durable::bads(&path).len(), 1, "{tag}");
            let _ = fs::remove_dir_all(&d);
        }
    }

    #[test]
    fn random_bytes_in_the_file_never_panic() {
        let d = scratch("paired-fuzz");
        let path = d.join("paired.json");
        let mut dice = crate::testing::Dice(0x5eed_0001);
        for _ in 0..60 {
            let _ = fs::remove_file(d.join("paired.json.bak"));
            fs::write(&path, dice.bytes(200)).unwrap();
            let p = Paired::load(&path);
            assert!(p.list().is_empty());
            let _ = p.trouble();
        }
        let _ = fs::remove_dir_all(&d);
    }
}
