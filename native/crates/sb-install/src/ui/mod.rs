//! The installer's window: welcome, a short tour, options, progress, done; and the uninstaller's confirm, progress and
//! done. The flows themselves (flow.rs) run on a worker thread; this only draws what they report.

mod assets;
mod screens;
mod theme;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc;
use std::time::Duration;

use gpui::{
    App, AppContext, Context, KeyBinding, Menu, MenuItem, TitlebarOptions, Window, WindowAppearance,
    WindowBackgroundAppearance, WindowBounds, WindowDecorations, WindowOptions, actions, point, px, size,
};

use crate::Args;
use crate::flow::{self, Event, Outcome, Setup, Task};
use crate::platform::{self, Dir, Platform};
use crate::record::Choices;
use crate::uninstall;
use theme::Theme;

actions!(installer, [Quit]);

pub const MAC: bool = cfg!(target_os = "macos");

/// Screenshots draw no hover, wherever the pointer happens to rest.
pub fn still() -> bool {
    #[cfg(feature = "shots")]
    return std::env::var_os("SB_SHOT").is_some();
    #[cfg(not(feature = "shots"))]
    false
}
const WIDTH: f32 = 800.;
const HEIGHT: f32 = 600.;

#[derive(Clone, Debug, PartialEq)]
pub enum Screen {
    Welcome,
    Tour(usize),
    Options,
    Working,
    Done,
    Uninstall,
    Error(String),
}

impl Screen {
    /// The tall header (welcome and done) or the short one.
    fn tall(&self) -> bool {
        matches!(self, Screen::Welcome | Screen::Done)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowState {
    Waiting,
    Active,
    Done,
}

#[derive(Clone, Debug)]
pub struct Row {
    pub task: Task,
    pub state: RowState,
    pub detail: String,
}

enum Msg {
    Event(Event),
    Finished(Result<Outcome, String>),
}

pub struct Installer {
    p: Arc<dyn Platform>,
    pub setup: Result<Setup, String>,
    /// The installed copy, for Uninstall: its folder, bundle and version.
    pub installed: Option<(Dir, PathBuf, Option<String>)>,
    pub screen: Screen,
    /// The header's height before the last screen change, so it can glide to the new one.
    pub header_from: f32,
    /// Bumped on every screen change: the entrance animations key on it, so each screen animates in once.
    pub nav: usize,
    pub choices: Choices,
    pub remove_data: bool,
    pub uninstalling: bool,
    /// Opened with --uninstall: Cancel closes the window instead of going back to Welcome.
    pub uninstall_only: bool,
    pub rows: Vec<Row>,
    pub stuck: bool,
    pub notes: Vec<String>,
    pub open_after: bool,
    pub eject_after: bool,
    force_tx: Option<mpsc::Sender<bool>>,
    /// The theme from SB_THEME (screenshots); otherwise it follows the system.
    pub theme_override: Option<bool>,
}

/// The platform the window is using, so whatever happens it gets its cleanup once the window is gone.
static PLATFORM: std::sync::OnceLock<Arc<dyn Platform>> = std::sync::OnceLock::new();

/// Says why the window cannot open: a message box on Windows (there is no console there), stderr elsewhere.
fn cannot_open(why: &str) {
    eprintln!("switchboard-install: {why}");
    #[cfg(windows)]
    platform::windows::alert(&format!("{} installer", crate::brand::NAME), why);
}

pub fn run(args: Args) {
    // GPUI draws with DirectX 11 on Windows and stops before any window when there is none: say so plainly instead.
    #[cfg(windows)]
    if std::env::var_os("SB_SHOT").is_none() && platform::windows::graphics() == platform::windows::Graphics::None {
        let why = platform::windows::Graphics::None.warning().unwrap_or_default();
        cannot_open(&format!("{why}\n\nThe installer needs it too. To install without a window, run it with --quiet."));
        std::process::exit(1);
    }
    let uninstall = args.uninstall;
    gpui_platform::application().with_assets(assets::Assets).run(move |cx: &mut App| {
        assets::load_fonts(cx);
        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.bind_keys([KeyBinding::new(if MAC { "cmd-q" } else { "ctrl-q" }, Quit, None)]);
        let name = crate::brand::NAME;
        cx.set_menus(vec![Menu::new(format!("Install {name}")).items([MenuItem::action("Quit", Quit)])]);

        let bounds = gpui::Bounds::centered(None, size(px(WIDTH), px(HEIGHT)), cx);
        let opts = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions {
                title: Some(if uninstall { format!("Uninstall {name}") } else { format!("Install {name}") }.into()),
                appears_transparent: MAC,
                traffic_light_position: MAC.then(|| point(px(16.), px(16.))),
            }),
            is_resizable: false,
            is_minimizable: true,
            app_id: Some("switchboard-install".into()),
            window_background: WindowBackgroundAppearance::Opaque,
            window_decorations: Some(WindowDecorations::Server),
            ..Default::default()
        };
        let args = Arc::new(args);
        match cx.open_window(opts, move |window, cx| cx.new(|cx| Installer::new(&args, window, cx))) {
            Ok(_) => {
                cx.on_window_closed(|cx, _| cx.quit()).detach();
            }
            Err(e) => {
                cannot_open(&format!("Could not open the installer's window ({e:#}). Run it with --quiet instead."));
                cx.quit();
            }
        }
        cx.activate(true);
    });
    if let Some(p) = PLATFORM.get() {
        p.cleanup();
    }
}

