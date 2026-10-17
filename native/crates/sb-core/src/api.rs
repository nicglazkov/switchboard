//! The wire format of the Switchboard server, exactly as it sends it. Every field is optional or defaulted so
//! an older or newer server never sinks a whole refresh; `model.rs` turns this into the shapes the UI uses.

use serde::{Deserialize, Serialize};

/// GET /api/sessions
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ApiSessions {
    #[serde(default)]
    pub machines: Vec<ApiMachine>,
    #[serde(default)]
    pub now: Option<f64>,
    #[serde(default)]
    pub summary: ApiSummary,
    #[serde(default)]
    pub web: Option<String>,
    /// v2: the live-update version; it moves whenever something on the board changed.
    #[serde(default, deserialize_with = "lenient_opt")]
    pub v: Option<u64>,
    /// v2: the cleanup card: chats not in use and what they hold, and what was stopped from it today.
    #[serde(default, deserialize_with = "lenient_opt")]
    pub cleanup: Option<ApiCleanup>,
    /// Tab groups, shared by every Mac (missing on older servers).
    #[serde(default, deserialize_with = "lenient_vec")]
    pub groups: Vec<ApiGroup>,
    /// The usage budget: its settings and where today stands (missing on older servers).
    #[serde(default, deserialize_with = "lenient_opt")]
    pub budget: Option<ApiBudget>,
}

/// The usage budget (docs/specs/2026-10-13-usage-budget.md), from the snapshot or GET/POST /api/budget. Percents are
/// of the weekly limit, except the 5-hour ones (of the 5-hour window).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ApiBudget {
    #[serde(default, deserialize_with = "lenient_opt")]
    pub settings: Option<ApiBudgetSettings>,
    #[serde(default, deserialize_with = "lenient_opt")]
    pub status: Option<ApiBudgetStatus>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ApiBudgetSettings {
    /// Soft daily budget, percent of the week per day; None is off.
    #[serde(default, deserialize_with = "lenient_opt")]
    pub daily: Option<f64>,
    #[serde(default, deserialize_with = "lenient_opt")]
    pub five_hour: Option<f64>,
    #[serde(default, deserialize_with = "lenient_opt")]
    pub hard: Option<ApiBudgetHard>,
    #[serde(default, deserialize_with = "lenient_opt")]
    pub lifted_until: Option<f64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ApiBudgetHard {
    #[serde(default, deserialize_with = "lenient_opt")]
    pub on: Option<bool>,
    #[serde(default, deserialize_with = "lenient_opt")]
    pub daily: Option<f64>,
    #[serde(default, deserialize_with = "lenient_opt")]
    pub five_hour: Option<f64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ApiBudgetStatus {
    #[serde(default, deserialize_with = "lenient_opt")]
    pub known: Option<bool>,
    /// Percent of the week used today.
    #[serde(default, deserialize_with = "lenient_opt")]
    pub today: Option<f64>,
    #[serde(default, deserialize_with = "lenient_opt")]
    pub over_daily: Option<bool>,
    #[serde(default, deserialize_with = "lenient_opt")]
    pub over_budget: Option<bool>,
    #[serde(default, deserialize_with = "lenient_opt")]
    pub over_hard: Option<bool>,
    #[serde(default, deserialize_with = "lenient_opt")]
    pub hard_reason: Option<String>,
    #[serde(default, deserialize_with = "lenient_opt")]
    pub lifted: Option<bool>,
    #[serde(default, deserialize_with = "lenient_opt")]
    pub blocking: Option<bool>,
    /// One plain sentence or two on where usage stands.
    #[serde(default, deserialize_with = "lenient_opt")]
    pub summary: Option<String>,
}

impl ApiBudget {
    pub fn settings(&self) -> ApiBudgetSettings {
        self.settings.clone().unwrap_or_default()
    }
    pub fn status(&self) -> ApiBudgetStatus {
        self.status.clone().unwrap_or_default()
    }
}

impl ApiBudgetSettings {
    pub fn hard(&self) -> ApiBudgetHard {
        self.hard.clone().unwrap_or_default()
    }
}

impl ApiBudgetStatus {
    pub fn over_daily(&self) -> bool {
        self.over_daily == Some(true)
    }
    pub fn over_budget(&self) -> bool {
        self.over_budget == Some(true)
    }
    pub fn over_hard(&self) -> bool {
        self.over_hard == Some(true)
    }
    pub fn lifted(&self) -> bool {
        self.lifted == Some(true)
    }
    pub fn blocking(&self) -> bool {
        self.blocking == Some(true)
    }
}

/// One tab group: its members are thread sids, in order.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ApiGroup {
    pub id: String,
    #[serde(default, deserialize_with = "lenient_string")]
    pub name: String,
    #[serde(default, deserialize_with = "lenient_string")]
    pub color: String,
    #[serde(default, deserialize_with = "lenient_strings")]
    pub members: Vec<String>,
}

/// POST /api/groups: every group after the change, and the one a create made.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ApiGroupsReply {
    #[serde(default, deserialize_with = "lenient_vec")]
    pub groups: Vec<ApiGroup>,
    #[serde(default, deserialize_with = "lenient_opt")]
    pub group: Option<ApiGroup>,
}

fn lenient_string<'de, D>(d: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let v = serde_json::Value::deserialize(d)?;
    Ok(v.as_str().unwrap_or_default().to_string())
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ApiMachine {
    pub name: String,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub os: Option<String>,
    #[serde(default)]
    pub online: Option<bool>,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub updated: Option<f64>,
    /// When the server last heard from it (server clock); its sessions' states are as of then. Older servers send
    /// only `updated`.
    #[serde(default)]
    pub as_of: Option<f64>,
    #[serde(default)]
    pub root: Option<String>,
    #[serde(default)]
    pub sessions: Vec<ApiSession>,
    #[serde(default, deserialize_with = "lenient_vec")]
    pub replaced: Vec<ApiReplaced>,
    /// v2: Claude's memory on the machine.
    #[serde(default, deserialize_with = "lenient_opt")]
    pub mem: Option<ApiMem>,
    /// v2: what the machine can do (null when the user has not turned it on for this machine).
    #[serde(default)]
    pub caps: Option<serde_json::Value>,
    /// v2: the one-line summary of `caps` ("iOS, Android SDK, CUDA (RTX GPU)").
    #[serde(default)]
    pub can: Option<String>,
    /// v2: what the machine's agent can do ("flags", "browse", "rename", "summary", ...); empty for an older agent.
    #[serde(default, deserialize_with = "lenient_vec")]
    pub features: Vec<String>,
    /// v2: Claude in Chrome there (never read from a browser profile, only whether the extension is installed).
    #[serde(default, deserialize_with = "lenient_opt")]
    pub chrome: Option<ApiChrome>,
    /// v2: the machine Chrome sessions go to first.
    #[serde(default)]
    pub chrome_pref: Option<bool>,
    /// Problems with its Claude Code setup that would bite a chat later (agent setup_issues).
    #[serde(default, deserialize_with = "lenient_vec")]
    pub setup: Vec<ApiSetupIssue>,
}

/// One setup problem on a machine. `fix` names the repair its agent can make (POST /api/setup_fix), if any.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ApiSetupIssue {
    pub id: String,
    #[serde(default)]
    pub tone: Option<String>,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub fix: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ApiChrome {
    #[serde(default)]
    pub installed: bool,
    #[serde(default)]
    pub connector: bool,
    #[serde(default)]
    pub connected: bool,
}

