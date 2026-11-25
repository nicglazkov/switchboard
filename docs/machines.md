---
description: Add each computer where you use Claude Code to your home computer, with a one-time code or over ssh. Rename or remove a computer, read its row on the board, and ask its helper how it is connected.
---

# Add your computers

<p class="sb-meta">Last validated: Nov 22, 2026</p>

Adding a computer puts the helper on it and links it to your home computer, the one of your computers that runs
Switchboard itself. The helper is two small programs: `sb.py`, which reports each chat from inside Claude Code,
and `switchboard-node`, which keeps a link open to the home computer. Once a computer is added, its chats show
on the board.

Switchboard's own screens and commands say "machine" for a computer.

## Two ways to add a computer

<figure class="sb-fig" markdown="0">
--8<-- "docs/diagrams/add-computer.svg"
<figcaption>Figure 1. A computer added with a code connects out to the home computer. A computer added over ssh
is reached the other way.</figcaption>
</figure>

| | With a code | Over ssh |
|---|---|---|
| **What you do** | Paste one line on the computer | Paste one line on the computer, then type its ssh address in setup |
| **The computer needs** | Python 3.9 or newer | Python 3.9 or newer, and an ssh server running |
| **Who connects** | The computer connects out, so it can be behind another router | The home computer logs in to the computer |
| **The home computer needs** | The hub. A Mac or Linux home computer has it | Nothing more. Works on a Windows home computer too |

