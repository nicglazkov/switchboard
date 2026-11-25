---
description: A computer stays offline and its helper's log says the hub does not know this machine's token any more. Why the home computer refuses it and how to link it again.
---

# The home computer refuses a computer

<p class="sb-meta">Last validated: Nov 23, 2026</p>

A computer stays **Offline**, and the log of its helper says "The hub does not know this machine's token any
more."

## Cause

Each computer added with a code holds a token. Your home computer, the one of your computers that runs
Switchboard itself, checks it each time the helper connects, and here it refused the token. One reason is that
the computer was removed in **Settings**, **Machines**.

A helper that is refused doesn't give up. It asks again after 5 minutes, then less often, up to every 30
minutes. A refusal that was a passing fault therefore clears by itself.

## Fix

1. Wait for the next try. If the computer comes back, you are done.
2. If it is refused again, get a new code. In the app, open **Settings**, select **Machines**, then **Add a
   machine**.
3. On the computer, link it again with the command the log shows:

    ```sh
    switchboard-node pair 192.0.2.10:8132 ABC234
    ```

    Replace `192.0.2.10` with your home computer's address and `ABC234` with your code.

## When the log names another computer

If the log says "another computer is using this machine's link", two computers hold the same token. This
happens after you copy a disk, restore a backup onto a second computer, or copy `node.json`. The helper stops
taking the link back after three times in a row. Link each of the two computers again with a code of its own,
as in step 3.
