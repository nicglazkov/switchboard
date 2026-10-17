//! Which web pages may open a terminal. Native apps send no Origin; a browser always does, and only the
//! Switchboard server's own pages are let in: the origins the server lists in machines.json (from its own config),
//! plus SWITCHBOARD_ORIGINS, for a test server on another port. Nothing is built in.

/// Built-in origins: none. The server says which pages are its own.
pub const DEFAULT_ORIGINS: &[&str] = &[];

pub fn normalize(o: &str) -> String {
    o.trim().trim_end_matches('/').to_ascii_lowercase()
}

/// Comma or space separated, as in SWITCHBOARD_ORIGINS.
pub fn parse_list(s: &str) -> Vec<String> {
    s.split(|c: char| c == ',' || c.is_whitespace()).map(normalize).filter(|o| !o.is_empty()).collect()
}

pub fn allowed(origin: Option<&str>, list: &[String]) -> bool {
    match origin {
        None => true,
        Some(o) => {
            let o = normalize(o);
            // "null" is what sandboxed frames and file:// pages send
            o != "null" && !o.is_empty() && list.iter().any(|a| normalize(a) == o)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list() -> Vec<String> {
        let mut l: Vec<String> = DEFAULT_ORIGINS.iter().map(|s| s.to_string()).collect();
        l.extend(parse_list("http://10.0.0.5:8130, http://100.64.0.9:8130, http://10.0.0.5:8150, http://localhost:8151"));
        l
    }

    #[test]
    fn native_apps_send_none() {
        assert!(allowed(None, &list()));
    }

    #[test]
    fn own_pages_and_extras() {
        let l = list();
        assert!(allowed(Some("http://10.0.0.5:8130"), &l));
        assert!(allowed(Some("http://100.64.0.9:8130"), &l));
        assert!(allowed(Some("HTTP://10.0.0.5:8130/"), &l));
        assert!(allowed(Some("http://10.0.0.5:8150"), &l));
        assert!(allowed(Some("http://localhost:8151"), &l));
    }

    #[test]
    fn everything_else_is_refused() {
        let l = list();
        for o in ["http://evil.example", "https://10.0.0.5:8130", "http://10.0.0.5:8131", "http://10.0.0.5",
            "null", "", "http://10.0.0.5:8130.evil.example"] {
            assert!(!allowed(Some(o), &l), "{o}");
        }
    }

    #[test]
    fn nothing_is_built_in() {
        assert!(DEFAULT_ORIGINS.is_empty());
        assert!(!allowed(Some("http://10.0.0.5:8130"), &[]));
        assert!(allowed(None, &[]));
    }

    #[test]
    fn parse_list_skips_blanks() {
        assert_eq!(parse_list(" a ,, b c"), vec!["a", "b", "c"]);
        assert!(parse_list("").is_empty());
    }
}
