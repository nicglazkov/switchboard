//! About Switchboard (Switchboard > About Switchboard): a small window of its own, as macOS apps do. The icon, name
//! and version, the build and what it runs on, the server it talks to, Copy version info, and What's new with an
//! entry for every version. For bug reports: Copy diagnostics (version, build, OS and the last 200 log lines) and
//! Show log (the log folder, src/logs.rs). Open source licenses opens the notices for everything the app is built
//! from (native/packaging/THIRD-PARTY-NOTICES.txt, made by native/scripts/notices.sh).

use std::path::{Path, PathBuf};
use std::time::Duration;

use gpui::{
    App, AppContext, ClickEvent, Context, Entity, FontWeight, InteractiveElement, IntoElement, ParentElement, Render,
    StatefulInteractiveElement, Styled, Window, div, prelude::*, px, relative,
};

use crate::state::AppState;
use crate::theme::{NUM, Theme};
use crate::ui;

/// What each version changed, newest first. Add an entry with every version bump.
pub const RELEASES: &[(&str, &[&str])] = &[
    (
        "0.8.10",
        &[
            "Copy and paste in a terminal work the same everywhere: selecting text copies it, Ctrl+C copies when text is selected, and Ctrl+V pastes on Windows and Linux.",
            "A right-click in a terminal copies the selection, or pastes when nothing is selected.",
            "Terminals on a Windows PC start without waiting for the app to answer the Windows console's first question.",
        ],
    ),
    (
        "0.8.9",
        &[
            "Updates come from Switchboard's GitHub releases, through your server. The app still checks every build's signature and SHA-256 before it installs it.",
            "A check for updates that fails says why, instead of saying this is the newest version. The update panel has What's new.",
            "The setup guides open the new documentation site.",
            "Ctrl+Shift+Q quits on Windows and Linux, where a terminal keeps Ctrl+Q for itself.",
            "Settings, Notifications says On this computer on Windows and Linux.",
            "switchboard logs shows the server's logs on a Mac, and removing the helper from a Windows PC also removes its logon task.",
        ],
    ),
    (
        "0.8.8",
        &[
            "Adding a Windows PC works from an ordinary PowerShell window: the node starts at logon without administrator rights.",
            "Adding a Windows PC again replaces a node that is still running instead of stopping at Access is denied.",
            "Add a machine downloads the node and agent of this release, not an old one that could not link.",
        ],
    ),
    (
        "0.8.7",
        &[
            "Windows finds the server on the network again: on a PC with several network adapters it asked the wrong one.",
        ],
    ),
    (
        "0.8.6",
        &[
            "The installer's \"Eject the disk image\" works on the signed download: it used to stop before ejecting.",
            "The disk image's window has room for Finder's tab bar, so Install Switchboard no longer drops out of sight.",
            "The Switchboard name shows in its own typeface again; the bundled font was missing most letters.",
            "The kit keeps the terminal schemes, and its port check ignores connections that are closing.",
        ],
    ),
    (
        "0.8.5",
        &[
            "New chat starts on Auto, and its model list is the models your machines' Claude Code takes, from the server.",
            "Plainer wording across the app, the setup screens and the messages from the server and the helper. A folder or the local network that macOS hasn't allowed says \"macOS permission not granted\".",
        ],
    ),
    (
        "0.8.4",
        &[
            "A chat open in a tab is never offered for stopping, even while Switchboard is closed or the Mac is asleep. The app tells the server its tabs; closing the tab is what makes a chat eligible again.",
        ],
    ),
    (
        "0.8.3",
        &[
            "Typing exit (or /exit) in a chat closes its tab. Before, Claude Code's list of background agents opened in the tab instead.",
            "Stop is honest: a chat the server did not stop (in use again, kept, or Claude Code could not confirm it) says so and stays on the card, instead of a \"Stopped\" that left it running.",
            "A background chat whose reply has finished shows as quiet, not Working, even while a server or command it started keeps running. Its subagents or commands still writing keep it Working.",
        ],
    ),
    (
        "0.8.2",
        &[
            "Rename a chat: double-click a terminal tab's title, or pick Rename chat in its right-click menu, and type over the name right in the tab. Enter saves, Escape puts it back.",
            "Rename from the board too: the pencil in the details (or a double-click on the name), a card's right-click menu, or Rename chat in the palette.",
            "The name shows everywhere at once: the tab, the board card, the details, the palette, History and the iPhone. It follows the chat through /clear and Start fresh (\"Your name (part 2)\") and onto copies, and Claude Code's own /resume list shows it too.",
            "Clear the name and press Enter to go back to the automatic one.",
            "Many tabs: past a few letters each, tabs shrink to just their dot (the one that shows keeps its title) before the row has to scroll, so a group's chip no longer slides half out of sight, even while you name it.",
        ],
    ),
    (
        "0.8.1",
        &[
            "Naming a tab group: the name field is a light box outlined in the group's color with its dot in front, so the text and the caret show. Before, the field sat on the solid color and its text could not be seen.",
            "The name field keeps the keyboard until Enter, Escape or a click elsewhere; the terminal no longer takes it away and leaves the field stuck open.",
        ],
    ),
    (
        "0.8.0",
        &[
            "One answer for every view: the server decides each chat's state once, and the board, the terminal tab and strip, the inspector and the iPhone all show that same answer.",
            "Cold now simply means the cache has run out. \"Expensive\" (150k or more) is its own badge and can show with any state. When the real cache time is not known the state says so instead of guessing.",
            "Old information looks old: if a machine has not reported for 45 seconds, its states turn grey with \"as of 2m ago\". Stalled chats say Stalled, not Working.",
            "Terminals: every window and the iPhone share one session per chat, so they no longer fight over its size. The screen only changes size when the chat has redrawn for it, which ends the stray letters and doubled rows.",
            "A dropped terminal link reconnects by itself and keeps what you could see and scroll back to. Redraw (Cmd+Shift+R) repaints a terminal on demand, and a big paste waits instead of closing the terminal.",
        ],
    ),
    (
        "0.7.9",
        &[
            "Notices tray: whatever leaves the bottom-right corner (its X, a click, or its time running out) waits behind \"Notices\" at the right of the status bar, with a count of what is new. Open it to see each one again, with its button (Resume, Open terminal, Fix).",
            "Nothing in the tray goes until you say so: Ignore asks \"Confirm ignore\" first, Clear all asks \"Confirm clear all\". A setup notice's X sends it to the tray too; ignoring it there is what keeps it away.",
            "The tray survives a restart.",
        ],
    ),
    (
        "0.7.8",
        &[
            "Stop on a working chat (or closing its tab) gets its own card: \"Stops when its work is done\", with Stop now and Keep running. The chat's card says \"Stops after this work\" meanwhile, and once it stops the card offers Resume.",
            "Start fresh waiting on a busy chat: the card breathes instead of spinning, counts how long it has waited, and has Start now beside Cancel. Calling it off says \"Nothing was changed\" in grey and goes away by itself, instead of a red \"did not finish\".",
            "Confirm stop counts 3, 2, 1 on the second, and Esc or a click anywhere else puts the button back.",
            "Stopping from the Not in use list: chats still working get their own cards instead of one vague toast; a stopped chat's toast has Resume.",
            "Toasts: a check, warning or info mark, the same width as the cards above them, a short rise into view, and they stay while the pointer is on them. How long one stays depends on how much it says; the same message twice refreshes the one on screen.",
            "A job that finishes while Switchboard is in the background posts a notification.",
        ],
    ),
    (
        "0.7.7",
        &[
            "History: the Resume, Start fresh and Open terminal buttons grow taller to fit their notes, so a long note no longer runs into the next button.",
            "Finished and failed job cards, and notices, have an X in the corner to close them. A failed card shows a warning sign instead of an X, so the only X is the one that closes it.",
            "Start fresh in a folder Claude Code had not trusted yet (a worktree, say) trusts it and goes again by itself, instead of stopping with \"Workspace not trusted\".",
            "Setup check: each machine's agent looks for setup problems that would bite a chat later (MCP servers background chats cannot start, background chats coming up without Claude in Chrome) and the app says so once, with Fix when it can repair it and an X to close it. The Claude folder is trusted quietly.",
            "Start fresh spins its spiral while the new chat comes up, on the History button and on the job card.",
            "A chat launched with Claude in Chrome that came up without it (Claude Code 2.1.300 background chats) now says so in its details, and its Chrome chip goes off instead of claiming Chrome.",
            "A starred tab shows one star, not two under the pointer. Click that star to unstar the tab.",
            "Chats that stopped while working or waiting no longer show as Needs you, and a terminal chat whose turn is over no longer shows as Working while its background commands run.",
        ],
    ),
    (
        "0.7.6",
        &[
            "Start fresh (or a move) on a chat that is still working now says so: \"Claude is replying. It starts once that is done.\" instead of \"Writing the handoff\" for minutes on end, with a Cancel button while it waits.",
            "A window that comes back on screen (uncovered, unminimized, a display waking) gets its frames going again, and after 20 seconds or more away its terminals redraw their screens, as dragging the window's corner did. No more frozen terminals or leftover letters in the status line.",
        ],
    ),
    (
        "0.7.5",
        &[
            "Closing a chat's tab (X, Cmd+W, typing exit, closing its group) stops the chat, so nothing sits running in the background. A chat that is working stops once its reply is done. Resume brings any of them back. Chats set to Keep running, and chats open in a terminal on their own machine, are left alone.",
        ],
    ),
    (
        "0.7.4",
        &[
            "Settings > Library > Set key... on this Mac is a hidden field with Save: the value goes straight into this Mac's keychain. Other machines still show the command to run there.",
            "Tabs from the last launch reopen once live data is in, not from the saved snapshot, so a chat started since the last save no longer loses its tab.",
            "The command palette lists chats parked for a day or more below the working ones.",
            "A terminal tab closes only when you leave (exit, Ctrl-D, Ctrl-Z). If the attach ends on its own, the tab stays with Reconnect instead of vanishing from your saved tabs.",
        ],
    ),
    (
        "0.7.3",
        &[
            "The command palette (Cmd+K, Cmd+T) keeps every row on one line: a long status like \"Needs: answer 3 questions...\" ends in an ellipsis instead of squeezing the chat's name into a column of letters and spilling over the rows below.",
            "A machine's name on the board stays on one line in a narrow window (\"MacB...\" instead of \"MacBoo\" over \"k\").",
        ],
    ),
    (
        "0.7.2",
        &[
            "The Working / Warm / Cold strip and the tab's live dot now show for a chat started or restarted from Switchboard and after a /clear: the tab finds its chat by its background id, not only by the chat id it was opened with.",
        ],
    ),
    (
        "0.7.1",
        &[
            "Stars stick to a chat through /clear, Start fresh, Continue and Resume: every new part of a starred chat is starred, and unstarring any part unstars them all.",
            "A starred tab saved before a /clear or a fresh start reopens on the newest part of that chat when Switchboard launches.",
        ],
    ),
    (
        "0.7.0",
        &[
            "Tab groups, as in Chrome: right-click a tab for Add to new group or Add to group, and the group gets a colored chip and a line under its tabs. Click the chip to fold the group into a count; right-click it to rename, pick a color, ungroup, close its tabs, or move it.",
            "Drag a tab into a group to join it, out of it to leave, or drag a chip to move the whole group. Starred tabs stay at the left, outside groups.",
            "Groups are kept on the server, so every Mac shows the same ones (folding is per Mac). Board cards and the command palette show a chat's group color.",
            "Usage panel: days read as a share of your weekly limit (\"Today 4%, 12% of the week so far\"), the plan note is gone, and \"Costly to go back to\" lists the 3 cold chats that would re-read the most. Click one to open it.",
            "Typing exit (or /exit, quit, /quit) and Enter in a terminal tab now leaves: the tab closes and the chat keeps running in the background.",
            "Many tabs stay readable: \"Updated\" in the header makes way first, then machine labels, then titles shrink to a few letters; past that the tab row scrolls sideways and keeps the tab you are on in sight.",
            "On a narrow window the board scrolls sideways instead of squeezing cards until names break into letters, and card buttons always show their whole label.",
            "The strip on top of an open terminal is bigger and reads as one line: \"Working, for 2m 10s\", \"Warm, cache good for 4:12\", \"Cold, next message re-reads 99k\" or what Claude needs, with the context as a small meter and \"98k of 300k context\" on the right.",
            "Auto tools: a new chat starts with the library tools in Auto, or you pick them by hand (Tools: Auto | Pick). In Settings > Library each tool has Include in Auto and a one-line When to use; a chat's details say \"Tools: Auto (5)\" or \"Tools: 3 picked\".",
            "Usage budget: Settings > Budget sets a daily share of the week (14.3% by default), an optional 5-hour cap and an optional hard limit that blocks new messages and stops working background chats, with Lift for today. The usage panel says \"Today 7% of 14% budget\", and today's bar and the Week chip turn amber over budget and red at the hard limit.",
            "The Switchboard logo at the top left opens Settings, and its mark and name line up with Board and History.",
            "About shows the app's own icon above its name and version, like a Mac About panel.",
            "A waiting chat's card tags a plain question \"Open question\" (it said \"Reply\" next to the Answer... button).",
        ],
    ),
    (
        "0.6.0",
        &[
            "A waiting chat's details open on a \"Needs you\" card shaped by what Claude asked: option buttons with its assumptions to tick (Choose), a short form with one line per question (Questions), a checklist and Done (Do this, then confirm), or the fix as one button (Fix setup). Send turns it into one message typed into the chat.",
            "Questions about you are never pre-answered, and Send waits until every question has an answer.",
            "Board cards for waiting chats show what Claude needs on two lines, the kind of ask, and one button: the recommended option, Done, the fix, or Answer..., which opens the card.",
            "\"Restart this chat with Chrome\" relaunches it with --chrome after saying what it costs.",
            "Terminal tabs look like tabs: a raised tab for the one showing, a thin line between the others, and room for the name, the machine and the close button.",
            "Star a chat (its tab's menu, the star on a tab under the pointer, or the details sheet): starred tabs sit at the left, come back on launch even when the chat has stopped (with Resume, never on their own), and starred chats come first in the chat list. A star follows the chat into Start fresh and Continue.",
            "Each terminal tab has a live state dot (working breathes, warm, cold, waiting), and an open chat's terminal has a slim strip on top: Working, \"Warm, 4:12 left\" counting down each second (amber in the last minute), or Cold and what the next message re-reads. It follows the terminal's own output, so it no longer waits for the next poll.",
            "Long names, needs and answers wrap or end in an ellipsis instead of running into buttons, and the confirm box, the usage panel and the header fit narrow windows.",
        ],
    ),
    (
        "0.5.1",
        &[
            "The details sheet's top line ends in an ellipsis when a chat's need is long, so its close and pop-out buttons always stay on screen.",
        ],
    ),
    (
        "0.5.0",
        &[
            "Usage: the Week chip opens a bigger panel. This week as a bar per day (hover a bar for its exact number), and tiles for the 5-hour window, this week with its pace, and the last 4 weeks.",
            "Terminal tabs drag to reorder. The order is saved, and Cmd+1 to 9 follow it. Dragged below the tab row, a tab still opens in its own window.",
            "A gear at the top right opens Settings. Light and dark are in Settings > Appearance, the View menu and Shift Cmd L.",
            "About Switchboard is its own window now, with this list and Copy version info.",
            "Waiting chats say what they need (\"Needs: confirm option 1 or 2\"), and the details sheet has a Reply button with the suggested answer, typed into the chat's terminal.",
            "A chat waiting more than a day is parked: a muted pill and \"Waiting 2 days\". It no longer counts toward the badge or notifications.",
        ],
    ),
    (
        "0.4.0",
        &[
            "Tool Library, a new Settings page: the skills and MCP servers you build, kept on the server and shared by every machine. Import one from a machine, or describe a new one and a chat builds it.",
            "Each tool shows its versions (any one can be made the latest again), which machines have it, and the keys it needs there. Keys stay in each machine's keychain.",
            "The New sheet has a Tools row, with the tools that are on for every new chat already checked. A chat's details show its tools, and Attach tool adds more.",
            "A machine's MCP servers each have Add to library.",
        ],
    ),
    (
        "0.3.1",
        &[
            "Double-clicking a stopped chat resumes it into a terminal tab, on any machine, instead of opening the Claude app.",
        ],
    ),
    (
        "0.3.0",
        &[
            "Terminal tabs you leave open come back the next time Switchboard opens.",
            "An About page, under Switchboard > About Switchboard.",
        ],
    ),
    (
        "0.2.0",
        &[
            "Board cards show each chat's live state (working, warm or cold) instead of how it was started.",
            "New names say \"part 3\" for a chat's third start and \"copy\" for a resumed copy. A normal new chat has no tag.",
        ],
    ),
];

