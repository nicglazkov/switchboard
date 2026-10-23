//! Settings > Server: the computer that runs Switchboard, for someone who never opens a terminal. Whether each part
//! is running, its version and a one-click update to the newest release (server/serverctl.py installs the kit the
//! signed list names), a restart, its logs, and its details to copy for a bug report.

use std::time::Duration;

use gpui::{
    AnyElement, ClickEvent, Context, FontWeight, IntoElement, ParentElement, SharedString, StatefulInteractiveElement,
    Styled, div, prelude::*, px, relative,
};
use sb_core::api::{ApiServer, ApiServerUpdate};
use sb_core::update::ServerOffer;

use super::SettingsView;
use crate::state::AppState;
use crate::theme::{NUM, Theme};
use crate::ui;

/// How often the page asks while an update or a restart is under way, and how long it waits for the server to come
/// back before it says something went wrong.
const POLL: Duration = Duration::from_millis(1500);
const BACK_WITHIN: f64 = 180.;
/// Restart asks twice: the second click within this many seconds.
const ARM_FOR: f64 = 4.;
const LOG_LINES: u32 = 300;

#[derive(Default)]
pub(super) struct Pane {
    /// The server's answer; `old` when it is from before 0.8.11 (no /api/server).
    info: Option<Result<ApiServer, String>>,
    old: bool,
    loading: bool,
    offer: Option<Result<Option<ServerOffer>, String>>,
    checking: bool,
    /// An update or a restart was asked for at this time, and the page is watching for the server to come back.
    waiting: Option<(Kind, f64)>,
    /// The server stopped answering while it was being waited for (it is restarting).
    away: bool,
    sending: bool,
    error: Option<String>,
    restart_armed: Option<f64>,
    logs: Option<Logs>,
    /// Bumped when the page closes or starts watching anew, so an older watch stops.
    watch: u64,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Update,
    Restart,
}

struct Logs {
    part: &'static str,
    text: Option<Result<String, String>>,
}

/// An update's steps as the page shows them, from selfupdate.py's.
const STEPS: [(&str, &[&str]); 4] = [
    ("Download", &["starting", "downloading"]),
    ("Check", &["unpacking", "checking"]),
    ("Install", &["server", "hub", "downloads"]),
    ("Restart", &["restarting", "starting_up"]),
];

fn step_index(step: &str) -> usize {
    if step == "done" {
        return STEPS.len();
    }
    STEPS.iter().position(|(_, s)| s.contains(&step)).unwrap_or(0)
}

fn size_text(b: u64) -> String {
    let g = b as f64 / 1e9;
    if g >= 10. {
        format!("{g:.0} GB")
    } else if g >= 1. {
        format!("{g:.1} GB")
    } else {
        format!("{:.0} MB", b as f64 / 1e6)
    }
}

fn uptime(started: f64, now: f64) -> String {
    let s = (now - started).max(0.);
    if s < 3600. {
        format!("{} min", (s / 60.).floor().max(1.))
    } else if s < 86400. {
        let h = (s / 3600.).floor();
        if h == 1. { "1 hour".into() } else { format!("{h} hours") }
    } else {
        let d = (s / 86400.).floor();
        if d == 1. { "1 day".into() } else { format!("{d} days") }
    }
}

fn mode_text(m: &str) -> &'static str {
    match m {
        "systemd" => "systemd user services",
        "launchd" => "LaunchAgents",
        "plain" => "background processes",
        "windows" => "Windows scheduled tasks",
        "checkout" => "a copy of the code",
        _ => "",
    }
}

/// The install line for a server that can't say it (from before 0.8.11).
fn install_line() -> String {
    format!(
        "curl -fsSL {}/releases/latest/download/get-server.sh | bash",
        sb_core::brand::HOMEPAGE.trim_end_matches('/')
    )
}

impl SettingsView {
    /// Opening the page: what the server says, and whether the signed list has a newer one.
    pub(super) fn server_open(&mut self, cx: &mut Context<Self>) {
        self.srv.error = None;
        self.server_load(true, cx);
        // Screenshots: SB_SHOT_SERVER_LOGS=1 opens with the logs showing.
        #[cfg(feature = "shots")]
        if std::env::var("SB_SHOT_SERVER_LOGS").is_ok_and(|v| v == "1") {
            self.server_logs("server", cx);
        }
    }

