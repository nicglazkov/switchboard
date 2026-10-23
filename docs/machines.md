---
description: Link each computer where you use Claude Code to your Switchboard server, with a one-time code or over ssh.
---

# Add your computers

Each computer where you use Claude Code gets the helper: `sb.py`, which is Claude Code's status line and hooks
there, and `switchboard-node`, which keeps a link open to the server. There are two ways to add one.

<div class="grid cards" markdown>

-   :material-numeric:{ .lg .middle } **With a code** (recommended)

    ---

    Get a one-time code, paste one line on the new computer. No ssh is needed, and the computer can be behind
    another router. Needs the hub, so a Mac or Linux server.

-   :material-key-variant:{ .lg .middle } **Over ssh**

    ---

    The server reaches the computer with an ssh key that can only run the helper, never a shell. The computer
    needs an ssh server running. Works with any server, including Windows.

</div>

## With a code

Get a code in any of these places:

- in the app: **Settings, Machines, Add a machine**, the **Add a machine** row in the command palette
  (++cmd+k++ on a Mac, ++ctrl+k++ elsewhere), or **Add a machine** in Files;
- on the server: `switchboard setup`, or `switchboard setup --code` for just the code and the lines.

The code works **once, for 10 minutes**. Pick the new computer's system and paste the line there, in a terminal
(PowerShell on Windows). The computer shows up on the board within a minute.

=== "Mac and Linux"

    The line `switchboard setup` shows:

    ```sh
    curl -fsSL http://<server>:8131/install-agent.sh | bash -s -- --server <server> --code <code>
    ```

=== "Windows"

    The line `switchboard setup` shows, for PowerShell:

    ```powershell
    & ([scriptblock]::Create((iwr -UseBasicParsing http://<server>:8131/install-agent.ps1).Content)) -Server <server> -Code <code>
    ```

The installer it runs (`install-agent.sh`, or `install-agent.ps1` on Windows):

1. checks for Python 3.9 or newer, and warns when Claude Code is not installed;
2. downloads `sb.py` and `switchboard-node` from the server's downloads (port 8131) and checks them against the
   server's `SHA256SUMS`;
3. installs the helper into Claude Code's settings (`sb.py install`), keeping one that is already there unless you
   pass `--update`;
4. links the computer (`switchboard-node pair <server>:8132 <code>`) and starts the node at login
   (`switchboard-node install`): a LaunchAgent on a Mac, a systemd user service on Linux, a logon task on Windows.
   None of these needs an administrator.

The line the **app** shows is shorter: it downloads `switchboard-node` straight from the server's downloads (or,
when the server has none, from the newest GitHub release), links it and installs it, and fetches `sb.py` only
when the computer has none.

!!! tip "Read the script first"
    To read the installer before running it, download it from `http://<server>:8131/install-agent.sh` (or
    `install-agent.ps1`) and run it yourself. It takes `--server`, `--code`, `--ssh-key`, `--name`, `--label`,
    `--sb`, `--node` and `--update` (`-Server`, `-Code`, `-SshKey`, `-Name`, `-Label`, `-Sb`, `-Node`, `-Update`
    in PowerShell).

### Adding the server itself

If you use Claude Code on the server too, paste the line there like on any other computer.

## Over ssh

In `switchboard setup`, choose ssh. Setup shows a line to paste on the computer: it installs the helper and adds
the server's key (`~/.ssh/switchboard_ed25519.pub`) to the computer's `authorized_keys` with a `command=` that
only runs the helper, with no port forwarding. Then type `user@host` in setup.

The computer needs an ssh server running:

- **Mac:** System Settings, General, Sharing, turn on **Remote Login**.
- **Linux:** `openssh-server`.
- **Windows:** the OpenSSH server feature. For an administrator account, Windows reads the key from
  `%ProgramData%\ssh\administrators_authorized_keys`, which is where the installer puts it.

From a script:

```sh
switchboard setup --ssh tower=me@tower.local --root tower=~/Code
```

## The Claude folder

Each computer has a Claude folder: where new chats start, and where the folder picker starts. Setup asks for it,
and **Settings, Machines** changes it later. With `sb.py places` on the computer you can add more places the file
browser and new chats may use:

```sh
python3 ~/.claude/switchboard/sb.py places list
python3 ~/.claude/switchboard/sb.py places add ~/Work --name Work
python3 ~/.claude/switchboard/sb.py places remove Work
python3 ~/.claude/switchboard/sb.py places roam on    # anywhere in the home folder
```

## Removing a computer

Remove it in **Settings, Machines**. On the computer itself:

```sh
python3 ~/.claude/switchboard/sb.py uninstall   # takes the helper out of Claude Code's settings
```

`sb.py uninstall` removes only what it added (on Windows, also its logon task `SwitchboardLaunch`). To stop the node, remove its login item: on a Mac
`launchctl bootout gui/$(id -u)/com.switchboard.node` and delete
`~/Library/LaunchAgents/com.switchboard.node.plist`; on Linux `systemctl --user disable --now switchboard-node`;
on Windows `schtasks /Delete /TN SwitchboardNode /F`.