/// v2: the cleanup card. Nothing is stopped without a click.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ApiCleanup {
    #[serde(default, deserialize_with = "lenient_vec")]
    pub items: Vec<ApiCleanItem>,
    #[serde(default, deserialize_with = "lenient_vec")]
    pub stopped: Vec<ApiCleanStopped>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ApiCleanItem {
    pub machine: String,
    #[serde(default)]
    pub label: Option<String>,
    /// A chat, or a leftover Claude process (`pid`).
    #[serde(default)]
    pub sid: Option<String>,
    #[serde(default)]
    pub pid: Option<u64>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub why: Option<String>,
    #[serde(default)]
    pub mb: Option<f64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ApiCleanStopped {
    pub machine: String,
    #[serde(default)]
    pub label: Option<String>,
    pub sid: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub at: Option<f64>,
    #[serde(default)]
    pub mb: Option<f64>,
}

/// v2: GET /api/prefs, the New sheet's memory and the flags the server passes on.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ApiPrefs {
    #[serde(default)]
    pub chrome_machine: Option<String>,
    #[serde(default, deserialize_with = "lenient_opt")]
    pub last: Option<ApiSetup>,
    #[serde(default, deserialize_with = "lenient_vec")]
    pub presets: Vec<ApiSetup>,
    /// Every flag Switchboard passes, and whether it takes a value.
    #[serde(default)]
    pub takes: std::collections::HashMap<String, bool>,
    #[serde(default)]
    pub aliases: std::collections::HashMap<String, String>,
    /// v2: the look every device shares: the app's theme and the terminal's scheme, size, cursor and blink.
    #[serde(default, deserialize_with = "lenient_opt")]
    pub appearance: Option<crate::term::scheme::Appearance>,
    /// Who writes Start fresh and Compact summaries: "opus" (the default) or "sonnet".
    #[serde(default)]
    pub summary_model: Option<String>,
    /// The opt-in switches (server/options.py): features that send chat text to Claude, and changes to each
    /// machine's own Claude setup. All off for a new server. Shown as the server words them.
    #[serde(default, deserialize_with = "lenient_vec")]
    pub options: Vec<ApiOption>,
    /// Whether iPhone notifications are set up on the server, and what that means.
    #[serde(default, deserialize_with = "lenient_opt")]
    pub push: Option<ApiPush>,
}

impl ApiPrefs {
    /// One opt-in switch by key (`ai_auto_machine`, ...), when the server knows it.
    pub fn option(&self, key: &str) -> Option<&ApiOption> {
        self.options.iter().find(|o| o.key == key)
    }

    /// The options of one kind ("ai" or "setup"), in the server's order.
    pub fn options_of<'a>(&'a self, kind: &'a str) -> impl Iterator<Item = &'a ApiOption> + 'a {
        self.options.iter().filter(move |o| o.kind == kind)
    }

    /// Flips one option here, as soon as it is clicked; the POST that saves it follows.
    pub fn set_option(&mut self, key: &str, on: bool) {
        if let Some(o) = self.options.iter_mut().find(|o| o.key == key) {
            o.on = on;
        }
    }
}

/// The POST /api/prefs body that turns one opt-in switch on or off: `{"<key>": true|false}`.
pub fn option_body(key: &str, on: bool) -> serde_json::Value {
    let mut m = serde_json::Map::new();
    m.insert(key.to_string(), serde_json::Value::Bool(on));
    serde_json::Value::Object(m)
}

/// One opt-in switch: its key, its kind ("ai" sends chat text to Claude, "setup" changes each machine's Claude
/// setup), its label, whether it is on, what it does when on, and what happens when it is off.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ApiOption {
    pub key: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub on: bool,
    #[serde(default)]
    pub explain: String,
    #[serde(default)]
    pub off: String,
}

/// iPhone notifications: `ready` once the server has its APNs key, `explain` in words, and `setup`, the file to add
/// while it is not ready.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ApiPush {
    #[serde(default)]
    pub ready: bool,
    #[serde(default)]
    pub explain: String,
    #[serde(default)]
    pub setup: Option<String>,
}

/// A New sheet setup: the last one used, or a preset.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ApiSetup {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub effort: Option<String>,
    #[serde(default)]
    pub mode: Option<String>,
    #[serde(default, deserialize_with = "lenient_vec")]
    pub flags: Vec<Vec<String>>,
    #[serde(default)]
    pub other: Option<String>,
}

