//! "Run the server on this computer": a guided sheet over the empty board. Intro (what being the server means),
//! the checks (a laptop, Python 3, the ports, another server on the network), a confirm, the bundled installer's
//! progress, then Done (connected to http://localhost:8130, how to keep it awake, how the other devices find it), or
//! the installer's last lines and Try again. The decisions and the words for the checks are in sb_core::hostsetup.
//!
//! The installer is the kit's own `install-server.sh --yes --no-setup`, run as the user with its output piped (so it
//! prints plain `== <step>` lines), never with sudo. The app changes no power settings itself: Done opens the right
//! pane of System Settings.

use std::io::BufRead;
use std::path::PathBuf;
use std::time::Duration;

use gpui::{
    AnyElement, ClickEvent, Context, FontWeight, Hsla, IntoElement, MouseButton, ParentElement,
    StatefulInteractiveElement, Styled, div, prelude::*, px, relative,
};
use sb_core::hostsetup::{
    self, Check, Facts, Install, Key, Kit, LOCAL_URL, Mark, Os, PORTS, Power, Python, StepState, Verdict,
};

use crate::theme::{NUM, Theme};
use crate::ui;
use crate::view::{BoardView, Ctx};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Intro,
    Checks,
    Confirm,
    Running,
    Done,
    Failed,
}

/// The sheet's state.
pub struct Setup {
    pub page: Page,
    os: Os,
    power: Option<Power>,
    python: Option<Python>,
    busy: Option<Vec<u16>>,
    kit: Option<Kit>,
    /// The kit was looked for (None before the first check).
    kit_looked: bool,
    laptop_yes: bool,
    install: Install,
    /// Counts check runs, so a slow one that was overtaken is dropped.
    pass: u64,
    /// `shots`: fixed facts, nothing runs.
    shot: bool,
    /// `shots`: whether the kit has a hub, since the frames' kit is not on disk.
    hub_shot: Option<bool>,
}

/// The kind of computer the words are for. Frames from the Linux shots host show the Mac's.
pub fn shown_os() -> Os {
    if cfg!(feature = "shots")
        && std::env::var("SB_SHOT").is_ok_and(|s| s.starts_with("find") || s.starts_with("setup"))
    {
        return Os::Mac;
    }
    Os::this()
}

impl Setup {
    fn new() -> Setup {
        Setup {
            page: Page::Intro,
            os: shown_os(),
            power: None,
            python: None,
            busy: None,
            kit: None,
            kit_looked: false,
            laptop_yes: false,
            install: Install::new(&hostsetup::plan(true)),
            pass: 0,
            shot: false,
            hub_shot: None,
        }
    }

    fn laptop(&self) -> bool {
        matches!(self.power, Some(Power::Laptop(_)))
    }

    fn hub(&self) -> bool {
        self.hub_shot.unwrap_or_else(|| self.kit.as_ref().is_some_and(|k| k.hub(self.os).is_some()))
    }
}

/// What the installer thread reports.
enum Said {
    Line(String),
    End(bool),
}

impl BoardView {
    pub fn open_setup(&mut self, cx: &mut Context<Self>) {
        self.setup = Some(Setup::new());
        cx.notify();
    }

    fn close_setup(&mut self, cx: &mut Context<Self>) {
        if self.setup.as_ref().is_some_and(|s| s.page == Page::Running) {
            return;
        }
        self.setup = None;
        cx.notify();
    }

    fn setup_page(&mut self, page: Page, cx: &mut Context<Self>) {
        if let Some(s) = &mut self.setup {
            s.page = page;
        }
        if page == Page::Checks {
            self.run_checks(cx);
        }
        cx.notify();
    }

