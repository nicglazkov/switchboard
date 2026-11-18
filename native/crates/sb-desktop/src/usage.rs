//! The usage panel under the Week chip, top to bottom: every limit of the plan with when it resets, extra usage,
//! the week by product, this week by day and the last four weeks, the chats that used the most this week, and
//! the budget. The limits and the chip read the same live plan numbers; the rest comes from GET /api/usage
//! (read when the panel opens and every two minutes while it stays open) and, for the account's own report, from
//! one machine's Claude Code (asked again by Refresh).

use gpui::{
    AnyElement, ClickEvent, Div, FontWeight, Hsla, InteractiveElement, IntoElement, MouseButton, ParentElement,
    SharedString, StatefulInteractiveElement, Styled, div, prelude::*, px, relative,
};
use sb_core::usage::{self, DaySlot, ExtraView, LimitRow, ProductRow, TopRow, TopView};
use sb_core::{Tone, fmt};

use crate::theme::Theme;
use crate::ui;
use crate::view::{BoardView, Ctx, UsageBar};

/// The panel's width, the day chart's height, and the room the panel's own header and footer take.
const W: f32 = 520.;
const CHART_H: f32 = 84.;
const FRAME_H: f32 = 94.;

impl BoardView {
    pub fn plan_popover(&self, c: &Ctx, right: f32, win_h: f32) -> impl IntoElement {
        let t = c.t;
        let cx = c.cx;
        c.app.usage_shown.set(c.now);
        // As tall as it needs, never taller than the window: the middle scrolls, the header and footer stay.
        let room = (win_h - ui::header() - 4. - 12. - FRAME_H).max(160.);
        // Lined up under the Week chip, and kept inside the window: on a narrow window the panel gets narrower
        // instead of running off the left edge.
        div()
            .absolute()
            .inset_0()
            .occlude()
            .flex()
            .justify_end()
            .items_start()
            .pt(px(ui::header() + 4.))
            .pr(px(right))
            .pl(px(8.))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.plan_open = false;
                    this.usage_hover = None;
                    cx.notify();
                }),
            )
            .child(
                div()
                    .id("plan")
                    .w(px(W))
                    .flex_shrink(1.)
                    .min_w_0()
                    .rounded(px(ui::R_POP))
                    .border_1()
                    .border_color(t.line2)
                    .bg(t.panel2)
                    .shadow(t.drop())
                    .occlude()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .flex()
                    .flex_col()
                    .child(self.usage_head(c))
                    .child(
                        div()
                            .relative()
                            .child(
                                div()
                                    .id("usage-scroll")
                                    .max_h(px(room))
                                    .overflow_y_scroll()
                                    .track_scroll(&self.usage_scroll)
                                    .px(px(16.))
                                    .pt(px(14.))
                                    .pb(px(14.))
                                    .child(self.usage_body(c)),
                            )
                            .child(ui::scrollbar(&self.usage_scroll)),
                    )
                    .child(self.usage_foot(c)),
            )
    }

    /// "Usage", the plan's name, how old the numbers are, and Refresh.
    fn usage_head(&self, c: &Ctx) -> impl IntoElement {
        let t = c.t;
        let cx = c.cx;
        let app = c.app;
        let u = app.usage.as_ref();
        let account = u.and_then(|u| u.account.as_ref());
        let busy = app.usage_asking || app.usage_loading;
        let state: SharedString = if app.usage_asking {
            "Asking Claude Code".into()
        } else if app.usage_loading && u.is_none() {
            "Loading".into()
        } else {
            usage::freshness(&app.snap.plan, u, c.now).unwrap_or_else(|| "No numbers yet".into()).into()
        };
        div()
            .h(px(48.))
            .flex_none()
            .pl(px(16.))
            .pr(px(10.))
            .border_b_1()
            .border_color(t.line)
            .flex()
            .items_center()
            .gap(px(8.))
            .child(div().flex_none().text_size(px(15.)).font_weight(FontWeight::BOLD).text_color(t.ink).child("Usage"))
            .children(usage::plan_name(account).map(|p| {
                div()
                    .flex_none()
                    .h(px(20.))
                    .px(px(7.))
                    .flex()
                    .items_center()
                    .rounded(px(ui::R_CHIP))
                    .bg(t.chip_bg)
                    .text_size(px(11.5))
                    .text_color(t.chip_ink)
                    .child(p)
            }))
            .child(ui::spacer())
            .child(ui::line(state).flex_shrink(1.).font_features(ui::tnum()).text_size(px(12.)).text_color(t.dim))
            .child(if busy {
                div()
                    .flex_none()
                    .size(px(26.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(ui::spinner("usage-spin", 14., t.dim))
                    .into_any_element()
            } else {
                ui::icon_btn(&t, "usage-refresh", "update", 26.)
                    .tooltip(ui::tip("Refresh"))
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.app.update(cx, |s, cx| s.refresh_usage(cx));
                    }))
                    .into_any_element()
            })
    }

    /// The way to where the plan is changed, since nothing here can change it.
    fn usage_foot(&self, c: &Ctx) -> impl IntoElement {
        let t = c.t;
        let cx = c.cx;
        div()
            .flex_none()
            .min_h(px(44.))
            .px(px(16.))
            .py(px(10.))
            .border_t_1()
            .border_color(t.line)
            .flex()
            .flex_wrap()
            .items_center()
            .gap_x(px(12.))
            .gap_y(px(4.))
            .text_size(px(12.))
            .child(
                ui::link(&t, "usage-manage", "Manage on claude.ai")
                    .flex()
                    .items_center()
                    .gap(px(5.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(ui::icon("out", 12., t.accent))
                    .on_click(cx.listener(|_, _: &ClickEvent, _, cx| crate::state::open_link(cx, usage::MANAGE_URL))),
            )
            .child(ui::wrap("Limits and extra usage are changed there.").flex_1().min_w(px(180.)).text_color(t.faint))
    }

    fn usage_body(&self, c: &Ctx) -> impl IntoElement {
        let t = c.t;
        let app = c.app;
        let p = &app.snap.plan;
        let u = app.usage.as_ref();
        let account = u.and_then(|u| u.account.as_ref());
        let top = usage::top_view(u, usage::TOP_SHOWN);
        let rows = usage::limit_rows(p, account, c.now);
        let extra = usage::extra_view(account);
        let products = usage::product_rows(account);
        // What went wrong, or why part of the panel is missing or old: the server not answering comes first.
        // The error is a sentence or two of its own ("Can't reach Switchboard. Check that ...").
        let notice = match app.usage_error.as_deref().map(|e| e.trim().trim_end_matches('.')) {
            Some(e) if u.is_none() => Some((format!("{e}. Refresh to try again."), Tone::Warn)),
            Some(e) => Some((format!("{e}. Some numbers here may be old until the next refresh."), Tone::Warn)),
            None => usage::account_note(u, c.now),
        };
        div()
            .flex()
            .flex_col()
            .children(notice.map(|(text, tone)| {
                div()
                    .mb(px(12.))
                    .px(px(10.))
                    .py(px(8.))
                    .rounded(px(ui::R_CTRL))
                    .bg(t.bg)
                    .flex()
                    .items_start()
                    .gap(px(8.))
                    .text_size(px(12.))
                    .line_height(relative(1.45))
                    .text_color(t.ink2)
                    .child(div().flex_none().mt(px(2.)).child(match tone {
                        Tone::Ok => ui::icon("info", 13., t.dim),
                        _ => ui::icon("warn", 13., t.cold),
                    }))
                    .child(ui::wrap(text))
            }))
            .child(self.limits_section(c, &rows))
            .children(extra.as_ref().map(|v| extra_section(&t, v)))
            .when(!products.is_empty(), |d| d.child(products_section(c, &products)))
            .child(self.day_chart(c))
            .child(self.weeks_row(c))
            .children(self.top_section(c, &top))
            .child(self.budget_section(c))
    }

    /// Where you stand: one row per limit, each with its bar, percent and reset.
    fn limits_section(&self, c: &Ctx, rows: &[LimitRow]) -> Div {
        let t = c.t;
        let app = c.app;
        let u = app.usage.as_ref();
        let account = u.and_then(|u| u.account.as_ref());
        let mut body = div().flex().flex_col().gap(px(14.));
        for r in rows {
            body = body.child(limit_row(&t, r));
        }
        if rows.is_empty() {
            let (title, sub) = if app.usage_loading && u.is_none() {
                ("Loading your limits", "Reading the numbers from the server.")
            } else if app.usage_asking {
                ("Asking Claude Code", "Your plan's limits show here in a few seconds.")
            } else {
                ("No plan numbers yet", "They show after a chat's next reply. Refresh asks Claude Code for them now.")
            };
            body = body.child(
                div()
                    .py(px(14.))
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(4.))
                    .rounded(px(ui::R_CTRL))
                    .bg(t.bg)
                    .child(div().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).text_color(t.ink2).child(title))
                    .child(ui::wrap(sub).px(px(20.)).text_size(px(12.)).text_color(t.dim)),
            );
        }
        body = body.children(
            usage::missing_limits(&app.snap.plan, account, c.now)
                .map(|m| div().text_size(px(12.)).text_color(t.faint).child(m)),
        );
        section(&t, "Plan limits", None, true).child(body)
    }

    /// The last 4 weeks in one line: each one's final percent as a small bar, and their average.
    fn weeks_row(&self, c: &Ctx) -> impl IntoElement {
        let t = c.t;
        let cx = c.cx;
        let weeks = c.app.usage.as_ref().map(|u| usage::last_weeks(u, 4)).unwrap_or_default();
        let avg = usage::weeks_avg(&weeks);
        let hover = match self.usage_hover {
            Some(UsageBar::Week(i)) => weeks.get(i),
            _ => None,
        };
        let mut bars = div().flex_none().w(px(84.)).h(px(22.)).flex().items_end().gap(px(4.));
        if weeks.is_empty() {
            // Four empty slots, so the row keeps its shape until there is history.
            for _ in 0..4 {
                bars = bars.child(div().flex_1().h(px(3.)).rounded(px(1.5)).bg(t.track));
            }
        }
        for (i, w) in weeks.iter().enumerate() {
            let on = self.usage_hover == Some(UsageBar::Week(i));
            let frac = (w.pct.unwrap_or(0.0) / 100.0).clamp(0.0, 1.0) as f32;
            let color = match w.pct {
                None => t.track,
                Some(x) if x >= 95.0 => t.bad,
                Some(x) if x >= 80.0 => t.cold,
                Some(_) if on => t.ink2,
                Some(_) => t.fill.alpha(0.55),
            };
            bars = bars.child(
                div()
                    .id(("uw", i))
                    .flex_1()
                    .h_full()
                    .flex()
                    .flex_col()
                    .justify_end()
                    .cursor_default()
                    .on_hover(cx.listener(move |this, h: &bool, _, cx| this.hover_bar(UsageBar::Week(i), *h, cx)))
                    .child(div().w_full().h(px((22. * frac).max(3.))).rounded_t(px(2.)).rounded_b(px(1.)).bg(color)),
            );
        }
        let readout = match (hover, weeks.is_empty()) {
            (Some(w), _) => {
                let when = w
                    .reset
                    .filter(|r| *r > 0.0)
                    .map(|r| format!("Week to {}", fmt::md(r)))
                    .unwrap_or_else(|| "That week".into());
                match w.pct {
                    Some(x) => format!("{when}: ended at {}%", x.round() as i64),
                    None => format!("{when}: no number"),
                }
            }
            (None, true) => "History starts this week".into(),
            // The number is the average of where those weeks ended, not of anything about this one.
            (None, false) => match (avg, weeks.len()) {
                (Some(a), 1) => format!("Last week ended at {}%", a.round() as i64),
                (Some(a), n) => format!("{n} weeks ended at {}% on average", a.round() as i64),
                (None, _) => "No numbers for those weeks".into(),
            },
        };
        div()
            .mt(px(12.))
            .flex()
            .items_center()
            .gap(px(12.))
            .child(div().flex_none().text_size(px(12.)).text_color(t.dim).child("Last 4 weeks"))
            .child(bars)
            .child(ui::spacer())
            .child(
                ui::line(readout)
                    .flex_shrink(1.)
                    .font_features(ui::tnum())
                    .text_size(px(12.))
                    .text_color(if hover.is_some() { t.ink } else { t.dim }),
            )
    }

    /// "Most used this week": the chats that took the most of the plan since the weekly reset, each with its live
    /// dot, its name, where it ran and its share of what Switchboard saw. A row opens the chat when it is still
    /// listed; one that is gone (stopped and cleared, or on a machine that left) is only named.
    fn top_section(&self, c: &Ctx, view: &TopView) -> Option<Div> {
        let t = c.t;
        let (rows, rest, foot) = match view {
            TopView::Hidden => return None,
            TopView::Note(n) => {
                return Some(
                    section(&t, "Most used this week", None, false)
                        .child(ui::wrap(n.clone()).text_size(px(12.)).line_height(relative(1.45)).text_color(t.dim)),
                );
            }
            TopView::Rows { rows, rest, foot } => (rows, rest, foot),
        };
        let mut list = div().mx(px(-8.)).flex().flex_col();
        for (i, r) in rows.iter().enumerate() {
            list = list.child(self.top_row(c, i, r));
        }
        if let Some((what, pct)) = rest {
            // What is not listed, in the same columns, so the percents on show add up.
            list = list.child(
                div()
                    .h(px(26.))
                    .px(px(8.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .text_size(px(12.))
                    .text_color(t.dim)
                    .child(div().flex_none().size(px(7.)))
                    .child(ui::line(what.clone()).flex_1())
                    .child(
                        div().flex_none().w(px(36.)).flex().justify_end().font_features(ui::tnum()).child(pct.clone()),
                    ),
            );
        }
        let head = div().text_size(px(12.)).text_color(t.dim).child("Share of use in Switchboard").into_any_element();
        Some(
            section(&t, "Most used this week", Some(head), false)
                .child(list)
                .child(ui::wrap(foot.clone()).text_size(px(12.)).line_height(relative(1.45)).text_color(t.faint)),
        )
    }

    fn top_row(&self, c: &Ctx, i: usize, r: &TopRow) -> AnyElement {
        let t = c.t;
        let cx = c.cx;
        // The chat as the board has it: its live dot and the name the board shows. None when no machine lists it.
        let s = r.key.as_deref().and_then(|k| c.app.snap.session(k)).filter(|s| !c.app.is_hidden(&s.key));
        let dot = match s {
            Some(s) => crate::chrome::live_dot(&t, &c.app.live_of(s, c.now, None), ("ut-live", i), 7.),
            None => div().flex_none().size(px(7.)).rounded_full().border_1().border_color(t.faint).into_any_element(),
        };
        let name = s.map(|s| s.topic.clone()).unwrap_or_else(|| r.name.clone());
        let row = div()
            .id(("ut", i))
            .h(px(28.))
            .px(px(8.))
            .flex()
            .items_center()
            .gap(px(8.))
            .rounded(px(ui::R_CTRL))
            .child(dot)
            .child(ui::line(name).flex_1().text_size(px(12.5)).text_color(t.ink2))
            .when(r.sent, |d| d.child(ui::sent_mark(&t)))
            .child(ui::line(r.place.clone()).flex_shrink(1.).max_w(px(128.)).text_size(px(12.)).text_color(t.dim))
            .child(div().flex_none().w(px(64.)).child(ui::meter(&t, (r.share / 100.0) as f32, t.fill, None, 6.)))
            .child(
                div()
                    .flex_none()
                    .w(px(36.))
                    .flex()
                    .justify_end()
                    .font_features(ui::tnum())
                    .text_size(px(12.5))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(t.ink)
                    .child(r.pct.clone()),
            );
        let Some(key) = s.map(|s| s.key.clone()) else {
            // Stopped and cleared, or on a machine that is away: it still counts, and there is nothing to open.
            return row.tooltip(ui::tip("This chat is no longer open on a machine")).into_any_element();
        };
        ui::focusable(row, &t)
            .cursor_pointer()
            .hover(|d| d.bg(t.hover))
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.plan_open = false;
                this.usage_hover = None;
                this.view_board(cx);
                this.select(Some(key.clone()), cx);
                cx.notify();
            }))
            .into_any_element()
    }

    /// The budget as it stands today, and the way to its settings.
    fn budget_section(&self, c: &Ctx) -> impl IntoElement {
        let t = c.t;
        let cx = c.cx;
        let budget = c.app.snap.budget.clone().unwrap_or_default();
        let (bs, bst) = (budget.settings(), budget.status());
        let hard = bs.hard();
        let daily = match (bst.daily.or(bs.daily), bst.today) {
            (Some(d), Some(x)) => format!("Today {}% of the week, daily budget {}%", usage::pts(x), usage::pts(d)),
            (Some(d), None) => format!("Daily budget {}% of the week", usage::pts(d)),
            (None, Some(x)) => format!("Today {}% of the week, no daily budget", usage::pts(x)),
            (None, None) => "No daily budget".into(),
        };
        // One state, worst first, in words (the dot only repeats it).
        let (state, dot) = if bst.blocking() {
            ("Hard limit reached: new messages are blocked", t.bad)
        } else if bst.over_hard() && bst.lifted() {
            ("Hard limit lifted for today", t.cold)
        } else if bst.over_budget() {
            ("Over budget", t.cold)
        } else if hard.on == Some(true) {
            ("Within budget, hard limit on", t.fill)
        } else {
            ("Within budget, hard limit off", t.fill)
        };
        let link = ui::link(&t, "usage-budget", "Budget settings")
            .text_size(px(12.))
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.plan_open = false;
                this.usage_hover = None;
                cx.notify();
                cx.defer(|cx| crate::settings::open_at(crate::settings::Section::Budget, cx));
            }))
            .into_any_element();
        section(&t, "Budget", Some(link), false).child(
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap_x(px(12.))
                .gap_y(px(4.))
                .text_size(px(12.5))
                .child(div().font_features(ui::tnum()).text_color(t.ink2).child(daily))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .text_color(t.dim)
                        .child(div().flex_none().size(px(6.)).rounded_full().bg(dot))
                        .child(state),
                ),
        )
    }

    /// This week by day: a bar per local day since the reset, today in the accent, the day under the pointer named
    /// with its share of the weekly limit in the line above.
    fn day_chart(&self, c: &Ctx) -> impl IntoElement {
        let t = c.t;
        let cx = c.cx;
        let u = c.app.usage.as_ref();
        let offset = chrono::Local::now().offset().local_minus_utc();
        let slots: Vec<DaySlot> =
            u.map(|u| usage::week_days(u, c.app.snap.plan.week_reset, c.now, offset)).unwrap_or_default();
        let scale = usage::day_scale(&slots);
        let total: f64 = slots.iter().filter_map(|s| s.used).sum();
        let hovered = match self.usage_hover {
            Some(UsageBar::Day(i)) => slots.get(i),
            _ => None,
        };
        // A day's number is weekly-limit percent points, so each reads as a share of the week.
        let help = self.usage_hover == Some(UsageBar::Help);
        // Today's bar is amber over the daily budget, red at the hard limit.
        let bst = c.app.snap.budget.as_ref().map(|b| b.status()).unwrap_or_default();
        let today_c = if bst.over_hard() && !bst.lifted() {
            t.bad
        } else if bst.over_daily() {
            t.cold
        } else {
            t.accent
        };
        let readout = match hovered {
            _ if help => "Each bar is how much of your weekly limit that day used.".to_string(),
            Some(s) if s.future => format!("{}: still to come", s.label),
            Some(s) => match s.used {
                Some(x) => format!("{}: {}% of your weekly limit", s.label, usage::pts(x)),
                None => format!("{}: no number for that day", s.label),
            },
            None if slots.iter().any(|s| s.used.is_some()) => {
                let today = slots.iter().find(|s| s.today).and_then(|s| s.used);
                match today {
                    Some(x) if (total - x).abs() < 0.05 => format!("Today {}% of your weekly limit", usage::pts(x)),
                    Some(x) => format!("Today {}%, {}% this week so far", usage::pts(x), usage::pts(total)),
                    None => format!("{}% of your weekly limit so far", usage::pts(total)),
                }
            }
            None => String::new(),
        };
        let right = div()
            .flex()
            .items_center()
            .gap(px(8.))
            .min_w_0()
            .child(
                ui::line(readout)
                    .flex_shrink(1.)
                    .font_features(ui::tnum())
                    .text_size(px(12.))
                    .text_color(if hovered.is_some() || help { t.ink } else { t.dim }),
            )
            .child(
                div()
                    .id("uhelp")
                    .flex_none()
                    .cursor_default()
                    .on_hover(cx.listener(|this, h: &bool, _, cx| this.hover_bar(UsageBar::Help, *h, cx)))
                    .child(ui::icon("info", 12., if help { t.ink2 } else { t.faint })),
            )
            .into_any_element();
        let body: AnyElement = if slots.iter().all(|s| s.used.is_none()) {
            // Nothing to draw yet: say why in the chart's own space, so the panel keeps its shape.
            let (title, sub) = if c.app.usage_loading && u.is_none() {
                ("Loading this week", "Reading the numbers from the server.")
            } else if u.is_none() {
                ("No history from the server", "Each day's share shows once the server answers.")
            } else {
                ("History starts this week", "Each day's share fills in as chats reply.")
            };
            div()
                .h(px(CHART_H + 20.))
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(4.))
                .rounded(px(ui::R_CTRL))
                .bg(t.bg)
                .child(div().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).text_color(t.ink2).child(title))
                .child(ui::wrap(sub).px(px(20.)).text_size(px(12.)).text_color(t.dim))
                .into_any_element()
        } else {
            let mut cols = div().flex().items_end().gap(px(6.));
            for (i, s) in slots.iter().enumerate() {
                let on = self.usage_hover == Some(UsageBar::Day(i));
                let h = s.used.map(|x| ((x / scale) as f32 * CHART_H).max(if x > 0.0 { 4. } else { 2. })).unwrap_or(2.);
                let color = if s.future || s.used.is_none() {
                    t.track
                } else if s.today {
                    if on { crate::theme::mix(today_c, t.ink, 0.15) } else { today_c }
                } else if on {
                    t.ink2
                } else {
                    t.fill.alpha(0.5)
                };
                let label_c = if s.today || on { t.ink } else { t.dim };
                cols = cols.child(
                    div()
                        .id(("ud", i))
                        .flex_1()
                        .flex()
                        .flex_col()
                        .items_center()
                        .gap(px(6.))
                        .cursor_default()
                        .on_hover(cx.listener(move |this, h: &bool, _, cx| this.hover_bar(UsageBar::Day(i), *h, cx)))
                        .child(
                            div().w_full().h(px(CHART_H)).flex().flex_col().justify_end().items_center().child(
                                div()
                                    .w(relative(0.72))
                                    .max_w(px(34.))
                                    .h(px(h))
                                    .rounded_t(px(4.))
                                    .rounded_b(px(1.5))
                                    .bg(color),
                            ),
                        )
                        .child(
                            div()
                                .text_size(px(12.))
                                .text_color(label_c)
                                .when(s.today, |d| d.font_weight(FontWeight::BOLD))
                                .child(if s.today { "Today" } else { s.label }),
                        ),
                );
            }
            // A faint baseline under the bars.
            div()
                .relative()
                .child(div().absolute().left_0().right_0().top(px(CHART_H)).h(px(1.)).bg(t.line))
                .child(cols)
                .into_any_element()
        };
        section(&t, "This week by day", Some(right), false).child(body)
    }

    fn hover_bar(&mut self, bar: UsageBar, on: bool, cx: &mut gpui::Context<Self>) {
        if on {
            self.usage_hover = Some(bar);
        } else if self.usage_hover == Some(bar) {
            self.usage_hover = None;
        }
        cx.notify();
    }
}