/// v2: GET /api/browse, every chat on every machine.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ApiBrowse {
    #[serde(default)]
    pub total: Option<u64>,
    #[serde(default, deserialize_with = "lenient_vec")]
    pub rows: Vec<ApiBrowseRow>,
    #[serde(default, deserialize_with = "lenient_opt")]
    pub facets: Option<ApiFacets>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ApiBrowseRow {
    pub machine: String,
    #[serde(default)]
    pub label: Option<String>,
    pub sid: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub folder: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub ctx: Option<f64>,
    #[serde(default)]
    pub size: Option<f64>,
    #[serde(default)]
    pub last: Option<f64>,
    #[serde(default)]
    pub started: Option<f64>,
    /// live | quiet | stopped | archived | replaced
    #[serde(default)]
    pub state: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub first_prompt: Option<String>,
    #[serde(default)]
    pub last_prompt: Option<String>,
    /// One row per thread: its earlier chats, newest first, each with what Additional history shows.
    #[serde(default, deserialize_with = "lenient_vec")]
    pub history: Vec<ApiBrowseRow>,
    #[serde(default)]
    pub history_count: Option<u64>,
    /// A summary (handoff) is written for this chat.
    #[serde(default)]
    pub handoff: Option<bool>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ApiFacets {
    #[serde(default, deserialize_with = "lenient_vec")]
    pub machines: Vec<ApiFacetMachine>,
    /// Folders (under each machine's Claude folder) that have chats, for History's folder filter.
    #[serde(default)]
    pub folders: Vec<String>,
    #[serde(default)]
    pub states: std::collections::HashMap<String, u64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ApiFacetMachine {
    pub name: String,
    #[serde(default)]
    pub label: Option<String>,
}

/// v2: GET /api/files/<machine>: one folder under the machine's Claude folder, or one file (`file`).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ApiFiles {
    #[serde(default)]
    pub root: String,
    #[serde(default)]
    pub folder: String,
    #[serde(default)]
    pub dirs: Vec<ApiFileDir>,
    #[serde(default)]
    pub files: Vec<ApiFileEntry>,
    #[serde(default)]
    pub more: bool,
    #[serde(default)]
    pub file: Option<ApiFile>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ApiFileDir {
    pub folder: String,
    pub name: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ApiFileEntry {
    pub name: String,
    #[serde(default)]
    pub size: f64,
    #[serde(default)]
    pub mtime: f64,
    /// False for a name that looks like a key or a secret: listed, never read.
    #[serde(default)]
    pub open: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ApiFile {
    pub name: String,
    #[serde(default)]
    pub size: f64,
    #[serde(default)]
    pub mtime: f64,
    #[serde(default)]
    pub secret: bool,
    #[serde(default)]
    pub binary: bool,
    #[serde(default)]
    pub cut: bool,
    #[serde(default)]
    pub text: Option<String>,
}

/// v2: POST /api/pair (Add a machine): a code from the hub and the line to paste on the new machine, per OS.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ApiPair {
    pub code: String,
    #[serde(default)]
    pub expires: f64,
    #[serde(default)]
    pub hub: String,
    #[serde(default)]
    pub install: std::collections::HashMap<String, String>,
}

/// v2: GET /api/summary/<machine>/<sid>, read from the chat's transcript on its machine.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ApiChatSummary {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default, deserialize_with = "lenient_opt")]
    pub first: Option<ApiSaid>,
    #[serde(default, deserialize_with = "lenient_vec")]
    pub recent: Vec<ApiSaid>,
    #[serde(default)]
    pub answer: Option<String>,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub ctx: Option<f64>,
    #[serde(default)]
    pub size: Option<f64>,
    #[serde(default)]
    pub last: Option<f64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ApiSaid {
    #[serde(default)]
    pub ts: Option<f64>,
    #[serde(default)]
    pub text: Option<String>,
}

/// v2 (spec section 2): `{"claude_mb": 3120, "procs": 7, "orphans": [{"pid", "mb", "age_h"}]}`
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ApiMem {
    #[serde(default)]
    pub claude_mb: Option<f64>,
    #[serde(default)]
    pub procs: Option<f64>,
    #[serde(default, deserialize_with = "lenient_vec")]
    pub orphans: Vec<ApiOrphan>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ApiOrphan {
    #[serde(default)]
    pub pid: Option<f64>,
    #[serde(default)]
    pub mb: Option<f64>,
    #[serde(default)]
    pub age_h: Option<f64>,
}

/// v2: the reaper stopped this chat. `{"at", "reason": "idle" | "stalled" | "replaced", "idle_h"}`
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ApiReaped {
    #[serde(default)]
    pub at: Option<f64>,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub idle_h: Option<f64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ApiSession {
    pub sid: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub cwd: Option<String>,
    /// Relative to the machine's Claude folder; "" means the Claude folder itself; missing means use cwd.
    #[serde(default)]
    pub folder: Option<String>,
    #[serde(default)]
    pub mtime: Option<f64>,
    #[serde(default)]
    pub active: Option<f64>,
    #[serde(default)]
    pub ctx: Option<f64>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub cache_left: Option<f64>,
    /// An older server's word that the cache is cold (newer ones send `cache`).
    #[serde(default)]
    pub cold: Option<bool>,
    /// The server's live state (server/fleet.py derive): held | needs | parked | working | stalled | quiet. Missing on
    /// older servers; the app then reads `status` (model::session).
    #[serde(default, deserialize_with = "lenient_opt")]
    pub activity: Option<String>,
    /// warm | cold | unknown
    #[serde(default, deserialize_with = "lenient_opt")]
    pub cache: Option<String>,
    /// When the cache goes cold, on the server's clock (epoch seconds), when known.
    #[serde(default, deserialize_with = "lenient_opt")]
    pub cold_at: Option<f64>,
    /// ctx at or over the guard's 150k: an "expensive" badge.
    #[serde(default, deserialize_with = "lenient_opt")]
    pub heavy: Option<bool>,
    /// "interactive" | "background" | null (closed)
    #[serde(default)]
    pub kind: Option<String>,
    /// "busy" | "idle" | "waiting" | "stopped" | "done" | null
    #[serde(default)]
    pub status: Option<String>,
    /// "working" | "blocked" | "running" | "done" | null
    #[serde(default)]
    pub state: Option<String>,
    #[serde(default)]
    pub bg_id: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub handoff: Option<bool>,
    #[serde(default)]
    pub stale: Option<bool>,
    #[serde(default)]
    pub stalled: Option<bool>,
    #[serde(default)]
    pub held: Option<bool>,
    /// This chat's own Chrome machine (the server's chrome.json), when one was picked or carried over.
    #[serde(default)]
    pub chrome_machine: Option<String>,
    #[serde(default)]
    pub repeats: Option<serde_json::Value>,
    #[serde(default)]
    pub flags: Option<ApiFlags>,
    /// v2: marked Keep running, so the reaper leaves it alone.
    #[serde(default)]
    pub keep: Option<bool>,
    /// Starred (a favorite): its tab sits at the left and comes back on launch. Follows the chat's thread.
    #[serde(default)]
    pub star: Option<bool>,
    /// Launched with Claude in Chrome, but Claude Code started it without the connection.
    #[serde(default)]
    pub chrome_missing: Option<bool>,
    /// The tab group its thread is in (an id from `groups`).
    #[serde(default, deserialize_with = "lenient_opt")]
    pub group: Option<String>,
    #[serde(default, deserialize_with = "lenient_opt")]
    pub reaped: Option<ApiReaped>,
    /// v2: the root chat of its thread, and how many older chats the thread has.
    #[serde(default)]
    pub thread: Option<String>,
    #[serde(default)]
    pub history: Option<f64>,
    /// A summary written after the chat's last reply, so Start fresh does not need a new one.
    #[serde(default)]
    pub handoff_fresh: Option<bool>,
    /// Resume vs Start fresh, worked out by the agent (agent/sb.py cost_model). The apps only display it.
    #[serde(default, deserialize_with = "lenient_opt")]
    pub cost: Option<ApiCost>,
    /// Library tools this chat was started or attached with. Missing on older servers, so it defaults to empty.
    #[serde(default, deserialize_with = "lenient_strings")]
    pub tools: Vec<String>,
    /// "auto" (the server picked its tools) or "pick" (you did). Missing on older servers.
    #[serde(default, deserialize_with = "lenient_opt")]
    pub tools_mode: Option<String>,
    /// What a blocked or waiting chat wants from you (the background job's `needs`, up to 300 characters). Missing on
    /// older servers.
    #[serde(default, deserialize_with = "lenient_opt")]
    pub needs: Option<String>,
    /// The answer the job suggests (up to 200 characters): the details sheet's "Reply: ..." button types it.
    #[serde(default, deserialize_with = "lenient_opt")]
    pub suggested_reply: Option<String>,
    /// What a waiting chat needs, shaped by kind (see `ask`). Present only while it waits; missing on older servers.
    #[serde(default, deserialize_with = "lenient_opt")]
    pub ask: Option<ApiAsk>,
}

/// The `ask` object on a session row, as the agent builds it. Every field is optional and an odd one is dropped, so
/// `crate::ask::Ask::from_api` decides what is usable.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ApiAsk {
    #[serde(default, deserialize_with = "lenient_opt")]
    pub kind: Option<String>,
    #[serde(default, deserialize_with = "lenient_opt")]
    pub summary: Option<String>,
    #[serde(default, deserialize_with = "lenient_vec")]
    pub options: Vec<ApiAskOption>,
    #[serde(default, deserialize_with = "lenient_strings")]
    pub assumptions: Vec<String>,
    #[serde(default, deserialize_with = "lenient_vec")]
    pub questions: Vec<ApiAskQuestion>,
    #[serde(default, deserialize_with = "lenient_strings")]
    pub steps: Vec<String>,
    #[serde(default, deserialize_with = "lenient_opt")]
    pub done_reply: Option<String>,
    #[serde(default, deserialize_with = "lenient_opt")]
    pub fix: Option<ApiAskFix>,
    #[serde(default, deserialize_with = "lenient_opt")]
    pub source: Option<String>,
    /// The transcript mtime it was built from (a number or a string).
    #[serde(default, rename = "for")]
    pub at: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ApiAskOption {
    #[serde(default, deserialize_with = "lenient_opt")]
    pub label: Option<String>,
    #[serde(default, deserialize_with = "lenient_opt")]
    pub reply: Option<String>,
    #[serde(default, deserialize_with = "lenient_opt")]
    pub recommended: Option<bool>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ApiAskQuestion {
    #[serde(default, deserialize_with = "lenient_opt")]
    pub q: Option<String>,
    #[serde(default, deserialize_with = "lenient_strings")]
    pub options: Vec<String>,
    #[serde(default, deserialize_with = "lenient_opt")]
    pub multi: Option<bool>,
    #[serde(default, deserialize_with = "lenient_opt")]
    pub about_you: Option<bool>,
    /// An option's index or its label (null: none).
    #[serde(default)]
    pub recommended: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ApiAskFix {
    #[serde(default, deserialize_with = "lenient_opt")]
    pub action: Option<String>,
    #[serde(default, deserialize_with = "lenient_opt")]
    pub label: Option<String>,
    #[serde(default, deserialize_with = "lenient_opt")]
    pub alt_reply: Option<String>,
}

/// GET /api/usage: the plan's numbers over time. This week day by day (weekly-percent points used each local day
/// since the reset), the 5-hour window and the week now, and the final percent of past weeks, newest first.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ApiUsage {
    #[serde(default)]
    pub ok: bool,
    #[serde(default, deserialize_with = "lenient_f64")]
    pub now: Option<f64>,
    #[serde(default, deserialize_with = "lenient_opt")]
    pub tz: Option<String>,
    #[serde(default, deserialize_with = "lenient_opt")]
    pub five: Option<ApiUsageWindow>,
    #[serde(default, deserialize_with = "lenient_opt")]
    pub week: Option<ApiUsageWindow>,
    #[serde(default, deserialize_with = "lenient_vec")]
    pub days: Vec<ApiUsageDay>,
    #[serde(default, deserialize_with = "lenient_vec")]
    pub weeks: Vec<ApiUsageWeek>,
}

/// The 5-hour window or the week: percent used, when it resets, and (the week) the projected percent at reset.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ApiUsageWindow {
    #[serde(default, deserialize_with = "lenient_f64")]
    pub pct: Option<f64>,
    #[serde(default, deserialize_with = "lenient_f64")]
    pub reset: Option<f64>,
    #[serde(default, deserialize_with = "lenient_f64")]
    pub pace: Option<f64>,
}

/// One local day of this week: "2026-10-13" and the weekly-percent points used that day (None when unknown).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ApiUsageDay {
    #[serde(default)]
    pub day: String,
    #[serde(default, deserialize_with = "lenient_f64")]
    pub used: Option<f64>,
}

/// A past week: when it reset and the final percent it reached.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ApiUsageWeek {
    #[serde(default, deserialize_with = "lenient_f64")]
    pub reset: Option<f64>,
    #[serde(default, deserialize_with = "lenient_f64")]
    pub pct: Option<f64>,
}

/// A number, or None for null, a string or anything else.
fn lenient_f64<'de, D>(d: D) -> Result<Option<f64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let v = serde_json::Value::deserialize(d)?;
    Ok(v.as_f64().filter(|x| x.is_finite()))
}

/// What each choice costs, as token weights (tokens times API price ratios), for a warm and a cold cache.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ApiCost {
    #[serde(default)]
    pub ctx: f64,
    /// small | medium | big
    #[serde(default)]
    pub band: String,
    #[serde(default)]
    pub condensed: f64,
    #[serde(default)]
    pub condensed_guessed: bool,
    #[serde(default)]
    pub note: f64,
    #[serde(default)]
    pub summary_ready: bool,
    #[serde(default)]
    pub summary_model: Option<String>,
    #[serde(default)]
    pub resume: ApiCostResume,
    #[serde(default)]
    pub fresh: ApiCostFresh,
    /// "resume" | "fresh", for each cache state.
    #[serde(default)]
    pub pick: WarmCold,
    /// "Start fresh: about 91k (summary 79k + new chat 12k). Resume: about 290k, because the cache is cold..."
    #[serde(default)]
    pub text: WarmCold,
    /// "145k, resuming re-reads 145k; Start fresh is cheaper if you do not need the history"
    #[serde(default)]
    pub band_text: WarmCold,
    /// The "How is this estimated?" text: inputs, ratios, and that plan usage weighting is an estimate.
    #[serde(default)]
    pub how: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ApiCostResume {
    #[serde(default)]
    pub warm: f64,
    #[serde(default)]
    pub cold: f64,
    #[serde(default)]
    pub each: f64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ApiCostFresh {
    #[serde(default)]
    pub summary: f64,
    #[serde(default)]
    pub first: f64,
    #[serde(default)]
    pub each: f64,
    #[serde(default)]
    pub total: f64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct WarmCold {
    #[serde(default)]
    pub warm: String,
    #[serde(default)]
    pub cold: String,
}

/// The server sends an object (`{effort, chrome, model, fast, perm, agent, remote}`); the design lab's sample sends
/// a list of ids (`["ultracode", "plan"]`). Both are accepted.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ApiFlags {
    Object(ApiFlagObject),
    List(Vec<String>),
    Other(serde_json::Value),
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ApiFlagObject {
    #[serde(default)]
    pub effort: Option<String>,
    #[serde(default)]
    pub chrome: Option<bool>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub fast: Option<bool>,
    #[serde(default)]
    pub perm: Option<String>,
    #[serde(default)]
    pub agent: Option<String>,
    #[serde(default)]
    pub remote: Option<bool>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ApiReplaced {
    pub sid: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub folder: Option<String>,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub active: Option<f64>,
    #[serde(default)]
    pub how: Option<String>,
    #[serde(default)]
    pub by: Option<ApiBy>,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub bg_id: Option<String>,
    #[serde(default, deserialize_with = "lenient_opt")]
    pub reaped: Option<ApiReaped>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ApiBy {
    #[serde(default)]
    pub sid: Option<String>,
    #[serde(default)]
    pub machine: Option<String>,
}

/// GET /api/summary, and the `summary` inside /api/sessions.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ApiSummary {
    #[serde(default)]
    pub five_hour: Option<f64>,
    #[serde(default)]
    pub seven_day: Option<f64>,
    #[serde(default)]
    pub five_hour_reset: Option<f64>,
    #[serde(default)]
    pub seven_day_reset: Option<f64>,
    #[serde(default)]
    pub week_projected: Option<f64>,
    /// When a session last saw the plan numbers (unix seconds). Switchboard learns them only when a session replies.
    #[serde(default)]
    pub limits_updated: Option<f64>,
    #[serde(default)]
    pub sessions: Option<i64>,
    #[serde(default)]
    pub running: Option<i64>,
    #[serde(default)]
    pub cold: Option<i64>,
    #[serde(default)]
    pub online: Option<i64>,
    #[serde(default)]
    pub machines: Option<i64>,
    /// v2: the one status line every client shows (server/notes.py), or null when there is nothing to say.
    #[serde(default, deserialize_with = "lenient_opt")]
    pub note: Option<ApiNote>,
}

/// The server's status note: the thing most worth knowing right now (server/notes.py).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ApiNote {
    /// need | cooling | burn | week | five | pace
    #[serde(default)]
    pub kind: String,
    /// need | warm | warn | bad | ok
    #[serde(default)]
    pub tone: String,
    #[serde(default)]
    pub text: String,
    /// Set when the note is about one chat.
    #[serde(default)]
    pub machine: Option<String>,
    #[serde(default)]
    pub sid: Option<String>,
}

impl ApiNote {
    /// "<machine>:<sid>" when the note is about one chat.
    pub fn key(&self) -> Option<String> {
        Some(format!("{}:{}", self.machine.as_deref()?, self.sid.as_deref()?))
    }
}

impl ApiSummary {
    /// A real Switchboard summary always carries these counts. Used by the host probe so some other
    /// device at the saved address, on another network, is never mistaken for the server.
    pub fn looks_real(&self) -> bool {
        self.sessions.is_some()
            && self.running.is_some()
            && self.cold.is_some()
            && self.online.is_some()
            && self.machines.is_some()
    }
}

/// GET /api/folders/<machine>?parent=<rel>: one level of the folder tree.
/// Without `parent` the server sends the flat list with recents (`folders: [{folder, recent}]`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ApiFolders {
    #[serde(default)]
    pub ok: Option<bool>,
    #[serde(default)]
    pub root: Option<String>,
    /// Present only on a tree reply.
    #[serde(default)]
    pub parent: Option<String>,
    #[serde(default)]
    pub folders: Vec<ApiFolder>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ApiFolder {
    #[serde(default)]
    pub folder: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub kids: Option<bool>,
    #[serde(default)]
    pub recent: Option<f64>,
}

/// POST replies. Jobs come back as `{ok, job, machine: "server"}`; plain verbs as the machine's reply.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ApiPostReply {
    #[serde(default)]
    pub ok: Option<bool>,
    #[serde(default)]
    pub job: Option<String>,
    #[serde(default)]
    pub machine: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
}

/// GET /api/job/server/<id>
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ApiJob {
    #[serde(default)]
    pub ok: Option<bool>,
    /// "queued" | "running" | "done" | "error"
    #[serde(default)]
    pub state: Option<String>,
    #[serde(default)]
    pub step: Option<String>,
    #[serde(default)]
    pub note: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub sid: Option<String>,
    #[serde(default)]
    pub machine: Option<String>,
    #[serde(default)]
    pub kind: Option<String>,
    /// The background id of the chat a resume or new job started (spec v2 section 1: "Resume and open").
    #[serde(default)]
    pub bg_id: Option<String>,
    /// v2, a job sent to "auto": "Auto picked Laptop. Has Xcode 26." once the machine is chosen.
    #[serde(default)]
    pub picked: Option<String>,
    /// "claude" | "rules": who made that pick.
    #[serde(default)]
    pub by: Option<String>,
    #[serde(default)]
    pub label: Option<String>,
}

/// v2: GET /api/thread/<machine>/<sid>, the thread oldest first.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ApiThread {
    #[serde(default)]
    pub ok: Option<bool>,
    #[serde(default)]
    pub thread: Option<String>,
    #[serde(default, deserialize_with = "lenient_vec")]
    pub chats: Vec<ApiThreadChat>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ApiThreadChat {
    pub machine: String,
    pub sid: String,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub started: Option<f64>,
    #[serde(default)]
    pub ended: Option<f64>,
    #[serde(default)]
    pub ctx: Option<f64>,
    /// fresh | resumed | continued | handoff | same-topic | linked | current
    #[serde(default)]
    pub how: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    /// The chat this thread was asked for.
    #[serde(default)]
    pub this: Option<bool>,
}

/// v2: GET /api/chats?q=&exclude=<machine>/<sid>, older chats for "This continues...".
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ApiChats {
    #[serde(default)]
    pub ok: Option<bool>,
    #[serde(default, deserialize_with = "lenient_vec")]
    pub chats: Vec<ApiChatPick>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ApiChatPick {
    pub machine: String,
    pub sid: String,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub started: Option<f64>,
    #[serde(default)]
    pub ended: Option<f64>,
    #[serde(default)]
    pub ctx: Option<f64>,
    #[serde(default)]
    pub folder: Option<String>,
    #[serde(default)]
    pub in_thread: Option<bool>,
}

/// v2: POST /api/route, where Auto would run a task.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ApiRoute {
    #[serde(default)]
    pub ok: Option<bool>,
    #[serde(default)]
    pub machine: Option<String>,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub folder: Option<String>,
    #[serde(default)]
    pub create: Option<bool>,
    #[serde(default)]
    pub why: Option<String>,
    /// "claude" | "rules"
    #[serde(default)]
    pub by: Option<String>,
    /// false when Claude picks are off (Settings, Let Claude pick for Auto), so the rules picked. Absent otherwise.
    #[serde(default)]
    pub ai: Option<bool>,
}

impl ApiRoute {
    /// The rules picked because Claude picks are turned off, so the app can say where to turn them on.
    pub fn ai_off(&self) -> bool {
        self.ai == Some(false)
    }
}

/// v2: the reply to the keep verb.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ApiKeep {
    #[serde(default)]
    pub ok: Option<bool>,
    #[serde(default)]
    pub keep: Option<bool>,
    /// The reply to the star verb.
    #[serde(default)]
    pub star: Option<bool>,
}

fn lenient_vec<'de, D, T>(d: D) -> Result<Vec<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::de::DeserializeOwned,
{
    let v = serde_json::Value::deserialize(d)?;
    match v {
        serde_json::Value::Array(items) => {
            Ok(items.into_iter().filter_map(|x| serde_json::from_value(x).ok()).collect())
        }
        _ => Ok(Vec::new()),
    }
}

/// An optional object that is dropped (None) when it is not the expected shape, so one odd field never sinks a refresh.
fn lenient_opt<'de, D, T>(d: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::de::DeserializeOwned,
{
    let v = serde_json::Value::deserialize(d)?;
    Ok(if v.is_null() { None } else { serde_json::from_value(v).ok() })
}

/// v2: GET /api/flags?machine=&sid=: what one chat runs with (the flags drawer's "Active now").
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ApiFlagsView {
    #[serde(default, deserialize_with = "lenient_vec")]
    pub active: Vec<ApiFlagItem>,
    /// Flags on its command line Switchboard does not carry over (a relaunch leaves them out).
    #[serde(default, deserialize_with = "lenient_vec")]
    pub other: Vec<String>,
    /// "Opus · High · Asks first", for the terminal's header.
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub short: String,
    /// False when that machine's agent does not report every flag yet.
    #[serde(default)]
    pub complete: bool,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub running: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ApiFlagItem {
    pub name: String,
    #[serde(default)]
    pub value: Option<String>,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub text: String,
    /// live | key | relaunch | locked
    #[serde(default)]
    pub how: String,
}

/// v2: POST /api/flags: each change against what it runs with, and how it gets there.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ApiFlagsPlan {
    #[serde(default, deserialize_with = "lenient_vec")]
    pub changes: Vec<ApiFlagChange>,
    /// Slash commands typed into the chat, in order (only when nothing needs a relaunch).
    #[serde(default)]
    pub cmds: Vec<String>,
    #[serde(default)]
    pub relaunch: bool,
    #[serde(default)]
    pub running: bool,
    #[serde(default)]
    pub busy: bool,
    #[serde(default)]
    pub dropped: Vec<String>,
    /// What POST /api/relaunch takes: {model, effort, flags}.
    #[serde(default)]
    pub body: serde_json::Value,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ApiFlagChange {
    pub name: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub from: Option<String>,
    #[serde(default)]
    pub to: Option<String>,
    #[serde(default)]
    pub how: String,
    #[serde(default)]
    pub cmd: Option<String>,
}

/// v2: POST /api/relaunch's answer: a job, or what to run in the chat's own terminal (`need` "terminal").
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ApiRelaunch {
    #[serde(default)]
    pub ok: Option<bool>,
    #[serde(default)]
    pub job: Option<String>,
    #[serde(default)]
    pub need: Option<String>,
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
}

/// v2: GET /api/machines (Settings): every machine as configured, in order.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ApiMachines {
    #[serde(default, deserialize_with = "lenient_vec")]
    pub machines: Vec<ApiMachineRow>,
    #[serde(default)]
    pub max: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ApiMachineRow {
    pub name: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub os: Option<String>,
    #[serde(default)]
    pub form: Option<String>,
    /// "link" (paired with switchboard-node) or "ssh".
    #[serde(default)]
    pub via: Option<String>,
    /// user@host for an ssh machine.
    #[serde(default)]
    pub target: Option<String>,
    /// The Claude folder as picked in Settings ("~/Desktop/Claude"); None while the machine uses its home folder.
    #[serde(default)]
    pub root: Option<String>,
    /// The Claude folder as shown: the one picked, else the one the machine reports using.
    #[serde(default)]
    pub root_shown: Option<String>,
    /// Nobody has picked this machine's Claude folder yet: Settings asks.
    #[serde(default)]
    pub needs_root: bool,
    /// Where the machine thinks its Claude folder is ("~/Desktop/Claude" on a Mac when it exists).
    #[serde(default)]
    pub root_suggest: Option<String>,
    #[serde(default)]
    pub online: Option<bool>,
    #[serde(default)]
    pub updated: Option<f64>,
    #[serde(default, deserialize_with = "lenient_opt")]
    pub versions: Option<ApiVersions>,
    #[serde(default, deserialize_with = "lenient_vec")]
    pub features: Vec<String>,
    /// Auto may send work here.
    #[serde(default)]
    pub auto: bool,
    /// What Auto sees it can do, in one line.
    #[serde(default)]
    pub can: Option<String>,
}

impl ApiMachineRow {
    pub fn has(&self, feature: &str) -> bool {
        self.features.iter().any(|f| f == feature)
    }
}

/// The build of Switchboard's agent and of Claude Code on a machine.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ApiVersions {
    #[serde(default)]
    pub agent: Option<String>,
    #[serde(default)]
    pub claude: Option<String>,
}

/// GET /api/machines/<name>/folders?path=: one level of folders under a machine's home folder, to pick its Claude folder.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ApiRootDirs {
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub shown: String,
    #[serde(default, deserialize_with = "lenient_vec")]
    pub folders: Vec<ApiRootDir>,
    #[serde(default)]
    pub current: Option<String>,
    #[serde(default)]
    pub set: bool,
    #[serde(default)]
    pub suggest: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ApiRootDir {
    pub name: String,
    /// Home-relative, with forward slashes.
    pub path: String,
    #[serde(default)]
    pub kids: bool,
}

/// GET /api/machines/<name>/mcp: the MCP servers Claude Code has configured there, with every secret taken out.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ApiMcp {
    #[serde(default, deserialize_with = "lenient_vec")]
    pub servers: Vec<ApiMcpServer>,
    #[serde(default)]
    pub root: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ApiMcpServer {
    pub name: String,
    /// user, local, project or plugin
    #[serde(default)]
    pub scope: String,
    #[serde(default)]
    pub plugin: Option<String>,
    #[serde(default, rename = "type")]
    pub kind: Option<String>,
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub url: Option<String>,
    /// Names only: the values never leave the machine.
    #[serde(default)]
    pub env: Vec<String>,
    #[serde(default)]
    pub headers: Vec<String>,
    #[serde(default)]
    pub status: Option<String>,
    /// For local scope: the folder under the Claude folder it belongs to.
    #[serde(default)]
    pub folder: Option<String>,
}

impl ApiMcpServer {
    /// "npx -y @modelcontextprotocol/server-github", or the URL.
    pub fn line(&self) -> String {
        match (&self.command, &self.url) {
            (Some(c), _) => {
                std::iter::once(c.as_str()).chain(self.args.iter().map(|a| a.as_str())).collect::<Vec<_>>().join(" ")
            }
            (None, Some(u)) => u.clone(),
            _ => String::new(),
        }
    }

    /// "User", "Project", "Local, in proj", "Plugin tools".
    pub fn scope_label(&self) -> String {
        match self.scope.as_str() {
            "user" => "User".into(),
            "project" => "Project".into(),
            "local" => match self.folder.as_deref().filter(|f| !f.is_empty()) {
                Some(f) => format!("Local, in {f}"),
                None => "Local".into(),
            },
            "plugin" => match &self.plugin {
                Some(p) => format!("Plugin {p}"),
                None => "Plugin".into(),
            },
            o => o.to_string(),
        }
    }
}

/// GET /api/machines/<name>/caps: what Settings shows under Capabilities.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ApiCapsView {
    /// Auto may send work here.
    #[serde(default)]
    pub on: bool,
    /// The user's own lines: "this machine can also do...".
    #[serde(default)]
    pub extra: Vec<String>,
    #[serde(default)]
    pub off: Vec<String>,
    /// What was found, each with its switch; None before the machine has answered.
    #[serde(default)]
    pub tools: Option<Vec<ApiCapTool>>,
    #[serde(default)]
    pub measured: Option<f64>,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub can: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ApiCapTool {
    pub key: String,
    pub name: String,
    #[serde(default)]
    pub detail: Option<String>,
    #[serde(default)]
    pub on: bool,
}

/// v2: POST /api/upload's answer: where the file landed on the chat's machine.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ApiUpload {
    #[serde(default)]
    pub ok: Option<bool>,
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub size: Option<u64>,
    #[serde(default)]
    pub error: Option<String>,
}

/// POST /api/compact: without confirm, what compacting a History thread costs; with it, the job.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ApiCompact {
    #[serde(default)]
    pub chats: u64,
    /// How many of them could be read (a machine that is offline leaves its chats out).
    #[serde(default)]
    pub found: u64,
    /// How many already have a written summary (those are used as they are).
    #[serde(default)]
    pub notes: u64,
    #[serde(default)]
    pub machine: Option<String>,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub sid: Option<String>,
    #[serde(default)]
    pub cost: Option<ApiCompactCost>,
    #[serde(default)]
    pub job: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ApiCompactCost {
    #[serde(default)]
    pub tokens: f64,
    #[serde(default)]
    pub summary: f64,
    #[serde(default)]
    pub first: f64,
    #[serde(default)]
    pub total: f64,
    #[serde(default)]
    pub each: f64,
    #[serde(default)]
    pub text: String,
}

/// GET /api/handoff/<machine>/<sid>: a chat's written summary.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ApiHandoff {
    #[serde(default)]
    pub text: String,
}

