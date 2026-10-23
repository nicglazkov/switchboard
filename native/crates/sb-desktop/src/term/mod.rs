//! A chat's own terminal, in a tab next to the board (spec section 1): `claude attach` on the chat's machine, through
//! the server's terminal hub. alacritty_terminal keeps the screen and parses what arrives; grid.rs draws it with the
//! app's colors. Closing the tab only detaches: the chat keeps running on its machine.
//!
//! The flow: ask the server for a ticket (POST /api/term), dial the hub's WebSocket with it, then bytes both ways.
//! A chat that is not running offers Resume and open (the resume job, then a ticket for its new background id).
//!
//! Every window showing a chat shares one terminal on the hub (spec 2026-10-14, terminal pipeline). The hub sends
//! the screen as a snapshot on connecting, and the terminal's size as a frame in the output: the grid here changes
//! size only then, so output is always parsed at the size it was written for. The view asks for the size it has
//! room for; the window that last had focus or typed gets it. A dropped link comes back by itself (reconnect.rs)
//! with the screen kept.

pub mod colors;
pub mod exitline;
pub mod grid;
pub mod links;
pub mod local;
pub mod upload;

use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;
use std::time::{Duration, Instant};

use alacritty_terminal::Term;
use alacritty_terminal::event::{Event as TermEv, EventListener, WindowSize};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Point as GridPoint, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{Config, TermMode, viewport_to_point};
use alacritty_terminal::vte::ansi::{Processor, StdSyncHandler};
use gpui::{
    AnyElement, App, Bounds, Context, ElementInputHandler, Entity, EntityInputHandler, EventEmitter, FocusHandle,
    Focusable, Hsla, InteractiveElement, IntoElement, KeyBinding, KeyDownEvent, Modifiers, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, NoAction, ParentElement, Pixels, Render, ScrollWheelEvent,
    StatefulInteractiveElement, Styled, Subscription, UTF16Selection, Window, actions, canvas, div, point, prelude::*,
    px, size,
};
use sb_core::names;
use sb_core::term::keys::{self, Mods};
use sb_core::term::reconnect::{self, Backoff};
use sb_core::term::shells::Profile;
use sb_core::term::ws::{Closed, FromHub, ToHub};
use sb_core::term::{Need, TermError, TermRequest, TermTicket};
use smol::channel::{Receiver, Sender};

use crate::state::{AppEvent, AppState};
use crate::theme::{Theme, mix};
use crate::ui::{self, MAC};
use grid::Metrics;

actions!(terminal, [TermCopy, TermPaste, TermPageUp, TermPageDown, TermBigger, TermSmaller, TermSizeReset, TermRedraw]);

/// Keys inside a terminal. Registered after the app's own, so these win: Enter, Esc and Tab go to the program, and on
/// Windows and Linux so do the Ctrl keys a shell needs (Ctrl+W deletes a word there); the app takes Ctrl+Shift instead.
pub fn bind_keys(cx: &mut App) {
    let t = Some("Terminal");
    let mut b = vec![
        KeyBinding::new("enter", NoAction, t),
        KeyBinding::new("escape", NoAction, t),
        KeyBinding::new("tab", NoAction, t),
        KeyBinding::new("shift-tab", NoAction, t),
        KeyBinding::new("shift-pageup", TermPageUp, t),
        KeyBinding::new("shift-pagedown", TermPageDown, t),
        // Redraw: the hub sends the screen again (a screen that looks wrong comes right).
        KeyBinding::new("secondary-shift-r", TermRedraw, t),
    ];
    // Text size: Cmd+Plus, Cmd+Minus and Cmd+0 (Ctrl on Windows and Linux), as in every terminal.
    for (k, big) in [
        ("secondary-=", Some(true)),
        ("secondary-+", Some(true)),
        ("secondary-shift-=", Some(true)),
        ("secondary--", Some(false)),
        ("secondary-0", None),
    ] {
        b.push(match big {
            Some(true) => KeyBinding::new(k, TermBigger, t),
            Some(false) => KeyBinding::new(k, TermSmaller, t),
            None => KeyBinding::new(k, TermSizeReset, t),
        });
    }
    // Copy and paste. Ctrl+C copies too when text is selected (`on_key`); with nothing selected it goes to the program.
    if MAC {
        b.push(KeyBinding::new("cmd-c", TermCopy, t));
        b.push(KeyBinding::new("cmd-v", TermPaste, t));
    } else {
        b.push(KeyBinding::new("ctrl-v", TermPaste, t));
        b.push(KeyBinding::new("ctrl-shift-c", TermCopy, t));
        b.push(KeyBinding::new("ctrl-shift-v", TermPaste, t));
        b.push(KeyBinding::new("ctrl-insert", TermCopy, t));
        b.push(KeyBinding::new("shift-insert", TermPaste, t));
        for k in ["ctrl-n", "ctrl-k", "ctrl-enter", "ctrl-q", "ctrl-w", "ctrl-t", "ctrl-o", "ctrl-b", "ctrl-c"] {
            b.push(KeyBinding::new(k, NoAction, t));
        }
    }
    cx.bind_keys(b);
    // From the keys in a terminal and from the Terminal menu, whichever window is in front.
    cx.on_action(|_: &TermBigger, cx: &mut App| step_size(Some(true), cx));
    cx.on_action(|_: &TermSmaller, cx: &mut App| step_size(Some(false), cx));
    cx.on_action(|_: &TermSizeReset, cx: &mut App| step_size(None, cx));
}

/// Collects what the terminal asks for while parsing (replies to queries, the bell, clipboard writes).
#[derive(Clone, Default)]
pub struct Listener(Rc<RefCell<Vec<TermEv>>>);

impl EventListener for Listener {
    fn send_event(&self, event: TermEv) {
        self.0.borrow_mut().push(event);
    }
}

#[derive(Clone, Copy)]
struct GridSize {
    cols: usize,
    rows: usize,
}

impl Dimensions for GridSize {
    fn total_lines(&self) -> usize {
        self.rows
    }
    fn screen_lines(&self) -> usize {
        self.rows
    }
    fn columns(&self) -> usize {
        self.cols
    }
}

/// Parsing per slice of the UI thread: a flood of output is parsed this much at a time, with frames drawn between.
const PARSE_BUDGET: Duration = Duration::from_millis(4);
/// Output is parsed in pieces of at most this much, so one big chunk cannot overrun the budget by much.
const PARSE_PIECE: usize = 16 * 1024;

/// What a view that changed size does with its grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SizeStep {
    /// Resize the grid now and tell the hub (nobody is drawing into it yet).
    Now,
    /// Resize the grid once the window stops moving, and tell the hub (an older hub that sends no sizes).
    Settled,
    /// Only ask the hub, once the window stops moving: the grid follows the hub's size frame, in order with the
    /// output, so nothing is parsed at the wrong size.
    Ask,
}

fn size_step(hub_sizes: bool, open: bool, first: bool) -> SizeStep {
    match (hub_sizes && open, first || !open) {
        (true, _) => SizeStep::Ask,
        (false, true) => SizeStep::Now,
        (false, false) => SizeStep::Settled,
    }
}

/// Coming back by itself after the link dropped.
#[derive(Debug, Default)]
pub struct Reconnect {
    /// Bumped per wait, so "Reconnect now" and a new wait cancel the old timer.
    wait_id: u64,
}

/// How soon after Ctrl-D or Ctrl-Z the attach must end for the end to count as the person leaving. A key a program
/// kept for itself (Ctrl-D in an editor) does not make an end minutes later look asked for.
const QUIT_WINDOW: Duration = Duration::from_secs(5);

fn quit_asked(at: Option<Instant>, now: Instant) -> bool {
    at.is_some_and(|t| now.saturating_duration_since(t) <= QUIT_WINDOW)
}

/// The attach ended because the person asked: they left with `exit` (and typed nothing since, however long the end
/// took to come), or pressed Ctrl-D or Ctrl-Z just before.
fn end_asked(leaving: bool, quit_key: Option<Instant>, now: Instant) -> bool {
    leaving || quit_asked(quit_key, now)
}

/// A job's tab asks by background id only when the job reported one: one that did not leaves an ordinary chat.
fn prefer_bg_after(bg: Option<&str>) -> bool {
    bg.is_some()
}

/// What to ask /api/term for: the background id right after a job that reported one, else the chat.
fn term_request(machine: &str, bg: Option<&str>, sid: Option<&str>, prefer_bg: bool) -> Option<TermRequest> {
    match (bg, sid) {
        (Some(bg), _) if prefer_bg => Some(TermRequest::bg(machine, bg)),
        (_, Some(sid)) => Some(TermRequest::chat(machine, sid)),
        (Some(bg), None) => Some(TermRequest::bg(machine, bg)),
        (None, None) => None,
    }
}

fn new_term(cols: u16, rows: u16, events: &Listener) -> Term<Listener> {
    let config = Config { scrolling_history: 5000, ..Config::default() };
    Term::new(config, &GridSize { cols: cols as usize, rows: rows as usize }, events.clone())
}

/// Where a tab is.
#[derive(Clone, Debug, PartialEq)]
pub enum Phase {
    /// Asking the server for a ticket.
    Ticket,
    /// Waiting on a job (Resume and open, or New session) before asking.
    Job(String),
    /// Dialing the hub.
    Dial,
    /// Connected; the hub is starting `claude attach` on the machine.
    Hub,
    Open,
    /// The connection closed: why, and whether Reconnect can help. `code`: what `claude attach` exited with, when it
    /// ended with an error (a clean exit closes the tab instead, like a real terminal).
    Ended {
        note: String,
        retry: bool,
        code: Option<i64>,
    },
    /// The server would not open it (not running, interactive, offline).
    Need {
        need: Need,
        message: String,
    },
    Failed(String),
}

impl Phase {
    pub fn connecting(&self) -> bool {
        matches!(self, Phase::Ticket | Phase::Job(_) | Phase::Dial | Phase::Hub)
    }

    pub fn color(&self, t: &Theme) -> Hsla {
        match self {
            Phase::Open => t.run,
            Phase::Ticket | Phase::Job(_) | Phase::Dial | Phase::Hub => t.cold,
            Phase::Ended { .. } => t.stall,
            Phase::Need { need: Need::Resume, .. } => t.stall,
            Phase::Need { .. } => t.need,
            Phase::Failed(_) => t.bad,
        }
    }

    pub fn word(&self) -> &'static str {
        match self {
            Phase::Open => "Attached",
            Phase::Ticket | Phase::Dial | Phase::Hub => "Connecting",
            Phase::Job(_) => "Starting",
            Phase::Ended { code: Some(_), .. } => "Exited",
            Phase::Ended { .. } => "Detached",
            Phase::Need { need: Need::Resume, .. } => "Not running",
            Phase::Need { need: Need::Terminal, .. } => "Open on its machine",
            Phase::Need { need: Need::Offline, .. } => "Offline",
            Phase::Failed(_) => "Not connected",
        }
    }
}

/// How a tab begins.
pub enum Start {
    /// A running background chat, by board key.
    Chat(String),
    /// A stopped chat: resume it, then open.
    Resume(String),
    /// A starred chat that has stopped, brought back on launch: a placeholder with Resume (resuming costs tokens,
    /// so it never happens by itself).
    Stopped(String),
    /// A job already started (New session with "Open its terminal").
    Job { id: String, machine: String, label: String, title: String },
    /// A shell on this computer (local.rs): no chat, no server, never on the board.
    Local(Profile),
}

/// What a local terminal's tab shows where a chat's shows its machine.
pub const LOCAL_LABEL: &str = if MAC { "This Mac" } else { "This PC" };

/// The message of a starred tab that came back on launch while its chat had stopped.
pub const STOPPED_STAR: &str = "stopped-starred";

/// A terminal with key `tab_key` and background id `tab_bg` holds the chat at `key` (background id `bg`).
pub fn same_chat(tab_key: Option<&str>, tab_bg: Option<&str>, key: &str, bg: Option<&str>) -> bool {
    tab_key == Some(key) || (bg.is_some() && tab_bg == bg)
}

/// A clean exit (code 0: Ctrl-D) closes the tab. An error, a lost connection (ssh's 255) or no code keeps it open
/// with the last output.
pub fn exit_closes_tab(exit: Option<Option<i64>>) -> bool {
    exit == Some(Some(0))
}

/// The machine's agent ends the terminal with this code when the person left the chat (`exit`, `/exit`, the left
/// arrow), where Claude Code would open its agent list instead (sb.py TERM_LEFT). The tab closes, asked for or not.
pub const TERM_LEFT: i64 = 100;

pub fn left_chat(exit: Option<Option<i64>>) -> bool {
    exit == Some(Some(TERM_LEFT))
}

pub enum TermEvent {
    /// The title, state or bell changed: the tab strip redraws.
    Changed,
    /// The bar's flags button: open the flags drawer for this chat.
    Flags(String),
    /// Close this tab (detaches).
    Close,
}

