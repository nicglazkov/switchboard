//! The cursor on a Windows terminal. ConPTY, which every terminal on Windows goes through, hides the cursor at the start
//! of each repaint and shows it again at the end. A working chat repaints its spinner several times a second, so the
//! cursor flashed at that rate, out of step with the 530 ms blink (a Mac chat's cursor stays put). On such a terminal
//! the cursor shows once the program has kept it shown for `SETTLE`; hiding it is immediate.
//!
//! Typing is the exception. Each key's echo is a repaint too, so the wait kept the cursor away for as long as one
//! typed. For `TYPING` after a key the cursor is drawn as soon as the program shows it, and stays drawn through the
//! short hides around a repaint (`FLICKER`). A program that keeps its cursor hidden still has none.
//!
//! The state is looked at after every slice of output parsed, not only when a frame is drawn, so a hide between two
//! frames counts.

use std::time::{Duration, Instant};

/// How long the cursor must stay shown before it is drawn. ConPTY's hidden and shown spans while Claude Code works run
/// about 120 ms each (a capture from a Windows PC), rarely past 300 ms.
pub const SETTLE: Duration = Duration::from_millis(500);
/// For this long after a key the cursor does not wait to settle.
pub const TYPING: Duration = Duration::from_secs(1);
/// A hide no longer than this is ConPTY's, around a repaint.
pub const FLICKER: Duration = Duration::from_millis(300);

