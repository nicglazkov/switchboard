//! v2, History (the Session Browser): every chat on every machine, as a tab next to Board in the main window (and
//! still a window of its own from the palette). Search, machine and state filters,
//! threads folded into one row, and on the right the chat itself (its first message, the latest ones and Claude's
//! last answer, read from the transcript on its machine) with what brings it back: Resume, Open in Claude, a
//! terminal, and a name.

use std::collections::HashMap;
use std::time::Duration;

use gpui::{
    AnyElement, AnyWindowHandle, App, AppContext, ClickEvent, Context, Entity, FocusHandle, Focusable, FontWeight, InteractiveElement, IntoElement, ParentElement,
    Render, ScrollHandle, SharedString, StatefulInteractiveElement, Styled, Subscription, TitlebarOptions, Window, WindowBackgroundAppearance,
    WindowBounds, WindowDecorations, WindowOptions, div, prelude::*, px, relative, size,
};
use gpui_base::input::{InputEvent, InputState};
use sb_core::api::{ApiBrowseRow, ApiChatSummary, ApiCompact};
use sb_core::{fmt, names};

use crate::state::{AppEvent, AppState, copy_text, open_link};
use crate::theme::{NUM, Theme};
use crate::ui;

const STATES: [(&str, &str); 6] = [("", "All"), ("live", "Running"), ("quiet", "Quiet"), ("stopped", "Stopped"), ("archived", "Archived"), ("replaced", "Replaced")];
const PAGE: usize = 200;

/// Opens the browser, or brings it forward when it is open.
pub fn open(cx: &mut App) {
    let app = crate::state::app(cx);
    if let Some(h) = app.read(cx).browser_win {
        if h.update(cx, |_, window, _| window.activate_window()).is_ok() {
            return;
        }
    }
    let bounds = gpui::Bounds::centered(None, size(px(1180.), px(780.)), cx);
    let opts = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: Some(TitlebarOptions { title: Some("Sessions, Switchboard".into()), appears_transparent: false, traffic_light_position: None }),
        window_min_size: Some(size(px(760.), px(480.))),
        app_id: Some("switchboard".into()),
        window_background: WindowBackgroundAppearance::Opaque,
        window_decorations: Some(WindowDecorations::Server),
        ..Default::default()
    };
    match cx.open_window(opts, |window, cx| cx.new(|cx| BrowserView::new(window, cx))) {
        Ok(h) => {
            let any: AnyWindowHandle = h.into();
            app.update(cx, |s, _| s.browser_win = Some(any));
        }
        Err(e) => log::error!("could not open the session browser: {e:#}"),
    }
}

fn key(machine: &str, sid: &str) -> String {
    format!("{machine}:{sid}")
}

/// A chat name the server takes: letters, digits, spaces and . , : ' + # ( ) - _ (up to 64).
fn size_text(bytes: f64) -> String {
    if bytes >= 1_000_000. { format!("{:.1} MB", bytes / 1_000_000.) } else { format!("{} KB", (bytes / 1000.).round().max(1.) as u64) }
}

pub struct BrowserView {
    app: Entity<AppState>,
    /// Drawn inside the main window as the History tab (no window of its own).
    embedded: bool,
    /// The folder filter ("" is every folder), the folders that have chats, and its menu.
    folder: String,
    folders: Vec<String>,
    folder_open: bool,
    focus: FocusHandle,
    q: Entity<InputState>,
    rn: Entity<InputState>,
    machine: String,
    state: &'static str,
    threads: bool,
    rows: Vec<ApiBrowseRow>,
    total: u64,
    loading: bool,
    error: Option<String>,
    seq: u64,
    /// The chat shown on the right ("machine:sid"), and its row (a row of the list or an older chat of its thread).
    sel: Option<String>,
    sel_row: Option<ApiBrowseRow>,
    sums: HashMap<String, Result<ApiChatSummary, String>>,
    /// The thread whose chats Additional history lists: the list row picked last ("machine:sid"). Picking one of
    /// its earlier chats keeps it, so the list stays put.
    head: Option<String>,
    /// Written summaries, read when a chat that has one is shown.
    notes: HashMap<String, Result<String, String>>,
    /// Compact into one fresh chat: the thread it is for, and its cost once the server has worked it out.
    compact: Option<(String, Option<Result<ApiCompact, String>>)>,
    renaming: bool,
    list: ScrollHandle,
    side: ScrollHandle,
    _subs: Vec<Subscription>,
}

impl BrowserView {
    /// History, inside the main window.
    pub fn embedded(window: &mut Window, cx: &mut Context<Self>) -> BrowserView {
        let mut v = BrowserView::new(window, cx);
        v.embedded = true;
        v
    }

    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> BrowserView {
        let app = crate::state::app(cx);
        let q = cx.new(|cx| InputState::new(window, cx).placeholder("Search names, folders, prompts"));
        let rn = cx.new(|cx| InputState::new(window, cx).placeholder("A name for this chat"));
        let subs = vec![
            cx.observe(&app, |_, _, cx| cx.notify()),
            cx.subscribe(&q, |this, _, ev: &InputEvent, cx| {
                if let InputEvent::Change = ev {
                    this.load_soon(cx);
                }
            }),
            cx.observe_window_appearance(window, |_, _, cx| cx.notify()),
        ];
        let focus = cx.focus_handle();
        q.update(cx, |q, cx| q.focus(window, cx));
        let mut v = BrowserView {
            app,
            embedded: false,
            folder: String::new(),
            folders: Vec::new(),
            folder_open: false,
            focus,
            q,
            rn,
            machine: String::new(),
            state: "",
            threads: true,
            rows: Vec::new(),
            total: 0,
            loading: false,
            error: None,
            seq: 0,
            sel: None,
            sel_row: None,
            sums: HashMap::new(),
            head: None,
            notes: HashMap::new(),
            compact: None,
            renaming: false,
            list: ScrollHandle::new(),
            side: ScrollHandle::new(),
            _subs: subs,
        };
        v.load(false, cx);
        v
    }

