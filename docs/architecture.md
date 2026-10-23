# How the pieces fit

Switchboard is four programs. This page says what each one does and how they talk, so you know where to look when
you change something.

```
  app (sb-desktop)  --HTTP :8130-->  server (server/)   --ssh, or through the hub-->  helper (agent/sb.py)
        |                                 |                                               ^
        +--------WebSocket :8132--------> hub (sb-hub) <----WebSocket, outgoing---- switchboard-node
```

## The server: `server/`

Python, standard library only, one process. It runs on the computer that stays on.

- Every few seconds it asks each computer's helper what is running and keeps the answer as one snapshot.
- It serves the HTTP API on port 8130 (`/api/sessions`, `/api/stream` for live updates, and the actions: start,
  resume, stop, continue on another computer). The apps only ever talk to this API and the hub.
- It keeps settings in `~/.config/switchboard/` and history in `~/.local/share/switchboard/`.
- Port 8131 serves the files a new computer downloads when it is added.

Module map: `server.py` (HTTP routes, the poller), `fleet.py` (calls to the computers), `nodes.py` (computers linked
through the hub), `term.py` (terminal tickets), `features.py` (cleanup, threads, picking a computer, dispatch),
`budget.py` (the usage budget), `library.py` (tools shared between computers), `flags.py`, `notes.py`,
`bonjour.py` (finding the server on the local network), `addresses.py` (the addresses it tells the apps to keep).

## The helper: `agent/sb.py`

One Python file, copied to `~/.claude/switchboard/sb.py` on each computer. It is Claude Code's status line and
hooks there, so it knows each chat's state as it changes, and it answers the server's requests (list chats, start
one, stop one). The server reaches it in one of two ways:

- **ssh**, with a key that can only run `sb.py` (a forced command, no shell);
- **a link through the hub**, for computers that have no ssh server or sit behind another router.

### Places: where chats and the file browser may go

Each computer has a Claude folder, but chats are not kept in it. An app names a folder in one of three ways, and
never by a full path:

| Written as | Means | Who can open it |
|---|---|---|
| `site/app` | under the Claude folder | always |
| `~/Documents/site` | anywhere in the home folder | unless Settings turned "Anywhere in home" off for that computer |
| `@Data/work` | under a place added on that computer | only after someone at the computer ran `sb.py places add D:\Data --name Data` |

- **Private folders.** Dot folders (`.ssh`, `.aws`, `.config`, `.claude`), `~/Library` and `AppData` are never
  listed or opened. The one exception is iCloud Drive (`~/Library/Mobile Documents/com~apple~CloudDocs`). A file
  that looks like a key or a secret is listed but never read.
- **Folders outside home.** These come only from a place added at the computer, never from the server or an app.
  The server can already move the Claude folder anywhere in home, so "Anywhere in home" gives an app nothing new.
- **Trust.** Inside the Claude folder, chats start without a question, as before. Elsewhere Claude goes only into
  a folder it was already let into. For any other folder, `/api/new` and `/api/continue` answer 409 with
  `need: "trust"` and the folder. The app then shows the folder and sends `trust: true` once the user says yes,
  which writes the same answer that accepting Claude's own prompt would. A folder the user once said no to is
  asked again and never opened silently.
- **macOS privacy.** Desktop, Documents, Downloads and iCloud Drive are behind macOS's Files and Folders switches.
  A folder the helper is not let into comes back as `blocked: "privacy"` with a hint, not as an empty list.
  `privacy_settings` opens that pane on the Mac's own screen.

Routes:
- `GET /api/places/<m>`: the sidebar.
- `GET /api/files/<m>?folder=`: one folder, or one file with `&name=`.
- `GET /api/folder_check/<m>?folder=`: answers `yes`, `ask` or `no`.
- `POST /api/machines {op: "roam"}`: turns "Anywhere in home" on or off.
- `POST /api/privacy_settings {machine}`: opens the Files and Folders pane on that Mac's screen.

### Notifications

The server sends four kinds of iPhone push, and each one can be turned off in Settings (`notify` in
`/api/prefs`):

| Kind | When | Grouping |
|---|---|---|
| `held` | The guard held a message typed into a big chat that went cold | One per held message; a newer one replaces it |
| `jobs` | Something started from an app finished or failed while no app showed it | Grouped as tasks |
| `cleanup` | Chats that do nothing are taking memory | At most once a day |
| `budget` | The daily budget stopped new chats | Once a day |

