//! What names the app, from /branding.toml at the repo root (read by build.rs when this crate builds).

/// The desktop app's id (`[app] desktop_id`). Names its cache and settings folders on macOS.
pub const DESKTOP_ID: &str = env!("SB_DESKTOP_ID");

/// Where the project lives (`[app] homepage`): the setup guide is its README.
pub const HOMEPAGE: &str = env!("SB_HOMEPAGE");

#[cfg(test)]
mod tests {
    #[test]
    fn read_from_branding_toml() {
        assert!(super::DESKTOP_ID.contains('.') && !super::DESKTOP_ID.contains('"'));
        assert!(super::HOMEPAGE.starts_with("https://"));
    }
}
