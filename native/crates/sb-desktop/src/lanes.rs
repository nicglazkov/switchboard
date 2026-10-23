//! The Lanes board: machines down the left, states across (Needs you, Working, Warm
//! now, Gone cold, Quiet). Above it, the status line (the server's note, from notes.py) with "Stopped today" on its
//! right, and, only while it matters, a slim strip for chats that are open but not in use. Details float in a sheet
//! over the board.

use gpui::{
    AnimationExt, AnyElement, ClickEvent, FontWeight, InteractiveElement, IntoElement, MouseButton, ParentElement,
    SharedString, StatefulInteractiveElement, Styled, div, prelude::*, px, relative,
};
use sb_core::api::ApiCleanItem;
use sb_core::board::{self, Column};
use sb_core::{Lamp, Session, fmt, names};

use crate::state::clean_key;
use crate::theme::Theme;
use crate::ui;
use crate::view::{BoardView, Ctx};

/// The board's layout at a window width: the side padding and the machine column.
#[derive(Clone, Copy)]
pub struct Geo {
    pub pad: f32,
    pub label: f32,
    pub wide: bool,
    /// Room kept free on the right for the detail sheet, so no column sits hidden under it.
    pub sheet: f32,
    /// The grid's narrowest width (machine column and five state columns); the board scrolls sideways below it.
    pub grid_min: f32,
}

/// The narrowest a state column gets before the board scrolls sideways instead (a card's name needs this much
/// beside its pills to read as words).
const COL_MIN: f32 = 220.;

impl Geo {
    /// `sheet`: the detail sheet's width when it is open, else 0.
    pub fn at(w: f32, solo: bool, sheet: f32) -> Geo {
        let wide = w >= 1280.;
        // Next to the sheet the machine column narrows first, so the five state columns keep their room.
        let label = if solo {
            0.
        } else if sheet > 0. {
            132.
        } else if wide {
            196.
        } else {
            150.
        };
        let sheet = if sheet > 0. { sheet + 24. } else { 0. };
        Geo { pad: if wide { ui::EDGE } else { 14. }, label, wide, sheet, grid_min: label + 5. * COL_MIN + 50. }
    }

    /// The width the board has to itself: the window less the sheet.
    pub fn board_w(&self, w: f32) -> f32 {
        (w - self.sheet).max(0.)
    }

    /// True when the five columns do not fit next to the sheet, so the grid scrolls sideways.
    pub fn scrolls(&self, w: f32) -> bool {
        self.board_w(w) - self.pad * 2. < self.grid_min
    }
}

/// Column names as the board heads say them.
pub fn col_name(c: Column) -> &'static str {
    match c {
        Column::Need => "Needs you",
        Column::Run => "Working",
        Column::Warm => "Warm now",
        Column::Cold => "Gone cold",
        Column::Quiet => "Quiet",
    }
}

fn plural(n: usize, one: &str) -> String {
    if n == 1 { format!("1 {one}") } else { format!("{n} {one}s") }
}

fn os_name(os: Option<&str>) -> String {
    match os.unwrap_or("") {
        "mac" | "macos" | "darwin" => "macOS".into(),
        "windows" => "Windows".into(),
        "linux" => "Linux".into(),
        o => o.to_string(),
    }
}

/// Quiet rows shown per machine before "n more", and how recent a quiet chat is to stay on the board.
const QUIET_CAP: usize = 4;
const QUIET_RECENT: f64 = 24. * 3600.;

