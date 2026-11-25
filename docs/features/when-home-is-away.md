---
description: What still works when the home computer is off or out of reach. Each paired computer answers for its own chats, the board says what is missing, and you can block a device on one computer.
---

# When the home computer is away

<p class="sb-meta">Last validated: Nov 24, 2026</p>

!!! note ""
    This arrived with device keys, in Switchboard 0.9.0. It needs keys turned on and every
    computer on that release.

When the home computer, the one of your computers that runs Switchboard itself, is off or can't be reached, your
other computers answer for their own chats. You still see those chats, open their terminals and reply to them.
What only the home computer holds waits until it is back.

## Why it matters

Without this, one computer that is asleep, restarting or offline hides every chat on every other computer.
With it, a chat is out of reach only when the computer it runs on is.

## Benefits

- **Chats stay in reach**: a chat that runs on a computer you can reach is on the board, whether or not the home
  computer answers.
- **Nothing to switch on**: the board changes by itself about 20 seconds after the home computer stops
  answering, and changes back when it answers again.
- **The same trust**: a computer answers only the devices on the home computer's list, exactly as the home
  computer would.

## How it works

The home computer keeps the list of paired devices. Each time the list changes, the home computer signs it with
its device key and every paired computer and phone takes a copy. For more information, refer to
[Device keys and pairing](../concepts/device-keys-and-pairing.md).

When a computer's helper hasn't heard from the home computer for 20 seconds, it stops waiting. It shows the app
on that computer its own chats, and asks the other computers on the list for theirs. A phone does the same from
the app.

A computer answers a device only when all of these are true:

- The device is on the signed list that computer holds.
- That copy of the list is less than 7 days old.
- The device isn't blocked on that computer.

Nobody but the home computer can change the list. This is because the list is only accepted with the home
computer's signature, and no other computer holds that key.

## What you see

The board's status line becomes one banner. It reads the same in the desktop app, on an iPhone and on an Android
phone:

| Banner | Meaning |
|---|---|
| **Home is away. Showing chats from Laptop and Tower.** | Those computers answer. Their chats are on the board |
| **Home is away. Asking the other computers for their chats.** | The first answers haven't arrived yet |
| **Home is away. No other computer can be reached.** | No computer on the list answers this device |

A computer whose chats aren't showing keeps its row, with the reason in it:

| Under its name | Reason |
|---|---|
| **Away** | It is the home computer |
| **Not reachable** | It is off, asleep or on a network this device can't reach |
| **Needs an update** | Its helper is older than the release that brings this |
| **Not answering** | It is on the list, and it refuses this device. This device is blocked there, or its copy of the list is too old |

## What works, and what waits

| Works while the home computer is away | Waits for the home computer |
|---|---|
| The board, for computers that answer | **History** |
| A chat's terminal | **New chat** and **Start fresh** |
| **Resume**, **Send anyway**, **Stop** and **Dismiss** | **Files** |
| Replies to a chat on a computer that answers | Plan usage and the budget |
| | Pairing and removing devices |

What waits is drawn dimmed. Point at it, or tap it on a phone, and Switchboard says the reason: "The home
computer is away. This needs the home computer."

**Start fresh** waits because the handoff note is written on the home computer. The chat's sheet says "Start
fresh needs the home computer. It is away, so resuming is what works now."

On a phone, a reply to a chat whose computer doesn't answer is held on the phone and sent when it can be, for up
to a day.

## Block a device on one computer

Removing a device needs the home computer. When the home computer is away and you no longer trust a device, for
example a lost laptop, block it on each computer that should stop answering it.

### In the desktop app

1. On a computer that is paired with the home computer, open **Settings** and select **Devices**.
2. Find the card **While the home computer is away**. It lists every device on this computer's copy of the list
   and says how old the copy is.
3. Select **Block** next to the device. The row then says **Blocked here**.

To undo it, select **Unblock**.

### From the command line

```text
switchboard-node block example-laptop
```

Replace `example-laptop` with the device's name as `switchboard-node devices` shows it. To undo it:

```text
switchboard-node unblock example-laptop
```

A block applies to that one computer, and it lasts until the home computer's list changes. When the home computer
is back, remove the device there so that every computer refuses it. Refer to
[Pair a computer](pair-a-computer.md#remove-a-device).

The home computer and the computer you are at can't be blocked.

## Limitations

- A chat on the home computer itself is out of reach while it is away.
- A computer that hasn't reached the home computer for 7 days stops answering every device. Its status says:
  "The home computer's list of devices is more than 7 days old, so no device is answered by this computer until
  it reaches the home computer again."
- A device whose own copy of the list is 7 days old is no longer answered by the other computers. Start the home
  computer, or replace it. Refer to [Move or replace the home computer](move-or-replace-home.md).
- A computer that was never connected while keys were on holds no list yet, so it answers nobody until it has
  reached the home computer once.
- For up to about 30 seconds after the home computer comes back, another computer can show as offline on the
  board while its helper reconnects.
- The board refreshes every 5 seconds while the home computer is away. It doesn't update the moment something
  changes.

## Troubleshooting

### The banner says no other computer can be reached

The other computers are off, on a network this device can't reach without a relay, or on an older release.
Check each row for its reason. For more information about relays, refer to
[Connections](../concepts/connections.md).

### A computer says it needs an update

Update Switchboard on that computer. Refer to [Updates](../updates.md).

### The banner stays after the home computer is back

Check that Switchboard is running on the home computer:

```text
switchboard status
```

If it is, the device can't reach the home computer from the network it is on. Refer to
[The board is out of reach away from home](../troubleshooting/away-from-home.md).
