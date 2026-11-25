//! This computer's own surface: its own chats, answered by this helper to the devices on the home computer's
//! roster, so they can be reached while the home computer is away.
//!
//! It is HTTP, served on tunnel streams and on no port: a device opens a stream with the first line
//! `{"to":"own"}` (sb-link tunnel.rs), the endpoint hands the stream over with the caller's id, and this file
//! serves one HTTP/1.1 connection on it, WebSocket upgrades included. Who may open such a stream is decided
//! before this file sees it (sb-link endpoint.rs and roster.rs): at the handshake, when the connection is
//! taken, and again for every stream.
//!
//!   GET  /own/v1/hello      who this is, and the roster it holds
//!   GET  /own/v1/roster     the roster it holds, as home signed it (404: none)
//!   POST /own/v1/roster     {envelope}: a newer one is taken; the newer of the two stands
//!   GET  /own/v1/board      this computer's chats with their live state, as one row of the board
//!   POST /own/v1/term       {"sid"} or {"bg"}: a ticket for that chat's terminal, good once for a minute
//!   GET  /own/v1/term?t=    the terminal, as the hub serves it (a WebSocket)
//!   POST /own/v1/action     {"verb": stop | dismiss | pass | resume, "sid"}
//!
//! The home computer answers the first three only: its chats are reached through its server.
//!
//! A computer that is not home also answers its home computer, and nobody else, about taking over as the home
//! computer (keys/home.rs has the steps, and what each does on disk):
//!
//!   GET  /own/v1/home/status    {"ok", "state": idle | staged, "staged": <sha256> | null, "can_host", "version"}
//!   POST /own/v1/home/receive?sha256=<hex>   the home computer's state as one stream (server/homestate.py): it
//!                                is unpacked into a staging folder and checked; nothing here changes yet
//!   POST /own/v1/home/commit    {"staged", "epoch", "key", "old": {"id" | null, "name", "machine"}, "retire"}:
//!                                this computer becomes the home computer. {"ok", "epoch"}
//!   POST /own/v1/home/discard   {}: what was staged is thrown away
//!
//! Any other caller gets 403 `not_home`, whatever it sent.
//!
//! A refusal is `{"ok": false, "error": "<sentence>", "kind": "<word>"}`: `bad_request` (400), `not_here` (404,
//! or 200 with `need` where the server's `/api/term` answers that way), `busy` (the chat is open in a terminal
//! there) and `failed` (502: the helper script here did not answer).

use crate::NodeConfig;
use crate::link::log;
use axum::extract::{Extension, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use sb_hub::server::{Hub, Local, LocalCommand};
use sb_link::endpoint::OwnIncoming;
use sb_link::identity::{DeviceId, DeviceKey};
use sb_link::roster::{Envelope, Roster, RosterError};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::mpsc;

/// How long one answer of the helper script stands for the board. Callers that arrive while it runs wait for
/// that one run.
pub const BOARD_FOR: Duration = Duration::from_secs(3);
/// What a chat here can be asked to do by a device that reached this computer by itself.
pub const VERBS: [&str; 4] = ["stop", "dismiss", "pass", "resume"];
/// The most a request's body may be: a roster's envelope, with room.
const BODY_MAX: usize = 128 * 1024;
const PING_WHY: [&str; 6] = ["prompt", "stop", "wait", "start", "end", "status"];
/// A ping older than this is forgotten (the helper script itself takes one for five seconds).
const PING_KEPT: f64 = 60.0;

/// The roster this computer holds, and what it does with one it is shown.
pub trait Lists: Send + Sync + 'static {
    fn held(&self) -> Option<Arc<Roster>>;
    /// Ok(true): `envelope` is newer and was taken.
    fn offer(&self, envelope: &Envelope) -> Result<bool, RosterError>;
}

/// A refusal of one of the `home/` routes: the status, the kind and the sentence.
pub type MoveNo = (StatusCode, &'static str, String);
pub type MoveFut<'a> = std::pin::Pin<Box<dyn Future<Output = Result<Value, MoveNo>> + Send + 'a>>;

/// Taking over as the home computer, as the computer that is asked to (keys/home.rs).
pub trait Moves: Send + Sync + 'static {
    /// The home computer this one is paired with: the only caller the `home/` routes answer.
    fn home(&self) -> Option<DeviceId>;
    /// Whether the parts a home computer needs are on this computer.
    fn can_host(&self) -> bool;
    fn status(&self) -> MoveFut<'_>;
    fn receive(&self, sha256: String, body: axum::body::Body) -> MoveFut<'_>;
    fn commit(&self, ask: Value) -> MoveFut<'_>;
    fn discard(&self) -> MoveFut<'_>;
}

/// The device a request came from.
#[derive(Debug, Clone)]
pub struct Caller {
    pub id: DeviceId,
    pub name: String,
}

pub struct Surface {
    id: DeviceId,
    home: bool,
    /// node.json: read at each request for this computer's name at the hub and its helper script.
    config: PathBuf,
    lists: Arc<dyn Lists>,
    /// Set once, on a computer that is not home.
    moves: std::sync::OnceLock<Arc<dyn Moves>>,
    board: tokio::sync::Mutex<Option<(Instant, Value)>>,
    pings: Mutex<HashMap<String, Value>>,
    /// The terminals of this computer's chats: a hub of its own, with one machine and a ticket key made for this
    /// run. None on the home computer.
    hub: Option<Arc<Hub>>,
}

fn now_f64() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

/// A session id as the server takes one (server.py SID_RE).
pub fn valid_sid(s: &str) -> bool {
    (1..=64).contains(&s.len()) && s.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
}

/// The program a terminal here runs for the chat with background id `bg`: the helper script's term mode, which
/// attaches to it. The same command the hub link runs (link.rs `open_term`).
pub fn term_command(cfg: &NodeConfig, bg: &str) -> LocalCommand {
    LocalCommand {
        argv: vec![cfg.python(), cfg.sb(), "rpc".into()],
        env: vec![
            ("SSH_ORIGINAL_COMMAND".to_string(), format!("term {bg}")),
            ("TERM".to_string(), "xterm-256color".to_string()),
            ("COLORTERM".to_string(), "truecolor".to_string()),
        ],
        env_remove: vec![],
        cwd: Some(crate::home()),
    }
}