pub struct TermTab {
    app: Entity<AppState>,
    /// The board key ("machine:sid") once known.
    pub key: Option<String>,
    pub machine: String,
    pub label: String,
    pub sid: Option<String>,
    pub bg: Option<String>,
    pub title: String,
    /// The url to open the chat in Claude (for an interactive chat that cannot open here).
    url: Option<String>,
    /// After a job, ask by background id (spec: when the job reports bg_id, ask with {machine, bg}).
    prefer_bg: bool,
    pub phase: Phase,
    pub focus: FocusHandle,
    pub(crate) term: Term<Listener>,
    parser: Processor<StdSyncHandler>,
    events: Listener,
    conn: Option<Sender<ToHub>>,
    /// Bumped on every connection, so late events from an old one are dropped.
    epoch: u64,
    /// The grid now, and what the view has room for.
    size: (u16, u16),
    want: Option<(u16, u16)>,
    resize_gen: u64,
    sync_armed: bool,
    geom: Option<(Metrics, Bounds<Pixels>)>,
    /// Text an input method is composing, shown at the cursor.
    pub(crate) marked: Option<String>,
    selecting: bool,
    /// The link under the mouse (its viewport row and cells), underlined, with a tip saying what a click does.
    pub(crate) hover: Option<(usize, links::Found)>,
    /// A plain click that started on a link: it opens on release unless the mouse dragged a selection.
    link_press: Option<(usize, usize)>,
    /// The button a program is being told about (mouse reporting), and the last cell reported.
    mouse_held: Option<u8>,
    last_cell: Option<(usize, usize)>,
    scroll_px: f32,
    pub bell: bool,
    pub(crate) focused: bool,
    exit_note: Option<String>,
    /// What the hub said `claude attach` exited with, until the connection closes.
    exit_code: Option<Option<i64>>,
    /// A new connection: the next output starts on a clean screen.
    fresh: bool,
    /// This connection's hub sends size frames: the grid changes size only when one arrives.
    hub_sizes: bool,
    /// This view sets the terminal's size (it had focus or typed last). Only it answers the program's questions
    /// (cursor position, colors) and reports focus, so the program never gets two answers.
    owner: bool,
    /// Inside a snapshot: the screen being redrawn, not new output.
    in_snapshot: bool,
    /// Events from the hub waiting for the next slice of parsing.
    pending: std::collections::VecDeque<FromHub>,
    /// The link dropped and it is coming back by itself: the screen stays, the bar says so.
    pub(crate) reconnect: Option<Reconnect>,
    /// The waits between tries at coming back. A link that opens and drops again at once does not start them over.
    backoff: Backoff,
    has_output: bool,
    /// When the terminal last showed output of Claude's own (not an echo of typing, a redraw after a resize, or the
    /// screen sent on connecting): the live state's hint that it is working, and where its warm countdown starts.
    pub last_out: Option<f64>,
    /// Output before this is not counted (just connected, just resized).
    quiet_until: f64,
    /// The last keystroke or paste sent; output within a moment of it is the echo.
    last_key: std::cell::Cell<f64>,
    /// What was typed since the last Enter, for "exit" (exitline.rs), and a leave sent that the attach's end closes.
    line: exitline::ExitLine,
    leaving: bool,
    /// When the last key typed was Ctrl-D or Ctrl-Z: a clean end soon after it is the user leaving, so the tab goes.
    quit_key: Option<Instant>,
    /// When this terminal saw the current stretch of work start ("Working for 2m 10s").
    work_since: Option<f64>,
    /// Screenshots only: a live state to show without a connection.
    pub fake_live: Option<sb_core::live::Live>,
    /// A local terminal's profile, and its PTY while the shell runs. The tab's title is the shell's own (OSC 0 or 2)
    /// when it sets one, else the profile's name.
    pub local: Option<Profile>,
    pty: Option<local::Pty>,
    /// The color scheme popover above the bottom bar is open.
    colors_open: bool,
    /// What the chat runs with, in a few words ("Opus · High · Asks first"), read once it is attached.
    flags_sum: Option<String>,
    flags_asked: bool,
    /// Files on their way to the chat's machine (upload.rs).
    pub(crate) uploads: Vec<upload::Upload>,
    pub(crate) up_seq: u64,
    /// The cursor's blink: showing now, and the timer that flips it while this tab has focus.
    blink_on: bool,
    blink_task: Option<gpui::Task<()>>,
    _subs: Vec<Subscription>,
}

impl EventEmitter<TermEvent> for TermTab {}

impl Focusable for TermTab {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl TermTab {
    pub fn new(start: Start, cx: &mut Context<Self>) -> TermTab {
        let app = crate::state::app(cx);
        let events = Listener::default();
        let term = new_term(120, 36, &events);
        let sub = cx.subscribe(&app, |this, _, ev: &AppEvent, cx| this.on_app(ev, cx));
        let mut tab = TermTab {
            app: app.clone(),
            key: None,
            machine: String::new(),
            label: String::new(),
            sid: None,
            bg: None,
            title: String::new(),
            url: None,
            prefer_bg: false,
            phase: Phase::Ticket,
            focus: cx.focus_handle(),
            term,
            parser: Processor::new(),
            events,
            conn: None,
            epoch: 0,
            size: (120, 36),
            want: None,
            resize_gen: 0,
            sync_armed: false,
            geom: None,
            marked: None,
            selecting: false,
            hover: None,
            link_press: None,
            mouse_held: None,
            last_cell: None,
            scroll_px: 0.,
            bell: false,
            focused: false,
            exit_note: None,
            exit_code: None,
            fresh: true,
            hub_sizes: false,
            owner: true,
            in_snapshot: false,
            pending: Default::default(),
            reconnect: None,
            backoff: Backoff::default(),
            has_output: false,
            last_out: None,
            quiet_until: 0.0,
            last_key: std::cell::Cell::new(0.0),
            line: Default::default(),
            leaving: false,
            quit_key: None,
            work_since: None,
            fake_live: None,
            local: None,
            pty: None,
            colors_open: false,
            flags_sum: None,
            flags_asked: false,
            uploads: Vec::new(),
            up_seq: 0,
            blink_on: true,
            blink_task: None,
            _subs: vec![sub],
        };
        match start {
            Start::Chat(key) | Start::Resume(key) | Start::Stopped(key)
                if app.read(cx).snap.session(&key).is_none() =>
            {
                tab.key = Some(key);
                tab.phase = Phase::Failed("This chat isn't in the list anymore.".into());
            }
            Start::Chat(key) => {
                tab.fill(&key, cx);
                tab.connect(cx);
            }
            Start::Resume(key) => {
                tab.fill(&key, cx);
                tab.resume(cx);
            }
            Start::Stopped(key) => {
                tab.fill(&key, cx);
                tab.phase = Phase::Need { need: Need::Resume, message: STOPPED_STAR.into() };
            }
            Start::Job { id, machine, label, title } => {
                tab.machine = machine;
                tab.label = label;
                tab.title = if title.is_empty() { "New chat".into() } else { title };
                tab.phase = Phase::Job(id);
            }
            Start::Local(p) => {
                tab.title = p.name.clone();
                tab.label = LOCAL_LABEL.into();
                tab.local = Some(p);
                tab.connect(cx);
            }
        }
        tab
    }

    fn fill(&mut self, key: &str, cx: &App) {
        if let Some(s) = self.app.read(cx).snap.session(key) {
            self.key = Some(key.to_string());
            self.machine = s.machine.clone();
            self.label = s.machine_label.clone();
            self.sid = Some(s.sid.clone());
            self.bg = s.bg_id.clone();
            self.title = s.topic.clone();
            self.url = s.url.clone();
        }
    }

    /// The chat's live state (Working, Warm with its countdown, Cold), from the board's numbers and this terminal's
    /// own output. None until the tab knows its chat, or while the chat is not running here.
    pub fn live_state(&self, cx: &App) -> Option<sb_core::live::Live> {
        if let Some(l) = &self.fake_live {
            return Some(l.clone());
        }
        let app = self.app.read(cx);
        let s = self.chat(cx)?;
        if !s.alive && self.phase != Phase::Open {
            return None;
        }
        Some(app.live_of(s, AppState::now(), self.last_out))
    }

