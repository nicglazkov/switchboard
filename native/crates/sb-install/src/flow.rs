//! The install, update and uninstall flows, shared by the screens and --quiet. Each runs on a worker thread and
//! reports what it is doing through `Event`s; the one question it may need answered (force a copy that will not quit)
//! goes back through `ask_force`.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::brand::{NAME, bundle};
use crate::platform::{Applied, Dir, DiskImage, Platform, writable};
use crate::record::{self, Choices, Decision, Record};
use crate::swap;
use crate::uninstall;
use crate::version::{Kind, Version, kind};

/// How long a running copy gets to quit after it is asked, before the user is asked about forcing it.
pub const QUIT_WAIT: Duration = Duration::from_secs(15);

/// What the installer was told on its command line.
#[derive(Clone, Debug, Default)]
pub struct Opts {
    /// The new bundle; found next to the installer when not given.
    pub app: Option<PathBuf>,
    /// The folder to install into, instead of the usual ones.
    pub dest: Option<PathBuf>,
    pub choices: Choices,
    pub open: bool,
    pub force_quit: bool,
    pub remove_data: bool,
    pub eject: bool,
}

/// Everything known before anything changes: what is being installed, where, over what.
#[derive(Clone, Debug)]
pub struct Setup {
    pub payload: PathBuf,
    pub version: Version,
    pub dir: Dir,
    pub dest: PathBuf,
    /// The installed version: None when nothing is installed, Some(None) when its version is unreadable.
    pub installed: Option<Option<Version>>,
    pub kind: Kind,
    pub record: Option<Record>,
    pub disk_image: Option<DiskImage>,
    /// A copy is running from `dest`.
    pub running: bool,
    /// Why `dir` was picked over the first choice ("Applications needs an admin password"), if it was.
    pub why_dir: Option<String>,
}

impl Setup {
    /// The window's headline and --quiet's first line.
    pub fn title(&self) -> String {
        match &self.kind {
            Kind::Fresh => format!("Install {NAME} {}", self.version),
            Kind::Update { from } => format!("Update {NAME}, from {from} to {}", self.version),
            Kind::Reinstall => format!("Reinstall {NAME} {}", self.version),
            Kind::Unknown => format!("Update {NAME} to {}", self.version),
            Kind::Downgrade { .. } => format!("A newer {NAME} is already installed"),
        }
    }

    /// The refusal, for a downgrade.
    pub fn refusal(&self) -> Option<String> {
        let Kind::Downgrade { installed } = &self.kind else { return None };
        Some(format!(
            "{NAME} {installed} is installed in {}, which is newer than {} in this installer. Nothing was changed. \
             To go back to an older version, uninstall this one first.",
            self.dir.label, self.version
        ))
    }

    pub fn is_update(&self) -> bool {
        matches!(self.kind, Kind::Update { .. } | Kind::Unknown)
    }

    /// The first-run choices were made at an earlier install; they are not asked for again.
    pub fn chosen_before(&self) -> bool {
        self.record.is_some()
    }
}

