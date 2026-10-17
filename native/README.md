# Switchboard native

The desktop Switchboard (macOS, Windows, Linux) and the Rust core that future phone apps will share.
Why this stack, what is verified and how SwiftUI and Compose plug in: [STACK.md](STACK.md).

```
native/
  Cargo.toml            workspace, every dependency pinned here
  Cargo.lock            committed; all builds use --locked
  crates/sb-core/       models, rules, HTTP client, snapshot cache. No UI. Tested.
  crates/sb-desktop/    the GPUI app. Binary and window title: Switchboard
  packaging/            Info.plist + AppIcon.icns, app.ico, .desktop + png
  scripts/              build-mac.sh, build-windows.sh, build-linux.sh, publish.sh, make-icons.sh
  dist/                 build outputs (not committed)
```

## What the app does

The Lanes board: one row per machine (any number, any names) with its OS, chats and
memory, columns Needs you, Working, Warm now, Gone cold and Quiet, the server's status note with Stopped today, a
strip for chats open but not in use, and a floating detail sheet with Resume vs Start fresh side by side. Tabs for
Board, History and each terminal; a terminal can also open in a window of its own or as a floating sheet. The flags
drawer, Settings (Cmd+Comma: appearance, terminal look, machines, notifications, the Chrome machine), Files, Ctrl/Cmd
K commands, the New session sheet, Replaced, the plan popover, toasts, job progress and Restart to update. Since the
web app is gone, everything it did is here: open, start fresh, resume, continue on another machine, write a handoff,
hand off and stop, stop (two steps), dismiss, send anyway, relaunch with other flags, files sent to the chat's
machine, new session (machine, mode, model, folder, new folder, first message). Rows can be folded or torn off into
their own window.

Screenshots in a VM with no fleet: `native/scripts/vm-shot.sh <out> <name>:<SB_SHOT state>:<theme>[:<w>x<h>]`, which
serves a fixture snapshot through `native/scripts/fixture-server.py`.

Startup never waits on the network. The first frame paints the snapshot the last run saved to disk;
the live board replaces it when the server answers, then it refreshes every 10 s (sooner after an
action). The board itself never waits behind a spinner; a small turning ring shows only where one
part is still loading (a folder, a chat's history, a machine's tools). With no snapshot yet it shows
"Connecting to Switchboard", and if the server cannot be reached, "Switchboard is not reachable" with the reason, and keeps trying.

Needs-you notifications are native (macOS Notification Center, Windows toast, Linux
notification daemon), informational only, no action buttons; clicking one brings the board forward
with that chat selected. They can be turned off from the commands palette or in Settings > Notifications.

### Keys

| Key | Does |
| --- | --- |
| J K or Down Up | next, previous chat |
| H L or Left Right | previous, next column |
| Enter | the selected chat's lead action (usually Open in Claude) |
| F, R, C | start fresh, resume as is, continue on another machine |
| N or Cmd/Ctrl N | new session |
| Cmd/Ctrl K | commands and chat search |
| Cmd/Ctrl Enter | start, in the New session sheet |
| Esc | close the top sheet, menu or selection |
| Cmd/Ctrl W, Cmd/Ctrl Q | close window, quit |

Letter keys only act on the board; while a text field has focus they type.

On Windows and Linux every Cmd key above is Ctrl, and key caps say so. In a terminal Ctrl+W, T, K and N belong to the
program (a shell deletes a word with Ctrl+W), so there the app takes Ctrl+Shift+W, T, K and N, copy and paste are
Ctrl+Shift+C and V (and Shift+Insert), as in the Windows and Linux terminals. The Mac's menu bar (Settings, History,
light and dark, terminal text size, Quit) is the "..." button at the end of the header.

### Window frames

| | Title bar and window buttons |
| --- | --- |
| macOS | The traffic lights sit on the header's tab row. |
| Windows | The header is the title bar: minimize, maximize (Snap Layouts) and close are drawn at its end and Windows handles their clicks. Other windows use the native title bar. |
| Linux, X11 or a compositor that draws frames (KDE, most wlroots ones) | The system's title bar above the app's header. |
| Linux, GNOME on Wayland (no server-side frames) | The header holds minimize, maximize or restore and close; the other windows get a slim title bar with the same buttons; every window can be resized from its edges and corners (not the sides tiled against the screen). Right click a title bar for the window menu. |

