//! "This continues..." (spec section 3): pick the older chat a chat carries on from. They share one history, and
//! only the newer one stays on the list. Typing searches names and folders (250 ms after the last key); the arrows
//! move, Enter links.

use std::time::Duration;

use gpui::{
    AnyElement, ClickEvent, Context, Entity, EventEmitter, Focusable, FontWeight, InteractiveElement, IntoElement, ParentElement, Render,
    ScrollHandle, StatefulInteractiveElement, Styled, Subscription, Window, div, prelude::*, px, relative,
};
use gpui_base::input::{InputEvent, InputState};
use sb_core::api::ApiChatPick;
use sb_core::fmt;
use sb_core::names::{self, TagKind};

use crate::state::AppState;
use crate::theme::Theme;
use crate::ui;
use crate::view::{Activate, PalNext, PalPrev};

/// A title's first line (a few chats are named after a long first message).
pub fn first_line<'a>(title: Option<&'a str>, or: &'a str) -> &'a str {
    title.and_then(|t| t.lines().map(str::trim).find(|l| !l.is_empty())).unwrap_or(or)
}

pub enum LinkEvent {
    Close,
}

pub struct LinkSheet {
    app: Entity<AppState>,
    /// The chat that carries on ("machine:sid").
    pub key: String,
    machine: String,
    sid: String,
    topic: String,
    tag: Option<String>,
    kind: Option<TagKind>,
    pub q: Entity<InputState>,
    /// None until the first answer.
    rows: Option<Vec<ApiChatPick>>,
    /// The search the rows answer.
    shown: String,
    failed: Option<String>,
    seq: u64,
    idx: usize,
    /// The row being linked.
    sending: Option<String>,
    scroll: ScrollHandle,
    _subs: Vec<Subscription>,
}

impl EventEmitter<LinkEvent> for LinkSheet {}

