---
description: A Windows computer stays offline after a restart or a power cut until someone signs in. Why, and what to do with the chats that were running.
---

# A Windows computer doesn't come back after a restart

<p class="sb-meta">Last validated: Nov 23, 2026</p>

A Windows PC restarted, after an update or a power cut, and it stays **Offline** on the board. A chat that was
running there has a tab that says "Stopped when example-pc restarted", with the PC's own name.

## Cause

On Windows, the helper is a logon task named `SwitchboardNode`. It starts when someone signs in to Windows, not
when the PC starts. A PC that waits at the sign-in screen is on, but its helper isn't running.

## Fix

1. Sign in to Windows on the PC. The helper starts, and the PC shows on the board within a minute.
2. Select **Resume** on a chat that was stopped by the restart. Nothing is lost: the chat starts again where
   it was.

If the PC is signed in and still offline, refer to [A computer is offline or missing](computer-offline.md).
