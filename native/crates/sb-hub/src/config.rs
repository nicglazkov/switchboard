//! Settings, from the environment. The defaults suit a hub next to the server; tests and a dev hub override them.
//!
//! SWITCHBOARD_HUB_LISTEN   address to listen on (default 0.0.0.0:8132, all interfaces like :8130)
//! SWITCHBOARD_HUB_TUNNEL_LISTEN  where connections a tunnel forwards come in (default 127.0.0.1:8136; an address on
//!                          this computer only). Listened on only when keys are on.
//! SWITCHBOARD_CONFIG_DIR   the folder for settings and keys (default ~/.config/switchboard), as the server reads it
//! SWITCHBOARD_STATE_DIR    the folder for lists and history (default ~/.local/share/switchboard)
//! SWITCHBOARD_SERVER_JSON  the server's settings file, read for "keys" (default ~/.config/switchboard/server.json)
//! SWITCHBOARD_ORIGINS      extra web origins allowed to open terminals (comma separated)
//! SWITCHBOARD_TERM_KEY    ticket key file (default ~/.config/switchboard/term.key)
//! SWITCHBOARD_MACHINES     machine list the server writes (default ~/.local/share/switchboard/machines.json)
//! SWITCHBOARD_NODES        paired machines (default nodes.json next to the machine list)
//! SWITCHBOARD_HUB_SSH      the ssh program (tests point it at a fake)
//! SWITCHBOARD_HUB_MAX      terminals open at once (default 16); a chat open on several devices is one terminal
//! SWITCHBOARD_HUB_MAX_VIEWERS  views of those terminals open at once, over all devices (default: 4 for each
//!                          terminal allowed, so 64)
//! SWITCHBOARD_HUB_IDLE     seconds with no input or output before a terminal closes (default 7200; 0 = never)

use std::net::{Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// server.json's "keys": how the other computers reach this one.
///
/// - `Off`: by address, as before keys. Nothing here changes.
/// - `Both`: by address and through the tunnel. The tunnel listener is on.
/// - `Only`: through the tunnel only. The main listener answers this computer alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Keys {
    Off,
    Both,
    Only,
}

/// What reading server.json for "keys" came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeysRead {
    pub keys: Keys,
    /// A line for the log, when the answer is not simply what the file says.
    pub note: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Config {
    /// Where the hub is asked to listen. `main_listen` is where it does: `Only` keeps it on this computer.
    pub listen: SocketAddr,
    /// Where connections forwarded from a tunnel come in. They arrive from loopback, so this listener has none
    /// of the `/internal/*` routes (server.rs `public_router`). Always an address on this computer.
    pub tunnel_listen: SocketAddr,
    pub keys: Keys,
    /// The server's settings file. Only read, never written: the server owns it.
    pub server_json: PathBuf,
    pub key_path: PathBuf,
    pub machines_path: PathBuf,
    pub nodes_path: PathBuf,
    pub ssh: String,
    pub ssh_key: PathBuf,
    pub origins: Vec<String>,
    /// Terminals open at once: chats with a terminal, however many devices show each.
    pub max_terms: usize,
    /// Views open at once, over every terminal: what one more device or window showing a chat adds to.
    pub max_viewers: usize,
    pub rate: u64,
    pub idle: Duration,
    /// A viewer the hub has heard nothing from for this long (not even a pong) is gone: a laptop that slept, Wi-Fi
    /// that dropped. The hub pings each viewer three times in this span.
    pub silence: Duration,
    /// How long one message to a linked machine may take to leave. A machine that stopped reading (it slept, or
    /// its network went away) fills the socket; past this its link is dropped, so it shows as offline and its
    /// terminals and jobs end instead of waiting until the system gives up on the connection, minutes later.
    pub link_patience: Duration,
}

pub const PORT: u16 = 8132;
/// Views allowed for each terminal allowed, unless SWITCHBOARD_HUB_MAX_VIEWERS says otherwise.
const VIEWERS_EACH: usize = 4;

/// The tunnel listener's port (plan section 3.3, "Home targets").
pub const TUNNEL_PORT: u16 = 8136;

/// The user's folder. On Windows that is USERPROFILE, and HOME only where there is none: an ssh session, Git Bash or
/// a test sets HOME there, and a program that followed it kept its files in one folder when started from a logon
/// task and in another when started from a shell (and once wrote a test's link over a real one). Everywhere else
/// it is HOME.
pub fn home() -> PathBuf {
    home_from(|k| std::env::var_os(k), cfg!(windows))
}

