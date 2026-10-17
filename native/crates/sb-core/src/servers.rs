//! The saved server addresses: the ones the user typed, and the ones the server said it has (the `addresses` in its
//! summary, server/addresses.py), learned on every good connect. So an app that reached the server once at home also
//! has its Tailscale address for when it is away, with nothing typed.
//!
//! Rules: an address the user typed is never dropped or moved by the app; a learned one is added at the end, marked
//! as learned, and may go when the list is full or when it has failed for weeks while another address worked. The
//! address that worked last is tried first.
//!
//! In settings.json the list stays `servers` (plain strings, so an older build still reads it) and what the app knows
//! about each one is beside it in `server_notes`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::api::ApiAddress;
use crate::net::normalize_base;

/// The most addresses kept. Typed ones always stay, so the list can only be longer when the user typed that many.
pub const MAX: usize = 12;
/// A learned address that failed for this long, while another one worked, is dropped.
pub const STALE_AFTER: f64 = 21.0 * 86_400.0;
/// Times are written down at most this often, so a poll every few seconds does not rewrite the settings file.
pub const NOTE_EVERY: f64 = 3_600.0;
/// Learned addresses the user removed, remembered so they are not learned again.
const HIDDEN_MAX: usize = 32;

/// What the app knows about one saved address.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Note {
    /// The server listed it; the user did not type it.
    #[serde(default)]
    pub learned: bool,
    /// lan | bonjour | tailscale | magicdns, as the server said.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// When the app last used it (seconds since the epoch).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_ok: Option<f64>,
    /// When it started failing; cleared once it answers again.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failing_since: Option<f64>,
    /// When the server last listed it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub advertised: Option<f64>,
}

/// The saved list, as it sits in the desktop app's settings.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Saved {
    /// Every address, typed and learned, in the order they were added ("http://192.0.2.10:8130").
    #[serde(default)]
    pub servers: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub server_notes: BTreeMap<String, Note>,
    /// Learned addresses the user removed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub server_hidden: Vec<String>,
}

/// One way to compare addresses: scheme and host in lower case, the port always written, no trailing slash.
pub fn key(url: &str) -> String {
    normalize_base(url).unwrap_or_else(|| url.trim().trim_end_matches('/').to_string()).to_ascii_lowercase()
}

impl Saved {
    fn find(&self, url: &str) -> Option<usize> {
        let k = key(url);
        self.servers.iter().position(|u| key(u) == k)
    }

    pub fn note(&self, url: &str) -> Option<&Note> {
        self.server_notes.get(self.servers.get(self.find(url)?)?)
    }

    /// The server listed it (the user did not type it).
    pub fn is_learned(&self, url: &str) -> bool {
        self.note(url).is_some_and(|n| n.learned)
    }

    /// An address the user typed: first in the list, and never dropped. Already there (typed or learned), it moves
    /// to the front and counts as typed.
    pub fn add_typed(&mut self, base: &str) {
        let url = match self.find(base) {
            Some(i) => self.servers.remove(i),
            None => base.to_string(),
        };
        self.server_hidden.retain(|h| key(h) != key(&url));
        let n = self.server_notes.entry(url.clone()).or_default();
        n.learned = false;
        n.failing_since = None;
        self.servers.insert(0, url);
    }

    /// The user's Remove: gone, typed or learned. A learned one is not learned again until the user types it.
    pub fn remove(&mut self, url: &str) -> bool {
        let Some(i) = self.find(url) else { return false };
        let u = self.servers.remove(i);
        if self.server_notes.remove(&u).is_some_and(|n| n.learned) {
            self.server_hidden.push(u);
            let over = self.server_hidden.len().saturating_sub(HIDDEN_MAX);
            self.server_hidden.drain(..over);
        }
        true
    }

    /// Adds what the server listed. Returns whether anything changed (so the caller saves).
    pub fn learn(&mut self, listed: &[ApiAddress], now: f64) -> bool {
        let mut changed = self.tidy();
        for a in listed {
            let Some(url) = normalize_base(&a.url) else { continue };
            if self.server_hidden.iter().any(|h| key(h) == key(&url)) {
                continue;
            }
            let kind = (!a.kind.is_empty()).then(|| a.kind.clone());
            match self.find(&url) {
                Some(i) => {
                    let n = self.server_notes.entry(self.servers[i].clone()).or_default();
                    if n.learned && kind.is_some() && n.kind != kind {
                        n.kind = kind;
                        changed = true;
                    }
                    if n.advertised.is_none_or(|t| now - t >= NOTE_EVERY) {
                        n.advertised = Some(now);
                        changed = true;
                    }
                }
                None => {
                    self.servers.push(url.clone());
                    self.server_notes
                        .insert(url, Note { learned: true, kind, advertised: Some(now), ..Note::default() });
                    changed = true;
                }
            }
        }
        changed | self.cap()
    }