/// A list of strings that drops anything that is not one (and reads null or a non-list as empty).
fn lenient_strings<'de, D>(d: D) -> Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let v = serde_json::Value::deserialize(d)?;
    Ok(match v {
        serde_json::Value::Array(items) => items.into_iter().filter_map(|x| x.as_str().map(str::to_string)).collect(),
        _ => Vec::new(),
    })
}

/// machine -> tool -> its state there.
pub type ApiLibraryMachines = std::collections::BTreeMap<String, std::collections::BTreeMap<String, ApiToolOnMachine>>;

/// GET /api/library: the tool library, the tools on for every new chat, and what each machine has.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ApiLibrary {
    #[serde(default, deserialize_with = "lenient_vec")]
    pub tools: Vec<ApiTool>,
    #[serde(default, deserialize_with = "lenient_strings")]
    pub defaults: Vec<String>,
    /// From each machine's last answer, kept while it is offline.
    #[serde(default, deserialize_with = "lenient_machine_tools")]
    pub machines: ApiLibraryMachines,
}

impl ApiLibrary {
    pub fn tool(&self, name: &str) -> Option<&ApiTool> {
        self.tools.iter().find(|t| t.name == name)
    }

    pub fn is_default(&self, name: &str) -> bool {
        self.defaults.iter().any(|d| d == name)
    }