impl Installer {
    fn new(args: &Args, window: &mut Window, cx: &mut Context<Self>) -> Installer {
        window.observe_window_appearance(|window, _| window.refresh()).detach();
        let theme_override = std::env::var("SB_THEME").ok().map(|t| t == "dark");
        #[cfg(feature = "shots")]
        if let Ok(shot) = std::env::var("SB_SHOT") {
            let mut v = Installer::sample(&shot, theme_override);
            v.capture_when_ready(window, cx);
            return v;
        }
        let p: Arc<dyn Platform> = Arc::from(platform::native());
        let _ = PLATFORM.set(p.clone());
        let setup = flow::prepare(p.as_ref(), &args.opts);
        let installed = flow::find_installed(p.as_ref(), args.opts.dest.as_deref()).map(|(d, app)| {
            let v = p.bundle_version(&app);
            (d, app, v)
        });
        let mut v = Installer {
            p,
            setup,
            installed,
            screen: Screen::Welcome,
            header_from: 168.,
            nav: 0,
            choices: args.opts.choices,
            remove_data: args.opts.remove_data,
            uninstalling: false,
            uninstall_only: args.uninstall,
            rows: vec![],
            stuck: false,
            notes: vec![],
            open_after: false,
            // On by default: the disk image isn't needed once the app is installed.
            eject_after: true,
            force_tx: None,
            theme_override,
        };
        if args.uninstall {
            v.screen = if v.installed.is_some() {
                Screen::Uninstall
            } else {
                Screen::Error(format!("{} is not installed, so there is nothing to uninstall.", crate::brand::NAME))
            };
            v.header_from = 76.;
        }
        let _ = cx;
        v
    }

    pub fn theme(&self, window: &Window) -> Theme {
        let dark = self
            .theme_override
            .unwrap_or_else(|| matches!(window.appearance(), WindowAppearance::Dark | WindowAppearance::VibrantDark));
        if dark { Theme::dark() } else { Theme::light() }
    }

    pub fn go(&mut self, s: Screen, cx: &mut Context<Self>) {
        self.header_from = if self.screen.tall() { 168. } else { 76. };
        self.screen = s;
        self.nav += 1;
        cx.notify();
    }

    /// Welcome's Continue: the tour on a first install, else straight on.
    pub fn after_welcome(&mut self, cx: &mut Context<Self>) {
        let first = self.setup.as_ref().is_ok_and(|s| s.installed.is_none());
        if first { self.go(Screen::Tour(0), cx) } else { self.after_tour(cx) }
    }