    /// The chat this tab shows, found the one way the tab's dot and the strip both use (Snapshot::chat_for): after
    /// /clear the background id still leads to it.
    fn chat<'a>(&self, cx: &'a App) -> Option<&'a sb_core::Session> {
        self.app.read(cx).snap.chat_for(&self.machine, self.bg.as_deref(), self.key.as_deref())
    }

    /// The chat this tab shows, for Rename: its board key and its name as the server has it.
    pub fn chat_name(&self, cx: &App) -> Option<(String, String)> {
        self.chat(cx).map(|s| (s.key.clone(), s.raw_title.clone()))
    }

    /// The tab's title follows its chat's name (a rename here or on another device, the next part after /clear).
    pub fn sync_title(&mut self, cx: &App) -> bool {
        match self.chat(cx).map(|s| s.topic.clone()).filter(|t| !t.trim().is_empty()) {
            Some(t) if t != self.title => {
                self.title = t;
                true
            }
            _ => false,
        }
    }

    /// This terminal holds the chat at `key` (background id `bg`): by its key, or by the background id while the key
    /// has not caught up with a /clear yet.
    pub fn shows(&self, key: &str, bg: Option<&str>) -> bool {
        same_chat(self.key.as_deref(), self.bg.as_deref(), key, bg)
    }

    /// After /clear the chat goes on under a new session id with the same background id: the tab's key follows it, so
    /// the tab is saved, closed and shown as the chat it holds now. True when it changed.
    pub fn sync_key(&mut self, cx: &App) -> bool {
        if self.local.is_some() || matches!(self.phase, Phase::Job(_)) {
            return false;
        }
        let snap = &self.app.read(cx).snap;
        let Some(s) = self.bg.as_deref().and_then(|b| snap.session_by_bg(&self.machine, b)) else { return false };
        if self.key.as_deref() == Some(s.key.as_str()) {
            return false;
        }
        self.key = Some(s.key.clone());
        self.sid = Some(s.sid.clone());
        true
    }

    /// The tab is no longer on screen (another tab shows, or it closed): it stops counting as focused, so its bell
    /// shows on the tab again. Hidden, it gets no frame to notice by itself.
    pub fn hide(&mut self) {
        if !self.focused {
            return;
        }
        self.focused = false;
        if self.phase == Phase::Open && self.owner && self.term.mode().contains(TermMode::FOCUS_IN_OUT) {
            self.send(b"\x1b[O".to_vec());
        }
    }

    /// Screenshots: show the strip in a state (run, warm, warmend, cold, wait, parked) with no connection.
    pub fn shot_live(&mut self, kind: &str, cx: &mut Context<Self>) {
        use sb_core::live::{Live, LiveKind as K, clock};
        let now = AppState::now();
        let mk = |kind, word, detail: String, left: u64| Live {
            kind,
            word,
            detail,
            pill: None,
            pill_kind: kind,
            left,
            urgent: left > 0 && left <= 60,
            expensive: None,
            stale: None,
            hint: false,
        };
        let l = match kind {
            "run" => {
                self.work_since = Some(now - 130.);
                mk(K::Run, "Working", String::new(), 0)
            }
            "warm" => mk(K::Warm, "Warm", format!("cache good for {}", clock(252)), 252),
            "warmend" => mk(K::Warm, "Warm", format!("cache good for {}", clock(41)), 41),
            "wait" => mk(K::Wait, "Needs you", "waiting for your answer".into(), 0),
            "parked" => mk(K::Parked, "Parked", "waiting 2d".into(), 0),
            "stalled" => mk(K::Stall, "Stalled", "no word for 7h".into(), 0),
            "stale" => Live { stale: Some("as of 2m ago".into()), ..mk(K::Warm, "Warm", "as of 2m ago".into(), 0) },
            _ => Live {
                expensive: Some("re-reads 312k".into()),
                ..mk(K::Cold, "Cold", "next message re-reads 312k".into(), 0)
            },
        };
        self.fake_live = Some(l);
        self.phase = Phase::Open;
        self.has_output = true;
        cx.notify();
    }

    /// "<title> - <machine>", as the tab shows it.
    pub fn ended(&self) -> bool {
        !matches!(self.phase, Phase::Open) && !self.phase.connecting()
    }

    fn changed(&mut self, cx: &mut Context<Self>) {
        cx.emit(TermEvent::Changed);
        cx.notify();
    }

    // ---------- connecting ----------

    /// Asks for a ticket, then dials. Also Reconnect and Try again.
    pub fn connect(&mut self, cx: &mut Context<Self>) {
        self.reconnect = None;
        self.backoff = Backoff::default();
        if self.local.is_some() {
            return self.start_local(cx);
        }
        self.dial_again(cx);
    }

    /// A local terminal: the shell starts in its PTY, and its events are read as the hub's are. Restart comes here
    /// too, with a clean screen.
    fn start_local(&mut self, cx: &mut Context<Self>) {
        let Some(p) = self.local.clone() else { return };
        self.epoch += 1;
        self.drop_conn();
        self.pty = None;
        self.pending.clear();
        self.exit_note = None;
        self.exit_code = None;
        self.hub_sizes = false;
        self.owner = true;
        self.in_snapshot = false;
        self.apply_size(false);
        let (cols, rows) = self.size;
        match local::spawn(&p, cols, rows) {
            Ok((conn, pty)) => {
                self.conn = Some(conn.tx);
                self.pty = Some(pty);
                self.fresh = true;
                self.phase = Phase::Dial;
                self.read(conn.rx, self.epoch, cx);
            }
            Err(e) => {
                log::warn!("local terminal: {} wouldn't start: {e}", p.name);
                self.phase = Phase::Failed(format!("{} wouldn't start: {e}", p.program));
            }
        }
        self.changed(cx);
    }

    /// A local terminal running something other than its shell (a build, an editor): its name, so closing the tab
    /// asks first.
    pub fn busy(&self) -> Option<String> {
        self.pty.as_ref().filter(|_| self.phase == Phase::Open).and_then(|p| p.busy())
    }

    fn dial_again(&mut self, cx: &mut Context<Self>) {
        self.epoch += 1;
        let epoch = self.epoch;
        self.drop_conn();
        self.pending.clear();
        self.phase = Phase::Ticket;
        self.exit_note = None;
        self.exit_code = None;
        let Some(req) = term_request(&self.machine, self.bg.as_deref(), self.sid.as_deref(), self.prefer_bg) else {
            self.phase = Phase::Failed("Switchboard doesn't know which chat this is.".into());
            return self.changed(cx);
        };
        let core = self.app.read(cx).core.clone();
        cx.spawn(async move |this, cx| {
            let res = cx.background_executor().spawn(async move { core.term_ticket(&req) }).await;
            let _ = this.update(cx, |t, cx| t.took_ticket(epoch, res, cx));
        })
        .detach();
        self.changed(cx);
    }

    fn took_ticket(&mut self, epoch: u64, res: Result<TermTicket, TermError>, cx: &mut Context<Self>) {
        if epoch != self.epoch {
            return;
        }
        match res {
            Ok(ticket) => {
                if let Some(bg) = ticket.bg.clone().filter(|b| !b.is_empty()) {
                    self.bg = Some(bg);
                }
                if let Some(title) = ticket.title.as_deref().filter(|s| !s.trim().is_empty()) {
                    self.title = names::split(title).0;
                }
                if let Some(l) = ticket.label.clone().filter(|s| !s.is_empty()) {
                    self.label = l;
                }
                self.dial(&ticket, cx);
            }
            Err(e) if self.reconnect.is_some() && reconnect::ticket_retry(&e) => {
                // the server or the machine is away for a moment: wait and try again
                if !self.retry_later(cx) {
                    self.phase = match e {
                        TermError::Need { need, message } => Phase::Need { need, message },
                        TermError::Core(e) => Phase::Failed(e.to_string()),
                    };
                }
            }
            Err(e) => {
                self.reconnect = None;
                self.phase = match e {
                    TermError::Need { need, message } => Phase::Need { need, message },
                    TermError::Core(e) => Phase::Failed(e.to_string()),
                };
            }
        }
        self.changed(cx);
    }

    /// The next try at coming back, after the backoff's wait. False once it has tried for long enough.
    fn retry_later(&mut self, cx: &mut Context<Self>) -> bool {
        self.backoff.dropped(Instant::now());
        let Some(wait) = self.backoff.wait() else {
            self.reconnect = None;
            return false;
        };
        let r = self.reconnect.get_or_insert_with(Reconnect::default);
        r.wait_id += 1;
        let g = r.wait_id;
        self.epoch += 1;
        self.drop_conn();
        self.phase = Phase::Ticket;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(wait).await;
            let _ = this.update(cx, |t, cx| {
                if t.reconnect.as_ref().is_some_and(|r| r.wait_id == g) {
                    t.dial_again(cx);
                }
            });
        })
        .detach();
        true
    }

    /// "Reconnect now" while it is coming back by itself: skip the rest of the wait.
    fn reconnect_now(&mut self, cx: &mut Context<Self>) {
        if let Some(r) = self.reconnect.as_mut() {
            r.wait_id += 1;
            self.dial_again(cx);
        }
    }

    fn dial(&mut self, ticket: &TermTicket, cx: &mut Context<Self>) {
        let (cols, rows) = self.want.unwrap_or(self.size);
        self.hub_sizes = false;
        self.owner = true;
        self.in_snapshot = false;
        self.apply_size(false);
        // coming back with a screen: keep this view's own scrollback, the hub sends only the screen
        let conn = self.app.read(cx).core.term_connect(ticket, cols, rows, !self.has_output);
        self.conn = Some(conn.tx);
        self.fresh = true;
        self.phase = Phase::Dial;
        self.read(conn.rx, self.epoch, cx);
    }

    /// Reads the hub's events in batches and parses them a slice at a time (`PARSE_BUDGET`), drawing frames in between;
    /// it reads no more until what it has is parsed, so a flood waits in the bounded channel, then the hub.
    fn read(&mut self, rx: Receiver<FromHub>, epoch: u64, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            while let Ok(first) = rx.recv().await {
                let mut batch = vec![first];
                while batch.len() < 512 {
                    match rx.try_recv() {
                        Ok(more) => batch.push(more),
                        Err(_) => break,
                    }
                }
                let last = matches!(batch.last(), Some(FromHub::Closed(_)));
                let mut more = match this.update(cx, |t, cx| t.on_hub(epoch, batch, cx)) {
                    Ok(m) => m,
                    Err(_) => break,
                };
                while more {
                    cx.background_executor().timer(PARSE_BUDGET).await;
                    more = match this.update(cx, |t, cx| t.work(epoch, cx)) {
                        Ok(m) => m,
                        Err(_) => return,
                    };
                }
                if last {
                    break;
                }
            }
        })
        .detach();
    }

    /// Queues the hub's events and parses what fits in this slice. True if some are left for the next.
    fn on_hub(&mut self, epoch: u64, batch: Vec<FromHub>, cx: &mut Context<Self>) -> bool {
        if epoch != self.epoch {
            return false;
        }
        for ev in batch {
            match ev {
                FromHub::Data(b) if b.len() > PARSE_PIECE => {
                    self.pending.extend(b.chunks(PARSE_PIECE).map(|p| FromHub::Data(p.to_vec())));
                }
                ev => self.pending.push_back(ev),
            }
        }
        self.work(epoch, cx)
    }

    /// Parses queued events for up to `PARSE_BUDGET`. True if some are left.
    fn work(&mut self, epoch: u64, cx: &mut Context<Self>) -> bool {
        if epoch != self.epoch {
            // an old connection's leftovers (a new one cleared them, and may have queued its own)
            return false;
        }
        let start = Instant::now();
        let before = self.phase.clone();
        while start.elapsed() < PARSE_BUDGET {
            let Some(ev) = self.pending.pop_front() else { break };
            match ev {
                FromHub::Connected => {
                    if self.phase == Phase::Dial {
                        self.phase = Phase::Hub;
                    }
                }
                FromHub::Status(s) => {
                    if s == "open" {
                        self.opened();
                    } else if self.phase == Phase::Dial {
                        self.phase = Phase::Hub;
                    }
                }
                FromHub::Size { cols, rows } => {
                    // the terminal's size from here on in the stream: what follows was written for it
                    self.hub_sizes = true;
                    self.resize_grid(cols, rows);
                }
                FromHub::Owner(o) => self.owner = o,
                FromHub::SnapshotBegin => {
                    // the screen follows, for a clean grid: this view's scrollback stays above it
                    self.rebuild();
                    self.in_snapshot = true;
                    self.quiet_until = self.quiet_until.max(AppState::now() + 1.0);
                }
                FromHub::SnapshotEnd => self.in_snapshot = false,
                FromHub::Data(bytes) => {
                    if self.fresh {
                        // A new connection redraws everything: start from a clean screen, the scrollback kept.
                        self.rebuild();
                    }
                    if self.phase != Phase::Open {
                        self.opened();
                    }
                    let now = AppState::now();
                    if !self.in_snapshot && now >= self.quiet_until && now - self.last_key.get() > 0.6 {
                        self.last_out = Some(now);
                    }
                    crate::perf::bytes(bytes.len());
                    crate::perf::time(crate::perf::Kind::Hub, || self.parser.advance(&mut self.term, &bytes));
                    self.has_output = true;
                }
                FromHub::Exit { code, note } if self.local.is_some() => {
                    self.exit_code = Some(code);
                    self.exit_note = note;
                }
                FromHub::Exit { code, note } => {
                    self.exit_code = Some(code);
                    self.exit_note = Some(note.filter(|n| !n.trim().is_empty()).unwrap_or_else(|| match code {
                        Some(c) if c != 0 => format!("claude attach ended with code {c}. The chat may have stopped."),
                        _ => "claude attach ended. Reconnect to attach again.".into(),
                    }));
                }
                FromHub::Error(e) => self.exit_note = Some(e),
                FromHub::Closed(c) => {
                    self.conn = None;
                    self.mouse_held = None;
                    self.pending.clear();
                    let exit = self.exit_code.take();
                    if self.local.is_some() {
                        // A shell that exits cleanly (exit, Ctrl-D) takes its tab with it; an error keeps the last
                        // output with Restart.
                        self.pty = None;
                        self.leaving = false;
                        self.quit_key = None;
                        let note = self.exit_note.take();
                        if exit_closes_tab(exit) {
                            self.phase = Phase::Ended { note: String::new(), retry: true, code: None };
                            cx.emit(TermEvent::Close);
                            continue;
                        }
                        let code = exit.flatten().unwrap_or(1);
                        let note = note
                            .unwrap_or_else(|| format!("The shell exited with code {code}. The last output is above."));
                        self.phase = Phase::Ended { note, retry: true, code: Some(code) };
                        continue;
                    }
                    let asked = end_asked(std::mem::take(&mut self.leaving), self.quit_key.take(), Instant::now());
                    let attached = self.has_output || self.reconnect.is_some();
                    if !asked && reconnect::after_close(&c, exit, attached) && self.retry_later(cx) {
                        // the link dropped: back by itself, the screen kept
                        self.exit_note = None;
                        continue;
                    }
                    if (exit_closes_tab(exit) && asked) || left_chat(exit) {
                        // `exit`, `/exit` or Ctrl-D: the process is done, so the tab goes, as in a real terminal. A
                        // clean end nobody asked for (attached elsewhere, a restart) keeps the tab, with Reconnect.
                        self.exit_note = None;
                        self.phase = Phase::Ended { note: String::new(), retry: true, code: None };
                        cx.emit(TermEvent::Close);
                        continue;
                    }
                    let code = exit.flatten().filter(|c| *c != 0 && *c != 255);
                    let note = match code {
                        Some(c) => format!("Process exited (code {c}). The last output is above."),
                        None => self.exit_note.take().unwrap_or_else(|| c.message()),
                    };
                    self.exit_note = None;
                    self.phase = Phase::Ended { note, retry: !matches!(c, Closed::Origin), code };
                }
            }
        }
        self.drain(cx);
        self.arm_sync(cx);
        if self.phase != before {
            cx.emit(TermEvent::Changed);
        }
        cx.notify();
        !self.pending.is_empty()
    }

    /// A clean grid at the current size for a new screen, with this view's scrollback carried over.
    fn rebuild(&mut self) {
        let mut fresh = new_term(self.size.0, self.size.1, &self.events);
        if self.has_output {
            sb_core::term::screen::carry_history(&mut self.term, &mut fresh);
        }
        self.term = fresh;
        self.parser = Processor::new();
        self.fresh = false;
    }

    /// The grid takes the size the hub says the terminal has now.
    fn resize_grid(&mut self, cols: u16, rows: u16) {
        if (cols, rows) == self.size || cols == 0 || rows == 0 {
            return;
        }
        self.size = (cols, rows);
        self.quiet_until = self.quiet_until.max(AppState::now() + 1.0);
        self.term.resize(GridSize { cols: cols as usize, rows: rows as usize });
    }

    fn opened(&mut self) {
        self.phase = Phase::Open;
        self.reconnect = None;
        self.backoff.opened(Instant::now());
        self.quiet_until = AppState::now() + 2.0;
        self.flags_asked = false;
        // The view may have changed size while connecting, and a resize sent before the hub opened may be lost:
        // say the size once more now (to a hub that sends sizes, as what this view would like).
        if !self.hub_sizes {
            self.apply_size(false);
        }
        if let Some(tx) = &self.conn {
            let (cols, rows) = if self.hub_sizes { self.want.unwrap_or(self.size) } else { self.size };
            let _ = tx.try_send(ToHub::Resize { cols, rows });
            if self.focused {
                let _ = tx.try_send(ToHub::Focus);
            }
        }
    }

    /// Redraw (Cmd+Shift+R, the Terminal menu): the hub sends the screen again, and this view draws it on a clean
    /// grid. A screen that looks wrong (leftover letters in the status line) comes right.
    pub fn redraw(&mut self, cx: &mut Context<Self>) {
        let Some(tx) = self.conn.as_ref().filter(|_| self.phase == Phase::Open) else { return };
        let _ = tx.try_send(ToHub::Redraw);
        self.quiet_until = self.quiet_until.max(AppState::now() + 1.0);
        cx.notify();
    }

    /// This view has focus (or was clicked): the shared terminal takes its size, and it answers the program.
    fn take_size(&mut self) {
        if !self.hub_sizes || self.owner {
            return;
        }
        if let Some(tx) = self.conn.as_ref().filter(|_| self.phase == Phase::Open) {
            let _ = tx.try_send(ToHub::Focus);
            // the hub makes this view the owner for this; say so here at once, so the focus report goes out
            self.owner = true;
        }
    }

    fn drop_conn(&mut self) {
        if let Some(tx) = self.conn.take() {
            let _ = tx.try_send(ToHub::Close);
        }
    }

    /// Detach: close the connection. The chat keeps running on its machine.
    pub fn detach(&mut self) {
        self.epoch += 1;
        self.reconnect = None;
        self.drop_conn();
    }

    /// Resume and open: the resume job first; `on_app` asks for the ticket when it ends.
    pub fn resume(&mut self, cx: &mut Context<Self>) {
        let Some(key) = self.key.clone() else { return };
        self.epoch += 1;
        self.reconnect = None;
        self.drop_conn();
        self.phase = Phase::Job(String::new());
        let epoch = self.epoch;
        let weak = cx.entity().downgrade();
        self.app.update(cx, |s, cx| {
            s.resume_for_term(
                &key,
                move |res, cx| {
                    let _ = weak.update(cx, |t, cx| {
                        if t.epoch != epoch {
                            return;
                        }
                        t.phase = match res {
                            Ok(jid) => Phase::Job(jid),
                            Err(e) => Phase::Failed(e),
                        };
                        t.changed(cx);
                    });
                },
                cx,
            )
        });
        self.changed(cx);
    }

    /// A relaunch of this chat started: wait for its job, then attach to the chat's new background session.
    pub fn follow_job(&mut self, job: String, cx: &mut Context<Self>) {
        self.epoch += 1;
        self.reconnect = None;
        self.drop_conn();
        self.phase = Phase::Job(job);
        self.changed(cx);
    }

    /// Reads what the chat runs with, for the bar's flags summary.
    pub fn load_flags(&mut self, cx: &mut Context<Self>) {
        let (Some(sid), m) = (self.sid.clone(), self.machine.clone()) else { return };
        let core = self.app.read(cx).core.clone();
        cx.spawn(async move |this, cx| {
            let res = cx.background_executor().spawn(async move { core.flags_view(&m, &sid) }).await;
            if let Ok(v) = res {
                let _ = this.update(cx, |t, cx| {
                    t.flags_sum = Some(v.summary).filter(|s| !s.is_empty());
                    cx.notify();
                });
            }
        })
        .detach();
    }

    /// Attached and ready to type into.
    pub fn live(&self) -> bool {
        self.phase == Phase::Open
    }

    /// Types slash commands into the chat one at a time, as if from the keyboard (the flags drawer's live changes).
    pub fn type_cmds(&mut self, cmds: Vec<String>, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            for c in cmds {
                if this.update(cx, |t, _| t.send(c.clone().into_bytes())).is_err() {
                    return;
                }
                cx.background_executor().timer(Duration::from_millis(160)).await;
                let _ = this.update(cx, |t, _| t.send(b"\r".to_vec()));
                cx.background_executor().timer(Duration::from_millis(900)).await;
            }
        })
        .detach();
    }

    /// Pastes `text` (a bracketed paste, no Enter) once the tab is attached.
    pub fn paste_when_open(&mut self, text: String, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            for _ in 0..120 {
                match this.update(cx, |t, _| t.live()) {
                    Ok(true) => break,
                    Ok(false) => cx.background_executor().timer(Duration::from_millis(500)).await,
                    Err(_) => return,
                }
            }
            let _ = this.update(cx, |t, cx| {
                let bracketed = t.term.mode().contains(TermMode::BRACKETED_PASTE);
                t.typed(keys::paste(&text, bracketed), cx);
            });
        })
        .detach();
    }

    /// Types them once the tab is attached (it may still be connecting), giving up after a minute.
    pub fn type_when_open(&mut self, cmds: Vec<String>, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            for _ in 0..120 {
                match this.update(cx, |t, _| t.live()) {
                    Ok(true) => break,
                    Ok(false) => cx.background_executor().timer(Duration::from_millis(500)).await,
                    Err(_) => return,
                }
            }
            // Claude Code draws its prompt a moment after the attach.
            cx.background_executor().timer(Duration::from_millis(1200)).await;
            let _ = this.update(cx, |t, cx| t.type_cmds(cmds, cx));
        })
        .detach();
    }

    fn on_app(&mut self, ev: &AppEvent, cx: &mut Context<Self>) {
        let Phase::Job(id) = &self.phase else { return };
        match ev {
            AppEvent::JobDone { id: done, machine, sid, bg } if done == id => {
                if let Some(m) = machine {
                    self.machine = m.clone();
                }
                if sid.is_some() {
                    self.sid = sid.clone();
                }
                // A job that reported no background id leaves an ordinary chat: an older one is not asked for again.
                self.prefer_bg = prefer_bg_after(bg.as_deref());
                if bg.is_some() {
                    self.bg = bg.clone();
                }
                if let Some(s) = &self.sid {
                    self.key = Some(format!("{}:{}", self.machine, s));
                }
                if self.label.is_empty() {
                    self.label = self.app.read(cx).snap.label_of(&self.machine).unwrap_or_else(|| self.machine.clone());
                }
                self.connect(cx);
            }
            AppEvent::JobFailed { id: failed, error } if failed == id => {
                self.phase = Phase::Failed(error.clone());
                self.changed(cx);
            }
            _ => {}
        }
    }

    /// What Enter (or the card's main button) does when the terminal is not attached.
    fn primary(&mut self, cx: &mut Context<Self>) {
        if self.reconnect.is_some() {
            return self.reconnect_now(cx);
        }
        match &self.phase {
            Phase::Ended { retry: true, .. } | Phase::Failed(_) | Phase::Need { need: Need::Offline, .. } => {
                self.connect(cx)
            }
            Phase::Need { need: Need::Resume, .. } => self.resume(cx),
            Phase::Need { need: Need::Terminal, .. } => {
                if let Some(u) = self.url.clone() {
                    crate::state::open_link(cx, &u);
                }
            }
            _ => {}
        }
    }

    // ---------- size ----------

    /// From the canvas, before each frame: the cell metrics and the room there is.
    fn layout(&mut self, m: Metrics, b: Bounds<Pixels>, cx: &mut Context<Self>) {
        self.geom = Some((m, b));
        let (c, r) = m.grid_for(b);
        let want = (c.min(1000) as u16, r.min(500) as u16);
        if self.want == Some(want) {
            return;
        }
        let first = self.want.is_none();
        self.want = Some(want);
        match size_step(self.hub_sizes, self.phase == Phase::Open, first) {
            SizeStep::Now => self.apply_size(true),
            step => {
                // Only once the window stops moving, so a drag does not reflow the program on every frame.
                self.resize_gen += 1;
                let g = self.resize_gen;
                cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(Duration::from_millis(120)).await;
                    let _ = this.update(cx, |t, cx| {
                        if t.resize_gen != g {
                            return;
                        }
                        if step == SizeStep::Ask {
                            if let (Some((cols, rows)), Some(tx)) = (t.want, t.conn.as_ref()) {
                                let _ = tx.try_send(ToHub::Resize { cols, rows });
                            }
                        } else {
                            t.apply_size(true);
                            cx.notify();
                        }
                    });
                })
                .detach();
            }
        }
    }

    fn apply_size(&mut self, send: bool) {
        let Some((c, r)) = self.want else { return };
        if (c, r) == self.size {
            return;
        }
        self.size = (c, r);
        self.quiet_until = self.quiet_until.max(AppState::now() + 1.0);
        self.term.resize(GridSize { cols: c as usize, rows: r as usize });
        if send && let Some(tx) = &self.conn {
            let _ = tx.try_send(ToHub::Resize { cols: c, rows: r });
        }
    }

    // ---------- output side ----------

    /// Handles what the terminal asked for while parsing.
    fn drain(&mut self, cx: &mut Context<Self>) {
        let evs: Vec<TermEv> = std::mem::take(&mut *self.events.0.borrow_mut());
        for e in evs {
            // Another window sets the size and answers for the shared terminal; two answers would reach the program.
            let answers = self.owner;
            match e {
                TermEv::PtyWrite(_) | TermEv::ColorRequest(..) | TermEv::TextAreaSizeRequest(_) if !answers => {}
                TermEv::PtyWrite(s) => self.send(s.into_bytes()),
                TermEv::ColorRequest(i, f) => {
                    let pal = colors::current(cx);
                    let c = colors::to_rgb(pal.index(i, self.term.colors()));
                    self.send(f(c).into_bytes());
                }
                TermEv::TextAreaSizeRequest(f) => {
                    let (cw, lh) = self.geom.map(|(m, _)| (m.cell_w, m.line_h)).unwrap_or((8., 18.));
                    let ws = WindowSize {
                        num_lines: self.size.1,
                        num_cols: self.size.0,
                        cell_width: cw as u16,
                        cell_height: lh as u16,
                    };
                    self.send(f(ws).into_bytes());
                }
                TermEv::ClipboardStore(_, text) => crate::state::copy_text(cx, text),
                // A local shell names its tab (the folder, the program running); a chat's tab keeps the chat's name.
                TermEv::Title(s) if self.local.is_some() => {
                    let s = s.trim().to_string();
                    let name = self.local.as_ref().map(|p| p.name.clone()).unwrap_or_default();
                    let next = if s.is_empty() { name } else { s };
                    if next != self.title {
                        self.title = next;
                        cx.emit(TermEvent::Changed);
                    }
                }
                TermEv::ResetTitle if self.local.is_some() => {
                    if let Some(p) = &self.local
                        && p.name != self.title
                    {
                        self.title = p.name.clone();
                        cx.emit(TermEvent::Changed);
                    }
                }
                TermEv::Bell if !self.focused && !self.bell => {
                    self.bell = true;
                    cx.emit(TermEvent::Changed);
                }
                _ => {}
            }
        }
    }

    /// A program in a synchronized update (Claude Code draws in them) that never ends one: flush at the timeout.
    fn arm_sync(&mut self, cx: &mut Context<Self>) {
        if self.sync_armed {
            return;
        }
        let Some(at) = self.parser.sync_timeout().sync_timeout() else { return };
        self.sync_armed = true;
        cx.spawn(async move |this, cx| {
            let mut at = at;
            loop {
                cx.background_executor().timer(at.saturating_duration_since(Instant::now())).await;
                let next = this.update(cx, |t, cx| match t.parser.sync_timeout().sync_timeout() {
                    Some(a) if a > Instant::now() => Some(a),
                    Some(_) => {
                        t.parser.stop_sync(&mut t.term);
                        t.drain(cx);
                        t.sync_armed = false;
                        cx.notify();
                        None
                    }
                    None => {
                        t.sync_armed = false;
                        None
                    }
                });
                match next {
                    Ok(Some(a)) => at = a,
                    _ => break,
                }
            }
        })
        .detach();
    }

    // ---------- input side ----------

    fn send(&self, bytes: Vec<u8>) {
        self.last_key.set(AppState::now());
        if let Some(tx) = &self.conn {
            let _ = tx.try_send(ToHub::Input(bytes));
        }
    }

    /// Typing: back to the bottom, selection gone, bytes out.
    pub(crate) fn typed(&mut self, bytes: Vec<u8>, cx: &mut Context<Self>) {
        // Typing shows the cursor at once, then the blink carries on.
        self.blink_on = true;
        if self.term.grid().display_offset() != 0 {
            self.term.scroll_display(Scroll::Bottom);
        }
        self.term.selection = None;
        self.send(bytes);
        cx.notify();
    }

    fn on_key(&mut self, ev: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let k = &ev.keystroke;
        let m = &k.modifiers;
        if self.colors_open && k.key == "escape" {
            // the scheme under the pointer was only shown: put back the chosen one
            colors::set_preview(None, cx);
            self.colors_open = false;
            cx.notify();
            cx.stop_propagation();
            return;
        }
        // Ctrl+C with text selected copies it, as in Windows Terminal; the selection goes, so the next one interrupts.
        if m.control && !m.shift && !m.alt && !m.platform && k.key == "c" && self.copy_selection(true, cx) {
            cx.stop_propagation();
            return;
        }
        if self.phase != Phase::Open {
            if k.key == "enter" && !m.modified() {
                self.primary(cx);
                cx.stop_propagation();
            } else if !MAC && m.control && !m.shift && !m.alt && k.key == "w" {
                cx.emit(TermEvent::Close);
                cx.stop_propagation();
            }
            return;
        }
        if self.marked.is_some() {
            return;
        }
        // Native text-editing keys first (Cmd+Left to the line start, Option+Backspace a word, Cmd+Up to the top).
        let all = Mods { ctrl: m.control, alt: m.alt, shift: m.shift, platform: m.platform };
        let alt_screen = self.term.mode().contains(TermMode::ALT_SCREEN);
        if let Some(e) = keys::edit_key(&k.key, all, MAC, alt_screen) {
            self.line.unknown();
            match e {
                keys::Edit::Send(bytes) => self.typed(bytes, cx),
                keys::Edit::ScrollTop => {
                    self.term.scroll_display(Scroll::Top);
                    cx.notify();
                }
                keys::Edit::ScrollBottom => {
                    self.term.scroll_display(Scroll::Bottom);
                    cx.notify();
                }
            }
            cx.stop_propagation();
            return;
        }
        // Cmd is the app's; AltGr and an input method composing want the character, not a key.
        if m.platform || ev.prefer_character_input {
            return;
        }
        let mods = Mods { ctrl: m.control, alt: m.alt, shift: m.shift, platform: false };
        let app_cursor = self.term.mode().contains(TermMode::APP_CURSOR);
        if let Some(bytes) = keys::encode(&k.key, mods, app_cursor) {
            if self.local.is_some() {
                // A shell's own exit ends it; nothing to leave.
            } else if bytes == b"\r" && !m.modified() {
                // "exit" (or /exit, quit, /quit) and Enter: leave instead (clear the box, then Ctrl+Z).
                if let Some(n) = self.line.enter() {
                    self.leaving = true;
                    self.typed(exitline::leave_bytes(n), cx);
                    cx.stop_propagation();
                    return;
                }
            } else {
                self.line.key(&bytes);
            }
            // A leave that was sent and then typed over did not happen.
            self.leaving = false;
            self.quit_key = (bytes == [0x04] || bytes == [0x1a]).then(Instant::now);
            self.typed(bytes, cx);
            cx.stop_propagation();
        }
    }

    fn on_copy(&mut self, _: &TermCopy, _: &mut Window, cx: &mut Context<Self>) {
        self.copy_selection(false, cx);
    }

    /// Puts the selected text on the clipboard, and with `clear` lets go of the selection. False if none is selected.
    fn copy_selection(&mut self, clear: bool, cx: &mut Context<Self>) -> bool {
        let Some(s) = self.term.selection_to_string().filter(|s| !s.is_empty()) else { return false };
        crate::state::copy_text(cx, s);
        if clear {
            self.term.selection = None;
            cx.notify();
        }
        true
    }

    fn on_paste(&mut self, _: &TermPaste, _: &mut Window, cx: &mut Context<Self>) {
        self.paste(cx);
    }

    fn paste(&mut self, cx: &mut Context<Self>) {
        if self.phase != Phase::Open {
            return;
        }
        // An image or files on the clipboard are sent to the chat's machine, then their path is pasted.
        #[cfg(not(feature = "shots"))]
        if self.local.is_none()
            && let Some(item) = cx.read_from_clipboard()
        {
            for e in item.entries() {
                match e {
                    gpui::ClipboardEntry::Image(img) => {
                        let ext = format!("{:?}", img.format).to_lowercase();
                        let name = format!("pasted-image.{}", if ext == "jpeg" { "jpg" } else { ext.as_str() });
                        return self.send_bytes(name, img.bytes.clone(), cx);
                    }
                    gpui::ClipboardEntry::ExternalPaths(p) => return self.send_files(p.paths().to_vec(), cx),
                    _ => {}
                }
            }
        }
        if let Some(text) = clipboard_text(cx).filter(|t| !t.is_empty()) {
            let bracketed = self.term.mode().contains(TermMode::BRACKETED_PASTE);
            self.line.unknown();
            self.typed(keys::paste(&text, bracketed), cx);
        }
    }

    fn page(&mut self, up: bool, cx: &mut Context<Self>) {
        self.term.scroll_display(if up { Scroll::PageUp } else { Scroll::PageDown });
        cx.notify();
    }

    // ---------- mouse ----------

    /// The cell under `pos` (viewport line, column) and which half of it.
    fn cell_at(&self, pos: gpui::Point<Pixels>) -> Option<(usize, usize, Side)> {
        let (m, b) = self.geom?;
        let o = m.origin(b);
        let x = f32::from(pos.x - o.x);
        let y = f32::from(pos.y - o.y);
        let cols = self.term.columns();
        let rows = self.term.screen_lines();
        let fx = (x / m.cell_w).max(0.);
        let col = (fx.floor() as usize).min(cols - 1);
        let side = if (fx.floor() as usize) >= cols || fx.fract() > 0.5 { Side::Right } else { Side::Left };
        let line = (((y / m.line_h).max(0.)).floor() as usize).min(rows - 1);
        Some((line, col, side))
    }

    fn reporting(&self, mods: &Modifiers) -> bool {
        self.phase == Phase::Open && self.term.mode().intersects(TermMode::MOUSE_MODE) && !mods.shift
    }

    /// Mouse reporting, SGR when the program asked for it, the classic encoding otherwise.
    fn report(&self, button: u8, line: usize, col: usize, press: bool, motion: bool, mods: &Modifiers) {
        if let Some(bytes) = self.encode_mouse(button, line, col, press, motion, mods) {
            self.send(bytes);
        }
    }

    fn encode_mouse(
        &self,
        button: u8,
        line: usize,
        col: usize,
        press: bool,
        motion: bool,
        mods: &Modifiers,
    ) -> Option<Vec<u8>> {
        let mut b = button;
        if mods.shift {
            b += 4;
        }
        if mods.alt {
            b += 8;
        }
        if mods.control {
            b += 16;
        }
        if motion {
            b += 32;
        }
        let (x, y) = (col + 1, line + 1);
        let bytes = if self.term.mode().contains(TermMode::SGR_MOUSE) {
            format!("\x1b[<{b};{x};{y}{}", if press { 'M' } else { 'm' }).into_bytes()
        } else {
            if x > 222 || y > 222 {
                return None;
            }
            let b = if press { b } else { (b & !3) | 3 };
            vec![0x1b, b'[', b'M', 32 + b, 32 + x as u8, 32 + y as u8]
        };
        Some(bytes)
    }

    fn mouse_down(&mut self, ev: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
        self.take_size();
        self.line.unknown();
        let Some((line, col, side)) = self.cell_at(ev.position) else { return };
        self.link_press = None;
        if ev.button == MouseButton::Left
            && let Some(f) = self
                .hover
                .as_ref()
                .filter(|(l, f)| *l == line && (f.start..f.end).contains(&col))
                .map(|(_, f)| f.clone())
        {
            let cmd = if MAC { ev.modifiers.platform } else { ev.modifiers.control };
            if cmd {
                self.open_target(f.target, cx);
                return;
            }
            if !self.reporting(&ev.modifiers) && ev.click_count <= 1 {
                self.link_press = Some((line, col));
            }
        }
        if self.reporting(&ev.modifiers) {
            let b = match ev.button {
                MouseButton::Left => 0,
                MouseButton::Middle => 1,
                MouseButton::Right => 2,
                _ => return,
            };
            self.report(b, line, col, true, false, &ev.modifiers);
            self.mouse_held = Some(b);
            self.last_cell = Some((line, col));
            return;
        }
        // Right-click copies the selection, or pastes when nothing is selected (as in Windows Terminal and PuTTY).
        if ev.button == MouseButton::Right {
            if !self.copy_selection(true, cx) {
                self.paste(cx);
            }
            return;
        }
        if ev.button != MouseButton::Left {
            return;
        }
        let ty = match ev.click_count {
            0 | 1 => SelectionType::Simple,
            2 => SelectionType::Semantic,
            _ => SelectionType::Lines,
        };
        let p = viewport_to_point(self.term.grid().display_offset(), GridPoint::new(line, Column(col)));
        self.term.selection = Some(Selection::new(ty, p, side));
        self.selecting = true;
        cx.notify();
    }

    fn mouse_move(&mut self, ev: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        let Some((line, col, side)) = self.cell_at(ev.position) else {
            if self.hover.take().is_some() {
                cx.notify();
            }
            return;
        };
        let hover = if ev.pressed_button.is_none() && !self.selecting && self.mouse_held.is_none() {
            self.link_at(line, col).map(|f| (line, f))
        } else {
            None
        };
        if hover != self.hover {
            self.hover = hover;
            cx.notify();
        }
        if let Some(b) = self.mouse_held {
            if self.term.mode().intersects(TermMode::MOUSE_DRAG | TermMode::MOUSE_MOTION)
                && self.last_cell != Some((line, col))
            {
                self.report(b, line, col, true, true, &ev.modifiers);
                self.last_cell = Some((line, col));
            }
            return;
        }
        if self.selecting && ev.pressed_button == Some(MouseButton::Left) {
            let p = viewport_to_point(self.term.grid().display_offset(), GridPoint::new(line, Column(col)));
            if let Some(s) = self.term.selection.as_mut() {
                s.update(p, side);
            }
            cx.notify();
        } else if ev.pressed_button.is_none()
            && self.reporting(&ev.modifiers)
            && self.term.mode().contains(TermMode::MOUSE_MOTION)
            && self.last_cell != Some((line, col))
        {
            self.report(3, line, col, true, true, &ev.modifiers);
            self.last_cell = Some((line, col));
        }
    }

    fn mouse_up(&mut self, pos: gpui::Point<Pixels>, mods: &Modifiers, cx: &mut Context<Self>) {
        if let Some(b) = self.mouse_held.take() {
            if let Some((line, col, _)) = self.cell_at(pos) {
                self.report(b, line, col, false, false, mods);
            }
            return;
        }
        let press = self.link_press.take();
        if self.selecting {
            self.selecting = false;
            if self.term.selection.as_ref().is_some_and(|s| s.is_empty()) {
                self.term.selection = None;
                // a click, not a drag, on a link: open it
                let here = self.cell_at(pos).map(|(l, c, _)| (l, c));
                if let (Some(p), Some((l, f))) = (press, self.hover.clone())
                    && here == Some(p)
                    && l == p.0
                    && (f.start..f.end).contains(&p.1)
                {
                    self.open_target(f.target, cx);
                }
            } else {
                // Selecting copies: the text is on the clipboard as soon as the mouse lets go.
                self.copy_selection(false, cx);
            }
            cx.notify();
        }
    }

    /// The link at a cell: one the program set (OSC 8), else a web address or file path in the row's text.
    fn link_at(&self, line: usize, col: usize) -> Option<links::Found> {
        let grid = self.term.grid();
        let off = grid.display_offset();
        let cols = self.term.columns();
        if line >= self.term.screen_lines() || col >= cols {
            return None;
        }
        let at = |c: usize| &grid[viewport_to_point(off, GridPoint::new(line, Column(c)))];
        if let Some(h) = at(col).hyperlink() {
            let same = |c: usize| at(c).hyperlink().as_ref() == Some(&h);
            let mut s = col;
            while s > 0 && same(s - 1) {
                s -= 1;
            }
            let mut e = col + 1;
            while e < cols && same(e) {
                e += 1;
            }
            return Some(links::Found { start: s, end: e, target: links::Target::Url(h.uri().to_string()) });
        }
        let cells: Vec<char> = (0..cols)
            .map(|c| {
                let cell = at(c);
                if cell.flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER) {
                    '\0'
                } else {
                    cell.c
                }
            })
            .collect();
        links::find(&cells, col)
    }

    /// Web links open in the browser. Paths live on the chat's machine and other kinds of link could start any app,
    /// so those are copied instead.
    fn open_target(&mut self, target: links::Target, cx: &mut Context<Self>) {
        let on = if self.label.is_empty() { self.machine.clone() } else { self.label.clone() };
        let (title, sub, text) = match target {
            links::Target::Url(u) if web(&u) => {
                crate::state::open_link(cx, &u);
                return;
            }
            links::Target::Url(u) => ("Link copied", "It's not a web page, so it was copied instead.".to_string(), u),
            links::Target::Path(p) => ("Path copied", format!("The file is on {on}."), p),
        };
        crate::state::copy_text(cx, text);
        self.app.update(cx, |s, cx| s.toast(title, Some(sub), false, cx));
    }

    /// The tip under a hovered link: what a click does, and where it goes.
    fn link_tip(&self, t: &Theme, mouse_mode: bool) -> Option<AnyElement> {
        let (line, f) = self.hover.as_ref()?;
        let (m, b) = self.geom?;
        let o = m.origin(b);
        let x = f32::from(o.x - b.origin.x) + f.start as f32 * m.cell_w;
        let below = (*line + 2) as f32 * m.line_h + 30. < f32::from(b.size.height);
        let y = f32::from(o.y - b.origin.y)
            + if below { (*line + 1) as f32 * m.line_h + 4. } else { *line as f32 * m.line_h - 26. };
        let click = if mouse_mode { if MAC { "⌘-click" } else { "Ctrl-click" } } else { "Click" };
        let (verb, what) = match &f.target {
            links::Target::Url(u) if web(u) => ("to open", u.clone()),
            links::Target::Url(u) => ("to copy", u.clone()),
            links::Target::Path(p) => ("to copy the path", p.clone()),
        };
        let what =
            if what.chars().count() > 72 { format!("{}…", what.chars().take(71).collect::<String>()) } else { what };
        Some(
            div()
                .absolute()
                .left(px(x.max(4.)))
                .top(px(y.max(2.)))
                .max_w(px((f32::from(b.size.width) - x - 12.).max(220.)))
                .px(px(8.))
                .py(px(3.))
                .rounded(px(ui::R_ITEM))
                .bg(t.bg)
                .border_1()
                .border_color(t.line)
                .shadow(vec![gpui::BoxShadow {
                    color: gpui::black().opacity(if t.dark { 0.4 } else { 0.12 }),
                    offset: gpui::point(px(0.), px(2.)),
                    blur_radius: px(8.),
                    spread_radius: px(0.),
                    inset: false,
                }])
                .flex()
                .items_center()
                .gap(px(6.))
                .text_size(px(11.5))
                .child(
                    div()
                        .flex_none()
                        .text_color(t.ink)
                        .font_weight(gpui::FontWeight::BOLD)
                        .child(format!("{click} {verb}")),
                )
                .child(
                    div().min_w_0().overflow_hidden().text_ellipsis().whitespace_nowrap().text_color(t.dim).child(what),
                )
                .into_any_element(),
        )
    }

    fn wheel(&mut self, ev: &ScrollWheelEvent, _: &mut Window, cx: &mut Context<Self>) {
        crate::perf::wheel();
        let lh = self.geom.map(|(m, _)| m.line_h).unwrap_or(18.);
        let dy = f32::from(ev.delta.pixel_delta(px(lh)).y);
        self.scroll_px += dy;
        let lines = (self.scroll_px / lh).trunc() as i32;
        if lines == 0 {
            return;
        }
        self.scroll_px -= lines as f32 * lh;
        self.hover = None;
        let mode = *self.term.mode();
        let up = lines > 0;
        let n = lines.unsigned_abs().min(50);
        if self.reporting(&ev.modifiers) {
            // one message for the whole flick, not one per line: each is a trip through the hub to the program
            if let Some((line, col, _)) = self.cell_at(ev.position)
                && let Some(one) = self.encode_mouse(if up { 64 } else { 65 }, line, col, true, false, &ev.modifiers)
            {
                self.send(one.repeat(n as usize));
            }
        } else if self.phase == Phase::Open
            && mode.contains(TermMode::ALT_SCREEN | TermMode::ALTERNATE_SCROLL)
            && !ev.modifiers.shift
        {
            // A full-screen program without mouse reporting (less, man): the wheel is the arrow keys.
            let one: &[u8] = match (up, mode.contains(TermMode::APP_CURSOR)) {
                (true, true) => b"\x1bOA",
                (true, false) => b"\x1b[A",
                (false, true) => b"\x1bOB",
                (false, false) => b"\x1b[B",
            };
            self.send(one.repeat(n as usize));
        } else {
            self.term.scroll_display(Scroll::Delta(lines));
            cx.notify();
        }
    }

    // ---------- drawing ----------

    /// Shown while scrolled back into the history: how far, and a way down (typing also returns).
    fn back_pill(&self, t: &Theme, back: usize, cx: &mut Context<Self>) -> impl IntoElement {
        let lines = if back == 1 { "1 line up".to_string() } else { format!("{back} lines up") };
        div().absolute().right(px(18.)).bottom(px(12.)).child(
            div()
                .id("term-back")
                .h(px(28.))
                .px(px(12.))
                .flex()
                .items_center()
                .gap(px(8.))
                .rounded(px(ui::R_SHEET))
                .border_1()
                .border_color(t.line2)
                .bg(t.panel2)
                .shadow(t.drop())
                .cursor_pointer()
                .hover(|d| d.border_color(t.accent))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(|this, _, _, cx| {
                    this.term.scroll_display(Scroll::Bottom);
                    cx.notify();
                }))
                .child(div().text_size(px(12.)).text_color(t.dim).child(lines))
                .child(div().size(px(3.)).rounded_full().bg(t.faint))
                .child(div().text_size(px(12.)).text_color(t.accent).child("Back to the bottom"))
                .child(ui::icon("down", 12., t.accent)),
        )
    }

    fn card(&self, t: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        let label = if self.label.is_empty() { self.machine.clone() } else { self.label.clone() };
        if self.reconnect.is_some() {
            // coming back by itself: the screen stays in view, the bar says what is happening
            return None;
        }
        let (title, body, main): (String, String, Option<&str>) = match &self.phase {
            Phase::Open => return None,
            Phase::Ticket | Phase::Dial | Phase::Hub if self.local.is_some() => {
                (format!("Starting {}", self.title), "Opening a shell on this computer.".into(), None)
            }
            Phase::Ended { note, retry, code: Some(_) } if self.local.is_some() => {
                ("The shell exited".into(), note.clone(), retry.then_some("Restart"))
            }
            Phase::Failed(m) if self.local.is_some() => {
                ("Couldn't start the shell".into(), ensure_stop(m), Some("Try again"))
            }
            Phase::Ticket | Phase::Dial | Phase::Hub => {
                let step = match self.phase {
                    Phase::Ticket => "Asking Switchboard for a terminal.",
                    Phase::Dial => "Reaching the terminal hub.",
                    _ => "Starting claude attach on its machine.",
                };
                (format!("Connecting to {label}"), step.into(), None)
            }
            Phase::Job(id) => {
                let app = self.app.read(cx);
                let job = app.jobs.iter().find(|j| !id.is_empty() && j.id == *id);
                // The same step words as the progress toast.
                let step = job.map(|j| j.detail(AppState::now())).unwrap_or_else(|| "Waiting for the server".into());
                let title = match job.map(|j| (j.kind, j.label.as_str())) {
                    Some((sb_core::JobKind::Start, _)) => format!("Starting the chat on {label}"),
                    // v2, Auto: Claude picks the machine first, then it starts there.
                    Some((sb_core::JobKind::Auto, l)) if l != sb_core::jobs::AUTO => {
                        format!("Starting the chat on {l}")
                    }
                    Some((sb_core::JobKind::Auto, _)) => "Claude is picking the machine".into(),
                    _ => "Resuming the chat".into(),
                };
                (title, format!("{}. The terminal opens as soon as it's running.", step.trim_end_matches('.')), None)
            }
            Phase::Ended { note, retry, code: Some(_) } => {
                ("Process exited".into(), note.clone(), retry.then_some("Restart"))
            }
            Phase::Ended { note, retry, .. } => ("Detached".into(), note.clone(), retry.then_some("Reconnect")),
            // These say it in our own words: the server's message repeats the title.
            Phase::Need { need: Need::Resume, message } if message == STOPPED_STAR => (
                "Stopped. Resume to continue".into(),
                "Starred, so its tab came back. Resume starts it on its machine and re-reads its context.".into(),
                Some("Resume"),
            ),
            Phase::Need { need: Need::Resume, .. } => (
                "This chat isn't running".into(),
                "Resume and open starts it in the background on its machine, then opens it here.".into(),
                Some("Resume and open"),
            ),
            Phase::Need { need: Need::Terminal, .. } => (
                format!("It's open in a terminal on {label}"),
                if self.url.is_some() {
                    format!("Only background chats open here. Type in it on {label}, or open it in Claude.")
                } else {
                    format!("Only background chats open here. Type in it on {label}.")
                },
                self.url.is_some().then_some("Open in Claude"),
            ),
            Phase::Need { need: Need::Offline, .. } => (
                format!("{label} is offline"),
                format!("Switchboard can't reach {label} right now. Its chats open here again once it's back."),
                Some("Try again"),
            ),
            Phase::Failed(m) => ("Couldn't open the terminal".into(), ensure_stop(m), Some("Try again")),
        };
        let color = self.phase.color(t);
        let connecting = self.phase.connecting();
        let close = ui::line_btn(t, "tc-close", 30.)
            .on_click(cx.listener(|_, _, _, cx| cx.emit(TermEvent::Close)))
            .child("Close tab");
        let main_btn = main.map(|m| {
            ui::primary_btn(t, "tc-main", 30.)
                .on_click(cx.listener(|this, _, _, cx| this.primary(cx)))
                .child(m.to_string())
                .child(ui::kbd_in("↵", t.on_accent, mix(t.on_accent, t.accent, 0.55)))
        });
        Some(
            div()
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .child(
                    div()
                        .id("term-card")
                        .w(px(420.))
                        .p(px(20.))
                        .flex()
                        .flex_col()
                        .gap(px(10.))
                        .rounded(px(ui::R_SHEET))
                        .border_1()
                        .border_color(t.line2)
                        .bg(t.panel2)
                        .shadow(vec![gpui::BoxShadow {
                            color: t.shadow.opacity(0.55),
                            offset: gpui::point(px(0.), px(8.)),
                            blur_radius: px(28.),
                            spread_radius: px(0.),
                            inset: false,
                        }])
                        .occlude()
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .child(div().flex().items_center().gap(px(10.)).child(ui::icon("term", 16., color)).child(
                            div().text_size(px(14.)).font_weight(gpui::FontWeight::BOLD).text_color(t.ink).child(title),
                        ))
                        .child(div().pl(px(26.)).text_size(px(12.5)).line_height(px(18.)).text_color(t.dim).child(body))
                        .when(!connecting, |d| {
                            d.child(
                                div()
                                    .pl(px(26.))
                                    .pt(px(6.))
                                    .flex()
                                    .items_center()
                                    .gap(px(8.))
                                    .when_some(main_btn, |d, b| d.child(b))
                                    .child(close),
                            )
                        }),
                )
                .into_any_element(),
        )
    }

    /// Spec section 5: moving the chat to another machine (Continue) stays one click away from its terminal.
    fn continue_keys(&self, t: &Theme, wide: bool, cx: &Context<Self>) -> Option<AnyElement> {
        let key = self.key.clone()?;
        let app = self.app.read(cx);
        let s = app.snap.session(&key)?;
        let can = sb_core::actions::for_session(s, &app.snap, AppState::now())
            .into_iter()
            .any(|a| a.id == sb_core::actions::ActionId::Continue && !a.disabled);
        let to: Vec<(String, String)> = app
            .snap
            .other_machines(&s.machine)
            .into_iter()
            .filter(|m| !m.is_offline())
            .map(|m| (m.name.clone(), m.label.clone()))
            .collect();
        if !can || to.is_empty() {
            return None;
        }
        let busy = app.busy.contains(&format!("{key}:continue"));
        let mut row = div()
            .flex_none()
            .flex()
            .items_center()
            .gap(px(4.))
            .pl(px(10.))
            .border_l_1()
            .border_color(t.rule)
            .when(wide, |d| d.child(div().mr(px(2.)).child("Continue on")));
        if busy {
            return Some(row.child(div().text_color(t.ink2).child("Writing the handoff")).into_any_element());
        }
        for (name, label) in to {
            let (hover, k) = (t.hover, key.clone());
            row = row.child(
                div()
                    .id(gpui::SharedString::from(format!("tcont-{name}")))
                    .h(px(18.))
                    .px(px(6.))
                    .flex()
                    .items_center()
                    .gap(px(5.))
                    .rounded(px(ui::R_CHIP))
                    .text_color(t.ink2)
                    .cursor_pointer()
                    .hover(move |st| st.bg(hover))
                    .child(ui::machine_icon(&name, 12., t.dim))
                    .child(if wide { label } else { format!("Continue on {label}") })
                    .on_click(cx.listener(move |this, _: &gpui::ClickEvent, _, cx| {
                        let (k, to) = (k.clone(), name.clone());
                        this.app.update(cx, |s, cx| s.continue_on(&k, &to, cx));
                    })),
            );
        }
        Some(row.into_any_element())
    }

    /// The bottom bar's look button: the scheme's own swatch, so it reads as "colors" before it is clicked.
    fn colors_btn(&self, t: &Theme, pal: &colors::Palette, wide: bool, cx: &Context<Self>) -> impl IntoElement {
        let hover = t.hover;
        let name = colors::name_of(&colors::chosen(cx));
        div()
            .id("term-colors")
            .flex_none()
            .h(px(24.))
            .px(px(8.))
            .flex()
            .items_center()
            .gap(px(7.))
            .rounded(px(ui::R_ITEM))
            .text_color(t.ink2)
            .cursor_pointer()
            .when(self.colors_open, |d| d.bg(t.press))
            .hover(move |st| st.bg(hover))
            .child(swatch(t, pal, 30., 16.))
            .child(if wide { name } else { "Look".to_string() })
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(|this, _: &gpui::ClickEvent, _, cx| {
                this.colors_open = !this.colors_open;
                cx.notify();
            }))
    }

    /// The look in one place: the app's theme, then the terminal's scheme (shown on hover), text size, cursor and
    /// blink. The terminal keeps its own scheme because Claude Code paints for a dark terminal whatever the app looks
    /// like. Everything here is synced to every device.
    fn colors_pop(&self, t: &Theme, cx: &Context<Self>) -> impl IntoElement {
        let look = colors::look(cx);
        let chosen = look.scheme.clone();
        let mode = crate::theme::Mode::parse(self.app.read(cx).settings.theme.as_deref());
        let head = |text: &str| {
            div()
                .px(px(12.))
                .pt(px(10.))
                .pb(px(6.))
                .text_size(px(12.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(t.dim)
                .child(text.to_string())
        };
        let mut modes = ui::seg_box(t).mx(px(10.)).mb(px(4.));
        for (i, (m, label)) in [
            (crate::theme::Mode::Light, "Light"),
            (crate::theme::Mode::System, "System"),
            (crate::theme::Mode::Dark, "Dark"),
        ]
        .into_iter()
        .enumerate()
        {
            modes = modes.child(
                ui::seg_btn(t, ("term-mode", i), mode == m, false).flex_1().justify_center().child(label).on_click(
                    cx.listener(move |_, _: &gpui::ClickEvent, window, cx| crate::theme::set_mode(m, window, cx)),
                ),
            );
        }
        let hover = t.hover;
        let row = |id: String, name: String, pal: colors::Palette, on: bool| {
            let pick = id.clone();
            let peek = id.clone();
            div()
                .id(gpui::SharedString::from(format!("scheme-{id}")))
                .h(px(34.))
                .px(px(6.))
                .flex()
                .items_center()
                .gap(px(10.))
                .rounded(px(ui::R_CTRL))
                .cursor_pointer()
                .when(on, |d| d.bg(t.sel))
                .when(!on, |d| d.hover(move |st| st.bg(hover)))
                .child(swatch(t, &pal, 52., 24.))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_size(px(13.))
                        .text_color(if on { t.ink } else { t.ink2 })
                        .when(on, |d| d.font_weight(gpui::FontWeight::SEMIBOLD))
                        .child(name),
                )
                .when(on, |d| d.child(ui::icon("check", 14., t.accent)))
                .on_hover(cx.listener(move |_, h: &bool, _, cx| {
                    if *h {
                        colors::set_preview(Some(peek.clone()), cx);
                    }
                }))
                .on_click(cx.listener(move |this, _: &gpui::ClickEvent, _, cx| {
                    colors::set_preview(None, cx);
                    crate::theme::set_term_colors(&pick, cx);
                    this.colors_open = false;
                    cx.notify();
                }))
        };
        let shown_light = match colors::shown(cx).as_str() {
            colors::MATCH => !t.dark,
            id => colors::scheme::by_id(id).map(|s| !s.dark).unwrap_or(false),
        };
        // Two columns of schemes, dark ones first (the default is dark), then Match the app on its own row.
        let all = colors::scheme::all();
        let mut dark = div().flex_1().min_w_0().flex().flex_col().gap(px(1.));
        let mut light = div().flex_1().min_w_0().flex().flex_col().gap(px(1.));
        for s in all {
            let r = row(s.id.clone(), s.name.clone(), colors::palette_of(s), chosen == s.id);
            if s.dark { dark = dark.child(r) } else { light = light.child(r) }
        }
        let schemes = div().id("schemes").px(px(6.)).flex().gap(px(6.)).child(dark).child(light).on_hover(cx.listener(
            |_, h: &bool, _, cx| {
                if !*h {
                    colors::set_preview(None, cx);
                }
            },
        ));
        let matching = div().px(px(6.)).pt(px(2.)).child(row(
            colors::MATCH.to_string(),
            "Match the app".into(),
            colors::palette(t),
            chosen == colors::MATCH,
        ));
        let size = look.size.unwrap_or(sb_core::term::scheme::SIZE_DEFAULT);
        let step = |id: &'static str, ic: &'static str, up: bool| {
            ui::icon_btn(t, id, ic, 26.)
                .shadow(vec![Theme::ring(t.line2)])
                .on_click(cx.listener(move |_, _: &gpui::ClickEvent, _, cx| step_size(Some(up), cx)))
        };
        let mut cursors = ui::seg_box(t);
        for (i, (v, label)) in [("block", "Block"), ("bar", "Bar"), ("underline", "Underline")].into_iter().enumerate()
        {
            cursors = cursors.child(ui::seg_btn(t, ("term-cursor", i), look.cursor == v, false).child(label).on_click(
                cx.listener(move |_, _: &gpui::ClickEvent, _, cx| {
                    crate::theme::save_look(cx, |s| s.term_cursor = Some(v.to_string()))
                }),
            ));
        }
        let blink = look.blink;
        let line = |label: &str, ctl: AnyElement| {
            div()
                .h(px(36.))
                .px(px(12.))
                .flex()
                .items_center()
                .gap(px(10.))
                .child(div().flex_1().text_size(px(13.)).text_color(t.ink2).child(label.to_string()))
                .child(ctl)
        };
        let close = cx.listener(|this, _: &MouseDownEvent, _, cx| {
            colors::set_preview(None, cx);
            this.colors_open = false;
            cx.notify();
        });
        div()
            .absolute()
            .inset_0()
            .occlude()
            .on_mouse_down(MouseButton::Left, close)
            .child(
                div()
                    .id("term-colors-pop")
                    .absolute()
                    .right(px(10.))
                    .bottom(px(ui::FOOTER + 6.))
                    .w(px(440.))
                    .pb(px(8.))
                    .flex()
                    .flex_col()
                    .rounded(px(ui::R_POP))
                    .border_1()
                    .border_color(t.line2)
                    .bg(t.panel2)
                    .shadow(t.drop())
                    .font_family(crate::theme::UI)
                    .occlude()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(head("App"))
                    .child(modes)
                    .child(div().mx(px(12.)).my(px(6.)).h(px(1.)).bg(t.rule))
                    .child(head("Terminal colors"))
                    .child(schemes)
                    .child(matching)
                    .when(shown_light, |d| {
                        d.child(
                            div()
                                .mx(px(12.))
                                .mt(px(6.))
                                .text_size(px(12.))
                                .line_height(px(17.))
                                .text_color(t.dim)
                                .child("Claude Code draws for a dark terminal. With a light scheme, run /theme in Claude and pick a light theme too."),
                        )
                    })
                    .child(div().mx(px(12.)).my(px(6.)).h(px(1.)).bg(t.rule))
                    .child(line(
                        "Text size",
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(step("term-smaller", "zoomout", false))
                            .child(div().w(px(30.)).flex().justify_center().text_size(px(13.)).font_weight(gpui::FontWeight::SEMIBOLD).text_color(t.ink).child(size.to_string()))
                            .child(step("term-bigger", "zoomin", true))
                            .child(div().text_size(px(11.5)).text_color(t.faint).child(format!("{} {}", ui::mod_cap("+"), ui::mod_cap("-"))))
                            .into_any_element(),
                    ))
                    .child(line("Cursor", cursors.into_any_element()))
                    .child(line(
                        "Blink",
                        ui::seg_box(t)
                            .child(ui::seg_btn(t, "blink-on", blink, false).child("On").on_click(cx.listener(|_, _: &gpui::ClickEvent, _, cx| {
                                crate::theme::save_look(cx, |s| s.term_blink = Some(true))
                            })))
                            .child(ui::seg_btn(t, "blink-off", !blink, false).child("Off").on_click(cx.listener(|_, _: &gpui::ClickEvent, _, cx| {
                                crate::theme::save_look(cx, |s| s.term_blink = Some(false))
                            })))
                            .into_any_element(),
                    )),
            )
    }

    /// The bar's flags button: what the chat runs with, and the way into the flags drawer.
    fn flags_btn(&self, t: &Theme, wide: bool, cx: &Context<Self>) -> Option<AnyElement> {
        let key = self.key.clone()?;
        let hover = t.hover;
        let sum = self.flags_sum.clone().filter(|_| wide).unwrap_or_else(|| "Flags".into());
        Some(
            div()
                .id("term-flags")
                .flex_none()
                .h(px(24.))
                .px(px(8.))
                .flex()
                .items_center()
                .gap(px(6.))
                .rounded(px(ui::R_ITEM))
                .text_color(t.ink2)
                .cursor_pointer()
                .hover(move |st| st.bg(hover))
                .child(ui::icon("flag", 13., t.dim))
                .child(sum)
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(move |_, _: &gpui::ClickEvent, _, cx| cx.emit(TermEvent::Flags(key.clone()))))
                .into_any_element(),
        )
    }

    fn attach_btn(&self, t: &Theme, wide: bool, cx: &Context<Self>) -> AnyElement {
        let hover = t.hover;
        div()
            .id("term-attach")
            .flex_none()
            .h(px(24.))
            .px(px(8.))
            .flex()
            .items_center()
            .gap(px(6.))
            .rounded(px(ui::R_ITEM))
            .text_color(t.ink2)
            .cursor_pointer()
            .hover(move |st| st.bg(hover))
            .child(ui::icon("attach", 13., t.dim))
            .when(wide, |d| d.child("Attach"))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(|this, _: &gpui::ClickEvent, _, cx| this.attach(cx)))
            .into_any_element()
    }

    /// The bar while the link is coming back: a breathing amber dot, "Reconnecting…", and a way to skip the wait.
    fn reconnecting(&self, t: &Theme, cx: &Context<Self>) -> Option<AnyElement> {
        use gpui::AnimationExt;
        self.reconnect.as_ref()?;
        let hover = t.hover;
        let dot = div().flex_none().size(px(8.)).rounded_full().bg(t.cold).with_animation(
            "term-reconnect-dot",
            gpui::Animation::new(Duration::from_millis(1400)).repeat().with_easing(gpui::pulsating_between(0.35, 1.0)),
            |d, a| d.opacity(a),
        );
        Some(
            div()
                .flex_none()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(dot)
                .child(div().flex_none().text_color(t.ink2).child("Reconnecting…"))
                .child(
                    div()
                        .id("term-reconnect-now")
                        .flex_none()
                        .h(px(22.))
                        .px(px(7.))
                        .flex()
                        .items_center()
                        .rounded(px(ui::R_ITEM))
                        .text_color(t.dim)
                        .cursor_pointer()
                        .hover(move |st| st.bg(hover))
                        .child("Reconnect now")
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(cx.listener(|this, _: &gpui::ClickEvent, _, cx| this.reconnect_now(cx))),
                )
                .into_any_element(),
        )
    }

    /// The terminal is shared with another window that set a different size: say so, and a click takes it here.
    fn size_chip(&self, t: &Theme, cx: &Context<Self>) -> Option<AnyElement> {
        let want = self.want?;
        if !self.hub_sizes || self.owner || self.phase != Phase::Open || want == self.size {
            return None;
        }
        let hover = t.hover;
        Some(
            div()
                .id("term-size-chip")
                .flex_none()
                .h(px(24.))
                .px(px(8.))
                .flex()
                .items_center()
                .gap(px(6.))
                .rounded(px(ui::R_ITEM))
                .text_color(t.dim)
                .cursor_pointer()
                .hover(move |st| st.bg(hover))
                .child(ui::icon("fit", 13., t.dim))
                .child(ui::num(format!("{}×{} from another window", self.size.0, self.size.1), 12., t.dim))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(|this, _: &gpui::ClickEvent, _, cx| {
                    this.take_size();
                    cx.notify();
                }))
                .into_any_element(),
        )
    }

    fn strip(
        &self,
        t: &Theme,
        wide: bool,
        cont: Option<AnyElement>,
        colors_btn: AnyElement,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let label = if self.label.is_empty() { self.machine.clone() } else { self.label.clone() };
        let attached = self.phase == Phase::Open || self.has_output;
        let hint = |keys: Vec<String>, text: &str| {
            div()
                .flex()
                .items_center()
                .gap(px(3.))
                .children(keys.iter().map(|k| ui::kbd(t, k)))
                .child(div().ml(px(4.)).child(text.to_string()))
        };
        // A calm bar on the footer grid: where it runs, how to move it, and the one key that leaves. Copy, paste and
        // the grid size are what every terminal does; the menus and the shortcut sheet list them.
        let close: Vec<String> =
            if MAC { vec!["⌘W".into()] } else { vec!["Ctrl".into(), "Shift".into(), "W".into()] };
        let local = self.local.is_some();
        let word = if local && self.phase == Phase::Open { "Running" } else { self.phase.word() };
        div()
            .flex_none()
            .h(px(ui::FOOTER))
            .flex()
            .items_center()
            .gap(px(12.))
            .pl(px(ui::lead()))
            .pr(px(10.))
            .bg(t.bg2)
            .border_t_1()
            .border_color(t.rule)
            .text_size(px(12.5))
            .text_color(t.faint)
            .map(|d| match self.reconnecting(t, cx) {
                Some(r) => d.child(r),
                None => d
                    .child(div().flex_none().size(px(8.)).rounded_full().bg(self.phase.color(t)))
                    .child(div().flex_none().text_color(t.ink2).child(word)),
            })
            .map(|d| match &self.local {
                // A local shell: this computer, and the program it runs.
                Some(p) => d
                    .child(ui::icon("term", 14., t.dim))
                    .child(div().flex_none().text_color(t.dim).child(label))
                    .child(ui::num(p.program.clone(), 12., t.faint)),
                None => d
                    .child(ui::machine_icon(&self.machine, 14., t.dim))
                    .child(div().flex_none().text_color(t.dim).child(label)),
            })
            // The attach command and the grid size only mean something once it has been attached.
            .when(attached, |d| {
                d.when_some(self.bg.clone(), |d, b| d.child(ui::num(format!("claude attach {b}"), 12., t.faint)))
            })
            .child(ui::spacer())
            .children(self.size_chip(t, cx))
            .children(self.flags_btn(t, wide, cx))
            .when(!local, |d| d.child(self.attach_btn(t, wide, cx)))
            .child(colors_btn)
            .children(cont)
            .when(wide, |d| d.child(hint(close, if local { "close" } else { "detach" })))
    }
}

