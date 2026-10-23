//! Folder addresses, the way the apps name a folder on a machine (never by a full path):
//!
//! - `site/app`: under the machine's Claude folder ("" is the Claude folder itself);
//! - `~/Documents/site`: anywhere in its home folder, while "Anywhere in home" is on;
//! - `@Data/work`: under a place added at the machine (`sb.py places add`).
//!
//! These are the rules for reading them: which place holds one, its parent, its crumbs, and how it reads on screen
//! (iCloud Drive's real folder is `~/Library/Mobile Documents/com~apple~CloudDocs`, which nobody wants to read).

use crate::api::ApiPlace;

/// iCloud Drive's folder, as an address.
pub const ICLOUD: &str = "~/Library/Mobile Documents/com~apple~CloudDocs";

/// An address outside the Claude folder: chats there need the user's yes first.
pub fn is_outside(addr: &str) -> bool {
    addr == "~" || addr.starts_with("~/") || addr.starts_with('@')
}

/// `addr` is `folder` or inside it. The Claude folder ("") holds only plain addresses.
pub fn contains(folder: &str, addr: &str) -> bool {
    let (folder, addr) = (folder.trim_end_matches('/'), addr.trim_end_matches('/'));
    if folder.is_empty() {
        return !is_outside(addr);
    }
    addr == folder || addr.strip_prefix(folder).is_some_and(|rest| rest.starts_with('/'))
}

/// The folder above, as an address: `~/Documents/site` is in `~/Documents`, `~/Documents` in `~`, `notes` in the
/// Claude folder. None at the top (the Claude folder, `~`, or a place's own folder).
pub fn parent(addr: &str) -> Option<String> {
    let addr = addr.trim_end_matches('/');
    if addr.is_empty() || addr == "~" || (addr.starts_with('@') && !addr.contains('/')) {
        return None;
    }
    Some(match addr.rsplit_once('/') {
        Some((up, _)) => up.to_string(),
        None => String::new(),
    })
}

/// An address as people read it: iCloud Drive by its name, `~` as "Home", the rest as written. `claude` is what
/// the Claude folder itself is called on this machine ("Claude folder", or its name).
pub fn shown(addr: &str, claude: &str) -> String {
    let addr = addr.trim_end_matches('/');
    if addr.is_empty() {
        return claude.to_string();
    }
    if addr == "~" {
        return "Home".into();
    }
    if let Some(rest) = addr.strip_prefix(ICLOUD) {
        return if rest.is_empty() { "iCloud Drive".into() } else { format!("iCloud Drive{rest}") };
    }
    addr.to_string()
}

/// The last name in an address, as people read it ("site", "Home", "iCloud Drive", "Data").
pub fn name_of(addr: &str, claude: &str) -> String {
    let s = shown(addr, claude);
    if addr.trim_end_matches('/') == ICLOUD || addr.is_empty() || addr == "~" {
        return s;
    }
    let last = s.rsplit('/').find(|x| !x.is_empty()).unwrap_or(&s);
    last.trim_start_matches('@').to_string()
}

/// One step of the path bar: what it says and the address it opens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Crumb {
    pub label: String,
    pub folder: String,
}

/// The path bar for an address: the place that holds it first (by its own name), then each folder below it. With no
/// place known, the address's own top: the Claude folder, Home, or the `@` place.
pub fn crumbs(addr: &str, place: Option<&ApiPlace>, claude: &str) -> Vec<Crumb> {
    let addr = addr.trim_end_matches('/');
    let (top_label, top) = match place {
        Some(p) if contains(&p.folder, addr) => (p.name.clone(), p.folder.trim_end_matches('/').to_string()),
        _ if addr.is_empty() || !is_outside(addr) => (claude.to_string(), String::new()),
        _ if addr == "~" || addr.starts_with("~/") => ("Home".to_string(), "~".to_string()),
        _ => {
            let top = addr.split('/').next().unwrap_or(addr).to_string();
            (top.trim_start_matches('@').to_string(), top)
        }
    };
    let mut out = vec![Crumb { label: top_label, folder: top.clone() }];
    let rest = addr.strip_prefix(top.as_str()).unwrap_or("").trim_start_matches('/');
    let mut cur = top;
    for part in rest.split('/').filter(|x| !x.is_empty()) {
        cur = if cur.is_empty() { part.to_string() } else { format!("{cur}/{part}") };
        out.push(Crumb { label: part.to_string(), folder: cur.clone() });
    }
    out
}

/// What a file is, for its icon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileKind {
    Text,
    Code,
    Data,
    Image,
    Media,
    Archive,
    Other,
}

