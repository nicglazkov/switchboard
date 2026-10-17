//! Switchboard core: everything the apps share and nothing that draws.
//!
//! - `ask`: what a waiting chat needs, shaped by kind, and the reply each answer becomes.
//! - `api`: the server's wire format. `model`: the normalized board (sessions, machines, plan) and its rules
//!   (lamp, phrase, explain, chips). `names`, `fmt`: words and numbers, as the web app prints them.
//! - `board`: the Swimlanes grid. `actions`: what you can do with a chat. `jobs`: following server jobs.
//! - `links`: claude.ai session links as the Claude desktop app's own links.
//! - `net`: HTTP with the fallback across saved addresses. `discover`: finding the server on the local network
//!   when none is saved. `cache`: the on-disk snapshot and folder trees.
//! - `term`: terminals: the /api/term ticket, the WebSocket to the hub, and the bytes each key sends.
//! - `Core`: a small facade over all of it with owned inputs and outputs, no generics and no callbacks, so
//!   UniFFI can wrap it for SwiftUI and Compose later (see native/STACK.md).

pub mod actions;
pub mod api;
pub mod ask;
pub mod board;
pub mod brand;
pub mod cache;
pub mod discover;
pub mod flags;
pub mod fmt;
pub mod groups;
pub mod jobs;
pub mod links;
pub mod live;
pub mod model;
pub mod names;
pub mod net;
pub mod search;
pub mod servers;
pub mod term;
pub mod update;
pub mod usage;

use std::path::PathBuf;
use std::sync::Mutex;

pub use actions::{Action, ActionId};
pub use board::{Board, Column, Tally};
pub use jobs::{Job, JobKind, JobState};
pub use model::{
    Activity, Cache, Chip, Kind, Lamp, Machine, MachineLamp, Mem, Plan, Reaped, Replaced, Session, Snapshot, TextRun,
    Tone,
};
pub use net::{BusyInfo, ContinueRequest, CoreError, NewRequest};
pub use term::{Need, TermError, TermRequest, TermTicket};

pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// One folder level for the picker.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FolderLevel {
    /// The machine's Claude folder (absolute path) when known.
    pub root: Option<String>,
    pub folders: Vec<api::ApiFolder>,
}

/// What the picker shows first for a machine, from the cache when there is one.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FolderStart {
    pub root: Option<String>,
    pub recent: Vec<api::ApiFolder>,
    pub top: Option<Vec<api::ApiFolder>>,
}

/// What a Stop from the cleanup card did.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CleanStopped {
    /// Waiting for their work to finish before they stop: ("machine/sid", job id).
    pub queued: Vec<(String, String)>,
    /// Skipped (in use again, kept) or failed: still running. ("machine/sid", why; empty when skipped).
    pub not_stopped: Vec<(String, String)>,
}

impl CleanStopped {
    pub fn read(r: &serde_json::Value) -> Self {
        let list = |k: &str| r.get(k).and_then(|v| v.as_array()).cloned().unwrap_or_default();
        let queued = list("queued")
            .iter()
            .filter_map(|e| Some((e.get("key")?.as_str()?.to_string(), e.get("job")?.as_str()?.to_string())))
            .collect();
        let mut not_stopped: Vec<(String, String)> =
            list("skipped").iter().filter_map(|k| Some((k.as_str()?.to_string(), String::new()))).collect();
        not_stopped.extend(list("failed").iter().filter_map(|e| {
            Some((
                e.get("key")?.as_str()?.to_string(),
                e.get("error").and_then(|x| x.as_str()).unwrap_or("").to_string(),
            ))
        }));
        CleanStopped { queued, not_stopped }
    }
}

#[derive(Debug, Clone, Default)]
pub struct CoreConfig {
    /// Server base URLs in order of preference. Empty: look for the server on the local network.
    pub servers: Vec<String>,
    /// Override the cache dir (tests, or a second profile).
    pub cache_dir: Option<String>,
    pub user_agent: Option<String>,
}

