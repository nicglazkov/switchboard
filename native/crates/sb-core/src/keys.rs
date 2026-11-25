//! Keys: paired devices, pairing codes, joining a home, and how this computer reaches home
//! (docs/plans/2026-11-22-keys-phase-1-2.md, sections 3.4 to 3.8 and 7).
//!
//! Three things live here, none of which draws:
//!
//! - **The wire, read in one place.** What the server's keys routes answer (`/api/keys/readiness`,
//!   `/api/keys/devices`, `/api/pair_device`) and what the node's control API on this computer answers
//!   (`127.0.0.1:8137`). Every field name is in the `read_*` functions below and nowhere else, and each is read
//!   leniently: a field that is missing or has another shape is left out, never an error for the whole answer.
//! - **The node's control API client.** `NodeControl` is the trait the app calls, `HttpNode` the real one and
//!   `FakeNode` the stand-in for tests. The node side was still being written when this was, so the shapes are
//!   this module's assumption of it; they are listed on `HttpNode`.
//! - **What the app says.** Rows for the Devices list, the code and its countdown, the join screen's state and
//!   its errors in plain words, the line that says how a computer is connected, and what is left before the
//!   network port can close.
//!
//! When keys are off and no node answers, every view model here is empty (`Keys::on`, `Keys::can_join`), and the
//! app shows nothing of this.

use std::sync::Mutex;
use std::time::Duration;

use serde_json::Value;

use crate::fmt;
use crate::hostsetup::and_list;

// ---------------------------------------------------------------------------------------------------------------
// Wire types
// ---------------------------------------------------------------------------------------------------------------

/// server.json's "keys" as the server reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    /// By address only, as before keys. Also every server from before keys.
    #[default]
    Off,
    /// By address, and with keys.
    Both,
    /// With keys only: the network port is closed.
    Only,
}

impl Mode {
    /// A value this app does not know counts as `Only`, the way the server reads one (server/keys.py `mode_of`).
    fn read(v: Option<&Value>) -> Mode {
        match v.and_then(Value::as_str) {
            None | Some("off") => Mode::Off,
            Some("both") => Mode::Both,
            Some(_) => Mode::Only,
        }
    }
}

/// How a paired device's traffic gets here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Path {
    Direct,
    Relay,
}

impl Path {
    fn read(v: Option<&Value>) -> Option<Path> {
        match v.and_then(Value::as_str) {
            Some("direct") => Some(Path::Direct),
            Some("relay") | Some("relayed") => Some(Path::Relay),
            _ => None,
        }
    }

    /// "Direct", "Through the relay".
    pub fn says(self) -> &'static str {
        match self {
            Path::Direct => "Direct",
            Path::Relay => "Through the relay",
        }
    }
}

/// One device paired with the home computer.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Device {
    pub id: String,
    pub name: String,
    /// "computer" or "phone".
    pub kind: String,
    /// Its name in the list of computers, when it is one of them.
    pub machine: Option<String>,
    /// When it was paired (epoch seconds, 0 when not known).
    pub paired: f64,
    /// "code", "link" or "upgrade".
    pub via: String,
    pub last_seen: Option<f64>,
    pub path: Option<Path>,
}

/// A code and a link that pair one device.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PairOffer {
    /// As the node printed it, usually `ABCD-EFGH`. Empty when the home computer has no relay: a short code can
    /// only find home through one, so there is the link (and its QR code) and nothing shorter.
    pub code: String,
    /// `switchboard://pair/1?...`, empty when the node sent none.
    pub link: String,
    /// When both stop working (epoch seconds).
    pub expires: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Role {
    /// The home computer: the others dial it.
    Home,
    /// A computer that joined a home.
    Member,
    /// Not paired with anything yet, or the node did not say.
    #[default]
    Alone,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Home {
    pub id: String,
    pub name: String,
    /// How this computer reaches it right now. None while it does not.
    pub path: Option<Path>,
    /// Why it does not, in the node's words.
    pub error: Option<String>,
    /// Home says this computer is not paired with it any more: it has to join again.
    pub refused: bool,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Peer {
    pub id: String,
    pub name: String,
    pub path: Option<Path>,
    /// Round trip in milliseconds.
    pub rtt_ms: Option<f64>,
}

/// Something the node says is wrong, as a plain sentence. `kind` is a short word when the node gave one.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Problem {
    pub kind: Option<String>,
    pub text: String,
}

impl Problem {
    /// macOS keeps the node off the local network (amendment 1): the relay connect fails with "No route to host".
    pub fn local_network(&self) -> bool {
        let kind = self.kind.as_deref().unwrap_or("").to_ascii_lowercase();
        kind == "local_network" || kind == "local-network" || self.text.to_ascii_lowercase().contains("local network")
    }
}

/// A port on this computer that the node carries to the home computer.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Forward {
    /// "api" (the home computer's server) or "hub".
    pub target: String,
    /// Where it listens, "127.0.0.1:8130".
    pub addr: String,
    /// False when another program holds the port.
    pub ok: bool,
}

/// The node's `GET /status`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct NodeStatus {
    pub keys: bool,
    pub role: Role,
    /// This computer's name in the list of computers.
    pub machine: Option<String>,
    /// This computer's own device id.
    pub id: Option<String>,
    pub home: Option<Home>,
    pub relay: Option<String>,
    /// Where the relay's address came from: "environment", "settings", "home", "built in" or "none".
    pub relay_source: Option<String>,
    /// Whether the node is connected to its relay, when it said.
    pub relay_connected: Option<bool>,
    /// Where other computers can reach this one without a relay ("192.0.2.20:41641"), on a home computer.
    pub addrs: Vec<String>,
    /// The ports on this computer that lead to the home computer, on a computer that joined one.
    pub forwards: Vec<Forward>,
    pub peers: Vec<Peer>,
    pub problems: Vec<Problem>,
}

/// The home computer's list of devices as this computer holds it (the node's `GET /roster`). It is what this
/// computer goes by while the home computer is away: who it answers, and where the others are.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct HeldList {
    /// When the home computer signed it, and when it runs out. None: this computer holds no list yet.
    pub at: Option<f64>,
    pub expires: Option<f64>,
    pub devices: Vec<HeldDevice>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct HeldDevice {
    pub id: String,
    pub name: String,
    pub phone: bool,
    /// The home computer itself, and this computer: neither can be blocked.
    pub home: bool,
    pub here: bool,
    /// This computer does not answer it, until the home computer's list changes.
    pub blocked: bool,
}

impl HeldDevice {
    pub fn can_block(&self) -> bool {
        !self.home && !self.here
    }
}

pub const HELD_TITLE: &str = "While the home computer is away";
pub const HELD_NONE: &str = "This computer holds no list of devices yet. It comes from the home computer the first time this computer reaches it.";
pub const HELD_STALE: &str = "The home computer's list of devices is more than 7 days old, so no device is answered by this computer until it reaches the home computer again.";
/// Under the rows: what Block does and does not do.
pub const BLOCK_EXPLAINS: &str = "Block stops this computer answering a device, here only, until the home computer's list changes. To remove a device everywhere, remove it on the home computer.";

/// The card's sentence: how old this computer's copy of the list is.
pub fn held_line(list: &HeldList, now: f64) -> String {
    match (list.at, list.expires) {
        (None, _) => HELD_NONE.to_string(),
        (Some(_), Some(e)) if e <= now => HELD_STALE.to_string(),
        (Some(at), _) => format!(
            "This computer answers the devices on the home computer's list. Its copy was signed {}.",
            fmt::ago(at, now)
        ),
    }
}

/// What `POST /pair/join` answers when it worked.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Joined {
    pub home: Option<String>,
    pub machine: Option<String>,
}

/// One thing that switching to keys only would cut off (server/keys.py `readiness`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Blocker {
    /// "machine", "direct", "watching", "node", "settings" or "mode".
    pub kind: String,
    /// The server's own sentence.
    pub text: String,
    pub label: Option<String>,
    pub paired: Option<bool>,
    /// "iPhone", "Android", "desktop", "helper", "web" or "other".
    pub client: Option<String>,
    pub last: Option<f64>,
    pub watched_days: Option<u64>,
    /// A phone app asked by address, and a paired phone has come with its key since.
    pub paired_since: bool,
}

/// One computer in the readiness report.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ReadyMachine {
    pub name: String,
    pub label: String,
    pub home: bool,
    pub paired: bool,
    pub seen: Option<f64>,
    pub path: Option<Path>,
}

/// `GET /api/keys/readiness`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Readiness {
    pub keys: Mode,
    pub ready: bool,
    pub days: u64,
    pub blockers: Vec<Blocker>,
    pub machines: Vec<ReadyMachine>,
    /// The server takes `POST /api/keys/mode`: the app can turn keys on and close the network port.
    pub switch: bool,
    /// The server says this app asked from the home computer itself. Means something only while keys are off:
    /// with keys on, everything through the tunnel arrives from the home computer too.
    pub here: bool,
}

// ---------------------------------------------------------------------------------------------------------------
// Reading the wire. Field names live here and nowhere else.
// ---------------------------------------------------------------------------------------------------------------

fn text(v: &Value, k: &str) -> Option<String> {
    v.get(k).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string)
}

fn num(v: &Value, k: &str) -> Option<f64> {
    v.get(k).and_then(Value::as_f64).filter(|n| n.is_finite())
}

/// A time in epoch seconds; one sent in milliseconds is brought down.
fn time(v: &Value, k: &str) -> Option<f64> {
    num(v, k).filter(|t| *t > 0.).map(|t| if t > 1e12 { t / 1000. } else { t })
}

fn list<'a>(v: &'a Value, k: &str) -> &'a [Value] {
    v.get(k).and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[])
}

fn read_device(v: &Value) -> Option<Device> {
    let id = text(v, "id")?;
    Some(Device {
        name: text(v, "name").unwrap_or_else(|| "Unnamed device".into()),
        kind: text(v, "kind").unwrap_or_else(|| "computer".into()),
        machine: text(v, "machine"),
        paired: time(v, "paired").unwrap_or(0.),
        via: text(v, "via").unwrap_or_default(),
        last_seen: time(v, "last_seen"),
        path: Path::read(v.get("path")),
        id,
    })
}

/// `{"devices": [...]}`, from the server's `/api/keys/devices` or the node's `/devices`.
pub fn read_devices(v: &Value) -> Option<Vec<Device>> {
    v.get("devices")?.as_array().map(|a| a.iter().filter_map(read_device).collect())
}

/// The node's `GET /roster`: the home computer's list of devices as this computer holds it.
pub fn read_held(v: &Value) -> HeldList {
    let r = v.get("roster").filter(|r| r.is_object());
    let devices = v
        .get("devices")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|d| {
                    let flag = |k: &str| d.get(k).and_then(Value::as_bool) == Some(true);
                    Some(HeldDevice {
                        id: text(d, "id")?,
                        name: text(d, "name").unwrap_or_default(),
                        phone: text(d, "kind").as_deref() == Some("phone"),
                        home: flag("home"),
                        here: flag("this"),
                        blocked: flag("blocked"),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    HeldList { at: r.and_then(|r| time(r, "at")), expires: r.and_then(|r| time(r, "expires")), devices }
}

/// `{"code", "link", "expires"}`, from the server's `/api/pair_device` or the node's `/pair/new`.
pub fn read_offer(v: &Value) -> Option<PairOffer> {
    let (code, link) = (text(v, "code"), text(v, "link"));
    // The code is null when home has no relay; the link alone still pairs.
    (code.is_some() || link.is_some()).then(|| PairOffer {
        code: code.unwrap_or_default(),
        link: link.unwrap_or_default(),
        expires: time(v, "expires").unwrap_or(0.),
    })
}

/// The node's `/status`.
pub fn read_status(v: &Value) -> NodeStatus {
    let home = v.get("home").filter(|h| h.is_object()).and_then(|h| {
        let (id, name) = (text(h, "id"), text(h, "name"));
        (id.is_some() || name.is_some()).then(|| Home {
            id: id.unwrap_or_default(),
            name: name.unwrap_or_default(),
            // A path counts only while connected (a node that says nothing about `connected` is taken at its word).
            path: Path::read(h.get("path")).filter(|_| h.get("connected").and_then(Value::as_bool) != Some(false)),
            error: text(h, "error"),
            refused: h.get("refused").and_then(Value::as_bool) == Some(true),
        })
    });
    let role = match v.get("role").and_then(Value::as_str) {
        Some("home") => Role::Home,
        Some("member") => Role::Member,
        _ => Role::Alone,
    };
    let keys = match v.get("keys") {
        Some(Value::Bool(b)) => *b,
        Some(Value::String(s)) => !matches!(s.as_str(), "off" | "false" | ""),
        // A node that says its role or its home and nothing about keys has them on.
        _ => role != Role::Alone || home.is_some(),
    };
    let relay = match v.get("relay") {
        Some(Value::String(s)) if !s.trim().is_empty() => Some(s.trim().to_string()),
        Some(r @ Value::Object(_)) => text(r, "url"),
        _ => None,
    };
    let peers = list(v, "peers")
        .iter()
        .filter(|p| p.is_object())
        .map(|p| Peer {
            id: text(p, "id").unwrap_or_default(),
            name: text(p, "name").unwrap_or_default(),
            path: Path::read(p.get("path")),
            rtt_ms: num(p, "rtt_ms").or_else(|| num(p, "rtt")).filter(|r| *r >= 0.),
        })
        .collect();
    let problems = list(v, "problems")
        .iter()
        .filter_map(|p| match p {
            Value::String(s) if !s.trim().is_empty() => Some(Problem { kind: None, text: s.trim().to_string() }),
            Value::Object(_) => {
                let words = text(p, "text").or_else(|| text(p, "message"))?;
                Some(Problem { kind: text(p, "kind"), text: words })
            }
            _ => None,
        })
        .collect();
    let row = v.get("relay").filter(|r| r.is_object());
    NodeStatus {
        keys,
        role,
        machine: text(v, "machine"),
        id: text(v, "id"),
        home,
        relay,
        relay_source: row.and_then(|r| text(r, "source")),
        relay_connected: row.and_then(|r| r.get("connected")).and_then(Value::as_bool),
        addrs: list(v, "addrs").iter().filter_map(|a| a.as_str()).map(str::to_string).collect(),
        forwards: list(v, "forwarders")
            .iter()
            .filter_map(|f| {
                Some(Forward {
                    target: text(f, "target")?,
                    addr: text(f, "addr")?,
                    ok: f.get("ok").and_then(Value::as_bool) != Some(false),
                })
            })
            .collect(),
        peers,
        problems,
    }
}

/// The node's answer to `/pair/join`.
pub fn read_joined(v: &Value) -> Joined {
    let home = v.get("home").and_then(|h| text(h, "name")).or_else(|| text(v, "home_name"));
    Joined { home, machine: text(v, "machine") }
}

/// The server's `/api/keys/readiness`.
pub fn read_readiness(v: &Value) -> Readiness {
    let blockers = list(v, "blockers")
        .iter()
        .filter(|b| b.is_object())
        .map(|b| Blocker {
            kind: text(b, "kind").unwrap_or_default(),
            text: text(b, "text").unwrap_or_default(),
            label: text(b, "label").or_else(|| text(b, "machine")),
            paired: b.get("paired").and_then(Value::as_bool),
            client: text(b, "client"),
            last: time(b, "last"),
            watched_days: b.get("watched_days").and_then(Value::as_u64),
            paired_since: b["paired_since"] == true,
        })
        .collect::<Vec<_>>();
    let machines = list(v, "machines")
        .iter()
        .filter_map(|m| {
            let name = text(m, "name")?;
            Some(ReadyMachine {
                label: text(m, "label").unwrap_or_else(|| name.clone()),
                home: m.get("home").and_then(Value::as_bool) == Some(true),
                paired: m.get("paired").and_then(Value::as_bool) == Some(true),
                seen: time(m, "seen"),
                path: Path::read(m.get("path")),
                name,
            })
        })
        .collect();
    Readiness {
        keys: Mode::read(v.get("keys")),
        // Ready only when the server says so and lists nothing in the way.
        ready: v.get("ready").and_then(Value::as_bool) == Some(true) && blockers.is_empty(),
        days: v.get("days").and_then(Value::as_u64).unwrap_or(7),
        blockers,
        machines,
        switch: v.get("switch").and_then(Value::as_bool) == Some(true),
        here: v.get("here").and_then(Value::as_bool) == Some(true),
    }
}

// ---------------------------------------------------------------------------------------------------------------
// The node's control API
// ---------------------------------------------------------------------------------------------------------------

/// Where the node's control API listens on this computer.
pub const CONTROL_PORT: u16 = 8137;
/// Where the node forwards this computer's own port to home once it has joined (section 3.1), unless it was
/// started with another (`home_here` reads the one it says).
pub const HOME_HERE: &str = "http://127.0.0.1:8130";
/// After a join the node starts keys again, and for a moment says they are not running: its status is asked
/// this many times, this far apart, before the usual port is assumed.
pub const HOME_HERE_TRIES: u32 = 24;
pub const HOME_HERE_PAUSE: Duration = Duration::from_millis(250);

/// The address on this computer that leads to the home computer's server, as the node's status lists it
/// ("http://127.0.0.1:8130"). None when it lists none.
pub fn home_here(n: &NodeStatus) -> Option<String> {
    n.forwards
        .iter()
        .filter(|f| f.target == "api")
        .filter_map(|f| f.addr.parse::<std::net::SocketAddr>().ok())
        .find(|a| a.ip().is_loopback() && a.port() != 0)
        .map(|a| format!("http://{a}"))
}

/// Joins a home (`NodeControl::join`) and then asks the node where on this computer it carries the home
/// computer's server, so the app connects to the port the node really opened. `HOME_HERE` only when the node
/// runs and lists none, or has not said after `tries` asks `pause` apart. Blocks; call it off the UI thread.
pub fn join_home(
    node: &dyn NodeControl,
    input: &str,
    name: Option<&str>,
    addrs: &[String],
    replace: bool,
    tries: u32,
    pause: Duration,
) -> Result<(Joined, String), NodeError> {
    let joined = node.join(input, name, addrs, replace)?;
    for n in 0..tries.max(1) {
        if n > 0 {
            std::thread::sleep(pause);
        }
        match node.status() {
            Ok(s) if home_here(&s).is_some() || s.role == Role::Member => {
                return Ok((joined, home_here(&s).unwrap_or_else(|| HOME_HERE.to_string())));
            }
            _ => {}
        }
    }
    Ok((joined, HOME_HERE.to_string()))
}

#[derive(Debug, Clone, PartialEq)]
pub enum NodeError {
    /// Nothing answers on the control port: no node here, or one from before keys.
    Absent,
    /// The node answered no. `kind` is its short word for why, when it gave one.
    Said { status: u16, kind: Option<String>, message: String },
    /// It took too long.
    Timeout,
    /// The answer was not what a node sends.
    Bad(String),
}

impl std::fmt::Display for NodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NodeError::Absent => write!(f, "The Switchboard node on this computer isn't answering."),
            NodeError::Said { message, .. } => write!(f, "{message}"),
            NodeError::Timeout => write!(f, "The Switchboard node on this computer took too long to answer."),
            NodeError::Bad(m) => write!(f, "{m}"),
        }
    }
}

