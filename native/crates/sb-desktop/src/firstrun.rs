//! The empty board when there is no server to show.
//!
//! First run (nothing saved): "Searching for local servers" while the Bonjour search runs (sb_core::discover), each
//! server listed as it answers. Exactly one: the app connects to it by itself. Several: the user picks. None: "No
//! Switchboard server on this network", with "Run the server on this computer" (the guided setup, serversetup.rs)
//! and "Type an address". What to show is decided in sb_core::firstrun, which has the tests.
//!
//! A saved address that does not answer: the address field as before, and Search again.
//!
//! With a node on this computer that can join a home with a code (sb_core::keys), the first run asks for the code
//! instead: "Join your home computer", with the address flow under Advanced as "Connect by address". Without such a
//! node none of that shows and the screens are the ones above.

use std::time::Duration;

use gpui::{
    AnyElement, ClickEvent, Context, FontWeight, IntoElement, ParentElement, StatefulInteractiveElement, Styled,
    Window, div, prelude::*, px, relative,
};
use sb_core::firstrun::{Found, SEARCH_FOR, Show};
use sb_core::keys::{self, JoinStep, Keys, Tone};

use crate::theme::{NUM, Theme};
use crate::ui;
use crate::view::{BoardView, Ctx};

/// The setup guide: the Install section of the project's README.
fn guide() -> String {
    ui::install_guide()
}

/// What the search thread reports.
enum Heard {
    Found(sb_core::discover::Announced),
    Here(String, Vec<String>),
    Done(Option<String>),
}

/// The width of the empty board's column.
const COL: f32 = 600.;
/// The join screen's column: one field and one button.
const JOIN_COL: f32 = 520.;
/// The Join button's width: the widest of "Join", "Joining" and "Paired" with its mark.
const JOIN_BTN: f32 = 118.;
/// "Paired with studio1. Opening the board." stays this long when the home computer does not answer after all;
/// then the usual "Can't reach Switchboard" takes over, with what the node says is wrong.
const JOINED_SHOWS: Duration = Duration::from_secs(8);

/// Whether the empty board asks for a code. Only with a node on this computer that can join a home and has not:
/// at once on the first run, and when asked for ("Join with a code") once an address is saved. A join that is
/// going, or just worked, keeps its screen. With no node, or one from before keys, never.
pub fn show_join(first: bool, keys: &Keys, asked: bool, step: &JoinStep) -> bool {
    if matches!(step, JoinStep::Joining | JoinStep::Joined(_)) {
        return true;
    }
    keys.offer_join() && (first || asked)
}

/// Whether the code field takes the keyboard on this frame: when the join screen has just come up (`was`: it
/// showed on the last frame), so the code can be typed without a click first. Not while a join runs or has worked.
pub fn focus_join(showing: bool, was: bool, step: &JoinStep) -> bool {
    showing && !was && !matches!(step, JoinStep::Joining | JoinStep::Joined(_))
}

impl BoardView {
    /// Whether the empty board shows the join screen now (lanes.rs `board_area` chooses it the same way).
    pub fn join_showing(&self, app: &crate::state::AppState) -> bool {
        if !app.snap.machines.is_empty() {
            return false;
        }
        let first = self.find_shot || app.core.needs_server();
        let wanted = self.join_shot || show_join(first, &app.keys, self.join_view, self.join.step());
        wanted && (first || !app.live || app.error.is_some())
    }

