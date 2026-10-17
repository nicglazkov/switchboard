//! The on-disk snapshot that paints the first frame before the network answers, plus the folder trees so the
//! folder picker is instant. Writes are atomic (temp file, then rename), so a crash never leaves half a file.

use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::api::{ApiFolder, ApiFolders, ApiSessions};

/// A snapshot older than this is not shown (the web app uses the same limit).
pub const MAX_AGE: f64 = 24.0 * 3600.0;

use crate::brand::DESKTOP_ID;

/// The platform cache dir for this app: ~/Library/Caches/<DESKTOP_ID> on macOS,
/// %LOCALAPPDATA%\Switchboard\cache on Windows, ~/.cache/switchboard on Linux.
pub fn default_dir() -> Option<PathBuf> {
    let base = dirs::cache_dir()?;
    Some(if cfg!(target_os = "macos") {
        base.join(DESKTOP_ID)
    } else if cfg!(windows) {
        base.join("Switchboard").join("cache")
    } else {
        base.join("switchboard")
    })
}

/// Where small settings live (theme, folded lanes): next to the cache on macOS and Linux config dirs elsewhere.
pub fn settings_dir() -> Option<PathBuf> {
    let base = dirs::config_dir()?;
    Some(if cfg!(target_os = "macos") { base.join(DESKTOP_ID) } else { base.join("Switchboard") })
}

/// Writes `bytes` to `path` atomically.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension(format!("tmp-{}", std::process::id()));
    {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all().ok();
    }
    match fs::rename(&tmp, path) {
        Ok(()) => Ok(()),
        Err(e) => {
            let _ = fs::remove_file(&tmp);
            Err(e)
        }
    }
}

#[derive(Deserialize)]
struct SessionsFile {
    fetched_at: f64,
    #[serde(default)]
    base: Option<String>,
    data: ApiSessions,
}

/// Saves the raw /api/sessions reply as it came (no re-encoding), with when and where it came from.
pub fn save_sessions(dir: &Path, raw: &[u8], fetched_at: f64, base: &str) -> std::io::Result<()> {
    let head = format!(
        "{{\"v\":1,\"fetched_at\":{fetched_at},\"base\":{},\"data\":",
        serde_json::to_string(base).unwrap_or_else(|_| "null".into())
    );
    let mut out = Vec::with_capacity(raw.len() + head.len() + 2);
    out.extend_from_slice(head.as_bytes());
    out.extend_from_slice(raw);
    out.push(b'}');
    write_atomic(&dir.join("sessions.json"), &out)
}

/// The last saved reply, with its fetch time and server, if it is younger than `MAX_AGE` at `now`.
pub fn load_sessions(dir: &Path, now: f64) -> Option<(ApiSessions, f64, Option<String>)> {
    let bytes = fs::read(dir.join("sessions.json")).ok()?;
    let f: SessionsFile = serde_json::from_slice(&bytes).ok()?;
    if now - f.fetched_at > MAX_AGE {
        return None;
    }
    Some((f.data, f.fetched_at, f.base))
}

/// The server that answered the last saved reply, however old it is: where to look first on the next start.
pub fn last_server(dir: &Path) -> Option<String> {
    let bytes = fs::read(dir.join("sessions.json")).ok()?;
    let f: SessionsFile = serde_json::from_slice(&bytes).ok()?;
    f.base.filter(|b| !b.is_empty())
}

/// Folder data for one machine: its Claude folder, the recents and every tree level seen so far.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MachineFolders {
    pub root: Option<String>,
    pub recent: Vec<ApiFolder>,
    pub tree: HashMap<String, Vec<ApiFolder>>,
    pub fetched_at: f64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FolderCache {
    pub machines: HashMap<String, MachineFolders>,
}

impl FolderCache {
    pub fn load(dir: &Path) -> FolderCache {
        fs::read(dir.join("folders.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    pub fn save(&self, dir: &Path) -> std::io::Result<()> {
        write_atomic(&dir.join("folders.json"), &serde_json::to_vec(self).unwrap_or_default())
    }

    /// Takes one reply: a tree level (has `parent`) or the flat list with recents.
    pub fn take(&mut self, machine: &str, parent: Option<&str>, reply: &ApiFolders, now: f64) {
        let m = self.machines.entry(machine.to_string()).or_default();
        if let Some(r) = &reply.root {
            m.root = Some(r.clone());
        }
        m.fetched_at = now;
        match (parent, &reply.parent) {
            (Some(p), Some(_)) => {
                m.tree.insert(p.to_string(), reply.folders.clone());
            }
            (Some(p), None) => {
                // An older server ignores ?parent and sends the flat list: keep it as a flat top level.
                if p.is_empty() {
                    let top = reply
                        .folders
                        .iter()
                        .filter(|f| !f.folder.is_empty())
                        .map(|f| ApiFolder {
                            folder: f.folder.clone(),
                            name: Some(f.folder.clone()),
                            kids: Some(false),
                            recent: f.recent,
                        })
                        .collect();
                    m.tree.insert(String::new(), top);
                }
            }
            (None, _) => {
                let mut rec: Vec<ApiFolder> = reply
                    .folders
                    .iter()
                    .filter(|f| !f.folder.is_empty() && f.recent.unwrap_or(0.0) > 0.0)
                    .cloned()
                    .collect();
                rec.sort_by(|a, b| b.recent.unwrap_or(0.0).total_cmp(&a.recent.unwrap_or(0.0)));
                rec.truncate(3);
                m.recent = rec;
            }
        }
    }

    pub fn machine(&self, machine: &str) -> Option<&MachineFolders> {
        self.machines.get(machine)
    }

    pub fn level(&self, machine: &str, parent: &str) -> Option<&Vec<ApiFolder>> {
        self.machines.get(machine)?.tree.get(parent)
    }
}

/// Loads a JSON settings file, or the default.
pub fn load_json<T: for<'de> Deserialize<'de> + Default>(path: &Path) -> T {
    fs::read(path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

pub fn save_json<T: Serialize>(path: &Path, value: &T) -> std::io::Result<()> {
    write_atomic(path, &serde_json::to_vec_pretty(value).unwrap_or_default())
}
