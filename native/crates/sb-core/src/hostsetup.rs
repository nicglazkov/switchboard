//! Making this computer the Switchboard server, from the empty board ("Run the server on this computer"). What the
//! guided setup checks first (is it a laptop, is Python 3 here, are the ports free, is there a server already), the
//! server kit the app carries (`Contents/Resources/server-kit` on a Mac: `server/`, `agent/sb.py`,
//! `scripts/install/` and the hub, laid out as the repository has them, so `install-server.sh` finds them), the
//! installer's command line, and its progress read from the `== <step>` lines it prints when its output is not a
//! terminal. The parsers and the decisions are plain functions with tests; only the `detect_*` and `find_*`
//! functions look at the machine.

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::firstrun::Found;

/// The server (8130), the downloads for Add a machine (8131) and the hub (8132).
pub const PORTS: [u16; 3] = [8130, 8131, 8132];

/// Where the app connects once the server runs here.
pub const LOCAL_URL: &str = "http://localhost:8130";

/// The oldest Python the server runs on.
pub const PYTHON_MIN: (u32, u32) = (3, 9);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Os {
    Mac,
    Windows,
    Linux,
}

impl Os {
    pub fn this() -> Os {
        if cfg!(target_os = "macos") {
            Os::Mac
        } else if cfg!(windows) {
            Os::Windows
        } else {
            Os::Linux
        }
    }

    /// "this Mac", "this PC", "this computer".
    pub fn this_one(self) -> &'static str {
        match self {
            Os::Mac => "this Mac",
            Os::Windows => "this PC",
            Os::Linux => "this computer",
        }
    }

    /// The installer's name for it (install-server.sh's `$OSN`).
    fn short(self) -> &'static str {
        match self {
            Os::Mac => "mac",
            Os::Windows => "windows",
            Os::Linux => "linux",
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Is it a laptop
// ---------------------------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Power {
    /// It has a built-in battery or a laptop's model name: why we think so.
    Laptop(String),
    Desktop,
    /// Could not tell.
    Unknown,
}

/// What Apple's laptop model names start with, lowercased.
const LAPTOP_MODEL: &str = "macbook"; // leakscan: allow

/// A Mac, from `pmset -g batt` and `sysctl -n hw.model`. Apple silicon laptops have model names like "Mac14,2", so
/// the built-in battery is the main sign; a laptop model name is the other.
pub fn mac_power(pmset: &str, model: &str) -> Power {
    if pmset.lines().any(|l| l.contains("InternalBattery")) {
        return Power::Laptop("It has a built-in battery.".into());
    }
    if model.to_ascii_lowercase().contains(LAPTOP_MODEL) {
        return Power::Laptop(format!("It is a {}.", model.trim()));
    }
    if pmset.trim().is_empty() && model.trim().is_empty() { Power::Unknown } else { Power::Desktop }
}

/// One entry of /sys/class/power_supply: its folder name, `type` and `scope`.
#[derive(Debug, Clone, Default)]
pub struct Supply {
    pub name: String,
    pub kind: String,
    pub scope: String,
}

/// Linux, from /sys/class/power_supply and /sys/class/dmi/id/chassis_type. A battery whose scope is "Device" is a
/// mouse's or a headset's, not the computer's.
pub fn linux_power(supplies: &[Supply], chassis: Option<u32>) -> Power {
    let own = supplies.iter().any(|s| {
        !s.scope.trim().eq_ignore_ascii_case("device")
            && (s.kind.trim().eq_ignore_ascii_case("battery") || s.name.starts_with("BAT"))
    });
    if own {
        return Power::Laptop("It has a built-in battery.".into());
    }
    // SMBIOS chassis types: portable, laptop, notebook, hand held, sub notebook, tablet, convertible, detachable.
    if chassis.is_some_and(|c| matches!(c, 8 | 9 | 10 | 11 | 14 | 30 | 31 | 32)) {
        return Power::Laptop("It's built as a laptop.".into());
    }
    if chassis.is_some() || !supplies.is_empty() { Power::Desktop } else { Power::Unknown }
}

/// Windows, from GetSystemPowerStatus's BatteryFlag: 128 is "no system battery", 255 "unknown".
pub fn windows_power(battery_flag: u8) -> Power {
    match battery_flag {
        128 => Power::Desktop,
        255 => Power::Unknown,
        _ => Power::Laptop("It has a built-in battery.".into()),
    }
}