    /// The empty board. `first`: nothing is saved, so it searches; otherwise the saved address did not answer and
    /// `error` says so.
    pub fn server_screen(&self, c: &Ctx, first: bool, error: &str) -> AnyElement {
        if self.join_wanted(c, first) {
            return self.join_screen(c, first);
        }
        if first || self.find_view {
            return self.find_screen(c, first);
        }
        let t = c.t;
        // What this computer's node says about its way home, when something is wrong with it (macOS keeping it off
        // the local network, most of all), and the way to join with a code when it never has.
        let home = c.app.keys.node.as_ref().and_then(keys::home_line).filter(|l| l.tone == Tone::Bad);
        let can_join = c.app.keys.offer_join();
        let using = Some(c.app.core.servers().join(", ")).filter(|s| !s.is_empty());
        // An address typed here that has never once answered (a typo, most likely) can be taken out again.
        // Not the one typed a moment ago whose first try is still on its way.
        let never = c.app.settings.saved.never_answered(c.app.trying.as_deref()).cloned();
        let again = ui::line_btn(&t, "server-search", 30.)
            .px(px(12.))
            .child(ui::icon("search", 13., t.dim))
            .child("Search again")
            .on_click(c.cx.listener(|this, _: &ClickEvent, _, cx| {
                this.find_view = true;
                this.start_search(cx);
            }));
        let action = div()
            .w_full()
            .flex()
            .flex_col()
            .gap(px(8.))
            .when_some(home, |d, l| d.child(trouble(&t, &l.text, l.fix.as_deref())))
            .when_some(using, |d, u| {
                d.child(
                    ui::wrap(format!("Trying {u}. To use a different address, type it here."))
                        .text_size(px(12.5))
                        .text_color(t.faint),
                )
            })
            .child(self.address_row(c))
            .when_some(self.server_err.clone(), |d, e| d.child(ui::wrap(e).text_size(px(12.5)).text_color(t.bad)))
            .when_some(never, |d, u| {
                let shown = u.strip_prefix("http://").unwrap_or(&u).to_string();
                d.child(
                    ui::link(&t, "server-forget", format!("Remove {shown}, it has never answered"))
                        .text_size(px(12.5))
                        .on_click(c.cx.listener(move |this, _: &ClickEvent, _, cx| {
                            let u = u.clone();
                            this.app.update(cx, |s, cx| {
                                s.remove_server(&u, cx);
                                s.refresh(cx);
                            });
                        })),
                )
            })
            .child(
                div()
                    .mt(px(4.))
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(px(14.))
                    .child(again)
                    .when(can_join, |d| {
                        d.child(ui::link(&t, "server-join", "Join with a code").text_size(px(12.5)).on_click(
                            c.cx.listener(|this, _: &ClickEvent, window, cx| {
                                this.join_view = true;
                                this.join_field.update(cx, |f, cx| f.focus(window, cx));
                                cx.notify();
                            }),
                        ))
                    })
                    .child(
                        ui::link(&t, "server-guide", "How to set up the computer that runs Switchboard")
                            .text_size(px(12.5))
                            .on_click(|_: &ClickEvent, _, cx| crate::state::open_link(cx, &guide())),
                    ),
            );
        column(
            &t,
            div().w_full().max_w(px(COL + 20.)).child(ui::setup_card(
                &t,
                "remote",
                "Can't reach Switchboard",
                // The usual reason starts with the same words as the title: say them once.
                error.strip_prefix("Can't reach Switchboard. ").unwrap_or(error).to_string(),
                Some(action.into_any_element()),
            )),
        )
    }

    /// Whether the empty board is the join screen right now.
    pub fn join_wanted(&self, c: &Ctx, first: bool) -> bool {
        self.join_shot || show_join(first, &c.app.keys, self.join_view, self.join.step())
    }