    /// The server knows Auto (its tools say whether they are in it). An older one does not: New chat keeps the list.
    pub fn has_auto(&self) -> bool {
        self.tools.iter().any(|t| t.auto.is_some())
    }

    /// What Auto gives a chat: (skills in Auto, skills in all, MCP tools in Auto), for "All skills + 4 MCP tools". A
    /// tool that is both counts as an MCP tool.
    pub fn auto_counts(&self) -> (usize, usize, usize) {
        let mcp = |t: &ApiTool| t.kinds.iter().any(|k| k == "mcp");
        let skills: Vec<&ApiTool> = self.tools.iter().filter(|t| !mcp(t)).collect();
        let in_auto = |t: &ApiTool| t.auto != Some(false);
        (
            skills.iter().filter(|t| in_auto(t)).count(),
            skills.len(),
            self.tools.iter().filter(|t| mcp(t) && in_auto(t)).count(),
        )
    }

    /// The tool's state on `machine`, if the machine has said anything about it.
    pub fn on_machine(&self, machine: &str, tool: &str) -> Option<&ApiToolOnMachine> {
        self.machines.get(machine).and_then(|m| m.get(tool))
    }
}

fn lenient_machine_tools<'de, D>(d: D) -> Result<ApiLibraryMachines, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let v = serde_json::Value::deserialize(d)?;
    let mut out = ApiLibraryMachines::new();
    if let serde_json::Value::Object(machines) = v {
        for (m, tools) in machines {
            let mut row = std::collections::BTreeMap::new();
            if let serde_json::Value::Object(tools) = tools {
                for (t, state) in tools {
                    if let Ok(st) = serde_json::from_value::<ApiToolOnMachine>(state) {
                        row.insert(t, st);
                    }
                }
            }
            out.insert(m, row);
        }
    }
    Ok(out)
}

