//! Terminals (spec v2 section 1): the ticket from POST /api/term, the WebSocket to the hub, and the bytes each key
//! sends. Nothing here draws; the desktop app keeps the VT state and paints it.
//!
//! - `ticket`: asks the server for a one-use ticket and reads its `need` errors.
//! - `ws`: one thread per open terminal speaking the hub's protocol (binary = bytes, text = JSON control).
//! - `keys`: a key plus modifiers to the bytes xterm sends, aware of application cursor mode.
//! - `reconnect`: when a dropped terminal comes back by itself, and how long it waits between tries.
//! - `rec`: the recorder (SB_TERM_REC=1) and its capture format.
//! - `screen` (feature `grid`): a screen as bytes, the snapshot the hub sends and the desktop app replays.

pub mod keys;
pub mod rec;
pub mod reconnect;
pub mod scheme;
#[cfg(feature = "grid")]
pub mod screen;
pub mod ws;

use serde::{Deserialize, Serialize};

use crate::net::CoreError;

/// The hub listens on this port on the same host as the server.
pub const HUB_PORT: u16 = 8132;

/// POST /api/term: either the chat (`sid`) or, after a resume, its background id (`bg`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TermRequest {
    pub machine: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sid: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bg: Option<String>,
}

impl TermRequest {
    pub fn chat(machine: &str, sid: &str) -> TermRequest {
        TermRequest { machine: machine.into(), sid: Some(sid.into()), bg: None }
    }
    pub fn bg(machine: &str, bg: &str) -> TermRequest {
        TermRequest { machine: machine.into(), sid: None, bg: Some(bg.into()) }
    }
}

/// The server's reply, every field optional.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ApiTerm {
    #[serde(default)]
    pub ok: Option<bool>,
    #[serde(default)]
    pub ws: Option<String>,
    #[serde(default)]
    pub bg: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub machine: Option<String>,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub need: Option<String>,
}

/// A ticket ready to dial. `ws` is absolute and has no size yet (see `ws_url`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TermTicket {
    pub ws: String,
    pub bg: Option<String>,
    pub title: Option<String>,
    pub machine: String,
    pub label: Option<String>,
}

/// Why the server would not hand out a ticket, in the spec's terms.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Need {
    /// The chat is not running: offer "Resume and open".
    Resume,
    /// It is open in an interactive terminal on its machine; Claude Code only attaches background chats.
    Terminal,
    /// The machine is offline.
    Offline,
}

impl Need {
    pub fn parse(s: &str) -> Option<Need> {
        match s {
            "resume" => Some(Need::Resume),
            "terminal" => Some(Need::Terminal),
            "offline" => Some(Need::Offline),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TermError {
    Need { need: Need, message: String },
    Core(CoreError),
}

impl std::fmt::Display for TermError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TermError::Need { message, .. } => write!(f, "{message}"),
            TermError::Core(e) => write!(f, "{e}"),
        }
    }
}

impl From<CoreError> for TermError {
    fn from(e: CoreError) -> Self {
        TermError::Core(e)
    }
}

pub const NO_TERMINALS: &str = "This Switchboard server does not have terminals yet.";

/// Reads a /api/term reply. `base` is the server that answered ("http://192.0.2.10:8130"); a relative `ws`
/// is completed with its host and the hub port.
pub fn parse_ticket(status: u16, body: &[u8], base: &str, req: &TermRequest) -> Result<TermTicket, TermError> {
    let reply: Option<ApiTerm> = serde_json::from_slice(body).ok();
    let failed = !(200..300).contains(&status) || reply.as_ref().and_then(|r| r.ok) == Some(false);
    if failed {
        let r = reply.unwrap_or_default();
        if let Some(need) = r.need.as_deref().and_then(Need::parse) {
            let message = r.error.unwrap_or_else(|| default_need_message(need).to_string());
            return Err(TermError::Need { need, message });
        }
        if status == 404
            && r.need.is_none()
            && r.error.as_deref().map(|e| e.contains("not found") || e.is_empty()).unwrap_or(true)
        {
            return Err(TermError::Core(CoreError::Server { status, message: NO_TERMINALS.into() }));
        }
        let message = r.error.unwrap_or_else(|| format!("Request failed ({status})"));
        return Err(TermError::Core(CoreError::Server { status, message }));
    }
    let r = reply.ok_or_else(|| TermError::Core(CoreError::Bad("Unexpected reply from /api/term".into())))?;
    let ws =
        r.ws.as_deref()
            .filter(|w| !w.is_empty())
            .ok_or_else(|| TermError::Core(CoreError::Bad("The server sent no terminal address.".into())))?;
    Ok(TermTicket {
        ws: absolute_ws(ws, base),
        bg: r.bg.filter(|b| !b.is_empty()).or_else(|| req.bg.clone()),
        title: r.title.filter(|t| !t.is_empty()),
        machine: r.machine.unwrap_or_else(|| req.machine.clone()),
        label: r.label.filter(|l| !l.is_empty()),
    })
}

fn default_need_message(need: Need) -> &'static str {
    match need {
        Need::Resume => "This chat is not running.",
        Need::Terminal => "This chat is open in a terminal on its machine, so it can't be opened here.",
        Need::Offline => "That machine is offline.",
    }
}

