//! The New session sheet: machine, mode, model, the first message and the folder (the machine's Claude folder by
//! default, a few recent ones, then the whole tree one level at a time, or a new folder inside the one picked).
//! Folder lists paint from the disk cache at once and refresh in the background.
//! v2: Auto, first in the machine row when a machine shares what it can do: Claude picks the machine (and the folder,
//! unless one is typed). While the message is typed, the rules preview the pick.

use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::time::Duration;

use gpui::{
    AnyElement, App, ClickEvent, Context, Entity, EventEmitter, Focusable, FontWeight, InteractiveElement, IntoElement,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement, Render, ScrollHandle, SharedString,
    StatefulInteractiveElement, Styled, Subscription, Window, canvas, div, prelude::*, px, relative,
};
use gpui_base::input::{InputEvent, InputState, TextareaState};
use sb_core::actions::{MODELS, MODES, ULTRA_NOTE, ultra_warn, valid_folder_name};
use sb_core::api::{ApiFolder, ApiPrefs, ApiSetup};
use sb_core::flags::{self, DIRS_MAX, EFFORTS, Opts, PERMS};
use sb_core::{NewRequest, fmt, names};

use crate::state::AppState;
use crate::theme::{NUM, Theme};
use crate::ui;

/// The first message box: its line height (13.5 px text at 1.5), padding, the rows it starts with, and the length at
/// which it shows how much is in it.
const MSG_LINE: f32 = 20.25;
const MSG_PAD: f32 = 22.;
const MSG_MIN_ROWS: usize = 3;
const MSG_COUNT_FROM: usize = 1000;

/// What is typed in the New session sheet, kept on disk as it changes so a crash or a close never loses it. It comes
/// back the next time the sheet opens, until it starts or is a week old.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Draft {
    #[serde(default)]
    pub msg: String,
    #[serde(default)]
    pub machine: String,
    #[serde(default)]
    pub folder: String,
    #[serde(default)]
    pub setup: ApiSetup,
    #[serde(default)]
    pub saved: f64,
}

const DRAFT_KEEP_S: f64 = 7. * 86400.;

impl Draft {
    pub fn load(path: &std::path::Path, now: f64) -> Option<Draft> {
        let d: Draft = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
        (!d.msg.trim().is_empty() && now - d.saved < DRAFT_KEEP_S).then_some(d)
    }

    /// Writes it, or removes the file when there is no message to keep.
    pub fn save(&self, path: &std::path::Path) {
        let r = if self.msg.trim().is_empty() {
            match std::fs::remove_file(path) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
                _ => Ok(()),
            }
        } else {
            sb_core::cache::save_json(path, self)
        };
        if let Err(e) = r {
            log::warn!("could not keep the New session draft: {e}");
        }
    }
}

/// The machine name the server takes for Auto.
pub const AUTO_MACHINE: &str = "auto";

/// A folder Auto may be pointed at: up to six names joined by "/", like the web app's AUTO_FOLDER_RE.
pub fn valid_auto_folder(v: &str) -> bool {
    let parts: Vec<&str> = v.split('/').collect();
    !v.is_empty()
        && parts.len() <= 6
        && parts.iter().all(|p| {
            (1..=64).contains(&p.len())
                && p.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | ' ' | '-'))
        })
}

