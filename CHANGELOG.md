# Changelog

What changed in each version of Switchboard, newest first. The app shows the same notes in
About > What's new. Downloads are on the
[releases page](https://github.com/nicglazkov/switchboard/releases).

## 0.8.23 (2026-11-24)

- Only your own networks: the server now looks at the address each request really comes from, and answers only this computer, home and office networks, link-local addresses and Tailscale. A request from anywhere else is refused, whatever name it asks for. Nothing changes on a home network or Tailscale. To let another network in, add it to "peers" in server.json.
- Damaged files: a settings or state file that cannot be read after a power cut or a full disk is no longer taken for an empty one and written over. The server keeps a copy of each file from before its last change and goes back to it, and keeps the damaged file beside it. With no copy to go back to it picks the careful answer, not the open one: a lost settings file no longer turns the opt-in settings on, a lost computer list no longer gives every computer its whole home folder, a lost server.json answers this computer only, and chats do not stop by themselves while the list of kept chats or open tabs is lost. A terminal key that was cut short is replaced instead of failing every terminal.
- Damaged files on each computer: the names you gave chats, the list of installed tools, and the secrets file on a Linux computer without a keychain are kept the same way. A damaged one goes back to its last good copy instead of being written over with the one entry that changed. A state file that holds the wrong kind of thing no longer breaks the list of chats.
- Budget: two devices changing the budget at the same moment both keep their change, and lifting the hard limit while it is being checked is not lost.
- A request that got lost: a chat you started, resumed or moved always gets an answer. If the helper stopped right after taking the request, or could not read it, you are told to try again at once instead of waiting for the timeout.
- Behind a proxy: the server's calls to its own hub and each computer's calls to your server no longer go through http_proxy, which made linked computers look offline.
- Accents and other alphabets: text files are always read as UTF-8, so a CLAUDE.md with accents on Windows no longer stops the setup changes halfway, and a chat title with an accent no longer fails its computer. CLAUDE.md is never left half-written.
- Server update: a server that cannot read its own version no longer takes whatever version it is sent. Run the install line again to fix it.
- Less work when idle: Refresh from several devices at once shares one round of asking the computers, and the server no longer rewrites a file to disk every 20 seconds.
- Honest costs: what Resume and Start fresh are said to cost now matches a month of real use (178 fresh starts). Start fresh was shown about 3.5 times too cheap, because a new chat was counted as only its summary; it holds about 58k tokens of its own before that. A cold Resume was shown 16% too high. Chats under 80k (it was 60k) are now called small, since that is where Start fresh starts to pay.
- Pasted text: text you paste into a message now reaches the summary, and a pasted message to a cold big chat is held like a typed one. Both used to be dropped as if Claude Code had written them.
- Scheduled prompts are not held: a prompt fired by a schedule or a loop goes through to a cold chat instead of waiting on a hold nobody is there to answer. A message you type is still held.
- Shorter handoffs: summaries are asked to stay under 1,000 words, and a long one loads without its least needed sections instead of making the new chat read a file first. Your standing instructions are never cut.
- What a summary costs: each summary's own usage is now recorded and counted against what Start fresh saved, and each computer keeps a short log of fresh starts, holds and stops (sizes and times, never chat text).

## 0.8.22 (2026-11-18)

- Several chats at once: Start fresh, Resume or Compact on several chats in a row no longer queues them behind each other. A computer used to write one summary at a time, about 45 seconds each, so the sixth chat waited five minutes and a plain new chat waited behind all of them. Summaries are now written three at a time, a new chat or a resume starts right away, and a large batch is no longer dropped for waiting too long.
- After a power cut or restart: a chat that was open in a tab and was ended by its computer restarting keeps its tab. The tab says it stopped when that computer restarted and offers Resume, instead of closing without a word. Tabs of chats you stopped yourself still go.
- Settings and state survive a power cut: every file Switchboard keeps (on the server and on each computer) is now forced to disk before it replaces the old one. A file could come back from a power cut filled with zeros. Half-written temp files left by a cut are cleared.
- Clean up: the card no longer suggests a starred chat, one waiting on your answer, one that is working, or one with a held message. It follows the same rules as Stop idle chats. A chat with a held message is not stopped by itself either.
- Server log: a phone or app that drops its connection no longer prints an error into the server's log.

## 0.8.21 (2026-11-18)

- Mac: the window keeps drawing when macOS reports it covered while you are using it. That state left the window movable but showing nothing new, while typing still reached the chat. The app now draws such a window itself, draws by hand any change that has waited seconds for a frame, and writes down at quit whether the window was drawing, what macOS thought of it and the last clicks, so a freeze that still happens explains itself in the log.
- Tabs show on the press: a click that moved a couple of pixels was taken for a drag and did not switch tabs.
- Idle chats stop by themselves: a chat that finished its reply and has been left alone for 6 hours is stopped to free its memory, and Resume brings it back where it was. Never a chat that is working, waiting on you, running a command, subagent or scheduled task, starred, set to Keep running, open in a tab or terminal, or resumed within the wait; its own computer checks once more at the moment of the stop. Change the wait or turn it off in Settings > Chats > Stop idle chats. A stopped chat can't be reached from the Claude phone app until you resume it.
- Less memory on each computer: the helpers behind every chat and terminal tab take about 24 MB instead of 55, a tab's helper no longer starts a process three times a second, and a closed tab's helpers leave sooner.
- Sent by Claude: tasks a chat sent to another computer are told apart from your own chats. They sit in a folded "Sent by Claude" line per computer, do not count in Needs you or the Board badge unless one is really stopped at a prompt, are stopped two minutes after they report, and move to History once the report is read. History can filter them, and Move to my chats keeps one.
- Usage: the panel now shows what Claude's own usage page shows: every plan limit with its reset time and time left, each model's weekly limit, extra usage (spent, monthly limit, left, balance), and this week by product. It says when it was updated, has Refresh, and links to claude.ai to change limits. Most used this week replaces Costly to go back to: the chats that used the most since the weekly reset, with each one's share (an estimate from its tokens). The numbers come from Claude Code on one of your computers; Switchboard never reads your sign-in.
- Context window: Settings > Context window replaces the "Compact at 300k" switch with one slider (150k, 200k, 300k recommended, 500k, or Claude Code's default) that says what each choice keeps and costs. Context meters and the Start fresh suggestion follow it. A value you set by hand on a computer is left alone and shown as such.
- Settings: the sidebar is grouped (General, Chats, Claude, Machines) with Updates and About at the bottom, every line is reachable with Tab, and Stop idle chats sits with Chats.
- Polish: grey text is easier to read in both themes, shell tab names are no longer cut short, History rows stay two lines in a small window, an empty search says what to try, a Library that fails to load offers Try again, and close buttons are larger.

## 0.8.20 (2026-11-14)

- Usage numbers hold still: the weekly and 5-hour figures are now worked out from every chat's latest reading for each window, so they no longer jump between old values when an idle chat redraws. A 5-hour window that has ended reads as empty instead of showing its last number, and a reading that cannot be right (a reset further away than the window is long) is ignored.
- Daily budget: budgets set below an even pace now take effect (the rule that leaves room for the rest of the week only applies in the last 48 hours before the weekly reset), the − and + buttons land on whole numbers, and the usage tool's wording matches the numbers it reports.
- Flags stay with the chat: Resume, Start fresh, Compact and Continue on another computer keep the chat's permission mode, extra folders, agent, extra instructions, model and Chrome setting. A flag the other computer cannot take is left out and named, and a chat always resumes even if an old flag is refused.
- Permissions: "Asks first" is a choice of its own and the empty choice reads "Machine default". Long flag values are kept whole, a worktree is shown as set at start, and Esc on a flags drawer with unsaved changes asks for a second press.
- No double starts: when a computer is slow to answer, Switchboard no longer sends the same start, resume or handoff again, and says the first one may have gone through. "Auto" with flags, checks on dispatched work, and waiting for a busy chat are fixed.
- Linked computers keep their settings: a computer the hub stops listing stays in the list, offline, with a line on how to link it again, instead of losing its folder and name.
- Smoother chats on Windows computers: the hub gathers a chat's output into fewer, larger pieces, so a PC chat that prints in tiny bursts no longer floods the app with redraws. Large pastes into a slow program no longer end the terminal.
- Terminal: keys typed while a connection drops are held for a moment and sent only to the same chat, a chat on its alternate screen keeps its scrollback across a reconnect, a program can copy to the clipboard only from the tab you are looking at, and a window that stops drawing is nudged awake and noted in the log.
- Board and keys: single-letter shortcuts no longer fire while you type or while a menu, drawer or question is open. Cmd/Ctrl+W on the board with tabs open asks for a second press, quitting asks when one of your own shells is running a program, tabs narrow before the row scrolls, a finished job no longer pulls the window forward, and New chat's Cancel keeps your draft.
- Updates: the header shows Downloading with a percentage, Installing, Installs on quit or Update failed. Opening the app twice during an update no longer rolls it back, and a real rollback says so once and offers to try again. A server update keeps the old server until the new one answers and puts it back if it does not.
- Settings: server addresses have their own page with a field to type one, removing things asks for a second click, switches that fail to save flip back, terminal text size is kept per computer, and Add a machine counts its code down and shows the computer when it links.
- Files privacy: a private folder can no longer be reached under another name (different capitals, a link, or with your home folder as the Claude folder).
- On each computer: one unreadable chat no longer blanks that computer's list, Switchboard never guesses that chats have stopped when Claude Code cannot be asked, changes to Claude Code's settings are written safely next to its own, very long first messages are delivered by file, and old leftover state is tidied.
- iPhone: the app reconnects by itself when the network comes back, tabs follow a chat that was resumed under a new id, messages sent while connecting are queued and returned to the box if they cannot be sent, choice buttons work while scrolled up, Stop and dismiss does both, and rows no longer go grey on a quiet day.

## 0.8.19 (2026-11-10)

- Closing a tab leaves its chat running: closing a chat's tab (X, Cmd+W, closing its group) no longer stops the chat, so looking in on a background chat never ends it. Stop on the chat's card still ends it. To have chats stop with their tabs as before, turn on Settings > Terminal > Stop a chat when its tab closes.
- Windows: updating the node no longer sets up its logon task again. Windows Defender could take that for malware and remove the node, leaving the PC offline. An update now only swaps the program.
- Mac: updating the node no longer leaves it stopped when the old one is slow to quit.

## 0.8.18 (2026-11-10)

- Tabs come back at once: when Switchboard opens, your chat tabs reopen right away from what it saved last time, instead of staying away until the server answers. If the server is slow or away, they keep trying and connect when it is back.
- A silent server shows: if the server stops answering while you type, the terminal says so within a few seconds and reconnects by itself. Before, the keys could look dead for up to a minute, as if the app had frozen.
- Typing never waits on output: a chat printing a lot at once no longer holds back what you type into it.
- Screens that fall behind are redrawn: if a terminal ever has output it has not drawn, it draws it and notes it in the log.
- Better freeze notes: the log now says when the window stops drawing while you use it, and what each tab was doing when the app quit, so a quit because it looked frozen leaves its reason behind.

## 0.8.17 (2026-11-09)

- Computer use, asked for in the chat: a chat that has no screen tools can now turn computer use on for itself. Switchboard sets it up for the chat's folder, relaunches the chat with it, and the chat carries on with what it was doing.
- Smoother chats: the terminal no longer redraws every row each time a status dot pulses, so an open chat uses much less of the computer while chats work.
- Far fewer disk writes: the saved copy of the board is written at most once a minute instead of every second (macOS had flagged the app for writing about 2 GB a day).
- Steady cursor on Windows chats: a chat running on a PC no longer flashes its cursor several times a second while Claude works.
- Freeze reports: if the app's window stops responding for 8 seconds or more, it notes it in its log and, on a Mac, saves where it was stuck, so a freeze can be found and fixed.

## 0.8.16 (2026-11-09)

- Model menu: the model picker is now one menu, built from Claude Code's own model catalog, so a new Claude model (Fable 5.2, out today) shows up the day it ships, with a line on what each is for. Older versions you can still pin sit under More models. Same menu in the desktop app, on the iPhone and on Android.

## 0.8.15 (2026-11-06)

- Computer use on Windows: Turn on computer use now works for a chat on a PC too. Switchboard sets up Windows-MCP for the chat's folder (installing it with uv the first time), so Claude can see the screen and click and type in its apps. If the PC's desktop is disconnected (a Remote Desktop window closed without signing out), the chat's details say so.

## 0.8.14 (2026-11-06)

- Turn on computer use works from the chat's details: in 0.8.13 the server stopped it with an error before the chat relaunched.

## 0.8.13 (2026-11-06)

- A chat relaunched with Claude in Chrome keeps it: the chat shows Chrome on, and the next relaunch, from the iPhone or the desktop, carries it over instead of dropping it.
- Computer use for any chat in one click: Turn on computer use in the chat's details on the desktop, or at the top of its flags on the iPhone. On a Mac Switchboard turns it on for the chat's folder and relaunches the chat once Claude is free, and it carries on where it was; a chat open in a terminal gets the command to run there. The first time, macOS asks to allow Accessibility and Screen Recording. On Windows it says why it isn't there.
- Jobs and messages in the bottom right corner move beside the flags drawer and the chat's details instead of hiding under them.
- The desktop app uses less CPU: its breathing dots and spinners draw a few frames a second instead of every frame.

## 0.8.12 (2026-11-02)

- Choosing a folder looks like Finder or File Explorer: places down the side, Back, Forward and Up, a path bar, and the folder's contents. It's how New chat picks its folder and adds folders Claude may also use, on the desktop and on the iPhone.
- A chat's terminal shows its cache on its own, Warm or Cold, apart from whether it needs you, and Start fresh is a click away beside it. It asks first, saying what it does.
- Claude Code's status line in a terminal refreshes every 15 seconds, so its warm or cold stays true while nobody types.
- Quiet keeps the newest 8 chats per computer.
- The iPhone app opens with no chats open after it was closed, instead of reopening old tabs.

## 0.8.11 (2026-10-23)

- Settings > Server shows whether your server and its parts are running, and updates it with one click when a new version is out. It can also restart it, show its logs and copy its details for a bug report.
- Quiet keeps each chat to one line, up to 12 per computer from the last two days. Older ones are in History.
- A button at the end of the tab row lists every open tab, by group.
- A right-click on a pill or tip over a terminal no longer pastes into it, and Windows terminals start reliably.

## 0.8.10 (2026-10-22)

- Copy and paste in a terminal work the same everywhere: selecting text copies it, Ctrl+C copies when text is selected, and Ctrl+V pastes on Windows and Linux.
- A right-click in a terminal copies the selection, or pastes when nothing is selected.
- Terminals on a Windows PC start without waiting for the app to answer the Windows console's first question.

## 0.8.9 (2026-10-22)

- Updates come from Switchboard's GitHub releases, through your server. The app still checks every build's signature and SHA-256 before it installs it.
- A check for updates that fails says why, instead of saying this is the newest version. The update panel has What's new.
- The setup guides open the new documentation site.
- Ctrl+Shift+Q quits on Windows and Linux, where a terminal keeps Ctrl+Q for itself.
- Settings, Notifications says On this computer on Windows and Linux.
- switchboard logs shows the server's logs on a Mac, and removing the helper from a Windows PC also removes its logon task.

## 0.8.8 (2026-10-22)

- Adding a Windows PC works from an ordinary PowerShell window: the node starts at logon without administrator rights.
- Adding a Windows PC again replaces a node that is still running instead of stopping at Access is denied.
- Add a machine downloads the node and agent of this release, not an old one that could not link.

## 0.8.7 (2026-10-22)

- Windows finds the server on the network again: on a PC with several network adapters it asked the wrong one.

## 0.8.6 (2026-10-20)

- The installer's "Eject the disk image" works on the signed download: it used to stop before ejecting.
- The disk image's window has room for Finder's tab bar, so Install Switchboard no longer drops out of sight.
- The Switchboard name shows in its own typeface again; the bundled font was missing most letters.
- The kit keeps the terminal schemes, and its port check ignores connections that are closing.

## 0.8.5 (2026-10-20)

- New chat starts on Auto, and its model list is the models your machines' Claude Code takes, from the server.
- Plainer wording across the app, the setup screens and the messages from the server and the helper. A folder or the local network that macOS hasn't allowed says "macOS permission not granted".

## 0.8.4 (2026-10-15)

- A chat open in a tab is never offered for stopping, even while Switchboard is closed or the Mac is asleep. The app tells the server its tabs; closing the tab is what makes a chat eligible again.

## 0.8.3 (2026-10-15)

- Typing exit (or /exit) in a chat closes its tab. Before, Claude Code's list of background agents opened in the tab instead.
- Stop is honest: a chat the server did not stop (in use again, kept, or Claude Code could not confirm it) says so and stays on the card, instead of a "Stopped" that left it running.
- A background chat whose reply has finished shows as quiet, not Working, even while a server or command it started keeps running. Its subagents or commands still writing keep it Working.

## 0.8.2 (2026-10-15)

- Rename a chat: double-click a terminal tab's title, or pick Rename chat in its right-click menu, and type over the name right in the tab. Enter saves, Escape puts it back.
- Rename from the board too: the pencil in the details (or a double-click on the name), a card's right-click menu, or Rename chat in the palette.
- The name shows everywhere at once: the tab, the board card, the details, the palette, History and the iPhone. It follows the chat through /clear and Start fresh ("Your name (part 2)") and onto copies, and Claude Code's own /resume list shows it too.
- Clear the name and press Enter to go back to the automatic one.
- Many tabs: past a few letters each, tabs shrink to just their dot (the one that shows keeps its title) before the row has to scroll, so a group's chip no longer slides half out of sight, even while you name it.

## 0.8.1 (2026-10-14)

- Naming a tab group: the name field is a light box outlined in the group's color with its dot in front, so the text and the caret show. Before, the field sat on the solid color and its text could not be seen.
- The name field keeps the keyboard until Enter, Escape or a click elsewhere; the terminal no longer takes it away and leaves the field stuck open.

## 0.8.0 (2026-10-14)

- One answer for every view: the server decides each chat's state once, and the board, the terminal tab and strip, the inspector and the iPhone all show that same answer.
- Cold now simply means the cache has run out. "Expensive" (150k or more) is its own badge and can show with any state. When the real cache time is not known the state says so instead of guessing.
- Old information looks old: if a machine has not reported for 45 seconds, its states turn grey with "as of 2m ago". Stalled chats say Stalled, not Working.
- Terminals: every window and the iPhone share one session per chat, so they no longer fight over its size. The screen only changes size when the chat has redrawn for it, which ends the stray letters and doubled rows.
- A dropped terminal link reconnects by itself and keeps what you could see and scroll back to. Redraw (Cmd+Shift+R) repaints a terminal on demand, and a big paste waits instead of closing the terminal.

## 0.7.9 (2026-10-14)

- Notices tray: whatever leaves the bottom-right corner (its X, a click, or its time running out) waits behind "Notices" at the right of the status bar, with a count of what is new. Open it to see each one again, with its button (Resume, Open terminal, Fix).
- Nothing in the tray goes until you say so: Ignore asks "Confirm ignore" first, Clear all asks "Confirm clear all". A setup notice's X sends it to the tray too; ignoring it there is what keeps it away.
- The tray survives a restart.

## 0.7.8 (2026-10-14)

- Stop on a working chat (or closing its tab) gets its own card: "Stops when its work is done", with Stop now and Keep running. The chat's card says "Stops after this work" meanwhile, and once it stops the card offers Resume.
- Start fresh waiting on a busy chat: the card breathes instead of spinning, counts how long it has waited, and has Start now beside Cancel. Calling it off says "Nothing was changed" in grey and goes away by itself, instead of a red "did not finish".
- Confirm stop counts 3, 2, 1 on the second, and Esc or a click anywhere else puts the button back.
- Stopping from the Not in use list: chats still working get their own cards instead of one vague toast; a stopped chat's toast has Resume.
- Toasts: a check, warning or info mark, the same width as the cards above them, a short rise into view, and they stay while the pointer is on them. How long one stays depends on how much it says; the same message twice refreshes the one on screen.
- A job that finishes while Switchboard is in the background posts a notification.

## 0.7.7 (2026-10-14)

- History: the Resume, Start fresh and Open terminal buttons grow taller to fit their notes, so a long note no longer runs into the next button.
- Finished and failed job cards, and notices, have an X in the corner to close them. A failed card shows a warning sign instead of an X, so the only X is the one that closes it.
- Start fresh in a folder Claude Code had not trusted yet (a worktree, say) trusts it and goes again by itself, instead of stopping with "Workspace not trusted".
- Setup check: each machine's agent looks for setup problems that would bite a chat later (MCP servers background chats cannot start, background chats coming up without Claude in Chrome) and the app says so once, with Fix when it can repair it and an X to close it. The Claude folder is trusted quietly.
- Start fresh spins its spiral while the new chat comes up, on the History button and on the job card.
- A chat launched with Claude in Chrome that came up without it (Claude Code 2.1.300 background chats) now says so in its details, and its Chrome chip goes off instead of claiming Chrome.
- A starred tab shows one star, not two under the pointer. Click that star to unstar the tab.
- Chats that stopped while working or waiting no longer show as Needs you, and a terminal chat whose turn is over no longer shows as Working while its background commands run.

## 0.7.6 (2026-10-14)

- Start fresh (or a move) on a chat that is still working now says so: "Claude is replying. It starts once that is done." instead of "Writing the handoff" for minutes on end, with a Cancel button while it waits.
- A window that comes back on screen (uncovered, unminimized, a display waking) gets its frames going again, and after 20 seconds or more away its terminals redraw their screens, as dragging the window's corner did. No more frozen terminals or leftover letters in the status line.

## 0.7.5 (2026-10-13)

- Closing a chat's tab (X, Cmd+W, typing exit, closing its group) stops the chat, so nothing sits running in the background. A chat that is working stops once its reply is done. Resume brings any of them back. Chats set to Keep running, and chats open in a terminal on their own machine, are left alone.

## 0.7.4 (2026-10-13)

- Settings > Library > Set key... on this Mac is a hidden field with Save: the value goes straight into this Mac's keychain. Other machines still show the command to run there.
- Tabs from the last launch reopen once live data is in, not from the saved snapshot, so a chat started since the last save no longer loses its tab.
- The command palette lists chats parked for a day or more below the working ones.
- A terminal tab closes only when you leave (exit, Ctrl-D, Ctrl-Z). If the attach ends on its own, the tab stays with Reconnect instead of vanishing from your saved tabs.

## 0.7.3 (2026-10-13)

- The command palette (Cmd+K, Cmd+T) keeps every row on one line: a long status like "Needs: answer 3 questions..." ends in an ellipsis instead of squeezing the chat's name into a column of letters and spilling over the rows below.
- A machine's name on the board stays on one line in a narrow window ("MacB..." instead of "MacBoo" over "k").

## 0.7.2 (2026-10-13)

- The Working / Warm / Cold strip and the tab's live dot now show for a chat started or restarted from Switchboard and after a /clear: the tab finds its chat by its background id, not only by the chat id it was opened with.

## 0.7.1 (2026-10-13)

- Stars stick to a chat through /clear, Start fresh, Continue and Resume: every new part of a starred chat is starred, and unstarring any part unstars them all.
- A starred tab saved before a /clear or a fresh start reopens on the newest part of that chat when Switchboard launches.

## 0.7.0 (2026-10-13)

- Tab groups, as in Chrome: right-click a tab for Add to new group or Add to group, and the group gets a colored chip and a line under its tabs. Click the chip to fold the group into a count; right-click it to rename, pick a color, ungroup, close its tabs, or move it.
- Drag a tab into a group to join it, out of it to leave, or drag a chip to move the whole group. Starred tabs stay at the left, outside groups.
- Groups are kept on the server, so every Mac shows the same ones (folding is per Mac). Board cards and the command palette show a chat's group color.
- Usage panel: days read as a share of your weekly limit ("Today 4%, 12% of the week so far"), the plan note is gone, and "Costly to go back to" lists the 3 cold chats that would re-read the most. Click one to open it.
- Typing exit (or /exit, quit, /quit) and Enter in a terminal tab now leaves: the tab closes and the chat keeps running in the background.
- Many tabs stay readable: "Updated" in the header makes way first, then machine labels, then titles shrink to a few letters; past that the tab row scrolls sideways and keeps the tab you are on in sight.
- On a narrow window the board scrolls sideways instead of squeezing cards until names break into letters, and card buttons always show their whole label.
- The strip on top of an open terminal is bigger and reads as one line: "Working, for 2m 10s", "Warm, cache good for 4:12", "Cold, next message re-reads 99k" or what Claude needs, with the context as a small meter and "98k of 300k context" on the right.
- Auto tools: a new chat starts with the library tools in Auto, or you pick them by hand (Tools: Auto | Pick). In Settings > Library each tool has Include in Auto and a one-line When to use; a chat's details say "Tools: Auto (5)" or "Tools: 3 picked".
- Usage budget: Settings > Budget sets a daily share of the week (14.3% by default), an optional 5-hour cap and an optional hard limit that blocks new messages and stops working background chats, with Lift for today. The usage panel says "Today 7% of 14% budget", and today's bar and the Week chip turn amber over budget and red at the hard limit.
- The Switchboard logo at the top left opens Settings, and its mark and name line up with Board and History.
- About shows the app's own icon above its name and version, like a Mac About panel.
- A waiting chat's card tags a plain question "Open question" (it said "Reply" next to the Answer... button).

## 0.6.0 (2026-10-13)

- A waiting chat's details open on a "Needs you" card shaped by what Claude asked: option buttons with its assumptions to tick (Choose), a short form with one line per question (Questions), a checklist and Done (Do this, then confirm), or the fix as one button (Fix setup). Send turns it into one message typed into the chat.
- Questions about you are never pre-answered, and Send waits until every question has an answer.
- Board cards for waiting chats show what Claude needs on two lines, the kind of ask, and one button: the recommended option, Done, the fix, or Answer..., which opens the card.
- "Restart this chat with Chrome" relaunches it with --chrome after saying what it costs.
- Terminal tabs look like tabs: a raised tab for the one showing, a thin line between the others, and room for the name, the machine and the close button.
- Star a chat (its tab's menu, the star on a tab under the pointer, or the details sheet): starred tabs sit at the left, come back on launch even when the chat has stopped (with Resume, never on their own), and starred chats come first in the chat list. A star follows the chat into Start fresh and Continue.
- Each terminal tab has a live state dot (working breathes, warm, cold, waiting), and an open chat's terminal has a slim strip on top: Working, "Warm, 4:12 left" counting down each second (amber in the last minute), or Cold and what the next message re-reads. It follows the terminal's own output, so it no longer waits for the next poll.
- Long names, needs and answers wrap or end in an ellipsis instead of running into buttons, and the confirm box, the usage panel and the header fit narrow windows.

## 0.5.1 (2026-10-13)

- The details sheet's top line ends in an ellipsis when a chat's need is long, so its close and pop-out buttons always stay on screen.

## 0.5.0 (2026-10-13)

- Usage: the Week chip opens a bigger panel. This week as a bar per day (hover a bar for its exact number), and tiles for the 5-hour window, this week with its pace, and the last 4 weeks.
- Terminal tabs drag to reorder. The order is saved, and Cmd+1 to 9 follow it. Dragged below the tab row, a tab still opens in its own window.
- A gear at the top right opens Settings. Light and dark are in Settings > Appearance, the View menu and Shift Cmd L.
- About Switchboard is its own window now, with this list and Copy version info.
- Waiting chats say what they need ("Needs: confirm option 1 or 2"), and the details sheet has a Reply button with the suggested answer, typed into the chat's terminal.
- A chat waiting more than a day is parked: a muted pill and "Waiting 2 days". It no longer counts toward the badge or notifications.

## 0.4.0 (2026-10-13)

- Tool Library, a new Settings page: the skills and MCP servers you build, kept on the server and shared by every machine. Import one from a machine, or describe a new one and a chat builds it.
- Each tool shows its versions (any one can be made the latest again), which machines have it, and the keys it needs there. Keys stay in each machine's keychain.
- The New sheet has a Tools row, with the tools that are on for every new chat already checked. A chat's details show its tools, and Attach tool adds more.
- A machine's MCP servers each have Add to library.

## 0.3.1 (2026-10-13)

- Double-clicking a stopped chat resumes it into a terminal tab, on any machine, instead of opening the Claude app.

## 0.3.0 (2026-10-13)

- Terminal tabs you leave open come back the next time Switchboard opens.
- An About page, under Switchboard > About Switchboard.

## 0.2.0 (2026-10-13)

- Board cards show each chat's live state (working, warm or cold) instead of how it was started.
- New names say "part 3" for a chat's third start and "copy" for a resumed copy. A normal new chat has no tag.
