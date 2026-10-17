# Switchboard native: the stack

## The decision

**One Rust core, `sb-core`, under every app. GPUI, `sb-desktop`, draws the desktop app on macOS, Windows
and Linux. Later, SwiftUI on iOS and Jetpack Compose on Android call the same core through UniFFI.**

This follows the recommendation in the design notes, with one change: the core talks to the
Switchboard server over HTTP, and the server keeps doing the SSH work. The notes sketched a core that
SSHes to the machines itself. Keeping SSH on the server means the keys and the
machine logic stay in one place, and the phones could never SSH anyway.

### Why GPUI

- **Fast.** It draws on the GPU (Metal on macOS, DirectX 11 on Windows, Vulkan through wgpu on
  Linux) and needs no web engine. On Linux, even with software rendering, the first frame is on
  screen about 0.1 s after launch. That first frame already shows the saved board.
- **One language.** On the desktop, the core is a normal Rust crate: no FFI, no bridge and no
  second copy of the rules.
- **Fits the design.** Actions, key bindings and key contexts are GPUI's input model, and this
  design is keyboard first. Every window reads the same app state entity, so a lane torn off into
  its own window stays in step with the board.
- **Used in production.** Zed runs on GPUI on all three desktops.

### What it costs (accepted)

- **No stable API.** GPUI ships as weekly snapshots of the Zed repo (`gpui-pre`). Callers break
  between snapshots. We pin exact versions and bump them on purpose (see "Upgrading GPUI" below).
- **No widget kit.** Cards, sheets, the palette, segmented controls, menus and toasts are built
  by hand in `sb-desktop` (about 4,300 lines). `gpui-base` supplies only the text input engine:
  selection, IME, clipboard and undo.
- **Gaps in GPUI.**
  - No tray or menu bar icon API, so there is no needs-you count in the tray.
  - No letter-spacing, so the small uppercase labels are set without the tracking the web uses.
  - Accessibility is immature, and screen readers get little.
- **Size.** The Linux binary is 33.5 MB stripped. wgpu, naga, cosmic-text and resvg make up most
  of it; the Switchboard code itself is small. Fat LTO and `opt-level = "s"` are untried.

### Alternatives, briefly

These are the options the design notes weighed:

- **Tauri** would run the HTML mockups almost as is. It costs a WebView per window and three
  different engines.
- **Slint** gives native-feeling widgets, but custom drawing is more work and its license needs a
  choice.
- **egui** looks like a tool, not a polished app.
- **SwiftUI plus WinUI** feels best on each OS, but means two or three UI codebases, and Linux
  still needs an answer.

## How it fits together

```
sb-core (no UI deps)                         sb-desktop (GPUI)
  api      wire format of /api/*               state.rs   AppState: snapshot, 10 s poll, 1 s tick, jobs,
  model    Session, Machine, Plan, rules                  toasts, settings, needs-you notifications
           (lamp, phrase, explain, chips)      view.rs    the Swimlanes board, title bar, footer, keys
  board    the Swimlanes grid                  inspector  the selected chat, actions, context meter
  actions  what you can do with a chat         palette    Cmd/Ctrl K
  jobs     following server jobs               sheet.rs   New session
  names    fresh/new/web naming                archive    Older and Replaced
  fmt      ages, tokens, times                 theme.rs   Converge tokens, dark and light
  net      HTTP, LAN then Tailscale            ui.rs      shared pieces (chips, kbd, buttons)
  cache    snapshot + folder trees on disk
  search   palette matching
  Core     the facade: owned in, owned out
```

- **Startup.** `AppState::init` reads the snapshot from disk and the first frame draws from it.
  The first `Core::refresh()` runs on GPUI's background executor, and its result replaces the
  board.
- **Polling and redraws.**
  - The board polls every 10 s, and sooner after an action.
  - A poll that brings the same board (`Snapshot::same_board`) does not redraw.
  - A 1 s tick redraws only when something on screen counts in seconds: a warm chat's countdown,
    a running job, or a stale "Updated" time. Otherwise it redraws once a minute, so ages move on.
