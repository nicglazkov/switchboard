//! HTTP to the Switchboard server, with the same fallback across saved addresses as the iPhone app
//! (ios/Shared/HostResolver.swift): every candidate is probed at once, the earliest one that answers wins, and once
//! a later one answers the earlier ones get only a short grace, so a LAN probe hanging away from home costs nothing.
//! Nothing is built in: with no saved address the server is looked for on the local network (discover.rs).

use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::api::{
    ApiBrowse, ApiChatSummary, ApiChats, ApiFiles, ApiGroupsReply, ApiPair, ApiFolders, ApiJob, ApiKeep, ApiPostReply, ApiPrefs, ApiRoute, ApiSessions, ApiSummary, ApiThread,
};

/// The server's port when an address leaves it out.
pub const PORT: u16 = 8130;

/// How long the local network search waits for answers.
pub const BROWSE_FOR: Duration = Duration::from_millis(1500);

/// A LAN address answers at once or not at all; anything else (a Tailscale address can take a few seconds to wake
/// after a network change) gets longer.
pub fn probe_timeout(base: &str) -> Duration {
    if is_lan(base) { Duration::from_millis(2500) } else { Duration::from_secs(6) }
}

/// Whether the address is on the local network: a private IPv4 address or a .local name.
pub fn is_lan(base: &str) -> bool {
    let rest = base.split_once("://").map(|(_, r)| r).unwrap_or(base);
    let host = rest.split(['/', ':']).next().unwrap_or("");
    if host.to_ascii_lowercase().ends_with(".local") {
        return true;
    }
    host.parse::<std::net::Ipv4Addr>().is_ok_and(|ip| ip.is_private() || ip.is_link_local())
}

/// A server address as typed ("192.0.2.10", "studio.local:8130", "https://sb.example.com/") as a base URL:
/// http:// when no scheme is given, the default port when none is, no trailing slash. None when it cannot be one.
pub fn normalize_base(typed: &str) -> Option<String> {
    let t = typed.trim().trim_end_matches('/');
    let (scheme, rest) = match t.split_once("://") {
        Some((s, r)) if s.eq_ignore_ascii_case("http") || s.eq_ignore_ascii_case("https") => (s.to_ascii_lowercase(), r),
        Some(_) => return None,
        None => ("http".to_string(), t),
    };
    if rest.is_empty() || rest.contains(char::is_whitespace) || rest.contains(['/', '?', '#', '@']) {
        return None;
    }
    let host_ok = |h: &str| !h.is_empty() && h.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'));
    match rest.rsplit_once(':') {
        Some((h, p)) if host_ok(h) && p.parse::<u16>().is_ok_and(|p| p > 0) => Some(format!("{scheme}://{h}:{p}")),
        Some(_) => None,
        None if host_ok(rest) && scheme == "https" => Some(format!("https://{rest}")),
        None if host_ok(rest) => Some(format!("http://{rest}:{PORT}")),
        None => None,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoreError {
    /// No candidate answered, or the connection dropped.
    Unreachable,
    /// The server answered with an error.
    Server { status: u16, message: String },
    /// The reply was not what Switchboard sends.
    Bad(String),
    /// Nothing was done: the chat still has work running that this would cut off (subagents, background commands,
    /// a reply). Ask, then send again with `when` "idle" (queued until they finish) or "now".
    Busy { message: String, busy: Vec<BusyInfo> },
}

/// What is still running in one chat (the server's need "busy" answer).
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Deserialize)]
pub struct BusyInfo {
    #[serde(default)]
    pub subagents: Vec<String>,
    #[serde(default)]
    pub commands: Vec<String>,
    #[serde(default)]
    pub replying: bool,
    #[serde(default)]
    pub pending: Vec<String>,
    #[serde(default)]
    pub summary: String,
    /// On a list of chats (the cleanup card): which one.
    #[serde(default)]
    pub title: Option<String>,
}

impl BusyInfo {
    /// Only a reply in progress: "when it finishes" rather than "when they finish".
    pub fn only_reply(&self) -> bool {
        self.replying && self.subagents.is_empty() && self.commands.is_empty() && self.pending.is_empty()
    }
}

impl std::fmt::Display for CoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CoreError::Unreachable => {
                write!(f, "Switchboard is not reachable. Check that the computer that runs Switchboard is on and that this one can reach it.")
            }
            CoreError::Server { message, .. } => write!(f, "{message}"),
            CoreError::Bad(m) => write!(f, "{m}"),
            CoreError::Busy { message, .. } => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for CoreError {}

pub type Probe = Arc<dyn Fn(&str) -> bool + Send + Sync>;

/// The earliest candidate whose probe says yes. All are asked at once; once a later one says yes, the earlier ones
/// get `grace` more before the best answer so far wins.
pub fn first_preferred(candidates: &[String], grace: Duration, probe: Probe) -> Option<String> {
    if candidates.is_empty() {
        return None;
    }
    let (tx, rx) = mpsc::channel::<(usize, bool)>();
    for (i, c) in candidates.iter().enumerate() {
        let tx = tx.clone();
        let c = c.clone();
        let probe = probe.clone();
        thread::Builder::new()
            .name(format!("sb-probe-{i}"))
            .spawn(move || {
                let ok = probe(&c);
                let _ = tx.send((i, ok));
            })
            .ok()?;
    }
    drop(tx);
    let mut results: Vec<Option<bool>> = vec![None; candidates.len()];
    let mut deadline: Option<Instant> = None;
    loop {
        let got = match deadline {
            Some(d) => {
                let left = d.saturating_duration_since(Instant::now());
                match rx.recv_timeout(left) {
                    Ok(v) => Some(v),
                    Err(mpsc::RecvTimeoutError::Timeout) => None,
                    Err(mpsc::RecvTimeoutError::Disconnected) => None,
                }
            }
            None => rx.recv().ok(),
        };
        let Some((i, ok)) = got else {
            // grace is over (or every probe finished): the best answer so far wins
            return results.iter().position(|r| *r == Some(true)).map(|i| candidates[i].clone());
        };
        results[i] = Some(ok);
        for (j, r) in results.iter().enumerate() {
            match r {
                None => break,
                Some(true) => return Some(candidates[j].clone()),
                Some(false) => {}
            }
        }
        if ok && deadline.is_none() {
            deadline = Some(Instant::now() + grace);
        }
    }
}

