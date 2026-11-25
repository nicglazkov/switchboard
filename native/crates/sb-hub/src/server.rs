//! The hub's HTTP side: `/term` (WebSocket), `/healthz`, and `/internal/open` for the server's reaper. The node
//! routes (`/node`, `/pair`, `/internal/rpc`, ...) are in nodeapi.rs.
//!
//! Two listeners share one hub. The main one (`router`) has every route. The tunnel one (`public_router`, on
//! 127.0.0.1:8136 when keys are on) has only what another computer may ask.

use crate::config::Config;
use crate::link::{Link, Links};
use crate::machines::{self, Machine};
use crate::nodes::Nodes;
use crate::origin;
use crate::proto::{self, Control};
use crate::pty::{self, Term};
use crate::shared::{Backend, Hello, Out, Seat, Shared};
use crate::ticket::{self, Claims, Nonces};
use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade};
use axum::extract::{ConnectInfo, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub struct Hub {
    pub cfg: Config,
    pub nodes: Nodes,
    pub links: Links,
    key: Vec<u8>,
    nonces: Nonces,
    open: Mutex<HashMap<u64, OpenTerm>>,
    next: AtomicU64,
    /// One terminal per chat, shared by everyone looking at it (shared.rs).
    sessions: tokio::sync::Mutex<HashMap<(String, String), Arc<Shared>>>,
    /// Set in a hub that a helper runs inside itself: terminals on this computer and nowhere else.
    local: Option<Local>,
}

/// What a hub inside a helper (switchboard-node) knows: the one computer it opens terminals on, and the program
/// for a chat there. Such a hub reads no machine list, has no linked computers and starts no ssh.
pub struct Local {
    /// The hub's name for this computer. A ticket for any other name is refused.
    pub machine: String,
    pub label: String,
    /// The program that attaches to the chat with this background id.
    pub command: Box<dyn Fn(&str) -> LocalCommand + Send + Sync>,
}

pub struct LocalCommand {
    pub argv: Vec<String>,
    pub env: Vec<(String, String)>,
    pub env_remove: Vec<String>,
    pub cwd: Option<std::path::PathBuf>,
}

#[derive(Debug, Clone, Serialize)]
pub struct OpenTerm {
    pub machine: String,
    pub bg: String,
    pub since: u64,
}

pub fn unix_now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

pub fn log(msg: impl AsRef<str>) {
    eprintln!("{}", msg.as_ref());
}

/// A place in the open list; leaving it (drop) frees the place.
struct Slot {
    hub: Arc<Hub>,
    id: u64,
}

impl Drop for Slot {
    fn drop(&mut self) {
        self.hub.open.lock().unwrap_or_else(|e| e.into_inner()).remove(&self.id);
    }
}

impl Hub {
    pub fn new(cfg: Config) -> std::io::Result<Arc<Hub>> {
        let key = ticket::load_or_create_key(&cfg.key_path)?;
        let nodes = Nodes::load(&cfg.nodes_path);
        Ok(Arc::new(Hub {
            cfg,
            nodes,
            links: Links::default(),
            key,
            nonces: Nonces::default(),
            open: Mutex::new(HashMap::new()),
            next: AtomicU64::new(1),
            sessions: tokio::sync::Mutex::new(HashMap::new()),
            local: None,
        }))
    }

    /// A hub for terminals on this computer only (`Local`). `key` signs its tickets; it is kept in memory and
    /// written nowhere, so a ticket dies with the program that made it. Nothing is read from `cfg`'s files.
    pub fn local(cfg: Config, key: Vec<u8>, local: Local) -> Arc<Hub> {
        Arc::new(Hub {
            nodes: Nodes::load(std::path::Path::new("")),
            cfg,
            links: Links::default(),
            key,
            nonces: Nonces::default(),
            open: Mutex::new(HashMap::new()),
            next: AtomicU64::new(1),
            sessions: tokio::sync::Mutex::new(HashMap::new()),
            local: Some(local),
        })
    }

    /// A ticket for one chat on this computer, good once and for `ticket::TTL` seconds. None in a hub that is
    /// not `local`: there the server mints tickets, from the key file.
    pub fn local_ticket(&self, bg: &str) -> Option<String> {
        let local = self.local.as_ref()?;
        ticket::valid_bg(bg).then(|| ticket::mint(&self.key, &local.machine, bg, unix_now(), ticket::TTL))
    }

    pub fn open_terms(&self) -> Vec<OpenTerm> {
        let mut v: Vec<OpenTerm> = self.open.lock().unwrap_or_else(|e| e.into_inner()).values().cloned().collect();
        v.sort_by(|a, b| (a.since, &a.machine, &a.bg).cmp(&(b.since, &b.machine, &b.bg)));
        v
    }

    /// A place for one more view of the chat in `c`, or what to tell the viewer when there is none.
    fn admit(self: &Arc<Self>, c: &Claims) -> Result<Slot, String> {
        let mut open = self.open.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(msg) = full(&open, c, self.cfg.max_terms, self.cfg.max_viewers) {
            return Err(msg);
        }
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        open.insert(id, OpenTerm { machine: c.m.clone(), bg: c.b.clone(), since: unix_now() });
        Ok(Slot { hub: self.clone(), id })
    }

    /// `admit`'s answer without taking the place: asked before the ticket is spent.
    fn full(&self, c: &Claims) -> Option<String> {
        let open = self.open.lock().unwrap_or_else(|e| e.into_inner());
        full(&open, c, self.cfg.max_terms, self.cfg.max_viewers)
    }

    fn terminals(&self) -> usize {
        terminals(&self.open.lock().unwrap_or_else(|e| e.into_inner()))
    }

    fn origins(&self, file: &[String]) -> Vec<String> {
        origin::DEFAULT_ORIGINS
            .iter()
            .map(|s| s.to_string())
            .chain(file.iter().cloned())
            .chain(self.cfg.origins.iter().cloned())
            .collect()
    }

    fn ssh_argv(&self, m: &Machine, bg: &str) -> Vec<String> {
        let mut a: Vec<String> = vec![
            self.cfg.ssh.clone(),
            "-tt".into(),
            "-e".into(),
            "none".into(),
            "-i".into(),
            self.cfg.ssh_key.display().to_string(),
        ];
        // -e none: a `~.` typed or pasted at the start of a line goes to Claude Code, not to ssh as a disconnect
        for o in [
            "BatchMode=yes",
            "IdentitiesOnly=yes",
            "ConnectTimeout=10",
            "ServerAliveInterval=15",
            "ServerAliveCountMax=4",
            "LogLevel=ERROR",
        ] {
            a.push("-o".into());
            a.push(o.into());
        }
        a.extend([m.target.clone(), "term".into(), bg.into()]);
        a
    }
}

/// How many terminals `open` holds: one for each chat, however many devices are looking at it.
fn terminals(open: &HashMap<u64, OpenTerm>) -> usize {
    open.values().map(|t| (&t.machine, &t.bg)).collect::<std::collections::HashSet<_>>().len()
}

/// Why one more view of the chat in `c` cannot open, if it cannot. The terminal limit counts chats, not viewers: a
/// second device looking at a chat that is already open takes no terminal, only one of the (many more) views.
fn full(open: &HashMap<u64, OpenTerm>, c: &Claims, max_terms: usize, max_viewers: usize) -> Option<String> {
    if open.len() >= max_viewers {
        return Some(format!(
            "{max_viewers} views of terminals are open over all your devices, the most at once. Close one, then \
             try again."
        ));
    }
    let shown = open.values().any(|t| t.machine == c.m && t.bg == c.b);
    (!shown && terminals(open) >= max_terms)
        .then(|| format!("{max_terms} terminals are open, the most at once. Close one, then try again."))
}

pub fn router(hub: Arc<Hub>) -> Router {
    Router::new()
        .route("/term", get(term))
        .route("/healthz", get(healthz))
        .route("/internal/open", get(internal_open))
        .merge(crate::nodeapi::routes())
        .layer(axum::middleware::from_fn(internal_guard))
        .with_state(hub)
}

/// The one route of a hub inside a helper (`Hub::local`): the terminal WebSocket, at `path`.
pub fn local_router(hub: Arc<Hub>, path: &str) -> Router {
    Router::new().route(path, get(term)).with_state(hub)
}

/// Everything the tunnel listener answers. Nothing else exists there.
pub const PUBLIC_PATHS: [&str; 4] = ["/term", "/node", "/pair", "/healthz"];

/// The routes for connections a tunnel forwarded (plan section 3.3, "Home targets"): what another computer may
/// ask the hub, and nothing more.
///
/// Such a connection arrives from 127.0.0.1, whoever sent it. On the main listener that would pass `not_local`,
/// which is all that keeps `/internal/*` (run a command on any linked computer, make a pairing code, unpair) to
/// the server on this computer. So this router is built without those routes, and `public_guard` answers 404 for
/// every path that is not on the list before any route is looked at: a route added to `router` later is not
/// reachable here until it is added to the list on purpose.
pub fn public_router(hub: Arc<Hub>) -> Router {
    Router::new()
        .route("/term", get(term))
        .route("/healthz", get(healthz))
        .merge(crate::nodeapi::public_routes())
        .layer(axum::middleware::from_fn(public_guard))
        .with_state(hub)
}

fn public_path(path: &str) -> bool {
    PUBLIC_PATHS.contains(&path)
}

async fn public_guard(req: axum::extract::Request, next: axum::middleware::Next) -> Response {
    if !public_path(req.uri().path()) {
        return (StatusCode::NOT_FOUND, Json(json!({"ok": false, "error": "not found"}))).into_response();
    }
    next.run(req).await
}

/// `/internal/*` is for the server on this machine, never a page in a browser here: a page could POST text/plain
/// without a preflight, or read the answers through DNS rebinding. So no Origin or Sec-Fetch-Site, a loopback Host,
/// and JSON for a POST. (`not_local` still checks the peer.)
async fn internal_guard(req: axum::extract::Request, next: axum::middleware::Next) -> Response {
    if req.uri().path().starts_with("/internal/") && !internal_ok(req.headers(), req.method()) {
        return (StatusCode::FORBIDDEN, Json(json!({"ok": false, "error": "only from this machine"}))).into_response();
    }
    next.run(req).await
}

fn internal_ok(h: &HeaderMap, method: &axum::http::Method) -> bool {
    if h.contains_key(header::ORIGIN) || h.contains_key("sec-fetch-site") {
        return false;
    }
    let host = h.get(header::HOST).and_then(|v| v.to_str().ok()).unwrap_or("");
    let name = match host.strip_prefix('[') {
        Some(rest) => rest.split(']').next().unwrap_or(""),
        None => host.rsplit_once(':').map_or(host, |(n, _)| n),
    };
    let json =
        h.get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).is_some_and(|c| c.starts_with("application/json"));
    matches!(name, "127.0.0.1" | "localhost" | "::1") && (method != axum::http::Method::POST || json)
}

