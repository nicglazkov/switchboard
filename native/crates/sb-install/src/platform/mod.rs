//! Everything that differs by operating system, behind one trait: where apps go, how a running copy is asked to quit,
//! login items, the Dock, disk images. The install flow (flow.rs), the swap and the screens only talk to this.
//! macOS is the one real backend today; `Plain` covers the rest with file operations only, and a Windows backend
//! slots in as another implementation.

use std::io;
use std::path::{Path, PathBuf};

use crate::uninstall::Layout;

#[cfg(target_os = "macos")]
mod mac;
// The Mac has its own backend; Plain is what every other system uses for now.
#[cfg_attr(target_os = "macos", allow(dead_code))]
mod plain;

#[cfg(target_os = "macos")]
pub use mac::eject_helper as mac_eject_helper;

/// A folder apps can be installed into.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dir {
    pub path: PathBuf,
    /// How the screens name it: "Applications", "Applications in your home folder".
    pub label: String,
}

/// The disk image the installer was opened from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiskImage {
    pub mount: PathBuf,
    /// The .dmg file, when the system says which one it is.
    pub image: Option<PathBuf>,
}

/// What became of a request to change something outside the app's own files.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Applied {
    Done,
    /// Not done, and why (said to the user as is).
    Skipped(String),
}

pub trait Platform: Send + Sync {
    /// Where a new install can go, best first. The first one this account can write to without an admin wins.
    fn install_dirs(&self) -> Vec<Dir>;

    /// The version a bundle states, if it states one.
    fn bundle_version(&self, app: &Path) -> Option<String> {
        info_plist_version(app)
    }

    /// Process ids of copies running from `app`, and only from there.
    fn running(&self, _app: &Path) -> Vec<u32> {
        vec![]
    }
    /// Asks the copies running from `app` to quit, the way quitting from the menu does. Does not wait.
    fn ask_to_quit(&self, _app: &Path) {}
    /// Stops them without asking. Only after the user said so.
    fn force_quit(&self, _app: &Path) {}

    /// Copies a bundle, keeping everything that makes it run (links, permissions, the signature).
    fn copy_app(&self, from: &Path, to: &Path) -> io::Result<()> {
        crate::swap::copy_tree(from, to)
    }
    /// Checks a copied bundle before it replaces the old one.
    fn verify_app(&self, _app: &Path) -> io::Result<()> {
        Ok(())
    }

    fn set_login_item(&self, app: &Path, on: bool) -> io::Result<Applied>;
    fn set_in_dock(&self, app: &Path, on: bool) -> io::Result<Applied>;

    fn open_app(&self, app: &Path) -> io::Result<()>;

    /// The disk image this installer runs from, if it does.
    fn disk_image(&self) -> Option<DiskImage> {
        None
    }
    /// Once this process has quit: ejects the image and moves the .dmg to the Trash.
    fn eject_after_exit(&self, _img: &DiskImage) -> io::Result<()> {
        Err(io::Error::other("not on this system"))
    }

    /// The app's settings folder (settings.json, first-run.json).
    fn settings_dir(&self) -> Option<PathBuf> {
        sb_core::cache::settings_dir()
    }
    /// What an app installed at `app` keeps on this computer, for the uninstaller.
    fn layout(&self, app: &Path) -> Layout;
}

/// The backend for this system.
pub fn native() -> Box<dyn Platform> {
    #[cfg(target_os = "macos")]
    return Box::new(mac::Mac);
    #[cfg(not(target_os = "macos"))]
    return Box::new(plain::Plain);
}

/// CFBundleShortVersionString from `<app>/Contents/Info.plist` (XML or binary).
pub fn info_plist_version(app: &Path) -> Option<String> {
    let v = plist::Value::from_file(app.join("Contents/Info.plist")).ok()?;
    let d = v.as_dictionary()?;
    d.get("CFBundleShortVersionString").or_else(|| d.get("CFBundleVersion"))?.as_string().map(str::to_string)
}

/// True when this account can create files in `dir` (tried, not guessed: a probe file is made and removed).
pub fn writable(dir: &Path) -> bool {
    let probe = dir.join(format!(".sb-install-probe-{}", std::process::id()));
    match std::fs::OpenOptions::new().write(true).create_new(true).open(&probe) {
        Ok(_) => std::fs::remove_file(&probe).is_ok(),
        Err(_) => false,
    }
}

/// The home folder.
pub fn home() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("."))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_version_from_info_plist() {
        let app = std::env::temp_dir().join(format!("sb-install-plist-{}/X.app", std::process::id()));
        std::fs::create_dir_all(app.join("Contents")).unwrap();
        assert_eq!(info_plist_version(&app), None);
        std::fs::write(
            app.join("Contents/Info.plist"),
            r#"<?xml version="1.0" encoding="UTF-8"?><plist version="1.0"><dict>
            <key>CFBundleShortVersionString</key><string>0.8.4</string></dict></plist>"#,
        )
        .unwrap();
        assert_eq!(info_plist_version(&app).as_deref(), Some("0.8.4"));
        std::fs::remove_dir_all(app.parent().unwrap()).unwrap();
    }

    #[test]
    fn probes_writability() {
        let d = std::env::temp_dir().join(format!("sb-install-w-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        assert!(writable(&d));
        assert!(!writable(&d.join("missing")));
        assert_eq!(std::fs::read_dir(&d).unwrap().count(), 0, "the probe is removed");
        std::fs::remove_dir_all(&d).unwrap();
    }
}