fn no(status: StatusCode, kind: &str, why: impl Into<String>) -> Response {
    let body = json!({"ok": false, "error": why.into(), "kind": kind});
    (status, [(header::CACHE_CONTROL, "no-store")], Json(body)).into_response()
}

fn yes(v: Value) -> Response {
    ([(header::CACHE_CONTROL, "no-store")], Json(v)).into_response()
}

/// A POST's body: JSON, said to be JSON, and an object.
fn object(headers: &HeaderMap, bytes: &[u8]) -> Result<Value, Box<Response>> {
    let json = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|c| c.starts_with("application/json"));
    match serde_json::from_slice::<Value>(bytes) {
        Ok(v @ Value::Object(_)) if json => Ok(v),
        _ => Err(Box::new(no(StatusCode::BAD_REQUEST, "bad_request", "The request is not a JSON object."))),
    }
}

impl Surface {
    /// `machine` is this computer's name at the hub (a computer that is not home has one from pairing).
    pub fn new(id: DeviceId, home: bool, config: PathBuf, machine: &str, lists: Arc<dyn Lists>) -> Arc<Surface> {
        let hub = (!home && sb_hub::ticket::valid_machine(machine)).then(|| {
            // (a folder that does not exist: this hub reads no file and writes none)
            let cfg = sb_hub::config::Config::defaults_in(&config.with_file_name("own-terminals"));
            let path = config.clone();
            let local = Local {
                machine: machine.to_string(),
                label: crate::hostname_label(),
                command: Box::new(move |bg| term_command(&crate::load_raw(&path).unwrap_or_default(), bg)),
            };
            Hub::local(cfg, DeviceKey::generate().secret_bytes().to_vec(), local)
        });
        Arc::new(Surface {
            id,
            home,
            config,
            lists,
            moves: std::sync::OnceLock::new(),
            board: tokio::sync::Mutex::new(None),
            pings: Mutex::default(),
            hub,
        })
    }

    fn cfg(&self) -> NodeConfig {
        crate::load_raw(&self.config).unwrap_or_default()
    }

    /// Who answers the `home/` routes here. Without it they refuse everybody.
    pub fn set_moves(&self, moves: Arc<dyn Moves>) {
        let _ = self.moves.set(moves);
    }

    /// What the home computer calls a device, for the log.
    pub fn name_of(&self, id: &DeviceId) -> String {
        if *id == self.id {
            return "this computer".into();
        }
        let held = self.lists.held();
        let name = held.as_ref().and_then(|r| r.device(id)).map(|d| d.name.trim()).filter(|n| !n.is_empty());
        name.map(str::to_string).unwrap_or_else(|| id.short())
    }

    /// What the home computer calls this computer.
    fn label(&self) -> String {
        let held = self.lists.held();
        let name = held.as_ref().and_then(|r| r.device(&self.id)).map(|d| d.name.trim()).filter(|n| !n.is_empty());
        name.map(str::to_string).unwrap_or_else(crate::hostname_label)
    }

    /// A hook's or the status line's ping about a chat here (the body of the server's `POST /api/event`): kept
    /// for the next board, the way the server keeps one over its last poll (server.py `ping`).
    pub fn ping(&self, body: &Value) {
        let (Some(sid), Some(why)) = (body["session_id"].as_str(), body["why"].as_str()) else { return };
        if !valid_sid(sid) || !PING_WHY.contains(&why) {
            return;
        }
        let now = now_f64();
        let mut o = serde_json::Map::new();
        match why {
            "prompt" => o.insert("status".into(), json!("busy")),
            "stop" => o.insert("status".into(), json!("idle")),
            "wait" if body["kind"] == "permission_prompt" => o.insert("status".into(), json!("waiting")),
            _ => None,
        };
        if matches!(why, "prompt" | "stop") || o.get("status") == Some(&json!("waiting")) {
            o.insert("why".into(), json!(why));
        }
        if why == "status" {
            if let Some(ctx) = body["ctx"].as_u64().filter(|c| (1..10_000_000).contains(c)) {
                o.insert("ctx".into(), json!(ctx));
            }
            if let (Some(exp), Some(ts)) = (body["cache_expires_at"].as_f64(), body["ts"].as_f64())
                && exp > 0.0
                && ts > 0.0
                && exp - ts > 0.0
                && exp - ts < 7200.0
            {
                o.insert("cold_at".into(), json!(((exp + (now - ts)) * 1000.0).round() / 1000.0));
            }
        }
        if o.is_empty() {
            return;
        }
        {
            let mut pings = self.pings.lock().unwrap_or_else(|e| e.into_inner());
            pings.retain(|_, p| now - p["at"].as_f64().unwrap_or(0.0) < PING_KEPT);
            if pings.len() >= 500 {
                return;
            }
            let mut kept = pings.get(sid).and_then(Value::as_object).cloned().unwrap_or_default();
            kept.extend(o);
            kept.insert("at".into(), json!(now));
            pings.insert(sid.to_string(), Value::Object(kept));
        }
        if why != "status" {
            // the chat changed: the next look asks the helper script again
            if let Ok(mut b) = self.board.try_lock() {
                *b = None;
            }
        }
    }