/// Looks at this computer.
pub fn detect_power() -> Power {
    #[cfg(target_os = "macos")]
    {
        let pmset = run_out("/usr/bin/pmset", &["-g", "batt"]).unwrap_or_default();
        let model = run_out("/usr/sbin/sysctl", &["-n", "hw.model"]).unwrap_or_default();
        mac_power(&pmset, &model)
    }
    #[cfg(windows)]
    {
        windows_power(win::battery_flag().unwrap_or(255))
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        let mut supplies = Vec::new();
        if let Ok(dir) = std::fs::read_dir("/sys/class/power_supply") {
            for e in dir.flatten() {
                let p = e.path();
                let read = |f: &str| std::fs::read_to_string(p.join(f)).unwrap_or_default();
                supplies.push(Supply {
                    name: e.file_name().to_string_lossy().into_owned(),
                    kind: read("type"),
                    scope: read("scope"),
                });
            }
        }
        let chassis =
            std::fs::read_to_string("/sys/class/dmi/id/chassis_type").ok().and_then(|s| s.trim().parse().ok());
        linux_power(&supplies, chassis)
    }
}

#[cfg(windows)]
mod win {
    #[repr(C)]
    #[derive(Default)]
    struct SystemPowerStatus {
        ac_line_status: u8,
        battery_flag: u8,
        battery_life_percent: u8,
        system_status_flag: u8,
        battery_life_time: u32,
        battery_full_life_time: u32,
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetSystemPowerStatus(status: *mut SystemPowerStatus) -> i32;
    }

    pub fn battery_flag() -> Option<u8> {
        let mut s = SystemPowerStatus::default();
        // SAFETY: a plain out parameter of the documented layout.
        let ok = unsafe { GetSystemPowerStatus(&mut s) };
        (ok != 0).then_some(s.battery_flag)
    }
}

#[cfg(target_os = "macos")]
fn run_out(program: &str, args: &[&str]) -> Option<String> {
    let out = std::process::Command::new(program).args(args).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

// ---------------------------------------------------------------------------------------------------------------
// Python 3
// ---------------------------------------------------------------------------------------------------------------

/// "3.12.4" from what `python3 -c 'print(...)'` or `python3 --version` printed.
pub fn python_version(out: &str) -> Option<(u32, u32, u32)> {
    let v = out.trim().trim_start_matches("Python").trim();
    let mut it = v.split(['.', ' ', '+']).map(|p| p.trim_end_matches(|c: char| !c.is_ascii_digit()).parse::<u32>());
    let major = it.next()?.ok()?;
    let minor = it.next()?.ok()?;
    let patch = it.next().and_then(|p| p.ok()).unwrap_or(0);
    Some((major, minor, patch))
}

pub fn python_new_enough(v: (u32, u32, u32)) -> bool {
    (v.0, v.1) >= PYTHON_MIN
}

/// Python 3 on this computer: the program and its version, or why there is none, in plain words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Python {
    Found { path: PathBuf, version: String },
    Missing(String),
    TooOld(String),
}

/// Where a Python 3 may be: Homebrew's first (an app started from the Dock has a short PATH), then PATH, then the
/// system's.
fn python_candidates(os: Os) -> Vec<PathBuf> {
    let exe = if os == Os::Windows { "python.exe" } else { "python3" };
    let mut dirs: Vec<PathBuf> = Vec::new();
    if os == Os::Mac {
        dirs.extend(["/opt/homebrew/bin", "/usr/local/bin"].map(PathBuf::from));
    }
    if let Some(path) = std::env::var_os("PATH") {
        dirs.extend(std::env::split_paths(&path));
    }
    if os != Os::Windows {
        dirs.push(PathBuf::from("/usr/bin"));
    }
    let mut out: Vec<PathBuf> = Vec::new();
    for d in dirs {
        let p = d.join(exe);
        if p.is_file() && !out.contains(&p) {
            out.push(p);
        }
    }
    out
}

/// Finds Python 3. On a Mac, /usr/bin/python3 is only a stand-in until Apple's command line tools are installed
/// (running it then opens Apple's install dialog), so it counts only when `xcode-select -p` says they are.
pub fn find_python(os: Os) -> Python {
    let mut old = None;
    for p in python_candidates(os) {
        if os == Os::Mac && p == Path::new("/usr/bin/python3") && !mac_tools_installed() {
            continue;
        }
        let Ok(out) = std::process::Command::new(&p)
            .args(["-c", "import sys; print('%d.%d.%d' % sys.version_info[:3])"])
            .output()
        else {
            continue;
        };
        let Some(v) = python_version(&String::from_utf8_lossy(&out.stdout)).filter(|_| out.status.success()) else {
            continue;
        };
        let shown = format!("{}.{}.{}", v.0, v.1, v.2);
        if python_new_enough(v) {
            return Python::Found { path: p, version: shown };
        }
        old.get_or_insert(shown);
    }
    match old {
        Some(v) => Python::TooOld(format!(
            "This has Python {v}, and Switchboard needs {}.{} or newer.",
            PYTHON_MIN.0, PYTHON_MIN.1
        )),
        None => Python::Missing(match os {
            Os::Mac => "Python 3 comes with Apple's command line tools, which aren't installed yet.".into(),
            Os::Windows => "Python 3 isn't installed. Get it from python.org, then check again.".into(),
            Os::Linux => "Python 3 isn't installed. Add it with your package manager, then check again.".into(),
        }),
    }
}

