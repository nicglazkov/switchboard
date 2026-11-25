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
        "0.9.1",
        &[
            "iPhone notifications work again on a home computer updated to 0.9.0. That version ran the server on its own Python, which lacked a package the notifications needed, so they turned off. The server now signs Apple's token itself and needs no package for it.",
        ],
    ),
    (
        "0.9.0",
        &[
            "Device keys: a computer can now reach your home computer by its own key in place of an address. You make a short code or a link on the home computer, enter it on the other one, and the two stay connected at home or away, encrypted between the two, with nothing to open on your router. Keys are off until you turn them on (`switchboard keys on`, or Settings > Devices).",
            "Devices: Settings has a new Devices section on the home computer. It lists the paired computers and phones, makes pairing codes, and removes or blocks a device.",
            "When the home computer is away: with keys on, each paired computer answers for its own chats while the home computer is off or out of reach. You still see those chats, open their terminals and reply. The board says what is missing and goes back by itself when the home computer answers again.",
            "Move or replace the home computer: the home computer can be moved to another of your computers, backed up behind a recovery code and restored on a new computer, with nothing to pair again. Command line only for now.",
            "Your own relay: two computers that cannot reach each other directly can meet through a relay you run. A script installs it on a small Linux server. The relay cannot read what passes through it.",
            "Its own Python: the installers bring Python 3.13 for Switchboard's own use, so the home computer and the computers you add need no Python of their own.",
            "The home computer adds itself: a first install links the home computer's own chats to the board with nothing to paste. `switchboard link` does it later.",
            "One command to remove everything: `switchboard uninstall` takes Switchboard off a computer, and `--purge` takes its settings and history too.",
            "A Windows home computer gets the hub: chat terminals and adding a computer with a code now work on a home computer that runs Windows.",
            "A busy computer is no longer called offline: a computer that took longer than 15 seconds to list its chats was shown offline and came back seconds later, which closed and reopened its chats in the apps. It now takes three slow answers in a row, and listing chats on a computer with a very full temp folder is about twenty times faster.",
            "After a restore from an old backup: a home computer brought back from a backup made before another one replaced it now hears of the newer one and stands down, so two computers never answer as home.",
            "Home is noticed sooner: a paired computer notices the home computer coming back in about 10 seconds, not up to a minute, and no longer says that no other computer can be reached while it is still asking them.",
            "Pairing on Linux with Docker: a pairing code no longer carries an address from a container or virtual machine network in place of the one that works.",
            "The home computer's name: a paired computer shows what the home computer is called now, after a rename or a move, not the name from the day it was paired.",
        ],
    ),
    (
        "0.8.23",
        &[
            "Only your own networks: the server now looks at the address each request really comes from, and answers only this computer, home and office networks, link-local addresses and Tailscale. A request from anywhere else is refused, whatever name it asks for. Nothing changes on a home network or Tailscale. To let another network in, add it to \"peers\" in server.json.",
            "Damaged files: a settings or state file that cannot be read after a power cut or a full disk is no longer taken for an empty one and written over. The server keeps a copy of each file from before its last change and goes back to it, and keeps the damaged file beside it. With no copy to go back to it picks the careful answer, not the open one: a lost settings file no longer turns the opt-in settings on, a lost computer list no longer gives every computer its whole home folder, a lost server.json answers this computer only, and chats do not stop by themselves while the list of kept chats or open tabs is lost. A terminal key that was cut short is replaced instead of failing every terminal.",
            "Damaged files on each computer: the names you gave chats, the list of installed tools, and the secrets file on a Linux computer without a keychain are kept the same way. A damaged one goes back to its last good copy instead of being written over with the one entry that changed. A state file that holds the wrong kind of thing no longer breaks the list of chats.",
            "Damaged files in the app and on a linked computer: the app's settings and a linked computer's link to the server are kept the same way. One that a power cut emptied goes back to its last good copy, so the app keeps your settings and the computer stays linked without pairing again. With no copy, the app uses its defaults and does not save them until you change a setting. On the server, the tool library goes back to its last good copy too, and a damaged list of launches is put aside and starts over.",
            "Windows: the node on a linked PC is started again within seconds if it stops or is ended in Task Manager. After a restart or a power cut it returns when you sign in to Windows.",
            "Budget: two devices changing the budget at the same moment both keep their change, and lifting the hard limit while it is being checked is not lost.",
            "A request that got lost: a chat you started, resumed or moved always gets an answer. If the helper stopped right after taking the request, or could not read it, you are told to try again at once instead of waiting for the timeout.",
            "Behind a proxy: the server's calls to its own hub and each computer's calls to your server no longer go through http_proxy, which made linked computers look offline.",
            "Accents and other alphabets: text files are always read as UTF-8, so a CLAUDE.md with accents on Windows no longer stops the setup changes halfway, and a chat title with an accent no longer fails its computer. CLAUDE.md is never left half-written.",
            "Server update: a server that cannot read its own version no longer takes whatever version it is sent. Run the install line again to fix it.",
            "Less work when idle: Refresh from several devices at once shares one round of asking the computers, and the server no longer rewrites a file to disk every 20 seconds.",
            "Honest costs: what Resume and Start fresh are said to cost now matches a month of real use (178 fresh starts). Start fresh was shown about 3.5 times too cheap, because a new chat was counted as only its summary; it holds about 58k tokens of its own before that. A cold Resume was shown 16% too high. Chats under 80k (it was 60k) are now called small, since that is where Start fresh starts to pay.",
            "Pasted text: text you paste into a message now reaches the summary, and a pasted message to a cold big chat is held like a typed one. Both used to be dropped as if Claude Code had written them.",
            "Scheduled prompts are not held: a prompt fired by a schedule or a loop goes through to a cold chat instead of waiting on a hold nobody is there to answer. A message you type is still held.",
            "Shorter handoffs: summaries are asked to stay under 1,000 words, and a long one loads without its least needed sections instead of making the new chat read a file first. Your standing instructions are never cut.",
            "What a summary costs: each summary's own usage is now recorded and counted against what Start fresh saved, and each computer keeps a short log of fresh starts, holds and stops (sizes and times, never chat text).",
        ],
    ),
    (
        "0.8.22",
        &[
            "Several chats at once: Start fresh, Resume or Compact on several chats in a row no longer queues them behind each other. A computer used to write one summary at a time, about 45 seconds each, so the sixth chat waited five minutes and a plain new chat waited behind all of them. Summaries are now written three at a time, a new chat or a resume starts right away, and a large batch is no longer dropped for waiting too long.",
            "After a power cut or restart: a chat that was open in a tab and was ended by its computer restarting keeps its tab. The tab says it stopped when that computer restarted and offers Resume, instead of closing without a word. Tabs of chats you stopped yourself still go.",
            "Settings and state survive a power cut: every file Switchboard keeps (on the server and on each computer) is now forced to disk before it replaces the old one. A file could come back from a power cut filled with zeros. Half-written temp files left by a cut are cleared.",
            "Clean up: the card no longer suggests a starred chat, one waiting on your answer, one that is working, or one with a held message. It follows the same rules as Stop idle chats. A chat with a held message is not stopped by itself either.",
            "Server log: a phone or app that drops its connection no longer prints an error into the server's log.",
        ],
    ),
    (
        "0.8.21",
        &[
            "Mac: the window keeps drawing when macOS reports it covered while you are using it. That state left the window movable but showing nothing new, while typing still reached the chat. The app now draws such a window itself, draws by hand any change that has waited seconds for a frame, and writes down at quit whether the window was drawing, what macOS thought of it and the last clicks, so a freeze that still happens explains itself in the log.",
            "Tabs show on the press: a click that moved a couple of pixels was taken for a drag and did not switch tabs.",
            "Idle chats stop by themselves: a chat that finished its reply and has been left alone for 6 hours is stopped to free its memory, and Resume brings it back where it was. Never a chat that is working, waiting on you, running a command, subagent or scheduled task, starred, set to Keep running, open in a tab or terminal, or resumed within the wait; its own computer checks once more at the moment of the stop. Change the wait or turn it off in Settings > Chats > Stop idle chats. A stopped chat can't be reached from the Claude phone app until you resume it.",
            "Less memory on each computer: the helpers behind every chat and terminal tab take about 24 MB instead of 55, a tab's helper no longer starts a process three times a second, and a closed tab's helpers leave sooner.",
            "Sent by Claude: tasks a chat sent to another computer are told apart from your own chats. They sit in a folded \"Sent by Claude\" line per computer, do not count in Needs you or the Board badge unless one is really stopped at a prompt, are stopped two minutes after they report, and move to History once the report is read. History can filter them, and Move to my chats keeps one.",
            "Usage: the panel now shows what Claude's own usage page shows: every plan limit with its reset time and time left, each model's weekly limit, extra usage (spent, monthly limit, left, balance), and this week by product. It says when it was updated, has Refresh, and links to claude.ai to change limits. Most used this week replaces Costly to go back to: the chats that used the most since the weekly reset, with each one's share (an estimate from its tokens). The numbers come from Claude Code on one of your computers; Switchboard never reads your sign-in.",
            "Context window: Settings > Context window replaces the \"Compact at 300k\" switch with one slider (150k, 200k, 300k recommended, 500k, or Claude Code's default) that says what each choice keeps and costs. Context meters and the Start fresh suggestion follow it. A value you set by hand on a computer is left alone and shown as such.",
            "Settings: the sidebar is grouped (General, Chats, Claude, Machines) with Updates and About at the bottom, every line is reachable with Tab, and Stop idle chats sits with Chats.",
            "Polish: grey text is easier to read in both themes, shell tab names are no longer cut short, History rows stay two lines in a small window, an empty search says what to try, a Library that fails to load offers Try again, and close buttons are larger.",
        ],
    ),
    (
        "0.8.20",
        &[
            "Usage numbers hold still: the weekly and 5-hour figures are now worked out from every chat's latest reading for each window, so they no longer jump between old values when an idle chat redraws. A 5-hour window that has ended reads as empty instead of showing its last number, and a reading that cannot be right (a reset further away than the window is long) is ignored.",
            "Daily budget: budgets set below an even pace now take effect (the rule that leaves room for the rest of the week only applies in the last 48 hours before the weekly reset), the − and + buttons land on whole numbers, and the usage tool's wording matches the numbers it reports.",
            "Flags stay with the chat: Resume, Start fresh, Compact and Continue on another computer keep the chat's permission mode, extra folders, agent, extra instructions, model and Chrome setting. A flag the other computer cannot take is left out and named, and a chat always resumes even if an old flag is refused.",
            "Permissions: \"Asks first\" is a choice of its own and the empty choice reads \"Machine default\". Long flag values are kept whole, a worktree is shown as set at start, and Esc on a flags drawer with unsaved changes asks for a second press.",
            "No double starts: when a computer is slow to answer, Switchboard no longer sends the same start, resume or handoff again, and says the first one may have gone through. \"Auto\" with flags, checks on dispatched work, and waiting for a busy chat are fixed.",
            "Linked computers keep their settings: a computer the hub stops listing stays in the list, offline, with a line on how to link it again, instead of losing its folder and name.",
            "Smoother chats on Windows computers: the hub gathers a chat's output into fewer, larger pieces, so a PC chat that prints in tiny bursts no longer floods the app with redraws. Large pastes into a slow program no longer end the terminal.",
            "Terminal: keys typed while a connection drops are held for a moment and sent only to the same chat, a chat on its alternate screen keeps its scrollback across a reconnect, a program can copy to the clipboard only from the tab you are looking at, and a window that stops drawing is nudged awake and noted in the log.",
            "Board and keys: single-letter shortcuts no longer fire while you type or while a menu, drawer or question is open. Cmd/Ctrl+W on the board with tabs open asks for a second press, quitting asks when one of your own shells is running a program, tabs narrow before the row scrolls, a finished job no longer pulls the window forward, and New chat's Cancel keeps your draft.",
            "Updates: the header shows Downloading with a percentage, Installing, Installs on quit or Update failed. Opening the app twice during an update no longer rolls it back, and a real rollback says so once and offers to try again. A server update keeps the old server until the new one answers and puts it back if it does not.",
            "Settings: server addresses have their own page with a field to type one, removing things asks for a second click, switches that fail to save flip back, terminal text size is kept per computer, and Add a machine counts its code down and shows the computer when it links.",
            "Files privacy: a private folder can no longer be reached under another name (different capitals, a link, or with your home folder as the Claude folder).",
            "On each computer: one unreadable chat no longer blanks that computer's list, Switchboard never guesses that chats have stopped when Claude Code cannot be asked, changes to Claude Code's settings are written safely next to its own, very long first messages are delivered by file, and old leftover state is tidied.",
            "iPhone: the app reconnects by itself when the network comes back, tabs follow a chat that was resumed under a new id, messages sent while connecting are queued and returned to the box if they cannot be sent, choice buttons work while scrolled up, Stop and dismiss does both, and rows no longer go grey on a quiet day.",
        ],
    ),
    (
        "0.8.19",
        &[
            "Closing a tab leaves its chat running: closing a chat's tab (X, Cmd+W, closing its group) no longer stops the chat, so looking in on a background chat never ends it. Stop on the chat's card still ends it. To have chats stop with their tabs as before, turn on Settings > Terminal > Stop a chat when its tab closes.",
            "Windows: updating the node no longer sets up its logon task again. Windows Defender could take that for malware and remove the node, leaving the PC offline. An update now only swaps the program.",
            "Mac: updating the node no longer leaves it stopped when the old one is slow to quit.",
        ],
    ),
    (
        "0.8.18",
        &[
            "Tabs come back at once: when Switchboard opens, your chat tabs reopen right away from what it saved last time, instead of staying away until the server answers. If the server is slow or away, they keep trying and connect when it is back.",
            "A silent server shows: if the server stops answering while you type, the terminal says so within a few seconds and reconnects by itself. Before, the keys could look dead for up to a minute, as if the app had frozen.",
            "Typing never waits on output: a chat printing a lot at once no longer holds back what you type into it.",
            "Screens that fall behind are redrawn: if a terminal ever has output it has not drawn, it draws it and notes it in the log.",
            "Better freeze notes: the log now says when the window stops drawing while you use it, and what each tab was doing when the app quit, so a quit because it looked frozen leaves its reason behind.",
        ],
    ),
    (
        "0.8.17",
        &[
            "Computer use, asked for in the chat: a chat that has no screen tools can now turn computer use on for itself. Switchboard sets it up for the chat's folder, relaunches the chat with it, and the chat carries on with what it was doing.",
            "Smoother chats: the terminal no longer redraws every row each time a status dot pulses, so an open chat uses much less of the computer while chats work.",
            "Far fewer disk writes: the saved copy of the board is written at most once a minute instead of every second (macOS had flagged the app for writing about 2 GB a day).",
            "Steady cursor on Windows chats: a chat running on a PC no longer flashes its cursor several times a second while Claude works.",
            "Freeze reports: if the app's window stops responding for 8 seconds or more, it notes it in its log and, on a Mac, saves where it was stuck, so a freeze can be found and fixed.",
        ],
    ),
    (
        "0.8.16",
        &[
            "Model menu: the model picker is now one menu, built from Claude Code's own model catalog, so a new Claude model (Fable 5.2, out today) shows up the day it ships, with a line on what each is for. Older versions you can still pin sit under More models. Same menu in the desktop app, on the iPhone and on Android.",
        ],
    ),
    (
        "0.8.15",
        &[
            "Computer use on Windows: Turn on computer use now works for a chat on a PC too. Switchboard sets up Windows-MCP for the chat's folder (installing it with uv the first time), so Claude can see the screen and click and type in its apps. If the PC's desktop is disconnected (a Remote Desktop window closed without signing out), the chat's details say so.",
        ],
    ),
    (
        "0.8.14",
        &[
            "Turn on computer use works from the chat's details: in 0.8.13 the server stopped it with an error before the chat relaunched.",
        ],
    ),
    (
        "0.8.13",
        &[
            "A chat relaunched with Claude in Chrome keeps it: the chat shows Chrome on, and the next relaunch, from the iPhone or the desktop, carries it over instead of dropping it.",
            "Computer use for any chat in one click: Turn on computer use in the chat's details on the desktop, or at the top of its flags on the iPhone. On a Mac Switchboard turns it on for the chat's folder and relaunches the chat once Claude is free, and it carries on where it was; a chat open in a terminal gets the command to run there. The first time, macOS asks to allow Accessibility and Screen Recording. On Windows it says why it isn't there.",
            "Jobs and messages in the bottom right corner move beside the flags drawer and the chat's details instead of hiding under them.",
            "The desktop app uses less CPU: its breathing dots and spinners draw a few frames a second instead of every frame.",
        ],
    ),
    (
        "0.8.12",
        &[
            "Choosing a folder looks like Finder or File Explorer: places down the side, Back, Forward and Up, a path bar, and the folder's contents. It's how New chat picks its folder and adds folders Claude may also use, on the desktop and on the iPhone.",
            "A chat's terminal shows its cache on its own, Warm or Cold, apart from whether it needs you, and Start fresh is a click away beside it. It asks first, saying what it does.",
            "Claude Code's status line in a terminal refreshes every 15 seconds, so its warm or cold stays true while nobody types.",
            "Quiet keeps the newest 8 chats per computer.",
            "The iPhone app opens with no chats open after it was closed, instead of reopening old tabs.",
        ],
    ),
    (
        "0.8.11",
        &[
            "Settings > Server shows whether your server and its parts are running, and updates it with one click when a new version is out. It can also restart it, show its logs and copy its details for a bug report.",
            "Quiet keeps each chat to one line, up to 12 per computer from the last two days. Older ones are in History.",
            "A button at the end of the tab row lists every open tab, by group.",
            "A right-click on a pill or tip over a terminal no longer pastes into it, and Windows terminals start reliably.",
        ],
    ),
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