    /// This computer's row of the board: the helper script's `own` answer, at most `BOARD_FOR` old. Err: why it
    /// has none.
    pub async fn board(&self) -> Result<Value, String> {
        let mut slot = self.board.lock().await;
        if let Some((at, v)) = &*slot
            && at.elapsed() < BOARD_FOR
        {
            return Ok(v.clone());
        }
        let cfg = self.cfg();
        let pings = {
            let now = now_f64();
            let mut pings = self.pings.lock().unwrap_or_else(|e| e.into_inner());
            pings.retain(|_, p| now - p["at"].as_f64().unwrap_or(0.0) < PING_KEPT);
            Value::Object(pings.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
        };
        let mut v = crate::link::rpc(&cfg, &json!({"verb": "own", "pings": pings}), 25).await;
        if v["ok"] != true || !v["machine"].is_object() {
            return Err(v["error"].as_str().unwrap_or("the helper script gave no answer").to_string());
        }
        v["machine"]["name"] = json!(cfg.machine);
        v["machine"]["label"] = json!(self.label());
        *slot = Some((Instant::now(), v.clone()));
        Ok(v)
    }

    async fn forget_board(&self) {
        *self.board.lock().await = None;
    }

    /// One connection to this surface from this computer itself, with no network in between: the other end of
    /// the stream is served like any device's.
    pub fn open_local(self: &Arc<Self>) -> tokio::io::DuplexStream {
        let (near, far) = tokio::io::duplex(256 * 1024);
        let caller = Caller { id: self.id, name: "this computer".into() };
        tokio::spawn(serve_stream(router(self.clone()).layer(Extension(caller)), far));
        near
    }
}

async fn hello(State(s): State<Arc<Surface>>) -> Response {
    let cfg = s.cfg();
    yes(json!({
        "ok": true, "v": 1, "id": s.id.to_string(),
        "machine": Some(cfg.machine).filter(|m| !m.is_empty()),
        "name": s.label(), "version": crate::VERSION,
        "role": if s.home { "home" } else { "member" },
        "can_host": s.moves.get().is_some_and(|m| m.can_host()),
        "roster": s.lists.held().map(|r| r.stamp()),
    }))
}

async fn roster_get(State(s): State<Arc<Surface>>) -> Response {
    match s.lists.held() {
        Some(r) => yes(serde_json::to_value(&r.envelope).unwrap_or(Value::Null)),
        None => no(StatusCode::NOT_FOUND, "not_here", "This computer holds no list of devices yet."),
    }
}

async fn roster_post(State(s): State<Arc<Surface>>, headers: HeaderMap, bytes: axum::body::Bytes) -> Response {
    let v = match object(&headers, &bytes) {
        Ok(v) => v,
        Err(r) => return *r,
    };
    let Ok(envelope) = serde_json::from_value::<Envelope>(v) else {
        return no(StatusCode::BAD_REQUEST, "bad_request", "That is not a list of devices.");
    };
    match s.lists.offer(&envelope) {
        Ok(took) => yes(json!({"ok": true, "took": took, "have": s.lists.held().map(|r| r.stamp())})),
        Err(e) => no(StatusCode::BAD_REQUEST, "bad_request", format!("That list of devices was not taken: {e}.")),
    }
}

/// The one caller the `home/` routes answer: the home computer this one is paired with. Checked before anything
/// of the request is looked at.
fn from_home(s: &Surface, caller: &Caller) -> Result<Arc<dyn Moves>, Box<Response>> {
    match s.moves.get() {
        Some(m) if m.home() == Some(caller.id) => Ok(m.clone()),
        _ => {
            log(format!(
                "own: {} ({}) asked about moving the home computer and was refused",
                caller.name,
                caller.id.short()
            ));
            Err(Box::new(no(StatusCode::FORBIDDEN, "not_home", "Only this computer's home computer may ask that.")))
        }
    }
}

fn moved(r: Result<Value, MoveNo>) -> Response {
    match r {
        Ok(v) => yes(v),
        Err((status, kind, why)) => no(status, kind, why),
    }
}

async fn home_status(State(s): State<Arc<Surface>>, Extension(caller): Extension<Caller>) -> Response {
    match from_home(&s, &caller) {
        Ok(m) => moved(m.status().await),
        Err(r) => *r,
    }
}

async fn home_receive(
    State(s): State<Arc<Surface>>,
    Extension(caller): Extension<Caller>,
    axum::extract::Query(q): axum::extract::Query<HashMap<String, String>>,
    body: axum::body::Body,
) -> Response {
    let m = match from_home(&s, &caller) {
        Ok(m) => m,
        Err(r) => return *r,
    };
    let sha = q.get("sha256").filter(|h| h.len() == 64 && h.bytes().all(|c| c.is_ascii_hexdigit()));
    let Some(sha) = sha else {
        return no(StatusCode::BAD_REQUEST, "bad_request", "The request does not say which file it sends.");
    };
    moved(m.receive(sha.to_ascii_lowercase(), body).await)
}

async fn home_commit(
    State(s): State<Arc<Surface>>,
    Extension(caller): Extension<Caller>,
    headers: HeaderMap,
    bytes: axum::body::Bytes,
) -> Response {
    let m = match from_home(&s, &caller) {
        Ok(m) => m,
        Err(r) => return *r,
    };
    match object(&headers, &bytes) {
        Ok(v) => moved(m.commit(v).await),
        Err(r) => *r,
    }
}

async fn home_discard(State(s): State<Arc<Surface>>, Extension(caller): Extension<Caller>) -> Response {
    match from_home(&s, &caller) {
        Ok(m) => moved(m.discard().await),
        Err(r) => *r,
    }
}

async fn board(State(s): State<Arc<Surface>>) -> Response {
    match s.board().await {
        Ok(v) => yes(v),
        Err(e) => no(StatusCode::BAD_GATEWAY, "failed", format!("This computer could not list its chats ({e}).")),
    }
}

const GONE: [&str; 2] = ["stopped", "done"];

/// `POST /own/v1/term`: the rules of the server's `/api/term` (server/term.py `ticket`) for one computer.
async fn term_ticket(State(s): State<Arc<Surface>>, headers: HeaderMap, bytes: axum::body::Bytes) -> Response {
    let v = match object(&headers, &bytes) {
        Ok(v) => v,
        Err(r) => return *r,
    };
    let bad = |why: &str| no(StatusCode::BAD_REQUEST, "bad_request", why);
    let (mut bg, sid) = (None, v["sid"].as_str());
    if !v["bg"].is_null() {
        match v["bg"].as_str().filter(|b| sb_hub::ticket::valid_bg(b)) {
            Some(b) => bg = Some(b.to_string()),
            None => return bad("That is not a background id."),
        }
    } else if !sid.is_some_and(valid_sid) {
        return bad("That is not a session id.");
    }
    let Some(hub) = &s.hub else {
        return no(StatusCode::NOT_FOUND, "not_here", "This computer has no name at the home computer yet.");
    };
    let board = s.board().await;
    let machine = board.as_ref().map(|b| b["machine"].clone()).unwrap_or(Value::Null);
    let mut chats = ["sessions", "replaced"].iter().flat_map(|k| machine[*k].as_array().into_iter().flatten());
    let chat = chats
        .find(|c| match &bg {
            Some(b) => c["bg_id"].as_str() == Some(b),
            None => c["sid"].as_str() == sid,
        })
        .cloned();
    let (name, label) = (s.cfg().machine, s.label());
    let title = chat.as_ref().map(|c| c["title"].clone()).unwrap_or(Value::Null);
    let need = |need: &str, kind: &str, why: String| {
        yes(json!({"ok": false, "error": why, "need": need, "kind": kind, "machine": name, "label": label,
                   "title": title, "sid": chat.as_ref().and_then(|c| c["sid"].as_str()).or(sid)}))
    };
    if bg.is_none() {
        let c = match (&chat, &board) {
            (Some(c), _) => c,
            (None, Err(e)) => {
                return no(StatusCode::BAD_GATEWAY, "failed", format!("This computer could not list its chats ({e})."));
            }
            (None, Ok(_)) => return no(StatusCode::NOT_FOUND, "not_here", "That chat isn't on the list anymore."),
        };
        let (kind, status) = (c["kind"].as_str(), c["status"].as_str().unwrap_or(""));
        let gone = GONE.contains(&status);
        match (kind, c["bg_id"].as_str()) {
            (Some("background"), Some(b)) if !gone && sb_hub::ticket::valid_bg(b) => bg = Some(b.to_string()),
            (Some("interactive"), _) if !gone => {
                let why = format!("This chat is open in a terminal on {label}, so it can't be opened here.");
                return need("terminal", "busy", why);
            }
            _ => {
                return need("resume", "not_here", "This chat isn't running. Resume it to open its terminal.".into());
            }
        }
    }
    let bg = bg.unwrap_or_default();
    match hub.local_ticket(&bg) {
        Some(t) => yes(json!({"ok": true, "t": t, "bg": bg, "title": title, "machine": name, "label": label})),
        None => bad("That is not a background id."),
    }
}

async fn action(
    State(s): State<Arc<Surface>>,
    Extension(caller): Extension<Caller>,
    headers: HeaderMap,
    bytes: axum::body::Bytes,
) -> Response {
    let v = match object(&headers, &bytes) {
        Ok(v) => v,
        Err(r) => return *r,
    };
    // Checked here, before the helper script sees anything: a verb from the list, and an id that is one.
    let Some(verb) = v["verb"].as_str().filter(|v| VERBS.contains(v)) else {
        return no(StatusCode::BAD_REQUEST, "bad_request", "That is not something a chat here can be asked to do.");
    };
    let Some(sid) = v["sid"].as_str().filter(|s| valid_sid(s)) else {
        return no(StatusCode::BAD_REQUEST, "bad_request", "That is not a session id.");
    };
    log(format!("own: {} ({}) asked to {verb} the chat {sid}", caller.name, caller.id.short()));
    let r = crate::link::rpc(&s.cfg(), &json!({"verb": verb, "sid": sid}), 60).await;
    s.forget_board().await;
    yes(r)
}

/// The routes. The home computer has the first three only.
pub fn router(s: Arc<Surface>) -> Router {
    let mut r =
        Router::new().route("/own/v1/hello", get(hello)).route("/own/v1/roster", get(roster_get).post(roster_post));
    if !s.home {
        r = r
            .route("/own/v1/board", get(board))
            .route("/own/v1/term", post(term_ticket))
            .route("/own/v1/action", post(action))
            .route("/own/v1/home/status", get(home_status))
            .route("/own/v1/home/commit", post(home_commit))
            .route("/own/v1/home/discard", post(home_discard));
    }
    let mut r = r.layer(axum::extract::DefaultBodyLimit::max(BODY_MAX));
    if !s.home {
        // (after the limit above, which is for JSON: this one is a whole home computer's state, written to a
        // file as it arrives)
        r = r.route("/own/v1/home/receive", post(home_receive));
    }
    let mut r = r.with_state(s.clone());
    if let Some(hub) = &s.hub {
        // (GET only: the POST above is the ticket)
        r = r.merge(sb_hub::server::local_router(hub.clone(), "/own/v1/term"));
    }
    r.fallback(|| async { no(StatusCode::NOT_FOUND, "not_here", "This computer does not answer that.") })
}

/// Serves every stream the endpoint hands over, each as its caller.
pub async fn serve(surface: Arc<Surface>, mut streams: mpsc::Receiver<OwnIncoming>) {
    let app = router(surface.clone());
    while let Some(inc) = streams.recv().await {
        let caller = Caller { id: inc.caller, name: surface.name_of(&inc.caller) };
        tokio::spawn(serve_stream(app.clone().layer(Extension(caller)), inc.stream));
    }
}

/// Serves `router` on one stream until the other side is done with it. An upgrade (the terminal's WebSocket)
/// keeps the stream for as long as it lasts.
pub async fn serve_stream<S>(router: Router, stream: S)
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let service = hyper_util::service::TowerToHyperService::new(router);
    let conn = hyper::server::conn::http1::Builder::new()
        .serve_connection(hyper_util::rt::TokioIo::new(stream), service)
        .with_upgrades();
    let _ = conn.await;
}

