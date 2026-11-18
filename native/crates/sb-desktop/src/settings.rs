//! Settings (Cmd+Comma): Appearance (light, system or dark), Terminal (scheme, text size, cursor, blink), Chats
//! (who writes summaries, saved setups, when an idle chat is stopped), Machines (rename, reorder, remove, add one over SSH or with Add a machine) with
//! a page for each machine (its Claude folder, picked by browsing it; its versions; what Auto may count on it for; its
//! MCP servers, each one a click from the library), Library (settings_library.rs), Notifications and the Chrome
//! machine. Most of it lives on the server, so every device agrees; what is kept on this computer alone (the default
//! terminal, the terminal's text size, Stop a chat when its tab closes, this computer's notifications, the server's
//! addresses) says "This computer only".

use gpui::{
    AnyElement, App, AppContext, ClickEvent, Context, Entity, FocusHandle, Focusable, FontWeight, InteractiveElement,
    IntoElement, ParentElement, Render, ScrollHandle, SharedString, StatefulInteractiveElement, Styled, Subscription,
    Window, div, prelude::*, px, relative,
};
use gpui_base::input::{InputEvent, InputState, TextareaState};
use sb_core::api::{ApiCapsView, ApiMachineRow, ApiMachines, ApiMcp, ApiRootDirs};
use std::collections::{HashMap, HashSet};

#[path = "settings_library.rs"]
mod library_ui;
#[path = "settings_server.rs"]
mod server_ui;

use crate::settings_context::ContextPage;
use crate::state::AppState;
use crate::term::colors;
use crate::theme::{Mode, NUM, Theme};
use crate::ui;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Section {
    Appearance,
    Terminal,
    NewSessions,
    /// Compact long chats at: its own page (settings_context.rs).
    Context,
    Machines,
    Library,
    Notifications,
    Chrome,
    Budget,
    Updates,
    Server,
    Privacy,
}

impl Section {
    /// Its name in the sidebar, and its icon.
    fn nav(self) -> (&'static str, &'static str) {
        match self {
            Section::Appearance => ("Appearance", "sun"),
            Section::Terminal => ("Terminal", "term"),
            Section::NewSessions => ("Chats", "send"),
            Section::Context => ("Context window", "mem"),
            Section::Machines => ("Machines", "mac"),
            Section::Library => ("Library", "cube"),
            Section::Notifications => ("Notifications", "bolt"),
            Section::Chrome => ("Chrome", "globe"),
            Section::Budget => ("Budget", "gauge"),
            Section::Updates => ("Updates", "update"),
            Section::Server => ("Server", "tower"),
            Section::Privacy => ("AI and your Claude setup", "spark"),
        }
    }
}

/// The sidebar, top to bottom: what belongs together sits under one label.
const NAV: [(&str, &[Section]); 4] = [
    ("General", &[Section::Appearance, Section::Notifications]),
    ("Chats", &[Section::NewSessions, Section::Context, Section::Terminal, Section::Budget]),
    ("Claude", &[Section::Library, Section::Chrome, Section::Privacy]),
    ("Machines", &[Section::Machines, Section::Server]),
];
/// Pinned to the sidebar's bottom edge, about the app itself. About comes after these: it is its own window
/// (about.rs), also at Switchboard > About Switchboard.
const NAV_FOOT: [Section; 1] = [Section::Updates];

/// Stop idle chats: the waits offered, in hours (0 is never).
pub(crate) const IDLE_STOP_CHOICES: [(f64, &str); 4] =
    [(0., "Never"), (2., "2 hours"), (6., "6 hours"), (24., "1 day")];
const IDLE_STOP_NOTE: &str = "Every computer. A chat that finished its reply and has been left alone this long is stopped to free its memory, and Resume brings it back where it was. Its cache has run out by then, so resuming costs what replying to it would have. Never a chat that is working, waiting on you, running a command or a scheduled task, starred, set to Keep running, or open in a tab.";

/// A "Click again to ..." button stays armed this long (seconds), then goes back to its first label.
pub(crate) const ARM_FOR: f64 = 4.;
/// The second click counts as the yes only after this long: sooner, it is the tail of a double-click.
const ARM_WAIT: f64 = 0.4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ArmClick {
    /// Not armed (or no longer): this click arms it.
    Arm,
    /// Armed a moment ago: ignored.
    Wait,
    /// The confirming click.
    Go,
}

/// What a click does on a button armed at `armed_at` (None: not armed).
pub(crate) fn arm_click(armed_at: Option<f64>, now: f64) -> ArmClick {
    match armed_at.map(|at| now - at) {
        Some(d) if (0.0..ARM_WAIT).contains(&d) => ArmClick::Wait,
        Some(d) if (ARM_WAIT..ARM_FOR).contains(&d) => ArmClick::Go,
        _ => ArmClick::Arm,
    }
}

/// A budget stepper's next values down and up from `x`: the nearest multiples of `step` on either side, so a
/// default like 14.3 goes to 14 or 15 and round numbers can be reached. Never under 1 (or under `x`, when that is
/// already smaller) nor over 100.
fn pct_steps(x: f64, step: f64) -> (f64, f64) {
    let r = |v: f64| (v * 10.).round() / 10.;
    let k = x / step;
    let dn = r(((k - 1e-9).ceil() - 1.) * step).max(1f64.min(x));
    let up = r(((k + 1e-9).floor() + 1.) * step).min(100.);
    (dn.min(x), up.max(x))
}

/// Why a tool's "When to use" line can't be saved (the server takes up to 300 characters), if it can't.
fn when_problem(line: &str) -> Option<String> {
    let n = line.chars().count();
    (n > 300).then(|| format!("\u{201c}When to use\u{201d} can be up to 300 characters. This is {n}."))
}

/// Opens Settings, or brings it forward.
pub fn open(cx: &mut App) {
    open_at(Section::Appearance, cx)
}

pub fn open_at(section: Section, cx: &mut App) {
    open_with(section, None, None, cx)
}

/// Opens Settings at one machine's page (a machine whose Claude folder nobody has picked yet, or one just added).
pub fn open_machine(name: String, cx: &mut App) {
    open_with(Section::Machines, Some(name), None, cx)
}

/// Screenshots: a machine's page, with its folder picker open at `pick` (home-relative).
pub fn open_machine_at(name: String, pick: Option<String>, cx: &mut App) {
    open_with(Section::Machines, Some(name), pick, cx)
}

fn open_with(section: Section, machine: Option<String>, pick: Option<String>, cx: &mut App) {
    let app = crate::state::app(cx);
    if let Some(h) = app.read(cx).settings_win {
        let (m, p) = (machine.clone(), pick.clone());
        if h.update(cx, |v, window, cx| {
            window.activate_window();
            if let Ok(v) = v.downcast::<SettingsView>() {
                v.update(cx, |s, cx| match m {
                    Some(m) => {
                        s.open_machine(m.clone(), cx);
                        if let Some(p) = p {
                            s.browse(m, p, cx);
                        }
                    }
                    None => s.go(section, cx),
                });
            }
        })
        .is_ok()
        {
            return;
        }
    }
    let (w, h) = (900., 680.);
    // Screenshots: SB_SETTINGS_SIZE=720x500 opens the window at another size (its smallest, say).
    #[cfg(feature = "shots")]
    let (w, h) = std::env::var("SB_SETTINGS_SIZE")
        .ok()
        .and_then(|v| v.split_once('x').and_then(|(a, b)| Some((a.parse().ok()?, b.parse().ok()?))))
        .unwrap_or((w, h));
    let opts = crate::files::options("Settings, Switchboard", w, h, (720., 500.), cx);
    match cx.open_window(opts, move |window, cx| {
        cx.new(|cx| {
            let mut v = SettingsView::new(section, window, cx);
            if let Some(m) = machine {
                v.open_machine(m.clone(), cx);
                if let Some(p) = pick {
                    v.browse(m, p, cx);
                }
            }
            v
        })
    }) {
        Ok(h) => app.update(cx, |s, _| s.settings_win = Some(h.into())),
        Err(e) => log::error!("could not open Settings: {e:#}"),
    }
}

/// One machine being edited: its name or its folder.
#[derive(Clone, Debug, PartialEq)]
enum Edit {
    Name(String),
    Root(String),
    /// A library tool's "When to use" line.
    When(String),
}

/// The Claude folder picker: one level of a machine's home folder at a time.
struct Picker {
    machine: String,
    path: String,
    dirs: Option<Result<ApiRootDirs, String>>,
}

pub struct SettingsView {
    app: Entity<AppState>,
    focus: FocusHandle,
    section: Section,
    /// The machine whose page is open (Machines only).
    machine: Option<String>,
    picker: Option<Picker>,
    caps: HashMap<String, Result<ApiCapsView, String>>,
    mcp: HashMap<String, Result<ApiMcp, String>>,
    extra: Entity<InputState>,
    /// A machine added over SSH: its page opens once the list has it.
    select_after: Option<String>,
    machines: Option<Result<ApiMachines, String>>,
    edit: Option<Edit>,
    /// Remove asks twice (the second click within a few seconds).
    removing: Option<(String, f64)>,
    adding: bool,
    add_os: &'static str,
    field: Entity<InputState>,
    add_id: Entity<InputState>,
    add_label: Entity<InputState>,
    add_target: Entity<InputState>,
    error: Option<String>,
    scroll: ScrollHandle,
    nav_scroll: ScrollHandle,
    /// The Claude-folder picker's list of folders.
    dir_scroll: ScrollHandle,
    /// Library: Import and New tool (one open at a time), their fields, open version lists, the Set key command
    /// showing (tool, machine, key), and Delete's first click.
    lib_import: Option<library_ui::Import>,
    lib_new: Option<library_ui::NewTool>,
    lib_name: Entity<InputState>,
    lib_brief: Entity<TextareaState>,
    lib_hist: HashSet<String>,
    lib_key: Option<(String, String, String)>,
    /// Set key on this Mac: the masked field (emptied as soon as Save takes the value) and what went wrong.
    lib_secret: Entity<InputState>,
    lib_key_err: Option<String>,
    lib_deleting: Option<(String, f64)>,
    /// Server addresses: the field for typing one, why what was typed can't be one, and the first click on the X
    /// of the address in use (it asks twice).
    addr: Entity<InputState>,
    addr_err: Option<String>,
    addr_removing: Option<(String, f64)>,
    /// A saved setup's Remove asks twice too: (its place in the list, when).
    preset_removing: Option<(usize, f64)>,
    /// The open dropdown (Terminal's default shell), if any.
    drop: ui::Drop,
    /// Settings > Server (settings_server.rs).
    srv: server_ui::Pane,
    _subs: Vec<Subscription>,
}