/// One part of the panel: a hairline above it (not above the first), its name, and something at the right.
fn section(t: &Theme, title: &str, right: Option<AnyElement>, first: bool) -> Div {
    div()
        .when(!first, |d| d.mt(px(16.)).pt(px(14.)).border_t_1().border_color(t.line))
        .pb(px(2.))
        .flex()
        .flex_col()
        .gap(px(12.))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(10.))
                .child(
                    div()
                        .flex_none()
                        .text_size(px(12.5))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(t.ink)
                        .child(SharedString::from(title.to_string())),
                )
                .child(ui::spacer())
                .children(right),
        )
}

/// A limit: its name and percent, the bar (with a tick where the week is heading), and when it resets.
fn limit_row(t: &Theme, r: &LimitRow) -> Div {
    let label = match r.pct {
        Some(p) if r.at_least => format!("at least {}% used", p.round() as i64),
        Some(p) => format!("{}% used", p.round() as i64),
        None => "--".into(),
    };
    div()
        .flex()
        .flex_col()
        .gap(px(8.))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .text_size(px(13.))
                .child(ui::line(r.title.clone()).flex_1().text_color(t.ink2))
                .child(
                    div()
                        .flex_none()
                        .font_features(ui::tnum())
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(if r.pct.is_some() { t.ink } else { t.dim })
                        .child(label),
                ),
        )
        .child(ui::meter(
            t,
            (r.pct.unwrap_or(0.0) / 100.0) as f32,
            tone_color(t, r.tone),
            r.pace.map(|x| (x / 100.0) as f32),
            6.,
        ))
        .child(
            div()
                .flex()
                .flex_wrap()
                .gap_x(px(12.))
                .gap_y(px(2.))
                .font_features(ui::tnum())
                .text_size(px(12.))
                .text_color(t.dim)
                .child(ui::wrap(r.note.clone()).flex_1().min_w(px(180.)))
                .children(r.pace.map(|x| div().flex_none().child(format!("On pace for {}%", x.round() as i64)))),
        )
}

