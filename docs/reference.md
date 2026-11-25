---
description: Switchboard's ports, where every file lives on each system, the keys of server.json, and the environment variables you can set.
---

# Ports, files and settings

<p class="sb-meta">Last validated: Nov 22, 2026</p>

This page lists the ports Switchboard uses, the files it keeps on each computer, and the environment variables
that change how it runs. File names and commands say server for the home computer, the one of your computers that
runs Switchboard itself.

## Ports

All three ports are on the home computer.

| Port | What it carries |
|---|---|
| `8130` | The server's API, which the apps use |
| `8131` | The downloads for adding a computer: `install-agent.sh`, `install-agent.ps1`, `node/sb.py`, the `switchboard-node` builds and `node/SHA256SUMS` |
| `8132` | The hub: chat terminals, and the links from your computers |

None of the ports has a password. For more information, refer to
[Privacy and security](privacy.md#no-passwords-on-purpose).

Version 0.9.0 added two ports. Both answer the computer they are on and nothing else.

| Port | Where | What it carries |
|---|---|---|
| `8136` | The home computer, while keys are on | The hub's listener for paired computers. `switchboard-node` hands it the terminal and link connections that arrive through the tunnel |
| `8137` | Every computer where `switchboard-node` runs | The helper's control API: status, making a code, pairing, listing and removing devices, and setting the relay |

On a paired computer that is not the home computer, `switchboard-node` also listens on that computer's own
`127.0.0.1:8130` and `127.0.0.1:8132`, and carries what connects there to the home computer.

## On the home computer

| What | Where |
|---|---|
| Settings | `~/.config/switchboard/` |
| History and state | `~/.local/share/switchboard/` |
| The server's program | `~/.local/share/switchboard/app/` |
| What new computers download | `~/.local/share/switchboard/downloads/` |
| The `switchboard` command | `~/.local/bin/switchboard` |
| The hub | `~/.local/bin/switchboard-hub` |
| The install log | `~/.local/share/switchboard/logs/install.log` |
| The update log | `~/.local/share/switchboard/update/update.log` |

On a Windows home computer, the settings are in `%USERPROFILE%\.config\switchboard\`, the history and state in
`%USERPROFILE%\.local\share\switchboard\`, and the program, the downloads, the logs and `switchboard.ps1` in
`%LOCALAPPDATA%\Switchboard\`. Since version 0.9.0 the hub is installed there too, as
`%LOCALAPPDATA%\Switchboard\switchboard-hub.exe`.

### Settings files

These files are in `~/.config/switchboard/`.

| File | What it holds |
|---|---|
| `server.json` | Where the server listens and who it answers |
| `fleet.json` | Your computers |
| `setup.json` | What first-run setup has done |
| `budget.json` | Your daily budget |
| `downloads.json` | Your own download page for updates, when you set one |
| `apns.json` and `apns/` | The key for iPhone notifications, when you set one |
| `term.key` | The key the server and the hub share for terminals |
| `device.key` | This computer's device key, with `keys.lock` beside it. Since version 0.9.0 |
| `blocked.json` | The devices blocked on this computer. Since version 0.9.0, with device keys |
| `backup.key` | The key that seals backups of the home computer, made from the recovery code. Since version 0.9.0, with device keys |
| `home-move.json`, `home-stepdown.json` | A move of the home computer that is under way, and the note that this computer was the home computer and no longer answers as it. Since version 0.9.0, with device keys |

### State files

These files are in `~/.local/share/switchboard/`. The app's Settings write `prefs.json`, so you don't need to edit
it by hand.

| File | What it holds |
|---|---|
| `prefs.json` | The choices you make in the app's Settings that belong to the home computer |
| `names.json`, `keep.json`, `tabs.json` | The names you gave chats, the chats you keep, and the tabs each app has open |
| `machines.json`, `nodes.json` | The list of computers the server writes for the hub, and the computers linked with a code |
| `devices.json`, `widget_tokens.json` | The phones to notify |
| `budget_state.json`, `spend.json`, `week_history.json` | Usage and budget history |
| `jobs.json` | Chats sent by Claude |
| `update/state.json` | The last update of the home computer |
| `env` | Environment variables the installer set, such as `SWITCHBOARD_TZ` |
| `paired.json` | The devices paired with this computer, and the home computer it is paired with. Since version 0.9.0 |
| `python/` | Switchboard's own Python. Since version 0.9.0 |
| `home-backup/` | Backups of the home computer, as `.sbk` files. Since version 0.9.0, with device keys |

### Services and logs

| System | Runs as | Logs |
|---|---|---|
| macOS | LaunchAgents `com.switchboard.server`, `com.switchboard.hub` and `com.switchboard.downloads` | `~/Library/Logs/Switchboard/` |
| Linux with systemd | User units `switchboard`, `switchboard-hub` and `switchboard-downloads` | `journalctl --user -u switchboard -u switchboard-hub -u switchboard-downloads` |
| Linux without systemd | Background processes, with their process ids in `~/.local/share/switchboard/run/` | `~/.local/share/switchboard/logs/` |
| Windows | Logon tasks `SwitchboardServer` and `SwitchboardDownloads`. Version 0.9.0 added `SwitchboardHub` | `%LOCALAPPDATA%\Switchboard\logs\` |

`switchboard logs` prints the last lines on every system. Refer to [Commands](commands.md).

### server.json {#serverjson}

`server.json` sets where the server listens and who it answers. The server writes the file on its first start and
reads it only when it starts, so run `switchboard restart` after you change it.

| Key | What it does | Default |
|---|---|---|
| `bind` | The addresses the server listens on. `0.0.0.0` is every address of the computer. | `["0.0.0.0"]` |
| `hosts` | The names and addresses that are the server's own. The server refuses a request for any other name. | Found at first start |
| `origins` | The server's addresses as the apps and the hub are told them, such as `http://192.0.2.10:8130`. | Found at first start |
| `allow` | The kinds of name the server also accepts: `private` for an address on a home network or Tailscale, `local` for a name that ends in `.local`, and `tailscale` for a name that ends in `.ts.net`. An empty list turns all three off. | All three |
| `keys` | How computers reach the home computer: `off` by address, `both` by address or by device key, `only` by device key with the ports closed to the network. A value the server doesn't know counts as `only`. Since version 0.9.0. Set it with `switchboard keys`. | `off` |
| `keys_ready_days` | How many days with no connection by address the home computer waits for before it calls `only` safe. A whole number from 1 to 90. Read without a restart. Since version 0.9.0. | `7` |
| `peers` | More addresses the server answers, each one an address or a network such as `203.0.113.0/24`. `["any"]` answers every address. | None |

The server always answers its own computer, private home and office networks, link-local addresses and Tailscale
addresses. A request from any other address is refused unless `peers` lists it, and the server's log names the
address it refused.

!!! note ""
    With `"peers": ["any"]`, anyone who can reach port 8130 can see and run your chats. This is because Switchboard
    has no passwords. Refer to [Privacy and security](privacy.md#no-passwords-on-purpose).

### Copies of its files {#copies-of-its-files}

The server keeps a copy of each settings and state file beside it, named `<name>.bak`. The copy is the file as it
was before the last change.

A file that can't be read, for example after a power cut, a full disk or a wrong edit, is never thrown away. The
server renames it `<name>.bad-<date>-<time>`, keeps the newest three of those, uses the `.bak` copy in its place and
says so in its log. A file that is there but won't open is tried again, and if it still won't open, the server
uses the `.bak` copy and leaves the file as it is.

When there is no good copy either, the server falls back to the careful choice:

| File | Until you fix it |
|---|---|
| `server.json` | The server answers its own computer only. Put a good file back, or delete the file and its `.bad` copies to start from the defaults, then restart. |
| `fleet.json` | Each of your computers is kept to its Claude folder, and **Anywhere in home** is off. |
| `prefs.json` | Settings are at their defaults, the ones you turn on yourself are off, and chats don't stop by themselves. |
| `keep.json`, `tabs.json` | Chats don't stop by themselves. |
| `term.key` | The server makes a new key. Restart the home computer's software if terminals don't open. |

The helper on each computer treats its own files in `~/.claude/switchboard/` the same way and says so in `sb.log`.
For more information, refer to [Troubleshooting](troubleshooting.md).

## On each computer

| What | Where |
|---|---|
| The helper | `~/.claude/switchboard/sb.py` |
| The helper's log | `~/.claude/switchboard/sb.log`, which starts again past 5 MB and keeps the old one as `sb.log.1` |
| Fresh starts, holds and stops | `~/.claude/switchboard/state/events.json`, which keeps the newest 2,000 with sizes and ids, never chat text |
| Places | `~/.claude/switchboard/agent.json` |
| The link to the home computer | `~/.config/switchboard/node.json` |
| The device key, since version 0.9.0 | `~/.config/switchboard/device.key` |
| Paired devices, since version 0.9.0 | `~/.local/share/switchboard/paired.json` |
| `switchboard-node` | `~/.local/bin/switchboard-node`, or `%LOCALAPPDATA%\Switchboard\switchboard-node.exe` on Windows |

`switchboard-node` starts when you sign in:

| System | Runs as | Log |
|---|---|---|
| macOS | LaunchAgent `com.switchboard.node` | `~/Library/Logs/switchboard-node.log` |
| Linux with systemd | User unit `switchboard-node` | `journalctl --user -u switchboard-node` |
| Linux without systemd | A background process | `~/.local/share/switchboard/node.log` |
| Windows | Logon task `SwitchboardNode` | None |

## The desktop app

| What | macOS | Windows | Linux |
|---|---|---|---|
| The app | `/Applications/Switchboard.app`, or `~/Applications` | `%LOCALAPPDATA%\Programs\Switchboard\` | Wherever you unpacked it |
| Settings | `~/Library/Application Support/com.glazkov.switchboard.desktop/` | `%APPDATA%\Switchboard\` | `~/.config/Switchboard/` <!-- leakscan: allow --> |
| Logs | `~/Library/Logs/Switchboard/` | `%LOCALAPPDATA%\Switchboard\logs\` | `~/.local/state/switchboard/logs/` |

The settings file is `settings.json`. The log is `Switchboard.log`. Past 2 MB it becomes `Switchboard.log.1`, and
five files are kept. A crash also writes `crash-<time>.txt` beside it, and the ten newest are kept. The log writes
your home folder as `~`. For more information, refer to [Get help](support.md).

## Environment variables

### The app

| Variable | What it does | Default |
|---|---|---|
| `SWITCHBOARD_URL` | The home computer to use for this run, such as `http://192.0.2.10:8130`. Separate several with commas. | The one in Settings |
| `SB_LOG`, or `RUST_LOG` | How much the log keeps, such as `warn` or `debug`. | `info` |
| `SB_LOG_DIR` | Another log folder. | The folder in the table above |
| `SB_SETTINGS` | Another settings file. | The file in the table above |
| `SB_THEME` | `light` or `dark`, whatever Settings says. | The choice in Settings |

### The server

| Variable | What it does | Default |
|---|---|---|
| `SWITCHBOARD_RELEASES` | Where updates come from: `owner/name` for another GitHub repository, or `off` for none. | `nicglazkov/switchboard` |
| `SWITCHBOARD_DOWNLOADS` | The address that new computers download `sb.py` and `switchboard-node` from. | The home computer's own port 8131 |
| `SWITCHBOARD_TZ` | The time zone for days in usage and budget. The installer writes it to `~/.local/share/switchboard/env`. | The computer's time zone |
| `SWITCHBOARD_HUB_PORT` | The port the server expects the hub on. | `8132` |

Version 0.9.0 added these:

| Variable | What it does | Default |
|---|---|---|
| `SWITCHBOARD_PORT` | The port the server listens on. | `8130` |
| `SWITCHBOARD_DOWNLOADS_PORT` | The port the downloads are served on. | `8131` |
| `SWITCHBOARD_HUB_URL` | The hub's address as the server reaches it. | `http://127.0.0.1:8132` |
| `SWITCHBOARD_CONFIG_DIR` | Another folder for settings files. | `~/.config/switchboard` |
| `SWITCHBOARD_STATE_DIR` | Another folder for state files. | `~/.local/share/switchboard` |
| `SWITCHBOARD_SERVER_JSON` | Another `server.json`. | The one in the settings folder |

### The hub

| Variable | What it does | Default |
|---|---|---|
| `SWITCHBOARD_HUB_LISTEN` | The address and port the hub listens on. | `0.0.0.0:8132` |
| `SWITCHBOARD_HUB_MAX` | The most terminals open at once. A chat open on several devices is one terminal. | `16` |
| `SWITCHBOARD_HUB_MAX_VIEWERS` | The most views of those terminals open at once, over all your devices. | Four for each terminal, so `64` |
| `SWITCHBOARD_HUB_IDLE` | The seconds with no input or output before the hub closes a terminal. `0` means never. | `7200` |
| `SWITCHBOARD_HUB_TUNNEL_LISTEN` | Since version 0.9.0: where the hub listens for paired computers while keys are on. It must be an address of the computer itself. | `127.0.0.1:8136` |

### The helper and switchboard-node

| Variable | What it does | Default |
|---|---|---|
| `SB_HOME` | The helper's folder. | `~/.claude/switchboard` |
| `SWITCHBOARD_NODE_CONFIG` | Another `node.json`. | `~/.config/switchboard/node.json` |
| `SWITCHBOARD_SB` | The `sb.py` that `switchboard-node` runs. | `~/.claude/switchboard/sb.py` |
| `SWITCHBOARD_PYTHON` | The Python that `switchboard-node` runs `sb.py` with. | The one found on the computer |

Version 0.9.0 added these for `switchboard-node`:

| Variable | What it does | Default |
|---|---|---|
| `SWITCHBOARD_RELAY` | The relay to use for this run. | The saved relay |
| `SWITCHBOARD_PAIRED` | Another `paired.json`. | `~/.local/share/switchboard/paired.json` |
| `SWITCHBOARD_NODE_CONTROL` | Where the control API listens. | `127.0.0.1:8137` |
| `SWITCHBOARD_NODE_API_TARGET` | On the home computer, where connections for the server are handed. | `127.0.0.1:8130` |
| `SWITCHBOARD_NODE_HUB_TARGET` | On the home computer, where connections for the hub are handed. | `127.0.0.1:8136` |
| `SWITCHBOARD_NODE_API_FORWARD` | On a paired computer, the address that leads to the home computer's server, or `off`. | `127.0.0.1:8130` |
| `SWITCHBOARD_NODE_HUB_FORWARD` | On a paired computer, the address that leads to the home computer's hub, or `off`. | `127.0.0.1:8132` |

Each of the five addresses must be on the computer itself, such as `127.0.0.1` and a port.

### The installers

| Variable | What it does | Default |
|---|---|---|
| `SWITCHBOARD_KIT_URL` | The server kit that `get-server.sh` downloads. | The newest public release |
| `NO_COLOR`, `SWITCHBOARD_PLAIN` | Turn colors off in `switchboard setup` when set. | Not set |
