//! Paired machines (spec section 6): pairing codes and the list of linked nodes.
//!
//! "Add a machine" mints a 6-character code (no 0, O, 1, I or L) that lasts 10 minutes and works once. A node
//! trades it at `POST /pair` for a token. The hub keeps only the token's SHA-256, in nodes.json (mode 600) next to
//! machines.json: {"nodes": [{"name", "label", "os", "token", "paired"}]}.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub const CODE_TTL: u64 = 600;
pub const CODE_LEN: usize = 6;
const ALPHABET: &[u8] = b"ABCDEFGHJKMNPQRSTUVWXYZ23456789";
const MAX_CODES: usize = 8;
/// Wrong codes allowed in a window before `/pair` refuses everything until the window ends; one address alone gets
/// fewer, so nobody can lock Add a machine for everyone (60 guesses at 8 live codes of 31^6 is still no real chance).
const MAX_MISSES: u32 = 60;
const MAX_IP_MISSES: u32 = 10;
const MISS_WINDOW: u64 = 600;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Node {
    pub name: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub os: String,
    /// SHA-256 of the token, hex.
    pub token: String,
    #[serde(default)]
    pub paired: u64,
}

impl Node {
    pub fn label(&self) -> &str {
        if self.label.is_empty() { &self.name } else { &self.label }
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct NodeFile {
    #[serde(default)]
    nodes: Vec<Node>,
}

#[derive(Default)]
struct Codes {
    live: HashMap<String, u64>,
    misses: u32,
    window: u64,
    by_ip: HashMap<std::net::IpAddr, u32>,
}

pub struct Nodes {
    path: PathBuf,
    list: Mutex<Vec<Node>>,
    codes: Mutex<Codes>,
    /// Why the file could not be read when the hub started, if it was there. Until a read works, nothing is
    /// written: saving the empty list this hub started with would unlink every machine for good.
    unread: Mutex<Option<String>>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum PairError {
    BadCode,
    TooMany,
    BadName,
    Taken(String),
    Store(String),
    /// The list of linked machines could not be read, so it is not being changed.
    Unread(String),
}

impl PairError {
    pub fn message(&self) -> String {
        match self {
            PairError::BadCode => "That code is wrong or has expired. Make a new one with Add a machine.".into(),
            PairError::TooMany => "Too many wrong codes. Wait 10 minutes, then try again.".into(),
            PairError::BadName => "A machine name is 1 to 32 lowercase letters and digits.".into(),
            PairError::Taken(l) => format!("{l} already uses that name. Pick another with --name."),
            PairError::Store(e) => format!("The hub could not save the machine ({e})."),
            PairError::Unread(e) => format!(
                "The hub could not read its list of linked machines ({e}), so it is not changing it. Check that \
                 file on the server, then try again."
            ),
        }
    }

    pub fn status(&self) -> u16 {
        match self {
            PairError::BadCode => 403,
            PairError::TooMany => 429,
            PairError::BadName => 400,
            PairError::Taken(_) => 409,
            PairError::Store(_) | PairError::Unread(_) => 500,
        }
    }
}

pub fn hash(token: &str) -> String {
    hex(&Sha256::digest(token.as_bytes()))
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn same(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn random(n: usize) -> Vec<u8> {
    let mut b = vec![0u8; n];
    getrandom::fill(&mut b).expect("the system random source");
    b
}

/// A code typed by hand: case, spaces and dashes do not matter.
pub fn normalize_code(c: &str) -> String {
    c.chars().filter(|c| c.is_ascii_alphanumeric()).map(|c| c.to_ascii_uppercase()).collect()
}

fn new_code() -> String {
    let mut out = String::with_capacity(CODE_LEN);
    while out.len() < CODE_LEN {
        for b in random(16) {
            // 248 = 31 * 8: below it every letter is equally likely
            if b < 248 && out.len() < CODE_LEN {
                out.push(ALPHABET[(b % 31) as usize] as char);
            }
        }
    }
    out
}

impl Nodes {
    /// The paired list from `path`. A file that does not parse is moved aside (nodes.json.bad) so it is never
    /// overwritten, and the hub starts with no nodes. A file that is there but cannot be read (a disk or
    /// permission error) is left alone and read again before anything is written.
    pub fn load(path: &Path) -> Nodes {
        let (list, unread) = match read(path) {
            Ok(list) => (list, None),
            Err(e) => {
                crate::server::log(format!(
                    "{}: {e}; no linked machines until it can be read, and it is not written until then",
                    path.display()
                ));
                (vec![], Some(e))
            }
        };
        Nodes {
            path: path.to_path_buf(),
            list: Mutex::new(list),
            codes: Mutex::new(Codes::default()),
            unread: Mutex::new(unread),
        }
    }

    /// Before a write: if the file could not be read at the start, read it now. Err while it still cannot be.
    fn readable(&self, list: &mut Vec<Node>) -> Result<(), String> {
        let mut unread = self.unread.lock().unwrap_or_else(|e| e.into_inner());
        if unread.is_some() {
            *list = read(&self.path)?;
            *unread = None;
        }
        Ok(())
    }

    pub fn list(&self) -> Vec<Node> {
        let mut list = self.list.lock().unwrap_or_else(|e| e.into_inner());
        let _ = self.readable(&mut list); // a file that could not be read at the start may be readable by now
        list.clone()
    }

    /// The list could not be read (and still cannot): a token that matches nothing may be a good one.
    pub fn unread(&self) -> bool {
        let mut list = self.list.lock().unwrap_or_else(|e| e.into_inner());
        self.readable(&mut list).is_err()
    }

    pub fn get(&self, name: &str) -> Option<Node> {
        self.list().into_iter().find(|n| n.name == name)
    }

    /// The node a token belongs to.
    pub fn find(&self, token: &str) -> Option<Node> {
        if token.is_empty() {
            return None;
        }
        let h = hash(token);
        let mut found = None;
        for n in self.list() {
            if same(&n.token, &h) {
                found = Some(n);
            }
        }
        found
    }

    /// A new pairing code and when it expires.
    pub fn mint(&self, now: u64) -> (String, u64) {
        let mut c = self.codes.lock().unwrap_or_else(|e| e.into_inner());
        c.live.retain(|_, exp| *exp > now);
        while c.live.len() >= MAX_CODES {
            let oldest = c.live.iter().min_by_key(|(_, exp)| **exp).map(|(k, _)| k.clone());
            if let Some(k) = oldest {
                c.live.remove(&k);
            }
        }
        let code = new_code();
        let exp = now + CODE_TTL;
        c.live.insert(code.clone(), exp);
        (code, exp)
    }

    /// Trade a code for a token. `taken` names an ssh machine that already has `name` (its label), if any.
    /// Pairing a name that is already a node replaces it: the old token stops working. (`/pair` lets that happen only
    /// with the node's current token: nodeapi.rs passes the node's label as `taken` otherwise.)
    pub fn pair(
        &self,
        code: &str,
        name: &str,
        label: &str,
        os: &str,
        taken: Option<&str>,
        now: u64,
    ) -> Result<(Node, String), PairError> {
        let code = normalize_code(code);
        {
            let mut c = self.codes.lock().unwrap_or_else(|e| e.into_inner());
            if now >= c.window + MISS_WINDOW {
                c.window = now;
                c.misses = 0;
                c.by_ip.clear();
            }
            if c.misses >= MAX_MISSES {
                return Err(PairError::TooMany);
            }
            if !crate::ticket::valid_machine(name) {
                return Err(PairError::BadName);
            }
            // the code first: without one, nobody learns which names are taken (and every try counts)
            c.live.retain(|_, exp| *exp > now);
            if !c.live.contains_key(&code) {
                c.misses += 1;
                return Err(PairError::BadCode);
            }
            if let Some(l) = taken {
                return Err(PairError::Taken(l.to_string()));
            }
            c.live.remove(&code);
        }
        let token = hex(&random(32));
        let clean = |s: &str, max: usize| {
            s.chars().filter(|c| !c.is_control()).take(max).collect::<String>().trim().to_string()
        };
        let node =
            Node { name: name.into(), label: clean(label, 40), os: clean(os, 16), token: hash(&token), paired: now };
        let mut list = self.list.lock().unwrap_or_else(|e| e.into_inner());
        let saved = match self.readable(&mut list) {
            Err(e) => Err(PairError::Unread(e)),
            Ok(()) => {
                let mut next: Vec<Node> = list.iter().filter(|n| n.name != name).cloned().collect();
                next.push(node.clone());
                save(&self.path, &next).map(|()| next).map_err(|e| PairError::Store(e.to_string()))
            }
        };
        let next = match saved {
            Ok(next) => next,
            Err(e) => {
                // the code was not used up: it works again once the file does
                self.codes.lock().unwrap_or_else(|e| e.into_inner()).live.insert(code, now + CODE_TTL);
                return Err(e);
            }
        };
        *list = next;
        Ok((node, token))
    }

    /// `pair` for a request from `ip`: an address that guessed wrong too often is refused on its own.
    #[allow(clippy::too_many_arguments)]
    pub fn pair_from(
        &self,
        ip: std::net::IpAddr,
        code: &str,
        name: &str,
        label: &str,
        os: &str,
        taken: Option<&str>,
        now: u64,
    ) -> Result<(Node, String), PairError> {
        {
            let c = self.codes.lock().unwrap_or_else(|e| e.into_inner());
            let fresh = now >= c.window + MISS_WINDOW;
            if !fresh && c.by_ip.get(&ip.to_canonical()).copied().unwrap_or(0) >= MAX_IP_MISSES {
                return Err(PairError::TooMany);
            }
        }
        let r = self.pair(code, name, label, os, taken, now);
        if r == Err(PairError::BadCode) {
            *self.codes.lock().unwrap_or_else(|e| e.into_inner()).by_ip.entry(ip.to_canonical()).or_default() += 1;
        }
        r
    }

    /// Forget a node: its token stops working at once.
    pub fn remove(&self, name: &str) -> std::io::Result<bool> {
        let mut list = self.list.lock().unwrap_or_else(|e| e.into_inner());
        self.readable(&mut list).map_err(std::io::Error::other)?;
        if !list.iter().any(|n| n.name == name) {
            return Ok(false);
        }
        let next: Vec<Node> = list.iter().filter(|n| n.name != name).cloned().collect();
        save(&self.path, &next)?;
        *list = next;
        Ok(true)
    }
}

/// The nodes in the file at `path`: none when there is no file. A file that does not parse is moved aside
/// (nodes.json.bad) and counts as none. Err only when the file is there and reading it failed.
fn read(path: &Path) -> Result<Vec<Node>, String> {
    let text = match std::fs::read(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
        Err(e) => return Err(e.to_string()),
    };
    match serde_json::from_slice::<NodeFile>(&text) {
        Ok(f) => Ok(f.nodes.into_iter().filter(|n| crate::ticket::valid_machine(&n.name)).collect()),
        Err(e) => {
            crate::server::log(format!("{}: {e}; moved to .bad, starting with no linked machines", path.display()));
            let _ = std::fs::rename(path, path.with_extension("json.bad"));
            Ok(vec![])
        }
    }
}

fn save(path: &Path, nodes: &[Node]) -> std::io::Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;
    let tmp = dir.join(format!(".nodes.json.{}.tmp", std::process::id()));
    let text = serde_json::to_string_pretty(&NodeFile { nodes: nodes.to_vec() }).map_err(std::io::Error::other)?;
    {
        use std::io::Write;
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut f = opts.open(&tmp)?;
        f.write_all(text.as_bytes())?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("sbnodes-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d.join("nodes.json")
    }

    #[test]
    fn codes() {
        let c = new_code();
        assert_eq!(c.len(), CODE_LEN);
        assert!(c.bytes().all(|b| ALPHABET.contains(&b)), "{c}");
        assert_eq!(normalize_code(" k7m-2qx "), "K7M2QX");
    }

    #[test]
    fn a_code_pairs_once_and_the_token_finds_the_node() {
        let p = tmp("once");
        let n = Nodes::load(&p);
        let (code, exp) = n.mint(1000);
        assert_eq!(exp, 1000 + CODE_TTL);
        let (node, token) = n.pair(&code.to_lowercase(), "lab", "Lab box", "linux", None, 1010).unwrap();
        assert_eq!(node.label(), "Lab box");
        assert_eq!(token.len(), 64);
        assert_ne!(node.token, token, "only the hash is kept");
        assert_eq!(n.find(&token).map(|x| x.name), Some("lab".into()));
        assert_eq!(n.find("nope"), None);
        assert_eq!(n.find(""), None);
        assert_eq!(n.pair(&code, "lab", "", "", None, 1020), Err(PairError::BadCode), "single use");

        // saved with only the hash, mode 600, and read back
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(!text.contains(&token));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&p).unwrap().permissions().mode() & 0o777, 0o600);
        }
        assert_eq!(Nodes::load(&p).find(&token).map(|x| x.name), Some("lab".into()));

        // pairing the name again replaces the node and its token
        let (code2, _) = n.mint(1030);
        let (_, token2) = n.pair(&code2, "lab", "Lab", "linux", None, 1031).unwrap();
        assert_eq!(n.find(&token), None);
        assert!(n.find(&token2).is_some());
        assert_eq!(n.list().len(), 1);
        assert!(n.remove("lab").unwrap());
        assert_eq!(n.find(&token2), None);
        assert!(!n.remove("lab").unwrap());
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }

    #[test]
    fn expiry_names_and_too_many_misses() {
        let p = tmp("rules");
        let n = Nodes::load(&p);
        let (code, _) = n.mint(0);
        assert_eq!(n.pair(&code, "Lab!", "", "", None, 1), Err(PairError::BadName));
        assert_eq!(n.pair(&code, "desk", "", "", Some("Desk"), 1), Err(PairError::Taken("Desk".into())));
        // a refused name does not spend the code
        assert!(n.pair(&code, "lab", "", "", None, 1).is_ok());
        let (code, _) = n.mint(2);
        assert_eq!(n.pair(&code, "lab", "", "", None, 2 + CODE_TTL), Err(PairError::BadCode), "expired");

        let n = Nodes::load(&p);
        let (good, _) = n.mint(100);
        for _ in 0..MAX_MISSES {
            assert_eq!(n.pair("AAAAAA", "x", "", "", None, 100), Err(PairError::BadCode));
        }
        assert_eq!(n.pair(&good, "x", "", "", None, 100), Err(PairError::TooMany));
        let (good, _) = n.mint(100 + MISS_WINDOW);
        assert!(n.pair(&good, "x", "", "", None, 100 + MISS_WINDOW).is_ok(), "the window ends");
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }

    #[test]
    fn one_address_cannot_lock_pairing_for_everyone() {
        // security M5: 20 wrong codes from anyone locked Add a machine for everyone
        let p = tmp("perip");
        let n = Nodes::load(&p);
        let (bad, good): (std::net::IpAddr, std::net::IpAddr) =
            ("10.0.0.66".parse().unwrap(), "10.0.0.5".parse().unwrap());
        for _ in 0..MAX_IP_MISSES {
            assert_eq!(n.pair_from(bad, "AAAAAA", "x", "", "", None, 1), Err(PairError::BadCode));
        }
        let (code, _) = n.mint(1);
        assert_eq!(n.pair_from(bad, &code, "x", "", "", None, 1), Err(PairError::TooMany));
        assert!(n.pair_from(good, &code, "lab", "", "", None, 1).is_ok(), "the owner still pairs");
        let (code, _) = n.mint(1 + MISS_WINDOW);
        assert!(n.pair_from(bad, &code, "box", "", "", None, 1 + MISS_WINDOW).is_ok(), "the window ends for it too");
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }

    #[test]
    fn a_wrong_code_never_says_a_name_is_taken() {
        // security L2: the name check ran before the code check and before the miss count
        let p = tmp("names");
        let n = Nodes::load(&p);
        assert_eq!(n.pair("AAAAAA", "desk", "", "", Some("Desk"), 1), Err(PairError::BadCode));
        for _ in 1..MAX_MISSES {
            let _ = n.pair("AAAAAA", "desk", "", "", Some("Desk"), 1);
        }
        let (code, _) = n.mint(1);
        assert_eq!(n.pair(&code, "lab", "", "", None, 1), Err(PairError::TooMany), "every guess counted");
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn a_file_that_cannot_be_read_is_never_written_over() {
        use std::os::unix::fs::PermissionsExt;
        let p = tmp("unread");
        let n = Nodes::load(&p);
        let (code, _) = n.mint(1);
        let (_, token) = n.pair(&code, "lab", "Lab", "linux", None, 1).unwrap();
        let before = std::fs::read(&p).unwrap();
        let mode = |m: u32| std::fs::set_permissions(&p, std::fs::Permissions::from_mode(m)).unwrap();
        mode(0o000);
        if std::fs::read(&p).is_ok() {
            return; // running as root: nothing is unreadable
        }
        // the hub starts while the file cannot be read: it knows no machines, and says so
        let n = Nodes::load(&p);
        assert!(n.list().is_empty());
        assert!(n.unread());
        assert!(n.find(&token).is_none());
        // ... and it changes nothing: pairing another machine would have saved a list without "lab"
        let (code, _) = n.mint(10);
        let refused = n.pair(&code, "desk", "", "", None, 10);
        assert!(matches!(refused, Err(PairError::Unread(_))), "{refused:?}");
        assert_eq!(refused.unwrap_err().status(), 500);
        assert!(n.remove("lab").is_err());
        mode(0o600);
        assert_eq!(std::fs::read(&p).unwrap(), before, "untouched");
        // readable again: the machines are back without a restart, and the same code still pairs
        assert_eq!(n.find(&token).map(|x| x.name), Some("lab".into()));
        assert!(!n.unread());
        n.pair(&code, "desk", "", "", None, 11).unwrap();
        let names: Vec<String> = Nodes::load(&p).list().into_iter().map(|x| x.name).collect();
        assert_eq!(names, ["lab", "desk"]);
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }

    #[test]
    fn a_broken_file_is_moved_aside() {
        let p = tmp("broken");
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, "{nope").unwrap();
        let n = Nodes::load(&p);
        assert!(n.list().is_empty());
        assert!(p.with_extension("json.bad").exists());
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }
}
