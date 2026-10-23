//! Going through the folders of one machine: its places (the sidebar) and one folder at a time. Files and the New
//! sheet's folder picker share it, so both see the same places, the same states (macOS privacy, a missing place,
//! "Anywhere in home" off, an older agent) and the same words for them.

use std::collections::HashMap;

use gpui::{App, Context, Entity};
use sb_core::api::{ApiFiles, ApiPlaces};
use sb_core::places;

use crate::state::AppState;

/// Why a folder shows no list.
#[derive(Debug, Clone, PartialEq)]
pub enum Closed {
    /// macOS keeps the helper out (Files and Folders). `hint` is the machine's own words.
    Privacy { hint: Option<String> },
    /// The place is not there any more (moved, renamed, a drive unplugged).
    Missing,
    /// The address is in home but "Anywhere in home" is off for this machine.
    RoamOff,
    /// Any other error, in the server's words.
    Error(String),
}

pub struct Nav {
    app: Entity<AppState>,
    pub machine: String,
    /// The sidebar, by machine (kept so going back to a machine paints at once).
    places: HashMap<String, Result<ApiPlaces, String>>,
    pub folder: String,
    pub listing: Option<ApiFiles>,
    pub error: Option<String>,
    pub loading: bool,
    /// Bumped per folder asked for, so a slow answer never replaces the one asked for after it.
    pub seq: u64,
    /// Files and Folders was opened on the machine's screen for this folder.
    pub privacy_opened: Option<String>,
}

impl Nav {
    pub fn new(app: Entity<AppState>, machine: String, folder: String, cx: &mut Context<Self>) -> Nav {
        let mut n = Nav {
            app,
            machine: String::new(),
            places: HashMap::new(),
            folder,
            listing: None,
            error: None,
            loading: false,
            seq: 0,
            privacy_opened: None,
        };
        n.set_machine(machine, cx);
        n
    }

    pub fn places(&self) -> Option<&ApiPlaces> {
        self.places.get(&self.machine).and_then(|r| r.as_ref().ok())
    }

    pub fn places_error(&self) -> Option<&str> {
        self.places.get(&self.machine).and_then(|r| r.as_ref().err()).map(|s| s.as_str())
    }

