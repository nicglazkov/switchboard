//! The phone while the home computer is away.
//!
//! The two loopback ports lead to the home computer. When it has not been reached for a few seconds
//! (`sb_link::standin::AWAY_AFTER_PHONE`), the same ports answer from what the other computers say about their
//! own chats, exactly as a computer's ports do: the stand-in is sb-link's, and so is the reaching of the other
//! computers and the comparing of lists with them (`sb_link::reach`). This file is only the phone's side of it:
//! where the streams come from, and the list of devices the phone keeps.
//!
//! The list (`roster.json` in the app's data folder) is what the home computer signed: which computers there
//! are, and where they were last reached. Nothing in it is secret. It is asked for each time home is reached
//! and once a minute after that, and it is what lets the phone find the other computers when home is not there
//! to ask. A phone answers for no chats of its own, so nobody connects to it and it blocks nobody.

use crate::gate::Carrier;
use sb_link::identity::DeviceId;
use sb_link::reach::{self, Take};
use sb_link::roster::{Envelope, Roster, RosterCache, RosterDevice};
use sb_link::standin::{Backend, Fut, Io};
use sb_link::tunnel::Target;
use std::io;
use std::path::Path;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

const NAME: &str = "roster.json";

fn unix_now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// The list of devices kept in `dir` for a phone paired with `home`. A file for another home computer, or one
/// that cannot be read, is no list: the next one home hands over takes its place.
pub fn list_in(dir: &Path, home: DeviceId) -> Arc<RosterCache> {
    let _ = std::fs::create_dir_all(dir);
    Arc::new(RosterCache::load(&dir.join(NAME), home))
}

/// How the phone takes a list the home computer signed: it keeps the newer one, and that is all.
pub fn take_into(cache: &Arc<RosterCache>) -> Take {
    let cache = cache.clone();
    Arc::new(move |envelope: &Envelope| cache.offer(envelope, unix_now()))
}

/// The phone, as the stand-in sees it.
pub struct Reach {
    pub carrier: Arc<Carrier>,
    pub cache: Arc<RosterCache>,
    /// Called when away begins or ends, so the app is told at once.
    pub changed: Box<dyn Fn() + Send + Sync>,
}

fn no_link() -> io::Error {
    io::Error::new(io::ErrorKind::NotConnected, "not connected to the home computer")
}

impl Backend for Reach {
    fn home_up(&self) -> bool {
        // A home computer that answers "you are not on my list" is not away: the phone was removed, and it is
        // not shown the other computers' chats in its place.
        self.carrier.get().is_some_and(|(home, _)| {
            let s = home.status();
            s.connected || s.refused
        })
    }

    fn open_home(&self, target: Target) -> Fut<'_, io::Result<Io>> {
        Box::pin(async move {
            let (home, _) = self.carrier.get().ok_or_else(no_link)?;
            Ok(Box::new(reach::open_to(&home, target).await?) as Io)
        })
    }

    fn roster(&self) -> Option<Arc<Roster>> {
        self.cache.get()
    }

    fn open_own(&self, device: &RosterDevice) -> Fut<'_, io::Result<Io>> {
        let device = device.clone();
        Box::pin(async move {
            let (_, others) = self.carrier.get().ok_or_else(no_link)?;
            Ok(Box::new(others.open(&device).await?) as Io)
        })
    }

    fn away_changed(&self, away: bool) {
        if !away && let Some((_, others)) = self.carrier.get() {
            others.drop_all();
        }
        (self.changed)();
    }
}
