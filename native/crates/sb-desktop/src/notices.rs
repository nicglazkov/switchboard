//! The Notices tray: toasts, job cards and setup notices that left the bottom-right corner (their X, or their time
//! ran out) wait here, behind "Notices" at the right of the status bar, until they are acted on or ignored. Ignore and
//! Clear all each ask once more ("Confirm") before anything is gone.

use gpui::{AnyElement, ClickEvent, FontWeight, InteractiveElement, IntoElement, MouseButton, ParentElement, SharedString, StatefulInteractiveElement, Styled, div, prelude::*, px, relative};
use sb_core::fmt;

use crate::state::{NoticeAct, Tone};
use crate::theme::Theme;
use crate::ui;
use crate::view::{BoardView, Ctx};

/// One row of the tray, whatever it came from.
struct Row {
    key: String,
    tone: Tone,
    title: String,
    sub: Option<String>,
    at: Option<f64>,
    new: bool,
    act: Option<(&'static str, RowAct)>,
    ignore: RowIgnore,
}

#[derive(Clone)]
enum RowAct {
    Notice(u64),
    Fix { machine: String, id: String, fix: String },
}

#[derive(Clone)]
enum RowIgnore {
    Notice(u64),
    Setup { machine: String, id: String },
}

impl BoardView {
    /// "Notices 3" at the right of the status bar; only while the tray holds something.
    pub fn notices_link(&self, c: &Ctx) -> Option<AnyElement> {
        let t = c.t;
        let (n, new) = c.app.tray_count();
        if n == 0 {
            return None;
        }
        Some(
            ui::ghost(&t, "sb-notices", None, None)
                .h(px(24.))
                .px(px(8.))
                .gap(px(6.))
                .text_size(px(12.5))
                .text_color(if new > 0 { t.ink } else { t.dim })
                .when(self.notices_open, |d| d.bg(t.press))
                .child(ui::icon("down", 13., if new > 0 { t.accent } else { t.faint }))
                .child("Notices")
                .child(
                    div()
                        .min_w(px(18.))
                        .h(px(16.))
                        .px(px(5.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded_full()
                        .text_size(px(11.))
                        .font_weight(FontWeight::BOLD)
                        .when(new > 0, |d| d.bg(t.accent).text_color(gpui::white()))
                        .when(new == 0, |d| d.bg(t.panel).text_color(t.dim))
                        .child(if new > 0 { new.to_string() } else { n.to_string() }),
                )
                .on_click(c.cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.notices_open = !this.notices_open;
                    this.ignore_armed = None;
                    if this.notices_open {
                        this.app.update(cx, |s, cx| s.tray_seen(cx));
                    }
                    cx.notify();
                }))
                .into_any_element(),
        )
    }

    fn tray_rows(&self, c: &Ctx) -> Vec<Row> {
        let multi = c.app.snap.machines.len() > 1;
        let mut rows: Vec<Row> = c
            .app
            .setup_parked()
            .into_iter()
            .map(|(m, label, i)| Row {
                key: format!("s:{m}:{}", i.id),
                tone: if i.tone.as_deref() == Some("bad") { Tone::Bad } else { Tone::Info },
                title: if multi { format!("{label}: {}", i.title) } else { i.title.clone() },
                sub: Some(i.text.clone()),
                at: None,
                new: false,
                act: i.fix.clone().map(|fix| ("Fix", RowAct::Fix { machine: m.clone(), id: i.id.clone(), fix })),
                ignore: RowIgnore::Setup { machine: m, id: i.id },
            })
            .collect();
        rows.extend(c.app.settings.notices.iter().map(|n| Row {
            key: format!("n:{}", n.id),
            tone: n.tone,
            title: n.title.clone(),
            sub: n.sub.clone(),
            at: Some(n.at),
            new: n.new,
            act: n.act.as_ref().map(|a| (a.label(), RowAct::Notice(n.id))),
            ignore: RowIgnore::Notice(n.id),
        }));
        rows
    }