impl std::error::Error for NodeError {}

/// The node's control API on this computer. Every call blocks; call it off the UI thread.
pub trait NodeControl: Send + Sync {
    /// `GET /status`.
    fn status(&self) -> Result<NodeStatus, NodeError>;
    /// `POST /pair/join {"input": "<code or link>", "name"?, "addrs"?}`: join a home from this computer. `name` is
    /// what this computer is called in the home computer's list; `addrs` are addresses to try for home, for a link
    /// that names no relay.
    /// `replace`: this computer may leave the home computer it is paired with for the one `input` leads to.
    /// Without it the node turns such a join down (kind "other_home") and changes nothing.
    fn join(&self, input: &str, name: Option<&str>, addrs: &[String], replace: bool) -> Result<Joined, NodeError>;
    /// `GET /devices` (home only).
    fn devices(&self) -> Result<Vec<Device>, NodeError>;
    /// `POST /devices/remove {"id"}` (home only).
    fn remove_device(&self, id: &str) -> Result<(), NodeError>;
    /// `POST /pair/new` (home only).
    fn pair_new(&self) -> Result<PairOffer, NodeError>;
    /// `POST /relay {"url"}`.
    fn set_relay(&self, url: &str) -> Result<(), NodeError>;
    /// `GET /roster`: the home computer's list of devices as this computer holds it.
    fn held(&self) -> Result<HeldList, NodeError>;
    /// `POST /devices/block {"id"}` or `/devices/unblock` (any computer but home): stop answering a device
    /// here, or answer it again.
    fn block(&self, id: &str, block: bool) -> Result<(), NodeError>;
}

/// The control API over HTTP on loopback.
///
/// The shapes are sb-link's `control.rs` (mirrored here: the desktop app does not link sb-link, which brings the
/// whole endpoint with it). `read_*` above is the one place that names the fields, and it is lenient about them.
///
/// - `GET /status` -> `{"ok", "version", "role": "home" | "member", "id", "name", "machine": "<name>" | null,
///   "relay": {"url", "source", "connected", "error"?, "same_id"}, "home"?: {"id", "name", "connected",
///   "path": "direct" | "relay" | null, "rtt_ms", "error"?, "refused"}, "forwarders": [...], "devices": <count>,
///   "updates": [...], "addrs"?: ["ip:port"], "problems": ["<sentence>"]}`. 503 with `{"ok": false, "error",
///   "kind": "off" | "not_running"}` while keys are not running: the node is there, and this computer can join.
/// - `POST /pair/join {"input": "<code or link>", "name"?, "addrs"?: ["ip:port"]}` -> `{"ok": true, "home": {"id",
///   "name"}, "machine"}`; 400 for input that is neither, 409 when pairing failed. It answers when pairing is over
///   (up to `JOIN_WAIT`).
/// - `GET /devices` -> `{"ok": true, "devices": [{"id", "name", "kind", "machine", "paired", "via": "code" | "link"
///   | "upgrade", "last_seen", "path": "direct" | "relay" | null, "rtt_ms"?}]}`
/// - `POST /devices/remove {"id"}` -> `{"ok": true}`
/// - `POST /pair/new {}` -> `{"ok": true, "code": "XXXX-XXXX" | null, "link", "expires"}` (no code without a relay)
/// - `POST /relay {"url"}` -> `{"ok": true}` ("" clears it)
/// - Any failure: a status that is not 2xx with `{"ok": false, "error": "<sentence>", "kind": "<one word>"}`. The
///   join screen goes by the kind (`join_problem`), and reads the sentence only from a node that sent none.
/// - The node answers only requests from this computer, with a loopback Host, no Origin, and JSON on POST.
pub struct HttpNode {
    base: String,
}

/// How long a join may take: the slow rendezvous hash, the dial through the relay and the exchange.
pub const JOIN_WAIT: Duration = Duration::from_secs(90);
const QUICK_WAIT: Duration = Duration::from_secs(3);
const CALL_WAIT: Duration = Duration::from_secs(12);

/// Whether `base` is plain HTTP on this computer. The control API is this computer's own, so an address that
/// points anywhere else is never called.
pub fn loopback_base(base: &str) -> bool {
    let Some(rest) = base.strip_prefix("http://") else { return false };
    let host = rest.split('/').next().unwrap_or("");
    let host = match host.rsplit_once(':') {
        Some((h, port)) if !h.is_empty() && port.bytes().all(|b| b.is_ascii_digit()) => h,
        _ => host,
    };
    let host = host.trim_start_matches('[').trim_end_matches(']');
    host == "localhost" || host.parse::<std::net::IpAddr>().is_ok_and(|ip| ip.is_loopback())
}

impl HttpNode {
    /// The node on this computer: `SWITCHBOARD_NODE_CONTROL` when it names a loopback address (tests and
    /// screenshots), else `127.0.0.1:8137`.
    pub fn local() -> HttpNode {
        let own = format!("http://127.0.0.1:{CONTROL_PORT}");
        let base = std::env::var("SWITCHBOARD_NODE_CONTROL")
            .ok()
            .map(|s| s.trim().trim_end_matches('/').to_string())
            .filter(|s| loopback_base(s))
            .unwrap_or(own);
        HttpNode { base }
    }

    /// A control API at `base`. None when `base` is not on this computer.
    pub fn at(base: &str) -> Option<HttpNode> {
        let base = base.trim().trim_end_matches('/').to_string();
        loopback_base(&base).then_some(HttpNode { base })
    }

    fn agent(wait: Duration) -> ureq::Agent {
        let cfg = ureq::Agent::config_builder()
            .timeout_global(Some(wait))
            .timeout_connect(Some(wait.min(Duration::from_secs(2))))
            .http_status_as_error(false)
            // Straight to this computer, whatever proxy the environment names.
            .proxy(None)
            .build();
        ureq::Agent::new_with_config(cfg)
    }

    fn call(&self, method: &str, path: &str, body: Option<Value>, wait: Duration) -> Result<Value, NodeError> {
        let url = format!("{}{path}", self.base);
        let agent = Self::agent(wait);
        let sent = match body {
            Some(b) => agent.post(&url).header("Content-Type", "application/json").send_json(&b),
            None if method == "POST" => {
                agent.post(&url).header("Content-Type", "application/json").send_json(serde_json::json!({}))
            }
            None => agent.get(&url).call(),
        };
        let mut r = sent.map_err(|e| unanswered(&e))?;
        let status = r.status().as_u16();
        let raw = r.body_mut().with_config().limit(1_000_000).read_to_vec().map_err(|e| unanswered(&e))?;
        read_answer(status, &raw)
    }
}

/// Why a call got no answer: nothing listens (`Absent`), or it was slow.
fn unanswered(e: &ureq::Error) -> NodeError {
    match e {
        ureq::Error::Timeout(t) if !matches!(t, ureq::Timeout::Resolve | ureq::Timeout::Connect) => NodeError::Timeout,
        ureq::Error::Io(e) if e.kind() == std::io::ErrorKind::TimedOut => NodeError::Timeout,
        ureq::Error::BodyExceedsLimit(_) => NodeError::Bad("The node's answer was too big to read.".into()),
        _ => NodeError::Absent,
    }
}

/// A control API answer as JSON, or the node's own words for why not.
pub fn read_answer(status: u16, raw: &[u8]) -> Result<Value, NodeError> {
    let v: Option<Value> = serde_json::from_slice(raw).ok().filter(Value::is_object);
    let refused = v.as_ref().and_then(|v| v.get("ok")).and_then(Value::as_bool) == Some(false);
    if (200..300).contains(&status) && !refused {
        return v.ok_or_else(|| NodeError::Bad("The node's answer could not be read.".into()));
    }
    // 404 with no JSON: something else holds the port, or a node from before this route.
    let Some(v) = v else {
        return Err(if status == 404 {
            NodeError::Absent
        } else {
            NodeError::Said { status, kind: None, message: format!("The node answered {status}.") }
        });
    };
    Err(NodeError::Said {
        status,
        kind: text(&v, "kind").or_else(|| text(&v, "reason")),
        message: text(&v, "error").unwrap_or_else(|| format!("The node answered {status}.")),
    })
}

/// What `GET /status` came to. A 503 in the node's own words is a node that is there with keys not running: a
/// computer that has not joined a home (it can), or one whose keys could not start (the sentence says why).
pub fn status_from(answer: Result<Value, NodeError>) -> Result<NodeStatus, NodeError> {
    match answer {
        Ok(v) => Ok(read_status(&v)),
        Err(NodeError::Said { status: 503, kind, message }) => Ok(NodeStatus {
            keys: true,
            role: Role::Alone,
            problems: vec![Problem { kind, text: message }],
            ..Default::default()
        }),
        Err(e) => Err(e),
    }
}

impl NodeControl for HttpNode {
    fn status(&self) -> Result<NodeStatus, NodeError> {
        status_from(self.call("GET", "/status", None, QUICK_WAIT))
    }

    fn join(&self, input: &str, name: Option<&str>, addrs: &[String], replace: bool) -> Result<Joined, NodeError> {
        self.call("POST", "/pair/join", Some(join_body(input, name, addrs, replace)), JOIN_WAIT)
            .map(|v| read_joined(&v))
    }

    fn devices(&self) -> Result<Vec<Device>, NodeError> {
        let v = self.call("GET", "/devices", None, CALL_WAIT)?;
        read_devices(&v).ok_or_else(|| NodeError::Bad("The node's answer could not be read.".into()))
    }

    fn remove_device(&self, id: &str) -> Result<(), NodeError> {
        self.call("POST", "/devices/remove", Some(serde_json::json!({ "id": id })), CALL_WAIT).map(|_| ())
    }

    fn pair_new(&self) -> Result<PairOffer, NodeError> {
        let v = self.call("POST", "/pair/new", None, CALL_WAIT)?;
        read_offer(&v).ok_or_else(|| NodeError::Bad("The node's answer could not be read.".into()))
    }

    fn set_relay(&self, url: &str) -> Result<(), NodeError> {
        self.call("POST", "/relay", Some(serde_json::json!({ "url": url })), CALL_WAIT).map(|_| ())
    }

    fn held(&self) -> Result<HeldList, NodeError> {
        self.call("GET", "/roster", None, CALL_WAIT).map(|v| read_held(&v))
    }

    fn block(&self, id: &str, block: bool) -> Result<(), NodeError> {
        let path = if block { "/devices/block" } else { "/devices/unblock" };
        self.call("POST", path, Some(serde_json::json!({ "id": id })), CALL_WAIT).map(|_| ())
    }
}

/// What `/pair/join` is sent: the code or link, this computer's name when there is one to give, and addresses
/// for home when there are any.
pub fn join_body(input: &str, name: Option<&str>, addrs: &[String], replace: bool) -> Value {
    let mut body = serde_json::json!({ "input": input });
    if replace {
        body["replace"] = Value::Bool(true);
    }
    if let Some(n) = name.map(str::trim).filter(|n| !n.is_empty()) {
        body["name"] = Value::String(n.to_string());
    }
    if !addrs.is_empty() {
        body["addrs"] = Value::Array(addrs.iter().map(|a| Value::String(a.clone())).collect());
    }
    body
}

/// This computer's name as the system has it ("Sams-Netbook-Pro"), for the home computer's list. None when the
/// system gives none: the node then uses its own.
pub fn this_computer() -> Option<String> {
    let out = std::process::Command::new("hostname").output().ok()?;
    computer_name(&String::from_utf8_lossy(&out.stdout))
}

/// A host name as a computer's name: its first part, when that says anything.
pub fn computer_name(raw: &str) -> Option<String> {
    let name = raw.trim().split('.').next().unwrap_or("").trim().to_string();
    let plain = !name.is_empty() && name.len() <= 63 && !name.chars().any(char::is_control);
    (plain && !name.eq_ignore_ascii_case("localhost")).then_some(name)
}

/// A node that lives in memory, for tests: what it answers is set on `state`, and what it was asked is in
/// `state.calls`.
#[derive(Default)]
pub struct FakeNode {
    pub state: Mutex<FakeState>,
}

#[derive(Default)]
pub struct FakeState {
    /// None: nothing answers (`NodeError::Absent`).
    pub status: Option<NodeStatus>,
    pub devices: Vec<Device>,
    pub offer: Option<PairOffer>,
    /// What the next joins answer, first in first out. Empty: the join works.
    pub joins: Vec<Result<Joined, NodeError>>,
    pub relay: Option<String>,
    /// Every call, as "status", "join ABCD-EFGH", "remove <id>", "pair_new", "relay <url>", "devices", "held",
    /// "block <id>", "unblock <id>".
    pub calls: Vec<String>,
    /// The list of devices this computer holds.
    pub held: HeldList,
    /// What each join was sent, as `join_body` makes it.
    pub join_bodies: Vec<Value>,
    /// What `status` answers once a join has worked, in this order; the last one stays. Empty: `status` as it was.
    pub after_join: Vec<NodeStatus>,
    /// A join has worked.
    pub joined: bool,
}

impl FakeNode {
    pub fn absent() -> FakeNode {
        FakeNode::default()
    }

    pub fn with(status: NodeStatus) -> FakeNode {
        FakeNode { state: Mutex::new(FakeState { status: Some(status), ..Default::default() }) }
    }

    fn state(&self) -> std::sync::MutexGuard<'_, FakeState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Notes the call; Err when nothing answers.
    fn asked(&self, call: String) -> Result<std::sync::MutexGuard<'_, FakeState>, NodeError> {
        let mut s = self.state();
        s.calls.push(call);
        if s.status.is_none() { Err(NodeError::Absent) } else { Ok(s) }
    }
}

impl NodeControl for FakeNode {
    fn status(&self) -> Result<NodeStatus, NodeError> {
        let mut s = self.asked("status".into())?;
        if s.joined && !s.after_join.is_empty() {
            let next = if s.after_join.len() > 1 { s.after_join.remove(0) } else { s.after_join[0].clone() };
            s.status = Some(next);
        }
        Ok(s.status.clone().unwrap_or_default())
    }

    fn join(&self, input: &str, name: Option<&str>, addrs: &[String], replace: bool) -> Result<Joined, NodeError> {
        let mut s = self.asked(format!("join {input}"))?;
        s.join_bodies.push(join_body(input, name, addrs, replace));
        let res = if s.joins.is_empty() {
            Ok(Joined { home: s.status.as_ref().and_then(|x| x.home.as_ref()).map(|h| h.name.clone()), machine: None })
        } else {
            s.joins.remove(0)
        };
        s.joined |= res.is_ok();
        res
    }