    /// Options when the first-run choices have not been made on this account yet, else install.
    pub fn after_tour(&mut self, cx: &mut Context<Self>) {
        let chosen = self.setup.as_ref().is_ok_and(|s| s.chosen_before());
        if chosen { self.start_install(cx) } else { self.go(Screen::Options, cx) }
    }

    pub fn start_install(&mut self, cx: &mut Context<Self>) {
        let Ok(setup) = self.setup.clone() else { return };
        let mut rows = vec![];
        if setup.running {
            rows.push(Task::Quit);
        }
        rows.extend([Task::Copy, Task::Verify, Task::Swap]);
        if self.p.registers() {
            rows.push(Task::Register);
        }
        if !setup.chosen_before() {
            rows.push(Task::Choices);
        }
        self.rows =
            rows.into_iter().map(|task| Row { task, state: RowState::Waiting, detail: String::new() }).collect();
        self.uninstalling = false;
        let (p, choices) = (self.p.clone(), self.choices);
        self.spawn(cx, move |ask, ev| flow::install(p.as_ref(), &setup, choices, ask, ev));
    }

    pub fn uninstall_plan(&self) -> Option<uninstall::Plan> {
        let (_, app, _) = self.installed.as_ref()?;
        Some(flow::uninstall_plan(self.p.as_ref(), app, self.remove_data))
    }

    /// The lists the confirm screen shows (screenshots: the sample's full lists).
    pub fn shown_plan(&self) -> Option<uninstall::Plan> {
        #[cfg(feature = "shots")]
        if std::env::var_os("SB_SHOT").is_some() {
            return self.sample_plan();
        }
        self.uninstall_plan()
    }

    pub fn start_uninstall(&mut self, cx: &mut Context<Self>) {
        let Some(plan) = self.uninstall_plan() else { return };
        let mut rows = vec![];
        if let Some((_, app, _)) = &self.installed
            && !self.p.running(app).is_empty()
        {
            rows.push(Task::Quit);
        }
        for i in &plan.remove {
            let t = Task::Remove(i.what);
            if !rows.contains(&t) {
                rows.push(t);
            }
        }
        // The flow removes the login item, the Dock icon and the shortcuts before the app; list them in that order too.
        rows.sort_by_key(|t| match t {
            Task::Quit => 0,
            Task::Remove(uninstall::What::LoginItem) => 1,
            Task::Remove(uninstall::What::Dock) => 2,
            Task::Remove(uninstall::What::Shortcuts) => 3,
            Task::Remove(uninstall::What::App) => 4,
            Task::Remove(uninstall::What::Registry) => 5,
            Task::Remove(uninstall::What::Caches) => 6,
            _ => 7,
        });
        self.rows =
            rows.into_iter().map(|task| Row { task, state: RowState::Waiting, detail: String::new() }).collect();
        self.uninstalling = true;
        let p = self.p.clone();
        self.spawn(cx, move |ask, ev| flow::uninstall(p.as_ref(), &plan, ask, ev));
    }

