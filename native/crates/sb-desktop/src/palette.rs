//! The command palette (Cmd-K): chats and commands in one list. Typing filters both; the arrows move, Enter runs.
//! Cmd-T opens it with chats only, and Enter opens the chat in a terminal tab.

use gpui::{
    AnyElement, App, ClickEvent, InteractiveElement, IntoElement, MouseButton, ParentElement,
    StatefulInteractiveElement, Styled, Window, div, prelude::*, px, relative,
};
use sb_core::actions::{self as acts, ActionId};
use sb_core::{Lamp, search};

use crate::archive::ArcTab;
use crate::theme::Theme;
use crate::ui;
use crate::view::{BoardView, Ctx};

#[derive(Clone)]
pub enum PalIcon {
    Icon(String),
    Machine(String),
    Lamp(Lamp, bool),
}

#[derive(Clone)]
pub enum PalRun {
    Select(String),
    Older(String),
    New(Option<String>),
    Act(ActionId, String),
    ContinueOn(String, String),
    Shut(String, bool),
    Tear(String),
    Archive(ArcTab),
    Theme(bool),
    Notify(bool),
    /// Open the chat in a terminal tab (resuming it first when it is stopped).
    Term(String, bool),
    /// Switch to the chats-only palette.
    Chats,
    /// v2: the session browser window.
    Browse,
    /// v2: Files and Add a machine windows.
    Files,
    Pair,
}

#[derive(Clone)]
pub struct PalRow {
    pub title: String,
    pub sub: Option<String>,
    pub icon: PalIcon,
    pub key: Option<String>,
    pub run: PalRun,
    score: u32,
    /// A chat in a tab group: its color, as a dot after the name.
    pub group: Option<sb_core::groups::GroupColor>,
}

