//! The bytes a key sends to the terminal, as xterm sends them (what Claude Code, bash, vim and htop expect).
//!
//! Plain printable text is not handled here: it comes through the platform's text input (IME, dead keys, emoji)
//! and is sent as typed. This covers named keys and combinations with Ctrl or Alt. Alt (Option on a Mac) is Meta:
//! ESC followed by the key, so Claude Code's Alt+P, Alt+T and Meta+Enter work. The platform shortcut key (Cmd on a
//! Mac) never reaches the terminal.

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Mods {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    /// Cmd on a Mac, the Windows key elsewhere: always the app's.
    pub platform: bool,
}

impl Mods {
    pub const NONE: Mods = Mods { ctrl: false, alt: false, shift: false, platform: false };

    /// xterm's modifier parameter: 1 + shift + 2 alt + 4 ctrl (1 means none).
    fn param(self) -> u8 {
        1 + self.shift as u8 + 2 * self.alt as u8 + 4 * self.ctrl as u8
    }
}

const ESC: u8 = 0x1b;

/// The bytes for `key` (a GPUI key name: "enter", "tab", "up", "f5", "a", "[") with `mods`.
/// `app_cursor` is DECCKM (the program asked for application cursor keys). None means "not a terminal key":
/// leave it to text input or to the app's shortcuts.
pub fn encode(key: &str, mods: Mods, app_cursor: bool) -> Option<Vec<u8>> {
    if mods.platform {
        return None;
    }
    let m = mods.param();
    let csi = |tail: &str| -> Vec<u8> { format!("\x1b[{tail}").into_bytes() };
    // Cursor-style keys: CSI X, SS3 X in application mode, CSI 1;m X with modifiers.
    let cursor = |x: char| -> Vec<u8> {
        if m > 1 {
            csi(&format!("1;{m}{x}"))
        } else if app_cursor {
            vec![ESC, b'O', x as u8]
        } else {
            csi(&x.to_string())
        }
    };
    // Tilde keys: CSI n ~, CSI n;m ~ with modifiers.
    let tilde = |n: u8| -> Vec<u8> {
        if m > 1 { csi(&format!("{n};{m}~")) } else { csi(&format!("{n}~")) }
    };
    let meta = |bytes: Vec<u8>| -> Vec<u8> {
        if mods.alt {
            let mut v = vec![ESC];
            v.extend(bytes);
            v
        } else {
            bytes
        }
    };
    let out = match key {
        "enter" => {
            // Shift+Enter and Alt+Enter are Meta+Enter: Claude Code takes it as a new line in the prompt.
            if mods.shift || mods.alt { vec![ESC, b'\r'] } else { vec![b'\r'] }
        }
        "tab" => {
            if mods.shift {
                csi("Z")
            } else {
                meta(vec![b'\t'])
            }
        }
        "backspace" => {
            if mods.ctrl {
                meta(vec![0x08])
            } else {
                meta(vec![0x7f])
            }
        }
        "escape" => meta(vec![ESC]),
        "space" => {
            if mods.ctrl {
                meta(vec![0])
            } else if mods.alt {
                vec![ESC, b' ']
            } else {
                return None;
            }
        }
        "up" => cursor('A'),
        "down" => cursor('B'),
        "right" => cursor('C'),
        "left" => cursor('D'),
        "home" => cursor('H'),
        "end" => cursor('F'),
        "insert" => tilde(2),
        "delete" => tilde(3),
        "pageup" => tilde(5),
        "pagedown" => tilde(6),
        "f1" | "f2" | "f3" | "f4" => {
            let x = match key {
                "f1" => 'P',
                "f2" => 'Q',
                "f3" => 'R',
                _ => 'S',
            };
            if m > 1 { csi(&format!("1;{m}{x}")) } else { vec![ESC, b'O', x as u8] }
        }
        "f5" => tilde(15),
        "f6" => tilde(17),
        "f7" => tilde(18),
        "f8" => tilde(19),
        "f9" => tilde(20),
        "f10" => tilde(21),
        "f11" => tilde(23),
        "f12" => tilde(24),
        _ => return char_key(key, mods),
    };
    Some(out)
}

