---
description: A Mac goes offline after its helper was updated, and the helper's log says No route to host. macOS took away the helper's Local Network permission.
---

# A Mac goes offline after its helper updates

<p class="sb-meta">Last validated: Nov 23, 2026</p>

A Mac was on the board, you updated its helper, and now the Mac says **Offline**. The helper's log,
`~/Library/Logs/switchboard-node.log`, says "No route to host".

## Cause

macOS asks before a program may talk to other devices on your local network, and it remembers the answer for
one exact program. A helper that isn't signed with a Developer ID counts as a new program after each update, so
the permission no longer applies to it. The helper then can't reach a home computer at an address on your home
network. The home computer is the one of your computers that runs Switchboard itself.

A helper signed with a Developer ID keeps its permission across updates. A helper you built yourself is not
signed that way.

## Fix

1. On the Mac, open System Settings, select **Privacy & Security**, then **Local Network**.
2. Turn on the switch for `switchboard-node`.
3. Wait for the Mac to show on the board. The helper keeps trying by itself.

If the Mac still can't connect, link it by an address that isn't on the local network, such as a Tailscale
name:

1. On the home computer, run `switchboard setup` and pick the Tailscale name as the address.
2. Add the Mac again with a new code. Refer to [Add your computers](../machines.md#add-a-computer-with-a-code).
