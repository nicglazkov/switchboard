// Reads what names the app from /branding.toml at the repo root (src/brand.rs), so a fork that renames the app
// changes it there. The same few lines of parsing as sb-core's build.rs.

use std::path::Path;

fn main() {
    let manifest = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let path = Path::new(&manifest).join("../../../branding.toml");
    println!("cargo:rerun-if-changed={}", path.display());
    for dir in ["../../packaging/tour", "../../packaging/installer"] {
        println!("cargo:rerun-if-changed={dir}");
    }
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    for (key, env) in [
        ("name", "SB_APP_NAME"),
        ("desktop_id", "SB_DESKTOP_ID"),
        ("launcher_id", "SB_LAUNCHER_ID"),
        ("homepage", "SB_HOMEPAGE"),
    ] {
        let v = value(&text, "app", key).unwrap_or_else(|| panic!("{} has no [app] {key}", path.display()));
        println!("cargo:rustc-env={env}={v}");
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
