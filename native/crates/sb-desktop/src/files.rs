//! v2: Files, a Mac-like browser of each machine (every machine one click away). A sidebar of places (the Claude
//! folder, Home and its usual folders, iCloud Drive, the places added at the machine), each with its state; a path
//! bar; folders first, then files with their icon, size and date; and a preview pane (keys and secrets are listed
//! but never opened). The keyboard: arrows, Enter, Left or Cmd+Up for the folder above, Space for a quick look.
//! New chat here, and the chats in that folder with their terminal one click away (a file's path can go straight
//! into one). Folders macOS keeps closed, a missing place, "Anywhere in home" off and an older agent each say so
//! calmly, with the one button that helps. And Add a machine, a pairing code and the one line that links a new
//! machine. Each opens in a window of its own.

use gpui::{
    AnyElement, App, AppContext, ClickEvent, Context, Entity, FocusHandle, FontWeight, Hsla, InteractiveElement,
    IntoElement, ParentElement, Render, ScrollHandle, SharedString, StatefulInteractiveElement, Styled, Subscription,
    TitlebarOptions, Window, WindowBackgroundAppearance, WindowBounds, WindowDecorations, WindowOptions, div,
    prelude::*, px, relative, size,
};
use sb_core::api::{ApiFile, ApiPair, ApiPlace};
use sb_core::fmt;
use sb_core::model::Machine;
use sb_core::places::{self, FileKind};

use crate::nav::{Closed, Nav};
use crate::state::{AppEvent, AppState, copy_text};
use crate::theme::{NUM, Theme};
use crate::ui;

const PREVIEW_LINES: usize = 2000;
const SIDEBAR_W: f32 = 214.;
const PANE_W: f32 = 380.;
const ROW_H: f32 = 32.;
/// The list's size and date columns.
const SIZE_W: f32 = 76.;
const DATE_W: f32 = 92.;

gpui::actions!(files, [FxUp, FxDown, FxOpen, FxParent, FxInto, FxLook]);

pub fn bind_keys(cx: &mut App) {
    let f = Some("Files");
    cx.bind_keys([
        gpui::KeyBinding::new("up", FxUp, f),
        gpui::KeyBinding::new("down", FxDown, f),
        gpui::KeyBinding::new("enter", FxOpen, f),
        gpui::KeyBinding::new("right", FxInto, f),
        gpui::KeyBinding::new("left", FxParent, f),
        gpui::KeyBinding::new("backspace", FxParent, f),
        gpui::KeyBinding::new("secondary-up", FxParent, f),
        gpui::KeyBinding::new("secondary-down", FxOpen, f),
        gpui::KeyBinding::new("space", FxLook, f),
    ]);
}

pub(crate) fn files_ok(m: &Machine) -> bool {
    m.online != Some(false) && m.features.iter().any(|f| f == "files")
}

pub(crate) fn options(title: &str, w: f32, h: f32, min: (f32, f32), cx: &App) -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(gpui::Bounds::centered(None, size(px(w), px(h)), cx))),
        titlebar: Some(TitlebarOptions {
            title: Some(title.to_string().into()),
            appears_transparent: false,
            traffic_light_position: None,
        }),
        window_min_size: Some(size(px(min.0), px(min.1))),
        app_id: Some("switchboard".into()),
        window_background: WindowBackgroundAppearance::Opaque,
        window_decorations: Some(WindowDecorations::Server),
        ..Default::default()
    }
}

/// Opens Files, or brings it forward when it is open.
pub fn open_files(cx: &mut App) {
    let app = crate::state::app(cx);
    if let Some(h) = app.read(cx).files_win
        && h.update(cx, |_, window, _| window.activate_window()).is_ok()
    {
        return;
    }
    let opts = options("Files, Switchboard", 1180., 740., (860., 500.), cx);
    match cx.open_window(opts, |window, cx| cx.new(|cx| FilesView::new(window, cx))) {
        Ok(h) => app.update(cx, |s, _| s.files_win = Some(h.into())),
        Err(e) => log::error!("could not open Files: {e:#}"),
    }
}

/// Opens Add a machine, or brings it forward when it is open.
pub fn open_pair(cx: &mut App) {
    let app = crate::state::app(cx);
    if let Some(h) = app.read(cx).pair_win
        && h.update(cx, |_, window, _| window.activate_window()).is_ok()
    {
        return;
    }
    let opts = options("Add a machine, Switchboard", 600., 600., (480., 460.), cx);
    match cx.open_window(opts, |window, cx| cx.new(|cx| PairView::new(window, cx))) {
        Ok(h) => app.update(cx, |s, _| s.pair_win = Some(h.into())),
        Err(e) => log::error!("could not open Add a machine: {e:#}"),
    }
}

// ---------- pieces Files and the New sheet's folder picker share ----------

/// A place's icon, by its kind.
pub fn place_icon(kind: &str) -> &'static str {
    match kind {
        "claude" => "spark",
        "home" => "home",
        "desktop" => "desktop",
        "documents" => "docs",
        "downloads" => "download",
        "code" => "code",
        "icloud" => "cloud",
        "place" => "drive",
        _ => "folder",
    }
}

/// A file's icon, by what it looks like.
pub fn file_icon(name: &str) -> &'static str {
    match places::file_kind(name) {
        FileKind::Text => "docs",
        FileKind::Code => "fcode",
        FileKind::Data => "fdata",
        FileKind::Image => "image",
        FileKind::Media => "media",
        FileKind::Archive => "archive",
        FileKind::Other => "file",
    }
}

/// The small badge beside a place that cannot be opened as it is.
pub fn place_badge(t: &Theme, p: &ApiPlace) -> Option<gpui::Div> {
    let (text, color) = if p.privacy() {
        ("No permission", t.cold)
    } else if p.missing() {
        ("Missing", t.faint)
    } else {
        return None;
    };
    Some(
        div()
            .flex_none()
            .h(px(18.))
            .px(px(6.))
            .flex()
            .items_center()
            .gap(px(4.))
            .rounded(px(ui::R_CHIP))
            .bg(color.alpha(if t.dark { 0.16 } else { 0.12 }))
            .text_size(px(10.5))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(color)
            .when(p.privacy(), |d| d.child(ui::icon("lock", 9., color)))
            .child(text),
    )
}

