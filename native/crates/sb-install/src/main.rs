//! Switchboard's installer, updater and uninstaller in one program: "Install Switchboard" on the disk image, and
//! Switchboard-Setup-<version>.exe on Windows (this program with the app's files attached, see pack.rs).
//!
//!   switchboard-install                      the installer's window
//!   switchboard-install --uninstall          the uninstaller's window
//!   switchboard-install --quiet [options]    no window: one line per step, non-zero exit on failure
//!   switchboard-install --make-setup <out.exe> <file>...   attach the app's files to a copy of this program
//!
//! Options (all choices start off): --app <Switchboard.app or folder> --dest <folder> --open-at-login --add-to-dock
//! --desktop-shortcut --open --force-quit (stop a copy that does not quit when asked) --remove-data (with
//! --uninstall) --eject. Nothing is installed but the app itself; no admin password is asked for.

// A window, not a console, when opened from Explorer. --quiet attaches to the console it was started from.
#![cfg_attr(windows, windows_subsystem = "windows")]

mod brand;
mod flow;
#[cfg_attr(not(windows), allow(dead_code))]
mod pack;
mod platform;
mod quiet;
mod record;
mod swap;
mod ui;
mod uninstall;
mod version;

use std::path::PathBuf;

use flow::Opts;

/// What to do, from the command line.
#[derive(Debug, Default)]
pub struct Args {
    pub quiet: bool,
    pub uninstall: bool,
    pub opts: Opts,
}

fn parse(mut it: impl Iterator<Item = String>) -> Result<Args, String> {
    let mut a = Args::default();
    while let Some(arg) = it.next() {
        let mut path = |name: &str| it.next().map(PathBuf::from).ok_or_else(|| format!("{name} needs a path"));
        match arg.as_str() {
            "--quiet" | "-q" => a.quiet = true,
            "--uninstall" => a.uninstall = true,
            "--app" => a.opts.app = Some(path("--app")?),
            "--dest" => a.opts.dest = Some(path("--dest")?),
            "--open-at-login" => a.opts.choices.open_at_login = true,
            "--add-to-dock" => a.opts.choices.add_to_dock = true,
            "--desktop-shortcut" => a.opts.choices.desktop_shortcut = true,
            "--open" => a.opts.open = true,
            "--force-quit" => a.opts.force_quit = true,
            "--remove-data" => a.opts.remove_data = true,
            "--eject" => a.opts.eject = true,
            // macOS adds this when an app is opened from Finder the first time.
            s if s.starts_with("-psn_") => {}
            other => return Err(format!("unknown option {other} (see --help)")),
        }
    }
    Ok(a)
}

const HELP: &str = "usage: switchboard-install [--quiet] [--uninstall [--remove-data]] [--app PATH] [--dest DIR]
                          [--open-at-login] [--add-to-dock] [--desktop-shortcut] [--open] [--force-quit] [--eject]
       switchboard-install --make-setup OUT.exe FILE...";

/// --make-setup: a copy of this program with `files` attached, plus this program again as the uninstaller that
/// stays in the app's folder (platform/windows.rs).
fn make_setup(args: &[String]) -> Result<String, String> {
    let (out, files) = args.split_first().ok_or("--make-setup needs an output file and the app's files")?;
    if files.is_empty() {
        return Err("--make-setup needs the app's files".into());
    }
    let me = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut list: Vec<(String, &std::path::Path)> = vec![];
    for f in files {
        let p = std::path::Path::new(f);
        let name = p.file_name().and_then(|n| n.to_str()).ok_or_else(|| format!("{f} is not a file"))?;
        list.push((name.to_string(), p));
    }
    let uninstaller = platform::windows::uninstaller_name();
    list.push((uninstaller, &me));
    let size = pack::make(&me, &list, std::path::Path::new(out)).map_err(|e| format!("{out}: {e}"))?;
    Ok(format!("{out}: {size} bytes, {} files attached", list.len()))
}

fn main() {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    #[cfg(windows)]
    platform::windows::attach_console();
    if raw.first().map(String::as_str) == Some("--make-setup") {
        match make_setup(&raw[1..]) {
            Ok(said) => println!("{said}"),
            Err(e) => {
                eprintln!("switchboard-install: {e}");
                std::process::exit(1);
            }
        }
        return;
    }
    if raw.iter().any(|a| a == "--help" || a == "-h") {
        println!("{HELP}");
        return;
    }
    if raw.first().map(String::as_str) == Some("--version") {
        println!("{} installer {}", brand::NAME, env!("CARGO_PKG_VERSION"));
        return;
    }
    // The helper a finished install starts to eject the disk image once the installer has quit (platform/mac.rs).
    #[cfg(target_os = "macos")]
    if raw.first().map(String::as_str) == Some("--eject-after") {
        let pid = raw.get(1).and_then(|p| p.parse().ok()).unwrap_or(0);
        if let Some(mount) = raw.get(2) {
            platform::mac_eject_helper(pid, std::path::Path::new(mount), raw.get(3).map(std::path::Path::new));
        }
        return;
    }
    let mut args = match parse(raw.into_iter()) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("switchboard-install: {e}\n{HELP}");
            std::process::exit(2);
        }
    };
    // The uninstaller Windows keeps in the app's folder: opening it means uninstalling that copy.
    if args.opts.dest.is_none()
        && let Some(app) = platform::native().own_install()
    {
        args.uninstall = true;
        args.opts.dest = app.parent().map(std::path::Path::to_path_buf);
    }
    if args.quiet {
        std::process::exit(quiet::run(&args));
    }
    ui::run(args);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> Result<Args, String> {
        parse(s.split_whitespace().map(str::to_string))
    }

    #[test]
    fn every_choice_starts_off() {
        let a = p("").unwrap();
        assert!(!a.quiet && !a.uninstall);
        assert_eq!(a.opts.choices, record::Choices::default());
        assert!(!a.opts.open && !a.opts.force_quit && !a.opts.remove_data && !a.opts.eject);
    }

    #[test]
    fn flags() {
        let a = p("--quiet --dest /tmp/apps --app /Volumes/S/Switchboard.app --open-at-login --force-quit").unwrap();
        assert!(a.quiet && a.opts.force_quit && a.opts.choices.open_at_login && !a.opts.choices.add_to_dock);
        assert_eq!(a.opts.dest, Some(PathBuf::from("/tmp/apps")));
        assert!(p("--dest").is_err());
        assert!(p("--bogus").unwrap_err().contains("--bogus"));
        assert!(p("-psn_0_12345").is_ok());
        assert!(p("--desktop-shortcut").unwrap().opts.choices.desktop_shortcut);
    }

    #[test]
    fn make_setup_needs_files() {
        assert!(make_setup(&[]).is_err());
        assert!(make_setup(&["out.exe".into()]).is_err());
    }
}
