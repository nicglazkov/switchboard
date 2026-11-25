//! The one file this crate keeps: where the home computer was last reached directly, so the next start can try
//! that address even when the relay is away. Nothing in it is secret (an id and an address on the network the
//! phone was on), and losing it costs nothing but that shortcut, so a file that cannot be read is no file.

use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

const NAME: &str = "home-addrs.json";
/// More than a few addresses is not a memory of where home was, it is a list to scan.
const MOST: usize = 4;

#[derive(Serialize, Deserialize)]
struct Kept {
    v: u32,
    home: String,
    addrs: Vec<SocketAddr>,
}

fn path(dir: &Path) -> PathBuf {
    dir.join(NAME)
}

/// The addresses kept for `home`. None kept for it, or a file for another home: empty.
pub fn load(dir: &Path, home: &str) -> Vec<SocketAddr> {
    let Ok(bytes) = std::fs::read(path(dir)) else { return vec![] };
    match serde_json::from_slice::<Kept>(&bytes) {
        Ok(k) if k.v == 1 && k.home == home => k.addrs.into_iter().take(MOST).collect(),
        _ => vec![],
    }
}

/// Keeps `addrs` for `home`, in place of whatever was kept. Written whole (a temporary file, then moved).
pub fn save(dir: &Path, home: &str, addrs: &[SocketAddr]) {
    let kept = Kept { v: 1, home: home.to_string(), addrs: addrs.iter().take(MOST).copied().collect() };
    let Ok(bytes) = serde_json::to_vec(&kept) else { return };
    let _ = std::fs::create_dir_all(dir);
    let tmp = dir.join(format!("{NAME}.tmp"));
    if std::fs::write(&tmp, bytes).is_ok() && std::fs::rename(&tmp, path(dir)).is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_are_kept_for_one_home_only() {
        let dir = crate::tests::scratch("store");
        let a: SocketAddr = "192.0.2.20:4433".parse().unwrap();
        assert!(load(&dir, "aa").is_empty());
        save(&dir, "aa", &[a]);
        assert_eq!(load(&dir, "aa"), vec![a]);
        assert!(load(&dir, "bb").is_empty(), "another home's addresses are not this one's");
        save(&dir, "bb", &[]);
        assert!(load(&dir, "aa").is_empty() && load(&dir, "bb").is_empty());
        std::fs::write(path(&dir), b"\0\0\0").unwrap();
        assert!(load(&dir, "bb").is_empty(), "a damaged file is no file");
        save(&dir, "bb", &[a; 9]);
        assert_eq!(load(&dir, "bb").len(), MOST);
    }
}