pub fn file_kind(name: &str) -> FileKind {
    let lower = name.to_ascii_lowercase();
    let ext = lower.rsplit_once('.').map(|(_, e)| e).unwrap_or("");
    if matches!(lower.as_str(), "makefile" | "dockerfile" | "justfile" | "rakefile" | "gemfile") {
        return FileKind::Code;
    }
    match ext {
        "md" | "markdown" | "txt" | "rtf" | "doc" | "docx" | "pages" | "pdf" | "org" | "rst" | "tex" | "log" => {
            FileKind::Text
        }
        "rs" | "py" | "js" | "mjs" | "cjs" | "ts" | "tsx" | "jsx" | "swift" | "kt" | "java" | "c" | "h" | "cc"
        | "cpp" | "hpp" | "m" | "mm" | "go" | "rb" | "php" | "cs" | "sh" | "zsh" | "bash" | "fish" | "ps1" | "lua"
        | "html" | "css" | "scss" | "vue" | "svelte" | "sql" | "zig" | "dart" | "r" | "pl" | "ex" | "exs" => {
            FileKind::Code
        }
        "json" | "yaml" | "yml" | "toml" | "csv" | "tsv" | "xml" | "plist" | "ini" | "cfg" | "conf" | "lock"
        | "xlsx" | "xls" | "numbers" | "sqlite" | "db" | "env" => FileKind::Data,
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "heic" | "svg" | "bmp" | "tiff" | "ico" | "psd" | "pcraft" => {
            FileKind::Image
        }
        "mp3" | "wav" | "m4a" | "flac" | "aac" | "mp4" | "mov" | "mkv" | "webm" | "avi" => FileKind::Media,
        "zip" | "tar" | "gz" | "tgz" | "bz2" | "xz" | "7z" | "rar" | "dmg" | "pkg" | "iso" => FileKind::Archive,
        _ => FileKind::Other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn place(folder: &str, name: &str) -> ApiPlace {
        ApiPlace {
            kind: "x".into(),
            name: name.into(),
            folder: folder.into(),
            shown: String::new(),
            state: "ok".into(),
        }
    }

    #[test]
    fn which_folder_holds_an_address() {
        assert!(contains("", "notes/today"));
        assert!(contains("", ""));
        assert!(!contains("", "~/Documents"), "the Claude folder never holds a home address");
        assert!(!contains("", "@Data/work"));
        assert!(contains("~", "~/Documents/site"));
        assert!(contains("~/Documents", "~/Documents"));
        assert!(!contains("~/Doc", "~/Documents"), "a whole name, not a prefix of one");
        assert!(contains("@Data", "@Data/work"));
        assert!(!contains("@Data", "@Database"));
    }

    #[test]
    fn the_folder_above() {
        assert_eq!(parent("~/Documents/site").as_deref(), Some("~/Documents"));
        assert_eq!(parent("~/Documents").as_deref(), Some("~"));
        assert_eq!(parent("~"), None);
        assert_eq!(parent("notes/today").as_deref(), Some("notes"));
        assert_eq!(parent("notes").as_deref(), Some(""));
        assert_eq!(parent(""), None);
        assert_eq!(parent("@Data/work").as_deref(), Some("@Data"));
        assert_eq!(parent("@Data"), None);
    }

    #[test]
    fn addresses_read_nicely() {
        assert_eq!(shown("", "Claude folder"), "Claude folder");
        assert_eq!(shown("~", "Claude folder"), "Home");
        assert_eq!(shown("~/Documents/site", ""), "~/Documents/site");
        assert_eq!(shown(ICLOUD, ""), "iCloud Drive");
        assert_eq!(shown(&format!("{ICLOUD}/Notes/a"), ""), "iCloud Drive/Notes/a");
        assert_eq!(shown("@Data/work", ""), "@Data/work");
        assert_eq!(name_of("~/Documents/site", ""), "site");
        assert_eq!(name_of(ICLOUD, ""), "iCloud Drive");
        assert_eq!(name_of("~", ""), "Home");
        assert_eq!(name_of("@Data", ""), "Data");
        assert_eq!(name_of("", "Claude"), "Claude");
    }

    #[test]
    fn the_path_bar() {
        let docs = place("~/Documents", "Documents");
        let c = crumbs("~/Documents/site/app", Some(&docs), "Claude folder");
        let labels: Vec<&str> = c.iter().map(|c| c.label.as_str()).collect();
        assert_eq!(labels, ["Documents", "site", "app"]);
        assert_eq!(c[0].folder, "~/Documents");
        assert_eq!(c[2].folder, "~/Documents/site/app");

        let c = crumbs("notes/today", None, "Claude folder");
        assert_eq!(c.iter().map(|c| c.folder.as_str()).collect::<Vec<_>>(), ["", "notes", "notes/today"]);
        assert_eq!(c[0].label, "Claude folder");

        let ic = place(ICLOUD, "iCloud Drive");
        let c = crumbs(&format!("{ICLOUD}/Notes"), Some(&ic), "");
        assert_eq!(c.iter().map(|c| c.label.as_str()).collect::<Vec<_>>(), ["iCloud Drive", "Notes"]);

        let c = crumbs("@Data/work", None, "");
        assert_eq!(c[0], Crumb { label: "Data".into(), folder: "@Data".into() });
        assert_eq!(c[1].folder, "@Data/work");

        let c = crumbs("~/Music", Some(&docs), "");
        assert_eq!(c[0].label, "Home", "a place that does not hold it is not used");
    }

    #[test]
    fn file_icons() {
        assert_eq!(file_kind("README.md"), FileKind::Text);
        assert_eq!(file_kind("main.RS"), FileKind::Code);
        assert_eq!(file_kind("Makefile"), FileKind::Code);
        assert_eq!(file_kind("Cargo.lock"), FileKind::Data);
        assert_eq!(file_kind("shot.PNG"), FileKind::Image);
        assert_eq!(file_kind("talk.mov"), FileKind::Media);
        assert_eq!(file_kind("build.tar.gz"), FileKind::Archive);
        assert_eq!(file_kind("LICENSE"), FileKind::Other);
    }
}
