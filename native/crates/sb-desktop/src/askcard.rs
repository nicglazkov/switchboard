//! "Needs you", shaped by kind (docs/specs/2026-10-13-needs-shaped-replies.md, direction C). The details sheet holds
//! the full card at the top: the kind's tag and Claude's summary, then option buttons (choose), a small form
//! (questions), a checklist and Done (task), the fix as one button (fix), or a free reply box (open). Send turns the
//! controls into one message (sb_core::ask) and types it into the chat's terminal with Enter. The board card shows the
//! summary and one button (`card_quick`). Every text sits at min width 0 and wraps; controls never shrink; chips wrap.

use gpui::{
    AnyElement, AppContext, ClickEvent, Context, Entity, FontWeight, Hsla, InteractiveElement, IntoElement,
    ParentElement, SharedString, StatefulInteractiveElement, Styled, Subscription, Window, WindowId, div, prelude::*,
    px, relative,
};
use gpui_base::input::{InputEvent, InputState, TextareaState};
use sb_core::Session;
use sb_core::actions::Confirm;
use sb_core::ask::{self, Ask, AskKind, Quick};
use sb_core::fmt;

use crate::state::AppState;
use crate::theme::Theme;
use crate::ui;
use crate::view::{BoardView, Ctx};

/// The card's controls for one chat's ask: what is picked, ticked and typed. Remade when the ask changes (a new
/// transcript) or the details move to another window (text fields belong to a window; what was typed carries over).
pub struct AskForm {
    pub key: String,
    sig: String,
    win: WindowId,
    pub pick: usize,
    pub ticked: Vec<bool>,
    pub answers: Vec<Vec<usize>>,
    pub steps: Vec<bool>,
    /// "Hit a problem...": the free reply box is open under a task.
    pub free_open: bool,
    /// What was last sent from the card (or what happened to the fix), shown under it.
    pub sent: Option<String>,
    pub note: Entity<InputState>,
    pub typed: Vec<Entity<InputState>>,
    pub free: Entity<TextareaState>,
    _subs: Vec<Subscription>,
}

fn sig(a: &Ask) -> String {
    format!("{:?}|{}|{}|{}|{}", a.kind, a.at, a.summary, a.options.len(), a.questions.len())
}

/// The line under the card once Send is done with: what went in, or that the terminal never opened for it.
fn sent_note(text: &str, ok: bool) -> String {
    if ok { format!("Sent: {text}") } else { "Not sent, its terminal didn't open. Try Send again.".into() }
}

/// A reply from the card to chat `key` sets off, unless one is still on its way there.
fn start_send(sending: &mut std::collections::HashSet<String>, key: &str) -> bool {
    sending.insert(key.to_string())
}

