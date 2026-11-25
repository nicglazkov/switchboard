---
description: For developers. The programs that make up Switchboard, the protocols between them, and how device keys, the tunnel, the relay and the push gateway fit in.
---

# How the pieces fit

<p class="sb-meta">Last validated: Nov 22, 2026</p>

Switchboard is four programs. This page says what each one does and how they talk, so you know where to look when
you change something. It uses the names in the code: the server is what the other pages call the home
computer's software, a machine is a computer, and a node is the helper program `switchboard-node`.

- **The app** (`sb-desktop`) talks HTTP to the server on port 8130, and WebSocket to the hub on port 8132 for
  terminals.
- **The server** (`server/`) reaches the helper on each computer over ssh, or through the hub.
- **The hub** (`sb-hub`) holds one outgoing WebSocket from each `switchboard-node`.
- **The helper** (`agent/sb.py`) runs on each computer, inside Claude Code.

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
Version 0.9.0 added `keys.py` (the keys mode, and whether keys only is safe), `pushcrypto.py` (signing and
sealing pushes for the gateway) and `downloads.py` (port 8131).

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
- `GET /api/transcript/<m>/<sid>`: a chat as its messages, for the phone apps' chat page: what was typed, what
  Claude wrote, and each tool call with its output or its changes, a slice at a time (`?before=` pages back,
  `?after=` asks for what is new). The computer the chat is on reads it from Claude Code's own transcript when
  asked. Nothing is kept by the server, and long output is cut to its start and its end.
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
| `cleanup` | Chats that do nothing are taking memory, and are not ones that stop by themselves | At most once a day |
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

