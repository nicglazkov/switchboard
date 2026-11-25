---
description: The Switchboard desktop app for macOS, Windows and Linux, what it can do on each, how to get it and remove it, and where the iPhone and Android apps stand today.
---

# The apps

<p class="sb-meta">Last validated: Nov 22, 2026</p>

!!! note ""
    The desktop app is available for macOS, Windows and Linux. There is no phone app to download today.

The app lets you see and run your chats from any of your computers. It shows the board that your home computer
keeps, the home computer being the one of your computers that runs Switchboard itself.

## Benefits

- **The same app everywhere**: the board, terminals, History, Files, Library and Settings work the same on macOS,
  Windows and Linux.
- **Nothing to sign in to**: the app finds your home computer on your network and has no account.
- **No administrator password**: the app installs for your user account only.
- **It keeps itself current**: the app looks for a new version once a day and asks before it installs anything.

## What each app can do today

| App | Where it stands | How you get it |
|---|---|---|
| macOS | Available | `Switchboard-<version>.dmg` |
| Windows | Available | `Switchboard-Setup-<version>.exe` |
| Linux | Available | `Switchboard-linux-x86_64.tar.gz`, or the plain `Switchboard-linux-x86_64` program |
| iPhone | Exists, but you can't get it | Not on the App Store or TestFlight |
| Android | Being written | Nothing to install |

The desktop app does the same on all three systems. What differs comes from the home computer, not from the app:
a chat's terminal needs a home computer on a Mac or Linux. For more information, refer to
[Terminals](terminals.md).

## Get the desktop app

To get the desktop app:

1. Download the app.
2. Install it.
3. Connect it to your home computer.

### Prerequisites

Before you can use the app, you must:

- Have a home computer, or be ready to make this computer the home computer. Refer to
  [Install Switchboard](install.md).
- Have one of these: macOS 11 or later on Apple silicon or Intel, 64-bit Windows, or x86_64 Linux with a Vulkan
  driver, libxkbcommon, and X11 or Wayland.

### Download the app

Download the file for your system from the
[latest release](https://github.com/nicglazkov/switchboard/releases/latest).

### Install it

=== "macOS"

    1. Open the disk image.
    2. Double-click **Install Switchboard**.
    3. Choose whether Switchboard opens at login and whether it's added to the Dock. Both are off until you turn
       them on.

    The installer copies the app to Applications. If Applications needs an administrator password, it uses the
    Applications folder in your home folder instead and never asks for the password.

=== "Windows"

    1. Run `Switchboard-Setup-<version>.exe`.
    2. Choose whether Switchboard opens at login and whether it gets a desktop shortcut. Both are off until you
       turn them on.

    The installer needs no administrator rights. It puts the app in `%LOCALAPPDATA%\Programs\Switchboard`, with a
    Start menu shortcut and an entry in Windows Settings under Apps. To install from a script, refer to
    [Commands](commands.md#the-desktop-installer).

=== "Linux"

    Unpack `Switchboard-linux-x86_64.tar.gz` and run `Switchboard` from the folder. The archive also holds a
    `.desktop` file and an icon for your launcher, the license notices and the server kit.

    The plain download is the app alone. Make it runnable first:

    ```sh
    chmod +x Switchboard-linux-x86_64
    ```

    Nothing is installed outside the folder you unpacked.

### Connect it to your home computer

When it first opens, the app searches your network for a home computer and connects to the one it finds. If it
finds none, it offers two choices:

- **Type an address**: enter your home computer's address, such as `http://192.0.2.10:8130`, and click
  **Connect**. Replace `192.0.2.10` with your home computer's address.
- **Run the server on this computer**: makes this computer the home computer.

For more information, refer to [Install Switchboard](install.md).

## Update the app

The app looks for a new version once a day, through your home computer, and shows **Update ready** at the top of
the window. For more information, refer to [Updates](updates.md).

## Remove the app

=== "macOS"

    Open the disk image again and double-click **Install Switchboard**. When the app is installed, the first
    screen has **Uninstall**, which lists what it removes and what it keeps before it does anything.

=== "Windows"

    Uninstall Switchboard in Windows Settings under Apps, like any other app.

=== "Linux"

    Delete the files you unpacked.

Removing the app doesn't remove the home computer's software or the helper on your computers. Refer to
[Install Switchboard](install.md#uninstalling).

## iPhone {#iphone}

An iPhone app exists, with the board, terminals, History, Files, Library, usage and widgets. You can't get it
today. It isn't on the App Store or TestFlight, there is no download, and its source isn't in the public
repository.

Where these docs mention the iPhone, they apply once you have the app. Away from home, the iPhone app needs a VPN
(virtual private network) to reach your home computer, like the desktop app. Refer to
[Install Switchboard](install.md#away-from-home).

Since version 0.9.0, the iPhone app pairs with the home computer by a device key. You scan a code once, and the
iPhone finds the home computer from any network with no VPN. **Connect by address** stays in the app, and the
widgets still use addresses. Refer to [Pair a phone](features/pair-a-phone.md).

### Set up iPhone notifications {#iphone-notifications}

Your home computer can send notifications to the iPhone app through Apple's push service. It needs an Apple Push
Notification service key from your own Apple Developer account, made for the app's bundle ID. Until the key is in
place, the app says that iPhone notifications aren't set up.

To set up iPhone notifications, on the home computer:

1. Put the key file, `AuthKey_<key id>.p8`, in `~/.config/switchboard/apns/`.
2. Write `~/.config/switchboard/apns.json`:

    ```json
    {
      "key_id": "ABC123DEFG",
      "team": "YOUR_TEAM_ID",
      "topic": "com.example.app",
      "sandbox": true
    }
    ```

    Replace `ABC123DEFG` with the key's id, `YOUR_TEAM_ID` with your Apple team id, and `com.example.app` with the
    app's bundle ID. `sandbox` is `true` for a development build of the app, which is the default, and `false` for
    a release build. Add `"key"` to name a key file that has another name.

3. Restart the home computer's software:

    ```sh
    switchboard restart
    ```

Sending needs the Python `cryptography` package and a `curl` with HTTP/2 on the home computer. If either is
missing, the server's log says `push disabled` and why.

For what the notifications are and what they carry, refer to [Notifications](features/notifications.md).

## Android {#android}

There is no Android app to install: nothing on Google Play and no download. A native app is being written, and its
source isn't in the public repository.

The app being written pairs with the home computer the same way as the iPhone app, by scanning a code, typing it
or opening a link. Refer to [Pair a phone](features/pair-a-phone.md).

## Limitations

- **No phone app to download**: the desktop app is the only app you can install today.
- **Linux is x86_64 only**: there is no build for ARM.
- **The app needs a home computer**: it shows nothing until it's connected to one.
- **Away from home needs a VPN**: the app reaches the home computer only on your own network or over a VPN.
  Device keys, which arrived with version 0.9.0, remove this limit for a paired computer or phone. Refer to
  [Pair a computer](features/pair-a-computer.md).
- **No file picker on some Linux desktops**: without `xdg-desktop-portal`, a terminal can't open a file picker, so
  drag files in instead.

## Troubleshooting

### Windows says "Windows protected your PC"

SmartScreen can stop an installer it hasn't seen often.

1. Check that you downloaded the file from the releases page.
2. Click **More info**, then **Run anyway**.

### The app says "No Switchboard server on this network"

The app searched your network and found no home computer.

1. Check that the home computer is on and on the same network. Run `switchboard status` on it.
2. Click **Search again**.
3. Click **Type an address** and enter the home computer's address.

If it still doesn't connect, refer to [Troubleshooting](troubleshooting.md).
