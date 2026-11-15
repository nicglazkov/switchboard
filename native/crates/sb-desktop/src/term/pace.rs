//! Repaints at the screen's pace. Output is parsed a slice at a time (every few milliseconds while it floods), and
//! each slice used to ask for a repaint: a chat on a Windows machine, which sends its output in many small pieces,
//! had the tab and its grid rebuilt far more often than a frame can show. A slice that changed something now asks
//! here: the first is shown at once, and those within a frame of it share one repaint when that frame is up.

use std::time::{Duration, Instant};

/// One frame at 60 Hz: no more repaints than this are asked for.
pub const FRAME: Duration = Duration::from_millis(16);

#[derive(Debug, PartialEq)]
pub enum Step {
    /// Repaint now.
    Now,
    /// Repaint after this long (call `fire` then): the last repaint was less than a frame ago.
    Later(Duration),
    /// A repaint is already on its way and will show this change too.
    Waiting,
}

#[derive(Default)]
pub struct Pace {
    shown: Option<Instant>,
    due: bool,
}

impl Pace {
    /// Something changed at `now` and wants to be seen.
    pub fn changed(&mut self, now: Instant) -> Step {
        if self.due {
            return Step::Waiting;
        }
        match self.shown.map(|t| now.saturating_duration_since(t)) {
            Some(ago) if ago < FRAME => {
                self.due = true;
                Step::Later(FRAME - ago)
            }
            _ => {
                self.shown = Some(now);
                Step::Now
            }
        }
    }

    /// The wait from `Later` is over and the repaint is asked for.
    pub fn fire(&mut self, now: Instant) {
        self.due = false;
        self.shown = Some(now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_change_after_a_quiet_spell_is_shown_at_once() {
        let mut p = Pace::default();
        let t0 = Instant::now();
        assert_eq!(p.changed(t0), Step::Now);
        assert_eq!(p.changed(t0 + FRAME), Step::Now);
        assert_eq!(p.changed(t0 + Duration::from_secs(3)), Step::Now, "a key's echo does not wait");
    }

    #[test]
    fn a_flood_of_slices_repaints_once_a_frame_and_the_last_change_is_shown() {
        let mut p = Pace::default();
        let t0 = Instant::now();
        // a slice every 4 ms for a second, as a flood parses; the timer fires when asked to
        let (mut repaints, mut fire_at, mut last_repaint) = (0, None, t0);
        for ms in 0..=1000u64 {
            let now = t0 + Duration::from_millis(ms);
            if fire_at.is_some_and(|at| now >= at) {
                fire_at = None;
                p.fire(now);
                repaints += 1;
                last_repaint = now;
            }
            if ms % 4 == 0 && ms <= 990 {
                match p.changed(now) {
                    Step::Now => {
                        repaints += 1;
                        last_repaint = now;
                    }
                    Step::Later(wait) => {
                        assert!(fire_at.is_none() && wait <= FRAME);
                        fire_at = Some(now + wait);
                    }
                    Step::Waiting => assert!(fire_at.is_some(), "waiting only on a repaint that is coming"),
                }
            }
        }
        assert!((55..=64).contains(&repaints), "248 slices, {repaints} repaints");
        assert!(fire_at.is_none(), "nothing left waiting");
        assert!(last_repaint >= t0 + Duration::from_millis(988), "the last slice was shown");
    }
}