/// Looks for servers on the local network: their addresses, best first.
pub type Discover = Arc<dyn Fn() -> Vec<String> + Send + Sync>;

/// Finds the server and remembers it for `ttl`. With no candidates it asks `discover`, when it has one.
pub struct Resolver {
    candidates: Mutex<Vec<String>>,
    ttl: Duration,
    grace: Duration,
    probe: Probe,
    discover: Option<Discover>,
    cached: Mutex<Option<(String, Instant)>>,
}

impl Resolver {
    pub fn new(candidates: Vec<String>, ttl: Duration, probe: Probe) -> Resolver {
        Resolver { candidates: Mutex::new(candidates), ttl, grace: Duration::from_millis(400), probe, discover: None, cached: Mutex::new(None) }
    }

    pub fn with_discovery(mut self, discover: Discover) -> Resolver {
        self.discover = Some(discover);
        self
    }

    /// The saved addresses, in order of preference.
    pub fn candidates(&self) -> Vec<String> {
        self.candidates.lock().map(|c| c.clone()).unwrap_or_default()
    }

    /// New saved addresses (the user typed one): the next call looks again.
    pub fn set_candidates(&self, candidates: Vec<String>) {
        if let Ok(mut c) = self.candidates.lock() {
            *c = candidates;
        }
        self.invalidate();
    }

    pub fn resolve(&self) -> Option<String> {
        if let Some((url, at)) = self.cached.lock().ok()?.as_ref() {
            if at.elapsed() < self.ttl {
                return Some(url.clone());
            }
        }
        let mut candidates = self.candidates();
        if candidates.is_empty()
            && let Some(d) = &self.discover
        {
            candidates = d();
        }
        let found = first_preferred(&candidates, self.grace, self.probe.clone());
        if let Ok(mut c) = self.cached.lock() {
            *c = found.clone().map(|u| (u, Instant::now()));
        }
        found
    }

    /// The last answer, without probing.
    pub fn current(&self) -> Option<String> {
        self.cached.lock().ok()?.as_ref().map(|(u, _)| u.clone())
    }

    pub fn invalidate(&self) {
        if let Ok(mut c) = self.cached.lock() {
            *c = None;
        }
    }
}

/// POST /api/new
#[derive(Debug, Clone, Default, Serialize)]
pub struct NewRequest {
    pub machine: String,
    pub folder: String,
    pub create: bool,
    pub prompt: String,
    /// opus | sonnet | haiku | null (the machine's default)
    pub model: Option<String>,
    /// normal | max | ultracode
    pub mode: String,
    /// v2: low | medium | high | xhigh (None: the machine's default). Sent only when set, so a plain start reads
    /// the same to an older server.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    /// v2: the options as flags, like [["--chrome"], ["--add-dir", "notes"]].
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub flags: Vec<Vec<String>>,
    /// v2: "Other flags", as typed.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub other: String,
    /// This chat's own Chrome machine: Auto sends a Chrome session there, and its continuations keep it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chrome_machine: Option<String>,
    /// Library tools to start with: "auto", or the picked list (an older server adds its defaults to a list). Not
    /// sent when None, for older servers.
    #[serde(skip_serializing_if = "Tools::is_none")]
    pub tools: Tools,
}

/// The tools a new chat asks for.
#[derive(Debug, Clone, Default, PartialEq)]
pub enum Tools {
    /// Say nothing (an older server, and nothing picked).
    #[default]
    None,
    /// The server picks: every skill, and the MCP tools in Auto.
    Auto,
    Pick(Vec<String>),
}

impl Tools {
    pub fn is_none(&self) -> bool {
        matches!(self, Tools::None)
    }
}

impl Serialize for Tools {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            Tools::None => s.serialize_none(),
            Tools::Auto => s.serialize_str("auto"),
            Tools::Pick(v) => v.serialize(s),
        }
    }
}

/// POST /api/continue
#[derive(Debug, Clone, Serialize)]
pub struct ContinueRequest {
    pub machine: String,
    pub sid: String,
    pub to: String,
    pub folder: String,
    pub create: bool,
    /// "idle" or "now", once the user has answered a need "busy".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub when: Option<String>,
}

#[derive(Serialize)]
struct ActionBody<'a> {
    machine: &'a str,
    verb: &'a str,
    sid: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    close: Option<bool>,
    /// v2 keep: on or off.
    #[serde(skip_serializing_if = "Option::is_none")]
    on: Option<bool>,
    /// v2 link: the older chat.
    #[serde(skip_serializing_if = "Option::is_none")]
    to: Option<ChatRef<'a>>,
    /// v2 unlink on an older chat: the chats that carry on from it go their own way.
    #[serde(skip_serializing_if = "Option::is_none")]
    older: Option<bool>,
    /// v2 rename: the new name.
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<&'a str>,
    /// "idle" or "now", once the user has answered a need "busy".
    #[serde(skip_serializing_if = "Option::is_none")]
    when: Option<&'a str>,
}

#[derive(Serialize)]
struct ChatRef<'a> {
    machine: &'a str,
    sid: &'a str,
}

/// POST /api/route
#[derive(Serialize)]
struct RouteBody<'a> {
    prompt: &'a str,
    folder: &'a str,
    rules: bool,
    /// v2: a Chrome session; Auto prefers this chat's Chrome machine.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    chrome: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    chrome_machine: Option<&'a str>,
}