    fn devices(&self) -> Result<Vec<Device>, NodeError> {
        self.asked("devices".into()).map(|s| s.devices.clone())
    }

    fn remove_device(&self, id: &str) -> Result<(), NodeError> {
        let mut s = self.asked(format!("remove {id}"))?;
        s.devices.retain(|d| d.id != id);
        Ok(())
    }

    fn pair_new(&self) -> Result<PairOffer, NodeError> {
        let s = self.asked("pair_new".into())?;
        s.offer.clone().ok_or_else(|| NodeError::Said {
            status: 502,
            kind: None,
            message: "This computer is not a home computer.".into(),
        })
    }

    fn set_relay(&self, url: &str) -> Result<(), NodeError> {
        let mut s = self.asked(format!("relay {url}"))?;
        s.relay = Some(url.to_string());
        Ok(())
    }

    fn held(&self) -> Result<HeldList, NodeError> {
        self.asked("held".into()).map(|s| s.held.clone())
    }

    fn block(&self, id: &str, block: bool) -> Result<(), NodeError> {
        let mut s = self.asked(format!("{} {id}", if block { "block" } else { "unblock" }))?;
        match s.held.devices.iter_mut().find(|d| d.id == id) {
            Some(d) if d.can_block() => {
                d.blocked = block;
                Ok(())
            }
            Some(_) => {
                Err(NodeError::Said { status: 409, kind: None, message: "The home computer cannot be blocked.".into() })
            }
            None => Err(NodeError::Said {
                status: 409,
                kind: None,
                message: "No such device is on the home computer's list.".into(),
            }),
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// What is on and what shows
// ---------------------------------------------------------------------------------------------------------------

/// What the app knows about keys: the server's report and this computer's node. Both None is every install
/// from before keys, and then nothing of keys shows anywhere.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Keys {
    /// `/api/keys/readiness`, when the server has the route.
    pub server: Option<Readiness>,
    /// The node on this computer, when its control API answers.
    pub node: Option<NodeStatus>,
}

impl Keys {
    /// The server in use connects devices with keys: Devices shows in Settings.
    pub fn on(&self) -> bool {
        self.server.as_ref().is_some_and(|r| r.keys != Mode::Off)
    }

    /// This computer can join a home with a code: it has a node with keys on, and it is not a home itself.
    pub fn can_join(&self) -> bool {
        self.node.as_ref().is_some_and(|n| n.keys && n.role != Role::Home)
    }

    /// This computer has joined a home: its node carries this computer's own port there (`HOME_HERE`). Not when
    /// home has since removed it: then it has to join again.
    pub fn joined(&self) -> bool {
        self.node
            .as_ref()
            .is_some_and(|n| n.keys && n.role == Role::Member && n.home.as_ref().is_some_and(|h| !h.refused))
    }

    /// The first run leads with "type a code": this computer can join a home and has not joined one.
    pub fn offer_join(&self) -> bool {
        self.can_join() && !self.joined()
    }

    /// Nothing of keys is in use: the app looks exactly as it did before them.
    pub fn hidden(&self) -> bool {
        !self.on() && !self.can_join() && self.node.as_ref().is_none_or(|n| !n.keys)
    }

    /// The server answers paired devices only.
    pub fn only(&self) -> bool {
        self.server.as_ref().is_some_and(|r| r.keys == Mode::Only)
    }

    /// Keys are off and this app can turn them on: the server has the route for it, and the app runs on the home
    /// computer itself. Devices then shows "Turn keys on". Never with a server from before the route.
    pub fn can_turn_on(&self) -> bool {
        self.server.as_ref().is_some_and(|r| r.keys == Mode::Off && r.switch && r.here)
    }

    /// Devices is in Settings: the server connects devices with keys, or this app can turn them on, or this
    /// computer has joined a home (the page is then also where it blocks a device while home is away, when
    /// the server cannot be asked anything).
    pub fn devices_page(&self) -> bool {
        self.on() || self.can_turn_on() || self.joined()
    }

    /// This computer is the home computer and its node runs: the relay it uses is this app's to show and change.
    pub fn home_here(&self) -> bool {
        self.node.as_ref().is_some_and(|n| n.keys && n.role == Role::Home)
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Turning keys on, and the answers of the route that changes the mode
// ---------------------------------------------------------------------------------------------------------------

/// Under the Devices heading: what the page lists, or while keys are off, what it will list once they are on.
pub const DEVICES_INTRO: &str = "The computers and phones paired with your home computer. Each one has its own key, so it connects from any network without an address.";
pub const DEVICES_INTRO_OFF: &str = "Keys are off, so nothing is paired yet. Once they are on, the computers and phones you pair with your home computer are listed here.";

/// Under the heading on a computer that joined a home, while the home computer cannot be asked for its list.
pub const DEVICES_INTRO_AWAY: &str = "The home computer keeps the list of devices, and it is not answering just now. What this computer holds of it is below.";

/// The line under the Devices heading.
pub fn devices_intro(keys_on: bool) -> &'static str {
    if keys_on { DEVICES_INTRO } else { DEVICES_INTRO_OFF }
}

/// The card in Devices' place while keys are off, on the home computer.
pub const TURN_ON_TITLE: &str = "Connect your computers with keys";
pub const TURN_ON_BODY: &str = "Each computer pairs with this home computer once, with a code, and then connects with its own key from any network, with no address to set up. Connecting by address keeps working, and you can turn keys off again.";
pub const TURN_ON_BUTTON: &str = "Turn keys on";
pub const TURN_ON_BUSY: &str = "Turning keys on";
/// Said while the server restarts with the new setting.
pub const RESTARTING: &str = "Switchboard is restarting on the home computer. This takes a few seconds.";

/// The server said no to a change of mode (its 409), in its own words.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ModeRefusal {
    /// What to show: the server's sentence, without the command when there is one (that is shown beside it).
    pub text: String,
    /// The command to run on the home computer instead, when the server cannot restart itself.
    pub command: Option<String>,
    /// Everything keys only would cut off, one sentence each.
    pub blockers: Vec<String>,
}

/// What `POST /api/keys/mode` answered: Ok(restarting) for its 200, Err for its 409. None for anything else
/// (the caller says that in the usual way).
pub fn read_mode_answer(status: u16, raw: &[u8]) -> Option<Result<bool, ModeRefusal>> {
    let v: Value = serde_json::from_slice(raw).ok().filter(Value::is_object)?;
    if (200..300).contains(&status) && v.get("ok").and_then(Value::as_bool) != Some(false) {
        return Some(Ok(v.get("restarting").and_then(Value::as_bool) == Some(true)));
    }
    if status != 409 {
        return None;
    }
    let message = text(&v, "error")?;
    let blockers: Vec<String> = list(&v, "blockers").iter().filter_map(|b| text(b, "text")).collect();
    if !blockers.is_empty() {
        // The sentence ends with the first of them: say the lead once and list them all.
        let lead = message.strip_suffix(blockers[0].as_str()).map(str::trim).filter(|l| !l.is_empty());
        return Some(Err(ModeRefusal { text: lead.unwrap_or(&message).to_string(), command: None, blockers }));
    }
    let command = text(&v, "command").or_else(|| {
        // An earlier server put the command in the sentence only.
        message.rsplit_once(": ").map(|(_, c)| c.to_string()).filter(|c| c.starts_with("switchboard "))
    });
    let said = match &command {
        Some(c) => message.strip_suffix(c.as_str()).map(|m| m.trim().to_string()).unwrap_or(message),
        None => message,
    };
    Some(Err(ModeRefusal { text: said, command, blockers }))
}

// ---------------------------------------------------------------------------------------------------------------
// The relay, on the home computer
// ---------------------------------------------------------------------------------------------------------------

/// The line in Devices that says which relay the home computer uses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelayLine {
    /// What a relay is for, in one short line: the card leads with it.
    pub label: String,
    /// "relay.example.com", or "None set". Shown second, and quieter.
    pub value: String,
    /// Where it comes from, and whether it is connected.
    pub note: String,
    pub tone: Tone,
    /// Whether changing it here would take: not when the place Switchboard starts from names it.
    pub can_change: bool,
}

/// What Change is for, under its field.
pub const RELAY_HINT: &str =
    "The relay cannot read anything sent through it. Leave the field empty to use none of your own.";
/// What a relay is for: the card's first line, with a relay and without one.
pub const RELAY_LABEL: &str = "The relay helps computers on other networks find this one.";
pub const RELAY_LABEL_NONE: &str = "A relay helps computers on other networks find this one.";
pub const RELAY_PLACEHOLDER: &str = "https://relay.example.com";

/// A relay's address as it is shown: without the scheme and a trailing slash.
pub fn relay_shown(url: &str) -> String {
    let u = url.trim().trim_end_matches('/');
    u.strip_prefix("https://").or_else(|| u.strip_prefix("http://")).unwrap_or(u).to_string()
}

/// The relay line, on a home computer whose node runs. None anywhere else.
pub fn relay_line(n: &NodeStatus) -> Option<RelayLine> {
    if !n.keys || n.role != Role::Home {
        return None;
    }
    let source = n.relay_source.as_deref().unwrap_or("");
    let can_change = source != "environment";
    let Some(url) = n.relay.as_deref() else {
        return Some(RelayLine {
            label: RELAY_LABEL_NONE.into(),
            value: "None set".into(),
            note: "Short codes need a relay. Without one, a computer pairs with the link and this computer's address."
                .into(),
            tone: Tone::Quiet,
            can_change,
        });
    };
    let from = match source {
        "settings" => "Set on this computer",
        "environment" => "Set where Switchboard starts, so it is changed there",
        "built in" => "The one Switchboard comes with",
        "home" => "From the home computer",
        _ => "",
    };
    let down = n.relay_connected == Some(false);
    let note = match (from, down) {
        ("", false) => "Connected".to_string(),
        ("", true) => "Not connected".to_string(),
        (f, false) => f.to_string(),
        (f, true) => format!("{f}. Not connected"),
    };
    Some(RelayLine {
        label: RELAY_LABEL.into(),
        value: relay_shown(url),
        note,
        tone: if down { Tone::Bad } else { Tone::Quiet },
        can_change,
    })
}

/// What was typed into the relay field, as it is sent: "" clears it. Err says what is wrong with it.
pub fn relay_input(typed: &str) -> Result<String, String> {
    let t = typed.trim();
    if t.is_empty() {
        return Ok(String::new());
    }
    let url = if t.contains("://") { t.to_string() } else { format!("https://{t}") };
    let rest = url.strip_prefix("https://").or_else(|| url.strip_prefix("http://"));
    let host = rest.map(|r| r.split('/').next().unwrap_or("")).unwrap_or("");
    if host.is_empty() || host.contains(char::is_whitespace) || !host.contains(|c: char| c.is_ascii_alphanumeric()) {
        return Err("That isn't a relay address. It looks like https://relay.example.com".into());
    }
    Ok(url)
}

/// The address a computer on the same network would type for this home computer: the first that is not this
/// computer alone, IPv4 before IPv6. None when the node listed none.
pub fn home_address(n: &NodeStatus) -> Option<String> {
    let usable = |a: &&String| {
        a.parse::<std::net::SocketAddr>().is_ok_and(|s| !s.ip().is_loopback() && !s.ip().is_unspecified())
    };
    let v4 = n.addrs.iter().filter(usable).find(|a| a.parse::<std::net::SocketAddr>().is_ok_and(|s| s.is_ipv4()));
    v4.or_else(|| n.addrs.iter().find(usable)).cloned()
}

/// What was typed into the join screen's address field, as the node takes it ("192.0.2.20:41641"). Err says
/// what is wrong with it.
pub fn address_input(typed: &str) -> Result<String, String> {
    let t = typed.trim();
    match t.parse::<std::net::SocketAddr>() {
        Ok(a) if a.port() != 0 => Ok(a.to_string()),
        _ if t.is_empty() => {
            Err("Type your home computer's address. It is shown there, in Settings, under Devices.".into())
        }
        _ => Err("That isn't an address with a port. It looks like 192.0.2.20:41641".into()),
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Devices
// ---------------------------------------------------------------------------------------------------------------

/// A device seen this recently is connected now (the node notes one every few seconds while its link is up).
pub const CONNECTED_WITHIN: f64 = 90.;

/// One line of the Devices list.
#[derive(Debug, Clone, PartialEq)]
pub struct DeviceRow {
    pub id: String,
    pub name: String,
    /// "Computer" or "Phone".
    pub kind: &'static str,
    pub phone: bool,
    /// "Paired 3d ago with a code".
    pub paired: String,
    /// "Connected", "Last seen 2h ago" or "Not connected yet".
    pub seen: String,
    pub connected: bool,
    /// "Direct" or "Through the relay", while it is connected.
    pub path: Option<&'static str>,
    /// The computer this app runs on.
    pub here: bool,
}

/// The Devices list, connected ones first and then the most recently seen. `own`: this computer's device id and
/// its name in the list of computers, from its node.
pub fn device_rows(devices: &[Device], now: f64, own: Option<&NodeStatus>) -> Vec<DeviceRow> {
    let own_id = own.and_then(|n| n.id.as_deref());
    let own_machine = own.filter(|n| n.role == Role::Member).and_then(|n| n.machine.as_deref());
    let mut rows: Vec<(f64, DeviceRow)> = devices
        .iter()
        .map(|d| {
            let phone = d.kind == "phone";
            // The home computer names a path only while the connection is up, and a connection can be quiet.
            let connected = d.path.is_some() || d.last_seen.is_some_and(|t| now - t < CONNECTED_WITHIN);
            let how = match d.via.as_str() {
                "code" => " with a code",
                "link" => " with a link",
                "upgrade" => " automatically",
                _ => "",
            };
            let paired =
                if d.paired > 0. { format!("Paired {}{how}", fmt::ago(d.paired, now)) } else { "Paired".to_string() };
            let seen = match d.last_seen {
                _ if connected => "Connected".to_string(),
                Some(t) => format!("Last seen {}", fmt::ago(t, now)),
                None => "Not connected yet".to_string(),
            };
            let here = own_id.is_some_and(|id| id == d.id)
                || (own_id.is_none() && !phone && own_machine.is_some() && d.machine.as_deref() == own_machine);
            let row = DeviceRow {
                id: d.id.clone(),
                name: d.name.clone(),
                kind: if phone { "Phone" } else { "Computer" },
                phone,
                paired,
                seen,
                connected,
                path: d.path.filter(|_| connected).map(Path::says),
                here,
            };
            (d.last_seen.unwrap_or(0.), row)
        })
        .collect();
    rows.sort_by(|a, b| b.1.connected.cmp(&a.1.connected).then(b.0.total_cmp(&a.0)).then(a.1.name.cmp(&b.1.name)));
    rows.into_iter().map(|(_, r)| r).collect()
}

/// What removing a device does, said before it is done.
pub fn remove_explains(row: &DeviceRow) -> String {
    let base = format!(
        "Removing {} turns its key off. It loses its connection to this home computer right away and needs a new \
         code to come back.",
        row.name
    );
    if row.here {
        format!("{base} This is the computer you are using, so this app disconnects too.")
    } else if row.phone {
        base
    } else {
        format!("{base} Chats running on it are not stopped.")
    }
}

/// "3 devices", "1 device", "No devices yet".
pub fn device_count(n: usize) -> String {
    match n {
        0 => "No devices yet".into(),
        1 => "1 device".into(),
        n => format!("{n} devices"),
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Add a device: the code, the link and the countdown
// ---------------------------------------------------------------------------------------------------------------

/// A code as it is shown, `ABCD-EFGH`: letters and digits only, upper case, in two halves.
pub fn group_code(code: &str) -> String {
    let c: Vec<char> = code.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_uppercase()).collect();
    if c.len() < 2 {
        return c.into_iter().collect();
    }
    let half = c.len().div_ceil(2);
    format!("{}-{}", c[..half].iter().collect::<String>(), c[half..].iter().collect::<String>())
}

/// Seconds until the offer runs out, never below zero.
pub fn offer_left(offer: &PairOffer, now: f64) -> f64 {
    (offer.expires - now).max(0.)
}

/// The countdown under the code: "Works once, for 9:41", then "Expired".
pub fn offer_line(left: f64) -> String {
    match offer_parts(left) {
        (words, Some(clock)) => format!("{words} {clock}"),
        (words, None) => words.to_string(),
    }
}

/// The countdown in its two parts, so the clock can be set in the number font and not jump about as it counts:
/// ("Works once, for", Some("9:41")), then ("Expired", None).
pub fn offer_parts(left: f64) -> (&'static str, Option<String>) {
    if left > 0. { ("Works once, for", Some(fmt::clock(left.ceil() as u64))) } else { ("Expired", None) }
}

/// What the Add a device card shows.
#[derive(Debug, Clone, PartialEq)]
pub enum OfferView {
    /// The code, with this many seconds left.
    Waiting(f64),
    /// The code ran out before anything joined.
    Expired,
    /// A device joined with it: its name.
    Paired(String),
}

/// The card's state: a device that joined wins over a code that has since run out.
pub fn offer_view(offer: &PairOffer, paired: Option<&str>, now: f64) -> OfferView {
    match (paired, offer_left(offer, now)) {
        (Some(name), _) => OfferView::Paired(name.to_string()),
        (None, left) if left > 0. => OfferView::Waiting(left),
        _ => OfferView::Expired,
    }
}

/// Said under the code: where to type it.
pub const OFFER_HOW: &str = "On the other computer, open Switchboard. It asks for this code the first time it starts. \
                             You can paste the link there instead.";

/// The device that was not there when the code was made: the one that just joined with it.
pub fn newly_paired<'a>(known: &[String], devices: &'a [Device]) -> Option<&'a Device> {
    devices.iter().filter(|d| !known.contains(&d.id)).max_by(|a, b| a.paired.total_cmp(&b.paired))
}

// ---------------------------------------------------------------------------------------------------------------
// Join a home from this computer
// ---------------------------------------------------------------------------------------------------------------

/// The letters a code can have (sb-link code.rs): no 0, O, 1, I or L.
pub const CODE_ALPHABET: &str = "ABCDEFGHJKMNPQRSTUVWXYZ23456789";
pub const CODE_LEN: usize = 8;
pub const LINK_PREFIX: &str = "switchboard://pair/1?";

/// Why what was typed cannot be a code or a link, found before anything is sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputProblem {
    Empty,
    /// How many letters and digits were typed, when it is not 8.
    Length(usize),
    /// A character no code has.
    Letter(char),
    /// It has a colon, so it is not a code, and it is not a pairing link either.
    NotALink,
}

impl InputProblem {
    pub fn says(&self) -> String {
        match self {
            InputProblem::Empty => "Type the code from your home computer, or paste its link.".into(),
            InputProblem::Length(n) => {
                format!("A code has {CODE_LEN} letters and digits. That was {n}.")
            }
            InputProblem::Letter(c) if matches!(c.to_ascii_uppercase(), '0' | 'O' | '1' | 'I' | 'L') => {
                format!("Codes never use 0, O, 1, I or L. Check the \u{201c}{c}\u{201d}.")
            }
            InputProblem::Letter(c) => format!("A code has only letters and digits. Check the \u{201c}{c}\u{201d}."),
            InputProblem::NotALink => "That isn't a Switchboard pairing link. Copy it again from Add a device on your \
                                       home computer."
                .into(),
        }
    }
}

/// What was typed or pasted on the join screen, as it is sent to the node: a code as `ABCD-EFGH`, or the link.
/// Forgiving about case, spaces and dashes, the way the node reads a code.
pub fn read_input(typed: &str) -> Result<String, InputProblem> {
    let typed = typed.trim();
    if typed.is_empty() {
        return Err(InputProblem::Empty);
    }
    if typed.contains(':') {
        let ok = typed.len() <= 1024
            && typed.starts_with(LINK_PREFIX)
            && typed.len() > LINK_PREFIX.len()
            && !typed.chars().any(|c| c.is_whitespace() || c.is_control());
        return if ok { Ok(typed.to_string()) } else { Err(InputProblem::NotALink) };
    }
    let mut out = String::new();
    for c in typed.chars().take(64) {
        if c.is_whitespace() || matches!(c, '-' | '\u{2010}'..='\u{2015}' | '\u{2212}' | '_' | '.') {
            continue;
        }
        let up = c.to_ascii_uppercase();
        if !CODE_ALPHABET.contains(up) {
            return Err(InputProblem::Letter(c));
        }
        out.push(up);
    }
    if out.len() != CODE_LEN {
        return Err(InputProblem::Length(out.len()));
    }
    Ok(group_code(&out))
}

/// What stands between the halves of a code, or around it, however it was copied.
fn code_gap(c: char) -> bool {
    c.is_whitespace() || code_dash(c) || matches!(c, '_' | '.')
}

fn code_dash(c: char) -> bool {
    matches!(c, '-' | '\u{2010}'..='\u{2015}' | '\u{2212}')
}

/// The join field's text put in a code's shape while it is typed or after it is pasted: upper case, and the dash
/// between the halves (`7JBE-42SU`). None when the text is in shape already, or is not a code so far: a link, the
/// start of one, an address, a letter no code has, or more than a code holds. Those stay exactly as they are, and
/// `read_input` says what is wrong with them when Join is pressed.
pub fn code_as_typed(field: &str) -> Option<String> {
    let typed = field.trim();
    if typed.is_empty() || LINK_PREFIX.starts_with(&typed.to_ascii_lowercase()) {
        return None;
    }
    let mut code = String::new();
    for c in typed.chars() {
        if code_gap(c) {
            continue;
        }
        let up = c.to_ascii_uppercase();
        if !CODE_ALPHABET.contains(up) || code.len() == CODE_LEN {
            return None;
        }
        code.push(up);
    }
    let half = CODE_LEN / 2;
    let shaped = if code.len() > half {
        format!("{}-{}", &code[..half], &code[half..])
    } else if code.len() == half && typed.ends_with(code_dash) {
        // The dash was typed by hand: it stays, so the next letter does not make a second one.
        format!("{code}-")
    } else {
        code
    };
    (shaped != field).then_some(shaped)
}

/// Why a join did not work, in the app's words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JoinProblem {
    WrongCode,
    Expired,
    TriesUsedUp,
    HomeUnreachable,
    Busy,
    LocalNetwork,
    /// A short code needs a relay to find home, and this computer has none: the link carries one.
    NeedsLink,
    /// The link names no relay, so it works only where the home computer's address is known.
    NeedsAddress,
    /// No node answers on this computer.
    NoNode,
    /// This computer is paired with another home computer, and nothing was changed. The same join with
    /// `replace` moves it.
    OtherHome,
    /// The node's own sentence.
    Other(String),
}

