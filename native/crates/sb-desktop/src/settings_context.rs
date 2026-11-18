//! Settings > Context window: how big a chat may grow before Claude Code compacts it. One setting (the slider), the
//! words for where it is (sb_core::compact), what follows from it, and what each computer has right now.
//!
//! The page is its own view so the settings window only has to place it. A change is shown at once and saved a
//! moment later, so walking the stops with the arrow keys sends the last one only.

use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use gpui::{
    AnyElement, App, AppContext as _, ClickEvent, Context, Entity, FontWeight, IntoElement, ParentElement, Render,
    SharedString, StatefulInteractiveElement, Styled, Subscription, Task, Window, div, px, relative,
};
use sb_core::api::{ApiMachineRow, ApiPrefs};
use sb_core::compact::{self, Choice, Tone};

use crate::settings::SettingsView;
use crate::slider::{Stop, StopEvent, StopSlider};
use crate::state::AppState;
use crate::theme::{NUM, Theme};
use crate::ui;

/// The server's switch behind the slider: on is every stop but the last.
const SWITCH: &str = "setup_compact_window";
/// A choice waits this long before it is sent, so only the last of a quick run goes out.
const SEND_AFTER: Duration = Duration::from_millis(350);
/// After a change the computers are asked again this often, this many times, while any has yet to follow.
const WATCH_EVERY: Duration = Duration::from_millis(2500);
const WATCH_TIMES: u8 = 16;

/// Saves in order, last one wins. They outlive the page (leaving Settings right after a change still saves it), so
/// the count is kept here and not on the view.
static NEWEST: AtomicU64 = AtomicU64::new(0);
static WAITING: AtomicU64 = AtomicU64::new(0);
static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

/// What the server says is chosen: the value when it has one, else the old switch (on meant 300k).
fn saved(p: &ApiPrefs) -> Option<Choice> {
    match &p.compact {
        Some(c) => Some(Choice::of(c)),
        None => p.option(SWITCH).map(|o| Choice::from_server(o.on, None)),
    }
}

/// Shows the choice here at once and sends it after a moment; a newer choice replaces one still waiting.
fn save(app: &mut AppState, choice: Choice, cx: &mut Context<AppState>) {
    if let Some(p) = app.prefs.as_mut() {
        p.set_option(SWITCH, choice != Choice::Default);
        if let Some(c) = p.compact.as_mut() {
            c.on = choice != Choice::Default;
            if let Choice::At(v) = choice {
                c.at = v;
            }
        }
    }
    let mine = NEWEST.fetch_add(1, Ordering::SeqCst) + 1;
    WAITING.fetch_add(1, Ordering::SeqCst);
    let body = choice.body();
    cx.spawn(async move |this, cx| {
        cx.background_executor().timer(SEND_AFTER).await;
        let sent = this.update(cx, |s, cx| {
            s.call(
                cx,
                move |core| {
                    let _one = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
                    if NEWEST.load(Ordering::SeqCst) != mine {
                        return Ok(false); // a newer choice is on its way: this one is never sent
                    }
                    core.set_prefs(&body).map(|_| true)
                },
                |st, res, cx| {
                    WAITING.fetch_sub(1, Ordering::SeqCst);
                    match res {
                        Ok(true) => {
                            st.toast("Saved", None, false, cx);
                            st.load_prefs(|_, _| {}, cx);
                            st.refresh_in(Duration::from_millis(200), cx);
                        }
                        Ok(false) => {}
                        Err(e) => {
                            // not saved: say so, and show what the server really has
                            st.toast(e.to_string(), None, true, cx);
                            st.load_prefs(|_, _| {}, cx);
                        }
                    }
                    cx.notify();
                },
            )
        });
        if sent.is_err() {
            WAITING.fetch_sub(1, Ordering::SeqCst);
        }
    })
    .detach();
}

pub struct ContextPage {
    app: Entity<AppState>,
    slider: Entity<StopSlider>,
    /// While the thumb is dragged: the stop it is nearest. Nothing is chosen yet.
    preview: Option<Choice>,
    /// Chosen here and not yet answered by the server: shown until the save lands, whatever an older read says.
    held: Option<Choice>,
    /// Each computer as the server last saw it.
    machines: Option<Result<Vec<ApiMachineRow>, String>>,
    watch: Option<Task<()>>,
    _subs: Vec<Subscription>,
}