/// The system the notes are worded for. They are written once, as a Mac says them; `note` rewords them elsewhere.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Os {
    Mac,
    Windows,
    Linux,
}

impl Os {
    pub(crate) fn here() -> Os {
        if cfg!(target_os = "macos") {
            Os::Mac
        } else if ui::win() {
            Os::Windows
        } else {
            Os::Linux
        }
    }
}

/// Phrases in the notes that name a Mac's keys, menus or apps, as written in RELEASES, with what a Windows and a
/// Linux computer call the same thing. The keys are the ones bound in view.rs and term/mod.rs: Ctrl where a Mac
/// has Cmd, Ctrl+Shift for the ones a terminal would otherwise take.
const WORDING: &[(&str, &str, &str)] = &[
    ("Cmd/Ctrl+W on the board", "Ctrl+W on the board", "Ctrl+W on the board"),
    ("(X, Cmd+W,", "(X, Ctrl+Shift+W,", "(X, Ctrl+Shift+W,"),
    ("looks like Finder or File Explorer", "looks like File Explorer", "looks like a file manager"),
    ("Redraw (Cmd+Shift+R)", "Redraw (Ctrl+Shift+R)", "Redraw (Ctrl+Shift+R)"),
    ("(Cmd+K, Cmd+T)", "(Ctrl+K, Ctrl+T)", "(Ctrl+K, Ctrl+T)"),
    ("Cmd+1 to 9", "Ctrl+1 to 9", "Ctrl+1 to 9"),
    (
        "the View menu and Shift Cmd L",
        "the menu at the top right and Ctrl+Shift+L",
        "the menu at the top right and Ctrl+Shift+L",
    ),
    (
        "so every Mac shows the same ones (folding is per Mac)",
        "so every computer shows the same ones (folding is per computer)",
        "so every computer shows the same ones (folding is per computer)",
    ),
    ("or the Mac is asleep", "or the computer is asleep", "or the computer is asleep"),
    ("under Switchboard > About Switchboard", "in the menu at the top right", "in the menu at the top right"),
];

