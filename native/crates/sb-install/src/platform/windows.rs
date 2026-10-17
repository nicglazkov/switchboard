//! Windows. A per-user install with no admin rights: the app goes in %LOCALAPPDATA%\Programs\Switchboard (the
//! exe, its license notices and the uninstaller), and everything else is under HKCU or in the user's own folders:
//! a Start menu shortcut (always), a desktop shortcut and an HKCU Run value for "Open at login" (both only when
//! chosen), and the Settings > Apps entry under HKCU\...\Uninstall.
//!
//! A running copy is asked to quit by closing its windows (WM_CLOSE, the same as clicking the close button), and is
//! stopped with TerminateProcess only when the user says so. The uninstaller lives in the app's folder, so when it
//! removes that folder it first moves its own exe to the temp folder and deletes it from there once it has exited.
//!
//! Tests point every place at a throwaway folder and registry key with SB_INSTALL_TEST_ROOT and
//! SB_INSTALL_TEST_REG (both or neither), so a test run never touches the real Start menu, desktop, Run key or
//! Settings > Apps.

use std::path::{Path, PathBuf};

use crate::brand::{DESKTOP_ID, HOMEPAGE, NAME, PUBLISHER};

/// A throwaway folder that stands in for every folder the installer writes to. Tests only.
pub const TEST_ROOT: &str = "SB_INSTALL_TEST_ROOT";
/// A throwaway HKCU key (`Software\SwitchboardTest-<anything>`) that stands in for the Uninstall, Run and
/// AppUserModelId keys. Tests only.
pub const TEST_REG: &str = "SB_INSTALL_TEST_REG";
const TEST_REG_PREFIX: &str = r"Software\SwitchboardTest-";

/// The app's exe in its folder.
pub fn exe_name() -> String {
    format!("{NAME}.exe")
}

/// The uninstaller's name in the app's folder (a copy of the installer with nothing attached).
pub fn uninstaller_name() -> String {
    format!("Uninstall {NAME}.exe")
}

/// Every place the Windows backend reads or writes outside the app's folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Places {
    /// The folder the app's folder goes in: %LOCALAPPDATA%\Programs.
    pub programs: PathBuf,
    /// The user's Start menu programs folder.
    pub start_menu: PathBuf,
    pub desktop: PathBuf,
    /// The app's settings folder (settings.json, first-run.json): %APPDATA%\Switchboard.
    pub settings: PathBuf,
    /// %LOCALAPPDATA%\Switchboard\cache, and \logs (sb-desktop's logs.rs), which the server shares on Windows.
    pub cache: PathBuf,
    pub logs: PathBuf,
    /// HKCU subkeys.
    pub uninstall_key: String,
    pub run_key: String,
    /// Where Windows looks up the name the app's notifications show (GPUI writes it the first time).
    pub aumid_key: String,
    /// Running against a test sandbox.
    pub test: bool,
}

impl Places {
    /// The real places, or the test sandbox when SB_INSTALL_TEST_ROOT and SB_INSTALL_TEST_REG are set.
    pub fn from_env() -> Result<Places, String> {
        let root = std::env::var_os(TEST_ROOT).filter(|v| !v.is_empty());
        let reg = std::env::var(TEST_REG).ok().filter(|v| !v.is_empty());
        match (root, reg) {
            (None, None) => Places::real(),
            (Some(root), Some(reg)) => Places::sandbox(Path::new(&root), &reg),
            _ => Err(format!("set both {TEST_ROOT} and {TEST_REG}, or neither")),
        }
    }

    pub fn real() -> Result<Places, String> {
        let env = |v: &str| std::env::var_os(v).map(PathBuf::from).filter(|p| p.is_absolute());
        let local = env("LOCALAPPDATA").ok_or("LOCALAPPDATA is not set")?;
        let roaming = env("APPDATA").ok_or("APPDATA is not set")?;
        let desktop = dirs::desktop_dir().ok_or("Windows did not say where the desktop is")?;
        let settings = sb_core::cache::settings_dir().unwrap_or_else(|| roaming.join(NAME));
        let cache = sb_core::cache::default_dir().unwrap_or_else(|| local.join(NAME).join("cache"));
        Ok(Places {
            programs: local.join("Programs"),
            start_menu: roaming.join(r"Microsoft\Windows\Start Menu\Programs"),
            desktop,
            settings,
            cache,
            logs: local.join(NAME).join("logs"),
            uninstall_key: format!(r"Software\Microsoft\Windows\CurrentVersion\Uninstall\{DESKTOP_ID}"),
            run_key: r"Software\Microsoft\Windows\CurrentVersion\Run".into(),
            aumid_key: format!(r"Software\Classes\AppUserModelId\{DESKTOP_ID}"),
            test: false,
        })
    }

