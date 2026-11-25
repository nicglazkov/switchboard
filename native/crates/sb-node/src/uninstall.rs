//! `switchboard-node uninstall`: stop the node, take it out of what starts it at login, and remove the program and
//! its settings (node.json, which holds the token). What is to be done is worked out first as a list of steps, the
//! same for every OS to read and test; then the list is carried out and each thing removed is said.
//!
//! The hub is not told: this computer stays in Switchboard's list, offline, until it is removed in Settings,
//! Machines. Logs are left where they are.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::{LABEL, TASK};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Os {
    Mac,
    Linux,
    Windows,
}

impl Os {
    pub fn here() -> Os {
        if cfg!(target_os = "macos") {
            Os::Mac
        } else if cfg!(windows) {
            Os::Windows
        } else {
            Os::Linux
        }
    }
}

/// Where the node's things are on this computer.
pub struct Places {
    pub os: Os,
    pub home: PathBuf,
    /// where `install` put the program
    pub program: PathBuf,
    /// node.json
    pub config: PathBuf,
    /// node.lock, beside it
    pub lock: PathBuf,
    /// the user's number, for launchd (Mac)
    pub uid: String,
    /// this process, which is not to stop itself (Windows)
    pub pid: u32,
    /// True when the program at `program` is the one running this (Windows cannot delete that one).
    pub running_from_program: bool,
    /// Programs an update moved aside, still in the folder (Windows).
    pub old: Vec<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// A command. One that fails had nothing to do (no such service, nothing running), and nothing is said then.
    /// When it works `did` is said, unless that is empty or what it printed does not have `only_if_says` in it.
    Run { cmd: &'static str, args: Vec<String>, did: String, only_if_says: &'static str },
    /// A file. One that is not there is passed over without a word.
    Remove { what: &'static str, path: PathBuf },
    /// A file that has to stay for now, and why.
    Leave { path: PathBuf, why: &'static str },
}

fn run(cmd: &'static str, args: &[&str], did: &str) -> Step {
    Step::Run { cmd, args: args.iter().map(|a| a.to_string()).collect(), did: did.into(), only_if_says: "" }
}

/// A path as a pattern for `pkill -f`, which reads its argument as a regular expression.
fn literal(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if r"\.^$*+?()[]{}|".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// Everything `uninstall` does, in order.
///
/// On a Mac and on Linux the node is stopped last, after its files are gone. `uninstall` may itself be running
/// under the node (asked for from Switchboard, in a terminal the node opened), and stopping the service stops
/// everything it started: stopped first, the uninstaller would end there with the program and the token still
/// in place. Windows has to stop the node first, since a program that is running cannot be deleted there, and
/// stopping it there does not stop what it started.
pub fn plan(p: &Places) -> Vec<Step> {
    let program = p.program.display().to_string();
    // (a node started by hand, or by the installer on a system with no service manager)
    let by_hand = || run("pkill", &["-f", &format!("{} run", literal(&program))], "Stopped the node that was running");
    let (mut steps, mut last) = (vec![], vec![]);
    match p.os {
        Os::Mac => {
            let plist = p.home.join(format!("Library/LaunchAgents/{LABEL}.plist"));
            steps.push(Step::Remove { what: "its login item", path: plist });
            let target = format!("gui/{}/{LABEL}", p.uid);
            last.push(run("launchctl", &["bootout", &target], &format!("Stopped the node ({LABEL})")));
            last.push(by_hand());
        }
        Os::Linux => {
            // (out of what starts at login, while its file is still there to say where that is)
            steps.push(run("systemctl", &["--user", "disable", "switchboard-node"], ""));
            let unit = p.home.join(".config/systemd/user/switchboard-node.service");
            steps.push(Step::Remove { what: "its user service", path: unit });
            steps.push(run("systemctl", &["--user", "daemon-reload"], ""));
            let did = "Stopped the user service switchboard-node";
            last.push(run("systemctl", &["--user", "stop", "switchboard-node"], did));
            last.push(by_hand());
        }
        Os::Windows => {
            steps.push(run("schtasks", &["/End", "/TN", TASK], ""));
            steps.push(run("schtasks", &["/Delete", "/TN", TASK, "/F"], &format!("Removed the logon task {TASK}")));
            // The task only starts the node, through a launcher: ending it leaves the node running. Every node but
            // this process is stopped (taskkill reports success with nothing to stop, hence the word looked for).
            let name = p.program.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            steps.push(Step::Run {
                cmd: "taskkill",
                args: ["/F", "/IM", &name, "/FI", &format!("PID ne {}", p.pid)].map(String::from).to_vec(),
                did: "Stopped the node that was running".into(),
                only_if_says: "SUCCESS",
            });
            steps.push(Step::Remove {
                what: "the launcher its logon task ran",
                path: p.program.with_file_name("switchboard-node.vbs"),
            });
            steps.push(Step::Remove {
                what: "a leftover of its logon task",
                path: p.program.with_file_name("switchboard-node-task.xml"),
            });
            for old in &p.old {
                steps.push(Step::Remove { what: "an older copy of the program", path: old.clone() });
            }
        }
    }
    if p.os == Os::Windows && p.running_from_program {
        let why = "it is the program doing this, and Windows will not delete a program while it runs. Delete it \
                   yourself now";
        steps.push(Step::Leave { path: p.program.clone(), why });
    } else {
        steps.push(Step::Remove { what: "the program", path: p.program.clone() });
    }
    steps.push(Step::Remove { what: "its settings and the token for the server", path: p.config.clone() });
    steps.push(Step::Remove { what: "its lock file", path: p.lock.clone() });
    steps.extend(last);
    steps
}

/// Runs a command with its arguments: what it printed, or why it failed.
pub type Sh<'a> = &'a dyn Fn(&str, &[String]) -> Result<String, String>;

/// How long a file that will not go is tried again for on Windows. A node that was just stopped still holds its
/// program for a moment after taskkill returns, and deleting it in that moment is refused (a sharing violation).
pub const WINDOWS_PATIENCE: Duration = Duration::from_secs(2);

/// Removes a file, trying again for up to `patience` while that fails (a file that is not there is never tried
/// again: there is nothing to wait for).
fn remove(path: &Path, patience: Duration) -> std::io::Result<()> {
    let began = Instant::now();
    loop {
        match std::fs::remove_file(path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound && began.elapsed() < patience => {
                std::thread::sleep(Duration::from_millis(100));
            }
            done => return done,
        }
    }
}

/// Does the steps. What was done, one line each, and whether something that should have gone is still there.
/// `patience` is how long a file that cannot be removed is tried again for (`WINDOWS_PATIENCE` there, none
/// elsewhere).
#[cfg(test)]
pub fn carry_out(steps: &[Step], sh: Sh, patience: Duration) -> (Vec<String>, bool) {
    let mut said = vec![];
    let failed = carry_out_saying(steps, sh, patience, &mut |line| said.push(line));
    (said, failed)
}

/// `carry_out`, with each line handed over as soon as its step is done: the last step may be the end of this
/// process too (see `plan`), and what was done before it should have been said by then.
pub fn carry_out_saying(steps: &[Step], sh: Sh, patience: Duration, say: &mut dyn FnMut(String)) -> bool {
    let mut failed = false;
    for step in steps {
        match step {
            Step::Run { cmd, args, did, only_if_says } => {
                if let Ok(out) = sh(cmd, args)
                    && !did.is_empty()
                    && out.contains(only_if_says)
                {
                    say(format!("{did}."));
                }
            }
            Step::Remove { what, path } => match remove(path, patience) {
                Ok(()) => say(format!("Removed {what}: {}", path.display())),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => {
                    failed = true;
                    say(format!("Could not remove {what}, {} ({e}).", path.display()));
                }
            },
            Step::Leave { path, why } => {
                if path.exists() {
                    say(format!("Left {}: {why}.", path.display()));
                }
            }
        }
    }
    failed
}

/// The moved-aside programs ("switchboard-node.<pid>.old") in the folder of `program`.
pub fn old_programs(program: &Path) -> Vec<PathBuf> {
    let Some(dir) = program.parent() else { return vec![] };
    let stem = program.file_stem().map(|s| s.to_string_lossy().to_lowercase()).unwrap_or_default();
    let mut out: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| {
            let name = e.file_name().to_string_lossy().to_lowercase();
            name.strip_prefix(&format!("{stem}.")).is_some_and(|r| r.ends_with(".old"))
        })
        .map(|e| e.path())
        .collect();
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn places(os: Os, home: &Path) -> Places {
        let program = match os {
            Os::Windows => home.join("AppData/Local/Switchboard/switchboard-node.exe"),
            _ => home.join(".local/bin/switchboard-node"),
        };
        Places {
            os,
            home: home.into(),
            program,
            config: home.join(".config/switchboard/node.json"),
            lock: home.join(".config/switchboard/node.lock"),
            uid: "501".into(),
            pid: 4242,
            running_from_program: false,
            old: vec![],
        }
    }

    /// The plan as short lines: "run <cmd> <args>", "rm <path under home>", "leave <path under home>".
    fn lines(p: &Places) -> Vec<String> {
        let rel = |q: &Path| q.strip_prefix(&p.home).unwrap().to_string_lossy().replace('\\', "/");
        plan(p)
            .iter()
            .map(|s| match s {
                Step::Run { cmd, args, .. } => format!("run {cmd} {}", args.join(" ")),
                Step::Remove { path, .. } => format!("rm {}", rel(path)),
                Step::Leave { path, .. } => format!("leave {}", rel(path)),
            })
            .collect()
    }

    #[test]
    fn what_uninstall_does_on_each_system() {
        let home = Path::new("/Users/you");
        assert_eq!(
            lines(&places(Os::Mac, home)),
            [
                "rm Library/LaunchAgents/com.switchboard.node.plist",
                "rm .local/bin/switchboard-node",
                "rm .config/switchboard/node.json",
                "rm .config/switchboard/node.lock",
                // last: run from a terminal the node opened, this is where the uninstaller itself may end
                "run launchctl bootout gui/501/com.switchboard.node",
                r"run pkill -f /Users/you/\.local/bin/switchboard-node run",
            ]
        );
        let home = Path::new("/home/you");
        assert_eq!(
            lines(&places(Os::Linux, home)),
            [
                "run systemctl --user disable switchboard-node",
                "rm .config/systemd/user/switchboard-node.service",
                "run systemctl --user daemon-reload",
                "rm .local/bin/switchboard-node",
                "rm .config/switchboard/node.json",
                "rm .config/switchboard/node.lock",
                "run systemctl --user stop switchboard-node",
                r"run pkill -f /home/you/\.local/bin/switchboard-node run",
            ]
        );
        let mut win = places(Os::Windows, home);
        win.old = vec![home.join("AppData/Local/Switchboard/switchboard-node.77.old")];
        let common = [
            "run schtasks /End /TN SwitchboardNode",
            "run schtasks /Delete /TN SwitchboardNode /F",
            "run taskkill /F /IM switchboard-node.exe /FI PID ne 4242",
            "rm AppData/Local/Switchboard/switchboard-node.vbs",
            "rm AppData/Local/Switchboard/switchboard-node-task.xml",
            "rm AppData/Local/Switchboard/switchboard-node.77.old",
        ];
        let ends = |program: &str| {
            let mut all: Vec<String> = common.iter().map(|s| s.to_string()).collect();
            all.extend(
                [program, "rm .config/switchboard/node.json", "rm .config/switchboard/node.lock"].map(String::from),
            );
            all
        };
        assert_eq!(lines(&win), ends("rm AppData/Local/Switchboard/switchboard-node.exe"));
        // run from the installed copy itself, Windows keeps that one file, and uninstall says so
        win.running_from_program = true;
        assert_eq!(lines(&win), ends("leave AppData/Local/Switchboard/switchboard-node.exe"));
        // nowhere else is the program ever left
        let mut mac = places(Os::Mac, home);
        mac.running_from_program = true;
        assert!(lines(&mac).contains(&"rm .local/bin/switchboard-node".to_string()));
    }

    #[test]
    fn uninstall_removes_what_is_there_and_says_only_that() {
        let home = std::env::temp_dir().join(format!("sb-node-uninstall-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let p = places(Os::Linux, &home);
        let unit = home.join(".config/systemd/user/switchboard-node.service");
        for f in [&p.program, &p.config, &unit] {
            std::fs::create_dir_all(f.parent().unwrap()).unwrap();
            std::fs::write(f, "x").unwrap();
        }
        let other = home.join(".config/switchboard/config.json");
        std::fs::write(&other, "{}").unwrap();
        let ran = std::sync::Mutex::new(vec![]);
        // the service was there to stop; nothing ran outside it (pkill found nothing)
        let sh = |cmd: &str, args: &[String]| {
            ran.lock().unwrap().push(format!("{cmd} {}", args.join(" ")));
            if cmd == "pkill" { Err("nothing".to_string()) } else { Ok(String::new()) }
        };
        let (said, failed) = carry_out(&plan(&p), &sh, Duration::ZERO);
        assert!(!failed, "{said:?}");
        assert_eq!(ran.lock().unwrap().len(), 4);
        assert_eq!(
            said,
            [
                format!("Removed its user service: {}", unit.display()),
                format!("Removed the program: {}", p.program.display()),
                format!("Removed its settings and the token for the server: {}", p.config.display()),
                "Stopped the user service switchboard-node.".to_string(),
            ],
            "the lock file was never there, and the reload is not news"
        );
        assert!(!p.program.exists() && !p.config.exists() && !unit.exists());
        assert!(other.exists(), "the server's and the helper's settings in the same folder are not the node's");
        // a second time there is nothing to do, and nothing is said
        let (said, failed) = carry_out(&plan(&p), &|_, _| Err("no such unit".into()), Duration::ZERO);
        assert_eq!((said, failed), (vec![], false));
        // something that will not go is said, and counts as not done (a folder where the file should be)
        std::fs::create_dir_all(&p.config).unwrap();
        let (said, failed) = carry_out(&plan(&p), &|_, _| Err("no".into()), Duration::ZERO);
        assert!(failed && said.len() == 1 && said[0].starts_with("Could not remove its settings"), "{said:?}");
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn an_uninstall_ended_by_stopping_its_own_node_has_removed_and_said_everything_by_then() {
        // run from a terminal the node opened: stopping the service is the end of the uninstaller too
        let home = std::env::temp_dir().join(format!("sb-node-uninstall-own-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        for os in [Os::Linux, Os::Mac] {
            let p = places(os, &home);
            for f in [&p.program, &p.config, &p.lock] {
                std::fs::create_dir_all(f.parent().unwrap()).unwrap();
                std::fs::write(f, "x").unwrap();
            }
            let said = std::sync::Mutex::new(vec![]);
            let at_the_stop = std::sync::Mutex::new(None);
            let sh = |cmd: &str, args: &[String]| {
                if args.iter().any(|a| a == "stop" || a == "bootout") {
                    // where this process would be killed: what is on disk, and what has been said, so far
                    let left = [&p.program, &p.config, &p.lock].iter().filter(|f| f.exists()).count();
                    *at_the_stop.lock().unwrap() = Some((left, said.lock().unwrap().len()));
                }
                let _ = cmd;
                Ok(String::new())
            };
            carry_out_saying(&plan(&p), &sh, Duration::ZERO, &mut |l| said.lock().unwrap().push(l));
            assert_eq!(*at_the_stop.lock().unwrap(), Some((0, 3)), "{os:?}: {:?}", said.lock().unwrap());
        }
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn a_command_that_reports_success_with_nothing_done_is_not_news() {
        let step = Step::Run { cmd: "taskkill", args: vec![], did: "Stopped the node".into(), only_if_says: "SUCCESS" };
        let none = |_: &str, _: &[String]| Ok("INFO: No tasks running with the specified criteria.".to_string());
        assert_eq!(carry_out(std::slice::from_ref(&step), &none, Duration::ZERO).0, Vec::<String>::new());
        let one = |_: &str, _: &[String]| Ok("SUCCESS: The process with PID 77 has been terminated.".to_string());
        assert_eq!(carry_out(&[step], &one, Duration::ZERO).0, ["Stopped the node."]);
    }

    #[test]
    fn a_file_still_held_for_a_moment_is_tried_again() {
        // What Windows does with the program of a node that was just stopped: for a moment it will not be
        // deleted. Here a folder stands where the file should be for that moment, which fails the same way.
        let dir = std::env::temp_dir().join(format!("sb-node-held-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("switchboard-node.exe");
        let held = |path: &Path| {
            std::fs::create_dir_all(path).unwrap();
            let path = path.to_path_buf();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(350));
                std::fs::remove_dir(&path).unwrap();
                std::fs::write(&path, "x").unwrap();
            })
        };
        let step = [Step::Remove { what: "the program", path: path.clone() }];
        let nothing = |_: &str, _: &[String]| Err(String::new());
        // with no patience it is reported as not removed, as it was
        let let_go = held(&path);
        let (said, failed) = carry_out(&step, &nothing, Duration::ZERO);
        assert!(failed && said[0].starts_with("Could not remove the program"), "{said:?}");
        let_go.join().unwrap();
        std::fs::remove_file(&path).unwrap();
        // with it, the file goes once it is let go, well inside the two seconds
        let let_go = held(&path);
        let began = Instant::now();
        let (said, failed) = carry_out(&step, &nothing, WINDOWS_PATIENCE);
        let_go.join().unwrap();
        assert_eq!((said, failed), (vec![format!("Removed the program: {}", path.display())], false));
        assert!(!path.exists() && began.elapsed() < Duration::from_millis(1500), "{:?}", began.elapsed());
        // one that never lets go is given up on after the patience, and a missing file is not waited for
        std::fs::create_dir_all(&path).unwrap();
        let began = Instant::now();
        let (_, failed) = carry_out(&step, &nothing, Duration::from_millis(300));
        assert!(failed && began.elapsed() >= Duration::from_millis(300) && began.elapsed() < Duration::from_secs(2));
        std::fs::remove_dir(&path).unwrap();
        let began = Instant::now();
        assert_eq!(carry_out(&step, &nothing, WINDOWS_PATIENCE), (vec![], false));
        assert!(began.elapsed() < Duration::from_millis(100));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn older_copies_of_the_program_are_found_beside_it() {
        let dir = std::env::temp_dir().join(format!("sb-node-old-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for n in
            ["switchboard-node.exe", "switchboard-node.812.old", "Switchboard-Node.9.OLD", "switchboard.ps1", "x.old"]
        {
            std::fs::write(dir.join(n), "").unwrap();
        }
        let mut names: Vec<String> = old_programs(&dir.join("switchboard-node.exe"))
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().to_lowercase())
            .collect();
        names.sort();
        assert_eq!(names, ["switchboard-node.812.old", "switchboard-node.9.old"]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