pub struct Client {
    resolver: Resolver,
    agent: ureq::Agent,
    /// v2: for /api/stream, which stays open for ten minutes at a time.
    stream_agent: ureq::Agent,
}

/// The server ends a live stream after ten minutes; past this, a silent one counts as gone.
const STREAM_FOR: Duration = Duration::from_secs(660);

fn agent(timeout: Duration, user_agent: &str) -> ureq::Agent {
    let cfg = ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .timeout_connect(Some(timeout.min(Duration::from_secs(6))))
        .http_status_as_error(false)
        .user_agent(user_agent)
        .build();
    ureq::Agent::new_with_config(cfg)
}

/// Counts only a real Switchboard reply, so some other device at a saved address on another network is skipped.
pub fn http_probe(user_agent: String) -> Probe {
    Arc::new(move |base: &str| {
        let a = agent(probe_timeout(base), &user_agent);
        match a.get(&format!("{base}/api/summary")).header("X-Switchboard", "1").call() {
            Ok(mut r) if r.status().as_u16() == 200 => {
                r.body_mut().read_json::<ApiSummary>().map(|s| s.looks_real()).unwrap_or(false)
            }
            _ => false,
        }
    })
}

impl Client {
    /// `candidates` in order of preference. Empty: look on the local network, then on this computer itself (a
    /// server does not hear its own question on some systems).
    pub fn new(candidates: Vec<String>, user_agent: &str) -> Client {
        let discover: Discover = Arc::new(|| {
            let mut found = crate::discover::browse(BROWSE_FOR);
            found.push(format!("http://127.0.0.1:{PORT}"));
            found
        });
        Client {
            resolver: Resolver::new(candidates, Duration::from_secs(60), http_probe(user_agent.to_string())).with_discovery(discover),
            agent: agent(Duration::from_secs(20), user_agent),
            stream_agent: agent(STREAM_FOR, user_agent),
        }
    }

    pub fn with_resolver(resolver: Resolver, user_agent: &str) -> Client {
        Client { resolver, agent: agent(Duration::from_secs(20), user_agent), stream_agent: agent(STREAM_FOR, user_agent) }
    }

    /// The server in use, if one was found.
    pub fn base(&self) -> Option<String> {
        self.resolver.current()
    }

    pub fn resolver(&self) -> &Resolver {
        &self.resolver
    }

    /// Runs `f` against the resolved server; on a dropped connection it looks for the server again once.
    fn with_base<T>(&self, f: impl Fn(&str) -> Result<T, CoreError>) -> Result<T, CoreError> {
        let base = self.resolver.resolve().ok_or(CoreError::Unreachable)?;
        match f(&base) {
            Err(CoreError::Unreachable) => {
                self.resolver.invalidate();
                let again = self.resolver.resolve().ok_or(CoreError::Unreachable)?;
                f(&again)
            }
            other => other,
        }
    }

    fn get_bytes(&self, base: &str, path: &str) -> Result<Vec<u8>, CoreError> {
        let mut r = self.agent.get(&format!("{base}{path}")).header("X-Switchboard", "1").call().map_err(|_| CoreError::Unreachable)?;
        let status = r.status().as_u16();
        let body = r.body_mut().with_config().limit(64 * 1024 * 1024).read_to_vec().map_err(|_| CoreError::Unreachable)?;
        check(status, &body)?;
        Ok(body)
    }

    /// A POST with the same headers as every other, returning the status and body unchecked, so the caller can
    /// read fields `check` drops (like /api/term's `need`).
    fn post_raw<B: Serialize>(&self, base: &str, path: &str, body: &B) -> Result<(u16, Vec<u8>), CoreError> {
        let mut r = self
            .agent
            .post(&format!("{base}{path}"))
            .header("X-Switchboard", "1")
            .header("Content-Type", "application/json")
            .send_json(body)
            .map_err(|_| CoreError::Unreachable)?;
        let status = r.status().as_u16();
        let out = r.body_mut().read_to_vec().map_err(|_| CoreError::Unreachable)?;
        Ok((status, out))
    }

    fn post_json<B: Serialize>(&self, base: &str, path: &str, body: &B) -> Result<Vec<u8>, CoreError> {
        let (status, out) = self.post_raw(base, path, body)?;
        check(status, &out)?;
        Ok(out)
    }

    /// POST /api/term. Returns the status, the body and the server that answered (the hub shares its host).
    pub fn term(&self, req: &crate::term::TermRequest) -> Result<(u16, Vec<u8>, String), CoreError> {
        self.with_base(|base| {
            let (status, body) = self.post_raw(base, "/api/term", req)?;
            Ok((status, body, base.to_string()))
        })
    }

    /// GET /api/sessions. Returns the parsed reply, the raw bytes (for the cache) and the server it came from.
    pub fn sessions(&self) -> Result<(ApiSessions, Vec<u8>, String), CoreError> {
        self.with_base(|base| {
            let raw = self.get_bytes(base, "/api/sessions")?;
            let parsed: ApiSessions = serde_json::from_slice(&raw).map_err(|e| CoreError::Bad(format!("Unexpected reply: {e}")))?;
            Ok((parsed, raw, base.to_string()))
        })
    }

    pub fn summary(&self) -> Result<ApiSummary, CoreError> {
        self.with_base(|base| {
            let raw = self.get_bytes(base, "/api/summary")?;
            serde_json::from_slice(&raw).map_err(|e| CoreError::Bad(format!("Unexpected reply: {e}")))
        })
    }

    /// One level of a machine's folder tree (`parent` = Some("") for the top), or the flat list with recents (None).
    pub fn folders(&self, machine: &str, parent: Option<&str>) -> Result<ApiFolders, CoreError> {
        let path = match parent {
            Some(p) => format!("/api/folders/{}?parent={}", enc(machine), enc(p)),
            None => format!("/api/folders/{}", enc(machine)),
        };
        self.with_base(|base| {
            let raw = self.get_bytes(base, &path)?;
            serde_json::from_slice(&raw).map_err(|e| CoreError::Bad(format!("Unexpected reply: {e}")))
        })
    }

