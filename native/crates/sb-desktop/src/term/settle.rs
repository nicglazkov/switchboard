//! The cursor on a Windows terminal. ConPTY, which every terminal on Windows goes through, hides the cursor at the start
//! of each repaint and shows it again at the end. A working chat repaints its spinner several times a second, so the
//! cursor flashed at that rate, out of step with the 530 ms blink (a Mac chat's cursor stays put). On such a terminal
//! the cursor shows once the program has kept it shown for `SETTLE`; hiding it is immediate.

use std::time::{Duration, Instant};

/// How long the cursor must stay shown before it is drawn. ConPTY's hidden and shown spans while Claude Code works run
/// about 120 ms each (a capture from a Windows PC), rarely past 300 ms.
pub const SETTLE: Duration = Duration::from_millis(500);

#[derive(Default)]
pub struct CursorSettle {
    since: Option<Instant>,
}

impl CursorSettle {
    /// The program shows the cursor (`shown`) at `now`: whether to draw it, and if it is shown but not yet settled,
    /// how long until it is (look again then).
    pub fn see(&mut self, shown: bool, now: Instant) -> (bool, Option<Duration>) {
        if !shown {
            self.since = None;
            return (false, None);
        }
        let held = now.saturating_duration_since(*self.since.get_or_insert(now));
        if held >= SETTLE { (true, None) } else { (false, Some(SETTLE - held)) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_cursor_shown_only_between_repaints_is_never_drawn() {
        let mut s = CursorSettle::default();
        let t0 = Instant::now();
        for i in 0..20u64 {
            let at = t0 + Duration::from_millis(i * 240);
            assert!(!s.see(true, at).0, "shown {i}");
            assert!(!s.see(false, at + Duration::from_millis(120)).0);
        }
    }

    #[test]
    fn a_cursor_that_stays_shown_is_drawn_after_the_settle_time() {
        let mut s = CursorSettle::default();
        let t0 = Instant::now();
        assert_eq!(s.see(true, t0), (false, Some(SETTLE)));
        assert_eq!(s.see(true, t0 + Duration::from_millis(200)), (false, Some(Duration::from_millis(300))));
        assert_eq!(s.see(true, t0 + SETTLE), (true, None));
        assert_eq!(s.see(true, t0 + Duration::from_secs(5)), (true, None));
        // Hidden again: gone at once, and the wait starts over.
        assert_eq!(s.see(false, t0 + Duration::from_secs(6)), (false, None));
        assert!(!s.see(true, t0 + Duration::from_secs(6)).0);
    }
}
