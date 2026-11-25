//! The control API: how the server and the command line on this computer talk to keys in the running helper.
//!
//! It listens on 127.0.0.1:8137 whenever the helper runs, keys on or off (while they are off it is the only thing
//! of keys that exists: `/status` says so with 503, and `/pair/join` is how this computer joins a home computer).
//! It answers this computer alone, guarded the way the
//! hub guards its own internal routes: the connection must come from this computer, the Host must name this
//! computer (so a web page cannot reach it through a name that resolves here), there must be no Origin or
//! Sec-Fetch-Site (no browser), and a POST must be JSON (which a web page's plain form cannot send).
//!
//!   GET  /status                       what keys are doing here (sb-link control.rs `Status`)
//!   POST /pair/new {}                  home: {"ok", "code", "link", "expires"} for one new device
//!   POST /pair/join {"input", "name"?, "addrs"?, "replace"?}   any other computer: pair with a home computer
//!                                      ("replace": true moves a computer that is paired with another home)
//!   GET  /devices                      home: {"ok", "devices": [...]}
//!   POST /devices/remove {"id"}        home: forget a device and cut it off
//!   (GET /devices also carries "ver" and "answers_away" on a computer's row once it has been sent the list of
//!   devices, and "roster": {epoch, seq, at, expires} | null beside "devices")
//!   POST /relay {"url"}                set the relay ("" or null: none)
//!   GET  /roster                       the home computer's list of devices as this computer holds it:
//!                                      {"ok", "roster": {epoch, seq, at, expires} | null, "devices": [{id, name,
//!                                      kind, machine, ver, home, this, blocked}]}
//!   POST /devices/block {"id"}         any other computer: stop answering a device on that list, here only,
//!                                      until the list changes
//!   POST /devices/unblock {"id"}       ... and answer it again
//!
//!   GET  /home/status                  {"ok", "role": home | member | off, "can_host", "epoch", "stood_down":
//!                                      {why, at, line} | null, "move": {state, to, started, retire} | null,
//!                                      "arriving", "backup": {"code_made"}}
//!   POST /home/move {"to": <id>, "retire"?} | {"resume": true} | {"undo": true}   home: move the home computer
//!                                      to a paired computer; answers when it is done: {"ok", "state": done |
//!                                      undone, "home": {id, name}, "retired", "gap_ms", "epoch"}
//!   POST /home/backup {"to"?: <file>}  home: {"ok", "file", "bytes", "epoch", "code": <recovery code> | null}.
//!                                      The code is there only in the answer to the backup that made it.
//!   POST /home/restore {"file", "code"}   a computer that is not home: become the home computer in that backup.
//!                                      {"ok", "home", "epoch", "made", "version", "devices": [{name, kind,
//!                                      machine}]}
//!
//! A refusal is `{"ok": false, "error": "...", "kind": "..."}` with a status that is not 200. "error" is a sentence
//! for the person and may change; "kind" is one word for the program that asked, where there is one to give:
//!   /status     off, not_running
//!   /pair/new   off, not_running, not_home, moving (the home computer is being moved)
//!   /home/move  busy, unfinished, nothing, not_home, not_found, unreachable, version, refused, no_parts, held,
//!               save, failed, key, endpoint, no_answer, committed, unsure, stood_down
//!   /home/backup   not_home, busy, no_parts, key, failed, save, held
//!   /home/restore  bad_code, busy, no_parts, not_backup, newer, wrong_code, damaged, failed, is_home,
//!               other_home, held, key_in_use, save
//!   /pair/join  bad_input (400), is_home, key_in_use, key, no_relay, link_no_relay, endpoint, local_network,
//!               unreachable, wrong_code, expired (also used already, or stopped after too many wrong tries),
//!               busy, refused, wrong_home, version, protocol, timeout, connection_lost, save, other_home (this
//!               computer is paired with another home computer, and "replace" was not sent: nothing changed)
//!
//! The second half of this file is the command line's side: each verb asks the running helper, and where that
//! makes sense does the work itself when no helper is running with keys on.

