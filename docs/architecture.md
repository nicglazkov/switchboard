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
`bonjour.py` (finding the server on the local network).

## The helper: `agent/sb.py`

One Python file, copied to `~/.claude/switchboard/sb.py` on each computer. It is Claude Code's status line and
hooks there, so it knows each chat's state as it changes, and it answers the server's requests (list chats, start
one, stop one). The server reaches it in one of two ways:

- **ssh**, with a key that can only run `sb.py` (a forced command, no shell);
- **a link through the hub**, for computers that have no ssh server or sit behind another router.

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

## Updates

The app updates itself only when the user says so (`sb-core/src/update.rs` for the rules, `sb-desktop/src/update.rs`
for the app's side).

- **The list.** The download page has `latest.json` (each build's file, size, SHA-256, version and build id) and
  `latest.json.sig`, an Ed25519 signature of its exact bytes. The app gets both from the server (`GET /api/app`,
  which passes them on unchanged) or, when the build names one, from a feed URL. It checks the signature with the
  public key compiled in from `SB_UPDATE_PUBKEY`. A build made without that key has updates off and says so in
  Settings > Updates.
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

None of them has a password. See [SECURITY.md](../SECURITY.md).
