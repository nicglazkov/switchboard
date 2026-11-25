//! switchboard-node (spec section 6): links this machine to the Switchboard hub with one outgoing WebSocket, so a
//! machine behind NAT or without SSH still shows up, runs jobs and opens terminals.
//!
//!   switchboard-node pair <hub> <code> [--name NAME] [--label LABEL]   trade an Add a machine code for a token
//!   switchboard-node pair <code-or-link> [--name NAME] [--addr IP:PORT] [--replace]
//!                                                                       pair with a home computer by its key
//!                                                                       (--replace: leave the home it has)
//!   switchboard-node code                                              (home) a code and a link for one new device
//!   switchboard-node devices                                           (home) the devices paired with this computer
//!   switchboard-node forget <name>                                     (home) remove a paired device
//!   switchboard-node relay <url> | none                                which relay this computer uses
//!   switchboard-node run                                               keep the link up (the user service runs this)
//!   switchboard-node status                                            what this machine is linked as
//!   switchboard-node uninstall [--purge]                               stop it and remove it from this machine
//!                                                                      (--purge: this computer's key as well)
//!
//! <hub> is the hub's address: 192.0.2.10, 192.0.2.10:8132, a tailnet name, or an http:// URL.
//! Settings live in ~/.config/switchboard/node.json (mode 600): {hub, machine, token, sb?, python?, home?, relay?}.
//! Keys (keys.rs) add device.key and keys.lock next to it; they are off until the server's settings on this
//! computer turn them on, or this computer is paired with a home computer.
//! SWITCHBOARD_NODE_CONFIG moves that file; SWITCHBOARD_SB and SWITCHBOARD_PYTHON say where sb.py and Python are
//! (default ~/.claude/switchboard/sb.py, run with python3, or python on Windows).

/// A line to standard output. `println!` panics when the output is a pipe whose reader has gone
/// (`switchboard-node devices | head -1`, a terminal that closed): this writes what it can and carries on, so an
/// `uninstall` half way through still finishes.
macro_rules! say {
    ($($a:tt)*) => { $crate::write_line(&mut std::io::stdout().lock(), format_args!($($a)*)) };
}
/// A line to standard error, which is the log of a node that runs as a service. Never a panic, as `say!`.
macro_rules! complain {
    ($($a:tt)*) => { $crate::write_line(&mut std::io::stderr().lock(), format_args!($($a)*)) };
}

mod control;
mod keys;
mod link;
mod own;
mod uninstall;

use sb_core::durable::{Bak, Status, Store};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
const HUB_PORT: u16 = 8132;
/// How often `run` looks at node.json again, and at whether a wait is over, while it has no token the hub takes.
const RECHECK: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NodeConfig {
    /// The hub's address. Empty on a computer that reaches the hub only through the home computer's tunnel.
    #[serde(default)]
    pub hub: String,
    #[serde(default)]
    pub machine: String,
    #[serde(default)]
    pub token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sb: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub python: Option<String>,
    /// The home computer this one is paired with by key (keys.rs). With it, keys are on here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub home: Option<HomeConfig>,
    /// The relay this computer uses, when the user chose one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relay: Option<String>,
}

/// The home computer, as pairing recorded it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HomeConfig {
    pub id: sb_link::identity::DeviceId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relay: Option<String>,
    #[serde(default)]
    pub name: String,
    /// Where it was last reached directly.
    #[serde(default)]
    pub addrs: Vec<std::net::SocketAddr>,
}

pub(crate) fn write_line(to: &mut dyn std::io::Write, line: std::fmt::Arguments) {
    let _ = writeln!(to, "{line}");
}

/// The user's folder, by the hub's rule: on Windows USERPROFILE, and HOME only where there is none.
pub fn home() -> PathBuf {
    sb_hub::config::home()
}

pub(crate) fn env(k: &str) -> Option<String> {
    std::env::var(k).ok().filter(|v| !v.trim().is_empty())
}

fn config_path() -> PathBuf {
    config_path_from(&home(), &|k: &str| env(k))
}

/// node.json: where SWITCHBOARD_NODE_CONFIG says, else in the settings folder (SWITCHBOARD_CONFIG_DIR, else
/// .config/switchboard in the user's folder).
fn config_path_from(home: &Path, env: &dyn Fn(&str) -> Option<String>) -> PathBuf {
    env("SWITCHBOARD_NODE_CONFIG")
        .filter(|v| !v.trim().is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| sb_hub::config::config_dir(home, env).join("node.json"))
}

/// The node.json an earlier version on Windows would have used, when that is another file than the one used now:
/// it followed HOME where one was set. Nothing reads or writes it; `load` names it for whoever finds this computer
/// "not paired" after an update.
fn left_behind(windows: bool, env: &dyn Fn(&str) -> Option<String>) -> Option<PathBuf> {
    let set = |k: &str| env(k).filter(|v| !v.trim().is_empty());
    if !windows || set("SWITCHBOARD_NODE_CONFIG").is_some() || set("SWITCHBOARD_CONFIG_DIR").is_some() {
        return None;
    }
    let (old, now) = (PathBuf::from(set("HOME")?), PathBuf::from(set("USERPROFILE")?));
    (old != now).then(|| old.join(".config/switchboard/node.json"))
}

fn left_behind_says(old: &Path, now: &Path) -> String {
    format!(
        "There is a link in {}, where earlier versions kept it when HOME was set. To use it, copy that file to {}.",
        old.display(),
        now.display()
    )
}

/// Why `pair` will not put a link to the hub at `hub` over the one in node.json, if it will not: the file links
/// this computer to another hub, and nobody said to replace that (a test or a second install that found the real
/// file must not take the computer off its board). The same hub again is the usual way to get a new token.
fn other_hub(old: Option<&NodeConfig>, hub: &str, replace: bool, path: &Path) -> Option<String> {
    let old = old.filter(|o| !o.hub.is_empty() && !o.token.is_empty())?;
    if replace || old.hub.eq_ignore_ascii_case(hub) {
        return None;
    }
    Some(format!(
        "This computer is linked to the hub at {} already ({}), so nothing was changed. To link it to the hub at \
         {hub} instead, run the same command again with --replace at the end.",
        old.hub,
        path.display()
    ))
}

impl NodeConfig {
    pub fn sb(&self) -> String {
        env("SWITCHBOARD_SB")
            .or_else(|| self.sb.clone())
            .unwrap_or_else(|| home().join(".claude/switchboard/sb.py").display().to_string())
    }

    pub fn python(&self) -> String {
        env("SWITCHBOARD_PYTHON")
            .or_else(|| self.python.clone())
            .or_else(|| own_python(&home(), env("LOCALAPPDATA").map(PathBuf::from).as_deref()))
            .unwrap_or_else(|| if cfg!(windows) { "python" } else { "python3" }.into())
    }
}

/// Switchboard's own Python, when an installer put one on this computer (install-server and install-agent do): the
/// helper then runs on a computer that has no Python of its own.
fn own_python(home: &Path, local_app_data: Option<&Path>) -> Option<String> {
    let p = if cfg!(windows) {
        local_app_data
            .map(Path::to_path_buf)
            .unwrap_or_else(|| home.join("AppData/Local"))
            .join("Switchboard/python/python.exe")
    } else {
        home.join(".local/share/switchboard/python/bin/python3")
    };
    p.is_file().then(|| p.display().to_string())
}

fn load() -> Result<NodeConfig, String> {
    let p = config_path();
    load_from(&p).map_err(|e| match left_behind(cfg!(windows), &|k: &str| env(k)) {
        Some(old) if !p.exists() && old.exists() => format!("{e} {}", left_behind_says(&old, &p)),
        _ => e,
    })
}

/// node.json as a store (sb-core durable.rs): this user's alone, with a copy beside it that always matches the
/// last save, so a token that was replaced never comes back from the copy.
fn store(p: &Path) -> Store {
    Store::new(p).private().bak(Bak::Fresh)
}

/// node.json. One a power cut emptied or cut short is put aside as node.json.bad-<time> and its copy
/// (node.json.bak) is used and written back, so the machine stays linked without pairing again.
fn load_from(p: &Path) -> Result<NodeConfig, String> {
    let store = store(p);
    let c = match store.load_json::<NodeConfig>() {
        (Some(c), Status::Recovered) => {
            if let Some(kept) = store.kept() {
                link::log(format!(
                    "{} could not be read: its last good copy is in use, and the file as it was is kept as {kept}.",
                    p.display()
                ));
            }
            c
        }
        (Some(c), _) => c,
        (None, Status::Broken) => {
            let kept = store.kept().map(|k| format!(" (kept as {k})")).unwrap_or_default();
            return Err(format!("{} is not readable{kept} and has no good copy. Pair again.", p.display()));
        }
        (None, _) => return Err("This machine is not paired yet.".to_string()),
    };
    // (a file that only holds what keys need, such as the relay on the home computer, links nothing)
    if c.token.is_empty() || (c.hub.is_empty() && c.home.is_none()) {
        return Err("This machine is not paired yet.".to_string());
    }
    // (a hub address written by hand: `pair` never saves one like this)
    match asks_for_tls(&c.hub) {
        Some(why) => Err(format!("{}: {why}", p.display())),
        None => Ok(c),
    }
}

/// node.json as it is, paired or not, for what keys keep in it. None when there is none that reads.
pub(crate) fn load_raw(p: &Path) -> Option<NodeConfig> {
    store(p).load_json::<NodeConfig>().0
}

fn save(c: &NodeConfig) -> std::io::Result<PathBuf> {
    let p = config_path();
    save_to(&p, c).map(|()| p)
}

