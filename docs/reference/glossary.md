---
description: Short definitions of the words Switchboard uses, from board and chat to warm, cold, Start fresh, the guard, places, the library and the daily budget.
---

# Terminology

<p class="sb-meta">Last validated: Nov 22, 2026</p>

This page defines the words that Switchboard and these docs use, in alphabetical order. A word in **bold** is
spelled the way the app spells it.

## Anywhere in home

**Anywhere in home** is a switch for each computer that lets chats and Files use any folder in its home folder,
not only the Claude folder. It's on unless you turn it off. Refer to
[Places and folders](../concepts/places-and-folders.md).

## Archived

An archived chat is one that was archived in the Claude app. It stays in History, where **Archived** is one of the
state filters.

## Auto

**Auto** lets Switchboard pick the computer for a new chat. It goes by what the task needs and what each computer
can do, or asks Claude to pick when **Let Claude pick for Auto** is on. Refer to
[Start a chat](../features/start-a-chat.md).

## Background chat

A background chat is a Claude Code chat that runs on its computer without a terminal window of its own. Chats
that Switchboard starts are background chats, and only background chats open in a tab in the app.

## Board

The board is Switchboard's main view: every chat on every computer, sorted into columns by what each one needs
from you. Refer to [The board](../board.md).

## Cache

The cache is the copy of a recent chat that Claude keeps ready, so the next message doesn't have to read the
whole chat again. Claude Code's cache lasts 5 minutes or 1 hour after each reply.

## Capabilities

Capabilities are what a computer can do, such as the developer tools Switchboard found on it and anything you
added in your own words. Auto goes by them when it picks a computer.

## Card

A card is one chat on the board. It shows the chat's name, one line for where it stands, and what a reply would
cost.

## Chat

A chat is one Claude Code conversation, running on one computer in one folder. The app always says chat, where
Claude Code itself says session.

## Claude folder

The Claude folder is the folder on a computer where new chats start unless you pick another one. Each computer
has its own, and it's always open to the app.

## Claude in Chrome

Claude in Chrome is Claude's extension for the Chrome browser. A chat that uses it picks a Chrome computer, which
is one of your computers that has Chrome and the extension.

## Cold

A chat is cold when its cache has run out. The next message makes Claude read the whole chat again at full price,
which costs more the larger the chat is. Refer to [Warm and cold](../concepts/warm-and-cold.md).

## Compact

To compact a chat is to replace its older part with a summary, which Claude Code does by itself when a chat
grows past a certain size. **Compact into one chat** in History does the same for a whole thread.

## Computer

A computer is any Mac, Windows PC or Linux computer that runs Claude Code and that you added to Switchboard. The
app's Settings call it a machine.

## Context

A chat's context is everything Claude reads with each message: the chat so far, plus its instructions and tools.
Switchboard shows it in tokens, such as 290k.

## Context window

The context window is the most context a model can hold. **Compact long chats at** in Settings sets how much of a
long chat is kept word for word before Claude Code compacts it.

## Continue

**Continue** moves a chat to another computer. Switchboard writes a summary on the first computer and starts a
new chat from it on the other one, in the same folder. Refer to
[Continue on another computer](../features/continue-on-another-computer.md).

## Daily budget

The daily budget is the share of your weekly limit that one day should use. It's 14.3% unless you change it,
which spreads the week evenly over seven days. Refer to [Daily budget](../features/daily-budget.md).

## Device key

A device key is a secret that one computer makes for itself and keeps in a file only your account can read. Its
public half is the computer's id. A phone makes one too, and the app keeps it in the phone's Keychain or
Keystore. Device keys arrived with version 0.9.0. Refer to
[Device keys and pairing](../concepts/device-keys-and-pairing.md).

## Direct connection

A direct connection runs straight between two of your computers, over your home network, Tailscale or any other
address that reaches. The helper's status says **connected directly**, and the apps say **Direct**. Refer to
[Connections](../concepts/connections.md).

## 5-hour window

The 5-hour window is the shorter of the two limits a Claude plan reports. Switchboard shows how full it is, and
a **5-hour cap** can warn you before it fills.

## Floating sheet

A floating sheet is a terminal shown over the board, not in a tab or a window of its own.

## Gone cold

**Gone cold** is the board's column for quiet chats that are cold and large, 150k tokens or more. A reply to one
of them reads all of it again.

## Guard

The guard holds a message you typed into a chat of 150k tokens or more that has gone cold, so the chat isn't read
again in full by surprise. Refer to [The cold chat guard](../features/cold-chat-guard.md).

## Hard limit

The hard limit is an optional stop on your usage. When a day or the 5-hour window passes it, new messages are
blocked and running background chats are stopped. It's off unless you turn it on.

## Held reply

A held reply is a message the guard is holding. The chat moves to **Needs you** and says **Message on hold**
until you send the message anyway or start fresh.

## Helper

The helper is the small part of Switchboard that runs on every computer you add. It reports that computer's
chats, starts and stops them, and lists folders for Files.

