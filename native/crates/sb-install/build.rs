// Reads what names the app from /branding.toml at the repo root (src/brand.rs), so a fork that renames the app
// changes it there. The same few lines of parsing as sb-core's build.rs. On Windows it also embeds the installer's
// icon (packaging/windows/setup.ico, from scripts/make-installer-art.sh) and its version info.

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
    for dir in ["../../packaging/tour", "../../packaging/installer"] {
        println!("cargo:rerun-if-changed={dir}");
    }
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    for (key, env) in [
        ("name", "SB_APP_NAME"),
        ("desktop_id", "SB_DESKTOP_ID"),
        ("launcher_id", "SB_LAUNCHER_ID"),
        ("homepage", "SB_HOMEPAGE"),
        ("publisher", "SB_PUBLISHER"),
    ] {
        let v = value(&text, "app", key).unwrap_or_else(|| panic!("{} has no [app] {key}", path.display()));
        println!("cargo:rustc-env={env}={v}");
    }
    println!("cargo:rerun-if-changed=../../packaging/windows/setup.ico");
    #[cfg(windows)]
    windows_resources(&text);
}

/// The installer's icon and version info (ProductName, CompanyName = the publisher, FileVersion).
#[cfg(windows)]
fn windows_resources(branding: &str) {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let name = value(branding, "app", "name").unwrap_or_default();
    let publisher = value(branding, "app", "publisher").unwrap_or_default();
    let mut res = winresource::WindowsResource::new();
    res.set_icon("../../packaging/windows/setup.ico")
        .set("ProductName", &name)
        .set("FileDescription", &format!("{name} installer"))
        .set("InternalName", "switchboard-install")
        .set("OriginalFilename", "switchboard-install.exe")
        .set("CompanyName", &publisher)
        .set("LegalCopyright", &format!("Copyright {publisher}"));
    res.compile().expect("embed Windows resources (icon, version info)");
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