impl SettingsView {
    pub fn new(section: Section, window: &mut Window, cx: &mut Context<Self>) -> SettingsView {
        let app = crate::state::app(cx);
        let field = cx.new(|cx| InputState::new(window, cx));
        let add_id = cx.new(|cx| InputState::new(window, cx).placeholder("studio"));
        let add_label = cx.new(|cx| InputState::new(window, cx).placeholder("Studio"));
        let add_target = cx.new(|cx| InputState::new(window, cx).placeholder("you@studio.local"));
        let extra = cx.new(|cx| InputState::new(window, cx).placeholder("Something else it can do"));
        let lib_name = cx.new(|cx| InputState::new(window, cx).placeholder("my-tool"));
        let lib_brief = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder("What it does, what it needs, and how you'd check it works.")
                .auto_grow(3, 8)
        });
        let lib_secret = cx.new(|cx| InputState::new(window, cx).masked(true).placeholder("Paste the value"));
        let addr = cx.new(|cx| InputState::new(window, cx).placeholder("192.0.2.10:8130 or studio.local"));
        let subs = vec![
            cx.subscribe_in(&addr, window, |this, _, ev: &InputEvent, window, cx| match ev {
                InputEvent::PressEnter { .. } => this.add_address(window, cx),
                InputEvent::Change if this.addr_err.is_some() => {
                    this.addr_err = None;
                    cx.notify();
                }
                _ => {}
            }),
            cx.observe(&app, |_, _, cx| cx.notify()),
            cx.observe_window_appearance(window, |_, _, cx| cx.notify()),
            cx.subscribe_in(&field, window, |this, _, ev: &InputEvent, _, cx| {
                if let InputEvent::PressEnter { .. } = ev {
                    this.save_edit(cx);
                }
            }),
            cx.subscribe_in(&extra, window, |this, _, ev: &InputEvent, window, cx| {
                if let InputEvent::PressEnter { .. } = ev {
                    this.add_extra(window, cx);
                }
            }),
            // The name's hint turns red as soon as it is not a tool name.
            cx.subscribe(&lib_name, |_, _, ev: &InputEvent, cx| {
                if let InputEvent::Change = ev {
                    cx.notify();
                }
            }),
            cx.subscribe_in(&lib_secret, window, |this, _, ev: &InputEvent, window, cx| {
                if let InputEvent::PressEnter { .. } = ev {
                    this.save_key(window, cx);
                }
            }),
        ];
        let mut v = SettingsView {
            app,
            focus: cx.focus_handle(),
            section,
            machine: None,
            picker: None,
            caps: HashMap::new(),
            mcp: HashMap::new(),
            extra,
            select_after: None,
            machines: None,
            edit: None,
            removing: None,
            addr,
            addr_err: None,
            addr_removing: None,
            preset_removing: None,
            adding: false,
            add_os: "mac",
            field,
            add_id,
            add_label,
            add_target,
            error: None,
            scroll: ScrollHandle::new(),
            nav_scroll: ScrollHandle::new(),
            dir_scroll: ScrollHandle::new(),
            lib_import: None,
            lib_new: None,
            lib_name,
            lib_brief,
            lib_hist: HashSet::new(),
            lib_key: None,
            lib_secret,
            lib_key_err: None,
            lib_deleting: None,
            drop: Default::default(),
            srv: Default::default(),
            _subs: subs,
        };
        v.load(cx);
        if section == Section::Server {
            v.server_open(cx);
        }
        v.app.update(cx, |s, cx| s.load_library(cx));
        v
    }

    pub fn go(&mut self, s: Section, cx: &mut Context<Self>) {
        if self.section == Section::Server && s != Section::Server {
            self.server_close();
        }
        if s == Section::Server && self.section != Section::Server {
            self.server_open(cx);
        }
        self.section = s;
        self.machine = None;
        self.picker = None;
        self.edit = None;
        self.scroll.scroll_to_item(0);
        if s == Section::Library {
            self.app.update(cx, |s, cx| s.load_library(cx));
        }
        cx.notify();
    }

    pub fn open_machine(&mut self, name: String, cx: &mut Context<Self>) {
        self.section = Section::Machines;
        self.edit = None;
        self.error = None;
        if self.picker.as_ref().is_some_and(|p| p.machine != name) {
            self.picker = None;
        }
        self.machine = Some(name.clone());
        self.scroll.scroll_to_item(0);
        self.load_machine(name, cx);
        cx.notify();
    }

    fn row_of(&self, name: &str) -> Option<ApiMachineRow> {
        match &self.machines {
            Some(Ok(m)) => m.machines.iter().find(|x| x.name == name).cloned(),
            _ => None,
        }
    }

    /// What the machine's page shows live: its capabilities and its MCP servers.
    fn load_machine(&mut self, name: String, cx: &mut Context<Self>) {
        let core = self.app.read(cx).core.clone();
        cx.spawn(async move |this, cx| {
            let (c2, n2) = (core.clone(), name.clone());
            let caps = cx.background_executor().spawn(async move { c2.machine_caps(&n2) }).await;
            let _ = this.update(cx, |v, cx| {
                v.caps.insert(name.clone(), caps.map_err(|e| e.to_string()));
                cx.notify();
            });
            let n3 = name.clone();
            let mcp = cx.background_executor().spawn(async move { core.machine_mcp(&n3) }).await;
            let _ = this.update(cx, |v, cx| {
                v.mcp.insert(name, mcp.map_err(|e| e.to_string()));
                cx.notify();
            });
        })
        .detach();
    }

    /// Opens the folder picker at `path` (home-relative, '' for the home folder).
    /// Draws once more when an armed button's time is up, so "Click again to ..." goes back to its first label
    /// (the label is worked out when drawn, and nothing else would draw just then).
    pub(crate) fn disarm_later(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(std::time::Duration::from_secs_f64(ARM_FOR + 0.05)).await;
            let _ = this.update(cx, |_, cx| cx.notify());
        })
        .detach();
    }

    fn browse(&mut self, machine: String, path: String, cx: &mut Context<Self>) {
        self.picker = Some(Picker { machine: machine.clone(), path: path.clone(), dirs: None });
        self.dir_scroll.set_offset(gpui::point(px(0.), px(0.)));
        self.edit = None;
        let core = self.app.read(cx).core.clone();
        cx.spawn(async move |this, cx| {
            let (m2, p2) = (machine.clone(), path.clone());
            let res = cx.background_executor().spawn(async move { core.machine_folders(&m2, &p2) }).await;
            let _ = this.update(cx, |v, cx| {
                if let Some(p) = v.picker.as_mut().filter(|p| p.machine == machine && p.path == path) {
                    p.dirs = Some(res.map_err(|e| e.to_string()));
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn use_folder(&mut self, machine: String, root: String, cx: &mut Context<Self>) {
        self.picker = None;
        self.send(serde_json::json!({"op": "root", "machine": machine, "root": root}), cx);
    }

    /// Auto for a machine: on or off, a tool left out, or the user's own lines. Shown at once, then saved.
    fn set_caps(&mut self, machine: String, body: serde_json::Value, cx: &mut Context<Self>) {
        // As it was, to put back when the change doesn't go through.
        let before = self.caps.get(&machine).and_then(|c| c.as_ref().ok()).cloned();
        if let Some(Ok(c)) = self.caps.get_mut(&machine) {
            if let Some(on) = body.get("on").and_then(|v| v.as_bool()) {
                c.on = on;
            }
            if let Some(off) = body.get("off").and_then(|v| v.as_array()) {
                c.off = off.iter().filter_map(|x| x.as_str().map(String::from)).collect();
                for t in c.tools.iter_mut().flatten() {
                    t.on = !c.off.contains(&t.key);
                }
            }
            if let Some(extra) = body.get("extra").and_then(|v| v.as_array()) {
                c.extra = extra.iter().filter_map(|x| x.as_str().map(String::from)).collect();
            }
        }
        let mut b = body;
        b["op"] = "caps".into();
        b["machine"] = machine.clone().into();
        let core = self.app.read(cx).core.clone();
        cx.spawn(async move |this, cx| {
            let res = cx.background_executor().spawn(async move { core.machines_edit(&b) }).await;
            let _ = this.update(cx, |v, cx| {
                match res {
                    Ok(m) => {
                        v.machines = Some(Ok(m));
                        v.error = None;
                        v.load_machine(machine, cx);
                    }
                    // The switch goes back, and the reason shows wherever the page is scrolled to.
                    Err(e) => {
                        if let Some(b) = before {
                            v.caps.insert(machine, Ok(b));
                        }
                        let e = e.to_string();
                        v.error = Some(e.clone());
                        v.app.update(cx, |s, cx| s.toast("That change wasn't saved", Some(e), true, cx));
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn add_extra(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(name) = self.machine.clone() else { return };
        let line = self.extra.read(cx).value().trim().to_string();
        let Some(Ok(c)) = self.caps.get(&name) else { return };
        if line.is_empty() || c.extra.iter().any(|x| x.eq_ignore_ascii_case(&line)) {
            return;
        }
        let mut extra = c.extra.clone();
        extra.push(line);
        self.extra.update(cx, |f, cx| f.set_value("", window, cx));
        self.set_caps(name, serde_json::json!({"extra": extra}), cx);
    }

    fn load(&mut self, cx: &mut Context<Self>) {
        let core = self.app.read(cx).core.clone();
        cx.spawn(async move |this, cx| {
            let res = cx.background_executor().spawn(async move { core.machines() }).await;
            let _ = this.update(cx, |v, cx| {
                v.machines = Some(res.map_err(|e| e.to_string()));
                cx.notify();
            });
        })
        .detach();
    }

    fn send(&mut self, body: serde_json::Value, cx: &mut Context<Self>) {
        let core = self.app.read(cx).core.clone();
        self.error = None;
        cx.spawn(async move |this, cx| {
            let res = cx.background_executor().spawn(async move { core.machines_edit(&body) }).await;
            let _ = this.update(cx, |v, cx| {
                match res {
                    Ok(m) => {
                        let added = v.select_after.take().filter(|n| m.machines.iter().any(|x| &x.name == n));
                        v.machines = Some(Ok(m));
                        v.edit = None;
                        v.adding = false;
                        if let Some(n) = added {
                            v.open_machine(n, cx);
                        }
                        v.app.update(cx, |s, cx| s.refresh_in(std::time::Duration::from_millis(300), cx));
                    }
                    Err(e) => v.error = Some(e.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn start_edit(&mut self, e: Edit, value: String, window: &mut Window, cx: &mut Context<Self>) {
        self.edit = Some(e);
        self.error = None;
        self.field.update(cx, |f, cx| {
            f.set_value(value, window, cx);
            f.focus(window, cx);
        });
        cx.notify();
    }

    fn save_edit(&mut self, cx: &mut Context<Self>) {
        let v = self.field.read(cx).value().trim().to_string();
        match self.edit.clone() {
            Some(Edit::Name(n)) => self.send(serde_json::json!({"op": "rename", "machine": n, "label": v}), cx),
            Some(Edit::Root(n)) => self.send(serde_json::json!({"op": "root", "machine": n, "root": v}), cx),
            Some(Edit::When(tool)) => {
                // The field stays open, with what was typed, until the line is one the server takes and it is
                // on its way.
                let why = when_problem(&v).or_else(|| {
                    let auto =
                        self.app.read(cx).lib().and_then(|l| l.tool(&tool)).and_then(|x| x.auto).unwrap_or(false);
                    let sent = self.app.update(cx, |s, cx| s.lib_auto(&tool, auto, &v, cx));
                    (!sent).then(|| "The last change to this tool is still being saved. Try again in a moment.".into())
                });
                match why {
                    Some(e) => self.app.update(cx, |s, cx| s.toast("Not saved yet", Some(e), true, cx)),
                    None => self.edit = None,
                }
                cx.notify();
            }
            None => {}
        }
    }

    /// Settings > Server, Add: what the field holds becomes a saved address, tried along with the others.
    fn add_address(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let typed = self.addr.read(cx).value().trim().to_string();
        if typed.is_empty() {
            return;
        }
        match self.app.update(cx, |s, cx| s.use_server(&typed, cx)) {
            Ok(()) => {
                self.addr_err = None;
                self.addr.update(cx, |f, cx| f.set_value("", window, cx));
            }
            Err(e) => self.addr_err = Some(e),
        }
        cx.notify();
    }

    fn move_machine(&mut self, i: usize, up: bool, cx: &mut Context<Self>) {
        let Some(Ok(m)) = &self.machines else { return };
        let mut names: Vec<String> = m.machines.iter().map(|x| x.name.clone()).collect();
        let j = if up { i.checked_sub(1) } else { (i + 1 < names.len()).then_some(i + 1) };
        if let Some(j) = j {
            names.swap(i, j);
            self.send(serde_json::json!({"op": "order", "names": names}), cx);
        }
    }

    fn add(&mut self, cx: &mut Context<Self>) {
        let id = self.add_id.read(cx).value().trim().to_lowercase();
        let label = self.add_label.read(cx).value().trim().to_string();
        let target = self.add_target.read(cx).value().trim().to_string();
        self.select_after = Some(id.clone());
        self.send(serde_json::json!({"op": "add", "name": id, "label": if label.is_empty() { None } else { Some(label) }, "target": target, "os": self.add_os}), cx);
    }

    // ---------- sections ----------

    pub(crate) fn head(t: &Theme, title: &str, sub: &str) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .gap(px(4.))
            .mb(px(18.))
            .child(div().text_size(px(20.)).font_weight(FontWeight::BOLD).text_color(t.ink).child(title.to_string()))
            .child(ui::wrap(sub.to_string()).text_size(px(13.)).line_height(relative(1.45)).text_color(t.dim))
            .into_any_element()
    }

    /// A row on a section's page, ruled under: the shared settings row (ui::setting_row).
    fn row(t: &Theme, label: &str, note: Option<&str>, ctl: AnyElement) -> AnyElement {
        div()
            .py(px(12.))
            .border_b_1()
            .border_color(t.line)
            .child(ui::setting_row(t, label.to_string(), note.map(|n| SharedString::from(n.to_string())), Some(ctl)))
            .into_any_element()
    }

    fn appearance(&self, t: &Theme, cx: &Context<Self>) -> AnyElement {
        let mode = Mode::parse(self.app.read(cx).settings.theme.as_deref());
        let mut seg = ui::seg_box(t);
        for (i, (m, label, ic)) in
            [(Mode::Light, "Light", "sun"), (Mode::System, "System", "contrast"), (Mode::Dark, "Dark", "moon")]
                .into_iter()
                .enumerate()
        {
            seg = seg.child(
                ui::seg_btn(t, ("st-mode", i), mode == m, false)
                    .child(ui::icon(ic, 13., if mode == m { t.ink } else { t.dim }))
                    .child(label)
                    .on_click(cx.listener(move |_, _: &ClickEvent, window, cx| crate::theme::set_mode(m, window, cx))),
            );
        }
        div()
            .child(Self::head(
                t,
                "Appearance",
                if ui::MAC {
                    "How Switchboard looks. Synced, so your other Macs and the iPhone app match."
                } else {
                    "How Switchboard looks. Synced, so your other computers and the iPhone app match."
                },
            ))
            .child(Self::row(
                t,
                "Theme",
                Some(if ui::MAC {
                    "Also in the header, and ⌘⇧L switches light and dark."
                } else {
                    "Also in the header, and Ctrl+Shift+L switches light and dark."
                }),
                seg.into_any_element(),
            ))
            .into_any_element()
    }

    fn terminal(&self, t: &Theme, cx: &Context<Self>) -> AnyElement {
        let look = colors::look(cx);
        let hover = t.hover;
        let mut grid = div().flex().flex_wrap().gap(px(10.));
        let mut all: Vec<(String, String, colors::Palette)> =
            colors::scheme::all().iter().map(|s| (s.id.clone(), s.name.clone(), colors::palette_of(s))).collect();
        all.push((colors::MATCH.to_string(), "Match the app".into(), colors::palette(t)));
        for (id, name, pal) in all {
            let on = look.scheme == id;
            let (pick, peek) = (id.clone(), id.clone());
            grid = grid.child(
                div()
                    .id(SharedString::from(format!("st-scheme-{id}")))
                    .w(px(176.))
                    .p(px(8.))
                    .flex()
                    .flex_col()
                    .gap(px(7.))
                    .rounded(px(ui::R_CARD))
                    .cursor_pointer()
                    .when(on, |d| d.bg(t.sel).shadow(vec![Theme::ring(t.accent)]))
                    .when(!on, |d| d.hover(move |s| s.bg(hover)))
                    .child(crate::term::swatch(t, &pal, 160., 44.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .child(
                                div()
                                    .flex_1()
                                    .text_size(px(13.))
                                    .text_color(if on { t.ink } else { t.ink2 })
                                    .child(name),
                            )
                            .when(on, |d| d.child(ui::icon("check", 13., t.accent))),
                    )
                    .on_hover(cx.listener(move |_, h: &bool, _, cx| colors::set_preview(h.then(|| peek.clone()), cx)))
                    .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                        colors::set_preview(None, cx);
                        crate::theme::set_term_colors(&pick, cx);
                    })),
            );
        }
        let size = look.size.unwrap_or(sb_core::term::scheme::SIZE_DEFAULT);
        let step = |id: &'static str, ic: &'static str, up: bool| {
            ui::icon_btn(t, id, ic, 28.).shadow(vec![Theme::ring(t.line2)]).on_click(cx.listener(
                move |_, _: &ClickEvent, _, cx| {
                    let next = sb_core::term::scheme::step_size(colors::look(cx).size, up);
                    crate::theme::save_look_here(cx, |s| s.term_size = Some(next));
                },
            ))
        };
        let mut cursors = ui::seg_box(t);
        for (i, (v, label)) in [("block", "Block"), ("bar", "Bar"), ("underline", "Underline")].into_iter().enumerate()
        {
            cursors = cursors.child(ui::seg_btn(t, ("st-cur", i), look.cursor == v, false).child(label).on_click(
                cx.listener(move |_, _: &ClickEvent, _, cx| {
                    crate::theme::save_look(cx, |s| s.term_cursor = Some(v.to_string()))
                }),
            ));
        }
        let blink = look.blink;
        let close_stops = self.app.read(cx).close_stops_on();
        // The shell a new local terminal runs (Cmd+T, the "+" chevron's default).
        let profiles = crate::state::profiles();
        let saved = self.app.read(cx).settings.term_profile.clone();
        let def = sb_core::term::shells::pick_default(profiles, saved.as_deref()).map(|p| p.id.clone());
        let open = self.drop.is_open("st-shell");
        let cur =
            profiles.iter().find(|p| def.as_deref() == Some(p.id.as_str())).map(|p| p.name.clone()).unwrap_or_default();
        let trigger = ui::dropdown(t, "st-shell", Some("term"), cur, open).on_click(cx.listener(
            |this, _: &ClickEvent, _, cx| {
                this.drop.toggle("st-shell");
                cx.notify();
            },
        ));
        let menu = open.then(|| {
            let items = profiles
                .iter()
                .map(|p| {
                    let id = p.id.clone();
                    ui::DropItem {
                        label: format!("{}  {}", p.name, p.program).into(),
                        on: def.as_deref() == Some(p.id.as_str()),
                        off: false,
                        pick: Box::new(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.drop.close();
                            let id = id.clone();
                            this.app.update(cx, |s, _| {
                                s.settings.term_profile = Some(id);
                                s.save_settings();
                            });
                            cx.notify();
                        })),
                        sub: None,
                        fold: None,
                    }
                })
                .collect();
            ui::drop_menu(
                t,
                "st-shell",
                items,
                cx.listener(|this, _: &gpui::MouseDownEvent, _, cx| {
                    this.drop.close_out();
                    cx.notify();
                }),
            )
        });
        let shell = div().relative().flex_none().child(trigger).children(menu).into_any_element();
        div()
            .child(Self::head(t, "Terminal", "Claude Code draws for a dark terminal, so the terminal keeps its own colors. Hover a scheme to preview it in every open terminal."))
            .child(grid)
            .child(div().h(px(10.)))
            .child(Self::row(
                t,
                "Default terminal",
                Some(&format!("This computer only. The shell {} opens here. Every shell is also in the menu beside \"+\" and in {}.", ui::term_cap("T"), ui::mod_cap("K"))),
                shell,
            ))
            .child(Self::row(
                t,
                "Text size",
                Some(&format!("This computer only, since screens differ. {} and {} in a terminal.", ui::mod_cap("+"), ui::mod_cap("-"))),
                div().flex().items_center().gap(px(8.)).child(step("st-smaller", "zoomout", false)).child(div().w(px(32.)).flex().justify_center().font_family(NUM).text_size(px(14.)).text_color(t.ink).child(size.to_string())).child(step("st-bigger", "zoomin", true)).into_any_element(),
            ))
            .child(Self::row(t, "Cursor", None, cursors.into_any_element()))
            .child(Self::row(
                t,
                "Blink",
                None,
                ui::switch(t, "st-blink", blink, false)
                    .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| crate::theme::save_look(cx, |s| s.term_blink = Some(!blink))))
                    .into_any_element(),
            ))
            .child(Self::row(
                t,
                "Stop a chat when its tab closes",
                Some("This computer only. Off: closing a tab leaves its chat running, and Stop on the chat's card ends it. On: the chat stops with its tab, once its reply is done. Chats set to Keep running are never stopped this way."),
                ui::switch(t, "st-close-stops", close_stops, false)
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.app.update(cx, |s, cx| {
                            s.settings.close_stops = Some(!close_stops);
                            s.save_settings();
                            cx.notify();
                        })
                    }))
                    .into_any_element(),
            ))
            .into_any_element()
    }

    /// Stop idle chats: how long a finished chat is left alone before the server stops it to free its memory.
    /// Kept on the server, so every computer agrees. Not shown for a server that does not do this.
    fn idle_stop_row(&self, t: &Theme, cx: &Context<Self>) -> Option<AnyElement> {
        let hours = self.app.read(cx).prefs.as_ref().and_then(|p| p.auto_stop_h)?;
        let mut seg = ui::seg_box(t);
        for (i, (h, label)) in IDLE_STOP_CHOICES.into_iter().enumerate() {
            seg = seg.child(ui::seg_btn(t, ("st-idle-stop", i), hours == h, false).child(label).on_click(cx.listener(
                move |this, _: &ClickEvent, _, cx| {
                    this.app.update(cx, |s, cx| {
                        if let Some(p) = s.prefs.as_mut() {
                            p.auto_stop_h = Some(h);
                        }
                        s.save_prefs(serde_json::json!({"auto_stop_h": h}), Some("Saved".into()), cx)
                    })
                },
            )));
        }
        Some(Self::group(
            t,
            "Idle chats",
            None,
            vec![Self::crow(t, "Stop idle chats", Some(IDLE_STOP_NOTE.into()), Some(seg.into_any_element()))],
        ))
    }

    /// A titled group of rows on a card, like the Mac's own Settings.
    pub(crate) fn group(t: &Theme, title: &str, sub: Option<&str>, rows: Vec<AnyElement>) -> AnyElement {
        let mut card = div().rounded(px(ui::R_CARD)).border_1().border_color(t.line).bg(t.bg).flex().flex_col();
        for (i, r) in rows.into_iter().enumerate() {
            card =
                card.child(div().px(px(14.)).py(px(11.)).when(i > 0, |d| d.border_t_1().border_color(t.line)).child(r));
        }
        div()
            .mt(px(22.))
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(3.))
                    .child(
                        div()
                            .text_size(px(13.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(t.ink2)
                            .child(title.to_string()),
                    )
                    .children(sub.map(|s| {
                        ui::wrap(s.to_string()).text_size(px(12.5)).line_height(relative(1.45)).text_color(t.faint)
                    })),
            )
            .child(card)
            .into_any_element()
    }

    /// One row on a group's card: the shared settings row (ui::setting_row).
    fn crow(t: &Theme, label: impl Into<SharedString>, note: Option<String>, ctl: Option<AnyElement>) -> AnyElement {
        ui::setting_row(t, label, note.map(SharedString::from), ctl).into_any_element()
    }

    fn os_name(os: Option<&str>) -> &'static str {
        match os {
            Some("mac") => "macOS",
            Some("windows") => "Windows",
            Some("linux") => "Linux",
            _ => "",
        }
    }

    fn status_line(m: &ApiMachineRow) -> String {
        match (m.online, m.updated) {
            (Some(true), Some(u)) => format!("Online, checked {}", sb_core::fmt::ago(u, AppState::now())),
            (Some(true), None) => "Online".into(),
            (Some(false), _) => "Offline".into(),
            _ => "Not checked yet".into(),
        }
    }

    fn versions_line(m: &ApiMachineRow) -> Option<String> {
        let v = m.versions.as_ref()?;
        let mut parts = vec![];
        if let Some(c) = &v.claude {
            parts.push(format!("Claude Code {c}"));
        }
        if let Some(a) = &v.agent {
            parts.push(format!("Switchboard agent {a}"));
        }
        (!parts.is_empty()).then(|| parts.join(", "))
    }

    fn text_field(&self, t: &Theme, id: &'static str, focused: bool) -> gpui::Stateful<gpui::Div> {
        div()
            .id(id)
            .flex_1()
            .min_w(px(160.))
            .h(px(32.))
            .px(px(10.))
            .flex()
            .items_center()
            .rounded(px(ui::R_CTRL))
            .border_1()
            .border_color(if focused { t.accent } else { t.line2 })
            .bg(t.bg)
            .text_size(px(13.))
            .text_color(t.ink)
            .child(gpui_base::Input::new(&self.field))
    }

    fn edit_row(
        &self,
        t: &Theme,
        ids: (&'static str, &'static str, &'static str),
        focused: bool,
        cx: &Context<Self>,
    ) -> AnyElement {
        div()
            .flex()
            .flex_wrap()
            .gap(px(8.))
            .child(self.text_field(t, ids.0, focused))
            .child(
                ui::primary_btn(t, ids.1, 32.)
                    .px(px(12.))
                    .text_size(px(13.))
                    .child("Save")
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.save_edit(cx))),
            )
            .child(ui::ghost(t, ids.2, Some("Cancel"), None).h(px(32.)).px(px(10.)).text_size(px(13.)).on_click(
                cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.edit = None;
                    cx.notify();
                }),
            ))
            .into_any_element()
    }

    fn machines(&self, t: &Theme, window: &Window, cx: &Context<Self>) -> AnyElement {
        if let Some(m) = self.machine.as_deref().and_then(|n| self.row_of(n)) {
            return self.machine_page(t, m, window, cx);
        }
        let mut body = div().child(Self::head(t, "Machines", "Every machine Switchboard watches, in board order. Open one to set its Claude folder, or to see what it can do and its MCP servers."));
        let list = match &self.machines {
            None => {
                return body.child(ui::loading(t, "st-m-load", "Loading the machines")).into_any_element();
            }
            Some(Err(e)) => {
                return body
                    .child(ui::wrap(format!("Couldn't load the machines. {e}")).text_size(px(13.)).text_color(t.bad))
                    .into_any_element();
            }
            Some(Ok(m)) => m.machines.clone(),
        };
        let n = list.len();
        let hover = t.hover;
        let mut card = div().rounded(px(ui::R_CARD)).border_1().border_color(t.line).bg(t.bg).flex().flex_col();
        for (i, m) in list.into_iter().enumerate() {
            let os = Self::os_name(m.os.as_deref());
            let open = m.name.clone();
            let folder = m.root_shown.clone().unwrap_or_else(|| "Its home folder".into());
            card = card.child(
                div()
                    .id(SharedString::from(format!("st-m-{}", m.name)))
                    .px(px(14.))
                    .py(px(12.))
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .cursor_pointer()
                    .hover(move |s| s.bg(hover))
                    .when(i == 0, |d| d.rounded_t(px(ui::R_CARD - 1.)))
                    .when(i + 1 == n, |d| d.rounded_b(px(ui::R_CARD - 1.)))
                    .when(i > 0, |d| d.border_t_1().border_color(t.line))
                    .child(
                        div().flex_none().relative().child(ui::machine_icon(&m.name, 22., t.dim)).child(
                            div()
                                .absolute()
                                .right(px(-2.))
                                .bottom(px(-1.))
                                .size(px(9.))
                                .rounded_full()
                                .border_2()
                                .border_color(t.bg)
                                .bg(match m.online {
                                    Some(true) => t.warm,
                                    Some(false) => t.bad,
                                    None => t.cold,
                                }),
                        ),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(3.))
                            .child(
                                div()
                                    .flex()
                                    .flex_wrap()
                                    .items_center()
                                    .gap(px(8.))
                                    .child(
                                        ui::wrap(m.label.clone())
                                            .text_size(px(14.5))
                                            .font_weight(FontWeight::BOLD)
                                            .text_color(t.ink),
                                    )
                                    .child(div().text_size(px(12.5)).text_color(t.faint).child(if os.is_empty() {
                                        m.name.clone()
                                    } else {
                                        format!("{os}, {}", m.name)
                                    }))
                                    .when(m.needs_root, |d| {
                                        d.child(ui::note_tag(t, "folder", "Pick its Claude folder"))
                                    }),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(6.))
                                    .text_size(px(12.5))
                                    .text_color(t.dim)
                                    .child(ui::icon("folder", 13., t.faint))
                                    .child(ui::wrap(folder).font_family(NUM)),
                            )
                            .child(
                                ui::wrap(match Self::versions_line(&m) {
                                    Some(v) => format!("{}. {v}", Self::status_line(&m)),
                                    None => Self::status_line(&m),
                                })
                                .text_size(px(12.))
                                .text_color(t.faint),
                            ),
                    )
                    .child(
                        div()
                            .flex_none()
                            .flex()
                            .items_center()
                            .gap(px(2.))
                            .child(
                                ui::icon_btn(t, SharedString::from(format!("st-up-{}", m.name)), "up", 28.)
                                    .when(i == 0, |d| d.opacity(0.35))
                                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                        cx.stop_propagation();
                                        this.move_machine(i, true, cx)
                                    })),
                            )
                            .child(
                                ui::icon_btn(t, SharedString::from(format!("st-down-{}", m.name)), "down", 28.)
                                    .when(i + 1 == n, |d| d.opacity(0.35))
                                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                        cx.stop_propagation();
                                        this.move_machine(i, false, cx)
                                    })),
                            )
                            .child(div().ml(px(4.)).child(ui::icon("chev", 14., t.faint))),
                    )
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.open_machine(open.clone(), cx))),
            );
        }
        body = body.child(card);
        if let Some(e) = &self.error {
            body = body.child(ui::wrap(e.clone()).mt(px(10.)).text_size(px(13.)).text_color(t.bad));
        }
        let field = |inp: &Entity<InputState>, w: f32| {
            div()
                .w(px(w))
                .h(px(32.))
                .px(px(10.))
                .flex()
                .items_center()
                .rounded(px(ui::R_CTRL))
                .border_1()
                .border_color(t.line2)
                .bg(t.bg)
                .text_size(px(13.))
                .text_color(t.ink)
                .child(gpui_base::Input::new(inp))
        };
        let labeled = |l: &str, el: AnyElement| {
            div()
                .flex()
                .flex_col()
                .gap(px(5.))
                .child(div().text_size(px(12.)).text_color(t.dim).child(l.to_string()))
                .child(el)
        };
        if self.adding {
            let mut oss = ui::seg_box(t);
            for (i, (v, l)) in [("mac", "macOS"), ("linux", "Linux"), ("windows", "Windows")].into_iter().enumerate() {
                oss = oss.child(ui::seg_btn(t, ("st-os", i), self.add_os == v, false).child(l).on_click(cx.listener(
                    move |this, _: &ClickEvent, _, cx| {
                        this.add_os = v;
                        cx.notify();
                    },
                )));
            }
            body = body.child(
                div()
                    .mt(px(16.))
                    .p(px(16.))
                    .rounded(px(ui::R_CARD))
                    .border_1()
                    .border_color(t.line)
                    .bg(t.bg)
                    .flex()
                    .flex_col()
                    .gap(px(12.))
                    .child(div().text_size(px(14.)).font_weight(FontWeight::SEMIBOLD).text_color(t.ink).child("Add a machine over SSH"))
                    .child(ui::wrap("Switchboard connects the way you would, over ssh with your keys already set up. Then you pick its Claude folder.").text_size(px(12.5)).text_color(t.dim))
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap(px(12.))
                            .child(labeled("Short id", field(&self.add_id, 140.).into_any_element()))
                            .child(labeled("Name", field(&self.add_label, 180.).into_any_element()))
                            .child(labeled("Address", field(&self.add_target, 220.).into_any_element())),
                    )
                    .child(labeled("System", oss.into_any_element()))
                    .child(
                        div()
                            .flex()
                            .justify_end()
                            .gap(px(8.))
                            .child(ui::ghost(t, "st-add-x", Some("Cancel"), None).h(px(32.)).px(px(10.)).text_size(px(13.)).on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.adding = false;
                                cx.notify();
                            })))
                            .child(ui::primary_btn(t, "st-add-go", 32.).px(px(14.)).text_size(px(13.)).child("Add it").on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.add(cx)))),
                    ),
            );
        } else {
            body = body.child(
                div()
                    .mt(px(16.))
                    .flex()
                    .flex_wrap()
                    .gap(px(8.))
                    .child(
                        ui::primary_btn(t, "st-pair", 34.)
                            .px(px(14.))
                            .text_size(px(13.))
                            .child(ui::icon("plus", 14., t.on_accent))
                            .child("Add a machine")
                            .on_click(cx.listener(|_, _: &ClickEvent, _, cx| crate::files::open_pair(cx))),
                    )
                    .child(
                        ui::line_btn(t, "st-ssh", 34.)
                            .px(px(14.))
                            .text_size(px(13.))
                            .child("Add one over SSH")
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.adding = true;
                                cx.notify();
                            })),
                    ),
            );
        }
        body.into_any_element()
    }

    /// Where this app finds the server (Settings > Server): the saved addresses, typed and learned, each with
    /// Remove, and a field to type one more.
    fn server_group(&self, t: &Theme, cx: &Context<Self>) -> AnyElement {
        use gpui::AnimationExt;
        let sub = "This computer only. How this app finds the computer that runs Switchboard. Learned addresses come from the server, so it still connects when you're away from home. The last one that worked gets tried first.";
        let app = self.app.read(cx);
        let saved = &app.settings.saved;
        let in_use = app.core.server().map(|u| sb_core::servers::key(&u));
        let now = crate::state::AppState::now();
        // Type an address: one the server doesn't list by itself (a Tailscale name, before leaving home).
        let add_row = div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(160.))
                            .h(px(32.))
                            .px(px(10.))
                            .flex()
                            .items_center()
                            .rounded(px(ui::R_CTRL))
                            .border_1()
                            .border_color(t.line2)
                            .bg(t.panel)
                            .text_size(px(13.))
                            .text_color(t.ink)
                            .overflow_hidden()
                            .child(div().flex_1().min_w_0().child(gpui_base::Input::new(&self.addr))),
                    )
                    .child(
                        ui::line_btn(t, "st-srv-add", 32.)
                            .px(px(12.))
                            .text_size(px(13.))
                            .child("Add")
                            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.add_address(window, cx))),
                    ),
            )
            .child(match &self.addr_err {
                Some(e) => ui::wrap(e.clone()).text_size(px(12.)).text_color(t.bad),
                None => {
                    ui::wrap("Type an address the app should also try, like a Tailscale name for when you're away.")
                        .text_size(px(12.))
                        .text_color(t.faint)
                }
            })
            .into_any_element();
        if saved.servers.is_empty() {
            let row = ui::wrap("No saved addresses yet. The app looks for the server on your local network and saves the addresses it learns once it connects.")
                .text_size(px(13.))
                .text_color(t.faint)
                .into_any_element();
            return Self::group(t, "Server addresses", Some(sub), vec![row, add_row]);
        }
        let mut rows: Vec<AnyElement> = saved
            .servers
            .iter()
            .enumerate()
            .map(|(i, url)| {
                let note = saved.note(url);
                let learned = note.is_some_and(|n| n.learned);
                let mut words = vec![if learned {
                    match note.and_then(|n| n.kind.as_deref()) {
                        Some("lan") => "Learned: local network".to_string(),
                        Some("bonjour") => "Learned: local network name".to_string(),
                        Some("tailscale") => "Learned: Tailscale".to_string(),
                        Some("magicdns") => "Learned: Tailscale name".to_string(),
                        _ => "Learned".to_string(),
                    }
                } else {
                    "Typed by you".to_string()
                }];
                let using = in_use.as_deref() == Some(sb_core::servers::key(url).as_str());
                let armed = self.addr_removing.as_ref().filter(|(u, _)| u == url).map(|(_, at)| *at);
                let arming = armed.is_some_and(|at| now - at < ARM_FOR);
                if using && arming {
                    words = vec!["The app is connected through this one. Click the X again to remove it.".to_string()];
                } else if using {
                    words.push("in use now".into());
                } else if let Some(f) = note.and_then(|n| n.failing_since) {
                    words.push(format!("not answering since {}", sb_core::fmt::ago(f, now)));
                } else if let Some(ok) = note.and_then(|n| n.last_ok) {
                    words.push(format!("last worked {}", sb_core::fmt::ago(ok, now)));
                }
                let shown = url.strip_prefix("http://").unwrap_or(url).to_string();
                let gone = url.clone();
                let ctl = ui::icon_btn(t, SharedString::from(format!("st-srv-x-{i}")), "x", 26.)
                    .when(arming, |d| d.bg(t.bad.alpha(0.14)))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        let u = gone.clone();
                        // The one in use asks first: without it the app has to find the server again.
                        if using {
                            let at = this.addr_removing.as_ref().filter(|(x, _)| *x == u).map(|(_, at)| *at);
                            let now = crate::state::AppState::now();
                            match arm_click(at, now) {
                                ArmClick::Go => {}
                                ArmClick::Wait => return,
                                ArmClick::Arm => {
                                    this.addr_removing = Some((u, now));
                                    this.disarm_later(cx);
                                    return cx.notify();
                                }
                            }
                        }
                        this.addr_removing = None;
                        this.app.update(cx, |s, cx| s.remove_server(&u, cx));
                    }))
                    .into_any_element();
                div()
                    .child(ui::setting_row(t, shown, Some(SharedString::from(words.join(", "))), Some(ctl)))
                    .with_animation(
                        SharedString::from(format!("st-srv-in-{url}")),
                        gpui::Animation::new(ui::FADE).with_easing(gpui::ease_out_quint()),
                        |d, k| d.opacity(k),
                    )
                    .into_any_element()
            })
            .collect();
        rows.push(add_row);
        Self::group(t, "Server addresses", Some(sub), rows)
    }

    /// One machine: about it, its Claude folder, what Auto may count on it for, its MCP servers, and Remove.
    fn machine_page(&self, t: &Theme, m: ApiMachineRow, window: &Window, cx: &Context<Self>) -> AnyElement {
        let name = m.name.clone();
        let focused = self.field.focus_handle(cx).is_focused(window);
        let os = Self::os_name(m.os.as_deref());
        let back = ui::ghost(t, "st-back", Some("All machines"), Some("back"))
            .h(px(28.))
            .px(px(8.))
            .text_size(px(13.))
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.go(Section::Machines, cx)));
        let n1 = name.clone();
        let label = m.label.clone();
        let title = if self.edit == Some(Edit::Name(name.clone())) {
            self.edit_row(t, ("st-name", "st-name-save", "st-name-x"), focused, cx)
        } else {
            div()
                .flex()
                .items_center()
                .gap(px(12.))
                .child(ui::machine_icon(&name, 30., t.ink2))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(2.))
                        .child(
                            ui::wrap(m.label.clone())
                                .text_size(px(20.))
                                .font_weight(FontWeight::BOLD)
                                .text_color(t.ink),
                        )
                        .child(ui::wrap(Self::status_line(&m)).text_size(px(13.)).text_color(match m.online {
                            Some(true) => t.dim,
                            Some(false) => t.bad,
                            None => t.faint,
                        })),
                )
                .child(ui::line_btn(t, "st-rename", 30.).px(px(12.)).text_size(px(13.)).child("Rename").on_click(
                    cx.listener(move |this, _: &ClickEvent, window, cx| {
                        this.start_edit(Edit::Name(n1.clone()), label.clone(), window, cx)
                    }),
                ))
                .into_any_element()
        };
        let mut body = div().flex().flex_col().child(div().mb(px(10.)).ml(px(-8.)).child(back)).child(title);
        if let Some(e) = &self.error {
            body = body.child(ui::wrap(e.clone()).mt(px(12.)).text_size(px(13.)).text_color(t.bad));
        }

        // About
        let how = if m.via.as_deref() == Some("link") {
            "Linked with switchboard-node".to_string()
        } else {
            format!("Over SSH, {}", m.target.clone().unwrap_or_default())
        };
        let v = m.versions.clone().unwrap_or_default();
        let unknown = if m.online == Some(true) {
            "Its agent doesn't say yet. Update Switchboard there."
        } else {
            "Shows up once it's online."
        };
        body = body.child(Self::group(
            t,
            "About this machine",
            None,
            vec![
                Self::crow(
                    t,
                    "System",
                    None,
                    Some(
                        div()
                            .text_size(px(13.))
                            .text_color(t.dim)
                            .child(if os.is_empty() { "Not known yet".to_string() } else { os.to_string() })
                            .into_any_element(),
                    ),
                ),
                Self::crow(
                    t,
                    "Id",
                    Some("Switchboard's own name for it. It never changes.".into()),
                    Some(
                        div()
                            .font_family(NUM)
                            .text_size(px(13.))
                            .text_color(t.dim)
                            .child(name.clone())
                            .into_any_element(),
                    ),
                ),
                Self::crow(t, "Connection", Some(how), None),
                Self::crow(
                    t,
                    "Claude Code",
                    if v.claude.is_none() { Some(unknown.into()) } else { None },
                    v.claude.clone().map(|c| {
                        div().font_family(NUM).text_size(px(13.)).text_color(t.dim).child(c).into_any_element()
                    }),
                ),
                Self::crow(
                    t,
                    "Switchboard agent",
                    if v.agent.is_none() {
                        Some(unknown.into())
                    } else {
                        Some("The build running there, to compare machines.".into())
                    },
                    v.agent.clone().map(|c| {
                        div().font_family(NUM).text_size(px(13.)).text_color(t.dim).child(c).into_any_element()
                    }),
                ),
            ],
        ));

        // Claude folder
        body = body.child(self.folder_group(t, &m, focused, cx));
        // Beyond it: Anywhere in home
        body = body.child(self.roam_group(t, &m, cx));
        // Capabilities
        body = body.child(self.caps_group(t, &m, window, cx));
        // MCP servers
        body = body.child(self.mcp_group(t, &m, cx));

        // Remove
        let now = AppState::now();
        let arming = self.removing.as_ref().is_some_and(|(n, at)| *n == name && now - at < ARM_FOR);
        let (n3, n4) = (name.clone(), name.clone());
        body = body.child(Self::group(
            t,
            "Remove",
            None,
            vec![Self::crow(
                t,
                "Remove this machine",
                Some("Switchboard stops watching it. Its chats and files stay on it, and you can add it again.".into()),
                Some(
                    ui::line_btn(t, "st-rm", 30.)
                        .px(px(12.))
                        .text_size(px(13.))
                        .text_color(t.bad)
                        .when(arming, |d| d.bg(t.bad.alpha(0.12)))
                        .child(if arming { "Click again to remove" } else { "Remove" })
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            let now = AppState::now();
                            let armed = this.removing.as_ref().filter(|(n, _)| *n == n3).map(|(_, at)| *at);
                            match arm_click(armed, now) {
                                ArmClick::Go => {
                                    this.removing = None;
                                    this.machine = None;
                                    this.send(serde_json::json!({"op": "remove", "machine": n4}), cx);
                                }
                                ArmClick::Wait => {}
                                ArmClick::Arm => {
                                    this.removing = Some((n3.clone(), now));
                                    this.disarm_later(cx);
                                    cx.notify();
                                }
                            }
                        }))
                        .into_any_element(),
                ),
            )],
        ));
        body.child(div().h(px(20.))).into_any_element()
    }

    /// "Anywhere in home": chats and Files may use any folder in the machine's home folder, not only its Claude
    /// folder. Claude still asks before it works in a folder it was never let into.
    fn roam_group(&self, t: &Theme, m: &ApiMachineRow, cx: &Context<Self>) -> AnyElement {
        let knows = m.features.iter().any(|f| f == "places");
        let row = match m.roam {
            Some(on) if knows => {
                let name = m.name.clone();
                ui::option_row(
                    t,
                    "Anywhere in home",
                    "Chats and Files can use any folder in its home folder, like Documents, Desktop or iCloud Drive. Claude asks you before it works in a folder for the first time.",
                    if on {
                        "Off keeps chats and Files in its Claude folder and the places added there."
                    } else {
                        "Off now: chats and Files stay in its Claude folder and the places added there."
                    },
                    ui::switch(t, "st-roam", on, false)
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.send(serde_json::json!({"op": "roam", "machine": name, "roam": !on}), cx)
                        }))
                        .into_any_element(),
                )
                .into_any_element()
            }
            _ if m.online == Some(true) => Self::crow(
                t,
                "Anywhere in home",
                Some(format!("Update Switchboard on {} to let chats and Files go beyond its Claude folder.", m.label)),
                None,
            ),
            _ => Self::crow(t, "Anywhere in home", Some("Shows up once it's online.".into()), None),
        };
        Self::group(t, "Beyond the Claude folder", None, vec![row])
    }

    fn folder_group(&self, t: &Theme, m: &ApiMachineRow, focused: bool, cx: &Context<Self>) -> AnyElement {
        let name = m.name.clone();
        let shown = m.root_shown.clone().unwrap_or_else(|| "Its home folder".into());
        let mut rows = vec![];
        if m.needs_root {
            let sug = m.root_suggest.clone().unwrap_or_else(|| "~".into());
            let (n1, s1) = (name.clone(), sug.clone());
            rows.push(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(10.))
                    .child(
                        div()
                            .flex()
                            .items_start()
                            .gap(px(10.))
                            .child(div().mt(px(1.)).child(ui::icon("info", 15., t.accent)))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .flex()
                                    .flex_col()
                                    .gap(px(3.))
                                    .child(ui::wrap("Where does Claude work on this machine?").text_size(px(13.5)).font_weight(FontWeight::SEMIBOLD).text_color(t.ink))
                                    .child(ui::wrap(format!("No Claude folder is picked yet, so chats start in its home folder. {sug} looks right.")).text_size(px(12.5)).line_height(relative(1.4)).text_color(t.dim)),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap(px(8.))
                            .child(ui::primary_btn(t, "st-root-sug", 30.).px(px(12.)).text_size(px(13.)).child(format!("Use {sug}")).on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.use_folder(n1.clone(), s1.clone(), cx)))),
                    )
                    .into_any_element(),
            );
        }
        if self.edit == Some(Edit::Root(name.clone())) {
            rows.push(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .child(
                        ui::wrap(if m.os.as_deref() == Some("windows") {
                            "A full path, like ~/Claude or C:\\Users\\you\\Claude."
                        } else {
                            "A full path, like ~/Claude or /Users/you/Claude."
                        })
                        .text_size(px(12.5))
                        .text_color(t.faint),
                    )
                    .child(self.edit_row(t, ("st-root", "st-root-save", "st-root-x"), focused, cx))
                    .into_any_element(),
            );
        } else if let Some(p) = self.picker.as_ref().filter(|p| p.machine == name) {
            rows.push(self.picker_view(t, p, cx));
        } else {
            let (n2, n3, root) = (name.clone(), name.clone(), m.root.clone().unwrap_or_default());
            let can_browse = m.has("root_dirs");
            rows.push(Self::crow(
                t,
                shown.clone(),
                Some("New sessions start here or in a folder under it, and stay in it. Files opens here too.".into()),
                Some(
                    div()
                        .flex()
                        .gap(px(6.))
                        .when(can_browse, |d| {
                            d.child(
                                ui::line_btn(t, "st-root-pick", 30.)
                                    .px(px(12.))
                                    .text_size(px(13.))
                                    .child("Choose")
                                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                        this.browse(n2.clone(), String::new(), cx)
                                    })),
                            )
                        })
                        .child(
                            ui::ghost(t, "st-root-type", Some("Type a path"), None)
                                .h(px(30.))
                                .px(px(8.))
                                .text_size(px(13.))
                                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                                    this.picker = None;
                                    this.start_edit(Edit::Root(n3.clone()), root.clone(), window, cx)
                                })),
                        )
                        .into_any_element(),
                ),
            ));
        }
        Self::group(
            t,
            "Claude folder",
            Some(
                "Every chat on this machine starts in this folder or one inside it. Nothing you give a chat can point outside it.",
            ),
            rows,
        )
    }

    fn picker_view(&self, t: &Theme, p: &Picker, cx: &Context<Self>) -> AnyElement {
        let hover = t.hover;
        let machine = p.machine.clone();
        // Breadcrumbs: ~ / Desktop / Claude, each one a step back up.
        let mut crumbs = div().flex().flex_wrap().items_center().gap(px(2.));
        let parts: Vec<&str> = p.path.split('/').filter(|s| !s.is_empty()).collect();
        for i in 0..=parts.len() {
            let to = parts[..i].join("/");
            let label = if i == 0 { "~".to_string() } else { parts[i - 1].to_string() };
            let last = i == parts.len();
            let m2 = machine.clone();
            if i > 0 {
                crumbs = crumbs.child(div().text_size(px(13.)).text_color(t.faint).child("/"));
            }
            crumbs = crumbs.child(
                div()
                    .id(SharedString::from(format!("st-crumb-{i}")))
                    .px(px(5.))
                    .py(px(2.))
                    .rounded(px(ui::R_CHIP))
                    .font_family(NUM)
                    .text_size(px(13.))
                    .text_color(if last { t.ink } else { t.dim })
                    .when(last, |d| d.font_weight(FontWeight::SEMIBOLD))
                    .child(label)
                    .when(!last, |d| {
                        d.cursor_pointer().hover(move |s| s.bg(hover)).on_click(
                            cx.listener(move |this, _: &ClickEvent, _, cx| this.browse(m2.clone(), to.clone(), cx)),
                        )
                    }),
            );
        }
        // Scrolls: a home folder has more folders than fit here, and the ones past the edge must be reachable.
        let mut list = div()
            .id("st-dir-list")
            .flex()
            .flex_col()
            .max_h(px(260.))
            .overflow_y_scroll()
            .track_scroll(&self.dir_scroll);
        match &p.dirs {
            None => list = list.child(ui::loading(t, "st-dirs-load", "Looking at its folders").py(px(10.))),
            Some(Err(e)) => {
                list = list.child(
                    ui::wrap(format!("Couldn't list its folders. {e}"))
                        .py(px(10.))
                        .text_size(px(13.))
                        .text_color(t.bad),
                )
            }
            Some(Ok(d)) if d.folders.is_empty() => {
                list = list.child(div().py(px(10.)).text_size(px(13.)).text_color(t.faint).child("No folders in here."))
            }
            Some(Ok(d)) => {
                for (i, f) in d.folders.iter().enumerate() {
                    let (m2, to) = (machine.clone(), f.path.clone());
                    list = list.child(
                        div()
                            .id(SharedString::from(format!("st-dir-{i}")))
                            .flex_none()
                            .h(px(32.))
                            .px(px(8.))
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .rounded(px(ui::R_ITEM))
                            .cursor_pointer()
                            .hover(move |s| s.bg(hover))
                            .child(ui::icon("folder", 14., t.dim))
                            .child(ui::wrap(f.name.clone()).flex_1().min_w_0().text_size(px(13.)).text_color(t.ink))
                            .when(f.kids, |d| d.child(ui::icon("chev", 12., t.faint)))
                            .on_click(
                                cx.listener(move |this, _: &ClickEvent, _, cx| this.browse(m2.clone(), to.clone(), cx)),
                            ),
                    );
                }
                // The machine lists at most this many.
                if d.folders.len() >= 400 {
                    list = list.child(
                        div()
                            .flex_none()
                            .px(px(8.))
                            .py(px(8.))
                            .text_size(px(12.))
                            .text_color(t.faint)
                            .child("Showing the first 400 folders. For one that isn't here, use Type a path."),
                    );
                }
            }
        }
        let pick = if p.path.is_empty() { "~".to_string() } else { format!("~/{}", p.path) };
        let (m3, pick2) = (machine.clone(), pick.clone());
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(crumbs)
            .child(div().p(px(4.)).rounded(px(ui::R_CTRL)).border_1().border_color(t.line).bg(t.panel).child(list))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(px(8.))
                    .child(
                        ui::wrap("Open a folder to look inside, then use the one you're in.")
                            .flex_1()
                            .min_w(px(180.))
                            .text_size(px(12.5))
                            .text_color(t.faint),
                    )
                    .child(
                        ui::ghost(t, "st-pick-x", Some("Cancel"), None)
                            .h(px(30.))
                            .px(px(10.))
                            .text_size(px(13.))
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.picker = None;
                                cx.notify();
                            })),
                    )
                    .child(
                        ui::primary_btn(t, "st-pick-use", 30.)
                            .px(px(12.))
                            .text_size(px(13.))
                            .child(format!("Use {pick}"))
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                this.use_folder(m3.clone(), pick2.clone(), cx)
                            })),
                    ),
            )
            .into_any_element()
    }

    fn caps_group(&self, t: &Theme, m: &ApiMachineRow, window: &Window, cx: &Context<Self>) -> AnyElement {
        let name = m.name.clone();
        let sub = "What Switchboard found on this machine. The Auto machine pick only counts what's switched on here, plus anything you add.";
        let c = match self.caps.get(&name) {
            None => {
                return Self::group(
                    t,
                    "Capabilities",
                    Some(sub),
                    vec![ui::loading(t, "st-caps-load", "Asking the machine").into_any_element()],
                );
            }
            Some(Err(e)) => {
                return Self::group(
                    t,
                    "Capabilities",
                    Some(sub),
                    vec![
                        ui::wrap(format!("Couldn't ask it. {e}"))
                            .text_size(px(13.))
                            .text_color(t.bad)
                            .into_any_element(),
                    ],
                );
            }
            Some(Ok(c)) => c.clone(),
        };
        let mut rows = vec![];
        let n1 = name.clone();
        let on = c.on;
        rows.push(Self::crow(
            t,
            "Let Auto send work here",
            Some(if on {
                c.can.clone().map(|x| format!("Auto sees: {x}")).unwrap_or_else(|| "Auto counts it in.".into())
            } else {
                "Auto leaves this machine out.".into()
            }),
            Some(
                ui::switch(t, "st-auto", on, false)
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.set_caps(n1.clone(), serde_json::json!({"on": !on}), cx)
                    }))
                    .into_any_element(),
            ),
        ));
        match &c.tools {
            None => rows.push(Self::crow(
                t,
                "Nothing found yet",
                Some(
                    c.error
                        .clone()
                        .map(|e| format!("The machine didn't answer: {e}"))
                        .unwrap_or_else(|| "Shows up once the machine answers.".into()),
                ),
                None,
            )),
            Some(tools) if tools.is_empty() => rows.push(Self::crow(
                t,
                "No developer tools found",
                Some("Auto still counts its free memory and its repos.".into()),
                None,
            )),
            Some(tools) => {
                for tool in tools {
                    let (n2, key, off) = (name.clone(), tool.key.clone(), c.off.clone());
                    let ton = tool.on;
                    rows.push(Self::crow(
                        t,
                        tool.name.clone(),
                        tool.detail.clone(),
                        Some(
                            ui::switch(t, SharedString::from(format!("st-cap-{}", tool.key)), ton, !on)
                                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                    let mut off = off.clone();
                                    if ton {
                                        off.push(key.clone());
                                    } else {
                                        off.retain(|k| k != &key);
                                    }
                                    this.set_caps(n2.clone(), serde_json::json!({"off": off}), cx)
                                }))
                                .into_any_element(),
                        ),
                    ));
                }
            }
        }
        // The user's own lines
        let mut own = div().flex().flex_col().gap(px(8.)).child(
            div()
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(div().text_size(px(13.5)).text_color(t.ink).child("This machine can also do"))
                .child(
                    ui::wrap("Anything detection missed, in your own words. Auto reads these too.")
                        .text_size(px(12.5))
                        .text_color(t.faint),
                ),
        );
        for (i, line) in c.extra.iter().enumerate() {
            let (n3, rest) = (
                name.clone(),
                c.extra.iter().enumerate().filter(|(j, _)| *j != i).map(|(_, x)| x.clone()).collect::<Vec<_>>(),
            );
            own = own.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .pl(px(10.))
                    .pr(px(4.))
                    .py(px(4.))
                    .rounded(px(ui::R_CTRL))
                    .bg(t.panel)
                    .child(ui::icon("check", 13., t.accent))
                    .child(ui::wrap(line.clone()).flex_1().min_w_0().text_size(px(13.)).text_color(t.ink))
                    .child(ui::icon_btn(t, SharedString::from(format!("st-extra-x-{i}")), "x", 24.).on_click(
                        cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.set_caps(n3.clone(), serde_json::json!({"extra": rest.clone()}), cx)
                        }),
                    )),
            );
        }
        let focused = self.extra.focus_handle(cx).is_focused(window);
        own = own.child(
            div()
                .flex()
                .gap(px(8.))
                .child(
                    div()
                        .id("st-extra")
                        .flex_1()
                        .h(px(32.))
                        .px(px(10.))
                        .flex()
                        .items_center()
                        .rounded(px(ui::R_CTRL))
                        .border_1()
                        .border_color(if focused { t.accent } else { t.line2 })
                        .bg(t.bg)
                        .text_size(px(13.))
                        .text_color(t.ink)
                        .child(gpui_base::Input::new(&self.extra)),
                )
                .child(
                    ui::line_btn(t, "st-extra-add", 32.)
                        .px(px(12.))
                        .text_size(px(13.))
                        .child("Add")
                        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.add_extra(window, cx))),
                ),
        );
        rows.push(own.into_any_element());
        Self::group(t, "Capabilities", Some(sub), rows)
    }

    fn mcp_group(&self, t: &Theme, m: &ApiMachineRow, cx: &Context<Self>) -> AnyElement {
        let sub = "Read from Claude Code's settings on this machine: your own, this Claude folder's, and plugins'. Only names and commands show here, and secrets never leave the machine. Add to library makes one a tool any chat can use.";
        let app = self.app.read(cx);
        let lib = app.lib();
        let rows: Vec<AnyElement> = match self.mcp.get(&m.name) {
            None => vec![ui::loading(t, "st-mcp-load", "Asking the machine").into_any_element()],
            Some(Err(e)) => vec![
                ui::wrap(e.clone())
                    .text_size(px(13.))
                    .text_color(if m.online == Some(true) { t.bad } else { t.faint })
                    .into_any_element(),
            ],
            Some(Ok(x)) if x.servers.is_empty() => vec![
                ui::wrap("No MCP servers are set up in Claude Code here.")
                    .text_size(px(13.))
                    .text_color(t.faint)
                    .into_any_element(),
            ],
            Some(Ok(x)) => {
                x.servers
                    .iter()
                    .map(|s| {
                        let mut uses = vec![];
                        if !s.env.is_empty() {
                            uses.push(format!("Uses {}", s.env.join(", ")));
                        }
                        if !s.headers.is_empty() {
                            uses.push(format!("Sends {}", s.headers.join(", ")));
                        }
                        let info =
                            div()
                                .flex_1()
                                .min_w_0()
                                .flex()
                                .flex_col()
                                .gap(px(4.))
                                .child(
                                    div()
                                        .flex()
                                        .flex_wrap()
                                        .items_center()
                                        .gap(px(8.))
                                        .child(
                                            ui::wrap(s.name.clone())
                                                .text_size(px(13.5))
                                                .font_weight(FontWeight::SEMIBOLD)
                                                .text_color(t.ink),
                                        )
                                        .child(ui::tag(t, &s.scope_label()))
                                        .children(s.kind.as_deref().map(|k| ui::tag(t, &k.to_uppercase())))
                                        .children(s.status.as_deref().map(|st| {
                                            div().text_size(px(12.)).text_color(t.dim).child(st.to_string())
                                        })),
                                )
                                .child(
                                    ui::wrap(s.line())
                                        .font_family(NUM)
                                        .text_size(px(12.))
                                        .line_height(relative(1.4))
                                        .text_color(t.dim),
                                )
                                .when(!uses.is_empty(), |d| {
                                    d.child(ui::wrap(uses.join(". ")).text_size(px(12.)).text_color(t.faint))
                                });
                        // Add to library: plugins' servers already come from a plugin, so only the user's own ones.
                        let add = (lib.is_some() && s.scope != "plugin").then(|| {
                            let busy = app.lib_busy.contains(&format!("import:{}/mcp/{}", m.name, s.name));
                            let have = lib.and_then(|l| l.tool(&s.name.to_lowercase())).is_some();
                            let (machine, source) = (m.name.clone(), s.name.clone());
                            ui::line_btn(t, SharedString::from(format!("st-mcp-lib-{}", s.name)), 28.)
                                .px(px(10.))
                                .text_size(px(12.5))
                                .when(busy, |d| d.opacity(0.55))
                                .child(ui::icon("cube", 13., t.dim))
                                .child(if busy {
                                    "Adding"
                                } else if have {
                                    "Update in library"
                                } else {
                                    "Add to library"
                                })
                                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                    if !busy {
                                        this.import_one(machine.clone(), "mcp", source.clone(), cx)
                                    }
                                }))
                        });
                        div().flex().items_start().gap(px(12.)).child(info).children(add).into_any_element()
                    })
                    .collect()
            }
        };
        Self::group(t, "MCP servers", Some(sub), rows)
    }

    fn new_sessions(&self, t: &Theme, cx: &Context<Self>) -> AnyElement {
        let app = self.app.read(cx);
        let model = app.prefs.as_ref().and_then(|p| p.summary_model.clone()).unwrap_or_else(|| "opus".into());
        let presets = app.prefs.as_ref().map(|p| p.presets.clone()).unwrap_or_default();
        let mut seg = ui::seg_box(t);
        for (i, (v, l)) in [("opus", "Opus"), ("sonnet", "Sonnet")].into_iter().enumerate() {
            seg = seg.child(ui::seg_btn(t, ("st-sum", i), model == v, false).child(l).on_click(cx.listener(
                move |this, _: &ClickEvent, _, cx| {
                    this.app.update(cx, |s, cx| {
                        if let Some(p) = s.prefs.as_mut() {
                            p.summary_model = Some(v.to_string());
                        }
                        s.save_prefs(serde_json::json!({"summary_model": v}), Some("Saved".into()), cx)
                    })
                },
            )));
        }
        let mut setups = vec![];
        if presets.is_empty() {
            setups.push(Self::crow(
                t,
                "No saved setups yet",
                Some("Save one from the New sheet: a model, effort, mode and flags you use often.".into()),
                None,
            ));
        }
        let arming = self.preset_removing.filter(|(_, at)| AppState::now() - at < ARM_FOR).map(|(i, _)| i);
        for (i, p) in presets.iter().enumerate() {
            let mut bits = vec![];
            if let Some(m) = &p.model {
                bits.push(sb_core::fmt::model(m));
            }
            if let Some(e) = &p.effort {
                bits.push(format!("{e} effort"));
            }
            if let Some(m) = p.mode.as_deref().filter(|m| *m != "normal") {
                bits.push(m.to_string());
            }
            bits.extend(p.flags.iter().map(|f| f.join(" ")));
            let rest: Vec<_> = presets.iter().enumerate().filter(|(j, _)| *j != i).map(|(_, x)| x.clone()).collect();
            setups.push(Self::crow(
                t,
                p.name.clone().unwrap_or_else(|| "Setup".into()),
                Some(if bits.is_empty() { "The defaults".into() } else { bits.join(", ") }),
                Some(
                    ui::ghost(
                        t,
                        SharedString::from(format!("st-preset-x-{i}")),
                        Some(if arming == Some(i) { "Click again to remove" } else { "Remove" }),
                        None,
                    )
                    .h(px(28.))
                    .px(px(8.))
                    .text_size(px(12.5))
                    .when(arming == Some(i), |d| d.text_color(t.bad))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        // A saved setup can't be brought back, so Remove asks first.
                        let now = AppState::now();
                        let at = this.preset_removing.filter(|(j, _)| *j == i).map(|(_, at)| at);
                        match arm_click(at, now) {
                            ArmClick::Go => this.preset_removing = None,
                            ArmClick::Wait => return,
                            ArmClick::Arm => {
                                this.preset_removing = Some((i, now));
                                this.disarm_later(cx);
                                return cx.notify();
                            }
                        }
                        let rest = rest.clone();
                        this.app.update(cx, |s, cx| {
                            if let Some(p) = s.prefs.as_mut() {
                                p.presets = rest.clone();
                            }
                            s.save_prefs(serde_json::json!({"presets": rest}), Some("Removed".into()), cx)
                        })
                    }))
                    .into_any_element(),
                ),
            ));
        }
        div()
            .child(Self::head(t, "Chats", "What new chats start with, who writes the summary when a chat starts fresh or a thread is compacted, and when an idle chat is stopped."))
            .child(Self::group(t, "Summaries", None, vec![Self::crow(
                t,
                "Who writes them",
                Some("Opus writes the most careful summaries. Sonnet is quicker and costs less of your plan.".into()),
                Some(seg.into_any_element()),
            )]))
            .child(Self::group(t, "Saved setups", Some("The New sheet offers these, along with Defaults and Last used."), setups))
            .children(self.idle_stop_row(t, cx))
            .into_any_element()
    }

    fn notifications(&self, t: &Theme, cx: &Context<Self>) -> AnyElement {
        let on = self.app.read(cx).notify_on();
        let mac = Self::crow(
            t,
            "When a chat needs you",
            Some("A plain notification with the chat's name and the machine. Click it to open the board.".into()),
            Some(
                ui::switch(t, "st-notify", on, false)
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.app.update(cx, |s, cx| {
                            s.settings.notify = Some(!on);
                            s.save_settings();
                            cx.notify();
                        })
                    }))
                    .into_any_element(),
            ),
        );
        let prefs = self.app.read(cx).prefs.clone();
        let phone: Vec<AnyElement> = match &prefs {
            None => vec![ui::loading(t, "st-n-kinds", "Asking the server").into_any_element()],
            Some(p) if p.notify.is_empty() => vec![Self::crow(
                t,
                "Which ones come",
                Some("Update Switchboard on the server to pick the kinds of iPhone notifications.".into()),
                None,
            )],
            Some(p) => p
                .notify
                .iter()
                .map(|n| {
                    let (key, on) = (n.key.clone(), n.on);
                    Self::crow(
                        t,
                        n.label.clone(),
                        (!n.explain.is_empty()).then(|| n.explain.clone()),
                        Some(
                            ui::switch(t, SharedString::from(format!("st-n-{}", n.key)), on, false)
                                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                    let body = sb_core::api::notify_body(&key, !on);
                                    let key = key.clone();
                                    this.app.update(cx, |s, cx| {
                                        if let Some(p) = &mut s.prefs {
                                            p.set_notify(&key, !on);
                                        }
                                        s.save_prefs(body, None, cx);
                                    });
                                    cx.notify();
                                }))
                                .into_any_element(),
                        ),
                    )
                })
                .collect(),
        };
        div()
            .child(Self::head(
                t,
                "Notifications",
                if ui::MAC {
                    "What reaches you, here on this Mac and on your iPhone. Each is its own switch."
                } else {
                    "What reaches you, here on this computer and on your iPhone. Each is its own switch."
                },
            ))
            .child(Self::group(
                t,
                if ui::MAC { "On this Mac" } else { "On this computer" },
                Some("Shown by this app, while it is open."),
                vec![mac],
            ))
            .child(Self::group(
                t,
                "On your iPhone",
                Some("Sent by the server to iPhones signed in to Switchboard, even when the app is closed."),
                phone,
            ))
            .child(div().h(px(18.)))
            .child(self.push_block(t, "st-n-push", true, cx))
            .into_any_element()
    }

    /// iPhone notifications: what they are, and while the server has no push key, where it goes (from /api/prefs).
    /// `card`: the shared not-set-up card (the Notifications page); otherwise a row on a group's card.
    fn push_block(&self, t: &Theme, id: &'static str, card: bool, cx: &Context<Self>) -> AnyElement {
        let push = self.app.read(cx).prefs.as_ref().and_then(|p| p.push.clone());
        let Some(push) = push else {
            return ui::loading(t, id, "Asking the server about iPhone notifications").into_any_element();
        };
        if push.ready {
            return ui::setting_row(t, "iPhone notifications", Some(push.explain.into()), None).into_any_element();
        }
        let setup = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(6.))
            .text_size(px(12.5))
            .text_color(t.ink2)
            .child("Set up on the server:")
            .child(
                div()
                    .px(px(6.))
                    .py(px(2.))
                    .rounded(px(ui::R_MIN))
                    .bg(t.panel3)
                    .font_family(NUM)
                    .text_size(px(12.))
                    .child(push.setup.clone().unwrap_or_default()),
            );
        let guide = SharedString::from(format!("{id}-guide"));
        let open_guide = |_: &ClickEvent, _: &mut Window, cx: &mut App| {
            crate::state::open_link(cx, &sb_core::brand::docs("apps/#iphone-notifications"))
        };
        let action = div().flex().flex_col().gap(px(10.)).child(setup).child(div().flex().child(if card {
            ui::setup_btn(t, guide, "Open the setup guide").on_click(open_guide)
        } else {
            ui::link(t, guide, "Setup guide").text_size(px(12.5)).on_click(open_guide)
        }));
        if card {
            return ui::setup_card(
                t,
                "bolt",
                "iPhone notifications aren't set up",
                push.explain,
                Some(action.into_any_element()),
            )
            .into_any_element();
        }
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(ui::setting_row(t, "iPhone notifications", Some(push.explain.into()), None))
            .child(action)
            .into_any_element()
    }

    /// AI and your Claude setup: the opt-in switches, in two groups, as /api/prefs words them (server/options.py).
    fn privacy(&self, t: &Theme, cx: &Context<Self>) -> AnyElement {
        let head = Self::head(
            t,
            "AI and your Claude setup",
            "Each switch says what it sends or changes. A newly set up server starts with all of them off.",
        );
        let prefs = self.app.read(cx).prefs.clone();
        let Some(prefs) = prefs.filter(|p| !p.options.is_empty()) else {
            let body = if self.app.read(cx).prefs.is_some() {
                ui::setup_card(
                    t,
                    "info",
                    "This server has no switches yet",
                    "Update the server to choose what Switchboard sends to Claude and what it changes on each machine.",
                    None,
                )
                .into_any_element()
            } else {
                ui::loading(t, "st-pv-load", "Reading your choices from the server").into_any_element()
            };
            return div().child(head).child(body).into_any_element();
        };
        let group = |kind: &str, title: &str| {
            let mut rows: Vec<AnyElement> = prefs
                .options_of(kind)
                // where long chats are compacted has its own page, with a slider (Context window)
                .filter(|o| o.key != "setup_compact_window")
                .map(|o| {
                    let (key, on) = (o.key.clone(), o.on);
                    let sw = ui::switch(t, SharedString::from(format!("st-opt-{key}")), on, false).on_click(
                        cx.listener(move |this, _: &ClickEvent, _, cx| {
                            let key = key.clone();
                            this.app.update(cx, |s, cx| {
                                if let Some(p) = s.prefs.as_mut() {
                                    p.set_option(&key, !on);
                                }
                                s.save_prefs(sb_core::api::option_body(&key, !on), Some("Saved".into()), cx)
                            })
                        }),
                    );
                    ui::option_row(t, o.label.clone(), o.explain.clone(), o.off.clone(), sw.into_any_element())
                        .into_any_element()
                })
                .collect();
            if kind == "setup" && prefs.option("setup_compact_window").is_some() {
                let note = "Also written to each machine's Claude Code settings. It has its own page: Context window.";
                rows.push(Self::crow(t, "Where long chats are compacted", Some(note.into()), None));
            }
            (!rows.is_empty()).then(|| Self::group(t, title, None, rows))
        };
        div()
            .child(head)
            .children(group("ai", "Sends chat text to Claude"))
            .children(group("setup", "Changes to each machine's Claude setup"))
            .child(Self::group(
                t,
                "iPhone notifications",
                Some("The same switch as on the Notifications page: turning it on sends a line about the chat to your iPhone."),
                vec![self.push_block(t, "st-pv-push", false, cx)],
            ))
            .into_any_element()
    }

    /// A percent setting: a switch (off is null) and, when on, a stepper with the number between.
    #[allow(clippy::too_many_arguments)]
    fn pct_ctl(
        &self,
        t: &Theme,
        id: &'static str,
        v: Option<f64>,
        def: f64,
        step: f64,
        mk: fn(Option<f64>) -> serde_json::Value,
        busy: bool,
        cx: &Context<Self>,
    ) -> AnyElement {
        let send = move |this: &mut Self, nv: Option<f64>, cx: &mut Context<Self>| {
            this.app.update(cx, |s, cx| s.budget_op(mk(nv), cx))
        };
        let mut row = div().flex().items_center().gap(px(10.));
        if let Some(x) = v {
            let (dn, up) = pct_steps(x, step);
            let btn = |sfx: &str, label: &'static str, off: bool| {
                ui::seg_btn(t, SharedString::from(format!("{id}-{sfx}")), false, off || busy)
                    .w(px(28.))
                    .justify_center()
                    .text_size(px(15.))
                    .child(label)
            };
            row = row.child(
                ui::seg_box(t)
                    .child(
                        btn("dn", "\u{2212}", dn >= x)
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| send(this, Some(dn), cx))),
                    )
                    .child(
                        div()
                            .w(px(58.))
                            .text_center()
                            .text_size(px(13.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .font_features(ui::tnum())
                            .text_color(t.ink)
                            .child(pct(x)),
                    )
                    .child(
                        btn("up", "+", up <= x)
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| send(this, Some(up), cx))),
                    ),
            );
        } else {
            row = row.child(div().text_size(px(13.)).text_color(t.faint).child("Off"));
        }
        let on = v.is_some();
        row.child(ui::switch(t, SharedString::from(format!("{id}-sw")), on, busy).on_click(
            cx.listener(move |this, _: &ClickEvent, _, cx| send(this, if on { None } else { Some(def) }, cx)),
        ))
        .into_any_element()
    }

    /// Budget: a daily share of the week, an optional 5-hour cap, and the hard limit that stops chats.
    fn budget(&self, t: &Theme, cx: &Context<Self>) -> AnyElement {
        let app = self.app.read(cx);
        let head = Self::head(
            t,
            "Budget",
            "A daily share of your weekly limit. Switchboard tells you when a day goes over it, and can stop chats.",
        );
        let Some(b) = app.snap.budget.clone() else {
            return div()
                .child(head)
                .child(Self::group(
                    t,
                    "Not on this server yet",
                    None,
                    vec![Self::crow(
                        t,
                        "The server has no budget",
                        Some("Update Switchboard on the server to set one.".into()),
                        None,
                    )],
                ))
                .into_any_element();
        };
        let busy = app.budget_busy;
        let (s, st) = (b.settings(), b.status());
        let hard = s.hard();
        let hard_on = hard.on == Some(true);
        // A plan that reports no 5-hour window gives a 5-hour cap nothing to check: say so, or it would never fire
        // without a word.
        let five_help = if app.snap.plan.five_unreported(AppState::now()) {
            "Your plan reports no 5-hour window right now, so this cannot be checked."
        } else {
            "Percent of the 5-hour window. Off unless you set one."
        };
        let mut body = div().child(head);

        // Where today stands: lifted (with Undo), or the hard limit reached (with Lift for today).
        if st.lifted() {
            body = body.child(
                div()
                    .mt(px(18.))
                    .p(px(14.))
                    .rounded(px(ui::R_CARD))
                    .border_1()
                    .border_color(t.line)
                    .bg(t.bg)
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .child(ui::icon("unlock", 16., t.cold))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(2.))
                            .child(
                                div()
                                    .text_size(px(13.5))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(t.ink)
                                    .child("Lifted until midnight"),
                            )
                            .child(
                                ui::wrap("The hard limit won't stop anything else today.")
                                    .text_size(px(12.5))
                                    .text_color(t.faint),
                            ),
                    )
                    .child(ui::line_btn(t, "bud-unlift", 30.).px(px(12.)).text_size(px(13.)).child("Undo").on_click(
                        cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.app.update(cx, |s, cx| s.budget_op(serde_json::json!({"op": "unlift"}), cx))
                        }),
                    )),
            );
        } else if st.over_hard() {
            let why = if st.hard_reason.as_deref() == Some("five_hour") {
                "The 5-hour window is past its hard limit."
            } else {
                "Today is past its hard limit."
            };
            body = body.child(
                div()
                    .mt(px(18.))
                    .p(px(14.))
                    .rounded(px(ui::R_CARD))
                    .border_1()
                    .border_color(t.bad.alpha(0.5))
                    .bg(t.bad.alpha(if t.dark { 0.12 } else { 0.07 }))
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .child(ui::icon("lock", 18., t.bad))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(2.))
                            .child(
                                div()
                                    .text_size(px(14.))
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(t.ink)
                                    .child("Hard limit reached"),
                            )
                            .child(
                                ui::wrap(format!(
                                    "{why} New messages are blocked, and background chats that were working got stopped."
                                ))
                                .text_size(px(12.5))
                                .line_height(relative(1.4))
                                .text_color(t.dim),
                            ),
                    )
                    .child(
                        ui::primary_btn(t, "bud-lift", 32.)
                            .px(px(14.))
                            .text_size(px(13.))
                            .child("Lift for today")
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.app.update(cx, |s, cx| s.budget_op(serde_json::json!({"op": "lift"}), cx))
                            })),
                    ),
            );
        }
        if let Some(sum) = st.summary.clone().filter(|x| !x.is_empty()) {
            body = body.child(
                ui::wrap(sum)
                    .mt(px(14.))
                    .text_size(px(12.5))
                    .line_height(relative(1.45))
                    .text_color(if st.over_budget() { t.cold } else { t.dim }),
            );
        }

        body = body.child(Self::group(
            t,
            "Budget",
            Some("Over budget: a chat tells you once a day."),
            vec![
                Self::crow(
                    t,
                    "Daily budget",
                    Some("Percent of the week per day. 14.3% spreads the week evenly over 7 days.".into()),
                    Some(self.pct_ctl(
                        t,
                        "bud-daily",
                        s.daily,
                        14.3,
                        1.0,
                        |v| serde_json::json!({"op": "set", "daily": v}),
                        busy,
                        cx,
                    )),
                ),
                Self::crow(
                    t,
                    "5-hour cap",
                    Some(five_help.into()),
                    Some(self.pct_ctl(
                        t,
                        "bud-five",
                        s.five_hour,
                        50.0,
                        5.0,
                        |v| serde_json::json!({"op": "set", "five_hour": v}),
                        busy,
                        cx,
                    )),
                ),
            ],
        ));
        let mut hard_rows = vec![Self::crow(
            t,
            "Hard limit",
            Some("Hard limit: blocks new messages and stops running background chats. You can resume them.".into()),
            Some(
                ui::switch(t, "bud-hard-sw", hard_on, busy)
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.app.update(cx, |s, cx| {
                            s.budget_op(serde_json::json!({"op": "set", "hard": {"on": !hard_on}}), cx)
                        })
                    }))
                    .into_any_element(),
            ),
        )];
        if hard_on {
            hard_rows.push(Self::crow(
                t,
                "Daily limit",
                Some("Percent of the week per day.".into()),
                Some(self.pct_ctl(
                    t,
                    "bud-hdaily",
                    hard.daily,
                    20.0,
                    1.0,
                    |v| serde_json::json!({"op": "set", "hard": {"daily": v}}),
                    busy,
                    cx,
                )),
            ));
            hard_rows.push(Self::crow(
                t,
                "5-hour limit",
                Some(five_help.into()),
                Some(self.pct_ctl(
                    t,
                    "bud-hfive",
                    hard.five_hour,
                    80.0,
                    5.0,
                    |v| serde_json::json!({"op": "set", "hard": {"five_hour": v}}),
                    busy,
                    cx,
                )),
            ));
        }
        body.child(Self::group(t, "Hard limit", Some("Off unless you turn it on."), hard_rows)).into_any_element()
    }

    /// Claude in Chrome on each machine. There is no machine for all chats: each chat picks its own Chrome machine
    /// (the New sheet's Chrome line, or the chat's detail sheet), and Start fresh and Continue keep it.
    fn chrome(&self, t: &Theme, cx: &Context<Self>) -> AnyElement {
        let app = self.app.read(cx);
        let mut list = div().flex().flex_col();
        for m in &app.snap.machines {
            let (state, ok) = match &m.chrome {
                Some(c) if c.connected => ("Claude in Chrome is ready", true),
                Some(c) if c.connector => ("Claude in Chrome is set up, not connected right now", true),
                Some(c) if c.installed => ("Chrome is there, but not the extension", false),
                _ => ("No Claude in Chrome there", false),
            };
            list = list.child(
                div()
                    .py(px(10.))
                    .px(px(10.))
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .child(ui::icon("globe", 16., if ok { t.accent } else { t.faint }))
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .gap(px(1.))
                            .child(div().text_size(px(14.)).text_color(t.ink).child(m.label.clone()))
                            .child(div().text_size(px(12.5)).text_color(t.faint).child(state)),
                    ),
            );
        }
        let none = !app.snap.machines.iter().any(|m| m.chrome.as_ref().is_some_and(|c| c.connector));
        let card = none.then(|| {
            div().mb(px(14.)).child(ui::setup_card(
                t,
                "globe",
                "Claude in Chrome isn't set up",
                "None of your machines has the Claude in Chrome extension. Install it in Chrome on a machine, and that machine shows up here.",
                Some(
                    ui::setup_btn(t, "st-chrome-guide", "Open the guide")
                        .on_click(|_, _, cx| crate::state::open_link(cx, &sb_core::brand::docs("settings/#chrome-machine")))
                        .into_any_element(),
                ),
            ))
        });
        div()
            .child(Self::head(t, "Chrome", "Each chat picks its own Chrome machine: in the New sheet's Chrome line, or in the chat's details. Start fresh and Continue keep it."))
            .children(card)
            .child(list)
            .into_any_element()
    }
}