Only a prompt typed by a person is held. Task notifications, agent messages (with or without the plain line
"Another Claude session sent a message:" before them), loop wake-ups and prompts from a subagent (`agent_id`
in the hook input) always go through, so no push is ever sent for them. More messages typed into a chat that
is already held send nothing new; after an answer, a new hold of the same chat pushes again only after 30
minutes. A chat stops counting as held when a prompt goes through or Claude finishes a turn there, and after
six hours in any case. Pushes nobody could receive within a day are dropped, not shown late.

The Mac app notifies when a chat starts needing you, once per waiting spell: a chat that drops out of
"needs you" for a poll and comes back is not notified again.

## The hub: `native/crates/sb-hub`

Rust, runs next to the server on port 8132. It carries terminals: the app asks the server for a ticket
(`POST /api/term`), then opens a WebSocket to the hub with it, and the hub connects that to the chat's terminal on
its computer. It also holds the links from `switchboard-node` and the pairing codes for adding a computer.

## The node: `native/crates/sb-node`

A small Rust program on computers added with a code. It keeps one outgoing WebSocket open to the hub, so the
service can reach that computer without ssh.

## The app: `native/crates/sb-desktop` and `sb-core`

`sb-core` has the models, the rules for which column a chat belongs in, the HTTP client and the snapshot cache. It
has no UI and most of the tests. `sb-desktop` is the app, built with GPUI.

The app never waits on the network to draw: it shows the last snapshot it saved, then replaces it when the
service answers, and refreshes in the background.

### How the app finds its server

Nothing is built in. In order, the app uses:

1. `SWITCHBOARD_URL`, when set (for one run; the saved list is left alone unless `SB_SETTINGS` names a settings
   file of its own);
2. the saved addresses (`servers` in the app's settings.json), the one that worked last first;
3. a server found by the empty board's search, or an address the user types there.

All candidates are probed at once with `GET /api/summary` and the earliest in the list that answers wins; a later
one only gets a short wait, so a LAN address hanging away from home costs nothing (`sb-core/src/net.rs`).

**The empty board's search.** With nothing saved (and nothing in `SWITCHBOARD_URL`), the board says "Searching for
local servers" and runs a Bonjour search for `_switchboard._tcp` for three seconds (`sb-core/src/discover.rs`; the
server announces itself in `server/bonjour.py`). Each server is listed as it answers, by the host in its instance
name. At the same time the app asks `http://127.0.0.1:8130/api/summary`, and the addresses that summary lists mark
which entry is this computer's own server, so it is not shown twice. When the search ends:

- one server: the app connects to it by itself;
- several: the user picks one;
- none: "No Switchboard server on this network", with "Run the server on this computer" and "Type an address";
- the search could not run (on a Mac, Local Network access turned off): the same two choices, with the reason.

The state is plain code in `sb-core/src/firstrun.rs`; the screens are `sb-desktop/src/firstrun.rs`. The resolver's
own Bonjour step (`Resolver::set_discovery`) is off in the desktop app, so it never picks one of several servers by
itself; the board's search decides. "Search again" is also on the screen for a saved address that does not answer.
The Mac app lists `_switchboard._tcp` in `NSBonjourServices`, which macOS needs before it lets the app browse.

**Running the server here.** "Run the server on this computer" opens a guided sheet (`sb-desktop/src/serversetup.rs`;
the checks and words are plain code in `sb-core/src/hostsetup.rs`). It checks that this is not a laptop (a built-in
battery from `pmset -g batt` or a laptop model name on a Mac, `/sys/class/power_supply` and the chassis type on Linux,
`GetSystemPowerStatus` on Windows; a laptop needs an explicit "use it anyway"), that Python 3.9 or newer is there,
that ports 8130 to 8132 are free, and whether the search found another server (a warning, not a stop). Then it runs
the installer from the app's server kit as the user, never with sudo:
`install-server.sh --yes --no-setup`, plus `--hub-binary` when the kit has a hub or `--no-hub` when it does not, with
stdout piped. Each `== <step>` line the installer prints moves the progress list on; a failure shows its last lines
and "Try again". When it is done the app saves `http://localhost:8130`, and the last page says how to keep the
computer awake (it opens the Energy or Battery settings and never changes them itself) and how phones and other
machines find it.

The server kit is the server, `agent/sb.py`, the installer and, when the build has them, the hub and the node
binary, in the repository's own layout so `install-server.sh` finds them two folders up from itself
(`native/scripts/server-kit.sh`). The Mac app carries it in `Contents/Resources/server-kit` (built by
`scripts/remote/mac-build.sh`, with a universal hub), the Linux tar.gz beside the program, and `SB_SERVER_KIT` points
at one for a test. Windows has no kit yet: the sheet says so and links to the setup guide.