#[derive(Clone)]
pub enum PalItem {
    Head(&'static str),
    Row(PalRow),
}

fn row(title: impl Into<String>, sub: Option<String>, icon: PalIcon, key: Option<&str>, run: PalRun) -> PalRow {
    PalRow { title: title.into(), sub, icon, key: key.map(str::to_string), run, score: 1, group: None }
}

/// Needs you first, then working; a chat parked for a day or more waits below the working ones.
fn rank(s: &sb_core::Session, now: f64) -> u8 {
    match s.lamp(now) {
        Lamp::Held | Lamp::Wait if s.parked(now) => 2,
        Lamp::Held | Lamp::Wait => 0,
        Lamp::Run | Lamp::Stall => 1,
        Lamp::Warm => 3,
        Lamp::Cold => 4,
        Lamp::Idle => 5,
    }
}

impl BoardView {
    fn commands(&self, cx: &App) -> Vec<PalRow> {
        let app = self.app.read(cx);
        let snap = &app.snap;
        let now = crate::state::AppState::now();
        let mut out = vec![row("New session", None, PalIcon::Icon("plus".into()), Some("N"), PalRun::New(None))];
        out.push(row(
            "Open a chat in a terminal",
            None,
            PalIcon::Icon("term".into()),
            Some(&ui::mod_cap("T")),
            PalRun::Chats,
        ));
        out.push(row(
            "Browse every session",
            Some("Search, threads, summaries".into()),
            PalIcon::Icon("search".into()),
            Some(&ui::mod_cap("B")),
            PalRun::Browse,
        ));
        out.push(row(
            "Files",
            Some("The Claude folder on each machine".into()),
            PalIcon::Icon("folder".into()),
            None,
            PalRun::Files,
        ));
        out.push(row(
            "Add a machine",
            Some("A code and one line to paste".into()),
            PalIcon::Icon("plus".into()),
            None,
            PalRun::Pair,
        ));
        for m in snap.machines.iter().filter(|m| !m.is_offline()) {
            out.push(row(
                format!("New session on {}", m.label),
                None,
                PalIcon::Machine(m.name.clone()),
                None,
                PalRun::New(Some(m.name.clone())),
            ));
        }
        // The terminal in front: Rename chat names the chat it shows (the selected chat's actions follow).
        let in_front = self.tab.checked_sub(1).and_then(|i| self.tabs.get(i)).and_then(|t| t.read(cx).chat_name(cx));
        if let Some((k, raw)) = in_front.filter(|(k, _)| {
            self.sel.as_deref() != Some(k.as_str()) && snap.can_rename(k.split(':').next().unwrap_or(""))
        }) {
            out.push(row(
                "Rename chat",
                Some(sb_core::names::split(&raw).0),
                PalIcon::Icon("pencil".into()),
                None,
                PalRun::Act(ActionId::Rename, k),
            ));
        }
        if let Some(s) = self.sel.as_deref().and_then(|k| snap.session(k)) {
            for a in acts::for_session(s, snap, now) {
                if a.disabled {
                    continue;
                }
                if a.id == ActionId::Continue {
                    for m in snap.other_machines(&s.machine) {
                        out.push(row(
                            format!("Continue on {}", m.label),
                            Some(s.topic.clone()),
                            PalIcon::Icon("cont".into()),
                            None,
                            PalRun::ContinueOn(s.key.clone(), m.name.clone()),
                        ));
                    }
                    continue;
                }
                let cap = match a.key.as_deref() {
                    Some("Enter") => Some("↵"),
                    Some(k @ ("F" | "R" | "C")) => Some(k),
                    _ => None,
                };
                out.push(row(
                    a.title.clone(),
                    Some(s.topic.clone()),
                    PalIcon::Icon(a.icon.clone()),
                    cap,
                    PalRun::Act(a.id, s.key.clone()),
                ));
            }
        }
        for m in snap.machines.iter().filter(|m| self.solo.as_deref().is_none_or(|o| o == m.name)) {
            let shut = app.is_shut(&m.name);
            out.push(row(
                format!("{} the {} lane", if shut { "Show" } else { "Fold" }, m.label),
                None,
                PalIcon::Machine(m.name.clone()),
                None,
                PalRun::Shut(m.name.clone(), !shut),
            ));
            if self.solo.is_none() {
                out.push(row(
                    format!("Open the {} lane in a new window", m.label),
                    None,
                    PalIcon::Icon("window".into()),
                    None,
                    PalRun::Tear(m.name.clone()),
                ));
            }
        }
        out.push(row(
            "Show older chats",
            Some(app.older().len().to_string()),
            PalIcon::Icon("archive".into()),
            None,
            PalRun::Archive(ArcTab::Older),
        ));
        out.push(row(
            "Show replaced chats",
            Some(app.replaced().len().to_string()),
            PalIcon::Icon("archive".into()),
            None,
            PalRun::Archive(ArcTab::Replaced),
        ));
        let dark = Theme::get(cx).dark;
        out.push(row(
            if dark { "Switch to light" } else { "Switch to dark" },
            None,
            PalIcon::Icon(if dark { "sun" } else { "moon" }.into()),
            None,
            PalRun::Theme(!dark),
        ));
        let on = app.notify_on();
        out.push(row(
            if on { "Turn off needs-you notifications" } else { "Turn on needs-you notifications" },
            None,
            PalIcon::Icon("bolt".into()),
            None,
            PalRun::Notify(!on),
        ));
        out
    }