/// What a text-editing shortcut does in the terminal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Edit {
    /// Send these bytes (readline's line and word moves, which Claude Code's prompt and shells both take).
    Send(Vec<u8>),
    /// Scroll the view to the top or bottom of the history; nothing goes to the program.
    ScrollTop,
    ScrollBottom,
}

/// The editing keys people expect from a native text field, as iTerm2's "Natural Text Editing" maps them.
/// macOS: Cmd+Left/Right line start and end, Option+Left/Right a word back and forward, Cmd+Backspace delete to
/// line start, Option+Backspace delete a word, Cmd+Up/Down the top and bottom of the history.
/// Windows and Linux: Home/End, Ctrl+Left/Right, Ctrl+Backspace, Ctrl+Home/End the same way.
/// `alt_screen`: a full-screen program (vim, less) gets Home and End as the keys themselves.
/// None: not one of these, so the usual encoding applies.
pub fn edit_key(key: &str, mods: Mods, mac: bool, alt_screen: bool) -> Option<Edit> {
    let only = |platform: bool, alt: bool, ctrl: bool| mods.platform == platform && mods.alt == alt && mods.ctrl == ctrl && !mods.shift;
    let send = |b: &[u8]| Some(Edit::Send(b.to_vec()));
    if mac {
        if only(true, false, false) {
            return match key {
                "left" => send(b"\x01"),
                "right" => send(b"\x05"),
                "backspace" => send(b"\x15"),
                "up" => Some(Edit::ScrollTop),
                "down" => Some(Edit::ScrollBottom),
                _ => None,
            };
        }
        if only(false, true, false) {
            return match key {
                "left" => send(b"\x1bb"),
                "right" => send(b"\x1bf"),
                "backspace" => send(b"\x17"),
                _ => None,
            };
        }
        return None;
    }
    if only(false, false, true) {
        return match key {
            "left" => send(b"\x1bb"),
            "right" => send(b"\x1bf"),
            "backspace" => send(b"\x17"),
            "home" => Some(Edit::ScrollTop),
            "end" => Some(Edit::ScrollBottom),
            _ => None,
        };
    }
    if only(false, false, false) && !alt_screen {
        return match key {
            "home" => send(b"\x01"),
            "end" => send(b"\x05"),
            _ => None,
        };
    }
    None
}

/// A one-character key with Ctrl and/or Alt. Plain and Shift-only keys are text, so None.
fn char_key(key: &str, mods: Mods) -> Option<Vec<u8>> {
    let mut chars = key.chars();
    let c = chars.next()?;
    if chars.next().is_some() || !(mods.ctrl || mods.alt) {
        return None;
    }
    let base = if mods.ctrl {
        vec![ctrl_byte(c)?]
    } else if mods.shift && c.is_ascii_alphabetic() {
        vec![c.to_ascii_uppercase() as u8]
    } else {
        let mut b = [0u8; 4];
        c.encode_utf8(&mut b).as_bytes().to_vec()
    };
    if mods.alt {
        let mut v = vec![ESC];
        v.extend(base);
        Some(v)
    } else {
        Some(base)
    }
}

/// Ctrl+key as the C0 control it means (Ctrl+A = 0x01 ... Ctrl+Z = 0x1A, Ctrl+[ = ESC, and xterm's digit aliases).
pub fn ctrl_byte(c: char) -> Option<u8> {
    let c = c.to_ascii_lowercase();
    Some(match c {
        'a'..='z' => c as u8 - b'a' + 1,
        '@' | '2' | ' ' => 0x00,
        '[' | '3' => 0x1b,
        '\\' | '4' => 0x1c,
        ']' | '5' => 0x1d,
        '^' | '6' => 0x1e,
        '_' | '-' | '/' | '7' => 0x1f,
        '8' | '?' => 0x7f,
        _ => return None,
    })
}

