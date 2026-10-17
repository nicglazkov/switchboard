//! The inspector: one chat in full, what you can do with it and its numbers. Docked beside the board on wide windows
//! (its width set by a drag handle and saved), floating on narrow ones, and on the right of the Archive sheet.

use gpui::{
    AnyElement, ClickEvent, FontWeight, Hsla, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, div, prelude::*, px, relative,
};
use sb_core::actions::{self as acts, Action, ActionId, Pick};
use sb_core::api::ApiThreadChat;
use sb_core::live::{Live, LiveKind};
use sb_core::model::COMPACT;
use sb_core::names::{self, TagKind};
use sb_core::{Replaced, Session, TextRun, fmt};

use crate::place::Place;
use crate::theme::Theme;
use crate::theme::{NUM, UI};
use crate::ui;
use crate::view::{BoardView, Ctx};

/// The state in a few words, at the top of the sheet: the shared live words (sb_core::live), with the wait in days
/// and a stopped chat said plainly.
fn state_text(s: &Session, l: &Live, now: f64) -> String {
    if let Some(a) = &l.stale {
        return format!("{}, {a}", l.word);
    }
    match l.kind {
        LiveKind::Held => "Message on hold".into(),
        // What it needs is in the card under the title, in full: the head only says how long it has waited.
        LiveKind::Parked => {
            let d = s.wait_days(now);
            format!("Waiting {d} {}", if d == 1 { "day" } else { "days" })
        }
        LiveKind::Wait => format!("Needs you, waiting {}", fmt::mins((now - s.active).max(60.) as u64)),
        LiveKind::Run => "Working".into(),
        LiveKind::Warm => format!("Warm, {} left", sb_core::live::clock(l.left)),
        LiveKind::Cold => format!("Cold, {} to re-read", s.ctx_label()),
        LiveKind::Idle if s.reaped_note() => "Stopped".into(),
        LiveKind::Stall | LiveKind::Idle => l.text(),
    }
}

/// One plain sentence about where it stands.
fn state_sentence(s: &Session, l: &Live, now: f64, offline: bool) -> String {
    if offline {
        return format!("{} is offline, so this chat can't be opened right now.", s.machine_label);
    }
    let said = match l.kind {
        LiveKind::Held => format!(
            "Your message is on hold. The chat went cold at {}, so sending it now would re-read all of it.",
            s.ctx_label()
        ),
        LiveKind::Parked => format!(
            "Waiting for your answer since {}. After a day it is parked: it stays here but no longer counts toward the badge or notifications.",
            fmt::when(s.active, now)
        ),
        LiveKind::Wait => format!("Waiting for your answer since {}.", fmt::when(s.active, now)),
        LiveKind::Run => "Working right now.".into(),
        LiveKind::Stall => format!(
            "No word from it for {}, though it never finished its turn. It may be stuck on one step.",
            fmt::sago(s.active, now)
        ),
        LiveKind::Warm => format!("Warm for {} more. Replies stay cheap until then.", fmt::mins(l.left)),
        LiveKind::Cold if s.cache_until > 0.0 => {
            format!("Cold since {}. The next message re-reads {}.", fmt::when(s.cache_until, now), s.ctx_label())
        }
        LiveKind::Cold => format!("Cold. The next message re-reads {}.", s.ctx_label()),
        LiveKind::Idle => match &s.reaped {
            Some(r) if s.reaped_note() => r.text(),
            _ if s.ctx > 0 => {
                format!("Quiet since {}. Whether its cache is still warm is not known yet.", fmt::when(s.active, now))
            }
            _ => format!("Quiet since {}.", fmt::when(s.active, now)),
        },
    };
    match &l.stale {
        Some(a) => format!("{said} This is {a}: {} has not reported since.", s.machine_label),
        None => said,
    }
}

/// Attach tool: the chat it is for, what is checked, whether a running chat's restart was asked about, and the
/// command to run when the chat lives in a terminal Switchboard cannot restart (its relaunch need "terminal").
pub struct ToolsPick {
    pub key: String,
    pub sel: Vec<String>,
    pub confirm: bool,
    pub cmd: Option<(String, String)>,
}

pub struct InsOpts {
    /// A close button (not in the Archive sheet, where the list owns the selection).
    pub close: bool,
    /// The corner radius the body paints with: the floating sheet's, or 0 where it fills a square panel.
    pub radius: f32,
    /// In its own window (detached.rs): the head offers Dock instead of Pop out.
    pub detached: bool,
}

fn keycap(k: &str) -> Option<&'static str> {
    match k {
        "Enter" => Some("↵"),
        "F" => Some("F"),
        "R" => Some("R"),
        "C" => Some("C"),
        _ => None,
    }
}

impl BoardView {
    /// The inspector body for `key` (a current chat or a replaced one). Fills its parent.
    pub fn inspector(&self, c: &Ctx, key: &str, o: InsOpts) -> AnyElement {
        let t = c.t;
        let body = if let Some(s) = c.app.snap.session(key) {
            self.ins_session(c, s, &o)
        } else if let Some(r) = c.app.snap.replaced_row(key) {
            self.ins_replaced(c, r, &o)
        } else {
            return div().size_full().bg(t.panel).into_any_element();
        };
        // GPUI clips children to a rectangle, so the body takes the sheet's own radius (less its 1 px border) to keep
        // all four corners round.
        div().size_full().min_h_0().flex().flex_col().bg(t.panel).rounded(px(o.radius)).child(body).into_any_element()
    }

    /// The docked panel or the floating card's content. Only drawn with a selection.
    pub fn inspector_panel(&self, c: &Ctx, key: Option<&str>, docked: bool) -> AnyElement {
        let t = c.t;
        let Some(key) = key else { return div().into_any_element() };
        let inner = self.inspector(
            c,
            key,
            InsOpts { close: true, radius: if docked { 0. } else { ui::R_SHEET - 1. }, detached: false },
        );
        if docked {
            // beside the board on a wide window: the dock (view.rs) sizes it and draws the handle on its left
            div().size_full().min_h_0().bg(t.panel).child(inner).into_any_element()
        } else {
            inner
        }
    }