async fn healthz(State(hub): State<Arc<Hub>>) -> impl IntoResponse {
    let (open, viewers) = hub.open.lock().map(|o| (terminals(&o), o.len())).unwrap_or((0, 0));
    let body = json!({"ok": true, "service": "switchboard-hub", "version": env!("CARGO_PKG_VERSION"), "open": open, "max": hub.cfg.max_terms,
                      "viewers": viewers, "max_viewers": hub.cfg.max_viewers, "linked": hub.links.len()});
    ([(header::CACHE_CONTROL, "no-store")], Json(body))
}

/// The `/internal/*` routes answer only this machine (the server); anything else gets this 403.
pub fn not_local(peer: &SocketAddr) -> Option<Response> {
    (!peer.ip().to_canonical().is_loopback())
        .then(|| (StatusCode::FORBIDDEN, Json(json!({"ok": false, "error": "only from this machine"}))).into_response())
}

/// Which background sessions have a terminal open right now. The server's reaper never stops those.
async fn internal_open(State(hub): State<Arc<Hub>>, ConnectInfo(peer): ConnectInfo<SocketAddr>) -> Response {
    if let Some(r) = not_local(&peer) {
        return r;
    }
    ([(header::CACHE_CONTROL, "no-store")], Json(json!({"ok": true, "open": hub.open_terms()}))).into_response()
}