    fn load_soon(&mut self, cx: &mut Context<Self>) {
        self.seq += 1;
        let seq = self.seq;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(220)).await;
            let _ = this.update(cx, |s, cx| {
                if s.seq == seq {
                    s.load(false, cx);
                }
            });
        })
        .detach();
    }

    fn load(&mut self, more: bool, cx: &mut Context<Self>) {
        use sb_core::net::enc;
        self.seq += 1;
        let seq = self.seq;
        let text = self.q.read(cx).value().trim().to_string();
        let mut query = format!("thread={}&limit={PAGE}", if self.threads { 1 } else { 0 });
        if !text.is_empty() {
            query.push_str(&format!("&q={}", enc(&text)));
        }
        if !self.machine.is_empty() {
            query.push_str(&format!("&machine={}", enc(&self.machine)));
        }
        if !self.state.is_empty() {
            query.push_str(&format!("&state={}", self.state));
        }
        if !self.folder.is_empty() {
            query.push_str(&format!("&folder={}", enc(&self.folder)));
        }
        if more {
            query.push_str(&format!("&offset={}", self.rows.len()));
        }
        self.loading = true;
        cx.notify();
        let core = self.app.read(cx).core.clone();
        cx.spawn(async move |this, cx| {
            let res = cx.background_executor().spawn(async move { core.browse(&query) }).await;
            let _ = this.update(cx, |s, cx| {
                if s.seq != seq {
                    return;
                }
                s.loading = false;
                match res {
                    Ok(b) => {
                        s.error = None;
                        if let Some(f) = &b.facets {
                            s.folders = f.folders.clone();
                        }
                        s.total = b.total.unwrap_or(b.rows.len() as u64);
                        if more {
                            s.rows.extend(b.rows);
                        } else {
                            s.rows = b.rows;
                            s.list.scroll_to_item(0);
                        }
                        let still = s.sel.as_ref().is_some_and(|k| s.rows.iter().any(|r| key(&r.machine, &r.sid) == *k));
                        if !still && s.sel_row.is_none() {
                            if let Some(r) = s.rows.first().cloned() {
                                s.pick(r, cx);
                            }
                        }
                        // Screenshots: SB_SHOT_HIST="<row>[,older][,cost]" picks a thread, one of its earlier chats, the cost.
                        #[cfg(feature = "shots")]
                        if let Some(spec) = std::env::var("SB_SHOT_HIST").ok().filter(|_| !more) {
                            let n: usize = spec.split(',').next().and_then(|x| x.parse().ok()).unwrap_or(0);
                            if let Some(r) = s.rows.get(n).cloned() {
                                s.pick(r.clone(), cx);
                                if spec.contains("older") {
                                    if let Some(o) = r.history.first().cloned() {
                                        s.show(o, cx);
                                    }
                                }
                                if spec.contains("cost") {
                                    s.compact_cost(&r, cx);
                                }
                            }
                        }
                    }
                    Err(e) => s.error = Some(e.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Shows a chat from the list: its thread's earlier chats go under Additional history.
    fn pick(&mut self, r: ApiBrowseRow, cx: &mut Context<Self>) {
        self.head = Some(key(&r.machine, &r.sid));
        self.show(r, cx)
    }

    /// Shows a chat on the right, from the list or from Additional history, and reads what it said (and its
    /// summary, when one is written).
    fn show(&mut self, r: ApiBrowseRow, cx: &mut Context<Self>) {
        let k = key(&r.machine, &r.sid);
        if r.handoff == Some(true) && !self.notes.contains_key(&k) {
            let core = self.app.read(cx).core.clone();
            let (m, sid, k2) = (r.machine.clone(), r.sid.clone(), k.clone());
            cx.spawn(async move |this, cx| {
                let res = cx.background_executor().spawn(async move { core.handoff_text(&m, &sid) }).await;
                let _ = this.update(cx, |s, cx| {
                    s.notes.insert(k2, res.map_err(|e| e.to_string()));
                    cx.notify();
                });
            })
            .detach();
        }
        self.renaming = false;
        self.sel = Some(k.clone());
        self.sel_row = Some(r.clone());
        self.side.scroll_to_item(0);
        cx.notify();
        let can = self.app.read(cx).snap.machine(&r.machine).is_none_or(|m| m.features.is_empty() || m.features.iter().any(|f| f == "summary"));
        if self.sums.contains_key(&k) || !can {
            return;
        }
        let core = self.app.read(cx).core.clone();
        let (m, sid) = (r.machine.clone(), r.sid.clone());
        cx.spawn(async move |this, cx| {
            let res = cx.background_executor().spawn(async move { core.chat_summary(&m, &sid) }).await;
            let _ = this.update(cx, |s, cx| {
                s.sums.insert(k, res.map_err(|e| e.to_string()));
                cx.notify();
            });
        })
        .detach();
    }

    /// The cost of compacting the thread of `head` (the newest chat), asked of the server; nothing is written yet.
    fn compact_cost(&mut self, head: &ApiBrowseRow, cx: &mut Context<Self>) {
        let k = key(&head.machine, &head.sid);
        self.compact = Some((k.clone(), None));
        cx.notify();
        let core = self.app.read(cx).core.clone();
        let (m, sid) = (head.machine.clone(), head.sid.clone());
        cx.spawn(async move |this, cx| {
            let res = cx.background_executor().spawn(async move { core.compact(&m, &sid, false) }).await;
            let _ = this.update(cx, |s, cx| {
                if s.compact.as_ref().is_some_and(|(x, _)| *x == k) {
                    s.compact = Some((k, Some(res.map_err(|e| e.to_string()))));
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn set_filter(&mut self, machine: Option<String>, state: Option<&'static str>, cx: &mut Context<Self>) {
        if let Some(m) = machine {
            self.machine = m;
        }
        if let Some(s) = state {
            self.state = s;
        }
        self.load(false, cx);
    }

    fn resume(&mut self, r: &ApiBrowseRow, cx: &mut Context<Self>) {
        let (m, sid, title) = (r.machine.clone(), r.sid.clone(), r.title.clone());
        self.app.update(cx, |s, cx| s.resume_chat(&m, &sid, title, cx));
    }

    /// A terminal for the chat in the board window: attached when it runs, resumed first when it does not.
    fn terminal(&mut self, r: &ApiBrowseRow, cx: &mut Context<Self>) {
        let k = key(&r.machine, &r.sid);
        let app = self.app.clone();
        let on_board = app.read(cx).snap.session(&k).map(|s| s.alive);
        match on_board {
            Some(alive) => app.update(cx, |_, cx| cx.emit(AppEvent::OpenTerm { key: k, resume: !alive })),
            None => {
                let (m, sid) = (r.machine.clone(), r.sid.clone());
                let label = r.label.clone().unwrap_or_else(|| m.clone());
                let title = r.title.clone().unwrap_or_else(|| "Untitled".into());
                let core = app.read(cx).core.clone();
                cx.spawn(async move |_, cx| {
                    let res = cx.background_executor().spawn(async move { core.act(&m, "resume", &sid, None).map(|j| (j, m)) }).await;
                    let _ = app.update(cx, |s, cx| match res {
                        Ok((Some(job), machine)) => cx.emit(AppEvent::OpenJobTerm { job, machine, label, title: names::resumed(&names::split(&title).0) }),
                        Ok((None, _)) => s.toast("The server took the resume but did not say which job it is.", None, true, cx),
                        Err(e) => s.toast(e.to_string(), None, true, cx),
                    });
                })
                .detach();
            }
        }
    }

    fn save_name(&mut self, cx: &mut Context<Self>) {
        let Some(r) = self.sel_row.clone() else { return };
        // The same rules as everywhere (rename.rs): the tag it had stays, an empty name goes back to the automatic one.
        let typed = self.rn.read(cx).value().to_string();
        let name = names::given(&typed, r.title.as_deref().unwrap_or(""));
        if name.is_empty() && !typed.trim().is_empty() {
            self.app.update(cx, |s, cx| s.toast("A name needs letters or digits.", Some("It may use letters, digits, spaces and . , : ' + # ( ) - _".into()), true, cx));
            return;
        }
        let me = cx.entity().downgrade();
        let (m, sid, n2) = (r.machine.clone(), r.sid.clone(), name.clone());
        self.app.update(cx, |s, cx| {
            s.rename(
                m,
                sid,
                name,
                move |ok, cx| {
                    let _ = me.update(cx, |b, cx| {
                        if ok {
                            b.renaming = false;
                            for row in b.rows.iter_mut().chain(b.sel_row.iter_mut()) {
                                if row.machine == r.machine && row.sid == r.sid && !n2.is_empty() {
                                    row.title = Some(n2.clone());
                                }
                            }
                        }
                        cx.notify();
                    });
                },
                cx,
            )
        });
    }

    // ---------- rendering ----------

    fn lamp(&self, t: &Theme, r: &ApiBrowseRow, now: f64, cx: &App) -> AnyElement {
        if let Some(s) = self.app.read(cx).snap.session(&key(&r.machine, &r.sid)).filter(|s| s.alive) {
            return ui::lamp(t, s.lamp(now), 8.).into_any_element();
        }
        match r.state.as_deref() {
            Some("live") => div().flex_none().size(px(8.)).rounded_full().bg(t.run).into_any_element(),
            Some("quiet") => div().flex_none().size(px(8.)).rounded_full().bg(t.idle).into_any_element(),
            _ => ui::ring_lamp(t, 8.).into_any_element(),
        }
    }

    fn state_word(r: &ApiBrowseRow) -> Option<&'static str> {
        match r.state.as_deref() {
            Some("live") => Some("Running"),
            Some("quiet") => Some("Quiet"),
            Some("stopped") => Some("Stopped"),
            Some("archived") => Some("Archived"),
            Some("replaced") => Some("Replaced"),
            _ => None,
        }
    }

    fn row(&self, t: &Theme, r: &ApiBrowseRow, now: f64, cx: &Context<Self>) -> AnyElement {
        let k = key(&r.machine, &r.sid);
        let on = self.sel.as_deref() == Some(k.as_str());
        let title = r.title.clone().filter(|x| !x.trim().is_empty()).unwrap_or_else(|| "Untitled".into());
        let (topic, tag) = names::split(&title);
        let mut meta = vec![r.label.clone().unwrap_or_else(|| r.machine.clone())];
        if let Some(f) = r.folder.clone().filter(|f| !f.is_empty()) {
            meta.push(f);
        }
        if let Some(p) = r.last_prompt.clone().or_else(|| r.first_prompt.clone()).filter(|p| !p.trim().is_empty()) {
            meta.push(p.lines().next().unwrap_or("").to_string());
        }
        let n_older = r.history_count.unwrap_or(r.history.len() as u64);
        let hover = t.hover;
        let r2 = r.clone();
        div()
            .id(SharedString::from(format!("br-{k}")))
            .min_h(px(52.))
            .py(px(7.))
            .px(px(12.))
            .flex()
            .items_center()
            .gap(px(12.))
            .rounded(px(ui::R_CTRL))
            .cursor_pointer()
            .when(on, |d| d.bg(t.sel))
            .when(!on, |d| d.hover(move |s| s.bg(hover)))
            .on_click(cx.listener(move |this, ev: &ClickEvent, _, cx| {
                this.sel_row = None;
                this.pick(r2.clone(), cx);
                if ev.click_count() >= 2 {
                    let r = r2.clone();
                    this.resume(&r, cx);
                }
            }))
            .child(div().flex_none().w(px(10.)).flex().justify_center().child(self.lamp(t, r, now, cx)))
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
                            .items_center()
                            .gap(px(6.))
                            .min_w_0()
                            .flex_wrap()
                            .child(div().max_w_full().text_size(px(13.5)).line_height(relative(1.35)).font_weight(FontWeight::MEDIUM).text_color(if on { t.ink } else { t.ink2 }).child(topic.to_string()))
                            .when_some(tag, |d, tg| d.child(ui::tag(t, &tg))),
                    )
                    .child(ui::wrap(meta.join(" · ")).text_size(px(12.)).line_height(relative(1.4)).text_color(t.faint)),
            )
            .when(n_older > 0, |d| {
                d.child(
                    div()
                        .flex_none()
                        .flex()
                        .items_center()
                        .gap(px(4.))
                        .px(px(7.))
                        .h(px(20.))
                        .rounded_full()
                        .bg(t.chip_bg)
                        .text_size(px(11.))
                        .text_color(t.chip_ink)
                        .child(ui::icon("history", 11., t.dim))
                        .child(format!("{} chats", n_older + 1)),
                )
            })
            .child(
                div()
                    .flex_none()
                    .w(px(54.))
                    .flex()
                    .justify_end()
                    .font_family(NUM)
                    .text_size(px(11.5))
                    .text_color(t.faint)
                    .child(fmt::sago(r.last.unwrap_or(0.), now)),
            )
            .into_any_element()
    }

    fn fact(t: &Theme, label: &str, value: String) -> AnyElement {
        div()
            .flex()
            .gap(px(10.))
            .text_size(px(12.5))
            .child(div().flex_none().w(px(76.)).text_color(t.faint).child(label.to_string()))
            .child(ui::wrap(value).flex_1().min_w_0().text_color(t.ink2))
            .into_any_element()
    }

    fn said(t: &Theme, label: &str, text: &str, when: Option<String>) -> AnyElement {
        let text: String = text.chars().take(700).collect();
        div()
            .flex()
            .flex_col()
            .gap(px(5.))
            .child(div().flex().items_center().gap(px(8.)).child(ui::lbl(label, t.faint)).children(when.map(|w| div().text_size(px(11.)).text_color(t.faint).child(w))))
            .child(
                div()
                    .px(px(11.))
                    .py(px(8.))
                    .rounded(px(ui::R_CTRL))
                    .bg(if t.dark { t.bg2 } else { t.bg })
                    .border_1()
                    .border_color(t.rule)
                    .text_size(px(12.5))
                    .line_height(relative(1.5))
                    .text_color(t.ink2)
                    .child(text),
            )
            .into_any_element()
    }

    /// One chat in Additional history: its name, when and where, its size, its first and last message, and a
    /// tag when a summary is written. Clicking it shows it in full above.
    fn older_card(&self, t: &Theme, o: &ApiBrowseRow, badge: &str, shown: &str, cx: &Context<Self>) -> AnyElement {
        let ok = key(&o.machine, &o.sid);
        let on = ok == shown;
        let hover = t.hover;
        let title = o.title.clone().filter(|x| !x.trim().is_empty()).unwrap_or_else(|| "Untitled".into());
        let (topic, tag) = names::split(&title);
        let mut meta = Vec::new();
        if let Some(st) = o.started.or(o.last) {
            meta.push(format!("{} {}", fmt::md(st), fmt::time(st)));
        }
        meta.push(o.label.clone().unwrap_or_else(|| o.machine.clone()));
        if let Some(c) = o.ctx.filter(|c| *c > 0.) {
            meta.push(fmt::k(c as u64));
        }
        let said = |label: &str, text: &Option<String>| {
            text.as_ref().and_then(|p| p.lines().find(|l| !l.trim().is_empty())).map(|line| {
                div()
                    .flex()
                    .gap(px(6.))
                    .text_size(px(12.))
                    .line_height(relative(1.45))
                    .child(div().flex_none().w(px(34.)).text_color(t.faint).child(label.to_string()))
                    .child(ui::wrap(line.to_string()).flex_1().text_color(t.dim))
            })
        };
        let row = o.clone();
        div()
            .id(SharedString::from(format!("br-h-{}", o.sid)))
            .p(px(10.))
            .flex()
            .flex_col()
            .gap(px(4.))
            .rounded(px(ui::R_CARD))
            .border_1()
            .border_color(if on { t.accent } else { t.line })
            .when(on, |d| d.bg(t.sel))
            .when(!on, |d| d.cursor_pointer().hover(move |s| s.bg(hover)))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap_x(px(6.))
                    .gap_y(px(3.))
                    .child(div().max_w_full().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).text_color(t.ink).child(topic))
                    .when_some(tag, |d, tg| d.child(ui::tag(t, &tg)))
                    .when(!badge.is_empty(), |d| d.child(ui::tag(t, badge)))
                    .when(o.handoff == Some(true), |d| d.child(ui::note_tag(t, "handoff", "Summary written"))),
            )
            .child(ui::wrap(meta.join(" · ")).text_size(px(12.)).text_color(t.faint))
            .children(said("First", &o.first_prompt))
            .children(said("Last", &o.last_prompt).filter(|_| o.last_prompt != o.first_prompt))
            .when(!on, |d| d.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.show(row.clone(), cx))))
            .into_any_element()
    }

    /// Compact into one fresh chat: what it does, then its cost, then a confirm. Nothing is written until then.
    fn compact_card(&self, t: &Theme, head: &ApiBrowseRow, cx: &Context<Self>) -> AnyElement {
        let hk = key(&head.machine, &head.sid);
        let n = head.history_count.unwrap_or(head.history.len() as u64) + 1;
        let busy = self.app.read(cx).busy.contains(&format!("{hk}:compact"));
        let ask = self.compact.as_ref().filter(|(x, _)| *x == hk).map(|(_, r)| r.clone());
        let mut card = div()
            .p(px(ui::PAD_CARD))
            .rounded(px(ui::R_CARD))
            .bg(t.bg)
            .border_1()
            .border_color(t.line)
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(div().flex().items_center().gap(px(7.)).child(ui::icon("fresh", 14., t.accent)).child(div().text_size(px(13.5)).font_weight(FontWeight::BOLD).text_color(t.ink).child("Compact into one fresh chat")))
            .child(ui::wrap(format!(
                "Writes one summary of all {n} chats in this thread (each chat's own summary where it has one, otherwise its condensed transcript), then opens a new chat from it. The old chats stay here."
            )).text_size(px(12.5)).line_height(relative(1.45)).text_color(t.dim));
        let h2 = head.clone();
        card = match ask {
            None => card.child(div().flex().child(ui::line_btn(t, "br-compact-cost", 30.).px(px(12.)).text_size(px(12.5)).child("See what it costs").on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.compact_cost(&h2, cx))))),
            Some(None) => card.child(div().text_size(px(12.5)).text_color(t.faint).child("Working out the cost on each machine...")),
            Some(Some(Err(e))) => card.child(ui::wrap(e).text_size(px(12.5)).text_color(t.bad)).child(div().flex().child(ui::line_btn(t, "br-compact-retry", 30.).px(px(12.)).text_size(px(12.5)).child("Try again").on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.compact_cost(&h2, cx))))),
            Some(Some(Ok(c))) => {
                let cost = c.cost.clone().unwrap_or_default();
                let mut note = cost.text.clone();
                if c.found < c.chats {
                    note.push_str(&format!(" {} of the {} chats could not be read (their machine is offline) and are left out.", c.chats - c.found, c.chats));
                }
                if c.notes > 0 {
                    note.push_str(&format!(" {} already {} a summary, used as it is.", c.notes, if c.notes == 1 { "has" } else { "have" }));
                }
                let (m, sid, title, label) = (c.machine.clone().unwrap_or_else(|| head.machine.clone()), c.sid.clone().unwrap_or_else(|| head.sid.clone()), head.title.clone(), c.label.clone().unwrap_or_else(|| head.label.clone().unwrap_or_else(|| head.machine.clone())));
                card.child(ui::wrap(note).text_size(px(12.5)).line_height(relative(1.45)).text_color(t.ink2)).child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap(px(8.))
                        .child(
                            ui::primary_btn(t, "br-compact-go", 32.)
                                .px(px(14.))
                                .text_size(px(13.))
                                .when(busy, |d| d.opacity(0.6))
                                .child(ui::icon("fresh", 14., t.on_accent))
                                .child(if busy { "Starting" } else { "Compact into one chat" })
                                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                    let (m, sid, title, label) = (m.clone(), sid.clone(), title.clone(), label.clone());
                                    this.app.update(cx, |s, cx| s.compact_thread(&m, &sid, title, label, cx));
                                    this.compact = None;
                                    cx.notify();
                                })),
                        )
                        .child(ui::line_btn(t, "br-compact-x", 32.).px(px(12.)).text_size(px(12.5)).child("Cancel").on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.compact = None;
                            cx.notify();
                        }))),
                )
            }
        };
        card.into_any_element()
    }

    fn detail(&self, t: &Theme, now: f64, focused: bool, cx: &Context<Self>) -> AnyElement {
        let Some(r) = self.sel_row.clone() else {
            let msg = if self.loading { "Looking through every machine..." } else { "Pick a chat to see what it was about." };
            return div().size_full().flex().items_center().justify_center().text_size(px(13.)).text_color(t.faint).child(msg).into_any_element();
        };
        let k = key(&r.machine, &r.sid);
        let app = self.app.read(cx);
        let s = app.snap.session(&k);
        let alive = s.is_some_and(|s| s.alive);
        let label = r.label.clone().unwrap_or_else(|| r.machine.clone());
        let title = r.title.clone().filter(|x| !x.trim().is_empty()).unwrap_or_else(|| "Untitled".into());
        let busy = app.busy.contains(&format!("{k}:resume"));
        let url = r.url.clone().or_else(|| s.and_then(|s| s.url.clone()));
        let can_rename = app.snap.machine(&r.machine).is_none_or(|m| m.features.iter().any(|f| f == "rename"));

        let head = if self.renaming {
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .child(
                    div()
                        .flex_1()
                        .h(px(32.))
                        .px(px(10.))
                        .flex()
                        .items_center()
                        .rounded(px(ui::R_CTRL))
                        .border_1()
                        .border_color(if focused { t.accent } else { t.line })
                        .text_size(px(13.5))
                        .text_color(t.ink)
                        .child(gpui_base::Input::new(&self.rn)),
                )
                .child(ui::primary_btn(t, "br-rn-save", 32.).px(px(12.)).text_size(px(12.5)).child("Save").on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.save_name(cx))))
                .child(ui::ghost(t, "br-rn-x", None, None).h(px(32.)).px(px(8.)).text_size(px(12.5)).child("Cancel").on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.renaming = false;
                    cx.notify();
                })))
                .into_any_element()
        } else {
            let t2 = title.clone();
            div()
                .flex()
                .items_start()
                .gap(px(8.))
                // The name and its tag ("fresh") wrap together, the tag after the last word.
                .child({
                    let (topic, tag) = names::split(&title);
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap_x(px(8.))
                        .gap_y(px(4.))
                        .child(div().max_w_full().text_size(px(16.)).font_weight(FontWeight::BOLD).line_height(relative(1.3)).text_color(t.ink).child(topic))
                        .when_some(tag, |d, tg| d.child(ui::tag(t, &tg)))
                })
                .when(can_rename, |d| {
                    d.child(ui::ghost(t, "br-rn", None, None).h(px(26.)).px(px(8.)).text_size(px(12.)).child("Rename").on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        this.renaming = true;
                        let v = names::split(&t2).0.to_string();
                        this.rn.update(cx, |n, cx| {
                            n.set_value(v, window, cx);
                            n.focus(window, cx);
                        });
                        cx.notify();
                    })))
                })
                .into_any_element()
        };

        let state = Self::state_word(&r).unwrap_or("");
        let mut facts = vec![Self::fact(t, "Where", format!("{label}{}", r.folder.as_ref().filter(|f| !f.is_empty()).map(|f| format!(", {f}")).unwrap_or_default()))];
        facts.push(Self::fact(t, "State", format!("{state}{}", r.last.map(|l| format!(", last active {}", fmt::ago(l, now))).unwrap_or_default())));
        if let Some(m) = &r.model {
            facts.push(Self::fact(t, "Model", fmt::model(m)));
        }
        let mut sizes = Vec::new();
        if let Some(c) = r.ctx.filter(|c| *c > 0.) {
            sizes.push(format!("{} context", fmt::k(c as u64)));
        }
        if let Some(b) = r.size.filter(|b| *b > 0.) {
            sizes.push(format!("{} transcript", size_text(b)));
        }
        if !sizes.is_empty() {
            facts.push(Self::fact(t, "Size", sizes.join(", ")));
        }
        if let Some(st) = r.started {
            facts.push(Self::fact(t, "Started", format!("{} {}", fmt::md(st), fmt::time(st))));
        }

        // Resume, Start fresh and Open terminal, each one click, with what it does under it.
        let r_res = r.clone();
        let r_term = r.clone();
        let r_fresh = r.clone();
        let fresh_busy = app.busy.contains(&format!("{k}:fresh"));
        let big = |id: &'static str, ic: &'static str, title: &str, note: String, primary: bool| {
            let b = if primary { ui::primary_btn(t, id, 54.) } else { ui::line_btn(t, id, 54.) };
            let (ink, sub) = if primary { (t.on_accent, t.on_accent.alpha(0.82)) } else { (t.ink, t.dim) };
            // Grows taller rather than letting a long note run into the next button.
            b.flex_1()
                .min_w(px(110.))
                .h_auto()
                .min_h(px(54.))
                .py(px(8.))
                .px(px(12.))
                .justify_start()
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(2.))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(6.))
                                .text_size(px(13.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(ink)
                                .child(if ic == "fresh" && fresh_busy { ui::spin_icon("br-fresh-spin", ic, 14., ink) } else { ui::icon(ic, 14., ink).into_any_element() })
                                .child(title.to_string()),
                        )
                        .child(div().w_full().text_size(px(11.5)).line_height(relative(1.3)).text_color(sub).child(note)),
                )
        };
        let ctx_note = r.ctx.filter(|c| *c > 0.).map(|c| format!("Re-reads {}", fmt::k(c as u64))).unwrap_or_else(|| "The same chat".into());
        let mut acts = div().flex().flex_wrap().gap(px(8.));
        acts = acts.child(
            big("br-resume", "resume", if busy { "Resuming" } else if alive { "Open terminal" } else { "Resume" }, if alive { "It is running".into() } else { ctx_note }, true)
                .when(busy, |d| d.opacity(0.6))
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    let r = r_res.clone();
                    if alive { this.terminal(&r, cx) } else { this.resume(&r, cx) }
                })),
        );
        acts = acts.child(
            big("br-fresh", "fresh", if fresh_busy { "Starting" } else { "Start fresh" }, "New chat from a handoff".into(), false)
                .when(fresh_busy, |d| d.opacity(0.6))
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    let r = r_fresh.clone();
                    let (m, sid, title) = (r.machine.clone(), r.sid.clone(), r.title.clone());
                    this.app.update(cx, |s, cx| s.fresh_chat(&m, &sid, title, cx));
                })),
        );
        if !alive {
            acts = acts.child(big("br-term", "term", "Open terminal", "Resumes it there".into(), false).on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                let r = r_term.clone();
                this.terminal(&r, cx)
            })));
        }
        let mut more = div().flex().items_center().gap(px(6.));
        more = more.child(match url {
            Some(u) => ui::ghost(t, "br-open", None, None)
                .h(px(28.))
                .px(px(8.))
                .text_size(px(12.5))
                .child(ui::icon("out", 13., t.dim))
                .child("Open in Claude")
                .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| open_link(cx, &u)))
                .into_any_element(),
            None => div().into_any_element(),
        });
        let sid = r.sid.clone();
        more = more.child(ui::ghost(t, "br-copy", None, None).h(px(28.)).px(px(8.)).text_size(px(12.5)).child(ui::icon("copy", 13., t.dim)).child("Copy its id").on_click(cx.listener(
            move |this, _: &ClickEvent, _, cx| {
                copy_text(cx, sid.clone());
                this.app.update(cx, |s, cx| s.toast("Copied the chat's id", None, false, cx));
            },
        )));

        let mut body = div().flex().flex_col().gap(px(16.)).child(head).child(acts).child(more).child(div().flex().flex_col().gap(px(6.)).children(facts));
        match self.sums.get(&k) {
            Some(Ok(sum)) => {
                if let Some(first) = sum.first.as_ref().and_then(|f| f.text.clone()).filter(|x| !x.trim().is_empty()) {
                    body = body.child(Self::said(t, "First message", &first, sum.first.as_ref().and_then(|f| f.ts).map(|ts| fmt::ago(ts, now))));
                }
                let recent: Vec<_> = sum.recent.iter().filter(|x| x.text.as_deref().is_some_and(|s| !s.trim().is_empty())).collect();
                for (i, x) in recent.iter().rev().take(3).rev().enumerate() {
                    let label = if i == 0 { "Lately" } else { "" };
                    body = body.child(Self::said(t, label, x.text.as_deref().unwrap_or(""), x.ts.map(|ts| fmt::ago(ts, now))));
                }
                if let Some(a) = sum.answer.clone().filter(|x| !x.trim().is_empty()) {
                    body = body.child(Self::said(t, "Claude's last answer", &a, None));
                }
            }
            Some(Err(e)) => body = body.child(div().text_size(px(12.5)).text_color(t.faint).child(format!("Could not read the chat on {label}. {e}"))),
            None => {
                body = body.child(div().text_size(px(12.5)).text_color(t.faint).child(match (&r.first_prompt, &r.last_prompt) {
                    (Some(f), _) if !f.is_empty() => format!("First: {f}"),
                    (_, Some(l)) if !l.is_empty() => format!("Lately: {l}"),
                    _ => format!("Reading it on {label}..."),
                }))
            }
        }
        // Its written summary, when there is one.
        if r.handoff == Some(true) {
            let note = match self.notes.get(&k) {
                Some(Ok(text)) => ui::wrap(text.clone()).text_size(px(12.5)).line_height(relative(1.5)).text_color(t.ink2),
                Some(Err(e)) => ui::wrap(format!("Could not read the summary on {label}. {e}")).text_size(px(12.5)).text_color(t.faint),
                None => ui::wrap(format!("Reading the summary on {label}...")).text_size(px(12.5)).text_color(t.faint),
            };
            body = body.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .child(div().flex().items_center().gap(px(6.)).child(ui::icon("handoff", 13., t.faint)).child(ui::lbl("Summary", t.faint)))
                    .child(div().p(px(12.)).rounded(px(ui::R_CARD)).bg(t.bg).child(note)),
            );
        }
        // The thread: compact it, and every earlier chat, newest first.
        let head_row = self.head.as_ref().and_then(|h| self.rows.iter().find(|x| key(&x.machine, &x.sid) == *h)).cloned();
        if let Some(head) = head_row.filter(|h| !h.history.is_empty()) {
            body = body.child(self.compact_card(t, &head, cx));
            let hk = key(&head.machine, &head.sid);
            let n = head.history_count.unwrap_or(head.history.len() as u64);
            let mut list = div()
                .flex()
                .flex_col()
                .gap(px(8.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .child(ui::icon("history", 13., t.faint))
                        .child(ui::lbl("Additional history", t.faint))
                        .child(div().text_size(px(12.)).text_color(t.faint).child(format!("{n} earlier {}", if n == 1 { "chat" } else { "chats" }))),
                );
            if k != hk {
                list = list.child(self.older_card(t, &head, "Newest", &k, cx));
            }
            for o in &head.history {
                list = list.child(self.older_card(t, o, "", &k, cx));
            }
            if (head.history.len() as u64) < n {
                list = list.child(ui::wrap(format!("{} older chats are not shown here. Search for them by name.", n - head.history.len() as u64)).text_size(px(12.)).text_color(t.faint));
            }
            body = body.child(list);
        }
        div().id("br-side").size_full().overflow_y_scroll().track_scroll(&self.side).px(px(20.)).py(px(18.)).child(body).into_any_element()
    }
}

