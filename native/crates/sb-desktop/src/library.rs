//! The tool library, app side: one shared copy of GET /api/library (Settings > Library, the New sheet's Tools and a
//! chat's Tools all read it) and every change to it. Each change shows at once where it can, then the library is read
//! again so what the machines say wins.

use std::time::Duration;

use gpui::{App, Context};
use sb_core::api::{ApiLibrary, ApiToolStored};
use sb_core::{Core, CoreError, JobKind};

use crate::state::AppState;

/// Tool names, as the server takes them: a lowercase letter or digit, then up to 39 more or dashes.
pub fn valid_tool_name(s: &str) -> bool {
    let b = s.as_bytes();
    !b.is_empty() && b.len() <= 40 && (b[0].is_ascii_lowercase() || b[0].is_ascii_digit()) && b.iter().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-')
}

/// The command that stores one secret in a machine's keychain. It asks for the value without showing it.
pub fn secret_command(os: Option<&str>, tool: &str, name: &str) -> String {
    if os == Some("windows") {
        format!("python \"$HOME/.claude/switchboard/sb.py\" secret set {tool} {name}")
    } else {
        format!("python3 ~/.claude/switchboard/sb.py secret set {tool} {name}")
    }
}

/// The keychain item the agent reads a tool's secrets from (agent/sb.py KEY_SERVICE, account "tool/NAME").
const KEY_SERVICE: &str = "switchboard-library";

/// A machine name as the server takes it: 1 to 32 lowercase letters or digits (agent/sb.py MACHINE_RE).
fn valid_machine(s: &str) -> bool {
    !s.is_empty() && s.len() <= 32 && s.bytes().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
}

/// The name the server knows this computer by, worked out the way the agent does (agent/sb.py machine_name): what
/// switchboard-node paired it as (node.json), else what the server told the agent (agent.json), else the hostname.
pub fn local_machine_name(node_json: Option<&str>, agent_json: Option<&str>, host: &str) -> String {
    let from = |j: Option<&str>| {
        let v: serde_json::Value = serde_json::from_str(j?).ok()?;
        v.get("machine")?.as_str().filter(|n| valid_machine(n)).map(str::to_string)
    };
    from(node_json).or_else(|| from(agent_json)).unwrap_or_else(|| {
        let n: String = host.split('.').next().unwrap_or("").to_lowercase().chars().filter(|c| c.is_ascii_lowercase() || c.is_ascii_digit()).take(32).collect();
        if n.is_empty() { "machine".into() } else { n }
    })
}

/// Is this machine row the Mac the app runs on? Its name matches and it is a Mac (or does not say).
pub fn is_this_mac(name: &str, os: Option<&str>, here: &str) -> bool {
    name == here && os.is_none_or(|o| o == "mac")
}

/// This Mac's machine name, read once. None off macOS (Set key there stays a command to copy).
pub fn this_mac() -> Option<&'static str> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    static HERE: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    Some(HERE.get_or_init(|| {
        let home = std::path::PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
        let sb_home = std::env::var_os("SB_HOME").map(std::path::PathBuf::from).unwrap_or_else(|| home.join(".claude/switchboard"));
        let node = std::fs::read_to_string(home.join(".config/switchboard/node.json")).ok();
        let agent = std::fs::read_to_string(sb_home.join("agent.json")).ok();
        let host = std::process::Command::new("hostname").output().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default();
        local_machine_name(node.as_deref(), agent.as_deref(), &host)
    }))
}

/// A secret as the agent takes it: 1 to 4096 printable characters on one line (agent/sb.py check_secret_value).
pub fn check_secret_value(v: &str) -> Result<(), String> {
    if v.is_empty() || v.len() > 4096 || !v.chars().all(|c| (' '..='~').contains(&c)) {
        return Err("A key is 1 to 4096 printable characters on one line.".into());
    }
    Ok(())
}