/// A scheme in miniature: its background with a text bar and red, green, yellow and blue.
pub(crate) fn swatch(t: &Theme, pal: &colors::Palette, w: f32, h: f32) -> gpui::Div {
    let dot = (h * 0.27).round();
    div()
        .flex_none()
        .w(px(w))
        .h(px(h))
        .overflow_hidden()
        .flex()
        .items_center()
        .justify_center()
        .gap(px((dot * 0.5).round()))
        .rounded(px((h * 0.25).round()))
        .bg(pal.bg)
        .border_1()
        .border_color(t.line)
        .when(w >= 32., |d| d.child(div().flex_none().w(px(dot * 1.6)).h(px(2.)).rounded_full().bg(pal.fg)))
        // Red, green, (yellow,) blue: the small one has room for three.
        .children(
            [1usize, 2, 3, 4]
                .into_iter()
                .filter(|&i| w >= 32. || i != 3)
                .map(|i| div().flex_none().size(px(dot)).rounded_full().bg(pal.ansi[i])),
        )
}

/// Links a click may open in the browser. Anything else is copied.
fn web(u: &str) -> bool {
    let l = u.to_ascii_lowercase();
    l.starts_with("https://") || l.starts_with("http://")
}

/// A server message as the first sentence of a card.
fn ensure_stop(s: &str) -> String {
    let s = s.trim();
    if s.ends_with(['.', '!', '?']) { s.to_string() } else { format!("{s}.") }
}

