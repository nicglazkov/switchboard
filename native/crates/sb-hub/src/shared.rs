//! One terminal per chat, shared by everyone looking at it (spec 2026-10-14, terminal pipeline).
//!
//! The hub runs one PTY per chat (machine + background id), over ssh or a node link, and fans its output out to
//! every viewer: a Mac tab, the iPhone, the web page. A viewer leaving leaves the PTY to the others; the last one
//! leaving hangs up, and `claude attach` detaches as before.
//!
//! The hub also keeps the screen: everything the program writes goes through a headless alacritty grid, so a
//! viewer that joins, comes back or asks for a redraw gets a snapshot of it first and needs no resize nudge.
//!
//! Size: the viewer that last had focus or typed owns it. A viewer asks (`resize`); only the owner's ask changes
//! the PTY. The hub resizes the PTY and its grid together and sends `size` to every viewer in the same ordered
//! stream as the output, so each client parses bytes before it at the old size and bytes after at the new one.
//!
//! A viewer that cannot keep up is not waited for (the others would stall): its output is skipped and, once it has
//! read what was queued, it gets a snapshot instead.

use crate::link::NodeTerm;
use crate::pty::Term;
use alacritty_terminal::event::VoidListener;
use sb_core::term::screen::{self, Screen};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::Notify;

/// Output queued for one viewer before it counts as behind. Counted in bytes: a program that writes in many tiny
/// pieces (ConPTY on Windows) must not put a viewer behind that is only a few kilobytes late.
pub const VIEWER_BACKLOG_BYTES: usize = 1024 * 1024;
/// Messages queued for one viewer before it counts as behind, whatever their size.
pub const VIEWER_BACKLOG: usize = 256;
/// Output waiting for a viewer is joined into messages of up to this much.
const MERGE_MAX: usize = 64 * 1024;

/// Where a terminal runs: a PTY on this computer (running ssh to the machine, or the chat's own program when
/// this computer is the machine), or a PTY on the machine through its node link.
pub enum Backend {
    Pty(Term),
    Node(NodeTerm),
}

impl Backend {
    /// Writes input, waiting while the program is slow to read it (a big paste) instead of ending the terminal.
    /// False if the terminal ended or would not take it for a long time.
    async fn write_all(&self, b: &[u8]) -> bool {
        match self {
            Backend::Pty(t) => t.write_all(b).await,
            Backend::Node(t) => t.write_all(b).await,
        }
    }

    fn resize(&self, cols: u16, rows: u16) {
        match self {
            Backend::Pty(t) => t.resize(cols, rows),
            Backend::Node(t) => t.resize(cols, rows),
        }
    }
}

/// What goes to one viewer, in order.
#[derive(Debug, Clone)]
pub enum Out {
    /// The program has started writing (`status: open`).
    Open,
    Bytes(Vec<u8>),
    Size(u16, u16),
    Owner(bool),
    /// The screen as bytes, between `snapshot_begin` and `snapshot_end`.
    Snapshot(Vec<u8>),
}

/// How a viewer talks: `proto=2` clients know size frames and snapshots; older ones only reflow on their own.
#[derive(Debug, Clone, Copy)]
pub struct Hello {
    pub cols: u16,
    pub rows: u16,
    /// Knows `size`, `snapshot_begin`; a snapshot needs no reset in front of it.
    pub modern: bool,
    /// Its first snapshot carries the scrollback (a client that already has its own asks for none).
    pub history: bool,
}

/// What is waiting to go to one viewer. Output that arrives while output is still waiting is joined to it, so a
/// burst of small writes leaves as a few messages.
#[derive(Default)]
struct Waiting {
    items: VecDeque<Out>,
    /// Output bytes in `items` (snapshots and the rest do not count).
    bytes: usize,
    closed: bool,
}

#[derive(Default)]
pub struct Queue {
    q: Mutex<Waiting>,
    more: Notify,
}

