---
description: The Switchboard app for macOS, Windows and Linux, how each one installs and uninstalls, and where the iPhone app stands.
---

# The apps

The desktop app is the same on every system: the board, terminals, History, Files and Settings. Download it from
the [latest release](https://github.com/nicglazkov/switchboard/releases/latest).

| | Download | Needs |
|---|---|---|
| :material-apple: **Mac** | `Switchboard-<version>.dmg` | macOS 11 or later, Apple silicon or Intel |
| :material-microsoft-windows: **Windows** | `Switchboard-Setup-<version>.exe` | 64-bit Windows |
| :material-linux: **Linux** | `Switchboard-linux-x86_64.tar.gz`, or the plain `Switchboard-linux-x86_64` binary | x86_64, Vulkan, libxkbcommon, and X11 or Wayland |

On first launch the app looks for your server on the local network. If it doesn't find one, type the server's
address (`http://<server>:8130`), or let this computer [be the server](install.md).

## Mac

Open the disk image and double-click **Install Switchboard**. It copies the app to Applications, or to the
Applications folder in your home folder when Applications needs an administrator password, and offers to open it
at login and add it to the Dock. Nothing else is installed, and no password is asked for. Dragging the app to
Applications works too.

The disk image and the app are signed with a Developer ID and notarized by Apple.

## Windows

Run **Switchboard-Setup**. It installs for you alone, with no administrator rights: the app goes in
`%LOCALAPPDATA%\Programs\Switchboard`, with a Start menu shortcut and an entry in **Settings, Apps**. It offers to
open Switchboard at login and to put a shortcut on the desktop.

!!! warning "Windows protected your PC"
    The installer isn't code signed yet, so SmartScreen may stop it the first time. Click **More info**, then
    **Run anyway**.

Uninstall it from **Settings, Apps**, like any other app.

??? note "Install from a script"
    `Switchboard-Setup-<version>.exe --quiet` installs without a window, one line per step. Add
    `--open-at-login`, `--desktop-shortcut` or `--open` for those choices; `--uninstall --quiet` removes it
    (`--remove-data` also removes its settings). [Every flag](commands.md#the-desktop-installer)

## Linux

The `.tar.gz` holds the app, a `.desktop` file and icon for your launcher, the license notices, and the server
kit, for running the server on that computer. The plain binary is just the app:

```bash
chmod +x Switchboard-linux-x86_64
./Switchboard-linux-x86_64
```

It needs a Vulkan driver (Mesa or your GPU vendor's), libxkbcommon, and X11 or Wayland, which most desktop
installs already have. Without `xdg-desktop-portal`, a terminal's **Attach** has no file picker, so drag files in
instead.

## Updates

Each app checks for a new version once a day, through your server, and asks before it installs anything.
[How updates work](updates.md)

## iPhone

There is an iPhone app, with the board, a terminal for every chat, History, Files, usage, and widgets for your
week. It isn't on the App Store, and its source isn't in this repository yet.

### iPhone notifications

When the iPhone app is set up, your server can send it notifications through Apple's push service: **Held
messages**, **Finished tasks**, **Chats not in use** and **Hard limit**, each one switchable in **Settings,
Notifications**.

The server needs an Apple Push Notification service key from your Apple Developer account, for the app's bundle
ID:

1. Put the key file (`AuthKey_<key id>.p8`) in `~/.config/switchboard/apns/` on the server.
2. Write `~/.config/switchboard/apns.json`:

    ```json
    {
      "key_id": "ABC123DEFG",
      "team": "YOUR_TEAM_ID",
      "topic": "the app's bundle ID",
      "sandbox": true
    }
    ```

    `sandbox` is `true` for a development build of the app (the default) and `false` for a release build. Add
    `"key"` to name a key file with a different name.

3. Restart the server (`switchboard restart`). Sending needs the Python `cryptography` package and a `curl` with
   HTTP/2 on the server; if anything is missing, the server log says `push disabled` and why.

Until then, **Settings, Notifications** says notifications are off. A notification carries chat names, computer
names and a short status line. [What leaves your network](privacy.md#what-leaves-your-network-and-when)

## Android

There is no Android app right now.