    /// Chats matching `q`. `term`: for the chats-only palette, where Enter opens a terminal.
    fn chat_rows(&self, q: &str, term: bool, cx: &App) -> Vec<PalRow> {
        let app = self.app.read(cx);
        let now = crate::state::AppState::now();
        let mut list: Vec<(u32, &sb_core::Session)> = app
            .snap
            .sessions()
            .into_iter()
            .filter(|s| !app.is_hidden(&s.key) && (!s.stale || !q.is_empty()))
            .filter(|s| self.solo.as_deref().is_none_or(|m| m == s.machine))
            .filter_map(|s| {
                let chips: Vec<String> = s.chips(true).into_iter().map(|c| c.long).collect();
                let group = app.snap.group_of(s).map(|g| g.name.as_str()).unwrap_or("");
                let text = format!(
                    "{} {} {} {} {} {} {group}",
                    s.topic,
                    s.machine_label,
                    s.project,
                    s.folder,
                    s.tag.as_deref().unwrap_or(""),
                    chips.join(" ")
                );
                let sc = search::score(&text, q);
                (sc > 0).then_some((sc, s))
            })
            .collect();
        // Starred chats first; a typed search still ranks by how well it matches, the star then breaking ties.
        let typed = !q.is_empty();
        list.sort_by(|(sa, a), (sb, b)| {
            let star = b.star.cmp(&a.star);
            if typed { sb.cmp(sa).then(star) } else { star.then(sb.cmp(sa)) }
                .then(a.stale.cmp(&b.stale))
                .then(rank(a, now).cmp(&rank(b, now)))
                .then(b.active.total_cmp(&a.active))
        });
        if term {
            // Chats that open right away first.
            list.sort_by_key(|(_, s)| (!s.star || typed, !(s.background() && s.alive)));
        }
        let n = if q.is_empty() && !term { 7 } else { 40 };
        list.into_iter()
            .take(n)
            .map(|(sc, s)| {
                let (sub, run) = if term {
                    let how = if s.background() && s.alive {
                        ""
                    } else if s.alive {
                        ", open on its machine"
                    } else {
                        ", resumes first"
                    };
                    (format!("{}, {}{how}", s.phrase(now), s.machine_label), PalRun::Term(s.key.clone(), !s.alive))
                } else {
                    (
                        format!("{}{}, {}", if s.stale { "Older, " } else { "" }, s.phrase(now), s.machine_label),
                        if s.stale { PalRun::Older(s.key.clone()) } else { PalRun::Select(s.key.clone()) },
                    )
                };
                let mut r = row(
                    s.topic.clone(),
                    Some(sub),
                    PalIcon::Lamp(s.lamp(now), s.stale || app.live_of(s, now, None).stale.is_some()),
                    None,
                    run,
                );
                r.score = sc;
                r.group = app.snap.group_of(s).map(|g| g.color);
                r
            })
            .collect()
    }

    pub fn pal_items(&self, cx: &App) -> Vec<PalItem> {
        let q = self.pal.read(cx).value().trim().to_string();
        if self.pal_chats {
            let chats = self.chat_rows(&q, true, cx);
            if chats.is_empty() {
                return Vec::new();
            }
            let mut out = vec![PalItem::Head("Open in a terminal")];
            out.extend(chats.into_iter().map(PalItem::Row));
            return out;
        }
        let mut cmds: Vec<PalRow> = self
            .commands(cx)
            .into_iter()
            .filter_map(|mut c| {
                c.score = search::score(&format!("{} {}", c.title, c.sub.as_deref().unwrap_or("")), &q);
                (c.score > 0).then_some(c)
            })
            .collect();
        if !q.is_empty() {
            cmds.sort_by_key(|c| std::cmp::Reverse(c.score));
        }
        let chats = self.chat_rows(&q, false, cx);
        let top_cmd = cmds.first().map(|c| c.score).unwrap_or(0);
        let mut sections: Vec<(&'static str, Vec<PalRow>)> = if q.is_empty() {
            cmds.truncate(7);
            vec![("Commands", cmds), ("Chats", chats)]
        } else {
            cmds.truncate(14);
            vec![("Chats", chats), ("Commands", cmds)]
        };
        if !q.is_empty() && !sections[1].1.is_empty() && (sections[0].1.is_empty() || top_cmd > 2) {
            sections.reverse();
        }
        let mut out = Vec::new();
        for (name, rows) in sections {
            if rows.is_empty() {
                continue;
            }
            out.push(PalItem::Head(name));
            out.extend(rows.into_iter().map(PalItem::Row));
        }
        out
    }

    pub fn pal_move(&mut self, d: i32, cx: &mut gpui::Context<Self>) {
        let items = self.pal_items(cx);
        let n = items.iter().filter(|i| matches!(i, PalItem::Row(_))).count();
        if n == 0 {
            return;
        }
        self.pal_idx = ((self.pal_idx as i32 + d).rem_euclid(n as i32)) as usize;
        // The scroll list's children include the section heads.
        let mut seen = 0;
        for (ci, it) in items.iter().enumerate() {
            if let PalItem::Row(_) = it {
                if seen == self.pal_idx {
                    self.pal_scroll.scroll_to_item(ci, gpui::ScrollStrategy::Nearest);
                    break;
                }
                seen += 1;
            }
        }
        cx.notify();
    }