use crate::keys::home::MoveAsk;
use crate::keys::{Refusal, Shared};
use axum::extract::{ConnectInfo, State};
use axum::http::{HeaderMap, Method, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use sb_link::control::{DeviceRow, PairNew, Status};
use sb_link::identity::DeviceId;
use serde_json::{Value, json};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

fn refuse(status: StatusCode, why: impl Into<String>) -> Response {
    (status, [(header::CACHE_CONTROL, "no-store")], Json(json!({"ok": false, "error": why.into()}))).into_response()
}

/// A refusal a program can act on: the sentence, and its kind.
fn refuse_as(status: StatusCode, why: &Refusal) -> Response {
    let body = json!({"ok": false, "error": why.text, "kind": why.kind});
    (status, [(header::CACHE_CONTROL, "no-store")], Json(body)).into_response()
}

fn answer(v: Value) -> Response {
    ([(header::CACHE_CONTROL, "no-store")], Json(v)).into_response()
}

/// Whether a request's headers are ones a program on this computer sends, and no browser does.
fn headers_ok(h: &HeaderMap, method: &Method) -> bool {
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
    matches!(name, "127.0.0.1" | "localhost" | "::1") && (method != Method::POST || json)
}

async fn guard(
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    if !peer.ip().to_canonical().is_loopback() || !headers_ok(req.headers(), req.method()) {
        return refuse(StatusCode::FORBIDDEN, "only from this computer");
    }
    next.run(req).await
}

fn body(bytes: &[u8]) -> Result<Value, Box<Response>> {
    match serde_json::from_slice::<Value>(if bytes.is_empty() { b"{}" } else { bytes }) {
        Ok(v @ Value::Object(_)) => Ok(v),
        _ => Err(Box::new(refuse(StatusCode::BAD_REQUEST, "the request is not a JSON object"))),
    }
}

async fn status(State(k): State<Arc<Shared>>) -> Response {
    match k.status() {
        Ok(s) => answer(serde_json::to_value(s).unwrap_or(Value::Null)),
        Err(_) => refuse_as(StatusCode::SERVICE_UNAVAILABLE, &k.idle()),
    }
}

async fn pair_new(State(k): State<Arc<Shared>>) -> Response {
    match k.pair_new().await {
        Ok(p) => answer(serde_json::to_value(p).unwrap_or(Value::Null)),
        Err(e) => refuse_as(StatusCode::CONFLICT, &e),
    }
}

async fn pair_join(State(k): State<Arc<Shared>>, bytes: axum::body::Bytes) -> Response {
    let v = match body(&bytes) {
        Ok(v) => v,
        Err(r) => return *r,
    };
    let bad = |text: String| refuse_as(StatusCode::BAD_REQUEST, &Refusal::new("bad_input", text));
    let Some(input) = v["input"].as_str() else { return bad("No code or link was given.".into()) };
    let name = v["name"].as_str().map(str::to_string);
    let mut addrs = vec![];
    for a in v["addrs"].as_array().map(Vec::as_slice).unwrap_or(&[]) {
        match a.as_str().and_then(|a| a.parse::<SocketAddr>().ok()) {
            Some(a) => addrs.push(a),
            None => return bad(format!("{a} is not an address like 192.0.2.10:4433")),
        }
    }
    let replace = v["replace"] == true;
    match k.join(input, name, addrs, replace).await {
        Ok(w) => answer(json!({"ok": true, "home": {"id": w.home_id.to_string(), "name": w.home_name},
                               "machine": w.machine})),
        Err(e) if e.kind == "bad_input" => refuse_as(StatusCode::BAD_REQUEST, &e),
        Err(e) => refuse_as(StatusCode::CONFLICT, &e),
    }
}

async fn devices(State(k): State<Arc<Shared>>) -> Response {
    match k.devices() {
        // ("roster": the list of devices as home last signed it, for the server's own pages)
        Ok(d) => answer(json!({"ok": true, "devices": d, "roster": k.status().ok().and_then(|s| s.roster)})),
        Err(e) => refuse(StatusCode::CONFLICT, e),
    }
}

async fn devices_remove(State(k): State<Arc<Shared>>, bytes: axum::body::Bytes) -> Response {
    let v = match body(&bytes) {
        Ok(v) => v,
        Err(r) => return *r,
    };
    let Some(id) = v["id"].as_str().and_then(|s| s.parse::<DeviceId>().ok()) else {
        return refuse(StatusCode::BAD_REQUEST, "that is not a device id");
    };
    match k.remove(&id).await {
        Ok(true) => answer(json!({"ok": true})),
        Ok(false) => refuse(StatusCode::NOT_FOUND, "no such device is paired"),
        Err(e) => refuse(StatusCode::CONFLICT, e),
    }
}

async fn relay(State(k): State<Arc<Shared>>, bytes: axum::body::Bytes) -> Response {
    let v = match body(&bytes) {
        Ok(v) => v,
        Err(r) => return *r,
    };
    let url = v["url"].as_str().map(str::trim).filter(|u| !u.is_empty());
    match k.set_relay(url) {
        Ok(()) => answer(json!({"ok": true, "relay": url})),
        Err(e) => refuse(StatusCode::BAD_REQUEST, e),
    }
}

async fn roster(State(k): State<Arc<Shared>>) -> Response {
    match k.roster_rows() {
        Ok(v) => answer(v),
        Err(e) => refuse(StatusCode::CONFLICT, e),
    }
}

async fn devices_block(State(k): State<Arc<Shared>>, uri: axum::http::Uri, bytes: axum::body::Bytes) -> Response {
    let v = match body(&bytes) {
        Ok(v) => v,
        Err(r) => return *r,
    };
    let Some(id) = v["id"].as_str().and_then(|s| s.parse::<DeviceId>().ok()) else {
        return refuse(StatusCode::BAD_REQUEST, "that is not a device id");
    };
    if uri.path().ends_with("/unblock") {
        return match k.unblock(&id) {
            Ok(was) => answer(json!({"ok": true, "was": was})),
            Err(e) => refuse(StatusCode::CONFLICT, e),
        };
    }
    match k.block(&id) {
        Ok(name) => answer(json!({"ok": true, "name": name})),
        Err(e) => refuse(StatusCode::CONFLICT, e),
    }
}

pub fn router(shared: Arc<Shared>) -> Router {
    Router::new()
        .route("/status", get(status))
        .route("/roster", get(roster))
        .route("/devices/block", post(devices_block))
        .route("/devices/unblock", post(devices_block))
        .route("/pair/new", post(pair_new))
        .route("/pair/join", post(pair_join))
        .route("/devices", get(devices))
        .route("/devices/remove", post(devices_remove))
        .route("/relay", post(relay))
        .route("/home/status", get(home_status))
        .route("/home/move", post(home_move))
        .route("/home/backup", post(home_backup))
        .route("/home/restore", post(home_restore))
        .layer(axum::extract::DefaultBodyLimit::max(64 * 1024))
        .layer(axum::middleware::from_fn(guard))
        .with_state(shared)
}

async fn home_status(State(k): State<Arc<Shared>>) -> Response {
    answer(k.home_status())
}

async fn home_move(State(k): State<Arc<Shared>>, bytes: axum::body::Bytes) -> Response {
    let v = match body(&bytes) {
        Ok(v) => v,
        Err(r) => return *r,
    };
    let bad =
        || refuse_as(StatusCode::BAD_REQUEST, &Refusal::new("bad_input", "Say which computer, or resume, or undo."));
    let ask = match (v["resume"] == true, v["undo"] == true, v["to"].as_str()) {
        (true, false, None) => MoveAsk::Resume,
        (false, true, None) => MoveAsk::Undo,
        (false, false, Some(id)) => match id.parse::<DeviceId>() {
            Ok(id) => MoveAsk::To { id, retire: v["retire"] == true },
            Err(_) => return bad(),
        },
        _ => return bad(),
    };
    match k.home_move(ask).await {
        Ok(v) => answer(v),
        Err(e) => refuse_as(StatusCode::CONFLICT, &e),
    }
}

async fn home_backup(State(k): State<Arc<Shared>>, bytes: axum::body::Bytes) -> Response {
    let v = match body(&bytes) {
        Ok(v) => v,
        Err(r) => return *r,
    };
    match k.home_backup(v["to"].as_str().filter(|t| !t.is_empty()).map(std::path::PathBuf::from)).await {
        Ok(v) => answer(v),
        Err(e) => refuse_as(StatusCode::CONFLICT, &e),
    }
}

async fn home_restore(State(k): State<Arc<Shared>>, bytes: axum::body::Bytes) -> Response {
    let v = match body(&bytes) {
        Ok(v) => v,
        Err(r) => return *r,
    };
    let (Some(file), Some(code)) = (v["file"].as_str(), v["code"].as_str()) else {
        return refuse_as(
            StatusCode::BAD_REQUEST,
            &Refusal::new("bad_input", "Say which file, and its recovery code."),
        );
    };
    match k.home_restore(std::path::Path::new(file), code).await {
        Ok(v) => answer(v),
        Err(e) => refuse_as(StatusCode::CONFLICT, &e),
    }
}

pub async fn serve(shared: Arc<Shared>, listener: tokio::net::TcpListener) -> std::io::Result<()> {
    axum::serve(listener, router(shared).into_make_service_with_connect_info::<SocketAddr>()).await
}

// ---- the command line's side ----------------------------------------------------------------------------------

/// What asking the running helper came to.
enum Asked {
    Yes(Value),
    /// It answered, and said no.
    No(String),
    /// Nothing listens there: no helper is running with keys on.
    Nobody,
}

async fn ask(k: &Shared, method: &str, path: &str, body: Option<Value>, secs: u64) -> Asked {
    let addr = k.ports.control.to_string();
    match crate::http_json(&addr, method, path, body.as_ref(), Duration::from_secs(secs)).await {
        Ok((200, v)) if v["ok"] == true => Asked::Yes(v),
        Ok((0, _)) | Err(_) => Asked::Nobody,
        Ok((status, v)) => Asked::No(
            v["error"].as_str().map(str::to_string).unwrap_or_else(|| format!("The helper answered {status}.")),
        ),
    }
}

const NOT_RUNNING: &str = "The helper is not running with keys on, on this computer. On the home computer, turn \
                           keys on in the app first; then start it: switchboard-node run";

fn shared() -> Result<Arc<Shared>, i32> {
    Shared::here(&crate::config_path()).map_err(|e| crate::fail(&e, 2))
}

fn ago(now: u64, then: u64) -> String {
    match now.saturating_sub(then) {
        s if s < 90 => "just now".into(),
        s if s < 5400 => format!("{} minutes ago", s / 60),
        s if s < 129_600 => format!("{} hours ago", s / 3600),
        s => format!("{} days ago", s / 86_400),
    }
}

/// The lines `status` prints about keys. None when keys are off here (nothing answers on the control port).
pub async fn status_lines() -> Option<Vec<String>> {
    let k = Shared::here(&crate::config_path()).ok()?;
    match ask(&k, "GET", "/status", None, 5).await {
        Asked::Yes(v) => Some(lines_of(&serde_json::from_value::<Status>(v).ok()?)),
        // (a helper answers on the control port with keys off too: that is "off", not a problem)
        Asked::No(_) if k.role() == crate::keys::Role::Off => None,
        Asked::No(why) => Some(vec!["Keys: on, and not running.".into(), format!("Problem: {why}")]),
        Asked::Nobody if k.role() != crate::keys::Role::Off => {
            Some(vec!["Keys: on, but the helper is not running. Start it: switchboard-node run".into()])
        }
        Asked::Nobody => None,
    }
}

pub fn lines_of(s: &Status) -> Vec<String> {
    let mut out = vec![];
    let how = |path: &Option<String>, rtt: Option<u64>| match (path.as_deref(), rtt) {
        (Some("relay"), Some(ms)) => format!("connected through the relay ({ms} ms)"),
        (Some("relay"), None) => "connected through the relay".to_string(),
        (Some(_), Some(ms)) => format!("connected directly ({ms} ms)"),
        _ => "connected".to_string(),
    };
    if s.role == "home" {
        out.push(format!("Keys: on. This is the home computer ({}).", s.id.short()));
        out.push(match s.devices {
            0 => "Paired devices: none yet. Add one with: switchboard-node code".to_string(),
            1 => "Paired devices: 1 (switchboard-node devices lists it)".to_string(),
            n => format!("Paired devices: {n} (switchboard-node devices lists them)"),
        });
        // A link that names no relay needs this typed on the computer that joins (pair --addr).
        let reach = |a: &&String| a.parse::<std::net::SocketAddr>().is_ok_and(|s| !s.ip().is_loopback());
        if s.relay.url.is_none() {
            let v4 = s.addrs.iter().filter(reach).find(|a| !a.starts_with('['));
            if let Some(a) = v4.or_else(|| s.addrs.iter().find(reach)) {
                out.push(format!("Address for a link without a relay: {a}"));
            }
        }
    } else {
        out.push(format!("Keys: on. This computer's id is {}.", s.id.short()));
        if let Some(h) = &s.home {
            let name = if h.name.is_empty() { h.id.short() } else { format!("{} ({})", h.name, h.id.short()) };
            out.push(if h.connected {
                format!("Home computer: {name}, {}", how(&h.path, h.rtt_ms))
            } else {
                format!("Home computer: {name}, not connected")
            });
        }
        for f in &s.forwarders {
            let what = if f.target == "api" { "server" } else { "hub" };
            out.push(if f.ok {
                format!("{} here leads to the home computer's {what}.", f.addr)
            } else {
                format!("{} here is in use by another program, so nothing leads to the home computer's {what}.", f.addr)
            });
        }
    }
    if s.role != "home" {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        out.push(match &s.roster {
            Some(r) => format!("List of devices: signed by the home computer {}.", ago(now, r.at)),
            None => "List of devices: none yet. It comes from the home computer the first time this computer \
                     reaches it."
                .to_string(),
        });
    }
    if let Some(a) = &s.away {
        out.extend(a["line"].as_str().map(str::to_string));
        out.extend(a["notes"].as_array().into_iter().flatten().filter_map(|n| n.as_str().map(str::to_string)));
    }
    out.push(match (&s.relay.url, s.relay.connected) {
        (Some(u), true) => format!("Relay: {u} (connected; from {})", s.relay.source),
        (Some(u), false) => format!("Relay: {u} (not connected; from {})", s.relay.source),
        (None, _) => "Relay: none".to_string(),
    });
    out.extend(s.problems.iter().map(|p| format!("Problem: {p}")));
    out
}

/// `pair <code-or-link> [--replace]`: through the running helper when it has keys on, else here and now.
pub async fn pair(typed: &str, name: Option<String>, addrs: Vec<String>, replace: bool) -> i32 {
    let k = match shared() {
        Ok(k) => k,
        Err(code) => return code,
    };
    let mut parsed = vec![];
    for a in &addrs {
        match a.parse::<SocketAddr>() {
            Ok(a) => parsed.push(a),
            Err(_) => return crate::fail(&format!("{a} is not an address like 192.0.2.10:4433"), 2),
        }
    }
    let running = !matches!(ask(&k, "GET", "/status", None, 5).await, Asked::Nobody) || k.held_elsewhere();
    let done = if running {
        let body = json!({"input": typed, "name": name, "addrs": addrs, "replace": replace});
        match ask(&k, "POST", "/pair/join", Some(body), 90).await {
            Asked::Yes(v) => {
                Ok((v["home"]["name"].as_str().unwrap_or("").to_string(), v["machine"].as_str().map(str::to_string)))
            }
            Asked::No(why) => Err(why),
            Asked::Nobody => Err("The helper that is running did not answer. Stop it and try again.".to_string()),
        }
    } else {
        k.join(typed, name, parsed, replace).await.map(|w| (w.home_name, w.machine)).map_err(|e| e.text)
    };
    match done {
        Ok((home, machine)) => {
            let home = if home.is_empty() { "the home computer".to_string() } else { home };
            match machine {
                Some(m) => say!("Paired with {home}. This computer is {m} there."),
                None => say!("Paired with {home}."),
            }
            if !running {
                say!("Start it with: switchboard-node run");
            }
            0
        }
        Err(why) => crate::fail(&why, 1),
    }
}

/// A QR code for a terminal, two rows of the code to each line of text.
pub fn qr(text: &str) -> Option<String> {
    use qrcode::render::unicode::Dense1x2;
    let code = qrcode::QrCode::with_error_correction_level(text.as_bytes(), qrcode::EcLevel::L).ok()?;
    Some(code.render::<Dense1x2>().dark_color(Dense1x2::Light).light_color(Dense1x2::Dark).build())
}

/// `code`: a short code and a link for one new device.
pub async fn code() -> i32 {
    let k = match shared() {
        Ok(k) => k,
        Err(code) => return code,
    };
    match ask(&k, "POST", "/pair/new", Some(json!({})), 30).await {
        Asked::Yes(v) => {
            let Ok(p) = serde_json::from_value::<PairNew>(v) else {
                return crate::fail("The helper's answer could not be read.", 1);
            };
            match &p.code {
                Some(c) => say!("Code: {c}"),
                None => say!("Code: none (a short code needs a relay: switchboard-node relay <url>)"),
            }
            say!("Link: {}", p.link);
            say!(
                "Both work once, for the next 10 minutes. On the other computer: switchboard-node pair <code or link>"
            );
            if let Some(q) = qr(&p.link) {
                say!("\n{q}");
            }
            0
        }
        Asked::No(why) => crate::fail(&why, 1),
        Asked::Nobody => crate::fail(NOT_RUNNING, 1),
    }
}

async fn list(k: &Shared) -> Result<Vec<DeviceRow>, i32> {
    match ask(k, "GET", "/devices", None, 10).await {
        Asked::Yes(v) => serde_json::from_value::<Vec<DeviceRow>>(v["devices"].clone())
            .map_err(|_| crate::fail("The helper's answer could not be read.", 1)),
        Asked::No(why) => Err(crate::fail(&why, 1)),
        Asked::Nobody => Err(crate::fail(NOT_RUNNING, 1)),
    }
}

/// `devices`: the devices paired with this home computer.
pub async fn devices_cmd() -> i32 {
    let k = match shared() {
        Ok(k) => k,
        Err(code) => return code,
    };
    let rows = match list(&k).await {
        Ok(r) => r,
        Err(code) => return code,
    };
    if rows.is_empty() {
        say!("No devices are paired yet. Add one with: switchboard-node code");
        return 0;
    }
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    for d in rows {
        let state = match (d.path.as_deref(), d.last_seen) {
            (Some("relay"), _) => "connected through the relay".to_string(),
            (Some(_), _) => "connected directly".to_string(),
            (None, Some(t)) => format!("last seen {}", ago(now, t)),
            (None, None) => "not seen since the helper started".to_string(),
        };
        let kind = if d.kind == sb_link::paired::Kind::Phone { "phone" } else { "computer" };
        say!("{}  {kind}  {}  {state}", d.name, d.id.short());
    }
    0
}

/// Which device `forget <name>` means: its name, its name at the hub, or the start of its id. Err says why
/// none or more than one fits.
pub fn pick(rows: &[DeviceRow], name: &str) -> Result<DeviceId, String> {
    let low = name.to_lowercase();
    let fits: Vec<&DeviceRow> = rows
        .iter()
        .filter(|d| {
            d.name.to_lowercase() == low
                || d.machine.as_deref() == Some(low.as_str())
                || (low.len() >= 6 && d.id.to_string().starts_with(&low))
        })
        .collect();
    match fits.as_slice() {
        [one] => Ok(one.id),
        [] => Err(format!("No paired device is called {name}. switchboard-node devices lists them.")),
        _ => Err(format!(
            "More than one paired device is called {name}. Use the start of its id instead (switchboard-node devices \
             shows it)."
        )),
    }
}

const OLD_COPIES: &str = "Copies of this computer's disk made before the move (system backups, disk images) still \
                          hold the home computer's key. Wipe them, or keep them as safe as the home computer itself.";

/// `home status | move | backup | restore`: where the home computer is, moving it, a copy of it, and bringing it
/// back from one.
pub async fn home_cmd(args: &[String]) -> i32 {
    let k = match shared() {
        Ok(k) => k,
        Err(code) => return code,
    };
    let flag = |f: &str| args.iter().any(|a| a == f);
    let plain: Vec<&String> = args.iter().skip(1).filter(|a| !a.starts_with("--")).collect();
    match args.first().map(String::as_str) {
        Some("status") => match ask(&k, "GET", "/home/status", None, 10).await {
            Asked::Yes(v) => {
                say!(
                    "{}",
                    match v["role"].as_str() {
                        Some("home") => "This computer is the home computer.",
                        Some("member") => "This computer is paired with a home computer. It is not the home computer.",
                        _ => "This computer is not a home computer, and is not paired with one.",
                    }
                );
                if let Some(line) = v["stood_down"]["line"].as_str() {
                    say!("{line}");
                }
                if let Some(to) = v["move"]["to"].as_str() {
                    say!(
                        "A move to {to} was started and is not finished. Finish it: switchboard-node home move --resume"
                    );
                }
                say!(
                    "{}",
                    if v["can_host"] == true {
                        "It has the parts a home computer needs, so the home computer can move here."
                    } else {
                        "It does not have the parts a home computer needs. To add them: install-server.sh --standby"
                    }
                );
                if v["role"] == "home" {
                    say!(
                        "{}",
                        if v["backup"]["code_made"] == true {
                            "A recovery code was made. Make a backup: switchboard-node home backup"
                        } else {
                            "No backup was ever made. Make the first one: switchboard-node home backup"
                        }
                    );
                }
                0
            }
            Asked::No(why) => crate::fail(&why, 1),
            Asked::Nobody => {
                crate::fail("The helper is not running on this computer. Start it: switchboard-node run", 1)
            }
        },
        Some("move") => {
            let ask_body = if flag("--resume") {
                json!({"resume": true})
            } else if flag("--undo") {
                json!({"undo": true})
            } else {
                let Some(name) = plain.first() else { return crate::usage() };
                let rows = match list(&k).await {
                    Ok(r) => r,
                    Err(code) => return code,
                };
                match pick(&rows, name) {
                    Ok(id) => json!({"to": id.to_string(), "retire": flag("--retire")}),
                    Err(why) => return crate::fail(&why, 1),
                }
            };
            if ask_body["to"].is_string() {
                say!(
                    "Moving the home computer. The server here stops now, and nobody answers as home until the other computer has taken over."
                );
            }
            match ask(&k, "POST", "/home/move", Some(ask_body), 3700).await {
                Asked::Yes(v) if v["state"] == "undone" => {
                    say!("The move was called off. This computer is the home computer, as before.");
                    0
                }
                Asked::Yes(v) => {
                    let name = v["home"]["name"].as_str().unwrap_or("the other computer");
                    say!("The home computer is now {name}.");
                    if let Some(ms) = v["gap_ms"].as_u64().filter(|m| *m > 0) {
                        say!("Nobody answered as the home computer for {}.{} seconds.", ms / 1000, (ms % 1000) / 100);
                    }
                    if v["retired"] == true {
                        say!("This computer was retired: it holds no key and is on no list.");
                    } else {
                        say!("This computer goes on as one of its computers. Nothing else needs doing here.");
                    }
                    say!("{OLD_COPIES}");
                    0
                }
                Asked::No(why) => crate::fail(&why, 1),
                Asked::Nobody => {
                    crate::fail("The helper is not running on this computer. Start it: switchboard-node run", 1)
                }
            }
        }
        Some("backup") => {
            let to = args.iter().position(|a| a == "--to").and_then(|i| args.get(i + 1));
            if flag("--to") && to.is_none() {
                return crate::usage();
            }
            // (a path as the person typed it means from where they stand, not from where the helper runs)
            let to = to.map(|t| std::path::absolute(t).map(|p| p.display().to_string()).unwrap_or_else(|_| t.clone()));
            match ask(&k, "POST", "/home/backup", Some(json!({"to": to})), 3700).await {
                Asked::Yes(v) => {
                    say!(
                        "Backup written to {} ({} bytes).",
                        v["file"].as_str().unwrap_or("?"),
                        v["bytes"].as_u64().unwrap_or(0)
                    );
                    if let Some(code) = v["code"].as_str() {
                        say!("");
                        say!("Recovery code: {code}");
                        say!("");
                        say!("Write it down now. It is shown this once and is kept nowhere on this computer.");
                        say!(
                            "Without it no backup of this home computer can be opened. With it and a backup file, anybody can become this home computer: keep the two apart."
                        );
                    } else {
                        say!("It opens with the recovery code that was shown when the first backup was made.");
                    }
                    0
                }
                Asked::No(why) => crate::fail(&why, 1),
                Asked::Nobody => crate::fail(NOT_RUNNING, 1),
            }
        }
        Some("restore") => {
            let Some(file) = plain.first() else { return crate::usage() };
            let file = std::path::absolute(file).map(|p| p.display().to_string()).unwrap_or_else(|_| file.to_string());
            complain!("Recovery code:");
            let mut code = String::new();
            if std::io::stdin().read_line(&mut code).is_err() || code.trim().is_empty() {
                return crate::fail("No recovery code was given.", 1);
            }
            let done =
                match ask(&k, "POST", "/home/restore", Some(json!({"file": file, "code": code.trim()})), 3700).await {
                    Asked::Yes(v) => Ok(v),
                    Asked::No(why) => Err(why),
                    // no helper is running here: the work is done by this command itself
                    Asked::Nobody => k.home_restore(std::path::Path::new(&file), code.trim()).await.map_err(|e| e.text),
                };
            match done {
                Ok(v) => {
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs())
                        .unwrap_or(0);
                    say!(
                        "This computer is the home computer now, as it was when the backup was made ({}).",
                        ago(now, v["made"].as_u64().unwrap_or(now))
                    );
                    for d in v["devices"].as_array().map(Vec::as_slice).unwrap_or(&[]) {
                        say!(
                            "  paired: {} ({})",
                            d["name"].as_str().unwrap_or("?"),
                            d["kind"].as_str().unwrap_or("device")
                        );
                    }
                    say!(
                        "What happened after the backup was made is lost: chats and settings since then, and devices paired since then (pair those again)."
                    );
                    say!(
                        "A device that was removed after the backup is removed again as soon as another of your computers connects."
                    );
                    say!(
                        "If the old home computer still exists, do not start it. If it does start, it stands down by itself when it hears of this one."
                    );
                    0
                }
                Err(why) => crate::fail(&why, 1),
            }
        }
        _ => crate::usage(),
    }
}