/// The new bundle: `--app`, else `<NAME>.app` next to the installer app (on the disk image), else inside its
/// Resources folder.
pub fn find_payload(given: Option<&Path>) -> Result<PathBuf, String> {
    if let Some(p) = given {
        return if p.join("Contents").is_dir() {
            Ok(p.to_path_buf())
        } else {
            Err(format!("{} is not an app bundle.", p.display()))
        };
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    payload_near(&exe).into_iter().find(|p| p.join("Contents").is_dir()).ok_or_else(|| {
        format!("Could not find {} next to the installer. Open the installer from the disk image.", bundle())
    })
}

/// Where the app can sit relative to the installer's binary, best first.
fn payload_near(exe: &Path) -> Vec<PathBuf> {
    let mut tries = vec![];
    // <folder>/<Install X.app>/Contents/MacOS/<exe>: the folder holding the installer is four levels up.
    if let Some(folder) = exe.ancestors().nth(4) {
        tries.push(folder.join(bundle()));
    }
    if let Some(contents) = exe.ancestors().nth(2) {
        tries.push(contents.join("Resources").join(bundle()));
    }
    if let Some(dir) = exe.parent() {
        tries.push(dir.join(bundle()));
    }
    tries
}

/// The installed copy, if any: in `dest` when given, else in the usual folders, first found.
pub fn find_installed(p: &dyn Platform, dest: Option<&Path>) -> Option<(Dir, PathBuf)> {
    dirs_for(p, dest).into_iter().map(|d| (d.path.join(bundle()), d)).find(|(app, _)| app.exists()).map(|(a, d)| (d, a))
}

fn dirs_for(p: &dyn Platform, dest: Option<&Path>) -> Vec<Dir> {
    match dest {
        Some(d) => vec![Dir { path: d.to_path_buf(), label: d.display().to_string() }],
        None => p.install_dirs(),
    }
}

/// Picks the folder and works out what this install means. Changes nothing.
pub fn prepare(p: &dyn Platform, opts: &Opts) -> Result<Setup, String> {
    let payload = find_payload(opts.app.as_deref())?;
    let version = p
        .bundle_version(&payload)
        .and_then(|v| Version::parse(&v))
        .ok_or_else(|| format!("{} has no version this installer can read.", payload.display()))?;
    let (dir, why_dir) = match find_installed(p, opts.dest.as_deref()) {
        Some((dir, _)) => (dir, None),
        None => pick_dir(&dirs_for(p, opts.dest.as_deref()))?,
    };
    let dest = dir.path.join(bundle());
    let installed = dest.exists().then(|| p.bundle_version(&dest).and_then(|v| Version::parse(&v)));
    let kind = kind(installed.as_ref().map(|v| v.as_ref()), &version);
    let record = p.settings_dir().and_then(|d| record::read(&d));
    let running = installed.is_some() && !p.running(&dest).is_empty();
    Ok(Setup { payload, version, dir, dest, installed, kind, record, disk_image: p.disk_image(), running, why_dir })
}

/// The first folder this account can write to, creating a missing one in the home folder. The reason the first
/// choice was passed over comes back with it.
fn pick_dir(dirs: &[Dir]) -> Result<(Dir, Option<String>), String> {
    let mut why = None;
    for (i, d) in dirs.iter().enumerate() {
        if !d.path.exists() && (i > 0 || dirs.len() == 1) {
            let _ = std::fs::create_dir_all(&d.path);
        }
        if writable(&d.path) {
            return Ok((d.clone(), why));
        }
        if why.is_none() {
            why = Some(format!("{} needs an admin password, which this installer never asks for", d.label));
        }
    }
    Err(format!(
        "There is no folder this account can install {NAME} into ({}).",
        dirs.iter().map(|d| d.path.display().to_string()).collect::<Vec<_>>().join(", ")
    ))
}

/// A step on the progress list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Task {
    Quit,
    Copy,
    Verify,
    Swap,
    Choices,
    Remove(uninstall::What),
}

