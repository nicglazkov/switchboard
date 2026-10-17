//! v2: Files, a Mac-like browser of the Claude folder on each machine (every machine one click away): breadcrumbs,
//! the keyboard (arrows, Enter, Left or Cmd+Up for the folder above, Space for a quick look), a text preview (keys and
//! secrets are listed but never opened), New session here, and the chats in that folder with their terminal one
//! click away (a file's path can go straight into one). And Add a machine, a pairing code and the one line that links
//! a new machine. Each opens in a window of its own.

use gpui::{
    AnyElement, App, AppContext, ClickEvent, Context, Entity, FocusHandle, FontWeight, InteractiveElement, IntoElement, ParentElement, Render,
    ScrollHandle, SharedString, StatefulInteractiveElement, Styled, Subscription, TitlebarOptions, Window, WindowBackgroundAppearance,
    WindowBounds, WindowDecorations, WindowOptions, div, prelude::*, px, size,
};
use sb_core::api::{ApiFile, ApiFiles, ApiPair};
use sb_core::fmt;
use sb_core::model::Machine;

use crate::state::{AppEvent, AppState, copy_text};
use crate::theme::{NUM, Theme};
use crate::ui;

const PREVIEW_LINES: usize = 2000;

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

fn files_ok(m: &Machine) -> bool {
    m.online != Some(false) && m.features.iter().any(|f| f == "files")
}

pub(crate) fn options(title: &str, w: f32, h: f32, min: (f32, f32), cx: &App) -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(gpui::Bounds::centered(None, size(px(w), px(h)), cx))),
        titlebar: Some(TitlebarOptions { title: Some(title.to_string().into()), appears_transparent: false, traffic_light_position: None }),
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
    if let Some(h) = app.read(cx).files_win {
        if h.update(cx, |_, window, _| window.activate_window()).is_ok() {
            return;
        }
    }
    let opts = options("Files, Switchboard", 1080., 720., (680., 420.), cx);
    match cx.open_window(opts, |window, cx| cx.new(|cx| FilesView::new(window, cx))) {
        Ok(h) => app.update(cx, |s, _| s.files_win = Some(h.into())),
        Err(e) => log::error!("could not open Files: {e:#}"),
    }
}

/// Opens Add a machine, or brings it forward when it is open.
pub fn open_pair(cx: &mut App) {
    let app = crate::state::app(cx);
    if let Some(h) = app.read(cx).pair_win {
        if h.update(cx, |_, window, _| window.activate_window()).is_ok() {
            return;
        }
    }
    let opts = options("Add a machine, Switchboard", 600., 600., (480., 460.), cx);
    match cx.open_window(opts, |window, cx| cx.new(|cx| PairView::new(window, cx))) {
        Ok(h) => app.update(cx, |s, _| s.pair_win = Some(h.into())),
        Err(e) => log::error!("could not open Add a machine: {e:#}"),
    }
}