/// "20261013193012" (the build scripts' UTC stamp) as "2026-10-13 19:30 UTC".
pub fn build_text(b: &str) -> String {
    if b.len() == 14 && b.bytes().all(|c| c.is_ascii_digit()) {
        format!("{}-{}-{} {}:{} UTC", &b[0..4], &b[4..6], &b[6..8], &b[8..10], &b[10..12])
    } else {
        b.to_string()
    }
}

fn os_arch() -> String {
    let os = match sb_core::update::platform() {
        "mac" => "macOS",
        "windows" => "Windows",
        _ => "Linux",
    };
    let arch = if cfg!(target_os = "macos") {
        if cfg!(target_arch = "aarch64") { "Apple silicon" } else { "Intel" }
    } else if cfg!(target_arch = "aarch64") {
        "ARM"
    } else {
        "x86-64"
    };
    format!("{os}, {arch}")
}

/// The notices built into the app, for a build that has no copy next to it (a dev build, the bare Linux binary).
const NOTICES: &str = include_str!("../../../packaging/THIRD-PARTY-NOTICES.txt");
const NOTICES_FILE: &str = "THIRD-PARTY-NOTICES.txt";

/// The notices file to open: the copy shipped beside the app (Switchboard.app/Contents/Resources on a Mac, next to
/// the exe or binary elsewhere), else the built-in copy written to the temp folder.
fn notices_path() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    let shipped = [dir.join("../Resources").join(NOTICES_FILE), dir.join(NOTICES_FILE)];
    if let Some(p) = shipped.into_iter().find(|p| p.is_file()) {
        return Some(p);
    }
    let p = std::env::temp_dir().join(format!("Switchboard-{NOTICES_FILE}"));
    std::fs::write(&p, NOTICES).ok()?;
    Some(p)
}