/// What to do when the home computer has removed this one.
pub const REFUSED_FIX: &str = "It was removed there. On your home computer, open Settings, then Devices, choose Add a \
                               device, and join again here with the new code.";

/// Said in place of the code when the home computer has no relay.
pub const NO_CODE: &str = "Short codes need a relay, and this home computer has none set. Use the link or the QR code: \
                           they work on the same network or over Tailscale.";

/// What to do about macOS keeping the node off the local network (amendment 1).
pub const LOCAL_NETWORK_FIX: &str = "macOS is blocking Switchboard from the local network on this Mac. Open System \
                                     Settings > Privacy & Security > Local Network and turn on switchboard-node. It \
                                     reconnects by itself.";

/// The same steps, for under a line that has already said what is blocked.
pub const LOCAL_NETWORK_STEPS: &str = "Open System Settings > Privacy & Security > Local Network and turn on \
                                       switchboard-node. It reconnects by itself.";

impl JoinProblem {
    pub fn says(&self) -> String {
        match self {
            JoinProblem::WrongCode => {
                "That code didn't match. Check it against the one on your home computer and try again.".into()
            }
            JoinProblem::Expired => "That code has expired or was already used. On your home computer, choose Add a \
                                     device to get a new one."
                .into(),
            JoinProblem::TriesUsedUp => "That code was tried too many times and no longer works. On your home \
                                         computer, choose Add a device to get a new one."
                .into(),
            JoinProblem::HomeUnreachable => "Couldn't reach your home computer. Check that it is on and online, then \
                                             try again. If the code keeps failing, paste the link instead."
                .into(),
            JoinProblem::Busy => "Another device is pairing with that code right now. Try again in a moment.".into(),
            JoinProblem::LocalNetwork => LOCAL_NETWORK_FIX.into(),
            JoinProblem::NeedsLink => "A short code finds your home computer through a relay, and this computer has \
                                       none set. Paste the link instead."
                .into(),
            JoinProblem::NeedsAddress => "That link has no relay in it, so this computer needs your home computer's \
                                          address as well. It is shown on the home computer, in Settings, under Devices."
                .into(),
            JoinProblem::NoNode => "The Switchboard node on this computer isn't running, so it can't join with a \
                                    code. Restart this computer, or connect by address."
                .into(),
            JoinProblem::OtherHome => "This computer is paired with another home computer already, so nothing was \
                                       changed. To pair it with this one instead, choose Replace."
                .into(),
            JoinProblem::Other(s) => s.clone(),
        }
    }

    /// Typing the same thing again cannot work: the field is the place to look.
    pub fn retype(&self) -> bool {
        matches!(self, JoinProblem::WrongCode | JoinProblem::Expired | JoinProblem::TriesUsedUp)
    }
}

/// The app's words for a failed join: the node's `kind` when it sent one, else what its sentence says.
pub fn join_problem(e: &NodeError) -> JoinProblem {
    let (kind, message) = match e {
        NodeError::Absent => return JoinProblem::NoNode,
        NodeError::Timeout => return JoinProblem::HomeUnreachable,
        NodeError::Bad(m) => return JoinProblem::Other(m.clone()),
        NodeError::Said { kind, message, .. } => (kind.as_deref().unwrap_or("").to_ascii_lowercase(), message),
    };
    // The node's kinds (sb-node control.rs; sb-link pairing.rs `PairError::kind`).
    match kind.as_str() {
        "wrong_code" | "wrong" | "mismatch" => return JoinProblem::WrongCode,
        "expired" | "gone" | "used" => return JoinProblem::Expired,
        "tries" | "tries_used" | "locked" | "dead" => return JoinProblem::TriesUsedUp,
        "unreachable" | "timeout" | "connection_lost" | "undelivered" | "relay" | "offline" => {
            return JoinProblem::HomeUnreachable;
        }
        "busy" => return JoinProblem::Busy,
        "local_network" | "local-network" => return JoinProblem::LocalNetwork,
        "no_relay" => return JoinProblem::NeedsLink,
        "link_no_relay" => return JoinProblem::NeedsAddress,
        "other_home" => return JoinProblem::OtherHome,
        // Kinds with nothing better to say than the node's own sentence, which is written for a person.
        "bad_input" | "is_home" | "key_in_use" | "key" | "endpoint" | "refused" | "wrong_home" | "version"
        | "protocol" | "save" | "internal" => return JoinProblem::Other(message.clone()),
        _ => {}
    }
    // No kind, or one from a newer node: sb-link's own sentences, and the usual words for the rest.
    let m = message.to_ascii_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| m.contains(w));
    if has(&["local network"]) {
        JoinProblem::LocalNetwork
    } else if has(&["none is set here"]) {
        JoinProblem::NeedsLink
    } else if has(&["names no relay"]) {
        JoinProblem::NeedsAddress
    } else if has(&["too many", "tries"]) {
        JoinProblem::TriesUsedUp
    } else if has(&["expired", "already used"]) {
        JoinProblem::Expired
    } else if has(&["code is wrong", "wrong code", "didn't match", "did not match"]) {
        JoinProblem::WrongCode
    } else if has(&["right now", "busy"]) {
        JoinProblem::Busy
    } else if has(&["took too long", "reach", "no route", "relay", "timed out", "offline"]) {
        JoinProblem::HomeUnreachable
    } else {
        JoinProblem::Other(message.clone())
    }
}

/// Where the join screen is.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum JoinStep {
    /// Waiting for a code or a link.
    #[default]
    Idle,
    /// What was typed cannot be sent.
    Invalid(InputProblem),
    /// The node is pairing.
    Joining,
    /// Paired with this home (its name, when the node said it). The board comes next.
    Joined(Option<String>),
    Failed(JoinProblem),
}

/// The join screen's state: one join at a time, and a late answer to an older one is ignored.
#[derive(Debug, Clone, Default)]
pub struct Join {
    run: u64,
    step: JoinStep,
    wants_addr: bool,
    addr_problem: Option<String>,
    /// The last try was turned down because this computer has another home: the next one says to replace it.
    replace: bool,
}

impl Join {
    /// The last try found that this computer is paired with another home computer. The button says Replace,
    /// and the join it starts is sent with `replace`. Typing something else takes the offer back.
    pub fn replaces(&self) -> bool {
        self.replace
    }

    pub fn step(&self) -> &JoinStep {
        &self.step
    }

    pub fn busy(&self) -> bool {
        self.step == JoinStep::Joining
    }

    /// Join was pressed with `typed` in the field. Some((run, what to send the node)) when a join starts; None
    /// when one is already going, it already worked, or what was typed cannot be a code or a link.
    ///
    /// A join that failed for want of the home computer's address (`needs_address`) is started again with
    /// `start_with`, which checks the address as well.
    pub fn start(&mut self, typed: &str) -> Option<(u64, String)> {
        if matches!(self.step, JoinStep::Joining | JoinStep::Joined(_)) {
            return None;
        }
        match read_input(typed) {
            Ok(input) => {
                self.run += 1;
                self.step = JoinStep::Joining;
                Some((self.run, input))
            }
            Err(p) => {
                self.step = JoinStep::Invalid(p);
                None
            }
        }
    }

    /// The last try needs the home computer's address: the screen shows a field for it, and keeps showing it
    /// while that address is typed and tried.
    pub fn needs_address(&self) -> bool {
        self.wants_addr
    }

    /// Join was pressed while the address field shows. Some((run, input, address)) when a join starts; None when
    /// one is going, or either field cannot be sent (`address_problem` says what is wrong with the address).
    pub fn start_with(&mut self, typed: &str, address: &str) -> Option<(u64, String, String)> {
        if matches!(self.step, JoinStep::Joining | JoinStep::Joined(_)) {
            return None;
        }
        self.addr_problem = None;
        let addr = match address_input(address) {
            Ok(a) => a,
            Err(e) => {
                self.addr_problem = Some(e);
                // The code or link is looked at too, so both fields say what is wrong at once.
                if let Err(p) = read_input(typed) {
                    self.step = JoinStep::Invalid(p);
                }
                return None;
            }
        };
        self.start(typed).map(|(run, input)| (run, input, addr))
    }

    /// What is wrong with the address that was typed, under its field.
    pub fn address_problem(&self) -> Option<&str> {
        self.addr_problem.as_deref()
    }

    /// The address field changed: what was said about it no longer applies.
    pub fn address_edited(&mut self) -> bool {
        self.addr_problem.take().is_some()
    }

    /// The node answered join number `run`. True when this is the answer being waited for.
    pub fn finish(&mut self, run: u64, res: Result<Joined, NodeError>) -> bool {
        if run != self.run || self.step != JoinStep::Joining {
            return false;
        }
        self.step = match res {
            Ok(j) => JoinStep::Joined(j.home),
            Err(e) => JoinStep::Failed(join_problem(&e)),
        };
        if self.step == JoinStep::Failed(JoinProblem::NeedsAddress) {
            self.wants_addr = true;
        }
        self.replace = self.step == JoinStep::Failed(JoinProblem::OtherHome);
        true
    }

    /// The field changed: what was said about the last try no longer applies. Nothing changes while a join runs.
    pub fn edited(&mut self) -> bool {
        let offered = std::mem::take(&mut self.replace);
        if matches!(self.step, JoinStep::Invalid(_) | JoinStep::Failed(_)) {
            self.step = JoinStep::Idle;
            return true;
        }
        offered
    }

    /// Back to the start (the screen was left).
    pub fn reset(&mut self) {
        self.run += 1;
        self.step = JoinStep::Idle;
        self.wants_addr = false;
        self.addr_problem = None;
        self.replace = false;
    }