fn mac_tools_installed() -> bool {
    std::process::Command::new("/usr/bin/xcode-select")
        .arg("-p")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

// ---------------------------------------------------------------------------------------------------------------
// Ports
// ---------------------------------------------------------------------------------------------------------------

/// The ports something already listens on. A port counts as taken when it cannot be bound, or when something
/// answers on it (a program listening only on 127.0.0.1 does not stop a bind to every address on every system).
pub fn busy_ports(ports: &[u16]) -> Vec<u16> {
    use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
    ports
        .iter()
        .copied()
        .filter(|&p| {
            let bound = TcpListener::bind(SocketAddr::from((Ipv4Addr::UNSPECIFIED, p))).is_ok();
            let answers =
                TcpStream::connect_timeout(&SocketAddr::from((Ipv4Addr::LOCALHOST, p)), Duration::from_millis(300))
                    .is_ok();
            !bound || answers
        })
        .collect()
}

/// "8131", "8130 and 8132", "8130, 8131 and 8132".
pub fn and_list(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [a] => a.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

// ---------------------------------------------------------------------------------------------------------------
// The check list
// ---------------------------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mark {
    /// Still looking.
    Wait,
    Ok,
    /// Fine to go on, after reading it (the laptop warning needs a yes of its own).
    Warn,
    /// Has to be fixed first.
    Bad,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Kit,
    Laptop,
    Python,
    Ports,
    Others,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Check {
    pub key: Key,
    pub title: String,
    pub mark: Mark,
    pub detail: String,
}

/// What the checks found. None: still looking.
#[derive(Debug, Clone, Default)]
pub struct Facts {
    pub power: Option<Power>,
    pub python: Option<Python>,
    pub busy: Option<Vec<u16>>,
    /// The search is still running.
    pub searching: bool,
    /// Servers on other computers.
    pub others: Vec<Found>,
    /// A server that already runs here.
    pub here: Option<Found>,
    /// The app carries the server kit.
    pub kit: bool,
}

pub fn checks(os: Os, f: &Facts) -> Vec<Check> {
    let this = os.this_one();
    let mut out = Vec::new();
    if !f.kit {
        out.push(Check {
            key: Key::Kit,
            title: "The server comes with this app".into(),
            mark: Mark::Bad,
            detail: "This copy of the app doesn't include the server. Use the script in the setup guide \
                     instead."
                .into(),
        });
    }
    out.push(match &f.power {
        None => Check {
            key: Key::Laptop,
            title: "Looking at this computer".into(),
            mark: Mark::Wait,
            detail: String::new(),
        },
        Some(Power::Laptop(why)) => Check {
            key: Key::Laptop,
            title: format!("{} looks like a laptop", capital(this)),
            mark: Mark::Warn,
            detail: format!(
                "{why} A laptop probably isn't a good fit for the server: it sleeps when the lid closes and leaves \
                 the network when it goes with you, and then your phone and your other machines lose Switchboard. A \
                 computer that stays on at home, like a desktop or a Mac mini, works best."
            ),
        },
        Some(Power::Desktop) => Check {
            key: Key::Laptop,
            title: format!("{} can stay on", capital(this)),
            mark: Mark::Ok,
            detail: "It has no built-in battery, so it's made to stay plugged in.".into(),
        },
        Some(Power::Unknown) => Check {
            key: Key::Laptop,
            title: format!("{} should stay on", capital(this)),
            mark: Mark::Ok,
            detail: "Switchboard couldn't tell if this is a laptop. The server works best on a computer that \
                     stays on at home."
                .into(),
        },
    });
    out.push(match &f.python {
        None => {
            Check { key: Key::Python, title: "Looking for Python 3".into(), mark: Mark::Wait, detail: String::new() }
        }
        Some(Python::Found { version, .. }) => Check {
            key: Key::Python,
            title: format!("Python {version} is here"),
            mark: Mark::Ok,
            detail: "The server is a Python program, and this is all it needs.".into(),
        },
        Some(Python::Missing(why)) => {
            Check { key: Key::Python, title: "Python 3 is missing".into(), mark: Mark::Bad, detail: why.clone() }
        }
        Some(Python::TooOld(why)) => {
            Check { key: Key::Python, title: "Python is too old".into(), mark: Mark::Bad, detail: why.clone() }
        }
    });
    out.push(match &f.busy {
        None => Check { key: Key::Ports, title: "Checking the ports".into(), mark: Mark::Wait, detail: String::new() },
        Some(b) if b.is_empty() => Check {
            key: Key::Ports,
            title: "Ports 8130, 8131 and 8132 are free".into(),
            mark: Mark::Ok,
            detail: "The server, its downloads and its hub each listen on one.".into(),
        },
        Some(b) => {
            let names: Vec<String> = b.iter().map(|p| p.to_string()).collect();
            let (one, verb) = if b.len() == 1 { ("Port", "is") } else { ("Ports", "are") };
            Check {
                key: Key::Ports,
                title: format!("{one} {} {verb} in use", and_list(&names)),
                mark: Mark::Bad,
                detail: format!(
                    "Another program on {this} listens on {}. Switchboard needs 8130, 8131 and 8132. Quit that \
                     program, then check again.",
                    and_list(&names)
                ),
            }
        }
    });
    out.push(if let Some(h) = &f.here {
        Check {
            key: Key::Others,
            title: format!("Switchboard already runs on {this}"),
            mark: Mark::Bad,
            detail: format!("It answers at {}. Connect to it instead of setting it up again.", h.shown()),
        }
    } else if f.searching {
        Check { key: Key::Others, title: "Looking for other servers".into(), mark: Mark::Wait, detail: String::new() }
    } else if f.others.is_empty() {
        Check {
            key: Key::Others,
            title: "No other server on this network".into(),
            mark: Mark::Ok,
            detail: "This will be the one your machines and phones use.".into(),
        }
    } else {
        let names: Vec<String> = f.others.iter().map(|o| o.name.clone()).collect();
        Check {
            key: Key::Others,
            title: format!("{} already runs a server", and_list(&names)),
            mark: Mark::Warn,
            detail: "One server is enough for all your machines. You can still set up another one here, but the \
                     apps on this network will then ask which to use."
                .into(),
        }
    });
    out
}

fn capital(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// A check is still running.
    Checking,
    /// Something has to be fixed first.
    Blocked,
    /// The laptop warning has to be answered first.
    NeedsYes,
    Go,
}

/// Whether the setup may go on. `laptop_yes`: the user said to use the laptop anyway.
pub fn verdict(checks: &[Check], laptop_yes: bool) -> Verdict {
    if checks.iter().any(|c| c.mark == Mark::Bad) {
        return Verdict::Blocked;
    }
    if checks.iter().any(|c| c.mark == Mark::Wait) {
        return Verdict::Checking;
    }
    if !laptop_yes && checks.iter().any(|c| c.key == Key::Laptop && c.mark == Mark::Warn) {
        return Verdict::NeedsYes;
    }
    Verdict::Go
}

// ---------------------------------------------------------------------------------------------------------------
// The server kit and the installer
// ---------------------------------------------------------------------------------------------------------------

/// The server and its installer, as the app carries them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Kit {
    pub root: PathBuf,
}

impl Kit {
    /// A kit at `root`, when everything the installer needs is there.
    pub fn at(root: &Path) -> Option<Kit> {
        let has = |p: &str| root.join(p).is_file();
        let script = has("scripts/install/install-server.sh") || has("scripts/install/install-server.ps1");
        (script && has("server/server.py") && has("agent/sb.py") && has("scripts/install/setup.py"))
            .then(|| Kit { root: root.to_path_buf() })
    }

    /// Where the app keeps it: SB_SERVER_KIT when set, else beside the app (a Mac's `Contents/Resources/server-kit`,
    /// elsewhere a `server-kit` folder next to the program).
    pub fn find() -> Option<Kit> {
        if let Some(p) = std::env::var_os("SB_SERVER_KIT") {
            return Kit::at(Path::new(&p));
        }
        let exe = std::env::current_exe().ok()?;
        kit_places(&exe).iter().find_map(|p| Kit::at(p))
    }

    /// The hub built for this kind of computer, when the kit has one.
    pub fn hub(&self, os: Os) -> Option<PathBuf> {
        let p = self.root.join("native/dist").join(format!("switchboard-hub-{}", os.short()));
        p.is_file().then_some(p)
    }

    /// The installer and its arguments, run as the user who runs the app (never with sudo).
    pub fn command(&self, os: Os) -> (PathBuf, Vec<String>) {
        let script = |n: &str| self.root.join("scripts/install").join(n).display().to_string();
        match os {
            Os::Windows => (
                PathBuf::from("powershell.exe"),
                ["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"]
                    .iter()
                    .map(|s| s.to_string())
                    .chain([script("install-server.ps1"), "-NoSetup".into()])
                    .collect(),
            ),
            _ => (
                PathBuf::from("/bin/bash"),
                install_args(&script("install-server.sh"), self.hub(os).map(|h| h.display().to_string()).as_deref()),
            ),
        }
    }
}

/// Where a kit may sit for the program at `exe`.
pub fn kit_places(exe: &Path) -> Vec<PathBuf> {
    let Some(dir) = exe.parent() else { return Vec::new() };
    vec![dir.join("../Resources/server-kit"), dir.join("server-kit"), dir.join("../share/switchboard/server-kit")]
}

/// install-server.sh's arguments: nothing asked, no first-run setup (the app does that part), and the bundled hub,
/// or none (then there are no terminals and machines are added over ssh, not with a code).
pub fn install_args(script: &str, hub: Option<&str>) -> Vec<String> {
    let mut a = vec![script.to_string(), "--yes".into(), "--no-setup".into()];
    match hub {
        Some(h) => a.extend(["--hub-binary".into(), h.to_string()]),
        None => a.push("--no-hub".into()),
    }
    a
}

/// PATH for the installer: the folder of the Python the check found first, then Homebrew's and the usual ones,
/// then the app's own. An app started from the Dock has only the system's folders.
pub fn install_path(os: Os, python: Option<&Path>, inherited: &str) -> String {
    let sep = if os == Os::Windows { ';' } else { ':' };
    let mut parts: Vec<String> = Vec::new();
    if let Some(d) = python.and_then(|p| p.parent()) {
        parts.push(d.display().to_string());
    }
    if os == Os::Mac {
        parts.extend(["/opt/homebrew/bin", "/usr/local/bin"].map(String::from));
    }
    parts.extend(inherited.split(sep).filter(|s| !s.is_empty()).map(String::from));
    if os != Os::Windows {
        parts.extend(["/usr/bin", "/bin", "/usr/sbin", "/sbin"].map(String::from));
    }
    let mut seen: Vec<String> = Vec::new();
    for p in parts {
        if !seen.contains(&p) {
            seen.push(p);
        }
    }
    seen.join(&sep.to_string())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepState {
    Todo,
    Now,
    Done,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    /// The installer's own words after "== ".
    pub raw: String,
    /// What the app shows.
    pub title: String,
    pub state: StepState,
}

/// The installer's progress, read line by line.
#[derive(Debug, Clone, Default)]
pub struct Install {
    pub steps: Vec<Step>,
    tail: Vec<String>,
    /// Some once it ended: whether it worked.
    pub ended: Option<bool>,
}

/// How many of the installer's last lines are kept, for an error.
const TAIL: usize = 14;

/// The steps install-server.sh takes, in its words. The hub step only when there is a hub.
pub fn plan(hub: bool) -> Vec<&'static str> {
    let mut p = vec!["Checking this machine", "Putting the server in place"];
    if hub {
        p.push("The hub");
    }
    p.extend(["Downloads for Add a machine", "Running it"]);
    p
}

/// The app's words for an installer step ("Running it (launchd)" is "Starting Switchboard").
pub fn step_title(raw: &str) -> String {
    let r = raw.trim();
    let known = [
        ("Checking this machine", "Checking this computer"),
        ("Putting the server in place", "Putting the server in place"),
        ("The hub", "Adding the hub, for terminals and linked machines"),
        ("Downloads for Add a machine", "Getting downloads ready for your other machines"),
        ("Running it", "Starting Switchboard and keeping it running"),
        ("First-run setup", "First-run setup"),
    ];
    known.iter().find(|(k, _)| r.starts_with(k)).map(|(_, v)| v.to_string()).unwrap_or_else(|| r.to_string())
}

/// A line without terminal colors.
pub fn plain(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut it = line.chars().peekable();
    while let Some(c) = it.next() {
        if c == '\u{1b}' {
            if it.peek() == Some(&'[') {
                it.next();
                for d in it.by_ref() {
                    if d.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            continue;
        }
        if c != '\r' {
            out.push(c);
        }
    }
    out
}

impl Install {
    pub fn new(plan: &[&str]) -> Install {
        Install {
            steps: plan
                .iter()
                .map(|r| Step { raw: r.to_string(), title: step_title(r), state: StepState::Todo })
                .collect(),
            tail: Vec::new(),
            ended: None,
        }
    }

    /// One line the installer printed.
    pub fn line(&mut self, raw: &str) {
        let l = plain(raw);
        let l = l.trim_end();
        if let Some(name) = l.strip_prefix("== ") {
            let name = name.trim();
            for s in self.steps.iter_mut().filter(|s| s.state == StepState::Now) {
                s.state = StepState::Done;
            }
            let at = self.steps.iter().position(|s| s.state == StepState::Todo && name.starts_with(s.raw.as_str()));
            match at {
                Some(i) => {
                    // Steps it went past without a line of their own count as done.
                    for s in self.steps[..i].iter_mut().filter(|s| s.state == StepState::Todo) {
                        s.state = StepState::Done;
                    }
                    self.steps[i].state = StepState::Now;
                }
                None => {
                    let i = self.steps.iter().rposition(|s| s.state == StepState::Done).map_or(0, |i| i + 1);
                    self.steps
                        .insert(i, Step { raw: name.to_string(), title: step_title(name), state: StepState::Now });
                }
            }
        }
        if !l.trim().is_empty() {
            self.tail.push(l.to_string());
            let over = self.tail.len().saturating_sub(TAIL);
            self.tail.drain(..over);
        }
    }

    /// The installer ended.
    pub fn finish(&mut self, ok: bool) {
        self.ended = Some(ok);
        for s in &mut self.steps {
            s.state = match (ok, s.state) {
                (true, _) => StepState::Done,
                (false, StepState::Now) => StepState::Failed,
                (false, st) => st,
            };
        }
        if !ok
            && !self.steps.iter().any(|s| s.state == StepState::Failed)
            && let Some(s) = self.steps.iter_mut().find(|s| s.state == StepState::Todo)
        {
            s.state = StepState::Failed;
        }
    }

    /// Its last lines, oldest first.
    pub fn tail(&self) -> &[String] {
        &self.tail
    }
}

// ---------------------------------------------------------------------------------------------------------------
// The words for the last step
// ---------------------------------------------------------------------------------------------------------------

/// How to keep the server awake: what to do, and the button that opens the right settings (label, link).
pub fn awake_tip(os: Os, laptop: bool) -> (String, Option<(&'static str, &'static str)>) {
    match (os, laptop) {
        (Os::Mac, false) => (
            "Open System Settings > Energy and turn on \"Prevent automatic sleeping when the display is off\" and \
             \"Wake for network access\". \"Start up automatically after a power failure\" helps too."
                .into(),
            Some(("Open Energy settings", "x-apple.systempreferences:com.apple.Energy-Saver-Settings.extension")),
        ),
        (Os::Mac, true) => (
            "Keep it plugged in and open. In System Settings > Battery > Options, turn on \"Prevent automatic \
             sleeping on power adapter when the display is off\" and \"Wake for network access\"."
                .into(),
            Some(("Open Battery settings", "x-apple.systempreferences:com.apple.Battery-Settings.extension")),
        ),
        (Os::Windows, _) => (
            "Open Settings > System > Power and set sleep to Never when plugged in.".into(),
            Some(("Open Power settings", "ms-settings:powersleep")),
        ),
        (Os::Linux, _) => {
            ("In your system's power settings, turn off automatic suspend while it's plugged in.".into(), None)
        }
    }
}

/// How phones and the other machines find the new server.
pub fn find_tip(os: Os) -> String {
    let mut s = "Your iPhone and your other computers on this network find it by themselves: open Switchboard there \
                 and it shows up while it searches. To run chats on another machine, use Add a machine in Settings."
        .to_string();
    if os == Os::Mac {
        s.push_str(
            " If macOS asks whether Python may accept incoming network connections, choose Allow, so they can reach \
             it.",
        );
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    const LAPTOP: &str = "Now drawing from 'Battery Power'\n -InternalBattery-0 (id=4653155)\t85%; discharging; \
                           4:12 remaining present: true\n";
    const MINI: &str = "Now drawing from 'AC Power'\n";
    const UPS: &str = "Now drawing from 'AC Power'\n -CyberPower UPS (id=1234)\t100%; charged; present: true\n";

    #[test]
    fn a_mac_with_a_battery_is_a_laptop() {
        assert!(matches!(mac_power(LAPTOP, "Mac14,2"), Power::Laptop(_)), "Apple silicon names say nothing");
        assert_eq!(mac_power(MINI, "Mac14,3"), Power::Desktop);
        assert_eq!(mac_power(UPS, "Macmini9,1"), Power::Desktop, "a UPS is not a built-in battery");
        assert!(matches!(mac_power(MINI, "MacBookPro18,3"), Power::Laptop(ref w) if w.contains("MacBookPro18,3")));
        assert_eq!(mac_power("", ""), Power::Unknown);
    }

    #[test]
    fn linux_and_windows_batteries() {
        let s = |n: &str, k: &str, sc: &str| Supply { name: n.into(), kind: k.into(), scope: sc.into() };
        assert!(matches!(linux_power(&[s("AC", "Mains\n", ""), s("BAT0", "Battery\n", "")], None), Power::Laptop(_)));
        assert_eq!(
            linux_power(&[s("hidpp_battery_0", "Battery\n", "Device\n")], Some(3)),
            Power::Desktop,
            "a mouse battery does not make a laptop"
        );
        assert!(matches!(linux_power(&[], Some(10)), Power::Laptop(_)));
        assert_eq!(linux_power(&[], None), Power::Unknown);
        assert_eq!(windows_power(128), Power::Desktop);
        assert_eq!(windows_power(255), Power::Unknown);
        assert!(matches!(windows_power(1), Power::Laptop(_)));
        assert!(matches!(windows_power(8), Power::Laptop(_)));
    }

    #[test]
    fn python_versions() {
        assert_eq!(python_version("3.12.4\n"), Some((3, 12, 4)));
        assert_eq!(python_version("Python 3.9.6"), Some((3, 9, 6)));
        assert_eq!(python_version("3.13.0rc1"), Some((3, 13, 0)));
        assert_eq!(python_version("nope"), None);
        assert!(python_new_enough((3, 9, 0)));
        assert!(!python_new_enough((3, 8, 18)));
        assert!(python_new_enough((4, 0, 0)));
    }

    fn all_good() -> Facts {
        Facts {
            power: Some(Power::Desktop),
            python: Some(Python::Found { path: "/usr/bin/python3".into(), version: "3.9.6".into() }),
            busy: Some(vec![]),
            searching: false,
            others: vec![],
            here: None,
            kit: true,
        }
    }

    #[test]
    fn the_check_list() {
        let c = checks(Os::Mac, &all_good());
        let keys: Vec<Key> = c.iter().map(|c| c.key).collect();
        assert_eq!(keys, vec![Key::Laptop, Key::Python, Key::Ports, Key::Others]);
        assert!(c.iter().all(|c| c.mark == Mark::Ok));
        assert_eq!(c[0].title, "This Mac can stay on");
        assert_eq!(verdict(&c, false), Verdict::Go);

        let c = checks(Os::Mac, &Facts::default());
        assert_eq!(c[0].key, Key::Kit, "no kit: said first");
        assert_eq!(verdict(&c, true), Verdict::Blocked);
        let c = checks(Os::Mac, &Facts { kit: true, searching: true, ..Default::default() });
        assert!(c.iter().all(|c| c.mark == Mark::Wait));
        assert_eq!(verdict(&c, false), Verdict::Checking);
    }

    #[test]
    fn a_laptop_needs_its_own_yes() {
        let f = Facts { power: Some(Power::Laptop("It has a built-in battery.".into())), ..all_good() };
        let c = checks(Os::Mac, &f);
        assert_eq!(c[0].mark, Mark::Warn);
        assert_eq!(c[0].title, "This Mac looks like a laptop");
        assert!(c[0].detail.contains("sleeps when the lid closes"));
        assert_eq!(verdict(&c, false), Verdict::NeedsYes);
        assert_eq!(verdict(&c, true), Verdict::Go);
    }

    #[test]
    fn what_blocks_and_what_warns() {
        let c = checks(Os::Linux, &Facts { busy: Some(vec![8131]), ..all_good() });
        let p = c.iter().find(|c| c.key == Key::Ports).unwrap();
        assert_eq!((p.mark, p.title.as_str()), (Mark::Bad, "Port 8131 is in use"));
        let c = checks(Os::Linux, &Facts { busy: Some(vec![8130, 8132]), ..all_good() });
        assert_eq!(c.iter().find(|c| c.key == Key::Ports).unwrap().title, "Ports 8130 and 8132 are in use");
        assert_eq!(verdict(&c, true), Verdict::Blocked);

        let c = checks(Os::Mac, &Facts { python: Some(Python::Missing("not here".into())), ..all_good() });
        assert_eq!(verdict(&c, true), Verdict::Blocked);

        let other = Found { name: "studio1".into(), url: "http://studio1.local:8130".into(), here: false };
        let c = checks(Os::Mac, &Facts { others: vec![other.clone()], ..all_good() });
        let o = c.iter().find(|c| c.key == Key::Others).unwrap();
        assert_eq!((o.mark, o.title.as_str()), (Mark::Warn, "studio1 already runs a server"));
        assert_eq!(verdict(&c, false), Verdict::Go, "another server is a note, not a stop");

        let c = checks(Os::Mac, &Facts { here: Some(Found { here: true, ..other }), ..all_good() });
        assert_eq!(c.iter().find(|c| c.key == Key::Others).unwrap().mark, Mark::Bad);

        let c = checks(Os::Mac, &Facts { searching: true, ..all_good() });
        assert_eq!(verdict(&c, false), Verdict::Checking);
        for c in checks(Os::Mac, &Facts { power: Some(Power::Laptop("x".into())), ..all_good() }) {
            assert!(!c.title.contains('\u{2014}') && !c.detail.contains('\u{2014}'), "no em dashes");
        }
    }

    #[test]
    fn the_installer_command() {
        assert_eq!(
            install_args("/k/scripts/install/install-server.sh", Some("/k/native/dist/switchboard-hub-mac")),
            vec![
                "/k/scripts/install/install-server.sh",
                "--yes",
                "--no-setup",
                "--hub-binary",
                "/k/native/dist/switchboard-hub-mac"
            ]
        );
        assert_eq!(install_args("i.sh", None), vec!["i.sh", "--yes", "--no-setup", "--no-hub"]);
        assert!(!install_args("i.sh", None).iter().any(|a| a.contains("sudo")));
        let p = install_path(Os::Mac, Some(Path::new("/opt/homebrew/bin/python3")), "/usr/bin:/bin");
        assert_eq!(p, "/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin");
        let places = kit_places(Path::new("/Applications/Switchboard.app/Contents/MacOS/Switchboard"));
        assert_eq!(places[0], Path::new("/Applications/Switchboard.app/Contents/MacOS/../Resources/server-kit"));
    }

    #[test]
    fn a_kit_needs_its_files_and_finds_its_hub() {
        let d = std::env::temp_dir().join(format!("sb-kit-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        assert_eq!(Kit::at(&d), None);
        std::fs::create_dir_all(d.join("server")).unwrap();
        std::fs::write(d.join("server/server.py"), "").unwrap();
        assert_eq!(Kit::at(&d), None, "half a kit is no kit");
        for f in ["agent/sb.py", "scripts/install/setup.py", "scripts/install/install-server.sh"] {
            std::fs::create_dir_all(d.join(f).parent().unwrap()).unwrap();
            std::fs::write(d.join(f), "").unwrap();
        }
        let k = Kit::at(&d).unwrap();
        assert_eq!(k.hub(Os::Mac), None);
        assert_eq!(k.command(Os::Mac).1.last().map(String::as_str), Some("--no-hub"));
        std::fs::create_dir_all(d.join("native/dist")).unwrap();
        std::fs::write(d.join("native/dist/switchboard-hub-mac"), "").unwrap();
        let (prog, args) = k.command(Os::Mac);
        assert_eq!(prog, Path::new("/bin/bash"));
        assert!(args[0].ends_with("scripts/install/install-server.sh"));
        assert_eq!(args[3], "--hub-binary");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn progress_from_the_installer_lines() {
        let mut i = Install::new(&plan(true));
        assert_eq!(i.steps.len(), 5);
        assert!(i.steps.iter().all(|s| s.state == StepState::Todo));
        for l in ["", "== Checking this machine", "Python 3.12.4 at /opt/homebrew/bin/python3", "Time zone UTC"] {
            i.line(l);
        }
        assert_eq!(i.steps[0].state, StepState::Now);
        assert_eq!(i.steps[0].title, "Checking this computer");
        i.line("\u{1b}[1m== Putting the server in place\u{1b}[0m");
        assert_eq!(i.steps[0].state, StepState::Done);
        assert_eq!(i.steps[1].state, StepState::Now);
        // It skipped the hub and went on: the hub counts as done.
        i.line("== Downloads for Add a machine");
        assert_eq!(i.steps[2].state, StepState::Done);
        i.line("== Running it (launchd)");
        assert_eq!(i.steps[4].state, StepState::Now);
        assert_eq!(i.steps[4].title, "Starting Switchboard and keeping it running");
        i.line("install-server.sh: the server did not start. See: switchboard logs");
        i.finish(false);
        assert_eq!(i.steps[4].state, StepState::Failed);
        assert_eq!(i.tail().last().unwrap(), "install-server.sh: the server did not start. See: switchboard logs");
        assert!(!i.tail().iter().any(|l| l.is_empty()));

        let mut i = Install::new(&plan(false));
        assert_eq!(i.steps.len(), 4);
        i.line("== Something new");
        assert_eq!(i.steps[0].title, "Something new", "a step it did not plan for still shows");
        i.finish(true);
        assert!(i.steps.iter().all(|s| s.state == StepState::Done));
        let mut i = Install::new(&plan(false));
        i.finish(false);
        assert_eq!(i.steps[0].state, StepState::Failed, "a failure before any step marks the first");
    }

    #[test]
    fn the_tail_keeps_the_last_lines() {
        let mut i = Install::new(&plan(false));
        for n in 0..40 {
            i.line(&format!("line {n}"));
        }
        assert_eq!(i.tail().len(), TAIL);
        assert_eq!(i.tail()[0], "line 26");
    }

    #[test]
    fn the_last_words() {
        let (t, b) = awake_tip(Os::Mac, false);
        assert!(t.contains("System Settings > Energy"));
        assert_eq!(b.unwrap().0, "Open Energy settings");
        assert!(awake_tip(Os::Mac, true).0.contains("Battery"));
        assert!(awake_tip(Os::Linux, false).1.is_none());
        assert!(find_tip(Os::Mac).contains("Allow"));
        assert!(!find_tip(Os::Linux).contains("Allow"));
        assert_eq!(and_list(&["a".into(), "b".into(), "c".into()]), "a, b and c");
    }
}
