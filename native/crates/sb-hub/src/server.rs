//! The hub's HTTP side: `/term` (WebSocket), `/healthz`, and `/internal/open` for the server's reaper. The node
//! routes (`/node`, `/pair`, `/internal/rpc`, ...) are in nodeapi.rs.

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
        }))
    }

    pub fn open_terms(&self) -> Vec<OpenTerm> {
        let mut v: Vec<OpenTerm> = self.open.lock().unwrap_or_else(|e| e.into_inner()).values().cloned().collect();
        v.sort_by(|a, b| (a.since, &a.machine, &a.bg).cmp(&(b.since, &b.machine, &b.bg)));
        v
    }

    fn admit(self: &Arc<Self>, c: &Claims) -> Option<Slot> {
        let mut open = self.open.lock().unwrap_or_else(|e| e.into_inner());
        if open.len() >= self.cfg.max_terms {
            return None;
        }
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        open.insert(id, OpenTerm { machine: c.m.clone(), bg: c.b.clone(), since: unix_now() });
        Some(Slot { hub: self.clone(), id })
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

pub fn router(hub: Arc<Hub>) -> Router {
    Router::new()
        .route("/term", get(term))
        .route("/healthz", get(healthz))
        .route("/internal/open", get(internal_open))
        .merge(crate::nodeapi::routes())
        .layer(axum::middleware::from_fn(internal_guard))
        .with_state(hub)
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
    let open = hub.open.lock().map(|o| o.len()).unwrap_or(0);
    let body = json!({"ok": true, "service": "switchboard-hub", "version": env!("CARGO_PKG_VERSION"), "open": open, "max": hub.cfg.max_terms,
                      "linked": hub.links.len()});
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
    Ssh(Machine),
    Node(Arc<Link>, String),
}

enum End {
    Exited(Option<i64>),
    ClientLeft,
    Idle,
}

async fn session(hub: Arc<Hub>, mut ws: WebSocket, origin: Option<String>, q: TermQuery) {
    let file = machines::load(&hub.cfg.machines_path);
    let file_origins = file.as_ref().map(|f| f.origins.clone()).unwrap_or_default();
    if !origin::allowed(origin.as_deref(), &hub.origins(&file_origins)) {
        log(format!("term refused: origin {:?}", origin.unwrap_or_default()));
        return refuse(&mut ws, proto::CLOSE_ORIGIN, "This page is not allowed to open terminals.").await;
    }
    if hub.open.lock().map(|o| o.len()).unwrap_or(0) >= hub.cfg.max_terms {
        // full: say so before the ticket is spent, so a full hub never burns the client's one try (admit checks again)
        let msg = format!("{} terminals are open, the most at once. Close one, then try again.", hub.cfg.max_terms);
        return refuse(&mut ws, proto::CLOSE_LIMIT, &msg).await;
    }
    let claims = match ticket::redeem(&hub.key, &hub.nonces, q.t.as_deref().unwrap_or(""), unix_now()) {
        Ok(c) => c,
        Err(e) => {
            log(format!("term refused: ticket {e:?}"));
            return refuse(&mut ws, proto::CLOSE_TICKET, e.message()).await;
        }
    };
    let paired = hub.nodes.get(&claims.m);
    let route = match (hub.links.get(&claims.m), &paired, &file) {
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
    let Some(slot) = hub.admit(&claims) else {
        log(format!("term refused: {} terminals open", hub.cfg.max_terms));
        let msg = format!("{} terminals are open, the most at once. Close one, then try again.", hub.cfg.max_terms);
        return refuse(&mut ws, proto::CLOSE_LIMIT, &msg).await;
    };
    let (cols, rows) = size_from(&q);
    let hello =
        Hello { cols, rows, modern: q.proto.as_deref() == Some("2"), history: q.history.as_deref() != Some("0") };
    if !send_text(&mut ws, proto::status("connecting")).await {
        return;
    }
    let key = (claims.m.clone(), claims.b.clone());
    let (shared, seat) = {
        // under the map's lock: a viewer joining and the last one leaving never cross
        let mut map = hub.sessions.lock().await;
        let shared = match map.get(&key).filter(|s| s.alive()) {
            Some(s) => s.clone(),
            None => {
                let (backend, io, label, via) = match route {
                    Route::Node(link, label) => {
                        let (t, io) = link.open_term(&claims.b, cols, rows);
                        (Backend::Node(t), io, label, "node")
                    }
                    Route::Ssh(machine) => match spawn_ssh(&hub, &machine, &claims.b, cols, rows).await {
                        Ok((t, io)) => (Backend::Ssh(t), io, machine.label().to_string(), "ssh"),
                        Err(e) => {
                            drop(map);
                            log(format!("term failed: spawn: {e}"));
                            return refuse(&mut ws, proto::CLOSE_INTERNAL, "The hub could not start ssh.").await;
                        }
                    },
                };
                let s = Shared::new(&claims.m, &claims.b, label, via, backend, cols, rows);
                tokio::spawn(pump(hub.clone(), s.clone(), io));
                map.insert(key.clone(), s.clone());
                s
            }
        };
        let seat = shared.join(hello);
        (shared, seat)
    };
    let started = Instant::now();
    log(format!(
        "term open {} {} {cols}x{rows} by {}, {} looking ({} open)",
        claims.m,
        claims.b,
        shared.via,
        shared.viewers(),
        hub.open_terms().len()
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
        Out::Bytes(b) => send(ws, Message::Binary(b)).await,
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

async fn relay(hub: &Hub, ws: &mut WebSocket, shared: &Arc<Shared>, seat: Seat) -> End {
    let Seat { id, mut rx, behind } = seat;
    let mut sent_open = false;
    let mut last = tokio::time::Instant::now();
    // pings at the WebSocket level, which every client answers by itself, so a viewer that went away is noticed
    let mut ping = tokio::time::interval(hub.cfg.silence / 3);
    ping.tick().await;
    let mut heard = tokio::time::Instant::now();
    let end = loop {
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
            msg = ws.recv() => {
                heard = tokio::time::Instant::now();
                match msg {
                    Some(Ok(Message::Binary(b))) => {
                        last = tokio::time::Instant::now();
                        // waits while the machine is slow to read (a big paste); if the terminal is gone, its output
                        // ends too and the exit follows
                        shared.input(id, &b).await;
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
            _ = ping.tick() => {
                if heard.elapsed() > hub.cfg.silence || !send(ws, Message::Ping(Vec::new().into())).await {
                    break End::ClientLeft;
                }
            }
            _ = tokio::time::sleep_until(last + hub.cfg.idle) => break End::Idle,
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
