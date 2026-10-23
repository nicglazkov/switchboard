//! What names the app, from /branding.toml at the repo root (read by build.rs when this crate builds).

/// The desktop app's id (`[app] desktop_id`). Names its cache and settings folders on macOS.
pub const DESKTOP_ID: &str = env!("SB_DESKTOP_ID");

/// Where the project lives (`[app] homepage`).
pub const HOMEPAGE: &str = env!("SB_HOMEPAGE");

/// The documentation site (`[app] docs`), ending in "/". Help links open pages under it.
pub const DOCS: &str = env!("SB_DOCS");

/// A page of the documentation site: `docs("install/")`, `docs("updates/#where-updates-come-from")`.
pub fn docs(page: &str) -> String {
    format!("{}/{page}", DOCS.trim_end_matches('/'))
}

#[cfg(test)]
mod tests {
    #[test]
    fn read_from_branding_toml() {
        assert!(super::DESKTOP_ID.contains('.') && !super::DESKTOP_ID.contains('"'));
        assert!(super::HOMEPAGE.starts_with("https://"));
        assert!(super::DOCS.starts_with("https://") && super::DOCS.ends_with('/'));
        assert_eq!(super::docs("install/"), format!("{}install/", super::DOCS));
    }
}