pub struct FilesView {
    app: Entity<AppState>,
    focus: FocusHandle,
    machine: String,
    folder: String,
    root: String,
    listing: Option<ApiFiles>,
    error: Option<String>,
    loading: bool,
    file: Option<String>,
    preview: Option<Result<ApiFile, String>>,
    seq: u64,
    /// The row the keyboard is on (folders first, then files).
    cur: usize,
    list: ScrollHandle,
    pane: ScrollHandle,
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
        let subs = vec![cx.observe(&app, |_, _, cx| cx.notify()), cx.observe_window_appearance(window, |_, _, cx| cx.notify())];
        let machine = app.read(cx).snap.machines.iter().find(|m| files_ok(m)).map(|m| m.name.clone()).unwrap_or_default();
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        let mut v = FilesView {
            app,
            focus,
            machine,
            folder: String::new(),
            root: String::new(),
            listing: None,
            error: None,
            loading: false,
            file: None,
            preview: None,
            seq: 0,
            cur: 0,
            list: ScrollHandle::new(),
            pane: ScrollHandle::new(),
            _subs: subs,
        };
        v.load(cx);
        v
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
            let _ = this.update(cx, |s, cx| {
                if s.seq != seq {
                    return;
                }
                s.loading = false;
                match res {
                    Ok(r) => {
                        s.root = r.root.clone();
                        s.listing = Some(r);
                        s.list.scroll_to_item(0);
                    }
                    Err(e) => s.error = Some(e.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn rows(&self) -> Vec<Row> {
        let Some(l) = &self.listing else { return Vec::new() };
        l.dirs.iter().map(|d| Row::Dir(d.folder.clone())).chain(l.files.iter().map(|f| Row::File(f.name.clone(), f.open))).collect()
    }

    fn step(&mut self, d: i32, cx: &mut Context<Self>) {
        let n = self.rows().len();
        if n == 0 {
            return;
        }
        self.cur = (self.cur as i32 + d).clamp(0, n as i32 - 1) as usize;
        self.list.scroll_to_item(self.cur);
        // A file under the keyboard shows in the preview, like Quick Look.
        if let Some(Row::File(name, true)) = self.rows().into_iter().nth(self.cur) {
            self.open_file(name, cx);
        }
        cx.notify();
    }

    fn open_cur(&mut self, cx: &mut Context<Self>) {
        match self.rows().into_iter().nth(self.cur) {
            Some(Row::Dir(f)) => self.go(f, cx),
            Some(Row::File(name, true)) => self.open_file(name, cx),
            _ => {}
        }
    }

    fn parent(&mut self, cx: &mut Context<Self>) {
        if self.folder.is_empty() {
            return;
        }
        let up = self.folder.rsplit_once('/').map(|(a, _)| a.to_string()).unwrap_or_default();
        self.go(up, cx);
    }

    /// The full path of a file in this folder, on its machine.
    fn path_of(&self, name: &str) -> String {
        remote_path(&self.root, &self.folder, name)
    }

    /// Chats on this machine whose folder is this one (or inside it), newest first.
    fn chats_here(&self, cx: &App) -> Vec<(String, String, bool)> {
        let app = self.app.read(cx);
        let Some(m) = app.snap.machine(&self.machine) else { return Vec::new() };
        let here = self.folder.trim_matches('/');
        m.sessions
            .iter()
            .filter(|s| !s.stale && (here.is_empty() && s.folder == m.root_label || !here.is_empty() && (s.folder == here || s.folder.starts_with(&format!("{here}/")))))
            .take(6)
            .map(|s| (s.key.clone(), s.topic.clone(), s.alive))
            .collect()
    }

    fn go(&mut self, folder: String, cx: &mut Context<Self>) {
        self.cur = 0;
        self.folder = folder;
        self.file = None;
        self.preview = None;
        self.load(cx);
    }

    fn set_machine(&mut self, machine: String, cx: &mut Context<Self>) {
        if machine == self.machine {
            return;
        }
        self.machine = machine;
        self.root.clear();
        self.listing = None;
        self.go(String::new(), cx);
    }

    fn open_file(&mut self, name: String, cx: &mut Context<Self>) {
        self.file = Some(name.clone());
        self.preview = None;
        cx.notify();
        let (core, m, f, seq) = (self.app.read(cx).core.clone(), self.machine.clone(), self.folder.clone(), self.seq);
        cx.spawn(async move |this, cx| {
            let res = cx.background_executor().spawn(async move { core.files(&m, &f, Some(&name)) }).await;
            let _ = this.update(cx, |s, cx| {
                if s.seq != seq {
                    return;
                }
                s.preview = Some(match res {
                    Ok(r) => r.file.ok_or_else(|| "The machine sent no file.".to_string()),
                    Err(e) => Err(e.to_string()),
                });
                s.pane.scroll_to_item(0);
                cx.notify();
            });
        })
        .detach();
    }

    fn new_here(&mut self, cx: &mut Context<Self>) {
        let (machine, folder) = (self.machine.clone(), self.folder.clone());
        self.app.update(cx, |_, cx| cx.emit(AppEvent::NewIn { machine, folder }));
    }

    fn crumbs(&self, t: &Theme, cx: &Context<Self>) -> AnyElement {
        let hover = t.hover;
        let crumb = |id: String, label: String, ic: Option<&str>, to: Option<String>, cx: &Context<Self>| {
            let cur = to.is_none();
            div()
                .id(SharedString::from(id))
                .flex_none()
                .h(px(26.))
                .px(px(7.))
                .flex()
                .items_center()
                .gap(px(6.))
                .rounded(px(ui::R_ITEM))
                .text_size(px(13.))
                .text_color(if cur { t.ink } else { t.accent })
                .when(!cur, |d| d.cursor_pointer().hover(move |s| s.bg(hover)))
                .children(ic.map(|i| ui::icon(i, 13., if cur { t.ink } else { t.accent })))
                .child(label)
                .when_some(to, |d, to| d.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.go(to.clone(), cx))))
        };
        let parts: Vec<&str> = if self.folder.is_empty() { Vec::new() } else { self.folder.split('/').collect() };
        let root = if self.root.is_empty() { "Claude folder".to_string() } else { self.root.clone() };
        let at_root = parts.is_empty() && self.file.is_none();
        let mut row = div().flex().flex_wrap().items_center().gap(px(2.)).child(crumb("fx-c-root".into(), root, Some("home"), (!at_root).then(String::new), cx));
        for (i, p) in parts.iter().enumerate() {
            let last = i + 1 == parts.len() && self.file.is_none();
            row = row
                .child(div().text_size(px(13.)).text_color(t.faint).child("/"))
                .child(crumb(format!("fx-c-{i}"), p.to_string(), None, (!last).then(|| parts[..=i].join("/")), cx));
        }
        if let Some(f) = &self.file {
            row = row.child(div().text_size(px(13.)).text_color(t.faint).child("/")).child(crumb("fx-c-file".into(), f.clone(), None, None, cx));
        }
        row.into_any_element()
    }

    fn entry(&self, t: &Theme, id: String, ic: &str, name: String, meta: Option<String>, on: bool, enabled: bool) -> gpui::Stateful<gpui::Div> {
        let (hover, sel) = (t.hover, t.sel);
        div()
            .id(SharedString::from(id))
            .h(px(34.))
            .px(px(10.))
            .flex()
            .items_center()
            .gap(px(10.))
            .rounded(px(ui::R_CTRL))
            .when(on, |d| d.bg(sel))
            .when(enabled && !on, |d| d.cursor_pointer().hover(move |s| s.bg(hover)))
            .child(ui::icon(ic, 15., if enabled { t.dim } else { t.faint }))
            .child(ui::wrap(name).flex_1().text_size(px(13.)).text_color(if enabled { t.ink } else { t.dim }))
            .children(meta.map(|m| div().flex_none().font_family(NUM).text_size(px(11.5)).text_color(t.faint).child(m)))
    }

    fn pane(&self, t: &Theme, now: f64, cx: &Context<Self>) -> AnyElement {
        let _ = &self.focus;
        let here = if self.folder.is_empty() { "New session here".to_string() } else { format!("New session in {}", self.folder.rsplit('/').next().unwrap_or("")) };
        let new_btn = ui::primary_btn(t, "fx-new", 32.)
            .px(px(14.))
            .text_size(px(12.5))
            .child(ui::icon("plus", 12., t.on_accent))
            .child(here)
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.new_here(cx)));
        let col = div().id("fx-pane").size_full().flex().flex_col().overflow_y_scroll().track_scroll(&self.pane).p(px(18.)).gap(px(12.));
        // The chats in this folder: their terminal one click away, and a file's path straight into one.
        let chats = self.chats_here(cx);
        let path = self.file.as_ref().map(|n| self.path_of(n));
        let hover = t.hover;
        let chats_el = (!chats.is_empty()).then(|| {
            let mut c = div().flex().flex_col().gap(px(2.)).child(div().text_size(px(12.)).font_weight(FontWeight::SEMIBOLD).text_color(t.dim).child("Chats in this folder"));
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
                        .child(ui::wrap(topic).flex_1().min_w_0().text_size(px(13.)).text_color(t.ink))
                        .when_some(p, |d, p| {
                            d.child(ui::ghost(t, SharedString::from(format!("fx-paste-{k2}")), Some("Paste the path"), None).h(px(26.)).px(px(8.)).text_size(px(12.)).on_click(cx.listener(
                                move |this, _: &ClickEvent, _, cx| {
                                    let (key, text) = (k2.clone(), format!("{p} "));
                                    this.app.update(cx, |_, cx| cx.emit(AppEvent::PasteInto { key, text }));
                                },
                            )))
                        })
                        .child(ui::line_btn(t, SharedString::from(format!("fx-open-{k1}")), 26.).px(px(9.)).text_size(px(12.)).child(if alive { "Open terminal" } else { "Resume" }).on_click(cx.listener(
                            move |this, _: &ClickEvent, _, cx| {
                                let key = k1.clone();
                                this.app.update(cx, |_, cx| cx.emit(AppEvent::OpenTerm { key, resume: !alive }));
                            },
                        ))),
                );
            }
            c
        });
        let Some(name) = &self.file else {
            return col
                .child(ui::wrap(if self.folder.is_empty() { self.root.clone() } else { self.folder.clone() }).text_size(px(15.)).font_weight(FontWeight::BOLD))
                .child(div().text_size(px(12.5)).text_color(t.dim).child("Pick a file to read it here, or move with the arrow keys. Files that look like keys or secrets stay closed."))
                .child(div().flex().child(new_btn))
                .children(chats_el)
                .into_any_element();
        };
        let col = col.children(chats_el);
        let mut col = col.child(ui::wrap(name.clone()).text_size(px(15.)).font_weight(FontWeight::BOLD));
        match &self.preview {
            None => col.child(div().text_size(px(12.5)).text_color(t.faint).child("Reading...")),
            Some(Err(e)) => col.child(div().text_size(px(12.5)).text_color(t.bad).child(e.clone())),
            Some(Ok(f)) => {
                let meta = format!("{} · changed {}", size_text(f.size), fmt::ago(f.mtime, now));
                col = col.child(div().flex().items_center().gap(px(12.)).child(div().flex_1().text_size(px(12.)).text_color(t.dim).child(meta)).child(new_btn));
                if f.secret {
                    return col.child(div().text_size(px(13.)).text_color(t.dim).child("This looks like a key or a secret, so Switchboard does not open it.")).into_any_element();
                }
                let Some(text) = f.text.as_ref().filter(|_| !f.binary) else {
                    return col.child(div().text_size(px(13.)).text_color(t.dim).child("Not a text file, so there is nothing to show here.")).into_any_element();
                };
                let mut body = div().p(px(12.)).rounded(px(ui::R_CTRL)).border_1().border_color(t.line).bg(t.bg2).font_family(NUM).text_size(px(12.)).text_color(t.ink).flex().flex_col();
                let lines: Vec<&str> = text.lines().collect();
                for l in lines.iter().take(PREVIEW_LINES) {
                    body = body.child(div().min_h(px(16.)).child(l.replace('\t', "    ")));
                }
                col = col.child(body);
                if f.cut || lines.len() > PREVIEW_LINES {
                    col = col.child(div().text_size(px(12.)).text_color(t.faint).child("Showing the start of the file."));
                }
                col
            }
        }
        .into_any_element()
    }
}

