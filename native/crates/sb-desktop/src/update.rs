//! Updates in the app (docs/architecture.md, Updates). It looks for a newer build a little after launch and then
//! once a day, and offers it in the header: "Restart to update" installs it now and opens it, "Install on quit"
//! installs it when Switchboard quits. Nothing is installed until the user picks one. sb-core's update.rs checks the
//! signed list, downloads and swaps; this file unpacks the download, makes sure the new build runs, and handles
//! the first start after an install (the old build comes back if the new one never reports healthy).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use gpui::{
    AnyElement, ClickEvent, Context, FontWeight, InteractiveElement, IntoElement, MouseButton, ParentElement,
    StatefulInteractiveElement, Styled, div, prelude::FluentBuilder, px, relative,
};
use sb_core::update::{self as up, Offer};

use crate::state::AppState;
use crate::ui;
use crate::view::{BoardView, Ctx};

/// A new build that has run this long without crashing counts as healthy, and the old copy goes.
const HEALTHY_AFTER: Duration = Duration::from_secs(20);

/// When the download is done: install now and reopen, or wait for quit.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Then {
    Restart,
    OnQuit,
}

#[derive(Clone, Default)]
pub enum Upd {
    #[default]
    Idle,
    Getting {
        got: Arc<AtomicU64>,
        total: Arc<AtomicU64>,
        then: Then,
    },
    /// Downloaded, checked and unpacked; it goes in when Switchboard quits.
    OnQuit(Box<Staged>),
    Failed(String),
}

/// A checked build unpacked beside the one it replaces, ready for the swap.
#[derive(Clone)]
pub struct Staged {
    new: PathBuf,
    target: PathBuf,
    offer: Offer,
    /// Folders to delete once it is in place.
    scratch: Vec<PathBuf>,
}

/// Before any window: handles the first start after an install (sb_core::update::at_launch). When the new build
/// failed its first run, this puts the old one back, opens it and exits.
pub fn at_launch() {
    let Some(state) = up::state_path() else { return };
    if let Some(back) = up::at_launch(up::kind(), &state, up::this_build()) {
        if let Err(e) = open_app(&back) {
            log::error!("could not open the previous version again: {e}");
        }
        std::process::exit(0);
    }
}

