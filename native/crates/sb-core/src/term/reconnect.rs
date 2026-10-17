//! When a terminal comes back by itself (spec 2026-10-14, terminal pipeline). A link that drops while attached
//! reconnects with backoff: 0.5 s, doubling to 10 s, for about five minutes, then it waits for the person. The
//! screen and its scrollback stay; the hub's snapshot brings the screen up to date once it is back.

use std::time::Duration;

use super::ws::Closed;
use super::{Need, TermError};

const FIRST: Duration = Duration::from_millis(500);
const MOST: Duration = Duration::from_secs(10);
/// Attempts before giving up: 0.5 + 1 + 2 + 4 + 8, then 10 s apart, comes to about five minutes.
pub const ATTEMPTS: u32 = 34;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Backoff {
    /// Attempts made so far in this run of them.
    pub attempt: u32,
}

impl Backoff {
    /// The wait before the next attempt, or None once it is time to stop and show the card.
    pub fn wait(&mut self) -> Option<Duration> {
        if self.attempt >= ATTEMPTS {
            return None;
        }
        let d = FIRST.saturating_mul(1u32 << self.attempt.min(10)).min(MOST);
        self.attempt += 1;
        Some(d)
    }
}

/// How a connection ended, and whether coming back on its own makes sense.
/// `exit`: what the hub said `claude attach` exited with, if it said. `attached`: the terminal had been open (or
/// was already coming back), so the person expects it to stay.
pub fn after_close(closed: &Closed, exit: Option<Option<i64>>, attached: bool) -> bool {
    if !attached {
        return false;
    }
    match exit {
        // ssh's own "could not reach the machine, or the connection dropped": worth another go
        Some(Some(255)) => true,
        // `claude attach` ended (detached elsewhere, the chat stopped): the person decides
        Some(_) => false,
        None => matches!(closed, Closed::Lost(_) | Closed::Failed(_) | Closed::Limit),
    }
}

/// While coming back, which ticket errors are worth another go (the server or the machine is briefly away), and
/// which need the person (the chat stopped, it is open in a terminal on its machine).
pub fn ticket_retry(e: &TermError) -> bool {
    match e {
        TermError::Need { need, .. } => *need == Need::Offline,
        TermError::Core(_) => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backs_off_to_ten_seconds_then_stops() {
        let mut b = Backoff::default();
        let waits: Vec<f32> = std::iter::from_fn(|| b.wait()).map(|d| d.as_secs_f32()).collect();
        assert_eq!(&waits[..6], &[0.5, 1.0, 2.0, 4.0, 8.0, 10.0]);
        assert!(waits[6..].iter().all(|w| *w == 10.0));
        assert_eq!(waits.len() as u32, ATTEMPTS);
        let total: f32 = waits.iter().sum();
        assert!((280.0..=330.0).contains(&total), "about five minutes: {total}");
        assert_eq!(b.wait(), None);
    }

    #[test]
    fn which_ends_come_back() {
        let lost = Closed::Lost("The terminal hub stopped answering.".into());
        assert!(after_close(&lost, None, true), "a dropped link while attached");
        assert!(!after_close(&lost, None, false), "never attached: show why instead");
        assert!(after_close(&Closed::Failed("Could not reach the terminal hub.".into()), None, true), "the hub is away for a moment");
        assert!(after_close(&lost, Some(Some(255)), true), "ssh lost the machine");
        assert!(!after_close(&Closed::Normal, Some(Some(0)), true), "detached on purpose");
        assert!(!after_close(&Closed::Normal, Some(Some(1)), true), "the attach ended with an error");
        assert!(!after_close(&Closed::ByClient, None, true));
        assert!(!after_close(&Closed::Origin, None, true));
        assert!(!after_close(&Closed::Ticket, None, true));
    }

    #[test]
    fn which_ticket_errors_retry() {
        use crate::net::CoreError;
        assert!(ticket_retry(&TermError::Core(CoreError::Bad("timeout".into()))));
        assert!(ticket_retry(&TermError::Need { need: Need::Offline, message: String::new() }));
        assert!(!ticket_retry(&TermError::Need { need: Need::Resume, message: String::new() }));
        assert!(!ticket_retry(&TermError::Need { need: Need::Terminal, message: String::new() }));
    }
}