/// A figure with its name above it.
fn stat(t: &Theme, name: &'static str, value: String) -> Div {
    div()
        .flex_1()
        .min_w(px(96.))
        .flex()
        .flex_col()
        .gap(px(3.))
        .child(div().text_size(px(12.)).text_color(t.dim).child(name))
        .child(
            div()
                .font_features(ui::tnum())
                .text_size(px(15.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(t.ink)
                .child(value),
        )
}

/// Extra usage: on or off, what is spent this month, the monthly limit and what is left under it.
fn extra_section(t: &Theme, v: &ExtraView) -> Div {
    let dot = match (v.tone, v.on) {
        (Tone::Bad, _) => t.bad,
        (Tone::Warn, _) => t.cold,
        (Tone::Ok, true) => t.fill,
        (Tone::Ok, false) => t.faint,
    };
    let state = div()
        .flex_none()
        .flex()
        .items_center()
        .gap(px(6.))
        .text_size(px(12.))
        .text_color(t.ink2)
        .child(div().flex_none().size(px(6.)).rounded_full().bg(dot))
        .child(v.state)
        .into_any_element();
    let mut stats = div().flex().flex_wrap().gap_x(px(12.)).gap_y(px(10.));
    stats = stats.child(stat(t, "Spent this month", v.spent.clone().unwrap_or_else(|| "--".into())));
    stats = stats.child(stat(t, "Monthly limit", v.limit.clone().unwrap_or_else(|| "None set".into())));
    if let Some(left) = &v.left {
        stats = stats.child(stat(t, "Left this month", left.clone()));
    }
    if let Some(b) = &v.balance {
        stats = stats.child(stat(t, "Balance", b.clone()));
    }
    section(t, "Extra usage", Some(state), false)
        .child(stats)
        .children(v.pct.map(|p| {
            div()
                .flex()
                .items_center()
                .gap(px(10.))
                .child(div().flex_1().child(ui::meter(t, (p / 100.0) as f32, tone_color(t, v.tone), None, 6.)))
                .child(
                    div()
                        .flex_none()
                        .font_features(ui::tnum())
                        .text_size(px(12.))
                        .text_color(t.dim)
                        .child(format!("{}% of the limit", p.round() as i64)),
                )
        }))
        .children(v.note.clone().map(|n| ui::wrap(n).text_size(px(12.)).line_height(relative(1.45)).text_color(t.dim)))
}

/// The week by product: one bar per product, its share of what was used, and that use against the weekly limit.
fn products_section(c: &Ctx, rows: &[ProductRow]) -> Div {
    let t = &c.t;
    let as_of =
        c.app.usage.as_ref().and_then(|u| u.account.as_ref()).and_then(|a| a.products.as_ref()).and_then(|p| p.as_of);
    let head = div().text_size(px(12.)).text_color(t.dim).child("Share of this week's use").into_any_element();
    let mut list = div().flex().flex_col().gap(px(8.));
    for r in rows {
        list = list.child(
            div()
                .flex()
                .items_center()
                .gap(px(10.))
                .text_size(px(12.5))
                .child(ui::line(r.name.clone()).flex_none().w(px(96.)).text_color(t.ink2))
                .child(div().flex_1().min_w(px(40.)).child(ui::meter(t, (r.share / 100.0) as f32, t.fill, None, 6.)))
                .child(
                    div()
                        .flex_none()
                        .w(px(36.))
                        .flex()
                        .justify_end()
                        .font_features(ui::tnum())
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(t.ink)
                        .child(format!("{}%", r.share.round() as i64)),
                )
                .child(
                    div()
                        .flex_none()
                        .w(px(84.))
                        .flex()
                        .justify_end()
                        .font_features(ui::tnum())
                        .text_size(px(12.))
                        .text_color(t.dim)
                        .child(match r.of_limit {
                            Some(x) => format!("{}% of limit", x.round() as i64),
                            None => String::new(),
                        }),
                ),
        );
    }
    let source = match as_of.filter(|a| *a > 0.0 && c.now - *a >= 600.0) {
        Some(a) => format!("From your account, across every Claude app, as of {}.", fmt::when_exact(a, c.now)),
        None => "From your account, across every Claude app.".into(),
    };
    section(t, "This week by product", Some(head), false)
        .child(list)
        .child(ui::wrap(source).text_size(px(12.)).text_color(t.faint))
}

fn tone_color(t: &Theme, tone: Tone) -> Hsla {
    match tone {
        Tone::Ok => t.fill,
        Tone::Warn => t.cold,
        Tone::Bad => t.bad,
    }
}
