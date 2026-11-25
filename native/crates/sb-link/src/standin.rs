//! The stand-in: what a device's two loopback ports answer while the home computer is away.
//!
//! A paired device reaches the home computer through two loopback ports, one for the server's API and one for
//! the hub (`tunnel::Forwarder`). While home answers, the stand-in is that and nothing more: every connection
//! is one stream to home, byte for byte. When home has not been reached for `Options::away_after`, the device
//! is "away from home", and the same two ports answer from what the other computers say about their own chats
//! (each computer's own surface, `/own/v1/...` on a stream that starts `{"to":"own"}`):
//!
//!   API port   GET  /api/summary    the counts, from the computers that answered, with `away`
//!              GET  /api/sessions   one row for each computer on the roster, with `away`
//!              POST /api/refresh    the same, asked again now
//!              POST /api/term       {"sid"} or {"bg"} (and "machine"): a ticket from the computer the chat is
//!                                   on; `ws` leads to this device's hub port with `own=<machine>`
//!              POST /api/action     {"verb": stop | dismiss | pass | resume, "sid"} (and "machine")
//!              POST /api/event      a hook's word about a chat here: taken and answered at once
//!              anything else        503 {"ok":false,"kind":"home_away","error":"The home computer is away. ..."}
//!   hub port   GET  /term?t=..&own=<machine>   carried to that computer's `/own/v1/term`
//!              anything else        503, as above
//!
//! Away ends the moment home is reached again: every connection the stand-in was answering is dropped, and the
//! programs behind them connect again and get the home computer.
//!
//! Nothing here knows how a stream is opened: `Backend` is the device's side of that (a computer's helper, a
//! phone's library), and it is all a caller has to write. The ports are loopback sockets the caller binds; the
//! stand-in also takes any stream (`carry`), so a device with no sockets can use it.
//!
//! Who is asked: the computers on the roster this device holds. A roster past its seven days names nobody to
//! ask, since they would all refuse; `away.line` then says so.

use crate::http1::{self, Head};
use crate::paired::Kind;
use crate::roster::{Roster, RosterDevice};
use crate::tunnel::{self, PipeOptions, Target};
use serde_json::{Value, json};
use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::atomic::{AtomicU16, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::{Semaphore, watch};

/// How long home may be out of reach before a computer's ports answer without it.
pub const AWAY_AFTER: Duration = Duration::from_secs(20);
/// The same for a phone, where somebody is looking at the screen.
pub const AWAY_AFTER_PHONE: Duration = Duration::from_secs(5);
/// What a chat can be asked to do while the home computer is away.
pub const VERBS: [&str; 4] = ["stop", "dismiss", "pass", "resume"];
/// What is not there while the home computer is away, as the apps name it.
pub const LACKS: [&str; 8] = ["new_chat", "move", "history", "files", "usage", "library", "settings", "notifications"];
pub const LACKS_LINE: &str = "Not available while the home computer is away: new chats, moving a chat, history, \
                              files, usage, library, settings, notifications.";
pub const HOME_AWAY: &str = "The home computer is away. This needs the home computer.";
const STALE_LINE: &str = "What this device knows about your computers is 7 days old, so they no longer answer it. \
                          Start the home computer, or replace it.";
const ASKING_LINE: &str = "Home is away. Asking the other computers for their chats.";
const BODY_MAX: usize = 256 * 1024;
const BOARD_MAX: usize = 8 << 20;

/// A stream to another device.
pub trait Stream: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Stream for T {}
pub type Io = Box<dyn Stream>;
pub type Fut<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// The device's side: how home and the other computers are reached. Everything the stand-in needs from the
/// program it runs in.
pub trait Backend: Send + Sync + 'static {
    /// Whether the home computer is reached just now.
    fn home_up(&self) -> bool;
    /// A stream to the home computer's `target`, its first line already written. It should fail at once when
    /// home is known to be out of reach.
    fn open_home(&self, target: Target) -> Fut<'_, io::Result<Io>>;
    /// The roster this device holds.
    fn roster(&self) -> Option<Arc<Roster>>;
    /// A stream to the own surface of a computer on the roster, its first line (`{"to":"own"}`) already
    /// written. A computer asked for itself answers in the same way, with no network in between.
    /// `PermissionDenied` when the computer refused this device; any other error when it was not reached.
    fn open_own(&self, device: &RosterDevice) -> Fut<'_, io::Result<Io>>;
    /// The body of a `POST /api/event` made on this device while home is away (a hook's ping).
    fn event(&self, _body: &Value) {}
    /// Away began (true) or ended (false).
    fn away_changed(&self, _away: bool) {}
    /// Seconds since 1970.
    fn now(&self) -> u64 {
        SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
    }
}

