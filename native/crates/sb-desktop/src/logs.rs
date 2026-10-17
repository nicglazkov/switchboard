//! The app's log: every `log::` line goes to a file in the app's log folder, so a bug report can say what happened.
//!   macOS    ~/Library/Logs/Switchboard/
//!   Windows  %LOCALAPPDATA%\Switchboard\logs\
//!   Linux    $XDG_STATE_HOME/switchboard/logs/ (~/.local/state/switchboard/logs/)
//! Switchboard.log is the current file. Past 2 MB it becomes Switchboard.log.1 (and .1 becomes .2, and so on), and
//! 5 files are kept. Each line is "time LEVEL module: message". SB_LOG (or RUST_LOG) filters as env_logger does:
//! "debug", "warn,Switchboard=debug" (this binary's modules), "sb_core::term=trace"; the default is info.
//! Lines never carry the home folder (it reads ~) or a token in a URL. Chat text must not be logged at all.
use std::fs::File;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use log::LevelFilter;

pub const FILE: &str = "Switchboard.log";
/// Rotate past this many bytes.
pub const MAX_BYTES: u64 = 2 << 20;
/// Files kept, the current one included.
pub const KEEP: usize = 5;

/// The log folder (SB_LOG_DIR overrides it, for tests and odd setups).
pub fn dir() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("SB_LOG_DIR") {
        return Some(PathBuf::from(p));
    }
    if cfg!(target_os = "macos") {
        home().map(|h| h.join("Library/Logs/Switchboard"))
    } else if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA").map(|d| PathBuf::from(d).join("Switchboard").join("logs"))
    } else {
        std::env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .or_else(|| home().map(|h| h.join(".local/state")))
            .map(|d| d.join("switchboard/logs"))
    }
}

fn home() -> Option<PathBuf> {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from).filter(|h| h.is_absolute())
}

/// Text fit for the log: the home folder reads ~, and a secret in a URL query (token=, key=, ...) reads ***.
pub fn scrub(s: &str) -> String {
    let mut out = s.to_string();
    if let Some(h) = home() {
        let h = h.to_string_lossy().trim_end_matches(['/', '\\']).to_string();
        // A bare "/" or "C:" home would eat every path.
        if h.len() > 3 {
            out = out.replace(&h, "~");
            if cfg!(windows) {
                out = out.replace(&h.replace('\\', "/"), "~");
            }
        }
    }
    redact_query(&out)
}

/// `?token=abc&x=1` reads `?token=***&x=1`, for every name that holds a secret.
fn redact_query(s: &str) -> String {
    const NAMES: &[&str] = &["token", "key", "secret", "password", "ticket", "auth", "code"];
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find(['?', '&']) {
        out.push_str(&rest[..=i]);
        rest = &rest[i + 1..];
        let Some(eq) = rest.find('=') else { continue };
        let name = &rest[..eq];
        if !name.is_empty() && name.len() <= 24 && NAMES.iter().any(|n| name.eq_ignore_ascii_case(n)) {
            let end = rest[eq + 1..]
                .find(|c: char| c == '&' || c == '#' || c.is_whitespace())
                .map_or(rest.len(), |e| eq + 1 + e);
            out.push_str(name);
            out.push_str("=***");
            rest = &rest[end..];
        }
    }
    out.push_str(rest);
    out
}

/// SB_LOG / RUST_LOG, parsed: a default level and per-module levels (longest module prefix wins).
#[derive(Debug, Clone, PartialEq)]
pub struct Filter {
    default: LevelFilter,
    modules: Vec<(String, LevelFilter)>,
}

impl Filter {
    pub fn parse(spec: Option<&str>) -> Filter {
        let mut f = Filter { default: LevelFilter::Info, modules: Vec::new() };
        for part in spec.unwrap_or("").split(',').map(str::trim).filter(|p| !p.is_empty()) {
            match part.split_once('=') {
                Some((m, l)) => {
                    if let Ok(l) = l.trim().parse() {
                        f.modules.push((m.trim().to_string(), l));
                    }
                }
                None => match part.parse() {
                    Ok(l) => f.default = l,
                    // A bare module name turns everything on for it, as env_logger does.
                    Err(_) => f.modules.push((part.to_string(), LevelFilter::Trace)),
                },
            }
        }
        f.modules.sort_by_key(|(m, _)| std::cmp::Reverse(m.len()));
        f
    }

    pub fn from_env() -> Filter {
        let v = std::env::var("SB_LOG").or_else(|_| std::env::var("RUST_LOG")).ok();
        Filter::parse(v.as_deref())
    }