#[derive(Debug, Default, Deserialize)]
pub struct TermQuery {
    t: Option<String>,
    cols: Option<String>,
    rows: Option<String>,
    /// `2`: the client knows size frames and snapshots (spec 2026-10-14).
    proto: Option<String>,
    /// `0`: the client keeps its own scrollback; its first snapshot is only the screen.
    history: Option<String>,
}

async fn term(
    ws: WebSocketUpgrade,
    State(hub): State<Arc<Hub>>,
    headers: HeaderMap,
    Query(q): Query<TermQuery>,
) -> Response {
    // a header that is not text counts as an origin, and matches nothing
    let origin = headers.get(header::ORIGIN).map(|v| v.to_str().unwrap_or("").to_string());
    ws.max_message_size(1 << 20).max_frame_size(1 << 20).on_upgrade(move |socket| session(hub, socket, origin, q))
}

/// How long one message to a viewer may take. A viewer that stopped reading (it slept, or its network went away)
/// fills the socket, and without this the send would wait until the system gives up on it, minutes later.
const SEND_PATIENCE: Duration = Duration::from_secs(10);

/// Sends one message to the viewer. False if it left, or took nothing for `SEND_PATIENCE`.
async fn send(ws: &mut WebSocket, m: Message) -> bool {
    matches!(tokio::time::timeout(SEND_PATIENCE, ws.send(m)).await, Ok(Ok(())))
}

