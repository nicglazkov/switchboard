---
description: How to get help with Switchboard, report a bug, ask for a feature, or report a security problem.
---

# Get help

<div class="grid cards" markdown>

-   :material-lifebuoy:{ .lg .middle } **Something isn't working**

    ---

    Most problems have a quick fix.

    [:octicons-arrow-right-24: Troubleshooting](troubleshooting.md)

-   :material-bug-outline:{ .lg .middle } **Report a bug**

    ---

    Open an issue with what you did, what happened, and the diagnostics below.

    [:octicons-arrow-right-24: New issue](https://github.com/nicglazkov/switchboard/issues/new)

-   :material-lightbulb-outline:{ .lg .middle } **Ask for a feature**

    ---

    Open an issue that says what you were trying to do.

    [:octicons-arrow-right-24: Issues](https://github.com/nicglazkov/switchboard/issues)

-   :material-shield-lock-outline:{ .lg .middle } **A security problem**

    ---

    Report it privately, not in an issue.

    [:octicons-arrow-right-24: Report it](privacy.md#report-a-security-problem)

</div>

## What to put in a bug report

1. **Copy diagnostics** in **About Switchboard**: the app's version, build, system and its last 200 log lines.
   It writes your home folder as `~`.
2. What you did, what you expected, and what happened.
3. If it's about the server or a computer, the output of `switchboard status` and `switchboard logs` on the
   server.

Look through the lines before you post them: logs can name your chats and folders.

## Where the logs are

| | Mac | Windows | Linux |
|---|---|---|---|
| **The app** | `~/Library/Logs/Switchboard/` | `%LOCALAPPDATA%\Switchboard\logs\` | `~/.local/state/switchboard/logs/` |
| **The server** | `switchboard logs` | `switchboard.ps1 logs` | `switchboard logs` |
| **The link** (`switchboard-node`) | `~/Library/Logs/switchboard-node.log` | | `journalctl --user -u switchboard-node` |
| **The helper** (`sb.py`) | `~/.claude/switchboard/sb.log` | `~\.claude\switchboard\sb.log` | `~/.claude/switchboard/sb.log` |

**Show log** in About Switchboard opens the app's log folder. If the app crashes, it writes a `crash-*.txt` next to
the log; nothing is sent anywhere. [Every file Switchboard keeps](reference.md)
