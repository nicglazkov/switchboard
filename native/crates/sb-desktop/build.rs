// Windows only: embed the Converge icon and version info into Switchboard.exe.
// Other platforms get their icon from the bundle (macOS AppIcon.icns) or the desktop file (Linux).
// We always build natively (host == target), so `cfg(windows)` here also means a Windows target.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    // The build id ("Restart to update" compares it with the download page); set by the build scripts.
    println!("cargo:rerun-if-env-changed=SB_BUILD");
    println!("cargo:rerun-if-changed=../../packaging/windows/app.ico");
    #[cfg(windows)]
    windows_resources();
}

#[cfg(windows)]
fn windows_resources() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    // The name and publisher come from /branding.toml (a build host gets it inside native/).
    let manifest = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let path = ["../../../branding.toml", "../../branding.toml"]
        .iter()
        .map(|p| std::path::Path::new(&manifest).join(p))
        .find(|p| p.is_file())
        .expect("branding.toml");
    println!("cargo:rerun-if-changed={}", path.display());
    let text = std::fs::read_to_string(&path).expect("read branding.toml");
    let name = value(&text, "app", "name").expect("[app] name");
    let publisher = value(&text, "app", "publisher").expect("[app] publisher");
    let mut res = winresource::WindowsResource::new();
    res.set_icon("../../packaging/windows/app.ico")
        .set("ProductName", &name)
        .set("FileDescription", &name)
        .set("InternalName", &name)
        .set("OriginalFilename", &format!("{name}.exe"))
        .set("CompanyName", &publisher)
        .set("LegalCopyright", &format!("Copyright {publisher}"));
    res.compile().expect("embed Windows resources (icon, version info)");
}

/// The quoted string `key = "..."` under `[section]` (the same few lines as sb-core's build.rs).
#[cfg(windows)]
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
