---
description: Move the home computer to another of your computers without pairing anything again, back it up behind a recovery code, and restore it on a new computer when the old one is gone.
---

# Move or replace the home computer

<p class="sb-meta">Last validated: Nov 24, 2026</p>

!!! note ""
    This arrived with device keys, in Switchboard 0.9.0, and is command line only. It needs keys
    turned on.

The home computer is the one of your computers that runs Switchboard itself. You can move that job to another of
your computers, or bring it back from a backup on a new computer. Your other computers and phones follow by
themselves, with nothing to pair again.

## Why it matters

Every device is paired with the home computer's device key, not with its address or its name. So the home
computer is whichever computer holds that key and the home computer's files. Moving both moves the home computer.

## Benefits

- **Nothing pairs again**: computers and phones find the home computer by the same key, wherever it now is.
- **A short gap**: in a test between a Mac and a Windows PC, nobody answered as the home computer for about
  3 seconds.
- **Never two at once**: the old home computer stops answering before the other one starts. A move that is cut
  short leaves no home computer, not two, and one command finishes it.
- **A way back from a dead computer**: a backup file and its recovery code are enough to make a new home
  computer.

## Use cases

- **A new computer**: you bought a desktop that is on all day and want it to be the home computer.
- **A computer that is leaving**: you are selling or wiping the current home computer.
- **A computer that died**: the home computer's disk failed and you have a backup.

## How it works

A move has a fixed order. The home computer checks the other computer, stops its own server, packs its files and
sends them. It then gives itself a new device key and stops answering as home. Only after that does it send the
home computer's key, and the other computer takes over.

The old home computer stays paired as one of your computers, under its new key. With `--retire` it keeps no key
and leaves the list.

A backup is one file, sealed with a key made from a recovery code. A restore opens the file with that code on
another computer, which then answers as the home computer.

## Move the home computer

1. Prepare the computer that will take over.
2. Check that the move is possible.
3. Move.

### Prerequisites

Before you can move the home computer, you must:

- Have keys turned on. Refer to [Pair a computer](pair-a-computer.md#turn-keys-on).
- Have the other computer paired, connected and on the same release.

### Prepare the other computer

The other computer needs the parts a home computer runs. On a Mac or Linux computer, run the installer with
`--standby`:

```text
install-server.sh --standby
```

On Windows:

```text
install-server.ps1 -Standby
```

A standby install starts nothing. It ends with "Installed as a standby: nothing runs. This computer can take
over as the home computer."

### Check that the move is possible

On either computer:

```text
switchboard-node home status
```

On the other computer, the answer includes "It has the parts a home computer needs, so the home computer can
move here."

### Move

On the home computer:

```text
switchboard-node home move example-desktop
```

Replace `example-desktop` with the other computer's name as `switchboard-node devices` shows it.

Switchboard says "Moving the home computer. The server here stops now, and nobody answers as home until the
other computer has taken over." When it ends, it says "The home computer is now example-desktop." and how long
nobody answered.

To have the old home computer leave your computers altogether, add `--retire`:

```text
switchboard-node home move example-desktop --retire
```

After a move to a standby install, run the installer once more on the new home computer without `--standby`.
This is because a standby install doesn't start Switchboard when the computer starts.

### Finish or call off a move that stopped

If the move is interrupted, `switchboard-node home status` says "A move to example-desktop was started and is
not finished." To finish it:

```text
switchboard-node home move --resume
```

To call it off:

```text
switchboard-node home move --undo
```

Calling off works until the other computer has taken over. If Switchboard can't tell whether it has, it refuses,
and `--resume` is the way forward.

## Back up the home computer

On the home computer:

```text
switchboard-node home backup
```

The first backup prints a recovery code, 26 characters in groups of five:

```text
Recovery code: XXXXX-XXXXX-XXXXX-XXXXX-XXXXX-X
Write it down now. It is shown this once and is kept nowhere on this computer.
```

Later backups use the same code and print none. To choose where the file goes, add `--to` and a path:

```text
switchboard-node home backup --to /Volumes/example-disk/home.sbk
```

Without `--to`, the file goes into the folder `home-backup` in the home computer's state folder. Copy it to
another computer or disk. A backup that stays on the home computer is lost with it.

!!! note ""
    A backup file and its recovery code together are your home computer. Anybody who has both can become it.
    Keep them apart: the file on another disk, the code on paper.

## Restore the home computer

Restore on a computer that has the home computer's parts (a standby install, as above) and isn't a home
computer already.

```text
switchboard-node home restore home.sbk
```

Replace `home.sbk` with the path of the backup file. Type the recovery code when asked. The terminal shows the
code as you type it.

When it ends, the computer is the home computer as it was when the backup was made:

- Chats, settings and devices paired since the backup are lost. Pair those devices again.
- A device you removed after the backup is removed again as soon as another of your computers connects.

**Never start the old home computer after a restore**, and never restore one backup on two computers. If the old
one does start, it stands down by itself when it hears of the new one, and says "This computer was the home
computer until it moved. It is not answering as home."

## After a move

The old home computer's key is overwritten, but copies of its disk made before the move still hold it. That
includes Time Machine, File History, disk images, cloud sync of the home folder and a cloned disk. Delete the
folder `~/.config/switchboard` from those copies (on Windows, `%USERPROFILE%\.config\switchboard`), or keep them
as safe as the home computer itself.

Before you sell, give away or throw out a disk that held the home computer's key, wipe it, unless it was
encrypted all along with FileVault, BitLocker or LUKS. `--retire` alone isn't enough.

If somebody else may have copied the key, a move doesn't help. This is because the key that moves is the same
key. Set up a new home computer and pair every device again.

## Limitations

- There are no screens for this in the apps yet, and no scheduled backups.
- **A backup made before a move must not be restored while the newer home computer still runs.** The two don't
  find each other, and both keep answering as home. Stop the newer one for good first.
- An old home computer that starts from an old copy answers as home until it reaches one of your computers or a
  device reaches it. Until then, a device that reaches only that computer sees old chats.
- After a move, the old home computer's chat history, library and settings stay on its disk. Only the keys are
  wiped.
- A computer that took no part in the move keeps showing the home computer under its old name.
- A move that stops before the old home computer has stepped down leaves Switchboard stopped there until you run
  `--resume` or `--undo`.
- A Windows computer can take over or be restored to. A Windows home computer is still installed by hand.

## Troubleshooting

### It does not have the parts a home computer needs

Run the installer with `--standby` on that computer, then try again.

### Only this computer's home computer may ask that

The command was run on a computer that isn't the home computer. Run `switchboard-node home move` on the home
computer itself.

### The backup doesn't open

A wrong recovery code and a damaged file can't be told apart, and the message names both. Check the code first,
one group at a time. A restore that fails changes nothing.
