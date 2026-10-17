//! A crash leaves a note: the panic's message, where it happened and a backtrace go to the log (src/logs.rs) and to a
//! crash file of its own in the same folder, crash-<local time>.txt, so the next crash can be read. Release builds
//! keep their symbols outside the app (docs/building.md says how to read a backtrace with them). The 10 newest crash
//! files are kept. After the note the default hook still runs, so the process ends as it always did.
use std::path::{Path, PathBuf};

/// Crash files kept.
const KEEP: usize = 10;

/// One note: when, which version and build, which thread, the message with its place, where the app was loaded and
/// the backtrace with raw addresses (a stripped build has no names; the symbols turn address minus load address into
/// a function and line).
pub fn note(info: &std::panic::PanicHookInfo<'_>) -> String {
    let msg = info
        .payload()
        .downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| info.payload().downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "(no message)".into());
    let at = info.location().map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column())).unwrap_or_default();
    let when = chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f %:z");
    let thread = std::thread::current().name().unwrap_or("unnamed").to_string();
    let build = sb_core::update::this_build().unwrap_or("development");
    let base = image_base().map_or_else(|| "unknown".into(), |b| format!("{b:#x}"));
    let bt = std::backtrace::Backtrace::force_capture();
    crate::logs::scrub(&format!(
        "Switchboard {} (build {build}, {} {}) panicked at {when} on thread {thread}\n{msg}\nat {at}\n\
         Load address: {base}\n{bt:#}\n",
        sb_core::version(),
        std::env::consts::OS,
        std::env::consts::ARCH,
    ))
}

/// Where the running executable starts in memory this run (ASLR moves it every launch).
#[cfg(target_os = "macos")]
pub fn image_base() -> Option<usize> {
    unsafe extern "C" {
        fn _dyld_get_image_header(index: u32) -> *const std::ffi::c_void;
    }
    // SAFETY: image 0 is always the main executable; the call only reads dyld's table.
    let h = unsafe { _dyld_get_image_header(0) };
    (!h.is_null()).then_some(h as usize)
}

#[cfg(windows)]
pub fn image_base() -> Option<usize> {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetModuleHandleW(name: *const u16) -> *mut std::ffi::c_void;
    }
    // SAFETY: a null name asks for the exe's own module handle, which is its base address.
    let h = unsafe { GetModuleHandleW(std::ptr::null()) };
    (!h.is_null()).then_some(h as usize)
}

/// Linux: the first mapping of the executable in /proc/self/maps.
#[cfg(not(any(target_os = "macos", windows)))]
pub fn image_base() -> Option<usize> {
    let exe = std::fs::read_link("/proc/self/exe").ok()?;
    let exe = exe.to_str()?;
    let maps = std::fs::read_to_string("/proc/self/maps").ok()?;
    let line = maps.lines().find(|l| l.split_whitespace().nth(5) == Some(exe))?;
    usize::from_str_radix(line.split('-').next()?, 16).ok()
}

/// Writes the crash file and drops the oldest past KEEP. Returns its path.
pub fn write_crash(dir: &Path, text: &str) -> Option<PathBuf> {
    std::fs::create_dir_all(dir).ok()?;
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S%.3f");
    let path = dir.join(format!("crash-{stamp}.txt"));
    std::fs::write(&path, text).ok()?;
    let mut old: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("crash-") && n.ends_with(".txt"))
        })
        .collect();
    // The stamp sorts by time.
    old.sort();
    for p in old.iter().take(old.len().saturating_sub(KEEP)) {
        let _ = std::fs::remove_file(p);
    }
    Some(path)
}

/// Installs the hook; the default one still runs after it (it prints to stderr).
pub fn install() {
    install_in(crate::logs::dir());
}

fn install_in(dir: Option<PathBuf>) {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let text = note(info);
        log::error!("{}", text.trim_end());
        log::logger().flush();
        if let Some(d) = &dir {
            write_crash(d, &text);
        }
        prev(info);
    }));
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_panic_leaves_a_note() {
        let dir = std::env::temp_dir().join(format!("sb-panic-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        super::install_in(Some(dir.clone()));
        let r = std::thread::Builder::new().name("t".into()).spawn(|| panic!("boom at the start")).unwrap().join();
        assert!(r.is_err());
        let crash = std::fs::read_dir(&dir).unwrap().next().unwrap().unwrap().path();
        assert!(crash.file_name().unwrap().to_string_lossy().starts_with("crash-"), "{crash:?}");
        let text = std::fs::read_to_string(&crash).unwrap();
        assert!(text.contains("boom at the start"), "{text}");
        assert!(text.contains("panics.rs"), "{text}");
        assert!(text.contains("thread t"), "{text}");
        assert!(text.contains("Load address: 0x"), "{text}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn old_crash_files_go() {
        let dir = std::env::temp_dir().join(format!("sb-crash-keep-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for i in 0..14 {
            std::fs::write(dir.join(format!("crash-20260101-0000{i:02}.000.txt")), "x").unwrap();
        }
        std::fs::write(dir.join("Switchboard.log"), "keep").unwrap();
        super::write_crash(&dir, "new").unwrap();
        let mut names: Vec<String> =
            std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        assert_eq!(names.len(), super::KEEP + 1, "{names:?}");
        assert!(names.contains(&"Switchboard.log".to_string()));
        assert!(!names.contains(&"crash-20260101-000000.000.txt".to_string()));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
