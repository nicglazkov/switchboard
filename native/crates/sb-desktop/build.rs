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
    let mut res = winresource::WindowsResource::new();
    res.set_icon("../../packaging/windows/app.ico")
        .set("ProductName", "Switchboard")
        .set("FileDescription", "Switchboard")
        .set("InternalName", "Switchboard")
        .set("OriginalFilename", "Switchboard.exe")
        .set("CompanyName", "Switchboard contributors")
        .set("LegalCopyright", "Copyright Switchboard contributors");
    res.compile().expect("embed Windows resources (icon, version info)");
}