## Build

Each platform builds on a machine of its own OS over ssh. The machine you run the scripts from only copies
sources over and brings the results back. The build hosts are yours to choose: the scripts read them from
`~/.config/switchboard/build-hosts.env`, outside the repo.

| Platform | Command (from the repo root) | Builds on | Output in `native/dist/` |
| --- | --- | --- | --- |
| macOS 11+, universal | `native/scripts/build-mac.sh` | a Mac | `Switchboard-mac.zip` (ad hoc signed Switchboard.app) |
| Windows 10/11 x64 | `native/scripts/build-windows.sh` | a Windows PC, MSVC | `Switchboard-windows.zip` (Switchboard.exe, GUI subsystem, icon) |
| Linux x86_64 | `native/scripts/build-linux.sh` | a Linux box | `Switchboard-linux-x86_64` and `.tar.gz` (binary + .desktop + icon) |

Then `native/scripts/publish.sh` uploads them with the download page to the server's `/app/` folder, where
"Restart to update" and Add a machine look for new builds.

None of the build scripts launch the app or install it anywhere.

From Linux (a cloud VM), `native/scripts/cross-windows.sh` cross-compiles a debug Switchboard.exe for
`x86_64-pc-windows-msvc` with cargo-xwin and runs the sb-core and sb-desktop tests under Wine. It is a check, not a
release: a release exe needs fxc.exe (GPUI's shaders) and rc.exe (the icon) from the Windows SDK.

### Building by hand

Any machine with Rust 1.90 or newer (`rustup`):

```
cd native
cargo test -p sb-core                 # core tests, no network, no UI
cargo build --release -p sb-desktop   # target/release/Switchboard(.exe)
cargo run --release -p sb-desktop
```

Per platform:

- **macOS**: Xcode command line tools. GPUI compiles its Metal shaders at build time with
  `xcrun metal`, which needs Xcode's Metal Toolchain component. Without it, build with
  `--features gpui_platform/runtime_shaders` and the shaders compile at first launch instead
  (build-mac.sh picks automatically; `SHADERS=precompiled|runtime` forces one). Universal binary:
  build `--target aarch64-apple-darwin` and `--target x86_64-apple-darwin`, then `lipo`; the
  script also writes the .app bundle and signs it ad hoc. Unpack the zip with Finder or
  `ditto -x -k`, which keep the signature intact.
- **Windows**: Visual Studio Build Tools with the C++ workload and a Windows SDK (`rc.exe` embeds
  the icon and version info through `winresource`). Release builds use the GUI subsystem, so no
  console window. The window uses the native Windows title bar.
- **Linux**: the packages an Ubuntu build needs: `sudo apt install build-essential pkg-config
  libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev libx11-xcb-dev libxcb1-dev libfontconfig-dev
  libfreetype-dev libvulkan-dev`. At runtime it needs Vulkan (Mesa or a vendor driver),
  libxkbcommon, and X11 or Wayland. Without a GPU, Mesa's lavapipe works (software Vulkan).

## Run

| Platform | Install | Launch |
| --- | --- | --- |
| macOS | `ditto -x -k Switchboard-mac.zip ~/Applications` | `open ~/Applications/Switchboard.app` |
| Windows | unzip anywhere | `Switchboard.exe` |
| Linux | `chmod +x Switchboard-linux-x86_64` | `./Switchboard-linux-x86_64` (the tar.gz adds a .desktop entry and icon) |

`Switchboard --version` prints the version and exits.

### Environment

| Variable | Default | Use |
| --- | --- | --- |
| `SWITCHBOARD_URL` | the servers set in Settings | servers to try, comma separated, like `http://192.0.2.10:8130,http://100.64.0.10:8130`. Point it at a fake server for testing actions |
| `SWITCHBOARD_CACHE` | see below | where the snapshot and folder trees are saved |
| `SWITCHBOARD_OPEN` | the Claude app when installed | `browser` opens chats in the browser even when the Claude desktop app is installed |
| `SB_THEME` | the saved choice, else the system | `dark`, `light` or `system` |
| `SB_SIZE` | `1440x900` | first window size, `WxH` |
| `SB_SHOT` | none | opens a state for screenshots: `inspector`, `palette[=query]`, `new`, `new,ultra`, `archive[=rep]`, `plan`, `inspector,confirm`, `inspector,continue` |
| `SB_TIMING` | off | prints when the first frame was drawn, in ms since `main()` |
| `SB_SETTINGS` | see Files | settings file to use instead (test runs leave your own settings untouched) |
| `SB_SHOT_OUT` | none | `shots` builds only: saves the rendered frame (GPU, no screen capture) to this PNG and quits |
| `SB_DRIVE` | none | `shots` builds only: a script of keys, clicks, typing and frames (see `capture_when_ready` in view.rs) |
| `SB_AS_WINDOWS` | off | `shots` builds only: draws the Windows header (caption buttons) on any OS, to check it on Linux |
| `SB_AS_CSD` | off | `shots` builds only: draws the Linux frame a window shows when the compositor draws none (window buttons, title bars, resize edges), since Xvfb has no compositor |

"Open in Claude" opens a Claude Code session in the Claude desktop app when it is installed (Mac: Claude.app in
/Applications or ~/Applications; Windows: the `claude:` link scheme in the registry; Linux: an
`x-scheme-handler/claude` handler), as `claude://code/session_<id>`, and in the browser otherwise.

Servers are tried in order (put the LAN address before the tailnet one); a reply only counts when it is really a
Switchboard server, so a different machine at the same address on another network is skipped.

### Files

| | macOS | Windows | Linux |
| --- | --- | --- | --- |
| Snapshot cache | `~/Library/Caches/<desktop_id>` | `%LOCALAPPDATA%\Switchboard\cache` | `~/.cache/switchboard` |
| Settings (theme, notifications, folded lanes) | `~/Library/Application Support/<desktop_id>/settings.json` | `%APPDATA%\Switchboard\settings.json` | `~/.config/Switchboard/settings.json` |

`<desktop_id>` is the one in `branding.toml` at the repo root.

Deleting the cache only means the next start paints an empty board until the server answers.

## Pinned versions

Everything is pinned in `native/Cargo.toml` and `Cargo.lock`, and every build uses `--locked`.

| Crate | Version | Why |
| --- | --- | --- |
| gpui (crates.io `gpui-pre`) | =0.3.8 | Zed's GPUI, a weekly snapshot of the Zed repo (zed@279fe070bb389b79652e52065b2f001edcc0b11b, 2026-10-05). GPU rendered (Metal, DirectX 11, Vulkan), native windows and menus, fast startup. Exact pin because GPUI has no stable API: every snapshot can break callers |
| gpui_platform (`gpui-pre-platform`) | =0.3.8, features `x11`, `wayland` | the per-OS platform layer from the same snapshot. Must match gpui exactly |
| gpui-base | =0.7.2 | Longbridge's unstyled GPUI behaviors. Used only for the text input and textarea engine (selection, IME, clipboard, undo). Built against exactly gpui-pre 0.3.8; bump all three together |
| serde, serde_json | 1.0.229, 1.0.154 | the server's JSON |
| ureq | 3.4.2, no default features, `json` | small blocking HTTP client, no async runtime, no TLS (the server is plain HTTP on the LAN and tailnet) |
| dirs | 5.0.1 | per-OS cache and config folders |
| chrono | 0.4.45, `clock` + `std` only | local times like "Wed 3:36am" |
| log | 0.4.34 | logging facade |
| winresource | =0.1.34 (Windows build only) | embeds the icon and version info in Switchboard.exe |
| Rust | `rust-version = 1.90`, edition 2024 | a floor we set, not tested at 1.90. The builds here used current stable (rustc 1.99.0 on Ubuntu) |

Fonts are bundled in the binary (OFL): Instrument Sans 400 to 700 (UI) and JetBrains Mono (numbers), as in the
Lanes design, and Manrope Bold (the wordmark).

Release profile: `opt-level = 3`, thin LTO, 16 codegen units, stripped.

## Testing without touching real sessions

Actions POST to the server and act on real chats. Test them against a fake server, never the live
one: point `SWITCHBOARD_URL` at it and use a throwaway `SWITCHBOARD_CACHE`. The visual checks and
action tests for this build ran on a Linux box under Xvfb with lavapipe against a local fake server
that serves a sample snapshot shifted to now and logs every POST body.
