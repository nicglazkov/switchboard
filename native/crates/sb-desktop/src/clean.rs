//! v2, "Stopped today": the popover under its pill on the board (lanes.rs) with what was stopped today (Resume) and
//! the chats put off with "Not now" on the not-in-use strip (Stop, Keep, or back on the board). Switchboard never
//! stops anything on its own.

use gpui::{
    AnyElement, ClickEvent, InteractiveElement, IntoElement, MouseButton, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, div, prelude::*, px, relative,
};
use sb_core::api::{ApiCleanItem, ApiCleanStopped};
use sb_core::{fmt, names};

use crate::state::clean_key;
use crate::theme::NUM;
use crate::ui;
use crate::view::{BoardView, Ctx};

impl BoardView {
    /// What the status bar's "Stopped today" holds: the chats stopped today, and those put off with "Not now".
    fn clean_parts(&self, c: &Ctx) -> (Vec<ApiCleanItem>, Vec<ApiCleanStopped>) {
        let Some(cl) = c.app.snap.cleanup.as_ref() else { return (Vec::new(), Vec::new()) };
        let mine = |m: &str| self.solo.as_deref().is_none_or(|s| s == m);
        let items: Vec<ApiCleanItem> = cl.items.iter().filter(|d| mine(&d.machine)).cloned().collect();
        let all_later = !items.is_empty() && items.iter().all(|d| c.app.clean_later.contains(&clean_key(d)));
        let later = if all_later { items } else { Vec::new() };
        (later, cl.stopped.iter().filter(|d| mine(&d.machine)).cloned().collect())
    }