    /// The line under the field, and whether it is an error.
    pub fn line(&self) -> Option<(String, bool)> {
        match &self.step {
            JoinStep::Idle => None,
            JoinStep::Invalid(p) => Some((p.says(), true)),
            JoinStep::Joining => Some(("Finding your home computer. This can take a few seconds.".into(), false)),
            JoinStep::Joined(Some(home)) => Some((format!("Paired with {home}. Opening the board."), false)),
            JoinStep::Joined(None) => Some(("Paired. Opening the board.".into(), false)),
            // Not something that went wrong: the screen asks for one more thing.
            JoinStep::Failed(p @ JoinProblem::NeedsAddress) => Some((p.says(), false)),
            JoinStep::Failed(p) => Some((p.says(), true)),
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// How a computer is connected
// ---------------------------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    /// Nothing to act on.
    Quiet,
    /// Something is wrong and the line says what.
    Bad,
}

/// One quiet line about a connection, and what to do when there is something to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkLine {
    pub text: String,
    pub tone: Tone,
    pub fix: Option<String>,
}

/// How this computer reaches home, from its node: "Direct", "Through the relay", or "Can't reach home: ...".
/// None on the home computer itself, on a computer that has not joined one, and with keys off.
pub fn home_line(n: &NodeStatus) -> Option<LinkLine> {
    if !n.keys || n.role != Role::Member {
        return None;
    }
    // A computer that has not joined a home has no way there to speak of.
    let home = n.home.as_ref()?;
    let peer = n.peers.iter().find(|p| !home.id.is_empty() && p.id == home.id);
    if let Some(path) = home.path.or(peer.and_then(|p| p.path)) {
        return Some(LinkLine { text: path.says().to_string(), tone: Tone::Quiet, fix: None });
    }
    if home.refused {
        return Some(LinkLine {
            text: "Can't reach home: it no longer lists this computer.".into(),
            tone: Tone::Bad,
            fix: Some(REFUSED_FIX.into()),
        });
    }
    if n.problems.iter().any(Problem::local_network) {
        return Some(LinkLine {
            text: "Can't reach home: macOS is blocking the local network.".into(),
            tone: Tone::Bad,
            fix: Some(LOCAL_NETWORK_STEPS.into()),
        });
    }
    match n.problems.first().map(|p| p.text.as_str()).or(home.error.as_deref()) {
        Some(why) => {
            Some(LinkLine { text: format!("Can't reach home: {}", sentence(why)), tone: Tone::Bad, fix: None })
        }
        None => Some(LinkLine { text: "Connecting to home".into(), tone: Tone::Quiet, fix: None }),
    }
}

/// A sentence as it follows a colon: its own capital kept, and one full stop at the end.
fn sentence(s: &str) -> String {
    let s = s.trim().trim_end_matches('.');
    format!("{s}.")
}

/// The line for one computer in Settings, from the server's report: "Home computer", "Direct" or "Through the
/// relay". None when nothing is known, and for every server with keys off.
pub fn machine_line(r: &Readiness, name: &str) -> Option<&'static str> {
    if r.keys == Mode::Off {
        return None;
    }
    let m = r.machines.iter().find(|m| m.name == name)?;
    if m.home {
        return Some("Home computer");
    }
    m.path.map(Path::says)
}

// ---------------------------------------------------------------------------------------------------------------
// Moving to keys
// ---------------------------------------------------------------------------------------------------------------

/// The card that says what is left before the network port can close, or that it can.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Moving {
    pub ready: bool,
    pub title: &'static str,
    pub body: String,
    /// What is left, one plain line each. Empty when ready.
    pub left: Vec<String>,
}

/// What the button that closes the network port does, said next to it.
pub const CLOSE_EXPLAINS: &str = "Closing the network port means this home computer answers paired devices only. \
                                  Connecting by address stops working, and so does the board in a browser on other \
                                  devices. Paired devices are not interrupted.";

/// THE SWITCH THAT CLOSES THE NETWORK PORT, and the one that turns keys on. The server route that sets "keys":
/// `POST /api/keys/mode {"keys": "off" | "both" | "only"}` (server.py `keys_mode`). A server says it has the route
/// with `"switch": true` in its readiness report (`Readiness::switch`); for one that does not, the app shows no
/// button and says how to do it by hand (`CLOSE_BY_HAND`).
pub const CLOSE_PORT_ROUTE: Option<&str> = Some("/api/keys/mode");

/// The button's label, and its label after the first click (it asks twice).
pub const CLOSE_BUTTON: &str = "Close the network port";
pub const CLOSE_BUTTON_ARMED: &str = "Click again to close the port";

/// Shown in place of the button for a server that has no route for the switch.
pub const CLOSE_BY_HAND: &str = "The server on your home computer is too old to close the port from the app. Update \
                                 it, or run there: switchboard keys only";
/// Said under the button once the server took the switch.
pub const CLOSING: &str = "Closing the network port. Switchboard is restarting on the home computer.";

/// The card, when the server connects both ways. None with keys off (nothing has started) and with keys only
/// (it is done).
pub fn moving(r: &Readiness, now: f64) -> Option<Moving> {
    if r.keys != Mode::Both {
        return None;
    }
    if r.ready {
        return Some(Moving {
            ready: true,
            title: "Ready to close the network port",
            body: format!(
                "Every computer connects with its key, and nothing has connected by address for {} days.",
                r.days
            ),
            left: Vec::new(),
        });
    }
    Some(Moving {
        ready: false,
        title: "Moving to keys",
        body: "Paired devices already connect with their keys. When nothing connects by address any more, this home \
               computer can stop answering on the network."
            .into(),
        left: left_lines(r, now),
    })
}