async fn send_text(ws: &mut WebSocket, text: String) -> bool {
    send(ws, Message::Text(text.into())).await
}

async fn close(ws: &mut WebSocket, code: u16, reason: &str) {
    let mut r = reason.to_string();
    while r.len() > 120 {
        r.pop();
    }
    if !send(ws, Message::Close(Some(CloseFrame { code, reason: r.into() }))).await {
        return;
    }
    // let the client's close come back, so the code reaches it cleanly
    let _ = tokio::time::timeout(Duration::from_secs(2), async {
        while let Some(Ok(m)) = ws.recv().await {
            if matches!(m, Message::Close(_)) {
                break;
            }
        }
    })
    .await;
}

async fn refuse(ws: &mut WebSocket, code: u16, msg: &str) {
    send_text(ws, proto::error(msg)).await;
    close(ws, code, msg).await;
}

fn size_from(q: &TermQuery) -> (u16, u16) {
    let num = |s: &Option<String>, d: u16| s.as_deref().and_then(|v| v.parse::<u32>().ok()).unwrap_or(d as u32);
    proto::clamp_size(num(&q.cols, proto::DEFAULT_COLS), num(&q.rows, proto::DEFAULT_ROWS))
}

enum Route {
    /// The chat's program in a PTY on this computer (`Hub::local`).
    Local,
    Ssh(Machine),
    Node(Arc<Link>, String),
}

enum End {
    Exited(Option<i64>),
    ClientLeft,
    Idle,
}