    /// Runs a flow on a worker thread and feeds its events to the screen.
    fn spawn(
        &mut self,
        cx: &mut Context<Self>,
        work: impl FnOnce(&mut dyn FnMut() -> bool, &mut dyn FnMut(Event)) -> Result<Outcome, String> + Send + 'static,
    ) {
        let (tx, rx) = mpsc::channel::<Msg>();
        let (force_tx, force_rx) = mpsc::channel::<bool>();
        self.force_tx = Some(force_tx);
        self.stuck = false;
        self.notes.clear();
        std::thread::spawn(move || {
            let ev_tx = tx.clone();
            let mut ev = move |e: Event| {
                let _ = ev_tx.send(Msg::Event(e));
            };
            let mut ask = move || force_rx.recv().unwrap_or(false);
            let res = work(&mut ask, &mut ev);
            let _ = tx.send(Msg::Finished(res));
        });
        self.go(Screen::Working, cx);
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_millis(60)).await;
                let mut finished = None;
                while let Ok(m) = rx.try_recv() {
                    match m {
                        Msg::Event(e) => {
                            let _ = this.update(cx, |v, cx| v.on_event(e, cx));
                        }
                        Msg::Finished(r) => finished = Some(r),
                    }
                }
                let Some(res) = finished else { continue };
                // A beat on the finished list before moving on, so the last check is seen.
                cx.background_executor().timer(Duration::from_millis(650)).await;
                let _ = this.update(cx, |v, cx| v.on_finished(res, cx));
                break;
            }
        })
        .detach();
    }

    fn on_event(&mut self, e: Event, cx: &mut Context<Self>) {
        match e {
            Event::Start(t) => {
                if !self.rows.iter().any(|r| r.task == t) {
                    self.rows.insert(0, Row { task: t, state: RowState::Waiting, detail: String::new() });
                }
                for r in self.rows.iter_mut().filter(|r| r.task == t) {
                    r.state = RowState::Active;
                }
            }
            Event::Done(t, d) => {
                for r in self.rows.iter_mut().filter(|r| r.task == t) {
                    r.state = RowState::Done;
                    r.detail = d.clone();
                }
                self.stuck = false;
            }
            Event::Stuck => self.stuck = true,
        }
        cx.notify();
    }

    fn on_finished(&mut self, res: Result<Outcome, String>, cx: &mut Context<Self>) {
        self.force_tx = None;
        self.stuck = false;
        match res {
            Ok(out) => {
                self.notes = out.notes;
                for r in &mut self.rows {
                    r.state = RowState::Done;
                }
                self.go(Screen::Done, cx);
            }
            Err(e) => self.go(Screen::Error(e), cx),
        }
    }

    /// The answer to "stop it now?" while a copy will not quit.
    pub fn answer_force(&mut self, yes: bool, cx: &mut Context<Self>) {
        if let Some(tx) = &self.force_tx {
            let _ = tx.send(yes);
        }
        self.stuck = false;
        cx.notify();
    }

    /// Done: opens the app and ejects the disk image if asked, then quits.
    pub fn finish(&mut self, cx: &mut Context<Self>) {
        if !self.uninstalling
            && let Ok(s) = &self.setup
        {
            if self.open_after
                && let Err(e) = self.p.open_app(&s.dest)
            {
                log::warn!("could not open {}: {e}", s.dest.display());
            }
            if self.eject_after
                && let Some(img) = &s.disk_image
                && let Err(e) = self.p.eject_after_exit(img)
            {
                log::warn!("could not eject {}: {e}", img.mount.display());
            }
        }
        self.p.cleanup();
        cx.quit();
    }
}

