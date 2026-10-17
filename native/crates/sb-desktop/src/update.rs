//! "Restart to update": the app notices a newer build on the download page (through the server, GET /api/app) and
//! offers it in the header. Nothing happens until the user clicks Restart to update: then it downloads the build,
//! checks its SHA-256, puts it in place of this one and opens it again. It never restarts by itself.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use gpui::{AnyElement, App, ClickEvent, Context, FontWeight, InteractiveElement, IntoElement, MouseButton, ParentElement, StatefulInteractiveElement, Styled, div, px, relative};
use sb_core::update::Offer;

use crate::state::AppState;
use crate::ui;
use crate::view::{BoardView, Ctx};

/// How often to look for a newer build.
const EVERY: f64 = 6. * 3600.;

#[derive(Clone, Default)]
pub enum Upd {
    #[default]
    Idle,
    Getting { got: Arc<AtomicU64>, total: Arc<AtomicU64> },
    Failed(String),
}

impl AppState {
    /// Looks for a newer build now and then (the first look a little after start).
    pub fn check_update(&mut self, cx: &mut Context<Self>) {
        let now = AppState::now();
        if sb_core::update::this_build().is_none() || now - self.update_at < EVERY {
            return;
        }
        self.update_at = now;
        let core = self.core.clone();
        cx.spawn(async move |this, cx| {
            let res = cx.background_executor().spawn(async move { core.update_offer() }).await;
            let _ = this.update(cx, |s, cx| {
                match res {
                    Ok(o) => s.update = o,
                    Err(e) => log::info!("update check: {e}"),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Restart to update: download, check, install, open the new one, quit this one.
    pub fn run_update(&mut self, cx: &mut Context<Self>) {
        let Some(o) = self.update.clone() else { return };
        if matches!(self.upd, Upd::Getting { .. }) {
            return;
        }
        let (got, total) = (Arc::new(AtomicU64::new(0)), Arc::new(AtomicU64::new(0)));
        self.upd = Upd::Getting { got: got.clone(), total: total.clone() };
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
                    let dir = std::env::temp_dir().join(format!("switchboard-update-{}", o.build));
                    let file = sb_core::update::download(&o, &dir, &got, &total, &ua).map_err(|e| e.to_string())?;
                    install(&file, &dir)
                })
                .await;
            let _ = this.update(cx, |s, cx| {
                match res {
                    Ok(next) => {
                        s.upd = Upd::Idle;
                        relaunch(&next, cx);
                    }
                    Err(e) => s.upd = Upd::Failed(e),
                }
                cx.notify();
            });
        })
        .detach();
    }
}

/// Puts the downloaded build in place of this one. Returns what to open afterwards.
fn install(file: &Path, dir: &Path) -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| format!("Could not find this app: {e}"))?;
    if cfg!(target_os = "macos") {
        // .../Switchboard.app/Contents/MacOS/Switchboard; the zip holds Switchboard.app.
        let app = exe.ancestors().nth(3).filter(|p| p.extension().is_some_and(|x| x == "app")).ok_or("This copy is not inside Switchboard.app, so it can't update itself. Download the new one from the page.")?;
        let parent = app.parent().ok_or("No folder above the app")?;
        let out = dir.join("unpacked");
        let _ = std::fs::remove_dir_all(&out);
        run("ditto", &["-x", "-k", &file.to_string_lossy(), &out.to_string_lossy()])?;
        let new = out.join("Switchboard.app");
        if !new.is_dir() {
            return Err("The download has no Switchboard.app in it.".into());
        }
        let old = parent.join(".Switchboard-old.app");
        let _ = std::fs::remove_dir_all(&old);
        std::fs::rename(app, &old).map_err(|e| format!("Could not move the old app aside: {e}"))?;
        if let Err(e) = run("ditto", &[&new.to_string_lossy(), &app.to_string_lossy()]) {
            let _ = std::fs::rename(&old, app); // put it back
            return Err(e);
        }
        let _ = std::fs::remove_dir_all(&old);
        let _ = std::fs::remove_dir_all(dir);
        Ok(app.to_path_buf())
    } else if cfg!(target_os = "windows") {
        // A running exe can be renamed, not overwritten: move it aside, unpack the new one next to it.
        let folder = exe.parent().ok_or("No folder above the app")?;
        let old = exe.with_extension("old.exe");
        let _ = std::fs::remove_file(&old);
        std::fs::rename(&exe, &old).map_err(|e| format!("Could not move the old app aside: {e}"))?;
        if let Err(e) = run(&windows_tar(), &["-xf", &file.to_string_lossy(), "-C", &folder.to_string_lossy()]) {
            let _ = std::fs::rename(&old, &exe);
            return Err(e);
        }
        Ok(exe)
    } else {
        // Linux: the download is the binary itself. A rename over a running binary is fine.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(file, std::fs::Permissions::from_mode(0o755));
        }
        let next = exe.with_extension("new");
        std::fs::copy(file, &next).map_err(|e| format!("Could not put the new app in place: {e}"))?;
        std::fs::rename(&next, &exe).map_err(|e| format!("Could not put the new app in place: {e}"))?;
        let _ = std::fs::remove_dir_all(dir);
        Ok(exe)
    }
}