    pub fn job(&self, id: &str) -> Result<ApiJob, CoreError> {
        let path = format!("/api/job/server/{}", enc(id));
        self.with_base(|base| {
            let raw = self.get_bytes(base, &path)?;
            serde_json::from_slice(&raw).map_err(|e| CoreError::Bad(format!("Unexpected reply: {e}")))
        })
    }

    /// POST /api/job/cancel: a job still waiting for its chat's work to end stops there, and nothing changes.
    pub fn cancel_job(&self, id: &str) -> Result<(), CoreError> {
        let body = serde_json::json!({ "job": id });
        self.with_base(|base| self.post_json(base, "/api/job/cancel", &body).map(|_| ()))
    }

    /// POST /api/setup_fix: the machine's agent repairs one setup problem.
    pub fn setup_fix(&self, machine: &str, fix: &str) -> Result<(), CoreError> {
        let body = serde_json::json!({ "machine": machine, "fix": fix });
        self.with_base(|base| self.post_json(base, "/api/setup_fix", &body).map(|_| ()))
    }

    /// POST /api/action. `verb` is one of resume, fresh, handoff, stop, dismiss, pass.
    pub fn action(&self, machine: &str, verb: &str, sid: &str, close: Option<bool>, when: Option<&str>) -> Result<ApiPostReply, CoreError> {
        let body = ActionBody { machine, verb, sid, close, on: None, to: None, older: None, name: None, when };
        self.with_base(|base| parse_reply(&self.post_json(base, "/api/action", &body)?))
    }

    /// v2: Keep running on or off. Returns what the server stored.
    pub fn keep(&self, machine: &str, sid: &str, on: bool) -> Result<bool, CoreError> {
        let body = ActionBody { machine, verb: "keep", sid, close: None, on: Some(on), to: None, older: None, name: None, when: None };
        self.with_base(|base| {
            let raw = self.post_json(base, "/api/action", &body)?;
            let r: ApiKeep = serde_json::from_slice(&raw).unwrap_or_default();
            Ok(r.keep.unwrap_or(on))
        })
    }

    /// Star on or off (`on` None toggles). Returns what the server stored.
    pub fn star(&self, machine: &str, sid: &str, on: Option<bool>) -> Result<bool, CoreError> {
        let body = ActionBody { machine, verb: "star", sid, close: None, on, to: None, older: None, name: None, when: None };
        self.with_base(|base| {
            let raw = self.post_json(base, "/api/action", &body)?;
            let r: ApiKeep = serde_json::from_slice(&raw).unwrap_or_default();
            Ok(r.star.unwrap_or(on.unwrap_or(true)))
        })
    }

    /// v2: "This continues...": links (machine, sid) to the older chat (to_machine, to_sid). Returns the thread.
    pub fn link(&self, machine: &str, sid: &str, to_machine: &str, to_sid: &str) -> Result<ApiThread, CoreError> {
        let body = ActionBody { machine, verb: "link", sid, close: None, on: None, to: Some(ChatRef { machine: to_machine, sid: to_sid }), older: None, name: None, when: None };
        self.with_base(|base| Ok(serde_json::from_slice(&self.post_json(base, "/api/action", &body)?).unwrap_or_default()))
    }

    /// v2: "Not the same topic". `older`: on a replaced (older) chat, the chats that carry on from it split off.
    pub fn unlink(&self, machine: &str, sid: &str, older: bool) -> Result<ApiThread, CoreError> {
        let body = ActionBody { machine, verb: "unlink", sid, close: None, on: None, to: None, older: older.then_some(true), name: None, when: None };
        self.with_base(|base| Ok(serde_json::from_slice(&self.post_json(base, "/api/action", &body)?).unwrap_or_default()))
    }

    /// v2: GET /api/thread/<machine>/<sid>
    pub fn thread(&self, machine: &str, sid: &str) -> Result<ApiThread, CoreError> {
        let path = format!("/api/thread/{}/{}", enc(machine), enc(sid));
        self.with_base(|base| {
            let raw = self.get_bytes(base, &path)?;
            serde_json::from_slice(&raw).map_err(|e| CoreError::Bad(format!("Unexpected reply: {e}")))
        })
    }

    /// v2: GET /api/chats?q=&exclude=<machine>/<sid>
    pub fn chats(&self, q: &str, exclude: Option<(&str, &str)>) -> Result<ApiChats, CoreError> {
        let mut path = format!("/api/chats?q={}", enc(q));
        if let Some((m, sid)) = exclude {
            path.push_str(&format!("&exclude={}", enc(&format!("{m}/{sid}"))));
        }
        self.with_base(|base| {
            let raw = self.get_bytes(base, &path)?;
            serde_json::from_slice(&raw).map_err(|e| CoreError::Bad(format!("Unexpected reply: {e}")))
        })
    }

    /// v2: POST /api/route. `rules`: the instant rule-based pick only (the New session preview).
    pub fn route(&self, prompt: &str, folder: &str, rules: bool, chrome: bool, chrome_machine: Option<&str>) -> Result<ApiRoute, CoreError> {
        let body = RouteBody { prompt, folder, rules, chrome, chrome_machine };
        self.with_base(|base| {
            let raw = self.post_json(base, "/api/route", &body)?;
            serde_json::from_slice(&raw).map_err(|e| CoreError::Bad(format!("Unexpected reply: {e}")))
        })
    }

    fn get_json<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T, CoreError> {
        self.with_base(|base| {
            let raw = self.get_bytes(base, path)?;
            serde_json::from_slice(&raw).map_err(|e| CoreError::Bad(format!("Unexpected reply: {e}")))
        })
    }