    /// Every folder under `root` and every key under HKCU\`reg`.
    pub fn sandbox(root: &Path, reg: &str) -> Result<Places, String> {
        if !root.is_absolute() {
            return Err(format!("{TEST_ROOT} must be a full path"));
        }
        let ok = reg.strip_prefix(TEST_REG_PREFIX).is_some_and(|rest| {
            !rest.is_empty() && rest.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
        });
        if !ok {
            return Err(format!(r"{TEST_REG} must be {TEST_REG_PREFIX}<letters, digits or dashes>"));
        }
        Ok(Places {
            programs: root.join("Programs"),
            start_menu: root.join(r"Start Menu\Programs"),
            desktop: root.join("Desktop"),
            settings: root.join("Roaming").join(NAME),
            cache: root.join("Local").join(NAME).join("cache"),
            logs: root.join("Local").join(NAME).join("logs"),
            uninstall_key: format!(r"{reg}\Uninstall\{DESKTOP_ID}"),
            run_key: format!(r"{reg}\Run"),
            aumid_key: format!(r"{reg}\AppUserModelId\{DESKTOP_ID}"),
            test: true,
        })
    }

    pub fn start_shortcut(&self) -> PathBuf {
        self.start_menu.join(format!("{NAME}.lnk"))
    }

    pub fn desktop_shortcut(&self) -> PathBuf {
        self.desktop.join(format!("{NAME}.lnk"))
    }
}

/// The options screen's words for a choice on Windows.
pub fn choice_text(c: crate::record::Choice) -> (String, String) {
    use crate::record::Choice;
    match c {
        Choice::OpenAtLogin => (
            format!("Open {NAME} when you sign in"),
            "Starts the app when you sign in to Windows. Nothing else runs in the background.".into(),
        ),
        Choice::DesktopShortcut => {
            (format!("Put a {NAME} shortcut on the desktop"), "The Start menu shortcut is added either way.".into())
        }
        Choice::Dock => super::mac_choice_text(c),
    }
}

/// What every Windows install adds besides the app's folder, said on the welcome screen.
pub fn also_adds() -> String {
    "Adds a Start menu shortcut, and an entry in Settings > Apps to uninstall it from.".into()
}

/// A registry value as the Settings > Apps entry holds it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Value {
    Str(String),
    Dword(u32),
}

/// A path in double quotes, the way Windows command lines want it.
pub fn quoted(p: &Path) -> String {
    format!("\"{}\"", p.display())
}

/// The values of the Settings > Apps entry for the app installed at `app`.
pub fn uninstall_values(app: &Path, version: &str, size_kb: u32) -> Vec<(&'static str, Value)> {
    let exe = app.join(exe_name());
    let uninstaller = quoted(&app.join(uninstaller_name()));
    let s = |v: String| Value::Str(v);
    vec![
        ("DisplayName", s(NAME.into())),
        ("DisplayVersion", s(version.into())),
        ("Publisher", s(PUBLISHER.into())),
        ("DisplayIcon", s(format!("{},0", exe.display()))),
        ("InstallLocation", s(app.display().to_string())),
        ("UninstallString", s(format!("{uninstaller} --uninstall"))),
        ("QuietUninstallString", s(format!("{uninstaller} --uninstall --quiet"))),
        ("URLInfoAbout", s(HOMEPAGE.into())),
        ("EstimatedSize", Value::Dword(size_kb)),
        ("NoModify", Value::Dword(1)),
        ("NoRepair", Value::Dword(1)),
    ]
}

/// The Run value for "Open at login".
pub fn run_value(app: &Path) -> String {
    quoted(&app.join(exe_name()))
}

/// The size of a folder in KB, rounded up, as Settings > Apps shows it.
pub fn folder_kb(dir: &Path) -> u32 {
    fn walk(p: &Path) -> u64 {
        match p.symlink_metadata() {
            Ok(m) if m.is_dir() => std::fs::read_dir(p).map_or(0, |rd| rd.flatten().map(|e| walk(&e.path())).sum()),
            Ok(m) => m.len(),
            Err(_) => 0,
        }
    }
    u32::try_from(walk(dir).div_ceil(1024)).unwrap_or(u32::MAX)
}

