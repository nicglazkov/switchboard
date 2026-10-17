# Install Switchboard

Switchboard shows the Claude Code chats on all your computers in one place, in the desktop app. It has two parts besides the app:

- **The server**, on one computer that stays on. It keeps the list of your computers and chats, so the board is
  there even when your laptop is closed. A Mac mini, a desktop that never sleeps or a small Linux box all work. If
  you only have one computer, it can be the server too.
- **The helper**, on each computer where you use Claude Code. It's a small Python program, `sb.py`, plus
  `switchboard-node`, which links the computer to the server.

Setup takes three steps, about ten minutes in all.

## What you need

| | Server machine | Each Claude Code machine |
|---|---|---|
| System | macOS or Linux (best), or Windows (see the note below) | macOS, Linux or Windows |
| Python | 3.9 or newer | 3.9 or newer |
| Claude Code | not needed | installed and signed in |
| Other | Rust, only if you have no ready-made `switchboard-hub` ([rustup.rs](https://rustup.rs)) | nothing |

Macs have Python once the command line tools are installed (`xcode-select --install`). Most Linux systems come
with it. On Windows, run `winget install -e --id Python.Python.3.12`.

**Network.** Your devices must be able to reach the server on ports 8130 (the apps), 8131 (the
downloads for adding machines) and 8132 (terminals and linked machines). A home network or Tailscale both work.
Switchboard has no passwords: anyone who can reach those ports can use it. Keep it on your own network or
tailnet, and never forward those ports to the internet.

**A Windows server** reaches machines over ssh only. The hub, which does terminals and linking with a code, doesn't
run on Windows yet. If you can, put the server on a Mac or a Linux machine. Any Windows PC can still be one of
your machines.

## 1. Install the server

Get a copy of Switchboard on the server machine, then run the installer from it:

```sh
git clone https://github.com/nicglazkov/switchboard.git
cd switchboard
scripts/install/install-server.sh
```

On Windows, in PowerShell:

```powershell
git clone https://github.com/nicglazkov/switchboard.git
cd switchboard
powershell -ExecutionPolicy Bypass -File scripts\install\install-server.ps1
```

The installer needs no `sudo` and changes nothing outside your own account. It:

- puts the server in `~/.local/share/switchboard/app`;
- builds the hub with Rust, or uses one you give it with `--hub-binary`;
- sets up three services that start when you log in: the server (8130), the hub (8132) and the downloads (8131).
  These are systemd user services on Linux and LaunchAgents on macOS. On Windows they are logon tasks;
- adds the `switchboard` command (`~/.local/bin/switchboard`);
- runs first-run setup.

On Linux, to keep Switchboard running when you're logged out, run `sudo loginctl enable-linger $USER` once.

## 2. First-run setup

Setup runs at the end of the installer. Run it again any time with `switchboard setup`. It asks three things.

1. **The server's address.** This is what the app and your other computers use to reach the server. It
   lists the addresses it finds: this machine on your network, its Tailscale name if Tailscale is running, and
   its own name. Pick one, or type another. The Tailscale name works away from home too.
2. **Your machines.** Add each machine where you use Claude Code, in one of two ways:
   - **With a code** (recommended). Setup shows a code and one line to paste on the new machine, for its system.
     The line installs the helper and links the machine to the server. No ssh is needed, and the machine can be
     behind another router. The code works once, for 10 minutes. To add the server machine itself, paste the
     line there.
   - **Over ssh.** Setup shows a line to paste on the machine. It installs the helper and lets the server in with
     a key that can only run the helper (no shell). Then you type `user@host`. The machine needs an ssh server
     running: on a Mac, turn on Remote Login (System Settings, General, Sharing).
3. **Each machine's Claude folder.** This is where new sessions start and where folders are picked from. Setup
   lets you browse the machine's folders, starting from its home folder. You can change it later in Settings,
   Machines.

Setup also has options for scripts. Run `switchboard setup --help` to see them all:

```sh
switchboard setup --address studio.local      # use this address
switchboard setup --code                      # print a new code and the lines to paste
switchboard setup --ssh tower=me@tower.local  # add a machine reached over ssh
switchboard setup --root tower=~/Code         # set a machine's Claude folder
```

## 3. Open Switchboard

Open the desktop app. If the computer that stays on is on the same network, the app finds it by itself (Bonjour;
on a Linux server, install `avahi-utils` or `python3-zeroconf`). Otherwise it asks for the address you picked in
setup, such as `192.0.2.10` or a Tailscale name. Change it later in Settings.

To point the app somewhere for one run, set `SWITCHBOARD_URL=http://<address>:8130` before starting it.

## Adding a machine later

Run `switchboard setup` on the server, or use **Add a machine** in Settings. To get a code without the menus,
run `switchboard setup --code`. The pasted lines look like this:

```sh
# macOS and Linux
curl -fsSL http://<server>:8131/install-agent.sh | bash -s -- --server <server> --code <code>
```

```powershell
# Windows (PowerShell)
& ([scriptblock]::Create((iwr -UseBasicParsing http://<server>:8131/install-agent.ps1).Content)) -Server <server> -Code <code>
```

If you would rather read the script before running it, download it first. The helper installer is
`scripts/install/install-agent.sh`, or `install-agent.ps1` on Windows. It takes `--server`, `--code`, `--ssh-key`,
`--name` and `--label`. It keeps an helper that is already installed unless you pass `--update`.

## Everyday commands

```sh
switchboard status     # is everything running?
switchboard logs       # the last lines of each part's log
switchboard restart
switchboard setup
```

On Windows, the same commands are `powershell -File %LOCALAPPDATA%\Switchboard\switchboard.ps1 <command>`.

**Updating.** Get the new version (`git pull`) and run the installer again. Your machines, settings and history
stay.

**Uninstalling.** Run `scripts/install/install-server.sh --uninstall`, or `install-server.ps1 -Uninstall` on
Windows. This keeps your settings and history; delete `~/.config/switchboard` and `~/.local/share/switchboard`
too if you don't want them. On a machine, `python3 ~/.claude/switchboard/sb.py uninstall` takes the helper out of
Claude Code's settings.

## Where things are

| What | Where |
|---|---|
| Settings: your machines, the server's addresses | `~/.config/switchboard/` (`fleet.json`, `server.json`) |
| History and state | `~/.local/share/switchboard/` |
| The server itself | `~/.local/share/switchboard/app/` (Windows: `%LOCALAPPDATA%\Switchboard\app\`) |
| What new machines download | `~/.local/share/switchboard/downloads/` |
| Logs | `switchboard logs`. The files are in `journalctl --user` on Linux, `~/Library/Logs/Switchboard/` on macOS, and `~/.local/share/switchboard/logs/` otherwise |
| The helper, on each machine | `~/.claude/switchboard/sb.py` |
| The link to the server, on each machine | `~/.config/switchboard/node.json` |

## When something doesn't work

| What you see | What to do |
|---|---|
| "Python 3 is not installed" | Install it with the command the script shows, then run the script again. |
| "Rust is not installed, so the hub can't be built" | Switchboard still works, but without terminals and without linking machines with a code. Install Rust from [rustup.rs](https://rustup.rs) and run the installer again, or add machines over ssh. |
| "the server has no switchboard-node for mac" (or windows) | The server can only build the node for its own system. Copy a ready-made `switchboard-node-mac` or `switchboard-node.exe` into `~/.local/share/switchboard/downloads/node/` (from a release, or from `native/scripts/build-mac.sh` or `build-windows.sh`), then run `install-server.sh --node-dir <folder>`. Until then, add that machine over ssh. |
| "That code is wrong or has expired" | Codes work once, for 10 minutes. Get a new one with `switchboard setup --code`. |
| The app can't reach the server | Check that the address you picked is reachable from that device, and that a firewall allows ports 8130 to 8132. On Windows, add the firewall rule the installer printed. Run `switchboard setup` to pick another address. |
| A machine shows "not answering" | On that machine: check `~/.local/share/switchboard/node.log` if it's linked with a code, or that its ssh server is on if it's added over ssh. Then check `~/.claude/switchboard/sb.log`. |
| A Linux machine with no systemd (a container, WSL without systemd) | The installers start everything in the background instead. Start it again after a restart with `switchboard start` (server) or `~/.local/bin/switchboard-node run &` (machine). |
