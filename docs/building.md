# Building and releasing

This page is for working on Switchboard. To install it, see [install.md](install.md).

## On your own computer

Everything builds with plain `cargo` and runs with plain `python3`:

```sh
python3 -m unittest discover -s tests -t .      # server and helper tests
cd native
cargo test --workspace --locked                 # Rust tests
cargo run -p sb-desktop                         # the app, against the server in SWITCHBOARD_URL or Settings
```

The app's release build is `cargo build --release --locked -p sb-desktop`. The Mac app needs a universal build to be
shipped; see below.

## Build hosts

The release scripts in `native/scripts/` can build on other computers over ssh, so one laptop can make the Mac,
Windows and Linux builds. They read the hosts from `~/.config/switchboard/build-hosts.env`, which is never part of
the repository. Start from the example:

```sh
cp native/scripts/build-hosts.example.env ~/.config/switchboard/build-hosts.env
```

A script only asks for the hosts it uses, and says which one is missing. Set `SB_REMOTE` to give each working copy
its own build folder on the remote host, so two builds don't overwrite each other.

| Script | Builds | Needs |
|---|---|---|
| `native/scripts/build-mac.sh` | `dist/Switchboard-mac.zip`, universal (Apple silicon and Intel) | a Mac with Rust and the Xcode command line tools |
| `native/scripts/build-dmg.sh` | `dist/Switchboard-<version>.dmg`: the app, the installer and an Applications shortcut | the same Mac, plus Python 3 and access to PyPI |
| `native/scripts/build-windows.sh` | `dist/Switchboard-Setup-<version>.exe` and `dist/Switchboard-windows.zip` | Windows with MSVC and OpenSSH |
| `native/scripts/build-linux.sh` | `dist/Switchboard-linux-x86_64` | Linux with Rust and the GPUI packages |
| `native/scripts/linux-shots.sh` | screenshots from a headless Linux build | the same Linux host |
| `native/scripts/installer-shots.sh` | screenshots of every installer screen, light and dark | the same Linux host |
| `native/scripts/make-installer-art.sh` | the DMG background and installer header in `native/packaging/installer/` | the Linux host, with `rsvg-convert` |

## The Mac installer

The disk image holds `Switchboard.app`, an `Applications` shortcut and `Install Switchboard.app`, a small GPUI app
from `native/crates/sb-install`. People can drag the app across, or open the installer for a guided setup: a
welcome, a short tour (screenshots in `native/packaging/tour/`), two choices that both start off (open at login,
add to the Dock), progress, and a done screen that can open the app and eject the image.

- It installs into `/Applications` when that folder can be written without an admin password, else into
  `~/Applications`, and says which. It never asks for an admin password and installs nothing else.
- An update copies the new app next to the old one, checks it, swaps the two by renaming and puts the old one back
  if anything fails. It refuses to install an older version over a newer one. A running copy is asked to quit, and
  the user is asked before it is stopped.
- The first-run choices are saved in `first-run.json` in the app's settings folder and applied only that once.
  Settings are never touched.
- `--uninstall` (or Uninstall on the welcome screen) lists what is removed and what is kept before it does
  anything. Settings are kept unless the user asks for them to go.
- `--quiet` shows no window, prints one line per step and exits non-zero on failure (3 when it refused a
  downgrade). `--app` and `--dest` pick the app and the folder, which is how the tests install into a throwaway
  folder. `switchboard-install --help` lists every flag.

The background, the header art and the tour shots are files in the repository. `make-installer-art.sh` redraws
the first two from the colors and the `[dmg]` layout in `branding.toml`. The tour shots are `linux-shots.sh` frames
of the app against `native/scripts/fixture-server.py` (the board, the inspector, New chat and History, light and
dark), scaled to 1120 by 700; redo them when those screens change. The image's Finder layout is written
straight into its `.DS_Store` with the `ds_store` and `mac_alias` Python packages, installed in a throwaway venv
for each build, so the build needs no Finder, AppleScript or logged-in user.

## The Windows installer

`build-windows.sh` makes `dist/Switchboard-Setup-<version>.exe`, the one file people download. It is the same
`sb-install` program with `Switchboard.exe` and `THIRD-PARTY-NOTICES.txt` attached to its end
(`switchboard-install --make-setup <out.exe> <files>`, see `src/pack.rs`), plus a copy of the plain installer that
stays in the app's folder as `Uninstall Switchboard.exe`. The plain zip is still made for the in-app updater.

```sh
native/scripts/build-windows.sh      # needs SB_WINDOWS_BUILD_HOST in build-hosts.env
```