impl SettingsView {
    /// Updates: whether this build gets them, when it last looked, and Check now. A build without an update key
    /// says updates are off and links to how a build gets them.
    fn updates(&self, t: &Theme, cx: &Context<Self>) -> AnyElement {
        let app = self.app.read(cx);
        let off = sb_core::update::off_reason().is_some();
        let ctl = if off {
            ui::ghost(t, "st-upd-how", Some("How builds get updates"), None)
                .h(px(32.))
                .px(px(10.))
                .text_size(px(13.))
                .on_click(|_, _, cx| {
                    crate::state::open_link(cx, &sb_core::brand::docs("updates/#builds-without-updates"))
                })
                .into_any_element()
        } else {
            ui::ghost(t, "st-upd-check", Some(if app.update_busy { "Checking" } else { "Check now" }), None)
                .h(px(32.))
                .px(px(10.))
                .text_size(px(13.))
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.app.update(cx, |s, cx| s.look_for_update(cx))))
                .into_any_element()
        };
        // A version that was put back because it didn't start is not offered again until asked for.
        let ctl = if !off && app.rolled_back.is_some() && app.update.is_none() {
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .child(
                    ui::line_btn(t, "st-upd-retry", 32.).px(px(12.)).text_size(px(13.)).child("Try it again").on_click(
                        cx.listener(|this, _: &ClickEvent, _, cx| this.app.update(cx, |s, cx| s.retry_rolled_back(cx))),
                    ),
                )
                .child(ctl)
                .into_any_element()
        } else {
            ctl
        };
        div()
            .child(Self::head(
                t,
                "Updates",
                &format!(
                    "Switchboard {}. Updates never change your settings or what you chose when you set it up.",
                    sb_core::version()
                ),
            ))
            .child(Self::row(t, "New versions", Some(&crate::update::status_line(app)), ctl))
            .into_any_element()
    }
}