impl Queue {
    /// Adds `o`. False, with nothing added, when the queue is at its bound.
    fn push(&self, o: Out) -> bool {
        let mut q = lock(&self.q);
        if q.closed {
            return true;
        }
        if let Out::Bytes(b) = &o {
            if q.bytes + b.len() > VIEWER_BACKLOG_BYTES {
                return false;
            }
            if let Some(Out::Bytes(last)) = q.items.back_mut()
                && last.len() + b.len() <= MERGE_MAX
            {
                last.extend_from_slice(b);
                q.bytes += b.len();
                return true; // the viewer was already told there is something to read
            }
        }
        if q.items.len() >= VIEWER_BACKLOG {
            return false;
        }
        if let Out::Bytes(b) = &o {
            q.bytes += b.len();
        }
        q.items.push_back(o);
        drop(q);
        self.more.notify_one();
        true
    }

    fn close(&self) {
        lock(&self.q).closed = true;
        self.more.notify_one();
    }

    fn pop(&self) -> Result<Out, bool> {
        let mut q = lock(&self.q);
        match q.items.pop_front() {
            Some(o) => {
                if let Out::Bytes(b) = &o {
                    q.bytes -= b.len();
                }
                Ok(o)
            }
            None => Err(q.closed),
        }
    }

    /// The next message, waiting for one. None once the terminal ended and everything before that was read.
    /// Only one task reads a queue (the viewer's relay).
    pub async fn recv(&self) -> Option<Out> {
        loop {
            match self.pop() {
                Ok(o) => return Some(o),
                Err(true) => return None,
                // a push between the look above and this wait leaves its wake-up behind, so none is missed
                Err(false) => self.more.notified().await,
            }
        }
    }

    /// The next message if one is waiting.
    pub fn try_recv(&self) -> Option<Out> {
        self.pop().ok()
    }

    pub fn is_empty(&self) -> bool {
        lock(&self.q).items.is_empty()
    }
}

struct Viewer {
    id: u64,
    queue: Arc<Queue>,
    want: (u16, u16),
    /// When it last had focus or typed (a counter), to pick the next owner.
    active: u64,
    behind: Arc<AtomicBool>,
    hello: Hello,
    /// It has been sent output or a screen: an old client needs a reset before the next screen.
    has_screen: bool,
    sent_history: bool,
}

impl Viewer {
    /// Queues `o`, unless this viewer is behind; a full queue makes it behind (it gets a snapshot later).
    fn push(&mut self, o: Out) {
        if self.behind.load(Ordering::Relaxed) {
            return;
        }
        if matches!(o, Out::Bytes(_) | Out::Snapshot(_)) {
            self.has_screen = true;
        }
        if !self.queue.push(o) {
            self.behind.store(true, Ordering::Relaxed);
        }
    }
}

struct State {
    screen: Screen<VoidListener>,
    size: (u16, u16),
    viewers: Vec<Viewer>,
    owner: Option<u64>,
    opened: bool,
    /// The PTY ended, with this exit code.
    exited: Option<Option<i64>>,
    tick: u64,
    out_bytes: u64,
}

pub struct Shared {
    pub machine: String,
    pub bg: String,
    pub label: String,
    pub via: &'static str,
    backend: Mutex<Option<Arc<Backend>>>,
    st: Mutex<State>,
    next: AtomicU64,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// A viewer's end of a shared terminal.
pub struct Seat {
    pub id: u64,
    pub rx: Arc<Queue>,
    pub behind: Arc<AtomicBool>,
}

impl Shared {
    pub fn new(
        machine: &str,
        bg: &str,
        label: String,
        via: &'static str,
        backend: Backend,
        cols: u16,
        rows: u16,
    ) -> Arc<Shared> {
        Arc::new(Shared {
            machine: machine.into(),
            bg: bg.into(),
            label,
            via,
            backend: Mutex::new(Some(Arc::new(backend))),
            st: Mutex::new(State {
                screen: Screen::new(cols, rows, screen::HISTORY, VoidListener),
                size: (cols, rows),
                viewers: Vec::new(),
                owner: None,
                opened: false,
                exited: None,
                tick: 0,
                out_bytes: 0,
            }),
            next: AtomicU64::new(1),
        })
    }