/// The host part of "http://host:port/...".
pub fn host_of(base: &str) -> &str {
    let rest = base.split_once("://").map(|(_, r)| r).unwrap_or(base);
    let hostport = rest.split('/').next().unwrap_or(rest);
    if let Some(stripped) = hostport.strip_prefix('[') {
        // [v6]:port
        return stripped.split(']').next().unwrap_or(stripped);
    }
    hostport.rsplit_once(':').map(|(h, _)| h).unwrap_or(hostport)
}

/// `ws` as the server sent it, made absolute against the server the client used.
pub fn absolute_ws(ws: &str, base: &str) -> String {
    if ws.starts_with("ws://") || ws.starts_with("wss://") {
        return ws.to_string();
    }
    let host = host_of(base);
    let host = if host.contains(':') { format!("[{host}]") } else { host.to_string() };
    let path = if ws.starts_with('/') { ws.to_string() } else { format!("/{ws}") };
    format!("ws://{host}:{HUB_PORT}{path}")
}

/// The ticket URL with the terminal size the hub should start the PTY at.
/// The ticket's WebSocket address with the size this view has room for. `proto=2`: this client takes size frames and
/// snapshots. `history=0` (a reconnect that kept its own scrollback): the first snapshot is only the screen.
pub fn ws_url(ticket_ws: &str, cols: u16, rows: u16, history: bool) -> String {
    let sep = if ticket_ws.contains('?') { '&' } else { '?' };
    let h = if history { "" } else { "&history=0" };
    format!("{ticket_ws}{sep}cols={cols}&rows={rows}&proto=2{h}")
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: &str = "http://192.0.2.10:8130";

    fn req() -> TermRequest {
        TermRequest::chat("studio", "0f9c")
    }

    #[test]
    fn request_has_sid_or_bg_only() {
        assert_eq!(
            serde_json::to_string(&TermRequest::chat("studio", "abc")).unwrap(),
            r#"{"machine":"studio","sid":"abc"}"#
        );
        assert_eq!(
            serde_json::to_string(&TermRequest::bg("studio", "c03ef0d9")).unwrap(),
            r#"{"machine":"studio","bg":"c03ef0d9"}"#
        );
    }

    #[test]
    fn reads_a_ticket() {
        let body = br#"{"ok":true,"ws":"ws://192.0.2.10:8132/term?t=abc.def","bg":"c03ef0d9","title":"Fix login bug","machine":"studio","label":"Studio"}"#;
        let t = parse_ticket(200, body, BASE, &req()).unwrap();
        assert_eq!(t.ws, "ws://192.0.2.10:8132/term?t=abc.def");
        assert_eq!(t.bg.as_deref(), Some("c03ef0d9"));
        assert_eq!(t.title.as_deref(), Some("Fix login bug"));
        assert_eq!(t.label.as_deref(), Some("Studio"));
        assert_eq!(ws_url(&t.ws, 120, 36, true), "ws://192.0.2.10:8132/term?t=abc.def&cols=120&rows=36&proto=2");
        assert_eq!(
            ws_url(&t.ws, 80, 24, false),
            "ws://192.0.2.10:8132/term?t=abc.def&cols=80&rows=24&proto=2&history=0"
        );
    }

    #[test]
    fn relative_ws_follows_the_server_host() {
        let body = br#"{"ok":true,"ws":"/term?t=x"}"#;
        let t = parse_ticket(200, body, "http://100.64.0.10:8130", &req()).unwrap();
        assert_eq!(t.ws, "ws://100.64.0.10:8132/term?t=x");
        assert_eq!(t.machine, "studio");
        assert_eq!(absolute_ws("term?t=y", "http://[fd7a::1]:8130"), "ws://[fd7a::1]:8132/term?t=y");
        assert_eq!(host_of("http://localhost:8161"), "localhost");
    }

    #[test]
    fn need_errors_keep_their_kind_and_words() {
        let body = br#"{"ok":false,"error":"This chat is open in a terminal on Studio, so it can't be opened here.","need":"terminal"}"#;
        assert_eq!(
            parse_ticket(409, body, BASE, &req()),
            Err(TermError::Need {
                need: Need::Terminal,
                message: "This chat is open in a terminal on Studio, so it can't be opened here.".into()
            })
        );
        let body = br#"{"ok":false,"need":"resume"}"#;
        assert!(matches!(parse_ticket(200, body, BASE, &req()), Err(TermError::Need { need: Need::Resume, .. })));
        let body = br#"{"ok":false,"error":"Studio is offline","need":"offline"}"#;
        assert_eq!(parse_ticket(503, body, BASE, &req()).unwrap_err().to_string(), "Studio is offline");
    }

    #[test]
    fn old_server_and_plain_errors() {
        let e = parse_ticket(404, br#"{"ok":false,"error":"not found"}"#, BASE, &req()).unwrap_err();
        assert_eq!(e.to_string(), NO_TERMINALS);
        let e = parse_ticket(404, b"<html>", BASE, &req()).unwrap_err();
        assert_eq!(e.to_string(), NO_TERMINALS);
        let e = parse_ticket(400, br#"{"ok":false,"error":"bad session id"}"#, BASE, &req()).unwrap_err();
        assert_eq!(e, TermError::Core(CoreError::Server { status: 400, message: "bad session id".into() }));
        let e = parse_ticket(200, br#"{"ok":true}"#, BASE, &req()).unwrap_err();
        assert!(matches!(e, TermError::Core(CoreError::Bad(_))));
    }
}
