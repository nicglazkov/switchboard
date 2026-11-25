//! How a device that is not home reaches the computers on its list, and keeps that list fresh.
//!
//! The stand-in (`standin`) asks a `Backend` for streams and knows nothing of how they are made. This is the
//! making, the same for a computer's helper and a phone's library, so the two cannot drift apart:
//!
//! - one tunnel to each other computer, made when it is first asked and kept while the home computer is away
//!   (`Others::open`);
//! - before a computer is asked, the first time on a connection and every ten minutes after, the two compare
//!   the lists of devices they hold, and the newer one stands on both sides (`Others::compare`). Only the home
//!   computer can sign a list, so nothing is taken on trust. That is how a device that was off learns, with home
//!   away, that another was removed;
//! - asking the home computer for its list over the tunnel to it (`Others::pull`).
//!
//! What "taking a list" means is the caller's (`Take`): a computer also prunes its blocks and cuts devices the
//! new list no longer names; a phone only keeps it.

use crate::endpoint::{LinkEndpoint, LinkStream, Peer, Tunnel};
use crate::http1;
use crate::identity::DeviceId;
use crate::roster::{Envelope, RosterCache, RosterDevice, RosterError, Stamp};
use crate::tunnel::{self, Target};
use serde_json::Value;
use std::collections::HashMap;
use std::io;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::task::JoinHandle;

/// How often two computers compare the lists of devices they hold, while they keep talking with home away.
pub const COMPARE_EVERY: Duration = Duration::from_secs(600);
/// How long the other computer may take over one question of that.
pub const COMPARE_WAIT: Duration = Duration::from_secs(5);
/// How long a computer may take to be reached, when it is asked while the home computer is away.
pub const REACH_WAIT: Duration = Duration::from_secs(6);
/// How long the home computer may take to hand over its list.
pub const PULL_WAIT: Duration = Duration::from_secs(15);
/// The most a list of devices may be on the wire.
const LIST_MAX: usize = 256 * 1024;

/// Takes a list of devices the home computer signed, if it is newer than the one held. Ok(true): it was taken.
pub type Take = Arc<dyn Fn(&Envelope) -> Result<bool, RosterError> + Send + Sync>;
/// Something for the device's log, as a sentence without a prefix.
pub type Note = Arc<dyn Fn(String) + Send + Sync>;

/// A connection to another computer, its task, and when the two last compared their lists of devices.
type Kept = (Arc<Tunnel>, JoinHandle<()>, Arc<Mutex<Option<Instant>>>);

/// The other computers, as reached from this device over one endpoint.
pub struct Others {
    ep: Arc<LinkEndpoint>,
    cache: Arc<RosterCache>,
    take: Take,
    note: Note,
    kept: Mutex<HashMap<DeviceId, Kept>>,
    /// A list the home key signed that is newer than the one held and came from a home computer that refuses
    /// this device (`show`). It is not taken: it is handed to the home computer that does answer.
    carried: Mutex<Option<Envelope>>,
}

/// A new stream on `tunnel` that starts with the line saying where it goes.
pub async fn open_to(tunnel: &Tunnel, target: Target) -> io::Result<LinkStream> {
    let mut s = tunnel.open().await?;
    let wrote = tokio::time::timeout(tunnel::HEADER_TIMEOUT, tunnel::write_header(&mut s, target)).await;
    wrote.map_err(|_| io::Error::from(io::ErrorKind::TimedOut))??;
    Ok(s)
}

impl Others {
    pub fn new(ep: Arc<LinkEndpoint>, cache: Arc<RosterCache>, take: Take, note: Note) -> Others {
        Others { ep, cache, take, note, kept: Mutex::default(), carried: Mutex::default() }
    }