fn left_lines(r: &Readiness, now: f64) -> Vec<String> {
    let mut out = Vec::new();
    let names = |paired: bool| -> Vec<String> {
        r.blockers
            .iter()
            .filter(|b| b.kind == "machine" && b.paired.unwrap_or(false) == paired)
            .map(|b| b.label.clone().unwrap_or_else(|| "A computer".into()))
            .collect()
    };
    match names(false).as_slice() {
        [] => {}
        [one] => out.push(format!("{one} isn't paired yet. Update Switchboard on it, or add it with a code.")),
        many => out.push(format!(
            "{} computers aren't paired yet: {}. Update Switchboard on them, or add them with a code.",
            many.len(),
            and_list(many)
        )),
    }
    match names(true).as_slice() {
        [] => {}
        [one] => out.push(format!("{one} is paired but hasn't connected with its key yet.")),
        many => out.push(format!(
            "{} computers are paired but haven't connected with their keys yet: {}.",
            many.len(),
            and_list(many)
        )),
    }
    for b in &r.blockers {
        let when = b.last.map(|t| fmt::ago(t, now)).unwrap_or_else(|| "recently".into());
        match (b.kind.as_str(), b.client.as_deref()) {
            ("machine", _) => {}
            ("direct", Some(phone @ ("iPhone" | "Android"))) => {
                let which = if phone == "iPhone" { "An iPhone" } else { "An Android phone" };
                out.push(if b.paired_since {
                    format!("{which} connected by address {when}. A paired phone has connected with its key since.")
                } else {
                    format!("{which} connected by address {when}. Pair it with the home computer in the app first.")
                })
            }
            ("direct", Some("desktop")) => out.push(format!("A desktop app connected by address {when}.")),
            ("direct", Some("helper")) => out.push(format!("A computer's helper connected by address {when}.")),
            ("direct", Some("web")) => out.push(format!("A browser opened the board by address {when}.")),
            ("direct", _) => out.push(format!("Another program connected by address {when}.")),
            ("watching", _) => out.push(format!(
                "Connections by address have been watched for {} of {} days.",
                b.watched_days.unwrap_or(0),
                r.days
            )),
            ("node", _) => out.push(
                "The node on the home computer isn't answering, so paired computers can't be checked.".to_string(),
            ),
            // "settings", "mode" and anything newer: the server's own sentence.
            _ if !b.text.is_empty() => out.push(b.text.clone()),
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn with_keys_off_the_devices_page_says_what_keys_will_do() {
        // Nothing is paired while keys are off: the line does not describe devices that are not there.
        assert!(devices_intro(true).starts_with("The computers and phones paired with your home computer."));
        let off = devices_intro(false);
        assert!(off.starts_with("Keys are off"), "{off}");
        assert!(off.contains("are listed here") && !off.contains("has its own key"), "{off}");
    }

    /// A sentence written over two source lines once reached the screen with the indentation in it.
    #[test]
    fn the_sentences_have_single_spaces() {
        for s in [
            DEVICES_INTRO,
            DEVICES_INTRO_OFF,
            RELAY_LABEL,
            RELAY_LABEL_NONE,
            TURN_ON_TITLE,
            TURN_ON_BODY,
            RESTARTING,
            CLOSING,
            CLOSE_BY_HAND,
            RELAY_HINT,
            LOCAL_NETWORK_STEPS,
            &JoinProblem::NeedsAddress.says(),
        ] {
            assert!(!s.contains("  ") && !s.contains('\n'), "{s:?}");
        }
    }
    use serde_json::json;

    const NOW: f64 = 1_800_000_000.;

    fn dev(id: &str, name: &str, kind: &str, seen: Option<f64>, path: Option<Path>) -> Device {
        Device {
            id: id.into(),
            name: name.into(),
            kind: kind.into(),
            machine: None,
            paired: NOW - 3. * 86400.,
            via: "code".into(),
            last_seen: seen,
            path,
        }
    }

    // ---------- the wire ----------

    #[test]
    fn devices_are_read_as_the_server_passes_them_on() {
        let v = json!({"ok": true, "devices": [
            {"id": "aaaaaaaa1", "name": "Netbook", "kind": "computer", "machine": "laptop", "paired": 1_799_000_000,
             "via": "code", "last_seen": 1_799_999_990.5, "path": "direct"},
            {"id": "bbbbbbbb2", "name": "Sam's iPhone", "kind": "phone", "paired": 1_799_000_000_000u64, "via": "link",
             "last_seen": null, "path": null},
            {"name": "no id"},
            "not a device",
        ]});
        let d = read_devices(&v).unwrap();
        assert_eq!(d.len(), 2, "a row with no id is left out, and so is one that is not a row");
        assert_eq!(d[0].machine.as_deref(), Some("laptop"));
        assert_eq!(d[0].path, Some(Path::Direct));
        assert_eq!(d[1].paired, 1_799_000_000., "milliseconds are brought down to seconds");
        assert_eq!(d[1].last_seen, None);
        assert_eq!(d[1].path, None);
        assert_eq!(read_devices(&json!({"ok": true})), None, "no list is not an empty list");
    }

    #[test]
    fn a_status_is_read_whatever_shape_its_parts_have() {
        let v = json!({"ok": true, "keys": "on", "role": "member", "machine": "laptop", "id": "me",
            "home": {"id": "h1", "name": "Studio"}, "relay": {"url": "https://relay.example"},
            "peers": [{"id": "h1", "name": "Studio", "path": "relay", "rtt_ms": 41.5}],
            "problems": ["The relay is slow.", {"kind": "local_network", "text": "Local Network access is off."}, 7]});
        let s = read_status(&v);
        assert!(s.keys);
        assert_eq!(s.role, Role::Member);
        assert_eq!(s.home, Some(Home { id: "h1".into(), name: "Studio".into(), ..Default::default() }));
        assert_eq!(s.relay.as_deref(), Some("https://relay.example"));
        assert_eq!(s.peers[0].path, Some(Path::Relay));
        assert_eq!(s.peers[0].rtt_ms, Some(41.5));
        assert_eq!(s.problems.len(), 2);
        assert!(!s.problems[0].local_network());
        assert!(s.problems[1].local_network());

        // The barest answer the plan promises: only "machine".
        let bare = read_status(&json!({"ok": true, "machine": "studio"}));
        assert!(!bare.keys, "a node that says nothing of keys or a home has none");
        assert_eq!(bare.role, Role::Alone);
        // Booleans and plain strings.
        let home = read_status(&json!({"keys": true, "role": "home", "relay": "http://r:3340"}));
        assert!(home.keys);
        assert_eq!(home.role, Role::Home);
        assert_eq!(home.relay.as_deref(), Some("http://r:3340"));
        assert!(!read_status(&json!({"keys": "off", "role": "member"})).keys);
        assert!(read_status(&json!({"role": "member"})).keys, "a member has keys on");
    }

    #[test]
    fn readiness_is_read_as_the_server_writes_it() {
        let v = json!({"ok": true, "keys": "both", "ready": false, "days": 7, "blockers": [
            {"kind": "machine", "text": "Tower is not paired with this home computer.", "machine": "tower",
             "label": "Tower", "paired": false},
            {"kind": "direct", "text": "The iPhone app connected by address 2 hours ago.", "client": "iPhone",
             "last": NOW - 7200.},
            {"kind": "watching", "text": "...", "watched_days": 3}],
            "machines": [{"name": "studio", "label": "Studio", "home": true, "paired": false, "seen": null, "path": null},
                         {"name": "laptop", "label": "Netbook", "home": false, "paired": true, "seen": NOW - 5., "path": "relay"}]});
        let r = read_readiness(&v);
        assert_eq!(r.keys, Mode::Both);
        assert!(!r.ready);
        assert_eq!(r.blockers.len(), 3);
        assert_eq!(r.blockers[0].label.as_deref(), Some("Tower"));
        assert_eq!(r.blockers[1].client.as_deref(), Some("iPhone"));
        assert_eq!(r.blockers[2].watched_days, Some(3));
        assert_eq!(r.machines[1].path, Some(Path::Relay));
        assert_eq!(machine_line(&r, "studio"), Some("Home computer"));
        assert_eq!(machine_line(&r, "laptop"), Some("Through the relay"));
        assert_eq!(machine_line(&r, "tower"), None);

        assert_eq!(read_readiness(&json!({"ok": true})).keys, Mode::Off, "no keys entry is off");
        assert_eq!(read_readiness(&json!({"keys": "off"})).keys, Mode::Off);
        assert_eq!(read_readiness(&json!({"keys": "only"})).keys, Mode::Only);
        assert_eq!(read_readiness(&json!({"keys": "newer"})).keys, Mode::Only, "an unknown mode opens nothing");
        let odd = read_readiness(&json!({"keys": "both", "ready": true, "blockers": [{"kind": "node", "text": "x"}]}));
        assert!(!odd.ready, "ready with something in the way is not ready");
    }

    #[test]
    fn an_answer_that_says_no_keeps_the_nodes_words() {
        assert!(read_answer(200, br#"{"ok": true}"#).is_ok());
        assert_eq!(
            read_answer(403, br#"{"ok": false, "error": "That code is wrong.", "kind": "wrong_code"}"#),
            Err(NodeError::Said {
                status: 403,
                kind: Some("wrong_code".into()),
                message: "That code is wrong.".into()
            })
        );
        assert!(
            matches!(read_answer(200, br#"{"ok": false, "error": "No."}"#), Err(NodeError::Said { status: 200, .. })),
            "ok false is a no, whatever the status"
        );
        assert_eq!(read_answer(404, b"Not Found"), Err(NodeError::Absent), "something else holds the port");
        assert!(matches!(read_answer(200, b"<html>"), Err(NodeError::Bad(_))));
        assert!(matches!(read_answer(500, b""), Err(NodeError::Said { status: 500, kind: None, .. })));
    }

    #[test]
    fn the_nodes_own_status_reads_as_it_sends_it() {
        // sb-link control.rs `Status`, on a computer that joined a home and reaches it through the relay.
        let member = json!({"ok": true, "version": "0.8.23", "role": "member", "id": "aa11", "name": "Laptop",
            "machine": "laptop", "relay": {"url": "https://relay.example", "source": "home", "connected": true, "same_id": false},
            "home": {"id": "h1", "name": "Studio", "connected": true, "path": "relay", "rtt_ms": 41, "refused": false},
            "forwarders": [{"target": "api", "addr": "127.0.0.1:8130", "ok": true}], "devices": 0, "updates": [], "problems": []});
        let s = read_status(&member);
        assert!(s.keys, "the control API is there only while keys are on");
        assert_eq!((s.role, s.id.as_deref(), s.machine.as_deref()), (Role::Member, Some("aa11"), Some("laptop")));
        assert_eq!(s.relay.as_deref(), Some("https://relay.example"));
        assert_eq!(home_line(&s).unwrap().text, "Through the relay");
        let k = Keys { server: None, node: Some(s) };
        assert!(k.joined() && !k.offer_join());

        // Keys on, never joined: the first run asks for a code, and there is no line about home.
        let fresh = read_status(&json!({"ok": true, "role": "member", "id": "aa11", "machine": null,
            "relay": {"url": null, "source": "none", "connected": false}, "problems": []}));
        assert_eq!(home_line(&fresh), None);
        assert!(Keys { server: None, node: Some(fresh) }.offer_join());

        // Home removed this computer: it is said, with what to do, and the code is asked for again.
        let refused = read_status(&json!({"ok": true, "role": "member", "id": "aa11",
            "home": {"id": "h1", "name": "Studio", "connected": false, "path": null, "rtt_ms": null, "refused": true},
            "problems": ["The home computer no longer lists this computer. Pair it again: switchboard-node pair <code or link>"]}));
        let line = home_line(&refused).unwrap();
        assert_eq!(line.text, "Can't reach home: it no longer lists this computer.");
        assert_eq!(line.fix.as_deref(), Some(REFUSED_FIX));
        assert!(Keys { server: None, node: Some(refused) }.offer_join());

        // A Mac that keeps the node off the local network, in the node's own words.
        let blocked = read_status(&json!({"ok": true, "role": "member", "id": "aa11",
            "home": {"id": "h1", "name": "Studio", "connected": false, "path": "direct", "rtt_ms": null, "error": "No route to host (os error 65)"},
            "problems": ["This Mac is not letting switchboard-node use the local network. Turn it on in System Settings, Privacy & Security, Local Network."]}));
        let line = home_line(&blocked).unwrap();
        assert_eq!(line.tone, Tone::Bad, "a path left over from before it dropped is not a connection");
        assert_eq!(line.fix.as_deref(), Some(LOCAL_NETWORK_STEPS));

        // Not connected, and only the tunnel's error to go on.
        let down = read_status(&json!({"ok": true, "role": "member", "id": "aa11",
            "home": {"id": "h1", "name": "Studio", "connected": false, "path": null, "error": "timed out"}, "problems": []}));
        assert_eq!(home_line(&down).unwrap().text, "Can't reach home: timed out.");
    }

    #[test]
    fn keys_not_running_is_a_node_that_can_join() {
        let off = read_answer(503, br#"{"ok": false, "error": "Keys are not running on this computer yet."}"#);
        let s = status_from(off).unwrap();
        assert!(s.keys && s.role == Role::Alone && s.home.is_none());
        let k = Keys { server: None, node: Some(s.clone()) };
        assert!(k.offer_join() && !k.hidden());
        assert_eq!(home_line(&s), None);
        // Nothing listening, or something that is not a node, is still no node at all.
        assert_eq!(status_from(Err(NodeError::Absent)), Err(NodeError::Absent));
        assert_eq!(status_from(read_answer(404, b"Not Found")), Err(NodeError::Absent));
        assert!(status_from(read_answer(500, b"")).is_err());
    }

    #[test]
    fn the_nodes_own_reasons_for_a_failed_join_are_said_in_the_apps_words() {
        let said = |m: &str| join_problem(&NodeError::Said { status: 409, kind: None, message: m.into() });
        assert_eq!(said("That code is wrong. Check it and try again."), JoinProblem::WrongCode);
        assert_eq!(said("That code has expired or was already used. Make a new one."), JoinProblem::Expired);
        assert_eq!(
            said("Another device is pairing with that code right now. Try again in a moment."),
            JoinProblem::Busy
        );
        assert_eq!(said("Pairing took too long. Try again."), JoinProblem::HomeUnreachable);
        assert_eq!(said("Cannot reach the home computer (timed out)."), JoinProblem::HomeUnreachable);
        assert_eq!(
            said(
                "This Mac is not letting switchboard-node use the local network. Turn it on in System Settings, Privacy & Security, Local Network."
            ),
            JoinProblem::LocalNetwork
        );
        // Sentences written for the command line are not shown as they are.
        let no_relay = said(
            "A short code finds the home computer through a relay, and none is set here. Use the link instead, or set the relay first: switchboard-node relay <url>",
        );
        assert_eq!(no_relay, JoinProblem::NeedsLink);
        let no_addr = said(
            "That link names no relay, so this computer needs the home computer's address: add --addr <ip:port> (the home computer's status shows it).",
        );
        assert_eq!(no_addr, JoinProblem::NeedsAddress);
        for p in [no_relay, no_addr] {
            assert!(!p.says().contains("switchboard-node") && !p.says().contains("--"), "{}", p.says());
        }
    }

    #[test]
    fn a_home_without_a_relay_offers_the_link_alone() {
        let o =
            read_offer(&json!({"ok": true, "code": null, "link": "switchboard://pair/1?h=aa", "expires": 1700000600}))
                .unwrap();
        assert_eq!((o.code.as_str(), o.link.as_str()), ("", "switchboard://pair/1?h=aa"));
        assert_eq!(read_offer(&json!({"ok": true, "code": null, "link": null})), None, "nothing to pair with");
    }

    #[test]
    fn only_this_computer_is_ever_called() {
        assert!(loopback_base("http://127.0.0.1:8137"));
        assert!(loopback_base("http://localhost:8137"));
        assert!(loopback_base("http://[::1]:8137"));
        assert!(!loopback_base("http://192.0.2.10:8137"));
        assert!(!loopback_base("https://127.0.0.1:8137"));
        assert!(!loopback_base("http://127.0.0.1.example.com:8137"));
        assert!(!loopback_base("http://"));
        assert!(HttpNode::at("http://192.0.2.10:8137").is_none());
        assert!(HttpNode::at("http://127.0.0.1:9/").is_some());
    }

    // ---------- on, off and hidden ----------

    #[test]
    fn nothing_shows_with_keys_off_and_no_node() {
        let none = Keys::default();
        assert!(none.hidden());
        assert!(!none.on());
        assert!(!none.can_join());
        // A server from after keys with them off, and a node from after keys with them off.
        let off = Keys {
            server: Some(read_readiness(
                &json!({"keys": "off", "blockers": [{"kind": "mode", "text": "Keys are off."}]}),
            )),
            node: Some(read_status(&json!({"ok": true, "keys": false, "machine": "studio"}))),
        };
        assert!(off.hidden());
        assert!(!off.on());
        assert!(!off.can_join());
        assert_eq!(moving(off.server.as_ref().unwrap(), NOW), None, "no card with keys off");
        assert_eq!(home_line(off.node.as_ref().unwrap()), None);
        assert_eq!(machine_line(off.server.as_ref().unwrap(), "studio"), None);
    }

    #[test]
    fn what_shows_follows_the_server_and_the_node() {
        let both = Keys { server: Some(Readiness { keys: Mode::Both, ..Default::default() }), node: None };
        assert!(both.on() && !both.hidden() && !both.can_join() && !both.only());
        let only = Keys { server: Some(Readiness { keys: Mode::Only, ..Default::default() }), node: None };
        assert!(only.on() && only.only());
        // No server yet, a node that can join: the first run offers a code.
        let fresh = Keys { server: None, node: Some(NodeStatus { keys: true, ..Default::default() }) };
        assert!(fresh.can_join() && fresh.offer_join() && !fresh.on() && !fresh.hidden());
        // A computer that already joined: its own port answers, so the first run does not ask for a code again.
        let member = Keys {
            server: None,
            node: Some(NodeStatus {
                keys: true,
                role: Role::Member,
                home: Some(Home { id: "h1".into(), name: "Studio".into(), ..Default::default() }),
                ..Default::default()
            }),
        };
        assert!(member.joined() && member.can_join() && !member.offer_join());
        assert!(!Keys::default().offer_join() && !Keys::default().joined());
        let home = Keys { server: None, node: Some(NodeStatus { keys: true, role: Role::Home, ..Default::default() }) };
        assert!(!home.can_join(), "a home does not join another");
        assert!(!home.hidden());
    }

    // ---------- devices ----------

    #[test]
    fn device_rows_say_when_and_how() {
        let mut laptop = dev("a", "Netbook", "computer", Some(NOW - 5.), Some(Path::Direct));
        laptop.machine = Some("laptop".into());
        let mut phone = dev("b", "Sam's iPhone", "phone", Some(NOW - 7200.), None);
        phone.via = "link".into();
        let mut tower = dev("c", "Tower", "computer", None, None);
        tower.via = "upgrade".into();
        tower.paired = NOW - 30.;
        let rows = device_rows(&[tower, phone, laptop], NOW, None);
        assert_eq!(rows.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(), vec!["Netbook", "Sam's iPhone", "Tower"]);
        assert_eq!(rows[0].kind, "Computer");
        assert_eq!(rows[0].paired, "Paired 3d ago with a code");
        assert_eq!(rows[0].seen, "Connected");
        assert_eq!(rows[0].path, Some("Direct"));
        assert_eq!(rows[1].kind, "Phone");
        assert_eq!(rows[1].paired, "Paired 3d ago with a link");
        assert_eq!(rows[1].seen, "Last seen 2h ago");
        assert_eq!(rows[1].path, None, "a path is only said while it is connected");
        assert_eq!(rows[2].paired, "Paired just now automatically");
        assert_eq!(rows[2].seen, "Not connected yet");
        assert!(rows.iter().all(|r| !r.here));
    }

    /// A computer that holds its connection open and sends nothing for a while is still connected: the home
    /// computer names its path only while the connection is up.
    #[test]
    fn a_device_with_a_path_is_connected_however_long_ago_it_last_spoke() {
        let quiet = dev("a", "Netbook", "computer", Some(NOW - 240.), Some(Path::Relay));
        let gone = dev("b", "Tower", "computer", Some(NOW - 240.), None);
        let rows = device_rows(&[gone, quiet], NOW, None);
        assert_eq!((rows[0].name.as_str(), rows[0].seen.as_str(), rows[0].connected), ("Netbook", "Connected", true));
        assert_eq!(rows[0].path, Some("Through the relay"));
        assert_eq!((rows[1].seen.as_str(), rows[1].connected), ("Last seen 4m ago", false));
    }

    #[test]
    fn this_computer_is_marked_and_its_removal_says_so() {
        let mut laptop = dev("a", "Netbook", "computer", Some(NOW), Some(Path::Relay));
        laptop.machine = Some("laptop".into());
        let phone = dev("b", "Sam's iPhone", "phone", None, None);
        let devices = [laptop, phone];
        let by_id = NodeStatus { keys: true, role: Role::Member, id: Some("a".into()), ..Default::default() };
        let rows = device_rows(&devices, NOW, Some(&by_id));
        assert!(rows[0].here && !rows[1].here);
        assert_eq!(rows[0].path, Some("Through the relay"));
        // A node that does not say its id: its name in the list of computers does.
        let by_name =
            NodeStatus { keys: true, role: Role::Member, machine: Some("laptop".into()), ..Default::default() };
        assert!(device_rows(&devices, NOW, Some(&by_name))[0].here);
        // The home computer is not one of its own devices.
        let home = NodeStatus { keys: true, role: Role::Home, machine: Some("laptop".into()), ..Default::default() };
        assert!(!device_rows(&devices, NOW, Some(&home))[0].here);

        let here = remove_explains(&rows[0]);
        assert!(here.starts_with("Removing Netbook turns its key off."));
        assert!(here.contains("this app disconnects too"));
        let other = remove_explains(&device_rows(&devices, NOW, None)[0]);
        assert!(other.ends_with("Chats running on it are not stopped."));
        let phone = remove_explains(&rows[1]);
        assert!(phone.ends_with("needs a new code to come back."), "{phone}");
        assert_eq!(device_count(0), "No devices yet");
        assert_eq!(device_count(1), "1 device");
        assert_eq!(device_count(4), "4 devices");
    }

    // ---------- add a device ----------

    #[test]
    fn a_code_is_shown_in_two_halves_and_counts_down() {
        assert_eq!(group_code("ABCDEFGH"), "ABCD-EFGH");
        assert_eq!(group_code("abcd-efgh"), "ABCD-EFGH");
        assert_eq!(group_code("ABCD EFGH"), "ABCD-EFGH");
        assert_eq!(group_code("ABCDEF"), "ABC-DEF", "a code of another length still splits in the middle");
        assert_eq!(group_code(""), "");
        let offer = PairOffer { code: "ABCD-EFGH".into(), link: String::new(), expires: NOW + 581. };
        assert_eq!(offer_left(&offer, NOW), 581.);
        assert_eq!(offer_line(581.), "Works once, for 9:41");
        assert_eq!(offer_line(600.), "Works once, for 10:00");
        assert_eq!(offer_line(0.4), "Works once, for 0:01");
        assert_eq!(offer_left(&offer, NOW + 900.), 0.);
        assert_eq!(offer_line(0.), "Expired");
        assert_eq!(offer_parts(581.), ("Works once, for", Some("9:41".to_string())));
        assert_eq!(offer_view(&offer, None, NOW), OfferView::Waiting(581.));
        assert_eq!(offer_view(&offer, None, NOW + 581.), OfferView::Expired);
        assert_eq!(offer_view(&offer, Some("Tower"), NOW), OfferView::Paired("Tower".into()));
        assert_eq!(
            offer_view(&offer, Some("Tower"), NOW + 900.),
            OfferView::Paired("Tower".into()),
            "it joined in time, whatever the clock says now"
        );
        assert_eq!(
            read_offer(&json!({"code": "ABCD-EFGH", "link": "switchboard://pair/1?h=1", "expires": 5}))
                .unwrap()
                .expires,
            5.
        );
        assert_eq!(read_offer(&json!({"expires": 5})), None, "neither a code nor a link is no offer");
    }

    #[test]
    fn the_device_that_joins_is_the_one_that_was_not_there() {
        let known = vec!["a".to_string(), "b".to_string()];
        let a = dev("a", "Netbook", "computer", None, None);
        let b = dev("b", "Phone", "phone", None, None);
        assert_eq!(newly_paired(&known, &[a.clone(), b.clone()]), None);
        let c = dev("c", "Tower", "computer", None, None);
        assert_eq!(newly_paired(&known, &[a.clone(), c.clone(), b.clone()]).map(|d| d.name.as_str()), Some("Tower"));
        // A device removed while the code shows is not a new one.
        assert_eq!(newly_paired(&known, &[a]), None);
    }

    // ---------- join ----------

    #[test]
    fn typed_codes_are_forgiven_case_spaces_and_dashes() {
        assert_eq!(read_input("abcd-efgh").as_deref(), Ok("ABCD-EFGH"));
        assert_eq!(read_input("  ABCD EFGH\n").as_deref(), Ok("ABCD-EFGH"));
        assert_eq!(read_input("abcdefgh").as_deref(), Ok("ABCD-EFGH"));
        assert_eq!(read_input("AB.CD_EF\u{2013}GH").as_deref(), Ok("ABCD-EFGH"));
        assert_eq!(read_input(""), Err(InputProblem::Empty));
        assert_eq!(read_input("   "), Err(InputProblem::Empty));
        assert_eq!(read_input("ABCD-EFG"), Err(InputProblem::Length(7)));
        assert_eq!(read_input("ABCD-EFGH-J"), Err(InputProblem::Length(9)));
        assert_eq!(read_input("ABCD-EFG0"), Err(InputProblem::Letter('0')));
        assert_eq!(read_input("ABCD-EFG!"), Err(InputProblem::Letter('!')));
        assert!(InputProblem::Letter('0').says().contains("never use 0, O, 1, I or L"));
        assert!(InputProblem::Letter('!').says().contains("only letters and digits"));
        assert_eq!(InputProblem::Length(7).says(), "A code has 8 letters and digits. That was 7.");
    }

    #[test]
    fn a_code_is_grouped_as_it_is_typed() {
        // Letter by letter: upper case, and the dash once the second half starts.
        let mut field = String::new();
        let mut seen = vec![];
        for c in "7jbe42su".chars() {
            field.push(c);
            if let Some(shaped) = code_as_typed(&field) {
                field = shaped;
            }
            seen.push(field.clone());
        }
        assert_eq!(seen, ["7", "7J", "7JB", "7JBE", "7JBE-4", "7JBE-42", "7JBE-42S", "7JBE-42SU"]);
        // Pasted, in every way a code gets copied.
        for pasted in
            ["7jbe42su", "7JBE42SU", "7jbe-42su", "7jbe 42su", " 7JBE - 42SU \n", "7jbe\u{2013}42su", "7JBE-42SU-"]
        {
            assert_eq!(code_as_typed(pasted).as_deref(), Some("7JBE-42SU"), "{pasted:?}");
        }
        // Already in shape: nothing to change, so the caret stays where it is.
        for fine in ["", "7", "7JBE", "7JBE-4", "7JBE-42SU"] {
            assert_eq!(code_as_typed(fine), None, "{fine:?}");
        }
        // A dash typed after the first half stays, and goes again with the backspace that follows it.
        assert_eq!(code_as_typed("7JBE-"), None);
        assert_eq!(code_as_typed("7jbe-").as_deref(), Some("7JBE-"));
        assert_eq!(code_as_typed("7JB-").as_deref(), Some("7JB"));
        // Not a code so far (a letter no code has, or too many): left exactly as typed, and Join says what is wrong.
        for other in ["7JBE-42S0", "hello", "7JBE-42SUX", "7jbe42sux", "ABCD EFGH J"] {
            assert_eq!(code_as_typed(other), None, "{other:?}");
        }
    }

    #[test]
    fn a_pasted_link_is_never_reshaped() {
        for link in [
            "switchboard://pair/1?h=00ff&r=https%3A%2F%2Frelay.example&s=0011&n=Studio",
            "switchboard://pair/1?h=6f1c22d0&s=9a2e41b7",
            "  switchboard://pair/1?h=6f1c22d0&s=9a2e41b7\n",
            "switchboard://pair/1?h=abcd&s=efgh",
            "SWITCHBOARD://PAIR/1?H=ABCD",
            "http://192.0.2.10:8130",
            "abcd:efgh",
            "abcd/efgh",
            "abcd?efgh",
            "h=abcdefgh",
        ] {
            assert_eq!(code_as_typed(link), None, "{link:?}");
        }
        // A link that is still being typed or was cut short is left alone too.
        for part in ["s", "sw", "switchb", "switchboard", "switchboard:", "switchboard://pair"] {
            assert_eq!(code_as_typed(part), None, "{part:?}");
        }
    }

    #[test]
    fn a_code_is_taken_however_it_was_typed_or_pasted() {
        for typed in ["7JBE-42SU", "7jbe-42su", "7JBE42SU", "7jbe42su", "7jbe 42su", " 7JBE - 42SU \n", "7j be 42 su"] {
            assert_eq!(read_input(typed).as_deref(), Ok("7JBE-42SU"), "{typed:?}");
        }
    }

    #[test]
    fn a_pasted_link_goes_as_it_is() {
        let link = "switchboard://pair/1?h=00ff&r=https://relay.example&s=0011&n=Studio";
        assert_eq!(read_input(&format!("  {link}\n")).as_deref(), Ok(link));
        assert_eq!(read_input("http://192.0.2.10:8130"), Err(InputProblem::NotALink));
        assert_eq!(read_input("switchboard://pair/1?"), Err(InputProblem::NotALink));
        assert_eq!(read_input("switchboard://pair/1?h=1 2"), Err(InputProblem::NotALink));
        assert!(InputProblem::NotALink.says().contains("pairing link"));
    }

    #[test]
    fn a_failed_join_is_said_in_plain_words() {
        let said = |kind: Option<&str>, message: &str| NodeError::Said {
            status: 403,
            kind: kind.map(str::to_string),
            message: message.into(),
        };
        assert_eq!(join_problem(&said(Some("wrong_code"), "x")), JoinProblem::WrongCode);
        assert_eq!(join_problem(&said(Some("expired"), "x")), JoinProblem::Expired);
        assert_eq!(join_problem(&said(Some("tries"), "x")), JoinProblem::TriesUsedUp);
        assert_eq!(join_problem(&said(Some("unreachable"), "x")), JoinProblem::HomeUnreachable);
        assert_eq!(join_problem(&said(Some("busy"), "x")), JoinProblem::Busy);
        assert_eq!(join_problem(&said(Some("local_network"), "x")), JoinProblem::LocalNetwork);
        // Every kind the node sends (sb-node control.rs, sb-link pairing.rs), whatever its sentence says.
        assert_eq!(join_problem(&said(Some("no_relay"), "x")), JoinProblem::NeedsLink);
        assert_eq!(join_problem(&said(Some("other_home"), "x")), JoinProblem::OtherHome);
        assert_eq!(join_problem(&said(Some("link_no_relay"), "x")), JoinProblem::NeedsAddress);
        for kind in ["timeout", "connection_lost", "undelivered"] {
            assert_eq!(join_problem(&said(Some(kind), "That code is wrong.")), JoinProblem::HomeUnreachable, "{kind}");
        }
        for kind in [
            "bad_input",
            "is_home",
            "key_in_use",
            "key",
            "endpoint",
            "refused",
            "wrong_home",
            "version",
            "protocol",
            "save",
            "internal",
        ] {
            assert_eq!(
                join_problem(&said(Some(kind), "That code is wrong, says a sentence the kind overrules.")),
                JoinProblem::Other("That code is wrong, says a sentence the kind overrules.".into()),
                "{kind}"
            );
        }
        // A kind from a newer node falls back to the sentence.
        assert_eq!(join_problem(&said(Some("new_kind"), "That code is wrong.")), JoinProblem::WrongCode);
        // No kind: sb-link's own sentences.
        assert_eq!(join_problem(&said(None, "That code is wrong. Check it and try again.")), JoinProblem::WrongCode);
        assert_eq!(
            join_problem(&said(None, "That code has expired or was already used. Make a new one.")),
            JoinProblem::Expired
        );
        assert_eq!(
            join_problem(&said(None, "Another device is pairing with that code right now. Try again in a moment.")),
            JoinProblem::Busy
        );
        assert_eq!(join_problem(&said(None, "Pairing took too long. Try again.")), JoinProblem::HomeUnreachable);
        assert_eq!(join_problem(&said(None, "That code was tried too many times.")), JoinProblem::TriesUsedUp);
        assert_eq!(
            join_problem(&said(None, "The other computer could not add this device (the list is full).")),
            JoinProblem::Other("The other computer could not add this device (the list is full).".into())
        );
        assert_eq!(join_problem(&NodeError::Absent), JoinProblem::NoNode);
        assert_eq!(join_problem(&NodeError::Timeout), JoinProblem::HomeUnreachable);
        for p in [
            JoinProblem::WrongCode,
            JoinProblem::Expired,
            JoinProblem::TriesUsedUp,
            JoinProblem::HomeUnreachable,
            JoinProblem::Busy,
            JoinProblem::LocalNetwork,
            JoinProblem::NoNode,
        ] {
            let s = p.says();
            assert!(s.ends_with('.') && !s.contains('\u{2014}') && !s.contains('!'), "{s}");
        }
        assert!(JoinProblem::WrongCode.retype() && !JoinProblem::HomeUnreachable.retype());
    }

    #[test]
    fn the_join_screen_runs_one_join_at_a_time() {
        let mut j = Join::default();
        assert_eq!(j.step(), &JoinStep::Idle);
        assert_eq!(j.line(), None);
        assert_eq!(j.start("abc"), None);
        assert_eq!(j.step(), &JoinStep::Invalid(InputProblem::Length(3)));
        assert!(j.line().unwrap().1, "said as an error");
        assert!(j.edited());
        assert_eq!(j.step(), &JoinStep::Idle);

        let (run, input) = j.start("abcd efgh").unwrap();
        assert_eq!(input, "ABCD-EFGH");
        assert!(j.busy());
        assert_eq!(j.start("abcd efgh"), None, "a second press while it runs starts nothing");
        assert!(!j.edited(), "typing while it runs changes nothing");
        assert!(j.busy());
        assert!(!j.finish(run + 1, Ok(Joined::default())), "not the join being waited for");
        assert!(
            j.finish(run, Err(NodeError::Said { status: 403, kind: Some("wrong_code".into()), message: "no".into() }))
        );
        assert_eq!(j.step(), &JoinStep::Failed(JoinProblem::WrongCode));
        assert!(!j.finish(run, Ok(Joined::default())), "an answer comes once");

        let (run2, _) = j.start("abcd efgh").unwrap();
        assert!(j.finish(run2, Ok(Joined { home: Some("Studio".into()), machine: None })));
        assert_eq!(j.step(), &JoinStep::Joined(Some("Studio".into())));
        assert_eq!(j.line(), Some(("Paired with Studio. Opening the board.".into(), false)));
        assert_eq!(j.start("abcd efgh"), None, "paired: nothing more to join");
    }

    fn member_at(api: Option<&str>) -> NodeStatus {
        let forwards = api
            .map(|a| {
                vec![
                    Forward { target: "hub".into(), addr: "127.0.0.1:28172".into(), ok: true },
                    Forward { target: "api".into(), addr: a.into(), ok: true },
                ]
            })
            .unwrap_or_default();
        NodeStatus {
            keys: true,
            role: Role::Member,
            home: Some(Home { id: "h1".into(), name: "Studio".into(), ..Default::default() }),
            forwards,
            ..Default::default()
        }
    }

    #[test]
    fn the_nodes_status_says_where_its_port_to_home_is() {
        let s = read_status(&json!({"ok": true, "role": "member", "id": "aa11",
            "home": {"id": "h1", "name": "Studio", "connected": true, "path": "direct"},
            "forwarders": [{"target": "hub", "addr": "127.0.0.1:28172", "ok": true},
                           {"target": "api", "addr": "127.0.0.1:28170", "ok": true}], "problems": []}));
        assert_eq!(s.forwards.len(), 2);
        assert_eq!(home_here(&s).as_deref(), Some("http://127.0.0.1:28170"));
        // A port another program holds is still where the node means to be: the board then says what the node says.
        let taken = read_status(&json!({"role": "member", "home": {"id": "h1"},
            "forwarders": [{"target": "api", "addr": "127.0.0.1:8130", "ok": false, "error": "in use"}]}));
        assert!(!taken.forwards[0].ok);
        assert_eq!(home_here(&taken).as_deref(), Some("http://127.0.0.1:8130"));
        // Nothing listed, only the hub's port, or an address that is not on this computer: none.
        assert_eq!(home_here(&member_at(None)), None);
        let hub_only =
            read_status(&json!({"role": "member", "forwarders": [{"target": "hub", "addr": "127.0.0.1:8132"}]}));
        assert_eq!(home_here(&hub_only), None);
        for bad in ["192.0.2.10:8130", "studio.local:8130", "127.0.0.1", "127.0.0.1:0", ""] {
            assert_eq!(home_here(&member_at(Some(bad))), None, "{bad}");
        }
        assert_eq!(home_here(&member_at(Some("[::1]:8130"))).as_deref(), Some("http://[::1]:8130"));
    }

    #[test]
    fn a_join_ends_at_the_port_the_node_says_it_carries_home() {
        let calls = |n: &FakeNode| n.state.lock().unwrap().calls.clone();
        // The node on this computer carries home on a port of its own, not 8130.
        let alone = NodeStatus { keys: true, role: Role::Alone, ..Default::default() };
        let node = FakeNode::with(alone.clone());
        node.state.lock().unwrap().after_join = vec![member_at(Some("127.0.0.1:28170"))];
        let (joined, at) = join_home(&node, "ABCD-EFGH", Some("Laptop"), &[], false, 3, Duration::ZERO).unwrap();
        assert_eq!(joined, Joined::default());
        assert_eq!(at, "http://127.0.0.1:28170", "never the usual port while the node names its own");
        assert_eq!(calls(&node), ["join ABCD-EFGH", "status"]);

        // The node starts keys again after a join: for a moment it says they are not running yet.
        let node = FakeNode::with(alone.clone());
        node.state.lock().unwrap().after_join = vec![alone.clone(), alone.clone(), member_at(Some("127.0.0.1:28170"))];
        let (_, at) = join_home(&node, "ABCD-EFGH", None, &[], false, 5, Duration::ZERO).unwrap();
        assert_eq!(at, "http://127.0.0.1:28170");
        assert_eq!(calls(&node), ["join ABCD-EFGH", "status", "status", "status"]);

        // A node that runs and lists no port for the server: the usual one, without waiting for one to appear.
        let node = FakeNode::with(alone.clone());
        node.state.lock().unwrap().after_join = vec![member_at(None)];
        let (_, at) = join_home(&node, "ABCD-EFGH", None, &[], false, 5, Duration::ZERO).unwrap();
        assert_eq!(at, HOME_HERE);
        assert_eq!(calls(&node), ["join ABCD-EFGH", "status"]);

        // A node that never says: the usual one, after the tries.
        let node = FakeNode::with(alone.clone());
        let (_, at) = join_home(&node, "ABCD-EFGH", None, &[], false, 3, Duration::ZERO).unwrap();
        assert_eq!(at, HOME_HERE);
        assert_eq!(calls(&node).len(), 4);

        // A join that failed asks nothing more.
        let node = FakeNode::with(alone);
        node.state.lock().unwrap().joins = vec![Err(NodeError::Timeout)];
        assert_eq!(join_home(&node, "ABCD-EFGH", None, &[], false, 3, Duration::ZERO), Err(NodeError::Timeout));
        assert_eq!(calls(&node), ["join ABCD-EFGH"]);
    }

    #[test]
    fn a_link_without_a_relay_asks_for_homes_address() {
        let mut j = Join::default();
        let link = "switchboard://pair/1?h=aa";
        let Some((run, _)) = j.start(link) else { return assert!(read_input(link).is_err(), "not a link here") };
        assert!(!j.needs_address());
        let no = NodeError::Said { status: 409, kind: Some("link_no_relay".into()), message: "x".into() };
        assert!(j.finish(run, Err(no)));
        assert!(j.needs_address());
        assert!(!JoinProblem::NeedsAddress.says().contains("--"));
        assert!(!j.line().unwrap().1, "asked for, not said as an error");
        // Nothing typed, then something that is not an address: said under the field, and nothing is sent.
        assert_eq!(j.start_with(link, " "), None);
        assert!(j.address_problem().unwrap().contains("home computer's address"));
        assert_eq!(j.start_with(link, "192.0.2.20"), None);
        assert!(j.address_problem().unwrap().contains("192.0.2.20:41641"));
        assert!(j.address_edited() && j.address_problem().is_none());
        let (run, input, addr) = j.start_with(link, " 192.0.2.20:41641 ").unwrap();
        assert_eq!((input.as_str(), addr.as_str()), (link, "192.0.2.20:41641"));
        assert!(j.needs_address(), "the field stays while the address is tried");
        assert!(
            j.finish(run, Err(NodeError::Said { status: 409, kind: Some("unreachable".into()), message: "x".into() }))
        );
        assert!(j.needs_address());
        j.reset();
        assert!(!j.needs_address());
    }

    #[test]
    fn an_address_is_an_ip_and_a_port() {
        assert_eq!(address_input("192.0.2.20:41641"), Ok("192.0.2.20:41641".into()));
        assert_eq!(address_input("[fe80::1]:41641"), Ok("[fe80::1]:41641".into()));
        for bad in ["", "studio.local:41641", "192.0.2.20", "192.0.2.20:0", "http://192.0.2.20:41641"] {
            let e = address_input(bad).unwrap_err();
            assert!(e.ends_with(|c: char| c == '.' || c.is_ascii_digit()) && !e.contains('!'), "{e}");
        }
        let n =
            |addrs: &[&str]| NodeStatus { addrs: addrs.iter().map(|a| a.to_string()).collect(), ..Default::default() };
        assert_eq!(home_address(&n(&["127.0.0.1:1", "[fe80::1]:2", "192.0.2.20:3"])).as_deref(), Some("192.0.2.20:3"));
        assert_eq!(home_address(&n(&["[2001:db8::1]:2"])).as_deref(), Some("[2001:db8::1]:2"));
        assert_eq!(home_address(&n(&["0.0.0.0:5", "nonsense"])), None);
    }

    #[test]
    fn this_computers_name_is_the_first_part_of_its_host_name() {
        assert_eq!(computer_name("Sams-Netbook-Pro.local\n").as_deref(), Some("Sams-Netbook-Pro"));
        assert_eq!(computer_name("tower").as_deref(), Some("tower"));
        assert_eq!(computer_name("localhost"), None);
        assert_eq!(computer_name("  \n"), None);
        assert_eq!(join_body("ABCD-EFGH", Some("  "), &[], false), json!({"input": "ABCD-EFGH"}));
        assert_eq!(join_body("ABCD-EFGH", None, &[], true), json!({"input": "ABCD-EFGH", "replace": true}));
    }

    #[test]
    fn a_computer_with_another_home_is_offered_replace_and_only_then_is_it_sent() {
        let other = || NodeError::Said { status: 409, kind: Some("other_home".into()), message: "node words".into() };
        let node = FakeNode::with(NodeStatus::default());
        node.state.lock().unwrap().joins =
            vec![Err(other()), Ok(Joined { home: Some("Studio".into()), machine: None })];
        let mut j = Join::default();
        let (run, input) = j.start("ABCD-EFGH").unwrap();
        assert!(!j.replaces());
        let res = join_home(&node, &input, None, &[], j.replaces(), 1, Duration::ZERO).map(|(joined, _)| joined);
        assert!(j.finish(run, res));
        assert_eq!(j.step(), &JoinStep::Failed(JoinProblem::OtherHome));
        assert!(j.replaces() && !JoinProblem::OtherHome.retype());
        let (said, bad) = j.line().unwrap();
        assert!(bad);
        assert_eq!(
            said,
            "This computer is paired with another home computer already, so nothing was changed. To pair it with \
             this one instead, choose Replace."
        );
        // Replace: the same thing again, and this time the node is told
        let (run, input) = j.start("ABCD-EFGH").unwrap();
        assert!(j.replaces(), "still so while it runs");
        let res = join_home(&node, &input, None, &[], j.replaces(), 1, Duration::ZERO).map(|(joined, _)| joined);
        assert!(j.finish(run, res));
        assert_eq!(j.step(), &JoinStep::Joined(Some("Studio".into())));
        assert!(!j.replaces());
        let bodies = node.state.lock().unwrap().join_bodies.clone();
        assert_eq!(bodies, vec![json!({"input": "ABCD-EFGH"}), json!({"input": "ABCD-EFGH", "replace": true})]);

        // typing something else takes the offer back: another code may be another home's
        let mut j = Join::default();
        let (run, _) = j.start("ABCD-EFGH").unwrap();
        j.finish(run, Err(other()));
        assert!(j.replaces() && j.edited());
        assert!(!j.replaces() && j.step() == &JoinStep::Idle);
        let (run, _) = j.start("ABCD-EFGH").unwrap();
        j.finish(run, Err(other()));
        j.reset();
        assert!(!j.replaces());
    }

    // ---------- turning keys on, and the route's answers ----------

    #[test]
    fn keys_can_be_turned_on_from_the_home_computer_when_the_server_can() {
        let off = |switch: bool, here: bool| Keys {
            server: Some(read_readiness(&json!({"keys": "off", "switch": switch, "here": here}))),
            node: None,
        };
        assert!(off(true, true).can_turn_on() && off(true, true).devices_page());
        assert!(!off(true, true).on(), "nothing else shows as if keys were on");
        assert!(!off(false, true).devices_page(), "a server from before the route: hidden as it was");
        assert!(!off(true, false).devices_page(), "another computer does not turn the home computer's keys on");
        let old = Keys { server: Some(read_readiness(&json!({"keys": "off"}))), node: None };
        assert!(!old.can_turn_on() && !old.devices_page());
        let both =
            Keys { server: Some(read_readiness(&json!({"keys": "both", "switch": true, "here": true}))), node: None };
        assert!(!both.can_turn_on() && both.devices_page());
        assert!(!Keys::default().devices_page());
        assert_eq!(CLOSE_PORT_ROUTE, Some("/api/keys/mode"));
        for s in [TURN_ON_TITLE, TURN_ON_BODY, TURN_ON_BUTTON, TURN_ON_BUSY, RESTARTING, CLOSING, RELAY_HINT] {
            assert!(!s.contains('\u{2014}') && !s.contains('!'), "{s}");
        }
        assert_eq!(TURN_ON_BODY.matches(". ").count() + 1, 2, "two sentences");
    }

    #[test]
    fn the_routes_answers_are_read_as_the_server_sends_them() {
        assert_eq!(read_mode_answer(200, br#"{"ok": true, "keys": "both", "restarting": true}"#), Some(Ok(true)));
        assert_eq!(read_mode_answer(200, br#"{"ok": true, "keys": "both", "restarting": false}"#), Some(Ok(false)));
        // Not ready: the lead once, and every blocker.
        let r = read_mode_answer(
            409,
            br#"{"ok": false, "error": "Not yet: switching to keys only now would cut something off. Tower is not paired.",
                "blockers": [{"kind": "machine", "text": "Tower is not paired."}, {"kind": "watching", "text": "Watched for 2 of 7 days."}]}"#,
        );
        assert_eq!(
            r,
            Some(Err(ModeRefusal {
                text: "Not yet: switching to keys only now would cut something off.".into(),
                command: None,
                blockers: vec!["Tower is not paired.".into(), "Watched for 2 of 7 days.".into()],
            }))
        );
        // It cannot restart itself: its sentence, and the command apart.
        let r = read_mode_answer(
            409,
            br#"{"ok": false, "error": "Nothing was changed. This server was started by hand from a copy of the code, so it cannot restart itself. To turn keys on, run this on the home computer, then start the server again: python3 /srv/switchboard/server/keys.py on", "command": "python3 /srv/switchboard/server/keys.py on"}"#,
        )
        .unwrap()
        .unwrap_err();
        assert_eq!(r.command.as_deref(), Some("python3 /srv/switchboard/server/keys.py on"));
        assert_eq!(
            r.text,
            "Nothing was changed. This server was started by hand from a copy of the code, so it cannot restart itself. To turn keys on, run this on the home computer, then start the server again:"
        );
        // A server on Windows: the PowerShell line, with its quotes and backslashes.
        let win = read_mode_answer(
            409,
            br#"{"ok": false, "error": "Nothing was changed. On Windows the server cannot restart itself from the app. To turn keys on, run this in PowerShell on the home computer: & \"$env:LOCALAPPDATA\\Switchboard\\switchboard.ps1\" keys on", "command": "& \"$env:LOCALAPPDATA\\Switchboard\\switchboard.ps1\" keys on"}"#,
        )
        .unwrap()
        .unwrap_err();
        assert_eq!(win.command.as_deref(), Some(r#"& "$env:LOCALAPPDATA\Switchboard\switchboard.ps1" keys on"#));
        assert!(win.text.ends_with("run this in PowerShell on the home computer:"), "{}", win.text);
        // The earlier sentence, from a server not yet updated.
        let was = read_mode_answer(
            409,
            br#"{"ok": false, "error": "This server cannot restart itself from here. It was started by hand. To change this, run on the home computer: switchboard keys on", "command": "switchboard keys on"}"#,
        )
        .unwrap()
        .unwrap_err();
        assert_eq!(was.command.as_deref(), Some("switchboard keys on"));
        // The same from a server that sent no "command".
        let old = read_mode_answer(
            409,
            br#"{"ok": false, "error": "This server cannot restart itself from here. To change this, run on the home computer: switchboard keys only"}"#,
        )
        .unwrap()
        .unwrap_err();
        assert_eq!(old.command.as_deref(), Some("switchboard keys only"));
        // A refusal with nothing to run.
        let plain = read_mode_answer(409, br#"{"ok": false, "error": "The settings file cannot be written."}"#);
        assert_eq!(
            plain.unwrap().unwrap_err(),
            ModeRefusal { text: "The settings file cannot be written.".into(), ..Default::default() }
        );
        // Anything else is not this route's answer.
        assert_eq!(read_mode_answer(404, b"Not Found"), None);
        assert_eq!(read_mode_answer(400, br#"{"ok": false, "error": "keys must be off, both or only"}"#), None);
        assert_eq!(read_mode_answer(200, b"<html>"), None);
    }

    // ---------- the relay line ----------

    #[test]
    fn the_relay_line_says_which_relay_and_where_it_is_from() {
        let home = |url: Option<&str>, source: &str, connected: Option<bool>| NodeStatus {
            keys: true,
            role: Role::Home,
            relay: url.map(str::to_string),
            relay_source: Some(source.into()),
            relay_connected: connected,
            ..Default::default()
        };
        let l = relay_line(&home(Some("https://relay.example.com/"), "settings", Some(true))).unwrap();
        assert_eq!(
            (l.value.as_str(), l.note.as_str(), l.tone, l.can_change),
            ("relay.example.com", "Set on this computer", Tone::Quiet, true)
        );
        let l = relay_line(&home(Some("https://relay.example.com"), "built in", Some(false))).unwrap();
        assert_eq!((l.note.as_str(), l.tone), ("The one Switchboard comes with. Not connected", Tone::Bad));
        // It leads with what a relay is for; the address comes second.
        assert_eq!(l.label, "The relay helps computers on other networks find this one.");
        let l = relay_line(&home(Some("http://10.0.0.5:3340"), "environment", None)).unwrap();
        assert!(!l.can_change && l.value == "10.0.0.5:3340");
        // Set where Switchboard starts: no Change here, and the line says where it is changed.
        assert_eq!(l.note, "Set where Switchboard starts, so it is changed there");
        let none = relay_line(&home(None, "none", Some(false))).unwrap();
        assert_eq!((none.value.as_str(), none.tone, none.can_change), ("None set", Tone::Quiet, true));
        assert_eq!(none.label, "A relay helps computers on other networks find this one.");
        assert!(none.note.contains("Short codes need a relay"));
        // Under the open field: what the label has said already is not said again.
        assert!(!RELAY_HINT.contains("helps computers"));
        // Only on the home computer.
        let mut member = home(Some("https://relay.example.com"), "home", Some(true));
        member.role = Role::Member;
        assert_eq!(relay_line(&member), None);
        assert_eq!(relay_line(&NodeStatus::default()), None);
        // The node's status, read.
        let s = read_status(&json!({"ok": true, "role": "home", "id": "h1", "addrs": ["192.0.2.20:41641"],
            "relay": {"url": "https://relay.example", "source": "settings", "connected": true}}));
        assert_eq!((s.relay_source.as_deref(), s.relay_connected), (Some("settings"), Some(true)));
        assert_eq!(home_address(&s).as_deref(), Some("192.0.2.20:41641"));
        assert!(Keys { server: None, node: Some(s) }.home_here());
    }

    #[test]
    fn a_typed_relay_is_sent_as_an_address_or_cleared() {
        assert_eq!(relay_input("  "), Ok(String::new()));
        assert_eq!(relay_input("relay.example.com"), Ok("https://relay.example.com".into()));
        assert_eq!(relay_input(" http://10.0.0.5:3340/ "), Ok("http://10.0.0.5:3340/".into()));
        assert!(relay_input("ftp://relay.example.com").is_err());
        assert!(relay_input("https://").is_err());
        assert!(relay_input("two words").is_err());
    }

    #[test]
    fn keys_not_running_keeps_the_nodes_kind() {
        let off = read_answer(503, br#"{"ok": false, "error": "Keys are off on this computer.", "kind": "off"}"#);
        let s = status_from(off).unwrap();
        assert_eq!(s.problems[0].kind.as_deref(), Some("off"));
        assert!(Keys { server: None, node: Some(s) }.offer_join(), "a fresh computer can join");
    }

    #[test]
    fn a_join_that_was_left_ignores_its_late_answer() {
        let mut j = Join::default();
        let (run, _) = j.start("ABCD-EFGH").unwrap();
        j.reset();
        assert!(!j.finish(run, Ok(Joined::default())));
        assert_eq!(j.step(), &JoinStep::Idle);
    }

    #[test]
    fn the_fake_node_answers_and_remembers() {
        let absent = FakeNode::absent();
        assert_eq!(absent.status(), Err(NodeError::Absent));
        assert_eq!(absent.join("ABCD-EFGH", None, &[], false), Err(NodeError::Absent));

        let node = FakeNode::with(NodeStatus { keys: true, ..Default::default() });
        node.state.lock().unwrap().joins.push(Err(NodeError::Said {
            status: 403,
            kind: Some("expired".into()),
            message: "gone".into(),
        }));
        node.state.lock().unwrap().devices = vec![dev("a", "Netbook", "computer", None, None)];
        let mut j = Join::default();
        let (run, input) = j.start("abcd efgh").unwrap();
        j.finish(run, node.join(&input, None, &[], false));
        assert_eq!(j.step(), &JoinStep::Failed(JoinProblem::Expired));
        let (run, input) = j.start("abcd efgh").unwrap();
        j.finish(run, node.join(&input, Some(" Laptop "), &["192.0.2.20:41641".to_string()], false));
        assert!(matches!(j.step(), JoinStep::Joined(_)));
        assert_eq!(node.devices().unwrap().len(), 1);
        node.remove_device("a").unwrap();
        assert!(node.devices().unwrap().is_empty());
        node.set_relay("https://relay.example").unwrap();
        assert!(node.pair_new().is_err(), "no offer was set");
        let s = node.state.lock().unwrap();
        assert_eq!(s.relay.as_deref(), Some("https://relay.example"));
        assert_eq!(
            s.join_bodies,
            vec![
                json!({"input": "ABCD-EFGH"}),
                json!({"input": "ABCD-EFGH", "name": "Laptop", "addrs": ["192.0.2.20:41641"]})
            ],
            "the name and the address go only when there is one"
        );
        assert_eq!(
            s.calls,
            vec![
                "join ABCD-EFGH",
                "join ABCD-EFGH",
                "devices",
                "remove a",
                "devices",
                "relay https://relay.example",
                "pair_new"
            ]
        );
    }

    // ---------- how a computer is connected ----------

    #[test]
    fn the_home_line_says_direct_relay_or_why_not() {
        let member = |peers: Vec<Peer>, problems: Vec<Problem>| NodeStatus {
            keys: true,
            role: Role::Member,
            home: Some(Home { id: "h1".into(), name: "Studio".into(), ..Default::default() }),
            peers,
            problems,
            ..Default::default()
        };
        let peer = |id: &str, path| Peer { id: id.into(), name: String::new(), path, rtt_ms: None };
        let direct = home_line(&member(vec![peer("other", Some(Path::Relay)), peer("h1", Some(Path::Direct))], vec![]));
        assert_eq!(direct, Some(LinkLine { text: "Direct".into(), tone: Tone::Quiet, fix: None }));
        assert_eq!(home_line(&member(vec![peer("h1", Some(Path::Relay))], vec![])).unwrap().text, "Through the relay");
        assert_eq!(home_line(&member(vec![peer("h1", None)], vec![])).unwrap().text, "Connecting to home");

        let why = home_line(&member(
            vec![],
            vec![Problem { kind: None, text: "The relay at relay.example did not answer".into() }],
        ))
        .unwrap();
        assert_eq!(why.text, "Can't reach home: The relay at relay.example did not answer.");
        assert_eq!(why.tone, Tone::Bad);
        assert_eq!(why.fix, None);

        let mac = home_line(&member(
            vec![],
            vec![
                Problem { kind: None, text: "Something else.".into() },
                Problem {
                    kind: Some("local_network".into()),
                    text: "Local Network access is off for the node.".into(),
                },
            ],
        ))
        .unwrap();
        assert_eq!(mac.text, "Can't reach home: macOS is blocking the local network.");
        let fix = mac.fix.as_deref().unwrap();
        assert!(fix.contains("Privacy & Security > Local Network") && fix.contains("switchboard-node"));
        assert!(!fix.contains("blocking"), "what is blocked is said once, in the line above: {fix}");
        // A problem while the path is up is not this line's to say.
        let fine = home_line(&member(
            vec![peer("h1", Some(Path::Relay))],
            vec![Problem { kind: None, text: "Direct paths are blocked.".into() }],
        ));
        assert_eq!(fine.unwrap().tone, Tone::Quiet);

        let mut home = member(vec![], vec![]);
        home.role = Role::Home;
        assert_eq!(home_line(&home), None, "the home computer does not reach itself");
        let mut off = member(vec![peer("h1", Some(Path::Direct))], vec![]);
        off.keys = false;
        assert_eq!(home_line(&off), None);
        assert_eq!(home_line(&NodeStatus { keys: true, ..Default::default() }), None, "not joined yet");
    }

    // ---------- moving to keys ----------

    fn blocker(kind: &str, f: impl FnOnce(&mut Blocker)) -> Blocker {
        let mut b = Blocker { kind: kind.into(), text: format!("server words for {kind}"), ..Default::default() };
        f(&mut b);
        b
    }

    #[test]
    fn the_card_lists_what_is_left() {
        let r = Readiness {
            keys: Mode::Both,
            ready: false,
            days: 7,
            blockers: vec![
                blocker("machine", |b| (b.label, b.paired) = (Some("Tower".into()), Some(false))),
                blocker("machine", |b| (b.label, b.paired) = (Some("Mini".into()), Some(false))),
                blocker("machine", |b| (b.label, b.paired) = (Some("Netbook".into()), Some(true))),
                blocker("watching", |b| b.watched_days = Some(3)),
                blocker("direct", |b| (b.client, b.last) = (Some("iPhone".into()), Some(NOW - 60.))),
                blocker("direct", |b| (b.client, b.last) = (Some("desktop".into()), Some(NOW - 7200.))),
                blocker("direct", |b| {
                    (b.client, b.last, b.paired_since) = (Some("Android".into()), Some(NOW - 7200.), true)
                }),
                blocker("settings", |_| {}),
            ],
            ..Default::default()
        };
        let m = moving(&r, NOW).unwrap();
        assert!(!m.ready);
        assert_eq!(m.title, "Moving to keys");
        assert_eq!(
            m.left,
            vec![
                "2 computers aren't paired yet: Tower and Mini. Update Switchboard on them, or add them with a code.",
                "Netbook is paired but hasn't connected with its key yet.",
                "Connections by address have been watched for 3 of 7 days.",
                "An iPhone connected by address 1m ago. Pair it with the home computer in the app first.",
                "A desktop app connected by address 2h ago.",
                "An Android phone connected by address 2h ago. A paired phone has connected with its key since.",
                "server words for settings",
            ]
        );
        let one = Readiness {
            keys: Mode::Both,
            blockers: vec![blocker("machine", |b| (b.label, b.paired) = (Some("Tower".into()), Some(false)))],
            ..Default::default()
        };
        assert_eq!(
            moving(&one, NOW).unwrap().left,
            vec!["Tower isn't paired yet. Update Switchboard on it, or add it with a code."]
        );
        for line in m.left.iter().chain([&m.body]) {
            assert!(!line.contains('\u{2014}') && !line.contains('!'), "{line}");
        }
    }

    #[test]
    fn the_card_says_when_the_port_can_close_and_goes_when_it_has() {
        let ready = Readiness { keys: Mode::Both, ready: true, days: 7, ..Default::default() };
        let m = moving(&ready, NOW).unwrap();
        assert!(m.ready && m.left.is_empty());
        assert_eq!(m.title, "Ready to close the network port");
        assert!(m.body.contains("7 days"));
        assert_eq!(moving(&Readiness { keys: Mode::Only, ready: true, ..Default::default() }, NOW), None);
        assert_eq!(moving(&Readiness { keys: Mode::Off, ..Default::default() }, NOW), None);
        assert!(CLOSE_EXPLAINS.contains("paired devices only"));
        assert!(CLOSE_BY_HAND.contains("switchboard keys only") && !CLOSE_BY_HAND.contains("server.json"));
    }
}
