//! Moving the home computer to another computer, keeping a sealed copy of it, and bringing it back from one.
//!
//! The home computer is its key and its state. The state is everything the server and the hub keep (packed and
//! unpacked by `server/homestate.py`, which never touches a key); the key is `device.key`. Whoever holds both
//! is the home computer, so the rule every step here keeps is: **never two**. Zero for a moment is fine.
//!
//! # A move, on the computer that is home
//!
//! `home-move.json` (next to the server's settings) says how far it got, written before each step is taken:
//!
//! 1. `packing`: the other computer was asked whether it can take over (`home/status`). Pairing and removing
//!    are refused from here on. The server and the hub here are stopped, and the state is packed.
//! 2. `packed`, then `sent`: the state is sent (`home/receive`). The other computer unpacks it into a staging
//!    folder and checks it against the hash; nothing it uses has changed, and it has no key.
//! 3. `stepped_down`: `home-stepdown.json` is on disk (the server, the hub and this helper all refuse to be
//!    home while it is there) and the endpoint that answered as home is closed. Nobody is home now.
//! 4. `committing`: only now is the key sent (`home/commit`), with the home key itself as the caller: the
//!    other computer answers its pinned home only, and checks that the key it is handed is that home's.
//! 5. `committed`: it answered. This computer takes a new key of its own, notes the other computer as its home
//!    and goes on as an ordinary computer (or, with `--retire`, as none).
//!
//! Stopped anywhere before 3, nothing has changed: `--resume` starts again, `--undo` tidies up. Stopped at 3,
//! the key never left: `--undo` puts this computer back. Stopped at 4, the other computer may or may not have
//! taken over, and only it knows: `--resume` asks it (as the home, if it still answers as the computer it was;
//! as this computer's next key, if it answers as home already) and finishes either way; `--undo` is refused
//! unless the other computer says it has not taken over, and has thrown the state away.
//!
//! # The same move, on the computer that takes over
//!
//! `home/commit` writes `home-arrive.json`, puts the key aside as `device.key.home`, swaps the staged state in,
//! makes the key this computer's own, takes itself off the list of devices and puts the old home on it, raises
//! the list's epoch, and answers. A stop anywhere in that is finished at the next look (`settle`); until it is,
//! this computer is not home, whatever the settings that arrived say.
//!
//! # A copy, and a home computer brought back from one
//!
//! `backup` packs the state while everything runs and seals it with the key (sb-link backup.rs) under a key
//! worked out from a recovery code. The code is made at the first backup, returned once, and kept nowhere:
//! `backup.key` holds what seals the next backup and cannot be turned back into the code. `restore` is the
//! second half of a move with nobody on the other side: the epoch rises by one, and for seven days this home
//! takes what the other devices show it of the list as it was after the backup was made (`adopt`).
//!
//! # A home computer that was replaced
//!
//! A home computer that is shown a list its own key signed with a higher epoch than it ever used has been
//! moved or restored somewhere else. It writes `home-stepdown.json`, says why, and stops answering as home.
//!
//! No key and no recovery code is ever logged or printed by this file, except the code once, in the answer to
//! the backup that made it.

use super::*;
use sb_link::backup::{self, BackupError, RecoveryCode, Sealer};
use sb_link::zeroize::{Zeroize, Zeroizing};
use serde::{Deserialize, Serialize};
use std::io::Write as _;
use std::sync::Weak;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;
use tokio::io::AsyncWriteExt;

/// The note a computer keeps once it has stopped being the home computer. The server (server/homestate.py) and
/// the hub look for the same file.
pub const MARKER: &str = "home-stepdown.json";
const MOVE_JOURNAL: &str = "home-move.json";
const ARRIVE_JOURNAL: &str = "home-arrive.json";
/// The key the old home computer goes on with, made before it steps down.
const NEXT_KEY: &str = "device.key.next";
/// The home key on the computer that takes over, from the moment it arrives until it is this computer's own.
const ARRIVING_KEY: &str = "device.key.home";
const BACKUP_KEY: &str = "backup.key";
/// A list of devices a computer that was not home held when it became home from a backup: looked at once.
const WITNESS: &str = "home-witness.json";
const INCOMING: &str = "home-incoming";
const OUTGOING: &str = "home-outgoing";
const BACKUPS: &str = "home-backup";

pub const STOOD_DOWN: &str = "This computer was the home computer until it moved. It is not answering as home.";
pub const MOVING: &str = "The home computer is moving to another computer. Try again when that is done.";
const NO_PARTS: &str = "That computer does not have the parts a home computer needs. Install them there first: \
                        install-server.sh --standby";
const NOBODY_HOME: &str = "This computer has stepped down and the other computer has not confirmed that it took \
                           over, so nobody is the home computer right now. When the other computer can be reached, \
                           run: switchboard-node home move --resume";

/// The two folders the server keeps its settings and its state in.
#[derive(Debug, Clone)]
pub struct Folders {
    pub config: PathBuf,
    pub state: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Packed {
    pub sha256: String,
    pub bytes: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Staged {
    /// The hash of the archive that is staged and checked, when one is.
    pub staged: Option<String>,
    /// A server or a hub is running on the state folder.
    pub held: bool,
}

pub type Fut<'a, T> = std::pin::Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Packs and unpacks the home computer's state. The real one runs `server/homestate.py`.
pub trait StateMover: Send + Sync + 'static {
    /// Whether the tool is on this computer.
    fn present(&self) -> bool;
    fn pack<'a>(&'a self, f: &'a Folders, out: &'a Path) -> Fut<'a, Result<Packed, Refusal>>;
    fn stage<'a>(
        &'a self,
        f: &'a Folders,
        archive: &'a Path,
        sha256: Option<&'a str>,
    ) -> Fut<'a, Result<Packed, Refusal>>;
    fn swap<'a>(&'a self, f: &'a Folders) -> Fut<'a, Result<(), Refusal>>;
    fn discard<'a>(&'a self, f: &'a Folders) -> Fut<'a, Result<(), Refusal>>;
    fn status<'a>(&'a self, f: &'a Folders) -> Fut<'a, Result<Staged, Refusal>>;
}

/// The server and the hub on this computer, as one thing to stop and start. The real one runs the installed
/// `switchboard` command with `stop` or `start`.
pub trait Parts: Send + Sync + 'static {
    fn present(&self) -> bool;
    fn run<'a>(&'a self, verb: &'a str) -> Fut<'a, Result<(), String>>;
}

fn kind_of(kind: Option<&str>) -> &'static str {
    match kind {
        Some("held") => "held",
        Some("damaged") => "damaged",
        Some("newer") => "newer",
        Some("nothing") => "nothing",
        Some("unsafe") => "unsafe",
        Some("unreadable") => "unreadable",
        _ => "failed",
    }
}

/// `python homestate.py <verb> --config DIR --state DIR ...`, which prints one line of JSON.
pub struct PyMover {
    tool: PathBuf,
    /// node.json: which Python this computer's helper runs.
    config: PathBuf,
}

impl PyMover {
    async fn run(&self, f: &Folders, verb: &str, more: &[(&str, String)]) -> Result<Value, Refusal> {
        let python = crate::load_raw(&self.config).unwrap_or_default().python();
        let mut cmd = tokio::process::Command::new(&python);
        cmd.arg(&self.tool).arg(verb).arg("--config").arg(&f.config).arg("--state").arg(&f.state);
        for (k, v) in more {
            cmd.arg(k).arg(v);
        }
        cmd.stdin(std::process::Stdio::null()).kill_on_drop(true);
        let out = cmd.output().await.map_err(|e| {
            Refusal::new(
                "failed",
                format!("The home computer's state could not be handled: {python} did not start ({e})."),
            )
        })?;
        let text = String::from_utf8_lossy(&out.stdout);
        let v = text.lines().rev().find(|l| !l.trim().is_empty()).and_then(|l| serde_json::from_str::<Value>(l).ok());
        match v {
            Some(v) if v["ok"] == true => Ok(v),
            Some(v) => Err(Refusal::new(
                kind_of(v["kind"].as_str()),
                v["error"].as_str().unwrap_or("The home computer's state could not be handled.").to_string(),
            )),
            None => {
                Err(Refusal::new("failed", "The home computer's state could not be handled: the tool said nothing."))
            }
        }
    }

    fn packed(v: &Value) -> Result<Packed, Refusal> {
        match (v["sha256"].as_str(), v["bytes"].as_u64()) {
            (Some(s), Some(b)) => Ok(Packed { sha256: s.to_string(), bytes: b }),
            _ => {
                Err(Refusal::new("failed", "The home computer's state could not be handled: the tool's answer is odd."))
            }
        }
    }
}

impl StateMover for PyMover {
    fn present(&self) -> bool {
        self.tool.is_file()
    }

    fn pack<'a>(&'a self, f: &'a Folders, out: &'a Path) -> Fut<'a, Result<Packed, Refusal>> {
        Box::pin(async move { PyMover::packed(&self.run(f, "pack", &[("--out", out.display().to_string())]).await?) })
    }

    fn stage<'a>(
        &'a self,
        f: &'a Folders,
        archive: &'a Path,
        sha256: Option<&'a str>,
    ) -> Fut<'a, Result<Packed, Refusal>> {
        Box::pin(async move {
            let mut more = vec![("--in", archive.display().to_string())];
            if let Some(s) = sha256 {
                more.push(("--sha256", s.to_string()));
            }
            PyMover::packed(&self.run(f, "stage", &more).await?)
        })
    }

    fn swap<'a>(&'a self, f: &'a Folders) -> Fut<'a, Result<(), Refusal>> {
        Box::pin(async move { self.run(f, "swap", &[]).await.map(|_| ()) })
    }

    fn discard<'a>(&'a self, f: &'a Folders) -> Fut<'a, Result<(), Refusal>> {
        Box::pin(async move { self.run(f, "discard", &[]).await.map(|_| ()) })
    }

    fn status<'a>(&'a self, f: &'a Folders) -> Fut<'a, Result<Staged, Refusal>> {
        Box::pin(async move {
            let v = self.run(f, "status", &[]).await?;
            Ok(Staged { staged: v["staged"].as_str().map(str::to_string), held: v["held"] == true })
        })
    }
}