    /// "Join your home computer": the code from Add a device, or its link, then the board. The address flow is
    /// under Advanced.
    pub fn join_screen(&self, c: &Ctx, first: bool) -> AnyElement {
        let t = c.t;
        let cx = c.cx;
        let step = self.join.step().clone();
        let busy = self.join.busy();
        let done = matches!(step, JoinStep::Joined(_));
        let wrong = matches!(&step, JoinStep::Invalid(_)) || matches!(&step, JoinStep::Failed(p) if p.retype());
        let field = div()
            .flex_1()
            .min_w(px(220.))
            .h(px(44.))
            .px(px(14.))
            .flex()
            .items_center()
            .rounded(px(ui::R_CTRL))
            .border_1()
            .border_color(if wrong { t.bad } else { t.line2 })
            .bg(t.bg)
            .text_size(px(16.))
            .text_color(t.ink)
            .overflow_hidden()
            .when(busy || done, |d| d.opacity(0.7))
            .child(div().flex_1().min_w_0().child(gpui_base::Input::new(&self.join_field)));
        // One width for all three labels, so the field beside it does not move when the label changes.
        let join = ui::primary_btn(&t, "join-go", 44.)
            .px(px(20.))
            .min_w(px(JOIN_BTN))
            .justify_center()
            .text_size(px(14.))
            .when(busy || done, |d| d.opacity(0.75))
            .when(busy, |d| d.child(ui::spinner("join-spin", 14., t.on_accent)))
            .when(done, |d| d.child(ui::icon("check", 14., t.on_accent)))
            .child(if busy {
                "Joining"
            } else if done {
                "Paired"
            } else if self.join.replaces() {
                "Replace"
            } else {
                "Join"
            })
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.join_now(cx)));
        let said = self.join.line().map(|(text, bad)| {
            if bad {
                trouble(&t, &text, None)
            } else {
                div()
                    .flex()
                    .items_start()
                    .gap(px(8.))
                    .text_size(px(12.5))
                    .text_color(t.dim)
                    .when(done, |d| d.child(div().pt(px(2.)).child(ui::icon("check", 13., t.warm))))
                    .child(ui::wrap(text).line_height(relative(1.45)))
                    .into_any_element()
            }
        });
        let adv = self.adv_open;
        let links = div()
            .w_full()
            .flex()
            .flex_wrap()
            .items_center()
            .justify_center()
            .gap_x(px(18.))
            .gap_y(px(8.))
            .when(first, |d| {
                d.child(
                    ui::link(&t, "join-run-here", "Make this the home computer")
                        .text_size(px(12.5))
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.run_here(cx))),
                )
            })
            .when(!first && !done, |d| {
                d.child(ui::link(&t, "join-back", "Back to the saved address").text_size(px(12.5)).on_click(
                    cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.join_view = false;
                        this.join.reset();
                        cx.notify();
                    }),
                ))
            })
            .when(first, |d| {
                d.child(
                    ui::ghost(&t, "join-adv", Some("Advanced"), None)
                        .h(px(26.))
                        .px(px(8.))
                        .gap(px(5.))
                        .text_size(px(12.5))
                        .child(ui::icon(if adv { "up" } else { "down" }, 11., t.dim))
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.adv_open = !this.adv_open;
                            cx.notify();
                        })),
                )
            });
        let body = div()
            .w_full()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(18.))
            .child(sonar(&t, busy))
            .child(heading(
                &t,
                "Join your home computer",
                "On your home computer, open Settings, then Devices, and choose Add a device. Type the code it shows, or paste the link.",
            ))
            .child(
                div()
                    .w_full()
                    .flex()
                    .flex_col()
                    .gap(px(10.))
                    .child(div().w_full().flex().flex_wrap().items_center().gap(px(8.)).child(field).child(join))
                    .children(said)
                    .when(self.join.needs_address(), |d| d.child(self.join_address(c, busy || done))),
            )
            .child(links)
            .when(first && adv, |d| d.child(self.by_address(c)));
        column(&t, div().w_full().max_w(px(JOIN_COL)).child(body))
    }

    /// Advanced, on the join screen: the address flow as it was, and the servers the search heard.
    fn by_address(&self, c: &Ctx) -> AnyElement {
        let t = c.t;
        let found = self.find.found().to_vec();
        let searching = self.find.searching();
        div()
            .w_full()
            .p(px(14.))
            .rounded(px(ui::R_CARD))
            .border_1()
            .border_color(t.line)
            .bg(t.panel)
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(3.))
                    .child(
                        div()
                            .text_size(px(13.5))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(t.ink)
                            .child("Connect by address"),
                    )
                    .child(
                        ui::wrap(
                            "For a home computer that doesn't use codes yet. Its address works on the same network, or over Tailscale.",
                        )
                        .text_size(px(12.5))
                        .line_height(relative(1.45))
                        .text_color(t.dim),
                    ),
            )
            .child(self.address_row(c))
            .when_some(self.server_err.clone(), |d, e| d.child(ui::wrap(e).text_size(px(12.5)).text_color(t.bad)))
            .when(!found.is_empty(), |d| d.child(self.server_list(c, &found, None)))
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(px(14.))
                    .child(if searching {
                        ui::loading(&t, "join-searching", "Searching this network").into_any_element()
                    } else {
                        ui::line_btn(&t, "join-search", 28.)
                            .px(px(12.))
                            .child(ui::icon("search", 13., t.dim))
                            .child("Search this network")
                            .on_click(c.cx.listener(|this, _: &ClickEvent, _, cx| this.start_search(cx)))
                            .into_any_element()
                    })
                    .child(
                        ui::link(&t, "join-guide", "Setup guide")
                            .text_size(px(12.5))
                            .on_click(|_: &ClickEvent, _, cx| crate::state::open_link(cx, &guide())),
                    ),
            )
            .into_any_element()
    }

    /// The home computer's address, under the code: asked for only when a link names no relay.
    fn join_address(&self, c: &Ctx, quiet: bool) -> AnyElement {
        let t = c.t;
        let bad = self.join.address_problem().map(str::to_string);
        div()
            .w_full()
            .flex()
            .flex_col()
            .gap(px(6.))
            .child(div().text_size(px(12.5)).text_color(t.ink2).child("Home computer's address"))
            .child(
                div()
                    .w_full()
                    .h(px(36.))
                    .px(px(12.))
                    .flex()
                    .items_center()
                    .rounded(px(ui::R_CTRL))
                    .border_1()
                    .border_color(if bad.is_some() { t.bad } else { t.line2 })
                    .bg(t.bg)
                    .text_size(px(14.))
                    .text_color(t.ink)
                    .overflow_hidden()
                    .when(quiet, |d| d.opacity(0.7))
                    .child(div().flex_1().min_w_0().child(gpui_base::Input::new(&self.join_addr))),
            )
            .children(bad.map(|e| trouble(&t, &e, None)))
            .into_any_element()
    }

    /// Join: sends what the field holds to this computer's node, which pairs with the home computer. The node is
    /// told this computer's name, for the home computer's list, and the home computer's address when the screen
    /// asked for one.
    pub fn join_now(&mut self, cx: &mut Context<Self>) {
        let typed = self.join_field.read(cx).value().to_string();
        let (started, addrs) = if self.join.needs_address() {
            let addr = self.join_addr.read(cx).value().to_string();
            match self.join.start_with(&typed, &addr) {
                Some((run, input, a)) => (Some((run, input)), vec![a]),
                None => (None, vec![]),
            }
        } else {
            (self.join.start(&typed), vec![])
        };
        cx.notify();
        let Some((run, input)) = started else { return };
        if self.join_shot {
            return;
        }
        // (the last try said this computer has another home, and the button said Replace)
        let replace = self.join.replaces();
        let node = self.app.read(cx).node.clone();
        cx.spawn(async move |this, cx| {
            let res = cx
                .background_executor()
                .spawn(async move {
                    let name = keys::this_computer();
                    // Paired: the node says which port on this computer it carries home on.
                    keys::join_home(
                        &*node,
                        &input,
                        name.as_deref(),
                        &addrs,
                        replace,
                        keys::HOME_HERE_TRIES,
                        keys::HOME_HERE_PAUSE,
                    )
                })
                .await;
            let _ = this.update(cx, |v, cx| v.join_took(run, res, cx));
        })
        .detach();
    }

    fn join_took(&mut self, run: u64, res: Result<(keys::Joined, String), keys::NodeError>, cx: &mut Context<Self>) {
        let (res, here) = match res {
            Ok((joined, here)) => (Ok(joined), here),
            Err(e) => (Err(e), String::new()),
        };
        if !self.join.finish(run, res) {
            return;
        }
        cx.notify();
        if !matches!(self.join.step(), JoinStep::Joined(_)) {
            return;
        }
        // Paired: the node carries the home computer's server to a port on this computer from now on.
        let used = self.app.update(cx, |s, cx| {
            s.load_keys(cx);
            s.use_server(&here, cx)
        });
        match used {
            Ok(()) => {
                self.server_err = None;
                self.find_view = false;
                self.joining = None;
            }
            Err(e) => self.server_err = Some(e),
        }
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(JOINED_SHOWS).await;
            let _ = this.update(cx, |v, cx| {
                if matches!(v.join.step(), JoinStep::Joined(_)) {
                    v.join.reset();
                    v.join_view = false;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// "Run the server on this computer": the guided setup, or the guide when this copy has no server in it.
    fn run_here(&mut self, cx: &mut Context<Self>) {
        // A copy of the app without the server in it (Windows, a bare Linux binary) can't set one up: the
        // guide says how, instead of three steps that end in "this copy doesn't include the server".
        if self.find_shot || sb_core::hostsetup::Kit::find().is_some() {
            self.open_setup(cx)
        } else {
            crate::state::open_link(cx, &guide())
        }
    }

    /// The address field and Connect.
    fn address_row(&self, c: &Ctx) -> gpui::Div {
        let t = c.t;
        let field = div()
            .flex_1()
            .min_w(px(200.))
            .h(px(32.))
            .px(px(10.))
            .flex()
            .items_center()
            .rounded(px(ui::R_CTRL))
            .border_1()
            .border_color(t.line2)
            .bg(t.bg)
            .text_size(px(13.))
            .text_color(t.ink)
            .overflow_hidden()
            .child(div().flex_1().min_w_0().child(gpui_base::Input::new(&self.server_field)));
        let connect = ui::primary_btn(&t, "server-connect", 32.)
            .child("Connect")
            .on_click(c.cx.listener(|this, _: &ClickEvent, window, cx| this.connect_server(window, cx)));
        div().w_full().flex().items_center().gap(px(8.)).child(field).child(connect)
    }

    /// Searching, found, or nothing found.
    fn find_screen(&self, c: &Ctx, first: bool) -> AnyElement {
        let t = c.t;
        let cx = c.cx;
        let show = match self.find.show() {
            Show::Idle => Show::Searching(Vec::new()),
            s => s,
        };
        let back = (!first).then(|| {
            ui::link(&t, "find-back", "Back to the saved address").text_size(px(12.5)).on_click(cx.listener(
                |this, _: &ClickEvent, _, cx| {
                    this.find_view = false;
                    cx.notify();
                },
            ))
        });
        let body = match show {
            Show::Searching(found) => {
                let n = found.len();
                div()
                    .w_full()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(18.))
                    .child(sonar(&t, true))
                    .child(heading(
                        &t,
                        "Searching for local servers",
                        "Looking for the computer on this network that runs Switchboard.",
                    ))
                    .when(n > 0, |d| d.child(self.server_list(c, &found, None)))
                    .into_any_element()
            }
            Show::Connecting(f) => div()
                .w_full()
                .flex()
                .flex_col()
                .items_center()
                .gap(px(18.))
                .child(sonar(&t, false))
                .child(heading(
                    &t,
                    &format!("Connecting to {}", f.name),
                    "Found one Switchboard server on this network.",
                ))
                .child(self.server_list(c, std::slice::from_ref(&f), Some(&f.url)))
                .into_any_element(),
            Show::Pick(found) => div()
                .w_full()
                .flex()
                .flex_col()
                .items_center()
                .gap(px(18.))
                .child(sonar(&t, false))
                .child(heading(
                    &t,
                    &format!("Found {} Switchboard servers", found.len()),
                    "Pick one. It's usually the computer that stays on at home.",
                ))
                .child(self.server_list(c, &found, None))
                .child(self.find_footer(c, false, back))
                .into_any_element(),
            Show::None | Show::Error(_) => {
                let (title, sub) = match self.find.show() {
                    Show::Error(e) => ("Couldn't search this network".to_string(), e),
                    _ => (
                        "No Switchboard server on this network".to_string(),
                        "Switchboard runs on one computer that stays on, and the apps on your computers and phones \
                         connect to it."
                            .to_string(),
                    ),
                };
                div()
                    .w_full()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(18.))
                    .child(sonar(&t, false))
                    .child(heading(&t, &title, &sub))
                    .child(self.choices(c))
                    .when(self.type_open, |d| {
                        d.child(
                            div()
                                .w_full()
                                .flex()
                                .flex_col()
                                .gap(px(6.))
                                .child(self.address_row(c))
                                .when_some(self.server_err.clone(), |d, e| {
                                    d.child(ui::wrap(e).text_size(px(12.5)).text_color(t.bad))
                                }),
                        )
                    })
                    .child(self.find_footer(c, true, back))
                    .into_any_element()
            }
            Show::Idle => div().into_any_element(),
        };
        column(&t, div().w_full().max_w(px(COL)).child(body))
    }

    /// The servers found, each with Connect. `joining`: the one being connected to, with a spinner instead.
    fn server_list(&self, c: &Ctx, found: &[Found], joining: Option<&str>) -> AnyElement {
        let t = c.t;
        let rows = found.iter().enumerate().map(|(i, f)| {
            let url = f.url.clone();
            let busy = joining == Some(f.url.as_str());
            let right: AnyElement = if busy {
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .text_size(px(12.5))
                    .text_color(t.dim)
                    .child(ui::spinner(gpui::ElementId::from(format!("find-spin-{i}")), 14., t.dim))
                    .child("Connecting")
                    .into_any_element()
            } else {
                ui::line_btn(&t, gpui::ElementId::from(format!("find-use-{i}")), 28.)
                    .px(px(12.))
                    .child("Connect")
                    .on_click(c.cx.listener(move |this, _: &ClickEvent, _, cx| this.use_found(&url, cx)))
                    .into_any_element()
            };
            div()
                .w_full()
                .px(px(14.))
                .py(px(10.))
                .flex()
                .items_center()
                .gap(px(12.))
                .when(i > 0, |d| d.border_t_1().border_color(t.line))
                .child(
                    div()
                        .flex_none()
                        .size(px(30.))
                        .rounded(px(ui::R_CTRL))
                        .bg(t.accent.alpha(if t.dark { 0.16 } else { 0.1 }))
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(ui::icon(if f.here { "desktop" } else { "remote" }, 15., t.accent)),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(2.))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(8.))
                                .child(
                                    ui::line(f.name.clone())
                                        .text_size(px(13.5))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(t.ink),
                                )
                                .when(f.here && f.name != sb_core::firstrun::HERE, |d| {
                                    d.child(ui::tag(&t, "This computer"))
                                }),
                        )
                        .child(ui::line(f.shown()).font_family(NUM).text_size(px(11.5)).text_color(t.faint)),
                )
                .child(right)
        });
        div()
            .w_full()
            .rounded(px(ui::R_CARD))
            .border_1()
            .border_color(t.line)
            .bg(t.panel)
            .overflow_hidden()
            .children(rows)
            .into_any_element()
    }

    /// Run the server here, or type an address.
    fn choices(&self, c: &Ctx) -> AnyElement {
        let t = c.t;
        let os = crate::serversetup::shown_os();
        let tile =
            |id: &'static str, ic: &str, title: String, body: &str, primary: bool| {
                let (hover, press) = (t.hover, t.press);
                ui::focusable(div().id(id), &t)
                    .flex_1()
                    .min_w(px(220.))
                    .p(px(14.))
                    .rounded(px(ui::R_CARD))
                    .border_1()
                    .border_color(if primary { t.accent.alpha(0.55) } else { t.line2 })
                    .bg(if primary { t.accent.alpha(if t.dark { 0.1 } else { 0.05 }) } else { t.panel })
                    .cursor_pointer()
                    .hover(move |s| s.bg(hover))
                    .active(move |s| s.bg(press))
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .child(
                        div().flex().items_center().gap(px(8.)).child(ui::icon(ic, 15., t.accent)).child(
                            ui::wrap(title).text_size(px(13.5)).font_weight(FontWeight::SEMIBOLD).text_color(t.ink),
                        ),
                    )
                    .child(ui::wrap(body.to_string()).text_size(px(12.)).line_height(relative(1.45)).text_color(t.dim))
            };
        let here = tile(
            "find-run-here",
            if os == sb_core::hostsetup::Os::Mac { "mac" } else { "desktop" },
            "Run the server on this computer".into(),
            &format!("Make {} the server, step by step. Best on a computer that stays on.", os.this_one()),
            true,
        )
        .on_click(c.cx.listener(|this, _: &ClickEvent, _, cx| this.run_here(cx)));
        let typed = tile(
            "find-type",
            "globe",
            "Type an address".into(),
            "The server runs on another computer, at home or over Tailscale.",
            false,
        )
        .on_click(c.cx.listener(|this, _: &ClickEvent, window, cx| {
            this.type_open = !this.type_open;
            if this.type_open {
                this.server_field.update(cx, |f, cx| f.focus(window, cx));
            }
            cx.notify();
        }));
        div().w_full().flex().flex_wrap().gap(px(10.)).child(here).child(typed).into_any_element()
    }

    /// Search again and the guide, under the choices.
    fn find_footer(&self, c: &Ctx, guide_link: bool, back: Option<gpui::Stateful<gpui::Div>>) -> AnyElement {
        let t = c.t;
        div()
            .w_full()
            .flex()
            .flex_wrap()
            .items_center()
            .justify_center()
            .gap(px(16.))
            .child(
                ui::line_btn(&t, "find-again", 30.)
                    .px(px(12.))
                    .child(ui::icon("search", 13., t.dim))
                    .child("Search again")
                    .on_click(c.cx.listener(|this, _: &ClickEvent, _, cx| this.start_search(cx))),
            )
            .when(guide_link, |d| {
                d.child(
                    ui::link(&t, "find-guide", "Setup guide")
                        .text_size(px(12.5))
                        .on_click(|_: &ClickEvent, _, cx| crate::state::open_link(cx, &guide())),
                )
            })
            .children(back)
            .into_any_element()
    }

    /// Starts a search (again): Bonjour on the network, and this computer's own port, at once.
    pub fn start_search(&mut self, cx: &mut Context<Self>) {
        let run = self.find.start();
        self.type_open = false;
        cx.notify();
        if self.find_shot {
            return;
        }
        let ua = self.app.read(cx).core.user_agent();
        let (tx, rx) = smol::channel::unbounded::<Heard>();
        let spawned = std::thread::Builder::new().name("sb-find".into()).spawn(move || {
            let here_tx = tx.clone();
            let here = std::thread::spawn(move || {
                let url = format!("http://127.0.0.1:{}", sb_core::net::PORT);
                if let Some(s) = sb_core::net::summary_at(&url, &ua) {
                    let addrs = s.addresses.map(|a| a.list.into_iter().map(|x| x.url).collect()).unwrap_or_default();
                    let _ = here_tx.send_blocking(Heard::Here(url, addrs));
                }
            });
            let res = sb_core::discover::browse_each(SEARCH_FOR, |a| {
                let _ = tx.send_blocking(Heard::Found(a));
            });
            let _ = here.join();
            let _ = tx.send_blocking(Heard::Done(res.err().map(|e| sb_core::firstrun::search_error(&e))));
        });
        if let Err(e) = spawned {
            self.find.finish(run, Some(format!("Switchboard couldn't start the search ({e}).")));
            return;
        }
        cx.spawn(async move |this, cx| {
            while let Ok(h) = rx.recv().await {
                let done = matches!(h, Heard::Done(_));
                if this.update(cx, |v, cx| v.heard(run, h, cx)).is_err() || done {
                    break;
                }
            }
        })
        .detach();
    }

    fn heard(&mut self, run: u64, h: Heard, cx: &mut Context<Self>) {
        match h {
            Heard::Found(a) => {
                self.find.add(run, &a.name, &a.url);
            }
            Heard::Here(url, addrs) => self.find.local(run, &url, &addrs),
            Heard::Done(err) => {
                self.find.finish(run, err);
                // Exactly one answered: connect to it, unless the setup is open (it reads the result itself), or
                // this computer can join with a code: then the code is asked for, and what answered is listed
                // under Advanced for whoever wants the address after all.
                if self.setup.is_none()
                    && !self.app.read(cx).keys.offer_join()
                    && let Some(f) = self.find.take_single()
                {
                    self.joining = Some(f.clone());
                    // A moment on the found server, so the screen does not just flash past.
                    cx.spawn(async move |this, cx| {
                        cx.background_executor().timer(Duration::from_millis(700)).await;
                        let _ = this.update(cx, |v, cx| v.use_found(&f.url, cx));
                    })
                    .detach();
                }
            }
        }
        cx.notify();
    }

    /// Uses a server the search found: saved like a typed address, and tried at once.
    pub fn use_found(&mut self, url: &str, cx: &mut Context<Self>) {
        if self.find_shot {
            return;
        }
        if let Some(f) = self.find.found().iter().find(|f| f.url == url).cloned() {
            self.joining = Some(f);
        }
        match self.app.update(cx, |s, cx| s.use_server(url, cx)) {
            Ok(()) => {
                self.server_err = None;
                self.find_view = false;
            }
            // What the search heard is not an address the app can use: say so where the address is typed,
            // instead of "Connecting" for ever.
            Err(e) => {
                self.joining = None;
                self.server_err = Some(format!("{url}: {e}"));
                self.type_open = true;
            }
        }
        cx.notify();
    }

    /// The first run starts its search by itself (and again after every saved address was removed).
    pub fn search_if_first(&mut self, cx: &mut Context<Self>) {
        if !self.find.started() && self.app.read(cx).core.needs_server() {
            self.start_search(cx);
        }
    }

    /// "Connecting to studio1" while the server the search found is tried.
    pub fn joining_screen(&self, c: &Ctx, f: &Found) -> AnyElement {
        let t = c.t;
        column(
            &t,
            div()
                .w_full()
                .max_w(px(COL))
                .flex()
                .flex_col()
                .items_center()
                .gap(px(18.))
                .child(sonar(&t, false))
                .child(heading(&t, &format!("Connecting to {}", f.name), "The board shows up as soon as it answers."))
                .child(self.server_list(c, std::slice::from_ref(f), Some(&f.url))),
        )
    }

    /// Connect: saves what the field holds as the server's address and tries it.
    pub fn connect_server(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let typed = self.server_field.read(cx).value().to_string();
        match self.app.update(cx, |s, cx| s.use_server(&typed, cx)) {
            // The field keeps what was typed: when the address doesn't answer, one wrong digit is fixed in place,
            // not typed again from the start.
            Ok(()) => {
                self.server_err = None;
                self.find_view = false;
                self.type_open = false;
                // The search's "Connecting to studio1" is over: this is a different address.
                self.joining = None;
            }
            Err(e) => self.server_err = Some(e),
        }
        cx.notify();
    }

    /// `shots` states for the empty board: SB_SHOT=find=<searching|found1|one|several|none|error|typing>, and the
    /// join screen's: find=<join|join-typed|join-busy|join-wrong|join-expired|join-tries|join-unreachable|
    /// join-localnet|join-badcode|join-done|join-adv>.
    pub fn shot_find(&mut self, what: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.find_shot = true;
        let run = self.find.start();
        let studio = ("studio1", "http://studio1.local:8130");
        if let Some(state) = what.strip_prefix("join") {
            self.join_shot = true;
            self.find.add(run, studio.0, studio.1);
            self.find.finish(run, None);
            let state = state.trim_start_matches('-');
            let typed = match state {
                "" | "adv" => "",
                "badcode" => "ABCD-EFG0",
                "needaddr" | "badaddr" => "switchboard://pair/1?h=6f1c22d0&s=9a2e41b7",
                _ => "K7PM-2XQ9",
            };
            self.join_field.update(cx, |f, cx| f.set_value(typed, window, cx));
            let said =
                |kind: &str| keys::NodeError::Said { status: 403, kind: Some(kind.into()), message: String::new() };
            let res = match state {
                "wrong" => Some(Err(said("wrong_code"))),
                "expired" => Some(Err(said("expired"))),
                "tries" => Some(Err(said("tries"))),
                "unreachable" => Some(Err(said("unreachable"))),
                "localnet" => Some(Err(said("local_network"))),
                "needaddr" | "badaddr" => Some(Err(said("link_no_relay"))),
                "done" => Some(Ok(keys::Joined { home: Some("studio1".into()), machine: None })),
                _ => None,
            };
            if matches!(state, "busy" | "badcode") || res.is_some() {
                let started = self.join.start(typed);
                if let (Some((n, _)), Some(res)) = (started, res) {
                    self.join.finish(n, res);
                }
            }
            if state == "badaddr" {
                self.join_addr.update(cx, |f, cx| f.set_value("192.0.2.20", window, cx));
                let _ = self.join.start_with(typed, "192.0.2.20");
            }
            self.adv_open = state == "adv";
            cx.notify();
            return;
        }
        match what {
            "found1" => {
                self.find.add(run, studio.0, studio.1);
            }
            "one" => {
                self.find.add(run, studio.0, studio.1);
                self.find.finish(run, None);
            }
            "several" => {
                self.find.add(run, studio.0, studio.1);
                self.find.add(run, "tower", "http://192.0.2.20:8130");
                self.find.local(run, "http://127.0.0.1:8130", &[]);
                self.find.finish(run, None);
            }
            "none" => self.find.finish(run, None),
            "typing" => {
                self.find.finish(run, None);
                self.type_open = true;
            }
            "error" => self.find.finish(
                run,
                Some(sb_core::firstrun::search_error(&std::io::Error::from(std::io::ErrorKind::PermissionDenied))),
            ),
            _ => {}
        }
        cx.notify();
    }
}