/// One tool in the library.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ApiTool {
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// "skill" and or "mcp".
    #[serde(default, deserialize_with = "lenient_strings")]
    pub kinds: Vec<String>,
    /// "local" (copied to machines); "hosted" is reserved.
    #[serde(default)]
    pub mode: String,
    #[serde(default, deserialize_with = "lenient_vec")]
    pub secrets: Vec<ApiToolSecret>,
    /// A skill declares secrets: its scripts run through Claude's Bash, so Claude could read them.
    #[serde(default)]
    pub skill_secrets: bool,
    #[serde(default)]
    pub latest: u32,
    #[serde(default, deserialize_with = "lenient_vec")]
    pub versions: Vec<ApiToolVersion>,
    /// In Auto: given to every chat that starts with tools "auto" (a skill or an MCP tool alike). None from an older
    /// server, which has no Auto.
    #[serde(default, deserialize_with = "lenient_opt")]
    pub auto: Option<bool>,
    /// When Claude should reach for it, one line; Auto passes it on.
    #[serde(default, deserialize_with = "lenient_opt")]
    pub when: Option<String>,
}

/// A secret a tool needs, by name only. The value lives in each machine's keychain.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ApiToolSecret {
    pub name: String,
    #[serde(default)]
    pub about: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ApiToolVersion {
    pub v: u32,
    /// Epoch seconds.
    #[serde(default)]
    pub at: f64,
    /// The machine that published it.
    #[serde(default)]
    pub by: String,
    #[serde(default)]
    pub sha256: String,
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub note: String,
}