- **Core calls.** Every `Core` call blocks, and the desktop always calls them off the UI thread.

## Verified, and not

| | Status |
| --- | --- |
| sb-core: models, rules, board, actions, names, fmt, cache, LAN then Tailscale fallback over real sockets, POST bodies and headers | **verified**: 26 tests (`cargo test -p sb-core`), all against fixtures and local fake servers |
| Linux x86_64 build and run (X11) | **verified** on Ubuntu: Xvfb plus Mesa lavapipe (software Vulkan) |
| Every screen at 1440x900 and 1100x720, dark and light: board, inspector (docked and floating), palette, New session (normal and UltraCode), confirm, continue menu, archive (Older, Replaced), plan popover | **verified** by screenshots, compared with the Swimlanes mockup |
| Actions: start fresh (confirm, then Enter), new session (typed, Ctrl+Enter), dismiss and continue on another machine from the palette, a failing new session (error toast, sheet stays open), keyboard navigation | **verified** with xdotool against a local fake server, POST bodies checked. The live server was never sent a POST |
| Startup, idle CPU, memory on Linux | **measured**, below |
| macOS: universal (arm64 + x86_64) build, .app bundle, ad hoc signature, zip | **builds**. Never launched: the build Mac runs no GUI tests |
| Windows: MSVC build, GUI subsystem, embedded icon and version info, zip | **builds**. Never launched: the build PC runs no GUI tests |
| How it looks and feels on macOS and Windows; title bar and traffic lights; Retina and HiDPI scaling | **not verified** |
| macOS first launch with runtime-compiled Metal shaders (the build Mac has no Metal Toolchain) | **not verified**. Expect extra time on first launch only |
| Needs-you notifications | **not verified**: compiled for all three, never shown (Xvfb has no notification daemon) |
| Wayland | **not verified**. Built with the `wayland` feature, tested on X11 only |
| Tear-off lane windows, multi-monitor, window restore | **not verified** |
| Screen readers | **not verified**, and likely weak (GPUI) |
| UniFFI bindings for Swift and Kotlin | **not built**. The facade is shaped for them (below) |

## Measured on Linux

Hardware: an Ubuntu desktop, i7-12700KF (20 threads). It renders through Xvfb and Mesa lavapipe, so all
drawing is done on the CPU. Window size 1440x900. Data: 76 chats on 3 machines from the local fake
server. Release build.

| | Result |
| --- | --- |
| Launch to first painted frame, board from the saved snapshot (5 runs) | median 105 ms (101 to 126 ms), with all 76 chats on the first frame |
| Launch to first painted frame, no snapshot yet | 110 ms ("Connecting to Switchboard", then the board when the server answers) |
| Idle CPU, nothing counting down on screen | 0.55% of one core (one redraw a minute; polls that bring no change draw nothing) |
| Idle CPU, warm chats on screen (their countdown redraws every second) | 17% of one core under lavapipe (one full software-rendered frame per second) |
| Memory after 80 s | RSS 207 MB, PSS 143 MB, 114 threads (all three include lavapipe, which owns most of the threads) |
| Binary | 33.5 MB stripped (Linux x86_64); sizes of every artifact are in the publish notes |

How the numbers were taken:

- **First frame.** `SB_TIMING=1` makes the app print when GPUI finishes its first frame, timed
  from `main()`. The launch time is measured from `exec` with a wall clock.
- **Idle CPU.** Process CPU time over 60 s, read after a 20 s settle.

lavapipe makes every redraw a CPU job (about 0.16 s per full 1440x900 frame here), so the idle
numbers are a worst case. On a real GPU the same redraw costs a few milliseconds of CPU. lavapipe's
LLVM JIT and its worker threads also make up much of the memory.

## Phones later: SwiftUI and Compose over UniFFI

The core is already shaped for UniFFI:

- `Core` takes and returns owned values.
- Records are plain structs of `String`, `f64`, `u64`, `bool`, `Option`, `Vec` and simple
  enums.