/// What a paste sends. `shots` builds (driven tests) take it from SB_TEST_CLIPBOARD, never the real clipboard.
fn clipboard_text(cx: &App) -> Option<String> {
    #[cfg(feature = "shots")]
    {
        let _ = cx;
        std::env::var("SB_TEST_CLIPBOARD").ok().map(|s| s.replace("\\n", "\n"))
    }
    #[cfg(not(feature = "shots"))]
    {
        cx.read_from_clipboard().and_then(|c| c.text())
    }
}

impl Render for TermTab {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Screenshots: a faked live state stands for an open terminal, whatever the (fixture) connection did.
        if self.fake_live.is_some() {
            self.phase = Phase::Open;
        }
        let t = Theme::get(cx);
        let pal = colors::current(cx);
        // Focus reporting (Claude Code dims its prompt when the terminal loses focus) and the bell clearing.
        // Test builds run on displays with no window manager, where no window is ever active.
        let focused = self.focus.is_focused(window) && (cfg!(feature = "shots") || window.is_window_active());
        if focused != self.focused {
            self.focused = focused;
            if focused && self.bell {
                self.bell = false;
                cx.emit(TermEvent::Changed);
            }
            if focused {
                self.take_size();
            }
            if self.phase == Phase::Open && self.owner && self.term.mode().contains(TermMode::FOCUS_IN_OUT) {
                self.send(if focused { b"\x1b[I".to_vec() } else { b"\x1b[O".to_vec() });
            }
        }
        let wide = f32::from(window.viewport_size().width) >= 1100.;
        let mouse_mode = self.phase == Phase::Open && self.term.mode().intersects(TermMode::MOUSE_MODE);
        if let Some((l, f)) = &self.hover
            && self.link_at(*l, f.start).as_ref() != Some(f)
        {
            self.hover = None;
        }
        let tip = self.link_tip(&t, mouse_mode);
        let on_link = tip.is_some();
        let card = self.card(&t, cx);
        // A light scheme gets light chrome around it (and a dark one dark chrome), so the bar matches the terminal.
        let ct = if pal.dark == t.dark {
            t
        } else if pal.dark {
            Theme::dark()
        } else {
            Theme::light()
        };
        let cont = self.continue_keys(&ct, wide, cx);
        let colors_btn = self.colors_btn(&ct, &pal, wide, cx).into_any_element();
        let pop = self.colors_open.then(|| self.colors_pop(&ct, cx).into_any_element());
        let dim = card.is_some() && self.has_output;
        // The live state on a strip at the top, read as one line: the state, what goes with it, and the context
        // on the right. It moves with the terminal, not the server's polls.
        let live = self.live_state(cx).filter(|_| self.phase == Phase::Open);
        let now = AppState::now();
        match &live {
            Some(l) if l.working() => {
                if self.work_since.is_none() {
                    self.work_since = Some(now);
                }
            }
            _ => self.work_since = None,
        }
        let strip_w = f32::from(window.viewport_size().width);
        let strip = live.map(|l| {
            use sb_core::live::LiveKind as K;
            // The same chat the tab's dot reads (after /clear too), so the strip and the dot never disagree.
            let snap_s = self.chat(cx).cloned();
            let ctx_k = snap_s.as_ref().map(|s| s.ctx).unwrap_or(0);
            let word = l.word;
            // Working says for how long (this terminal saw it start); everything else is the shared detail.
            let detail = match self.work_since {
                Some(t) if l.working() && l.stale.is_none() => {
                    format!("for {}", sb_core::live::span((now - t).max(0.) as u64))
                }
                _ => l.detail.clone(),
            };
            let c = crate::chrome::live_color(&ct, &l);
            // The detail goes first as room runs out (an ellipsis), then the context label shortens, then the meter.
            let ctx_short = strip_w < 700.;
            let meter = strip_w >= 520.;
            let frac = (ctx_k as f32 / sb_core::model::COMPACT as f32).clamp(0., 1.);
            // Colored as the inspector's context meter: heavy is amber, over is red.
            let fill = match &snap_s {
                Some(s) if s.over() => ct.bad,
                Some(s) if s.heavy() => ct.cold,
                _ => ct.fill,
            };
            let detail_c = if l.stale.is_some() {
                ct.faint
            } else if l.kind == K::Warm && l.urgent {
                ct.cold
            } else {
                ct.dim
            };
            let no_detail = detail.is_empty();
            div()
                .flex_none()
                .h(px(36.))
                .pl(px(14.))
                .pr(px(14.))
                .flex()
                .items_center()
                .gap(px(9.))
                .border_b_1()
                .border_color(ct.line)
                .bg(crate::theme::mix(ct.panel, c, if ct.dark { 0.07 } else { 0.05 }))
                .child(crate::chrome::live_dot(&ct, &l, "term-live", 8.))
                .child(
                    div()
                        .flex_none()
                        .whitespace_nowrap()
                        .text_size(px(14.))
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(c)
                        .child(word),
                )
                .when(!no_detail, |d| {
                    d.child(div().flex_none().size(px(3.)).rounded_full().bg(ct.faint)).child(
                        ui::line(detail).flex_1().text_size(px(13.)).text_color(detail_c).font_features(ui::tnum()),
                    )
                })
                .when(no_detail, |d| d.child(div().flex_1()))
                .when_some(l.expensive.clone(), |d, e| d.child(crate::chrome::cost_badge(&ct, &e, l.stale.is_some())))
                .when(ctx_k > 0, |d| {
                    d.when(meter, |d| d.child(div().flex_none().w(px(80.)).child(ui::meter(&ct, frac, fill, None, 4.))))
                        .child(
                            div()
                                .flex_none()
                                .whitespace_nowrap()
                                .text_size(px(13.))
                                .text_color(ct.dim)
                                .font_features(ui::tnum())
                                .child(sb_core::live::ctx_label(ctx_k, ctx_short)),
                        )
                })
        });
        let back = self.term.grid().display_offset();
        if self.phase == Phase::Open && !self.flags_asked {
            self.flags_asked = true;
            self.load_flags(cx);
        }
        let look = colors::look(cx);
        self.run_blink(look.blink && focused, window, cx);
        let cursor_look = grid::CursorLook {
            shape: match look.cursor.as_str() {
                "bar" => alacritty_terminal::vte::ansi::CursorShape::Beam,
                "underline" => alacritty_terminal::vte::ansi::CursorShape::Underline,
                _ => alacritty_terminal::vte::ansi::CursorShape::Block,
            },
            visible: !look.blink || self.blink_on,
        };
        let this = cx.entity();
        let focus = self.focus.clone();
        let grid = canvas(
            {
                let this = this.clone();
                move |bounds, window, cx| {
                    let m = Metrics::new(window, colors::font_size(cx));
                    this.update(cx, |tab, cx| tab.layout(m, bounds, cx));
                    crate::perf::time(crate::perf::Kind::Build, || {
                        grid::build(this.read(cx), &m, &pal, cursor_look, bounds, window)
                    })
                }
            },
            move |bounds, frame, window, cx| {
                window.handle_input(&focus, ElementInputHandler::new(bounds, this), cx);
                crate::perf::time(crate::perf::Kind::Paint, || grid::paint(frame, window, cx));
            },
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full();
        div()
            .id("term-tab")
            .key_context("Terminal")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key))
            .on_action(cx.listener(Self::on_copy))
            .on_action(cx.listener(Self::on_paste))
            .on_action(cx.listener(|this, _: &TermPageUp, _, cx| this.page(true, cx)))
            .on_action(cx.listener(|this, _: &TermRedraw, _, cx| this.redraw(cx)))
            .on_action(cx.listener(|this, _: &TermPageDown, _, cx| this.page(false, cx)))
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .bg(pal.bg)
            .when_some(strip, |d, s| d.child(s))
            .child(
                div()
                    .id("term-grid")
                    .group("term-drop")
                    .relative()
                    .on_drop(cx.listener(|this, paths: &gpui::ExternalPaths, _, cx| {
                        this.send_files(paths.paths().to_vec(), cx)
                    }))
                    .flex_1()
                    .min_h_0()
                    .overflow_hidden()
                    .when(on_link, |d| d.cursor_pointer())
                    .when(mouse_mode && !on_link, |d| d.cursor_default())
                    .when(!mouse_mode && !on_link, |d| d.cursor_text())
                    .on_mouse_down(MouseButton::Left, cx.listener(Self::mouse_down))
                    .on_mouse_down(MouseButton::Middle, cx.listener(Self::mouse_down))
                    .on_mouse_down(MouseButton::Right, cx.listener(Self::mouse_down))
                    .on_mouse_move(cx.listener(Self::mouse_move))
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(|this, ev: &MouseUpEvent, _, cx| this.mouse_up(ev.position, &ev.modifiers, cx)),
                    )
                    .on_mouse_up(
                        MouseButton::Middle,
                        cx.listener(|this, ev: &MouseUpEvent, _, cx| this.mouse_up(ev.position, &ev.modifiers, cx)),
                    )
                    .on_mouse_up(
                        MouseButton::Right,
                        cx.listener(|this, ev: &MouseUpEvent, _, cx| this.mouse_up(ev.position, &ev.modifiers, cx)),
                    )
                    .on_mouse_up_out(
                        MouseButton::Left,
                        cx.listener(|this, ev: &MouseUpEvent, _, cx| this.mouse_up(ev.position, &ev.modifiers, cx)),
                    )
                    .on_scroll_wheel(cx.listener(Self::wheel))
                    .child(grid)
                    .when(dim, |d| {
                        d.child(div().absolute().inset_0().bg(if pal.dark {
                            mix(pal.bg, t.scrim, 0.5).opacity(0.72)
                        } else {
                            pal.bg.opacity(0.78)
                        }))
                    })
                    .when(back > 0 && card.is_none(), |d| d.child(self.back_pill(&t, back, cx)))
                    .when_some(tip.filter(|_| card.is_none()), |d, c| d.child(c))
                    .when_some(card, |d, c| d.child(c))
                    .children(self.uploads_el(&t, cx))
                    .child(self.drop_hint(&t)),
            )
            .child(self.strip(&ct, wide, cont, colors_btn, cx))
            .children(pop)
    }
}