impl AppState {
    /// The healthy report and Install on quit, set up once at start.
    pub fn update_hooks(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |_, cx| {
            cx.background_executor().timer(HEALTHY_AFTER).await;
            healthy();
        })
        .detach();
        cx.on_app_quit(|s, _| {
            // A clean quit counts as a healthy run too.
            healthy();
            if let Upd::OnQuit(st) = &s.upd {
                match swap(st) {
                    Ok(()) => log::info!("update: {} installed on quit", st.offer.version),
                    Err(e) => log::error!("update on quit: {e}"),
                }
            }
            async {}
        })
        .detach();
    }

    /// Looks for a newer build when one is due (a little after launch, then daily), one look at a time.
    pub fn check_update(&mut self, cx: &mut Context<Self>) {
        let now = AppState::now();
        if up::check_due(now, self.update_started, self.update_at, self.update_busy) {
            self.look_for_update(cx);
        }
    }

    /// Looks now (Settings > Updates, Check now), unless a look is already under way.
    pub fn look_for_update(&mut self, cx: &mut Context<Self>) {
        if up::off_reason().is_some() || self.update_busy {
            return;
        }
        self.update_at = Some(AppState::now());
        self.update_busy = true;
        cx.notify();
        let core = self.core.clone();
        cx.spawn(async move |this, cx| {
            let res = cx
                .background_executor()
                .spawn(async move {
                    let bad = up::state_path().and_then(|p| up::load_state(&p).bad_build);
                    core.update_offer(bad.as_deref())
                })
                .await;
            let _ = this.update(cx, |s, cx| {
                s.update_busy = false;
                match res {
                    // A download or a queued install keeps the offer it is for.
                    Ok(o) if matches!(s.upd, Upd::Idle | Upd::Failed(_)) => s.update = o,
                    Ok(_) => {}
                    Err(e) => log::info!("update check: {e}"),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Restart to update or Install on quit: download (resuming), check, unpack, make sure it runs; then install
    /// and reopen, or wait for quit.
    pub fn run_update(&mut self, then: Then, cx: &mut Context<Self>) {
        let Some(o) = self.update.clone() else { return };
        match (&self.upd, then) {
            (Upd::Getting { .. }, _) => return,
            (Upd::OnQuit(st), Then::Restart) => {
                let st = (**st).clone();
                self.upd = Upd::Idle;
                self.finish(Ok(st), Then::Restart, cx);
                return;
            }
            (Upd::OnQuit(_), Then::OnQuit) => return,
            _ => {}
        }
        let (got, total) = (Arc::new(AtomicU64::new(0)), Arc::new(AtomicU64::new(0)));
        self.upd = Upd::Getting { got: got.clone(), total: total.clone(), then };
        cx.notify();
        // Redraw while it comes down, so the bar moves.
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_millis(200)).await;
                let going = this.update(cx, |s, cx| {
                    cx.notify();
                    matches!(s.upd, Upd::Getting { .. })
                });
                if !going.unwrap_or(false) {
                    break;
                }
            }
        })
        .detach();
        cx.spawn(async move |this, cx| {
            let ua = format!("SwitchboardDesktop/{}", sb_core::version());
            let res = cx
                .background_executor()
                .spawn(async move {
                    let dir = download_dir().ok_or("There is no folder to download the update into.")?;
                    let file = up::download(&o, &dir, &got, &total, &ua).map_err(|e| e.to_string())?;
                    stage(&file, &dir, &o)
                })
                .await;
            let _ = this.update(cx, |s, cx| s.finish(res, then, cx));
        })
        .detach();
    }

    fn finish(&mut self, res: Result<Staged, String>, then: Then, cx: &mut Context<Self>) {
        self.upd = match (res, then) {
            (Ok(st), Then::OnQuit) => Upd::OnQuit(Box::new(st)),
            (Ok(st), Then::Restart) => match swap(&st) {
                Ok(()) => match open_app(&st.target) {
                    Ok(()) => {
                        cx.quit();
                        Upd::Idle
                    }
                    Err(e) => Upd::Failed(format!(
                        "Version {} is installed, but it did not open: {e}. Quit Switchboard and open it again.",
                        st.offer.version
                    )),
                },
                Err(e) => Upd::Failed(e),
            },
            (Err(e), _) => Upd::Failed(e),
        };
        cx.notify();
    }

    /// Undoes Install on quit (the download stays, so choosing it again is quick).
    fn cancel_on_quit(&mut self, cx: &mut Context<Self>) {
        if let Upd::OnQuit(st) = &self.upd {
            for d in &st.scratch {
                if d.file_name().is_some_and(|n| n.to_string_lossy().starts_with(".Switchboard-update")) {
                    up::remove_any(d);
                }
            }
            if st.new.extension().is_some_and(|x| x == "new") {
                up::remove_any(&st.new);
            }
            self.upd = Upd::Idle;
            cx.notify();
        }
    }
}

/// The new build has run a while, or quit cleanly.
fn healthy() {
    if let Some(state) = up::state_path() {
        up::report_healthy(&state, up::this_build());
    }
}

/// Downloads go into the cache folder, so a stopped one can resume after a restart.
fn download_dir() -> Option<PathBuf> {
    sb_core::cache::default_dir().map(|d| d.join("updates"))
}

/// What is replaced: the .app bundle on a Mac, the program elsewhere.
fn this_app() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| format!("Could not find this app: {e}"))?;
    if cfg!(target_os = "macos") {
        // .../Switchboard.app/Contents/MacOS/Switchboard
        exe.ancestors().nth(3).filter(|p| p.extension().is_some_and(|x| x == "app")).map(Path::to_path_buf).ok_or_else(
            || "This copy is not inside Switchboard.app, so it can't update itself. Download the new one from the page.".into(),
        )
    } else {
        Ok(exe)
    }
}

