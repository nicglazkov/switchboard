// Reads the app's desktop id, home page and documentation site from /branding.toml at the repo root (src/brand.rs), so a fork that
// renames the app changes them there. Only three `[app]` keys are needed, so a few lines of parsing do.

use std::path::Path;

fn main() {
    let manifest = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    // The build scripts copy only native/ to a build host, with branding.toml put inside it.
    let path = ["../../../branding.toml", "../../branding.toml"]
        .iter()
        .map(|p| Path::new(&manifest).join(p))
        .find(|p| p.is_file())
        .unwrap_or_else(|| Path::new(&manifest).join("../../../branding.toml"));
    println!("cargo:rerun-if-changed={}", path.display());
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    for (key, env) in [("desktop_id", "SB_DESKTOP_ID"), ("homepage", "SB_HOMEPAGE"), ("docs", "SB_DOCS")] {
        let v = value(&text, "app", key).unwrap_or_else(|| panic!("{} has no [app] {key}", path.display()));
        println!("cargo:rustc-env={env}={v}");
    }
    update_env();
}

/// Updates come from the build environment, never from the repo (src/update.rs): SB_UPDATE_PUBKEY, the base64
/// Ed25519 public key the feed is signed with (native/scripts/update-key.sh prints one), and SB_UPDATE_FEED, an
/// optional feed URL. A build without the key has updates off. A key or URL that is set but wrong stops the build,
/// so a release never ships with updates quietly off.
fn update_env() {
    use base64::Engine;
    println!("cargo:rerun-if-env-changed=SB_UPDATE_PUBKEY");
    println!("cargo:rerun-if-env-changed=SB_UPDATE_FEED");
    if let Some(k) = std::env::var("SB_UPDATE_PUBKEY").ok().map(|k| k.trim().to_string()).filter(|k| !k.is_empty()) {
        match base64::engine::general_purpose::STANDARD.decode(&k) {
            Ok(raw) if raw.len() == 32 => println!("cargo:rustc-env=SB_UPDATE_PUBKEY={k}"),
            _ => panic!("SB_UPDATE_PUBKEY must be a base64 Ed25519 public key (32 bytes), as update-key.sh prints it"),
        }
    }
    if let Some(f) = std::env::var("SB_UPDATE_FEED").ok().map(|f| f.trim().to_string()).filter(|f| !f.is_empty()) {
        // The app's HTTP client is built without TLS; the signature is what makes the feed safe to trust.
        let ok = f.strip_prefix("http://").is_some_and(|rest| {
            !rest.is_empty() && rest.bytes().all(|c| c.is_ascii_alphanumeric() || b".:-_/%~".contains(&c))
        });
        assert!(ok, "SB_UPDATE_FEED must be an http:// URL of the folder with latest.json, got {f:?}");
        println!("cargo:rustc-env=SB_UPDATE_FEED={f}");
    }
}

/// The quoted string `key = "..."` under `[section]`, ignoring comments after it.
fn value(text: &str, section: &str, key: &str) -> Option<String> {
    let mut current = String::new();
    for line in text.lines() {
        let line = line.trim();
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            current = name.trim().to_string();
            continue;
        }
        if current != section {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else { continue };
        if k.trim() != key {
            continue;
        }
        let v = v.trim().strip_prefix('"')?;
        return Some(v[..v.find('"')?].to_string());
    }
    None
}
