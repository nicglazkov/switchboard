//! Local terminal profiles: the shells this computer has, as Windows Terminal lists them. A profile is a program and
//! its arguments, never a chat; the desktop app runs it in a PTY of its own and shows it only as a tab.
//!
//! The rules are plain functions over facts (the text of /etc/shells, $SHELL, which files exist, what `wsl.exe -l -q`
//! printed), so they are tested here on every system. `detect` gathers the facts on the computer it runs on.

use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Profile {
    /// Stable across launches, so the default in settings still names it: the program's path, or "wsl:<distro>".
    pub id: String,
    /// What the menus show: "zsh", "PowerShell", "Ubuntu".
    pub name: String,
    pub program: String,
    pub args: Vec<String>,
}

/// Shells that are only there for scripts or to lock an account out, left out unless they are the person's $SHELL.
const SKIP: &[&str] = &["sh", "dash", "csh", "rbash", "git-shell", "nologin", "false", "true", "tmux", "screen"];

/// The order the common ones go in after $SHELL. Anything else from /etc/shells follows in the file's order.
const ORDER: &[&str] = &["zsh", "bash", "fish", "nu", "pwsh", "tcsh", "ksh", "mksh", "elvish", "xonsh"];

fn base(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

/// macOS and Linux. `etc_shells` is the text of /etc/shells, `env_shell` is $SHELL, `login` adds "-l" (a Mac
/// terminal starts login shells, so ~/.zprofile runs), and `exists` says whether a path is there. $SHELL comes first;
/// a name seen twice (/bin/bash and /usr/bin/bash) is kept once, at its first path.
pub fn unix_profiles(
    etc_shells: &str,
    env_shell: Option<&str>,
    login: bool,
    exists: &dyn Fn(&str) -> bool,
) -> Vec<Profile> {
    let mut paths: Vec<&str> =
        etc_shells.lines().map(|l| l.split('#').next().unwrap_or("").trim()).filter(|l| l.starts_with('/')).collect();
    let rank = |p: &str| ORDER.iter().position(|n| *n == base(p)).unwrap_or(ORDER.len());
    paths.sort_by_key(|p| rank(p));
    let own = env_shell.map(str::trim).filter(|s| s.starts_with('/'));
    let mut out: Vec<Profile> = Vec::new();
    for p in own.into_iter().chain(paths) {
        let name = base(p);
        let is_own = Some(p) == own;
        if name.is_empty() || (!is_own && SKIP.contains(&name)) || out.iter().any(|o| o.name == name) || !exists(p) {
            continue;
        }
        out.push(Profile {
            id: p.to_string(),
            name: name.to_string(),
            program: p.to_string(),
            args: if login { vec!["-l".into()] } else { Vec::new() },
        });
    }
    if out.is_empty() {
        // No /etc/shells and no $SHELL (a bare container): sh is always there.
        out.push(Profile { id: "/bin/sh".into(), name: "sh".into(), program: "/bin/sh".into(), args: Vec::new() });
    }
    out
}

/// What `detect` found on Windows.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WinFacts {
    /// pwsh.exe (PowerShell 7), when installed.
    pub pwsh: Option<String>,
    /// powershell.exe (Windows PowerShell 5), which every Windows has.
    pub powershell: Option<String>,
    pub cmd: Option<String>,
    /// Git for Windows' bash.exe.
    pub git_bash: Option<String>,
    /// The WSL distros, from `parse_wsl_list`.
    pub wsl: Vec<String>,
    pub wsl_exe: Option<String>,
}

/// Windows, in Windows Terminal's order: PowerShell 7 if installed, Windows PowerShell, Command Prompt, Git Bash,
/// then one per WSL distro.
pub fn windows_profiles(f: &WinFacts) -> Vec<Profile> {
    let mut out = Vec::new();
    let mut add = |id: String, name: &str, program: &str, args: &[&str]| {
        out.push(Profile {
            id,
            name: name.to_string(),
            program: program.to_string(),
            args: args.iter().map(|a| a.to_string()).collect(),
        })
    };
    if let Some(p) = &f.pwsh {
        add(p.clone(), "PowerShell", p, &["-NoLogo"]);
    }
    if let Some(p) = &f.powershell {
        add(p.clone(), "Windows PowerShell", p, &["-NoLogo"]);
    }
    if let Some(p) = &f.cmd {
        add(p.clone(), "Command Prompt", p, &[]);
    }
    if let Some(p) = &f.git_bash {
        add(p.clone(), "Git Bash", p, &["--login", "-i"]);
    }
    if let Some(w) = &f.wsl_exe {
        for d in &f.wsl {
            add(format!("wsl:{d}"), d, w, &["-d", d, "--cd", "~"]);
        }
    }
    out
}

