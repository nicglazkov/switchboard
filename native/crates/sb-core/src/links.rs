//! Links into Claude.

/// The Claude desktop app's own link for a Claude Code session on claude.ai (`claude://code/session_…`), or None for
/// any other link. The app's handler (Claude 2.x, `claudeURLHandler`) takes `session_` and `cse_` ids: it opens its
/// local copy of the session when it has one, and a viewer of the remote session otherwise.
pub fn claude_app_link(url: &str) -> Option<String> {
    let rest = url.strip_prefix("https://claude.ai/code/")?;
    let id = rest.split(['?', '#', '/']).next()?;
    let (kind, tail) = id.split_once('_')?;
    let ok = matches!(kind, "session" | "cse")
        && !tail.is_empty()
        && tail.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    ok.then(|| format!("claude://code/{id}"))
}
