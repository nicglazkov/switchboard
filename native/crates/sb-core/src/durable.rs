//! State that survives a power cut, with the rules of server/durable.py.
//!
//! A store is one JSON file. It is written whole to a temp file beside it, forced to disk, and renamed over the
//! old one, so a reader never sees half a file and a power cut never leaves zeros where the contents were (seen
//! on a Windows machine, November 2026: the rename had reached the disk, the contents had not).
//!
//! A file that cannot be read is not an empty one. `Store::load` tells the two apart:
//!
//! - `Absent`: no file. The caller's default is right.
//! - `Ok`: read.
//! - `Recovered`: the file was empty, cut short, not JSON or not what the store holds. It is put aside as
//!   `<name>.bad-<time>` (the newest three are kept) and the copy `put` left beside it (`<name>.bak`) is used and
//!   written back.
//! - `Broken`: damaged, and no good copy. The caller gets nothing, and `put` refuses to write over the store
//!   until someone who means it saves it again (`force`), so defaults are never written down as if the user had
//!   chosen them.
//!
//! A file that is there and will not open (no permission, another program holding it) is not damaged. It is read
//! again after a moment, and then the copy is used with the file left exactly as it is.
//!
//! On Unix a private store is made with mode 0600 and the folder is forced to disk after the rename. On Windows
//! the file is as private as the profile folder it lives in.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::Duration;

/// Copies of a file that could not be read: the newest are kept.
pub const BAD_KEEP: usize = 3;
/// Waits before reading again a file that is there and would not open.
const READ_RETRY: [Duration; 2] = [Duration::from_millis(50), Duration::from_millis(200)];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Absent,
    Ok,
    Recovered,
    Broken,
}

/// What `put` keeps beside the file as `<name>.bak`, for `load` to go back to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Bak {
    /// nothing: a cache, which is gathered again
    #[default]
    None,
    /// the file as it was before this write
    Keep,
    /// a copy of the file as it is after this write, for a store whose old contents may hold what was just
    /// taken out of it (a removed server, a replaced token). `load` must never bring that back.
    Fresh,
}

/// A write to a store that could not be read and has no good copy.
#[derive(Debug)]
struct Refused(String);

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} could not be read and has no good copy, so it is not written over", self.0)
    }
}

impl std::error::Error for Refused {}

/// Whether a `put` failed because the store is broken (and the write was not forced).
pub fn is_refused(e: &io::Error) -> bool {
    e.get_ref().is_some_and(|inner| inner.is::<Refused>())
}

#[derive(Debug, Clone)]
struct Mark {
    status: Status,
    /// the name the damaged file has now; None when it would not open and was left alone
    kept: Option<String>,
    /// a broken store that has been saved again since
    replaced: bool,
}

#[derive(Default)]
struct Seen {
    /// stores found damaged since this start
    marks: HashMap<PathBuf, Mark>,
    /// stores whose folder was searched once for a copy an earlier run put aside
    looked: HashSet<PathBuf>,
}

fn seen() -> MutexGuard<'static, Seen> {
    static SEEN: OnceLock<Mutex<Seen>> = OnceLock::new();
    SEEN.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner())
}

fn name_of(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

fn bak_path(path: &Path) -> PathBuf {
    path.with_file_name(format!("{}.bak", name_of(path)))
}

/// The names of the copies of `path` that were put aside, oldest first.
pub fn bads(path: &Path) -> Vec<String> {
    let prefix = format!("{}.bad-", name_of(path));
    let dir = path.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let mut names: Vec<String> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with(&prefix))
        .collect();
    names.sort();
    names
}

fn bad_path(path: &Path) -> PathBuf {
    path.with_file_name(format!("{}.bad-{}", name_of(path), chrono::Local::now().format("%Y%m%d-%H%M%S")))
}

fn prune_bads(path: &Path) {
    let all = bads(path);
    for old in all.iter().take(all.len().saturating_sub(BAD_KEEP)) {
        let _ = fs::remove_file(path.with_file_name(old));
    }
}

/// Moves a file that cannot be used to `<name>.bad-<time>`, keeping the newest `BAD_KEEP`. Its new name, or None
/// when there was nothing to move.
pub fn aside(path: &Path) -> Option<String> {
    let bad = bad_path(path);
    fs::rename(path, &bad).ok()?;
    prune_bads(path);
    Some(name_of(&bad))
}