pub struct CmdParts {
    /// The program and what comes before the verb.
    argv: Vec<String>,
    /// The file that has to be there for the parts to count as installed.
    file: PathBuf,
}

impl Parts for CmdParts {
    fn present(&self) -> bool {
        self.file.is_file()
    }

    fn run<'a>(&'a self, verb: &'a str) -> Fut<'a, Result<(), String>> {
        Box::pin(async move {
            let mut cmd = tokio::process::Command::new(&self.argv[0]);
            // (nothing of what it prints is read: a part it starts must not keep this waiting on an open pipe)
            let null = std::process::Stdio::null;
            cmd.args(&self.argv[1..]).arg(verb).stdin(null()).stdout(null()).stderr(null()).kill_on_drop(true);
            let ran = tokio::time::timeout(Duration::from_secs(180), cmd.status()).await;
            match ran {
                Err(_) => Err(format!("{} {verb} did not finish", self.argv[0])),
                Ok(Err(e)) => Err(format!("{} did not start ({e})", self.argv[0])),
                Ok(Ok(st)) if st.success() => Ok(()),
                Ok(Ok(st)) => Err(format!("{} {verb} ended with {st}", self.argv[0])),
            }
        })
    }
}

/// What moving, copying and restoring the home computer work with on this computer.
pub struct Kit {
    pub folders: Folders,
    pub mover: Arc<dyn StateMover>,
    pub parts: Arc<dyn Parts>,
    /// Held by the one move, backup or restore that is under way.
    one: tokio::sync::Mutex<()>,
    /// While the home computer is moving, nobody is paired or removed.
    frozen: AtomicBool,
    /// Until then the files are not looked at: an answer is on its way to the computer that asked.
    hold: Mutex<Option<Instant>>,
    /// Where a test stops a move, as a power cut would.
    #[cfg(test)]
    pub stop_at: Mutex<Option<&'static str>>,
}

impl Kit {
    /// From the environment: `SWITCHBOARD_HOMESTATE` names homestate.py and `SWITCHBOARD_HOME_PARTS` the program
    /// that stops and starts the server and the hub, for a setup beside the installed one. Without them, where
    /// the installer puts both.
    pub fn new(places: &Places, env: &dyn Fn(&str) -> Option<String>) -> Kit {
        let dir = |p: &Path| p.parent().map(Path::to_path_buf).unwrap_or_default();
        let folders = Folders { config: dir(&places.server_json), state: dir(&places.paired) };
        let home = crate::home();
        let app = if cfg!(windows) {
            env("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(|| home.join("AppData/Local")).join("Switchboard")
        } else {
            home.join(".local/share/switchboard")
        };
        let tool = env("SWITCHBOARD_HOMESTATE")
            .map(PathBuf::from)
            .unwrap_or_else(|| app.join("app").join("server").join("homestate.py"));
        let parts = match env("SWITCHBOARD_HOME_PARTS") {
            Some(p) => CmdParts { argv: vec![p.clone()], file: PathBuf::from(p) },
            None if cfg!(windows) => {
                let script = app.join("switchboard.ps1");
                let argv = ["powershell", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File"];
                CmdParts {
                    argv: argv.iter().map(|s| s.to_string()).chain([script.display().to_string()]).collect(),
                    file: script,
                }
            }
            None => {
                let script = home.join(".local/bin/switchboard");
                CmdParts { argv: vec![script.display().to_string()], file: script }
            }
        };
        Kit::with(folders, Arc::new(PyMover { tool, config: places.config.clone() }), Arc::new(parts))
    }

    /// Nothing to pack with and nothing to stop or start: this computer can keep no copy of a home computer
    /// and cannot become one.
    pub fn none(places: &Places) -> Kit {
        struct Absent;
        impl StateMover for Absent {
            fn present(&self) -> bool {
                false
            }
            fn pack<'a>(&'a self, _: &'a Folders, _: &'a Path) -> Fut<'a, Result<Packed, Refusal>> {
                Box::pin(async { Err(Refusal::new("no_parts", "The home computer's parts are not on this computer.")) })
            }
            fn stage<'a>(
                &'a self,
                _: &'a Folders,
                _: &'a Path,
                _: Option<&'a str>,
            ) -> Fut<'a, Result<Packed, Refusal>> {
                Box::pin(async { Err(Refusal::new("no_parts", "The home computer's parts are not on this computer.")) })
            }
            fn swap<'a>(&'a self, _: &'a Folders) -> Fut<'a, Result<(), Refusal>> {
                Box::pin(async { Err(Refusal::new("no_parts", "The home computer's parts are not on this computer.")) })
            }
            fn discard<'a>(&'a self, _: &'a Folders) -> Fut<'a, Result<(), Refusal>> {
                Box::pin(async { Ok(()) })
            }
            fn status<'a>(&'a self, _: &'a Folders) -> Fut<'a, Result<Staged, Refusal>> {
                Box::pin(async { Ok(Staged::default()) })
            }
        }
        impl Parts for Absent {
            fn present(&self) -> bool {
                false
            }
            fn run<'a>(&'a self, _: &'a str) -> Fut<'a, Result<(), String>> {
                Box::pin(async { Ok(()) })
            }
        }
        let dir = |p: &Path| p.parent().map(Path::to_path_buf).unwrap_or_default();
        let folders = Folders { config: dir(&places.server_json), state: dir(&places.paired) };
        Kit::with(folders, Arc::new(Absent), Arc::new(Absent))
    }

    pub fn with(folders: Folders, mover: Arc<dyn StateMover>, parts: Arc<dyn Parts>) -> Kit {
        Kit {
            folders,
            mover,
            parts,
            one: tokio::sync::Mutex::new(()),
            frozen: AtomicBool::new(false),
            hold: Mutex::new(None),
            #[cfg(test)]
            stop_at: Mutex::new(None),
        }
    }

    /// Whether this computer could take over as the home computer.
    pub fn can_host(&self) -> bool {
        self.mover.present() && self.parts.present()
    }

    pub fn frozen(&self) -> bool {
        self.frozen.load(Ordering::SeqCst)
    }

    pub(super) fn held_back(&self) -> bool {
        self.hold.lock().unwrap_or_else(|e| e.into_inner()).is_some_and(|t| Instant::now() < t)
    }

    /// A move that was started here and is not finished.
    pub(super) fn unfinished(&self) -> bool {
        self.file(MOVE_JOURNAL).exists()
    }

    /// At each look at the files: a move that a stopped helper left half done still holds pairing back.
    pub(super) fn look(&self) {
        if self.unfinished() {
            self.frozen.store(true, Ordering::SeqCst);
        }
    }

    fn hold_for(&self, d: Duration) {
        *self.hold.lock().unwrap_or_else(|e| e.into_inner()) = Some(Instant::now() + d);
    }

    fn file(&self, name: &str) -> PathBuf {
        self.folders.config.join(name)
    }

    #[cfg(test)]
    fn stops_at(&self, step: &'static str) -> bool {
        *self.stop_at.lock().unwrap() == Some(step)
    }
    /// A build for trying things out can be told to end, as a power cut would end it, at one step of a move
    /// (SWITCHBOARD_NODE_DIE_AT=packed | stepped_down | committing | committed). A release build has none of this.
    #[cfg(all(not(test), debug_assertions))]
    fn stops_at(&self, step: &'static str) -> bool {
        if std::env::var("SWITCHBOARD_NODE_DIE_AT").is_ok_and(|v| v == step) {
            log(format!("keys: ending here, at the step \"{step}\", because SWITCHBOARD_NODE_DIE_AT says so"));
            std::process::exit(86);
        }
        false
    }
    #[cfg(all(not(test), not(debug_assertions)))]
    fn stops_at(&self, _: &'static str) -> bool {
        false
    }
}

// ---- the note of a computer that is no longer home -------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StoodDown {
    pub v: u32,
    /// `moving` (a move is under way), `moved`, or `superseded` (another computer answers as this home).
    pub why: String,
    pub at: u64,
    /// The home this computer was.
    #[serde(default)]
    pub home: String,
    #[serde(default)]
    pub epoch: u64,
}

/// The note in `config`, if there is one. A note that is there and cannot be read still counts.
pub fn stood_down(config: &Path) -> Option<StoodDown> {
    let unreadable = || StoodDown { v: 1, why: "unreadable".into(), at: 0, home: String::new(), epoch: 0 };
    match std::fs::read(config.join(MARKER)) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(_) => Some(unreadable()),
        Ok(raw) => Some(serde_json::from_slice(&raw).unwrap_or_else(|_| unreadable())),
    }
}

fn write_marker(config: &Path, why: &str, home: &DeviceId, epoch: u64) -> std::io::Result<()> {
    let note = StoodDown { v: 1, why: why.into(), at: unix_now(), home: home.to_string(), epoch };
    // (forced to the disk before the name exists: this is the step everything after it relies on)
    Store::new(config.join(MARKER)).private().put_json(&note, true)
}

fn clear_marker(config: &Path) {
    let _ = std::fs::remove_file(config.join(MARKER));
}

/// What a person is told on a computer that stood down.
pub fn stood_down_line(note: &StoodDown) -> String {
    match note.why.as_str() {
        "moving" => format!("{STOOD_DOWN} The move is not finished: switchboard-node home move --resume"),
        "superseded" => format!(
            "{STOOD_DOWN} Another computer answers as the home computer now (it was moved there, or brought back \
             there from a backup). To use this computer again, pair it with the home computer: switchboard-node \
             pair <code or link>"
        ),
        _ => STOOD_DOWN.to_string(),
    }
}

// ---- small things ----------------------------------------------------------------------------------------------

