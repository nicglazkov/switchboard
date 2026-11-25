---
description: Install Switchboard on your home computer, check that it runs, update it and remove it. What the installer sets up, how to reach the board away from home, and what a Windows home computer can do.
---

# Install Switchboard

<p class="sb-meta">Last validated: Nov 22, 2026</p>

Installing Switchboard means setting up one of your computers as the home computer, the one that runs Switchboard
itself and keeps the board. One command does it, for your account only and with no administrator password.

Switchboard has three parts, and this page covers the first:

- **The home computer's software**: installed once, on a computer that stays on. This page.
- **The helper**: on each computer where you use Claude Code. Refer to [Add your computers](machines.md).
- **The app**: on each computer you look at the board from. Refer to [The apps](apps.md).

To go through all three in one sitting, refer to [Quickstart](quickstart.md).

## What you need

- **The home computer**: macOS or Linux, with Python 3.9 or newer. Windows works
  [with less](#a-windows-server). It doesn't need Claude Code.
- **Each computer with Claude Code**: macOS, Linux or Windows, with Python 3.9 or newer, and Claude Code
  installed and signed in.
- **The app**: macOS 11 or newer, Windows, or Linux x86_64. It needs neither Python nor Claude Code.

A Mac has Python once the command line tools are installed, which `xcode-select --install` does. Most Linux
systems come with it. On Windows, run `winget install -e --id Python.Python.3.12`.

Pick a home computer that stays on: a Mac mini on a shelf, a desktop that never sleeps, a small Linux computer.
With one computer only, that computer is the home computer too. On a laptop, the board is out of reach for every
other device while the laptop sleeps.

!!! note ""
    Keep the home computer on a network that only you can reach, such as your home network or Tailscale. This is
    because Switchboard has no passwords: anyone who can reach its ports can see and run your chats. Never
    forward the ports to the internet. Refer to [Privacy and security](privacy.md#no-passwords-on-purpose).

## Install the home computer

To install the home computer:

1. Run the installer.
2. Answer setup's questions.
3. Open the app.

### Prerequisites

Before you can install the home computer, you must:

- Have Python 3.9 or newer on it.
- Have ports 8130, 8131 and 8132 free on it. The installer checks, and names the port that is taken.

### Run the installer

=== "macOS"

    Open Terminal on the home computer and run:

    ```sh
    curl -fsSL https://github.com/nicglazkov/switchboard/releases/latest/download/get-server.sh | bash -s -- --kit https://github.com/nicglazkov/switchboard/releases/latest/download/switchboard-server-kit.tar.gz
    ```

=== "Linux"

    Open a terminal on the home computer and run:

    ```sh
    curl -fsSL https://github.com/nicglazkov/switchboard/releases/latest/download/get-server.sh | bash -s -- --kit https://github.com/nicglazkov/switchboard/releases/latest/download/switchboard-server-kit.tar.gz
    ```

    Linux stops your services when you log out. To keep Switchboard running, run this once:

    ```sh
    sudo loginctl enable-linger $USER
    ```

=== "Windows"

    There is no one-line install for Windows. In PowerShell, with git, Python 3.9 or newer and the OpenSSH client
    installed, run these three lines one after another:

    ```powershell
    git clone https://github.com/nicglazkov/switchboard.git
    cd switchboard
    powershell -ExecutionPolicy Bypass -File scripts\install\install-server.ps1
    ```

    A Windows home computer installed this way does less than a Mac or Linux one. Refer to
    [A Windows home computer](#a-windows-server).

The line downloads the server kit from the newest release, checks its SHA-256, and runs the installer from it.
`--kit` says where the kit is. Without it, the released `get-server.sh` stops with "no server kit to install
from".

In a terminal, the installer first checks the computer: its system, Python, the three ports, disk space, and
whether it runs on a battery. Then it shows each step and how long it took. It keeps the full output in
`~/.local/share/switchboard/logs/install.log` and shows a step's last lines when that step fails.

You can also install from two other places:

- **From the desktop app**: on a Mac, or on Linux with the app from the `.tar.gz`, open the app. When it finds no
  home computer on the network, choose **Run the server on this computer**. The app installs from the copy it
  carries, as you, and then connects to `http://localhost:8130`. This installs the home computer's software
  only, so add each computer afterwards, this one included.
- **From a copy of the repository**: clone it and run `scripts/install/install-server.sh`. Without a ready-made
  hub, the installer builds one with [Rust](https://rustup.rs), or installs without it when Rust is missing.

### Answer setup's questions

First-run setup starts when the installer is done. It asks three things:

1. **The home computer's address**: what the app and your other computers use to reach it. Setup lists the
   addresses it found: the computer's address on your network, its Tailscale name when Tailscale is running, and
   its own name. Pick one, or type another. The Tailscale name works away from home too.
2. **Your computers**: add each computer where you use Claude Code, with a one-time code or over ssh. Refer to
   [Add your computers](machines.md).
3. **Each computer's Claude folder**: where new chats start. Setup lets you browse the computer's folders.

To run setup again later, run:

```sh
switchboard setup
```

### Open the app

Install the app on the computers you look at the board from. On the same network as the home computer, the app
finds it and connects by itself. Otherwise it asks for the address you picked in setup. Refer to
[The apps](apps.md).

Once connected, the home computer tells the app every address it has, and the app keeps them. An app that
connected once at home therefore also connects over Tailscale, with nothing to type.

## What the installer sets up

<figure class="sb-fig" markdown="0">
--8<-- "docs/diagrams/install-parts.svg"
<figcaption>Figure 1. The installer starts three services on the home computer. Each kind of device talks to one
of them.</figcaption>
</figure>

The installer changes nothing outside your own account. It puts in place:

- **The server**: in `~/.local/share/switchboard/app`. It keeps the board and answers the apps on port 8130.
- **The downloads**: on port 8131. A computer you add fetches the helper from here, not from the internet.
- **The hub**: `~/.local/bin/switchboard-hub`, on port 8132. It carries chat terminals and the links from your
  computers.
- **Three services**: one for each part, started when you log in. They are LaunchAgents on a Mac and systemd
  user services on Linux.
- **The `switchboard` command**: in `~/.local/bin`. Refer to [Commands](commands.md).

The figure doesn't draw a second use of the hub: the app also connects to port 8132 when you open a chat's
terminal.

### Since version 0.9.0

Since version 0.9.0 the installer does more by itself:

- **Its own Python**: the installer brings Python 3.13 for Switchboard's own use, in
  `~/.local/share/switchboard/python`, so the home computer and the computers you add need no Python of their
  own. A computer's own Python 3.9 or newer is used only where that build doesn't run.
- **The home computer adds itself**: on the first install, the installer sets up the helper on the home computer
  and links it, so its own chats show on the board with nothing to paste. `--no-link` skips this.
- **One command to link it later**: `switchboard link` does the same at any time.

### Installer options {#installer-options}

Flags for the installer go at the end of the line, after the `--kit` address.

| Flag | What it does |
|---|---|
| `--hub-binary PATH` | Uses this `switchboard-hub` in place of a ready-made or built one |
| `--node-dir DIR` | Takes the `switchboard-node` builds for other systems from this folder, to hand to computers you add |
| `--no-hub` | Installs without the hub: no terminals and no adding a computer with a code |
| `--no-service` | Sets up no services, and starts plain background processes |
| `--no-start` | Installs, and starts nothing |
| `--no-setup` | Skips first-run setup |
| `--yes`, `-y` | Asks nothing, and skips first-run setup |
| `--plain` | Prints plain lines with no colors or spinner |
| `--uninstall` | Removes the home computer's software. Refer to [Uninstall](#uninstalling) |

`get-server.sh` itself takes `--kit URL|FILE` and `--sha256 HEX`. Every other flag goes to the installer.

### Host your own kit {#host-your-own-kit}

The server kit is one archive with everything the installer needs. Each release has one. To make your own, run
this in a copy of the repository:

```sh
scripts/install/pack-server-kit.sh --base https://kit.example.com/switchboard
```

Replace `https://kit.example.com/switchboard` with the folder you will serve the files from. The script writes
three files to `dist/server-kit/`: the kit, its `.sha256` file and a `get-server.sh` that downloads from your
folder. Put all three there, then install with:

```sh
curl -fsSL https://kit.example.com/switchboard/get-server.sh | bash
```

With no web server at all, copy the kit and `get-server.sh` to the new computer and run
`bash get-server.sh --kit switchboard-server-kit.tar.gz`.

## Check that it works

On the home computer, run:

```sh
switchboard status
```

It answers with one line for each part:

```text
Switchboard on this machine (launchd)
  server     http://127.0.0.1:8130  running
  hub        http://127.0.0.1:8132  running
  downloads  http://127.0.0.1:8131  running
```

The word in brackets is how the parts run: `launchd` on a Mac, `systemd` on Linux, or `plain` for background
processes. A part that says `not answering` has stopped. To see why, run:

```sh
switchboard logs
```

For more information, refer to [Troubleshooting](troubleshooting.md).

## Update

Each part updates in its own way:

- **The app** offers a new version by itself and installs it when you say so.
- **The home computer** updates from the app: open **Settings**, select **Server**, and select **Update
  server**. Running the install line again does the same. Your computers, settings and history stay.
- **The helper** on each computer updates with one command.

For more information, refer to [Updates](updates.md).

## Uninstall {#uninstalling}

To remove the home computer's software, run the install line with `--uninstall` at the end:

```sh
curl -fsSL https://github.com/nicglazkov/switchboard/releases/latest/download/get-server.sh | bash -s -- --kit https://github.com/nicglazkov/switchboard/releases/latest/download/switchboard-server-kit.tar.gz --uninstall
```

From a copy of the repository, `scripts/install/install-server.sh --uninstall` does the same. On Windows, run
`powershell -ExecutionPolicy Bypass -File scripts\install\install-server.ps1 -Uninstall`.

This stops and removes the three services, the server, the downloads, the hub and the `switchboard` command. It
keeps your settings, history and logs:

- **macOS and Linux**: delete `~/.config/switchboard` and `~/.local/share/switchboard` if you don't want them,
  and `~/Library/Logs/Switchboard` on a Mac.
- **Windows**: the uninstall removes `%LOCALAPPDATA%\Switchboard`, logs included. It keeps
  `%USERPROFILE%\.config\switchboard` and `%USERPROFILE%\.local\share\switchboard`. A firewall rule you added
  stays until you remove it.

A helper on the home computer that is linked to this same home computer is removed with it. A helper that is
linked to another home computer is left alone, and the uninstall says so.

The rest stays on each computer until you remove it there:

- **The helper**: refer to [Remove a computer](machines.md#remove-a-computer).
- **The app**: refer to [The apps](apps.md).

### One command, since version 0.9.0

Version 0.9.0 added one command that removes everything Switchboard put on a computer:

```sh
switchboard uninstall
```

It takes the helper out of Claude Code, removes `switchboard-node`, the home computer's software and
Switchboard's own Python, and then lists what it removed and what it left. It asks before it starts.

Common flags:

- `--purge`: also deletes your settings, history and logs, and this computer's device key and list of paired
  devices. The default is to keep them, so that installing again needs no pairing again.
- `--yes`, `-y`: asks nothing.

## Away from home {#away-from-home}

Today, the app reaches your home computer from another network only over a VPN (virtual private network). The
ports must never be forwarded to the internet, so a VPN is the only way.

With [Tailscale](https://tailscale.com):

1. Install Tailscale on the home computer and on each device you take with you, signed in to the same account.
2. Connect the app once at home. The home computer passes on its Tailscale address and name, and the app keeps
   them. You can also type the name in **Settings**, **Server**, **Server addresses**.
3. Away from home, turn Tailscale on on the device.

A computer added with a code keeps the one address from the line you pasted. For a laptop that runs chats and
leaves home, pick the Tailscale name in `switchboard setup` before you add it.

Since version 0.9.0, device keys remove the need for a VPN. A paired computer or phone finds the home computer
by its key from any network, directly where it can and through a relay where it can't. The phone apps that pair
are not released yet. Refer to [Pair a computer](features/pair-a-computer.md) and
[Pair a phone](features/pair-a-phone.md).

## With device keys

!!! note ""
    Device keys arrived with Switchboard 0.9.0 and are off until you turn them on.

With device keys, a computer joins your home computer with a short code or a link, and there is no address to
type. The installers take two more flags for this. You can also turn keys on later, from the desktop app or with
`switchboard keys on`. Refer to [Pair a computer](features/pair-a-computer.md#turn-keys-on).

### Install a home computer with keys on

Add `--keys` to the installer's flags, or `-Keys` on Windows:

=== "macOS"

    ```sh
    scripts/install/install-server.sh --keys
    ```

=== "Linux"

    ```sh
    scripts/install/install-server.sh --keys
    ```

=== "Windows"

    ```powershell
    powershell -ExecutionPolicy Bypass -File scripts\install\install-server.ps1 -Keys
    ```

    On Windows, keys need the hub, so the copy you install from must carry `switchboard-hub.exe`. Refer to
    [A Windows home computer](#a-windows-server).

The installer installs as usual, links the home computer to itself, turns keys on and prints a code and a link
for the next computer. A code needs a relay, so without one the installer prints a link only. The code and the
link each work once, for ten minutes. For another, run:

```sh
switchboard code
```

### Add a computer with a code or a link

On the computer you are adding, run the helper's installer with `--pair`:

=== "macOS"

    ```sh
    bash install-agent.sh --pair K7PM-2XQ9 --node ./switchboard-node-mac
    ```

=== "Linux"

    ```sh
    bash install-agent.sh --pair K7PM-2XQ9 --node ./switchboard-node-linux
    ```

=== "Windows"

    ```powershell
    powershell -ExecutionPolicy Bypass -File install-agent.ps1 -Pair K7PM-2XQ9 -Node .\switchboard-node.exe
    ```

Replace `K7PM-2XQ9` with your code, or with the whole link in quotes.

Common flags:

- `--node PATH` (`-Node`): the `switchboard-node` program to install. `--pair` needs either this or `--server`.
  This is because the helper has no public download address yet.
- `--server ADDR` (`-Server`): downloads `switchboard-node` from a home computer that still answers by address.
- `--relay URL` (`-Relay`): the relay to find the home computer through, when a short code needs one and the
  computer has none set.

A Windows PC can join a home computer this way.

## A Windows home computer {#a-windows-server}

Today, a Windows home computer reaches your computers over ssh only. The hub, which carries terminals and adding
a computer with a code, isn't installed on Windows. If Windows is all you have, that means:

- **No terminals**: the board works, but a chat's terminal doesn't open in the app.
- **Every computer is added over ssh**: the home computer's own PC included, so each one needs an ssh server
  running. On Windows that is the OpenSSH Server feature. Refer to [Add your computers](machines.md#over-ssh).
- **Two logon tasks, not three services**: the server on port 8130 and the downloads on port 8131. They start
  when you sign in to Windows, not when the PC starts.
- **A firewall rule to add**: the installer prints a rule for ports 8130 and 8131 to run as administrator.
- **No update from the app**: update it by hand. Refer to [Updates](updates.md#the-server).

`install-server.ps1` takes `-NoStart`, `-NoSetup` and `-Uninstall`. The `switchboard` commands are in a
PowerShell script on Windows. Refer to [Commands](commands.md).

If you can, put the home computer on a Mac or a Linux computer. Any Windows PC can still be one of your
computers.

### Since version 0.9.0

Version 0.9.0 installs the hub on Windows too, when the copy you install from carries
`native\dist\switchboard-hub.exe`. The server kit carries it. A plain copy of the repository doesn't, and the
installer then says "This copy has no switchboard-hub.exe (native\dist), so there is no hub".

With the hub in place, the installer says "The hub is here (terminals, and linking machines with a code)", and a
Windows home computer gains:

- **A third logon task**: `SwitchboardHub`, on port 8132, beside the server and the downloads.
- **Its own chats on the board**: the PC links itself to its own server, as a Mac or Linux home computer does.
- **Adding a computer with a code**: no ssh server needed on the computers you add.
- **Device keys**: `-Keys` at install, or `keys on` later. Other computers and phones can pair with it.

The firewall rule the installer prints then lists port 8132 as well. Computers that joined with a key need no
rule.

Since version 0.9.0, `install-server.ps1` takes these flags:

| Flag | What it does |
|---|---|
| `-HubBinary PATH` | Uses this `switchboard-hub.exe` in place of the one in `native\dist` |
| `-NodeDir DIR` | Takes the `switchboard-node` builds for the downloads from this folder first |
| `-NoHub` | Skips the hub: no terminals, and computers are added over ssh |
| `-NoLink` | Doesn't link this PC to its own server |
| `-Keys` | Turns keys on at the end and prints a code for the next computer |
| `-NoStart` | Installs, and starts nothing |
| `-NoSetup` | Skips first-run setup |
| `-NoTasks` | Registers no logon tasks, so nothing starts when you sign in |
| `-Uninstall` | Removes the home computer's software. Settings and history are kept |

One thing stays different on Windows. The desktop app can't turn keys on or close the network port for a Windows
home computer. This is because the server can't restart itself there. The app shows the PowerShell command to
run on the home computer in its place.