/// One sidebar row: a place's icon, its name and its badge.
pub fn place_row(t: &Theme, id: impl Into<gpui::ElementId>, p: &ApiPlace, on: bool) -> gpui::Stateful<gpui::Div> {
    let hover = t.hover;
    let ink = if on { t.ink } else { t.ink2 };
    div()
        .id(id.into())
        .h(px(30.))
        .px(px(8.))
        .flex()
        .items_center()
        .gap(px(9.))
        .rounded(px(ui::R_CTRL))
        .cursor_pointer()
        .when(on, |d| d.bg(t.sel))
        .when(!on, |d| d.hover(move |s| s.bg(hover)))
        .child(ui::icon(place_icon(&p.kind), 15., if on { t.accent } else { t.dim }))
        .child(
            ui::line(p.name.clone())
                .flex_1()
                .text_size(px(13.))
                .text_color(if p.missing() { t.faint } else { ink })
                .when(on, |d| d.font_weight(FontWeight::SEMIBOLD)),
        )
        .children(place_badge(t, p))
}

/// A calm card in the middle of an empty list: an icon, what is going on, and what helps.
pub fn state_card(t: &Theme, ic: &str, tint: Hsla, title: String, body: String, actions: Vec<AnyElement>) -> gpui::Div {
    div().size_full().flex().items_center().justify_center().p(px(24.)).child(
        div()
            .w(px(380.))
            .max_w(relative(1.))
            .flex()
            .flex_col()
            .items_center()
            .gap(px(10.))
            .child(
                div()
                    .size(px(52.))
                    .rounded(px(14.))
                    .bg(tint.alpha(if t.dark { 0.16 } else { 0.1 }))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(ui::icon(ic, 24., tint)),
            )
            .child(
                ui::wrap(title)
                    .mt(px(4.))
                    .text_size(px(15.))
                    .font_weight(FontWeight::BOLD)
                    .text_color(t.ink)
                    .text_center()
                    .line_height(relative(1.35)),
            )
            .child(ui::wrap(body).text_size(px(12.5)).text_color(t.dim).text_center().line_height(relative(1.5)))
            .when(!actions.is_empty(), |d| {
                d.child(div().mt(px(6.)).flex().flex_wrap().justify_center().gap(px(8.)).children(actions))
            }),
    )
}

/// The words for a folder that shows no list, and the label of the button that helps (the caller wires it).
pub struct ClosedWords {
    pub icon: &'static str,
    pub warm: bool,
    pub title: String,
    pub body: String,
    pub button: Option<String>,
}

pub fn closed_words(c: &Closed, name: &str, machine: &str, opened: bool) -> ClosedWords {
    match c {
        Closed::Privacy { .. } if opened => ClosedWords {
            icon: "lock",
            warm: true,
            title: format!("Turn on the switch on {machine}"),
            body: format!(
                "System Settings is open on {machine}, at Files and Folders. Turn {name} on for the app that runs Claude Code, then check again here."
            ),
            button: Some("Check again".into()),
        },
        Closed::Privacy { .. } => ClosedWords {
            icon: "lock",
            warm: true,
            title: format!("macOS permission not granted for {name} on {machine}"),
            body: "It might not really be empty. Turn on the permission in System Settings, under Privacy and Security, Files and Folders.".into(),
            button: Some(format!("Open the switch on {machine}")),
        },
        Closed::Missing => ClosedWords {
            icon: "folder",
            warm: false,
            title: format!("{name} isn't there anymore"),
            body: format!("It was moved or renamed on {machine}, or it's on a drive that isn't plugged in."),
            button: Some("Open the Claude folder".into()),
        },
        Closed::RoamOff => ClosedWords {
            icon: "home",
            warm: false,
            title: format!("Anywhere in home is off for {machine}"),
            body: format!(
                "Chats and Files on {machine} stay in its Claude folder. Turn on Anywhere in home in Settings to go further. Claude still asks before it works in a new folder."
            ),
            button: Some("Open Settings".into()),
        },
        Closed::Error(e) => ClosedWords {
            icon: "warn",
            warm: true,
            title: "Couldn't read this folder".into(),
            body: e.clone(),
            button: Some("Try again".into()),
        },
    }
}

/// "3 folders, 12 files", "1 file", "Empty".
pub fn counts(dirs: usize, files: usize) -> String {
    let n = |k: usize, one: &str, many: &str| format!("{k} {}", if k == 1 { one } else { many });
    match (dirs, files) {
        (0, 0) => "Empty".into(),
        (d, 0) => n(d, "folder", "folders"),
        (0, f) => n(f, "file", "files"),
        (d, f) => format!("{}, {}", n(d, "folder", "folders"), n(f, "file", "files")),
    }
}

// ---------- Files ----------

pub struct FilesView {
    app: Entity<AppState>,
    nav: Entity<Nav>,
    focus: FocusHandle,
    file: Option<String>,
    preview: Option<Result<ApiFile, String>>,
    /// Bumped per file opened, so a preview that comes back late does not replace the one picked after it.
    file_seq: u64,
    /// The row the keyboard is on (folders first, then files).
    cur: usize,
    /// The folder the keyboard row belongs to (a new folder starts at the top).
    cur_of: String,
    /// The folder's rows: only the ones on screen are drawn (a uniform list of 32 px rows).
    list: gpui::UniformListScrollHandle,
    /// The machine dropdown, when there are more than four machines.
    drop: ui::Drop,
    pane: ScrollHandle,
    side: ScrollHandle,
    _subs: Vec<Subscription>,
}

/// One row of the list, for the keyboard.
enum Row {
    Dir(String),
    File(String, bool),
}