#[cfg(test)]
mod spike {
    //! The question the plan left open (section 0, risk 5): does HTTP with a WebSocket upgrade run directly on a
    //! tunnel stream, or does the own surface need a loopback listener? It runs on the stream.
    use super::*;
    use axum::extract::ws::{Message as AxMessage, WebSocketUpgrade};
    use axum::routing::get;
    use futures_util::{SinkExt, StreamExt};
    use sb_link::endpoint::{Clocks, Gate, LinkEndpoint, Options, OwnIncoming, Peer, Tunnel};
    use sb_link::identity::DeviceKey;
    use sb_link::tunnel::{Target, Targets, write_header};
    use std::net::SocketAddr;
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::sync::mpsc;
    use tokio_tungstenite::tungstenite::Message;

    fn app() -> Router {
        Router::new().route("/hello", get(|| async { axum::Json(serde_json::json!({"ok": true})) })).route(
            "/echo",
            get(|ws: WebSocketUpgrade| async move {
                ws.on_upgrade(|mut socket| async move {
                    while let Some(Ok(m)) = socket.recv().await {
                        match m {
                            AxMessage::Text(t) => {
                                let _ = socket.send(AxMessage::Text(format!("echo {t}").into())).await;
                            }
                            AxMessage::Binary(b) => {
                                let _ = socket.send(AxMessage::Binary(b)).await;
                            }
                            AxMessage::Close(_) => break,
                            _ => {}
                        }
                    }
                })
            }),
        )
    }