    pub(super) fn server_close(&mut self) {
        self.srv.watch += 1;
        self.srv.waiting = None;
        self.srv.restart_armed = None;
    }

    fn server_load(&mut self, offer_too: bool, cx: &mut Context<Self>) {
        if self.srv.loading {
            return;
        }
        self.srv.loading = true;
        let core = self.app.read(cx).core.clone();
        cx.spawn(async move |this, cx| {
            let res = cx.background_executor().spawn(async move { core.server_info() }).await;
            let _ = this.update(cx, |v, cx| {
                v.srv.loading = false;
                v.srv.old = matches!(&res, Err(sb_core::net::CoreError::Server { status: 404, .. }));
                let ver = res.as_ref().map(|i| i.version.clone()).unwrap_or_default();
                v.srv.info = Some(res.map_err(|e| e.to_string()));
                if offer_too {
                    v.server_check(ver, cx);
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn server_check(&mut self, ver: String, cx: &mut Context<Self>) {
        if self.srv.checking {
            return;
        }
        self.srv.checking = true;
        let core = self.app.read(cx).core.clone();
        cx.spawn(async move |this, cx| {
            let res = cx.background_executor().spawn(async move { core.server_offer(&ver) }).await;
            // Screenshots: SB_SHOT_SERVER_OFFER=<version> stands in for a newer kit in the signed list.
            #[cfg(feature = "shots")]
            let res = match std::env::var("SB_SHOT_SERVER_OFFER") {
                Ok(v) if !v.is_empty() => Ok(Some(ServerOffer {
                    version: v.clone(),
                    sha256: "0".repeat(64),
                    size: 7_400_000,
                    page: Some(format!("{}/releases/tag/v{v}", sb_core::brand::HOMEPAGE)),
                })),
                _ => res,
            };
            let _ = this.update(cx, |v, cx| {
                v.srv.checking = false;
                v.srv.offer = Some(res.map_err(|e| e.to_string()));
                cx.notify();
            });
        })
        .detach();
    }

    fn server_update(&mut self, o: ServerOffer, cx: &mut Context<Self>) {
        if self.srv.sending {
            return;
        }
        self.srv.sending = true;
        self.srv.error = None;
        cx.notify();
        let core = self.app.read(cx).core.clone();
        cx.spawn(async move |this, cx| {
            let res = cx.background_executor().spawn(async move { core.server_update(&o.version, &o.sha256) }).await;
            let _ = this.update(cx, |v, cx| {
                v.srv.sending = false;
                match res {
                    Ok(()) => v.server_watch(Kind::Update, cx),
                    Err(e) => v.srv.error = Some(e.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn server_restart(&mut self, cx: &mut Context<Self>) {
        let now = AppState::now();
        if !self.srv.restart_armed.is_some_and(|at| now - at < ARM_FOR) {
            self.srv.restart_armed = Some(now);
            cx.notify();
            return;
        }
        self.srv.restart_armed = None;
        self.srv.sending = true;
        self.srv.error = None;
        cx.notify();
        let core = self.app.read(cx).core.clone();
        cx.spawn(async move |this, cx| {
            let res = cx.background_executor().spawn(async move { core.server_restart() }).await;
            let _ = this.update(cx, |v, cx| {
                v.srv.sending = false;
                match res {
                    Ok(()) => v.server_watch(Kind::Restart, cx),
                    Err(e) => v.srv.error = Some(e.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Asks every POLL until the update has finished (or the restart is over) and the server answers again.
    fn server_watch(&mut self, kind: Kind, cx: &mut Context<Self>) {
        self.srv.watch += 1;
        let ticket = self.srv.watch;
        let since = AppState::now();
        self.srv.waiting = Some((kind, since));
        self.srv.away = false;
        let core = self.app.read(cx).core.clone();
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(POLL).await;
                let c = core.clone();
                let res = cx.background_executor().spawn(async move { c.server_info() }).await;
                let go_on = this.update(cx, |v, cx| {
                    if v.srv.watch != ticket {
                        return false;
                    }
                    let now = AppState::now();
                    let done = match &res {
                        Err(_) => {
                            v.srv.away = true;
                            false
                        }
                        Ok(i) => {
                            let fin = match kind {
                                // Over once it has an answer, and has been away (or long enough that it went
                                // and came back between two looks).
                                Kind::Restart => v.srv.away || now - since > 12.,
                                Kind::Update => i.update.as_ref().is_some_and(|u| u.ok.is_some() && u.at >= since - 5.),
                            };
                            v.srv.info = Some(Ok(i.clone()));
                            v.srv.old = false;
                            fin
                        }
                    };
                    let late = now - since > BACK_WITHIN;
                    if done || late {
                        v.srv.waiting = None;
                        if late && !done {
                            v.srv.error = Some(if v.srv.away {
                                "The server hasn't come back yet. Give it a minute, then check the computer it runs on."
                                    .into()
                            } else {
                                "The update is taking longer than it should. Its log may say why.".into()
                            });
                        }
                        let ver = v.srv.info.as_ref().and_then(|i| i.as_ref().ok()).map(|i| i.version.clone());
                        if let Some(ver) = ver {
                            v.srv.offer = None;
                            v.server_check(ver, cx);
                        }
                        // The board's connection came through a restart too: look again now.
                        v.app.update(cx, |s, cx| s.refresh_in(Duration::from_millis(200), cx));
                    }
                    cx.notify();
                    !(done || late)
                });
                if !go_on.unwrap_or(false) {
                    break;
                }
            }
        })
        .detach();
    }

    fn server_logs(&mut self, part: &'static str, cx: &mut Context<Self>) {
        self.srv.logs = Some(Logs { part, text: None });
        cx.notify();
        let core = self.app.read(cx).core.clone();
        cx.spawn(async move |this, cx| {
            let res = cx.background_executor().spawn(async move { core.server_logs(part, LOG_LINES) }).await;
            let _ = this.update(cx, |v, cx| {
                if let Some(l) = v.srv.logs.as_mut().filter(|l| l.part == part) {
                    l.text = Some(res.map_err(|e| e.to_string()));
                }
                cx.notify();
            });
        })
        .detach();
    }

    // ---------- the page ----------

    pub(super) fn server(&self, t: &Theme, cx: &Context<Self>) -> AnyElement {
        let now = AppState::now();
        let addr = self.app.read(cx).core.server().unwrap_or_default();
        let sub = "The computer that runs Switchboard for all your devices. Your chats run on your own computers, so \
                   updating or restarting it never stops them.";
        let page = div().child(Self::head(t, "Server", sub));
        let info = match &self.srv.info {
            None => return page.child(ui::loading(t, "srv-loading", "Asking the server")).into_any_element(),
            Some(Err(_)) if self.srv.old => return page.child(self.server_old(t, &addr, cx)).into_any_element(),
            Some(Err(e)) => {
                return page
                    .child(self.server_banner(t, "warn", t.bad, "Can't reach the server", e))
                    .child(
                        div().mt(px(12.)).child(
                            ui::line_btn(t, "srv-retry", 32.)
                                .child("Try again")
                                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.server_load(true, cx))),
                        ),
                    )
                    .into_any_element();
            }
            Some(Ok(i)) => i,
        };
        page.child(self.server_status(t, info, &addr, now))
            .child(self.server_updates(t, info, now, cx))
            .child(self.server_care(t, info, &addr, now, cx))
            .into_any_element()
    }

    /// A strip across the top: an icon in a tinted circle, a title and a line under it.
    fn server_banner(&self, t: &Theme, ic: &str, c: gpui::Hsla, title: &str, note: &str) -> AnyElement {
        div()
            .p(px(14.))
            .flex()
            .items_center()
            .gap(px(12.))
            .rounded(px(ui::R_CARD))
            .border_1()
            .border_color(t.line)
            .bg(t.bg)
            .child(
                div()
                    .flex_none()
                    .size(px(34.))
                    .rounded_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(c.alpha(if t.dark { 0.18 } else { 0.12 }))
                    .child(ui::icon(ic, 17., c)),
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
                            .text_size(px(14.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(t.ink)
                            .child(title.to_string()),
                    )
                    .child(
                        ui::wrap(note.to_string()).text_size(px(12.5)).line_height(relative(1.45)).text_color(t.dim),
                    ),
            )
            .into_any_element()
    }

    fn server_status(&self, t: &Theme, i: &ApiServer, addr: &str, now: f64) -> AnyElement {
        let restarting = self.srv.waiting.is_some() && self.srv.away;
        let hub_ok = !i.hub.installed || i.hub.running;
        let (ic, c, title) = if restarting {
            ("relaunch", t.cold, "Restarting".to_string())
        } else if hub_ok && i.downloads.running {
            ("check", t.warm, "Running".to_string())
        } else {
            ("warn", t.need, "Running, with a part stopped".to_string())
        };
        let mut note = vec![];
        if !i.host.is_empty() {
            note.push(i.host.clone());
        }
        if !i.os.trim().is_empty() {
            note.push(i.os.trim().to_string());
        }
        if i.started > 0. && !restarting {
            note.push(format!("up {}", uptime(i.started, now)));
        }
        let dot = |on: bool| div().flex_none().size(px(8.)).rounded_full().bg(if on { t.warm } else { t.bad });
        let part = |on: bool, text: String| {
            div().flex().items_center().gap(px(8.)).text_size(px(13.)).text_color(t.ink2).child(dot(on)).child(text)
        };
        let hub_text = if !i.hub.installed {
            "Not installed: no terminals in the app, and no computers linked with a code".to_string()
        } else if !i.hub.running {
            "Not running: terminals and linked computers are down. Restart the server.".to_string()
        } else {
            let open = match i.hub.open {
                0 => "no terminals open".to_string(),
                1 => "1 terminal open".to_string(),
                n => format!("{n} terminals open"),
            };
            let linked = match i.hub.linked {
                0 => String::new(),
                1 => ", 1 computer linked".to_string(),
                n => format!(", {n} computers linked"),
            };
            format!("Running, {open}{linked}")
        };
        let mut rows = vec![
            Self::crow(
                t,
                "Address",
                None,
                Some(
                    div()
                        .font_family(NUM)
                        .text_size(px(12.5))
                        .text_color(t.ink2)
                        .child(addr.to_string())
                        .into_any_element(),
                ),
            ),
            Self::crow(
                t,
                "Terminals and linked computers",
                None,
                Some(part(!i.hub.installed || i.hub.running, hub_text).into_any_element()),
            ),
            Self::crow(
                t,
                "Add a machine downloads",
                None,
                Some(
                    part(
                        i.downloads.running,
                        if i.downloads.running { "Running".into() } else { "Not running. Restart the server.".into() },
                    )
                    .into_any_element(),
                ),
            ),
        ];
        if let Some(d) = i.disk.as_ref().filter(|d| d.total > 0) {
            let low = d.free < 2_000_000_000;
            rows.push(Self::crow(
                t,
                "Disk space",
                low.then(|| "Running low: history and logs need room to grow.".to_string()),
                Some(
                    div()
                        .text_size(px(13.))
                        .text_color(if low { t.bad } else { t.ink2 })
                        .child(format!("{} free of {}", size_text(d.free), size_text(d.total)))
                        .into_any_element(),
                ),
            ));
        }
        div()
            .flex()
            .flex_col()
            .child(self.server_banner(t, ic, c, &title, &note.join(" · ")))
            .child(Self::group(t, "Parts", None, rows))
            .into_any_element()
    }

    fn server_updates(&self, t: &Theme, i: &ApiServer, now: f64, cx: &Context<Self>) -> AnyElement {
        let ver = if i.version.is_empty() { "an older version".to_string() } else { i.version.clone() };
        let up = i.update.as_ref();
        let running = up.is_some_and(|u| u.ok.is_none()) || self.srv.waiting.is_some_and(|(k, _)| k == Kind::Update);
        let mut rows: Vec<AnyElement> = vec![];
        if running {
            rows.push(self.server_progress(t, up, now));
        } else if !i.can_manage {
            rows.push(Self::crow(t, format!("Version {ver}"), Some(i.why_not.clone().unwrap_or_default()), None));
            if i.mode != "windows" && i.mode != "checkout" {
                rows.push(self.server_line_row(t, &i.install_line));
            }
        } else {
            rows.push(self.server_offer_row(t, &ver, cx));
            // How the last update went, once, until a newer look replaces it.
            if let Some(u) = up.filter(|u| u.ok.is_some() && now - u.finished.unwrap_or(u.at) < 86400.) {
                rows.push(self.server_result_row(t, u));
            }
        }
        if let Some(e) = &self.srv.error {
            rows.push(
                div()
                    .flex()
                    .items_start()
                    .gap(px(8.))
                    .text_size(px(12.5))
                    .text_color(t.bad)
                    .child(ui::icon("warn", 14., t.bad))
                    .child(ui::wrap(e.clone()).line_height(relative(1.45)))
                    .into_any_element(),
            );
        }
        Self::group(
            t,
            "Updates",
            Some("New versions of the server come from the same signed releases as this app."),
            rows,
        )
    }

    fn server_offer_row(&self, t: &Theme, ver: &str, cx: &Context<Self>) -> AnyElement {
        let check = ui::ghost(t, "srv-check", Some(if self.srv.checking { "Checking" } else { "Check now" }), None)
            .h(px(32.))
            .px(px(10.))
            .text_size(px(13.))
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                let ver =
                    this.srv.info.as_ref().and_then(|i| i.as_ref().ok()).map(|i| i.version.clone()).unwrap_or_default();
                this.server_check(ver, cx);
            }));
        match &self.srv.offer {
            Some(Ok(Some(o))) => {
                let o2 = o.clone();
                let page = o.page.clone();
                let ctl = div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .when_some(page, |d, p| {
                        d.child(
                            ui::ghost(t, "srv-new", Some("What's new"), None)
                                .h(px(32.))
                                .px(px(10.))
                                .text_size(px(13.))
                                .on_click(move |_, _, cx| crate::state::open_link(cx, &p)),
                        )
                    })
                    .child(
                        ui::primary_btn(t, "srv-update", 32.)
                            .when(self.srv.sending, |d| d.opacity(0.6))
                            .child(ui::icon("update", 14., t.on_accent))
                            .child(if self.srv.sending { "Starting" } else { "Update server" })
                            .on_click(
                                cx.listener(move |this, _: &ClickEvent, _, cx| this.server_update(o2.clone(), cx)),
                            ),
                    );
                Self::crow(
                    t,
                    format!("Version {} is ready", o.version),
                    Some(format!(
                        "This server has {ver}. Updating takes about a minute: your chats keep running, and open \
                         terminals reconnect by themselves."
                    )),
                    Some(ctl.into_any_element()),
                )
            }
            Some(Ok(None)) => Self::crow(
                t,
                format!("Version {ver}"),
                Some(if sb_core::update::key().is_none() {
                    "This build of the app can't check releases, so it can't offer server updates.".into()
                } else {
                    "This is the newest version.".into()
                }),
                Some(check.into_any_element()),
            ),
            Some(Err(e)) => Self::crow(
                t,
                format!("Version {ver}"),
                Some(format!("Couldn't look for a newer one: {e}")),
                Some(check.into_any_element()),
            ),
            None => Self::crow(t, format!("Version {ver}"), Some("Looking for a newer version".into()), None),
        }
    }

    /// Download, Check, Install, Restart: done ones ticked, the current one with a spinner, and the download's bar.
    fn server_progress(&self, t: &Theme, u: Option<&ApiServerUpdate>, _now: f64) -> AnyElement {
        let step = u.map(|u| u.step.as_str()).unwrap_or("starting");
        let at = if self.srv.away { 3 } else { step_index(step) };
        let to = u.map(|u| u.version.clone()).unwrap_or_default();
        let mut steps = div().flex().items_center().gap(px(6.));
        for (n, (label, _)) in STEPS.iter().enumerate() {
            let (c, ic): (gpui::Hsla, Option<&str>) = if n < at {
                (t.warm, Some("check"))
            } else if n == at {
                (t.ink, None)
            } else {
                (t.faint, None)
            };
            if n > 0 {
                steps = steps.child(div().flex_none().w(px(18.)).h(px(1.)).bg(if n <= at { t.warm } else { t.line2 }));
            }
            steps = steps.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .text_size(px(12.5))
                    .text_color(c)
                    .when(n == at, |d| d.font_weight(FontWeight::SEMIBOLD))
                    .child(match ic {
                        Some(i) => ui::icon(i, 13., c).into_any_element(),
                        None if n == at => ui::spinner(("srv-step", n), 13., t.accent),
                        None => div().flex_none().size(px(7.)).mx(px(3.)).rounded_full().bg(t.line2).into_any_element(),
                    })
                    .child(*label),
            );
        }
        let frac = u
            .filter(|u| u.step == "downloading" && u.total > 0)
            .map(|u| (u.got as f32 / u.total as f32).clamp(0.02, 1.));
        let note = match at {
            0 => "Downloading the new server from the release.".to_string(),
            1 => "Checking it is exactly the release's file.".to_string(),
            2 => "Putting it in place.".to_string(),
            _ => "Starting it again. The app reconnects by itself.".to_string(),
        };
        div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(div().text_size(px(13.5)).font_weight(FontWeight::SEMIBOLD).text_color(t.ink).child(
                if to.is_empty() { "Updating the server".to_string() } else { format!("Updating the server to {to}") },
            ))
            .child(steps)
            .when_some(frac, |d, f| {
                d.child(
                    div()
                        .h(px(4.))
                        .rounded(px(2.))
                        .bg(t.track)
                        .child(div().h_full().w(relative(f)).rounded(px(2.)).bg(t.accent)),
                )
            })
            .child(div().text_size(px(12.5)).text_color(t.dim).child(note))
            .into_any_element()
    }

    fn server_result_row(&self, t: &Theme, u: &ApiServerUpdate) -> AnyElement {
        let ok = u.ok == Some(true);
        let text = if ok {
            if u.from.is_empty() {
                format!("Updated to {}", u.version)
            } else {
                format!("Updated from {} to {}", u.from, u.version)
            }
        } else {
            format!("The update to {} didn't finish: {}", u.version, u.error.clone().unwrap_or_default())
        };
        div()
            .flex()
            .items_start()
            .gap(px(8.))
            .text_size(px(12.5))
            .text_color(if ok { t.ink2 } else { t.bad })
            .child(ui::icon(if ok { "check" } else { "warn" }, 14., if ok { t.warm } else { t.bad }))
            .child(ui::wrap(text).line_height(relative(1.45)))
            .into_any_element()
    }

    /// The install line, to run once on the server, with Copy.
    fn server_line_row(&self, t: &Theme, line: &str) -> AnyElement {
        let copy = line.to_string();
        div()
            .flex()
            .items_center()
            .gap(px(8.))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .px(px(10.))
                    .py(px(8.))
                    .rounded(px(ui::R_CTRL))
                    .bg(t.panel)
                    .border_1()
                    .border_color(t.line)
                    .font_family(NUM)
                    .text_size(px(12.))
                    .text_color(t.ink2)
                    .child(ui::wrap(line.to_string())),
            )
            .child(
                ui::line_btn(t, "srv-copy-line", 32.)
                    .child(ui::icon("copy", 13., t.dim))
                    .child("Copy")
                    .on_click(move |_, _, cx| crate::state::copy_text(cx, copy.clone())),
            )
            .into_any_element()
    }

    /// A server from before 0.8.11: it can't say its version or update itself, so it is updated once by hand.
    fn server_old(&self, t: &Theme, addr: &str, cx: &Context<Self>) -> AnyElement {
        let line = install_line();
        div()
            .flex()
            .flex_col()
            .child(self.server_banner(
                t,
                "update",
                t.accent,
                "This server needs one update by hand",
                &format!(
                    "The server at {addr} is from before version 0.8.11, so it can't update itself yet. Run this line \
                     once in a terminal on that computer. It keeps your computers, settings and history, and from then \
                     on updates are a click here."
                ),
            ))
            .child(Self::group(t, "Update it", None, vec![self.server_line_row(t, &line)]))
            .child(
                div().mt(px(14.)).flex().gap(px(8.)).child(
                    ui::line_btn(t, "srv-recheck", 32.)
                        .child(ui::icon("update", 13., t.dim))
                        .child("Check again")
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.server_load(true, cx))),
                ),
            )
            .into_any_element()
    }

    fn server_care(&self, t: &Theme, i: &ApiServer, addr: &str, now: f64, cx: &Context<Self>) -> AnyElement {
        let mut rows = vec![];
        if i.can_manage {
            let armed = self.srv.restart_armed.is_some_and(|at| now - at < ARM_FOR);
            let busy = self.srv.waiting.is_some() || self.srv.sending;
            let btn = ui::line_btn(t, "srv-restart", 32.)
                .when(armed, |d| d.text_color(t.bad))
                .when(busy, |d| d.opacity(0.5))
                .child(ui::icon("relaunch", 13., if armed { t.bad } else { t.dim }))
                .child(if armed { "Click again to restart" } else { "Restart" });
            let btn = if busy {
                btn
            } else {
                btn.on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.server_restart(cx)))
            };
            rows.push(Self::crow(
                t,
                "Restart the server",
                Some(
                    "Fixes most problems. It takes a few seconds; open terminals reconnect by themselves and chats keep \
                     running."
                        .into(),
                ),
                Some(btn.into_any_element()),
            ));
        }
        let open = self.srv.logs.is_some();
        rows.push(Self::crow(
            t,
            "Logs",
            Some("What the server has been doing, for when something isn't right.".into()),
            Some(
                ui::line_btn(t, "srv-logs", 32.)
                    .child(ui::icon(if open { "up" } else { "down" }, 12., t.dim))
                    .child(if open { "Hide logs" } else { "Show logs" })
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        if this.srv.logs.is_some() {
                            this.srv.logs = None;
                            cx.notify();
                        } else {
                            this.server_logs("server", cx);
                        }
                    }))
                    .into_any_element(),
            ),
        ));
        if let Some(l) = &self.srv.logs {
            rows.push(self.server_log_view(t, l, cx));
        }
        let details = server_details(i, addr);
        rows.push(Self::crow(
            t,
            "Details for a bug report",
            Some(format!("Version, system and how it runs ({}).", mode_text(&i.mode))),
            Some(
                ui::line_btn(t, "srv-copy", 32.)
                    .child(ui::icon("copy", 13., t.dim))
                    .child("Copy details")
                    .on_click(move |_, _, cx| crate::state::copy_text(cx, details.clone()))
                    .into_any_element(),
            ),
        ));
        rows.push(Self::crow(
            t,
            "Help",
            Some("Installing, updating and moving the server.".into()),
            Some(
                ui::ghost(t, "srv-help", Some("Server guide"), Some("docs"))
                    .h(px(32.))
                    .px(px(10.))
                    .text_size(px(13.))
                    .on_click(|_, _, cx| crate::state::open_link(cx, &sb_core::brand::docs("updates/#the-server")))
                    .into_any_element(),
            ),
        ));
        Self::group(t, "Care", None, rows)
    }

    fn server_log_view(&self, t: &Theme, l: &Logs, cx: &Context<Self>) -> AnyElement {
        let (hover, sel) = (t.hover, t.sel);
        let mut tabs = div().flex().items_center().gap(px(2.));
        for (part, label) in [("server", "Server"), ("hub", "Terminals"), ("downloads", "Downloads")] {
            let on = l.part == part;
            tabs = tabs.child(
                div()
                    .id(SharedString::from(format!("srv-log-{part}")))
                    .h(px(26.))
                    .px(px(10.))
                    .flex()
                    .items_center()
                    .rounded(px(ui::R_ITEM))
                    .cursor_pointer()
                    .text_size(px(12.5))
                    .when(on, |d| d.bg(sel).text_color(t.ink).font_weight(FontWeight::SEMIBOLD))
                    .when(!on, |d| d.text_color(t.dim).hover(move |s| s.bg(hover)))
                    .child(label)
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.server_logs(part, cx))),
            );
        }
        let part = l.part;
        let text = match &l.text {
            Some(Ok(s)) => s.clone(),
            _ => String::new(),
        };
        let body = match &l.text {
            None => ui::loading(t, "srv-log-loading", "Reading the log").into_any_element(),
            Some(Err(e)) => div().text_size(px(12.5)).text_color(t.bad).child(e.clone()).into_any_element(),
            Some(Ok(s)) if s.trim().is_empty() => {
                div().text_size(px(12.5)).text_color(t.faint).child("Nothing in this log yet.").into_any_element()
            }
            Some(Ok(s)) => div()
                .font_family(NUM)
                .text_size(px(11.5))
                .line_height(relative(1.5))
                .text_color(t.ink2)
                .children(s.lines().rev().map(|line| ui::wrap(line.to_string())))
                .into_any_element(),
        };
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .child(tabs)
                    .child(div().flex_1())
                    .child(div().mr(px(6.)).text_size(px(12.)).text_color(t.faint).child("Newest first"))
                    .child(
                        ui::ghost(t, "srv-log-refresh", Some("Refresh"), Some("update"))
                            .h(px(28.))
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.server_logs(part, cx))),
                    )
                    .child(
                        ui::ghost(t, "srv-log-copy", Some("Copy"), Some("copy"))
                            .h(px(28.))
                            .on_click(move |_, _, cx| crate::state::copy_text(cx, text.clone())),
                    ),
            )
            .child(
                div()
                    .id("srv-log-text")
                    .h(px(300.))
                    .overflow_y_scroll()
                    .p(px(10.))
                    .rounded(px(ui::R_CTRL))
                    .bg(t.panel)
                    .border_1()
                    .border_color(t.line)
                    .child(body),
            )
            .into_any_element()
    }
}

