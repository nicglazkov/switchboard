//! What names the app, from /branding.toml at the repo root (read by build.rs).

/// The app's name (`[app] name`): the bundle is `<NAME>.app`.
pub const NAME: &str = env!("SB_APP_NAME");
/// The desktop app's bundle id (`[app] desktop_id`). Names its settings and cache folders.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub const DESKTOP_ID: &str = env!("SB_DESKTOP_ID");
/// The login item's launchd label (`[app] launcher_id`).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub const LAUNCHER_ID: &str = env!("SB_LAUNCHER_ID");
/// Where the project lives (`[app] homepage`).
pub const HOMEPAGE: &str = env!("SB_HOMEPAGE");

/// The app bundle's folder name.
pub fn bundle() -> String {
    format!("{NAME}.app")
}

#[cfg(test)]
mod tests {
    #[test]
    fn read_from_branding_toml() {
        assert!(!super::NAME.is_empty() && !super::NAME.contains('"'));
        let prefix = super::DESKTOP_ID.rsplit_once('.').unwrap().0;
        assert!(super::LAUNCHER_ID.starts_with(prefix));
        assert_eq!(super::bundle(), format!("{}.app", super::NAME));
    }
}