    /// The address the app is using now answered. One found on the local network that is not saved yet joins the
    /// list as learned (unless the user removed it).
    pub fn worked(&mut self, url: &str, now: f64) -> bool {
        let mut changed = false;
        let i = match self.find(url) {
            Some(i) => i,
            None => {
                let Some(u) = normalize_base(url) else { return false };
                if self.server_hidden.iter().any(|h| key(h) == key(&u)) {
                    return false;
                }
                self.servers.push(u.clone());
                self.server_notes.insert(u, Note { learned: true, ..Note::default() });
                changed = true;
                self.servers.len() - 1
            }
        };
        let u = self.servers[i].clone();
        let was_last = self.last_good().is_some_and(|l| l == u);
        let n = self.server_notes.entry(u).or_default();
        changed |= n.failing_since.take().is_some();
        if !was_last || n.last_ok.is_none_or(|t| now - t >= NOTE_EVERY) {
            n.last_ok = Some(now);
            changed = true;
        }
        changed | self.cap()
    }

    /// A probe of this address got a Switchboard answer (it was not the one picked, so it does not become the last
    /// one that worked).
    pub fn answered(&mut self, url: &str) -> bool {
        let Some(i) = self.find(url) else { return false };
        self.server_notes.get_mut(&self.servers[i]).is_some_and(|n| n.failing_since.take().is_some())
    }

    /// A probe of this address got no answer.
    pub fn failed(&mut self, url: &str, now: f64) -> bool {
        let Some(i) = self.find(url) else { return false };
        let n = self.server_notes.entry(self.servers[i].clone()).or_default();
        if n.failing_since.is_some() {
            return false;
        }
        n.failing_since = Some(now);
        true
    }

    /// The address that worked most recently.
    pub fn last_good(&self) -> Option<String> {
        let mut best: Option<(f64, &String)> = None;
        for u in &self.servers {
            if let Some(t) = self.server_notes.get(u).and_then(|n| n.last_ok)
                && best.is_none_or(|(b, _)| t > b)
            {
                best = Some((t, u));
            }
        }
        best.map(|(_, u)| u.clone())
    }

    /// The order to try them in: the last one that worked, then the rest as saved.
    pub fn connect_order(&self) -> Vec<String> {
        let last = self.last_good();
        let mut out: Vec<String> = last.iter().cloned().collect();
        out.extend(self.servers.iter().filter(|u| Some(*u) != last.as_ref()).cloned());
        out
    }

    /// Drops learned addresses that failed for weeks while another one worked, and that the server stopped listing
    /// since. Typed ones always stay.
    pub fn prune(&mut self, now: f64) -> bool {
        let newest_ok = self.server_notes.values().filter_map(|n| n.last_ok).fold(f64::MIN, f64::max);
        let stale = |n: &Note| {
            n.learned
                && n.failing_since.is_some_and(|f| {
                    now - f >= STALE_AFTER && newest_ok >= f + STALE_AFTER && n.advertised.is_none_or(|a| a < f)
                })
        };
        let gone: Vec<String> =
            self.servers.iter().filter(|u| self.server_notes.get(*u).is_some_and(stale)).cloned().collect();
        for u in &gone {
            self.servers.retain(|s| s != u);
            self.server_notes.remove(u);
        }
        !gone.is_empty()
    }

    /// An upgrade from a build that had the server's address built in leaves no list. Seeds it from the server that
    /// answered last (`last`, from the snapshot cache, at `at`) and the addresses that answer carried, so the app
    /// connects at once, at home or away. Does nothing when a list is already saved.
    pub fn migrate(&mut self, last: Option<&str>, at: f64, listed: &[ApiAddress], now: f64) -> bool {
        if !self.servers.is_empty() {
            return false;
        }
        if let Some(url) = last.and_then(normalize_base) {
            self.servers.push(url.clone());
            self.server_notes.insert(url, Note { learned: true, last_ok: Some(at), ..Note::default() });
        }
        self.learn(listed, now);
        !self.servers.is_empty()
    }