/// Notes about the Mac app alone (its disk image, the key field that saves into the Mac's keychain), by how they
/// start: left out on Windows and Linux. A note that starts "Mac:" stays everywhere: it is labelled, and a Mac may
/// be one of the computers this app looks after.
const MAC_APP_ONLY: &[&str] = &[
    "The disk image's window",
    "The installer's \"Eject the disk image\"",
    "Settings > Library > Set key... on this Mac",
];

/// One note as this system should read it, or None when it is about another system's app alone.
pub(crate) fn note(text: &str, os: Os) -> Option<String> {
    if os == Os::Mac {
        return Some(text.to_string());
    }
    if MAC_APP_ONLY.iter().any(|p| text.starts_with(p)) {
        return None;
    }
    let mut out = text.to_string();
    for (mac, win, linux) in WORDING {
        if out.contains(mac) {
            out = out.replace(mac, if os == Os::Windows { win } else { linux });
        }
    }
    Some(out)
}

/// The versions What's new lists in a build of `version`: that one and the ones before it. Notes for a version
/// this build is not yet (they are written before the number goes up) are left out. A version that cannot be read
/// hides nothing.
pub(crate) fn releases_upto(version: &str) -> impl Iterator<Item = &'static (&'static str, &'static [&'static str])> {
    use sb_core::update::Ver;
    let mine = Ver::parse(version);
    RELEASES.iter().filter(move |(v, _)| match (&mine, Ver::parse(v)) {
        (Some(mine), Some(v)) => v <= *mine,
        _ => true,
    })
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
        let os = Os::here();
        for (v, notes) in releases_upto(version) {
            let current = *v == version;
            let mut list = div().flex().flex_col().gap(px(7.));
            for n in notes.iter().filter_map(|n| note(n, os)) {
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
                            ui::wrap(n)
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
    use super::{MAC_APP_ONLY, Os, RELEASES, WORDING, note, releases_upto};

    fn all_notes() -> impl Iterator<Item = &'static str> {
        RELEASES.iter().flat_map(|(_, notes)| notes.iter().copied())
    }

    #[test]
    fn whats_new_stops_at_the_version_that_is_running() {
        let shown = |v: &str| releases_upto(v).map(|(v, _)| *v).collect::<Vec<_>>();
        assert!(RELEASES.iter().any(|(v, _)| *v == "0.8.22"));
        let at = shown("0.8.22");
        assert_eq!(at.first(), Some(&"0.8.22"), "the running version leads");
        assert!(!at.contains(&"0.8.23") && at.contains(&"0.8.21"));
        assert_eq!(shown("0.8.19").first(), Some(&"0.8.19"));
        // A version past every note, and one that cannot be read: all of them.
        assert_eq!(shown("99.0.0").len(), RELEASES.len());
        assert_eq!(shown("dev").len(), RELEASES.len());
        // Whatever this build is, it never lists a version after its own.
        let mine = sb_core::update::Ver::parse(sb_core::version()).expect("the app's version parses");
        for (v, _) in releases_upto(sb_core::version()) {
            assert!(sb_core::update::Ver::parse(v).is_none_or(|v| v <= mine), "{v}");
        }
    }

    #[test]
    fn notes_say_ctrl_and_the_right_file_manager_off_a_mac() {
        // A Mac reads the notes as they are written.
        for n in all_notes() {
            assert_eq!(note(n, Os::Mac).as_deref(), Some(n));
        }
        // Every phrase the table rewords is still in the notes, and so is every Mac-only note.
        for (mac, win, linux) in WORDING {
            assert!(all_notes().any(|n| n.contains(mac)), "no note says {mac:?} any more");
            assert!(!win.contains("Cmd") && !linux.contains("Cmd") && !linux.contains("Explorer"));
        }
        for p in MAC_APP_ONLY {
            assert!(all_notes().any(|n| n.starts_with(p)), "no note starts {p:?} any more");
        }
        // Elsewhere: no Mac key, menu or app is named. "Mac" itself may be: a note can be about a Mac computer.
        for os in [Os::Windows, Os::Linux] {
            for n in all_notes().filter_map(|n| note(n, os)) {
                for mac in ["Cmd", "⌘", "⌥", "Finder", "View menu", "this Mac", "Switchboard > About", "disk image"]
                {
                    assert!(!n.contains(mac), "{os:?} would read {mac:?} in: {n}");
                }
            }
        }
        let folder = all_notes().find(|n| n.starts_with("Choosing a folder looks like")).unwrap();
        assert!(note(folder, Os::Windows).unwrap().starts_with("Choosing a folder looks like File Explorer: "));
        assert!(note(folder, Os::Linux).unwrap().starts_with("Choosing a folder looks like a file manager: "));
        let close = all_notes().find(|n| n.starts_with("Closing a tab leaves its chat running")).unwrap();
        assert!(note(close, Os::Windows).unwrap().contains("(X, Ctrl+Shift+W, closing its group)"));
        assert_eq!(note("The disk image's window has room.", Os::Linux), None);
    }

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