    /// Still running (not exited, not hung up).
    pub fn alive(&self) -> bool {
        lock(&self.backend).is_some() && lock(&self.st).exited.is_none()
    }

    pub fn viewers(&self) -> usize {
        lock(&self.st).viewers.len()
    }

    pub fn size(&self) -> (u16, u16) {
        lock(&self.st).size
    }

    pub fn out_bytes(&self) -> u64 {
        lock(&self.st).out_bytes
    }

    fn backend(&self) -> Option<Arc<Backend>> {
        lock(&self.backend).clone()
    }

    /// A new viewer. The first one owns the size; a later one sees the terminal at its current size, then the
    /// screen, and takes the size over once it has focus or types.
    pub fn join(&self, hello: Hello) -> Seat {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let rx = Arc::new(Queue::default());
        let behind = Arc::new(AtomicBool::new(false));
        let mut st = lock(&self.st);
        st.tick += 1;
        let active = st.tick;
        st.viewers.push(Viewer {
            id,
            queue: rx.clone(),
            want: (hello.cols, hello.rows),
            active,
            behind: behind.clone(),
            hello,
            has_screen: false,
            sent_history: false,
        });
        if st.owner.is_none() {
            st.owner = Some(id);
        }
        let (size, owner, opened) = (st.size, st.owner == Some(id), st.opened);
        let snap = opened.then(|| snapshot_for(&mut st, id));
        let v = st.viewers.last_mut().expect("just pushed");
        v.push(Out::Size(size.0, size.1));
        v.push(Out::Owner(owner));
        if let Some(s) = snap {
            v.push(Out::Open);
            v.push(Out::Snapshot(s));
        }
        Seat { id, rx, behind }
    }

    /// Output from the program: into the hub's grid, then to every viewer.
    pub fn output(&self, b: Vec<u8>) {
        let mut st = lock(&self.st);
        st.screen.feed(&b);
        st.out_bytes += b.len() as u64;
        let first = !st.opened;
        st.opened = true;
        for v in st.viewers.iter_mut() {
            if first {
                v.push(Out::Open);
            }
            v.push(Out::Bytes(b.clone()));
        }
    }

    /// When a synchronized update the program never ended times out.
    pub fn sync_deadline(&self) -> Option<std::time::Instant> {
        lock(&self.st).screen.sync_deadline()
    }

    pub fn flush_sync(&self) {
        lock(&self.st).screen.flush();
    }

    /// The size this viewer would like. It takes effect if the viewer owns the size.
    pub fn want(&self, id: u64, cols: u16, rows: u16) {
        let mut st = lock(&self.st);
        let Some(v) = st.viewers.iter_mut().find(|v| v.id == id) else { return };
        v.want = (cols, rows);
        if st.owner == Some(id) {
            self.apply(&mut st, cols, rows);
        }
    }

    /// This viewer has focus or typed: it owns the size now.
    pub fn focus(&self, id: u64) {
        let mut st = lock(&self.st);
        st.tick += 1;
        let tick = st.tick;
        let Some(v) = st.viewers.iter_mut().find(|v| v.id == id) else { return };
        v.active = tick;
        let want = v.want;
        if st.owner != Some(id) {
            st.owner = Some(id);
            for v in st.viewers.iter_mut() {
                let mine = v.id == id;
                v.push(Out::Owner(mine));
            }
            self.apply(&mut st, want.0, want.1);
        }
    }