/// True when `path` is `dir` or inside it, compared the way Windows compares paths (case and slashes ignored).
pub fn inside(path: &Path, dir: &Path) -> bool {
    let norm = |p: &Path| {
        let s = p.display().to_string().replace('/', "\\").to_lowercase();
        let s = s.strip_prefix(r"\\?\").map(str::to_string).unwrap_or(s);
        s.trim_end_matches('\\').to_string()
    };
    let (p, d) = (norm(path), norm(dir));
    !d.is_empty() && (p == d || p.starts_with(&format!("{d}\\")))
}

/// The command a finished uninstaller leaves behind to delete its own exe (and the folder it sits in) once it has
/// exited: cmd waits two seconds with ping, then deletes. It runs with CREATE_NO_WINDOW, so no console shows.
pub fn self_delete_command(folder: &Path) -> String {
    format!("/d /c ping -n 3 127.0.0.1 >nul & rmdir /s /q {}", quoted(folder))
}

/// The messages for what the GPU check found (see sys::graphics).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Graphics {
    Hardware,
    /// Only Windows' software renderer (the Basic Render Driver) can draw.
    SoftwareOnly,
    None,
}

impl Graphics {
    pub fn warning(self) -> Option<String> {
        match self {
            Graphics::Hardware => None,
            Graphics::SoftwareOnly => Some(format!(
                "Only Windows' basic display driver is working, so {NAME} will draw slowly. \
                 Installing the driver for your graphics card fixes that."
            )),
            Graphics::None => Some(format!(
                "{NAME} draws its window with DirectX 11, and no graphics driver here supports it, so it will not \
                 open yet. Install or update the graphics driver from your PC or graphics card maker, then open it."
            )),
        }
    }
}

#[cfg(windows)]
pub use real::Windows;
#[cfg(windows)]
pub use sys::{alert, attach_console, graphics};

#[cfg(windows)]
mod real {
    use std::io;
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;

    use super::super::{Applied, Dir, Platform};
    use super::{Places, Value, exe_name, folder_kb, inside, run_value, sys, uninstall_values, uninstaller_name};
    use crate::brand::NAME;
    use crate::record::Choice;
    use crate::uninstall::{Item, Layout, What};

    pub struct Windows {
        places: Places,
        /// Where the attached app was unpacked, removed at cleanup.
        unpacked: Mutex<Option<PathBuf>>,
        /// The folder the uninstaller moved its own exe to, deleted after it exits.
        self_delete: Mutex<Option<PathBuf>>,
    }

    impl Windows {
        pub fn new(places: Places) -> Windows {
            Windows { places, unpacked: Mutex::new(None), self_delete: Mutex::new(None) }
        }

        fn present(&self, key: &str, value: Option<&str>) -> bool {
            match windows_registry::CURRENT_USER.open(key) {
                Ok(k) => value.is_none_or(|v| k.get_value(v).is_ok()),
                Err(_) => false,
            }
        }
    }

    fn reg_err(e: windows::core::Error) -> io::Error {
        io::Error::other(e.message())
    }

    /// Missing is fine when removing.
    fn gone(r: windows::core::Result<()>) -> io::Result<()> {
        match r {
            Err(e) if e.code() == windows::core::HRESULT::from_win32(2) => Ok(()),
            r => r.map_err(reg_err),
        }
    }

    impl Platform for Windows {
        fn install_dirs(&self) -> Vec<Dir> {
            vec![Dir { path: self.places.programs.clone(), label: "Programs in your user folder".into() }]
        }

        fn bundle_name(&self) -> String {
            NAME.into()
        }

        fn find_payload(&self, given: Option<&Path>) -> Result<PathBuf, String> {
            if let Some(p) = given {
                return if p.join(exe_name()).is_file() {
                    Ok(p.to_path_buf())
                } else {
                    Err(format!("{} has no {} in it.", p.display(), exe_name()))
                };
            }
            if let Some(p) = self.unpacked.lock().ok().and_then(|u| u.clone()) {
                return Ok(p);
            }
            let exe = std::env::current_exe().map_err(|e| e.to_string())?;
            let missing =
                || format!("This installer has no copy of {NAME} attached. Download {NAME}-Setup again and open that.");
            if !crate::pack::attached(&exe).map_err(|e| e.to_string())?.is_some() {
                return Err(missing());
            }
            let dir = std::env::temp_dir().join(format!("sb-setup-{}", std::process::id())).join(NAME);
            let _ = crate::swap::remove_tree(&dir);
            if let Err(e) = crate::pack::extract(&exe, &dir) {
                let _ = crate::swap::remove_tree(dir.parent().unwrap_or(&dir));
                return Err(format!("Could not unpack {NAME} from the installer ({e}). Download it again."));
            }
            if let Ok(mut u) = self.unpacked.lock() {
                *u = Some(dir.clone());
            }
            Ok(dir)
        }