    /// What the Claude folder is called here: "Claude folder" (the sidebar's own name for it).
    pub fn claude_name(&self) -> String {
        self.places()
            .and_then(|p| p.places.iter().find(|p| p.kind == "claude"))
            .map(|p| p.name.clone())
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| "Claude folder".into())
    }

    pub fn machine_label(&self, cx: &App) -> String {
        self.app.read(cx).snap.label_of(&self.machine).unwrap_or_else(|| self.machine.clone())
    }

    /// The current folder's name, as people read it.
    pub fn name(&self) -> String {
        places::name_of(&self.folder, &self.claude_name())
    }

    /// The current folder's address as people read it ("~/Documents/site", "iCloud Drive/Notes").
    pub fn shown(&self) -> String {
        places::shown(&self.folder, &self.claude_name())
    }

    /// Switches machine: its places (from memory first), then the Claude folder, unless `folder` was set for it.
    pub fn set_machine(&mut self, machine: String, cx: &mut Context<Self>) {
        if machine == self.machine {
            return;
        }
        let first = self.machine.is_empty();
        self.machine = machine;
        if !first {
            self.folder.clear();
        }
        self.listing = None;
        self.privacy_opened = None;
        self.load_places(cx);
        self.load(cx);
    }

    pub fn load_places(&mut self, cx: &mut Context<Self>) {
        if self.machine.is_empty() {
            return;
        }
        let (core, m) = (self.app.read(cx).core.clone(), self.machine.clone());
        cx.spawn(async move |this, cx| {
            let m2 = m.clone();
            let res = cx.background_executor().spawn(async move { core.places(&m2) }).await;
            let _ = this.update(cx, |n, cx| {
                // A server too old for places: the Claude folder is the only place, as before.
                let res = match res {
                    Ok(p) => Ok(p),
                    Err(sb_core::CoreError::Server { status: 404, .. }) => Ok(ApiPlaces {
                        old: true,
                        places: vec![sb_core::api::ApiPlace {
                            kind: "claude".into(),
                            name: "Claude folder".into(),
                            state: "ok".into(),
                            ..Default::default()
                        }],
                        ..Default::default()
                    }),
                    Err(e) => Err(e.to_string()),
                };
                n.places.insert(m, res);
                cx.notify();
            });
        })
        .detach();
    }

    /// Opens a folder by its address.
    pub fn go(&mut self, folder: String, cx: &mut Context<Self>) {
        self.folder = folder;
        self.privacy_opened = None;
        self.load(cx);
    }

    /// The folder above, when there is one.
    pub fn up(&mut self, cx: &mut Context<Self>) -> bool {
        match places::parent(&self.folder) {
            Some(p) => {
                self.go(p, cx);
                true
            }
            None => false,
        }
    }

    pub fn reload(&mut self, cx: &mut Context<Self>) {
        self.load_places(cx);
        self.load(cx);
    }

    fn load(&mut self, cx: &mut Context<Self>) {
        if self.machine.is_empty() {
            return;
        }
        self.seq += 1;
        let seq = self.seq;
        self.loading = true;
        self.error = None;
        cx.notify();
        let (core, m, f) = (self.app.read(cx).core.clone(), self.machine.clone(), self.folder.clone());
        cx.spawn(async move |this, cx| {
            let res = cx.background_executor().spawn(async move { core.files(&m, &f, None) }).await;
            let _ = this.update(cx, |n, cx| {
                if n.seq != seq {
                    return;
                }
                n.loading = false;
                match res {
                    Ok(r) => n.listing = Some(r),
                    Err(e) => {
                        n.listing = None;
                        n.error = Some(e.to_string());
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// The listing for the folder on screen, once it is in (an older one stays while the next loads).
    pub fn shown_listing(&self) -> Option<&ApiFiles> {
        self.listing.as_ref().filter(|l| !self.loading || l.folder == self.folder)
    }

    /// Why the folder on screen shows no list, if it does not.
    pub fn closed(&self) -> Option<Closed> {
        let place = self.places().and_then(|p| p.place_of(&self.folder));
        if let Some(l) = self.shown_listing()
            && l.privacy_blocked()
        {
            return Some(Closed::Privacy { hint: l.hint.clone() });
        }
        let roam_off = self.places().is_some_and(|p| !p.roam && !p.old)
            && places::is_outside(&self.folder)
            && !self.folder.starts_with('@');
        let e = self.error.as_ref()?;
        if roam_off || e.contains("Anywhere in home") || e.contains("keeps chats in its Claude folder") {
            return Some(Closed::RoamOff);
        }
        if place.is_some_and(|p| p.missing()) || e.contains("does not exist") || e.contains("no such place") {
            return Some(Closed::Missing);
        }
        if place.is_some_and(|p| p.privacy()) {
            return Some(Closed::Privacy { hint: None });
        }
        Some(Closed::Error(e.clone()))
    }

    /// Opens Files and Folders on the machine's own screen.
    pub fn open_privacy(&mut self, cx: &mut Context<Self>) {
        let (core, m, f) = (self.app.read(cx).core.clone(), self.machine.clone(), self.folder.clone());
        let label = self.machine_label(cx);
        cx.spawn(async move |this, cx| {
            let res = cx.background_executor().spawn(async move { core.privacy_settings(&m) }).await;
            let _ = this.update(cx, |n, cx| {
                match res {
                    Ok(true) => n.privacy_opened = Some(f),
                    Ok(false) => n.app.update(cx, |s, cx| {
                        s.toast(
                            format!("Couldn't open System Settings on {label}. Open it there yourself."),
                            None,
                            true,
                            cx,
                        )
                    }),
                    Err(e) => n.app.update(cx, |s, cx| s.toast(e.to_string(), None, true, cx)),
                }
                cx.notify();
            });
        })
        .detach();
    }
}