    /// v2: POST /api/cleanup, as the card sends it (stop a list, or keep one).
    pub fn cleanup(&self, body: &serde_json::Value) -> Result<serde_json::Value, CoreError> {
        self.with_base(|base| Ok(serde_json::from_slice(&self.post_json(base, "/api/cleanup", body)?).unwrap_or_default()))
    }

    /// POST /api/tabs: the chats a desktop app has open in tabs.
    pub fn tabs(&self, body: &serde_json::Value) -> Result<serde_json::Value, CoreError> {
        self.with_base(|base| Ok(serde_json::from_slice(&self.post_json(base, "/api/tabs", body)?).unwrap_or_default()))
    }

    /// v2: POST /api/upload?machine=&name= with the file as the body. `sent` counts the bytes as they go out, for a
    /// progress bar.
    pub fn upload(&self, machine: &str, name: &str, data: &[u8], sent: &std::sync::atomic::AtomicU64) -> Result<crate::api::ApiUpload, CoreError> {
        use std::sync::atomic::Ordering;
        self.with_base(|base| {
            sent.store(0, Ordering::Relaxed);
            let mut reader = Counted { inner: data, sent };
            let url = format!("{base}/api/upload?machine={}&name={}", enc(machine), enc(name));
            // The long-lived agent: the server passes a big file to its machine in chunks before it answers.
            let mut r = self
                .stream_agent
                .post(&url)
                .header("X-Switchboard", "1")
                .header("Content-Type", "application/octet-stream")
                .header("Content-Length", &data.len().to_string())
                .send(ureq::SendBody::from_reader(&mut reader))
                .map_err(|_| CoreError::Unreachable)?;
            let status = r.status().as_u16();
            let out = r.body_mut().read_to_vec().map_err(|_| CoreError::Unreachable)?;
            check(status, &out)?;
            let u: crate::api::ApiUpload = serde_json::from_slice(&out).map_err(|e| CoreError::Bad(format!("Unexpected reply: {e}")))?;
            if u.path.is_empty() {
                return Err(CoreError::Bad("The machine did not say where it put the file.".into()));
            }
            Ok(u)
        })
    }

    /// v2: GET /api/app (the newest build on the download page)
    pub fn app_latest(&self) -> Result<crate::update::ApiApp, CoreError> {
        self.get_json("/api/app")
    }

    /// v2: GET /api/machines
    pub fn machines(&self) -> Result<crate::api::ApiMachines, CoreError> {
        self.get_json("/api/machines")
    }

    /// v2: POST /api/machines {op: rename|root|order|remove|add, ...}; answers with the list as it is now.
    pub fn machines_edit(&self, body: &serde_json::Value) -> Result<crate::api::ApiMachines, CoreError> {
        self.with_base(|base| {
            let raw = self.post_json(base, "/api/machines", body)?;
            serde_json::from_slice(&raw).map_err(|e| CoreError::Bad(format!("Unexpected reply: {e}")))
        })
    }

    /// v2, Settings: GET /api/machines/<name>/folders?path= (the folders under its home folder)
    pub fn machine_folders(&self, name: &str, path: &str) -> Result<crate::api::ApiRootDirs, CoreError> {
        self.get_json(&format!("/api/machines/{}/folders?path={}", enc(name), enc(path)))
    }

    /// v2, Settings: GET /api/machines/<name>/mcp
    pub fn machine_mcp(&self, name: &str) -> Result<crate::api::ApiMcp, CoreError> {
        self.get_json(&format!("/api/machines/{}/mcp", enc(name)))
    }

    /// v2, Settings: GET /api/machines/<name>/caps
    pub fn machine_caps(&self, name: &str) -> Result<crate::api::ApiCapsView, CoreError> {
        self.get_json(&format!("/api/machines/{}/caps", enc(name)))
    }

    /// v2: GET /api/flags?machine=&sid=
    pub fn flags_view(&self, machine: &str, sid: &str) -> Result<crate::api::ApiFlagsView, CoreError> {
        self.get_json(&format!("/api/flags?machine={}&sid={}", enc(machine), enc(sid)))
    }

    /// v2: POST /api/flags {machine, sid, want}
    pub fn flags_plan(&self, machine: &str, sid: &str, want: &serde_json::Value) -> Result<crate::api::ApiFlagsPlan, CoreError> {
        let body = serde_json::json!({"machine": machine, "sid": sid, "want": want});
        self.with_base(|base| {
            let raw = self.post_json(base, "/api/flags", &body)?;
            serde_json::from_slice(&raw).map_err(|e| CoreError::Bad(format!("Unexpected reply: {e}")))
        })
    }

    /// v2: POST /api/relaunch. A need "terminal" comes back as the reply (its command), a need "busy" as CoreError::Busy.
    pub fn relaunch(&self, body: &serde_json::Value) -> Result<crate::api::ApiRelaunch, CoreError> {
        self.with_base(|base| {
            let (status, raw) = self.post_raw(base, "/api/relaunch", body)?;
            let r: crate::api::ApiRelaunch = serde_json::from_slice(&raw).unwrap_or_default();
            if r.need.as_deref() == Some("terminal") {
                return Ok(r);
            }
            check(status, &raw)?;
            Ok(r)
        })
    }

    /// v2: GET /api/prefs
    pub fn prefs(&self) -> Result<ApiPrefs, CoreError> {
        self.get_json("/api/prefs")
    }

    /// v2: POST /api/prefs with only the fields that change (chrome_machine, presets, last).
    /// POST /api/chrome: one chat's own Chrome machine, or None to clear it.
    pub fn set_chrome(&self, machine: &str, sid: &str, chrome_machine: Option<&str>) -> Result<(), CoreError> {
        let body = serde_json::json!({ "machine": machine, "sid": sid, "chrome_machine": chrome_machine });
        self.with_base(|base| self.post_json(base, "/api/chrome", &body).map(|_| ()))
    }

