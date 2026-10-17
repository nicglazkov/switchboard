//! A crash leaves a note: the panic's message, where it happened and a backtrace go to panic.log, so the next crash
//! can be read (release builds are stripped, so the message and its file and line are what a crash report lacks).
//!   macOS    ~/Library/Logs/Switchboard/panic.log
//!   Windows  %LOCALAPPDATA%\Switchboard\Logs\panic.log
//!   Linux    $XDG_STATE_HOME/switchboard/panic.log (~/.local/state/switchboard/panic.log)
use std::io::Write;
use std::path::PathBuf;

/// The file the notes go to (SB_PANIC_LOG overrides it, for tests).
pub fn log_path() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("SB_PANIC_LOG") {
        return Some(PathBuf::from(p));
    }
    let home = || std::env::var_os("HOME").map(PathBuf::from);
    if cfg!(target_os = "macos") {
        home().map(|h| h.join("Library/Logs/Switchboard/panic.log"))
    } else if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA").map(|d| PathBuf::from(d).join("Switchboard").join("Logs").join("panic.log"))
    } else {
        std::env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .or_else(|| home().map(|h| h.join(".local/state")))
            .map(|d| d.join("switchboard/panic.log"))
    }
}

/// One note: when, which version, which thread, the message with its place, and the backtrace.
pub fn note(info: &std::panic::PanicHookInfo<'_>) -> String {
    let msg = info
        .payload()
        .downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| info.payload().downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "(no message)".into());
    let at = info.location().map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column())).unwrap_or_default();
    let when = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let thread = std::thread::current().name().unwrap_or("unnamed").to_string();
    let bt = std::backtrace::Backtrace::force_capture();
    format!("--- Switchboard {} panicked (unix time {when}, thread {thread})\n{msg}\nat {at}\n{bt}\n", sb_core::version())
}

/// Installs the hook; the default one still runs after it (it prints to stderr).
pub fn install() {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let text = note(info);
        if let Some(p) = log_path() {
            if let Some(dir) = p.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            // Keep the file small: past 1 MB, start over.
            if std::fs::metadata(&p).is_ok_and(|m| m.len() > 1 << 20) {
                let _ = std::fs::remove_file(&p);
            }
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&p) {
                let _ = f.write_all(text.as_bytes());
            }
        }
        prev(info);
    }));
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_panic_leaves_a_note() {
        let dir = std::env::temp_dir().join(format!("sb-panic-{}", std::process::id()));
        let path = dir.join("panic.log");
        // SAFETY: tests in this module run alone in their own process for this variable; nothing else reads it.
        unsafe { std::env::set_var("SB_PANIC_LOG", &path) };
        super::install();
        let r = std::thread::Builder::new().name("t".into()).spawn(|| panic!("boom at the start")).unwrap().join();
        assert!(r.is_err());
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("boom at the start"), "{text}");
        assert!(text.contains("panics.rs"), "{text}");
        assert!(text.contains("thread t"), "{text}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