/// One tool on one machine.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ApiToolOnMachine {
    #[serde(default)]
    pub connected: bool,
    /// The version installed there, if any.
    #[serde(default)]
    pub installed: Option<u32>,
    #[serde(default, deserialize_with = "lenient_strings")]
    pub missing_secrets: Vec<String>,
    #[serde(default)]
    pub error: Option<String>,
}

/// GET /api/library/sources?machine=: what can be imported from that machine.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ApiLibrarySources {
    #[serde(default, deserialize_with = "lenient_vec")]
    pub skills: Vec<ApiSkillSource>,
    #[serde(default, deserialize_with = "lenient_vec")]
    pub mcp: Vec<ApiMcpSource>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ApiSkillSource {
    pub name: String,
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub files: u64,
    #[serde(default)]
    pub size: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ApiMcpSource {
    pub name: String,
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default, deserialize_with = "lenient_strings")]
    pub args: Vec<String>,
    #[serde(default, deserialize_with = "lenient_strings")]
    pub env_names: Vec<String>,
    #[serde(default)]
    pub scope: String,
}

impl ApiMcpSource {
    /// "npx -y @modelcontextprotocol/server-github".
    pub fn line(&self) -> String {
        self.command.iter().chain(self.args.iter()).map(|a| a.as_str()).collect::<Vec<_>>().join(" ")
    }
}

/// POST /api/library/import (and publish): the tool and the version it stored.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ApiToolStored {
    #[serde(default)]
    pub tool: String,
    #[serde(default)]
    pub version: u32,
}

#[cfg(test)]
mod library_tests {
    use super::*;

