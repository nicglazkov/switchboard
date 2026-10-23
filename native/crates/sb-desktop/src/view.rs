//! One window: the Swimlanes board (machines top to bottom, states left to right), the inspector, the title and
//! status bars, and the overlays (palette, sheets, popover, confirm, toasts and job progress).
//! A torn-off lane is the same view with `solo` set to one machine.

use std::collections::{HashMap, HashSet};

use gpui::{
    AnimationExt, AnyElement, App, AppContext, ClickEvent, Context, Decorations, Entity, FocusHandle, FontWeight,
    InteractiveElement, IntoElement, KeyBinding, MouseButton, ParentElement, Render, ScrollHandle, SharedString,
    StatefulInteractiveElement, Styled, Subscription, TitlebarOptions, Window, WindowAppearance,
    WindowBackgroundAppearance, WindowBounds, WindowDecorations, WindowOptions, actions, div, point, prelude::*, px,
    relative, size,
};
use gpui_base::input::{InputEvent, InputState};
use sb_core::actions::{self as acts, ActionId, Confirm};
use sb_core::board::{self, Board, Column};
use sb_core::fmt;

use crate::archive::ArcTab;
use crate::link::{LinkEvent, LinkSheet};
use crate::sheet::{NewSheet, SheetEvent};
use crate::state::{AppEvent, AppState};
use crate::term::{Start, TermEvent, TermTab};
use crate::theme::{Mode, NUM, Theme, UI};
use crate::ui::{self, MAC};

actions!(
    switchboard,
    [
        Next,
        Prev,
        Left,
        Right,
        Activate,
        Fresh,
        Resume,
        Continue,
        NewSession,
        TogglePalette,
        Escape,
        StartNew,
        PalNext,
        PalPrev,
        SwapField,
        Quit,
        CloseWindow,
        CloseTab,
        ChatPalette,
        NewTerminal,
        Browse,
        OpenWindow,
        OpenFloat,
        ToggleTheme,
        Settings,
        About,
        Tab1,
        Tab2,
        Tab3,
        Tab4,
        Tab5,
        Tab6,
        Tab7,
        Tab8,
        Tab9,
        FocusNext,
        FocusPrev
    ]
);

pub fn bind_keys(cx: &mut App) {
    let b = Some("Board");
    // Plain keys only while no text field could be typing: the board adds "keys" to its context then.
    let k = Some("Board && keys");
    cx.bind_keys([
        KeyBinding::new("j", Next, k),
        KeyBinding::new("down", Next, k),
        KeyBinding::new("k", Prev, k),
        KeyBinding::new("up", Prev, k),
        KeyBinding::new("h", Left, k),
        KeyBinding::new("left", Left, k),
        KeyBinding::new("l", Right, k),
        KeyBinding::new("right", Right, k),
        KeyBinding::new("enter", Activate, b),
        // Tab walks the buttons; the focus ring shows only once the keyboard is in use (ui::focusable).
        KeyBinding::new("tab", FocusNext, k),
        KeyBinding::new("shift-tab", FocusPrev, k),
        KeyBinding::new("tab", FocusNext, Some("Settings")),
        KeyBinding::new("shift-tab", FocusPrev, Some("Settings")),
        KeyBinding::new("shift-enter", OpenWindow, k),
        KeyBinding::new("alt-enter", OpenFloat, k),
        KeyBinding::new("secondary-shift-l", ToggleTheme, None),
        KeyBinding::new("secondary-,", Settings, None),
        KeyBinding::new("f", Fresh, k),
        KeyBinding::new("r", Resume, k),
        KeyBinding::new("c", Continue, k),
        KeyBinding::new("n", NewSession, k),
        KeyBinding::new("secondary-n", NewSession, b),
        KeyBinding::new("secondary-k", TogglePalette, b),
        KeyBinding::new("escape", Escape, b),
        KeyBinding::new("secondary-enter", StartNew, b),
        KeyBinding::new("secondary-enter", StartNew, Some("NewSheet > Input")),
        // Tab moves between the message and the new folder name instead of typing spaces into the message.
        KeyBinding::new("tab", SwapField, Some("NewSheet > Input")),
        KeyBinding::new("shift-tab", SwapField, Some("NewSheet > Input")),
        // The palette's arrows win over the text field's own up and down.
        KeyBinding::new("up", PalPrev, Some("LinkSheet > Input")),
        KeyBinding::new("down", PalNext, Some("LinkSheet > Input")),
        KeyBinding::new("up", PalPrev, Some("Palette > Input")),
        KeyBinding::new("down", PalNext, Some("Palette > Input")),
        KeyBinding::new("ctrl-p", PalPrev, Some("Palette > Input")),
        KeyBinding::new("ctrl-n", PalNext, Some("Palette > Input")),
        KeyBinding::new("secondary-q", Quit, None),
        // Tabs: Cmd/Ctrl+W closes the terminal tab (detaches), or the window on the board; Cmd+T opens a shell on
        // this computer and Cmd+O finds a chat to open; Cmd/Ctrl+1 is the board, 2 to 8 the terminals in order, 9
        // the last one.
        KeyBinding::new("secondary-w", CloseTab, b),
        // A terminal in a window of its own: Cmd/Ctrl+W closes that window (place.rs).
        KeyBinding::new("secondary-w", CloseTab, Some("TermWindow")),
        KeyBinding::new("secondary-t", NewTerminal, b),
        KeyBinding::new("secondary-o", ChatPalette, b),
        KeyBinding::new("secondary-b", Browse, b),
        KeyBinding::new("secondary-1", Tab1, b),
        KeyBinding::new("secondary-2", Tab2, b),
        KeyBinding::new("secondary-3", Tab3, b),
        KeyBinding::new("secondary-4", Tab4, b),
        KeyBinding::new("secondary-5", Tab5, b),
        KeyBinding::new("secondary-6", Tab6, b),
        KeyBinding::new("secondary-7", Tab7, b),
        KeyBinding::new("secondary-8", Tab8, b),
        KeyBinding::new("secondary-9", Tab9, b),
    ]);
    if !MAC {
        // In a terminal, Ctrl+W, T, O, B, K, N and Q belong to the program (a shell deletes a word with Ctrl+W): the
        // app takes them with Shift there, like the Windows and Linux terminals do.
        // The Shift forms work on the board too, so one habit covers both.
        cx.bind_keys([
            KeyBinding::new("ctrl-shift-w", CloseTab, b),
            KeyBinding::new("ctrl-shift-w", CloseTab, Some("TermWindow")),
            KeyBinding::new("ctrl-shift-t", NewTerminal, b),
            KeyBinding::new("ctrl-shift-o", ChatPalette, b),
            KeyBinding::new("ctrl-shift-b", Browse, b),
            KeyBinding::new("ctrl-shift-k", TogglePalette, b),
            KeyBinding::new("ctrl-shift-n", NewSession, b),
            KeyBinding::new("ctrl-shift-q", Quit, None),
        ]);
    }
    crate::files::bind_keys(cx);
    // Last, so the terminal's own keys win over the board's.
    crate::term::bind_keys(cx);
    cx.on_action(|_: &Quit, cx: &mut App| cx.quit());
    cx.on_action(|_: &Settings, cx: &mut App| crate::settings::open(cx));
    cx.on_action(|_: &About, cx: &mut App| crate::about::open(cx));
}

/// Opens the board window, or a lane on its own (`solo`).
pub fn open_window(solo: Option<String>, cx: &mut App) -> Option<gpui::WindowId> {
    let main = solo.is_none();
    let (w, h) = match std::env::var("SB_SIZE")
        .ok()
        .and_then(|v| v.split_once('x').map(|(a, b)| (a.parse::<f32>(), b.parse::<f32>())))
    {
        Some((Ok(w), Ok(h))) if main => (w, h),
        _ if main => (1440., 900.),
        _ => (560., 760.),
    };
    let label = solo.as_ref().map(|m| crate::state::app(cx).read(cx).snap.label_of(m).unwrap_or_else(|| m.clone()));
    let title = match &label {
        Some(l) => format!("{l} lane, Switchboard"),
        None => "Switchboard".to_string(),
    };
    let centered = gpui::Bounds::centered(None, size(px(w), px(h)), cx);
    // The board reopens where it was left, at its last size, unless SB_SIZE asks for one (shots) or that spot is
    // no longer on any screen (a display was unplugged).
    let saved =
        (main && std::env::var("SB_SIZE").is_err()).then(|| crate::state::app(cx).read(cx).settings.window).flatten();
    let window_bounds = match saved {
        Some(s) if s.w >= 900. && s.h >= 600. => {
            let b = gpui::Bounds::new(point(px(s.x), px(s.y)), size(px(s.w), px(s.h)));
            if cx.displays().iter().any(|d| d.bounds().intersects(&b)) {
                if s.maximized { WindowBounds::Maximized(b) } else { WindowBounds::Windowed(b) }
            } else {
                WindowBounds::Windowed(gpui::Bounds::centered(None, b.size, cx))
            }
        }
        _ => WindowBounds::Windowed(centered),
    };
    let opts = WindowOptions {
        window_bounds: Some(window_bounds),
        titlebar: Some(TitlebarOptions {
            title: Some(title.into()),
            appears_transparent: MAC || cfg!(target_os = "windows"),
            traffic_light_position: MAC.then(|| point(px(16.), px(12.))),
        }),
        app_owns_titlebar_drag: MAC,
        window_min_size: Some(if main { size(px(900.), px(600.)) } else { size(px(420.), px(420.)) }),
        app_id: Some("switchboard".into()),
        window_background: WindowBackgroundAppearance::Opaque,
        window_decorations: Some(WindowDecorations::Server),
        ..Default::default()
    };
    match cx.open_window(opts, move |window, cx| cx.new(|cx| BoardView::new(solo, window, cx))) {
        Ok(h) => Some(h.window_id()),
        Err(e) => {
            log::error!("could not open a window: {e:#}");
            None
        }
    }
}

pub enum Overlay {
    None,
    Palette,
    New(Entity<NewSheet>),
    Archive {
        tab: ArcTab,
        key: Option<String>,
    },
    /// "This continues...": pick the older chat the selected one carries on from.
    Link(Entity<LinkSheet>),
}

impl Overlay {
    pub fn is_none(&self) -> bool {
        matches!(self, Overlay::None)
    }
}

/// A bar in the usage panel the pointer is on: a day of this week, or one of the last four weeks.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum UsageBar {
    Day(usize),
    Week(usize),
    /// The "?" beside "This week by day": its line says what a bar is.
    Help,
}

/// What the first tab shows: the board, or History (everything older).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Main {
    Board,
    History,
}

pub struct BoardView {
    pub app: Entity<AppState>,
    pub main: Main,
    /// "Not in use" strip: its list is open.
    pub spare_open: bool,
    /// The two-step stop armed now, and a stopped chat on its way to "Stopped today".
    pub stop_armed: Option<crate::stop::Armed>,
    pub flight: Option<crate::stop::Flight>,
    /// Freed memory last frame, and (from, since) while it ticks up to the new total.
    pub freed_seen: std::cell::Cell<f64>,
    pub freed_from: std::cell::Cell<(f64, f64)>,
    /// The header's "+" menu (from the chevron beside "+": with the local shells first), and the update popover.
    pub add_open: bool,
    pub add_terms: bool,
    /// Windows and Linux: the app menu (the Mac's menu bar) is open.
    pub app_menu_open: bool,
    pub update_open: bool,
    /// A terminal tab's own menu: which tab (found again on each use, as tabs before it may close), and where.
    pub tab_menu: Option<(gpui::EntityId, gpui::Point<gpui::Pixels>)>,
    /// The flags drawer: the chat it is for, and its pending changes.
    pub flags: Option<crate::flagsui::Drawer>,
    /// History: its search box, filters and scroll.
    pub hist: crate::history::Hist,
    /// A terminal shown as a floating sheet over the board (focus it on the next frame).
    pub float: Option<Entity<TermTab>>,
    pub focus_float: bool,
    pub solo: Option<String>,
    pub focus: FocusHandle,
    pub sel: Option<String>,
    /// The details popped out into their own window (detached.rs), which follows the selection; None = the sheet.
    pub detached: Option<gpui::AnyWindowHandle>,
    /// Lanes showing every row instead of the cap.
    pub open: HashSet<String>,
    pub overlay: Overlay,
    /// The Week chip's usage panel is open; the bar under the pointer there (a day of this week, or a past week).
    pub plan_open: bool,
    pub usage_hover: Option<UsageBar>,
    /// A terminal tab dragged along the tab row: (the tab, the gap it would drop into).
    pub tab_drop: Option<(usize, usize)>,
    /// The terminal tab under the pointer: the dividers on both sides of it hide, as in Chrome.
    pub tab_hover: Option<usize>,
    /// The tab row is crowded (tabs down to their dots): the header's "Updated ..." line gives its room to the tabs.
    pub tabs_crowded: std::cell::Cell<bool>,
    /// The tab row's sideways scroll (many tabs on a narrow window), and the tab last scrolled into sight.
    pub tab_scroll: ScrollHandle,
    pub tab_scrolled: std::cell::Cell<(usize, usize, i32)>,
    pub tab_scroll_pending: std::cell::Cell<bool>,
    /// The tab row's width last frame; a change draws one more frame so the tab that shows is brought into sight.
    tab_row_w: f32,
    /// The tab row's arrows: where a move under way is headed, that move (a newer one cancels it), and the press
    /// being held (released or newer, it stops repeating).
    pub tab_glide_to: Option<f32>,
    pub tab_glide: u64,
    pub tab_held: u64,
    /// Tab groups (tabgroups.rs): a chip's menu (which group, where), the group whose name is being typed on its
    /// chip and that field, the tab menu's "Add to group" list, the group a dragged tab would join, and a dragged
    /// chip's group with the place among the groups it would move to.
    pub group_menu: Option<(String, gpui::Point<gpui::Pixels>)>,
    pub group_edit: Option<String>,
    pub group_name: Entity<InputState>,
    /// A chat being renamed (rename.rs): which one and where its field shows, and that field. Picked from somewhere
    /// without a window (the right-click menu), it opens on the next frame.
    pub chat_edit: Option<crate::rename::ChatEdit>,
    pub chat_name: Entity<InputState>,
    /// The empty board's address field (firstrun.rs), and why the last address typed there was not taken.
    pub server_field: Entity<InputState>,
    pub server_err: Option<String>,
    /// The empty board's search for a server (firstrun.rs): what it found, whether the unreachable screen shows it
    /// instead of the saved address, whether the address field is open under "Type an address", and the server being
    /// connected to.
    pub find: sb_core::firstrun::Search,
    pub find_view: bool,
    pub type_open: bool,
    pub joining: Option<sb_core::firstrun::Found>,
    /// `shots`: the empty board shows a fixed search state and nothing is searched.
    pub find_shot: bool,
    /// "Run the server on this computer" (serversetup.rs).
    pub setup: Option<crate::serversetup::Setup>,
    pub(crate) pending_rename: Option<String>,
    pub tab_menu_sub: bool,
    pub drop_group: Option<String>,
    pub group_drop: Option<(String, usize)>,
    /// The once-a-second redraw that moves the live state (a warm countdown) along while a terminal tab shows.
    live_ticking: bool,
    /// The window's width at the last frame: how much room the terminal tabs have.
    pub win_w: f32,
    /// "How is this estimated?" under Resume or Start fresh is open.
    pub how_open: bool,
    /// The chat whose Continue choices are showing in the inspector.
    pub cont_open: Option<String>,
    /// The inspector's Attach tool picker: which chat, what is checked, and where it stands.
    pub tools_pick: Option<crate::inspector::ToolsPick>,
    /// The right-click menu: which chat, and where the pointer was.
    pub menu: Option<(String, gpui::Point<gpui::Pixels>)>,
    pub confirm: Option<(String, Confirm)>,
    /// The confirm is the fix "Restart with Chrome" (not Start fresh).
    pub confirm_chrome: bool,
    /// The confirm is closing this local terminal, which still runs a program (not Start fresh).
    pub confirm_close: Option<Entity<TermTab>>,
    /// The details sheet's "Needs you" card: its picks and text fields (askcard.rs), and whether one is typing.
    pub ask: Option<crate::askcard::AskForm>,
    pub ask_focus: bool,
    /// Chats whose reply from the card is on its way into their terminal: Send waits for it.
    pub ask_sending: std::collections::HashSet<String>,
    /// A chat being restarted with Chrome from its card, until the relaunch starts or asks for its terminal.
    pub chrome_pending: Option<String>,
    /// The note of the action under the pointer.
    pub hint: Option<SharedString>,
    pub pal: Entity<InputState>,
    pub pal_idx: usize,
    pub pal_scroll: gpui::UniformListScrollHandle,
    pub board_scroll: ScrollHandle,
    /// The details panel, the flags drawer, the notices tray and the stopped list: each wears the shared scrollbar.
    pub ins_scroll: ScrollHandle,
    pub flags_scroll: ScrollHandle,
    pub tray_scroll: ScrollHandle,
    pub clean_scroll: ScrollHandle,
    pub arc_scroll: gpui::UniformListScrollHandle,
    /// The open dropdown on the board (the flags drawer's effort or permissions), if any.
    pub drop: ui::Drop,
    /// Rows per cell before "n more", from the window height.
    pub cap: usize,
    /// Terminal tabs, after the board. `tab` 0 is the board, n is `tabs[n - 1]`.
    pub tabs: Vec<Entity<TermTab>>,
    /// Set while the last launch's tabs reopen, so each one opening does not save a half-built list.
    restoring: bool,
    /// The last launch's tabs reopen once live data is in (the saved snapshot can miss newer chats), or after a few
    /// seconds without it.
    restore_due: bool,
    /// Saved tabs the snapshot did not know when they reopened without live data: they stay saved, and open once the
    /// server answers.
    restore_left: Vec<String>,
    /// Moves and resizes of the window: its frame is saved once it has been still for a moment, or on the way out.
    bounds: Debounce,
    pub tab: usize,
    /// Focus the current tab on the next frame (actions without a window switch tabs).
    pub(crate) focus_tab: bool,
    /// The palette lists only chats, to open in a terminal (Cmd+O).
    pub pal_chats: bool,
    /// "This continues..." was picked where there is no window to build its sheet: the next frame opens it.
    pending_link: Option<String>,
    /// "Stopped today" in the status bar: its popover is open; how many it held last frame, and when it last grew
    /// (it pulses then).
    pub stopped_open: bool,
    /// The Notices tray is open (notices.rs).
    pub notices_open: bool,
    /// The tray row whose Ignore asks for its confirm ("all" for Clear all).
    pub ignore_armed: Option<String>,
    pub stopped_seen: std::cell::Cell<Option<usize>>,
    pub stopped_flash: std::cell::Cell<f64>,
    shot: Option<String>,
    /// Each terminal tab's events (tabs in the row and the floating sheet), dropped when it closes or leaves.
    tab_subs: HashMap<gpui::EntityId, Subscription>,
    /// The New or "This continues..." sheet's events, for the one open now.
    sheet_sub: Option<Subscription>,
    pub(crate) _subs: Vec<Subscription>,
}