/// Cmd+Plus (Some(true)), Cmd+Minus (Some(false)) or Cmd+0 (None): the text size for every terminal, synced.
pub(crate) fn step_size(up: Option<bool>, cx: &mut App) {
    let cur = colors::look(cx).size;
    let next = up.map(|u| sb_core::term::scheme::step_size(cur, u));
    crate::theme::save_look(cx, |s| s.term_size = next);
}

impl TermTab {
    /// Starts or stops the cursor's blink (530 ms, like most terminals). Only the focused tab blinks.
    fn run_blink(&mut self, on: bool, window: &mut Window, cx: &mut Context<Self>) {
        if !on {
            self.blink_task = None;
            self.blink_on = true;
            return;
        }
        if self.blink_task.is_some() {
            return;
        }
        self.blink_task = Some(cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_millis(530)).await;
                if this
                    .update(cx, |t, cx| {
                        t.blink_on = !t.blink_on;
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        }));
    }
}

impl EntityInputHandler for TermTab {
    fn text_for_range(
        &mut self,
        _: Range<usize>,
        _: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        None
    }

    fn selected_text_range(&mut self, _: bool, _: &mut Window, _: &mut Context<Self>) -> Option<UTF16Selection> {
        Some(UTF16Selection { range: 0..0, reversed: false })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked.as_ref().map(|m| 0..m.encode_utf16().count())
    }

    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.marked = None;
        cx.notify();
    }

    fn replace_text_in_range(&mut self, _: Option<Range<usize>>, text: &str, _: &mut Window, cx: &mut Context<Self>) {
        self.marked = None;
        if self.phase == Phase::Open && !text.is_empty() {
            self.line.text(text);
            self.typed(text.as_bytes().to_vec(), cx);
        }
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        _: Option<Range<usize>>,
        text: &str,
        _: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.marked = (!text.is_empty()).then(|| text.to_string());
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        _: Range<usize>,
        _: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        // The input method's window goes under the cursor cell.
        let (m, b) = self.geom?;
        let o = m.origin(b);
        let c = self.term.grid().cursor.point;
        let line = c.line.0.max(0) as f32;
        Some(Bounds::new(
            point(o.x + px(c.column.0 as f32 * m.cell_w), o.y + px(line * m.line_h)),
            size(px(m.cell_w), px(m.line_h)),
        ))
    }

    fn character_index_for_point(
        &mut self,
        _: gpui::Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        None
    }

    fn accepts_text_input(&self, _: &mut Window, _: &mut Context<Self>) -> bool {
        self.phase == Phase::Open
    }
}