/// Something is wrong, in the page's red, and under it what to do about it when there is something to do.
fn trouble(t: &Theme, text: &str, fix: Option<&str>) -> AnyElement {
    div()
        .w_full()
        .flex()
        .items_start()
        .gap(px(8.))
        .child(div().flex_none().pt(px(2.)).child(ui::icon("warn", 13., t.bad)))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(3.))
                .child(ui::wrap(text.to_string()).text_size(px(12.5)).line_height(relative(1.45)).text_color(t.bad))
                .children(fix.map(|f| {
                    ui::wrap(f.to_string()).text_size(px(12.5)).line_height(relative(1.45)).text_color(t.ink2)
                })),
        )
        .into_any_element()
}

/// Centers `inner` on the empty board, with the mark above it.
fn column(t: &Theme, inner: impl IntoElement) -> AnyElement {
    let _ = t;
    div()
        .id("empty-board")
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .flex()
        .flex_col()
        .items_center()
        .px(px(16.))
        .py(px(32.))
        .child(div().flex_1().min_h(px(12.)))
        .child(inner)
        .child(div().flex_1().min_h(px(12.)))
        .into_any_element()
}

/// The title and the line under it, centered.
fn heading(t: &Theme, title: &str, sub: &str) -> AnyElement {
    div()
        .w_full()
        .flex()
        .flex_col()
        .items_center()
        .gap(px(6.))
        .child(
            ui::wrap(title.to_string())
                .text_center()
                .text_size(px(18.))
                .font_weight(FontWeight::BOLD)
                .text_color(t.ink),
        )
        .child(
            ui::wrap(sub.to_string())
                .max_w(px(440.))
                .text_center()
                .text_size(px(13.))
                .line_height(relative(1.45))
                .text_color(t.dim),
        )
        .into_any_element()
}