    pub fn pal_run(&mut self, idx: usize, window: &mut Window, cx: &mut gpui::Context<Self>) {
        let run = self
            .pal_items(cx)
            .into_iter()
            .filter_map(|i| match i {
                PalItem::Row(r) => Some(r.run),
                PalItem::Head(_) => None,
            })
            .nth(idx);
        self.close_overlay(window, cx);
        let Some(run) = run else { return };
        match run {
            PalRun::Select(k) => self.select(Some(k), cx),
            PalRun::Older(k) => self.open_archive(ArcTab::Older, Some(k), window, cx),
            PalRun::New(m) => self.open_new(m, window, cx),
            PalRun::Act(id, k) => {
                if id == ActionId::Open {
                    self.app.update(cx, |s, cx| s.open(&k, cx));
                } else if id == ActionId::Rename {
                    self.rename_chat(k, window, cx);
                } else {
                    self.run_action(id, k, cx);
                }
            }
            PalRun::ContinueOn(k, to) => self.app.update(cx, |s, cx| s.continue_on(&k, &to, cx)),
            PalRun::Shut(m, v) => self.app.update(cx, |s, cx| s.set_shut(&m, v, cx)),
            PalRun::Tear(m) => self.tear(m, cx),
            PalRun::Archive(tab) => self.open_archive(tab, None, window, cx),
            PalRun::Theme(dark) => self.set_theme(dark, cx),
            PalRun::Term(k, resume) => self.open_terminal(k, resume, cx),
            PalRun::Chats => self.open_chat_palette(window, cx),
            PalRun::Browse => crate::browser::open(cx),
            PalRun::Files => crate::files::open_files(cx),
            PalRun::Pair => crate::files::open_pair(cx),
            PalRun::Notify(on) => self.app.update(cx, |s, cx| {
                s.settings.notify = Some(on);
                s.save_settings();
                let msg = if on { "Needs-you notifications are on." } else { "Needs-you notifications are off." };
                s.toast(msg, None, false, cx);
            }),
        }
    }

    pub fn palette(&self, c: &Ctx, w: f32, h: f32) -> AnyElement {
        let t = c.t;
        let cx = c.cx;
        let items = self.pal_items(cx);
        let q = self.pal.read(cx).value().trim().to_string();
        let pw = 600f32.min(w - 32.);
        // Only the rows on screen are drawn: a uniform list (heads and rows are both 34 px), sized to its rows up to
        // the cap, so a long chat list opens and scrolls as fast as a short one.
        let any = items.iter().any(|i| matches!(i, PalItem::Row(_)));
        let max_h = 374f32.min(h - 220.).max(120.);
        let list = if any {
            let n = items.len();
            let list_h = (n as f32 * PAL_ROW + 12.).min(max_h);
            let items = std::rc::Rc::new(items);
            let list = gpui::uniform_list(
                "pal-list",
                n,
                cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
                    let t = Theme::get(cx);
                    let mut ri = items[..range.start].iter().filter(|i| matches!(i, PalItem::Row(_))).count();
                    let mut out = Vec::with_capacity(range.len());
                    for it in &items[range] {
                        out.push(match it {
                            PalItem::Head(name) => div()
                                .w_full()
                                .h(px(PAL_ROW))
                                .flex()
                                .items_end()
                                .px(px(10.))
                                .pb(px(6.))
                                .child(ui::lbl(name, t.label))
                                .into_any_element(),
                            PalItem::Row(r) => {
                                let i = ri;
                                ri += 1;
                                pal_row(&t, r, i, i == this.pal_idx, cx)
                            }
                        });
                    }
                    out
                }),
            )
            .track_scroll(&self.pal_scroll)
            .h(px(list_h))
            .p(px(6.));
            div().relative().child(list).child(ui::scrollbar(&self.pal_scroll)).into_any_element()
        } else {
            div()
                .px(px(18.))
                .py(px(24.))
                .text_size(px(13.))
                .text_color(t.faint)
                .child(format!("Nothing matches \"{q}\"."))
                .into_any_element()
        };
        let cap = |k: &str| ui::kbd(&t, k);
        let foot = |keys: Vec<&str>, text: &str| {
            div().flex().items_center().gap(px(4.)).children(keys.into_iter().map(cap)).child(text.to_string())
        };
        div()
            .absolute()
            .inset_0()
            .occlude()
            .on_mouse_down(MouseButton::Left, cx.listener(|this, _, window, cx| this.close_overlay(window, cx)))
            .child(
                div()
                    .id("palette")
                    .absolute()
                    .top(px(88.))
                    .left(px(((w - pw) / 2.).max(16.)))
                    .w(px(pw))
                    .flex()
                    .flex_col()
                    .rounded(px(ui::R_POP))
                    .border_1()
                    .border_color(t.line2)
                    .bg(t.panel2)
                    .shadow(t.drop())
                    .overflow_hidden()
                    .occlude()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .key_context("Palette")
                    .child(
                        div()
                            .flex_none()
                            .h(px(48.))
                            .pl(px(16.))
                            .pr(px(12.))
                            .flex()
                            .items_center()
                            .gap(px(10.))
                            .border_b_1()
                            .border_color(t.rule)
                            .child(ui::icon("search", 16., t.faint))
                            .child(
                                div()
                                    .flex_1()
                                    .h_full()
                                    .flex()
                                    .items_center()
                                    .text_size(px(15.))
                                    .text_color(t.ink)
                                    .child(gpui_base::input::Input::new(&self.pal)),
                            )
                            .child(ui::kbd(&t, "esc")),
                    )
                    .child(list)
                    .child(
                        div()
                            .flex_none()
                            .h(px(30.))
                            .px(px(14.))
                            .flex()
                            .items_center()
                            .gap(px(16.))
                            .border_t_1()
                            .border_color(t.rule)
                            .text_size(px(11.5))
                            .text_color(t.faint)
                            .child(foot(vec!["↑", "↓"], "move"))
                            .child(foot(vec!["↵"], if self.pal_chats { "open" } else { "run" }))
                            .child(foot(vec!["esc"], "close"))
                            .child(ui::spacer())
                            .child(if self.pal_chats { "Chats open in a terminal tab" } else { "Chats and commands" }),
                    ),
            )
            .into_any_element()
    }
}

