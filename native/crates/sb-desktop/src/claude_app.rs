//! Is the Claude desktop app on this computer? Then "Open in Claude" opens the session there instead of in a browser
//! tab (see `sb_core::links`). Checked once per run; SWITCHBOARD_OPEN=browser always uses the browser.

use std::sync::OnceLock;

pub fn installed() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var("SWITCHBOARD_OPEN").map_or(true, |v| v != "browser") && detect())
}

/// The link to open: the Claude app's own link for a session when the app is here, the web link otherwise.
pub fn link_for(url: &str) -> String {
    match sb_core::links::claude_app_link(url) {
        Some(app) if installed() => app,
        _ => url.to_string(),
    }
}

#[cfg(target_os = "macos")]
fn detect() -> bool {
    let home = std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join("Applications/Claude.app"));
    std::path::Path::new("/Applications/Claude.app").exists() || home.is_some_and(|p| p.exists())
}

/// The installer registers the claude: scheme for the user (the Store build too), so HKCR has it.
#[cfg(target_os = "windows")]
fn detect() -> bool {
    windows_registry::CLASSES_ROOT.open("claude").is_ok()
}

/// No official Linux app, but community builds register x-scheme-handler/claude.
#[cfg(all(unix, not(target_os = "macos")))]
fn detect() -> bool {
    std::process::Command::new("xdg-mime")
        .args(["query", "default", "x-scheme-handler/claude"])
        .output()
        .is_ok_and(|o| o.status.success() && !o.stdout.trim_ascii().is_empty())
}
