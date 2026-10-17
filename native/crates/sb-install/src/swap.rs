//! Putting a new app in place in one step, with the old one put back if anything fails.
//!
//! The new bundle is copied next to the old one first (`.Switchboard.app.new`, same folder, so the swap is a rename on
//! one volume). Then the old one is renamed aside (`.Switchboard.app.old`), the new one renamed into its place, and
//! the old one removed. A failure before the swap leaves the old app untouched; a failure during it renames the old
//! one back.

use std::io;
use std::path::{Path, PathBuf};

/// The file operations a swap needs. `Disk` is the real one; tests use a stand-in that fails on purpose.
pub trait Fs {
    fn exists(&self, p: &Path) -> bool;
    fn copy_tree(&self, from: &Path, to: &Path) -> io::Result<()>;
    fn rename(&self, from: &Path, to: &Path) -> io::Result<()>;
    fn remove_tree(&self, p: &Path) -> io::Result<()>;
    /// Checks a copied bundle before it goes in (on a Mac: its signature). Ok by default.
    fn verify(&self, _app: &Path) -> io::Result<()> {
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    /// The new bundle (on the disk image, or wherever the installer found it).
    pub src: PathBuf,
    /// Where it ends up: `<folder>/Switchboard.app`.
    pub dest: PathBuf,
    /// The copy made first, next to `dest`.
    pub staged: PathBuf,
    /// Where the old bundle waits during the swap; None for a fresh install.
    pub backup: Option<PathBuf>,
}

/// The plan for installing `src` as `dest`. `replacing`: there is a bundle at `dest` already.
pub fn plan(src: &Path, dest: &Path, replacing: bool) -> Plan {
    let name = dest.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let dir = dest.parent().unwrap_or(Path::new("."));
    Plan {
        src: src.to_path_buf(),
        dest: dest.to_path_buf(),
        staged: dir.join(format!(".{name}.new")),
        backup: replacing.then(|| dir.join(format!(".{name}.old"))),
    }
}

/// One step as it happens, for the progress list and --quiet's lines.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Copy,
    Verify,
    Swap,
    Cleanup,
}

#[derive(Debug)]
pub enum Error {
    /// Nothing changed: the old app (if any) is where it was.
    Untouched(Step, io::Error),
    /// The swap failed and the old app was put back.
    RolledBack(io::Error),
    /// The swap failed and so did putting the old app back: it is at `backup`.
    Stranded { err: io::Error, backup: PathBuf },
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Untouched(Step::Copy, e) => write!(f, "Could not copy the app ({e}). Nothing was changed."),
            Error::Untouched(Step::Verify, e) => {
                write!(f, "The copied app did not pass its check ({e}). Nothing was changed.")
            }
            Error::Untouched(_, e) => write!(f, "Could not move the old app aside ({e}). Nothing was changed."),
            Error::RolledBack(e) => write!(f, "Could not put the new app in place ({e}). The old one was put back."),
            Error::Stranded { err, backup } => write!(
                f,
                "Could not put the new app in place ({err}), and the old one could not be moved back. It is at {}.",
                backup.display()
            ),
        }
    }
}