/// Windows' own bsdtar (Windows 10 1803 and later), which unpacks zips. A bare "tar" could find Git's GNU tar first on
/// the PATH, and that one can't read a zip.
fn windows_tar() -> String {
    match std::env::var_os("SystemRoot") {
        Some(root) => std::path::Path::new(&root).join("System32").join("tar.exe").to_string_lossy().into_owned(),
        None => "tar".into(),
    }
}

/// Windows: the build that Restart to update replaced is left next to this one as Switchboard.old.exe, because a
/// running exe can't be deleted. Remove it once that one has quit.
#[cfg(target_os = "windows")]
pub fn sweep_old() {
    let Ok(exe) = std::env::current_exe() else { return };
    let old = exe.with_extension("old.exe");
    if !old.exists() {
        return;
    }
    std::thread::spawn(move || {
        for _ in 0..10 {
            if std::fs::remove_file(&old).is_ok() || !old.exists() {
                return;
            }
            std::thread::sleep(Duration::from_secs(3));
        }
    });
}

fn run(cmd: &str, args: &[&str]) -> Result<(), String> {
    match std::process::Command::new(cmd).args(args).status() {
        Ok(s) if s.success() => Ok(()),
        Ok(s) => Err(format!("{cmd} failed ({s}).")),
        Err(e) => Err(format!("Could not run {cmd}: {e}")),
    }
}

/// Opens the new build and quits this one. Chats keep running on their machines; terminals reattach.
fn relaunch(next: &Path, cx: &mut App) {
    let started = if cfg!(target_os = "macos") {
        // `open -n` after a moment, so this one has quit first.
        std::process::Command::new("sh").arg("-c").arg(format!("sleep 1; open -n '{}'", next.display())).spawn()
    } else {
        std::process::Command::new(next).spawn()
    };
    match started {
        Ok(_) => cx.quit(),
        Err(e) => log::error!("could not open the new build: {e}"),
    }
}

impl BoardView {
    /// The header's "Update ready" pill, when there is a newer build.
    pub fn update_chip(&self, c: &Ctx) -> Option<AnyElement> {
        c.app.update.as_ref()?;
        let t = c.t;
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
                .child("Update ready")
                .into_any_element(),
        )
    }

    /// What the update is, and Restart to update (or how it is going).
    pub fn update_pop(&self, c: &Ctx) -> Option<AnyElement> {
        if !self.update_open {
            return None;
        }
        let o: Offer = c.app.update.clone()?;
        let t = c.t;
        let cx = c.cx;
        let built = o.built.map(|b| format!(", built {}", sb_core::fmt::md(b))).unwrap_or_default();
        let body = match &c.app.upd {
            Upd::Getting { got, total } => {
                let (g, n) = (got.load(Ordering::Relaxed), total.load(Ordering::Relaxed));
                let f = if n > 0 { (g as f32 / n as f32).clamp(0.02, 1.) } else { 0.1 };
                div()
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .child(div().text_size(px(13.)).text_color(t.ink2).child(if f >= 1. { "Installing. Switchboard opens again in a moment." } else { "Downloading the new build" }))
                    .child(div().h(px(4.)).rounded_full().bg(t.track).child(div().h_full().w(relative(f)).rounded_full().bg(t.accent)))
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
                        .child(ui::ghost(&t, "upd-later", Some("Later"), None).h(px(32.)).px(px(10.)).text_size(px(13.)).on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.update_open = false;
                            cx.notify();
                        })))
                        .child(ui::primary_btn(&t, "upd-go", 32.).px(px(14.)).text_size(px(13.)).child(ui::icon("update", 14., t.on_accent)).child("Restart to update").on_click(cx.listener(
                            |this, _: &ClickEvent, _, cx| this.app.update(cx, |s, cx| s.run_update(cx)),
                        ))),
                )
                .into_any_element(),
        };
        Some(
            div()
                .absolute()
                .inset_0()
                .occlude()
                .on_mouse_down(MouseButton::Left, cx.listener(|this, _, _, cx| {
                    this.update_open = false;
                    cx.notify();
                }))
                .child(
                    div()
                        .id("update-pop")
                        .absolute()
                        .top(px(ui::header() + 4.))
                        .right(px(200. + crate::chrome::app_menu_room()))
                        .w(px(340.))
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
                        .child(div().text_size(px(15.)).font_weight(FontWeight::BOLD).text_color(t.ink).child("A newer Switchboard is ready"))
                        .child(ui::wrap(format!("Version {}{built}. Restart to update installs it and opens Switchboard again. Your chats keep running.", o.version)).text_size(px(13.)).line_height(relative(1.45)).text_color(t.dim))
                        .child(body),
                )
                .into_any_element(),
        )
    }
}