/// `wsl.exe -l -q` prints UTF-16 (little endian, sometimes with a BOM), one distro a line with CRLF. Docker Desktop's
/// own distros are not shells anyone opens, so they are left out.
pub fn parse_wsl_list(bytes: &[u8]) -> Vec<String> {
    let text = if bytes.len() >= 2 && bytes.len().is_multiple_of(2) && bytes.iter().skip(1).step_by(2).any(|b| *b == 0)
    {
        let units: Vec<u16> = bytes.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect();
        String::from_utf16_lossy(&units)
    } else {
        String::from_utf8_lossy(bytes).into_owned()
    };
    let mut out: Vec<String> = Vec::new();
    for line in text.lines() {
        let name = line.trim_matches(|c: char| c.is_whitespace() || c == '\u{feff}' || c == '\0');
        if name.is_empty() || name.starts_with("docker-desktop") || out.iter().any(|o| o == name) {
            continue;
        }
        out.push(name.to_string());
    }
    out
}

/// The profile Cmd+T opens: the one saved in settings while it is still there, else the first.
pub fn pick_default<'a>(profiles: &'a [Profile], saved: Option<&str>) -> Option<&'a Profile> {
    saved.and_then(|id| profiles.iter().find(|p| p.id == id)).or_else(|| profiles.first())
}

/// What every local shell gets on top of the app's own environment. LANG only when it is missing, so a UTF-8 locale
/// is there when the app was started from the Dock (which sets none).
pub fn env(lang: Option<&str>) -> Vec<(&'static str, String)> {
    let mut v = vec![
        ("TERM", "xterm-256color".to_string()),
        ("COLORTERM", "truecolor".to_string()),
        ("TERM_PROGRAM", "Switchboard".to_string()),
    ];
    if lang.is_none_or(|l| l.trim().is_empty()) {
        v.push(("LANG", "en_US.UTF-8".to_string()));
    }
    v
}

/// The profiles on this computer. Reads files and, on Windows, runs `wsl.exe -l -q` once, so it is called off the UI
/// thread or once at start.
pub fn detect() -> Vec<Profile> {
    #[cfg(windows)]
    {
        detect_windows()
    }
    #[cfg(not(windows))]
    {
        let etc = std::fs::read_to_string("/etc/shells").unwrap_or_default();
        let shell = std::env::var("SHELL").ok();
        unix_profiles(&etc, shell.as_deref(), cfg!(target_os = "macos"), &|p| Path::new(p).exists())
    }
}

