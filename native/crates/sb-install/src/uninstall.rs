//! What the uninstaller removes and what it keeps. The app, its login item and its caches always go; the user's
//! settings and data go only when the user asks. The lists shown before anything happens are the same lists the
//! uninstaller then works through, so what it says is what it does.

use std::path::{Path, PathBuf};

/// Where the app keeps things on this computer (from the platform backend).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Layout {
    pub app: PathBuf,
    pub login_item: Option<PathBuf>,
    /// The app has an icon in the Dock (it would be left pointing at nothing).
    pub in_dock: bool,
    /// Shortcut files that open the app (Windows: the Start menu and the desktop).
    pub shortcuts: Vec<PathBuf>,
    /// Things that are not files and that the backend found present, such as registry keys and values (shown by
    /// their registry path). Always removed.
    pub entries: Vec<Item>,
    /// Rebuilt by the app as needed: snapshot cache, saved window state, logs.
    pub caches: Vec<PathBuf>,
    /// The user's: settings, saved places and the first-run record.
    pub data: Vec<PathBuf>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum What {
    App,
    LoginItem,
    Dock,
    Shortcuts,
    /// Windows: the Settings > Apps entry and the name notifications show.
    Registry,
    Caches,
    Data,
}

impl What {
    pub fn label(self) -> &'static str {
        match self {
            What::App => "The app",
            What::LoginItem => "Opening it when you sign in",
            What::Dock => "Its icon in the Dock",
            What::Shortcuts => "Its shortcuts",
            What::Registry => "Its entry in Settings > Apps",
            What::Caches => "Caches, logs and saved window state",
            What::Data => "Your settings and saved layout",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub what: What,
    pub path: PathBuf,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Plan {
    pub remove: Vec<Item>,
    pub keep: Vec<Item>,
}

/// The plan for this layout. Only paths that exist are listed; the app is always listed, so a missing one reads as
/// "already gone" rather than an empty plan.
pub fn plan(layout: &Layout, remove_data: bool, exists: impl Fn(&Path) -> bool) -> Plan {
    let mut p = Plan::default();
    p.remove.push(Item { what: What::App, path: layout.app.clone() });
    if let Some(l) = layout.login_item.as_ref().filter(|l| exists(l)) {
        p.remove.push(Item { what: What::LoginItem, path: l.clone() });
    }
    if layout.in_dock {
        p.remove.push(Item { what: What::Dock, path: layout.app.clone() });
    }
    p.remove.extend(layout.entries.iter().filter(|i| i.what == What::LoginItem).cloned());
    for s in layout.shortcuts.iter().filter(|s| exists(s)) {
        p.remove.push(Item { what: What::Shortcuts, path: s.clone() });
    }
    p.remove.extend(layout.entries.iter().filter(|i| i.what != What::LoginItem).cloned());
    let mut add = |what: What, path: &PathBuf, remove: bool| {
        if exists(path) {
            let item = Item { what, path: path.clone() };
            if remove { p.remove.push(item) } else { p.keep.push(item) }
        }
    };
    for c in &layout.caches {
        add(What::Caches, c, true);
    }
    for d in &layout.data {
        add(What::Data, d, remove_data);
    }
    p
}

impl Plan {
    /// One line per kind of thing, in order, for the lists on screen.
    pub fn labels(items: &[Item]) -> Vec<&'static str> {
        let mut out: Vec<&'static str> = vec![];
        for i in items {
            if !out.contains(&i.what.label()) {
                out.push(i.what.label());
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout() -> Layout {
        Layout {
            app: "/Applications/Switchboard.app".into(),
            login_item: Some("/Users/you/Library/LaunchAgents/launcher.plist".into()),
            in_dock: true,
            caches: vec![
                "/Users/you/Library/Caches/app".into(),
                "/Users/you/Library/Saved Application State/app".into(),
            ],
            data: vec![
                "/Users/you/Library/Application Support/app".into(),
                "/Users/you/Library/Preferences/app.plist".into(),
            ],
            ..Default::default()
        }
    }

    fn whats(items: &[Item]) -> Vec<What> {
        items.iter().map(|i| i.what).collect()
    }

    #[test]
    fn keeps_data_by_default() {
        let p = plan(&layout(), false, |_| true);
        assert_eq!(whats(&p.remove), [What::App, What::LoginItem, What::Dock, What::Caches, What::Caches]);
        assert_eq!(whats(&p.keep), [What::Data, What::Data]);
        assert_eq!(
            Plan::labels(&p.remove),
            ["The app", "Opening it when you sign in", "Its icon in the Dock", "Caches, logs and saved window state"],
            "one line per kind"
        );
        assert_eq!(Plan::labels(&p.keep), ["Your settings and saved layout"]);
    }

    #[test]
    fn removes_data_only_when_asked() {
        let p = plan(&layout(), true, |_| true);
        assert!(p.keep.is_empty());
        assert_eq!(p.remove.iter().filter(|i| i.what == What::Data).count(), 2);
    }

    #[test]
    fn lists_only_what_is_there() {
        let l = Layout { in_dock: false, ..layout() };
        let p = plan(&l, false, |p| p.ends_with("Caches/app") || p.ends_with("Preferences/app.plist"));
        assert_eq!(whats(&p.remove), [What::App, What::Caches]);
        assert_eq!(p.keep, [Item { what: What::Data, path: "/Users/you/Library/Preferences/app.plist".into() }]);
    }

    #[test]
    fn the_app_is_always_listed() {
        let l = Layout { login_item: None, in_dock: false, caches: vec![], data: vec![], ..layout() };
        let p = plan(&l, true, |_| false);
        assert_eq!(whats(&p.remove), [What::App]);
        assert!(p.keep.is_empty());
    }

    #[test]
    fn windows_shortcuts_and_registry_entries() {
        let reg = |what, p: &str| Item { what, path: p.into() };
        let l = Layout {
            app: r"C:\Users\you\AppData\Local\Programs\Switchboard".into(),
            shortcuts: vec!["start/Switchboard.lnk".into(), "desktop/Switchboard.lnk".into()],
            entries: vec![reg(What::Registry, r"HKCU\Uninstall\app"), reg(What::LoginItem, r"HKCU\Run\Switchboard")],
            caches: vec!["logs".into()],
            data: vec!["settings".into()],
            ..Default::default()
        };
        // The desktop shortcut was never made; registry entries are listed as the backend found them.
        let p = plan(&l, false, |p| !p.starts_with("desktop"));
        assert_eq!(whats(&p.remove), [What::App, What::LoginItem, What::Shortcuts, What::Registry, What::Caches]);
        assert_eq!(p.remove[2].path, PathBuf::from("start/Switchboard.lnk"));
        assert_eq!(
            Plan::labels(&p.remove)[1..4],
            ["Opening it when you sign in", "Its shortcuts", "Its entry in Settings > Apps"]
        );
    }
}
