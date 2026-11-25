//! Settings, from the environment. The defaults suit a hub next to the server; tests and a dev hub override them.
//!
//! SWITCHBOARD_HUB_LISTEN   address to listen on (default 0.0.0.0:8132, all interfaces like :8130)
//! SWITCHBOARD_ORIGINS      extra web origins allowed to open terminals (comma separated)
//! SWITCHBOARD_TERM_KEY     ticket key file (default ~/.config/switchboard/term.key)
//! SWITCHBOARD_MACHINES     machine list the server writes (default ~/.local/share/switchboard/machines.json)
//! SWITCHBOARD_NODES        paired machines (default nodes.json next to the machine list)
//! SWITCHBOARD_HUB_SSH      the ssh program (tests point it at a fake)
//! SWITCHBOARD_HUB_MAX      terminals open at once (default 16); a chat open on several devices is one terminal
//! SWITCHBOARD_HUB_MAX_VIEWERS  views of those terminals open at once, over all devices (default: 4 for each
//!                          terminal allowed, so 64)
//! SWITCHBOARD_HUB_IDLE     seconds with no input or output before a terminal closes (default 7200; 0 = never)

use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct Config {
    pub listen: SocketAddr,
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

fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."))
}

impl Config {
    pub fn defaults_in(home: &std::path::Path) -> Config {
        Config {
            listen: SocketAddr::from(([0, 0, 0, 0], PORT)),
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
        if let Some(v) = var("SWITCHBOARD_HUB_LISTEN") {
            c.listen = v.parse().map_err(|_| format!("SWITCHBOARD_HUB_LISTEN: not an address: {v}"))?;
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
}

#[cfg(test)]
mod tests {
    use super::*;

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
