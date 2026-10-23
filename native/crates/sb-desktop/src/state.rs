//! App-wide state shared by every window: the snapshot (from the disk cache first, then live), the 10 s refresh,
//! actions and the jobs they start, toasts, settings and needs-you notifications. Network calls run on the
//! background executor; the UI thread only ever swaps in finished results.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use gpui::{App, AppContext, Context, Entity, EventEmitter, Global, SystemNotification};
use sb_core::actions::{self, ActionId};
use sb_core::api::{ApiCleanItem, ApiJob, ApiPrefs, ApiThreadChat};
use sb_core::{
    BusyInfo, ContinueRequest, Core, CoreConfig, CoreError, Job, JobKind, JobState, Lamp, NewRequest, Snapshot, cache,
    fmt, names,
};
use serde::{Deserialize, Serialize};

const REFRESH: Duration = Duration::from_secs(10);
const TOAST_SECS: f64 = 3.8;

/// Opens a link: a Claude Code session in the Claude desktop app when it is installed, anything else in the browser.
/// `shots` builds (driven tests) only log it, so a test never opens a tab or the Claude app.
pub fn open_link(cx: &mut App, url: &str) {
    let url = &crate::claude_app::link_for(url);
    #[cfg(feature = "shots")]
    {
        let _ = cx;
        eprintln!("sb-open {url}");
    }
    #[cfg(not(feature = "shots"))]
    cx.open_url(url);
}

/// Puts text on the clipboard. `shots` builds only log it, so a test never replaces what you copied.
pub fn copy_text(cx: &mut App, text: String) {
    #[cfg(feature = "shots")]
    {
        let _ = cx;
        eprintln!("sb-copy {text}");
    }
    #[cfg(not(feature = "shots"))]
    cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
}
const HIDE_SECS: f64 = 30.0;
/// The open tabs go to the server at least this often, even unchanged (it forgets an app silent for 30 days).
const TABS_RESEND: f64 = 86400.0;

/// The open tabs need sending: never sent this run, changed since, or last sent a day ago.
fn tabs_due(sent: Option<&(Vec<String>, f64)>, tabs: &[String], now: f64) -> bool {
    sent.is_none_or(|(keys, at)| keys != tabs || now - at >= TABS_RESEND)
}

/// Reads the settings file. One that no longer parses keeps every setting that still reads, and the file as it was
/// is copied to settings.json.bad first, so the next save cannot lose what was in it. True when that happened.
fn load_settings(path: &std::path::Path) -> (Settings, bool) {
    let Ok(bytes) = std::fs::read(path) else { return (Settings::default(), false) };
    if let Ok(s) = serde_json::from_slice(&bytes) {
        return (s, false);
    }
    let mut kept = serde_json::Map::new();
    if let Ok(serde_json::Value::Object(all)) = serde_json::from_slice::<serde_json::Value>(&bytes) {
        for (k, v) in all {
            let one = serde_json::Value::Object([(k.clone(), v.clone())].into_iter().collect());
            if serde_json::from_value::<Settings>(one).is_ok() {
                kept.insert(k, v);
            }
        }
    }
    let mut bad = path.as_os_str().to_owned();
    bad.push(".bad");
    if let Err(e) = std::fs::copy(path, &bad) {
        log::warn!("could not keep a copy of the settings that did not read: {e}");
    }
    (serde_json::from_value(serde_json::Value::Object(kept)).unwrap_or_default(), true)
}

/// The chat a notification's tag stands for: a needs-you one is the chat's key, a job's ("job:<id>") the chat the
/// job made, else the one it came from. None when that chat is not in the list any more.
fn tag_chat(tag: &str, jobs: &[Job], known: impl Fn(&str) -> bool) -> Option<String> {
    let key = match tag.strip_prefix("job:") {
        Some(id) => {
            let j = jobs.iter().find(|j| j.id == id)?;
            match (&j.new_machine, &j.new_sid) {
                (Some(m), Some(s)) if known(&format!("{m}:{s}")) => format!("{m}:{s}"),
                _ => j.key.clone()?,
            }
        }
        None => tag.to_string(),
    };
    known(&key).then_some(key)
}

/// A refresh that left before a change made here showed has the old value of what changed.
fn overtaken(edits_then: u64, edits_now: u64) -> bool {
    edits_then != edits_now
}

/// The stops of chat `key` that closing its tab queued and that have not started yet.
fn stops_from_close(jobs: &[Job], from_close: &HashSet<String>, key: &str) -> Vec<String> {
    jobs.iter().filter(|j| from_close.contains(&j.id) && j.stop_waits_for(key)).map(|j| j.id.clone()).collect()
}

/// Follows a new job, unless one with its id is followed already. Says whether it was added.
fn add_job(jobs: &mut Vec<Job>, job: Job) -> bool {
    if jobs.iter().any(|j| j.id == job.id) {
        return false;
    }
    jobs.push(job);
    true
}

/// A relaunch left the queue (it waited for the chat's reply to end): only now is the old chat going away, so only
/// now does its terminal let go of it.
fn relaunch_started(kind: JobKind, was: JobState, now: JobState) -> bool {
    kind == JobKind::Relaunch && was == JobState::Queued && matches!(now, JobState::Running | JobState::Done)
}

pub struct Shared(pub Entity<AppState>);
impl Global for Shared {}

pub fn app(cx: &App) -> Entity<AppState> {
    cx.global::<Shared>().0.clone()
}

/// The shells on this computer, for local terminal tabs. Found once (main.rs starts it on a thread at launch).
pub fn profiles() -> &'static [sb_core::term::shells::Profile] {
    static P: std::sync::OnceLock<Vec<sb_core::term::shells::Profile>> = std::sync::OnceLock::new();
    P.get_or_init(sb_core::term::shells::detect)
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Settings {
    /// Setup notices ignored from the Notices tray ("machine:id"): gone until the problem changes.
    #[serde(default)]
    pub setup_seen: Vec<String>,
    /// Setup notices closed with their X: out of the corner, still in the Notices tray.
    #[serde(default)]
    pub setup_parked: Vec<String>,
    /// The Notices tray: toasts and job cards that left the corner, newest first.
    #[serde(default)]
    pub notices: Vec<Notice>,
    /// "dark" | "light" | absent (follow the system).
    #[serde(default)]
    pub theme: Option<String>,
    /// The terminal's color scheme id, "match", or absent (the default dark scheme). Synced through /api/prefs; this is
    /// the offline cache.
    #[serde(default)]
    pub term_colors: Option<String>,
    /// The terminal's text size in points (absent: the default), cursor style (block, bar, underline) and blink.
    #[serde(default)]
    pub term_size: Option<u16>,
    #[serde(default)]
    pub term_cursor: Option<String>,
    #[serde(default)]
    pub term_blink: Option<bool>,
    /// The shell Cmd+T opens, by profile id (sb_core::term::shells). Absent: the first one found.
    #[serde(default)]
    pub term_profile: Option<String>,
    /// Folded lanes, by machine name.
    #[serde(default)]
    pub shut: Vec<String>,
    /// Needs-you notifications (on unless turned off).
    #[serde(default)]
    pub notify: Option<bool>,
    /// Machines Settings has already asked about their Claude folder (it asks once per machine).
    #[serde(default)]
    pub root_asked: Vec<String>,
    /// Where the board window was and how big, so it reopens the same way.
    #[serde(default)]
    pub window: Option<SavedWindow>,
    /// The docked details panel's width, as its drag handle left it (absent: the default for the window's width).
    #[serde(default)]
    pub inspector_w: Option<f32>,
    /// The board window's terminal tabs (chat keys, in order) and which one showed (0: the board), reopened on launch.
    #[serde(default)]
    pub tabs: Vec<String>,
    #[serde(default)]
    pub tab: usize,
    /// Tab groups folded on this Mac (group ids). Groups themselves are shared through the server; folding is not.
    #[serde(default)]
    pub folded_groups: Vec<String>,
    /// The computer that runs Switchboard: its addresses ("http://192.0.2.10:8130"), typed and learned from the
    /// server, as `servers` with `server_notes` beside it (sb_core::servers). Empty until one answers or the user
    /// types one; until then the app looks on the local network.
    #[serde(flatten)]
    pub saved: sb_core::servers::Saved,
}

/// The board window's last frame in screen points, and whether it was zoomed to fill the screen.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct SavedWindow {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    #[serde(default)]
    pub maximized: bool,
}

#[derive(Debug, Clone)]
pub struct Toast {
    pub id: u64,
    pub title: String,
    pub sub: Option<String>,
    pub bad: bool,
    pub until: f64,
    /// It did what was asked (a check), rather than a word of information.
    pub ok: bool,
    /// A button on it.
    pub act: Option<NoticeAct>,
}

/// What a toast's (or a notice's) button does.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum NoticeAct {
    /// Resume the chat it just stopped.
    Resume { machine: String, sid: String, title: Option<String> },
    /// Open the terminal of the chat a job started ("machine:sid").
    Terminal { key: String },
    /// Open a link (a saved handoff).
    Link { url: String },
}

