//! Settings, from the environment. The defaults suit a hub next to the server; tests and a dev hub override them.
//!
//! SWITCHBOARD_HUB_LISTEN   address to listen on (default 0.0.0.0:8132, all interfaces like :8130)
//! SWITCHBOARD_ORIGINS      extra web origins allowed to open terminals (comma separated)
//! SWITCHBOARD_TERM_KEY     ticket key file (default ~/.config/switchboard/term.key)
//! SWITCHBOARD_MACHINES     machine list the server writes (default ~/.local/share/switchboard/machines.json)
//! SWITCHBOARD_NODES        paired machines (default nodes.json next to the machine list)
//! SWITCHBOARD_HUB_SSH      the ssh program (tests point it at a fake)
//! SWITCHBOARD_HUB_MAX      terminals open at once (default 16)
//! SWITCHBOARD_HUB_IDLE     seconds with no input or output before a terminal closes (default 7200)

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
    pub max_terms: usize,
    pub rate: u64,
    pub idle: Duration,
    /// A viewer the hub has heard nothing from for this long (not even a pong) is gone: a laptop that slept, Wi-Fi
    /// that dropped. The hub pings each viewer three times in this span.
    pub silence: Duration,
}

pub const PORT: u16 = 8132;

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
            rate: 2 * 1024 * 1024,
            idle: Duration::from_secs(2 * 3600),
            silence: Duration::from_secs(45),
        }
    }

    pub fn from_env() -> Result<Config, String> {
        let mut c = Config::defaults_in(&home());
        let var = |k: &str| std::env::var(k).ok().filter(|v| !v.trim().is_empty());
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
        }
        if let Some(v) = var("SWITCHBOARD_HUB_IDLE") {
            c.idle = Duration::from_secs(v.parse().map_err(|_| format!("SWITCHBOARD_HUB_IDLE: not a number: {v}"))?);
        }
        Ok(c)
    }
}
