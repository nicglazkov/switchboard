//! The empty board's search for a server: what it found so far and what the board shows for it. Plain state with no
//! drawing and no network, so every case is a unit test; the desktop app feeds it what the Bonjour search
//! (discover.rs) and the look at this computer's own port turn up.
//!
//! Searching, then one of: exactly one server (the app connects to it by itself), several (the user picks), none
//! ("No Switchboard server on this network"), or the search could not run and found nothing (an error, with the same
//! choices as none).

use std::time::Duration;

use crate::servers::key;

/// How long the board's search listens for answers. Servers show up as they answer; the result is decided when this
/// is up.
pub const SEARCH_FOR: Duration = Duration::from_millis(3000);

/// One server the search found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    /// What it goes by ("studio1"), or "This computer" for one that only answered here.
    pub name: String,
    /// Its address ("http://studio1.local:8130").
    pub url: String,
    /// It runs on this computer.
    pub here: bool,
}

impl Found {
    /// The address as the board shows it: no "http://".
    pub fn shown(&self) -> String {
        self.url.trim_start_matches("http://").to_string()
    }
}

/// What the board shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Show {
    /// Not searched yet: nothing to show but the empty board.
    Idle,
    /// Still listening; the servers found so far.
    Searching(Vec<Found>),
    /// Exactly one answered: the app connects to it.
    Connecting(Found),
    /// Several answered: the user picks.
    Pick(Vec<Found>),
    /// Nothing answered.
    None,
    /// The search could not run and nothing answered: why.
    Error(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Phase {
    #[default]
    Idle,
    Searching,
    Done,
}

/// One search after another. Each `start` gets a number, and what an older search reports late is ignored.
#[derive(Debug, Clone, Default)]
pub struct Search {
    run: u64,
    phase: Phase,
    found: Vec<Found>,
    error: Option<String>,
    /// The server this computer's own port answered for, with the addresses it said it has.
    local: Option<(String, Vec<String>)>,
    /// The single server was handed out to connect to (once per search).
    handed: bool,
}

impl Search {
    /// Starts a new search and returns its number.
    pub fn start(&mut self) -> u64 {
        self.run += 1;
        self.phase = Phase::Searching;
        self.found.clear();
        self.error = None;
        self.local = None;
        self.handed = false;
        self.run
    }

    pub fn started(&self) -> bool {
        self.phase != Phase::Idle
    }

    pub fn searching(&self) -> bool {
        self.phase == Phase::Searching
    }

    /// A server answered the Bonjour search. False when it is from an older search or already listed.
    pub fn add(&mut self, run: u64, name: &str, url: &str) -> bool {
        if run != self.run || self.phase != Phase::Searching {
            return false;
        }
        let k = key(url);
        if self.found.iter().any(|f| key(&f.url) == k) {
            return false;
        }
        let here = self.local.as_ref().is_some_and(|(_, addrs)| addrs.iter().any(|a| key(a) == k));
        let name =
            if name.trim().is_empty() { url.trim_start_matches("http://").to_string() } else { name.to_string() };
        // The server on this computer may have answered on its own port first: the network's name for it replaces
        // that row.
        if here {
            self.found.retain(|f| !(f.here && f.name == HERE));
        }
        self.found.push(Found { name, url: url.trim_end_matches('/').to_string(), here });
        true
    }

    /// This computer's own port answered at `url`, from a server that says it can be reached at `addresses`. A server
    /// on the list at one of those addresses is the same one and is marked as here; otherwise it is listed as
    /// "This computer" (a server does not hear its own Bonjour question on some systems).
    pub fn local(&mut self, run: u64, url: &str, addresses: &[String]) {
        if run != self.run || self.phase != Phase::Searching {
            return;
        }
        let keys: Vec<String> = addresses.iter().map(|a| key(a)).chain([key(url)]).collect();
        self.local = Some((url.to_string(), addresses.to_vec()));
        let mut marked = false;
        for f in self.found.iter_mut().filter(|f| keys.contains(&key(&f.url))) {
            f.here = true;
            marked = true;
        }
        if !marked {
            self.found.push(Found { name: HERE.into(), url: url.trim_end_matches('/').to_string(), here: true });
        }
    }

    /// The search is over; `error` when it could not run at all.
    pub fn finish(&mut self, run: u64, error: Option<String>) {
        if run != self.run || self.phase != Phase::Searching {
            return;
        }
        self.phase = Phase::Done;
        self.error = error;
    }

    /// What it found so far.
    pub fn found(&self) -> &[Found] {
        &self.found
    }

    /// Servers that run on other computers: the setup's "no other server" check.
    pub fn others(&self) -> Vec<Found> {
        self.found.iter().filter(|f| !f.here).cloned().collect()
    }

    /// The server on this computer, if one answered.
    pub fn here(&self) -> Option<Found> {
        self.found.iter().find(|f| f.here).cloned()
    }

    pub fn show(&self) -> Show {
        match self.phase {
            Phase::Idle => Show::Idle,
            Phase::Searching => Show::Searching(self.found.clone()),
            Phase::Done => match self.found.len() {
                1 => Show::Connecting(self.found[0].clone()),
                0 => match &self.error {
                    Some(e) => Show::Error(e.clone()),
                    None => Show::None,
                },
                _ => Show::Pick(self.found.clone()),
            },
        }
    }

    /// The one server to connect to by itself: only once per search, and only when exactly one answered.
    pub fn take_single(&mut self) -> Option<Found> {
        if self.handed {
            return None;
        }
        match self.show() {
            Show::Connecting(f) => {
                self.handed = true;
                Some(f)
            }
            _ => None,
        }
    }
}

/// The name for a server that only answered on this computer's own port.
pub const HERE: &str = "This computer";

/// The plain words for a search that could not run.
pub fn search_error(e: &std::io::Error) -> String {
    match e.kind() {
        std::io::ErrorKind::PermissionDenied => {
            "Local network permission not granted. On a Mac, turn on Switchboard in System Settings > Privacy & \
             Security > Local Network."
                .into()
        }
        _ => format!("Switchboard couldn't look on this network ({e})."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn searching_shows_each_server_as_it_answers() {
        let mut s = Search::default();
        assert_eq!(s.show(), Show::Idle);
        let r = s.start();
        assert_eq!(s.show(), Show::Searching(vec![]));
        assert!(s.add(r, "studio1", "http://studio1.local:8130/"));
        assert!(!s.add(r, "studio1", "http://STUDIO1.local:8130"), "the same address twice is one server");
        match s.show() {
            Show::Searching(f) => {
                assert_eq!(f.len(), 1);
                assert_eq!(f[0].url, "http://studio1.local:8130");
                assert_eq!(f[0].shown(), "studio1.local:8130");
            }
            o => panic!("{o:?}"),
        }
    }

    #[test]
    fn exactly_one_connects_by_itself_once() {
        let mut s = Search::default();
        let r = s.start();
        s.add(r, "studio1", "http://studio1.local:8130");
        assert_eq!(s.take_single(), None, "not while still searching");
        s.finish(r, None);
        assert!(matches!(s.show(), Show::Connecting(ref f) if f.name == "studio1"));
        assert_eq!(s.take_single().map(|f| f.name), Some("studio1".into()));
        assert_eq!(s.take_single(), None, "handed out once");
    }

    #[test]
    fn several_let_the_user_pick() {
        let mut s = Search::default();
        let r = s.start();
        s.add(r, "studio1", "http://studio1.local:8130");
        s.add(r, "tower", "http://192.0.2.20:8130");
        s.finish(r, None);
        assert!(matches!(s.show(), Show::Pick(ref f) if f.len() == 2));
        assert_eq!(s.take_single(), None);
    }

    #[test]
    fn none_and_error() {
        let mut s = Search::default();
        let r = s.start();
        s.finish(r, None);
        assert_eq!(s.show(), Show::None);
        let r = s.start();
        s.finish(r, Some("no network".into()));
        assert_eq!(s.show(), Show::Error("no network".into()));
        // The error only matters when nothing answered.
        let r = s.start();
        s.add(r, "studio1", "http://studio1.local:8130");
        s.finish(r, Some("no network".into()));
        assert!(matches!(s.show(), Show::Connecting(_)));
    }

    #[test]
    fn a_late_answer_from_an_older_search_is_ignored() {
        let mut s = Search::default();
        let old = s.start();
        let new = s.start();
        assert!(!s.add(old, "studio1", "http://studio1.local:8130"));
        s.finish(old, None);
        assert!(s.searching());
        s.finish(new, None);
        assert!(!s.add(new, "late", "http://192.0.2.9:8130"), "nothing joins a finished search");
        assert_eq!(s.show(), Show::None);
    }

    #[test]
    fn the_server_here_is_one_server_not_two() {
        let mut s = Search::default();
        let r = s.start();
        s.add(r, "studio1", "http://studio1.local:8130");
        s.local(r, "http://127.0.0.1:8130", &["http://192.0.2.10:8130".into(), "http://studio1.local:8130".into()]);
        assert_eq!(s.found().len(), 1);
        assert!(s.found()[0].here);
        assert!(s.others().is_empty());
        // Answered here first, then on the network: the network's name replaces "This computer".
        let r = s.start();
        s.local(r, "http://127.0.0.1:8130", &["http://studio1.local:8130".into()]);
        assert_eq!(s.found()[0].name, HERE);
        s.add(r, "studio1", "http://studio1.local:8130");
        assert_eq!(s.found().len(), 1);
        assert_eq!(s.found()[0].name, "studio1");
        assert!(s.found()[0].here);
        // A server elsewhere and one here: two, and only one counts as another server.
        s.add(r, "tower", "http://192.0.2.20:8130");
        s.finish(r, None);
        assert!(matches!(s.show(), Show::Pick(_)));
        assert_eq!(s.others().len(), 1);
        assert_eq!(s.here().map(|f| f.name), Some("studio1".into()));
    }

    #[test]
    fn plain_words_for_a_search_that_could_not_run() {
        let e = std::io::Error::from(std::io::ErrorKind::PermissionDenied);
        assert!(search_error(&e).contains("Local Network"));
        assert!(!search_error(&e).contains('\u{2014}'));
    }
}
