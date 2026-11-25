---
description: The phone app can't join the home computer with a code, or a paired phone says Can't reach home. What each message means and how to fix it.
---

# A phone can't join or can't reach home

<p class="sb-meta">Last validated: Nov 22, 2026</p>

!!! note ""
    Device keys arrived with Switchboard 0.9.0. The iPhone and Android apps that pair are not
    released yet.

The app on your phone doesn't get past **Join your home computer**, or a phone that was paired says **Can't
reach home**. Find the message the app shows below.

## The phone can't join

### The camera doesn't scan

The app says "Switchboard isn't allowed to use the camera. Allow it in Settings, or type the code."

1. Select **Open Settings** and allow the camera for Switchboard.
2. Or select **Type the code instead** and enter the code by hand.

### A short code finds your home computer through a relay

The app says "A short code finds your home computer through a relay, and this phone has none set. Scan the QR
code or paste the link instead."

**Cause:** a short code is too short to say where the home computer is, and the phone has no relay to look for
it at.

**Fix:** scan the QR code beside the short code, or paste the link. Both name the relay.

### That link has no relay in it

The app says "That link has no relay in it, so it can't find your home computer from here. Set a relay on the
home computer and get a new link, or connect by address."

**Cause:** the home computer has no relay set. A phone finds the home computer through one.

**Fix:**

1. On the home computer, open **Settings**, select **Devices**, and under **Relay** select **Change**. Refer to
   [Run your own relay](../features/run-your-own-relay.md).
2. Select **Add a device** to get a new code, and scan it.

To use the phone without a relay, select **Connect by address** at the bottom of the join screen.

### The code didn't match, or has expired

The app says "That code didn't match. Check it against the one on your home computer and try again." or "That
code has expired or was already used. On your home computer, choose Add a device to get a new one."

A code works once, for ten minutes, and five wrong tries end it. On the home computer, select **Add a device**
again and scan the new code.

### The phone doesn't find the home computer

The app says "Couldn't reach your home computer. Check that it is on and online, then try again. If the code
keeps failing, paste the link instead."

1. Check that the home computer is on and online.
2. On the home computer, open **Settings**, select **Devices**, and check that **Relay** doesn't say **Not
   connected**.
3. Get a new code and try again.

## A paired phone can't reach home

The app shows the last board it saw, with its age, and keeps trying by itself. To see why, open **Settings** and
select **Home computer**.

| The page says | Cause | Fix |
|---|---|---|
| **Can't reach home: it isn't answering.** | The home computer is off, asleep or offline | Check that the home computer is on and online. The phone keeps trying |
| **Can't reach home: the relay isn't answering.** | The phone can't reach the relay | Check that the home computer is on and online. If it is, check the relay. Refer to [Run your own relay](../features/run-your-own-relay.md#check-that-it-works) |
| **Can't reach home: it no longer lists this phone.** | The phone was removed on the home computer | On the home computer, select **Add a device**, and join again on the phone with the new code |
| **Can't reach home: no relay is set and it isn't on this network.** | The phone paired without a relay and has left the home network | Join again with a link from the home computer once it has a relay set |

A reply you send in that time waits on the phone for up to a day. Refer to
[Pair a phone](../features/pair-a-phone.md#use-the-app-when-the-home-computer-is-out-of-reach).