impl BoardView {
    pub fn new(solo: Option<String>, window: &mut Window, cx: &mut Context<Self>) -> BoardView {
        let app = crate::state::app(cx);
        let focus = cx.focus_handle();
        let pal = cx.new(|cx| InputState::new(window, cx).placeholder("Search chats and commands"));
        let group_name = cx.new(|cx| InputState::new(window, cx).placeholder("Name this group"));
        let chat_name = cx.new(|cx| InputState::new(window, cx).placeholder("Name this chat"));
        let server_field = cx.new(|cx| InputState::new(window, cx).placeholder("192.0.2.10:8130"));
        let mut subs = vec![
            cx.observe(&app, |this, app, cx| {
                if this.restore_due && app.read(cx).live {
                    this.restore_due = false;
                    this.restore_tabs(cx);
                } else if !this.restore_left.is_empty() && app.read(cx).live {
                    this.restore_late(cx);
                }
                // a chat renamed anywhere: its tab says the new name; after /clear, its key follows the chat
                let mut moved = false;
                for tab in this.tabs.iter().chain(this.float.iter()).cloned().collect::<Vec<_>>() {
                    tab.update(cx, |tb, cx| {
                        let key = tb.sync_key(cx);
                        if tb.sync_title(cx) || key {
                            cx.notify();
                        }
                        moved |= key;
                    });
                }
                if moved {
                    this.remember_tabs(cx);
                }
                cx.notify()
            }),
            cx.subscribe_in(&app, window, |this, _, ev: &AppEvent, window, cx| match ev {
                AppEvent::Select(key) => {
                    let mine = match &this.solo {
                        Some(m) => key.starts_with(&format!("{m}:")),
                        None => true,
                    };
                    if mine {
                        this.select(Some(key.clone()), cx);
                        window.activate_window();
                    }
                }
                // Terminal tabs follow their own jobs.
                AppEvent::JobDone { .. } | AppEvent::JobFailed { .. } => {}
                // From the session browser: the board window opens the terminal tab.
                AppEvent::OpenTerm { key, resume } if this.solo.is_none() => {
                    this.open_terminal(key.clone(), *resume, cx);
                    window.activate_window();
                }
                AppEvent::OpenJobTerm { job, machine, label, title } if this.solo.is_none() => {
                    this.open_job_terminal(job.clone(), machine.clone(), label.clone(), title.clone(), cx);
                    window.activate_window();
                }
                AppEvent::NewIn { machine, folder } if this.solo.is_none() => {
                    this.open_new_in(machine.clone(), folder.clone(), window, cx);
                    window.activate_window();
                }
                AppEvent::OpenTerm { .. } | AppEvent::OpenJobTerm { .. } | AppEvent::NewIn { .. } => {}
                AppEvent::Appearance => this.apply_theme(window, cx),
                AppEvent::Flags(k) if this.solo.is_none() => {
                    this.open_flags(k.clone(), cx);
                    window.activate_window();
                }
                AppEvent::Flags(_) => {}
                AppEvent::PasteInto { key, text } if this.solo.is_none() => {
                    this.open_terminal(key.clone(), false, cx);
                    if let Some(tab) =
                        this.tabs.iter().find(|t| t.read(cx).key.as_deref() == Some(key.as_str())).cloned()
                    {
                        let text = text.clone();
                        tab.update(cx, |t, cx| t.paste_when_open(text, cx));
                    }
                    window.activate_window();
                }
                AppEvent::PasteInto { .. } => {}
                AppEvent::Relaunching { key } => {
                    this.ask_relaunch_news(key, None, cx);
                    this.flags_relaunching(key, cx);
                }
                // Not while it waits for the chat's reply to end: the terminal stays live, and a Cancel leaves it so.
                AppEvent::RelaunchStarted { key, job } => {
                    for tab in this.tabs.clone() {
                        if tab.read(cx).key.as_deref() == Some(key.as_str()) {
                            let j = job.clone();
                            tab.update(cx, |t, cx| t.follow_job(j, cx));
                        }
                    }
                }
                AppEvent::RelaunchCmd { key, command, error } => {
                    this.ask_relaunch_news(key, Some((command.clone(), error.clone())), cx);
                    this.flags_need_terminal(key, command.clone(), error.clone(), cx);
                    this.tools_need_terminal(key, command.clone(), error.clone(), cx);
                }
            }),
            cx.subscribe_in(&group_name, window, |this, _, ev: &InputEvent, _, cx| match ev {
                InputEvent::PressEnter { .. } | InputEvent::Blur => this.commit_group_name(cx),
                _ => {}
            }),
            cx.subscribe_in(&server_field, window, |this, _, ev: &InputEvent, window, cx| match ev {
                InputEvent::PressEnter { .. } => this.connect_server(window, cx),
                InputEvent::Change if this.server_err.is_some() => {
                    this.server_err = None;
                    cx.notify();
                }
                _ => {}
            }),
            cx.subscribe_in(&chat_name, window, |this, _, ev: &InputEvent, _, cx| match ev {
                InputEvent::PressEnter { .. } => this.commit_chat_name(true, cx),
                InputEvent::Blur => this.commit_chat_name(false, cx),
                _ => {}
            }),
            cx.subscribe_in(&pal, window, |this, _, ev: &InputEvent, _, cx| {
                if let InputEvent::Change = ev {
                    this.pal_idx = 0;
                    this.pal_scroll.scroll_to_item(0, gpui::ScrollStrategy::Top);
                    cx.notify();
                }
            }),
            // Back on screen (or in front again): make sure frames are flowing, no corner drag needed.
            crate::wake::watch(window),
            // In front again: frames flowing. The terminals need no nudge: the hub keeps their screens (Redraw, Cmd+Shift+R,
            // sends one again by hand).
            cx.observe_window_activation(window, |_, window, cx| {
                if window.is_window_active() {
                    crate::wake::soon(window, cx);
                }
            }),
        ];
        if solo.is_none() {
            subs.push(cx.observe_window_appearance(window, |this, window, cx| {
                this.apply_theme(window, cx);
            }));
            // Moved or resized: remember it for the next launch. Full screen keeps the size it had before.
            subs.push(cx.observe_window_bounds(window, |this, window, cx| {
                if std::env::var("SB_SIZE").is_ok() {
                    return;
                }
                let (b, maximized) = match window.window_bounds() {
                    WindowBounds::Windowed(b) => (b, false),
                    WindowBounds::Maximized(b) => (b, true),
                    WindowBounds::Fullscreen(b) => (b, false),
                };
                let saved = crate::state::SavedWindow {
                    x: b.origin.x.into(),
                    y: b.origin.y.into(),
                    w: b.size.width.into(),
                    h: b.size.height.into(),
                    maximized,
                };
                if this.app.read(cx).settings.window == Some(saved) {
                    return;
                }
                this.app.update(cx, |s, _| s.settings.window = Some(saved));
                // A drag or a resize sends many of these: the file is written once the window has been still for
                // half a second.
                let n = this.bounds.bump();
                cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(std::time::Duration::from_millis(500)).await;
                    let _ = this.update(cx, |this, cx| {
                        if this.bounds.settled(n) {
                            this.app.read(cx).save_settings();
                        }
                    });
                })
                .detach();
            }));
            // Quit or closed within that half second: the new frame is still written.
            subs.push(cx.on_app_quit(|this, cx| {
                if this.bounds.flush() {
                    this.app.read(cx).save_settings();
                }
                async {}
            }));
            subs.push(cx.on_release(|this, cx| {
                if this.bounds.flush() {
                    this.app.read(cx).save_settings();
                }
            }));
        }
        window.focus(&focus, cx);
        let shot = std::env::var("SB_SHOT").ok().filter(|s| !s.is_empty() && solo.is_none());
        let hist = crate::history::Hist::new(window, cx);
        let mut v = BoardView {
            app,
            main: Main::Board,
            spare_open: false,
            stop_armed: None,
            flight: None,
            freed_seen: std::cell::Cell::new(0.),
            freed_from: std::cell::Cell::new((0., 0.)),
            add_open: false,
            add_terms: false,
            app_menu_open: false,
            update_open: false,
            tab_menu: None,
            flags: None,
            hist,
            float: None,
            focus_float: false,
            solo,
            focus,
            sel: None,
            detached: None,
            open: HashSet::new(),
            overlay: Overlay::None,
            plan_open: false,
            usage_hover: None,
            tab_drop: None,
            tab_hover: None,
            tabs_crowded: std::cell::Cell::new(false),
            tab_scroll: ScrollHandle::new(),
            tab_scrolled: std::cell::Cell::new((usize::MAX, 0, 0)),
            tab_scroll_pending: std::cell::Cell::new(false),
            tab_row_w: 0.,
            tab_glide_to: None,
            tab_glide: 0,
            tab_held: 0,
            group_menu: None,
            group_edit: None,
            group_name,
            chat_edit: None,
            chat_name,
            server_field,
            server_err: None,
            find: sb_core::firstrun::Search::default(),
            find_view: false,
            type_open: false,
            joining: None,
            find_shot: false,
            setup: None,
            pending_rename: None,
            tab_menu_sub: false,
            drop_group: None,
            group_drop: None,
            live_ticking: false,
            win_w: 1440.,
            how_open: false,
            cont_open: None,
            tools_pick: None,
            menu: None,
            confirm: None,
            confirm_chrome: false,
            confirm_close: None,
            ask: None,
            ask_focus: false,
            ask_sending: Default::default(),
            chrome_pending: None,
            hint: None,
            pal,
            pal_idx: 0,
            pal_scroll: gpui::UniformListScrollHandle::new(),
            board_scroll: ScrollHandle::new(),
            ins_scroll: ScrollHandle::new(),
            flags_scroll: ScrollHandle::new(),
            tray_scroll: ScrollHandle::new(),
            clean_scroll: ScrollHandle::new(),
            arc_scroll: gpui::UniformListScrollHandle::new(),
            drop: ui::Drop::default(),
            cap: 4,
            tabs: Vec::new(),
            tab: 0,
            restoring: false,
            restore_due: false,
            restore_left: Vec::new(),
            bounds: Debounce::default(),
            focus_tab: false,
            pal_chats: false,
            pending_link: None,
            stopped_open: false,
            notices_open: false,
            ignore_armed: None,
            stopped_seen: std::cell::Cell::new(None),
            stopped_flash: std::cell::Cell::new(0.),
            shot,
            tab_subs: HashMap::new(),
            sheet_sub: None,
            _subs: subs,
        };
        if v.solo.is_none() {
            v.apply_theme(window, cx);
            // SB_TIMING=1 prints when the first frame was on screen (the callback runs once it has been drawn).
            if std::env::var_os("SB_TIMING").is_some() {
                let chats = v.app.read(cx).snap.sessions().len();
                window.on_next_frame(move |_, _| {
                    let ms = crate::STARTED.get().map(|t| t.elapsed().as_secs_f64() * 1000.0).unwrap_or(-1.0);
                    let epoch = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs_f64())
                        .unwrap_or(0.0);
                    eprintln!("sb-timing first-frame main_ms={ms:.1} epoch={epoch:.3} chats_from_cache={chats}");
                });
            }
            #[cfg(feature = "shots")]
            if let Some(out) = std::env::var_os("SB_SHOT_OUT") {
                capture_when_ready(out.into(), window, cx);
            }
            if v.shot.is_none() && std::env::var("SB_SIZE").is_err() {
                v.restore_due = true;
                cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(std::time::Duration::from_secs(8)).await;
                    let _ = this.update(cx, |this, cx| {
                        if this.restore_due {
                            this.restore_due = false;
                            this.restore_tabs(cx);
                        }
                    });
                })
                .detach();
            }
        }
        v
    }

    /// Follows the system unless a theme was picked.
    pub fn apply_theme(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mode = Mode::parse(self.app.read(cx).settings.theme.as_deref());
        let dark = match mode {
            Mode::Dark => true,
            Mode::Light => false,
            Mode::System => matches!(window.appearance(), WindowAppearance::Dark | WindowAppearance::VibrantDark),
        };
        let t = if dark { Theme::dark() } else { Theme::light() };
        if cx.try_global::<Theme>().map(|x| x.dark) != Some(dark) {
            t.install(cx);
            cx.refresh_windows();
        }
    }

    // ---------- selection and keys ----------

    fn board(&self, cx: &App) -> Board {
        let app = self.app.read(cx);
        board::build(&app.snap, AppState::now(), &app.hidden_set(), self.solo.as_deref())
    }

    pub fn cap_for(&self, c: Column) -> usize {
        if c.quiet() { self.cap * 2 } else { self.cap }
    }

    fn order(&self, cx: &App) -> Vec<String> {
        let b = self.board(cx);
        let app = self.app.read(cx);
        let (open, shut): (HashSet<String>, HashSet<String>) = if self.solo.is_some() {
            (b.lanes.iter().map(|l| l.machine.clone()).collect(), HashSet::new())
        } else {
            (self.open.clone(), app.settings.shut.iter().cloned().collect())
        };
        b.order(|c| self.cap_for(c), &open, &shut)
    }

    pub fn select(&mut self, key: Option<String>, cx: &mut Context<Self>) {
        if key != self.sel {
            self.ins_scroll.set_offset(gpui::point(px(0.), px(0.)));
        }
        self.cont_open = None;
        self.hint = None;
        if let Some(k) = &key {
            let b = self.board(cx);
            if let Some((li, col, row)) = b.locate(k) {
                let machine = b.lanes[li].machine.clone();
                if self.solo.is_none() && self.app.read(cx).is_shut(&machine) {
                    self.app.update(cx, |s, cx| s.set_shut(&machine, false, cx));
                }
                if row >= self.cap_for(col) {
                    self.open.insert(machine);
                }
                self.board_scroll.scroll_to_item(li);
            }
        }
        self.sel = key;
        cx.notify();
    }

    fn step(&mut self, d: i32, cx: &mut Context<Self>) {
        if let Overlay::Archive { tab, key } = &self.overlay {
            let (tab, key) = (*tab, key.clone());
            let list = crate::archive::rows(self.app.read(cx), tab);
            if list.is_empty() {
                return;
            }
            let i = key.and_then(|k| list.iter().position(|x| *x == k));
            let n = match i {
                None => 0,
                Some(i) => (i as i32 + d).clamp(0, list.len() as i32 - 1) as usize,
            };
            self.overlay = Overlay::Archive { tab, key: Some(list[n].clone()) };
            self.arc_scroll.scroll_to_item(n, gpui::ScrollStrategy::Nearest);
            self.cont_open = None;
            cx.notify();
            return;
        }
        if !self.overlay.is_none() || self.confirm.is_some() {
            return;
        }
        let order = self.order(cx);
        if order.is_empty() {
            return;
        }
        let i = self.sel.as_ref().and_then(|k| order.iter().position(|x| x == k));
        let n = match i {
            None => {
                if d > 0 {
                    0
                } else {
                    order.len() - 1
                }
            }
            Some(i) => (i as i32 + d).clamp(0, order.len() as i32 - 1) as usize,
        };
        self.select(Some(order[n].clone()), cx);
    }

    fn on_next(&mut self, _: &Next, _: &mut Window, cx: &mut Context<Self>) {
        self.step(1, cx);
    }
    fn on_prev(&mut self, _: &Prev, _: &mut Window, cx: &mut Context<Self>) {
        self.step(-1, cx);
    }
    fn across(&mut self, dir: i32, cx: &mut Context<Self>) {
        if !self.overlay.is_none() || self.confirm.is_some() {
            return;
        }
        if let Some(k) = self.board(cx).across(self.sel.as_deref(), dir) {
            self.select(Some(k), cx);
        }
    }
    fn on_left(&mut self, _: &Left, _: &mut Window, cx: &mut Context<Self>) {
        self.across(-1, cx);
    }
    fn on_right(&mut self, _: &Right, _: &mut Window, cx: &mut Context<Self>) {
        self.across(1, cx);
    }

    /// The chat the keys act on: the archive's selection while the archive is open.
    pub fn target(&self) -> Option<String> {
        match &self.overlay {
            Overlay::Archive { key, .. } => key.clone(),
            Overlay::None => self.sel.clone(),
            _ => None,
        }
    }

    fn on_activate(&mut self, _: &Activate, window: &mut Window, cx: &mut Context<Self>) {
        // A button reached with Tab has the focus: Enter presses that button, not the selected card.
        if self.key_context().contains("keys") && window.focused(cx).is_some_and(|f| f != self.focus) {
            cx.propagate();
            return;
        }
        if self.app.read(cx).trust_ask.is_some() {
            self.app.update(cx, |s, cx| s.answer_trust(true, cx));
            return;
        }
        if self.app.read(cx).busy_ask.is_some() {
            // Enter takes the default: when they finish
            self.app.update(cx, |s, cx| s.answer_busy(Some("idle"), cx));
            return;
        }
        if self.confirm.is_some() {
            self.confirm_ok(cx);
            return;
        }
        match &self.overlay {
            Overlay::Palette => return self.pal_run(self.pal_idx, window, cx),
            Overlay::New(_) | Overlay::Link(_) => return,
            _ => {}
        }
        let Some(key) = self.target() else {
            return self.step(1, cx);
        };
        let newer = self.app.read(cx).snap.replaced_row(&key).map(|r| r.newer_key());
        match newer {
            Some(Some(n)) => {
                self.close_overlay(window, cx);
                self.select(Some(n), cx);
            }
            Some(None) => {}
            None => self.enter(key, cx),
        }
    }

    /// Enter or a double-click on a chat: a running background chat opens in a terminal tab, anything else in Claude.
    /// Double click or Enter on a card: a running background chat opens in a tab; a stopped one resumes into a tab
    /// (on whichever machine it lives). Only a chat open in a terminal on its machine, which Switchboard cannot attach
    /// to, or one that cannot be resumed (it ran outside the Claude folder) falls back to its Claude link.
    pub fn enter(&mut self, key: String, cx: &mut Context<Self>) {
        // Some(false): attach; Some(true): resume first; None: the Claude link.
        let tab = {
            let app = self.app.read(cx);
            match app.snap.session(&key) {
                Some(s) if s.alive && s.background() => Some(false),
                Some(s)
                    if !s.alive
                        && acts::for_session(s, &app.snap, AppState::now())
                            .iter()
                            .any(|a| a.id == ActionId::Resume && !a.disabled) =>
                {
                    Some(true)
                }
                _ => None,
            }
        };
        match tab {
            Some(resume) => self.open_terminal(key, resume, cx),
            None => self.app.update(cx, |s, cx| s.open(&key, cx)),
        }
    }

    fn on_fresh(&mut self, _: &Fresh, _: &mut Window, cx: &mut Context<Self>) {
        if matches!(self.overlay, Overlay::Palette | Overlay::New(_)) {
            return;
        }
        if let Some(k) = self.target() {
            self.run_action(ActionId::Fresh, k, cx);
        }
    }

    fn on_resume(&mut self, _: &Resume, _: &mut Window, cx: &mut Context<Self>) {
        if matches!(self.overlay, Overlay::Palette | Overlay::New(_)) {
            return;
        }
        let Some(k) = self.target() else { return };
        let app = self.app.read(cx);
        let now = AppState::now();
        let ok = match app.snap.session(&k) {
            Some(s) => acts::for_session(s, &app.snap, now).iter().any(|a| a.id == ActionId::Resume && !a.disabled),
            None => false,
        };
        if ok {
            self.app.update(cx, |s, cx| s.act(ActionId::Resume, &k, cx));
        }
    }

    fn on_continue(&mut self, _: &Continue, _: &mut Window, cx: &mut Context<Self>) {
        if matches!(self.overlay, Overlay::Palette | Overlay::New(_)) {
            return;
        }
        if let Some(k) = self.target()
            && self.app.read(cx).snap.session(&k).is_some()
        {
            self.cont_open = Some(k);
            cx.notify();
        }
    }

    fn on_new(&mut self, _: &NewSession, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(self.overlay, Overlay::New(_)) {
            return;
        }
        self.open_new(None, window, cx);
    }

    fn on_palette(&mut self, _: &TogglePalette, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(self.overlay, Overlay::Palette) {
            self.close_overlay(window, cx);
        } else if matches!(self.overlay, Overlay::New(_)) {
        } else {
            self.open_palette(window, cx);
        }
    }

    fn on_start_new(&mut self, _: &StartNew, window: &mut Window, cx: &mut Context<Self>) {
        if let Overlay::New(sheet) = &self.overlay {
            let sheet = sheet.clone();
            sheet.update(cx, |s, cx| if s.trust_open() { s.answer_trust(true, cx) } else { s.start(window, cx) });
        }
    }

    fn on_escape(&mut self, _: &Escape, window: &mut Window, cx: &mut Context<Self>) {
        if self.disarm_stop() {
        } else if self.ignore_armed.is_some() {
            self.ignore_armed = None;
        } else if self.notices_open {
            self.notices_open = false;
        } else if self.group_edit.is_some() {
            self.cancel_group_name(cx);
        } else if self.chat_edit.is_some() {
            self.cancel_chat_name(cx);
        } else if self.group_menu.is_some() || self.tab_menu.is_some() {
            self.group_menu = None;
            self.tab_menu = None;
            self.tab_menu_sub = false;
        } else if let Overlay::New(sheet) = &self.overlay
            && sheet.read(cx).trust_open()
        {
            let sheet = sheet.clone();
            sheet.update(cx, |s, cx| s.answer_trust(false, cx));
        } else if self.app.read(cx).trust_ask.is_some() {
            self.app.update(cx, |s, cx| s.answer_trust(false, cx));
        } else if self.app.read(cx).busy_ask.is_some() {
            self.app.update(cx, |s, cx| s.answer_busy(None, cx));
        } else if self.menu.is_some() {
            self.menu = None;
        } else if self.app_menu_open {
            self.app_menu_open = false;
        } else if self.plan_open {
            self.plan_open = false;
        } else if self.stopped_open {
            self.stopped_open = false;
        } else if self.confirm.is_some() {
            self.drop_confirm();
        } else if self.cont_open.is_some() {
            self.cont_open = None;
        } else if !self.overlay.is_none() {
            self.close_overlay(window, cx);
        } else if self.sel.is_some() {
            self.sel = None;
        }
        cx.notify();
    }

    fn on_close_window(&mut self, _: &CloseWindow, window: &mut Window, _: &mut Context<Self>) {
        window.remove_window();
    }

    /// Cmd+W closes what is on top: an open sheet or the palette, then the floating sheet, then the terminal tab
    /// showing, and on the board the window.
    fn on_close_tab(&mut self, _: &CloseTab, window: &mut Window, cx: &mut Context<Self>) {
        if !self.overlay.is_none() {
            self.close_overlay(window, cx);
        } else if self.float.is_some() {
            self.close_float(cx);
            self.focus_tab = true;
        } else if self.tab == 0 {
            window.remove_window();
        } else {
            self.close_tab(self.tab - 1, cx);
        }
    }

    fn on_new_terminal(&mut self, _: &NewTerminal, window: &mut Window, cx: &mut Context<Self>) {
        if !self.overlay.is_none() {
            self.close_overlay(window, cx);
        }
        self.open_local(None, cx);
    }

    fn on_chat_palette(&mut self, _: &ChatPalette, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(self.overlay, Overlay::Palette) && self.pal_chats {
            self.close_overlay(window, cx);
        } else if !matches!(self.overlay, Overlay::New(_)) {
            self.open_chat_palette(window, cx);
        }
    }

    // ---------- terminal tabs ----------

    /// Opens a chat's terminal in a tab, or goes to the tab it already has (reconnecting one that ended).
    /// `resume`: the chat is stopped; resume it first.
    pub fn open_terminal(&mut self, key: String, resume: bool, cx: &mut Context<Self>) {
        self.menu = None;
        self.overlay = Overlay::None;
        self.cont_open = None;
        self.app.update(cx, |s, cx| s.call_off_stops(&key, cx));
        if let Some(i) = self.tab_for(&key, cx) {
            let tab = self.tabs[i].clone();
            tab.update(cx, |t, cx| {
                if t.ended() {
                    if resume { t.resume(cx) } else { t.connect(cx) }
                }
            });
            self.set_tab(i + 1, cx);
            return;
        }
        let tab = cx.new(|cx| TermTab::new(if resume { Start::Resume(key) } else { Start::Chat(key) }, cx));
        self.add_tab(tab, cx);
    }

    /// A shell on this computer in a new tab: the profile with this id, or the default one (Settings > Terminal).
    pub fn open_local(&mut self, id: Option<&str>, cx: &mut Context<Self>) {
        self.menu = None;
        self.add_open = false;
        self.cont_open = None;
        let p = {
            let app = self.app.read(cx);
            let saved = id.or(app.settings.term_profile.as_deref());
            sb_core::term::shells::pick_default(crate::state::profiles(), saved).cloned()
        };
        let Some(p) = p else { return };
        let tab = cx.new(|cx| TermTab::new(Start::Local(p), cx));
        self.add_tab(tab, cx);
    }

    /// New session with "Open its terminal": a tab that waits for the job, then attaches.
    pub fn open_job_terminal(
        &mut self,
        job: String,
        machine: String,
        label: String,
        title: String,
        cx: &mut Context<Self>,
    ) {
        let tab = cx.new(|cx| TermTab::new(Start::Job { id: job, machine, label, title }, cx));
        self.add_tab(tab, cx);
    }

    fn add_tab(&mut self, tab: Entity<TermTab>, cx: &mut Context<Self>) {
        self.watch_tab(&tab, cx);
        self.tabs.push(tab);
        self.set_tab(self.tabs.len(), cx);
    }

    /// Follows a terminal in this window, in the tab row or as the floating sheet: its Close closes whichever it is.
    pub(crate) fn watch_tab(&mut self, tab: &Entity<TermTab>, cx: &mut Context<Self>) {
        let sub = cx.subscribe(tab, |this, tab, ev: &TermEvent, cx| match ev {
            TermEvent::Changed => {
                this.remember_tabs(cx); // a job's tab learns its chat once the job is done
                cx.notify()
            }
            TermEvent::Flags(k) => this.open_flags(k.clone(), cx),
            TermEvent::Close if this.float.as_ref() == Some(&tab) => this.close_float(cx),
            TermEvent::Close => {
                if let Some(i) = this.tabs.iter().position(|t| *t == tab) {
                    this.close_tab(i, cx);
                }
            }
        });
        self.tab_subs.insert(tab.entity_id(), sub);
    }

    /// A terminal leaves this window (closed, or off to a window of its own, which follows it there).
    pub(crate) fn unwatch_tab(&mut self, tab: &Entity<TermTab>) {
        self.tab_subs.remove(&tab.entity_id());
        if self.tab_menu.is_some_and(|(id, _)| id == tab.entity_id()) {
            self.tab_menu = None;
        }
    }

    /// Terminal tab `i`'s place in `tabs` now, from its id (a menu or a drag keeps the id: tabs may close meanwhile).
    pub fn tab_index(&self, id: gpui::EntityId) -> Option<usize> {
        self.tabs.iter().position(|t| t.entity_id() == id)
    }

    /// The chat's background id, which a terminal still holds after /clear moved the chat to a new key.
    fn bg_of(&self, key: &str, cx: &App) -> Option<String> {
        self.app.read(cx).snap.session(key).and_then(|s| s.bg_id.clone())
    }

    /// The tab (of `tabs`) holding the chat at `key`, by its key or its background id.
    pub fn tab_for(&self, key: &str, cx: &App) -> Option<usize> {
        let bg = self.bg_of(key, cx);
        self.tabs.iter().position(|t| t.read(cx).shows(key, bg.as_deref()))
    }

    /// The chat is still on screen somewhere other than tab `except` (the floating sheet, a terminal window, another
    /// tab), so closing that one does not stop it.
    fn shown_elsewhere(&self, key: &str, except: gpui::EntityId, cx: &App) -> bool {
        let bg = self.bg_of(key, cx);
        let holds = |t: &Entity<TermTab>| t.entity_id() != except && t.read(cx).shows(key, bg.as_deref());
        self.tabs.iter().any(holds)
            || self.float.as_ref().is_some_and(holds)
            || crate::place::window_shows(key, bg.as_deref(), cx)
    }

    /// Closing tab `tab`, which holds this chat, stops it: a background chat that runs, unless Keep running is on or
    /// it still shows somewhere else (force_close_tab).
    pub fn close_stops(&self, key: &str, tab: gpui::EntityId, cx: &App) -> bool {
        self.app.read(cx).snap.session(key).is_some_and(|s| s.background() && s.alive && !s.keep)
            && !self.shown_elsewhere(key, tab, cx)
    }

    /// Only the tab showing gets frames: the others stop counting as focused, so a bell there marks their tab.
    fn hide_others(&self, cx: &mut Context<Self>) {
        let shown = self.tab.checked_sub(1).and_then(|i| self.tabs.get(i));
        for t in self.tabs.iter().filter(|t| Some(*t) != shown) {
            t.update(cx, |t, _| t.hide());
        }
    }

    /// Closes tab `i` (of `tabs`): detaches, the chat keeps running. A local terminal's shell ends with it; while a
    /// program other than the shell runs there, it asks first.
    pub fn close_tab(&mut self, i: usize, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(i).cloned() else { return };
        if let Some(what) = tab.read(cx).busy() {
            self.confirm = Some((
                String::new(),
                Confirm {
                    title: "Close this terminal?".into(),
                    body: format!("{what} is still running in it. Closing the tab ends it."),
                    ok: "Close".into(),
                    cancel: "Cancel".into(),
                },
            ));
            self.confirm_chrome = false;
            self.confirm_close = Some(tab);
            // The keyboard leaves the terminal, so Enter and Esc answer the question instead of reaching the shell.
            self.focus_tab = true;
            return cx.notify();
        }
        self.force_close_tab(i, cx);
    }

    /// Closes tab `i` without asking.
    fn force_close_tab(&mut self, i: usize, cx: &mut Context<Self>) {
        if i >= self.tabs.len() {
            return;
        }
        let tab = self.tabs.remove(i);
        let key = tab.read(cx).key.clone();
        tab.update(cx, |t, _| t.detach());
        self.unwatch_tab(&tab);
        // Closing a chat's tab ends the chat, so nothing piles up in the background (Resume brings it back). One
        // that is working stops once its reply is done; Keep running, a chat open on its machine and one still
        // showing here (the floating sheet, a window) stay.
        if let Some(key) = key.filter(|k| !self.shown_elsewhere(k, tab.entity_id(), cx)) {
            let now = crate::state::AppState::now();
            let stop = self
                .app
                .read(cx)
                .snap
                .session(&key)
                .filter(|s| s.background() && s.alive && !s.keep)
                .map(|s| matches!(s.lamp(now), sb_core::Lamp::Run | sb_core::Lamp::Stall));
            if let Some(working) = stop {
                self.app.update(cx, |s, cx| s.stop_on_close(&key, working, cx));
            }
        }
        // Closing the current tab shows the one to its left (the board after the first terminal).
        if self.tab > i {
            self.tab -= 1;
        }
        self.tab = self.tab.min(self.tabs.len());
        self.focus_tab = true;
        self.hide_others(cx);
        self.remember_tabs(cx);
        cx.notify();
    }

    /// Opens the Week chip's usage panel and reads the latest numbers for it.
    pub fn open_plan(&mut self, cx: &mut Context<Self>) {
        self.plan_open = true;
        self.add_open = false;
        self.usage_hover = None;
        self.app.update(cx, |s, cx| s.load_usage(cx));
        cx.notify();
    }

    /// "Reply: <suggested reply>": opens the chat's terminal tab and types the reply there, then Enter. The card
    /// says Sent only once it went in.
    pub fn reply_with(&mut self, key: String, text: String, cx: &mut Context<Self>) {
        self.open_terminal(key.clone(), false, cx);
        let me = cx.entity().downgrade();
        let (k, t2) = (key.clone(), text.clone());
        let done = move |ok: bool, cx: &mut App| {
            let _ = me.update(cx, |v, cx| v.ask_sent(&k, &t2, ok, cx));
        };
        match self.tab_for(&key, cx).and_then(|i| self.tabs.get(i)).cloned() {
            Some(tab) => tab.update(cx, |t, cx| t.reply_when_open(text, done, cx)),
            None => cx.defer(move |cx| done(false, cx)),
        }
    }

    pub fn set_tab(&mut self, n: usize, cx: &mut Context<Self>) {
        self.tab = n.min(self.tabs.len());
        // Brought into sight even when it already showed (the arrows may have moved it out).
        self.tab_scrolled.set((usize::MAX, 0, 0));
        self.menu = None;
        self.plan_open = false;
        self.focus_tab = true;
        self.hide_others(cx);
        self.remember_tabs(cx);
        cx.notify();
    }

    /// Saves the board window's terminal tabs (their chats, in order) and which one shows, for the next launch. Only
    /// tabs that know their chat count: one still waiting on a job is saved once the job names it.
    pub fn remember_tabs(&mut self, cx: &mut Context<Self>) {
        if self.solo.is_some()
            || self.shot.is_some()
            || std::env::var("SB_SIZE").is_ok()
            || self.restoring
            || self.restore_due
        {
            return;
        }
        let keys: Vec<Option<String>> = self.tabs.iter().map(|t| t.read(cx).key.clone()).collect();
        // The tab showing, counted among the saved ones (0 is the board; an unsaved tab showing counts as the board).
        let at = match self.tab.checked_sub(1).and_then(|i| keys.get(i)) {
            Some(Some(_)) => keys[..self.tab].iter().filter(|k| k.is_some()).count(),
            _ => 0,
        };
        let keys = saved_keys(keys, &self.restore_left);
        self.app.update(cx, |s, _| {
            if s.settings.tabs != keys || s.settings.tab != at {
                s.settings.tabs = keys;
                s.settings.tab = at;
                s.save_settings();
            }
        });
    }

    /// Reopens the tabs that were open when Switchboard last closed. A chat that has since stopped or left the board
    /// is skipped (reopening it would mean resuming it, which costs tokens; that stays a click), except a starred
    /// one: its tab comes back saying it stopped, with Resume.
    fn restore_tabs(&mut self, cx: &mut Context<Self>) {
        let (keys, at) = {
            let s = self.app.read(cx);
            (s.settings.tabs.clone(), s.settings.tab)
        };
        if keys.is_empty() {
            return;
        }
        // Without live data yet (the server slow or away), a chat the saved snapshot does not know may still be
        // there: it stays in the saved list and opens once the server answers.
        let live = self.app.read(cx).live;
        self.restoring = true;
        let mut show = 0;
        let mut seen = std::collections::HashSet::new();
        for (i, key) in keys.iter().enumerate() {
            // a chat cleared or started fresh since: its tab reopens on the newer part, once
            let key = &self.app.read(cx).snap.latest_key(key);
            if !seen.insert(key.clone()) {
                continue;
            }
            let known = self.app.read(cx).snap.session(key).map(|s| (s.alive, s.star));
            if known.is_none() && !live {
                self.restore_left.push(key.clone());
                continue;
            }
            let (alive, star) = known.unwrap_or_default();
            if alive {
                self.open_terminal(key.clone(), false, cx);
            } else if star {
                let tab = cx.new(|cx| TermTab::new(Start::Stopped(key.clone()), cx));
                self.add_tab(tab, cx);
            } else {
                continue;
            }
            if i + 1 == at {
                show = self.tabs.len();
            }
        }
        self.restoring = false;
        self.set_tab(show, cx);
    }

    /// The saved tabs that waited for the server open now, after the others, as on a launch with live data (a chat
    /// that stopped meanwhile stays closed unless starred). The tab showing stays.
    fn restore_late(&mut self, cx: &mut Context<Self>) {
        for key in std::mem::take(&mut self.restore_left) {
            let key = self.app.read(cx).snap.latest_key(&key);
            if self.tabs.iter().any(|t| t.read(cx).key.as_deref() == Some(key.as_str())) {
                continue;
            }
            let (alive, star) = self.app.read(cx).snap.session(&key).map(|s| (s.alive, s.star)).unwrap_or_default();
            let start = if alive {
                Start::Chat(key)
            } else if star {
                Start::Stopped(key)
            } else {
                continue;
            };
            let tab = cx.new(|cx| TermTab::new(start, cx));
            self.watch_tab(&tab, cx);
            self.tabs.push(tab);
        }
        self.hide_others(cx);
        self.remember_tabs(cx);
        cx.notify();
    }

    /// Cmd+1..9: 1 the board, 2 to 8 the terminals in order, 9 the last tab.
    fn tab_key(&mut self, n: usize, cx: &mut Context<Self>) {
        if !self.overlay.is_none() && !matches!(self.overlay, Overlay::Archive { .. }) {
            return;
        }
        let to = if n == 9 { self.tabs.len() } else { n - 1 };
        if to <= self.tabs.len() {
            self.overlay = Overlay::None;
            self.set_tab(to, cx);
        }
    }

    /// Focus where typing should go: the current terminal, or the board (also while a confirm asks something).
    fn focus_current(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.tab.checked_sub(1).and_then(|i| self.tabs.get(i)).filter(|_| self.confirm.is_none()) {
            Some(t) => {
                let f = t.read(cx).focus.clone();
                window.focus(&f, cx);
            }
            None => window.focus(&self.focus, cx),
        }
    }

    // ---------- overlays ----------

    pub fn close_overlay(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.overlay = Overlay::None;
        self.cont_open = None;
        self.pal_chats = false;
        self.focus_current(window, cx);
        cx.notify();
    }

    pub fn open_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.plan_open = false;
        self.overlay = Overlay::Palette;
        self.pal_chats = false;
        self.pal_idx = 0;
        self.pal.update(cx, |p, cx| {
            p.set_placeholder("Search chats and commands", window, cx);
            p.set_value("", window, cx);
            p.focus(window, cx);
        });
        cx.notify();
    }

    /// The palette with chats only: Enter opens the chat in a terminal tab (Cmd+O).
    pub fn open_chat_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.plan_open = false;
        self.menu = None;
        self.overlay = Overlay::Palette;
        self.pal_chats = true;
        self.pal_idx = 0;
        self.pal_scroll.scroll_to_item(0, gpui::ScrollStrategy::Top);
        self.pal.update(cx, |p, cx| {
            p.set_placeholder("Open a chat in a terminal", window, cx);
            p.set_value("", window, cx);
            p.focus(window, cx);
        });
        cx.notify();
    }

    pub fn open_new(&mut self, machine: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        self.plan_open = false;
        let machine = machine
            .or_else(|| self.solo.clone())
            .or_else(|| self.sel.as_ref().and_then(|k| k.split_once(':').map(|(m, _)| m.to_string())));
        let app = self.app.clone();
        let sheet = cx.new(|cx| NewSheet::new(app, machine, window, cx));
        let sub = cx.subscribe_in(&sheet, window, |this, _, ev: &SheetEvent, window, cx| match ev {
            SheetEvent::Close => this.close_overlay(window, cx),
            SheetEvent::Started { job, machine, label, title } => {
                this.open_job_terminal(job.clone(), machine.clone(), label.clone(), title.clone(), cx)
            }
        });
        self.sheet_sub = Some(sub);
        sheet.update(cx, |s, cx| s.focus_message(window, cx));
        self.overlay = Overlay::New(sheet);
        cx.notify();
    }

    /// From Files: New with that machine and folder picked.
    pub fn open_new_in(&mut self, machine: String, folder: String, window: &mut Window, cx: &mut Context<Self>) {
        self.open_new(Some(machine), window, cx);
        if let Overlay::New(s) = &self.overlay {
            s.update(cx, |s, cx| s.preset_folder(folder, cx));
        }
    }

    /// "This continues..." for the chat at `key`.
    pub fn open_link(&mut self, key: String, window: &mut Window, cx: &mut Context<Self>) {
        if self.app.read(cx).snap.session(&key).is_none() {
            return;
        }
        self.plan_open = false;
        self.menu = None;
        self.cont_open = None;
        let app = self.app.clone();
        let sheet = cx.new(|cx| LinkSheet::new(app, key, window, cx));
        let sub = cx.subscribe_in(&sheet, window, |this, _, ev: &LinkEvent, window, cx| match ev {
            LinkEvent::Close => this.close_overlay(window, cx),
        });
        self.sheet_sub = Some(sub);
        sheet.update(cx, |s, cx| s.focus(window, cx));
        self.overlay = Overlay::Link(sheet);
        cx.notify();
    }

    pub fn open_archive(&mut self, tab: ArcTab, key: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        self.plan_open = false;
        let key = key.or_else(|| crate::archive::rows(self.app.read(cx), tab).into_iter().next());
        self.overlay = Overlay::Archive { tab, key };
        self.cont_open = None;
        window.focus(&self.focus, cx);
        cx.notify();
    }

    pub fn tear(&mut self, machine: String, cx: &mut Context<Self>) {
        let label = self.app.read(cx).snap.label_of(&machine).unwrap_or_else(|| machine.clone());
        open_window(Some(machine), cx);
        self.app.update(cx, |s, cx| s.toast(format!("Opened the {label} lane in its own window"), None, false, cx));
    }

    /// What an inspector button does.
    pub fn run_action(&mut self, id: ActionId, key: String, cx: &mut Context<Self>) {
        let app = self.app.read(cx);
        match id {
            ActionId::Fresh => {
                if let Some(c) = app.fresh_confirm(&key) {
                    self.confirm = Some((key, c));
                    self.confirm_chrome = false;
                }
            }
            ActionId::Continue => {
                self.cont_open = if self.cont_open.as_deref() == Some(key.as_str()) { None } else { Some(key) };
            }
            ActionId::Newer => {
                if let Some(n) = app.snap.replaced_row(&key).and_then(|r| r.newer_key()) {
                    self.overlay = Overlay::None;
                    self.select(Some(n), cx);
                }
            }
            ActionId::Terminal => return self.open_terminal(key, false, cx),
            ActionId::ResumeOpen => return self.open_terminal(key, true, cx),
            ActionId::Link => {
                self.menu = None;
                self.pending_link = Some(key);
            }
            ActionId::Rename => {
                self.menu = None;
                self.pending_rename = Some(key);
            }
            ActionId::Dismiss => {
                self.app.update(cx, |s, cx| s.act(ActionId::Dismiss, &key, cx));
                if self.sel.as_deref() == Some(key.as_str()) {
                    self.sel = None;
                }
                if let Overlay::Archive { tab, key: k } = &mut self.overlay {
                    if k.as_deref() == Some(key.as_str()) {
                        *k = None;
                    }
                    let _ = tab;
                }
            }
            _ => self.app.update(cx, |s, cx| s.act(id, &key, cx)),
        }
        cx.notify();
    }

    /// Screenshot states for visual checks (SB_SHOT=inspector,palette,new,archive,plan,confirm).
    /// "Board", plus "keys" when single-letter shortcuts are safe (nothing open that takes typing).
    fn key_context(&self) -> gpui::KeyContext {
        let mut kc = gpui::KeyContext::new_with_defaults();
        kc.add("Board");
        // History embeds the browser, whose search box takes letters: c, f, r, n... must type there, not act.
        // The details card's fields too (askcard.rs).
        if self.tab == 0
            && self.main == Main::Board
            && matches!(self.overlay, Overlay::None | Overlay::Archive { .. })
            && self.confirm.is_none()
            && !self.ask_focus
            && self.group_edit.is_none()
            && self.chat_edit.is_none()
        {
            kc.add("keys");
        }
        kc
    }

    fn apply_shot(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(shot) = self.shot.clone() else { return };
        if self.app.read(cx).snap.machines.is_empty() {
            return;
        }
        self.shot = None;
        for part in shot.split(',') {
            let (what, arg) = part.split_once('=').unwrap_or((part, ""));
            match what {
                "inspector" => {
                    let key = if arg.is_empty() {
                        self.order(cx).into_iter().nth(2)
                    } else {
                        Some(arg.replacen('/', ":", 1))
                    };
                    self.select(key, cx);
                }
                "palette" => {
                    self.open_palette(window, cx);
                    if !arg.is_empty() {
                        self.pal.update(cx, |p, cx| p.set_value(arg, window, cx));
                    }
                }
                "new" => self.open_new(None, window, cx),
                "ultra" => {
                    if let Overlay::New(s) = &self.overlay {
                        s.update(cx, |s, cx| s.set_mode("ultracode", cx));
                    }
                }
                "archive" => {
                    self.open_archive(if arg == "rep" { ArcTab::Replaced } else { ArcTab::Older }, None, window, cx)
                }
                "plan" => self.open_plan(cx),
                "confirm" => {
                    if let Some(k) = self.sel.clone() {
                        self.run_action(ActionId::Fresh, k, cx);
                    }
                }
                "busy" => {
                    // What the server answers when a stop would cut off a chat's work (the fake fleet has no busy verb).
                    let key = self.sel.clone().unwrap_or_default();
                    let info = sb_core::BusyInfo {
                        subagents: vec!["Research PPF shops".into(), "Audit the hub logs".into()],
                        commands: vec!["background command".into()],
                        summary: "2 subagents and 1 background command are still running.".into(),
                        ..Default::default()
                    };
                    self.app.update(cx, |s, cx| {
                        s.busy_ask = Some(crate::state::BusyAsk {
                            what: "Stop it",
                            message: "2 subagents and 1 background command are still running. Stop it when they finish, or now, which cuts them off.".into(),
                            busy: vec![info],
                            retry: crate::state::BusyRetry::Act { id: ActionId::Stop, key },
                        });
                        cx.notify();
                    });
                }
                "continue" => {
                    if let Some(k) = self.sel.clone() {
                        self.cont_open = Some(k);
                    }
                }
                // term=<machine:sid>: open that chat's terminal; term without a key, the selected chat's.
                "term" => {
                    // (machine/sid also works: the shots script splits its arguments on ':')
                    if let Some(k) = if arg.is_empty() { self.sel.clone() } else { Some(arg.replacen('/', ":", 1)) } {
                        self.open_terminal(k, false, cx);
                    }
                }
                // Tab groups: fold=<group id> folds one; groupmenu=<group id> opens its chip's menu; tabmenu=<i>
                // opens tab i's menu, tabmenu=<i>/sub with its "Add to group" list.
                // live=<run|warm|warmend|cold|wait|parked>: the shown terminal's strip in that state.
                "live" => {
                    if let Some(tb) = self.tab.checked_sub(1).and_then(|i| self.tabs.get(i)).cloned() {
                        tb.update(cx, |tb, cx| tb.shot_live(arg, cx));
                    }
                }
                "fold" => self.toggle_fold(arg, cx),
                "groupmenu" => self.group_menu = Some((arg.to_string(), gpui::point(px(360.), px(ui::header() - 6.)))),
                // rename=<i>: tab i with its name field open; rename=<machine/sid>: that chat's, where it would open.
                "rename" => match arg.parse::<usize>() {
                    Ok(i) => self.rename_tab(i, window, cx),
                    Err(_) => self.rename_chat(arg.replacen('/', ":", 1), window, cx),
                },
                // groupname=<group id>: its chip with the name field open.
                "groupname" => self.edit_group_name(arg.to_string(), window, cx),
                "tabmenu" => {
                    let (i, sub) = arg.split_once('/').unwrap_or((arg, ""));
                    if let Ok(i) = i.parse::<usize>() {
                        self.open_tab_menu(i, gpui::point(px(420.), px(ui::header() - 8.)), cx);
                        self.tab_menu_sub = sub == "sub";
                    }
                }
                // termstop=<machine/sid>: the tab a starred chat that stopped comes back as on launch.
                "termstop" => {
                    let key = arg.replacen('/', ":", 1);
                    let tab = cx.new(|cx| TermTab::new(Start::Stopped(key), cx));
                    self.add_tab(tab, cx);
                }
                "chats" => self.open_chat_palette(window, cx),
                // Lanes and the parity pass: flags (the drawer for the selected chat), flagpick (a pending change),
                // history, spare (the not-in-use list open), add (the + menu), stopped (its popover), stoparm (an armed stop).
                "flags" => {
                    let k = self.sel.clone().or_else(|| self.order(cx).into_iter().nth(1));
                    if let Some(k) = k {
                        self.open_flags(k, cx);
                    }
                }
                // menu (the right-click menu on the first cold chat with costs), menu=<machine/sid>.
                "menu" => {
                    let now = AppState::now();
                    let k = if arg.is_empty() {
                        self.order(cx).into_iter().find(|k| {
                            self.app
                                .read(cx)
                                .snap
                                .session(k)
                                .is_some_and(|s| s.cost.is_some() && s.cache_left(now) == 0 && !s.alive)
                        })
                    } else {
                        Some(arg.replacen('/', ":", 1))
                    };
                    if let Some(k) = k {
                        self.open_menu(k, gpui::point(px(420.), px(200.)), cx);
                    }
                }
                "history" => self.view_history(cx),
                // popout: the first chat's details in their own window, opened the way the sheet's button does it.
                "popout" => {
                    let k = self.order(cx).into_iter().next();
                    self.select(k, cx);
                    self.pop_out(cx);
                }
                "spare" => self.spare_open = true,
                "add" => self.add_open = true,
                // addterm: the chevron's menu, the shells first. local=<n>: n tabs with a shell on this computer.
                "addterm" => {
                    self.add_open = true;
                    self.add_terms = true;
                }
                "local" => {
                    for _ in 0..arg.parse::<usize>().unwrap_or(1).max(1) {
                        self.open_local(None, cx);
                    }
                }
                "appmenu" => self.app_menu_open = true,
                "stopped" => self.stopped_open = true,
                "files" => cx.defer(crate::files::open_files),
                // newin=<machine>/<folder>: New in that folder, as Files' New chat here does.
                "newin" => {
                    let (m, f) = arg.split_once('/').unwrap_or((arg, ""));
                    self.open_new_in(m.to_string(), f.to_string(), window, cx);
                }
                // newtrust: the New sheet's trust question for the folder it is in.
                "newtrust" => {
                    if let Overlay::New(sheet) = &self.overlay {
                        sheet.update(cx, |s, cx| s.start(window, cx));
                    }
                }
                // trust=<declined 0|1>: Continue's trust question over the board.
                "trust" => {
                    let key = self.sel.clone().unwrap_or_default();
                    self.app.update(cx, |s, cx| {
                        s.trust_ask = Some(crate::state::TrustAsk {
                            shown: "~/Documents/Papers".into(),
                            declined: arg == "1",
                            on: "Studio".into(),
                            retry: crate::state::TrustRetry::Continue { key, to: "studio".into(), when: None },
                        });
                        cx.notify();
                    });
                }
                "update" => {
                    self.app.update(cx, |s, _| {
                        s.update = Some(sb_core::update::Offer {
                            from: sb_core::version().into(),
                            version: "9.0.0".into(),
                            size: 1,
                            build: "20261013120000".into(),
                            built: Some(AppState::now() - 3600.),
                            url: String::new(),
                            sha256: String::new(),
                            file: "Switchboard-mac.zip".into(),
                            page: String::new(),
                        })
                    });
                    self.update_open = arg == "open";
                }
                // settings=machine/<name> opens a machine's page; settings=pick/<name>/<path> its folder picker there.
                "settings" if arg.starts_with("machine/") || arg.starts_with("pick/") => {
                    let mut it = arg.splitn(3, '/');
                    let (what, name, path) = (
                        it.next().unwrap_or_default().to_string(),
                        it.next().unwrap_or_default().to_string(),
                        it.next().unwrap_or_default().to_string(),
                    );
                    cx.defer(move |cx| crate::settings::open_machine_at(name, (what == "pick").then_some(path), cx));
                }
                "settings" => {
                    let sec = match arg {
                        "new" => crate::settings::Section::NewSessions,
                        "terminal" => crate::settings::Section::Terminal,
                        "machines" => crate::settings::Section::Machines,
                        "notifications" => crate::settings::Section::Notifications,
                        "chrome" => crate::settings::Section::Chrome,
                        "library" => crate::settings::Section::Library,
                        "budget" => crate::settings::Section::Budget,
                        "updates" => crate::settings::Section::Updates,
                        "privacy" => crate::settings::Section::Privacy,
                        _ => crate::settings::Section::Appearance,
                    };
                    cx.defer(move |cx| crate::settings::open_at(sec, cx));
                }
                // about: the About window.
                "about" => cx.defer(crate::about::open),
                // pick: the New sheet's tools on Pick.
                "pick" => {
                    if let Overlay::New(s) = &self.overlay {
                        s.update(cx, |s, cx| s.shot_pick(cx));
                    }
                }
                "stoparm" => {
                    if let Some(k) = self.sel.clone() {
                        self.arm_stop(format!("ins-{k}"), cx);
                    }
                }
                // v2: link (the picker for the selected chat), link=<query>; auto (the New sheet on Auto).
                "link" => {
                    if let Some(k) = self.sel.clone() {
                        self.open_link(k, window, cx);
                        if let (Overlay::Link(l), false) = (&self.overlay, arg.is_empty()) {
                            l.clone().update(cx, |l, cx| l.set_query(arg, window, cx));
                        }
                    }
                }
                "auto" => {
                    if let Overlay::New(s) = &self.overlay {
                        s.update(cx, |s, cx| s.pick_auto(cx));
                    }
                }
                "msg" => {
                    if let Overlay::New(s) = &self.overlay {
                        s.update(cx, |s, cx| s.set_message(arg, window, cx));
                    }
                }
                // chrome: Claude in Chrome on; chrome=set also opens the list of machines to prefer for it.
                "chrome" => {
                    if let Overlay::New(s) = &self.overlay {
                        s.update(cx, |s, cx| s.shot_chrome(arg == "set", cx));
                    }
                }
                _ => {}
            }
        }
        cx.notify();
    }
}