Use a code unless your home computer runs Windows. Before version 0.9.0 a Windows home computer had no hub.
Since 0.9.0 the installer puts one there too. Refer to [Install Switchboard](install.md#a-windows-server).

## Add a computer with a code

To add a computer with a code:

1. Get a code.
2. Paste the line on the computer.
3. Pick its Claude folder.

### Prerequisites

Before you can add a computer with a code, you must:

- Have a home computer with the hub that is running. Today that is a Mac or Linux home computer. Refer to
  [Install Switchboard](install.md).
- Have Python 3.9 or newer on the computer you are adding.
- Have Claude Code installed there. The installer warns when it isn't, and carries on.

### Get a code

Get a code in any of these places:

- **In the app**: open **Settings**, select **Machines**, then **Add a machine**. The command palette and Files
  have **Add a machine** too.
- **On the home computer**: run `switchboard setup` and choose **Add a machine with a code**.
- **On the home computer, with nothing else asked**: run this command.

```sh
switchboard setup --code
```

Each of them shows a code and one line for each system. A code works once, for 10 minutes.

### Paste the line on the computer

Open a terminal on the computer you are adding, pick the line for its system, and paste it.

=== "macOS"

    ```sh
    curl -fsSL http://192.0.2.10:8131/install-agent.sh | bash -s -- --server 192.0.2.10 --code ABC234
    ```

=== "Linux"

    ```sh
    curl -fsSL http://192.0.2.10:8131/install-agent.sh | bash -s -- --server 192.0.2.10 --code ABC234
    ```

    On a Linux computer without `curl`, setup shows a second line that downloads with Python.

=== "Windows"

    In PowerShell:

    ```powershell
    & ([scriptblock]::Create((iwr -UseBasicParsing http://192.0.2.10:8131/install-agent.ps1).Content)) -Server 192.0.2.10 -Code ABC234
    ```

Copy the line you were shown, not this one. Yours has your home computer's address in place of `192.0.2.10` and
your code in place of `ABC234`.

The line runs the helper's installer, which does four things:

1. Checks for Python 3.9 or newer.
2. Downloads `sb.py` and `switchboard-node` from your home computer, on port 8131, and checks each against the
   home computer's list of SHA-256 sums.
3. Sets `sb.py` up in Claude Code's settings. A helper that is already there is kept, unless you add `--update`.
4. Links the computer and sets `switchboard-node` to start when you log in: a LaunchAgent on a Mac, a systemd
   user service on Linux, a logon task on Windows. None of them needs an administrator.

The computer shows on the board within a minute.

The line the app shows is shorter. It downloads `switchboard-node` from your home computer, or from the newest
release on GitHub when the home computer has none for that system, and fetches `sb.py` only when the computer
has none.

!!! note ""
    To read the installer before you run it, download `http://192.0.2.10:8131/install-agent.sh`, or
    `install-agent.ps1` for Windows, and run the file yourself. Refer to
    [Commands](commands.md#install-agentsh-and-install-agentps1).

### Pick its Claude folder

Each computer has a Claude folder, where new chats and the folder picker start. Setup asks for it once the
computer is linked. In the app, open **Settings**, select **Machines**, open the computer, and use **Choose**
or **Type a path**.

For more information, refer to [Places and folders](concepts/places-and-folders.md).

### The home computer itself

Installing Switchboard doesn't add the home computer to its own board. If you use Claude Code there too, paste
the line there like on any other computer. Since version 0.9.0, the installer does this for you, and
`switchboard link` does it at any time.

### When the address changes

A computer added with a code keeps the one address from the line you pasted. If your home computer's address
changes, or the computer moves to a network where that address doesn't lead home, add it again with a new code.
For a laptop that leaves home, use an address that works everywhere. Refer to
[Install Switchboard](install.md#away-from-home).

## Add a computer over ssh {#over-ssh}

Over ssh, the home computer logs in to the computer with a key that can run the helper and nothing else.

### Prerequisites

Before you can add a computer over ssh, you must have an ssh server running on it:

- **macOS**: open System Settings, select **General**, then **Sharing**, and turn on **Remote Login**.
- **Linux**: install `openssh-server`.
- **Windows**: turn on the OpenSSH Server feature.

### Add it in setup

1. On the home computer, run `switchboard setup` and choose **Add a machine over ssh**.
2. Paste the line setup shows on the computer you are adding. It installs the helper and adds the home
   computer's key to the computer's `authorized_keys`.
3. Back in setup, type the computer's ssh address, such as `me@example-mac.local`.

The key is `~/.ssh/switchboard_ed25519.pub` on the home computer. The line adds it with a `command=` that runs
the helper only, with no shell and no port forwarding. For an administrator account, Windows reads keys from
`%ProgramData%\ssh\administrators_authorized_keys`, so the installer puts the key there.

To do the same from a script, with no questions:

```sh
switchboard setup --ssh example-mac=me@example-mac.local --root example-mac=~/Code
```

Replace `example-mac` with a name for the computer, `me@example-mac.local` with its ssh address, and `~/Code`
with its Claude folder.

## Add a computer with device keys

!!! note ""
    Device keys arrived with Switchboard 0.9.0 and are off until you turn them on.

With device keys, a computer joins by a short code or a link, not by an address. It then finds the home computer
from any network. On the home computer, turn keys on once and make a code. The desktop app does both in
**Settings**, **Devices**, with **Turn keys on** and **Add a device**. The commands are:

```sh
switchboard keys on
```

```sh
switchboard code
```

On a computer that already has the helper, enter the code:

```sh
switchboard-node pair K7PM-2XQ9
```

Replace `K7PM-2XQ9` with your code, or with the whole link in quotes. A new computer gets the helper and joins
in one step with the installer's `--pair` flag. On a computer with the desktop app, type the code under **Join
your home computer** the first time the app starts.

A phone pairs with the same code, by scanning it. The phone apps that pair are not released yet.

For more information, refer to [Pair a computer](features/pair-a-computer.md),
[Pair a phone](features/pair-a-phone.md) and [Install Switchboard](install.md#with-device-keys).

## What the board shows for a computer

Each computer is one row on the board. The label at the left of its row shows:

- **Its name**: the one you gave it, or its own name.
- **Its system and chat count**: for example "macOS, 4 chats". A computer that can't be reached says
  **Offline**. Its chats come back when it does.
- **Memory**: how much memory Claude is using there.
- **What it can do**: the tools the computer reports, when it reports any.
- **New terminal**: starts a new chat there.

A computer that hasn't reported for 45 seconds shows its chats in gray, with a note such as "As of 2m ago".

For more information, refer to [The board](board.md#a-computers-row).

## Rename a computer

1. Open **Settings** and select **Machines**.
2. Open the computer.
3. Select **Rename** and type the new name.

The board shows the new name at once. The computer's **Id** never changes, so its chats and history stay with
it.

## Remove a computer {#remove-a-computer}

Removing a computer takes two steps: one in the app, and one on the computer.

### Remove it from the board

1. Open **Settings** and select **Machines**.
2. Open the computer.
3. Select **Remove this machine**, and select it again to confirm.

Switchboard stops watching the computer. Its chats and files stay on it. A computer that only lost its link is
not removed: it stays in the list, offline, with its settings, until you remove it or add it again.

### Remove the helper from the computer

On the computer, take `sb.py` out of Claude Code's settings:

```sh
python3 ~/.claude/switchboard/sb.py uninstall
```

This removes only what the helper added. The folder `~/.claude/switchboard` stays, so delete it if you don't
want it. A computer added with a code also has `switchboard-node`. Remove it with:

```sh
switchboard-node uninstall
```

This stops it and removes its login item, the program and its settings, then prints what it removed.

Since version 0.9.0, `switchboard uninstall` does both at once. Refer to
[Install Switchboard](install.md#uninstalling).

## Ask the helper how it is connected

On a computer added with a code, run:

```sh
switchboard-node status
```

It answers with the name the computer has on the board and the home computer it is linked to:

```text
Linked as example-mac to the hub at 192.0.2.10:8132.
sb.py: /Users/you/.claude/switchboard/sb.py  python: python3
```

A computer that was never linked says so, and tells you to get a code.

Since version 0.9.0, the same command also says whether keys are on, which home computer this one is paired
with, whether the connection is direct or through a relay, and one line that starts with `Problem:` for each
thing that is wrong. Refer to [Pair a computer](features/pair-a-computer.md#troubleshooting).

## Limitations

- **One address for each computer**: a computer added with a code keeps the address it was added with.
- **A Windows home computer adds over ssh only**: this holds for one installed before version 0.9.0, which has no hub. Since
  0.9.0 the installer puts the hub on Windows too.
- **A Windows computer returns at sign-in**: its helper is a logon task, so after a restart it comes back when
  someone signs in to Windows.
- **Removing is two steps**: the app can't take the helper off a computer for you.

## Troubleshooting

For a computer that doesn't show up, goes offline or stops connecting, refer to
[Troubleshooting](troubleshooting.md#computers).
