# Changelog

What changed in each version of Switchboard, newest first. The app shows the same notes in
About > What's new. Downloads are on the
[releases page](https://github.com/nicglazkov/switchboard/releases).

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