        fn choices(&self) -> &'static [Choice] {
            &[Choice::OpenAtLogin, Choice::DesktopShortcut]
        }

        fn also_adds(&self) -> Option<String> {
            Some(super::also_adds())
        }

        fn choice_text(&self, c: Choice) -> (String, String) {
            super::choice_text(c)
        }

        fn settings_app(&self) -> &'static str {
            "Settings"
        }

        fn warnings(&self) -> Vec<String> {
            sys::graphics().warning().into_iter().collect()
        }

        fn bundle_version(&self, app: &Path) -> Option<String> {
            sys::file_version(&app.join(exe_name()))
        }

        fn running(&self, app: &Path) -> Vec<u32> {
            let me = std::process::id();
            sys::processes().into_iter().filter(|(pid, exe)| *pid != me && inside(exe, app)).map(|(p, _)| p).collect()
        }

        fn ask_to_quit(&self, app: &Path) {
            sys::close_windows(&self.running(app));
        }

        fn force_quit(&self, app: &Path) {
            for pid in self.running(app) {
                sys::terminate(pid);
            }
        }

        fn verify_app(&self, app: &Path) -> io::Result<()> {
            // Never run here: a copy that reads back as a Windows program with a version is good enough, and
            // starting it could open its window.
            let exe = app.join(exe_name());
            let mut head = [0u8; 2];
            std::io::Read::read_exact(&mut std::fs::File::open(&exe)?, &mut head)?;
            if &head != b"MZ" {
                return Err(io::Error::other(format!("{} is not a Windows program", exe.display())));
            }
            sys::file_version(&exe).map(|_| ()).ok_or_else(|| io::Error::other("it states no version"))
        }

        fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
            // A just-stopped process or a virus scan of the new files can hold them for a moment.
            let mut last = None;
            for _ in 0..15 {
                match std::fs::rename(from, to) {
                    Ok(()) => return Ok(()),
                    Err(e) if e.kind() == io::ErrorKind::PermissionDenied || e.raw_os_error() == Some(32) => {
                        last = Some(e);
                        std::thread::sleep(std::time::Duration::from_millis(200));
                    }
                    Err(e) => return Err(e),
                }
            }
            Err(last.unwrap_or_else(|| io::Error::other("busy")))
        }

        fn set_login_item(&self, app: &Path, on: bool) -> io::Result<Applied> {
            let key = windows_registry::CURRENT_USER;
            if on {
                key.create(&self.places.run_key).and_then(|k| k.set_string(NAME, run_value(app))).map_err(reg_err)?;
            } else if let Ok(k) = key.options().read().write().open(&self.places.run_key) {
                gone(k.remove_value(NAME))?;
            }
            Ok(Applied::Done)
        }

        fn set_in_dock(&self, _app: &Path, _on: bool) -> io::Result<Applied> {
            Ok(Applied::Skipped("Windows has no Dock; the Start menu shortcut is always added".into()))
        }

        fn set_desktop_shortcut(&self, app: &Path, on: bool) -> io::Result<Applied> {
            let lnk = self.places.desktop_shortcut();
            if on {
                std::fs::create_dir_all(&self.places.desktop)?;
                sys::shortcut(&lnk, &app.join(exe_name()), app)?;
            } else {
                crate::swap::remove_tree(&lnk)?;
            }
            Ok(Applied::Done)
        }

        fn registers(&self) -> bool {
            true
        }

        fn register(&self, app: &Path, version: &str) -> io::Result<String> {
            std::fs::create_dir_all(&self.places.start_menu)?;
            sys::shortcut(&self.places.start_shortcut(), &app.join(exe_name()), app)?;
            let k = windows_registry::CURRENT_USER.create(&self.places.uninstall_key).map_err(reg_err)?;
            for (name, v) in uninstall_values(app, version, folder_kb(app)) {
                match v {
                    Value::Str(s) => k.set_string(name, s),
                    Value::Dword(d) => k.set_u32(name, d),
                }
                .map_err(reg_err)?;
            }
            Ok(format!("{}, HKCU\\{}", self.places.start_shortcut().display(), self.places.uninstall_key))
        }

        fn unregister(&self, _app: &Path) -> io::Result<Applied> {
            gone(windows_registry::CURRENT_USER.remove_tree(&self.places.uninstall_key))?;
            gone(windows_registry::CURRENT_USER.remove_tree(&self.places.aumid_key))?;
            Ok(Applied::Done)
        }

        fn remove_app(&self, app: &Path) -> io::Result<()> {
            let me = std::env::current_exe()?;
            if inside(&me, app) {
                // A running exe can be renamed but not deleted: move it out first, and delete it once it has quit.
                let away = std::env::temp_dir().join(format!("sb-uninstall-{}", std::process::id()));
                std::fs::create_dir_all(&away)?;
                let moved = away.join(uninstaller_name());
                match std::fs::rename(&me, &moved) {
                    Ok(()) => {
                        if let Ok(mut s) = self.self_delete.lock() {
                            *s = Some(away);
                        }
                    }
                    Err(_) => {
                        // The temp folder is on another drive. Leave the exe and remove the whole folder afterwards.
                        let _ = std::fs::remove_dir(&away);
                        for e in std::fs::read_dir(app)?.flatten() {
                            if !inside(&me, &e.path()) {
                                crate::swap::remove_tree(&e.path())?;
                            }
                        }
                        if let Ok(mut s) = self.self_delete.lock() {
                            *s = Some(app.to_path_buf());
                        }
                        return Ok(());
                    }
                }
            }
            crate::swap::remove_tree(app)
        }

        fn cleanup(&self) {
            if let Some(dir) = self.unpacked.lock().ok().and_then(|mut u| u.take()) {
                let _ = crate::swap::remove_tree(dir.parent().unwrap_or(&dir));
            }
            // The logs folder and %LOCALAPPDATA%\Switchboard once they are empty (remove_dir only takes an empty folder;
            // the server, when it runs here, keeps its own files there).
            let _ = std::fs::remove_dir(&self.places.logs);
            if let Some(parent) = self.places.logs.parent() {
                let _ = std::fs::remove_dir(parent);
            }
            if let Some(dir) = self.self_delete.lock().ok().and_then(|mut s| s.take()) {
                sys::delete_after_exit(&dir);
            }
        }

        fn own_install(&self) -> Option<PathBuf> {
            let me = std::env::current_exe().ok()?;
            let dir = me.parent()?;
            let named = me.file_name().is_some_and(|n| n.eq_ignore_ascii_case(uninstaller_name().as_str()));
            (named && dir.join(exe_name()).is_file()).then(|| dir.to_path_buf())
        }

        fn open_app(&self, app: &Path) -> io::Result<()> {
            sys::start_detached(&app.join(exe_name()), app)
        }

        fn settings_dir(&self) -> Option<PathBuf> {
            Some(self.places.settings.clone())
        }

        fn layout(&self, app: &Path) -> Layout {
            let p = &self.places;
            let mut entries = vec![];
            if self.present(&p.run_key, Some(NAME)) {
                entries.push(Item { what: What::LoginItem, path: format!(r"HKCU\{}\{NAME}", p.run_key).into() });
            }
            for key in [&p.uninstall_key, &p.aumid_key] {
                if self.present(key, None) {
                    entries.push(Item { what: What::Registry, path: format!(r"HKCU\{key}").into() });
                }
            }
            Layout {
                app: app.to_path_buf(),
                shortcuts: vec![p.start_shortcut(), p.desktop_shortcut()],
                entries,
                caches: std::iter::once(p.cache.clone()).chain(super::super::app_log_files(&p.logs)).collect(),
                data: vec![p.settings.clone()],
                ..Default::default()
            }
        }
    }
}

