---
description: Every command Switchboard installs, on the home computer and on each computer you add, with its synopsis, its flags and their defaults.
---

# Commands

<p class="sb-meta">Last validated: Nov 22, 2026</p>

This page lists the commands that Switchboard installs. Most people never need them, because the app's Settings
do the same things. The commands and their output say server for the home computer, and machine for a computer.

Commands that arrived with version 0.9.0 are listed apart, under
[New in version 0.9.0](#new-in-version-090).

## On the home computer

The installer adds the `switchboard` command to `~/.local/bin` on a Mac or Linux home computer. It works the same
whichever way the home computer's software runs: as LaunchAgents on a Mac, as systemd user services on Linux, or
as plain background processes when neither is set up.

On a Windows home computer the same commands are in a PowerShell script. Run it as
`powershell -File %LOCALAPPDATA%\Switchboard\switchboard.ps1`, followed by `start`, `stop`, `restart`, `status`,
`logs` or `setup`.

### switchboard status

Says whether each part is running: the server on port 8130, the hub on port 8132 and the downloads on port 8131.

```sh
switchboard status
```

`switchboard` with nothing after it does the same. On Windows the hub line says that the hub isn't available. For
more information, refer to [Ports, files and settings](reference.md).

### switchboard start

Starts the server, the hub and the downloads, then prints their status.

```sh
switchboard start
```

### switchboard stop

Stops the server, the hub and the downloads. Chats keep running on their own computers.

```sh
switchboard stop
```

### switchboard restart

Stops the three parts, starts them again and prints their status. Restart after you edit `server.json`.

```sh
switchboard restart
```

### switchboard logs

Prints the last lines of each part's log.

```sh
switchboard logs [LINES]
```

Common flags:

- `LINES`: how many lines to print. The default is 60 lines from the journal on Linux with systemd, and 20 lines
  from each log file otherwise. On Windows, `logs` always prints 30 lines of the server's and the downloads' logs.

For more information, refer to [Get help](support.md).

### switchboard setup

Runs first-run setup again. With no flags it asks for three things in turn: the home computer's address, your
computers, and each computer's Claude folder.

```sh
switchboard setup [--address ADDR] [--code [--json]] [--ssh ID=USER@HOST] [--root ID=FOLDER] [--ssh-key] [--yes]
```

Common flags:

- `--address ADDR`: the address your devices use for the home computer. Skips the first question.
- `--code`: prints a one-time code and the lines to paste on a new computer, then stops. A code works once, for
  10 minutes. Add `--json` to print the same as JSON.
- `--ssh ID=USER@HOST`: adds a computer that is reached over ssh. Repeat it for more computers.
- `--root ID=FOLDER`: sets a computer's Claude folder. Repeat it for more computers.
- `--ssh-key`: prints the home computer's ssh public key, and makes the key the first time.
- `--yes`, `-y`: takes the suggested answer everywhere and asks nothing. The default is to ask.

Setting `NO_COLOR` or `SWITCHBOARD_PLAIN` in the environment turns colors off. For more information, refer to
[Install Switchboard](install.md) and [Add your computers](machines.md).

## Installers

### get-server.sh

Installs the home computer's software on a Mac or Linux computer that has no copy of the repository. It downloads
the server kit, checks its SHA-256, unpacks it and runs `install-server.sh` from it.

```sh
curl -fsSL https://github.com/nicglazkov/switchboard/releases/latest/download/get-server.sh | bash -s -- --kit https://github.com/nicglazkov/switchboard/releases/latest/download/switchboard-server-kit.tar.gz
```

To pass flags to a piped script, add `-s --` after `bash`, then the flags.

Common flags:

- `--kit URL|FILE`: the kit to install. The default is `SWITCHBOARD_KIT_URL` when that is set, or the address
  written into the copy of `get-server.sh` you run. The copy in the newest release has none written in, so pass
  `--kit` as shown.
- `--sha256 HEX`: the SHA-256 the kit must have. The default is the `.sha256` file beside the kit, when there is
  one.
- Any other flag is passed on to `install-server.sh`.

For more information, refer to [Install Switchboard](install.md).

### install-server.sh

Installs or removes the home computer's software on a Mac or Linux computer, from a copy of the repository or of
the server kit. It doesn't use `sudo`.

```sh
scripts/install/install-server.sh [--no-hub] [--no-service] [--no-start] [--no-setup] [--yes] [--plain] [--uninstall]
```

Common flags:

- `--hub-binary PATH`: uses this `switchboard-hub`. The default is the ready-made one in the kit, or one built on
  the computer.
- `--node-dir DIR`: a folder with `switchboard-node-mac`, `switchboard-node-linux` and `switchboard-node.exe` to
  hand to computers you add.
- `--no-hub`: skips the hub. Without it there are no chat terminals and no adding a computer with a code. Off by
  default.
- `--no-service`: doesn't register services, and starts plain background processes. Off by default.
- `--no-start`: installs only. Off by default.
- `--no-setup`: doesn't run first-run setup at the end. Off by default.
- `--yes`, `-y`: asks nothing. Setup is skipped.
- `--plain`: prints plain lines with no colors or spinner. This is also what you get when the output is piped.
- `--uninstall`: stops and removes what the installer put in place. Your settings and history are kept.

For more information, refer to [Install Switchboard](install.md#installer-options).

### install-server.ps1

Installs or removes the home computer's software on a Windows PC, from a copy of the repository or of the server
kit. It needs no administrator rights.

```powershell
powershell -ExecutionPolicy Bypass -File scripts\install\install-server.ps1 [-NoStart] [-NoSetup] [-Uninstall]
```

Common flags:

- `-NoStart`: installs only. Off by default.
- `-NoSetup`: doesn't run first-run setup at the end. Off by default.
- `-Uninstall`: stops and removes what the installer put in place. Your settings and history are kept.

For more information, refer to [Install Switchboard](install.md#a-windows-server).

### pack-server-kit.sh

Makes the server kit that `get-server.sh` downloads. Run it in a copy of the repository. It writes the kit, its
`.sha256` file and a copy of `get-server.sh`.

```sh
scripts/install/pack-server-kit.sh [--base URL] [--out DIR] [--no-binaries] [--no-sources] [--no-vendor]
```

Common flags:

- `--base URL`: the folder the three files will be served from. The `get-server.sh` it writes then downloads
  the kit from that folder by default.
- `--out DIR`: where to write the files. The default is `dist/server-kit`.
- `--no-binaries`: leaves out the ready-made `switchboard-hub` and `switchboard-node` builds.
- `--no-sources`: leaves out the Rust sources.
- `--no-vendor`: leaves out the bundled `python-zeroconf`.

For more information, refer to [Install Switchboard](install.md#host-your-own-kit).

### install-agent.sh and install-agent.ps1

Installs the helper on a computer that runs Claude Code and links that computer to your home computer. The home
computer serves both scripts on port 8131, and `switchboard setup --code` prints the full line to paste.

```sh
curl -fsSL http://192.0.2.10:8131/install-agent.sh | bash -s -- --server 192.0.2.10 --code EXAMPLE-CODE
```

Replace `192.0.2.10` with your home computer's address and `EXAMPLE-CODE` with the code. On Windows, each flag has
a PowerShell form.

Common flags:

- `--server ADDR` (`-Server`): the home computer's address or name.
- `--code CODE` (`-Code`): the one-time code. With a code, the installer also installs `switchboard-node`.
- `--ssh-key "KEY"` (`-SshKey`): lets the home computer in over ssh with this public key, limited to the helper.
- `--name ID` (`-Name`): the computer's id, in lowercase letters and digits. The default comes from the
  computer's own name.
- `--label NAME` (`-Label`): the name the board shows. The default is the computer's own name.
- `--sb PATH` (`-Sb`): installs this `sb.py`. The default is to download it from the home computer.
- `--node PATH` (`-Node`): installs this `switchboard-node`. The default is to download it from the home computer.
- `--update` (`-Update`): replaces a helper that is already installed. The default is to keep it.

For more information, refer to [Add your computers](machines.md) and [Updates](updates.md).

## On each computer: switchboard-node

`switchboard-node` is the program that links a computer you added with a code to your home computer. It is
installed in `~/.local/bin`, or in `%LOCALAPPDATA%\Switchboard` on Windows. Its settings are in
`~/.config/switchboard/node.json`.

### switchboard-node pair

Links this computer with a one-time code and saves the link in `node.json`.

```sh
switchboard-node pair HUB CODE [--name NAME] [--label LABEL]
```

Common flags:

- `HUB`: the home computer's address. Port 8132 is used when you give none.
- `--name NAME`: the computer's id. The default comes from the computer's own name.
- `--label LABEL`: the name the board shows. The default is the computer's own name.

### switchboard-node install

Copies the program into place, sets it to run when you sign in and starts it. The computer must be paired first.

```sh
switchboard-node install
```

### switchboard-node run

Keeps the link to the home computer open. This is what runs when you sign in.

```sh
switchboard-node run
```

### switchboard-node status

Says which computer this is linked as and to which home computer.

```sh
switchboard-node status
```

### switchboard-node uninstall

Stops the program, takes it out of what starts it when you sign in, and removes the program and `node.json`. It
prints what it removed. The computer stays in **Settings**, **Machines** until you remove it there.

```sh
switchboard-node uninstall
```

### switchboard-node --version

Prints the program's version.

```sh
switchboard-node --version
```

For more information, refer to [Add your computers](machines.md).

## On each computer: sb.py, the helper {#sbpy-the-helper}

The helper is the file `~/.claude/switchboard/sb.py`. Run it with `python3`, or with `python` on Windows. Claude
Code and the home computer run its other commands, such as `hook`, `statusline`, `rpc` and `mcp`, by themselves.

### sb.py install

Adds the helper to Claude Code: its hooks and status line in `~/.claude/settings.json`, and the `switchboard` MCP
(Model Context Protocol) server in `~/.claude.json`. It makes a dated copy of the settings file first and saves
the status line you had. On Windows it also adds a logon task named `SwitchboardLaunch`.

```sh
python3 ~/.claude/switchboard/sb.py install [--dry-run]
```

Common flags:

- `--dry-run`: prints the change and writes nothing. Off by default.

### sb.py uninstall

Takes out only what `install` added, and puts back the status line you had. The `~/.claude/switchboard` folder
stays.

```sh
python3 ~/.claude/switchboard/sb.py uninstall
```

### sb.py places

Lists and changes the places on this computer, which are the folders that chats and Files may use.

```sh
python3 ~/.claude/switchboard/sb.py places [list | add PATH [--name NAME] | remove NAME | roam on|off]
```

Common flags:

- `list`: prints each place, its folder and its state. This is the default.
- `add PATH`: adds a folder. `--name NAME` sets its name, which defaults to the folder's own name.
- `remove NAME`: removes a place.
- `roam on|off`: turns **Anywhere in home** on or off for this computer.

For more information, refer to [Places and folders](concepts/places-and-folders.md).

### sb.py secret

Saves, removes and lists the keys that a library tool needs, in this computer's keychain.

```sh
python3 ~/.claude/switchboard/sb.py secret set TOOL NAME
```

Common flags:

- `set TOOL NAME`: asks for the value and saves it. What you type is hidden.
- `rm TOOL NAME`: removes the key.
- `list [TOOL]`: prints which keys are set and which are missing, never their values. The default is every tool.

Replace `TOOL` with the tool's name and `NAME` with the key's name. For more information, refer to
[Library](features/library.md).

## New in version 0.9.0

!!! note ""
    The commands and flags in this section arrived with Switchboard 0.9.0. Device keys are off until you turn
    them on.

On a Windows home computer, `switchboard.ps1` takes the same commands: `link`, `keys`, `code`, `pair`,
`devices`, `forget` and `uninstall`. Keys need the hub and `switchboard-node` on the home computer. Without the
hub, `keys on` answers "Keys need the hub on this home computer, and it is not installed here." Refer to
[Install Switchboard](install.md#a-windows-server).

### switchboard link

Links the home computer to its own server, so that its chats show on the board. It sets `sb.py` up in Claude
Code, installs `switchboard-node` and links it with a code from the hub on the same computer.

```sh
switchboard link
```

On a computer that is linked already, it says so and changes nothing. The installer runs this by itself on a
first install, unless you pass `--no-link`.

### switchboard keys

Sets how computers reach the home computer: by address, by device key, or both. It writes the `keys` setting in
`server.json` and restarts what needs it.

```sh
switchboard keys on|only|off|status [--force]
```

Common flags:

- `on`: computers connect with a key or by address. Needs `switchboard-node` on the home computer, which
  `switchboard link` installs.
- `only`: computers connect with a key, and the ports are closed to the network. Switchboard first checks that
  this cuts nothing off, and changes nothing when it would.
- `off`: computers connect by address only. This is the default.
- `status`: prints the mode, one line for each computer, and whether the home computer is ready for `only`.
- `--force`: with `only`, switches even when something would be cut off.

`status` answers "Ready for keys only." or "Not ready for keys only yet:" with what stands in the way. Ready
means that every computer connects with its key and that nothing has connected by address for 7 days. To change
the 7, set `keys_ready_days` in `server.json`. Refer to [Ports, files and settings](reference.md#serverjson).

The desktop app does the same in **Settings**, **Devices**, with **Turn keys on** and **Close the network
port**. For more information, refer to [Connections](concepts/connections.md#the-three-modes).

### switchboard code

Makes a code and a link that pair one computer with this home computer. Both work once, for 10 minutes. A short
code needs a relay, so without one it prints a link only.

```sh
switchboard code
```

### switchboard pair

Pairs this computer with a home computer, using a code or a link made there.

```sh
switchboard pair CODE-OR-LINK
```

Replace `CODE-OR-LINK` with the code, such as `K7PM-2XQ9`, or with the whole link in quotes.

### switchboard devices

Lists the devices paired with this home computer: each one's name, its kind, the start of its id, and how it is
connected.

```sh
switchboard devices
```

### switchboard forget

Removes a paired device. Its key stops working at once.

```sh
switchboard forget NAME
```

Replace `NAME` with the device's name as `switchboard devices` shows it, or with the start of its id.

`switchboard code`, `pair`, `devices` and `forget` pass what you type to `switchboard-node`, which has the same
four commands. For more information, refer to [Pair a computer](features/pair-a-computer.md).

### switchboard uninstall

Removes everything Switchboard put on this computer: the helper from Claude Code, `switchboard-node`, the home
computer's software and services, and Switchboard's own Python. It asks first, then lists what it removed and
what it left. The desktop app is named and not removed.

```sh
switchboard uninstall [--purge] [--yes]
```

Common flags:

- `--purge`: also deletes settings, history and logs, and this computer's device key and list of paired devices.
  The default is to keep them.
- `--yes`, `-y`: asks nothing.

For more information, refer to [Install Switchboard](install.md#uninstalling).

### switchboard-node pair, with a code or a link

Pairs this computer with a home computer by device key. The form with an address and a one-time code, described
above, stays as it is.

```sh
switchboard-node pair CODE-OR-LINK [--name NAME] [--addr IP:PORT]
```

Common flags:

- `--name NAME`: the name the home computer lists this computer under. The default is the computer's own name.
- `--addr IP:PORT`: an address to try for the home computer. Only a link made without a relay needs it.

### switchboard-node code, devices and forget

The same as `switchboard code`, `switchboard devices` and `switchboard forget`, on any computer that has
`switchboard-node`.

```sh
switchboard-node code
```

### switchboard-node block and unblock

On a paired computer, stops that computer answering one device, or lets it answer again. It applies to that
computer only, until the home computer's list of devices changes.

```sh
switchboard-node block example-laptop
```

Replace `example-laptop` with the device's name. Refer to
[When the home computer is away](features/when-home-is-away.md#block-a-device-on-one-computer).

### switchboard-node home

Moves the home computer to another of your computers, backs it up, or restores it. `switchboard home` passes
through to it.

```sh
switchboard-node home status
```

Other forms:

- `home move COMPUTER`: moves the home computer to that paired computer. Add `--retire` to have the old one
  keep no key and leave the list.
- `home move --resume`: finishes a move that stopped.
- `home move --undo`: calls off a move that stopped, while that is still possible.
- `home backup`: writes a backup file. The first one prints the recovery code, once. `--to FILE` names the
  file. Default: a new file in `home-backup` in the state folder.
- `home restore FILE`: makes this computer the home computer from a backup. It reads the recovery code from
  standard input.

For more information, refer to [Move or replace the home computer](features/move-or-replace-home.md).

### switchboard-node relay

Sets the relay this computer meets other computers through, or sets none.

```sh
switchboard-node relay URL
```

Replace `URL` with your relay's address, such as `https://relay.example.com`, or with `none`. For more
information, refer to [Run your own relay](features/run-your-own-relay.md).

### switchboard-node uninstall --purge

`switchboard-node uninstall` keeps this computer's device key and its list of paired devices. Add `--purge` to
delete them too.

```sh
switchboard-node uninstall --purge
```

### New installer flags

| Script | Flag | What it does |
|---|---|---|
| `install-server.sh` | `--no-link` | Doesn't link the home computer to its own server |
| `install-server.sh` | `--standby` | Installs the parts a home computer needs and starts nothing, so that the home computer can move here |
| `install-server.ps1` | `-Standby` | The same, in PowerShell |
| `install-server.sh` | `--keys` | Turns keys on after installing, and prints a code and a link for the next computer |
| `install-agent.sh` | `--pair CODE-OR-LINK` | Pairs the computer by device key. Needs `--node` or `--server`, and can't be used with `--code` |
| `install-agent.sh` | `--relay URL` | The relay to use with `--pair` |
| `install-agent.ps1` | `-Pair`, `-Relay` | The same two, in PowerShell |
| `install-server.ps1` | `-HubBinary PATH` | Uses this `switchboard-hub.exe` in place of the one in `native\dist` |
| `install-server.ps1` | `-NodeDir DIR` | Takes the `switchboard-node` builds for the downloads from this folder first |
| `install-server.ps1` | `-NoHub` | Skips the hub: no terminals, and computers are added over ssh |
| `install-server.ps1` | `-NoLink` | Doesn't link the PC to its own server |
| `install-server.ps1` | `-Keys` | Turns keys on after installing, and prints a code for the next computer |
| `install-server.ps1` | `-NoTasks` | Registers no logon tasks, so nothing starts when you sign in |

The installers also bring Python for Switchboard's own use, so a computer needs none of its own. For more
information, refer to [Install Switchboard](install.md#with-device-keys).

## The desktop installer {#the-desktop-installer}

**Install Switchboard** on the Mac disk image and `Switchboard-Setup-<version>.exe` on Windows are the same
program. Opened normally, it shows a window. From a script, it takes flags.

```sh
Switchboard-Setup-0.8.23.exe --quiet [--open-at-login] [--desktop-shortcut] [--open]
```

Replace `0.8.23` with the version you downloaded.

Common flags:

- `--quiet`, `-q`: installs with no window and prints one line for each step.
- `--uninstall`: uninstalls. With `--remove-data`, it also removes the app's settings, and `switchboard-node`
  when this computer has one.
- `--app PATH`: the app to install. The default is the one the installer carries.
- `--dest DIR`: where to install. The default is Applications on a Mac and `%LOCALAPPDATA%\Programs` on Windows.
- `--open-at-login`: opens Switchboard when you sign in. Off by default.
- `--add-to-dock`: adds Switchboard to the Dock on a Mac. Off by default.
- `--desktop-shortcut`: adds a desktop shortcut on Windows. Off by default.
- `--open`: opens Switchboard when the install is done. Off by default.
- `--force-quit`: stops a running copy that doesn't quit when asked. Off by default.
- `--eject`: ejects the disk image when done, on a Mac. Off by default.
- `--help`, `-h`: prints the flags.

With `--quiet`, the exit code says what happened:

| Exit code | Meaning |
|---|---|
| `0` | Done |
| `1` | Failed |
| `2` | Wrong flags |
| `3` | Refused, because a newer version is already installed |

For more information, refer to [The apps](apps.md).