async fn session(hub: Arc<Hub>, mut ws: WebSocket, origin: Option<String>, q: TermQuery) {
    let file = match &hub.local {
        Some(_) => Ok(machines::MachineFile::default()),
        None => machines::load(&hub.cfg.machines_path),
    };
    let file_origins = file.as_ref().map(|f| f.origins.clone()).unwrap_or_default();
    if !origin::allowed(origin.as_deref(), &hub.origins(&file_origins)) {
        log(format!("term refused: origin {:?}", origin.unwrap_or_default()));
        return refuse(&mut ws, proto::CLOSE_ORIGIN, "This page is not allowed to open terminals.").await;
    }
    // Checked here, spent further down: a ticket works once, and a full hub, a machine that is not linked this
    // second or a hub with no machine list yet must not use that one time up.
    let claims = match ticket::verify(&hub.key, q.t.as_deref().unwrap_or(""), unix_now()) {
        Ok(c) => c,
        Err(e) => {
            log(format!("term refused: ticket {e:?}"));
            return refuse(&mut ws, proto::CLOSE_TICKET, e.message()).await;
        }
    };
    if let Some(msg) = hub.full(&claims) {
        // (which chat the ticket is for decides whether it needs a terminal of its own; admit checks again)
        return refuse(&mut ws, proto::CLOSE_LIMIT, &msg).await;
    }
    let paired = hub.nodes.get(&claims.m);
    let route = match (hub.links.get(&claims.m), &paired, &file) {
        _ if hub.local.is_some() => {
            // (the key is this program's own, so only a ticket it minted gets here; the name is checked all the same)
            if hub.local.as_ref().is_some_and(|l| l.machine != claims.m) {
                log("term refused: a ticket for another computer");
                return refuse(&mut ws, proto::CLOSE_TICKET, "This terminal ticket is for another computer.").await;
            }
            Route::Local
        }
        (Some(link), _, _) => {
            Route::Node(link, paired.as_ref().map(|n| n.label().to_string()).unwrap_or_else(|| claims.m.clone()))
        }
        (None, Some(n), _) => {
            let msg = format!(
                "{} is not linked to the hub right now. Check that switchboard-node is running there.",
                n.label()
            );
            return refuse(&mut ws, proto::CLOSE_INTERNAL, &msg).await;
        }
        (None, None, Err(e)) => {
            log(format!("term failed: machine list: {e}"));
            return refuse(
                &mut ws,
                proto::CLOSE_INTERNAL,
                "The hub has no machine list yet. Restart the Switchboard server.",
            )
            .await;
        }
        (None, None, Ok(f)) => match f.machines.iter().find(|m| m.name == claims.m) {
            Some(m) => Route::Ssh(m.clone()),
            None => {
                return refuse(
                    &mut ws,
                    proto::CLOSE_INTERNAL,
                    &format!("The hub does not know the machine {}.", claims.m),
                )
                .await;
            }
        },
    };
    if !hub.nonces.spend(&claims, unix_now()) {
        let e = ticket::TicketError::Used;
        log(format!("term refused: ticket {e:?}"));
        return refuse(&mut ws, proto::CLOSE_TICKET, e.message()).await;
    }
    let slot = match hub.admit(&claims) {
        Ok(slot) => slot,
        Err(msg) => {
            log(format!("term refused: {} terminals open, {} views", hub.terminals(), hub.open_terms().len()));
            return refuse(&mut ws, proto::CLOSE_LIMIT, &msg).await;
        }
    };
    let (cols, rows) = size_from(&q);
    let hello =
        Hello { cols, rows, modern: q.proto.as_deref() == Some("2"), history: q.history.as_deref() != Some("0") };
    if !send_text(&mut ws, proto::status("connecting")).await {
        return;
    }
    let key = (claims.m.clone(), claims.b.clone());
    let mut started: Option<(Term, pty::TermIo)> = None;
    let (shared, seat) = loop {
        // under the map's lock: a viewer joining and the last one leaving never cross
        let mut map = hub.sessions.lock().await;
        if let Some(s) = map.get(&key).filter(|s| s.alive()).cloned() {
            // (an ssh started for this viewer while another viewer opened the same chat is dropped: it hangs up)
            // Dropped here, not when this viewer leaves: until then it was a second `claude attach` on the chat.
            drop(started.take());
            let seat = s.join(hello);
            break (s, seat);
        }
        let (backend, io, label, via) = match (&route, started.take()) {
            (Route::Node(link, label), _) => {
                let (t, io) = link.open_term(&claims.b, cols, rows);
                (Backend::Node(t), io, label.clone(), "node")
            }
            (Route::Ssh(machine), Some((t, io))) => (Backend::Pty(t), io, machine.label().to_string(), "ssh"),
            (Route::Local, Some((t, io))) => {
                let label = hub.local.as_ref().map(|l| l.label.clone()).unwrap_or_default();
                (Backend::Pty(t), io, label, "local")
            }
            (Route::Local, None) => {
                // (as for ssh below: started outside the lock, then looked at again)
                drop(map);
                match spawn_local(&hub, &claims.b, cols, rows).await {
                    Ok(x) => started = Some(x),
                    Err(e) => {
                        log(format!("term failed: spawn: {e}"));
                        return refuse(&mut ws, proto::CLOSE_INTERNAL, "This computer could not start the terminal.")
                            .await;
                    }
                }
                continue;
            }
            (Route::Ssh(machine), None) => {
                // Starting ssh can take a moment. Not under the lock, which every terminal opening or closing
                // needs; then look again, in case another viewer opened this chat meanwhile.
                drop(map);
                match spawn_ssh(&hub, machine, &claims.b, cols, rows).await {
                    Ok(x) => started = Some(x),
                    Err(e) => {
                        log(format!("term failed: spawn: {e}"));
                        return refuse(&mut ws, proto::CLOSE_INTERNAL, "The hub could not start ssh.").await;
                    }
                }
                continue;
            }
        };
        let s = Shared::new(&claims.m, &claims.b, label, via, backend, cols, rows);
        tokio::spawn(pump(hub.clone(), s.clone(), io));
        map.insert(key.clone(), s.clone());
        let seat = s.join(hello);
        break (s, seat);
    };
    let started = Instant::now();
    log(format!(
        "term open {} {} {cols}x{rows} by {}, {} looking ({} open)",
        claims.m,
        claims.b,
        shared.via,
        shared.viewers(),
        hub.terminals()
    ));
    let end = relay(&hub, &mut ws, &shared, seat).await;
    drop(slot); // and the reaper may stop the session again
    let note = match &end {
        End::Exited(code) => Some((*code, proto::exit_note(*code, &shared.label), format!("exit {code:?}"))),
        End::Idle => Some((
            None,
            format!("Closed after {} with nothing typed or shown. The chat keeps running.", hours(hub.cfg.idle)),
            "idle".into(),
        )),
        End::ClientLeft => None,
    };
    let why = match note {
        Some((code, note, why)) => {
            send_text(&mut ws, proto::exit(code, &note)).await;
            close(&mut ws, proto::CLOSE_NORMAL, "").await;
            why
        }
        None => "client left".into(),
    };
    log(format!(
        "term close {} {} after {}s, {} KB out so far, {} still looking: {why}",
        claims.m,
        claims.b,
        started.elapsed().as_secs(),
        shared.out_bytes() / 1024,
        shared.viewers()
    ));
}

