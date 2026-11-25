---
description: What Switchboard keeps on your own computers, what crosses the network, what a relay, Apple and Google can see, and how to report a security problem.
---

# Privacy and security

<p class="sb-meta">Last validated: Nov 22, 2026</p>

Switchboard runs on your own computers. There is no account to make and no Switchboard service in the cloud, and
it sends no usage data or crash reports anywhere. This page lists what is kept where, what crosses the network,
and what anyone outside your devices can see.

Switchboard connects your devices in one of two ways. By address is how it works until you choose otherwise. By device key arrived
with version 0.9.0 and is off until you turn it on. Where the two differ, this page says so.

## What stays home

- **Your chats, files and history** stay on your computers. The apps talk only to your home computer, the one of
  your computers that runs Switchboard itself.
- **The app's log** stays on the computer it runs on. It records what the app does, not what your chats say, and
  your home folder is written as `~`. Nothing is sent unless you copy it out yourself with **About**, **Copy
  diagnostics**.
- **Private folders stay private.** The file browser hides dot files and private folders. A file that looks like
  a key or a secret is listed but never opened: **This looks like a key or a secret, so Switchboard won't open
  it.**
- **New folders need your yes.** Before Claude works in a folder it hasn't been in, Switchboard asks you first,
  the same question Claude Code would ask.

## What is stored where

| What | Where it is kept |
|---|---|
| A chat's text | On the computer the chat runs on, in Claude Code's own files |
| The board, your settings, the history and the library | On the home computer, in `~/.config/switchboard/` and `~/.local/share/switchboard/` |
| A secret that a library tool needs | In the keychain of each computer that uses the tool. Only its name is shared |
| The app's log and its saved addresses | On the computer the app runs on |

Device keys add these files:

| What | Where it is kept |
|---|---|
| A computer's device key | On that computer only, in `~/.config/switchboard/device.key`, readable by your account alone |
| The list of paired devices: each one's id, name, kind, and when and how it was paired | On the home computer, in `~/.local/share/switchboard/paired.json` |
| The home computer's id, its name, its relay and the address it was last reached at | On each paired computer, in `~/.config/switchboard/node.json` |
| When each kind of app last connected by address, as a time and nothing else | On the home computer |

A device key's secret half never leaves its device. On a computer, Switchboard keeps it in that one file and
doesn't put it in a keychain. It is never sent anywhere, and no other device ever needs it.

There is one exception, and it happens only when you ask for it. Moving the home computer sends its key to the
computer that takes over, inside the encrypted connection between the two, and a backup of the home computer
holds the key sealed behind a recovery code. Both are command line only for now. Refer to
[Move or replace the home computer](features/move-or-replace-home.md).

| What | Where it is kept |
|---|---|
| A copy of the list of paired devices, signed by the home computer | On each paired computer and phone, so that it can tell who to answer while the home computer is away |
| The devices you blocked on one computer | On that computer, in `blocked.json` |
| A backup of the home computer: the board's history, your settings, the library, the paired list and the home computer's key | In one file on the home computer, in `~/.local/share/switchboard/home-backup/`, or where you put it with `--to`. It is sealed, and opens only with the recovery code |
| The recovery code | Nowhere. It is shown once, when the first backup is made. The home computer keeps a key made from it, in `~/.config/switchboard/backup.key`, so that later backups don't ask |

A backup doesn't hold the text of your chats. That stays in Claude Code's own files on each computer. A backup
file and its recovery code together are the home computer, so keep them apart.

A paired phone keeps these in the app. The phone apps that pair are not released yet.

| What | Where it is kept |
|---|---|
| An iPhone's device key | In the iPhone's Keychain, for this device only. It isn't in iCloud Keychain, and a backup restored to another phone doesn't carry it |
| An Android phone's device key | In a file in the app's storage, sealed by a key in the Android Keystore. The app's data is left out of cloud backup and device transfer |
| The home computer's id, its name and its relay | In the app's settings on the phone |
| The last board the phone saw, so it has something to show when the home computer is out of reach | In a file in the app's storage, left out of backups |
| Replies written while the home computer was out of reach, for up to a day | In a file in the app's storage, left out of backups |