/// The mark with rings going out from it while the search listens; still rings once it is over.
pub fn sonar(t: &Theme, moving: bool) -> AnyElement {
    use gpui::AnimationExt;
    const BOX: f32 = 112.;
    const MARK: f32 = 30.;
    let accent = t.accent;
    let ring = |size: f32, alpha: f32| {
        div()
            .absolute()
            .top(px((BOX - size) / 2.))
            .left(px((BOX - size) / 2.))
            .size(px(size))
            .rounded_full()
            .border_1()
            .border_color(accent.alpha(alpha))
    };
    let mut d = div().relative().flex_none().size(px(BOX));
    if moving {
        for (i, phase) in [0.0f32, 0.5].into_iter().enumerate() {
            d = d.child(ring(BOX, 0.).with_animation(
                gpui::ElementId::from(format!("sonar-{i}")),
                gpui::Animation::new(Duration::from_millis(2400)).repeat().with_max_fps(crate::ui::SPIN_FPS),
                move |r, k| {
                    let k = (k + phase) % 1.0;
                    let size = MARK + 14. + (BOX - MARK - 14.) * k;
                    r.top(px((BOX - size) / 2.))
                        .left(px((BOX - size) / 2.))
                        .size(px(size))
                        .border_color(accent.alpha(0.55 * (1. - k)))
                },
            ));
        }
    } else {
        d = d.child(ring(BOX - 28., 0.18)).child(ring(BOX, 0.08));
    }
    let halo = div()
        .absolute()
        .top(px((BOX - MARK - 22.) / 2.))
        .left(px((BOX - MARK - 22.) / 2.))
        .size(px(MARK + 22.))
        .rounded_full()
        .bg(accent.alpha(if t.dark { 0.12 } else { 0.08 }));
    d.child(halo)
        .child(div().absolute().top(px((BOX - MARK) / 2.)).left(px((BOX - MARK) / 2.)).child(ui::mark(t, MARK)))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use sb_core::keys::{Home, JoinProblem, Mode, NodeStatus, Readiness, Role};

    fn node(keys: bool, role: Role, home: bool) -> Keys {
        let home = home.then(|| Home { id: "h1".into(), name: "studio1".into(), ..Default::default() });
        Keys { server: None, node: Some(NodeStatus { keys, role, home, ..Default::default() }) }
    }

    #[test]
    fn without_a_node_that_can_join_the_empty_board_is_what_it_was() {
        let idle = JoinStep::Idle;
        // No node on this computer, whatever the server says: the search, then the address, as always.
        for server in [None, Some(Mode::Off), Some(Mode::Both), Some(Mode::Only)] {
            let k = Keys { server: server.map(|keys| Readiness { keys, ..Default::default() }), node: None };
            for (first, asked) in [(true, false), (true, true), (false, false), (false, true)] {
                assert!(!show_join(first, &k, asked, &idle), "{server:?} first {first} asked {asked}");
            }
        }
        // A node from before keys, or with keys off.
        assert!(!show_join(true, &node(false, Role::Alone, false), true, &idle));
        // The home computer itself never joins one.
        assert!(!show_join(true, &node(true, Role::Home, false), true, &idle));
        // A computer that has joined already is not asked again.
        assert!(!show_join(true, &node(true, Role::Member, true), true, &idle));
    }

    #[test]
    fn a_node_that_can_join_is_asked_for_the_code() {
        let k = node(true, Role::Alone, false);
        let idle = JoinStep::Idle;
        assert!(show_join(true, &k, false, &idle), "the first run asks at once");
        assert!(!show_join(false, &k, false, &idle), "a saved address is tried as before");
        assert!(show_join(false, &k, true, &idle), "until Join with a code is chosen");
        // Mistakes stay on the screen they were made on.
        assert!(show_join(true, &k, false, &JoinStep::Failed(JoinProblem::WrongCode)));
        assert!(!show_join(false, &k, false, &JoinStep::Failed(JoinProblem::WrongCode)));
    }

    #[test]
    fn a_join_that_worked_keeps_its_screen_until_the_board_opens() {
        // The node says it is a member now, so it would no longer be offered: the screen still says "Paired".
        let joined = node(true, Role::Member, true);
        assert!(show_join(false, &joined, false, &JoinStep::Joined(Some("studio1".into()))));
        assert!(show_join(false, &joined, false, &JoinStep::Joining));
        assert!(!show_join(false, &joined, false, &JoinStep::Idle));
    }

    #[test]
    fn the_code_field_takes_the_keys_when_the_join_screen_arrives() {
        // The frame the screen first shows on, and never again while it stays: a click elsewhere keeps its focus.
        assert!(focus_join(true, false, &JoinStep::Idle));
        assert!(!focus_join(true, true, &JoinStep::Idle));
        assert!(!focus_join(false, false, &JoinStep::Idle));
        assert!(!focus_join(false, true, &JoinStep::Idle));
        // Back on the screen after a try that failed: the field again.
        assert!(focus_join(true, false, &JoinStep::Failed(JoinProblem::WrongCode)));
        // A join that is going or done has no field to type in.
        assert!(!focus_join(true, false, &JoinStep::Joining));
        assert!(!focus_join(true, false, &JoinStep::Joined(None)));
    }
}
