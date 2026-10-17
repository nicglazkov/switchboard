//! macOS. Apps go in /Applications when this account can write there, else ~/Applications; no admin password is
//! ever asked for. A running copy is asked to quit through NSRunningApplication (the same request as Quit in its
//! menu, and no Automation prompt). The login item is a LaunchAgent that only opens the app at login. The Dock is
//! changed through `defaults`, like System Settings does, and only for the account's real home folder, so a test run
//! with HOME pointed elsewhere never touches the real Dock or launchd.

use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use super::{Applied, Dir, DiskImage, Platform, home};
use crate::brand::{DESKTOP_ID, LAUNCHER_ID, NAME};
use crate::uninstall::Layout;

pub struct Mac;

#[link(name = "AppKit", kind = "framework")]
unsafe extern "C" {}

impl Platform for Mac {
    fn install_dirs(&self) -> Vec<Dir> {
        vec![
            Dir { path: "/Applications".into(), label: "Applications".into() },
            Dir { path: home().join("Applications"), label: "Applications in your home folder".into() },
        ]
    }

    fn running(&self, app: &Path) -> Vec<u32> {
        running_from(app)
    }

    fn ask_to_quit(&self, app: &Path) {
        for pid in running_from(app) {
            objc_terminate(pid, false);
        }
    }

    fn force_quit(&self, app: &Path) {
        for pid in running_from(app) {
            if !objc_terminate(pid, true) {
                unsafe { libc::kill(pid as i32, libc::SIGKILL) };
            }
        }
    }

    fn copy_app(&self, from: &Path, to: &Path) -> io::Result<()> {
        run(Command::new("/usr/bin/ditto").arg(from).arg(to))?;
        // The installer was opened from this disk image, so Gatekeeper has already checked it. The copy keeps the
        // download's quarantine flag otherwise, and macOS would then run it from a read-only stand-in folder
        // ("App Translocation"), where it cannot update itself.
        let _ = run(Command::new("/usr/bin/xattr").args(["-dr", "com.apple.quarantine"]).arg(to));
        Ok(())
    }

    fn verify_app(&self, app: &Path) -> io::Result<()> {
        run(Command::new("/usr/bin/codesign").args(["--verify", "--deep", "--strict"]).arg(app)).map(|_| ())
    }

    fn set_login_item(&self, app: &Path, on: bool) -> io::Result<Applied> {
        let plist = login_item_path();
        if on {
            let mut d = plist::Dictionary::new();
            d.insert("Label".into(), LAUNCHER_ID.into());
            let args = vec!["/usr/bin/open".into(), "-a".into(), app.display().to_string().into()];
            d.insert("ProgramArguments".into(), plist::Value::Array(args));
            d.insert("RunAtLoad".into(), true.into());
            d.insert("LimitLoadToSessionType".into(), "Aqua".into());
            std::fs::create_dir_all(plist.parent().unwrap_or(Path::new("/")))?;
            plist::Value::Dictionary(d).to_file_xml(&plist).map_err(io::Error::other)?;
            // Not loaded now: loading it would open the app right away. launchd picks it up at the next login.
            return Ok(Applied::Done);
        }
        if real_home() {
            let target = format!("gui/{}/{LAUNCHER_ID}", unsafe { libc::getuid() });
            let loaded = Command::new("/bin/launchctl")
                .args(["print", &target])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
            if loaded.is_ok_and(|s| s.success()) {
                let _ = Command::new("/bin/launchctl").args(["bootout", &target]).stderr(Stdio::null()).status();
            }
        }
        crate::swap::remove_tree(&plist)?;
        Ok(Applied::Done)
    }

