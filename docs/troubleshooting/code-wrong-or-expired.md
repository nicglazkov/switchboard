---
description: Adding a computer stops with That code is wrong or has expired, or linking failed. Why a code stops working and how to get a new one.
---

# A code is wrong or has expired

<p class="sb-meta">Last validated: Nov 23, 2026</p>

The line you pasted on a new computer stops with "That code is wrong or has expired", followed by "linking
failed".

## Cause

A code works once, for 10 minutes. This is because the code is all a computer needs to join your home computer,
the one of your computers that runs Switchboard itself. A code stops working when:

- More than 10 minutes passed since it was made.
- It was already used, by this computer or another one.
- It was typed wrong.

## Fix

1. Get a new code. In the app, open **Settings**, select **Machines**, then **Add a machine**. Or run this on
   the home computer:

    ```sh
    switchboard setup --code
    ```

2. Copy the whole line for the computer's system, and paste it there within 10 minutes.
3. Wait for the app or setup to show the computer as linked.

Each computer needs a code of its own. To add several, get a new code for each one.