/// Sample states for the screenshots (SB_SHOT), with nothing installed or changed.
#[cfg(feature = "shots")]
impl Installer {
    fn sample(shot: &str, theme_override: Option<bool>) -> Installer {
        use crate::version::{Kind, Version};
        let v = |s: &str| Version::parse(s).unwrap();
        // "-win" shots answer like Windows: the per-user Programs folder, the Start menu, Settings > Apps.
        let win = shot.contains("win");
        let dir = if win {
            Dir { path: r"C:\Users\you\AppData\Local\Programs".into(), label: "Programs in your user folder".into() }
        } else {
            Dir { path: "/Applications".into(), label: "Applications".into() }
        };
        let mut setup = Setup {
            payload: "/Volumes/Switchboard/Switchboard.app".into(),
            version: v("0.9.0"),
            dir: dir.clone(),
            dest: if win {
                r"C:\Users\you\AppData\Local\Programs\Switchboard".into()
            } else {
                "/Applications/Switchboard.app".into()
            },
            installed: None,
            kind: Kind::Fresh,
            record: None,
            disk_image: (!win).then(|| platform::DiskImage {
                mount: "/Volumes/Switchboard".into(),
                image: Some("/Users/you/Downloads/Switchboard-0.9.0.dmg".into()),
            }),
            running: false,
            why_dir: None,
            warnings: if shot.contains("slowgpu") {
                platform::windows::Graphics::SoftwareOnly.warning().into_iter().collect()
            } else {
                vec![]
            },
        };
        let update = shot.contains("update");
        if update || shot.contains("downgrade") || shot.starts_with("uninstall") {
            let from = if shot.contains("downgrade") { "0.9.1" } else { "0.8.4" };
            setup.installed = Some(Some(v(from)));
            setup.kind = if shot.contains("downgrade") {
                Kind::Downgrade { installed: v(from) }
            } else {
                Kind::Update { from: v(from) }
            };
            setup.running = shot.contains("running");
            setup.record = Some(crate::record::Record { choices: Choices::default(), version: "0.8.4".into(), at: 0 });
        }
        if shot.contains("home") {
            setup.dir =
                Dir { path: "/Users/you/Applications".into(), label: "Applications in your home folder".into() };
            setup.dest = "/Users/you/Applications/Switchboard.app".into();
            setup.why_dir = Some("Applications needs an admin password, which this installer never asks for".into());
        }
        let installed = setup.installed.is_some().then(|| (dir, setup.dest.clone(), Some("0.8.4".to_string())));
        let mut i = Installer {
            p: Arc::new(Sample { win }),
            setup: Ok(setup),
            installed,
            screen: Screen::Welcome,
            header_from: 168.,
            nav: 0,
            choices: Choices::default(),
            remove_data: shot.contains("data"),
            uninstalling: shot.starts_with("uninstall"),
            uninstall_only: false,
            rows: vec![],
            stuck: false,
            notes: vec![],
            open_after: false,
            eject_after: false,
            force_tx: None,
            theme_override,
        };
        let row = |task, state, detail: &str| Row { task, state, detail: detail.into() };
        let base = shot.split('-').next().unwrap_or(shot);
        i.screen = match base {
            "tour" => Screen::Tour(shot.trim_start_matches("tour").trim_start_matches('-').parse().unwrap_or(1) - 1),
            "options" => Screen::Options,
            "progress" => {
                i.rows = vec![
                    row(Task::Copy, RowState::Done, ""),
                    row(Task::Verify, RowState::Done, ""),
                    row(Task::Swap, RowState::Active, ""),
                    row(Task::Choices, RowState::Waiting, ""),
                ];
                if win {
                    i.rows.insert(3, row(Task::Register, RowState::Waiting, ""));
                }
                if shot.contains("stuck") {
                    i.rows.insert(0, row(Task::Quit, RowState::Active, ""));
                    for r in i.rows.iter_mut().skip(1) {
                        r.state = RowState::Waiting;
                    }
                    i.stuck = true;
                }
                Screen::Working
            }
            "done" => Screen::Done,
            "uninstall" if shot.contains("done") => Screen::Done,
            "uninstall" => Screen::Uninstall,
            "error" => Screen::Error(
                "Could not put the new app in place (Operation not permitted). The old one was put back.".into(),
            ),
            _ => Screen::Welcome,
        };
        if base == "done" && shot.contains("notes") {
            i.notes = vec!["Skipped Dock: HOME is not this account's home folder, so the Dock was left alone.".into()];
        }
        if !i.screen.tall() {
            i.header_from = 76.;
        }
        i
    }

