//! Updates (docs/architecture.md, Updates). The download page has latest.json, the list of builds, and
//! latest.json.sig, an Ed25519 signature of its exact bytes. The app gets both from the server (GET /api/app) or,
//! when the build names one, straight from a feed URL. It checks the signature with the public key compiled in
//! (SB_UPDATE_PUBKEY, see build.rs), offers a build only when it is newer in version and build, downloads it on a
//! click (resuming a .partial file), and checks its size and SHA-256 from the signed list. A build made without a
//! key never offers updates. Nothing here installs or restarts anything; sb-desktop's update.rs does, when the user
//! says so, and the rollback rules for that are the pure functions at the bottom.

use std::cmp::Ordering as Cmp;
use std::collections::HashMap;
use std::io::{Read, Seek, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use base64::Engine;
use ed25519_dalek::{Signature, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::net::CoreError;

/// What GET /api/app answers. `latest` is the parsed list, kept for older apps; this one reads only `feed`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ApiApp {
    #[serde(default)]
    pub latest: Option<Latest>,
    /// The download page ("http://host:port/app").
    #[serde(default)]
    pub base: Option<String>,
    /// latest.json as the page has it, byte for byte, and its signature.
    #[serde(default)]
    pub feed: Option<Feed>,
    /// The release's page, for What's new (the server sends the GitHub release it read).
    #[serde(default)]
    pub page: Option<String>,
    /// Why the server couldn't look (GitHub or the download page didn't answer).
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Feed {
    /// The text of latest.json, exactly as signed.
    pub manifest: String,
    /// latest.json.sig: the base64 Ed25519 signature of `manifest`.
    #[serde(default)]
    pub sig: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Latest {
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub built: Option<f64>,
    #[serde(default)]
    pub files: HashMap<String, LatestFile>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LatestFile {
    pub file: String,
    #[serde(default)]
    pub sha256: String,
    /// The build id (UTC yyyymmddHHMMSS) the build scripts stamp into the app as SB_BUILD.
    #[serde(default)]
    pub build: String,
    /// Bytes.
    #[serde(default)]
    pub size: u64,
}

/// A newer build, ready to download.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Offer {
    /// The version running now, for "from 0.8.4 to 0.9.0".
    pub from: String,
    pub version: String,
    pub build: String,
    pub built: Option<f64>,
    pub url: String,
    pub sha256: String,
    pub size: u64,
    pub file: String,
    /// The download page, for "see what's new" and for platforms that install by hand.
    pub page: String,
}

/// "mac", "windows" or "linux": the key in latest.json's files.
pub fn platform() -> &'static str {
    if cfg!(target_os = "macos") {
        "mac"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else {
        "linux"
    }
}

/// This build's id, when the build scripts stamped one (a development build has none and is never offered updates).
pub fn this_build() -> Option<&'static str> {
    option_env!("SB_BUILD").filter(|b| !b.is_empty())
}

/// The public key the feed must be signed with (base64, 32 bytes; build.rs checked it).
const PUBKEY: Option<&str> = option_env!("SB_UPDATE_PUBKEY");
/// A feed URL from the build environment. Without one the app asks its server where the download page is.
const FEED: Option<&str> = option_env!("SB_UPDATE_FEED");

/// The compiled-in key, if this build has one.
pub fn key() -> Option<VerifyingKey> {
    PUBKEY.filter(|k| !k.is_empty()).and_then(parse_key)
}

/// The feed URL from the build environment, if any.
pub fn feed_url() -> Option<&'static str> {
    FEED.filter(|f| !f.is_empty())
}

/// A base64 public key, as update-key.sh prints it.
pub fn parse_key(b64: &str) -> Option<VerifyingKey> {
    let raw = base64::engine::general_purpose::STANDARD.decode(b64.trim()).ok()?;
    VerifyingKey::from_bytes(&raw.try_into().ok()?).ok()
}

/// Why this build never offers updates, in words for Settings, or None when it does.
pub fn off_reason() -> Option<&'static str> {
    if key().is_none() {
        Some(
            "Updates are off in this build. It was made without an update key, so it can't check that a new version is genuine.",
        )
    } else if this_build().is_none() {
        Some("Updates are off in this build. It's a development build, with no build number to compare.")
    } else {
        None
    }
}

/// Checks `sig` (base64) against `manifest` and reads the list. Anything that does not check out is an error, so a
/// tampered or unsigned list is never offered.
pub fn verify(key: &VerifyingKey, manifest: &str, sig: Option<&str>) -> Result<Latest, String> {
    let sig = sig.map(str::trim).filter(|s| !s.is_empty()).ok_or("The update list isn't signed.")?;
    let raw =
        base64::engine::general_purpose::STANDARD.decode(sig).map_err(|_| "The update signature is unreadable.")?;
    let sig = Signature::from_slice(&raw).map_err(|_| "The update signature is unreadable.")?;
    key.verify_strict(manifest.as_bytes(), &sig).map_err(|_| "The update list's signature doesn't match.")?;
    serde_json::from_str(manifest).map_err(|e| format!("The update list is unreadable: {e}"))
}

/// A semantic version: 1.2.3, 1.2.3-rc.1 (build metadata after "+" is ignored, as semver says).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ver {
    pub nums: [u64; 3],
    pub pre: Vec<String>,
}

impl Ver {
    pub fn parse(s: &str) -> Option<Ver> {
        let s = s.trim().trim_start_matches('v');
        let s = s.split('+').next()?;
        let (core, pre) = match s.split_once('-') {
            Some((c, p)) => (c, p.split('.').map(str::to_string).collect::<Vec<_>>()),
            None => (s, Vec::new()),
        };
        let mut it = core.split('.');
        let mut nums = [0u64; 3];
        for n in &mut nums {
            *n = it.next()?.parse().ok()?;
        }
        if it.next().is_some() || pre.iter().any(|p| p.is_empty()) {
            return None;
        }
        Some(Ver { nums, pre })
    }
}

impl PartialOrd for Ver {
    fn partial_cmp(&self, o: &Ver) -> Option<Cmp> {
        Some(self.cmp(o))
    }
}

impl Ord for Ver {
    fn cmp(&self, o: &Ver) -> Cmp {
        self.nums.cmp(&o.nums).then_with(|| match (self.pre.is_empty(), o.pre.is_empty()) {
            (true, true) => Cmp::Equal,
            (true, false) => Cmp::Greater, // 1.0.0 is after 1.0.0-rc.1
            (false, true) => Cmp::Less,
            (false, false) => {
                for (a, b) in self.pre.iter().zip(&o.pre) {
                    let c = match (a.parse::<u64>(), b.parse::<u64>()) {
                        (Ok(x), Ok(y)) => x.cmp(&y),
                        (Ok(_), Err(_)) => Cmp::Less,
                        (Err(_), Ok(_)) => Cmp::Greater,
                        (Err(_), Err(_)) => a.cmp(b),
                    };
                    if c != Cmp::Equal {
                        return c;
                    }
                }
                self.pre.len().cmp(&o.pre.len())
            }
        })
    }
}