/// A viewer is done (it left, or sat idle): the last one hangs up, and `claude attach` detaches.
async fn leave(hub: &Hub, shared: &Arc<Shared>, id: u64) {
    let mut map = hub.sessions.lock().await;
    if shared.leave(id) {
        let key = (shared.machine.clone(), shared.bg.clone());
        if map.get(&key).is_some_and(|s| Arc::ptr_eq(s, shared)) {
            map.remove(&key);
        }
        shared.close();
    }
}

/// Reads the program's output into the shared screen and out to every viewer, until it ends.
async fn pump(hub: Arc<Hub>, shared: Arc<Shared>, mut io: pty::TermIo) {
    loop {
        let deadline = shared.sync_deadline().map(tokio::time::Instant::from_std);
        tokio::select! {
            chunk = io.output.recv() => match chunk {
                Some(b) => shared.output(b),
                None => break,
            },
            // a synchronized update the program never ended: draw it anyway, as a terminal does
            _ = tokio::time::sleep_until(deadline.unwrap_or_else(tokio::time::Instant::now)), if deadline.is_some() => {
                shared.flush_sync();
            }
        }
    }
    // everything it wrote is read; the exit code follows at once
    let code = tokio::time::timeout(Duration::from_secs(3), &mut io.exited).await.ok().and_then(|r| r.ok()).flatten();
    {
        let mut map = hub.sessions.lock().await;
        let key = (shared.machine.clone(), shared.bg.clone());
        if map.get(&key).is_some_and(|s| Arc::ptr_eq(s, &shared)) {
            map.remove(&key);
        }
    }
    shared.ended(code);
}

async fn spawn_ssh(
    hub: &Hub,
    machine: &Machine,
    bg: &str,
    cols: u16,
    rows: u16,
) -> std::io::Result<(Term, pty::TermIo)> {
    let argv = hub.ssh_argv(machine, bg);
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    let env = [
        ("TERM".to_string(), "xterm-256color".to_string()),
        ("COLORTERM".to_string(), "truecolor".to_string()),
        ("LANG".to_string(), "en_US.UTF-8".to_string()),
    ];
    let remove = ["LC_ALL".to_string(), "LC_CTYPE".to_string(), "SSH_AUTH_SOCK".to_string()];
    let rate = hub.cfg.rate;
    tokio::task::spawn_blocking(move || {
        pty::spawn(pty::Spawn { argv: &argv, env: &env, env_remove: &remove, cwd: home.as_deref(), cols, rows, rate })
    })
    .await
    .map_err(std::io::Error::other)?
}

async fn spawn_local(hub: &Hub, bg: &str, cols: u16, rows: u16) -> std::io::Result<(Term, pty::TermIo)> {
    let Some(local) = &hub.local else { return Err(std::io::Error::other("not a local hub")) };
    let c = (local.command)(bg);
    let rate = hub.cfg.rate;
    tokio::task::spawn_blocking(move || {
        pty::spawn(pty::Spawn {
            argv: &c.argv,
            env: &c.env,
            env_remove: &c.env_remove,
            cwd: c.cwd.as_deref(),
            cols,
            rows,
            rate,
        })
    })
    .await
    .map_err(std::io::Error::other)?
}

fn hours(d: Duration) -> String {
    let s = d.as_secs();
    if s >= 3600 && s.is_multiple_of(3600) {
        let h = s / 3600;
        format!("{h} hour{}", if h == 1 { "" } else { "s" })
    } else if s >= 60 {
        format!("{} minute{}", s / 60, if s / 60 == 1 { "" } else { "s" })
    } else {
        format!("{s} second{}", if s == 1 { "" } else { "s" })
    }
}

