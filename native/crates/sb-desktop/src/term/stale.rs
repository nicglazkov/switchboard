//! A terminal that has output its view never drew. The grid is a cached view (`GridView`): it draws again only when
//! it is told to, so a lost notification would leave the screen as it was while the chat goes on, which looks like a
//! frozen app that still takes clicks. The tab counts the output it has parsed and the view remembers the count it
//! last drew; a view that stays behind for `AFTER` is drawn again, and the log says it happened.

use std::time::{Duration, Instant};

/// A view behind this long is not just a frame late.
pub const AFTER: Duration = Duration::from_millis(400);

#[derive(Default)]
pub struct Stale {
    /// The count the view had drawn when it was first seen behind, and when that was.
    noted: Option<(u64, Instant)>,
}

impl Stale {
    /// One look, as the tab renders: `have` is the output parsed so far (a count) and `drawn` the count the view last
    /// drew. True when the view has stayed behind, at the same count, for `AFTER`: draw it now.
    pub fn check(&mut self, have: u64, drawn: u64, now: Instant) -> bool {
        if drawn >= have {
            self.noted = None;
            return false;
        }
        match self.noted {
            Some((d, since)) if d == drawn => {
                if now.saturating_duration_since(since) < AFTER {
                    return false;
                }
                self.noted = None;
                true
            }
            // behind, but it has drawn since the last look: busy, not stuck
            _ => {
                self.noted = Some((drawn, now));
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_view_that_keeps_up_is_left_alone() {
        let mut s = Stale::default();
        let t0 = Instant::now();
        // output arrives all the time; each render the view is one behind, then draws
        for i in 1..200u64 {
            assert!(!s.check(i, i - 1, t0 + Duration::from_millis(i * 16)), "{i}");
        }
        assert!(!s.check(200, 200, t0 + Duration::from_secs(9)));
    }

    #[test]
    fn a_view_that_stays_behind_is_drawn_again_once() {
        let mut s = Stale::default();
        let t0 = Instant::now();
        assert!(!s.check(5, 4, t0));
        assert!(!s.check(6, 4, t0 + Duration::from_millis(200)), "not yet");
        assert!(s.check(7, 4, t0 + AFTER));
        // told to draw: it gets the same time again before the next nudge
        assert!(!s.check(7, 4, t0 + AFTER + Duration::from_millis(16)));
        assert!(!s.check(7, 7, t0 + AFTER + Duration::from_millis(32)));
        assert!(!s.check(7, 7, t0 + Duration::from_secs(60)));
    }
}