impl BoardView {
    pub fn board_area(&self, c: &Ctx, g: Geo, w: f32) -> AnyElement {
        let t = c.t;
        let app = c.app;
        if app.snap.machines.is_empty() {
            // No server to show (firstrun.rs): the first run searches the local network; a saved address that does
            // not answer takes a typed one or a search; a server that was just picked is being connected to.
            if self.find_shot || app.core.needs_server() {
                return self.server_screen(c, true, "");
            }
            if let Some(e) = &app.error {
                return self.server_screen(c, false, e);
            }
            if !app.live
                && let Some(f) = &self.joining
            {
                return self.joining_screen(c, f);
            }
            if app.live {
                // Connected, and the server has no machines yet (a server just set up here).
                let add = ui::setup_btn(&t, "board-add-machine", "Add a machine")
                    .on_click(|_: &ClickEvent, _, cx| crate::files::open_pair(cx));
                return div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap(px(16.))
                    .px(px(16.))
                    .child(ui::mark(&t, 28.))
                    .child(div().w_full().max_w(px(540.)).child(ui::setup_card(
                        &t,
                        "plus",
                        "No machines yet",
                        "Switchboard is running. Add the computers where Claude Code runs, this one too, and their \
                         chats show up here.",
                        Some(add.into_any_element()),
                    )))
                    .into_any_element();
            }
            let (title, sub) = (
                "Connecting to Switchboard".to_string(),
                "The board shows up as soon as the computer that runs Switchboard answers.".to_string(),
            );
            return div()
                .flex_1()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(8.))
                .child(ui::mark(&t, 28.))
                .child(div().mt(px(6.)).text_size(px(16.)).font_weight(FontWeight::BOLD).text_color(t.ink).child(title))
                .child(div().max_w(px(440.)).text_size(px(13.)).text_color(t.dim).text_center().child(sub))
                .into_any_element();
        }
        let hidden = app.hidden_set();
        let b = board::build(&app.snap, c.now, &hidden, self.solo.as_deref());
        let heads: Vec<AnyElement> =
            Column::ALL.iter().map(|col| self.lane_head(c, *col, b.column_counts[col.index()] as usize)).collect();
        let rows: Vec<AnyElement> = b
            .lanes
            .iter()
            .map(|lane| {
                let shut = self.solo.is_none() && app.is_shut(&lane.machine);
                self.machine_row(c, lane, g, shut)
            })
            .collect();
        let head = crate::view::grid_row((g.label > 0.).then(|| div().w(px(g.label)).into_any_element()), heads)
            .border_b_1()
            .border_color(t.line);
        let lanes = div()
            .id("lanes")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&self.board_scroll)
            .pb(px(40.))
            .children(rows);
        let lanes =
            div().flex_1().min_h_0().relative().flex().flex_col().child(lanes).child(ui::scrollbar(&self.board_scroll));
        // Next to the detail sheet the columns share what is left; when that is too little for five columns,
        // the heads and rows scroll sideways together rather than slide under the sheet.
        let grid = if g.scrolls(w) {
            div().flex_1().min_h_0().min_w_0().px(px(g.pad)).child(
                div().id("lanes-x").size_full().overflow_x_scroll().child(
                    div()
                        .w(px(g.grid_min))
                        .h_full()
                        .flex()
                        .flex_col()
                        .child(div().flex_none().child(head))
                        .child(lanes),
                ),
            )
        } else {
            div()
                .flex_1()
                .min_h_0()
                .min_w_0()
                .flex()
                .flex_col()
                .px(px(g.pad))
                .child(div().flex_none().child(head))
                .child(lanes)
        };
        div()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .flex()
            .flex_col()
            // The status line sits above the sheet's top edge, so only what is beside the sheet makes room for it.
            .child(self.sub_bar(c, g))
            .children(self.spare_strip(c, g).map(|s| div().pr(px(g.sheet)).child(s)))
            .child(div().flex_1().min_h_0().min_w_0().flex().flex_col().pr(px(g.sheet)).child(grid))
            .into_any_element()
    }

    /// The status line: the server's note (or a calm word), its one action, and "Stopped today" on the right.
    fn sub_bar(&self, c: &Ctx, g: Geo) -> impl IntoElement {
        let t = c.t;
        let cx = c.cx;
        let note = c.app.snap.plan.note.clone();
        let (dot, text) = match &note {
            Some(n) => (
                match n.tone.as_str() {
                    "need" => t.need,
                    "warm" => t.warm,
                    "warn" => t.cold,
                    "bad" => t.bad,
                    _ => t.faint,
                },
                n.text.clone(),
            ),
            None => (t.idle, "Nothing needs you right now.".to_string()),
        };
        let key = note.as_ref().and_then(|n| n.key()).filter(|k| c.app.snap.session(k).is_some());
        let act: Option<(&str, String)> = match (&note, key) {
            (Some(n), Some(k)) if n.kind == "need" => Some(("Open", k)),
            (Some(_), Some(k)) => Some(("Reply now", k)),
            (Some(n), None) if matches!(n.kind.as_str(), "week" | "five" | "burn" | "pace") => {
                Some(("Plan details", String::new()))
            }
            _ => None,
        };
        div()
            .flex_none()
            .min_h(px(52.))
            .px(px(g.pad))
            .py(px(10.))
            .flex()
            .items_center()
            .gap(px(14.))
            .child(div().flex_none().size(px(8.)).rounded_full().bg(dot))
            .child(
                ui::wrap(text)
                    .flex_1()
                    .min_w_0()
                    .text_size(px(14.))
                    .line_height(relative(1.4))
                    .text_color(if note.is_some() { t.ink2 } else { t.faint }),
            )
            .children(act.map(|(label, k)| {
                ui::line_btn(&t, "note-act", 30.)
                    .px(px(12.))
                    .text_size(px(13.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(label.to_string())
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        if k.is_empty() {
                            this.open_plan(cx);
                        } else {
                            this.open_terminal(k.clone(), false, cx);
                        }
                    }))
                    .into_any_element()
            }))
            .child(self.stopped_pill(c))
    }

    /// "Stopped today 3, freed 1.2 GB": where stopped chats land. The freed memory ticks up as one arrives.
    fn stopped_pill(&self, c: &Ctx) -> AnyElement {
        let t = c.t;
        let cl = c.app.snap.cleanup.as_ref();
        let mine = |m: &str| self.solo.as_deref().is_none_or(|s| s == m);
        let stopped: Vec<_> = cl.map(|cl| cl.stopped.iter().filter(|d| mine(&d.machine)).collect()).unwrap_or_default();
        let freed: f64 = stopped.iter().filter_map(|d| d.mb).sum();
        let prev = self.freed_seen.get();
        self.freed_seen.set(freed);
        // Ticks up when a stop lands (not on the first frame, which only learns the total).
        if freed > prev + 0.5 && self.stopped_seen.get().is_some() {
            self.freed_from.set((prev, c.now));
        }
        let (from, since) = self.freed_from.get();
        let n = stopped.len();
        let seen = self.stopped_seen.get();
        if seen.is_some_and(|s| n > s) {
            self.stopped_flash.set(c.now);
        }
        self.stopped_seen.set(Some(n));
        let flash = self.stopped_flash.get();
        let accent = t.accent;
        let pill = div()
            .id("stopped-pill")
            .flex_none()
            .h(px(32.))
            .px(px(14.))
            .flex()
            .items_center()
            .gap(px(8.))
            .rounded_full()
            .bg(t.panel)
            .shadow(vec![Theme::ring(t.line2)])
            .text_size(px(13.))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(t.ink2)
            .cursor_pointer()
            .when(self.stopped_open, |d| d.bg(t.press))
            .on_click(c.cx.listener(|this, _: &ClickEvent, _, cx| {
                this.stopped_open = !this.stopped_open;
                cx.notify();
            }))
            .child(ui::icon("stop", 14., t.faint))
            .child("Stopped today")
            .child(div().font_weight(FontWeight::BOLD).text_color(t.ink).child(n.to_string()))
            .when(freed > 0., |d| {
                let ticking = c.now - since < 1.2;
                d.child(if ticking {
                    div()
                        .text_color(t.warm)
                        .with_animation(
                            SharedString::from(format!("freed-{since}")),
                            gpui::Animation::new(std::time::Duration::from_millis(900)),
                            move |d, k| {
                                let e = 1. - (1. - k as f64).powi(3);
                                d.child(format!("freed {}", fmt::mem(from + (freed - from) * e)))
                            },
                        )
                        .into_any_element()
                } else {
                    div().text_color(t.warm).child(format!("freed {}", fmt::mem(freed))).into_any_element()
                })
            });
        if flash > 0. && c.now - flash < 3. {
            pill.with_animation(
                SharedString::from(format!("landed-{flash}")),
                gpui::Animation::new(std::time::Duration::from_millis(1600)),
                move |d, k| {
                    let a = if k < 0.2 { k / 0.2 } else { 1. - (k - 0.2) / 0.8 };
                    d.bg(accent.alpha(0.22 * a))
                },
            )
            .into_any_element()
        } else {
            pill.into_any_element()
        }
    }

    /// Where "Stopped today" sits, for the stop flight to land on.
    pub fn stopped_target(&self, w: f32, g: Geo) -> (f32, f32) {
        (w - g.pad - 90., ui::header() + 26.)
    }

    /// "N chats are open but not in use": only when there are some (and they were not put off with "Not now"). Review
    /// lists them with a Stop each.
    fn spare_strip(&self, c: &Ctx, g: Geo) -> Option<AnyElement> {
        let t = c.t;
        let cx = c.cx;
        let cl = c.app.snap.cleanup.as_ref()?;
        let mine = |m: &str| self.solo.as_deref().is_none_or(|s| s == m);
        let items: Vec<ApiCleanItem> = cl.items.iter().filter(|d| mine(&d.machine)).cloned().collect();
        if items.is_empty() {
            return None;
        }
        let keys: Vec<String> = items.iter().map(clean_key).collect();
        if keys.iter().all(|k| c.app.clean_later.contains(k)) {
            return None;
        }
        let n = items.len();
        let mb: f64 = items.iter().filter_map(|d| d.mb).sum();
        let open = self.spare_open;
        let head = if n == 1 {
            "1 chat is open but not in use.".to_string()
        } else {
            format!("{n} chats are open but not in use.")
        };
        let rest = format!(
            "{}Stopping one frees its memory, and you can resume it later.",
            if mb > 0. {
                format!("{} {}. ", if n == 1 { "It holds" } else { "Together they hold" }, fmt::mem(mb))
            } else {
                String::new()
            }
        );
        let mut strip = div()
            .flex_none()
            .mx(px(g.pad))
            .mb(px(10.))
            .rounded(px(ui::R_POP))
            .bg(t.panel)
            .border_1()
            .border_color(t.line)
            .flex()
            .flex_col()
            .child(
                div()
                    .min_h(px(44.))
                    .pl(px(14.))
                    .pr(px(8.))
                    .py(px(6.))
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .child(ui::icon("mem", 16., t.faint))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(px(13.))
                            .line_height(relative(1.4))
                            .text_color(t.ink2)
                            .child(ui::runs_text(
                                &[
                                    sb_core::TextRun { text: head, bold: true },
                                    sb_core::TextRun { text: format!(" {rest}"), bold: false },
                                ],
                                t.ink,
                            )),
                    )
                    .when(open && n > 1, |d| {
                        let all = items.clone();
                        d.child(self.stop_btn(
                            c,
                            "spare-all".into(),
                            &format!("Stop all {n}"),
                            format!("{n} chats"),
                            Some(mb),
                            move |this, cx| {
                                let all = all.clone();
                                this.app.update(cx, |s, cx| s.clean_stop(all, cx));
                            },
                        ))
                    })
                    .when(open, |d| {
                        let later = keys.clone();
                        d.child(
                            ui::ghost(&t, "spare-later", Some("Not now"), None)
                                .h(px(28.))
                                .px(px(10.))
                                .text_size(px(13.))
                                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                    let k = later.clone();
                                    this.spare_open = false;
                                    this.app.update(cx, |s, cx| s.clean_fold(k, true, cx));
                                })),
                        )
                    })
                    .child(
                        ui::line_btn(&t, "spare-toggle", 30.)
                            .px(px(12.))
                            .text_size(px(13.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(ui::icon(if open { "down" } else { "chev" }, 13., t.dim))
                            .child(if open { "Hide" } else { "Review" })
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.spare_open = !this.spare_open;
                                cx.notify();
                            })),
                    ),
            );
        if open {
            // Two columns when there is room, each row with what it holds and its own Stop.
            let cols = if g.wide { 2 } else { 1 };
            let mut grid =
                div().px(px(14.)).pb(px(6.)).border_t_1().border_color(t.line).flex().flex_wrap().gap_x(px(18.));
            for d in items {
                grid = grid.child(
                    div()
                        .w(relative(1. / cols as f32))
                        .max_w(if cols == 2 { relative(0.485) } else { relative(1.) })
                        .child(self.spare_row(c, d)),
                );
            }
            strip = strip.child(grid);
        }
        Some(strip.into_any_element())
    }

    fn spare_row(&self, c: &Ctx, d: ApiCleanItem) -> AnyElement {
        let t = c.t;
        let k = clean_key(&d);
        let busy = c.app.clean_busy.contains(&k);
        let title = match (&d.sid, &d.title) {
            (None, t) => t.clone().unwrap_or_else(|| "Leftover Claude process".into()),
            (Some(_), Some(t)) => names::split(t).0.to_string(),
            (Some(_), None) => "Untitled".into(),
        };
        let board_key =
            d.sid.as_ref().map(|s| format!("{}:{s}", d.machine)).filter(|k| c.app.snap.session(k).is_some());
        let mut meta = d.label.clone().unwrap_or_else(|| d.machine.clone());
        if let Some(w) = d.why.as_deref().filter(|w| !w.is_empty()) {
            meta.push_str(&format!(", {w}"));
        }
        if let Some(mb) = d.mb.filter(|m| *m > 0.) {
            meta.push_str(&format!(", {}", fmt::mem(mb)));
        }
        let hover = t.hover;
        let keep = d.clone();
        let stop = d.clone();
        div()
            .id(SharedString::from(format!("sp-{k}")))
            .min_h(px(48.))
            .py(px(6.))
            .flex()
            .items_center()
            .gap(px(10.))
            .border_b_1()
            .border_color(t.line)
            .when(busy, |e| e.opacity(0.6))
            .child(ui::lamp(&t, Lamp::Idle, 7.))
            .child(
                div()
                    .id(SharedString::from(format!("sp-n-{k}")))
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(1.))
                    .rounded(px(ui::R_ITEM))
                    .when_some(board_key, |e, bk| {
                        e.cursor_pointer().hover(move |s| s.bg(hover)).on_click(
                            c.cx.listener(move |this, _: &ClickEvent, _, cx| this.select(Some(bk.clone()), cx)),
                        )
                    })
                    .child(
                        ui::wrap(title.clone()).text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).text_color(t.ink),
                    )
                    .child(ui::wrap(meta).text_size(px(12.)).text_color(t.faint)),
            )
            .child(
                ui::ghost(&t, SharedString::from(format!("sp-k-{k}")), Some("Keep"), None)
                    .h(px(28.))
                    .px(px(10.))
                    .text_size(px(12.5))
                    .on_click(c.cx.listener(move |this, _: &ClickEvent, _, cx| {
                        let d = keep.clone();
                        this.app.update(cx, |s, cx| s.clean_keep(d, cx));
                    })),
            )
            .child(if busy {
                div().text_size(px(12.5)).text_color(t.faint).child("Stopping").into_any_element()
            } else {
                self.stop_btn(c, format!("sp-{k}"), "Stop", title, d.mb, move |this, cx| {
                    let d = stop.clone();
                    this.app.update(cx, |s, cx| s.clean_stop(vec![d], cx));
                })
            })
            .into_any_element()
    }

    fn lane_head(&self, c: &Ctx, col: Column, n: usize) -> AnyElement {
        let t = c.t;
        let ink = if col == Column::Need { t.need } else { t.faint };
        div()
            .h(px(38.))
            .px(px(10.))
            .flex()
            .items_center()
            .gap(px(8.))
            .text_size(px(13.))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(ink)
            .child(ui::lamp(&t, col.lamp(), 7.))
            .child(div().whitespace_nowrap().child(col_name(col)))
            .child(ui::spacer())
            .child(div().text_color(if n > 0 { ink } else { t.faint }).child(n.to_string()))
            .into_any_element()
    }

    /// One machine: its column on the left (name, OS and chats, memory, New terminal), then its five cells.
    fn machine_row(&self, c: &Ctx, lane: &board::Lane, g: Geo, shut: bool) -> AnyElement {
        let t = c.t;
        let m = c.app.snap.machine(&lane.machine);
        let offline = m.is_some_and(|m| m.is_offline());
        let label = (g.label > 0.).then(|| self.machine_cell(c, &lane.machine, lane, g, shut));
        let cells: Vec<AnyElement> = if offline {
            let name = m.map(|m| m.label.clone()).unwrap_or_else(|| lane.machine.clone());
            let n: usize = lane.cells.iter().map(|c| c.len()).sum();
            vec![
                div()
                    .py(px(14.))
                    .px(px(6.))
                    .text_size(px(13.))
                    .text_color(t.faint)
                    .child(if n > 0 {
                        format!("{name} is offline. Its {} will show here when it is back.", plural(n, "chat"))
                    } else {
                        format!("{name} is offline.")
                    })
                    .into_any_element(),
            ]
        } else if shut {
            lane.cells
                .iter()
                .enumerate()
                .map(|(i, keys)| {
                    div()
                        .h(px(44.))
                        .px(px(10.))
                        .flex()
                        .items_center()
                        .gap(px(7.))
                        .when(!keys.is_empty(), |d| {
                            d.child(ui::lamp(&t, Column::ALL[i].lamp(), 7.))
                                .child(div().text_size(px(13.)).text_color(t.dim).child(keys.len().to_string()))
                        })
                        .into_any_element()
                })
                .collect()
        } else {
            lane.cells.iter().enumerate().map(|(i, keys)| self.cell(c, &lane.machine, Column::ALL[i], keys)).collect()
        };
        let row = if offline {
            let mut r = div().flex().flex_row().items_start().gap(px(10.));
            if let Some(l) = label {
                r = r.child(l);
            }
            r.child(div().flex_1().min_w_0().children(cells))
        } else {
            crate::view::grid_row(label, cells)
        };
        div().border_b_1().border_color(t.line).child(row).into_any_element()
    }

    fn machine_cell(&self, c: &Ctx, machine: &str, lane: &board::Lane, g: Geo, shut: bool) -> AnyElement {
        let t = c.t;
        let cx = c.cx;
        let m = c.app.snap.machine(machine);
        let label = m.map(|m| m.label.clone()).unwrap_or_else(|| machine.to_string());
        let offline = m.is_some_and(|m| m.is_offline());
        let ml = m.map(|m| m.lamp()).unwrap_or(sb_core::MachineLamp::Warn);
        let n = lane.tally.n as usize;
        let os = os_name(m.and_then(|m| m.os.as_deref()));
        let line1 = if offline {
            "Offline".to_string()
        } else if os.is_empty() {
            plural(n, "chat")
        } else {
            format!("{os}, {}", plural(n, "chat"))
        };
        let mem = m
            .and_then(|m| m.mem.as_ref())
            .filter(|x| x.claude_mb > 0.)
            .map(|x| format!("Claude {}", fmt::mem(x.claude_mb)));
        let can = m.and_then(|m| m.can.clone()).filter(|x| !x.is_empty());
        let (m1, m2, m3) = (machine.to_string(), machine.to_string(), machine.to_string());
        div()
            .w(px(g.label))
            .flex_none()
            .pt(px(14.))
            .pb(px(14.))
            .pr(px(12.))
            .pl(px(4.))
            .flex()
            .flex_col()
            .gap(px(4.))
            .child(
                div()
                    .flex()
                    .items_start()
                    .gap(px(8.))
                    .child(div().flex_none().mt(px(6.)).size(px(8.)).rounded_full().bg(t.machine(ml)))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(14.5))
                            .line_height(relative(1.35))
                            .font_weight(FontWeight::BOLD)
                            .text_color(t.ink)
                            .child(label),
                    ) // one line, never broken mid-word
                    .child(
                        ui::icon_btn(&t, SharedString::from(format!("tear-{machine}")), "popout", 22.)
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.tear(m2.clone(), cx))),
                    )
                    .child(
                        ui::icon_btn(
                            &t,
                            SharedString::from(format!("fold-{machine}")),
                            if shut { "chev" } else { "down" },
                            22.,
                        )
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            let s = this.app.read(cx).is_shut(&m3);
                            let m = m3.clone();
                            this.app.update(cx, |a, cx| a.set_shut(&m, !s, cx));
                        })),
                    ),
            )
            .when(!shut, |d| {
                d.child(
                    div().pl(px(16.)).text_size(px(12.5)).line_height(relative(1.45)).text_color(t.faint).child(line1),
                )
                .when_some(mem.filter(|_| !offline), |d, mem| {
                    d.child(
                        div()
                            .pl(px(16.))
                            .text_size(px(12.5))
                            .line_height(relative(1.45))
                            .text_color(t.faint)
                            .child(mem),
                    )
                })
                .when_some(can.filter(|_| !offline), |d, can| {
                    d.child(ui::wrap(can).pl(px(16.)).text_size(px(12.)).line_height(relative(1.4)).text_color(t.faint))
                })
                .when(!offline, |d| {
                    d.child(
                        ui::ghost(&t, SharedString::from(format!("new-{machine}")), None, None)
                            .mt(px(6.))
                            .ml(px(10.))
                            .h(px(28.))
                            .text_size(px(13.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(t.ink2)
                            .child(ui::icon("plus", 14., t.ink2))
                            .child("New terminal")
                            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                                this.open_new(Some(m1.clone()), window, cx)
                            })),
                    )
                })
            })
            .into_any_element()
    }

    fn cell(&self, c: &Ctx, machine: &str, col: Column, keys: &[String]) -> AnyElement {
        let t = c.t;
        let mut cell = div()
            .py(px(10.))
            .px(px(if c.card_w < 160. { 4. } else { 6. }))
            .flex()
            .flex_col()
            .gap(px(if col.quiet() { 2. } else { 6. }))
            .min_w_0();
        if col.quiet() {
            // Quiet keeps only the last day's idle chats, a few at a time; everything older is in History.
            let recent: Vec<&String> = keys
                .iter()
                .filter(|k| c.app.snap.session(k).is_some_and(|s| c.now - s.active < QUIET_RECENT))
                .collect();
            let open = self.open.contains(machine) || self.solo.is_some();
            let shown = if open { recent.len() } else { recent.len().min(QUIET_CAP) };
            for k in &recent[..shown] {
                if let Some(s) = c.app.snap.session(k) {
                    cell = cell.child(self.quiet_row(c, s));
                }
            }
            let hover = t.ink;
            let more = |id: String, label: String| {
                div()
                    .id(SharedString::from(id))
                    .px(px(6.))
                    .py(px(4.))
                    .text_size(px(12.5))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(t.faint)
                    .cursor_pointer()
                    .hover(move |s| s.text_color(hover))
                    .child(label)
            };
            if recent.len() > QUIET_CAP && self.solo.is_none() {
                let m = machine.to_string();
                cell = cell.child(
                    more(
                        format!("q-more-{machine}"),
                        if open {
                            "Show fewer".to_string()
                        } else {
                            format!("{} more today", recent.len() - QUIET_CAP)
                        },
                    )
                    .on_click(c.cx.listener(move |this, _: &ClickEvent, _, cx| {
                        if !this.open.remove(&m) {
                            this.open.insert(m.clone());
                        }
                        cx.notify();
                    })),
                );
            }
            let older = keys.len() - recent.len();
            if older > 0 && self.solo.is_none() {
                cell = cell.child(
                    more(format!("q-hist-{machine}"), format!("{} older in History", older))
                        .on_click(c.cx.listener(|this, _: &ClickEvent, _, cx| this.view_history(cx))),
                );
            }
        } else {
            for k in keys {
                if let Some(s) = c.app.snap.session(k) {
                    cell = cell.child(self.card(c, s, col));
                }
            }
        }
        cell.into_any_element()
    }

    /// A chat card: its name and live state pill, one line of state, the context size and the flags button; when it
    /// waits, the kind of ask, Claude's summary (two lines at most) and one button (askcard.rs). How the chat was born ("part 2", "copy") is not on the card: the details sheet's History has it.
    fn card(&self, c: &Ctx, s: &Session, col: Column) -> AnyElement {
        let t = c.t;
        let now = c.now;
        let on = self.sel.as_deref() == Some(s.key.as_str());
        let job = c.app.job_for(&s.key);
        let key = s.key.clone();
        // Waiting more than a day: still in Needs you, but muted and not counted (model.rs PARKED).
        let parked = col == Column::Need && s.parked(now);
        let needs = col == Column::Need && !parked;
        // A waiting chat: its kind's tag on the state line, then Claude's summary (two lines) and one button.
        let ask = if col == Column::Need && job.is_none() { s.ask_now(now) } else { None };
        // The one live mapping (sb_core::live): the pill, the cost badge, and the words for warm, cold and quiet.
        let live = c.app.live_of(s, now, None);
        let stale = live.stale.is_some();
        let (state, state_c): (String, gpui::Hsla) = if let Some(j) = job {
            (j.state_word().to_string(), if j.waiting.is_some() { t.dim } else { t.run })
        } else {
            match col {
                Column::Need if s.held => ("Message on hold".into(), t.need),
                Column::Need if ask.is_some() && parked => {
                    let d = s.wait_days(now);
                    (format!("Waiting {d} {}", if d == 1 { "day" } else { "days" }), t.dim)
                }
                Column::Need if ask.is_some() => {
                    (format!("Waiting {}", fmt::mins((now - s.active).max(60.) as u64)), t.need)
                }
                Column::Need if parked => {
                    (s.wait_line(now, Some(sb_core::model::NEEDS_CARD)).unwrap_or_else(|| "Waiting".into()), t.dim)
                }
                Column::Need => match s.wait_line(now, Some(sb_core::model::NEEDS_CARD)) {
                    Some(line) => (line, t.need),
                    None => (format!("Waiting {}", fmt::mins((now - s.active).max(60.) as u64)), t.need),
                },
                Column::Run if s.stalled => (live.text(), t.dim),
                Column::Run => ("Working".into(), t.ink2),
                Column::Warm => (cap(&live.detail), if live.urgent { t.cold } else { t.ink2 }),
                // What each choice costs right now (the agent's model); the detail sheet says how it is estimated.
                Column::Cold => match &s.cost {
                    Some(c) => (
                        format!(
                            "Resume {}, fresh {}",
                            fmt::k(c.resume.cold.round() as u64),
                            fmt::k(c.fresh.total.round() as u64)
                        ),
                        t.cold,
                    ),
                    None => (format!("Re-reads {}", s.ctx_label()), t.cold),
                },
                Column::Quiet if live.kind == sb_core::live::LiveKind::Cold => (cap(&live.detail), t.dim),
                Column::Quiet => (s.phrase(now), t.dim),
            }
        };
        // Old data: the line says how old instead (the pill keeps the last word, in grey).
        let (state, state_c) = match &live.stale {
            Some(a) if job.is_none() => (cap(a), t.faint),
            _ => (state, state_c),
        };
        // The pill says whether going back is cheap, whatever column the chat is in: a chat waiting 41 hours is cold.
        let pill: Option<(&str, gpui::Hsla)> =
            if job.is_some() { None } else { live.pill.map(|w| (w, crate::chrome::pill_color(&t, &live))) };
        // A big chat says so: "expensive", or what the next message re-reads once it is cold (the Cold column's line
        // already gives the cost, so its cards skip it).
        let cost = live.expensive.clone().filter(|_| job.is_none() && col != Column::Cold);
        let tag = s.tag.as_ref().filter(|_| s.tag_kind.is_some_and(|k| k.on_card()));
        let line2 = t.line2;
        let accent = t.accent;
        let need = t.need;
        let mut ring = vec![Theme::ring(if on { accent } else { t.line })];
        if on {
            ring.push(gpui::BoxShadow {
                color: accent,
                offset: gpui::point(px(0.), px(0.)),
                blur_radius: px(0.),
                spread_radius: px(2.),
                inset: true,
            });
        }
        if needs {
            // An inset edge rather than a clipped bar: it follows the card's rounded corners, and sits over the
            // selection ring so a selected card still shows it needs you.
            ring.push(gpui::BoxShadow {
                color: need,
                offset: gpui::point(px(3.), px(0.)),
                blur_radius: px(0.),
                spread_radius: px(0.),
                inset: true,
            });
        }
        let quick = ask.as_ref().map(|a| self.card_quick(c, s, a, if parked { t.dim } else { t.ink2 }));
        div()
            .id(SharedString::from(format!("card-{}", s.key)))
            .relative()
            .rounded(px(ui::R_CARD))
            .bg(t.panel)
            .pt(px(9.))
            .pb(px(8.))
            .pl(px(12.))
            .pr(px(10.))
            .flex()
            .flex_col()
            .gap(px(3.))
            .overflow_hidden()
            .shadow(ring)
            .when(!on, |d| {
                d.hover(move |st| {
                    let mut h = vec![Theme::ring(line2)];
                    if needs {
                        h.push(gpui::BoxShadow {
                            color: need,
                            offset: gpui::point(px(3.), px(0.)),
                            blur_radius: px(0.),
                            spread_radius: px(0.),
                            inset: true,
                        });
                    }
                    st.shadow(h)
                })
            })
            .cursor_pointer()
            .on_mouse_down(
                MouseButton::Right,
                c.cx.listener({
                    let k = s.key.clone();
                    move |this, ev: &gpui::MouseDownEvent, _, cx| this.open_menu(k.clone(), ev.position, cx)
                }),
            )
            .on_click(c.cx.listener(move |this, ev: &ClickEvent, _, cx| {
                if ev.click_count() >= 2 {
                    this.enter(key.clone(), cx);
                } else {
                    this.select(Some(key.clone()), cx);
                }
            }))
            .child(
                // The name wraps; a mode tag (UltraCode, for Studio) follows where it ends; the state pill sits top right.
                div()
                    .flex()
                    .items_start()
                    .gap_x(px(8.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .gap_x(px(6.))
                            .gap_y(px(3.))
                            .child(
                                div()
                                    .max_w_full()
                                    .text_size(px(13.5))
                                    .line_height(relative(1.38))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(t.ink)
                                    .child(s.topic.clone()),
                            )
                            .when_some(tag, |d, tg| d.child(ui::name_tag(&t, tg, s.tag_kind))),
                    )
                    // In a tab group: a small dot in its color, apart from the star and the pill.
                    .when_some(c.app.snap.group_of(s).map(|g| g.color), |d, gc| {
                        d.child(crate::tabgroups::group_dot(&t, gc, 7.).mt(px(7.)).mr(px(1.)))
                    })
                    // Starred: a small filled star beside the pill (the name keeps the whole left side).
                    .when(s.star, |d| d.child(div().flex_none().mt(px(5.)).child(ui::icon("star-on", 12., t.cold))))
                    .when_some(pill, |d, (w, pc)| d.child(ui::state_pill(&t, w, pc).mt(px(1.)))),
            )
            .child(
                // One line: the state, the size when there is room, and the flags button at the right edge.
                div()
                    .min_h(px(24.))
                    .flex()
                    .items_center()
                    .gap_x(px(6.))
                    .text_size(px(12.))
                    .text_color(t.faint)
                    // The state wraps rather than push the flags button out of a narrow card.
                    .when(stale && job.is_none(), |d| d.child(ui::icon("clock", 11., t.faint).flex_none()))
                    .child(
                        ui::wrap(state)
                            .flex_shrink(1.)
                            .min_w_0()
                            .text_color(state_c)
                            .when(needs && !stale, |d| d.font_weight(FontWeight::SEMIBOLD)),
                    )
                    // The context size goes on narrow windows (the state line says what matters), as in the design.
                    // A big chat's badge takes its place: it names the cost the size stands for.
                    .when_some(cost.clone().filter(|_| c.card_w >= 170.), |d, e| {
                        d.child(crate::chrome::cost_badge(&t, &e, stale))
                    })
                    .when(cost.is_none() && col != Column::Cold && s.ctx > 0 && c.card_w >= 190., |d| {
                        d.child(div().whitespace_nowrap().child(s.ctx_label()))
                    })
                    .child(ui::spacer())
                    .child(div().flex_none().child(self.flags_btn(c, s))),
            )
            .when_some(quick, |d, q| d.child(div().mt(px(2.)).child(q)))
            .into_any_element()
    }

    /// The flag button on a card: how many launch flags it runs with; it opens the flags drawer.
    pub fn flags_btn(&self, c: &Ctx, s: &Session) -> AnyElement {
        let t = c.t;
        let key = s.key.clone();
        let hover = t.hover;
        div()
            .id(SharedString::from(format!("flags-{}", s.key)))
            .flex_none()
            .h(px(22.))
            .px(px(4.))
            .mr(px(-5.))
            .flex()
            .items_center()
            .gap(px(3.))
            .rounded(px(ui::R_CHIP))
            .text_size(px(12.))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(t.dim)
            .cursor_pointer()
            .hover(move |st| st.bg(hover))
            .child(ui::icon("flag", 12., t.dim))
            .child(s.flags.count().to_string())
            .on_click(c.cx.listener(move |this, _: &ClickEvent, _, cx| {
                cx.stop_propagation();
                this.open_flags(key.clone(), cx);
            }))
            .into_any_element()
    }

    fn quiet_row(&self, c: &Ctx, s: &Session) -> AnyElement {
        let t = c.t;
        let on = self.sel.as_deref() == Some(s.key.as_str());
        let key = s.key.clone();
        let hover = t.hover;
        // The time sits beside the name when the Quiet column has room, under it when it does not.
        let wide = c.card_w * 0.85 >= 240.;
        let when = fmt::when(s.active, c.now);
        let live = c.app.live_of(s, c.now, None);
        let (pill, pill_c) = (live.pill, crate::chrome::pill_color(&t, &live));
        div()
            .id(SharedString::from(format!("q-{}", s.key)))
            .px(px(6.))
            .py(px(5.))
            .rounded(px(ui::R_ITEM))
            .flex()
            .when(wide, |d| d.items_start().gap(px(8.)))
            .when(!wide, |d| d.flex_col())
            .text_size(px(13.))
            .text_color(if on { t.ink } else { t.ink2 })
            .cursor_pointer()
            .when(on, |d| d.bg(t.sel))
            .when(!on, |d| d.hover(move |st| st.bg(hover)))
            .on_mouse_down(
                MouseButton::Right,
                c.cx.listener({
                    let k = s.key.clone();
                    move |this, ev: &gpui::MouseDownEvent, _, cx| this.open_menu(k.clone(), ev.position, cx)
                }),
            )
            .on_click(c.cx.listener(move |this, ev: &ClickEvent, _, cx| {
                if ev.click_count() >= 2 {
                    this.enter(key.clone(), cx);
                } else {
                    this.select(Some(key.clone()), cx);
                }
            }))
            .child(ui::wrap(s.topic.clone()).flex_1().min_w_0().line_height(relative(1.35)))
            .child(div().flex_none().pt(px(1.)).text_size(px(12.)).text_color(t.faint).child(if s.reaped_note() {
                format!("Stopped, {when}")
            } else {
                when
            }))
            // A small chat gone cold stays here, and says so with the same pill a card would show.
            .when(pill == Some("cold"), |d| d.child(ui::state_pill(&t, "cold", pill_c).flex_none()))
            .into_any_element()
    }

    /// The footer under the board: key hints, then Files, History and Replaced, on the same grid as the header.
    pub fn statusbar(&self, c: &Ctx, w: f32) -> impl IntoElement {
        let t = c.t;
        let cx = c.cx;
        let rep = c.app.replaced().len();
        let hint = |keys: &[&str], text: &str| {
            div()
                .flex()
                .items_center()
                .gap(px(3.))
                .children(keys.iter().map(|k| ui::kbd(&t, k)))
                .child(div().ml(px(4.)).child(text.to_string()))
        };
        let full = w >= 1280.;
        let link = |id: &'static str, ic: &'static str, label: String| {
            ui::ghost(&t, id, None, None)
                .h(px(24.))
                .px(px(8.))
                .text_size(px(12.5))
                .text_color(t.dim)
                .child(ui::icon(ic, 13., t.faint))
                .child(label)
        };
        div()
            .flex_none()
            .h(px(ui::FOOTER))
            .flex()
            .items_center()
            .gap(px(16.))
            .pl(px(ui::lead()))
            .pr(px(10.))
            .bg(t.bg2)
            .border_t_1()
            .border_color(t.rule)
            .text_size(px(12.5))
            .text_color(t.faint)
            .child(hint(&[&ui::mod_cap("K")], "commands"))
            .child(hint(&[&ui::mod_cap("T")], "terminal"))
            .child(hint(&["N"], "new"))
            .when(full, |d| {
                d.child(hint(&["J", "K"], "chats")).child(hint(&["H", "L"], "columns")).child(hint(&["F"], "fresh"))
            })
            .child(ui::spacer())
            .child(
                link("sb-files", "folder", "Files".into())
                    .on_click(cx.listener(|_, _: &ClickEvent, _, cx| crate::files::open_files(cx))),
            )
            .child(
                link("sb-history", "history", "History".into())
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.view_history(cx))),
            )
            .when(rep > 0, |d| {
                d.child(link("sb-rep", "swap", format!("Replaced {rep}")).on_click(cx.listener(
                    |this, _: &ClickEvent, window, cx| {
                        this.open_archive(crate::archive::ArcTab::Replaced, None, window, cx)
                    },
                )))
            })
            .children(self.notices_link(c))
    }
}

/// "Cache good for 4:12": a shared detail as the start of a line.
fn cap(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().chain(c).collect(),
        None => String::new(),
    }
}