/// The calls into Windows.
#[cfg(windows)]
mod sys {
    use std::io;
    use std::os::windows::process::CommandExt;
    use std::path::{Path, PathBuf};

    use windows::Win32::Foundation::{CloseHandle, HMODULE, HWND, LPARAM, WPARAM};
    use windows::Win32::Graphics::Direct3D::{
        D3D_DRIVER_TYPE_UNKNOWN, D3D_FEATURE_LEVEL_10_1, D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_11_1,
    };
    use windows::Win32::Graphics::Direct3D11::{
        D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_SDK_VERSION, D3D11CreateDevice,
    };
    use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, DXGI_ADAPTER_FLAG_SOFTWARE, IDXGIFactory1};
    use windows::Win32::Storage::EnhancedStorage::PKEY_AppUserModel_ID;
    use windows::Win32::Storage::FileSystem::{GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW};
    use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
    use windows::Win32::System::Com::{
        CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx, IPersistFile,
    };
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
    };
    use windows::Win32::System::Threading::{
        OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE,
        QueryFullProcessImageNameW, TerminateProcess,
    };
    use windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;
    use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetWindowThreadProcessId, IsWindowVisible, MB_ICONWARNING, MB_OK, MessageBoxW, PostMessageW,
        WM_CLOSE,
    };
    use windows::core::{BOOL, HSTRING, Interface, PWSTR};

    use super::Graphics;
    use crate::brand::{DESKTOP_ID, NAME};

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    const DETACHED_PROCESS: u32 = 0x0000_0008;

    /// ProductVersion from the exe's version resource ("0.8.4", "1.0.0-beta.2"), else its fixed file version.
    pub fn file_version(exe: &Path) -> Option<String> {
        let name = HSTRING::from(exe.as_os_str());
        unsafe {
            let size = GetFileVersionInfoSizeW(&name, None);
            if size == 0 {
                return None;
            }
            let mut buf = vec![0u8; size as usize];
            GetFileVersionInfoW(&name, None, size, buf.as_mut_ptr().cast()).ok()?;
            let query = |sub: &str| -> Option<(*const u8, u32)> {
                let mut p = std::ptr::null_mut();
                let mut len = 0u32;
                VerQueryValueW(buf.as_ptr().cast(), &HSTRING::from(sub), &mut p, &mut len)
                    .as_bool()
                    .then_some((p as *const u8, len))
                    .filter(|(p, l)| !p.is_null() && *l > 0)
            };
            let mut langs = vec![(0x0409u16, 0x04b0u16)];
            if let Some((p, len)) = query(r"\VarFileInfo\Translation")
                && len >= 4
            {
                let w = p as *const u16;
                langs.insert(0, (*w, *w.add(1)));
            }
            for (lang, cp) in langs {
                if let Some((p, len)) = query(&format!(r"\StringFileInfo\{lang:04x}{cp:04x}\ProductVersion")) {
                    let s = std::slice::from_raw_parts(p as *const u16, len as usize);
                    let v = String::from_utf16_lossy(s).trim_end_matches('\0').trim().to_string();
                    if !v.is_empty() {
                        return Some(v);
                    }
                }
            }
            let (p, len) = query("\\")?;
            if (len as usize) < 16 {
                return None;
            }
            // VS_FIXEDFILEINFO: dwFileVersionMS at 8, dwFileVersionLS at 12.
            let d =
                |at: usize| u32::from_le_bytes(std::slice::from_raw_parts(p.add(at), 4).try_into().unwrap_or([0; 4]));
            let (ms, ls) = (d(8), d(12));
            Some(format!("{}.{}.{}", ms >> 16, ms & 0xffff, ls >> 16))
        }
    }

    /// Every process this account can see, with its exe's full path.
    pub fn processes() -> Vec<(u32, PathBuf)> {
        let mut out = vec![];
        unsafe {
            let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else { return out };
            let mut e = PROCESSENTRY32W { dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
            let mut more = Process32FirstW(snap, &mut e).is_ok();
            while more {
                if let Some(p) = image_path(e.th32ProcessID) {
                    out.push((e.th32ProcessID, p));
                }
                more = Process32NextW(snap, &mut e).is_ok();
            }
            let _ = CloseHandle(snap);
        }
        out
    }

    fn image_path(pid: u32) -> Option<PathBuf> {
        unsafe {
            let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
            let mut buf = vec![0u16; 1024];
            let mut len = buf.len() as u32;
            let ok = QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len).is_ok();
            let _ = CloseHandle(h);
            ok.then(|| PathBuf::from(String::from_utf16_lossy(&buf[..len as usize])))
        }
    }

    /// Posts WM_CLOSE to the visible top-level windows of these processes: the same as clicking their close button.
    pub fn close_windows(pids: &[u32]) {
        unsafe extern "system" fn each(hwnd: HWND, lp: LPARAM) -> BOOL {
            let pids = unsafe { &*(lp.0 as *const Vec<u32>) };
            let mut pid = 0u32;
            unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
            if pids.contains(&pid) && unsafe { IsWindowVisible(hwnd) }.as_bool() {
                let _ = unsafe { PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0)) };
            }
            BOOL(1)
        }
        if pids.is_empty() {
            return;
        }
        let list = pids.to_vec();
        unsafe {
            let _ = EnumWindows(Some(each), LPARAM(&list as *const Vec<u32> as isize));
        }
    }

    pub fn terminate(pid: u32) {
        unsafe {
            if let Ok(h) = OpenProcess(PROCESS_TERMINATE, false, pid) {
                let _ = TerminateProcess(h, 1);
                let _ = CloseHandle(h);
            }
        }
    }

    /// A .lnk at `lnk` that opens `exe` in `dir`, carrying the app's AppUserModelID so the taskbar groups its
    /// windows with the pinned shortcut.
    pub fn shortcut(lnk: &Path, exe: &Path, dir: &Path) -> io::Result<()> {
        let io = |e: windows::core::Error| io::Error::other(e.message());
        unsafe {
            // Already initialized on this thread is fine.
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).map_err(io)?;
            link.SetPath(&HSTRING::from(exe.as_os_str())).map_err(io)?;
            link.SetWorkingDirectory(&HSTRING::from(dir.as_os_str())).map_err(io)?;
            link.SetDescription(&HSTRING::from(NAME)).map_err(io)?;
            link.SetIconLocation(&HSTRING::from(exe.as_os_str()), 0).map_err(io)?;
            let store: IPropertyStore = link.cast().map_err(io)?;
            store.SetValue(&PKEY_AppUserModel_ID, &PROPVARIANT::from(DESKTOP_ID)).map_err(io)?;
            store.Commit().map_err(io)?;
            let file: IPersistFile = link.cast().map_err(io)?;
            file.Save(&HSTRING::from(lnk.as_os_str()), true).map_err(io)?;
        }
        Ok(())
    }

    /// What can draw GPUI's window: a Direct3D 11 device at feature level 10.1 or better, as gpui_windows makes it.
    pub fn graphics() -> Graphics {
        unsafe {
            let Ok(factory) = CreateDXGIFactory1::<IDXGIFactory1>() else { return Graphics::None };
            let mut found = Graphics::None;
            let mut i = 0;
            while let Ok(adapter) = factory.EnumAdapters1(i) {
                i += 1;
                let soft = adapter.GetDesc1().is_ok_and(|d| d.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32 != 0);
                let levels = [D3D_FEATURE_LEVEL_11_1, D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_10_1];
                let ok = D3D11CreateDevice(
                    &adapter,
                    D3D_DRIVER_TYPE_UNKNOWN,
                    HMODULE::default(),
                    D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                    Some(&levels),
                    D3D11_SDK_VERSION,
                    None,
                    None,
                    None,
                )
                .is_ok();
                // The Basic Render Driver shows up as an adapter with no SOFTWARE flag but this vendor id.
                let basic = adapter.GetDesc1().is_ok_and(|d| d.VendorId == 0x1414 && d.DeviceId == 0x8c);
                if ok && !soft && !basic {
                    return Graphics::Hardware;
                }
                if ok {
                    found = Graphics::SoftwareOnly;
                }
            }
            found
        }
    }

    /// Deletes `folder` (the uninstaller's own exe and its folder) once this process has exited.
    pub fn delete_after_exit(folder: &Path) {
        let _ = std::process::Command::new("cmd.exe")
            .raw_arg(super::self_delete_command(folder))
            .current_dir(std::env::temp_dir())
            .creation_flags(CREATE_NO_WINDOW)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
    }

    pub fn start_detached(exe: &Path, dir: &Path) -> io::Result<()> {
        std::process::Command::new(exe)
            .current_dir(dir)
            .creation_flags(DETACHED_PROCESS)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map(|_| ())
    }

    /// A plain message box, for when the installer's own window cannot open (no graphics driver, say).
    pub fn alert(title: &str, text: &str) {
        unsafe {
            MessageBoxW(None, &HSTRING::from(text), &HSTRING::from(title), MB_OK | MB_ICONWARNING);
        }
    }

    /// The installer is a window program, so it has no console of its own. When it was started from one and its
    /// output is not already going to a pipe or file (--quiet, --help, --version), print there.
    pub fn attach_console() {
        use windows::Win32::System::Console::{ATTACH_PARENT_PROCESS, AttachConsole, GetStdHandle, STD_OUTPUT_HANDLE};
        unsafe {
            let out = GetStdHandle(STD_OUTPUT_HANDLE);
            if out.map_or(true, |h| h.is_invalid() || h.0.is_null()) {
                let _ = AttachConsole(ATTACH_PARENT_PROCESS);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sandbox_puts_everything_under_its_root() {
        let root = if cfg!(windows) { PathBuf::from(r"C:\t\sb") } else { PathBuf::from("/t/sb") };
        let p = Places::sandbox(&root, r"Software\SwitchboardTest-ab12").unwrap();
        for dir in [&p.programs, &p.start_menu, &p.desktop, &p.settings, &p.cache, &p.logs] {
            assert!(dir.starts_with(&root), "{}", dir.display());
        }
        for key in [&p.uninstall_key, &p.run_key, &p.aumid_key] {
            assert!(key.starts_with(r"Software\SwitchboardTest-ab12\"), "{key}");
        }
        assert!(p.test);
        assert!(Places::sandbox(&root, r"Software\Microsoft\Windows").is_err());
        assert!(Places::sandbox(&root, r"Software\SwitchboardTest-").is_err());
        assert!(Places::sandbox(&root, r"Software\SwitchboardTest-a\..\..\Microsoft").is_err());
        assert!(Places::sandbox(Path::new("relative"), r"Software\SwitchboardTest-x").is_err());
    }

    #[test]
    fn real_keys_are_per_user() {
        // Only the key paths are checked here: Places::real reads the environment.
        let p = Places::sandbox(Path::new(if cfg!(windows) { r"C:\x" } else { "/x" }), r"Software\SwitchboardTest-x")
            .unwrap();
        assert!(p.uninstall_key.ends_with(&format!(r"\Uninstall\{DESKTOP_ID}")));
        assert_eq!(p.start_shortcut().file_name().unwrap(), format!("{NAME}.lnk").as_str());
    }

    #[test]
    fn settings_apps_entry() {
        let app = Path::new(r"C:\Users\you\AppData\Local\Programs\Switchboard");
        let v = uninstall_values(app, "0.8.4", 41_000);
        let get = |k: &str| v.iter().find(|(n, _)| *n == k).map(|(_, v)| v.clone()).unwrap();
        assert_eq!(get("DisplayName"), Value::Str(NAME.into()));
        assert_eq!(get("DisplayVersion"), Value::Str("0.8.4".into()));
        assert_eq!(get("Publisher"), Value::Str(PUBLISHER.into()));
        let Value::Str(u) = get("UninstallString") else { panic!() };
        assert!(u.starts_with('"') && u.ends_with(&format!("{}\" --uninstall", uninstaller_name())), "{u}");
        let Value::Str(q) = get("QuietUninstallString") else { panic!() };
        assert!(q.ends_with("--uninstall --quiet"));
        assert_eq!(get("EstimatedSize"), Value::Dword(41_000));
        assert_eq!(get("NoModify"), Value::Dword(1));
        assert_eq!(get("NoRepair"), Value::Dword(1));
        for k in ["DisplayIcon", "InstallLocation", "QuietUninstallString"] {
            assert!(v.iter().any(|(n, _)| *n == k), "{k}");
        }
        assert!(run_value(app).starts_with('"') && run_value(app).ends_with(&format!("{}\"", exe_name())));
    }

    #[test]
    fn paths_compare_like_windows() {
        let app = Path::new(r"C:\Users\You\AppData\Local\Programs\Switchboard");
        assert!(inside(Path::new(r"c:\users\you\appdata\local\programs\switchboard\Switchboard.exe"), app));
        assert!(inside(Path::new(r"\\?\C:\Users\You\AppData\Local\Programs\Switchboard\x\y.exe"), app));
        assert!(inside(app, app));
        assert!(!inside(Path::new(r"C:\Users\You\AppData\Local\Programs\Switchboard2\Switchboard.exe"), app));
        assert!(!inside(Path::new(r"C:\Windows\explorer.exe"), app));
    }

    #[test]
    fn folder_sizes_round_up() {
        let d = std::env::temp_dir().join(format!("sb-install-kb-{}", std::process::id()));
        std::fs::create_dir_all(d.join("sub")).unwrap();
        std::fs::write(d.join("a"), vec![0u8; 1500]).unwrap();
        std::fs::write(d.join("sub/b"), vec![0u8; 600]).unwrap();
        assert_eq!(folder_kb(&d), 3);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn self_delete_waits_then_removes_only_its_folder() {
        let c = self_delete_command(Path::new(r"C:\Temp\sb-uninstall-42"));
        assert!(c.starts_with("/d /c ping -n 3 127.0.0.1 >nul & rmdir /s /q "), "{c}");
        assert!(c.ends_with(r#""C:\Temp\sb-uninstall-42""#), "{c}");
    }

    #[test]
    fn graphics_messages() {
        assert_eq!(Graphics::Hardware.warning(), None);
        assert!(Graphics::None.warning().unwrap().contains("DirectX 11"));
        assert!(Graphics::SoftwareOnly.warning().unwrap().contains("draw slowly"));
    }
}