    /// The popover under "Stopped today": put-off chats (Stop, Keep, or back on the board) and the ones stopped today
    /// (Resume).
    pub fn stopped_popover(&self, c: &Ctx) -> Option<AnyElement> {
        let t = c.t;
        let cx = c.cx;
        let (later, stopped) = self.clean_parts(c);
        if later.is_empty() && stopped.is_empty() {
            return None;
        }
        let keys: Vec<String> = later.iter().map(clean_key).collect();
        let mut body = div().id("stopped-list").max_h(px(420.)).overflow_y_scroll().pb(px(6.)).flex().flex_col();
        if !later.is_empty() {
            body = body.child(
                div()
                    .pl(px(20.))
                    .pr(px(12.))
                    .pt(px(12.))
                    .pb(px(4.))
                    .flex()
                    .items_center()
                    .child(ui::lbl("Not in use", t.faint))
                    .child(div().flex_1())
                    .child(
                        ui::ghost(&t, "cln-unfold", None, None).h(px(22.)).px(px(8.)).text_size(px(12.)).child("Show on the board").on_click(cx.listener(
                            move |this, _: &ClickEvent, _, cx| {
                                this.stopped_open = false;
                                let k = keys.clone();
                                this.app.update(cx, |s, cx| s.clean_fold(k, false, cx));
                            },
                        )),
                    ),
            );
            for d in later {
                body = body.child(self.clean_row(c, d));
            }
        }
        if !stopped.is_empty() {
            body = body.child(div().pl(px(20.)).pr(px(14.)).pt(px(12.)).pb(px(4.)).child(ui::lbl("Stopped today", t.faint)));
            for d in stopped {
                body = body.child(self.clean_done_row(c, d));
            }
        }
        Some(
            div()
                .absolute()
                .inset_0()
                .occlude()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, _, cx| {
                        this.stopped_open = false;
                        cx.notify();
                    }),
                )
                .child(
                    div()
                        .id("stopped-pop")
                        .absolute()
                        .top(px(ui::header() + 50.))
                        .right(px(14.))
                        .w(px(460.))
                        .rounded(px(ui::R_POP))
                        .border_1()
                        .border_color(t.line2)
                        .bg(t.panel2)
                        .shadow(t.drop())
                        .overflow_hidden()
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .child(body),
                )
                .into_any_element(),
        )
    }

    fn clean_row(&self, c: &Ctx, d: ApiCleanItem) -> AnyElement {
        let t = c.t;
        let cx = c.cx;
        let k = clean_key(&d);
        let busy = c.app.clean_busy.contains(&k);
        let title = match (&d.sid, &d.title) {
            (None, t) => t.clone().unwrap_or_else(|| "Leftover Claude process".into()),
            (Some(_), Some(t)) => names::split(t).0.to_string(),
            (Some(_), None) => "Untitled".into(),
        };
        let board_key = d.sid.as_ref().map(|s| format!("{}:{s}", d.machine)).filter(|k| c.app.snap.session(k).is_some());
        let meta = format!("{} · {}", d.label.clone().unwrap_or_else(|| d.machine.clone()), d.why.clone().unwrap_or_default());
        let hover = t.hover;
        let (keep_d, stop_d) = (d.clone(), d.clone());
        div()
            .id(SharedString::from(format!("cln-{k}")))
            .min_h(px(40.))
            .mx(px(6.))
            .px(px(8.))
            .flex()
            .items_center()
            .gap(px(10.))
            .rounded(px(ui::R_CTRL))
            .when(busy, |e| e.opacity(0.6))
            .child(
                div()
                    .id(SharedString::from(format!("cln-n-{k}")))
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .px(px(6.))
                    .py(px(3.))
                    .rounded(px(ui::R_ITEM))
                    .when_some(board_key, |e, bk| {
                        e.cursor_pointer().hover(move |s| s.bg(hover)).on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.select(Some(bk.clone()), cx)))
                    })
                    .child(ui::wrap(title).text_size(px(13.)).text_color(t.ink).line_height(relative(1.3)))
                    .child(ui::wrap(meta).text_size(px(11.5)).text_color(t.faint)),
            )
            .child(div().flex_none().w(px(64.)).flex().justify_end().font_family(NUM).text_size(px(12.)).text_color(t.dim).children(d.mb.map(fmt::mem)))
            .child(
                ui::ghost(&t, SharedString::from(format!("cln-k-{k}")), None, None)
                    .h(px(26.))
                    .px(px(10.))
                    .text_size(px(12.5))
                    .child("Keep")
                    .when(!busy, |e| e.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        let d = keep_d.clone();
                        this.app.update(cx, |s, cx| s.clean_keep(d, cx));
                    }))),
            )
            .child(
                ui::line_btn(&t, SharedString::from(format!("cln-s-{k}")), 26.)
                    .px(px(11.))
                    .text_size(px(12.5))
                    .text_color(t.bad)
                    .child(if busy { "Stopping" } else { "Stop" })
                    .when(!busy, |e| e.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        let d = stop_d.clone();
                        this.app.update(cx, |s, cx| s.clean_stop(vec![d], cx));
                    }))),
            )
            .into_any_element()
    }

    fn clean_done_row(&self, c: &Ctx, d: ApiCleanStopped) -> AnyElement {
        let t = c.t;
        let cx = c.cx;
        let title = d.title.as_deref().map(|x| names::split(x).0.to_string()).unwrap_or_else(|| "Untitled".into());
        let mut meta = format!("{} · stopped {} ago", d.label.clone().unwrap_or_else(|| d.machine.clone()), fmt::sago(d.at.unwrap_or(c.now), c.now));
        if let Some(mb) = d.mb.filter(|m| *m > 0.0) {
            meta.push_str(&format!(", freed {}", fmt::mem(mb)));
        }
        let busy = c.app.busy.contains(&format!("{}:{}:resume", d.machine, d.sid));
        let (m, sid, tt) = (d.machine.clone(), d.sid.clone(), d.title.clone());
        div()
            .min_h(px(40.))
            .mx(px(6.))
            .px(px(8.))
            .flex()
            .items_center()
            .gap(px(10.))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .px(px(6.))
                    .py(px(3.))
                    .child(ui::wrap(title).text_size(px(13.)).text_color(t.ink2).line_height(relative(1.3)))
                    .child(ui::wrap(meta).text_size(px(11.5)).text_color(t.faint)),
            )
            .child(
                ui::line_btn(&t, SharedString::from(format!("cln-r-{}", d.sid)), 26.)
                    .px(px(11.))
                    .text_size(px(12.5))
                    .child(ui::icon("resume", 13., t.dim))
                    .child(if busy { "Resuming" } else { "Resume" })
                    .when(!busy, |e| e.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        let (m, sid, tt) = (m.clone(), sid.clone(), tt.clone());
                        this.app.update(cx, |s, cx| s.resume_chat(&m, &sid, tt, cx));
                    }))),
            )
            .into_any_element()
    }
}
