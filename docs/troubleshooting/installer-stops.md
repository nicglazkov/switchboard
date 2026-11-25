---
description: The installer for the home computer or for a computer's helper stops with a message. What each message means and what to do next.
---

# The installer stops with a message

<p class="sb-meta">Last validated: Nov 23, 2026</p>

The installer for your home computer, the one of your computers that runs Switchboard itself, or the line you
pasted on a computer you are adding, stops before it is done and prints one of the messages below.

The home computer's installer keeps its full output in `~/.local/share/switchboard/logs/install.log`. It also
shows the last lines of the step that failed.

## "Python 3.9 or newer is needed" or "is too old"

**Cause**: the home computer and each computer you add need Python 3.9 or newer.

**Fix**:

1. Install Python. On a Mac, run `xcode-select --install`. On Linux, install `python3` with your system's
   package manager. On Windows, run `winget install -e --id Python.Python.3.12`.
2. Run the installer again.

Since version 0.9.0, the installers bring Python of their own, and this message goes away.

## "no server kit to install from"

**Cause**: the released `get-server.sh` doesn't know where the server kit is unless you tell it.

**Fix**: run the install line with `--kit` and the kit's address, as shown in
[Install Switchboard](../install.md#run-the-installer).

## Ports "in use by another program"

**Cause**: the home computer needs ports 8130, 8131 and 8132, and another program holds one of them. The
installer's check names the port. A Switchboard that is already installed is fine: it restarts.

**Fix**:

1. Stop the program that holds the port, or pick another computer as the home computer.
2. Run the installer again.

## "the hub did not build"

**Cause**: you are installing from a copy of the repository, there is no ready-made hub, and building one with
Rust failed.

**Fix**: install from a release, which brings a ready-made hub. Refer to
[Install Switchboard](../install.md#run-the-installer). To install without the hub, add `--no-hub`. You then
have no terminals and add computers over ssh only.

## "the server did not start"

**Cause**: the server was installed and didn't answer in time. When an older Switchboard was there, the
installer puts it back and starts it again.

**Fix**:

1. Run `switchboard logs` and read the server's last lines.
2. Fix what the log names, such as a port that is taken, then run the installer again.

## "the server has no switchboard-node for" a system

**Cause**: the home computer hands out the helper only for the systems it has a build for.

**Fix**: use one of these.

- Use the line from **Add a machine** in the app. When the home computer has no helper for the system, that
  line downloads it from the newest release on GitHub.
- Give the home computer every build: run its installer again with `--node-dir` and a folder that holds
  `switchboard-node-mac`, `switchboard-node-linux` and `switchboard-node.exe` from a release.
- Add the computer over ssh. Refer to [Add your computers](../machines.md#over-ssh).

## "did not match its checksum; nothing was changed"

**Cause**: the file the computer downloaded isn't the file your home computer lists. The download was cut
short, or something between the two changed it.

**Fix**: run the line again. Nothing was installed, so it is safe to repeat.