/// Unpacks the checked download next to the app it replaces (the same folder, so the swap is a rename), and runs
/// the new build once with --version to be sure it starts on this computer.
fn stage(file: &Path, dir: &Path, o: &Offer) -> Result<Staged, String> {
    let target = this_app()?;
    let folder = target.parent().ok_or("No folder above the app")?.to_path_buf();
    let unpack = folder.join(".Switchboard-update");
    let (new, bin) = match up::kind() {
        up::Kind::Bundle => {
            up::remove_any(&unpack);
            run("ditto", &["-x", "-k", &file.to_string_lossy(), &unpack.to_string_lossy()])?;
            let new = unpack.join("Switchboard.app");
            let bin = new.join("Contents").join("MacOS").join("Switchboard");
            (new, bin)
        }
        up::Kind::Exe => {
            up::remove_any(&unpack);
            std::fs::create_dir_all(&unpack).map_err(|e| format!("Could not unpack the update: {e}"))?;
            run(&windows_tar(), &["-xf", &file.to_string_lossy(), "-C", &unpack.to_string_lossy()])?;
            let new = unpack.join("Switchboard.exe");
            (new.clone(), new)
        }
        up::Kind::Bin => {
            // The download is the binary itself.
            let new = target.with_extension("new");
            std::fs::copy(file, &new).map_err(|e| format!("Could not unpack the update: {e}"))?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(&new, std::fs::Permissions::from_mode(0o755));
            }
            (new.clone(), new)
        }
    };
    let fail = |m: String| {
        up::remove_any(&unpack);
        if up::kind() == up::Kind::Bin {
            up::remove_any(&new);
        }
        m
    };
    if !bin.is_file() {
        return Err(fail("The download has no Switchboard app in it.".into()));
    }
    starts(&bin, &o.version).map_err(fail)?;
    Ok(Staged { new, target, offer: o.clone(), scratch: vec![unpack, dir.to_path_buf()] })
}

/// Runs `bin --version` and expects "Switchboard <version>" back within a few seconds.
fn starts(bin: &Path, version: &str) -> Result<(), String> {
    let bad = |why: &str| format!("The new version would not start on this computer ({why}), so it was not installed.");
    let mut cmd = std::process::Command::new(bin);
    // CREATE_NO_WINDOW, in case the new build is a console program.
    #[cfg(target_os = "windows")]
    std::os::windows::process::CommandExt::creation_flags(&mut cmd, 0x0800_0000);
    let mut child = cmd
        .arg("--version")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| bad(&e.to_string()))?;
    for _ in 0..100 {
        match child.try_wait() {
            Ok(Some(_)) => {
                let out = child.wait_with_output().map_err(|e| bad(&e.to_string()))?;
                let text = String::from_utf8_lossy(&out.stdout);
                return if out.status.success() && text.trim() == format!("Switchboard {version}") {
                    Ok(())
                } else {
                    Err(bad("it did not say its version"))
                };
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(100)),
            Err(e) => return Err(bad(&e.to_string())),
        }
    }
    let _ = child.kill();
    Err(bad("it did not answer"))
}

/// The swap: the new build in place, the old one kept, update.json noting it (sb_core::update::swap_in).
fn swap(st: &Staged) -> Result<(), String> {
    let state = up::state_path().ok_or("There is no settings folder to note the update in.")?;
    let from = up::this_build().unwrap_or_default();
    up::swap_in(up::kind(), &st.new, &st.target, &state, &st.offer, from)?;
    for d in &st.scratch {
        up::remove_any(d);
    }
    Ok(())
}

/// Windows' own bsdtar (Windows 10 1803 and later), which unpacks zips. A bare "tar" could find Git's GNU tar first on
/// the PATH, and that one can't read a zip.
fn windows_tar() -> String {
    match std::env::var_os("SystemRoot") {
        Some(root) => std::path::Path::new(&root).join("System32").join("tar.exe").to_string_lossy().into_owned(),
        None => "tar".into(),
    }
}