/// Forced: `pair` is the user saying what this machine's link is, also over a node.json that was lost.
pub(crate) fn save_to(p: &Path, c: &NodeConfig) -> std::io::Result<()> {
    store(p).put_json(c, true)
}

/// Why an address that asks for TLS is refused, if `s` is one. The link is plain ws: dropping the "s" quietly
/// would send the token in the clear to an address its owner believes is encrypted.
fn asks_for_tls(s: &str) -> Option<String> {
    let low = s.trim().to_ascii_lowercase();
    let scheme = ["https://", "wss://"].into_iter().find(|p| low.starts_with(p))?;
    Some(format!(
        "{} asks for an encrypted link ({scheme}), which Switchboard does not have yet: the node talks to the hub \
         in plain ws. Use the hub's plain address, like 192.0.2.10 or http://192.0.2.10:{HUB_PORT}, on a network \
         you trust (your own LAN or a tailnet).",
        s.trim()
    ))
}

/// "192.0.2.10", "http://host:8132/", "ws://[fd7a::1]" -> "host:port". An https:// or wss:// address is refused.
pub fn hub_addr(s: &str) -> Result<String, String> {
    if let Some(why) = asks_for_tls(s) {
        return Err(why);
    }
    let mut h = s.trim();
    for scheme in ["http://", "ws://"] {
        if let Some(rest) = h.strip_prefix(scheme) {
            h = rest;
        }
    }
    let h = h.split('/').next().unwrap_or("");
    let ok =
        !h.is_empty() && h.bytes().all(|c| c.is_ascii_alphanumeric() || b".-:[]%_".contains(&c)) && !h.starts_with('-');
    if !ok {
        return Err(format!("{s} is not a hub address. Use the hub's IP or name, like 192.0.2.10."));
    }
    let has_port = if h.starts_with('[') { h.contains("]:") } else { h.matches(':').count() == 1 };
    Ok(if has_port { h.to_string() } else { format!("{h}:{HUB_PORT}") })
}

/// This computer's host name. Asked of the system once a minute at most: the helper's status is asked for far
/// more often than that, and every asking would start a program.
fn hostname() -> String {
    static LAST: std::sync::Mutex<Option<(std::time::Instant, String)>> = std::sync::Mutex::new(None);
    let mut last = LAST.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((at, name)) = last.as_ref()
        && at.elapsed() < Duration::from_secs(60)
    {
        return name.clone();
    }
    let out = std::process::Command::new("hostname")
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string());
    let name = out.filter(|h| !h.is_empty()).or_else(|| env("COMPUTERNAME")).unwrap_or_default();
    *last = Some((std::time::Instant::now(), name.clone()));
    name
}

/// The host name without its domain, for the machine's label.
pub fn hostname_label() -> String {
    let h = hostname();
    h.split('.').next().unwrap_or("").to_string()
}

/// A machine name the hub accepts (1 to 32 lowercase letters and digits) made from the host name.
pub fn name_from(host: &str) -> String {
    let short = host.split('.').next().unwrap_or("");
    let n: String =
        short.chars().filter(|c| c.is_ascii_alphanumeric()).map(|c| c.to_ascii_lowercase()).take(32).collect();
    if n.is_empty() { "machine".into() } else { n }
}

pub(crate) fn os_name() -> &'static str {
    match std::env::consts::OS {
        "macos" => "mac",
        o => o,
    }
}

/// One small HTTP/1.1 POST of JSON to the hub; the answer's status and JSON body.
async fn post_json(addr: &str, path: &str, body: &Value) -> Result<(u16, Value), String> {
    http_json(addr, "POST", path, Some(body), Duration::from_secs(20)).await
}

/// One small HTTP/1.1 request to `addr` (the hub, or the control port on this computer); the answer's status
/// and JSON body.
pub(crate) async fn http_json(
    addr: &str,
    method: &str,
    path: &str,
    body: Option<&Value>,
    patience: Duration,
) -> Result<(u16, Value), String> {
    let text = body.map(Value::to_string).unwrap_or_default();
    let io = async {
        let mut s = tokio::net::TcpStream::connect(addr).await?;
        let kind = if body.is_some() { "Content-Type: application/json\r\n" } else { "" };
        let req = format!(
            "{method} {path} HTTP/1.1\r\nHost: {addr}\r\n{kind}Content-Length: {}\r\nConnection: close\r\n\r\n{text}",
            text.len()
        );
        s.write_all(req.as_bytes()).await?;
        let mut out = Vec::new();
        s.read_to_end(&mut out).await?;
        Ok::<_, std::io::Error>(out)
    };
    let out = match tokio::time::timeout(patience, io).await {
        Err(_) => return Err(format!("The hub at {addr} did not answer.")),
        Ok(Err(e)) => return Err(format!("Cannot reach the hub at {addr} ({e}).")),
        Ok(Ok(o)) => o,
    };
    let out = String::from_utf8_lossy(&out);
    let (head, body) =
        out.split_once("\r\n\r\n").ok_or_else(|| format!("The hub at {addr} sent something that is not HTTP."))?;
    let status = head.split_whitespace().nth(1).and_then(|s| s.parse().ok()).unwrap_or(0);
    Ok((status, serde_json::from_str(body.trim()).unwrap_or(Value::Null)))
}

async fn pair(args: &[String]) -> i32 {
    let mut pos = vec![];
    let (mut name, mut label) = (None, None);
    let mut addrs = vec![];
    let mut replace = false;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--replace" => replace = true,
            "--name" => name = it.next().cloned(),
            "--label" => label = it.next().cloned(),
            "--addr" => addrs.extend(it.next().cloned()),
            _ => pos.push(a.clone()),
        }
    }
    // one thing to go on is a code or a link from the home computer; two are a hub's address and its code
    if let [typed] = pos.as_slice() {
        return control::pair(typed, name, addrs, replace).await;
    }
    let [hub, code] = pos.as_slice() else { return usage() };
    let addr = match hub_addr(hub) {
        Ok(a) => a,
        Err(e) => return fail(&e, 2),
    };
    // (before the hub is asked: a refusal here leaves the code unused)
    let path = config_path();
    if let Some(why) = other_hub(load_raw(&path).as_ref(), &addr, replace, &path) {
        return fail(&why, 1);
    }
    let host = hostname_label();
    let name = name.unwrap_or_else(|| name_from(&host));
    let label = label.unwrap_or_else(|| if host.is_empty() { name.clone() } else { host });
    // pairing again as the same machine shows the hub its current token; anyone else gets "that name is taken"
    let current = load().ok().filter(|o| o.machine == name).map(|o| o.token).unwrap_or_default();
    let (status, v) = match post_json(
        &addr,
        "/pair",
        &json!({"code": code, "name": name, "label": label, "os": os_name(), "token": current}),
    )
    .await
    {
        Ok(x) => x,
        Err(e) => return fail(&e, 1),
    };
    let token = v["token"].as_str().unwrap_or("");
    if status != 200 || token.is_empty() {
        let why = v["error"].as_str().map(str::to_string).unwrap_or_else(|| format!("The hub answered {status}."));
        return fail(&why, 1);
    }
    let machine = v["machine"].as_str().unwrap_or(&name).to_string();
    let old = load_raw(&path);
    let cfg = NodeConfig {
        hub: addr.clone(),
        machine: machine.clone(),
        token: token.into(),
        sb: old.as_ref().and_then(|o| o.sb.clone()),
        python: old.as_ref().and_then(|o| o.python.clone()),
        // (what keys keep here stays: pairing with a hub by address changes none of it)
        home: old.as_ref().and_then(|o| o.home.clone()),
        relay: old.and_then(|o| o.relay),
    };
    match save(&cfg) {
        Ok(p) => {
            say!(
                "Linked as {machine} ({}) to the hub at {addr}. Settings: {}",
                v["label"].as_str().unwrap_or(&label),
                p.display()
            );
            say!("Start it with: switchboard-node run");
            0
        }
        Err(e) => fail(&format!("Paired, but could not save the settings ({e}). Pair again with a new code."), 1),
    }
}

async fn status() -> i32 {
    let keys = control::status_lines().await;
    let code = match load() {
        Ok(c) => {
            if c.hub.is_empty() {
                say!("Linked as {} through the home computer.", c.machine);
            } else {
                say!("Linked as {} to the hub at {}.", c.machine, c.hub);
            }
            say!("sb.py: {}  python: {}", c.sb(), c.python());
            if cfg!(windows) {
                say!("{WINDOWS_RETURNS}");
            }
            0
        }
        // (the home computer runs keys without being a machine in its own list)
        Err(_) if keys.is_some() => 0,
        Err(e) => fail(&format!("{e} Get a code from Add a machine, then run: switchboard-node pair <hub> <code>"), 1),
    };
    match keys {
        Some(lines) => lines.iter().for_each(|l| say!("{l}")),
        None => say!("Keys: off"),
    }
    code
}

fn fail(msg: &str, code: i32) -> i32 {
    complain!("switchboard-node: {msg}");
    code
}

/// Where `install` keeps the node: a fixed place the service can start it from.
fn install_path() -> std::path::PathBuf {
    if cfg!(windows) {
        let base = std::env::var_os("LOCALAPPDATA")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| home().join("AppData/Local"));
        base.join("Switchboard").join("switchboard-node.exe")
    } else {
        home().join(".local/bin/switchboard-node")
    }
}