/// A build id: 14 digits, UTC yyyymmddHHMMSS, so a plain comparison orders them.
fn build_id(b: &str) -> bool {
    b.len() == 14 && b.bytes().all(|c| c.is_ascii_digit())
}

/// Whether `(ver, build)` is newer than what runs now. The version may not go back, and the build must move
/// forward, so neither an older version with a newer build nor the same build again is ever installed.
pub fn is_newer(mine_ver: &str, mine_build: &str, ver: &str, build: &str) -> bool {
    let (Some(a), Some(b)) = (Ver::parse(mine_ver), Ver::parse(ver)) else { return false };
    build_id(mine_build) && build_id(build) && b >= a && build > mine_build
}

/// The offer for `plat` in a signed list, when it is newer than this build and not the one that was rolled back.
pub fn offer_from(
    latest: &Latest,
    base: &str,
    plat: &str,
    mine_ver: &str,
    mine_build: &str,
    bad_build: Option<&str>,
) -> Option<Offer> {
    let f = latest.files.get(plat)?;
    let base = base.trim_end_matches('/');
    let plain = |n: &str| {
        !n.is_empty() && !n.contains(['/', '\\']) && !n.starts_with('.') && n.bytes().all(|c| c.is_ascii_graphic())
    };
    if !is_newer(mine_ver, mine_build, &latest.version, &f.build)
        || bad_build == Some(f.build.as_str())
        || f.sha256.len() != 64
        || !f.sha256.bytes().all(|c| c.is_ascii_hexdigit())
        || f.size == 0
        || !plain(&f.file)
        || base.is_empty()
    {
        return None;
    }
    Some(Offer {
        from: mine_ver.to_string(),
        version: latest.version.clone(),
        build: f.build.clone(),
        built: latest.built,
        url: format!("{base}/{}", f.file),
        sha256: f.sha256.to_ascii_lowercase(),
        size: f.size,
        file: f.file.clone(),
        page: format!("{base}/"),
    })
}

/// The offer in a GET /api/app answer: the signature first, then the version rules.
pub fn offer(
    app: &ApiApp,
    key: &VerifyingKey,
    plat: &str,
    mine_build: &str,
    bad_build: Option<&str>,
) -> Result<Option<Offer>, CoreError> {
    let (Some(feed), Some(base)) = (app.feed.as_ref(), app.base.as_deref()) else {
        return match &app.error {
            Some(e) => Err(CoreError::Bad(e.clone())),
            None => Ok(None),
        };
    };
    let latest = verify(key, &feed.manifest, feed.sig.as_deref()).map_err(CoreError::Bad)?;
    let page = app.page.as_ref().filter(|p| p.starts_with("https://") && !p.contains(char::is_whitespace));
    Ok(offer_from(&latest, base, plat, crate::version(), mine_build, bad_build).map(|o| match page {
        Some(p) => Offer { page: p.clone(), ..o },
        None => o,
    }))
}

/// A newer server kit in the signed list: what Settings > Server offers to install on the server.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ServerOffer {
    pub version: String,
    pub sha256: String,
    pub size: u64,
    /// The release's page, for What's new.
    pub page: Option<String>,
}

/// The server kit in a GET /api/app answer, checked against the signature, when it is newer than `server_ver` (or
/// the server can't say its version). None when the list has no kit (releases before 0.8.11).
pub fn server_offer(app: &ApiApp, key: &VerifyingKey, server_ver: &str) -> Result<Option<ServerOffer>, CoreError> {
    let Some(feed) = app.feed.as_ref() else {
        return match &app.error {
            Some(e) => Err(CoreError::Bad(e.clone())),
            None => Ok(None),
        };
    };
    let latest = verify(key, &feed.manifest, feed.sig.as_deref()).map_err(CoreError::Bad)?;
    Ok(server_offer_from(&latest, server_ver, app.page.as_deref()))
}

pub fn server_offer_from(latest: &Latest, server_ver: &str, page: Option<&str>) -> Option<ServerOffer> {
    let f = latest.files.get("server")?;
    let new = Ver::parse(&latest.version)?;
    if Ver::parse(server_ver).is_some_and(|v| v >= new)
        || f.sha256.len() != 64
        || !f.sha256.bytes().all(|c| c.is_ascii_hexdigit())
    {
        return None;
    }
    Some(ServerOffer {
        version: latest.version.clone(),
        sha256: f.sha256.to_ascii_lowercase(),
        size: f.size,
        page: page.filter(|p| p.starts_with("https://") && !p.contains(char::is_whitespace)).map(str::to_string),
    })
}

fn agent(user_agent: &str, secs: u64) -> ureq::Agent {
    let cfg = ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(secs)))
        .timeout_connect(Some(std::time::Duration::from_secs(8)))
        .http_status_as_error(false)
        .user_agent(user_agent)
        .build();
    ureq::Agent::new_with_config(cfg)
}

/// Reads latest.json and latest.json.sig straight from a feed URL (a build with SB_UPDATE_FEED), in the shape the
/// server's /api/app has.
pub fn fetch_feed(base: &str, user_agent: &str) -> Result<ApiApp, CoreError> {
    let base = base.trim_end_matches('/');
    let a = agent(user_agent, 20);
    let get = |name: &str| -> Result<Option<String>, CoreError> {
        let mut r = a.get(&format!("{base}/{name}")).call().map_err(|_| CoreError::Unreachable)?;
        match r.status().as_u16() {
            200 => {
                r.body_mut().with_config().limit(200_000).read_to_string().map(Some).map_err(|_| CoreError::Unreachable)
            }
            404 => Ok(None),
            s => Err(CoreError::Bad(format!("The update feed answered {s}."))),
        }
    };
    let Some(manifest) = get("latest.json")? else { return Ok(ApiApp::default()) };
    let sig = get("latest.json.sig")?;
    Ok(ApiApp { base: Some(base.to_string()), feed: Some(Feed { manifest, sig }), ..Default::default() })
}

/// Whether the file at `path` is the offer's: its size, then its SHA-256.
pub fn matches(o: &Offer, path: &Path) -> std::io::Result<bool> {
    let mut f = std::fs::File::open(path)?;
    if f.metadata()?.len() != o.size {
        return Ok(false);
    }
    let mut h = Sha256::new();
    std::io::copy(&mut f, &mut h)?;
    Ok(hex(&h.finalize()) == o.sha256)
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|b| format!("{b:02x}")).collect()
}