/// Whether a terminal settles its cursor: a local shell in the Windows app, or a chat on a Windows machine. A chat
/// whose machine's system is not known yet (the list has not loaded, or the server did not say) settles too:
/// on a machine that is not Windows that costs one half-second wait after a real hide, and on Windows not settling
/// brings the flashing back.
pub fn on(local: bool, os: &str) -> bool {
    if local { cfg!(windows) } else { os == "windows" || os.is_empty() }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Draw {
    No,
    Yes,
    /// Drawn although the program hides it this moment: typing, through a repaint's hide.
    Held,
}

impl Draw {
    pub fn drawn(self) -> bool {
        self != Draw::No
    }
}

#[derive(Default)]
pub struct CursorSettle {
    since: Option<Instant>,
    last_shown: Option<Instant>,
    typed: Option<Instant>,
}

impl CursorSettle {
    /// A key was typed at `now`.
    pub fn typed(&mut self, now: Instant) {
        self.typed = Some(now);
    }

    /// The program shows the cursor (`shown`) at `now`: whether to draw it, and when its answer may change with
    /// nothing new from the program (look again then).
    pub fn see(&mut self, shown: bool, now: Instant) -> (Draw, Option<Duration>) {
        let typing = self.typed.and_then(|t| TYPING.checked_sub(now.saturating_duration_since(t)));
        if !shown {
            self.since = None;
            // typing, and it was there a moment ago: a repaint's hide
            let flicker = self.last_shown.and_then(|t| FLICKER.checked_sub(now.saturating_duration_since(t)));
            return match (typing, flicker) {
                (Some(t), Some(f)) if !t.is_zero() && !f.is_zero() => (Draw::Held, Some(t.min(f))),
                _ => (Draw::No, None),
            };
        }
        self.last_shown = Some(now);
        let held = now.saturating_duration_since(*self.since.get_or_insert(now));
        if held >= SETTLE {
            return (Draw::Yes, None);
        }
        match typing {
            // when the typing is over it must have settled, or it goes
            Some(t) if !t.is_zero() => (Draw::Yes, Some(t.min(SETTLE - held))),
            _ => (Draw::No, Some(SETTLE - held)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn a_cursor_shown_only_between_repaints_is_never_drawn() {
        let mut s = CursorSettle::default();
        let t0 = Instant::now();
        for i in 0..20u64 {
            let at = t0 + ms(i * 240);
            assert!(!s.see(true, at).0.drawn(), "shown {i}");
            assert!(!s.see(false, at + ms(120)).0.drawn());
        }
    }

    #[test]
    fn a_cursor_that_stays_shown_is_drawn_after_the_settle_time() {
        let mut s = CursorSettle::default();
        let t0 = Instant::now();
        assert_eq!(s.see(true, t0), (Draw::No, Some(SETTLE)));
        assert_eq!(s.see(true, t0 + ms(200)), (Draw::No, Some(ms(300))));
        assert_eq!(s.see(true, t0 + SETTLE), (Draw::Yes, None));
        assert_eq!(s.see(true, t0 + Duration::from_secs(5)), (Draw::Yes, None));
        // Hidden again: gone at once, and the wait starts over.
        assert_eq!(s.see(false, t0 + Duration::from_secs(6)), (Draw::No, None));
        assert!(!s.see(true, t0 + Duration::from_secs(6)).0.drawn());
    }

    #[test]
    fn typing_shows_the_cursor_at_once_and_keeps_it_through_each_echo() {
        let mut s = CursorSettle::default();
        let t0 = Instant::now();
        // a working chat: hidden and shown every 20 ms, never drawn
        for i in 0..50u64 {
            assert!(!s.see(i % 2 == 1, t0 + ms(i * 20)).0.drawn());
        }
        // a key every 200 ms for two seconds, each echoed by a repaint, the flicker going on all the while
        let t1 = t0 + ms(1000);
        for i in 0..100u64 {
            let now = t1 + ms(i * 20);
            if i % 10 == 0 {
                s.typed(now);
            }
            let shown = i % 2 == 1;
            let (draw, wake) = s.see(shown, now);
            if i == 0 {
                // hidden, and last shown 20 ms ago
                assert_eq!(draw, Draw::Held);
            }
            assert_eq!(draw, if shown { Draw::Yes } else { Draw::Held }, "look {i}");
            assert!(wake.is_some(), "it looks again when the typing is over");
        }
        // the typing stops and the flicker goes on: back to waiting for it to settle
        let t2 = t1 + ms(1980) + TYPING;
        assert_eq!(s.see(false, t2), (Draw::No, None));
        assert_eq!(s.see(true, t2 + ms(20)), (Draw::No, Some(SETTLE)));
        assert_eq!(s.see(false, t2 + ms(40)), (Draw::No, None));
    }

    #[test]
    fn typing_never_draws_a_cursor_the_program_keeps_hidden() {
        let mut s = CursorSettle::default();
        let t0 = Instant::now();
        s.typed(t0);
        for i in 0..40u64 {
            assert_eq!(s.see(false, t0 + ms(i * 50)), (Draw::No, None));
        }
        // shown once, then hidden for good: held for a repaint's length, no longer
        s.typed(t0 + ms(2000));
        assert_eq!(s.see(true, t0 + ms(2000)).0, Draw::Yes);
        assert_eq!(s.see(false, t0 + ms(2100)), (Draw::Held, Some(ms(200))));
        assert_eq!(s.see(false, t0 + ms(2000) + FLICKER), (Draw::No, None));
    }

    #[test]
    fn a_cursor_shown_while_typing_stays_when_the_typing_ends_only_if_it_settled() {
        let mut s = CursorSettle::default();
        let t0 = Instant::now();
        s.typed(t0);
        assert_eq!(s.see(true, t0 + ms(100)), (Draw::Yes, Some(SETTLE)));
        assert_eq!(s.see(true, t0 + ms(900)), (Draw::Yes, None), "shown for 800 ms: settled");
        assert_eq!(s.see(true, t0 + ms(5000)), (Draw::Yes, None));
        // shown only just before the typing window closes: it waits out the rest of the settle time
        let mut s = CursorSettle::default();
        s.typed(t0);
        assert_eq!(s.see(true, t0 + ms(900)), (Draw::Yes, Some(ms(100))));
        assert_eq!(s.see(true, t0 + ms(1000)), (Draw::No, Some(ms(400))));
        assert_eq!(s.see(true, t0 + ms(1400)), (Draw::Yes, None));
    }

    #[test]
    fn a_chat_on_a_machine_of_unknown_system_settles() {
        assert!(on(false, "windows"));
        assert!(on(false, ""), "not known yet: as if Windows");
        assert!(!on(false, "mac") && !on(false, "linux"));
        assert_eq!(on(true, ""), cfg!(windows), "a local shell is this computer's");
    }
}
