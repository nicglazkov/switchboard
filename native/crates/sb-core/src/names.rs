//! Session names: "<stamp> <topic> (<tag>)". The server stamps new names with a date and time
//! ("Oct 11 3:40p Topic (part 2)"); the stamp is never shown. See docs/decisions.md.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TagKind {
    Fresh,
    Move,
    Ultra,
    Max,
    /// v2: "for Studio": a dispatched chat doing work another machine's session asked for.
    For,
    Plain,
}

impl TagKind {
    /// Whether a card shows it. Tags that only say how a chat was born ("part 2", "copy", "from Studio") stay off the
    /// card, which shows the chat's live state instead; the details sheet's History says where it came from.
    pub fn on_card(self) -> bool {
        matches!(self, TagKind::For | TagKind::Ultra | TagKind::Max)
    }
}

/// Strips a leading "Oct 11 3:40p " stamp, if there is one.
fn strip_stamp(t: &str) -> &str {
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let b = t.as_bytes();
    if b.len() < 4 || !MONTHS.iter().any(|m| t.starts_with(m)) || b[3] != b' ' {
        return t;
    }
    let mut i = 4;
    let digits = |i: &mut usize, min: usize, max: usize| -> bool {
        let start = *i;
        while *i < b.len() && b[*i].is_ascii_digit() && *i - start < max {
            *i += 1;
        }
        *i - start >= min
    };
    if !digits(&mut i, 1, 2) || i >= b.len() || b[i] != b' ' {
        return t;
    }
    i += 1;
    if !digits(&mut i, 1, 2) || i >= b.len() || b[i] != b':' {
        return t;
    }
    i += 1;
    let s = i;
    if !digits(&mut i, 2, 2) || i - s != 2 {
        return t;
    }
    if i >= b.len() || !(b[i] == b'a' || b[i] == b'p') {
        return t;
    }
    i += 1;
    if i >= b.len() || b[i] != b' ' {
        return t;
    }
    i += 1;
    // the stamp must be followed by a name
    if i >= b.len() || b[i].is_ascii_whitespace() {
        return t;
    }
    &t[i..]
}

/// First non-empty line, stamp removed, capped at a word near 90 characters.
pub fn plain(title: &str) -> String {
    let first = title.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("Untitled session");
    let t = strip_stamp(first);
    if t.chars().count() > 90 {
        let cut: String = t.chars().take(90).collect();
        let at = cut.rfind(' ').map(|i| cut[..i].chars().count()).unwrap_or(0).max(40);
        let mut s: String = cut.chars().take(at).collect();
        while s.ends_with(|c: char| c.is_whitespace() || ".,;:".contains(c)) {
            s.pop();
        }
        s.push_str("...");
        return s;
    }
    t.to_string()
}

/// "Topic (fresh 2)" -> ("Topic", Some("fresh 2")); "Topic fresh" -> ("Topic", Some("fresh")).
pub fn split(title: &str) -> (String, Option<String>) {
    let t = plain(title);
    if t.ends_with(')') {
        if let Some(open) = t.rfind('(') {
            let inner = &t[open + 1..t.len() - 1];
            let head = t[..open].trim_end();
            let n = inner.chars().count();
            if (1..=40).contains(&n) && !inner.contains('(') && !inner.contains(')') && !head.is_empty() {
                return (head.to_string(), Some(inner.to_string()));
            }
        }
    }
    let lower = t.to_lowercase();
    if lower.ends_with(" fresh") || lower.ends_with("\tfresh") {
        let head = t[..t.len() - 5].trim_end();
        if !head.is_empty() {
            return (head.to_string(), Some("fresh".into()));
        }
    }
    (t, None)
}

pub fn tag_kind(tag: Option<&str>) -> Option<TagKind> {
    let tag = tag?;
    if tag.is_empty() {
        return None;
    }
    let l = tag.to_lowercase();
    Some(if l.starts_with("fresh") || l.starts_with("part ") {
        TagKind::Fresh
    } else if l.starts_with("from ") {
        TagKind::Move
    } else if l.starts_with("for ") {
        TagKind::For
    } else if l.contains("ultracode") {
        TagKind::Ultra
    } else if l.contains("max effort") {
        TagKind::Max
    } else {
        TagKind::Plain
    })
}

