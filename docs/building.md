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
| `native/scripts/build-windows.sh` | `dist/Switchboard-windows.zip` | Windows with MSVC and OpenSSH |
| `native/scripts/build-linux.sh` | `dist/Switchboard-linux-x86_64` | Linux with Rust and the GPUI packages |
| `native/scripts/linux-shots.sh` | screenshots from a headless Linux build | the same Linux host |

## Signing

Signing identities and notarization keys come from environment variables when a script runs. They are never
written into the repository, and the scripts never copy a certificate or key off the machine that holds it.

## Versions and releases

- The version is in `native/Cargo.toml` (`[workspace.package] version`) and in `Cargo.lock` for the four crates.
  Each release adds an entry at the top of `RELEASES` in `native/crates/sb-desktop/src/about.rs`, which is the
  change list shown in About.
- A release is two commits: `Switchboard X.Y.Z` with the code, then `Packaging for X.Y.Z` with the installers'
  changes. Installers are written to `dist/`.
- Updates never go backwards: the app ignores a build that is not newer than the one running.

## Branding

`branding.toml` holds the app's name, bundle ids, colors and DMG layout. A fork that renames the app changes that
file; the build scripts and installers read it.
