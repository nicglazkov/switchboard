//! Settings (Cmd+Comma): Appearance (light, system or dark), Terminal (scheme, text size, cursor, blink), New sessions
//! (who writes summaries, saved setups), Machines (rename, reorder, remove, add one over SSH or with Add a machine) with
//! a page for each machine (its Claude folder, picked by browsing it; its versions; what Auto may count on it for; its
//! MCP servers, each one a click from the library), Library (settings_library.rs), Notifications and the Chrome
//! machine. All of it lives on the server, so every device agrees.

use gpui::{
    AnyElement, App, AppContext, ClickEvent, Context, Entity, FocusHandle, Focusable, FontWeight, InteractiveElement, IntoElement, ParentElement,
    Render, ScrollHandle, SharedString, StatefulInteractiveElement, Styled, Subscription, Window, div, prelude::*, px, relative,
};
use gpui_base::input::{InputEvent, InputState, TextareaState};
use sb_core::api::{ApiCapsView, ApiMachineRow, ApiMachines, ApiMcp, ApiRootDirs};
use std::collections::{HashMap, HashSet};

#[path = "settings_library.rs"]
mod library_ui;

use crate::state::AppState;
use crate::term::colors;
use crate::theme::{Mode, NUM, Theme};
use crate::ui;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Section {
    Appearance,
    Terminal,
    NewSessions,
    Machines,
    Library,
    Notifications,
    Chrome,
    Budget,
}