impl LinkSheet {
    pub fn new(app: Entity<AppState>, key: String, window: &mut Window, cx: &mut Context<Self>) -> LinkSheet {
        let q = cx.new(|cx| InputState::new(window, cx).placeholder("Search names and folders"));
        let subs = vec![cx.subscribe(&q, |this, _, ev: &InputEvent, cx| {
            if let InputEvent::Change = ev {
                this.search(false, cx);
            }
        })];
        let st = app.read(cx);
        let (machine, sid, raw) = match st.snap.session(&key) {
            Some(s) => (s.machine.clone(), s.sid.clone(), s.title.clone()),
            None => {
                let (m, s) = key.split_once(':').unwrap_or((key.as_str(), ""));
                (m.to_string(), s.to_string(), String::new())
            }
        };
        let (topic, tag) = names::split(first_line(Some(raw.as_str()), "Untitled session"));
        let kind = names::tag_kind(tag.as_deref());
        let mut s = LinkSheet {
            app,
            key,
            machine,
            sid,
            topic,
            tag,
            kind,
            q,
            rows: None,
            shown: String::new(),
            failed: None,
            seq: 0,
            idx: 0,
            sending: None,
            scroll: ScrollHandle::new(),
            _subs: subs,
        };
        s.search(true, cx);
        s
    }

    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.q.update(cx, |q, cx| q.focus(window, cx));
    }

    /// Fills the search box and searches at once (setting the value does not count as typing).
    pub fn set_query(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.q.update(cx, |q, cx| q.set_value(text, window, cx));
        self.search(true, cx);
    }

    /// Asks the server for older chats: at once when `now`, else 250 ms after the last key.
    fn search(&mut self, now: bool, cx: &mut Context<Self>) {
        let q = self.q.read(cx).value().trim().to_string();
        if !now && self.rows.is_some() && q == self.shown {
            return;
        }
        self.seq += 1;
        let seq = self.seq;
        let core = self.app.read(cx).core.clone();
        let (m, s) = (self.machine.clone(), self.sid.clone());
        cx.spawn(async move |this, cx| {
            if !now {
                cx.background_executor().timer(Duration::from_millis(250)).await;
            }
            if this.update(cx, |me, _| me.seq != seq).unwrap_or(true) {
                return;
            }
            let q2 = q.clone();
            let res = cx.background_executor().spawn(async move { core.chats(&q2, &m, &s) }).await;
            let _ = this.update(cx, |me, cx| {
                if me.seq != seq {
                    return;
                }
                match res {
                    Ok(rows) => {
                        me.rows = Some(rows);
                        me.failed = None;
                    }
                    Err(e) => me.failed = Some(e.to_string()),
                }
                me.shown = q;
                me.idx = 0;
                me.scroll.scroll_to_item(0);
                cx.notify();
            });
        })
        .detach();
    }

    /// The chat it continues now, if any ("machine:sid"), from its thread (which loads in the background).
    fn parent(&self, cx: &gpui::App) -> Option<String> {
        self.app.read(cx).threads.get(&self.key).and_then(|t| t.parent()).map(|p| format!("{}:{}", p.machine, p.sid))
    }

    fn step(&mut self, d: i32, cx: &mut Context<Self>) {
        let n = self.rows.as_ref().map(|r| r.len()).unwrap_or(0);
        if n == 0 {
            return;
        }
        self.idx = (self.idx as i32 + d).rem_euclid(n as i32) as usize;
        self.scroll.scroll_to_item(self.idx);
        cx.notify();
    }

    fn pick(&mut self, i: usize, cx: &mut Context<Self>) {
        if self.sending.is_some() {
            return;
        }
        let Some(c) = self.rows.as_ref().and_then(|r| r.get(i)).cloned() else { return };
        let to = format!("{}:{}", c.machine, c.sid);
        if self.parent(cx).as_deref() == Some(to.as_str()) {
            // Already the one it continues: nothing to change.
            return cx.emit(LinkEvent::Close);
        }
        self.sending = Some(to);
        self.idx = i;
        cx.notify();
        let title = names::split(first_line(c.title.as_deref(), "that chat")).0;
        let me = cx.entity().downgrade();
        let key = self.key.clone();
        self.app.update(cx, |s, cx| {
            s.link(
                &key,
                c.machine.clone(),
                c.sid.clone(),
                title,
                move |res, cx| {
                    let _ = me.update(cx, |me, cx| {
                        me.sending = None;
                        if res.is_ok() {
                            cx.emit(LinkEvent::Close);
                        }
                        cx.notify();
                    });
                },
                cx,
            )
        });
    }

    fn row(&self, t: &Theme, i: usize, c: &ApiChatPick, parent: Option<&str>, cx: &Context<Self>) -> AnyElement {
        let on = i == self.idx;
        let key = format!("{}:{}", c.machine, c.sid);
        let current = parent == Some(key.as_str());
        let sending = self.sending.as_deref() == Some(key.as_str());
        let (topic, tag) = names::split(first_line(c.title.as_deref(), "Untitled session"));
        let kind = names::tag_kind(tag.as_deref());
        let label = c.label.clone().or_else(|| self.app.read(cx).snap.label_of(&c.machine)).unwrap_or_else(|| c.machine.clone());
        let sub = [
            Some(label),
            c.folder.clone().filter(|f| !f.is_empty()),
            c.ended.filter(|v| *v > 0.0).map(|v| format!("last used {}", fmt::md(v))),
            c.ctx.filter(|v| *v > 0.0).map(|v| fmt::k(v as u64)),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ");
        let note: Option<(&str, gpui::Hsla)> = if current {
            Some(("it continues this now", t.accent))
        } else if c.in_thread == Some(true) {
            Some(("already in its history", t.faint))
        } else {
            None
        };
        div()
            .id(("link-row", i))
            .relative()
            .px(px(12.))
            .py(px(8.))
            .flex()
            .items_center()
            .gap(px(12.))
            .rounded(px(ui::R_CTRL))
            .cursor_pointer()
            .when(on, |d| d.bg(t.sel))
            .on_mouse_move(cx.listener(move |this, _: &gpui::MouseMoveEvent, _, cx| {
                if this.idx != i {
                    this.idx = i;
                    cx.notify();
                }
            }))
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.pick(i, cx)))
            .when(current, |d| d.child(div().absolute().left(px(3.)).top(px(10.)).bottom(px(10.)).w(px(2.)).rounded_full().bg(t.accent)))
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
                            .child(ui::wrap(topic).flex_shrink(1.).text_size(px(13.)).font_weight(FontWeight::BOLD).text_color(if on { t.ink } else { t.ink2 }))
                            .when_some(tag, |d, tg| d.child(ui::name_tag(t, &tg, kind))),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .min_w_0()
                            .text_size(px(12.))
                            .text_color(t.dim)
                            .child(ui::line(sub).flex_shrink(1.))
                            .when_some(note, |d, (n, col)| {
                                d.child(div().flex_none().whitespace_nowrap().text_color(col).when(current, |d| d.font_weight(FontWeight::BOLD)).child(format!(" · {n}")))
                            }),
                    ),
            )
            .child(if sending {
                div().flex_none().text_size(px(12.)).text_color(t.dim).child("Linking").into_any_element()
            } else if current {
                ui::icon("check", 15., t.accent).into_any_element()
            } else {
                div()
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .text_size(px(12.))
                    .font_weight(FontWeight::BOLD)
                    .text_color(t.accent)
                    .when(!on, |d| d.invisible())
                    .child("Link")
                    .child(ui::icon("link", 13., t.accent))
                    .into_any_element()
            })
            .into_any_element()
    }
}