impl NoticeAct {
    pub fn label(&self) -> &'static str {
        match self {
            NoticeAct::Resume { .. } => "Resume",
            NoticeAct::Terminal { .. } => "Open terminal",
            NoticeAct::Link { .. } => "Open",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Tone {
    Ok,
    Info,
    Bad,
}

/// A toast or job card that left the corner (closed, or its time ran out): kept in the Notices tray until Ignore.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Notice {
    pub id: u64,
    pub at: f64,
    pub title: String,
    #[serde(default)]
    pub sub: Option<String>,
    pub tone: Tone,
    #[serde(default)]
    pub act: Option<NoticeAct>,
    /// Not looked at in the tray yet.
    #[serde(default)]
    pub new: bool,
}

/// How many notices the tray keeps (the oldest go first).
const NOTICES_KEEP: usize = 80;

/// Whether a toast's title says something was done (it gets a check).
fn toast_ok(title: &str) -> bool {
    const DONE: [&str; 16] = [
        "Stopped",
        "Fixed",
        "Copied",
        "Saved",
        "Sent",
        "Kept",
        "Dismissed",
        "Renamed",
        "Moved",
        "Resumed",
        "Started",
        "Done",
        "Updated",
        "Added",
        "Removed",
        "Starred",
    ];
    DONE.iter().any(|w| title.starts_with(w))
}

/// Something a window should do: select a chat (a job finished, a notification was clicked), or a job a terminal tab
/// waits on ended (Resume and open, New session with "Open its terminal").
#[derive(Debug, Clone)]
pub enum AppEvent {
    Select(String),
    /// v2, from the session browser: open a chat's terminal in the board window (`resume` when it is stopped).
    OpenTerm {
        key: String,
        resume: bool,
    },
    /// v2, from the session browser: a chat resumed by id; its terminal follows the job.
    OpenJobTerm {
        job: String,
        machine: String,
        label: String,
        title: String,
    },
    /// v2, from Files: a new session in that folder.
    NewIn {
        machine: String,
        folder: String,
    },
    JobDone {
        id: String,
        machine: Option<String>,
        sid: Option<String>,
        bg: Option<String>,
    },
    JobFailed {
        id: String,
        error: String,
    },
    /// The synced look changed (from the server, or Settings): every window applies it.
    Appearance,
    /// Open the flags drawer for this chat in the board window (from a terminal in its own window).
    Flags(String),
    /// From Files: open the chat's terminal and paste this (a file's path there) into it.
    PasteInto {
        key: String,
        text: String,
    },
    /// v2, the flags drawer: a relaunch was sent. It may wait for the chat's reply to end, so its terminal stays.
    Relaunching {
        key: String,
    },
    /// The relaunch left the queue: the chat's terminal follows the job now.
    RelaunchStarted {
        key: String,
        job: String,
    },
    /// v2, the flags drawer: the chat is open in a terminal Switchboard does not own; run this there instead.
    RelaunchCmd {
        key: String,
        command: String,
        error: String,
    },
}

/// v2: a chat's thread as the inspector's History shows it, fetched when the chat is shown and again after a minute
/// or when its count of earlier chats changes.
#[derive(Debug, Clone, Default)]
pub struct ThreadView {
    pub at: f64,
    pub history: u32,
    /// Oldest first, the chat itself included (`this`).
    pub chats: Vec<ApiThreadChat>,
    pub loading: bool,
    pub failed: Option<String>,
}

impl ThreadView {
    /// The chat this one carries on from now, if any.
    pub fn parent(&self) -> Option<&ApiThreadChat> {
        let me = self.chats.iter().position(|c| c.this == Some(true))?;
        me.checked_sub(1).and_then(|i| self.chats.get(i))
    }
}

/// The question shown when the server answers need "busy": what is running, and what to send again once the user
/// picks "when they finish" (the default) or "now".
pub struct BusyAsk {
    /// "Stop it", "Start fresh", "Close it", "Move it", "Stop".
    pub what: &'static str,
    pub message: String,
    pub busy: Vec<BusyInfo>,
    pub retry: BusyRetry,
}

pub enum BusyRetry {
    Act {
        id: ActionId,
        key: String,
    },
    Clean(Vec<ApiCleanItem>),
    Continue {
        key: String,
        to: String,
    },
    /// v2, the flags drawer: relaunch with this body (the plan's {model, effort, flags}).
    Relaunch {
        key: String,
        body: serde_json::Value,
    },
}

/// The question shown when the server answers need "trust": Claude was never let into the folder a chat would move
/// to. Yes sends the same request again with `trust: true`.
pub struct TrustAsk {
    /// The folder as people read it ("~/Documents/site").
    pub shown: String,
    /// The user once said no to it in Claude's own prompt.
    pub declined: bool,
    /// The machine it is on, by its label.
    pub on: String,
    pub retry: TrustRetry,
}

pub enum TrustRetry {
    Continue { key: String, to: String, when: Option<&'static str> },
}

impl BusyAsk {
    pub fn only_reply(&self) -> bool {
        self.busy.len() == 1 && self.busy[0].only_reply()
    }
}

pub struct AppState {
    pub core: Arc<Core>,
    pub snap: Snapshot,
    /// True once live data arrived in this run (before that, the board shows the saved snapshot).
    pub live: bool,
    /// When the data on screen was fetched (epoch seconds, 0 when there is none).
    pub synced: f64,
    /// The last refresh failed with this.
    pub error: Option<String>,
    /// Dismissed here, hidden until the server agrees (or 30 s pass).
    pub hidden: HashMap<String, f64>,
    pub jobs: Vec<Job>,
    pub toasts: Vec<Toast>,
    /// The toast under the pointer: it stays while it is read.
    pub toast_hover: Option<u64>,
    pub settings: Settings,
    /// Actions in flight, "key:verb", so their buttons can say so.
    pub busy: HashSet<String>,
    /// Chats opened again while the stop from closing their tab was still on its way: the stop is called off once
    /// the server answers with its job.
    stop_off: HashSet<String>,
    /// Stops sent because a chat's tab closed: by key while the request is out, then by job id while the job waits.
    /// Only these are called off when the chat opens again.
    close_stop: HashSet<String>,
    close_jobs: HashSet<String>,
    /// Stars changed here and not yet seen in a refresh: key -> (on, when). A refresh that left before the change
    /// would otherwise flip the star back for a moment.
    pub star_want: std::collections::HashMap<String, (bool, f64)>,
    /// Tab groups changed here and not yet seen in a refresh: the groups to show, and when. Laid over each refresh
    /// until it agrees (or 15 s pass), so a poll that left before the change cannot undo it on screen.
    pub groups_want: Option<(Vec<sb_core::groups::Group>, f64)>,
    /// Group changes sent and not yet answered (only the last answer replaces what shows).
    pub groups_pending: u32,
    /// A budget change is on its way to the server.
    pub budget_busy: bool,
    /// The groups the last refresh brought, what a failed change goes back to.
    pub groups_server: Vec<sb_core::groups::Group>,
    /// A group made here before the server named it: its stand-in id -> the server's id, and changes to it that
    /// wait for that answer.
    pub group_ids: HashMap<String, String>,
    pub group_waiting: Vec<sb_core::groups::Op>,
    /// v2, the cleanup card: rows being stopped or kept, and rows put off with "Not now" (until the list changes).
    pub clean_busy: HashSet<String>,
    /// The saved tabs the server last took and when (None: not yet this run), and whether a send is out. A chat open
    /// in a tab is never offered for stopping, even while this app is closed.
    tabs_sent: Option<(Vec<String>, f64)>,
    tabs_sending: bool,
    /// A relaunch, stop, move or fresh start that would cut off a chat's own work: asked before anything happens.
    pub busy_ask: Option<BusyAsk>,
    /// A move to a folder Claude was never let into: asked before it is sent again.
    pub trust_ask: Option<TrustAsk>,
    pub clean_later: HashSet<String>,
    /// v2: the New sheet's memory (last used, presets) and the flags the server passes; fetched when the sheet opens.
    pub prefs: Option<ApiPrefs>,
    /// The models the machines' Claude Code takes (GET /api/models), read with the prefs; None from an older server.
    pub models: Option<Vec<sb_core::api::ApiModel>>,
    /// v2: the session browser's window, while it is open.
    pub browser_win: Option<gpui::AnyWindowHandle>,
    pub files_win: Option<gpui::AnyWindowHandle>,
    pub pair_win: Option<gpui::AnyWindowHandle>,
    pub settings_win: Option<gpui::AnyWindowHandle>,
    pub about_win: Option<gpui::AnyWindowHandle>,
    /// The usage panel's numbers (GET /api/usage), read each time the Week chip's panel opens; whether a read is
    /// under way, and why the last one failed.
    pub usage: Option<sb_core::api::ApiUsage>,
    pub usage_loading: bool,
    pub usage_error: Option<String>,
    /// v2: threads by chat key, for History and "This continues...".
    pub threads: HashMap<String, ThreadView>,
    needs: Option<HashSet<String>>,
    /// The waiting spell each chat was last notified for (when it last did something): a chat that flickers out of
    /// "needs you" and back, or is held again with nothing new, is not notified twice.
    notified: HashMap<String, i64>,
    inflight: bool,
    again: bool,
    /// Bumped by each change made here that the snapshot shows at once (a rename, Keep running, the budget): a
    /// refresh that was already on its way has the old value, so it is read again instead.
    edits: u64,
    seq: u64,
    settings_path: Option<PathBuf>,
    /// The minute last drawn, so ages like "14m" move on without a redraw every second.
    minute: i64,
    /// When /api/prefs was last read (the synced look is checked every half minute).
    prefs_at: f64,
    /// A newer build on the download page, the download under way, when it was last looked for (None: not since
    /// launch, which was at `update_started`) and whether a look is under way.
    pub update: Option<sb_core::update::Offer>,
    pub upd: crate::update::Upd,
    pub update_at: Option<f64>,
    /// Why the last look failed, until a look works again.
    pub update_err: Option<String>,
    pub update_started: f64,
    pub update_busy: bool,
    /// The tool library (Settings > Library, the New sheet's Tools, a chat's Tools), once asked for.
    pub library: Option<Result<sb_core::api::ApiLibrary, String>>,
    /// Bumped per read of the library, and the newest read shown: an older read that comes back later is dropped.
    pub lib_seq: u64,
    pub lib_shown: u64,
    /// Library changes in flight, by what they change ("connect:tool@machine", "default:tool", "attach:key").
    pub lib_busy: HashSet<String>,
    /// Keys saved on this Mac from Settings ("tool/NAME", names only), shown as set until the server's next look agrees.
    pub lib_keys_here: HashSet<String>,
}

impl EventEmitter<AppEvent> for AppState {}

/// The card's id for a row: "machine/sid", or "machine/pid:123" for a leftover process.
/// From a terminal in a window of its own: the board window opens the flags drawer.
pub fn open_flags_in_board(key: String, cx: &mut App) {
    app(cx).update(cx, |_, cx| cx.emit(AppEvent::Flags(key)));
}

fn remember_os(snap: &Snapshot) {
    crate::ui::set_machine_os(
        snap.machines.iter().map(|m| (m.name.clone(), m.os.clone().unwrap_or_default())).collect(),
    );
}

pub fn clean_key(d: &ApiCleanItem) -> String {
    match (&d.sid, d.pid) {
        (Some(sid), _) => format!("{}/{sid}", d.machine),
        (None, pid) => format!("{}/pid:{}", d.machine, pid.unwrap_or(0)),
    }
}

impl AppState {
    /// Builds the state from the disk cache (no network), installs it as a global and starts the refresh and tick loops.
    pub fn init(cx: &mut App) -> Entity<AppState> {
        let settings_path = match std::env::var_os("SB_SETTINGS") {
            Some(p) => Some(std::path::PathBuf::from(p)),
            None => cache::settings_dir().map(|d| d.join("settings.json")),
        };
        let (mut settings, settings_bad) = settings_path.as_deref().map(load_settings).unwrap_or_default();
        let cache_dir = std::env::var("SWITCHBOARD_CACHE").ok().filter(|s| !s.is_empty());
        // Where the server is: SWITCHBOARD_URL, else the saved addresses (the last one that worked first), else
        // nothing (Core looks on the local network, and the board asks for an address if that finds none).
        let mut servers: Vec<String> = std::env::var("SWITCHBOARD_URL")
            .ok()
            .map(|v| {
                v.split(',').map(|s| s.trim().trim_end_matches('/').to_string()).filter(|s| !s.is_empty()).collect()
            })
            .unwrap_or_default();
        if servers.is_empty() {
            // An upgrade from a build that had the address built in saved no list: seed it from the server that
            // answered last time and the addresses it listed, so this start connects at once, at home or away.
            let dir = cache_dir.as_deref().map(PathBuf::from).or_else(cache::default_dir);
            if let Some(dir) = dir
                && settings.saved.migrate_from_cache(&dir, Self::now())
                && let Some(p) = &settings_path
                && let Err(e) = cache::save_json(p, &settings)
            {
                log::warn!("could not save settings: {e}");
            }
            servers = settings.saved.connect_order();
        }
        let core = Arc::new(Core::new(CoreConfig { servers, cache_dir, user_agent: None }));
        // With nothing saved, the empty board runs the search itself and lets the user pick (firstrun.rs).
        core.set_discovery(false);
        let (snap, synced) = match core.cached_snapshot() {
            Some(s) => {
                let at = s.fetched_at;
                (s, at)
            }
            None => (Snapshot::empty(), 0.0),
        };
        remember_os(&snap);
        let entity = cx.new(|cx| {
            let mut s = AppState {
                core,
                snap,
                live: false,
                synced,
                error: None,
                hidden: HashMap::new(),
                jobs: Vec::new(),
                toasts: Vec::new(),
                toast_hover: None,
                settings,
                busy: HashSet::new(),
                stop_off: HashSet::new(),
                close_stop: HashSet::new(),
                close_jobs: HashSet::new(),
                star_want: Default::default(),
                groups_want: None,
                groups_pending: 0,
                budget_busy: false,
                groups_server: Vec::new(),
                group_ids: HashMap::new(),
                group_waiting: Vec::new(),
                clean_busy: HashSet::new(),
                tabs_sent: None,
                tabs_sending: false,
                busy_ask: None,
                trust_ask: None,
                clean_later: HashSet::new(),
                prefs: None,
                models: None,
                browser_win: None,
                files_win: None,
                pair_win: None,
                settings_win: None,
                about_win: None,
                usage: None,
                usage_loading: false,
                usage_error: None,
                threads: HashMap::new(),
                needs: None,
                notified: HashMap::new(),
                inflight: false,
                again: false,
                edits: 0,
                seq: 0,
                settings_path,
                minute: 0,
                prefs_at: 0.0,
                update: None,
                upd: Default::default(),
                update_at: None,
                update_err: None,
                update_started: AppState::now(),
                update_busy: false,
                library: None,
                lib_seq: 0,
                lib_shown: 0,
                lib_busy: HashSet::new(),
                lib_keys_here: HashSet::new(),
            };
            if settings_bad {
                s.toast(
                    "Some settings couldn't be read",
                    Some("The rest were kept. The old file is saved next to it as settings.json.bad.".into()),
                    true,
                    cx,
                );
            }
            s.refresh(cx);
            s.listen(cx);
            s.load_prefs(|_, _| {}, cx);
            s.load_library(cx);
            s.update_hooks(cx);
            cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor().timer(REFRESH).await;
                    if this.update(cx, |s, cx| s.refresh(cx)).is_err() {
                        break;
                    }
                }
            })
            .detach();
            cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor().timer(Duration::from_secs(1)).await;
                    if this.update(cx, |s, cx| s.tick(cx)).is_err() {
                        break;
                    }
                }
            })
            .detach();
            s
        });
        cx.set_global(Shared(entity.clone()));
        entity
    }

    pub fn now() -> f64 {
        fmt::now_epoch()
    }

    /// The link to the server is up: live data came in this run and the last refresh worked.
    pub fn link_up(&self) -> bool {
        self.live && self.error.is_none()
    }

    /// A chat's live state as every view shows it (sb_core::live). `last_out`: a terminal's own last output.
    pub fn live_of(&self, s: &sb_core::Session, now: f64, last_out: Option<f64>) -> sb_core::live::Live {
        sb_core::live::live(s, now, self.link_up(), last_out)
    }

    // ---------- data ----------

    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        if self.inflight {
            self.again = true;
            return;
        }
        self.inflight = true;
        let core = self.core.clone();
        let edits = self.edits;
        cx.spawn(async move |this, cx| {
            let res = cx.background_executor().spawn(async move { core.refresh() }).await;
            let _ = this.update(cx, |s, cx| {
                s.inflight = false;
                if res.is_ok() && overtaken(edits, s.edits) {
                    s.again = true;
                } else {
                    s.take(res, cx);
                }
                if std::mem::take(&mut s.again) {
                    s.refresh(cx);
                }
            });
        })
        .detach();
    }

    /// v2: the server's live stream. A change loads the board at once; the ten second poll stays as the fallback.
    fn listen(&mut self, cx: &mut Context<Self>) {
        let (tx, rx) = smol::channel::unbounded::<u64>();
        let core = self.core.clone();
        let spawned = std::thread::Builder::new().name("sb-live".into()).spawn(move || {
            let mut wait = 2u64;
            loop {
                let began = std::time::Instant::now();
                let res = core.stream(|v| tx.send_blocking(v).is_ok());
                if tx.is_closed() {
                    break;
                }
                if let Err(e) = res {
                    log::info!("live stream: {e}");
                }
                // A stream that ran a while ended on its own (the server closes it every ten minutes): back at once.
                wait = if began.elapsed() > Duration::from_secs(30) { 1 } else { (wait * 2).min(30) };
                std::thread::sleep(Duration::from_secs(wait));
            }
        });
        if let Err(e) = spawned {
            log::warn!("no live stream: {e}");
            return;
        }
        let timing = std::env::var_os("SB_TIMING").is_some();
        cx.spawn(async move |this, cx| {
            while let Ok(v) = rx.recv().await {
                if timing {
                    eprintln!("sb-timing live v={v} epoch={:.3}", fmt::now_epoch());
                }
                let gone = this
                    .update(cx, |s, cx| {
                        if v == 0 || s.snap.v != Some(v) {
                            s.refresh(cx);
                        }
                    })
                    .is_err();
                if gone {
                    break;
                }
            }
        })
        .detach();
    }

    /// Refreshes after `delay` (after an action, so the board catches up without waiting for the next poll).
    pub fn refresh_in(&mut self, delay: Duration, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(delay).await;
            let _ = this.update(cx, |s, cx| s.refresh(cx));
        })
        .detach();
    }

    fn take(&mut self, res: Result<Snapshot, CoreError>, cx: &mut Context<Self>) {
        let now = Self::now();
        match res {
            Ok(snap) => {
                // Nothing new: keep the frame on screen (the tick still moves ages and countdowns along).
                let same = self.live && self.error.is_none() && self.snap.same_board(&snap);
                let new_machine = self.snap.machines.len() != snap.machines.len()
                    || snap.machines.iter().any(|m| !self.snap.machines.iter().any(|o| o.name == m.name));
                self.snap = snap;
                self.apply_stars(now);
                self.groups_server = self.snap.groups.clone();
                self.apply_groups(now);
                remember_os(&self.snap);
                if new_machine {
                    self.ask_roots(cx);
                }
                self.live = true;
                self.send_tabs(cx);
                if now - self.prefs_at > 30.0 {
                    self.prefs_at = now;
                    self.load_prefs(|_, _| {}, cx);
                }
                self.check_update(cx);
                self.synced = now;
                self.error = None;
                self.keep_server();
                let snap = &self.snap;
                let hidden = self.hidden.len();
                self.hidden.retain(|k, t| {
                    now - *t < HIDE_SECS && (snap.session(k).is_some() || snap.replaced_row(k).is_some())
                });
                if same && hidden == self.hidden.len() {
                    return;
                }
                self.notify_needs(cx);
            }
            Err(e) => {
                log::info!("refresh failed: {e}");
                self.error = Some(e.to_string());
            }
        }
        cx.notify();
    }

    /// After a good refresh: the addresses the server listed join the saved list, the one in use is noted as working
    /// (one found on the local network is saved, so the next start goes straight to it), and probes that got no
    /// answer are noted as failing (sb_core::servers). With SWITCHBOARD_URL set the run names its own server: the
    /// list then only learns when the run has its own settings file too (SB_SETTINGS), and is not used.
    fn keep_server(&mut self) {
        let env = std::env::var_os("SWITCHBOARD_URL").is_some();
        if env && std::env::var_os("SB_SETTINGS").is_none() {
            return;
        }
        let now = Self::now();
        let saved = &mut self.settings.saved;
        let mut changed = saved.learn(&self.snap.addresses, now);
        for (url, ok) in self.core.take_probe_outcomes() {
            changed |= if ok { saved.answered(&url) } else { saved.failed(&url, now) };
        }
        if !env && let Some(base) = self.core.server() {
            changed |= saved.worked(&base, now);
        }
        changed |= saved.prune(now);
        if changed {
            self.save_settings();
            self.use_saved_order();
        }
    }

    /// Hands the saved list to Core in the order to try it (the last one that worked first). The server in use stays
    /// in use while it is still on the list.
    fn use_saved_order(&self) {
        if std::env::var_os("SWITCHBOARD_URL").is_some() {
            return;
        }
        let order = self.settings.saved.connect_order();
        if order != self.core.servers() {
            self.core.set_servers(order);
        }
    }

    /// Settings > Machines: the user's Remove on a saved address.
    pub fn remove_server(&mut self, url: &str, cx: &mut Context<Self>) {
        if self.settings.saved.remove(url) {
            self.save_settings();
            self.use_saved_order();
            cx.notify();
        }
    }

    /// The address the user typed on the board's first-run screen: saved and tried at once. Err says why it is not
    /// an address.
    pub fn use_server(&mut self, typed: &str, cx: &mut Context<Self>) -> Result<(), String> {
        let Some(base) = sb_core::net::normalize_base(typed) else {
            return Err("That's not an address. Try something like 192.0.2.10:8130 or studio.local.".into());
        };
        self.settings.saved.add_typed(&base);
        self.save_settings();
        self.core.set_servers(self.settings.saved.connect_order());
        self.error = None;
        self.refresh(cx);
        cx.notify();
        Ok(())
    }

    /// The first run, or a machine just added: when a machine's Claude folder was never picked, Settings opens at it
    /// and asks (once per machine).
    fn ask_roots(&mut self, cx: &mut Context<Self>) {
        let core = self.core.clone();
        cx.spawn(async move |this, cx| {
            let Ok(list) = cx.background_executor().spawn(async move { core.machines() }).await else { return };
            let _ = this.update(cx, |s, cx| {
                let Some(m) = list
                    .machines
                    .iter()
                    .find(|m| m.needs_root && m.online == Some(true) && !s.settings.root_asked.contains(&m.name))
                else {
                    return;
                };
                s.settings.root_asked.push(m.name.clone());
                s.save_settings();
                let name = m.name.clone();
                cx.defer(move |cx| crate::settings::open_machine(name, cx));
            });
        })
        .detach();
    }

    /// Once a second: drops expired toasts and jobs, and redraws only when something on screen counts in seconds
    /// (a warm chat's countdown, a running job, a stale "Updated" time) or a minute has passed.
    fn tick(&mut self, cx: &mut Context<Self>) {
        let now = Self::now();
        let (nt, nj) = (self.toasts.len(), self.jobs.len());
        if let Some(t) = self.toast_hover.and_then(|h| self.toasts.iter_mut().find(|t| t.id == h)) {
            t.until = t.until.max(now + 1.5);
        }
        // What leaves the corner by itself goes to the Notices tray, where it can be looked at again.
        let (gone, keep): (Vec<Toast>, Vec<Toast>) =
            std::mem::take(&mut self.toasts).into_iter().partition(|t| t.until <= now);
        self.toasts = keep;
        // A finished job stays on screen for 9 s, one called off for 5, an error until it is clicked away (or a minute).
        let (done, keep): (Vec<Job>, Vec<Job>) =
            std::mem::take(&mut self.jobs).into_iter().partition(|j| match (j.state, j.ended) {
                (JobState::Done, Some(e)) => now - e >= 9.0,
                (JobState::Error, Some(e)) if j.cancelled => now - e >= 5.0,
                (JobState::Error, Some(e)) => now - e >= 60.0,
                _ => false,
            });
        self.jobs = keep;
        if !gone.is_empty() || !done.is_empty() {
            for t in gone {
                self.park_toast(t);
            }
            for j in &done {
                self.park_job(j);
            }
            self.save_settings();
        }
        let minute = (now / 60.0).floor() as i64;
        let since = now - self.synced;
        let seconds = !self.jobs.is_empty()
            || (self.synced > 0.0 && (15.0..90.0).contains(&since))
            || self.snap.sessions().iter().any(|s| !self.is_hidden(&s.key) && s.lamp(now) == Lamp::Warm);
        if seconds || minute != self.minute || nt != self.toasts.len() || nj != self.jobs.len() {
            self.minute = minute;
            cx.notify();
        }
    }

    pub fn is_hidden(&self, key: &str) -> bool {
        self.hidden.contains_key(key)
    }

    pub fn hidden_set(&self) -> HashSet<String> {
        self.hidden.keys().cloned().collect()
    }

    /// Idle two days or more, not hidden.
    pub fn older(&self) -> Vec<&sb_core::Session> {
        self.snap.sessions().into_iter().filter(|s| s.stale && !self.is_hidden(&s.key)).collect()
    }

    pub fn replaced(&self) -> Vec<&sb_core::Replaced> {
        self.snap.replaced().into_iter().filter(|r| !self.is_hidden(&r.key)).collect()
    }

    /// The running job that came from `key`, if any.
    pub fn job_for(&self, key: &str) -> Option<&Job> {
        self.jobs.iter().find(|j| j.key.as_deref() == Some(key) && !j.state.finished())
    }

    // ---------- settings ----------

    /// Where the New session draft is kept: beside the settings.
    pub fn draft_path(&self) -> Option<PathBuf> {
        self.settings_path.as_deref().and_then(|p| p.parent()).map(|d| d.join("new-draft.json"))
    }

    pub fn save_settings(&self) {
        if let Some(p) = &self.settings_path
            && let Err(e) = cache::save_json(p, &self.settings)
        {
            log::warn!("could not save settings: {e}");
        }
    }

    pub fn is_shut(&self, machine: &str) -> bool {
        self.settings.shut.iter().any(|m| m == machine)
    }

    pub fn set_shut(&mut self, machine: &str, shut: bool, cx: &mut Context<Self>) {
        self.settings.shut.retain(|m| m != machine);
        if shut {
            self.settings.shut.push(machine.to_string());
        }
        self.save_settings();
        cx.notify();
    }

    pub fn notify_on(&self) -> bool {
        self.settings.notify != Some(false)
    }

    // ---------- toasts ----------

    pub fn toast(&mut self, title: impl Into<String>, sub: Option<String>, bad: bool, cx: &mut Context<Self>) {
        self.toast_with(title, sub, bad, None, cx)
    }

    /// A toast with a button. It stays long enough to read (about 15 characters a second), longer for an error or a
    /// button, and while the pointer is on it. The same message again refreshes the one on screen.
    pub fn toast_with(
        &mut self,
        title: impl Into<String>,
        sub: Option<String>,
        bad: bool,
        act: Option<NoticeAct>,
        cx: &mut Context<Self>,
    ) {
        let title = title.into();
        let now = Self::now();
        let chars = title.chars().count() + sub.as_ref().map_or(0, |s| s.chars().count());
        let mut secs = (2.5 + chars as f64 / 15.0).clamp(TOAST_SECS, 9.0);
        if bad {
            secs = secs.max(8.0);
        }
        if act.is_some() {
            secs += 3.0;
        }
        if let Some(t) = self.toasts.iter_mut().find(|t| t.title == title && t.sub == sub && t.bad == bad) {
            t.until = now + secs;
            t.act = act;
            cx.notify();
            return;
        }
        self.seq += 1;
        let ok = !bad && toast_ok(&title);
        self.toasts.push(Toast { id: self.seq, title, sub, bad, until: now + secs, ok, act });
        if self.toasts.len() > 3 {
            let n = self.toasts.len() - 3;
            let gone: Vec<Toast> = self.toasts.drain(..n).collect();
            for t in gone {
                self.park_toast(t);
            }
            self.save_settings();
        }
        cx.notify();
    }

    /// Closes a toast (its X, or a click on it): out of the corner, into the Notices tray.
    pub fn drop_toast(&mut self, id: u64, cx: &mut Context<Self>) {
        if let Some(t) = self.take_toast(id) {
            self.park_toast(t);
            self.save_settings();
        }
        cx.notify();
    }

    fn take_toast(&mut self, id: u64) -> Option<Toast> {
        if self.toast_hover == Some(id) {
            self.toast_hover = None;
        }
        let i = self.toasts.iter().position(|t| t.id == id)?;
        Some(self.toasts.remove(i))
    }

    // ---------- the Notices tray ----------

    fn notice_id(&mut self) -> u64 {
        self.seq += 1;
        (Self::now() * 1000.0) as u64 * 100 + self.seq % 100
    }

    fn park(&mut self, n: Notice) {
        self.settings.notices.insert(0, n);
        self.settings.notices.truncate(NOTICES_KEEP);
    }

    fn park_toast(&mut self, t: Toast) {
        let tone = if t.bad {
            Tone::Bad
        } else if t.ok {
            Tone::Ok
        } else {
            Tone::Info
        };
        let id = self.notice_id();
        self.park(Notice { id, at: Self::now(), title: t.title, sub: t.sub, tone, act: t.act, new: true });
    }

    fn park_job(&mut self, j: &Job) {
        let now = Self::now();
        let done = j.state == JobState::Done;
        let tone = if j.cancelled {
            Tone::Info
        } else if done {
            Tone::Ok
        } else {
            Tone::Bad
        };
        let act = if !done {
            None
        } else if j.kind == JobKind::Stop {
            j.key.as_deref().and_then(|k| k.split_once(':')).map(|(m, s)| NoticeAct::Resume {
                machine: m.into(),
                sid: s.into(),
                title: j.from.clone(),
            })
        } else if let (Some(m), Some(s), false) = (&j.new_machine, &j.new_sid, j.kind == JobKind::Handoff) {
            Some(NoticeAct::Terminal { key: format!("{m}:{s}") })
        } else {
            j.url.clone().map(|url| NoticeAct::Link { url })
        };
        let id = self.notice_id();
        self.park(Notice {
            id,
            at: j.ended.unwrap_or(now),
            title: j.headline(),
            sub: Some(j.detail(now)),
            tone,
            act,
            new: true,
        });
    }

    /// Setup problems closed with their X that are still there and not ignored.
    pub fn setup_parked(&self) -> Vec<(String, String, sb_core::api::ApiSetupIssue)> {
        self.snap
            .machines
            .iter()
            .filter(|m| m.online == Some(true))
            .flat_map(|m| m.setup.iter().map(move |i| (m.name.clone(), m.label.clone(), i.clone())))
            .filter(|(m, _, i)| {
                let k = format!("{m}:{}", i.id);
                self.settings.setup_parked.contains(&k) && !self.settings.setup_seen.contains(&k)
            })
            .collect()
    }

    /// (everything in the tray, not looked at yet).
    pub fn tray_count(&self) -> (usize, usize) {
        let parked = self.setup_parked().len();
        (self.settings.notices.len() + parked, self.settings.notices.iter().filter(|n| n.new).count())
    }

    /// The tray was opened: nothing in it is new any more.
    pub fn tray_seen(&mut self, cx: &mut Context<Self>) {
        if self.settings.notices.iter().any(|n| n.new) {
            for n in &mut self.settings.notices {
                n.new = false;
            }
            self.save_settings();
            cx.notify();
        }
    }

    /// Ignore, confirmed: gone from the tray for good.
    pub fn ignore_notice(&mut self, id: u64, cx: &mut Context<Self>) {
        self.settings.notices.retain(|n| n.id != id);
        self.save_settings();
        cx.notify();
    }

    /// Clear all, confirmed: every notice and parked setup problem is ignored.
    pub fn clear_tray(&mut self, cx: &mut Context<Self>) {
        self.settings.notices.clear();
        for (m, _, i) in self.setup_parked() {
            self.settings.setup_seen.push(format!("{m}:{}", i.id));
        }
        self.settings.setup_parked.clear();
        self.save_settings();
        cx.notify();
    }

    /// A notice's button: it does what it says, and the notice has done its job.
    pub fn notice_act(&mut self, id: u64, cx: &mut Context<Self>) -> Option<NoticeAct> {
        let i = self.settings.notices.iter().position(|n| n.id == id)?;
        let act = self.settings.notices[i].act.clone()?;
        self.settings.notices.remove(i);
        self.save_settings();
        match &act {
            NoticeAct::Resume { machine, sid, title } => self.resume_chat(machine, sid, title.clone(), cx),
            NoticeAct::Link { url } => open_link(cx, url),
            NoticeAct::Terminal { .. } => {} // the window opens it
        }
        cx.notify();
        Some(act)
    }

    pub fn hover_toast(&mut self, id: u64, on: bool) {
        if on {
            self.toast_hover = Some(id);
        } else if self.toast_hover == Some(id) {
            self.toast_hover = None;
        }
    }

    /// A toast's button: done, so it does not go to the tray.
    pub fn toast_act(&mut self, id: u64, cx: &mut Context<Self>) {
        let Some(act) = self.take_toast(id).and_then(|t| t.act) else { return };
        match act {
            NoticeAct::Resume { machine, sid, title } => self.resume_chat(&machine, &sid, title, cx),
            NoticeAct::Link { url } => open_link(cx, &url),
            NoticeAct::Terminal { key } => cx.emit(AppEvent::Select(key)),
        }
        cx.notify();
    }

    /// Closes a finished job card. `park`: it goes to the Notices tray (its X); not when its button was used.
    pub fn drop_job(&mut self, id: &str, park: bool, cx: &mut Context<Self>) {
        if let Some(i) = self.jobs.iter().position(|j| j.id == id && j.state.finished()) {
            let j = self.jobs.remove(i);
            if park {
                self.park_job(&j);
                self.save_settings();
            }
        }
        cx.notify();
    }

    /// Setup problems not yet closed: (machine, its label, the problem).
    pub fn setup_open(&self) -> Vec<(String, String, sb_core::api::ApiSetupIssue)> {
        self.snap
            .machines
            .iter()
            .filter(|m| m.online == Some(true))
            .flat_map(|m| m.setup.iter().map(move |i| (m.name.clone(), m.label.clone(), i.clone())))
            .filter(|(m, _, i)| {
                let k = format!("{m}:{}", i.id);
                !self.settings.setup_seen.contains(&k) && !self.settings.setup_parked.contains(&k)
            })
            .collect()
    }

    /// A setup notice's X: out of the corner, into the Notices tray.
    pub fn park_setup(&mut self, machine: &str, id: &str, cx: &mut Context<Self>) {
        let key = format!("{machine}:{id}");
        if !self.settings.setup_parked.contains(&key) {
            self.settings.setup_parked.push(key);
            if self.settings.setup_parked.len() > 200 {
                self.settings.setup_parked.drain(..50);
            }
            self.save_settings();
        }
        cx.notify();
    }

    /// Ignored (from the tray) or fixed: gone until the problem changes.
    pub fn close_setup(&mut self, machine: &str, id: &str, cx: &mut Context<Self>) {
        let key = format!("{machine}:{id}");
        if !self.settings.setup_seen.contains(&key) {
            self.settings.setup_seen.push(key);
            if self.settings.setup_seen.len() > 200 {
                self.settings.setup_seen.drain(..50);
            }
            self.save_settings();
        }
        cx.notify();
    }

    pub fn fix_setup(&mut self, machine: &str, id: &str, fix: &str, cx: &mut Context<Self>) {
        let (core, m, f, id) = (self.core.clone(), machine.to_string(), fix.to_string(), id.to_string());
        self.busy.insert(format!("setup:{m}:{id}"));
        cx.notify();
        cx.spawn(async move |this, cx| {
            let m2 = m.clone();
            let res = cx.background_executor().spawn(async move { core.setup_fix(&m2, &f) }).await;
            let _ = this.update(cx, |s, cx| {
                s.busy.remove(&format!("setup:{m}:{id}"));
                match res {
                    Ok(()) => {
                        s.close_setup(&m, &id, cx);
                        s.toast("Fixed. New chats there get those tools", None, false, cx);
                    }
                    Err(e) => s.toast("Couldn't fix it", Some(e.to_string()), true, cx),
                }
            });
        })
        .detach();
    }

    /// Stops the chat at `key` because its tab closed (`working`: once its reply is done). Only a stop sent this way
    /// is called off when the chat opens again; one asked for on the board stays.
    pub fn stop_on_close(&mut self, key: &str, working: bool, cx: &mut Context<Self>) {
        let Some(busy) = ActionId::Stop.verb().map(|v| format!("{key}:{v}")) else { return };
        if self.busy.contains(&busy) {
            return; // a stop is on its way already
        }
        self.close_stop.insert(key.to_string());
        self.act_when(ActionId::Stop, key, working.then_some("idle"), cx);
        if !self.busy.contains(&busy) {
            self.close_stop.remove(key); // not sent (the chat left the list)
        }
    }

    /// The chat at `key` is open again: a stop still waiting for its reply to end, from closing its tab, is called
    /// off, so it does not end the chat just reopened.
    pub fn call_off_stops(&mut self, key: &str, cx: &mut Context<Self>) {
        if self.close_stop.contains(key) {
            self.stop_off.insert(key.to_string());
        }
        for id in stops_from_close(&self.jobs, &self.close_jobs, key) {
            self.cancel_job(&id, cx);
        }
    }

    /// Calls off a job still waiting for its chat's work to end (Cancel, Keep running). Nothing has changed yet, so
    /// its card says so at once and goes away by itself.
    pub fn cancel_job(&mut self, id: &str, cx: &mut Context<Self>) {
        let busy = format!("job-off:{id}");
        if !self.busy.insert(busy.clone()) {
            return;
        }
        cx.notify();
        let (core, id) = (self.core.clone(), id.to_string());
        cx.spawn(async move |this, cx| {
            let jid = id.clone();
            let res = cx.background_executor().spawn(async move { core.cancel_job(&jid) }).await;
            let _ = this.update(cx, |s, cx| {
                s.busy.remove(&busy);
                match res {
                    Ok(()) => {
                        if let Some(j) = s.jobs.iter_mut().find(|j| j.id == id) {
                            j.call_off(Self::now());
                        }
                        s.refresh_in(Duration::from_millis(400), cx);
                    }
                    Err(e) => s.toast("It already started", Some(e.to_string()), false, cx),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// "Stop now" or "Start now" on a job still waiting for its chat's work: called off, then done at once, which
    /// cuts the work off.
    pub fn job_now(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(j) = self.jobs.iter().find(|j| j.id == id) else { return };
        let act = match j.kind {
            JobKind::Stop => ActionId::Stop,
            JobKind::Fresh => ActionId::Fresh,
            _ => return,
        };
        let Some(key) = j.key.clone() else { return };
        let busy = format!("job-off:{id}");
        if !self.busy.insert(busy.clone()) {
            return;
        }
        cx.notify();
        let (core, id) = (self.core.clone(), id.to_string());
        cx.spawn(async move |this, cx| {
            let jid = id.clone();
            let res = cx.background_executor().spawn(async move { core.cancel_job(&jid) }).await;
            let _ = this.update(cx, |s, cx| {
                s.busy.remove(&busy);
                match res {
                    Ok(()) => {
                        s.jobs.retain(|j| j.id != id);
                        s.act_when(act, &key, Some("now"), cx);
                    }
                    Err(e) => s.toast("It already started", Some(e.to_string()), false, cx),
                }
                cx.notify();
            });
        })
        .detach();
    }

    // ---------- notifications ----------

    /// Posts a notification when a chat starts needing you, and takes it back when it stops. Never on the first
    /// load (that would replay everything), never while a Switchboard window is in front, and once per waiting
    /// spell: a chat that drops out of "needs you" for a poll and comes back has nothing new to say.
    fn notify_needs(&mut self, cx: &mut Context<Self>) {
        let now = Self::now();
        let current: HashSet<String> = self
            .snap
            .sessions()
            .into_iter()
            // Parked chats (waiting more than a day) neither count nor notify.
            .filter(|s| !s.stale && !self.is_hidden(&s.key) && s.needs_you(now))
            .map(|s| s.key.clone())
            .collect();
        let spell = |s: &sb_core::Session| s.active.round() as i64;
        let Some(before) = self.needs.replace(current.clone()) else {
            // The first load is not news: what waits now counts as told.
            self.notified =
                current.iter().filter_map(|k| self.snap.session(k).map(|s| (k.clone(), spell(s)))).collect();
            return;
        };
        for gone in before.difference(&current) {
            cx.dismiss_system_notification(gone);
        }
        let snap = &self.snap;
        self.notified.retain(|k, _| snap.session(k).is_some());
        let mut new = Vec::new();
        for key in current.difference(&before) {
            let Some(s) = self.snap.session(key) else { continue };
            if self.notified.insert(key.clone(), spell(s)) != Some(spell(s)) {
                new.push(key.clone()); // not the spell it was already told about
            }
        }
        if !self.notify_on() || cx.active_window().is_some() {
            return;
        }
        for key in &new {
            let Some(s) = self.snap.session(key) else { continue };
            cx.show_system_notification(SystemNotification {
                tag: key.clone().into(),
                title: s.topic.clone().into(),
                body: format!("{}, on {}", s.phrase(now), s.machine_label).into(),
                actions: vec![],
            });
        }
    }

    /// Selects a chat in every window: a key, or a notification's tag. One for a chat that is gone leaves the
    /// selection as it is.
    pub fn select(&mut self, key: String, cx: &mut Context<Self>) {
        let snap = &self.snap;
        let known = |k: &str| snap.session(k).is_some() || snap.replaced_row(k).is_some();
        if let Some(key) = tag_chat(&key, &self.jobs, known) {
            cx.emit(AppEvent::Select(key));
        }
    }

    // ---------- actions ----------

    /// Tells the server which chats this app has open in tabs (the saved list, so it holds while the app is closed),
    /// when that changed since it last took it, and once a day anyway: the server forgets an app it has not heard
    /// from in 30 days. A failed send goes again on the next refresh.
    fn send_tabs(&mut self, cx: &mut Context<Self>) {
        let tabs = self.settings.tabs.clone();
        let now = Self::now();
        if self.tabs_sending || !tabs_due(self.tabs_sent.as_ref(), &tabs, now) {
            return;
        }
        self.tabs_sending = true;
        let device = crate::library::this_mac().unwrap_or("desktop").to_string();
        let sent = tabs.clone();
        self.call(
            cx,
            move |core| core.report_tabs(&device, &tabs),
            move |st, res, _| {
                st.tabs_sending = false;
                match res {
                    Ok(()) => st.tabs_sent = Some((sent, now)),
                    Err(e) => log::info!("sending tabs failed: {e}"),
                }
            },
        );
    }

    /// Runs `work` against the core off the UI thread, then `done` back on it.
    pub(crate) fn call<R: Send + 'static>(
        &mut self,
        cx: &mut Context<Self>,
        work: impl FnOnce(&Core) -> Result<R, CoreError> + Send + 'static,
        done: impl FnOnce(&mut AppState, Result<R, CoreError>, &mut Context<AppState>) + 'static,
    ) {
        let core = self.core.clone();
        cx.spawn(async move |this, cx| {
            let res = cx.background_executor().spawn(async move { work(&core) }).await;
            let _ = this.update(cx, |s, cx| done(s, res, cx));
        })
        .detach();
    }

    /// Open in Claude (Enter): the chat's link, or a word on why there is none.
    pub fn open(&mut self, key: &str, cx: &mut Context<Self>) {
        if let Some(s) = self.snap.session(key) {
            match &s.url {
                Some(u) => open_link(cx, u),
                None => {
                    let sub = if s.alive && !s.background() {
                        format!("It's open in the terminal on {}.", s.machine_label)
                    } else if let (true, Some(id)) = (s.alive, &s.bg_id) {
                        format!(
                            "It runs in the background on {}. Run claude attach {id} there to watch it.",
                            s.machine_label
                        )
                    } else {
                        "Resume it to get one.".to_string()
                    };
                    let title = format!("No Claude link for {}", s.topic);
                    self.toast(title, Some(sub), false, cx);
                }
            }
        } else if let Some(r) = self.snap.replaced_row(key)
            && let Some(u) = &r.url
        {
            open_link(cx, u);
        }
    }

    /// Every action except Continue (needs a machine), Newer (a selection) and Start fresh's confirm (the window asks).
    pub fn act(&mut self, id: ActionId, key: &str, cx: &mut Context<Self>) {
        self.act_when(id, key, None, cx)
    }

    /// The user's answer to a need "busy": "idle" (queued until the work ends), "now", or None (cancel).
    pub fn answer_busy(&mut self, when: Option<&'static str>, cx: &mut Context<Self>) {
        let Some(ask) = self.busy_ask.take() else { return };
        cx.notify();
        let Some(w) = when else { return };
        match ask.retry {
            BusyRetry::Act { id, key } => self.act_when(id, &key, Some(w), cx),
            BusyRetry::Clean(items) => self.clean_stop_when(items, Some(w), cx),
            BusyRetry::Continue { key, to } => self.continue_when(&key, &to, Some(w), cx),
            BusyRetry::Relaunch { key, body } => self.relaunch(&key, body, Some(w), cx),
        }
    }

    fn ask_busy(
        &mut self,
        what: &'static str,
        message: String,
        busy: Vec<BusyInfo>,
        retry: BusyRetry,
        cx: &mut Context<Self>,
    ) {
        self.busy_ask = Some(BusyAsk { what, message, busy, retry });
        cx.notify();
    }

    pub fn act_when(&mut self, id: ActionId, key: &str, when: Option<&'static str>, cx: &mut Context<Self>) {
        let now = Self::now();
        match id {
            ActionId::Open => return self.open(key, cx),
            ActionId::ArchiveInClaude => {
                if let Some(u) = self.snap.replaced_row(key).and_then(|r| r.url.clone()) {
                    open_link(cx, &u);
                }
                return;
            }
            ActionId::Attach => {
                if let Some(id) = self.snap.session(key).and_then(|s| s.bg_id.clone()) {
                    copy_text(cx, format!("claude attach {id}"));
                    self.toast("Copied. Run it in a terminal on that machine.", None, false, cx);
                }
                return;
            }
            ActionId::Newer | ActionId::Continue | ActionId::Link | ActionId::Rename => return,
            ActionId::Keep => return self.keep(key, cx),
            ActionId::Unlink => return self.unlink(key, false, cx),
            ActionId::Split => return self.unlink(key, true, cx),
            _ => {}
        }
        let Some(verb) = id.verb() else { return };
        // The row can be a current chat or a replaced one (Resume as is, Dismiss).
        let (machine, sid, title, label, bg_alive) = if let Some(s) = self.snap.session(key) {
            (s.machine.clone(), s.sid.clone(), s.title.clone(), s.machine_label.clone(), s.background() && s.alive)
        } else if let Some(r) = self.snap.replaced_row(key) {
            (r.machine.clone(), r.sid.clone(), r.title.clone(), r.machine_label.clone(), r.background() && r.alive)
        } else {
            return;
        };
        let busy = format!("{key}:{verb}");
        if !self.busy.insert(busy.clone()) {
            return;
        }
        cx.notify();
        let close = if id == ActionId::Handoff { Some(bg_alive) } else { None };
        if id == ActionId::Dismiss {
            self.hidden.insert(key.to_string(), now);
        }
        let key = key.to_string();
        let (m2, s2, v2) = (machine.clone(), sid.clone(), verb.to_string());
        self.call(
            cx,
            move |core| core.act_when(&m2, &v2, &s2, close, when),
            move |st, res, cx| {
                st.busy.remove(&busy);
                let off = id == ActionId::Stop && st.stop_off.remove(&key);
                let closing = id == ActionId::Stop && st.close_stop.remove(&key);
                match res {
                    Err(CoreError::Busy { message, busy }) => {
                        if id == ActionId::Dismiss {
                            st.hidden.remove(&key);
                        }
                        let what = match id {
                            ActionId::Stop => "Stop it",
                            ActionId::Fresh => "Start fresh",
                            ActionId::Handoff => "Close it",
                            _ => "Go ahead",
                        };
                        st.ask_busy(what, message, busy, BusyRetry::Act { id, key }, cx);
                    }
                    // Queued until its work ends: a card follows it, with Stop now and Keep running.
                    Ok(Some(jid)) if id == ActionId::Stop => {
                        st.start_job(&jid, JobKind::Stop, Some(key), &label, &label, Some(title), None, cx);
                        if off {
                            st.cancel_job(&jid, cx);
                        } else if closing {
                            st.close_jobs.insert(jid);
                        }
                    }
                    Ok(job) if id.is_job() => {
                        let (topic, tag) = names::split(&title);
                        let (kind, name) = match id {
                            ActionId::Fresh => {
                                (JobKind::Fresh, Some(names::fresh(&topic, names::fresh_n(tag.as_deref()))))
                            }
                            ActionId::Resume => (JobKind::Resume, Some(names::resumed(&topic))),
                            _ => (JobKind::Handoff, None),
                        };
                        match job {
                            Some(jid) => st.start_job(&jid, kind, Some(key), &label, &label, Some(title), name, cx),
                            None => st.toast(format!("{} was sent", kind.label()), None, false, cx),
                        }
                    }
                    Ok(_) => {
                        let msg = match id {
                            ActionId::Pass => format!("Send it again on {label}. It goes through this time."),
                            ActionId::Stop => format!("Stopped {}", names::split(&title).0),
                            _ => "Dismissed".into(),
                        };
                        let sub = match id {
                            ActionId::Dismiss => Some("It comes back when Claude replies there again.".to_string()),
                            ActionId::Stop => Some(format!("On {label}. Its memory is free again.")),
                            _ => None,
                        };
                        let act = (id == ActionId::Stop).then(|| NoticeAct::Resume {
                            machine: machine.clone(),
                            sid: sid.clone(),
                            title: Some(title.clone()),
                        });
                        st.toast_with(msg, sub, false, act, cx);
                        st.refresh_in(Duration::from_millis(600), cx);
                    }
                    Err(e) => {
                        if id == ActionId::Dismiss {
                            st.hidden.remove(&key);
                        }
                        st.toast(e.to_string(), None, true, cx);
                    }
                }
                cx.notify();
            },
        );
    }

    // ---------- v2: the cleanup card, prefs, rename ----------

    /// Stops what the card lists (one row or all of them). Only ever on a click.
    pub fn clean_stop(&mut self, items: Vec<ApiCleanItem>, cx: &mut Context<Self>) {
        self.clean_stop_when(items, None, cx)
    }

    pub fn clean_stop_when(&mut self, items: Vec<ApiCleanItem>, when: Option<&'static str>, cx: &mut Context<Self>) {
        let again = items.clone();
        let info = items.clone();
        let keys: Vec<String> = items.iter().map(clean_key).filter(|k| !self.clean_busy.contains(k)).collect();
        if keys.is_empty() {
            return;
        }
        self.clean_busy.extend(keys.iter().cloned());
        cx.notify();
        let (n, mb) = (items.len(), items.iter().filter_map(|d| d.mb).sum::<f64>());
        self.call(
            cx,
            move |core| core.cleanup_stop(&items, when),
            move |st, res, cx| {
                for k in &keys {
                    st.clean_busy.remove(k);
                }
                match res {
                    Err(CoreError::Busy { message, busy }) => {
                        st.ask_busy("Stop", message, busy, BusyRetry::Clean(again), cx);
                    }
                    // What the server did not stop (in use again, kept, or Claude Code could not stop it) is still
                    // running: say so instead of "Stopped".
                    Ok(r) if !r.not_stopped.is_empty() => {
                        for (k, jid) in &r.queued {
                            let Some(d) = info
                                .iter()
                                .find(|d| d.sid.as_ref().is_some_and(|s| format!("{}/{s}", d.machine) == *k))
                            else {
                                continue;
                            };
                            let label = d.label.clone().unwrap_or_else(|| d.machine.clone());
                            let key = d.sid.as_ref().map(|s| format!("{}:{s}", d.machine));
                            st.start_job(jid, JobKind::Stop, key, &label, &label, d.title.clone(), None, cx);
                        }
                        let left = r.not_stopped.len();
                        let why =
                            r.not_stopped.iter().map(|(_, e)| e.as_str()).find(|e| !e.is_empty()).map(str::to_string);
                        let title = if left < n {
                            format!("Stopped {}, {left} still running", n - left)
                        } else if n == 1 {
                            "Didn't stop it".into()
                        } else {
                            format!("Didn't stop {n}")
                        };
                        st.toast(title, Some(why.unwrap_or_else(|| "In use again, or kept.".into())), true, cx)
                    }
                    // Those still working wait for it, each with its own card (Stop now, Keep running).
                    Ok(r) if !r.queued.is_empty() => {
                        let queued = r.queued;
                        for (k, jid) in &queued {
                            let Some(d) = info
                                .iter()
                                .find(|d| d.sid.as_ref().is_some_and(|s| format!("{}/{s}", d.machine) == *k))
                            else {
                                continue;
                            };
                            let label = d.label.clone().unwrap_or_else(|| d.machine.clone());
                            let key = d.sid.as_ref().map(|s| format!("{}:{s}", d.machine));
                            st.start_job(jid, JobKind::Stop, key, &label, &label, d.title.clone(), None, cx);
                        }
                        if queued.len() < n {
                            let left = n - queued.len();
                            st.toast(
                                format!("Stopped {left}"),
                                Some("The rest stop when their work is done.".into()),
                                false,
                                cx,
                            )
                        }
                    }
                    Ok(_) => {
                        let one = (n == 1).then(|| info[0].clone());
                        let title = match one.as_ref().and_then(|d| d.title.as_deref()) {
                            Some(t) => format!("Stopped {}", names::split(t).0),
                            None if n == 1 => "Stopped".to_string(),
                            None => format!("Stopped {n}"),
                        };
                        let sub = if mb > 0.0 {
                            format!("About {} freed. Resume any time today.", fmt::mem(mb))
                        } else {
                            "Resume any time today.".into()
                        };
                        let act = one.and_then(|d| {
                            d.sid.clone().map(|sid| NoticeAct::Resume {
                                machine: d.machine.clone(),
                                sid,
                                title: d.title.clone(),
                            })
                        });
                        st.toast_with(title, Some(sub), false, act, cx)
                    }
                    Err(e) => st.toast(e.to_string(), None, true, cx),
                }
                st.refresh_in(Duration::from_millis(400), cx);
                cx.notify();
            },
        );
    }

    /// "Keep": leave it running; the card never suggests it again.
    pub fn clean_keep(&mut self, item: ApiCleanItem, cx: &mut Context<Self>) {
        let k = clean_key(&item);
        if !self.clean_busy.insert(k.clone()) {
            return;
        }
        cx.notify();
        self.call(
            cx,
            move |core| core.cleanup_keep(&item),
            move |st, res, cx| {
                st.clean_busy.remove(&k);
                if let Err(e) = res {
                    st.toast(e.to_string(), None, true, cx);
                }
                st.refresh_in(Duration::from_millis(300), cx);
                cx.notify();
            },
        );
    }

    /// "Not now" folds the card to one line until something new joins it; "Show" opens it again.
    pub fn clean_fold(&mut self, keys: Vec<String>, fold: bool, cx: &mut Context<Self>) {
        if fold {
            self.clean_later.extend(keys);
        } else {
            for k in &keys {
                self.clean_later.remove(k);
            }
        }
        cx.notify();
    }

    /// Resume a chat by its id: the board's own action when it is on the board, else a resume job from here.
    pub fn resume_chat(&mut self, machine: &str, sid: &str, title: Option<String>, cx: &mut Context<Self>) {
        let key = format!("{machine}:{sid}");
        if self.snap.session(&key).is_some() || self.snap.replaced_row(&key).is_some() {
            return self.act(ActionId::Resume, &key, cx);
        }
        let busy = format!("{key}:resume");
        if !self.busy.insert(busy.clone()) {
            return;
        }
        cx.notify();
        let label = self.snap.label_of(machine).unwrap_or_else(|| machine.to_string());
        let (m, s) = (machine.to_string(), sid.to_string());
        self.call(
            cx,
            move |core| core.act(&m, "resume", &s, None),
            move |st, res, cx| {
                st.busy.remove(&busy);
                match res {
                    Ok(Some(jid)) => {
                        let name = title.as_deref().map(|t| names::resumed(&names::split(t).0));
                        st.start_job(&jid, JobKind::Resume, Some(key), &label, &label, title, name, cx)
                    }
                    Ok(None) => st.toast(format!("Resuming on {label}"), None, false, cx),
                    Err(e) => st.toast(e.to_string(), None, true, cx),
                }
                cx.notify();
            },
        );
    }

    /// v2, History: Start fresh from any chat, on the board or not (a handoff, then a new chat from it).
    pub fn fresh_chat(&mut self, machine: &str, sid: &str, title: Option<String>, cx: &mut Context<Self>) {
        let key = format!("{machine}:{sid}");
        if self.snap.session(&key).is_some() {
            return self.act(ActionId::Fresh, &key, cx);
        }
        let busy = format!("{key}:fresh");
        if !self.busy.insert(busy.clone()) {
            return;
        }
        cx.notify();
        let label = self.snap.label_of(machine).unwrap_or_else(|| machine.to_string());
        let (m, s) = (machine.to_string(), sid.to_string());
        self.call(
            cx,
            move |core| core.act(&m, "fresh", &s, None),
            move |st, res, cx| {
                st.busy.remove(&busy);
                match res {
                    Ok(Some(jid)) => {
                        let topic = title.as_deref().map(|t| names::split(t).0.to_string());
                        st.start_job(&jid, JobKind::Fresh, Some(key), &label, &label, title, topic, cx)
                    }
                    Ok(None) => st.toast(format!("Starting fresh on {label}"), None, false, cx),
                    Err(CoreError::Busy { message, busy }) => {
                        st.ask_busy("Start fresh", message, busy, BusyRetry::Act { id: ActionId::Fresh, key }, cx)
                    }
                    Err(e) => st.toast(e.to_string(), None, true, cx),
                }
                cx.notify();
            },
        );
    }

    /// Compacts a History thread into one fresh chat, after the user saw the cost (POST /api/compact, confirm).
    pub fn compact_thread(
        &mut self,
        machine: &str,
        sid: &str,
        title: Option<String>,
        label: String,
        cx: &mut Context<Self>,
    ) {
        let key = format!("{machine}:{sid}");
        let busy = format!("{key}:compact");
        if !self.busy.insert(busy.clone()) {
            return;
        }
        cx.notify();
        let (m, s) = (machine.to_string(), sid.to_string());
        self.call(
            cx,
            move |core| core.compact(&m, &s, true),
            move |st, res, cx| {
                st.busy.remove(&busy);
                match res {
                    Ok(r) => match r.job {
                        Some(jid) => {
                            let topic = title.as_deref().map(|t| names::split(t).0.to_string());
                            st.start_job(&jid, JobKind::Compact, Some(key), &label, &label, title, topic, cx)
                        }
                        None => st.toast("Compacting didn't start. Try again in a moment.", None, true, cx),
                    },
                    Err(e) => st.toast(e.to_string(), None, true, cx),
                }
                cx.notify();
            },
        );
    }

    /// Fetches the New sheet's prefs; `done` hears them (or the copy from before, when the server cannot be reached).
    /// Reads the usage panel's numbers. The last ones stay on screen while it reads, so reopening never flashes empty.
    pub fn load_usage(&mut self, cx: &mut Context<Self>) {
        if self.usage_loading {
            return;
        }
        self.usage_loading = true;
        cx.notify();
        self.call(
            cx,
            |core| core.usage(),
            |st, res, cx| {
                st.usage_loading = false;
                match res {
                    Ok(u) => {
                        st.usage = Some(u);
                        st.usage_error = None;
                    }
                    Err(e) => {
                        log::info!("usage: {e}");
                        st.usage_error = Some(e.to_string());
                    }
                }
                cx.notify();
            },
        );
    }

    /// The model picker: Default and what the server says the machines take, or the built-in list.
    pub fn model_choices(&self) -> Vec<(String, String)> {
        sb_core::actions::model_choices(self.models.as_deref())
    }

    pub fn load_prefs(
        &mut self,
        done: impl FnOnce(&mut AppState, &mut Context<AppState>) + 'static,
        cx: &mut Context<Self>,
    ) {
        self.call(
            cx,
            |core| core.prefs().map(|p| (p, core.models())),
            move |st, res, cx| {
                match res {
                    Ok((p, models)) => {
                        if models.is_some() {
                            st.models = models;
                        }
                        st.prefs_at = Self::now();
                        if p.appearance.as_ref().is_some_and(|a| st.take_appearance(a)) {
                            cx.emit(AppEvent::Appearance);
                            cx.refresh_windows();
                        }
                        st.prefs = Some(p);
                    }
                    Err(e) => log::info!("prefs: {e}"),
                }
                done(st, cx);
                cx.notify();
            },
        );
    }

    /// The look as /api/prefs carries it, from the local settings.
    pub fn appearance_body(&self) -> serde_json::Value {
        let st = &self.settings;
        serde_json::json!({"appearance": {
            "theme": st.theme.clone().unwrap_or_else(|| "system".into()),
            "term": {
                "scheme": sb_core::term::scheme::normalize(st.term_colors.as_deref()),
                "size": st.term_size,
                "cursor": st.term_cursor.clone().unwrap_or_else(|| "block".into()),
                "blink": st.term_blink.unwrap_or(true),
            },
        }})
    }

    /// Sends the look to the server, so every device agrees (the local settings stay as the offline cache).
    pub fn sync_appearance(&mut self, cx: &mut Context<Self>) {
        let body = self.appearance_body();
        self.call(
            cx,
            move |core| core.set_prefs(&body),
            |_, res, _| {
                if let Err(e) = res {
                    log::info!("appearance not synced: {e}");
                }
            },
        );
    }

    /// Takes the server's look when it differs from the cache. True when anything changed.
    pub fn take_appearance(&mut self, a: &sb_core::term::scheme::Appearance) -> bool {
        let before = (
            self.settings.theme.clone(),
            self.settings.term_colors.clone(),
            self.settings.term_size,
            self.settings.term_cursor.clone(),
            self.settings.term_blink,
        );
        // A shots run picks its theme with SB_THEME; the server's never overrides it there.
        if let Some(th) = a.theme.as_deref().filter(|_| std::env::var_os("SB_THEME").is_none()) {
            self.settings.theme = match th {
                "light" | "dark" => Some(th.to_string()),
                _ => None,
            };
        }
        if let Some(t) = &a.term {
            self.settings.term_colors = Some(sb_core::term::scheme::normalize(Some(&t.scheme)));
            self.settings.term_size = t.size;
            self.settings.term_cursor = Some(t.cursor.clone());
            self.settings.term_blink = Some(t.blink);
        }
        let after = (
            self.settings.theme.clone(),
            self.settings.term_colors.clone(),
            self.settings.term_size,
            self.settings.term_cursor.clone(),
            self.settings.term_blink,
        );
        if before != after {
            self.save_settings();
            true
        } else {
            false
        }
    }

    /// Saves a change to the prefs (the Chrome machine, the presets), then loads them again.
    pub fn save_prefs(&mut self, body: serde_json::Value, ok: Option<String>, cx: &mut Context<Self>) {
        self.call(
            cx,
            move |core| core.set_prefs(&body),
            move |st, res, cx| {
                match res {
                    Ok(()) => {
                        if let Some(t) = ok {
                            st.toast(t, None, false, cx);
                        }
                        st.load_prefs(|_, _| {}, cx);
                        st.refresh_in(Duration::from_millis(200), cx);
                    }
                    Err(e) => st.toast(e.to_string(), None, true, cx),
                }
                cx.notify();
            },
        );
    }

    /// One chat's own Chrome machine (None clears it). Only that chat: there is no Chrome machine for all.
    pub fn set_chat_chrome(
        &mut self,
        machine: String,
        sid: String,
        chrome: Option<String>,
        ok: String,
        cx: &mut Context<Self>,
    ) {
        self.call(
            cx,
            move |core| core.set_chrome(&machine, &sid, chrome.as_deref()),
            move |st, res, cx| {
                match res {
                    Ok(()) => {
                        st.toast(ok, None, false, cx);
                        st.refresh_in(Duration::from_millis(200), cx);
                    }
                    Err(e) => st.toast(e.to_string(), None, true, cx),
                }
                cx.notify();
            },
        );
    }

    /// Renames a chat everywhere (and in a stopped chat's transcript); `done` hears whether it worked.
    /// An empty name clears it: the chat goes back to its automatic name. A new name shows at once everywhere.
    pub fn rename(
        &mut self,
        machine: String,
        sid: String,
        name: String,
        done: impl FnOnce(bool, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        if !name.is_empty() && self.snap.retitle(&format!("{machine}:{sid}"), &name) {
            self.edits += 1;
            cx.notify();
        }
        let (key, shown) = (format!("{machine}:{sid}"), name.clone());
        self.call(
            cx,
            move |core| core.rename(&machine, &sid, &name),
            move |st, res, cx| {
                match res {
                    Ok(()) => {
                        // Shown again, over a refresh that came back meanwhile with the old name.
                        if !shown.is_empty() {
                            st.snap.retitle(&key, &shown);
                        }
                        st.edits += 1;
                        st.refresh_in(Duration::from_millis(600), cx);
                        cx.defer(move |cx| done(true, cx))
                    }
                    Err(e) => {
                        st.toast(e.to_string(), None, true, cx);
                        st.refresh_in(Duration::from_millis(200), cx); // the name shown goes back to what it is
                        cx.defer(move |cx| done(false, cx))
                    }
                }
                cx.notify();
            },
        );
    }

    /// Star or unstar a chat. Shown at once (its tab moves to the starred group), then the server is told; a
    /// failure puts it back.
    pub fn star(&mut self, key: &str, on: bool, cx: &mut Context<Self>) {
        let Some(s) = self.snap.session(key) else { return };
        let (machine, sid) = (s.machine.clone(), s.sid.clone());
        self.star_want.insert(key.to_string(), (on, Self::now()));
        self.apply_stars(Self::now());
        cx.notify();
        let key = key.to_string();
        self.call(
            cx,
            move |core| core.star(&machine, &sid, on),
            move |st, res, cx| {
                match res {
                    Ok(stored) => {
                        st.star_want.insert(key.clone(), (stored, Self::now()));
                        st.apply_stars(Self::now());
                        st.refresh_in(Duration::from_millis(400), cx);
                    }
                    Err(e) => {
                        st.star_want.insert(key.clone(), (!on, Self::now()));
                        st.apply_stars(Self::now());
                        st.toast(e.to_string(), None, true, cx);
                    }
                }
                cx.notify();
            },
        );
    }

    /// A tab group change: shown at once, then sent; the server's answer replaces it, and a failure goes back to
    /// what the last refresh said, with a notice. A change to a group made here that the server has not named yet
    /// waits for that answer. Returns the stand-in id a create gets until the server names the group.
    /// Changes the usage budget (Settings > Budget). The answer stands in the snapshot at once, before the next poll.
    pub fn budget_op(&mut self, body: serde_json::Value, cx: &mut Context<Self>) {
        self.budget_busy = true;
        cx.notify();
        self.call(
            cx,
            move |core| core.budget_op(&body),
            |st, res, cx| {
                st.budget_busy = false;
                match res {
                    Ok(b) => {
                        st.snap.budget = Some(b);
                        st.edits += 1;
                    }
                    Err(e) => st.toast("Couldn't change the budget", Some(e.to_string()), true, cx),
                }
                cx.notify();
            },
        );
    }

    pub fn group_op(&mut self, op: sb_core::groups::Op, cx: &mut Context<Self>) -> String {
        use sb_core::groups::Op;
        let now = Self::now();
        let op = self.real_group_ids(op);
        let mut gs = self.snap.groups.clone();
        self.seq += 1;
        let temp = format!("new-{}", self.seq);
        op.apply(&mut gs, &temp);
        self.groups_want = Some((gs.clone(), now));
        self.snap.regroup(gs);
        cx.notify();
        let names = |o: &Op| match o {
            Op::Update { id, .. } | Op::Delete { id } | Op::Add { id, .. } | Op::Move { id, .. } => Some(id.clone()),
            _ => None,
        };
        if names(&op).is_some_and(|id| id.starts_with("new-")) {
            self.group_waiting.push(op);
            return temp;
        }
        self.groups_pending += 1;
        let created = matches!(op, Op::Create { .. }).then(|| temp.clone());
        let sent = op.clone();
        self.call(
            cx,
            move |core| core.group_op(&sent),
            move |st, res, cx| {
                st.groups_pending = st.groups_pending.saturating_sub(1);
                match res {
                    Ok((server, new_id)) => {
                        if let Some(temp) = created {
                            let waiting = std::mem::take(&mut st.group_waiting);
                            let (mine, rest): (Vec<Op>, Vec<Op>) =
                                waiting.into_iter().partition(|o| names(o).as_deref() == Some(temp.as_str()));
                            st.group_waiting = rest;
                            if let Some(real) = new_id {
                                st.group_ids.insert(temp, real);
                                for o in mine {
                                    st.group_op(o, cx);
                                }
                            }
                        }
                        if st.groups_pending == 0 {
                            st.groups_want = Some((server, Self::now()));
                            st.apply_groups(Self::now());
                        }
                        st.refresh_in(Duration::from_millis(400), cx);
                    }
                    Err(e) => {
                        st.groups_want = None;
                        st.group_waiting.clear();
                        let back = st.groups_server.clone();
                        st.snap.regroup(back);
                        st.toast("The tab group change didn't save", Some(e.to_string()), true, cx);
                        st.refresh_in(Duration::from_millis(200), cx);
                    }
                }
                cx.notify();
            },
        );
        temp
    }

    /// A stand-in id the server has since named, swapped for the server's id.
    fn real_group_ids(&self, op: sb_core::groups::Op) -> sb_core::groups::Op {
        use sb_core::groups::Op;
        let real = |id: String| self.group_ids.get(&id).cloned().unwrap_or(id);
        match op {
            Op::Update { id, name, color } => Op::Update { id: real(id), name, color },
            Op::Delete { id } => Op::Delete { id: real(id) },
            Op::Add { id, thread, index } => Op::Add { id: real(id), thread, index },
            Op::Move { id, index } => Op::Move { id: real(id), index },
            o => o,
        }
    }

    /// The server's id for a group, when `id` is a stand-in it has named since.
    pub fn group_id(&self, id: &str) -> String {
        self.group_ids.get(id).cloned().unwrap_or_else(|| id.to_string())
    }

    /// Lays the groups changed here over the snapshot until a refresh agrees (or 15 s pass).
    fn apply_groups(&mut self, now: f64) {
        let Some((want, at)) = &self.groups_want else { return };
        let settled = self.groups_pending == 0 && ((self.snap.groups == *want && now - *at > 1.0) || now - *at > 15.0);
        if settled {
            self.groups_want = None;
            return;
        }
        let want = want.clone();
        self.snap.regroup(want);
    }

    /// Lays the stars changed here over the snapshot until a refresh agrees (or 15 s pass).
    fn apply_stars(&mut self, now: f64) {
        let snap = &mut self.snap;
        self.star_want.retain(|key, (on, at)| {
            let Some((m, sid)) = key.split_once(':') else { return false };
            let Some(s) = snap
                .machines
                .iter_mut()
                .find(|x| x.name == m)
                .and_then(|x| x.sessions.iter_mut().find(|x| x.sid == sid))
            else {
                return now - *at < 15.0;
            };
            if s.star == *on && now - *at > 1.0 {
                return false;
            }
            s.star = *on;
            now - *at < 15.0
        });
    }

    pub fn keep(&mut self, key: &str, cx: &mut Context<Self>) {
        let Some(s) = self.snap.session(key) else { return };
        let (machine, sid, on) = (s.machine.clone(), s.sid.clone(), !s.keep);
        let busy = format!("{key}:keep");
        if !self.busy.insert(busy.clone()) {
            return;
        }
        cx.notify();
        let key = key.to_string();
        self.call(
            cx,
            move |core| core.keep(&machine, &sid, on),
            move |st, res, cx| {
                st.busy.remove(&busy);
                match res {
                    Ok(kept) => {
                        // Say it at once; the next refresh agrees.
                        if let Some((m, sid)) = key.split_once(':')
                            && let Some(s) = st
                                .snap
                                .machines
                                .iter_mut()
                                .find(|x| x.name == m)
                                .and_then(|x| x.sessions.iter_mut().find(|x| x.sid == sid))
                        {
                            s.keep = kept;
                        }
                        st.edits += 1;
                        let msg = if kept {
                            "Switchboard will keep it running."
                        } else {
                            "Switchboard may stop it again when it sits idle."
                        };
                        st.toast(msg, None, false, cx);
                        st.refresh_in(Duration::from_millis(400), cx);
                    }
                    Err(e) => st.toast(e.to_string(), None, true, cx),
                }
                cx.notify();
            },
        );
    }

    /// The thread of a chat with earlier chats, loaded when it is missing, a minute old, or its count changed.
    pub fn needs_thread(&self, key: &str) -> bool {
        let Some(s) = self.snap.session(key) else { return false };
        match self.threads.get(key) {
            _ if s.history == 0 => false,
            Some(t) => !t.loading && (t.history != s.history || Self::now() - t.at >= 60.0),
            None => true,
        }
    }

    pub fn want_thread(&mut self, key: &str, cx: &mut Context<Self>) {
        if !self.needs_thread(key) {
            return;
        }
        let Some(s) = self.snap.session(key) else { return };
        let (machine, sid, history) = (s.machine.clone(), s.sid.clone(), s.history);
        let e = self.threads.entry(key.to_string()).or_default();
        e.loading = true;
        e.history = history;
        let key = key.to_string();
        self.call(
            cx,
            move |core| core.thread(&machine, &sid),
            move |st, res, cx| {
                let e = st.threads.entry(key).or_default();
                e.loading = false;
                e.at = Self::now();
                match res {
                    Ok(t) => {
                        e.chats = t.chats;
                        e.failed = None;
                    }
                    Err(err) => e.failed = Some(err.to_string()),
                }
                cx.notify();
            },
        );
    }

    /// "This continues...": the chat at `key` carries on from the older chat (to_machine, to_sid).
    /// `done` hears whether it worked (the picker stays open on an error).
    pub fn link(
        &mut self,
        key: &str,
        to_machine: String,
        to_sid: String,
        to_title: String,
        done: impl FnOnce(Result<(), String>, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        let Some(s) = self.snap.session(key) else {
            return cx.defer(move |cx| done(Err("This chat isn't in the list anymore.".into()), cx));
        };
        let (machine, sid) = (s.machine.clone(), s.sid.clone());
        let busy = format!("{key}:link");
        self.busy.insert(busy.clone());
        cx.notify();
        self.call(
            cx,
            move |core| core.link(&machine, &sid, &to_machine, &to_sid),
            move |st, res, cx| {
                st.busy.remove(&busy);
                match res {
                    Ok(_) => {
                        st.threads.clear();
                        st.toast(format!("Linked. It now continues {to_title}."), None, false, cx);
                        st.refresh(cx);
                        cx.defer(move |cx| done(Ok(()), cx));
                    }
                    Err(e) => {
                        st.toast(e.to_string(), None, true, cx);
                        cx.defer(move |cx| done(Err(e.to_string()), cx));
                    }
                }
                cx.notify();
            },
        );
    }

    /// "Not the same topic". On a current chat it starts its own history; on a replaced one (`older`) the chats that
    /// carry on from it go their own way, and it is back on the list.
    pub fn unlink(&mut self, key: &str, older: bool, cx: &mut Context<Self>) {
        let (machine, sid) = if let Some(s) = self.snap.session(key) {
            (s.machine.clone(), s.sid.clone())
        } else if let Some(r) = self.snap.replaced_row(key) {
            (r.machine.clone(), r.sid.clone())
        } else {
            return;
        };
        let busy = format!("{key}:unlink");
        if !self.busy.insert(busy.clone()) {
            return;
        }
        cx.notify();
        self.call(
            cx,
            move |core| core.unlink(&machine, &sid, older),
            move |st, res, cx| {
                st.busy.remove(&busy);
                match res {
                    Ok(_) => {
                        st.threads.clear();
                        let msg = if older {
                            "Split off. It's back on the list as a chat of its own."
                        } else {
                            "Split off. This chat starts its own history now."
                        };
                        st.toast(msg, None, false, cx);
                        st.refresh(cx);
                    }
                    Err(e) => st.toast(e.to_string(), None, true, cx),
                }
                cx.notify();
            },
        );
    }

    /// Continue on another machine: a handoff here, then a new chat there in its Claude folder.
    pub fn continue_on(&mut self, key: &str, to: &str, cx: &mut Context<Self>) {
        self.continue_when(key, to, None, cx)
    }

    pub fn continue_when(&mut self, key: &str, to: &str, when: Option<&'static str>, cx: &mut Context<Self>) {
        self.continue_with(key, to, when, false, cx)
    }

    /// The user's answer to a need "trust": yes sends the move again with `trust: true`; no (None) drops it.
    pub fn answer_trust(&mut self, yes: bool, cx: &mut Context<Self>) {
        let Some(ask) = self.trust_ask.take() else { return };
        cx.notify();
        if !yes {
            return;
        }
        match ask.retry {
            TrustRetry::Continue { key, to, when } => self.continue_with(&key, &to, when, true, cx),
        }
    }

    fn continue_with(&mut self, key: &str, to: &str, when: Option<&'static str>, trust: bool, cx: &mut Context<Self>) {
        let Some(s) = self.snap.session(key) else { return };
        let busy = format!("{key}:continue");
        if !self.busy.insert(busy.clone()) {
            return;
        }
        let req = ContinueRequest {
            machine: s.machine.clone(),
            sid: s.sid.clone(),
            to: to.to_string(),
            folder: String::new(),
            create: false,
            when: when.map(str::to_string),
            trust,
        };
        let to_s = to.to_string();
        let src = s.machine_label.clone();
        let dst = self.snap.label_of(to).unwrap_or_else(|| to.to_string());
        let title = s.title.clone();
        let topic = s.topic.clone();
        let key = key.to_string();
        cx.notify();
        self.call(
            cx,
            move |core| core.continue_on(req),
            move |st, res, cx| {
                st.busy.remove(&busy);
                match res {
                    Ok(Some(jid)) => {
                        let name = names::from_machine(&topic, &src);
                        st.start_job(&jid, JobKind::Continue, Some(key), &dst, &src, Some(title), Some(name), cx);
                    }
                    Ok(None) => st.toast("Sent", None, false, cx),
                    Err(CoreError::Busy { message, busy }) => {
                        st.ask_busy("Move it", message, busy, BusyRetry::Continue { key, to: to_s }, cx)
                    }
                    Err(CoreError::Trust { folder_shown, declined, .. }) => {
                        st.trust_ask = Some(TrustAsk {
                            shown: folder_shown,
                            declined,
                            on: dst,
                            retry: TrustRetry::Continue { key, to: to_s, when },
                        });
                    }
                    Err(e) => st.toast(e.to_string(), None, true, cx),
                }
                cx.notify();
            },
        );
    }

    /// v2: restarts a chat with new launch flags, after the same "busy" ask as stop and move when its own work is
    /// still running. Followed as a job; a terminal open on it reconnects when it is done.
    pub fn relaunch(&mut self, key: &str, body: serde_json::Value, when: Option<&'static str>, cx: &mut Context<Self>) {
        let Some(s) = self.snap.session(key) else { return };
        let busy = format!("{key}:relaunch");
        if !self.busy.insert(busy.clone()) {
            return;
        }
        let (m, sid, label, title) = (s.machine.clone(), s.sid.clone(), s.machine_label.clone(), s.title.clone());
        let key = key.to_string();
        let b2 = body.clone();
        cx.notify();
        self.call(
            cx,
            move |core| core.relaunch(&m, &sid, &b2, when),
            move |st, res, cx| {
                st.busy.remove(&busy);
                match res {
                    Ok(r) if r.need.as_deref() == Some("terminal") => cx.emit(AppEvent::RelaunchCmd {
                        key,
                        command: r.command.unwrap_or_default(),
                        error: r.error.unwrap_or_else(|| "Run this in the chat's own terminal.".into()),
                    }),
                    Ok(r) => match r.job {
                        Some(jid) => {
                            st.start_job(
                                &jid,
                                JobKind::Relaunch,
                                Some(key.clone()),
                                &label,
                                &label,
                                Some(title),
                                None,
                                cx,
                            );
                            cx.emit(AppEvent::Relaunching { key });
                        }
                        None => st.toast(
                            r.error.unwrap_or_else(|| "Switchboard couldn't relaunch it.".into()),
                            None,
                            true,
                            cx,
                        ),
                    },
                    Err(CoreError::Busy { message, busy }) => {
                        st.ask_busy("Relaunch it", message, busy, BusyRetry::Relaunch { key, body }, cx)
                    }
                    Err(e) => st.toast(e.to_string(), None, true, cx),
                }
                cx.notify();
            },
        );
    }

    /// New session. `done` hears whether the server took it (the sheet stays open on an error), and the job's id.
    pub fn start_new(
        &mut self,
        req: NewRequest,
        label: String,
        name: String,
        done: impl FnOnce(Result<Option<String>, CoreError>, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        let kind = if req.machine == "auto" { JobKind::Auto } else { JobKind::Start };
        self.call(
            cx,
            move |core| core.start_new(req),
            move |st, res, cx| {
                match res {
                    Ok(Some(jid)) => {
                        st.start_job(&jid, kind, None, &label, &label, None, Some(name), cx);
                        cx.defer(move |cx| done(Ok(Some(jid)), cx));
                    }
                    Ok(None) => {
                        st.toast(format!("Sent to {label}"), None, false, cx);
                        cx.defer(move |cx| done(Ok(None), cx));
                    }
                    // The sheet shows the error under the message (or asks, for a folder Claude was never let
                    // into), so nothing typed is lost.
                    Err(e) => cx.defer(move |cx| done(Err(e), cx)),
                }
            },
        );
    }

    /// Resume and open: the resume job, whose id `done` hears. The terminal tab follows the job and asks for its
    /// ticket when it ends.
    pub fn resume_for_term(
        &mut self,
        key: &str,
        done: impl FnOnce(Result<String, String>, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        let Some(s) = self.snap.session(key) else {
            return cx.defer(move |cx| done(Err("This chat isn't in the list anymore.".into()), cx));
        };
        let (machine, sid, title, label, topic) =
            (s.machine.clone(), s.sid.clone(), s.title.clone(), s.machine_label.clone(), s.topic.clone());
        let busy = format!("{key}:resume");
        self.busy.insert(busy.clone());
        let key = key.to_string();
        cx.notify();
        self.call(
            cx,
            move |core| core.act(&machine, "resume", &sid, None),
            move |st, res, cx| {
                st.busy.remove(&busy);
                match res {
                    Ok(Some(jid)) => {
                        st.start_job(
                            &jid,
                            JobKind::Resume,
                            Some(key),
                            &label,
                            &label,
                            Some(title),
                            Some(names::resumed(&topic)),
                            cx,
                        );
                        cx.defer(move |cx| done(Ok(jid), cx));
                    }
                    Ok(None) => cx.defer(move |cx| {
                        done(Err("The server took the resume but didn't say which job it is.".into()), cx)
                    }),
                    Err(e) => cx.defer(move |cx| done(Err(e.to_string()), cx)),
                }
                cx.notify();
            },
        );
    }

    // ---------- jobs ----------

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn start_job(
        &mut self,
        id: &str,
        kind: JobKind,
        key: Option<String>,
        label: &str,
        src: &str,
        from: Option<String>,
        name: Option<String>,
        cx: &mut Context<Self>,
    ) {
        if !add_job(&mut self.jobs, Job::new(id, kind, key, label, src, from, name, Self::now())) {
            return; // the server took the same action twice (a double click) and gave back the job it has
        }
        self.poll_job(id.to_string(), Duration::from_millis(1200), cx);
        self.refresh_in(Duration::from_millis(1200), cx);
        cx.notify();
    }

    fn poll_job(&mut self, id: String, first: Duration, cx: &mut Context<Self>) {
        let core = self.core.clone();
        cx.spawn(async move |this, cx| {
            let mut delay = first;
            loop {
                cx.background_executor().timer(delay).await;
                let (c, i) = (core.clone(), id.clone());
                let res = cx.background_executor().spawn(async move { c.poll_job(&i) }).await;
                match this.update(cx, |s, cx| s.take_job(&id, res, cx)) {
                    Ok(Some(d)) => delay = d,
                    _ => break,
                }
            }
        })
        .detach();
    }

    /// One poll reply. Returns when to look again, or None when the job is over.
    fn take_job(&mut self, id: &str, res: Result<ApiJob, CoreError>, cx: &mut Context<Self>) -> Option<Duration> {
        let now = Self::now();
        let job = self.jobs.iter_mut().find(|j| j.id == id)?;
        let was = job.state;
        match res {
            Ok(j) => job.update(&j, now),
            // The job carries on at the server; look again.
            Err(CoreError::Unreachable) => return Some(Duration::from_secs(3)),
            Err(CoreError::Server { status: 404, .. }) => {
                job.fail("Switchboard lost track of this one. Check the list in a minute.", now)
            }
            Err(e) => job.fail(&e.to_string(), now),
        }
        let finished = job.state.finished();
        let note =
            (finished && !was.finished() && !job.cancelled).then(|| (job.id.clone(), job.headline(), job.detail(now)));
        if relaunch_started(job.kind, was, job.state)
            && let Some(key) = job.key.clone()
        {
            cx.emit(AppEvent::RelaunchStarted { key, job: job.id.clone() });
        }
        if finished && !was.finished() {
            let ev = if job.state == JobState::Done {
                AppEvent::JobDone {
                    id: job.id.clone(),
                    machine: job.new_machine.clone(),
                    sid: job.new_sid.clone(),
                    bg: job.new_bg.clone(),
                }
            } else {
                AppEvent::JobFailed {
                    id: job.id.clone(),
                    error: job.error.clone().unwrap_or_else(|| "Something went wrong.".into()),
                }
            };
            cx.emit(ev);
        }
        let job = self.jobs.iter().find(|j| j.id == id)?;
        let select = if job.state == JobState::Done && !was.finished() {
            match (&job.new_machine, &job.new_sid) {
                (Some(m), Some(s)) => Some(format!("{m}:{s}")),
                _ => None,
            }
        } else {
            None
        };
        if finished && !was.finished() {
            self.refresh(cx);
            if let Some(k) = select {
                // Select it once the refresh has it.
                self.refresh_in(Duration::from_millis(900), cx);
                let key = k.clone();
                cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(Duration::from_millis(1500)).await;
                    let _ = this.update(cx, |s, cx| {
                        if s.snap.session(&key).is_some() {
                            s.select(key.clone(), cx);
                        }
                    });
                })
                .detach();
            }
        }
        // Not watching: the finished job is a notification ("Switchboard tells you when it is done").
        if let Some((jid, head, body)) = note
            && self.notify_on()
            && cx.active_window().is_none()
        {
            cx.show_system_notification(SystemNotification {
                tag: format!("job:{jid}").into(),
                title: head.into(),
                body: body.into(),
                actions: vec![],
            });
        }
        cx.notify();
        if finished { None } else { Some(Duration::from_millis(1500)) }
    }

    // ---------- helpers for views ----------

    pub fn fresh_confirm(&self, key: &str) -> Option<actions::Confirm> {
        self.snap.session(key).map(actions::fresh_confirm)
    }
}

#[cfg(test)]
mod tests {
    /// The 2026-10-12 crash: a callback that hears back from the server (`done`, given `&mut App`) ran while AppState
    /// was still being updated, and the New session sheet's callback updated AppState again. GPUI panics ("cannot
    /// update AppState while it is already being updated") inside a main-thread callout, and the app dies. Every
    /// such callback now runs deferred, after AppState is released.
    #[test]
    fn callbacks_run_after_app_state_is_released() {
        let src = include_str!("state.rs").replace("\r\n", "\n");
        let body = &src[..src.find("#[cfg(test)]\nmod tests").unwrap()];
        let lines: Vec<&str> = body.lines().map(str::trim).collect();
        // A call rustfmt wrapped onto its own line counts when the line before opens the deferred closure.
        let deferred = |i: usize| i > 0 && lines[i - 1].ends_with("cx.defer(move |cx| {");
        let bad: Vec<&str> = lines
            .iter()
            .enumerate()
            .filter(|(_, l)| l.contains("done(") && !l.starts_with("//") && !l.contains("done: impl"))
            .filter(|&(i, l)| {
                !l.contains("cx.defer(move |cx| done(")
                    && !deferred(i)
                    && *l != "done(st, cx);"
                    && !l.contains("|s, cx| done(s, res, cx)")
            })
            .map(|(_, l)| *l)
            .collect();
        assert!(bad.is_empty(), "call these through cx.defer: {bad:#?}");
    }

    #[test]
    fn unchanged_tabs_still_go_once_a_day() {
        let tabs = vec!["studio:a".to_string()];
        assert!(super::tabs_due(None, &tabs, 1000.0), "the first refresh sends them");
        let sent = (tabs.clone(), 1000.0);
        assert!(!super::tabs_due(Some(&sent), &tabs, 1000.0 + 3600.0), "unchanged within the day: not again");
        assert!(super::tabs_due(Some(&sent), &tabs, 1000.0 + 86400.0), "a day later: again");
        assert!(super::tabs_due(Some(&sent), &["studio:b".to_string()], 1001.0), "changed: at once");
    }

    fn job(id: &str, kind: super::JobKind, key: Option<&str>) -> super::Job {
        super::Job::new(id, kind, key.map(str::to_string), "Studio", "Studio", None, None, 1000.0)
    }

    #[test]
    fn the_same_job_twice_is_followed_once() {
        use super::{JobKind, JobState};
        let mut jobs = Vec::new();
        assert!(super::add_job(&mut jobs, job("j1", JobKind::Resume, Some("studio:a"))));
        assert!(!super::add_job(&mut jobs, job("j1", JobKind::Resume, Some("studio:a"))), "a double click");
        assert_eq!(jobs.len(), 1);
        let done = sb_core::api::ApiJob { state: Some("done".into()), ..Default::default() };
        jobs[0].update(&done, 1010.0);
        assert!(jobs.iter().all(|j| j.state == JobState::Done), "no card is left running");
    }

    #[test]
    fn a_relaunch_lets_go_of_its_terminal_only_once_it_runs() {
        use super::{JobKind, JobState, relaunch_started};
        assert!(!relaunch_started(JobKind::Relaunch, JobState::Queued, JobState::Queued), "waiting for the reply");
        assert!(relaunch_started(JobKind::Relaunch, JobState::Queued, JobState::Running));
        assert!(relaunch_started(JobKind::Relaunch, JobState::Queued, JobState::Done), "done before the next look");
        assert!(!relaunch_started(JobKind::Relaunch, JobState::Queued, JobState::Error), "cancelled: it stays live");
        assert!(!relaunch_started(JobKind::Relaunch, JobState::Running, JobState::Done), "said once");
        assert!(!relaunch_started(JobKind::Resume, JobState::Queued, JobState::Running));
    }

    #[test]
    fn settings_that_do_not_parse_are_kept_aside() {
        let dir = std::env::temp_dir().join(format!("sb-settings-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("settings.json");
        let text = r#"{"theme": "dark", "tabs": "not a list", "servers": ["http://192.0.2.10:8130"]}"#;
        std::fs::write(&p, text).unwrap();
        let (s, bad) = super::load_settings(&p);
        assert!(bad);
        assert_eq!(s.theme.as_deref(), Some("dark"), "what still reads is kept");
        assert!(s.tabs.is_empty());
        assert_eq!(s.saved.servers, vec!["http://192.0.2.10:8130".to_string()]);
        assert_eq!(std::fs::read_to_string(dir.join("settings.json.bad")).unwrap(), text, "the file as it was");
        std::fs::write(&p, r#"{"theme": "light"}"#).unwrap();
        let (s, bad) = super::load_settings(&p);
        assert!(!bad);
        assert_eq!(s.theme.as_deref(), Some("light"));
        assert!(!super::load_settings(&dir.join("missing.json")).1, "no file yet is not a problem");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn opening_a_chat_calls_off_only_the_stop_from_closing_its_tab() {
        use super::{JobKind, stops_from_close};
        let jobs = vec![job("j1", JobKind::Stop, Some("studio:a")), job("j2", JobKind::Stop, Some("studio:a"))];
        let from_close: std::collections::HashSet<String> = ["j1".to_string()].into();
        assert_eq!(stops_from_close(&jobs, &from_close, "studio:a"), vec!["j1".to_string()]);
        assert!(stops_from_close(&jobs, &Default::default(), "studio:a").is_empty(), "one from the board stays");
        assert!(stops_from_close(&jobs, &from_close, "studio:b").is_empty());
    }

    #[test]
    fn a_refresh_that_left_before_a_rename_is_read_again() {
        assert!(!super::overtaken(3, 3));
        assert!(super::overtaken(3, 4), "renamed while it was on its way: it has the old name");
    }

    #[test]
    fn a_notification_for_a_chat_that_is_gone_selects_nothing() {
        use super::{JobKind, tag_chat};
        let known = |k: &str| k == "studio:a" || k == "studio:b";
        let mut jobs = vec![job("j1", JobKind::Resume, Some("studio:a")), job("j2", JobKind::Stop, Some("studio:z"))];
        assert_eq!(tag_chat("studio:a", &jobs, known).as_deref(), Some("studio:a"));
        assert_eq!(tag_chat("studio:gone", &jobs, known), None);
        assert_eq!(tag_chat("job:j1", &jobs, known).as_deref(), Some("studio:a"), "the chat it came from");
        jobs[0].new_machine = Some("studio".into());
        jobs[0].new_sid = Some("b".into());
        assert_eq!(tag_chat("job:j1", &jobs, known).as_deref(), Some("studio:b"), "the chat it made");
        assert_eq!(tag_chat("job:j2", &jobs, known), None, "its chat was removed");
        assert_eq!(tag_chat("job:j9", &jobs, known), None);
    }
}
