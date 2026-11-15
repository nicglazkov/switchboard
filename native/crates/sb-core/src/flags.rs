//! v2: the New session options as flags, checked the way the web app checks them before the server checks them
//! again (and the machine's agent once more, against its own `claude --help`).

use std::collections::HashMap;

use crate::api::ApiSetup;

/// Permissions: the flag's value and the button's name. "" sends no flag, so the chat starts in whatever mode that
/// machine's own Claude Code settings say (it may not ask first); "default" asks every time (the server writes it
/// the way that machine's Claude Code takes it, "manual" on a newer one). The same six modes as the flags drawer.
pub const PERMS: [(&str, &str); 7] = [
    ("", "Machine default"),
    ("default", "Asks first"),
    ("acceptEdits", "Accept edits"),
    ("plan", "Plan"),
    ("auto", "Auto"),
    ("bypassPermissions", "Bypass"),
    ("dontAsk", "Never asks"),
];
/// Flags whose value may be left out.
pub const OPTIONAL: [&str; 1] = ["--worktree"];
/// Effort: "" leaves it to the machine; Max and UltraCode set their own.
pub const EFFORTS: [(&str, &str); 5] =
    [("", "Default"), ("low", "Low"), ("medium", "Medium"), ("high", "High"), ("xhigh", "Extra high")];
/// Extra folders (--add-dir) at most.
pub const DIRS_MAX: usize = 8;

/// The options under the message: Claude in Chrome, permissions, effort, extra folders and anything else typed.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Opts {
    pub chrome: bool,
    pub perm: String,
    pub effort: String,
    pub dirs: Vec<String>,
    pub other: String,
}

impl Opts {
    /// The options as the server takes them: [["--chrome"], ["--permission-mode", "plan"], ["--add-dir", "notes"]].
    pub fn flags(&self) -> Vec<Vec<String>> {
        let mut f = Vec::new();
        if self.chrome {
            f.push(vec!["--chrome".to_string()]);
        }
        if !self.perm.is_empty() {
            f.push(vec!["--permission-mode".to_string(), self.perm.clone()]);
        }
        for d in &self.dirs {
            f.push(vec!["--add-dir".to_string(), d.clone()]);
        }
        f
    }

    /// What a setup (the last one used, or a preset) sets here. Flags without a control of their own go to Other.
    pub fn from_setup(s: &ApiSetup) -> Opts {
        let mut o = Opts {
            effort: s
                .effort
                .clone()
                .filter(|e| EFFORTS.iter().any(|x| !x.0.is_empty() && x.0 == e))
                .unwrap_or_default(),
            ..Default::default()
        };
        let mut rest = Vec::new();
        for f in &s.flags {
            match (f.first().map(String::as_str), f.get(1)) {
                (Some("--chrome"), _) => o.chrome = true,
                (Some("--permission-mode"), Some(v)) if v == "manual" => o.perm = "default".into(),
                (Some("--permission-mode"), Some(v)) if PERMS.iter().any(|p| !p.0.is_empty() && p.0 == v) => {
                    o.perm = v.clone()
                }
                (Some("--add-dir"), Some(v)) if o.dirs.len() < DIRS_MAX => o.dirs.push(v.clone()),
                (Some(name), Some(v)) => rest.push(format!("{name} {}", shq(v))),
                (Some(name), None) => rest.push(name.to_string()),
                (None, _) => {}
            }
        }
        let mut other: Vec<String> =
            s.other.iter().filter(|x| !x.trim().is_empty()).map(|x| x.trim().to_string()).collect();
        other.extend(rest);
        o.other = other.join(" ").trim().to_string();
        o
    }

    /// What the setup used last fills in when the sheet opens. Its extra folders stay behind: they are folders
    /// on the machine that chat ran on, and this sheet may be on another.
    pub fn from_last(s: &ApiSetup) -> Opts {
        Opts { dirs: Vec::new(), ..Opts::from_setup(s) }
    }

    /// The sheet moved to another machine: the extra folders were picked on the one before.
    pub fn machine_changed(&mut self) {
        self.dirs.clear();
    }
}

/// Max and UltraCode set the effort themselves.
pub fn effort_locked(mode: &str) -> bool {
    mode == "max" || mode == "ultracode"
}

/// A word as a shell would read it back: bare when it is plain, else in single quotes.
pub fn shq(v: &str) -> String {
    let plain = !v.is_empty() && v.chars().all(|c| c.is_ascii_alphanumeric() || "._/:=,@+%-".contains(c));
    if plain { v.to_string() } else { format!("'{}'", v.replace('\'', "'\\''")) }
}