fn sh(cmd: &str, args: &[&str]) -> Result<String, String> {
    let out = std::process::Command::new(cmd).args(args).output().map_err(|e| format!("could not run {cmd} ({e})"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        Err(format!("{cmd} {} failed: {}", args.first().unwrap_or(&""), String::from_utf8_lossy(&out.stderr).trim()))
    }
}

/// The node's own program file, for telling an update's leftover process from a second node (Windows).
///
/// An update there cannot stop or replace a node that is running: it moves the file aside
/// (switchboard-node.<pid>.old) and starts the new one, which takes the link. The old process then held on for
/// good, one more with every update, each keeping a file the next update could not clear.
pub struct Own {
    /// where this process's program was when it started
    started: PathBuf,
    installed: PathBuf,
}

impl Own {
    fn here() -> Option<Own> {
        Some(Own { started: std::env::current_exe().ok()?, installed: install_path() })
    }

    /// True when this process is one an update moved aside. A moved-aside file that can be deleted is a leftover
    /// from a node that has already gone; one that cannot be is a program still running, this one.
    pub fn superseded(&self) -> bool {
        let Some(dir) = self.installed.parent() else { return false };
        let names: Vec<String> = std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        match moved_aside(&self.started, &self.installed, &names) {
            Aside::Yes => true,
            Aside::No => false,
            Aside::IfRunning(files) => files.iter().any(|n| {
                let p = dir.join(n);
                std::fs::remove_file(&p).is_err() && p.exists()
            }),
        }
    }
}

#[derive(Debug, PartialEq)]
enum Aside {
    /// this program's own path is a moved-aside file
    Yes,
    /// it did not start from where the node is installed, or nothing there was moved aside
    No,
    /// it started from where the node is installed, and these files next to it were moved aside
    IfRunning(Vec<String>),
}

/// The decision behind `Own::superseded`, from the names in the install folder.
fn moved_aside(started: &Path, installed: &Path, names: &[String]) -> Aside {
    let same = |a: &Path, b: &Path| a.to_string_lossy().eq_ignore_ascii_case(&b.to_string_lossy());
    let stem = installed.file_stem().map(|s| s.to_string_lossy().to_lowercase()).unwrap_or_default();
    // "switchboard-node.<pid>.old", as `install` names the one it moves aside
    let is_aside = |name: &str| {
        let name = name.to_lowercase();
        name.strip_prefix(&format!("{stem}."))
            .and_then(|r| r.strip_suffix(".old"))
            .is_some_and(|pid| !pid.is_empty() && pid.bytes().all(|b| b.is_ascii_digit()))
    };
    let own = started.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    if started.parent().zip(installed.parent()).is_some_and(|(a, b)| same(a, b)) && is_aside(&own) {
        return Aside::Yes;
    }
    if !same(started, installed) {
        return Aside::No;
    }
    let files: Vec<String> = names.iter().filter(|n| is_aside(n)).cloned().collect();
    if files.is_empty() { Aside::No } else { Aside::IfRunning(files) }
}

/// A path as text inside a plist.
fn xml(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// A path as one quoted word of a systemd `ExecStart=`: systemd reads `%` and `$` specially there, and `\` and
/// `"` inside the quotes.
fn unit_word(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"").replace('%', "%%").replace('$', "$$"))
}

/// The user service text for each OS (LaunchAgent plist, systemd unit, the hidden launcher a logon task runs).
fn service_text(exe: &str, home: &str) -> String {
    if cfg!(target_os = "macos") {
        let (exe, home) = (xml(exe), xml(home));
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>Label</key><string>{LABEL}</string>
  <key>ProgramArguments</key><array><string>{exe}</string><string>run</string></array>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><true/>
  <key>EnvironmentVariables</key><dict><key>PATH</key><string>{home}/.local/bin:/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin</string></dict>
  <key>StandardOutPath</key><string>{home}/Library/Logs/switchboard-node.log</string>
  <key>StandardErrorPath</key><string>{home}/Library/Logs/switchboard-node.log</string>
</dict></plist>
"#
        )
    } else if cfg!(windows) {
        launcher_text(Path::new(exe).file_name().and_then(|n| n.to_str()).unwrap_or("switchboard-node.exe"))
    } else {
        format!(
            "[Unit]\nDescription=Switchboard node (links this machine to the hub)\nAfter=network-online.target\n\n[Service]\n\
             ExecStart={} run\nRestart=always\nRestartSec=5\n\n[Install]\nWantedBy=default.target\n",
            unit_word(exe)
        )
    }
}

/// The hidden launcher the Windows logon task runs (switchboard-node.vbs, beside the program): it starts the node,
/// waits for it to end, and starts it again five seconds later, for as long as the launcher file is there. So a
/// node that crashed or was ended in Task Manager is back within seconds, the way launchd (KeepAlive) and systemd
/// (Restart=always) bring it back on a Mac and on Linux, and an update that put a new program in place is what
/// the next turn starts.
///
/// - The program is found beside the script, so the text holds no path: a user folder with an accent in it cannot
///   be misread (a .vbs is read in the system code page), and the text is the same on every computer.
/// - `On Error Resume Next`: a program that is missing for a moment (an update is swapping it) is tried again,
///   never a message box on the desktop.
/// - `uninstall` removes the launcher file, which ends the loop even if the task could not be ended.
fn launcher_text(exe_name: &str) -> String {
    [
        "' Switchboard: keeps this computer's node running. Written by \"switchboard-node install\".",
        "On Error Resume Next",
        "Set sh = CreateObject(\"WScript.Shell\")",
        "Set fs = CreateObject(\"Scripting.FileSystemObject\")",
        &format!("exe = fs.BuildPath(fs.GetParentFolderName(WScript.ScriptFullName), \"{exe_name}\")"),
        "Do While fs.FileExists(WScript.ScriptFullName)",
        "  sh.Run \"\"\"\" & exe & \"\"\" run\", 0, True",
        "  WScript.Sleep 5000",
        "Loop",
        "",
    ]
    .join("\r\n")
}

/// What `status` and `install` say about when the node on a Windows PC is running: its task starts at this user's
/// logon, so after a restart or a power cut it is back when they sign in, not before.
const WINDOWS_RETURNS: &str = "It is started again if it stops, and after a restart or a power cut it returns when \
                               you sign in to Windows.";

const LABEL: &str = "com.switchboard.node";
/// The Windows logon task.
const TASK: &str = "SwitchboardNode";

/// The Windows logon task: this user's logon only, no window, never stopped for running long or on battery.
fn task_xml(user: &str, vbs: &str) -> String {
    let x = |s: &str| s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;");
    let (user, vbs) = (x(user), x(vbs));
    format!(
        r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo><Description>Switchboard: links this machine to the hub</Description></RegistrationInfo>
  <Triggers><LogonTrigger><Enabled>true</Enabled><UserId>{user}</UserId></LogonTrigger></Triggers>
  <Principals><Principal id="Author"><UserId>{user}</UserId><LogonType>InteractiveToken</LogonType><RunLevel>LeastPrivilege</RunLevel></Principal></Principals>
  <Settings><MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy><DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries><ExecutionTimeLimit>PT0S</ExecutionTimeLimit><Enabled>true</Enabled>
    <AllowStartOnDemand>true</AllowStartOnDemand></Settings>
  <Actions Context="Author"><Exec><Command>wscript.exe</Command><Arguments>"{vbs}"</Arguments></Exec></Actions>
</Task>
"#
    )
}

/// Whether the logon task, as `schtasks /Query /XML` prints it, already runs this launcher. An update then leaves
/// the task and the launcher alone and only swaps the program: registering a logon task that starts a script is
/// what malware does too, and an antivirus that sees a program do it again with every update can take the node
/// for one and remove it.
fn task_runs(xml: &str, vbs: &str) -> bool {
    let x = |s: &str| s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").to_lowercase();
    let xml = xml.to_lowercase().replace("&quot;", "\"");
    xml.contains("<command>wscript.exe</command>") && xml.contains(&format!("<arguments>\"{}\"</arguments>", x(vbs)))
}

/// UTF-16 with a byte order mark, as schtasks /XML reads it.
fn utf16(s: &str) -> Vec<u8> {
    [0xFF, 0xFE].into_iter().chain(s.encode_utf16().flat_map(|u| u.to_le_bytes())).collect()
}