#[derive(Debug, Clone)]
pub struct Options {
    pub away_after: Duration,
    /// How often `run` looks whether home is reached.
    pub poll: Duration,
    /// How long one computer may take to show its chats.
    pub board_wait: Duration,
    /// How long one round of asking every computer stands.
    pub board_for: Duration,
    /// The clocks of the connections carried to home and to a terminal.
    pub pipe: PipeOptions,
    /// Connections carried at once, both ports together; more wait to be accepted.
    pub max_streams: usize,
    /// While away, every computer is asked this often even when no program is looking, so `away` has
    /// something to say. None: only when a program asks.
    pub rounds_every: Option<Duration>,
    /// For this long after home went away, a round in which nobody answered is not believed yet: the
    /// connections to the other computers are only being made then, and a first one can take longer than a
    /// computer is waited for. The line says they are being asked, and they are asked again every few seconds.
    pub settle: Duration,
}

impl Default for Options {
    fn default() -> Options {
        Options {
            away_after: AWAY_AFTER,
            poll: Duration::from_millis(250),
            board_wait: Duration::from_secs(8),
            board_for: Duration::from_secs(2),
            pipe: PipeOptions::default(),
            max_streams: tunnel::MAX_STREAMS,
            rounds_every: Some(Duration::from_secs(30)),
            settle: Duration::from_secs(20),
        }
    }
}

/// Why a computer on the roster shows no chats. `NeedsUpdate` is said only of a computer the roster marks as
/// one whose helper did not know what a roster is (`RosterDevice::own` is false), and that did not answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Why {
    Unreachable,
    NeedsUpdate,
    Refused,
    IsHome,
}

impl Why {
    pub fn name(self) -> &'static str {
        match self {
            Why::Unreachable => "unreachable",
            Why::NeedsUpdate => "needs_update",
            Why::Refused => "refused",
            Why::IsHome => "is_home",
        }
    }

    fn sentence(self, label: &str) -> String {
        match self {
            Why::Unreachable => format!("{label} cannot be reached just now."),
            Why::NeedsUpdate => {
                format!("{label} needs an update before it can answer while the home computer is away.")
            }
            Why::Refused => format!("{label} does not answer this device."),
            Why::IsHome => "The home computer is away.".to_string(),
        }
    }
}

struct Row {
    device: RosterDevice,
    machine: String,
    /// Its row of the board, or why there is none.
    board: Result<Value, Why>,
}

/// One round of asking every computer on the roster.
struct Merged {
    rows: Vec<Row>,
    roster: Option<Arc<Roster>>,
    stale: bool,
    v: u64,
}

pub struct StandIn {
    backend: Arc<dyn Backend>,
    opts: Options,
    /// Since when this device is away from home, in seconds since 1970. None: it is not.
    away: watch::Sender<Option<u64>>,
    last_seen: AtomicU64,
    hub_port: AtomicU16,
    merged: tokio::sync::Mutex<Option<(Instant, Arc<Merged>)>>,
    last: Mutex<Option<Arc<Merged>>>,
    rounds: AtomicU64,
    room: Arc<Semaphore>,
    /// When away began, on this program's own clock.
    away_at: Mutex<Option<Instant>>,
}