    /// The tray, above "Notices" in the bottom-right corner.
    pub fn notices_popover(&self, c: &Ctx) -> Option<AnyElement> {
        if !self.notices_open {
            return None;
        }
        let t = c.t;
        let cx = c.cx;
        let rows = self.tray_rows(c);
        let n = rows.len();
        let clear_armed = self.ignore_armed.as_deref() == Some("all");
        let head = div()
            .pl(px(16.))
            .pr(px(8.))
            .h(px(44.))
            .flex()
            .items_center()
            .gap(px(8.))
            .border_b_1()
            .border_color(t.rule)
            .child(div().text_size(px(13.)).font_weight(FontWeight::BOLD).text_color(t.ink).child("Notices"))
            .child(div().text_size(px(12.)).text_color(t.faint).child(n.to_string()))
            .child(ui::spacer())
            .when(n > 0, |d| {
                d.child(if clear_armed {
                    ui::line_btn(&t, "tray-clear-yes", 26.)
                        .px(px(10.))
                        .text_size(px(12.5))
                        .text_color(t.bad)
                        .child("Confirm clear all")
                        .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                            if this.ignore_armed.as_deref() == Some("all") {
                                this.ignore_armed = None;
                                cx.notify();
                            }
                        }))
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.ignore_armed = None;
                            this.app.update(cx, |s, cx| s.clear_tray(cx));
                        }))
                        .into_any_element()
                } else {
                    ui::ghost(&t, "tray-clear", None, None)
                        .h(px(26.))
                        .px(px(10.))
                        .text_size(px(12.5))
                        .text_color(t.dim)
                        .child("Clear all")
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.ignore_armed = Some("all".into());
                            cx.notify();
                        }))
                        .into_any_element()
                })
            })
            .child(ui::icon_btn(&t, "tray-x", "x", 24.).on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.notices_open = false;
                this.ignore_armed = None;
                cx.notify();
            })));
        let mut list = div().id("tray-list").max_h(px(460.)).overflow_y_scroll().py(px(6.)).flex().flex_col();
        if rows.is_empty() {
            list = list.child(
                div()
                    .px(px(16.))
                    .py(px(18.))
                    .text_size(px(12.5))
                    .text_color(t.faint)
                    .child("Nothing here. What you close in the corner waits here until you ignore it."),
            );
        }
        for r in rows {
            list = list.child(self.tray_row(c, r));
        }
        Some(
            div()
                .absolute()
                .inset_0()
                .occlude()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, _, cx| {
                        this.notices_open = false;
                        this.ignore_armed = None;
                        cx.notify();
                    }),
                )
                .child(
                    div()
                        .id("tray-pop")
                        .absolute()
                        .bottom(px(ui::FOOTER + 8.))
                        .right(px(12.))
                        .w(px(440.))
                        .rounded(px(ui::R_POP))
                        .border_1()
                        .border_color(t.line2)
                        .bg(t.panel2)
                        .shadow(t.drop())
                        .shadow(vec![Theme::ring(t.line2)])
                        .overflow_hidden()
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .child(head)
                        .child(list),
                )
                .into_any_element(),
        )
    }

    fn tray_row(&self, c: &Ctx, r: Row) -> AnyElement {
        let t = c.t;
        let cx = c.cx;
        let (icon, ic) = match r.tone {
            Tone::Bad => ("warn", t.bad),
            Tone::Ok => ("check", t.warm),
            Tone::Info => ("info", t.dim),
        };
        let armed = self.ignore_armed.as_deref() == Some(r.key.as_str());
        let fixing = matches!(&r.act, Some((_, RowAct::Fix { machine, id, .. })) if c.app.busy.contains(&format!("setup:{machine}:{id}")));
        let meta = r.at.map(|a| format!("{} ago", fmt::sago(a, c.now)));
        let key = r.key.clone();
        let ignore = r.ignore.clone();
        div()
            .id(SharedString::from(format!("tray-{}", r.key)))
            .mx(px(6.))
            .px(px(10.))
            .py(px(9.))
            .flex()
            .items_start()
            .gap(px(10.))
            .rounded(px(ui::R_CTRL))
            .when(r.new, |d| d.bg(t.accent.alpha(if t.dark { 0.08 } else { 0.05 })))
            .child(ui::icon(icon, 15., ic).mt(px(1.)))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(3.))
                    .child(ui::wrap(r.title.clone()).text_size(px(12.5)).font_weight(FontWeight::BOLD).text_color(if r.tone == Tone::Bad { t.bad } else { t.ink }))
                    .when_some(r.sub.clone(), |d, s| d.child(ui::wrap(s).text_size(px(12.)).line_height(relative(1.4)).text_color(t.dim)))
                    .when_some(meta, |d, m| d.child(div().text_size(px(11.5)).text_color(t.faint).child(m)))
                    .child(
                        div()
                            .mt(px(4.))
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .when_some(r.act.clone(), |d, (label, act)| {
                                d.child(
                                    ui::line_btn(&t, SharedString::from(format!("tray-act-{}", r.key)), 26.)
                                        .px(px(10.))
                                        .text_size(px(12.5))
                                        .when(fixing, |d| d.opacity(0.6))
                                        .child(if fixing { "Fixing" } else { label })
                                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                            if fixing {
                                                return;
                                            }
                                            match act.clone() {
                                                RowAct::Fix { machine, id, fix } => this.app.update(cx, |s, cx| s.fix_setup(&machine, &id, &fix, cx)),
                                                RowAct::Notice(id) => {
                                                    if let Some(NoticeAct::Terminal { key }) = this.app.update(cx, |s, cx| s.notice_act(id, cx)) {
                                                        this.notices_open = false;
                                                        this.open_terminal(key, false, cx);
                                                    }
                                                }
                                            }
                                            cx.notify();
                                        })),
                                )
                            })
                            .child(if armed {
                                ui::line_btn(&t, SharedString::from(format!("tray-ign-yes-{}", r.key)), 26.)
                                    .px(px(10.))
                                    .text_size(px(12.5))
                                    .text_color(t.bad)
                                    .child("Confirm ignore")
                                    .on_mouse_down_out(cx.listener(move |this, _, _, cx| {
                                        if this.ignore_armed.as_deref() == Some(key.as_str()) {
                                            this.ignore_armed = None;
                                            cx.notify();
                                        }
                                    }))
                                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                        this.ignore_armed = None;
                                        match ignore.clone() {
                                            RowIgnore::Notice(id) => this.app.update(cx, |s, cx| s.ignore_notice(id, cx)),
                                            RowIgnore::Setup { machine, id } => this.app.update(cx, |s, cx| s.close_setup(&machine, &id, cx)),
                                        }
                                    }))
                                    .into_any_element()
                            } else {
                                let k = r.key.clone();
                                ui::ghost(&t, SharedString::from(format!("tray-ign-{}", r.key)), None, None)
                                    .h(px(26.))
                                    .px(px(10.))
                                    .text_size(px(12.5))
                                    .text_color(t.dim)
                                    .child("Ignore")
                                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                        this.ignore_armed = Some(k.clone());
                                        cx.notify();
                                    }))
                                    .into_any_element()
                            }),
                    ),
            )
            .into_any_element()
    }
}