The script builds `sb-desktop` and `sb-install` on the build host, checks that both are window (GUI subsystem)
programs with an icon, version info (product name, publisher from `branding.toml`, file version) and GPUI's
manifest, and that neither imports `vcruntime140.dll` or `msvcp140.dll`: `native/.cargo/config.toml` links the C
runtime in, so nothing else has to be installed first. It then makes the Setup file and checks that it answers
`--version`. Neither the app nor the installer's window is ever opened on the build host.

What the installer does on a PC:

- It installs per user with no admin rights: the app goes in `%LOCALAPPDATA%\Programs\Switchboard`, everything
  else is under `HKCU` or in the user's own folders.
- A Start menu shortcut is always added. A desktop shortcut and "Open at login" (an `HKCU\...\Run` value) are
  choices that start off and are applied once, on the first install for that account.
- It adds an entry to Settings > Apps (`HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\<desktop_id>`)
  with the name, version, publisher, icon, folder, size and the uninstall commands, and no Modify or Repair.
- Updates follow the Mac rules: no downgrades (exit code 3 with `--quiet`), "Update Switchboard, from X to Y", the
  new copy is checked before an atomic swap that rolls back on failure, and a running copy is asked to close its
  windows before the user is asked whether to stop it. Settings and `first-run.json` are never touched.
- It checks for a Direct3D 11 graphics driver, which the app needs to draw, and says so when there is only
  Windows' basic driver or none.
- `Uninstall Switchboard.exe` (also what Settings > Apps runs, with `--uninstall`) removes the shortcuts, the
  registry entries, the app and its caches and own logs, asks whether to keep settings, and deletes itself a few
  seconds after it exits, with no console window.

**Testing without touching the PC.** Two variables point every place the installer writes at a throwaway folder
and registry key, and they must be set together:

```powershell
$env:SB_INSTALL_TEST_ROOT = "$env:TEMP\sb-install-test"          # Programs, Start menu, Desktop, settings, caches
$env:SB_INSTALL_TEST_REG  = 'Software\SwitchboardTest-1234'      # HKCU key for the Uninstall, Run and app id keys
.\Switchboard-Setup-0.8.4.exe --quiet | Out-Host; $LASTEXITCODE
```

The key must start with `Software\SwitchboardTest-`, or the installer refuses to run (exit code 2). Delete the
folder and `reg delete HKCU\Software\SwitchboardTest-1234 /f` afterwards. The installer is a window program, so
PowerShell only waits for it and sets `$LASTEXITCODE` when its output is piped or captured, as above, or with
`Start-Process -Wait -PassThru`.

## Debug symbols and crash reports

Release builds ship stripped. Their symbols go to `native/dist/symbols/<version>-<build>/`, which never goes into
the zip, DMG, installer or tarball. Keep that folder for every build you hand out: only the symbols from the same
build match its crashes.

| Platform | Files | Made by |
|---|---|---|
| macOS | `Switchboard-aarch64-apple-darwin.dSYM`, `Switchboard-x86_64-apple-darwin.dSYM` (and the same for `switchboard-node`) | rustc (`split-debuginfo = "packed"` in `native/Cargo.toml`) |
| Windows | `Switchboard.pdb` and the `Switchboard.exe` it belongs to (and the same for `switchboard-node`) | the MSVC linker |
| Linux | `Switchboard.debug`, `switchboard-node.debug` | `objcopy --only-keep-debug` in `build-linux.sh` |

When the app panics it writes `crash-<time>.txt` to its log folder (About > Show log). The file has the message, the
file and line of the panic, a `Load address:` line and a backtrace of raw addresses. Each frame's offset into the
app is its address minus the load address. To check that a build and its symbols line up, start it once with
`SB_PANIC_TEST=1`: it crashes on purpose right after it starts.

macOS, with the dSYM for the Mac's architecture (`-arch x86_64` and the Intel dSYM on an Intel Mac):

```sh
atos -arch arm64 -o Switchboard-aarch64-apple-darwin.dSYM -l <load address> <address> <address> ...
```

atos prints Rust's mangled names with the file and line; `rustfilt` (`cargo install rustfilt`) makes the names
readable. `dwarfdump --uuid` on the app and on a dSYM shows whether they belong together.

