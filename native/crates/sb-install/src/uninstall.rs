//! What the uninstaller removes and what it keeps. The app, its login item and its caches always go; the user's
//! settings and data go only when the user asks. The lists shown before anything happens are the same lists the
//! uninstaller then works through, so what it says is what it does.

use std::path::{Path, PathBuf};

/// Where the app keeps things on this computer (from the platform backend).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layout {
    pub app: PathBuf,
    pub login_item: Option<PathBuf>,
    /// The app has an icon in the Dock (it would be left pointing at nothing).
    pub in_dock: bool,
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
    Caches,
    Data,
}

impl What {
    pub fn label(self) -> &'static str {
        match self {
            What::App => "The app",
            What::LoginItem => "Its login item",
            What::Dock => "Its icon in the Dock",
            What::Caches => "Caches and saved window state",
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
            ["The app", "Its login item", "Its icon in the Dock", "Caches and saved window state"],
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
}