fn hex(bytes: &[u8]) -> Zeroizing<String> {
    let mut out = Zeroizing::new(String::with_capacity(bytes.len() * 2));
    for b in bytes {
        out.push(char::from_digit(u32::from(b >> 4), 16).unwrap_or('0'));
        out.push(char::from_digit(u32::from(b & 15), 16).unwrap_or('0'));
    }
    out
}

fn unhex32(text: &str) -> Option<Zeroizing<[u8; 32]>> {
    let t = text.as_bytes();
    if t.len() != 64 {
        return None;
    }
    let mut out = Zeroizing::new([0u8; 32]);
    for (i, pair) in t.chunks(2).enumerate() {
        let hi = char::from(pair[0]).to_digit(16)?;
        let lo = char::from(pair[1]).to_digit(16)?;
        out[i] = (hi * 16 + lo) as u8;
    }
    Some(out)
}

/// Writes zeros over one of the key files this file makes, then removes it. Only those names, only in `dir`.
fn scrub(dir: &Path, name: &str) {
    const MINE: [&str; 5] = ["device.key", "device.key.bak", NEXT_KEY, ARRIVING_KEY, BACKUP_KEY];
    if !MINE.contains(&name) {
        return;
    }
    let p = dir.join(name);
    if let Ok(meta) = std::fs::metadata(&p)
        && meta.is_file()
        && let Ok(mut f) = std::fs::OpenOptions::new().write(true).open(&p)
    {
        let _ = f.write_all(&vec![0u8; meta.len().min(4096) as usize]);
        let _ = f.sync_all();
    }
    let _ = std::fs::remove_file(&p);
}

/// A key file of this file's own: 32 bytes, for this user only, on the disk before the name exists.
fn put_key(path: &Path, key: &DeviceKey) -> std::io::Result<()> {
    Store::new(path).private().put(key.secret_bytes(), true)
}

fn read_key(path: &Path) -> Option<DeviceKey> {
    let mut raw = std::fs::read(path).ok()?;
    let key: Option<[u8; 32]> = raw.as_slice().try_into().ok();
    raw.zeroize();
    key.map(DeviceKey::from_bytes)
}

/// A folder of this file's own inside the state folder, for this user only.
fn private_dir(state: &Path, name: &str) -> std::io::Result<PathBuf> {
    let d = state.join(name);
    std::fs::create_dir_all(&d)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&d, std::fs::Permissions::from_mode(0o700));
    }
    Ok(d)
}

fn private_file(path: &Path) -> std::io::Result<std::fs::File> {
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    o.open(path)
}

fn load<T: serde::de::DeserializeOwned>(path: &Path) -> Option<T> {
    Store::new(path).private().load_json::<T>().0
}

fn save<T: Serialize>(path: &Path, v: &T) -> Result<(), Refusal> {
    Store::new(path)
        .private()
        .put_json(v, true)
        .map_err(|e| Refusal::new("save", format!("{} could not be saved ({e}). Nothing was changed.", path.display())))
}

// ---- asking another computer -----------------------------------------------------------------------------------

/// One question to a computer's own surface over a tunnel to it.
async fn own_ask(
    tunnel: &Tunnel,
    method: &str,
    path: &str,
    body: Option<&[u8]>,
    secs: u64,
) -> Result<(u16, Value), String> {
    let asked = async {
        let mut s = tunnel.open().await?;
        sb_link::tunnel::write_header(&mut s, Target::Own).await?;
        sb_link::http1::ask(&mut s, method, path, body, 64 * 1024).await
    };
    let a = tokio::time::timeout(Duration::from_secs(secs), asked)
        .await
        .map_err(|_| "it did not answer".to_string())?
        .map_err(|e| e.to_string())?;
    Ok((a.status, a.json().unwrap_or(Value::Null)))
}

/// Sends a file as the body of `home/receive`.
async fn own_send(tunnel: &Tunnel, path: &str, file: &Path, bytes: u64) -> Result<(u16, Value), String> {
    let sent = async {
        let mut s = tunnel.open().await?;
        sb_link::tunnel::write_header(&mut s, Target::Own).await?;
        let head = format!(
            "POST {path} HTTP/1.1\r\nHost: own\r\nConnection: close\r\nContent-Type: application/octet-stream\r\n\
             Content-Length: {bytes}\r\n\r\n"
        );
        s.write_all(head.as_bytes()).await?;
        let mut f = tokio::fs::File::open(file).await?;
        tokio::io::copy(&mut f, &mut s).await?;
        s.flush().await?;
        let head = sb_link::http1::read_head(&mut s).await?;
        let body = sb_link::http1::read_body(&mut s, &head, 64 * 1024).await?;
        Ok::<_, std::io::Error>((head.status(), serde_json::from_slice(&body).unwrap_or(Value::Null)))
    };
    // (an hour: a home computer's state can be large, and the link slow)
    tokio::time::timeout(Duration::from_secs(3600), sent)
        .await
        .map_err(|_| "it took too long".to_string())?
        .map_err(|e| e.to_string())
}

