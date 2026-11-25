---
description: How each part of Switchboard updates, the app, the home computer and the helper on each computer, how the update key keeps a release genuine, and what happens when a new version doesn't start.
---

# Updates

<p class="sb-meta">Last validated: Nov 22, 2026</p>

Updates let you move each part of Switchboard to a new release. The app offers a new version by itself and
installs it when you say so. You update the home computer, the one of your computers that runs Switchboard
itself, with one button in the app, and the helper on each computer with one command.

## Benefits

- **Nothing installs by itself**: the app tells you a new version is ready, and you choose when it goes in.
- **A release can't be swapped**: the app checks every release against a key built into it, so nothing between
  the release and your computer can change what you install.
- **A bad update undoes itself**: when a new version of the app or of the home computer doesn't start, the
  version before it is put back.
- **Your setup stays**: an update never changes your settings, your computers or your history.

## How it works

Switchboard has three parts, and each one updates in its own way.

| Part | Where it runs | How it updates |
|---|---|---|
| The app | The computer you look at the board from | It looks for a new version once a day and offers it at the top of the window. |
| The home computer's software | Your home computer | You click **Update server** in the app's Settings. |
| The helper, `sb.py`, and the link program, `switchboard-node` | Every computer you added | You run the install line again on that computer. |

### Where updates come from {#where-updates-come-from}

