---
description: A Windows PC went offline after its helper was updated, and switchboard-node.exe is gone. Windows Defender removed it. How to put it back.
---

# Windows Defender removed the helper

<p class="sb-meta">Last validated: Nov 23, 2026</p>

A Windows PC went offline right after its helper was updated. `switchboard-node.exe` is missing from
`%LOCALAPPDATA%\Switchboard`, and Windows Security shows a threat it removed.

## Cause

Before version 0.8.19, updating the helper on Windows also set up its logon task again. Windows Defender could
take that for malware and remove the helper, which left the PC offline. Since 0.8.19, an update only swaps the
program and leaves the logon task alone.

## Fix

1. Update your home computer, the one of your computers that runs Switchboard itself, so that it hands out a
   helper of version 0.8.19 or newer. In the app, open **Settings**, select **Server**, then **Update server**.
2. On the PC, open Windows Security, select **Virus & threat protection**, then **Protection history**. If
   `switchboard-node.exe` is listed, allow it.
3. Add the PC again with a new code. Refer to [Add your computers](../machines.md#add-a-computer-with-a-code).
4. On the PC, run `switchboard-node status` to check that it is linked.
