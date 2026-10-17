//! Links in the terminal text: web addresses and file paths under the mouse. Links a program sets itself (OSC 8) are
//! read from the cells in mod.rs; this finds the ones that are only text.

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    Url(String),
    /// A path on the chat's machine, with any ":line:col" after it.
    Path(String),
}

/// A link covering the cells `start..end` of one row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    pub start: usize,
    pub end: usize,
    pub target: Target,
}

/// Characters that never belong to a link: they end one.
fn stops(c: char) -> bool {
    c.is_whitespace() || c == '\0' || matches!(c, '"' | '\'' | '`' | '<' | '>' | '|' | '│' | '┃' | '║' | '{' | '}' | '^')
}

const SCHEMES: [&str; 5] = ["https://", "http://", "file://", "ftp://", "ssh://"];

/// The link at column `col` of a row, one char per cell.
pub fn find(cells: &[char], col: usize) -> Option<Found> {
    if col >= cells.len() || stops(cells[col]) {
        return None;
    }
    let mut a = col;
    while a > 0 && !stops(cells[a - 1]) {
        a -= 1;
    }
    let mut z = col + 1;
    while z < cells.len() && !stops(cells[z]) {
        z += 1;
    }
    let word: String = cells[a..z].iter().collect();
    let chars: Vec<char> = word.chars().collect();

    // A web address may follow other text in the same word ("see:https://..."): start at its scheme.
    let lower = word.to_ascii_lowercase();
    let at = SCHEMES.iter().filter_map(|s| lower.find(s)).min().or_else(|| lower.find("www.").filter(|&i| i == 0 || !chars[i - 1].is_alphanumeric()));
    let (mut s, mut e) = (0usize, chars.len());
    let url = if let Some(i) = at {
        s = lower[..i].chars().count();
        true
    } else {
        // A path has no brackets or commas of its own: "Update(src/a.ts)" is the path inside.
        let edge = |c: char| matches!(c, '(' | ')' | '[' | ']' | ',' | '=');
        let k = col - a;
        if edge(chars[k]) {
            return None;
        }
        while s < k && chars[..k].iter().skip(s).any(|&c| edge(c)) {
            s += 1;
        }
        e = (k..chars.len()).find(|&i| edge(chars[i])).unwrap_or(chars.len());
        false
    };
    // Leading brackets and trailing punctuation are sentence, not link. A ")" stays when the link opened one.
    while s < e && matches!(chars[s], '(' | '[') {
        s += 1;
    }
    loop {
        if s >= e {
            return None;
        }
        let c = chars[e - 1];
        let open = |o: char| chars[s..e].iter().filter(|&&x| x == o).count();
        let close = |x: char| chars[s..e].iter().filter(|&&y| y == x).count();
        let trim = match c {
            '.' | ',' | ';' | ':' | '!' | '?' | '*' => true,
            ')' => close(')') > open('('),
            ']' => close(']') > open('['),
            _ => false,
        };
        if !trim {
            break;
        }
        e -= 1;
    }
    let (start, end) = (a + s, a + e);
    if col < start || col >= end {
        return None;
    }
    let text: String = chars[s..e].iter().collect();
    if url {
        let host = text.split("://").nth(1).unwrap_or(&text);
        if host.is_empty() || !host.chars().next().is_some_and(|c| c.is_alphanumeric() || c == '/' || c == '[') {
            return None;
        }
        let target = if text.to_ascii_lowercase().starts_with("www.") { format!("https://{text}") } else { text };
        return Some(Found { start, end, target: Target::Url(target) });
    }
    is_path(&text).then(|| Found { start, end, target: Target::Path(text) })
}

/// A file path: absolute, from home, relative with ./ or ../, or folders down to a file with an extension
/// ("src/main.rs:42"). Fractions, dates and lone slashes are not.
fn is_path(t: &str) -> bool {
    let bare = t.trim_end_matches(|c: char| c.is_ascii_digit() || c == ':');
    if bare.len() < 2 || !bare.contains('/') || bare.contains("//") {
        return false;
    }
    if bare.starts_with('/') || bare.starts_with("~/") || bare.starts_with("./") || bare.starts_with("../") {
        return bare.chars().any(|c| c.is_alphabetic());
    }
    let last = bare.rsplit('/').next().unwrap_or("");
    let first = bare.split('/').next().unwrap_or("");
    match last.rsplit_once('.') {
        Some((name, ext)) => {
            !name.is_empty() && (1..=8).contains(&ext.len()) && ext.chars().all(|c| c.is_ascii_alphanumeric()) && ext.chars().any(|c| c.is_alphabetic())
                && first.chars().any(|c| c.is_alphabetic())
        }
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(s: &str, needle: &str) -> Option<Found> {
        let cells: Vec<char> = s.chars().collect();
        let col = s[..s.find(needle).unwrap()].chars().count();
        find(&cells, col)
    }

    fn url(s: &str, needle: &str) -> Option<String> {
        match at(s, needle)?.target {
            Target::Url(u) => Some(u),
            _ => None,
        }
    }

    fn path(s: &str, needle: &str) -> Option<String> {
        match at(s, needle)?.target {
            Target::Path(p) => Some(p),
            _ => None,
        }
    }

    #[test]
    fn web_addresses() {
        assert_eq!(url("open https://example.com/a?b=1 now", "example").as_deref(), Some("https://example.com/a?b=1"));
        assert_eq!(url("(see https://example.com/x).", "example").as_deref(), Some("https://example.com/x"));
        assert_eq!(url("https://en.wikipedia.org/wiki/Rust_(language), ok", "wiki").as_deref(), Some("https://en.wikipedia.org/wiki/Rust_(language)"));
        assert_eq!(url("at www.example.com.", "example").as_deref(), Some("https://www.example.com"));
        assert_eq!(url("Continue at https://claude.ai/code/session_01DM", "claude").as_deref(), Some("https://claude.ai/code/session_01DM"));
        assert_eq!(url("url:http://x.io/p", "x.io").as_deref(), Some("http://x.io/p"));
        assert_eq!(url("│https://a.b/c│", "a.b").as_deref(), Some("https://a.b/c"));
        assert!(at("https:// nothing", "https").is_none());
        // the word in front of the address is not part of it
        assert!(at("url:http://x.io/p", "url").is_none());
    }

    #[test]
    fn file_paths() {
        assert_eq!(path("edit ./src/main.rs:42 now", "src").as_deref(), Some("./src/main.rs:42"));
        assert_eq!(path("in ~/notes/todo.md.", "notes").as_deref(), Some("~/notes/todo.md"));
        assert_eq!(path("Update(src/sync/client.ts)", "sync").as_deref(), Some("src/sync/client.ts"));
        assert_eq!(path("ls /usr/local/bin", "local").as_deref(), Some("/usr/local/bin"));
        assert!(at("half is 1/2 and 3/4.5", "1/2").is_none());
        assert!(at("and/or", "and").is_none());
        assert!(at("a / b", "/").is_none());
        assert!(at("plain words", "words").is_none());
    }

    #[test]
    fn the_range_is_the_link_only() {
        let f = at("go (https://a.io/x).", "a.io").unwrap();
        assert_eq!((f.start, f.end), (4, 18));
        let cells: Vec<char> = "go (https://a.io/x).".chars().collect();
        assert!(find(&cells, 3).is_none());
        assert!(find(&cells, 18).is_none());
    }
}