/// The part number the next fresh start gets: no tag -> 2, "part 2" -> 3. Older names: "fresh" -> 3, "fresh 2" -> 4.
pub fn fresh_n(tag: Option<&str>) -> u32 {
    let Some(tag) = tag else { return 2 };
    let l = tag.to_lowercase();
    if l == "fresh" {
        return 3;
    }
    if let Some(n) = l.strip_prefix("fresh ").and_then(|n| n.parse::<u32>().ok()) {
        return n + 2;
    }
    if let Some(n) = l.strip_prefix("part ").and_then(|n| n.parse::<u32>().ok()) {
        return n + 1;
    }
    2
}

/// The topic the server will likely pick for a new chat: the first words of the message, or the folder.
pub fn topic_from(msg: &str, folder: &str) -> String {
    let cleaned: String = msg
        .chars()
        .map(|c| if c.is_alphanumeric() || c.is_whitespace() || "'./-".contains(c) { c } else { ' ' })
        .collect();
    let mut words: Vec<&str> = cleaned.split_whitespace().take(7).collect();
    const STOP: [&str; 19] =
        ["a", "an", "the", "to", "of", "for", "in", "on", "at", "by", "and", "or", "with", "from", "into", "that", "so", "is", "my"];
    while words.len() > 2 {
        let last = words[words.len() - 1].to_lowercase();
        if STOP.contains(&last.as_str()) || last == "our" {
            words.pop();
        } else {
            break;
        }
    }
    if !words.is_empty() {
        let t = words.join(" ");
        let mut c = t.chars();
        return match c.next() {
            Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
            None => t,
        };
    }
    if !folder.is_empty() {
        return crate::fmt::last_seg(folder);
    }
    "New session".into()
}

pub fn fresh(topic: &str, n: u32) -> String {
    format!("{topic} (part {})", n.max(2))
}

pub fn from_machine(topic: &str, label: &str) -> String {
    format!("{topic} (from {label})")
}

pub fn resumed(topic: &str) -> String {
    format!("{topic} (copy)")
}

/// A new chat's name: only its run mode is tagged ("UltraCode", "Max effort"); a normal one has no tag.
pub fn created(topic: &str, mode: &str) -> String {
    match mode {
        "ultracode" => format!("{topic} (UltraCode)"),
        "max" => format!("{topic} (Max effort)"),
        _ => topic.to_string(),
    }
}

/// The longest name a chat can be given (the machines refuse longer ones).
pub const NAME_MAX: usize = 64;

/// What a typed name becomes: letters, digits, spaces and . , : ' + # ( ) - _ only (anything else turns into a space,
/// as the machines do), spaces squeezed, cut at a word to fit `room`.
pub fn clean_name(typed: &str, room: usize) -> String {
    let ok: String = typed.chars().map(|c| if c.is_ascii_alphanumeric() || " .,:'+#()-_".contains(c) { c } else { ' ' }).collect();
    let t = ok.split_whitespace().collect::<Vec<_>>().join(" ");
    if t.len() <= room {
        return t;
    }
    // ASCII only from here, so byte offsets are characters
    let cut = &t[..room + 1];
    let at = cut.rfind(' ').filter(|i| *i > room / 2).unwrap_or(room);
    cut[..at].trim_end_matches([' ', '.', ',', ':', '-']).to_string()
}

/// Tags that say where a chat came from, kept when it is renamed ("part 2", "from Studio", "UltraCode", "copy").
fn lineage_tag(tag: &str) -> bool {
    let l = tag.to_lowercase();
    matches!(tag_kind(Some(tag)), Some(k) if k != TagKind::Plain) || l == "copy" || l == "resumed"
}