#[cfg(windows)]
fn detect_windows() -> Vec<Profile> {
    use std::os::windows::process::CommandExt;
    const NO_WINDOW: u32 = 0x0800_0000;
    let env = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
    let file = |p: String| Path::new(&p).is_file().then_some(p);
    let on_path = |exe: &str| {
        env("PATH").and_then(|path| {
            std::env::split_paths(&path).map(|d| d.join(exe)).find(|p| p.is_file()).map(|p| p.display().to_string())
        })
    };
    let root = env("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
    let sys = format!("{root}\\System32");
    let pf = env("ProgramFiles").unwrap_or_else(|| "C:\\Program Files".into());
    let pwsh = on_path("pwsh.exe").or_else(|| file(format!("{pf}\\PowerShell\\7\\pwsh.exe")));
    let git_bash = file(format!("{pf}\\Git\\bin\\bash.exe"))
        .or_else(|| env("LOCALAPPDATA").and_then(|l| file(format!("{l}\\Programs\\Git\\bin\\bash.exe"))));
    let wsl_exe = file(format!("{sys}\\wsl.exe"));
    let wsl = wsl_exe
        .as_ref()
        .and_then(|w| std::process::Command::new(w).args(["-l", "-q"]).creation_flags(NO_WINDOW).output().ok())
        .filter(|o| o.status.success())
        .map(|o| parse_wsl_list(&o.stdout))
        .unwrap_or_default();
    windows_profiles(&WinFacts {
        pwsh,
        powershell: file(format!("{sys}\\WindowsPowerShell\\v1.0\\powershell.exe")),
        cmd: file(format!("{sys}\\cmd.exe")).or_else(|| env("ComSpec")),
        git_bash,
        wsl,
        wsl_exe,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const ETC: &str = "# List of acceptable shells for chpass(1).\n\n/bin/bash\n/bin/csh\n/bin/dash\n/bin/ksh\n/bin/sh\n\
                       /bin/tcsh\n/bin/zsh\n/opt/homebrew/bin/fish  # added by brew\n/usr/bin/bash\n";

    fn names(v: &[Profile]) -> Vec<&str> {
        v.iter().map(|p| p.name.as_str()).collect()
    }

    #[test]
    fn shell_first_then_common_order() {
        let v = unix_profiles(ETC, Some("/bin/zsh"), true, &|_| true);
        assert_eq!(names(&v), ["zsh", "bash", "fish", "tcsh", "ksh"]);
        assert_eq!(v[0].id, "/bin/zsh");
        assert_eq!(v[0].args, ["-l"]);
        // /usr/bin/bash is the same shell as /bin/bash.
        assert_eq!(v[1].program, "/bin/bash");
    }

    #[test]
    fn own_shell_is_kept_even_when_skipped_or_unlisted() {
        let v = unix_profiles(ETC, Some("/bin/sh"), false, &|_| true);
        assert_eq!(v[0].name, "sh");
        assert!(v[0].args.is_empty());
        let v = unix_profiles(ETC, Some("/home/you/.cargo/bin/nu"), false, &|_| true);
        assert_eq!(names(&v)[..2], ["nu", "zsh"]);
    }

    #[test]
    fn missing_files_are_left_out() {
        let v = unix_profiles(ETC, Some("/bin/zsh"), false, &|p| p != "/opt/homebrew/bin/fish" && p != "/bin/bash");
        // /usr/bin/bash stands in for the missing /bin/bash.
        assert_eq!(names(&v), ["zsh", "bash", "tcsh", "ksh"]);
        assert_eq!(v[1].program, "/usr/bin/bash");
    }

    #[test]
    fn nothing_found_falls_back_to_sh() {
        let v = unix_profiles("", None, false, &|_| false);
        assert_eq!(names(&v), ["sh"]);
        let v = unix_profiles("", Some("  "), false, &|_| true);
        assert_eq!(names(&v), ["sh"]);
    }

    #[test]
    fn windows_order() {
        let f = WinFacts {
            pwsh: Some("C:\\Program Files\\PowerShell\\7\\pwsh.exe".into()),
            powershell: Some("C:\\Windows\\System32\\WindowsPowerShell\\v1.0\\powershell.exe".into()),
            cmd: Some("C:\\Windows\\System32\\cmd.exe".into()),
            git_bash: Some("C:\\Program Files\\Git\\bin\\bash.exe".into()),
            wsl: vec!["Ubuntu".into(), "Debian".into()],
            wsl_exe: Some("C:\\Windows\\System32\\wsl.exe".into()),
        };
        let v = windows_profiles(&f);
        assert_eq!(names(&v), ["PowerShell", "Windows PowerShell", "Command Prompt", "Git Bash", "Ubuntu", "Debian"]);
        assert_eq!(v[4].id, "wsl:Ubuntu");
        assert_eq!(v[4].args, ["-d", "Ubuntu", "--cd", "~"]);
        assert_eq!(v[3].args, ["--login", "-i"]);
    }

    #[test]
    fn windows_without_pwsh_starts_with_windows_powershell() {
        let f = WinFacts {
            powershell: Some("powershell.exe".into()),
            cmd: Some("cmd.exe".into()),
            wsl: vec!["Ubuntu".into()],
            ..Default::default()
        };
        // No wsl.exe, no WSL rows even with a list.
        assert_eq!(names(&windows_profiles(&f)), ["Windows PowerShell", "Command Prompt"]);
    }

    fn utf16(s: &str, bom: bool) -> Vec<u8> {
        let mut out = Vec::new();
        if bom {
            out.extend([0xff, 0xfe]);
        }
        for u in s.encode_utf16() {
            out.extend(u.to_le_bytes());
        }
        out
    }

    #[test]
    fn wsl_list_utf16() {
        let raw = "Ubuntu-22.04\r\ndocker-desktop\r\ndocker-desktop-data\r\nDebian\r\n\r\n";
        assert_eq!(parse_wsl_list(&utf16(raw, false)), ["Ubuntu-22.04", "Debian"]);
        assert_eq!(parse_wsl_list(&utf16(raw, true)), ["Ubuntu-22.04", "Debian"]);
        // Some builds pad with NULs.
        assert_eq!(parse_wsl_list(&utf16("Ubuntu\0\r\n", false)), ["Ubuntu"]);
    }

    #[test]
    fn wsl_list_utf8_and_empty() {
        assert_eq!(parse_wsl_list(b"Ubuntu\nUbuntu\n"), ["Ubuntu"]);
        assert!(parse_wsl_list(b"").is_empty());
    }

    #[test]
    fn default_is_saved_while_it_exists() {
        let v = unix_profiles(ETC, Some("/bin/zsh"), false, &|_| true);
        assert_eq!(pick_default(&v, None).unwrap().name, "zsh");
        assert_eq!(pick_default(&v, Some("/bin/bash")).unwrap().name, "bash");
        assert_eq!(pick_default(&v, Some("/gone/xonsh")).unwrap().name, "zsh");
        assert!(pick_default(&[], Some("/bin/zsh")).is_none());
    }

    #[test]
    fn env_sets_lang_only_when_missing() {
        fn keys(v: Vec<(&'static str, String)>) -> Vec<&'static str> {
            v.into_iter().map(|(k, _)| k).collect()
        }
        assert_eq!(keys(env(None)), ["TERM", "COLORTERM", "TERM_PROGRAM", "LANG"]);
        assert_eq!(keys(env(Some(""))), ["TERM", "COLORTERM", "TERM_PROGRAM", "LANG"]);
        assert_eq!(keys(env(Some("de_DE.UTF-8"))), ["TERM", "COLORTERM", "TERM_PROGRAM"]);
        assert_eq!(env(None)[0].1, "xterm-256color");
    }
}