impl Focusable for SettingsView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for SettingsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body = self.body(window, cx);
        crate::frame::framed(window, &Theme::get(cx), "Settings", body)
    }
}

impl SettingsView {
    /// One line of the sidebar: an icon and a name, washed when it is the page shown. A keyboard stop.
    fn nav_item(t: &Theme, id: &str, label: &'static str, ic: &str, on: bool) -> gpui::Stateful<gpui::Div> {
        let hover = t.hover;
        ui::focusable(div().id(SharedString::from(id.to_string())), t)
            .flex_none()
            .h(px(32.))
            .px(px(10.))
            .flex()
            .items_center()
            .gap(px(10.))
            .rounded(px(ui::R_CTRL))
            .cursor_pointer()
            .text_size(px(13.5))
            .when(on, |d| d.bg(t.sel).text_color(t.ink).font_weight(FontWeight::SEMIBOLD))
            .when(!on, |d| d.text_color(t.ink2).hover(move |st| st.bg(hover)))
            .child(if ic == "mac" {
                ui::machine_icon("", 15., if on { t.ink } else { t.dim })
            } else {
                ui::icon(ic, 15., if on { t.ink } else { t.dim })
            })
            .child(ui::line(label))
    }

    /// A section's line in the sidebar.
    fn nav_row(&self, t: &Theme, s: Section, cx: &Context<Self>) -> gpui::Stateful<gpui::Div> {
        let (label, ic) = s.nav();
        let on = self.section == s && (s != Section::Machines || self.machine.is_none());
        Self::nav_item(t, &format!("st-nav-{label}"), label, ic, on)
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.go(s, cx)))
    }

    fn body(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = Theme::get(cx);
        let hover = t.hover;
        // Screenshots: SB_SHOT_SCROLL=<px> shows a page further down (a machine's Capabilities and MCP servers, or the
        // Machines list's server addresses).
        #[cfg(feature = "shots")]
        if let Some(y) = std::env::var("SB_SHOT_SCROLL").ok().and_then(|v| v.parse::<f32>().ok()) {
            let list = (self.section == Section::Machines && self.machine.is_none() && self.machines.is_some())
                || self.section == Section::Server;
            if list || self.machine.as_ref().is_some_and(|m| self.caps.contains_key(m) && self.mcp.contains_key(m)) {
                self.scroll.set_offset(gpui::point(px(0.), px(-y)));
            }
        }
        let mut nav = div()
            .id("st-nav")
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&self.nav_scroll)
            .pt(px(14.))
            .pb(px(10.))
            .px(px(10.))
            .flex()
            .flex_col()
            .gap(px(2.));
        for (i, (group, sections)) in NAV.into_iter().enumerate() {
            nav = nav.child(
                div()
                    .flex_none()
                    .h(px(24.))
                    .when(i > 0, |d| d.mt(px(12.)))
                    .px(px(10.))
                    .flex()
                    .items_center()
                    .text_size(px(11.5))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(t.faint)
                    .child(group),
            );
            for &s in sections {
                nav = nav.child(self.nav_row(&t, s, cx));
                // Under Machines, each machine: its own page.
                if s == Section::Machines
                    && let Some(Ok(ms)) = &self.machines
                {
                    for m in &ms.machines {
                        let mon = self.section == Section::Machines && self.machine.as_deref() == Some(m.name.as_str());
                        let name = m.name.clone();
                        nav = nav.child(
                            ui::focusable(div().id(SharedString::from(format!("st-nav-m-{}", m.name))), &t)
                                .flex_none()
                                .min_h(px(30.))
                                .py(px(5.))
                                .pl(px(30.))
                                .pr(px(10.))
                                .flex()
                                .items_center()
                                .gap(px(8.))
                                .rounded(px(ui::R_CTRL))
                                .cursor_pointer()
                                .text_size(px(13.))
                                .when(mon, |d| d.bg(t.sel).text_color(t.ink).font_weight(FontWeight::SEMIBOLD))
                                .when(!mon, |d| d.text_color(t.dim).hover(move |st| st.bg(hover)))
                                .child(div().flex_none().size(px(7.)).rounded_full().bg(match m.online {
                                    Some(true) => t.warm,
                                    Some(false) => t.bad,
                                    None => t.cold,
                                }))
                                .child(ui::wrap(m.label.clone()).flex_1().min_w_0())
                                .when(m.needs_root, |d| d.child(ui::icon("info", 13., t.accent)))
                                .on_click(
                                    cx.listener(move |this, _: &ClickEvent, _, cx| this.open_machine(name.clone(), cx)),
                                ),
                        );
                    }
                }
            }
        }
        // The app itself, at the bottom edge whatever the window's height: the list above scrolls, this stays.
        let mut foot =
            div().flex_none().px(px(10.)).py(px(8.)).flex().flex_col().gap(px(2.)).border_t_1().border_color(t.line);
        for s in NAV_FOOT {
            foot = foot.child(self.nav_row(&t, s, cx));
        }
        let foot = foot.child(
            Self::nav_item(&t, "st-nav-about", "About", "info", false)
                .on_click(|_: &ClickEvent, _, cx| cx.defer(crate::about::open)),
        );
        let nav = div()
            .flex_none()
            .w(px(220.))
            .h_full()
            .flex()
            .flex_col()
            .bg(t.bg)
            .border_r_1()
            .border_color(t.line)
            .child(div().relative().flex_1().min_h_0().child(nav).child(ui::scrollbar(&self.nav_scroll)))
            .child(foot);
        let body = match self.section {
            Section::Appearance => self.appearance(&t, cx),
            Section::Terminal => self.terminal(&t, cx),
            Section::NewSessions => self.new_sessions(&t, cx),
            Section::Context => {
                let app = self.app.clone();
                let page = window.use_keyed_state("st-context", cx, |window, cx| ContextPage::new(app, window, cx));
                page.into_any_element()
            }
            Section::Machines => self.machines(&t, window, cx),
            Section::Library => self.library(&t, window, cx),
            Section::Notifications => self.notifications(&t, cx),
            Section::Chrome => self.chrome(&t, cx),
            Section::Budget => self.budget(&t, cx),
            Section::Updates => self.updates(&t, cx),
            Section::Server => div()
                .child(self.server(&t, cx))
                .child(div().h(px(18.)))
                .child(self.server_group(&t, cx))
                .into_any_element(),
            Section::Privacy => self.privacy(&t, cx),
        };
        div()
            .id("settings")
            .key_context("Settings")
            .track_focus(&self.focus)
            .on_action(|_: &crate::view::FocusNext, window, cx| window.focus_next(cx))
            .on_action(|_: &crate::view::FocusPrev, window, cx| window.focus_prev(cx))
            .size_full()
            .flex()
            .bg(t.panel)
            .font_family(crate::theme::UI)
            .text_color(t.ink)
            .child(nav)
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .child(
                        div()
                            .id("st-body")
                            .size_full()
                            .overflow_y_scroll()
                            .track_scroll(&self.scroll)
                            .px(px(32.))
                            .py(px(26.))
                            .child(body),
                    )
                    .child(ui::scrollbar(&self.scroll)),
            )
    }
}