/// "Other flags" as Claude Code reads them: shell-style words (no shell ever sees them). The server's rules
/// (Python's shlex): single quotes keep everything, a backslash outside quotes keeps the next character, and
/// inside double quotes it only does so before a `"` or another backslash. So `'don'\''t'`, which `shq` writes
/// for "don't", reads back as one word.
pub fn words(text: &str) -> Result<Vec<String>, String> {
    let (mut out, mut cur, mut q): (Vec<String>, Option<String>, Option<char>) = (Vec::new(), None, None);
    let mut it = text.chars().peekable();
    while let Some(ch) = it.next() {
        if let Some(qc) = q {
            if ch == qc {
                q = None;
            } else if ch == '\\' && qc == '"' && matches!(it.peek(), Some('"') | Some('\\')) {
                cur.get_or_insert_with(String::new).extend(it.next());
            } else {
                cur.get_or_insert_with(String::new).push(ch);
            }
            continue;
        }
        if ch == '\\' {
            match it.next() {
                Some(next) => cur.get_or_insert_with(String::new).push(next),
                None => return Err("A \\ at the end has nothing after it.".into()),
            }
        } else if ch == '"' || ch == '\'' {
            q = Some(ch);
            cur.get_or_insert_with(String::new);
        } else if ch.is_whitespace() {
            if let Some(w) = cur.take() {
                out.push(w);
            }
        } else {
            cur.get_or_insert_with(String::new).push(ch);
        }
    }
    if q.is_some() {
        return Err("A quote isn't closed.".into());
    }
    out.extend(cur);
    Ok(out)
}

/// What is wrong with "Other flags", in words, or None. `takes` is every flag the server passes and whether it
/// takes a value; empty (an older server) leaves the check to Start.
pub fn check_other(
    o: &Opts,
    model: &str,
    mode: &str,
    takes: &HashMap<String, bool>,
    aliases: &HashMap<String, String>,
) -> Option<String> {
    let text = o.other.trim();
    if text.is_empty() {
        return None;
    }
    let ws = match words(text) {
        Ok(w) => w,
        Err(e) => return Some(e),
    };
    if takes.is_empty() {
        return None;
    }
    let cut = |s: &str| s.chars().take(40).collect::<String>();
    let mut seen: Vec<String> = Vec::new();
    let mut i = 0;
    while i < ws.len() {
        let w = &ws[i];
        let eq = w.find('=').filter(|&e| e > 0);
        let raw = eq.map_or(w.as_str(), |e| &w[..e]);
        let name = aliases.get(raw).cloned().unwrap_or_else(|| raw.to_string());
        if !name.starts_with("--") {
            return Some(format!("{} isn't a flag. Flags start with --.", cut(w)));
        }
        let chip = name == "--model" || name == "--effort";
        if !takes.contains_key(&name) && !chip {
            return Some(format!("{} isn't a flag Switchboard passes to Claude Code.", cut(&name)));
        }
        if name == "--chrome" && o.chrome {
            return Some("Claude in Chrome is already on above.".into());
        }
        if name == "--permission-mode" && !o.perm.is_empty() {
            return Some("Permissions are already picked above.".into());
        }
        if name == "--model" && (!model.is_empty() || mode == "ultracode") {
            return Some("The model is already picked above.".into());
        }
        if name == "--effort" && (!o.effort.is_empty() || effort_locked(mode)) {
            return Some("The effort is already picked above.".into());
        }
        if seen.contains(&name) && name != "--add-dir" {
            return Some(format!("{name} is given twice."));
        }
        seen.push(name.clone());
        let needs = chip || takes.get(&name).copied().unwrap_or(false);
        if !needs && eq.is_some() {
            return Some(format!("{name} takes no value."));
        }
        if needs && eq.is_none() {
            let bare = i + 1 >= ws.len() || ws[i + 1].starts_with("--");
            if bare && !OPTIONAL.contains(&name.as_str()) {
                return Some(format!("{name} needs a value."));
            }
            if !bare {
                i += 1;
            }
        }
        i += 1;
    }
    None
}