/// A snapshot goes out in binary frames of at most this much, between `snapshot_begin` and `snapshot_end`.
const SNAPSHOT_FRAME: usize = 256 * 1024;

/// Sends one queued message to the viewer. False if it left.
async fn send_out(ws: &mut WebSocket, o: Out, sent_open: &mut bool) -> bool {
    match o {
        Out::Open if *sent_open => true,
        Out::Open => {
            *sent_open = true;
            send_text(ws, proto::status("open")).await
        }
        Out::Bytes(b) => send(ws, Message::Binary(b.into())).await,
        Out::Size(c, r) => send_text(ws, proto::size(c, r)).await,
        Out::Owner(o) => send_text(ws, proto::owner(o)).await,
        Out::Snapshot(bytes) => {
            if !send_text(ws, proto::snapshot_begin()).await {
                return false;
            }
            for piece in bytes.chunks(SNAPSHOT_FRAME) {
                if !send(ws, Message::Binary(piece.to_vec().into())).await {
                    return false;
                }
            }
            send_text(ws, proto::snapshot_end()).await
        }
    }
}

/// Input one viewer has sent that the program has not taken yet. Past this the relay stops reading that viewer's
/// socket until there is room again, so a paste the program never reads cannot grow without limit.
const INPUT_WAITING: usize = 4 << 20;

/// Typed and pasted bytes on their way to the program, written by a task of their own: a program that is slow to
/// read (a big paste) then holds up neither this viewer's output nor the answers to its pings.
struct Input {
    tx: tokio::sync::mpsc::UnboundedSender<axum::body::Bytes>,
    waiting: Arc<std::sync::atomic::AtomicUsize>,
    room: Arc<tokio::sync::Notify>,
}

impl Input {
    fn start(shared: &Arc<Shared>, id: u64) -> Input {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<axum::body::Bytes>();
        let waiting = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let room = Arc::new(tokio::sync::Notify::new());
        let (shared, w, r) = (shared.clone(), waiting.clone(), room.clone());
        tokio::spawn(async move {
            // ends when the viewer's relay does; what it already sent is still written
            while let Some(b) = rx.recv().await {
                // waits while the machine is slow to read; if the terminal is gone, its output ends too and the
                // exit follows
                shared.input(id, &b).await;
                w.fetch_sub(b.len(), Ordering::Relaxed);
                r.notify_one();
            }
        });
        Input { tx, waiting, room }
    }

    fn send(&self, b: axum::body::Bytes) {
        self.waiting.fetch_add(b.len(), Ordering::Relaxed);
        if let Err(e) = self.tx.send(b) {
            self.waiting.fetch_sub(e.0.len(), Ordering::Relaxed);
        }
    }

    fn full(&self) -> bool {
        self.waiting.load(Ordering::Relaxed) >= INPUT_WAITING
    }
}

