---
description: Switchboard's ports, where every file lives on each system, and the environment variables you can set.
---

# Ports, files and settings

## Ports

| Port | On | What |
|---|---|---|
| **8130** | the server | The server's API, which the apps use |
| **8131** | the server | Downloads for adding a computer (`install-agent.sh`, `sb.py`, `switchboard-node`, `SHA256SUMS`) |
| **8132** | the server | The hub: terminals, and the links from your computers |

None of them has a password. [Keep them on your own network.](privacy.md#no-passwords-on-purpose) The links from
your computers go out to 8132, so the computers themselves need no open port.

## On the server

| What | Where |
|---|---|
| Settings: your computers, the server's addresses, push, budget | `~/.config/switchboard/` (`fleet.json`, `server.json`, `setup.json`, `apns.json`, `budget.json`, `downloads.json`, ...) |
| History and state | `~/.local/share/switchboard/` |
| The server itself | `~/.local/share/switchboard/app/` (Windows: `%LOCALAPPDATA%\Switchboard\app\`) |
| What new computers download | `~/.local/share/switchboard/downloads/` |
| The `switchboard` command | `~/.local/bin/switchboard` |
| The hub | `~/.local/bin/switchboard-hub` |
| Install log | `~/.local/share/switchboard/logs/install.log` |
| Logs | `switchboard logs`. On Linux with systemd: `journalctl --user -u switchboard -u switchboard-hub -u switchboard-downloads`. On a Mac: `~/Library/Logs/Switchboard/`. Otherwise `~/.local/share/switchboard/logs/` |
| Services | Linux: systemd user units `switchboard`, `switchboard-hub`, `switchboard-downloads`. Mac: LaunchAgents `com.switchboard.server`, `.hub`, `.downloads`. Windows: logon tasks `SwitchboardServer`, `SwitchboardDownloads` |

### server.json

Where the server listens and who it answers. The server writes it on its first start and reads it when it starts,
so restart the server after a change (`switchboard restart`).

| Key | What it does |
|---|---|
| `bind` | The addresses the server listens on. `["0.0.0.0"]` is every address of the computer |
| `hosts` | The names and addresses that are the server's own. A request for any other `Host` is refused |
| `origins` | The server's addresses as the apps and the hub are told them, like `http://192.0.2.10:8130` |
| `allow` | Kinds of `Host` taken as well: `"private"` (a LAN or Tailscale address typed as a number), `"local"` (a Bonjour name like `studio.local`), `"tailscale"` (a `*.ts.net` name). Left out, all three are on |
| `peers` | More addresses the server answers, beside the ones it always does: this computer, home and office networks, link-local and Tailscale addresses. Each entry is an address or a network, like `"203.0.113.0/24"`. `["any"]` answers every address |

A request from an address that is not let in gets `403` and "forbidden address". Left out, `peers` adds nothing,
which is right for a home network or Tailscale.

!!! danger "Answering every address takes the check away"
    With `any`, anyone who can reach port 8130 can use your server. Switchboard has no passwords.
    [Keep it on your own network.](privacy.md#no-passwords-on-purpose)

A proxy on the server's own computer makes every request look like it comes from that computer, so the check
cannot tell them apart. Have the proxy do the checking.

### Copies of its files

Beside each settings and state file the server keeps `<name>.bak`, the file as it was before its last change.
For a file you edit by hand (`server.json`, `fleet.json`, `prefs.json`) the copy is brought up to date every time
the server reads the file and finds it good, so a later typo brings back your last good edit and nothing older.
The copies of `fleet.json` and of the lists of phones to notify are of the file as it is now, so a computer or a
phone you removed is not in them. So is the copy of a computer's `secrets.json`.
A file that cannot be read (after a power cut, a full disk, a bad edit) is never thrown away: it is renamed
`<name>.bad-<date>-<time>` (the newest three are kept), the `.bak` takes its place, and the log says so.

A file that is there and will not open (no permission, or another program is holding it) is read again after a
moment. If it still will not open, the `.bak` is used and the file is left exactly as it is.

With no good copy the server does not fall back to "everything on":

| File | Until you fix it |
|---|---|
| `server.json` | The server answers its own computer only. Put a good file back, or delete it and its `.bad` copies to start from the defaults, then restart. |
| `fleet.json` | Your computers stay on the list, each kept to its Claude folder. Set them again in **Settings, Computers**. When the list comes back from its copy, "Anywhere in home" is turned off as well, until you turn it on again. |
| `prefs.json` | Settings are at their defaults, the opt-in ones are off, and chats do not stop by themselves. Save anything in **Settings** to start the file again. |
| `keep.json`, `tabs.json` | Chats do not stop by themselves until you keep a chat or an app sends its tabs again. |
| `term.key` | A new key is made. Restart the hub if terminals do not open. |

`GET /api/server` lists the files this happened to under `stores`.

The helper on each computer does the same for the names you gave chats, its list of installed tools and its
secrets file, in `~/.claude/switchboard/`, and says so in `sb.log`.

## On each computer

| What | Where |
|---|---|
| The helper | `~/.claude/switchboard/sb.py` |
| The helper's log | `~/.claude/switchboard/sb.log` (rolls over past 5 MB) |
| Fresh starts, holds and stops | `~/.claude/switchboard/state/events.json` (the newest 2,000; sizes and ids, never chat text) |
| The link to the server | `~/.config/switchboard/node.json` |
| `switchboard-node` | `~/.local/bin/switchboard-node` (Windows: `%LOCALAPPDATA%\Switchboard\switchboard-node.exe`) |
| Its log | Mac: `~/Library/Logs/switchboard-node.log`. Linux: `journalctl --user -u switchboard-node`, or `~/.local/share/switchboard/node.log` without systemd |
| Its login item | Mac: LaunchAgent `com.switchboard.node`. Linux: systemd user unit `switchboard-node`. Windows: logon task `SwitchboardNode` |

## The desktop app

| What | Mac | Windows | Linux |
|---|---|---|---|
| The app | `/Applications/Switchboard.app` (or `~/Applications`) | `%LOCALAPPDATA%\Programs\Switchboard\` | wherever you unpacked it |
| Settings | `~/Library/Application Support/com.glazkov.switchboard.desktop/` | `%APPDATA%\Switchboard\` | `~/.config/Switchboard/` <!-- leakscan: allow --> |
| Logs | `~/Library/Logs/Switchboard/` | `%LOCALAPPDATA%\Switchboard\logs\` | `~/.local/state/switchboard/logs/` |

The log is `Switchboard.log`. Past 2 MB it becomes `Switchboard.log.1`, and five are kept. A crash also writes
`crash-<time>.txt` beside it (the ten newest are kept). Log lines never hold chat text, and your home folder is
written as `~`. **About, Copy diagnostics** copies the version, the system and the last 200 log lines, without
the server's address, for a bug report.

## Environment variables

=== "The app"

    | Variable | What it does |
    |---|---|
    | `SWITCHBOARD_URL` | The server to use for this run, like `http://192.0.2.10:8130` |
    | `SB_LOG` (or `RUST_LOG`) | What the log keeps, env_logger style: `SB_LOG=warn,Switchboard=debug`. Default `info` |
    | `SB_LOG_DIR` | Another log folder |
    | `SB_SETTINGS` | Another settings file |
    | `SB_THEME` | `light` or `dark`, whatever Settings says |

=== "The server"

    | Variable | What it does |
    |---|---|
    | `SWITCHBOARD_RELEASES` | Where updates come from: `owner/name` for another GitHub repository, `off` for none. Default `nicglazkov/switchboard` |
    | `SWITCHBOARD_DOWNLOADS` | Where new computers download `sb.py` and `switchboard-node` from |
    | `SWITCHBOARD_TZ` | The time zone for days in usage and budget (the installer writes it to `~/.local/share/switchboard/env`) |

=== "The helper"

    | Variable | What it does |
    |---|---|
    | `SB_HOME` | The helper's folder. Default `~/.claude/switchboard` |
    | `SWITCHBOARD_NODE_CONFIG` | Another `node.json` |
    | `SWITCHBOARD_PYTHON` | The Python the node runs `sb.py` with |
