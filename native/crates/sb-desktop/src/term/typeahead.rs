//! Keys typed while a chat's link is coming back. They used to vanish: the tab took the key and sent it nowhere.
//! Now they wait here and go out, in order, when the link is back, if that is soon. Kept for longer they are thrown
//! away instead (and the bar says so): an Enter typed at a screen that has since changed should not land blind.

use std::time::{Duration, Instant};

/// What is typed waits at most this long for the link, counted from the first key.
pub const KEEP: Duration = Duration::from_secs(5);
/// And at most this much of it: a few lines of typing. More means something is leaning on the keyboard.
pub const MAX: usize = 4096;

/// What the bar says about it.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Held {
    Nothing,
    /// Keys wait for the link.
    Waiting,
    /// Keys were typed and will not be sent.
    Dropped,
}

#[derive(Default)]
pub struct Typeahead {
    bytes: Vec<u8>,
    since: Option<Instant>,
    dropped: bool,
}

impl Typeahead {
    fn late(&self, now: Instant) -> bool {
        self.since.is_some_and(|t| now.saturating_duration_since(t) >= KEEP)
    }

    /// A key typed at `now` while the link is away. False when it is not kept (and nothing typed before it either).
    pub fn key(&mut self, bytes: &[u8], now: Instant) -> bool {
        if self.dropped || self.late(now) || self.bytes.len() + bytes.len() > MAX {
            self.dropped = true;
            self.bytes.clear();
            return false;
        }
        self.since.get_or_insert(now);
        self.bytes.extend_from_slice(bytes);
        true
    }

    pub fn state(&self, now: Instant) -> Held {
        if self.dropped || (self.late(now) && !self.bytes.is_empty()) {
            Held::Dropped
        } else if self.bytes.is_empty() {
            Held::Nothing
        } else {
            Held::Waiting
        }
    }

    /// The reconnect these keys waited for is over, or another attach is starting: they are not sent anywhere.
    pub fn clear(&mut self) {
        *self = Typeahead::default();
    }

    /// The link is back at `now`: what to send, if it was typed recently enough. Empty again either way.
    pub fn open(&mut self, now: Instant) -> Option<Vec<u8>> {
        let send = self.state(now) == Held::Waiting;
        let bytes = std::mem::take(&mut self.bytes);
        *self = Typeahead::default();
        send.then_some(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_typed_during_a_short_drop_go_out_in_order() {
        let mut t = Typeahead::default();
        let t0 = Instant::now();
        assert_eq!(t.state(t0), Held::Nothing);
        assert!(t.key(b"l", t0));
        assert!(t.key(b"s", t0 + Duration::from_millis(200)));
        assert!(t.key(b"\r", t0 + Duration::from_millis(900)));
        assert_eq!(t.state(t0 + Duration::from_secs(1)), Held::Waiting);
        assert_eq!(t.open(t0 + Duration::from_secs(2)).as_deref(), Some(&b"ls\r"[..]));
        // sent once
        assert_eq!(t.state(t0 + Duration::from_secs(2)), Held::Nothing);
        assert_eq!(t.open(t0 + Duration::from_secs(3)), None);
    }

    #[test]
    fn keys_that_waited_too_long_are_not_sent() {
        let mut t = Typeahead::default();
        let t0 = Instant::now();
        assert!(t.key(b"rm -rf build\r", t0));
        assert_eq!(t.state(t0 + KEEP - Duration::from_millis(1)), Held::Waiting);
        assert_eq!(t.state(t0 + KEEP), Held::Dropped, "the bar says so while it still reconnects");
        // more typing does not start a new wait: the screen it was typed at is gone
        assert!(!t.key(b"y", t0 + KEEP + Duration::from_secs(1)));
        assert_eq!(t.state(t0 + KEEP + Duration::from_secs(1)), Held::Dropped);
        assert_eq!(t.open(t0 + Duration::from_secs(30)), None);
        // after the link is back, a later drop starts afresh
        let t1 = t0 + Duration::from_secs(60);
        assert_eq!(t.state(t1), Held::Nothing);
        assert!(t.key(b"a", t1));
        assert_eq!(t.open(t1 + Duration::from_secs(1)).as_deref(), Some(&b"a"[..]));
    }

    #[test]
    fn a_link_back_just_too_late_sends_nothing() {
        let mut t = Typeahead::default();
        let t0 = Instant::now();
        assert!(t.key(b"yes\r", t0));
        assert_eq!(t.open(t0 + KEEP), None);
    }

    #[test]
    fn keys_held_for_a_reconnect_that_gave_up_never_reach_the_next_attach() {
        let mut t = Typeahead::default();
        let t0 = Instant::now();
        assert!(t.key(b"y\r", t0));
        // the reconnect gives up a second later; Resume attaches something else within the five seconds
        t.clear();
        assert_eq!(t.state(t0 + Duration::from_secs(1)), Held::Nothing);
        assert_eq!(t.open(t0 + Duration::from_secs(2)), None);
        // and a drop that had already thrown its keys away starts afresh too
        assert!(t.key(b"a", t0));
        assert!(!t.key(b"b", t0 + KEEP));
        t.clear();
        assert_eq!(t.state(t0 + KEEP), Held::Nothing);
    }

    #[test]
    fn more_than_a_few_lines_is_dropped_whole() {
        let mut t = Typeahead::default();
        let t0 = Instant::now();
        assert!(t.key(&[b'a'; MAX], t0));
        assert!(!t.key(b"b", t0), "never a part of what was typed");
        assert_eq!(t.state(t0), Held::Dropped);
        assert_eq!(t.open(t0 + Duration::from_secs(1)), None);
    }
}