/// The facade. Every call blocks; call it off the UI thread.
pub struct Core {
    client: net::Client,
    ua: String,
    dir: Option<PathBuf>,
    folders: Mutex<cache::FolderCache>,
}

impl Core {
    pub fn new(config: CoreConfig) -> Core {
        let ua = config.user_agent.unwrap_or_else(|| format!("SwitchboardDesktop/{}", version()));
        let dir = config.cache_dir.map(PathBuf::from).or_else(cache::default_dir);
        let folders = dir.as_deref().map(cache::FolderCache::load).unwrap_or_default();
        Core { client: net::Client::new(config.servers, &ua), ua, dir, folders: Mutex::new(folders) }
    }

    pub fn cache_dir(&self) -> Option<String> {
        self.dir.as_ref().map(|d| d.display().to_string())
    }

    /// The snapshot saved by the last good refresh, if it is less than a day old. Paints the first frame.
    pub fn cached_snapshot(&self) -> Option<Snapshot> {
        let dir = self.dir.as_ref()?;
        let (raw, at, _) = cache::load_sessions(dir, fmt::now_epoch())?;
        Some(model::normalize(&raw, at))
    }

    /// Fetches the board, saves it for the next start and returns it normalized.
    pub fn refresh(&self) -> Result<Snapshot, CoreError> {
        let (raw, bytes, base) = self.client.sessions()?;
        let now = fmt::now_epoch();
        if let Some(dir) = &self.dir
            && let Err(e) = cache::save_sessions(dir, &bytes, now, &base)
        {
            log::warn!("could not save the snapshot: {e}");
        }
        Ok(model::normalize(&raw, now))
    }

    /// The server in use ("http://192.0.2.10:8130"), once one has answered.
    pub fn server(&self) -> Option<String> {
        self.client.base()
    }

    /// The saved server addresses, in order of preference (empty: it looks on the local network).
    pub fn servers(&self) -> Vec<String> {
        self.client.resolver().candidates()
    }

    /// Replaces the saved addresses (the user typed one); the next call looks again.
    pub fn set_servers(&self, servers: Vec<String>) {
        self.client.resolver().set_candidates(servers);
    }

    /// What the probes found since the last call, (address, answered): how the saved list learns which addresses fail
    /// (servers.rs).
    pub fn take_probe_outcomes(&self) -> Vec<(String, bool)> {
        self.client.resolver().take_outcomes()
    }

    /// No address is saved and none was found on the local network: the app has to ask for one.
    pub fn needs_server(&self) -> bool {
        self.servers().is_empty() && self.server().is_none()
    }

    /// What the folder picker can show at once for `machine`, from the cache.
    pub fn cached_folders(&self, machine: &str) -> FolderStart {
        let f = self.folders.lock().map(|f| f.machine(machine).cloned()).ok().flatten().unwrap_or_default();
        FolderStart { root: f.root.clone(), recent: f.recent.clone(), top: f.tree.get("").cloned() }
    }

    pub fn cached_level(&self, machine: &str, parent: &str) -> Option<Vec<api::ApiFolder>> {
        self.folders.lock().ok()?.level(machine, parent).cloned()
    }

    /// One level of the folder tree ("" is the top), fetched and cached.
    pub fn folder_level(&self, machine: &str, parent: &str) -> Result<FolderLevel, CoreError> {
        let r = self.client.folders(machine, Some(parent))?;
        self.remember(machine, Some(parent), &r);
        let folders =
            if r.parent.is_some() { r.folders.clone() } else { self.cached_level(machine, parent).unwrap_or_default() };
        Ok(FolderLevel { root: r.root.clone(), folders })
    }

    /// The flat list with recents; updates the cached recents and root.
    pub fn folder_recents(&self, machine: &str) -> Result<FolderStart, CoreError> {
        let r = self.client.folders(machine, None)?;
        self.remember(machine, None, &r);
        Ok(self.cached_folders(machine))
    }

