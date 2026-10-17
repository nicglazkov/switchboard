//! "exit" in a terminal tab leaves the chat. A tab runs `claude attach`, where /exit only goes back to the agent
//! view; Ctrl+Z ends the attach cleanly and the chat keeps running. So the tab follows what was typed since the
//! last Enter, and when that line is exactly a leave word, Enter becomes: clear the input box, then Ctrl+Z.
//!
//! The line is only trusted when every key since the last Enter was plain typing or Backspace. Arrows, escape
//! sequences, Ctrl keys, a paste or a click into the terminal could have moved the cursor or changed the box in ways
//! this cannot see, so the line is unknown until the next Enter (and Enter then works as always).

/// What a leave sends instead of Enter: one Backspace per typed character, then Ctrl+Z.
pub fn leave_bytes(typed: usize) -> Vec<u8> {
    let mut b = vec![0x7f; typed];
    b.push(0x1a);
    b
}

const WORDS: [&str; 4] = ["exit", "/exit", "quit", "/quit"];

#[derive(Debug, Clone)]
pub struct ExitLine {
    buf: String,
    known: bool,
}

impl Default for ExitLine {
    fn default() -> Self {
        ExitLine { buf: String::new(), known: true }
    }
}

impl ExitLine {
    /// Text typed (the input method's characters): plain characters extend the line, anything else spoils it.
    pub fn text(&mut self, text: &str) {
        if !self.known {
            return;
        }
        if text.chars().all(|c| !c.is_control()) {
            self.buf.push_str(text);
        } else {
            self.unknown();
        }
    }

    /// The bytes a key sent (Enter is handled by `enter`): Backspace takes a character off, one printable byte adds
    /// it, and anything else (arrows, escape sequences, Ctrl keys) makes the line unknown.
    pub fn key(&mut self, bytes: &[u8]) {
        if !self.known {
            return;
        }
        match bytes {
            [0x7f] | [0x08] => {
                self.buf.pop();
            }
            [b] if (0x20..0x7f).contains(b) => self.buf.push(*b as char),
            _ => self.unknown(),
        }
    }

    /// A paste, a click into the terminal or an editing shortcut: the line can no longer be followed.
    pub fn unknown(&mut self) {
        self.known = false;
        self.buf.clear();
    }

    /// Enter: when the line is a leave word, how many characters to erase first. The line starts over either way.
    pub fn enter(&mut self) -> Option<usize> {
        let hit = self.known && WORDS.contains(&self.buf.trim().to_lowercase().as_str());
        let n = self.buf.chars().count();
        *self = ExitLine::default();
        hit.then_some(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typed(s: &str) -> ExitLine {
        let mut l = ExitLine::default();
        for c in s.chars() {
            l.text(&c.to_string());
        }
        l
    }

    #[test]
    fn leave_words_match_and_count_what_to_erase() {
        assert_eq!(typed("exit").enter(), Some(4));
        assert_eq!(typed(" /EXIT ").enter(), Some(7));
        assert_eq!(typed("quit").enter(), Some(4));
        assert_eq!(typed("/quit").enter(), Some(5));
    }

    #[test]
    fn other_lines_are_left_alone() {
        assert_eq!(typed("exit now").enter(), None);
        assert_eq!(typed("exits").enter(), None);
        assert_eq!(typed("").enter(), None);
    }

    #[test]
    fn backspace_edits_the_line() {
        let mut l = typed("exot");
        l.key(&[0x7f]);
        l.key(&[0x7f]);
        l.text("it");
        assert_eq!(l.enter(), Some(4));
        let mut l = typed("exit");
        l.key(&[0x7f]);
        assert_eq!(l.enter(), None, "exi");
    }

    #[test]
    fn arrows_ctrl_keys_and_paste_make_it_unknown_until_enter() {
        let mut l = typed("exit");
        l.key(b"\x1b[D");
        assert_eq!(l.enter(), None);
        let mut l = typed("ex");
        l.key(&[0x01]);
        l.text("it");
        assert_eq!(l.enter(), None);
        let mut l = typed("ex");
        l.unknown();
        l.text("it");
        assert_eq!(l.enter(), None);
        // and the next line is followed again
        assert_eq!(typed("exit").enter(), Some(4));
        let mut l = ExitLine::default();
        l.unknown();
        assert_eq!(l.enter(), None);
        l.text("exit");
        assert_eq!(l.enter(), Some(4));
    }

    #[test]
    fn typed_keys_as_bytes_count_too() {
        let mut l = ExitLine::default();
        for b in b"/exit" {
            l.key(&[*b]);
        }
        assert_eq!(l.enter(), Some(5));
    }

    #[test]
    fn leaving_erases_then_sends_ctrl_z() {
        assert_eq!(leave_bytes(4), vec![0x7f, 0x7f, 0x7f, 0x7f, 0x1a]);
    }
}