    /// Resizes the PTY and the grid together, and tells every viewer where in the stream it happened.
    fn apply(&self, st: &mut State, cols: u16, rows: u16) {
        if (cols, rows) == st.size {
            return;
        }
        if let Some(b) = self.backend() {
            b.resize(cols, rows);
        }
        st.screen.resize(cols, rows);
        st.size = (cols, rows);
        for v in st.viewers.iter_mut() {
            v.push(Out::Size(cols, rows));
        }
    }

    /// Input from a viewer: it owns the size from now on, then the bytes go to the program, waiting while it is
    /// slow to read them. False if the terminal is gone.
    pub async fn input(&self, id: u64, bytes: &[u8]) -> bool {
        let owner = lock(&self.st).owner;
        if owner != Some(id) {
            self.focus(id);
        }
        match self.backend() {
            Some(b) => b.write_all(bytes).await,
            None => false,
        }
    }

    /// The screen again for this viewer (`redraw`), queued behind what it has not read yet.
    pub fn redraw(&self, id: u64) {
        let mut st = lock(&self.st);
        if !st.opened {
            return;
        }
        let size = st.size;
        let snap = snapshot_for(&mut st, id);
        if let Some(v) = st.viewers.iter_mut().find(|v| v.id == id) {
            v.push(Out::Size(size.0, size.1));
            v.push(Out::Snapshot(snap));
        }
    }

    /// A viewer that fell behind has read everything queued: what to send it now (the size and the screen), after
    /// which it gets output again. Taken under the same lock as the output, so nothing is lost or doubled.
    pub fn catch_up(&self, id: u64) -> Vec<Out> {
        let mut st = lock(&self.st);
        let (size, owner, opened) = (st.size, st.owner == Some(id), st.opened);
        let snap = opened.then(|| snapshot_for(&mut st, id));
        let Some(v) = st.viewers.iter_mut().find(|v| v.id == id) else { return vec![] };
        v.behind.store(false, Ordering::Relaxed);
        v.has_screen = true;
        let mut out = vec![Out::Size(size.0, size.1), Out::Owner(owner)];
        out.extend(snap.map(Out::Snapshot));
        out
    }

    /// A viewer left. True when it was the last one (the caller then hangs up with `close`). The size goes to
    /// whoever was active most recently.
    pub fn leave(&self, id: u64) -> bool {
        let mut st = lock(&self.st);
        st.viewers.retain(|v| v.id != id);
        if st.viewers.is_empty() {
            st.owner = None;
            return true;
        }
        if st.owner == Some(id) {
            let next = st.viewers.iter().max_by_key(|v| v.active).map(|v| (v.id, v.want));
            if let Some((nid, want)) = next {
                st.owner = Some(nid);
                for v in st.viewers.iter_mut() {
                    let mine = v.id == nid;
                    v.push(Out::Owner(mine));
                }
                self.apply(&mut st, want.0, want.1);
            }
        }
        false
    }

    /// Hang up: the PTY closes and `claude attach` detaches. The session keeps running.
    pub fn close(&self) {
        lock(&self.backend).take();
    }

    /// The program ended: every viewer's queue closes once it has read the rest, then it reads the exit code.
    pub fn ended(&self, code: Option<i64>) {
        let mut st = lock(&self.st);
        st.exited = Some(code);
        for v in st.viewers.iter() {
            v.queue.close();
        }
        drop(st);
        lock(&self.backend).take();
    }