**Leave this home computer** in the app makes the phone forget the home computer. Refer to
[Pair a phone](features/pair-a-phone.md).

## What leaves your network, and when

| What | Goes to | When |
|---|---|---|
| The newest release's list of builds, and the builds | GitHub. The home computer fetches them | An app checks for updates, at most every 5 minutes, or downloads one. Refer to [Updates](updates.md#where-updates-come-from) |
| A request for the helper's programs | GitHub. The new computer fetches them | You add a computer with the app's line while your home computer has no downloads of its own |
| iPhone notifications: chat names, computer names and short status lines | Apple's push service | Only once you add an Apple push key to the home computer. Each kind has its own switch |
| A waiting chat's last message | Claude, on your own plan | Only with **Summarize waiting chats** on |
| A new chat's first message | Claude, on your own plan | Only with **Name new chats** on |
| A task sent to Auto, and each computer's tools, free memory and repo names | Claude, on your own plan | Only with **Let Claude pick for Auto** on |
| A condensed copy of a chat | Claude, on your own plan | When you use **Start fresh**, **Continue** or **Compact into one chat**, which write a summary |
| Encrypted traffic between two of your computers | The relay | Only with device keys, a relay set, and no direct path between the two |

The three switches are in **Settings**, **AI and your Claude setup**, and use Haiku. Summaries use Opus, or
Sonnet if you pick it. All of them run Claude Code on your own computers, on your own plan, like any other chat.

!!! note ""
    The AI features and the setup changes below are off for a new home computer. One that was already running
    before October 2026 turned them all on once, so check **Settings**, **AI and your Claude setup** if you'd
    rather have them off.

## What crosses the network

### By address

Today, the apps and the helpers reach the home computer at its address, on ports `8130` to `8132`. Switchboard
adds no encryption of its own to that traffic. It relies on the network it runs on:

- **On your home network**, the traffic is as private as that network is.
- **Over Tailscale**, a private network between your own devices, Tailscale encrypts it.
- **A computer added over ssh** is reached through ssh, which encrypts it.

### By device key

With device keys, a helper on each computer carries the traffic, and everything between two helpers is encrypted
from one computer to the other. Only those two computers hold the keys for it. This is the same whether the
connection is direct or passes through a relay.

A program on your computer, such as the app, then talks only to the helper on that same computer. That last
step never leaves the computer.

A paired phone has no helper, so the phone app carries its own traffic to the home computer, with the same
encryption. Inside the phone, the app talks to itself on two local ports, and each request there must carry a
secret that the app makes new every time it starts. This is because other apps on the phone can reach those
ports too.

While `keys` is set to `both`, connections by address still work beside the keyed ones, as described above. In
`only`, the home computer stops answering by address at all. For more information, refer to
[Connections](concepts/connections.md).

## What others can see

### A relay

A relay is used only with device keys, and only when one is set. It passes traffic it can't read. What it can
see:

- **Which device ids talk to each other.** An id is the public half of a device key.
- **The network address** each device connects from.
- **How much traffic** passes, and when.

It can't see your chats, your files, your terminals or the names of your computers, and it can't connect to
your home computer itself. While a short pairing code is live, the relay also sees the meeting point worked out
from that code. That value is made slow to compute on purpose, so nobody can work back from it to the code
within the code's ten minutes.

For keyed connections, Switchboard contacts only the relay it is given and uses no lookup service to find your
computers. You can run the relay yourself. Refer to
[Run your own relay](features/run-your-own-relay.md).

### Apple, with notifications as they work today

iPhone notifications are off until you add your own Apple push key to the home computer. The home computer then
sends each notification straight to Apple, which delivers it to your phone. Apple can read what a notification
says: chat names, computer names and short status lines.

### The push gateway, Apple and Google

!!! note ""
    The push gateway and notifications sealed for one phone are not released yet, and the phone apps aren't
    wired to them. This holds for a paired phone too.

