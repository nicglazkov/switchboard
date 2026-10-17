//! The first-run record: what the user chose the first time Switchboard was installed on this account. The choices
//! are applied once, when the record is written, and never again: later installs and updates read it and leave the
//! login item and the Dock as the user has them now. It lives in the app's settings folder, next to settings.json,
//! and the installer never writes anything else there.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const FILE: &str = "first-run.json";

/// The choices on the options screen. Every one starts off.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Choices {
    #[serde(default)]
    pub open_at_login: bool,
    #[serde(default)]
    pub add_to_dock: bool,
    /// Windows: a shortcut on the desktop (the Start menu one is always made).
    #[serde(default)]
    pub desktop_shortcut: bool,
}

/// One of the choices. Each system offers some of them (Platform::choices).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Choice {
    OpenAtLogin,
    Dock,
    DesktopShortcut,
}

impl Choice {
    pub const ALL: [Choice; 3] = [Choice::OpenAtLogin, Choice::Dock, Choice::DesktopShortcut];

    /// How the welcome screen names it in "... start off".
    pub fn phrase(self) -> &'static str {
        match self {
            Choice::OpenAtLogin => "Opening at login",
            Choice::Dock => "the Dock icon",
            Choice::DesktopShortcut => "a desktop shortcut",
        }
    }

    /// How --quiet and the progress list name it.
    pub fn name(self) -> &'static str {
        match self {
            Choice::OpenAtLogin => "open at login",
            Choice::Dock => "Dock",
            Choice::DesktopShortcut => "desktop shortcut",
        }
    }
}

impl Choices {
    pub fn get(&self, c: Choice) -> bool {
        match c {
            Choice::OpenAtLogin => self.open_at_login,
            Choice::Dock => self.add_to_dock,
            Choice::DesktopShortcut => self.desktop_shortcut,
        }
    }

    pub fn toggle(&mut self, c: Choice) {
        let v = match c {
            Choice::OpenAtLogin => &mut self.open_at_login,
            Choice::Dock => &mut self.add_to_dock,
            Choice::DesktopShortcut => &mut self.desktop_shortcut,
        };
        *v = !*v;
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Record {
    pub choices: Choices,
    /// The version that was installed then.
    pub version: String,
    /// Seconds since 1970.
    pub at: u64,
}

pub fn path(settings_dir: &Path) -> PathBuf {
    settings_dir.join(FILE)
}

/// The saved record. A file that is there but unreadable still counts as made, so choices are never applied twice.
pub fn read(settings_dir: &Path) -> Option<Record> {
    let p = path(settings_dir);
    let bytes = std::fs::read(&p).ok()?;
    Some(serde_json::from_slice(&bytes).unwrap_or(Record {
        choices: Choices::default(),
        version: String::new(),
        at: 0,
    }))
}

/// What to do with the user's choices on this install.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Decision {
    /// First install on this account: apply these, then save them.
    Apply(Choices),
    /// Already chosen at an earlier install: change nothing.
    Keep(Record),
}

pub fn decide(existing: Option<Record>, asked: Choices) -> Decision {
    match existing {
        Some(r) => Decision::Keep(r),
        None => Decision::Apply(asked),
    }
}

/// Saves the record, only if there is none yet (create_new, so two installers at once cannot both write it).
/// Ok(false) when one was already there.
pub fn save(settings_dir: &Path, choices: Choices, version: &str, at: u64) -> std::io::Result<bool> {
    use std::io::Write;
    std::fs::create_dir_all(settings_dir)?;
    let rec = Record { choices, version: version.to_string(), at };
    let body = serde_json::to_vec_pretty(&rec).map_err(std::io::Error::other)?;
    match std::fs::OpenOptions::new().write(true).create_new(true).open(path(settings_dir)) {
        Ok(mut f) => {
            f.write_all(&body)?;
            f.write_all(b"\n")?;
            Ok(true)
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(false),
        Err(e) => Err(e),
    }
}

pub fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("sb-install-record-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn choices_start_off() {
        assert_eq!(Choices::default(), Choices { open_at_login: false, add_to_dock: false, desktop_shortcut: false });
        assert!(Choice::ALL.iter().all(|&c| !Choices::default().get(c)));
    }

    #[test]
    fn first_install_applies_then_later_ones_keep() {
        let d = dir("once");
        let asked = Choices { open_at_login: true, ..Default::default() };
        assert_eq!(decide(read(&d), asked), Decision::Apply(asked));
        assert!(save(&d, asked, "0.8.4", 100).unwrap());

        // An update that asks for something else changes nothing and keeps the first record.
        let later = Choices { add_to_dock: true, desktop_shortcut: true, ..Default::default() };
        let Decision::Keep(r) = decide(read(&d), later) else { panic!("applied twice") };
        assert_eq!(r, Record { choices: asked, version: "0.8.4".into(), at: 100 });
        assert!(!save(&d, later, "0.9.0", 200).unwrap(), "never overwritten");
        assert_eq!(read(&d).unwrap().version, "0.8.4");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_record_from_before_desktop_shortcuts_reads() {
        let d = dir("older");
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(path(&d), r#"{"choices":{"open_at_login":true,"add_to_dock":false},"version":"0.8.4","at":1}"#)
            .unwrap();
        let r = read(&d).unwrap();
        assert!(r.choices.open_at_login && !r.choices.desktop_shortcut);
        let mut c = r.choices;
        c.toggle(Choice::DesktopShortcut);
        assert!(c.get(Choice::DesktopShortcut));
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_damaged_record_still_counts() {
        let d = dir("bad");
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(path(&d), "{not json").unwrap();
        assert!(matches!(decide(read(&d), Choices { open_at_login: true, ..Default::default() }), Decision::Keep(_)));
        assert!(!save(&d, Choices::default(), "1.0.0", 1).unwrap());
        assert_eq!(std::fs::read_to_string(path(&d)).unwrap(), "{not json");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn other_settings_are_left_alone() {
        let d = dir("others");
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("settings.json"), r#"{"theme":"dark"}"#).unwrap();
        save(&d, Choices::default(), "0.8.4", 5).unwrap();
        assert_eq!(std::fs::read_to_string(d.join("settings.json")).unwrap(), r#"{"theme":"dark"}"#);
        std::fs::remove_dir_all(&d).unwrap();
    }
}