// ---------- rendering ----------

pub struct Ctx<'a> {
    pub t: Theme,
    pub app: &'a AppState,
    pub now: f64,
    pub cx: &'a Context<'a, BoardView>,
    /// Inner width of a board card, for dropping chips whole instead of clipping them.
    pub card_w: f32,
}

/// `shots` builds: once live data is in and the SB_SHOT state is open, lets the motion settle (SB_SHOT_DELAY ms,
/// default 1800), then either saves one frame to SB_SHOT_OUT and quits, or runs the SB_DRIVE script: steps separated
/// by ';' or newlines, each one of
///   wait <ms> | key <keystroke, e.g. cmd-k, escape, j> | type <text> | move <x> <y> | click <x> <y> [n]
///   | rclick <x> <y> | scroll <x> <y> <dy> | drag <x0> <y0> <x1> <y1> [mods] | drop <x> <y> <path>... | resize <w> <h>
///   | shot <file.png> | quit
/// Coordinates are window points (a 2x frame divided by 2). Input goes through Window::dispatch_event, the same path
/// real mouse and keyboard events take, and frames are the GPU renderer's own output (on Linux, the X screen).
#[cfg(feature = "shots")]
fn x_screen_shot(path: &std::path::Path) -> bool {
    std::env::var_os("DISPLAY").is_some()
        && std::process::Command::new("import").arg("-window").arg("root").arg(path).status().is_ok_and(|s| s.success())
}