    /// `migrate` from the snapshot cache in `dir` (the file every build since the first one writes).
    pub fn migrate_from_cache(&mut self, dir: &std::path::Path, now: f64) -> bool {
        if !self.servers.is_empty() {
            return false;
        }
        let Some((last, at, listed)) = crate::cache::last_seen(dir) else { return false };
        self.migrate(last.as_deref(), at, &listed, now)
    }

    /// No two entries for one address, and no notes for addresses that are gone.
    fn tidy(&mut self) -> bool {
        let before = (self.servers.len(), self.server_notes.len());
        let mut seen: Vec<String> = Vec::new();
        self.servers.retain(|u| {
            let k = key(u);
            let fresh = !seen.contains(&k);
            seen.push(k);
            fresh
        });
        let keep = &self.servers;
        self.server_notes.retain(|u, _| keep.contains(u));
        before != (self.servers.len(), self.server_notes.len())
    }

    /// Back to `MAX`, dropping learned ones: the longest failing first, then the ones used longest ago (never used
    /// counts as oldest), the newest added first among equals.
    fn cap(&mut self) -> bool {
        let mut changed = false;
        while self.servers.len() > MAX {
            let rank = |i: usize, u: &String| {
                let n = self.server_notes.get(u)?;
                if !n.learned {
                    return None;
                }
                Some(match n.failing_since {
                    Some(f) => (0, f, -(i as f64)),
                    None => (1, n.last_ok.unwrap_or(f64::MIN), -(i as f64)),
                })
            };
            let victim = self
                .servers
                .iter()
                .enumerate()
                .filter_map(|(i, u)| Some((rank(i, u)?, i)))
                .min_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal))
                .map(|(_, i)| i);
            let Some(i) = victim else { break };
            let u = self.servers.remove(i);
            self.server_notes.remove(&u);
            changed = true;
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: f64 = 86_400.0;

    fn adv(kind: &str, url: &str) -> ApiAddress {
        ApiAddress { kind: kind.into(), host: String::new(), url: url.into() }
    }

    fn home() -> Vec<ApiAddress> {
        vec![
            adv("lan", "http://192.0.2.10:8130"),
            adv("bonjour", "http://studio.local:8130"),
            adv("tailscale", "http://100.64.0.10:8130"),
            adv("magicdns", "http://studio.tail0000.example.ts.net:8130"),
        ]
    }

    fn typed(urls: &[&str]) -> Saved {
        let mut s = Saved::default();
        for u in urls.iter().rev() {
            s.add_typed(u);
        }
        s
    }

    #[test]
    fn learned_addresses_go_after_typed_ones_and_are_marked() {
        let mut s = typed(&["http://192.0.2.20:8130"]);
        assert!(s.learn(&home(), 1000.0));
        assert_eq!(s.servers[0], "http://192.0.2.20:8130", "the typed one stays first");
        assert_eq!(s.servers.len(), 5);
        assert!(!s.is_learned("http://192.0.2.20:8130"));
        assert!(s.is_learned("http://100.64.0.10:8130"));
        assert_eq!(s.note("http://100.64.0.10:8130").unwrap().kind.as_deref(), Some("tailscale"));
        assert!(!s.learn(&home(), 1010.0), "the same list again changes nothing (no save every poll)");
        assert!(s.learn(&home(), 1000.0 + NOTE_EVERY), "an hour on, it notes the server still lists them");
    }

    #[test]
    fn duplicates_are_dropped_and_typed_ones_never_move() {
        let mut s = typed(&["http://studio.local:8130", "http://192.0.2.10:8130"]);
        s.servers.push("HTTP://192.0.2.10:8130/".into());
        s.learn(&home(), 1000.0);
        assert_eq!(s.servers.iter().filter(|u| key(u) == "http://192.0.2.10:8130").count(), 1);
        assert_eq!(&s.servers[..2], &["http://studio.local:8130", "http://192.0.2.10:8130"]);
        assert!(!s.is_learned("http://192.0.2.10:8130"), "listed by the server too, it stays typed");
        assert!(s.note("http://studio.local:8130").is_some_and(|n| n.kind.is_none()));
        // bad entries from the server are skipped
        s.learn(&[adv("lan", "ftp://192.0.2.30"), adv("lan", "")], 1000.0);
        assert_eq!(s.servers.len(), 4);
    }