    /// `star`: the chat's key and whether it is starred (a replaced, older chat has no star button).
    fn ins_head(
        &self,
        c: &Ctx,
        state: AnyElement,
        machine: &str,
        star: Option<(String, bool)>,
        o: &InsOpts,
    ) -> impl IntoElement {
        let t = c.t;
        let cx = c.cx;
        // Rename: a pencil before the star, for a current chat on a machine that can keep a name (rename.rs). Not in
        // the popped-out window: the name field lives in the board's window.
        let rename_btn = star.as_ref().filter(|_| !o.detached && c.app.snap.can_rename(machine)).map(|(key, _)| {
            let key = key.clone();
            let on = self.renaming(&key, crate::rename::EditAt::Details);
            ui::icon_btn(&t, "ins-rename", "pencil", 26.).flex_none().when(on, |d| d.bg(t.press)).on_click(cx.listener(
                move |this, _: &ClickEvent, window, cx| {
                    if this.renaming(&key, crate::rename::EditAt::Details) {
                        this.cancel_chat_name(cx);
                    } else {
                        this.rename_chat(key.clone(), window, cx);
                    }
                },
            ))
        });
        let star_btn = star.map(|(key, on)| {
            let (hover, press) = (t.hover, t.press);
            div()
                .id("ins-star")
                .flex_none()
                .size(px(26.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(ui::R_ITEM))
                .cursor_pointer()
                .hover(move |s| s.bg(hover))
                .active(move |s| s.bg(press))
                .child(ui::icon(if on { "star-on" } else { "star" }, 14., if on { t.cold } else { t.dim }))
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    let key = key.clone();
                    this.app.update(cx, |s, cx| s.star(&key, !on, cx));
                }))
        });
        div()
            .flex_none()
            .h(px(48.))
            .pl(px(20.))
            .pr(px(10.))
            .flex()
            .items_center()
            .gap(px(4.))
            // A long state line (a chat's need) ends in an ellipsis; the buttons on the right always stay.
            .child(div().flex_1().min_w_0().overflow_hidden().child(state))
            .children(rename_btn)
            .children(star_btn)
            // Pop out: the details move to their own window and keep following what is clicked; Dock brings them back.
            .when(o.close && !o.detached, |d| {
                d.child(
                    ui::icon_btn(&t, "ins-popout", "popout", 26.)
                        .flex_none()
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.pop_out(cx))),
                )
            })
            .when(o.detached, |d| {
                d.child(
                    ui::icon_btn(&t, "ins-dock", "sidebar", 26.)
                        .flex_none()
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.dock(cx))),
                )
            })
            .when(o.close && !o.detached, |d| {
                d.child(
                    ui::icon_btn(&t, "ins-close", "x", 26.)
                        .flex_none()
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.select(None, cx))),
                )
            })
    }

    fn ins_title(&self, c: &Ctx, topic: &str, tag: Option<&String>, kind: Option<TagKind>) -> impl IntoElement {
        let t = c.t;
        // The title wraps; the tag sits on its first line.
        div()
            .flex()
            .items_start()
            .gap(px(8.))
            .child(
                div()
                    .min_w_0()
                    .flex_shrink(1.)
                    .text_size(px(18.))
                    .line_height(relative(1.3))
                    .font_weight(FontWeight::BOLD)
                    .text_color(t.ink)
                    .child(topic.to_string()),
            )
            .when_some(tag, |d, tg| d.child(ui::name_tag(&t, tg, kind).mt(px(2.5))))
    }

    fn ins_sub(&self, c: &Ctx, machine: &str, label: &str, folder: &str, active: f64) -> impl IntoElement {
        let t = c.t;
        let offline = c.app.snap.machine(machine).map(|m| m.is_offline()).unwrap_or(false);
        let label = c.app.snap.label_of(machine).unwrap_or_else(|| label.to_string());
        let label = if offline { format!("{label} (offline)") } else { label };
        let ago = fmt::ago(active, c.now);
        div().mt(px(6.)).text_size(px(12.)).line_height(relative(1.5)).child(ui::mixed(&[
            (&label, UI, t.dim),
            (" / ", UI, t.faint),
            (folder, NUM, t.ink2),
            (" / ", UI, t.faint),
            (&ago, UI, t.dim),
        ]))
    }

    fn ins_session(&self, c: &Ctx, s: &Session, o: &InsOpts) -> AnyElement {
        let t = c.t;
        let now = c.now;
        // The one live mapping, the same words and colors as the card and a terminal's strip.
        let live = c.app.live_of(s, now, None);
        let lc = match live.kind {
            _ if live.stale.is_some() => t.faint,
            LiveKind::Idle | LiveKind::Stall | LiveKind::Parked => t.dim,
            _ => crate::chrome::live_color(&t, &live),
        };
        let state = div()
            .flex()
            .items_center()
            .min_w_0()
            .gap(px(8.))
            .child(crate::chrome::live_dot(&t, &live, SharedString::from(format!("ins-live-{}", s.key)), 8.))
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_size(px(13.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(lc)
                    .child(state_text(s, &live, now)),
            )
            .when_some(live.expensive.clone(), |d, e| d.child(crate::chrome::cost_badge(&t, &e, live.stale.is_some())))
            .into_any_element();
        let acts = acts::for_session(s, &c.app.snap, now);
        // The choice and the rows below cover these; the rest stay in a short list.
        let covered = [
            ActionId::Terminal,
            ActionId::ResumeOpen,
            ActionId::Fresh,
            ActionId::Resume,
            ActionId::Continue,
            ActionId::Handoff,
            ActionId::Stop,
        ];
        // Rename is the pencil by the star (in the popped-out window, a row here).
        let rest: Vec<Action> = acts
            .iter()
            .filter(|a| !covered.contains(&a.id) && !(a.id == ActionId::Rename && !o.detached))
            .cloned()
            .map(|mut a| {
                a.primary = false;
                a
            })
            .collect();
        let kept = s.keep && s.alive;
        let offline = c.app.snap.machine(&s.machine).is_some_and(|m| m.is_offline());

        let body = div()
            .id(SharedString::from(format!("ins-body-{}", s.key)))
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&self.ins_scroll)
            .pt(px(4.))
            .px(px(20.))
            .pb(px(24.))
            .child(if self.renaming(&s.key, crate::rename::EditAt::Details) && !o.detached {
                self.rename_details(c)
            } else {
                // a double-click on the name renames it, as on a terminal tab
                let k = s.key.clone();
                let can = !o.detached && c.app.snap.can_rename(&s.machine);
                div()
                    .id("ins-title")
                    .child(self.ins_title(c, &s.topic, s.tag.as_ref(), s.tag_kind))
                    .when(can, |d| {
                        d.on_click(c.cx.listener(move |this, ev: &ClickEvent, window, cx| {
                            if ev.click_count() >= 2 {
                                this.rename_chat(k.clone(), window, cx);
                            }
                        }))
                    })
                    .into_any_element()
            })
            .child(
                div()
                    .mt(px(6.))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .text_size(px(12.5))
                    .text_color(t.dim)
                    .child(ui::icon("folder", 13., t.faint).flex_none())
                    .child(ui::wrap(format!("{}, {}", s.machine_label, s.folder)).flex_1()),
            )
            // A waiting chat's "Needs you" card, the only place with the full controls (askcard.rs).
            .children(self.ask_card(c, s, offline))
            .child(
                ui::wrap(state_sentence(s, &live, now, offline))
                    .mt(px(8.))
                    .text_size(px(13.))
                    .line_height(relative(1.5))
                    .text_color(t.ink2),
            )
            .when(kept, |d| d.child(div().mt(px(8.)).flex().child(ui::note_tag(&t, "pin", "Kept running"))))
            .child(self.ins_choice(c, s, offline))
            .child(self.ins_places(c, s, offline))
            .child(self.ins_facts(c, s))
            .child(self.ins_flags(c, s))
            .children(self.ins_tools(c, s))
            .children(self.ins_chrome(c, s))
            .child(self.ins_more(c, s, offline))
            .when(!rest.is_empty(), |d| d.child(self.ins_actions(c, &s.key, &rest)))
            .children(self.ins_history(c, s));
        div()
            .size_full()
            .min_h_0()
            .flex()
            .flex_col()
            .child(self.ins_head(c, state, &s.machine, Some((s.key.clone(), s.star)), o))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .relative()
                    .flex()
                    .flex_col()
                    .child(body)
                    .child(ui::scrollbar(&self.ins_scroll)),
            )
            .into_any_element()
    }

    /// Resume or Start fresh: what each does and costs, in plain words, and which one is better now and why.
    fn ins_choice(&self, c: &Ctx, s: &Session, offline: bool) -> AnyElement {
        let t = c.t;
        let cx = c.cx;
        let now = c.now;
        let wrap = |d: gpui::Div| {
            div().mt(px(16.)).p(px(14.)).rounded(px(ui::R_CARD)).bg(t.bg).border_1().border_color(t.line).child(d)
        };
        if offline {
            return wrap(
                div().child(
                    ui::wrap(format!(
                        "{} is offline. When it is back, you can resume or start fresh from here.",
                        s.machine_label
                    ))
                    .text_size(px(13.))
                    .text_color(t.dim),
                ),
            )
            .into_any_element();
        }
        let g = acts::suggest(s, now);
        let key = s.key.clone();
        let Some(pick) = g.pick else {
            let k = key.clone();
            return wrap(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(10.))
                    .child(ui::wrap(g.why.clone()).text_size(px(13.)).line_height(relative(1.45)).text_color(t.ink2))
                    .child(
                        div().flex().child(
                            ui::primary_btn(&t, "ch-term", 34.)
                                .px(px(14.))
                                .child(ui::icon("term", 15., t.on_accent))
                                .child("Open terminal")
                                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                    this.open_terminal(k.clone(), false, cx)
                                })),
                        ),
                    ),
            )
            .into_any_element();
        };
        let left = s.cache_left(now);
        // Both choices from the agent's cost model, for the cache as it is this second. An older agent sends none:
        // then the words only say what resuming re-reads, and no number is made up for Start fresh.
        let cost = s.cost.as_ref();
        let warm = left > 0;
        let (r_up, f_up) = match cost {
            Some(c) => ((if warm { c.resume.warm } else { c.resume.cold }) as f32, c.fresh.total as f32),
            None => (if warm { 0.0 } else { s.ctx as f32 }, 0.0),
        };
        let scale = r_up.max(f_up).max(1.0);
        let r_w = (r_up / scale).clamp(0.03, 1.);
        let f_w = (f_up / scale).clamp(0.03, 1.);
        let k = |n: f64| fmt::k(n.max(0.0).round() as u64);
        let resume_cost = match cost {
            Some(c) if warm => format!(
                "About {} up front, because the cache is warm. Then about {} each reply.",
                k(c.resume.warm),
                k(c.resume.each)
            ),
            Some(c) => format!(
                "About {} up front, because the cache is cold and all {} is read again. Then about {} each reply.",
                k(c.resume.cold),
                s.ctx_label(),
                k(c.resume.each)
            ),
            None => g.resume_cost.clone(),
        };
        let fresh_cost = match cost {
            Some(c) if c.summary_ready => format!(
                "About {} up front: the summary is already written, and the new chat reads {}. Then about {} each reply.",
                k(c.fresh.total),
                k(c.fresh.first),
                k(c.fresh.each)
            ),
            Some(c) => format!(
                "About {} up front: writing the summary {} + the new chat {}. Then about {} each reply.",
                k(c.fresh.total),
                k(c.fresh.summary),
                k(c.fresh.first),
                k(c.fresh.each)
            ),
            None => "Writing the summary reads the chat once, then the new chat starts small.".into(),
        };
        let fresh_what = if s.handoff_fresh {
            "Opens a new chat from the summary already written. This one stays in your history."
        } else {
            "Writes a summary when you pick it (about a minute), then opens a new chat that starts from it. This one stays in your history."
        };
        let needs = s.lamp(now).needs_you();
        // A waiting chat is answered in its card above, so this only opens the terminal.
        let resume_label = if s.held {
            "Send anyway"
        } else if needs || (s.alive && s.background()) {
            "Open terminal"
        } else {
            "Resume"
        };
        let resume_id = if s.held {
            ActionId::Pass
        } else if s.alive && s.background() {
            ActionId::Terminal
        } else if s.alive {
            ActionId::Open
        } else {
            ActionId::ResumeOpen
        };
        // One card per choice, stacked: at the sheet's 400 to 452 px two cards side by side squeeze their words, so
        // they never sit side by side here. Each card owns its badge, meter and button, all inside its own padding.
        let opt = |id: &'static str,
                   k: Pick,
                   ic: &'static str,
                   h: &'static str,
                   what: String,
                   cost: String,
                   w: f32,
                   btn: &'static str,
                   act: ActionId| {
            let sug = pick == k;
            let key = key.clone();
            let bar_c = if k == Pick::Fresh { t.accent } else { t.run };
            div()
                .w_full()
                .min_w_0()
                .p(px(ui::PAD_CARD))
                .rounded(px(ui::R_CARD))
                .bg(t.panel)
                .flex()
                .flex_col()
                .gap(px(8.))
                .shadow(vec![Theme::ring(if sug { t.accent } else { t.line })])
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(7.))
                        .child(ui::icon(ic, 15., if sug { t.accent } else { t.dim }))
                        .child(ui::wrap(h).flex_1().text_size(px(13.5)).font_weight(FontWeight::BOLD).text_color(t.ink))
                        .when(sug, |d| {
                            d.child(
                                div()
                                    .flex_none()
                                    .px(px(7.))
                                    .h(px(18.))
                                    .flex()
                                    .items_center()
                                    .rounded_full()
                                    .bg(t.accent)
                                    .text_size(px(10.5))
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(t.on_accent)
                                    .child("Suggested"),
                            )
                        }),
                )
                .child(ui::wrap(what).text_size(px(12.5)).line_height(relative(1.45)).text_color(t.dim))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(5.))
                        .child(
                            div()
                                .h(px(4.))
                                .rounded_full()
                                .bg(t.track)
                                .overflow_hidden()
                                .child(div().h_full().w(relative(w)).rounded_full().bg(bar_c)),
                        )
                        .child(ui::wrap(cost).text_size(px(12.)).line_height(relative(1.45)).text_color(t.ink2)),
                )
                .child({
                    let b = if sug { ui::primary_btn(&t, id, 32.) } else { ui::line_btn(&t, id, 32.) };
                    b.justify_center()
                        .text_size(px(13.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(ui::icon(ic, 14., if sug { t.on_accent } else { t.dim }))
                        .child(btn)
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            if act == ActionId::Fresh {
                                // The choice already said what it does and costs: no second ask.
                                let k = key.clone();
                                this.app.update(cx, |s, cx| s.act(ActionId::Fresh, &k, cx));
                                cx.notify();
                            } else {
                                this.run_action(act, key.clone(), cx);
                            }
                        }))
                })
        };
        let head = format!("{} is the better pick now.", if pick == Pick::Fresh { "Start fresh" } else { "Resume" });
        let how = g.how.clone();
        let how_open = self.how_open;
        div()
            .mt(px(16.))
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(div().min_w_0().text_size(px(13.)).line_height(relative(1.45)).text_color(t.ink2).child(
                ui::runs_text(
                    &[TextRun { text: head, bold: true }, TextRun { text: format!(" {}", g.why), bold: false }],
                    t.ink,
                ),
            ))
            .child(opt(
                "ch-resume",
                Pick::Resume,
                "resume",
                "Resume this chat",
                "Opens the same chat. Claude still has everything said so far.".into(),
                resume_cost,
                r_w,
                resume_label,
                resume_id,
            ))
            .child(opt(
                "ch-fresh",
                Pick::Fresh,
                "fresh",
                "Start fresh",
                fresh_what.into(),
                fresh_cost,
                f_w,
                "Start fresh",
                ActionId::Fresh,
            ))
            .children(how.map(|how| {
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .child(
                        div()
                            .id("ch-how")
                            .flex()
                            .items_center()
                            .gap(px(5.))
                            .cursor_pointer()
                            .text_size(px(12.))
                            .text_color(t.dim)
                            .hover(|s| s.opacity(0.8))
                            .child(ui::icon(if how_open { "down" } else { "chev" }, 12., t.dim))
                            .child("How is this estimated?")
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.how_open = !this.how_open;
                                cx.notify();
                            })),
                    )
                    .when(how_open, |d| {
                        d.child(ui::wrap(how).text_size(px(12.)).line_height(relative(1.5)).text_color(t.dim))
                    })
            }))
            .child(
                ui::wrap("Once the new chat is ready, Switchboard offers to open its terminal.")
                    .text_size(px(12.))
                    .text_color(t.faint),
            )
            .into_any_element()
    }

    /// Where its terminal opens: a tab, a window of its own, or a floating sheet, with the shortcut for each.
    fn ins_places(&self, c: &Ctx, s: &Session, offline: bool) -> AnyElement {
        let t = c.t;
        if offline || !(s.alive && s.background()) && s.alive {
            return div().into_any_element();
        }
        div()
            .mt(px(14.))
            .flex()
            .flex_col()
            .gap(px(6.))
            .child(
                div().text_size(px(12.)).font_weight(FontWeight::SEMIBOLD).text_color(t.dim).child("Open its terminal"),
            )
            .child(div().flex().gap(px(6.)).children([Place::Tab, Place::Window, Place::Float].into_iter().map(|p| {
                let k = s.key.clone();
                let label = match p {
                    Place::Tab => "In a tab",
                    Place::Window => "In a window",
                    Place::Float => "As a sheet",
                };
                ui::line_btn(&t, SharedString::from(format!("ins-place-{p:?}")), 30.)
                    .flex_1()
                    .px(px(8.))
                    .justify_center()
                    .text_size(px(12.5))
                    .child(ui::icon(p.icon(), 13., t.dim))
                    .child(label)
                    .child(div().text_size(px(11.)).text_color(t.faint).child(p.keys()))
                    .on_click(c.cx.listener(move |this, _: &ClickEvent, _, cx| this.open_terminal_in(k.clone(), p, cx)))
            })))
            .into_any_element()
    }

    /// Context, cache, model, last active, machine and memory, two to a row.
    fn ins_facts(&self, c: &Ctx, s: &Session) -> AnyElement {
        let t = c.t;
        let now = c.now;
        let mb = c
            .app
            .snap
            .cleanup
            .as_ref()
            .and_then(|cl| cl.items.iter().find(|d| d.machine == s.machine && d.sid.as_deref() == Some(s.sid.as_str())))
            .and_then(|d| d.mb);
        let cell = |k: &str, v: AnyElement| {
            div()
                .flex_1()
                .min_w_0()
                .p(px(10.))
                .rounded(px(ui::R_CTRL))
                .bg(t.bg)
                .flex()
                .flex_col()
                .gap(px(3.))
                .child(div().text_size(px(11.5)).text_color(t.faint).child(k.to_string()))
                .child(v)
        };
        let txt = |v: String| {
            div().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).text_color(t.ink).child(v).into_any_element()
        };
        let frac = (s.ctx as f32 / COMPACT as f32).clamp(0., 1.);
        let tone = if s.over() {
            t.bad
        } else if s.heavy() {
            t.cold
        } else {
            t.fill
        };
        let ctx = div()
            .flex()
            .flex_col()
            .gap(px(5.))
            .child(div().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).text_color(t.ink).child(if s.ctx > 0 {
                format!("{} of 300k", s.ctx_label())
            } else {
                "None yet".into()
            }))
            .child(
                div()
                    .relative()
                    .h(px(4.))
                    .rounded_full()
                    .bg(t.track)
                    .child(div().absolute().left_0().top_0().h_full().w(relative(frac)).rounded_full().bg(tone))
                    .child(div().absolute().top(px(-2.)).h(px(8.)).w(px(2.)).left(relative(0.5)).bg(t.faint)),
            )
            .into_any_element();
        let cache =
            if s.cache_left(now) > 0 { format!("Warm, {} left", fmt::clock(s.cache_left(now))) } else { "Cold".into() };
        let machine = if c.app.snap.machine(&s.machine).is_some_and(|m| m.is_offline()) {
            format!("{} (offline)", s.machine_label)
        } else {
            s.machine_label.clone()
        };
        let mem = if !s.alive { "Not running".into() } else { mb.map(fmt::mem).unwrap_or_else(|| "Running".into()) };
        let row = |a: AnyElement, b: AnyElement| div().flex().gap(px(8.)).child(a).child(b);
        div()
            .mt(px(16.))
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(row(cell("Context", ctx).into_any_element(), cell("Cache", txt(cache)).into_any_element()))
            .child(row(
                cell("Model", txt(if s.model.is_empty() { "Default".into() } else { s.model.clone() }))
                    .into_any_element(),
                cell("Last active", txt(fmt::when(s.active, now))).into_any_element(),
            ))
            .child(row(cell("Machine", txt(machine)).into_any_element(), cell("Memory", txt(mem)).into_any_element()))
            .into_any_element()
    }

    /// Its launch flags, and the way into the flags drawer.
    fn ins_flags(&self, c: &Ctx, s: &Session) -> AnyElement {
        let t = c.t;
        let k = s.key.clone();
        let chips = s.chips(true);
        div()
            .mt(px(16.))
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(12.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(t.dim)
                            .child("Flags"),
                    )
                    .child(
                        ui::ghost(&t, "ins-flags", Some("Change"), Some("flag"))
                            .h(px(26.))
                            .px(px(8.))
                            .text_size(px(12.5))
                            .on_click(c.cx.listener(move |this, _: &ClickEvent, _, cx| this.open_flags(k.clone(), cx))),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap(px(5.))
                    .child(ui::tag(&t, if s.model.is_empty() { "Default model" } else { &s.model }))
                    .children(chips.iter().map(|ch| ui::tag(&t, &ch.long))),
            )
            .into_any_element()
    }

    /// A running chat restarted to load its tools, but it lives in a terminal: show the command to run there.
    pub fn tools_need_terminal(&mut self, key: &str, cmd: String, err: String, cx: &mut gpui::Context<Self>) {
        if self.flags.as_ref().is_some_and(|d| d.key == key) {
            return;
        }
        self.tools_pick =
            Some(ToolsPick { key: key.to_string(), sel: Vec::new(), confirm: false, cmd: Some((cmd, err)) });
        cx.notify();
    }

    /// Tools: the library tools this chat runs with, and Attach tool. A running chat restarts to load them, so that
    /// is asked first.
    fn ins_tools(&self, c: &Ctx, s: &Session) -> Option<AnyElement> {
        let lib = c.app.lib();
        // An older server has no library: say nothing unless the chat somehow has tools.
        if lib.is_none() && s.tools.is_empty() {
            return None;
        }
        let t = c.t;
        let cx = c.cx;
        let pick = self.tools_pick.as_ref().filter(|p| p.key == s.key);
        let busy = c.app.lib_busy.contains(&format!("attach:{}", s.key));
        let key = s.key.clone();
        let cur = s.tools.clone();
        let title = match s.tools_word() {
            Some(w) => format!("Tools: {w}"),
            None => "Tools".to_string(),
        };
        let mut head = div()
            .flex()
            .items_center()
            .child(div().flex_1().text_size(px(12.)).font_weight(FontWeight::SEMIBOLD).text_color(t.dim).child(title));
        if lib.is_some() && pick.is_none() {
            let (k, cur) = (key.clone(), cur.clone());
            head = head.child(
                ui::ghost(&t, "ins-tools", Some(if busy { "Attaching" } else { "Attach tool..." }), Some("cube"))
                    .h(px(26.))
                    .px(px(8.))
                    .text_size(px(12.5))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.tools_pick =
                            Some(ToolsPick { key: k.clone(), sel: cur.clone(), confirm: false, cmd: None });
                        cx.notify();
                    })),
            );
        }
        let chips = if s.tools.is_empty() {
            div()
                .text_size(px(12.5))
                .text_color(t.faint)
                .child("None. Tools on for every new chat join when one starts.")
                .into_any_element()
        } else {
            div()
                .flex()
                .flex_wrap()
                .gap(px(5.))
                .children(s.tools.iter().map(|n| ui::tool_chip(&t, n)))
                .into_any_element()
        };
        let mut out = div().mt(px(16.)).flex().flex_col().gap(px(8.)).child(head).child(chips);
        if let Some(p) = pick {
            out = out.child(self.tools_panel(c, s, p));
        }
        Some(out.into_any_element())
    }

    fn tools_panel(&self, c: &Ctx, s: &Session, p: &ToolsPick) -> AnyElement {
        let t = c.t;
        let cx = c.cx;
        let close = |id: &'static str, label: &'static str| {
            ui::ghost(&t, id, Some(label), None).h(px(30.)).px(px(10.)).text_size(px(12.5)).on_click(cx.listener(
                |this, _: &ClickEvent, _, cx| {
                    this.tools_pick = None;
                    cx.notify();
                },
            ))
        };
        let card = div()
            .p(px(12.))
            .rounded(px(ui::R_CARD))
            .bg(t.bg)
            .border_1()
            .border_color(t.line)
            .flex()
            .flex_col()
            .gap(px(10.));
        // The chat lives in a terminal: Switchboard cannot restart it, so here is what to run there.
        if let Some((cmd, err)) = &p.cmd {
            let c2 = cmd.clone();
            return card
                .child(ui::wrap(err.clone()).text_size(px(12.5)).line_height(relative(1.45)).text_color(t.ink2))
                .child(
                    div()
                        .px(px(10.))
                        .py(px(7.))
                        .rounded(px(ui::R_ITEM))
                        .bg(t.panel)
                        .border_1()
                        .border_color(t.line2)
                        .child(ui::wrap(cmd.clone()).font_family(NUM).text_size(px(12.)).text_color(t.ink)),
                )
                .child(div().flex().justify_end().gap(px(6.)).child(close("ins-tools-done", "Done")).child(
                    ui::line_btn(&t, "ins-tools-copy", 30.).child(ui::icon("copy", 13., t.dim)).child("Copy").on_click(
                        cx.listener(move |this, _: &ClickEvent, _, cx| {
                            crate::state::copy_text(cx, c2.clone());
                            this.app.update(cx, |s, cx| s.toast("Copied the command", None, false, cx));
                        }),
                    ),
                ))
                .into_any_element();
        }
        let Some(lib) = c.app.lib() else { return div().into_any_element() };
        let mut chips = div().flex().flex_wrap().gap(px(6.));
        for tool in &lib.tools {
            let on = p.sel.contains(&tool.name);
            let name = tool.name.clone();
            let ic = if tool.kinds.iter().any(|k| k == "mcp") { "cube" } else { "spark" };
            chips = chips.child(
                ui::check_chip(&t, SharedString::from(format!("ins-tool-{}", tool.name)), on, false, ic, &tool.name)
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        if let Some(p) = this.tools_pick.as_mut() {
                            if p.sel.contains(&name) {
                                p.sel.retain(|x| x != &name);
                            } else {
                                p.sel.push(name.clone());
                            }
                            p.confirm = false;
                        }
                        cx.notify();
                    })),
            );
        }
        if lib.tools.is_empty() {
            chips = chips.child(
                div()
                    .text_size(px(12.5))
                    .text_color(t.faint)
                    .child("No tools in the library yet. Add some in Settings, Library."),
            );
        }
        let changed = {
            let mut a = p.sel.clone();
            let mut b = s.tools.clone();
            a.sort();
            b.sort();
            a != b
        };
        let running = s.alive;
        let (k1, k2, sel1, sel2) = (s.key.clone(), s.key.clone(), p.sel.clone(), p.sel.clone());
        let mut card = card.child(chips);
        if p.confirm {
            card = card
                .child(
                    div()
                        .flex()
                        .items_start()
                        .gap(px(8.))
                        .p(px(10.))
                        .rounded(px(ui::R_CTRL))
                        .bg(t.need.alpha(if t.dark { 0.14 } else { 0.1 }))
                        .child(div().mt(px(1.)).child(ui::icon("warn", 14., t.need)))
                        .child(
                            ui::wrap("Attaching restarts this chat, and it re-reads its whole context.")
                                .flex_1()
                                .text_size(px(12.5))
                                .line_height(relative(1.4))
                                .text_color(t.ink),
                        ),
                )
                .child(
                    div().flex().justify_end().gap(px(6.)).child(close("ins-tools-x", "Cancel")).child(
                        ui::primary_btn(&t, "ins-tools-restart", 30.)
                            .px(px(12.))
                            .text_size(px(12.5))
                            .child(ui::icon("relaunch", 13., t.on_accent))
                            .child("Restart and attach")
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                this.tools_pick = None;
                                let (k, sel) = (k1.clone(), sel1.clone());
                                this.app.update(cx, |s, cx| s.attach_tools(&k, sel, true, cx));
                                cx.notify();
                            })),
                    ),
                );
        } else {
            card = card
                .child(
                    ui::wrap(if running {
                        "It is running, so it restarts to load them."
                    } else {
                        "It loads them the next time it starts."
                    })
                    .text_size(px(12.))
                    .text_color(t.faint),
                )
                .child(
                    div().flex().justify_end().gap(px(6.)).child(close("ins-tools-x", "Cancel")).child(
                        ui::primary_btn(&t, "ins-tools-go", 30.)
                            .px(px(12.))
                            .text_size(px(12.5))
                            .when(!changed, |d| d.opacity(0.5))
                            .child("Attach")
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                if !changed {
                                    return;
                                }
                                if running {
                                    if let Some(p) = this.tools_pick.as_mut() {
                                        p.confirm = true;
                                    }
                                } else {
                                    this.tools_pick = None;
                                    let (k, sel) = (k2.clone(), sel2.clone());
                                    this.app.update(cx, |s, cx| s.attach_tools(&k, sel, false, cx));
                                }
                                cx.notify();
                            })),
                    ),
                );
        }
        card.into_any_element()
    }

    /// This chat's own Chrome machine, for a chat with Claude in Chrome: where Auto, Start fresh and Continue send
    /// its Chrome work. Each chat has its own; tapping the picked one again clears it (its own machine then).
    fn ins_chrome(&self, c: &Ctx, s: &Session) -> Option<AnyElement> {
        if !s.flags.chrome && s.chrome_machine.is_none() && !s.chrome_missing {
            return None;
        }
        let t = c.t;
        let cur = s.chrome_machine.clone().unwrap_or_else(|| s.machine.clone());
        let mut keys = div().flex().flex_wrap().gap(px(6.));
        for m in &c.app.snap.machines {
            let on = m.name == cur;
            let has = m.chrome.as_ref().is_some_and(|x| x.connector);
            let (machine, sid, name, label) = (s.machine.clone(), s.sid.clone(), m.name.clone(), m.label.clone());
            let own = name == s.machine;
            keys = keys.child(
                ui::seg_btn(&t, SharedString::from(format!("ins-chrome-{}", m.name)), on, false)
                    .h_auto()
                    .min_h(px(24.))
                    .py(px(3.))
                    .rounded(px(ui::R_CTRL))
                    .border_1()
                    .border_color(if on { t.accent.alpha(0.6) } else { t.line })
                    .child(ui::wrap(m.label.clone()).max_w(px(200.)).line_height(relative(1.25)))
                    .when(!has, |d| d.child(div().text_color(t.faint).child("(no extension)")))
                    .on_click(c.cx.listener(move |this, _: &ClickEvent, _, cx| {
                        let pick = if own { None } else { Some(name.clone()) };
                        let ok = format!("{label} is this chat's Chrome machine");
                        let (m2, s2) = (machine.clone(), sid.clone());
                        this.app.update(cx, |st, cx| st.set_chat_chrome(m2, s2, pick, ok, cx));
                    })),
            );
        }
        Some(
            div()
                .mt(px(16.))
                .flex()
                .flex_col()
                .gap(px(8.))
                .child(div().text_size(px(12.)).font_weight(FontWeight::SEMIBOLD).text_color(t.dim).child("Chrome machine"))
                .child(div().text_size(px(12.)).text_color(t.faint).child("This chat only. Start fresh and Continue keep it."))
                .when(s.chrome_missing, |d| {
                    d.child(
                        div()
                            .flex()
                            .items_start()
                            .gap(px(6.))
                            .child(ui::icon("warn", 13., t.need).mt(px(2.)))
                            .child(ui::wrap("Claude in Chrome did not connect: Claude Code started this chat without it, so it cannot use your browser. Starting it in a terminal (claude --chrome) connects it.").flex_1().text_size(px(12.)).line_height(relative(1.45)).text_color(t.ink2)),
                    )
                })
                .child(keys)
                .into_any_element(),
        )
    }

    /// Continue on another machine, write a handoff, and stop it to free memory (the two-step stop).
    fn ins_more(&self, c: &Ctx, s: &Session, offline: bool) -> AnyElement {
        let t = c.t;
        let cx = c.cx;
        let hover = t.hover;
        let mrow = |id: String, ic: &'static str, title: &'static str, sub: String| {
            div()
                .id(SharedString::from(id))
                .min_h(px(46.))
                .px(px(10.))
                .py(px(6.))
                .flex()
                .items_center()
                .gap(px(12.))
                .rounded(px(ui::R_CTRL))
                .cursor_pointer()
                .hover(move |st| st.bg(hover))
                .child(ui::icon(ic, 16., t.dim))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(1.))
                        .child(
                            div().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).text_color(t.ink).child(title),
                        )
                        .child(ui::wrap(sub).text_size(px(12.)).text_color(t.faint)),
                )
        };
        let (k1, k2) = (s.key.clone(), s.key.clone());
        let others = c.app.snap.other_machines(&s.machine).len();
        let mut more = div().mt(px(14.)).pt(px(8.)).border_t_1().border_color(t.line).flex().flex_col();
        if !offline && others > 0 {
            more = more.child(
                mrow(
                    format!("m-cont-{}", s.key),
                    "cont",
                    "Continue on another machine",
                    "Moves it with a handoff".into(),
                )
                .on_click(
                    cx.listener(move |this, _: &ClickEvent, _, cx| this.run_action(ActionId::Continue, k1.clone(), cx)),
                ),
            );
            if self.cont_open.as_deref() == Some(s.key.as_str()) {
                more = more.child(self.continue_menu(c, &s.key));
            }
        }
        if !offline {
            more = more.child(
                mrow(
                    format!("m-hand-{}", s.key),
                    "handoff",
                    "Write a handoff",
                    if s.handoff {
                        "A handoff exists; this writes a newer one".into()
                    } else {
                        "Saves notes without starting anything".into()
                    },
                )
                .on_click(
                    cx.listener(move |this, _: &ClickEvent, _, cx| this.run_action(ActionId::Handoff, k2.clone(), cx)),
                ),
            );
        }
        if s.alive && s.background() && !offline {
            let mb = c
                .app
                .snap
                .cleanup
                .as_ref()
                .and_then(|cl| {
                    cl.items.iter().find(|d| d.machine == s.machine && d.sid.as_deref() == Some(s.sid.as_str()))
                })
                .and_then(|d| d.mb);
            let frees = mb
                .map(|m| format!("Frees {}. You can resume it later.", fmt::mem(m)))
                .unwrap_or_else(|| "Frees its memory. You can resume it later.".into());
            let k3 = s.key.clone();
            more = more.child(
                div()
                    .min_h(px(46.))
                    .px(px(10.))
                    .py(px(6.))
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .child(ui::icon("mem", 16., t.dim))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(1.))
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(t.ink)
                                    .child("Stop it"),
                            )
                            .child(ui::wrap(frees).text_size(px(12.)).text_color(t.faint)),
                    )
                    .child(self.stop_btn(c, format!("ins-{}", s.key), "Stop", s.topic.clone(), mb, move |this, cx| {
                        let k = k3.clone();
                        this.app.update(cx, |s, cx| s.act(ActionId::Stop, &k, cx));
                    })),
            );
        }
        more.into_any_element()
    }

    fn ins_replaced(&self, c: &Ctx, r: &Replaced, o: &InsOpts) -> AnyElement {
        let t = c.t;
        let state = div()
            .flex()
            .items_center()
            .gap(px(8.))
            .child(ui::ring_lamp(&t, 8.))
            .child(ui::lbl("Replaced", t.dim))
            .into_any_element();
        let acts = acts::for_replaced(r, &c.app.snap);
        let how = r.how_text(|m| c.app.snap.label_of(m));
        let newer = r.newer_key().and_then(|k| c.app.snap.session(&k).map(|s| s.topic.clone()));
        let mut runs = vec![TextRun { text: how, bold: true }];
        if let Some(n) = newer {
            runs.push(TextRun { text: ", now ".into(), bold: false });
            runs.push(TextRun { text: n, bold: true });
        }
        runs.push(TextRun {
            text: ". It stays out of the main view so the newer chat is the one you see. Nothing was deleted.".into(),
            bold: false,
        });
        let body = div()
            .id(SharedString::from(format!("ins-body-{}", r.key)))
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&self.ins_scroll)
            .pt(px(16.))
            .px(px(16.))
            .pb(px(22.))
            .child(self.ins_title(c, &r.topic, r.tag.as_ref(), r.tag_kind))
            .child(self.ins_sub(c, &r.machine, &r.machine_label, &r.folder, r.active))
            .child(
                div()
                    .mt(px(12.))
                    .text_size(px(13.))
                    .line_height(relative(1.55))
                    .text_color(t.ink2)
                    .child(ui::runs_text(&runs, t.ink)),
            )
            .when_some(r.reaped.as_ref().filter(|_| r.reaped_note()), |d, rp| {
                d.child(
                    div().mt(px(8.)).text_size(px(13.)).line_height(relative(1.55)).text_color(t.ink2).child(rp.text()),
                )
            })
            .child(self.ins_actions(c, &r.key, &acts))
            .child(
                div()
                    .mt(px(14.))
                    .child(fact(c, "Folder", r.folder.clone(), true))
                    .child(fact(c, "Last active", fmt::when(r.active, c.now), true))
                    .child(fact(c, "Session", r.sid.chars().take(8).collect(), true)),
            );
        div()
            .size_full()
            .min_h_0()
            .flex()
            .flex_col()
            .child(self.ins_head(c, state, &r.machine, None, o))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .relative()
                    .flex()
                    .flex_col()
                    .child(body)
                    .child(ui::scrollbar(&self.ins_scroll)),
            )
            .into_any_element()
    }

    fn ins_actions(&self, c: &Ctx, key: &str, acts: &[Action]) -> impl IntoElement {
        let t = c.t;
        let lead = acts.first().filter(|a| a.primary);
        let rest = if lead.is_some() { &acts[1..] } else { acts };
        // A hover note only counts while its row is still here (a split or keep can remove the row under the mouse).
        let hovered = self.hint.clone().filter(|h| acts.iter().any(|a| a.note.as_str() == h.as_ref()));
        let hint: SharedString =
            hovered.unwrap_or_else(|| lead.map(|a| SharedString::from(a.note.clone())).unwrap_or_default());
        let mut list = div().mt(px(16.)).flex().flex_col().gap(px(1.));
        if let Some(a) = lead {
            list = list.child(self.action_row(c, key, a, true));
        }
        for a in rest {
            list = list.child(self.action_row(c, key, a, false));
            if a.id == ActionId::Continue && self.cont_open.as_deref() == Some(key) {
                list = list.child(self.continue_menu(c, key));
            }
        }
        list.child(
            div()
                .min_h(px(34.))
                .mt(px(6.))
                .px(px(9.))
                .text_size(px(12.))
                .line_height(relative(1.4))
                .text_color(t.faint)
                .child(hint),
        )
    }

    fn action_row(&self, c: &Ctx, key: &str, a: &Action, lead: bool) -> AnyElement {
        let t = c.t;
        let id = a.id;
        let k = key.to_string();
        let note = SharedString::from(a.note.clone());
        let verb = if id == ActionId::Continue { Some("continue") } else { id.verb() };
        let busy = verb.is_some_and(|v| c.app.busy.contains(&format!("{key}:{v}")));
        let cap = a.key.as_deref().and_then(keycap).filter(|_| !a.disabled);
        let el_id = SharedString::from(format!("act-{key}-{id:?}"));
        let open_cont = id == ActionId::Continue && self.cont_open.as_deref() == Some(key);
        let row = if lead {
            ui::primary_btn(&t, el_id, 34.)
                .mb(px(8.))
                .pl(px(11.))
                .pr(px(8.))
                .gap(px(9.))
                .child(ui::icon(&a.icon, 15., t.on_accent))
                .child(ui::line(a.title.clone()).flex_1())
                .when_some(cap, |d, k| d.child(ui::kbd_in(k, t.on_accent.alpha(0.75), t.on_accent.alpha(0.35))))
        } else {
            let ink: Hsla = if a.disabled {
                t.faint
            } else if a.danger {
                t.bad
            } else if open_cont {
                t.ink
            } else {
                t.ink2
            };
            let ic = if a.danger {
                t.bad
            } else if open_cont {
                t.ink2
            } else {
                t.faint
            };
            let hover = t.hover;
            div()
                .id(el_id)
                .h(px(30.))
                .pl(px(9.))
                .pr(px(6.))
                .flex()
                .items_center()
                .gap(px(10.))
                .rounded(px(ui::R_ITEM))
                .text_size(px(13.))
                .text_color(ink)
                .when(open_cont, |d| d.bg(t.press))
                .when(!a.disabled, |d| d.cursor_pointer().hover(move |s| s.bg(hover)))
                .child(ui::icon(&a.icon, 15., ic))
                .child(ui::line(a.title.clone()).flex_1())
                .when_some(cap, |d, k| d.child(ui::kbd(&t, k)))
        };
        let disabled = a.disabled;
        row.when(busy, |d| d.opacity(0.55))
            .on_hover(c.cx.listener(move |this, h: &bool, _, cx| {
                if *h {
                    this.hint = Some(note.clone());
                } else if this.hint.as_ref() == Some(&note) {
                    this.hint = None;
                }
                cx.notify();
            }))
            .on_click(c.cx.listener(move |this, _: &ClickEvent, _, cx| {
                if !disabled && !busy {
                    this.run_action(id, k.clone(), cx);
                }
            }))
            .into_any_element()
    }

    /// Continue on another machine: the choices open inline under the action.
    fn continue_menu(&self, c: &Ctx, key: &str) -> impl IntoElement {
        let t = c.t;
        let Some(s) = c.app.snap.session(key) else { return div() };
        let others = c.app.snap.other_machines(&s.machine);
        let press = t.press;
        let mut menu = div()
            .ml(px(34.))
            .mt(px(2.))
            .mb(px(6.))
            .py(px(6.))
            .px(px(4.))
            .rounded(px(ui::R_CTRL))
            .bg(t.panel2)
            .shadow(vec![crate::theme::Theme::ring(t.line)])
            .flex()
            .flex_col()
            .child(ui::lbl("Continue on", t.label).px(px(8.)).pt(px(2.)).pb(px(6.)));
        if others.is_empty() {
            menu = menu.child(
                div().px(px(8.)).pb(px(2.)).text_size(px(12.)).text_color(t.dim).child("No other machine is online."),
            );
        }
        for m in others {
            let (k, to) = (key.to_string(), m.name.clone());
            menu = menu.child(
                div()
                    .id(SharedString::from(format!("cont-{key}-{}", m.name)))
                    .h(px(30.))
                    .px(px(8.))
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .rounded(px(ui::R_ITEM))
                    .cursor_pointer()
                    .text_size(px(13.))
                    .text_color(t.ink)
                    .hover(move |st| st.bg(press))
                    .child(ui::machine_icon(&m.name, 15., t.dim))
                    .child(m.label.clone())
                    .on_click(c.cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.cont_open = None;
                        let (k, to) = (k.clone(), to.clone());
                        this.app.update(cx, |s, cx| s.continue_on(&k, &to, cx));
                        cx.notify();
                    })),
            );
        }
        menu.child(
            div()
                .px(px(8.))
                .pt(px(6.))
                .text_size(px(12.))
                .line_height(relative(1.4))
                .text_color(t.dim)
                .child(format!("Writes a handoff on {}, then starts it there.", s.machine_label)),
        )
    }

    /// v2: History, the earlier chats on this topic, newest first: when, what, where, how big and how it ended, with
    /// Open (its Claude link) or View (it is still on the list here).
    fn ins_history(&self, c: &Ctx, s: &Session) -> Option<AnyElement> {
        if s.history == 0 {
            return None;
        }
        let t = c.t;
        let tv = c.app.threads.get(&s.key);
        let chats: &[ApiThreadChat] = tv.map(|v| v.chats.as_slice()).unwrap_or(&[]);
        let earlier: Vec<usize> = match chats.iter().position(|x| x.this == Some(true)) {
            Some(i) => (0..i).rev().collect(),
            None => (0..chats.len()).filter(|&i| chats[i].how.as_deref() != Some("current")).rev().collect(),
        };
        let n = if earlier.is_empty() { s.history as usize } else { earlier.len() };
        let first = earlier.last().and_then(|&i| chats[i].started).filter(|v| *v > 0.0).map(fmt::md);
        let sum = match (n, first) {
            (1, Some(f)) => format!("One earlier chat, from {f}"),
            (1, None) => "One earlier chat".to_string(),
            (n, Some(f)) => format!("{n} earlier chats since {f}"),
            (n, None) => format!("{n} earlier chats"),
        };
        let mut list = div().mt(px(8.)).flex().flex_col();
        if earlier.is_empty() {
            list = list.child(match tv {
                Some(v) if !v.loading && v.failed.is_some() => div()
                    .py(px(6.))
                    .text_size(px(12.))
                    .text_color(t.faint)
                    .child(format!("Could not load it: {}", v.failed.clone().unwrap_or_default())),
                _ => ui::loading(&t, "ins-hist-load", "Loading the thread").py(px(6.)).text_size(px(12.)),
            });
        }
        for i in earlier {
            list = list.child(self.history_row(c, chats, i));
        }
        Some(
            div()
                .mt(px(14.))
                .pt(px(14.))
                .border_t_1()
                .border_color(t.rule)
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(7.))
                        .child(ui::icon("history", 13., t.label))
                        .child(ui::lbl("History", t.label)),
                )
                .child(div().mt(px(8.)).text_size(px(13.)).text_color(t.ink2).child(sum))
                .child(list)
                .into_any_element(),
        )
    }

    fn history_row(&self, c: &Ctx, chats: &[ApiThreadChat], i: usize) -> AnyElement {
        let t = c.t;
        let x = &chats[i];
        let next = chats.get(i + 1);
        let label = x.label.clone().or_else(|| c.app.snap.label_of(&x.machine)).unwrap_or_else(|| x.machine.clone());
        let how = match x.how.as_deref() {
            Some("fresh") => "Started fresh".to_string(),
            Some("resumed") => "Resumed in a newer copy".to_string(),
            Some("handoff") => "Handed off".to_string(),
            Some("compacted") => "Compacted with its thread into a fresh chat".to_string(),
            Some("continued") => match next {
                Some(n) => format!(
                    "Continued on {}",
                    n.label.clone().or_else(|| c.app.snap.label_of(&n.machine)).unwrap_or_else(|| n.machine.clone())
                ),
                None => "Continued on another machine".to_string(),
            },
            Some("linked") => "Linked by you".to_string(),
            _ => "Picked up in a newer chat".to_string(),
        };
        let ctx = x.ctx.filter(|v| *v > 0.0).map(|v| fmt::k(v as u64));
        let sub = [Some(label), ctx, Some(how)].into_iter().flatten().collect::<Vec<_>>().join(" · ");
        let raw = crate::link::first_line(x.title.as_deref(), "Untitled session").to_string();
        let (topic, tag) = names::split(&raw);
        let kind = names::tag_kind(tag.as_deref());
        let when = fmt::md(x.started.or(x.ended).unwrap_or(0.0));
        let key = format!("{}:{}", x.machine, x.sid);
        let on_list = c.app.snap.session(&key).is_some() || c.app.snap.replaced_row(&key).is_some();
        let url = x.url.clone().filter(|u| !u.is_empty());
        let (go, go_icon) = if url.is_some() {
            ("Open", "out")
        } else if on_list {
            ("View", "chev")
        } else {
            ("", "")
        };
        let hover = t.hover;
        let clickable = url.is_some() || on_list;
        div()
            .id(SharedString::from(format!("hist-{key}")))
            .mx(px(-6.))
            .px(px(6.))
            .py(px(8.))
            .flex()
            .items_start()
            .gap(px(10.))
            .rounded(px(ui::R_ITEM))
            .border_t_1()
            .border_color(t.rule)
            .when(clickable, |d| d.cursor_pointer().hover(move |st| st.bg(hover)))
            .child(
                div()
                    .flex_none()
                    .w(px(40.))
                    .pt(px(1.))
                    .font_family(NUM)
                    .text_size(px(11.5))
                    .text_color(t.dim)
                    .child(when),
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
                            .items_center()
                            .gap(px(6.))
                            .min_w_0()
                            .child(
                                ui::wrap(topic)
                                    .flex_shrink(1.)
                                    .text_size(px(12.5))
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(t.ink),
                            )
                            .when_some(tag, |d, tg| d.child(ui::name_tag(&t, &tg, kind))),
                    )
                    .child(div().text_size(px(11.5)).line_height(relative(1.4)).text_color(t.dim).child(sub)),
            )
            .when(clickable, |d| {
                d.child(
                    div()
                        .flex_none()
                        .pt(px(1.))
                        .flex()
                        .items_center()
                        .gap(px(3.))
                        .text_size(px(12.))
                        .font_weight(FontWeight::BOLD)
                        .text_color(t.accent)
                        .child(go)
                        .child(ui::icon(go_icon, 12., t.accent)),
                )
            })
            .on_click(c.cx.listener(move |this, _: &ClickEvent, _, cx| match &url {
                Some(u) => crate::state::open_link(cx, u),
                None if on_list => this.select(Some(key.clone()), cx),
                None => {}
            }))
            .into_any_element()
    }
}

/// One fact row: a label on the left, the value on the right, a hairline above.
fn fact(c: &Ctx, k: &str, v: String, mono: bool) -> impl IntoElement {
    let t = c.t;
    div()
        .h(px(30.))
        .flex()
        .items_center()
        .justify_between()
        .gap(px(12.))
        .border_t_1()
        .border_color(t.rule)
        .child(div().flex_none().text_size(px(12.)).text_color(t.dim).child(k.to_string()))
        .child(
            ui::line(v)
                .text_color(t.ink2)
                .when(mono, |d| d.font_family(NUM).text_size(px(12.)))
                .when(!mono, |d| d.text_size(px(12.5))),
        )
}