Since version 0.9.0 it also holds the computer's device key and runs the tunnel and the control API, described
under [Device keys and the tunnel](#device-keys-and-the-tunnel).

## Device keys and the tunnel

!!! note ""
    This section describes what arrived with version 0.9.0. Keys are off until
    `switchboard keys on` turns them on.

With keys on, a paired computer reaches home by its key, not by an address. Nothing in the app, the server or
`sb.py` changes for this: they keep talking plain HTTP and WebSocket to a port on their own computer, and
`switchboard-node` carries those connections.

<figure class="sb-fig" markdown="0">
--8<-- "docs/diagrams/arch-tunnel.svg"
<figcaption>Figure 1. The app and sb.py connect to sb-node on their own computer. It carries each connection to
sb-node at home, which hands it to the server or to the hub's tunnel listener.</figcaption>
</figure>

### The crate: `native/crates/sb-link`

`sb-link` has the keys, pairing and the tunnel. Its modules don't know how two computers reach each other, except
`endpoint`, which is the only file that names the network library, iroh.

- `identity`: the device key: 32 random bytes in `~/.config/switchboard/device.key`, an Ed25519 secret key. The device's id is its public key.
- `paired`: `~/.local/share/switchboard/paired.json`, the devices home has paired with. It is the allow list for the tunnel, and a list that can't be read allows nobody.
- `code`: what a person carries to the joining device: a short code of 8 letters, shown as `K7PM-2XQ9`, or a link `switchboard://pair/1?h=&r=&s=&n=` with a 128-bit secret.
- `pairing`: the exchange in which both ends prove they hold the code or the link's secret, bound to both ids and to the channel under it, so a middleman ends up with keys that don't match.
- `tunnel`: one stream for each connection, with a first line that says where it goes.
- `endpoint`: one iroh endpoint for each computer, with the computer's key. It speaks `switchboard/tunnel/1` and `switchboard/pair/1`.
- `control`: the control API's answers as data, shared by the node, the server and the command line.

### The tunnel

On a paired computer that is not home, `sb-node` listens on `127.0.0.1:8130` and `127.0.0.1:8132`. For each
connection it opens one stream to home and writes one line of JSON, `{"to":"api"}` or `{"to":"hub"}`. From then
on it copies bytes both ways.

Home's side reads the line and connects to the local address its own settings give for that name: the server
on `127.0.0.1:8130` for `api`, the hub's tunnel listener on `127.0.0.1:8136` for `hub`. A name it has no address
for is refused, so the other side never picks an address.

Who may open a stream is decided before that. The endpoint refuses a device that isn't on the paired list when
its connection arrives, when the connection is taken, and again at every stream. A device that is removed is
cut off without a restart, and its open connections are closed. `switchboard/pair/1` is open to anyone who can
reach the endpoint, and leads to the pairing exchange and nothing else.

The endpoint uses no servers but the relay it is given: no DNS or pkarr lookup, no mDNS and no port mapping.

### The control API

`sb-node` serves a control API on `127.0.0.1:8137` whenever it runs, with keys on or off
(`sb-node/src/control.rs`). It is how the server and the command line on the same computer talk to keys in the
running node.

| Route | What it does |
|---|---|
| `GET /status` | What keys are doing on this computer |
| `POST /pair/new` | At home: a code and a link for one new device |
| `POST /pair/join` | On any other computer: pair with a home computer |
| `GET /devices` | At home: the paired devices |
| `POST /devices/remove` | At home: forget a device and cut it off |
| `POST /relay` | Set the relay, or none |

It answers its own computer only. The connection must come from this computer, the `Host` must name it, there
must be no `Origin` or `Sec-Fetch-Site`, and a POST must be JSON. A refusal carries a sentence for the person in
`error` and one word for the program in `kind`.

The server passes these on to the apps (`server/nodes.py`): `GET` and `POST /api/keys/devices`,
`POST /api/pair_device`, `GET /api/keys/readiness` and `POST /api/keys/mode`. The desktop app's **Settings**,
**Devices** page uses them. `POST /api/keys/mode` writes the mode and restarts the server. Where the server
can't restart itself, as on Windows, it answers 409 with the command to run by hand, and the app shows it.
`GET /api/keys/readiness` takes `?days=N`, and otherwise uses `keys_ready_days` from `server.json`.

### Phones: `native/crates/sb-mobile`

A phone has no node, so the same pieces of `sb-link` run inside the phone apps, behind a few calls that Swift
and Kotlin reach through generated code (UniFFI). The app gets two loopback ports, one for the server's API and
one for the hub, and its HTTP and WebSocket code points at them. Each connection there becomes one stream to
home. Other apps on the phone can connect to `127.0.0.1` too, so the ports ask for a secret first.

The crate keeps no secrets. The app stores the phone's key, in the iOS Keychain or an Android Keystore-wrapped
file, and remembers which home it paired with. The phone apps that use it are not released yet.

### The hub's tunnel listener

With keys on, the hub has two listeners. The main one on port 8132 keeps every route. The second, on
`127.0.0.1:8136`, is where connections that arrived through a tunnel come in, and it has the routes a paired
computer needs and no others: `/term`, `/node`, `/pair` and `/healthz`. `SWITCHBOARD_HUB_TUNNEL_LISTEN` moves it,
to an address on the same computer only. The hub reads the mode from `keys` in `server.json`: `off`, `both` or
`only`, where a value it doesn't know counts as `only`.

### The relay

Two computers that can't reach each other directly meet at a relay, which passes encrypted packets and can't
read them. The relay is the stock `iroh-relay`, version 1.5.0. `scripts/deploy-relay.sh` sets one up, and
`scripts/relay/README.md` describes it. A short code needs a relay, because the code alone must find home. A
link carries home's id and works without one.

### The push gateway: `gateway/`

Apple and Google accept notifications only from whoever published the app. The gateway is a small Go service
that holds those keys, so that no home computer has to. It has one route, `POST /v1/push`.

- The home computer signs each request with its device key. The gateway checks the signature and passes the push
  on to APNs or FCM.
- The words of a notification are sealed on the home computer for one phone (`server/pushcrypto.py`: Ed25519,
  and HPKE with X25519, HKDF-SHA256 and ChaCha20-Poly1305). The gateway can't read them.
- It keeps nothing on disk. Its replay and rate-limit counters are in memory.
- `"mode": "direct"` in `apns.json` still sends straight to Apple with your own key, for someone who builds the
  app themselves.

`gateway/README.md` has the request format and the sealed format for the phone's side.

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
  then ends the process as before. To read its backtrace, refer to
  [Building and releasing](building.md).
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
| 8136 | Since version 0.9.0: the hub's tunnel listener, on `127.0.0.1` only, while keys are on |
| 8137 | Since version 0.9.0: the node's control API, on `127.0.0.1` only |

None of them has a password. Refer to [Privacy and security](privacy.md).