/// Windows: after an update, the build that was replaced is Switchboard.old.exe until the new one reports healthy,
/// and one that failed its first run is left as Switchboard.failed.exe, because a running exe can't be deleted.
/// Remove a failed one once it has quit, and an old one only when no install is waiting to prove itself.
#[cfg(target_os = "windows")]
pub fn sweep_old() {
    let Ok(exe) = std::env::current_exe() else { return };
    let pending = up::state_path().is_some_and(|p| up::load_state(&p).pending.is_some());
    let mut gone = vec![exe.with_extension("failed.exe")];
    if !pending {
        gone.push(exe.with_extension("old.exe"));
    }
    gone.retain(|p| p.exists());
    if gone.is_empty() {
        return;
    }
    std::thread::spawn(move || {
        for _ in 0..10 {
            gone.retain(|p| std::fs::remove_file(p).is_err() && p.exists());
            if gone.is_empty() {
                return;
            }
            std::thread::sleep(Duration::from_secs(3));
        }
    });
}

fn run(cmd: &str, args: &[&str]) -> Result<(), String> {
    let mut c = std::process::Command::new(cmd);
    c.args(args);
    // No console window flashing up for tar.exe.
    #[cfg(target_os = "windows")]
    std::os::windows::process::CommandExt::creation_flags(&mut c, 0x0800_0000);
    match c.status() {
        Ok(s) if s.success() => Ok(()),
        Ok(s) => Err(format!("{cmd} failed ({s}).")),
        Err(e) => Err(format!("Could not run {cmd}: {e}")),
    }
}

/// Opens `app` (a bundle on a Mac, a program elsewhere). Chats keep running on their machines; terminals reattach.
fn open_app(app: &Path) -> std::io::Result<()> {
    if cfg!(target_os = "macos") {
        // `open -n` after a moment, so this one has quit first.
        std::process::Command::new("sh").arg("-c").arg("sleep 1; open -n \"$0\"").arg(app).spawn().map(|_| ())
    } else {
        std::process::Command::new(app).spawn().map(|_| ())
    }
}

/// "Update Switchboard, from 0.8.4 to 0.9.0".
pub fn offer_title(o: &Offer) -> String {
    if o.from == o.version {
        format!("Update Switchboard, from {} to a newer {} build", o.from, o.version)
    } else {
        format!("Update Switchboard, from {} to {}", o.from, o.version)
    }
}