    /// The connection to `device`, and whether it is time the two compared their lists of devices.
    fn tunnel_to(&self, device: &RosterDevice) -> (Arc<Tunnel>, bool) {
        let mut kept = self.kept.lock().unwrap_or_else(|e| e.into_inner());
        let (t, _, compared) = kept.entry(device.id).or_insert_with(|| {
            let relay = self.cache.get().and_then(|r| r.body.relay.clone());
            let t = Tunnel::new(self.ep.clone(), Peer { id: device.id, relay, addrs: device.addrs.clone() });
            (t.clone(), tokio::spawn(t.clone().run()), Arc::default())
        });
        let mut at = compared.lock().unwrap_or_else(|e| e.into_inner());
        let due = at.is_none_or(|t| t.elapsed() >= COMPARE_EVERY);
        if due {
            *at = Some(Instant::now());
        }
        (t.clone(), due)
    }

    /// Ends every connection to another computer: the home computer answers again, or this device is stopping.
    pub fn drop_all(&self) {
        for (_, (t, task, _)) in self.kept.lock().unwrap_or_else(|e| e.into_inner()).drain() {
            task.abort();
            t.close();
        }
    }

    /// How many other computers a connection is kept to.
    pub fn kept(&self) -> usize {
        self.kept.lock().unwrap_or_else(|e| e.into_inner()).len()
    }

    /// Before another computer is asked for its chats: this device says which list of devices it holds, and
    /// takes the other's if that is newer. Err: the other computer said nothing useful, and the next question may
    /// still work.
    pub async fn compare(&self, tunnel: &Tunnel) -> Result<bool, String> {
        async fn ask(tunnel: &Tunnel, method: &str, body: Option<&[u8]>) -> Result<http1::Answer, String> {
            let asked = async {
                let mut s = tunnel.open().await?;
                tunnel::write_header(&mut s, Target::Own).await?;
                http1::ask(&mut s, method, "/own/v1/roster", body, LIST_MAX).await
            };
            let a = tokio::time::timeout(COMPARE_WAIT, asked).await;
            a.map_err(|_| "it did not answer".to_string())?.map_err(|e| e.to_string())
        }
        let mine = self.cache.get();
        if let Some(r) = &mine {
            let body = serde_json::to_vec(&r.envelope).map_err(|e| e.to_string())?;
            let a = ask(tunnel, "POST", Some(&body)).await?;
            let have: Option<Stamp> = serde_json::from_slice::<Value>(&a.body)
                .ok()
                .and_then(|v| serde_json::from_value(v["have"].clone()).ok());
            if a.status == 200 && have == Some(r.stamp()) {
                return Ok(false); // the two hold the same one now
            }
        }
        let a = ask(tunnel, "GET", None).await?;
        if a.status != 200 {
            return Ok(false);
        }
        let envelope: Envelope = serde_json::from_slice(&a.body).map_err(|e| e.to_string())?;
        (self.take)(&envelope).map_err(|e| e.to_string())
    }