    pub fn level_for(&self, target: &str) -> LevelFilter {
        self.modules
            .iter()
            .find(|(m, _)| target == m || target.strip_prefix(m.as_str()).is_some_and(|r| r.starts_with("::")))
            .map_or(self.default, |(_, l)| *l)
    }

    pub fn max(&self) -> LevelFilter {
        self.modules.iter().map(|(_, l)| *l).fold(self.default, std::cmp::max)
    }
}

/// The open file and how big it is, so a write knows when to rotate without asking the disk.
struct Sink {
    dir: PathBuf,
    file: Option<File>,
    len: u64,
}

impl Sink {
    fn open(dir: &Path) -> Sink {
        let _ = std::fs::create_dir_all(dir);
        let path = dir.join(FILE);
        let file = std::fs::OpenOptions::new().create(true).append(true).open(&path).ok();
        let len = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        Sink { dir: dir.to_path_buf(), file, len }
    }

    fn write(&mut self, line: &[u8]) {
        if self.len > 0 && self.len + line.len() as u64 > MAX_BYTES {
            self.file = None;
            rotate(&self.dir);
            *self = Sink::open(&self.dir.clone());
        }
        if let Some(f) = self.file.as_mut()
            && f.write_all(line).is_ok()
        {
            self.len += line.len() as u64;
        }
    }
}

/// Switchboard.log.4 goes, .3 becomes .4, ..., Switchboard.log becomes .1.
pub fn rotate(dir: &Path) {
    let name = |i: usize| if i == 0 { dir.join(FILE) } else { dir.join(format!("{FILE}.{i}")) };
    let _ = std::fs::remove_file(name(KEEP - 1));
    for i in (0..KEEP - 1).rev() {
        let _ = std::fs::rename(name(i), name(i + 1));
    }
}

pub struct Logger {
    filter: Filter,
    sink: Mutex<Sink>,
    /// Debug builds also print each line to stderr, as `cargo run` users expect.
    echo: bool,
}

impl Logger {
    pub fn new(dir: &Path, filter: Filter) -> Logger {
        Logger { filter, sink: Mutex::new(Sink::open(dir)), echo: cfg!(debug_assertions) }
    }

    pub fn line(record: &log::Record<'_>) -> String {
        let when = chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
        let module = record.module_path().unwrap_or(record.target());
        let msg = scrub(&record.args().to_string());
        format!("{when} {:<5} {module}: {msg}\n", record.level())
    }
}

impl log::Log for Logger {
    fn enabled(&self, m: &log::Metadata<'_>) -> bool {
        m.level() <= self.filter.level_for(m.target())
    }

    fn log(&self, record: &log::Record<'_>) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let line = Logger::line(record);
        if self.echo {
            eprint!("{line}");
        }
        if let Ok(mut s) = self.sink.lock() {
            s.write(line.as_bytes());
        }
    }

    fn flush(&self) {
        if let Ok(mut s) = self.sink.lock()
            && let Some(f) = s.file.as_mut()
        {
            let _ = f.flush();
        }
    }
}

/// Installs the logger once, at the start of main. No log folder (no HOME) leaves logging off, as before.
pub fn install() {
    let Some(dir) = dir() else { return };
    let filter = Filter::from_env();
    let max = filter.max();
    if log::set_boxed_logger(Box::new(Logger::new(&dir, filter))).is_ok() {
        log::set_max_level(max);
    }
}

/// The last `n` lines of the log, oldest first, reaching into Switchboard.log.1 when the current file is short.
pub fn tail(dir: &Path, n: usize) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for name in [format!("{FILE}.1"), FILE.to_string()] {
        if let Ok(f) = File::open(dir.join(name)) {
            lines.extend(std::io::BufReader::new(f).lines().map_while(Result::ok));
        }
    }
    let skip = lines.len().saturating_sub(n);
    lines.split_off(skip)
}