/// `install`: copy this binary to a fixed place, run it at login as a user service, and start it now.
fn install() -> i32 {
    if load().is_err() {
        return fail("Pair this machine first: switchboard-node pair <hub> <code>", 2);
    }
    let dest = install_path();
    let res = (|| -> Result<String, String> {
        let me = std::env::current_exe().map_err(|e| e.to_string())?;
        let home_s = home().display().to_string();
        if me.canonicalize().ok() != dest.canonicalize().ok() {
            let dir = dest.parent().ok_or("no folder")?;
            std::fs::create_dir_all(dir).map_err(|e| format!("could not make {} ({e})", dir.display()))?;
            let tmp = dest.with_extension("new");
            std::fs::copy(&me, &tmp).map_err(|e| format!("could not copy the node ({e})"))?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())?;
            }
            // A node that is running can't be replaced on Windows, but it can be renamed: move it aside first.
            // Old ones still running stay until they exit; the next install clears them.
            if cfg!(windows) && dest.exists() {
                for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
                    if e.file_name().to_string_lossy().ends_with(".old") {
                        let _ = std::fs::remove_file(e.path());
                    }
                }
                let _ = std::fs::rename(&dest, dest.with_extension(format!("{}.old", std::process::id())));
            }
            std::fs::rename(&tmp, &dest).map_err(|e| format!("could not put the node in {} ({e})", dest.display()))?;
        }
        let exe = dest.display().to_string();
        let text = service_text(&exe, &home_s);
        if cfg!(target_os = "macos") {
            let plist = home().join(format!("Library/LaunchAgents/{LABEL}.plist"));
            std::fs::create_dir_all(plist.parent().unwrap()).map_err(|e| e.to_string())?;
            std::fs::write(&plist, text).map_err(|e| e.to_string())?;
            let uid = sh("id", &["-u"])?;
            let _ = sh("launchctl", &["bootout", &format!("gui/{uid}/{LABEL}")]);
            // The old node may still be on its way out, and until it is gone launchd refuses the new one
            // ("Bootstrap failed: 5"), which would leave no node running: ask again for a few seconds.
            let mut up = Err(String::new());
            for _ in 0..20 {
                up = sh("launchctl", &["bootstrap", &format!("gui/{uid}"), &plist.display().to_string()]);
                if up.is_ok() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(500));
            }
            if let Err(e) = up {
                // Still refused: the old service never left. Starting that one again runs the new program, which
                // is already in its place, and beats leaving this machine with no node at all.
                sh("launchctl", &["kickstart", "-k", &format!("gui/{uid}/{LABEL}")]).map_err(|_| e)?;
            }
            Ok(format!("Runs at login (LaunchAgent {LABEL}); log in ~/Library/Logs/switchboard-node.log."))
        } else if cfg!(windows) {
            let vbs = dest.with_file_name("switchboard-node.vbs");
            let vbs_s = vbs.display().to_string();
            // An update finds the task as the first install left it, and leaves it alone (see `task_runs`). The
            // launcher is a file of its own: it is written again only when its text changed, and that does not
            // register anything.
            if !std::fs::read_to_string(&vbs).is_ok_and(|t| t == text) {
                std::fs::write(&vbs, &text).map_err(|e| format!("could not write {} ({e})", vbs.display()))?;
            }
            if !sh("schtasks", &["/Query", "/TN", TASK, "/XML"]).is_ok_and(|x| task_runs(&x, &vbs_s)) {
                // "/SC ONLOGON" means any user's logon, which only an administrator may set: from a plain
                // PowerShell window it is "Access is denied". A task whose logon trigger names this user needs no
                // rights.
                let user = sh("whoami", &[])?;
                let xml = dest.with_file_name("switchboard-node-task.xml");
                std::fs::write(&xml, utf16(&task_xml(&user, &vbs_s)))
                    .map_err(|e| format!("could not write {} ({e})", xml.display()))?;
                let made = sh("schtasks", &["/Create", "/F", "/TN", TASK, "/XML", &xml.display().to_string()]);
                let _ = std::fs::remove_file(&xml);
                made?;
            }
            // The launcher loops for as long as the task runs, and a task that is running ignores a start
            // (IgnoreNew): the one from before is ended first, now that the new program is in place, so the
            // launcher that starts next runs the new one. A node the old launcher left behind gives up the link
            // to the new node and exits (`Own::superseded`).
            let _ = sh("schtasks", &["/End", "/TN", TASK]);
            sh("schtasks", &["/Run", "/TN", TASK])?;
            // A launcher that was still on its way out when that start came makes the task ignore it. Asking
            // once more costs nothing: with the launcher running, it is the one that is ignored.
            std::thread::sleep(Duration::from_secs(2));
            let _ = sh("schtasks", &["/Run", "/TN", TASK]);
            Ok(format!("Runs at logon (task {TASK}), with no window. {WINDOWS_RETURNS}"))
        } else {
            let unit = home().join(".config/systemd/user/switchboard-node.service");
            std::fs::create_dir_all(unit.parent().unwrap()).map_err(|e| e.to_string())?;
            std::fs::write(&unit, text).map_err(|e| e.to_string())?;
            sh("systemctl", &["--user", "daemon-reload"])?;
            sh("systemctl", &["--user", "enable", "switchboard-node"])?;
            sh("systemctl", &["--user", "restart", "switchboard-node"])?;
            Ok("Runs as the user service switchboard-node. To keep it up while logged out: loginctl enable-linger"
                .into())
        }
    })();
    match res {
        Ok(how) => {
            say!("switchboard-node is installed at {}. {how}", dest.display());
            0
        }
        Err(e) => fail(&e, 1),
    }
}

/// `uninstall`: stop the node, take it out of what starts it at login, and remove the program, node.json (with the
/// token) and node.lock. Says each thing it removed. This computer's key and its list of paired devices stay,
/// so installing again needs no pairing again, unless `--purge` is given. See uninstall.rs.
fn uninstall(args: &[String]) -> i32 {
    use uninstall::{Os, Places};
    let purge = match args {
        [] => false,
        [a] if a == "--purge" => true,
        _ => return usage(),
    };
    let (config, program) = (config_path(), install_path());
    let key_places = keys::Places::new(&config, &home(), &|k: &str| env(k));
    let machine = load_from(&config).ok().map(|c| c.machine);
    let same = |a: &Path, b: &Path| a.canonicalize().ok().zip(b.canonicalize().ok()).is_some_and(|(a, b)| a == b);
    let copies = uninstall::config_copies(&config);
    let places = Places {
        os: Os::here(),
        home: home(),
        lock: lock_path(&config),
        config,
        uid: if cfg!(target_os = "macos") { sh("id", &["-u"]).unwrap_or_default() } else { String::new() },
        pid: std::process::id(),
        running_from_program: std::env::current_exe().is_ok_and(|me| same(&me, &program)),
        old: if cfg!(windows) { uninstall::old_programs(&program) } else { vec![] },
        copies,
        program,
        purge,
        // (the home computer's list of devices and the local blocks go with the key they belong to)
        key: uninstall::with_copies(&key_places.key)
            .into_iter()
            .chain([key_places.lock])
            .chain(uninstall::with_copies(&key_places.roster))
            .chain(uninstall::with_copies(&key_places.blocked))
            .collect(),
        paired: uninstall::with_copies(&key_places.paired)
            .into_iter()
            .chain(uninstall::with_copies(&key_places.roster_state))
            .collect(),
    };
    let sh = |cmd: &str, args: &[String]| sh(cmd, &args.iter().map(String::as_str).collect::<Vec<_>>());
    let patience = if cfg!(windows) { uninstall::WINDOWS_PATIENCE } else { Duration::ZERO };
    // Each line is printed as its step is done, and the reminder with the first of them: on a Mac and on Linux
    // the last step stops the node, which may be what this is running under.
    let mut any = false;
    let failed = uninstall::carry_out_saying(&uninstall::plan(&places), &sh, patience, &mut |line| {
        if !any && let Some(m) = &machine {
            say!("Switchboard still lists this machine, as {m}, until you remove it in Settings, Machines.");
        }
        any = true;
        say!("{line}");
    });
    if !any {
        say!("switchboard-node was not installed on this machine: there was nothing to remove.");
        return 0;
    }
    if failed { fail("not everything could be removed (see above).", 1) } else { 0 }
}

/// How long `run` waits before it asks the hub again about a token the hub turned down or another node took:
/// `first`, then twice as long each time, up to `max`.
#[derive(Clone, Copy)]
struct Slow {
    first: Duration,
    max: Duration,
    /// Being replaced this many times in a row is another computer with this token, not a node that came and
    /// went: the node then stops taking the link back.
    replaced_most: u32,
    /// ... unless it had the link for this long in between, which starts the count again.
    steady: Duration,
}

const SLOW: Slow = Slow {
    first: Duration::from_secs(5 * 60),
    max: Duration::from_secs(30 * 60),
    replaced_most: 3,
    steady: Duration::from_secs(60 * 60),
};

/// A token `run` is not using for now, and when it asks the hub about it again.
struct Parked {
    token: String,
    replaced: bool,
    /// how many times in a row it was replaced
    times: u32,
    wait: Duration,
    /// None: not again, until node.json has another token (or the node is started again)
    until: Option<Instant>,
}

/// The file a node holds locked while it has the link (link.rs): node.lock next to node.json.
fn lock_path(config: &Path) -> PathBuf {
    config.with_extension("lock")
}

fn minutes(d: Duration) -> String {
    match d.as_secs().div_ceil(60) {
        1 => "1 minute".into(),
        m => format!("{m} minutes"),
    }
}

/// What to type to pair this computer again: with the hub's address where it has one, and with a code or a link
/// from the home computer where it is linked through that (node.json then has no address).
fn pair_again(hub: &str) -> String {
    if hub.is_empty() {
        "switchboard-node pair <code or link>".into()
    } else {
        format!("switchboard-node pair {hub} <code>")
    }
}

/// The line logged when the hub turned this machine's token down.
fn turned_down(why: &str, hub: &str, next: &str, most: &str) -> String {
    let (again, whom) = (pair_again(hub), if hub.is_empty() { "the home computer" } else { "the hub" });
    format!(
        "{why} Pair it again: {again}. Until then this node asks {whom} again in {next}, then less often, up to \
         every {most}."
    )
}

/// The line logged when this node stops taking the link back from another computer.
fn taken_for_good(times: u32, hub: &str) -> String {
    format!(
        "another switchboard-node took over the link {times} times in a row: another computer is using this \
         machine's link (a copy of this disk, or of node.json). This node has stopped taking it back. Pair this \
         computer again so it has a link of its own: {}",
        pair_again(hub)
    )
}

/// `run`: link with the token in node.json. With no token it waits for `pair` to write one instead of exiting,
/// since the user service (KeepAlive, Restart=always) would only start it again at once, over and over.
///
/// A token the hub turned down, or one another node took the link with, is not given up for good: a hub that
/// lost its list of machines and got it back, or a second node run by hand for a minute, would otherwise leave
/// this machine offline until someone signed in again. It is tried again on a slow timer (`slow`: minutes, each
/// wait twice the last), so a machine that really was removed costs the hub next to nothing, and at once when
/// `pair` writes a new token. Each change is logged once.
///
/// Two nodes with one token take the link from each other in turns, and every turn drops the terminals open on
/// it. On one computer that is seen and avoided: a replaced node does not try while another node here holds
/// node.lock. Another computer with the same token (a cloned disk, a restored backup, a copied node.json) cannot
/// be seen that way, so a node that is replaced `slow.replaced_most` times in a row stops for good and says so;
/// it starts again when `pair` writes a new token, or when it is restarted.
async fn run(path: &Path, every: Duration, slow: Slow, keys: Option<std::sync::Arc<keys::Shared>>) -> i32 {
    let own = Own::here();
    // keys look after themselves beside the link, and do nothing at all until they are turned on (keys.rs)
    let watching = keys.clone().map(|k| tokio::spawn(k.supervise(every)));
    let code = run_link(path, every, slow, own, keys.as_ref()).await;
    if let (Some(w), Some(k)) = (watching, keys) {
        w.abort();
        k.shutdown().await;
    }
    code
}