    /// POST /api/budget with {op: set|lift|unlift, ...}: the budget after the change.
    pub fn budget_op(&self, body: &serde_json::Value) -> Result<crate::api::ApiBudget, CoreError> {
        self.with_base(|base| serde_json::from_slice(&self.post_json(base, "/api/budget", body)?).map_err(|e| CoreError::Bad(format!("Unexpected reply: {e}"))))
    }

    /// POST /api/groups with {op, ...} (groups.rs builds it).
    pub fn groups(&self, body: &serde_json::Value) -> Result<ApiGroupsReply, CoreError> {
        self.with_base(|base| serde_json::from_slice(&self.post_json(base, "/api/groups", body)?).map_err(|e| CoreError::Bad(format!("Unexpected reply: {e}"))))
    }

    pub fn set_prefs(&self, body: &serde_json::Value) -> Result<(), CoreError> {
        self.with_base(|base| self.post_json(base, "/api/prefs", body).map(|_| ()))
    }

    /// v2: GET /api/browse?<query>, the query already encoded.
    pub fn browse(&self, query: &str) -> Result<ApiBrowse, CoreError> {
        self.get_json(&format!("/api/browse?{query}"))
    }

    /// v2: GET /api/files/<machine>?folder=..[&name=..], the Claude folder one level at a time, or one file.
    pub fn files(&self, machine: &str, folder: &str, name: Option<&str>) -> Result<ApiFiles, CoreError> {
        let mut p = format!("/api/files/{}?folder={}", enc(machine), enc(folder));
        if let Some(n) = name {
            p.push_str(&format!("&name={}", enc(n)));
        }
        self.get_json(&p)
    }

    /// v2: POST /api/pair, Add a machine.
    pub fn pair(&self) -> Result<ApiPair, CoreError> {
        self.with_base(|base| {
            let raw = self.post_json(base, "/api/pair", &serde_json::json!({}))?;
            serde_json::from_slice(&raw).map_err(|e| CoreError::Bad(format!("Unexpected reply: {e}")))
        })
    }

    /// POST /api/compact: the cost of compacting this chat's History thread, or with `confirm`, the job.
    pub fn compact(&self, machine: &str, sid: &str, confirm: bool) -> Result<crate::api::ApiCompact, CoreError> {
        let body = serde_json::json!({ "machine": machine, "sid": sid, "confirm": confirm });
        self.with_base(|base| {
            let raw = self.post_json(base, "/api/compact", &body)?;
            serde_json::from_slice(&raw).map_err(|e| CoreError::Bad(format!("Unexpected reply: {e}")))
        })
    }

    /// GET /api/handoff/<machine>/<sid>: the chat's written summary.
    pub fn handoff_text(&self, machine: &str, sid: &str) -> Result<String, CoreError> {
        let h: crate::api::ApiHandoff = self.get_json(&format!("/api/handoff/{}/{}", enc(machine), enc(sid)))?;
        Ok(h.text)
    }

    /// GET /api/usage: this week by day, the 5-hour window and week, and past weeks.
    pub fn usage(&self) -> Result<crate::api::ApiUsage, CoreError> {
        self.get_json("/api/usage")
    }

    /// v2: GET /api/summary/<machine>/<sid>
    pub fn chat_summary(&self, machine: &str, sid: &str) -> Result<ApiChatSummary, CoreError> {
        self.get_json(&format!("/api/summary/{}/{}", enc(machine), enc(sid)))
    }

    /// v2: rename a chat (the machine writes it into a stopped chat's transcript too).
    pub fn rename(&self, machine: &str, sid: &str, name: &str) -> Result<(), CoreError> {
        let body = ActionBody { machine, verb: "rename", sid, close: None, on: None, to: None, older: None, name: Some(name), when: None };
        self.with_base(|base| self.post_json(base, "/api/action", &body).map(|_| ()))
    }

    /// v2: GET /api/stream, the server's live updates. `seen` hears the version on connect and on every change;
    /// this returns when the stream ends (the server closes it every ten minutes), drops, or `seen` says stop.
    pub fn stream(&self, mut seen: impl FnMut(u64) -> bool) -> Result<(), CoreError> {
        use std::io::BufRead;
        let base = self.resolver.resolve().ok_or(CoreError::Unreachable)?;
        let mut r = self
            .stream_agent
            .get(&format!("{base}/api/stream"))
            .header("X-Switchboard", "1")
            .header("Accept", "text/event-stream")
            .call()
            .map_err(|_| {
                self.resolver.invalidate();
                CoreError::Unreachable
            })?;
        let status = r.status().as_u16();
        if status != 200 {
            return Err(CoreError::Bad(format!("The live stream answered {status}.")));
        }
        for line in std::io::BufReader::new(r.body_mut().as_reader()).lines() {
            let line = line.map_err(|_| CoreError::Unreachable)?;
            if let Some(data) = line.strip_prefix("data:") {
                let v = serde_json::from_str::<serde_json::Value>(data.trim()).ok().and_then(|j| j.get("v").and_then(|v| v.as_u64()));
                if !seen(v.unwrap_or(0)) {
                    break;
                }
            }
        }
        Ok(())
    }

    /// A POST that may take minutes (the library packs a tool on one machine or pushes it to others before it
    /// answers), on the long-lived agent. Only for requests that are safe to send twice.
    fn post_long(&self, base: &str, path: &str, body: &serde_json::Value) -> Result<Vec<u8>, CoreError> {
        let mut r = self
            .stream_agent
            .post(&format!("{base}{path}"))
            .header("X-Switchboard", "1")
            .header("Content-Type", "application/json")
            .send_json(body)
            .map_err(|_| CoreError::Unreachable)?;
        let status = r.status().as_u16();
        let out = r.body_mut().read_to_vec().map_err(|_| CoreError::Unreachable)?;
        check(status, &out)?;
        Ok(out)
    }

