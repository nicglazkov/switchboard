---
description: Fixes for the problems people hit setting up and running Switchboard, from installer messages to a computer that stops answering.
---

# Troubleshooting

Start with `switchboard status` on the server. It says whether the server, the hub and the downloads are running,
and `switchboard logs` shows the last lines of each. On a Windows server, use
`powershell -File %LOCALAPPDATA%\Switchboard\switchboard.ps1 status` (and `logs`, `setup`, `start`) instead.

## Installing

??? failure "Python 3 is not installed"
    The server and the helper on each computer need Python 3.9 or newer. Install it with the command the script
    shows, then run the script again.

??? failure "Rust is not installed, so the hub can't be built here, and there is no ready-made one"
    The server works without the hub, but with no terminals and no linking computers with a code, so computers are
    added over ssh. Install the server from a [release](install.md), which brings a ready-made hub, or install Rust
    from [rustup.rs](https://rustup.rs) and run the installer again.

??? failure "The server has no switchboard-node for mac (or Windows) yet"
    The server holds the link program only for systems it has one for. Use the line from **Add a machine** in the
    app instead: when the server doesn't have the program, that line downloads it from the latest GitHub release.
    Or give the server all three with `install-server.sh --node-dir <folder>` (a folder with
    `switchboard-node-mac`, `switchboard-node-linux` and `switchboard-node.exe` from a release). Or add that
    computer [over ssh](machines.md).

??? failure "That code is wrong or has expired"
    A code works once, for 10 minutes. Get a new one: **Settings, Machines, Add a machine** in the app, or
    `switchboard setup --code` on the server.

??? failure "Ports in use"
    The server needs 8130, 8131 and 8132. If something else holds one, the installer says which; stop it, or run
    the server on another computer. A Switchboard already there is fine: it restarts.

??? failure "Windows protected your PC"
    The Windows installer isn't code signed yet. Click **More info**, then **Run anyway**.

## Connecting

??? failure "The app can't find the server"
    The app looks for it on the local network. If it isn't on the same network, or the network blocks that,
    type its address: `http://<server>:8130`. Check that a firewall allows ports 8130 to 8132 from that device.
    On a Windows server, the installer prints a firewall rule for 8130 and 8131 (it has no hub); run it in an
    administrator PowerShell. Run `switchboard setup` on
    the server to pick the address it gives out.

    Once connected, the app saves every address the server knows, so it still connects from elsewhere when the
    server is reachable there. **Settings, Machines, Server addresses** lists them.

??? failure "A computer is offline, or not answering"
    Its chats come back when it does. On that computer:

    - **Linked with a code:** `switchboard-node status` says which server it's linked to. Its log is
      `~/Library/Logs/switchboard-node.log` on a Mac, `journalctl --user -u switchboard-node` on Linux with
      systemd, and `~/.local/share/switchboard/node.log` on Linux without it.
    - **Added over ssh:** check that its ssh server is on and that `ssh <computer>` works from the server.
    - **Either way:** the helper's log is `~/.claude/switchboard/sb.log`.

??? failure "Cards are gray and say “As of 2m ago”"
    That computer hasn't reported for 45 seconds, so Switchboard shows what it last knew. If it doesn't come back,
    see the item above.

??? failure "No restart on Linux without systemd"
    In a container, or WSL without systemd, the installers start everything in the background instead. After a
    restart, start it again: `switchboard start` on the server, `~/.local/bin/switchboard-node run &` on a
    computer.

## Using it

??? failure "A message I typed didn't go through"
    It's probably held: the chat is big and cold, so the message would read all of it again. The terminal shows
    what it would cost. Send it again or type `go` within 15 minutes, or type `/clear` to start fresh.
    [The guard](chats.md#the-guard-for-big-cold-chats)

??? failure "Files says No permission"
    macOS asks before an app reads Desktop, Documents or Downloads. Turn on the switch for that folder on that Mac,
    in **System Settings, Privacy & Security, Files and Folders**. Files has a button that opens it.

??? failure "Every message is blocked with “Switchboard hard limit”"
    The budget's hard limit was reached. **Lift for today** in **Settings, Budget**, or wait until tomorrow.
    [Hard limit](usage.md#hard-limit)

??? failure "An update check says it couldn't look"
    The message says why, like "GitHub didn't answer", or "The download page didn't answer" when your server
    uses its own download page. The server does the looking, so check that it can reach the internet.
    **Check now** in **Settings, Updates** tries again. [Updates](updates.md)

??? failure "The terminal looks garbled"
    **Redraw** (++cmd+shift+r++, or ++ctrl+shift+r++ on Windows and Linux) asks for a fresh copy of the screen.

??? failure "The Linux app won't start"
    It needs a Vulkan driver (Mesa or your GPU vendor's), libxkbcommon, and X11 or Wayland. Start it from a
    terminal to see what's missing.

## Still stuck

[Get help](support.md): open an issue with **Copy diagnostics** from About Switchboard.