async fn run_link(
    path: &Path,
    every: Duration,
    slow: Slow,
    own: Option<Own>,
    keys: Option<&std::sync::Arc<keys::Shared>>,
) -> i32 {
    let lock = lock_path(path);
    let mut parked = None::<Parked>;
    let mut said = false;
    loop {
        match load_from(path) {
            Ok(c) => {
                let fresh = parked.as_ref().is_none_or(|p| p.token != c.token);
                let due = parked.as_ref().is_some_and(|p| {
                    p.until.is_some_and(|t| Instant::now() >= t) && !(p.replaced && link::held_elsewhere(&lock))
                });
                if fresh || due {
                    said = false;
                    let before = parked.take().filter(|_| !fresh);
                    let began = Instant::now();
                    let (why, linked, replaced) =
                        match link::run(c.clone(), own.as_ref(), &lock, before.is_some(), keys).await {
                            link::End::Exit(code) => return code,
                            link::End::Unpaired { why, linked } => (why, linked, false),
                            link::End::Replaced => (String::new(), true, true),
                        };
                    // the same answer again, soon: wait twice as long. Anything else starts over.
                    let wait = match &before {
                        Some(p) if p.replaced == replaced && began.elapsed() < slow.max * 2 => {
                            (p.wait * 2).min(slow.max)
                        }
                        _ => slow.first,
                    };
                    // replaced again without having kept the link for long in between
                    let times = match &before {
                        _ if !replaced => 0,
                        Some(p) if p.replaced && began.elapsed() < slow.steady => p.times + 1,
                        _ => 1,
                    };
                    let stop = replaced && times >= slow.replaced_most;
                    if stop {
                        link::log(taken_for_good(times, &c.hub));
                    } else if linked || before.is_none_or(|p| p.replaced != replaced) {
                        let (next, most) = (minutes(wait), minutes(slow.max));
                        link::log(if replaced {
                            format!(
                                "another switchboard-node for this machine took over the link. This one waits, and \
                                 takes the link back in {next} if no node on this computer has it (then less often, \
                                 up to every {most})."
                            )
                        } else {
                            turned_down(&why, &c.hub, &next, &most)
                        });
                    }
                    let until = (!stop).then(|| Instant::now() + wait);
                    parked = Some(Parked { token: c.token, replaced, times, wait, until });
                }
            }
            // (the home computer need not be a machine in its own list: with keys on, that is not worth a word)
            Err(_) if !said && keys.is_some_and(|k| k.role() == keys::Role::Home) => said = true,
            Err(e) if !said => {
                fail(&format!("{e} Get a code from Add a machine, then run: switchboard-node pair <hub> <code>"), 2);
                said = true;
            }
            Err(_) => {}
        }
        tokio::select! {
            _ = tokio::time::sleep(every) => {}
            _ = link::shutdown() => return 0,
        }
    }
}

