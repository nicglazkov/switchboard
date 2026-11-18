---
description: What each section of Switchboard's Settings holds, from your computers and their Claude folders to notifications, Chrome, the budget, updates and the opt-in AI features.
---

# Settings

++cmd+comma++ opens **Settings**. Most of it lives on the server, so it's the same on every computer you use the
app on. The few settings kept on one computer say **This computer only**: the default shell, the terminal's text
size, **Stop a chat when its tab closes**, that computer's own notifications and the server's addresses.

The sidebar groups the sections: **General** (Appearance, Notifications), **Chats** (Chats, Context window, Terminal, Budget),
**Claude** (Library, Chrome, AI and your Claude setup) and **Machines** (Machines, Server). **Updates** and
**About** stay at the bottom.

## Appearance

Light, dark, or the system's. Synced, so your other computers match.

## Terminal

The color scheme, the default shell for new terminals, the text size, the cursor's shape and blink, and **Stop a
chat when its tab closes**. The text size is kept on each computer, since screens differ.
[Terminals](terminals.md#look)

## Chats

- **Summaries:** who writes them for Start fresh and Continue: **Opus** (the most careful) or **Sonnet** (quicker,
  and less of your plan).
- **Saved setups:** the combinations you saved in New chat, which it offers along with Defaults and Last used.
  **Remove** asks you to click again.
- **Stop idle chats** is for every computer: how long a finished chat is left alone before it's stopped to free
  its memory (**6 hours** unless you change it, or **Never**). Chats replaced by a newer one or archived in the
  Claude app wait the same time, and resuming a chat starts its wait again. [What stops, and what never does](board.md#tidy-up)

## Context window

One slider: **Compact long chats at**. It sets how much of a long chat Claude Code keeps word for word before it
compacts it, which means replacing the older part with a summary. The same value goes to every computer and
covers every model.

| Stop | What it means |
|---|---|
| **150k** (Lean) | Chats stay small, so coming back to a cold one costs the least. Compacts soonest |
| **200k** (Standard) | The size of a model's window before the 1M ones |
| **300k** (Balanced), recommended | Room for a long working session, and a cold resume that still costs a fraction of a full window |
| **500k** (Long memory) | Half the window stays word for word. A cold resume re-reads all of it |
| **Default** (Claude Code's default) | Switchboard sets nothing. Today's models compact at about 967k |

The words under the slider change with the stop and say what coming back to a cold chat that size costs. A model
with a smaller window than the value compacts at its own window, so one value is safe for all of them. To give
each model its own window, pick **Default** and use `/autocompact` in Claude Code.

Two things follow the value and are not settings of their own: the context meters fill at the point where that
chat compacts, and [Start fresh](chats.md#resume-or-start-fresh) is suggested from nine tenths of it (270k at
300k).

**On each computer** lists what every computer has. Switchboard writes its value
(`CLAUDE_CODE_AUTO_COMPACT_WINDOW` in that computer's Claude Code settings) only where no compact window was
chosen by hand, and only ever changes or removes what it wrote:

- **Set on this computer: 500k, left alone** means you set a window there yourself, in the settings file or with
  `/autocompact`. The slider never touches it.
- **Older helper** means that computer's Switchboard is from before the slider and only knows 300k. Update it
  there.
- A computer that is offline follows when it's back.

A chat picks the value up the next time it starts. On a server from before the slider, the page offers the old
switch instead (300k or Claude Code's default).

## Machines

Every computer Switchboard watches, in board order: rename, reorder, remove, and **Add a machine**. Open one for:

- **Claude folder:** where new chats start.
- **Beyond the Claude folder:** whether chats and Files may go anywhere in its home folder, or stay in the Claude
  folder and the places added there.
- **Capabilities:** tools like Xcode, an Android SDK or a GPU, which [Auto](chats.md#auto) goes by, and a box
  for anything else it can do that Switchboard can't find by itself.
- **MCP servers:** read from Claude Code's settings there (names and commands only; secrets never leave the
  computer), with **Add to library** to share one with every chat.

## Library

Tools you built, kept on the server so a chat on any computer can use them. A tool is a Claude Code plugin: a
skill or an MCP server.

- **Import...** a skill or an MCP server one of your computers already has.
- **New tool...** describes what it should do and picks a computer to build it on; a chat there builds it, tests
  it and publishes it to the library. You can follow it on the board.
- Each tool has **On for every new chat**, **Include in Auto**, **When to use**, the computers it's on, and every
  version, with **Make latest** to go back.
- **Keys it needs** are names only: each computer keeps its own values in its system keychain
  (`sb.py secret set <tool> <NAME>`).

A chat gets a tool when you attach it (the chat restarts, and reads its context again) or when the tool is on for
every new chat. Chats can also publish and list tools themselves, through the `switchboard` MCP server the helper
adds to Claude Code.

## Notifications

- **On this Mac** (or **On this computer**): **When a chat needs you**, a plain notification with the chat's name and computer. Click
  it to open the board. It doesn't show while a Switchboard window is in front.
- **On your iPhone:** a switch for each kind: **Held messages**, **Finished tasks**, **Chats not in use** (at
  most once a day) and **Hard limit**. [iPhone notifications](apps.md#iphone-notifications)

Toasts that leave the corner wait in **Notices**, at the right of the status bar, until you ignore them.

## Chrome

For chats that use [Claude in Chrome](https://claude.ai/chrome). It lists which computers have Chrome, the
extension, and a live connection. Each chat picks its own Chrome computer, in New chat's **Claude in Chrome**
line or in the chat's details, and Start fresh and Continue keep it. **Auto** sends a Chrome chat to its Chrome
computer.

## Budget

A daily share of your weekly limit, a 5-hour cap, and the hard limit. The − and + buttons move to whole numbers.
[Usage and budget](usage.md)

## Updates

Whether this build gets updates, what it last found, and **Check now**. [Updates](updates.md)

## Server

The computer that runs Switchboard: its version, whether each part is running, free disk space, **Update
server** when a newer one is out, **Restart**, its logs, and **Copy details**. [Updates](updates.md#the-server)

**Server addresses** lists every address this app knows for the server and marks the ones it learned. Type one
more in the field under the list, such as a Tailscale name: paste a whole link and the app keeps only the
address. Addresses start with `http://`; an `https://` address is not accepted, because the app does not connect
over https. Any address can be removed; the one in use asks you to click again first.

## AI and your Claude setup

Everything here is off on a new server until you turn it on. A server set up before these switches existed
had them all turned on once, so look them over.

| | |
|---|---|
| **Summarize waiting chats** | Claude sums up what a waiting chat is asking. Its last message goes to Claude |
| **Name new chats** | Claude names a new chat from its first message |
| **Let Claude pick for Auto** | Claude picks the computer for Auto from the task and each computer's tools, free memory and repo names |
| **Subagent model rule** | A short rule in `~/.claude/CLAUDE.md` on each computer: Haiku for lookups, Sonnet for search and reading |
| **Explore agent on Sonnet** | `~/.claude/agents/Explore.md` on each computer |

The first three use your own Claude plan. Turning a setup change off removes only what Switchboard added. Where
long chats are compacted is also a change to each computer's Claude Code settings; it has its own page,
[Context window](#context-window).
[Privacy and security](privacy.md#what-leaves-your-network-and-when)

## About

In the app menu: the version and build, the server it talks to, **Copy version info**, **What's new**, **Show
log**, **Copy diagnostics** and the open source licenses.