#[cfg(test)]
mod size_tests {
    use super::{SizeStep, size_step};

    #[test]
    fn with_a_hub_that_sends_sizes_the_grid_waits_for_the_frame() {
        assert_eq!(size_step(true, true, false), SizeStep::Ask);
        assert_eq!(size_step(true, true, true), SizeStep::Ask, "even the first size, once attached");
        assert_eq!(size_step(true, false, false), SizeStep::Now, "not attached: nothing is drawing into it");
        assert_eq!(size_step(false, true, false), SizeStep::Settled, "an older hub: as before");
        assert_eq!(size_step(false, true, true), SizeStep::Now);
    }
}

#[cfg(test)]
mod chat_tests {
    use super::same_chat;

    #[test]
    fn a_terminal_holds_its_chat_by_key_or_by_background_id() {
        assert!(same_chat(Some("studio:a"), None, "studio:a", None));
        assert!(same_chat(Some("studio:a"), Some("bg-1"), "studio:b", Some("bg-1")), "after /clear, key not moved yet");
        assert!(!same_chat(Some("studio:a"), Some("bg-1"), "studio:b", Some("bg-2")));
        assert!(!same_chat(None, None, "studio:b", None), "no background id on either side is no match");
    }
}

#[cfg(test)]
mod request_tests {
    use super::term_request;
    use sb_core::term::TermRequest;