/// Opens a file or folder with the system (Finder, Explorer, the desktop's file manager). `shots` builds only log it.
fn open_path(cx: &mut App, p: &Path) {
    #[cfg(feature = "shots")]
    {
        let _ = cx;
        eprintln!("sb-open {}", p.display());
    }
    #[cfg(not(feature = "shots"))]
    cx.open_with_system(p);
}

/// Copy diagnostics: what a bug report needs and nothing private (no server address, home folder as ~).
pub fn diagnostics(log_dir: Option<&Path>) -> String {
    let build = sb_core::update::this_build().map(build_text).unwrap_or_else(|| "development build".into());
    let mut s = format!(
        "Switchboard {}\nBuild: {build}\nRuns on: {}\nOS: {}\n",
        sb_core::version(),
        os_arch(),
        crate::logs::os_version()
    );
    let lines = log_dir.map(|d| crate::logs::tail(d, 200)).unwrap_or_default();
    if lines.is_empty() {
        s.push_str("\nNo log lines yet.\n");
    } else {
        s.push_str(&format!("\nLast {} log lines:\n", lines.len()));
        for l in lines {
            s.push_str(&crate::logs::scrub(&l));
            s.push('\n');
        }
    }
    s
}

/// Opens About, or brings it forward when it is open.
pub fn open(cx: &mut App) {
    let app = crate::state::app(cx);
    if let Some(h) = app.read(cx).about_win
        && h.update(cx, |_, window, _| window.activate_window()).is_ok()
    {
        return;
    }
    let opts = crate::files::options("About Switchboard", 460., 620., (400., 420.), cx);
    match cx.open_window(opts, |_, cx| cx.new(AboutView::new)) {
        Ok(h) => app.update(cx, |s, _| s.about_win = Some(h.into())),
        Err(e) => log::error!("could not open About: {e:#}"),
    }
}

