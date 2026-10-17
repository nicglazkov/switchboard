//! The shared terminal color schemes (schemes.json, the same file the server and the phone use) and the synced
//! terminal look (/api/prefs `appearance.term`): scheme, text size, cursor style and blink.

use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

/// A scheme as schemes.json writes it: colors as "#RRGGBB" (or "#RRGGBBAA" for the selection).
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Scheme {
    pub id: String,
    pub name: String,
    pub dark: bool,
    pub bg: String,
    pub fg: String,
    pub cursor: String,
    #[serde(rename = "cursorText", default)]
    pub cursor_text: Option<String>,
    #[serde(default)]
    pub sel: Option<String>,
    pub ansi: Vec<String>,
}

#[derive(Deserialize)]
struct File {
    #[serde(rename = "match")]
    matching: Match,
    schemes: Vec<Scheme>,
}

#[derive(Debug, Clone, Deserialize)]
struct Match {
    dark: String,
    light: String,
}

/// "Match the app": not a scheme, the app's own dark or light one.
pub const MATCH: &str = "match";
/// What a fresh install uses. Claude Code paints for a dark terminal, so the default is a dark scheme.
pub const DEFAULT: &str = "converge-dark";

fn file() -> &'static File {
    static F: OnceLock<File> = OnceLock::new();
    F.get_or_init(|| serde_json::from_str(include_str!("schemes.json")).expect("schemes.json is valid"))
}

/// Every scheme, in the file's order.
pub fn all() -> &'static [Scheme] {
    &file().schemes
}

pub fn by_id(id: &str) -> Option<&'static Scheme> {
    all().iter().find(|s| s.id == id)
}

/// The scheme "Match the app" stands for.
pub fn matching(dark: bool) -> &'static Scheme {
    let m = &file().matching;
    by_id(if dark { &m.dark } else { &m.light }).unwrap_or(&all()[0])
}

/// A stored choice made valid: a known id or "match". Older desktop builds saved "match-app" and "switchboard-*".
pub fn normalize(id: Option<&str>) -> String {
    match id {
        Some(MATCH) | Some("match-app") => MATCH.into(),
        Some("switchboard-dark") | None => DEFAULT.into(),
        Some("switchboard-light") => "converge-light".into(),
        Some(x) if by_id(x).is_some() => x.into(),
        Some(_) => DEFAULT.into(),
    }
}

/// "#RRGGBB" or "#RRGGBBAA" to (r, g, b, a) in 0..=255.
pub fn hex(s: &str) -> Option<(u8, u8, u8, u8)> {
    let h = s.strip_prefix('#')?;
    let byte = |i: usize| u8::from_str_radix(h.get(i..i + 2)?, 16).ok();
    match h.len() {
        6 => Some((byte(0)?, byte(2)?, byte(4)?, 255)),
        8 => Some((byte(0)?, byte(2)?, byte(4)?, byte(6)?)),
        _ => None,
    }
}

/// Text sizes the terminal offers (the server accepts 9 to 24).
pub const SIZES: std::ops::RangeInclusive<u16> = 9..=24;
pub const SIZE_DEFAULT: u16 = 13;

/// The synced terminal look, as /api/prefs carries it under `appearance.term`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TermLook {
    #[serde(default = "match_id")]
    pub scheme: String,
    /// Desktop text size; None is the default.
    #[serde(default)]
    pub size: Option<u16>,
    /// The phone's own size (kept as it is when the desktop saves).
    #[serde(default)]
    pub size_phone: Option<u16>,
    /// block | bar | underline
    #[serde(default = "block")]
    pub cursor: String,
    #[serde(default = "yes")]
    pub blink: bool,
}

fn match_id() -> String {
    MATCH.into()
}
fn block() -> String {
    "block".into()
}
fn yes() -> bool {
    true
}

impl Default for TermLook {
    fn default() -> Self {
        TermLook { scheme: DEFAULT.into(), size: None, size_phone: None, cursor: block(), blink: true }
    }
}

/// /api/prefs `appearance`: the app's theme (system, light or dark) and the terminal's look.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Appearance {
    #[serde(default)]
    pub theme: Option<String>,
    #[serde(default)]
    pub term: Option<TermLook>,
}

/// One step of Cmd+Plus or Cmd+Minus from `size` (None is the default).
pub fn step_size(size: Option<u16>, up: bool) -> u16 {
    let s = size.unwrap_or(SIZE_DEFAULT);
    let s = if up { s.saturating_add(1) } else { s.saturating_sub(1) };
    s.clamp(*SIZES.start(), *SIZES.end())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shared_list_parses_and_the_default_is_dark() {
        assert_eq!(all().len(), 12);
        assert!(by_id(DEFAULT).is_some_and(|s| s.dark), "Claude Code paints for a dark terminal");
        assert!(by_id(MATCH).is_none());
        for s in all() {
            assert_eq!(s.ansi.len(), 16, "{}", s.id);
            for c in s.ansi.iter().chain([&s.bg, &s.fg, &s.cursor]) {
                assert!(hex(c).is_some(), "{} {c}", s.id);
            }
        }
        assert!(!matching(false).dark);
        assert!(matching(true).dark);
    }

    #[test]
    fn old_desktop_choices_carry_over() {
        assert_eq!(normalize(Some("match-app")), MATCH);
        assert_eq!(normalize(Some("switchboard-dark")), DEFAULT);
        assert_eq!(normalize(Some("nord")), "nord");
        assert_eq!(normalize(Some("gone")), DEFAULT);
        assert_eq!(normalize(None), DEFAULT);
    }

    #[test]
    fn sizes_step_and_stop_at_the_ends() {
        assert_eq!(step_size(None, true), 14);
        assert_eq!(step_size(Some(24), true), 24);
        assert_eq!(step_size(Some(9), false), 9);
        assert_eq!(hex("#6CB6FF57"), Some((0x6C, 0xB6, 0xFF, 0x57)));
    }

    #[test]
    fn appearance_reads_what_the_server_sends() {
        let a: Appearance = serde_json::from_str(
            r#"{"theme":"dark","term":{"scheme":"nord","size":null,"size_phone":12,"cursor":"bar","blink":false}}"#,
        )
        .unwrap();
        let t = a.term.unwrap();
        assert_eq!(
            (a.theme.as_deref(), t.scheme.as_str(), t.size_phone, t.cursor.as_str(), t.blink),
            (Some("dark"), "nord", Some(12), "bar", false)
        );
    }
}