/// Runs the plan. `progress` hears each step before it starts. On success, a failed cleanup is not an error: it
/// returns the leftover path so the caller can mention it.
pub fn run(p: &Plan, fs: &dyn Fs, progress: &mut dyn FnMut(Step)) -> Result<Option<PathBuf>, Error> {
    // Leftovers from an earlier run that stopped half way. The old app itself is never among them.
    for left in [Some(&p.staged), p.backup.as_ref()].into_iter().flatten() {
        if fs.exists(left) {
            fs.remove_tree(left).map_err(|e| Error::Untouched(Step::Copy, e))?;
        }
    }
    progress(Step::Copy);
    if let Err(e) = fs.copy_tree(&p.src, &p.staged) {
        let _ = fs.remove_tree(&p.staged);
        return Err(Error::Untouched(Step::Copy, e));
    }
    progress(Step::Verify);
    if let Err(e) = fs.verify(&p.staged) {
        let _ = fs.remove_tree(&p.staged);
        return Err(Error::Untouched(Step::Verify, e));
    }
    progress(Step::Swap);
    if let Some(backup) = &p.backup
        && let Err(e) = fs.rename(&p.dest, backup)
    {
        let _ = fs.remove_tree(&p.staged);
        return Err(Error::Untouched(Step::Swap, e));
    }
    if let Err(e) = fs.rename(&p.staged, &p.dest) {
        let _ = fs.remove_tree(&p.staged);
        let Some(backup) = &p.backup else { return Err(Error::Untouched(Step::Swap, e)) };
        // A rename is all or nothing, so dest is empty again and the old app can go straight back.
        return match fs.rename(backup, &p.dest) {
            Ok(()) => Err(Error::RolledBack(e)),
            Err(_) => Err(Error::Stranded { err: e, backup: backup.clone() }),
        };
    }
    progress(Step::Cleanup);
    if let Some(backup) = &p.backup
        && fs.remove_tree(backup).is_err()
    {
        return Ok(Some(backup.clone()));
    }
    Ok(None)
}

/// The real file system with the plain copy (tests; the installer goes through the platform's copy and check).
#[cfg(test)]
pub struct Disk;

#[cfg(test)]
impl Fs for Disk {
    fn exists(&self, p: &Path) -> bool {
        p.symlink_metadata().is_ok()
    }
    fn copy_tree(&self, from: &Path, to: &Path) -> io::Result<()> {
        copy_tree(from, to)
    }
    fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        std::fs::rename(from, to)
    }
    fn remove_tree(&self, p: &Path) -> io::Result<()> {
        remove_tree(p)
    }
}

/// Removes a file, a symlink or a folder with everything in it. Missing is fine.
pub fn remove_tree(p: &Path) -> io::Result<()> {
    match p.symlink_metadata() {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
        Ok(m) if m.is_dir() => std::fs::remove_dir_all(p),
        Ok(_) => std::fs::remove_file(p),
    }
}

