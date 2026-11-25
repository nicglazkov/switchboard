---
description: Short answers to common questions about Switchboard, what it is, what it needs, what it does with your chats and your plan, and where the phone apps stand.
---

# Questions

<p class="sb-meta">Last validated: Nov 22, 2026</p>

This page gives short answers to the questions people ask most, grouped by area. Each answer links to the page
that covers the subject in full.

## About Switchboard

### Is Switchboard made by Anthropic?

No. Switchboard is an independent open source project that works with
[Claude Code](https://claude.com/claude-code). It isn't made by or affiliated with Anthropic.

### What does it cost?

Nothing. Switchboard is free software under the GNU AGPL-3.0 license. Your chats use your own Claude plan, the
same as they would without Switchboard.

### Does it replace Claude Code?

No. Your chats still run in Claude Code, on the computers where you started them. Switchboard reads what Claude
Code reports and gathers it on one board. Refer to [What is Switchboard?](concepts/what-is-switchboard.md).

### Does it change how Claude Code works?

It adds three things to Claude Code's settings on each computer: hooks, a status line and an MCP (Model Context
Protocol) server named `switchboard`. It saves the status line you had, and `sb.py uninstall` removes only what
it added. Refer to [Privacy and security](privacy.md#changes-to-your-claude-setup).

## Computers and setup

### Do I need more than one computer?

No. One computer can run Switchboard itself, the app and Claude Code. The board still sorts your chats and shows
what a reply would cost. Refer to [Install Switchboard](install.md).

### Which computer should be the home computer?

Pick one that stays on and that your other computers can reach, such as a desktop that never sleeps. The home
computer is the one that runs Switchboard itself and keeps the board. A laptop works until it sleeps. Nothing
else can reach the board while it does. Refer to [Install Switchboard](install.md).

### Can the home computer run Windows?

Yes, with less. Before version 0.9.0 a Windows home computer had no chat terminals and couldn't add computers with a code.
Since 0.9.0 the installer puts the hub on Windows too. Refer to
[Install Switchboard](install.md#a-windows-server).

### Does it work away from home?

Today, only over a VPN (virtual private network), such as Tailscale, on the home computer and on the device you
carry. Since version 0.9.0, a paired computer or phone reaches the home computer by its device key with no
VPN. Refer to [Install Switchboard](install.md#away-from-home).

### Do I have to keep the app open?

No. The home computer keeps the board whether or not an app is open. The app only shows it.

### How do I add another computer?

Refer to [Add your computers](machines.md).

### How do I remove Switchboard?

Refer to [Install Switchboard](install.md#uninstalling).

## Chats and what they cost

### Why does a reply to an old chat cost more?

Claude keeps a recent chat ready for a few minutes. After that the chat is cold, and the next message makes
Claude read the whole chat again. Refer to [Warm and cold](concepts/warm-and-cold.md).

### What does Start fresh do?

**Start fresh** continues the work of a chat in a new, small chat that starts from a summary of the old one.
Refer to [Resume or start fresh](features/resume-or-start-fresh.md).

### Why didn't my message go through?

If the chat is large and has gone cold, Switchboard holds the message and asks you first. The chat moves to
**Needs you** and says **Message on hold**. Refer to [The cold chat guard](features/cold-chat-guard.md).

### Will Switchboard stop my chats?

Only a chat that loses nothing by it. A chat that finished its reply and has been left alone for 6 hours is
stopped to free its memory, and **Resume** brings it back. Refer to [Tidy up](features/tidy-up.md).

### Where do the numbers for my plan come from?

From Claude Code on one of your computers, which reports how much of the 5-hour window and the week you've used.
Switchboard never reads your sign-in. Refer to [Usage and budget](usage.md).

## Privacy

### Does it send my chats anywhere?

No. Your chats, files and history stay on your own computers. The few things that do leave your network, such as
the look for updates on GitHub, are listed in [Privacy and security](privacy.md#what-leaves-your-network-and-when).

### Is there a password?

No. Anyone who can reach your home computer's ports can see and run your chats, so keep it on a network that only
you can reach. Refer to [Privacy and security](privacy.md#no-passwords-on-purpose).

### Is there an account to create?

No. Switchboard runs on your own computers and has no account.

## Apps and updates

### Is there an iPhone or Android app?

Not one you can download today. An iPhone app exists, but it isn't on the App Store or TestFlight. There is no
Android app to install. Since version 0.9.0, both apps pair with the home computer by a code. Refer to
[The apps](apps.md#iphone) and [Pair a phone](features/pair-a-phone.md).

### How do I update Switchboard?

The app offers a new version by itself. You update the home computer from the app's Settings and the helper on
each computer with one command. Refer to [Updates](updates.md).

### Where do I report a bug?

Refer to [Get help](support.md).