The app never contacts the internet for an update. It asks your home computer, and the home computer reads the
newest release of [nicglazkov/switchboard](https://github.com/nicglazkov/switchboard/releases) on GitHub.

<figure class="sb-fig" markdown="0">
--8<-- "docs/diagrams/updates-flow.svg"
<figcaption>Figure 1. The app asks only your home computer. The home computer fetches the list from GitHub and
passes it on unchanged, and the app is the one that checks it.</figcaption>
</figure>

1. The app asks the home computer whether there is a newer version.
2. The home computer reads the newest release on GitHub. It keeps the answer for 5 minutes, so several apps
   asking at once cause one request.
3. GitHub returns `latest.json`, the list of builds, and `latest.json.sig`, the signature of that list.
4. The home computer passes both to the app exactly as it received them.
5. The app checks the signature with its update key and decides whether to offer the version.

The download takes the same path. The app downloads the new version through the home computer, which fetches the
file from the GitHub release. The figure doesn't draw this second trip.

### Why you can trust an update {#why-you-can-trust-an-update}

Every release is signed with a private key that only the publisher holds. The matching public key, known as the
update key, is built into the app. The app uses it in three checks:

1. **The signature**: the app ignores a list that isn't signed with the release key.
2. **The version**: the app offers a build only when its version isn't older and its build number is newer, so
   nothing can move you back to an old release.
3. **The file**: the app compares the downloaded file's size and SHA-256 with the signed list, and deletes the
   file if either is wrong.

This means neither your home computer nor GitHub can change what the app installs. The home computer's own update
relies on the same check: the app reads the expected SHA-256 from the signed list, and the home computer installs
the download only if it matches.

### Builds without updates {#builds-without-updates}

A build of the app that was made without an update key can't check that a new version is genuine, so it never
offers one. **Settings**, **Updates** says "Updates are off in this build" and shows **How builds get updates**
in place of **Check now**. A development build, which has no build number to compare, says the same.

The apps on the releases page have the key. If you build the app yourself, refer to
[Building and releasing](building.md#updates).

## Update the app

To update the app:

1. Check for a new version.
2. Choose when to install it.

### Prerequisites

Before you can update the app, you must:

- Run an app that has an update key. The apps on the releases page do.
- Have the app connected to your home computer, and the home computer able to reach GitHub.
- Keep the app in a folder that your account can change. If it can't write there, the app says so before it
  downloads anything.

### Check for a new version

The app looks 90 seconds after it opens, and then once a day. To look now, open **Settings**, select **Updates**,
and click **Check now**. When there is a new version, **Update ready** appears at the top of the window.

### Choose when to install it

Click **Update ready**, then pick one:

- **Restart to update**: downloads the new version, installs it and opens Switchboard again.
- **Install on quit**: downloads the new version now and installs it the next time you quit.
- **Later**: closes the panel. Nothing is downloaded.

**What's new** in the same panel opens the release's notes. While the update is in progress, the label at the top
of the window says **Downloading** with a percentage, **Installing** or **Installs on quit**. A download that was
interrupted continues from where it stopped. **Don't install** cancels an update that is waiting for you to quit.

Your chats keep running during an update. This is because they run on their own computers, not in the app.

## Update the home computer {#the-server}

Open **Settings** and select **Server**. When the newest release has a newer version of the home computer's
software, the **Updates** group says **Version 0.9.0 is ready**, with the real version number. Click
**Update server**.

The page shows four steps: **Download**, **Check**, **Install** and **Restart**. Your chats keep running, and the
app reconnects by itself when the home computer is back. **What's new** opens the release's notes.

!!! note ""
    A home computer on Windows, one you start by hand from a copy of the code, and one older than version 0.8.11
    can't update themselves from the app. Update them by hand.

### Update the home computer by hand

Updating by hand keeps your computers, settings and history.

=== "macOS and Linux"

    Run the install line again on the home computer:

    ```sh
    curl -fsSL https://github.com/nicglazkov/switchboard/releases/latest/download/get-server.sh | bash
    ```

    For a home computer older than 0.8.11, **Settings**, **Server** shows this line under **This server needs one
    update by hand**, with **Copy**. If you installed from a copy of the repository, you can get the new code and
    run `scripts/install/install-server.sh` again instead.

=== "Windows"

    Get the new version, as a copy of the repository or as the server kit from the release. Then run its installer
    in PowerShell, from that folder:

    ```powershell
    powershell -ExecutionPolicy Bypass -File scripts\install\install-server.ps1
    ```

=== "Started by hand"

    If you start `server/server.py` yourself from a copy of the repository, get the new code and start it again.

After the home computer is updated, the files it hands to computers you add are the new version's.

## Update the helper on a computer

The helper on each computer doesn't update by itself. The installer keeps a helper that is already there unless
you add `--update`.

=== "macOS and Linux"

    On the computer, run:

    ```sh
    curl -fsSL http://192.0.2.10:8131/install-agent.sh | bash -s -- --server 192.0.2.10 --update
    ```

=== "Windows"

    On the computer, in PowerShell, run:

    ```powershell
    & ([scriptblock]::Create((iwr -UseBasicParsing http://192.0.2.10:8131/install-agent.ps1).Content)) -Server 192.0.2.10 -Update
    ```

Replace `192.0.2.10` with your home computer's address.

This replaces `sb.py` with the one your home computer hands out and sets it up in Claude Code again. To replace
`switchboard-node` as well, get a new code with `switchboard setup --code` on the home computer and add
`--code` and the code (`-Code` in PowerShell) to the same line. If you gave the computer a name with `--name` when
you added it, pass the same name again. For more information, refer to [Add your computers](machines.md).

## Rolling back

Switchboard puts the old version back by itself when a new one doesn't start. There is no button that goes back
to an older release.

- **The app**: the old version is kept until the new one has run for 20 seconds. If the new one crashes or won't
  start before then, the next launch puts the old one back and opens it. The app tells you once, and it doesn't
  offer that build again by itself. To try it anyway, open **Settings**, select **Updates**, and click
  **Try it again**.
- **The home computer**: the old version is kept until the new one answers. If the new one hasn't answered after
  a minute, the old one is put back and started again, and **Settings**, **Server** says what happened.
  **Update server** tries again.

An update replaces only the program. Your settings aren't touched, so a version that was put back finds them as
they were.

To go back to an older version of the app on purpose, uninstall the app and install the older release. This is
because the installer refuses to replace a newer version with an older one.

## Change where updates come from

By default the home computer reads the releases of `nicglazkov/switchboard` on GitHub. You can change that on the
home computer:

- **Another GitHub repository**: set the environment variable `SWITCHBOARD_RELEASES` to `owner/name`.
- **No look at GitHub**: set `SWITCHBOARD_RELEASES` to `off`.
- **Your own download page**: write `{"url": "http://192.0.2.10/app"}` to `~/.config/switchboard/downloads.json`.
  The home computer then reads `latest.json` and `latest.json.sig` from that page instead of GitHub.

Replace `192.0.2.10/app` with the address of your page. The app still accepts only a list that is signed with its
own update key, wherever the list comes from. For more information, refer to
[Building and releasing](building.md#updates).

## Limitations

- **No single update for everything**: the app, the home computer and each computer's helper update separately.
- **The helper has no update button**: you run a command on each computer.
- **A Windows home computer can't update from the app**: you run its installer again.
- **Updates need the home computer**: the app looks for new versions through it, so an app that isn't connected
  doesn't find any.

## Troubleshooting

### Settings says "Couldn't look for a newer one"

The home computer couldn't reach GitHub or your download page, or the release has no list yet. The message says
which.

1. Check that the home computer has an internet connection.
2. Click **Check now** again. The app also tries again the next day.

### The top of the window says Update failed

The download or the install didn't finish, and the version you had is still in place.

1. Click **Update failed** to read why.
2. If the message says Switchboard can't update itself where it is, move the app to a folder of your own, such as
   Applications on a Mac.
3. Choose **Restart to update** again.

### The app says a version didn't start and the one before it was put back

The new version crashed or didn't open in its first 20 seconds, so Switchboard went back to the old one.

1. Keep using the version you have. The next release is offered as usual.
2. To try the same version once more, open **Settings**, select **Updates**, and click **Try it again**.
3. If it fails again, report it. Refer to [Get help](support.md).

### Update server isn't offered

1. Check the line under the version in **Settings**, **Server**. It says why this home computer can't update
   itself, for example because it runs on Windows.
2. If it says the app can't check releases, the app was built without an update key. Use an app from the releases
   page.
3. Update the home computer by hand.