/// The command the machine runs, as the sheet previews it: every flag Switchboard passes, in its order. Left out
/// are the chat's name (made from the message once it starts) and the message itself; an extra folder is shown as
/// it was picked, and the machine passes its full path.
pub fn cmd_text(o: &Opts, model: &str, mode: &str) -> String {
    let ultra = mode == "ultracode";
    let mut a: Vec<String> = vec!["claude".into(), "--bg".into(), "--remote-control".into()];
    let mdl = if ultra { "opus" } else { model };
    if !mdl.is_empty() {
        a.extend(["--model".to_string(), mdl.to_string()]);
    }
    let eff = if ultra {
        "ultracode"
    } else if mode == "max" {
        "max"
    } else {
        o.effort.as_str()
    };
    if !eff.is_empty() {
        a.extend(["--effort".to_string(), eff.to_string()]);
    }
    for f in o.flags() {
        a.push(f[0].clone());
        if let Some(v) = f.get(1) {
            a.push(shq(v));
        }
    }
    if !o.other.trim().is_empty() {
        a.push(o.other.trim().to_string());
    }
    a.join(" ")
}

/// The setup to save as a preset (or compare with one).
pub fn setup(o: &Opts, model: &str, mode: &str, name: Option<String>) -> ApiSetup {
    ApiSetup {
        name,
        model: (!model.is_empty()).then(|| model.to_string()),
        effort: (!o.effort.is_empty() && !effort_locked(mode)).then(|| o.effort.clone()),
        mode: Some(if mode.is_empty() { "normal" } else { mode }.to_string()),
        flags: o.flags(),
        other: Some(o.other.trim().to_string()),
    }
}