impl FilesView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> FilesView {
        let app = crate::state::app(cx);
        let machine =
            app.read(cx).snap.machines.iter().find(|m| files_ok(m)).map(|m| m.name.clone()).unwrap_or_default();
        // Shots: SB_FILES=<machine>:<folder>[:<file>] opens that folder (and file) first.
        #[cfg(feature = "shots")]
        let shot: Option<(String, String, Option<String>)> = std::env::var("SB_FILES").ok().map(|v| {
            let mut it = v.splitn(3, ':');
            let m = it.next().unwrap_or("").to_string();
            let f = it.next().unwrap_or("").to_string();
            (m, f, it.next().map(str::to_string).filter(|x| !x.is_empty()))
        });
        #[cfg(not(feature = "shots"))]
        let shot: Option<(String, String, Option<String>)> = None;
        let (machine, folder) = match &shot {
            Some((m, f, _)) if !m.is_empty() => (m.clone(), f.clone()),
            _ => (machine, String::new()),
        };
        let nav = cx.new(|cx| Nav::new(app.clone(), machine, folder, cx));
        let subs = vec![
            cx.observe(&app, |_, _, cx| cx.notify()),
            cx.observe(&nav, |this, nav, cx| {
                let f = nav.read(cx).folder.clone();
                if f != this.cur_of {
                    this.cur_of = f;
                    this.cur = 0;
                    this.list.scroll_to_item(0, gpui::ScrollStrategy::Top);
                }
                cx.notify()
            }),
            cx.observe_window_appearance(window, |_, _, cx| cx.notify()),
        ];
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        let mut v = FilesView {
            app,
            nav,
            focus,
            file: None,
            preview: None,
            file_seq: 0,
            cur: 0,
            cur_of: String::new(),
            list: gpui::UniformListScrollHandle::new(),
            drop: ui::Drop::default(),
            pane: ScrollHandle::new(),
            side: ScrollHandle::new(),
            _subs: subs,
        };
        if let Some((_, _, Some(name))) = shot {
            v.open_file(name, cx);
        }
        v
    }

    fn machine(&self, cx: &App) -> String {
        self.nav.read(cx).machine.clone()
    }

    fn rows(&self, cx: &App) -> Vec<Row> {
        let Some(l) = self.nav.read(cx).shown_listing() else { return Vec::new() };
        l.dirs
            .iter()
            .map(|d| Row::Dir(d.folder.clone()))
            .chain(l.files.iter().map(|f| Row::File(f.name.clone(), f.open)))
            .collect()
    }

    fn step(&mut self, d: i32, cx: &mut Context<Self>) {
        let rows = self.rows(cx);
        if rows.is_empty() {
            return;
        }
        self.cur = (self.cur as i32 + d).clamp(0, rows.len() as i32 - 1) as usize;
        self.list.scroll_to_item(self.cur, gpui::ScrollStrategy::Nearest);
        // A file under the keyboard shows in the preview, like Quick Look.
        if let Some(Row::File(name, true)) = rows.into_iter().nth(self.cur) {
            self.open_file(name, cx);
        }
        cx.notify();
    }

    fn open_cur(&mut self, cx: &mut Context<Self>) {
        match self.rows(cx).into_iter().nth(self.cur) {
            Some(Row::Dir(f)) => self.go(f, cx),
            Some(Row::File(name, true)) => self.open_file(name, cx),
            _ => {}
        }
    }

    fn parent(&mut self, cx: &mut Context<Self>) {
        self.file = None;
        self.preview = None;
        self.nav.update(cx, |n, cx| {
            n.up(cx);
        });
    }

    /// The full path of a file in this folder, on its machine (for pasting into a terminal there).
    fn path_of(&self, name: &str, cx: &App) -> String {
        let nav = self.nav.read(cx);
        match nav.shown_listing() {
            Some(l) if places::is_outside(&nav.folder) && !l.shown.is_empty() => {
                let sep = if l.shown.contains('\\') && !l.shown.contains('/') { "\\" } else { "/" };
                format!("{}{sep}{name}", l.shown.trim_end_matches(['/', '\\']))
            }
            Some(l) => remote_path(&l.root, &nav.folder, name),
            None => name.to_string(),
        }
    }

    /// Chats on this machine whose folder is this one (or inside it), newest first.
    fn chats_here(&self, cx: &App) -> Vec<(String, String, bool)> {
        let nav = self.nav.read(cx);
        let app = self.app.read(cx);
        let Some(m) = app.snap.machine(&nav.machine) else { return Vec::new() };
        let here = nav.folder.trim_end_matches('/');
        m.sessions
            .iter()
            .filter(|s| {
                let f = s.folder_addr.trim_end_matches('/');
                !s.stale && if here.is_empty() { f.is_empty() } else { places::contains(here, f) }
            })
            .take(6)
            .map(|s| (s.key.clone(), s.topic.clone(), s.alive))
            .collect()
    }

    fn go(&mut self, folder: String, cx: &mut Context<Self>) {
        self.file = None;
        self.preview = None;
        self.nav.update(cx, |n, cx| n.go(folder, cx));
    }

    fn set_machine(&mut self, machine: String, cx: &mut Context<Self>) {
        self.file = None;
        self.preview = None;
        self.nav.update(cx, |n, cx| n.set_machine(machine, cx));
    }

    fn open_file(&mut self, name: String, cx: &mut Context<Self>) {
        self.file = Some(name.clone());
        self.preview = None;
        self.file_seq += 1;
        cx.notify();
        let nav = self.nav.read(cx);
        let (core, m, f) = (self.app.read(cx).core.clone(), nav.machine.clone(), nav.folder.clone());
        let (seq, file_seq) = (nav.seq, self.file_seq);
        cx.spawn(async move |this, cx| {
            let res = cx.background_executor().spawn(async move { core.files(&m, &f, Some(&name)) }).await;
            let _ = this.update(cx, |s, cx| {
                if !still_asked((seq, file_seq), (s.nav.read(cx).seq, s.file_seq)) {
                    return;
                }
                s.preview = Some(match res {
                    Ok(r) => r.file.ok_or_else(|| "The machine didn't send a file.".to_string()),
                    Err(e) => Err(e.to_string()),
                });
                s.pane.scroll_to_item(0);
                cx.notify();
            });
        })
        .detach();
    }

    fn new_here(&mut self, cx: &mut Context<Self>) {
        let nav = self.nav.read(cx);
        let (machine, folder) = (nav.machine.clone(), nav.folder.clone());
        self.app.update(cx, |_, cx| cx.emit(AppEvent::NewIn { machine, folder }));
    }

    // ---------- the sidebar ----------

    fn sidebar(&self, t: &Theme, cx: &Context<Self>) -> AnyElement {
        let nav = self.nav.read(cx);
        let label = nav.machine_label(cx);
        let folder = nav.folder.clone();
        let mut col = div()
            .id("fx-side")
            .size_full()
            .flex()
            .flex_col()
            .gap(px(1.))
            .overflow_y_scroll()
            .track_scroll(&self.side)
            .px(px(10.))
            .pt(px(14.))
            .pb(px(12.));
        let group = |text: String| div().px(px(8.)).pt(px(4.)).pb(px(6.)).child(ui::lbl(&text, t.faint));
        let Some(p) = nav.places().cloned() else {
            // Not in yet (or the server could not say): the Claude folder is always there.
            let claude = ApiPlace {
                kind: "claude".into(),
                name: "Claude folder".into(),
                state: "ok".into(),
                ..Default::default()
            };
            col = col.child(group(format!("On {label}"))).child(
                place_row(t, "fx-p-claude", &claude, places::contains("", &folder))
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.go(String::new(), cx))),
            );
            col = col.child(match nav.places_error() {
                Some(e) => ui::wrap(e.to_string())
                    .px(px(8.))
                    .pt(px(8.))
                    .text_size(px(12.))
                    .text_color(t.faint)
                    .into_any_element(),
                None => ui::loading(t, "fx-p-load", "Finding places")
                    .px(px(8.))
                    .pt(px(8.))
                    .text_size(px(12.))
                    .into_any_element(),
            });
            return col.into_any_element();
        };
        let on_place = p.place_of(&folder).map(|x| x.folder.clone());
        let row = |pl: &ApiPlace, i: usize| {
            let to = pl.folder.clone();
            place_row(t, SharedString::from(format!("fx-p-{i}")), pl, on_place.as_deref() == Some(pl.folder.as_str()))
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.go(to.clone(), cx)))
                .into_any_element()
        };
        col = col.child(group(format!("On {label}")));
        for (i, pl) in p.usual().enumerate() {
            col = col.child(row(pl, i));
        }
        let added: Vec<&ApiPlace> = p.added().collect();
        if !added.is_empty() {
            col = col.child(div().h(px(12.))).child(group("Added on this machine".into()));
            for (i, pl) in added.into_iter().enumerate() {
                col = col.child(row(pl, 100 + i));
            }
        }
        // What the sidebar cannot show, said once at its foot.
        let foot = if p.old {
            Some((
                "update",
                format!("Update Switchboard on {label} to browse its home folder, Desktop, Documents and more."),
                None,
            ))
        } else if !p.roam {
            Some((
                "home",
                "Anywhere in home is off, so only the Claude folder and added places show.".to_string(),
                Some("Settings"),
            ))
        } else {
            None
        };
        if let Some((ic, text, link)) = foot {
            let m = nav.machine.clone();
            col = col.child(div().flex_1().min_h(px(16.))).child(
                div()
                    .mx(px(2.))
                    .p(px(10.))
                    .rounded(px(ui::R_CARD))
                    .bg(t.bg2)
                    .border_1()
                    .border_color(t.line)
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .child(
                        div()
                            .flex()
                            .items_start()
                            .gap(px(8.))
                            .child(div().pt(px(1.)).child(ui::icon(ic, 13., t.dim)))
                            .child(
                                ui::wrap(text)
                                    .flex_1()
                                    .text_size(px(12.))
                                    .line_height(relative(1.45))
                                    .text_color(t.dim),
                            ),
                    )
                    .children(link.map(|l| {
                        div().pl(px(21.)).child(ui::link(t, "fx-roam-settings", l).text_size(px(12.)).on_click(
                            cx.listener(move |_, _: &ClickEvent, _, cx| crate::settings::open_machine(m.clone(), cx)),
                        ))
                    })),
            );
        }
        col.into_any_element()
    }

    // ---------- the path bar ----------

    fn path_bar(&self, t: &Theme, cx: &Context<Self>) -> AnyElement {
        let nav = self.nav.read(cx);
        let place = nav.places().and_then(|p| p.place_of(&nav.folder)).cloned();
        let crumbs = places::crumbs(&nav.folder, place.as_ref(), &nav.claude_name());
        let n = crumbs.len();
        let hover = t.hover;
        let mut row = div().flex_1().min_w_0().flex().items_center().gap(px(1.)).overflow_hidden();
        for (i, c) in crumbs.into_iter().enumerate() {
            let last = i + 1 == n && self.file.is_none();
            if i > 0 {
                row = row.child(div().flex_none().px(px(1.)).child(ui::icon("chev", 10., t.faint)));
            }
            let ic = (i == 0).then(|| {
                place_icon(place.as_ref().map(|p| p.kind.as_str()).unwrap_or(if c.folder.starts_with('~') {
                    "home"
                } else if c.folder.starts_with('@') {
                    "place"
                } else {
                    "claude"
                }))
            });
            let to = c.folder.clone();
            row = row.child(
                div()
                    .id(SharedString::from(format!("fx-c-{i}")))
                    .flex_shrink(1.)
                    .min_w(px(24.))
                    .h(px(26.))
                    .px(px(7.))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .rounded(px(ui::R_ITEM))
                    .text_size(px(13.))
                    .text_color(if last { t.ink } else { t.dim })
                    .when(last, |d| d.font_weight(FontWeight::SEMIBOLD))
                    .when(!last, |d| d.cursor_pointer().hover(move |s| s.bg(hover)))
                    .children(ic.map(|i| ui::icon(i, 13., if last { t.accent } else { t.dim })))
                    .child(ui::line(c.label))
                    .when(!last, |d| {
                        d.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.go(to.clone(), cx)))
                    }),
            );
        }
        if let Some(f) = &self.file {
            row = row.child(div().flex_none().px(px(1.)).child(ui::icon("chev", 10., t.faint))).child(
                div()
                    .flex_shrink(1.)
                    .min_w(px(24.))
                    .h(px(26.))
                    .px(px(7.))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .text_size(px(13.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(ui::icon(file_icon(f), 13., t.dim))
                    .child(ui::line(f.clone())),
            );
        }
        let has_up = places::parent(&nav.folder).is_some();
        let closed = nav.closed().is_some() || nav.machine.is_empty();
        div()
            .flex_none()
            .h(px(46.))
            .px(px(10.))
            .flex()
            .items_center()
            .gap(px(8.))
            .border_b_1()
            .border_color(t.rule)
            .child(
                ui::icon_btn(t, "fx-up", "up", 28.)
                    .when(!has_up, |d| d.opacity(0.35).cursor_default())
                    .when(has_up, |d| d.on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.parent(cx)))),
            )
            .child(row)
            .child(
                ui::primary_btn(t, "fx-new", 30.)
                    .px(px(12.))
                    .text_size(px(12.5))
                    .when(closed, |d| d.opacity(0.45).cursor_default())
                    .child(ui::icon("plus", 12., t.on_accent))
                    .child("New chat here")
                    .when(!closed, |d| d.on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.new_here(cx)))),
            )
            .into_any_element()
    }

    // ---------- the list ----------

    fn entry(
        t: &Theme,
        id: String,
        ic: &str,
        tint: Hsla,
        name: String,
        on: bool,
        enabled: bool,
    ) -> gpui::Stateful<gpui::Div> {
        let (hover, sel) = (t.hover, t.sel);
        div()
            .id(SharedString::from(id))
            .h(px(ROW_H - 2.))
            .px(px(10.))
            .flex()
            .items_center()
            .gap(px(10.))
            .rounded(px(ui::R_CTRL))
            .when(on, |d| d.bg(sel))
            .when(enabled && !on, |d| d.cursor_pointer().hover(move |s| s.bg(hover)))
            .child(ui::icon(ic, 15., if enabled { tint } else { t.faint }))
            .child(ui::line(name).flex_1().text_size(px(13.)).text_color(if enabled { t.ink } else { t.dim }))
    }

    fn column_head(t: &Theme) -> gpui::Div {
        let h = |text: &'static str| {
            div().text_size(px(11.)).font_weight(FontWeight::SEMIBOLD).text_color(t.faint).child(text)
        };
        div()
            .flex_none()
            .h(px(28.))
            .px(px(20.))
            .flex()
            .items_center()
            .gap(px(10.))
            .border_b_1()
            .border_color(t.rule)
            .child(div().w(px(15.)))
            .child(h("Name").flex_1())
            .child(h("Size").w(px(SIZE_W)).flex().justify_end())
            .child(h("Changed").w(px(DATE_W)).flex().justify_end())
    }

    fn list(&self, t: &Theme, cx: &Context<Self>) -> AnyElement {
        let nav = self.nav.read(cx);
        if nav.machine.is_empty() {
            return state_card(
                t,
                "mac",
                t.dim,
                "No machine to browse yet".into(),
                "Files show up here once a machine has the latest Switchboard update.".into(),
                vec![],
            )
            .into_any_element();
        }
        if let Some(c) = nav.closed() {
            return self.closed_card(t, &c, cx);
        }
        let Some(l) = nav.shown_listing() else {
            return ui::loading(t, "fx-load", "Reading the folder").p(px(22.)).into_any_element();
        };
        let (nd, n) = (l.dirs.len(), l.dirs.len() + l.files.len());
        if n == 0 {
            return state_card(
                t,
                "folder",
                t.dim,
                "Nothing in this folder".into(),
                "There's nothing here Switchboard shows. Dot files and private folders stay hidden.".into(),
                vec![],
            )
            .into_any_element();
        }
        let rows = gpui::uniform_list(
            "fx-list",
            n,
            cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
                let t = Theme::get(cx);
                let now = crate::state::AppState::now();
                let nav = this.nav.read(cx);
                let Some(l) = nav.shown_listing() else { return Vec::new() };
                let meta = |text: String, w: f32| {
                    div()
                        .flex_none()
                        .w(px(w))
                        .flex()
                        .justify_end()
                        .font_family(NUM)
                        .text_size(px(11.5))
                        .text_color(t.faint)
                        .child(text)
                };
                let mut out = Vec::with_capacity(range.len());
                for i in range {
                    let e = if let Some(d) = l.dirs.get(i) {
                        let to = d.folder.clone();
                        let id = format!("fx-d-{}", d.folder);
                        Self::entry(&t, id, "folder", t.accent, d.name.clone(), this.cur == i, true)
                            .child(meta(String::new(), SIZE_W))
                            .child(
                                div()
                                    .flex_none()
                                    .w(px(DATE_W))
                                    .flex()
                                    .justify_end()
                                    .child(ui::icon("chev", 11., t.faint)),
                            )
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.go(to.clone(), cx)))
                    } else if let Some(f) = i.checked_sub(nd).and_then(|j| l.files.get(j)) {
                        let on = this.file.as_deref() == Some(f.name.as_str()) || this.cur == i;
                        let id = format!("fx-f-{}", f.name);
                        let ic = if f.open { file_icon(&f.name) } else { "lock" };
                        let mut e = Self::entry(&t, id, ic, t.dim, f.name.clone(), on, f.open)
                            .child(meta(size_text(f.size), SIZE_W))
                            .child(meta(fmt::when(f.mtime, now), DATE_W));
                        if f.open {
                            let name = f.name.clone();
                            let k = i;
                            e = e.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                this.cur = k;
                                this.open_file(name.clone(), cx)
                            }));
                        }
                        e
                    } else {
                        continue;
                    };
                    out.push(div().h(px(ROW_H)).child(e));
                }
                out
            }),
        )
        .track_scroll(&self.list)
        .size_full()
        .px(px(10.))
        .py(px(6.));
        let mut col = div()
            .size_full()
            .flex()
            .flex_col()
            .child(Self::column_head(t))
            .child(div().relative().flex_1().min_h_0().child(rows).child(ui::scrollbar(&self.list)));
        let foot = format!(
            "{}{}",
            counts(l.dirs.len(), l.files.len()),
            if l.more { ". Showing the first 400 of each." } else { "" }
        );
        col = col.child(
            div()
                .flex_none()
                .h(px(28.))
                .px(px(20.))
                .flex()
                .items_center()
                .border_t_1()
                .border_color(t.rule)
                .text_size(px(11.5))
                .text_color(t.faint)
                .child(foot),
        );
        col.into_any_element()
    }

    fn closed_card(&self, t: &Theme, c: &Closed, cx: &Context<Self>) -> AnyElement {
        let nav = self.nav.read(cx);
        let opened = nav.privacy_opened.as_deref() == Some(nav.folder.as_str());
        let w = closed_words(c, &nav.name(), &nav.machine_label(cx), opened);
        let tint = if w.warm { t.cold } else { t.dim };
        let machine = nav.machine.clone();
        let mut actions = Vec::new();
        if let Some(b) = w.button {
            let btn = ui::primary_btn(t, "fx-closed-go", 32.).px(px(14.)).text_size(px(12.5)).child(b);
            let btn = match c {
                Closed::Privacy { .. } if !opened => btn.on_click(
                    cx.listener(|this, _: &ClickEvent, _, cx| this.nav.update(cx, |n, cx| n.open_privacy(cx))),
                ),
                Closed::Missing => btn.on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.go(String::new(), cx))),
                Closed::RoamOff => btn.on_click(
                    cx.listener(move |_, _: &ClickEvent, _, cx| crate::settings::open_machine(machine.clone(), cx)),
                ),
                _ => btn.on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.nav.update(cx, |n, cx| n.reload(cx)))),
            };
            actions.push(btn.into_any_element());
        }
        if !matches!(c, Closed::Missing) {
            actions.push(
                ui::line_btn(t, "fx-closed-home", 32.)
                    .px(px(14.))
                    .text_size(px(12.5))
                    .child("Open the Claude folder")
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.go(String::new(), cx)))
                    .into_any_element(),
            );
        }
        state_card(t, w.icon, tint, w.title, w.body, actions).into_any_element()
    }

    // ---------- the preview pane ----------

    /// The right pane with the shared scrollbar over it.
    fn pane(&self, t: &Theme, now: f64, cx: &Context<Self>) -> AnyElement {
        div()
            .size_full()
            .relative()
            .child(self.pane_body(t, now, cx))
            .child(ui::scrollbar(&self.pane))
            .into_any_element()
    }

    fn chats_block(&self, t: &Theme, cx: &Context<Self>) -> Option<gpui::Div> {
        let chats = self.chats_here(cx);
        if chats.is_empty() {
            return None;
        }
        let path = self.file.as_ref().map(|n| self.path_of(n, cx));
        let hover = t.hover;
        let mut c =
            div().flex().flex_col().gap(px(2.)).child(div().pb(px(4.)).child(ui::lbl("Chats in this folder", t.faint)));
        for (key, topic, alive) in chats {
            let (k1, k2, p) = (key.clone(), key.clone(), path.clone());
            c = c.child(
                div()
                    .id(SharedString::from(format!("fx-chat-{key}")))
                    .min_h(px(34.))
                    .px(px(8.))
                    .py(px(4.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .rounded(px(ui::R_CTRL))
                    .hover(move |s| s.bg(hover))
                    .child(ui::icon("term", 14., t.dim))
                    .child(ui::line(topic).flex_1().min_w_0().text_size(px(13.)).text_color(t.ink))
                    .when_some(p, |d, p| {
                        d.child(
                            ui::ghost(t, SharedString::from(format!("fx-paste-{k2}")), Some("Paste the path"), None)
                                .h(px(26.))
                                .px(px(8.))
                                .text_size(px(12.))
                                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                    let (key, text) = (k2.clone(), format!("{p} "));
                                    this.app.update(cx, |_, cx| cx.emit(AppEvent::PasteInto { key, text }));
                                })),
                        )
                    })
                    .child(
                        ui::line_btn(t, SharedString::from(format!("fx-open-{k1}")), 26.)
                            .px(px(9.))
                            .text_size(px(12.))
                            .child(if alive { "Open terminal" } else { "Resume" })
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                let key = k1.clone();
                                this.app.update(cx, |_, cx| cx.emit(AppEvent::OpenTerm { key, resume: !alive }));
                            })),
                    ),
            );
        }
        Some(c)
    }

    /// The folder on screen, when no file is open: what it is, where, and whether Claude may work there yet.
    fn folder_card(&self, t: &Theme, cx: &Context<Self>) -> gpui::Div {
        let nav = self.nav.read(cx);
        let place = nav.places().and_then(|p| p.place_of(&nav.folder)).cloned();
        let at_place =
            place.as_ref().is_some_and(|p| p.folder.trim_end_matches('/') == nav.folder.trim_end_matches('/'));
        let ic = if at_place { place_icon(place.as_ref().map(|p| p.kind.as_str()).unwrap_or("")) } else { "folder" };
        let listing = nav.shown_listing();
        let shown = listing.map(|l| l.shown.clone()).filter(|s| !s.is_empty()).unwrap_or_else(|| nav.shown());
        let shown = places::shown(&shown, &nav.claude_name());
        let meta = listing.filter(|l| !l.privacy_blocked()).map(|l| counts(l.dirs.len(), l.files.len()));
        let trust = listing.and_then(|l| l.trust.clone());
        let closed = nav.closed().is_some();
        let (tic, tcol, ttext) = match trust.as_deref() {
            _ if closed => ("info", t.faint, ""),
            Some("ask") => ("info", t.dim, "Claude hasn't worked here yet. A new chat here asks you first."),
            Some("no") => ("warn", t.cold, "You said no to Claude here before. A new chat here asks again."),
            Some(_) | None if listing.is_some() => ("check", t.warm, "Claude can work here."),
            _ => ("info", t.faint, ""),
        };
        div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .child(
                        div()
                            .flex_none()
                            .size(px(44.))
                            .rounded(px(12.))
                            .bg(t.accent.alpha(if t.dark { 0.16 } else { 0.1 }))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(ui::icon(ic, 22., t.accent)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(3.))
                            .child(
                                ui::wrap(nav.name()).text_size(px(16.)).font_weight(FontWeight::BOLD).text_color(t.ink),
                            )
                            .children(meta.map(|m| div().text_size(px(12.)).text_color(t.dim).child(m))),
                    ),
            )
            .child(
                div()
                    .px(px(10.))
                    .py(px(7.))
                    .rounded(px(ui::R_CTRL))
                    .bg(t.bg2)
                    .border_1()
                    .border_color(t.line)
                    .font_family(NUM)
                    .text_size(px(11.5))
                    .text_color(t.ink2)
                    .child(ui::wrap(shown)),
            )
            .when(!ttext.is_empty(), |d| {
                d.child(
                    div()
                        .flex()
                        .items_start()
                        .gap(px(8.))
                        .child(div().pt(px(1.)).child(ui::icon(tic, 13., tcol)))
                        .child(
                            ui::wrap(ttext).flex_1().text_size(px(12.5)).line_height(relative(1.45)).text_color(t.dim),
                        ),
                )
            })
    }

    fn pane_body(&self, t: &Theme, now: f64, cx: &Context<Self>) -> AnyElement {
        let col = div()
            .id("fx-pane")
            .size_full()
            .flex()
            .flex_col()
            .overflow_y_scroll()
            .track_scroll(&self.pane)
            .p(px(18.))
            .gap(px(16.));
        let Some(name) = &self.file else {
            let nav = self.nav.read(cx);
            if let Some(c) = nav.closed() {
                // The machine's own words for a closed folder (macOS privacy), else nothing more to say here.
                let hint = match c {
                    Closed::Privacy { hint } => hint,
                    _ => None,
                };
                return col
                    .child(self.folder_card(t, cx))
                    .children(
                        hint.map(|h| ui::wrap(h).text_size(px(12.)).line_height(relative(1.45)).text_color(t.faint)),
                    )
                    .into_any_element();
            }
            return col
                .child(self.folder_card(t, cx))
                .children(self.chats_block(t, cx))
                .child(
                    ui::wrap("Pick a file to read it here, or move with the arrow keys. Files that look like keys or secrets stay closed.")
                        .text_size(px(12.))
                        .line_height(relative(1.45))
                        .text_color(t.faint),
                )
                .into_any_element();
        };
        let head = div()
            .flex()
            .items_center()
            .gap(px(12.))
            .child(
                div()
                    .flex_none()
                    .size(px(44.))
                    .rounded(px(12.))
                    .bg(t.panel3)
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(ui::icon(file_icon(name), 22., t.dim)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(3.))
                    .child(ui::wrap(name.clone()).text_size(px(15.)).font_weight(FontWeight::BOLD).text_color(t.ink))
                    .children(match &self.preview {
                        Some(Ok(f)) => Some(div().text_size(px(12.)).text_color(t.dim).child(format!(
                            "{}, changed {}",
                            size_text(f.size),
                            fmt::ago(f.mtime, now)
                        ))),
                        _ => None,
                    }),
            );
        let mut col = col.child(head);
        let say = |text: &str| div().text_size(px(13.)).text_color(t.dim).child(text.to_string());
        col = match &self.preview {
            None => col.child(ui::loading(t, "fx-col-load", "Reading").text_size(px(12.5))),
            Some(Err(e)) => col.child(ui::wrap(e.clone()).text_size(px(12.5)).text_color(t.bad)),
            Some(Ok(f)) if f.secret => {
                col.child(say("This looks like a key or a secret, so Switchboard won't open it."))
            }
            Some(Ok(f)) => match f.text.as_ref().filter(|_| !f.binary) {
                None => col.child(say("It's not a text file, so there's nothing to show.")),
                Some(text) => {
                    let mut body = div()
                        .p(px(12.))
                        .rounded(px(ui::R_CTRL))
                        .border_1()
                        .border_color(t.line)
                        .bg(t.bg2)
                        .font_family(NUM)
                        .text_size(px(12.))
                        .text_color(t.ink)
                        .flex()
                        .flex_col();
                    let lines: Vec<&str> = text.lines().collect();
                    for l in lines.iter().take(PREVIEW_LINES) {
                        body = body.child(div().min_h(px(16.)).child(l.replace('\t', "    ")));
                    }
                    col = col.child(body);
                    if f.cut || lines.len() > PREVIEW_LINES {
                        col = col.child(
                            div().text_size(px(12.)).text_color(t.faint).child("Showing the start of the file."),
                        );
                    }
                    col
                }
            },
        };
        col.children(self.chats_block(t, cx)).into_any_element()
    }
}