/// 52340 as "52,340".
fn thousands(n: usize) -> String {
    let d = n.to_string();
    let mut out = String::new();
    for (i, c) in d.chars().enumerate() {
        if i > 0 && (d.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// "Has Xcode 26." as the middle of a sentence: "has Xcode 26".
fn lc_first(t: &str) -> String {
    let t = t.trim().trim_end_matches('.');
    let mut ch = t.chars();
    match (ch.next(), ch.next()) {
        (Some(a), Some(b)) if a.is_ascii_uppercase() && b.is_ascii_lowercase() => {
            format!("{}{}", a.to_ascii_lowercase(), &t[1..])
        }
        _ => t.to_string(),
    }
}

/// The rules' preview of where Auto would run it.
#[derive(Clone)]
enum Preview {
    Pick { label: String, why: String, typed: bool },
    Off(String),
}

pub enum SheetEvent {
    Close,
    /// The server took it and "Open its terminal" is on: the window opens a tab that waits for the job.
    Started {
        job: String,
        machine: String,
        label: String,
        title: String,
    },
}

pub struct NewSheet {
    app: Entity<AppState>,
    /// The open dropdown (effort or permissions), if any.
    drop: ui::Drop,
    machine: String,
    mode: &'static str,
    model: &'static str,
    /// The folder picked ("" is the machine's Claude folder).
    folder: String,
    msg: Entity<TextareaState>,
    /// Characters in the first message, counted as it changes (the count shows once it is long).
    msg_len: usize,
    /// The rows the box grows to before it scrolls (set from the window height), or the rows dragged to by hand.
    msg_rows: (usize, usize),
    msg_fixed: Option<usize>,
    /// Dragging the box's handle: where the drag started and the rows then. The box's drawn height, for that start.
    msg_drag: Option<(f32, usize)>,
    msg_h: Rc<Cell<f32>>,
    /// The middle of the sheet (everything between the title and the buttons) scrolls when it does not fit.
    body_scroll: ScrollHandle,
    /// The draft as last written, and a counter that lets only the last of a burst of changes write it.
    draft_path: Option<std::path::PathBuf>,
    draft_saved: Draft,
    draft_seq: u64,
    newf: Entity<InputState>,
    newf_ph: String,
    root: Option<String>,
    recent: Vec<ApiFolder>,
    /// Folder levels by parent ("" is the top).
    tree: HashMap<String, Vec<ApiFolder>>,
    open: HashSet<String>,
    failed: HashSet<String>,
    sending: bool,
    /// Open the new chat's terminal once it is running (on unless turned off here).
    open_term: bool,
    /// Why the last Start failed, until the message changes.
    error: Option<String>,
    /// v2, Auto: the preview shown, the question it answers ("message\nfolder"), and the question asked last.
    prev: Option<Preview>,
    prev_q: Option<String>,
    prev_want: Option<String>,
    prev_seq: u64,
    /// The folder field was last set up for Auto (true) or a machine (false); it is cleared when that changes.
    newf_auto: bool,
    /// v2: the options (Claude in Chrome, permissions, effort, extra folders, Other flags).
    opt: Opts,
    other: Entity<InputState>,
    /// Text for the Other field, set on the next frame (setting it needs the window).
    pending_other: Option<String>,
    /// The options were changed here, so the setup used last no longer fills them in.
    touched: bool,
    prefs: Option<ApiPrefs>,
    /// The folder panel adds extra folders (--add-dir) instead of picking the one to work in.
    adding: bool,
    /// The Chrome line lists the machines this chat's Chrome work can go to.
    chrome_set: bool,
    /// This chat's own Chrome machine (Auto sends it there; Start fresh and Continue keep it). Never saved for all.
    chrome_machine: Option<String>,
    /// Saving the setup as a preset: the name field is showing.
    naming: bool,
    preset: Entity<InputState>,
    scroll: ScrollHandle,
    /// Library tools picked for this chat, beyond the ones on for every new chat.
    tools: Vec<String>,
    /// Pick (the checklist) instead of Auto, on a server that knows Auto.
    tools_pick: bool,
    _subs: Vec<Subscription>,
}

impl EventEmitter<SheetEvent> for NewSheet {}

impl NewSheet {
    pub fn new(
        app: Entity<AppState>,
        machine: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> NewSheet {
        let msg = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder("What should it work on? Leave it empty to start idle.")
                .auto_grow(MSG_MIN_ROWS, 8)
        });
        let newf = cx.new(|cx| InputState::new(window, cx).placeholder("New folder"));
        let other = cx.new(|cx| InputState::new(window, cx).placeholder("--agent reviewer  --worktree feature"));
        let preset = cx.new(|cx| InputState::new(window, cx).placeholder("Name this setup"));
        let subs = vec![
            cx.subscribe(&other, |this, input, ev: &InputEvent, cx| {
                if let InputEvent::Change = ev {
                    let v = input.read(cx).value().to_string();
                    if v != this.opt.other {
                        this.opt.other = v;
                        this.touched = true;
                        cx.notify();
                    }
                }
            }),
            cx.subscribe(&msg, |this, input, ev: &InputEvent, cx| {
                if let InputEvent::Change = ev {
                    this.msg_len = input.read(cx).text().len_chars();
                    this.error = None;
                    this.preview(cx);
                    cx.notify();
                }
            }),
            cx.subscribe(&newf, |this, _, ev: &InputEvent, cx| {
                if let InputEvent::Change = ev {
                    this.preview(cx);
                    cx.notify();
                }
            }),
        ];
        let snap = &app.read(cx).snap;
        let online = |n: &str| snap.machine(n).is_some_and(|m| !m.is_offline());
        let draft_path = app.read(cx).draft_path();
        let draft = draft_path.as_deref().and_then(|p| Draft::load(p, AppState::now()));
        let asked = machine.is_some();
        let machine = machine
            .filter(|m| online(m))
            .or_else(|| draft.as_ref().map(|d| d.machine.clone()).filter(|m| m == AUTO_MACHINE || online(m)))
            .or_else(|| snap.machines.iter().find(|m| !m.is_offline()).map(|m| m.name.clone()))
            .or_else(|| snap.machines.first().map(|m| m.name.clone()))
            .unwrap_or_default();
        let mut s = NewSheet {
            app,
            drop: ui::Drop::default(),
            machine: String::new(),
            mode: "normal",
            model: "",
            folder: String::new(),
            msg,
            msg_len: 0,
            msg_rows: (MSG_MIN_ROWS, 8),
            msg_fixed: None,
            msg_drag: None,
            msg_h: Rc::new(Cell::new(0.)),
            body_scroll: ScrollHandle::new(),
            draft_path: None,
            draft_saved: Draft::default(),
            draft_seq: 0,
            newf,
            newf_ph: String::new(),
            root: None,
            recent: Vec::new(),
            tree: HashMap::new(),
            open: HashSet::new(),
            failed: HashSet::new(),
            sending: false,
            open_term: true,
            error: None,
            prev: None,
            prev_q: None,
            prev_want: None,
            prev_seq: 0,
            newf_auto: false,
            opt: Opts::default(),
            other,
            pending_other: None,
            touched: false,
            prefs: None,
            adding: false,
            chrome_set: false,
            chrome_machine: None,
            naming: false,
            preset,
            scroll: ScrollHandle::new(),
            tools: Vec::new(),
            tools_pick: false,
            _subs: subs,
        };
        s.set_machine(machine, cx);
        let cached = s.app.read(cx).prefs.clone();
        s.take_prefs(cached, cx);
        s.draft_path = draft_path;
        // Closed (or replaced by another panel) before the last change was written: write it now.
        let rel = cx.on_release(|s: &mut NewSheet, _| {
            if let (false, Some(p)) = (s.sending, &s.draft_path) {
                s.draft_saved.save(p);
            }
        });
        s._subs.push(rel);
        if let Some(d) = draft {
            // The message always comes back; the options and the folder with it, unless another machine was asked for.
            s.msg.update(cx, |m, cx| m.set_value(d.msg.clone(), window, cx));
            s.msg_len = d.msg.chars().count();
            s.apply_setup(&d.setup, cx);
            if !asked || d.machine == s.machine {
                s.folder = d.folder.clone();
            }
            s.draft_saved = d;
        }
        s.app.update(cx, |st, cx| st.load_library(cx));
        let me = cx.entity().downgrade();
        s.app.update(cx, |st, cx| {
            st.load_prefs(
                move |st, cx| {
                    let p = st.prefs.clone();
                    cx.defer(move |cx| {
                        let _ = me.update(cx, |s, cx| s.take_prefs(p, cx));
                    });
                },
                cx,
            )
        });
        s
    }

    /// The prefs arrived: the setup used last fills in the options until they are changed here.
    fn take_prefs(&mut self, p: Option<ApiPrefs>, cx: &mut Context<Self>) {
        let Some(p) = p else { return };
        if let (false, Some(last)) = (self.touched, &p.last) {
            self.opt = Opts::from_setup(last);
            self.pending_other = Some(self.opt.other.clone());
        }
        self.prefs = Some(p);
        self.preview(cx);
        cx.notify();
    }

    /// A setup (last used or a preset) picked from the chips: mode, model and every option.
    fn apply_setup(&mut self, s: &ApiSetup, cx: &mut Context<Self>) {
        let mode = s.mode.as_deref().unwrap_or("normal");
        if let Some(m) = MODES.iter().find(|m| m.0 == mode) {
            self.mode = m.0;
        }
        let model = s.model.as_deref().unwrap_or("");
        self.model = MODELS.iter().find(|m| m.0 == model).map(|m| m.0).unwrap_or("");
        self.opt = Opts::from_setup(s);
        self.pending_other = Some(self.opt.other.clone());
        self.touched = true;
        self.preview(cx);
        cx.notify();
    }

    fn current_setup(&self, name: Option<String>) -> ApiSetup {
        flags::setup(&self.opt, self.model, self.mode, name)
    }

    fn save_preset(&mut self, cx: &mut Context<Self>) {
        let name = self.preset.read(cx).value().trim().to_string();
        let ok = !name.is_empty()
            && name.chars().count() <= 64
            && name.chars().all(|c| c.is_alphanumeric() || " .,:'+#()-_".contains(c));
        if !ok {
            self.app.update(cx, |s, cx| s.toast("Name it with letters, digits and spaces (up to 64).", None, true, cx));
            return;
        }
        let mut list: Vec<ApiSetup> = self.prefs.as_ref().map(|p| p.presets.clone()).unwrap_or_default();
        list.retain(|p| p.name.as_deref() != Some(name.as_str()));
        list.push(self.current_setup(Some(name.clone())));
        self.naming = false;
        let body = serde_json::json!({ "presets": list });
        self.app.update(cx, |s, cx| s.save_prefs(body, Some(format!("Saved the {name} setup")), cx));
        if let Some(p) = &mut self.prefs {
            p.presets = list;
        }
        cx.notify();
    }

    fn toggle_dir(&mut self, f: String, cx: &mut Context<Self>) {
        if f.is_empty() {
            return;
        }
        if let Some(i) = self.opt.dirs.iter().position(|d| *d == f) {
            self.opt.dirs.remove(i);
        } else if self.opt.dirs.len() >= DIRS_MAX {
            self.app.update(cx, |s, cx| s.toast(format!("Up to {DIRS_MAX} extra folders."), None, true, cx));
            return;
        } else {
            self.opt.dirs.push(f);
        }
        self.touched = true;
        cx.notify();
    }

    fn other_error(&self) -> Option<String> {
        let (takes, aliases) = match &self.prefs {
            Some(p) => (p.takes.clone(), p.aliases.clone()),
            None => Default::default(),
        };
        flags::check_other(&self.opt, self.model, self.mode, &takes, &aliases)
    }

    pub fn focus_message(&self, window: &mut Window, cx: &mut App) {
        self.msg.update(cx, |m, cx| m.focus(window, cx));
    }

    fn swap_field(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.msg.focus_handle(cx).is_focused(window) {
            self.newf.update(cx, |n, cx| n.focus(window, cx));
        } else {
            self.msg.update(cx, |m, cx| m.focus(window, cx));
        }
        cx.notify();
    }

    pub fn set_mode(&mut self, mode: &'static str, cx: &mut Context<Self>) {
        self.mode = mode;
        cx.notify();
    }

    fn locked(&self) -> bool {
        self.mode == "ultracode"
    }

    fn is_auto(&self) -> bool {
        self.machine == AUTO_MACHINE
    }

    /// Auto: Claude picks the machine.
    pub fn pick_auto(&mut self, cx: &mut Context<Self>) {
        self.set_machine(AUTO_MACHINE.to_string(), cx);
    }

    fn draft(&self, cx: &App) -> Draft {
        Draft {
            msg: self.msg.read(cx).value().to_string(),
            machine: self.machine.clone(),
            folder: self.folder.clone(),
            setup: self.current_setup(None),
            saved: 0.,
        }
    }

    fn keep_draft(&mut self, cx: &mut Context<Self>) {
        if self.draft_path.is_none() || self.sending {
            return;
        }
        let d = self.draft(cx);
        if (Draft { saved: 0., ..self.draft_saved.clone() }) == d {
            return;
        }
        self.draft_saved = Draft { saved: AppState::now(), ..d };
        self.draft_seq += 1;
        let seq = self.draft_seq;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(400)).await;
            let _ = this.update(cx, |s, _| {
                if let (true, Some(p)) = (s.draft_seq == seq, &s.draft_path) {
                    s.draft_saved.save(p);
                }
            });
        })
        .detach();
    }

    fn drop_draft(&mut self) {
        self.draft_seq += 1;
        self.draft_saved = Draft::default();
        if let Some(p) = &self.draft_path {
            self.draft_saved.save(p);
        }
    }

    /// Claude in Chrome on, and its machine list open (screenshot builds).
    pub fn shot_chrome(&mut self, list: bool, cx: &mut Context<Self>) {
        self.opt.chrome = true;
        self.chrome_set = list;
        self.preview(cx);
        cx.notify();
    }

    /// Fills the first message (screenshot builds). "@long" is a dozen lines and "@huge" about 50,000 characters,
    /// with a long unbroken line in each.
    pub fn set_message(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        let para = "Look through the sync client and add retries with backoff where a request can fail. Keep the public API the same and add tests for each case.";
        let path = "/Users/you/Claude/atlas/src/sync/very/deeply/nested/module/path/that/never/breaks/client_retry_backoff_handler.ts";
        let text = match text {
            "@long" => (0..12)
                .map(|i| if i == 3 { path.to_string() } else { format!("{}. {para}", i + 1) })
                .collect::<Vec<_>>()
                .join("\n"),
            "@huge" => {
                let mut s = format!("{path}{path}\n");
                while s.len() < 50_000 {
                    s.push_str(para);
                    s.push('\n');
                }
                s
            }
            t => t.to_string(),
        };
        self.msg.update(cx, |m, cx| m.set_value(text.clone(), window, cx));
        self.msg_len = text.chars().count();
        self.preview(cx);
    }

    /// The rules' pick for what is typed: the first one at once, later ones 600 ms after the last key.
    fn preview(&mut self, cx: &mut Context<Self>) {
        if !self.is_auto() {
            return;
        }
        let msg = self.msg.read(cx).value().to_string();
        let folder = self.target(cx).map(|t| t.0).unwrap_or_default();
        let chrome = self.opt.chrome;
        let cm = self.chrome_machine.clone();
        let q = format!("{msg}\n{folder}\n{chrome}\n{}", cm.as_deref().unwrap_or(""));
        if self.prev_q.as_deref() == Some(q.as_str()) || self.prev_want.as_deref() == Some(q.as_str()) {
            return;
        }
        self.prev_want = Some(q.clone());
        self.prev_seq += 1;
        let seq = self.prev_seq;
        let wait = if self.prev.is_some() { 600 } else { 0 };
        let core = self.app.read(cx).core.clone();
        cx.spawn(async move |this, cx| {
            if wait > 0 {
                cx.background_executor().timer(Duration::from_millis(wait)).await;
            }
            if this.update(cx, |s, _| s.prev_seq != seq).unwrap_or(true) {
                return;
            }
            let (m2, f2) = (msg.clone(), folder.clone());
            let res =
                cx.background_executor().spawn(async move { core.route(&m2, &f2, true, chrome, cm.as_deref()) }).await;
            let _ = this.update(cx, |s, cx| {
                if s.prev_seq != seq || !s.is_auto() {
                    return;
                }
                s.prev_q = Some(q);
                s.prev_want = None;
                s.prev = Some(match res {
                    Ok(r) => Preview::Pick {
                        label: r.label.or(r.machine).unwrap_or_else(|| "a machine".into()),
                        why: r.why.unwrap_or_default(),
                        typed: !msg.trim().is_empty(),
                    },
                    Err(e) => Preview::Off(e.to_string()),
                });
                cx.notify();
            });
        })
        .detach();
    }

    /// From Files: start in that folder.
    pub fn preset_folder(&mut self, folder: String, cx: &mut Context<Self>) {
        self.folder = folder;
        self.preview(cx);
        cx.notify();
    }

    fn set_machine(&mut self, machine: String, cx: &mut Context<Self>) {
        if machine == self.machine {
            return;
        }
        self.machine = machine.clone();
        self.folder.clear();
        self.open.clear();
        self.failed.clear();
        self.tree.clear();
        if self.is_auto() {
            self.root = None;
            self.recent.clear();
            self.preview(cx);
            cx.notify();
            return;
        }
        let core = self.app.read(cx).core.clone();
        let start = core.cached_folders(&machine);
        self.root = start
            .root
            .or_else(|| self.app.read(cx).snap.machine(&machine).map(|m| m.root.clone()).filter(|r| !r.is_empty()));
        self.recent = start.recent;
        if let Some(top) = start.top {
            self.tree.insert(String::new(), top);
        }
        // Fresh recents and the top level, in the background.
        let (c1, m1, want) = (core.clone(), machine.clone(), machine.clone());
        cx.spawn(async move |this, cx| {
            let res = cx.background_executor().spawn(async move { c1.folder_recents(&m1) }).await;
            let _ = this.update(cx, |s, cx| {
                if let Ok(f) = res
                    && s.machine == want
                {
                    if f.root.is_some() {
                        s.root = f.root;
                    }
                    s.recent = f.recent;
                    cx.notify();
                }
            });
        })
        .detach();
        self.load_level(String::new(), cx);
        cx.notify();
    }

    fn load_level(&mut self, parent: String, cx: &mut Context<Self>) {
        let core = self.app.read(cx).core.clone();
        let machine = self.machine.clone();
        if let Some(l) = core.cached_level(&machine, &parent) {
            self.tree.insert(parent.clone(), l);
        }
        cx.spawn(async move |this, cx| {
            let (c, m, p) = (core.clone(), machine.clone(), parent.clone());
            let res = cx.background_executor().spawn(async move { c.folder_level(&m, &p) }).await;
            let _ = this.update(cx, |s, cx| {
                if s.machine != machine {
                    return;
                }
                match res {
                    Ok(l) => {
                        if parent.is_empty() && l.root.is_some() {
                            s.root = l.root;
                        }
                        s.failed.remove(&parent);
                        s.tree.insert(parent, l.folders);
                    }
                    Err(e) => {
                        log::info!("folders for {machine}: {e}");
                        s.failed.insert(parent);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn toggle(&mut self, folder: String, cx: &mut Context<Self>) {
        if !self.open.remove(&folder) {
            self.open.insert(folder.clone());
            self.load_level(folder, cx);
        }
        cx.notify();
    }

    /// The new folder's name, if one is typed: Ok(name) when the server takes it.
    fn new_name(&self, cx: &App) -> Option<Result<String, ()>> {
        let v = self.newf.read(cx).value().trim().to_string();
        if self.is_auto() {
            let v = v.trim_matches('/').to_string();
            return if v.is_empty() {
                None
            } else if valid_auto_folder(&v) {
                Some(Ok(v))
            } else {
                Some(Err(()))
            };
        }
        if v.is_empty() {
            None
        } else if valid_folder_name(&v) {
            Some(Ok(v))
        } else {
            Some(Err(()))
        }
    }

    /// The folder the chat will start in, and whether the server creates it.
    fn target(&self, cx: &App) -> Option<(String, bool)> {
        if self.is_auto() {
            // A folder typed for Auto is used as it is; Auto never creates one.
            return match self.new_name(cx) {
                None => Some((String::new(), false)),
                Some(Ok(f)) => Some((f, false)),
                Some(Err(())) => None,
            };
        }
        match self.new_name(cx) {
            None => Some((self.folder.clone(), false)),
            Some(Ok(n)) => Some((if self.folder.is_empty() { n } else { format!("{}/{n}", self.folder) }, true)),
            Some(Err(())) => None,
        }
    }

    fn label(&self, cx: &App) -> String {
        if self.is_auto() {
            return sb_core::jobs::AUTO.to_string();
        }
        self.app.read(cx).snap.label_of(&self.machine).unwrap_or_else(|| self.machine.clone())
    }

    /// True when Let Claude pick for Auto is off on the server, so Auto's pick comes from its rules. An older
    /// server that has no such switch keeps the old wording.
    fn rules_pick(&self) -> bool {
        self.prefs.as_ref().and_then(|p| p.option("ai_auto_machine")).is_some_and(|o| !o.on)
    }

    fn root_text(&self) -> String {
        self.root.clone().unwrap_or_else(|| "its Claude folder".into())
    }

    fn name(&self, cx: &App) -> String {
        let msg = self.msg.read(cx).value().to_string();
        let folder = self.target(cx).map(|t| t.0).unwrap_or_default();
        names::created(&names::topic_from(&msg, &folder), self.mode)
    }

    fn where_text(&self, cx: &App) -> String {
        let Some((folder, create)) = self.target(cx) else {
            return if self.is_auto() { "Check the folder name" } else { "Check the new folder name" }.into();
        };
        let auto_place = if folder.is_empty() { String::new() } else { format!(", in {folder}") };
        let place = if create {
            format!("a new folder, {folder}")
        } else if folder.is_empty() {
            self.root_text()
        } else {
            folder
        };
        let mode = match self.mode {
            "ultracode" => ", UltraCode on Opus",
            "max" => ", max effort",
            _ => "",
        };
        let model = if !self.locked() && !self.model.is_empty() {
            MODELS.iter().find(|m| m.0 == self.model).map(|m| format!(", {}", m.1)).unwrap_or_default()
        } else {
            String::new()
        };
        if self.is_auto() {
            let who = if self.rules_pick() { "Rules pick" } else { "Claude picks" };
            return format!("{who} the machine{auto_place}{mode}{model}");
        }
        format!("On {} in {place}{mode}{model}", self.label(cx))
    }

    fn can_start(&self, cx: &App) -> bool {
        let snap = &self.app.read(cx).snap;
        let online = if self.is_auto() {
            snap.auto_ready()
        } else {
            snap.machine(&self.machine).is_some_and(|m| !m.is_offline())
        };
        !self.sending && online && self.target(cx).is_some() && self.other_error().is_none()
    }

    pub fn start(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if !self.can_start(cx) {
            return;
        }
        let Some((folder, create)) = self.target(cx) else { return };
        let prompt = self.msg.read(cx).value().trim().to_string();
        let model = if self.locked() {
            Some("opus".to_string())
        } else {
            (!self.model.is_empty()).then(|| self.model.to_string())
        };
        let effort = (!flags::effort_locked(self.mode) && !self.opt.effort.is_empty()).then(|| self.opt.effort.clone());
        let req = NewRequest {
            machine: self.machine.clone(),
            folder,
            create,
            prompt,
            model,
            mode: self.mode.to_string(),
            effort,
            flags: self.opt.flags(),
            other: self.opt.other.trim().to_string(),
            chrome_machine: if self.opt.chrome { self.chrome_machine.clone() } else { None },
            tools: self.tools_to_send(cx),
        };
        let label = self.label(cx);
        let name = self.name(cx);
        self.sending = true;
        self.error = None;
        cx.notify();
        let me = cx.entity().downgrade();
        let app = self.app.clone();
        let (open_term, machine) = (self.open_term, self.machine.clone());
        let (l2, n2) = (label.clone(), name.clone());
        self.app.update(cx, |s, cx| {
            s.start_new(
                req,
                label,
                name,
                move |res: Result<Option<String>, String>, cx: &mut App| {
                    let shown = me.update(cx, |s, cx| {
                        s.sending = false;
                        match &res {
                            Ok(job) => {
                                s.drop_draft();
                                // The server remembers this setup as the last one used.
                                s.app.update(cx, |st, cx| st.load_prefs(|_, _| {}, cx));
                                if let (true, Some(job)) = (open_term, job) {
                                    cx.emit(SheetEvent::Started { job: job.clone(), machine, label: l2, title: n2 });
                                }
                                cx.emit(SheetEvent::Close)
                            }
                            Err(e) => s.error = Some(e.clone()),
                        }
                        cx.notify();
                    });
                    // Closed before the answer came back: say it in the corner instead.
                    if let (Err(_), Err(e)) = (shown, res) {
                        app.update(cx, |s, cx| s.toast(e, None, true, cx));
                    }
                },
                cx,
            )
        });
    }

    // ---------- rendering ----------

    /// "Open its terminal": a check box left of Cancel, on by default.
    fn term_toggle(&self, t: &Theme, cx: &Context<Self>) -> impl IntoElement {
        let on = self.open_term;
        let (hover, accent, line2, on_accent) = (t.hover, t.accent, t.line2, t.on_accent);
        div()
            .id("nw-term")
            .flex_none()
            .h(px(32.))
            .px(px(8.))
            .mr(px(4.))
            .flex()
            .items_center()
            .gap(px(8.))
            .rounded(px(ui::R_CTRL))
            .cursor_pointer()
            .hover(move |s| s.bg(hover))
            .text_size(px(12.5))
            .text_color(if on { t.ink } else { t.dim })
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.open_term = !this.open_term;
                cx.notify();
            }))
            .child(
                div()
                    .flex_none()
                    .size(px(15.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(ui::R_MIN))
                    .when(on, |d| d.bg(accent))
                    .when(!on, |d| d.shadow(vec![Theme::ring(line2)]))
                    .when(on, |d| d.child(ui::icon("check", 11., on_accent))),
            )
            .child("Open its terminal")
    }

    /// The tools this chat starts with: Auto, or the ones on for every new chat (the server adds them too) then the
    /// picked ones. An older server (no Auto) gets the list as before, and nothing when it is empty.
    fn tools_to_send(&self, cx: &App) -> sb_core::net::Tools {
        use sb_core::net::Tools;
        let Some(lib) = self.app.read(cx).lib() else { return Tools::None };
        if lib.has_auto() && !self.tools_pick {
            return Tools::Auto;
        }
        let mut out: Vec<String> = lib.defaults.iter().filter(|d| lib.tool(d).is_some()).cloned().collect();
        for t in &self.tools {
            if lib.tool(t).is_some() && !out.contains(t) {
                out.push(t.clone());
            }
        }
        if out.is_empty() && !lib.has_auto() { Tools::None } else { Tools::Pick(out) }
    }

    /// Screenshots: the tools on Pick.
    pub fn shot_pick(&mut self, cx: &mut Context<Self>) {
        self.tools_pick = true;
        cx.notify();
    }

    /// Auto | Pick, on a server that knows Auto; Auto says in one line what the chat gets.
    fn tools_mode_line(&self, t: &Theme, cx: &Context<Self>) -> Option<AnyElement> {
        let app = self.app.read(cx);
        let lib = app.lib()?;
        if !lib.has_auto() {
            return None;
        }
        let pick = self.tools_pick;
        let seg = ui::seg_box(t)
            .child(
                ui::seg_btn(t, "nw-tools-auto", !pick, false)
                    .child(ui::icon("auto", 12., if pick { t.dim } else { t.accent }))
                    .child("Auto")
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.tools_pick = false;
                        cx.notify();
                    })),
            )
            .child(ui::seg_btn(t, "nw-tools-pick", pick, false).child("Pick").on_click(cx.listener(
                |this, _: &ClickEvent, _, cx| {
                    this.tools_pick = true;
                    cx.notify();
                },
            )));
        let mut row = div().flex_1().min_w_0().flex().flex_wrap().items_center().gap(px(10.)).child(seg);
        if !pick {
            let (skills, all_skills, mcps) = lib.auto_counts();
            let plural = |n: usize, one: &str| format!("{n} {one}{}", if n == 1 { "" } else { "s" });
            let skill_part = if skills > 0 && skills == all_skills {
                Some("All skills".to_string())
            } else {
                (skills > 0).then(|| plural(skills, "skill"))
            };
            let mcp_part = (mcps > 0).then(|| plural(mcps, "MCP tool"));
            let what = match (skill_part, mcp_part) {
                (Some(a), Some(b)) => format!("{a} + {b}"),
                (Some(a), None) => a,
                (None, Some(b)) => b,
                (None, None) => "No tools".to_string(),
            };
            let on = if self.machine == AUTO_MACHINE {
                "the machine Auto picks".to_string()
            } else {
                app.snap.label_of(&self.machine).unwrap_or_else(|| self.machine.clone())
            };
            let n = skills + mcps;
            row = row.child(
                ui::line(format!("{what} {} ready on {on}", if n == 1 { "is" } else { "are" }))
                    .flex_1()
                    .text_size(px(12.5))
                    .text_color(t.dim),
            );
        }
        Some(row.into_any_element())
    }

    /// Tools: every tool in the library as a check chip. The ones on for every new chat are checked and locked.
    fn tools_line(&self, t: &Theme, cx: &Context<Self>) -> Option<AnyElement> {
        let lib = self.app.read(cx).lib()?;
        let mut row = div().flex_1().min_w_0().flex().flex_wrap().items_center().gap(px(6.));
        if lib.tools.is_empty() {
            let open =
                ui::setup_btn(t, "nw-tools-lib", "Open Library").on_click(cx.listener(|_, _: &ClickEvent, _, cx| {
                    crate::settings::open_at(crate::settings::Section::Library, cx)
                }));
            return Some(
                ui::setup_note(t, "cube", "No tools in the library yet. Add one there to offer it to new chats.", open)
                    .into_any_element(),
            );
        }
        let mut locked_any = false;
        for tool in &lib.tools {
            let locked = lib.is_default(&tool.name);
            locked_any |= locked;
            let on = locked || self.tools.contains(&tool.name);
            let name = tool.name.clone();
            let ic = if tool.kinds.iter().any(|k| k == "mcp") { "cube" } else { "spark" };
            row = row.child(
                ui::check_chip(t, SharedString::from(format!("nw-tool-{}", tool.name)), on, locked, ic, &tool.name)
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        if locked {
                            return;
                        }
                        if !this.tools.contains(&name) {
                            this.tools.push(name.clone());
                        } else {
                            this.tools.retain(|x| x != &name);
                        }
                        cx.notify();
                    })),
            );
        }
        if locked_any {
            row =
                row.child(div().text_size(px(12.)).text_color(t.faint).child("Locked ones are on for every new chat."));
        }
        Some(row.into_any_element())
    }

    /// A word that acts, in the accent color.
    fn link(t: &Theme, id: impl Into<gpui::ElementId>, text: impl Into<SharedString>) -> gpui::Stateful<gpui::Div> {
        let accent = t.accent;
        div()
            .id(id)
            .flex_none()
            .cursor_pointer()
            .text_color(accent)
            .hover(move |s| s.text_color(accent.alpha(0.75)))
            .child(text.into())
    }

    /// An option that is on or off: a check box, an icon and its name.
    fn toggle_chip(t: &Theme, id: &'static str, on: bool, ic: &str, label: &str) -> gpui::Stateful<gpui::Div> {
        ui::check_chip(t, id, on, false, ic, label)
    }

    /// The setups in the header: the one used last, the saved presets, and Save.
    fn setups(&self, t: &Theme, focused: bool, cx: &Context<Self>) -> AnyElement {
        let Some(p) = &self.prefs else { return div().into_any_element() };
        let cur = self.current_setup(None);
        let blank = flags::setup(&Opts::default(), "", "normal", None);
        let chip = |id: SharedString, label: String, on: bool| {
            let (hover, accent) = (t.hover, t.accent);
            div()
                .id(id)
                .flex_none()
                .h(px(26.))
                .px(px(10.))
                .flex()
                .items_center()
                .rounded(px(ui::R_SHEET))
                .cursor_pointer()
                .border_1()
                .border_color(if on { accent.alpha(0.6) } else { t.line })
                .when(on, |d| d.bg(accent.alpha(if t.dark { 0.16 } else { 0.08 })).text_color(t.ink))
                .when(!on, |d| d.text_color(t.ink2).hover(move |s| s.bg(hover)))
                .text_size(px(12.))
                .child(label)
        };
        let mut row = div().flex_none().pt(px(2.)).flex().items_center().gap(px(6.));
        if self.naming {
            return row
                .child(
                    div()
                        .w(px(170.))
                        .h(px(28.))
                        .px(px(9.))
                        .flex()
                        .items_center()
                        .rounded(px(ui::R_CTRL))
                        .border_1()
                        .border_color(if focused { t.accent } else { t.line })
                        .text_size(px(12.5))
                        .text_color(t.ink)
                        .child(gpui_base::Input::new(&self.preset)),
                )
                .child(
                    ui::primary_btn(t, "nw-ps-save", 28.)
                        .px(px(10.))
                        .text_size(px(12.5))
                        .child("Save")
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.save_preset(cx))),
                )
                .child(
                    ui::ghost(t, "nw-ps-cancel", None, None)
                        .h(px(28.))
                        .px(px(8.))
                        .text_size(px(12.5))
                        .child("Cancel")
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.naming = false;
                            cx.notify();
                        })),
                )
                .into_any_element();
        }
        if let Some(last) = p.last.clone().filter(|l| !flags::same_setup(l, &blank)) {
            let on = flags::same_setup(&cur, &last);
            row = row.child(
                chip("nw-ps-last".into(), "Last used".into(), on)
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.apply_setup(&last, cx))),
            );
        }
        for (i, ps) in p.presets.iter().enumerate().take(6) {
            let on = flags::same_setup(&cur, ps);
            let ps2 = ps.clone();
            row = row.child(
                chip(
                    SharedString::from(format!("nw-ps-{i}")),
                    ps.name.clone().unwrap_or_else(|| format!("Setup {}", i + 1)),
                    on,
                )
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.apply_setup(&ps2, cx))),
            );
        }
        let saved = p.presets.iter().any(|x| flags::same_setup(&cur, x));
        if !saved && !flags::same_setup(&cur, &blank) {
            row = row.child(
                ui::ghost(t, "nw-ps-new", None, None)
                    .h(px(26.))
                    .px(px(8.))
                    .text_size(px(12.))
                    .child(ui::icon("plus", 12., t.dim))
                    .child("Save setup")
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                        this.naming = true;
                        this.preset.update(cx, |n, cx| {
                            n.set_value("", window, cx);
                            n.focus(window, cx);
                        });
                        cx.notify();
                    })),
            );
        }
        row.into_any_element()
    }

    /// The rows the first message box takes: from three lines up to `max`, or what its handle was dragged to.
    fn fit_message(&mut self, max: usize, cx: &mut Context<Self>) {
        let want = match self.msg_fixed {
            Some(r) => (r, r),
            None => (MSG_MIN_ROWS, max.max(MSG_MIN_ROWS)),
        };
        if want != self.msg_rows {
            self.msg_rows = want;
            self.msg.update(cx, |m, cx| m.set_auto_grow(want.0, want.1, cx));
        }
    }

    fn drag_message(&mut self, y: f32, cx: &mut Context<Self>) {
        if let Some((y0, r0)) = self.msg_drag {
            let r = (r0 as f32 + (y - y0) / MSG_LINE).round().clamp(2., 40.) as usize;
            if self.msg_fixed != Some(r) {
                self.msg_fixed = Some(r);
                cx.notify();
            }
        }
    }

    fn message_box(&self, t: &Theme, focused: bool, cx: &Context<Self>) -> impl IntoElement {
        let well = if t.dark { t.bg2 } else { t.bg };
        let hcell = self.msg_h.clone();
        let count = (self.msg_len >= MSG_COUNT_FROM).then(|| format!("{} characters", thousands(self.msg_len)));
        div()
            .flex_none()
            .flex()
            .flex_col()
            .gap(px(7.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(ui::lbl("First message", t.label))
                    .when_some(count, |d, c| {
                        d.child(div().font_family(NUM).text_size(px(11.)).text_color(t.faint).child(c))
                    }),
            )
            .child(
                div()
                    .relative()
                    .px(px(12.))
                    .pt(px(10.))
                    .pb(px(12.))
                    .rounded(px(ui::R_CARD))
                    .border_1()
                    .border_color(if focused { t.accent } else { t.line })
                    .when(focused, |d| d.shadow(vec![Theme::ring(t.accent)]))
                    .bg(well)
                    .text_size(px(13.5))
                    .line_height(relative(1.5))
                    .text_color(t.ink)
                    .child(gpui_base::Textarea::new(&self.msg))
                    .child(
                        canvas(move |b, _, _| hcell.set(f32::from(b.size.height)), |_, _, _, _| {})
                            .absolute()
                            .inset_0(),
                    )
                    // The handle: drag it to make the box taller or shorter; a double click lets it follow the text again.
                    .child(
                        div()
                            .id("nw-msg-grip")
                            .absolute()
                            .bottom_0()
                            .left(px(40.))
                            .right(px(40.))
                            .h(px(9.))
                            .flex()
                            .justify_center()
                            .items_center()
                            .cursor_ns_resize()
                            .child(div().w(px(28.)).h(px(3.)).rounded_full().bg(if self.msg_drag.is_some() {
                                t.accent
                            } else {
                                t.line2
                            }))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, e: &MouseDownEvent, _, cx| {
                                    cx.stop_propagation();
                                    if e.click_count >= 2 {
                                        this.msg_fixed = None;
                                        this.msg_drag = None;
                                    } else {
                                        let rows = ((this.msg_h.get() - MSG_PAD) / MSG_LINE).round().max(2.) as usize;
                                        this.msg_drag = Some((f32::from(e.position.y), rows));
                                    }
                                    cx.notify();
                                }),
                            ),
                    ),
            )
    }

    /// Claude in Chrome: where it runs, and this chat's own Chrome machine.
    fn chrome_line(&self, t: &Theme, cx: &Context<Self>) -> Option<AnyElement> {
        if !self.opt.chrome {
            return None;
        }
        let snap = &self.app.read(cx).snap;
        let pref = self.chrome_machine.as_deref().and_then(|n| snap.machine(n));
        let up = |m: &sb_core::Machine| !m.is_offline();
        let has = |m: &sb_core::Machine| m.chrome.as_ref().is_some_and(|c| c.connector);
        let mut text = String::new();
        let mut use_to: Option<(String, String)> = None;
        if self.is_auto() {
            text = match pref {
                Some(p) => format!("Auto sends this chat to {}, its Chrome machine.", p.label),
                None => "Auto sends it to a machine with Claude in Chrome.".into(),
            };
        } else if let Some(m) = snap.machine(&self.machine) {
            let alt = pref
                .filter(|p| up(p) && p.name != m.name)
                .or_else(|| snap.machines.iter().find(|x| x.name != m.name && up(x) && has(x)));
            match &m.chrome {
                Some(c) if !c.connector => {
                    text = format!(
                        "{} does not have Claude in Chrome{}.",
                        m.label,
                        if c.installed { " (Chrome is there, the extension is not)" } else { "" }
                    );
                    match alt {
                        Some(a) => use_to = Some((a.name.clone(), a.label.clone())),
                        None => text.push_str(" No machine online has it right now."),
                    }
                }
                _ => match pref {
                    Some(p) if p.name != m.name && up(p) => {
                        text = format!("{} is this chat's Chrome machine.", p.label);
                        use_to = Some((p.name.clone(), p.label.clone()));
                    }
                    Some(p) if p.name == m.name => text = format!("{} is this chat's Chrome machine.", m.label),
                    _ if has(m) => text = format!("Claude in Chrome on {}.", m.label),
                    _ => {}
                },
            }
        }
        // No machine online has Claude in Chrome: the shared not-set-up note, pointing at the Chrome settings.
        if !snap.machines.iter().any(|x| up(x) && has(x)) && pref.is_none() {
            let open =
                ui::setup_btn(t, "nw-chrome-set", "Open Settings").on_click(cx.listener(|_, _: &ClickEvent, _, cx| {
                    crate::settings::open_at(crate::settings::Section::Chrome, cx)
                }));
            return Some(
                ui::setup_note(
                    t,
                    "globe",
                    "No machine online has Claude in Chrome yet. Settings shows which machines have it.",
                    open,
                )
                .into_any_element(),
            );
        }
        let mut row = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(8.))
            .text_size(px(12.))
            .text_color(t.dim)
            .child(ui::icon("globe", 13., t.faint));
        if !text.is_empty() {
            row = row.child(text);
        }
        if let Some((name, label)) = use_to {
            row = row.child(Self::link(t, "nw-chrome-use", format!("Use {label}")).on_click(cx.listener(
                move |this, _: &ClickEvent, _, cx| {
                    this.chrome_machine = Some(name.clone());
                    this.set_machine(name.clone(), cx)
                },
            )));
        }
        row = row.child(
            Self::link(t, "nw-chrome-set", if self.chrome_set { "Done" } else { "Change" }).text_color(t.dim).on_click(
                cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.chrome_set = !this.chrome_set;
                    cx.notify();
                }),
            ),
        );
        if self.chrome_set {
            let mut keys = div()
                .w_full()
                .flex()
                .flex_wrap()
                .items_center()
                .gap(px(6.))
                .child(div().text_color(t.faint).child("This chat's Chrome machine:"));
            for m in &snap.machines {
                let on = self.chrome_machine.as_deref() == Some(m.name.as_str());
                let name = m.name.clone();
                keys = keys.child(
                    ui::seg_btn(t, SharedString::from(format!("nw-cpref-{}", m.name)), on, false)
                        .h_auto()
                        .min_h(px(24.))
                        .py(px(3.))
                        .rounded(px(ui::R_CTRL))
                        .border_1()
                        .border_color(if on { t.accent.alpha(0.6) } else { t.line })
                        .child(ui::wrap(m.label.clone()).max_w(px(200.)).line_height(relative(1.25)))
                        .when(!has(m), |d| d.child(div().text_color(t.faint).child("(no extension)")))
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            // only this chat: tap again to clear; a picked machine also runs it unless Auto picks
                            if this.chrome_machine.as_deref() == Some(name.as_str()) {
                                this.chrome_machine = None;
                            } else {
                                this.chrome_machine = Some(name.clone());
                                if !this.is_auto() {
                                    this.set_machine(name.clone(), cx);
                                }
                            }
                            this.prev_q = None;
                            this.preview(cx);
                            cx.notify();
                        })),
                );
            }
            row = row.child(keys);
        }
        Some(row.into_any_element())
    }

    /// The options band under the message and the folder: effort, Chrome, permissions, extra folders, Other
    /// flags, and the command the machine runs.
    fn options(&self, t: &Theme, other_focused: bool, cx: &Context<Self>) -> impl IntoElement {
        let label =
            |text: &str| div().flex_none().w(px(84.)).text_size(px(12.)).text_color(t.label).child(text.to_string());
        let locked = flags::effort_locked(self.mode);
        // Five choices each: dropdowns (two to four short choices are segmented buttons).
        let efforts = ui::choice(
            t,
            cx,
            "nw-effort",
            self.drop.is_open("nw-effort"),
            &EFFORTS,
            &self.opt.effort,
            locked,
            None,
            |v| &mut v.drop,
            |v, e, _| {
                v.opt.effort = e.to_string();
                v.touched = true;
            },
        );
        let perms = ui::choice(
            t,
            cx,
            "nw-perm",
            self.drop.is_open("nw-perm"),
            &PERMS,
            &self.opt.perm,
            false,
            Some("bypassPermissions"),
            |v| &mut v.drop,
            |v, p, _| {
                v.opt.perm = p.to_string();
                v.touched = true;
            },
        );
        let chrome = Self::toggle_chip(t, "nw-chrome", self.opt.chrome, "globe", "Claude in Chrome").on_click(
            cx.listener(|this, _: &ClickEvent, _, cx| {
                this.opt.chrome = !this.opt.chrome;
                this.touched = true;
                this.preview(cx);
                cx.notify();
            }),
        );
        let mut dirs = div().flex_1().min_w_0().flex().flex_wrap().items_center().gap(px(6.));
        for d in &self.opt.dirs {
            let d2 = d.clone();
            dirs = dirs.child(
                div()
                    .flex_none()
                    .h(px(26.))
                    .pl(px(9.))
                    .pr(px(3.))
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .rounded(px(ui::R_CTRL))
                    .bg(t.chip_bg)
                    .text_size(px(12.))
                    .text_color(t.chip_ink)
                    .child(ui::icon("folder", 12., t.dim))
                    .child(fmt::last_seg(d))
                    .child(
                        ui::icon_btn(t, SharedString::from(format!("nw-dir-x-{d}")), "x", 20.)
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.toggle_dir(d2.clone(), cx))),
                    ),
            );
        }
        let auto = self.is_auto();
        if !auto {
            dirs = dirs.child(
                ui::ghost(t, "nw-dirs", None, None)
                    .h(px(26.))
                    .px(px(8.))
                    .text_size(px(12.))
                    .when(self.adding, |d| d.bg(t.sel))
                    .child(ui::icon("plus", 12., t.dim))
                    .child(if self.opt.dirs.is_empty() { "Add a folder" } else { "Add" })
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.adding = !this.adding;
                        cx.notify();
                    })),
            );
        } else if self.opt.dirs.is_empty() {
            dirs = dirs.child(div().text_size(px(12.)).text_color(t.faint).child("Pick a machine to add folders."));
        }
        let err = self.other_error();
        let well = if t.dark { t.bg2 } else { t.bg };
        let other = div()
            .flex_1()
            .min_w_0()
            .h(px(30.))
            .px(px(10.))
            .flex()
            .items_center()
            .rounded(px(ui::R_CTRL))
            .border_1()
            .border_color(if err.is_some() {
                t.bad
            } else if other_focused {
                t.accent
            } else {
                t.line
            })
            .bg(well)
            .font_family(NUM)
            .text_size(px(12.))
            .text_color(t.ink)
            .child(gpui_base::Input::new(&self.other));
        let cmd = flags::cmd_text(&self.opt, if self.locked() { "opus" } else { self.model }, self.mode);
        div()
            .flex_none()
            .px(px(22.))
            .pt(px(12.))
            .pb(px(10.))
            .flex()
            .flex_col()
            .gap(px(9.))
            .border_t_1()
            .border_color(t.rule)
            .child(div().flex().items_center().gap(px(10.)).child(label("Browser")).child(chrome))
            .children(self.chrome_line(t, cx).map(|l| div().pl(px(94.)).child(l)))
            .children(
                self.tools_mode_line(t, cx)
                    .map(|l| div().flex().items_center().gap(px(10.)).child(label("Tools")).child(l)),
            )
            .children(
                self.tools_line(t, cx)
                    .filter(|_| self.tools_pick || !self.app.read(cx).lib().is_some_and(|l| l.has_auto()))
                    .map(|l| {
                        let auto = self.app.read(cx).lib().is_some_and(|l| l.has_auto());
                        div()
                            .flex()
                            .items_start()
                            .gap(px(10.))
                            .child(if auto { label("").pt(px(7.)) } else { label("Tools").pt(px(7.)) })
                            .child(l)
                    }),
            )
            .child(div().flex().items_center().gap(px(10.)).child(label("Permissions")).child(perms))
            .child(div().flex().items_center().gap(px(10.)).child(label("Effort")).child(efforts).when(locked, |d| {
                d.child(div().text_size(px(12.)).text_color(t.faint).child(if self.mode == "max" {
                    "Max sets it."
                } else {
                    "UltraCode sets it."
                }))
            }))
            .child(div().flex().items_center().gap(px(10.)).child(label("Folders")).child(dirs))
            .child(div().flex().items_center().gap(px(10.)).child(label("Other flags")).child(other))
            .when_some(err, |d, e| d.child(div().pl(px(94.)).text_size(px(12.)).text_color(t.bad).child(e)))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .pl(px(94.))
                    .font_family(NUM)
                    .text_size(px(11.5))
                    .text_color(t.faint)
                    .child(ui::icon("term", 12., t.faint))
                    .child(ui::wrap(cmd).flex_1()),
            )
    }

    fn field(t: &Theme, label: &str, body: impl IntoElement) -> gpui::Div {
        div().flex().flex_col().items_start().gap(px(7.)).child(ui::lbl(label, t.label)).child(body)
    }

    fn folder_row(&self, t: &Theme, f: &ApiFolder, depth: usize, expander: bool, cx: &Context<Self>) -> AnyElement {
        let added = self.adding && self.opt.dirs.contains(&f.folder);
        let on = if self.adding { added } else { f.folder == self.folder };
        let accent = t.accent;
        let name = f.name.clone().filter(|n| !n.is_empty()).unwrap_or_else(|| fmt::last_seg(&f.folder));
        let open = self.open.contains(&f.folder);
        let (pick, tog) = (f.folder.clone(), f.folder.clone());
        let hover = t.hover;
        let mut row = div().pl(px(depth as f32 * 16.)).flex().items_center().gap(px(2.));
        row = if expander && f.kids == Some(true) {
            row.child(
                div()
                    .id(SharedString::from(format!("fx-{}", f.folder)))
                    .flex_none()
                    .w(px(24.))
                    .h(px(30.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(ui::R_ITEM))
                    .cursor_pointer()
                    .child(ui::icon(if open { "down" } else { "chev" }, 13., t.faint))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.toggle(tog.clone(), cx))),
            )
        } else {
            row.child(div().flex_none().w(px(24.)))
        };
        row.child(
            div()
                .id(SharedString::from(format!("fr-{}-{}", depth, f.folder)))
                .flex_1()
                .min_w_0()
                .h(px(30.))
                .px(px(10.))
                .flex()
                .items_center()
                .gap(px(10.))
                .rounded(px(ui::R_CTRL))
                .cursor_pointer()
                .text_size(px(13.))
                .text_color(if on { t.ink } else { t.ink2 })
                .when(on, |d| d.bg(t.sel))
                .when(!on, |d| d.hover(move |s| s.bg(hover)))
                .child(ui::line(name).flex_1())
                .when_some(f.recent.filter(|r| *r > 0.0 && !self.adding), |d, r| {
                    d.child(
                        div()
                            .flex_none()
                            .font_family(NUM)
                            .text_size(px(11.5))
                            .text_color(t.faint)
                            .child(fmt::sago(r, AppState::now())),
                    )
                })
                .when(added, |d| d.child(ui::icon("check", 13., accent)))
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    if this.adding {
                        return this.toggle_dir(pick.clone(), cx);
                    }
                    this.folder = pick.clone();
                    cx.notify();
                })),
        )
        .into_any_element()
    }

    /// Auto's line under the machine row: the rules' pick for what is typed so far.
    fn preview_line(&self, t: &Theme, cx: &Context<Self>) -> impl IntoElement {
        use sb_core::TextRun;
        let run = |text: &str, bold: bool| TextRun { text: text.to_string(), bold };
        let body = match &self.prev {
            None => div().text_color(t.faint).child("Finding the best fit..."),
            Some(Preview::Off(e)) => div().text_color(t.bad).child(e.clone()),
            Some(Preview::Pick { label, why, typed }) => {
                let why = lc_first(why);
                let mut runs = vec![run("Auto: ", false), run(label, true)];
                if !why.is_empty() {
                    runs.push(run(&format!(", {why}"), false));
                }
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(div().text_color(t.ink2).child(ui::runs_text(&runs, t.ink)))
                    .child(if self.rules_pick() {
                        div()
                            .flex()
                            .flex_col()
                            .items_start()
                            .gap(px(2.))
                            .text_size(px(11.5))
                            .text_color(t.faint)
                            .child(ui::wrap(rules_note()))
                            .child(ui::link(t, "nw-ai-auto", "Open Settings").on_click(cx.listener(
                                |_, _: &ClickEvent, _, cx| {
                                    crate::settings::open_at(crate::settings::Section::Privacy, cx)
                                },
                            )))
                    } else {
                        div().text_size(px(11.5)).text_color(t.faint).child(if *typed {
                            "A preview. Claude makes the final pick when you start."
                        } else {
                            "Type the first message and the pick follows it."
                        })
                    })
            }
        };
        div()
            .w_full()
            .flex()
            .items_start()
            .gap(px(7.))
            .text_size(px(12.))
            .line_height(relative(1.4))
            .child(div().flex_none().pt(px(2.)).child(ui::icon("auto", 13., t.accent)))
            .child(body)
    }

    /// Auto's folder panel: the folder comes with the machine unless one is typed.
    fn auto_note(&self, t: &Theme) -> impl IntoElement {
        use sb_core::TextRun;
        div()
            .flex_1()
            .min_h_0()
            .px(px(16.))
            .py(px(16.))
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(
                div()
                    .flex_none()
                    .size(px(32.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(ui::R_CARD))
                    .bg(t.accent.alpha(if t.dark { 0.16 } else { 0.10 }))
                    .child(ui::icon("auto", 17., t.accent)),
            )
            .child(div().text_size(px(12.5)).line_height(relative(1.5)).text_color(t.ink2).child(ui::runs_text(
                &[
                    TextRun { text: "Picked with the machine.".into(), bold: true },
                    TextRun {
                        text: " A repo the message names, when that machine has it; otherwise its Claude folder. Type a folder below to choose it yourself."
                            .into(),
                        bold: false,
                    },
                ],
                t.ink,
            )))
    }

    fn tree_rows(&self, t: &Theme, parent: &str, depth: usize, out: &mut Vec<AnyElement>, cx: &Context<Self>) {
        let note = |text: &str, depth: usize| {
            div()
                .ml(px(26. + depth as f32 * 16.))
                .my(px(6.))
                .text_size(px(12.5))
                .text_color(t.faint)
                .child(text.to_string())
                .into_any_element()
        };
        match self.tree.get(parent) {
            Some(list) if list.is_empty() => {
                out.push(note(if parent.is_empty() { "No folders here yet." } else { "No folders inside" }, depth))
            }
            Some(list) => {
                for f in list {
                    out.push(self.folder_row(t, f, depth, true, cx));
                    if self.open.contains(&f.folder) {
                        self.tree_rows(t, &f.folder, depth + 1, out, cx);
                    }
                }
            }
            None if self.failed.contains(parent) => {
                out.push(note("Could not load the folders. It tries again next time.", depth))
            }
            None => {}
        }
    }

    fn picker(&self, t: &Theme, cx: &Context<Self>) -> impl IntoElement {
        let label = self.label(cx);
        let hover = t.hover;
        let on = self.folder.is_empty() && !self.adding;
        let group =
            |text: &str| div().mt(px(12.)).mb(px(4.)).ml(px(26.)).child(ui::lbl(text, t.faint)).into_any_element();
        let mut rows: Vec<AnyElement> = vec![
            div()
                .flex()
                .items_center()
                .gap(px(2.))
                .child(
                    div()
                        .flex_none()
                        .w(px(24.))
                        .h(px(30.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(ui::icon("home", 15., t.faint)),
                )
                .child(
                    div()
                        .id("fr-home")
                        .flex_1()
                        .min_w_0()
                        .min_h(px(30.))
                        .py(px(5.))
                        .px(px(10.))
                        .flex()
                        .items_center()
                        .gap(px(10.))
                        .rounded(px(ui::R_CTRL))
                        .cursor_pointer()
                        .text_size(px(13.))
                        .text_color(if on { t.ink } else { t.ink2 })
                        .when(on, |d| d.bg(t.sel))
                        .when(!on, |d| d.hover(move |s| s.bg(hover)))
                        // The name never truncates; the path beside it gives way first.
                        .justify_between()
                        .child(ui::wrap(format!("{label} Claude folder")).flex_shrink(1.).line_height(relative(1.3)))
                        .when_some(self.root.clone(), |d, r| {
                            d.child(
                                div()
                                    .flex_shrink(1.)
                                    .min_w_0()
                                    .max_w(px(120.))
                                    .font_family(NUM)
                                    .text_size(px(11.))
                                    .text_color(t.faint)
                                    .child(ui::line(r)),
                            )
                        })
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            if this.adding {
                                return;
                            }
                            this.folder.clear();
                            cx.notify();
                        })),
                )
                .into_any_element(),
        ];
        let recent: Vec<&ApiFolder> = self.recent.iter().filter(|f| !f.folder.is_empty()).take(3).collect();
        if !recent.is_empty() {
            rows.push(group("Recent"));
            for f in recent {
                let mut f = f.clone();
                f.name = Some(f.folder.clone());
                f.kids = None;
                rows.push(self.folder_row(t, &f, 0, false, cx));
            }
        }
        rows.push(group("All folders"));
        self.tree_rows(t, "", 0, &mut rows, cx);
        div()
            .id("fpick")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .px(px(6.))
            .pt(px(4.))
            .pb(px(8.))
            .children(rows)
    }
}