The push gateway is a small service that holds the app publisher's push keys, so that your home computer doesn't
have to hold any. As built, this is what each party can see:

- **The gateway** sees your home computer's id, your phone's push token, and which kind of notification it is.
  The words are sealed on the home computer for your phone alone, so the gateway can't read them. It writes
  nothing to disk, and its log holds only the first characters of a hash of the id and the token.
- **Apple and Google** receive the sealed words and one fixed sentence, **A chat needs you**, which the phone
  replaces once it has opened the seal.
- **A Live Activity or widget update** carries numbers, yes or no values and one of three words: `working`,
  `done` or `failed`. It carries no chat name and no link.

Each request to the gateway is signed with the home computer's device key, and the home computer refuses a
gateway address that doesn't start with `https://`.

## Changes to your Claude setup

These are also in **Settings**, **AI and your Claude setup**, and also off until you turn them on. Each applies
to every computer:

| Setting | What it changes | Turning it off |
|---|---|---|
| **Subagent model rule** | Adds a short rule to `~/.claude/CLAUDE.md`: Haiku for single lookups, Sonnet for search and reading, the default model for design and code | Removes only the rule it added |
| **Explore agent on Sonnet** | Writes `~/.claude/agents/Explore.md`, so the Explore agent runs on Sonnet and only reads. One you wrote yourself is left alone | Deletes the file only if Switchboard wrote it |
| **Compact long chats at** (**Settings**, **Context window**) | Sets `CLAUDE_CODE_AUTO_COMPACT_WINDOW` to the value you pick in `~/.claude/settings.json`, such as 300000 for 300k. A computer where you chose a compact window yourself is left alone | On **Default**, removes it only if it's still the value Switchboard wrote |

## No passwords, on purpose

By address, Switchboard has no passwords. Anyone on your network who can reach its ports, `8130` to `8132`, can
read your chats, type into their terminals and start new chats on your computers. It is meant for your own home
network or your own [Tailscale](https://tailscale.com) network.

!!! note ""
    Never forward ports `8130` to `8132` to the internet. Use Tailscale, or another private network, to reach
    Switchboard away from home.

With device keys there are still no passwords. The home computer accepts a connection only from a device whose
key is on its paired list, and with `keys` set to `only` its ports stop answering your network at all. For more
information, refer to [Device keys and pairing](concepts/device-keys-and-pairing.md).

What Switchboard protects, in either way of connecting:

- **Adding a computer** takes a one-time code that works once and expires after 10 minutes.
- **A computer added over ssh** gets a key that can only run the Switchboard helper: no shell and no port
  forwarding.
- **The home computer answers only your own networks.** It looks at the address each request really comes from,
  and answers only this computer, home and office networks (10.x, 172.16 to 172.31, 192.168.x), link-local
  addresses, Tailscale (100.64.0.0/10) and their IPv6 kinds. Anything else gets **forbidden address**, and the
  log says so at most once a minute. This covers port `8130`. Ports `8131` and `8132` still rely on your
  network. To let another network in, refer to [Ports, files and settings](reference.md#serverjson).
- **The home computer answers only to its own names.** It checks the `Host` of each request against the
  addresses it knows, so a web page you visit can't reach it by renaming itself.
- **Updates are signed.** The app installs only builds signed with the release key, and checks each file's
  SHA-256. Refer to [Updates](updates.md#why-you-can-trust-an-update).
- **Downloads are checked for damage.** `install-agent.sh` checks `sb.py` and `switchboard-node` against the
  home computer's `SHA256SUMS` when it has one, and `get-server.sh` checks the kit against the `.sha256` beside
  it. The sums come from the same place as the files, so they catch a broken download, not a source that lies.
- **Nothing runs as an administrator.** Every part of Switchboard installs for your own account. A few optional
  steps need an administrator, and you run them yourself: a Windows firewall rule, keeping a Linux home
  computer's services running after you log out (`loginctl enable-linger`), and adding a Windows computer over
  ssh when your account there is an administrator.

## Report a security problem

--8<-- "SECURITY.md:report"
