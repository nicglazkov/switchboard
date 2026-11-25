---
description: The app says No Switchboard server on this network, or can't connect to the address you typed. What to check on the home computer and the network.
---

# The app can't find the home computer

<p class="sb-meta">Last validated: Nov 23, 2026</p>

The app says "No Switchboard server on this network", or it can't connect to the address you typed.

## Cause

The app searches the network it is on for the home computer, the one of your computers that runs Switchboard
itself. The search fails when:

- The home computer is off, or Switchboard isn't running on it.
- The app is on another network, or the network blocks the search.
- A firewall on the home computer blocks ports 8130 to 8132.

## Fix

1. On the home computer, check that Switchboard is running:

    ```sh
    switchboard status
    ```

    If a part says `not answering`, run `switchboard restart`, then `switchboard logs` to see why it stopped.

2. In the app, select **Search again**.
3. Select **Type an address** and enter the home computer's address, such as `http://192.0.2.10:8130`. Replace
   `192.0.2.10` with your home computer's address. To see or change the address the home computer gives out,
   run `switchboard setup` on it.
4. Check that a firewall on the home computer lets your device reach ports 8130, 8131 and 8132. On a Windows
   home computer, the installer prints a firewall rule for ports 8130 and 8131. Run it in PowerShell as
   administrator.

Once the app has connected, it keeps every address the home computer has. Open **Settings**, select **Server**,
then **Server addresses** to see them.

If the app works at home and not elsewhere, refer to
[The board is out of reach away from home](away-from-home.md).