/// Copies a file to `<name>.bad-<time>` and leaves it where it is: for a file that still reads in part, before
/// the part that reads is saved over it. The copy's name.
pub fn keep_copy(path: &Path) -> Option<String> {
    let bad = bad_path(path);
    let raw = fs::read(path).ok()?;
    write_new(&bad, &raw, mode_of(path)).ok()?;
    prune_bads(path);
    Some(name_of(&bad))
}

/// Whether there is anything but zeros and blanks in `raw` (a file a power cut emptied has neither).
fn blank(raw: &[u8]) -> bool {
    raw.iter().all(|b| matches!(b, 0 | b' ' | b'\t' | b'\r' | b'\n'))
}

fn holds_json(raw: &[u8]) -> bool {
    !blank(raw) && serde_json::from_slice::<serde::de::IgnoredAny>(raw).is_ok()
}

#[cfg(unix)]
fn mode_of(path: &Path) -> Option<u32> {
    use std::os::unix::fs::PermissionsExt;
    fs::metadata(path).ok().map(|m| m.permissions().mode() & 0o777)
}

#[cfg(not(unix))]
fn mode_of(_path: &Path) -> Option<u32> {
    None
}

/// Writes a file in place and forces it to disk. `mode`: its permissions on Unix, set before anything is written
/// into it, also when an older run left the file behind.
fn write_new(path: &Path, raw: &[u8], mode: Option<u32>) -> io::Result<()> {
    let mut opts = fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        if let Some(mode) = mode {
            opts.mode(mode);
        }
    }
    let mut f = opts.open(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Some(mode) = mode {
            f.set_permissions(fs::Permissions::from_mode(mode))?;
        }
    }
    #[cfg(not(unix))]
    let _ = mode;
    f.write_all(raw)?;
    f.sync_all()
}

/// Forces the folder's own entry (the rename) to disk. Windows has no such call for a folder.
fn sync_dir(path: &Path) {
    #[cfg(unix)]
    {
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty())
            && let Ok(d) = fs::File::open(dir)
        {
            let _ = d.sync_all();
        }
    }
    #[cfg(not(unix))]
    let _ = path;
}

/// Keeps the file as it is now as `<name>.bak`, when it holds JSON. False: nothing kept, and the copy from before
/// stays. `link`: a hard link where the disk has them, for a `put` that replaces the file right after, so the
/// link keeps the old contents for free. Otherwise a copy of its own, which damage to the file cannot reach.
fn backup(path: &Path, link: bool) -> bool {
    let Ok(raw) = fs::read(path) else { return false };
    if !holds_json(&raw) {
        return false;
    }
    let bak = bak_path(path);
    match fs::remove_file(&bak) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => return false,
        _ => {}
    }
    if link && fs::hard_link(path, &bak).is_ok() {
        return true;
    }
    write_new(&bak, &raw, mode_of(path)).is_ok()
}

fn read_file(path: &Path) -> io::Result<Vec<u8>> {
    fs::read(path)
}

enum Read {
    Gone,
    /// there, and it could not be opened, also after the waits
    Unopened,
    Bytes(Vec<u8>),
}

fn read_patiently(path: &Path, waits: &[Duration], read: &dyn Fn(&Path) -> io::Result<Vec<u8>>) -> Read {
    let mut waits = waits.iter();
    loop {
        match read(path) {
            Ok(b) => return Read::Bytes(b),
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Read::Gone,
            Err(_) => match waits.next() {
                Some(w) => std::thread::sleep(*w),
                None => return Read::Unopened,
            },
        }
    }
}

/// One JSON file and how it is kept.
#[derive(Debug, Clone)]
pub struct Store {
    path: PathBuf,
    private: bool,
    bak: Bak,
}

impl Store {
    pub fn new(path: impl Into<PathBuf>) -> Store {
        Store { path: path.into(), private: false, bak: Bak::None }
    }

    /// The file, its copy and its temp file are this user's alone (mode 0600 on Unix).
    pub fn private(mut self) -> Store {
        self.private = true;
        self
    }