/// What "Copy details" puts on the clipboard.
fn server_details(i: &ApiServer, addr: &str) -> String {
    let mut out = vec![
        format!(
            "Switchboard server {}{}",
            if i.version.is_empty() { "(version unknown)" } else { &i.version },
            if i.commit.is_empty() { String::new() } else { format!(" ({})", i.commit) }
        ),
        format!("Address: {addr}"),
        format!("Host: {}, {}", i.host, i.os.trim()),
        format!("Runs as: {} ({})", mode_text(&i.mode), i.mode),
        format!("Python: {}", i.python),
        format!(
            "Hub: {}",
            if !i.hub.installed {
                "not installed".to_string()
            } else if i.hub.running {
                format!("running {}, {} open, {} linked", i.hub.version, i.hub.open, i.hub.linked)
            } else {
                "not running".to_string()
            }
        ),
        format!("Downloads: {}", if i.downloads.running { "running" } else { "not running" }),
        format!("App: Switchboard {}", sb_core::version()),
    ];
    if let Some(u) = &i.update {
        out.push(format!(
            "Last update: {} to {}, {}",
            if u.from.is_empty() { "?" } else { &u.from },
            u.version,
            match u.ok {
                Some(true) => "done".to_string(),
                Some(false) => format!("failed: {}", u.error.clone().unwrap_or_default()),
                None => format!("at {}", u.step),
            }
        ));
    }
    out.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_follow_the_updater() {
        assert_eq!(step_index("downloading"), 0);
        assert_eq!(step_index("checking"), 1);
        assert_eq!(step_index("hub"), 2);
        assert_eq!(step_index("starting_up"), 3);
        assert_eq!(step_index("done"), 4);
    }

    #[test]
    fn uptime_reads_plainly() {
        assert_eq!(uptime(0., 90.), "1 min");
        assert_eq!(uptime(0., 3600. * 5.), "5 hours");
        assert_eq!(uptime(0., 86400. * 3.), "3 days");
        assert_eq!(size_text(120_000_000_000), "120 GB");
        assert_eq!(size_text(1_500_000_000), "1.5 GB");
    }

    #[test]
    fn the_install_line_comes_from_the_homepage() {
        assert!(install_line().starts_with("curl -fsSL https://"));
        assert!(install_line().ends_with("/releases/latest/download/get-server.sh | bash"));
    }
}
