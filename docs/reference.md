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

## On each computer

| What | Where |
|---|---|
| The helper | `~/.claude/switchboard/sb.py` |
| The helper's log | `~/.claude/switchboard/sb.log` (rolls over past 5 MB) |
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