/// Two setups that start the same session (names aside, flags in any order).
pub fn same_setup(a: &ApiSetup, b: &ApiSetup) -> bool {
    let sig = |s: &ApiSetup| {
        let o = Opts::from_setup(s);
        let mut f: Vec<String> = o.flags().iter().map(|f| f.join("=")).collect();
        f.sort();
        (s.mode.clone().unwrap_or_else(|| "normal".into()), s.model.clone().unwrap_or_default(), o.effort, f, o.other)
    };
    sig(a) == sig(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn takes() -> (HashMap<String, bool>, HashMap<String, String>) {
        let t = [
            ("--chrome", false),
            ("--add-dir", true),
            ("--agent", true),
            ("--permission-mode", true),
            ("--worktree", true),
            ("--verbose", false),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();
        (t, [("-w".to_string(), "--worktree".to_string())].into_iter().collect())
    }

    #[test]
    fn command_preview_reads_like_the_web() {
        let o = Opts {
            chrome: true,
            perm: "plan".into(),
            effort: "high".into(),
            dirs: vec!["My Notes".into()],
            other: "--agent reviewer".into(),
        };
        assert_eq!(
            cmd_text(&o, "sonnet", "normal"),
            "claude --bg --remote-control --model sonnet --effort high --chrome --permission-mode plan --add-dir 'My Notes' --agent reviewer"
        );
        assert_eq!(
            cmd_text(&Opts::default(), "", "ultracode"),
            "claude --bg --remote-control --model opus --effort ultracode"
        );
    }

    #[test]
    fn asks_first_is_a_choice_of_its_own() {
        // "" sends no flag (the machine's own default mode); "Asks first" says so by name
        assert_eq!(PERMS[0], ("", "Machine default"));
        assert!(Opts::default().flags().is_empty());
        let o = Opts { perm: "default".into(), ..Default::default() };
        assert_eq!(o.flags(), vec![vec!["--permission-mode".to_string(), "default".to_string()]]);
        assert_eq!(PERMS.iter().find(|p| p.0 == "default").unwrap().1, "Asks first");
        // a setup saved by a client that calls the mode "manual" lands on the same choice
        let s = ApiSetup { flags: vec![vec!["--permission-mode".into(), "manual".into()]], ..Default::default() };
        assert_eq!(Opts::from_setup(&s).perm, "default");
        let s = ApiSetup { flags: vec![vec!["--permission-mode".into(), "dontAsk".into()]], ..Default::default() };
        assert_eq!(Opts::from_setup(&s).perm, "dontAsk");
    }

    #[test]
    fn words_read_quotes_the_way_the_server_does() {
        let w = |t: &str| words(t).unwrap();
        assert_eq!(w(r"--append-system-prompt 'don'\''t stop'"), ["--append-system-prompt", "don't stop"]);
        assert_eq!(w(&format!("--x {}", shq("it's a 'test'"))), ["--x", "it's a 'test'"]);
        assert_eq!(w(r#"--x "a \"b\" c\d" e\ f ''"#), ["--x", r#"a "b" c\d"#, "e f", ""]);
        assert_eq!(w(r"'a\b'"), [r"a\b"]);
        assert!(words("--x \\").is_err());
        assert!(words("--x 'open").is_err());
        // what a setup writes into Other reads back as the same words
        let s =
            ApiSetup { flags: vec![vec!["--append-system-prompt".into(), "don't stop".into()]], ..Default::default() };
        let o = Opts::from_setup(&s);
        assert_eq!(w(&o.other), ["--append-system-prompt", "don't stop"]);
        let (t, a) = takes();
        let t: HashMap<String, bool> = t.into_iter().chain([("--append-system-prompt".to_string(), true)]).collect();
        assert!(check_other(&o, "", "normal", &t, &a).is_none());
    }

    #[test]
    fn extra_folders_belong_to_the_machine_they_were_picked_on() {
        let mut o = Opts { chrome: true, dirs: vec!["notes".into()], other: "--verbose".into(), ..Default::default() };
        o.machine_changed();
        assert_eq!(o, Opts { chrome: true, other: "--verbose".into(), ..Default::default() });
        let last = ApiSetup {
            flags: vec![vec!["--chrome".into()], vec!["--add-dir".into(), "notes".into()]],
            effort: Some("high".into()),
            ..Default::default()
        };
        assert_eq!(Opts::from_last(&last), Opts { chrome: true, effort: "high".into(), ..Default::default() });
        assert_eq!(Opts::from_setup(&last).dirs, ["notes"]); // picked by hand, a setup still brings them
    }

    #[test]
    fn a_worktree_may_go_without_a_name() {
        let (t, a) = takes();
        let chk = |other: &str| check_other(&Opts { other: other.into(), ..Default::default() }, "", "normal", &t, &a);
        assert!(chk("--worktree").is_none());
        assert!(chk("-w --verbose").is_none());
        assert!(chk("--worktree fix --verbose").is_none());
        assert_eq!(chk("--agent --verbose").unwrap(), "--agent needs a value.");
        assert_eq!(chk("--worktree fix extra").unwrap(), "extra isn't a flag. Flags start with --.");
    }

    #[test]
    fn other_flags_are_checked() {
        let (t, a) = takes();
        let chk = |other: &str, o: Opts| check_other(&Opts { other: other.into(), ..o }, "", "normal", &t, &a);
        assert_eq!(chk("agent x", Opts::default()).unwrap(), "agent isn't a flag. Flags start with --.");
        assert_eq!(chk("--nope", Opts::default()).unwrap(), "--nope isn't a flag Switchboard passes to Claude Code.");
        assert_eq!(chk("--agent", Opts::default()).unwrap(), "--agent needs a value.");
        assert_eq!(chk("--agent 'x", Opts::default()).unwrap(), "A quote isn't closed.");
        assert_eq!(
            chk("--chrome", Opts { chrome: true, ..Default::default() }).unwrap(),
            "Claude in Chrome is already on above."
        );
        assert_eq!(chk("--verbose=1", Opts::default()).unwrap(), "--verbose takes no value.");
        assert_eq!(chk("--agent a --agent b", Opts::default()).unwrap(), "--agent is given twice.");
        assert_eq!(
            check_other(&Opts { other: "--model opus".into(), ..Default::default() }, "sonnet", "normal", &t, &a)
                .unwrap(),
            "The model is already picked above."
        );
        assert!(chk("-w feat --agent \"code reviewer\" --add-dir a --add-dir=b", Opts::default()).is_none());
        assert!(
            check_other(&Opts { other: "--anything".into(), ..Default::default() }, "", "normal", &HashMap::new(), &a)
                .is_none()
        );
    }

    #[test]
    fn setups_round_trip() {
        let s = ApiSetup {
            flags: vec![
                vec!["--chrome".into()],
                vec!["--add-dir".into(), "notes".into()],
                vec!["--agent".into(), "code reviewer".into()],
            ],
            effort: Some("high".into()),
            other: Some("--verbose".into()),
            ..Default::default()
        };
        let o = Opts::from_setup(&s);
        assert!(o.chrome && o.dirs == ["notes"] && o.effort == "high");
        assert_eq!(o.other, "--verbose --agent 'code reviewer'");
        assert!(same_setup(&setup(&o, "", "normal", None), &setup(&o, "", "normal", Some("x".into()))));
        assert!(!same_setup(&setup(&o, "", "normal", None), &setup(&Opts::default(), "", "normal", None)));
    }
}
