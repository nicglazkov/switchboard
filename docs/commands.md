---
description: Every command Switchboard installs, on the server and on each computer, with its options.
---

# Commands

## On the server: `switchboard`

The installer adds `~/.local/bin/switchboard`.

| Command | What it does |
|---|---|
| `switchboard` or `switchboard status` | Is everything running? Asks the server (8130), the hub (8132) and the downloads (8131) |
| `switchboard start` | Start the server, the hub and the downloads |
| `switchboard stop` | Stop them |
| `switchboard restart` | Stop, then start |
| `switchboard logs [lines]` | The last lines of each part's log (60 from the journal on systemd, 20 per file otherwise) |
| `switchboard setup [options]` | First-run setup, again (below) |

It works with whatever runs the server: LaunchAgents on a Mac, systemd user services on Linux, or plain background
processes when neither is set up.

!!! note "On a Windows server"
    The same commands are
    `powershell -File %LOCALAPPDATA%\Switchboard\switchboard.ps1 start|stop|restart|status|logs|setup`.
    There is no hub on Windows, and `logs` shows the last 30 lines of the server's and the downloads' logs.

### `switchboard setup`

With no options it asks three things: the server's address, your computers, and each computer's Claude folder.
With options it does just that and asks nothing.

| Option | What it does |
|---|---|
| `--address ADDR` | The address devices use for this server |
| `--code` | Print a code and the lines that link a computer, then stop |
| `--code --json` | The same, as JSON |
| `--ssh ID=USER@HOST` | Add a computer reached over ssh (repeat for more) |
| `--root ID=FOLDER` | Set a computer's Claude folder (repeat for more) |
| `--ssh-key` | Print this server's ssh public key (made the first time) |
| `--yes`, `-y` | Take the suggested answer everywhere |

```sh
switchboard setup --address studio.local      # use this address
switchboard setup --code                      # a new code and the lines to paste
switchboard setup --ssh tower=me@tower.local  # add a computer reached over ssh
switchboard setup --root tower=~/Code         # set a computer's Claude folder
```

`NO_COLOR` or `SWITCHBOARD_PLAIN` turn off colors.

## Installing the server

| Script | Options |
|---|---|
| `get-server.sh` | `--kit URL\|FILE`, `--sha256 HEX`; everything else goes to `install-server.sh` |
| `install-server.sh` | `--hub-binary PATH`, `--node-dir DIR`, `--no-hub`, `--no-service`, `--no-start`, `--no-setup`, `--yes`, `--plain`, `--uninstall` |
| `install-server.ps1` | `-NoStart`, `-NoSetup`, `-Uninstall` |
| `pack-server-kit.sh` | `--base URL`, `--out DIR`, `--no-binaries`, `--no-sources`, `--no-vendor` |

[Install](install.md#installer-options) says what each one does.

## On each computer

### `install-agent.sh` and `install-agent.ps1`

What the line from **Add a machine** runs. Served by the server at `http://<server>:8131/`.

| Option | PowerShell | What it does |
|---|---|---|
| `--server ADDR` | `-Server` | The server to link to |
| `--code CODE` | `-Code` | The one-time code |
| `--ssh-key "KEY"` | `-SshKey` | Let the server in over ssh with this key, limited to the helper |
| `--name ID` | `-Name` | The computer's id on the board |
| `--label NAME` | `-Label` | The name it shows |
| `--sb PATH` | `-Sb` | Install this `sb.py` instead of downloading it |
| `--node PATH` | `-Node` | Install this `switchboard-node` instead of downloading it |
| `--update` | `-Update` | Replace a helper that is already installed |

### `switchboard-node`

Installed in `~/.local/bin` (`%LOCALAPPDATA%\Switchboard` on Windows).

| Command | What it does |
|---|---|
| `switchboard-node pair <server>:8132 <code> [--name NAME] [--label LABEL]` | Link this computer, with a code from Add a machine |
| `switchboard-node install` | Copy itself into place and run at login (needs pairing first) |
| `switchboard-node run` | Keep the link open (what the login item runs) |
| `switchboard-node status` | Which machine it is linked as, and to which hub |
| `switchboard-node --version` | Its version |

Its settings are in `~/.config/switchboard/node.json`. Its log: `~/Library/Logs/switchboard-node.log` on a Mac,
`journalctl --user -u switchboard-node` on Linux with systemd, `~/.local/share/switchboard/node.log` on Linux
without it.

### `sb.py`, the helper

In `~/.claude/switchboard/sb.py`. Run it with `python3` (`python` on Windows).

| Command | What it does |
|---|---|
| `sb.py install [--dry-run]` | Add the helper to Claude Code: its hooks and status line in `~/.claude/settings.json`, and the `switchboard` MCP server in `~/.claude.json`. Backs up both first and saves a status line you had; on Windows it also adds the logon task `SwitchboardLaunch`. `--dry-run` prints the change instead |
| `sb.py uninstall` | Take out only what `install` added. The `~/.claude/switchboard` folder stays |
| `sb.py places list` | The places chats and the file browser may use |
| `sb.py places add PATH [--name NAME]` | Add one |
| `sb.py places remove NAME` | Remove one |
| `sb.py places roam on\|off` | Whether chats may start anywhere in the home folder |
| `sb.py secret set <tool> <NAME>` | Save a secret a shared tool needs, in the system keychain; typing is hidden |
| `sb.py secret rm <tool> <NAME>` | Remove it |
| `sb.py secret list [<tool>]` | Which secrets are set (never their values) |

The other commands (`hook`, `statusline`, `rpc`, `mcp` and so on) are what Claude Code and the server run.

## The desktop installer

`Install Switchboard` on a Mac and `Switchboard-Setup-<version>.exe` on Windows take options for scripts:

| Option | What it does |
|---|---|
| `--quiet`, `-q` | Install with no window |
| `--uninstall` | Uninstall |
| `--app PATH` | The app to install, instead of the one it carries |
| `--dest DIR` | Where to install |
| `--open-at-login` | Open Switchboard when you sign in |
| `--add-to-dock` | Add it to the Dock (Mac) |
| `--desktop-shortcut` | Add a desktop shortcut (Windows) |
| `--open` | Open Switchboard when done |
| `--force-quit` | Stop a running copy that does not quit when asked |
| `--remove-data` | With `--uninstall`: remove settings too |
| `--eject` | Eject the disk image when done (Mac) |
| `--help` | All of the above |

Everything that is a choice is off unless you pass it. With `--quiet` the exit code says what happened: `0` done,
`1` failed, `2` wrong options, `3` refused because a newer version is installed.
