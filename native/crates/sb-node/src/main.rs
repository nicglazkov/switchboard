//! switchboard-node (spec section 6): links this machine to the Switchboard hub with one outgoing WebSocket, so a
//! machine behind NAT or without SSH still shows up, runs jobs and opens terminals.
//!
//!   switchboard-node pair <hub> <code> [--name NAME] [--label LABEL]   trade an Add a machine code for a token
//!   switchboard-node run                                               keep the link up (the user service runs this)
//!   switchboard-node status                                            what this machine is linked as
//!
//! <hub> is the hub's address: 192.0.2.10, 192.0.2.10:8132, a tailnet name, or an http:// URL.
//! Settings live in ~/.config/switchboard/node.json (mode 600): {hub, machine, token, sb?, python?}.
//! SWITCHBOARD_NODE_CONFIG moves that file; SWITCHBOARD_SB and SWITCHBOARD_PYTHON say where sb.py and Python are
//! (default ~/.claude/switchboard/sb.py, run with python3, or python on Windows).

mod link;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
const HUB_PORT: u16 = 8132;
/// How often `run` looks at node.json again while it has no token the hub takes.
const RECHECK: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeConfig {
    pub hub: String,
    pub machine: String,
    pub token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sb: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub python: Option<String>,
}

pub fn home() -> PathBuf {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn env(k: &str) -> Option<String> {
    std::env::var(k).ok().filter(|v| !v.trim().is_empty())
}

fn config_path() -> PathBuf {
    env("SWITCHBOARD_NODE_CONFIG").map(PathBuf::from).unwrap_or_else(|| home().join(".config/switchboard/node.json"))
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
            .unwrap_or_else(|| if cfg!(windows) { "python" } else { "python3" }.into())
    }
}

fn load() -> Result<NodeConfig, String> {
    load_from(&config_path())
}

fn load_from(p: &Path) -> Result<NodeConfig, String> {
    let text = std::fs::read_to_string(p).map_err(|_| "This machine is not paired yet.".to_string())?;
    serde_json::from_str(&text).map_err(|e| format!("{} is not readable ({e}). Pair again.", p.display()))
}

fn save(c: &NodeConfig) -> std::io::Result<PathBuf> {
    let p = config_path();
    let dir = p.parent().map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
    std::fs::create_dir_all(&dir)?;
    let tmp = dir.join(format!(".node.json.{}.tmp", std::process::id()));
    {
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        use std::io::Write;
        let mut f = opts.open(&tmp)?;
        f.write_all(serde_json::to_string_pretty(c).map_err(std::io::Error::other)?.as_bytes())?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, &p)?;
    Ok(p)
}

/// "192.0.2.10", "http://host:8132/", "ws://[fd7a::1]" -> "host:port".
pub fn hub_addr(s: &str) -> Result<String, String> {
    let mut h = s.trim();
    for scheme in ["http://", "https://", "ws://", "wss://"] {
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

fn hostname() -> String {
    let out = std::process::Command::new("hostname")
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string());
    out.filter(|h| !h.is_empty()).or_else(|| env("COMPUTERNAME")).unwrap_or_default()
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

fn os_name() -> &'static str {
    match std::env::consts::OS {
        "macos" => "mac",
        o => o,
    }
}

/// One small HTTP/1.1 POST of JSON to the hub; the answer's status and JSON body.
async fn post_json(addr: &str, path: &str, body: &Value) -> Result<(u16, Value), String> {
    let text = body.to_string();
    let io = async {
        let mut s = tokio::net::TcpStream::connect(addr).await?;
        let req = format!(
            "POST {path} HTTP/1.1\r\nHost: {addr}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}",
            text.len()
        );
        s.write_all(req.as_bytes()).await?;
        let mut out = Vec::new();
        s.read_to_end(&mut out).await?;
        Ok::<_, std::io::Error>(out)
    };
    let out = match tokio::time::timeout(Duration::from_secs(20), io).await {
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
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--name" => name = it.next().cloned(),
            "--label" => label = it.next().cloned(),
            _ => pos.push(a.clone()),
        }
    }
    let [hub, code] = pos.as_slice() else { return usage() };
    let addr = match hub_addr(hub) {
        Ok(a) => a,
        Err(e) => return fail(&e, 2),
    };
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
    let old = load().ok();
    let cfg = NodeConfig {
        hub: addr.clone(),
        machine: machine.clone(),
        token: token.into(),
        sb: old.as_ref().and_then(|o| o.sb.clone()),
        python: old.and_then(|o| o.python),
    };
    match save(&cfg) {
        Ok(p) => {
            println!(
                "Linked as {machine} ({}) to the hub at {addr}. Settings: {}",
                v["label"].as_str().unwrap_or(&label),
                p.display()
            );
            println!("Start it with: switchboard-node run");
            0
        }
        Err(e) => fail(&format!("Paired, but could not save the settings ({e}). Pair again with a new code."), 1),
    }
}

fn status() -> i32 {
    match load() {
        Ok(c) => {
            println!("Linked as {} to the hub at {}.", c.machine, c.hub);
            println!("sb.py: {}  python: {}", c.sb(), c.python());
            0
        }
        Err(e) => fail(&format!("{e} Get a code from Add a machine, then run: switchboard-node pair <hub> <code>"), 1),
    }
}