    /// Looks at this computer again: battery, Python, ports and the kit (the network search was already done).
    fn run_checks(&mut self, cx: &mut Context<Self>) {
        let Some(s) = &mut self.setup else { return };
        if s.shot {
            return;
        }
        s.pass += 1;
        let pass = s.pass;
        s.power = None;
        s.python = None;
        s.busy = None;
        let os = s.os;
        if !self.find.started() {
            self.start_search(cx);
        }
        cx.spawn(async move |this, cx| {
            let looked = cx
                .background_executor()
                .spawn(async move {
                    (hostsetup::detect_power(), hostsetup::find_python(os), hostsetup::busy_ports(&PORTS), Kit::find())
                })
                .await;
            // Long enough to see that each row was looked at.
            cx.background_executor().timer(Duration::from_millis(450)).await;
            let _ = this.update(cx, |v, cx| {
                if let Some(s) = &mut v.setup
                    && s.pass == pass
                {
                    (s.power, s.python, s.busy, s.kit) = (Some(looked.0), Some(looked.1), Some(looked.2), looked.3);
                    s.kit_looked = true;
                    s.install = Install::new(&hostsetup::plan(s.hub()));
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn facts(&self, s: &Setup) -> Facts {
        Facts {
            power: s.power.clone(),
            python: s.python.clone(),
            busy: s.busy.clone(),
            searching: self.find.searching() || !self.find.started(),
            others: self.find.others(),
            here: self.find.here(),
            kit: s.kit.is_some() || !s.kit_looked,
        }
    }

    /// Runs the kit's installer and follows its lines.
    fn run_install(&mut self, cx: &mut Context<Self>) {
        let Some(s) = &mut self.setup else { return };
        s.page = Page::Running;
        s.install = Install::new(&hostsetup::plan(s.hub()));
        cx.notify();
        if s.shot {
            return;
        }
        let Some(kit) = s.kit.clone() else {
            s.install.line("install-server.sh: this copy of the app does not carry the server.");
            s.install.finish(false);
            s.page = Page::Failed;
            return;
        };
        let (prog, args) = kit.command(s.os);
        let py = match &s.python {
            Some(Python::Found { path, .. }) => Some(path.clone()),
            _ => None,
        };
        let path = hostsetup::install_path(s.os, py.as_deref(), &std::env::var("PATH").unwrap_or_default());
        let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from);
        let (tx, rx) = smol::channel::unbounded::<Said>();
        let spawned = std::thread::Builder::new().name("sb-install".into()).spawn(move || {
            let mut cmd = std::process::Command::new(&prog);
            cmd.args(&args)
                .env("PATH", path)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped());
            if let Some(h) = &home {
                cmd.current_dir(h);
            }
            let mut child = match cmd.spawn() {
                Ok(c) => c,
                Err(e) => {
                    let _ = tx.send_blocking(Said::Line(format!("Couldn't start the installer: {e}")));
                    let _ = tx.send_blocking(Said::End(false));
                    return;
                }
            };
            let err = child.stderr.take().map(|e| {
                let tx = tx.clone();
                std::thread::spawn(move || {
                    for l in std::io::BufReader::new(e).lines().map_while(Result::ok) {
                        let _ = tx.send_blocking(Said::Line(l));
                    }
                })
            });
            if let Some(out) = child.stdout.take() {
                for l in std::io::BufReader::new(out).lines().map_while(Result::ok) {
                    let _ = tx.send_blocking(Said::Line(l));
                }
            }
            if let Some(h) = err {
                let _ = h.join();
            }
            let ok = child.wait().is_ok_and(|s| s.success());
            let _ = tx.send_blocking(Said::End(ok));
        });
        if let Err(e) = spawned {
            s.install.line(&format!("Couldn't start the installer: {e}"));
            s.install.finish(false);
            s.page = Page::Failed;
            return;
        }
        cx.spawn(async move |this, cx| {
            while let Ok(said) = rx.recv().await {
                let end = matches!(said, Said::End(_));
                let gone = this.update(cx, |v, cx| v.installer_said(said, cx)).is_err();
                if gone || end {
                    break;
                }
            }
        })
        .detach();
    }

    fn installer_said(&mut self, said: Said, cx: &mut Context<Self>) {
        let Some(s) = &mut self.setup else { return };
        match said {
            Said::Line(l) => s.install.line(&l),
            Said::End(ok) => {
                s.install.finish(ok);
                s.page = if ok { Page::Done } else { Page::Failed };
                if ok {
                    let _ = self.app.update(cx, |a, cx| a.use_server(LOCAL_URL, cx));
                }
            }
        }
        cx.notify();
    }

    /// The sheet over the window.
    pub fn setup_box(&self, c: &Ctx, s: &Setup) -> impl IntoElement {
        let t = c.t;
        let cx = c.cx;
        let body = match s.page {
            Page::Intro => self.setup_intro(c, s),
            Page::Checks => self.setup_checks(c, s),
            Page::Confirm => self.setup_confirm(c, s),
            Page::Running | Page::Failed => self.setup_running(c, s),
            Page::Done => self.setup_done(c, s),
        };
        let closable = s.page != Page::Running;
        div()
            .absolute()
            .inset_0()
            .bg(t.scrim)
            .occlude()
            .flex()
            .items_center()
            .justify_center()
            .p(px(16.))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .id("setup-sheet")
                    .relative()
                    .w(px(560.))
                    .max_w_full()
                    .max_h_full()
                    .overflow_y_scroll()
                    .rounded(px(ui::R_SHEET))
                    .border_1()
                    .border_color(t.line2)
                    .bg(t.panel2)
                    .shadow(t.drop())
                    .occlude()
                    .child(
                        div()
                            .px(px(26.))
                            .pt(px(22.))
                            .pb(px(22.))
                            .flex()
                            .flex_col()
                            .gap(px(16.))
                            .child(progress(&t, s.page))
                            .child(body),
                    )
                    .when(closable, |d| {
                        d.child(
                            div().absolute().top(px(12.)).right(px(12.)).child(
                                ui::icon_btn(&t, "setup-close", "x", 26.)
                                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.close_setup(cx))),
                            ),
                        )
                    }),
            )
    }

    fn setup_intro(&self, c: &Ctx, s: &Setup) -> AnyElement {
        let t = c.t;
        let this = s.os.this_one();
        let point = |ic: &str, title: &str, body: String| {
            div()
                .flex()
                .items_start()
                .gap(px(12.))
                .child(
                    div()
                        .flex_none()
                        .mt(px(1.))
                        .size(px(28.))
                        .rounded(px(ui::R_CTRL))
                        .bg(t.bg)
                        .border_1()
                        .border_color(t.line)
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(ui::icon(ic, 14., t.ink2)),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(2.))
                        .child(
                            ui::wrap(title.to_string())
                                .text_size(px(13.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(t.ink),
                        )
                        .child(ui::wrap(body).text_size(px(12.5)).line_height(relative(1.45)).text_color(t.dim)),
                )
        };
        div()
            .flex()
            .flex_col()
            .gap(px(16.))
            .child(title_block(
                &t,
                if s.os == Os::Mac { "mac" } else { "desktop" },
                t.accent,
                format!("Set up {this} as your Switchboard server?"),
                "One computer runs Switchboard, and everything else connects to it.".into(),
            ))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(14.))
                    .child(point(
                        "remote",
                        "It serves all your machines and phones",
                        format!(
                            "{} keeps the list of your Claude Code chats and machines. The apps on your other \
                             computers and your iPhone connect to it.",
                            capital(this)
                        ),
                    ))
                    .child(point(
                        "clock",
                        "It needs to stay on and awake, 24/7",
                        "If it sleeps or shuts down, your phone and your other machines lose Switchboard until it's \
                         back."
                            .into(),
                    ))
                    .child(point(
                        "lock",
                        "It stays on your network",
                        "Your devices talk to it directly, at home or over Tailscale. It's not a cloud service, and \
                         there's no account."
                            .into(),
                    )),
            )
            .child(footer(
                &t,
                Some(
                    ui::line_btn(&t, "setup-notnow", 32.)
                        .px(px(14.))
                        .text_size(px(13.))
                        .child("Not now")
                        .on_click(c.cx.listener(|this, _: &ClickEvent, _, cx| this.close_setup(cx)))
                        .into_any_element(),
                ),
                ui::primary_btn(&t, "setup-check", 32.)
                    .px(px(16.))
                    .child(format!("Check {this}"))
                    .on_click(c.cx.listener(|this, _: &ClickEvent, _, cx| this.setup_page(Page::Checks, cx)))
                    .into_any_element(),
            ))
            .into_any_element()
    }