pub fn home_from(get: impl Fn(&str) -> Option<std::ffi::OsString>, windows: bool) -> PathBuf {
    let order = if windows { ["USERPROFILE", "HOME"] } else { ["HOME", "USERPROFILE"] };
    order.into_iter().filter_map(get).find(|v| !v.is_empty()).map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."))
}

fn folder(get: &dyn Fn(&str) -> Option<String>, var: &str) -> Option<PathBuf> {
    get(var).filter(|v| !v.trim().is_empty()).map(PathBuf::from)
}

/// Where settings and keys are kept: SWITCHBOARD_CONFIG_DIR, as the server reads it (a run beside the installed
/// one), else .config/switchboard in the user's folder.
pub fn config_dir(home: &std::path::Path, get: &dyn Fn(&str) -> Option<String>) -> PathBuf {
    folder(get, "SWITCHBOARD_CONFIG_DIR").unwrap_or_else(|| home.join(".config/switchboard"))
}

/// Where history and lists are kept: SWITCHBOARD_STATE_DIR, else .local/share/switchboard in the user's folder.
pub fn state_dir(home: &std::path::Path, get: &dyn Fn(&str) -> Option<String>) -> PathBuf {
    folder(get, "SWITCHBOARD_STATE_DIR").unwrap_or_else(|| home.join(".local/share/switchboard"))
}

/// The JSON object in `bytes`, if that is what they hold. An empty or zeroed file (a power cut) holds none.
fn object(bytes: &[u8]) -> Option<serde_json::Map<String, serde_json::Value>> {
    match serde_json::from_slice(bytes).ok()? {
        serde_json::Value::Object(m) => Some(m),
        _ => None,
    }
}

/// "keys" in a server.json that was read. A file without the entry is an install from before keys: `Off`.
/// Anything this hub does not know ("onyl", a number, a mode from a newer version) is not guessed at: `None`.
fn keys_in(d: &serde_json::Map<String, serde_json::Value>) -> Option<Keys> {
    match d.get("keys") {
        None => Some(Keys::Off),
        Some(serde_json::Value::String(s)) => match s.as_str() {
            "off" => Some(Keys::Off),
            "both" => Some(Keys::Both),
            "only" => Some(Keys::Only),
            _ => None,
        },
        Some(_) => None,
    }
}

/// Whether the server put a damaged server.json aside (server/durable.py names it `<name>.bad-<time>`).
fn put_aside(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else { return false };
    let prefix = format!("{name}.bad-");
    let Some(dir) = path.parent() else { return false };
    std::fs::read_dir(dir)
        .map(|d| d.flatten().any(|e| e.file_name().to_str().is_some_and(|n| n.starts_with(&prefix))))
        .unwrap_or(false)
}

/// The copy the server keeps beside its settings (`server.json.bak`), when it holds an object.
fn copy_of(path: &Path) -> Option<serde_json::Map<String, serde_json::Value>> {
    let mut name = path.file_name()?.to_os_string();
    name.push(".bak");
    object(&std::fs::read(path.with_file_name(name)).ok()?)
}