/// Saves one secret in this Mac's login keychain: the same item, and the same `security -i` line, as the agent's
/// MacKeychain.set. The value goes in on stdin, never in argv, and nothing here logs it.
#[cfg(target_os = "macos")]
fn keychain_set(tool: &str, name: &str, value: String) -> Result<(), String> {
    use std::io::Write;
    use std::process::{Command, Stdio};
    check_secret_value(&value)?;
    let q = |s: &str| format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""));
    let acct = format!("{tool}/{name}");
    let line = format!("add-generic-password -U -s {} -a {} -l {} -w {}\n", q(KEY_SERVICE), q(&acct), q(&format!("Switchboard {acct}")), q(&value));
    drop(value);
    let run = |e: std::io::Error| format!("Could not run security: {e}");
    let mut child = Command::new("security").arg("-i").stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::piped()).spawn().map_err(run)?;
    // Written, then stdin closes (dropped) so `security -i` runs the line and exits.
    let wrote = child.stdin.take().map_or(Ok(()), |mut i| i.write_all(line.as_bytes()));
    drop(line);
    let out = child.wait_with_output().map_err(run)?;
    // Checked like the agent does: the item is there now.
    let has = Command::new("security").args(["find-generic-password", "-s", KEY_SERVICE, "-a", &acct]).stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success());
    if wrote.is_err() || !out.status.success() || !has {
        let why: String = String::from_utf8_lossy(&out.stderr).trim().chars().take(160).collect();
        return Err(format!("The keychain would not save it: {}", if why.is_empty() { "no reason given" } else { &why }));
    }
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn keychain_set(_: &str, _: &str, _: String) -> Result<(), String> {
    Err("Only a Mac keeps keys from here.".into())
}

impl AppState {
    /// The library as last read, if it was.
    pub fn lib(&self) -> Option<&ApiLibrary> {
        self.library.as_ref().and_then(|l| l.as_ref().ok())
    }

    /// Reads the library again. A failure keeps what is already shown.
    pub fn load_library(&mut self, cx: &mut Context<Self>) {
        self.call(
            cx,
            |core| core.library(),
            |st, res, cx| {
                match res {
                    Ok(l) => {
                        // Keys saved on this Mac are forgotten once the server no longer lists them as missing here.
                        if let Some(mine) = this_mac().and_then(|h| l.machines.get(h)) {
                            st.lib_keys_here.retain(|k| k.split_once('/').is_some_and(|(t, n)| mine.get(t).is_some_and(|x| x.missing_secrets.iter().any(|m| m == n))));
                        }
                        st.library = Some(Ok(l))
                    }
                    Err(e) if st.lib().is_none() => st.library = Some(Err(e.to_string())),
                    Err(e) => log::warn!("could not read the library: {e}"),
                }
                cx.notify();
            },
        );
    }

    fn label(&self, machine: &str) -> String {
        self.snap.label_of(machine).unwrap_or_else(|| machine.to_string())
    }

    /// One change: marked busy while it runs, a toast when it is done or fails, then the library is read again.
    fn lib_change(&mut self, busy: String, ok: Option<String>, work: impl FnOnce(&Core) -> Result<(), CoreError> + Send + 'static, cx: &mut Context<Self>) {
        if !self.lib_busy.insert(busy.clone()) {
            return;
        }
        cx.notify();
        self.call(cx, work, move |st, res, cx| {
            st.lib_busy.remove(&busy);
            match res {
                Ok(()) => {
                    if let Some(m) = ok {
                        st.toast(m, None, false, cx);
                    }
                }
                Err(e) => st.toast(e.to_string(), None, true, cx),
            }
            st.load_library(cx);
            cx.notify();
        });
    }

    /// Puts a tool's latest version on a machine, or takes it off.
    pub fn lib_connect(&mut self, tool: &str, machine: &str, on: bool, cx: &mut Context<Self>) {
        if let Some(Ok(l)) = self.library.as_mut() {
            l.machines.entry(machine.to_string()).or_default().entry(tool.to_string()).or_default().connected = on;
        }
        let label = self.label(machine);
        let ok = if on { format!("{tool} is on {label}") } else { format!("{tool} is off {label}") };
        let (t, m) = (tool.to_string(), machine.to_string());
        self.lib_change(format!("connect:{tool}@{machine}"), Some(ok), move |core| core.library_connect(&t, &m, on), cx);
    }

    /// On for every new chat, or not.
    pub fn lib_default(&mut self, tool: &str, on: bool, cx: &mut Context<Self>) {
        if let Some(Ok(l)) = self.library.as_mut() {
            l.defaults.retain(|d| d != tool);
            if on {
                l.defaults.push(tool.to_string());
            }
        }
        let t = tool.to_string();
        self.lib_change(format!("default:{tool}"), None, move |core| core.library_defaults(&t, on), cx);
    }