/// About is not here: it is its own window (about.rs), from Switchboard > About Switchboard.
const SECTIONS: [(Section, &str, &str); 8] = [
    (Section::Appearance, "Appearance", "sun"),
    (Section::Terminal, "Terminal", "term"),
    (Section::NewSessions, "New sessions", "plus"),
    (Section::Machines, "Machines", "mac"),
    (Section::Library, "Library", "cube"),
    (Section::Notifications, "Notifications", "bolt"),
    (Section::Chrome, "Chrome machine", "globe"),
    (Section::Budget, "Budget", "gauge"),
];

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
    let opts = crate::files::options("Settings, Switchboard", 900., 680., (720., 500.), cx);
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
            TextareaState::new(window, cx).placeholder("What it does, what it needs, and how you would check it works.").auto_grow(3, 8)
        });
        let lib_secret = cx.new(|cx| InputState::new(window, cx).masked(true).placeholder("Paste the value"));
        let subs = vec![
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
            adding: false,
            add_os: "mac",
            field,
            add_id,
            add_label,
            add_target,
            error: None,
            scroll: ScrollHandle::new(),
            lib_import: None,
            lib_new: None,
            lib_name,
            lib_brief,
            lib_hist: HashSet::new(),
            lib_key: None,
            lib_secret,
            lib_key_err: None,
            lib_deleting: None,
            _subs: subs,
        };
        v.load(cx);
        v.app.update(cx, |s, cx| s.load_library(cx));
        v
    }

    pub fn go(&mut self, s: Section, cx: &mut Context<Self>) {
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
    fn browse(&mut self, machine: String, path: String, cx: &mut Context<Self>) {
        self.picker = Some(Picker { machine: machine.clone(), path: path.clone(), dirs: None });
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
                    Err(e) => v.error = Some(e.to_string()),
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
                let auto = self.app.read(cx).lib().and_then(|l| l.tool(&tool)).and_then(|x| x.auto).unwrap_or(false);
                self.app.update(cx, |s, cx| s.lib_auto(&tool, auto, &v, cx));
                self.edit = None;
                cx.notify();
            }
            None => {}
        }
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

    fn head(t: &Theme, title: &str, sub: &str) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .gap(px(4.))
            .mb(px(18.))
            .child(div().text_size(px(20.)).font_weight(FontWeight::BOLD).text_color(t.ink).child(title.to_string()))
            .child(ui::wrap(sub.to_string()).text_size(px(13.)).line_height(relative(1.45)).text_color(t.dim))
            .into_any_element()
    }

    fn row(t: &Theme, label: &str, note: Option<&str>, ctl: AnyElement) -> AnyElement {
        div()
            .py(px(12.))
            .flex()
            .items_center()
            .gap(px(16.))
            .border_b_1()
            .border_color(t.line)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(div().text_size(px(14.)).font_weight(FontWeight::SEMIBOLD).text_color(t.ink).child(label.to_string()))
                    .children(note.map(|n| ui::wrap(n.to_string()).text_size(px(12.5)).line_height(relative(1.4)).text_color(t.faint))),
            )
            .child(ctl)
            .into_any_element()
    }

    fn appearance(&self, t: &Theme, cx: &Context<Self>) -> AnyElement {
        let mode = Mode::parse(self.app.read(cx).settings.theme.as_deref());
        let mut seg = ui::seg_box(t);
        for (i, (m, label, ic)) in [(Mode::Light, "Light", "sun"), (Mode::System, "System", "contrast"), (Mode::Dark, "Dark", "moon")].into_iter().enumerate() {
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
                if ui::MAC { "How Switchboard looks. Synced, so your other Macs and the iPhone app match." } else { "How Switchboard looks. Synced, so your other computers and the iPhone app match." },
            ))
            .child(Self::row(t, "Theme", Some(if ui::MAC { "Also in the header, and ⌘⇧L switches light and dark." } else { "Also in the header, and Ctrl+Shift+L switches light and dark." }), seg.into_any_element()))
            .into_any_element()
    }

    fn terminal(&self, t: &Theme, cx: &Context<Self>) -> AnyElement {
        let look = colors::look(cx);
        let hover = t.hover;
        let mut grid = div().flex().flex_wrap().gap(px(10.));
        let mut all: Vec<(String, String, colors::Palette)> = colors::scheme::all().iter().map(|s| (s.id.clone(), s.name.clone(), colors::palette_of(s))).collect();
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
                    .child(div().flex().items_center().gap(px(6.)).child(div().flex_1().text_size(px(13.)).text_color(if on { t.ink } else { t.ink2 }).child(name)).when(on, |d| d.child(ui::icon("check", 13., t.accent))))
                    .on_hover(cx.listener(move |_, h: &bool, _, cx| colors::set_preview(h.then(|| peek.clone()), cx)))
                    .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                        colors::set_preview(None, cx);
                        crate::theme::set_term_colors(&pick, cx);
                    })),
            );
        }
        let size = look.size.unwrap_or(sb_core::term::scheme::SIZE_DEFAULT);
        let step = |id: &'static str, ic: &'static str, up: bool| {
            ui::icon_btn(t, id, ic, 28.).shadow(vec![Theme::ring(t.line2)]).on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                let next = sb_core::term::scheme::step_size(colors::look(cx).size, up);
                crate::theme::save_look(cx, |s| s.term_size = Some(next));
            }))
        };
        let mut cursors = ui::seg_box(t);
        for (i, (v, label)) in [("block", "Block"), ("bar", "Bar"), ("underline", "Underline")].into_iter().enumerate() {
            cursors = cursors.child(ui::seg_btn(t, ("st-cur", i), look.cursor == v, false).child(label).on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                crate::theme::save_look(cx, |s| s.term_cursor = Some(v.to_string()))
            })));
        }
        let blink = look.blink;
        div()
            .child(Self::head(t, "Terminal", "The terminal keeps its own colors, because Claude Code paints for a dark terminal. Point at a scheme to see it in every open terminal."))
            .child(grid)
            .child(div().h(px(10.)))
            .child(Self::row(
                t,
                "Text size",
                Some(&format!("{} and {} in a terminal.", ui::mod_cap("+"), ui::mod_cap("-"))),
                div().flex().items_center().gap(px(8.)).child(step("st-smaller", "zoomout", false)).child(div().w(px(32.)).flex().justify_center().font_family(NUM).text_size(px(14.)).text_color(t.ink).child(size.to_string())).child(step("st-bigger", "zoomin", true)).into_any_element(),
            ))
            .child(Self::row(t, "Cursor", None, cursors.into_any_element()))
            .child(Self::row(
                t,
                "Blink",
                None,
                ui::seg_box(t)
                    .child(ui::seg_btn(t, "st-blink-on", blink, false).child("On").on_click(cx.listener(|_, _: &ClickEvent, _, cx| crate::theme::save_look(cx, |s| s.term_blink = Some(true)))))
                    .child(ui::seg_btn(t, "st-blink-off", !blink, false).child("Off").on_click(cx.listener(|_, _: &ClickEvent, _, cx| crate::theme::save_look(cx, |s| s.term_blink = Some(false)))))
                    .into_any_element(),
            ))
            .into_any_element()
    }

    /// A titled group of rows on a card, like the Mac's own Settings.
    fn group(t: &Theme, title: &str, sub: Option<&str>, rows: Vec<AnyElement>) -> AnyElement {
        let mut card = div().rounded(px(ui::R_CARD)).border_1().border_color(t.line).bg(t.bg).flex().flex_col();
        for (i, r) in rows.into_iter().enumerate() {
            card = card.child(div().px(px(14.)).py(px(11.)).when(i > 0, |d| d.border_t_1().border_color(t.line)).child(r));
        }
        div()
            .mt(px(22.))
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(
                div()
                    .px(px(2.))
                    .flex()
                    .flex_col()
                    .gap(px(3.))
                    .child(div().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).text_color(t.ink2).child(title.to_string()))
                    .children(sub.map(|s| ui::wrap(s.to_string()).text_size(px(12.5)).line_height(relative(1.45)).text_color(t.faint))),
            )
            .child(card)
            .into_any_element()
    }

    /// One row on a group's card: a label (and a note under it) and its control on the right.
    fn crow(t: &Theme, label: impl Into<SharedString>, note: Option<String>, ctl: Option<AnyElement>) -> AnyElement {
        div()
            .flex()
            .items_center()
            .gap(px(16.))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(ui::wrap(label.into()).text_size(px(13.5)).text_color(t.ink))
                    .children(note.map(|n| ui::wrap(n).text_size(px(12.5)).line_height(relative(1.4)).text_color(t.faint))),
            )
            .children(ctl.map(|c| div().flex_none().child(c)))
            .into_any_element()
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

    fn edit_row(&self, t: &Theme, ids: (&'static str, &'static str, &'static str), focused: bool, cx: &Context<Self>) -> AnyElement {
        div()
            .flex()
            .flex_wrap()
            .gap(px(8.))
            .child(self.text_field(t, ids.0, focused))
            .child(ui::primary_btn(t, ids.1, 32.).px(px(12.)).text_size(px(13.)).child("Save").on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.save_edit(cx))))
            .child(ui::ghost(t, ids.2, Some("Cancel"), None).h(px(32.)).px(px(10.)).text_size(px(13.)).on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.edit = None;
                cx.notify();
            })))
            .into_any_element()
    }

    fn machines(&self, t: &Theme, window: &Window, cx: &Context<Self>) -> AnyElement {
        if let Some(m) = self.machine.as_deref().and_then(|n| self.row_of(n)) {
            return self.machine_page(t, m, window, cx);
        }
        let mut body = div().child(Self::head(t, "Machines", "Every machine Switchboard watches, in the order the board shows them. Open one to set its Claude folder, see what it can do, and its MCP servers."));
        let list = match &self.machines {
            None => return body.child(div().text_size(px(13.)).text_color(t.faint).child("Loading the machines")).into_any_element(),
            Some(Err(e)) => return body.child(ui::wrap(format!("Could not load the machines. {e}")).text_size(px(13.)).text_color(t.bad)).into_any_element(),
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
                    .child(div().flex_none().relative().child(ui::machine_icon(&m.name, 22., t.dim)).child(
                        div().absolute().right(px(-2.)).bottom(px(-1.)).size(px(9.)).rounded_full().border_2().border_color(t.bg).bg(match m.online {
                            Some(true) => t.warm,
                            Some(false) => t.bad,
                            None => t.cold,
                        }),
                    ))
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
                                    .child(ui::wrap(m.label.clone()).text_size(px(14.5)).font_weight(FontWeight::BOLD).text_color(t.ink))
                                    .child(div().text_size(px(12.5)).text_color(t.faint).child(if os.is_empty() { m.name.clone() } else { format!("{os}, {}", m.name) }))
                                    .when(m.needs_root, |d| d.child(ui::note_tag(t, "folder", "Pick its Claude folder"))),
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
                            .child(ui::wrap(match Self::versions_line(&m) {
                                Some(v) => format!("{}. {v}", Self::status_line(&m)),
                                None => Self::status_line(&m),
                            }).text_size(px(12.)).text_color(t.faint)),
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
            div().w(px(w)).h(px(32.)).px(px(10.)).flex().items_center().rounded(px(ui::R_CTRL)).border_1().border_color(t.line2).bg(t.bg).text_size(px(13.)).text_color(t.ink).child(gpui_base::Input::new(inp))
        };
        let labeled = |l: &str, el: AnyElement| div().flex().flex_col().gap(px(5.)).child(div().text_size(px(12.)).text_color(t.dim).child(l.to_string())).child(el);
        if self.adding {
            let mut oss = ui::seg_box(t);
            for (i, (v, l)) in [("mac", "macOS"), ("linux", "Linux"), ("windows", "Windows")].into_iter().enumerate() {
                oss = oss.child(ui::seg_btn(t, ("st-os", i), self.add_os == v, false).child(l).on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.add_os = v;
                    cx.notify();
                })));
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
                    .child(ui::wrap("Switchboard reaches it the way you would: ssh to the address, with your keys already set up. Next you pick its Claude folder.").text_size(px(12.5)).text_color(t.dim))
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
                    .child(ui::primary_btn(t, "st-pair", 34.).px(px(14.)).text_size(px(13.)).child(ui::icon("plus", 14., t.on_accent)).child("Add a machine").on_click(cx.listener(|_, _: &ClickEvent, _, cx| crate::files::open_pair(cx))))
                    .child(ui::line_btn(t, "st-ssh", 34.).px(px(14.)).text_size(px(13.)).child("Add one over SSH").on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.adding = true;
                        cx.notify();
                    }))),
            );
        }
        body.into_any_element()
    }

    /// One machine: about it, its Claude folder, what Auto may count on it for, its MCP servers, and Remove.
    fn machine_page(&self, t: &Theme, m: ApiMachineRow, window: &Window, cx: &Context<Self>) -> AnyElement {
        let name = m.name.clone();
        let focused = self.field.focus_handle(cx).is_focused(window);
        let os = Self::os_name(m.os.as_deref());
        let back = ui::ghost(t, "st-back", Some("All machines"), Some("back")).h(px(28.)).px(px(8.)).text_size(px(13.)).on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.go(Section::Machines, cx)));
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
                        .child(ui::wrap(m.label.clone()).text_size(px(20.)).font_weight(FontWeight::BOLD).text_color(t.ink))
                        .child(ui::wrap(Self::status_line(&m)).text_size(px(13.)).text_color(match m.online {
                            Some(true) => t.dim,
                            Some(false) => t.bad,
                            None => t.faint,
                        })),
                )
                .child(ui::line_btn(t, "st-rename", 30.).px(px(12.)).text_size(px(13.)).child("Rename").on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                    this.start_edit(Edit::Name(n1.clone()), label.clone(), window, cx)
                })))
                .into_any_element()
        };
        let mut body = div().flex().flex_col().child(div().mb(px(10.)).ml(px(-8.)).child(back)).child(title);
        if let Some(e) = &self.error {
            body = body.child(ui::wrap(e.clone()).mt(px(12.)).text_size(px(13.)).text_color(t.bad));
        }

        // About
        let how = if m.via.as_deref() == Some("link") { "Linked with switchboard-node".to_string() } else { format!("Over SSH, {}", m.target.clone().unwrap_or_default()) };
        let v = m.versions.clone().unwrap_or_default();
        let unknown = if m.online == Some(true) { "Its agent does not say yet. Update Switchboard there." } else { "Shown once it is online." };
        body = body.child(Self::group(t, "About this machine", None, vec![
            Self::crow(t, "System", None, Some(div().text_size(px(13.)).text_color(t.dim).child(if os.is_empty() { "Not known yet".to_string() } else { os.to_string() }).into_any_element())),
            Self::crow(t, "Id", Some("Switchboard's own name for it. It never changes.".into()), Some(div().font_family(NUM).text_size(px(13.)).text_color(t.dim).child(name.clone()).into_any_element())),
            Self::crow(t, "Connection", Some(how), None),
            Self::crow(t, "Claude Code", if v.claude.is_none() { Some(unknown.into()) } else { None }, v.claude.clone().map(|c| div().font_family(NUM).text_size(px(13.)).text_color(t.dim).child(c).into_any_element())),
            Self::crow(t, "Switchboard agent", if v.agent.is_none() { Some(unknown.into()) } else { Some("The build running there, to compare machines.".into()) }, v.agent.clone().map(|c| div().font_family(NUM).text_size(px(13.)).text_color(t.dim).child(c).into_any_element())),
        ]));

        // Claude folder
        body = body.child(self.folder_group(t, &m, focused, cx));
        // Capabilities
        body = body.child(self.caps_group(t, &m, window, cx));
        // MCP servers
        body = body.child(self.mcp_group(t, &m, cx));

        // Remove
        let now = AppState::now();
        let arming = self.removing.as_ref().is_some_and(|(n, at)| *n == name && now - at < 4.);
        let (n3, n4) = (name.clone(), name.clone());
        body = body.child(Self::group(t, "Remove", None, vec![Self::crow(
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
                        if this.removing.as_ref().is_some_and(|(n, at)| *n == n3 && now - at < 4.) {
                            this.removing = None;
                            this.machine = None;
                            this.send(serde_json::json!({"op": "remove", "machine": n4}), cx);
                        } else {
                            this.removing = Some((n3.clone(), now));
                            cx.notify();
                        }
                    }))
                    .into_any_element(),
            ),
        )]));
        body.child(div().h(px(20.))).into_any_element()
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
                                    .child(ui::wrap(format!("Nobody has picked its Claude folder yet, so sessions start in its home folder. {sug} looks right.")).text_size(px(12.5)).line_height(relative(1.4)).text_color(t.dim)),
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
                    .child(ui::wrap(if m.os.as_deref() == Some("windows") { "A full path, like ~/Claude or C:\\Users\\you\\Claude." } else { "A full path, like ~/Claude or /Users/you/Claude." }).text_size(px(12.5)).text_color(t.faint))
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
                            d.child(ui::line_btn(t, "st-root-pick", 30.).px(px(12.)).text_size(px(13.)).child("Choose").on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                this.browse(n2.clone(), String::new(), cx)
                            })))
                        })
                        .child(ui::ghost(t, "st-root-type", Some("Type a path"), None).h(px(30.)).px(px(8.)).text_size(px(13.)).on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                            this.picker = None;
                            this.start_edit(Edit::Root(n3.clone()), root.clone(), window, cx)
                        })))
                        .into_any_element(),
                ),
            ));
        }
        Self::group(t, "Claude folder", Some("Every session on this machine starts in this folder or one under it. Nothing a session is given can step outside it."), rows)
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
                    .when(!last, |d| d.cursor_pointer().hover(move |s| s.bg(hover)).on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.browse(m2.clone(), to.clone(), cx)))),
            );
        }
        let mut list = div().flex().flex_col().max_h(px(260.)).overflow_hidden();
        match &p.dirs {
            None => list = list.child(div().py(px(10.)).text_size(px(13.)).text_color(t.faint).child("Looking at its folders")),
            Some(Err(e)) => list = list.child(ui::wrap(format!("Could not list its folders. {e}")).py(px(10.)).text_size(px(13.)).text_color(t.bad)),
            Some(Ok(d)) if d.folders.is_empty() => list = list.child(div().py(px(10.)).text_size(px(13.)).text_color(t.faint).child("No folders in here.")),
            Some(Ok(d)) => {
                for (i, f) in d.folders.iter().enumerate() {
                    let (m2, to) = (machine.clone(), f.path.clone());
                    list = list.child(
                        div()
                            .id(SharedString::from(format!("st-dir-{i}")))
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
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.browse(m2.clone(), to.clone(), cx))),
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
                    .child(ui::wrap("Open a folder to look inside, then use the one you are in.").flex_1().min_w(px(180.)).text_size(px(12.5)).text_color(t.faint))
                    .child(ui::ghost(t, "st-pick-x", Some("Cancel"), None).h(px(30.)).px(px(10.)).text_size(px(13.)).on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.picker = None;
                        cx.notify();
                    })))
                    .child(ui::primary_btn(t, "st-pick-use", 30.).px(px(12.)).text_size(px(13.)).child(format!("Use {pick}")).on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.use_folder(m3.clone(), pick2.clone(), cx)))),
            )
            .into_any_element()
    }

    fn caps_group(&self, t: &Theme, m: &ApiMachineRow, window: &Window, cx: &Context<Self>) -> AnyElement {
        let name = m.name.clone();
        let sub = "What Switchboard found on this machine. The Auto machine pick only counts what is switched on here, plus anything you add.";
        let c = match self.caps.get(&name) {
            None => return Self::group(t, "Capabilities", Some(sub), vec![div().text_size(px(13.)).text_color(t.faint).child("Asking the machine").into_any_element()]),
            Some(Err(e)) => return Self::group(t, "Capabilities", Some(sub), vec![ui::wrap(format!("Could not ask it. {e}")).text_size(px(13.)).text_color(t.bad).into_any_element()]),
            Some(Ok(c)) => c.clone(),
        };
        let mut rows = vec![];
        let n1 = name.clone();
        let on = c.on;
        rows.push(Self::crow(
            t,
            "Let Auto send work here",
            Some(if on { c.can.clone().map(|x| format!("Auto sees: {x}")).unwrap_or_else(|| "Auto counts it in.".into()) } else { "Auto leaves this machine out.".into() }),
            Some(ui::switch(t, "st-auto", on, false).on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.set_caps(n1.clone(), serde_json::json!({"on": !on}), cx))).into_any_element()),
        ));
        match &c.tools {
            None => rows.push(Self::crow(t, "Nothing found yet", Some(c.error.clone().map(|e| format!("The machine did not answer: {e}")).unwrap_or_else(|| "Shown once the machine answers.".into())), None)),
            Some(tools) if tools.is_empty() => rows.push(Self::crow(t, "No developer tools found", Some("Auto still counts its free memory and its repos.".into()), None)),
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
                .child(ui::wrap("Anything detection missed, in your own words. Auto reads these too.").text_size(px(12.5)).text_color(t.faint)),
        );
        for (i, line) in c.extra.iter().enumerate() {
            let (n3, rest) = (name.clone(), c.extra.iter().enumerate().filter(|(j, _)| *j != i).map(|(_, x)| x.clone()).collect::<Vec<_>>());
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
                    .child(ui::icon_btn(t, SharedString::from(format!("st-extra-x-{i}")), "x", 24.).on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.set_caps(n3.clone(), serde_json::json!({"extra": rest.clone()}), cx)
                    }))),
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
                .child(ui::line_btn(t, "st-extra-add", 32.).px(px(12.)).text_size(px(13.)).child("Add").on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.add_extra(window, cx)))),
        );
        rows.push(own.into_any_element());
        Self::group(t, "Capabilities", Some(sub), rows)
    }

    fn mcp_group(&self, t: &Theme, m: &ApiMachineRow, cx: &Context<Self>) -> AnyElement {
        let sub = "Read from Claude Code's settings on this machine: your own, this Claude folder's, and plugins'. Only names and commands are shown; secrets never leave the machine. Add to library makes one a tool any chat can use.";
        let app = self.app.read(cx);
        let lib = app.lib();
        let rows: Vec<AnyElement> = match self.mcp.get(&m.name) {
            None => vec![div().text_size(px(13.)).text_color(t.faint).child("Asking the machine").into_any_element()],
            Some(Err(e)) => vec![ui::wrap(e.clone()).text_size(px(13.)).text_color(if m.online == Some(true) { t.bad } else { t.faint }).into_any_element()],
            Some(Ok(x)) if x.servers.is_empty() => vec![ui::wrap("No MCP servers are set up in Claude Code here.").text_size(px(13.)).text_color(t.faint).into_any_element()],
            Some(Ok(x)) => x
                .servers
                .iter()
                .map(|s| {
                    let mut uses = vec![];
                    if !s.env.is_empty() {
                        uses.push(format!("Uses {}", s.env.join(", ")));
                    }
                    if !s.headers.is_empty() {
                        uses.push(format!("Sends {}", s.headers.join(", ")));
                    }
                    let info = div()
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
                                .child(ui::wrap(s.name.clone()).text_size(px(13.5)).font_weight(FontWeight::SEMIBOLD).text_color(t.ink))
                                .child(ui::tag(t, &s.scope_label()))
                                .children(s.kind.as_deref().map(|k| ui::tag(t, &k.to_uppercase())))
                                .children(s.status.as_deref().map(|st| div().text_size(px(12.)).text_color(t.dim).child(st.to_string()))),
                        )
                        .child(ui::wrap(s.line()).font_family(NUM).text_size(px(12.)).line_height(relative(1.4)).text_color(t.dim))
                        .when(!uses.is_empty(), |d| d.child(ui::wrap(uses.join(". ")).text_size(px(12.)).text_color(t.faint)));
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
                            .child(if busy { "Adding" } else if have { "Update in library" } else { "Add to library" })
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                if !busy {
                                    this.import_one(machine.clone(), "mcp", source.clone(), cx)
                                }
                            }))
                    });
                    div().flex().items_start().gap(px(12.)).child(info).children(add).into_any_element()
                })
                .collect(),
        };
        Self::group(t, "MCP servers", Some(sub), rows)
    }

    fn new_sessions(&self, t: &Theme, cx: &Context<Self>) -> AnyElement {
        let app = self.app.read(cx);
        let model = app.prefs.as_ref().and_then(|p| p.summary_model.clone()).unwrap_or_else(|| "opus".into());
        let presets = app.prefs.as_ref().map(|p| p.presets.clone()).unwrap_or_default();
        let mut seg = ui::seg_box(t);
        for (i, (v, l)) in [("opus", "Opus"), ("sonnet", "Sonnet")].into_iter().enumerate() {
            seg = seg.child(ui::seg_btn(t, ("st-sum", i), model == v, false).child(l).on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.app.update(cx, |s, cx| {
                    if let Some(p) = s.prefs.as_mut() {
                        p.summary_model = Some(v.to_string());
                    }
                    s.save_prefs(serde_json::json!({"summary_model": v}), Some("Saved".into()), cx)
                })
            })));
        }
        let mut setups = vec![];
        if presets.is_empty() {
            setups.push(Self::crow(t, "No saved setups yet", Some("Save one from the New sheet: a model, effort, mode and flags you use often.".into()), None));
        }
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
                    ui::ghost(t, SharedString::from(format!("st-preset-x-{i}")), Some("Remove"), None)
                        .h(px(28.))
                        .px(px(8.))
                        .text_size(px(12.5))
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
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
            .child(Self::head(t, "New sessions", "What new chats start with, and who writes the summary when a chat starts fresh or a thread is compacted."))
            .child(Self::group(t, "Summaries", None, vec![Self::crow(
                t,
                "Who writes them",
                Some("Opus writes the most careful summaries. Sonnet is quicker and costs less of your plan.".into()),
                Some(seg.into_any_element()),
            )]))
            .child(Self::group(t, "Saved setups", Some("The New sheet offers these, along with Defaults and Last used."), setups))
            .into_any_element()
    }

    fn notifications(&self, t: &Theme, cx: &Context<Self>) -> AnyElement {
        let on = self.app.read(cx).notify_on();
        div()
            .child(Self::head(t, "Notifications", "A plain notification when a chat starts waiting for you. Nothing else."))
            .child(Self::row(
                t,
                "When a chat needs you",
                Some("Shows the chat's name and the machine. Click it to open the board."),
                ui::seg_box(t)
                    .child(ui::seg_btn(t, "st-n-on", on, false).child("On").on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.app.update(cx, |s, cx| {
                            s.settings.notify = Some(true);
                            s.save_settings();
                            cx.notify();
                        })
                    })))
                    .child(ui::seg_btn(t, "st-n-off", !on, false).child("Off").on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.app.update(cx, |s, cx| {
                            s.settings.notify = Some(false);
                            s.save_settings();
                            cx.notify();
                        })
                    })))
                    .into_any_element(),
            ))
            .into_any_element()
    }

    /// A percent setting: a switch (off is null) and, when on, a stepper with the number between.
    #[allow(clippy::too_many_arguments)]
    fn pct_ctl(&self, t: &Theme, id: &'static str, v: Option<f64>, def: f64, step: f64, mk: fn(Option<f64>) -> serde_json::Value, busy: bool, cx: &Context<Self>) -> AnyElement {
        let send = move |this: &mut Self, nv: Option<f64>, cx: &mut Context<Self>| this.app.update(cx, |s, cx| s.budget_op(mk(nv), cx));
        let mut row = div().flex().items_center().gap(px(10.));
        if let Some(x) = v {
            let dn = ((x - step).max(0.1) * 10.).round() / 10.;
            let up = ((x + step).min(100.) * 10.).round() / 10.;
            let btn = |sfx: &str, label: &'static str, off: bool| {
                ui::seg_btn(t, SharedString::from(format!("{id}-{sfx}")), false, off || busy).w(px(28.)).justify_center().text_size(px(15.)).child(label)
            };
            row = row.child(
                ui::seg_box(t)
                    .child(btn("dn", "\u{2212}", x <= 0.1).on_click(cx.listener(move |this, _: &ClickEvent, _, cx| send(this, Some(dn), cx))))
                    .child(div().w(px(58.)).text_center().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).font_features(ui::tnum()).text_color(t.ink).child(pct(x)))
                    .child(btn("up", "+", x >= 100.).on_click(cx.listener(move |this, _: &ClickEvent, _, cx| send(this, Some(up), cx)))),
            );
        } else {
            row = row.child(div().text_size(px(13.)).text_color(t.faint).child("Off"));
        }
        let on = v.is_some();
        row.child(
            ui::switch(t, SharedString::from(format!("{id}-sw")), on, busy)
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| send(this, if on { None } else { Some(def) }, cx))),
        )
        .into_any_element()
    }

    /// Budget: a daily share of the week, an optional 5-hour cap, and the hard limit that stops chats.
    fn budget(&self, t: &Theme, cx: &Context<Self>) -> AnyElement {
        let app = self.app.read(cx);
        let head = Self::head(t, "Budget", "A daily share of your weekly limit. Switchboard tells you when a day goes over it, and can stop chats.");
        let Some(b) = app.snap.budget.clone() else {
            return div()
                .child(head)
                .child(Self::group(t, "Not on this server yet", None, vec![Self::crow(t, "The server has no budget", Some("Update Switchboard on the server to set one.".into()), None)]))
                .into_any_element();
        };
        let busy = app.budget_busy;
        let (s, st) = (b.settings(), b.status());
        let hard = s.hard();
        let hard_on = hard.on == Some(true);
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
                            .child(div().text_size(px(13.5)).font_weight(FontWeight::SEMIBOLD).text_color(t.ink).child("Lifted until midnight"))
                            .child(ui::wrap("The hard limit stops nothing more today.").text_size(px(12.5)).text_color(t.faint)),
                    )
                    .child(ui::line_btn(t, "bud-unlift", 30.).px(px(12.)).text_size(px(13.)).child("Undo").on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.app.update(cx, |s, cx| s.budget_op(serde_json::json!({"op": "unlift"}), cx))
                    }))),
            );
        } else if st.over_hard() {
            let why = if st.hard_reason.as_deref() == Some("five_hour") { "The 5-hour window is past its hard limit." } else { "Today is past its hard limit." };
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
                            .child(div().text_size(px(14.)).font_weight(FontWeight::BOLD).text_color(t.ink).child("Hard limit reached"))
                            .child(ui::wrap(format!("{why} New messages are blocked and working background chats were stopped.")).text_size(px(12.5)).line_height(relative(1.4)).text_color(t.dim)),
                    )
                    .child(ui::primary_btn(t, "bud-lift", 32.).px(px(14.)).text_size(px(13.)).child("Lift for today").on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.app.update(cx, |s, cx| s.budget_op(serde_json::json!({"op": "lift"}), cx))
                    }))),
            );
        }
        if let Some(sum) = st.summary.clone().filter(|x| !x.is_empty()) {
            body = body.child(ui::wrap(sum).mt(px(14.)).px(px(2.)).text_size(px(12.5)).line_height(relative(1.45)).text_color(if st.over_budget() { t.cold } else { t.dim }));
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
                    Some(self.pct_ctl(t, "bud-daily", s.daily, 14.3, 1.0, |v| serde_json::json!({"op": "set", "daily": v}), busy, cx)),
                ),
                Self::crow(
                    t,
                    "5-hour cap",
                    Some("Percent of the 5-hour window. Off unless you set one.".into()),
                    Some(self.pct_ctl(t, "bud-five", s.five_hour, 50.0, 5.0, |v| serde_json::json!({"op": "set", "five_hour": v}), busy, cx)),
                ),
            ],
        ));
        let mut hard_rows = vec![Self::crow(
            t,
            "Hard limit",
            Some("Hard limit: blocks new messages and stops running background chats; they can be resumed.".into()),
            Some(
                ui::switch(t, "bud-hard-sw", hard_on, busy)
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.app.update(cx, |s, cx| s.budget_op(serde_json::json!({"op": "set", "hard": {"on": !hard_on}}), cx))
                    }))
                    .into_any_element(),
            ),
        )];
        if hard_on {
            hard_rows.push(Self::crow(
                t,
                "Daily limit",
                Some("Percent of the week per day.".into()),
                Some(self.pct_ctl(t, "bud-hdaily", hard.daily, 20.0, 1.0, |v| serde_json::json!({"op": "set", "hard": {"daily": v}}), busy, cx)),
            ));
            hard_rows.push(Self::crow(
                t,
                "5-hour limit",
                Some("Percent of the 5-hour window. Off unless you set one.".into()),
                Some(self.pct_ctl(t, "bud-hfive", hard.five_hour, 80.0, 5.0, |v| serde_json::json!({"op": "set", "hard": {"five_hour": v}}), busy, cx)),
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
                Some(c) if c.installed => ("Chrome is there, the extension is not", false),
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
                    .child(div().flex_1().flex().flex_col().gap(px(1.)).child(div().text_size(px(14.)).text_color(t.ink).child(m.label.clone())).child(div().text_size(px(12.5)).text_color(t.faint).child(state))),
            );
        }
        div()
            .child(Self::head(t, "Chrome", "Each chat picks its own Chrome machine: in the New sheet's Chrome line, or in the chat's details. Start fresh and Continue keep it."))
            .child(list)
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
    fn body(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = Theme::get(cx);
        let hover = t.hover;
        // Screenshots: SB_SHOT_SCROLL=<px> shows a page further down (a machine's Capabilities and MCP servers).
        #[cfg(feature = "shots")]
        if let Some(y) = std::env::var("SB_SHOT_SCROLL").ok().and_then(|v| v.parse::<f32>().ok()) {
            if self.machine.as_ref().is_some_and(|m| self.caps.contains_key(m) && self.mcp.contains_key(m)) {
                self.scroll.set_offset(gpui::point(px(0.), px(-y)));
            }
        }
        let mut nav = div().id("st-nav").flex_none().w(px(220.)).h_full().overflow_y_scroll().pt(px(18.)).px(px(10.)).flex().flex_col().gap(px(2.)).bg(t.bg).border_r_1().border_color(t.line);
        for (s, label, ic) in SECTIONS {
            let on = self.section == s && (s != Section::Machines || self.machine.is_none());
            nav = nav.child(
                div()
                    .id(SharedString::from(format!("st-nav-{label}")))
                    .h(px(34.))
                    .px(px(10.))
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .rounded(px(ui::R_CTRL))
                    .cursor_pointer()
                    .text_size(px(13.5))
                    .when(on, |d| d.bg(t.sel).text_color(t.ink).font_weight(FontWeight::SEMIBOLD))
                    .when(!on, |d| d.text_color(t.ink2).hover(move |st| st.bg(hover)))
                    .child(if s == Section::Machines { ui::machine_icon("", 15., if on { t.ink } else { t.dim }) } else { ui::icon(ic, 15., if on { t.ink } else { t.dim }) })
                    .child(label)
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.go(s, cx))),
            );
            // Under Machines, each machine: its own page.
            if s == Section::Machines {
                if let Some(Ok(ms)) = &self.machines {
                    for m in &ms.machines {
                        let mon = self.section == Section::Machines && self.machine.as_deref() == Some(m.name.as_str());
                        let name = m.name.clone();
                        nav = nav.child(
                            div()
                                .id(SharedString::from(format!("st-nav-m-{}", m.name)))
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
                                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.open_machine(name.clone(), cx))),
                        );
                    }
                }
            }
        }
        let body = match self.section {
            Section::Appearance => self.appearance(&t, cx),
            Section::Terminal => self.terminal(&t, cx),
            Section::NewSessions => self.new_sessions(&t, cx),
            Section::Machines => self.machines(&t, window, cx),
            Section::Library => self.library(&t, window, cx),
            Section::Notifications => self.notifications(&t, cx),
            Section::Chrome => self.chrome(&t, cx),
            Section::Budget => self.budget(&t, cx),
        };
        div()
            .id("settings")
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .bg(t.panel)
            .font_family(crate::theme::UI)
            .text_color(t.ink)
            .child(nav)
            .child(div().id("st-body").flex_1().min_w_0().h_full().overflow_y_scroll().track_scroll(&self.scroll).px(px(32.)).py(px(26.)).child(body))
    }
}

/// "14.3%", "20%".
fn pct(x: f64) -> String {
    if (x - x.round()).abs() < 0.05 { format!("{}%", x.round() as i64) } else { format!("{x:.1}%") }
}