/// `forget <name>`: remove a paired device.
pub async fn forget(name: &str) -> i32 {
    let k = match shared() {
        Ok(k) => k,
        Err(code) => return code,
    };
    let rows = match list(&k).await {
        Ok(r) => r,
        Err(code) => return code,
    };
    let id = match pick(&rows, name) {
        Ok(id) => id,
        Err(why) => return crate::fail(&why, 1),
    };
    match ask(&k, "POST", "/devices/remove", Some(json!({"id": id.to_string()})), 30).await {
        Asked::Yes(_) => {
            say!("Removed {name}. Its key no longer works here.");
            0
        }
        Asked::No(why) => crate::fail(&why, 1),
        Asked::Nobody => crate::fail(NOT_RUNNING, 1),
    }
}

/// Which device on the home computer's list `block <name>` means: its name, its name at the hub, or the start
/// of its id.
pub fn pick_listed(rows: &[Value], name: &str) -> Result<(DeviceId, String), String> {
    let low = name.to_lowercase();
    let fits: Vec<&Value> = rows
        .iter()
        .filter(|d| {
            d["name"].as_str().is_some_and(|n| n.to_lowercase() == low)
                || d["machine"].as_str() == Some(low.as_str())
                || (low.len() >= 6 && d["id"].as_str().is_some_and(|i| i.starts_with(&low)))
        })
        .collect();
    match fits.as_slice() {
        [one] => match one["id"].as_str().and_then(|i| i.parse::<DeviceId>().ok()) {
            Some(id) => Ok((id, one["name"].as_str().unwrap_or(name).to_string())),
            None => Err("The helper's answer could not be read.".into()),
        },
        [] => Err(format!("No device on the home computer's list is called {name}.")),
        _ => {
            Err(format!("More than one device on the home computer's list is called {name}. Use the start of its id."))
        }
    }
}