    pub fn bak(mut self, bak: Bak) -> Store {
        self.bak = bak;
        self
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Whether the store was found damaged with no good copy, and nobody has saved it again since.
    pub fn broken(&self) -> bool {
        seen().marks.get(&self.path).is_some_and(|m| m.status == Status::Broken && !m.replaced)
    }

    /// The name the damaged file was put aside under, when `load` found one since this start.
    pub fn kept(&self) -> Option<String> {
        seen().marks.get(&self.path).and_then(|m| m.kept.clone())
    }

    fn mark(&self, status: Status, kept: Option<String>) {
        seen().marks.insert(self.path.clone(), Mark { status, kept, replaced: false });
    }

    /// What the store holds, and how it was found: see the top of this file. `parse` says what the file's bytes
    /// are, or None when they are not what this store holds.
    pub fn load<T>(&self, parse: impl Fn(&[u8]) -> Option<T>) -> (Option<T>, Status) {
        self.load_with(parse, &read_file, &READ_RETRY)
    }

    pub fn load_json<T: serde::de::DeserializeOwned>(&self) -> (Option<T>, Status) {
        self.load(|raw| serde_json::from_slice(raw).ok())
    }

    fn load_with<T>(
        &self,
        parse: impl Fn(&[u8]) -> Option<T>,
        read: &dyn Fn(&Path) -> io::Result<Vec<u8>>,
        waits: &[Duration],
    ) -> (Option<T>, Status) {
        let path = &self.path;
        let parse = |raw: &[u8]| if blank(raw) { None } else { parse(raw) };
        let from_copy = || fs::read(bak_path(path)).ok().and_then(|raw| parse(&raw).map(|v| (v, raw)));
        let marked = seen().marks.contains_key(path);
        let found = read_patiently(path, if marked { &[] } else { waits }, read);
        let kept = match found {
            Read::Unopened => {
                // There, and it will not open. That says nothing about what is in it, so it is not put aside and
                // the copy is not written over it: the copy is only read.
                return match from_copy() {
                    Some((v, _)) => {
                        self.mark(Status::Recovered, None);
                        (Some(v), Status::Recovered)
                    }
                    None => {
                        self.mark(Status::Broken, None);
                        (None, Status::Broken)
                    }
                };
            }
            Read::Bytes(raw) => {
                if let Some(v) = parse(&raw) {
                    // someone put a readable file back, or it opens again and was never damaged
                    let mut s = seen();
                    if s.marks.get(path).is_some_and(|m| match m.status {
                        Status::Broken => !m.replaced,
                        Status::Recovered => m.kept.is_none(),
                        _ => false,
                    }) {
                        s.marks.remove(path);
                    }
                    return (Some(v), Status::Ok);
                }
                seen().looked.insert(path.clone());
                aside(path)
            }
            Read::Gone => {
                if self.broken() {
                    return (None, Status::Broken);
                }
                let recovered = seen().marks.get(path).filter(|m| m.status == Status::Recovered).cloned();
                match recovered {
                    // its copy could not be written back the last time: the copy is still what it holds
                    Some(m) => m.kept,
                    None => {
                        if !seen().looked.insert(path.clone()) {
                            return (None, Status::Absent);
                        }
                        // put aside by an earlier run, and nobody has dealt with it yet
                        match bads(path).pop() {
                            Some(last) => Some(last),
                            None => return (None, Status::Absent),
                        }
                    }
                }
            }
        };
        match from_copy() {
            Some((v, raw)) => {
                self.mark(Status::Recovered, kept);
                // (a failure leaves it answered from the copy, and the next load tries again)
                let _ = self.write(&raw, Bak::None);
                (Some(v), Status::Recovered)
            }
            None => {
                self.mark(Status::Broken, kept);
                (None, Status::Broken)
            }
        }
    }

    /// Replaces the file with `bytes`, durably. `force`: write even over a store `load` found broken, for a save
    /// the user asked for or a store that can be gathered again. Any other write to a broken store fails with an
    /// error `is_refused` knows. The temp file is removed when the write fails.
    pub fn put(&self, bytes: &[u8], force: bool) -> io::Result<()> {
        if let Some(m) = seen().marks.get_mut(&self.path)
            && m.status == Status::Broken
            && !m.replaced
        {
            if !force {
                return Err(io::Error::other(Refused(name_of(&self.path))));
            }
            m.replaced = true;
        }
        self.write(bytes, self.bak)
    }

    pub fn put_json<T: serde::Serialize>(&self, value: &T, force: bool) -> io::Result<()> {
        self.put(&serde_json::to_vec_pretty(value).map_err(io::Error::other)?, force)
    }

    fn write(&self, bytes: &[u8], bak: Bak) -> io::Result<()> {
        static N: AtomicU64 = AtomicU64::new(0);
        let path = &self.path;
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            fs::create_dir_all(dir)?;
        }
        // unique to the writer: the process, and each write in it
        let n = N.fetch_add(1, Ordering::Relaxed);
        let tmp = path.with_file_name(format!("{}.{}.{n}.tmp", name_of(path), std::process::id()));
        let done = write_new(&tmp, bytes, self.private.then_some(0o600)).and_then(|()| {
            if bak == Bak::Keep {
                backup(path, true);
            }
            fs::rename(&tmp, path)
        });
        if let Err(e) = done {
            let _ = fs::remove_file(&tmp);
            return Err(e);
        }
        if bak == Bak::Fresh && !backup(path, false) {
            // no copy at all, rather than one from before the change
            let _ = fs::remove_file(bak_path(path));
        }
        sync_dir(path);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    const ZEROS: [u8; 439] = [0; 439];

    fn scratch(tag: &str) -> PathBuf {
        static N: AtomicU64 = AtomicU64::new(0);
        let d = std::env::temp_dir().join(format!(
            "sb-durable-{tag}-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn names(dir: &Path) -> Vec<String> {
        let mut v: Vec<String> =
            fs::read_dir(dir).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        v.sort();
        v
    }

    /// As after a restart: nothing is remembered about any store in `dir`.
    fn restart(dir: &Path) {
        let mut s = seen();
        s.marks.retain(|p, _| !p.starts_with(dir));
        s.looked.retain(|p| !p.starts_with(dir));
    }

    /// The store here holds an object.
    fn object(raw: &[u8]) -> Option<Value> {
        serde_json::from_slice::<Value>(raw).ok().filter(Value::is_object)
    }

    #[test]
    fn a_write_leaves_the_file_and_nothing_else() {
        let d = scratch("write");
        let s = Store::new(d.join("deep/a.json"));
        assert_eq!(s.load(object), (None, Status::Absent));
        s.put(br#"{"a": 1}"#, false).unwrap();
        assert_eq!(s.load(object), (Some(json!({"a": 1})), Status::Ok));
        assert_eq!(names(&d.join("deep")), ["a.json"]);
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn a_file_of_zeros_goes_back_to_its_last_good_copy() {
        let d = scratch("zeros");
        let s = Store::new(d.join("a.json")).bak(Bak::Fresh);
        s.put(br#"{"a": 1}"#, false).unwrap();
        fs::write(s.path(), ZEROS).unwrap();
        assert_eq!(s.load(object), (Some(json!({"a": 1})), Status::Recovered));
        assert!(!s.broken());
        let kept = s.kept().expect("the damaged file is put aside");
        assert!(kept.starts_with("a.json.bad-"), "{kept}");
        assert_eq!(fs::read(d.join(&kept)).unwrap(), ZEROS);
        assert_eq!(fs::read(s.path()).unwrap(), br#"{"a": 1}"#, "and the copy is written back");
        assert_eq!(s.load(object).1, Status::Ok);
        s.put(br#"{"a": 2}"#, false).expect("a recovered store is saved as before");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn a_file_cut_short_with_no_copy_is_broken_and_nothing_is_written_in_its_place() {
        let d = scratch("cut");
        let s = Store::new(d.join("a.json"));
        let cut = br#"{"servers": ["http://192.0.2.10:81"#;
        fs::write(s.path(), cut).unwrap();
        assert_eq!(s.load(object), (None, Status::Broken));
        assert!(s.broken());
        assert!(!s.path().exists());
        assert_eq!(fs::read(d.join(s.kept().unwrap())).unwrap(), cut);
        assert_eq!(s.load(object), (None, Status::Broken), "the same on every look");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn valid_json_of_the_wrong_shape_is_damage_too() {
        let d = scratch("shape");
        let s = Store::new(d.join("a.json")).bak(Bak::Keep);
        s.put(br#"{"a": 1}"#, false).unwrap();
        s.put(br#"{"a": 2}"#, false).unwrap();
        fs::write(s.path(), "[]").unwrap();
        assert_eq!(s.load(object), (Some(json!({"a": 1})), Status::Recovered), "the file from before the last write");
        // and a copy of the wrong shape is no copy
        let t = Store::new(d.join("b.json"));
        fs::write(t.path(), "[]").unwrap();
        fs::write(d.join("b.json.bak"), "7").unwrap();
        assert_eq!(t.load(object), (None, Status::Broken));
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn a_file_that_will_not_open_is_read_from_its_copy_and_left_alone() {
        let d = scratch("unopened");
        let s = Store::new(d.join("a.json")).bak(Bak::Fresh);
        s.put(br#"{"a": 1}"#, false).unwrap();
        let held = |p: &Path| {
            if p.ends_with("a.json") { Err(io::Error::from(io::ErrorKind::PermissionDenied)) } else { fs::read(p) }
        };
        let before = names(&d);
        assert_eq!(s.load_with(object, &held, &[Duration::ZERO]), (Some(json!({"a": 1})), Status::Recovered));
        assert_eq!(s.kept(), None);
        assert_eq!(names(&d), before, "nothing is put aside and nothing is written");
        // it opens again: it was never damaged
        assert_eq!(s.load(object).1, Status::Ok);
        assert!(!seen().marks.contains_key(s.path()));

        // with no copy the store is broken, and the file is still left exactly as it is
        let t = Store::new(d.join("b.json"));
        fs::write(t.path(), br#"{"b": 1}"#).unwrap();
        let held = |p: &Path| {
            if p.ends_with("b.json") { Err(io::Error::from(io::ErrorKind::PermissionDenied)) } else { fs::read(p) }
        };
        assert_eq!(t.load_with(object, &held, &[]), (None, Status::Broken));
        assert!(is_refused(&t.put(b"{}", false).unwrap_err()));
        assert_eq!(fs::read(t.path()).unwrap(), br#"{"b": 1}"#);
        assert_eq!(t.load(object), (Some(json!({"b": 1})), Status::Ok));
        assert!(!t.broken());
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn a_file_that_opens_on_the_second_try_is_simply_read() {
        let d = scratch("retry");
        let s = Store::new(d.join("a.json"));
        fs::write(s.path(), br#"{"a": 1}"#).unwrap();
        let tries = AtomicU64::new(0);
        let slow = |p: &Path| {
            if tries.fetch_add(1, Ordering::Relaxed) == 0 {
                Err(io::Error::from(io::ErrorKind::PermissionDenied))
            } else {
                fs::read(p)
            }
        };
        assert_eq!(s.load_with(object, &slow, &[Duration::ZERO]), (Some(json!({"a": 1})), Status::Ok));
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn a_broken_store_refuses_a_write_unless_forced() {
        let d = scratch("refuse");
        let s = Store::new(d.join("a.json")).bak(Bak::Fresh);
        fs::write(s.path(), ZEROS).unwrap();
        assert_eq!(s.load(object), (None, Status::Broken));
        let e = s.put(br#"{"theme": "default"}"#, false).unwrap_err();
        assert!(is_refused(&e), "{e}");
        assert!(e.to_string().contains("a.json could not be read"), "{e}");
        assert!(!s.path().exists(), "defaults are not written down");
        assert_eq!(names(&d).len(), 1, "only the damaged file, put aside: {:?}", names(&d));

        // over a restart too: the file is gone, the one put aside says why
        restart(&d);
        assert_eq!(s.load(object), (None, Status::Broken));
        assert!(is_refused(&s.put(b"{}", false).unwrap_err()));

        // a save the user asked for ends it
        s.put(br#"{"theme": "dark"}"#, true).unwrap();
        assert!(!s.broken());
        assert_eq!(s.load(object), (Some(json!({"theme": "dark"})), Status::Ok));
        s.put(br#"{"theme": "light"}"#, false).expect("and later saves go through");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn a_missing_file_is_not_a_broken_one() {
        let d = scratch("absent");
        let s = Store::new(d.join("a.json"));
        assert_eq!(s.load(object), (None, Status::Absent));
        assert_eq!(s.load(object), (None, Status::Absent));
        assert!(!s.broken());
        s.put(b"{}", false).unwrap();
        // a copy alone is not the store: a file someone removed stays removed
        let t = Store::new(d.join("b.json")).bak(Bak::Fresh);
        t.put(br#"{"b": 1}"#, false).unwrap();
        fs::remove_file(t.path()).unwrap();
        assert_eq!(t.load(object), (None, Status::Absent));
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn a_removed_entry_never_comes_back_from_the_copy() {
        let d = scratch("removed");
        let s = Store::new(d.join("a.json")).bak(Bak::Fresh);
        s.put(br#"{"servers": ["home", "office"]}"#, false).unwrap();
        s.put(br#"{"servers": ["home"]}"#, false).unwrap();
        assert_eq!(fs::read(d.join("a.json.bak")).unwrap(), br#"{"servers": ["home"]}"#);
        fs::write(s.path(), ZEROS).unwrap();
        assert_eq!(s.load(object), (Some(json!({"servers": ["home"]})), Status::Recovered));

        // a save that cannot be copied leaves no copy, rather than the one from before
        let t = Store::new(d.join("b.json")).bak(Bak::Fresh);
        t.put(br#"{"servers": ["home", "office"]}"#, false).unwrap();
        t.put(b"not json", false).unwrap();
        assert!(!d.join("b.json.bak").exists());
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn only_the_newest_damaged_copies_are_kept() {
        let d = scratch("bads");
        let s = Store::new(d.join("a.json"));
        for day in 1..=5 {
            fs::write(d.join(format!("a.json.bad-2026110{day}-120000")), "x").unwrap();
        }
        fs::write(d.join("other.json.bad-20261101-120000"), "x").unwrap();
        fs::write(s.path(), ZEROS).unwrap();
        assert_eq!(s.load(object).1, Status::Broken);
        let left = bads(s.path());
        assert_eq!(left.len(), BAD_KEEP, "{left:?}");
        assert_eq!(left[..2], ["a.json.bad-20261104-120000", "a.json.bad-20261105-120000"]);
        assert_eq!(Some(&left[2]), s.kept().as_ref());
        assert!(d.join("other.json.bad-20261101-120000").exists(), "another store's copies are its own");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn a_file_that_reads_in_part_is_copied_before_it_is_saved_over() {
        let d = scratch("copy");
        let p = d.join("a.json");
        fs::write(&p, br#"{"tabs": 7}"#).unwrap();
        let name = keep_copy(&p).unwrap();
        assert!(name.starts_with("a.json.bad-"));
        assert_eq!(fs::read(d.join(name)).unwrap(), br#"{"tabs": 7}"#);
        assert!(p.exists());
        assert_eq!(keep_copy(&d.join("missing.json")), None);
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn a_failed_write_keeps_the_old_file_and_leaves_no_temp_file() {
        let d = scratch("failed");
        // a folder with something in it is where the file should go: the rename fails on every OS
        let s = Store::new(d.join("a.json"));
        fs::create_dir_all(s.path().join("in the way")).unwrap();
        assert!(s.put(b"{}", false).is_err());
        assert_eq!(names(&d), ["a.json"]);
        let _ = fs::remove_dir_all(&d);
    }

    #[cfg(unix)]
    #[test]
    fn a_private_store_and_its_copy_are_this_users_alone() {
        use std::os::unix::fs::PermissionsExt;
        let d = scratch("private");
        let mode = |n: &str| fs::metadata(d.join(n)).unwrap().permissions().mode() & 0o777;
        for bak in [Bak::Fresh, Bak::Keep] {
            let s = Store::new(d.join("node.json")).private().bak(bak);
            s.put(br#"{"token": "a"}"#, false).unwrap();
            s.put(br#"{"token": "b"}"#, false).unwrap();
            assert_eq!((mode("node.json"), mode("node.json.bak")), (0o600, 0o600), "{bak:?}");
            // and what load writes back from the copy
            fs::write(s.path(), ZEROS).unwrap();
            assert_eq!(s.load(object).1, Status::Recovered);
            assert_eq!(mode("node.json"), 0o600);
            assert_eq!(mode(&s.kept().unwrap()), 0o600, "the damaged file keeps the permissions it had");
            restart(&d);
        }
        let _ = fs::remove_dir_all(&d);
    }
}