## History

**History** lists every chat on every computer, including the ones that have left the board. Refer to
[History](../features/history.md).

## Home computer

The home computer is the one of your computers that runs Switchboard itself. It keeps the board, the library and
your settings. The app's Settings call it the server.

## Hub

The hub is the part of the home computer's software that carries chat terminals and the links from computers you
added with a code. It runs on a Mac or Linux home computer, on port 8132. Since version 0.9.0 it is installed on a
Windows home computer too.

## Keep running

**Keep running** is a mark on a chat that tells Switchboard never to stop it by itself, however long it sits
idle.

## Key

A key is a secret that a library tool needs, such as an API key. Each computer keeps its own copy in its
keychain, and only the key's name is shared.

## Library

The library is the set of tools you built, kept on the home computer so that a chat on any computer can use
them. Refer to [Library](../features/library.md).

## Needs you

**Needs you** is the board's first column. A chat is there when Claude is waiting for an answer or a permission,
or when the guard is holding a message you typed.

## Notices

**Notices** is the tray at the right of the status bar. Messages that left the corner of the window wait there
until you act on them or ignore them.

## One-time code

A one-time code is six letters and digits that add one computer to your home computer by address. It works once
and for 10 minutes. Refer to [Add your computers](../machines.md).

## Paired device

A paired device is a computer or a phone whose device key the home computer accepts. The home computer keeps the
list of them, and removing a device from it cuts that device off at once. The desktop app shows the list in
**Settings**, **Devices**.

## Pairing code

A pairing code is eight letters and digits, shown as `K7PM-2XQ9`, that pairs one computer or phone with the home
computer. It works once and for ten minutes. Refer to [Pair a computer](../features/pair-a-computer.md) and
[Pair a phone](../features/pair-a-phone.md).

## Parked

A parked chat has waited for you for more than a day. It stays in **Needs you**, muted, and stops counting toward
the badge and notifications.

## Place

A place is a folder on a computer that Switchboard may open: the Claude folder, a folder in the home folder, or
a folder someone added at that computer. Refer to [Places and folders](../concepts/places-and-folders.md).

## Private folder

A private folder is one that Switchboard never lists or opens: any folder whose name starts with a dot,
`~/Library` on a Mac except iCloud Drive, and `AppData` on Windows.

## Quiet

**Quiet** is the board's column for chats that need nothing and cost little: small chats that have gone cold,
chats whose cache isn't known yet, and stopped chats.

## Relay

A relay is a small server that two of your devices meet at when they can't reach each other directly. It passes
their encrypted traffic and can't read it. Refer to [Run your own relay](../features/run-your-own-relay.md).

## Replaced

A replaced chat is one that a newer chat continues, after **Start fresh** or **Continue**. It leaves the board
and stays in History.

## Resume

To resume a chat is to go back to it as it is, with everything it has said so far. A warm chat resumes cheaply.
A cold one reads all of its context again.

## Saved setup

A saved setup is a combination of model, effort, mode and flags that you saved when starting a chat, so you can
pick it again in one click.

## Sent by Claude

**Sent by Claude** marks a task that one of your chats sent to another computer. It reports to the chat that
sent it, not to you. Refer to [Chats sent by Claude](../features/chats-sent-by-claude.md).

## Stalled

A stalled chat says it's working but has made no progress for six hours.

## Star

A star keeps a terminal tab first in the tab row. Switchboard also never stops a starred chat by itself.

## Start fresh

**Start fresh** continues the work of a chat in a new, small chat. Switchboard writes a summary of the old chat
and opens a new chat in the same folder with only that summary. Refer to
[Resume or start fresh](../features/resume-or-start-fresh.md).

## Stop idle chats

**Stop idle chats** is the setting for how long a finished chat is left alone before Switchboard stops it to free
its memory. It's 6 hours unless you change it, and **Resume** brings a stopped chat back.

## Summary

A summary is what Switchboard writes when a chat starts fresh, continues on another computer or is compacted with
its thread. It keeps your instructions and points back to the old chat. The app sometimes calls it a handoff.

## Tab group

A tab group is a named, colored set of terminal tabs that you can fold. Groups are the same on every computer you
use the app on.

## Thread

A thread is a chat together with the newer chats that continued it. History can show a thread as one row.

## Tool

A tool is something in the library: a skill, an MCP (Model Context Protocol) server, or both in one Claude Code
plugin folder.

## Trust

Trust is Claude's permission to work in a folder. Outside the Claude folder, Switchboard asks **Let Claude work
in** this folder the first time, the same question Claude Code itself would ask.

## Warm

A chat is warm while its cache lasts. A reply to a warm chat is cheap, because Claude reads the chat from the
cache.

## Warm now

**Warm now** is the board's column for quiet chats whose cache is still warm. The one about to go cold comes
first.

## Weekly limit

The weekly limit is the longer of the two limits a Claude plan reports. The daily budget is a share of it.

## Working

**Working** is the board's column for chats where Claude is working right now.