    /// Puts a tool in Auto or takes it out, and sets its "When to use" line.
    pub fn lib_auto(&mut self, tool: &str, auto: bool, when: &str, cx: &mut Context<Self>) {
        if let Some(x) = self.library.as_mut().and_then(|l| l.as_mut().ok()).and_then(|l| l.tools.iter_mut().find(|x| x.name == tool)) {
            x.auto = Some(auto);
            x.when = Some(when.to_string());
        }
        let (t, w) = (tool.to_string(), when.to_string());
        self.lib_change(format!("auto:{tool}"), None, move |core| core.library_auto(&t, auto, &w), cx);
    }

    /// Makes an older version the latest again; machines that have the tool get it.
    pub fn lib_rollback(&mut self, tool: &str, version: u32, cx: &mut Context<Self>) {
        let t = tool.to_string();
        self.lib_change(format!("rollback:{tool}"), Some(format!("{tool} v{version} is the latest again")), move |core| core.library_rollback(&t, version), cx);
    }

    /// Deletes a tool everywhere.
    pub fn lib_delete(&mut self, tool: &str, cx: &mut Context<Self>) {
        if let Some(Ok(l)) = self.library.as_mut() {
            l.tools.retain(|x| x.name != tool);
            l.defaults.retain(|d| d != tool);
        }
        let t = tool.to_string();
        self.lib_change(format!("delete:{tool}"), Some(format!("Deleted {tool}")), move |core| core.library_delete(&t), cx);
    }

    /// Adds a skill or an MCP server from `machine` to the library (a new tool, or the next version of one).
    pub fn lib_import(&mut self, machine: &str, kind: &str, source: &str, done: impl FnOnce(Result<ApiToolStored, String>, &mut App) + 'static, cx: &mut Context<Self>) {
        let busy = format!("import:{machine}/{kind}/{source}");
        if !self.lib_busy.insert(busy.clone()) {
            return;
        }
        cx.notify();
        let (m, k, s) = (machine.to_string(), kind.to_string(), source.to_string());
        self.call(
            cx,
            move |core| core.library_import(&m, &k, &s, None, None),
            move |st, res, cx| {
                st.lib_busy.remove(&busy);
                match &res {
                    Ok(r) => st.toast(if r.version > 1 { format!("Added {} v{} to the library", r.tool, r.version) } else { format!("Added {} to the library", r.tool) }, None, false, cx),
                    Err(e) => st.toast(e.to_string(), None, true, cx),
                }
                st.load_library(cx);
                let res = res.map_err(|e| e.to_string());
                cx.defer(move |cx| done(res, cx));
                cx.notify();
            },
        );
    }

    /// New tool: a chat on `machine` builds it from `brief`, tests it and publishes it. Followed like a new session.
    pub fn lib_new(&mut self, machine: &str, tool: &str, brief: &str, done: impl FnOnce(Result<(), String>, &mut App) + 'static, cx: &mut Context<Self>) {
        let (m, t, b) = (machine.to_string(), tool.to_string(), brief.to_string());
        let label = self.label(machine);
        let name = tool.to_string();
        self.call(
            cx,
            move |core| core.library_new(&m, &t, &b),
            move |st, res, cx| {
                let res = match res {
                    Ok(Some(jid)) => {
                        st.start_job(&jid, JobKind::Start, None, &label, &label, None, Some(name.clone()), cx);
                        st.toast(format!("Building {name} on {label}"), Some("It publishes to the library when it is done.".into()), false, cx);
                        Ok(())
                    }
                    Ok(None) => {
                        st.toast(format!("Building {name} on {label}"), None, false, cx);
                        Ok(())
                    }
                    Err(e) => Err(e.to_string()),
                };
                cx.defer(move |cx| done(res, cx));
            },
        );
    }