impl Render for NewSheet {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = Theme::get(cx);
        let h = f32::from(window.viewport_size().height);
        let sheet_max = (h - ui::header() - 24.).max(360.);
        self.keep_draft(cx);
        // The message box grows until the rest of the sheet would no longer fit beside it, then scrolls.
        self.fit_message((((sheet_max - 560.) / MSG_LINE) as usize).clamp(MSG_MIN_ROWS, 14), cx);
        if let Some(v) = self.pending_other.take() {
            self.other.update(cx, |n, cx| n.set_value(v, window, cx));
        }
        let other_focused = self.other.focus_handle(cx).is_focused(window);
        let preset_focused = self.preset.focus_handle(cx).is_focused(window);
        let snap = &self.app.read(cx).snap;
        let well = if t.dark { t.bg2 } else { t.bg };
        let locked = self.locked();

        // Machine: Auto first, while some machine online shares what it can do.
        // Many machines or long names wrap onto more rows instead of running under the folder column.
        let mut machines = ui::seg_box(&t).flex_shrink(1.).max_w_full().flex_wrap();
        let auto_ready = snap.auto_ready();
        if auto_ready || self.is_auto() {
            let on = self.is_auto();
            let b = ui::seg_btn(&t, "nm-auto", on, !auto_ready)
                .child(ui::icon("auto", 13., if on { t.accent } else { t.dim }))
                .child("Auto");
            machines = machines.child(if auto_ready {
                b.on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.pick_auto(cx))).into_any_element()
            } else {
                b.into_any_element()
            });
        }
        for m in &snap.machines {
            let off = m.is_offline();
            let name = m.name.clone();
            let b = ui::seg_btn(&t, SharedString::from(format!("nm-{}", m.name)), m.name == self.machine, off)
                .min_w_0()
                .h_auto()
                .min_h(px(26.))
                .py(px(3.))
                .child(div().flex_none().size(px(6.)).rounded_full().bg(t.machine(m.lamp())))
                .child(ui::wrap(m.label.clone()).max_w(px(200.)).line_height(relative(1.25)));
            machines = machines.child(if off {
                b.into_any_element()
            } else {
                b.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.set_machine(name.clone(), cx)))
                    .into_any_element()
            });
        }
        // Mode
        let mut modes = ui::seg_box(&t);
        for (v, label, _) in MODES {
            let on = self.mode == v;
            modes = modes.child(
                ui::seg_btn(&t, SharedString::from(format!("mode-{v}")), on, false)
                    .when(on && v == "ultracode", |d| d.text_color(t.cold))
                    .child(label)
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.set_mode(v, cx))),
            );
        }
        let mode_hint = MODES.iter().find(|m| m.0 == self.mode).map(|m| m.2).unwrap_or("");
        // Model
        let mut models = ui::seg_box(&t);
        for (v, label) in MODELS {
            let on = if locked { v == "opus" } else { self.model == v };
            let dis = locked && v != "opus";
            let b = ui::seg_btn(&t, SharedString::from(format!("model-{v}")), on, dis)
                .when(locked && v == "opus", |d| d.child(ui::icon("lock", 13., t.cold)))
                .child(label);
            models = models.child(if dis {
                b.into_any_element()
            } else {
                b.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.model = v;
                    cx.notify();
                }))
                .into_any_element()
            });
        }
        let warn = if locked { ultra_warn(&snap.plan, AppState::now()) } else { None };
        let ultra = locked.then(|| {
            div()
                .mt(px(-4.))
                .px(px(11.))
                .py(px(9.))
                .rounded(px(ui::R_CARD))
                .bg(t.cold.alpha(0.09))
                .flex()
                .flex_col()
                .gap(px(5.))
                .text_size(px(12.))
                .line_height(relative(1.45))
                .text_color(t.ink2)
                .child(ULTRA_NOTE)
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .text_color(t.cold)
                        .child(ui::icon("lock", 13., t.cold))
                        .child("The model is locked to Opus."),
                )
                .when_some(warn, |d, w| d.child(div().text_color(t.cold).child(w)))
        });

        let path = match self.target(cx) {
            _ if self.is_auto() => match self.target(cx) {
                Some((f, _)) if !f.is_empty() => f,
                _ => "Picked with the machine".to_string(),
            },
            Some((f, _)) if f.is_empty() => self.root_text(),
            Some((f, _)) => match &self.root {
                Some(r) => format!("{r}/{f}"),
                None => f,
            },
            None => self.root_text(),
        };
        let auto = self.is_auto();
        if self.newf_auto != auto {
            // A name typed for one kind of pick means something else for the other: start the field empty.
            self.newf_auto = auto;
            self.newf.update(cx, |n, cx| n.set_value("", window, cx));
        }
        let bad_name = matches!(self.new_name(cx), Some(Err(())));
        let place = if self.folder.is_empty() { self.root_text() } else { self.folder.clone() };
        let ph =
            if auto { "Folder (optional), like switchboard".to_string() } else { format!("New folder in {place}") };
        if self.newf_ph != ph {
            self.newf_ph = ph.clone();
            self.newf.update(cx, |n, cx| n.set_placeholder(ph, window, cx));
        }
        let name = self.name(cx);
        let (topic, tag) = names::split(&name);
        let where_text = self.where_text(cx);
        let can = self.can_start(cx);
        let msg_focused = self.msg.focus_handle(cx).is_focused(window);
        let newf_focused = self.newf.focus_handle(cx).is_focused(window);

        let left = div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(14.))
            .child(Self::field(&t, "Machine", machines).when(auto, |d| d.child(self.preview_line(&t, cx))))
            .child(Self::field(&t, "Mode", modes).when(!locked, |d| {
                d.child(div().text_size(px(12.)).line_height(relative(1.35)).text_color(t.faint).child(mode_hint))
            }))
            .children(ultra)
            .child(Self::field(&t, "Model", models))
            .child(self.message_box(&t, msg_focused, cx));

        // The folder column is as tall as the left one (it is drawn over its own empty box, so its list never sets
        // the height), and never shorter than a few folders.
        let right = div()
            .absolute()
            .inset_0()
            .flex()
            .flex_col()
            .gap(px(7.))
            .child(ui::lbl("Folder", t.label))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .rounded(px(ui::R_CARD))
                    .border_1()
                    .border_color(t.line)
                    .bg(well)
                    .overflow_hidden()
                    .child(if self.adding && !auto {
                        div()
                            .flex_none()
                            .h(px(34.))
                            .px(px(12.))
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .border_b_1()
                            .border_color(t.rule)
                            .bg(t.accent.alpha(if t.dark { 0.14 } else { 0.07 }))
                            .text_size(px(12.))
                            .text_color(t.ink)
                            .child(ui::icon("plus", 13., t.accent))
                            .child(ui::line("Folders Claude may also use").flex_1())
                            .child(Self::link(&t, "nw-dirs-done", "Done").on_click(cx.listener(
                                |this, _: &ClickEvent, _, cx| {
                                    this.adding = false;
                                    cx.notify();
                                },
                            )))
                    } else {
                        div()
                            .flex_none()
                            .h(px(34.))
                            .px(px(12.))
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .border_b_1()
                            .border_color(t.rule)
                            .font_family(NUM)
                            .text_size(px(11.5))
                            .text_color(t.ink2)
                            .child(if auto { ui::icon("auto", 14., t.accent) } else { ui::icon("folder", 14., t.dim) })
                            .child(ui::wrap(path).flex_1())
                    })
                    .child(if auto {
                        self.auto_note(&t).into_any_element()
                    } else {
                        self.picker(&t, cx).into_any_element()
                    }),
            )
            .child(
                div()
                    .flex_none()
                    .h(px(30.))
                    .px(px(10.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .rounded(px(ui::R_CTRL))
                    .border_1()
                    .border_color(if bad_name {
                        t.bad
                    } else if newf_focused {
                        t.accent
                    } else {
                        t.line
                    })
                    .bg(well)
                    .text_size(px(12.5))
                    .text_color(t.ink)
                    .child(ui::icon(if auto { "folder" } else { "plus" }, 13., t.faint))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .flex()
                            .items_center()
                            .child(gpui_base::Input::new(&self.newf)),
                    ),
            )
            .when(bad_name, |d| {
                d.child(
                    div()
                        .text_size(px(11.5))
                        .text_color(t.bad)
                        .child("Use letters, numbers, spaces, dots, dashes or underscores."),
                )
            });
        let right = div().relative().w(px(300.)).flex_none().min_h(px(300.)).child(right);

        let start = ui::primary_btn(&t, "nw-go", 32.)
            .px(px(12.))
            .child(ui::icon("send", 15., t.on_accent))
            .child(if self.sending { "Starting" } else { "Start" })
            .child(ui::kbd_in(&ui::mod_cap("↵"), t.on_accent.alpha(0.75), t.on_accent.alpha(0.35)))
            .when(!can, |d| d.opacity(0.5).cursor_default())
            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.start(window, cx)));

        div()
            .key_context("NewSheet")
            .on_action(cx.listener(|this, _: &crate::view::SwapField, window, cx| this.swap_field(window, cx)))
            .max_h(px(sheet_max))
            .flex()
            .flex_col()
            .when(self.msg_drag.is_some(), |d| {
                d.cursor_ns_resize()
                    .on_mouse_move(cx.listener(|this, e: &MouseMoveEvent, _, cx| {
                        if e.pressed_button == Some(MouseButton::Left) {
                            this.drag_message(f32::from(e.position.y), cx);
                        } else {
                            this.msg_drag = None;
                            cx.notify();
                        }
                    }))
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(|this, _: &MouseUpEvent, _, cx| {
                            this.msg_drag = None;
                            cx.notify();
                        }),
                    )
                    .on_mouse_up_out(
                        MouseButton::Left,
                        cx.listener(|this, _: &MouseUpEvent, _, cx| {
                            this.msg_drag = None;
                            cx.notify();
                        }),
                    )
            })
            .child(
                div()
                    .flex_none()
                    .pt(px(16.))
                    .pr(px(12.))
                    .pb(px(6.))
                    .pl(px(22.))
                    .flex()
                    .items_start()
                    .gap(px(12.))
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .gap(px(4.))
                            .child(
                                div()
                                    .text_size(px(15.))
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(t.ink)
                                    .child("New session"),
                            )
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(t.faint)
                                    .child("Runs in the background on that machine, with a link to open it anywhere."),
                            ),
                    )
                    .child(self.setups(&t, preset_focused, cx))
                    .child(
                        ui::icon_btn(&t, "nw-close", "x", 26.)
                            .on_click(cx.listener(|_, _: &ClickEvent, _, cx| cx.emit(SheetEvent::Close))),
                    ),
            )
            .child(
                div()
                    .relative()
                    .flex_shrink(1.)
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .id("nw-body")
                            .flex_shrink(1.)
                            .min_h_0()
                            .overflow_y_scroll()
                            .track_scroll(&self.body_scroll)
                            .child(
                                div().flex().gap(px(22.)).pt(px(10.)).px(px(22.)).pb(px(14.)).child(left).child(right),
                            )
                            .child(self.options(&t, other_focused, cx)),
                    )
                    .child(div().absolute().inset_0().child(gpui_base::Scrollbar::vertical(&self.body_scroll))),
            )
            .child(
                div()
                    .flex_none()
                    .py(px(12.))
                    .pr(px(14.))
                    .pl(px(22.))
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .border_t_1()
                    .border_color(t.rule)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(4.))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(6.))
                                    .min_w_0()
                                    .child(
                                        ui::wrap(topic)
                                            .text_size(px(13.))
                                            .font_weight(FontWeight::BOLD)
                                            .text_color(t.ink),
                                    )
                                    .when_some(tag, |d, tg| d.child(ui::tag(&t, &tg))),
                            )
                            .child(match &self.error {
                                Some(e) => div()
                                    .flex()
                                    .items_center()
                                    .gap(px(6.))
                                    .min_w_0()
                                    .child(ui::icon("x", 13., t.bad))
                                    .child(
                                        ui::wrap(format!("Did not start. {e}")).text_size(px(12.)).text_color(t.bad),
                                    ),
                                None => ui::wrap(where_text).text_size(px(12.)).text_color(if bad_name {
                                    t.bad
                                } else {
                                    t.dim
                                }),
                            }),
                    )
                    .child(self.term_toggle(&t, cx))
                    .child(
                        ui::line_btn(&t, "nw-cancel", 32.)
                            .px(px(12.))
                            .text_size(px(13.))
                            .child("Cancel")
                            .on_click(cx.listener(|_, _: &ClickEvent, _, cx| cx.emit(SheetEvent::Close))),
                    )
                    .child(start),
            )
    }
}

/// What Auto says when its pick came from its rules, because Let Claude pick for Auto is off.
pub fn rules_note() -> &'static str {
    "Picked by rules. Turn on Let Claude pick for Auto in Settings to have Claude choose."
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_draft_comes_back_until_it_starts_or_a_week_passes() {
        let dir = std::env::temp_dir().join(format!("sb-draft-{}", std::process::id()));
        let p = dir.join("new-draft.json");
        let d = Draft {
            msg: "Fix the build 🚀 日本語".into(),
            machine: "studio".into(),
            saved: 1000.,
            ..Default::default()
        };
        d.save(&p);
        assert_eq!(Draft::load(&p, 1000. + 3600.).as_ref(), Some(&d));
        assert_eq!(Draft::load(&p, 1000. + DRAFT_KEEP_S + 1.), None);
        // nothing typed: nothing kept
        Draft { msg: "  ".into(), ..d.clone() }.save(&p);
        assert!(!p.exists());
        Draft::default().save(&p);
        assert_eq!(thousands(52340), "52,340");
        assert_eq!(thousands(999), "999");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