#[cfg(feature = "shots")]
fn capture_when_ready(out: std::path::PathBuf, window: &mut Window, cx: &mut Context<BoardView>) {
    use std::time::{Duration, Instant};
    let settle = std::env::var("SB_SHOT_DELAY").ok().and_then(|v| v.parse().ok()).unwrap_or(1800u64);
    let script = std::env::var("SB_DRIVE").ok().filter(|s| !s.trim().is_empty());
    cx.spawn_in(window, async move |this, cx| {
        let start = Instant::now();
        loop {
            cx.background_executor().timer(Duration::from_millis(200)).await;
            let ready =
                this.update(cx, |v, cx| v.shot.is_none() && (v.find_shot || v.app.read(cx).live)).unwrap_or(true);
            if ready || start.elapsed() > Duration::from_secs(20) {
                break;
            }
        }
        cx.background_executor().timer(Duration::from_millis(settle)).await;
        let steps: Vec<String> = match &script {
            Some(s) => s.split([';', '\n']).map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect(),
            None => vec![format!("shot {}", out.display()), "quit".into()],
        };
        for step in steps {
            let mut it = step.splitn(2, ' ');
            let verb = it.next().unwrap_or("");
            let rest = it.next().unwrap_or("").trim().to_string();
            let nums: Vec<f32> = rest.split_whitespace().filter_map(|n| n.parse().ok()).collect();
            let at = |i: usize| {
                gpui::point(px(nums.get(i).copied().unwrap_or(0.)), px(nums.get(i + 1).copied().unwrap_or(0.)))
            };
            match verb {
                "wait" => {
                    cx.background_executor()
                        .timer(Duration::from_millis(nums.first().copied().unwrap_or(300.) as u64))
                        .await
                }
                "browse" => {
                    let _ = cx.update(|_, cx| crate::browser::open(cx));
                }
                "files" => {
                    let _ = cx.update(|_, cx| crate::files::open_files(cx));
                }
                "pair" => {
                    let _ = cx.update(|_, cx| crate::files::open_pair(cx));
                }
                "quit" => {
                    let _ = cx.update(|_, cx| cx.quit());
                    return;
                }
                "shot" => {
                    let path = std::path::PathBuf::from(&rest);
                    let _ = cx.update(|window, cx| {
                        // A Mac whose display sleeps draws no frames on its own, so the frame would lag one step
                        // behind: draw the latest state first.
                        if cfg!(target_os = "macos") {
                            window.draw(cx).clear(cx);
                        }
                        match window.render_to_image() {
                            Ok(img) => match img.save(&path) {
                                Ok(()) => {
                                    eprintln!("sb-shot saved {} ({}x{})", path.display(), img.width(), img.height())
                                }
                                Err(e) => eprintln!("sb-shot could not save {}: {e}", path.display()),
                            },
                            // Linux has no offscreen frame: grab the X screen instead (tests run alone on Xvfb).
                            Err(_) if cfg!(target_os = "linux") && x_screen_shot(&path) => {
                                eprintln!("sb-shot saved {} (X screen)", path.display())
                            }
                            Err(e) => eprintln!("sb-shot could not render: {e:#}"),
                        }
                    });
                }
                // winshot <settings|about> <path>: a frame of that window instead of the board's.
                "winshot" => {
                    let (which, path) = rest.split_once(' ').unwrap_or((rest.as_str(), ""));
                    let path = std::path::PathBuf::from(path);
                    let which = which.to_string();
                    let _ = cx.update(|_, cx| {
                        let app = crate::state::app(cx);
                        let h = if which == "about" { app.read(cx).about_win } else { app.read(cx).settings_win };
                        let Some(h) = h else {
                            eprintln!("sb-shot no {which} window");
                            return;
                        };
                        let r = h.update(cx, |_, window, cx| {
                            if cfg!(target_os = "macos") {
                                window.draw(cx).clear(cx);
                            }
                            window.render_to_image()
                        });
                        match r {
                            Ok(Ok(img)) => match img.save(&path) {
                                Ok(()) => {
                                    eprintln!("sb-shot saved {} ({}x{})", path.display(), img.width(), img.height())
                                }
                                Err(e) => eprintln!("sb-shot could not save {}: {e}", path.display()),
                            },
                            Ok(Err(e)) => eprintln!("sb-shot could not render: {e:#}"),
                            Err(e) => eprintln!("sb-shot no {which} window: {e:#}"),
                        }
                    });
                }
                "key" => {
                    let _ = cx.update(|window, cx| match gpui::Keystroke::parse(&rest) {
                        Ok(k) => {
                            window.dispatch_keystroke(k, cx);
                        }
                        Err(e) => eprintln!("sb-drive bad key {rest}: {e}"),
                    });
                }
                "type" => {
                    for ch in rest.chars() {
                        let k = if ch == ' ' { "space".to_string() } else { ch.to_string() };
                        let _ = cx.update(|window, cx| {
                            if let Ok(k) = gpui::Keystroke::parse(&k) {
                                window.dispatch_keystroke(k, cx);
                            }
                        });
                        cx.background_executor().timer(Duration::from_millis(30)).await;
                    }
                }
                "move" => {
                    let p = at(0);
                    let _ = cx.update(|window, cx| {
                        window.dispatch_event(
                            gpui::PlatformInput::MouseMove(gpui::MouseMoveEvent {
                                position: p,
                                pressed_button: None,
                                modifiers: Default::default(),
                            }),
                            cx,
                        );
                    });
                }
                "click" | "rclick" => {
                    let p = at(0);
                    let button = if verb == "rclick" { gpui::MouseButton::Right } else { gpui::MouseButton::Left };
                    let count = nums.get(2).copied().unwrap_or(1.) as usize;
                    let _ = cx.update(|window, cx| {
                        window.dispatch_event(
                            gpui::PlatformInput::MouseMove(gpui::MouseMoveEvent {
                                position: p,
                                pressed_button: None,
                                modifiers: Default::default(),
                            }),
                            cx,
                        );
                    });
                    cx.background_executor().timer(Duration::from_millis(40)).await;
                    for n in 1..=count {
                        let _ = cx.update(|window, cx| {
                            window.dispatch_event(
                                gpui::PlatformInput::MouseDown(gpui::MouseDownEvent {
                                    button,
                                    position: p,
                                    modifiers: Default::default(),
                                    click_count: n,
                                    first_mouse: false,
                                }),
                                cx,
                            );
                            window.dispatch_event(
                                gpui::PlatformInput::MouseUp(gpui::MouseUpEvent {
                                    button,
                                    position: p,
                                    modifiers: Default::default(),
                                    click_count: n,
                                }),
                                cx,
                            );
                        });
                    }
                }
                "scroll" => {
                    let p = at(0);
                    let dy = nums.get(2).copied().unwrap_or(0.);
                    let _ = cx.update(|window, cx| {
                        window.dispatch_event(
                            gpui::PlatformInput::ScrollWheel(gpui::ScrollWheelEvent {
                                position: p,
                                delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(dy))),
                                modifiers: Default::default(),
                                touch_phase: gpui::TouchPhase::Moved,
                            }),
                            cx,
                        );
                    });
                }
                "drag" => {
                    // A left-button drag in eight steps; a trailing word ("shift", "alt") holds that modifier.
                    let (p0, p1) = (at(0), at(2));
                    let mods = gpui::Modifiers {
                        shift: rest.contains("shift"),
                        alt: rest.contains("alt"),
                        ..Default::default()
                    };
                    let button = gpui::MouseButton::Left;
                    let _ = cx.update(|window, cx| {
                        window.dispatch_event(
                            gpui::PlatformInput::MouseMove(gpui::MouseMoveEvent {
                                position: p0,
                                pressed_button: None,
                                modifiers: mods,
                            }),
                            cx,
                        );
                        window.dispatch_event(
                            gpui::PlatformInput::MouseDown(gpui::MouseDownEvent {
                                button,
                                position: p0,
                                modifiers: mods,
                                click_count: 1,
                                first_mouse: false,
                            }),
                            cx,
                        );
                    });
                    for i in 1..=8 {
                        cx.background_executor().timer(Duration::from_millis(16)).await;
                        let p = p0 + (p1 - p0) * (i as f32 / 8.);
                        let _ = cx.update(|window, cx| {
                            window.dispatch_event(
                                gpui::PlatformInput::MouseMove(gpui::MouseMoveEvent {
                                    position: p,
                                    pressed_button: Some(button),
                                    modifiers: mods,
                                }),
                                cx,
                            );
                        });
                    }
                    let _ = cx.update(|window, cx| {
                        window.dispatch_event(
                            gpui::PlatformInput::MouseUp(gpui::MouseUpEvent {
                                button,
                                position: p1,
                                modifiers: mods,
                                click_count: 1,
                            }),
                            cx,
                        );
                    });
                }
                "drop" => {
                    // Files dragged in from the system and dropped at x y: the same events X11's XDND, Wayland's data
                    // device and Windows' IDropTarget deliver. The paths are the rest of the step, split on spaces.
                    let p = at(0);
                    let paths: Vec<std::path::PathBuf> =
                        rest.split_whitespace().skip(2).map(std::path::PathBuf::from).collect();
                    let _ = cx.update(|window, cx| {
                        use gpui::{FileDropEvent as F, PlatformInput as I};
                        window.dispatch_event(
                            I::FileDrop(F::Entered { position: p, paths: gpui::ExternalPaths(paths.into()) }),
                            cx,
                        );
                        window.dispatch_event(I::FileDrop(F::Pending { position: p }), cx);
                    });
                    cx.background_executor().timer(Duration::from_millis(300)).await;
                    let _ = cx.update(|window, cx| {
                        window.dispatch_event(
                            gpui::PlatformInput::FileDrop(gpui::FileDropEvent::Submit { position: p }),
                            cx,
                        );
                    });
                }
                "resize" => {
                    let s = gpui::size(
                        px(nums.first().copied().unwrap_or(1200.)),
                        px(nums.get(1).copied().unwrap_or(800.)),
                    );
                    let _ = cx.update(|window, _| window.resize(s));
                }
                other => eprintln!("sb-drive unknown step: {other}"),
            }
            eprintln!("sb-drive {step}");
            cx.background_executor().timer(Duration::from_millis(120)).await;
        }
    })
    .detach();
}