    /// Lets the entrance animations finish (SB_SHOT_DELAY ms, default 1500), saves the frame to SB_SHOT_OUT, quits.
    fn capture_when_ready(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(out) = std::env::var_os("SB_SHOT_OUT").map(PathBuf::from) else { return };
        let settle = std::env::var("SB_SHOT_DELAY").ok().and_then(|v| v.parse().ok()).unwrap_or(1500u64);
        cx.spawn_in(window, async move |_, cx| {
            cx.background_executor().timer(Duration::from_millis(settle)).await;
            let _ = cx.update(|window, cx| {
                if MAC {
                    window.draw(cx).clear(cx);
                }
                match window.render_to_image() {
                    Ok(img) => match img.save(&out) {
                        Ok(()) => eprintln!("sb-shot saved {} ({}x{})", out.display(), img.width(), img.height()),
                        Err(e) => eprintln!("sb-shot could not save {}: {e}", out.display()),
                    },
                    // Linux has no offscreen frame: grab the X screen (shots run alone on Xvfb).
                    Err(_) if cfg!(target_os = "linux") => {
                        let ok = std::process::Command::new("import")
                            .args(["-window", "root"])
                            .arg(&out)
                            .status()
                            .is_ok_and(|s| s.success());
                        eprintln!(
                            "sb-shot {} {} (X screen)",
                            if ok { "saved" } else { "could not save" },
                            out.display()
                        );
                    }
                    Err(e) => eprintln!("sb-shot could not render: {e:#}"),
                }
                cx.quit();
            });
        })
        .detach();
    }
}

/// The screenshots' platform: answers like a Mac with Switchboard in /Applications (or like Windows, for the "-win"
/// shots), and changes nothing.
#[cfg(feature = "shots")]
struct Sample {
    win: bool,
}

#[cfg(feature = "shots")]
impl Platform for Sample {
    fn install_dirs(&self) -> Vec<Dir> {
        vec![Dir { path: "/Applications".into(), label: "Applications".into() }]
    }
    fn choices(&self) -> &'static [crate::record::Choice] {
        use crate::record::Choice;
        if self.win { &[Choice::OpenAtLogin, Choice::DesktopShortcut] } else { &[Choice::OpenAtLogin, Choice::Dock] }
    }
    fn choice_text(&self, c: crate::record::Choice) -> (String, String) {
        if self.win { platform::windows::choice_text(c) } else { platform::mac_choice_text(c) }
    }
    fn also_adds(&self) -> Option<String> {
        self.win.then(platform::windows::also_adds)
    }
    fn settings_app(&self) -> &'static str {
        if self.win { "Settings" } else { "System Settings" }
    }
    fn registers(&self) -> bool {
        self.win
    }
    fn set_login_item(&self, _: &std::path::Path, _: bool) -> std::io::Result<platform::Applied> {
        Ok(platform::Applied::Done)
    }
    fn set_in_dock(&self, _: &std::path::Path, _: bool) -> std::io::Result<platform::Applied> {
        Ok(platform::Applied::Done)
    }
    fn open_app(&self, _: &std::path::Path) -> std::io::Result<()> {
        Ok(())
    }
    fn layout(&self, app: &std::path::Path) -> uninstall::Layout {
        use uninstall::{Item, What};
        if self.win {
            let user = PathBuf::from(r"C:\Users\you\AppData");
            return uninstall::Layout {
                app: app.into(),
                shortcuts: vec![user.join(r"Roaming\Microsoft\Windows\Start Menu\Programs\Switchboard.lnk")],
                entries: vec![
                    Item { what: What::LoginItem, path: r"HKCU\Run\Switchboard".into() },
                    Item { what: What::Registry, path: r"HKCU\Uninstall\app".into() },
                ],
                caches: vec![user.join(r"Local\Switchboard\cache")],
                data: vec![user.join(r"Roaming\Switchboard")],
                ..Default::default()
            };
        }
        let lib = PathBuf::from("/Users/you/Library");
        uninstall::Layout {
            app: app.into(),
            login_item: Some(lib.join("LaunchAgents/launcher.plist")),
            in_dock: true,
            caches: vec![lib.join("Caches/app")],
            data: vec![lib.join("Application Support/app")],
            ..Default::default()
        }
    }
}

#[cfg(feature = "shots")]
impl Installer {
    /// The sample platform says every path exists, so the uninstall lists are full.
    pub fn sample_plan(&self) -> Option<uninstall::Plan> {
        let (_, app, _) = self.installed.as_ref()?;
        Some(uninstall::plan(&self.p.layout(app), self.remove_data, |_| true))
    }
}