    #[test]
    fn a_library_reads() {
        let raw = r#"{"tools":[{"name":"web-fetch","description":"Fetch blocked pages","kinds":["skill","mcp"],"mode":"local",
            "secrets":[{"name":"WEB_FETCH_KEY","about":"API key"}],"skill_secrets":true,"latest":3,
            "versions":[{"v":1,"at":1791000000,"by":"studio","sha256":"ab","size":1200,"note":""},{"v":3,"at":1791000500.5,"by":"studio","sha256":"cd","size":1300,"note":"faster"}]},
            {"name":"odd","kinds":null,"secrets":"no","versions":[{"nope":1}]}],
          "defaults":["web-fetch", 7],
          "machines":{"studio":{"web-fetch":{"connected":true,"installed":3,"missing_secrets":[],"error":null}},
                      "tower":{"web-fetch":{"connected":true,"installed":null,"missing_secrets":["WEB_FETCH_KEY"],"error":"no keychain"},"odd":"bad"},
                      "win":null}}"#;
        let lib: ApiLibrary = serde_json::from_str(raw).unwrap();
        assert_eq!(lib.tools.len(), 2);
        let t = lib.tool("web-fetch").unwrap();
        assert_eq!(t.kinds, vec!["skill", "mcp"]);
        assert!(t.skill_secrets);
        assert_eq!(t.latest, 3);
        assert_eq!(t.versions[1].note, "faster");
        assert_eq!(t.secrets[0].name, "WEB_FETCH_KEY");
        let odd = lib.tool("odd").unwrap();
        assert!(odd.kinds.is_empty() && odd.secrets.is_empty() && odd.versions.is_empty());
        assert_eq!(lib.defaults, vec!["web-fetch"]);
        assert!(lib.is_default("web-fetch") && !lib.is_default("odd"));
        assert_eq!(lib.on_machine("studio", "web-fetch").unwrap().installed, Some(3));
        let b = lib.on_machine("tower", "web-fetch").unwrap();
        assert_eq!(b.installed, None);
        assert_eq!(b.missing_secrets, vec!["WEB_FETCH_KEY"]);
        assert_eq!(b.error.as_deref(), Some("no keychain"));
        assert!(lib.on_machine("tower", "odd").is_none());
        assert!(lib.machines.get("win").unwrap().is_empty());
        assert_eq!(serde_json::from_str::<ApiLibrary>("{}").unwrap(), ApiLibrary::default());
    }

    #[test]
    fn budget_reads_leniently() {
        let raw = r#"{"budget": {"settings": {"daily": 14.3, "five_hour": null, "hard": {"on": true, "daily": 20.0}, "lifted_until": null},
          "status": {"known": true, "today": 22.0, "over_daily": true, "over_budget": true, "over_hard": true, "hard_reason": "daily",
                     "lifted": false, "blocking": true, "summary": "Today: 22% of the week."}}}"#;
        let s: ApiSessions = serde_json::from_str(raw).unwrap();
        let b = s.budget.unwrap();
        assert_eq!(b.settings().daily, Some(14.3));
        assert_eq!(b.settings().hard().on, Some(true));
        assert!(b.status().blocking() && b.status().over_daily() && !b.status().lifted());
        // Odd types drop the field, not the snapshot; an older server has no budget.
        let odd: ApiSessions =
            serde_json::from_str(r#"{"budget": {"settings": {"daily": "lots"}, "status": 3}}"#).unwrap();
        assert_eq!(odd.budget.unwrap().settings().daily, None);
        assert!(serde_json::from_str::<ApiSessions>("{}").unwrap().budget.is_none());
    }

    #[test]
    fn auto_reads_leniently() {
        // A server with Auto: tools say whether they are in it; skills count whole, MCP tools only when in Auto.
        let raw = r#"{"tools":[{"name":"a","kinds":["mcp"],"auto":true,"when":"blocked pages"},{"name":"b","kinds":["mcp"],"auto":false},
          {"name":"c","kinds":["skill"],"auto":"yes","when":null},{"name":"d","kinds":["skill"]}]}"#;
        let lib: ApiLibrary = serde_json::from_str(raw).unwrap();
        assert!(lib.has_auto());
        assert_eq!(lib.auto_counts(), (2, 2, 1));
        assert_eq!(lib.tool("a").unwrap().when.as_deref(), Some("blocked pages"));
        assert_eq!(lib.tool("c").unwrap().auto, None);
        // An older server: no Auto.
        let old: ApiLibrary = serde_json::from_str(r#"{"tools":[{"name":"a","kinds":["mcp"]}]}"#).unwrap();
        assert!(!old.has_auto());
        let s: ApiSession = serde_json::from_str(r#"{"sid":"a","tools":["x"],"tools_mode":"auto"}"#).unwrap();
        assert_eq!(s.tools_mode.as_deref(), Some("auto"));
        let s: ApiSession = serde_json::from_str(r#"{"sid":"a","tools_mode":7}"#).unwrap();
        assert_eq!(s.tools_mode, None);
    }

    #[test]
    fn sources_read() {
        let raw = r#"{"skills":[{"name":"video-notes","path":"/Users/you/.claude/skills/video-notes","files":4,"size":9100}],
          "mcp":[{"name":"github","command":"npx","args":["-y","@modelcontextprotocol/server-github"],"env_names":["GITHUB_TOKEN"],"scope":"user"},
                 {"name":"remote","command":null}]}"#;
        let s: ApiLibrarySources = serde_json::from_str(raw).unwrap();
        assert_eq!(s.skills[0].files, 4);
        assert_eq!(s.mcp.len(), 2);
        assert_eq!(s.mcp[0].env_names, vec!["GITHUB_TOKEN"]);
        assert_eq!(s.mcp[0].line(), "npx -y @modelcontextprotocol/server-github");
        assert_eq!(s.mcp[1].command, None);
        assert!(s.mcp[1].args.is_empty());
        let stored: ApiToolStored = serde_json::from_str(r#"{"ok":true,"tool":"github","version":2}"#).unwrap();
        assert_eq!(stored, ApiToolStored { tool: "github".into(), version: 2 });
    }

    #[test]
    fn session_tools_default_to_empty() {
        let old: ApiSession = serde_json::from_str(r#"{"sid":"a"}"#).unwrap();
        assert!(old.tools.is_empty());
        let null: ApiSession = serde_json::from_str(r#"{"sid":"a","tools":null}"#).unwrap();
        assert!(null.tools.is_empty());
        let new: ApiSession = serde_json::from_str(r#"{"sid":"a","tools":["web-fetch","video-notes"]}"#).unwrap();
        assert_eq!(new.tools, vec!["web-fetch", "video-notes"]);
    }
}

#[cfg(test)]
mod options_tests {
    use super::*;

    /// The shape server/options.py view() returns, inside GET /api/prefs.
    const PREFS: &str = r#"{"ok": true, "chrome_machine": null, "presets": [], "summary_model": "opus",
      "options": [
        {"key": "ai_ask_cards", "kind": "ai", "label": "Summarize waiting chats", "on": false,
         "explain": "Uses your Claude plan to write a short summary of what a waiting chat is asking. The chat's last message is sent to Claude.",
         "off": "When off, the card shows what the chat says it needs, as it said it."},
        {"key": "ai_auto_machine", "kind": "ai", "label": "Let Claude pick for Auto", "on": true,
         "explain": "Uses your Claude plan to pick the machine for a task sent to Auto.", "off": "When off, Auto picks by fixed rules."},
        {"key": "setup_explore_agent", "kind": "setup", "label": "Explore agent on Sonnet", "on": false,
         "explain": "Writes ~/.claude/agents/Explore.md on each machine.",
         "off": "When off, Switchboard deletes the file only if it is the one Switchboard wrote."},
        {"kind": "ai", "label": "no key"}, "odd"],
      "push": {"ready": false, "explain": "Notifications are off.", "setup": "~/.config/switchboard/apns.json"}}"#;

    #[test]
    fn options_and_push_read() {
        let p: ApiPrefs = serde_json::from_str(PREFS).unwrap();
        assert_eq!(p.options.len(), 3, "a row with no key, and one that is not an object, are dropped");
        let ai: Vec<&str> = p.options_of("ai").map(|o| o.key.as_str()).collect();
        assert_eq!(ai, ["ai_ask_cards", "ai_auto_machine"]);
        assert_eq!(p.options_of("setup").count(), 1);
        let a = p.option("ai_ask_cards").unwrap();
        assert_eq!(a.label, "Summarize waiting chats");
        assert!(!a.on);
        assert!(a.off.starts_with("When off,"));
        assert!(p.option("ai_auto_machine").unwrap().on);
        let push = p.push.as_ref().unwrap();
        assert!(!push.ready);
        assert_eq!(push.setup.as_deref(), Some("~/.config/switchboard/apns.json"));
    }

    #[test]
    fn push_ready_has_no_setup() {
        let raw = r#"{"push": {"ready": true, "explain": "Notifications go through Apple.", "setup": null}}"#;
        let push = serde_json::from_str::<ApiPrefs>(raw).unwrap().push.unwrap();
        assert!(push.ready && push.setup.is_none());
    }

    #[test]
    fn an_older_server_has_no_options() {
        let p: ApiPrefs = serde_json::from_str(r#"{"ok": true, "presets": [], "options": "nope", "push": 3}"#).unwrap();
        assert!(p.options.is_empty() && p.push.is_none() && p.option("ai_ask_cards").is_none());
    }

    #[test]
    fn set_option_and_body() {
        let mut p: ApiPrefs = serde_json::from_str(PREFS).unwrap();
        p.set_option("ai_ask_cards", true);
        assert!(p.option("ai_ask_cards").unwrap().on);
        p.set_option("not_a_key", true);
        assert_eq!(p.options.len(), 3);
        assert_eq!(option_body("setup_explore_agent", false), serde_json::json!({"setup_explore_agent": false}));
        assert_eq!(option_body("ai_chat_names", true).to_string(), r#"{"ai_chat_names":true}"#);
    }

    #[test]
    fn route_says_when_claude_picks_are_off() {
        let raw = r#"{"ok": true, "machine": "studio", "label": "Studio", "folder": "", "create": false,
            "why": "free memory", "by": "rules", "ai": false}"#;
        assert!(serde_json::from_str::<ApiRoute>(raw).unwrap().ai_off());
        let r: ApiRoute =
            serde_json::from_str(r#"{"ok": true, "machine": "studio", "by": "claude", "ms": 900}"#).unwrap();
        assert!(!r.ai_off());
    }
}