impl Render for BrowserView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body = self.body(window, cx);
        crate::frame::framed(window, &Theme::get(cx), "Sessions", body)
    }
}

impl BrowserView {
    fn body(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = Theme::get(cx);
        let now = AppState::now();
        let q_focused = self.q.focus_handle(cx).is_focused(window);
        let rn_focused = self.rn.focus_handle(cx).is_focused(window);
        let well = if t.dark { t.bg2 } else { t.bg };

        let machines: Vec<(String, String)> = self.app.read(cx).snap.machines.iter().map(|m| (m.name.clone(), m.label.clone())).collect();
        let mut mseg = ui::seg_box(&t).child(ui::seg_btn(&t, "br-m-all", self.machine.is_empty(), false).child("All machines").on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.set_filter(Some(String::new()), None, cx))));
        for (name, label) in machines {
            let on = self.machine == name;
            mseg = mseg.child(
                ui::seg_btn(&t, SharedString::from(format!("br-m-{name}")), on, false)
                    .child(label)
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.set_filter(Some(name.clone()), None, cx))),
            );
        }
        let mut sseg = ui::seg_box(&t);
        for (v, label) in STATES {
            sseg = sseg.child(ui::seg_btn(&t, SharedString::from(format!("br-s-{v}")), self.state == v, false).child(label).on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.set_filter(None, Some(v), cx))));
        }
        let threads = self.threads;
        let (hover, accent, line2, on_accent) = (t.hover, t.accent, t.line2, t.on_accent);
        let thread_btn = div()
            .id("br-threads")
            .flex_none()
            .h(px(28.))
            .px(px(8.))
            .flex()
            .items_center()
            .gap(px(7.))
            .rounded(px(ui::R_CTRL))
            .cursor_pointer()
            .hover(move |s| s.bg(hover))
            .text_size(px(12.5))
            .text_color(if threads { t.ink } else { t.dim })
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.threads = !this.threads;
                this.load(false, cx);
            }))
            .child(
                div()
                    .flex_none()
                    .size(px(14.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(ui::R_MIN))
                    .when(threads, |d| d.bg(accent))
                    .when(!threads, |d| d.shadow(vec![Theme::ring(line2)]))
                    .when(threads, |d| d.child(ui::icon("check", 10., on_accent))),
            )
            .child("One row per thread");

        let search = div()
            .flex_1()
            .min_w(px(220.))
            .max_w(px(420.))
            .h(px(34.))
            .px(px(11.))
            .flex()
            .items_center()
            .gap(px(8.))
            .rounded(px(ui::R_CARD))
            .border_1()
            .border_color(if q_focused { t.accent } else { t.line })
            .when(q_focused, |d| d.shadow(vec![Theme::ring(t.accent)]))
            .bg(well)
            .text_size(px(13.))
            .text_color(t.ink)
            .child(ui::icon("search", 14., t.faint))
            .child(div().flex_1().min_w_0().child(gpui_base::Input::new(&self.q)));

        // The folder filter: every folder that has chats, in a menu under its button.
        let folder_label = if self.folder.is_empty() { "Every folder".to_string() } else { self.folder.clone() };
        let folder_btn = ui::line_btn(&t, "br-folder", 28.)
            .px(px(10.))
            .text_size(px(12.5))
            .when(self.folder_open, |d| d.bg(t.press))
            .child(ui::icon("folder", 13., t.dim))
            .child(folder_label)
            .child(ui::icon("down", 11., t.faint))
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.folder_open = !this.folder_open;
                cx.notify();
            }));
        let folder_menu = self.folder_open.then(|| {
            let hover = t.hover;
            let mut menu = div()
                .id("br-folders")
                .absolute()
                .top(px(96.))
                .left(px(200.))
                .w(px(320.))
                .max_h(px(360.))
                .overflow_y_scroll()
                .p(px(6.))
                .rounded(px(ui::R_POP))
                .border_1()
                .border_color(t.line2)
                .bg(t.panel2)
                .shadow(t.drop())
                .occlude();
            for f in std::iter::once(String::new()).chain(self.folders.iter().cloned()) {
                let on = self.folder == f;
                let label = if f.is_empty() { "Every folder".to_string() } else { f.clone() };
                menu = menu.child(
                    div()
                        .id(SharedString::from(format!("br-f-{f}")))
                        .min_h(px(30.))
                        .px(px(8.))
                        .py(px(5.))
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .rounded(px(ui::R_ITEM))
                        .cursor_pointer()
                        .hover(move |s| s.bg(hover))
                        .text_size(px(13.))
                        .text_color(if on { t.ink } else { t.ink2 })
                        .child(ui::wrap(label).flex_1())
                        .when(on, |d| d.child(ui::icon("check", 13., t.accent)))
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.folder = f.clone();
                            this.folder_open = false;
                            this.load(false, cx);
                        })),
                );
            }
            menu
        });
        let count = if self.loading && self.rows.is_empty() {
            "Looking...".to_string()
        } else {
            format!("{} {}", self.total, if self.total == 1 { "chat" } else if self.threads { "threads" } else { "chats" })
        };

        let mut list = div().id("br-list").flex_1().min_h_0().overflow_y_scroll().track_scroll(&self.list).px(px(10.)).pb(px(16.));
        if let Some(e) = &self.error {
            list = list.child(div().px(px(12.)).py(px(18.)).text_size(px(13.)).text_color(t.bad).child(format!("Could not load the chats. {e}")));
        } else if self.rows.is_empty() && !self.loading {
            list = list.child(div().px(px(12.)).py(px(18.)).text_size(px(13.)).text_color(t.faint).child("No chats match."));
        }
        let mut group = String::new();
        for r in &self.rows {
            let g = fmt::day_heading(r.last.unwrap_or(0.), now);
            if g != group {
                list = list.child(div().pt(px(16.)).pb(px(6.)).px(px(12.)).text_size(px(12.5)).font_weight(FontWeight::SEMIBOLD).text_color(t.dim).child(g.clone()));
                group = g;
            }
            list = list.child(self.row(&t, r, now, cx));
        }
        if (self.rows.len() as u64) < self.total {
            list = list.child(
                div().pt(px(10.)).flex().justify_center().child(
                    ui::line_btn(&t, "br-more", 30.)
                        .px(px(12.))
                        .text_size(px(12.5))
                        .child(if self.loading { "Loading".to_string() } else { format!("Show more ({} left)", self.total - self.rows.len() as u64) })
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.load(true, cx))),
                ),
            );
        }

        div()
            .id("browser")
            .relative()
            .key_context("Browser")
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .flex_col()
            .bg(t.bg)
            .font_family(crate::theme::UI)
            .text_color(t.ink)
            .child(
                div()
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
                            .child(div().flex_none().text_size(px(17.)).font_weight(FontWeight::BOLD).child(if self.embedded { "History" } else { "Sessions" }))
                            .child(search)
                            .child(div().flex_1())
                            .child(div().flex_none().text_size(px(12.5)).text_color(t.faint).child(count))
                            .child(ui::ghost(&t, "br-reload", Some("Refresh"), Some("update")).h(px(28.)).px(px(8.)).text_size(px(12.5)).on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.load(false, cx)))),
                    )
                    .child(div().flex().flex_wrap().items_center().gap(px(10.)).child(mseg).child(folder_btn).child(sseg).child(thread_btn)),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .child(div().flex_1().min_w_0().flex().flex_col().child(list))
                    .child(div().flex_none().w(px(if self.embedded { 460. } else { 400. })).h_full().min_h_0().border_l_1().border_color(t.rule).bg(t.panel).child(self.detail(&t, now, rn_focused, cx))),
            )
            .children(folder_menu)
    }
}