    fn remember(&self, machine: &str, parent: Option<&str>, r: &api::ApiFolders) {
        if let Ok(mut f) = self.folders.lock() {
            f.take(machine, parent, r, fmt::now_epoch());
            if let Some(dir) = &self.dir {
                let _ = f.save(dir);
            }
        }
    }

    /// POST /api/action. Returns the job id for fresh, resume and handoff.
    pub fn act(&self, machine: &str, verb: &str, sid: &str, close: Option<bool>) -> Result<Option<String>, CoreError> {
        self.act_when(machine, verb, sid, close, None)
    }

    /// The same, after the user answered a need "busy": `when` "idle" (queued until the chat's work ends) or "now".
    pub fn act_when(
        &self,
        machine: &str,
        verb: &str,
        sid: &str,
        close: Option<bool>,
        when: Option<&str>,
    ) -> Result<Option<String>, CoreError> {
        Ok(self.client.action(machine, verb, sid, close, when)?.job)
    }

    /// v2: Keep running on or off; returns what the server stored.
    pub fn keep(&self, machine: &str, sid: &str, on: bool) -> Result<bool, CoreError> {
        self.client.keep(machine, sid, on)
    }

    /// Star (favorite) on or off; returns what the server stored.
    pub fn star(&self, machine: &str, sid: &str, on: bool) -> Result<bool, CoreError> {
        self.client.star(machine, sid, Some(on))
    }

    /// A tab group change (POST /api/groups). Returns every group as the server now has them, and the server's id
    /// for a group a create made.
    pub fn group_op(&self, op: &groups::Op) -> Result<(Vec<groups::Group>, Option<String>), CoreError> {
        let r = self.client.groups(&op.body())?;
        Ok((groups::from_api(&r.groups), r.group.map(|g| g.id).filter(|id| !id.is_empty())))
    }

    /// v2: "This continues...": (machine, sid) carries on from the older chat `to`.
    pub fn link(&self, machine: &str, sid: &str, to_machine: &str, to_sid: &str) -> Result<api::ApiThread, CoreError> {
        self.client.link(machine, sid, to_machine, to_sid)
    }

    /// v2: "Not the same topic". `older` on a replaced chat: what carries on from it splits off.
    pub fn unlink(&self, machine: &str, sid: &str, older: bool) -> Result<api::ApiThread, CoreError> {
        self.client.unlink(machine, sid, older)
    }

    /// v2: the chat's thread, oldest first.
    pub fn thread(&self, machine: &str, sid: &str) -> Result<api::ApiThread, CoreError> {
        self.client.thread(machine, sid)
    }

    /// v2: older chats to link to, matching `q`, its own folder first.
    pub fn chats(&self, q: &str, exclude_machine: &str, exclude_sid: &str) -> Result<Vec<api::ApiChatPick>, CoreError> {
        Ok(self.client.chats(q, Some((exclude_machine, exclude_sid)))?.chats)
    }

    /// v2: where Auto would run a task. `rules`: the instant preview (no Claude call).
    pub fn route(
        &self,
        prompt: &str,
        folder: &str,
        rules: bool,
        chrome: bool,
        chrome_machine: Option<&str>,
    ) -> Result<api::ApiRoute, CoreError> {
        self.client.route(prompt, folder, rules, chrome, chrome_machine)
    }

    /// One chat's own Chrome machine (None clears it).
    pub fn set_chrome(&self, machine: &str, sid: &str, chrome_machine: Option<&str>) -> Result<(), CoreError> {
        self.client.set_chrome(machine, sid, chrome_machine)
    }