impl BoardView {
    /// The header's "Update ready" pill, when there is a newer build.
    pub fn update_chip(&self, c: &Ctx) -> Option<AnyElement> {
        c.app.update.as_ref()?;
        let t = c.t;
        let label = if matches!(c.app.upd, Upd::OnQuit(_)) { "Installs on quit" } else { "Update ready" };
        Some(
            div()
                .id("update-chip")
                .flex_none()
                .h(px(28.))
                .px(px(10.))
                .mr(px(8.))
                .flex()
                .items_center()
                .gap(px(6.))
                .rounded(px(ui::R_SHEET))
                .bg(t.accent.alpha(if t.dark { 0.16 } else { 0.11 }))
                .text_size(px(12.5))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(t.accent)
                .cursor_pointer()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(c.cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.update_open = !this.update_open;
                    cx.notify();
                }))
                .child(ui::icon("update", 13., t.accent))
                .child(label)
                .into_any_element(),
        )
    }

    /// What the update is, with Restart to update and Install on quit (or how the download is going).
    pub fn update_pop(&self, c: &Ctx) -> Option<AnyElement> {
        if !self.update_open {
            return None;
        }
        let o: Offer = c.app.update.clone()?;
        let t = c.t;
        let cx = c.cx;
        let built = o.built.map(|b| format!("Built {}. ", sb_core::fmt::md(b))).unwrap_or_default();
        let queued = matches!(c.app.upd, Upd::OnQuit(_));
        let note = if queued {
            format!("{built}It installs when you quit Switchboard. Your chats keep running.")
        } else {
            format!(
                "{built}Restart to update installs it now and opens Switchboard again. Install on quit waits until you quit. Your chats keep running."
            )
        };
        let btn = |id: &'static str, label: &'static str| {
            ui::ghost(&t, id, Some(label), None).h(px(32.)).px(px(10.)).text_size(px(13.))
        };
        let body = match &c.app.upd {
            Upd::Getting { got, total, then } => {
                let (g, n) = (got.load(Ordering::Relaxed), total.load(Ordering::Relaxed));
                let f = if n > 0 { (g as f32 / n as f32).clamp(0.02, 1.) } else { 0.1 };
                div()
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .child(div().text_size(px(13.)).text_color(t.ink2).child(match (f >= 1., then) {
                        (false, _) => "Downloading the new version",
                        (true, Then::Restart) => "Installing. Switchboard opens again in a moment.",
                        (true, Then::OnQuit) => "Checking the new version",
                    }))
                    .child(
                        div()
                            .h(px(4.))
                            .rounded_full()
                            .bg(t.track)
                            .child(div().h_full().w(relative(f)).rounded_full().bg(t.accent)),
                    )
                    .into_any_element()
            }
            st => div()
                .flex()
                .flex_col()
                .gap(px(10.))
                .children(match st {
                    Upd::Failed(e) => Some(ui::wrap(e.clone()).text_size(px(12.5)).text_color(t.bad)),
                    _ => None,
                })
                .child(
                    div()
                        .flex()
                        .justify_end()
                        .gap(px(8.))
                        .child(if queued {
                            btn("upd-cancel", "Don't install").on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.app.update(cx, |s, cx| s.cancel_on_quit(cx))
                            }))
                        } else {
                            btn("upd-later", "Later").on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.update_open = false;
                                cx.notify();
                            }))
                        })
                        .when(!queued, |d| {
                            d.child(btn("upd-quit", "Install on quit").on_click(cx.listener(
                                |this, _: &ClickEvent, _, cx| {
                                    this.app.update(cx, |s, cx| s.run_update(Then::OnQuit, cx))
                                },
                            )))
                        })
                        .child(
                            ui::primary_btn(&t, "upd-go", 32.)
                                .px(px(14.))
                                .text_size(px(13.))
                                .child(ui::icon("update", 14., t.on_accent))
                                .child("Restart to update")
                                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                    this.app.update(cx, |s, cx| s.run_update(Then::Restart, cx))
                                })),
                        ),
                )
                .into_any_element(),
        };
        Some(
            div()
                .absolute()
                .inset_0()
                .occlude()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, _, cx| {
                        this.update_open = false;
                        cx.notify();
                    }),
                )
                .child(
                    div()
                        .id("update-pop")
                        .absolute()
                        .top(px(ui::header() + 4.))
                        .right(px(200. + crate::chrome::app_menu_room()))
                        .w(px(400.))
                        .p(px(16.))
                        .rounded(px(ui::R_POP))
                        .border_1()
                        .border_color(t.line2)
                        .bg(t.panel2)
                        .shadow(t.drop())
                        .occlude()
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .flex()
                        .flex_col()
                        .gap(px(8.))
                        .child(
                            ui::wrap(offer_title(&o))
                                .text_size(px(15.))
                                .font_weight(FontWeight::BOLD)
                                .text_color(t.ink),
                        )
                        .child(ui::wrap(note).text_size(px(13.)).line_height(relative(1.45)).text_color(t.dim))
                        .child(body),
                )
                .into_any_element(),
        )
    }
}

/// Settings > Updates: what this build does about updates, in plain words.
pub fn status_line(app: &AppState) -> String {
    if let Some(off) = up::off_reason() {
        return off.to_string();
    }
    let last = match app.update_at {
        _ if app.update_busy => "Checking now.".to_string(),
        Some(t) => format!("Last checked {}.", sb_core::fmt::md(t)),
        None => "It checks a minute or two after it opens.".to_string(),
    };
    let found = match (&app.update, &app.upd) {
        (Some(o), Upd::OnQuit(_)) => format!(" Version {} installs when you quit.", o.version),
        (Some(o), _) => format!(" Version {} is ready: use Update ready in the header.", o.version),
        (None, _) if app.update_at.is_some() && !app.update_busy => " This is the newest version.".to_string(),
        _ => String::new(),
    };
    format!("Switchboard checks for a newer version once a day and asks before installing it. {last}{found}")
}