fn size_text(bytes: f64) -> String {
    if bytes <= 0. {
        "Empty".into()
    } else if bytes >= 1_000_000_000. {
        format!("{:.1} GB", bytes / 1_000_000_000.)
    } else if bytes >= 1_000_000. {
        format!("{:.1} MB", bytes / 1_000_000.)
    } else {
        format!("{} KB", (bytes / 1000.).round().max(1.) as u64)
    }
}

impl Render for FilesView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body = self.body(window, cx);
        crate::frame::framed(window, &Theme::get(cx), "Files", body)
    }
}

impl FilesView {
    fn body(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = Theme::get(cx);
        let now = AppState::now();
        let cur_machine = self.machine(cx);
        let machines: Vec<(String, String, bool)> =
            self.app.read(cx).snap.machines.iter().map(|m| (m.name.clone(), m.label.clone(), files_ok(m))).collect();
        // The machine: segmented buttons for up to four machines, a dropdown past that.
        let mseg = if machines.len() <= 4 {
            let mut seg = ui::seg_box(&t);
            for (name, label, ok) in machines {
                let on = cur_machine == name;
                let mut b = ui::seg_btn(&t, SharedString::from(format!("fx-m-{name}")), on, !ok).child(label);
                if ok {
                    b = b.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.set_machine(name.clone(), cx)));
                }
                seg = seg.child(b);
            }
            seg.into_any_element()
        } else {
            let open = self.drop.is_open("fx-machine");
            let cur = machines.iter().find(|m| m.0 == cur_machine).map(|m| m.1.clone()).unwrap_or_default();
            let trigger = ui::dropdown(&t, "fx-machine", Some("mac"), cur, open).on_click(cx.listener(
                |this, _: &ClickEvent, _, cx| {
                    this.drop.toggle("fx-machine");
                    cx.notify();
                },
            ));
            let menu = open.then(|| {
                let items = machines
                    .into_iter()
                    .map(|(name, label, ok)| ui::DropItem {
                        label: label.into(),
                        on: cur_machine == name,
                        off: !ok,
                        pick: Box::new(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.drop.close();
                            this.set_machine(name.clone(), cx);
                        })),
                    })
                    .collect();
                ui::drop_menu(
                    &t,
                    "fx-machine",
                    items,
                    cx.listener(|this, _: &gpui::MouseDownEvent, _, cx| {
                        this.drop.close_out();
                        cx.notify();
                    }),
                )
            });
            div().relative().flex_none().child(trigger).children(menu).into_any_element()
        };
        let header = div()
            .flex_none()
            .h(px(52.))
            .px(px(16.))
            .flex()
            .items_center()
            .gap(px(12.))
            .border_b_1()
            .border_color(t.rule)
            .bg(t.bg2)
            .child(div().flex_none().text_size(px(15.)).font_weight(FontWeight::BOLD).child("Files"))
            .child(mseg)
            .child(div().flex_1())
            .child(
                ui::ghost(&t, "fx-pair", None, None)
                    .h(px(28.))
                    .text_size(px(12.5))
                    .child(ui::icon("plus", 12., t.dim))
                    .child("Add a machine")
                    .on_click(cx.listener(|_, _: &ClickEvent, _, cx| open_pair(cx))),
            )
            .child(
                ui::icon_btn(&t, "fx-reload", "restore", 28.)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.nav.update(cx, |n, cx| n.reload(cx)))),
            );

        div()
            .id("files")
            .key_context("Files")
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &FxUp, _, cx| this.step(-1, cx)))
            .on_action(cx.listener(|this, _: &FxDown, _, cx| this.step(1, cx)))
            .on_action(cx.listener(|this, _: &FxOpen, _, cx| this.open_cur(cx)))
            .on_action(cx.listener(|this, _: &FxInto, _, cx| {
                if let Some(Row::Dir(f)) = this.rows(cx).into_iter().nth(this.cur) {
                    this.go(f, cx)
                }
            }))
            .on_action(cx.listener(|this, _: &FxParent, _, cx| this.parent(cx)))
            .on_action(cx.listener(|this, _: &FxLook, _, cx| this.open_cur(cx)))
            .size_full()
            .flex()
            .flex_col()
            .bg(t.bg)
            .font_family(crate::theme::UI)
            .text_color(t.ink)
            .child(header)
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .child(
                        div()
                            .flex_none()
                            .w(px(SIDEBAR_W))
                            .h_full()
                            .min_h_0()
                            .border_r_1()
                            .border_color(t.rule)
                            .bg(t.panel)
                            .child(self.sidebar(&t, cx)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .flex()
                            .flex_col()
                            .child(self.path_bar(&t, cx))
                            .child(div().flex_1().min_h_0().child(self.list(&t, cx))),
                    )
                    .child(
                        div()
                            .flex_none()
                            .w(px(PANE_W))
                            .h_full()
                            .min_h_0()
                            .border_l_1()
                            .border_color(t.rule)
                            .bg(t.panel)
                            .child(self.pane(&t, now, cx)),
                    ),
            )
    }
}

