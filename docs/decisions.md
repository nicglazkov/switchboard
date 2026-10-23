# Decisions

One line per judgment call, newest last. Older history lives with the private working copy.

- 2026-10-16: The public repository starts with a fresh history; the private working copy is never pushed to it.
- 2026-10-16: Every export to the public repository passes `scripts/public/leakscan.py` with a private term list kept outside the repo, and CI runs the scan on files and history.
- 2026-10-16: License is MIT, held by "Switchboard contributors" so no personal name is in the file. (Replaced on 2026-10-17, see below.)
- 2026-10-16: Bundle ids keep their existing prefix so installed apps, caches and keychain items carry over; it lives only in `branding.toml`.
- 2026-10-16: User-facing text calls `agent/sb.py` "the helper" and the server "the server", introduced as "one computer that stays on"; code names stay as they are.
- 2026-10-16: macOS and Windows CI jobs run only by hand while the repository is private, to save runner minutes, and on every push once it is public.
- 2026-10-16: Example machines in tests and docs are Studio, Laptop, Tower, NAS, Lab and Office; example addresses are 192.0.2.x, with 10.0.0.x and 100.64.0.x only where a test needs a LAN or tailnet shape.
- 2026-10-16: Everything that sends chat text to Claude on its own (ask card summaries, chat names, Auto's machine pick) is a server setting, off for a new server; handoff and thread summaries stay as they are because the user starts them and sees a cost first.
- 2026-10-16: The helper's changes to the user's own Claude setup (the subagent rule in CLAUDE.md, Explore.md, CLAUDE_CODE_AUTO_COMPACT_WINDOW) are server settings too, off for a new server; turning one off takes back only what Switchboard added, tracked in setup.json beside sb.py.
- 2026-10-16: A server that already has state (anything in its state folder, or a fleet.json) turns all six settings on once, recorded as "opt-in-2026-10" in prefs.json, so existing installs keep working as before.
- 2026-10-16: A helper upgraded before the server has sent the settings leaves the three Claude setup changes as they are; an older install's exact copies of them count as Switchboard's, so turning them off later removes them.
- 2026-10-16: The subagent rule is now written between marker comments in CLAUDE.md, so it can be found and removed without touching lines the user wrote; the bare line older installs wrote is still recognized.
- 2026-10-16: With Claude picks off, Auto picks by its existing rules and says so (`"ai": false`) instead of refusing, since the rules already pick well enough to start a task.
- 2026-10-16: The Claude folder suggestion is the home folder on every system; no folder name is special, and Settings lists the folders that exist.
- 2026-10-16: Work started from the home folder still runs in ~/Desktop/Claude when that folder exists (home_standin), because moving it would split existing chats' memory; only the suggestion changed.
- 2026-10-16: The server's CSP no longer allows Google Fonts; no page loads them.
- 2026-10-16: `native/rustfmt.toml` is `max_width = 120` with `use_small_heuristics = "Max"`. Measured `cargo fmt` changed lines: defaults 24,824, 120 alone 18,639, 120 + Max 10,585. Wider limits keep lowering it (150: 6,138, 200: 4,918) with no sweet spot, so 120, the width most of the code already keeps to, wins.
- 2026-10-16: Four functions keep 8 arguments under `#[allow(clippy::too_many_arguments)]` with a reason each (a job constructor, two GPUI row builders, a probe tool); an argument struct would only repeat the fields.
- 2026-10-16: `switchboard-hub` waits for SIGTERM only on Unix, so the workspace builds on the Windows CI runner; on Windows it stops on Ctrl+C as before.
- 2026-10-16: The update list is signed with ed25519-dalek 2.2.0 (added, pinned, std only), because neither it nor ring was in Cargo.lock; base64 0.22.1 was already there.
- 2026-10-16: Releases are signed with OpenSSL 3 on the computer that publishes, so the private key never goes to the publish host; the publish host keeps latest.json byte for byte and only checks it against the files.
- 2026-10-16: The server's GET /api/app passes latest.json on as text with its signature (`feed`) and keeps the parsed `latest` for apps from before signing, so they can still update to a signed release.
- 2026-10-16: A newer offer must not go back in semantic version and must have a later build id; a rebuild of the same version is offered, an older version with a later build is not.
- 2026-10-16: A new build counts as healthy after 20 seconds of running or a clean quit; one that starts a second time without that gets the old build put back, and its build is not offered again.
- 2026-10-16: Before the swap the new build is run once with --version, so a build that can't start on this computer never replaces the old one.
- 2026-10-16: SB_UPDATE_FEED must be http://, since the app's HTTP client has no TLS; the signature is what makes the list trustworthy.
- 2026-10-16: Updates have their own Settings page (Settings > Updates) with the off notice next to Check now and a link to how a build gets updates, rather than a line in About.
- 2026-10-16: Desktop controls: a switch for every on/off setting, `seg_btn` for 2 to 4 short choices, a dropdown (`ui::choice`) for 5 or more and for machine filters past 4 machines. The New sheet's machine row stays a wrapping picker because it shows each machine's online lamp, and the terminal scheme grid stays a swatch picker because the choice is visual.
- 2026-10-16: Palette section heads are 34 px tall, the same as its rows, so the palette can be a `uniform_list` that draws only what is on screen.
- 2026-10-16: The details panel docks beside the board when the five columns still fit next to it, and floats over the board when they do not. Its docked width (320 to 640 px) is a local app setting (`inspector_w`), not a server pref, since it depends on the screen.
- 2026-10-16: The "Picked by rules" note shows when the `ai_auto_machine` pref is off, because the New sheet's live preview always asks for a rules-only pick and so never says which picker the real run will use.
- 2026-10-16: The menu bar and file pickers stay native: macOS users expect the system menu bar (and its shortcuts and Services), and the system pickers keep sandbox access grants and accessibility. Menus, dialogs, toasts and the in-window title row are drawn by the app.
- 2026-10-16: On macOS the window keeps the native traffic lights over a transparent title bar (`appears_transparent`, as Zed does); GPUI supports it cleanly. Linux keeps server-side decorations.
- 2026-10-16: iPhone push setup shows as a not-set-up card on Settings > Notifications and as a short line in Settings > AI and your Claude setup, both naming `~/.config/switchboard/apns.json` and linking the install guide.
- 2026-10-16: Tab and Shift+Tab move focus between buttons only on the board page with no text field active and in Settings, so terminals and text fields keep their own Tab.
- 2026-10-16: The app's logger is a small one written in sb-desktop (src/logs.rs), not env_logger or a log-file crate: none is in Cargo.lock, and rotation plus env_logger-style filtering is under 200 lines.
- 2026-10-16: The Linux log folder is `$XDG_STATE_HOME/switchboard/logs` (lowercase, like the old panic.log place) and the Windows one `%LOCALAPPDATA%\Switchboard\logs`; the old panic.log files are left where they are.
- 2026-10-16: Crash files are `crash-<local time>.txt` and the 10 newest are kept, so a crash loop cannot fill the disk.
- 2026-10-16: Copy diagnostics leaves out the server address that Copy version info has, since diagnostics are meant to be pasted into public bug reports.
- 2026-10-16: Release builds keep line tables only (`debug = "line-tables-only"`): enough for function, file and line, and much faster to link with fat LTO than full debug info.
- 2026-10-16: The crash note prints the backtrace in full form with raw addresses plus the executable's load address, because a stripped build's frames have no names and ASLR moves the base each run.
- 2026-10-16: `SB_PANIC_TEST=1` makes the app crash right after it starts, to check a build against its symbols.
- 2026-10-16: License notices come from cargo-about over all four shipping targets. MPL-2.0 (cbindgen, dwrote, option-ext) and bzip2-1.0.6 (libbz2-rs-sys) are accepted along with the permissive licenses; nothing is GPL, LGPL, AGPL, SSPL or unknown.
- 2026-10-16: About > Open source licenses opens the notices file shipped beside the app, and falls back to a built-in copy written to the temp folder, so a dev build or the bare Linux binary can still show it. No in-app viewer: there is no shared scrollbar yet.
- 2026-10-16: switchboard-hub and switchboard-node get no log rotation: they only print to stderr. The node's LaunchAgent log (~/Library/Logs/switchboard-node.log) still grows without a limit; that is launchd's file, left for later.
- 2026-10-16: sb-core/build.rs also looks for branding.toml inside native/, and build-linux.sh and build-windows.sh now send it, because the build hosts only get native/ and the Mac build stopped on the missing file.
- 2026-10-16: leakscan skips the private term list, emails, em dashes and filler words in THIRD-PARTY-NOTICES.txt only, because other projects' license texts must be shipped word for word; addresses, home folders and keys are still checked there.
- 2026-10-16: The Mac installer is its own small GPUI crate (`sb-install`) that copies the app's color tokens from `sb-desktop/src/theme.rs` with a comment pointing there, instead of depending on `sb-desktop`, so it builds in seconds and stays small.
- 2026-10-16: The DMG's Finder layout is written into `.DS_Store` with the `ds_store` and `mac_alias` Python packages in a throwaway venv on the build Mac, because Finder scripting needs a logged-in user and permission prompts.
- 2026-10-16: "Open at login" is a LaunchAgent plist running `open -a` on the app, written but not loaded at install (loading it would start the app); it takes effect at the next login and needs no helper app or admin rights.
- 2026-10-16: "Add to the Dock" edits `com.apple.dock` with `defaults export/import` and restarts the Dock, and only when HOME is the account's real home folder, so a test with a throwaway HOME can never touch the real Dock or login items.
- 2026-10-16: The installer clears the quarantine flag on the copy it makes, so the first launch runs from its folder instead of a translocated read-only copy; the user already chose to run the installer from the same download.
- 2026-10-16: Installing the same version again is allowed and called a reinstall (it repairs a damaged copy); only an older version over a newer one is refused.
- 2026-10-16: The options screen shows whenever there is no first-run record, even on an update, so people who installed before the installer existed get asked once too.
- 2026-10-16: The DMG background is light in both appearances; Finder draws icon labels in black on it either way, and a dark image would make them unreadable.
- 2026-10-16: `branding.toml [dmg]` moved to a 660 by 460 window with the app and Applications on one row and a new `installer_x`/`installer_y` slot under them; nothing else read the old values.
- 2026-10-16: The uninstaller removes only the app's own files from `~/Library/Logs/Switchboard` (`panic.log`, `Switchboard.log` and its 5 rotations, `crash-*.txt`), because the server's LaunchAgents log to the same folder and the uninstaller never touches the server.
- 2026-10-16: The tour uses real frames of the app against the fixture server, rendered by the Linux shot scripts and stored in `native/packaging/tour/`, so they are reproducible and show no one's real chats.
- 2026-10-17: Windows exes link the C runtime statically (`native/.cargo/config.toml`, `+crt-static` for the MSVC targets), so the installer never has to check for or fetch the Visual C++ Redistributable; `windows-build.ps1` fails a build whose exes import `vcruntime140.dll` or `msvcp140.dll`.
- 2026-10-17: The Windows download is one `Switchboard-Setup-<version>.exe`: the installer exe with the app's files appended (`sb-install/src/pack.rs`), not an MSI or a self-extracting zip, so the same GPUI screens and flow as the Mac run with no other tool. The reader finds the files before an Authenticode signature, so signing later needs no change.
- 2026-10-17: The Windows install is per user with no admin rights: `%LOCALAPPDATA%\Programs\Switchboard` and HKCU only, the same place and registry hive per-user installers of other apps use, so the Mac rule "never ask for an admin password" holds.
- 2026-10-17: The uninstaller stays in the app's folder as `Uninstall Switchboard.exe` (the plain installer, nothing attached). Opening it means uninstalling that copy. To remove its own folder it moves its exe to the temp folder first and leaves a hidden `cmd` (CREATE_NO_WINDOW) that waits two seconds and deletes it.
- 2026-10-17: A running Switchboard is found by its exe's path inside the install folder and asked to quit by posting WM_CLOSE to its windows, the same as clicking close; TerminateProcess only after the user agrees (or `--force-quit`).
- 2026-10-17: The installer reads the installed and the new version from the exe's version resource and never runs either exe, so checking a copy can never open the app's window.
- 2026-10-17: Windows offers a desktop shortcut and "Open at login" (an HKCU Run value) as its two choices, both off; the Start menu shortcut and the Settings > Apps entry are always made, because Windows has no other way to find or uninstall a per-user app. They are rewritten on every update, the choices only on the first install.
- 2026-10-17: `SB_INSTALL_TEST_ROOT` and `SB_INSTALL_TEST_REG` move every folder and registry key the Windows backend touches into a throwaway folder and a `HKCU\Software\SwitchboardTest-*` key. They must be set together and the key must have that prefix, so a test can never write the real Uninstall or Run keys by mistake.
- 2026-10-17: The graphics check creates a Direct3D 11 device the way GPUI does. With only Windows' basic driver the welcome screen says the app will draw slowly; with none the installer shows a message box saying which driver to install, and `--quiet` still works.
- 2026-10-17: `sb-desktop` calls `set_app_identity` on Windows with the app's desktop id, because GPUI only shows Windows notifications for an app with an AppUserModelID; the Start menu shortcut carries the same id.
- 2026-10-17: The uninstaller removes only the app's own files from `%LOCALAPPDATA%\Switchboard\logs`, as on the Mac, because the Windows server writes `server.log` and `downloads.log` to the same folder; the folder goes only when it is empty.
- 2026-10-17: The hub now waits up to two seconds for a node's answering close before dropping the link socket, because Windows reset the connection before the node read why it was closed (the unpair test failed 2 runs in 6 there).
- 2026-10-17: `sb-hub/tests/relay.rs` runs on Unix only: its fake ssh is a shell script, which Windows cannot start.
- 2026-10-17: The installer is a window program on Windows (no console flashes up when it is opened); `--quiet`, `--help` and `--version` attach to the console they were started from. PowerShell waits for it and sets `$LASTEXITCODE` only when its output is piped or captured, which the docs show.
- 2026-10-17: The installer's Windows icon is the app icon with an accent download badge, drawn by `installer_art.py` from `branding.toml`, with a heavier arrow at 16 to 32 px.
- 2026-10-17: Uninstall labels changed for both systems: shortcuts and the Settings > Apps entry are their own lines, and "Caches and saved window state" became "Caches, logs and saved window state", since logs were already removed.
- 2026-10-17: Uninstalling deletes first-run.json even when settings are kept, because the login item, Dock icon and shortcuts it recorded are removed; a later install asks the choices again, still once.
- 2026-10-17: Public commits carry no Co-Authored-By line from now on; the two earlier ones keep theirs (history is not rewritten).
- 2026-10-17: Mac notarizing can use a saved notarytool profile (SB_NOTARY_PROFILE) instead of an API key's ids and path, so the build host's settings hold only a profile name; the signing identity is given by certificate hash so the right one of two same-named Developer IDs is used.
- 2026-10-17: The license is GNU AGPL-3.0-only instead of MIT, so no one can sell or host a closed copy; every dependency (MIT, Apache-2.0, MPL-2.0, BSD, Zlib, OFL) can be used in an AGPL program.
- 2026-10-17: Signed Mac builds run in the build Mac's logged-in session (started with `open`), because an ssh session can't use the login keychain; this needs no extra keychain or a stored password.
- 2026-10-17: The server's addresses ride in the existing `/api/summary` reply (and the summary inside `/api/sessions`) as `addresses`, so the app learns them from the probe and the poll it already makes, with no new request.
- 2026-10-17: The saved list stays `servers` as plain strings, with what the app knows about each address in a separate `server_notes` map, so a settings file from this build still opens in an older one.
- 2026-10-17: The server lists only addresses its own Host check would accept, and only those it listens on when `bind` names specific ones, so the app is never told an address that would refuse it.
- 2026-10-17: The `.local` name is listed whenever the server has a LAN address, whether or not Bonjour announcing works there; a name that does not resolve just fails its probe.
- 2026-10-17: The app keeps 12 addresses at most; typed ones never count against being dropped, so a user who typed more keeps them all.
- 2026-10-17: A learned address is dropped only after failing for 21 days while another address worked and the server stopped listing it, so a laptop that was off for weeks, or a LAN address the server still has, is never dropped.
- 2026-10-17: Removing a learned address hides it from learning until the user types it, or it would come back on the next poll.
- 2026-10-17: The address that worked last is tried first, even when a LAN one also answers; the race already gives the LAN only a short wait, and the order flips as soon as another one is used.
- 2026-10-17: A probe that answered but lost the race clears that address's failure but is not marked as used, so two working addresses do not trade places every poll.
- 2026-10-17: With `SWITCHBOARD_URL` set the app does not touch the saved list unless `SB_SETTINGS` gives the run its own settings file, so pointing a dev build at a test server never adds its addresses to the user's settings.
- 2026-10-17: The upgrade seed marks the last server as learned (it was not typed) with the cache's fetch time as when it last worked, so it is tried first and can still go stale later.
- 2026-10-17: Every terminal tab is one fixed width (240 px) whatever its title, hover buttons or the number of tabs; a tab that grew under the pointer could slide out from under it and flicker. The title shortens with an ellipsis, the machine name never does, and many tabs scroll sideways.
- 2026-10-19: Before a chat starts in a folder, the helper gives that folder its own trust entry when the nearest folder above it with an answer was trusted (a worktree inside a trusted project), because a background launch does not go by a parent's answer; a folder or parent that said no is never changed.
- 2026-10-19: A local terminal reuses the chat terminal's tab and view, with the PTY behind the same channels the hub's WebSocket uses, instead of a second terminal view; the parts that only mean something for a chat (flags, Continue on, claude attach, uploads, typing "exit" to leave) are off for it.
- 2026-10-19: Cmd+T (Ctrl+Shift+T on Windows and Linux) now opens a shell on this computer, and finding a chat to open in a terminal moved to Cmd+O (Ctrl+Shift+O), which nothing used and reads as "open"; the menus, the palette and the "+" menu show the new keys.
- 2026-10-19: "+" keeps opening the chat menu it always did, since the app is about Claude chats; the chevron joined to it opens the same menu with the shells on this computer first, so a terminal is one click further, not in the way.
- 2026-10-19: Local terminals are not saved with the tabs for the next launch: a shell's state (its folder, what ran in it) cannot come back, and an empty shell reopening by itself would be noise.
- 2026-10-19: A shell that exits with code 0 closes its tab, as in any terminal; another code keeps the tab with the last output and Restart.
- 2026-10-19: Closing a local terminal asks first only when the PTY's foreground process is not the shell (an editor, a build); on Windows ConPTY cannot tell, so it closes without asking there.
- 2026-10-19: On Windows and Linux, Ctrl+O and Ctrl+B in a terminal go to the program (readline uses both); the app takes them with Shift there, and Ctrl+Shift+B now opens History too.
- 2026-10-19: The tab row's arrows move it one tab width (245 px) per click with a short ease, repeat every 170 ms after a 400 ms hold, are dimmed at their end and hidden when nothing overflows; clicking a tab, even the one showing, brings it fully into sight again.
- 2026-10-19: On the iPhone the open chat fills the screen and the board is a sidebar from the left edge, as in Claude's app; a tap on a chat opens it at once, and its details moved to the row's swipe and long press.
- 2026-10-19: iPhone tabs: at most 8 (opening a ninth closes the one used longest ago), and the 4 used last stay connected while the app is in front; the rest hang up and keep their screen.
- 2026-10-19: The iPhone's tab strip shows only with two or more tabs, so one chat gets the whole height.
- 2026-10-19: A sideways swipe on the chat's title switches tabs; the terminal itself takes no sideways swipe, because it scrolls, selects and sends wheel events.
- 2026-10-19: The sidebar closes by a tap or a swipe on the strip of chat beside it, or its button, not by a swipe on the list, because rows keep their swipe actions.
- 2026-10-19: The edge swipe does nothing while a sheet is up; the sheet's own swipe wins.
- 2026-10-19: A notification or link to a chat (route `.session`) opens it as a tab; its details sheet only when the list no longer has it.
- 2026-10-19: The phone keeps each tab's last screen (the visible rows, not scrollback) in Caches, at most 40 chats and 512 KB each, so a tab shows it at once while it connects.
- 2026-10-19: The phone fetches terminal tickets ahead for the 3 chats most likely to be opened (starred, needs you, working, warm) when it connects and when the sidebar opens; a ticket is used within 40 s or dropped.
- 2026-10-19: With no tab open at launch, the sidebar opens by itself.
- 2026-10-19: Opening a chat from a sheet no longer waits 420 ms for the sheet to go: tabs sit under sheets, so the sheet closes and the tab is already there.
- 2026-10-19: A tab kept connected behind the front one never takes focus, so it never resizes the chat on another screen; only the tab in front does.
- 2026-10-19: A message typed before the tab is live waits on the tab and goes in when it connects, with Edit to take it back.
- 2026-10-19: The terminal stays at the bottom unless a finger moved it, so a reset or a new size from the hub no longer shows "1 line up".
- 2026-10-19: The iPhone status bar follows what is behind it: while a chat fills the screen the window takes the terminal's light or dark (only the status bar, menus and keyboard follow it), with the sidebar or no chat it is the app's own, and sheets always ask for the app's own look.
- 2026-10-19: The key row scrolls with an 18 pt fade on the side that has more keys, and the compose and key rows share a 10 pt side margin.
- 2026-10-19: An empty terminal (connecting, not running, cannot connect) shows no cursor until something is drawn.
- 2026-10-19: Notarizing names the keychain that holds the notarytool profile (login, or SB_NOTARY_KEYCHAIN), because notarytool otherwise reads only the default keychain, which other build tools on the same Mac can change.
- 2026-10-19: On the iPhone, opening a chat that is not running brings it back in place behind its tab (the plain resume of the same chat), followed in the tab instead of a job sheet, with no Resume button or progress screen; the title says "starting" and a message typed meanwhile waits. The desktop app still asks first with Resume and open.
- 2026-10-19: The phone tells a home-folder chat ("/Users/you", "/home/you", "/root", "C:\Users\you") from the list and offers Start fresh instead of trying a resume the machine would refuse; any other refusal comes back as one inline line with Try Again and Start Fresh.
- 2026-10-19: Resume from a chat's details or row with "Open in a Terminal When Ready" on now opens its tab the same way instead of the job sheet; a resume that stays in the list (Stopped today) still follows the job.
- 2026-10-19: The server asks the machine about a job every second for its first 20 s, then every 3 s as before, so an app waiting to open a resumed chat does not also wait out the slower poll.
- 2026-10-19: The tab in front when the iPhone app launches or comes back to the foreground counts as opened, so its stopped chat comes back by itself with no Resume button (a plain resume with no message costs no tokens); tabs behind it still wait until switched to, a failed one keeps Try Again instead of retrying on every return, and closing the front tab counts as switching to the one that takes its place.
- 2026-10-19: Chats and the file browser reach the whole home folder, not just the Claude folder, unless Settings turns "Anywhere in home" off for a computer. The server could already move the Claude folder anywhere in home, so an app gains nothing it could not do before.
- 2026-10-19: Folders outside home (another drive, /Volumes, D:\) are added only at the computer itself (`sb.py places add`). The server's poll can turn roaming off but can never add a place, so a phone can never widen what a computer shares.
- 2026-10-19: Apps name folders as `~/...` or `@place/...`, never as full paths, so the helper checks every folder the same way and an old app that only knows root-relative folders keeps working.
- 2026-10-19: Dot folders, ~/Library and AppData stay private even with roaming on (keys, tokens, mail, browser data); iCloud Drive is the one folder let out of ~/Library.
- 2026-10-19: Outside the Claude folder Claude starts only in a folder it was already let into, or after the user says yes in the app (409 need: trust, then trust: true). Inside the Claude folder nothing changes and no question is asked. A no from before is asked again, never overridden silently.
- 2026-10-19: A folder macOS keeps the helper out of (Files and Folders) shows as blocked with the steps to allow it, instead of as an empty folder.
- 2026-10-19: Names of existing files and folders may use any characters (Résumé & notes, com~apple~CloudDocs) in the browser; a folder made from an app still takes the plain-name rule.

## 2026-10-19: one push per held message, and a setting for each kind

"Old session held" pushes kept arriving with no clear source. The server log showed most came on Oct 11 and 12, from task
notifications held before injected prompts were let through; since then, every hold was a real typed message, but
a held chat could push again every two minutes as more messages went in, and the server never forgot a hold. Now
a held chat pushes once until it is answered, then not again for 30 minutes; a hold lapses when the chat moves on
or after six hours; agent messages with a plain line before the tag and subagent prompts are never held; pushes
carry a collapse id per chat and a thread per kind; and each kind can be turned off. Quiet hours were left out:
iOS Focus already does that, per device and better.

## 2026-10-19: the daily budget gives way near the reset

Chats were told the user was over a 14% daily budget with 45% of the week left and the reset under a day away,
usage that could never be spent in time. The daily budget and the hard daily limit now count as reached only past
the larger of the setting and the room left (what was left at the start of today, shared over the days left). The
5-hour cap is unchanged: it guards a different limit. The hard limit gives way too, since stopping chats to save
usage that will expire unused protects nothing.


## 2026-10-19: files, history and first run, as built

- Trust is asked up front (a folder check when a folder is picked) and again if the server answers 409 at start.
  The words: "Let Claude work in <folder>?", what yes allows, and a note when it was refused before.
- On the Mac a blocked place reads "Locked", an unplugged one "Missing"; `~` reads "Home". Roam off and a missing
  place are told apart from the helper's own error text.
- `/api/prefs` keeps `notify` as the stored `{kind: bool}` and lists the kinds with their words as `notify_kinds`,
  so the list never hides the stored values.
- The iPhone keeps its existing Title Case labels; new text is sentence case.
- First run on the Mac searches for 3 seconds, connects by itself when exactly one server answers, and lets the
  user pick among several. The resolver's own silent Bonjour pick is off in the desktop app.
- "Run the server on this computer": a laptop is a warning that needs "Use this laptop anyway", never a stop;
  another server on the network is a warning; one already running here is a stop with "Connect to it". Power
  settings are never changed; the Energy pane is opened instead. Python counts only when it is already usable, so
  Apple's command line tools prompt never appears by surprise.
- The terminal installer keeps its piped output byte for byte (the Mac app reads its `== ` lines) and only dresses
  up a real terminal. The one-line install needs a published server kit; no address is made up until there is one.
- No machine addresses, user names or home paths live in the repo. Scripts read them from the hosts file
  (`native/scripts/hosts.sh`): `SB_HELPER_HOSTS` lists the machines that run the helper, `SB_LAB_BIND` the lab's
  addresses, and `SB_SERVER_TS_URL` the server on Tailscale. The Android app gets its server addresses at build
  time and generates its cleartext network config from them. The design lab works out the live app from its own
  address. Fixtures use documentation addresses (192.0.2.x, 100.64.0.x).