/// `block <name>` and `unblock <name>`: a device this computer stops answering, or answers again.
pub async fn block_cmd(name: &str, block: bool) -> i32 {
    let k = match shared() {
        Ok(k) => k,
        Err(code) => return code,
    };
    let rows = match ask(&k, "GET", "/roster", None, 10).await {
        Asked::Yes(v) => v["devices"].as_array().cloned().unwrap_or_default(),
        Asked::No(why) => return crate::fail(&why, 1),
        Asked::Nobody => return crate::fail(NOT_RUNNING, 1),
    };
    let (id, called) = match pick_listed(&rows, name) {
        Ok(found) => found,
        Err(why) => return crate::fail(&why, 1),
    };
    let path = if block { "/devices/block" } else { "/devices/unblock" };
    match ask(&k, "POST", path, Some(json!({"id": id.to_string()})), 10).await {
        Asked::Yes(_) if block => {
            say!(
                "Blocked {called}. This computer no longer answers it. That lasts until the home computer's list of \
                 devices changes. To remove it everywhere, on the home computer: switchboard-node forget {called}"
            );
            0
        }
        Asked::Yes(v) if v["was"] == true => {
            say!("Unblocked {called}. This computer answers it again.");
            0
        }
        Asked::Yes(_) => {
            say!("{called} was not blocked.");
            0
        }
        Asked::No(why) => crate::fail(&why, 1),
        Asked::Nobody => crate::fail(NOT_RUNNING, 1),
    }
}