/// Text pasted into a program that asked for bracketed paste is wrapped so it is not run as typed; ESC is
/// dropped from the text so a paste can never end the bracket early. Otherwise line breaks become CR, as typing.
pub fn paste(text: &str, bracketed: bool) -> Vec<u8> {
    let normal = text.replace("\r\n", "\r").replace('\n', "\r");
    if bracketed {
        let clean: String = normal.chars().filter(|c| *c != '\x1b').collect();
        let mut v = b"\x1b[200~".to_vec();
        v.extend(clean.into_bytes());
        v.extend(b"\x1b[201~");
        v
    } else {
        normal.into_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn k(key: &str) -> Option<Vec<u8>> {
        encode(key, Mods::NONE, false)
    }
    fn with(key: &str, ctrl: bool, alt: bool, shift: bool) -> Option<Vec<u8>> {
        encode(key, Mods { ctrl, alt, shift, platform: false }, false)
    }

    #[test]
    fn named_keys() {
        assert_eq!(k("enter").unwrap(), b"\r");
        assert_eq!(k("tab").unwrap(), b"\t");
        assert_eq!(k("backspace").unwrap(), b"\x7f");
        assert_eq!(k("escape").unwrap(), b"\x1b");
        assert_eq!(k("delete").unwrap(), b"\x1b[3~");
        assert_eq!(k("insert").unwrap(), b"\x1b[2~");
        assert_eq!(k("pageup").unwrap(), b"\x1b[5~");
        assert_eq!(k("pagedown").unwrap(), b"\x1b[6~");
        assert_eq!(k("home").unwrap(), b"\x1b[H");
        assert_eq!(k("end").unwrap(), b"\x1b[F");
        assert_eq!(k("f1").unwrap(), b"\x1bOP");
        assert_eq!(k("f4").unwrap(), b"\x1bOS");
        assert_eq!(k("f5").unwrap(), b"\x1b[15~");
        assert_eq!(k("f12").unwrap(), b"\x1b[24~");
    }

    #[test]
    fn shift_tab_is_back_tab() {
        // Claude Code cycles its modes with Shift+Tab.
        assert_eq!(with("tab", false, false, true).unwrap(), b"\x1b[Z");
    }

    #[test]
    fn arrows_follow_application_cursor_mode() {
        assert_eq!(k("up").unwrap(), b"\x1b[A");
        assert_eq!(k("left").unwrap(), b"\x1b[D");
        assert_eq!(encode("up", Mods::NONE, true).unwrap(), b"\x1bOA");
        assert_eq!(encode("home", Mods::NONE, true).unwrap(), b"\x1bOH");
        // Modifiers always use the CSI form.
        assert_eq!(encode("right", Mods { ctrl: true, ..Mods::NONE }, true).unwrap(), b"\x1b[1;5C");
        assert_eq!(with("left", false, true, false).unwrap(), b"\x1b[1;3D");
        assert_eq!(with("up", false, false, true).unwrap(), b"\x1b[1;2A");
        assert_eq!(with("end", true, false, true).unwrap(), b"\x1b[1;6F");
        assert_eq!(with("delete", true, false, false).unwrap(), b"\x1b[3;5~");
        assert_eq!(with("f5", false, false, true).unwrap(), b"\x1b[15;2~");
        assert_eq!(with("f1", true, false, false).unwrap(), b"\x1b[1;5P");
    }

    #[test]
    fn ctrl_keys_are_c0_controls() {
        assert_eq!(with("c", true, false, false).unwrap(), [0x03]);
        assert_eq!(with("a", true, false, false).unwrap(), [0x01]);
        assert_eq!(with("z", true, false, false).unwrap(), [0x1a]);
        assert_eq!(with("C", true, false, true).unwrap(), [0x03]);
        assert_eq!(with("[", true, false, false).unwrap(), [0x1b]);
        assert_eq!(with("\\", true, false, false).unwrap(), [0x1c]);
        assert_eq!(with("]", true, false, false).unwrap(), [0x1d]);
        assert_eq!(with("space", true, false, false).unwrap(), [0x00]);
        assert_eq!(with("2", true, false, false).unwrap(), [0x00]);
        assert_eq!(with("6", true, false, false).unwrap(), [0x1e]);
        assert_eq!(with("-", true, false, false).unwrap(), [0x1f]);
        assert_eq!(with("backspace", true, false, false).unwrap(), [0x08]);
        assert_eq!(with("=", true, false, false), None);
    }

    #[test]
    fn alt_is_meta() {
        assert_eq!(with("p", false, true, false).unwrap(), b"\x1bp");
        assert_eq!(with("t", false, true, false).unwrap(), b"\x1bt");
        assert_eq!(with("b", false, true, true).unwrap(), b"\x1bB");
        assert_eq!(with("enter", false, true, false).unwrap(), b"\x1b\r");
        assert_eq!(with("enter", false, false, true).unwrap(), b"\x1b\r");
        assert_eq!(with("backspace", false, true, false).unwrap(), b"\x1b\x7f");
        assert_eq!(with("c", true, true, false).unwrap(), b"\x1b\x03");
        assert_eq!(with("space", false, true, false).unwrap(), b"\x1b ");
    }

    #[test]
    fn natural_text_editing_table() {
        let m = |platform, alt, ctrl, shift| Mods { platform, alt, ctrl, shift };
        let s = |b: &[u8]| Some(Edit::Send(b.to_vec()));
        let mac = |key, mods| edit_key(key, mods, true, false);
        let pc = |key, mods| edit_key(key, mods, false, false);
        // macOS
        assert_eq!(mac("left", m(true, false, false, false)), s(b"\x01"));
        assert_eq!(mac("right", m(true, false, false, false)), s(b"\x05"));
        assert_eq!(mac("left", m(false, true, false, false)), s(b"\x1bb"));
        assert_eq!(mac("right", m(false, true, false, false)), s(b"\x1bf"));
        assert_eq!(mac("backspace", m(true, false, false, false)), s(b"\x15"));
        assert_eq!(mac("backspace", m(false, true, false, false)), s(b"\x17"));
        assert_eq!(mac("up", m(true, false, false, false)), Some(Edit::ScrollTop));
        assert_eq!(mac("down", m(true, false, false, false)), Some(Edit::ScrollBottom));
        assert_eq!(mac("left", m(true, false, false, true)), None, "Cmd+Shift+Left is left to the app");
        assert_eq!(mac("c", m(true, false, false, false)), None, "Cmd+C stays copy");
        assert_eq!(mac("left", Mods::NONE), None);
        assert_eq!(mac("home", Mods::NONE), None, "Home is a key of its own on a Mac keyboard");
        // Windows and Linux
        assert_eq!(pc("home", Mods::NONE), s(b"\x01"));
        assert_eq!(pc("end", Mods::NONE), s(b"\x05"));
        assert_eq!(edit_key("home", Mods::NONE, false, true), None, "vim and less get the Home key itself");
        assert_eq!(pc("left", m(false, false, true, false)), s(b"\x1bb"));
        assert_eq!(pc("right", m(false, false, true, false)), s(b"\x1bf"));
        assert_eq!(pc("backspace", m(false, false, true, false)), s(b"\x17"));
        assert_eq!(pc("home", m(false, false, true, false)), Some(Edit::ScrollTop));
        assert_eq!(pc("end", m(false, false, true, false)), Some(Edit::ScrollBottom));
        assert_eq!(pc("left", m(false, true, false, false)), None, "Alt+Left keeps its xterm encoding");
    }

    #[test]
    fn text_and_shortcuts_are_left_alone() {
        assert_eq!(k("a"), None);
        assert_eq!(with("a", false, false, true), None);
        assert_eq!(k("space"), None);
        assert_eq!(k("/"), None);
        assert_eq!(encode("c", Mods { platform: true, ..Mods::NONE }, false), None);
        assert_eq!(encode("enter", Mods { platform: true, ..Mods::NONE }, false), None);
    }

    #[test]
    fn bracketed_paste() {
        assert_eq!(paste("ls\n", false), b"ls\r");
        assert_eq!(paste("a\r\nb", true), b"\x1b[200~a\rb\x1b[201~");
        assert_eq!(paste("x\x1b[201~rm", true), b"\x1b[200~x[201~rm\x1b[201~");
    }
}
