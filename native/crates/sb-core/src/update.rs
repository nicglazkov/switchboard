//! "Restart to update" (the web app's self-update, 4d01e22, for the desktop apps): the server says what the download
//! page has (GET /api/app, from its latest.json); a build newer than this one is offered, downloaded on a click,
//! checked against its SHA-256, and installed only when the user says so. Nothing here restarts anything.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::net::CoreError;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ApiApp {
    #[serde(default)]
    pub latest: Option<Latest>,
    /// The download page ("http://host:port/app").
    #[serde(default)]
    pub base: Option<String>,
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
}

/// A newer build, ready to download.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Offer {
    pub version: String,
    pub build: String,
    pub built: Option<f64>,
    pub url: String,
    pub sha256: String,
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

/// The offer for `plat` when the page has a newer build than `mine`. Build ids are UTC timestamps of the same width,
/// so a plain comparison orders them.
pub fn offer(app: &ApiApp, plat: &str, mine: Option<&str>) -> Option<Offer> {
    let mine = mine?;
    let latest = app.latest.as_ref()?;
    let f = latest.files.get(plat)?;
    let base = app.base.as_deref()?.trim_end_matches('/');
    let ok_id = |b: &str| b.len() == 14 && b.bytes().all(|c| c.is_ascii_digit());
    if !ok_id(&f.build) || !ok_id(mine) || f.build.as_str() <= mine || f.sha256.len() != 64 || f.file.contains('/') {
        return None;
    }
    Some(Offer {
        version: latest.version.clone(),
        build: f.build.clone(),
        built: latest.built,
        url: format!("{base}/{}", f.file),
        sha256: f.sha256.to_ascii_lowercase(),
        file: f.file.clone(),
        page: format!("{base}/"),
    })
}

/// Downloads the offer into `dir`, counting bytes into `got` (and the size into `total` once known), and checks the
/// SHA-256. A file that does not match is deleted and reported.
pub fn download(o: &Offer, dir: &Path, got: &AtomicU64, total: &AtomicU64, user_agent: &str) -> Result<PathBuf, CoreError> {
    let cfg = ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(900)))
        .timeout_connect(Some(std::time::Duration::from_secs(8)))
        .http_status_as_error(false)
        .user_agent(user_agent)
        .build();
    let agent = ureq::Agent::new_with_config(cfg);
    let mut r = agent.get(&o.url).call().map_err(|_| CoreError::Unreachable)?;
    if r.status().as_u16() != 200 {
        return Err(CoreError::Bad(format!("The download page answered {}.", r.status().as_u16())));
    }
    if let Some(n) = r.headers().get("content-length").and_then(|v| v.to_str().ok()).and_then(|v| v.parse::<u64>().ok()) {
        total.store(n, Ordering::Relaxed);
    }
    std::fs::create_dir_all(dir).map_err(|e| CoreError::Bad(format!("Could not make a folder for the update: {e}")))?;
    let path = dir.join(&o.file);
    let mut out = std::fs::File::create(&path).map_err(|e| CoreError::Bad(format!("Could not save the update: {e}")))?;
    let mut body = r.body_mut().with_config().limit(1024 * 1024 * 1024).reader();
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = body.read(&mut buf).map_err(|_| CoreError::Unreachable)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
        out.write_all(&buf[..n]).map_err(|e| CoreError::Bad(format!("Could not save the update: {e}")))?;
        got.fetch_add(n as u64, Ordering::Relaxed);
    }
    drop(out);
    let sum: String = h.finalize().iter().map(|b| format!("{b:02x}")).collect();
    if sum != o.sha256 {
        let _ = std::fs::remove_file(&path);
        return Err(CoreError::Bad("The download did not match the page's checksum, so it was not installed.".into()));
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(build: &str) -> ApiApp {
        let mut files = HashMap::new();
        files.insert("mac".into(), LatestFile { file: "Switchboard-mac.zip".into(), sha256: "AB".repeat(32), build: build.into() });
        ApiApp { latest: Some(Latest { version: "0.1.0".into(), built: Some(1.0), files }), base: Some("http://10.0.0.5:8131/app/".into()) }
    }

    #[test]
    fn only_a_newer_build_is_offered() {
        let o = offer(&app("20261013120000"), "mac", Some("20261012090000")).unwrap();
        assert_eq!(o.url, "http://10.0.0.5:8131/app/Switchboard-mac.zip");
        assert_eq!(o.sha256, "ab".repeat(32));
        assert!(offer(&app("20261013120000"), "mac", Some("20261013120000")).is_none(), "the same build");
        assert!(offer(&app("20261011120000"), "mac", Some("20261013120000")).is_none(), "an older one");
        assert!(offer(&app("20261013120000"), "mac", None).is_none(), "a development build is never offered one");
        assert!(offer(&app("20261013120000"), "linux", Some("20261012090000")).is_none(), "nothing for this platform");
        assert!(offer(&app(""), "mac", Some("20261012090000")).is_none(), "a page without build ids");
    }
}