async fn relay(hub: &Hub, ws: &mut WebSocket, shared: &Arc<Shared>, seat: Seat) -> End {
    let Seat { id, rx, behind } = seat;
    let input = Input::start(shared, id);
    let mut sent_open = false;
    let mut last = tokio::time::Instant::now();
    // pings at the WebSocket level, which every client answers by itself, so a viewer that went away is noticed
    let mut ping = tokio::time::interval(hub.cfg.silence / 3);
    ping.tick().await;
    let mut heard = tokio::time::Instant::now();
    let end = loop {
        // too much input waiting: this viewer's socket is not read (so not counted as silent either) until some went
        let full = input.full();
        tokio::select! {
            out = rx.recv() => match out {
                Some(o) => {
                    if matches!(o, Out::Bytes(_)) {
                        last = tokio::time::Instant::now();
                    }
                    if !send_out(ws, o, &mut sent_open).await {
                        break End::ClientLeft;
                    }
                    // it fell behind and has now read everything queued: the screen instead of what it missed
                    if rx.is_empty() && behind.load(std::sync::atomic::Ordering::Relaxed) {
                        let mut ok = true;
                        for o in shared.catch_up(id) {
                            ok = ok && send_out(ws, o, &mut sent_open).await;
                        }
                        if !ok {
                            break End::ClientLeft;
                        }
                    }
                }
                // the program ended and this viewer has read everything it wrote
                None => break End::Exited(shared.exit_code().flatten()),
            },
            msg = ws.recv(), if !full => {
                heard = tokio::time::Instant::now();
                match msg {
                    Some(Ok(Message::Binary(b))) => {
                        last = tokio::time::Instant::now();
                        input.send(b);
                    }
                    Some(Ok(Message::Text(t))) => match proto::parse_control(t.as_str()) {
                        Some(Control::Resize { cols, rows }) => {
                            let (c, r) = proto::clamp_size(cols, rows);
                            shared.want(id, c, r);
                        }
                        Some(Control::Focus) => shared.focus(id),
                        Some(Control::Redraw) => shared.redraw(id),
                        Some(Control::Ping) if !send_text(ws, proto::pong()).await => break End::ClientLeft,
                        Some(Control::Ping) => {}
                        None => {}
                    },
                    Some(Ok(Message::Ping(_) | Message::Pong(_))) => {}
                    Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break End::ClientLeft,
                }
            }
            _ = input.room.notified(), if full => {
                heard = tokio::time::Instant::now();
                last = heard;
            }
            _ = ping.tick() => {
                if (!full && heard.elapsed() > hub.cfg.silence) || !send(ws, Message::Ping(Vec::new().into())).await {
                    break End::ClientLeft;
                }
            }
            // (an idle time of 0 means never)
            _ = tokio::time::sleep_until(last + hub.cfg.idle), if !hub.cfg.idle.is_zero() => break End::Idle,
        }
    };
    if !matches!(end, End::Exited(_)) {
        leave(hub, shared, id).await;
    }
    end
}

pub async fn serve(hub: Arc<Hub>, listener: tokio::net::TcpListener) -> std::io::Result<()> {
    axum::serve(listener, router(hub).into_make_service_with_connect_info::<SocketAddr>()).await
}

/// Serves `public_router` on the tunnel listener. The same hub as `serve`: one list of terminals, links and
/// pairing codes, reached two ways.
pub async fn serve_public(hub: Arc<Hub>, listener: tokio::net::TcpListener) -> std::io::Result<()> {
    axum::serve(listener, public_router(hub).into_make_service_with_connect_info::<SocketAddr>()).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tunnel_listener_knows_four_paths_and_no_other() {
        for p in ["/term", "/node", "/pair", "/healthz"] {
            assert!(public_path(p), "{p}");
        }
        for p in [
            "/internal/open",
            "/internal/pair",
            "/internal/nodes",
            "/internal/rpc/lab",
            "/internal/unpair/lab",
            "/internal",
            "/internal/",
            "//internal/open",
            "/term/../internal/open",
            "/pair/../internal/pair",
            "/%69nternal/open",
            "/INTERNAL/open",
            "/term/",
            "/pair/x",
            "/",
            "",
        ] {
            assert!(!public_path(p), "{p}");
        }
    }

    #[test]
    fn the_terminal_limit_counts_chats_and_the_viewer_limit_counts_views() {
        let claims = |m: &str, b: &str| Claims { m: m.into(), b: b.into(), x: 0, n: String::new() };
        let mut open = HashMap::new();
        // two devices each showing the same two chats: four views, two terminals
        for (id, m) in ["lab", "desk", "lab", "desk"].into_iter().enumerate() {
            open.insert(id as u64, OpenTerm { machine: m.into(), bg: "c03ef0d9".into(), since: 0 });
        }
        assert_eq!((terminals(&open), open.len()), (2, 4));
        // at the terminal limit a third device still opens the chats that are open, but no new one
        assert_eq!(full(&open, &claims("lab", "c03ef0d9"), 2, 64), None);
        let msg = full(&open, &claims("lab", "b52d78ef"), 2, 64).unwrap();
        assert!(msg.starts_with("2 terminals are open"), "{msg}");
        assert_eq!(full(&open, &claims("lab", "b52d78ef"), 3, 64), None);
        // the same chat id on another machine is another terminal
        assert!(full(&open, &claims("nas", "c03ef0d9"), 2, 64).is_some());
        // the viewer limit stops even a chat that is open
        let msg = full(&open, &claims("lab", "c03ef0d9"), 16, 4).unwrap();
        assert!(msg.starts_with("4 views of terminals are open"), "{msg}");
        assert_eq!(full(&open, &claims("lab", "c03ef0d9"), 16, 5), None);
    }
}