impl Render for BoardView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(shot) = self.shot.clone()
            && let Some((what, arg)) = shot.split_once('=').filter(|(w, _)| *w == "find" || *w == "setup")
        {
            self.shot = None;
            if what == "find" {
                self.shot_find(arg, cx);
            } else {
                self.shot_setup(arg, cx);
            }
        }
        if self.shot.is_some() {
            self.apply_shot(window, cx);
        }
        if !self.find_shot {
            self.search_if_first(cx);
        }
        if let Some(k) = self.pending_link.take() {
            self.open_link(k, window, cx);
        }
        if let Some(k) = self.pending_rename.take() {
            self.rename_chat(k, window, cx);
        }
        self.win_w = f32::from(window.viewport_size().width);
        let row_w = f32::from(self.tab_scroll.bounds().size.width).round();
        if row_w != self.tab_row_w {
            self.tab_row_w = row_w;
            window.request_animation_frame();
        }
        // Starred tabs sit at the left; a star changed anywhere (here, the iPhone) moves its tab.
        self.order_tabs(cx);
        if !self.tabs.is_empty() && !self.live_ticking {
            self.live_ticking = true;
            cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor().timer(std::time::Duration::from_secs(1)).await;
                    // Local only: no server call. Redraws the tab row and the terminal that shows.
                    let go = this
                        .update(cx, |v, cx| {
                            if v.tabs.is_empty() {
                                v.live_ticking = false;
                                return false;
                            }
                            if let Some(tab) = v.tab.checked_sub(1).and_then(|i| v.tabs.get(i)).cloned() {
                                tab.update(cx, |_, cx| cx.notify());
                                cx.notify();
                            }
                            true
                        })
                        .unwrap_or(false);
                    if !go {
                        break;
                    }
                }
            })
            .detach();
        }
        // v2: the shown chat's History loads in the background (a minute old or its count changed: again).
        {
            let app = self.app.read(cx);
            let arc = match &self.overlay {
                Overlay::Archive { key, .. } => key.clone(),
                _ => None,
            };
            let want: Vec<String> =
                self.sel.iter().chain(arc.iter()).filter(|k| app.needs_thread(k)).cloned().collect();
            if !want.is_empty() {
                let app = self.app.clone();
                cx.defer(move |cx| {
                    app.update(cx, |s, cx| {
                        for k in &want {
                            s.want_thread(k, cx);
                        }
                    })
                });
            }
        }
        self.ensure_history(window, cx);
        self.flags_inputs(window, cx);
        // The "Needs you" card's fields, for the details this window shows (the archive's, or the sheet's unless the
        // details are popped out: then detached.rs makes them for that window).
        let ask_key = match &self.overlay {
            Overlay::Archive { key, .. } => key.clone(),
            _ if self.detached.is_none() => self.sel.clone(),
            _ => None,
        };
        self.ask_inputs(ask_key, window, cx);
        // A tab drag that ended anywhere (dropped, or let go outside the window) leaves no gap marked.
        if (self.tab_drop.is_some() || self.drop_group.is_some() || self.group_drop.is_some()) && !cx.has_active_drag()
        {
            self.tab_drop = None;
            self.drop_group = None;
            self.group_drop = None;
        }
        // A group named here before the server answered: the chip being named follows it to the server's id.
        if let Some(e) = self.group_edit.clone() {
            let real = self.app.read(cx).group_id(&e);
            if real != e {
                self.group_edit = Some(real);
            }
        }
        if self.focus_float {
            self.focus_float = false;
            if let Some(f) = &self.float {
                let h = f.read(cx).focus.clone();
                window.focus(&h, cx);
            }
        }
        if self.focus_tab {
            self.focus_tab = false;
            // A group being named keeps the keyboard: taking it away would leave the field open with no caret.
            if self.overlay.is_none() && self.group_edit.is_none() && self.chat_edit.is_none() {
                self.focus_current(window, cx);
            }
        }
        let term_tab = self.tab.checked_sub(1).and_then(|i| self.tabs.get(i)).cloned();
        let vp = window.viewport_size();
        let (w, h) = (f32::from(vp.width), f32::from(vp.height));
        self.cap = if h >= 860. {
            4
        } else if h >= 700. {
            3
        } else {
            2
        };
        let decorations = match crate::frame::client(window) {
            Some(tiling) => Decorations::Client { tiling },
            None => window.window_decorations(),
        };
        let active = window.is_window_active();
        let maxed = (ui::win() || matches!(decorations, Decorations::Client { .. })) && window.is_maximized();
        let t = Theme::get(cx);
        let app_e = self.app.clone();
        let cx = &*cx;
        let app = app_e.read(cx);
        let now = AppState::now();
        let selected =
            self.sel.as_ref().filter(|k| app.snap.session(k).is_some() || app.snap.replaced_row(k).is_some()).cloned();
        let sheet_w = if w >= 1280. { 452. } else { 400. };
        // Details dock beside the board when the five columns still fit next to them (a wide window), at the width
        // their drag handle left; otherwise the sheet floats over the board's right side and the columns keep the
        // whole width.
        let ins_w = app.settings.inspector_w.unwrap_or(sheet_w).clamp(INS_MIN, INS_MAX);
        let shows_details = term_tab.is_none() && self.main == Main::Board && self.detached.is_none();
        let dock_w = w - ins_w - INS_HANDLE;
        let docked = shows_details
            && selected.is_some()
            && !crate::lanes::Geo::at(dock_w, self.solo.is_some(), 0.).scrolls(dock_w);
        let bw = if docked { dock_w } else { w };
        let g = crate::lanes::Geo::at(bw, self.solo.is_some(), 0.);
        // The board's geometry, worked out once: a Quiet row puts its time under the name when the column is narrow.
        let grid_w = (g.board_w(bw) - g.pad * 2.).max(g.grid_min);
        let card_w = ((grid_w - g.label - 50.) / 4.85 - 16.).max(80.);
        let c = Ctx { t, app, now, cx, card_w };

        let mut root = div()
            .id("root")
            .key_context(self.key_context())
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::on_next))
            .on_action(cx.listener(Self::on_prev))
            .on_action(cx.listener(Self::on_left))
            .on_action(cx.listener(Self::on_right))
            .on_action(cx.listener(Self::on_activate))
            .on_action(|_: &FocusNext, window, cx| window.focus_next(cx))
            .on_action(|_: &FocusPrev, window, cx| window.focus_prev(cx))
            .on_action(cx.listener(Self::on_fresh))
            .on_action(cx.listener(Self::on_resume))
            .on_action(cx.listener(Self::on_continue))
            .on_action(cx.listener(Self::on_new))
            .on_action(cx.listener(Self::on_palette))
            .on_action(cx.listener(Self::on_start_new))
            .on_action(cx.listener(Self::on_escape))
            .on_action(cx.listener(Self::on_close_window))
            .on_action(cx.listener(Self::on_close_tab))
            .on_action(cx.listener(Self::on_chat_palette))
            .on_action(cx.listener(Self::on_new_terminal))
            .on_action(cx.listener(|this, _: &Browse, _, cx| this.view_history(cx)))
            .on_action(cx.listener(|this, _: &OpenWindow, _, cx| {
                if let Some(k) = this.target() {
                    this.open_terminal_in(k, crate::place::Place::Window, cx)
                }
            }))
            .on_action(cx.listener(|this, _: &OpenFloat, _, cx| {
                if let Some(k) = this.target() {
                    this.open_terminal_in(k, crate::place::Place::Float, cx)
                }
            }))
            .on_action(cx.listener(|this, _: &ToggleTheme, window, cx| {
                let dark = cx.try_global::<Theme>().is_some_and(|t| t.dark);
                this.set_mode(if dark { Mode::Light } else { Mode::Dark }, window, cx)
            }))
            .on_drop::<crate::chrome::TabDrag>(cx.listener(|this, d: &crate::chrome::TabDrag, window, cx| {
                // A tab dragged down out of the tab row opens in a window of its own.
                if f32::from(window.mouse_position().y) > ui::header() + 6.
                    && let Some(i) = this.tab_index(d.0)
                {
                    this.move_tab(i, crate::place::Place::Window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &Tab1, _, cx| this.tab_key(1, cx)))
            .on_action(cx.listener(|this, _: &Tab2, _, cx| this.tab_key(2, cx)))
            .on_action(cx.listener(|this, _: &Tab3, _, cx| this.tab_key(3, cx)))
            .on_action(cx.listener(|this, _: &Tab4, _, cx| this.tab_key(4, cx)))
            .on_action(cx.listener(|this, _: &Tab5, _, cx| this.tab_key(5, cx)))
            .on_action(cx.listener(|this, _: &Tab6, _, cx| this.tab_key(6, cx)))
            .on_action(cx.listener(|this, _: &Tab7, _, cx| this.tab_key(7, cx)))
            .on_action(cx.listener(|this, _: &Tab8, _, cx| this.tab_key(8, cx)))
            .on_action(cx.listener(|this, _: &Tab9, _, cx| this.tab_key(9, cx)))
            .on_action(cx.listener(|this, _: &PalNext, _, cx| this.pal_move(1, cx)))
            .on_action(cx.listener(|this, _: &PalPrev, _, cx| this.pal_move(-1, cx)))
            // The docked details' drag handle: the panel follows the pointer, and the width is saved on release.
            .on_drag_move(cx.listener(|this, e: &gpui::DragMoveEvent<InsResize>, window, cx| {
                let w = f32::from(window.viewport_size().width);
                let x = f32::from(e.event.position.x);
                let nw = (w - x - INS_HANDLE / 2.).clamp(INS_MIN, INS_MAX);
                this.app.update(cx, |s, cx| {
                    if s.settings.inspector_w != Some(nw) {
                        s.settings.inspector_w = Some(nw);
                        cx.notify();
                    }
                });
            }))
            .on_drop::<InsResize>(cx.listener(|this, _: &InsResize, _, cx| {
                this.app.update(cx, |s, _| s.save_settings());
            }))
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .bg(t.bg)
            .text_color(t.ink)
            .font_family(UI)
            .text_size(px(13.))
            .child(self.titlebar(&c, decorations, active, maxed));
        root = root.children(self.flight_layer(&c, self.stopped_target(w, g)));
        let on_board = term_tab.is_none();
        root = match term_tab {
            Some(tab) => root.child(div().flex_1().min_h_0().flex().flex_col().child(tab)),
            None if self.main == Main::History => root.child(self.history_view(&c, w)).child(self.statusbar(&c, w)),
            None if docked => root
                .child(
                    div()
                        .flex_1()
                        .min_h_0()
                        .flex()
                        .child(div().flex_1().min_w_0().flex().flex_col().child(self.board_area(&c, g, bw)))
                        .child(self.ins_handle(&c))
                        .child(
                            div()
                                .id("detail-dock")
                                .flex_none()
                                .w(px(ins_w))
                                .h_full()
                                .min_h_0()
                                .bg(t.panel)
                                .child(self.inspector_panel(&c, selected.as_deref(), true)),
                        ),
                )
                .child(self.statusbar(&c, w)),
            None => root
                .child(div().flex_1().min_h_0().flex().flex_col().child(self.board_area(&c, g, w)))
                .child(self.statusbar(&c, w)),
        };
        // Narrow windows: details float in a sheet over the board, which stays usable underneath; popped out, they
        // live in their own window.
        if on_board
            && !docked
            && self.main == Main::Board
            && self.detached.is_none()
            && let Some(k) = &selected
        {
            root = root.child(
                div()
                    .id("detail-sheet")
                    .absolute()
                    .top(px(ui::header() + 56.))
                    .right(px(12.))
                    .bottom(px(ui::FOOTER + 12.))
                    .w(px(sheet_w))
                    .rounded(px(ui::R_SHEET))
                    .border_1()
                    .border_color(t.line2)
                    .bg(t.panel)
                    .shadow(t.drop())
                    .overflow_hidden()
                    .occlude()
                    .child(self.inspector_panel(&c, Some(k), false))
                    .with_animation(
                        SharedString::from(format!("sheet-in-{k}")),
                        gpui::Animation::new(ui::FADE).with_easing(gpui::ease_out_quint()),
                        |d, e| d.opacity(e).mr(px(-28. * (1. - e))),
                    ),
            );
        }
        if self.stopped_open {
            root = root.children(self.stopped_popover(&c));
        }
        if self.plan_open {
            // Under the Week chip and the gear, which move left when the header holds window buttons (Linux CSD 82,
            // Windows 138).
            let right = if ui::win() {
                154.
            } else if matches!(decorations, Decorations::Client { .. }) {
                98.
            } else {
                16.
            };
            let right = right + crate::chrome::app_menu_room();
            root = root.child(self.plan_popover(&c, right));
        }
        if self.menu.is_some() {
            root = root.child(self.ctx_menu(&c, w, h));
        }
        if let Some(i) = self.tab_menu.and_then(|(id, _)| self.tab_index(id)) {
            root = root.children(self.tab_menu_el(&c, i, w));
        }
        root = root.children(self.group_menu_el(&c, w));
        root = root.children(self.float_layer(&c, w, h));
        root = root.children(self.add_menu(&c, w));
        root = root.children(self.app_menu(&c, decorations));
        root = root.children(self.update_pop(&c));
        root = root.children(self.flags_layer(&c, w, h));
        match &self.overlay {
            Overlay::None => {}
            Overlay::Palette => root = root.child(self.palette(&c, w, h)),
            Overlay::New(sheet) => root = root.child(self.scrim(&c, sheet.clone().into_any_element(), 860.)),
            Overlay::Link(sheet) => root = root.child(self.scrim(&c, sheet.clone().into_any_element(), 580.)),
            Overlay::Archive { tab, key } => {
                let body = self.archive(&c, *tab, key.clone(), h);
                root = root.child(self.scrim(&c, body, 880.));
            }
        }
        if let Some((_, conf)) = &self.confirm {
            root = root.child(self.confirm_box(&c, conf));
        }
        root = root.children(self.rename_sheet(&c));
        if let Some(ask) = &c.app.busy_ask {
            root = root.child(self.busy_box(&c, ask));
        }
        if let Some(ask) = &c.app.trust_ask {
            root = root.child(self.trust_box(&c, ask));
        }
        if let Some(s) = &self.setup {
            root = root.child(self.setup_box(&c, s));
        }
        // Jobs and toasts stay above every layer, so an error from the New sheet is readable while it stays open.
        root = root.child(self.corner(&c));
        root = root.children(self.notices_popover(&c));
        // Linux, when the window draws its own frame: the edges it is resized by, over everything.
        if let Decorations::Client { tiling } = decorations {
            root = root.children(crate::frame::edges(tiling));
        }
        root
    }
}

/// The docked details panel: its narrowest and widest, and its drag handle's width.
const INS_MIN: f32 = 320.;
const INS_MAX: f32 = 640.;
const INS_HANDLE: f32 = 7.;

/// What the docked details' handle drags (the width follows the pointer; see the root's on_drag_move).
#[derive(Clone, Copy)]
pub struct InsResize;

impl BoardView {
    /// The line between the board and the docked details: drag it to make them wider or narrower.
    fn ins_handle(&self, c: &Ctx) -> impl IntoElement {
        let t = c.t;
        let (rule, accent) = (t.rule, t.accent);
        div()
            .id("ins-handle")
            .flex_none()
            .w(px(INS_HANDLE))
            .h_full()
            .flex()
            .justify_center()
            .cursor_col_resize()
            .group("ins-handle")
            .child(
                div().w(px(1.)).h_full().bg(rule).group_hover("ins-handle", move |s| s.w(px(3.)).bg(accent.alpha(0.6))),
            )
            .on_drag(InsResize, |_, _, _, cx| cx.new(|_| gpui::EmptyView))
    }

    // ---------- popover, scrim, confirm, toasts ----------

    /// A dimmed backdrop with a sheet hung under the title bar. A click on the backdrop closes it.
    fn scrim(&self, c: &Ctx, body: AnyElement, w: f32) -> impl IntoElement {
        let t = c.t;
        div()
            .absolute()
            .inset_0()
            .bg(t.scrim)
            .occlude()
            .on_mouse_down(MouseButton::Left, c.cx.listener(|this, _, window, cx| this.close_overlay(window, cx)))
            .child(
                div().absolute().top(px(ui::header())).left_0().right_0().flex().justify_center().child(
                    div()
                        .id("sheet")
                        .w(px(w))
                        .max_w(relative(0.96))
                        .rounded_b(px(ui::R_SHEET))
                        .border_1()
                        .border_t_0()
                        .border_color(t.line2)
                        .bg(t.panel2)
                        .shadow(t.drop())
                        .overflow_hidden()
                        .occlude()
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .child(body),
                ),
            )
    }

    /// Asked before a relaunch, stop, move or fresh start would cut off a chat's subagents, background commands or
    /// reply: what is running, then "when they finish" (the default, queued), "now" (with the cost said), or Cancel.
    fn busy_box(&self, c: &Ctx, ask: &crate::state::BusyAsk) -> impl IntoElement {
        let t = c.t;
        let cx = c.cx;
        let reply = ask.only_reply();
        let mut rows: Vec<String> = Vec::new();
        for b in &ask.busy {
            match &b.title {
                Some(title) => rows.push(format!("{}: {}", sb_core::names::split(title).0, b.summary)),
                None => rows.extend(b.subagents.iter().chain(b.pending.iter()).cloned()),
            }
        }
        rows.truncate(6);
        let answer = |when: Option<&'static str>| {
            cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.app.update(cx, |s, cx| s.answer_busy(when, cx));
            })
        };
        let choice = |id: &'static str, title: String, note: &'static str, primary: bool| {
            let b = if primary { ui::primary_btn(&t, id, 52.) } else { ui::line_btn(&t, id, 52.) };
            let ink = if primary { t.on_accent } else { t.ink };
            b.w_full().px(px(14.)).justify_start().child(
                div()
                    .flex()
                    .flex_col()
                    .items_start()
                    .gap(px(2.))
                    .child(div().text_size(px(13.5)).font_weight(FontWeight::SEMIBOLD).text_color(ink).child(title))
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(if primary { t.on_accent.alpha(0.85) } else { t.dim })
                            .child(note),
                    ),
            )
        };
        let finish = if reply { "when it finishes" } else { "when they finish" };
        div()
            .absolute()
            .inset_0()
            .bg(t.scrim)
            .occlude()
            .flex()
            .items_center()
            .justify_center()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| this.app.update(cx, |s, cx| s.answer_busy(None, cx))),
            )
            .child(
                div()
                    .id("busy-ask")
                    .w(px(460.))
                    .p(px(22.))
                    .rounded(px(ui::R_SHEET))
                    .border_1()
                    .border_color(t.line2)
                    .bg(t.panel2)
                    .shadow(t.drop())
                    .occlude()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    // one chat: just what is running (the buttons say the rest); several: the server's sentence
                    .child(
                        ui::wrap(match ask.busy.as_slice() {
                            [one] if one.title.is_none() && !one.summary.is_empty() => one.summary.clone(),
                            _ => ask.message.clone(),
                        })
                        .text_size(px(15.))
                        .line_height(relative(1.4))
                        .font_weight(FontWeight::BOLD)
                        .text_color(t.ink),
                    )
                    .children((!rows.is_empty()).then(|| {
                        div().flex().flex_col().gap(px(3.)).children(rows.into_iter().map(|n| {
                            ui::wrap(format!("\u{2022}  {n}"))
                                .text_size(px(12.5))
                                .line_height(relative(1.4))
                                .text_color(t.dim)
                        }))
                    }))
                    .child(
                        div()
                            .mt(px(10.))
                            .flex()
                            .flex_col()
                            .gap(px(8.))
                            .child(
                                choice(
                                    "busy-idle",
                                    format!("{} {finish}", ask.what),
                                    "Waits, then does it and tells you",
                                    true,
                                )
                                .on_click(answer(Some("idle"))),
                            )
                            .child(
                                choice(
                                    "busy-now",
                                    format!("{} now", ask.what),
                                    if reply {
                                        "Stops the reply where it is"
                                    } else {
                                        "Stops them. Their results may be lost."
                                    },
                                    false,
                                )
                                .on_click(answer(Some("now"))),
                            )
                            .child(
                                div()
                                    .flex()
                                    .justify_end()
                                    .child(ui::ghost(&t, "busy-cancel", Some("Cancel"), None).on_click(answer(None))),
                            ),
                    ),
            )
    }

    /// Asked when a move would take a chat into a folder Claude was never let into (need "trust").
    fn trust_box(&self, c: &Ctx, ask: &crate::state::TrustAsk) -> impl IntoElement {
        let t = c.t;
        let cx = c.cx;
        let answer = |yes: bool| {
            cx.listener(move |this, _: &ClickEvent, _, cx| this.app.update(cx, |s, cx| s.answer_trust(yes, cx)))
        };
        let words = crate::trust::words(&ask.shown, ask.declined, &ask.on, "move it");
        div()
            .absolute()
            .inset_0()
            .bg(t.scrim)
            .occlude()
            .flex()
            .items_center()
            .justify_center()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| this.app.update(cx, |s, cx| s.answer_trust(false, cx))),
            )
            .child(
                div()
                    .id("trust-ask")
                    .occlude()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(crate::trust::card(&t, "trust-mv", &ask.shown, words, answer(true), answer(false))),
            )
    }

    /// The confirm goes away unanswered, and the keyboard goes back where it was.
    fn drop_confirm(&mut self) {
        self.confirm = None;
        self.confirm_chrome = false;
        self.confirm_close = None;
        self.focus_tab = true;
    }

    /// The confirm's OK: Start fresh, the card's fix (restart with Chrome), or closing a busy local terminal.
    fn confirm_ok(&mut self, cx: &mut Context<Self>) {
        if let Some(tab) = self.confirm_close.take() {
            self.confirm = None;
            self.focus_tab = true;
            if let Some(i) = self.tabs.iter().position(|t| *t == tab) {
                self.force_close_tab(i, cx);
            }
        } else if let Some((key, _)) = self.confirm.take() {
            if std::mem::take(&mut self.confirm_chrome) {
                self.relaunch_chrome(key, cx);
            } else {
                self.app.update(cx, |s, cx| s.act(ActionId::Fresh, &key, cx));
            }
        }
        cx.notify();
    }

    fn confirm_box(&self, c: &Ctx, conf: &Confirm) -> impl IntoElement {
        let t = c.t;
        let cx = c.cx;
        div()
            .absolute()
            .inset_0()
            .bg(t.scrim)
            .occlude()
            .flex()
            .items_center()
            .justify_center()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.drop_confirm();
                    cx.notify();
                }),
            )
            .child(
                div()
                    .id("confirm")
                    .w(px(440.))
                    .max_w(relative(0.92))
                    .p(px(22.))
                    .rounded(px(ui::R_SHEET))
                    .border_1()
                    .border_color(t.line2)
                    .bg(t.panel2)
                    .shadow(t.drop())
                    .occlude()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .flex()
                    .flex_col()
                    .gap(px(10.))
                    .child(
                        div()
                            .min_w_0()
                            .text_size(px(16.))
                            .font_weight(FontWeight::BOLD)
                            .text_color(t.ink)
                            .child(conf.title.clone()),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .text_size(px(13.))
                            .line_height(relative(1.55))
                            .text_color(t.dim)
                            .child(conf.body.clone()),
                    )
                    .child(
                        div()
                            .mt(px(10.))
                            .flex()
                            .justify_end()
                            .gap(px(8.))
                            .child(
                                ui::line_btn(&t, "cf-no", 36.)
                                    .px(px(14.))
                                    .gap(px(8.))
                                    .child(conf.cancel.clone())
                                    .child(div().font_family(NUM).text_size(px(11.)).text_color(t.faint).child("Esc"))
                                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                        this.drop_confirm();
                                        cx.notify();
                                    })),
                            )
                            .child(
                                ui::primary_btn(&t, "cf-ok", 36.)
                                    .px(px(14.))
                                    .child(ui::icon(
                                        if self.confirm_close.is_some() {
                                            "x"
                                        } else if self.confirm_chrome {
                                            "relaunch"
                                        } else {
                                            "fresh"
                                        },
                                        15.,
                                        t.on_accent,
                                    ))
                                    .child(conf.ok.clone())
                                    .child(ui::kbd_in("↵", t.on_accent, t.on_accent.alpha(0.5)))
                                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.confirm_ok(cx))),
                            ),
                    ),
            )
    }

    /// Bottom right: running jobs, then toasts.
    fn corner(&self, c: &Ctx) -> impl IntoElement {
        let t = c.t;
        let cx = c.cx;
        let now = c.now;
        // Let Claude pick for Auto is off: an Auto card says its pick came from the rules, and where to turn it on.
        let rules_only = c.app.prefs.as_ref().and_then(|p| p.option("ai_auto_machine")).is_some_and(|o| !o.on);
        let jobs: Vec<AnyElement> = c
            .app
            .jobs
            .iter()
            .map(|j| {
                let done = j.state == sb_core::JobState::Done;
                let off = j.cancelled;
                let bad = j.state == sb_core::JobState::Error && !off;
                let ended = done || bad || off;
                let waiting = j.waiting.is_some() && !ended;
                let id = j.id.clone();
                let url = j.url.clone();
                let steps = j.kind.steps(&j.label).len().max(2) as f32 - 1.0;
                let frac = if done {
                    1.0
                } else {
                    ((j.at(now) as f32 + ((now - j.started) as f32 / 45.0).min(0.85)) / steps).clamp(0.06, 0.96)
                };
                let el = (now - j.started).max(0.0) as u64;
                let calling_off = c.app.busy.contains(&format!("job-off:{}", j.id));
                let kind_icon = match j.kind {
                    sb_core::JobKind::Stop => "stop",
                    sb_core::JobKind::Handoff => "handoff",
                    sb_core::JobKind::Relaunch => "relaunch",
                    sb_core::JobKind::Resume => "resume",
                    _ => "fresh",
                };
                let card = div()
                    .id(SharedString::from(format!("job-{}", j.id)))
                    .w(px(CORNER_W))
                    .relative()
                    .rounded(px(ui::R_CARD))
                    .border_1()
                    .border_color(if bad { t.bad.alpha(0.5) } else { t.line2 })
                    .bg(t.panel2)
                    .shadow(t.drop())
                    .overflow_hidden()
                    .when(ended, |d| d.cursor_pointer())
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        if let Some(u) = &url {
                            crate::state::open_link(cx, u);
                        }
                        let (id, park) = (id.clone(), url.is_none());
                        this.app.update(cx, |s, cx| s.drop_job(&id, park, cx));
                    }))
                    .child(
                        div()
                            .px(px(12.))
                            .py(px(10.))
                            .flex()
                            .items_center()
                            .gap(px(10.))
                            .child(if off {
                                ui::icon("info", 16., t.dim).into_any_element()
                            } else if done || bad {
                                ui::icon(if done { "check" } else { "warn" }, 16., if done { t.warm } else { t.bad })
                                    .into_any_element()
                            } else if waiting {
                                // Queued behind the chat's own work: it breathes rather than spins.
                                ui::pulse_icon(SharedString::from(format!("job-wait-{}", j.id)), kind_icon, 16., t.ink2)
                            } else {
                                // Still going: the ring keeps turning until the job finishes.
                                ui::spinner(SharedString::from(format!("job-spin-{}", j.id)), 16., t.run)
                            })
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .flex()
                                    .flex_col()
                                    .gap(px(2.))
                                    .child(
                                        div()
                                            .text_size(px(12.5))
                                            .font_weight(FontWeight::BOLD)
                                            .text_color(if bad { t.bad } else { t.ink })
                                            .child(j.headline()),
                                    )
                                    .child(
                                        ui::wrap(j.detail(now))
                                            .text_size(px(12.))
                                            .line_height(relative(1.4))
                                            .text_color(t.dim),
                                    ),
                            )
                            .child(
                                if let (true, sb_core::JobKind::Stop, Some((m, s))) =
                                    (done, j.kind, j.key.as_deref().and_then(|k| k.split_once(':')))
                                {
                                    // Stopped: one click brings it back.
                                    let jid = j.id.clone();
                                    let (m, s, from) = (m.to_string(), s.to_string(), j.from.clone());
                                    ui::line_btn(&t, SharedString::from(format!("job-resume-{}", j.id)), 28.)
                                        .px(px(10.))
                                        .text_size(px(12.5))
                                        .child(ui::icon("resume", 13., t.dim))
                                        .child("Resume")
                                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                            cx.stop_propagation();
                                            let (id, m, s, f) = (jid.clone(), m.clone(), s.clone(), from.clone());
                                            this.app.update(cx, |st, cx| {
                                                st.drop_job(&id, false, cx);
                                                st.resume_chat(&m, &s, f, cx);
                                            });
                                        }))
                                        .into_any_element()
                                } else if let (true, Some(nk)) = (
                                    done,
                                    j.new_machine
                                        .as_ref()
                                        .zip(j.new_sid.as_ref())
                                        .map(|(m, s)| format!("{m}:{s}"))
                                        .filter(|_| j.kind != sb_core::JobKind::Handoff),
                                ) {
                                    // A fresh start, a resume or a new session is ready: its terminal, one click away.
                                    let jid = j.id.clone();
                                    ui::line_btn(&t, SharedString::from(format!("job-term-{}", j.id)), 28.)
                                        .px(px(10.))
                                        .text_size(px(12.5))
                                        .child(ui::icon("term", 13., t.dim))
                                        .child("Open terminal")
                                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                            cx.stop_propagation();
                                            let id = jid.clone();
                                            this.app.update(cx, |s, cx| s.drop_job(&id, false, cx));
                                            this.open_terminal(nk.clone(), false, cx);
                                        }))
                                        .into_any_element()
                                } else if done && j.url.is_some() {
                                    div().text_size(px(12.)).text_color(t.accent).child("Open").into_any_element()
                                } else if ended {
                                    div().into_any_element()
                                } else {
                                    // How long it has been at it (or, queued, how long it has waited).
                                    ui::num(fmt::clock(el), 12., t.faint).into_any_element()
                                },
                            )
                            .when(ended, |d| {
                                // A finished card says plainly how it goes away (a click anywhere on it does too).
                                let jid = j.id.clone();
                                d.child(
                                    ui::icon_btn(&t, SharedString::from(format!("job-x-{}", j.id)), "x", 24.).on_click(
                                        cx.listener(move |this, _: &ClickEvent, _, cx| {
                                            cx.stop_propagation();
                                            let id = jid.clone();
                                            this.app.update(cx, |s, cx| s.drop_job(&id, true, cx));
                                        }),
                                    ),
                                )
                            }),
                    )
                    .when_some(j.pick(), |d, (who, what)| {
                        let (label, why) = match what.split_once(". ") {
                            Some((l, w)) => (l.to_string(), Some(w.to_string())),
                            None => (what.trim_end_matches('.').to_string(), None),
                        };
                        let waiting = j.label == sb_core::jobs::AUTO;
                        let what = if waiting && rules_only {
                            "Auto is checking what each machine can do.".to_string()
                        } else {
                            what
                        };
                        let by_rules = rules_only && !waiting && j.by.as_deref() != Some("claude");
                        d.child(
                            div()
                                .mx(px(12.))
                                .mb(px(10.))
                                .ml(px(38.))
                                .px(px(10.))
                                .py(px(7.))
                                .flex()
                                .flex_col()
                                .gap(px(3.))
                                .rounded(px(ui::R_CTRL))
                                .bg(t.accent.alpha(if t.dark { 0.10 } else { 0.06 }))
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap(px(5.))
                                        .child(ui::icon("auto", 11., t.accent))
                                        .child(ui::lbl(&who, t.accent)),
                                )
                                .child(if waiting {
                                    div().text_size(px(12.)).text_color(t.ink2).child(what)
                                } else {
                                    div().text_size(px(12.)).line_height(relative(1.4)).text_color(t.ink2).child(
                                        ui::runs_text(
                                            &[
                                                sb_core::TextRun { text: label, bold: true },
                                                sb_core::TextRun {
                                                    text: why.map(|w| format!(". {w}")).unwrap_or_default(),
                                                    bold: false,
                                                },
                                            ],
                                            t.ink,
                                        ),
                                    )
                                })
                                .when(by_rules, |d| {
                                    d.child(
                                        div()
                                            .flex()
                                            .flex_col()
                                            .items_start()
                                            .gap(px(2.))
                                            .text_size(px(11.5))
                                            .line_height(relative(1.4))
                                            .text_color(t.dim)
                                            .child(ui::wrap(crate::sheet::rules_note()))
                                            .child(
                                                ui::link(
                                                    &t,
                                                    SharedString::from(format!("job-ai-{}", j.id)),
                                                    "Open Settings",
                                                )
                                                .on_click(
                                                    |_, _, cx| {
                                                        cx.stop_propagation();
                                                        crate::settings::open_at(crate::settings::Section::Privacy, cx)
                                                    },
                                                ),
                                            ),
                                    )
                                }),
                        )
                    })
                    .when(waiting, |d| {
                        // Nothing has changed yet: it can still be done at once (cutting the work off) or called off.
                        let now_label = match j.kind {
                            sb_core::JobKind::Stop => Some("Stop now"),
                            sb_core::JobKind::Fresh => Some("Start now"),
                            _ => None,
                        };
                        let off_label = if j.kind == sb_core::JobKind::Stop { "Keep running" } else { "Cancel" };
                        let (a, b) = (j.id.clone(), j.id.clone());
                        d.child(
                            div()
                                .ml(px(38.))
                                .mr(px(12.))
                                .mb(px(10.))
                                .flex()
                                .items_center()
                                .gap(px(8.))
                                .when(calling_off, |d| d.opacity(0.55))
                                .when_some(now_label, |d, l| {
                                    d.child(
                                        ui::line_btn(&t, SharedString::from(format!("job-now-{}", j.id)), 28.)
                                            .px(px(11.))
                                            .text_size(px(12.5))
                                            .when(j.kind == sb_core::JobKind::Stop, |d| d.text_color(t.bad))
                                            .child(l)
                                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                                cx.stop_propagation();
                                                if !calling_off {
                                                    let id = a.clone();
                                                    this.app.update(cx, |s, cx| s.job_now(&id, cx));
                                                }
                                            })),
                                    )
                                })
                                .child(
                                    ui::ghost(&t, SharedString::from(format!("job-cancel-{}", j.id)), None, None)
                                        .h(px(28.))
                                        .px(px(10.))
                                        .text_size(px(12.5))
                                        .child(off_label)
                                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                            cx.stop_propagation();
                                            if !calling_off {
                                                let id = b.clone();
                                                this.app.update(cx, |s, cx| s.cancel_job(&id, cx));
                                            }
                                        })),
                                ),
                        )
                    })
                    .when(!ended && !waiting, |d| {
                        d.child(div().absolute().left_0().bottom_0().h(px(2.)).w(relative(frac)).bg(t.run))
                    });
                ui::rise(card, SharedString::from(format!("job-in-{}", j.id)))
            })
            .collect();
        // Setup problems a machine's agent found: each shows once, until its X (or Fix) closes it.
        let multi = c.app.snap.machines.len() > 1;
        let setup: Vec<AnyElement> = c
            .app
            .setup_open()
            .into_iter()
            .map(|(m, label, i)| {
                let bad = i.tone.as_deref() == Some("bad");
                let busy = c.app.busy.contains(&format!("setup:{m}:{}", i.id));
                let (mx, idx) = (m.clone(), i.id.clone());
                let title = if multi { format!("{label}: {}", i.title) } else { i.title.clone() };
                let card = div()
                    .id(SharedString::from(format!("setup-{m}-{}", i.id)))
                    .relative()
                    .w(px(CORNER_W))
                    .px(px(12.))
                    .py(px(10.))
                    .rounded(px(ui::R_CARD))
                    .border_1()
                    .border_color(if bad { t.bad.alpha(0.5) } else { t.need.alpha(0.45) })
                    .bg(t.panel2)
                    .shadow(t.drop())
                    .flex()
                    .items_start()
                    .gap(px(10.))
                    .child(ui::icon("warn", 16., if bad { t.bad } else { t.need }).mt(px(1.)))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(4.))
                            .child(ui::wrap(title).text_size(px(12.5)).font_weight(FontWeight::BOLD).text_color(t.ink))
                            .child(
                                ui::wrap(i.text.clone())
                                    .text_size(px(12.))
                                    .line_height(relative(1.45))
                                    .text_color(t.dim),
                            )
                            .when_some(i.fix.clone(), |d, fix| {
                                let (m2, id2) = (m.clone(), i.id.clone());
                                d.child(
                                    div().mt(px(4.)).flex().child(
                                        ui::line_btn(&t, SharedString::from(format!("setup-fix-{m}-{}", i.id)), 28.)
                                            .px(px(12.))
                                            .text_size(px(12.5))
                                            .child(if busy { "Fixing" } else { "Fix" })
                                            .when(busy, |d| d.opacity(0.6))
                                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                                if !busy {
                                                    let (m, id, f) = (m2.clone(), id2.clone(), fix.clone());
                                                    this.app.update(cx, |s, cx| s.fix_setup(&m, &id, &f, cx));
                                                }
                                            })),
                                    ),
                                )
                            }),
                    )
                    .child(ui::icon_btn(&t, SharedString::from(format!("setup-x-{m}-{}", i.id)), "x", 22.).on_click(
                        cx.listener(move |this, _: &ClickEvent, _, cx| {
                            let (m, id) = (mx.clone(), idx.clone());
                            this.app.update(cx, |s, cx| s.park_setup(&m, &id, cx));
                        }),
                    ));
                ui::rise(card, SharedString::from(format!("setup-in-{m}-{}", i.id)))
            })
            .collect();
        let toasts: Vec<AnyElement> = c
            .app
            .toasts
            .iter()
            .map(|to| {
                let id = to.id;
                let (icon, ic) = if to.bad {
                    ("warn", t.bad)
                } else if to.ok {
                    ("check", t.warm)
                } else {
                    ("info", t.dim)
                };
                let card = div()
                    .id(SharedString::from(format!("toast-{}", to.id)))
                    .relative()
                    .w(px(CORNER_W))
                    .pl(px(12.))
                    .pr(px(6.))
                    .py(px(10.))
                    .rounded(px(ui::R_CARD))
                    .border_1()
                    .border_color(if to.bad { t.bad.alpha(0.5) } else { t.line2 })
                    .bg(t.panel2)
                    .shadow(t.drop())
                    .cursor_pointer()
                    // It stays while the pointer is on it.
                    .on_hover(
                        cx.listener(move |this, on: &bool, _, cx| this.app.update(cx, |s, _| s.hover_toast(id, *on))),
                    )
                    .on_click(
                        cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.app.update(cx, |s, cx| s.drop_toast(id, cx))
                        }),
                    )
                    .flex()
                    .items_start()
                    .gap(px(10.))
                    .child(ui::icon(icon, 16., ic).mt(px(1.)))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(3.))
                            .child(
                                ui::wrap(to.title.clone())
                                    .text_size(px(12.5))
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(if to.bad { t.bad } else { t.ink }),
                            )
                            .when_some(to.sub.clone(), |d, s| {
                                d.child(ui::wrap(s).text_size(px(12.)).line_height(relative(1.4)).text_color(t.dim))
                            }),
                    )
                    .when_some(to.act.as_ref().map(|a| a.label()), |d, label| {
                        d.child(
                            ui::line_btn(&t, SharedString::from(format!("toast-act-{}", to.id)), 26.)
                                .px(px(10.))
                                .text_size(px(12.5))
                                .child(label)
                                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                    cx.stop_propagation();
                                    this.app.update(cx, |s, cx| s.toast_act(id, cx));
                                })),
                        )
                    })
                    .child(ui::icon_btn(&t, SharedString::from(format!("toast-x-{}", to.id)), "x", 22.).on_click(
                        cx.listener(move |this, _: &ClickEvent, _, cx| {
                            cx.stop_propagation();
                            this.app.update(cx, |s, cx| s.drop_toast(id, cx));
                        }),
                    ));
                ui::rise(card, SharedString::from(format!("toast-in-{}", to.id)))
            })
            .collect();
        div()
            .absolute()
            .right(px(12.))
            .bottom(px(34.))
            .flex()
            .flex_col()
            .items_end()
            .gap(px(8.))
            .children(setup)
            .children(jobs)
            .children(toasts)
    }
}