pub struct AboutView {
    app: Entity<AppState>,
    /// The button that shows "Copied" for a moment after a copy.
    copied: Option<&'static str>,
}

impl AboutView {
    fn new(cx: &mut Context<Self>) -> AboutView {
        let app = crate::state::app(cx);
        cx.observe(&app, |_, _, cx| cx.notify()).detach();
        AboutView { app, copied: None }
    }

    /// The lines Copy version info puts on the clipboard, for a bug report.
    fn info(&self, cx: &App) -> String {
        let build = sb_core::update::this_build().map(build_text).unwrap_or_else(|| "development build".into());
        let server = self.app.read(cx).core.server().unwrap_or_else(|| "not connected".into());
        format!("Switchboard {}\nBuild: {build}\nRuns on: {}\nServer: {server}", sb_core::version(), os_arch())
    }

    fn copy(&mut self, which: &'static str, cx: &mut Context<Self>) {
        let text = if which == "diagnostics" { diagnostics(crate::logs::dir().as_deref()) } else { self.info(cx) };
        crate::state::copy_text(cx, text);
        self.copied = Some(which);
        cx.notify();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(1800)).await;
            let _ = this.update(cx, |v, cx| {
                v.copied = None;
                cx.notify();
            });
        })
        .detach();
    }
}