    /// v2, the cleanup card: stop these (machine, sid) chats and (machine, pid) leftovers. Only ever on a click.
    /// Stops them; says which wait for their work to end and which the server did not stop (still running).
    pub fn cleanup_stop(&self, items: &[api::ApiCleanItem], when: Option<&str>) -> Result<CleanStopped, CoreError> {
        let items: Vec<serde_json::Value> = items
            .iter()
            .map(|d| match (&d.sid, d.pid) {
                (Some(sid), _) => serde_json::json!({"machine": d.machine, "sid": sid}),
                (None, pid) => serde_json::json!({"machine": d.machine, "pid": pid}),
            })
            .collect();
        let mut body = serde_json::json!({"action": "stop", "items": items});
        if let Some(w) = when {
            body["when"] = serde_json::json!(w);
        }
        self.client.cleanup(&body).map(|r| CleanStopped::read(&r))
    }

    /// The chats this app has open in tabs ("machine:sid"), for `device`: the cleanup card never offers them.
    pub fn report_tabs(&self, device: &str, tabs: &[String]) -> Result<(), CoreError> {
        self.client.tabs(&serde_json::json!({"device": device, "tabs": tabs})).map(|_| ())
    }

    /// v2, the cleanup card: leave it running and never suggest it again.
    pub fn cleanup_keep(&self, d: &api::ApiCleanItem) -> Result<(), CoreError> {
        let body = match (&d.sid, d.pid) {
            (Some(sid), _) => serde_json::json!({"action": "keep", "machine": d.machine, "sid": sid}),
            (None, pid) => serde_json::json!({"action": "keep", "machine": d.machine, "pid": pid}),
        };
        self.client.cleanup(&body).map(|_| ())
    }

    /// v2: sends a file to `machine` (where the chat runs); answers with the path there.
    pub fn upload(
        &self,
        machine: &str,
        name: &str,
        data: &[u8],
        sent: &std::sync::atomic::AtomicU64,
    ) -> Result<api::ApiUpload, CoreError> {
        self.client.upload(machine, name, data, sent)
    }

    /// v2: a newer build of this app on the download page, if there is one. Always None in a build with updates off
    /// (update::off_reason). The list comes from the build's feed URL when it has one, else from the server.
    pub fn update_offer(&self, bad_build: Option<&str>) -> Result<Option<update::Offer>, CoreError> {
        let (Some(key), Some(mine)) = (update::key(), update::this_build()) else { return Ok(None) };
        let app = match update::feed_url() {
            Some(feed) => update::fetch_feed(feed, &format!("SwitchboardDesktop/{}", version()))?,
            None => self.client.app_latest()?,
        };
        update::offer(&app, &key, update::platform(), mine, bad_build)
    }

    /// v2, Settings: the machines as configured.
    pub fn machines(&self) -> Result<api::ApiMachines, CoreError> {
        self.client.machines()
    }

    /// v2, Settings: rename, reorder, set the folder, remove or add a machine.
    pub fn machines_edit(&self, body: &serde_json::Value) -> Result<api::ApiMachines, CoreError> {
        self.client.machines_edit(body)
    }

    /// v2, Settings: one level of folders under a machine's home folder, to pick its Claude folder from.
    pub fn machine_folders(&self, name: &str, path: &str) -> Result<api::ApiRootDirs, CoreError> {
        self.client.machine_folders(name, path)
    }

    /// v2, Settings: a machine's MCP servers, read only and with nothing secret.
    pub fn machine_mcp(&self, name: &str) -> Result<api::ApiMcp, CoreError> {
        self.client.machine_mcp(name)
    }

    /// v2, Settings: what a machine can do, with the user's switches and lines.
    pub fn machine_caps(&self, name: &str) -> Result<api::ApiCapsView, CoreError> {
        self.client.machine_caps(name)
    }

    /// v2: what a chat runs with, for the flags drawer.
    pub fn flags_view(&self, machine: &str, sid: &str) -> Result<api::ApiFlagsView, CoreError> {
        self.client.flags_view(machine, sid)
    }

    /// v2: the changes from what it runs with to `want`, and how each applies.
    pub fn flags_plan(
        &self,
        machine: &str,
        sid: &str,
        want: &serde_json::Value,
    ) -> Result<api::ApiFlagsPlan, CoreError> {
        self.client.flags_plan(machine, sid, want)
    }

