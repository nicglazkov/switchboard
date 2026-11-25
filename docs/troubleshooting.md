---
description: Find the fix for a Switchboard problem by its symptom, from an installer that stops to a computer that goes offline, a chat that turns cold or a board you can't reach.
---

# Troubleshooting

<p class="sb-meta">Last validated: Nov 22, 2026</p>

This page lists the problems people run into with Switchboard, grouped by where they show. Each link leads to a
page with the cause and the fix.

## Start here

Most problems show in one of two places. On your home computer, the one of your computers that runs Switchboard
itself, run:

```sh
switchboard status
```

It says whether the server, the hub and the downloads are running. `switchboard logs` shows the last lines of
each. On a Windows home computer, run
`powershell -File %LOCALAPPDATA%\Switchboard\switchboard.ps1 status` in their place.

On a computer you added with a code, run:

```sh
switchboard-node status
```

It says which home computer the helper is linked to.

## Installing

- [The installer stops with a message](troubleshooting/installer-stops.md)
- [A code is wrong or has expired](troubleshooting/code-wrong-or-expired.md)
- [Switchboard isn't running after a restart](troubleshooting/not-running-after-restart.md)
- [Windows says "Windows protected your PC"](apps.md#windows-says-windows-protected-your-pc)

## Connecting

- [The app can't find the home computer](troubleshooting/app-cant-find-home.md)
- [The board is out of reach away from home](troubleshooting/away-from-home.md)
- [Settings are back to defaults after a power cut](troubleshooting/settings-reset-after-power-cut.md)

## Computers

- [A computer is offline or missing](troubleshooting/computer-offline.md)
- [A Mac goes offline after its helper updates](troubleshooting/mac-offline-after-update.md)
- [Windows Defender removed the helper](troubleshooting/windows-defender-removed-helper.md)
- [A Windows computer doesn't come back after a restart](troubleshooting/windows-after-restart.md)
- [The home computer refuses a computer](troubleshooting/hub-refused-computer.md)

## Chats

- [A chat says cold sooner than you expect](troubleshooting/chat-cold-too-soon.md)
- [A message was held](features/cold-chat-guard.md#troubleshooting)
- [A chat can't be answered from the board](features/answer-from-the-board.md#troubleshooting)
- [A new chat doesn't start](features/start-a-chat.md#troubleshooting)
- [Every message is blocked by the hard limit](features/daily-budget.md#troubleshooting)

## The app

- [The board](board.md#troubleshooting)
- [Terminals](terminals.md#troubleshooting)
- [Files](features/files.md#troubleshooting)
- [History](features/history.md#troubleshooting)
- [Notifications](features/notifications.md#troubleshooting)
- [Usage](usage.md#troubleshooting)
- [Updates](updates.md#troubleshooting)
- [The apps](apps.md#troubleshooting)

## Device keys

!!! note ""
    Device keys arrived with Switchboard 0.9.0 and are off until you turn them on.

- [Pair a computer](features/pair-a-computer.md#troubleshooting)
- [A phone can't join or can't reach home](troubleshooting/phone-cant-join.md)
- [Run your own relay](features/run-your-own-relay.md#troubleshooting)

## Still stuck

Open **About Switchboard**, select **Copy diagnostics**, and open an issue with what it copied. Refer to
[Get help](support.md).