/// `relay <url>` or `relay none`: written by the running helper, or straight to node.json when none is running.
pub async fn relay_cmd(url: &str) -> i32 {
    let k = match shared() {
        Ok(k) => k,
        Err(code) => return code,
    };
    let url = Some(url.trim()).filter(|u| !u.is_empty() && *u != "none");
    let done = match ask(&k, "POST", "/relay", Some(json!({"url": url})), 10).await {
        Asked::Yes(_) => Ok(()),
        Asked::No(why) => Err(why),
        Asked::Nobody => k.set_relay(url),
    };
    match (done, url) {
        (Ok(()), Some(u)) => {
            say!("The relay is now {u}.");
            0
        }
        (Ok(()), None) => {
            say!("No relay is set now.");
            0
        }
        (Err(why), _) => crate::fail(&why, 1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sb_link::control::{HomeRow, RelayRow};
    use sb_link::paired::{Device, Kind, Via};

    #[test]
    fn only_a_program_on_this_computer_gets_past_the_headers() {
        let h = |pairs: &[(&str, &str)]| {
            let mut m = HeaderMap::new();
            for (k, v) in pairs {
                m.insert(axum::http::HeaderName::from_bytes(k.as_bytes()).unwrap(), v.parse().unwrap());
            }
            m
        };
        assert!(headers_ok(&h(&[("host", "127.0.0.1:8137")]), &Method::GET));
        assert!(headers_ok(&h(&[("host", "localhost:8137")]), &Method::GET));
        assert!(headers_ok(&h(&[("host", "[::1]:8137")]), &Method::GET));
        assert!(!headers_ok(&h(&[("host", "evil.example:8137")]), &Method::GET), "a name that resolves here");
        assert!(!headers_ok(&h(&[]), &Method::GET));
        assert!(!headers_ok(&h(&[("host", "127.0.0.1:8137"), ("origin", "http://127.0.0.1:8130")]), &Method::GET));
        assert!(!headers_ok(&h(&[("host", "127.0.0.1:8137"), ("sec-fetch-site", "same-origin")]), &Method::GET));
        assert!(!headers_ok(&h(&[("host", "127.0.0.1:8137")]), &Method::POST), "a form post");
        assert!(!headers_ok(&h(&[("host", "127.0.0.1:8137"), ("content-type", "text/plain")]), &Method::POST));
        assert!(headers_ok(&h(&[("host", "127.0.0.1:8137"), ("content-type", "application/json")]), &Method::POST));
    }

    fn row(name: &str, machine: Option<&str>, byte: u8) -> DeviceRow {
        DeviceRow::of(Device {
            id: DeviceId([byte; 32]),
            name: name.into(),
            kind: Kind::Computer,
            machine: machine.map(str::to_string),
            paired: 1,
            via: Via::Code,
        })
    }

    #[test]
    fn forget_finds_one_device_or_says_why_not() {
        let rows = [row("Laptop", Some("laptop"), 0x11), row("Desk", Some("desk2"), 0x22), row("Desk", None, 0x33)];
        assert_eq!(pick(&rows, "laptop"), Ok(DeviceId([0x11; 32])));
        assert_eq!(pick(&rows, "desk2"), Ok(DeviceId([0x22; 32])));
        assert_eq!(pick(&rows, "333333"), Ok(DeviceId([0x33; 32])));
        assert!(pick(&rows, "Desk").unwrap_err().contains("More than one"));
        assert!(pick(&rows, "phone").unwrap_err().contains("No paired device"));
        assert!(pick(&rows, "11").is_err(), "two characters of an id are not enough");
    }

    #[test]
    fn status_is_said_in_plain_words() {
        let mut s = Status {
            ok: true,
            version: "1".into(),
            role: "member".into(),
            id: DeviceId([0xab; 32]),
            name: "laptop".into(),
            machine: Some("laptop".into()),
            relay: RelayRow {
                url: Some("https://r.example".into()),
                source: "home".into(),
                connected: true,
                error: None,
                same_id: false,
            },
            home: Some(HomeRow {
                id: DeviceId([0xcd; 32]),
                name: "studio".into(),
                connected: true,
                path: Some("direct".into()),
                rtt_ms: Some(3),
                error: None,
                refused: false,
            }),
            forwarders: vec![],
            devices: 0,
            updates: vec![],
            addrs: vec![],
            roster: None,
            away: None,
            problems: vec!["Something is wrong.".into()],
        };
        let text = lines_of(&s).join("\n");
        assert!(text.contains("Home computer: studio") && text.contains("connected directly (3 ms)"), "{text}");
        assert!(text.contains("Relay: https://r.example (connected; from home)"), "{text}");
        assert!(text.contains("Problem: Something is wrong."), "{text}");
        s.home.as_mut().unwrap().path = Some("relay".into());
        assert!(lines_of(&s).join("\n").contains("connected through the relay (3 ms)"));
        s.role = "home".into();
        s.devices = 2;
        s.relay = RelayRow::default();
        let text = lines_of(&s).join("\n");
        assert!(text.contains("This is the home computer") && text.contains("Paired devices: 2"), "{text}");
        assert!(text.contains("Relay: none"), "{text}");
        assert!(!text.contains("Address"), "{text}");
        s.addrs = vec!["127.0.0.1:41641".into(), "192.0.2.20:41641".into()];
        let text = lines_of(&s).join("\n");
        assert!(text.contains("Address for a link without a relay: 192.0.2.20:41641"), "{text}");
        s.relay.url = Some("https://r.example".into());
        assert!(!lines_of(&s).join("\n").contains("Address for a link"), "with a relay nobody types an address");
        for line in lines_of(&s) {
            assert!(!line.contains('!') && !line.contains('\u{2014}'), "{line}");
        }
        // a computer that is not home says what it knows of the others, and when home is away what it shows
        s.role = "member".into();
        assert!(lines_of(&s).join("\n").contains("List of devices: none yet."));
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
        s.roster = Some(sb_link::control::RosterRow { epoch: 1, seq: 4, at: now - 7200, expires: now + 86400 });
        s.away = Some(json!({
            "line": "Home is away. Showing chats from Laptop and Tower.",
            "notes": ["Mini needs an update before it can answer while the home computer is away."],
        }));
        let text = lines_of(&s).join("\n");
        assert!(text.contains("List of devices: signed by the home computer 2 hours ago."), "{text}");
        assert!(text.contains("\nHome is away. Showing chats from Laptop and Tower.\nMini needs an update"), "{text}");
        for line in lines_of(&s) {
            assert!(!line.contains('!') && !line.contains('\u{2014}'), "{line}");
        }
    }

    #[test]
    fn a_link_makes_a_qr_code() {
        let q = qr("switchboard://pair/1?h=abc").unwrap();
        assert!(q.lines().count() > 10 && q.contains('\u{2588}'), "{q}");
    }
}