    async fn get_over<S: AsyncRead + AsyncWrite + Unpin>(mut s: S, path: &str) -> String {
        let req = format!("GET {path} HTTP/1.1\r\nHost: own\r\nConnection: close\r\n\r\n");
        s.write_all(req.as_bytes()).await.unwrap();
        let mut out = Vec::new();
        let _ = tokio::time::timeout(Duration::from_secs(10), s.read_to_end(&mut out)).await.unwrap();
        String::from_utf8_lossy(&out).into_owned()
    }

    async fn echo_over<S: AsyncRead + AsyncWrite + Unpin>(s: S) {
        let (mut ws, resp) = tokio_tungstenite::client_async("ws://own/echo", s).await.unwrap();
        assert_eq!(resp.status(), 101);
        ws.send(Message::Text("one".into())).await.unwrap();
        assert_eq!(ws.next().await.unwrap().unwrap(), Message::Text("echo one".into()));
        let big = vec![7u8; 200_000];
        ws.send(Message::Binary(big.clone().into())).await.unwrap();
        assert_eq!(ws.next().await.unwrap().unwrap(), Message::Binary(big.into()));
        ws.close(None).await.unwrap();
    }

    #[tokio::test]
    async fn http_and_a_websocket_run_on_a_plain_stream() {
        let (near, far) = tokio::io::duplex(64 * 1024);
        tokio::spawn(serve_stream(app(), far));
        let text = get_over(near, "/hello").await;
        assert!(text.starts_with("HTTP/1.1 200") && text.ends_with("{\"ok\":true}"), "{text}");
        let (near, far) = tokio::io::duplex(64 * 1024);
        tokio::spawn(serve_stream(app(), far));
        echo_over(near).await;
    }

    struct Anyone;
    impl Gate for Anyone {
        fn allows(&self, _: &[u8; 32]) -> bool {
            true
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn http_and_a_websocket_run_on_a_tunnel_stream_between_two_endpoints() {
        let clocks = Clocks { dial: Duration::from_secs(5), ..Clocks::default() };
        let bind = Some(SocketAddr::from(([127, 0, 0, 1], 0)));
        let opts = Options { bind, tunnel: true, clocks: clocks.clone(), ..Options::default() };
        let member = LinkEndpoint::bind(&DeviceKey::generate(), opts, Arc::new(Anyone)).await.unwrap();
        let (pairs, _pairs_rx) = mpsc::channel(1);
        let (owns, mut owns_rx) = mpsc::channel::<OwnIncoming>(8);
        let serving = member.clone();
        tokio::spawn(async move { serving.serve_with(Targets::default(), pairs, Some(owns)).await });
        tokio::spawn(async move {
            while let Some(inc) = owns_rx.recv().await {
                tokio::spawn(serve_stream(app(), inc.stream));
            }
        });

        let caller_key = DeviceKey::generate();
        let opts = Options { bind, clocks, ..Options::default() };
        let caller = LinkEndpoint::bind(&caller_key, opts, Arc::new(sb_link::endpoint::Nobody)).await.unwrap();
        let tunnel = Tunnel::new(caller.clone(), Peer { id: member.id(), relay: None, addrs: member.addrs() });
        tokio::spawn(tunnel.clone().run());

        let mut s = tunnel.open().await.unwrap();
        write_header(&mut s, Target::Own).await.unwrap();
        let text = get_over(s, "/hello").await;
        assert!(text.starts_with("HTTP/1.1 200") && text.ends_with("{\"ok\":true}"), "{text}");

        let mut s = tunnel.open().await.unwrap();
        write_header(&mut s, Target::Own).await.unwrap();
        echo_over(s).await;

        // and the targets that lead to an address stay refused on a computer that names none
        let mut s = tunnel.open().await.unwrap();
        write_header(&mut s, Target::Api).await.unwrap();
        let text = get_over(s, "/hello").await;
        assert!(text.is_empty(), "{text}");
        tunnel.close();
        member.close().await;
        caller.close().await;
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use futures_util::StreamExt;
    use sb_link::http1;
    use sb_link::paired::Kind;
    use sb_link::roster::{self, Body, RosterCache, RosterDevice};
    use std::sync::atomic::{AtomicU64, Ordering};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio_tungstenite::tungstenite::Message;

    /// The helper script of the lab, run with /bin/sh in place of Python: it notes every request it is given,
    /// answers `own` with the board next to it, and as a terminal says what it was started for.
    const SB: &str = r#"
if [ -n "$SSH_ORIGINAL_COMMAND" ]; then echo "ATTACHED $SSH_ORIGINAL_COMMAND"; sleep 20; exit 0; fi
req=$(cat)
printf '%s\n' "$req" >> "$0.log"
case "$req" in
  *'"verb":"own"'*) cat "$0.board" ;;
  *) echo '{"ok":true,"did":"it"}' ;;
esac
"#;

    const BG: &str = "11111111-1111-4111-8111-111111111111";

    struct Held(RosterCache);