    /// Library: GET /api/library
    pub fn library(&self) -> Result<crate::api::ApiLibrary, CoreError> {
        self.get_json("/api/library")
    }

    /// Library: GET /api/library/sources?machine= (the skills and MCP servers that machine can give)
    pub fn library_sources(&self, machine: &str) -> Result<crate::api::ApiLibrarySources, CoreError> {
        self.get_json(&format!("/api/library/sources?machine={}", enc(machine)))
    }

    /// Library: POST /api/library/import {machine, kind, source, tool?, description?}
    /// Sent once, never retried: each import stores a new version.
    pub fn library_import(&self, body: &serde_json::Value) -> Result<crate::api::ApiToolStored, CoreError> {
        let base = self.resolver.resolve().ok_or(CoreError::Unreachable)?;
        let raw = self.post_long(&base, "/api/library/import", body)?;
        serde_json::from_slice(&raw).map_err(|e| CoreError::Bad(format!("Unexpected reply: {e}")))
    }

    /// Library: POST /api/library/<verb> for connect, defaults, rollback, delete and attach (answers only ok). Each
    /// is idempotent, and connect and rollback push bundles to machines first, so they get the long timeout.
    pub fn library_post(&self, verb: &str, body: &serde_json::Value) -> Result<(), CoreError> {
        let path = format!("/api/library/{verb}");
        self.with_base(|base| self.post_long(base, &path, body).map(|_| ()))
    }

    /// Library: POST /api/library/new {machine, tool, brief}, a start like /api/new.
    pub fn library_new(&self, body: &serde_json::Value) -> Result<ApiPostReply, CoreError> {
        self.with_base(|base| parse_reply(&self.post_json(base, "/api/library/new", body)?))
    }

    pub fn new_session(&self, req: &NewRequest) -> Result<ApiPostReply, CoreError> {
        self.with_base(|base| parse_reply(&self.post_json(base, "/api/new", req)?))
    }

    pub fn continue_session(&self, req: &ContinueRequest) -> Result<ApiPostReply, CoreError> {
        self.with_base(|base| parse_reply(&self.post_json(base, "/api/continue", req)?))
    }
}

fn parse_reply(raw: &[u8]) -> Result<ApiPostReply, CoreError> {
    Ok(serde_json::from_slice(raw).unwrap_or_default())
}

/// Turns a non-2xx status or `{ok: false}` into the server's own message.
fn check(status: u16, body: &[u8]) -> Result<(), CoreError> {
    if let Some(e) = busy_of(body) {
        return Err(e);
    }
    let reply: Option<ApiPostReply> = serde_json::from_slice(body).ok();
    let failed = !(200..300).contains(&status) || reply.as_ref().and_then(|r| r.ok) == Some(false);
    if failed {
        let message = reply.and_then(|r| r.error).unwrap_or_else(|| format!("Request failed ({status})"));
        return Err(CoreError::Server { status, message });
    }
    Ok(())
}

/// A need "busy" answer: what is running, for one chat (`busy` is an object) or several (a list).
fn busy_of(body: &[u8]) -> Option<CoreError> {
    let v: serde_json::Value = serde_json::from_slice(body).ok()?;
    if v.get("need").and_then(|n| n.as_str()) != Some("busy") {
        return None;
    }
    let busy = match v.get("busy") {
        Some(serde_json::Value::Array(a)) => a.iter().filter_map(|b| serde_json::from_value(b.clone()).ok()).collect(),
        Some(b @ serde_json::Value::Object(_)) => serde_json::from_value(b.clone()).ok().into_iter().collect(),
        _ => Vec::new(),
    };
    let message = v.get("error").and_then(|e| e.as_str()).unwrap_or("This chat still has work running.").to_string();
    Some(CoreError::Busy { message, busy })
}

/// A body reader that counts what has been read (sent).
struct Counted<'a> {
    inner: &'a [u8],
    sent: &'a std::sync::atomic::AtomicU64,
}

impl std::io::Read for Counted<'_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.sent.fetch_add(n as u64, std::sync::atomic::Ordering::Relaxed);
        Ok(n)
    }
}