/// The name sent for a chat renamed to `typed`: the typed words, then the tag it had, if that tag says where it came
/// from (so "Topic (part 2)" renamed to "Widget fix" is "Widget fix (part 2)"). Empty when nothing is typed: the chat
/// goes back to its automatic name.
pub fn given(typed: &str, old_raw: &str) -> String {
    let tag = split(old_raw).1.filter(|t| lineage_tag(t));
    let tail = tag.map(|t| format!(" ({t})")).unwrap_or_default();
    let topic = clean_name(typed, NAME_MAX.saturating_sub(tail.len()));
    if topic.is_empty() {
        return String::new();
    }
    format!("{topic}{tail}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn given_names() {
        assert_eq!(given("  Widget   fix ", "Oct 11 3:40p Old topic (part 2)"), "Widget fix (part 2)");
        assert_eq!(given("Widget fix", "Old topic (from Studio)"), "Widget fix (from Studio)");
        assert_eq!(given("Widget fix", "Old topic (copy)"), "Widget fix (copy)");
        assert_eq!(given("Widget fix", "Fix (the) thing (v2)"), "Widget fix");
        assert_eq!(given("Widget/fix & more", "x"), "Widget fix more");
        assert_eq!(given("Caf\u{e9} notes", "x"), "Caf notes");
        assert_eq!(given("   ", "Old (part 2)"), "");
        let long = given(&"word ".repeat(30), "Old (part 12)");
        assert!(long.len() <= NAME_MAX && long.ends_with("word (part 12)"), "{long}");
        assert_eq!(clean_name(&"x".repeat(80), NAME_MAX).len(), NAME_MAX);
    }

    #[test]
    fn stamps_and_tags() {
        assert_eq!(plain("Oct 11 3:40p Folder audit (fresh)"), "Folder audit (fresh)");
        assert_eq!(split("Oct 11 3:40p Folder audit (fresh)"), ("Folder audit".into(), Some("fresh".into())));
        assert_eq!(split("Claude plan usage and session management fresh"), ("Claude plan usage and session management".into(), Some("fresh".into())));
        assert_eq!(split("labv2"), ("labv2".into(), None));
        assert_eq!(split("Oct 1 9:05a x"), ("x".into(), None));
        assert_eq!(plain("Oct 1 9:05a "), "Oct 1 9:05a");
        assert_eq!(plain("\n\n  first line\nsecond"), "first line");
        assert_eq!(plain(""), "Untitled session");
        assert_eq!(split("Kitchen remodel (from Laptop)").1.as_deref(), Some("from Laptop"));
        assert_eq!(tag_kind(Some("from Laptop")), Some(TagKind::Move));
        assert_eq!(tag_kind(Some("UltraCode, web")), Some(TagKind::Ultra));
        assert_eq!(tag_kind(Some("Max effort, iPhone")), Some(TagKind::Max));
        assert_eq!(tag_kind(Some("fresh 3")), Some(TagKind::Fresh));
        assert_eq!(tag_kind(Some("resumed")), Some(TagKind::Plain));
        assert_eq!(tag_kind(Some("part 2")), Some(TagKind::Fresh));
        assert_eq!(tag_kind(Some("copy")), Some(TagKind::Plain));
        assert!(!TagKind::Fresh.on_card() && !TagKind::Move.on_card() && !TagKind::Plain.on_card());
        assert!(TagKind::For.on_card() && TagKind::Ultra.on_card() && TagKind::Max.on_card());
        assert_eq!(tag_kind(Some("for Studio")), Some(TagKind::For));
        assert_eq!(split("Build the Android app (for Studio)"), ("Build the Android app".to_string(), Some("for Studio".to_string())));
        assert_eq!(tag_kind(None), None);
    }

    #[test]
    fn long_titles_are_cut_at_a_word() {
        let long = "word ".repeat(40);
        let p = plain(&long);
        assert!(p.ends_with("..."));
        assert!(p.chars().count() <= 93);
        assert!(!p.contains("  "));
    }

    #[test]
    fn new_names() {
        assert_eq!(fresh_n(None), 2);
        assert_eq!(fresh_n(Some("fresh")), 3);
        assert_eq!(fresh_n(Some("fresh 2")), 4);
        assert_eq!(fresh_n(Some("part 3")), 4);
        assert_eq!(fresh_n(Some("copy")), 2);
        assert_eq!(fresh("Topic", 2), "Topic (part 2)");
        assert_eq!(fresh("Topic", 4), "Topic (part 4)");
        assert_eq!(resumed("Topic"), "Topic (copy)");
        assert_eq!(created("Topic", "normal"), "Topic");
        assert_eq!(from_machine("Topic", "Studio"), "Topic (from Studio)");
        assert_eq!(created("Fix the login bug", "ultracode"), "Fix the login bug (UltraCode)");
        assert_eq!(topic_from("fix the login bug in the app for", ""), "Fix the login bug in the app");
        assert_eq!(topic_from("  ", "Developer/notebook-cli"), "notebook-cli");
        assert_eq!(topic_from("", ""), "New session");
        assert_eq!(topic_from("what's up? :) ok", ""), "What's up ok");
    }
}