    fn setup_checks(&self, c: &Ctx, s: &Setup) -> AnyElement {
        let t = c.t;
        let cx = c.cx;
        let list = hostsetup::checks(s.os, &self.facts(s));
        let verdict = hostsetup::verdict(&list, s.laptop_yes);
        let rows = list.iter().enumerate().map(|(i, ch)| self.check_row(c, s, ch, i));
        let note = match verdict {
            Verdict::Checking => None,
            Verdict::Blocked => Some("Fix what's marked, then check again.".to_string()),
            Verdict::NeedsYes => Some("Decide whether to use this laptop anyway.".to_string()),
            Verdict::Go => None,
        };
        let go = verdict == Verdict::Go;
        let next = ui::primary_btn(&t, "setup-next", 32.)
            .px(px(16.))
            .child("Continue")
            .when(!go, |b| b.opacity(0.45).cursor_default())
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                if go {
                    this.setup_page(Page::Confirm, cx);
                }
            }));
        let again = (verdict == Verdict::Blocked).then(|| {
            ui::line_btn(&t, "setup-recheck", 32.)
                .px(px(14.))
                .text_size(px(13.))
                .child("Check again")
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.setup_page(Page::Checks, cx)))
        });
        div()
            .flex()
            .flex_col()
            .gap(px(16.))
            .child(title_block(
                &t,
                "search",
                t.accent,
                format!("Checking {}", s.os.this_one()),
                "A few things the server needs first.".into(),
            ))
            .child(
                div()
                    .rounded(px(ui::R_CARD))
                    .border_1()
                    .border_color(t.line)
                    .bg(t.panel)
                    .overflow_hidden()
                    .children(rows),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(
                        ui::line_btn(&t, "setup-back1", 32.)
                            .px(px(14.))
                            .text_size(px(13.))
                            .child("Back")
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.setup_page(Page::Intro, cx))),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .children(note.map(|n| ui::wrap(n).text_right().text_size(px(12.)).text_color(t.faint))),
                    )
                    .children(again)
                    .child(next),
            )
            .into_any_element()
    }

    fn check_row(&self, c: &Ctx, s: &Setup, ch: &Check, i: usize) -> AnyElement {
        let t = c.t;
        let cx = c.cx;
        let mark = match ch.mark {
            Mark::Wait => ui::spinner(gpui::ElementId::from(format!("chk-spin-{i}")), 16., t.dim),
            Mark::Ok => badge(t.warm, "check"),
            Mark::Warn => badge(t.cold, "warn"),
            Mark::Bad => badge(t.bad, "x"),
        };
        let laptop_warn = ch.key == Key::Laptop && ch.mark == Mark::Warn;
        let yes = s.laptop_yes;
        let fix: Option<AnyElement> = match (ch.key, ch.mark) {
            (Key::Python, Mark::Bad) if s.os == Os::Mac && matches!(s.python, Some(Python::Missing(_))) => Some(
                ui::line_btn(&t, "setup-clt", 28.)
                    .px(px(12.))
                    .child("Install Apple's command line tools")
                    .on_click(cx.listener(|_, _: &ClickEvent, _, _| {
                        // Apple's own installer asks first; nothing is installed without the user's yes there.
                        if !cfg!(feature = "shots") {
                            let _ = std::process::Command::new("/usr/bin/xcode-select").arg("--install").spawn();
                        }
                    }))
                    .into_any_element(),
            ),
            (Key::Others, Mark::Bad) => self.find.here().map(|h| {
                let url = h.url.clone();
                ui::line_btn(&t, "setup-use-here", 28.)
                    .px(px(12.))
                    .child("Connect to it")
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.setup = None;
                        this.use_found(&url, cx);
                    }))
                    .into_any_element()
            }),
            (Key::Kit, _) => Some(
                ui::link(&t, "setup-guide", "Open the setup guide")
                    .text_size(px(12.5))
                    .on_click(|_: &ClickEvent, _, cx| crate::state::open_link(cx, &ui::install_guide()))
                    .into_any_element(),
            ),
            _ => None,
        };
        div()
            .px(px(14.))
            .py(px(12.))
            .flex()
            .items_start()
            .gap(px(12.))
            .when(i > 0, |d| d.border_t_1().border_color(t.line))
            .when(laptop_warn, |d| d.bg(t.cold.alpha(if t.dark { 0.08 } else { 0.07 })))
            .child(div().flex_none().pt(px(1.)).child(mark))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(3.))
                    .child(
                        ui::wrap(ch.title.clone())
                            .text_size(px(13.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(if ch.mark == Mark::Wait { t.dim } else { t.ink }),
                    )
                    .when(!ch.detail.is_empty(), |d| {
                        d.child(
                            ui::wrap(ch.detail.clone())
                                .text_size(px(12.))
                                .line_height(relative(1.45))
                                .text_color(t.dim),
                        )
                    })
                    .when(laptop_warn, |d| {
                        d.child(
                            div().mt(px(6.)).flex().child(
                                ui::check_chip(&t, "setup-laptop-yes", yes, false, "laptop", "Use this laptop anyway")
                                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                        if let Some(s) = &mut this.setup {
                                            s.laptop_yes = !s.laptop_yes;
                                        }
                                        cx.notify();
                                    })),
                            ),
                        )
                    })
                    .children(fix.map(|f| div().mt(px(6.)).flex().child(f))),
            )
            .into_any_element()
    }

    fn setup_confirm(&self, c: &Ctx, s: &Setup) -> AnyElement {
        let t = c.t;
        let cx = c.cx;
        let this = s.os.this_one();
        let hub = s.hub();
        let item = |ic: &str, text: String| {
            div()
                .flex()
                .items_start()
                .gap(px(10.))
                .child(div().flex_none().pt(px(2.)).child(ui::icon(ic, 13., t.dim)))
                .child(ui::wrap(text).flex_1().text_size(px(12.5)).line_height(relative(1.45)).text_color(t.ink2))
        };
        let services = match s.os {
            Os::Mac => "It starts now and each time you log in, as your own background items. No admin password.",
            Os::Linux => "It starts now and each time you log in, as your own user services. No admin password.",
            Os::Windows => "It starts now and each time you log in, as your own logon tasks.",
        };
        div()
            .flex()
            .flex_col()
            .gap(px(16.))
            .child(title_block(
                &t,
                if s.os == Os::Mac { "mac" } else { "desktop" },
                t.accent,
                format!("Set up {this} as your Switchboard server"),
                "This is what happens next. It takes about a minute.".into(),
            ))
            .child(
                div()
                    .p(px(14.))
                    .rounded(px(ui::R_CARD))
                    .border_1()
                    .border_color(t.line)
                    .bg(t.panel)
                    .flex()
                    .flex_col()
                    .gap(px(10.))
                    .child(item("folder", "The server goes in your home folder, in ~/.local/share/switchboard.".into()))
                    .child(item("relaunch", services.into()))
                    .child(if hub {
                        item(
                            "term",
                            "The hub comes with it, for terminals and for linking machines with a code.".into(),
                        )
                    } else {
                        item(
                            "info",
                            "This build has no hub, so there are no terminals and machines are added over ssh \
                             instead of with a code. Running the installer again from a full copy adds it."
                                .into(),
                        )
                    })
                    .child(item("globe", "It answers on ports 8130, 8131 and 8132, on your network only.".into())),
            )
            .child(footer(
                &t,
                Some(
                    ui::line_btn(&t, "setup-back2", 32.)
                        .px(px(14.))
                        .text_size(px(13.))
                        .child("Back")
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.setup_page(Page::Checks, cx)))
                        .into_any_element(),
                ),
                ui::primary_btn(&t, "setup-run", 32.)
                    .px(px(16.))
                    .child(format!("Set up {this} as your Switchboard server"))
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.run_install(cx)))
                    .into_any_element(),
            ))
            .into_any_element()
    }

    fn setup_running(&self, c: &Ctx, s: &Setup) -> AnyElement {
        let t = c.t;
        let cx = c.cx;
        let failed = s.page == Page::Failed;
        let steps = s.install.steps.iter().enumerate().map(|(i, st)| {
            let lead: AnyElement = match st.state {
                StepState::Todo => div()
                    .size(px(16.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(div().size(px(6.)).rounded_full().bg(t.line2))
                    .into_any_element(),
                StepState::Now => ui::spinner(gpui::ElementId::from(format!("step-spin-{i}")), 16., t.accent),
                StepState::Done => badge(t.warm, "check"),
                StepState::Failed => badge(t.bad, "x"),
            };
            div().flex().items_center().gap(px(12.)).child(lead).child(
                ui::wrap(st.title.clone())
                    .flex_1()
                    .text_size(px(13.))
                    .when(st.state == StepState::Now, |d| d.font_weight(FontWeight::SEMIBOLD))
                    .text_color(match st.state {
                        StepState::Todo => t.faint,
                        StepState::Failed => t.bad,
                        _ => t.ink,
                    }),
            )
        });
        let tail = s.install.tail();
        let shown: Vec<String> = if failed {
            // The step lines are the list above; what the installer said under them is what helps.
            let said: Vec<&String> = tail.iter().filter(|l| !l.starts_with("== ")).collect();
            said[said.len().saturating_sub(8)..].iter().map(|l| l.to_string()).collect()
        } else {
            tail.iter().rev().find(|l| !l.starts_with("== ")).cloned().into_iter().collect()
        };
        let log = (!shown.is_empty()).then(|| {
            div()
                .px(px(12.))
                .py(px(10.))
                .rounded(px(ui::R_CTRL))
                .bg(t.bg2)
                .border_1()
                .border_color(if failed { t.bad.alpha(0.35) } else { t.line })
                .flex()
                .flex_col()
                .gap(px(2.))
                .font_family(NUM)
                .text_size(px(11.5))
                .text_color(if failed { t.ink2 } else { t.faint })
                .children(shown.into_iter().map(ui::wrap))
        });
        let (ic, color, title, sub) = if failed {
            (
                "warn",
                t.bad,
                "Setup didn't finish".to_string(),
                "Here's the last thing the installer said. Fix that, then try again.".to_string(),
            )
        } else {
            (
                "update",
                t.accent,
                format!("Setting up {}", s.os.this_one()),
                "This takes about a minute. Keep the app open.".to_string(),
            )
        };
        div()
            .flex()
            .flex_col()
            .gap(px(16.))
            .child(title_block(&t, ic, color, title, sub))
            .child(
                div()
                    .p(px(14.))
                    .rounded(px(ui::R_CARD))
                    .border_1()
                    .border_color(t.line)
                    .bg(t.panel)
                    .flex()
                    .flex_col()
                    .gap(px(10.))
                    .children(steps),
            )
            .children(log)
            .when(failed, |d| {
                d.child(footer(
                    &t,
                    Some(
                        ui::line_btn(&t, "setup-fail-close", 32.)
                            .px(px(14.))
                            .text_size(px(13.))
                            .child("Close")
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.close_setup(cx)))
                            .into_any_element(),
                    ),
                    ui::primary_btn(&t, "setup-retry", 32.)
                        .px(px(16.))
                        .child("Try again")
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.setup_page(Page::Checks, cx)))
                        .into_any_element(),
                ))
            })
            .into_any_element()
    }

    fn setup_done(&self, c: &Ctx, s: &Setup) -> AnyElement {
        let t = c.t;
        let cx = c.cx;
        let this = s.os.this_one();
        let (awake, btn) = hostsetup::awake_tip(s.os, s.laptop());
        let section = |ic: &str, title: &str, body: String, action: Option<AnyElement>| {
            div()
                .p(px(14.))
                .rounded(px(ui::R_CARD))
                .border_1()
                .border_color(t.line)
                .bg(t.panel)
                .flex()
                .items_start()
                .gap(px(12.))
                .child(div().flex_none().pt(px(1.)).child(ui::icon(ic, 15., t.accent)))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(4.))
                        .child(
                            ui::wrap(title.to_string())
                                .text_size(px(13.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(t.ink),
                        )
                        .child(ui::wrap(body).text_size(px(12.5)).line_height(relative(1.45)).text_color(t.dim))
                        .children(action.map(|a| div().mt(px(6.)).flex().child(a))),
                )
        };
        let open = btn.map(|(label, url)| {
            ui::line_btn(&t, "setup-energy", 28.)
                .px(px(12.))
                .child(ui::icon("gear", 13., t.dim))
                .child(label)
                .on_click(move |_: &ClickEvent, _, cx| crate::state::open_link(cx, url))
                .into_any_element()
        });
        div()
            .flex()
            .flex_col()
            .gap(px(16.))
            .child(title_block(
                &t,
                "check",
                t.warm,
                format!("{} is your Switchboard server", capital(this)),
                "The app is connected to it at localhost:8130.".into(),
            ))
            .child(section("moon", "Keep it awake", awake, open))
            .child(section("remote", "Your phone and other machines", hostsetup::find_tip(s.os), None))
            .child(footer(
                &t,
                Some(
                    ui::line_btn(&t, "setup-add", 32.)
                        .px(px(14.))
                        .text_size(px(13.))
                        .child(ui::icon("plus", 13., t.dim))
                        .child("Add a machine")
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.setup = None;
                            cx.notify();
                            if !cfg!(feature = "shots") {
                                crate::files::open_pair(cx);
                            }
                        }))
                        .into_any_element(),
                ),
                ui::primary_btn(&t, "setup-done", 32.)
                    .px(px(18.))
                    .child("Done")
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.close_setup(cx)))
                    .into_any_element(),
            ))
            .into_any_element()
    }

    /// `shots` states for the sheet: SB_SHOT=setup=<intro|checking|checks|laptop|laptopyes|blocked|confirm|nohub|
    /// running|failed|done|donelaptop>.
    pub fn shot_setup(&mut self, what: &str, cx: &mut Context<Self>) {
        self.find_shot = true;
        let run = self.find.start();
        self.find.finish(run, None);
        let mut s = Setup::new();
        s.shot = true;
        s.kit_looked = true;
        s.kit = Some(Kit { root: PathBuf::from("/Applications/Switchboard.app/Contents/Resources/server-kit") });
        let good = |s: &mut Setup| {
            s.power = Some(Power::Desktop);
            s.python = Some(Python::Found { path: "/usr/bin/python3".into(), version: "3.9.6".into() });
            s.busy = Some(vec![]);
        };
        let laptop = Power::Laptop("It has a built-in battery.".into());
        match what {
            "checking" => s.page = Page::Checks,
            "checks" => {
                good(&mut s);
                s.page = Page::Checks;
            }
            "laptop" | "laptopyes" => {
                good(&mut s);
                s.power = Some(laptop.clone());
                s.laptop_yes = what == "laptopyes";
                s.page = Page::Checks;
            }
            "blocked" => {
                good(&mut s);
                s.python = Some(Python::Missing(
                    "Python 3 comes with Apple's command line tools, which aren't installed yet.".into(),
                ));
                s.busy = Some(vec![8131]);
                let run = self.find.start();
                self.find.add(run, "studio1", "http://studio1.local:8130");
                self.find.finish(run, None);
                s.page = Page::Checks;
            }
            "confirm" | "nohub" => {
                good(&mut s);
                s.page = Page::Confirm;
            }
            "running" => {
                good(&mut s);
                s.page = Page::Running;
            }
            "failed" => {
                good(&mut s);
                s.page = Page::Failed;
            }
            "done" | "donelaptop" => {
                good(&mut s);
                if what == "donelaptop" {
                    s.power = Some(laptop);
                }
                s.page = Page::Done;
            }
            _ => {}
        }
        s.hub_shot = Some(what != "nohub");
        s.install = Install::new(&hostsetup::plan(s.hub()));
        match what {
            "running" => {
                for l in
                    ["== Checking this machine", "Python 3.9.6 at /usr/bin/python3", "== Putting the server in place"]
                {
                    s.install.line(l);
                }
                s.install.line("Server in ~/.local/share/switchboard/app");
            }
            "failed" => {
                for l in [
                    "== Checking this machine",
                    "Python 3.9.6 at /usr/bin/python3",
                    "Time zone America/Los_Angeles",
                    "== Putting the server in place",
                    "Server in ~/.local/share/switchboard/app",
                    "== The hub",
                    "Hub: switchboard-hub 0.9.0",
                    "== Downloads for Add a machine",
                    "  ready: switchboard-node-mac",
                    "== Running it (launchd)",
                    "install-server.sh: the server did not start. See: switchboard logs",
                ] {
                    s.install.line(l);
                }
                s.install.finish(false);
            }
            "done" | "donelaptop" => s.install.finish(true),
            _ => {}
        }
        self.setup = Some(s);
        cx.notify();
    }
}