    /// Set key on this Mac: the value goes straight into its keychain (off the main thread), then the library is read
    /// again. The value is kept nowhere here; `lib_keys_here` holds the key's name until the server's next look agrees.
    pub fn lib_set_key_here(&mut self, tool: &str, name: &str, value: String, done: impl FnOnce(Result<(), String>, &mut App) + 'static, cx: &mut Context<Self>) {
        let busy = format!("key:{tool}/{name}");
        if !self.lib_busy.insert(busy.clone()) {
            return;
        }
        cx.notify();
        let (t, n) = (tool.to_string(), name.to_string());
        cx.spawn(async move |this, cx| {
            let (t2, n2) = (t.clone(), n.clone());
            let res = cx.background_executor().spawn(async move { keychain_set(&t2, &n2, value) }).await;
            let _ = this.update(cx, |st, cx| {
                st.lib_busy.remove(&busy);
                if res.is_ok() {
                    st.lib_keys_here.insert(format!("{t}/{n}"));
                    st.toast(format!("Saved {n} in this Mac's keychain"), None, false, cx);
                    st.load_library(cx);
                }
                cx.defer(move |cx| done(res, cx));
                cx.notify();
            });
        })
        .detach();
    }

    /// Sets a chat's tools.`restart`: it is running, so it restarts to load them (it re-reads its context).
    pub fn attach_tools(&mut self, key: &str, tools: Vec<String>, restart: bool, cx: &mut Context<Self>) {
        let Some(s) = self.snap.session(key) else { return };
        let busy = format!("attach:{key}");
        if !self.lib_busy.insert(busy.clone()) {
            return;
        }
        let (m, sid) = (s.machine.clone(), s.sid.clone());
        let key = key.to_string();
        let t2 = tools.clone();
        cx.notify();
        self.call(
            cx,
            move |core| core.library_attach(&m, &sid, &t2),
            move |st, res, cx| {
                st.lib_busy.remove(&busy);
                match res {
                    Ok(()) => {
                        // What the board shows until the next refresh says so too.
                        if let Some((m, sid)) = key.split_once(':') {
                            if let Some(s) = st.snap.machines.iter_mut().find(|x| x.name == m).and_then(|x| x.sessions.iter_mut().find(|s| s.sid == sid)) {
                                s.tools = tools;
                            }
                        }
                        if restart {
                            st.relaunch(&key, serde_json::json!({}), None, cx);
                        } else {
                            st.toast("Tools saved", Some("The chat loads them the next time it starts.".into()), false, cx);
                        }
                        st.refresh_in(Duration::from_millis(600), cx);
                    }
                    Err(e) => st.toast(e.to_string(), None, true, cx),
                }
                cx.notify();
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_names() {
        assert!(valid_tool_name("web-fetch"));
        assert!(valid_tool_name("yt-research2"));
        assert!(valid_tool_name("9lives"));
        assert!(!valid_tool_name(""));
        assert!(!valid_tool_name("-x"));
        assert!(!valid_tool_name("Caps"));
        assert!(!valid_tool_name("a_b"));
        assert!(!valid_tool_name(&"a".repeat(41)));
        assert!(valid_tool_name(&"a".repeat(40)));
    }

    #[test]
    fn secret_commands() {
        assert_eq!(secret_command(Some("mac"), "web-fetch", "WEB_FETCH_KEY"), "python3 ~/.claude/switchboard/sb.py secret set web-fetch WEB_FETCH_KEY");
        assert!(secret_command(Some("windows"), "a", "B").starts_with("python \""));
    }

    #[test]
    fn this_mac_name() {
        let node = r#"{"hub":"x","machine":"studio","token":"t"}"#;
        let agent = r#"{"machine":"desk"}"#;
        assert_eq!(local_machine_name(Some(node), Some(agent), "Other.local"), "studio");
        assert_eq!(local_machine_name(None, Some(agent), "Other.local"), "desk");
        assert_eq!(local_machine_name(Some(r#"{"machine":"Bad Name"}"#), Some("not json"), "Your-Mac-Studio.local"), "yourmacstudio");
        assert_eq!(local_machine_name(None, None, ""), "machine");
        assert!(is_this_mac("studio", Some("mac"), "studio"));
        assert!(is_this_mac("studio", None, "studio"));
        assert!(!is_this_mac("studio", Some("linux"), "studio"));
        assert!(!is_this_mac("laptop", Some("mac"), "studio"));
    }

    #[test]
    fn secret_values() {
        assert!(check_secret_value("sk-abc 123").is_ok());
        assert!(check_secret_value("").is_err());
        assert!(check_secret_value("a\nb").is_err());
        assert!(check_secret_value(&"a".repeat(4097)).is_err());
    }
}