fn usage() -> i32 {
    complain!("usage: switchboard-node pair <hub> <code> [--name NAME] [--label LABEL] [--replace]");
    complain!("       switchboard-node pair <code-or-link> [--name NAME] [--addr IP:PORT] [--replace]");
    complain!("       switchboard-node code");
    complain!("       switchboard-node devices");
    complain!("       switchboard-node forget <name>");
    complain!("       switchboard-node relay <url> | none");
    complain!("       switchboard-node block <name> | unblock <name>");
    complain!("       switchboard-node home status");
    complain!("       switchboard-node home move <computer> [--retire] | --resume | --undo");
    complain!("       switchboard-node home backup [--to FILE]");
    complain!("       switchboard-node home restore <file>");
    complain!("       switchboard-node install");
    complain!("       switchboard-node run");
    complain!("       switchboard-node status");
    complain!("       switchboard-node uninstall [--purge]");
    2
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = match args.first().map(String::as_str) {
        Some("pair") => pair(&args[1..]).await,
        Some("run") => match keys::Shared::here(&config_path()) {
            Ok(keys) => run(&config_path(), RECHECK, SLOW, Some(keys)).await,
            Err(e) => fail(&e, 2),
        },
        Some("status") => status().await,
        Some("code") => control::code().await,
        Some("devices") => control::devices_cmd().await,
        Some("forget") => match args.get(1) {
            Some(name) => control::forget(name).await,
            None => usage(),
        },
        Some("relay") => match args.get(1) {
            Some(url) => control::relay_cmd(url).await,
            None => usage(),
        },
        Some(verb @ ("block" | "unblock")) => match args.get(1) {
            Some(name) => control::block_cmd(name, verb == "block").await,
            None => usage(),
        },
        Some("home") => control::home_cmd(&args[1..]).await,
        Some("install") => install(),
        Some("uninstall") => uninstall(&args[1..]),
        Some("--version" | "version") => {
            say!("switchboard-node {VERSION}");
            0
        }
        _ => usage(),
    };
    std::process::exit(code);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};

    #[test]
    fn node_json_is_in_the_settings_folder_unless_its_own_variable_names_a_file() {
        let home = Path::new("/home/you");
        let with = |vars: &'static [(&'static str, &'static str)]| {
            config_path_from(home, &move |k: &str| vars.iter().find(|(n, _)| *n == k).map(|(_, v)| v.to_string()))
        };
        assert_eq!(with(&[]), home.join(".config/switchboard/node.json"), "where every install has it");
        assert_eq!(with(&[("SWITCHBOARD_CONFIG_DIR", "/t/cfg")]), PathBuf::from("/t/cfg/node.json"));
        assert_eq!(
            with(&[("SWITCHBOARD_CONFIG_DIR", "/t/cfg"), ("SWITCHBOARD_NODE_CONFIG", "/n/node.json")]),
            PathBuf::from("/n/node.json")
        );
        assert_eq!(with(&[("SWITCHBOARD_CONFIG_DIR", " "), ("SWITCHBOARD_NODE_CONFIG", "")]), with(&[]));
    }

    #[test]
    fn a_link_an_earlier_windows_version_kept_under_home_is_named_and_never_used() {
        let with = |windows: bool, vars: &'static [(&'static str, &'static str)]| {
            left_behind(windows, &move |k: &str| vars.iter().find(|(n, _)| *n == k).map(|(_, v)| v.to_string()))
        };
        let two: &[(&str, &str)] = &[("HOME", "C:/msys/home/you"), ("USERPROFILE", "C:/Users/you")];
        assert_eq!(with(true, two), Some(PathBuf::from("C:/msys/home/you/.config/switchboard/node.json")));
        // one folder under both names (an ssh session), or only one of the two set: there is no other file
        assert_eq!(with(true, &[("HOME", "C:/Users/you"), ("USERPROFILE", "C:/Users/you")]), None);
        assert_eq!(with(true, &[("USERPROFILE", "C:/Users/you")]), None);
        assert_eq!(with(true, &[("HOME", "C:/msys/home/you")]), None);
        // the user said where the file is: nothing is guessed
        for said in ["SWITCHBOARD_NODE_CONFIG", "SWITCHBOARD_CONFIG_DIR"] {
            let get = move |k: &str| match k {
                "HOME" => Some("C:/msys/home/you".to_string()),
                "USERPROFILE" => Some("C:/Users/you".to_string()),
                k if k == said => Some("C:/t".to_string()),
                _ => None,
            };
            assert_eq!(left_behind(true, &get), None, "{said}");
        }
        assert_eq!(with(false, two), None, "HOME is the folder everywhere else");
        let says = left_behind_says(Path::new("C:/old/node.json"), Path::new("C:/new/node.json"));
        assert_eq!(
            says,
            "There is a link in C:/old/node.json, where earlier versions kept it when HOME was set. To use it, copy \
             that file to C:/new/node.json."
        );
    }

    #[test]
    fn pair_does_not_write_over_a_link_to_another_hub_unless_told_to_replace_it() {
        let linked = |hub: &str, token: &str| NodeConfig {
            hub: hub.into(),
            machine: "tower".into(),
            token: token.into(),
            sb: None,
            python: None,
            home: None,
            relay: None,
        };
        let path = Path::new("/home/you/.config/switchboard/node.json");
        let old = linked("192.0.2.10:8132", "t");
        let why = other_hub(Some(&old), "127.0.0.1:8151", false, path).expect("another hub is refused");
        assert_eq!(
            why,
            "This computer is linked to the hub at 192.0.2.10:8132 already \
             (/home/you/.config/switchboard/node.json), so nothing was changed. To link it to the hub at \
             127.0.0.1:8151 instead, run the same command again with --replace at the end."
        );
        assert!(!why.contains("  "), "{why:?}");
        // said outright, it is replaced
        assert_eq!(other_hub(Some(&old), "127.0.0.1:8151", true, path), None);
        // the same hub again is how a computer gets a new token, however the name is typed
        assert_eq!(other_hub(Some(&old), "192.0.2.10:8132", false, path), None);
        let named = linked("Tower.tail1234.ts.net:8132", "t");
        assert_eq!(other_hub(Some(&named), "tower.tail1234.ts.net:8132", false, path), None);
        // nothing to lose: no file, one that links nothing, one that only holds what keys keep
        assert_eq!(other_hub(None, "127.0.0.1:8151", false, path), None);
        assert_eq!(other_hub(Some(&linked("192.0.2.10:8132", "")), "127.0.0.1:8151", false, path), None);
        assert_eq!(other_hub(Some(&linked("", "t")), "127.0.0.1:8151", false, path), None);
    }

    #[test]
    fn what_a_node_that_was_turned_down_says() {
        // linked through the home computer there is no address to name: a code or a link pairs it again
        assert_eq!(
            turned_down("The hub unlinked this machine.", "", "5 minutes", "30 minutes"),
            "The hub unlinked this machine. Pair it again: switchboard-node pair <code or link>. Until then this \
             node asks the home computer again in 5 minutes, then less often, up to every 30 minutes."
        );
        assert_eq!(
            turned_down("The hub unlinked this machine.", "192.0.2.10:8132", "5 minutes", "30 minutes"),
            "The hub unlinked this machine. Pair it again: switchboard-node pair 192.0.2.10:8132 <code>. Until then \
             this node asks the hub again in 5 minutes, then less often, up to every 30 minutes."
        );
        for hub in ["", "192.0.2.10:8132"] {
            let line = taken_for_good(3, hub);
            assert!(!line.contains("  "), "no runs of spaces: {line:?}");
            assert!(
                line.starts_with("another switchboard-node took over the link 3 times in a row: another computer is")
            );
        }
        assert!(taken_for_good(3, "").ends_with("a link of its own: switchboard-node pair <code or link>"));
        assert!(taken_for_good(3, "192.0.2.10:8132").ends_with("switchboard-node pair 192.0.2.10:8132 <code>"));
    }

    #[test]
    fn a_line_to_a_closed_pipe_is_dropped_not_a_panic() {
        struct Closed;
        impl std::io::Write for Closed {
            fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
                Err(std::io::ErrorKind::BrokenPipe.into())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Err(std::io::ErrorKind::BrokenPipe.into())
            }
        }
        write_line(&mut Closed, format_args!("Code: {}", 7));
        let mut out = vec![];
        write_line(&mut out, format_args!("Code: {}", 7));
        assert_eq!(out, b"Code: 7\n");
    }

    #[test]
    fn hub_addresses() {
        assert_eq!(hub_addr("192.0.2.10").unwrap(), "192.0.2.10:8132");
        assert_eq!(hub_addr("http://192.0.2.10:8151/").unwrap(), "192.0.2.10:8151");
        assert_eq!(hub_addr("ws://tower.tail1234.ts.net").unwrap(), "tower.tail1234.ts.net:8132");
        assert_eq!(hub_addr("[fd7a::1]").unwrap(), "[fd7a::1]:8132");
        assert_eq!(hub_addr("[fd7a::1]:9").unwrap(), "[fd7a::1]:9");
        for bad in ["", "-oProxy", "a b", "host;rm"] {
            assert!(hub_addr(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn an_address_that_asks_for_tls_is_refused_not_downgraded() {
        for s in ["https://hub.example.com", "wss://192.0.2.10:8132/", " HTTPS://hub.example.com:443 "] {
            let e = hub_addr(s).unwrap_err();
            assert!(e.contains("does not have yet") && e.contains(s.trim()), "{e}");
            assert!(e.contains("http://192.0.2.10:8132"), "it says what to type instead: {e}");
        }
        // plain addresses are read as before
        assert_eq!(hub_addr("http://192.0.2.10").unwrap(), "192.0.2.10:8132");
        assert_eq!(hub_addr("ws://192.0.2.10:9").unwrap(), "192.0.2.10:9");
        assert_eq!(hub_addr("https.example.com").unwrap(), "https.example.com:8132");
        // a node.json edited by hand to say https is refused when it is read, before any token is sent
        let d = scratch("tls");
        let path = d.join("node.json");
        let json = |hub: &str| format!(r#"{{"hub": "{hub}", "machine": "lab", "token": "t"}}"#);
        std::fs::write(&path, json("wss://hub.example.com:8132")).unwrap();
        let e = load_from(&path).unwrap_err();
        assert!(e.contains("node.json") && e.contains("wss://"), "{e}");
        std::fs::write(&path, json("192.0.2.10:8132")).unwrap();
        assert_eq!(load_from(&path).unwrap().hub, "192.0.2.10:8132");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn service_runs_the_installed_node() {
        let t = service_text("/home/you/.local/bin/switchboard-node", "/home/you");
        assert!(t.contains("switchboard-node"));
        assert!(t.contains("run"));
        assert!(install_path().ends_with(if cfg!(windows) { "switchboard-node.exe" } else { "switchboard-node" }));
    }

    #[test]
    fn service_text_keeps_odd_folder_names_whole() {
        // a home folder with a space, an ampersand and a percent sign
        let t = service_text("/opt/R&D <lab> 100%/.local/bin/switchboard-node", "/opt/R&D <lab> 100%");
        if cfg!(target_os = "macos") {
            assert!(t.contains("<string>/opt/R&amp;D &lt;lab&gt; 100%/.local/bin/switchboard-node</string>"), "{t}");
            assert!(t.contains("<string>/opt/R&amp;D &lt;lab&gt; 100%/Library/Logs/switchboard-node.log</string>"));
            assert!(!t.contains("R&D"), "{t}");
        }
        assert_eq!(xml("/opt/a&b/<c>"), "/opt/a&amp;b/&lt;c&gt;");
        assert_eq!(unit_word("/home/you/.local/bin/switchboard-node"), "\"/home/you/.local/bin/switchboard-node\"");
        assert_eq!(unit_word("/srv/R&D 100%/a$b/c\"d\\e"), "\"/srv/R&D 100%%/a$$b/c\\\"d\\\\e\"");
    }

    #[test]
    fn a_node_an_update_moved_aside_knows_it() {
        let names = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let dir = Path::new(r"C:\Users\you\AppData\Local\Switchboard");
        let installed = dir.join("switchboard-node.exe");
        let (installed, dir_s) = if cfg!(windows) {
            (installed, String::new())
        } else {
            // (off Windows the folder and file are one name with backslashes in it; give it a real folder)
            (Path::new("/opt/sb/switchboard-node.exe").to_path_buf(), "/opt/sb".to_string())
        };
        let at = |name: &str| if cfg!(windows) { dir.join(name) } else { Path::new(&dir_s).join(name) };
        // the node the update started: nothing moved aside is running beside it
        assert_eq!(
            moved_aside(&installed, &installed, &names(&["switchboard-node.exe", "switchboard-node.vbs"])),
            Aside::No
        );
        // the node that was running through an update: its file is now the .old one
        let after = names(&["switchboard-node.exe", "switchboard-node.4242.old", "switchboard-node.vbs", "notes.old"]);
        assert_eq!(
            moved_aside(&installed, &installed, &after),
            Aside::IfRunning(names(&["switchboard-node.4242.old"]))
        );
        // letter case does not matter there
        let shouted = at("SWITCHBOARD-NODE.EXE");
        assert_eq!(
            moved_aside(&shouted, &installed, &names(&["Switchboard-Node.77.OLD"])),
            Aside::IfRunning(names(&["Switchboard-Node.77.OLD"]))
        );
        // a system that reports the path it was moved to
        assert_eq!(moved_aside(&at("switchboard-node.4242.old"), &installed, &[]), Aside::Yes);
        // a second node run by hand from somewhere else is a second node, not a leftover
        let other = Path::new(if cfg!(windows) {
            r"C:\Users\you\Downloads\switchboard-node.exe"
        } else {
            "/tmp/dl/switchboard-node.exe"
        });
        assert_eq!(moved_aside(other, &installed, &after), Aside::No);
        let other_old = Path::new(if cfg!(windows) {
            r"C:\Users\you\Downloads\switchboard-node.1.old"
        } else {
            "/tmp/dl/switchboard-node.1.old"
        });
        assert_eq!(moved_aside(other_old, &installed, &after), Aside::No);
        // names that only look alike
        for n in ["switchboard-node.old", "switchboard-node.x1.old", "switchboard-node.12.old.txt", "other.12.old"] {
            assert_eq!(moved_aside(&installed, &installed, &names(&[n])), Aside::No, "{n}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_leftover_file_from_a_node_that_is_gone_does_not_count() {
        use std::os::unix::fs::PermissionsExt;
        let d = scratch("aside");
        let installed = d.join("switchboard-node");
        std::fs::write(&installed, "new").unwrap();
        let old = d.join("switchboard-node.4242.old");
        std::fs::write(&old, "old").unwrap();
        let own = Own { started: installed.clone(), installed: installed.clone() };
        // the file can be deleted, so nothing runs from it: a stale leftover, cleared
        assert!(!own.superseded());
        assert!(!old.exists());
        // one that cannot be deleted (here: a folder that refuses; on Windows: a program that is running)
        std::fs::write(&old, "old").unwrap();
        std::fs::set_permissions(&d, std::fs::Permissions::from_mode(0o555)).unwrap();
        let stuck = std::fs::remove_file(&old).is_err();
        let said = own.superseded();
        std::fs::set_permissions(&d, std::fs::Permissions::from_mode(0o755)).unwrap();
        if stuck {
            assert!(said); // (as root nothing refuses, and there is nothing to check)
        }
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn the_logon_task_is_this_users_alone() {
        let t = task_xml(r"DESKTOP-1\Pat & Co", r"C:\Users\you\AppData\Local\Switchboard\switchboard-node.vbs");
        assert!(
            t.contains(r"<LogonTrigger><Enabled>true</Enabled><UserId>DESKTOP-1\Pat &amp; Co</UserId></LogonTrigger>")
        );
        assert!(t.contains("<RunLevel>LeastPrivilege</RunLevel>"));
        assert!(t.contains("<ExecutionTimeLimit>PT0S</ExecutionTimeLimit>"));
        assert!(t.contains(r#"<Arguments>"C:\Users\you\AppData\Local\Switchboard\switchboard-node.vbs"</Arguments>"#));
        assert_eq!(&utf16("<T")[..], &[0xFF, 0xFE, b'<', 0, b'T', 0]);
    }

    #[test]
    fn an_update_knows_the_logon_task_it_made() {
        let vbs = r"C:\Users\you\AppData\Local\R&D\Switchboard\switchboard-node.vbs";
        // what install registers
        assert!(task_runs(&task_xml(r"DESKTOP-1\Pat & Co", vbs), vbs));
        // as schtasks prints it back: its own layout and letter case
        let printed = "<Task version=\"1.2\">\r\n  <Actions Context=\"Author\">\r\n    <Exec>\r\n      \
                       <Command>wscript.exe</Command>\r\n      <Arguments>\"c:\\users\\you\\appdata\\local\\r&amp;d\\\
                       Switchboard\\switchboard-node.vbs\"</Arguments>\r\n    </Exec>\r\n  </Actions>\r\n</Task>";
        assert!(task_runs(printed, vbs));
        // a task that starts something else, or none, is made again
        assert!(!task_runs(printed, r"D:\Apps\Switchboard\switchboard-node.vbs"));
        assert!(!task_runs(&printed.replace("wscript.exe", "cmd.exe"), vbs));
        assert!(!task_runs("", vbs));
    }

    /// The launcher of 0.8.22 and before: it started the node once and ended.
    const ONE_SHOT: &str = "CreateObject(\"WScript.Shell\").Run \"\"\"C:\\Users\\you\\AppData\\Local\\Switchboard\\\
                            switchboard-node.exe\"\" run\", 0, False\r\n";

    #[test]
    fn the_windows_launcher_starts_the_node_again_when_it_stops() {
        let t = launcher_text("switchboard-node.exe");
        let lines: Vec<&str> = t.split("\r\n").collect();
        assert_eq!(t.matches('\n').count(), t.matches("\r\n").count(), "Windows line ends throughout: {t:?}");
        assert!(t.ends_with("\r\n"));
        // it waits for the node (True), hidden (0), then waits five seconds and goes round again
        let run = lines.iter().position(|l| l.trim() == r#"sh.Run """" & exe & """ run", 0, True"#).expect("runs it");
        let top = lines.iter().position(|l| l.starts_with("Do While ")).expect("a loop");
        let end = lines.iter().position(|l| *l == "Loop").expect("a loop");
        let nap = lines.iter().position(|l| l.trim() == "WScript.Sleep 5000").expect("a pause");
        assert!(top < run && run < nap && nap < end, "{t}");
        assert!(!t.contains("False"), "it must not start the node and leave: {t}");
        // no message box, whatever goes wrong: the first thing it says
        assert_eq!(lines.iter().find(|l| !l.starts_with('\'')), Some(&"On Error Resume Next"));
        // the program is the one beside the launcher, by name: no folder in the text, so nothing to misread
        assert!(lines.contains(
            &r#"exe = fs.BuildPath(fs.GetParentFolderName(WScript.ScriptFullName), "switchboard-node.exe")"#
        ));
        assert!(t.is_ascii() && !t.contains(":\\") && !t.contains("Users"), "{t}");
        // removing the launcher (uninstall) ends the loop
        assert_eq!(lines[top], "Do While fs.FileExists(WScript.ScriptFullName)");
        // every quote is closed: VBScript reads "" inside a string as one quote
        for l in lines.iter().filter(|l| !l.starts_with('\'')) {
            assert_eq!(l.matches('"').count() % 2, 0, "{l}");
        }
        if cfg!(windows) {
            assert_eq!(
                service_text(r"C:\Users\you\AppData\Local\Switchboard\switchboard-node.exe", r"C:\Users\you"),
                t
            );
        }
    }

    #[test]
    fn an_update_from_the_one_shot_launcher_changes_the_file_and_not_the_task() {
        // The launcher's text is new, so `install` writes it again...
        assert_ne!(launcher_text("switchboard-node.exe"), ONE_SHOT);
        // ...and the task it is run by is the one already registered: same program, same file. Registering it
        // again is what an antivirus takes for malware (`task_runs`).
        let vbs = r"C:\Users\you\AppData\Local\Switchboard\switchboard-node.vbs";
        assert!(task_runs(&task_xml(r"DESKTOP-1\you", vbs), vbs));
    }

    #[test]
    fn the_logon_task_waits_for_a_sign_in_and_holds_no_password() {
        let t = task_xml(r"DESKTOP-1\you", r"C:\Users\you\AppData\Local\Switchboard\switchboard-node.vbs");
        // one trigger: this user's logon. Not at boot, which needs an administrator or a stored password.
        assert_eq!(t.matches("Trigger>").count(), 2, "{t}");
        assert!(t.contains("<LogonTrigger>") && !t.contains("BootTrigger"));
        assert!(t.contains("<LogonType>InteractiveToken</LogonType>"));
        assert!(!t.contains("S4U") && !t.to_lowercase().contains("password"));
        // the launcher loops, so the task runs for as long as the user is signed in: it must never be stopped
        // for running long, and a second start while it runs must not make a second launcher
        assert!(t.contains("<ExecutionTimeLimit>PT0S</ExecutionTimeLimit>"));
        assert!(t.contains("<MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>"));
        assert!(WINDOWS_RETURNS.contains("returns when you sign in"));
    }

    #[test]
    fn a_node_json_of_zeros_comes_back_from_its_copy() {
        let d = scratch("zeros");
        let path = d.join("node.json");
        let cfg = |token: &str| NodeConfig {
            hub: "192.0.2.10:8132".into(),
            machine: "lab".into(),
            token: token.into(),
            sb: None,
            python: None,
            ..Default::default()
        };
        save_to(&path, &cfg("first")).unwrap();
        save_to(&path, &cfg("second")).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            for n in ["node.json", "node.json.bak"] {
                assert_eq!(std::fs::metadata(d.join(n)).unwrap().permissions().mode() & 0o777, 0o600, "{n}");
            }
        }
        // as a power cut leaves it: the machine stays linked, with the token of its last pairing
        std::fs::write(&path, [0u8; 439]).unwrap();
        assert_eq!(load_from(&path).unwrap().token, "second");
        assert_eq!(load_from(&path).unwrap().token, "second", "and the file is whole again");
        assert_eq!(sb_core::durable::bads(&path).len(), 1, "the file as it was found is kept");
        // cut short, and no copy: it says so, writes nothing in its place, and pairing again starts it over
        std::fs::write(&path, r#"{"hub": "192.0.2.10:8132", "mach"#).unwrap();
        std::fs::remove_file(d.join("node.json.bak")).unwrap();
        let e = load_from(&path).unwrap_err();
        assert!(e.contains("is not readable") && e.contains("node.json.bad-") && e.contains("Pair again"), "{e}");
        assert!(!path.exists());
        assert!(load_from(&path).is_err());
        save_to(&path, &cfg("third")).unwrap();
        assert_eq!(load_from(&path).unwrap().token, "third");
        assert_eq!(load_from(&d.join("none.json")).unwrap_err(), "This machine is not paired yet.");
        let _ = std::fs::remove_dir_all(&d);
    }

    fn scratch(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("sbnode-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[tokio::test]
    async fn run_waits_instead_of_exiting_when_not_paired() {
        let d = scratch("unpaired");
        let path = d.join("node.json");
        let gone = tokio::time::timeout(Duration::from_millis(300), run(&path, Duration::from_millis(10), SLOW, None));
        assert!(gone.await.is_err(), "still running, so the service does not start it again and again");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[tokio::test]
    async fn a_new_link_is_taken_up_while_the_old_hub_cannot_be_reached() {
        // the old hub: an address nothing listens on
        let gone = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let old = gone.local_addr().unwrap().to_string();
        drop(gone);
        // the new hub: counts the links it is asked for
        let hub = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let new = hub.local_addr().unwrap().to_string();
        let tries = Arc::new(AtomicUsize::new(0));
        let n = tries.clone();
        tokio::spawn(async move {
            while let Ok((mut s, _)) = hub.accept().await {
                n.fetch_add(1, SeqCst);
                let mut buf = [0u8; 4096];
                let _ = s.read(&mut buf).await;
                let _ = s.write_all(b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\n\r\n").await;
            }
        });
        let d = scratch("relinked");
        let path = d.join("node.json");
        let write = |hub: &str, token: &str| {
            let c = NodeConfig { hub: hub.into(), machine: "lab".into(), token: token.into(), ..Default::default() };
            std::fs::write(&path, serde_json::to_string(&c).unwrap()).unwrap();
        };
        write(&old, "old");
        let node = tokio::spawn({
            let path = path.clone();
            async move { run(&path, Duration::from_millis(10), SLOW, None).await }
        });
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_eq!(tries.load(SeqCst), 0);
        // linked again, to a hub somewhere else, while the helper is still trying the old one
        write(&new, "new");
        for _ in 0..80 {
            if tries.load(SeqCst) > 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert!(tries.load(SeqCst) > 0, "the helper went on trying a hub that is gone");
        node.abort();
        let _ = std::fs::remove_dir_all(&d);
    }

    #[tokio::test]
    async fn a_turned_down_token_is_not_tried_again_until_it_changes() {
        // a hub that answers 401 to every link: the token is unknown there
        let hub = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = hub.local_addr().unwrap().to_string();
        let tries = Arc::new(AtomicUsize::new(0));
        let n = tries.clone();
        tokio::spawn(async move {
            while let Ok((mut s, _)) = hub.accept().await {
                n.fetch_add(1, SeqCst);
                let mut buf = [0u8; 4096];
                let _ = s.read(&mut buf).await;
                let _ = s.write_all(b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\n\r\n").await;
            }
        });
        let d = scratch("rejected");
        let path = d.join("node.json");
        let write = |token: &str| {
            let c = NodeConfig {
                hub: addr.clone(),
                machine: "lab".into(),
                token: token.into(),
                sb: None,
                python: None,
                ..Default::default()
            };
            std::fs::write(&path, serde_json::to_string(&c).unwrap()).unwrap();
        };
        write("old");
        let node = tokio::spawn({
            let path = path.clone();
            async move { run(&path, Duration::from_millis(10), SLOW, None).await }
        });
        tokio::time::sleep(Duration::from_millis(500)).await;
        assert_eq!(tries.load(SeqCst), 1, "once, then it waits");
        assert!(!node.is_finished());
        write("new");
        tokio::time::sleep(Duration::from_millis(500)).await;
        assert_eq!(tries.load(SeqCst), 2, "a new pairing is picked up without a restart");
        node.abort();
        let _ = std::fs::remove_dir_all(&d);
    }

    /// Waits of 100 ms, then 200, then 400 at most: the slow timer, sped up.
    const QUICK: Slow = Slow {
        first: Duration::from_millis(100),
        max: Duration::from_millis(400),
        replaced_most: 3,
        steady: Duration::from_secs(3600),
    };

    #[tokio::test]
    async fn a_turned_down_token_is_asked_about_again_slowly() {
        // a hub that lost its list of machines: 401 to every link, until the list is back
        let hub = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = hub.local_addr().unwrap().to_string();
        let tries = Arc::new(AtomicUsize::new(0));
        let n = tries.clone();
        tokio::spawn(async move {
            while let Ok((mut s, _)) = hub.accept().await {
                n.fetch_add(1, SeqCst);
                let mut buf = [0u8; 4096];
                let _ = s.read(&mut buf).await;
                let _ = s.write_all(b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\n\r\n").await;
            }
        });
        let d = scratch("slow");
        let path = d.join("node.json");
        let c = NodeConfig {
            hub: addr,
            machine: "lab".into(),
            token: "t".into(),
            sb: None,
            python: None,
            ..Default::default()
        };
        std::fs::write(&path, serde_json::to_string(&c).unwrap()).unwrap();
        let node = tokio::spawn({
            let path = path.clone();
            async move { run(&path, Duration::from_millis(10), QUICK, None).await }
        });
        // asked at 0, 100, 300, 700, 1100 and 1500 ms: each wait twice the last, up to the most
        tokio::time::sleep(Duration::from_millis(1600)).await;
        let n = tries.load(SeqCst);
        assert!(n >= 3, "{n}: it never asked again, and stays offline once the hub knows it again");
        assert!(n <= 7, "{n}: it asks far more often than the timer allows");
        assert!(!node.is_finished());
        node.abort();
        let _ = std::fs::remove_dir_all(&d);
    }

    /// A hub where another node with this token already took over: it closes every link as replaced.
    async fn replacing_hub() -> (String, Arc<AtomicUsize>) {
        use futures_util::StreamExt;
        use tokio_tungstenite::tungstenite::protocol::{CloseFrame, frame::coding::CloseCode};
        let hub = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = hub.local_addr().unwrap().to_string();
        let tries = Arc::new(AtomicUsize::new(0));
        let n = tries.clone();
        tokio::spawn(async move {
            while let Ok((s, _)) = hub.accept().await {
                n.fetch_add(1, SeqCst);
                tokio::spawn(async move {
                    let Ok(mut ws) = tokio_tungstenite::accept_async(s).await else { return };
                    let code = CloseCode::from(sb_hub::link_proto::CLOSE_REPLACED);
                    let _ = ws.close(Some(CloseFrame { code, reason: "".into() })).await;
                    while let Some(Ok(_)) = ws.next().await {}
                });
            }
        });
        (addr, tries)
    }

    #[tokio::test]
    async fn a_replaced_node_does_not_take_the_link_back_at_once() {
        let (addr, tries) = replacing_hub().await;
        let d = scratch("replaced");
        let path = d.join("node.json");
        let c = NodeConfig {
            hub: addr,
            machine: "lab".into(),
            token: "same".into(),
            sb: None,
            python: None,
            ..Default::default()
        };
        std::fs::write(&path, serde_json::to_string(&c).unwrap()).unwrap();
        let node = tokio::spawn({
            let path = path.clone();
            async move { run(&path, Duration::from_millis(10), SLOW, None).await }
        });
        tokio::time::sleep(Duration::from_millis(800)).await;
        assert_eq!(tries.load(SeqCst), 1, "it links once and then leaves the link to the other node");
        assert!(!node.is_finished());
        node.abort();
        let _ = std::fs::remove_dir_all(&d);
    }

    #[tokio::test]
    async fn a_replaced_node_takes_the_link_back_only_when_no_node_here_has_it() {
        let (addr, tries) = replacing_hub().await;
        let d = scratch("replaced-again");
        let path = d.join("node.json");
        let c = NodeConfig {
            hub: addr,
            machine: "lab".into(),
            token: "same".into(),
            sb: None,
            python: None,
            ..Default::default()
        };
        std::fs::write(&path, serde_json::to_string(&c).unwrap()).unwrap();
        // the other node on this computer has the link: it holds node.lock
        let other = std::fs::File::create(lock_path(&path)).unwrap();
        other.try_lock().unwrap();
        assert!(link::held_elsewhere(&lock_path(&path)));
        let node = tokio::spawn({
            let path = path.clone();
            async move { run(&path, Duration::from_millis(10), QUICK, None).await }
        });
        tokio::time::sleep(Duration::from_millis(900)).await;
        assert_eq!(tries.load(SeqCst), 1, "the two would take the link from each other in turns");
        // that node is gone (it was run by hand, and stopped): this one is the machine's node again
        drop(other);
        // (a program some other test starts at this moment holds a copy of the file for a few milliseconds)
        let end = std::time::Instant::now() + Duration::from_millis(500);
        while link::held_elsewhere(&lock_path(&path)) && tries.load(SeqCst) == 1 && std::time::Instant::now() < end {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
        assert!(tries.load(SeqCst) > 1 || !link::held_elsewhere(&lock_path(&path)));
        tokio::time::sleep(Duration::from_millis(900)).await;
        let n = tries.load(SeqCst);
        assert!((2..=6).contains(&n), "{n}: it links again, on the slow timer");
        assert!(!node.is_finished());
        node.abort();
        let _ = std::fs::remove_dir_all(&d);
    }

    #[tokio::test]
    async fn a_node_replaced_three_times_in_a_row_stops_taking_the_link_back() {
        // another computer has this token: nothing on this one holds node.lock, and the hub keeps saying replaced
        let (addr, tries) = replacing_hub().await;
        let d = scratch("replaced-elsewhere");
        let path = d.join("node.json");
        let c = NodeConfig {
            hub: addr,
            machine: "lab".into(),
            token: "same".into(),
            sb: None,
            python: None,
            ..Default::default()
        };
        std::fs::write(&path, serde_json::to_string(&c).unwrap()).unwrap();
        let node = tokio::spawn({
            let path = path.clone();
            async move { run(&path, Duration::from_millis(10), QUICK, None).await }
        });
        // (waits of 100 and 200 ms, then it would be every 400 ms for good)
        tokio::time::sleep(Duration::from_millis(2500)).await;
        assert_eq!(tries.load(SeqCst), 3, "the two computers would trade the link for ever");
        assert!(!node.is_finished(), "it stays up, waiting to be paired again");
        // pairing this computer again gives it a token of its own, and it links at once
        let c = NodeConfig { token: "its-own".into(), ..c };
        std::fs::write(&path, serde_json::to_string(&c).unwrap()).unwrap();
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(tries.load(SeqCst) >= 4, "a new token is tried");
        node.abort();
        let _ = std::fs::remove_dir_all(&d);
    }

    #[tokio::test]
    async fn a_link_kept_for_a_long_stretch_starts_the_count_of_replacements_again() {
        // (here every link counts as kept long enough, so no run of three ever builds up)
        let (addr, tries) = replacing_hub().await;
        let d = scratch("replaced-steady");
        let path = d.join("node.json");
        let c = NodeConfig {
            hub: addr,
            machine: "lab".into(),
            token: "same".into(),
            sb: None,
            python: None,
            ..Default::default()
        };
        std::fs::write(&path, serde_json::to_string(&c).unwrap()).unwrap();
        let slow = Slow { steady: Duration::ZERO, ..QUICK };
        let node = tokio::spawn({
            let path = path.clone();
            async move { run(&path, Duration::from_millis(10), slow, None).await }
        });
        tokio::time::sleep(Duration::from_millis(2500)).await;
        let n = tries.load(SeqCst);
        assert!(n > 3, "{n}: it went on asking, on the slow timer");
        node.abort();
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn waits_read_as_minutes() {
        assert_eq!(minutes(SLOW.first), "5 minutes");
        assert_eq!(minutes(SLOW.max), "30 minutes");
        assert_eq!(minutes(Duration::from_secs(20)), "1 minute");
    }

    #[test]
    fn names() {
        assert_eq!(name_from("Your-Mac-Studio.local"), "yourmacstudio");
        assert_eq!(name_from("DESKTOP-RTX5"), "desktoprtx5");
        assert_eq!(name_from("---"), "machine");
    }
}
