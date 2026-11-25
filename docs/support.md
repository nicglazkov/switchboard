---
description: Where to get help with Switchboard, how to report a bug or a security problem, and what to include so that it can be fixed.
---

# Get help

<p class="sb-meta">Last validated: Nov 22, 2026</p>

Switchboard is an open source project, and help comes through its GitHub repository. This page lists where to
go for each kind of question and what to send with a bug report.

## Where to go

- **Something isn't working**: most problems have a known fix. Refer to [Troubleshooting](troubleshooting.md).
- **A short question**: refer to [Questions](faq.md).
- **A bug**: open an issue at
  [github.com/nicglazkov/switchboard/issues](https://github.com/nicglazkov/switchboard/issues/new) with the
  details listed below.
- **A feature you want**: open an issue that says what you were trying to do.
- **A security problem**: report it privately, not in an issue. Refer to
  [Privacy and security](privacy.md#report-a-security-problem).

## What to include in a bug report

1. **What happened**: what you did, what you expected and what you got.
2. **The app's diagnostics**: open **About Switchboard** and click **Copy diagnostics**. It copies the version,
   the build, the system and the last 200 lines of the app's log.
3. **The home computer's details**, when the problem is about the home computer or one of your computers: open
   **Settings**, select **Server**, and click **Copy details**. If the app can't reach the home computer, run
   `switchboard status` and `switchboard logs` in a terminal on it and include what they print.

!!! note ""
    Read the lines before you post them. The app writes your home folder as `~` and leaves out the home computer's
    address, but a log can still name your chats, folders and computers.

## Where the logs are

**Show log** in **About Switchboard** opens the app's log folder. If the app crashes, it writes a file named
`crash-<time>.txt` in the same folder. Nothing is sent anywhere.

| Log | macOS | Windows | Linux |
|---|---|---|---|
| The app | `~/Library/Logs/Switchboard/` | `%LOCALAPPDATA%\Switchboard\logs\` | `~/.local/state/switchboard/logs/` |
| The home computer | `switchboard logs` | `switchboard.ps1 logs` | `switchboard logs` |
| The link program, `switchboard-node` | `~/Library/Logs/switchboard-node.log` | None | `journalctl --user -u switchboard-node`, or `~/.local/share/switchboard/node.log` without systemd |
| The helper, `sb.py` | `~/.claude/switchboard/sb.log` | `%USERPROFILE%\.claude\switchboard\sb.log` | `~/.claude/switchboard/sb.log` |

For every file Switchboard keeps, refer to [Ports, files and settings](reference.md).