    /// v2: restarts the chat with these flags (`body` from the plan); `when` after a need "busy" ("idle" or "now").
    pub fn relaunch(
        &self,
        machine: &str,
        sid: &str,
        body: &serde_json::Value,
        when: Option<&str>,
    ) -> Result<api::ApiRelaunch, CoreError> {
        let mut b = serde_json::json!({"machine": machine, "sid": sid});
        if let Some(o) = body.as_object() {
            for k in ["model", "effort", "flags"] {
                if let Some(v) = o.get(k) {
                    b[k] = v.clone();
                }
            }
        }
        if let Some(w) = when {
            b["when"] = serde_json::json!(w);
        }
        self.client.relaunch(&b)
    }

    /// The usage panel: this week by day, the 5-hour window and week, and past weeks (GET /api/usage).
    pub fn usage(&self) -> Result<api::ApiUsage, CoreError> {
        self.client.usage()
    }

    /// Changes the usage budget: {op: "set", ...}, {op: "lift"} or {op: "unlift"}.
    pub fn budget_op(&self, body: &serde_json::Value) -> Result<api::ApiBudget, CoreError> {
        self.client.budget_op(body)
    }

    /// Library: every tool, the defaults, and each machine's state.
    pub fn library(&self) -> Result<api::ApiLibrary, CoreError> {
        self.client.library()
    }

    /// Library: the skills and MCP servers `machine` could add to the library.
    pub fn library_sources(&self, machine: &str) -> Result<api::ApiLibrarySources, CoreError> {
        self.client.library_sources(machine)
    }

    /// Library: packs a skill or MCP server (`kind` "skill" or "mcp") on `machine` and stores it as a tool's next
    /// version. `tool` None: named after the source.
    pub fn library_import(
        &self,
        machine: &str,
        kind: &str,
        source: &str,
        tool: Option<&str>,
        description: Option<&str>,
    ) -> Result<api::ApiToolStored, CoreError> {
        let mut b = serde_json::json!({"machine": machine, "kind": kind, "source": source});
        if let Some(t) = tool.filter(|t| !t.is_empty()) {
            b["tool"] = serde_json::json!(t);
        }
        if let Some(d) = description.filter(|d| !d.is_empty()) {
            b["description"] = serde_json::json!(d);
        }
        self.client.library_import(&b)
    }

    /// Library: puts the tool's latest version on `machine` (on) or takes it off.
    pub fn library_connect(&self, tool: &str, machine: &str, on: bool) -> Result<(), CoreError> {
        self.client.library_post("connect", &serde_json::json!({"tool": tool, "machine": machine, "on": on}))
    }

    /// Library: on for every new chat, or not.
    pub fn library_defaults(&self, tool: &str, on: bool) -> Result<(), CoreError> {
        self.client.library_post("defaults", &serde_json::json!({"tool": tool, "on": on}))
    }

    /// Library: makes an older version the latest again (and pushes it to connected machines).
    /// Puts a tool in Auto or takes it out, with its "When to use" line: POST /api/library/update.
    pub fn library_auto(&self, tool: &str, auto: bool, when: &str) -> Result<(), CoreError> {
        self.client.library_post("update", &serde_json::json!({"tool": tool, "auto": auto, "when": when}))
    }

    pub fn library_rollback(&self, tool: &str, version: u32) -> Result<(), CoreError> {
        self.client.library_post("rollback", &serde_json::json!({"tool": tool, "version": version}))
    }

    /// Library: removes the tool from every machine, the defaults and every chat, and deletes it.
    pub fn library_delete(&self, tool: &str) -> Result<(), CoreError> {
        self.client.library_post("delete", &serde_json::json!({"tool": tool}))
    }

