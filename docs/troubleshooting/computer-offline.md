---
description: A computer says Offline on the board, shows gray cards, or never showed up. How to find out why and bring it back.
---

# A computer is offline or missing

<p class="sb-meta">Last validated: Nov 23, 2026</p>

A computer's row on the board says **Offline**, its cards are gray with a note such as "As of 2m ago", or the
computer never showed up after you added it.

## Cause

The home computer, the one of your computers that runs Switchboard itself, has stopped hearing from the helper
on that computer. A computer that hasn't reported for 45 seconds shows what Switchboard last knew, in gray. The
usual reasons are:

- The computer is off or asleep.
- The helper isn't running there.
- The computer can't reach the address it was added with.

Nothing is lost. The computer's chats come back when it does.

## Fix

1. Check that the computer is on, awake and on the network.
2. On the home computer, check that Switchboard is running:

    ```sh
    switchboard status
    ```

3. On the computer, ask the helper which home computer it is linked to:

    ```sh
    switchboard-node status
    ```

    If the address it prints is no longer your home computer's address, add the computer again with a new code.
    Refer to [Add your computers](../machines.md#when-the-address-changes).

4. Read the helper's log for the reason:

    - **macOS**: `~/Library/Logs/switchboard-node.log`
    - **Linux**: run `journalctl --user -u switchboard-node`. Without systemd, read
      `~/.local/share/switchboard/node.log`.
    - **Any system**: `~/.claude/switchboard/sb.log` is the log of `sb.py`.

5. For a computer added over ssh, check that its ssh server is on and that `ssh` to it works from the home
   computer.

## Related pages

- [A Mac goes offline after its helper updates](mac-offline-after-update.md)
- [Windows Defender removed the helper](windows-defender-removed-helper.md)
- [A Windows computer doesn't come back after a restart](windows-after-restart.md)
- [The home computer refuses a computer](hub-refused-computer.md)