fn size_text(bytes: f64) -> String {
    if bytes <= 0. {
        "Empty".into()
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
        let machines: Vec<(String, String, bool)> = self.app.read(cx).snap.machines.iter().map(|m| (m.name.clone(), m.label.clone(), files_ok(m))).collect();
        let mut mseg = ui::seg_box(&t);
        for (name, label, ok) in machines {
            let on = self.machine == name;
            let mut b = ui::seg_btn(&t, SharedString::from(format!("fx-m-{name}")), on, !ok).child(label);
            if ok {
                b = b.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.set_machine(name.clone(), cx)));
            }
            mseg = mseg.child(b);
        }
        let header = div()
            .flex_none()
            .px(px(16.))
            .pt(px(14.))
            .pb(px(10.))
            .flex()
            .flex_col()
            .gap(px(10.))
            .border_b_1()
            .border_color(t.rule)
            .bg(t.bg2)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.))
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
                    .child(ui::icon_btn(&t, "fx-reload", "restore", 28.).on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.load(cx)))),
            )
            .child(self.crumbs(&t, cx));

        let mut list = div().id("fx-list").flex_1().min_h_0().overflow_y_scroll().track_scroll(&self.list).p(px(10.)).flex().flex_col().gap(px(1.));
        if self.machine.is_empty() {
            list = list.child(div().p(px(12.)).text_size(px(13.)).text_color(t.faint).child("Files show up here once a machine has the latest Switchboard update."));
        } else if let Some(e) = &self.error {
            list = list.child(div().p(px(12.)).text_size(px(13.)).text_color(t.bad).child(format!("Could not read the folder. {e}")));
        } else if let Some(l) = self.listing.as_ref().filter(|l| l.folder == self.folder || !self.loading) {
            for (i, d) in l.dirs.iter().enumerate() {
                let to = d.folder.clone();
                list = list.child(
                    self.entry(&t, format!("fx-d-{}", d.folder), "folder", d.name.clone(), None, self.cur == i, true)
                        .child(ui::icon("chev", 11., t.faint))
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.go(to.clone(), cx))),
                );
            }
            let nd = l.dirs.len();
            for (i, f) in l.files.iter().enumerate() {
                let meta = format!("{}{} · {}", if f.open { "" } else { "hidden · " }, size_text(f.size), fmt::sago(f.mtime, now));
                let on = self.file.as_deref() == Some(f.name.as_str()) || self.cur == nd + i;
                let mut e = self.entry(&t, format!("fx-f-{}", f.name), "file", f.name.clone(), Some(meta), on, f.open);
                if f.open {
                    let name = f.name.clone();
                    e = e.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.open_file(name.clone(), cx)));
                }
                list = list.child(e);
            }
            if l.dirs.is_empty() && l.files.is_empty() {
                list = list.child(div().p(px(12.)).text_size(px(13.)).text_color(t.faint).child("Nothing in this folder."));
            }
            if l.more {
                list = list.child(div().p(px(12.)).text_size(px(12.)).text_color(t.faint).child("Showing the first 400 of each."));
            }
        } else {
            list = list.child(div().p(px(12.)).text_size(px(13.)).text_color(t.faint).child("Reading the folder..."));
        }

        div()
            .id("files")
            .key_context("Files")
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &FxUp, _, cx| this.step(-1, cx)))
            .on_action(cx.listener(|this, _: &FxDown, _, cx| this.step(1, cx)))
            .on_action(cx.listener(|this, _: &FxOpen, _, cx| this.open_cur(cx)))
            .on_action(cx.listener(|this, _: &FxInto, _, cx| {
                if let Some(Row::Dir(f)) = this.rows().into_iter().nth(this.cur) {
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
                    .child(div().flex_1().min_w_0().flex().flex_col().child(list))
                    .child(div().flex_none().w(px(480.)).h_full().min_h_0().border_l_1().border_color(t.rule).bg(t.panel).child(self.pane(&t, now, cx))),
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

#[cfg(test)]
mod tests {
    #[test]
    fn a_files_path_on_its_machine() {
        assert_eq!(super::remote_path("/Users/you/Claude/", "notes/today", "a.md"), "/Users/you/Claude/notes/today/a.md");
        assert_eq!(super::remote_path("C:\\Users\\you\\code", "", "b.txt"), "C:\\Users\\you\\code\\b.txt");
    }
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
        let os = if cfg!(target_os = "macos") { "mac" } else if cfg!(windows) { "windows" } else { "linux" };
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
                "Link another computer that runs Claude Code. It dials out to this hub, so it needs no SSH, only your network or your tailnet.",
            ));
        let err = self.error.clone().map(|e| div().text_size(px(12.5)).text_color(t.bad).child(e));
        let Some(r) = self.r.clone() else {
            let steps = ["Get a code here. It works once, for 10 minutes.", "Paste one line in a terminal on the new machine.", "It shows up on the board within a minute."];
            for (i, s) in steps.iter().enumerate() {
                col = col.child(
                    div()
                        .flex()
                        .gap(px(10.))
                        .text_size(px(13.))
                        .child(div().flex_none().w(px(18.)).font_family(NUM).text_color(t.faint).child(format!("{}.", i + 1)))
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
            seg = seg.child(ui::seg_btn(&t, SharedString::from(format!("pair-os-{k}")), self.os == k, false).child(label).on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.os = k;
                this.copied = false;
                cx.notify();
            })));
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
                "It downloads switchboard-node, links it with the code and starts it at login. If Claude Code there has no Switchboard yet, it adds that too; one already there is left alone.",
            ))
            .into_any_element()
    }
}