    pub fn exit_code(&self) -> Option<Option<i64>> {
        lock(&self.st).exited
    }
}

/// The screen for one viewer: with scrollback the first time if it asked, and a reset in front for an old client
/// that already has a screen (it does not know a snapshot is coming).
fn snapshot_for(st: &mut State, id: u64) -> Vec<u8> {
    let Some(i) = st.viewers.iter().position(|v| v.id == id) else { return vec![] };
    let (modern, history, sent, has) = {
        let v = &st.viewers[i];
        (v.hello.modern, v.hello.history, v.sent_history, v.has_screen)
    };
    let lines = if history && !sent { screen::HISTORY } else { 0 };
    let mut out = if !modern && has { b"\x1bc".to_vec() } else { Vec::new() };
    out.extend(st.screen.snapshot(lines));
    st.viewers[i].sent_history = true;
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::link::Link;
    use crate::link_proto::ToNode;
    use axum::extract::ws::Message;
    use tokio::sync::mpsc;

    /// A shared terminal on a fake node link, so resizes show up as messages to read.
    fn lab() -> (Arc<Shared>, mpsc::Receiver<Message>, crate::pty::TermIo) {
        let (tx, rx) = mpsc::channel(64);
        let link = Arc::new(Link::new("lab", 1, 0, tx));
        let (t, io) = link.open_term("c03ef0d9", 80, 24);
        (Shared::new("lab", "c03ef0d9", "Lab".into(), "node", Backend::Node(t), 80, 24), rx, io)
    }

    fn hello(cols: u16, rows: u16) -> Hello {
        Hello { cols, rows, modern: true, history: true }
    }

    fn drain(s: &mut Seat) -> Vec<Out> {
        let mut v = vec![];
        while let Some(o) = s.rx.try_recv() {
            v.push(o);
        }
        v
    }

    fn resizes(rx: &mut mpsc::Receiver<Message>) -> Vec<(u16, u16)> {
        let mut v = vec![];
        while let Ok(m) = rx.try_recv() {
            if let Message::Text(t) = m
                && let Ok(ToNode::TermResize { cols, rows, .. }) = serde_json::from_str::<ToNode>(t.as_str())
            {
                v.push((cols, rows));
            }
        }
        v
    }

    fn kinds(outs: &[Out]) -> String {
        outs.iter()
            .map(|o| match o {
                Out::Open => "open".to_string(),
                Out::Bytes(b) => format!("bytes:{}", String::from_utf8_lossy(b)),
                Out::Size(c, r) => format!("size:{c}x{r}"),
                Out::Owner(o) => format!("owner:{o}"),
                Out::Snapshot(_) => "snapshot".to_string(),
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    #[test]
    fn output_fans_out_to_every_viewer() {
        let (sh, _rx, _io) = lab();
        let mut a = sh.join(hello(80, 24));
        sh.output(b"hello".to_vec());
        let mut b = sh.join(hello(100, 30));
        sh.output(b" world".to_vec());
        assert_eq!(kinds(&drain(&mut a)), "size:80x24 owner:true open bytes:hello world");
        // the late one sees the size, that it does not own it, then the screen, then live output
        assert_eq!(kinds(&drain(&mut b)), "size:80x24 owner:false open snapshot bytes: world");
    }

    #[test]
    fn the_last_viewer_leaving_hangs_up() {
        let (sh, mut rx, _io) = lab();
        let a = sh.join(hello(80, 24));
        let b = sh.join(hello(80, 24));
        assert!(!sh.leave(a.id), "one is still looking");
        assert!(sh.alive());
        assert!(sh.leave(b.id), "the last one");
        sh.close();
        assert!(!sh.alive());
        let closed = std::iter::from_fn(|| rx.try_recv().ok())
            .any(|m| matches!(m, Message::Text(t) if t.contains("term.close")));
        assert!(closed, "the node was told to hang up");
    }

    #[test]
    fn size_follows_focus_and_frames_are_in_order() {
        let (sh, mut rx, _io) = lab();
        let mut a = sh.join(hello(80, 24));
        let mut b = sh.join(hello(120, 40));
        sh.output(b"one".to_vec());
        // b is not the owner: its size is remembered, nothing changes
        sh.want(b.id, 130, 40);
        assert_eq!(resizes(&mut rx), vec![]);
        sh.focus(b.id);
        assert_eq!(resizes(&mut rx), vec![(130, 40)]);
        sh.output(b"two".to_vec());
        assert_eq!(kinds(&drain(&mut a)), "size:80x24 owner:true open bytes:one owner:false size:130x40 bytes:two");
        assert_eq!(kinds(&drain(&mut b)), "size:80x24 owner:false open bytes:one owner:true size:130x40 bytes:two");
        // typing takes it back to a's size
        sh.want(a.id, 90, 30);
        assert_eq!(resizes(&mut rx), vec![]);
        sh.focus(a.id);
        assert_eq!(resizes(&mut rx), vec![(90, 30)]);
        assert_eq!(sh.size(), (90, 30));
        // the owner leaving hands the size to the other
        sh.leave(a.id);
        assert_eq!(resizes(&mut rx), vec![(130, 40)]);
        let _ = drain(&mut b);
    }

    #[test]
    fn a_viewer_that_falls_behind_gets_the_screen_instead() {
        let (sh, _rx, _io) = lab();
        let mut a = sh.join(hello(80, 24));
        let line = |i: usize| format!("line {i} {}\r\n", "x".repeat(60)).into_bytes();
        let lines = VIEWER_BACKLOG_BYTES / line(0).len() + 50;
        for i in 0..lines {
            sh.output(line(i));
        }
        assert!(a.behind.load(Ordering::Relaxed));
        let queued = drain(&mut a);
        let bytes: usize = queued.iter().map(|o| if let Out::Bytes(b) = o { b.len() } else { 0 }).sum();
        assert!(bytes <= VIEWER_BACKLOG_BYTES, "nothing past the bound: {bytes}");
        assert!(bytes > VIEWER_BACKLOG_BYTES - 1024, "and everything up to it: {bytes}");
        let again = sh.catch_up(a.id);
        assert_eq!(kinds(&again), "size:80x24 owner:true snapshot");
        let Out::Snapshot(snap) = &again[2] else { panic!() };
        // the snapshot shows the last line written, which the queue never had
        let mut copy: Screen<VoidListener> = Screen::new(80, 24, 100, VoidListener);
        copy.feed(snap);
        let text = screen::dump(&copy.term, 0).join("\n");
        assert!(text.contains(&format!("line {} ", lines - 1)), "{text}");
        sh.output(b"live".to_vec());
        assert_eq!(kinds(&drain(&mut a)), "bytes:live");
    }

    #[test]
    fn many_small_writes_wait_as_a_few_messages_and_leave_nobody_behind() {
        // ConPTY on Windows: hundreds of tiny writes a second. A thousand of them is 10 KB, not a viewer behind.
        let (sh, _rx, _io) = lab();
        let mut a = sh.join(hello(80, 24));
        let mut want = Vec::new();
        for i in 0..1000 {
            let piece = format!("{:>9}\n", i).into_bytes();
            want.extend_from_slice(&piece);
            sh.output(piece);
        }
        assert!(!a.behind.load(Ordering::Relaxed));
        let queued = drain(&mut a);
        assert_eq!(queued.len(), 4, "size, owner, open and one piece of output: {}", queued.len());
        let Some(Out::Bytes(got)) = queued.last() else { panic!() };
        assert_eq!(got, &want, "in order, nothing lost");
    }

    #[test]
    fn joined_output_stays_in_order_around_other_frames_and_within_a_message_size() {
        let (sh, _rx, _io) = lab();
        let mut a = sh.join(hello(80, 24));
        let mut b = sh.join(hello(100, 30));
        sh.output(b"one".to_vec());
        sh.output(b"two".to_vec());
        sh.focus(b.id); // a size frame goes between: output after it is not joined to output before it
        sh.output(b"three".to_vec());
        assert_eq!(
            kinds(&drain(&mut a)),
            "size:80x24 owner:true open bytes:onetwo owner:false size:100x30 bytes:three"
        );
        let _ = drain(&mut b);
        // 200 KB waiting is four messages or so, none larger than a join allows
        for _ in 0..200 {
            sh.output(vec![b'x'; 1000]);
        }
        let sizes = |outs: Vec<Out>| -> Vec<usize> {
            outs.iter().map(|o| if let Out::Bytes(b) = o { b.len() } else { 0 }).collect()
        };
        let got = sizes(drain(&mut a));
        assert!(got.len() <= 4, "{got:?}");
        assert_eq!(got.iter().sum::<usize>(), 200_000);
        assert!(got.iter().all(|n| *n <= MERGE_MAX), "{got:?}");
        // each viewer has its own copy: joining for one never changes what another reads
        assert_eq!(sizes(drain(&mut b)).iter().sum::<usize>(), 200_000);
    }

    #[test]
    fn a_viewer_far_behind_in_messages_gets_the_screen_too() {
        // frames that cannot be joined (a size change between every write) still have a bound
        let (sh, _rx, _io) = lab();
        let mut a = sh.join(hello(80, 24));
        for i in 0..VIEWER_BACKLOG as u16 {
            sh.want(a.id, 81 + i % 2, 24);
            sh.output(b"x".to_vec());
        }
        assert!(a.behind.load(Ordering::Relaxed));
        assert_eq!(drain(&mut a).len(), VIEWER_BACKLOG, "nothing past the bound");
    }

    #[tokio::test]
    async fn a_waiting_viewer_wakes_for_output_and_for_the_end() {
        let (sh, _rx, _io) = lab();
        let a = sh.join(hello(80, 24));
        while a.rx.try_recv().is_some() {}
        let reader = tokio::spawn({
            let rx = a.rx.clone();
            async move {
                let mut got = Vec::new();
                while let Some(o) = rx.recv().await {
                    got.push(o);
                }
                got
            }
        });
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        sh.output(b"late".to_vec());
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        sh.ended(Some(0));
        let got = tokio::time::timeout(std::time::Duration::from_secs(5), reader).await.unwrap().unwrap();
        assert_eq!(kinds(&got), "open bytes:late");
    }

    #[test]
    fn snapshots_round_trip_through_a_fresh_grid() {
        let (sh, _rx, _io) = lab();
        let mut a = sh.join(hello(80, 24));
        sh.output(b"\x1b[1;32mgreen\x1b[0m and plain\r\n\x1b[?2004h\x1b[5;10Hthere".to_vec());
        sh.redraw(a.id);
        let outs = drain(&mut a);
        let Some(Out::Snapshot(snap)) = outs.last() else { panic!("{}", kinds(&outs)) };
        let mut copy: Screen<VoidListener> = Screen::new(80, 24, 100, VoidListener);
        copy.feed(snap);
        let st = lock(&sh.st);
        assert_eq!(screen::dump(&copy.term, 0), screen::dump(&st.screen.term, 0));
        assert_eq!(screen::state(&copy.term), screen::state(&st.screen.term));
    }

    #[test]
    fn old_clients_get_a_reset_before_a_later_screen() {
        let (sh, _rx, _io) = lab();
        let mut old = sh.join(Hello { cols: 80, rows: 24, modern: false, history: true });
        sh.output(b"x".to_vec());
        sh.redraw(old.id);
        let outs = drain(&mut old);
        let Some(Out::Snapshot(snap)) = outs.last() else { panic!() };
        assert!(snap.starts_with(b"\x1bc"));
        let mut new = sh.join(hello(80, 24));
        let outs = drain(&mut new);
        let Some(Out::Snapshot(snap)) = outs.last() else { panic!() };
        assert!(!snap.starts_with(b"\x1bc"));
    }

    #[test]
    fn the_end_closes_every_queue_after_the_rest() {
        let (sh, _rx, _io) = lab();
        let mut a = sh.join(hello(80, 24));
        sh.output(b"last words".to_vec());
        sh.ended(Some(0));
        assert_eq!(kinds(&drain(&mut a)), "size:80x24 owner:true open bytes:last words");
        assert!(a.rx.try_recv().is_none());
        assert_eq!(sh.exit_code(), Some(Some(0)));
        assert!(!sh.alive());
    }
}