/// The kind's tag: a soft accent pill.
pub fn kind_tag(t: &Theme, k: AskKind) -> gpui::Div {
    div()
        .flex_none()
        .px(px(7.))
        .h(px(18.))
        .flex()
        .items_center()
        .rounded(px(9.))
        .bg(t.accent.alpha(if t.dark { 0.18 } else { 0.11 }))
        .text_size(px(11.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(t.accent)
        .child(k.label())
}

/// A button whose label wraps instead of running out of a narrow card: never wider than its row, at least `h` tall.
fn btn(t: &Theme, id: impl Into<gpui::ElementId>, primary: bool, on: bool) -> gpui::Stateful<gpui::Div> {
    let b = if primary { ui::primary_btn(t, id, 32.) } else { ui::line_btn(t, id, 32.) };
    b.h_auto().min_h(px(32.)).py(px(5.)).max_w_full().text_size(px(13.)).line_height(relative(1.35)).when(
        on && !primary,
        |d| {
            d.bg(t.accent.alpha(if t.dark { 0.2 } else { 0.1 }))
                .shadow(vec![Theme::ring(t.accent)])
                .font_weight(FontWeight::SEMIBOLD)
        },
    )
}

fn label(text: impl Into<SharedString>) -> gpui::Div {
    ui::wrap(text).flex_shrink(1.)
}

/// A checkbox row: the box stays its size, the text wraps beside it.
fn check_row(t: &Theme, id: SharedString, on: bool, text: &str) -> gpui::Stateful<gpui::Div> {
    let hover = t.hover;
    div()
        .id(id)
        .w_full()
        .min_w_0()
        .flex()
        .items_start()
        .gap(px(8.))
        .px(px(4.))
        .py(px(3.))
        .rounded(px(ui::R_ITEM))
        .cursor_pointer()
        .hover(move |s| s.bg(hover))
        .child(
            div()
                .flex_none()
                .mt(px(2.))
                .size(px(14.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(ui::R_MIN))
                .when(on, |d| d.bg(t.accent))
                .when(!on, |d| d.shadow(vec![Theme::ring(t.line2)]))
                .when(on, |d| d.child(ui::icon("check", 10., t.on_accent))),
        )
        .child(ui::wrap(text.to_string()).flex_1().text_size(px(12.5)).line_height(relative(1.45)).text_color(if on {
            t.ink
        } else {
            t.ink2
        }))
}

/// A one-line field (the note, "Or type it...").
fn field(t: &Theme, inp: &Entity<InputState>) -> gpui::Div {
    div()
        .w_full()
        .min_w_0()
        .h(px(30.))
        .px(px(9.))
        .flex()
        .items_center()
        .rounded(px(ui::R_CTRL))
        .border_1()
        .border_color(t.line2)
        .bg(t.bg)
        .text_size(px(12.5))
        .text_color(t.ink)
        .overflow_hidden()
        .child(div().flex_1().min_w_0().child(gpui_base::Input::new(inp)))
}

fn section(t: &Theme) -> gpui::Div {
    div().w_full().min_w_0().pt(px(10.)).border_t_1().border_color(t.line).flex().flex_col().gap(px(6.))
}

impl BoardView {
    /// Makes (or keeps) the card's text fields for the chat the details show in this window. `key` None: this
    /// window shows no details, so the form is left as it is.
    pub fn ask_inputs(&mut self, key: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(key) = key else { return };
        let Some(a) = self.app.read(cx).snap.session(&key).and_then(|s| s.ask_now(AppState::now())) else { return };
        let sig = sig(&a);
        let win = window.window_handle().window_id();
        if self.ask.as_ref().is_some_and(|f| f.key == key && f.sig == sig && f.win == win) {
            return;
        }
        // The same ask in another window: keep what was picked and typed.
        let carry = self.ask.take().filter(|f| f.key == key && f.sig == sig);
        let val = |e: &Entity<InputState>, cx: &Context<Self>| e.read(cx).value().to_string();
        let typed_carry: Vec<String> =
            carry.as_ref().map(|f| f.typed.iter().map(|e| val(e, cx)).collect()).unwrap_or_default();
        let note_carry = carry.as_ref().map(|f| val(&f.note, cx)).unwrap_or_default();
        let free_carry = carry.as_ref().map(|f| f.free.read(cx).value().to_string()).unwrap_or_default();

        let note = cx.new(|cx| InputState::new(window, cx).placeholder("Anything to change? (optional)"));
        let typed: Vec<Entity<InputState>> = a
            .questions
            .iter()
            .map(|q| {
                let ph = if q.multi { "Anything else..." } else { "Or type it..." };
                cx.new(|cx| InputState::new(window, cx).placeholder(ph))
            })
            .collect();
        let ph = if a.kind == AskKind::Task { "What went wrong?" } else { "Type your answer..." };
        let free = cx.new(|cx| TextareaState::new(window, cx).placeholder(ph).auto_grow(2, 6));
        if !note_carry.is_empty() {
            note.update(cx, |n, cx| n.set_value(note_carry, window, cx));
        }
        for (e, v) in typed.iter().zip(typed_carry) {
            if !v.is_empty() {
                e.update(cx, |n, cx| n.set_value(v, window, cx));
            }
        }
        if !free_carry.is_empty() {
            free.update(cx, |n, cx| n.set_value(free_carry, window, cx));
        }
        // Typing in the card turns the board's plain-letter keys off (j, k, f, r... must type there).
        let mut subs = Vec::new();
        for e in std::iter::once(&note).chain(typed.iter()) {
            subs.push(cx.subscribe_in(e, window, |this, _, ev: &InputEvent, _, cx| this.ask_event(ev, cx)));
        }
        subs.push(cx.subscribe_in(&free, window, |this, _, ev: &InputEvent, _, cx| this.ask_event(ev, cx)));
        self.ask_focus = false;
        self.ask = Some(match carry {
            Some(f) => AskForm { win, note, typed, free, _subs: subs, ..f },
            None => AskForm {
                key,
                sig,
                win,
                pick: a.default_pick(),
                ticked: vec![true; a.assumptions.len()],
                answers: a.default_answers(),
                steps: vec![false; a.steps.len()],
                free_open: false,
                sent: None,
                note,
                typed,
                free,
                _subs: subs,
            },
        });
    }

    fn ask_event(&mut self, ev: &InputEvent, cx: &mut Context<Self>) {
        match ev {
            InputEvent::Focus => self.ask_focus = true,
            InputEvent::Blur => self.ask_focus = false,
            _ => {}
        }
        cx.notify();
    }

    fn form_mut(&mut self, key: &str) -> Option<&mut AskForm> {
        self.ask.as_mut().filter(|f| f.key == key)
    }

    /// Types `text` into the chat's terminal (its tab opens) with Enter, and notes it under the card. A click while
    /// one is still on its way does nothing, so the reply goes in once.
    pub fn ask_send(&mut self, key: String, text: String, cx: &mut Context<Self>) {
        if !start_send(&mut self.ask_sending, &key) {
            return;
        }
        if let Some(f) = self.form_mut(&key) {
            f.free_open = false;
            f.sent = Some("Sending...".into());
        }
        self.reply_with(key, text, cx);
        cx.notify();
    }

    /// The reply went into the chat's terminal, or the terminal never opened for it.
    pub fn ask_sent(&mut self, key: &str, text: &str, ok: bool, cx: &mut Context<Self>) {
        self.ask_sending.remove(key);
        if let Some(f) = self.form_mut(key) {
            f.sent = Some(sent_note(text, ok));
        }
        cx.notify();
    }

    /// The board card's one button.
    pub fn ask_quick(&mut self, key: String, q: Quick, cx: &mut Context<Self>) {
        match q {
            Quick::Send(text) => self.ask_send(key, text, cx),
            Quick::Fix => self.ask_fix(key, cx),
            Quick::Answer => self.select(Some(key), cx),
        }
    }

    /// The fix (v1: relaunch with --chrome), after the usual confirm with what it costs.
    pub fn ask_fix(&mut self, key: String, cx: &mut Context<Self>) {
        let now = AppState::now();
        let Some(s) = self.app.read(cx).snap.session(&key) else { return };
        let k = |n: f64| fmt::k(n.max(0.0).round() as u64);
        let cost = match &s.cost {
            Some(c) if s.cache_left(now) > 0 => {
                format!("About {} up front, because the cache is warm.", k(c.resume.warm))
            }
            Some(c) => format!(
                "About {} up front, since the cache is cold and all {} gets read again.",
                k(c.resume.cold),
                s.ctx_label()
            ),
            None if s.cache_left(now) > 0 => "The cache is warm, so it's cheap right now.".to_string(),
            None => format!("It re-reads {} of context.", s.ctx_label()),
        };
        let conf = Confirm {
            title: format!("Restart {} with Chrome?", s.topic),
            body: format!(
                "Relaunches the chat with --chrome and --resume, so it keeps its history and can drive the browser. {cost}"
            ),
            ok: "Restart with Chrome".into(),
            cancel: "Cancel".into(),
        };
        self.confirm = Some((key, conf));
        self.confirm_chrome = true;
        cx.notify();
    }

    /// Confirmed: what the chat runs with now, plus --chrome, through the flags drawer's plan and relaunch.
    pub fn relaunch_chrome(&mut self, key: String, cx: &mut Context<Self>) {
        let Some(s) = self.app.read(cx).snap.session(&key) else { return };
        let (m, sid) = (s.machine.clone(), s.sid.clone());
        let core = self.app.read(cx).core.clone();
        self.chrome_pending = Some(key.clone());
        if let Some(f) = self.form_mut(&key) {
            f.sent = Some("Restarting it with Chrome...".into());
        }
        cx.spawn(async move |this, cx| {
            let res = cx
                .background_executor()
                .spawn(async move {
                    let v = core.flags_view(&m, &sid)?;
                    let mut want = crate::flagsui::Want::of(&v);
                    want.chrome = true;
                    core.flags_plan(&m, &sid, &want.body())
                })
                .await;
            let _ = this.update(cx, |v, cx| {
                match res {
                    Ok(p) => v.app.update(cx, |s, cx| s.relaunch(&key, p.body, None, cx)),
                    Err(e) => {
                        v.chrome_pending = None;
                        if let Some(f) = v.form_mut(&key) {
                            f.sent = None;
                        }
                        v.app.update(cx, |s, cx| {
                            s.toast(format!("Couldn't restart it with Chrome: {e}"), None, true, cx)
                        });
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// A relaunch started or needs its own terminal: say so under the card when it was the fix.
    pub fn ask_relaunch_news(&mut self, key: &str, cmd: Option<(String, String)>, cx: &mut Context<Self>) {
        if self.chrome_pending.as_deref() != Some(key) {
            return;
        }
        self.chrome_pending = None;
        let msg = match cmd {
            None => "Restarting it with Chrome. You can follow it in the corner.".to_string(),
            Some((command, error)) => {
                crate::state::copy_text(cx, command.clone());
                format!("{error} The command is copied: {command}")
            }
        };
        if let Some(f) = self.form_mut(key) {
            f.sent = Some(msg);
        }
        cx.notify();
    }

    /// The full card at the top of the details sheet, for a chat that waits.
    pub fn ask_card(&self, c: &Ctx, s: &Session, offline: bool) -> Option<AnyElement> {
        let t = c.t;
        let now = c.now;
        let a = s.ask_now(now)?;
        let parked = s.parked(now);
        let form = self.ask.as_ref().filter(|f| f.key == s.key && f.sig == sig(&a));
        let edge = if parked { t.line2 } else { t.need.alpha(0.55) };
        let mut card = div()
            .mt(px(12.))
            .w_full()
            .min_w_0()
            .p(px(ui::PAD_CARD))
            .rounded(px(ui::R_CARD))
            .bg(t.panel)
            .shadow(vec![Theme::ring(edge)])
            .flex()
            .flex_col()
            .gap(px(10.));
        // The tag, what kind of ask it is, and how long it has waited: wraps to a second line before anything overlaps.
        let waited = if parked {
            let d = s.wait_days(now);
            format!("Waiting {d} {}", if d == 1 { "day" } else { "days" })
        } else {
            format!("Waiting {}", fmt::mins((now - s.active).max(60.) as u64))
        };
        card = card.child(
            div()
                .w_full()
                .min_w_0()
                .flex()
                .flex_wrap()
                .items_center()
                .gap_x(px(8.))
                .gap_y(px(4.))
                .child(kind_tag(&t, a.kind))
                .children(a.caption().map(|cap| div().min_w_0().text_size(px(12.)).text_color(t.dim).child(cap)))
                .child(
                    div().min_w_0().text_size(px(12.)).text_color(if parked { t.dim } else { t.need }).child(waited),
                ),
        );
        card = card.child(
            ui::wrap(a.summary.clone())
                .text_size(px(13.5))
                .line_height(relative(1.45))
                .font_weight(FontWeight::MEDIUM)
                .text_color(t.ink),
        );
        let Some(f) = form else {
            // The fields are made on the next frame (they need the window).
            return Some(card.into_any_element());
        };
        let key = s.key.clone();
        let cx = c.cx;
        let live = !offline;
        let send_btn = |id: &str, text: &str, msg: Option<String>| {
            let ready = msg.is_some() && live;
            let k = key.clone();
            btn(&t, SharedString::from(format!("ask-{id}-{}", s.key)), true, false)
                .child(ui::icon("send", 14., t.on_accent))
                .child(label(text.to_string()))
                .when(!ready, |d| d.opacity(0.45).cursor_default())
                .when_some(msg.filter(|_| live), |d, m| {
                    d.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.ask_send(k.clone(), m.clone(), cx)))
                })
        };
        let free_box = |id: &'static str, cancel: bool| {
            let text = f.free.read(cx).value().to_string();
            let k2 = key.clone();
            div()
                .w_full()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(8.))
                .child(
                    div()
                        .w_full()
                        .min_w_0()
                        .px(px(9.))
                        .py(px(7.))
                        .rounded(px(ui::R_CTRL))
                        .border_1()
                        .border_color(t.line2)
                        .bg(t.bg)
                        .text_size(px(13.))
                        .line_height(relative(1.45))
                        .text_color(t.ink)
                        .child(gpui_base::Textarea::new(&f.free)),
                )
                .child(div().flex().flex_wrap().gap(px(6.)).child(send_btn(id, "Send", ask::compose_free(&text))).when(
                    cancel,
                    |d| {
                        d.child(
                            ui::ghost(&t, SharedString::from(format!("ask-cancel-{}", s.key)), Some("Cancel"), None)
                                .h(px(32.))
                                .px(px(10.))
                                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                    if let Some(f) = this.form_mut(&k2) {
                                        f.free_open = false;
                                    }
                                    cx.notify();
                                })),
                        )
                    },
                ))
        };
        match a.kind {
            AskKind::Choose => {
                let mut row = div().w_full().min_w_0().flex().flex_wrap().gap(px(6.));
                for (i, o) in a.options.iter().enumerate() {
                    let k = key.clone();
                    let on = f.pick == i;
                    row = row.child(
                        btn(&t, SharedString::from(format!("ask-opt-{i}-{}", s.key)), false, on)
                            .child(label(o.label.clone()).text_color(if on { t.ink } else { t.ink2 }))
                            .when(o.recommended, |d| {
                                d.child(
                                    div()
                                        .flex_none()
                                        .text_size(px(11.))
                                        .font_weight(FontWeight::NORMAL)
                                        .text_color(t.dim)
                                        .child("Claude recommends"),
                                )
                            })
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                if let Some(f) = this.form_mut(&k) {
                                    f.pick = i;
                                }
                                cx.notify();
                            })),
                    );
                }
                card = card.child(row);
                let mut sec = section(&t);
                if !a.assumptions.is_empty() {
                    sec = sec.child(
                        ui::wrap("Its assumptions (untick any that are wrong)")
                            .text_size(px(12.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(t.ink2),
                    );
                    for (i, text) in a.assumptions.iter().enumerate() {
                        let k = key.clone();
                        let on = f.ticked.get(i).copied().unwrap_or(true);
                        sec = sec.child(
                            check_row(&t, SharedString::from(format!("ask-as-{i}-{}", s.key)), on, text).on_click(
                                cx.listener(move |this, _: &ClickEvent, _, cx| {
                                    if let Some(f) = this.form_mut(&k)
                                        && let Some(x) = f.ticked.get_mut(i)
                                    {
                                        *x = !*x;
                                    }
                                    cx.notify();
                                }),
                            ),
                        );
                    }
                }
                sec = sec.child(field(&t, &f.note));
                card = card.child(sec);
                let note = f.note.read(cx).value().to_string();
                card = card.child(div().flex().child(send_btn(
                    "send",
                    "Send",
                    ask::compose_choose(&a, f.pick, &f.ticked, &note),
                )));
            }
            AskKind::Questions => {
                let typed: Vec<String> = f.typed.iter().map(|e| e.read(cx).value().to_string()).collect();
                for (qi, q) in a.questions.iter().enumerate() {
                    let mut chips = div().w_full().min_w_0().flex().flex_wrap().gap(px(6.));
                    for (oi, o) in q.options.iter().enumerate() {
                        let on = f.answers.get(qi).is_some_and(|v| v.contains(&oi));
                        let k = key.clone();
                        let multi = q.multi;
                        let hover = t.hover;
                        chips = chips.child(
                            div()
                                .id(SharedString::from(format!("ask-q{qi}-{oi}-{}", s.key)))
                                .flex_none()
                                .max_w_full()
                                .min_h(px(26.))
                                .px(px(10.))
                                .py(px(3.))
                                .flex()
                                .items_center()
                                .rounded(px(13.))
                                .text_size(px(12.5))
                                .line_height(relative(1.35))
                                .cursor_pointer()
                                .when(on, |d| d.bg(t.accent).text_color(t.on_accent).font_weight(FontWeight::SEMIBOLD))
                                .when(!on, |d| {
                                    d.shadow(vec![Theme::ring(t.line2)]).text_color(t.ink).hover(move |s| s.bg(hover))
                                })
                                .child(label(if q.recommended == Some(oi) {
                                    format!("{o} · suggested")
                                } else {
                                    o.clone()
                                }))
                                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                    if let Some(v) = this.form_mut(&k).and_then(|f| f.answers.get_mut(qi)) {
                                        if v.contains(&oi) {
                                            v.retain(|x| *x != oi);
                                        } else if multi {
                                            v.push(oi);
                                        } else {
                                            *v = vec![oi];
                                        }
                                    }
                                    cx.notify();
                                })),
                        );
                    }
                    let block =
                        if qi == 0 { div().w_full().min_w_0().flex().flex_col().gap(px(6.)) } else { section(&t) };
                    card = card.child(
                        block
                            .child(
                                ui::wrap(format!("{}. {}", qi + 1, q.q))
                                    .text_size(px(12.5))
                                    .line_height(relative(1.4))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(t.ink2),
                            )
                            .when(!q.options.is_empty(), |d| d.child(chips))
                            .when(q.about_you, |d| {
                                d.child(div().text_size(px(11.5)).text_color(t.faint).child("About you, so no guess"))
                            })
                            .children(f.typed.get(qi).map(|e| field(&t, e))),
                    );
                }
                let n = a.questions.len();
                let ready = ask::questions_ready(&a, &f.answers, &typed);
                let text = if n == 1 { "Send answer".to_string() } else { format!("Send {n} answers") };
                card = card.child(
                    div()
                        .w_full()
                        .min_w_0()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap(px(8.))
                        .child(send_btn("send", &text, ask::compose_questions(&a, &f.answers, &typed)))
                        .when(!ready, |d| {
                            d.child(
                                div()
                                    .min_w_0()
                                    .text_size(px(12.))
                                    .text_color(t.faint)
                                    .child("Answer every question first, with a chip or by typing."),
                            )
                        }),
                );
            }
            AskKind::Task => {
                let mut list = div().w_full().min_w_0().flex().flex_col().gap(px(2.));
                for (i, step) in a.steps.iter().enumerate() {
                    let k = key.clone();
                    let on = f.steps.get(i).copied().unwrap_or(false);
                    list = list.child(
                        check_row(&t, SharedString::from(format!("ask-st-{i}-{}", s.key)), on, step).on_click(
                            cx.listener(move |this, _: &ClickEvent, _, cx| {
                                if let Some(x) = this.form_mut(&k).and_then(|f| f.steps.get_mut(i)) {
                                    *x = !*x;
                                }
                                cx.notify();
                            }),
                        ),
                    );
                }
                card = card.child(list);
                let k = key.clone();
                card = card.child(
                    div()
                        .w_full()
                        .flex()
                        .flex_wrap()
                        .gap(px(6.))
                        .child(send_btn("done", "Done", Some(ask::compose_task(&a))))
                        .when(!f.free_open, |d| {
                            d.child(
                                btn(&t, SharedString::from(format!("ask-problem-{}", s.key)), false, false)
                                    .child(label("Hit a problem..."))
                                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                        if let Some(f) = this.form_mut(&k) {
                                            f.free_open = true;
                                        }
                                        cx.notify();
                                    })),
                            )
                        }),
                );
                if f.free_open {
                    card = card.child(free_box("problem-send", true));
                }
            }
            AskKind::Fix => {
                let fx = a.fix.clone();
                let label_text = fx.as_ref().map(|x| x.label.clone()).unwrap_or_default();
                let k = key.clone();
                card = card.child(
                    div()
                        .w_full()
                        .min_w_0()
                        .flex()
                        .flex_wrap()
                        .gap(px(6.))
                        .child(
                            btn(&t, SharedString::from(format!("ask-fix-{}", s.key)), true, false)
                                .child(ui::icon("relaunch", 14., t.on_accent))
                                .child(label(label_text))
                                .when(!live, |d| d.opacity(0.45).cursor_default())
                                .when(live, |d| {
                                    d.on_click(
                                        cx.listener(move |this, _: &ClickEvent, _, cx| this.ask_fix(k.clone(), cx)),
                                    )
                                }),
                        )
                        .children(ask::compose_fix_alt(&a).map(|alt| {
                            let k = key.clone();
                            btn(&t, SharedString::from(format!("ask-alt-{}", s.key)), false, false)
                                .child(label(alt.clone()))
                                .when(!live, |d| d.opacity(0.45).cursor_default())
                                .when(live, |d| {
                                    d.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                        this.ask_send(k.clone(), alt.clone(), cx)
                                    }))
                                })
                        })),
                );
                card = card.child(
                    ui::wrap(format!(
                        "Relaunches with --chrome and keeps its history. Re-reads {} of context.",
                        s.ctx_label()
                    ))
                    .text_size(px(12.))
                    .line_height(relative(1.45))
                    .text_color(t.faint),
                );
            }
            AskKind::Open => {
                card = card.child(free_box("free-send", false));
            }
        }
        if offline {
            card = card.child(
                ui::wrap(format!("{} is offline, so you can't send answers right now.", s.machine_label))
                    .text_size(px(12.))
                    .text_color(t.dim),
            );
        } else if let Some(sent) = &f.sent {
            card = card.child(ui::wrap(sent.clone()).text_size(px(12.)).line_height(relative(1.45)).text_color(t.warm));
        } else if a.kind != AskKind::Fix {
            card = card.child(
                ui::wrap("Send opens its terminal and types this in for you.").text_size(px(11.5)).text_color(t.faint),
            );
        }
        Some(card.into_any_element())
    }

    /// The board card's part: Claude's summary on at most two lines, and its one button. `ink` colors the summary.
    pub fn card_quick(&self, c: &Ctx, s: &Session, a: &Ask, ink: Hsla) -> AnyElement {
        let t = c.t;
        let key = s.key.clone();
        let (text, q) = a.quick();
        let primary = !matches!(q, Quick::Answer) && !s.parked(c.now);
        let icon = match q {
            Quick::Send(_) => "send",
            Quick::Fix => "relaunch",
            Quick::Answer => "reply",
        };
        div()
            .w_full()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(6.))
            // The kind's tag on a line of its own: beside the state line a long one ("Do this, then confirm") squeezed it.
            .child(div().flex().child(kind_tag(&t, a.kind)))
            .child(
                div()
                    .w_full()
                    .min_w_0()
                    .text_size(px(12.5))
                    .line_height(relative(1.4))
                    .text_color(ink)
                    .line_clamp(2)
                    .text_ellipsis()
                    .child(a.summary.clone()),
            )
            .child(
                div().w_full().min_w_0().flex().child(
                    btn(&t, SharedString::from(format!("quick-{}", s.key)), primary, false)
                        .min_h(px(28.))
                        .py(px(3.))
                        .px(px(10.))
                        .text_size(px(12.5))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(ui::icon(icon, 13., if primary { t.on_accent } else { t.ink2 }))
                        // one line on the card: a long option label ends in an ellipsis instead of growing the
                        // card; a fixed label (Answer..., the fix) always shows whole
                        .child(
                            div()
                                .min_w_0()
                                .flex_shrink(1.)
                                .truncate()
                                .when(!matches!(q, Quick::Send(_)), |d| d.flex_none())
                                .child(text),
                        )
                        .on_click(c.cx.listener(move |this, _: &ClickEvent, _, cx| {
                            cx.stop_propagation();
                            this.ask_quick(key.clone(), q.clone(), cx);
                        })),
                ),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn sent_shows_only_once_the_reply_went_in() {
        assert_eq!(super::sent_note("yes", true), "Sent: yes");
        assert!(!super::sent_note("yes", false).starts_with("Sent"), "a terminal that never opened sent nothing");
    }

    #[test]
    fn a_second_send_while_one_is_on_its_way_types_nothing() {
        let mut sending = std::collections::HashSet::new();
        assert!(super::start_send(&mut sending, "studio:a"));
        assert!(!super::start_send(&mut sending, "studio:a"), "clicked again");
        assert!(super::start_send(&mut sending, "studio:b"), "another chat");
        sending.remove("studio:a");
        assert!(super::start_send(&mut sending, "studio:a"), "the first one is done");
    }
}
