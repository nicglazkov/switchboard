//! The empty board when there is no server to show.
//!
//! First run (nothing saved): "Searching for local servers" while the Bonjour search runs (sb_core::discover), each
//! server listed as it answers. Exactly one: the app connects to it by itself. Several: the user picks. None: "No
//! Switchboard server on this network", with "Run the server on this computer" (the guided setup, serversetup.rs)
//! and "Type an address". What to show is decided in sb_core::firstrun, which has the tests.
//!
//! A saved address that does not answer: the address field as before, and Search again.

use std::time::Duration;

use gpui::{
    AnyElement, ClickEvent, Context, FontWeight, IntoElement, ParentElement, StatefulInteractiveElement, Styled,
    Window, div, prelude::*, px, relative,
};
use sb_core::firstrun::{Found, SEARCH_FOR, Show};

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

impl BoardView {
    /// The empty board. `first`: nothing is saved, so it searches; otherwise the saved address did not answer and
    /// `error` says so.
    pub fn server_screen(&self, c: &Ctx, first: bool, error: &str) -> AnyElement {
        if first || self.find_view {
            return self.find_screen(c, first);
        }
        let t = c.t;
        let using = Some(c.app.core.servers().join(", ")).filter(|s| !s.is_empty());
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
            .when_some(using, |d, u| {
                d.child(
                    ui::wrap(format!("Trying {u}. To use a different address, type it here."))
                        .text_size(px(12.5))
                        .text_color(t.faint),
                )
            })
            .child(self.address_row(c))
            .when_some(self.server_err.clone(), |d, e| d.child(ui::wrap(e).text_size(px(12.5)).text_color(t.bad)))
            .child(
                div().mt(px(4.)).flex().flex_wrap().items_center().gap(px(14.)).child(again).child(
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
                error.to_string(),
                Some(action.into_any_element()),
            )),
        )
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
                        "Switchboard runs on one computer that stays on, and the apps on your machines and phones \
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
        .on_click(c.cx.listener(|this, _: &ClickEvent, _, cx| this.open_setup(cx)));
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
                // Exactly one answered: connect to it, unless the setup is open (it reads the result itself).
                if self.setup.is_none()
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
        let _ = self.app.update(cx, |s, cx| s.use_server(url, cx));
        self.find_view = false;
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
    pub fn connect_server(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let typed = self.server_field.read(cx).value().to_string();
        match self.app.update(cx, |s, cx| s.use_server(&typed, cx)) {
            Ok(()) => {
                self.server_err = None;
                self.find_view = false;
                self.type_open = false;
                self.server_field.update(cx, |f, cx| f.set_value("", window, cx));
            }
            Err(e) => self.server_err = Some(e),
        }
        cx.notify();
    }

    /// `shots` states for the empty board: SB_SHOT=find=<searching|found1|one|several|none|error|typing>.
    pub fn shot_find(&mut self, what: &str, cx: &mut Context<Self>) {
        self.find_shot = true;
        let run = self.find.start();
        let studio = ("studio1", "http://studio1.local:8130");
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
                gpui::Animation::new(Duration::from_millis(2400)).repeat(),
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