/// The width of everything in the bottom-right corner: setup notices, job cards and toasts line up.
const CORNER_W: f32 = 360.;

/// The board grid: an optional label column, then four columns and a slightly narrower Quiet one.
pub fn grid_row(label: Option<AnyElement>, cells: Vec<AnyElement>) -> gpui::Div {
    let mut row = div().flex().flex_row().items_start().gap(px(10.));
    if let Some(l) = label {
        row = row.child(l);
    }
    for (i, cell) in cells.into_iter().enumerate() {
        row = row.child(div().flex_basis(px(0.)).flex_grow(if i == 4 { 0.82 } else { 1.0 }).min_w_0().child(cell));
    }
    row
}

/// The tabs to save: those that know their chat, in order, then the saved ones still waiting for the server (a launch
/// without live data keeps them rather than forgetting them).
fn saved_keys(open: Vec<Option<String>>, waiting: &[String]) -> Vec<String> {
    let mut keys: Vec<String> = open.into_iter().flatten().collect();
    for k in waiting {
        if !keys.contains(k) {
            keys.push(k.clone());
        }
    }
    keys
}

/// Changes that come in bursts (a window being dragged): written once they stop, or on the way out.
#[derive(Default)]
struct Debounce {
    changed: u64,
    saved: u64,
}

