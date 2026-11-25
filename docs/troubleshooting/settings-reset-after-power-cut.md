---
description: After a power cut, settings are back to their defaults, or only the home computer itself can connect. A settings file could not be read. How to put it back.
---

# Settings are back to defaults after a power cut

<p class="sb-meta">Last validated: Nov 23, 2026</p>

After a power cut or a full disk, your settings are back to their defaults, your computers are kept to their
Claude folders, or only the home computer itself can open the board.

## Cause

A settings file on your home computer, the one of your computers that runs Switchboard itself, could not be
read, and its spare copy was no good either. Switchboard never throws such a file away. It renames it
`<name>.bad-<date>-<time>` and carries on with the careful choice for that file:

| File | What you see |
|---|---|
| `server.json` | The home computer answers itself only, so no other device connects |
| `fleet.json` | Each computer is kept to its Claude folder, and **Anywhere in home** is off |
| `prefs.json` | Settings are at their defaults |

## Fix

1. On the home computer, run `switchboard logs`. The server's log names the file it couldn't read.
2. Open `~/.config/switchboard/` for `server.json` and `fleet.json`, or `~/.local/share/switchboard/` for
   `prefs.json`. The damaged file is beside the real one, with `.bad-` in its name.
3. Put a good file back from your own backup. With no backup, set things again: run `switchboard setup` for
   the address, and use the app's **Settings** for the rest. For `server.json`, you can also delete the file
   and its `.bad` copies to start from the defaults.
4. Run `switchboard restart`.

For more information, refer to [Ports, files and settings](../reference.md#copies-of-its-files).