    /// A stream to `device`'s own surface. PermissionDenied: it refuses this device, or the newer list the two
    /// just compared no longer names it. NotConnected: it was not reached in `REACH_WAIT`.
    pub async fn open(&self, device: &RosterDevice) -> io::Result<LinkStream> {
        let (tunnel, compare) = self.tunnel_to(device);
        let end = Instant::now() + REACH_WAIT;
        loop {
            let s = tunnel.status();
            if s.connected {
                break;
            }
            if s.refused {
                return Err(io::Error::new(io::ErrorKind::PermissionDenied, "refused"));
            }
            if Instant::now() >= end {
                let why = s.error.unwrap_or_else(|| "not reached".into());
                return Err(io::Error::new(io::ErrorKind::NotConnected, why));
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        if compare {
            match self.compare(&tunnel).await {
                // the newer list may no longer name that computer: then it is not asked
                Ok(true) if self.cache.get().is_some_and(|r| r.device(&device.id).is_none()) => {
                    return Err(io::Error::new(io::ErrorKind::PermissionDenied, "no longer listed"));
                }
                Ok(_) => {}
                Err(e) => (self.note)(format!("{} did not say which list of devices it holds ({e})", device.name)),
            }
        }
        open_to(&tunnel, Target::Own).await
    }

    /// The home computer refuses this device: it is shown the list this device holds, outside the tunnel. Two
    /// computers can answer as one home after an old backup was brought back while the newer home computer
    /// still ran, and they have no device in common that could tell either about the other. The one shown a
    /// list from after its own time stands down, or, in the days after it was brought back, takes what the
    /// list says and answers with the list it then signs. That answer is kept here, not taken (this device is
    /// not on it), and `pull` hands it to the home computer that does answer, which then stands down.
    /// Ok(true): an answer is being carried.
    pub async fn show(&self, home: &Peer) -> Result<bool, String> {
        let Some(mine) = self.cache.get() else { return Ok(false) };
        let body = serde_json::to_vec(&mine.envelope).map_err(|e| e.to_string())?;
        let answer = self.ep.show(home, &body).await.map_err(|e| e.to_string())?;
        if answer.is_empty() {
            return Ok(false);
        }
        let envelope: Envelope = serde_json::from_slice(&answer).map_err(|e| e.to_string())?;
        let theirs = crate::roster::verify(&envelope, &self.cache.home(), None).map_err(|e| e.to_string())?;
        if theirs.stamp() <= mine.stamp() {
            return Ok(false);
        }
        (self.note)(format!(
            "a home computer that refuses this device holds a newer list of devices (number {}): the home \
             computer that answers is shown it",
            theirs.body.seq
        ));
        *self.carried.lock().unwrap_or_else(|e| e.into_inner()) = Some(envelope);
        Ok(true)
    }

    /// Asks the home computer for its list of devices, over the tunnel to it. Ok(true): a newer one was taken.
    pub async fn pull(&self, home: &Tunnel) -> Result<bool, String> {
        // What a home computer that refuses this device answered goes to the one that does not (`show`).
        let carried = self.carried.lock().unwrap_or_else(|e| e.into_inner()).clone();
        if let Some(body) = carried.as_ref().and_then(|c| serde_json::to_vec(c).ok()) {
            let told = async {
                let mut s = home.open().await?;
                tunnel::write_header(&mut s, Target::Own).await?;
                http1::ask(&mut s, "POST", "/own/v1/roster", Some(&body), LIST_MAX).await
            };
            if matches!(tokio::time::timeout(PULL_WAIT, told).await, Ok(Ok(_))) {
                *self.carried.lock().unwrap_or_else(|e| e.into_inner()) = None;
            }
        }
        // First this device says which list it holds. A home computer that was replaced (moved, or brought back
        // from a backup somewhere else) learns it from this, and stands down; one that was brought back from a
        // backup learns what changed after the backup was made.
        if let Some(r) = self.cache.get()
            && let Ok(body) = serde_json::to_vec(&r.envelope)
        {
            let told = async {
                let mut s = home.open().await?;
                tunnel::write_header(&mut s, Target::Own).await?;
                http1::ask(&mut s, "POST", "/own/v1/roster", Some(&body), LIST_MAX).await
            };
            let _ = tokio::time::timeout(PULL_WAIT, told).await;
        }
        let asked = async {
            let mut s = home.open().await?;
            tunnel::write_header(&mut s, Target::Own).await?;
            http1::ask(&mut s, "GET", "/own/v1/roster", None, LIST_MAX).await
        };
        let a = tokio::time::timeout(PULL_WAIT, asked)
            .await
            .map_err(|_| "the home computer did not answer".to_string())?
            .map_err(|e| e.to_string())?;
        if a.status != 200 {
            return Err(format!("the home computer answered {}", a.status));
        }
        let envelope: Envelope = serde_json::from_slice(&a.body).map_err(|e| e.to_string())?;
        (self.take)(&envelope).map_err(|e| e.to_string())
    }
}

impl Drop for Others {
    fn drop(&mut self) {
        self.drop_all();
    }
}