impl Debounce {
    /// A change came in; its number, for the timer that waits after it.
    fn bump(&mut self) -> u64 {
        self.changed += 1;
        self.changed
    }

    /// The wait after change `n` ran out: true (write it) when no other change came since.
    fn settled(&mut self, n: u64) -> bool {
        if n != self.changed || self.saved == n {
            return false;
        }
        self.saved = n;
        true
    }

    /// Leaving: true when a change is not written yet.
    fn flush(&mut self) -> bool {
        self.settled(self.changed)
    }
}

#[cfg(test)]
mod tests {
    use super::{Debounce, saved_keys};

    #[test]
    fn a_drag_is_written_once_and_a_quick_quit_still_writes_it() {
        let mut d = Debounce::default();
        let ns: Vec<u64> = (0..100).map(|_| d.bump()).collect();
        assert_eq!(ns.iter().filter(|n| d.settled(**n)).count(), 1);
        assert!(!d.flush(), "written already");
        d.bump();
        assert!(d.flush(), "quit before the wait ran out");
        assert!(!d.flush());
        assert!(!Debounce::default().flush(), "never moved: nothing to write");
    }

    #[test]
    fn tabs_waiting_for_the_server_stay_saved() {
        let open = vec![Some("studio:a".to_string()), None, Some("studio:b".to_string())];
        assert_eq!(saved_keys(open.clone(), &[]), vec!["studio:a", "studio:b"]);
        let waiting = vec!["studio:c".to_string(), "studio:a".to_string()];
        assert_eq!(saved_keys(open, &waiting), vec!["studio:a", "studio:b", "studio:c"], "once, after the open ones");
        assert_eq!(saved_keys(Vec::new(), &waiting), vec!["studio:c", "studio:a"], "none opened: the list stays");
    }
}