    impl Lists for Held {
        fn held(&self) -> Option<Arc<Roster>> {
            self.0.get()
        }
        fn offer(&self, envelope: &Envelope) -> Result<bool, RosterError> {
            self.0.offer(envelope, now_f64() as u64)
        }
    }

    struct Lab {
        dir: PathBuf,
        surface: Arc<Surface>,
        home: DeviceKey,
        caller: DeviceId,
    }

    fn body(lab_home: &DeviceKey, me: DeviceId, caller: DeviceId, seq: u64) -> Body {
        let device = |id, name: &str, kind, machine: Option<&str>| RosterDevice {
            id,
            name: name.into(),
            kind,
            machine: machine.map(str::to_string),
            ver: None,
            own: None,
            addrs: vec![],
        };
        Body {
            home: lab_home.id(),
            home_name: "Home".into(),
            epoch: 1,
            seq,
            at: now_f64() as u64,
            relay: None,
            devices: vec![
                device(lab_home.id(), "Home", Kind::Computer, Some("home")),
                device(me, "Laptop", Kind::Computer, Some("laptop")),
                device(caller, "Phone", Kind::Phone, None),
            ],
        }
    }

    fn lab(tag: &str, home: bool) -> Lab {
        static N: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "sbnode-own-{tag}-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let sb = dir.join("sb.sh");
        std::fs::write(&sb, SB).unwrap();
        let board = json!({"ok": true, "machine": {"name": "?", "label": "?", "online": true, "replaced": [], "sessions": [
            {"sid": BG, "title": "In the background", "kind": "background", "status": "idle", "bg_id": "c03ef0d9"},
            {"sid": "term-chat", "title": "Open in a terminal", "kind": "interactive", "status": "busy"},
            {"sid": "old-chat", "title": "Long over", "kind": "background", "status": "stopped", "bg_id": "aaaaaaa1"},
        ]}});
        std::fs::write(dir.join("sb.sh.board"), board.to_string()).unwrap();
        let cfg = NodeConfig {
            hub: "127.0.0.1:9".into(),
            machine: "laptop".into(),
            token: "t".into(),
            sb: Some(sb.display().to_string()),
            python: Some("/bin/sh".into()),
            ..Default::default()
        };
        crate::save_to(&dir.join("node.json"), &cfg).unwrap();
        let (home_key, me, caller) = (DeviceKey::generate(), DeviceKey::generate().id(), DeviceKey::generate().id());
        let me = if home { home_key.id() } else { me };
        let cache = RosterCache::load(&dir.join("roster.json"), home_key.id());
        let first = roster::sign(&home_key, body(&home_key, me, caller, 5)).unwrap();
        assert_eq!(cache.offer(&first.envelope, now_f64() as u64), Ok(true));
        let surface = Surface::new(me, home, dir.join("node.json"), "laptop", Arc::new(Held(cache)));
        Lab { dir, surface, home: home_key, caller }
    }

    impl Lab {
        fn stream(&self) -> tokio::io::DuplexStream {
            let (near, far) = tokio::io::duplex(256 * 1024);
            let caller = Caller { id: self.caller, name: "Phone".into() };
            tokio::spawn(serve_stream(router(self.surface.clone()).layer(Extension(caller)), far));
            near
        }

        async fn ask(&self, method: &str, path: &str, body: Option<Value>) -> (u16, Value) {
            let mut s = self.stream();
            let bytes = body.map(|b| b.to_string().into_bytes());
            let a = http1::ask(&mut s, method, path, bytes.as_deref(), 1 << 20).await.unwrap();
            (a.status, a.json().unwrap_or(Value::Null))
        }

        /// What a request written byte for byte is answered with.
        async fn raw(&self, request: &[u8]) -> String {
            let mut s = self.stream();
            s.write_all(request).await.unwrap();
            let mut out = Vec::new();
            let _ = tokio::time::timeout(Duration::from_secs(10), s.read_to_end(&mut out)).await.unwrap();
            String::from_utf8_lossy(&out).into_owned()
        }

        /// Every request the helper script was given.
        fn seen(&self) -> Vec<Value> {
            let text = std::fs::read_to_string(self.dir.join("sb.sh.log")).unwrap_or_default();
            text.lines().filter(|l| !l.trim().is_empty()).map(|l| serde_json::from_str(l).unwrap()).collect()
        }

        fn runs(&self, verb: &str) -> usize {
            self.seen().iter().filter(|r| r["verb"] == verb).count()
        }

        fn seq(&self) -> u64 {
            self.surface.lists.held().map_or(0, |r| r.body.seq)
        }
    }