impl Task {
    pub fn label(self) -> String {
        match self {
            Task::Quit => format!("Asking {NAME} to quit"),
            Task::Copy => format!("Copying {NAME}"),
            Task::Verify => "Checking the copy".into(),
            Task::Swap => "Putting it in place".into(),
            Task::Choices => "Saving your choices".into(),
            Task::Remove(w) => format!("Removing {}", w.label().to_lowercase().replace("its ", "the ")),
        }
    }
    /// One word for --quiet's lines.
    pub fn tag(self) -> &'static str {
        match self {
            Task::Quit => "quit",
            Task::Copy => "copy",
            Task::Verify => "verify",
            Task::Swap => "swap",
            Task::Choices => "first-run",
            Task::Remove(_) => "remove",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    Start(Task),
    Done(Task, String),
    /// The running copy did not quit; the flow waits for `ask_force`.
    Stuck,
}

/// What happened, for the done screen.
#[derive(Clone, Debug, Default)]
pub struct Outcome {
    pub notes: Vec<String>,
}

/// Asks a running copy to quit and waits. Err when it is still running and the user would not have it forced.
fn quit_running(
    p: &dyn Platform,
    app: &Path,
    ask_force: &mut dyn FnMut() -> bool,
    ev: &mut dyn FnMut(Event),
) -> Result<(), String> {
    if p.running(app).is_empty() {
        return Ok(());
    }
    ev(Event::Start(Task::Quit));
    p.ask_to_quit(app);
    if !wait_gone(p, app, QUIT_WAIT) {
        ev(Event::Stuck);
        if !ask_force() {
            return Err(format!("{NAME} is still open. Quit it, then try again. Nothing was changed."));
        }
        p.force_quit(app);
        if !wait_gone(p, app, Duration::from_secs(5)) {
            return Err(format!("{NAME} could not be stopped. Nothing was changed."));
        }
        ev(Event::Done(Task::Quit, "stopped without asking".into()));
        return Ok(());
    }
    ev(Event::Done(Task::Quit, "it quit".into()));
    Ok(())
}

fn wait_gone(p: &dyn Platform, app: &Path, within: Duration) -> bool {
    let start = Instant::now();
    while start.elapsed() < within {
        if p.running(app).is_empty() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    p.running(app).is_empty()
}

/// The swap's file operations through the platform (its copy and its check).
struct PlatformFs<'a>(&'a dyn Platform);

impl swap::Fs for PlatformFs<'_> {
    fn exists(&self, p: &Path) -> bool {
        p.symlink_metadata().is_ok()
    }
    fn copy_tree(&self, from: &Path, to: &Path) -> std::io::Result<()> {
        self.0.copy_app(from, to)
    }
    fn rename(&self, from: &Path, to: &Path) -> std::io::Result<()> {
        std::fs::rename(from, to)
    }
    fn remove_tree(&self, p: &Path) -> std::io::Result<()> {
        swap::remove_tree(p)
    }
    fn verify(&self, app: &Path) -> std::io::Result<()> {
        self.0.verify_app(app)
    }
}

/// Installs or updates. `choices` are applied only when no first-run record exists yet.
pub fn install(
    p: &dyn Platform,
    s: &Setup,
    choices: Choices,
    ask_force: &mut dyn FnMut() -> bool,
    ev: &mut dyn FnMut(Event),
) -> Result<Outcome, String> {
    if let Some(no) = s.refusal() {
        return Err(no);
    }
    let mut out = Outcome::default();
    quit_running(p, &s.dest, ask_force, ev)?;

    let plan = swap::plan(&s.payload, &s.dest, s.dest.symlink_metadata().is_ok());
    let fs = PlatformFs(p);
    let mut last: Option<Task> = None;
    let mut progress = |step: swap::Step| {
        let t = match step {
            swap::Step::Copy => Task::Copy,
            swap::Step::Verify => Task::Verify,
            swap::Step::Swap => Task::Swap,
            swap::Step::Cleanup => return,
        };
        if let Some(l) = last.replace(t) {
            ev(Event::Done(l, String::new()));
        }
        ev(Event::Start(t));
    };
    let left = swap::run(&plan, &fs, &mut progress).map_err(|e| e.to_string())?;
    ev(Event::Done(Task::Swap, s.dest.display().to_string()));
    if let Some(l) = left {
        out.notes.push(format!("The old copy could not be removed. It is at {}; you can delete it.", l.display()));
    }

    let Some(settings) = p.settings_dir() else { return Ok(out) };
    let Decision::Apply(c) = record::decide(record::read(&settings), choices) else {
        return Ok(out);
    };
    ev(Event::Start(Task::Choices));
    let mut said = vec![];
    for (on, what, login) in [(c.open_at_login, "open at login", true), (c.add_to_dock, "Dock", false)] {
        if !on {
            said.push(format!("{what} off"));
            continue;
        }
        let res = if login { p.set_login_item(&s.dest, true) } else { p.set_in_dock(&s.dest, true) };
        match res {
            Ok(Applied::Done) => said.push(format!("{what} on")),
            Ok(Applied::Skipped(why)) => {
                said.push(format!("{what} skipped"));
                out.notes.push(format!("Skipped {what}: {why}."));
            }
            Err(e) => {
                said.push(format!("{what} failed"));
                out.notes.push(format!("Could not set {what}: {e}."));
            }
        }
    }
    // Saved even when every choice is off, so a later install does not ask again.
    if let Err(e) = record::save(&settings, c, &s.version.to_string(), record::now()) {
        out.notes.push(format!("Could not save your choices in {}: {e}.", settings.display()));
    }
    ev(Event::Done(Task::Choices, said.join(", ")));
    Ok(out)
}