impl ContextPage {
    pub fn new(app: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let stops = (0..Choice::positions())
            .map(|i| {
                let c = Choice::at_position(i);
                Stop { label: c.stop_label().into(), recommended: c.is_recommended() }
            })
            .collect();
        let start = app.read(cx).prefs.as_ref().and_then(saved).unwrap_or(Choice::At(compact::RECOMMENDED));
        let slider = cx.new(|cx| StopSlider::new(stops, start.position(), window, cx));
        let _subs = vec![
            cx.subscribe(&slider, |this: &mut Self, _, ev: &StopEvent, cx| match ev {
                StopEvent::Preview(i) => {
                    this.preview = Some(Choice::at_position(*i));
                    cx.notify();
                }
                StopEvent::Commit(i) => this.choose(Choice::at_position(*i), cx),
            }),
            cx.observe(&app, |_, _, cx| cx.notify()),
        ];
        let mut page = ContextPage { app, slider, preview: None, held: None, machines: None, watch: None, _subs };
        page.watch(1, cx);
        page
    }

    fn choose(&mut self, choice: Choice, cx: &mut Context<Self>) {
        self.preview = None;
        let now = self.held.or_else(|| self.app.read(cx).prefs.as_ref().and_then(saved));
        if now != Some(choice) {
            self.held = Some(choice);
            self.app.update(cx, |s, cx| save(s, choice, cx));
        }
        cx.notify();
    }

    /// Reads the computers now, and again every few seconds while one of them has yet to follow the choice.
    fn watch(&mut self, times: u8, cx: &mut Context<Self>) {
        let core = self.app.read(cx).core.clone();
        self.watch = Some(cx.spawn(async move |this, cx| {
            for n in 0..times {
                if n > 0 {
                    cx.background_executor().timer(WATCH_EVERY).await;
                }
                let core = core.clone();
                let res = cx.background_executor().spawn(async move { core.machines() }).await;
                let more = this.update(cx, |v, cx| {
                    // a failed re-read keeps the last good list on screen
                    match res {
                        Ok(m) => v.machines = Some(Ok(m.machines)),
                        Err(e) if !matches!(v.machines, Some(Ok(_))) => v.machines = Some(Err(e.to_string())),
                        Err(_) => {}
                    }
                    cx.notify();
                    v.waiting(cx)
                });
                if !matches!(more, Ok(true)) {
                    break;
                }
            }
        }));
    }

    /// True while a save is on its way or a computer has yet to follow.
    fn waiting(&self, cx: &App) -> bool {
        if WAITING.load(Ordering::SeqCst) > 0 {
            return true;
        }
        let Some(choice) = self.app.read(cx).prefs.as_ref().and_then(saved) else { return false };
        match &self.machines {
            Some(Ok(ms)) => ms.iter().any(|m| {
                m.online != Some(false)
                    && compact::machine_line(choice, m.compact.as_ref(), m.has(compact::FEATURE), m.online).1
                        == Tone::Wait
            }),
            _ => false,
        }
    }

    /// The card with the slider: the name of the setting, where it is, and what that means.
    fn card(&self, t: &Theme, shown: Choice) -> AnyElement {
        let value = div()
            .flex_none()
            .font_family(NUM)
            .font_features(ui::tnum())
            .text_size(px(22.))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(t.ink)
            .child(ui::line(shown.value_label()));
        let name = div().text_size(px(12.5)).text_color(t.ink2).child(ui::line(shown.name()));
        div()
            .rounded(px(ui::R_CARD))
            .border_1()
            .border_color(t.line)
            .bg(t.bg)
            .flex()
            .flex_col()
            .child(
                div()
                    .px(px(14.))
                    .pt(px(13.))
                    .pb(px(14.))
                    .flex()
                    .flex_col()
                    .gap(px(14.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap(px(16.))
                            .h(px(40.))
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap(px(3.))
                                    .min_w_0()
                                    .child(
                                        div()
                                            .text_size(px(13.5))
                                            .font_weight(FontWeight::MEDIUM)
                                            .text_color(t.ink)
                                            .child(ui::line("Compact long chats at")),
                                    )
                                    .child(name),
                            )
                            .child(value),
                    )
                    .child(self.slider.clone())
                    .child(
                        ui::wrap(shown.sentence())
                            .text_size(px(13.))
                            .line_height(relative(1.5))
                            .text_color(t.ink2)
                            // three lines at the usual width: the card keeps its height from stop to stop
                            .min_h(px(59.)),
                    ),
            )
            .child(
                div()
                    .px(px(14.))
                    .py(px(10.))
                    .border_t_1()
                    .border_color(t.line)
                    .child(ui::wrap(shown.writes()).text_size(px(12.)).line_height(relative(1.45)).text_color(t.faint)),
            )
            .into_any_element()
    }

    /// One computer: its name, what it has, and a tag when it will not follow the choice.
    fn machine_row(t: &Theme, m: &ApiMachineRow, choice: Choice) -> AnyElement {
        let (line, tone) = compact::machine_line(choice, m.compact.as_ref(), m.has(compact::FEATURE), m.online);
        let ctl = match (tone, m.online) {
            (Tone::Wait, Some(false)) => None,
            (Tone::Wait, _) => {
                Some(ui::spinner(SharedString::from(format!("st-cx-wait-{}", m.name)), 14., t.dim).into_any_element())
            }
            (tone, _) => tone.tag().map(|w| ui::tag(t, w).into_any_element()),
        };
        let label = if m.label.is_empty() { m.name.clone() } else { m.label.clone() };
        ui::setting_row(t, label, Some(line.into()), ctl).into_any_element()
    }

