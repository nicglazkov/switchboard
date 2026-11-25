//! The on-disk snapshot that paints the first frame before the network answers, plus the folder trees so the
//! folder picker is instant. Writes are atomic and forced to disk (durable.rs), so a crash never leaves half a
//! file and a power cut never leaves zeros.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::api::{ApiFolder, ApiFolders, ApiSessions};
use crate::durable;

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

/// Writes `bytes` to `path` whole or not at all: a temp file beside it, forced to disk, then renamed over it
/// (durable.rs). A write that did not reach the disk is an error, not a file of zeros after the next power cut.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    // (forced: what goes through here is gathered again, or was never read through `load_json`)
    durable::Store::new(path).put(bytes, true)
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

/// What the last saved reply says about where the server is, however old it is: the server that answered, when, and
/// the addresses it listed (none from an older server). Seeds the saved list after an upgrade (servers.rs).
pub fn last_seen(dir: &Path) -> Option<(Option<String>, f64, Vec<crate::api::ApiAddress>)> {
    let bytes = fs::read(dir.join("sessions.json")).ok()?;
    let f: SessionsFile = serde_json::from_slice(&bytes).ok()?;
    let listed = f.data.summary.addresses.map(|a| a.list).unwrap_or_default();
    Some((f.base.filter(|b| !b.is_empty()), f.fetched_at, listed))
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

/// Loads a JSON settings file, or the default. A file that is there and cannot be read is not taken for an empty
/// one: it is put aside as <name>.bad-<time>, and `save_json` does not write the default over it (durable.rs).
pub fn load_json<T: for<'de> Deserialize<'de> + Default>(path: &Path) -> T {
    durable::Store::new(path).load_json().0.unwrap_or_default()
}

pub fn save_json<T: Serialize>(path: &Path, value: &T) -> std::io::Result<()> {
    durable::Store::new(path).put_json(value, false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_settings_file_of_zeros_is_not_saved_over_with_the_default() {
        let dir = std::env::temp_dir().join(format!("sb-cache-zeros-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let p = dir.join("some.json");
        save_json(&p, &vec![1, 2, 3]).unwrap();
        assert_eq!(load_json::<Vec<u32>>(&p), [1, 2, 3]);
        fs::write(&p, [0u8; 439]).unwrap();
        assert_eq!(load_json::<Vec<u32>>(&p), Vec::<u32>::new());
        let e = save_json(&p, &Vec::<u32>::new()).unwrap_err();
        assert!(durable::is_refused(&e), "{e}");
        assert!(!p.exists());
        let kept = durable::bads(&p);
        assert_eq!(fs::read(dir.join(&kept[0])).unwrap(), [0u8; 439], "the file as it was found");
        // a cache is simply written again
        write_atomic(&dir.join("sessions.json"), b"{}").unwrap();
        fs::write(dir.join("sessions.json"), [0u8; 439]).unwrap();
        assert!(load_sessions(&dir, 0.0).is_none());
        write_atomic(&dir.join("sessions.json"), b"{}").unwrap();
        let _ = fs::remove_dir_all(&dir);
    }
}