fn fail(msg: &str, code: i32) -> i32 {
    eprintln!("switchboard-node: {msg}");
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

/// The user service text for each OS (LaunchAgent plist, systemd unit, the hidden launcher a logon task runs).
fn service_text(exe: &str, home: &str) -> String {
    if cfg!(target_os = "macos") {
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
        format!("CreateObject(\"WScript.Shell\").Run \"\"\"{exe}\"\" run\", 0, False\r\n")
    } else {
        format!(
            "[Unit]\nDescription=Switchboard node (links this machine to the hub)\nAfter=network-online.target\n\n[Service]\n\
             ExecStart=\"{exe}\" run\nRestart=always\nRestartSec=5\n\n[Install]\nWantedBy=default.target\n"
        )
    }
}

const LABEL: &str = "com.switchboard.node";

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
        if cfg!(windows) {
            let _ = sh("schtasks", &["/End", "/TN", "SwitchboardNode"]);
        }
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
            sh("launchctl", &["bootstrap", &format!("gui/{uid}"), &plist.display().to_string()])?;
            Ok(format!("Runs at login (LaunchAgent {LABEL}); log in ~/Library/Logs/switchboard-node.log."))
        } else if cfg!(windows) {
            let vbs = dest.with_file_name("switchboard-node.vbs");
            std::fs::write(&vbs, text).map_err(|e| e.to_string())?;
            // "/SC ONLOGON" means any user's logon, which only an administrator may set: from a plain PowerShell
            // window it is "Access is denied". A task whose logon trigger names this user needs no rights.
            let user = sh("whoami", &[])?;
            let xml = dest.with_file_name("switchboard-node-task.xml");
            std::fs::write(&xml, utf16(&task_xml(&user, &vbs.display().to_string())))
                .map_err(|e| format!("could not write {} ({e})", xml.display()))?;
            let made = sh("schtasks", &["/Create", "/F", "/TN", "SwitchboardNode", "/XML", &xml.display().to_string()]);
            let _ = std::fs::remove_file(&xml);
            made?;
            sh("schtasks", &["/Run", "/TN", "SwitchboardNode"])?;
            Ok("Runs at logon (task SwitchboardNode), with no window.".into())
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
            println!("switchboard-node is installed at {}. {how}", dest.display());
            0
        }
        Err(e) => fail(&e, 1),
    }
}

/// `run`: link with the token in node.json. With no token, one the hub turned down, or one another node took the
/// link with, it waits for `pair` to write a new one (or a restart) instead of exiting, since the user service
/// (KeepAlive, Restart=always) would only start it again at once, over and over. A node that was replaced does not
/// link again by itself: two with one token would take the link from each other for good.
async fn run(path: &Path, every: Duration) -> i32 {
    let mut tried = None::<String>;
    let mut said = false;
    loop {
        match load_from(path) {
            Ok(c) if tried.as_deref() != Some(c.token.as_str()) => {
                said = false;
                match link::run(c.clone()).await {
                    link::UNPAIRED | link::REPLACED => tried = Some(c.token),
                    code => return code,
                }
            }
            Ok(_) => {} // still the token the hub turned down, or another node uses
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
    eprintln!("usage: switchboard-node pair <hub> <code> [--name NAME] [--label LABEL]");
    eprintln!("       switchboard-node install");
    eprintln!("       switchboard-node run");
    eprintln!("       switchboard-node status");
    2
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = match args.first().map(String::as_str) {
        Some("pair") => pair(&args[1..]).await,
        Some("run") => run(&config_path(), RECHECK).await,
        Some("status") => status(),
        Some("install") => install(),
        Some("--version" | "version") => {
            println!("switchboard-node {VERSION}");
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
    fn service_runs_the_installed_node() {
        let t = service_text("/home/you/.local/bin/switchboard-node", "/home/you");
        assert!(t.contains("switchboard-node"));
        assert!(t.contains("run"));
        assert!(install_path().ends_with(if cfg!(windows) { "switchboard-node.exe" } else { "switchboard-node" }));
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
        let gone = tokio::time::timeout(Duration::from_millis(300), run(&path, Duration::from_millis(10)));
        assert!(gone.await.is_err(), "still running, so the service does not start it again and again");
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
            let c =
                NodeConfig { hub: addr.clone(), machine: "lab".into(), token: token.into(), sb: None, python: None };
            std::fs::write(&path, serde_json::to_string(&c).unwrap()).unwrap();
        };
        write("old");
        let node = tokio::spawn({
            let path = path.clone();
            async move { run(&path, Duration::from_millis(10)).await }
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

    #[tokio::test]
    async fn a_replaced_node_does_not_take_the_link_back() {
        use futures_util::StreamExt;
        use tokio_tungstenite::tungstenite::protocol::{CloseFrame, frame::coding::CloseCode};
        // a hub where another node with this token already took over: it closes the link as replaced
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
        let d = scratch("replaced");
        let path = d.join("node.json");
        let c = NodeConfig { hub: addr, machine: "lab".into(), token: "same".into(), sb: None, python: None };
        std::fs::write(&path, serde_json::to_string(&c).unwrap()).unwrap();
        let node = tokio::spawn({
            let path = path.clone();
            async move { run(&path, Duration::from_millis(10)).await }
        });
        tokio::time::sleep(Duration::from_millis(800)).await;
        assert_eq!(tries.load(SeqCst), 1, "it links once and then leaves the link to the other node");
        assert!(!node.is_finished());
        node.abort();
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn names() {
        assert_eq!(name_from("Your-Mac-Studio.local"), "yourmacstudio");
        assert_eq!(name_from("DESKTOP-RTX5"), "desktoprtx5");
        assert_eq!(name_from("---"), "machine");
    }
}
