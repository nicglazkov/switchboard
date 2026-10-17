//! The usage panel under the Week chip: three tiles (the 5-hour window, this week, the last 4 weeks), this week as a
//! bar per day since the reset with today in the accent, and the chats that would cost the most to go back to. The tiles read the live plan numbers
//! the chip shows; the history comes from GET /api/usage, read each time the panel opens.

use gpui::{FontWeight, Hsla, InteractiveElement, IntoElement, MouseButton, ParentElement, SharedString, StatefulInteractiveElement, Styled, div, prelude::*, px, relative};
use sb_core::usage::{self, DaySlot};
use sb_core::{Session, Tone, fmt};

use crate::theme::Theme;
use crate::ui;
use crate::view::{BoardView, Ctx, UsageBar};

/// The panel's width, and the day chart's height.
const W: f32 = 468.;
const CHART_H: f32 = 92.;

impl BoardView {
    pub fn plan_popover(&self, c: &Ctx, right: f32) -> impl IntoElement {
        let t = c.t;
        let cx = c.cx;
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
            .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, cx| {
                this.plan_open = false;
                this.usage_hover = None;
                cx.notify();
            }))
            .child(
                div()
                    .id("plan")
                    .w(px(W))
                    .flex_shrink(1.)
                    .min_w_0()
                    .px(px(16.))
                    .pt(px(14.))
                    .pb(px(16.))
                    .rounded(px(ui::R_POP))
                    .border_1()
                    .border_color(t.line2)
                    .bg(t.panel2)
                    .shadow(t.drop())
                    .occlude()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(self.usage_body(c)),
            )
    }

    fn usage_body(&self, c: &Ctx) -> impl IntoElement {
        let t = c.t;
        let p = &c.app.snap.plan;
        // The chats board::tally counts as costly to go back to (gone cold with a big context), the 3 biggest.
        let mut costly: Vec<&Session> =
            c.app.snap.sessions().into_iter().filter(|s| !s.stale && !c.app.is_hidden(&s.key) && s.needs_fresh(c.now)).collect();
        costly.sort_by_key(|s| std::cmp::Reverse(s.ctx));
        costly.truncate(3);
        div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(div().text_size(px(15.)).font_weight(FontWeight::BOLD).text_color(t.ink).child("Usage"))
                    .child(ui::spacer())
                    .when(c.app.usage_loading, |d| d.child(div().text_size(px(12.)).text_color(t.faint).child("Updating")))
                    .children(p.as_of(c.now).filter(|_| !c.app.usage_loading).map(|a| div().text_size(px(12.)).text_color(t.faint).child(cap(&a)))),
            )
            .child(div().flex().gap(px(8.)).child(self.five_tile(c)).child(self.week_tile(c)).child(self.weeks_tile(c)))
            .child(self.day_chart(c))
            .when(!costly.is_empty(), |d| d.child(self.costly_list(c, &costly)))
            .child(
                div()
                    .flex()
                    .items_start()
                    .gap(px(8.))
                    .text_size(px(12.))
                    .line_height(relative(1.45))
                    .text_color(t.faint)
                    .child(div().flex_none().mt(px(1.)).child(ui::icon("info", 13., t.faint)))
                    .child(ui::wrap("Numbers update when a session replies, so use in the Claude apps shows up later.")),
            )
    }

    /// "Costly to go back to": each chat's live dot, its name and what going back would re-read. A row opens the chat.
    fn costly_list(&self, c: &Ctx, list: &[&Session]) -> impl IntoElement {
        let t = c.t;
        let cx = c.cx;
        let mut rows = div().flex().flex_col();
        for (i, s) in list.iter().enumerate() {
            let key = s.key.clone();
            let l = c.app.live_of(s, c.now, None);
            rows = rows.child(
                div()
                    .id(("uc", i))
                    .h(px(28.))
                    .px(px(8.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .rounded(px(ui::R_CTRL))
                    .cursor_pointer()
                    .hover(|d| d.bg(t.hover))
                    .on_click(cx.listener(move |this, _: &gpui::ClickEvent, _, cx| {
                        this.plan_open = false;
                        this.usage_hover = None;
                        this.view_board(cx);
                        this.select(Some(key.clone()), cx);
                        cx.notify();
                    }))
                    .child(crate::chrome::live_dot(&t, &l, ("uc-live", i), 7.))
                    .child(ui::line(s.topic.clone()).flex_1().text_size(px(12.5)).text_color(t.ink2))
                    .child(div().flex_none().font_features(ui::tnum()).text_size(px(12.)).text_color(t.dim).child(format!("re-reads {}", fmt::k(s.ctx)))),
            );
        }
        div()
            .flex()
            .flex_col()
            .gap(px(4.))
            .child(div().px(px(8.)).text_size(px(12.5)).font_weight(FontWeight::SEMIBOLD).text_color(t.ink2).child("Costly to go back to"))
            .child(rows)
    }

    /// The 5-hour window: percent used and when it resets.
    fn five_tile(&self, c: &Ctx) -> impl IntoElement {
        let t = c.t;
        let p = &c.app.snap.plan;
        let live = p.five_live(c.now);
        let pct = if live { p.five_hour } else { None };
        let sub = if live {
            p.five_hour_reset.map(|r| format!("Resets {}", fmt::when(r, c.now))).unwrap_or_else(|| "Reset time not known".into())
        } else if p.five_hour.is_some() {
            "The window has reset since".into()
        } else {
            "No numbers yet".into()
        };
        let tone = tone_color(&t, p.five_tone());
        tile(&t, "5 hours")
            .child(big(&t, if live { p.five_label(c.now) } else { "--".into() }, false))
            .child(ui::meter(&t, (pct.unwrap_or(0.0) / 100.0) as f32, tone, None, 4.))
            .child(small(&t, sub))
    }

    /// This week: percent used (with the projected tick), when it resets and the pace.
    fn week_tile(&self, c: &Ctx) -> impl IntoElement {
        let t = c.t;
        let p = &c.app.snap.plan;
        let label = p.week_label(c.now);
        let shown = if label == "--" { 0.0 } else { p.week.unwrap_or(0.0) };
        let pace = p.week_projected.or_else(|| c.app.usage.as_ref().and_then(|u| u.week.as_ref()).and_then(|w| w.pace));
        let reset = p.week_reset.or_else(|| c.app.usage.as_ref().and_then(|u| u.week.as_ref()).and_then(|w| w.reset));
        let at_least = label.starts_with("at least ");
        tile(&t, "This week")
            .child(
                div()
                    .flex()
                    .items_end()
                    .gap(px(5.))
                    .when(at_least, |d| d.child(div().pb(px(3.)).text_size(px(11.5)).text_color(t.dim).child("at least")))
                    .child(big(&t, label.trim_start_matches("at least ").to_string(), p.limits_old(c.now))),
            )
            .child(ui::meter(&t, (shown / 100.0) as f32, tone_color(&t, p.week_tone()), pace.map(|x| (x / 100.0) as f32), 4.))
            .child(small(&t, reset.map(|r| format!("Resets {}", fmt::when(r, c.now))).unwrap_or_else(|| "Reset not known yet".into())))
            .children(pace.map(|x| small(&t, format!("On pace for {}%", x.round() as i64))))
    }

    /// The last 4 weeks: each one's final percent as a small bar, and their average.
    fn weeks_tile(&self, c: &Ctx) -> impl IntoElement {
        let t = c.t;
        let cx = c.cx;
        let weeks = c.app.usage.as_ref().map(|u| usage::last_weeks(u, 4)).unwrap_or_default();
        let avg = usage::weeks_avg(&weeks);
        let hover = match self.usage_hover {
            Some(UsageBar::Week(i)) => weeks.get(i),
            _ => None,
        };
        let mut bars = div().h(px(26.)).flex().items_end().gap(px(4.));
        if weeks.is_empty() {
            // Four empty slots, so the tile keeps its shape until there is history.
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
                Some(_) => if on { t.ink2 } else { t.fill.alpha(0.55) },
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
                    .child(div().w_full().h(px((26. * frac).max(3.))).rounded_t(px(2.)).rounded_b(px(1.)).bg(color)),
            );
        }
        let sub = match (hover, weeks.is_empty()) {
            (Some(w), _) => {
                let when = w.reset.filter(|r| *r > 0.0).map(|r| format!("Week to {}", fmt::md(r))).unwrap_or_else(|| "That week".into());
                match w.pct {
                    Some(x) => format!("{when}: {}%", x.round() as i64),
                    None => format!("{when}: no number"),
                }
            }
            (None, true) => "History starts this week".into(),
            (None, false) => format!("Average of {} {}", weeks.len(), if weeks.len() == 1 { "week" } else { "weeks" }),
        };
        tile(&t, "Last 4 weeks")
            .child(big(&t, avg.map(|a| format!("{}%", a.round() as i64)).unwrap_or_else(|| "--".into()), false))
            .child(bars)
            .child(small(&t, sub))
    }

    /// This week by day: a bar per local day since the reset, today in the accent, the day under the pointer named
    /// with its share of the weekly limit in the line above.
    fn day_chart(&self, c: &Ctx) -> impl IntoElement {
        let t = c.t;
        let cx = c.cx;
        let u = c.app.usage.as_ref();
        let offset = chrono::Local::now().offset().local_minus_utc();
        let slots: Vec<DaySlot> = u.map(|u| usage::week_days(u, c.app.snap.plan.week_reset, c.now, offset)).unwrap_or_default();
        let scale = usage::day_scale(&slots);
        let total: f64 = slots.iter().filter_map(|s| s.used).sum();
        let hovered = match self.usage_hover {
            Some(UsageBar::Day(i)) => slots.get(i),
            _ => None,
        };
        // A day's number is weekly-limit percent points, so each reads as a share of the week.
        let help = self.usage_hover == Some(UsageBar::Help);
        // With a daily budget: "Today 7% of 14% budget", and today's bar amber over it, red at the hard limit.
        let budget = c.app.snap.budget.clone().unwrap_or_default();
        let (bs, bst) = (budget.settings(), budget.status());
        let today_used = bst.today.or_else(|| slots.iter().find(|s| s.today).and_then(|s| s.used));
        let budget_line = match (bs.daily, today_used) {
            (Some(d), Some(x)) => Some(format!("Today {}% of {}% budget", usage::pts(x), usage::pts(d))),
            _ => None,
        };
        let today_c = if bst.over_hard() && !bst.lifted() { t.bad } else if bst.over_daily() { t.cold } else { t.accent };
        let readout = match hovered {
            _ if help => "Each bar is how much of your weekly limit that day used.".to_string(),
            Some(s) if s.future => format!("{}: still to come", s.label),
            Some(s) => match s.used {
                Some(x) => format!("{}: {}% of the week", s.label, usage::pts(x)),
                None => format!("{}: no number for that day", s.label),
            },
            None if budget_line.is_some() => budget_line.clone().unwrap_or_default(),
            None if slots.iter().any(|s| s.used.is_some()) => {
                let today = slots.iter().find(|s| s.today).and_then(|s| s.used);
                match today {
                    Some(x) if (total - x).abs() < 0.05 => format!("Today: {}% of your weekly limit", usage::pts(x)),
                    Some(x) => format!("Today {}%, {}% of the week so far", usage::pts(x), usage::pts(total)),
                    None => format!("{}% of the week so far", usage::pts(total)),
                }
            }
            None => String::new(),
        };
        let head = div()
            .flex()
            .items_center()
            .gap(px(8.))
            .child(div().flex_none().text_size(px(12.5)).font_weight(FontWeight::SEMIBOLD).text_color(t.ink2).child("This week by day"))
            .child(
                div()
                    .id("uhelp")
                    .flex_none()
                    .cursor_default()
                    .on_hover(cx.listener(|this, h: &bool, _, cx| this.hover_bar(UsageBar::Help, *h, cx)))
                    .child(ui::icon("info", 12., if help { t.ink2 } else { t.faint })),
            )
            .child(ui::spacer())
            .child(
                ui::line(readout)
                    .flex_shrink(1.)
                    .font_features(ui::tnum())
                    .text_size(px(12.))
                    .text_color(if hovered.is_some() || help { t.ink } else { t.dim }),
            );
        let body: gpui::AnyElement = if slots.iter().all(|s| s.used.is_none()) {
            // Nothing to draw yet: say why in the chart's own space, so the panel keeps its shape.
            let (title, sub) = if c.app.usage_loading && u.is_none() {
                ("Loading this week", "Reading the numbers from the server.".to_string())
            } else if u.is_none() {
                (
                    "No history from the server",
                    match &c.app.usage_error {
                        Some(e) => format!("It did not answer for usage: {e}"),
                        None => "It has not sent any usage yet.".into(),
                    },
                )
            } else {
                ("History starts this week", "Each day's share fills in as sessions reply.".to_string())
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
                .child(ui::icon("plan", 18., t.faint))
                .child(div().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).text_color(t.ink2).child(title))
                .child(ui::wrap(sub).px(px(20.)).text_size(px(12.)).text_color(t.faint))
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
                let label_c = if s.today { today_c } else if on { t.ink } else { t.faint };
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
                            div()
                                .w_full()
                                .h(px(CHART_H))
                                .flex()
                                .flex_col()
                                .justify_end()
                                .items_center()
                                .child(div().w(relative(0.72)).max_w(px(34.)).h(px(h)).rounded_t(px(4.)).rounded_b(px(1.5)).bg(color)),
                        )
                        .child(
                            div()
                                .text_size(px(11.5))
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
        div()
            .p(px(12.))
            .rounded(px(ui::R_CARD))
            .border_1()
            .border_color(t.line)
            .bg(t.panel)
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(head)
            .child(body)
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

/// A stat tile: a small heading, then what the caller adds.
fn tile(t: &Theme, title: &str) -> gpui::Div {
    div()
        .flex_1()
        .min_w_0()
        .p(px(11.))
        .rounded(px(ui::R_CARD))
        .border_1()
        .border_color(t.line)
        .bg(t.panel)
        .flex()
        .flex_col()
        .gap(px(7.))
        .child(ui::lbl(title, t.label))
}

fn big(t: &Theme, text: impl Into<SharedString>, old: bool) -> gpui::Div {
    ui::num(text, 24., if old { t.dim } else { t.ink })
}

fn small(t: &Theme, text: impl Into<SharedString>) -> gpui::Div {
    div().text_size(px(11.5)).line_height(relative(1.35)).text_color(t.faint).child(text.into())
}

fn tone_color(t: &Theme, tone: Tone) -> Hsla {
    match tone {
        Tone::Ok => t.fill,
        Tone::Warn => t.cold,
        Tone::Bad => t.bad,
    }
}

/// "as of 2:10 PM" as "As of 2:10 PM".
fn cap(s: &str) -> String {
    let mut ch = s.chars();
    match ch.next() {
        Some(f) => f.to_uppercase().collect::<String>() + ch.as_str(),
        None => String::new(),
    }
}