    fn machines(&self, t: &Theme, choice: Choice) -> AnyElement {
        let rows: Vec<AnyElement> = match &self.machines {
            None => vec![ui::loading(t, "st-cx-load", "Asking each computer").into_any_element()],
            Some(Err(e)) => vec![
                ui::wrap(format!("The computers could not be read: {e}"))
                    .text_size(px(12.5))
                    .text_color(t.faint)
                    .into_any_element(),
            ],
            Some(Ok(ms)) if ms.is_empty() => vec![
                ui::wrap("No computers yet. Add one under Machines.")
                    .text_size(px(12.5))
                    .text_color(t.faint)
                    .into_any_element(),
            ],
            Some(Ok(ms)) => ms.iter().map(|m| Self::machine_row(t, m, choice)).collect(),
        };
        SettingsView::group(
            t,
            "On each computer",
            Some(
                "Switchboard sets the value only where no compact window was chosen by hand, and only ever changes or removes what it wrote.",
            ),
            rows,
        )
    }

    /// A server from before the value existed: it has the switch only, so the page offers that and says why.
    fn old_server(&self, t: &Theme, prefs: &ApiPrefs, cx: &Context<Self>) -> AnyElement {
        let Some(o) = prefs.option(SWITCH) else {
            return ui::setup_card(
                t,
                "info",
                "This server cannot set this yet",
                "Update the server to choose where long chats are compacted. Until then every computer keeps what its own Claude Code settings say.",
                None,
            )
            .into_any_element();
        };
        let on = o.on;
        let sw = ui::switch(t, "st-cx-old", on, false).on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
            this.app.update(cx, |s, cx| {
                if let Some(p) = s.prefs.as_mut() {
                    p.set_option(SWITCH, !on);
                }
                s.save_prefs(sb_core::api::option_body(SWITCH, !on), Some("Saved".into()), cx)
            });
        }));
        div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(ui::setup_card(
                t,
                "info",
                "This server only knows 300k",
                "It is older than this app: it can compact long chats at 300k or leave Claude Code's default, nothing between. Update the server to pick another value.",
                None,
            ))
            .child(
                div().rounded(px(ui::R_CARD)).border_1().border_color(t.line).bg(t.bg).px(px(14.)).py(px(11.)).child(
                    ui::option_row(t, o.label.clone(), o.explain.clone(), o.off.clone(), sw.into_any_element()),
                ),
            )
            .into_any_element()
    }
}

impl Render for ContextPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = Theme::get(cx);
        let head = SettingsView::head(
            &t,
            "Context window",
            "How much of a long chat Claude Code keeps word for word before it compacts it: replaces the older part with a summary. One setting, for every computer and every model.",
        );
        let Some(prefs) = self.app.read(cx).prefs.clone() else {
            let load = ui::loading(&t, "st-cx-prefs", "Reading your choice from the server");
            return div().child(head).child(load);
        };
        let server = saved(&prefs);
        // a save that landed, or one that failed: the server's word counts again
        if WAITING.load(Ordering::SeqCst) == 0 && self.held.take().is_some() {
            self.watch(WATCH_TIMES, cx); // and each computer is asked again, now that the server has it
        }
        let choice = self.held.or(server).unwrap_or(Choice::Default);
        let shown = self.preview.unwrap_or(choice);
        if prefs.compact.is_none() {
            // an older server passes on nothing about each computer, so there is no list to show
            return div().child(head).child(self.old_server(&t, &prefs, cx));
        }
        // the thumb follows the saved value when it changes somewhere else
        let (at, dragging) = {
            let s = self.slider.read(cx);
            (s.pos(cx), s.dragging())
        };
        if !dragging && self.preview.is_none() && (at - choice.position()).abs() > 0.001 {
            self.slider.update(cx, |s, cx| s.set_pos(choice.position(), window, cx));
        }
        let follows: Vec<AnyElement> = shown
            .follows()
            .into_iter()
            .map(|(label, value)| {
                let v = div()
                    .max_w(px(300.))
                    .text_size(px(13.))
                    .text_color(t.ink2)
                    .font_features(ui::tnum())
                    .child(ui::wrap(value));
                ui::setting_row(&t, label, None, Some(v.into_any_element())).into_any_element()
            })
            .collect();
        div().child(head).child(self.card(&t, shown)).child(self.machines(&t, choice)).child(SettingsView::group(
            &t,
            "Follows from this",
            Some("Not settings of their own: they move with the slider, chat by chat."),
            follows,
        ))
    }
}