/// "keys" from the server's server.json, the way the server itself reads that file (server/durable.py), without
/// changing anything on disk:
///
/// - no file, and none put aside: an install that never chose. `Off`.
/// - a file that reads: what it says.
/// - a file that does not read (empty, cut short, not an object, will not open), or one the server put aside:
///   the copy beside it (`server.json.bak`), which is what the server goes back to as well.
/// - no good copy either, or a "keys" this hub does not know: `Only`, the choice that listens on this computer
///   alone. A hub that cannot tell what it was told never opens a port to the network.
pub fn read_keys(path: &Path) -> KeysRead {
    let safe = |why: &str| KeysRead {
        keys: Keys::Only,
        note: Some(format!("{why}, so the hub answers this computer only until that file is put right")),
    };
    let said = |d: &serde_json::Map<String, serde_json::Value>, note: Option<&str>| match keys_in(d) {
        Some(keys) => KeysRead { keys, note: note.map(str::to_string) },
        None => safe("server.json has a \"keys\" setting this hub does not know"),
    };
    let from_copy = || match copy_of(path) {
        Some(d) => said(&d, Some("server.json could not be read: its last good copy is in use")),
        None => safe("server.json could not be read and has no good copy"),
    };
    match std::fs::read(path) {
        Ok(bytes) => match object(&bytes) {
            Some(d) => said(&d, None),
            None => from_copy(),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound && !put_aside(path) => {
            KeysRead { keys: Keys::Off, note: None }
        }
        Err(_) => from_copy(),
    }
}

/// An address for the tunnel listener. Never one another computer can reach: what comes in there is trusted to
/// have been let through by the tunnel, which checks the paired list.
pub fn tunnel_addr(v: &str) -> Result<SocketAddr, String> {
    let a: SocketAddr = v.trim().parse().map_err(|_| format!("not an address: {v}"))?;
    if !a.ip().to_canonical().is_loopback() {
        return Err(format!("{v} is not on this computer (use 127.0.0.1 and a port)"));
    }
    Ok(a)
}

impl Config {
    pub fn defaults_in(home: &std::path::Path) -> Config {
        Config {
            listen: SocketAddr::from(([0, 0, 0, 0], PORT)),
            tunnel_listen: SocketAddr::from((Ipv4Addr::LOCALHOST, TUNNEL_PORT)),
            keys: Keys::Off,
            server_json: home.join(".config/switchboard/server.json"),
            key_path: home.join(".config/switchboard/term.key"),
            machines_path: home.join(".local/share/switchboard/machines.json"),
            nodes_path: home.join(".local/share/switchboard/nodes.json"),
            ssh: "ssh".into(),
            ssh_key: home.join(".ssh/switchboard_ed25519"),
            origins: vec![],
            max_terms: 16,
            max_viewers: 16 * VIEWERS_EACH,
            rate: 2 * 1024 * 1024,
            idle: Duration::from_secs(2 * 3600),
            silence: Duration::from_secs(45),
            link_patience: Duration::from_secs(20),
        }
    }

    pub fn from_env() -> Result<Config, String> {
        Config::from_vars(&home(), |k| std::env::var(k).ok())
    }

    /// `from_env`, with the environment as a lookup.
    pub fn from_vars(home: &std::path::Path, get: impl Fn(&str) -> Option<String>) -> Result<Config, String> {
        let mut c = Config::defaults_in(home);
        let var = |k: &str| get(k).filter(|v| !v.trim().is_empty());
        // (the two folders first: a variable that names one file still wins over the folder it would be in)
        let (config, state) = (config_dir(home, &var), state_dir(home, &var));
        c.server_json = config.join("server.json");
        c.key_path = config.join("term.key");
        c.machines_path = state.join("machines.json");
        c.nodes_path = state.join("nodes.json");
        if let Some(v) = var("SWITCHBOARD_HUB_LISTEN") {
            c.listen = v.parse().map_err(|_| format!("SWITCHBOARD_HUB_LISTEN: not an address: {v}"))?;
        }
        if let Some(v) = var("SWITCHBOARD_HUB_TUNNEL_LISTEN") {
            c.tunnel_listen = tunnel_addr(&v).map_err(|e| format!("SWITCHBOARD_HUB_TUNNEL_LISTEN: {e}"))?;
        }
        if let Some(v) = var("SWITCHBOARD_SERVER_JSON") {
            c.server_json = v.into();
        }
        if let Some(v) = var("SWITCHBOARD_ORIGINS") {
            c.origins = crate::origin::parse_list(&v);
        }
        if let Some(v) = var("SWITCHBOARD_TERM_KEY") {
            c.key_path = v.into();
        }
        if let Some(v) = var("SWITCHBOARD_MACHINES") {
            c.machines_path = v.into();
            c.nodes_path = c.machines_path.with_file_name("nodes.json");
        }
        if let Some(v) = var("SWITCHBOARD_NODES") {
            c.nodes_path = v.into();
        }
        if let Some(v) = var("SWITCHBOARD_HUB_SSH") {
            c.ssh = v;
        }
        if let Some(v) = var("SWITCHBOARD_HUB_MAX") {
            c.max_terms = v.parse().map_err(|_| format!("SWITCHBOARD_HUB_MAX: not a number: {v}"))?;
            c.max_viewers = c.max_terms.saturating_mul(VIEWERS_EACH);
        }
        if let Some(v) = var("SWITCHBOARD_HUB_MAX_VIEWERS") {
            c.max_viewers = v.parse().map_err(|_| format!("SWITCHBOARD_HUB_MAX_VIEWERS: not a number: {v}"))?;
        }
        if let Some(v) = var("SWITCHBOARD_HUB_IDLE") {
            c.idle = Duration::from_secs(v.parse().map_err(|_| format!("SWITCHBOARD_HUB_IDLE: not a number: {v}"))?);
        }
        Ok(c)
    }

    /// Reads "keys" from server.json into `keys`. The line to log, when there is something to say.
    pub fn load_keys(&mut self) -> Option<String> {
        let r = read_keys(&self.server_json);
        self.keys = r.keys;
        r.note
    }

    /// Where the main listener really listens. In `Only` that is this computer alone, whatever `listen` (the
    /// environment, `--listen`, a service file written before keys) asks for: the port stays, the address does not.
    pub fn main_listen(&self) -> SocketAddr {
        if self.keys == Keys::Only && !self.listen.ip().to_canonical().is_loopback() {
            SocketAddr::from((Ipv4Addr::LOCALHOST, self.listen.port()))
        } else {
            self.listen
        }
    }

    /// Where the tunnel listener listens, when keys are on. `None` with keys off: nothing new is opened.
    pub fn tunnel(&self) -> Option<SocketAddr> {
        (self.keys != Keys::Off).then_some(self.tunnel_listen)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let d = std::env::temp_dir().join(format!(
            "sbhub-cfg-{tag}-{}-{}",
            std::process::id(),
            N.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn an_install_from_before_keys_listens_as_it_always_did() {
        // no server.json at all, and one with no "keys" entry: nothing changes
        let d = tmp("old");
        let p = d.join("server.json");
        assert_eq!(read_keys(&p), KeysRead { keys: Keys::Off, note: None });
        std::fs::write(&p, r#"{"bind": ["0.0.0.0"], "hosts": ["studio"], "origins": [], "peers": []}"#).unwrap();
        assert_eq!(read_keys(&p), KeysRead { keys: Keys::Off, note: None });
        let mut c = Config::defaults_in(&d);
        c.server_json = p;
        assert_eq!(c.load_keys(), None);
        assert_eq!(c.keys, Keys::Off);
        assert_eq!(c.main_listen(), "0.0.0.0:8132".parse().unwrap());
        assert_eq!(c.tunnel(), None, "no new port with keys off");
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn the_mode_decides_where_the_hub_listens() {
        let d = tmp("mode");
        let p = d.join("server.json");
        let mut c = Config::defaults_in(&d);
        c.server_json = p.clone();
        let tunnel: SocketAddr = "127.0.0.1:8136".parse().unwrap();
        for (text, keys, main, tun) in [
            ("off", Keys::Off, "0.0.0.0:8132", None),
            ("both", Keys::Both, "0.0.0.0:8132", Some(tunnel)),
            ("only", Keys::Only, "127.0.0.1:8132", Some(tunnel)),
        ] {
            std::fs::write(&p, format!(r#"{{"bind": ["0.0.0.0"], "keys": "{text}"}}"#)).unwrap();
            assert_eq!(c.load_keys(), None, "{text}");
            assert_eq!(c.keys, keys, "{text}");
            assert_eq!(c.main_listen(), main.parse().unwrap(), "{text}");
            assert_eq!(c.tunnel(), tun, "{text}");
        }
        // `only` wins over an address asked for in the environment or a service file from before keys; the port
        // stays
        c.listen = "10.0.0.5:9132".parse().unwrap();
        assert_eq!(c.main_listen(), "127.0.0.1:9132".parse().unwrap());
        c.listen = "[::]:8132".parse().unwrap();
        assert_eq!(c.main_listen(), "127.0.0.1:8132".parse().unwrap());
        // a loopback address that was asked for is kept as it is
        c.listen = "[::1]:8132".parse().unwrap();
        assert_eq!(c.main_listen(), "[::1]:8132".parse().unwrap());
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn a_server_json_that_cannot_be_read_means_this_computer_only() {
        let d = tmp("bad");
        let p = d.join("server.json");
        let only =
            |r: &KeysRead| r.keys == Keys::Only && r.note.as_deref().is_some_and(|n| n.contains("this computer only"));
        for (what, bytes) in [
            ("zeroed by a power cut", &b"\0\0\0\0\0\0\0\0"[..]),
            ("empty", &b""[..]),
            ("cut short", &br#"{"bind": ["0.0.0.0"], "ke"#[..]),
            ("not an object", &br#"["both"]"#[..]),
            ("a mode this hub does not know", &br#"{"keys": "onyl"}"#[..]),
            ("a mode of the wrong type", &br#"{"keys": true}"#[..]),
            ("a mode that is null", &br#"{"keys": null}"#[..]),
        ] {
            std::fs::write(&p, bytes).unwrap();
            let r = read_keys(&p);
            assert!(only(&r), "{what}: {r:?}");
            assert_eq!(std::fs::read(&p).unwrap(), bytes, "{what}: the server's file is only read");
        }
        // a folder where the file should be: there, and it will not open
        std::fs::remove_file(&p).unwrap();
        std::fs::create_dir(&p).unwrap();
        assert!(only(&read_keys(&p)));
        std::fs::remove_dir(&p).unwrap();
        // the server put the damaged file aside and has no good copy: still not "an install that never chose"
        std::fs::write(d.join("server.json.bad-20261122-101500"), b"").unwrap();
        assert!(only(&read_keys(&p)));
        let mut c = Config::defaults_in(&d);
        c.server_json = p;
        assert!(c.load_keys().is_some());
        assert_eq!(c.main_listen(), "127.0.0.1:8132".parse().unwrap());
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn a_damaged_server_json_goes_back_to_its_last_good_copy_as_the_server_does() {
        let d = tmp("bak");
        let p = d.join("server.json");
        std::fs::write(d.join("server.json.bak"), br#"{"bind": ["0.0.0.0"], "keys": "both"}"#).unwrap();
        std::fs::write(&p, b"\0\0\0").unwrap();
        let r = read_keys(&p);
        assert_eq!(r.keys, Keys::Both);
        assert!(r.note.unwrap().contains("last good copy"));
        // put aside by the server, which is answering from the copy
        std::fs::rename(&p, d.join("server.json.bad-20261122-101500")).unwrap();
        assert_eq!(read_keys(&p).keys, Keys::Both);
        // a copy from before keys: the install never chose
        std::fs::write(d.join("server.json.bak"), br#"{"bind": ["0.0.0.0"]}"#).unwrap();
        assert_eq!(read_keys(&p).keys, Keys::Off);
        // a copy that says something unknown, or is damaged too: this computer only
        std::fs::write(d.join("server.json.bak"), br#"{"keys": "everything"}"#).unwrap();
        assert_eq!(read_keys(&p).keys, Keys::Only);
        std::fs::write(d.join("server.json.bak"), b"").unwrap();
        assert_eq!(read_keys(&p).keys, Keys::Only);
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn the_tunnel_listener_is_never_on_an_address_another_computer_can_reach() {
        let home = std::path::Path::new("/home/you");
        let with = |v: &'static str| {
            Config::from_vars(home, move |k| (k == "SWITCHBOARD_HUB_TUNNEL_LISTEN").then(|| v.to_string()))
        };
        assert_eq!(with("127.0.0.1:9136").unwrap().tunnel_listen, "127.0.0.1:9136".parse().unwrap());
        assert_eq!(with("[::1]:9136").unwrap().tunnel_listen, "[::1]:9136".parse().unwrap());
        for bad in ["0.0.0.0:8136", "10.0.0.5:8136", "[::]:8136", "8136", "localhost"] {
            let e = with(bad).unwrap_err();
            assert!(e.starts_with("SWITCHBOARD_HUB_TUNNEL_LISTEN: "), "{bad}: {e}");
        }
        let c = Config::from_vars(home, |_| None).unwrap();
        assert_eq!(c.tunnel_listen, "127.0.0.1:8136".parse().unwrap());
        assert_eq!(c.server_json, home.join(".config/switchboard/server.json"));
        assert_eq!(c.keys, Keys::Off, "from_vars reads no file: main does, with load_keys");
        let c =
            Config::from_vars(home, |k| (k == "SWITCHBOARD_SERVER_JSON").then(|| "/etc/sb/server.json".to_string()));
        assert_eq!(c.unwrap().server_json, PathBuf::from("/etc/sb/server.json"));
    }

    #[test]
    fn on_windows_the_users_folder_is_userprofile_and_home_only_where_there_is_none() {
        let get = |vars: &'static [(&'static str, &'static str)]| {
            move |k: &str| vars.iter().find(|(n, _)| *n == k).map(|(_, v)| std::ffi::OsString::from(v))
        };
        let both: &[(&str, &str)] = &[("HOME", "C:/scratch"), ("USERPROFILE", "C:/Users/you")];
        // an ssh session or a test that sets HOME moves nothing on Windows
        assert_eq!(home_from(get(both), true), PathBuf::from("C:/Users/you"));
        assert_eq!(home_from(get(&[("HOME", "C:/msys/home/you")]), true), PathBuf::from("C:/msys/home/you"));
        assert_eq!(home_from(get(&[("HOME", "C:/h"), ("USERPROFILE", "")]), true), PathBuf::from("C:/h"));
        // everywhere else HOME is the folder, as before
        assert_eq!(home_from(get(both), false), PathBuf::from("C:/scratch"));
        assert_eq!(home_from(get(&[("USERPROFILE", "C:/Users/you")]), false), PathBuf::from("C:/Users/you"));
        assert_eq!(
            home_from(get(&[("HOME", ""), ("USERPROFILE", "C:/Users/you")]), false),
            PathBuf::from("C:/Users/you")
        );
        for windows in [true, false] {
            assert_eq!(home_from(get(&[]), windows), PathBuf::from("."));
        }
    }

    #[test]
    fn the_two_folders_move_every_file_and_a_variable_for_one_file_still_wins() {
        let home = std::path::Path::new("/home/you");
        let with = |vars: &'static [(&'static str, &'static str)]| {
            Config::from_vars(home, move |k| vars.iter().find(|(n, _)| *n == k).map(|(_, v)| v.to_string())).unwrap()
        };
        // nothing set: where every install has them
        let c = with(&[]);
        assert_eq!(c.server_json, home.join(".config/switchboard/server.json"));
        assert_eq!(c.key_path, home.join(".config/switchboard/term.key"));
        assert_eq!(c.machines_path, home.join(".local/share/switchboard/machines.json"));
        assert_eq!(c.nodes_path, home.join(".local/share/switchboard/nodes.json"));
        let c = with(&[("SWITCHBOARD_CONFIG_DIR", "/t/cfg"), ("SWITCHBOARD_STATE_DIR", "/t/state")]);
        assert_eq!(c.server_json, PathBuf::from("/t/cfg/server.json"));
        assert_eq!(c.key_path, PathBuf::from("/t/cfg/term.key"));
        assert_eq!(c.machines_path, PathBuf::from("/t/state/machines.json"));
        assert_eq!(c.nodes_path, PathBuf::from("/t/state/nodes.json"));
        assert_eq!(c.ssh_key, home.join(".ssh/switchboard_ed25519"), "the ssh key is the user's, not Switchboard's");
        let c = with(&[
            ("SWITCHBOARD_CONFIG_DIR", "/t/cfg"),
            ("SWITCHBOARD_STATE_DIR", "/t/state"),
            ("SWITCHBOARD_TERM_KEY", "/k/term.key"),
            ("SWITCHBOARD_MACHINES", "/m/machines.json"),
        ]);
        assert_eq!(c.key_path, PathBuf::from("/k/term.key"));
        assert_eq!(c.server_json, PathBuf::from("/t/cfg/server.json"));
        assert_eq!(c.machines_path, PathBuf::from("/m/machines.json"));
        assert_eq!(
            c.nodes_path,
            PathBuf::from("/m/nodes.json"),
            "the node list stays beside a machine list that moved"
        );
        // an empty value names no folder
        assert_eq!(with(&[("SWITCHBOARD_CONFIG_DIR", " ")]).key_path, home.join(".config/switchboard/term.key"));
    }

    #[test]
    fn the_terminal_limit_and_the_viewer_limit_come_from_the_environment() {
        let home = std::path::Path::new("/home/you");
        let with = |vars: &[(&str, &str)]| {
            let vars: Vec<(String, String)> = vars.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
            Config::from_vars(home, move |k| vars.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone()))
        };
        let c = with(&[]).unwrap();
        assert_eq!((c.max_terms, c.max_viewers), (16, 64));
        // the old setting still sets the terminals, and the viewers follow it
        let c = with(&[("SWITCHBOARD_HUB_MAX", "40")]).unwrap();
        assert_eq!((c.max_terms, c.max_viewers), (40, 160));
        let c = with(&[("SWITCHBOARD_HUB_MAX", "40"), ("SWITCHBOARD_HUB_MAX_VIEWERS", "50")]).unwrap();
        assert_eq!((c.max_terms, c.max_viewers), (40, 50));
        let c = with(&[("SWITCHBOARD_HUB_MAX_VIEWERS", "8"), ("SWITCHBOARD_HUB_MAX", " ")]).unwrap();
        assert_eq!((c.max_terms, c.max_viewers), (16, 8));
        assert!(with(&[("SWITCHBOARD_HUB_MAX", "many")]).unwrap_err().contains("SWITCHBOARD_HUB_MAX: not a number"));
        assert!(with(&[("SWITCHBOARD_HUB_MAX_VIEWERS", "-1")]).unwrap_err().contains("SWITCHBOARD_HUB_MAX_VIEWERS"));
    }
}