Windows, with the `.pdb` next to the matching `.exe` (`llvm-symbolizer` comes with LLVM; WinDbg's `ln` works too):

```sh
llvm-symbolizer --relative-address --obj=Switchboard.exe 0x<address minus load address>
```

Linux:

```sh
addr2line -f -C -i -e Switchboard.debug $(printf '0x%x' $(( <address> - <load address> )))
```

## License notices

`native/packaging/THIRD-PARTY-NOTICES.txt` lists the license of every Rust crate the apps are built from, on every
platform, and of the bundled fonts. Run `native/scripts/notices.sh` after changing dependencies and commit the
result. It needs `cargo-about` (`cargo install --locked cargo-about --features cli`) and stops when a crate's license
is not in `native/packaging/notices/about.toml`, so a new license gets looked at before it ships. The build scripts
put the file in `Switchboard.app/Contents/Resources`, next to `Switchboard.exe` in the zip and in the Linux tarball;
the app also has a copy built in for About > Open source licenses.

## Signing

Signing identities and notarization keys come from environment variables when a script runs. They are never
written into the repository, and the scripts never copy a certificate or key off the machine that holds it.

Without them every build is signed ad hoc. `build-dmg.sh` signs both apps and the image with a Developer ID when
`SB_SIGN_IDENTITY` is set (the name of a certificate in the build Mac's keychain), and notarizes and staples the
image when it also has a way to reach Apple: `SB_NOTARY_PROFILE`, the name of a profile saved on that Mac with
`xcrun notarytool store-credentials` (nothing else to keep track of), or `SB_NOTARY_KEY_ID`, `SB_NOTARY_ISSUER` and
`SB_NOTARY_KEY_PATH` (an App Store Connect API key file on that Mac). Both need the build Mac's login keychain
unlocked, which an ssh session is not by default.

Windows builds are unsigned unless `SB_WIN_SIGN_CERT` is set to a `.pfx` file on the Windows build host;
`SB_WIN_SIGN_PASSWORD`, `SB_WIN_SIGN_TIMESTAMP` (an RFC 3161 server URL) and `SB_WIN_SIGNTOOL` (signtool's path) are
optional. The marked block in `windows-build.ps1` then signs `Switchboard.exe` and the installer before the Setup
file is made, and the Setup file after. Its attached files are still found after signing, because the reader looks
where the PE header says the signature starts.

## Versions and releases

- The version is in `native/Cargo.toml` (`[workspace.package] version`) and in `Cargo.lock` for the five crates.
  Each release adds an entry at the top of `RELEASES` in `native/crates/sb-desktop/src/about.rs`, which is the
  change list shown in About.
- A release is two commits: `Switchboard X.Y.Z` with the code, then `Packaging for X.Y.Z` with the installers'
  changes. Installers are written to `dist/`.
- Updates never go backwards: the app ignores a build that is not newer than the one running, in version and in
  build id.

## Updates

The apps take an update only from a list signed with your Ed25519 key. Everything comes from environment
variables (or `~/.config/switchboard/build-hosts.env`), never from the repository:

| Variable | Used by | What |
|---|---|---|
| `SB_UPDATE_PUBKEY` | the build scripts, through `sb-core/build.rs` | The public key, base64. Compiled into the app. Without it the build has updates off. |
| `SB_UPDATE_KEY_FILE` | `publish.sh` | The private key file. It never leaves the computer that publishes. |
| `SB_UPDATE_FEED` | the build scripts (optional) | An `http://` folder with `latest.json`. Without it the app asks its server, which reads `SB_PUBLISH_BASE` from `downloads.json`. |
| `SB_OPENSSL` | `update-key.sh`, `publish.sh` (optional) | An OpenSSL 3 to use. The macOS `/usr/bin/openssl` is LibreSSL and can't sign. |

Once, make the key pair outside the repository and keep a backup of the private key. A lost key means the apps
already installed can't take another update until they are installed again by hand.

```sh
native/scripts/update-key.sh ~/.config/switchboard/update-key.pem
# prints SB_UPDATE_PUBKEY=...; put it and SB_UPDATE_KEY_FILE in build-hosts.env
```

For each release:

1. Build with `build-mac.sh`, `build-windows.sh` and `build-linux.sh`. They pass `SB_UPDATE_PUBKEY` and
   `SB_UPDATE_FEED` to cargo and say when the key is missing. A key that is set but malformed stops the build.
2. Run `native/scripts/publish.sh`. It writes `latest.json` from `dist/` (sizes, SHA-256 and each build's id from
   its `.build` file), signs it with `SB_UPDATE_KEY_FILE`, refuses a key that doesn't match `SB_UPDATE_PUBKEY`,
   and uploads both. The publish host checks the list against the uploaded files and serves it unchanged.
   `--unsigned` publishes without a signature, and then no app offers that release as an update.

`native/scripts/update-key.sh --public <key file>` prints the public key of an existing private key.

## Branding

`branding.toml` holds the app's name, bundle ids, colors and DMG layout. A fork that renames the app changes that
file; the build scripts and installers read it.