/// A file's full path on its machine: the Claude folder, the folder under it, the name (Windows roots keep "\\").
fn remote_path(root: &str, folder: &str, name: &str) -> String {
    let win = root.contains('\\') && !root.contains('/');
    let sep = if win { "\\" } else { "/" };
    let mut p = root.trim_end_matches(['/', '\\']).to_string();
    for part in folder.split('/').filter(|x| !x.is_empty()) {
        p.push_str(sep);
        p.push_str(part);
    }
    p.push_str(sep);
    p.push_str(name);
    p
}

const OSES: [(&str, &str); 3] = [("mac", "Mac"), ("linux", "Linux"), ("windows", "Windows")];

pub struct PairView {
    app: Entity<AppState>,
    focus: FocusHandle,
    os: &'static str,
    r: Option<ApiPair>,
    error: Option<String>,
    busy: bool,
    copied: bool,
    _subs: Vec<Subscription>,
}

impl PairView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> PairView {
        let app = crate::state::app(cx);
        let subs = vec![cx.observe_window_appearance(window, |_, _, cx| cx.notify())];
        let os = if cfg!(target_os = "macos") {
            "mac"
        } else if cfg!(windows) {
            "windows"
        } else {
            "linux"
        };
        PairView { app, focus: cx.focus_handle(), os, r: None, error: None, busy: false, copied: false, _subs: subs }
    }

    fn get_code(&mut self, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.error = None;
        cx.notify();
        let core = self.app.read(cx).core.clone();
        cx.spawn(async move |this, cx| {
            let res = cx.background_executor().spawn(async move { core.pair() }).await;
            let _ = this.update(cx, |s, cx| {
                s.busy = false;
                s.copied = false;
                match res {
                    Ok(r) => s.r = Some(r),
                    Err(e) => s.error = Some(e.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn line(&self) -> Option<String> {
        self.r.as_ref().and_then(|r| r.install.get(self.os).cloned())
    }
}

impl Render for PairView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body = self.body(window, cx);
        crate::frame::framed(window, &Theme::get(cx), "Add a machine", body)
    }
}

impl PairView {
    fn body(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = Theme::get(cx);
        let now = AppState::now();
        let mut col = div()
            .id("pair")
            .key_context("Pair")
            .track_focus(&self.focus)
            .size_full()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap(px(14.))
            .p(px(24.))
            .bg(t.bg)
            .font_family(crate::theme::UI)
            .text_color(t.ink)
            .child(div().text_size(px(18.)).font_weight(FontWeight::BOLD).child("Add a machine"))
            .child(div().text_size(px(13.)).text_color(t.dim).child(
                "Link another computer that runs Claude Code. It connects out to this hub, so you don't need SSH, just your network or tailnet.",
            ));
        let err = self.error.clone().map(|e| div().text_size(px(12.5)).text_color(t.bad).child(e));
        let Some(r) = self.r.clone() else {
            let steps = [
                "Get a code here. It works once, for 10 minutes.",
                "Paste one line in a terminal on the new machine.",
                "It shows up on the board within a minute.",
            ];
            for (i, s) in steps.iter().enumerate() {
                col = col.child(
                    div()
                        .flex()
                        .gap(px(10.))
                        .text_size(px(13.))
                        .child(
                            div()
                                .flex_none()
                                .w(px(18.))
                                .font_family(NUM)
                                .text_color(t.faint)
                                .child(format!("{}.", i + 1)),
                        )
                        .child(div().flex_1().text_color(t.ink2).child(*s)),
                );
            }
            return col
                .children(err)
                .child(
                    div().flex().child(
                        ui::primary_btn(&t, "pair-code", 36.)
                            .px(px(18.))
                            .text_size(px(13.))
                            .child(if self.busy { "Getting a code" } else { "Get a code" })
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.get_code(cx))),
                    ),
                )
                .into_any_element();
        };
        let exp = if r.expires > 1e12 { r.expires / 1000. } else { r.expires };
        let left = exp - now;
        let spaced: String = r.code.chars().map(|c| c.to_string()).collect::<Vec<_>>().join(" ");
        col = col.child(
            div()
                .p(px(16.))
                .rounded(px(ui::R_CARD))
                .border_1()
                .border_color(t.line)
                .flex()
                .flex_col()
                .gap(px(4.))
                .child(ui::lbl("CODE", t.faint))
                .child(div().font_family(NUM).text_size(px(30.)).child(spaced))
                .child(div().text_size(px(12.5)).text_color(t.dim).child(if left > 0. {
                    format!("Works once, for {} more min", ((left / 60.).ceil() as i64).max(1))
                } else {
                    "Expired. Get a new one.".to_string()
                })),
        );
        let mut seg = ui::seg_box(&t);
        for (k, label) in OSES {
            seg = seg.child(
                ui::seg_btn(&t, SharedString::from(format!("pair-os-{k}")), self.os == k, false).child(label).on_click(
                    cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.os = k;
                        this.copied = false;
                        cx.notify();
                    }),
                ),
            );
        }
        let line = self.line().unwrap_or_default();
        col.child(div().flex().child(seg))
            .child(div().text_size(px(12.5)).text_color(t.dim).child(if self.os == "windows" { "In PowerShell on the new machine:" } else { "In Terminal on the new machine:" }))
            .child(div().p(px(12.)).rounded(px(ui::R_CTRL)).border_1().border_color(t.line).bg(t.bg2).font_family(NUM).text_size(px(12.)).child(line.clone()))
            .children(err)
            .child(
                div()
                    .flex()
                    .gap(px(8.))
                    .child(
                        ui::primary_btn(&t, "pair-copy", 34.)
                            .px(px(16.))
                            .text_size(px(13.))
                            .child(ui::icon(if self.copied { "check" } else { "copy" }, 12., t.on_accent))
                            .child(if self.copied { "Copied" } else { "Copy the line" })
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                copy_text(cx, line.clone());
                                this.copied = true;
                                cx.notify();
                            })),
                    )
                    .child(
                        ui::line_btn(&t, "pair-new", 34.)
                            .px(px(14.))
                            .text_size(px(13.))
                            .child(if self.busy { "Getting a code" } else { "New code" })
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.get_code(cx))),
                    ),
            )
            .child(div().text_size(px(12.)).text_color(t.faint).child(
                "It downloads switchboard-node, links it with the code and starts it at login. If Claude Code there doesn't have Switchboard yet, it adds that too. If it does, it's left alone.",
            ))
            .into_any_element()
    }
}

/// A preview read for (folder, file) `asked` is still the one wanted: no other folder or file was opened since.
fn still_asked(asked: (u64, u64), now: (u64, u64)) -> bool {
    asked == now
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_preview_that_comes_back_late_is_dropped() {
        use super::still_asked;
        assert!(still_asked((1, 2), (1, 2)));
        assert!(!still_asked((1, 1), (1, 2)), "A picked, then B: A's preview is dropped");
        assert!(!still_asked((1, 2), (2, 2)), "another folder");
    }

    #[test]
    fn a_files_path_on_its_machine() {
        assert_eq!(
            super::remote_path("/Users/you/Claude/", "notes/today", "a.md"),
            "/Users/you/Claude/notes/today/a.md"
        );
        assert_eq!(super::remote_path("C:\\Users\\you\\code", "", "b.txt"), "C:\\Users\\you\\code\\b.txt");
    }
}