    #[test]
    fn the_list_is_capped_by_dropping_learned_ones() {
        let typed_ones: Vec<String> = (0..3).map(|i| format!("http://192.0.2.{}:8130", 20 + i)).collect();
        let mut s = typed(&typed_ones.iter().map(String::as_str).collect::<Vec<_>>());
        let many: Vec<ApiAddress> = (0..20).map(|i| adv("lan", &format!("http://192.0.2.{}:8130", 100 + i))).collect();
        s.learn(&many[..10], 1000.0);
        s.worked("http://192.0.2.101:8130", 1000.0);
        s.failed("http://192.0.2.102:8130", 1000.0);
        s.learn(&many[10..], 1001.0);
        assert_eq!(s.servers.len(), MAX);
        assert_eq!(&s.servers[..3], &typed_ones[..], "typed ones are never dropped");
        assert!(s.find("http://192.0.2.101:8130").is_some(), "the one in use stays");
        assert!(s.find("http://192.0.2.102:8130").is_none(), "a failing one goes first");
        assert_eq!(s.server_notes.len(), MAX);
        // all typed: over the cap is allowed
        let lots: Vec<String> = (0..MAX + 2).map(|i| format!("http://192.0.2.{}:8130", 150 + i)).collect();
        let mut t = typed(&lots.iter().map(String::as_str).collect::<Vec<_>>());
        t.learn(&home(), 1.0);
        assert_eq!(t.servers.len(), MAX + 2);
    }

    #[test]
    fn the_last_one_that_worked_is_tried_first() {
        let mut s = typed(&["http://192.0.2.20:8130"]);
        s.learn(&home(), 1000.0);
        assert_eq!(s.connect_order(), s.servers, "nothing has worked yet: saved order");
        assert!(s.worked("http://100.64.0.10:8130", 2000.0));
        let order = s.connect_order();
        assert_eq!(order[0], "http://100.64.0.10:8130");
        assert_eq!(order[1], "http://192.0.2.20:8130");
        assert_eq!(order.len(), s.servers.len());
        assert_eq!(s.servers[0], "http://192.0.2.20:8130", "the saved order itself does not change");
        assert!(!s.worked("http://100.64.0.10:8130", 2010.0), "the same one again: nothing to save");
        assert!(s.worked("http://192.0.2.10:8130", 2020.0), "a switch is noted at once");
        assert_eq!(s.connect_order()[0], "http://192.0.2.10:8130");
        // another one answering a probe clears its failure but does not take over
        s.failed("http://100.64.0.10:8130", 2030.0);
        assert!(s.answered("http://100.64.0.10:8130"));
        assert_eq!(s.connect_order()[0], "http://192.0.2.10:8130");
        // one found on the local network joins as learned, and is first next time
        assert!(s.worked("http://192.0.2.40:8130", 2040.0));
        assert!(s.is_learned("http://192.0.2.40:8130"));
        assert_eq!(s.connect_order()[0], "http://192.0.2.40:8130");
    }

    #[test]
    fn a_learned_address_that_fails_for_weeks_goes_but_a_typed_one_stays() {
        let mut s = typed(&["http://192.0.2.20:8130"]);
        s.learn(&home(), 0.0);
        s.failed("http://192.0.2.20:8130", DAY);
        s.failed("http://192.0.2.10:8130", DAY);
        s.failed("http://studio.local:8130", DAY);
        assert!(!s.failed("http://192.0.2.10:8130", 2.0 * DAY), "it keeps the first failure");
        s.worked("http://100.64.0.10:8130", 10.0 * DAY);
        assert!(!s.prune(30.0 * DAY), "nothing else has worked since the failures started three weeks ago");
        // the server still lists the LAN address (it is just not reachable from here): it stays
        s.learn(&home()[..1], 25.0 * DAY);
        s.worked("http://100.64.0.10:8130", 30.0 * DAY);
        assert!(s.prune(30.0 * DAY));
        assert!(s.find("http://192.0.2.10:8130").is_some(), "still listed by the server");
        assert!(s.find("http://studio.local:8130").is_none(), "failing three weeks, not listed since: dropped");
        assert!(s.find("http://192.0.2.20:8130").is_some(), "typed: never dropped");
        assert!(!s.server_notes.contains_key("http://studio.local:8130"));
    }

    #[test]
    fn an_answer_clears_a_failure() {
        let mut s = Saved::default();
        s.learn(&home(), 0.0);
        s.failed("http://studio.local:8130", DAY);
        s.answered("http://studio.local:8130");
        s.worked("http://100.64.0.10:8130", 40.0 * DAY);
        assert!(!s.prune(40.0 * DAY));
    }

