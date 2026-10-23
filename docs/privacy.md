---
description: What Switchboard keeps on your own computers, the few things that leave them, and how to report a security problem.
---

# Privacy and security

Switchboard runs on your own computers. There is no account to make, no cloud service, and no usage data or crash
reports sent anywhere.

## What stays home

- **Your chats, files and history** stay on your computers and your server. The apps talk only to your server and
  its hub.
- **The app's log** stays on the computer it runs on. It records what the app does, not what your chats say, and your
  home folder is written as `~`. Nothing is sent unless you copy it out yourself (**About, Copy diagnostics**).
- **Private folders stay private.** The file browser hides dot files and private folders, and a file that looks
  like a key or a secret is listed but never opened: "This looks like a key or a secret, so Switchboard won't open
  it."
- **New folders need your yes.** Before Claude works in a folder it has not been in, Switchboard asks you first,
  the same question Claude Code would ask.

## What leaves your network, and when

| What | Goes to | When |
|---|---|---|
| The newest release's list of builds, and the builds | GitHub (the server fetches them) | An app checks for updates (the server asks GitHub at most every 5 minutes), or downloads one. [Updates](updates.md#where-updates-come-from) |
| A request for the link program and helper | GitHub (the new computer fetches them) | You add a computer with the app's line while your server has no downloads of its own |
| iPhone notifications: chat names, machine names and short status lines | Apple's push service | Only once you add an APNs key to the server. Each kind has its own switch |
| A waiting chat's last message | Claude, on your own plan | Only with **Summarize waiting chats** on |
| A new chat's first message | Claude, on your own plan | Only with **Name new chats** on |
| A task sent to Auto, and each machine's tools, free memory and repo names | Claude, on your own plan | Only with **Let Claude pick for Auto** on |
| A condensed copy of a chat | Claude, on your own plan | When you use **Start fresh**, **Continue** or **Compact into one chat**, which write a summary |

The three switches are in **Settings, AI and your Claude setup**, and use Haiku. Summaries use Opus, or Sonnet
if you pick it. All of them run Claude Code on your own computers (`claude -p`), on your own plan, like any
other chat.

!!! warning "If you used Switchboard before October 2026"
    The AI features and the setup changes below are **off** for a new server. A server that was already running
    when these settings arrived turned them all **on** once, so an install that had them kept them. Check
    **Settings, AI and your Claude setup** if you'd rather have them off.

## Changes to your Claude setup

Also in **Settings, AI and your Claude setup**, and also off until you turn them on. Each applies to every
computer:

| Setting | What it changes | Turning it off |
|---|---|---|
| **Subagent model rule** | Adds a short rule to `~/.claude/CLAUDE.md`: Haiku for single lookups, Sonnet for search and reading, the default model for design and code | Removes only the rule it added |
| **Explore agent on Sonnet** | Writes `~/.claude/agents/Explore.md`, so the Explore agent runs on Sonnet and only reads. One you wrote yourself is left alone | Deletes the file only if Switchboard wrote it |
| **Compact at 300k tokens** | Sets `CLAUDE_CODE_AUTO_COMPACT_WINDOW=300000` in `~/.claude/settings.json` | Removes it only if it's still Switchboard's value |

## No passwords, on purpose

Switchboard has no passwords. Anyone who can reach its ports (8130 to 8132) can read your chats, type into their
terminals and start new chats on your computers. It is meant for your own home network or your own
[Tailscale](https://tailscale.com) network.

!!! danger "Never forward ports 8130 to 8132 to the internet"
    Use Tailscale (or another private network) to reach Switchboard away from home.

What it does protect:

- **Linking a computer** takes a one-time code that works once and expires after 10 minutes.
- **A computer added over ssh** gets a key that can only run the Switchboard helper: no shell and no port
  forwarding.
- **The server answers only to its own names.** It checks the `Host` of each request against the addresses it
  knows, so a web page you visit can't reach it by renaming itself (DNS rebinding).
- **Updates are signed.** The app installs only builds signed with the release key, and checks each file's
  SHA-256. [Why you can trust an update](updates.md#why-you-can-trust-an-update)
- **Downloads are checked for damage.** `install-agent.sh` checks `sb.py` and `switchboard-node` against the
  server's `SHA256SUMS` when it has one, and `get-server.sh` checks the server kit against the `.sha256` beside it
  (or the one you pass with `--sha256`). The sums come from the same place as the files, so they catch a broken
  download, not a server that lies.
- **Nothing runs as an administrator.** The server, the helper, the node and the app install for your own
  account. A few optional steps need one, and you run them yourself: a Windows server's firewall rule, keeping a
  Linux server's services running after you log out (`loginctl enable-linger`), and adding a Windows computer
  over ssh when your account there is an administrator.

## Report a security problem

--8<-- "SECURITY.md:report"