- There are no generics, lifetimes, callbacks or async in the API.

The cache folder can be passed in through `CoreConfig::cache_dir`, which the phones need because
their sandboxes have no standard cache folder.

1. **Annotate behind a feature,** so the desktop build never sees UniFFI:
   ```toml
   # crates/sb-core/Cargo.toml
   [features]
   ffi = ["dep:uniffi"]
   [dependencies]
   uniffi = { version = "<pin>", optional = true }
   ```
   ```rust
   #[cfg_attr(feature = "ffi", derive(uniffi::Record))]   // Session, Machine, Plan, Snapshot, NewRequest, ...
   #[cfg_attr(feature = "ffi", derive(uniffi::Enum))]     // Lamp, Kind, Tone, Column, ActionId, ...
   #[cfg_attr(feature = "ffi", derive(uniffi::Error))]    // CoreError
   #[cfg_attr(feature = "ffi", derive(uniffi::Object))]   // Core
   #[cfg_attr(feature = "ffi", uniffi::export)] impl Core { ... }
   ```
   Methods on records (such as `Session::lamp`, `board::build`) become exported free functions that
   take the record, so Swift and Kotlin get the same rules: `fn lamp(session: &Session, now: f64)`.
   Add `#[cfg_attr(feature = "ffi", uniffi::export)]` and `uniffi::setup_scaffolding!()` in lib.rs.
2. **A thin crate `crates/sb-ffi`,** `crate-type = ["staticlib", "cdylib"]`, depends on
   `sb-core` with `features = ["ffi"]`, and holds the `uniffi-bindgen` binary.
3. **iOS:**
   - Build: `cargo build --release -p sb-ffi --target aarch64-apple-ios` (plus
     `aarch64-apple-ios-sim`).
   - Bindings: `uniffi-bindgen generate --library libsb_ffi.a --language swift`.
   - Wrap the static libs in an XCFramework (`xcodebuild -create-xcframework`) and add it with the
     generated `.swift` file to the Xcode project.
   - Calling: SwiftUI calls `Core` from a `Task.detached` and publishes the `Snapshot` from an
     `@Observable` model.
   - The existing iOS app keeps its UI and swaps its own session logic for the core's.
4. **Android:**
   - Build: `cargo ndk -t arm64-v8a -t x86_64 -o app/src/main/jniLibs build --release -p sb-ffi`.
   - Bindings: `uniffi-bindgen generate --language kotlin`. They load through JNA.
   - Calling: Compose calls `Core` from `Dispatchers.IO` and exposes a `StateFlow<Snapshot>`.
5. **Plain HTTP:** the server is plain HTTP on the LAN and the tailnet. iOS needs an App
   Transport Security exception (`NSAllowsLocalNetworking`, plus the Tailscale IP), and Android
   needs a network security config that allows cleartext to those hosts.

`sb-core` builds today with no UI dependency (`cargo test -p sb-core` runs headless), but none of
the steps above have been built or tested.

## Upgrading GPUI

`gpui-pre`, `gpui-pre-platform` and `gpui-base` must move together: gpui-base 0.7.2 is built against
exactly gpui-pre 0.3.8 (zed@279fe070bb389b79652e52065b2f001edcc0b11b). To upgrade:

1. Pick a gpui-pre version that a gpui-base release targets.
2. Change all three pins in `native/Cargo.toml`, then run `cargo update -p gpui-pre -p gpui-pre-platform -p gpui-base`.
3. Fix what broke. The API this app leans on:
   - Elements and windows: `div()` styling and flex layout, `StyledText` with `TextRun`s,
     `Window::on_next_frame`, `open_window` with `WindowOptions`.
   - Input and actions: `KeyBinding` with context predicates (`"Board && keys"`), `actions!`.
   - Platform: `show_system_notification` and `on_system_notification_response`.
   - From gpui-base: `InputState` and `TextareaState`.
4. Rerun the screenshot set and the action test on Linux before shipping.