/// Each palette line (a section head or a row) is this tall, so the list can draw only what is on screen.
const PAL_ROW: f32 = 34.;

/// One palette row: its mark, its name (and the status after it), and its shortcut.
fn pal_row(t: &Theme, r: &PalRow, i: usize, on: bool, cx: &mut gpui::Context<BoardView>) -> AnyElement {
    let icon: AnyElement = match &r.icon {
        PalIcon::Icon(n) => ui::icon(n, 15., if on { t.ink2 } else { t.faint }).into_any_element(),
        PalIcon::Machine(m) => ui::machine_icon(m, 15., if on { t.ink2 } else { t.faint }).into_any_element(),
        PalIcon::Lamp(l, stale) => {
            let d = if *stale {
                div().size(px(8.)).rounded_full().border_1().border_color(t.lamp(*l))
            } else {
                ui::lamp(t, *l, 8.)
            };
            div().flex_none().w(px(15.)).flex().justify_center().child(d).into_any_element()
        }
    };
    div()
        .id(("pal-row", i))
        .w_full()
        .h(px(PAL_ROW))
        .px(px(10.))
        .flex()
        .items_center()
        .gap(px(10.))
        .rounded(px(ui::R_ITEM))
        .cursor_pointer()
        .text_size(px(13.))
        .text_color(if on { t.ink } else { t.ink2 })
        .when(on, |d| d.bg(t.sel))
        .on_mouse_move(cx.listener(move |this, _: &gpui::MouseMoveEvent, _, cx| {
            if this.pal_idx != i {
                this.pal_idx = i;
                cx.notify();
            }
        }))
        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| this.pal_run(i, window, cx)))
        .overflow_hidden()
        .child(icon)
        // one line each: the name keeps at least half the row, a long status ("Needs: ...") ends in an ellipsis
        // instead of pushing the name into a column of letters
        .child(div().flex_1().min_w(relative(0.5)).truncate().child(r.title.clone()))
        .when_some(r.group, |d, gc| d.child(div().flex_none().child(crate::tabgroups::group_dot(t, gc, 7.))))
        .when_some(r.sub.clone(), |d, s| {
            d.child(
                div()
                    .flex_shrink(1.)
                    .min_w_0()
                    .max_w(relative(0.45))
                    .pl(px(12.))
                    .text_size(px(12.))
                    .text_color(t.faint)
                    .truncate()
                    .child(s),
            )
        })
        .when_some(r.key.clone(), |d, k| d.child(div().flex_none().ml(px(8.)).child(ui::kbd(t, &k))))
        .into_any_element()
}