    /// Library: starts a chat on `machine` that builds `tool` from `brief` and publishes it. Returns the job.
    pub fn library_new(&self, machine: &str, tool: &str, brief: &str) -> Result<Option<String>, CoreError> {
        Ok(self.client.library_new(&serde_json::json!({"machine": machine, "tool": tool, "brief": brief}))?.job)
    }

    /// Library: sets a chat's tool list. Restarts nothing; follow with `relaunch` for a running chat.
    pub fn library_attach(&self, machine: &str, sid: &str, tools: &[String]) -> Result<(), CoreError> {
        self.client.library_post("attach", &serde_json::json!({"machine": machine, "sid": sid, "tools": tools}))
    }

    pub fn prefs(&self) -> Result<api::ApiPrefs, CoreError> {
        self.client.prefs()
    }

    pub fn set_prefs(&self, body: &serde_json::Value) -> Result<(), CoreError> {
        self.client.set_prefs(body)
    }

    /// Turns one opt-in switch (server/options.py) on or off: POST /api/prefs {"<key>": true|false}.
    pub fn set_option(&self, key: &str, on: bool) -> Result<(), CoreError> {
        self.client.set_prefs(&api::option_body(key, on))
    }

    pub fn browse(&self, query: &str) -> Result<api::ApiBrowse, CoreError> {
        self.client.browse(query)
    }

    pub fn files(&self, machine: &str, folder: &str, name: Option<&str>) -> Result<api::ApiFiles, CoreError> {
        self.client.files(machine, folder, name)
    }

    pub fn pair(&self) -> Result<api::ApiPair, CoreError> {
        self.client.pair()
    }

    pub fn chat_summary(&self, machine: &str, sid: &str) -> Result<api::ApiChatSummary, CoreError> {
        self.client.chat_summary(machine, sid)
    }

    pub fn compact(&self, machine: &str, sid: &str, confirm: bool) -> Result<api::ApiCompact, CoreError> {
        self.client.compact(machine, sid, confirm)
    }

    pub fn handoff_text(&self, machine: &str, sid: &str) -> Result<String, CoreError> {
        self.client.handoff_text(machine, sid)
    }

    pub fn rename(&self, machine: &str, sid: &str, name: &str) -> Result<(), CoreError> {
        self.client.rename(machine, sid, name)
    }

    /// v2: the live stream (see `net::Client::stream`). Blocks; run it on its own thread.
    pub fn stream(&self, seen: impl FnMut(u64) -> bool) -> Result<(), CoreError> {
        self.client.stream(seen)
    }

    pub fn start_new(&self, req: NewRequest) -> Result<Option<String>, CoreError> {
        Ok(self.client.new_session(&req)?.job)
    }

    pub fn continue_on(&self, req: ContinueRequest) -> Result<Option<String>, CoreError> {
        Ok(self.client.continue_session(&req)?.job)
    }

    pub fn poll_job(&self, id: &str) -> Result<api::ApiJob, CoreError> {
        self.client.job(id)
    }

    pub fn cancel_job(&self, id: &str) -> Result<(), CoreError> {
        self.client.cancel_job(id)
    }

    pub fn setup_fix(&self, machine: &str, fix: &str) -> Result<(), CoreError> {
        self.client.setup_fix(machine, fix)
    }

    /// POST /api/term: a one-use ticket for the chat's terminal, or why there is none (`TermError::Need`).
    pub fn term_ticket(&self, req: &TermRequest) -> Result<TermTicket, TermError> {
        let (status, body, base) = self.client.term(req)?;
        term::parse_ticket(status, &body, &base, req)
    }

    /// Opens a ticket's WebSocket at the given size. A ticket works once, within a minute. `history`: the hub's
    /// first snapshot brings the scrollback (false when this view kept its own, coming back after a drop).
    pub fn term_connect(&self, ticket: &TermTicket, cols: u16, rows: u16, history: bool) -> term::ws::Conn {
        term::ws::connect(term::ws_url(&ticket.ws, cols, rows, history), self.ua.clone())
    }
}