/// The uninstall plan for the installed copy.
pub fn uninstall_plan(p: &dyn Platform, app: &Path, remove_data: bool) -> uninstall::Plan {
    uninstall::plan(&p.layout(app), remove_data, |q| q.symlink_metadata().is_ok())
}

/// Removes what the plan lists, after the app has quit. Stops at the first failure and says what is left.
pub fn uninstall(
    p: &dyn Platform,
    plan: &uninstall::Plan,
    ask_force: &mut dyn FnMut() -> bool,
    ev: &mut dyn FnMut(Event),
) -> Result<Outcome, String> {
    use uninstall::What;
    let app = plan.remove.iter().find(|i| i.what == What::App).map(|i| i.path.clone()).unwrap_or_default();
    quit_running(p, &app, ask_force, ev)?;
    let mut out = Outcome::default();
    // The login item and the Dock icon first, so nothing points at a missing app if a later step fails.
    let order = [What::LoginItem, What::Dock, What::App, What::Caches, What::Data];
    for what in order {
        let items: Vec<_> = plan.remove.iter().filter(|i| i.what == what).collect();
        if items.is_empty() {
            continue;
        }
        let t = Task::Remove(what);
        ev(Event::Start(t));
        for i in &items {
            let res = match what {
                What::LoginItem => p.set_login_item(&app, false),
                What::Dock => p.set_in_dock(&app, false),
                _ => swap::remove_tree(&i.path).map(|_| Applied::Done),
            };
            match res {
                Ok(Applied::Done) => {}
                Ok(Applied::Skipped(why)) => out.notes.push(format!("Left {}: {why}.", what.label().to_lowercase())),
                Err(e) => return Err(format!("Could not remove {}: {e}.", i.path.display())),
            }
        }
        let detail = items.iter().map(|i| i.path.display().to_string()).collect::<Vec<_>>().join(", ");
        ev(Event::Done(t, detail));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::uninstall::Layout;
    use std::io;

    /// A platform whose folders are under a temp root.
    struct Test {
        root: PathBuf,
        locked_system: bool,
    }

    impl Platform for Test {
        fn install_dirs(&self) -> Vec<Dir> {
            vec![
                Dir { path: self.root.join("Applications"), label: "Applications".into() },
                Dir { path: self.root.join("home/Applications"), label: "Applications in your home folder".into() },
            ]
        }
        fn set_login_item(&self, _: &Path, _: bool) -> io::Result<Applied> {
            Ok(Applied::Done)
        }
        fn set_in_dock(&self, _: &Path, _: bool) -> io::Result<Applied> {
            Ok(Applied::Skipped("no Dock here".into()))
        }
        fn open_app(&self, _: &Path) -> io::Result<()> {
            Ok(())
        }
        fn settings_dir(&self) -> Option<PathBuf> {
            Some(self.root.join("home/settings"))
        }
        fn layout(&self, app: &Path) -> Layout {
            Layout {
                app: app.into(),
                login_item: None,
                in_dock: false,
                caches: vec![self.root.join("home/cache")],
                data: vec![self.root.join("home/settings")],
            }
        }
    }

    fn app(at: &Path, version: &str) {
        std::fs::create_dir_all(at.join("Contents")).unwrap();
        std::fs::write(
            at.join("Contents/Info.plist"),
            format!(
                r#"<?xml version="1.0" encoding="UTF-8"?><plist version="1.0"><dict><key>CFBundleShortVersionString</key><string>{version}</string></dict></plist>"#
            ),
        )
        .unwrap();
    }

    fn setup(tag: &str, locked_system: bool) -> Test {
        let root = std::env::temp_dir().join(format!("sb-install-flow-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("Applications")).unwrap();
        if locked_system {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(root.join("Applications"), std::fs::Permissions::from_mode(0o555)).unwrap();
            }
        }
        Test { root, locked_system }
    }

    impl Drop for Test {
        fn drop(&mut self) {
            #[cfg(unix)]
            if self.locked_system {
                use std::os::unix::fs::PermissionsExt;
                let _ =
                    std::fs::set_permissions(self.root.join("Applications"), std::fs::Permissions::from_mode(0o755));
            }
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    fn opts(t: &Test, v: &str) -> Opts {
        let payload = t.root.join(format!("dmg-{v}/Switchboard.app"));
        app(&payload, v);
        Opts { app: Some(payload), ..Default::default() }
    }

    #[test]
    fn fresh_install_then_update_then_refused_downgrade() {
        let t = setup("cycle", false);
        let s = prepare(&t, &opts(&t, "0.8.3")).unwrap();
        assert_eq!(s.kind, Kind::Fresh);
        assert_eq!(s.dir.label, "Applications");
        assert_eq!(s.title(), "Install Switchboard 0.8.3");
        let asked = Choices { open_at_login: true, add_to_dock: true };
        let mut events = vec![];
        let out = install(&t, &s, asked, &mut || false, &mut |e| events.push(e)).unwrap();
        assert!(events.contains(&Event::Done(Task::Choices, "open at login on, Dock skipped".into())), "{events:?}");
        assert_eq!(out.notes, ["Skipped Dock: no Dock here."]);
        assert_eq!(record::read(&t.root.join("home/settings")).unwrap().choices, asked);

        let s = prepare(&t, &opts(&t, "0.9.0")).unwrap();
        assert_eq!(s.title(), "Update Switchboard, from 0.8.3 to 0.9.0");
        assert!(s.chosen_before());
        let mut events = vec![];
        install(&t, &s, Choices::default(), &mut || false, &mut |e| events.push(e)).unwrap();
        assert!(!events.iter().any(|e| matches!(e, Event::Start(Task::Choices))), "first-run choices applied twice");
        assert_eq!(crate::platform::info_plist_version(&s.dest).as_deref(), Some("0.9.0"));

        let s = prepare(&t, &opts(&t, "0.8.4")).unwrap();
        assert_eq!(s.kind, Kind::Downgrade { installed: Version::parse("0.9.0").unwrap() });
        let no = install(&t, &s, Choices::default(), &mut || false, &mut |_| {}).unwrap_err();
        assert!(no.contains("0.9.0 is installed") && no.contains("newer than 0.8.4"), "{no}");
        assert_eq!(crate::platform::info_plist_version(&s.dest).as_deref(), Some("0.9.0"), "unchanged");
    }

    #[cfg(unix)]
    #[test]
    fn falls_back_to_the_home_folder_and_says_why() {
        let t = setup("locked", true);
        if writable(&t.root.join("Applications")) {
            return; // running as root: every folder is writable
        }
        let s = prepare(&t, &opts(&t, "1.0.0")).unwrap();
        assert_eq!(s.dir.label, "Applications in your home folder");
        assert!(s.why_dir.as_deref().unwrap().contains("needs an admin password"));
    }

    #[test]
    fn uninstall_keeps_data_unless_asked() {
        let t = setup("uninstall", false);
        let s = prepare(&t, &opts(&t, "1.0.0")).unwrap();
        install(&t, &s, Choices::default(), &mut || false, &mut |_| {}).unwrap();
        std::fs::create_dir_all(t.root.join("home/cache")).unwrap();

        let plan = uninstall_plan(&t, &s.dest, false);
        uninstall(&t, &plan, &mut || false, &mut |_| {}).unwrap();
        assert!(!s.dest.exists() && !t.root.join("home/cache").exists());
        assert!(t.root.join("home/settings/first-run.json").exists(), "data kept");

        app(&s.dest, "1.0.0");
        let plan = uninstall_plan(&t, &s.dest, true);
        uninstall(&t, &plan, &mut || false, &mut |_| {}).unwrap();
        assert!(!t.root.join("home/settings").exists());
    }

    #[test]
    fn finds_the_app_next_to_the_installer() {
        let tries = payload_near(Path::new("/Volumes/S/Install S.app/Contents/MacOS/switchboard-install"));
        assert_eq!(tries[0], Path::new("/Volumes/S").join(bundle()), "the disk image's root comes first");
        assert_eq!(tries[1], Path::new("/Volumes/S/Install S.app/Contents/Resources").join(bundle()));
    }

    #[test]
    fn missing_payload_says_where_to_open_from() {
        let e = find_payload(Some(Path::new("/nowhere/Switchboard.app"))).unwrap_err();
        assert!(e.contains("not an app bundle"));
    }
}