/// Percent-encodes a path segment or query value.
pub fn enc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn probe(answers: Vec<(&'static str, u64, bool)>) -> Probe {
        Arc::new(move |base: &str| {
            let (_, ms, ok) = answers.iter().find(|(b, _, _)| *b == base).copied().unwrap();
            thread::sleep(Duration::from_millis(ms));
            ok
        })
    }

    fn c(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_busy_answer_says_what_is_running() {
        let one = br#"{"ok":false,"need":"busy","error":"1 subagent is still running. Stop it when they finish, or now.","busy":{"live":true,"subagents":["Audit"],"commands":[],"replying":false,"pending":[],"summary":"1 subagent is still running."}}"#;
        match check(200, one) {
            Err(CoreError::Busy { message, busy }) => {
                assert!(message.starts_with("1 subagent"));
                assert_eq!(busy.len(), 1);
                assert_eq!(busy[0].subagents, vec!["Audit".to_string()]);
                assert!(!busy[0].only_reply());
            }
            other => panic!("{other:?}"),
        }
        let many = br#"{"ok":false,"need":"busy","error":"2 of these chats still have work running.","busy":[{"summary":"Claude is still replying.","replying":true,"title":"A"},{"summary":"x","title":"B"}]}"#;
        match check(200, many) {
            Err(CoreError::Busy { busy, .. }) => {
                assert_eq!(busy.len(), 2);
                assert!(busy[0].only_reply());
                assert_eq!(busy[1].title.as_deref(), Some("B"));
            }
            other => panic!("{other:?}"),
        }
        assert!(matches!(check(200, br#"{"ok":false,"error":"no"}"#), Err(CoreError::Server { .. })));
        assert!(check(200, br#"{"ok":true,"job":"s-1"}"#).is_ok());
    }

    #[test]
    fn lan_wins_when_it_answers() {
        let p = probe(vec![("lan", 50, true), ("ts", 5, true)]);
        assert_eq!(first_preferred(&c(&["lan", "ts"]), Duration::from_millis(400), p).as_deref(), Some("lan"));
    }

    #[test]
    fn tailscale_wins_when_lan_hangs() {
        let p = probe(vec![("lan", 3000, true), ("ts", 20, true)]);
        let t = Instant::now();
        assert_eq!(first_preferred(&c(&["lan", "ts"]), Duration::from_millis(200), p).as_deref(), Some("ts"));
        assert!(t.elapsed() < Duration::from_millis(1500), "waited for the hanging LAN probe: {:?}", t.elapsed());
    }

    #[test]
    fn tailscale_wins_when_lan_refuses() {
        let p = probe(vec![("lan", 10, false), ("ts", 60, true)]);
        assert_eq!(first_preferred(&c(&["lan", "ts"]), Duration::from_millis(400), p).as_deref(), Some("ts"));
    }

    #[test]
    fn nothing_answers() {
        let p = probe(vec![("lan", 10, false), ("ts", 30, false)]);
        assert_eq!(first_preferred(&c(&["lan", "ts"]), Duration::from_millis(100), p), None);
    }

    #[test]
    fn resolver_caches_and_invalidates() {
        let calls = Arc::new(AtomicUsize::new(0));
        let k = calls.clone();
        let p: Probe = Arc::new(move |b: &str| {
            k.fetch_add(1, Ordering::SeqCst);
            b == "ts"
        });
        let r = Resolver::new(c(&["lan", "ts"]), Duration::from_secs(60), p);
        assert_eq!(r.resolve().as_deref(), Some("ts"));
        assert_eq!(r.resolve().as_deref(), Some("ts"));
        assert_eq!(calls.load(Ordering::SeqCst), 2, "second resolve should come from the cache");
        r.invalidate();
        assert_eq!(r.resolve().as_deref(), Some("ts"));
        assert_eq!(calls.load(Ordering::SeqCst), 4);
    }

    #[test]
    fn resolver_looks_on_the_network_only_without_saved_addresses() {
        let looked = Arc::new(AtomicUsize::new(0));
        let l = looked.clone();
        let d: Discover = Arc::new(move || {
            l.fetch_add(1, Ordering::SeqCst);
            vec!["found".to_string()]
        });
        let p: Probe = Arc::new(|b: &str| b == "found" || b == "typed");
        let r = Resolver::new(vec![], Duration::from_secs(60), p).with_discovery(d);
        assert_eq!(r.resolve().as_deref(), Some("found"));
        assert_eq!(looked.load(Ordering::SeqCst), 1);
        r.set_candidates(c(&["typed"]));
        assert_eq!(r.resolve().as_deref(), Some("typed"));
        assert_eq!(looked.load(Ordering::SeqCst), 1, "a saved address is used as is");
        let none = Resolver::new(vec![], Duration::from_secs(60), Arc::new(|_: &str| true));
        assert_eq!(none.resolve(), None, "no address and no search: nothing is tried");
    }

    #[test]
    fn typed_addresses_become_base_urls() {
        assert_eq!(normalize_base("192.0.2.10").as_deref(), Some("http://192.0.2.10:8130"));
        assert_eq!(normalize_base(" 192.0.2.10:8130/ ").as_deref(), Some("http://192.0.2.10:8130"));
        assert_eq!(normalize_base("studio.local:9000").as_deref(), Some("http://studio.local:9000"));
        assert_eq!(normalize_base("HTTP://studio.local:8130").as_deref(), Some("http://studio.local:8130"));
        assert_eq!(normalize_base("https://sb.example.com").as_deref(), Some("https://sb.example.com"));
        for bad in ["", "   ", "ftp://x", "a b", "host:port", "host:0", "http://", "x.com/path", "you@x"] {
            assert_eq!(normalize_base(bad), None, "{bad}");
        }
        assert!(is_lan("http://10.0.0.5:8130") && is_lan("http://studio.local:8130"));
        assert!(!is_lan("http://100.64.0.10:8130"));
        assert_eq!(probe_timeout("http://10.0.0.5:8130"), Duration::from_millis(2500));
    }

    #[test]
    fn errors_read_like_the_web_app() {
        assert_eq!(check(400, br#"{"ok":false,"error":"bad session id"}"#), Err(CoreError::Server { status: 400, message: "bad session id".into() }));
        assert_eq!(check(500, b"oops"), Err(CoreError::Server { status: 500, message: "Request failed (500)".into() }));
        assert_eq!(check(200, br#"{"ok":false,"error":"machine is offline"}"#).unwrap_err().to_string(), "machine is offline");
        assert!(check(200, br#"{"ok":true}"#).is_ok());
        assert_eq!(enc("a b/c"), "a%20b%2Fc");
    }

    #[test]
    fn a_start_sends_tools_only_when_there_are_some() {
        let mut r = NewRequest { machine: "studio".into(), folder: "x".into(), prompt: "hi".into(), mode: "normal".into(), ..Default::default() };
        let plain = serde_json::to_value(&r).unwrap();
        assert!(plain.get("tools").is_none());
        r.tools = Tools::Pick(vec!["web-fetch".into()]);
        assert_eq!(serde_json::to_value(&r).unwrap()["tools"], serde_json::json!(["web-fetch"]));
        r.tools = Tools::Auto;
        assert_eq!(serde_json::to_value(&r).unwrap()["tools"], serde_json::json!("auto"));
        // Pick with nothing picked still says so to a server that knows Auto.
        r.tools = Tools::Pick(vec![]);
        assert_eq!(serde_json::to_value(&r).unwrap()["tools"], serde_json::json!([]));
    }
}