/// Copies a folder tree, keeping symlinks as symlinks and file permissions.
pub fn copy_tree(from: &Path, to: &Path) -> io::Result<()> {
    let meta = from.symlink_metadata()?;
    if meta.file_type().is_symlink() {
        #[cfg(unix)]
        return std::os::unix::fs::symlink(std::fs::read_link(from)?, to);
        #[cfg(not(unix))]
        return std::fs::copy(from, to).map(|_| ());
    }
    if meta.is_dir() {
        std::fs::create_dir_all(to)?;
        for entry in std::fs::read_dir(from)? {
            let entry = entry?;
            copy_tree(&entry.path(), &to.join(entry.file_name()))?;
        }
        std::fs::set_permissions(to, meta.permissions())?;
        return Ok(());
    }
    std::fs::copy(from, to).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::BTreeMap;

    /// A file system of named bundles in memory: each path holds a label ("old", "new"). Fails where told.
    #[derive(Default)]
    struct Mem {
        files: RefCell<BTreeMap<PathBuf, String>>,
        fail_copy: bool,
        fail_verify: bool,
        /// Fails the n-th rename (1-based).
        fail_rename: Option<usize>,
        fail_rename_back: bool,
        fail_remove: bool,
        renames: RefCell<usize>,
    }

    impl Mem {
        fn with(files: &[(&str, &str)]) -> Mem {
            let m = Mem::default();
            for (p, v) in files {
                m.files.borrow_mut().insert(PathBuf::from(p), v.to_string());
            }
            m
        }
        fn at(&self, p: &str) -> Option<String> {
            self.files.borrow().get(Path::new(p)).cloned()
        }
        fn paths(&self) -> Vec<String> {
            self.files.borrow().keys().map(|p| p.display().to_string()).collect()
        }
    }

    impl Fs for Mem {
        fn exists(&self, p: &Path) -> bool {
            self.files.borrow().contains_key(p)
        }
        fn copy_tree(&self, from: &Path, to: &Path) -> io::Result<()> {
            if self.fail_copy {
                // A partial copy is left behind, like a full disk would.
                self.files.borrow_mut().insert(to.into(), "partial".into());
                return Err(io::Error::other("disk full"));
            }
            let v = self.files.borrow().get(from).cloned().ok_or(io::ErrorKind::NotFound)?;
            self.files.borrow_mut().insert(to.into(), v);
            Ok(())
        }
        fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
            *self.renames.borrow_mut() += 1;
            let n = *self.renames.borrow();
            if self.fail_rename == Some(n) || (self.fail_rename_back && n == 3) {
                return Err(io::Error::other("busy"));
            }
            let v = self.files.borrow_mut().remove(from).ok_or(io::ErrorKind::NotFound)?;
            self.files.borrow_mut().insert(to.into(), v);
            Ok(())
        }
        fn remove_tree(&self, p: &Path) -> io::Result<()> {
            if self.fail_remove {
                return Err(io::Error::other("in use"));
            }
            self.files.borrow_mut().remove(p);
            Ok(())
        }
        fn verify(&self, _app: &Path) -> io::Result<()> {
            if self.fail_verify { Err(io::Error::other("bad signature")) } else { Ok(()) }
        }
    }

    const SRC: &str = "/Volumes/S/Switchboard.app";
    const DEST: &str = "/Applications/Switchboard.app";

    fn update_plan() -> Plan {
        plan(Path::new(SRC), Path::new(DEST), true)
    }

    #[test]
    fn plan_stages_next_to_the_app() {
        let p = update_plan();
        assert_eq!(p.staged, PathBuf::from("/Applications/.Switchboard.app.new"));
        assert_eq!(p.backup, Some(PathBuf::from("/Applications/.Switchboard.app.old")));
        assert_eq!(plan(Path::new(SRC), Path::new(DEST), false).backup, None);
    }

    #[test]
    fn fresh_install() {
        let fs = Mem::with(&[(SRC, "new")]);
        let mut steps = vec![];
        let p = plan(Path::new(SRC), Path::new(DEST), false);
        assert!(run(&p, &fs, &mut |s| steps.push(s)).unwrap().is_none());
        assert_eq!(steps, [Step::Copy, Step::Verify, Step::Swap, Step::Cleanup]);
        assert_eq!(fs.at(DEST).as_deref(), Some("new"));
        assert_eq!(fs.paths(), [DEST, SRC]);
    }

    #[test]
    fn update_swaps_and_removes_the_old_one() {
        let fs = Mem::with(&[(SRC, "new"), (DEST, "old")]);
        run(&update_plan(), &fs, &mut |_| {}).unwrap();
        assert_eq!(fs.at(DEST).as_deref(), Some("new"));
        assert_eq!(fs.paths(), [DEST, SRC]);
    }

    #[test]
    fn leftovers_from_a_stopped_run_are_cleared_first() {
        let fs = Mem::with(&[
            (SRC, "new"),
            (DEST, "old"),
            ("/Applications/.Switchboard.app.new", "partial"),
            ("/Applications/.Switchboard.app.old", "older"),
        ]);
        run(&update_plan(), &fs, &mut |_| {}).unwrap();
        assert_eq!(fs.paths(), [DEST, SRC]);
    }

    #[test]
    fn failed_copy_changes_nothing() {
        let fs = Mem { fail_copy: true, ..Mem::with(&[(SRC, "new"), (DEST, "old")]) };
        let e = run(&update_plan(), &fs, &mut |_| {}).unwrap_err();
        assert!(matches!(e, Error::Untouched(Step::Copy, _)), "{e:?}");
        assert_eq!(fs.at(DEST).as_deref(), Some("old"));
        assert_eq!(fs.paths(), [DEST, SRC], "the partial copy is removed");
    }

    #[test]
    fn failed_check_changes_nothing() {
        let fs = Mem { fail_verify: true, ..Mem::with(&[(SRC, "new"), (DEST, "old")]) };
        let e = run(&update_plan(), &fs, &mut |_| {}).unwrap_err();
        assert!(matches!(e, Error::Untouched(Step::Verify, _)));
        assert_eq!(fs.at(DEST).as_deref(), Some("old"));
        assert_eq!(fs.paths(), [DEST, SRC]);
    }

    #[test]
    fn old_app_that_cannot_move_aside_is_left_alone() {
        let fs = Mem { fail_rename: Some(1), ..Mem::with(&[(SRC, "new"), (DEST, "old")]) };
        let e = run(&update_plan(), &fs, &mut |_| {}).unwrap_err();
        assert!(matches!(e, Error::Untouched(Step::Swap, _)));
        assert_eq!(fs.at(DEST).as_deref(), Some("old"));
        assert_eq!(fs.paths(), [DEST, SRC]);
    }

    #[test]
    fn failed_swap_rolls_back() {
        let fs = Mem { fail_rename: Some(2), ..Mem::with(&[(SRC, "new"), (DEST, "old")]) };
        let e = run(&update_plan(), &fs, &mut |_| {}).unwrap_err();
        assert!(matches!(e, Error::RolledBack(_)), "{e:?}");
        assert_eq!(fs.at(DEST).as_deref(), Some("old"));
        assert_eq!(fs.paths(), [DEST, SRC]);
        assert!(e.to_string().contains("old one was put back"));
    }

    #[test]
    fn failed_rollback_says_where_the_old_app_is() {
        let fs = Mem { fail_rename: Some(2), fail_rename_back: true, ..Mem::with(&[(SRC, "new"), (DEST, "old")]) };
        let e = run(&update_plan(), &fs, &mut |_| {}).unwrap_err();
        let Error::Stranded { backup, .. } = &e else { panic!("{e:?}") };
        assert_eq!(fs.at(&backup.display().to_string()).as_deref(), Some("old"));
        assert!(e.to_string().contains("/Applications/.Switchboard.app.old"));
    }

    #[test]
    fn fresh_install_that_cannot_rename_leaves_nothing() {
        let fs = Mem { fail_rename: Some(1), ..Mem::with(&[(SRC, "new")]) };
        let p = plan(Path::new(SRC), Path::new(DEST), false);
        assert!(matches!(run(&p, &fs, &mut |_| {}), Err(Error::Untouched(Step::Swap, _))));
        assert_eq!(fs.paths(), [SRC]);
    }

    #[test]
    fn cleanup_failure_is_not_an_error() {
        let fs = Mem::with(&[(SRC, "new"), (DEST, "old")]);
        let p = update_plan();
        // Fail removes only after the swap: the first two removes (leftovers) find nothing to remove.
        let fs2 = Mem { fail_remove: true, ..fs };
        let left = run(&p, &fs2, &mut |_| {}).unwrap();
        assert_eq!(left, p.backup);
        assert_eq!(fs2.at(DEST).as_deref(), Some("new"));
    }

    #[test]
    fn real_disk_swap_and_rollback() {
        let root = std::env::temp_dir().join(format!("sb-install-swap-{}", std::process::id()));
        let _ = remove_tree(&root);
        let (src, dest) = (root.join("dmg/Switchboard.app"), root.join("apps/Switchboard.app"));
        for (app, label) in [(&src, "new"), (&dest, "old")] {
            std::fs::create_dir_all(app.join("Contents/MacOS")).unwrap();
            std::fs::write(app.join("Contents/MacOS/Switchboard"), label).unwrap();
        }
        #[cfg(unix)]
        std::os::unix::fs::symlink("MacOS/Switchboard", src.join("Contents/link")).unwrap();
        run(&plan(&src, &dest, true), &Disk, &mut |_| {}).unwrap();
        assert_eq!(std::fs::read_to_string(dest.join("Contents/MacOS/Switchboard")).unwrap(), "new");
        #[cfg(unix)]
        assert!(dest.join("Contents/link").symlink_metadata().unwrap().file_type().is_symlink());
        let names: Vec<_> = std::fs::read_dir(root.join("apps")).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(names, ["Switchboard.app"]);
        remove_tree(&root).unwrap();
    }
}
