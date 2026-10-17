//! Live node links: the `/node` WebSocket a paired machine keeps open to the hub. The server's RPCs go over it
//! (`/internal/rpc/<machine>`), and so do terminals, each on its own channel. A machine with a link needs no SSH.

use crate::link_proto::{self, FromNode, ToNode};
use crate::pty::TermIo;
use axum::extract::ws::Message;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{Notify, mpsc, oneshot};

/// Output chunks a terminal may have waiting for its viewer before the hub gives up on that viewer.
const CHAN_BACKLOG: usize = 1024;
/// Messages waiting to go out on a link (terminal input up to 1 MiB each) before the hub stops adding to it.
pub const LINK_BACKLOG: usize = 256;

#[derive(Debug, Clone, Default)]
pub struct Info {
    pub version: String,
    pub os: String,
    pub label: String,
}

struct Chan {
    out: mpsc::Sender<Vec<u8>>,
    exit: Option<oneshot::Sender<Option<i64>>>,
}

pub struct Link {
    pub name: String,
    pub since: u64,
    era: u64,
    info: Mutex<Info>,
    tx: mpsc::Sender<Message>,
    rpcs: Mutex<HashMap<u64, oneshot::Sender<Value>>>,
    chans: Mutex<HashMap<u32, Chan>>,
    next: AtomicU64,
    /// Told when the hub ends the link itself (a newer link, or the machine was unpaired).
    pub closing: Notify,
    close_with: Mutex<Option<(u16, String)>>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum RpcError {
    /// The link closed before the answer came.
    Gone,
    Timeout,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Link {
    pub fn new(name: &str, era: u64, since: u64, tx: mpsc::Sender<Message>) -> Link {
        Link {
            name: name.into(),
            since,
            era,
            info: Mutex::new(Info::default()),
            tx,
            rpcs: Mutex::new(HashMap::new()),
            chans: Mutex::new(HashMap::new()),
            next: AtomicU64::new(1),
            closing: Notify::new(),
            close_with: Mutex::new(None),
        }
    }

    /// End the link from the hub's side with this close code and reason.
    pub fn close(&self, code: u16, reason: &str) {
        *lock(&self.close_with) = Some((code, reason.into()));
        self.closing.notify_one();
    }

    pub fn close_reason(&self) -> (u16, String) {
        lock(&self.close_with).clone().unwrap_or((1000, String::new()))
    }

    pub fn info(&self) -> Info {
        lock(&self.info).clone()
    }

    fn send(&self, m: &ToNode) -> bool {
        self.tx.try_send(Message::Text(link_proto::encode(m).into())).is_ok()
    }

    /// Run `sb.py rpc` on the machine and return its JSON answer.
    pub async fn rpc(&self, req: Value, timeout: Duration) -> Result<Value, RpcError> {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        lock(&self.rpcs).insert(id, tx);
        if !self.send(&ToNode::Rpc { id, req, timeout: timeout.as_secs().max(1) }) {
            lock(&self.rpcs).remove(&id);
            return Err(RpcError::Gone);
        }
        // the node enforces the timeout on sb.py; a little more here covers the trip
        match tokio::time::timeout(timeout + Duration::from_secs(3), rx).await {
            Ok(Ok(v)) => Ok(v),
            Ok(Err(_)) => Err(RpcError::Gone),
            Err(_) => {
                lock(&self.rpcs).remove(&id);
                Err(RpcError::Timeout)
            }
        }
    }

    /// A terminal on the machine: `claude attach <bg>` in a PTY there. Output and the exit code come back in the
    /// same shape a local PTY gives, so the relay does not care which kind it has.
    pub fn open_term(self: &Arc<Self>, bg: &str, cols: u16, rows: u16) -> (NodeTerm, TermIo) {
        let ch = (self.next.fetch_add(1, Ordering::Relaxed) & 0xffff_ffff) as u32;
        let (out_tx, out_rx) = mpsc::channel(CHAN_BACKLOG);
        let (exit_tx, exit_rx) = oneshot::channel();
        lock(&self.chans).insert(ch, Chan { out: out_tx, exit: Some(exit_tx) });
        if !self.send(&ToNode::TermOpen { ch, bg: bg.into(), cols, rows }) {
            self.end_chan(ch, Some(255));
        }
        (NodeTerm { link: self.clone(), ch }, TermIo { output: out_rx, exited: exit_rx })
    }

    fn end_chan(&self, ch: u32, code: Option<i64>) -> bool {
        let Some(mut c) = lock(&self.chans).remove(&ch) else { return false };
        if let Some(e) = c.exit.take() {
            let _ = e.send(code);
        }
        true // dropping `c.out` ends the viewer's output once it has read everything before it
    }

    pub fn on_text(&self, text: &str) {
        match serde_json::from_str::<FromNode>(text) {
            Ok(FromNode::Hello { version, os, label }) => {
                let clip = |s: String, n: usize| s.chars().filter(|c| !c.is_control()).take(n).collect::<String>();
                *lock(&self.info) = Info { version: clip(version, 32), os: clip(os, 16), label: clip(label, 40) };
            }
            Ok(FromNode::RpcResult { id, res }) => {
                if let Some(tx) = lock(&self.rpcs).remove(&id) {
                    let _ = tx.send(res);
                }
            }
            Ok(FromNode::TermExit { ch, code }) => {
                self.end_chan(ch, code);
            }
            Err(_) => {} // a newer node's message
        }
    }

    pub fn on_binary(&self, b: &[u8]) {
        let Some((ch, data)) = link_proto::unframe(b) else { return };
        let full = match lock(&self.chans).get(&ch) {
            Some(c) => matches!(c.out.try_send(data.to_vec()), Err(mpsc::error::TrySendError::Full(_))),
            None => false,
        };
        if full {
            // the viewer stopped reading; never stall the whole link (and every other terminal) for one of them
            crate::server::log(format!("node {}: terminal {ch} fell behind, closing it", self.name));
            self.end_chan(ch, None);
            self.send(&ToNode::TermClose { ch });
        }
    }

    /// The link is gone: every waiting RPC fails and every terminal on it ends.
    pub fn shut(&self) {
        lock(&self.rpcs).clear();
        let chans: Vec<u32> = lock(&self.chans).keys().copied().collect();
        for ch in chans {
            self.end_chan(ch, Some(255));
        }
    }

    pub fn terminals(&self) -> usize {
        lock(&self.chans).len()
    }
}

/// One terminal carried on a link. Dropping it hangs up there (Claude Code detaches; the session keeps running).
pub struct NodeTerm {
    link: Arc<Link>,
    ch: u32,
}

impl NodeTerm {
    /// Writes, waiting while the link is full (a big paste) instead of ending the terminal: each piece goes out
    /// once the link has room. False if the terminal ended, or the link took nothing for `INPUT_PATIENCE` (then
    /// the terminal is ended so it does not hold the link up).
    pub async fn write_all(&self, bytes: &[u8]) -> bool {
        for piece in bytes.chunks(crate::pty::INPUT_PIECE) {
            if !lock(&self.link.chans).contains_key(&self.ch) {
                return false;
            }
            let m = Message::Binary(link_proto::frame(self.ch, piece).into());
            match tokio::time::timeout(crate::pty::INPUT_PATIENCE, self.link.tx.send(m)).await {
                Ok(Ok(())) => {}
                Ok(Err(_)) => return false,
                Err(_) => {
                    crate::server::log(format!("node {}: terminal {} input stuck, closing it", self.link.name, self.ch));
                    self.link.end_chan(self.ch, None);
                    return false;
                }
            }
        }
        true
    }

    pub fn write(&self, bytes: Vec<u8>) {
        if let Err(mpsc::error::TrySendError::Full(_)) = self.link.tx.try_send(Message::Binary(link_proto::frame(self.ch, &bytes).into())) {
            // typed faster than the machine takes it: end this terminal rather than grow the link's queue without limit
            crate::server::log(format!("node {}: terminal {} input fell behind, closing it", self.link.name, self.ch));
            self.link.end_chan(self.ch, None);
        }
    }

    pub fn resize(&self, cols: u16, rows: u16) {
        self.link.send(&ToNode::TermResize { ch: self.ch, cols, rows });
    }
}

impl Drop for NodeTerm {
    fn drop(&mut self) {
        if self.link.end_chan(self.ch, None) {
            self.link.send(&ToNode::TermClose { ch: self.ch });
        }
    }
}

/// Every live link, one per machine.
#[derive(Default)]
pub struct Links {
    map: Mutex<HashMap<String, Arc<Link>>>,
    era: AtomicU64,
}

impl Links {
    pub fn next_era(&self) -> u64 {
        self.era.fetch_add(1, Ordering::Relaxed) + 1
    }

    /// Make `link` the machine's link; the one it replaces (if any) is told to close.
    pub fn insert(&self, link: Arc<Link>) {
        if let Some(old) = lock(&self.map).insert(link.name.clone(), link) {
            old.close(link_proto::CLOSE_REPLACED, "A newer link for this machine took over.");
        }
    }

    /// Take `link` out, unless a newer one already took its place.
    pub fn remove(&self, link: &Link) {
        let mut m = lock(&self.map);
        if m.get(&link.name).is_some_and(|l| l.era == link.era) {
            m.remove(&link.name);
        }
    }

    pub fn get(&self, name: &str) -> Option<Arc<Link>> {
        lock(&self.map).get(name).cloned()
    }

    pub fn len(&self) -> usize {
        lock(&self.map).len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_the_machine_does_not_take_ends_that_terminal() {
        // security M7: a viewer typing faster than the node reads grew the link's queue without limit
        let (tx, _rx) = mpsc::channel(2);
        let link = Arc::new(Link::new("lab", 1, 0, tx));
        let (term, _io) = link.open_term("c03ef0d9", 80, 24); // term.open takes one place
        term.write(b"a".to_vec());
        assert_eq!(link.terminals(), 1);
        term.write(b"b".to_vec());
        assert_eq!(link.terminals(), 0, "full: the terminal ended, the link did not grow");
        assert!(!link.send(&ToNode::TermClose { ch: 1 }), "nothing is queued past the limit");
    }

    #[tokio::test]
    async fn a_big_paste_waits_for_room_instead_of_ending_the_terminal() {
        let (tx, mut rx) = mpsc::channel(4);
        let link = Arc::new(Link::new("lab", 1, 0, tx));
        let (term, _io) = link.open_term("c03ef0d9", 80, 24);
        let reader = tokio::spawn(async move {
            let mut bytes = 0usize;
            while let Some(m) = rx.recv().await {
                tokio::time::sleep(Duration::from_millis(1)).await; // a slow link
                if let Message::Binary(b) = m {
                    bytes += link_proto::unframe(&b).map(|(_, d)| d.len()).unwrap_or(0);
                }
                if bytes >= 200_000 {
                    break;
                }
            }
            bytes
        });
        assert!(term.write_all(&vec![b'x'; 200_000]).await);
        assert_eq!(link.terminals(), 1, "still open");
        assert_eq!(reader.await.unwrap(), 200_000);
    }
}