/// How often the computers are asked again while nobody has answered and away has only just begun.
const SETTLE_EVERY: Duration = Duration::from_secs(3);

fn refusal(kind: &str, why: impl Into<String>) -> Value {
    json!({"ok": false, "error": why.into(), "kind": kind})
}

fn home_away() -> (u16, Value) {
    (503, refusal("home_away", HOME_AWAY))
}

/// "A", "A and B", "A, B and C".
fn listed(names: &[String]) -> String {
    match names {
        [] => String::new(),
        [one] => one.clone(),
        [most @ .., last] => format!("{} and {last}", most.join(", ")),
    }
}

/// The Host of a request when it is this device itself, without its port: `127.0.0.1`, `localhost`, `::1`.
fn loopback_host(head: &Head) -> Option<String> {
    let h = head.header("host")?.trim().to_ascii_lowercase();
    let host = match h.strip_prefix('[') {
        Some(rest) => rest.split(']').next().unwrap_or(""),
        None => h.split(':').next().unwrap_or(""),
    };
    ["127.0.0.1", "localhost", "::1"].contains(&host).then(|| host.to_string())
}

/// A session id as the server takes one.
fn valid_sid(s: &str) -> bool {
    (1..=64).contains(&s.len()) && s.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
}

fn url_safe(s: &str) -> bool {
    !s.is_empty() && s.len() <= 1024 && s.bytes().all(|c| c.is_ascii_alphanumeric() || b"-_.~".contains(&c))
}

async fn say<S: AsyncWrite + Unpin>(local: &mut S, status: u16, body: &Value) {
    let _ = local.write_all(&http1::answer(status, body.to_string().as_bytes())).await;
    let _ = local.flush().await;
    let _ = local.shutdown().await;
}

impl StandIn {
    pub fn new(backend: Arc<dyn Backend>, opts: Options) -> Arc<StandIn> {
        let room = Arc::new(Semaphore::new(opts.max_streams.max(1)));
        Arc::new(StandIn {
            backend,
            opts,
            away: watch::Sender::new(None),
            last_seen: AtomicU64::new(0),
            hub_port: AtomicU16::new(0),
            merged: tokio::sync::Mutex::new(None),
            last: Mutex::new(None),
            rounds: AtomicU64::new(0),
            room,
            away_at: Mutex::new(None),
        })
    }

    /// The port of this device's hub door, which the `ws` of a terminal names.
    pub fn set_hub_port(&self, port: u16) {
        self.hub_port.store(port, Ordering::Relaxed);
    }

    /// Watches whether home is reached, and begins and ends "away". It never returns: run it for as long as
    /// the ports are open.
    pub async fn run(self: Arc<Self>) {
        let mut down: Option<Instant> = None;
        let mut round: Option<Instant> = None;
        loop {
            if self.backend.home_up() {
                (down, round) = (None, None);
                self.last_seen.store(self.backend.now(), Ordering::Relaxed);
                if self.away.borrow().is_some() {
                    self.away.send_replace(None);
                    *self.away_at.lock().unwrap_or_else(|e| e.into_inner()) = None;
                    *self.last.lock().unwrap_or_else(|e| e.into_inner()) = None;
                    if let Ok(mut m) = self.merged.try_lock() {
                        *m = None;
                    }
                    self.backend.away_changed(false);
                }
            } else {
                let since = *down.get_or_insert_with(Instant::now);
                if self.away.borrow().is_none() && since.elapsed() >= self.opts.away_after {
                    *self.away_at.lock().unwrap_or_else(|e| e.into_inner()) = Some(Instant::now());
                    self.away.send_replace(Some(self.backend.now()));
                    self.backend.away_changed(true);
                }
                if let Some(every) = self.opts.rounds_every
                    && let every = if self.settling(None) { every.min(SETTLE_EVERY) } else { every }
                    && self.away.borrow().is_some()
                    && round.is_none_or(|at| at.elapsed() >= every)
                {
                    round = Some(Instant::now());
                    let me = self.clone();
                    tokio::spawn(async move { me.merged(false).await });
                }
            }
            tokio::time::sleep(self.opts.poll).await;
        }
    }