    #[tokio::test]
    async fn the_board_is_this_computers_row_and_one_run_of_the_script_stands_for_three_seconds() {
        let lab = lab("board", false);
        let (status, v) = lab.ask("GET", "/own/v1/hello", None).await;
        assert_eq!(status, 200);
        assert_eq!(
            (v["role"].as_str(), v["machine"].as_str(), v["name"].as_str()),
            (Some("member"), Some("laptop"), Some("Laptop"))
        );
        assert_eq!(
            (v["v"].as_u64(), v["roster"]["seq"].as_u64(), v["can_host"].as_bool()),
            (Some(1), Some(5), Some(false))
        );
        assert_eq!(v["version"], crate::VERSION);
        assert_eq!(lab.runs("own"), 0, "saying hello runs nothing");

        let (status, v) = lab.ask("GET", "/own/v1/board", None).await;
        assert_eq!(status, 200, "{v}");
        let m = &v["machine"];
        assert_eq!(
            (m["name"].as_str(), m["label"].as_str()),
            (Some("laptop"), Some("Laptop")),
            "named as home names it"
        );
        assert_eq!(m["sessions"].as_array().unwrap().len(), 3);
        let (a, b) = tokio::join!(lab.ask("GET", "/own/v1/board", None), lab.ask("GET", "/own/v1/board", None));
        assert_eq!((a.0, &a.1), (200, &v));
        assert_eq!(b.1, v);
        assert_eq!(lab.runs("own"), 1, "three looks, one run");
        assert_eq!(lab.seen()[0], json!({"verb": "own", "pings": {}}));

        // a hook says a chat stopped: the next look asks the script again, and tells it
        lab.surface.ping(&json!({"session_id": "term-chat", "why": "stop"}));
        lab.surface.ping(&json!({"session_id": "no good; id", "why": "stop"}));
        lab.surface.ping(&json!({"session_id": "term-chat", "why": "made up"}));
        let (status, _) = lab.ask("GET", "/own/v1/board", None).await;
        assert_eq!((status, lab.runs("own")), (200, 2));
        let pings = lab.seen()[1]["pings"].clone();
        assert_eq!(pings.as_object().unwrap().len(), 1, "{pings}");
        assert_eq!(
            (pings["term-chat"]["status"].as_str(), pings["term-chat"]["why"].as_str()),
            (Some("idle"), Some("stop"))
        );
        // the status line's word changes nothing a look would show before its time
        lab.surface.ping(&json!({"session_id": "term-chat", "why": "status", "ctx": 1234}));
        lab.ask("GET", "/own/v1/board", None).await;
        assert_eq!(lab.runs("own"), 2);

        // a script that does not answer is said so, and nothing here is anything else
        std::fs::write(lab.dir.join("sb.sh.board"), "no json here").unwrap();
        lab.surface.forget_board().await;
        let (status, v) = lab.ask("GET", "/own/v1/board", None).await;
        assert_eq!((status, v["ok"].as_bool(), v["kind"].as_str()), (502, Some(false), Some("failed")), "{v}");
        for path in ["/own/v1/nothing", "/api/sessions", "/own/v1/board/x", "/"] {
            let (status, v) = lab.ask("GET", path, None).await;
            assert_eq!((status, v["kind"].as_str()), (404, Some("not_here")), "{path}");
        }
        let (status, _) = lab.ask("POST", "/own/v1/board", Some(json!({}))).await;
        assert_eq!(status, 405);
    }

    #[tokio::test]
    async fn a_chat_is_asked_only_what_is_on_the_list_and_only_by_an_id_that_is_one() {
        let lab = lab("action", false);
        let long = "a".repeat(65);
        let refused = [
            json!({"verb": "delete", "sid": "term-chat"}),
            json!({"verb": "send", "sid": "term-chat", "text": "hello"}),
            json!({"verb": "own", "sid": "term-chat"}),
            json!({"verb": "Stop", "sid": "term-chat"}),
            json!({"verb": ["stop"], "sid": "term-chat"}),
            json!({"sid": "term-chat"}),
            json!({"verb": "stop"}),
            json!({"verb": "stop", "sid": ""}),
            json!({"verb": "stop", "sid": "x; rm -rf ~"}),
            json!({"verb": "stop", "sid": "../../etc/passwd"}),
            json!({"verb": "stop", "sid": "a b"}),
            json!({"verb": "stop", "sid": long}),
            json!({"verb": "stop", "sid": 7}),
            json!(["stop", "term-chat"]),
        ];
        for body in refused {
            let (status, v) = lab.ask("POST", "/own/v1/action", Some(body.clone())).await;
            assert_eq!(
                (status, v["ok"].as_bool(), v["kind"].as_str()),
                (400, Some(false), Some("bad_request")),
                "{body}"
            );
        }
        // a body that is not said to be JSON, and one that is not JSON
        let post = |kind: &str, body: &str| {
            format!(
                "POST /own/v1/action HTTP/1.1\r\nHost: own\r\nConnection: close\r\nContent-Type: {kind}\r\n\
                 Content-Length: {}\r\n\r\n{body}",
                body.len()
            )
        };
        let good = r#"{"verb":"stop","sid":"term-chat"}"#;
        for request in [
            post("text/plain", good),
            post("application/x-www-form-urlencoded", good),
            post("application/json", "verb=stop"),
        ] {
            let text = lab.raw(request.as_bytes()).await;
            assert!(text.starts_with("HTTP/1.1 400") && text.contains("bad_request"), "{text}");
        }
        assert!(lab.seen().is_empty(), "the helper script saw none of it: {:?}", lab.seen());

        for verb in VERBS {
            // (what else the request carries stays here)
            let body = json!({"verb": verb, "sid": "term-chat", "cmd": "rm -rf ~", "machine": "home"});
            let (status, v) = lab.ask("POST", "/own/v1/action", Some(body)).await;
            assert_eq!((status, &v), (200, &json!({"ok": true, "did": "it"})));
            assert_eq!(lab.seen().last().unwrap(), &json!({"verb": verb, "sid": "term-chat"}));
        }
        assert_eq!(lab.seen().len(), 4);
        // ... and the next look at the board is a fresh one
        lab.ask("GET", "/own/v1/board", None).await;
        lab.ask("POST", "/own/v1/action", Some(json!({"verb": "stop", "sid": BG}))).await;
        lab.ask("GET", "/own/v1/board", None).await;
        assert_eq!(lab.runs("own"), 2);
    }