/// "14.3%", "20%".
fn pct(x: f64) -> String {
    if (x - x.round()).abs() < 0.05 { format!("{}%", x.round() as i64) } else { format!("{x:.1}%") }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_section_is_in_the_sidebar_once() {
        let all = [
            Section::Appearance,
            Section::Terminal,
            Section::NewSessions,
            Section::Context,
            Section::Machines,
            Section::Library,
            Section::Notifications,
            Section::Chrome,
            Section::Budget,
            Section::Updates,
            Section::Server,
            Section::Privacy,
        ];
        let shown: Vec<Section> = NAV.iter().flat_map(|(_, s)| s.iter().copied()).chain(NAV_FOOT).collect();
        for s in all {
            assert_eq!(shown.iter().filter(|x| **x == s).count(), 1, "{s:?}");
            assert!(!s.nav().0.is_empty());
        }
        assert_eq!(shown.len(), all.len());
        assert!(NAV.iter().all(|(name, s)| !name.is_empty() && !s.is_empty()));
    }

    #[test]
    fn a_click_again_button_wants_a_real_second_click() {
        assert_eq!(arm_click(None, 100.), ArmClick::Arm);
        assert_eq!(arm_click(Some(100.), 100.1), ArmClick::Wait, "the second half of a double-click");
        assert_eq!(arm_click(Some(100.), 100.5), ArmClick::Go);
        assert_eq!(arm_click(Some(100.), 103.9), ArmClick::Go);
        assert_eq!(arm_click(Some(100.), 104.1), ArmClick::Arm, "it went back to its first label: this arms again");
        assert_eq!(arm_click(Some(100.), 99.), ArmClick::Arm, "the clock moved back");
    }

    #[test]
    fn budget_steps_land_on_round_numbers() {
        assert_eq!(pct_steps(14.3, 1.), (14., 15.));
        assert_eq!(pct_steps(14., 1.), (13., 15.));
        assert_eq!(pct_steps(50., 5.), (45., 55.));
        assert_eq!(pct_steps(52.5, 5.), (50., 55.));
        assert_eq!(pct_steps(100., 5.), (95., 100.));
        assert_eq!(pct_steps(99.5, 1.), (99., 100.));
        assert_eq!(pct_steps(1., 1.), (1., 2.), "not under 1");
        assert_eq!(pct_steps(3., 5.), (1., 5.));
        assert_eq!(pct_steps(0.5, 1.), (0.5, 1.), "a smaller value set elsewhere only goes up");
    }

    #[test]
    fn when_to_use_fits_what_the_server_takes() {
        assert_eq!(when_problem(&"x".repeat(300)), None);
        assert!(when_problem(&"\u{e9}".repeat(301)).unwrap().ends_with("This is 301."));
    }
}