    /// Away has only just begun and nobody has answered yet, though somebody still may: `m`, or the last round
    /// when None, has a computer that was not reached and none that was. (A computer that refuses, needs an
    /// update or is the home computer will not answer later either.)
    fn settling(&self, m: Option<&Merged>) -> bool {
        let began = *self.away_at.lock().unwrap_or_else(|e| e.into_inner());
        if !began.is_some_and(|t| t.elapsed() < self.opts.settle) {
            return false;
        }
        let waits = |m: &Merged| {
            !m.stale
                && m.rows.iter().all(|r| r.board.is_err())
                && m.rows.iter().any(|r| r.board.as_ref().err() == Some(&Why::Unreachable))
        };
        match m {
            Some(m) => waits(m),
            None => self.last.lock().unwrap_or_else(|e| e.into_inner()).as_deref().is_some_and(waits),
        }
    }

    /// Since when this device is away from home. None: it is not.
    pub fn away_since(&self) -> Option<u64> {
        *self.away.borrow()
    }

    /// The `away` object the apps show, or None when home answers: from the last round of asking the
    /// computers. Before the first round has ended its `line` says they are being asked.
    pub fn away(&self) -> Option<Value> {
        let since = self.away_since()?;
        let last = self.last.lock().unwrap_or_else(|e| e.into_inner()).clone();
        Some(match last {
            Some(m) => self.away_of(since, &m),
            None => {
                let none = Merged { rows: vec![], roster: self.backend.roster(), stale: false, v: 0 };
                let mut a = self.away_of(since, &none);
                a["line"] = json!(ASKING_LINE);
                a
            }
        })
    }

    /// Accepts connections on a loopback port until dropped. Dropping the future closes the port; the
    /// connections it was carrying end with home, or with away.
    pub async fn serve(self: Arc<Self>, listener: TcpListener, target: Target) {
        loop {
            let Ok(place) = self.room.clone().acquire_owned().await else { return };
            let tcp = match listener.accept().await {
                Ok((tcp, _)) => tcp,
                Err(_) => {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    continue;
                }
            };
            let _ = tcp.set_nodelay(true);
            let me = self.clone();
            tokio::spawn(async move {
                let _place = place;
                me.carry(target, tcp).await;
            });
        }
    }