    fn set_in_dock(&self, app: &Path, on: bool) -> io::Result<Applied> {
        if !real_home() {
            return Ok(Applied::Skipped("HOME is not this account's home folder, so the Dock was left alone".into()));
        }
        let mut dock = plist::Value::from_reader(io::Cursor::new(run(Command::new("/usr/bin/defaults").args([
            "export",
            "com.apple.dock",
            "-",
        ]))?))
        .map_err(io::Error::other)?;
        let url = file_url(app);
        let Some(d) = dock.as_dictionary_mut() else {
            return Err(io::Error::other("the Dock's settings are not a dictionary"));
        };
        if !d.contains_key("persistent-apps") {
            d.insert("persistent-apps".into(), plist::Value::Array(vec![]));
        }
        let Some(apps) = d.get_mut("persistent-apps") else { return Err(io::Error::other("no Dock app list")) };
        let Some(apps) = apps.as_array_mut() else { return Err(io::Error::other("the Dock's app list is not a list")) };
        let before = apps.len();
        let has = apps.iter().any(|t| tile_url(t).as_deref() == Some(url.as_str()));
        if on && !has {
            apps.push(tile(&url));
        } else if !on {
            apps.retain(|t| tile_url(t).as_deref() != Some(url.as_str()));
        }
        if apps.len() == before {
            return Ok(Applied::Done);
        }
        let mut xml = vec![];
        dock.to_writer_xml(&mut xml).map_err(io::Error::other)?;
        let tmp = std::env::temp_dir().join(format!("sb-install-dock-{}.plist", std::process::id()));
        std::fs::write(&tmp, xml)?;
        let res = run(Command::new("/usr/bin/defaults").args(["import", "com.apple.dock"]).arg(&tmp));
        let _ = std::fs::remove_file(&tmp);
        res?;
        // The Dock reads its settings when it starts.
        let _ = Command::new("/usr/bin/killall").arg("Dock").status();
        Ok(Applied::Done)
    }

    fn open_app(&self, app: &Path) -> io::Result<()> {
        run(Command::new("/usr/bin/open").arg(app)).map(|_| ())
    }

    fn disk_image(&self) -> Option<DiskImage> {
        let exe = std::env::current_exe().ok()?.canonicalize().ok()?;
        let mut parts = exe.components();
        let (_, vols, name) = (parts.next()?, parts.next()?, parts.next()?);
        if vols.as_os_str() != "Volumes" {
            return None;
        }
        let mount = Path::new("/Volumes").join(name.as_os_str());
        Some(DiskImage { image: image_for(&mount), mount })
    }

    fn eject_after_exit(&self, img: &DiskImage) -> io::Result<()> {
        // This binary runs from the image, so a copy outside it does the work once this process is gone.
        let exe = std::env::current_exe()?;
        let helper = std::env::temp_dir().join(format!("sb-install-eject-{}", std::process::id()));
        std::fs::copy(&exe, &helper)?;
        let mut cmd = Command::new(&helper);
        cmd.arg("--eject-after").arg(std::process::id().to_string()).arg(&img.mount);
        if let Some(i) = &img.image {
            cmd.arg(i);
        }
        cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn()?;
        Ok(())
    }

    fn layout(&self, app: &Path) -> Layout {
        let lib = home().join("Library");
        let mut caches: Vec<PathBuf> = sb_core::cache::default_dir().into_iter().collect();
        caches.push(lib.join(format!("Saved Application State/{DESKTOP_ID}.savedState")));
        caches.push(lib.join(format!("HTTPStorages/{DESKTOP_ID}")));
        // Only the app's own logs (sb-desktop/src/logs.rs and panics.rs): the server's LaunchAgents write to the same
        // folder, and those stay.
        let logs = lib.join(format!("Logs/{NAME}"));
        caches.push(logs.join("panic.log"));
        caches.push(logs.join(format!("{NAME}.log")));
        caches.extend((1..=5).map(|n| logs.join(format!("{NAME}.log.{n}"))));
        if let Ok(rd) = std::fs::read_dir(&logs) {
            caches.extend(rd.flatten().map(|e| e.path()).filter(|p| {
                p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("crash-") && n.ends_with(".txt"))
            }));
        }
        let mut data: Vec<PathBuf> = self.settings_dir().into_iter().collect();
        data.push(lib.join(format!("Preferences/{DESKTOP_ID}.plist")));
        Layout { app: app.to_path_buf(), login_item: Some(login_item_path()), in_dock: in_dock(app), caches, data }
    }
}