**Remembering.** The server lists its own addresses in its summary (`server/addresses.py`): its LAN IPv4 addresses,
`<host>.local`, and its Tailscale IPv4 and MagicDNS name when Tailscale is up, each with its kind and the port.
Loopback, link-local, container and bridge interfaces and anything its allow list (`server.json`'s `allow`) would
refuse are left out. The list is worked out on a thread and kept for five minutes, so a slow `tailscale` never holds
up a request. After every good refresh the app merges it into the saved list (`sb-core/src/servers.rs`):

- duplicates are dropped; addresses the user typed are never moved or dropped by the app;
- learned ones are added at the end and marked as learned in `server_notes`, beside `servers` (which stays a plain
  list, so an older build still reads it);
- the list holds 12 at most, dropping learned ones first (failing longest, then used longest ago);
- a learned address that has failed for three weeks while another one worked, and that the server stopped listing,
  is dropped;
- a learned address the user removes in Settings is not learned again unless they type it.

The address in use is noted as the last one that worked, and probes that got no answer are noted as failing.
Times are written at most once an hour, so a poll does not rewrite the settings file.

**Upgrading.** Builds that had the server's address built in saved no list. On the first start without one, the
app seeds it from the snapshot cache: the server that answered last, and the addresses that reply listed if the
server was already updated. So the app connects at once after the update, at home or away.

### Local terminals

A tab can also hold a shell on the computer the app runs on. It never goes through the server or the hub and it is
never on the board, only in the tab row, and it is not saved with the tabs for the next launch.

- **Profiles.** `sb-core/src/term/shells.rs` lists the shells, as Windows Terminal does. On macOS and Linux that is
  `$SHELL` first, then `/etc/shells` (zsh, bash, fish, nu and the rest; sh, dash and the like only when they are
  `$SHELL`), one per name. On Windows it is PowerShell 7 if installed, Windows PowerShell, Command Prompt, Git Bash
  if found, and one per WSL distro from `wsl.exe -l -q` (which prints UTF-16). The rules are plain functions over
  those facts, with unit tests; `detect` gathers them once, on a thread at launch. Settings > Terminal picks the
  default, saved as `term_profile` in settings.json; the first profile is the default until then.
- **The PTY.** `sb-desktop/src/term/local.rs` starts the profile in a PTY (portable-pty, the version the hub uses)
  in the home folder, as a login shell on a Mac, with `TERM=xterm-256color`, `COLORTERM=truecolor` and `LANG` when
  it is missing. It speaks through the same channels as the hub's WebSocket (`term::ws::Conn`), so the tab draws it
  with the same code: a reader thread sends the output, a writer thread takes the keys and sizes, and a waiter sends
  the exit code and then a normal close.
- **The tab.** The title is what the shell sets (OSC 0 or 2), else the profile's name, with "This Mac" or "This PC"
  where a chat's tab shows its machine. A clean exit closes the tab; an error keeps the last output with Restart.
  Closing the tab ends the shell, and asks first when another program is in front (the PTY's foreground process
  group is not the shell's; Windows cannot tell, so it never asks). A dropped file types its path at the cursor.
- **Opening one.** Cmd+T (Ctrl+Shift+T on Windows and Linux) opens the default shell. The command palette (Cmd+K)
  lists a "New terminal" row per shell first, so Cmd+K then Enter does the same, and the chevron beside "+" lists
  them too. Cmd+O (Ctrl+Shift+O) finds a chat to open in a terminal, which Cmd+T used to do.

## Updates

The app updates itself only when the user says so (`sb-core/src/update.rs` for the rules, `sb-desktop/src/update.rs`
for the app's side).

- **The list.** Each release has `latest.json` (each build's file, size, SHA-256, version and build id) and
  `latest.json.sig`, an Ed25519 signature of its exact bytes. The app gets both from the server (`GET /api/app`,
  which passes them on unchanged) or, when the build names one, from a feed URL. It checks the signature with the
  public key compiled in from `SB_UPDATE_PUBKEY`. A build made without that key has updates off and says so in
  Settings > Updates.
- **Where the server looks.** When `~/.config/switchboard/downloads.json` names a download page (`publish.sh` writes
  one), the server reads the list there. Otherwise it reads the newest GitHub release of `nicglazkov/switchboard`
  (`server/releases.py`; `SWITCHBOARD_RELEASES=owner/name` names another repository, `off` turns it off), at most
  every 5 minutes. The app has no TLS of its own, so for a GitHub release it downloads through the server:
  `GET /api/app/files/<file>` passes the release's file on, Range requests included, and only for a file the
  newest list names. Since the app checks the signature and each file's SHA-256, neither the server nor GitHub can
  change what it installs. A check that fails says why in Settings > Updates instead of "This is the newest
  version".
- **When.** One check about a minute and a half after launch, then once a day, never two at once. Settings has
  Check now.
- **What.** Only a build that is newer in both senses: the version may not go back (semantic version order) and
  the build id must move forward. A build that was rolled back is not offered again.
- **Download.** Into the cache folder as `<file>.partial`, resumed with an HTTP Range request after a stop. The
  finished file must have the size and SHA-256 from the signed list, or it is deleted.
- **Install.** The user picks Restart to update or Install on quit. The new build is unpacked beside the old one
  and run once with `--version` to be sure it starts. Then one swap: on a Mac the two `.app` bundles trade places
  with `renamex_np(RENAME_SWAP)`; on Windows the running exe is renamed to `Switchboard.old.exe` and the new one
  takes its name; on Linux the new binary is renamed over the old one, which is kept as `Switchboard.old`. The old
  build stays until the new one reports healthy (20 seconds of running, or a clean quit).
- **Rollback.** `update.json` in the settings folder notes the install. If the new build starts a second time
  without having reported healthy, it crashed or would not start, so it puts the old build back, opens it and
  exits. The swap touches only the app, the kept copy and `update.json`: never settings or first-run choices.

## Diagnostics

The app writes its own log (`native/crates/sb-desktop/src/logs.rs`) and nothing leaves the computer unless you copy
it out. The log folder is `~/Library/Logs/Switchboard` on macOS, `%LOCALAPPDATA%\Switchboard\logs` on Windows and
`$XDG_STATE_HOME/switchboard/logs` (or `~/.local/state/switchboard/logs`) on Linux.

- `Switchboard.log` turns into `Switchboard.log.1` past 2 MB, and 5 files are kept. Each line has the time, the
  level and the module. `SB_LOG` (or `RUST_LOG`) filters it the way env_logger does, for example
  `SB_LOG=warn,Switchboard=debug`; the app's own modules are `Switchboard::...` and the shared ones `sb_core::...`.
  The default is `info`.
- Lines never hold the home folder (it is written as `~`) or a token in a URL, and the app does not log chat text.
- A panic (`src/panics.rs`) goes to the log and to `crash-<time>.txt` in the same folder, the 10 newest kept, and
  then ends the process as before. [building.md](building.md) explains how to read its backtrace.
- About has Show log (opens the folder), Copy diagnostics (version, build, OS and the last 200 log lines, without the
  server address) and Open source licenses.
- `switchboard-hub` and `switchboard-node` print to stderr and keep no log files of their own; whatever runs them
  (launchd, systemd) decides where that goes.

## The installer: `native/crates/sb-install`

`switchboard-install` installs, updates and uninstalls the app. It is one binary with a GPUI window and a
`--quiet` mode. Everything that differs by system (where apps go, login items, the Dock, shortcuts, the Settings >
Apps entry, asking a running app to quit, disk images) is behind the `Platform` trait in `src/platform/`. The Mac
(`mac.rs`) and Windows (`windows.rs`) have real backends; other systems get a plain one that only copies files.

On Windows the app is a folder, `%LOCALAPPDATA%\Programs\Switchboard`, holding `Switchboard.exe`, the license
notices and `Uninstall Switchboard.exe`. The download is the installer exe with those files attached (`pack.rs`:
zlib-compressed entries and an end marker, found again after Authenticode signing); it unpacks them to the temp
folder and installs from there with the same flow as the Mac. After the swap, `register` writes the Start menu
shortcut (carrying the app's AppUserModelID, which `sb-desktop` also sets so its notifications show) and the
`HKCU` Uninstall entry; the choices add a desktop shortcut or an `HKCU\...\Run` value. A running copy is found by
the path of its exe, asked to close its windows (WM_CLOSE), and stopped only when the user agrees. The version is
read from the exe's version resource, never by running it. `Places` holds every folder and key the backend
touches, and `SB_INSTALL_TEST_ROOT` with `SB_INSTALL_TEST_REG` moves them all into a throwaway folder and a
`HKCU\Software\SwitchboardTest-*` key for tests. The Win32 calls are only built on Windows; the paths, registry
values and messages are plain code, tested on every system.

The parts that decide what happens have no UI and are unit tested: version comparison (`version.rs`), the staged
copy, swap and roll back (`swap.rs`), the first-run record (`record.rs`) and the uninstaller's keep and remove
lists (`uninstall.rs`). `flow.rs` puts them together for both the window and `--quiet`. The window copies the
app's color tokens from `sb-desktop/src/theme.rs` and uses its fonts and icons.

## Ports

| Port | What |
|---|---|
| 8130 | The server's API |
| 8131 | Downloads for adding a computer |
| 8132 | The hub: terminals and linked computers |

None of them has a password. See [Privacy and security](privacy.md).