impl Render for AboutView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = Theme::get(cx);
        let version = sb_core::version();
        let build = sb_core::update::this_build().map(build_text);
        let server = self.app.read(cx).core.server();
        let mono = |s: String| div().font_family(NUM).text_size(px(12.5)).text_color(t.ink2).child(s);
        let row = |label: &str, value: gpui::Div, first: bool| {
            div()
                .px(px(14.))
                .py(px(10.))
                .flex()
                .items_center()
                .gap(px(16.))
                .when(!first, |d| d.border_t_1().border_color(t.line))
                .child(div().w(px(72.)).flex_none().text_size(px(13.)).text_color(t.dim).child(label.to_string()))
                .child(div().flex_1().min_w_0().flex().justify_end().child(value))
        };
        let hero = div()
            .pt(px(18.))
            .pb(px(10.))
            .flex()
            .flex_col()
            .items_center()
            .gap(px(4.))
            // The app's icon as the Dock shows it: the artwork keeps Apple's margin, so 120 px draws a ~96 px tile.
            .child(gpui::img(crate::assets::APP_ICON).flex_none().size(px(120.)).mb(px(2.)))
            .child(
                div()
                    .font_family(crate::theme::BRAND)
                    .text_size(px(24.))
                    .font_weight(FontWeight::BOLD)
                    .text_color(t.ink)
                    .child("Switchboard"),
            )
            .child(
                div()
                    .text_size(px(13.))
                    .text_color(t.dim)
                    .font_features(ui::tnum())
                    .child(format!("Version {version}")),
            )
            .child(
                div()
                    .mt(px(6.))
                    .text_size(px(12.5))
                    .text_color(t.faint)
                    .child("Claude Code on every machine, in one place."),
            );
        let facts = div()
            .rounded(px(ui::R_CARD))
            .border_1()
            .border_color(t.line)
            .bg(t.panel)
            .flex()
            .flex_col()
            .child(row("Version", mono(version.to_string()), true))
            .child(row(
                "Build",
                match &build {
                    Some(b) => mono(b.clone()),
                    None => div().text_size(px(12.5)).text_color(t.faint).child("Development build"),
                },
                false,
            ))
            .child(row("Runs on", mono(os_arch()), false))
            .child(row(
                "Server",
                match &server {
                    Some(s) => mono(s.clone()),
                    None => div().text_size(px(12.5)).text_color(t.faint).child("Not connected yet"),
                },
                false,
            ));
        let button = |id: &'static str, icon: &'static str, label: &'static str| {
            ui::line_btn(&t, id, 32.).px(px(14.)).text_size(px(13.)).child(ui::icon(icon, 14., t.ink2)).child(label)
        };
        let copy_button = |which: &'static str, label: &'static str| {
            let done = self.copied == Some(which);
            ui::line_btn(&t, which, 32.)
                .px(px(14.))
                .text_size(px(13.))
                .child(ui::icon(if done { "check" } else { "copy" }, 14., if done { t.warm } else { t.ink2 }))
                .child(if done { "Copied" } else { label })
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.copy(which, cx)))
        };
        let copy = div()
            .flex()
            .flex_wrap()
            .justify_center()
            .gap(px(8.))
            .child(copy_button("version", "Copy version info"))
            .child(copy_button("diagnostics", "Copy diagnostics"))
            .child(button("about-log", "folder", "Show log").on_click(|_: &ClickEvent, _, cx| {
                if let Some(d) = crate::logs::dir() {
                    let _ = std::fs::create_dir_all(&d);
                    open_path(cx, &d);
                }
            }))
            .child(button("about-licenses", "file", "Open source licenses").on_click(|_: &ClickEvent, _, cx| {
                match notices_path() {
                    Some(p) => open_path(cx, &p),
                    None => log::warn!("could not write the license notices to the temp folder"),
                }
            }));
        let mut news = div().flex().flex_col().gap(px(10.));
        for (v, notes) in RELEASES {
            let current = *v == version;
            let mut list = div().flex().flex_col().gap(px(7.));
            for n in notes.iter() {
                list = list.child(
                    div()
                        .flex()
                        .items_start()
                        .gap(px(9.))
                        .child(div().flex_none().mt(px(7.)).size(px(4.)).rounded_full().bg(if current {
                            t.accent
                        } else {
                            t.faint
                        }))
                        .child(
                            ui::wrap(n.to_string())
                                .flex_1()
                                .min_w_0()
                                .text_size(px(13.))
                                .line_height(relative(1.45))
                                .text_color(t.ink2),
                        ),
                );
            }
            news = news.child(
                div()
                    .p(px(14.))
                    .rounded(px(ui::R_CARD))
                    .border_1()
                    .border_color(if current { t.accent.alpha(0.45) } else { t.line })
                    .bg(t.panel)
                    .flex()
                    .flex_col()
                    .gap(px(10.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(
                                div()
                                    .font_family(NUM)
                                    .text_size(px(13.))
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(t.ink)
                                    .child(v.to_string()),
                            )
                            .when(current, |d| d.child(ui::tag(&t, "This version"))),
                    )
                    .child(list),
            );
        }
        let body = div()
            .id("about")
            .size_full()
            .overflow_y_scroll()
            .bg(t.bg)
            .text_color(t.ink)
            .font_family(crate::theme::UI)
            .child(
                div()
                    .px(px(22.))
                    .pb(px(26.))
                    .flex()
                    .flex_col()
                    .gap(px(16.))
                    .child(hero)
                    .child(facts)
                    .child(copy)
                    .child(
                        div()
                            .mt(px(6.))
                            .text_size(px(15.))
                            .font_weight(FontWeight::BOLD)
                            .text_color(t.ink)
                            .child("What's new"),
                    )
                    .child(news),
            );
        crate::frame::framed(window, &t, "About Switchboard", body)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn diagnostics_have_the_version_the_os_and_the_log_tail() {
        let dir = std::env::temp_dir().join(format!("sb-diag-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let lines: String = (0..250).map(|i| format!("line {i}\n")).collect();
        std::fs::write(dir.join(crate::logs::FILE), lines).unwrap();
        let d = super::diagnostics(Some(&dir));
        assert!(d.starts_with(&format!("Switchboard {}\n", sb_core::version())), "{d}");
        assert!(d.contains("\nOS: "), "{d}");
        assert!(d.contains("Last 200 log lines:\nline 50\n"), "{d}");
        assert!(d.ends_with("line 249\n"), "{d}");
        assert!(!d.contains("Server"), "{d}");
        assert!(super::diagnostics(None).contains("No log lines yet."));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_notices_are_built_in() {
        assert!(super::NOTICES.starts_with("Third-party notices for "));
        assert!(super::NOTICES.contains("SIL Open Font License"));
    }
}