fn capital(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
}

/// Four short bars over the sheet: where in the setup this is.
fn progress(t: &Theme, page: Page) -> AnyElement {
    let at = match page {
        Page::Intro => 0,
        Page::Checks => 1,
        Page::Confirm | Page::Running | Page::Failed => 2,
        Page::Done => 3,
    };
    let names = ["About", "Checks", "Set up", "Done"];
    div()
        .pr(px(34.))
        .flex()
        .gap(px(6.))
        .children(names.iter().enumerate().map(|(i, n)| {
            let on = i <= at;
            div()
                .flex_1()
                .flex()
                .flex_col()
                .gap(px(6.))
                .child(div().h(px(3.)).rounded_full().bg(if on { t.accent } else { t.track }))
                .child(
                    div()
                        .text_size(px(11.))
                        .font_weight(if i == at { FontWeight::SEMIBOLD } else { FontWeight::NORMAL })
                        .text_color(if i == at { t.ink2 } else { t.faint })
                        .child(n.to_string()),
                )
        }))
        .into_any_element()
}

/// An icon tile with the page's title and the line under it.
fn title_block(t: &Theme, ic: &str, color: Hsla, title: String, sub: String) -> AnyElement {
    div()
        .flex()
        .items_center()
        .gap(px(14.))
        .child(
            div()
                .flex_none()
                .size(px(44.))
                .rounded(px(ui::R_CARD + 2.))
                .bg(color.alpha(if t.dark { 0.16 } else { 0.1 }))
                .flex()
                .items_center()
                .justify_center()
                .child(ui::icon(ic, 22., color)),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(3.))
                .child(
                    ui::wrap(title)
                        .text_size(px(16.))
                        .line_height(relative(1.3))
                        .font_weight(FontWeight::BOLD)
                        .text_color(t.ink),
                )
                .child(ui::wrap(sub).text_size(px(12.5)).line_height(relative(1.4)).text_color(t.dim)),
        )
        .into_any_element()
}

/// A filled round mark: done (check), a warning, or failed (x).
fn badge(color: Hsla, ic: &str) -> AnyElement {
    div()
        .flex_none()
        .size(px(16.))
        .rounded_full()
        .bg(color.alpha(0.18))
        .flex()
        .items_center()
        .justify_center()
        .child(ui::icon(ic, 11., color))
        .into_any_element()
}

/// The sheet's buttons: one on the left, the main one on the right.
fn footer(t: &Theme, left: Option<AnyElement>, right: AnyElement) -> AnyElement {
    let _ = t;
    div()
        .mt(px(2.))
        .flex()
        .flex_wrap()
        .items_center()
        .justify_end()
        .gap(px(8.))
        .children(left)
        .child(right)
        .into_any_element()
}