/// What the OS calls itself, for Copy diagnostics: "macOS 15.6", "Windows 10.0.26100", "Ubuntu 24.04.1 LTS".
pub fn os_version() -> String {
    let run = |cmd: &str, args: &[&str]| -> Option<String> {
        let mut c = std::process::Command::new(cmd);
        c.args(args);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            c.creation_flags(0x0800_0000); // CREATE_NO_WINDOW: no console flashes up
        }
        let out = c.output().ok()?;
        Some(String::from_utf8_lossy(&out.stdout).trim().to_string()).filter(|s| !s.is_empty())
    };
    if cfg!(target_os = "macos") {
        run("sw_vers", &["-productVersion"]).map_or_else(|| "macOS".into(), |v| format!("macOS {v}"))
    } else if cfg!(windows) {
        // "Microsoft Windows [Version 10.0.26100.1234]"
        run("cmd", &["/c", "ver"])
            .and_then(|v| v.split("Version ").nth(1).map(|s| s.trim_end_matches(']').to_string()))
            .map_or_else(|| "Windows".into(), |v| format!("Windows {v}"))
    } else {
        let pretty = std::fs::read_to_string("/etc/os-release").ok().and_then(|s| {
            s.lines().find_map(|l| l.strip_prefix("PRETTY_NAME=").map(|v| v.trim_matches('"').to_string()))
        });
        let kernel = run("uname", &["-r"]);
        match (pretty, kernel) {
            (Some(p), Some(k)) => format!("{p}, kernel {k}"),
            (Some(p), None) => p,
            (None, Some(k)) => format!("Linux, kernel {k}"),
            (None, None) => "Linux".into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use log::Log;

    fn temp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("sb-logs-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn filters_read_like_env_logger() {
        let f = Filter::parse(None);
        assert_eq!(f.level_for("sb_desktop::about"), LevelFilter::Info);
        let f = Filter::parse(Some("warn,sb_desktop=debug,sb_desktop::term=trace,gpui"));
        assert_eq!(f.level_for("sb_core::live"), LevelFilter::Warn);
        assert_eq!(f.level_for("sb_desktop"), LevelFilter::Debug);
        assert_eq!(f.level_for("sb_desktop::about"), LevelFilter::Debug);
        assert_eq!(f.level_for("sb_desktop::term::view"), LevelFilter::Trace);
        // A prefix only matches at a module boundary.
        assert_eq!(f.level_for("sb_desktopx"), LevelFilter::Warn);
        assert_eq!(f.level_for("gpui::window"), LevelFilter::Trace);
        assert_eq!(f.max(), LevelFilter::Trace);
        assert_eq!(Filter::parse(Some("off")).max(), LevelFilter::Off);
        // Junk is ignored, not fatal.
        assert_eq!(Filter::parse(Some("x=loud")).level_for("x"), LevelFilter::Info);
    }

    #[test]
    fn secrets_in_urls_are_hidden() {
        assert_eq!(
            redact_query("GET http://h:8131/api/x?token=abc123&n=2 failed"),
            "GET http://h:8131/api/x?token=***&n=2 failed"
        );
        assert_eq!(redact_query("ws://h/t?a=1&Ticket=zz#f"), "ws://h/t?a=1&Ticket=***#f");
        assert_eq!(redact_query("what? and & or = stay"), "what? and & or = stay");
    }

    #[test]
    fn the_home_folder_reads_as_a_tilde() {
        let Some(h) = home() else { return };
        let s = scrub(&format!("could not read {}/x/y.json", h.display()));
        assert_eq!(s, "could not read ~/x/y.json");
    }

    #[test]
    fn lines_have_time_level_and_module() {
        let dir = temp("line");
        let l = Logger::new(&dir, Filter::parse(Some("info")));
        l.log(
            &log::Record::builder()
                .level(log::Level::Warn)
                .module_path(Some("sb_desktop::state"))
                .target("sb_desktop::state")
                .args(format_args!("no live stream"))
                .build(),
        );
        l.log(
            &log::Record::builder()
                .level(log::Level::Debug)
                .target("sb_desktop::state")
                .args(format_args!("hidden"))
                .build(),
        );
        l.flush();
        let text = std::fs::read_to_string(dir.join(FILE)).unwrap();
        let line = text.lines().next().unwrap();
        assert!(line.ends_with(" WARN  sb_desktop::state: no live stream"), "{line}");
        assert!(line.as_bytes()[4] == b'-' && line.as_bytes()[10] == b' ', "{line}");
        assert!(!text.contains("hidden"), "{text}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rotates_past_the_limit_and_keeps_five() {
        let dir = temp("rotate");
        let mut sink = Sink::open(&dir);
        let line = vec![b'x'; 1 << 19]; // 512 KB: four fill a file
        for _ in 0..(4 * 8) {
            sink.write(&line);
        }
        let mut names: Vec<String> =
            std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        assert_eq!(
            names,
            ["Switchboard.log", "Switchboard.log.1", "Switchboard.log.2", "Switchboard.log.3", "Switchboard.log.4"]
        );
        for n in &names {
            assert!(std::fs::metadata(dir.join(n)).unwrap().len() <= MAX_BYTES, "{n}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tail_reaches_into_the_previous_file() {
        let dir = temp("tail");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(format!("{FILE}.1")), "a\nb\nc\n").unwrap();
        std::fs::write(dir.join(FILE), "d\ne\n").unwrap();
        assert_eq!(tail(&dir, 3), ["c", "d", "e"]);
        assert_eq!(tail(&dir, 200).len(), 5);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