    #[test]
    fn the_background_id_wins_only_right_after_a_job_that_gave_one() {
        let bg = Some("bg-1");
        let sid = Some("s1");
        assert_eq!(term_request("studio", bg, sid, true), Some(TermRequest::bg("studio", "bg-1")));
        let chat = Some(TermRequest::chat("studio", "s1"));
        assert_eq!(term_request("studio", bg, sid, false), chat, "after a job without one");
        assert_eq!(term_request("studio", bg, None, false), Some(TermRequest::bg("studio", "bg-1")));
        assert_eq!(term_request("studio", None, None, true), None);
    }

    #[test]
    fn a_job_without_a_background_id_stops_asking_by_one() {
        assert!(super::prefer_bg_after(Some("bg-1")));
        assert!(!super::prefer_bg_after(None), "an older background id is not asked for again");
    }
}

#[cfg(test)]
mod exit_tests {
    use super::exit_closes_tab;

    #[test]
    fn only_a_clean_exit_closes_the_tab() {
        assert!(exit_closes_tab(Some(Some(0))));
        assert!(!exit_closes_tab(Some(Some(1))), "an error keeps the last output");
        assert!(!exit_closes_tab(Some(Some(255))), "a lost connection reconnects");
        assert!(!exit_closes_tab(Some(None)));
        assert!(!exit_closes_tab(None), "a dropped socket is not an exit");
        assert!(super::left_chat(Some(Some(super::TERM_LEFT))), "exit typed in the chat closes the tab");
        assert!(!super::left_chat(Some(Some(0))), "a clean end nobody asked for keeps the tab");
    }

    #[test]
    fn a_quit_key_counts_only_just_before_the_end() {
        use super::{QUIT_WINDOW, quit_asked};
        use std::time::{Duration, Instant};
        let t = Instant::now();
        assert!(quit_asked(Some(t), t + Duration::from_secs(1)));
        assert!(quit_asked(Some(t), t + QUIT_WINDOW));
        assert!(!quit_asked(Some(t), t + Duration::from_secs(10)), "Ctrl-D kept by a program, the end much later");
        assert!(!quit_asked(None, t));
    }

    #[test]
    fn a_leave_counts_however_long_the_end_takes() {
        use super::end_asked;
        use std::time::{Duration, Instant};
        let t = Instant::now();
        assert!(end_asked(true, None, t + Duration::from_secs(6)), "exit, then a slow link");
        assert!(!end_asked(false, None, t), "typed over: nothing was asked");
        assert!(!end_asked(false, Some(t), t + Duration::from_secs(10)));
    }
}
