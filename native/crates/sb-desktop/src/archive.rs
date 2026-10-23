//! The Archive sheet: Older (idle two days or more) and Replaced (started fresh, resumed or continued elsewhere),
//! with the inspector on the right. Nothing here is deleted; it is a quiet place you can reach, never in the way.

use gpui::{
    AnyElement, ClickEvent, FontWeight, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, div, prelude::*, px,
};
use sb_core::fmt;

use crate::inspector::InsOpts;
use crate::state::AppState;
use crate::ui;
use crate::view::{BoardView, Ctx, Overlay};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ArcTab {
    Older,
    Replaced,
}

/// The keys the tab lists, newest first.
pub fn rows(app: &AppState, tab: ArcTab) -> Vec<String> {
    match tab {
        ArcTab::Older => app.older().into_iter().map(|s| s.key.clone()).collect(),
        ArcTab::Replaced => app.replaced().into_iter().map(|r| r.key.clone()).collect(),
    }
}

impl BoardView {
    pub fn archive(&self, c: &Ctx, tab: ArcTab, key: Option<String>, h: f32) -> AnyElement {
        let t = c.t;
        let cx = c.cx;
        let app = c.app;
        let list = rows(app, tab);
        let key = key.filter(|k| list.contains(k)).or_else(|| list.first().cloned());
        let sheet_h = 720f32.min(h - ui::header() - 24.).max(300.);
        let (n_old, n_rep) = (app.older().len(), app.replaced().len());

        let tabs = ui::seg_box(&t)
            .child(ui::seg_btn(&t, "arc-older", tab == ArcTab::Older, false).child(format!("Older {n_old}")).on_click(
                cx.listener(|this, _: &ClickEvent, window, cx| this.open_archive(ArcTab::Older, None, window, cx)),
            ))
            .child(
                ui::seg_btn(&t, "arc-rep", tab == ArcTab::Replaced, false).child(format!("Replaced {n_rep}")).on_click(
                    cx.listener(|this, _: &ClickEvent, window, cx| {
                        this.open_archive(ArcTab::Replaced, None, window, cx)
                    }),
                ),
            );
        let note = match tab {
            ArcTab::Older => "Idle for two days or more. They're left out of every view and count.",
            ArcTab::Replaced => {
                "Started fresh, resumed or continued elsewhere. Only the newer chat shows in the main view."
            }
        };
        // Only the rows on screen are drawn (a uniform list of 40 px rows), so a long archive scrolls as fast as a
        // short one.
        let hover = t.hover;
        let now = c.now;
        let items: Vec<(String, Option<gpui::Hsla>, String, String)> = list
            .iter()
            .filter_map(|k| {
                if let Some(s) = app.snap.session(k) {
                    let l = s.lamp(now);
                    Some((
                        k.clone(),
                        Some(t.lamp(l)),
                        s.topic.clone(),
                        format!("{}, {}", s.machine_label, fmt::sago(s.active, now)),
                    ))
                } else {
                    app.snap
                        .replaced_row(k)
                        .map(|r| (k.clone(), None, r.topic.clone(), r.how_text(|m| app.snap.label_of(m))))
                }
            })
            .collect();
        let rows_el = if items.is_empty() {
            div()
                .flex_1()
                .min_h_0()
                .px(px(8.))
                .child(
                    div()
                        .px(px(12.))
                        .py(px(18.))
                        .text_size(px(13.))
                        .text_color(t.faint)
                        .child(if tab == ArcTab::Older { "No older chats." } else { "Nothing was replaced." }),
                )
                .into_any_element()
        } else {
            let sel = key.clone();
            let n = items.len();
            let items = std::rc::Rc::new(items);
            let list = gpui::uniform_list(
                "arc-list",
                n,
                cx.processor(move |_, range: std::ops::Range<usize>, _, cx| {
                    range
                        .filter_map(|i| items.get(i).cloned())
                        .map(|(k, lamp, topic, meta)| {
                            let on = sel.as_deref() == Some(k.as_str());
                            let kk = k.clone();
                            div()
                                .id(SharedString::from(format!("arc-{k}")))
                                .h(px(40.))
                                .px(px(10.))
                                .flex()
                                .items_center()
                                .gap(px(10.))
                                .rounded(px(ui::R_ITEM))
                                .cursor_pointer()
                                .text_color(if on { t.ink } else { t.ink2 })
                                .when(on, |d| d.bg(t.sel))
                                .when(!on, |d| d.hover(move |s| s.bg(hover)))
                                .on_click(cx.listener(move |this, ev: &ClickEvent, _, cx| {
                                    if let Overlay::Archive { key, .. } = &mut this.overlay {
                                        *key = Some(kk.clone());
                                    }
                                    this.cont_open = None;
                                    this.hint = None;
                                    if ev.click_count() >= 2 {
                                        let k = kk.clone();
                                        this.app.update(cx, |s, cx| s.open(&k, cx));
                                    }
                                    cx.notify();
                                }))
                                .child(match lamp {
                                    Some(color) => div()
                                        .flex_none()
                                        .size(px(8.))
                                        .rounded_full()
                                        .border_1()
                                        .border_color(color)
                                        .into_any_element(),
                                    None => ui::ring_lamp(&t, 8.).into_any_element(),
                                })
                                .child(ui::line(topic).flex_1().text_size(px(13.)))
                                .child(
                                    div()
                                        .flex_none()
                                        .text_size(px(11.5))
                                        .text_color(t.faint)
                                        .whitespace_nowrap()
                                        .child(meta),
                                )
                        })
                        .collect::<Vec<_>>()
                }),
            )
            .track_scroll(&self.arc_scroll)
            .size_full()
            .px(px(8.))
            .pb(px(12.));
            div().relative().flex_1().min_h_0().child(list).child(ui::scrollbar(&self.arc_scroll)).into_any_element()
        };
        let ins = match &key {
            Some(k) => self.inspector(c, k, InsOpts { close: false, radius: 0., detached: false }),
            None => div().size_full().bg(t.panel).into_any_element(),
        };
        div()
            .h(px(sheet_h))
            .flex()
            .flex_col()
            .child(
                div()
                    .flex_none()
                    .pt(px(16.))
                    .pr(px(12.))
                    .pb(px(10.))
                    .pl(px(22.))
                    .flex()
                    .items_start()
                    .gap(px(12.))
                    .child(
                        div()
                            .flex_1()
                            .flex()
                            .flex_col()
                            .gap(px(4.))
                            .child(div().text_size(px(15.)).font_weight(FontWeight::BOLD).text_color(t.ink).child("Archive"))
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(t.faint)
                                    .child("Nothing here was deleted. Chats come back on their own when Claude replies there again."),
                            ),
                    )
                    .child(ui::icon_btn(&t, "arc-close", "x", 26.).on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.close_overlay(window, cx)))),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .border_t_1()
                    .border_color(t.rule)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(div().flex_none().pt(px(10.)).px(px(14.)).pb(px(8.)).flex().child(tabs))
                            .child(div().flex_none().px(px(16.)).pb(px(8.)).text_size(px(12.)).text_color(t.faint).child(note))
                            .child(rows_el),
                    )
                    .child(div().flex_none().w(px(320.)).h_full().min_h_0().border_l_1().border_color(t.rule).child(ins)),
            )
            .into_any_element()
    }
}
