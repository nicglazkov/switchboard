---
description: The app or your phone connects at home and not from another network. Why, and how to reach your home computer with Tailscale or with device keys.
---

# The board is out of reach away from home

<p class="sb-meta">Last validated: Nov 22, 2026</p>

The app on your laptop or phone works at home. On another network, it can't connect, or a laptop you took
with you says **Offline** on the board.

## Cause

Your home computer, the one of your computers that runs Switchboard itself, answers at an address on your home
network. That address means nothing on another network. Switchboard has no passwords, so its ports must never
be forwarded to the internet.

## Fix

Today, use a VPN (virtual private network) such as [Tailscale](https://tailscale.com).

1. Install Tailscale on the home computer and on each device you take with you, signed in to the same account.
2. At home, open the app once. The home computer passes on its Tailscale address, and the app keeps it. To
   check, open **Settings**, select **Server**, then **Server addresses**.
3. Away from home, turn Tailscale on on the device, then open the app.

A laptop that runs chats needs one more step. This is because a computer added with a code keeps the one
address it was added with.

1. On the home computer, run `switchboard setup` and pick the Tailscale name as the address.
2. Add the laptop again with a new code. Refer to [Add your computers](../machines.md#add-a-computer-with-a-code).

## With device keys

Device keys remove the need for a VPN. A paired computer or phone finds the home computer by its key from any
network. The phone apps that pair are not released yet. Refer to
[Pair a computer](../features/pair-a-computer.md) and [Pair a phone](../features/pair-a-phone.md).