    #[tokio::test]
    async fn the_newer_list_of_devices_stands_whoever_shows_which() {
        let lab = lab("roster", false);
        let me = lab.surface.id;
        let (status, v) = lab.ask("GET", "/own/v1/roster", None).await;
        assert_eq!(status, 200);
        let had: Envelope = serde_json::from_value(v.clone()).unwrap();
        assert_eq!(roster::verify(&had, &lab.home.id(), None).unwrap().body.seq, 5);

        let newer = roster::sign(&lab.home, body(&lab.home, me, lab.caller, 6)).unwrap();
        let (status, a) = lab.ask("POST", "/own/v1/roster", Some(serde_json::to_value(&newer.envelope).unwrap())).await;
        assert_eq!((status, a["took"].as_bool(), a["have"]["seq"].as_u64()), (200, Some(true), Some(6)), "{a}");
        // the older one again: kept is the newer, and the answer says which
        let (status, a) = lab.ask("POST", "/own/v1/roster", Some(v)).await;
        assert_eq!((status, a["took"].as_bool(), a["have"]["seq"].as_u64()), (200, Some(false), Some(6)), "{a}");
        // one that somebody else signed, with a higher number
        let stranger = DeviceKey::generate();
        let forged = roster::sign(&stranger, Body { seq: 99, ..body(&lab.home, me, lab.caller, 99) }).unwrap();
        let (status, a) =
            lab.ask("POST", "/own/v1/roster", Some(serde_json::to_value(&forged.envelope).unwrap())).await;
        assert_eq!((status, a["kind"].as_str()), (400, Some("bad_request")), "{a}");
        let (status, a) = lab.ask("POST", "/own/v1/roster", Some(json!({"v": 1}))).await;
        assert_eq!((status, a["kind"].as_str()), (400, Some("bad_request")), "{a}");
        // one too large to be one
        let (status, _) = lab.ask("POST", "/own/v1/roster", Some(json!({"pad": "p".repeat(200_000)}))).await;
        assert_eq!(status, 413);
        assert_eq!(lab.seq(), 6);
        let (_, v) = lab.ask("GET", "/own/v1/roster", None).await;
        assert_eq!(serde_json::from_value::<Envelope>(v).unwrap(), newer.envelope);
        assert!(lab.seen().is_empty());
    }

    #[tokio::test]
    async fn the_home_computer_says_who_it_is_and_shows_its_list_and_nothing_else() {
        let lab = lab("home", true);
        let (status, v) = lab.ask("GET", "/own/v1/hello", None).await;
        assert_eq!((status, v["role"].as_str(), v["roster"]["seq"].as_u64()), (200, Some("home"), Some(5)));
        assert_eq!(lab.ask("GET", "/own/v1/roster", None).await.0, 200);
        for (method, path, body) in [
            ("GET", "/own/v1/board", None),
            ("POST", "/own/v1/term", Some(json!({"sid": BG}))),
            ("GET", "/own/v1/term?t=x", None),
            ("POST", "/own/v1/action", Some(json!({"verb": "stop", "sid": BG}))),
        ] {
            let (status, v) = lab.ask(method, path, body).await;
            assert_eq!((status, v["kind"].as_str()), (404, Some("not_here")), "{path}");
        }
        assert!(lab.seen().is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_terminal_is_opened_with_a_ticket_and_only_on_a_chat_that_runs_in_the_background() {
        let lab = lab("term", false);
        for body in [
            json!({}),
            json!({"sid": "x y"}),
            json!({"bg": "not hex"}),
            json!({"bg": "c03ef0d9; ls"}),
            json!({"bg": 7}),
        ] {
            let (status, v) = lab.ask("POST", "/own/v1/term", Some(body.clone())).await;
            assert_eq!((status, v["kind"].as_str()), (400, Some("bad_request")), "{body}");
        }
        assert!(lab.seen().is_empty());
        let (status, v) = lab.ask("POST", "/own/v1/term", Some(json!({"sid": "term-chat"}))).await;
        assert_eq!(
            (status, v["ok"].as_bool(), v["need"].as_str(), v["kind"].as_str()),
            (200, Some(false), Some("terminal"), Some("busy"))
        );
        assert_eq!(v["error"], "This chat is open in a terminal on Laptop, so it can't be opened here.");
        assert_eq!(
            (v["machine"].as_str(), v["title"].as_str(), v["sid"].as_str()),
            (Some("laptop"), Some("Open in a terminal"), Some("term-chat"))
        );
        let (status, v) = lab.ask("POST", "/own/v1/term", Some(json!({"sid": "old-chat"}))).await;
        assert_eq!((status, v["need"].as_str(), v["kind"].as_str()), (200, Some("resume"), Some("not_here")));
        assert_eq!(v["error"], "This chat isn't running. Resume it to open its terminal.");
        let (status, v) = lab.ask("POST", "/own/v1/term", Some(json!({"sid": "nobody-knows"}))).await;
        assert_eq!((status, v["kind"].as_str()), (404, Some("not_here")));
        assert_eq!(v["error"], "That chat isn't on the list anymore.");

        let (status, v) = lab.ask("POST", "/own/v1/term", Some(json!({"sid": BG}))).await;
        assert_eq!(
            (status, v["ok"].as_bool(), v["bg"].as_str(), v["label"].as_str()),
            (200, Some(true), Some("c03ef0d9"), Some("Laptop")),
            "{v}"
        );
        let t = v["t"].as_str().unwrap().to_string();
        // a ticket that is none opens nothing
        let text = lab.raw(b"GET /own/v1/term?t=made-up HTTP/1.1\r\nHost: own\r\nConnection: close\r\n\r\n").await;
        assert!(!text.starts_with("HTTP/1.1 101") && !text.starts_with("HTTP/1.1 200"), "{text}");

        let url = format!("ws://own/own/v1/term?t={t}&cols=80&rows=24&proto=2");
        let (mut ws, resp) = tokio_tungstenite::client_async(url.as_str(), lab.stream()).await.unwrap();
        assert_eq!(resp.status(), 101);
        let mut out = String::new();
        let end = Instant::now() + Duration::from_secs(15);
        while !out.contains("ATTACHED term c03ef0d9") {
            let left = end.checked_duration_since(Instant::now()).expect("the terminal never said what it runs");
            match tokio::time::timeout(left, ws.next()).await.expect("the terminal never said what it runs") {
                Some(Ok(Message::Binary(b))) => out.push_str(&String::from_utf8_lossy(&b)),
                Some(Ok(_)) => {}
                other => panic!("the terminal ended: {other:?} after {out:?}"),
            }
        }
        // the ticket was good once
        if let Ok((mut again, _)) = tokio_tungstenite::client_async(url.as_str(), lab.stream()).await {
            // (the hub takes the socket to say why, and closes it)
            let closed = async {
                while let Some(Ok(m)) = again.next().await {
                    assert!(!matches!(m, Message::Binary(_)), "a used ticket opened a terminal");
                }
            };
            tokio::time::timeout(Duration::from_secs(10), closed).await.expect("a used ticket was left open");
        }
        let _ = ws.close(None).await;
    }
}
