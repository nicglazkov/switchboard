//! The hub's routes for linked machines (spec section 6).
//!
//! GET  /node                      a node's link (WebSocket, `Authorization: Bearer <token>`, no Origin)
//! POST /pair                      {"code", "name", "label", "os", "token"?} -> {"ok", "machine", "label", "token"}
//! POST /internal/pair             -> {"ok", "code", "expires"}        (the server's Add a machine)
//! GET  /internal/nodes            -> {"ok", "nodes": [{name, label, os, online, since, version, paired}]}
//! POST /internal/rpc/<machine>    body: an sb.py RPC request -> its answer   (?timeout=seconds, default 40)
//! POST /internal/unpair/<machine> forget a node; its token stops working
//!
//! `/internal/*` answers only this machine, like `/internal/open`, and is not there at all on the tunnel listener.

use crate::link::{Link, RpcError};
use crate::link_proto::CLOSE_UNPAIRED;
use crate::machines;
use crate::server::{Hub, log, not_local, unix_now};
use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade};
use axum::extract::{ConnectInfo, Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures_util::{Sink, SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::{Value, json};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// The hub pings each link this often, and drops one it has not heard from in three times as long.
const PING_EVERY: Duration = Duration::from_secs(20);
const RPC_TIMEOUT: u64 = 40;
/// How the server's machine list writes a linked machine's target (`server/nodes.py`, `PREFIX`).
const NODE_TARGET: &str = "node:";
const RPC_TIMEOUT_MAX: u64 = 300;

/// What another computer may ask: its link, and pairing with a code. The tunnel listener has these and no more
/// of this file (server.rs `public_router`).
pub fn public_routes() -> Router<Arc<Hub>> {
    Router::new().route("/node", get(node)).route("/pair", post(pair))
}

pub fn routes() -> Router<Arc<Hub>> {
    public_routes()
        .route("/internal/pair", post(internal_pair))
        .route("/internal/nodes", get(internal_nodes))
        .route("/internal/rpc/{machine}", post(internal_rpc))
        .route("/internal/unpair/{machine}", post(internal_unpair))
}

fn err(status: u16, msg: impl Into<String>) -> Response {
    let code = StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    (code, [(header::CACHE_CONTROL, "no-store")], Json(json!({"ok": false, "error": msg.into()}))).into_response()
}

fn ok(v: Value) -> Response {
    ([(header::CACHE_CONTROL, "no-store")], Json(v)).into_response()
}

// ---------------------------------------------------------------- pairing

async fn internal_pair(State(hub): State<Arc<Hub>>, ConnectInfo(peer): ConnectInfo<SocketAddr>) -> Response {
    if let Some(r) = not_local(&peer) {
        return r;
    }
    let (code, expires) = hub.nodes.mint(unix_now());
    log("pairing code made");
    ok(json!({"ok": true, "code": code, "expires": expires}))
}

#[derive(Deserialize)]
struct PairReq {
    code: String,
    name: String,
    #[serde(default)]
    label: String,
    #[serde(default)]
    os: String,
    /// The node's current token, when a machine that is already linked pairs again.
    #[serde(default)]
    token: String,
}

async fn pair(State(hub): State<Arc<Hub>>, ConnectInfo(peer): ConnectInfo<SocketAddr>, body: String) -> Response {
    let Ok(req) = serde_json::from_str::<PairReq>(&body) else {
        return err(400, "Send {\"code\", \"name\"} as JSON.");
    };
    // A name an ssh machine already has stays that machine's. The server's list also holds every linked machine
    // (target `node:<name>`), and keeps one that was unlinked so that it has its place when it is linked again:
    // those rows are not owners of the name. Whether a linked machine may be paired again is decided just below.
    let taken = machines::load(&hub.cfg.machines_path)
        .ok()
        .and_then(|f| f.machines.into_iter().find(|m| m.name == req.name && !m.target.starts_with(NODE_TARGET)))
        .map(|m| m.label().to_string())
        // ... and so does a linked machine's, unless the one asking holds its token (never a hijack with a stray code)
        .or_else(|| {
            hub.nodes
                .get(&req.name)
                .filter(|n| hub.nodes.find(&req.token).is_none_or(|f| f.name != n.name))
                .map(|n| n.label().to_string())
        });
    let label = if req.label.trim().is_empty() { req.name.clone() } else { req.label.clone() };
    match hub.nodes.pair_from(peer.ip(), &req.code, &req.name, &label, &req.os, taken.as_deref(), unix_now()) {
        Ok((node, token)) => {
            log(format!("node {} paired ({})", node.name, node.os));
            // a link still open with the old token belongs to a machine that was just re-paired
            if let Some(old) = hub.links.get(&node.name) {
                old.close(CLOSE_UNPAIRED, "This machine was paired again; the old token no longer works.");
            }
            ok(json!({"ok": true, "machine": node.name, "label": node.label(), "token": token}))
        }
        Err(e) => {
            log(format!("pair refused: {e:?}"));
            err(e.status(), e.message())
        }
    }
}

async fn internal_unpair(
    State(hub): State<Arc<Hub>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Path(machine): Path<String>,
) -> Response {
    if let Some(r) = not_local(&peer) {
        return r;
    }
    match hub.nodes.remove(&machine) {
        Ok(true) => {
            if let Some(l) = hub.links.get(&machine) {
                l.close(CLOSE_UNPAIRED, "This machine was removed from Switchboard.");
            }
            log(format!("node {machine} unpaired"));
            ok(json!({"ok": true}))
        }
        Ok(false) => err(404, format!("No linked machine is named {machine}.")),
        Err(e) => err(500, format!("The hub could not save the change ({e}).")),
    }
}

async fn internal_nodes(State(hub): State<Arc<Hub>>, ConnectInfo(peer): ConnectInfo<SocketAddr>) -> Response {
    if let Some(r) = not_local(&peer) {
        return r;
    }
    let nodes: Vec<Value> = hub
        .nodes
        .list()
        .into_iter()
        .map(|n| {
            let link = hub.links.get(&n.name);
            let info = link.as_ref().map(|l| l.info()).unwrap_or_default();
            let os = if info.os.is_empty() { n.os.clone() } else { info.os };
            json!({"name": n.name, "label": n.label(), "os": os, "online": link.is_some(), "since": link.as_ref().map(|l| l.since),
                   "version": link.as_ref().map(|_| info.version), "terminals": link.as_ref().map(|l| l.terminals()).unwrap_or(0),
                   "paired": n.paired})
        })
        .collect();
    ok(json!({"ok": true, "nodes": nodes}))
}

// ---------------------------------------------------------------- the server's RPCs

#[derive(Deserialize, Default)]
struct RpcQuery {
    timeout: Option<u64>,
}

async fn internal_rpc(
    State(hub): State<Arc<Hub>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Path(machine): Path<String>,
    Query(q): Query<RpcQuery>,
    body: String,
) -> Response {
    if let Some(r) = not_local(&peer) {
        return r;
    }
    let Some(node) = hub.nodes.get(&machine) else {
        return err(404, format!("No linked machine is named {machine}."));
    };
    let Ok(req) = serde_json::from_str::<Value>(&body) else {
        return err(400, "The request is not JSON.");
    };
    let Some(link) = hub.links.get(&machine) else {
        return err(503, format!("{} is not linked to the hub right now.", node.label()));
    };
    let secs = q.timeout.unwrap_or(RPC_TIMEOUT).clamp(1, RPC_TIMEOUT_MAX);
    match link.rpc(req, Duration::from_secs(secs)).await {
        Ok(v) => ok(v),
        Err(RpcError::Gone) => err(503, format!("The link to {} dropped before it answered.", node.label())),
        Err(RpcError::Timeout) => err(504, format!("{} did not answer in {secs} s.", node.label())),
    }
}

// ---------------------------------------------------------------- the link itself

fn bearer(headers: &HeaderMap) -> &str {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::trim)
        .unwrap_or("")
}

async fn node(ws: WebSocketUpgrade, State(hub): State<Arc<Hub>>, headers: HeaderMap) -> Response {
    // nodes are programs, never pages: a browser always sends Origin
    if headers.contains_key(header::ORIGIN) {
        return err(403, "Pages cannot link machines.");
    }
    let Some(n) = hub.nodes.find(bearer(&headers)) else {
        if hub.nodes.unread() {
            // not a 401: a node told its token is unknown stops trying until it is paired again
            return err(503, "The hub cannot read its list of linked machines right now.");
        }
        return err(401, "This machine is not linked to the hub. Pair it again with a new code.");
    };
    ws.max_message_size(4 << 20).max_frame_size(4 << 20).on_upgrade(move |socket| serve_link(hub, socket, n.name))
}

/// Sends one message on a link. False if the node left, or took nothing for `patience`. A large message (a piece
/// of a file) gets a second more for each 32 KB, so a slow link that is still moving is not dropped.
async fn send(ws: &mut (impl Sink<Message> + Unpin), m: Message, patience: Duration) -> bool {
    let size = match &m {
        Message::Text(t) => t.len(),
        Message::Binary(b) => b.len(),
        _ => 0,
    };
    let patience = patience + Duration::from_secs((size / (32 << 10)) as u64);
    matches!(tokio::time::timeout(patience, ws.send(m)).await, Ok(Ok(())))
}

async fn serve_link(hub: Arc<Hub>, ws: WebSocket, name: String) {
    let patience = hub.cfg.link_patience;
    let (tx, mut rx) = tokio::sync::mpsc::channel(crate::link::LINK_BACKLOG);
    let link = Arc::new(Link::new(&name, hub.links.next_era(), unix_now(), tx));
    hub.links.insert(link.clone());
    log(format!("node {name} linked ({} linked)", hub.links.len()));
    // Writing has a task of its own, as on the node. A send to a node that is slow to take it can wait for as long
    // as `patience`, and done in the loop below it held up everything else on the link for that long: what the
    // node was saying went unread (output, answers, a terminal's exit), and so did the check that it still says
    // anything.
    let (mut sink, mut stream) = ws.split();
    // Pings and the closing message, which go ahead of whatever is waiting in `rx`.
    let (first_tx, mut first_rx) = tokio::sync::mpsc::channel::<Message>(2);
    let mut writer = tokio::spawn(async move {
        loop {
            let m = tokio::select! {
                biased;
                Some(m) = first_rx.recv() => m,
                m = rx.recv() => match m {
                    Some(m) => m,
                    None => return "closed",
                },
            };
            let last = matches!(m, Message::Close(_));
            if !send(&mut sink, m, patience).await {
                return "send failed";
            }
            if last {
                return "closed by the hub";
            }
        }
    });
    let mut ping = tokio::time::interval(PING_EVERY);
    ping.tick().await;
    let mut heard = Instant::now();
    let why = loop {
        tokio::select! {
            end = &mut writer => break end.unwrap_or("send failed"),
            m = stream.next() => {
                heard = Instant::now();
                match m {
                    Some(Ok(Message::Text(t))) => link.on_text(t.as_str()),
                    Some(Ok(Message::Binary(b))) => link.on_binary(&b),
                    Some(Ok(Message::Ping(_) | Message::Pong(_))) => {}
                    Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break "node left",
                }
            }
            _ = ping.tick() => {
                if heard.elapsed() > PING_EVERY * 3 {
                    break "went quiet";
                }
                // (no room: the last ping has not gone out yet, and a second one behind it would add nothing)
                let _ = first_tx.try_send(Message::Ping(Vec::new().into()));
            }
            _ = link.closing.notified() => {
                let (code, reason) = link.close_reason();
                let close = Message::Close(Some(CloseFrame { code, reason: reason.into() }));
                // The writer may be in the middle of a send: it gets as long as any send does to finish that one
                // and this one.
                let sent = async {
                    let _ = first_tx.send(close).await;
                    let _ = (&mut writer).await;
                };
                let _ = tokio::time::timeout(patience * 2 + Duration::from_secs(1), sent).await;
                // Wait a moment for the node's answering close before dropping the socket. Dropping it at once can
                // reset the connection on Windows, and the node then never reads why it was closed.
                let _ = tokio::time::timeout(Duration::from_secs(2), async {
                    while let Some(Ok(m)) = stream.next().await {
                        if matches!(m, Message::Close(_)) {
                            break;
                        }
                    }
                })
                .await;
                break "closed by the hub";
            }
        }
    };
    writer.abort();
    hub.links.remove(&link);
    link.shut();
    log(format!("node {name} unlinked: {why}"));
}
