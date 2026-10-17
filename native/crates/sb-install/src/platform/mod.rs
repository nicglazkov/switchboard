//! Everything that differs by operating system, behind one trait: where apps go, how a running copy is asked to quit,
//! login items, the Dock, shortcuts, disk images. The install flow (flow.rs), the swap and the screens only talk to
//! this. macOS (mac.rs) and Windows (windows.rs) have real backends; `Plain` covers the rest with file operations
//! only.

use std::io;
use std::path::{Path, PathBuf};

use crate::record::Choice;
use crate::uninstall::Layout;

#[cfg(target_os = "macos")]
mod mac;
// The Mac and Windows have their own backends; Plain is what every other system uses for now.
#[cfg_attr(any(target_os = "macos", windows), allow(dead_code))]
mod plain;
// Its paths, names and registry values are plain code, built and tested everywhere; the calls into Windows are
// only built there.
#[cfg_attr(not(windows), allow(dead_code))]
pub mod windows;

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

    /// The installed app's name inside its folder: `Switchboard.app` on a Mac, the `Switchboard` folder on Windows.
    fn bundle_name(&self) -> String {
        crate::brand::bundle()
    }

    /// The new app to install: `given` (--app) after a check, else the one that came with the installer.
    fn find_payload(&self, given: Option<&Path>) -> Result<PathBuf, String> {
        crate::flow::find_payload(given)
    }

    /// The first-run choices this system offers, in the order the options screen shows them.
    fn choices(&self) -> &'static [Choice] {
        &[Choice::OpenAtLogin, Choice::Dock]
    }
    /// The options screen's line and explanation for a choice.
    fn choice_text(&self, c: Choice) -> (String, String) {
        mac_choice_text(c)
    }
    /// What every install here adds besides the app, said on the welcome screen. None when it is only the app.
    fn also_adds(&self) -> Option<String> {
        None
    }
    /// Where people change login items and the like later: "System Settings" on a Mac.
    fn settings_app(&self) -> &'static str {
        "System Settings"
    }
    /// Problems worth saying before installing that do not stop it (Windows: no usable graphics).
    fn warnings(&self) -> Vec<String> {
        vec![]
    }

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
    /// The swap's renames. Windows retries for a moment, since a copy that just quit or a virus scan of the new
    /// files can hold them briefly.
    fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        std::fs::rename(from, to)
    }
    /// Checks a copied bundle before it replaces the old one.
    fn verify_app(&self, _app: &Path) -> io::Result<()> {
        Ok(())
    }

    fn set_login_item(&self, app: &Path, on: bool) -> io::Result<Applied>;
    fn set_in_dock(&self, app: &Path, on: bool) -> io::Result<Applied>;
    fn set_desktop_shortcut(&self, _app: &Path, _on: bool) -> io::Result<Applied> {
        Ok(Applied::Skipped("desktop shortcuts are only made on Windows".into()))
    }

    /// True when every install also registers the app with the system (`register`), as Windows does.
    fn registers(&self) -> bool {
        false
    }
    /// After the swap: the Start menu shortcut and the Settings > Apps entry, made fresh on every install and update.
    /// Returns what was done, for the progress list.
    fn register(&self, _app: &Path, _version: &str) -> io::Result<String> {
        Ok(String::new())
    }
    /// Removes what `register` added (the uninstaller, for Layout::entries).
    fn unregister(&self, _app: &Path) -> io::Result<Applied> {
        Ok(Applied::Done)
    }
    /// Removes the installed app. Windows overrides it, because the uninstaller may be running from inside it.
    fn remove_app(&self, app: &Path) -> io::Result<()> {
        crate::swap::remove_tree(app)
    }
    /// The last thing before the process exits: temp files, and on Windows the uninstaller's own exe.
    fn cleanup(&self) {}
    /// The install this program is part of, when it runs from inside one (the uninstaller Windows keeps in the app's
    /// folder). Uninstalling then means that copy.
    fn own_install(&self) -> Option<PathBuf> {
        None
    }

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

/// The options screen's words for a choice on a Mac.
pub fn mac_choice_text(c: Choice) -> (String, String) {
    let name = crate::brand::NAME;
    let (a, b) = match c {
        Choice::OpenAtLogin => (
            format!("Open {name} when you log in"),
            "Adds a login item that opens the app. Nothing else runs in the background.",
        ),
        Choice::Dock => (format!("Add {name} to the Dock"), "Puts its icon at the end of the Dock."),
        Choice::DesktopShortcut => (format!("Put a {name} shortcut on the desktop"), "Only made on Windows."),
    };
    (a, b.to_string())
}

/// The app's own files in its log folder (sb-desktop/src/logs.rs and panics.rs): the log and its rotations, the
/// crash files and panic.log. The server writes its logs to the same folder on macOS and Windows, and those stay.
#[cfg(any(target_os = "macos", windows))]
pub fn app_log_files(logs: &Path) -> Vec<PathBuf> {
    let name = crate::brand::NAME;
    let mut out = vec![logs.join("panic.log"), logs.join(format!("{name}.log"))];
    out.extend((1..=5).map(|n| logs.join(format!("{name}.log.{n}"))));
    if let Ok(rd) = std::fs::read_dir(logs) {
        let mut crashes: Vec<PathBuf> = rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("crash-") && n.ends_with(".txt"))
            })
            .collect();
        crashes.sort();
        out.extend(crashes);
    }
    out
}

/// The backend for this system.
pub fn native() -> Box<dyn Platform> {
    #[cfg(target_os = "macos")]
    return Box::new(mac::Mac);
    #[cfg(windows)]
    return Box::new(windows::Windows::new(windows::Places::from_env().unwrap_or_else(|e| {
        eprintln!("switchboard-install: {e}");
        std::process::exit(2)
    })));
    #[cfg(not(any(target_os = "macos", windows)))]
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