/// Waits for a tunnel to connect. Err(true): the other side refused this computer; Err(false): not reached.
async fn wait_up(tunnel: &Tunnel, most: Duration) -> Result<(), bool> {
    let end = Instant::now() + most;
    loop {
        let s = tunnel.status();
        if s.connected {
            return Ok(());
        }
        if s.refused {
            return Err(true);
        }
        if Instant::now() >= end {
            return Err(false);
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// An endpoint of its own for one errand, with a tunnel to one computer. It answers nobody.
struct Line {
    ep: Arc<LinkEndpoint>,
    tunnel: Arc<Tunnel>,
    task: JoinHandle<()>,
}

impl Line {
    async fn open(shared: &Shared, key: &DeviceKey, peer: Peer) -> Result<Line, String> {
        // (on a port of its own: the port this computer is known at is where the others look for home)
        let mut options = shared.options(&peer.relay, false, false);
        options.bind = options.bind.map(|a| SocketAddr::new(a.ip(), 0));
        let ep = LinkEndpoint::bind(key, options, Arc::new(Nobody))
            .await
            .map_err(|e| format!("the endpoint could not start ({e})"))?;
        let tunnel = Tunnel::new(ep.clone(), peer);
        let task = tokio::spawn(tunnel.clone().run());
        Ok(Line { ep, tunnel, task })
    }

    async fn close(self) {
        self.tunnel.close();
        self.task.abort();
        self.ep.close().await;
    }
}

/// A tunnel from an endpoint that is running anyway.
struct Borrowed {
    tunnel: Arc<Tunnel>,
    task: JoinHandle<()>,
}

impl Borrowed {
    fn open(ep: &Arc<LinkEndpoint>, peer: Peer) -> Borrowed {
        let tunnel = Tunnel::new(ep.clone(), peer);
        let task = tokio::spawn(tunnel.clone().run());
        Borrowed { tunnel, task }
    }
}

impl Drop for Borrowed {
    fn drop(&mut self) {
        self.tunnel.close();
        self.task.abort();
    }
}

// ---- what is written down ----------------------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TargetRow {
    id: DeviceId,
    name: String,
    #[serde(default)]
    machine: Option<String>,
    #[serde(default)]
    addrs: Vec<SocketAddr>,
    #[serde(default)]
    relay: Option<String>,
}

impl TargetRow {
    fn peer(&self) -> Peer {
        Peer { id: self.id, relay: self.relay.clone(), addrs: self.addrs.clone() }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct MoveJournal {
    v: u32,
    state: String,
    home: DeviceId,
    target: TargetRow,
    retire: bool,
    epoch: u64,
    started: u64,
    #[serde(default)]
    sha256: String,
    #[serde(default)]
    bytes: u64,
    /// When the server and the hub here were stopped, in milliseconds since 1970: where the gap starts.
    #[serde(default)]
    stopped_ms: u64,
    /// The id of the key this computer goes on with.
    #[serde(default)]
    next: Option<DeviceId>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ArriveJournal {
    v: u32,
    /// `committing` until the key is this computer's own, then `keyed`.
    state: String,
    home: DeviceId,
    /// The epoch the list is signed with from here on.
    epoch: u64,
    /// The key this computer had, which the list of devices names and must not any more.
    #[serde(default)]
    was: Option<DeviceId>,
    #[serde(default)]
    old_id: Option<DeviceId>,
    #[serde(default)]
    old_name: String,
    #[serde(default)]
    old_machine: Option<String>,
    #[serde(default)]
    retire: bool,
    /// From a backup, and when.
    #[serde(default)]
    restored: Option<u64>,
}

#[derive(Serialize, Deserialize)]
struct BackupKeyFile {
    v: u32,
    salt: String,
    key: String,
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// What a move is asked to do.
#[derive(Debug, Clone)]
pub enum MoveAsk {
    To { id: DeviceId, retire: bool },
    Resume,
    Undo,
}

// ---- the computer that is asked to take over --------------------------------------------------------------------

/// Answers the `home/` routes of this computer's own surface (own.rs), for the home computer it is paired with.
pub(super) struct Taker {
    pub shared: Weak<Shared>,
    pub home: DeviceId,
}

fn gone() -> own::MoveNo {
    (axum::http::StatusCode::SERVICE_UNAVAILABLE, "failed", "This computer's helper is stopping.".into())
}

fn conflict(r: Refusal) -> own::MoveNo {
    (axum::http::StatusCode::CONFLICT, r.kind, r.text)
}

impl own::Moves for Taker {
    fn home(&self) -> Option<DeviceId> {
        Some(self.home)
    }

    fn can_host(&self) -> bool {
        self.shared.upgrade().is_some_and(|s| s.kit.can_host())
    }

    fn status(&self) -> own::MoveFut<'_> {
        Box::pin(async move {
            let s = self.shared.upgrade().ok_or_else(gone)?;
            let can = s.kit.can_host();
            let st = if can { s.kit.mover.status(&s.kit.folders).await.unwrap_or_default() } else { Staged::default() };
            let state = if st.staged.is_some() { "staged" } else { "idle" };
            Ok(json!({"ok": true, "state": state, "staged": st.staged, "held": st.held, "can_host": can,
                      "version": crate::VERSION}))
        })
    }

    fn receive(&self, sha256: String, body: axum::body::Body) -> own::MoveFut<'_> {
        Box::pin(async move {
            let s = self.shared.upgrade().ok_or_else(gone)?;
            s.take_state(&sha256, body).await.map_err(conflict)
        })
    }

    fn commit(&self, ask: Value) -> own::MoveFut<'_> {
        Box::pin(async move {
            let s = self.shared.upgrade().ok_or_else(gone)?;
            let bad = |why: &str| (axum::http::StatusCode::BAD_REQUEST, "bad_request", why.to_string());
            let staged = ask["staged"].as_str().ok_or_else(|| bad("The request does not say what is staged."))?;
            let epoch = ask["epoch"].as_u64().ok_or_else(|| bad("The request does not say which epoch."))?;
            let key = ask["key"].as_str().and_then(unhex32).ok_or_else(|| bad("The request carries no key."))?;
            let key = DeviceKey::from_bytes(*key);
            if key.id() != self.home {
                // (the caller is the home computer, so this is a mistake and not an attack; still nothing is done)
                return Err((
                    axum::http::StatusCode::FORBIDDEN,
                    "bad_key",
                    "That is not the home computer's key.".into(),
                ));
            }
            let old = &ask["old"];
            let journal = ArriveJournal {
                v: 1,
                state: "committing".into(),
                home: self.home,
                epoch: epoch.saturating_add(1),
                was: None,
                old_id: old["id"].as_str().and_then(|s| s.parse::<DeviceId>().ok()).filter(DeviceId::is_key),
                old_name: old["name"].as_str().unwrap_or("").chars().filter(|c| !c.is_control()).take(64).collect(),
                old_machine: old["machine"].as_str().filter(|m| sb_hub::ticket::valid_machine(m)).map(str::to_string),
                retire: ask["retire"] == true,
                restored: None,
            };
            s.arrive(staged, journal, key).await.map_err(conflict)
        })
    }

    fn discard(&self) -> own::MoveFut<'_> {
        Box::pin(async move {
            let s = self.shared.upgrade().ok_or_else(gone)?;
            let _one =
                s.kit.one.try_lock().map_err(|_| conflict(Refusal::new("busy", "This computer is busy with it.")))?;
            if s.kit.file(ARRIVE_JOURNAL).exists() {
                return Err(conflict(Refusal::new("committed", "This computer has taken over already.")));
            }
            if s.kit.mover.present() {
                s.kit.mover.discard(&s.kit.folders).await.map_err(conflict)?;
            }
            log("keys: the home computer called its move off: what it had sent was thrown away");
            Ok(json!({"ok": true}))
        })
    }
}

impl Shared {
    /// "keys" in the server's settings, as this helper acts on it: a computer that stood down, or that is half
    /// way through taking over, is not home whatever the settings say.
    pub fn mode(&self) -> Mode {
        if stood_down(&self.kit.folders.config).is_some() || self.kit.file(ARRIVE_JOURNAL).exists() {
            return Mode::Off;
        }
        read_mode(&self.places.server_json)
    }

    /// The home computer's state arrives: written to a file of this computer's own, then staged and checked.
    async fn take_state(&self, sha256: &str, body: axum::body::Body) -> Result<Value, Refusal> {
        use futures_util::StreamExt;
        if !self.kit.can_host() {
            return Err(Refusal::new(
                "no_parts",
                NO_PARTS.replace("That computer", "This computer").replace(" there", ""),
            ));
        }
        let _one = self.kit.one.try_lock().map_err(|_| Refusal::new("busy", "This computer is busy with it."))?;
        if self.kit.file(ARRIVE_JOURNAL).exists() {
            return Err(Refusal::new("committed", "This computer has taken over already."));
        }
        let failed = |e: std::io::Error| Refusal::new("failed", format!("The state could not be written here ({e})."));
        let dir = private_dir(&self.kit.folders.state, INCOMING).map_err(failed)?;
        let file = dir.join("state.tar");
        let received = async {
            let mut out = tokio::fs::File::from_std(private_file(&file)?);
            let mut stream = body.into_data_stream();
            let mut n = 0u64;
            while let Some(chunk) = stream.next().await {
                let chunk = chunk.map_err(std::io::Error::other)?;
                n += chunk.len() as u64;
                out.write_all(&chunk).await?;
            }
            out.flush().await?;
            out.sync_all().await?;
            Ok::<u64, std::io::Error>(n)
        }
        .await;
        let staged = match received {
            Ok(_) => self.kit.mover.stage(&self.kit.folders, &file, Some(sha256)).await,
            Err(e) => Err(failed(e)),
        };
        let _ = std::fs::remove_file(&file);
        let p = staged?;
        log(format!(
            "keys: the home computer sent its state ({} bytes). It is staged and checked; nothing here has changed.",
            p.bytes
        ));
        Ok(json!({"ok": true, "staged": p.sha256, "bytes": p.bytes}))
    }

    /// This computer becomes the home computer: everything that has to be on disk is, before the answer goes out.
    async fn arrive(
        self: &Arc<Shared>,
        staged: &str,
        mut journal: ArriveJournal,
        key: DeviceKey,
    ) -> Result<Value, Refusal> {
        let _one = self.kit.one.try_lock().map_err(|_| Refusal::new("busy", "This computer is busy with it."))?;
        let _busy = self.busy.lock().await;
        let st = self.kit.mover.status(&self.kit.folders).await?;
        if st.staged.as_deref() != Some(staged) {
            return Err(Refusal::new(
                "nothing",
                "What the home computer sent is not staged on this computer any more.",
            ));
        }
        if st.held {
            return Err(Refusal::new(
                "held",
                "A Switchboard server is running on this computer. Stop it first: switchboard stop",
            ));
        }
        journal.was = self.runtime().map(|r| r.id).or_else(|| read_key(&self.places.key).map(|k| k.id()));
        let epoch = journal.epoch;
        save(&self.kit.file(ARRIVE_JOURNAL), &journal)?;
        if let Err(e) = put_key(&self.kit.file(ARRIVING_KEY), &key) {
            let _ = std::fs::remove_file(self.kit.file(ARRIVE_JOURNAL));
            return Err(Refusal::new("save", format!("The key could not be saved here ({e}). Nothing was changed.")));
        }
        drop(key);
        self.settle_locked().await?;
        // The answer first; then this helper starts again as the home computer, and the server and the hub with it.
        self.kit.hold_for(Duration::from_millis(400));
        let me = self.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(450)).await;
            // (the port this computer answered on as an ordinary computer can take a moment to come free)
            for _ in 0..20 {
                me.tick().await;
                if me.runtime().is_some_and(|rt| rt.home.is_some()) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
            me.start_parts().await;
        });
        Ok(json!({"ok": true, "epoch": epoch, "id": journal.home.to_string()}))
    }

    async fn start_parts(&self) {
        match self.kit.parts.run("start").await {
            Ok(()) => log("keys: started the server and the hub on this computer"),
            Err(e) => log(format!("keys: the server and the hub did not start ({e}). Start them: switchboard start")),
        }
    }

    /// Finishes taking over, if this computer was stopped half way through it. Called with `busy` held.
    pub(super) async fn settle_locked(&self) -> Result<bool, Refusal> {
        let path = self.kit.file(ARRIVE_JOURNAL);
        let Some(mut j) = load::<ArriveJournal>(&path) else { return Ok(false) };
        let config = &self.kit.folders.config;
        let arriving = self.kit.file(ARRIVING_KEY);
        if j.state == "committing" {
            let Some(key) = read_key(&arriving) else {
                // stopped before the key was on disk: nothing was changed, and the home computer still holds it
                let _ = std::fs::remove_file(&path);
                return Ok(false);
            };
            if key.id() != j.home {
                scrub(config, ARRIVING_KEY);
                let _ = std::fs::remove_file(&path);
                return Ok(false);
            }
            if self.kit.mover.status(&self.kit.folders).await?.staged.is_some() {
                self.kit.mover.swap(&self.kit.folders).await?;
            }
            let no = |e: std::io::Error| Refusal::new("save", format!("The key could not be saved here ({e})."));
            DeviceKey::install(&self.places.key, &key).map_err(no)?;
            j.state = "keyed".into();
            save(&path, &j)?;
        }
        scrub(config, ARRIVING_KEY);
        // this computer is the hub's own machine now, and its link goes to the hub here
        let mut cfg = self.config().unwrap_or_default();
        cfg.home = None;
        cfg.hub = self.ports.hub.to_string();
        crate::save_to(&self.places.config, &cfg)
            .map_err(|e| Refusal::new("save", format!("The settings could not be saved ({e}).")))?;
        let paired = Paired::load(&self.places.paired);
        let listed =
            |e: std::io::Error| Refusal::new("save", format!("The list of paired devices could not be saved ({e})."));
        if let Some(was) = &j.was {
            paired.remove(was).map_err(listed)?;
        }
        paired.remove(&j.home).map_err(listed)?;
        if let (Some(id), false) = (j.old_id, j.retire) {
            let name = if j.old_name.trim().is_empty() {
                "the computer that was home".to_string()
            } else {
                j.old_name.clone()
            };
            paired
                .add(Device {
                    id,
                    name,
                    kind: Kind::Computer,
                    machine: j.old_machine.clone(),
                    paired: unix_now(),
                    via: Via::Upgrade,
                })
                .map_err(listed)?;
        }
        let store = Store::new(&self.places.roster_state).private().bak(Bak::Fresh);
        let mut c = store.load_json::<Counters>().0.unwrap_or_default();
        let held = RosterCache::load(&self.places.roster, j.home).stamp();
        (c.v, c.epoch) = (1, c.epoch.max(j.epoch).max(held.map_or(0, |s| s.epoch + 1)));
        (c.restored, c.restored_epoch, c.adopted) = match j.restored {
            Some(at) => (Some(at), Some(c.epoch), held),
            None => (None, None, None),
        };
        store
            .put_json(&c, true)
            .map_err(|e| Refusal::new("save", format!("The list's numbers could not be saved ({e}).")))?;
        clear_marker(config);
        let _ = std::fs::remove_file(&path);
        log(match (&j.restored, j.old_name.is_empty()) {
            (Some(_), _) => {
                format!("keys: this computer is the home computer now ({}), brought back from a backup", j.home.short())
            }
            (None, true) => format!("keys: this computer is the home computer now ({})", j.home.short()),
            (None, false) => format!(
                "keys: this computer is the home computer now ({}), moved here from {}",
                j.home.short(),
                j.old_name
            ),
        });
        if j.retire
            && let Some(m) = &j.old_machine
        {
            log(format!(
                "keys: {m} was retired with the move: forget it at the hub when it runs (switchboard forget {m})"
            ));
        }
        Ok(true)
    }

    // ---- the computer that is home: what it is shown ---------------------------------------------------------

    /// A list of devices signed with this home computer's own key, shown to it by another device. A higher epoch
    /// than this computer ever used means another computer answers as this home: this one stands down. For
    /// seven days after it was brought back from a backup it takes what such a list says instead (`adopt`).
    pub(super) fn home_sees(self: &Arc<Shared>, envelope: &Envelope) -> Result<bool, RosterError> {
        let Some(rt) = self.runtime() else { return Ok(false) };
        let Some(home) = &rt.home else { return Ok(false) };
        let theirs = roster::verify(envelope, &rt.id, Some(unix_now()))?;
        let store = Store::new(&self.places.roster_state).private().bak(Bak::Fresh);
        let mut c = store.load_json::<Counters>().0.unwrap_or_default();
        let mine = home.held.get().map(|r| r.stamp());
        let my_epoch = c.epoch.max(mine.map_or(0, |s| s.epoch));
        let now = unix_now();
        if c.restored.is_some_and(|t| now < t.saturating_add(roster::HONOURED)) {
            let before = theirs.body.epoch < c.restored_epoch.unwrap_or(0);
            let newer = if before {
                c.adopted.is_none_or(|a| theirs.stamp() > a)
            } else {
                mine.is_none_or(|m| theirs.stamp() > m) && c.adopted.is_none_or(|a| theirs.stamp() > a)
            };
            if !newer {
                return Ok(false);
            }
            self.adopt(&rt, home, &theirs, c.restored);
            c.adopted = Some(theirs.stamp());
            (c.v, c.epoch) = (1, my_epoch.max(theirs.body.epoch.saturating_add(1)));
            if let Err(e) = store.put_json(&c, true) {
                log(format!("keys: {} could not be saved ({e})", self.places.roster_state.display()));
            }
            let (me, r) = (self.clone(), rt.clone());
            tokio::spawn(async move { me.publish(&r).await });
            return Ok(false);
        }
        if theirs.body.epoch > my_epoch {
            self.stand_down(&rt.id, theirs.body.epoch);
        }
        Ok(false)
    }

    /// A list shown by a device this home computer refuses (sb-link endpoint.rs `show`): the only way a list
    /// gets here from the devices of a home computer that replaced this one, since this one knows none of them.
    /// It is looked at as any list shown to this home is, which believes only what this home's own key signed.
    /// The caller gets an answer in one case: this home took its changes from that very list, so the caller is
    /// a device of the other home and can carry the list signed here back to it. Anybody else, a device that
    /// was removed among them, is told nothing.
    pub(super) fn home_shown(self: &Arc<Shared>, bytes: &[u8]) -> Option<Vec<u8>> {
        let envelope: Envelope = serde_json::from_slice(bytes).ok()?;
        self.home_sees(&envelope).ok()?;
        let rt = self.runtime()?;
        let theirs = roster::verify(&envelope, &rt.id, None).ok()?;
        let c = Store::new(&self.places.roster_state).private().bak(Bak::Fresh).load_json::<Counters>().0?;
        if c.adopted != Some(theirs.stamp()) {
            return None;
        }
        let held = rt.home.as_ref()?.held.get()?;
        if held.stamp() <= theirs.stamp() {
            return None; // (the list that says what was taken is not signed yet: the device asks again)
        }
        serde_json::to_vec(&held.envelope).ok()
    }

    /// Takes what a list made after the backup says: devices it no longer names are removed, phones it names
    /// and this home does not know are added. A computer paired after the backup has no link at the hub here,
    /// so it is named in the log and has to be paired again.
    fn adopt(self: &Arc<Shared>, rt: &Arc<Runtime>, home: &HomeSide, theirs: &Roster, restored_at: Option<u64>) {
        log(format!(
            "keys: another device holds a list of devices made after the backup (number {}), so this home computer \
             takes what it says",
            theirs.body.seq
        ));
        for d in home.paired.list() {
            if theirs.device(&d.id).is_some() {
                continue;
            }
            if restored_at.is_some_and(|t| d.paired > t) {
                continue; // paired here after this home computer was brought back: no older list knows it
            }
            if home.paired.remove(&d.id).is_ok() {
                rt.ep.cut(&d.id);
                log(format!("{} (it was removed after the backup was made)", removed_line(Some(&d), &d.id)));
                if let Some(m) = d.machine.clone() {
                    home.removed.lock().unwrap_or_else(|e| e.into_inner()).insert(m.clone());
                    let me = self.clone();
                    tokio::spawn(async move {
                        let _ = me.hub("POST", &format!("/internal/unpair/{m}"), Some(&json!({})), 20).await;
                    });
                }
            }
        }
        rt.ep.sweep();
        for d in theirs.body.devices.iter().filter(|d| d.id != rt.id) {
            if home.paired.get(&d.id).is_some() {
                continue;
            }
            if d.kind == Kind::Phone {
                let device = Device {
                    id: d.id,
                    name: d.name.clone(),
                    kind: d.kind,
                    machine: None,
                    paired: unix_now(),
                    via: Via::Link,
                };
                if home.paired.add(device).is_ok() {
                    log(format!(
                        "keys: {} ({}) was paired after the backup was made, and is paired again",
                        d.name,
                        d.id.short()
                    ));
                }
            } else {
                log(format!(
                    "keys: the computer {} ({}) was paired after the backup was made. Pair it again: switchboard-node \
                     pair <code or link>",
                    d.name,
                    d.id.short()
                ));
            }
        }
    }

    fn stand_down(self: &Arc<Shared>, id: &DeviceId, epoch: u64) {
        let config = &self.kit.folders.config;
        if stood_down(config).is_some() {
            return;
        }
        match write_marker(config, "superseded", id, epoch) {
            Ok(()) => log(format!(
                "keys: {STOOD_DOWN} Another device showed a list of devices signed with this computer's key and a \
                 higher epoch ({epoch}): the home computer was moved, or brought back from a backup, somewhere else."
            )),
            Err(e) => log(format!(
                "keys: this computer should stand down as the home computer, and the note could not be saved ({e})"
            )),
        }
        self.reload.notify_one();
        let me = self.clone();
        tokio::spawn(async move {
            if let Err(e) = me.kit.parts.run("stop").await {
                log(format!(
                    "keys: the server and the hub here could not be stopped ({e}). They do not answer as home while the note is there."
                ));
            }
        });
    }

    /// The home computer asks the computers on its list which list they hold, and so hears of a newer one made
    /// by another holder of its key. Run at the start, and now and then.
    /// True when one of them answered.
    pub(super) async fn ask_witnesses(self: &Arc<Shared>, rt: &Arc<Runtime>) -> bool {
        // a list this computer held as an ordinary computer, kept when it was brought back from a backup
        let witness = self.kit.file(WITNESS);
        if let Some(e) = load::<Envelope>(&witness) {
            let _ = std::fs::remove_file(&witness);
            let _ = self.home_sees(&e);
        }
        let Some(home) = &rt.home else { return true };
        let Some(held) = home.held.get() else { return false };
        let mut heard = false;
        let mut others = 0;
        for d in held.body.devices.iter().filter(|d| d.id != rt.id && d.kind == Kind::Computer) {
            others += 1;
            // (opening it compares the two lists, which is the whole point; the stream itself is not used)
            heard |= matches!(tokio::time::timeout(Duration::from_secs(8), home.witness.open(d)).await, Ok(Ok(_)));
        }
        home.witness.drop_all();
        heard || others == 0
    }

    // ---- the computer that is home: moving ---------------------------------------------------------------------

    fn epoch_now(&self, rt: &Runtime) -> u64 {
        let c = Store::new(&self.places.roster_state)
            .private()
            .bak(Bak::Fresh)
            .load_json::<Counters>()
            .0
            .unwrap_or_default();
        let held = rt.home.as_ref().and_then(|h| h.held.get()).map_or(0, |r| r.body.epoch);
        c.epoch.max(held).max(1)
    }

    /// Moves the home computer, finishes a move that stopped, or calls one off. Ok: what happened, in a line.
    pub async fn home_move(self: &Arc<Shared>, ask: MoveAsk) -> Result<Value, Refusal> {
        let _one = self
            .kit
            .one
            .try_lock()
            .map_err(|_| Refusal::new("busy", "A move, a backup or a restore is under way here."))?;
        let path = self.kit.file(MOVE_JOURNAL);
        let journal = load::<MoveJournal>(&path);
        let r = match (ask, journal) {
            (MoveAsk::To { .. }, Some(j)) => Err(Refusal::new(
                "unfinished",
                format!(
                    "A move to {} was started and did not finish. Finish it: switchboard-node home move --resume. Or \
                     call it off: switchboard-node home move --undo",
                    j.target.name
                ),
            )),
            (MoveAsk::To { id, retire }, None) => self.move_to(id, retire).await,
            (MoveAsk::Resume | MoveAsk::Undo, None) => {
                Err(Refusal::new("nothing", "No move is under way on this computer."))
            }
            (MoveAsk::Resume, Some(j)) => self.move_resume(j).await,
            (MoveAsk::Undo, Some(j)) => self.move_undo(j).await,
        };
        if !self.kit.file(MOVE_JOURNAL).exists() {
            self.kit.frozen.store(false, Ordering::SeqCst);
        }
        self.reload.notify_one();
        r
    }

    async fn move_to(self: &Arc<Shared>, id: DeviceId, retire: bool) -> Result<Value, Refusal> {
        let rt = self.runtime().ok_or_else(|| self.idle())?;
        let home = Shared::home_of(&rt).map_err(|e| Refusal::new("not_home", e))?;
        let device = home.paired.get(&id).filter(|d| d.kind == Kind::Computer).ok_or_else(|| {
            Refusal::new(
                "not_found",
                "No such computer is paired with this home computer. List them: switchboard-node devices",
            )
        })?;
        // where it is: where it connects from now, and where the last list said
        let mut addrs: Vec<SocketAddr> =
            rt.ep.peers().iter().find(|p| p.id == id).and_then(|p| p.remote).into_iter().collect();
        for a in home.held.get().as_ref().and_then(|r| r.device(&id)).map(|d| d.addrs.clone()).unwrap_or_default() {
            if !addrs.contains(&a) {
                addrs.push(a);
            }
        }
        let target = TargetRow {
            id,
            name: device.name.clone(),
            machine: device.machine.clone(),
            addrs,
            relay: rt.relay.0.clone(),
        };
        let journal = MoveJournal {
            v: 1,
            state: "packing".into(),
            home: rt.id,
            target,
            retire,
            epoch: self.epoch_now(&rt),
            started: unix_now(),
            sha256: String::new(),
            bytes: 0,
            stopped_ms: 0,
            next: None,
        };
        self.move_run(rt, journal).await
    }

    /// Steps 1 to 3 with the endpoint that is running as home, then the rest.
    async fn move_run(self: &Arc<Shared>, rt: Arc<Runtime>, mut j: MoveJournal) -> Result<Value, Refusal> {
        let path = self.kit.file(MOVE_JOURNAL);
        let name = j.target.name.clone();
        let line = Borrowed::open(&rt.ep, j.target.peer());
        if wait_up(&line.tunnel, Duration::from_secs(15)).await.is_err() {
            return Err(Refusal::new(
                "unreachable",
                format!("{name} cannot be reached from here right now. Nothing was changed."),
            ));
        }
        let (status, v) = own_ask(&line.tunnel, "GET", "/own/v1/home/status", None, 20)
            .await
            .map_err(|e| Refusal::new("unreachable", format!("{name} did not answer ({e}). Nothing was changed.")))?;
        if status == 404 {
            return Err(Refusal::new(
                "version",
                format!("{name} needs an update before it can become the home computer. Nothing was changed."),
            ));
        }
        if status != 200 || v["ok"] != true {
            return Err(Refusal::new(
                "refused",
                format!(
                    "{name} did not agree ({}). Nothing was changed.",
                    v["error"].as_str().unwrap_or("no reason given")
                ),
            ));
        }
        if v["can_host"] != true {
            return Err(Refusal::new("no_parts", format!("{NO_PARTS} Nothing was changed.")));
        }
        if v["held"] == true {
            return Err(Refusal::new(
                "held",
                format!(
                    "A Switchboard server is running on {name}. Stop it there first: switchboard stop. Nothing was changed."
                ),
            ));
        }
        save(&path, &j)?;
        self.kit.frozen.store(true, Ordering::SeqCst);
        log(format!(
            "keys: moving the home computer to {name} ({}). Pairing and removing wait until it is done.",
            j.target.id.short()
        ));

        // From here to the commit nothing is the home computer's server: this is where the gap starts.
        j.stopped_ms = now_ms();
        let back = |me: Arc<Shared>, why: Refusal| async move {
            // nothing irreversible has happened: put this computer back as it was
            let _ = std::fs::remove_file(me.kit.file(MOVE_JOURNAL));
            let _ = std::fs::remove_dir_all(me.kit.folders.state.join(OUTGOING));
            me.kit.frozen.store(false, Ordering::SeqCst);
            me.start_parts().await;
            Err::<Value, Refusal>(Refusal::new(
                why.kind,
                format!("{} The move was called off, and nothing was changed.", why.text),
            ))
        };
        if let Err(e) = self.kit.parts.run("stop").await {
            log(format!("keys: stopping the server and the hub said: {e}"));
        }
        let mut quiet = false;
        for _ in 0..150 {
            match self.kit.mover.status(&self.kit.folders).await {
                Ok(st) if !st.held => {
                    quiet = true;
                    break;
                }
                Ok(_) => tokio::time::sleep(Duration::from_millis(200)).await,
                Err(e) => return back(self.clone(), e).await,
            }
        }
        if !quiet {
            return back(self.clone(), Refusal::new("held", "The server on this computer did not stop.")).await;
        }
        let out = match private_dir(&self.kit.folders.state, OUTGOING) {
            Ok(d) => d.join("state.tar"),
            Err(e) => {
                return back(self.clone(), Refusal::new("failed", format!("The state could not be packed ({e}).")))
                    .await;
            }
        };
        let packed = match self.kit.mover.pack(&self.kit.folders, &out).await {
            Ok(p) => p,
            Err(e) => return back(self.clone(), e).await,
        };
        (j.state, j.sha256, j.bytes) = ("packed".into(), packed.sha256.clone(), packed.bytes);
        save(&path, &j)?;
        if self.kit.stops_at("packed") {
            return Err(Refusal::new("stopped", "stopped by the test after packing"));
        }
        let sent = own_send(&line.tunnel, &format!("/own/v1/home/receive?sha256={}", j.sha256), &out, j.bytes).await;
        let _ = std::fs::remove_dir_all(self.kit.folders.state.join(OUTGOING));
        match sent {
            Ok((200, v)) if v["ok"] == true && v["staged"] == j.sha256.as_str() => {}
            Ok((_, v)) => {
                let why = v["error"].as_str().unwrap_or("it gave no reason").to_string();
                return back(self.clone(), Refusal::new("refused", format!("{name} did not take the state ({why}).")))
                    .await;
            }
            Err(e) => {
                return back(
                    self.clone(),
                    Refusal::new("unreachable", format!("The state did not reach {name} ({e}).")),
                )
                .await;
            }
        }
        j.state = "sent".into();
        save(&path, &j)?;
        drop(line);

        // Step down, durably, before the key goes anywhere.
        if !j.retire {
            let next = DeviceKey::generate();
            put_key(&self.kit.file(NEXT_KEY), &next).map_err(|e| {
                Refusal::new(
                    "save",
                    format!(
                        "This computer's next key could not be saved ({e}). Run: switchboard-node home move --undo"
                    ),
                )
            })?;
            j.next = Some(next.id());
        }
        write_marker(&self.kit.folders.config, "moving", &j.home, j.epoch)
            .map_err(|e| Refusal::new("save", format!("The note that this computer stepped down could not be saved ({e}). Run: switchboard-node home move --undo")))?;
        j.state = "stepped_down".into();
        save(&path, &j)?;
        self.tick().await; // the note is there, so this stops the endpoint that answered as home
        if self.runtime().is_some_and(|r| r.home.is_some()) {
            return Err(Refusal::new(
                "failed",
                "This computer could not stop answering as home. Run: switchboard-node home move --undo",
            ));
        }
        log(format!("keys: {STOOD_DOWN} The key goes to {name} now."));
        if self.kit.stops_at("stepped_down") {
            return Err(Refusal::new("stopped", "stopped by the test after stepping down"));
        }
        self.move_commit(j).await
    }

    /// Step 4: the key, sent with the key itself as the caller.
    async fn move_commit(self: &Arc<Shared>, mut j: MoveJournal) -> Result<Value, Refusal> {
        let path = self.kit.file(MOVE_JOURNAL);
        let key = read_key(&self.places.key).filter(|k| k.id() == j.home).ok_or_else(|| {
            Refusal::new(
                "key",
                "The home computer's key is not on this computer any more, so the move cannot be finished from here.",
            )
        })?;
        let line = Line::open(self, &key, j.target.peer())
            .await
            .map_err(|e| Refusal::new("endpoint", format!("{NOBODY_HOME} ({e})")))?;
        let done = async {
            if let Err(refused) = wait_up(&line.tunnel, Duration::from_secs(20)).await {
                return Err(if refused { "refused" } else { "unreachable" });
            }
            let cfg = self.config().unwrap_or_default();
            let old = json!({"id": j.next.map(|n| n.to_string()), "name": crate::hostname_label(),
                             "machine": Some(cfg.machine).filter(|m| !m.is_empty())});
            let body = Zeroizing::new(
                json!({"staged": j.sha256, "epoch": j.epoch, "key": hex(key.secret_bytes()).as_str(), "old": old, "retire": j.retire})
                    .to_string()
                    .into_bytes(),
            );
            j.state = "committing".into();
            save(&path, &j).map_err(|_| "save")?;
            if self.kit.stops_at("committing") {
                return Err("stopped");
            }
            match own_ask(&line.tunnel, "POST", "/own/v1/home/commit", Some(&body), 120).await {
                Ok((200, v)) if v["ok"] == true => Ok(()),
                Ok((_, v)) => {
                    log(format!("keys: the other computer did not take over ({})", v["error"].as_str().unwrap_or("no reason given")));
                    Err("refused")
                }
                Err(_) => Err("no_answer"),
            }
        }
        .await;
        line.close().await;
        drop(key);
        match done {
            Ok(()) => {
                if self.kit.stops_at("committed") {
                    return Err(Refusal::new("stopped", "stopped by the test after the commit was answered"));
                }
                self.move_finish(j).await
            }
            Err(kind) => Err(Refusal::new(kind, NOBODY_HOME)),
        }
    }

    /// Step 5: this computer stops holding the home key and goes on as an ordinary computer, or as none.
    async fn move_finish(self: &Arc<Shared>, mut j: MoveJournal) -> Result<Value, Refusal> {
        let path = self.kit.file(MOVE_JOURNAL);
        let config = self.kit.folders.config.clone();
        j.state = "committed".into();
        save(&path, &j)?;
        let gap_ms = if j.stopped_ms > 0 { now_ms().saturating_sub(j.stopped_ms) } else { 0 };
        {
            let _busy = self.busy.lock().await;
            self.stop().await;
            let key_dir = self.places.key.parent().map(Path::to_path_buf).unwrap_or_default();
            let mut cfg = self.config().unwrap_or_default();
            match (j.retire, read_key(&self.kit.file(NEXT_KEY))) {
                (false, Some(next)) => {
                    DeviceKey::install(&self.places.key, &next)
                        .map_err(|e| Refusal::new("save", format!("This computer's new key could not be saved ({e}). Run: switchboard-node home move --resume")))?;
                    cfg.home = Some(HomeConfig {
                        id: j.home,
                        relay: j.target.relay.clone(),
                        name: j.target.name.clone(),
                        addrs: j.target.addrs.clone(),
                    });
                    cfg.hub = String::new();
                }
                (false, None) if read_key(&self.places.key).is_some_and(|k| k.id() != j.home) => {
                    // (finished up to here before: the new key is in place already)
                    cfg.home = Some(HomeConfig {
                        id: j.home,
                        relay: j.target.relay.clone(),
                        name: j.target.name.clone(),
                        addrs: j.target.addrs.clone(),
                    });
                    cfg.hub = String::new();
                }
                _ => {
                    // retired, or no next key to go on with: this computer holds no key at all
                    for name in ["device.key", "device.key.bak"] {
                        if self.places.key.file_name().is_some_and(|n| n == "device.key") {
                            scrub(&key_dir, name);
                        }
                    }
                    cfg.home = None;
                    cfg.hub = String::new();
                    cfg.token = String::new();
                }
            }
            scrub(&config, NEXT_KEY);
            // (what seals the home computer's backups went with it: this computer is not the one to hold it)
            scrub(&config, BACKUP_KEY);
            crate::save_to(&self.places.config, &cfg).map_err(|e| {
                Refusal::new(
                    "save",
                    format!("The settings could not be saved ({e}). Run: switchboard-node home move --resume"),
                )
            })?;
            let _ = write_marker(&config, "moved", &j.home, j.epoch);
            let _ = std::fs::remove_dir_all(self.kit.folders.state.join(OUTGOING));
            let _ = std::fs::remove_file(&path);
        }
        self.kit.frozen.store(false, Ordering::SeqCst);
        log(format!(
            "keys: {} is the home computer now. {}",
            j.target.name,
            if j.retire {
                "This computer was retired: it holds no key."
            } else {
                "This computer goes on as one of its computers."
            }
        ));
        Ok(json!({"ok": true, "state": "done", "home": {"id": j.home.to_string(), "name": j.target.name},
                  "retired": j.retire, "gap_ms": gap_ms, "epoch": j.epoch + 1}))
    }

    async fn move_resume(self: &Arc<Shared>, j: MoveJournal) -> Result<Value, Refusal> {
        self.kit.frozen.store(true, Ordering::SeqCst);
        match j.state.as_str() {
            "packing" | "packed" | "sent" => {
                // nothing irreversible happened: tidy up and go again from the top
                let _ = std::fs::remove_file(self.kit.file(MOVE_JOURNAL));
                let _ = std::fs::remove_dir_all(self.kit.folders.state.join(OUTGOING));
                self.start_parts().await;
                self.tick().await;
                let rt = self.runtime().ok_or_else(|| self.idle())?;
                let fresh = MoveJournal {
                    state: "packing".into(),
                    epoch: self.epoch_now(&rt),
                    sha256: String::new(),
                    bytes: 0,
                    stopped_ms: 0,
                    next: None,
                    ..j
                };
                self.move_run(rt, fresh).await
            }
            "committed" => self.move_finish(j).await,
            _ => {
                // Stepped down. Did the other computer take over? If somebody answers as home, it did. (Asked
                // first, and with a key that is not the home's: if it did, the home's key is never used from
                // here again.) Else it answers as the computer it was, and is asked to take over now.
                if self.took_over(&j).await {
                    return self.move_finish(j).await;
                }
                if let Some(key) = read_key(&self.places.key).filter(|k| k.id() == j.home)
                    && let Ok(line) = Line::open(self, &key, j.target.peer()).await
                {
                    let up = wait_up(&line.tunnel, Duration::from_secs(8)).await;
                    let st = match up {
                        Ok(()) => own_ask(&line.tunnel, "GET", "/own/v1/home/status", None, 15).await.ok(),
                        Err(_) => None,
                    };
                    line.close().await;
                    drop(key);
                    if let Some((200, v)) = st {
                        if v["staged"] == j.sha256.as_str() {
                            return self.move_commit(j).await;
                        }
                        return Err(Refusal::new(
                            "nothing",
                            format!(
                                "{} no longer holds the state this computer sent, so it cannot take over. Call the \
                                 move off, then start again: switchboard-node home move --undo",
                                j.target.name
                            ),
                        ));
                    }
                }
                Err(Refusal::new("unreachable", NOBODY_HOME))
            }
        }
    }

    /// Whether somebody else answers as this home now. Asked with a key that is not the home's: an answer, or a
    /// refusal, from the home's id can only come from the computer the key was sent to.
    async fn took_over(&self, j: &MoveJournal) -> bool {
        let key = read_key(&self.kit.file(NEXT_KEY)).unwrap_or_else(DeviceKey::generate);
        if key.id() == j.home {
            return false;
        }
        let peer = Peer { id: j.home, relay: j.target.relay.clone(), addrs: j.target.addrs.clone() };
        let Ok(line) = Line::open(self, &key, peer).await else { return false };
        let up = wait_up(&line.tunnel, Duration::from_secs(10)).await;
        line.close().await;
        matches!(up, Ok(()) | Err(true))
    }

    async fn move_undo(self: &Arc<Shared>, j: MoveJournal) -> Result<Value, Refusal> {
        let name = j.target.name.clone();
        let stepped = !matches!(j.state.as_str(), "packing" | "packed" | "sent");
        if j.state == "committed" {
            return Err(Refusal::new(
                "committed",
                format!(
                    "{name} is the home computer now, so there is nothing to call off. Finish here: switchboard-node home move --resume"
                ),
            ));
        }
        // The other computer is told to throw the state away. Once the key may have been sent, its answer is
        // the only thing that says the move can be called off.
        let key = read_key(&self.places.key).filter(|k| k.id() == j.home);
        let mut discarded = false;
        if let Some(key) = &key {
            let line = match (stepped, self.runtime().filter(|r| r.home.is_some())) {
                (false, Some(rt)) => {
                    let b = Borrowed::open(&rt.ep, j.target.peer());
                    if wait_up(&b.tunnel, Duration::from_secs(8)).await.is_ok() {
                        discarded = matches!(
                            own_ask(&b.tunnel, "POST", "/own/v1/home/discard", Some(b"{}"), 20).await,
                            Ok((200, _))
                        );
                    }
                    None
                }
                _ => Line::open(self, key, j.target.peer()).await.ok(),
            };
            if let Some(line) = line {
                if wait_up(&line.tunnel, Duration::from_secs(8)).await.is_ok() {
                    discarded = matches!(
                        own_ask(&line.tunnel, "POST", "/own/v1/home/discard", Some(b"{}"), 20).await,
                        Ok((200, _))
                    );
                }
                line.close().await;
            }
        }
        if j.state == "committing" && !discarded {
            return Err(Refusal::new(
                "unsure",
                format!(
                    "The key was on its way to {name} when the move stopped, and {name} does not say that it has not \
                     taken over. Calling the move off now could leave two home computers, so nothing was changed. \
                     When {name} can be reached, run: switchboard-node home move --resume"
                ),
            ));
        }
        if key.is_none() {
            return Err(Refusal::new(
                "key",
                "The home computer's key is not on this computer any more, so the move cannot be called off from here.",
            ));
        }
        scrub(&self.kit.folders.config, NEXT_KEY);
        let _ = std::fs::remove_dir_all(self.kit.folders.state.join(OUTGOING));
        clear_marker(&self.kit.folders.config);
        let _ = std::fs::remove_file(self.kit.file(MOVE_JOURNAL));
        self.kit.frozen.store(false, Ordering::SeqCst);
        self.start_parts().await;
        log(format!("keys: the move to {name} was called off. This computer is the home computer, as before."));
        Ok(json!({"ok": true, "state": "undone", "told": discarded}))
    }

    // ---- a copy --------------------------------------------------------------------------------------------------

    /// What seals this home computer's backups, and the recovery code when this call made it.
    async fn sealer(&self) -> Result<(Sealer, Option<RecoveryCode>), Refusal> {
        let path = self.kit.file(BACKUP_KEY);
        if let Some(f) = load::<BackupKeyFile>(&path)
            && let Some(s) = Sealer::from_stored(&f.salt, &f.key)
        {
            return Ok((s, None));
        }
        let code = RecoveryCode::generate();
        let c = code.clone();
        let sealer = tokio::task::spawn_blocking(move || Sealer::new(&c))
            .await
            .map_err(|e| Refusal::new("internal", e.to_string()))?;
        let (salt, key) = sealer.to_stored();
        save(&path, &BackupKeyFile { v: 1, salt, key: key.to_string() })?;
        Ok((sealer, Some(code)))
    }

    /// Writes a sealed copy of this home computer to `to`, or to its backup folder. The answer carries the
    /// recovery code only when this call made it: it is shown once and kept nowhere.
    pub async fn home_backup(self: &Arc<Shared>, to: Option<PathBuf>) -> Result<Value, Refusal> {
        let rt = self.runtime().ok_or_else(|| self.idle())?;
        Shared::home_of(&rt).map_err(|e| Refusal::new("not_home", e))?;
        let _one = self
            .kit
            .one
            .try_lock()
            .map_err(|_| Refusal::new("busy", "A move, a backup or a restore is under way here."))?;
        if !self.kit.mover.present() {
            return Err(Refusal::new(
                "no_parts",
                "The tool that packs the home computer's state is not on this computer.",
            ));
        }
        let key = read_key(&self.places.key)
            .filter(|k| k.id() == rt.id)
            .ok_or_else(|| Refusal::new("key", "This computer's key cannot be read."))?;
        let failed = |e: std::io::Error| Refusal::new("failed", format!("The backup could not be written ({e})."));
        let out = match to {
            Some(p) => p,
            None => private_dir(&self.kit.folders.state, BACKUPS).map_err(failed)?.join(format!(
                "{}-{}.sbk",
                rt.id.short(),
                unix_now()
            )),
        };
        let tar = private_dir(&self.kit.folders.state, OUTGOING).map_err(failed)?.join("backup.tar");
        let (sealer, code) = self.sealer().await?;
        let packed = self.kit.mover.pack(&self.kit.folders, &tar).await;
        let about = backup::About {
            home: rt.id.to_string(),
            epoch: self.epoch_now(&rt),
            made: unix_now(),
            version: crate::VERSION.to_string(),
        };
        let epoch = about.epoch;
        let sealed = match packed {
            Err(e) => Err(e),
            Ok(_) => {
                let (tar, out) = (tar.clone(), out.clone());
                tokio::task::spawn_blocking(move || {
                    let part = out.with_extension("sbk.part");
                    let done = (|| {
                        let mut from = std::io::BufReader::new(std::fs::File::open(&tar)?);
                        let mut to = std::io::BufWriter::new(private_file(&part)?);
                        backup::seal(&sealer, &about, &mut from, key.secret_bytes(), &mut to)
                            .map_err(std::io::Error::other)?;
                        let f = to.into_inner().map_err(|e| e.into_error())?;
                        f.sync_all()?;
                        std::fs::rename(&part, &out)?;
                        std::fs::metadata(&out).map(|m| m.len())
                    })();
                    if done.is_err() {
                        let _ = std::fs::remove_file(&part);
                    }
                    done
                })
                .await
                .map_err(|e| Refusal::new("internal", e.to_string()))
                .and_then(|r| r.map_err(failed))
            }
        };
        let _ = std::fs::remove_dir_all(self.kit.folders.state.join(OUTGOING));
        let bytes = sealed?;
        log(format!("keys: a backup of this home computer was written to {}", out.display()));
        Ok(json!({"ok": true, "file": out.display().to_string(), "bytes": bytes, "epoch": epoch,
                  "code": code.map(|c| c.to_string())}))
    }

    /// Makes this computer the home computer from a backup and its recovery code.
    pub async fn home_restore(self: &Arc<Shared>, file: &Path, code: &str) -> Result<Value, Refusal> {
        let code = RecoveryCode::parse(code)
            .ok_or_else(|| Refusal::new("bad_code", "That is not a recovery code. It is 26 letters and digits, as it was shown when the first backup was made."))?;
        let _one = self
            .kit
            .one
            .try_lock()
            .map_err(|_| Refusal::new("busy", "A move, a backup or a restore is under way here."))?;
        if !self.kit.can_host() {
            return Err(Refusal::new(
                "no_parts",
                NO_PARTS.replace("That computer", "This computer").replace(" there", ""),
            ));
        }
        let of = |e: BackupError| {
            let kind = match &e {
                BackupError::NotBackup => "not_backup",
                BackupError::Newer => "newer",
                BackupError::WrongCode => "wrong_code",
                BackupError::Damaged => "damaged",
                BackupError::Io(_) => "failed",
            };
            Refusal::new(kind, e.to_string())
        };
        let (header, _) = std::fs::File::open(file)
            .map_err(BackupError::Io)
            .and_then(|mut f| backup::read_header(&mut f))
            .map_err(of)?;
        let home: DeviceId =
            header.home.parse().ok().filter(DeviceId::is_key).ok_or_else(|| of(BackupError::Damaged))?;
        let _busy = self.busy.lock().await;
        if read_mode(&self.places.server_json) != Mode::Off && stood_down(&self.kit.folders.config).is_none() {
            return Err(Refusal::new(
                "is_home",
                "This computer is a home computer already. A backup is restored on a computer that is not one.",
            ));
        }
        let cfg = self.config().unwrap_or_default();
        if let Some(h) = cfg.home.as_ref().filter(|h| h.id != home) {
            return Err(other_home(h));
        }
        if self.kit.mover.status(&self.kit.folders).await?.held {
            return Err(Refusal::new(
                "held",
                "A Switchboard server is running on this computer. Stop it first: switchboard stop",
            ));
        }
        // what this computer held of the home's list, as one of its computers: usually newer than the backup
        let witness = cfg.home.as_ref().and_then(|_| RosterCache::load(&self.places.roster, home).get());
        let failed =
            |e: std::io::Error| Refusal::new("failed", format!("The backup could not be unpacked here ({e})."));
        let tar = private_dir(&self.kit.folders.state, INCOMING).map_err(failed)?.join("restore.tar");
        let (from, to) = (file.to_path_buf(), tar.clone());
        let opened = tokio::task::spawn_blocking(move || {
            let mut input = std::io::BufReader::new(std::fs::File::open(&from)?);
            let mut out = std::io::BufWriter::new(private_file(&to)?);
            let r = backup::open(&code, &mut input, &mut out)?;
            out.into_inner().map_err(|e| BackupError::Io(e.into_error()))?.sync_all()?;
            Ok::<_, BackupError>(r)
        })
        .await
        .map_err(|e| Refusal::new("internal", e.to_string()))?;
        let (header, key) = match opened {
            Ok(r) => r,
            Err(e) => {
                let _ = std::fs::remove_file(&tar);
                return Err(of(e));
            }
        };
        let key = DeviceKey::from_bytes(*key);
        if key.id() != home {
            let _ = std::fs::remove_file(&tar);
            return Err(of(BackupError::Damaged));
        }
        let staged = self.kit.mover.stage(&self.kit.folders, &tar, None).await;
        let _ = std::fs::remove_file(&tar);
        staged?;
        // From here on this computer is on its way to being home. What follows is the second half of a move.
        self.stop().await;
        if !self.take_lock() {
            let _ = self.kit.mover.discard(&self.kit.folders).await;
            return Err(Refusal::new(
                "key_in_use",
                "Another switchboard-node on this computer is using its key. Stop it and try again.",
            ));
        }
        let journal = ArriveJournal {
            v: 1,
            state: "committing".into(),
            home,
            epoch: header.epoch.max(witness.as_ref().map_or(0, |w| w.body.epoch)).saturating_add(1),
            was: read_key(&self.places.key).map(|k| k.id()),
            old_id: None,
            old_name: String::new(),
            old_machine: None,
            retire: false,
            restored: Some(unix_now()),
        };
        let epoch = journal.epoch;
        save(&self.kit.file(ARRIVE_JOURNAL), &journal)?;
        if let Err(e) = put_key(&self.kit.file(ARRIVING_KEY), &key) {
            let _ = std::fs::remove_file(self.kit.file(ARRIVE_JOURNAL));
            let _ = self.kit.mover.discard(&self.kit.folders).await;
            return Err(Refusal::new("save", format!("The key could not be saved here ({e}). Nothing was changed.")));
        }
        drop(key);
        if let Some(w) = &witness {
            let _ = save(&self.kit.file(WITNESS), &w.envelope);
        }
        self.settle_locked().await?;
        let devices = Paired::load(&self.places.paired).list();
        drop(_busy);
        self.start_parts().await;
        self.reload.notify_one();
        Ok(json!({
            "ok": true, "home": home.to_string(), "epoch": epoch, "made": header.made, "version": header.version,
            "devices": devices.iter().map(|d| json!({"name": d.name, "kind": d.kind, "machine": d.machine})).collect::<Vec<_>>(),
        }))
    }

    /// Where this computer stands: home or not, a move under way, a note that it stood down.
    pub fn home_status(&self) -> Value {
        let note = stood_down(&self.kit.folders.config);
        let journal = load::<MoveJournal>(&self.kit.file(MOVE_JOURNAL));
        let rt = self.runtime();
        let role = match (&rt, self.role()) {
            (Some(r), _) if r.home.is_some() => "home",
            (_, Role::Member(_)) => "member",
            (_, Role::Home) => "home",
            _ => "off",
        };
        json!({
            "ok": true, "role": role, "can_host": self.kit.can_host(),
            "epoch": rt.as_ref().filter(|r| r.home.is_some()).map(|r| self.epoch_now(r)),
            "stood_down": note.as_ref().map(|n| json!({"why": n.why, "at": n.at, "line": stood_down_line(n)})),
            "move": journal.map(|j| json!({"state": j.state, "to": j.target.name, "started": j.started, "retire": j.retire})),
            "arriving": self.kit.file(ARRIVE_JOURNAL).exists(),
            "backup": {"code_made": self.kit.file(BACKUP_KEY).exists()},
        })
    }

    /// A computer that stood down holds the home's key in its key file until it is paired again: it must never
    /// use it. Called before this computer's key is used to pair with a home computer.
    pub(super) fn drop_home_key(&self) {
        let Some(note) = stood_down(&self.kit.folders.config) else { return };
        if read_key(&self.places.key).is_some_and(|k| k.id().to_string() == note.home)
            && DeviceKey::replace(&self.places.key).is_ok()
        {
            log("keys: this computer was a home computer, so it made a new key of its own before pairing");
        }
    }
}