/// The file a download of `o` collects into. Its name carries the build's SHA-256, so a stopped download of one build
/// is never resumed with another build's bytes.
fn partial(o: &Offer, dir: &Path) -> PathBuf {
    dir.join(format!("{}.{}.partial", o.file, o.sha256.get(..16).unwrap_or(&o.sha256)))
}

/// Downloads the offer into `dir`, counting bytes into `got` (and the size into `total`). A download that stops
/// part way stays in `<file>.<hash>.partial` and the next try asks only for the rest (HTTP Range); a page that ignores
/// Range sends it all again. The finished file must have the signed size and SHA-256, or it is deleted.
pub fn download(
    o: &Offer,
    dir: &Path,
    got: &AtomicU64,
    total: &AtomicU64,
    user_agent: &str,
) -> Result<PathBuf, CoreError> {
    let save = |e: std::io::Error| CoreError::Bad(format!("Couldn't save the update: {e}"));
    let unreachable = || CoreError::Bad("Couldn't reach the download page. Try again later.".into());
    let mismatch =
        || CoreError::Bad("The download didn't match the signed list of files, so it wasn't installed.".into());
    total.store(o.size, Ordering::Relaxed);
    std::fs::create_dir_all(dir).map_err(save)?;
    let path = dir.join(&o.file);
    if path.is_file() {
        if matches(o, &path).map_err(save)? {
            got.store(o.size, Ordering::Relaxed);
            return Ok(path);
        }
        let _ = std::fs::remove_file(&path);
    }
    let part = partial(o, dir);
    // What other builds left part way is of no use now.
    if let Ok(entries) = std::fs::read_dir(dir) {
        let (start, own) = (format!("{}.", o.file), part.file_name());
        for e in entries.flatten() {
            let name = e.file_name();
            let n = name.to_string_lossy();
            if n.starts_with(&start) && n.ends_with(".partial") && Some(name.as_os_str()) != own {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
    let mut have = std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
    if have > o.size {
        let _ = std::fs::remove_file(&part);
        have = 0;
    }
    let a = agent(user_agent, 900);
    // Two tries at most: the second starts over when the page refused the range.
    for _ in 0..2 {
        if have == o.size {
            break;
        }
        let mut req = a.get(&o.url);
        if have > 0 {
            req = req.header("Range", &format!("bytes={have}-"));
        }
        let mut r = req.call().map_err(|_| unreachable())?;
        let resumed = match r.status().as_u16() {
            206 => {
                let from = r.headers().get("content-range").and_then(|v| v.to_str().ok()).and_then(range_start);
                if from != Some(have) {
                    return Err(CoreError::Bad("The download page sent the wrong part of the file.".into()));
                }
                true
            }
            200 => false,
            416 if have > 0 => {
                let _ = std::fs::remove_file(&part);
                have = 0;
                continue;
            }
            s => return Err(CoreError::Bad(format!("The download page answered {s}."))),
        };
        let mut out = std::fs::OpenOptions::new().create(true).write(true).truncate(false).open(&part).map_err(save)?;
        if !resumed {
            out.set_len(0).map_err(save)?;
            have = 0;
        }
        out.seek(std::io::SeekFrom::Start(have)).map_err(save)?;
        got.store(have, Ordering::Relaxed);
        // One byte past the size is enough to know the file is too long.
        let mut body = r.body_mut().with_config().limit(o.size - have + 1).reader();
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            let n = match body.read(&mut buf) {
                Ok(n) => n,
                Err(_) => {
                    let _ = out.flush();
                    return Err(unreachable()); // what came is kept for the next try
                }
            };
            if n == 0 {
                break;
            }
            if have + n as u64 > o.size {
                drop(out);
                let _ = std::fs::remove_file(&part);
                return Err(mismatch());
            }
            out.write_all(&buf[..n]).map_err(save)?;
            have += n as u64;
            got.store(have, Ordering::Relaxed);
        }
        out.flush().map_err(save)?;
        break;
    }
    if have != o.size || !matches(o, &part).map_err(save)? {
        // A short file is kept to resume; a full one that does not match is not.
        if have >= o.size {
            let _ = std::fs::remove_file(&part);
            return Err(mismatch());
        }
        return Err(unreachable());
    }
    std::fs::rename(&part, &path).map_err(save)?;
    Ok(path)
}

/// "bytes 100-199/200" -> 100.
fn range_start(v: &str) -> Option<u64> {
    v.trim().strip_prefix("bytes ")?.split('-').next()?.trim().parse().ok()
}

// ---- Checks: once a little after launch, then once a day, one at a time.

/// The first check after launch.
pub const SOON: f64 = 90.;
/// Then once a day.
pub const EVERY: f64 = 24. * 3600.;

/// Whether to look for a newer build now. `last` is when the last check began (None: not yet since launch).
pub fn check_due(now: f64, started: f64, last: Option<f64>, busy: bool) -> bool {
    !busy
        && match last {
            None => now - started >= SOON,
            Some(t) => now - t >= EVERY,
        }
}

// ---- Rollback: the old build is kept until the new one has started once and reported healthy.

/// update.json in the settings folder: an install waiting to prove itself, and the build that was rolled back.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct UpdState {
    #[serde(default)]
    pub pending: Option<Pending>,
    /// A build that failed its first run; it is not offered again.
    #[serde(default)]
    pub bad_build: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Pending {
    /// What was replaced: the .app bundle on a Mac, the program elsewhere.
    pub target: PathBuf,
    /// The previous build, kept beside it.
    pub old: PathBuf,
    pub from_build: String,
    pub to_build: String,
    #[serde(default)]
    pub from_version: String,
    #[serde(default)]
    pub to_version: String,
    /// Times the new build has started without reporting healthy.
    #[serde(default)]
    pub starts: u32,
}

/// What a starting app does about a pending install.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Launch {
    /// Nothing pending.
    Nothing,
    /// The new build's first start: count it, and report healthy once it has run a while.
    Trial,
    /// The new build started before and never reported healthy (it crashed or would not start): put the old one
    /// back and open it.
    RollBack,
    /// Forget the record and keep whatever is there (the old build is running, or there is nothing to go back to).
    Forget,
    /// Something else was installed since: the kept old build is stale, so delete it and forget the record.
    DropOld,
}

pub fn on_launch(p: Option<&Pending>, running: Option<&str>, old_exists: bool) -> Launch {
    let Some(p) = p else { return Launch::Nothing };
    match running {
        Some(b) if b == p.to_build => match (p.starts, old_exists) {
            (0, _) => Launch::Trial,
            (_, true) => Launch::RollBack,
            (_, false) => Launch::Forget,
        },
        Some(b) if b == p.from_build => Launch::Forget,
        _ if old_exists => Launch::DropOld,
        _ => Launch::Forget,
    }
}

/// When the running build reports healthy: true when it is the pending one, so the old copy can go.
pub fn healthy_drops_old(p: Option<&Pending>, running: Option<&str>) -> bool {
    p.is_some_and(|p| running == Some(p.to_build.as_str()))
}

/// Where update.json lives.
pub fn state_path() -> Option<PathBuf> {
    crate::cache::settings_dir().map(|d| d.join("update.json"))
}

pub fn load_state(path: &Path) -> UpdState {
    std::fs::read(path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

pub fn save_state(path: &Path, s: &UpdState) -> std::io::Result<()> {
    if *s == UpdState::default() {
        return match std::fs::remove_file(path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        };
    }
    crate::cache::write_atomic(path, &serde_json::to_vec_pretty(s).unwrap_or_default())
}

// ---- The swap. Only the app itself, a copy of the old one beside it and update.json are touched: never the
// settings, the cache or anything the first run set up.

/// What is being replaced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A Mac .app bundle (a folder): swapped with the new one in one rename.
    Bundle,
    /// The Windows program, which can be renamed but not overwritten while it runs.
    Exe,
    /// The Linux binary: the new one is renamed over it.
    Bin,
}

pub fn kind() -> Kind {
    if cfg!(target_os = "macos") {
        Kind::Bundle
    } else if cfg!(windows) {
        Kind::Exe
    } else {
        Kind::Bin
    }
}

/// Where the old build is kept until the new one reports healthy: .Switchboard-old.app beside the bundle,
/// Switchboard.old.exe, Switchboard.old.
pub fn old_path(kind: Kind, target: &Path) -> PathBuf {
    match kind {
        Kind::Bundle => {
            let name = target.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            target.with_file_name(format!(".{name}-old.app"))
        }
        Kind::Exe => target.with_extension("old.exe"),
        Kind::Bin => target.with_extension("old"),
    }
}

/// Deletes a file or folder, if it is there.
pub fn remove_any(p: &Path) {
    if p.is_dir() && !p.is_symlink() {
        let _ = std::fs::remove_dir_all(p);
    } else {
        let _ = std::fs::remove_file(p);
    }
}

/// Swaps two paths in one step (macOS renamex_np with RENAME_SWAP). Elsewhere, and on file systems that can't,
/// it answers Unsupported and the caller uses two renames.
fn exchange(a: &Path, b: &Path) -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    {
        use std::os::unix::ffi::OsStrExt;
        unsafe extern "C" {
            fn renamex_np(from: *const std::ffi::c_char, to: *const std::ffi::c_char, flags: u32) -> i32;
        }
        const RENAME_SWAP: u32 = 0x2;
        let ca = std::ffi::CString::new(a.as_os_str().as_bytes())?;
        let cb = std::ffi::CString::new(b.as_os_str().as_bytes())?;
        // SAFETY: two NUL-terminated paths that live for the call.
        if unsafe { renamex_np(ca.as_ptr(), cb.as_ptr(), RENAME_SWAP) } == 0 {
            return Ok(());
        }
        Err(std::io::Error::last_os_error())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (a, b);
        Err(std::io::ErrorKind::Unsupported.into())
    }
}

/// Puts `new` at `target` and keeps the old build at `old`, so that either both moves happen or neither does. A copy
/// an earlier update left at `old` is set aside first and deleted only once the new build is in place, so a swap
/// that fails keeps it.
fn put_in(kind: Kind, new: &Path, target: &Path, old: &Path) -> std::io::Result<()> {
    let mut aside = old.as_os_str().to_owned();
    aside.push(".prev");
    let aside = PathBuf::from(aside);
    if aside.exists() && !old.exists() {
        // A swap that stopped half way left it there.
        let _ = std::fs::rename(&aside, old);
    }
    remove_any(&aside);
    if old.exists() && std::fs::rename(old, &aside).is_err() {
        remove_any(old);
    }
    let res = move_in(kind, new, target, old);
    if res.is_ok() {
        remove_any(&aside);
    } else if aside.exists() && !old.exists() {
        let _ = std::fs::rename(&aside, old);
    }
    res
}

/// The moves of `put_in`, with nothing at `old` yet.
fn move_in(kind: Kind, new: &Path, target: &Path, old: &Path) -> std::io::Result<()> {
    match kind {
        Kind::Bundle => {
            if exchange(new, target).is_ok() {
                // `new` now holds the old bundle. If it can't be renamed, keep it there.
                return match std::fs::rename(new, old) {
                    Ok(()) => Ok(()),
                    Err(e) => {
                        let _ = exchange(new, target);
                        Err(e)
                    }
                };
            }
            std::fs::rename(target, old)?;
            std::fs::rename(new, target).inspect_err(|_| {
                let _ = std::fs::rename(old, target);
            })
        }
        Kind::Exe => {
            std::fs::rename(target, old)?;
            std::fs::rename(new, target).inspect_err(|_| {
                let _ = std::fs::rename(old, target);
            })
        }
        Kind::Bin => {
            // A second name for the running binary (or a copy), then one rename over it.
            if std::fs::hard_link(target, old).is_err() {
                std::fs::copy(target, old)?;
            }
            std::fs::rename(new, target).inspect_err(|_| remove_any(old))
        }
    }
}

/// Installs `new` (unpacked and checked) in place of `target` and records it in update.json at `state`, so the
/// next start can tell whether it worked. If the record can't be written the swap is undone, so there is never an
/// install that could not be rolled back.
pub fn swap_in(kind: Kind, new: &Path, target: &Path, state: &Path, o: &Offer, from_build: &str) -> Result<(), String> {
    let old = old_path(kind, target);
    put_in(kind, new, target, &old).map_err(|e| format!("Couldn't put the new version in place: {e}"))?;
    let p = Pending {
        target: target.to_path_buf(),
        old,
        from_build: from_build.to_string(),
        to_build: o.build.clone(),
        from_version: o.from.clone(),
        to_version: o.version.clone(),
        starts: 0,
    };
    let mut st = load_state(state);
    st.pending = Some(p.clone());
    if let Err(e) = save_state(state, &st) {
        let _ = put_back(kind, &p);
        return Err(format!("Couldn't save a note about the update, so it was undone: {e}"));
    }
    Ok(())
}

/// Puts the old build back where the new one is. The new one is deleted, except a running Windows program, which
/// is left as Switchboard.failed.exe for the next start to sweep.
pub fn put_back(kind: Kind, p: &Pending) -> Result<(), String> {
    let err = |e: std::io::Error| format!("Couldn't put the previous version back: {e}");
    if !p.old.exists() {
        return Err("The previous version isn't there anymore.".into());
    }
    match kind {
        Kind::Bundle => {
            if exchange(&p.old, &p.target).is_ok() {
                remove_any(&p.old);
                return Ok(());
            }
            let failed = p.target.with_extension("failed");
            remove_any(&failed);
            std::fs::rename(&p.target, &failed).map_err(err)?;
            std::fs::rename(&p.old, &p.target)
                .inspect_err(|_| {
                    let _ = std::fs::rename(&failed, &p.target);
                })
                .map_err(err)?;
            remove_any(&failed);
        }
        Kind::Exe => {
            let failed = p.target.with_extension("failed.exe");
            remove_any(&failed);
            std::fs::rename(&p.target, &failed).map_err(err)?;
            std::fs::rename(&p.old, &p.target)
                .inspect_err(|_| {
                    let _ = std::fs::rename(&failed, &p.target);
                })
                .map_err(err)?;
        }
        Kind::Bin => std::fs::rename(&p.old, &p.target).map_err(err)?,
    }
    Ok(())
}

/// What the app does with update.json as it starts, before any window: count the new build's first start, put the
/// old build back after a start that never reported healthy, or tidy up. Answers the app to open instead of this
/// one when it rolled back.
pub fn at_launch(kind: Kind, state: &Path, running: Option<&str>) -> Option<PathBuf> {
    let mut st = load_state(state);
    let p = st.pending.clone()?;
    match on_launch(Some(&p), running, p.old.exists()) {
        Launch::Nothing => {}
        Launch::Trial => st.pending = Some(Pending { starts: p.starts + 1, ..p }),
        Launch::RollBack => {
            st.pending = None;
            match put_back(kind, &p) {
                Ok(()) => {
                    log::warn!("update: {} did not start cleanly, so {} is back", p.to_version, p.from_version);
                    st.bad_build = Some(p.to_build.clone());
                    let _ = save_state(state, &st);
                    return Some(p.target);
                }
                Err(e) => log::error!("update: {e}"),
            }
        }
        Launch::Forget => st.pending = None,
        Launch::DropOld => {
            remove_any(&p.old);
            st.pending = None;
        }
    }
    let _ = save_state(state, &st);
    None
}

/// The new build has run a while (or quit cleanly): the old copy can go.
pub fn report_healthy(state: &Path, running: Option<&str>) {
    let mut st = load_state(state);
    if !healthy_drops_old(st.pending.as_ref(), running) {
        return;
    }
    if let Some(p) = st.pending.take() {
        remove_any(&p.old);
    }
    let _ = save_state(state, &st);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    use std::io::BufRead;
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    fn signer() -> SigningKey {
        SigningKey::from_bytes(&[7u8; 32])
    }

    fn b64(b: &[u8]) -> String {
        base64::engine::general_purpose::STANDARD.encode(b)
    }

    fn manifest(build: &str, version: &str) -> String {
        format!(
            r#"{{"version": "{version}", "built": 1.0, "files": {{"mac": {{"file": "Switchboard-mac.zip", "sha256": "{}", "build": "{build}", "size": 10}}}}}}"#,
            "AB".repeat(32)
        )
    }

    #[test]
    fn a_signed_list_is_read_and_anything_else_is_refused() {
        let k = signer();
        let m = manifest("20261013120000", "0.9.0");
        let sig = b64(&k.sign(m.as_bytes()).to_bytes());
        let latest = verify(&k.verifying_key(), &m, Some(&sig)).unwrap();
        assert_eq!(latest.version, "0.9.0");
        assert_eq!(latest.files["mac"].size, 10);

        assert!(verify(&k.verifying_key(), &m, None).is_err(), "unsigned");
        assert!(verify(&k.verifying_key(), &m, Some("not base64!")).is_err(), "garbage");
        let tampered = m.replace("0.9.0", "9.9.9");
        assert!(verify(&k.verifying_key(), &tampered, Some(&sig)).is_err(), "a changed list");
        let other = SigningKey::from_bytes(&[8u8; 32]);
        let theirs = b64(&other.sign(m.as_bytes()).to_bytes());
        assert!(verify(&k.verifying_key(), &m, Some(&theirs)).is_err(), "someone else's key");
        // The key as update-key.sh prints it.
        assert_eq!(parse_key(&b64(k.verifying_key().as_bytes())), Some(k.verifying_key()));
        assert!(parse_key("AAAA").is_none());
    }

    #[test]
    fn the_server_kit_is_offered_only_when_signed_and_newer() {
        let k = signer();
        let m = format!(
            r#"{{"version": "0.8.11", "files": {{"server": {{"file": "switchboard-server-kit.tar.gz", "sha256": "{}", "size": 9}}}}}}"#,
            "CD".repeat(32)
        );
        let sig = b64(&k.sign(m.as_bytes()).to_bytes());
        let app = ApiApp {
            feed: Some(Feed { manifest: m.clone(), sig: Some(sig.clone()) }),
            page: Some("https://github.com/acme/board/releases/tag/v0.8.11".into()),
            ..Default::default()
        };
        let o = server_offer(&app, &k.verifying_key(), "0.8.10").unwrap().unwrap();
        assert_eq!((o.version.as_str(), o.sha256.as_str()), ("0.8.11", "cd".repeat(32).as_str()));
        assert!(o.page.is_some());
        assert!(server_offer(&app, &k.verifying_key(), "0.8.11").unwrap().is_none(), "already there");
        assert!(server_offer(&app, &k.verifying_key(), "").unwrap().is_some(), "a server that can't say");
        let forged =
            ApiApp { feed: Some(Feed { manifest: m.replace("0.8.11", "0.9.0"), sig: Some(sig) }), ..app.clone() };
        assert!(server_offer(&forged, &k.verifying_key(), "0.8.10").is_err(), "the signature covers the kit");
        let none = manifest("20261013120000", "0.9.0");
        let sig = b64(&k.sign(none.as_bytes()).to_bytes());
        let old = ApiApp { feed: Some(Feed { manifest: none, sig: Some(sig) }), ..Default::default() };
        assert!(server_offer(&old, &k.verifying_key(), "0.8.10").unwrap().is_none(), "a list without a kit");
    }

    #[test]
    fn a_list_signed_by_publish_sh_checks_out() {
        // Made with a throwaway key the way update-key.sh and publish_render.py sign (OpenSSL 3).
        let pubkey = "bzsRWowIVYPmMWiumtwIf+qTIgmTqvyQObx+yEdMAuU=";
        let sig = "jXDqs9GRrVEqPN7z8qdfEePzUvz4UcaMmwyEqMvAtJaKwAtAJ0Ck1wbPeeAOoQmqpO9uM23RX3lVqlX4Bo/aDQ==\n";
        let m = "{\"version\": \"0.9.0\", \"files\": {}}\n";
        let k = parse_key(pubkey).unwrap();
        assert_eq!(verify(&k, m, Some(sig)).unwrap().version, "0.9.0");
        assert!(verify(&k, &m.replace("0.9.0", "0.9.1"), Some(sig)).is_err());
    }

    #[test]
    fn versions_order_like_semver() {
        let v = |s| Ver::parse(s).unwrap();
        assert!(v("0.9.0") > v("0.8.4"));
        assert!(v("0.10.0") > v("0.9.9"), "numbers, not text");
        assert!(v("1.0.0") > v("1.0.0-rc.2"));
        assert!(v("1.0.0-rc.10") > v("1.0.0-rc.2"));
        assert!(v("1.0.0-rc.1") > v("1.0.0-beta"));
        assert!(v("1.0.0-alpha.1") > v("1.0.0-alpha"));
        assert_eq!(v("1.2.3+abc"), v("1.2.3"));
        assert!(Ver::parse("1.2").is_none() && Ver::parse("1.2.3.4").is_none() && Ver::parse("x").is_none());
    }

    #[test]
    fn only_a_newer_version_and_build_is_offered() {
        assert!(is_newer("0.8.4", "20261012090000", "0.9.0", "20261013120000"));
        assert!(is_newer("0.8.4", "20261012090000", "0.8.4", "20261013120000"), "a rebuild of the same version");
        assert!(!is_newer("0.8.4", "20261013120000", "0.8.4", "20261013120000"), "the same build");
        assert!(!is_newer("0.8.4", "20261013120000", "0.9.0", "20261011120000"), "an older build");
        assert!(!is_newer("0.9.0", "20261012090000", "0.8.4", "20261013120000"), "an older version, newer build");
        assert!(!is_newer("0.8.4", "", "0.9.0", "20261013120000"), "no build id here");
        assert!(!is_newer("0.8.4", "20261012090000", "0.9.0", ""), "no build id there");
        assert!(!is_newer("0.8.4", "20261012090000", "nine", "20261013120000"));
    }

    #[test]
    fn the_offer_follows_the_signed_list() {
        let latest: Latest = serde_json::from_str(&manifest("20261013120000", "0.9.0")).unwrap();
        let base = "http://10.0.0.5:8131/app/";
        let o = offer_from(&latest, base, "mac", "0.8.4", "20261012090000", None).unwrap();
        assert_eq!(o.url, "http://10.0.0.5:8131/app/Switchboard-mac.zip");
        assert_eq!(o.sha256, "ab".repeat(32));
        assert_eq!((o.from.as_str(), o.version.as_str(), o.size), ("0.8.4", "0.9.0", 10));
        assert!(offer_from(&latest, base, "linux", "0.8.4", "20261012090000", None).is_none(), "nothing for linux");
        assert!(offer_from(&latest, base, "mac", "0.9.0", "20261013120000", None).is_none(), "the same build");
        assert!(
            offer_from(&latest, base, "mac", "0.8.4", "20261012090000", Some("20261013120000")).is_none(),
            "the build that was rolled back"
        );
        let mut bad = latest.clone();
        bad.files.get_mut("mac").unwrap().file = "../evil".into();
        assert!(offer_from(&bad, base, "mac", "0.8.4", "20261012090000", None).is_none(), "a path in the name");

        // Through /api/app: the signature decides first.
        let k = signer();
        let m = manifest("20261013120000", "99.0.0");
        let app = |sig: Option<String>| ApiApp {
            latest: None,
            base: Some(base.into()),
            feed: Some(Feed { manifest: m.clone(), sig }),
            ..Default::default()
        };
        let good = app(Some(b64(&k.sign(m.as_bytes()).to_bytes())));
        assert!(offer(&good, &k.verifying_key(), "mac", "20261012090000", None).unwrap().is_some());
        assert!(offer(&app(None), &k.verifying_key(), "mac", "20261012090000", None).is_err());
        assert_eq!(offer(&ApiApp::default(), &k.verifying_key(), "mac", "20261012090000", None).unwrap(), None);

        // From GitHub: What's new opens the release page; a look that failed says why instead of "up to date".
        let gh = ApiApp { page: Some("https://github.com/acme/board/releases/tag/v99.0.0".into()), ..good.clone() };
        let o = offer(&gh, &k.verifying_key(), "mac", "20261012090000", None).unwrap().unwrap();
        assert_eq!(o.page, "https://github.com/acme/board/releases/tag/v99.0.0");
        let odd = ApiApp { page: Some("javascript:alert(1)".into()), ..good.clone() };
        assert_eq!(offer(&odd, &k.verifying_key(), "mac", "20261012090000", None).unwrap().unwrap().page, base);
        let down = ApiApp { error: Some("GitHub didn't answer (timed out).".into()), ..Default::default() };
        let e = offer(&down, &k.verifying_key(), "mac", "20261012090000", None).unwrap_err();
        assert_eq!(e.to_string(), "GitHub didn't answer (timed out).");
    }

    #[test]
    fn checks_run_soon_after_launch_then_daily_and_one_at_a_time() {
        assert!(!check_due(100., 100., None, false), "not at launch");
        assert!(check_due(100. + SOON, 100., None, false), "a little after");
        assert!(!check_due(100. + SOON, 100., None, true), "never two at once");
        assert!(!check_due(5000., 0., Some(4000.), false), "not again the same day");
        assert!(check_due(4000. + EVERY, 0., Some(4000.), false), "a day later");
    }

    #[test]
    fn rollback_decisions() {
        let p = Pending {
            target: "/A/Switchboard.app".into(),
            old: "/A/.Switchboard-old.app".into(),
            from_build: "20261012090000".into(),
            to_build: "20261013120000".into(),
            from_version: "0.8.4".into(),
            to_version: "0.9.0".into(),
            starts: 0,
        };
        let new = Some("20261013120000");
        let old = Some("20261012090000");
        assert_eq!(on_launch(None, new, true), Launch::Nothing);
        assert_eq!(on_launch(Some(&p), new, true), Launch::Trial, "the first start of the new build");
        let crashed = Pending { starts: 1, ..p.clone() };
        assert_eq!(on_launch(Some(&crashed), new, true), Launch::RollBack, "started before, never healthy");
        assert_eq!(on_launch(Some(&crashed), new, false), Launch::Forget, "nothing to go back to");
        assert_eq!(on_launch(Some(&p), old, true), Launch::Forget, "the old build is the one running");
        assert_eq!(on_launch(Some(&p), Some("20261020000000"), true), Launch::DropOld, "installed by hand since");
        assert_eq!(on_launch(Some(&p), None, false), Launch::Forget);
        assert!(healthy_drops_old(Some(&p), new));
        assert!(!healthy_drops_old(Some(&p), old));
        assert!(!healthy_drops_old(None, new));
    }

    #[test]
    fn state_file_round_trips_and_an_empty_one_is_removed() {
        let dir = std::env::temp_dir().join(format!("sb-upd-state-{}", std::process::id()));
        let path = dir.join("update.json");
        let s = UpdState { pending: None, bad_build: Some("20261013120000".into()) };
        save_state(&path, &s).unwrap();
        assert_eq!(load_state(&path), s);
        save_state(&path, &UpdState::default()).unwrap();
        assert!(!path.exists());
        assert_eq!(load_state(&path), UpdState::default());
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---- Downloads against a tiny local page.

    /// Serves `body` for every request; honours Range unless `ranges` is false. Records each Range header.
    fn page(body: Vec<u8>, ranges: bool, cut_after: Option<usize>) -> (String, Arc<Mutex<Vec<Option<String>>>>) {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/Switchboard-mac.zip", l.local_addr().unwrap());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let seen2 = seen.clone();
        std::thread::spawn(move || {
            for (i, s) in l.incoming().enumerate() {
                let Ok(mut s) = s else { continue };
                let mut r = std::io::BufReader::new(s.try_clone().unwrap());
                let mut range = None;
                loop {
                    let mut line = String::new();
                    if r.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                        break;
                    }
                    if let Some(v) = line.to_ascii_lowercase().strip_prefix("range: bytes=") {
                        range = Some(v.trim().trim_end_matches('-').to_string());
                    }
                }
                seen2.lock().unwrap().push(range.clone());
                let from = range.filter(|_| ranges).and_then(|v| v.parse::<usize>().ok());
                let (head, part) = match from {
                    Some(f) => (
                        format!(
                            "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {f}-{}/{}\r\nContent-Length: {}\r\n",
                            body.len() - 1,
                            body.len(),
                            body.len() - f
                        ),
                        &body[f..],
                    ),
                    None => (format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n", body.len()), &body[..]),
                };
                let _ = s.write_all(format!("{head}Connection: close\r\n\r\n").as_bytes());
                // The first answer stops part way when asked to, like a dropped connection.
                let part = match cut_after {
                    Some(n) if i == 0 => &part[..n.min(part.len())],
                    _ => part,
                };
                let _ = s.write_all(part);
            }
        });
        (url, seen)
    }

    fn offer_for(body: &[u8], url: String) -> Offer {
        Offer {
            from: "0.8.4".into(),
            version: "0.9.0".into(),
            build: "20261013120000".into(),
            built: None,
            url,
            sha256: hex(&Sha256::digest(body)),
            size: body.len() as u64,
            file: "Switchboard-mac.zip".into(),
            page: String::new(),
        }
    }

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("sb-upd-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    fn body() -> Vec<u8> {
        (0..300_000u32).map(|i| (i % 251) as u8).collect()
    }

    #[test]
    fn a_download_that_stops_resumes_from_the_partial_file() {
        let b = body();
        let (url, seen) = page(b.clone(), true, Some(100_000));
        let o = offer_for(&b, url);
        let dir = tmp("resume");
        let (got, total) = (AtomicU64::new(0), AtomicU64::new(0));
        assert!(download(&o, &dir, &got, &total, "t").is_err(), "the connection dropped");
        let part = partial(&o, &dir);
        assert_eq!(std::fs::metadata(&part).unwrap().len(), 100_000, "what came is kept");
        let file = download(&o, &dir, &got, &total, "t").unwrap();
        assert_eq!(std::fs::read(&file).unwrap(), b);
        assert!(!part.exists());
        assert_eq!(seen.lock().unwrap().as_slice(), &[None, Some("100000".into())], "asked only for the rest");
        assert_eq!((got.load(Ordering::Relaxed), total.load(Ordering::Relaxed)), (b.len() as u64, b.len() as u64));
        // Done already: no second download.
        download(&o, &dir, &got, &total, "t").unwrap();
        assert_eq!(seen.lock().unwrap().len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_page_without_range_sends_it_all_again() {
        let b = body();
        let (url, seen) = page(b.clone(), false, None);
        let o = offer_for(&b, url);
        let dir = tmp("norange");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(partial(&o, &dir), &b[..5000]).unwrap();
        let file = download(&o, &dir, &AtomicU64::new(0), &AtomicU64::new(0), "t").unwrap();
        assert_eq!(std::fs::read(&file).unwrap(), b);
        assert_eq!(seen.lock().unwrap().as_slice(), &[Some("5000".into())]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_partial_file_of_another_build_is_not_resumed() {
        let b = body();
        let (url, seen) = page(b.clone(), true, None);
        let o = offer_for(&b, url);
        let dir = tmp("otherbuild");
        std::fs::create_dir_all(&dir).unwrap();
        let mut older = b[..5000].to_vec();
        older[10] ^= 1;
        let other = partial(&offer_for(&older, String::new()), &dir);
        std::fs::write(&other, &older).unwrap();
        std::fs::write(dir.join("Switchboard-mac.zip.partial"), &older).unwrap();
        let file = download(&o, &dir, &AtomicU64::new(0), &AtomicU64::new(0), "t").unwrap();
        assert_eq!(std::fs::read(&file).unwrap(), b);
        assert_eq!(seen.lock().unwrap().as_slice(), &[None], "started from the first byte");
        assert!(!other.exists() && !dir.join("Switchboard-mac.zip.partial").exists(), "the old partial files are gone");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_tampered_file_is_deleted_and_never_returned() {
        let b = body();
        let mut evil = b.clone();
        evil[1234] ^= 1;
        let (url, _) = page(evil, true, None);
        let o = offer_for(&b, url);
        let dir = tmp("tampered");
        let err = download(&o, &dir, &AtomicU64::new(0), &AtomicU64::new(0), "t").unwrap_err();
        assert!(err.to_string().contains("didn't match"), "{err}");
        assert!(!dir.join("Switchboard-mac.zip").exists() && !partial(&o, &dir).exists());

        // A partial file whose start was changed is caught at the end too.
        let (url, _) = page(b.clone(), true, None);
        let o = offer_for(&b, url);
        std::fs::write(partial(&o, &dir), vec![0u8; 1000]).unwrap();
        assert!(download(&o, &dir, &AtomicU64::new(0), &AtomicU64::new(0), "t").is_err());
        assert!(!partial(&o, &dir).exists());
        // A longer file than the list says is refused as it comes in.
        let mut long = b.clone();
        long.extend_from_slice(b"more");
        let (url, _) = page(long, true, None);
        let o = offer_for(&b, url);
        assert!(download(&o, &dir, &AtomicU64::new(0), &AtomicU64::new(0), "t").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---- The swap, in throwaway folders standing in for /Applications and the settings folder.

    /// Writes an app that says `what`: a bundle folder or a single file.
    fn app_at(kind: Kind, p: &Path, what: &str) {
        if kind == Kind::Bundle {
            std::fs::create_dir_all(p.join("Contents")).unwrap();
            std::fs::write(p.join("Contents/v"), what).unwrap();
        } else {
            std::fs::write(p, what).unwrap();
        }
    }

    fn says(kind: Kind, p: &Path) -> String {
        std::fs::read_to_string(if kind == Kind::Bundle { p.join("Contents/v") } else { p.to_path_buf() })
            .unwrap_or_default()
    }

    #[test]
    fn the_swap_keeps_the_old_build_and_rolls_back_after_a_failed_first_run() {
        for kind in [Kind::Bundle, Kind::Exe, Kind::Bin] {
            let root = tmp(&format!("swap-{kind:?}"));
            let apps = root.join("Applications");
            let conf = root.join("settings");
            std::fs::create_dir_all(&apps).unwrap();
            std::fs::create_dir_all(&conf).unwrap();
            let settings = conf.join("settings.json");
            std::fs::write(&settings, r#"{"theme": "dark", "first_run": "done"}"#).unwrap();
            let state = conf.join("update.json");
            let name = match kind {
                Kind::Bundle => "Switchboard.app",
                Kind::Exe => "Switchboard.exe",
                Kind::Bin => "Switchboard",
            };
            let target = apps.join(name);
            let o = offer_for(b"x", String::new());
            let (from, to) = ("20261012090000", o.build.clone());

            // Install, then a first run that crashes: the next start puts the old one back.
            app_at(kind, &target, "old");
            let new = apps.join(".Switchboard-update").join(name);
            std::fs::create_dir_all(new.parent().unwrap()).unwrap();
            app_at(kind, &new, "new");
            swap_in(kind, &new, &target, &state, &o, from).unwrap();
            assert_eq!(says(kind, &target), "new", "{kind:?}");
            assert_eq!(says(kind, &old_path(kind, &target)), "old", "{kind:?}: the old build is kept");
            assert_eq!(at_launch(kind, &state, Some(&to)), None, "{kind:?}: the first start goes on");
            assert_eq!(load_state(&state).pending.unwrap().starts, 1);
            assert_eq!(at_launch(kind, &state, Some(&to)), Some(target.clone()), "{kind:?}: it never said healthy");
            assert_eq!(says(kind, &target), "old", "{kind:?}: the old one is back");
            assert!(!old_path(kind, &target).exists());
            let st = load_state(&state);
            assert_eq!((st.pending, st.bad_build.as_deref()), (None, Some(to.as_str())));
            assert_eq!(at_launch(kind, &state, Some(from)), None, "{kind:?}: and then it starts as usual");

            // Install again, and this time it reports healthy: the old copy goes and nothing is left pending.
            std::fs::create_dir_all(new.parent().unwrap()).unwrap();
            app_at(kind, &new, "new");
            remove_any(&target.with_extension("failed.exe"));
            swap_in(kind, &new, &target, &state, &o, from).unwrap();
            assert_eq!(at_launch(kind, &state, Some(&to)), None);
            report_healthy(&state, Some(&to));
            assert_eq!(says(kind, &target), "new");
            assert!(!old_path(kind, &target).exists(), "{kind:?}: the old copy is gone once healthy");
            assert_eq!(load_state(&state).pending, None);
            assert_eq!(at_launch(kind, &state, Some(&to)), None);
            assert_eq!(says(kind, &target), "new", "{kind:?}: a healthy install stays");

            // An update never touches the settings or first-run choices.
            assert_eq!(std::fs::read_to_string(&settings).unwrap(), r#"{"theme": "dark", "first_run": "done"}"#);
            let _ = std::fs::remove_dir_all(&root);
        }
    }

    #[test]
    fn a_swap_that_fails_keeps_the_copy_an_earlier_update_left() {
        for kind in [Kind::Bundle, Kind::Exe, Kind::Bin] {
            let root = tmp(&format!("swap-fails-{kind:?}"));
            std::fs::create_dir_all(&root).unwrap();
            let target = root.join(if kind == Kind::Bundle { "Switchboard.app" } else { "Switchboard" });
            let old = old_path(kind, &target);
            app_at(kind, &target, "current");
            app_at(kind, &old, "earlier");
            // The new build is not there, so the swap can't finish.
            assert!(put_in(kind, &root.join("missing"), &target, &old).is_err(), "{kind:?}");
            assert_eq!(says(kind, &target), "current", "{kind:?}: the app is untouched");
            assert_eq!(says(kind, &old), "earlier", "{kind:?}: the earlier copy is still there");

            let new = root.join("new");
            app_at(kind, &new, "new");
            put_in(kind, &new, &target, &old).unwrap();
            assert_eq!(says(kind, &target), "new", "{kind:?}");
            assert_eq!(says(kind, &old), "current", "{kind:?}: the copy kept is the build just replaced");
            assert!(root.read_dir().unwrap().flatten().all(|e| !e.file_name().to_string_lossy().ends_with(".prev")));
            let _ = std::fs::remove_dir_all(&root);
        }
    }

    #[test]
    fn a_swap_that_can_not_be_recorded_is_undone() {
        let root = tmp("swap-norecord");
        std::fs::create_dir_all(&root).unwrap();
        let target = root.join("Switchboard");
        let new = root.join("Switchboard.new");
        app_at(Kind::Bin, &target, "old");
        app_at(Kind::Bin, &new, "new");
        // update.json's folder is a file, so the record can't be written.
        std::fs::write(root.join("conf"), "").unwrap();
        let err =
            swap_in(Kind::Bin, &new, &target, &root.join("conf/update.json"), &offer_for(b"x", String::new()), "1");
        assert!(err.is_err());
        assert_eq!(says(Kind::Bin, &target), "old");
        let _ = std::fs::remove_dir_all(&root);
    }
}