    /// One connection made to this device's API or hub port.
    pub async fn carry<S>(self: Arc<Self>, target: Target, local: S)
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        if self.away_since().is_none() {
            // as the forwarder does: one stream to home, and bytes both ways
            let opened = tokio::time::timeout(tunnel::OPEN_TIMEOUT, self.backend.open_home(target)).await;
            if let Ok(Ok(stream)) = opened {
                let _ = tunnel::pipe(local, stream, &self.opts.pipe).await;
            }
            return;
        }
        let mut ends = self.away.subscribe();
        tokio::select! {
            _ = self.answer(target, local) => {}
            // home is back: this connection goes, and the program behind it connects again
            _ = ends.wait_for(|a| a.is_none()) => {}
        }
    }

    async fn answer<S>(&self, target: Target, mut local: S)
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        let Ok(Ok(head)) = tokio::time::timeout(Duration::from_secs(10), http1::read_head(&mut local)).await else {
            return;
        };
        match target {
            Target::Api => {
                let (status, body) = self.api(&head, &mut local).await;
                say(&mut local, status, &body).await;
            }
            Target::Hub => self.hub(head, local).await,
            Target::Own => {}
        }
    }

    // ---- the API port ------------------------------------------------------------------------------------------

    async fn api<S: AsyncRead + Unpin>(&self, head: &Head, local: &mut S) -> (u16, Value) {
        let Some(host) = loopback_host(head) else {
            return (403, refusal("bad_request", "This port answers this device only."));
        };
        let body = match head.method() {
            "GET" => Value::Null,
            "POST" => {
                // what the server asks of a POST, so no web page can make one
                let json = head.header("content-type").is_some_and(|c| c.starts_with("application/json"));
                if head.header("x-switchboard") != Some("1") || !json {
                    return (403, refusal("bad_request", "That request is not one of Switchboard's."));
                }
                let bytes = match http1::read_request_body(local, head, BODY_MAX).await {
                    Ok(b) => b,
                    Err(_) => return (400, refusal("bad_request", "The request could not be read.")),
                };
                match serde_json::from_slice::<Value>(&bytes) {
                    Ok(v @ Value::Object(_)) => v,
                    _ => return (400, refusal("bad_request", "The request is not a JSON object.")),
                }
            }
            _ => return home_away(),
        };
        match (head.method(), head.path()) {
            ("POST", "/api/event") => {
                self.backend.event(&body);
                (200, json!({"ok": true}))
            }
            ("GET", "/api/summary") => {
                let m = self.merged(false).await;
                (200, self.summary_of(&m))
            }
            ("GET", "/api/sessions") => {
                let m = self.merged(false).await;
                (200, self.sessions_of(&m))
            }
            ("POST", "/api/refresh") => {
                let m = self.merged(true).await;
                (200, self.sessions_of(&m))
            }
            ("POST", "/api/term") => self.term(&host, &body).await,
            ("POST", "/api/action") => self.action(&body).await,
            _ => home_away(),
        }
    }

    /// Every computer on the roster asked for its chats, at most `board_for` ago.
    async fn merged(&self, fresh: bool) -> Arc<Merged> {
        let mut slot = self.merged.lock().await;
        if !fresh
            && let Some((at, m)) = &*slot
            && at.elapsed() < self.opts.board_for
        {
            return m.clone();
        }
        let m = Arc::new(self.merge().await);
        *slot = Some((Instant::now(), m.clone()));
        *self.last.lock().unwrap_or_else(|e| e.into_inner()) = Some(m.clone());
        m
    }

    async fn merge(&self) -> Merged {
        let v = self.rounds.fetch_add(1, Ordering::Relaxed) + 1;
        let Some(roster) = self.backend.roster() else {
            return Merged { rows: vec![], roster: None, stale: false, v };
        };
        let stale = roster.stale(self.backend.now());
        let computers = roster
            .body
            .devices
            .iter()
            .filter(|d| d.kind == Kind::Computer)
            .filter_map(|d| Some((d, d.machine.clone()?)));
        let mut asked = vec![];
        for (d, machine) in computers {
            let fixed = if d.id == roster.body.home {
                Some(Why::IsHome)
            } else if stale {
                // (nobody is asked: every one of them would refuse)
                Some(Why::Refused)
            } else {
                None
            };
            let task = fixed.is_none().then(|| {
                let (backend, device, wait) = (self.backend.clone(), d.clone(), self.opts.board_wait);
                tokio::spawn(async move { ask_board(&*backend, &device, wait).await })
            });
            asked.push((d.clone(), machine, fixed, task));
        }
        let mut rows = vec![];
        for (device, machine, fixed, task) in asked {
            let board = match (fixed, task) {
                (Some(why), _) => Err(why),
                (None, Some(task)) => match task.await {
                    Ok(Ok(mut row)) => {
                        // named as the home computer names it, whatever it calls itself
                        row["name"] = json!(machine);
                        row["label"] = json!(device.name);
                        row["online"] = json!(true);
                        Ok(row)
                    }
                    // Only what the home computer found out says a helper is too old to answer (`own` on
                    // the roster). A version is never guessed from: a computer that is off is out of reach,
                    // whatever it runs.
                    Ok(Err(Why::Unreachable)) if device.own == Some(false) => Err(Why::NeedsUpdate),
                    Ok(Err(why)) => Err(why),
                    Err(_) => Err(Why::Unreachable),
                },
                (None, None) => Err(Why::Unreachable),
            };
            rows.push(Row { device, machine, board });
        }
        Merged { rows, roster: Some(roster), stale, v }
    }

    fn away_of(&self, since: u64, m: &Merged) -> Value {
        let showing: Vec<&Row> = m.rows.iter().filter(|r| r.board.is_ok()).collect();
        let names: Vec<String> = showing.iter().map(|r| r.device.name.clone()).collect();
        let line = if m.stale {
            STALE_LINE.to_string()
        } else if names.is_empty() && self.settling(Some(m)) {
            ASKING_LINE.to_string()
        } else if names.is_empty() {
            "Home is away. No other computer can be reached.".to_string()
        } else {
            format!("Home is away. Showing chats from {}.", listed(&names))
        };
        let missing: Vec<Value> = m
            .rows
            .iter()
            .filter_map(|r| Some((r, r.board.as_ref().err()?)))
            .map(|(r, why)| json!({"machine": r.machine, "label": r.device.name, "why": why.name()}))
            .collect();
        let notes: Vec<String> = m
            .rows
            .iter()
            .filter(|r| r.board.as_ref().err() == Some(&Why::NeedsUpdate))
            .map(|r| Why::NeedsUpdate.sentence(&r.device.name))
            .collect();
        let seen = self.last_seen.load(Ordering::Relaxed);
        json!({
            "since": since,
            "home": {
                "id": m.roster.as_ref().map(|r| r.body.home.to_string()),
                "name": m.roster.as_ref().map(|r| r.body.home_name.clone()),
                "last_seen": (seen > 0).then_some(seen),
            },
            "showing": showing.iter().map(|r| json!({"machine": r.machine, "label": r.device.name})).collect::<Vec<_>>(),
            "missing": missing,
            "roster": m.roster.as_ref().map(|r| json!({"at": r.body.at, "expires": r.expires()})),
            "line": line,
            "notes": notes,
            "lacks": LACKS,
            "lacks_line": LACKS_LINE,
        })
    }

    fn machines_of(&self, m: &Merged) -> Vec<Value> {
        m.rows
            .iter()
            .map(|r| match &r.board {
                Ok(row) => row.clone(),
                Err(why) => json!({
                    "name": r.machine, "label": r.device.name, "online": false, "sessions": [], "replaced": [],
                    "error": why.sentence(&r.device.name), "why": why.name(),
                }),
            })
            .collect()
    }

    /// The counts the server's summary gives, from the computers that answered. What only home knows (usage,
    /// the week) is null.
    fn summary_of(&self, m: &Merged) -> Value {
        let (mut sessions, mut running, mut cold, mut online) = (0u64, 0u64, 0u64, 0u64);
        let mut list = vec![];
        for row in self.machines_of(m) {
            let up = row["online"] == true;
            online += u64::from(up);
            let (mut r, mut c) = (0u64, 0u64);
            for s in row["sessions"].as_array().into_iter().flatten().filter(|s| s["stale"] != true) {
                sessions += 1;
                let working = s["activity"] == "working";
                r += u64::from(working);
                c += u64::from(s["cache"] == "cold" && s["heavy"] == true && !working);
            }
            running += r;
            cold += c;
            list.push(json!({
                "name": row["name"], "label": row["label"], "online": up, "os": row["os"], "icon": row["icon"],
                "running": r, "cold": c,
            }));
        }
        json!({
            "five_hour": null, "seven_day": null, "five_hour_reset": null, "seven_day_reset": null,
            "limits_updated": null, "week_projected": null, "week_daily": [], "week_trace": [], "tz": null,
            "sessions": sessions, "running": running, "cold": cold, "online": online, "machines": m.rows.len(),
            "machine_list": list,
            "away": self.away_since().map(|since| self.away_of(since, m)),
        })
    }

    fn sessions_of(&self, m: &Merged) -> Value {
        let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0);
        json!({
            "machines": self.machines_of(m), "now": now, "summary": self.summary_of(m), "v": m.v, "prefs": {},
            "away": self.away_since().map(|since| self.away_of(since, m)),
        })
    }

    /// The computer a request is for: the one it names, or the one whose board shows the chat.
    async fn owner(&self, body: &Value) -> Result<RosterDevice, (u16, Value)> {
        let m = self.merged(false).await;
        let gone = || (404, refusal("not_here", "That chat isn't on the list anymore."));
        let row = match body["machine"].as_str() {
            Some(name) => m.rows.iter().find(|r| r.machine == name),
            None => m.rows.iter().find(|r| {
                let chats = ["sessions", "replaced"].into_iter();
                let mut chats =
                    chats.flat_map(|k| r.board.as_ref().ok().and_then(|b| b[k].as_array()).into_iter().flatten());
                chats.any(|c| {
                    (body["sid"].is_string() && c["sid"] == body["sid"])
                        || (body["bg"].is_string() && c["bg_id"] == body["bg"])
                })
            }),
        };
        let row = row.ok_or_else(gone)?;
        match &row.board {
            Err(Why::IsHome) => Err(home_away()),
            Err(why) if body["machine"].is_null() => Err((502, refusal("failed", why.sentence(&row.device.name)))),
            // (named outright: it is asked even if it did not answer the last round)
            _ => Ok(row.device.clone()),
        }
    }

    /// One question to a computer's own surface.
    async fn ask_own(&self, device: &RosterDevice, path: &str, body: &Value) -> (u16, Value) {
        let work = async {
            let mut s = self.backend.open_own(device).await?;
            http1::ask(&mut s, "POST", path, Some(body.to_string().as_bytes()), BODY_MAX).await
        };
        match tokio::time::timeout(Duration::from_secs(70), work).await {
            Ok(Ok(a)) => match a.json() {
                Some(v @ Value::Object(_)) => (a.status, v),
                _ => (502, refusal("failed", format!("{} gave no answer.", device.name))),
            },
            Ok(Err(e)) if e.kind() == io::ErrorKind::PermissionDenied => {
                (502, refusal("failed", Why::Refused.sentence(&device.name)))
            }
            _ => (502, refusal("failed", Why::Unreachable.sentence(&device.name))),
        }
    }

    async fn term(&self, host: &str, body: &Value) -> (u16, Value) {
        let ask = match (body["bg"].as_str(), body["sid"].as_str()) {
            (Some(bg), _) => json!({"bg": bg}),
            (None, Some(sid)) if valid_sid(sid) => json!({"sid": sid}),
            _ => return (400, refusal("bad_request", "That is not a session id.")),
        };
        let device = match self.owner(body).await {
            Ok(d) => d,
            Err(no) => return no,
        };
        let (status, mut v) = self.ask_own(&device, "/own/v1/term", &ask).await;
        if status != 200 || v["ok"] != true {
            return (status, v);
        }
        let machine = device.machine.clone().unwrap_or_default();
        let Some(t) = v["t"].as_str().filter(|t| url_safe(t)).map(str::to_string) else {
            return (502, refusal("failed", format!("{} gave no answer.", device.name)));
        };
        let host = if host.contains(':') { format!("[{host}]") } else { host.to_string() };
        let port = self.hub_port.load(Ordering::Relaxed);
        if let Some(o) = v.as_object_mut() {
            o.remove("t");
        }
        v["ws"] = json!(format!("ws://{host}:{port}/term?t={t}&own={machine}"));
        v["machine"] = json!(machine);
        v["label"] = json!(device.name);
        (200, v)
    }

    async fn action(&self, body: &Value) -> (u16, Value) {
        let Some(verb) = body["verb"].as_str().filter(|v| VERBS.contains(v)) else {
            return home_away();
        };
        let Some(sid) = body["sid"].as_str().filter(|s| valid_sid(s)) else {
            return (400, refusal("bad_request", "That is not a session id."));
        };
        let device = match self.owner(body).await {
            Ok(d) => d,
            Err(no) => return no,
        };
        let answer = self.ask_own(&device, "/own/v1/action", &json!({"verb": verb, "sid": sid})).await;
        // the chat changed: the next look asks again
        *self.merged.lock().await = None;
        answer
    }

    // ---- the hub port ------------------------------------------------------------------------------------------

    /// `GET /term?t=..&own=<machine>`: carried to that computer's own surface, where the ticket was made.
    async fn hub<S>(&self, head: Head, mut local: S)
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        let roster = self.backend.roster();
        let device = (head.method() == "GET" && head.path() == "/term" && loopback_host(&head).is_some())
            .then(|| head.query("own"))
            .flatten()
            .and_then(|m| {
                let r = roster.as_ref()?;
                let found = r.body.devices.iter().find(|d| d.kind == Kind::Computer && d.machine.as_deref() == Some(m));
                found.filter(|d| d.id != r.body.home).cloned()
            });
        let Some(device) = device else {
            let (status, body) = home_away();
            return say(&mut local, status, &body).await;
        };
        let opened = tokio::time::timeout(tunnel::OPEN_TIMEOUT, self.backend.open_own(&device)).await;
        let Ok(Ok(mut stream)) = opened else {
            let no = refusal("failed", Why::Unreachable.sentence(&device.name));
            return say(&mut local, 502, &no).await;
        };
        // The request goes on as it came, but for its first line (the path the own surface serves, without
        // `own`) and the Origin of a page of this device's own, which that computer could not know.
        let query = head.target().split_once('?').map(|(_, q)| q).unwrap_or("");
        let kept: Vec<&str> =
            query.split('&').filter(|p| !p.is_empty() && !p.starts_with("own=") && *p != "own").collect();
        let mut out = format!("GET /own/v1/term?{} HTTP/1.1\r\n", kept.join("&")).into_bytes();
        let text = String::from_utf8_lossy(&head.raw);
        for line in text.split("\r\n").skip(1).filter(|l| !l.is_empty()) {
            let own_page = line.split_once(':').is_some_and(|(k, v)| {
                let from = v.trim().to_ascii_lowercase();
                let host = from.split("://").nth(1).unwrap_or("");
                let host = host
                    .strip_prefix('[')
                    .map_or_else(|| host.split(':').next().unwrap_or(""), |h| h.split(']').next().unwrap_or(""));
                k.trim().eq_ignore_ascii_case("origin") && ["127.0.0.1", "localhost", "::1"].contains(&host)
            });
            if !own_page {
                out.extend_from_slice(line.as_bytes());
                out.extend_from_slice(b"\r\n");
            }
        }
        out.extend_from_slice(b"\r\n");
        out.extend_from_slice(&head.rest);
        if stream.write_all(&out).await.is_err() {
            return;
        }
        let _ = tunnel::pipe(local, stream, &self.opts.pipe).await;
    }
}

/// One computer's row of the board, from its own surface.
async fn ask_board(backend: &dyn Backend, device: &RosterDevice, wait: Duration) -> Result<Value, Why> {
    let work = async {
        let mut s = backend.open_own(device).await?;
        http1::ask(&mut s, "GET", "/own/v1/board", None, BOARD_MAX).await
    };
    match tokio::time::timeout(wait, work).await {
        Ok(Ok(a)) => match (a.status, a.json()) {
            (200, Some(v)) if v["machine"].is_object() => Ok(v["machine"].clone()),
            (401 | 403, _) => Err(Why::Refused),
            _ => Err(Why::Unreachable),
        },
        Ok(Err(e)) if e.kind() == io::ErrorKind::PermissionDenied => Err(Why::Refused),
        _ => Err(Why::Unreachable),
    }
}

#[cfg(test)]
#[path = "standin_tests.rs"]
mod tests;