impl Render for LinkSheet {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = Theme::get(cx);
        let h = f32::from(window.viewport_size().height);
        let well = if t.dark { t.bg2 } else { t.bg };
        let focused = self.q.focus_handle(cx).is_focused(window);
        let mut list = div().id("link-list").max_h(px(330f32.min(h - 300.).max(140.))).overflow_y_scroll().track_scroll(&self.scroll).p(px(6.));
        let note = |text: String| div().px(px(12.)).py(px(18.)).text_size(px(13.)).text_color(t.faint).child(text);
        match (&self.rows, &self.failed) {
            (_, Some(e)) => list = list.child(note(format!("Could not load the chats: {e}"))),
            (None, None) => list = list.child(note("Loading...".into())),
            (Some(rows), None) if rows.is_empty() => {
                list = list.child(note(if self.shown.is_empty() { "No older chats to pick from." } else { "No older chat matches that." }.into()))
            }
            (Some(rows), None) => {
                let parent = self.parent(cx);
                for (i, c) in rows.iter().enumerate() {
                    list = list.child(self.row(&t, i, c, parent.as_deref(), cx));
                }
            }
        }
        let n = self.rows.as_ref().map(|r| r.len()).unwrap_or(0);
        let cap = |k: &str| ui::kbd(&t, k);
        let foot = |keys: Vec<&str>, text: &str| div().flex().items_center().gap(px(4.)).children(keys.into_iter().map(cap)).child(text.to_string());
        div()
            .key_context("LinkSheet")
            .on_action(cx.listener(|this, _: &PalNext, _, cx| this.step(1, cx)))
            .on_action(cx.listener(|this, _: &PalPrev, _, cx| this.step(-1, cx)))
            .on_action(cx.listener(|this, _: &Activate, _, cx| this.pick(this.idx, cx)))
            .flex()
            .flex_col()
            .child(
                div()
                    .flex_none()
                    .pt(px(16.))
                    .pr(px(12.))
                    .pb(px(4.))
                    .pl(px(22.))
                    .flex()
                    .items_start()
                    .gap(px(12.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(6.))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(8.))
                                    .child(ui::icon("link", 16., t.accent))
                                    .child(div().text_size(px(15.)).font_weight(FontWeight::BOLD).text_color(t.ink).child("This continues...")),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(6.))
                                    .min_w_0()
                                    .child(ui::wrap(self.topic.clone()).flex_shrink(1.).text_size(px(13.)).text_color(t.ink2))
                                    .when_some(self.tag.clone(), |d, tg| d.child(ui::name_tag(&t, &tg, self.kind))),
                            ),
                    )
                    .child(ui::icon_btn(&t, "lk-close", "x", 26.).on_click(cx.listener(|_, _: &ClickEvent, _, cx| cx.emit(LinkEvent::Close)))),
            )
            .child(
                div()
                    .px(px(22.))
                    .pt(px(6.))
                    .text_size(px(12.5))
                    .line_height(relative(1.5))
                    .text_color(t.dim)
                    .child("Pick the older chat this one carries on from. They share one history, and only the newer one stays on the list."),
            )
            .child(
                div()
                    .px(px(22.))
                    .pt(px(14.))
                    .pb(px(6.))
                    .flex()
                    .flex_col()
                    .gap(px(7.))
                    .child(ui::lbl("Older chat", t.label))
                    .child(
                        div()
                            .h(px(36.))
                            .px(px(12.))
                            .flex()
                            .items_center()
                            .gap(px(9.))
                            .rounded(px(ui::R_CTRL))
                            .border_1()
                            .border_color(if focused { t.accent } else { t.line })
                            .when(focused, |d| d.shadow(vec![Theme::ring(t.accent)]))
                            .bg(well)
                            .child(ui::icon("search", 15., t.faint))
                            .child(div().flex_1().min_w_0().h_full().flex().items_center().text_size(px(13.5)).text_color(t.ink).child(gpui_base::input::Input::new(&self.q))),
                    ),
            )
            .child(div().px(px(16.)).pb(px(8.)).child(list))
            .child(
                div()
                    .flex_none()
                    .h(px(32.))
                    .px(px(22.))
                    .flex()
                    .items_center()
                    .gap(px(16.))
                    .border_t_1()
                    .border_color(t.rule)
                    .text_size(px(11.5))
                    .text_color(t.faint)
                    .child(foot(vec!["↑", "↓"], "move"))
                    .child(foot(vec!["↵"], "link"))
                    .child(foot(vec!["esc"], "close"))
                    .child(ui::spacer())
                    .when(n > 0, |d| d.child(ui::num(if n == 1 { "1 chat".to_string() } else { format!("{n} chats") }, 11.5, t.faint))),
            )
    }
}