    #[test]
    fn a_removed_learned_address_is_not_learned_again() {
        let mut s = typed(&["http://192.0.2.20:8130"]);
        s.learn(&home(), 0.0);
        assert!(s.remove("http://studio.local:8130"));
        s.learn(&home(), NOTE_EVERY * 2.0);
        assert!(s.find("http://studio.local:8130").is_none());
        assert!(s.remove("http://192.0.2.20:8130"), "typed ones can be removed by the user too");
        assert!(!s.remove("http://192.0.2.20:8130"));
        s.add_typed("http://studio.local:8130");
        assert_eq!(s.servers[0], "http://studio.local:8130");
        assert!(!s.is_learned("http://studio.local:8130"));
        assert!(s.server_hidden.is_empty());
    }

    #[test]
    fn typing_a_learned_address_makes_it_typed() {
        let mut s = Saved::default();
        s.learn(&home(), 0.0);
        s.add_typed("http://100.64.0.10:8130");
        assert_eq!(s.servers[0], "http://100.64.0.10:8130");
        assert!(!s.is_learned("http://100.64.0.10:8130"));
        assert_eq!(s.servers.len(), 4);
    }

    #[test]
    fn an_upgrade_seeds_the_list_from_the_last_server() {
        let mut s = Saved::default();
        let cached = home();
        assert!(s.migrate(Some("http://192.0.2.10:8130"), 500.0, &cached, 1000.0));
        assert_eq!(s.connect_order()[0], "http://192.0.2.10:8130", "the last server is tried first");
        assert!(s.find("http://100.64.0.10:8130").is_some(), "and the tailnet address it listed, for away from home");
        assert_eq!(s.servers.len(), 4);
        assert!(s.is_learned("http://192.0.2.10:8130"));
        // an older server sent no addresses: the last server alone
        let mut old = Saved::default();
        assert!(old.migrate(Some("http://100.64.0.10:8130"), 500.0, &[], 1000.0));
        assert_eq!(old.servers, vec!["http://100.64.0.10:8130"]);
        // nothing known: nothing to seed, and the local network search runs as before
        let mut none = Saved::default();
        assert!(!none.migrate(None, 0.0, &[], 1000.0));
        assert!(none.servers.is_empty());
        // a list already saved is left alone
        let mut kept = typed(&["http://192.0.2.20:8130"]);
        assert!(!kept.migrate(Some("http://192.0.2.10:8130"), 500.0, &cached, 1000.0));
        assert_eq!(kept.servers, vec!["http://192.0.2.20:8130"]);
    }

    #[test]
    fn an_upgrade_reads_the_cache_an_older_build_wrote() {
        let dir = std::env::temp_dir().join(format!("sb-servers-migrate-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut none = Saved::default();
        assert!(!none.migrate_from_cache(&dir, 1000.0), "no cache: nothing to seed");
        // an older build's reply from a server that already lists its addresses
        let raw = serde_json::json!({"machines": [], "summary": {"sessions": 1, "addresses": {"port": 8130, "list": [
            {"kind": "lan", "host": "192.0.2.10", "url": "http://192.0.2.10:8130"},
            {"kind": "tailscale", "host": "100.64.0.10", "url": "http://100.64.0.10:8130"}]}}});
        crate::cache::save_sessions(&dir, raw.to_string().as_bytes(), 500.0, "http://192.0.2.10:8130").unwrap();
        let mut s = Saved::default();
        assert!(s.migrate_from_cache(&dir, 1000.0));
        assert_eq!(s.servers, vec!["http://192.0.2.10:8130", "http://100.64.0.10:8130"]);
        assert_eq!(s.note("http://192.0.2.10:8130").unwrap().last_ok, Some(500.0));
        assert_eq!(s.note("http://100.64.0.10:8130").unwrap().kind.as_deref(), Some("tailscale"));
        // a reply from a server with no addresses yet: the last server alone
        crate::cache::save_sessions(&dir, br#"{"machines": [], "summary": {}}"#, 600.0, "http://100.64.0.10:8130")
            .unwrap();
        let mut old = Saved::default();
        assert!(old.migrate_from_cache(&dir, 1000.0));
        assert_eq!(old.servers, vec!["http://100.64.0.10:8130"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_older_settings_file_still_reads() {
        let s: Saved = serde_json::from_str(r#"{"servers": ["http://192.0.2.10:8130"]}"#).unwrap();
        assert_eq!(s.servers, vec!["http://192.0.2.10:8130"]);
        assert!(!s.is_learned("http://192.0.2.10:8130"), "an address saved before notes existed counts as typed");
        let out = serde_json::to_value(&s).unwrap();
        assert_eq!(out, serde_json::json!({"servers": ["http://192.0.2.10:8130"]}));
    }
}