/// The helper `eject_after_exit` starts: waits for the installer to quit, ejects the image, trashes the .dmg and
/// removes itself.
pub fn eject_helper(parent: u32, mount: &Path, image: Option<&Path>) {
    for _ in 0..240 {
        if unsafe { libc::kill(parent as i32, 0) } != 0 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
    let mut ejected = false;
    for _ in 0..5 {
        if Command::new("/usr/bin/hdiutil").arg("detach").arg(mount).arg("-quiet").status().is_ok_and(|s| s.success()) {
            ejected = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
    if ejected && let Some(i) = image {
        trash(i);
    }
    if let Ok(me) = std::env::current_exe() {
        let _ = std::fs::remove_file(me);
    }
}

fn login_item_path() -> PathBuf {
    home().join(format!("Library/LaunchAgents/{LAUNCHER_ID}.plist"))
}

/// True when HOME is this account's home folder (not a stand-in a test set).
fn real_home() -> bool {
    let pw = unsafe { libc::getpwuid(libc::getuid()) };
    if pw.is_null() {
        return false;
    }
    let dir = unsafe { std::ffi::CStr::from_ptr((*pw).pw_dir) };
    let real = PathBuf::from(dir.to_string_lossy().into_owned());
    let real = real.canonicalize().unwrap_or(real);
    home().canonicalize().is_ok_and(|h| h == real)
}

/// Runs a command and returns its output, or its error text as an io::Error.
fn run(cmd: &mut Command) -> io::Result<Vec<u8>> {
    let out = cmd.stdin(Stdio::null()).output()?;
    if out.status.success() {
        return Ok(out.stdout);
    }
    let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
    Err(io::Error::other(if err.is_empty() { format!("exit status {}", out.status) } else { err }))
}

/// Pids whose executable is inside `app`, from `ps` (fresh every call, unlike NSWorkspace's list).
fn running_from(app: &Path) -> Vec<u32> {
    let Ok(app) = app.canonicalize() else { return vec![] };
    let prefix = format!("{}/Contents/MacOS/", app.display());
    let Ok(out) = run(Command::new("/bin/ps").args(["-axo", "pid=,comm="])) else { return vec![] };
    // ps shows the path the process was started with, which may go through a link (/tmp is /private/tmp), so
    // paths inside any app bundle are resolved before they are compared.
    String::from_utf8_lossy(&out)
        .lines()
        .filter_map(|l| {
            let (pid, exe) = l.trim().split_once(' ')?;
            let exe = exe.trim();
            if !exe.contains(".app/Contents/MacOS/") {
                return None;
            }
            let real = Path::new(exe).canonicalize().map(|p| p.display().to_string()).unwrap_or(exe.to_string());
            real.starts_with(&prefix).then(|| pid.parse().ok()).flatten()
        })
        .collect()
}

/// `[[NSRunningApplication runningApplicationWithProcessIdentifier:pid] terminate]` (or forceTerminate).
#[allow(unexpected_cfgs)] // objc 0.2's msg_send! checks a cfg newer rustc does not know
fn objc_terminate(pid: u32, force: bool) -> bool {
    use objc::runtime::{BOOL, NO, Object};
    use objc::{class, msg_send, sel, sel_impl};
    objc::rc::autoreleasepool(|| unsafe {
        let app: *mut Object =
            msg_send![class!(NSRunningApplication), runningApplicationWithProcessIdentifier: pid as i32];
        if app.is_null() {
            return false;
        }
        let ok: BOOL = if force { msg_send![app, forceTerminate] } else { msg_send![app, terminate] };
        ok != NO
    })
}

/// Moves a file to the Trash the way Finder does (it can be put back).
#[allow(unexpected_cfgs)]
fn trash(path: &Path) {
    use objc::runtime::Object;
    use objc::{class, msg_send, sel, sel_impl};
    let Ok(c) = std::ffi::CString::new(path.display().to_string()) else { return };
    objc::rc::autoreleasepool(|| unsafe {
        let s: *mut Object = msg_send![class!(NSString), stringWithUTF8String: c.as_ptr()];
        let url: *mut Object = msg_send![class!(NSURL), fileURLWithPath: s];
        let fm: *mut Object = msg_send![class!(NSFileManager), defaultManager];
        let nil: *mut Object = std::ptr::null_mut();
        let _: objc::runtime::BOOL = msg_send![fm, trashItemAtURL: url resultingItemURL: nil error: nil];
    });
}

/// The .dmg mounted at `mount`, from `hdiutil info`.
fn image_for(mount: &Path) -> Option<PathBuf> {
    let out = run(Command::new("/usr/bin/hdiutil").args(["info", "-plist"])).ok()?;
    let v = plist::Value::from_reader(io::Cursor::new(out)).ok()?;
    for img in v.as_dictionary()?.get("images")?.as_array()? {
        let img = img.as_dictionary()?;
        let mounted = img.get("system-entities").and_then(|e| e.as_array()).is_some_and(|ents| {
            ents.iter().any(|e| {
                e.as_dictionary().and_then(|d| d.get("mount-point")).and_then(|m| m.as_string()).map(Path::new)
                    == Some(mount)
            })
        });
        if mounted {
            return img.get("image-path").and_then(|p| p.as_string()).map(PathBuf::from);
        }
    }
    None
}

fn in_dock(app: &Path) -> bool {
    if !real_home() {
        return false;
    }
    let Ok(out) = run(Command::new("/usr/bin/defaults").args(["export", "com.apple.dock", "-"])) else { return false };
    let Ok(v) = plist::Value::from_reader(io::Cursor::new(out)) else { return false };
    let url = file_url(app);
    v.as_dictionary()
        .and_then(|d| d.get("persistent-apps"))
        .and_then(|a| a.as_array())
        .is_some_and(|a| a.iter().any(|t| tile_url(t).as_deref() == Some(url.as_str())))
}

/// `file:///Applications/Switchboard.app/`, the form the Dock stores.
fn file_url(app: &Path) -> String {
    let mut s = String::from("file://");
    for b in app.display().to_string().bytes() {
        if b.is_ascii_alphanumeric() || b"/-._~".contains(&b) {
            s.push(b as char);
        } else {
            s.push_str(&format!("%{b:02X}"));
        }
    }
    if !s.ends_with('/') {
        s.push('/');
    }
    s
}

fn tile_url(tile: &plist::Value) -> Option<String> {
    let d = tile.as_dictionary()?.get("tile-data")?.as_dictionary()?.get("file-data")?.as_dictionary()?;
    d.get("_CFURLString")?.as_string().map(str::to_string)
}

fn tile(url: &str) -> plist::Value {
    let mut file = plist::Dictionary::new();
    file.insert("_CFURLString".into(), url.into());
    file.insert("_CFURLStringType".into(), 15.into());
    let mut data = plist::Dictionary::new();
    data.insert("file-data".into(), plist::Value::Dictionary(file));
    data.insert("file-label".into(), NAME.into());
    let mut t = plist::Dictionary::new();
    t.insert("tile-data".into(), plist::Value::Dictionary(data));
    t.insert("tile-type".into(), "file-tile".into());
    plist::Value::Dictionary(t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dock_urls() {
        assert_eq!(file_url(Path::new("/Applications/Switchboard.app")), "file:///Applications/Switchboard.app/");
        assert_eq!(file_url(Path::new("/Users/you/My Apps/X.app")), "file:///Users/you/My%20Apps/X.app/");
        assert_eq!(tile_url(&tile("file:///A.app/")).as_deref(), Some("file:///A.app/"));
    }
}
